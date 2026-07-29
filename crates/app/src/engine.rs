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
use crate::config::{Config, VideoConfig};
use crate::notify;
use crate::streams::{Actie, Streams};
use anyhow::{Context, Result};
use fitcom_audio::{PeerAdres, VoiceConfig, VoiceHandle};
use fitcom_net::{MeshCommand, MeshEvent, MeshHandle, PeerStatus};
use fitcom_proto::control::{StreamKind, VoiceJoin, VoiceLeave};
use fitcom_proto::{ControlMsg, OpId, PeerId};
use fitcom_store::{Store, Timeline};
use fitcom_video::{Bron, BronSoort, Codec, D3dContext, DelerConfig, DelerHandle};
use fitcom_video::{KijkerConfig, KijkerEvent, KijkerHandle};
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

/// Een bron die wij delen, zoals de UI hem toont.
#[derive(Debug, Clone)]
pub struct EigenStreamView {
    pub stream_id: u32,
    pub titel: String,
    pub kijkers: usize,
    pub is_geluid: bool,
}

/// Een bron die een ander deelt.
#[derive(Debug, Clone)]
pub struct StreamView {
    pub eigenaar: PeerId,
    pub stream_id: u32,
    pub titel: String,
    pub breedte: u32,
    pub hoogte: u32,
    pub kijken: bool,
    /// Geluid in plaats van beeld: geen venster, wel een volumeschuif.
    pub is_geluid: bool,
    pub volume: f32,
}

/// Alles wat de UI nodig heeft om zichzelf te tekenen.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub timeline: Arc<Timeline>,
    pub peers: Vec<PeerView>,
    pub voice: VoiceView,
    pub eigen_streams: Vec<EigenStreamView>,
    pub streams: Vec<StreamView>,
    pub video: VideoConfig,
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
    /// Een scherm of venster gaan delen. Er wordt nog niets opgenomen.
    DeelBron(Bron),
    /// Het geluid van deze PC meesturen. Vereist dat je in het gesprek zit.
    DeelBureaubladgeluid,
    StopDelen(u32),
    Kijken(PeerId, u32),
    StopKijken(PeerId, u32),
    /// Volume van één stream van een peer, los van zijn stem.
    StreamVolume(PeerId, u32, f32),
    /// Codec, framerate en bitrate voor screenshare. Geldt voor delers die al lopen
    /// meteen mee — die worden herstart met de nieuwe instellingen — en voor nieuw
    /// gestarte bronnen vanzelf, want die lezen `cfg.video` bij het starten.
    ZetVideoInstellingen(VideoConfig),
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
        streams: Streams::new(),
        d3d: None,
        bronnen: HashMap::new(),
        delers: HashMap::new(),
        kijkers: HashMap::new(),
        stream_volumes: HashMap::new(),
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

    /// Wie deelt wat en wie kijkt waarnaar. Neemt alle beslissingen; hieronder staat
    /// alleen het uitvoeren ervan.
    streams: Streams,
    /// Pas aangemaakt bij het eerste gebruik: op een machine die nooit deelt of kijkt
    /// hoeft er geen D3D11-apparaat te bestaan.
    d3d: Option<D3dContext>,
    /// De bron per eigen stream. Nodig omdat we de opname pas starten bij de eerste
    /// kijker, en dan moeten weten wát we ook alweer aankondigden.
    bronnen: HashMap<u32, Bron>,
    delers: HashMap<u32, DelerHandle>,
    kijkers: HashMap<(PeerId, u32), KijkerHandle>,
    /// Volume per bron, ook als de voice-sessie even niet draait. Zo blijft een
    /// zachtgezette stream zacht als je het gesprek verlaat en weer aansluit.
    stream_volumes: HashMap<(PeerId, u32), f32>,

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
                    self.lees_kijkers();
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
                            let id = *peer_id;
                            let r = self.chat.bij_verbinding(id);
                            self.verwerk(r);
                            // Zit ik al in het gesprek, dan moet deze peer dat weten;
                            // hij heeft mijn eerdere melding gemist. Hetzelfde geldt
                            // voor wat ik deel.
                            if self.voice.is_some() {
                                self.meld_voice_status(Some(id));
                            }
                            let cmds = self.streams.bij_verbinding(id);
                            self.stuur_alles(cmds);
                        }
                    }
                    _ => {
                        if let Some(id) = self.peers.get(target).and_then(|p| p.peer_id) {
                            self.verbonden.remove(&id);
                            // Weg is weg: uit het gesprek halen, anders blijven we
                            // audio naar een dood adres sturen.
                            self.peers_in_voice.remove(&id);
                            let acties = self.streams.bij_verbreking(id);
                            self.voer_uit(acties);
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
                    // Screenshare eerst: `bij_bericht` van de chat laat alles wat niet
                    // van hem is ongemoeid, en andersom net zo.
                    if let Some(ip) = self.verbonden.get(&from).map(|a| a.ip()) {
                        let (cmds, acties) = self.streams.bij_bericht(from, ip, &andere);
                        self.stuur_alles(cmds);
                        self.voer_uit(acties);
                    }

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
        if self.voice.take().is_none() {
            return;
        }
        self.meld_voice_status(None);

        // Bureaubladgeluid loopt over de voice-socket, en die is er nu niet meer.
        // Zowel wat we deelden als waar we naar luisterden moet daarom weg — anders
        // blijven de anderen naar een dood adres sturen en denkt de UI dat het werkt.
        let eigen: Vec<u32> = self
            .streams
            .eigen()
            .iter()
            .filter(|s| s.kind == StreamKind::DESKTOP_AUDIO)
            .map(|s| s.id)
            .collect();
        for id in eigen {
            let (cmds, acties) = self.streams.stop_delen(id);
            self.stuur_alles(cmds);
            self.voer_uit(acties);
        }

        let geluid: Vec<(PeerId, u32)> = self
            .streams
            .vreemd()
            .iter()
            .filter(|s| s.kijken && s.kind == StreamKind::DESKTOP_AUDIO)
            .map(|s| (s.eigenaar, s.id))
            .collect();
        for (eigenaar, id) in geluid {
            let (cmds, acties) = self.streams.stop_kijken(eigenaar, id);
            self.stuur_alles(cmds);
            self.voer_uit(acties);
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

    // -- screenshare -------------------------------------------------------

    /// Het D3D11-apparaat, aangemaakt bij het eerste gebruik. Deelt en kijkt hetzelfde
    /// apparaat, zodat een textuur nergens tussen apparaten hoeft te reizen.
    fn d3d(&mut self) -> Result<D3dContext> {
        if let Some(d) = &self.d3d {
            return Ok(d.clone());
        }
        let d = D3dContext::new().context("grafische kaart openen")?;
        self.d3d = Some(d.clone());
        Ok(d)
    }

    fn codec(&self) -> Codec {
        Codec::van_naam(&self.cfg.video.codec).unwrap_or_else(|| {
            tracing::warn!(gekozen = %self.cfg.video.codec, "onbekende codec in de config; h264 gebruikt");
            Codec::H264
        })
    }

    /// Een bron aankondigen. Er wordt nog niets opgenomen — dat is het hele punt.
    fn deel_bron(&mut self, bron: Bron) {
        let afmeting = match fitcom_video::capture::afmeting_van(&bron) {
            Ok(a) => a,
            Err(e) => {
                tracing::error!(error = %format!("{e:#}"), "bron niet te openen");
                self.fout = Some(format!("bron niet te openen: {e:#}"));
                return;
            }
        };

        let kind = match bron.soort {
            BronSoort::Monitor => StreamKind::MONITOR,
            BronSoort::Venster => StreamKind::WINDOW,
        };
        let (id, cmds) = self
            .streams
            .deel(kind, bron.naam.clone(), afmeting.0, afmeting.1);
        self.bronnen.insert(id, bron);
        self.stuur_alles(cmds);
    }

    /// Of dit een van onze streams is die geluid draagt in plaats van beeld.
    ///
    /// `Actie` zegt dat niet zelf: het is een eigenschap van de stream, niet van de
    /// beslissing, en die staat al in `streams`.
    fn is_geluid(&self, stream_id: u32) -> bool {
        self.streams
            .eigen()
            .iter()
            .any(|s| s.id == stream_id && s.kind == StreamKind::DESKTOP_AUDIO)
    }

    /// Het geluid van deze PC aankondigen. Net als bij een scherm wordt er nog niets
    /// opgenomen: dat begint pas als er iemand meeluistert.
    fn deel_bureaubladgeluid(&mut self) {
        if self.voice.is_none() {
            self.fout =
                Some("neem eerst deel aan het gesprek; bureaubladgeluid gaat daarover mee".into());
            return;
        }
        if self
            .streams
            .eigen()
            .iter()
            .any(|s| s.kind == StreamKind::DESKTOP_AUDIO)
        {
            return; // delen we al
        }
        // Geen afmeting: dit is geluid. De ontvanger leidt daar niets uit af — hij
        // kijkt naar `kind`.
        let (_, cmds) =
            self.streams
                .deel(StreamKind::DESKTOP_AUDIO, "Bureaubladgeluid".into(), 0, 0);
        self.stuur_alles(cmds);
    }

    /// Voert uit wat `streams` besloten heeft. Elke fout hierin is een fout in het
    /// uitvoeren, niet in de beslissing: de toestand blijft kloppen.
    fn voer_uit(&mut self, acties: Vec<Actie>) {
        for actie in acties {
            match actie {
                Actie::StartDelen { stream_id, kijkers }
                | Actie::ZetKijkers { stream_id, kijkers }
                    if self.is_geluid(stream_id) =>
                {
                    // Bureaubladgeluid gaat over de voice-socket mee, dus opnieuw
                    // aanzetten met een andere lijst luisteraars is hetzelfde als
                    // starten. De sessie zelf is idempotent.
                    if let Some(v) = &self.voice {
                        if let Err(e) = v.deel_bureaublad(stream_id, kijkers) {
                            tracing::error!(error = %format!("{e:#}"), "bureaubladgeluid delen mislukt");
                            self.fout = Some(format!("bureaubladgeluid: {e:#}"));
                        }
                    }
                }
                Actie::StartDelen { stream_id, kijkers } => {
                    if let Err(e) = self.start_deler(stream_id, kijkers) {
                        tracing::error!(error = %format!("{e:#}"), stream = stream_id, "delen starten mislukt");
                        self.fout = Some(format!("scherm delen: {e:#}"));
                    }
                }
                Actie::ZetKijkers { stream_id, kijkers } => {
                    if let Some(d) = self.delers.get(&stream_id) {
                        d.zet_kijkers(kijkers);
                    }
                }
                Actie::StopDelen { stream_id } => {
                    if self.is_geluid(stream_id) {
                        if let Some(v) = &self.voice {
                            v.stop_bureaublad();
                        }
                    }
                    self.delers.remove(&stream_id);
                }
                Actie::StuurKeyframe { stream_id } => {
                    if let Some(d) = self.delers.get(&stream_id) {
                        d.vraag_keyframe();
                    }
                }
                Actie::StartKijken {
                    eigenaar,
                    stream_id,
                    titel,
                    breedte,
                    hoogte,
                    is_geluid,
                } => {
                    let uitkomst = if is_geluid {
                        self.luister_mee(eigenaar, stream_id)
                    } else {
                        self.start_kijker(eigenaar, stream_id, titel, breedte, hoogte)
                    };
                    if let Err(e) = uitkomst {
                        tracing::error!(error = %format!("{e:#}"), "kijken starten mislukt");
                        self.fout = Some(format!("{e:#}"));
                        // De beslissing terugdraaien, anders denkt de UI dat we kijken.
                        let (cmds, _) = self.streams.stop_kijken(eigenaar, stream_id);
                        self.stuur_alles(cmds);
                    }
                }
                Actie::StopKijken {
                    eigenaar,
                    stream_id,
                } => {
                    self.kijkers.remove(&(eigenaar, stream_id));
                    if let Some(v) = &self.voice {
                        v.vergeet_bron((eigenaar, stream_id));
                    }
                }
            }
        }
    }

    /// Intekenen op andermans bureaubladgeluid. Er komt geen venster en geen eigen
    /// thread aan te pas: het komt binnen op de voice-poort en de mixer die daar al
    /// draait telt het er gewoon bij op.
    fn luister_mee(&mut self, eigenaar: PeerId, stream_id: u32) -> Result<()> {
        let poort = self
            .voice
            .as_ref()
            .map(|v| v.media_port())
            .context("neem eerst deel aan het gesprek om meegedeeld geluid te horen")?;
        let cmds = self.streams.kijker_draait(eigenaar, stream_id, poort);
        self.stuur_alles(cmds);
        Ok(())
    }

    fn start_deler(&mut self, stream_id: u32, kijkers: Vec<SocketAddr>) -> Result<()> {
        let bron = self
            .bronnen
            .get(&stream_id)
            .cloned()
            .context("bron van deze stream is verdwenen")?;
        let d3d = self.d3d()?;
        let handle = fitcom_video::deel(
            &d3d,
            DelerConfig {
                stream_id,
                bron,
                codec: self.codec(),
                fps: self.cfg.video.fps,
                bitrate: self.cfg.video.bitrate,
            },
            kijkers,
        )?;
        self.delers.insert(stream_id, handle);
        Ok(())
    }

    /// Herstart elke lopende deler met de huidige `cfg.video`. Wordt aangeroepen na een
    /// instellingenwijziging: zonder dit zou een nieuwe bitrate of codec pas gaan gelden
    /// bij de volgende keer delen, en dat is niet wat "instellingen aanpassen" belooft.
    ///
    /// Bureaubladgeluid draait niet via `delers` maar via de voice-sessie, en heeft geen
    /// codec-instelling — die blijft dus vanzelf buiten schot.
    fn herstart_lopende_delers(&mut self) {
        let actief: Vec<(u32, Vec<SocketAddr>)> = self
            .streams
            .eigen()
            .iter()
            .filter(|s| self.delers.contains_key(&s.id))
            .map(|s| (s.id, s.kijkers.values().copied().collect()))
            .collect();
        for (id, kijkers) in actief {
            if let Err(e) = self.start_deler(id, kijkers) {
                tracing::error!(error = %format!("{e:#}"), stream = id, "deler herstarten na instellingenwijziging mislukt");
                self.fout = Some(format!("scherm delen: {e:#}"));
            }
        }
    }

    fn start_kijker(
        &mut self,
        eigenaar: PeerId,
        stream_id: u32,
        titel: String,
        breedte: u32,
        hoogte: u32,
    ) -> Result<()> {
        let ip = self
            .verbonden
            .get(&eigenaar)
            .map(|a| a.ip())
            .context("die peer is niet verbonden")?;
        let d3d = self.d3d()?;
        let naam = self
            .peers
            .iter()
            .find(|p| p.peer_id == Some(eigenaar))
            .map(|p| p.label.clone())
            .unwrap_or_else(|| "peer".into());

        let handle = fitcom_video::kijk(
            &d3d,
            KijkerConfig {
                stream_id,
                titel: format!("{naam} — {titel}"),
                breedte,
                hoogte,
                codec: self.codec(),
                afzender: ip,
            },
        )?;

        // Nu pas intekenen: in het abonnement staat de poort van dit venster, en die
        // bestaat pas als het venster er is.
        let cmds = self
            .streams
            .kijker_draait(eigenaar, stream_id, handle.poort);
        self.stuur_alles(cmds);
        self.kijkers.insert((eigenaar, stream_id), handle);
        Ok(())
    }

    /// Haalt op wat de kijkvensters te melden hebben: gesloten vensters en verzoeken
    /// om een keyframe. Gebeurt op de tik, want de motor mag hier niet op wachten.
    fn lees_kijkers(&mut self) {
        let mut gesloten = Vec::new();
        let mut keyframes = Vec::new();

        for (&(eigenaar, stream_id), kijker) in &self.kijkers {
            while let Ok(ev) = kijker.events.try_recv() {
                match ev {
                    KijkerEvent::Gesloten => gesloten.push((eigenaar, stream_id)),
                    KijkerEvent::KeyframeNodig => keyframes.push((eigenaar, stream_id)),
                }
            }
        }

        for (eigenaar, stream_id) in keyframes {
            let cmds = self.streams.vraag_keyframe(eigenaar, stream_id);
            self.stuur_alles(cmds);
        }
        for (eigenaar, stream_id) in gesloten {
            let (cmds, acties) = self.streams.stop_kijken(eigenaar, stream_id);
            self.stuur_alles(cmds);
            self.voer_uit(acties);
        }
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
            UiCommand::DeelBron(bron) => self.deel_bron(bron),
            UiCommand::DeelBureaubladgeluid => self.deel_bureaubladgeluid(),
            UiCommand::StreamVolume(peer, id, vol) => {
                self.stream_volumes.insert((peer, id), vol);
                if let Some(v) = &self.voice {
                    v.zet_bron_volume((peer, id), vol);
                }
            }
            UiCommand::ZetVideoInstellingen(video) => {
                self.cfg.video = video;
                if let Err(e) = self.cfg.save(&self.config_path) {
                    tracing::warn!(error = %format!("{e:#}"), "video-instellingen niet opgeslagen");
                    self.fout = Some(format!("instellingen opslaan: {e:#}"));
                }
                self.herstart_lopende_delers();
            }
            UiCommand::StopDelen(id) => {
                let (cmds, acties) = self.streams.stop_delen(id);
                self.bronnen.remove(&id);
                self.stuur_alles(cmds);
                self.voer_uit(acties);
            }
            UiCommand::Kijken(eigenaar, id) => {
                let acties = self.streams.wil_kijken(eigenaar, id);
                self.voer_uit(acties);
            }
            UiCommand::StopKijken(eigenaar, id) => {
                let (cmds, acties) = self.streams.stop_kijken(eigenaar, id);
                self.stuur_alles(cmds);
                self.voer_uit(acties);
            }
        }
    }

    fn stuur_alles(&mut self, cmds: Vec<MeshCommand>) {
        for cmd in cmds {
            if let Err(e) = self.mesh.commands.try_send(cmd) {
                tracing::warn!(error = %e, "netwerkcommando niet verstuurd");
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
            Ok(cmds) => self.stuur_alles(cmds),
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

        let eigen_streams = self
            .streams
            .eigen()
            .iter()
            .map(|s| EigenStreamView {
                stream_id: s.id,
                titel: s.titel.clone(),
                kijkers: s.kijkers.len(),
                is_geluid: s.kind == StreamKind::DESKTOP_AUDIO,
            })
            .collect();

        let streams = self
            .streams
            .vreemd()
            .iter()
            .map(|s| StreamView {
                eigenaar: s.eigenaar,
                stream_id: s.id,
                titel: s.titel.clone(),
                breedte: s.breedte,
                hoogte: s.hoogte,
                kijken: s.kijken,
                is_geluid: s.kind == StreamKind::DESKTOP_AUDIO,
                volume: self
                    .stream_volumes
                    .get(&(s.eigenaar, s.id))
                    .copied()
                    .unwrap_or(1.0),
            })
            .collect();

        let _ = self.snap_tx.send(Arc::new(Snapshot {
            timeline: self.chat.timeline_arc(),
            peers,
            voice,
            eigen_streams,
            streams,
            video: self.cfg.video.clone(),
            ongelezen: self.chat.ongelezen,
            fout: self.fout.clone(),
        }));
    }
}

/// Namen van de beschikbare apparaten, voor het instellingenscherm.
pub fn audio_apparaten() -> Result<(Vec<String>, Vec<String>)> {
    fitcom_audio::session::apparaatnamen().context("geluidsapparaten opvragen")
}

/// Schermen en vensters die te delen zijn. Wordt bij het openen van het keuzemenu
/// opgevraagd; vensters komen en gaan, dus een eenmalige lijst zou snel verouderen.
pub fn deelbare_bronnen() -> Result<Vec<Bron>> {
    fitcom_video::beschikbare_bronnen().context("deelbare bronnen opvragen")
}
