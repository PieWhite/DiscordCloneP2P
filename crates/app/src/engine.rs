//! De motor: bezit de mesh, de oplog en de voice-sessie. Draait op de tokio-runtime.
//!
//! # Waarom dit niet in de UI zit
//!
//! Een venster tekent niet zolang het verborgen of geminimaliseerd is. Zat de chat- en
//! sync-lus in de weergavelus, dan stopt de synchronisatie zodra je minimaliseert of naar
//! de tray gaat — precies het moment waarop je een melding zou willen krijgen dat er
//! iemand iets zegt. Voor een app die naast een game moet kunnen draaien is dat het
//! verkeerde gedrag. Dat gold voor egui en geldt onverkort voor de webview die het
//! sinds fase 12 is.
//!
//! De UI leest daarom alleen nog een momentopname en stuurt commando's terug. Zij mag
//! stilvallen zonder dat er iets misgaat.

use crate::chat::Chat;
use crate::config::{self, Config, SoundConfig, VideoConfig};
use crate::files::{self, DownloadStatus, Files, StartUpload};
use crate::geluid;
use crate::notify;
use crate::release::{self, Release};
use crate::streams::{Actie, Streams};
use crate::tags;
use crate::updates::{UpdateStatus, Updates};
use crate::wordle::{self, Wordle};
use anyhow::{Context, Result};
use fitcom_audio::{PeerAdres, VoiceConfig, VoiceHandle};
use fitcom_net::{MeshCommand, MeshEvent, MeshHandle, PeerStatus, RecvStream, SendStream};
use fitcom_proto::control::{StreamKind, Typing, UserStatus, UserStatusValue, VoiceJoin, VoiceLeave};
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

/// Eigen versie. Gaat mee in de handshake (peers tonen elkaars versie) en is sinds fase
/// 13 waar de release-feed tegen vergeleken wordt (`fitcom_proto::is_newer`).
const EIGEN_VERSIE: &str = env!("CARGO_PKG_VERSION");

/// B-22: tot hier haalt de app een aangeboden afbeelding vanzelf op.
///
/// Een afbeelding is de enige soort die zichzelf downloadt én toont zonder dat er iemand
/// klikt, dus dit is het enige pad waarlangs bytes ongevraagd binnenkomen. 16 MiB is ruim
/// boven een schermafdruk of een foto uit een telefoon, en ver onder wat een schijf
/// volschrijft. Groter blijft gewoon in de lijst staan met een downloadknop — dan is het
/// een keuze in plaats van iets dat achter je rug gebeurt.
///
/// Dit begrenst het *ophalen*, niet het *decoderen*: de afmetingen van een PNG (een
/// 30000×30000 "decompression bomb" is een paar honderd kB op de draad en gigabytes in de
/// renderer) worden door WebView2 afgehandeld, en daar komen wij niet tussen. Dat blijft
/// een open deel van B-22.
const MAX_AUTO_AFBEELDING: u64 = 16 * 1024 * 1024;

/// Hoe vaak de release-feed geraadpleegd wordt. Er is niets aan gelegen om er sneller
/// bij te zijn, en dit is de enige verbinding die de app buiten het tailnet legt.
const UPDATE_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
/// Niet meteen bij het starten: eerst moet de mesh staan en moet de gebruiker zijn
/// venster zien. Een feed-check die de start vertraagt is een check te vroeg.
const UPDATE_EERSTE_CHECK: Duration = Duration::from_secs(60);

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
    /// De door de gebruiker gekozen status van deze peer, zoals hij hem zelf stuurde.
    /// `None` zolang hij nog niets stuurde (of offline was): dan is hij impliciet
    /// "online", en de UI hoeft niets extra's te tonen.
    pub user_status: Option<UserStatusValue>,
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
    /// Onze eigen camera in plaats van een scherm. De UI heeft dit nodig om de
    /// camera-knop ingedrukt te tonen en om te weten welke stream hij weer uitzet.
    pub is_camera: bool,
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
    /// Iemands camera in plaats van zijn scherm. Verder identiek — hetzelfde venster,
    /// dezelfde miniatuur — alleen het icoontje in de lijst verschilt.
    pub is_camera: bool,
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
    /// Waar de bytes op deze pc staan, als we ze hebben: ons eigen aanbod of een
    /// afgeronde download. `None` betekent dat er niets te openen valt. Zie
    /// `files::Files::lokaal_pad`.
    pub local_path: Option<PathBuf>,
}

/// Wordle, zoals de UI het toont. Zie `crate::wordle` voor de regels.
#[derive(Debug, Clone, Default)]
pub struct WordleView {
    /// De dag waar het huidige raadsel bij hoort, `YYYYMMDD`. Wisselt om 07:00.
    pub dag: u32,
    /// Het bord van vandaag. `None` zolang het woord niet binnen is — dan is er geen
    /// kaart en staat er niets in de weg.
    pub bord: Option<wordle::Bord>,
    /// Raadselnummer per dag die deze pc kent, voor de kop van een kaart. Alleen wat we
    /// zelf ooit ophaalden; van een dag die we misten weten we het nummer niet.
    pub nummers: HashMap<u32, u32>,
    /// Waarom de laatste gok niet aangenomen werd. Engels: dit gaat rechtstreeks het
    /// venster in.
    pub fout: Option<String>,
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
    /// Welke set tonen de app zelf maakt en hoe hard. Alleen voor het instellingenscherm;
    /// het afspelen leest de config rechtstreeks.
    pub geluid: SoundConfig,
    /// Gekozen microfoon, of `None` voor het Windows-standaardapparaat. Alleen voor het
    /// instellingenscherm: de sessie leest dit bij het starten uit de config.
    pub input_device: Option<String>,
    pub output_device: Option<String>,
    /// Waar downloads nu landen. Volgt `cfg.download_dir`, dus reactief zodra de
    /// gebruiker hem via het instellingenscherm wijzigt.
    pub download_dir: PathBuf,
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
    /// Clips (fase 15): ringbuffer-opname met hotkey. `None` op platforms zonder
    /// clipondersteuning — de UI verbergt er alles omheen.
    pub clips: Option<crate::clips::ClipsWeergave>,
    /// Wie nu aan het typen is: (kanaal, peer). Vluchtig — alleen levende indicaties
    /// van verbonden peers, zie `TYPING_TTL`.
    pub typing: Vec<(Channel, PeerId)>,
    /// Onze eigen gekozen status (`UserStatusValue`). Vluchtig, begint op online.
    pub eigen_status: UserStatusValue,
    /// Fase 11: staat een peer met een nieuwere versie aangeboden/onderweg/klaar?
    /// `None` betekent: niets aan de hand, iedereen die we spreken zit op onze versie
    /// (of ouder).
    pub update: Option<UpdateStatus>,
    /// Het spelletje van de dag en het scorebord (2026-08-20).
    pub wordle: WordleView,
    pub fout: Option<String>,
}

#[derive(Debug)]
pub enum UiCommand {
    /// Kanaal erbij: het algemene kanaal, of een DM met de gekozen peer. Met een
    /// `reply_to` wordt het bericht een antwoord op dat bericht.
    Plaats(String, Channel, Option<OpId>),
    Bewerk(OpId, String),
    Verwijder(OpId),
    /// Reactie op een bericht zetten of (als hij er al staat) intrekken.
    Reageer(OpId, String),
    /// "Ik ben aan het typen" in dit kanaal. Vluchtig; de motor throttle zelf.
    Typing(Channel),
    /// Eigen aanwezigheidsstatus: 0 online, 1 away, 2 busy (`UserStatusValue`).
    ZetStatus(u8),
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
    /// De camera aan (`true`) of uit (`false`). Eén schakelaar in plaats van de UI laten
    /// uitzoeken welke bron de camera is; bureaubladgeluid blijft er buiten.
    ZetCamera(bool),
    Kijken(PeerId, u32),
    StopKijken(PeerId, u32),
    /// Volume van één stream van een peer, los van zijn stem.
    StreamVolume(PeerId, u32, f32),
    /// Codec, framerate en bitrate voor screenshare. Geldt voor delers die al lopen
    /// meteen mee — die worden herstart met de nieuwe instellingen — en voor nieuw
    /// gestarte bronnen vanzelf, want die lezen `cfg.video` bij het starten.
    ZetVideoInstellingen(VideoConfig),
    /// Microfoon en weergaveapparaat, `None` = het Windows-standaardapparaat. De
    /// apparaten worden bij het openen van de voice-sessie gekozen, dus een lopend
    /// gesprek wordt hiervoor kort herstart.
    ZetGeluidsapparaten(Option<String>, Option<String>),
    /// Nieuwe downloadmap. Bestaande downloads blijven staan waar ze stonden; alleen
    /// nieuwe downloads landen vanaf nu op het nieuwe pad.
    ZetDownloadMap(PathBuf),
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
    /// Welke set tonen en hoe hard. Wordt meteen weggeschreven naar `config.toml`.
    ZetGeluidInstellingen(SoundConfig),
    /// Eén toon laten horen om te kiezen. Negeert niet-storen: wie op de knop drukt vraagt
    /// er expliciet om, en anders lijkt de knop stuk.
    ProefGeluid(String),
    /// Nu bij de release-feed kijken, op verzoek van de gebruiker. Anders dan de tik van
    /// zes uur meldt deze elke uitkomst, ook "niets nieuws" en "feed onbereikbaar".
    ZoekUpdate,
    /// Fase 11: een klaarstaande update bevestigen — start het updaterproces en sluit de
    /// app af.
    PasUpdateToe,
    /// Deze versie niet meer vanzelf aanbieden, deze sessie.
    NegeerUpdate(String),
    /// Een mislukte update-melding wegklikken.
    WisUpdateMelding,
    /// Eén gok op het Wordle-raadsel van vandaag. Het woord komt uit het venster; de
    /// motor beoordeelt het, want de oplossing blijft aan deze kant.
    WordleGok(String),    /// De kaart van vandaag met de hand in het algemene kanaal zetten, zodat *iedereen* hem
    /// ziet — de reddingsklep uit het `+`-menu. Haalt het raadsel eerst op als deze pc het
    /// nog niet heeft, want zonder dat valt er niets aan te kondigen.
    WordleInChat,
    /// Clipopname aan of uit, en dat meteen vastleggen in de config.
    ZetClips(bool),
    /// Nú de laatste `venster_sec` seconden als clip wegschrijven. Ook via de globale
    /// hotkey, die hier op hetzelfde kanaal uitkomt.
    ClipseNu,
    /// Welk scherm er voor clips opgenomen wordt (naam uit de bronlijst). Leeg =
    /// automatisch het eerste.
    ZetClipMonitor(String),
    /// De globale clip-sneltoets wisselen, zonder herstart.
    ZetClipsHotkey(String),
}

pub struct EngineHandle {
    pub snapshot: watch::Receiver<Arc<Snapshot>>,
    pub commands: mpsc::Sender<UiCommand>,
    /// Wordt door de UI bijgehouden. Staat het venster niet op de voorgrond, dan
    /// verstuurt de motor een Windows-melding bij een nieuw bericht.
    pub voorgrond: Arc<AtomicBool>,
    /// Gaat aan zodra de gebruiker een klaarstaande update bevestigt en het
    /// updater-proces gestart is. De UI sluit het venster dan net zo af als via het
    /// tray-menu — zie `tray::wil_afsluiten` en `ui.rs::afsluiten_of_verbergen`.
    pub afsluiten_voor_update: Arc<AtomicBool>,
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
    let (kijker_tx, kijker_rx) = mpsc::unbounded_channel();

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
    let updates_dir = config::resolve_updates_dir(&data_dir);
    std::fs::create_dir_all(&updates_dir).context("updatesmap aanmaken")?;
    let paden_index = data_dir.join("bestandspaden.json");
    let mut files = Files::new();
    let (aangeboden, gedownload) = lees_paden_index(&paden_index);
    files.herstel_paden(aangeboden, gedownload);
    let afsluiten_voor_update = Arc::new(AtomicBool::new(false));
    let clips_beheer = crate::clips::ClipsBeheer::nieuw(config::resolve_clips_dir(&data_dir));

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
            user_status: None,
        })
        .collect();

    // Globale clip-hotkey (standaard F9, instelbaar). Eigen Win32-draad; hij duwt
    // rechtstreeks op het motorcommandokanaal en werkt dus ook vanuit de tray. Vóór
    // de Engine-aanmaak: `cfg` verhuist daarin, en de draad hoort bij het object.
    let clip_hotkey_spec = cfg.clips.hotkey.clone();
    let (clip_hotkey_verzender, clip_hotkey_ontvanger) = std::sync::mpsc::channel();
    let hotkey_tx = clip_hotkey_verzender.clone();
    let hotkey_draad = crate::clips::start_hotkey(&clip_hotkey_spec, move || {
        let _ = hotkey_tx.send(());
    })
    .context("clip-hotkey registreren")
    .unwrap_or_else(|e| {
        tracing::warn!(error = %format!("{e:#}"), "hotkey uit; clips blijven via de UI werken");
        crate::clips::HotkeyDraad::dode()
    });

    let engine = Engine {
        mesh,
        chat,
        cfg,
        config_path,
        hotkey_draad,
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
        files,
        downloads_dir,
        pictures_dir,
        paden_index,
        wordle: Wordle::nieuw(&data_dir),
        wordle_aankondigen: false,
        updates: Updates::new(),
        updates_dir,
        file_tx,
        file_rx,
        kijker_tx,
        kijker_rx,
        fout: None,
        niet_storen: false,
        clips: clips_beheer,
        clip_hotkey_verzender,
        clip_hotkey_ontvanger,
        typing_tot: HashMap::new(),
        typing_verzonden: HashMap::new(),
        peer_status: HashMap::new(),
        eigen_status: UserStatusValue::ONLINE,
        snap_tx,
        voorgrond: voorgrond.clone(),
        afsluiten_voor_update: afsluiten_voor_update.clone(),
    };

    tokio::spawn(engine.run(cmd_rx));

    Ok(EngineHandle {
        snapshot: snap_rx,
        commands: cmd_tx,
        voorgrond,
        afsluiten_voor_update,
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
    /// Waar de padkaarten van `files` op schijf staan. Zie [`PadenIndex`].
    paden_index: PathBuf,
    /// Het raadsel van de dag, de eigen gokken en waar dat op schijf staat.
    /// Zie `crate::wordle`.
    wordle: Wordle,
    /// Er is op "zet hem in de chat" gedrukt terwijl het raadsel er nog niet was; zodra
    /// het binnenkomt gaat de kaart alsnog de log in. Zie `wordle_in_chat`.
    wordle_aankondigen: bool,
    /// Welke versie de release-feed aanbood en waar we staan met het ophalen daarvan.
    /// Zie `crate::updates` voor de beslissingen en `crate::release` voor het halen.
    updates: Updates,
    /// Waar een opgehaalde nieuwere exe landt, tot toepassing.
    updates_dir: PathBuf,
    /// Voor het klonen naar bestandstaken toe; `file_rx` is waar de motor op wacht.
    file_tx: mpsc::Sender<FileEvent>,
    file_rx: mpsc::Receiver<FileEvent>,
    /// Wat de kijkvensters melden. Bewust *niet* op de tik: een keyframe-verzoek dat
    /// honderd milliseconde blijft liggen is honderd milliseconde bevroren beeld bij de
    /// kijker, en dat was de helft van de gemeten hapering. Zie `lees_kijker`.
    kijker_tx: mpsc::UnboundedSender<(PeerId, u32, KijkerEvent)>,
    kijker_rx: mpsc::UnboundedReceiver<(PeerId, u32, KijkerEvent)>,

    fout: Option<String>,
    /// Zie `Snapshot::niet_storen`.
    niet_storen: bool,
    /// Clipopname-beheer (fase 15). Bestaat altijd; op platforms zonder ondersteuning
    /// is het een no-op die zichzelf afwezig meldt.
    clips: crate::clips::ClipsBeheer,
    /// De geregistreerde clip-hotkey. Vervangen (Drop → WM_QUIT → nieuwe draad) zodra
    /// de gebruiker een andere toets kiest.
    hotkey_draad: crate::clips::HotkeyDraad,
    /// De hotkey-draad praat via dit kanaal naar de motor: één seintje = "bewaar nu",
    /// dezelfde weg als het ClipseNu-commando.
    clip_hotkey_verzender: std::sync::mpsc::Sender<()>,
    clip_hotkey_ontvanger: std::sync::mpsc::Receiver<()>,
    /// Vluchtige typindicaties: tot wanneer deze peer in dit kanaal aan het typen is.
    /// Geen oplog, geen doorgifte — na een paar seconden stilte of een verbreking is de
    /// betekenis weg en mag de administratie dat ook zijn.
    typing_tot: HashMap<(PeerId, Channel), Instant>,
    /// Wanneer wij voor het laatst een `Typing` in een kanaal stuurden; throttle zodat
    /// elke toetsaanslag geen bericht wordt.
    typing_verzonden: HashMap<Channel, Instant>,
    /// De gekozen status van elke peer deze sessie. Een peer die nog niets stuurde is
    /// impliciet online.
    peer_status: HashMap<PeerId, UserStatusValue>,
    /// Onze eigen gekozen status. Vluchtig: na een herstart beginnen we weer op online.
    eigen_status: UserStatusValue,
    snap_tx: watch::Sender<Arc<Snapshot>>,
    voorgrond: Arc<AtomicBool>,
    afsluiten_voor_update: Arc<AtomicBool>,
}

/// Hoe lang een `Typing`-indicatie geldig blijft bij de ontvanger.
const TYPING_TTL: Duration = Duration::from_secs(5);
/// Hoe vaak we hooguit een `Typing` versturen tijdens doorlopend typen. Ruim onder de
/// TTL, zodat één verloren bericht de indicatie niet laat flikkeren.
const TYPING_THROTTLE: Duration = Duration::from_millis(2500);

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
        /// Waar het bestand na verificatie is beland. Niet te herleiden uit
        /// `FileEntry::name`: bij een naambotsing wordt het `naam (2).ext`.
        pad: PathBuf,
    },
    Mislukt {
        file: OpId,
        bericht: String,
    },
    /// Fase 13: verloop van een check bij de release-feed. Los van `Voortgang`/
    /// `Voltooid`/`Mislukt` hierboven omdat een update geen `OpId` heeft.
    ///
    /// De hele check — feed halen, handtekening controleren, downloaden — is één taak,
    /// dus dit is puur wat de UI moet weten.
    UpdateGestart {
        versie: String,
        totaal: u64,
    },
    UpdateVoortgang {
        ontvangen: u64,
    },
    /// De feed was bereikbaar en had niets nieuwers (of alleen iets weggeklikts).
    GeenUpdate,
    UpdateKlaar {
        pad: PathBuf,
        /// B-20: de geverifieerde hash reist mee tot aan de updater.
        hash: [u8; 32],
    },
    UpdateMislukt {
        bericht: String,
    },
    /// Het raadsel van vandaag is bij NYT opgehaald (2026-08-20). Zie `crate::wordle` voor
    /// waarom dat in de motor gebeurt en niet in de webview.
    WordleGeladen(wordle::Raadsel),
    /// Een *handmatige* poging is misgegaan; de reden hoort in het venster. Een
    /// automatische poging meldt niets — zie `wordle_taak`.
    WordleMislukt(String),
}

impl Engine {
    async fn run(mut self, mut cmds: mpsc::Receiver<UiCommand>) {
        // Eigen naam vastleggen in de oplog zodat de anderen hem zien. Doet niets als
        // hij al klopt, dus de log groeit hier niet van bij elke start.
        let naam = self.cfg.display_name.clone();
        let r = self.chat.zet_naam(&naam);
        self.verwerk(r);
        self.wordle_inhaalslag();
        if self.cfg.clips.enabled {
            self.zet_clips_motor(true);
        }
        self.publiceer();

        let mut ticker = tokio::time::interval(TIK);
        let mut update_ticker = tokio::time::interval_at(
            tokio::time::Instant::now() + UPDATE_EERSTE_CHECK,
            UPDATE_INTERVAL,
        );

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
                // `None` kan niet gebeuren zolang de motor zelf `kijker_tx` vasthoudt.
                ev = self.kijker_rx.recv() => if let Some((eigenaar, stream_id, ev)) = ev {
                    self.lees_kijker(eigenaar, stream_id, ev);
                },
                _ = ticker.tick() => {
                    let verbonden: Vec<PeerId> = self.verbonden.keys().copied().collect();
                    let r = self.chat.tick(&verbonden);
                    self.verwerk(r);
                    self.ruim_gestopte_camera_op();
                    self.wordle_tick();
                    // Verlopen typindicaties opruimen; de volgende publiceer() merkt
                    // het alleen als er ook echt iets verdween.
                    if !self.typing_tot.is_empty() {
                        let nu = Instant::now();
                        self.typing_tot.retain(|_, tot| *tot > nu);
                    }
                    // Clip-events (klaar/mislukt) binnenhalen en een doodlopende
                    // keten signaleren. Een klaargekomen clip krijgt hetzelfde
                    // geluidje als een stream die start — het is ook een "er wordt
                    // nu iets voor je opgenomen/geleverd"-moment.
                    if let Some(_pad) = self.clips.tik() {
                        self.geluid(crate::geluid::Geluid::StreamAan);
                    }
                    if let Some(f) = self.clips.fout() {
                        if self.fout.is_none() {
                            self.fout = Some(format!("clips: {f}"));
                        }
                    }
                    // Hotkey-seintjes (F9 of wat de gebruiker koos): zelfde pad als
                    // ClipseNu.
                    let mut hotkey_aanvragen = 0usize;
                    while self.clip_hotkey_ontvanger.try_recv().is_ok() {
                        hotkey_aanvragen += 1;
                    }
                    if hotkey_aanvragen > 0 {
                        tracing::info!(aanvragen = hotkey_aanvragen, "clip-save aangevraagd via de hotkey");
                        if self.cfg.clips.enabled && self.clips.aan() {
                            self.clips.bewaar_nu();
                        } else {
                            self.fout = Some("Clips staan uit (instellingen → clips).".into());
                        }
                    }
                }
                _ = update_ticker.tick() => self.zoek_update(false),
            }

            self.chat.refresh();
            self.volg_geluid_bij_beeld();
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
                            // voor wat ik deel en voor mijn gekozen status.
                            if self.voice.is_some() {
                                self.meld_voice_status(Some(id));
                            }
                            if self.eigen_status != UserStatusValue::ONLINE {
                                self.stuur_alles(vec![MeshCommand::Send {
                                    to: id,
                                    msg: ControlMsg::UserStatus(UserStatus {
                                        status: self.eigen_status,
                                    }),
                                }]);
                            }
                            let cmds = self.streams.bij_verbinding(id);
                            self.stuur_alles(cmds);
                        }
                    }
                    _ => {
                        if let Some(id) = self.peers.get(target).and_then(|p| p.peer_id) {
                            self.verbonden.remove(&id);
                            // Weg is weg: uit het gesprek halen, anders blijven we
                            // audio naar een dood adres sturen. Wegvallen klinkt hetzelfde
                            // als weggaan — voor wie meepraat is dat hetzelfde.
                            if self.peers_in_voice.remove(&id) {
                                self.geluid(geluid::Geluid::PeerLeave);
                            }
                            // Zijn vluchtige statussen zijn ze ook waard: een typindicatie
                            // van een peer die er niet meer is klopt per definitie niet.
                            self.typing_tot.retain(|(p, _), _| *p != id);
                            self.peer_status.remove(&id);
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

            // Een bulkoverdracht, los van de control-stream. Het kind-byte plus (voor een
            // bestand) de header erachteraan zegt om wat het gaat; `from` is hier niet
            // nodig om te routeren.
            MeshEvent::IncomingFileStream { from, stream } => {
                self.start_incoming_stream(from, stream);
            }

            MeshEvent::Message { from, msg } => match msg {
                ControlMsg::VoiceJoin(_) => {
                    tracing::info!(peer = ?from, "peer neemt deel aan het gesprek");
                    // Alleen bij een echte overgang: deze melding komt ook langs als
                    // *wij* net verbinding maken en hij al in het gesprek zat, en dan
                    // is er niets gebeurd om over te piepen.
                    if self.peers_in_voice.insert(from) {
                        self.geluid(geluid::Geluid::PeerJoin);
                    }
                    self.werk_voice_peers_bij();
                }
                ControlMsg::VoiceLeave(_) => {
                    tracing::info!(peer = ?from, "peer verlaat het gesprek");
                    if self.peers_in_voice.remove(&from) {
                        self.geluid(geluid::Geluid::PeerLeave);
                    }
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
                ControlMsg::Typing(t) => {
                    // Een DM-typindicatie hoort alleen van de gesprekspartner zelf te
                    // komen. Het bericht bewijst wíéér hem stuurde, niet dat hij erin
                    // zit — dus een kanaal waar wij niet de geadresseerde van zijn
                    // negeren we hier, net als de op zelf bij ontvangst.
                    if t.channel.dm_peer().is_some_and(|p| p != self.chat.me()) {
                        tracing::debug!(peer = ?from, "typindicatie voor een vreemde DM genegeerd");
                    } else {
                        // Vluchtig en single-hop: alleen bijhouden, nooit doorsturen.
                        // De tik ruimt de verlopen entries op, zodat de indicatie ook
                        // verdwijnt als er daarna geen bericht meer komt.
                        self.typing_tot
                            .insert((from, t.channel), Instant::now() + TYPING_TTL);
                    }
                }
                ControlMsg::UserStatus(s) => {
                    if s.status.is_known() {
                        let was = self.peer_status.insert(from, s.status);
                        if was != Some(s.status) {
                            tracing::debug!(peer = ?from, status = %s.status, "status gewijzigd");
                        }
                    }
                }
                // Fase 11 duwde exe's tussen peers door; sinds fase 13 komt een update
                // uit de getekende release-feed en gaat de eigen binary nergens meer
                // heen (`docs/BEVEILIGING.md` B-01, B-21). De varianten blijven in het
                // protocol staan — dat wijzigt alleen additief — maar worden hier
                // geweigerd, ook als de peer een oudere build draait die ze nog stuurt.
                ControlMsg::UpdateRequest(_) | ControlMsg::UpdateResponse(_) => {
                    tracing::warn!(
                        peer = ?from,
                        "update-bericht van een peer genegeerd; updates komen uit de release-feed"
                    );
                }
                andere => {
                    // Screenshare eerst: `bij_bericht` van de chat laat alles wat niet
                    // van hem is ongemoeid, en andersom net zo.
                    if let Some(ip) = self.verbonden.get(&from).map(|a| a.ip()) {
                        // Voor en na tellen in plaats van op het berichttype letten: een
                        // `StreamAnnounce` komt bij elke herverbinding opnieuw langs voor
                        // een stream die we al kenden, en daar hoort geen geluidje bij.
                        let voor = self.zichtbare_streams_van(from);
                        let (cmds, acties) = self.streams.bij_bericht(from, ip, &andere);
                        self.stuur_alles(cmds);
                        self.voer_uit(acties);
                        match self.zichtbare_streams_van(from).cmp(&voor) {
                            std::cmp::Ordering::Greater => self.geluid(geluid::Geluid::StreamAan),
                            std::cmp::Ordering::Less => self.geluid(geluid::Geluid::StreamUit),
                            std::cmp::Ordering::Equal => {}
                        }
                    }

                    // Alleen een live `OpBroadcast` van een `Post` komt in aanmerking
                    // voor een melding. Een `SyncResponse` is per definitie een
                    // inhaalslag van gemiste geschiedenis (zie docs/OVERDRACHT.md) en
                    // meldt daarom nooit, ongeacht de inhoud — dat onderscheid zit al
                    // in het berichttype, dus zonder aparte "sync klaar"-status.
                    let live_post = match &andere {
                        ControlMsg::OpBroadcast(b) => match b.op.kind() {
                            Ok(Some(OpKind::Post { body, .. })) => Some((b.op.channel, body)),
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
                                // B-22: alleen automatisch ophalen onder een plafond. Dit is
                                // het enige pad waarop bytes zonder één klik binnenkomen —
                                // een afbeelding haalt en toont zichzelf, live én bij het
                                // inhalen van geschiedenis. Zonder grens is dat een
                                // download-DoS met een `size` die niemand controleert.
                                // Boven het plafond blijft het bestand gewoon aanklikbaar;
                                // het gaat er alleen niet meer vanzelf achteraan.
                                && f.size <= MAX_AUTO_AFBEELDING
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

    // -- updates (fase 13) --------------------------------------------------

    /// Kijkt bij de release-feed of er een nieuwere, getekende versie klaarstaat, en haalt
    /// hem meteen op als dat zo is. Eén taak voor het hele traject: de motor houdt geen
    /// halve toestand vast tussen "feed gelezen" en "bytes binnen".
    /// `handmatig` is een druk op "Check for updates". Dan hoort elke uitkomst zichtbaar
    /// te worden; de periodieke tik blijft stil zolang er niets te melden is, want
    /// offline is een normale toestand en niet iets om over te berichten.
    fn zoek_update(&mut self, handmatig: bool) {
        // Op macOS is er geen updater: `fitcom-updater` is daar een lege stub, dus een
        // opgehaalde build zou nergens heen kunnen. De mac bouwt uit de broncode;
        // versies blijven per werkafspraak gelijk op. Zie docs/OVERDRACHT.md (mac-port).
        if cfg!(target_os = "macos") {
            if handmatig {
                self.updates.mislukt(
                    "op macOS werkt bijwerken niet vanzelf; deze build komt uit de broncode".into(),
                );
            }
            return;
        }
        if !self.updates.mag_zoeken() {
            return;
        }
        self.updates.zoeken_gestart(handmatig);
        tokio::spawn(update_check_taak(
            self.updates_dir.clone(),
            self.updates.genegeerde_versies(),
            handmatig,
            self.file_tx.clone(),
        ));
    }

    /// Het updater-proces naast de app, of niets.
    ///
    /// De naam mag afwijken. Een browser die hetzelfde bestand twee keer binnenhaalt maakt
    /// er `fitcom-updater (1).exe` van, en dan stond de app te wachten op een bestand dat
    /// er in de ogen van de gebruiker gewoon stond. Alles wat in de map van de app met
    /// `fitcom-updater` begint en op `.exe` eindigt telt daarom mee — die map is dezelfde
    /// vertrouwensgrens als `fitcom.exe` zelf.
    fn zoek_updater(verwacht: &std::path::Path) -> Option<PathBuf> {
        if verwacht.exists() {
            return Some(verwacht.to_path_buf());
        }
        let mut kandidaten: Vec<PathBuf> = std::fs::read_dir(verwacht.parent()?)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                let naam = p
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                naam.starts_with("fitcom-updater") && naam.ends_with(".exe")
            })
            .collect();
        // Vast op naam, zodat twee kopieën niet per toeval om en om gekozen worden.
        kandidaten.sort();
        if let Some(p) = kandidaten.first() {
            tracing::warn!(pad = %p.display(), "updater onder een afwijkende naam gevonden");
        }
        kandidaten.into_iter().next()
    }

    /// De gebruiker bevestigt "nu bijwerken en herstarten": start het losse
    /// updater-proces (dat wacht tot wíj afgesloten zijn, want een exe kan zichzelf niet
    /// overschrijven terwijl hij draait) en sluit daarna net zo af als via het tray-menu.
    fn pas_update_toe(&mut self) {
        let Some(UpdateStatus::KlaarOmToeTePassen { pad, hash, .. }) =
            self.updates.status().cloned()
        else {
            return;
        };
        let huidige_exe = match std::env::current_exe() {
            Ok(p) => p,
            Err(e) => {
                self.fout = Some(format!("eigen exe-pad niet te bepalen: {e}"));
                return;
            }
        };
        // Naast de hoofd-exe, net als bij een gewone build (zie `crates/app/src/bin/fitcom-updater.rs`).
        let verwacht = huidige_exe.with_file_name("fitcom-updater.exe");
        // Zonder dit is de melding "updater starten mislukt: Het systeem kan het
        // opgegeven bestand niet vinden" — waar niemand uit opmaakt dat er een tweede
        // bestand naast fitcom.exe uit de zip hoort.
        let Some(updater) = Self::zoek_updater(&verwacht) else {
            let bericht = format!(
                "fitcom-updater.exe staat niet naast fitcom.exe ({}). \
                 Pak hem uit de release naast de app; zonder hem kan een draaiende exe \
                 zichzelf niet vervangen.",
                verwacht.display()
            );
            tracing::error!(pad = %verwacht.display(), "updater ontbreekt");
            self.fout = Some(bericht);
            return;
        };
        // B-20: de hash gaat mee, zodat de updater hem opnieuw legt vlak vóór het
        // overschrijven. Hier verifiëren zou niets toevoegen — tussen hier en `vervang()`
        // zit precies hetzelfde gat.
        let resultaat = std::process::Command::new(&updater)
            .arg("--new")
            .arg(&pad)
            .arg("--target")
            .arg(&huidige_exe)
            .arg("--pid")
            .arg(std::process::id().to_string())
            .arg("--hash")
            .arg(release::bytes_naar_hex(&hash))
            .spawn();
        let mut kind = match resultaat {
            Ok(k) => k,
            Err(e) => {
                tracing::error!(error = %e, pad = %updater.display(), "updater niet te starten");
                self.fout = Some(format!("updater starten mislukt: {e}"));
                return;
            }
        };
        // Een geslaagde `spawn()` zegt alleen dat het proces bestónd, niet dat het leeft.
        // Op 2026-08-20 viel de updater van v1.2.4 die nog naast de app stond meteen om op
        // het nieuwe `--hash` (exitcode 2, vóór zijn eerste logregel, en zonder console
        // omdat hij een GUI-binary is). Wij zagen alleen "gelukt", sloten het venster, en
        // niemand verving iets: de app was weg, de update niet uitgevoerd, geen enkele
        // aanwijzing waarom. Zolang de updater leeft blokkeert hij op ons PID, dus een
        // proces dat binnen deze tijd al weg is, is per definitie stuk.
        //
        // ponytail: 300 ms slapen op de UI-draad; dit is de klik "nu bijwerken en
        // herstarten" en we sluiten hierna toch af. Een wachtdraad met terugkoppeling
        // pas als er ooit iets anders na deze plek moet gebeuren.
        std::thread::sleep(Duration::from_millis(300));
        match kind.try_wait() {
            Ok(None) => self.afsluiten_voor_update.store(true, Ordering::Relaxed),
            Ok(Some(status)) => {
                tracing::error!(
                    %status,
                    pad = %updater.display(),
                    "updater stopte meteen; er wordt niets vervangen"
                );
                self.fout = Some(format!(
                    "de updater stopte meteen ({status}) en heeft niets vervangen. \
                     Meestal staat er dan een oude fitcom-updater.exe naast de app: \
                     haal hem uit de nieuwste release en zet hem naast fitcom.exe. \
                     Zie ook updater.log in die map."
                ));
            }
            // Niet te bepalen — dan liever doorgaan zoals hiervoor dan een werkende
            // update tegenhouden op een vraag die we niet beantwoord krijgen.
            Err(e) => {
                tracing::warn!(error = %e, "updater-status niet op te vragen; toch afsluiten");
                self.afsluiten_voor_update.store(true, Ordering::Relaxed);
            }
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
                self.geluid(geluid::Geluid::EigenJoin);
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
        self.geluid(geluid::Geluid::EigenLeave);
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

    /// Of deze eigen stream een eigen terugblikvenster heeft. Dat is precies de camera:
    /// naar een gedeeld scherm kijk je al.
    ///
    /// Gevolg voor de levensduur van de deler: hij bestaat zolang de camera aan staat, en
    /// niet alleen zolang er iemand kijkt.
    fn heeft_voorbeeld(&self, stream_id: u32) -> bool {
        self.streams
            .eigen()
            .iter()
            .any(|s| s.id == stream_id && s.kind == StreamKind::CAMERA)
    }

    /// De titel van dat venster. Engelstalig, zoals alles wat de gebruiker ziet.
    fn voorbeeld_titel(bron: &Bron) -> Option<String> {
        (bron.soort == BronSoort::Camera).then(|| format!("You — {}", bron.naam))
    }

    /// Zet de camera uit als zijn deler er niet meer is.
    ///
    /// Nodig sinds de opname bij het *aanzetten* begint in plaats van bij de eerste kijker
    /// (beslissing 26): vanaf dat moment kan het aanzetten zelf mislukken — de camera is in
    /// gebruik door Teams, de encoder wil niet — en dat is iets waar de gebruiker op staat
    /// te wachten. De deler heeft geen kanaal terug naar de motor, dus hij legt zijn reden
    /// neer en die wordt hier op de tik opgehaald.
    ///
    /// Vangt tegelijk het nette geval: het voorbeeldvenster gesloten terwijl niemand kijkt.
    /// Dan is er niets mis, maar de knop hoort wel terug op "uit" te springen in plaats van
    /// "aan" te blijven staan boven iets dat niet meer draait.
    ///
    /// Alleen voor een camera. Een gedeeld scherm heeft geen voorbeeldvenster en hoort niet
    /// vanzelf te verdwijnen omdat een deler eruit klapte; daar blijft de aankondiging
    /// staan en probeert de volgende kijker het opnieuw, zoals altijd.
    /// Zet de clipopname aan of uit, zonder de config aan te raken — de commando-arm
    /// beslist of dat meegaat naar `config.toml`. Bij het aanzetten wordt hier pas een
    /// D3D-context geopend: een machine die nooit clips gebruikt hoeft hem niet.
    fn zet_clips_motor(&mut self, aan: bool) {
        let d3d = if aan {
            match self.d3d() {
                Ok(d) => Some(d),
                Err(e) => {
                    self.fout = Some(format!("clips: {e:#}"));
                    return;
                }
            }
        } else {
            None
        };
        if let Err(e) = self.clips.zet(
            aan,
            self.cfg.clips.venster_sec,
            self.cfg.video.fps,
            self.cfg.video.bitrate,
            self.cfg.clips.monitor.as_deref(),
            d3d.as_ref(),
        ) {
            tracing::error!(error = %format!("{e:#}"), "clipopname wisselen mislukt");
            self.fout = Some(format!("clips: {e:#}"));
        }
    }

    fn ruim_gestopte_camera_op(&mut self) {
        let dood: Vec<(u32, Option<String>)> = self
            .streams
            .eigen()
            .iter()
            .filter(|s| s.kind == StreamKind::CAMERA)
            .filter_map(|s| {
                let d = self.delers.get(&s.id)?;
                d.gestopt().then(|| (s.id, d.fout()))
            })
            .collect();

        for (id, fout) in dood {
            match fout {
                Some(f) => {
                    tracing::warn!(stream = id, error = %f, "camera gestopt door een fout");
                    self.fout = Some(format!("camera: {f}"));
                }
                None => tracing::info!(stream = id, "camera uit; eigen venster was gesloten"),
            }
            self.stop_met_delen(id);
        }
    }

    /// Een bron aankondigen. Er wordt nog niets opgenomen — dat is het hele punt.
    ///
    /// Levert het toegekende stream-id op, zodat de aanroeper er nog iets mee kan (de
    /// camera start meteen zijn eigen voorbeeldvenster). `None` als het niet lukte.
    fn deel_bron(&mut self, bron: Bron) -> Option<u32> {
        let afmeting = match fitcom_video::capture::afmeting_van(&bron) {
            Ok(a) => a,
            Err(e) => {
                tracing::error!(error = %format!("{e:#}"), "bron niet te openen");
                self.fout = Some(format!("bron niet te openen: {e:#}"));
                return None;
            }
        };

        let kind = match bron.soort {
            BronSoort::Monitor => StreamKind::MONITOR,
            BronSoort::Venster => StreamKind::WINDOW,
            BronSoort::Camera => StreamKind::CAMERA,
        };
        let (id, cmds) = self
            .streams
            .deel(kind, bron.naam.clone(), afmeting.0, afmeting.1);
        self.bronnen.insert(id, bron);
        self.stuur_alles(cmds);

        // Fase 10: geen losse knop meer, geluid van deze pc gaat automatisch mee zodra
        // er iets gedeeld wordt. `deel_bureaubladgeluid` is zelf al idempotent, dus dit
        // mag ook als er al een tweede scherm bij komt.
        //
        // Een camera valt er buiten: die deelt geen systeemgeluid, en je webcam aanzetten
        // hoort niet stilzwijgend je Spotify de kamer in te sturen.
        if kind.is_scherm() {
            self.deel_bureaubladgeluid();
        }
        Some(id)
    }

    /// Of we op dit moment een monitor of venster delen — dus of bureaubladgeluid mee
    /// hoort te lopen. Een camera telt niet mee.
    fn deelt_scherm_of_venster(&self) -> bool {
        self.streams.eigen().iter().any(|s| s.kind.is_scherm())
    }

    /// Stoppen met delen. Eén plek, want zowel de knop "stop met delen", de camera-knop
    /// als het intrekken bij een fout komen hier langs — en het opruimen van
    /// bureaubladgeluid mag geen van die drie overslaan.
    fn stop_met_delen(&mut self, id: u32) {
        let was_scherm = self
            .bronnen
            .remove(&id)
            .is_some_and(|b| b.soort != BronSoort::Camera);
        let (cmds, acties) = self.streams.stop_delen(id);
        self.stuur_alles(cmds);
        self.voer_uit(acties);
        // Expliciet, niet via `Actie::StopDelen`: die komt er alleen als er iemand kéék,
        // en een camera met een voorbeeldvenster loopt ook zonder kijkers. Dit sluit dus
        // zowel de opname als het eigen venster. Een no-op voor alles wat geen deler had.
        self.delers.remove(&id);
        // Fase 10: het laatste scherm weg betekent ook het geluid weg, zonder dat de
        // gebruiker dat apart hoeft te doen. Een camera heeft er niets mee te maken.
        if was_scherm && !self.deelt_scherm_of_venster() {
            self.stop_bureaubladgeluid();
        }
    }

    /// De camera aan- of uitzetten, als één schakelaar voor de UI.
    ///
    /// Aanzetten kondigt de eerste camera van deze machine aan als gewone stream — er
    /// wordt nog niets opgenomen en het lampje blijft uit tot iemand er echt naar kijkt,
    /// net als bij een gedeeld scherm. Idempotent: al in de gevraagde stand is een no-op,
    /// zodat twee snelle klikken geen tweede camerastream opleveren.
    fn zet_camera(&mut self, aan: bool) {
        let eigen = self
            .streams
            .eigen()
            .iter()
            .find(|s| s.kind == StreamKind::CAMERA)
            .map(|s| s.id);

        match (aan, eigen) {
            (true, None) => {
                let bronnen = match deelbare_bronnen() {
                    Ok(b) => b,
                    Err(e) => {
                        tracing::error!(error = %format!("{e:#}"), "camera's niet op te vragen");
                        self.fout = Some(format!("camera: {e:#}"));
                        return;
                    }
                };
                match bronnen.into_iter().find(|b| b.soort == BronSoort::Camera) {
                    Some(camera) => {
                        // Anders dan bij een scherm gaat de opname hier meteen aan: het
                        // eigen venster is de reden dat je de camera aanzet, en zonder
                        // opname is er niets in te zien. Het lampje gaat dus aan zodra je
                        // hem aanzet — dat is precies wat "ik wil mezelf zien" betekent.
                        if let Some(id) = self.deel_bron(camera) {
                            if let Err(e) = self.start_deler(id, Vec::new()) {
                                tracing::error!(error = %format!("{e:#}"), "camera starten mislukt");
                                self.fout = Some(format!("camera: {e:#}"));
                                // Niet half aan laten staan: de aankondiging weer intrekken.
                                self.stop_met_delen(id);
                            }
                        }
                    }
                    None => {
                        self.fout = Some(
                            "geen camera gevonden; zit hij erin en gebruikt iets anders hem niet?"
                                .into(),
                        );
                    }
                }
            }
            (false, Some(id)) => self.stop_met_delen(id),
            _ => {}
        }
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
                    // Een camera loopt al vóór de eerste kijker, want daar hangt het eigen
                    // voorbeeldvenster aan. Dan is "de eerste kijker" niets anders dan een
                    // adres erbij — tenzij die deler intussen gestopt is (venster dicht,
                    // of eruit geklapt), en dan hoort hij opnieuw op.
                    match self.delers.get(&stream_id) {
                        Some(d) if !d.gestopt() => d.zet_kijkers(kijkers),
                        _ => {
                            if let Err(e) = self.start_deler(stream_id, kijkers) {
                                tracing::error!(error = %format!("{e:#}"), stream = stream_id, "delen starten mislukt");
                                self.fout = Some(format!("scherm delen: {e:#}"));
                            }
                        }
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
                    // De laatste kijker weg betekent niet dat de deler weg moet: draagt
                    // deze stream een eigen voorbeeldvenster, dan blijf je jezelf zien.
                    // Alleen de kijkerslijst gaat leeg, en dan codeert de deel-lus niets
                    // meer. Het echte opruimen doet `stop_met_delen`.
                    if self.heeft_voorbeeld(stream_id) {
                        if let Some(d) = self.delers.get(&stream_id) {
                            d.zet_kijkers(Vec::new());
                        }
                    } else {
                        self.delers.remove(&stream_id);
                    }
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

    /// Zet het meeluisteren op andermans bureaubladgeluid gelijk aan wat we van hem
    /// bekijken. Staat in de lus en niet bij één commando, want de aanleiding kan ook
    /// een late aankondiging, een gesloten venster of een deelname aan het gesprek zijn.
    fn volg_geluid_bij_beeld(&mut self) {
        let (cmds, acties) = self.streams.stem_geluid_af_op_beeld(self.voice.is_some());
        if cmds.is_empty() && acties.is_empty() {
            return;
        }
        self.stuur_alles(cmds);
        self.voer_uit(acties);
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
        // Eerst de oude weg, dán de nieuwe erbij. Bij `insert` zou de oude pas ná het
        // opzetten van de nieuwe vallen, en een camera is dan nog door hem bezet.
        drop(self.delers.remove(&stream_id));

        let d3d = self.d3d()?;
        let voorbeeld = Self::voorbeeld_titel(&bron);
        let handle = fitcom_video::deel(
            &d3d,
            DelerConfig {
                stream_id,
                bron,
                codec: self.codec(),
                fps: self.cfg.video.fps,
                bitrate: self.cfg.video.bitrate,
                voorbeeld,
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

        // De codec hier is een startgok: de kijker leest de echte codec van de
        // pakketten zelf en wisselt zijn decoder als de deler iets anders stuurt.
        // Een gok die deze machine niet eens kán decoderen (HEVC zonder
        // Store-extensie, HEVC op macOS) liet het opzetten meteen stranden —
        // daarom valt hij terug op H.264, dat overal decodeert.
        let gok = self.codec();
        let gok = if gok.kan_decoderen() {
            gok
        } else {
            Codec::H264
        };

        let handle = fitcom_video::kijk(
            &d3d,
            KijkerConfig {
                stream_id,
                titel: format!("{naam} — {titel}"),
                breedte,
                hoogte,
                codec: gok,
                afzender: ip,
            },
        )?;

        // Nu pas intekenen: in het abonnement staat de poort van dit venster, en die
        // bestaat pas als het venster er is.
        let cmds = self
            .streams
            .kijker_draait(eigenaar, stream_id, handle.poort);
        self.stuur_alles(cmds);

        // Meldingen van het kijkvenster meteen de motorlus in laten vallen. Het venster
        // leeft op een eigen thread met een crossbeam-kanaal, en tokio kan daar niet op
        // wachten — vandaar dit ene doorgeefluik, dat vanzelf ophoudt zodra de kijker
        // stopt en zijn kant van het kanaal loslaat.
        let door = self.kijker_tx.clone();
        let ontvangen = handle.events.clone();
        std::thread::Builder::new()
            .name(format!("fitcom-kijkerpost-{stream_id}"))
            .spawn(move || {
                while let Ok(ev) = ontvangen.recv() {
                    if door.send((eigenaar, stream_id, ev)).is_err() {
                        break;
                    }
                }
            })
            .context("doorgeefthread voor kijkermeldingen starten")?;

        self.kijkers.insert((eigenaar, stream_id), handle);
        Ok(())
    }

    /// Eén melding van een kijkvenster: gesloten, een keyframe nodig, of een nieuwe
    /// miniatuur.
    ///
    /// Komt binnen via de select-lus en niet op de tik. Dat verschil is niet cosmetisch:
    /// een kijker die een fragment mist toont geen enkel beeld meer tot er een keyframe
    /// is, dus elke milliseconde die het verzoek hier blijft liggen is een milliseconde
    /// bevroren beeld aan de andere kant. Op de tik was dat tot honderd milliseconde —
    /// de helft van de gemeten hapering. Zie `crates/video/src/fragment.rs`.
    fn lees_kijker(&mut self, eigenaar: PeerId, stream_id: u32, ev: KijkerEvent) {
        match ev {
            KijkerEvent::KeyframeNodig => {
                let cmds = self.streams.vraag_keyframe(eigenaar, stream_id);
                self.stuur_alles(cmds);
            }
            KijkerEvent::Miniatuur(m) => {
                self.miniaturen.insert((eigenaar, stream_id), m);
            }
            KijkerEvent::Gesloten => {
                self.miniaturen.remove(&(eigenaar, stream_id));
                let (cmds, acties) = self.streams.stop_kijken(eigenaar, stream_id);
                self.stuur_alles(cmds);
                self.voer_uit(acties);
            }
        }
    }

    // -- UI ----------------------------------------------------------------

    fn op_ui_command(&mut self, cmd: UiCommand) {
        match cmd {
            UiCommand::Plaats(tekst, channel, reply_to) => {
                let r = self.chat.plaats_bericht(&tekst, channel, reply_to);
                self.verwerk(r);
                // Het antwoord is verstuurd: de "typt"-indicatie mag weg.
                self.typing_verzonden.remove(&channel);
            }
            UiCommand::Reageer(doel, emoji) => {
                let r = self.chat.reageer(doel, &emoji);
                self.verwerk(r);
            }
            UiCommand::Typing(channel) => self.stuur_typing(channel),
            UiCommand::ZetStatus(status) => self.zet_status(status),
            UiCommand::ZetClips(aan) => {
                if self.cfg.clips.enabled != aan {
                    self.cfg.clips.enabled = aan;
                    if let Err(e) = self.cfg.save(&self.config_path) {
                        self.fout = Some(format!("config opslaan: {e:#}"));
                    }
                }
                self.zet_clips_motor(aan);
                // Bij een mislukte start staat de motor ook echt uit; de schakelaar
                // mag niet liegen over wat er draait.
                if aan && !self.clips.aan() {
                    self.cfg.clips.enabled = false;
                    let _ = self.cfg.save(&self.config_path);
                }
            }
            UiCommand::ClipseNu => {
                tracing::info!(
                    enabled = self.cfg.clips.enabled,
                    opname_loopt = self.clips.aan(),
                    "clip-save aangevraagd via de knop"
                );
                if self.cfg.clips.enabled && self.clips.aan() {
                    self.clips.bewaar_nu();
                } else {
                    self.fout = Some("Clips staan uit (instellingen → clips).".into());
                }
            }
            UiCommand::ZetClipMonitor(naam) => {
                let gekozen = if naam.trim().is_empty() {
                    None
                } else {
                    Some(naam)
                };
                if self.cfg.clips.monitor != gekozen {
                    self.cfg.clips.monitor = gekozen.map(|s| s.to_string());
                    if let Err(e) = self.cfg.save(&self.config_path) {
                        self.fout = Some(format!("config opslaan: {e:#}"));
                        return;
                    }
                    // Draait de opname al? Dan meteen herstarten op het nieuwe
                    // scherm — een wisseling halverwege kan de keten niet aan,
                    // maar een paar seconden opnieuw opstarten is genoeg troost.
                    let stond_aan = self.clips.aan();
                    if stond_aan {
                        self.zet_clips_motor(false);
                        self.zet_clips_motor(true);
                    }
                }
            }
            UiCommand::ZetClipsHotkey(spec) => {
                let spec = spec.trim().to_string();
                // Eerst valideren: een typefout mag de werkende toets niet slopen.
                if let Err(e) = crate::clips::ontled_hotkey(&spec) {
                    self.fout = Some(format!("sneltoets: {e:#}"));
                    return;
                }
                self.cfg.clips.hotkey = spec.clone();
                if let Err(e) = self.cfg.save(&self.config_path) {
                    self.fout = Some(format!("config opslaan: {e:#}"));
                    return;
                }
                // Oude draad weg (Drop → WM_QUIT → unregister), nieuwe eraan. De
                // verzender naar onszelf blijft dezelfde: de tik leest het kanaal uit.
                self.hotkey_draad =
                    crate::clips::start_hotkey(&spec, {
                        let tx = self.clip_hotkey_verzender.clone();
                        move || {
                            let _ = tx.send(());
                        }
                    })
                    .unwrap_or_else(|e| {
                        tracing::warn!(error = %format!("{e:#}"), "nieuwe clip-hotkey mislukt");
                        self.fout = Some(format!("sneltoets: {e:#}"));
                        crate::clips::HotkeyDraad::dode()
                    });
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
                if self.files.verwijder_aanbod(doel) {
                    self.bewaar_paden();
                }
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
            UiCommand::DeelBron(bron) => {
                self.deel_bron(bron);
            }
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
            UiCommand::ZetGeluidInstellingen(geluid) => {
                // Volume begrenzen op wat het betekent, niet vertrouwen op de schuif: dit
                // komt uit de webview en 1.4 zou de tonen laten vervormen.
                // Door dezelfde opschoning als bij het inlezen: de webview kan een NaN of
                // een 1.4 doorgeven, en dat zou stilte of vervorming opleveren.
                let mut veilig = SoundConfig {
                    // Een onbekende naam niet opslaan: dan zou de config iets bevatten dat
                    // nergens naar verwijst en zou er in de kiezer niets geselecteerd staan.
                    // De frontend stuurt alleen namen die hij van de motor kreeg, dus dit is
                    // een grens en geen verwacht geval.
                    set: match geluid::Geluidset::van_naam(&geluid.set) {
                        Some(_) => geluid.set,
                        None => {
                            tracing::warn!(gekozen = %geluid.set, "onbekende geluidset geweigerd");
                            self.cfg.sound.set.clone()
                        }
                    },
                    volume: geluid.volume,
                };
                veilig.herstel();
                if veilig == self.cfg.sound {
                    // Niets veranderd: niet opslaan (dat is een schrijfactie per klik) en
                    // niet afspelen (dan piept de app bij elk hertekenen van het scherm).
                    return;
                }
                self.cfg.sound = veilig;
                if let Err(e) = self.cfg.save(&self.config_path) {
                    tracing::warn!(error = %format!("{e:#}"), "geluidsinstellingen niet opgeslagen");
                    self.fout = Some(format!("instellingen opslaan: {e:#}"));
                }
                // Meteen laten horen wat je gekozen hebt.
                self.speel_geluid(geluid::Geluid::EigenJoin);
            }
            UiCommand::ProefGeluid(naam) => match geluid::Geluid::van_naam(&naam) {
                Some(g) => self.speel_geluid(g),
                None => tracing::warn!(?naam, "onbekend geluid gevraagd voor de proef"),
            },
            UiCommand::ZetGeluidsapparaten(invoer, uitvoer) => {
                self.cfg.input_device = invoer;
                self.cfg.output_device = uitvoer;
                if let Err(e) = self.cfg.save(&self.config_path) {
                    tracing::warn!(error = %format!("{e:#}"), "geluidsapparaten niet opgeslagen");
                    self.fout = Some(format!("instellingen opslaan: {e:#}"));
                }
                // De apparaten worden één keer gekozen, bij het openen van de sessie
                // (`fitcom_audio::start`). Zit je in een gesprek, dan is opnieuw openen
                // de enige manier om te wisselen; `bind_met_geduld` vangt op dat de
                // mediapoort nog even bezet is.
                if self.voice.is_some() {
                    self.verlaten();
                    self.deelnemen();
                }
            }
            UiCommand::ZetDownloadMap(pad) => {
                if let Err(e) = std::fs::create_dir_all(&pad) {
                    tracing::warn!(error = %e, pad = %pad.display(), "downloadmap aanmaken mislukt");
                    self.fout = Some(format!("downloadmap aanmaken: {e}"));
                } else {
                    self.cfg.download_dir = Some(pad.clone());
                    if let Err(e) = self.cfg.save(&self.config_path) {
                        tracing::warn!(error = %format!("{e:#}"), "downloadmap niet opgeslagen");
                        self.fout = Some(format!("instellingen opslaan: {e:#}"));
                    }
                    self.downloads_dir = pad;
                }
            }
            UiCommand::StopDelen(id) => self.stop_met_delen(id),
            UiCommand::ZetCamera(aan) => self.zet_camera(aan),
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
            UiCommand::ZoekUpdate => self.zoek_update(true),
            UiCommand::PasUpdateToe => self.pas_update_toe(),
            UiCommand::NegeerUpdate(versie) => self.updates.negeer(&versie),
            UiCommand::WisUpdateMelding => self.updates.wis_melding(),
            UiCommand::WordleGok(woord) => self.wordle_gok(&woord),
            UiCommand::WordleInChat => self.wordle_in_chat(),
        }
    }

    // -- wordle (2026-08-20) -----------------------------------------------

    /// Eén gok. De uitslag van een afgerond spel gaat meteen de oplog in, zodat de anderen
    /// hem zien zonder dat er iets extra's over de draad hoeft.
    fn wordle_gok(&mut self, woord: &str) {
        if let wordle::Gok::Klaar {
            dag,
            pogingen,
            gewonnen,
            patroon,
        } = self.wordle.gok(woord)
        {
            tracing::info!(dag, pogingen, gewonnen, "wordle afgerond");
            let r = self.chat.meld_wordle(dag, pogingen, gewonnen, patroon);
            self.verwerk(r);
        }
    }

    /// Op elke tik: staat het raadsel van vandaag er nog niet, dan halen we het op.
    /// `moet_ophalen` houdt zelf bij dat dat hoogstens één keer per kwartier gebeurt, dus
    /// dit is in rust een maplookup en niets meer.
    fn wordle_tick(&mut self) {
        if let Some(datum) = self.wordle.moet_ophalen() {
            let tx = self.file_tx.clone();
            tokio::spawn(wordle_taak(datum, false, tx));
        }
    }

    /// Het `+`-menu: zet de kaart van vandaag in het algemene kanaal, voor iedereen.
    ///
    /// Twee gevallen, en het tweede is waar dit voor bestaat:
    ///
    /// - **Het raadsel staat er al.** Dan meteen de op, en klaar.
    /// - **Het raadsel ontbreekt** (jouw ophaal is mislukt). Dan eerst halen — buiten de
    ///   kwartierklok om, want daarom druk je erop — en pas aankondigen als hij binnen is.
    ///   Aankondigen wat we zelf niet hebben zou een kaart zonder nummer opleveren die
    ///   niemand kan spelen. Lukt het halen niet, dan komt de reden in het venster; de
    ///   automatische poging blijft stil, deze niet, want hier staat iemand te kijken.
    ///
    /// Tweemaal drukken is geen probleem: de log houdt per dag de eerste kaart (invariant
    /// 6, en `fitcom_store::WordleCardEntry` legt uit hoe).
    fn wordle_in_chat(&mut self) {
        match self.wordle.nu_ophalen() {
            // Niets te halen betekent op deze plek: we hebben hem al.
            None => self.kondig_wordle_aan(),
            Some(datum) => {
                tracing::info!(datum, "wordle handmatig opgevraagd om in de chat te zetten");
                self.wordle_aankondigen = true;
                let tx = self.file_tx.clone();
                tokio::spawn(wordle_taak(datum, true, tx));
            }
        }
    }

    /// De op die de kaart bij iedereen in de tijdlijn zet. Doet niets zonder raadsel — dan
    /// is er geen dagnummer om mee te sturen.
    fn kondig_wordle_aan(&mut self) {
        let dag = self.wordle.huidige_dag();
        let Some(nummer) = self.wordle.nummer_van(dag) else {
            return;
        };
        tracing::info!(dag, nummer, "wordle-kaart in de chat gezet");
        let r = self.chat.zet_wordle_kaart(dag, nummer);
        self.verwerk(r);
    }

    /// Bij het starten: staat er een afgerond spel van vandaag op schijf waarvan de uitslag
    /// niet in de log zit, dan komt hij er alsnog in. Zelfde gedachte als `zet_naam`, dat
    /// ook elke start controleert of de log klopt met wat hier waar is.
    fn wordle_inhaalslag(&mut self) {
        if let Some((dag, pogingen, gewonnen, patroon)) = self.wordle.te_melden() {
            let r = self.chat.meld_wordle(dag, pogingen, gewonnen, patroon);
            self.verwerk(r);
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
        // B-43: de oplog weigert een bijnaam boven `MAX_NAAM_LEN` *bytes*. Hier klemmen in
        // plaats van daar laten falen, want anders zou een te lange naam uit een
        // handgeschreven `config.toml` bij élke start opnieuw stuklopen en nooit
        // propageren — een foutmelding die je niet kunt wegnemen zonder het bestand te
        // bewerken. Klemmen gebeurt op een char-grens: midden in een multibyte-teken
        // knippen levert geen geldige `String` op (en 16 emoji zitten al op 64 bytes).
        let naam: String = if naam.len() > fitcom_proto::op::MAX_NAAM_LEN {
            let geknipt: String = naam
                .char_indices()
                .take_while(|(i, c)| i + c.len_utf8() <= fitcom_proto::op::MAX_NAAM_LEN)
                .map(|(_, c)| c)
                .collect();
            tracing::warn!(
                bytes = naam.len(),
                max = fitcom_proto::op::MAX_NAAM_LEN,
                "weergavenaam afgekapt"
            );
            geknipt
        } else {
            naam.to_string()
        };
        let naam = naam.as_str();
        self.cfg.display_name = naam.to_string();
        if let Err(e) = self.cfg.save(&self.config_path) {
            tracing::warn!(error = %format!("{e:#}"), "naam niet opgeslagen in config");
            self.fout = Some(format!("naam opslaan: {e:#}"));
        }
        let r = self.chat.zet_naam(naam);
        self.verwerk(r);
    }

    /// Schrijft de padkaarten van `files` naar schijf. Zie [`PadenIndex`] voor het
    /// waarom; falen is nooit fataal — dan werkt "openen" deze sessie nog wel en na een
    /// herstart niet meer.
    fn bewaar_paden(&self) {
        let (aangeboden, gedownload) = self.files.paden();
        if let Err(e) = schrijf_paden_index(&self.paden_index, aangeboden, gedownload) {
            tracing::warn!(error = %format!("{e:#}"), "bestandspaden niet opgeslagen");
        }
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

    /// Leest eerst het kind-byte van een inkomende uni-stream en stuurt hem dan naar het
    /// bestandspad. Zie `fitcom_net::filestream::read_kind`.
    ///
    /// B-04: `van` en de lijst lopende downloads gaan mee. Zonder die twee was elke
    /// inkomende stream goed genoeg zolang er érgens een op met die `OpId` bestond, en dat
    /// maakte ongevraagd wegschrijven mogelijk. De lijst wordt hier gekopieerd omdat de
    /// taak `self` niet mag vasthouden; hij is een momentopname van "waar wachten wij nu
    /// op", en dat is precies de vraag die telt op het moment dat de stream binnenkomt.
    fn start_incoming_stream(&mut self, van: PeerId, stream: RecvStream) {
        let downloads_dir = self.downloads_dir.clone();
        let pictures_dir = self.pictures_dir.clone();
        let timeline = self.chat.timeline_arc();
        let events = self.file_tx.clone();
        let verwacht = self.files.lopende_downloads();
        tokio::spawn(dispatch_inkomende_stream(
            van,
            stream,
            downloads_dir,
            pictures_dir,
            timeline,
            verwacht,
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
                    self.bewaar_paden();
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
            FileEvent::Voltooid { file, pad } => {
                self.files.zet_status(file, DownloadStatus::Voltooid);
                self.files.is_gedownload(file, pad);
                self.bewaar_paden();
            }
            FileEvent::Mislukt { file, bericht } => {
                tracing::warn!(?file, %bericht, "bestandsoverdracht mislukt");
                self.files
                    .zet_status(file, DownloadStatus::Mislukt(bericht));
            }
            FileEvent::UpdateGestart { versie, totaal } => {
                tracing::info!(?versie, %totaal, "nieuwere versie in de release-feed; ophalen");
                self.updates.gestart(versie, totaal);
            }
            FileEvent::UpdateVoortgang { ontvangen } => self.updates.voortgang(ontvangen),
            FileEvent::GeenUpdate => self.updates.niets_gevonden(),
            FileEvent::UpdateKlaar { pad, hash } => self.updates.klaar(pad, hash),
            FileEvent::UpdateMislukt { bericht } => {
                tracing::warn!(%bericht, "update ophalen mislukt");
                self.updates.mislukt(bericht);
            }
            FileEvent::WordleGeladen(raadsel) => {
                tracing::info!(
                    dag = raadsel.dag,
                    nummer = raadsel.nummer,
                    "wordle van vandaag binnen"
                );
                self.wordle.neem_op(raadsel);
                // Stond de knop uit het +-menu hierop te wachten, dan kan de kaart nu.
                if std::mem::take(&mut self.wordle_aankondigen) {
                    self.kondig_wordle_aan();
                }
            }
            FileEvent::WordleMislukt(reden) => {
                self.wordle_aankondigen = false;
                self.wordle.fout = Some(reden);
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

    /// Een kort geluidje bij iets dat je anders alleen ziet als je op dat moment naar het
    /// venster kijkt: iemand die het gesprek in of uit gaat, of een stream die aan of uit
    /// gaat.
    ///
    /// Niet-storen zet ze uit. Dat is dezelfde regel als voor meldingen, en om dezelfde
    /// reden: deze geluidjes zijn onderbrekingen, en dat is precies waar die stand voor is.
    /// Mute en deafen doen hier níéts: die gaan over het gesprek, niet over de app.
    fn geluid(&self, g: geluid::Geluid) {
        if self.niet_storen {
            return;
        }
        self.speel_geluid(g);
    }

    /// Hetzelfde, maar zonder de niet-storencontrole: voor de proefknop in de instellingen.
    /// Wie daarop drukt vraagt erom, en een knop die in niet-storen niets doet leest als
    /// een stukke knop.
    fn speel_geluid(&self, g: geluid::Geluid) {
        geluid::speel(self.geluidset(), g, self.cfg.sound.volume);
    }

    /// De gekozen set uit de config. Een onbekende naam — een config van een nieuwere
    /// build, of een typefout van de hand — valt terug op de standaard in plaats van
    /// stilte, want stilte is niet te onderscheiden van "de geluidjes zijn stuk".
    fn geluidset(&self) -> geluid::Geluidset {
        geluid::Geluidset::van_naam(&self.cfg.sound.set).unwrap_or_else(|| {
            tracing::warn!(gekozen = %self.cfg.sound.set, "onbekende geluidset in de config; standaard gebruikt");
            geluid::Geluidset::STANDAARD
        })
    }

    /// Hoeveel *zichtbare* streams (scherm, venster of camera) deze peer op dit moment
    /// aanbiedt. Bureaubladgeluid telt niet mee: dat gaat automatisch met een scherm mee
    /// en is geen aparte gebeurtenis.
    ///
    /// Gebruikt om aan het verschil vóór en ná een control-bericht te zien of er echt iets
    /// aan of uit ging. Aan de aankondiging zelf is dat niet te zien: die komt bij elke
    /// herverbinding opnieuw langs, en dan hoort er geen geluidje bij.
    fn zichtbare_streams_van(&self, peer: PeerId) -> usize {
        self.streams
            .vreemd()
            .iter()
            .filter(|s| s.eigenaar == peer && s.kind.is_beeld())
            .count()
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

    /// "Ik ben aan het typen" naar de anderen sturen, hooguit één keer per
    /// `TYPING_THROTTLE` per kanaal. Een DM gaat — net als een DM-bericht zelf —
    /// uitsluitend naar de geadresseerde.
    fn stuur_typing(&mut self, channel: Channel) {
        let nu = Instant::now();
        match self.typing_verzonden.get(&channel) {
            Some(t) if nu.duration_since(*t) < TYPING_THROTTLE => return,
            _ => {}
        }
        self.typing_verzonden.insert(channel, nu);
        let cmd = match channel.dm_peer() {
            Some(naar) => MeshCommand::Send {
                to: naar,
                msg: ControlMsg::Typing(Typing { channel }),
            },
            None => MeshCommand::Broadcast(ControlMsg::Typing(Typing { channel })),
        };
        self.stuur_alles(vec![cmd]);
    }

    /// Eigen status wisselen en iedereen op de hoogte brengen. Vluchtig: er zit geen op
    /// achter, dus wie offline was ziet de wisseling pas na zijn verbinding — daarom de
    /// extra melding bij het opkomen van een verbinding hierboven.
    fn zet_status(&mut self, status: u8) {
        let status = UserStatusValue(status);
        if !status.is_known() || status == self.eigen_status {
            return;
        }
        self.eigen_status = status;
        let cmds = self
            .verbonden
            .keys()
            .copied()
            .map(|to| MeshCommand::Send {
                to,
                msg: ControlMsg::UserStatus(UserStatus { status }),
            })
            .collect();
        self.stuur_alles(cmds);
    }

    fn verwerk(&mut self, r: Result<Vec<MeshCommand>>) {        match r {
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
                // Alleen tonen zolang hij er ook is; een status van een offline peer
                // is tegen de tijd dat hij terugkomt toch verlopen.
                if self.verbonden.contains_key(&id) {
                    p.user_status = self.peer_status.get(&id).copied();
                }
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
                is_camera: s.kind == StreamKind::CAMERA,
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
                is_camera: s.kind == StreamKind::CAMERA,
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
                local_path: self.files.lokaal_pad(f.id).map(Path::to_path_buf),
            })
            .collect();

        let _ = self.snap_tx.send(Arc::new(Snapshot {
            timeline: self.chat.timeline_arc(),
            peers,
            voice,
            eigen_streams,
            streams,
            video: self.cfg.video.clone(),
            geluid: self.cfg.sound.clone(),
            input_device: self.cfg.input_device.clone(),
            output_device: self.cfg.output_device.clone(),
            download_dir: self.downloads_dir.clone(),
            files,
            ongelezen: self.chat.ongelezen,
            ongelezen_dm: self.chat.ongelezen_dm().clone(),
            ongelezen_topic: self.chat.ongelezen_topic().clone(),
            niet_storen: self.niet_storen,
            clips: self
                .clips
                .aanwezig()
                .then(|| self.clips.weergave(&self.cfg.clips)),
            typing: self
                .typing_tot
                .iter()
                .filter(|((p, _), _)| self.verbonden.contains_key(p))
                .map(|((p, c), _)| (*c, *p))
                .collect(),
            eigen_status: self.eigen_status,
            update: self.updates.status().cloned(),
            wordle: WordleView {
                dag: self.wordle.huidige_dag(),
                bord: self.wordle.bord(),
                nummers: self.wordle.bekende_dagen().collect(),
                fout: self.wordle.fout.clone(),
            },
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

/// B-03: maakt van een naam die van een peer komt een naam die gegarandeerd één
/// bestandsnaam ís, en geen pad.
///
/// De reden dat dit bestaat: `FileEntry.name` is letterlijk overgenomen uit de op van de
/// peer en `Path::join` normaliseert `..` niet. Op Windows vervangt een absoluut of rooted
/// argument de basis zelfs volledig, dus `..\..\..\Startup\x.exe`,
/// `C:\Users\...\Startup\x.exe` en `\\aanvaller\share\x` landden alle drie buiten de
/// downloadmap — een exe in de Startup-map is code-uitvoering bij de volgende aanmelding,
/// zonder dat de gebruiker ook maar iets aanklikt.
///
/// Waarom filteren en niet weigeren: een naam die niet door de beugel kan is geen reden om
/// de overdracht te laten mislukken (dat zou een peer met een rare bestandsnaam onterecht
/// stukmaken), maar wél om er een onschuldige naam van te maken.
///
/// De gereserveerde DOS-namen staan erbij omdat `CON`, `NUL` en `COM1` op Windows nog
/// steeds apparaten zijn, ook mét extensie: schrijven naar `NUL.txt` schrijft naar het
/// bit-vat in plaats van naar een bestand.
fn veilige_bestandsnaam(naam: &str) -> String {
    // `file_name()` haalt elk pad-voorvoegsel eraf; het filter daarna dekt de scheidingstekens
    // die op het *andere* platform gelden (een Windows-peer die aan een mac levert, en
    // andersom) plus wat NTFS toch al weigert.
    let kaal = Path::new(naam)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    let kaal: String = kaal
        .chars()
        .filter(|c| {
            !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') && !c.is_control()
        })
        .collect();
    // Punten en spaties aan de randen: Windows kapt die zelf af, waardoor "evil.exe ." en
    // "evil.exe" hetzelfde bestand zijn en een controle op de naam te omzeilen valt.
    let kaal = kaal.trim_matches(|c: char| c == '.' || c == ' ');
    const GERESERVEERD: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let stam = kaal.split('.').next().unwrap_or("").to_ascii_uppercase();
    if kaal.is_empty() || GERESERVEERD.contains(&stam.as_str()) {
        return "bestand".into();
    }
    // Afkappen op chars en niet op bytes: een multibyte-naam mag geen paniek geven.
    kaal.chars().take(120).collect()
}

/// Waar de bytes van elk bestand op *deze* pc staan, over een herstart heen.
///
/// Puur lokaal: dit gaat nooit de draad op en is geen op. Het staat er omdat het pad niet
/// te herleiden is uit wat wél gesynct wordt. Bij een aanbod is dat het bestand van de
/// gebruiker zelf, ergens op schijf; bij een download de definitieve naam ná verificatie,
/// die bij een botsing `naam (2).ext` is geworden. Zonder deze afdruk kon de UI na een
/// herstart geen gedownload bestand meer openen, en — dat was al zo vóór dit bestand
/// bestond — kon een andere peer een bestand dat jij aanbood niet meer ophalen zodra jij
/// de app één keer had herstart.
///
/// JSON en geen TOML: de sleutel is een `OpId` (auteur + kanaal + seq) en dat is geen
/// TOML-tabelnaam. Een lijst van paren dus, geen map.
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct PadenIndex {
    #[serde(default)]
    aangeboden: Vec<(OpId, PathBuf)>,
    #[serde(default)]
    gedownload: Vec<(OpId, PathBuf)>,
}

/// Leest de padkaarten terug. Elk pad dat er niet meer is valt af: een bestand dat de
/// gebruiker intussen verplaatst of weggegooid heeft levert dan gewoon geen openknop op,
/// in plaats van een knop die niets doet — en een aanbod dat wij niet meer kunnen leveren
/// wordt weer eerlijk `NOT_AVAILABLE` in plaats van een upload die halverwege stukloopt.
///
/// Een ontbrekend of onleesbaar bestand is geen fout: dan is er niets te herstellen.
fn lees_paden_index(pad: &Path) -> (HashMap<OpId, PathBuf>, HashMap<OpId, PathBuf>) {
    let Ok(tekst) = std::fs::read_to_string(pad) else {
        return (HashMap::new(), HashMap::new());
    };
    let index: PadenIndex = match serde_json::from_str(&tekst) {
        Ok(i) => i,
        Err(e) => {
            tracing::warn!(error = %e, pad = %pad.display(), "bestandspaden onleesbaar; genegeerd");
            return (HashMap::new(), HashMap::new());
        }
    };
    let bestaand = |paren: Vec<(OpId, PathBuf)>| -> HashMap<OpId, PathBuf> {
        paren.into_iter().filter(|(_, p)| p.exists()).collect()
    };
    (bestaand(index.aangeboden), bestaand(index.gedownload))
}

fn schrijf_paden_index(
    pad: &Path,
    aangeboden: &HashMap<OpId, PathBuf>,
    gedownload: &HashMap<OpId, PathBuf>,
) -> Result<()> {
    let index = PadenIndex {
        aangeboden: aangeboden.iter().map(|(k, v)| (*k, v.clone())).collect(),
        gedownload: gedownload.iter().map(|(k, v)| (*k, v.clone())).collect(),
    };
    let tekst = serde_json::to_string_pretty(&index).context("bestandspaden serialiseren")?;
    // Via een tijdelijk bestand: een halve schrijfactie bij een stroomstoring mag geen
    // onleesbare index achterlaten — en dan zou `lees_paden_index` bij de volgende start
    // alles weggooien in plaats van één regel.
    let tijdelijk = pad.with_extension("json.tmp");
    std::fs::write(&tijdelijk, tekst).context("bestandspaden schrijven")?;
    std::fs::rename(&tijdelijk, pad).context("bestandspaden op hun plek zetten")?;
    Ok(())
}

/// De naam waaronder het bestand definitief landt. Voegt `" (2)"` etc. toe als de naam
/// al bestaat — bijvoorbeeld omdat twee peers hetzelfde bestand aanboden.
///
/// B-03: `naam` komt van een peer, dus hij gaat eerst door [`veilige_bestandsnaam`]. Het
/// resultaat ligt daarna per constructie onder `dir`, en de `debug_assert!` onderaan houdt
/// dat zo als hier ooit iets bijkomt.
fn unieke_bestandsnaam(dir: &Path, naam: &str) -> PathBuf {
    let naam = veilige_bestandsnaam(naam);
    let naam = naam.as_str();
    let kandidaat = dir.join(naam);
    if !kandidaat.exists() {
        debug_assert!(kandidaat.starts_with(dir), "B-03: pad buiten de doelmap");
        return kandidaat;
    }

    let pad = Path::new(naam);
    let stam = pad.file_stem().and_then(|s| s.to_str()).unwrap_or(naam);
    let ext = pad.extension().and_then(|s| s.to_str());
    // B-44: `2u64..` in plaats van `2u32..`, zodat de `unreachable!()` hieronder ook bij een
    // extreme naamcollisie onbereikbaar blijft in plaats van te overflowen. Kost niets.
    for i in 2u64.. {
        let naam_n = match ext {
            Some(e) => format!("{stam} ({i}).{e}"),
            None => format!("{stam} ({i})"),
        };
        let kandidaat = dir.join(&naam_n);
        if !kandidaat.exists() {
            debug_assert!(kandidaat.starts_with(dir), "B-03: pad buiten de doelmap");
            return kandidaat;
        }
    }
    unreachable!("dir.join blijft nieuwe paden opleveren")
}

/// Blake3-hash van een heel bestand, synchroon. Alleen aanroepen vanuit `spawn_blocking`.
fn blake3_hash_bestand(pad: &Path) -> std::io::Result<(u64, [u8; 32])> {
    let mut bestand = std::fs::File::open(pad)?;
    let mut hasher = blake3::Hasher::new();
    let grootte = std::io::copy(&mut bestand, &mut hasher)?;
    Ok((grootte, *hasher.finalize().as_bytes()))
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
            let (grootte, hash) = blake3_hash_bestand(&leespad)?;

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

/// Leest het kind-byte van een inkomende bulk-stream en stuurt hem naar het bestandspad.
/// Zie `fitcom_net::filestream::read_kind`.
async fn dispatch_inkomende_stream(
    van: PeerId,
    mut stream: RecvStream,
    downloads_dir: PathBuf,
    pictures_dir: PathBuf,
    timeline: Arc<Timeline>,
    verwacht: HashSet<OpId>,
    events: mpsc::Sender<FileEvent>,
) {
    match fitcom_net::filestream::read_kind(&mut stream).await {
        Ok(fitcom_net::filestream::Inkomend::Bestand(file)) => {
            download_taak(
                van,
                stream,
                file,
                downloads_dir,
                pictures_dir,
                timeline,
                verwacht,
                events,
            )
            .await;
        }
        // Fase 11-pad, sinds fase 13 dicht: een peer die ons een exe wil toeschuiven
        // wordt afgewezen, ongeacht wat hij erbij beweert. Zie B-01 in
        // `docs/BEVEILIGING.md` — dit was de wormstap.
        Ok(fitcom_net::filestream::Inkomend::Update) => {
            tracing::warn!("update-stream van een peer geweigerd; updates komen uit de feed");
        }
        Err(e) => {
            tracing::warn!(error = %format!("{e:#}"), "kind van inkomende overdracht onleesbaar");
        }
    }
}

/// Zoekt het bijbehorende bestand op in de (op het moment van binnenkomst al bekende)
/// timeline, en downloadt het.
#[allow(clippy::too_many_arguments)]
async fn download_taak(
    van: PeerId,
    mut stream: RecvStream,
    file: OpId,
    downloads_dir: PathBuf,
    pictures_dir: PathBuf,
    timeline: Arc<Timeline>,
    verwacht: HashSet<OpId>,
    events: mpsc::Sender<FileEvent>,
) {
    let Some(entry) = timeline.files.iter().find(|f| f.id == file).cloned() else {
        tracing::warn!(?file, "bestandsstream voor een onbekend bestand genegeerd");
        return;
    };

    // B-04, twee poorten. Eerst: komt deze stream van de peer die het bestand aanbood?
    // `entry.author` is de aanbieder, en dat is de enige van wie de bytes mogen komen.
    if entry.author != van {
        tracing::warn!(
            ?file,
            van = ?van,
            aanbieder = ?entry.author,
            "bestandsstream van een andere peer dan de aanbieder geweigerd"
        );
        return;
    }
    // En: hebben wij hier zelf om gevraagd? Zonder dit kan een peer ongevraagd bytes op
    // onze schijf zetten, en dat is de stap die B-03 van één klik naar nul klikken bracht.
    if !verwacht.contains(&file) {
        tracing::warn!(?file, van = ?van, "ongevraagde bestandsstream geweigerd");
        return;
    }

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

/// Het hele updatetraject in één taak: feed halen, handtekening controleren, en bij een
/// nieuwere versie de exe ophalen en tegen de ondertekende hash leggen.
///
/// Alles wat blokkeert (TLS, schijf, hashen) draait in `spawn_blocking`; `release.rs`
/// gebruikt bewust een blokkerende HTTP-client, want dit is geen heet pad en een tweede
/// async-stack ernaast zou alleen maar dependencies kosten.
/// Het raadsel van één dag ophalen. Losse taak op een blocking-thread, want `ureq` is
/// synchroon en de motor mag hier niet op wachten.
///
/// Mislukken is geen fout: `debug` en niet `warn`, precies zoals bij een YouTube-preview.
/// Offline is een normale toestand (invariant 7) en dit is een spelletje — er komt dan
/// gewoon geen kaart, en over een kwartier probeert de tik het opnieuw.
///
/// Behalve als iemand er zelf om vroeg (`handmatig`). Dan is stil mislukken juist wél
/// verkeerd: er staat iemand te kijken of er nu iets gebeurt. Alleen in dat geval komt de
/// reden terug het venster in.
async fn wordle_taak(datum: String, handmatig: bool, events: mpsc::Sender<FileEvent>) {
    let uitkomst = tokio::task::spawn_blocking(move || wordle::haal_op(&datum)).await;
    let reden = match uitkomst {
        Ok(Ok(raadsel)) => {
            let _ = events.send(FileEvent::WordleGeladen(raadsel)).await;
            return;
        }
        Ok(Err(e)) => {
            tracing::debug!(error = %format!("{e:#}"), "geen wordle van vandaag");
            "Today's puzzle could not be fetched. Is there internet?"
        }
        Err(e) => {
            tracing::warn!(error = %e, "de wordle-taak liep vast");
            "Fetching today's puzzle went wrong."
        }
    };
    if handmatig {
        let _ = events
            .send(FileEvent::WordleMislukt(reden.to_string()))
            .await;
    }
}

async fn update_check_taak(
    updates_dir: PathBuf,
    genegeerd: HashSet<String>,
    handmatig: bool,
    events: mpsc::Sender<FileEvent>,
) {
    let melder = events.clone();
    let uitkomst = tokio::task::spawn_blocking(move || {
        haal_update_op(&updates_dir, &genegeerd, handmatig, &events)
    })
    .await;
    if let Err(e) = uitkomst {
        tracing::warn!(error = %e, "update-check afgebroken");
        // Anders blijft het slot op slot en komt er nooit meer een check.
        let _ = melder
            .send(FileEvent::UpdateMislukt {
                bericht: "de update-check liep vast".into(),
            })
            .await;
    }
}

/// Blokkerende helft van `update_check_taak`. Meldt elke uitkomst zelf via `events`, want
/// de motor mag hier niet op wachten.
fn haal_update_op(
    updates_dir: &Path,
    genegeerd: &HashSet<String>,
    handmatig: bool,
    events: &mpsc::Sender<FileEvent>,
) {
    let melden = |ev: FileEvent| {
        let _ = events.blocking_send(ev);
    };

    let manifest = match release::haal_manifest() {
        Ok(m) => m,
        Err(e) => {
            // Een onbereikbare feed is bij de periodieke tik geen foutmelding waard —
            // offline is een normale toestand, en dit staat los van waar de gebruiker mee
            // bezig is. Wie er zelf om vroeg krijgt hem wél te zien: anders is "de feed
            // is stuk" niet te onderscheiden van "de knop doet niets".
            tracing::debug!(error = %format!("{e:#}"), "release-feed niet gelezen");
            melden(if handmatig {
                FileEvent::UpdateMislukt {
                    bericht: format!("{e:#}"),
                }
            } else {
                FileEvent::GeenUpdate
            });
            return;
        }
    };

    if !fitcom_proto::is_newer(&manifest.version, EIGEN_VERSIE)
        || genegeerd.contains(&manifest.version)
    {
        melden(FileEvent::GeenUpdate);
        return;
    }

    // Pas hierna wordt er iets binnengehaald: eerst moet de handtekening kloppen.
    let hash = match release::controleer(&manifest) {
        Ok(h) => h,
        Err(e) => {
            melden(FileEvent::UpdateMislukt {
                bericht: format!("{e:#}"),
            });
            return;
        }
    };

    melden(FileEvent::UpdateGestart {
        versie: manifest.version.clone(),
        totaal: manifest.size,
    });

    match download_met_voortgang(&manifest, hash, updates_dir, events) {
        Ok(pad) => melden(FileEvent::UpdateKlaar { pad, hash }),
        Err(e) => melden(FileEvent::UpdateMislukt {
            bericht: format!("{e:#}"),
        }),
    }
}

fn download_met_voortgang(
    manifest: &Release,
    hash: [u8; 32],
    updates_dir: &Path,
    events: &mpsc::Sender<FileEvent>,
) -> Result<PathBuf> {
    release::download(manifest, hash, updates_dir, |ontvangen| {
        let _ = events.blocking_send(FileEvent::UpdateVoortgang { ontvangen });
    })
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
                // B-13: de aangekondigde grootte wordt afgedwongen in plaats van alleen
                // getoond. Zonder dit las deze lus tot EOF zonder plafond, dus een peer
                // die bleef sturen schreef de schijf vol — en er was geen quotum en geen
                // vrije-ruimtecontrole. Het deelbestand gaat hier weg, want een overdracht
                // die de eigen aankondiging overschrijdt is geen hervatbare hapering maar
                // een peer die zich niet aan zijn woord houdt.
                if ontvangen + n as u64 > entry.size {
                    drop(bestand);
                    let _ = tokio::fs::remove_file(&deelpad).await;
                    anyhow::bail!(
                        "peer stuurde meer dan aangekondigd ({} > {})",
                        ontvangen + n as u64,
                        entry.size
                    );
                }
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
    let (_, hash) = tokio::task::spawn_blocking(move || blake3_hash_bestand(&te_hashen))
        .await
        .context("hash-taak afgebroken")??;
    let klopt = hash == verwacht;

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
    let _ = events
        .send(FileEvent::Voltooid {
            file: entry.id,
            pad: definitief,
        })
        .await;
    Ok(())
}

#[cfg(test)]
mod beveiliging_tests {
    use super::*;

    /// B-03: dit is de bevinding waar het doorlichtingsdocument "als er maar één ding kon"
    /// naar wijst, dus hij verdient een test die de aanvalsvormen letterlijk opnoemt.
    ///
    /// Alle drie de vormen kwamen langs `Path::join`, die `..` niet normaliseert en op
    /// Windows bij een absoluut of rooted argument de basis volledig vervangt. Een exe in de
    /// Startup-map is code-uitvoering bij de volgende aanmelding, zonder één klik.
    #[test]
    fn b03_een_bestandsnaam_van_een_peer_kan_geen_pad_meer_zijn() {
        for aanval in [
            r"..\..\..\Startup\evil.exe",
            r"C:\Users\rick\AppData\Roaming\Microsoft\Windows\Start Menu\Programs\Startup\evil.exe",
            r"\\aanvaller\share\evil",
            "../../../.zshrc",
            "/etc/cron.d/evil",
        ] {
            let veilig = veilige_bestandsnaam(aanval);
            assert!(
                !veilig.contains('/') && !veilig.contains('\\') && !veilig.contains(':'),
                "{aanval:?} leverde {veilig:?}, dat is nog een pad"
            );
            assert!(!veilig.is_empty(), "{aanval:?} leverde een lege naam op");
        }
    }

    /// En het resultaat blijft onder de doelmap. Dit is de eigenschap die telt; de test
    /// hierboven controleert de vorm, deze de uitkomst.
    #[test]
    fn b03_het_pad_blijft_onder_de_downloadmap() {
        let dir = Path::new("/tmp/fitcom-test-downloads");
        for aanval in [
            r"..\..\..\Startup\evil.exe",
            "../../../.zshrc",
            r"C:\Windows\System32\evil.dll",
        ] {
            let pad = unieke_bestandsnaam(dir, aanval);
            assert!(
                pad.starts_with(dir),
                "{aanval:?} landde op {pad:?}, buiten {dir:?}"
            );
        }
    }

    /// Stuurtekens horen er ook uit: een `\n` in een naam vervuilt logregels (B-33) en een
    /// naam die alleen uit punten bestaat is op Windows geen naam.
    #[test]
    fn b03_stuurtekens_en_randpunten_verdwijnen() {
        assert!(!veilige_bestandsnaam("regel\neen.txt").contains('\n'));
        assert_eq!(veilige_bestandsnaam("..."), "bestand");
        assert_eq!(veilige_bestandsnaam(""), "bestand");
        // Windows kapt een punt aan het eind zelf af, dus "evil.exe ." en "evil.exe" zijn
        // hetzelfde bestand; zonder trim valt een controle op de naam te omzeilen.
        assert_eq!(veilige_bestandsnaam("evil.exe ."), "evil.exe");
    }

    /// `CON`, `NUL` en `COM1` zijn op Windows apparaten, ook mét extensie: naar `NUL.txt`
    /// schrijven schrijft naar het bit-vat in plaats van naar een bestand.
    #[test]
    fn b03_gereserveerde_dos_namen_worden_geweigerd() {
        for naam in ["CON", "nul.txt", "COM1.dat", "LPT9"] {
            assert_eq!(
                veilige_bestandsnaam(naam),
                "bestand",
                "{naam:?} is een apparaatnaam en hoort niet als bestandsnaam gebruikt te worden"
            );
        }
    }

    /// Een gewone naam mag hier niets aan overhouden — anders is de fix een regressie voor
    /// iedereen die nooit iets kwaads stuurde.
    #[test]
    fn b03_een_normale_naam_blijft_ongemoeid() {
        for naam in [
            "vakantiefotos.zip",
            "notulen 2026-08-13.pdf",
            "スクリーン.png",
        ] {
            assert_eq!(veilige_bestandsnaam(naam), naam);
        }
    }

    /// Een multibyte-naam boven de afkapgrens mag geen paniek geven: afkappen gebeurt op
    /// chars, niet op bytes.
    #[test]
    fn b03_een_lange_multibyte_naam_paniekt_niet() {
        let lang = "スクリーンショット".repeat(40);
        let veilig = veilige_bestandsnaam(&lang);
        assert_eq!(veilig.chars().count(), 120);
    }
}
