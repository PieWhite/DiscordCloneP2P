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

use fitcom_net::{MeshCommand, PeerAdres};
use fitcom_proto::control::{
    Draw, DrawClear, DrawClearScope, DrawState, RequestKeyframe, StreamAnnounce, StreamKind,
    StreamRevoke, StreamSubscribe, StreamUnsubscribe,
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
        })
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
    /// Of een nieuw gedeeld scherm tekenen toestaat: de laatste stand van de schakelaar.
    /// Begint aan; de motor zet hem bij het starten uit de config.
    tekenen_standaard: bool,
}

impl Streams {
    pub fn new() -> Self {
        Self {
            volgende_id: 1,
            tekenen_standaard: true,
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
    pub fn deel(
        &mut self,
        kind: StreamKind,
        titel: String,
        breedte: u32,
        hoogte: u32,
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
            tekening: Arc::new(Tekening::nieuw(kind.is_scherm() && self.tekenen_standaard)),
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

    /// De schakelaar van de deler (`Config::allow_drawing`): mogen kijkers op zijn schermen
    /// tekenen. Geldt voor elk scherm dat loopt en elk dat hierna komt. Uit wist alles en
    /// zegt het iedereen via een nieuwe aankondiging, net als een nieuwe titel.
    pub fn zet_tekenen(&mut self, aan: bool) -> (Vec<MeshCommand>, Vec<Actie>) {
        self.tekenen_standaard = aan;
        let (mut cmds, mut acties) = (Vec::new(), Vec::new());
        for s in self.eigen.iter_mut().filter(|s| s.kind.is_scherm()) {
            if Arc::make_mut(&mut s.tekening).zet_aan(aan) {
                cmds.push(MeshCommand::Broadcast(s.aankondiging()));
                acties.push(Actie::Tekening {
                    eigenaar: None,
                    stream_id: s.id,
                });
            }
        }
        (cmds, acties)
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
            _ => (Vec::new(), Vec::new()),
        }
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

        // Opnieuw aankondigen is normaal: het gebeurt bij elke herverbinding. Alleen
        // de gegevens bijwerken, de kijkstatus laten staan.
        if let Some(s) = self.vind_vreemd(van, a.stream_id) {
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
                return vec![Actie::Tekening {
                    eigenaar: Some(van),
                    stream_id: a.stream_id,
                }];
            }
            return Vec::new();
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
        let (id, cmds) = s.deel(StreamKind::MONITOR, "Scherm".into(), 1920, 1080);
        assert_eq!(id, 1);
        assert_eq!(cmds.len(), 1, "alleen de aankondiging");
        assert!(!s.eigen()[0].wordt_bekeken());
    }

    #[test]
    fn eerste_kijker_start_het_opnemen_en_de_tweede_niet() {
        let mut s = Streams::new();
        let (id, _) = s.deel(StreamKind::MONITOR, "Scherm".into(), 1920, 1080);
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
        let (id, _) = s.deel(StreamKind::MONITOR, "Scherm".into(), 1920, 1080);
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
        let (id, _) = s.deel(StreamKind::MONITOR, "Scherm".into(), 1920, 1080);
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
        s.deel(StreamKind::MONITOR, "Scherm".into(), 1920, 1080);
        s.deel(StreamKind::WINDOW, "Kladblok".into(), 800, 600);

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
        let (id, _) = s.deel(StreamKind::MONITOR, "Scherm".into(), 1920, 1080);
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
        let (eerste, _) = s.deel(StreamKind::MONITOR, "A".into(), 1920, 1080);
        s.stop_delen(eerste);
        let (tweede, _) = s.deel(StreamKind::MONITOR, "B".into(), 1920, 1080);
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
        let (id, _) = s.deel(StreamKind::DESKTOP_AUDIO, "Bureaubladgeluid".into(), 0, 0);
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
        let (id, _) = s.deel(StreamKind::MONITOR, "Scherm".into(), 1920, 1080);
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
        let (id, _) = s.deel(StreamKind::MUSIC, "Spotify".into(), 0, 0);
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
        let (id, _) = s.deel(StreamKind::MONITOR, "Scherm".into(), 1920, 1080);
        let a = PeerId::new_random();
        let b = PeerId::new_random();
        s.bij_bericht(a, ip(), &intekening(id, 41000));
        s.bij_bericht(b, ip(), &intekening(id, 41001));
        (s, id, a, b)
    }

    #[test]
    fn een_scherm_kondigt_tekenen_aan_en_een_camera_niet() {
        let mut s = Streams::new();
        let (_, cmds) = s.deel(StreamKind::MONITOR, "Scherm".into(), 1920, 1080);
        let (_, cam) = s.deel(StreamKind::CAMERA, "Cam".into(), 1280, 720);
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

        s.zet_tekenen(false);
        let (cmds, _) = s.bij_bericht(
            a,
            ip(),
            &ControlMsg::Draw(lijn(id, 1, 0, &[[0, 0], [9, 9]])),
        );
        assert!(cmds.is_empty());
        assert!(s.eigen()[0].tekening.is_leeg());
    }

    #[test]
    fn uitzetten_wist_en_kondigt_opnieuw_aan_en_geldt_voor_het_volgende_scherm() {
        let (mut s, id, a, _) = deler_met_kijkers();
        s.bij_bericht(
            a,
            ip(),
            &ControlMsg::Draw(lijn(id, 1, 0, &[[0, 0], [9, 9]])),
        );

        let (cmds, acties) = s.zet_tekenen(false);
        assert!(matches!(
            cmds.as_slice(),
            [MeshCommand::Broadcast(ControlMsg::StreamAnnounce(an))] if !an.drawing
        ));
        assert_eq!(acties.len(), 1);
        assert!(s.eigen()[0].tekening.is_leeg());
        let (cmds, acties) = s.zet_tekenen(false);
        assert!(cmds.is_empty() && acties.is_empty(), "al uit");

        let (tweede, _) = s.deel(StreamKind::MONITOR, "Tweede".into(), 1920, 1080);
        assert!(
            !s.tekening(None, tweede).unwrap().aan,
            "de laatste stand geldt"
        );
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
        let (id, _) = d.deel(StreamKind::MONITOR, "Scherm".into(), 1920, 1080);
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
        let (id, _) = s.deel(StreamKind::MONITOR, "Scherm".into(), 1920, 1080);
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
}
