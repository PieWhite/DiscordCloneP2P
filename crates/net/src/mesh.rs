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

use crate::{framing, tls};
use anyhow::{bail, Context, Result};
use fitcom_proto::control::{Hello, HelloAck};
use fitcom_proto::{ControlMsg, PeerId, PROTOCOL_VERSION};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, watch};

const BACKOFF_START: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(30);
/// QUIC praat over UDP, dus een peer die uit staat weigert niets — die zwijgt gewoon.
/// Zonder eigen deadline blijven we tot de idle-timeout hangen en staat er seconden
/// lang "verbinden…" in de UI terwijl er niemand thuis is.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(4);
/// Hoe vaak we een verse RTT naar de UI duwen.
const STATUS_TICK: Duration = Duration::from_secs(1);

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
}

#[derive(Debug, Clone)]
pub struct PeerTarget {
    /// Tailnet-IP of MagicDNS-naam.
    pub address: String,
    pub label: String,
    /// Bekend uit eerdere sessies (trust-on-first-use).
    pub known_id: Option<PeerId>,
    pub control_port: u16,
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
    /// één van beiden update.
    VersionMismatch {
        theirs: u32,
        ours: u32,
    },
    /// Vanaf een bekend adres meldde zich een andere identiteit dan we eerder zagen.
    IdentityChanged {
        expected: PeerId,
        got: PeerId,
    },
}

#[derive(Debug)]
pub enum MeshEvent {
    Status {
        target: usize,
        status: PeerStatus,
    },
    /// Eerste keer dat we de identiteit achter een adres leren. De app hoort dit in
    /// de config vast te leggen zodat we het adres later kunnen verifiëren.
    LearnedIdentity {
        target: usize,
        peer_id: PeerId,
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
        }
    }
}

pub struct MeshHandle {
    pub commands: mpsc::Sender<MeshCommand>,
    pub events: mpsc::Receiver<MeshEvent>,
    /// Het adres waarop we daadwerkelijk luisteren.
    pub local_addr: SocketAddr,
    endpoint: quinn::Endpoint,
    _shutdown: ShutdownGuard,
}

impl MeshHandle {
    /// Sluit netjes af en wacht tot de UDP-poort weer vrij is.
    ///
    /// `drop` alleen is niet genoeg: dat zet de taken stil maar keert terug voordat
    /// de socket daadwerkelijk los is. Wie op dezelfde poort opnieuw wil starten —
    /// bij een config-herlaad bijvoorbeeld — moet dit gebruiken.
    pub async fn shutdown(self) {
        let endpoint = self.endpoint.clone();
        drop(self); // zet alle taken stil via de ShutdownGuard
        endpoint.close(0u32.into(), b"afsluiten");
        endpoint.wait_idle().await;
    }
}

/// Zet alle netwerktaken stil zodra de handle wegvalt. Zonder dit blijft de UDP-poort
/// bezet en blijven dialers doorproberen nadat de app ze niet meer nodig heeft.
struct ShutdownGuard(watch::Sender<bool>);

impl Drop for ShutdownGuard {
    fn drop(&mut self) {
        let _ = self.0.send(true);
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
    /// `None` voor een inkomende verbinding die nog aan een target gekoppeld moet worden.
    target: Option<usize>,
    peer_id: PeerId,
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

    let mut endpoint = quinn::Endpoint::server(tls::server_config()?, bind)
        .with_context(|| format!("QUIC-endpoint binden op {bind}"))?;
    endpoint.set_default_client_config(tls::client_config()?);
    let local_addr = endpoint.local_addr().context("luisteradres opvragen")?;
    tracing::info!(%local_addr, me = %cfg.me, "mesh luistert");

    let (cmd_tx, cmd_rx) = mpsc::channel(256);
    let (event_tx, event_rx) = mpsc::channel(1024);
    let (int_tx, int_rx) = mpsc::channel(1024);
    let (stop_tx, stop_rx) = watch::channel(false);

    // Commando's van de UI lopen door dezelfde interne wachtrij als netwerkgebeurtenissen,
    // zodat de actor één bron van waarheid heeft en geen locks nodig heeft.
    {
        let int_tx = int_tx.clone();
        tokio::spawn(async move {
            let mut cmd_rx = cmd_rx;
            while let Some(c) = cmd_rx.recv().await {
                if int_tx.send(Internal::Command(c)).await.is_err() {
                    break;
                }
            }
        });
    }

    // Elke target krijgt een vlag "er is verbinding". De dialer wacht daarop, zodat we
    // niet blijven dialen naar een peer die ons al gebeld heeft.
    let mut connected_flags = Vec::new();
    for (idx, target) in cfg.targets.iter().enumerate() {
        let (tx, rx) = watch::channel(false);
        connected_flags.push(tx);
        tokio::spawn(dial_loop(
            endpoint.clone(),
            cfg.clone(),
            idx,
            target.clone(),
            int_tx.clone(),
            rx,
            stop_rx.clone(),
        ));
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

    tokio::spawn(
        Actor {
            cfg,
            active: HashMap::new(),
            targets: connected_flags
                .into_iter()
                .map(|flag| TargetState {
                    connected: flag,
                    resolved: None,
                    known_id: None,
                    bound: None,
                    last_status: None,
                })
                .collect(),
            events: event_tx,
            pending: Vec::new(),
        }
        .run(int_rx),
    );

    Ok(MeshHandle {
        commands: cmd_tx,
        events: event_rx,
        local_addr,
        endpoint,
        _shutdown: ShutdownGuard(stop_tx),
    })
}

// ---------------------------------------------------------------------------
// Actor
// ---------------------------------------------------------------------------

struct TargetState {
    connected: watch::Sender<bool>,
    resolved: Option<SocketAddr>,
    known_id: Option<PeerId>,
    bound: Option<u64>,
    last_status: Option<PeerStatus>,
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
    /// Inkomende verbindingen die we nog niet aan een target konden koppelen.
    ///
    /// Koppelen gebeurt op het geresolvede IP van een target, en dat weten we pas
    /// zodra onze eigen dialer dat adres heeft opgezocht. Belt de ander eerder dan dat,
    /// dan zouden we een volstrekt legitieme verbinding weigeren. Daarom parkeren we
    /// hem kort in plaats van hem weg te gooien.
    pending: Vec<(std::time::Instant, Box<Established>)>,
}

/// Hoe lang een niet te koppelen inkomende verbinding blijft wachten voordat we
/// concluderen dat hij echt niet in de config staat.
const PENDING_GRACE: Duration = Duration::from_secs(5);

impl Actor {
    async fn run(mut self, mut rx: mpsc::Receiver<Internal>) {
        for (idx, t) in self.cfg.targets.iter().enumerate() {
            self.targets[idx].known_id = t.known_id;
        }

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
                Internal::Established(e) => self.on_established(*e).await,
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

    async fn on_command(&mut self, cmd: MeshCommand) {
        match cmd {
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

    async fn on_established(&mut self, e: Established) {
        // Koppel een inkomende verbinding aan een geconfigureerde target.
        match e.target.or_else(|| self.match_inbound(&e)) {
            Some(target) => self.install(e, target).await,
            None => {
                tracing::debug!(
                    peer = ?e.peer_id, remote = %e.remote,
                    "inkomende verbinding nog niet te koppelen, geparkeerd"
                );
                self.pending.push((std::time::Instant::now(), Box::new(e)));
            }
        }
    }

    /// Probeert geparkeerde verbindingen opnieuw te koppelen en gooit weg wat te oud is.
    async fn retry_pending(&mut self) {
        if self.pending.is_empty() {
            return;
        }

        let mut still_waiting = Vec::new();
        for (since, e) in std::mem::take(&mut self.pending) {
            match self.match_inbound(&e) {
                Some(target) => self.install(*e, target).await,
                None if since.elapsed() < PENDING_GRACE => still_waiting.push((since, e)),
                None => {
                    tracing::warn!(
                        peer = ?e.peer_id, remote = %e.remote,
                        "verbinding geweigerd: deze peer staat niet in de config"
                    );
                    e.conn.close(1u32.into(), b"onbekende peer");
                }
            }
        }
        self.pending = still_waiting;
    }

    async fn install(&mut self, e: Established, target: usize) {
        // Identiteitscontrole: hetzelfde adres hoort dezelfde peer te zijn.
        match self.targets[target].known_id {
            Some(known) if known != e.peer_id => {
                tracing::error!(
                    expected = ?known, got = ?e.peer_id, remote = %e.remote,
                    "identiteit achter dit adres is veranderd"
                );
                e.conn.close(1u32.into(), b"identiteit veranderd");
                self.set_status(
                    target,
                    PeerStatus::IdentityChanged {
                        expected: known,
                        got: e.peer_id,
                    },
                )
                .await;
                return;
            }
            Some(_) => {}
            None => {
                self.targets[target].known_id = Some(e.peer_id);
                let _ = self
                    .events
                    .send(MeshEvent::LearnedIdentity {
                        target,
                        peer_id: e.peer_id,
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

        let media_addr = SocketAddr::new(e.remote.ip(), e.media_port);
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

    /// Bepaalt bij welke geconfigureerde peer een inkomende verbinding hoort.
    ///
    /// Matchen op alleen het IP is niet genoeg: draaien er meerdere peers achter
    /// hetzelfde adres — drie instanties op één PC, of straks peers achter dezelfde
    /// exit node — dan wijzen we de verbinding aan de verkeerde toe en slaat de
    /// identiteitscontrole daarna ten onrechte alarm.
    fn match_inbound(&self, e: &Established) -> Option<usize> {
        // 1. Kennen we deze identiteit al, dan is het adres niet interessant.
        if let Some(i) = self
            .targets
            .iter()
            .position(|t| t.known_id == Some(e.peer_id))
        {
            return Some(i);
        }

        let unclaimed = |t: &&TargetState| t.known_id.is_none();

        // 2. Exact adres inclusief poort. Peers dialen vanaf de socket waarop ze zelf
        //    luisteren, dus de bronpoort is hun `control_port`.
        if let Some(i) = self
            .targets
            .iter()
            .position(|t| unclaimed(&t) && t.resolved == Some(e.remote))
        {
            return Some(i);
        }

        // 3. Alleen het IP, maar uitsluitend als dat ondubbelzinnig één peer aanwijst.
        //    Vangt het geval af waarin een NAT onderweg de bronpoort herschrijft.
        let mut hits = self
            .targets
            .iter()
            .enumerate()
            .filter(|(_, t)| unclaimed(t) && t.resolved.map(|a| a.ip()) == Some(e.remote.ip()));

        match (hits.next(), hits.next()) {
            (Some((i, _)), None) => Some(i),
            _ => None,
        }
    }

    async fn on_frame(&mut self, conn_id: u64, msg: ControlMsg) {
        let Some((&from, _)) = self.active.iter().find(|(_, a)| a.conn_id == conn_id) else {
            tracing::warn!(
                conn = conn_id,
                ?msg,
                "frame van onbekende verbinding genegeerd"
            );
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
        let Some((&peer, _)) = self.active.iter().find(|(_, a)| a.conn_id == conn_id) else {
            return;
        };
        let active = self.active.remove(&peer).expect("net gevonden");
        tracing::info!(peer = ?peer, %reason, "verbinding verbroken");

        if let Some(t) = active.target {
            // Alleen loslaten als deze verbinding ook echt de gebonden verbinding was;
            // anders zou een verlaten botsings-verbinding de goede verbinding wegkicken.
            if self.targets[t].bound == Some(conn_id) {
                self.targets[t].bound = None;
                let _ = self.targets[t].connected.send(false);
                self.set_status(t, PeerStatus::Offline { reason }).await;
            }
        }
    }

    async fn on_tick(&mut self) {
        self.retry_pending().await;

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
    peer: PeerTarget,
    int: mpsc::Sender<Internal>,
    connected: watch::Receiver<bool>,
    mut stop: watch::Receiver<bool>,
) {
    tokio::select! {
        _ = cancelled(&mut stop) => {}
        _ = dial_forever(endpoint, cfg, target, peer, int, connected) => {}
    }
}

async fn dial_forever(
    endpoint: quinn::Endpoint,
    cfg: MeshConfig,
    target: usize,
    peer: PeerTarget,
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

        let _ = int.send(Internal::Connecting { target }).await;

        match try_dial(&endpoint, &cfg, target, &peer, &int).await {
            Ok(()) => {
                backoff = BACKOFF_START;
            }
            Err(e) => {
                tracing::debug!(address = %peer.address, error = %format!("{e:#}"), "verbinden mislukt");
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

        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(BACKOFF_MAX);
    }
}

/// Verbindt, doet de handshake en keert pas terug als de verbinding weer weg is.
async fn try_dial(
    endpoint: &quinn::Endpoint,
    cfg: &MeshConfig,
    target: usize,
    peer: &PeerTarget,
    int: &mpsc::Sender<Internal>,
) -> Result<()> {
    let host = format!("{}:{}", peer.address, peer.control_port);
    let addr = tokio::net::lookup_host(&host)
        .await
        .with_context(|| format!("{host} opzoeken"))?
        .next()
        .with_context(|| format!("{host} leverde geen adres op"))?;

    let _ = int.send(Internal::Resolved { target, addr }).await;

    // Alleen het opzetten krijgt een deadline. De sessie erna mag uren duren.
    let (conn, send, recv, ack) = tokio::time::timeout(CONNECT_TIMEOUT, async {
        let conn = endpoint
            .connect(addr, tls::SERVER_NAME)?
            .await
            .with_context(|| format!("QUIC-verbinding naar {addr}"))?;

        let (mut send, mut recv) = conn.open_bi().await.context("control-stream openen")?;

        framing::write_frame(
            &mut send,
            &ControlMsg::Hello(Hello {
                protocol_version: PROTOCOL_VERSION,
                peer_id: cfg.me,
                display_name: cfg.display_name.clone(),
                media_port: cfg.media_port,
                app_version: cfg.app_version.clone(),
            }),
        )
        .await?;

        let ack = match framing::read_frame(&mut recv).await? {
            Some(Some(ControlMsg::HelloAck(a))) => a,
            Some(_) => bail!("peer antwoordde niet met HelloAck"),
            None => bail!("peer sloot de verbinding tijdens de handshake"),
        };

        Ok::<_, anyhow::Error>((conn, send, recv, ack))
    })
    .await
    .map_err(|_| anyhow::anyhow!("peer reageert niet"))??;

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
        return Ok(());
    }

    if ack.peer_id == cfg.me {
        bail!("dit adres wijst naar onszelf; controleer de config");
    }

    run_session(
        conn,
        send,
        recv,
        Some(target),
        ack.peer_id,
        ack.display_name,
        ack.media_port,
        ack.app_version,
        cfg.me, // wij dialden, dus wij zijn de initiator
        int.clone(),
    )
    .await;

    Ok(())
}

async fn accept_loop(
    endpoint: quinn::Endpoint,
    cfg: MeshConfig,
    int: mpsc::Sender<Internal>,
    mut stop: watch::Receiver<bool>,
) {
    loop {
        let incoming = tokio::select! {
            _ = cancelled(&mut stop) => break,
            inc = endpoint.accept() => match inc {
                Some(i) => i,
                None => break,
            },
        };

        let cfg = cfg.clone();
        let int = int.clone();
        tokio::spawn(async move {
            if let Err(e) = accept_one(incoming, cfg, int).await {
                tracing::debug!(error = %format!("{e:#}"), "inkomende verbinding afgewezen");
            }
        });
    }
}

async fn accept_one(
    incoming: quinn::Incoming,
    cfg: MeshConfig,
    int: mpsc::Sender<Internal>,
) -> Result<()> {
    let conn = incoming.await.context("inkomende QUIC-handshake")?;
    let (mut send, mut recv) = conn
        .accept_bi()
        .await
        .context("control-stream accepteren")?;

    let hello = match framing::read_frame(&mut recv).await? {
        Some(Some(ControlMsg::Hello(h))) => h,
        Some(_) => bail!("eerste bericht was geen Hello"),
        None => bail!("verbinding sloot voor de Hello"),
    };

    framing::write_frame(
        &mut send,
        &ControlMsg::HelloAck(HelloAck {
            protocol_version: PROTOCOL_VERSION,
            peer_id: cfg.me,
            display_name: cfg.display_name.clone(),
            media_port: cfg.media_port,
            app_version: cfg.app_version.clone(),
        }),
    )
    .await?;

    if hello.protocol_version != PROTOCOL_VERSION {
        // De ander ziet dit ook en meldt het aan zijn gebruiker; wij sluiten netjes.
        conn.close(1u32.into(), b"protocolversie");
        bail!(
            "protocolversie {} van peer wijkt af van {}",
            hello.protocol_version,
            PROTOCOL_VERSION
        );
    }

    if hello.peer_id == cfg.me {
        conn.close(1u32.into(), b"zelfverbinding");
        bail!("peer meldde onze eigen identiteit");
    }

    run_session(
        conn,
        send,
        recv,
        None, // target volgt uit de koppeling in de actor
        hello.peer_id,
        hello.display_name,
        hello.media_port,
        hello.app_version,
        hello.peer_id, // zij dialden, dus zij zijn de initiator
        int,
    )
    .await;

    Ok(())
}

/// Draait tot de verbinding weg is. Splitst lezen en schrijven in twee taken zodat een
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

/// trage lezer het schrijven niet blokkeert.
#[allow(clippy::too_many_arguments)]
async fn run_session(
    conn: quinn::Connection,
    mut send: quinn::SendStream,
    mut recv: quinn::RecvStream,
    target: Option<usize>,
    peer_id: PeerId,
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
            if let Err(e) = framing::write_frame(&mut send, &msg).await {
                tracing::debug!(error = %format!("{e:#}"), "schrijven mislukt");
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
}
