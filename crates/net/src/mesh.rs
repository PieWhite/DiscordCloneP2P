//! Volledig gelijkwaardige mesh over het tailnet.
//!
//! Elke peer luistert én dialt. Er is geen host. Eén of twee peers offline is een
//! normale toestand: dialers blijven met backoff doorproberen tot het lukt.
//!
//! # Botsingen
//!
//! Twee peers die tegelijk starten dialen elkaar tegelijk, dus er ontstaan twee
//! verbindingen. Beide kanten passen dezelfde regel toe: **de verbinding waarvan de
//! initiator het laagste `PeerId` heeft wint**, de andere wordt gesloten. Omdat de
//! regel deterministisch en symmetrisch is komen beide kanten op dezelfde uitkomst uit,
//! zonder onderhandeling.
//!
//! # Identiteit (protocol 6, beslissing 46)
//!
//! Elke installatie heeft een vaste Ed25519-sleutel. Bij het verbinden tekent elke kant de
//! TLS-exporter van déze verbinding (`fitcom_proto::control::Handtekening`), en een peer
//! komt alleen binnen met de sleutel die bij zijn target vastgepind staat. Een sleutel
//! leren (trust-on-first-use) gebeurt alleen op een tailnet- of loopbackadres
//! ([`mag_leren`]); een publiek adres krijgt zijn sleutel straks uit een uitnodigingscode.
//! Aan de opnemende kant gaat de `HelloAck` pas de deur uit als de actor de beller heeft
//! goedgekeurd (B-05a): wie niet binnenkomt hoort niets terug, ook geen naam of versie.

use crate::mediasleutel::{self, MediaSleutels};
use crate::verdeler::Verdeler;
use crate::{framing, tls};
use anyhow::{bail, Context, Result};
use fitcom_proto::control::{Handtekening, Hello, HelloAck};
use fitcom_proto::{ControlMsg, PeerId, MEDIA_SLEUTEL_LABEL, PROTOCOL_VERSION};
use ring::signature::{Ed25519KeyPair, KeyPair, UnparsedPublicKey, ED25519};
use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot, watch};

const BACKOFF_START: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(30);
/// Zolang er een kandidaat buiten het tailnet is (B3): beide kanten bellen elkaar dan vaak
/// genoeg terwijl de NAT-mapping van de vorige poging nog leeft, en op een makkelijke NAT is
/// dat het hele doorprikken. Kost een paar QUIC-Initials per 10 s.
const BACKOFF_MAX_PUBLIEK: Duration = Duration::from_secs(10);
/// Hoe vaak de eigen adressen opnieuw opgezocht worden (een DHCP-wissel, Tailscale aan of uit).
const EIGEN_ELKE: u32 = 30;
/// QUIC praat over UDP, dus een peer die uit staat weigert niets — die zwijgt gewoon.
/// Zonder eigen deadline blijven we tot de idle-timeout hangen en staat er seconden
/// lang "verbinden…" in de UI terwijl er niemand thuis is.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(4);
/// Hoe vaak we een verse RTT naar de UI duwen.
const STATUS_TICK: Duration = Duration::from_secs(1);

/// Hoeveel inkomende verbindingen er tegelijk mogen lopen (B-17).
///
/// Er was geen enkele rem: elke inkomende verbinding kreeg een taak, een lezer en een
/// framebuffer, van iemand die nog niets bewezen heeft.
///
/// Bewust rúim, want te krap is hier erger dan te ruim. Eén peer kan tegelijk meer dan
/// één verbinding bij ons open hebben staan: tijdens een botsing twee, en na een harde
/// herstart blijven zijn oude verbindingen hier hangen tot de idle-timeout van 15 s ze
/// opruimt terwijl hij met een backoff van één seconde alweer belt. Zestien per
/// geconfigureerde peer dekt dat geval met marge; de ondergrens van 32 houdt een kleine
/// of lege config werkbaar. Ook zo blijft het eindig: het uitgangspunt van B-17 was
/// 500 verbindingen × 16 MiB ≈ 8 GB, en hier is het plafond bij drie peers 32 × 2 MiB.
///
/// Wie erbuiten valt krijgt `refuse()` en dus meteen antwoord, en probeert het met zijn
/// eigen backoff gewoon opnieuw — invariant 7 blijft staan.
fn max_inkomend(targets: usize) -> usize {
    targets.saturating_mul(16).max(32)
}

/// Hoeveel gevalideerde verbindingspogingen één bron-IP per seconde mag doen (A3).
///
/// Ruim: drie instanties op één pc delen 127.0.0.1, en bij het opstarten en tijdens een
/// botsing bellen ze elkaar kort na elkaar. Wie erboven zit krijgt `refuse()` en probeert
/// het met zijn eigen backoff opnieuw. Alleen gevalideerde adressen tellen mee (zie
/// [`accept_loop`]), dus een gespoofde afzender kan het budget van een echt IP niet opmaken.
const MAX_POGINGEN_PER_IP: u32 = 8;

/// Hoogstens één logregel per bron-IP per zoveel tijd over een geweigerde poging (B-25).
const LOGREM: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
pub struct MeshConfig {
    pub me: PeerId,
    pub display_name: String,
    pub control_port: u16,
    pub media_port: u16,
    pub targets: Vec<PeerTarget>,
    /// `env!("CARGO_PKG_VERSION")` van deze build. Gaat mee in `Hello`/`HelloAck`, zodat
    /// de app-laag (fase 11: automatische updates) kan zien wanneer een peer verder is.
    pub app_version: String,
    /// De vaste sleutel van deze installatie (`identity.toml`). Bewijst bij elke
    /// verbinding dat wij `me` zijn.
    pub sleutel: Arc<Ed25519KeyPair>,
}

#[derive(Debug, Clone)]
pub struct PeerTarget {
    /// Tailnet-IP of MagicDNS-naam. Leeg voor een peer die alleen uit een uitnodigingscode
    /// komt.
    pub address: String,
    pub label: String,
    /// Bekend uit eerdere sessies (trust-on-first-use).
    pub known_id: Option<PeerId>,
    pub control_port: u16,
    /// De vastgepinde Ed25519-sleutel van deze peer. Zodra die er is komt alleen wie hem
    /// bezit nog binnen, vanaf welk adres dan ook.
    pub public_key: Option<[u8; 32]>,
    /// Control-adressen om naast `address` te proberen (B3): uit zijn uitnodigingscode en
    /// geleerd van peers (B4). Hints, nooit toegang; een sleutel leren gebeurt hier nooit.
    pub kandidaten: Vec<SocketAddr>,
}

/// Een vers sleutelpaar, voor tests en hulpprogramma's zonder `identity.toml`.
pub fn wegwerpsleutel() -> Arc<Ed25519KeyPair> {
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
        .expect("het besturingssysteem levert willekeur");
    Arc::new(Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).expect("net gemaakt"))
}

/// Of we op dit adres een sleutel mogen leren (trust-on-first-use, beslissing 46).
///
/// Alleen het tailnet (`100.64.0.0/10`, `fd7a:115c:a1e0::/48`): daar is WireGuard nog de
/// grens, dus zo pinnen de bestaande installaties na de update elkaars sleutel vanzelf. En
/// loopback, voor meerdere instanties op één pc en de tests. Nooit een publiek of LAN-adres:
/// daar zou de eerste die zich meldt de identiteit kapen.
pub fn mag_leren(ip: IpAddr) -> bool {
    match ip.to_canonical() {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_loopback() || (o[0] == 100 && o[1] & 0xc0 == 64)
        }
        IpAddr::V6(v6) => v6.is_loopback() || v6.segments()[..3] == [0xfd7a, 0x115c, 0xa1e0],
    }
}

/// De exporter van deze TLS-sessie voor de gegeven rol: wat een kant tekent.
fn exporter(conn: &quinn::Connection, rol: &[u8]) -> Result<[u8; 32]> {
    let mut uit = [0u8; 32];
    conn.export_keying_material(&mut uit, Handtekening::LABEL, rol)
        .map_err(|_| anyhow::anyhow!("TLS-exporter niet beschikbaar"))?;
    Ok(uit)
}

fn teken(sleutel: &Ed25519KeyPair, conn: &quinn::Connection, rol: &[u8]) -> Result<Vec<u8>> {
    Ok(sleutel.sign(&exporter(conn, rol)?).as_ref().to_vec())
}

/// De sleutel van de ander als zijn handtekening over `exporter` klopt, anders niets.
/// Los van de verbinding zodat het zonder netwerk te testen is.
fn bewezen_sleutel(exporter: &[u8; 32], sleutel: &[u8], handtekening: &[u8]) -> Option<[u8; 32]> {
    let sleutel: [u8; 32] = sleutel.try_into().ok()?;
    UnparsedPublicKey::new(&ED25519, &sleutel)
        .verify(exporter, handtekening)
        .ok()?;
    Some(sleutel)
}

/// De mediasleutel voor wat `afzender` over deze verbinding stuurt. Beide kanten rekenen
/// hem uit; zie `mediasleutel.rs`.
fn mediasleutel_van(conn: &quinn::Connection, afzender: PeerId) -> Option<[u8; 32]> {
    let mut k = [0u8; 32];
    conn.export_keying_material(&mut k, MEDIA_SLEUTEL_LABEL, afzender.as_bytes())
        .ok()?;
    Some(k)
}

#[derive(Debug, Clone, PartialEq)]
pub enum PeerStatus {
    Offline {
        reason: String,
    },
    Connecting,
    Online {
        peer_id: PeerId,
        display_name: String,
        /// Waar de audio- en videopakketten van deze peer heen moeten: zijn IP met de
        /// mediapoort die hij in de handshake opgaf.
        media_addr: SocketAddr,
        rtt_ms: u32,
        /// `env!("CARGO_PKG_VERSION")` van de peer, uit de handshake. `"0.0.0"` voor een
        /// peer die het veld nog niet kende.
        app_version: String,
    },
    /// De peer draait een andere protocolversie. Geen crash, wel onbruikbaar tot
    /// één van beiden update. `theirs == 0`: onbekend, maar ouder (een peer op 5 sluit
    /// voordat zijn antwoord aankomt, zie `gesloten_om_versie`).
    VersionMismatch {
        theirs: u32,
        ours: u32,
    },
    /// Vanaf een bekend adres meldde zich een andere identiteit dan we eerder zagen, of
    /// dezelfde `PeerId` met een andere sleutel dan vastgepind (dan is `expected == got`).
    IdentityChanged {
        expected: PeerId,
        got: PeerId,
    },
    /// Er staat voor deze peer nog geen sleutel vast, en zijn adres ligt buiten het
    /// tailnet, waar we er geen leren ([`mag_leren`]).
    UnknownKey,
}

#[derive(Debug)]
pub enum MeshEvent {
    Status {
        target: usize,
        status: PeerStatus,
    },
    /// Eerste keer dat we de identiteit en de sleutel achter een adres leren. De app hoort
    /// beide in de config vast te leggen: vanaf dan komt alleen deze sleutel nog binnen.
    LearnedIdentity {
        target: usize,
        peer_id: PeerId,
        public_key: [u8; 32],
    },
    Message {
        from: PeerId,
        msg: ControlMsg,
    },
    /// Een nieuwe uni-stream, geopend door de andere kant voor bulkdata (bestandsoverdracht).
    /// Los van `Message`: dit gaat niet over de control-stream en heeft dus geen `ControlMsg`.
    IncomingFileStream {
        from: PeerId,
        stream: quinn::RecvStream,
    },
    /// De control-adressen waarop deze pc zichzelf bereikbaar denkt (B1): wat in de eigen
    /// uitnodigingscode hoort. Alleen bij een wijziging.
    EigenAdressen(Vec<SocketAddr>),
}

pub enum MeshCommand {
    Send {
        to: PeerId,
        msg: ControlMsg,
    },
    Broadcast(ControlMsg),
    /// Opent een eigen uni-stream naar `to` voor het versturen van bulkbytes (een
    /// bestand), naast de control-stream. `None` als die peer niet (meer) verbonden is.
    OpenUploadStream {
        to: PeerId,
        respond: oneshot::Sender<Option<quinn::SendStream>>,
    },
    /// Een peer erbij, uit een uitnodigingscode (B1). Krijgt het volgende targetnummer: de
    /// app houdt dezelfde volgorde aan.
    VoegToe(PeerTarget),
    /// Sleutel, `PeerId` en kandidaten van een bestaand target, uit zijn code (B1). De app
    /// heeft al gecontroleerd dat de sleutel niet botst met wat er vastgepind staat.
    WerkBij {
        target: usize,
        known_id: PeerId,
        public_key: [u8; 32],
        kandidaten: Vec<SocketAddr>,
    },
}

impl std::fmt::Debug for MeshCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Send { to, msg } => f
                .debug_struct("Send")
                .field("to", to)
                .field("msg", msg)
                .finish(),
            Self::Broadcast(msg) => f.debug_tuple("Broadcast").field(msg).finish(),
            Self::OpenUploadStream { to, .. } => {
                f.debug_struct("OpenUploadStream").field("to", to).finish()
            }
            Self::VoegToe(t) => f.debug_tuple("VoegToe").field(&t.label).finish(),
            Self::WerkBij { target, .. } => {
                f.debug_struct("WerkBij").field("target", target).finish()
            }
        }
    }
}

pub struct MeshHandle {
    pub commands: mpsc::Sender<MeshCommand>,
    pub events: mpsc::Receiver<MeshEvent>,
    /// Het adres waarop we daadwerkelijk luisteren.
    pub local_addr: SocketAddr,
    /// De mediasleutels van de verbonden peers. De mesh houdt ze bij bij verbinden en
    /// verbreken; [`MeshHandle::post`] versleutelt en opent ermee.
    pub media: MediaSleutels,
    /// De ene mediasocket van dit proces, op `media_port` (fase B5). Geef een kloon aan de
    /// voice-sessie, elke deler en elke kijker.
    pub post: Verdeler,
    /// De publieke sleutel van deze installatie, voor de eigen uitnodigingscode (B1).
    pub public_key: [u8; 32],
    endpoint: quinn::Endpoint,
    /// De UDP-socket onder `endpoint`, alleen om in [`MeshHandle::shutdown`] te zien wanneer
    /// niemand anders hem nog vasthoudt.
    socket: Arc<dyn quinn::AsyncUdpSocket>,
    _shutdown: ShutdownGuard,
}

/// Hoe lang [`MeshHandle::shutdown`] hoogstens wacht tot de poort vrij is.
const AFSLUIT_GRENS: Duration = Duration::from_secs(4);

impl MeshHandle {
    /// Sluit netjes af en wacht tot de UDP-poort weer vrij is.
    ///
    /// `drop` alleen is niet genoeg: dat zet de taken stil maar keert terug voordat
    /// de socket daadwerkelijk los is. Wie op dezelfde poort opnieuw wil starten —
    /// bij een config-herlaad bijvoorbeeld — moet dit gebruiken.
    ///
    /// Begrensd op [`AFSLUIT_GRENS`]. Houdt iemand buiten de mesh nog iets van quinn vast
    /// (een [`MeshSluiter`], een bestandsstream), dan keert dit toch terug, met een
    /// waarschuwing, en is de poort nog bezet.
    pub async fn shutdown(self) {
        let endpoint = self.endpoint.clone();
        let socket = self.socket.clone();
        drop(self); // zet alle taken stil via de ShutdownGuard
        endpoint.close(0u32.into(), b"afsluiten");
        let los = tokio::time::timeout(AFSLUIT_GRENS, async {
            endpoint.wait_idle().await;
            drop(endpoint);
            // `wait_idle` zegt alleen dat quinn's verbindingstabel leeg is. De socket gaat pas
            // dicht als ook elke `quinn::Connection` in onze taken weg is en quinn's eigen
            // endpoint-taak klaar is, en van die taak geeft quinn geen handvat. Dat laatste is
            // alleen te zien aan de socket zelf: elke houder heeft er een `Arc` van.
            while Arc::strong_count(&socket) > 1 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await;
        if los.is_err() {
            tracing::warn!(
                houders = Arc::strong_count(&socket) - 1,
                "UDP-poort na afsluiten nog niet vrij"
            );
        }
    }

    /// Een handvat om bij het afsluiten van de app de verbindingen te sluiten, ook nadat
    /// de handle zelf aan de motor is gegeven.
    pub fn sluiter(&self) -> MeshSluiter {
        MeshSluiter(self.endpoint.clone())
    }
}

/// Zie [`MeshHandle::sluiter`]. Zonder dit verdween de app zonder CONNECTION_CLOSE: de
/// anderen zagen je nog 15 s (de idle-timeout) online en in het gesprek en bleven audio
/// naar een dode poort sturen.
#[derive(Clone)]
pub struct MeshSluiter(quinn::Endpoint);

impl MeshSluiter {
    /// Stuurt elke peer een nette afsluiting en wacht hoogstens `max` tot die weg is.
    /// De endpoint-taak moet daarvoor nog draaien: aanroepen vóór de runtime weg is.
    pub async fn sluit(&self, max: Duration) {
        self.0.close(0u32.into(), b"afsluiten");
        let _ = tokio::time::timeout(max, self.0.wait_idle()).await;
    }
}

/// Zet alle netwerktaken stil zodra de handle wegvalt. Zonder dit blijft de UDP-poort
/// bezet en blijven dialers doorproberen nadat de app ze niet meer nodig heeft.
struct ShutdownGuard(watch::Sender<bool>, Verdeler);

impl Drop for ShutdownGuard {
    fn drop(&mut self) {
        let _ = self.0.send(true);
        self.1.stop();
    }
}

/// Wacht tot de shutdown-vlag omgaat. Keert ook terug als de zender al weg is.
async fn cancelled(rx: &mut watch::Receiver<bool>) {
    loop {
        if *rx.borrow_and_update() {
            return;
        }
        if rx.changed().await.is_err() {
            return;
        }
    }
}

// ---------------------------------------------------------------------------
// Interne berichten van de losse taken naar de actor.
// ---------------------------------------------------------------------------

struct Established {
    conn_id: u64,
    /// Bij een uitgaande verbinding het target dat we belden, bij een inkomende het target
    /// dat de actor vóór de `HelloAck` aanwees.
    target: usize,
    peer_id: PeerId,
    /// Bewezen met een handtekening over de exporter van deze verbinding.
    sleutel: [u8; 32],
    /// De sessie-index die wij kozen voor media van hem, en die hij koos voor media van ons.
    mijn_index: u32,
    hun_index: u32,
    display_name: String,
    media_port: u16,
    app_version: String,
    /// Wie deze verbinding opzette. Bepaalt wie een botsing wint.
    initiator: PeerId,
    remote: SocketAddr,
    conn: quinn::Connection,
    out: mpsc::Sender<ControlMsg>,
}

enum Internal {
    Command(MeshCommand),
    Resolved {
        target: usize,
        addr: SocketAddr,
    },
    Connecting {
        target: usize,
    },
    Established(Box<Established>),
    /// Een inkomende beller met een geldige handtekening wacht op ons oordeel.
    Inkomend(Wachtend),
    /// Een opnemende taak weigerde een poging vóór de actor eraan te pas kwam (foute
    /// handtekening). Hier en niet daar gelogd, zodat er één logrem is.
    Geweigerd {
        ip: IpAddr,
        reden: &'static str,
    },
    Rejected {
        target: usize,
        status: PeerStatus,
    },
    Frame {
        conn_id: u64,
        msg: ControlMsg,
    },
    IncomingFileStream {
        conn_id: u64,
        stream: quinn::RecvStream,
    },
    Closed {
        conn_id: u64,
        reason: String,
    },
    Tick,
}

static NEXT_CONN_ID: AtomicU64 = AtomicU64::new(1);

pub fn spawn(cfg: MeshConfig) -> Result<MeshHandle> {
    let bind: SocketAddr = format!("0.0.0.0:{}", cfg.control_port)
        .parse()
        .context("luisteradres samenstellen")?;

    // Wat `quinn::Endpoint::server` doet, maar met de socket in de hand voor `shutdown`. Bij
    // een quinn-update opnieuw naast `Endpoint::server` leggen.
    let runtime = quinn::default_runtime().context("geen async-runtime")?;
    let socket = std::net::UdpSocket::bind(bind)
        .and_then(|s| runtime.wrap_udp_socket(s))
        .with_context(|| format!("QUIC-endpoint binden op {bind}"))?;
    let mut endpoint = quinn::Endpoint::new_with_abstract_socket(
        quinn::EndpointConfig::default(),
        Some(tls::server_config()?),
        socket.clone(),
        runtime,
    )
    .with_context(|| format!("QUIC-endpoint opzetten op {bind}"))?;
    endpoint.set_default_client_config(tls::client_config()?);
    let local_addr = endpoint.local_addr().context("luisteradres opvragen")?;
    tracing::info!(%local_addr, me = %cfg.me, "mesh luistert");

    let (cmd_tx, cmd_rx) = mpsc::channel(256);
    let (event_tx, event_rx) = mpsc::channel(1024);
    let (int_tx, int_rx) = mpsc::channel(1024);
    let (stop_tx, stop_rx) = watch::channel(false);
    let media = MediaSleutels::default();
    // Bij het starten en niet bij deelnemen aan het gesprek: de keepalives houden het gat
    // in de NAT open voordat iemand praat of deelt, en de handshake noemt de echte poort.
    //
    // Is de poort bezet, dan een vrije in plaats van niet starten (invariant 7): de handshake
    // noemt de echte poort, dus de anderen vinden hem toch.
    let post = match Verdeler::start(cfg.media_port, media.clone()) {
        Ok(p) => p,
        Err(e) if cfg.media_port != 0 => {
            tracing::warn!(poort = cfg.media_port, error = %format!("{e:#}"),
                "mediapoort bezet; een vrije poort gekozen");
            Verdeler::start(0, media.clone()).context("mediapoort binden")?
        }
        Err(e) => return Err(e),
    };
    let mut cfg = cfg;
    cfg.media_port = post.poort();

    // Commando's van de UI lopen door dezelfde interne wachtrij als netwerkgebeurtenissen,
    // zodat de actor één bron van waarheid heeft en geen locks nodig heeft.
    //
    // Ook deze stopt op het stopsignaal: de motor houdt klonen van `commands`, en zolang deze
    // taak leeft stopt de actor niet en houden de verbindingen in `active` de poort bezet.
    {
        let int_tx = int_tx.clone();
        let mut stop = stop_rx.clone();
        tokio::spawn(async move {
            let mut cmd_rx = cmd_rx;
            loop {
                let c = tokio::select! {
                    _ = cancelled(&mut stop) => break,
                    c = cmd_rx.recv() => match c {
                        Some(c) => c,
                        None => break,
                    },
                };
                if int_tx.send(Internal::Command(c)).await.is_err() {
                    break;
                }
            }
        });
    }

    tokio::spawn(accept_loop(
        endpoint.clone(),
        cfg.clone(),
        int_tx.clone(),
        stop_rx.clone(),
    ));

    // De endpoint moet actief dicht bij afsluiten. Lopende sessies houden een
    // `int_tx`-clone vast; zonder deze taak blijven die hangen, sluit de actor nooit
    // en blijft de UDP-poort bezet tot het proces eindigt.
    {
        let endpoint = endpoint.clone();
        let mut stop = stop_rx.clone();
        tokio::spawn(async move {
            cancelled(&mut stop).await;
            endpoint.close(0u32.into(), b"afsluiten");
        });
    }

    {
        let int_tx = int_tx.clone();
        let mut stop = stop_rx.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(STATUS_TICK);
            loop {
                tokio::select! {
                    _ = cancelled(&mut stop) => break,
                    _ = ticker.tick() => {
                        if int_tx.send(Internal::Tick).await.is_err() {
                            break;
                        }
                    }
                }
            }
        });
    }

    let public_key: [u8; 32] = cfg
        .sleutel
        .public_key()
        .as_ref()
        .try_into()
        .context("Ed25519-sleutel is 32 bytes")?;
    let mut actor = Actor {
        cfg: cfg.clone(),
        active: HashMap::new(),
        targets: Vec::new(),
        events: event_tx,
        pending: Vec::new(),
        gemelde_frames: HashSet::new(),
        logrem: Logrem::default(),
        media: media.clone(),
        // Zwak: de actor stopt pas als elke sterke zender weg is, en hij mag zichzelf daar
        // niet tussen zetten.
        spawner: Some(Spawner {
            endpoint: endpoint.clone(),
            int: int_tx.downgrade(),
            stop: stop_rx.clone(),
        }),
        eigen: Vec::new(),
        tikken: 0,
    };
    for t in &cfg.targets {
        actor.voeg_toe(t.clone());
    }
    tokio::spawn(actor.run(int_rx));

    Ok(MeshHandle {
        commands: cmd_tx,
        events: event_rx,
        local_addr,
        media,
        post: post.clone(),
        public_key,
        endpoint,
        socket,
        _shutdown: ShutdownGuard(stop_tx, post),
    })
}

// ---------------------------------------------------------------------------
// Actor
// ---------------------------------------------------------------------------

struct TargetState {
    /// "Er is verbinding". De dialer wacht daarop, zodat we niet blijven bellen naar een peer
    /// die ons al gebeld heeft.
    connected: watch::Sender<bool>,
    /// Wat de dialer probeert; bijgewerkt bij elke wijziging van pin of kandidaten.
    doel: watch::Sender<Doel>,
    /// Alleen het opgezochte configadres (het tailnet). Een sleutel leren kan alleen vanaf
    /// hier; kandidaten tellen daarvoor nooit mee.
    resolved: Option<SocketAddr>,
    known_id: Option<PeerId>,
    public_key: Option<[u8; 32]>,
    bound: Option<u64>,
    last_status: Option<PeerStatus>,
}

impl TargetState {
    /// Pin en kandidaten opnieuw naar de dialer, als ze veranderden.
    fn werk_doel_bij(&self, kandidaten: Option<Vec<SocketAddr>>) {
        self.doel.send_if_modified(|d| {
            let oud = d.clone();
            d.pin = self.public_key;
            d.known_id = self.known_id;
            if let Some(k) = kandidaten {
                d.kandidaten = k;
            }
            *d != oud
        });
    }
}

/// Wat de actor nodig heeft om er een target bij te zetten terwijl de mesh draait (B1).
struct Spawner {
    endpoint: quinn::Endpoint,
    int: mpsc::WeakSender<Internal>,
    stop: watch::Receiver<bool>,
}

/// Een inkomende verbinding die haar handtekening al bewees en nu wacht op het oordeel van
/// de actor, vóór de `HelloAck` (B-05a). De opnemende taak houdt de verbinding vast; hier
/// staat alleen wat nodig is om te beslissen, plus waar het antwoord heen moet.
struct Wachtend {
    sinds: Instant,
    peer_id: PeerId,
    sleutel: [u8; 32],
    remote: SocketAddr,
    /// `Some(target)` = welkom, `None` = stille close.
    antwoord: oneshot::Sender<Option<usize>>,
}

/// Bij welk target een inkomende beller hoort, of dat we nog moeten wachten.
#[derive(Debug, PartialEq)]
enum Koppeling {
    Target(usize),
    /// Hij kan bij een target horen waarvan we het adres nog niet opgezocht hebben.
    Wacht,
    Geen,
}

/// Waarom een bewezen sleutel toch niet binnenkomt.
#[derive(Debug, PartialEq)]
enum Weigering {
    /// Deze identiteit of sleutel hoort al bij een ander target (B-24).
    Dubbel(usize),
    /// Andere `PeerId` dan vastgelegd, of dezelfde met een andere sleutel.
    Identiteit { expected: PeerId, got: PeerId },
    /// Nog niets vastgepind, en het adres ligt buiten het tailnet.
    NietTeLeren,
}

impl Weigering {
    fn reden(&self) -> &'static str {
        match self {
            Self::Dubbel(_) => "dubbele identiteit in config",
            Self::Identiteit { .. } => "identiteit veranderd",
            Self::NietTeLeren => "sleutel onbekend buiten het tailnet",
        }
    }

    fn status(&self) -> PeerStatus {
        match *self {
            Self::Dubbel(_) => PeerStatus::Offline {
                reason: "zelfde peer als een ander adres".to_string(),
            },
            Self::Identiteit { expected, got } => PeerStatus::IdentityChanged { expected, got },
            Self::NietTeLeren => PeerStatus::UnknownKey,
        }
    }
}

#[derive(Debug, PartialEq)]
enum Oordeel {
    /// De vastgepinde sleutel.
    Bekend,
    /// Nog niets vastgepind en het adres mag: nu vastleggen.
    Leer,
}

/// Eén logregel per bron-IP per [`LOGREM`] (B-25). Een vreemde die blijft aankloppen is
/// één regel per minuut, geen regel per poging.
#[derive(Default)]
struct Logrem(HashMap<IpAddr, Instant>);

impl Logrem {
    fn mag(&mut self, ip: IpAddr) -> bool {
        let nu = Instant::now();
        if self.0.len() > 1024 {
            self.0.retain(|_, t| nu.duration_since(*t) < LOGREM);
            // Nog steeds vol: dan is dit een stortvloed en vergeten we liever dan dat we
            // bij elke poging de hele kaart nalopen.
            if self.0.len() > 1024 {
                self.0.clear();
            }
        }
        match self.0.get(&ip) {
            Some(t) if nu.duration_since(*t) < LOGREM => false,
            _ => {
                self.0.insert(ip, nu);
                true
            }
        }
    }
}

struct Active {
    conn_id: u64,
    target: Option<usize>,
    display_name: String,
    media_addr: SocketAddr,
    app_version: String,
    conn: quinn::Connection,
    out: mpsc::Sender<ControlMsg>,
}

struct Actor {
    cfg: MeshConfig,
    active: HashMap<PeerId, Active>,
    targets: Vec<TargetState>,
    events: mpsc::Sender<MeshEvent>,
    /// Inkomende bellers die we nog niet aan een target konden koppelen.
    ///
    /// Een sleutel leren gebeurt alleen vanaf het opgezochte adres van een target, en dat
    /// weten we pas zodra onze eigen dialer het heeft opgezocht. Belt de ander eerder dan
    /// dat, dan zouden we een volstrekt legitieme verbinding weigeren. Daarom parkeren we
    /// hem kort; zijn `HelloAck` wacht zolang.
    ///
    /// Begrensd op [`Actor::max_pending`] (B-26): achter elke wachtende staat een opnemende
    /// taak met een levende `quinn::Connection`, en `retry_pending` loopt de lijst bij elke
    /// tik helemaal na.
    pending: Vec<Wachtend>,
    /// Verbindingen waarover we al een regel schreven over een frame dat nergens bij
    /// hoorde (B-25). Eén regel per verbinding, niet één per frame.
    gemelde_frames: HashSet<u64>,
    logrem: Logrem,
    media: MediaSleutels,
    /// `None` in tests zonder netwerk.
    spawner: Option<Spawner>,
    /// De laatst gemelde eigen control-adressen (`MeshEvent::EigenAdressen`).
    eigen: Vec<SocketAddr>,
    tikken: u32,
}

/// Hoe lang een niet te koppelen inkomende beller blijft wachten voordat we concluderen
/// dat hij echt niet in de config staat.
///
/// Korter dan [`CONNECT_TIMEOUT`]: zolang hij geparkeerd staat wacht de beller op zijn
/// `HelloAck`, en na die 4 s geeft hij het op. Langer parkeren kan dus nooit meer slagen.
const PENDING_GRACE: Duration = Duration::from_secs(3);

impl Actor {
    async fn run(mut self, mut rx: mpsc::Receiver<Internal>) {
        self.meld_eigen_adressen().await;
        while let Some(msg) = rx.recv().await {
            match msg {
                Internal::Command(c) => self.on_command(c).await,
                Internal::Resolved { target, addr } => {
                    self.targets[target].resolved = Some(addr);
                    // Nu er een adres bekend is kan een geparkeerde verbinding alsnog
                    // bij een target horen.
                    self.retry_pending().await;
                }
                Internal::Connecting { target } => {
                    self.set_status(target, PeerStatus::Connecting).await;
                }
                Internal::Established(e) => {
                    let target = e.target;
                    self.install(*e, target).await
                }
                Internal::Inkomend(w) => self.on_inkomend(w).await,
                Internal::Geweigerd { ip, reden } => self.meld_weigering(ip, reden),
                Internal::Rejected { target, status } => self.set_status(target, status).await,
                Internal::Frame { conn_id, msg } => self.on_frame(conn_id, msg).await,
                Internal::IncomingFileStream { conn_id, stream } => {
                    self.on_incoming_file_stream(conn_id, stream).await
                }
                Internal::Closed { conn_id, reason } => self.on_closed(conn_id, reason).await,
                Internal::Tick => self.on_tick().await,
            }
        }
        // De endpoint wordt door de shutdown-taak gesloten, niet hier: die moet ook
        // afgaan als deze lus nog aan een lopende sessie vasthangt.
        tracing::debug!("mesh-actor gestopt");
    }

    /// Zet een target erbij en start zijn dialer. Het nummer is de plek in de lijst.
    fn voeg_toe(&mut self, t: PeerTarget) {
        let (connected, connected_rx) = watch::channel(false);
        let (doel, doel_rx) = watch::channel(Doel::van(&t));
        let target = self.targets.len();
        self.targets.push(TargetState {
            connected,
            doel,
            resolved: None,
            known_id: t.known_id,
            public_key: t.public_key,
            bound: None,
            last_status: None,
        });
        let Some(s) = &self.spawner else { return };
        let Some(int) = s.int.upgrade() else { return };
        tokio::spawn(dial_loop(
            s.endpoint.clone(),
            self.cfg.clone(),
            target,
            doel_rx,
            int,
            connected_rx,
            s.stop.clone(),
        ));
    }

    /// De eigen control-adressen opnieuw opzoeken, en melden als ze veranderden.
    async fn meld_eigen_adressen(&mut self) {
        let poort = self.cfg.control_port;
        let nu: Vec<SocketAddr> = lokale_ips()
            .into_iter()
            .map(|ip| SocketAddr::new(ip, poort))
            .collect();
        if nu != self.eigen {
            self.eigen = nu.clone();
            let _ = self.events.send(MeshEvent::EigenAdressen(nu)).await;
        }
    }

    async fn on_command(&mut self, cmd: MeshCommand) {
        match cmd {
            MeshCommand::VoegToe(t) => {
                tracing::info!(label = ?t.label, target = self.targets.len(), "peer toegevoegd");
                self.voeg_toe(t);
            }
            MeshCommand::WerkBij {
                target,
                known_id,
                public_key,
                kandidaten,
            } => {
                let Some(t) = self.targets.get_mut(target) else {
                    return;
                };
                // De app controleerde dit al tegen de config; hier nog eens tegen wat de actor
                // intussen geleerd kan hebben. Een vastgepinde sleutel verandert nooit stil.
                if t.public_key.is_some_and(|k| k != public_key)
                    || t.known_id.is_some_and(|k| k != known_id)
                {
                    tracing::warn!(target, "code botst met de vastgepinde sleutel; genegeerd");
                    return;
                }
                t.known_id = Some(known_id);
                t.public_key = Some(public_key);
                t.werk_doel_bij(Some(kandidaten));
                // Een geparkeerde beller kan nu op zijn sleutel gekoppeld worden.
                self.retry_pending().await;
            }
            MeshCommand::Send { to, msg } => {
                if let Some(a) = self.active.get(&to) {
                    let _ = a.out.try_send(msg);
                } else {
                    tracing::debug!(peer = ?to, "bericht verworpen: peer niet verbonden");
                }
            }
            MeshCommand::Broadcast(msg) => {
                for a in self.active.values() {
                    if let Err(e) = a.out.try_send(msg.clone()) {
                        tracing::warn!(conn = a.conn_id, error = %e, "broadcast niet verstuurd");
                    }
                }
            }
            MeshCommand::OpenUploadStream { to, respond } => {
                match self.active.get(&to) {
                    Some(a) => {
                        let conn = a.conn.clone();
                        // Los spawnen: `open_uni` wacht eventueel op flow-control-krediet
                        // van de andere kant, en de actor mag daar niet op blokkeren.
                        tokio::spawn(async move {
                            let stream = conn.open_uni().await.ok();
                            let _ = respond.send(stream);
                        });
                    }
                    None => {
                        let _ = respond.send(None);
                    }
                }
            }
        }
    }

    async fn on_incoming_file_stream(&mut self, conn_id: u64, stream: quinn::RecvStream) {
        let Some((&from, _)) = self.active.iter().find(|(_, a)| a.conn_id == conn_id) else {
            tracing::warn!(
                conn = conn_id,
                "bestandsstream van onbekende verbinding genegeerd"
            );
            return;
        };
        let _ = self
            .events
            .send(MeshEvent::IncomingFileStream { from, stream })
            .await;
    }

    /// Hoeveel verbindingen er tegelijk geparkeerd mogen staan (B-26).
    ///
    /// Vier per geconfigureerde peer, ondergrens acht. Ruim gekozen om dezelfde reden als
    /// [`max_inkomend`]: bij het opstarten kan iedereen tegelijk bellen, tijdens een
    /// botsing staan er heel even twee van dezelfde peer, en een geparkeerde verbinding
    /// die we ten onrechte wegkieperen is een peer die pas na zijn volgende backoff weer
    /// binnenkomt. Elke plek gaat sowieso na [`PENDING_GRACE`] weer open.
    fn max_pending(&self) -> usize {
        self.targets.len().saturating_mul(4).max(8)
    }

    /// Beslist over een inkomende beller die zijn handtekening al bewees, vóór de
    /// `HelloAck` (B-05a): welk target, en mag hij erin. Kan dat nog niet, dan parkeren.
    async fn on_inkomend(&mut self, w: Wachtend) {
        match self.koppel_inkomend(w.peer_id, &w.sleutel, w.remote) {
            Koppeling::Target(t) => match self.beoordeel(t, w.peer_id, &w.sleutel, w.remote.ip()) {
                Ok(_) => {
                    let _ = w.antwoord.send(Some(t));
                }
                Err(weigering) => {
                    self.meld_weigering(w.remote.ip(), weigering.reden());
                    // Alleen zonder verbinding: anders zou een vreemde met een geclaimde
                    // `PeerId` een peer die gewoon online is als "kapot" in beeld kunnen
                    // zetten. Zonder verbinding is het alarm wél waar.
                    if self.targets[t].bound.is_none() {
                        self.set_status(t, weigering.status()).await;
                    }
                    let _ = w.antwoord.send(None);
                }
            },
            Koppeling::Wacht if w.sinds.elapsed() < PENDING_GRACE => {
                // B-26: de **oudste** wijkt, niet de nieuwste. Andersom zou een stroom
                // pogingen van een vreemde precies die ene legitieme peer buiten de deur
                // houden die als eerste belde en nog op zijn adres wacht.
                while self.pending.len() >= self.max_pending() {
                    let oud = self.pending.remove(0);
                    tracing::debug!(
                        peer = ?oud.peer_id, remote = %oud.remote,
                        "wachtrij vol: oudste ongekoppelde beller losgelaten"
                    );
                    let _ = oud.antwoord.send(None);
                }
                tracing::debug!(
                    peer = ?w.peer_id, remote = %w.remote,
                    "inkomende beller nog niet te koppelen, geparkeerd"
                );
                self.pending.push(w);
            }
            Koppeling::Wacht | Koppeling::Geen => {
                self.meld_weigering(w.remote.ip(), "onbekende sleutel");
                let _ = w.antwoord.send(None);
            }
        }
    }

    /// Probeert geparkeerde bellers opnieuw en wijst af wat te lang wacht.
    async fn retry_pending(&mut self) {
        for w in std::mem::take(&mut self.pending) {
            self.on_inkomend(w).await;
        }
    }

    fn meld_weigering(&mut self, ip: IpAddr, reden: &str) {
        if self.logrem.mag(ip) {
            tracing::info!(%ip, reden, "inkomende verbinding geweigerd");
        }
    }

    /// Bij welk target een inkomende beller hoort.
    ///
    /// Eerst op de vastgepinde sleutel: wie hem heeft is die peer, vanaf welk adres ook
    /// (adressen zijn hints, nooit toegang). Daarna dezelfde `PeerId` met een *andere*
    /// vastgepinde sleutel: dat target, zodat [`Actor::beoordeel`] het alarm geeft (B-05b;
    /// vroeger kon dat op dit pad nooit vuren). Pas dan leren, en alleen bij targets
    /// zonder sleutel, vanaf hun eigen opgezochte IP.
    fn koppel_inkomend(
        &self,
        peer_id: PeerId,
        sleutel: &[u8; 32],
        remote: SocketAddr,
    ) -> Koppeling {
        let zoek = |pred: &dyn Fn(&TargetState) -> bool| self.targets.iter().position(pred);
        if let Some(i) = zoek(&|t| t.public_key == Some(*sleutel)) {
            return Koppeling::Target(i);
        }
        if let Some(i) = zoek(&|t| t.public_key.is_some() && t.known_id == Some(peer_id)) {
            return Koppeling::Target(i);
        }

        let ongepind = |t: &TargetState| t.public_key.is_none();
        let zelfde_ip = |t: &TargetState| t.resolved.map(|a| a.ip()) == Some(remote.ip());

        // Een installatie van vóór protocol 6: de `PeerId` kenden we al, de sleutel nog niet.
        if let Some(i) = zoek(&|t| ongepind(t) && t.known_id == Some(peer_id) && zelfde_ip(t)) {
            return Koppeling::Target(i);
        }

        let nieuw = |t: &TargetState| ongepind(t) && t.known_id.is_none();

        // Exact adres inclusief poort. Peers dialen vanaf de socket waarop ze zelf
        // luisteren, dus de bronpoort is hun `control_port`.
        if let Some(i) = zoek(&|t| nieuw(t) && t.resolved == Some(remote)) {
            return Koppeling::Target(i);
        }

        // Alleen het IP, maar uitsluitend als dat ondubbelzinnig één peer aanwijst. Vangt
        // het geval af waarin een NAT onderweg de bronpoort herschrijft.
        let mut hits = self
            .targets
            .iter()
            .enumerate()
            .filter(|(_, t)| nieuw(t) && zelfde_ip(t));
        if let (Some((i, _)), None) = (hits.next(), hits.next()) {
            return Koppeling::Target(i);
        }

        if self
            .targets
            .iter()
            .any(|t| ongepind(t) && t.resolved.is_none())
        {
            Koppeling::Wacht
        } else {
            Koppeling::Geen
        }
    }

    /// Mag deze bewezen sleutel met deze `PeerId` bij dit target, en moeten we hem leren?
    /// Eén regel voor bellen en opnemen, zodat de twee paden niet uit elkaar lopen.
    fn beoordeel(
        &self,
        target: usize,
        peer_id: PeerId,
        sleutel: &[u8; 32],
        ip: IpAddr,
    ) -> std::result::Result<Oordeel, Weigering> {
        // B-24: dezelfde identiteit mag nooit aan twee targets hangen. Lossen twee
        // adressen uit de config naar dezelfde peer op, dan verhuisde `active` naar het
        // laatste target terwijl `bound` en `connected` van het eerste bleven staan: die
        // peer stond daarna voorgoed "online" op een verbinding die al dicht was, en werd
        // nooit meer gebeld. Het eerste target dat de identiteit leerde houdt hem.
        if let Some(ander) = self.identiteit_op_ander_target(target, peer_id, sleutel) {
            return Err(Weigering::Dubbel(ander));
        }
        let t = &self.targets[target];
        if let Some(verwacht) = t.known_id.filter(|&k| k != peer_id) {
            return Err(Weigering::Identiteit {
                expected: verwacht,
                got: peer_id,
            });
        }
        match t.public_key {
            // Een vastgepinde sleutel zonder `PeerId` (een met de hand bewerkte config, straks
            // een code zonder id): de id nu vastleggen, anders kon wie de sleutel heeft bij
            // elke verbinding een andere `PeerId` opgeven.
            Some(k) if k == *sleutel && t.known_id.is_none() => Ok(Oordeel::Leer),
            Some(k) if k == *sleutel => Ok(Oordeel::Bekend),
            Some(_) => Err(Weigering::Identiteit {
                expected: peer_id,
                got: peer_id,
            }),
            None if mag_leren(ip) => Ok(Oordeel::Leer),
            None => Err(Weigering::NietTeLeren),
        }
    }

    async fn install(&mut self, e: Established, target: usize) {
        match self.beoordeel(target, e.peer_id, &e.sleutel, e.remote.ip()) {
            Err(weigering) => {
                if self.logrem.mag(e.remote.ip()) {
                    tracing::warn!(
                        peer = ?e.peer_id, remote = %e.remote, target, ?weigering,
                        "verbinding geweigerd"
                    );
                }
                e.conn.close(1u32.into(), weigering.reden().as_bytes());
                if self.targets[target].bound.is_none() {
                    self.set_status(target, weigering.status()).await;
                }
                return;
            }
            Ok(Oordeel::Bekend) => {}
            Ok(Oordeel::Leer) => {
                tracing::info!(peer = ?e.peer_id, remote = %e.remote, target, "sleutel vastgepind");
                self.targets[target].known_id = Some(e.peer_id);
                self.targets[target].public_key = Some(e.sleutel);
                self.targets[target].werk_doel_bij(None);
                let _ = self
                    .events
                    .send(MeshEvent::LearnedIdentity {
                        target,
                        peer_id: e.peer_id,
                        public_key: e.sleutel,
                    })
                    .await;
            }
        }

        // Botsing: welke verbinding wint is een **absolute** eigenschap, niet iets wat
        // van aankomstvolgorde afhangt. De winnaar is de verbinding die is opgezet door
        // de peer met het laagste PeerId.
        //
        // Vergelijken met "wat er al lag" is fout: bij ongelijke aankomstvolgorde houdt
        // de ene kant dan A→B over en de andere B→A, sluiten ze elkaars keuze, en blijft
        // er niets werkends over.
        let preferred = self.cfg.me.min(e.peer_id);

        if let Some(existing) = self.active.get(&e.peer_id) {
            if e.initiator != preferred {
                tracing::debug!(peer = ?e.peer_id, "dubbele verbinding: nieuwe is niet de voorkeur, sluiten");
                e.conn.close(0u32.into(), b"dubbele verbinding");
                return;
            }
            tracing::debug!(peer = ?e.peer_id, "dubbele verbinding: nieuwe is de voorkeur, oude sluiten");
            existing.conn.close(0u32.into(), b"dubbele verbinding");
        }

        tracing::info!(
            // B-33: `?` en niet `%` voor velden van de draad. `%` escapet niets, dus een
            // naam met een nieuwe regel erin kon overtuigende neplogregels fabriceren —
            // bijvoorbeeld een verzonnen "hash klopt niet" — precies wanneer je het log
            // nodig hebt om een incident te reconstrueren.
            peer = ?e.peer_id, name = ?e.display_name, remote = %e.remote,
            "verbonden"
        );

        self.targets[target].bound = Some(e.conn_id);
        let _ = self.targets[target].connected.send(true);

        // B-41 (de door de peer opgegeven mediapoort is ongevalideerd) is hier bewust
        // blijven liggen. Het IP komt uit de verbinding, dus dit wijst nooit naar een
        // derde; het kwaad is beperkt tot een peer die ons media naar een eigen poort
        // laat sturen, of naar poort 0 en dan mislukt elk pakket apart. Poort 0 weigeren
        // betekent de verbinding sluiten, en alle integratietests van `crates/app`
        // (`chat_sync`, `file_deling`, `stream_deling`) draaien met `media_port: 0` omdat
        // daar geen media loopt — die peers vonden elkaar daarna niet meer. Invariant 7
        // gaat vóór een LAAG-bevinding. Wie dit alsnog wil: geef die tests een echte
        // poort, of maak `media_addr` een `Option` (dat laatste raakt `crates/app`).
        let media_addr = SocketAddr::new(e.remote.ip(), e.media_port);

        // De mediasleutels van deze verbinding, nu zij de botsing gewonnen heeft. Een
        // verliezer krijgt er geen; de oude verbinding van een winnaar houdt haar
        // ontvangstsleutel nog even (`on_closed`).
        let (Some(zend), Some(ontvang)) = (
            mediasleutel_van(&e.conn, self.cfg.me),
            mediasleutel_van(&e.conn, e.peer_id),
        ) else {
            // Kan in de praktijk niet (32 bytes uit TLS 1.3), maar "online zonder media"
            // zou een stil gat zijn.
            tracing::warn!(peer = ?e.peer_id, "geen mediasleutels voor deze verbinding");
            e.conn.close(1u32.into(), b"geen mediasleutels");
            return;
        };
        if e.hun_index == 0 {
            // Een peer op 6 zet hier altijd iets neer; 0 is de standaardwaarde van een veld
            // dat ontbrak. Dan gaat er bij hem geen enkel pakket van ons open, zonder dat
            // iemand het ziet. Verbinden mag, maar dan wel met een spoor in de log.
            tracing::warn!(peer = ?e.peer_id, "peer gaf geen media-index; zijn media blijft stil");
        }
        self.media.zet(
            e.conn_id,
            e.peer_id,
            zend,
            e.hun_index,
            ontvang,
            e.mijn_index,
            media_addr,
        );

        let status = PeerStatus::Online {
            peer_id: e.peer_id,
            display_name: e.display_name.clone(),
            media_addr,
            rtt_ms: e.conn.rtt().as_millis() as u32,
            app_version: e.app_version.clone(),
        };

        self.active.insert(
            e.peer_id,
            Active {
                conn_id: e.conn_id,
                target: Some(target),
                display_name: e.display_name,
                media_addr,
                app_version: e.app_version,
                conn: e.conn,
                out: e.out,
            },
        );

        self.set_status(target, status).await;
    }

    /// Of deze identiteit of sleutel al bij een *ander* target hoort (B-24). Levert dat andere
    /// target op, zodat het log kan vertellen welke twee regels in de config botsen.
    fn identiteit_op_ander_target(
        &self,
        target: usize,
        peer_id: PeerId,
        sleutel: &[u8; 32],
    ) -> Option<usize> {
        self.targets
            .iter()
            .position(|t| t.known_id == Some(peer_id) || t.public_key == Some(*sleutel))
            .filter(|&i| i != target)
    }

    async fn on_frame(&mut self, conn_id: u64, msg: ControlMsg) {
        let Some((&from, _)) = self.active.iter().find(|(_, a)| a.conn_id == conn_id) else {
            // B-25: alleen de tag, nooit `?msg`. Hier stond de volledige gedecodeerde
            // `ControlMsg`, wat bij een `OpBroadcast` een logregel van megabytes werd — en
            // de appender schrijft bewust synchroon, dus stonden peerstatus, ping/pong en
            // `retry_pending` zolang stil. Eén regel per verbinding, want een nog niet
            // gekoppelde verbinding mag dan wel legitiem zijn, tienduizend regels zijn dat
            // nooit.
            if self.gemelde_frames.insert(conn_id) {
                tracing::debug!(
                    conn = conn_id,
                    tag = msg.tag(),
                    "frame van nog niet gekoppelde verbinding genegeerd"
                );
            }
            return;
        };

        // Ping wordt hier afgehandeld: liveness hoort niet in de applicatielaag terecht
        // te komen. De rest gaat door naar de app.
        if let ControlMsg::Ping(p) = &msg {
            if let Some(a) = self.active.get(&from) {
                let _ = a
                    .out
                    .try_send(ControlMsg::Pong(fitcom_proto::control::Pong {
                        nonce: p.nonce,
                    }));
            }
            return;
        }

        let _ = self.events.send(MeshEvent::Message { from, msg }).await;
    }

    async fn on_closed(&mut self, conn_id: u64, reason: String) {
        self.media.vergeet(conn_id);

        // B-24: losmaken gaat op `bound`, niet via een omgekeerde zoekactie in `active`.
        // Die zoekactie vond niets zodra de verbinding daar al door een nieuwere vervangen
        // was, en dan keerde deze functie vroeg terug — met `bound` en `connected` blijvend
        // op de dode verbinding, dus zonder dat de dialer ooit weer aan de slag ging.
        // `bound == Some(conn_id)` is bovendien nog steeds de test die voorkomt dat een
        // verlaten botsings-verbinding de goede wegkickt.
        if let Some(t) = self.targets.iter().position(|t| t.bound == Some(conn_id)) {
            self.targets[t].bound = None;
            let _ = self.targets[t].connected.send(false);
            self.set_status(
                t,
                PeerStatus::Offline {
                    reason: reason.clone(),
                },
            )
            .await;
        }

        // En pas daarna uit `active`, en alleen als deze verbinding daar nog echt staat.
        if let Some((&peer, _)) = self.active.iter().find(|(_, a)| a.conn_id == conn_id) {
            self.active.remove(&peer);
            tracing::info!(peer = ?peer, %reason, "verbinding verbroken");
        }

        self.gemelde_frames.remove(&conn_id);
    }

    async fn on_tick(&mut self) {
        self.retry_pending().await;
        self.media.ruim_op();
        self.tikken = self.tikken.wrapping_add(1);
        if self.tikken.is_multiple_of(EIGEN_ELKE) {
            self.meld_eigen_adressen().await;
        }

        let updates: Vec<(usize, PeerStatus)> = self
            .active
            .iter()
            .filter_map(|(peer_id, a)| {
                a.target.map(|t| {
                    (
                        t,
                        PeerStatus::Online {
                            peer_id: *peer_id,
                            display_name: a.display_name.clone(),
                            media_addr: a.media_addr,
                            rtt_ms: a.conn.rtt().as_millis() as u32,
                            app_version: a.app_version.clone(),
                        },
                    )
                })
            })
            .collect();
        for (t, s) in updates {
            self.set_status(t, s).await;
        }
    }

    async fn set_status(&mut self, target: usize, status: PeerStatus) {
        // Voorkomt dat de UI elke seconde een identiek statusbericht krijgt.
        if self.targets[target].last_status.as_ref() == Some(&status) {
            return;
        }
        self.targets[target].last_status = Some(status.clone());
        let _ = self.events.send(MeshEvent::Status { target, status }).await;
    }
}

// ---------------------------------------------------------------------------
// Dialen en accepteren
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
async fn dial_loop(
    endpoint: quinn::Endpoint,
    cfg: MeshConfig,
    target: usize,
    doel: watch::Receiver<Doel>,
    int: mpsc::Sender<Internal>,
    connected: watch::Receiver<bool>,
    mut stop: watch::Receiver<bool>,
) {
    tokio::select! {
        _ = cancelled(&mut stop) => {}
        _ = dial_forever(endpoint, cfg, target, doel, int, connected) => {}
    }
}

async fn dial_forever(
    endpoint: quinn::Endpoint,
    cfg: MeshConfig,
    target: usize,
    mut doel: watch::Receiver<Doel>,
    int: mpsc::Sender<Internal>,
    mut connected: watch::Receiver<bool>,
) {
    let mut backoff = BACKOFF_START;

    loop {
        // Wacht zolang deze peer al verbonden is — hij kan ons ook gebeld hebben.
        while *connected.borrow() {
            if connected.changed().await.is_err() {
                return;
            }
        }

        let d = doel.borrow_and_update().clone();
        let _ = int.send(Internal::Connecting { target }).await;

        match try_dial(&endpoint, &cfg, target, &d, &int).await {
            // B-40: alleen een sessie die er echt geweest is verdient een verse backoff.
            // `VersionMismatch` keerde ook als `Ok` terug en zette hem daarmee terug op
            // één seconde, dus tegen een peer die altijd een afwijkende versie meldt deden
            // we elke seconde een volledige QUIC+TLS-handshake — voor niets, want die
            // uitkomst verandert pas als iemand update.
            Ok(Dialresultaat::Sessie) => {
                backoff = BACKOFF_START;
            }
            Ok(Dialresultaat::Afgewezen) => {}
            Err(e) => {
                tracing::debug!(address = %d.address, error = %format!("{e:#}"), "verbinden mislukt");
                let _ = int
                    .send(Internal::Rejected {
                        target,
                        status: PeerStatus::Offline {
                            reason: short_reason(&e),
                        },
                    })
                    .await;
            }
        }

        let max = if d.buiten_tailnet() {
            BACKOFF_MAX_PUBLIEK
        } else {
            BACKOFF_MAX
        };
        tokio::select! {
            _ = tokio::time::sleep(backoff) => {}
            // Een geplakte code of een nieuw geleerd adres: meteen proberen, niet pas na de
            // backoff van een peer die tot nu toe onbereikbaar was.
            Ok(()) = doel.changed() => backoff = BACKOFF_START,
        }
        backoff = (backoff * 2).min(max);
    }
}

/// Wat een dialpoging opleverde (B-40). `Afgewezen` is geen fout — de peer is bereikbaar
/// maar we kunnen er niets mee — en mag daarom ook de backoff niet resetten.
enum Dialresultaat {
    /// De handshake slaagde en er heeft een sessie gedraaid.
    Sessie,
    /// Nette, verwachte afwijzing: de peer is bereikbaar maar draait een andere
    /// protocolversie. Dat verandert pas als iemand update, dus doorproberen mag rustig.
    Afgewezen,
}

/// Tussen het starten van twee belpogingen naar verschillende kandidaten (B3). Kort genoeg
/// dat een werkend adres achteraan in de lijst snel aan de beurt is, lang genoeg dat een
/// adres dat meteen antwoordt (het LAN, het tailnet) de rest niet eens laat starten.
const STAFFEL: Duration = Duration::from_millis(250);

/// Wat de dialer probeert (B3). De actor houdt hem bij; de dialer leest hem bij elke poging
/// opnieuw, zodat een geplakte code of een geleerd adres meteen meedoet.
#[derive(Debug, Clone, Default, PartialEq)]
struct Doel {
    /// Uit de config: het tailnet-adres of de MagicDNS-naam. Leeg voor een peer die alleen
    /// uit een uitnodigingscode komt.
    address: String,
    control_port: u16,
    /// Uit de code en geleerd (B4). Hints, nooit toegang.
    kandidaten: Vec<SocketAddr>,
    /// De vastgepinde sleutel en `PeerId`. Op een kandidaat waar een ander opneemt is het
    /// adres verkeerd, en daar blijft het bij.
    pin: Option<[u8; 32]>,
    known_id: Option<PeerId>,
}

impl Doel {
    fn van(t: &PeerTarget) -> Self {
        Self {
            address: t.address.clone(),
            control_port: t.control_port,
            kandidaten: t.kandidaten.clone(),
            pin: t.public_key,
            known_id: t.known_id,
        }
    }

    /// Of er een kandidaat buiten het tailnet bij is: dan bellen we vaker
    /// ([`BACKOFF_MAX_PUBLIEK`]).
    fn buiten_tailnet(&self) -> bool {
        self.kandidaten.iter().any(|a| !mag_leren(a.ip()))
    }
}

/// Verbindt, doet de handshake en keert pas terug als de verbinding weer weg is.
///
/// Belt het configadres en alle kandidaten gestaffeld vanaf hetzelfde endpoint (B3): de eerste
/// QUIC-verbinding die opkomt krijgt de `Hello`. Neemt daar iemand anders op dan deze peer,
/// dan gaat het verder met de rest.
async fn try_dial(
    endpoint: &quinn::Endpoint,
    cfg: &MeshConfig,
    target: usize,
    doel: &Doel,
    int: &mpsc::Sender<Internal>,
) -> Result<Dialresultaat> {
    let mut adressen = Vec::new();
    let mut configadres = None;
    if !doel.address.is_empty() {
        let host = format!("{}:{}", doel.address, doel.control_port);
        let opgezocht = tokio::net::lookup_host(&host)
            .await
            .with_context(|| format!("{host} opzoeken"))
            .and_then(|mut a| {
                a.next()
                    .with_context(|| format!("{host} leverde geen adres op"))
            });
        match opgezocht {
            Ok(addr) => {
                let _ = int.send(Internal::Resolved { target, addr }).await;
                adressen.push(addr);
                configadres = Some(addr);
            }
            // Zonder Tailscale is de MagicDNS-naam weg; de adressen uit de code niet.
            Err(e) if !doel.kandidaten.is_empty() => {
                tracing::debug!(error = %format!("{e:#}"), "configadres niet op te zoeken; de kandidaten wel");
            }
            Err(e) => return Err(e),
        }
    }
    for k in &doel.kandidaten {
        if !adressen.contains(k) {
            adressen.push(*k);
        }
    }
    if adressen.is_empty() {
        bail!("geen adres om te proberen");
    }

    // Een adres dat de QUIC-handshake haalt maar daarna niets wordt (de eigen pc op een oud
    // LAN-adres, een andere installatie die ons niet kent en stil sluit, een peer op een
    // andere versie) gaat eruit, en dan de rest. Anders won zo'n adres elke ronde de staffel
    // en kwam een werkend adres verderop nooit aan de beurt.
    let mut fout: Option<anyhow::Error> = None;
    let mut gemeld = false;
    loop {
        let (conn, addr) = verbind_eerste(endpoint, &adressen).await?;
        let bij_config = configadres == Some(addr);
        match opzetten(conn, cfg, target, doel, bij_config, int).await {
            Ok(Opgezet::Klaar(r)) => return Ok(r),
            Ok(Opgezet::Volgende { status_gemeld }) => {
                tracing::debug!(%addr, target, "op deze kandidaat werd het niets");
                gemeld |= status_gemeld;
                fout.get_or_insert_with(|| anyhow::anyhow!("op dit adres neemt een ander op"));
            }
            Err(e) => {
                tracing::debug!(%addr, target, error = %format!("{e:#}"), "handshake op deze kandidaat mislukt");
                fout = Some(e);
            }
        }
        adressen.retain(|a| *a != addr);
        if adressen.is_empty() {
            // Een versieverschil of een ander op het tailnet-adres staat al als status; die
            // niet overschrijven met een kale fout.
            return match fout {
                Some(e) if !gemeld => Err(e),
                _ => Ok(Dialresultaat::Afgewezen),
            };
        }
    }
}

/// Wat er na het opzetten van één verbinding uitkwam.
enum Opgezet {
    /// Klaar met deze poging: er heeft een sessie gedraaid (die nu voorbij is).
    Klaar(Dialresultaat),
    /// Op dit adres werd het niets vóór er een sessie was: een ander nam op, of een peer op
    /// een andere versie. Probeer de andere kandidaten. `status_gemeld`: er ging al een
    /// status naar de actor (versieverschil, `IdentityChanged`).
    Volgende { status_gemeld: bool },
}

/// Belt alle adressen gestaffeld vanaf hetzelfde endpoint en levert de eerste QUIC-verbinding
/// die opkomt. De rest wordt afgebroken vóór er een `Hello` over gaat, zodat er per poging
/// hooguit één verbinding ontstaat en de botsingsregel niet tussen twee eigen verbindingen
/// hoeft te kiezen.
async fn verbind_eerste(
    endpoint: &quinn::Endpoint,
    adressen: &[SocketAddr],
) -> Result<(quinn::Connection, SocketAddr)> {
    let mut lopend = tokio::task::JoinSet::new();
    let mut rest = adressen.iter().copied();
    let mut fout: Option<anyhow::Error> = None;
    let deadline = tokio::time::Instant::now() + CONNECT_TIMEOUT;
    let mut volgende = tokio::time::Instant::now();
    loop {
        if tokio::time::Instant::now() >= volgende {
            if let Some(addr) = rest.next() {
                match endpoint.connect(addr, tls::SERVER_NAME) {
                    Ok(c) => {
                        lopend.spawn(async move {
                            c.await
                                .map(|conn| (conn, addr))
                                .with_context(|| format!("QUIC-verbinding naar {addr}"))
                        });
                    }
                    Err(e) => fout = Some(anyhow::Error::new(e).context(format!("{addr}"))),
                }
                volgende += STAFFEL;
                continue;
            }
        }
        let meer = rest.len() > 0;
        if lopend.is_empty() && !meer {
            break;
        }
        tokio::select! {
            Some(r) = lopend.join_next() => match r {
                Ok(Ok(v)) => return Ok(v),
                Ok(Err(e)) => fout = Some(e),
                Err(_) => {}
            },
            _ = tokio::time::sleep_until(volgende), if meer => {}
            _ = tokio::time::sleep_until(deadline) => bail!("peer reageert niet"),
        }
    }
    Err(fout.unwrap_or_else(|| anyhow::anyhow!("peer reageert niet")))
}

/// De handshake op een opgezette QUIC-verbinding, en daarna de sessie.
async fn opzetten(
    conn: quinn::Connection,
    cfg: &MeshConfig,
    target: usize,
    doel: &Doel,
    bij_config: bool,
    int: &mpsc::Sender<Internal>,
) -> Result<Opgezet> {
    let mijn_index = mediasleutel::nieuwe_index();

    // Alleen het opzetten krijgt een deadline. De sessie erna mag uren duren.
    let opgezet = tokio::time::timeout(CONNECT_TIMEOUT, async {
        let (mut send, mut recv) = conn.open_bi().await.context("control-stream openen")?;

        framing::write_frame(
            &mut send,
            &ControlMsg::Hello(Hello {
                protocol_version: PROTOCOL_VERSION,
                peer_id: cfg.me,
                display_name: cfg.display_name.clone(),
                media_port: cfg.media_port,
                app_version: cfg.app_version.clone(),
                public_key: cfg.sleutel.public_key().as_ref().to_vec(),
                signature: teken(&cfg.sleutel, &conn, Handtekening::BELLER)?,
                media_index: mijn_index,
            }),
        )
        .await?;

        let ack = match framing::read_frame(&mut recv).await? {
            Some(Some(ControlMsg::HelloAck(a))) => a,
            Some(_) => bail!("peer antwoordde niet met HelloAck"),
            None => bail!("peer sloot de verbinding tijdens de handshake"),
        };

        Ok::<_, anyhow::Error>((send, recv, ack))
    })
    .await
    .map_err(|_| anyhow::anyhow!("peer reageert niet"))?;

    let (send, recv, ack) = match opgezet {
        Ok(v) => v,
        // Een peer op protocol 5 schrijft zijn `HelloAck` en sluit in dezelfde tik, dus het
        // antwoord komt nooit aan; alleen zijn sluitreden wel. Zonder dit zag wie al
        // bijgewerkt is de ander als "offline" in plaats van "update nodig".
        Err(e) if gesloten_om_versie(&e) => {
            let _ = int
                .send(Internal::Rejected {
                    target,
                    status: PeerStatus::VersionMismatch {
                        theirs: 0,
                        ours: PROTOCOL_VERSION,
                    },
                })
                .await;
            return Ok(Opgezet::Volgende {
                status_gemeld: true,
            });
        }
        Err(e) => return Err(e),
    };

    if ack.protocol_version != PROTOCOL_VERSION {
        conn.close(1u32.into(), b"protocolversie");
        let _ = int
            .send(Internal::Rejected {
                target,
                status: PeerStatus::VersionMismatch {
                    theirs: ack.protocol_version,
                    ours: PROTOCOL_VERSION,
                },
            })
            .await;
        // Geen fout: dit is een nette, verwachte uitkomst waar de gebruiker wat mee moet.
        return Ok(Opgezet::Volgende {
            status_gemeld: true,
        });
    }

    // Wie hier antwoordt moet bewijzen dat hij de sleutel heeft die hij noemt, over déze
    // verbinding. Of het de goede sleutel is beslist de actor (`install`): alleen die kent
    // de pins zoals ze nu zijn.
    let Some(sleutel) = bewezen_sleutel(
        &exporter(&conn, Handtekening::OPNEMER)?,
        &ack.public_key,
        &ack.signature,
    ) else {
        conn.close(1u32.into(), b"identiteit niet bewezen");
        let _ = int
            .send(Internal::Rejected {
                target,
                status: PeerStatus::Offline {
                    reason: "identiteit niet bewezen".to_string(),
                },
            })
            .await;
        return Ok(Opgezet::Volgende {
            status_gemeld: true,
        });
    };

    if ack.peer_id == cfg.me {
        conn.close(0u32.into(), b"");
        let _ = int
            .send(Internal::Rejected {
                target,
                status: PeerStatus::Offline {
                    reason: "dit adres wijst naar onszelf".to_string(),
                },
            })
            .await;
        return Ok(Opgezet::Volgende {
            status_gemeld: true,
        });
    }

    // B3: een kandidaat is een hint. Neemt daar iemand anders op dan de peer van deze pin,
    // dan is het adres verkeerd (een oud adres, een vals adres van een ander) en gaan we
    // verder met de rest. Een alarm is het bij dezelfde `PeerId` met een andere sleutel, en
    // op het tailnet-adres uit de config, zoals in fase A.
    if doel.pin.is_some_and(|p| p != sleutel) {
        conn.close(1u32.into(), b"andere sleutel");
        let alarm = doel.known_id == Some(ack.peer_id) || bij_config;
        if alarm {
            let _ = int
                .send(Internal::Rejected {
                    target,
                    status: PeerStatus::IdentityChanged {
                        expected: doel.known_id.unwrap_or(ack.peer_id),
                        got: ack.peer_id,
                    },
                })
                .await;
        }
        return Ok(Opgezet::Volgende {
            status_gemeld: alarm,
        });
    }

    let begin = Instant::now();
    run_session(
        conn,
        send,
        recv,
        target,
        ack.peer_id,
        sleutel,
        mijn_index,
        ack.media_index,
        ack.display_name,
        ack.media_port,
        ack.app_version,
        cfg.me, // wij dialden, dus wij zijn de initiator
        int.clone(),
    )
    .await;

    // B-40, opnieuw: weigert de actor de verbinding (een pin die niet klopt, een identiteit
    // die al bij een ander target hoort), dan sluit hij hem meteen, en dan was dit geen
    // sessie. Telde het wél als sessie, dan ging de backoff terug naar één seconde en belden
    // we elke seconde opnieuw, met bij de ander elke seconde een volle online/offline-ronde.
    // Met sleutels is dat de gewone foutvorm geworden (een oude pin, een vriend met een
    // nieuwe sleutel), dus een korte sessie telt als afwijzing.
    Ok(Opgezet::Klaar(if begin.elapsed() < KORTE_SESSIE {
        Dialresultaat::Afgewezen
    } else {
        Dialresultaat::Sessie
    }))
}

/// Een sessie korter dan dit was geen sessie maar een afwijzing (zie `try_dial`).
const KORTE_SESSIE: Duration = Duration::from_secs(10);

/// Of de ander de verbinding sloot met de reden "protocolversie": zo antwoordt een peer op
/// protocol 5 (of ouder) op iedereen met een andere versie.
fn gesloten_om_versie(e: &anyhow::Error) -> bool {
    let om_versie = |c: &quinn::ConnectionError| matches!(c, quinn::ConnectionError::ApplicationClosed(a) if &a.reason[..] == b"protocolversie");
    e.chain().any(|c| {
        if let Some(c) = c.downcast_ref::<quinn::ConnectionError>() {
            return om_versie(c);
        }
        let Some(lezen) = c
            .downcast_ref::<std::io::Error>()
            .and_then(|io| io.get_ref())
            .and_then(|i| i.downcast_ref::<quinn::ReadError>())
        else {
            return false;
        };
        matches!(lezen, quinn::ReadError::ConnectionLost(c) if om_versie(c))
    })
}

// B-41 (mediapoort 0 weigeren) staat hier bewust **niet**. Zie de uitleg bij `install`:
// het weigeren van poort 0 sluit de verbinding, en `crates/app` zet in al zijn
// integratietests `media_port: 0` omdat daar geen media loopt. Die tests zijn de bewaker
// van invariant 7 en die weegt zwaarder dan een LAAG-bevinding.

async fn accept_loop(
    endpoint: quinn::Endpoint,
    cfg: MeshConfig,
    int: mpsc::Sender<Internal>,
    mut stop: watch::Receiver<bool>,
) {
    let grens = max_inkomend(cfg.targets.len());
    let lopend = Arc::new(AtomicUsize::new(0));
    // A3: pogingen per bron-IP in de lopende seconde. Lokaal in deze ene lus, dus geen lock.
    let mut pogingen: HashMap<IpAddr, u32> = HashMap::new();
    let mut seconde = Instant::now();
    let mut logrem = Logrem::default();

    loop {
        let incoming = tokio::select! {
            _ = cancelled(&mut stop) => break,
            inc = endpoint.accept() => match inc {
                Some(i) => i,
                None => break,
            },
        };

        // A3: eerst het bronadres laten bewijzen. Een Retry kost ons één klein pakket en
        // geen toestand, en is kleiner dan wat de afzender stuurde, dus wie een adres
        // vervalst krijgt geen versterking en verbruikt het budget van dat adres niet.
        if !incoming.remote_address_validated() && incoming.may_retry() {
            let _ = incoming.retry();
            continue;
        }

        let ip = incoming.remote_address().ip();
        if seconde.elapsed() >= Duration::from_secs(1) {
            pogingen.clear();
            seconde = Instant::now();
        }
        let n = pogingen.entry(ip).or_insert(0);
        *n += 1;
        if *n > MAX_POGINGEN_PER_IP {
            if logrem.mag(ip) {
                tracing::info!(%ip, "inkomende verbindingen geweigerd: te veel pogingen per seconde");
            }
            incoming.refuse();
            continue;
        }

        // B-17: pas op hoeveel er tegelijk lopen. Elke inkomende verbinding kost een taak,
        // een lezer en een framebuffer, van iemand die nog niets bewezen heeft.
        // `refuse()` stuurt meteen een weigering terug, dus een echte peer die hier tegenaan
        // loopt hangt niet en probeert het gewoon opnieuw.
        if lopend.load(Ordering::Relaxed) >= grens {
            tracing::debug!(
                grens,
                remote = %incoming.remote_address(),
                "inkomende verbinding geweigerd: te veel tegelijk"
            );
            incoming.refuse();
            continue;
        }

        let cfg = cfg.clone();
        let int = int.clone();
        let teller = Teller::nieuw(&lopend);
        tokio::spawn(async move {
            let _teller = teller;
            if let Err(e) = accept_one(incoming, cfg, int).await {
                tracing::debug!(error = %format!("{e:#}"), "inkomende verbinding afgewezen");
            }
        });
    }
}

/// Telt zolang hij leeft mee in [`accept_loop`]. Een guard en geen losse `fetch_sub`,
/// zodat elk pad eruit — fout, timeout, of een sessie van uren — hem weer vrijgeeft.
struct Teller(Arc<AtomicUsize>);

impl Teller {
    fn nieuw(n: &Arc<AtomicUsize>) -> Self {
        n.fetch_add(1, Ordering::Relaxed);
        Self(n.clone())
    }
}

impl Drop for Teller {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

async fn accept_one(
    incoming: quinn::Incoming,
    cfg: MeshConfig,
    int: mpsc::Sender<Internal>,
) -> Result<()> {
    let remote = incoming.remote_address();

    // B-17: alleen het opzetten krijgt een deadline, net als bij `try_dial`. De sessie
    // erna mag uren duren, dus de timeout kan niet om deze hele functie heen: een peer die
    // niets stuurt hield anders een taak, een verbinding en een leesbuffer bezet tot de
    // idle-timeout, en niets belette hem dat duizend keer tegelijk te doen.
    let (conn, mut send, recv, hello) = tokio::time::timeout(CONNECT_TIMEOUT, async {
        let conn = incoming.await.context("inkomende QUIC-handshake")?;
        let (send, mut recv) = conn
            .accept_bi()
            .await
            .context("control-stream accepteren")?;

        let hello = match framing::read_frame(&mut recv).await? {
            Some(Some(ControlMsg::Hello(h))) => h,
            Some(_) => bail!("eerste bericht was geen Hello"),
            None => bail!("verbinding sloot voor de Hello"),
        };

        Ok::<_, anyhow::Error>((conn, send, recv, hello))
    })
    .await
    .map_err(|_| anyhow::anyhow!("peer maakte de handshake niet af"))??;

    // B-05a: tot hier heeft de beller niets van ons gehoord, en dat blijft zo tot de
    // controles hieronder klaar zijn.
    if hello.protocol_version != PROTOCOL_VERSION {
        // Alleen de versie, zodat hij "update nodig" ziet en niet "offline". Hij sluit
        // zelf zodra hij het leest; wacht daar kort op, want wie meteen sluit gooit het
        // antwoord weg dat nog onderweg is.
        let _ = tokio::time::timeout(CONNECT_TIMEOUT, async {
            framing::write_frame(&mut send, &ControlMsg::HelloAck(HelloAck::alleen_versie()))
                .await?;
            let _ = send.finish();
            conn.closed().await;
            Ok::<_, anyhow::Error>(())
        })
        .await;
        conn.close(1u32.into(), b"protocolversie");
        bail!(
            "protocolversie {} van peer wijkt af van {}",
            hello.protocol_version,
            PROTOCOL_VERSION
        );
    }

    let Some(sleutel) = bewezen_sleutel(
        &exporter(&conn, Handtekening::BELLER)?,
        &hello.public_key,
        &hello.signature,
    ) else {
        // Stil: wie zijn sleutel niet kan bewijzen krijgt niets terug.
        conn.close(0u32.into(), b"");
        let _ = int
            .send(Internal::Geweigerd {
                ip: remote.ip(),
                reden: "handtekening klopt niet",
            })
            .await;
        return Ok(());
    };

    // Stil, net als elke andere weigering vóór de actor: met een reden erbij kon iedereen
    // met een zelfgemaakte sleutel vragen "is PeerId X van jou?".
    if hello.peer_id == cfg.me {
        conn.close(0u32.into(), b"");
        bail!("peer meldde onze eigen identiteit");
    }

    // De actor kent de pins en de targets. Hij antwoordt binnen `PENDING_GRACE`, ook als
    // het antwoord "nee" is; daarom staat dit buiten de deadline hierboven.
    //
    // Toch zelf begrensd, en weg zodra de verbinding sluit: een `quinn::Connection` houdt de
    // UDP-socket vast. Een taak die hier bleef hangen terwijl de mesh afsloot (geen tik
    // meer die de wachtrij afhandelt) hield de poort bezet, en dan start de mesh niet
    // opnieuw op dezelfde poort.
    let (antwoord, oordeel) = oneshot::channel();
    let gevraagd = int
        .send(Internal::Inkomend(Wachtend {
            sinds: Instant::now(),
            peer_id: hello.peer_id,
            sleutel,
            remote,
            antwoord,
        }))
        .await;
    let oordeel = tokio::select! {
        o = tokio::time::timeout(CONNECT_TIMEOUT, oordeel) => o,
        _ = conn.closed() => return Ok(()),
    };
    let target = match (gevraagd, oordeel) {
        (Ok(()), Ok(Ok(Some(t)))) => t,
        _ => {
            conn.close(0u32.into(), b"");
            return Ok(());
        }
    };

    let mijn_index = mediasleutel::nieuwe_index();
    tokio::time::timeout(
        CONNECT_TIMEOUT,
        framing::write_frame(
            &mut send,
            &ControlMsg::HelloAck(HelloAck {
                protocol_version: PROTOCOL_VERSION,
                peer_id: cfg.me,
                display_name: cfg.display_name.clone(),
                media_port: cfg.media_port,
                app_version: cfg.app_version.clone(),
                public_key: cfg.sleutel.public_key().as_ref().to_vec(),
                signature: teken(&cfg.sleutel, &conn, Handtekening::OPNEMER)?,
                media_index: mijn_index,
            }),
        ),
    )
    .await
    .map_err(|_| anyhow::anyhow!("HelloAck kwam niet weg"))??;

    run_session(
        conn,
        send,
        recv,
        target,
        hello.peer_id,
        sleutel,
        mijn_index,
        hello.media_index,
        hello.display_name,
        hello.media_port,
        hello.app_version,
        hello.peer_id, // zij dialden, dus zij zijn de initiator
        int,
    )
    .await;

    Ok(())
}

/// B-33: bovengrens voor de twee strings die een peer in de handshake meestuurt. Een naam
/// is een naam en een versie is een versie; alles daarboven is geen invoer maar een poging.
const MAX_DISPLAY_NAME: usize = 64;
const MAX_APP_VERSION: usize = 32;

/// Maakt van een string van de draad iets dat veilig in een logregel, de UI of een
/// bestandsnaam past.
///
/// Twee dingen gaan eruit. **Stuurtekens**, want een `\n` in een weergavenaam kan een hele
/// neplogregel fabriceren — een verzonnen "hash klopt niet" op precies het moment dat je het
/// log gebruikt om een incident te reconstrueren. En **lengte**, want er staat op de draad
/// geen enkele grens op deze velden (B-43) en de enige rem is `MAX_FRAME_LEN` van 16 MiB.
///
/// Afkappen gebeurt op chars en niet op bytes: midden in een multibyte-teken knippen zou
/// hier een paniek geven op precies het pad dat een aanvaller kiest.
fn schone_wirestring(s: &str, max: usize) -> String {
    s.chars()
        .filter(|c| !c.is_control())
        .take(max)
        .collect::<String>()
        .trim()
        .to_string()
}

/// Draait tot de verbinding weg is. Splitst lezen en schrijven in twee taken zodat een
/// trage lezer het schrijven niet blokkeert.
#[allow(clippy::too_many_arguments)]
async fn run_session(
    conn: quinn::Connection,
    mut send: quinn::SendStream,
    mut recv: quinn::RecvStream,
    target: usize,
    peer_id: PeerId,
    sleutel: [u8; 32],
    mijn_index: u32,
    hun_index: u32,
    display_name: String,
    media_port: u16,
    app_version: String,
    initiator: PeerId,
    int: mpsc::Sender<Internal>,
) {
    let conn_id = NEXT_CONN_ID.fetch_add(1, Ordering::Relaxed);
    let (out_tx, mut out_rx) = mpsc::channel::<ControlMsg>(256);

    // B-33: hier is de ene plek waar beide handshakes samenkomen, dus hier worden deze twee
    // velden schoongemaakt in plaats van bij elke consument apart. Ze komen onbegrensd en
    // ongevalideerd van de draad en gaan daarna het log, de UI en (bij `app_version`) een
    // bestandsnaam in; wie ze hier opschoont, hoeft elders niets te onthouden.
    let display_name = schone_wirestring(&display_name, MAX_DISPLAY_NAME);
    let app_version = schone_wirestring(&app_version, MAX_APP_VERSION);

    let established = Established {
        conn_id,
        target,
        peer_id,
        sleutel,
        mijn_index,
        hun_index,
        display_name,
        media_port,
        app_version,
        initiator,
        remote: conn.remote_address(),
        conn: conn.clone(),
        out: out_tx,
    };
    if int
        .send(Internal::Established(Box::new(established)))
        .await
        .is_err()
    {
        return;
    }

    let writer = tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            // B-15: een bericht dat niet in een frame past wordt **overgeslagen**, niet
            // fataal. Hier stond één `break` voor beide gevallen, en dus sloopte één te
            // grote op de control-verbinding permanent: bij herverbinding bouwde
            // `beantwoord_sync` dezelfde batch opnieuw op en brak de schrijftaak opnieuw
            // af — alleen te herstellen door de database weg te gooien.
            let bytes = match framing::frame_bytes(&msg) {
                Ok(b) => b,
                Err(e) => {
                    tracing::warn!(
                        tag = msg.tag(), error = %format!("{e:#}"),
                        "bericht overgeslagen; de verbinding blijft staan"
                    );
                    continue;
                }
            };
            // Een kapotte stream is wél het einde: dan mislukt alles wat hierna komt ook,
            // en de sessie wordt zo meteen toch opgeruimd.
            if let Err(e) = send.write_all(&bytes).await {
                tracing::debug!(error = %e, "schrijven mislukt");
                break;
            }
        }
    });

    let reader = {
        let int = int.clone();
        tokio::spawn(async move {
            loop {
                match framing::read_frame(&mut recv).await {
                    Ok(Some(Some(msg))) => {
                        if int.send(Internal::Frame { conn_id, msg }).await.is_err() {
                            break;
                        }
                    }
                    // Onbekende tag van een nieuwere peer: overslaan, verbinding intact laten.
                    Ok(Some(None)) => continue,
                    Ok(None) => break,
                    Err(e) => {
                        tracing::debug!(error = %format!("{e:#}"), "lezen mislukt");
                        break;
                    }
                }
            }
        })
    };

    // Bestandsoverdrachten komen binnen als losse uni-streams naast de control-stream,
    // zodat een groot bestand nooit chat of screenshare-control laat wachten. Deze lus
    // stopt vanzelf zodra de verbinding dicht gaat: `accept_uni` geeft dan een fout.
    let file_acceptor = {
        let int = int.clone();
        let conn = conn.clone();
        tokio::spawn(async move {
            while let Ok(stream) = conn.accept_uni().await {
                if int
                    .send(Internal::IncomingFileStream { conn_id, stream })
                    .await
                    .is_err()
                {
                    break;
                }
            }
        })
    };

    let reason = conn.closed().await.to_string();
    reader.abort();
    writer.abort();
    file_acceptor.abort();

    let _ = int.send(Internal::Closed { conn_id, reason }).await;
}

/// De IP's die deze pc voor zichzelf kiest: naar internet (achter een router meestal het
/// LAN-adres) en naar het tailnet. Er gaat geen pakket heen: een UDP-socket "verbinden" kiest
/// alleen een route en een bronadres, zoals bij A0 (`bereik.rs`).
///
/// shortcut: alleen de voorkeursroutes, geen lijst van alle interfaces (netwatch opent
/// daarvoor op Windows WMI); een tweede netwerkkaart doet zo niet mee. Bij B7 echt opsommen.
fn lokale_ips() -> Vec<IpAddr> {
    let via = |doel: &str| {
        let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
        s.connect(doel).ok()?;
        let ip = s.local_addr().ok()?.ip();
        (!ip.is_unspecified() && !ip.is_loopback()).then_some(ip)
    };
    // Een documentatieadres (TEST-NET-1) en het vaste adres van Tailscale zelf. Staat
    // Tailscale uit, dan gaat ook dat tweede via het LAN, en valt het als dubbel weg.
    let mut uit: Vec<IpAddr> = [via("192.0.2.1:9"), via("100.100.100.100:9")]
        .into_iter()
        .flatten()
        .collect();
    uit.dedup();
    uit
}

/// Volledige anyhow-ketens zijn onleesbaar in een UI-label.
fn short_reason(e: &anyhow::Error) -> String {
    let s = e.to_string();
    match s.split_once(':') {
        Some((head, _)) if head.len() > 8 => head.to_string(),
        _ => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(n: u8) -> PeerId {
        let mut b = [0u8; 16];
        b[0] = n;
        PeerId::from_bytes(b)
    }

    /// Een actor zonder netwerk eronder. `active` blijft leeg — daar hoort een echte
    /// `quinn::Connection` in en die maak je niet zonder endpoint — maar alles wat met
    /// targets, `bound` en de wachtrij te maken heeft is zo wél te testen.
    ///
    /// De ontvangkant van de `connected`-vlaggen gaat mee terug: in het echt houdt de
    /// dialer die vast, en een `watch::Sender` zonder ontvangers laat zijn waarde staan.
    fn kale_actor(
        aantal_targets: usize,
    ) -> (Actor, mpsc::Receiver<MeshEvent>, Vec<watch::Receiver<bool>>) {
        let targets: Vec<PeerTarget> = (0..aantal_targets)
            .map(|i| PeerTarget {
                address: format!("100.64.0.{}", i + 1),
                label: format!("peer{i}"),
                known_id: None,
                control_port: 41000,
                public_key: None,
                kandidaten: Vec::new(),
            })
            .collect();
        let (tx, rx) = mpsc::channel(32);
        let mut vlaggen = Vec::new();
        let mut target_state = Vec::new();
        for t in &targets {
            let (vlag_tx, vlag_rx) = watch::channel(false);
            vlaggen.push(vlag_rx);
            target_state.push(TargetState {
                connected: vlag_tx,
                doel: watch::channel(Doel::van(t)).0,
                resolved: None,
                known_id: None,
                public_key: None,
                bound: None,
                last_status: None,
            });
        }
        let actor = Actor {
            cfg: MeshConfig {
                me: peer(9),
                display_name: "ik".into(),
                control_port: 41000,
                media_port: 41700,
                targets: targets.clone(),
                app_version: "1.0.0".into(),
                sleutel: wegwerpsleutel(),
            },
            active: HashMap::new(),
            targets: target_state,
            events: tx,
            pending: Vec::new(),
            gemelde_frames: HashSet::new(),
            logrem: Logrem::default(),
            media: MediaSleutels::default(),
            spawner: None,
            eigen: Vec::new(),
            tikken: 0,
        };
        (actor, rx, vlaggen)
    }

    #[tokio::test]
    async fn b24_dode_verbinding_laat_het_target_los_ook_zonder_active() {
        // Precies de desync uit B-24: de verbinding staat niet (meer) in `active` — daar
        // is hij door een nieuwere vervangen — maar het target hangt er nog aan vast. De
        // oude code zocht via `active`, vond niets en keerde vroeg terug; die peer stond
        // daarna voorgoed "online" op een verbinding die al dicht was en werd nooit meer
        // gebeld.
        let (mut actor, mut events, _vlaggen) = kale_actor(2);
        actor.targets[0].bound = Some(7);
        let _ = actor.targets[0].connected.send(true);

        actor.on_closed(7, "verbinding weg".into()).await;

        assert_eq!(actor.targets[0].bound, None);
        assert!(
            !*actor.targets[0].connected.borrow(),
            "de dialer moet weer aan de slag mogen"
        );
        match events.try_recv() {
            Ok(MeshEvent::Status {
                target: 0,
                status: PeerStatus::Offline { .. },
            }) => {}
            anders => panic!("verwachtte Offline voor target 0, kreeg {anders:?}"),
        }
    }

    #[tokio::test]
    async fn b24_een_andere_verbinding_kickt_het_target_niet_weg() {
        // De keerzijde: een verlaten botsings-verbinding die sluit mag de verbinding die
        // wél gebonden is niet meeslepen.
        let (mut actor, mut events, _vlaggen) = kale_actor(2);
        actor.targets[0].bound = Some(7);
        let _ = actor.targets[0].connected.send(true);

        actor.on_closed(8, "verliezer van de botsing".into()).await;

        assert_eq!(actor.targets[0].bound, Some(7));
        assert!(*actor.targets[0].connected.borrow());
        assert!(events.try_recv().is_err(), "geen statuswijziging verwacht");
    }

    #[test]
    fn b24_dezelfde_identiteit_hoort_bij_hoogstens_een_target() {
        let (mut actor, _events, _vlaggen) = kale_actor(3);
        actor.targets[0].known_id = Some(peer(1));

        actor.targets[2].public_key = Some([7; 32]);

        assert_eq!(
            actor.identiteit_op_ander_target(1, peer(1), &[0; 32]),
            Some(0)
        );
        // Op zijn eigen target is het geen botsing maar een herverbinding.
        assert_eq!(actor.identiteit_op_ander_target(0, peer(1), &[0; 32]), None);
        assert_eq!(actor.identiteit_op_ander_target(1, peer(2), &[0; 32]), None);
        // En een sleutel die al bij een ander target hoort is net zo goed een botsing.
        assert_eq!(
            actor.identiteit_op_ander_target(1, peer(2), &[7; 32]),
            Some(2)
        );
    }

    #[test]
    fn b26_wachtrij_schaalt_mee_met_de_config() {
        // Invariant 3: het aantal peers komt uit de config, dus deze grens ook. Met
        // ruimte voor botsingen — twee verbindingen per peer tegelijk is normaal.
        assert!(kale_actor(3).0.max_pending() >= 3 * 2);
        assert!(kale_actor(7).0.max_pending() >= 7 * 2);
        assert!(kale_actor(0).0.max_pending() >= 1);
    }

    #[test]
    fn b17_verbindingsgrens_schaalt_mee_met_de_config_en_heeft_ruimte() {
        // Ruim boven één per peer: botsingen en een herstartende peer leveren er
        // tijdelijk meer op, en een geweigerde legitieme peer is erger dan een ruime
        // grens (invariant 7).
        assert!(max_inkomend(3) >= 3 * 8);
        assert!(max_inkomend(12) >= 12 * 8);
        // Maar wel eindig, want dat is het hele punt van B-17.
        assert!(max_inkomend(3) <= 64);
    }

    #[test]
    fn botsingsregel_is_symmetrisch() {
        // A en B bellen elkaar tegelijk. Aan beide kanten bestaan er dan twee
        // verbindingen met initiator A en initiator B. Beide moeten dezelfde kiezen,
        // anders houden ze allebei een andere over en blijft er niets werken.
        let (a, b) = (peer(1), peer(2));

        let kant_a_houdt = if a <= b { a } else { b };
        let kant_b_houdt = if a <= b { a } else { b };

        assert_eq!(kant_a_houdt, kant_b_houdt);
        assert_eq!(kant_a_houdt, a, "laagste PeerId wint als initiator");
    }

    // -- identiteit (beslissing 46) -------------------------------------------------

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn alleen_tailnet_en_loopback_mogen_een_sleutel_leren() {
        for ja in [
            "100.64.0.0",
            "100.100.1.2",
            "100.127.255.255",
            "127.0.0.1",
            "::1",
            "fd7a:115c:a1e0::1",
            "fd7a:115c:a1e0:ffff::9",
            "::ffff:100.64.0.7",
        ] {
            assert!(mag_leren(ip(ja)), "{ja} hoort te mogen");
        }
        for nee in [
            "100.63.255.255",
            "100.128.0.0",
            "192.168.1.10",
            "10.0.0.2",
            "26.1.2.3", // Radmin VPN: geen tailnet, dus geen vertrouwen op eerste gezicht
            "8.8.8.8",
            "fd7a:115c:a1e1::1",
            "2a02:a46b::1",
        ] {
            assert!(!mag_leren(ip(nee)), "{nee} hoort niet te mogen");
        }
    }

    #[test]
    fn een_handtekening_geldt_alleen_voor_haar_eigen_exporter() {
        use ring::signature::KeyPair;
        let sleutel = wegwerpsleutel();
        let publiek = sleutel.public_key().as_ref().to_vec();
        let deze = [1u8; 32];
        let andere = [2u8; 32];
        let handtekening = sleutel.sign(&deze);

        assert_eq!(
            bewezen_sleutel(&deze, &publiek, handtekening.as_ref()),
            Some(publiek.clone().try_into().unwrap())
        );
        // De man-in-the-middle: dezelfde handtekening, doorgestuurd naar een verbinding met
        // een andere exporter.
        assert_eq!(
            bewezen_sleutel(&andere, &publiek, handtekening.as_ref()),
            None
        );
        // Een andere sleutel noemen dan waarmee getekend is.
        let vreemd = wegwerpsleutel().public_key().as_ref().to_vec();
        assert_eq!(bewezen_sleutel(&deze, &vreemd, handtekening.as_ref()), None);
        // Een peer op protocol 5 stuurt niets.
        assert_eq!(bewezen_sleutel(&deze, &[], &[]), None);
    }

    #[test]
    fn beoordeling_volgt_de_pins() {
        let (mut actor, _events, _vlaggen) = kale_actor(3);
        let tailnet = ip("100.64.0.1");
        let publiek = ip("84.84.1.2");
        actor.targets[0].known_id = Some(peer(1));
        actor.targets[0].public_key = Some([1; 32]);

        assert_eq!(
            actor.beoordeel(0, peer(1), &[1; 32], publiek),
            Ok(Oordeel::Bekend)
        );
        assert_eq!(
            actor.beoordeel(0, peer(1), &[9; 32], tailnet),
            Err(Weigering::Identiteit {
                expected: peer(1),
                got: peer(1)
            }),
            "zelfde PeerId met een andere sleutel komt er nooit in, ook niet op het tailnet"
        );
        assert_eq!(
            actor.beoordeel(0, peer(2), &[1; 32], tailnet),
            Err(Weigering::Identiteit {
                expected: peer(1),
                got: peer(2)
            })
        );

        // Een installatie van vóór 6: de PeerId bekend, de sleutel nog niet.
        actor.targets[1].known_id = Some(peer(2));
        assert_eq!(
            actor.beoordeel(1, peer(2), &[2; 32], tailnet),
            Ok(Oordeel::Leer)
        );
        assert_eq!(
            actor.beoordeel(1, peer(2), &[2; 32], publiek),
            Err(Weigering::NietTeLeren)
        );

        // Een sleutel die al bij een ander target hoort (B-24).
        assert_eq!(
            actor.beoordeel(2, peer(3), &[1; 32], tailnet),
            Err(Weigering::Dubbel(0))
        );
        assert_eq!(
            actor.beoordeel(2, peer(3), &[3; 32], tailnet),
            Ok(Oordeel::Leer)
        );

        // Een sleutel zonder PeerId (met de hand in de config gezet): de id wordt bij de
        // eerste verbinding vastgelegd, ook buiten het tailnet, want de sleutel is al bewezen.
        actor.targets[2].public_key = Some([3; 32]);
        assert_eq!(
            actor.beoordeel(2, peer(3), &[3; 32], publiek),
            Ok(Oordeel::Leer)
        );
    }

    #[test]
    fn inkomend_koppelen_gaat_op_sleutel_en_leert_alleen_vanaf_het_eigen_adres() {
        let (mut actor, _events, _vlaggen) = kale_actor(2);
        let a: SocketAddr = "100.64.0.1:41000".parse().unwrap();
        let b: SocketAddr = "100.64.0.2:41000".parse().unwrap();
        actor.targets[0].known_id = Some(peer(1));
        actor.targets[0].public_key = Some([1; 32]);
        actor.targets[0].resolved = Some(a);

        // Nog geen adres voor het ongepinde target: wachten, niet weigeren.
        assert_eq!(
            actor.koppel_inkomend(peer(2), &[2; 32], b),
            Koppeling::Wacht
        );
        actor.targets[1].resolved = Some(b);
        assert_eq!(
            actor.koppel_inkomend(peer(2), &[2; 32], b),
            Koppeling::Target(1)
        );
        // Vanaf een ander adres leren we niets.
        let elders: SocketAddr = "100.64.0.9:41000".parse().unwrap();
        assert_eq!(
            actor.koppel_inkomend(peer(2), &[2; 32], elders),
            Koppeling::Geen
        );

        // De gepinde sleutel wint, vanaf welk adres ook.
        assert_eq!(
            actor.koppel_inkomend(peer(1), &[1; 32], elders),
            Koppeling::Target(0)
        );
        // Zelfde PeerId, andere sleutel: naar dat target, zodat het alarm kan vuren.
        assert_eq!(
            actor.koppel_inkomend(peer(1), &[9; 32], b),
            Koppeling::Target(0)
        );
    }

    /// Een vrije poort, onthouden zodat parallelle tests hem niet dubbel krijgen.
    fn vrije_poort() -> u16 {
        use std::sync::{Mutex, OnceLock};
        static UITGEDEELD: OnceLock<Mutex<HashSet<u16>>> = OnceLock::new();
        let uitgedeeld = UITGEDEELD.get_or_init(Default::default);
        loop {
            let s = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
            let p = s.local_addr().unwrap().port();
            if uitgedeeld.lock().unwrap().insert(p) {
                return p;
            }
        }
    }

    /// Een peer met één target op 127.0.0.1:`ander`, zoals de app hem zou starten.
    fn peer_met_target(poort: u16, ander: u16, pin: Option<(PeerId, [u8; 32])>) -> MeshHandle {
        spawn(MeshConfig {
            me: PeerId::new_random(),
            display_name: "B".into(),
            control_port: poort,
            media_port: 0,
            app_version: "1.0.0".into(),
            sleutel: wegwerpsleutel(),
            targets: vec![PeerTarget {
                address: "127.0.0.1".into(),
                label: "A".into(),
                known_id: pin.map(|(id, _)| id),
                control_port: ander,
                public_key: pin.map(|(_, k)| k),
                kandidaten: Vec::new(),
            }],
        })
        .unwrap()
    }

    /// Een kale QUIC-client die zelf bepaalt wat er in de `Hello` staat.
    fn rauwe_client(poort: u16) -> quinn::Endpoint {
        let mut ep = quinn::Endpoint::client(SocketAddr::from(([127, 0, 0, 1], poort))).unwrap();
        ep.set_default_client_config(tls::client_config().unwrap());
        ep
    }

    fn hello(
        peer_id: PeerId,
        versie: u32,
        sleutel: &Ed25519KeyPair,
        handtekening: Vec<u8>,
    ) -> ControlMsg {
        ControlMsg::Hello(Hello {
            protocol_version: versie,
            peer_id,
            display_name: "A".into(),
            media_port: 0,
            app_version: "1.0.0".into(),
            public_key: sleutel.public_key().as_ref().to_vec(),
            signature: handtekening,
            media_index: 0,
        })
    }

    /// Stuurt een `Hello` en levert wat er terugkomt: `Some` met de `HelloAck`, of `None`
    /// als de ander zonder antwoord sloot.
    async fn vraag(conn: &quinn::Connection, msg: &ControlMsg) -> Option<HelloAck> {
        let (mut send, mut recv) = conn.open_bi().await.unwrap();
        framing::write_frame(&mut send, msg).await.unwrap();
        let antwoord = tokio::time::timeout(Duration::from_secs(8), framing::read_frame(&mut recv))
            .await
            .expect("een stille close moet de verbinding sluiten, niet laten hangen");
        match antwoord {
            Ok(Some(Some(ControlMsg::HelloAck(a)))) => Some(a),
            _ => None,
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn een_doorgestuurde_handtekening_krijgt_geen_hello_ack() {
        let (pa, pb) = (vrije_poort(), vrije_poort());
        let _b = peer_met_target(pb, pa, None);
        let doel = SocketAddr::from(([127, 0, 0, 1], pb));
        let ep = rauwe_client(pa);
        let echt = ep.connect(doel, tls::SERVER_NAME).unwrap().await.unwrap();
        let mitm = ep.connect(doel, tls::SERVER_NAME).unwrap().await.unwrap();

        let a = PeerId::new_random();
        let sleutel = wegwerpsleutel();
        let handtekening = teken(&sleutel, &echt, Handtekening::BELLER).unwrap();

        // Dezelfde handtekening over een andere verbinding: de exporter klopt niet.
        let doorgestuurd = hello(a, PROTOCOL_VERSION, &sleutel, handtekening.clone());
        assert!(
            vraag(&mitm, &doorgestuurd).await.is_none(),
            "een HelloAck vóór (of zonder) een geldige handtekening lekt wie we zijn (B-05a)"
        );

        // Controle op de opzet: op de eigen verbinding komt hij wél binnen (TOFU op
        // loopback, vanaf het adres van het target).
        let ack = vraag(&echt, &doorgestuurd)
            .await
            .expect("eigen verbinding hoort te lukken");
        assert_eq!(ack.public_key.len(), Handtekening::SLEUTEL_LEN);
        assert!(bewezen_sleutel(
            &exporter(&echt, Handtekening::OPNEMER).unwrap(),
            &ack.public_key,
            &ack.signature
        )
        .is_some());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn een_onbekende_sleutel_krijgt_een_stille_close() {
        let (pa, pb, vreemd) = (vrije_poort(), vrije_poort(), vrije_poort());
        let a = PeerId::new_random();
        let a_sleutel = wegwerpsleutel();
        let pin = a_sleutel.public_key().as_ref().try_into().unwrap();
        let _b = peer_met_target(pb, pa, Some((a, pin)));

        let ep = rauwe_client(vreemd);
        let conn = ep
            .connect(SocketAddr::from(([127, 0, 0, 1], pb)), tls::SERVER_NAME)
            .unwrap()
            .await
            .unwrap();
        // Een geldige handtekening, maar met een sleutel die nergens vastgepind staat, en
        // vanaf een adres waar we niets leren.
        let sleutel = wegwerpsleutel();
        let handtekening = teken(&sleutel, &conn, Handtekening::BELLER).unwrap();
        let msg = hello(
            PeerId::new_random(),
            PROTOCOL_VERSION,
            &sleutel,
            handtekening,
        );
        assert!(vraag(&conn, &msg).await.is_none());

        // En wie A's PeerId claimt met een andere sleutel evenmin.
        let conn = ep
            .connect(SocketAddr::from(([127, 0, 0, 1], pb)), tls::SERVER_NAME)
            .unwrap()
            .await
            .unwrap();
        let handtekening = teken(&sleutel, &conn, Handtekening::BELLER).unwrap();
        assert!(
            vraag(&conn, &hello(a, PROTOCOL_VERSION, &sleutel, handtekening))
                .await
                .is_none()
        );
    }

    /// Zo antwoordt een peer op protocol 5 (de code van 1.10.1): `HelloAck` met zijn versie
    /// erin, en in dezelfde tik dicht met "protocolversie". Het antwoord komt meestal niet
    /// aan; wie al op 6 zit moet toch "update nodig" zien en niet "offline".
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn een_peer_op_protocol_5_aan_de_andere_kant_is_een_versieverschil() {
        let (pa, pb) = (vrije_poort(), vrije_poort());
        let oud = quinn::Endpoint::server(
            tls::server_config().unwrap(),
            SocketAddr::from(([127, 0, 0, 1], pb)),
        )
        .unwrap();
        tokio::spawn(async move {
            while let Some(inkomend) = oud.accept().await {
                let Ok(conn) = inkomend.await else { continue };
                let Ok((mut send, mut recv)) = conn.accept_bi().await else {
                    continue;
                };
                let _ = framing::read_frame(&mut recv).await;
                let mut ack = HelloAck::alleen_versie();
                ack.protocol_version = 5;
                let _ = framing::write_frame(&mut send, &ControlMsg::HelloAck(ack)).await;
                conn.close(1u32.into(), b"protocolversie");
            }
        });

        let mut a = peer_met_target(pa, pb, None);
        let status = tokio::time::timeout(Duration::from_secs(10), async {
            while let Some(ev) = a.events.recv().await {
                if let MeshEvent::Status {
                    status: s @ (PeerStatus::VersionMismatch { .. } | PeerStatus::Online { .. }),
                    ..
                } = ev
                {
                    return s;
                }
            }
            panic!("kanaal dicht");
        })
        .await
        .expect("de oude peer had als versieverschil moeten verschijnen");
        assert!(
            matches!(
                status,
                PeerStatus::VersionMismatch {
                    theirs: 0 | 5,
                    ours: PROTOCOL_VERSION
                }
            ),
            "{status:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn een_peer_op_protocol_5_hoort_alleen_de_versie() {
        let (pa, pb) = (vrije_poort(), vrije_poort());
        let _b = peer_met_target(pb, pa, None);
        let ep = rauwe_client(pa);
        let conn = ep
            .connect(SocketAddr::from(([127, 0, 0, 1], pb)), tls::SERVER_NAME)
            .unwrap()
            .await
            .unwrap();
        let mut oud = hello(PeerId::new_random(), 5, &wegwerpsleutel(), Vec::new());
        if let ControlMsg::Hello(h) = &mut oud {
            h.public_key.clear();
        }

        let ack = vraag(&conn, &oud)
            .await
            .expect("hij moet 'update nodig' kunnen zien in plaats van 'offline'");
        assert_eq!(ack, HelloAck::alleen_versie());
    }
}
