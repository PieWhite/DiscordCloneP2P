//! De motor: bezit de mesh, de oplog en de voice-sessie. Draait op de tokio-runtime.
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
use anyhow::{Context, Result};
use fitcom_audio::{PeerAdres, VoiceConfig, VoiceHandle};
use fitcom_net::{MeshCommand, MeshEvent, MeshHandle, PeerStatus};
use fitcom_proto::control::{VoiceJoin, VoiceLeave};
use fitcom_proto::{ControlMsg, OpId, PeerId};
use fitcom_store::{Store, Timeline};
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};

/// Snel genoeg voor een spreekindicatie die niet hakkelt. Een momentopname is goedkoop:
/// de timeline zit erin als `Arc`, dus publiceren kost geen kopie van de geschiedenis.
const TIK: Duration = Duration::from_millis(100);

#[derive(Debug, Clone)]
pub struct PeerView {
    pub label: String,
    pub address: String,
    pub peer_id: Option<PeerId>,
    pub status: PeerStatus,
    /// Zit deze peer in het gesprek?
    pub in_voice: bool,
    /// 0..1, voor de spreekindicatie.
    pub niveau: f32,
    pub volume: f32,
}

#[derive(Debug, Clone, Default)]
pub struct VoiceView {
    pub actief: bool,
    pub muted: bool,
    pub deafened: bool,
    pub eigen_niveau: f32,
}

/// Alles wat de UI nodig heeft om zichzelf te tekenen.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub timeline: Arc<Timeline>,
    pub peers: Vec<PeerView>,
    pub voice: VoiceView,
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
    VoiceDeelnemen,
    VoiceVerlaten,
    Mute(bool),
    Deafen(bool),
    Volume(PeerId, f32),
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
            in_voice: false,
            niveau: 0.0,
            volume: 1.0,
        })
        .collect();

    let engine = Engine {
        mesh,
        chat,
        cfg,
        config_path,
        peers,
        verbonden: HashMap::new(),
        voice: None,
        peers_in_voice: HashSet::new(),
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
    /// Verbonden peers met het adres waar hun media heen moet.
    verbonden: HashMap<PeerId, SocketAddr>,
    voice: Option<VoiceHandle>,
    /// Peers die gemeld hebben dat ze in het gesprek zitten. Los van of wij meedoen:
    /// je wilt kunnen zien wie er praat voordat je zelf aansluit.
    peers_in_voice: HashSet<PeerId>,
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

        let mut ticker = tokio::time::interval(TIK);

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
                    let verbonden: Vec<PeerId> = self.verbonden.keys().copied().collect();
                    let r = self.chat.tick(&verbonden);
                    self.verwerk(r);
                }
            }

            self.chat.refresh();
            self.publiceer();
        }

        tracing::info!("motor gestopt");
    }

    // -- mesh --------------------------------------------------------------

    async fn op_mesh_event(&mut self, ev: MeshEvent) {
        match ev {
            MeshEvent::Status { target, status } => {
                let was_online = matches!(
                    self.peers.get(target).map(|p| &p.status),
                    Some(PeerStatus::Online { .. })
                );

                match &status {
                    PeerStatus::Online {
                        peer_id,
                        display_name,
                        media_addr,
                        ..
                    } => {
                        self.verbonden.insert(*peer_id, *media_addr);
                        if let Some(p) = self.peers.get_mut(target) {
                            p.peer_id = Some(*peer_id);
                            p.label = display_name.clone();
                        }
                        if !was_online {
                            let r = self.chat.bij_verbinding(*peer_id);
                            self.verwerk(r);
                            // Zit ik al in het gesprek, dan moet deze peer dat weten;
                            // hij heeft mijn eerdere melding gemist.
                            if self.voice.is_some() {
                                self.meld_voice_status(Some(*peer_id));
                            }
                        }
                    }
                    _ => {
                        if let Some(id) = self.peers.get(target).and_then(|p| p.peer_id) {
                            self.verbonden.remove(&id);
                            // Weg is weg: uit het gesprek halen, anders blijven we
                            // audio naar een dood adres sturen.
                            self.peers_in_voice.remove(&id);
                        }
                    }
                }

                if let Some(p) = self.peers.get_mut(target) {
                    p.status = status;
                }
                self.werk_voice_peers_bij();
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

            MeshEvent::Message { from, msg } => match msg {
                ControlMsg::VoiceJoin(_) => {
                    tracing::info!(peer = ?from, "peer neemt deel aan het gesprek");
                    self.peers_in_voice.insert(from);
                    self.werk_voice_peers_bij();
                }
                ControlMsg::VoiceLeave(_) => {
                    tracing::info!(peer = ?from, "peer verlaat het gesprek");
                    self.peers_in_voice.remove(&from);
                    self.werk_voice_peers_bij();
                }
                andere => {
                    let voor = self.chat.ongelezen;
                    let r = self.chat.bij_bericht(from, andere);
                    self.verwerk(r);
                    if self.chat.ongelezen > voor && !self.voorgrond.load(Ordering::Relaxed) {
                        self.meld_nieuw_bericht(from);
                    }
                }
            },
        }
    }

    // -- voice -------------------------------------------------------------

    fn deelnemen(&mut self) {
        if self.voice.is_some() {
            return;
        }
        let cfg = VoiceConfig {
            media_port: self.cfg.media_port,
            input_device: self.cfg.input_device.clone(),
            output_device: self.cfg.output_device.clone(),
        };
        match fitcom_audio::start(cfg) {
            Ok(h) => {
                self.voice = Some(h);
                self.werk_voice_peers_bij();
                self.meld_voice_status(None);
            }
            Err(e) => {
                tracing::error!(error = %format!("{e:#}"), "voice starten mislukt");
                self.fout = Some(format!("microfoon of weergave: {e:#}"));
            }
        }
    }

    fn verlaten(&mut self) {
        if self.voice.take().is_some() {
            self.meld_voice_status(None);
        }
    }

    /// `naar = None` betekent: iedereen.
    fn meld_voice_status(&mut self, naar: Option<PeerId>) {
        let msg = if self.voice.is_some() {
            ControlMsg::VoiceJoin(VoiceJoin {
                media_port: self.cfg.media_port,
            })
        } else {
            ControlMsg::VoiceLeave(VoiceLeave {})
        };
        let cmd = match naar {
            Some(to) => MeshCommand::Send { to, msg },
            None => MeshCommand::Broadcast(msg),
        };
        if let Err(e) = self.mesh.commands.try_send(cmd) {
            tracing::warn!(error = %e, "voice-status niet verstuurd");
        }
    }

    /// Alleen peers die én verbonden zijn én meedoen krijgen onze audio.
    fn werk_voice_peers_bij(&mut self) {
        let Some(voice) = &self.voice else { return };
        let doelen: Vec<PeerAdres> = self
            .peers_in_voice
            .iter()
            .filter_map(|id| {
                self.verbonden.get(id).map(|addr| PeerAdres {
                    id: *id,
                    addr: *addr,
                })
            })
            .collect();
        tracing::debug!(aantal = doelen.len(), "voice-deelnemers bijgewerkt");
        voice.zet_peers(doelen);
    }

    // -- UI ----------------------------------------------------------------

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
            UiCommand::VoiceDeelnemen => self.deelnemen(),
            UiCommand::VoiceVerlaten => self.verlaten(),
            UiCommand::Mute(aan) => {
                if let Some(v) = &self.voice {
                    v.zet_mute(aan);
                }
            }
            UiCommand::Deafen(aan) => {
                if let Some(v) = &self.voice {
                    v.zet_deafen(aan);
                }
            }
            UiCommand::Volume(peer, vol) => {
                if let Some(v) = &self.voice {
                    v.zet_volume(peer, vol);
                }
                if let Some(p) = self.peers.iter_mut().find(|p| p.peer_id == Some(peer)) {
                    p.volume = vol;
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

    fn publiceer(&mut self) {
        let niveaus = self.voice.as_ref().map(|v| v.niveaus()).unwrap_or_default();

        let mut peers = self.peers.clone();
        for p in &mut peers {
            if let Some(id) = p.peer_id {
                p.in_voice = self.peers_in_voice.contains(&id);
                p.niveau = niveaus.per_peer.get(&id).copied().unwrap_or(0.0);
            }
        }

        let voice = VoiceView {
            actief: self.voice.is_some(),
            muted: self.voice.as_ref().is_some_and(|v| v.is_mute()),
            deafened: self.voice.as_ref().is_some_and(|v| v.is_deafen()),
            eigen_niveau: niveaus.eigen,
        };

        let _ = self.snap_tx.send(Arc::new(Snapshot {
            timeline: self.chat.timeline_arc(),
            peers,
            voice,
            ongelezen: self.chat.ongelezen,
            fout: self.fout.clone(),
        }));
    }
}

/// Namen van de beschikbare apparaten, voor het instellingenscherm.
pub fn audio_apparaten() -> Result<(Vec<String>, Vec<String>)> {
    fitcom_audio::session::apparaatnamen().context("geluidsapparaten opvragen")
}
