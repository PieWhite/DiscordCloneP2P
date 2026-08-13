use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

/// Stabiele identiteit van een peer. Wordt eenmalig gegenereerd bij eerste start en
/// daarna nooit meer gewijzigd — de displaynaam is het cosmetische deel, dit niet.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PeerId(pub Uuid);

impl PeerId {
    pub fn new_random() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn as_bytes(&self) -> &[u8; 16] {
        self.0.as_bytes()
    }

    pub fn from_bytes(b: [u8; 16]) -> Self {
        Self(Uuid::from_bytes(b))
    }
}

impl fmt::Display for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Korte vorm voor logs — volledige UUID's maken logregels onleesbaar.
impl fmt::Debug for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = self.0.simple().to_string();
        write!(f, "peer:{}", &s[..8])
    }
}

impl std::str::FromStr for PeerId {
    type Err = uuid::Error;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

/// Identiteit van een subkanaal onder het algemene kanaal (fase 9), bijvoorbeeld
/// "algemeen" en "project X" naast het hoofdkanaal. Los type van `PeerId` ondanks
/// dezelfde onderliggende `Uuid`: een subkanaal-id is nooit een peer en andersom, en
/// een los type voorkomt dat die twee per ongeluk verwisseld worden.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TopicId(pub Uuid);

impl TopicId {
    pub fn new_random() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn as_bytes(&self) -> &[u8; 16] {
        self.0.as_bytes()
    }

    pub fn from_bytes(b: [u8; 16]) -> Self {
        Self(Uuid::from_bytes(b))
    }
}

impl fmt::Debug for TopicId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = self.0.simple().to_string();
        write!(f, "topic:{}", &s[..8])
    }
}

/// Welk kanaal een op bij hoort: het algemene kanaal (dat iedereen ziet), een subkanaal
/// daaronder (ook door iedereen te zien, zie `is_public`), of een direct bericht tussen de
/// auteur van de op en één andere peer.
///
/// Als `(tag, peer, topic)` op de draad, niet als een gewone Rust-enum: zo geeft een later
/// toegevoegde kanaalsoort (bijvoorbeeld een groepskanaal) geen decodeerfout bij een peer
/// die hem nog niet kent, net als `StreamKind` en `FileOutcome`. Een onbekende tag valt
/// terug op "niet algemeen/publiek en niet aan mij gericht" — dus nooit doorsturen naar wie
/// het niet aangaat. Zie `docs/ARCHITECTURE.md`, sectie "Kanalen".
///
/// # Waarom het decoderen valideert (B-08)
///
/// De velden zijn privé, dus *Rust*-code kan geen inconsistente waarde bouwen — serde wel,
/// en een aanvaller schrijft de msgpack zelf. `{tag:1, peer:null}` en `{tag:0, peer:<uuid>}`
/// decodeerden schoon en aliasten daarna in `channel_to_blob` allemaal op de opslagsleutel
/// van het *algemene* kanaal: een botsing op de primary key `(author, kanaal, seq)` met een
/// echte algemene op van dezelfde auteur, dus permanent dataverlies. Vandaar
/// `#[serde(try_from)]`: geen enkele eerlijke encoder produceert die vormen, dus weigeren
/// kost niets en de bekende tags 0, 1 en 2 decoderen onveranderd.
///
/// Een tag die *deze* build niet kent is bewust géén fout (uitbreidingsregel: onbekend
/// loggen en negeren, niet weigeren), maar wordt genormaliseerd naar één vorm per tag —
/// `peer` en `topic` eruit. Zonder dat zouden `{tag:3, peer:X}` en `{tag:3}` ongelijk
/// vergelijken in Rust terwijl ze op dezelfde opslagsleutel landen, en dan is de aliasing
/// van B-08 er nog steeds, alleen een tag verderop.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "ChannelWire")]
pub struct Channel {
    tag: u8,
    peer: Option<PeerId>,
    /// Alleen gezet bij `tag == 2`. Eigen veld naast `peer` in plaats van hergebruik:
    /// een subkanaal-id is geen peer, en een nieuw, additief veld met `#[serde(default)]`
    /// kost een oudere peer niets — die decodeert een `Channel` gewoon zonder dit veld.
    #[serde(default)]
    topic: Option<TopicId>,
}

/// De rauwe vorm van `Channel` op de draad. Exact dezelfde veldnamen en
/// `#[serde(default)]` als `Channel` zelf — dit verandert geen byte aan het
/// wire-formaat, het schuift alleen een controle tussen draad en type. Zie B-08.
#[derive(Deserialize)]
struct ChannelWire {
    tag: u8,
    peer: Option<PeerId>,
    #[serde(default)]
    topic: Option<TopicId>,
}

/// Een kanaalvorm die geen eerlijke encoder produceert. Zie B-08.
#[derive(Debug)]
pub struct KanaalFout(&'static str);

impl fmt::Display for KanaalFout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "misvormd kanaal: {}", self.0)
    }
}

impl std::error::Error for KanaalFout {}

impl TryFrom<ChannelWire> for Channel {
    type Error = KanaalFout;

    fn try_from(w: ChannelWire) -> std::result::Result<Self, Self::Error> {
        match (w.tag, w.peer, w.topic) {
            (0, None, None) => Ok(Self::GENERAL),
            (0, ..) => Err(KanaalFout("algemeen kanaal met peer of subkanaal-id")),
            (1, Some(p), None) => Ok(Self::dm(p)),
            (1, None, _) => Err(KanaalFout("DM zonder peer")),
            (1, ..) => Err(KanaalFout("DM met subkanaal-id")),
            (2, None, Some(t)) => Ok(Self::topic(t)),
            (2, _, None) => Err(KanaalFout("subkanaal zonder id")),
            (2, ..) => Err(KanaalFout("subkanaal met peer")),
            // Kanaalsoort van een nieuwere peer: geen fout, wel genormaliseerd.
            (tag, ..) => Ok(Self::onbekend(tag)),
        }
    }
}

impl Channel {
    pub const GENERAL: Self = Self {
        tag: 0,
        peer: None,
        topic: None,
    };

    /// Direct bericht tussen de auteur van de op en `other`.
    pub fn dm(other: PeerId) -> Self {
        Self {
            tag: 1,
            peer: Some(other),
            topic: None,
        }
    }

    /// Een naamgevbaar subkanaal onder het algemene kanaal — zichtbaar voor iedereen,
    /// net als het algemene kanaal zelf (zie `is_public`), met een eigen berichten- en
    /// bestandenstroom. Zie `docs/ARCHITECTURE.md`, sectie "Kanalen".
    pub fn topic(id: TopicId) -> Self {
        Self {
            tag: 2,
            peer: None,
            topic: Some(id),
        }
    }

    /// De genormaliseerde vorm van een kanaalsoort die deze build niet kent: alleen de
    /// tag, zonder `peer` of `topic`. Zo krijgt elke onbekende soort zijn eigen
    /// opslagsleutelruimte in plaats van te botsen met het algemene kanaal (B-08), en
    /// vergelijken twee gelijk-ogende waarden ook echt gelijk.
    ///
    /// Alleen bedoeld voor tags die deze build niet kent (op dit moment: 3 en hoger) en
    /// voor het teruglezen daarvan uit de opslag. Voor 0, 1 en 2 zijn er `GENERAL`, `dm`
    /// en `topic`.
    pub fn onbekend(tag: u8) -> Self {
        Self {
            tag,
            peer: None,
            topic: None,
        }
    }

    /// De rauwe tag, ook als deze build hem niet kent. Nodig voor de opslagencoder: die
    /// moet totaal en injectief zijn over álles wat van de draad kan komen, anders aliast
    /// een onbekende soort op het algemene kanaal (B-08).
    pub fn raw_tag(&self) -> u8 {
        self.tag
    }

    pub fn is_general(&self) -> bool {
        self.tag == 0
    }

    /// De ander in dit DM-kanaal, als dit er een is (en we de tag kennen).
    pub fn dm_peer(&self) -> Option<PeerId> {
        (self.tag == 1).then_some(self.peer).flatten()
    }

    /// Het subkanaal waar dit bij hoort, als dit er een is.
    pub fn topic_id(&self) -> Option<TopicId> {
        (self.tag == 2).then_some(self.topic).flatten()
    }

    /// Zichtbaar voor iedereen en met dezelfde doorstuur-/hersync-robuustheid als het
    /// algemene kanaal: dat geldt voor het algemene kanaal zelf én voor elk subkanaal
    /// daaronder. Alleen een DM is hiervan uitgezonderd — zie "Kanalen" in
    /// `docs/ARCHITECTURE.md` voor waarom.
    pub fn is_public(&self) -> bool {
        self.tag == 0 || self.tag == 2
    }
}

impl Default for Channel {
    fn default() -> Self {
        Self::GENERAL
    }
}

impl fmt::Debug for Channel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.tag, self.peer, self.topic) {
            (0, ..) => write!(f, "General"),
            (1, Some(p), _) => write!(f, "Dm({p:?})"),
            (2, _, Some(t)) => write!(f, "Topic({t:?})"),
            (tag, ..) => write!(f, "Onbekend({tag})"),
        }
    }
}

/// Globaal unieke identiteit van één operatie.
///
/// `seq` is per **(auteur, kanaal)** dicht (1, 2, 3, ... zonder gaten) omdat alleen de
/// auteur zelf zijn eigen ops nummert, per kanaal apart geteld. Dat laatste is nodig
/// zodra ops een deel van de peers niet mogen bereiken (DM's): telde `seq` per auteur
/// over alle kanalen heen, dan zou een peer die een DM nooit mag ontvangen daar een
/// permanent gat overhouden — en zijn aaneengesloten reeks voor die auteur zou überhaupt
/// nooit meer verder komen dan dat gat, óók voor latere algemene berichten. Zie
/// `crates/store/src/lib.rs` voor de uitwerking.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct OpId {
    pub author: PeerId,
    #[serde(default)]
    pub channel: Channel,
    pub seq: u64,
}

impl OpId {
    pub fn new(author: PeerId, channel: Channel, seq: u64) -> Self {
        Self {
            author,
            channel,
            seq,
        }
    }
}

impl fmt::Debug for OpId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.channel.is_general() {
            write!(f, "{:?}#{}", self.author, self.seq)
        } else {
            write!(f, "{:?}/{:?}#{}", self.author, self.channel, self.seq)
        }
    }
}

#[cfg(test)]
mod channel_tests {
    use super::*;

    fn peer(n: u8) -> PeerId {
        let mut b = [0u8; 16];
        b[0] = n;
        PeerId::from_bytes(b)
    }

    fn topic(n: u8) -> TopicId {
        TopicId::from_bytes([n; 16])
    }

    #[test]
    fn algemeen_is_general_en_publiek_zonder_peer_of_topic() {
        assert!(Channel::GENERAL.is_general());
        assert!(Channel::GENERAL.is_public());
        assert_eq!(Channel::GENERAL.dm_peer(), None);
        assert_eq!(Channel::GENERAL.topic_id(), None);
    }

    #[test]
    fn dm_is_niet_general_en_niet_publiek() {
        let dm = Channel::dm(peer(1));
        assert!(!dm.is_general());
        assert!(!dm.is_public());
        assert_eq!(dm.dm_peer(), Some(peer(1)));
        assert_eq!(dm.topic_id(), None);
    }

    #[test]
    fn subkanaal_is_niet_general_maar_wel_publiek() {
        let t = Channel::topic(topic(1));
        assert!(!t.is_general());
        assert!(t.is_public());
        assert_eq!(t.topic_id(), Some(topic(1)));
        assert_eq!(t.dm_peer(), None);
    }

    #[test]
    fn twee_verschillende_subkanalen_zijn_ongelijk() {
        assert_ne!(Channel::topic(topic(1)), Channel::topic(topic(2)));
        assert_eq!(Channel::topic(topic(1)), Channel::topic(topic(1)));
    }

    /// Alles wat een eerlijke encoder produceert moet blijven decoderen — anders is de
    /// validatie van B-08 een protocolbreuk in plaats van een reparatie.
    #[test]
    fn b08_de_drie_bekende_tags_overleven_de_draad_onveranderd() {
        for c in [
            Channel::GENERAL,
            Channel::dm(peer(1)),
            Channel::topic(topic(3)),
        ] {
            let bytes = rmp_serde::to_vec_named(&c).unwrap();
            assert_eq!(rmp_serde::from_slice::<Channel>(&bytes).unwrap(), c);
        }
    }

    #[test]
    fn b08_misvormd_kanaal_is_een_decodeerfout_geen_algemeen_kanaal() {
        // Precies de vormen uit B-08: geen enkele eerlijke encoder maakt ze, en vóór de
        // fix decodeerden ze allemaal schoon om daarna op de opslagsleutel van het
        // algemene kanaal te aliassen.
        let dm_zonder_peer = rmp_serde::to_vec_named(&RauwKanaal {
            tag: 1,
            peer: None,
            topic: None,
        })
        .unwrap();
        assert!(rmp_serde::from_slice::<Channel>(&dm_zonder_peer).is_err());

        let algemeen_met_peer = rmp_serde::to_vec_named(&RauwKanaal {
            tag: 0,
            peer: Some(peer(1)),
            topic: None,
        })
        .unwrap();
        assert!(rmp_serde::from_slice::<Channel>(&algemeen_met_peer).is_err());

        let subkanaal_zonder_id = rmp_serde::to_vec_named(&RauwKanaal {
            tag: 2,
            peer: None,
            topic: None,
        })
        .unwrap();
        assert!(rmp_serde::from_slice::<Channel>(&subkanaal_zonder_id).is_err());
    }

    #[test]
    fn b08_onbekende_tag_blijft_decoderen_maar_genormaliseerd() {
        // Een kanaalsoort van een nieuwere peer mag geen decodeerfout geven (dat is de
        // uitbreidingsregel), maar moet wel één vorm per tag opleveren: zonder dat
        // vergelijken `{tag:3, peer:X}` en `{tag:3}` ongelijk terwijl ze op dezelfde
        // opslagsleutel landen — de aliasing van B-08, een tag verderop.
        let met_peer = rmp_serde::to_vec_named(&RauwKanaal {
            tag: 3,
            peer: Some(peer(1)),
            topic: None,
        })
        .unwrap();
        let zonder = rmp_serde::to_vec_named(&RauwKanaal {
            tag: 3,
            peer: None,
            topic: None,
        })
        .unwrap();

        let a: Channel = rmp_serde::from_slice(&met_peer).unwrap();
        let b: Channel = rmp_serde::from_slice(&zonder).unwrap();
        assert_eq!(a, b);
        assert_eq!(a, Channel::onbekend(3));
        assert_eq!(a.raw_tag(), 3);

        // En hij gedraagt zich als "gaat mij niet aan": niet publiek, niet aan mij gericht.
        assert!(!a.is_public());
        assert!(!a.is_general());
        assert_eq!(a.dm_peer(), None);
        assert_eq!(a.topic_id(), None);

        // Wel een eigen sleutelruimte, dus niet gelijk aan het algemene kanaal.
        assert_ne!(a, Channel::GENERAL);
        assert_ne!(a, Channel::onbekend(4));
    }

    // `RauwKanaal` is privé en heeft alleen `Deserialize`; voor de tests hierboven moeten
    // we hem juist wél kunnen *schrijven*, want dat is precies wat een aanvaller doet.
    #[derive(Serialize)]
    struct RauwKanaal {
        tag: u8,
        peer: Option<PeerId>,
        topic: Option<TopicId>,
    }
}
