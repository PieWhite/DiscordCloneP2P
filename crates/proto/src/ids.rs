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
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Channel {
    tag: u8,
    peer: Option<PeerId>,
    /// Alleen gezet bij `tag == 2`. Eigen veld naast `peer` in plaats van hergebruik:
    /// een subkanaal-id is geen peer, en een nieuw, additief veld met `#[serde(default)]`
    /// kost een oudere peer niets — die decodeert een `Channel` gewoon zonder dit veld.
    #[serde(default)]
    topic: Option<TopicId>,
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
}
