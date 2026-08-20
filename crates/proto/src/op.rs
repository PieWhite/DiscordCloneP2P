//! De oplog: append-only, onveranderlijk, idempotent.
//!
//! Bewust niet chat-specifiek. Chat is nu de enige gebruiker, maar bijnamen,
//! instellingen en later filemetadata lopen over hetzelfde mechanisme mee.
//!
//! # Waarom de inhoud opaak is
//!
//! `Op` bewaart zijn soort als `kind_tag` + ongeïnterpreteerde `payload`, niet als
//! getypeerde enum. Dat is essentieel voor de gossip: een peer op een oudere versie
//! moet een op die hij níét begrijpt tóch kunnen opslaan en doorsturen naar de derde
//! peer. Zou hij hem laten vallen, dan convergeert de mesh niet meer zodra er
//! versieverschil is. Decoderen naar `OpKind` gebeurt pas bij het renderen.

use crate::{Channel, OpId, PeerId, TopicId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Hoogste `seq`/`lamport` die de opslag kan bewaren. SQLite kent geen `u64`, dus alles
/// erboven wordt als negatief getal opgeslagen — zie `ProtoError::GetalTeGroot`, B-14 en
/// B-34. Een eerlijke peer komt hier nooit in de buurt: dit zijn 9,2 × 10¹⁸ berichten.
pub const MAX_WIRE_GETAL: u64 = i64::MAX as u64;

/// Lengtegrenzen op de velden die van de draad komen (B-43). In **bytes**, niet in tekens:
/// het gaat om wat er aan geheugen en frameruimte omgaat, en dat is wat een aanvaller
/// stuurt. 4 KiB tekst is ruim 40 regels chat; een bestandsnaamcomponent kan op NTFS en
/// APFS toch al niet boven 255.
pub const MAX_BERICHT_LEN: usize = 4 * 1024;
pub const MAX_BESTANDSNAAM_LEN: usize = 255;
/// Voor een bijnaam en voor de titel van een subkanaal — beide korte labels in de UI.
pub const MAX_NAAM_LEN: usize = 64;
/// Het uitslagpatroon van een Wordle-dag: zes rijen van vijf tekens, geen scheidingsteken.
/// Zie `OpKind::WordleResult`.
pub const MAX_WORDLE_PATROON_LEN: usize = 30;

fn grens(veld: &'static str, waarde: &str, limiet: usize) -> crate::Result<()> {
    if waarde.len() > limiet {
        return Err(crate::ProtoError::VeldTeLang {
            veld,
            len: waarde.len(),
            limiet,
        });
    }
    Ok(())
}

/// Eén operatie in de log. Nooit muteren na aanmaak.
///
/// Het decoderen valideert (`#[serde(try_from)]`): `seq` en `lamport` moeten in een `i64`
/// passen (B-14, B-34) en de strings in de payload hebben een lengtegrens (B-43). Dit
/// verandert geen byte aan het wire-formaat — `OpWire` heeft exact dezelfde velden — maar
/// het zet een controle tussen de draad en het type. Een payload die niet te decoderen
/// valt, of een `kind_tag` die deze build niet kent, blijft bewust geen fout: die moet
/// opslaanbaar en doorstuurbaar blijven (zie de moduledoc hierboven).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "OpWire")]
pub struct Op {
    pub author: PeerId,
    /// Bepaalt wie deze op ooit mag zien. Staat, net als `author` en `seq`, altijd open
    /// leesbaar naast de opake `payload` — de gossip moet immers kunnen beslissen wie hem
    /// doorkrijgt zonder de soort te hoeven begrijpen.
    #[serde(default)]
    pub channel: Channel,
    /// Per (auteur, kanaal) monotoon en **dicht**: 1, 2, 3, ... zonder gaten.
    pub seq: u64,
    /// Voor totale ordening tussen auteurs.
    pub lamport: u64,
    /// Millis sinds epoch. Alleen voor weergave. Nooit voor correctheid gebruiken —
    /// de klokken van de drie PC's lopen uiteen.
    pub wall_clock: i64,
    pub kind_tag: u16,
    /// msgpack van de soort-specifieke velden. Opaak voor peers die de soort niet kennen.
    #[serde(with = "serde_bytes")]
    pub payload: Vec<u8>,
}

/// De rauwe vorm van `Op` op de draad: exact dezelfde velden en serde-attributen als `Op`
/// zelf, zodat de bytes identiek blijven. Zie de doc van `Op`.
#[derive(Deserialize)]
struct OpWire {
    author: PeerId,
    #[serde(default)]
    channel: Channel,
    seq: u64,
    lamport: u64,
    wall_clock: i64,
    kind_tag: u16,
    #[serde(with = "serde_bytes")]
    payload: Vec<u8>,
}

impl TryFrom<OpWire> for Op {
    type Error = crate::ProtoError;

    fn try_from(w: OpWire) -> crate::Result<Self> {
        let op = Self {
            author: w.author,
            channel: w.channel,
            seq: w.seq,
            lamport: w.lamport,
            wall_clock: w.wall_clock,
            kind_tag: w.kind_tag,
            payload: w.payload,
        };
        op.valideer_van_de_draad()?;
        Ok(op)
    }
}

impl Op {
    /// Wat een eerlijke peer nooit stuurt en de opslag niet kan bewaren. Zie B-14, B-34
    /// en B-43.
    fn valideer_van_de_draad(&self) -> crate::Result<()> {
        if self.seq > MAX_WIRE_GETAL {
            return Err(crate::ProtoError::GetalTeGroot {
                veld: "seq",
                waarde: self.seq,
            });
        }
        if self.lamport > MAX_WIRE_GETAL {
            return Err(crate::ProtoError::GetalTeGroot {
                veld: "lamport",
                waarde: self.lamport,
            });
        }
        // Een onbekende soort (`Ok(None)`) en een onleesbare payload (`Err`) blijven
        // allebei toegestaan — zonder dat kan een oudere peer een op van een nieuwere niet
        // meer doorgeven en convergeert de mesh niet meer. We begrenzen alleen wat we
        // begrijpen.
        if let Ok(Some(kind)) = self.kind() {
            kind.valideer_lengtes()?;
        }
        Ok(())
    }

    pub fn id(&self) -> OpId {
        OpId::new(self.author, self.channel, self.seq)
    }

    /// Weergavevolgorde. `author` breekt de gelijkstand zodat alle peers dezelfde
    /// volgorde tonen bij gelijke lamport-waarde.
    pub fn order_key(&self) -> (u64, PeerId) {
        (self.lamport, self.author)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        author: PeerId,
        channel: Channel,
        seq: u64,
        lamport: u64,
        wall_clock: i64,
        kind: &OpKind,
    ) -> crate::Result<Self> {
        // Ook bij het *maken* begrenzen, niet alleen bij het ontvangen: zou alleen de
        // ontvangkant weigeren, dan zou een te lang eigen bericht wel lokaal in de log
        // staan en bij iedereen anders geweigerd worden — stille divergentie in plaats
        // van een fout die de aanroeper ziet. Zie B-43.
        kind.valideer_lengtes()?;
        Ok(Self {
            author,
            channel,
            seq,
            lamport,
            wall_clock,
            kind_tag: kind.tag(),
            payload: kind.encode_payload()?,
        })
    }

    /// `Ok(None)` betekent: soort van een nieuwere peer. Opslaan en doorsturen,
    /// maar niet renderen.
    pub fn kind(&self) -> crate::Result<Option<OpKind>> {
        OpKind::decode(self.kind_tag, &self.payload)
    }
}

macro_rules! op_kinds {
    ($( $tag:literal => $variant:ident { $( $field:ident : $ty:ty ),* $(,)? } ),* $(,)?) => {
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub enum OpKind {
            $( $variant { $( $field: $ty, )* }, )*
        }

        /// Per soort een eigen struct, zodat velden per soort onafhankelijk mogen
        /// evolueren met `#[serde(default)]`.
        mod wire {
            use super::*;
            $(
                #[derive(Serialize, Deserialize)]
                pub struct $variant { $( pub $field: $ty, )* }
            )*
        }

        impl OpKind {
            pub fn tag(&self) -> u16 {
                match self { $( Self::$variant { .. } => $tag, )* }
            }

            pub fn encode_payload(&self) -> crate::Result<Vec<u8>> {
                let mut out = Vec::new();
                let mut ser = rmp_serde::Serializer::new(&mut out).with_struct_map();
                match self {
                    $( Self::$variant { $( $field, )* } => {
                        let w = wire::$variant { $( $field: $field.clone(), )* };
                        serde::Serialize::serialize(&w, &mut ser)?;
                    } )*
                }
                Ok(out)
            }

            pub fn decode(tag: u16, payload: &[u8]) -> crate::Result<Option<Self>> {
                Ok(match tag {
                    $( $tag => {
                        let w: wire::$variant = rmp_serde::from_slice(payload)?;
                        Some(Self::$variant { $( $field: w.$field, )* })
                    } )*
                    _ => None,
                })
            }
        }
    };
}

op_kinds! {
    1 => Post   { body: String },
    2 => Edit   { target: OpId, body: String },
    3 => Delete { target: OpId },
    4 => SetNick { name: String },
    // Wie aanbiedt staat niet als apart veld: dat is `op.author`, precies zoals Edit/Delete
    // hun eigenaarschap ook al via `op.author` regelen in plaats van een los veld dat uit de
    // pas zou kunnen lopen. Zie docs/ARCHITECTURE.md voor de rest van het ontwerp.
    10 => FileMeta { name: String, size: u64, hash: [u8; 32] },
    // 11-19 GERESERVEERD: React, Reply. Zie TODO.md.
    // Legt zowel het aanmaken (eerste keer gezien) als het hernoemen (latere keer) van een
    // subkanaal onder het algemene kanaal vast — laatste `(lamport, author)` wint, precies
    // zoals bij SetNick. Altijd op Channel::GENERAL geplaatst: dit is metadata over
    // kanalen zelf, geen gespreksinhoud van één specifiek kanaal. Zie fase 9 in
    // ROADMAP.md en docs/ARCHITECTURE.md, sectie "Kanalen".
    20 => SetTopicTitle { id: TopicId, title: String },
    // Verwijdert een subkanaal. Wint van een `SetTopicTitle` (of andersom) op dezelfde
    // `(lamport, author)`-vergelijking als Edit/Delete bij een bericht — een latere
    // hernoeming laat het subkanaal dus gewoon weer terugkomen. Geen aparte
    // auteurscheck: elke peer mag een subkanaal aanmaken/hernoemen, dus ook verwijderen.
    21 => DeleteTopic { id: TopicId },
    // De uitslag van één Wordle-dag (2026-08-20). Additief toegevoegd, geen protocolbump:
    // een oudere peer slaat hem op en stuurt hem door zonder hem te begrijpen.
    //
    // `day` is de *print_date* van het raadsel als `YYYYMMDD`, niet de dag waarop je hem
    // speelde en niet het raadselnummer van NYT. Die datum is de enige sleutel die alle
    // drie de peers zeker gelijk hebben: hij komt uit het antwoord van NYT en niet uit een
    // lokale klok, dus wie om 00:30 nog het raadsel van gisteren afmaakt scoort op de dag
    // waar het raadsel bij hoort. Een getal en geen string, want dan is er geen
    // lengtegrens nodig en blijft hij leesbaar in een dump.
    //
    // `guesses` is het aantal gedane pogingen (1 t/m 6), ook als het niet gelukt is;
    // `solved` zegt of de laatste poging het woord was. De uitslag is onveranderlijk: per
    // (auteur, dag) wint de *eerste* op, niet de laatste — zie `fitcom_store::timeline`.
    //
    // `pattern` is het gedeelde vierkantjesraster: vijf tekens per rij, `0` mis, `1` bijna,
    // `2` goed, rijen achter elkaar zonder scheidingsteken. Het gerade woord zelf gaat
    // nooit mee — dat zou het raadsel verklappen aan wie nog moet spelen, precies zoals
    // het echte Wordle alleen vierkantjes deelt.
    30 => WordleResult { day: u32, guesses: u8, solved: bool, pattern: String },
    // Nieuwe soorten toevoegen kost geen migratie — dat is het hele punt van deze opzet.
}

impl OpKind {
    /// Lengtegrenzen per soort (B-43). Geldt zowel bij het maken als bij het decoderen;
    /// zonder dit is `MAX_FRAME_LEN` (16 MiB) de enige rem op een berichttekst.
    ///
    /// Een nieuwe soort met een string erin hoort hier een regel bij te krijgen. Dat is
    /// bewust met de hand en niet via de macro: welke grens bij een veld past is een
    /// inhoudelijke keuze, geen mechanische.
    pub fn valideer_lengtes(&self) -> crate::Result<()> {
        match self {
            Self::Post { body } => grens("berichttekst", body, MAX_BERICHT_LEN),
            Self::Edit { body, .. } => grens("berichttekst", body, MAX_BERICHT_LEN),
            Self::Delete { .. } => Ok(()),
            Self::SetNick { name } => grens("bijnaam", name, MAX_NAAM_LEN),
            Self::FileMeta { name, .. } => grens("bestandsnaam", name, MAX_BESTANDSNAAM_LEN),
            Self::SetTopicTitle { title, .. } => grens("kanaaltitel", title, MAX_NAAM_LEN),
            Self::DeleteTopic { .. } => Ok(()),
            Self::WordleResult { pattern, .. } => {
                grens("wordle-patroon", pattern, MAX_WORDLE_PATROON_LEN)
            }
        }
    }
}

/// Eén regel van een `VersionVector` op de draad. Een benoemde struct in plaats van een
/// rauwe tuple: msgpack codeert een Rust-tuple altijd als vaste-lengte array, dus een
/// toekomstige extra veld zou — anders dan bij elke andere struct in dit protocol — geen
/// `#[serde(default)]` kunnen krijgen en een oudere peer zou de hele vector niet meer
/// kunnen decoderen. Met benoemde velden (als map, net als `Op`/`OpId`/`Hello`) kan dat
/// straks wél.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct VvEntry {
    author: PeerId,
    channel: Channel,
    seq: u64,
}

/// `{(auteur, kanaal) -> hoogste seq die ik heb}`.
///
/// Dit werkt alléén omdat `seq` per (auteur, kanaal) dicht is. Zodra er gaten in kunnen
/// zitten is één getal per sleutel niet meer genoeg en is dit hele mechanisme stuk. Zie
/// de moduledoc van `fitcom_store` voor waarom kanaal hier expliciet bij hoort en niet
/// per auteur alleen.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "Vec<VvEntry>", into = "Vec<VvEntry>")]
pub struct VersionVector(BTreeMap<(PeerId, Channel), u64>);

impl From<Vec<VvEntry>> for VersionVector {
    fn from(v: Vec<VvEntry>) -> Self {
        Self(
            v.into_iter()
                .map(|e| ((e.author, e.channel), e.seq))
                .collect(),
        )
    }
}

impl From<VersionVector> for Vec<VvEntry> {
    fn from(v: VersionVector) -> Self {
        v.0.into_iter()
            .map(|((author, channel), seq)| VvEntry {
                author,
                channel,
                seq,
            })
            .collect()
    }
}

impl VersionVector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Hoogste seq die we van deze auteur in dit kanaal hebben. 0 = niets.
    pub fn get(&self, author: PeerId, channel: Channel) -> u64 {
        self.0.get(&(author, channel)).copied().unwrap_or(0)
    }

    /// Alleen ophogen. Een op die we al hadden verandert hier niets — dat is de
    /// idempotentie waar de hele sync op leunt.
    pub fn observe(&mut self, author: PeerId, channel: Channel, seq: u64) {
        let e = self.0.entry((author, channel)).or_insert(0);
        *e = (*e).max(seq);
    }

    pub fn iter(&self) -> impl Iterator<Item = (PeerId, Channel, u64)> + '_ {
        self.0.iter().map(|(&(p, c), &s)| (p, c, s))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Alleen de sleutels die `viewer` ooit mag zien: het algemene kanaal en elk subkanaal
    /// daaronder (allebei publiek, zie `Channel::is_public`), een DM-kanaal gericht aan
    /// `viewer`, en (voor het geval `viewer` eigen data kwijtraakte) alles waarvan
    /// `viewer` zelf de auteur is. Gebruikt vóór we iets van deze vector naar `viewer`
    /// sturen of ermee vergelijken wat hij nog mist — zonder dit zou een DM tussen twee
    /// andere peers gewoon meegossipt worden naar wie het niet aangaat.
    pub fn visible_to(&self, viewer: PeerId) -> VersionVector {
        VersionVector(
            self.0
                .iter()
                .filter(|((author, channel), _)| {
                    channel.is_public() || *author == viewer || channel.dm_peer() == Some(viewer)
                })
                .map(|(&k, &v)| (k, v))
                .collect(),
        )
    }

    /// Welke seq-bereiken heeft `other` nog niet, die ik wel heb?
    /// Levert `(auteur, kanaal, van_seq, tot_seq)` inclusief aan beide kanten.
    ///
    /// Dit is de kern van de inhaalslag na offline zijn: één ronde, geen onderhandeling.
    pub fn ranges_missing_in(&self, other: &VersionVector) -> Vec<(PeerId, Channel, u64, u64)> {
        self.iter()
            .filter_map(|(author, channel, mine)| {
                let theirs = other.get(author, channel);
                (mine > theirs).then_some((author, channel, theirs + 1, mine))
            })
            .collect()
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
    fn op_roundtrip_per_soort() {
        let kinds = [
            OpKind::Post { body: "hoi".into() },
            OpKind::Edit {
                target: OpId::new(peer(1), Channel::GENERAL, 7),
                body: "aangepast".into(),
            },
            OpKind::Delete {
                target: OpId::new(peer(2), Channel::GENERAL, 3),
            },
            OpKind::SetNick {
                name: "Rick".into(),
            },
            OpKind::FileMeta {
                name: "vakantiefotos.zip".into(),
                size: 123_456_789,
                hash: [0x42; 32],
            },
            OpKind::SetTopicTitle {
                id: crate::TopicId::from_bytes([0x77; 16]),
                title: "project x".into(),
            },
            OpKind::WordleResult {
                day: 20_260_820,
                guesses: 4,
                solved: true,
                pattern: "000100120102201222222".into(),
            },
        ];
        for kind in kinds {
            let op = Op::new(peer(1), Channel::GENERAL, 1, 1, 0, &kind).unwrap();
            assert_eq!(op.kind().unwrap(), Some(kind));
        }
    }

    #[test]
    fn onbekende_opsoort_blijft_doorstuurbaar() {
        // Kern van de gossip: een op van een nieuwere peer moet je kunnen bewaren en
        // doorgeven aan de derde peer, ook zonder hem te begrijpen.
        let op = Op {
            author: peer(1),
            channel: Channel::GENERAL,
            seq: 1,
            lamport: 1,
            wall_clock: 0,
            kind_tag: 999,
            payload: vec![0x80],
        };
        assert_eq!(
            op.kind().unwrap(),
            None,
            "onbekende soort mag geen fout geven"
        );

        let msg = crate::ControlMsg::OpBroadcast(crate::control::OpBroadcast { op: op.clone() });
        let bytes = msg.encode().unwrap();
        match crate::ControlMsg::decode(&bytes).unwrap().unwrap() {
            crate::ControlMsg::OpBroadcast(b) => assert_eq!(b.op, op),
            other => panic!("verkeerde variant: {other:?}"),
        }
    }

    /// Een op zoals een aanvaller hem schrijft: velden zijn `pub`, dus Rust-code kan elke
    /// waarde neerzetten. De heenweg (`encode`) valideert niets — dat is precies wat de
    /// terugweg moet opvangen.
    fn rauwe_op(seq: u64, lamport: u64, kind_tag: u16, payload: Vec<u8>) -> Op {
        Op {
            author: peer(1),
            channel: Channel::GENERAL,
            seq,
            lamport,
            wall_clock: 0,
            kind_tag,
            payload,
        }
    }

    /// De reden waarom het decoderen faalde, als tekst. Serde kan een eigen foutsoort niet
    /// door een `Deserializer` heen dragen — `#[serde(try_from)]` wikkelt hem via
    /// `de::Error::custom` in de msgpack-fout — dus de reden staat in de melding en niet in
    /// de variant. Voor het logpad (`framing::read_frame` logt hem en gaat door) is dat
    /// genoeg; op de *maak*-kant blijft de nette variant wel bewaard.
    fn draad_fout(op: &Op) -> String {
        match over_de_draad(op) {
            Err(e) => e.to_string(),
            Ok(o) => panic!("had geweigerd moeten worden, kreeg {o:?}"),
        }
    }

    fn over_de_draad(op: &Op) -> crate::Result<Op> {
        let msg = crate::ControlMsg::OpBroadcast(crate::control::OpBroadcast { op: op.clone() });
        let bytes = msg.encode().unwrap();
        match crate::ControlMsg::decode(&bytes)? {
            Some(crate::ControlMsg::OpBroadcast(b)) => Ok(b.op),
            other => panic!("verkeerde variant: {other:?}"),
        }
    }

    #[test]
    fn b14_lamport_boven_i64_max_wordt_bij_het_decoderen_geweigerd() {
        // `u64::MAX` wordt in SQLite een `-1`, dus `MAX(lamport)` ziet hem nooit en de
        // eerlijke klok kan nooit meer inlopen — terwijl `timeline::build` in `u64`
        // vergelijkt en deze op dus élke last-writer-wins-vergelijking wint, voorgoed.
        let payload = OpKind::Post { body: "x".into() }.encode_payload().unwrap();
        let op = rauwe_op(1, u64::MAX, 1, payload);
        assert!(draad_fout(&op).contains("lamport"), "{}", draad_fout(&op));

        // De grenswaarde zelf mag nog wél: die is op te slaan.
        let net_goed = rauwe_op(1, MAX_WIRE_GETAL, 1, op.payload.clone());
        assert_eq!(over_de_draad(&net_goed).unwrap().lamport, MAX_WIRE_GETAL);
    }

    #[test]
    fn b34_seq_boven_i64_max_wordt_bij_het_decoderen_geweigerd() {
        // Zulke rijen zijn na opslag onbereikbaar voor `ops_range` en
        // `advance_contiguous` (die met positieve grenzen werken) maar tellen wel mee in
        // `op_count` en `all_ops()`: 2⁶³ permanent inerte sleutels.
        let op = rauwe_op(
            u64::MAX,
            1,
            1,
            OpKind::Post { body: "x".into() }.encode_payload().unwrap(),
        );
        assert!(draad_fout(&op).contains("seq"), "{}", draad_fout(&op));
    }

    #[test]
    fn b43_te_lange_velden_worden_geweigerd_bij_maken_en_bij_decoderen() {
        let te_lang = "a".repeat(MAX_BERICHT_LEN + 1);

        // Bij het maken, zodat een eigen te lang bericht niet stil bij de anderen
        // sneuvelt terwijl het lokaal wel in de log staat.
        assert!(matches!(
            Op::new(
                peer(1),
                Channel::GENERAL,
                1,
                1,
                0,
                &OpKind::Post {
                    body: te_lang.clone()
                }
            ),
            Err(crate::ProtoError::VeldTeLang {
                veld: "berichttekst",
                ..
            })
        ));

        // En bij het decoderen, want de afzender hoeft onze encoder niet te gebruiken.
        let payload = OpKind::Post { body: te_lang }.encode_payload().unwrap();
        let op = rauwe_op(1, 1, 1, payload);
        assert!(
            draad_fout(&op).contains("berichttekst"),
            "{}",
            draad_fout(&op)
        );

        for (kind, veld) in [
            (
                OpKind::SetNick {
                    name: "n".repeat(MAX_NAAM_LEN + 1),
                },
                "bijnaam",
            ),
            (
                OpKind::FileMeta {
                    name: "f".repeat(MAX_BESTANDSNAAM_LEN + 1),
                    size: 1,
                    hash: [0; 32],
                },
                "bestandsnaam",
            ),
            (
                OpKind::SetTopicTitle {
                    id: crate::TopicId::from_bytes([1; 16]),
                    title: "t".repeat(MAX_NAAM_LEN + 1),
                },
                "kanaaltitel",
            ),
            (
                OpKind::WordleResult {
                    day: 20_260_820,
                    guesses: 6,
                    solved: false,
                    pattern: "2".repeat(MAX_WORDLE_PATROON_LEN + 1),
                },
                "wordle-patroon",
            ),
        ] {
            let op = rauwe_op(1, 1, kind.tag(), kind.encode_payload().unwrap());
            let fout = draad_fout(&op);
            assert!(
                fout.contains(veld),
                "{veld} had geweigerd moeten worden: {fout}"
            );
        }
    }

    #[test]
    fn b43_normale_lengtes_blijven_gewoon_werken() {
        // De grens mag niets kosten aan echt gebruik: een lang-maar-normaal bericht, een
        // gewone bijnaam en een gewone bestandsnaam moeten er precies zo doorkomen.
        for kind in [
            OpKind::Post {
                body: "a".repeat(MAX_BERICHT_LEN),
            },
            OpKind::SetNick {
                name: "Rick".into(),
            },
            OpKind::FileMeta {
                name: "vakantiefotos.zip".into(),
                size: 1,
                hash: [0; 32],
            },
        ] {
            let op = Op::new(peer(1), Channel::GENERAL, 1, 1, 0, &kind).unwrap();
            assert_eq!(over_de_draad(&op).unwrap().kind().unwrap(), Some(kind));
        }
    }

    #[test]
    fn version_vector_observeert_alleen_omhoog() {
        let mut vv = VersionVector::new();
        vv.observe(peer(1), Channel::GENERAL, 5);
        vv.observe(peer(1), Channel::GENERAL, 3); // oude op nogmaals ontvangen
        assert_eq!(vv.get(peer(1), Channel::GENERAL), 5);
        assert_eq!(vv.get(peer(2), Channel::GENERAL), 0);
    }

    #[test]
    fn dm_en_algemeen_tellen_apart_ook_voor_dezelfde_auteur() {
        let mut vv = VersionVector::new();
        vv.observe(peer(1), Channel::GENERAL, 5);
        vv.observe(peer(1), Channel::dm(peer(2)), 2);
        assert_eq!(vv.get(peer(1), Channel::GENERAL), 5);
        assert_eq!(vv.get(peer(1), Channel::dm(peer(2))), 2);
    }

    #[test]
    fn ontbrekende_bereiken_na_lang_offline() {
        // A was online en maakte 100 ops; B stond uit na 10. C heeft niets van A.
        let mut a = VersionVector::new();
        a.observe(peer(1), Channel::GENERAL, 100);
        a.observe(peer(3), Channel::GENERAL, 4);

        let mut b = VersionVector::new();
        b.observe(peer(1), Channel::GENERAL, 10);
        b.observe(peer(3), Channel::GENERAL, 4);

        assert_eq!(
            a.ranges_missing_in(&b),
            vec![(peer(1), Channel::GENERAL, 11, 100)]
        );

        let leeg = VersionVector::new();
        let mut r = a.ranges_missing_in(&leeg);
        r.sort();
        assert_eq!(
            r,
            vec![
                (peer(1), Channel::GENERAL, 1, 100),
                (peer(3), Channel::GENERAL, 1, 4),
            ]
        );
    }

    #[test]
    fn gelijke_vectoren_vragen_niets_op() {
        let mut a = VersionVector::new();
        a.observe(peer(1), Channel::GENERAL, 7);
        assert!(a.ranges_missing_in(&a.clone()).is_empty());
    }

    #[test]
    fn version_vector_overleeft_de_draad() {
        let mut vv = VersionVector::new();
        vv.observe(peer(1), Channel::GENERAL, 9);
        vv.observe(peer(2), Channel::dm(peer(1)), 4);
        let msg = crate::ControlMsg::SyncRequest(crate::control::SyncRequest { have: vv.clone() });
        let bytes = msg.encode().unwrap();
        match crate::ControlMsg::decode(&bytes).unwrap().unwrap() {
            crate::ControlMsg::SyncRequest(r) => assert_eq!(r.have, vv),
            other => panic!("verkeerde variant: {other:?}"),
        }
    }

    #[test]
    fn visible_to_laat_algemeen_kanaal_altijd_door() {
        let mut vv = VersionVector::new();
        vv.observe(peer(1), Channel::GENERAL, 5);
        let gefilterd = vv.visible_to(peer(9));
        assert_eq!(gefilterd.get(peer(1), Channel::GENERAL), 5);
    }

    #[test]
    fn visible_to_laat_dm_alleen_aan_de_geadresseerde_zien() {
        let mut vv = VersionVector::new();
        // Peer 1 z'n DM aan peer 2.
        vv.observe(peer(1), Channel::dm(peer(2)), 3);

        assert_eq!(vv.visible_to(peer(2)).get(peer(1), Channel::dm(peer(2))), 3);
        assert!(
            vv.visible_to(peer(3)).is_empty(),
            "peer 3 mag deze DM niet zien"
        );
    }

    #[test]
    fn visible_to_laat_eigen_auteurschap_altijd_terug_naar_de_auteur() {
        // Zodat een peer die eigen data kwijtraakte zijn eigen DM's kan terugkrijgen van
        // de ander in het gesprek.
        let mut vv = VersionVector::new();
        vv.observe(peer(1), Channel::dm(peer(2)), 3);
        assert_eq!(vv.visible_to(peer(1)).get(peer(1), Channel::dm(peer(2))), 3);
    }

    #[test]
    fn visible_to_laat_een_subkanaal_net_als_algemeen_altijd_door() {
        // Een subkanaal onder "Algemeen" is net zo publiek als het hoofdkanaal zelf —
        // geen uitzondering zoals bij een DM.
        let topic = Channel::topic(crate::TopicId::from_bytes([0x99; 16]));
        let mut vv = VersionVector::new();
        vv.observe(peer(1), topic, 5);
        assert_eq!(vv.visible_to(peer(9)).get(peer(1), topic), 5);
    }

    #[test]
    fn dm_en_subkanaal_tellen_apart_ook_voor_dezelfde_auteur() {
        let topic = Channel::topic(crate::TopicId::from_bytes([0x11; 16]));
        let mut vv = VersionVector::new();
        vv.observe(peer(1), Channel::GENERAL, 5);
        vv.observe(peer(1), Channel::dm(peer(2)), 2);
        vv.observe(peer(1), topic, 9);
        assert_eq!(vv.get(peer(1), Channel::GENERAL), 5);
        assert_eq!(vv.get(peer(1), Channel::dm(peer(2))), 2);
        assert_eq!(vv.get(peer(1), topic), 9);
    }
}
