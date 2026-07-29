//! De motor: bezit de mesh en de oplog, draait op de tokio-runtime.
//!
//! # Waarom dit niet in de UI zit
//!
//! egui werkt zijn frames alleen bij zolang het venster zichtbaar is. Zat de chat- en
//! sync-lus in `update()`, dan stopt de synchronisatie zodra je minimaliseert of naar
//! de tray gaat — precies het moment waarop je een melding zou willen krijgen dat er
//! iemand iets zegt. Voor een app die naast een game moet kunnen draaien is dat het
//! verkeerde gedrag.
//!
//! De UI leest daarom alleen nog een momentopname en stuurt commando's terug. Zij mag
//! stilvallen zonder dat er iets misgaat.

use crate::chat::Chat;
use crate::config::Config;
use crate::notify;
use anyhow::Result;
use fitcom_net::{MeshCommand, MeshEvent, MeshHandle, PeerStatus};
use fitcom_proto::{OpId, PeerId};
use fitcom_store::{Store, Timeline};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};

/// Hoe vaak we een verse momentopname publiceren, ook als er niets veranderde.
/// Houdt de RTT in de ledenlijst actueel.
const PUBLICEER_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug, Clone)]
pub struct PeerView {
    pub label: String,
    pub address: String,
    pub peer_id: Option<PeerId>,
    pub status: PeerStatus,
}

/// Alles wat de UI nodig heeft om zichzelf te tekenen.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub timeline: Timeline,
    pub peers: Vec<PeerView>,
    pub ongelezen: usize,
    pub fout: Option<String>,
}

#[derive(Debug)]
pub enum UiCommand {
    Plaats(String),
    Bewerk(OpId, String),
    Verwijder(OpId),
    Gelezen,
    FoutWeg,
}

pub struct EngineHandle {
    pub snapshot: watch::Receiver<Arc<Snapshot>>,
    pub commands: mpsc::Sender<UiCommand>,
    /// Wordt door de UI bijgehouden. Staat het venster niet op de voorgrond, dan
    /// verstuurt de motor een Windows-melding bij een nieuw bericht.
    pub voorgrond: Arc<AtomicBool>,
}

pub fn spawn(
    mesh: MeshHandle,
    store: Store,
    cfg: Config,
    config_path: PathBuf,
) -> Result<EngineHandle> {
    let chat = Chat::new(store)?;
    let (cmd_tx, cmd_rx) = mpsc::channel(64);
    let (snap_tx, snap_rx) = watch::channel(Arc::new(Snapshot::default()));
    let voorgrond = Arc::new(AtomicBool::new(true));

    let peers = cfg
        .peers
        .iter()
        .map(|p| PeerView {
            label: if p.label.is_empty() {
                p.address.clone()
            } else {
                p.label.clone()
            },
            address: p.address.clone(),
            peer_id: p.known_id,
            status: PeerStatus::Offline {
                reason: "nog niet verbonden".into(),
            },
        })
        .collect();

    let engine = Engine {
        mesh,
        chat,
        cfg,
        config_path,
        peers,
        verbonden: Vec::new(),
        fout: None,
        snap_tx,
        voorgrond: voorgrond.clone(),
    };

    tokio::spawn(engine.run(cmd_rx));

    Ok(EngineHandle {
        snapshot: snap_rx,
        commands: cmd_tx,
        voorgrond,
    })
}

struct Engine {
    mesh: MeshHandle,
    chat: Chat,
    cfg: Config,
    config_path: PathBuf,
    peers: Vec<PeerView>,
    verbonden: Vec<PeerId>,
    fout: Option<String>,
    snap_tx: watch::Sender<Arc<Snapshot>>,
    voorgrond: Arc<AtomicBool>,
}

impl Engine {
    async fn run(mut self, mut cmds: mpsc::Receiver<UiCommand>) {
        // Eigen naam vastleggen in de oplog zodat de anderen hem zien. Doet niets als
        // hij al klopt, dus de log groeit hier niet van bij elke start.
        let naam = self.cfg.display_name.clone();
        let r = self.chat.zet_naam(&naam);
        self.verwerk(r);

        self.publiceer();

        let mut ticker = tokio::time::interval(PUBLICEER_INTERVAL);

        loop {
            tokio::select! {
                ev = self.mesh.events.recv() => match ev {
                    Some(ev) => self.op_mesh_event(ev).await,
                    None => break,
                },
                cmd = cmds.recv() => match cmd {
                    Some(cmd) => self.op_ui_command(cmd),
                    None => break,
                },
                _ = ticker.tick() => {
                    let verbonden = self.verbonden.clone();
                    let r = self.chat.tick(&verbonden);
                    self.verwerk(r);
                }
            }

            self.chat.refresh();
            self.publiceer();
        }

        tracing::info!("motor gestopt");
    }

    async fn op_mesh_event(&mut self, ev: MeshEvent) {
        match ev {
            MeshEvent::Status { target, status } => {
                let was_online = matches!(
                    self.peers.get(target).map(|p| &p.status),
                    Some(PeerStatus::Online { .. })
                );

                if let PeerStatus::Online {
                    peer_id,
                    display_name,
                    ..
                } = &status
                {
                    if !self.verbonden.contains(peer_id) {
                        self.verbonden.push(*peer_id);
                    }
                    if let Some(p) = self.peers.get_mut(target) {
                        p.peer_id = Some(*peer_id);
                        p.label = display_name.clone();
                    }
                    if !was_online {
                        // Net verbonden: vraag meteen op wat we gemist hebben.
                        let r = self.chat.bij_verbinding(*peer_id);
                        self.verwerk(r);
                    }
                } else if let Some(id) = self.peers.get(target).and_then(|p| p.peer_id) {
                    self.verbonden.retain(|p| *p != id);
                }

                if let Some(p) = self.peers.get_mut(target) {
                    p.status = status;
                }
            }

            MeshEvent::LearnedIdentity { target, peer_id } => {
                if let Some(p) = self.cfg.peers.get_mut(target) {
                    p.known_id = Some(peer_id);
                    if let Err(e) = self.cfg.save(&self.config_path) {
                        self.fout = Some(format!("config opslaan: {e:#}"));
                    }
                }
                if let Some(p) = self.peers.get_mut(target) {
                    p.peer_id = Some(peer_id);
                }
            }

            MeshEvent::Message { from, msg } => {
                let voor = self.chat.ongelezen;
                let r = self.chat.bij_bericht(from, msg);
                self.verwerk(r);

                if self.chat.ongelezen > voor && !self.voorgrond.load(Ordering::Relaxed) {
                    self.meld_nieuw_bericht(from);
                }
            }
        }
    }

    /// Toont het laatste bericht van deze peer als Windows-melding.
    fn meld_nieuw_bericht(&mut self, van: PeerId) {
        self.chat.refresh();
        let tl = self.chat.timeline();
        let naam = tl
            .nicknames
            .get(&van)
            .cloned()
            .unwrap_or_else(|| van.to_string()[..8].to_string());
        let laatste = tl
            .messages
            .iter()
            .rev()
            .find(|m| m.author == van)
            .map(|m| m.body.clone())
            .unwrap_or_default();

        notify::nieuw_bericht(&naam, &laatste);
    }

    fn op_ui_command(&mut self, cmd: UiCommand) {
        match cmd {
            UiCommand::Plaats(tekst) => {
                let r = self.chat.plaats_bericht(&tekst);
                self.verwerk(r);
            }
            UiCommand::Bewerk(doel, tekst) => {
                let r = self.chat.bewerk_bericht(doel, &tekst);
                self.verwerk(r);
            }
            UiCommand::Verwijder(doel) => {
                let r = self.chat.verwijder_bericht(doel);
                self.verwerk(r);
            }
            UiCommand::Gelezen => self.chat.markeer_gelezen(),
            UiCommand::FoutWeg => self.fout = None,
        }
    }

    fn verwerk(&mut self, r: Result<Vec<MeshCommand>>) {
        match r {
            Ok(cmds) => {
                for cmd in cmds {
                    if let Err(e) = self.mesh.commands.try_send(cmd) {
                        tracing::warn!(error = %e, "netwerkcommando niet verstuurd");
                    }
                }
            }
            Err(e) => {
                tracing::error!(error = %format!("{e:#}"), "chat-actie mislukt");
                self.fout = Some(format!("{e:#}"));
            }
        }
    }

    fn publiceer(&self) {
        let _ = self.snap_tx.send(Arc::new(Snapshot {
            timeline: self.chat.timeline().clone(),
            peers: self.peers.clone(),
            ongelezen: self.chat.ongelezen,
            fout: self.fout.clone(),
        }));
    }
}
