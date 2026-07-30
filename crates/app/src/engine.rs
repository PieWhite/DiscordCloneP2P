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
use crate::config::{self, Config, VideoConfig};
use crate::files::{self, DownloadStatus, Files, StartUpload};
use crate::notify;
use crate::streams::{Actie, Streams};
use crate::tags;
use anyhow::{Context, Result};
use fitcom_audio::{PeerAdres, VoiceConfig, VoiceHandle};
use fitcom_net::{MeshCommand, MeshEvent, MeshHandle, PeerStatus, RecvStream, SendStream};
use fitcom_proto::control::{StreamKind, VoiceJoin, VoiceLeave};
use fitcom_proto::{Channel, ControlMsg, OpId, OpKind, PeerId, TopicId};
use fitcom_store::{FileEntry, Store, Timeline};
use fitcom_video::{Bron, BronSoort, Codec, D3dContext, DelerConfig, DelerHandle};
use fitcom_video::{KijkerConfig, KijkerEvent, KijkerHandle, Miniatuur};
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot, watch};

/// Snel genoeg voor een spreekindicatie die niet hakkelt. Een momentopname is goedkoop:
/// de timeline zit erin als `Arc`, dus publiceren kost geen kopie van de geschiedenis.
const TIK: Duration = Duration::from_millis(100);

/// Hoe vaak een lopende download zijn voortgang naar de UI duwt. Bij 1 Gbit komen er
/// per seconde veel te veel gelezen stukjes langs om elk apart te melden.
const VOORTGANG_INTERVAL: Duration = Duration::from_millis(200);

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
    /// Verkleind beeld voor het overzicht in het hoofdvenster. `None` tot het eerste
    /// beeld binnen is, en altijd `None` voor geluid.
    pub miniatuur: Option<Miniatuur>,
}

/// Een aangeboden bestand zoals de UI het toont: de metadata uit de oplog plus, als wij
/// er iets mee doen, onze eigen downloadstatus.
#[derive(Debug, Clone)]
pub struct FileView {
    pub id: OpId,
    pub author: PeerId,
    /// Algemeen kanaal of een DM — bepaalt of dit bestand in de algemene lijst of in
    /// een gespreksvenster hoort.
    pub channel: Channel,
    pub name: String,
    pub size: u64,
    /// Ons eigen aanbod: geen downloadknop nodig, we hebben het al.
    pub is_mine: bool,
    /// `None` betekent: nog niet gedownload en niet mee bezig.
    pub status: Option<DownloadStatus>,
    /// Zie `fitcom_store::timeline::Message::lamport` — bepaalt waar dit bestand tussen
    /// de berichten in de tijdlijn komt te staan.
    pub lamport: u64,
    /// Voor een afbeelding: samen met `name`'s extensie het pad in `pictures_dir` waar
    /// de UI een miniatuur vandaan kan laden, zie `files::hash_bestandsnaam`.
    pub hash: [u8; 32],
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
    pub files: Vec<FileView>,
    pub ongelezen: usize,
    /// Ongelezen DM-berichten per gesprekspartner. Los van `ongelezen` (het algemene
    /// kanaal) — je kunt het een missen zonder het ander te missen.
    pub ongelezen_dm: HashMap<PeerId, usize>,
    /// Ongelezen berichten per subkanaal onder het algemene kanaal (fase 9). Los van
    /// zowel `ongelezen` als `ongelezen_dm`.
    pub ongelezen_topic: HashMap<TopicId, usize>,
    /// Onderdrukt alle Windows-meldingen, ook een directe tag naar jezelf. Geldt alleen
    /// voor deze sessie, net als mute/deafen — geen configvermelding.
    pub niet_storen: bool,
    pub fout: Option<String>,
}

#[derive(Debug)]
pub enum UiCommand {
    /// Kanaal erbij: het algemene kanaal, of een DM met de gekozen peer.
    Plaats(String, Channel),
    Bewerk(OpId, String),
    Verwijder(OpId),
    Gelezen,
    /// Een DM-gesprek is bekeken; telt niet mee voor `Snapshot::ongelezen_dm`.
    GelezenDm(PeerId),
    /// Een subkanaal is bekeken; telt niet mee voor `Snapshot::ongelezen_topic`.
    GelezenTopic(TopicId),
    FoutWeg,
    VoiceDeelnemen,
    VoiceVerlaten,
    Mute(bool),
    Deafen(bool),
    Volume(PeerId, f32),
    /// Een scherm of venster gaan delen. Er wordt nog niets opgenomen. Bureaubladgeluid
    /// gaat vanzelf mee als je in het gesprek zit — geen apart commando meer (fase 10).
    DeelBron(Bron),
    StopDelen(u32),
    Kijken(PeerId, u32),
    StopKijken(PeerId, u32),
    /// Volume van één stream van een peer, los van zijn stem.
    StreamVolume(PeerId, u32, f32),
    /// Codec, framerate en bitrate voor screenshare. Geldt voor delers die al lopen
    /// meteen mee — die worden herstart met de nieuwe instellingen — en voor nieuw
    /// gestarte bronnen vanzelf, want die lezen `cfg.video` bij het starten.
    ZetVideoInstellingen(VideoConfig),
    /// Een lokaal bestand kiezen en aanbieden aan de anderen (of aan één DM-partner).
    /// Hashen gebeurt op de motor: bij een groot bestand kan dat te lang duren om de UI
    /// op te laten wachten.
    BiedBestandAan(PathBuf, Channel),
    /// Downloaden, of hervatten na een eerdere onderbreking.
    DownloadBestand(OpId),
    /// Eigen weergavenaam wijzigen. Legt een `SetNick`-op vast zodra de motor hem
    /// verwerkt, net als bij het opstarten.
    ZetNaam(String),
    /// Niet-storenmodus aan/uit. Onderdrukt alle Windows-meldingen, ook een tag.
    NietStoren(bool),
    /// Wist alle bestanden in `pictures_dir` van schijf. Raakt alleen lokale
    /// schijfruimte — de kaarten blijven in de tijdlijn staan, net als bij een
    /// bronbestand dat toevallig van schijf verdwijnt (zie `files.rs`).
    VerwijderAlleAfbeeldingen,
    /// Nieuw subkanaal onder het algemene kanaal aanmaken, met een titel.
    MaakKanaal(String),
    /// Bestaand subkanaal hernoemen. Zelfde mechanisme als aanmaken — zie
    /// `Chat::zet_kanaal_titel`.
    HernoemKanaal(TopicId, String),
    /// Subkanaal verwijderen. UI vraagt hier eerst een bevestiging voor.
    VerwijderKanaal(TopicId),
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
    let (file_tx, file_rx) = mpsc::channel(64);

    // `config_path` staat altijd direct in de datamap (zie `main.rs`), dus dat is ook de
    // basis voor de standaard downloadmap zolang de gebruiker er niets voor gekozen heeft.
    let data_dir = config_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let downloads_dir = config::resolve_download_dir(&cfg, &data_dir);
    std::fs::create_dir_all(&downloads_dir).context("downloadmap aanmaken")?;
    let pictures_dir = config::resolve_pictures_dir(&data_dir);
    std::fs::create_dir_all(&pictures_dir).context("picturesmap aanmaken")?;

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
        miniaturen: HashMap::new(),
        stream_volumes: HashMap::new(),
        files: Files::new(),
        downloads_dir,
        pictures_dir,
        file_tx,
        file_rx,
        fout: None,
        niet_storen: false,
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
    /// Laatste miniatuur per bekeken stream, voor het overzicht in het hoofdvenster.
    /// Blijft hier staan in plaats van in `Streams`, want dat is pure beslislogica en
    /// dit is een bijproduct van het decoderen.
    miniaturen: HashMap<(PeerId, u32), Miniatuur>,
    /// Volume per bron, ook als de voice-sessie even niet draait. Zo blijft een
    /// zachtgezette stream zacht als je het gesprek verlaat en weer aansluit.
    stream_volumes: HashMap<(PeerId, u32), f32>,

    /// Wie wat aanbiedt en waar onze eigen downloads staan. Neemt de beslissingen;
    /// het lezen/schrijven/hashen zelf gebeurt in losse tokio-taken hieronder.
    files: Files,
    downloads_dir: PathBuf,
    /// Content-adresseerbare map voor afbeeldingen: `<hash-hex>.<ext>`, zowel voor wat
    /// wij aanbieden als voor wat we downloaden. Zie `files::hash_bestandsnaam`.
    pictures_dir: PathBuf,
    /// Voor het klonen naar bestandstaken toe; `file_rx` is waar de motor op wacht.
    file_tx: mpsc::Sender<FileEvent>,
    file_rx: mpsc::Receiver<FileEvent>,

    fout: Option<String>,
    /// Zie `Snapshot::niet_storen`.
    niet_storen: bool,
    snap_tx: watch::Sender<Arc<Snapshot>>,
    voorgrond: Arc<AtomicBool>,
}

/// Wat een bestandstaak (hashen, uploaden, downloaden) terugmeldt aan de motor. Losse
/// tokio-taken raken schijf en netwerk aan; de motor zelf blijft daar los van, net als
/// bij de kijker- en delerthreads van screenshare.
enum FileEvent {
    /// Het lokale bestand is gelezen en gehasht; nu kan het als op vastgelegd worden.
    NieuwAanbod {
        pad: PathBuf,
        naam: String,
        grootte: u64,
        hash: [u8; 32],
        channel: Channel,
    },
    Voortgang {
        file: OpId,
        ontvangen: u64,
    },
    Voltooid {
        file: OpId,
    },
    Mislukt {
        file: OpId,
        bericht: String,
    },
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
                // `None` kan niet gebeuren zolang de motor zelf `file_tx` vasthoudt.
                ev = self.file_rx.recv() => if let Some(ev) = ev {
                    self.op_file_event(ev);
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

            // Een bulkoverdracht, los van de control-stream. De header in de stream zelf
            // zegt om welk bestand het gaat; `from` is hier niet nodig om hem te routeren.
            MeshEvent::IncomingFileStream { from: _, stream } => {
                self.start_download(stream);
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
                ControlMsg::FileRequest(req) => {
                    let (cmd, upload) = self.files.verzoek_ontvangen(from, &req);
                    self.stuur_alles(vec![cmd]);
                    if let Some(actie) = upload {
                        self.start_upload(actie);
                    }
                }
                ControlMsg::FileResponse(resp) => {
                    self.files.antwoord_ontvangen(&resp);
                }
                andere => {
                    // Screenshare eerst: `bij_bericht` van de chat laat alles wat niet
                    // van hem is ongemoeid, en andersom net zo.
                    if let Some(ip) = self.verbonden.get(&from).map(|a| a.ip()) {
                        let (cmds, acties) = self.streams.bij_bericht(from, ip, &andere);
                        self.stuur_alles(cmds);
                        self.voer_uit(acties);
                    }

                    // Alleen een live `OpBroadcast` van een `Post` komt in aanmerking
                    // voor een melding. Een `SyncResponse` is per definitie een
                    // inhaalslag van gemiste geschiedenis (zie docs/OVERDRACHT.md) en
                    // meldt daarom nooit, ongeacht de inhoud — dat onderscheid zit al
                    // in het berichttype, dus zonder aparte "sync klaar"-status.
                    let live_post = match &andere {
                        ControlMsg::OpBroadcast(b) => match b.op.kind() {
                            Ok(Some(OpKind::Post { body })) => Some((b.op.channel, body)),
                            _ => None,
                        },
                        _ => None,
                    };
                    let voor_alg = self.chat.ongelezen;
                    let voor_dm = self.chat.ongelezen_dm().get(&from).copied().unwrap_or(0);
                    let voor_topic = live_post
                        .as_ref()
                        .and_then(|(c, _)| c.topic_id())
                        .map(|t| self.chat.ongelezen_topic().get(&t).copied().unwrap_or(0));

                    // Vóór `bij_bericht`: welke afbeeldingen kennen we al, zodat we straks
                    // alleen de écht nieuwe kunnen herkennen (zie de auto-download hieronder).
                    let bekende_afbeeldingen: HashSet<OpId> = self
                        .chat
                        .timeline()
                        .files
                        .iter()
                        .filter(|f| files::is_afbeelding(&f.name))
                        .map(|f| f.id)
                        .collect();

                    let r = self.chat.bij_bericht(from, andere);
                    self.verwerk(r);
                    // Nieuwe bestanden zitten pas in de timeline ná een refresh — die
                    // gebeurt normaal aan het eind van de hoofdlus, maar de auto-download
                    // hieronder heeft de bijgewerkte lijst nu al nodig.
                    self.chat.refresh();

                    if let Some((channel, body)) = live_post {
                        // Pas ná `bij_bericht` weten we of dit echt nieuw was: een
                        // dubbel bezorgde broadcast mag geen tweede melding geven.
                        let nieuw = if let Some(topic) = channel.topic_id() {
                            self.chat
                                .ongelezen_topic()
                                .get(&topic)
                                .copied()
                                .unwrap_or(0)
                                > voor_topic.unwrap_or(0)
                        } else if channel.is_general() {
                            self.chat.ongelezen > voor_alg
                        } else {
                            self.chat.ongelezen_dm().get(&from).copied().unwrap_or(0) > voor_dm
                        };
                        if nieuw {
                            self.overweeg_melding(from, &body);
                        }
                    }

                    // Afbeeldingen downloaden zichzelf automatisch, voor iedereen — live
                    // binnengekomen én ingehaald bij (her)verbinding. Andere bestanden
                    // blijven achter de bevestigingswal (de downloadknop) staan, precies
                    // zoals dat al was. Eigen aanbod slaan we over: dat staat al op schijf
                    // via `hash_en_bied_aan`, en downloaden zou alleen jezelf aanvragen.
                    let me = self.chat.me();
                    let nieuwe_afbeeldingen: Vec<OpId> = self
                        .chat
                        .timeline()
                        .files
                        .iter()
                        .filter(|f| {
                            f.author != me
                                && !bekende_afbeeldingen.contains(&f.id)
                                && files::is_afbeelding(&f.name)
                        })
                        .map(|f| f.id)
                        .collect();
                    for id in nieuwe_afbeeldingen {
                        if self.files.status(id).is_none() {
                            self.download_bestand(id);
                        }
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
                // Deelde je al een scherm vóór je het gesprek in kwam, dan hoort het
                // geluid er nu alsnog automatisch bij (fase 10).
                if self.deelt_scherm_of_venster() {
                    self.deel_bureaubladgeluid();
                }
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

        // Fase 10: geen losse knop meer, geluid van deze pc gaat automatisch mee zodra
        // er iets gedeeld wordt. `deel_bureaubladgeluid` is zelf al idempotent, dus dit
        // mag ook als er al een tweede scherm bij komt.
        self.deel_bureaubladgeluid();
    }

    /// Of we op dit moment een monitor of venster delen — dus of bureaubladgeluid mee
    /// hoort te lopen.
    fn deelt_scherm_of_venster(&self) -> bool {
        self.streams
            .eigen()
            .iter()
            .any(|s| s.kind == StreamKind::MONITOR || s.kind == StreamKind::WINDOW)
    }

    /// Stopt bureaubladgeluid omdat het laatste gedeelde scherm net gestopt is. Geen-op
    /// als er toch niets gedeeld wordt.
    fn stop_bureaubladgeluid(&mut self) {
        let Some(id) = self
            .streams
            .eigen()
            .iter()
            .find(|s| s.kind == StreamKind::DESKTOP_AUDIO)
            .map(|s| s.id)
        else {
            return;
        };
        let (cmds, acties) = self.streams.stop_delen(id);
        self.stuur_alles(cmds);
        self.voer_uit(acties);
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
    /// opgenomen: dat begint pas als er iemand meeluistert. Automatisch aangeroepen
    /// (fase 10), dus geen gesprek is normaal en geen fout — gewoon niets doen.
    fn deel_bureaubladgeluid(&mut self) {
        if self.voice.is_none() {
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
                Actie::StopDelen {
                    stream_id,
                    is_geluid,
                } => {
                    if is_geluid {
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
                    self.miniaturen.remove(&(eigenaar, stream_id));
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
                    KijkerEvent::Miniatuur(m) => {
                        self.miniaturen.insert((eigenaar, stream_id), m);
                    }
                }
            }
        }

        for (eigenaar, stream_id) in keyframes {
            let cmds = self.streams.vraag_keyframe(eigenaar, stream_id);
            self.stuur_alles(cmds);
        }
        for (eigenaar, stream_id) in gesloten {
            self.miniaturen.remove(&(eigenaar, stream_id));
            let (cmds, acties) = self.streams.stop_kijken(eigenaar, stream_id);
            self.stuur_alles(cmds);
            self.voer_uit(acties);
        }
    }

    // -- UI ----------------------------------------------------------------

    fn op_ui_command(&mut self, cmd: UiCommand) {
        match cmd {
            UiCommand::Plaats(tekst, channel) => {
                let r = self.chat.plaats_bericht(&tekst, channel);
                self.verwerk(r);
            }
            UiCommand::Bewerk(doel, tekst) => {
                let r = self.chat.bewerk_bericht(doel, &tekst);
                self.verwerk(r);
            }
            UiCommand::Verwijder(doel) => {
                // Generiek: `doel` kan een bericht zijn of een eigen bestandsaanbod. Is
                // het het laatste, dan moet `verzoek_ontvangen` het na deze klik ook
                // echt niet meer serveren — anders verdwijnt alleen de kaart uit de
                // tijdlijn terwijl het bestand nog gewoon downloadbaar blijft voor wie
                // de OpId al kende. Een no-op als `doel` geen eigen aanbod is.
                self.files.verwijder_aanbod(doel);
                let r = self.chat.verwijder_bericht(doel);
                self.verwerk(r);
            }
            UiCommand::Gelezen => self.chat.markeer_gelezen(),
            UiCommand::GelezenDm(peer) => self.chat.markeer_dm_gelezen(peer),
            UiCommand::GelezenTopic(topic) => self.chat.markeer_topic_gelezen(topic),
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
                let was_scherm = self.bronnen.remove(&id).is_some();
                let (cmds, acties) = self.streams.stop_delen(id);
                self.stuur_alles(cmds);
                self.voer_uit(acties);
                // Fase 10: het laatste scherm weg betekent ook het geluid weg, zonder
                // dat de gebruiker dat apart hoeft te doen.
                if was_scherm && !self.deelt_scherm_of_venster() {
                    self.stop_bureaubladgeluid();
                }
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
            UiCommand::BiedBestandAan(pad, channel) => {
                let tx = self.file_tx.clone();
                let pictures_dir = self.pictures_dir.clone();
                tokio::spawn(hash_en_bied_aan(pad, channel, pictures_dir, tx));
            }
            UiCommand::DownloadBestand(file) => self.download_bestand(file),
            UiCommand::ZetNaam(naam) => self.zet_naam(&naam),
            UiCommand::NietStoren(aan) => self.niet_storen = aan,
            UiCommand::VerwijderAlleAfbeeldingen => self.verwijder_alle_afbeeldingen(),
            UiCommand::MaakKanaal(titel) => {
                let r = self.chat.zet_kanaal_titel(TopicId::new_random(), &titel);
                self.verwerk(r);
            }
            UiCommand::HernoemKanaal(id, titel) => {
                let r = self.chat.zet_kanaal_titel(id, &titel);
                self.verwerk(r);
            }
            UiCommand::VerwijderKanaal(id) => {
                let r = self.chat.verwijder_kanaal(id);
                self.verwerk(r);
            }
        }
    }

    /// Leegt `pictures_dir`. Faalt een los bestand (bijvoorbeeld nog in gebruik), dan
    /// gaat de rest gewoon door — net als bij offline peers is dit geen foutpad om op
    /// vast te lopen. Een download of upload die net onderweg is naar/van een van deze
    /// paden krijgt de al bestaande "bronbestand verdwenen"-afhandeling; zie `files.rs`.
    fn verwijder_alle_afbeeldingen(&mut self) {
        let lezing = match std::fs::read_dir(&self.pictures_dir) {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!(error = %e, "picturesmap niet te lezen");
                return;
            }
        };
        let mut verwijderd = 0u32;
        for entry in lezing.flatten() {
            let pad = entry.path();
            if pad.is_file() {
                match std::fs::remove_file(&pad) {
                    Ok(()) => verwijderd += 1,
                    Err(e) => {
                        tracing::warn!(pad = %pad.display(), error = %e, "afbeelding niet te verwijderen")
                    }
                }
            }
        }
        tracing::info!(verwijderd, "picturesmap opgeschoond");
    }

    /// Legt een nieuwe weergavenaam vast: eerst in `config.toml` (zodat hij de volgende
    /// start meteen weer klopt), dan als `SetNick`-op zodat de andere peers hem zien.
    /// Een lege of ongewijzigde naam doet niets — `chat.zet_naam` dedupliceert dat laatste
    /// zelf al, dus de log groeit hier niet van bij een dubbelklik op "opslaan".
    fn zet_naam(&mut self, naam: &str) {
        let naam = naam.trim();
        if naam.is_empty() {
            return;
        }
        self.cfg.display_name = naam.to_string();
        if let Err(e) = self.cfg.save(&self.config_path) {
            tracing::warn!(error = %format!("{e:#}"), "naam niet opgeslagen in config");
            self.fout = Some(format!("naam opslaan: {e:#}"));
        }
        let r = self.chat.zet_naam(naam);
        self.verwerk(r);
    }

    /// Start (of hervat) een download. Het hervatpunt komt van wat er al op schijf staat
    /// van een eerdere, onderbroken poging.
    fn download_bestand(&mut self, file: OpId) {
        let Some(entry) = self
            .chat
            .timeline()
            .files
            .iter()
            .find(|f| f.id == file)
            .cloned()
        else {
            return;
        };
        if entry.author == self.chat.me() {
            return; // ons eigen aanbod; er valt niets te downloaden
        }

        let deelpad = self.downloads_dir.join(deelbestand_naam(&entry));
        let bestaand = std::fs::metadata(&deelpad).map(|m| m.len()).unwrap_or(0);

        let cmd = self.files.download_aanvragen(&entry, bestaand);
        self.stuur_alles(vec![cmd]);
    }

    fn start_upload(&mut self, actie: StartUpload) {
        let mesh_commands = self.mesh.commands.clone();
        tokio::spawn(upload_taak(
            mesh_commands,
            actie.naar,
            actie.file,
            actie.pad,
            actie.vanaf,
        ));
    }

    fn start_download(&mut self, stream: RecvStream) {
        let downloads_dir = self.downloads_dir.clone();
        let pictures_dir = self.pictures_dir.clone();
        let timeline = self.chat.timeline_arc();
        let events = self.file_tx.clone();
        tokio::spawn(download_taak(
            stream,
            downloads_dir,
            pictures_dir,
            timeline,
            events,
        ));
    }

    fn op_file_event(&mut self, ev: FileEvent) {
        match ev {
            FileEvent::NieuwAanbod {
                pad,
                naam,
                grootte,
                hash,
                channel,
            } => match self.chat.deel_bestand(&naam, grootte, hash, channel) {
                Ok((id, cmds)) => {
                    self.files.biedt_aan(id, pad);
                    self.stuur_alles(cmds);
                }
                Err(e) => {
                    tracing::error!(error = %format!("{e:#}"), "bestand aanbieden mislukt");
                    self.fout = Some(format!("bestand aanbieden: {e:#}"));
                }
            },
            FileEvent::Voortgang { file, ontvangen } => {
                self.files.zet_voortgang(file, ontvangen);
            }
            FileEvent::Voltooid { file } => {
                self.files.zet_status(file, DownloadStatus::Voltooid);
            }
            FileEvent::Mislukt { file, bericht } => {
                tracing::warn!(?file, %bericht, "bestandsoverdracht mislukt");
                self.files
                    .zet_status(file, DownloadStatus::Mislukt(bericht));
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

    /// Beslist of een net binnengekomen, live bericht een Windows-melding waard is.
    /// Alleen bij een geldige tag naar jezelf — nooit bij elk bericht, en nooit in
    /// niet-storenmodus, ongeacht de inhoud. Geldt gelijk voor het algemene kanaal en
    /// een DM: een DM is niet vanzelf een melding waard, net zomin als een gewoon
    /// bericht in het algemene kanaal dat is.
    fn overweeg_melding(&mut self, van: PeerId, body: &str) {
        if self.niet_storen || self.voorgrond.load(Ordering::Relaxed) {
            return;
        }
        let eigen_naam = self.eigen_weergavenaam();
        if tags::bevat_tag(body, &eigen_naam) {
            self.meld_nieuw_bericht(van, body);
        }
    }

    /// De weergavenaam waarop een tag naar "jezelf" gecontroleerd wordt: de naam zoals
    /// die in de oplog staat (kan afwijken van `cfg.display_name` vlak na een
    /// naamswijziging die nog niet verwerkt is), met de configwaarde als terugval.
    fn eigen_weergavenaam(&self) -> String {
        self.chat
            .timeline()
            .nicknames
            .get(&self.chat.me())
            .cloned()
            .unwrap_or_else(|| self.cfg.display_name.clone())
    }

    /// Toont een Windows-melding voor een bericht dat al bevestigd is als meldingswaardig.
    fn meld_nieuw_bericht(&mut self, van: PeerId, tekst: &str) {
        let naam = self
            .chat
            .timeline()
            .nicknames
            .get(&van)
            .cloned()
            .unwrap_or_else(|| van.to_string()[..8].to_string());
        notify::nieuw_bericht(&naam, tekst);
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
                miniatuur: self.miniaturen.get(&(s.eigenaar, s.id)).cloned(),
            })
            .collect();

        let me = self.chat.me();
        let files = self
            .chat
            .timeline()
            .files
            .iter()
            .map(|f| FileView {
                id: f.id,
                author: f.author,
                channel: f.channel,
                name: f.name.clone(),
                size: f.size,
                is_mine: f.author == me,
                status: self.files.status(f.id).cloned(),
                lamport: f.lamport,
                hash: f.hash,
            })
            .collect();

        let _ = self.snap_tx.send(Arc::new(Snapshot {
            timeline: self.chat.timeline_arc(),
            peers,
            voice,
            eigen_streams,
            streams,
            video: self.cfg.video.clone(),
            files,
            ongelezen: self.chat.ongelezen,
            ongelezen_dm: self.chat.ongelezen_dm().clone(),
            ongelezen_topic: self.chat.ongelezen_topic().clone(),
            niet_storen: self.niet_storen,
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

// ---------------------------------------------------------------------------
// Bestandsoverdracht: losse tokio-taken. Puur I/O en hashen, geen beslissingen —
// die zitten in `files.rs`. Zie `docs/ARCHITECTURE.md` voor waarom de bulkbytes over
// een eigen QUIC-stream gaan in plaats van over de control-stream.
// ---------------------------------------------------------------------------

/// De naam van het deelbestand (`.part`) op schijf tijdens het downloaden. Op basis
/// van `(author, channel, seq)` in plaats van de leesbare naam: twee aanbiedingen met
/// dezelfde bestandsnaam van verschillende peers mogen elkaar niet overschrijven. Het
/// kanaal moet erbij sinds `seq` per (auteur, kanaal) telt in plaats van per auteur
/// alleen — zonder dat zou een algemeen bestand, een DM-bestand en een bestand in een
/// subkanaal van dezelfde auteur met toevallig dezelfde `seq` op dezelfde tijdelijke naam
/// uitkomen.
fn deelbestand_naam(entry: &FileEntry) -> String {
    let kanaal = if let Some(p) = entry.channel.dm_peer() {
        format!("dm-{}", p.0.simple())
    } else if let Some(t) = entry.channel.topic_id() {
        format!("topic-{}", t.0.simple())
    } else {
        "algemeen".to_string()
    };
    format!("{}-{kanaal}-{}.part", entry.author.0.simple(), entry.id.seq)
}

/// De naam waaronder het bestand definitief landt. Voegt `" (2)"` etc. toe als de naam
/// al bestaat — bijvoorbeeld omdat twee peers hetzelfde bestand aanboden.
fn unieke_bestandsnaam(dir: &Path, naam: &str) -> PathBuf {
    let kandidaat = dir.join(naam);
    if !kandidaat.exists() {
        return kandidaat;
    }

    let pad = Path::new(naam);
    let stam = pad.file_stem().and_then(|s| s.to_str()).unwrap_or(naam);
    let ext = pad.extension().and_then(|s| s.to_str());
    for i in 2u32.. {
        let naam_n = match ext {
            Some(e) => format!("{stam} ({i}).{e}"),
            None => format!("{stam} ({i})"),
        };
        let kandidaat = dir.join(&naam_n);
        if !kandidaat.exists() {
            return kandidaat;
        }
    }
    unreachable!("dir.join blijft nieuwe paden opleveren")
}

/// Leest een lokaal bestand, hasht het, en meldt het resultaat terug zodat de motor het
/// als `FileMeta`-op kan vastleggen. Op een losse taak: bij een groot bestand kan dit
/// seconden duren, en dat mag de UI niet blokkeren.
async fn hash_en_bied_aan(
    pad: PathBuf,
    channel: Channel,
    pictures_dir: PathBuf,
    events: mpsc::Sender<FileEvent>,
) {
    let naam = pad
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "bestand".to_string());

    let leespad = pad.clone();
    let naam_voor_taak = naam.clone();
    let resultaat =
        tokio::task::spawn_blocking(move || -> std::io::Result<(u64, [u8; 32], PathBuf)> {
            let mut bestand = std::fs::File::open(&leespad)?;
            let mut hasher = blake3::Hasher::new();
            let grootte = std::io::copy(&mut bestand, &mut hasher)?;
            let hash = *hasher.finalize().as_bytes();

            // Een afbeelding krijgt een eigen, content-adresseerbare kopie in
            // `pictures_dir` — daar leest de UI straks een miniatuur van, op precies
            // hetzelfde pad dat een downloadende peer er ook voor gebruikt (zie
            // `files::hash_bestandsnaam`). Het origineel blijft ongemoeid: dat is
            // waarschijnlijk het bestand van de gebruiker zelf, ergens anders op schijf.
            let aanbodpad = if files::is_afbeelding(&naam_voor_taak) {
                let bestemming =
                    pictures_dir.join(files::hash_bestandsnaam(&hash, &naam_voor_taak));
                if bestemming != leespad && !bestemming.exists() {
                    std::fs::copy(&leespad, &bestemming)?;
                }
                bestemming
            } else {
                leespad
            };

            Ok((grootte, hash, aanbodpad))
        })
        .await;

    match resultaat {
        Ok(Ok((grootte, hash, aanbodpad))) => {
            let _ = events
                .send(FileEvent::NieuwAanbod {
                    pad: aanbodpad,
                    naam,
                    grootte,
                    hash,
                    channel,
                })
                .await;
        }
        Ok(Err(e)) => {
            tracing::error!(error = %e, pad = %pad.display(), "bestand lezen voor aanbieden mislukt");
        }
        Err(e) => {
            tracing::error!(error = %e, "hash-taak voor aanbieden afgebroken");
        }
    }
}

/// Opent de uploadstream naar de aanvrager en stuurt het bestand vanaf `vanaf`. Fouten
/// hier zijn alleen voor de logs: de aanvrager ziet gewoon geen bytes komen en kan het
/// later opnieuw proberen.
async fn upload_taak(
    mesh_commands: mpsc::Sender<MeshCommand>,
    naar: PeerId,
    file: OpId,
    pad: PathBuf,
    vanaf: u64,
) {
    // Openen vóór de uploadstream aanvragen: `Files::verzoek_ontvangen` weet alleen dat we
    // dit bestand ooit hebben aangeboden, niet of het nu nog op schijf staat. Blijkt het
    // weg (verplaatst, verwijderd), dan corrigeren we hier alsnog naar `NotAvailable` in
    // plaats van de aanvrager voor altijd op "bezig" te laten staan.
    let mut bestand = match tokio::fs::File::open(&pad).await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(error = %e, pad = %pad.display(), "bronbestand niet meer te openen");
            let _ = mesh_commands
                .send(MeshCommand::Send {
                    to: naar,
                    msg: ControlMsg::FileResponse(fitcom_proto::control::FileResponse {
                        file,
                        outcome: fitcom_proto::control::FileOutcome::NOT_AVAILABLE,
                    }),
                })
                .await;
            return;
        }
    };

    let (tx, rx) = oneshot::channel();
    if mesh_commands
        .send(MeshCommand::OpenUploadStream {
            to: naar,
            respond: tx,
        })
        .await
        .is_err()
    {
        return;
    }
    let stream = match rx.await {
        Ok(Some(s)) => s,
        _ => {
            tracing::debug!(peer = ?naar, "kon geen uploadstream openen; peer waarschijnlijk weg");
            return;
        }
    };

    if let Err(e) = upload_bytes(stream, file, &mut bestand, vanaf).await {
        tracing::warn!(error = %format!("{e:#}"), peer = ?naar, "bestandsupload mislukt");
    }
}

async fn upload_bytes(
    mut stream: SendStream,
    file: OpId,
    bestand: &mut tokio::fs::File,
    vanaf: u64,
) -> Result<()> {
    fitcom_net::filestream::write_header(&mut stream, file).await?;

    bestand
        .seek(std::io::SeekFrom::Start(vanaf))
        .await
        .context("hervatpunt opzoeken in het bronbestand")?;
    tokio::io::copy(bestand, &mut stream)
        .await
        .context("bytes versturen")?;
    stream.finish().context("uploadstream afsluiten")?;
    Ok(())
}

/// Leest de header van een inkomende bulk-stream, zoekt het bijbehorende bestand op in
/// de (op het moment van binnenkomst al bekende) timeline, en downloadt het.
async fn download_taak(
    mut stream: RecvStream,
    downloads_dir: PathBuf,
    pictures_dir: PathBuf,
    timeline: Arc<Timeline>,
    events: mpsc::Sender<FileEvent>,
) {
    let file = match fitcom_net::filestream::read_header(&mut stream).await {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(error = %format!("{e:#}"), "header van inkomende bestandsstream onleesbaar");
            return;
        }
    };

    let Some(entry) = timeline.files.iter().find(|f| f.id == file).cloned() else {
        tracing::warn!(?file, "bestandsstream voor een onbekend bestand genegeerd");
        return;
    };

    if let Err(e) =
        download_bytes(&mut stream, &downloads_dir, &pictures_dir, &entry, &events).await
    {
        let _ = events
            .send(FileEvent::Mislukt {
                file: entry.id,
                bericht: format!("{e:#}"),
            })
            .await;
    }
}

async fn download_bytes(
    stream: &mut RecvStream,
    downloads_dir: &Path,
    pictures_dir: &Path,
    entry: &FileEntry,
    events: &mpsc::Sender<FileEvent>,
) -> Result<()> {
    let deelpad = downloads_dir.join(deelbestand_naam(entry));
    let mut bestand = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&deelpad)
        .await
        .context("deelbestand openen")?;
    let mut ontvangen = bestand
        .metadata()
        .await
        .context("deelbestand-grootte opvragen")?
        .len();

    let mut buf = vec![0u8; 64 * 1024];
    let mut laatste_update = Instant::now();
    loop {
        match stream.read(&mut buf).await.context("bytes ontvangen")? {
            None => break,
            Some(n) => {
                bestand
                    .write_all(&buf[..n])
                    .await
                    .context("bytes wegschrijven")?;
                ontvangen += n as u64;
                if laatste_update.elapsed() >= VOORTGANG_INTERVAL {
                    laatste_update = Instant::now();
                    let _ = events
                        .send(FileEvent::Voortgang {
                            file: entry.id,
                            ontvangen,
                        })
                        .await;
                }
            }
        }
    }
    bestand.flush().await.context("deelbestand doorschrijven")?;
    drop(bestand);

    // Eén sequentiële leespas over het net geschreven bestand in plaats van meehashen
    // tijdens het schrijven: zo hoeft het hervatpunt niet bij te houden wat al eerder
    // gehasht was, en gaan de netwerkbytes rechtstreeks naar schijf zonder een tweede
    // kopie in het geheugen.
    let te_hashen = deelpad.clone();
    let verwacht = entry.hash;
    let klopt = tokio::task::spawn_blocking(move || -> std::io::Result<bool> {
        let mut bestand = std::fs::File::open(&te_hashen)?;
        let mut hasher = blake3::Hasher::new();
        std::io::copy(&mut bestand, &mut hasher)?;
        Ok(*hasher.finalize().as_bytes() == verwacht)
    })
    .await
    .context("hash-taak afgebroken")??;

    if !klopt {
        let _ = tokio::fs::remove_file(&deelpad).await;
        anyhow::bail!("hash klopt niet; bestand is corrupt geraakt en is verwijderd");
    }

    // Een afbeelding landt content-adresseerbaar in `pictures_dir` in plaats van in de
    // downloadmap: dat is het pad waar de UI een miniatuur vandaan leest, en het is
    // precies hetzelfde pad dat de aanbieder er zelf ook voor gebruikt (zie
    // `files::hash_bestandsnaam`) — zo is een afbeelding voor beide kanten inline te
    // tonen, niet alleen bij wie hem aanbood.
    let definitief = if files::is_afbeelding(&entry.name) {
        pictures_dir.join(files::hash_bestandsnaam(&entry.hash, &entry.name))
    } else {
        unieke_bestandsnaam(downloads_dir, &entry.name)
    };
    tokio::fs::rename(&deelpad, &definitief)
        .await
        .context("bestand hernoemen naar definitieve naam")?;
    let _ = events.send(FileEvent::Voltooid { file: entry.id }).await;
    Ok(())
}
