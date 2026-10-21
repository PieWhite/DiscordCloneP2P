//! Wie deelt wat, en wie kijkt waarnaar.
//!
//! Alle beslissingen over screenshare staan hier, en niets in deze module raakt een
//! GPU, een scherm of een socket aan. Wat er moet gebeuren komt eruit als [`Actie`];
//! de motor voert die uit. Daardoor is het gedrag te testen zonder hardware — precies
//! zoals de chat-logica in `chat.rs`.
//!
//! # De regel die alles bij elkaar houdt
//!
//! **Er wordt pas opgenomen en gecodeerd als er iemand kijkt.** Een aangekondigde bron
//! kost niets: geen capture, geen encoder, geen verkeer. Pas bij de eerste
//! `StreamSubscribe` begint het werk, en bij de laatste `StreamUnsubscribe` stopt het
//! weer. Dat is de reden dat je een scherm gedeeld kunt laten staan terwijl je gamet.
//!
//! # Waarom stream-id's alleen binnen één peer uniek zijn
//!
//! De deler kent ze uit: 1, 2, 3. Twee peers delen dus allebei een stream 1. Dat mag,
//! want een stream wordt overal geïdentificeerd door het paar (eigenaar, id) — en op
//! de draad door de UDP-poort waarop de kijker luistert, die per stream apart is.
//!
//! # Tekenen op een gedeeld scherm (beslissing 45)
//!
//! De deler is het middelpunt van zijn eigen scherm. Een kijker stuurt zijn lijnen alleen
//! naar de deler; de deler neemt ze alleen aan van wie nu kijkt, past ze toe op zijn eigen
//! [`Tekening`] en geeft ze in die volgorde door aan al zijn kijkers, de tekenaar zelf
//! inbegrepen — die ziet zijn lijn dus pas als hij terugkomt, zodat iedereen precies
//! hetzelfde ziet. Een kijker neemt tekenwerk alleen aan van de eigenaar van een stream
//! waar hij naar kijkt. Na elk wissen en bij elke nieuwe kijker stuurt de deler de hele
//! tekening. Stream-id's zijn alleen per eigenaar uniek, dus een `Draw` zonder tekenaar is
//! altijd "op jouw scherm" en een `Draw` mét tekenaar altijd "op het scherm van de afzender".
//!
//! # Een scherm laten bedienen (beslissing 54)
//!
//! Zelfde vorm als tekenen, maar met één bediener tegelijk en een klik van de deler ertussen.
//! Een kijker van een eigen *monitor* waarvoor de optie aan staat vraagt (`RemoteRequest`); de
//! deler onthoudt één vrager, de gebruiker klikt Allow of Deny, en pas dan is die kijker de
//! bediener (`RemoteState`). Alleen van hem, terwijl hij nog kijkt en de optie nog aan staat,
//! neemt de deler invoer en klembord aan; de afzender komt uit de verbinding. Het bedienen
//! stopt bij intrekken, de optie uit, stoppen met delen, en als de bediener stopt met kijken,
//! wegvalt of zelf stopt. Aan de kijkerskant neemt de motor een "ja" alleen aan als hij vroeg,
//! en alleen van de eigenaar van de stream; invoer en klembord gaan pas weg als hij bediener is.
//! Het klembord: stream-id's zijn per eigenaar uniek, dus een `ClipboardText` is eerst "voor
//! mijn scherm" als de afzender het bedient, en anders "van zijn scherm" als ik het zijne bedien.
//! Beide zetten hetzelfde klembord van deze PC.

use fitcom_net::{MeshCommand, PeerAdres};
use fitcom_proto::bediening::{
    begrens, schoon_klembord, Bedienstand, Emmer, INVOER_PER_SECONDE, INVOER_VOORRAAD,
    KLEMBORD_PER_SECONDE, KLEMBORD_VOORRAAD,
};
use fitcom_proto::control::{
    ClipboardText, Draw, DrawClear, DrawClearScope, DrawState, InputEvent, RemoteInput,
    RemoteRequest, RemoteState, RequestKeyframe, StreamAnnounce, StreamKind, StreamRevoke,
    StreamSubscribe, StreamUnsubscribe,
};
use fitcom_proto::teken::Tekening;
use fitcom_proto::{ControlMsg, PeerId, VOICE_STREAM_ID};
use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Hooguit zo vaak haalt een kijker de hele tekening opnieuw als zijn kopie een gat heeft.
/// Een gat komt alleen van verlies onderweg; elke keer opnieuw vragen zou bij een peer met
/// een nieuwere versie (andere grenzen) een lus worden.
const HERSYNC_PAUZE: Duration = Duration::from_secs(2);

/// Zo lang blijft een vraag om te bedienen staan zonder antwoord; daarna is het nee. Anders
/// staat de vraag nog in beeld als de deler na een potje terugkomt, en klikt hij op iets wat de
/// vrager allang vergeten is.
pub const VRAAG_GELDIG: Duration = Duration::from_secs(60);
/// Vragen per scherm: drie als uitschieter, dan één per vijf seconden. Een vraag geeft de deler
/// een melding, en die mag geen kijker blijven afvuren.
const VRAGEN_VOORRAAD: u32 = 3;
const VRAGEN_PER_SECONDE: f64 = 0.2;

/// Wat de deler per scherm toestaat. Uit de kiezer bij het delen, of de standaard uit de config
/// (`allow_drawing`, `allow_remote_control`) bij delen zonder kiezer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeelOpties {
    pub tekenen: bool,
    /// Alleen ooit waar voor een monitor; de motor zet hem op macOS altijd uit.
    pub bedienen: bool,
}

/// Wat de motor moet doen. Alles wat hardware aanraakt staat hier als opdracht en
/// wordt buiten deze module uitgevoerd.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Actie {
    /// Begin met opnemen en coderen: er kijkt nu iemand.
    StartDelen {
        stream_id: u32,
        kijkers: Vec<PeerAdres>,
    },
    /// Dezelfde stream, andere kijkers.
    ZetKijkers {
        stream_id: u32,
        kijkers: Vec<PeerAdres>,
    },
    /// Stop met opnemen en coderen.
    StopDelen { stream_id: u32, is_geluid: bool },
    /// Volgend beeld als keyframe versturen.
    StuurKeyframe { stream_id: u32 },
    /// Open een venster en ga luisteren.
    StartKijken {
        eigenaar: PeerId,
        stream_id: u32,
        titel: String,
        breedte: u32,
        hoogte: u32,
        /// Geluid in plaats van beeld: geen venster, geen eigen poort — dat komt binnen
        /// op de voice-poort en gaat de bestaande mixer in.
        is_geluid: bool,
    },
    /// Sluit het venster.
    StopKijken { eigenaar: PeerId, stream_id: u32 },
    /// De tekening op een scherm is veranderd: opnieuw tonen in het kijkvenster van die
    /// stream, of (`eigenaar == None`) in de laag over ons eigen gedeelde scherm.
    Tekening {
        eigenaar: Option<PeerId>,
        stream_id: u32,
    },
    /// Iemand vraagt ons scherm te bedienen: een melding, want de deler zit misschien te gamen.
    BedienVraag { stream_id: u32, van: PeerId },
    /// De bediener is toegelaten: invoer en klembord bij de deler beginnen.
    StartBediening { stream_id: u32 },
    /// Het bedienen van ons scherm is voorbij: alles los, klembord dicht.
    StopBediening { stream_id: u32 },
    /// Invoer van onze bediener, gecontroleerd en afgeremd.
    Invoer {
        stream_id: u32,
        events: Vec<InputEvent>,
    },
    /// Klembordtekst op het klembord van deze PC zetten: van onze bediener (`eigenaar ==
    /// None`) of van de deler waarvan we het scherm bedienen.
    Klembord {
        eigenaar: Option<PeerId>,
        stream_id: u32,
        tekst: String,
    },
    /// Onze bedienstand voor deze stream van een ander is (misschien) anders: naar het venster.
    Bediening { eigenaar: PeerId, stream_id: u32 },
}

/// Een eigen scherm laten bedienen.
#[derive(Debug, Clone)]
pub struct EigenBediening {
    /// De optie van de deler voor dit scherm. Alleen ooit waar voor een monitor.
    pub mag: bool,
    /// Wie vroeg en wanneer; wacht op een klik van de deler. Eén tegelijk.
    pub vrager: Option<(PeerId, Instant)>,
    /// Wie nu bedient. Eén tegelijk.
    pub bediener: Option<PeerId>,
    invoer: Emmer,
    klembord: Emmer,
    vragen: Emmer,
}

impl EigenBediening {
    fn nieuw(mag: bool) -> Self {
        let nu = Instant::now();
        Self {
            mag,
            vrager: None,
            bediener: None,
            invoer: Emmer::nieuw(INVOER_VOORRAAD, INVOER_PER_SECONDE, nu),
            klembord: Emmer::nieuw(KLEMBORD_VOORRAAD, KLEMBORD_PER_SECONDE, nu),
            vragen: Emmer::nieuw(VRAGEN_VOORRAAD, VRAGEN_PER_SECONDE, nu),
        }
    }
}

fn bedienstand_naar(peer: PeerId, stream_id: u32, granted: bool) -> MeshCommand {
    MeshCommand::Send {
        to: peer,
        msg: ControlMsg::RemoteState(RemoteState { stream_id, granted }),
    }
}

/// Een bron die wij delen.
#[derive(Debug, Clone)]
pub struct EigenStream {
    pub id: u32,
    pub kind: StreamKind,
    pub titel: String,
    pub breedte: u32,
    pub hoogte: u32,
    /// Wie er kijkt en waar zijn beeld heen moet.
    pub kijkers: BTreeMap<PeerId, SocketAddr>,
    /// Wat de kijkers op dit scherm tekenden. `aan` is de schakelaar van de deler; voor
    /// alles wat geen scherm is staat hij altijd uit.
    pub tekening: Arc<Tekening>,
    /// Wie dit scherm mag bedienen, wie het vraagt en wie het doet.
    pub bediening: EigenBediening,
}

impl EigenStream {
    pub fn wordt_bekeken(&self) -> bool {
        !self.kijkers.is_empty()
    }

    /// Waar het beeld heen moet, met wie het is: elk pakket gaat versleuteld voor die peer.
    pub fn doelen(&self) -> Vec<PeerAdres> {
        self.kijkers
            .iter()
            .map(|(&id, &addr)| PeerAdres { id, addr })
            .collect()
    }

    fn aankondiging(&self) -> ControlMsg {
        ControlMsg::StreamAnnounce(StreamAnnounce {
            stream_id: self.id,
            kind: self.kind,
            title: self.titel.clone(),
            width: self.breedte,
            height: self.hoogte,
            drawing: self.tekening.aan,
            remote_control: self.bediening.mag,
        })
    }

    /// Het bedienen van dit scherm is voorbij, om welke reden ook: wie vroeg hoort nee, wie
    /// bediende hoort dat het over is. `vertrokken` is een peer die niet meer kijkt of weg is;
    /// die krijgt geen bericht.
    fn eindig_bediening(&mut self, vertrokken: Option<PeerId>) -> (Vec<MeshCommand>, Vec<Actie>) {
        let (mut cmds, mut acties) = (Vec::new(), Vec::new());
        if let Some((vrager, _)) = self.bediening.vrager.take() {
            if Some(vrager) != vertrokken {
                cmds.push(bedienstand_naar(vrager, self.id, false));
            }
        }
        if let Some(bediener) = self.bediening.bediener.take() {
            tracing::info!(peer = ?bediener, stream = self.id, "bedienen gestopt");
            if Some(bediener) != vertrokken {
                cmds.push(bedienstand_naar(bediener, self.id, false));
            }
            acties.push(Actie::StopBediening { stream_id: self.id });
        }
        (cmds, acties)
    }

    /// Een kijker vertrekt (uitgetekend of weg): was hij de vrager of de bediener, dan is dat
    /// voorbij. Een vraag of het bedienen van een ander blijft staan.
    fn vergeet_bediener(&mut self, kijker: PeerId) -> Vec<Actie> {
        let b = &mut self.bediening;
        if b.vrager.is_some_and(|(v, _)| v == kijker) {
            b.vrager = None;
        }
        if b.bediener == Some(kijker) {
            tracing::info!(peer = ?kijker, stream = self.id, "bediener weg; bedienen gestopt");
            b.bediener = None;
            return vec![Actie::StopBediening { stream_id: self.id }];
        }
        Vec::new()
    }

    /// De hele tekening naar elke kijker: na wissen, ongedaan maken of een vertrokken
    /// tekenaar. Vervangt bij hen wat er stond.
    fn toestand_naar_kijkers(&self) -> Vec<MeshCommand> {
        let toestand = self.tekening.als_toestand(self.id);
        self.kijkers
            .keys()
            .map(|k| MeshCommand::Send {
                to: *k,
                msg: ControlMsg::DrawState(toestand.clone()),
            })
            .collect()
    }

    /// Haalt de lijnen van een vertrokken kijker weg. Zijn lijnen leven zolang hij kijkt:
    /// daarna kan hij ze zelf niet meer ongedaan maken.
    fn vergeet_tekenaar(&mut self, kijker: PeerId) -> (Vec<MeshCommand>, Vec<Actie>) {
        if !Arc::make_mut(&mut self.tekening).wis_van(kijker) {
            return (Vec::new(), Vec::new());
        }
        (
            self.toestand_naar_kijkers(),
            vec![Actie::Tekening {
                eigenaar: None,
                stream_id: self.id,
            }],
        )
    }
}

/// Een bron die een ander deelt.
#[derive(Debug, Clone)]
pub struct VreemdeStream {
    pub eigenaar: PeerId,
    pub id: u32,
    pub kind: StreamKind,
    pub titel: String,
    pub breedte: u32,
    pub hoogte: u32,
    /// Kijken wij ernaar?
    pub kijken: bool,
    /// Onze kopie van de tekening van de deler. Alleen gevuld terwijl we kijken; `aan` is
    /// wat de deler aankondigde.
    pub tekening: Arc<Tekening>,
    /// De poort waarop we ingetekend zijn, zolang we kijken: opnieuw intekenen op dezelfde
    /// poort is hoe een kopie met een gat de hele tekening terugkrijgt.
    pub poort: Option<u16>,
    /// Wanneer we dat voor het laatst deden; zie [`HERSYNC_PAUZE`].
    pub hersync: Option<Instant>,
    /// Of de deler bedienen toestaat en waar wij staan (beslissing 54).
    pub bediening: Bedienstand,
    /// De rem op klembordtekst van de deler zolang wij hem bedienen.
    klembord: Emmer,
}

impl VreemdeStream {
    /// Onze kopie mist iets: opnieuw intekenen op dezelfde poort. De deler werkt daarop
    /// alleen zijn kijkerslijst bij met hetzelfde adres en stuurt de hele tekening.
    fn hersync(&mut self) -> Vec<MeshCommand> {
        let Some(poort) = self.poort else {
            return Vec::new();
        };
        if self.hersync.is_some_and(|t| t.elapsed() < HERSYNC_PAUZE) {
            return Vec::new();
        }
        self.hersync = Some(Instant::now());
        tracing::info!(peer = ?self.eigenaar, stream = self.id, "gat in de tekening; alles opnieuw gevraagd");
        vec![MeshCommand::Send {
            to: self.eigenaar,
            msg: ControlMsg::StreamSubscribe(StreamSubscribe {
                stream_id: self.id,
                media_port: poort,
            }),
        }]
    }
}

#[derive(Debug, Default)]
pub struct Streams {
    /// Loopt alleen op. Een id hergebruiken terwijl er nog pakketten van de vorige
    /// stream onderweg zijn levert beeld op in het verkeerde venster.
    volgende_id: u32,
    eigen: Vec<EigenStream>,
    vreemd: Vec<VreemdeStream>,
}

impl Streams {
    pub fn new() -> Self {
        Self {
            volgende_id: 1,
            ..Default::default()
        }
    }

    pub fn eigen(&self) -> &[EigenStream] {
        &self.eigen
    }

    pub fn vreemd(&self) -> &[VreemdeStream] {
        &self.vreemd
    }

    // -- delen -------------------------------------------------------------

    /// Kondigt een bron aan. Levert het toegekende id op plus de melding aan de
    /// anderen. Er wordt nog niets opgenomen: dat begint pas als iemand intekent.
    ///
    /// Tekenen kan alleen op een scherm, bedienen alleen op een monitor (beslissing 54: een
    /// klik op een gedeeld venster valt op wat er toevallig bovenop ligt).
    pub fn deel(
        &mut self,
        kind: StreamKind,
        titel: String,
        breedte: u32,
        hoogte: u32,
        opties: DeelOpties,
    ) -> (u32, Vec<MeshCommand>) {
        let id = self.volgende_id.max(1);
        self.volgende_id = id + 1;

        let stream = EigenStream {
            id,
            kind,
            titel,
            breedte,
            hoogte,
            kijkers: BTreeMap::new(),
            tekening: Arc::new(Tekening::nieuw(kind.is_scherm() && opties.tekenen)),
            bediening: EigenBediening::nieuw(kind == StreamKind::MONITOR && opties.bedienen),
        };
        let melding = stream.aankondiging();
        self.eigen.push(stream);
        (id, vec![MeshCommand::Broadcast(melding)])
    }

    /// Nieuwe titel voor een eigen stream — bij muziek het nummer dat nu speelt. Een
    /// herhaalde aankondiging is voor de ontvanger al "gegevens bijwerken" (zie
    /// `aangekondigd`), dus er is geen eigen bericht voor nodig. Niets als er niets
    /// verandert: dit wordt elke tik gevraagd.
    pub fn zet_titel(&mut self, stream_id: u32, titel: &str) -> Vec<MeshCommand> {
        match self.eigen.iter_mut().find(|s| s.id == stream_id) {
            Some(s) if s.titel != titel => {
                s.titel = titel.to_string();
                vec![MeshCommand::Broadcast(s.aankondiging())]
            }
            _ => Vec::new(),
        }
    }

    pub fn stop_delen(&mut self, stream_id: u32) -> (Vec<MeshCommand>, Vec<Actie>) {
        let Some(plek) = self.eigen.iter().position(|s| s.id == stream_id) else {
            return (Vec::new(), Vec::new());
        };
        let stream = self.eigen.remove(plek);

        let mut acties = Vec::new();
        if stream.wordt_bekeken() {
            acties.push(Actie::StopDelen {
                stream_id,
                is_geluid: stream.kind.is_geluid(),
            });
        }
        // De bediener kijkt naar een stream die verdwijnt; zijn venster gaat vanzelf dicht.
        if stream.bediening.bediener.is_some() {
            acties.push(Actie::StopBediening { stream_id });
        }
        // De laag over ons scherm moet weg; de motor vindt de stream niet meer.
        if !stream.tekening.is_leeg() {
            acties.push(Actie::Tekening {
                eigenaar: None,
                stream_id,
            });
        }
        (
            vec![MeshCommand::Broadcast(ControlMsg::StreamRevoke(
                StreamRevoke { stream_id },
            ))],
            acties,
        )
    }

    /// Mogen kijkers op dit eigen scherm tekenen (de optie per gedeeld scherm). Uit wist alles
    /// en zegt het iedereen via een nieuwe aankondiging, net als een nieuwe titel.
    pub fn zet_tekenen(&mut self, stream_id: u32, aan: bool) -> (Vec<MeshCommand>, Vec<Actie>) {
        let Some(s) = self
            .eigen
            .iter_mut()
            .find(|s| s.id == stream_id && s.kind.is_scherm())
        else {
            return (Vec::new(), Vec::new());
        };
        if !Arc::make_mut(&mut s.tekening).zet_aan(aan) {
            return (Vec::new(), Vec::new());
        }
        (
            vec![MeshCommand::Broadcast(s.aankondiging())],
            vec![Actie::Tekening {
                eigenaar: None,
                stream_id,
            }],
        )
    }

    /// Mogen kijkers vragen dit eigen scherm te bedienen. Alleen een monitor. Uit maakt een
    /// eind aan een vraag en aan het bedienen, en zegt het iedereen via een nieuwe aankondiging.
    pub fn zet_bedienen(&mut self, stream_id: u32, aan: bool) -> (Vec<MeshCommand>, Vec<Actie>) {
        let Some(s) = self
            .eigen
            .iter_mut()
            .find(|s| s.id == stream_id && s.kind == StreamKind::MONITOR)
        else {
            return (Vec::new(), Vec::new());
        };
        if s.bediening.mag == aan {
            return (Vec::new(), Vec::new());
        }
        s.bediening.mag = aan;
        let (mut cmds, acties) = s.eindig_bediening(None);
        cmds.push(MeshCommand::Broadcast(s.aankondiging()));
        (cmds, acties)
    }

    /// De deler klikt Allow: `peer` mag dit scherm bedienen. Alleen als hij nog de vrager is,
    /// nog kijkt en de optie nog aan staat; anders is de klik verouderd en gebeurt er niets.
    pub fn sta_bediening_toe(
        &mut self,
        stream_id: u32,
        peer: PeerId,
    ) -> (Vec<MeshCommand>, Vec<Actie>) {
        let Some(s) = self.eigen.iter_mut().find(|s| s.id == stream_id) else {
            return (Vec::new(), Vec::new());
        };
        let geldig = s.bediening.mag
            && s.bediening.vrager.is_some_and(|(v, _)| v == peer)
            && s.kijkers.contains_key(&peer);
        if !geldig {
            tracing::debug!(peer = ?peer, stream = stream_id, "toestemming voor een vraag die er niet meer is");
            return (Vec::new(), Vec::new());
        }
        tracing::info!(peer = ?peer, stream = stream_id, "bedienen toegestaan");
        let nu = Instant::now();
        let b = &mut s.bediening;
        b.vrager = None;
        b.bediener = Some(peer);
        b.invoer = Emmer::nieuw(INVOER_VOORRAAD, INVOER_PER_SECONDE, nu);
        b.klembord = Emmer::nieuw(KLEMBORD_VOORRAAD, KLEMBORD_PER_SECONDE, nu);
        (
            vec![bedienstand_naar(peer, stream_id, true)],
            vec![Actie::StartBediening { stream_id }],
        )
    }

    /// De deler klikt Deny.
    pub fn weiger_bediening(&mut self, stream_id: u32) -> Vec<MeshCommand> {
        let Some(s) = self.eigen.iter_mut().find(|s| s.id == stream_id) else {
            return Vec::new();
        };
        match s.bediening.vrager.take() {
            Some((vrager, _)) => vec![bedienstand_naar(vrager, stream_id, false)],
            None => Vec::new(),
        }
    }

    /// De deler trekt het bedienen in, en een vraag die er nog lag.
    pub fn stop_bediening(&mut self, stream_id: u32) -> (Vec<MeshCommand>, Vec<Actie>) {
        match self.eigen.iter_mut().find(|s| s.id == stream_id) {
            Some(s) => s.eindig_bediening(None),
            None => (Vec::new(), Vec::new()),
        }
    }

    /// Vragen waar na [`VRAAG_GELDIG`] nog niemand op klikte worden nee. Op de tik van de motor.
    pub fn verlopen_vragen(&mut self, nu: Instant) -> Vec<MeshCommand> {
        let mut cmds = Vec::new();
        for s in &mut self.eigen {
            if let Some((vrager, sinds)) = s.bediening.vrager {
                if nu.saturating_duration_since(sinds) >= VRAAG_GELDIG {
                    tracing::info!(peer = ?vrager, stream = s.id, "vraag om te bedienen verlopen");
                    s.bediening.vrager = None;
                    cmds.push(bedienstand_naar(vrager, s.id, false));
                }
            }
        }
        cmds
    }

    /// Het klembord van de deler veranderde terwijl dit scherm bediend wordt: naar de bediener,
    /// en naar niemand anders.
    pub fn klembord_van_deler(&self, stream_id: u32, tekst: &str) -> Vec<MeshCommand> {
        let bediener = self
            .eigen
            .iter()
            .find(|s| s.id == stream_id && s.bediening.mag)
            .and_then(|s| s.bediening.bediener);
        match (bediener, schoon_klembord(tekst)) {
            (Some(to), Some(text)) => vec![MeshCommand::Send {
                to,
                msg: ControlMsg::ClipboardText(ClipboardText { stream_id, text }),
            }],
            _ => Vec::new(),
        }
    }

    /// Onze stand voor een stream van een ander, voor zijn kijkvenster.
    pub fn bedienstand(&self, eigenaar: PeerId, stream_id: u32) -> Bedienstand {
        self.vreemd
            .iter()
            .find(|s| s.eigenaar == eigenaar && s.id == stream_id)
            .map_or(Bedienstand::Uit, |s| s.bediening)
    }

    /// De deler wist alles op zijn scherm, van iedereen.
    pub fn wis_tekening(&mut self, stream_id: u32) -> (Vec<MeshCommand>, Vec<Actie>) {
        let Some(s) = self.eigen.iter_mut().find(|s| s.id == stream_id) else {
            return (Vec::new(), Vec::new());
        };
        if !Arc::make_mut(&mut s.tekening).wis() {
            return (Vec::new(), Vec::new());
        }
        (
            s.toestand_naar_kijkers(),
            vec![Actie::Tekening {
                eigenaar: None,
                stream_id,
            }],
        )
    }

    /// De tekening op een scherm: `None` voor een eigen stream.
    pub fn tekening(&self, eigenaar: Option<PeerId>, stream_id: u32) -> Option<Arc<Tekening>> {
        match eigenaar {
            None => self
                .eigen
                .iter()
                .find(|s| s.id == stream_id)
                .map(|s| s.tekening.clone()),
            Some(e) => self
                .vreemd
                .iter()
                .find(|s| s.eigenaar == e && s.id == stream_id)
                .map(|s| s.tekening.clone()),
        }
    }

    // -- kijken ------------------------------------------------------------

    /// De gebruiker wil naar een stream van een ander kijken. Het venster moet er zijn
    /// vóór we intekenen, want in het abonnement staat de poort waarop het luistert.
    pub fn wil_kijken(&mut self, eigenaar: PeerId, stream_id: u32) -> Vec<Actie> {
        let Some(s) = self.vind_vreemd(eigenaar, stream_id) else {
            return Vec::new();
        };
        if s.kijken {
            return Vec::new();
        }
        s.kijken = true;
        vec![Actie::StartKijken {
            eigenaar,
            stream_id,
            titel: s.titel.clone(),
            breedte: s.breedte,
            hoogte: s.hoogte,
            is_geluid: s.kind.is_geluid(),
        }]
    }

    /// Bureaubladgeluid volgt het beeld: kijken naar het scherm van een peer betekent
    /// zijn geluid horen, en het laatste venster van die peer sluiten zet het weer uit.
    ///
    /// Deze regel stond tot fase 12 in de egui-UI (`stuur_kijk_toggle`) en verdween bij
    /// de wissel naar Tauri, waardoor niemand nog op een `DESKTOP_AUDIO`-stream intekende
    /// en er dus geen geluid meer bij een gedeeld scherm kwam. Hier hoort hij: nu geldt
    /// hij ook voor een aankondiging die later binnenkomt, een venster dat zichzelf sluit
    /// en een peer die wegvalt.
    ///
    /// `in_gesprek` is `false` zonder voice-sessie — dan is er geen socket om het op te
    /// ontvangen. Idempotent: tweemaal aanroepen levert de tweede keer niets op.
    ///
    /// Alleen een gedeeld *scherm* trekt het geluid mee. Naar iemands camera kijken doet
    /// dat niet: daar hoort geen systeemgeluid bij, en de deler zet er ook geen
    /// `DESKTOP_AUDIO`-stream naast als hij alleen zijn camera aan heeft.
    pub fn stem_geluid_af_op_beeld(&mut self, in_gesprek: bool) -> (Vec<MeshCommand>, Vec<Actie>) {
        let met_beeld: Vec<PeerId> = self
            .vreemd
            .iter()
            .filter(|s| s.kijken && s.kind.is_scherm())
            .map(|s| s.eigenaar)
            .collect();

        // Muziek hangt niet aan beeld: wie in het gesprek zit hoort hem, net als een stem.
        let wissels: Vec<(PeerId, u32, bool)> = self
            .vreemd
            .iter()
            .filter(|s| s.kind.is_geluid())
            .filter_map(|s| {
                let hoort =
                    in_gesprek && (s.kind == StreamKind::MUSIC || met_beeld.contains(&s.eigenaar));
                (hoort != s.kijken).then_some((s.eigenaar, s.id, hoort))
            })
            .collect();

        let mut cmds = Vec::new();
        let mut acties = Vec::new();
        for (eigenaar, id, aan) in wissels {
            if aan {
                acties.extend(self.wil_kijken(eigenaar, id));
            } else {
                let (c, a) = self.stop_kijken(eigenaar, id);
                cmds.extend(c);
                acties.extend(a);
            }
        }
        (cmds, acties)
    }

    /// Het venster staat en luistert op `poort`; nu pas mag de deler beginnen.
    pub fn kijker_draait(
        &mut self,
        eigenaar: PeerId,
        stream_id: u32,
        poort: u16,
    ) -> Vec<MeshCommand> {
        if let Some(s) = self.vind_vreemd(eigenaar, stream_id) {
            s.poort = Some(poort);
        }
        vec![MeshCommand::Send {
            to: eigenaar,
            msg: ControlMsg::StreamSubscribe(StreamSubscribe {
                stream_id,
                media_port: poort,
            }),
        }]
    }

    pub fn stop_kijken(
        &mut self,
        eigenaar: PeerId,
        stream_id: u32,
    ) -> (Vec<MeshCommand>, Vec<Actie>) {
        let Some(s) = self.vind_vreemd(eigenaar, stream_id) else {
            return (Vec::new(), Vec::new());
        };
        if !s.kijken {
            return (Vec::new(), Vec::new());
        }
        s.kijken = false;
        // Zonder venster komt er niets meer binnen; bij opnieuw kijken stuurt de deler alles.
        s.tekening = Arc::new(Tekening::nieuw(s.tekening.aan));
        s.poort = None;
        // Wie niet kijkt bedient niet; de deler stopt het zelf zodra hij ons uittekenen ziet.
        if s.bediening != Bedienstand::Uit {
            s.bediening = Bedienstand::Mag;
        }
        (
            vec![MeshCommand::Send {
                to: eigenaar,
                msg: ControlMsg::StreamUnsubscribe(StreamUnsubscribe { stream_id }),
            }],
            vec![Actie::StopKijken {
                eigenaar,
                stream_id,
            }],
        )
    }

    /// De kijker is de draad kwijt en heeft een nieuw keyframe nodig.
    pub fn vraag_keyframe(&self, eigenaar: PeerId, stream_id: u32) -> Vec<MeshCommand> {
        vec![MeshCommand::Send {
            to: eigenaar,
            msg: ControlMsg::RequestKeyframe(RequestKeyframe { stream_id }),
        }]
    }

    /// Wij tekenen in het kijkvenster van een stream van `eigenaar`: een lijn (zonder
    /// tekenaar) of een wisverzoek, naar de deler en verder niets. We passen niets zelf toe:
    /// onze lijn komt terug zoals de deler hem doorgeeft, net als bij de anderen.
    pub fn kijker_tekent(
        &self,
        eigenaar: PeerId,
        stream_id: u32,
        msg: ControlMsg,
    ) -> Vec<MeshCommand> {
        let mag = self
            .vreemd
            .iter()
            .any(|s| s.eigenaar == eigenaar && s.id == stream_id && s.kijken && s.tekening.aan);
        let past = match &msg {
            ControlMsg::Draw(d) => d.stream_id == stream_id && d.author.is_none(),
            ControlMsg::DrawClear(c) => c.stream_id == stream_id,
            _ => false,
        };
        if mag && past {
            vec![MeshCommand::Send { to: eigenaar, msg }]
        } else {
            Vec::new()
        }
    }

    /// Wij bedienen (of vragen het) vanuit het kijkvenster van een stream van `eigenaar`: een
    /// vraag, stoppen, invoer of klembord, naar de deler en verder niets. Een vraag zet onze
    /// stand meteen om, en het venster hoort altijd hoe hij nu staat, ook als er niets weg mocht.
    pub fn kijker_bedient(
        &mut self,
        eigenaar: PeerId,
        stream_id: u32,
        msg: ControlMsg,
    ) -> (Vec<MeshCommand>, Vec<Actie>) {
        let Some(s) = self.vind_kijkend(eigenaar, stream_id) else {
            return (Vec::new(), Vec::new());
        };
        let stuur = |msg| vec![MeshCommand::Send { to: eigenaar, msg }];
        let melden = vec![Actie::Bediening {
            eigenaar,
            stream_id,
        }];
        match &msg {
            ControlMsg::RemoteRequest(r) if r.stream_id == stream_id => {
                let cmds = match (r.want, s.bediening) {
                    (true, Bedienstand::Mag) => {
                        s.bediening = Bedienstand::Gevraagd;
                        stuur(msg)
                    }
                    (false, Bedienstand::Gevraagd | Bedienstand::Actief) => {
                        s.bediening = Bedienstand::Mag;
                        stuur(msg)
                    }
                    _ => Vec::new(),
                };
                (cmds, melden)
            }
            ControlMsg::RemoteInput(i)
                if i.stream_id == stream_id && s.bediening == Bedienstand::Actief =>
            {
                (stuur(msg), Vec::new())
            }
            ControlMsg::ClipboardText(c)
                if c.stream_id == stream_id && s.bediening == Bedienstand::Actief =>
            {
                match schoon_klembord(&c.text) {
                    Some(text) => (
                        stuur(ControlMsg::ClipboardText(ClipboardText { stream_id, text })),
                        Vec::new(),
                    ),
                    None => (Vec::new(), Vec::new()),
                }
            }
            _ => (Vec::new(), Vec::new()),
        }
    }

    // -- gebeurtenissen van de mesh ---------------------------------------

    /// Een peer is (opnieuw) verbonden. Hij heeft onze eerdere aankondigingen gemist,
    /// dus die sturen we alsnog — anders ziet hij nooit dat we iets delen.
    pub fn bij_verbinding(&self, peer: PeerId) -> Vec<MeshCommand> {
        self.eigen
            .iter()
            .map(|s| MeshCommand::Send {
                to: peer,
                msg: s.aankondiging(),
            })
            .collect()
    }

    /// Een peer is weg. Wat hij deelde bestaat voor ons niet meer, waar hij naar keek
    /// hoeven we niet meer te versturen, en wat hij op onze schermen tekende gaat weg.
    pub fn bij_verbreking(&mut self, peer: PeerId) -> (Vec<MeshCommand>, Vec<Actie>) {
        let mut cmds = Vec::new();
        let mut acties = Vec::new();

        for s in self
            .vreemd
            .iter()
            .filter(|s| s.eigenaar == peer && s.kijken)
        {
            acties.push(Actie::StopKijken {
                eigenaar: peer,
                stream_id: s.id,
            });
        }
        self.vreemd.retain(|s| s.eigenaar != peer);

        for s in &mut self.eigen {
            if s.kijkers.remove(&peer).is_some() {
                acties.push(if s.wordt_bekeken() {
                    Actie::ZetKijkers {
                        stream_id: s.id,
                        kijkers: s.doelen(),
                    }
                } else {
                    Actie::StopDelen {
                        stream_id: s.id,
                        is_geluid: s.kind.is_geluid(),
                    }
                });
                let (c, a) = s.vergeet_tekenaar(peer);
                cmds.extend(c);
                acties.extend(a);
                acties.extend(s.vergeet_bediener(peer));
            }
        }

        (cmds, acties)
    }

    /// Een control-bericht dat over screenshare gaat. Alles wat er niet over gaat komt
    /// er ongemoeid weer uit als een leeg antwoord.
    pub fn bij_bericht(
        &mut self,
        van: PeerId,
        van_ip: IpAddr,
        msg: &ControlMsg,
    ) -> (Vec<MeshCommand>, Vec<Actie>) {
        match msg {
            ControlMsg::StreamAnnounce(a) => (Vec::new(), self.aangekondigd(van, a)),
            ControlMsg::StreamRevoke(r) => (Vec::new(), self.ingetrokken(van, r.stream_id)),
            ControlMsg::StreamSubscribe(s) => {
                self.ingetekend(van, SocketAddr::new(van_ip, s.media_port), s.stream_id)
            }
            ControlMsg::StreamUnsubscribe(u) => self.uitgetekend(van, u.stream_id),
            ControlMsg::RequestKeyframe(k) => {
                tracing::debug!(peer = ?van, stream = k.stream_id, "keyframe gevraagd door kijker");
                (Vec::new(), self.keyframe_gevraagd(van, k.stream_id))
            }
            ControlMsg::Draw(d) => match d.author {
                None => self.kijker_tekende(van, d),
                Some(tekenaar) => self.deler_gaf_door(van, tekenaar, d),
            },
            ControlMsg::DrawClear(c) => self.kijker_wist(van, c),
            ControlMsg::DrawState(t) => (Vec::new(), self.deler_gaf_alles(van, t)),
            ControlMsg::RemoteRequest(r) => self.bedienvraag(van, r, Instant::now()),
            ControlMsg::RemoteState(r) => (Vec::new(), self.bedienantwoord(van, r)),
            ControlMsg::RemoteInput(i) => (Vec::new(), self.invoer_van(van, i, Instant::now())),
            ControlMsg::ClipboardText(c) => (Vec::new(), self.klembord_van(van, c, Instant::now())),
            _ => (Vec::new(), Vec::new()),
        }
    }

    /// Als deler: een kijker vraagt dit scherm te bedienen, of stopt ermee. Eén vrager en één
    /// bediener tegelijk; wie er niet aan de beurt is hoort meteen nee.
    fn bedienvraag(
        &mut self,
        van: PeerId,
        r: &RemoteRequest,
        nu: Instant,
    ) -> (Vec<MeshCommand>, Vec<Actie>) {
        let Some(s) = self
            .eigen
            .iter_mut()
            .find(|s| s.id == r.stream_id && s.kijkers.contains_key(&van))
        else {
            tracing::debug!(peer = ?van, stream = r.stream_id, "vraag om te bedienen van wie niet kijkt");
            return (Vec::new(), Vec::new());
        };
        let nee = || vec![bedienstand_naar(van, r.stream_id, false)];
        let b = &mut s.bediening;
        if !r.want {
            return (Vec::new(), s.vergeet_bediener(van));
        }
        if b.bediener == Some(van) {
            // Al bediener: zeg het nog eens (zijn venster ging misschien opnieuw open).
            return (vec![bedienstand_naar(van, r.stream_id, true)], Vec::new());
        }
        if b.vrager.is_some_and(|(v, _)| v == van) {
            return (Vec::new(), Vec::new());
        }
        if !b.mag || b.bediener.is_some() || b.vrager.is_some() || !b.vragen.neem(nu) {
            tracing::debug!(peer = ?van, stream = r.stream_id, "vraag om te bedienen afgewezen");
            return (nee(), Vec::new());
        }
        tracing::info!(peer = ?van, stream = r.stream_id, "vraag om te bedienen");
        b.vrager = Some((van, nu));
        (
            Vec::new(),
            vec![Actie::BedienVraag {
                stream_id: r.stream_id,
                van,
            }],
        )
    }

    /// Als kijker: de deler zegt of we bedienen. Alleen van de eigenaar van een stream waar we
    /// naar kijken, en "ja" alleen als we vroegen.
    fn bedienantwoord(&mut self, van: PeerId, r: &RemoteState) -> Vec<Actie> {
        let Some(s) = self.vind_kijkend(van, r.stream_id) else {
            return Vec::new();
        };
        let nieuw = match (r.granted, s.bediening) {
            (true, Bedienstand::Gevraagd) => Bedienstand::Actief,
            (false, Bedienstand::Gevraagd | Bedienstand::Actief) => Bedienstand::Mag,
            _ => return Vec::new(),
        };
        tracing::info!(peer = ?van, stream = r.stream_id, bedienen = r.granted, "antwoord van de deler");
        s.bediening = nieuw;
        vec![Actie::Bediening {
            eigenaar: van,
            stream_id: r.stream_id,
        }]
    }

    /// Als deler: invoer. Alleen van de bediener, terwijl hij kijkt en het mag; geldig, binnen
    /// de grens per bericht en afgeremd, behalve het loslaten.
    fn invoer_van(&mut self, van: PeerId, i: &RemoteInput, nu: Instant) -> Vec<Actie> {
        let Some(s) = self.eigen.iter_mut().find(|s| {
            s.id == i.stream_id
                && s.bediening.mag
                && s.bediening.bediener == Some(van)
                && s.kijkers.contains_key(&van)
        }) else {
            tracing::debug!(peer = ?van, stream = i.stream_id, "invoer van wie niet bedient genegeerd");
            return Vec::new();
        };
        let rem = &mut s.bediening.invoer;
        let events: Vec<InputEvent> = begrens(&i.events)
            .into_iter()
            .filter(|e| e.is_loslaten() || rem.neem(nu))
            .collect();
        if events.is_empty() {
            return Vec::new();
        }
        vec![Actie::Invoer {
            stream_id: i.stream_id,
            events,
        }]
    }

    /// Klembordtekst. Eerst als deler (de afzender bedient ons scherm), anders als bediener (de
    /// afzender is de deler van een scherm dat wij bedienen). Beide zetten het klembord van deze
    /// PC; een rem per kant.
    fn klembord_van(&mut self, van: PeerId, c: &ClipboardText, nu: Instant) -> Vec<Actie> {
        let Some(tekst) = schoon_klembord(&c.text) else {
            tracing::debug!(peer = ?van, len = c.text.len(), "klembordtekst leeg of te groot");
            return Vec::new();
        };
        if let Some(s) = self.eigen.iter_mut().find(|s| {
            s.id == c.stream_id
                && s.bediening.mag
                && s.bediening.bediener == Some(van)
                && s.kijkers.contains_key(&van)
        }) {
            return if s.bediening.klembord.neem(nu) {
                vec![Actie::Klembord {
                    eigenaar: None,
                    stream_id: c.stream_id,
                    tekst,
                }]
            } else {
                Vec::new()
            };
        }
        let Some(s) = self.vind_kijkend(van, c.stream_id) else {
            return Vec::new();
        };
        if s.bediening != Bedienstand::Actief || !s.klembord.neem(nu) {
            return Vec::new();
        }
        vec![Actie::Klembord {
            eigenaar: Some(van),
            stream_id: c.stream_id,
            tekst,
        }]
    }

    /// Als deler: een kijker tekent op ons scherm. Alleen wie nu kijkt, alleen als het
    /// mag; de tekenaar is de afzender van de verbinding. Daarna in dezelfde volgorde door
    /// naar al onze kijkers, ook naar hem.
    fn kijker_tekende(&mut self, van: PeerId, d: &Draw) -> (Vec<MeshCommand>, Vec<Actie>) {
        let Some(s) = self
            .eigen
            .iter_mut()
            .find(|s| s.id == d.stream_id && s.kijkers.contains_key(&van))
        else {
            tracing::debug!(peer = ?van, stream = d.stream_id, "lijn van wie niet kijkt genegeerd");
            return (Vec::new(), Vec::new());
        };
        if !s.tekening.aan || !Arc::make_mut(&mut s.tekening).teken(van, d) {
            tracing::debug!(peer = ?van, stream = d.stream_id, "lijn van een kijker niet aangenomen");
            return (Vec::new(), Vec::new());
        }
        let door = Draw {
            author: Some(van),
            ..d.clone()
        };
        let cmds = s
            .kijkers
            .keys()
            .map(|k| MeshCommand::Send {
                to: *k,
                msg: ControlMsg::Draw(door.clone()),
            })
            .collect();
        (
            cmds,
            vec![Actie::Tekening {
                eigenaar: None,
                stream_id: s.id,
            }],
        )
    }

    /// Als deler: een kijker wist zijn laatste lijn of al zijn lijnen. Daarna krijgt
    /// iedereen de hele tekening.
    fn kijker_wist(&mut self, van: PeerId, c: &DrawClear) -> (Vec<MeshCommand>, Vec<Actie>) {
        let Some(s) = self
            .eigen
            .iter_mut()
            .find(|s| s.id == c.stream_id && s.kijkers.contains_key(&van))
        else {
            return (Vec::new(), Vec::new());
        };
        let t = Arc::make_mut(&mut s.tekening);
        let veranderd = match c.scope {
            DrawClearScope::LAST => t.ongedaan(van),
            DrawClearScope::MINE => t.wis_van(van),
            onbekend => {
                tracing::debug!(peer = ?van, scope = ?onbekend, "onbekende wissoort genegeerd");
                false
            }
        };
        if !veranderd {
            return (Vec::new(), Vec::new());
        }
        (
            s.toestand_naar_kijkers(),
            vec![Actie::Tekening {
                eigenaar: None,
                stream_id: s.id,
            }],
        )
    }

    /// Als kijker: de deler geeft een lijn door. Alleen van de eigenaar van een stream waar
    /// we naar kijken; wie er tekende zegt de deler.
    fn deler_gaf_door(
        &mut self,
        van: PeerId,
        tekenaar: PeerId,
        d: &Draw,
    ) -> (Vec<MeshCommand>, Vec<Actie>) {
        let Some(s) = self.vind_kijkend(van, d.stream_id) else {
            return (Vec::new(), Vec::new());
        };
        if s.tekening.heeft_gat(tekenaar, d) {
            return (s.hersync(), Vec::new());
        }
        if !Arc::make_mut(&mut s.tekening).teken(tekenaar, d) {
            tracing::debug!(peer = ?van, stream = d.stream_id, "lijn van de deler niet aangenomen");
            return (Vec::new(), Vec::new());
        }
        (
            Vec::new(),
            vec![Actie::Tekening {
                eigenaar: Some(van),
                stream_id: d.stream_id,
            }],
        )
    }

    /// Als kijker: de deler stuurt de hele tekening. Vervangt de onze, door dezelfde grenzen.
    fn deler_gaf_alles(&mut self, van: PeerId, t: &DrawState) -> Vec<Actie> {
        let Some(s) = self.vind_kijkend(van, t.stream_id) else {
            return Vec::new();
        };
        let nieuw = Tekening::uit_toestand(s.tekening.aan, t);
        if *s.tekening == nieuw {
            return Vec::new();
        }
        s.tekening = Arc::new(nieuw);
        vec![Actie::Tekening {
            eigenaar: Some(van),
            stream_id: t.stream_id,
        }]
    }

    fn aangekondigd(&mut self, van: PeerId, a: &StreamAnnounce) -> Vec<Actie> {
        if a.stream_id == VOICE_STREAM_ID {
            tracing::warn!(peer = ?van, "aankondiging op de voice-stream genegeerd");
            return Vec::new();
        }
        if !a.kind.is_known() {
            // Van een nieuwere peer. We weten niet wat het is, dus we kunnen er niets
            // zinnigs mee; loggen en verder gaan, niet als fout behandelen.
            tracing::info!(peer = ?van, kind = ?a.kind, "onbekende soort stream genegeerd");
            return Vec::new();
        }
        let tekenen = a.drawing && a.kind.is_scherm();
        // Bedienen alleen op een monitor, wat de deler ook zegt (beslissing 54).
        let bedienen = a.remote_control && a.kind == StreamKind::MONITOR;

        // Opnieuw aankondigen is normaal: het gebeurt bij elke herverbinding. Alleen
        // de gegevens bijwerken, de kijkstatus laten staan.
        if let Some(s) = self.vind_vreemd(van, a.stream_id) {
            // De deler zette bedienen aan of uit; uit maakt een eind aan vragen en bedienen.
            let mut acties = Vec::new();
            let stand = match (bedienen, s.bediening) {
                (true, Bedienstand::Uit) => Bedienstand::Mag,
                (false, _) => Bedienstand::Uit,
                (true, stand) => stand,
            };
            if stand != s.bediening {
                s.bediening = stand;
                if s.kijken {
                    acties.push(Actie::Bediening {
                        eigenaar: van,
                        stream_id: a.stream_id,
                    });
                }
            }
            s.kind = a.kind;
            // B-31: ook op dit pad afkappen — een herverbinding komt hier langs, niet
            // hierboven, en anders is de grens met één reconnect te omzeilen.
            s.titel = a
                .title
                .chars()
                .filter(|c| !c.is_control())
                .take(64)
                .collect();
            s.breedte = a.width;
            s.hoogte = a.height;
            // De deler zette tekenen aan of uit; uit wist ook onze kopie.
            if s.tekening.aan != tekenen {
                Arc::make_mut(&mut s.tekening).zet_aan(tekenen);
                acties.push(Actie::Tekening {
                    eigenaar: Some(van),
                    stream_id: a.stream_id,
                });
            }
            return acties;
        }

        // B-31: een plafond per peer. Zonder dit kan één peer onbeperkt streams
        // aankondigen — elk met een onbegrensde titel die opgeslagen, gelogd en getoond
        // wordt. Acht is ruim: je deelt je schermen en je camera, niet meer. Wie erboven
        // komt is niet iets aan het delen, maar iets aan het vullen.
        const MAX_STREAMS_PER_PEER: usize = 8;
        let van_deze_peer = self.vreemd.iter().filter(|s| s.eigenaar == van).count();
        if van_deze_peer >= MAX_STREAMS_PER_PEER {
            tracing::warn!(
                peer = ?van,
                aantal = van_deze_peer,
                "te veel streams van deze peer; aankondiging genegeerd"
            );
            return Vec::new();
        }

        // B-31/B-33: de titel komt onbegrensd van de draad en gaat het log en de UI in.
        // Afkappen op chars, niet op bytes: midden in een multibyte-teken knippen paniekt.
        const MAX_TITEL: usize = 64;
        let titel: String = a
            .title
            .chars()
            .filter(|c| !c.is_control())
            .take(MAX_TITEL)
            .collect();

        tracing::info!(peer = ?van, stream = a.stream_id, titel = ?titel, "peer deelt iets");
        self.vreemd.push(VreemdeStream {
            eigenaar: van,
            id: a.stream_id,
            kind: a.kind,
            titel,
            breedte: a.width,
            hoogte: a.height,
            kijken: false,
            tekening: Arc::new(Tekening::nieuw(tekenen)),
            poort: None,
            hersync: None,
            bediening: if bedienen {
                Bedienstand::Mag
            } else {
                Bedienstand::Uit
            },
            klembord: Emmer::nieuw(KLEMBORD_VOORRAAD, KLEMBORD_PER_SECONDE, Instant::now()),
        });
        Vec::new()
    }

    fn ingetrokken(&mut self, van: PeerId, stream_id: u32) -> Vec<Actie> {
        let keek = self
            .vind_vreemd(van, stream_id)
            .map(|s| s.kijken)
            .unwrap_or(false);
        self.vreemd
            .retain(|s| !(s.eigenaar == van && s.id == stream_id));

        if keek {
            vec![Actie::StopKijken {
                eigenaar: van,
                stream_id,
            }]
        } else {
            Vec::new()
        }
    }

    fn ingetekend(
        &mut self,
        van: PeerId,
        adres: SocketAddr,
        stream_id: u32,
    ) -> (Vec<MeshCommand>, Vec<Actie>) {
        let Some(s) = self.eigen.iter_mut().find(|s| s.id == stream_id) else {
            tracing::warn!(peer = ?van, stream = stream_id, "intekening op een stream die we niet delen");
            return (Vec::new(), Vec::new());
        };

        let eerste = !s.wordt_bekeken();
        s.kijkers.insert(van, adres);
        let kijkers = s.doelen();

        // Een nieuwe kijker ziet wat er al getekend staat, over dezelfde geordende verbinding
        // als alles wat hierna nog komt. Ook leeg: een kijker die opnieuw intekent omdat zijn
        // kopie een gat had, moet ook "er staat niets meer" kunnen horen.
        let cmds = if s.kind.is_scherm() {
            vec![MeshCommand::Send {
                to: van,
                msg: ControlMsg::DrawState(s.tekening.als_toestand(stream_id)),
            }]
        } else {
            Vec::new()
        };

        let actie = if eerste {
            tracing::info!(stream = stream_id, "eerste kijker; opnemen begint");
            Actie::StartDelen { stream_id, kijkers }
        } else {
            Actie::ZetKijkers { stream_id, kijkers }
        };
        (cmds, vec![actie])
    }

    fn uitgetekend(&mut self, van: PeerId, stream_id: u32) -> (Vec<MeshCommand>, Vec<Actie>) {
        let Some(s) = self.eigen.iter_mut().find(|s| s.id == stream_id) else {
            return (Vec::new(), Vec::new());
        };
        if s.kijkers.remove(&van).is_none() {
            return (Vec::new(), Vec::new());
        }

        let actie = if s.wordt_bekeken() {
            Actie::ZetKijkers {
                stream_id,
                kijkers: s.doelen(),
            }
        } else {
            tracing::info!(stream = stream_id, "laatste kijker weg; opnemen stopt");
            Actie::StopDelen {
                stream_id,
                is_geluid: s.kind.is_geluid(),
            }
        };
        let (cmds, mut acties) = s.vergeet_tekenaar(van);
        acties.insert(0, actie);
        acties.extend(s.vergeet_bediener(van));
        (cmds, acties)
    }

    fn keyframe_gevraagd(&self, van: PeerId, stream_id: u32) -> Vec<Actie> {
        // Alleen honoreren voor wie daadwerkelijk kijkt: anders kan iedereen die het
        // adres kent de bitrate laten ontploffen door keyframes te blijven vragen.
        let mag = self
            .eigen
            .iter()
            .any(|s| s.id == stream_id && s.kijkers.contains_key(&van));
        if mag {
            vec![Actie::StuurKeyframe { stream_id }]
        } else {
            Vec::new()
        }
    }

    fn vind_vreemd(&mut self, eigenaar: PeerId, stream_id: u32) -> Option<&mut VreemdeStream> {
        self.vreemd
            .iter_mut()
            .find(|s| s.eigenaar == eigenaar && s.id == stream_id)
    }

    /// Een stream van `eigenaar` waar we nu naar kijken: de enige waarvan we tekenwerk
    /// aannemen, en alleen van die eigenaar zelf.
    fn vind_kijkend(&mut self, eigenaar: PeerId, stream_id: u32) -> Option<&mut VreemdeStream> {
        self.vind_vreemd(eigenaar, stream_id).filter(|s| s.kijken)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wat de schermen in deze tests toestaan: tekenen, zoals vóór beslissing 54 standaard.
    const TEKENEN: DeelOpties = DeelOpties {
        tekenen: true,
        bedienen: false,
    };

    fn ip() -> IpAddr {
        IpAddr::from([100, 64, 0, 7])
    }

    fn aankondiging(stream_id: u32) -> ControlMsg {
        ControlMsg::StreamAnnounce(StreamAnnounce {
            stream_id,
            kind: StreamKind::MONITOR,
            title: "Scherm 1920×1080".into(),
            width: 1920,
            height: 1080,
            drawing: false,
            remote_control: false,
        })
    }

    fn intekening(stream_id: u32, poort: u16) -> ControlMsg {
        ControlMsg::StreamSubscribe(StreamSubscribe {
            stream_id,
            media_port: poort,
        })
    }

    #[test]
    fn delen_kost_niets_tot_iemand_kijkt() {
        // Dit is de belangrijkste eigenschap van de hele fase: een gedeeld scherm mag
        // geen enkele opdracht opleveren zolang niemand meekijkt.
        let mut s = Streams::new();
        let (id, cmds) = s.deel(StreamKind::MONITOR, "Scherm".into(), 1920, 1080, TEKENEN);
        assert_eq!(id, 1);
        assert_eq!(cmds.len(), 1, "alleen de aankondiging");
        assert!(!s.eigen()[0].wordt_bekeken());
    }

    #[test]
    fn eerste_kijker_start_het_opnemen_en_de_tweede_niet() {
        let mut s = Streams::new();
        let (id, _) = s.deel(StreamKind::MONITOR, "Scherm".into(), 1920, 1080, TEKENEN);
        let a = PeerId::new_random();
        let b = PeerId::new_random();

        let (_, acties) = s.bij_bericht(a, ip(), &intekening(id, 41000));
        assert_eq!(
            acties,
            vec![Actie::StartDelen {
                stream_id: id,
                kijkers: vec![PeerAdres {
                    id: a,
                    addr: SocketAddr::new(ip(), 41000)
                }],
            }]
        );

        let (_, acties) = s.bij_bericht(b, ip(), &intekening(id, 41001));
        assert!(
            matches!(acties.as_slice(), [Actie::ZetKijkers { .. }]),
            "de tweede kijker mag de encoder niet opnieuw starten: {acties:?}"
        );
        assert_eq!(s.eigen()[0].kijkers.len(), 2);
    }

    #[test]
    fn opnemen_stopt_pas_als_de_laatste_kijker_weg_is() {
        let mut s = Streams::new();
        let (id, _) = s.deel(StreamKind::MONITOR, "Scherm".into(), 1920, 1080, TEKENEN);
        let a = PeerId::new_random();
        let b = PeerId::new_random();
        s.bij_bericht(a, ip(), &intekening(id, 41000));
        s.bij_bericht(b, ip(), &intekening(id, 41001));

        let (_, acties) = s.bij_bericht(
            a,
            ip(),
            &ControlMsg::StreamUnsubscribe(StreamUnsubscribe { stream_id: id }),
        );
        assert!(
            matches!(acties.as_slice(), [Actie::ZetKijkers { .. }]),
            "b kijkt nog: {acties:?}"
        );

        let (_, acties) = s.bij_bericht(
            b,
            ip(),
            &ControlMsg::StreamUnsubscribe(StreamUnsubscribe { stream_id: id }),
        );
        assert_eq!(
            acties,
            vec![Actie::StopDelen {
                stream_id: id,
                is_geluid: false
            }]
        );
    }

    #[test]
    fn een_kijker_die_wegvalt_stopt_het_opnemen_ook() {
        // Zonder dit blijven we coderen en versturen naar een dood adres, en dat is
        // precies het geval waarin de app in de weg gaat zitten tijdens het gamen.
        let mut s = Streams::new();
        let (id, _) = s.deel(StreamKind::MONITOR, "Scherm".into(), 1920, 1080, TEKENEN);
        let a = PeerId::new_random();
        s.bij_bericht(a, ip(), &intekening(id, 41000));

        assert_eq!(
            s.bij_verbreking(a).1,
            vec![Actie::StopDelen {
                stream_id: id,
                is_geluid: false
            }]
        );
        assert!(!s.eigen()[0].wordt_bekeken());
    }

    #[test]
    fn een_peer_die_wegvalt_neemt_zijn_streams_mee() {
        let mut s = Streams::new();
        let a = PeerId::new_random();
        s.bij_bericht(a, ip(), &aankondiging(1));
        s.wil_kijken(a, 1);

        assert_eq!(
            s.bij_verbreking(a).1,
            vec![Actie::StopKijken {
                eigenaar: a,
                stream_id: 1
            }]
        );
        assert!(s.vreemd().is_empty(), "zijn streams horen weg te zijn");
    }

    #[test]
    fn opnieuw_aankondigen_gooit_de_kijkstatus_niet_om() {
        // Gebeurt bij elke herverbinding. Zou dit het kijken resetten, dan sprong het
        // venster dicht zodra de verbinding even hikte.
        let mut s = Streams::new();
        let a = PeerId::new_random();
        s.bij_bericht(a, ip(), &aankondiging(1));
        s.wil_kijken(a, 1);

        s.bij_bericht(a, ip(), &aankondiging(1));
        assert_eq!(s.vreemd().len(), 1, "geen duplicaat");
        assert!(s.vreemd()[0].kijken, "we keken al");
    }

    #[test]
    fn een_verbonden_peer_krijgt_onze_lopende_aankondigingen() {
        let mut s = Streams::new();
        s.deel(StreamKind::MONITOR, "Scherm".into(), 1920, 1080, TEKENEN);
        s.deel(StreamKind::WINDOW, "Kladblok".into(), 800, 600, TEKENEN);

        let a = PeerId::new_random();
        let cmds = s.bij_verbinding(a);
        assert_eq!(cmds.len(), 2);
        assert!(cmds
            .iter()
            .all(|c| matches!(c, MeshCommand::Send { to, .. } if *to == a)));
    }

    #[test]
    fn intrekken_sluit_het_venster_van_de_kijker() {
        let mut s = Streams::new();
        let a = PeerId::new_random();
        s.bij_bericht(a, ip(), &aankondiging(3));
        s.wil_kijken(a, 3);

        let (_, acties) = s.bij_bericht(
            a,
            ip(),
            &ControlMsg::StreamRevoke(StreamRevoke { stream_id: 3 }),
        );
        assert_eq!(
            acties,
            vec![Actie::StopKijken {
                eigenaar: a,
                stream_id: 3
            }]
        );
        assert!(s.vreemd().is_empty());
    }

    #[test]
    fn keyframe_alleen_voor_wie_daadwerkelijk_kijkt() {
        // Anders kan iedereen die het adres kent de bitrate laten ontploffen.
        let mut s = Streams::new();
        let (id, _) = s.deel(StreamKind::MONITOR, "Scherm".into(), 1920, 1080, TEKENEN);
        let kijker = PeerId::new_random();
        let vreemde = PeerId::new_random();
        s.bij_bericht(kijker, ip(), &intekening(id, 41000));

        let verzoek = ControlMsg::RequestKeyframe(RequestKeyframe { stream_id: id });
        let (_, acties) = s.bij_bericht(vreemde, ip(), &verzoek);
        assert!(acties.is_empty(), "die kijkt helemaal niet");

        let (_, acties) = s.bij_bericht(kijker, ip(), &verzoek);
        assert_eq!(acties, vec![Actie::StuurKeyframe { stream_id: id }]);
    }

    #[test]
    fn stream_ids_worden_nooit_hergebruikt() {
        // Een hergebruikt id zou beeld in het verkeerde venster kunnen zetten zolang
        // er nog pakketten van de vorige stream onderweg zijn.
        let mut s = Streams::new();
        let (eerste, _) = s.deel(StreamKind::MONITOR, "A".into(), 1920, 1080, TEKENEN);
        s.stop_delen(eerste);
        let (tweede, _) = s.deel(StreamKind::MONITOR, "B".into(), 1920, 1080, TEKENEN);
        assert_ne!(eerste, tweede);
    }

    #[test]
    fn aankondiging_op_de_voice_stream_wordt_geweigerd() {
        // Stream 0 is voice. Een peer die daar screenshare op aankondigt is stuk of
        // kwaadaardig; in beide gevallen willen we hem niet volgen.
        let mut s = Streams::new();
        let a = PeerId::new_random();
        s.bij_bericht(a, ip(), &aankondiging(VOICE_STREAM_ID));
        assert!(s.vreemd().is_empty());
    }

    #[test]
    fn onbekende_soort_stream_wordt_genegeerd_niet_getoond() {
        let mut s = Streams::new();
        let a = PeerId::new_random();
        s.bij_bericht(
            a,
            ip(),
            &ControlMsg::StreamAnnounce(StreamAnnounce {
                stream_id: 1,
                kind: StreamKind(200),
                title: "iets van later".into(),
                width: 1920,
                height: 1080,
                drawing: false,
                remote_control: false,
            }),
        );
        assert!(s.vreemd().is_empty());
    }

    #[test]
    fn intekenen_op_iets_wat_we_niet_delen_doet_niets() {
        let mut s = Streams::new();
        let a = PeerId::new_random();
        let (_, acties) = s.bij_bericht(a, ip(), &intekening(42, 41000));
        assert!(acties.is_empty());
    }

    #[test]
    fn tweemaal_kijken_opent_maar_een_venster() {
        let mut s = Streams::new();
        let a = PeerId::new_random();
        s.bij_bericht(a, ip(), &aankondiging(1));
        assert_eq!(s.wil_kijken(a, 1).len(), 1);
        assert!(s.wil_kijken(a, 1).is_empty(), "we keken al");
    }

    #[test]
    fn expliciet_stoppen_met_bureaubladgeluid_is_gemarkeerd_als_geluid() {
        // Bug (fase 10): stop_delen haalt de stream al uit `eigen` vóórdat de motor
        // met `is_geluid` zou kunnen navragen wat voor stream het was. Daarom draagt
        // de actie het zelf, in plaats van dat de motor het achteraf opzoekt.
        let mut s = Streams::new();
        let (id, _) = s.deel(
            StreamKind::DESKTOP_AUDIO,
            "Bureaubladgeluid".into(),
            0,
            0,
            TEKENEN,
        );
        let a = PeerId::new_random();
        s.bij_bericht(a, ip(), &intekening(id, 41000));

        let (_, acties) = s.stop_delen(id);
        assert_eq!(
            acties,
            vec![Actie::StopDelen {
                stream_id: id,
                is_geluid: true
            }]
        );
    }

    #[test]
    fn bureaubladgeluid_is_gemarkeerd_als_geluid() {
        // De motor moet aan de actie kunnen zien dat er geen venster open hoeft maar
        // dat het de mixer in gaat. Zonder deze vlag zou hij een decoder opstarten
        // voor iets wat geen beeld is.
        let mut s = Streams::new();
        let a = PeerId::new_random();
        s.bij_bericht(
            a,
            ip(),
            &ControlMsg::StreamAnnounce(StreamAnnounce {
                stream_id: 4,
                kind: StreamKind::DESKTOP_AUDIO,
                title: "Bureaubladgeluid".into(),
                width: 0,
                height: 0,
                drawing: false,
                remote_control: false,
            }),
        );

        match s.wil_kijken(a, 4).as_slice() {
            [Actie::StartKijken { is_geluid, .. }] => assert!(*is_geluid),
            andere => panic!("verwachtte één StartKijken, kreeg {andere:?}"),
        }
    }

    #[test]
    fn geluid_en_beeld_van_dezelfde_peer_staan_los_van_elkaar() {
        // Je wilt iemands scherm kunnen bekijken zonder zijn spelgeluid, en andersom.
        let mut s = Streams::new();
        let a = PeerId::new_random();
        s.bij_bericht(a, ip(), &aankondiging(1));
        s.bij_bericht(
            a,
            ip(),
            &ControlMsg::StreamAnnounce(StreamAnnounce {
                stream_id: 2,
                kind: StreamKind::DESKTOP_AUDIO,
                title: "Bureaubladgeluid".into(),
                width: 0,
                height: 0,
                drawing: false,
                remote_control: false,
            }),
        );

        s.wil_kijken(a, 1);
        assert!(s.vreemd().iter().any(|v| v.id == 1 && v.kijken));
        assert!(
            s.vreemd().iter().any(|v| v.id == 2 && !v.kijken),
            "het geluid hoort niet mee te liften op het beeld"
        );
    }

    #[test]
    fn geluid_volgt_het_beeld_aan_en_uit() {
        // De regressie van fase 12: de Tauri-UI tekent alleen op beeld in, dus zonder
        // deze afstemming hoort niemand nog geluid bij een gedeeld scherm.
        let mut s = Streams::new();
        let a = PeerId::new_random();
        s.bij_bericht(a, ip(), &aankondiging(1));
        s.bij_bericht(
            a,
            ip(),
            &ControlMsg::StreamAnnounce(StreamAnnounce {
                stream_id: 2,
                kind: StreamKind::DESKTOP_AUDIO,
                title: "Bureaubladgeluid".into(),
                width: 0,
                height: 0,
                drawing: false,
                remote_control: false,
            }),
        );

        // Niet in het gesprek: geen socket om het op te ontvangen.
        s.wil_kijken(a, 1);
        assert_eq!(s.stem_geluid_af_op_beeld(false).1, Vec::new());

        let (_, acties) = s.stem_geluid_af_op_beeld(true);
        assert!(
            matches!(
                acties.as_slice(),
                [Actie::StartKijken {
                    stream_id: 2,
                    is_geluid: true,
                    ..
                }]
            ),
            "geluid hoort mee te gaan met het beeld: {acties:?}"
        );
        assert!(s.stem_geluid_af_op_beeld(true).1.is_empty(), "idempotent");

        // Laatste beeld dicht: het geluid gaat mee uit, mét afmelding bij de deler.
        s.stop_kijken(a, 1);
        let (cmds, acties) = s.stem_geluid_af_op_beeld(true);
        assert_eq!(
            acties,
            vec![Actie::StopKijken {
                eigenaar: a,
                stream_id: 2
            }]
        );
        assert_eq!(cmds.len(), 1, "de deler moet stoppen met versturen");
    }

    #[test]
    fn naar_een_camera_kijken_haalt_geen_bureaubladgeluid_binnen() {
        // Een camera is beeld, maar geen scherm: er hoort geen systeemgeluid bij. Deelt
        // iemand beide, dan mag alleen het scherm het geluid meetrekken.
        let mut s = Streams::new();
        let a = PeerId::new_random();
        s.bij_bericht(
            a,
            ip(),
            &ControlMsg::StreamAnnounce(StreamAnnounce {
                stream_id: 1,
                kind: StreamKind::CAMERA,
                title: "Logitech C920".into(),
                width: 1280,
                height: 720,
                drawing: false,
                remote_control: false,
            }),
        );
        s.bij_bericht(
            a,
            ip(),
            &ControlMsg::StreamAnnounce(StreamAnnounce {
                stream_id: 2,
                kind: StreamKind::DESKTOP_AUDIO,
                title: "Bureaubladgeluid".into(),
                width: 0,
                height: 0,
                drawing: false,
                remote_control: false,
            }),
        );

        s.wil_kijken(a, 1);
        assert_eq!(
            s.stem_geluid_af_op_beeld(true).1,
            Vec::new(),
            "een camera trekt geen bureaubladgeluid mee"
        );

        // Zijn scherm erbij pakken doet het wél.
        s.bij_bericht(a, ip(), &aankondiging(3));
        s.wil_kijken(a, 3);
        let (_, acties) = s.stem_geluid_af_op_beeld(true);
        assert!(
            matches!(acties.as_slice(), [Actie::StartKijken { stream_id: 2, .. }]),
            "bij een gedeeld scherm hoort het geluid wel: {acties:?}"
        );
    }

    #[test]
    fn opnieuw_intekenen_met_een_andere_poort_verhuist_de_kijker() {
        // Gebeurt als iemand zijn venster sluit en meteen opnieuw opent.
        let mut s = Streams::new();
        let (id, _) = s.deel(StreamKind::MONITOR, "Scherm".into(), 1920, 1080, TEKENEN);
        let a = PeerId::new_random();
        s.bij_bericht(a, ip(), &intekening(id, 41000));
        let (_, acties) = s.bij_bericht(a, ip(), &intekening(id, 41005));

        assert_eq!(
            acties,
            vec![Actie::ZetKijkers {
                stream_id: id,
                kijkers: vec![PeerAdres {
                    id: a,
                    addr: SocketAddr::new(ip(), 41005)
                }],
            }],
            "de oude poort mag geen beeld meer krijgen"
        );
    }

    fn muziek(stream_id: u32, titel: &str) -> ControlMsg {
        ControlMsg::StreamAnnounce(StreamAnnounce {
            stream_id,
            kind: StreamKind::MUSIC,
            title: titel.into(),
            width: 0,
            height: 0,
            drawing: false,
            remote_control: false,
        })
    }

    #[test]
    fn muziek_hoor_je_in_het_gesprek_zonder_ergens_naar_te_kijken() {
        let mut s = Streams::new();
        let dj = PeerId::new_random();
        s.bij_bericht(dj, ip(), &muziek(3, "Spotify"));

        let (_, acties) = s.stem_geluid_af_op_beeld(false);
        assert!(acties.is_empty(), "buiten het gesprek geen geluid");

        let (_, acties) = s.stem_geluid_af_op_beeld(true);
        assert!(matches!(
            acties.as_slice(),
            [Actie::StartKijken {
                stream_id: 3,
                is_geluid: true,
                ..
            }]
        ));
        assert!(s.stem_geluid_af_op_beeld(true).1.is_empty(), "idempotent");

        // Een nieuw nummer is een nieuwe titel, geen nieuwe stream.
        s.bij_bericht(dj, ip(), &muziek(3, "Artiest – Nummer"));
        assert_eq!(s.vreemd().len(), 1);
        assert_eq!(s.vreemd()[0].titel, "Artiest – Nummer");
        assert!(s.vreemd()[0].kijken);

        let (cmds, acties) = s.stem_geluid_af_op_beeld(false);
        assert_eq!(cmds.len(), 1, "uittekenen bij het verlaten van het gesprek");
        assert!(matches!(acties.as_slice(), [Actie::StopKijken { .. }]));
    }

    #[test]
    fn titel_wijzigen_kondigt_alleen_een_echte_wijziging_aan() {
        let mut s = Streams::new();
        let (id, _) = s.deel(StreamKind::MUSIC, "Spotify".into(), 0, 0, TEKENEN);
        assert!(s.zet_titel(id, "Spotify").is_empty());
        assert_eq!(s.zet_titel(id, "A – B").len(), 1);
        assert!(s.zet_titel(id, "A – B").is_empty());
        assert!(s.zet_titel(id + 1, "C").is_empty());
    }

    // -- tekenen (beslissing 45) ------------------------------------------

    fn lijn(stream_id: u32, id: u32, start: u32, punten: &[[u16; 2]]) -> Draw {
        Draw {
            stream_id,
            author: None,
            line: id,
            color: 1,
            width: 1,
            start,
            points: punten.to_vec(),
        }
    }

    /// Wie een `Send` krijgt, en wat.
    fn verzonden(cmds: &[MeshCommand]) -> Vec<(PeerId, &ControlMsg)> {
        cmds.iter()
            .filter_map(|c| match c {
                MeshCommand::Send { to, msg } => Some((*to, msg)),
                _ => None,
            })
            .collect()
    }

    /// Een gedeeld scherm met twee kijkers.
    fn deler_met_kijkers() -> (Streams, u32, PeerId, PeerId) {
        let mut s = Streams::new();
        let (id, _) = s.deel(StreamKind::MONITOR, "Scherm".into(), 1920, 1080, TEKENEN);
        let a = PeerId::new_random();
        let b = PeerId::new_random();
        s.bij_bericht(a, ip(), &intekening(id, 41000));
        s.bij_bericht(b, ip(), &intekening(id, 41001));
        (s, id, a, b)
    }

    #[test]
    fn een_scherm_kondigt_tekenen_aan_en_een_camera_niet() {
        let mut s = Streams::new();
        let (_, cmds) = s.deel(StreamKind::MONITOR, "Scherm".into(), 1920, 1080, TEKENEN);
        let (_, cam) = s.deel(StreamKind::CAMERA, "Cam".into(), 1280, 720, TEKENEN);
        let tekenen = |c: &[MeshCommand]| match c {
            [MeshCommand::Broadcast(ControlMsg::StreamAnnounce(a))] => a.drawing,
            andere => panic!("verwachtte een aankondiging: {andere:?}"),
        };
        assert!(tekenen(&cmds));
        assert!(!tekenen(&cam));
    }

    #[test]
    fn de_deler_geeft_een_lijn_door_aan_alle_kijkers_met_de_tekenaar_erin() {
        let (mut s, id, a, b) = deler_met_kijkers();
        let (cmds, acties) = s.bij_bericht(
            a,
            ip(),
            &ControlMsg::Draw(lijn(id, 1, 0, &[[0, 0], [9, 9]])),
        );

        let mut ontvangers: Vec<PeerId> = verzonden(&cmds).iter().map(|(p, _)| *p).collect();
        ontvangers.sort();
        let mut verwacht = vec![a, b];
        verwacht.sort();
        assert_eq!(
            ontvangers, verwacht,
            "ook de tekenaar zelf krijgt hem terug"
        );
        assert!(verzonden(&cmds)
            .iter()
            .all(|(_, m)| matches!(m, ControlMsg::Draw(d) if d.author == Some(a))));
        assert_eq!(
            acties,
            vec![Actie::Tekening {
                eigenaar: None,
                stream_id: id
            }]
        );
        assert_eq!(s.eigen()[0].tekening.lijnen().len(), 1);
    }

    #[test]
    fn de_deler_neemt_niets_aan_van_wie_niet_kijkt_of_als_het_uit_staat() {
        let (mut s, id, a, _) = deler_met_kijkers();
        let vreemde = PeerId::new_random();
        let (cmds, _) = s.bij_bericht(
            vreemde,
            ip(),
            &ControlMsg::Draw(lijn(id, 1, 0, &[[0, 0], [9, 9]])),
        );
        assert!(cmds.is_empty());

        s.zet_tekenen(id, false);
        let (cmds, _) = s.bij_bericht(
            a,
            ip(),
            &ControlMsg::Draw(lijn(id, 1, 0, &[[0, 0], [9, 9]])),
        );
        assert!(cmds.is_empty());
        assert!(s.eigen()[0].tekening.is_leeg());
    }

    #[test]
    fn uitzetten_wist_en_kondigt_opnieuw_aan_en_geldt_per_scherm() {
        let (mut s, id, a, _) = deler_met_kijkers();
        s.bij_bericht(
            a,
            ip(),
            &ControlMsg::Draw(lijn(id, 1, 0, &[[0, 0], [9, 9]])),
        );
        let (tweede, _) = s.deel(StreamKind::MONITOR, "Tweede".into(), 1920, 1080, TEKENEN);

        let (cmds, acties) = s.zet_tekenen(id, false);
        assert!(matches!(
            cmds.as_slice(),
            [MeshCommand::Broadcast(ControlMsg::StreamAnnounce(an))] if !an.drawing
        ));
        assert_eq!(acties.len(), 1);
        assert!(s.eigen()[0].tekening.is_leeg());
        let (cmds, acties) = s.zet_tekenen(id, false);
        assert!(cmds.is_empty() && acties.is_empty(), "al uit");
        assert!(
            s.tekening(None, tweede).unwrap().aan,
            "het andere scherm houdt zijn eigen keuze"
        );
        // Een camera tekent nooit, wat er ook gevraagd wordt.
        let (cam, _) = s.deel(StreamKind::CAMERA, "Cam".into(), 1280, 720, TEKENEN);
        assert!(s.zet_tekenen(cam, true).0.is_empty());
        assert!(!s.tekening(None, cam).unwrap().aan);
    }

    #[test]
    fn een_nieuwe_kijker_krijgt_de_hele_tekening() {
        let (mut s, id, a, _) = deler_met_kijkers();
        s.bij_bericht(
            a,
            ip(),
            &ControlMsg::Draw(lijn(id, 1, 0, &[[0, 0], [9, 9]])),
        );

        let c = PeerId::new_random();
        let (cmds, _) = s.bij_bericht(c, ip(), &intekening(id, 41002));
        match verzonden(&cmds).as_slice() {
            [(wie, ControlMsg::DrawState(t))] => {
                assert_eq!(*wie, c);
                assert_eq!(t.lines.len(), 1);
                assert_eq!(t.lines[0].author, a);
            }
            andere => panic!("verwachtte een DrawState: {andere:?}"),
        }
    }

    #[test]
    fn wie_vertrekt_neemt_zijn_lijnen_mee() {
        let (mut s, id, a, b) = deler_met_kijkers();
        s.bij_bericht(
            a,
            ip(),
            &ControlMsg::Draw(lijn(id, 1, 0, &[[0, 0], [9, 9]])),
        );
        s.bij_bericht(
            b,
            ip(),
            &ControlMsg::Draw(lijn(id, 1, 0, &[[5, 5], [6, 6]])),
        );

        // Uittekenen: b krijgt een tekening zonder a.
        let (cmds, acties) = s.bij_bericht(
            a,
            ip(),
            &ControlMsg::StreamUnsubscribe(StreamUnsubscribe { stream_id: id }),
        );
        match verzonden(&cmds).as_slice() {
            [(wie, ControlMsg::DrawState(t))] => {
                assert_eq!(*wie, b);
                assert!(t.lines.iter().all(|l| l.author == b));
            }
            andere => panic!("verwachtte een DrawState naar b: {andere:?}"),
        }
        assert!(acties.contains(&Actie::Tekening {
            eigenaar: None,
            stream_id: id
        }));

        // Wegvallen doet hetzelfde.
        let (_, acties) = s.bij_verbreking(b);
        assert!(acties.contains(&Actie::Tekening {
            eigenaar: None,
            stream_id: id
        }));
        assert!(s.eigen()[0].tekening.is_leeg());
    }

    #[test]
    fn een_kijker_wist_alleen_zijn_eigen_lijnen_en_iedereen_ziet_het() {
        let (mut s, id, a, b) = deler_met_kijkers();
        s.bij_bericht(
            a,
            ip(),
            &ControlMsg::Draw(lijn(id, 1, 0, &[[0, 0], [9, 9]])),
        );
        s.bij_bericht(
            a,
            ip(),
            &ControlMsg::Draw(lijn(id, 2, 0, &[[1, 1], [2, 2]])),
        );
        s.bij_bericht(
            b,
            ip(),
            &ControlMsg::Draw(lijn(id, 1, 0, &[[5, 5], [6, 6]])),
        );

        let wis = |scope| {
            ControlMsg::DrawClear(DrawClear {
                stream_id: id,
                scope,
            })
        };
        let (cmds, _) = s.bij_bericht(a, ip(), &wis(DrawClearScope::LAST));
        assert_eq!(
            verzonden(&cmds).len(),
            2,
            "beide kijkers krijgen de tekening"
        );
        assert_eq!(s.eigen()[0].tekening.lijnen().len(), 2);

        s.bij_bericht(a, ip(), &wis(DrawClearScope::MINE));
        let over = s.eigen()[0].tekening.clone();
        assert!(over.lijnen().iter().all(|l| l.tekenaar == b));
        let (cmds, _) = s.bij_bericht(a, ip(), &wis(DrawClearScope(99)));
        assert!(cmds.is_empty(), "onbekende soort");

        let (cmds, _) = s.wis_tekening(id);
        assert_eq!(verzonden(&cmds).len(), 2);
        assert!(s.eigen()[0].tekening.is_leeg());
    }

    #[test]
    fn een_kijker_neemt_tekenwerk_alleen_aan_van_de_eigenaar_van_wat_hij_bekijkt() {
        let mut s = Streams::new();
        let deler = PeerId::new_random();
        let ander = PeerId::new_random();
        let mut an = match aankondiging(3) {
            ControlMsg::StreamAnnounce(a) => a,
            _ => unreachable!(),
        };
        an.drawing = true;
        s.bij_bericht(deler, ip(), &ControlMsg::StreamAnnounce(an.clone()));
        let mut door = lijn(3, 1, 0, &[[0, 0], [9, 9]]);
        door.author = Some(ander);

        // Nog niet kijken: niets.
        s.bij_bericht(deler, ip(), &ControlMsg::Draw(door.clone()));
        assert!(s.tekening(Some(deler), 3).unwrap().is_leeg());

        s.wil_kijken(deler, 3);
        // Van een ander dan de deler: niets, ook al noemt hij dezelfde stream.
        let (_, acties) = s.bij_bericht(ander, ip(), &ControlMsg::Draw(door.clone()));
        assert!(acties.is_empty());
        let (_, acties) = s.bij_bericht(deler, ip(), &ControlMsg::Draw(door.clone()));
        assert_eq!(
            acties,
            vec![Actie::Tekening {
                eigenaar: Some(deler),
                stream_id: 3
            }]
        );

        // Een `DrawState` vervangt alles.
        let (_, acties) = s.bij_bericht(
            deler,
            ip(),
            &ControlMsg::DrawState(DrawState {
                stream_id: 3,
                lines: vec![],
            }),
        );
        assert_eq!(acties.len(), 1);
        assert!(s.tekening(Some(deler), 3).unwrap().is_leeg());

        // De deler zet het uit: onze kopie gaat leeg en blijft leeg.
        s.bij_bericht(deler, ip(), &ControlMsg::Draw(door.clone()));
        an.drawing = false;
        let (_, acties) = s.bij_bericht(deler, ip(), &ControlMsg::StreamAnnounce(an));
        assert_eq!(acties.len(), 1);
        let t = s.tekening(Some(deler), 3).unwrap();
        assert!(t.is_leeg() && !t.aan);
    }

    #[test]
    fn eigen_tekenwerk_gaat_alleen_naar_de_deler_en_alleen_als_het_mag() {
        let mut s = Streams::new();
        let deler = PeerId::new_random();
        let mut an = match aankondiging(3) {
            ControlMsg::StreamAnnounce(a) => a,
            _ => unreachable!(),
        };
        an.drawing = true;
        s.bij_bericht(deler, ip(), &ControlMsg::StreamAnnounce(an));
        let msg = ControlMsg::Draw(lijn(3, 1, 0, &[[0, 0], [9, 9]]));

        assert!(
            s.kijker_tekent(deler, 3, msg.clone()).is_empty(),
            "niet kijkend"
        );
        s.wil_kijken(deler, 3);
        assert!(matches!(
            s.kijker_tekent(deler, 3, msg.clone()).as_slice(),
            [MeshCommand::Send { to, .. }] if *to == deler
        ));
        assert!(
            s.kijker_tekent(deler, 4, msg.clone()).is_empty(),
            "andere stream dan in het bericht"
        );

        // Stoppen met kijken gooit onze kopie weg; daarna mag er niets meer heen.
        s.stop_kijken(deler, 3);
        assert!(s.kijker_tekent(deler, 3, msg).is_empty());
    }

    #[test]
    fn een_kopie_met_een_gat_haalt_de_hele_tekening_opnieuw_en_niet_te_vaak() {
        let mut s = Streams::new();
        let deler = PeerId::new_random();
        let mut an = match aankondiging(3) {
            ControlMsg::StreamAnnounce(a) => a,
            _ => unreachable!(),
        };
        an.drawing = true;
        s.bij_bericht(deler, ip(), &ControlMsg::StreamAnnounce(an));
        s.wil_kijken(deler, 3);
        s.kijker_draait(deler, 3, 41000);

        // Het eerste stuk van deze lijn ging onderweg verloren.
        let mut verder = lijn(3, 1, 64, &[[9, 9], [10, 10]]);
        verder.author = Some(PeerId::new_random());
        let (cmds, acties) = s.bij_bericht(deler, ip(), &ControlMsg::Draw(verder.clone()));
        assert!(acties.is_empty());
        match verzonden(&cmds).as_slice() {
            [(wie, ControlMsg::StreamSubscribe(sub))] => {
                assert_eq!(*wie, deler);
                assert_eq!(
                    (sub.stream_id, sub.media_port),
                    (3, 41000),
                    "dezelfde poort"
                );
            }
            andere => panic!("verwachtte opnieuw intekenen: {andere:?}"),
        }
        let (cmds, _) = s.bij_bericht(deler, ip(), &ControlMsg::Draw(verder));
        assert!(cmds.is_empty(), "niet bij elk volgend stuk opnieuw");

        // De deler antwoordt op een intekening altijd met de tekening, ook als hij leeg is.
        let mut d = Streams::new();
        let (id, _) = d.deel(StreamKind::MONITOR, "Scherm".into(), 1920, 1080, TEKENEN);
        let (cmds, _) = d.bij_bericht(deler, ip(), &intekening(id, 41000));
        assert!(matches!(
            verzonden(&cmds).as_slice(),
            [(_, ControlMsg::DrawState(t))] if t.lines.is_empty()
        ));
    }

    #[test]
    fn hetzelfde_stream_id_bij_twee_eigenaren_raakt_niet_door_elkaar() {
        // Stream-id's zijn per eigenaar uniek. a kijkt naar ons scherm 1 én deelt zelf een
        // scherm 1 waar wij naar kijken. Zonder tekenaar is het ons scherm, met tekenaar het
        // zijne.
        let mut s = Streams::new();
        let (id, _) = s.deel(StreamKind::MONITOR, "Scherm".into(), 1920, 1080, TEKENEN);
        let a = PeerId::new_random();
        s.bij_bericht(a, ip(), &intekening(id, 41000));
        let mut an = match aankondiging(id) {
            ControlMsg::StreamAnnounce(x) => x,
            _ => unreachable!(),
        };
        an.drawing = true;
        s.bij_bericht(a, ip(), &ControlMsg::StreamAnnounce(an));
        s.wil_kijken(a, id);

        s.bij_bericht(
            a,
            ip(),
            &ControlMsg::Draw(lijn(id, 1, 0, &[[0, 0], [9, 9]])),
        );
        assert_eq!(s.tekening(None, id).unwrap().lijnen().len(), 1);
        assert!(s.tekening(Some(a), id).unwrap().is_leeg());

        let mut zijne = lijn(id, 1, 0, &[[0, 0], [9, 9]]);
        zijne.author = Some(PeerId::new_random());
        s.bij_bericht(a, ip(), &ControlMsg::Draw(zijne));
        assert_eq!(s.tekening(Some(a), id).unwrap().lijnen().len(), 1);
        assert_eq!(s.tekening(None, id).unwrap().lijnen().len(), 1);
    }

    // -- bedienen (beslissing 54) -----------------------------------------

    use fitcom_proto::control::InputKind;

    const BEDIENEN: DeelOpties = DeelOpties {
        tekenen: false,
        bedienen: true,
    };

    fn vraag(stream_id: u32, want: bool) -> ControlMsg {
        ControlMsg::RemoteRequest(RemoteRequest { stream_id, want })
    }

    fn invoer(stream_id: u32, events: Vec<InputEvent>) -> ControlMsg {
        ControlMsg::RemoteInput(RemoteInput { stream_id, events })
    }

    fn toets(scan: u16, kind: InputKind) -> InputEvent {
        InputEvent {
            kind,
            x: 0,
            y: 0,
            button: 0,
            delta: 0,
            scan,
            vk: 0,
        }
    }

    fn a_omlaag() -> InputEvent {
        toets(0x1E, InputKind::KEY_DOWN)
    }

    fn klembord(stream_id: u32, text: &str) -> ControlMsg {
        ControlMsg::ClipboardText(ClipboardText {
            stream_id,
            text: text.into(),
        })
    }

    fn stand(stream_id: u32, granted: bool) -> ControlMsg {
        ControlMsg::RemoteState(RemoteState { stream_id, granted })
    }

    /// Wat er aan `RemoteState` naar wie ging.
    fn standen(cmds: &[MeshCommand]) -> Vec<(PeerId, bool)> {
        verzonden(cmds)
            .into_iter()
            .filter_map(|(p, m)| match m {
                ControlMsg::RemoteState(r) => Some((p, r.granted)),
                _ => None,
            })
            .collect()
    }

    /// Een gedeelde monitor die bediend mag worden, met twee kijkers.
    fn monitor_met_kijkers() -> (Streams, u32, PeerId, PeerId) {
        let mut s = Streams::new();
        let (id, _) = s.deel(StreamKind::MONITOR, "Scherm".into(), 1920, 1080, BEDIENEN);
        let a = PeerId::new_random();
        let b = PeerId::new_random();
        s.bij_bericht(a, ip(), &intekening(id, 41000));
        s.bij_bericht(b, ip(), &intekening(id, 41001));
        (s, id, a, b)
    }

    /// `a` vroeg en de deler klikte Allow.
    fn a_bedient() -> (Streams, u32, PeerId, PeerId) {
        let (mut s, id, a, b) = monitor_met_kijkers();
        s.bij_bericht(a, ip(), &vraag(id, true));
        let (cmds, acties) = s.sta_bediening_toe(id, a);
        assert_eq!(standen(&cmds), vec![(a, true)]);
        assert_eq!(acties, vec![Actie::StartBediening { stream_id: id }]);
        (s, id, a, b)
    }

    fn aankondiging_met_bedienen(stream_id: u32, aan: bool) -> ControlMsg {
        match aankondiging(stream_id) {
            ControlMsg::StreamAnnounce(mut a) => {
                a.remote_control = aan;
                ControlMsg::StreamAnnounce(a)
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn alleen_een_monitor_met_de_optie_kondigt_bedienen_aan() {
        let mut s = Streams::new();
        let bedienen = |c: &[MeshCommand]| match c {
            [MeshCommand::Broadcast(ControlMsg::StreamAnnounce(a))] => a.remote_control,
            andere => panic!("verwachtte een aankondiging: {andere:?}"),
        };
        assert!(bedienen(
            &s.deel(StreamKind::MONITOR, "M".into(), 1, 1, BEDIENEN).1
        ));
        assert!(!bedienen(
            &s.deel(StreamKind::MONITOR, "M".into(), 1, 1, TEKENEN).1
        ));
        // Een venster of camera nooit: een klik zou vallen op wat er toevallig bovenop ligt.
        assert!(!bedienen(
            &s.deel(StreamKind::WINDOW, "V".into(), 1, 1, BEDIENEN).1
        ));
        assert!(!bedienen(
            &s.deel(StreamKind::CAMERA, "C".into(), 1, 1, BEDIENEN).1
        ));
        let venster = s.eigen()[2].id;
        assert!(s.zet_bedienen(venster, true).0.is_empty());
    }

    #[test]
    fn vragen_kan_alleen_een_kijker_en_alleen_als_het_mag() {
        let (mut s, id, a, _) = monitor_met_kijkers();
        let vreemde = PeerId::new_random();
        let (cmds, acties) = s.bij_bericht(vreemde, ip(), &vraag(id, true));
        assert!(cmds.is_empty() && acties.is_empty(), "kijkt niet");

        s.zet_bedienen(id, false);
        let (cmds, acties) = s.bij_bericht(a, ip(), &vraag(id, true));
        assert_eq!(standen(&cmds), vec![(a, false)], "optie uit: meteen nee");
        assert!(acties.is_empty());

        s.zet_bedienen(id, true);
        let (cmds, acties) = s.bij_bericht(a, ip(), &vraag(id, true));
        assert!(cmds.is_empty(), "wacht op een klik, geen antwoord");
        assert_eq!(
            acties,
            vec![Actie::BedienVraag {
                stream_id: id,
                van: a
            }]
        );
        assert_eq!(s.eigen()[0].bediening.vrager.map(|(v, _)| v), Some(a));
        let (cmds, acties) = s.bij_bericht(a, ip(), &vraag(id, true));
        assert!(
            cmds.is_empty() && acties.is_empty(),
            "dezelfde vraag nog eens: niets"
        );
    }

    #[test]
    fn zonder_klik_geen_invoer_en_daarna_alleen_van_de_bediener() {
        let (mut s, id, a, _) = monitor_met_kijkers();
        s.bij_bericht(a, ip(), &vraag(id, true));
        let (_, acties) = s.bij_bericht(a, ip(), &invoer(id, vec![a_omlaag()]));
        assert!(acties.is_empty(), "nog geen toestemming");

        let (mut s, id, a, b) = a_bedient();
        let (_, acties) = s.bij_bericht(a, ip(), &invoer(id, vec![a_omlaag()]));
        assert_eq!(
            acties,
            vec![Actie::Invoer {
                stream_id: id,
                events: vec![a_omlaag()]
            }]
        );
        // De afzender is de verbinding: een andere kijker die hetzelfde stuurt doet niets.
        let (_, acties) = s.bij_bericht(b, ip(), &invoer(id, vec![a_omlaag()]));
        assert!(acties.is_empty());
        // Ongeldige invoer valt weg, de rest niet.
        let rommel = toets(0, InputKind(77));
        let (_, acties) = s.bij_bericht(a, ip(), &invoer(id, vec![rommel, a_omlaag()]));
        assert!(matches!(acties.as_slice(), [Actie::Invoer { events, .. }] if events.len() == 1));
        // Een ander stream-id dan het bediende: niets.
        let (_, acties) = s.bij_bericht(a, ip(), &invoer(id + 1, vec![a_omlaag()]));
        assert!(acties.is_empty());
    }

    #[test]
    fn een_tegelijk_en_een_tweede_vrager_hoort_meteen_nee() {
        let (mut s, id, a, b) = monitor_met_kijkers();
        s.bij_bericht(a, ip(), &vraag(id, true));
        let (cmds, acties) = s.bij_bericht(b, ip(), &vraag(id, true));
        assert_eq!(standen(&cmds), vec![(b, false)], "a vraagt al");
        assert!(acties.is_empty());

        s.sta_bediening_toe(id, a);
        let (cmds, _) = s.bij_bericht(b, ip(), &vraag(id, true));
        assert_eq!(standen(&cmds), vec![(b, false)], "a bedient al");
        // Toestemming voor iemand die niet vroeg doet niets.
        let (cmds, acties) = s.sta_bediening_toe(id, b);
        assert!(cmds.is_empty() && acties.is_empty());
        assert_eq!(s.eigen()[0].bediening.bediener, Some(a));
    }

    #[test]
    fn weigeren_zegt_nee_en_een_verouderde_klik_doet_niets() {
        let (mut s, id, a, _) = monitor_met_kijkers();
        s.bij_bericht(a, ip(), &vraag(id, true));
        assert_eq!(standen(&s.weiger_bediening(id)), vec![(a, false)]);
        assert!(s.sta_bediening_toe(id, a).1.is_empty(), "de vraag is weg");

        // Vragen en dan wegkijken: de klik komt te laat.
        s.bij_bericht(a, ip(), &vraag(id, true));
        s.bij_bericht(
            a,
            ip(),
            &ControlMsg::StreamUnsubscribe(StreamUnsubscribe { stream_id: id }),
        );
        assert!(s.eigen()[0].bediening.vrager.is_none());
        assert!(s.sta_bediening_toe(id, a).1.is_empty());
    }

    #[test]
    fn het_bedienen_stopt_op_elke_manier() {
        let stop = |id| Actie::StopBediening { stream_id: id };

        // Ingetrokken door de deler: de bediener hoort het.
        let (mut s, id, a, _) = a_bedient();
        let (cmds, acties) = s.stop_bediening(id);
        assert_eq!((standen(&cmds), acties), (vec![(a, false)], vec![stop(id)]));

        // De optie uit: ook een nieuwe aankondiging, en daarna geen invoer meer.
        let (mut s, id, a, _) = a_bedient();
        let (cmds, acties) = s.zet_bedienen(id, false);
        assert_eq!(standen(&cmds), vec![(a, false)]);
        assert!(cmds.iter().any(|c| matches!(
            c,
            MeshCommand::Broadcast(ControlMsg::StreamAnnounce(an)) if !an.remote_control
        )));
        assert_eq!(acties, vec![stop(id)]);
        let (_, acties) = s.bij_bericht(a, ip(), &invoer(id, vec![a_omlaag()]));
        assert!(acties.is_empty());

        // Stoppen met delen.
        let (mut s, id, _, _) = a_bedient();
        assert!(s.stop_delen(id).1.contains(&stop(id)));

        // De bediener stopt met kijken.
        let (mut s, id, a, _) = a_bedient();
        let (_, acties) = s.bij_bericht(
            a,
            ip(),
            &ControlMsg::StreamUnsubscribe(StreamUnsubscribe { stream_id: id }),
        );
        assert!(acties.contains(&stop(id)));

        // De bediener valt weg.
        let (mut s, id, a, _) = a_bedient();
        assert!(s.bij_verbreking(a).1.contains(&stop(id)));

        // De bediener stopt zelf.
        let (mut s, id, a, _) = a_bedient();
        let (_, acties) = s.bij_bericht(a, ip(), &vraag(id, false));
        assert_eq!(acties, vec![stop(id)]);

        // Een andere kijker die wegvalt raakt het bedienen van a niet.
        let (mut s, id, a, b) = a_bedient();
        assert!(!s.bij_verbreking(b).1.contains(&stop(id)));
        assert_eq!(s.eigen()[0].bediening.bediener, Some(a));
    }

    #[test]
    fn invoer_wordt_afgeremd_maar_loslaten_nooit() {
        use fitcom_proto::bediening::MAX_INVOER_PER_BERICHT;
        let (mut s, id, a, _) = a_bedient();
        let mut aangenomen = 0;
        for _ in 0..20 {
            let (_, acties) = s.bij_bericht(
                a,
                ip(),
                &invoer(id, vec![a_omlaag(); MAX_INVOER_PER_BERICHT]),
            );
            if let [Actie::Invoer { events, .. }] = acties.as_slice() {
                aangenomen += events.len();
            }
        }
        assert!(
            aangenomen < 20 * MAX_INVOER_PER_BERICHT,
            "geremd: {aangenomen}"
        );
        // Leeg: loslaten gaat er nog steeds door, ook voorbij de grens per bericht.
        let mut veel = vec![a_omlaag(); MAX_INVOER_PER_BERICHT + 10];
        veel.push(toets(0x2A, InputKind::KEY_UP));
        let (_, acties) = s.bij_bericht(a, ip(), &invoer(id, veel));
        match acties.as_slice() {
            [Actie::Invoer { events, .. }] => {
                assert_eq!(events.last().map(|e| e.kind), Some(InputKind::KEY_UP))
            }
            andere => panic!("het loslaten moet erdoor: {andere:?}"),
        }
    }

    #[test]
    fn een_vraag_verloopt_en_vragen_zelf_worden_afgeremd() {
        let (mut s, id, a, _) = monitor_met_kijkers();
        s.bij_bericht(a, ip(), &vraag(id, true));
        assert!(s.verlopen_vragen(Instant::now()).is_empty(), "nog vers");
        let later = Instant::now() + VRAAG_GELDIG + Duration::from_secs(1);
        assert_eq!(standen(&s.verlopen_vragen(later)), vec![(a, false)]);
        assert!(s.eigen()[0].bediening.vrager.is_none());

        // Steeds vragen en intrekken: na de voorraad is het meteen nee, zonder melding.
        let mut meldingen = 0;
        for _ in 0..10 {
            let (_, acties) = s.bij_bericht(a, ip(), &vraag(id, true));
            meldingen += acties.len();
            s.bij_bericht(a, ip(), &vraag(id, false));
        }
        assert!(meldingen < 10, "{meldingen} meldingen");
    }

    #[test]
    fn het_klembord_gaat_alleen_tussen_deler_en_bediener() {
        let (mut s, id, a, b) = a_bedient();
        let (_, acties) = s.bij_bericht(a, ip(), &klembord(id, "hallo"));
        assert_eq!(
            acties,
            vec![Actie::Klembord {
                eigenaar: None,
                stream_id: id,
                tekst: "hallo".into()
            }]
        );
        let (_, acties) = s.bij_bericht(b, ip(), &klembord(id, "hallo"));
        assert!(acties.is_empty(), "een gewone kijker niet");
        let groot = "x".repeat(fitcom_proto::bediening::MAX_KLEMBORD + 1);
        assert!(s.bij_bericht(a, ip(), &klembord(id, &groot)).1.is_empty());

        // Van de deler naar de bediener, en alleen naar hem, zonder NUL.
        let cmds = s.klembord_van_deler(id, "terug\0");
        match verzonden(&cmds).as_slice() {
            [(wie, ControlMsg::ClipboardText(c))] => {
                assert_eq!((*wie, c.text.as_str()), (a, "terug"));
            }
            andere => panic!("verwachtte klembord naar a: {andere:?}"),
        }
        s.stop_bediening(id);
        assert!(s.klembord_van_deler(id, "na afloop").is_empty());
        assert!(s
            .bij_bericht(a, ip(), &klembord(id, "na afloop"))
            .1
            .is_empty());
    }

    /// Een monitor van `deler` die bediend mag worden, en wij kijken.
    fn kijker_bij(deler: PeerId, id: u32) -> Streams {
        let mut s = Streams::new();
        s.bij_bericht(deler, ip(), &aankondiging_met_bedienen(id, true));
        s.wil_kijken(deler, id);
        s
    }

    #[test]
    fn als_kijker_alleen_een_ja_als_we_vroegen_en_alleen_van_de_eigenaar() {
        let deler = PeerId::new_random();
        let ander = PeerId::new_random();
        let mut s = kijker_bij(deler, 3);
        assert_eq!(s.bedienstand(deler, 3), Bedienstand::Mag);

        // Ongevraagd ja: niets, en invoer gaat nog niet weg.
        assert!(s.bij_bericht(deler, ip(), &stand(3, true)).1.is_empty());
        assert_eq!(s.bedienstand(deler, 3), Bedienstand::Mag);
        let (cmds, _) = s.kijker_bedient(deler, 3, invoer(3, vec![a_omlaag()]));
        assert!(cmds.is_empty());

        let (cmds, acties) = s.kijker_bedient(deler, 3, vraag(3, true));
        assert!(matches!(
            verzonden(&cmds).as_slice(),
            [(p, ControlMsg::RemoteRequest(r))] if *p == deler && r.want
        ));
        assert_eq!(acties.len(), 1, "het venster hoort het");
        assert_eq!(s.bedienstand(deler, 3), Bedienstand::Gevraagd);

        // Een ja van een ander dan de eigenaar telt niet.
        assert!(s.bij_bericht(ander, ip(), &stand(3, true)).1.is_empty());
        assert_eq!(s.bedienstand(deler, 3), Bedienstand::Gevraagd);
        let (_, acties) = s.bij_bericht(deler, ip(), &stand(3, true));
        assert_eq!(
            acties,
            vec![Actie::Bediening {
                eigenaar: deler,
                stream_id: 3
            }]
        );
        assert_eq!(s.bedienstand(deler, 3), Bedienstand::Actief);

        // Nu gaan invoer en klembord weg, naar de deler, en zijn klembord komt binnen.
        let (cmds, _) = s.kijker_bedient(deler, 3, invoer(3, vec![a_omlaag()]));
        assert!(matches!(
            verzonden(&cmds).as_slice(),
            [(p, ControlMsg::RemoteInput(_))] if *p == deler
        ));
        let (_, acties) = s.bij_bericht(deler, ip(), &klembord(3, "van de deler"));
        assert!(matches!(
            acties.as_slice(),
            [Actie::Klembord { eigenaar: Some(p), .. }] if *p == deler
        ));
        assert!(s.bij_bericht(ander, ip(), &klembord(3, "x")).1.is_empty());

        // De deler zet de optie uit: wij staan uit, en er gaat niets meer heen.
        let (_, acties) = s.bij_bericht(deler, ip(), &aankondiging_met_bedienen(3, false));
        assert!(acties.contains(&Actie::Bediening {
            eigenaar: deler,
            stream_id: 3
        }));
        assert_eq!(s.bedienstand(deler, 3), Bedienstand::Uit);
        let (cmds, _) = s.kijker_bedient(deler, 3, invoer(3, vec![a_omlaag()]));
        assert!(cmds.is_empty());
        assert!(s.kijker_bedient(deler, 3, vraag(3, true)).0.is_empty());
    }

    #[test]
    fn als_kijker_brengen_nee_en_zelf_stoppen_ons_terug() {
        let deler = PeerId::new_random();
        let mut s = kijker_bij(deler, 3);
        s.kijker_bedient(deler, 3, vraag(3, true));
        s.bij_bericht(deler, ip(), &stand(3, false));
        assert_eq!(s.bedienstand(deler, 3), Bedienstand::Mag, "geweigerd");

        s.kijker_bedient(deler, 3, vraag(3, true));
        s.bij_bericht(deler, ip(), &stand(3, true));
        let (cmds, _) = s.kijker_bedient(deler, 3, vraag(3, false));
        assert!(matches!(
            verzonden(&cmds).as_slice(),
            [(_, ControlMsg::RemoteRequest(r))] if !r.want
        ));
        assert_eq!(s.bedienstand(deler, 3), Bedienstand::Mag);

        // Een gedeeld venster kan het nooit, ook als de deler het beweert.
        let mut v = Streams::new();
        v.bij_bericht(
            deler,
            ip(),
            &ControlMsg::StreamAnnounce(StreamAnnounce {
                stream_id: 4,
                kind: StreamKind::WINDOW,
                title: "Kladblok".into(),
                width: 800,
                height: 600,
                drawing: false,
                remote_control: true,
            }),
        );
        assert_eq!(v.bedienstand(deler, 4), Bedienstand::Uit);
    }

    #[test]
    fn klembord_bij_hetzelfde_stream_id_in_twee_richtingen() {
        // a bedient ons scherm en wij kijken naar zijn scherm met hetzelfde id. Zijn klembord
        // geldt eerst als "voor ons scherm"; beide wegen zetten hetzelfde klembord van deze pc.
        let (mut s, id, a, _) = a_bedient();
        s.bij_bericht(a, ip(), &aankondiging_met_bedienen(id, true));
        s.wil_kijken(a, id);
        let (_, acties) = s.bij_bericht(a, ip(), &klembord(id, "tekst"));
        assert!(matches!(
            acties.as_slice(),
            [Actie::Klembord { eigenaar: None, .. }]
        ));
    }
}
