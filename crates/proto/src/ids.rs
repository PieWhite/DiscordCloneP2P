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

/// Welk kanaal een op bij hoort: het algemene kanaal, dat iedereen ziet, of een direct
/// bericht tussen de auteur van de op en één andere peer.
///
/// Als `(tag, peer)` op de draad, niet als een gewone Rust-enum: zo geeft een later
/// toegevoegde kanaalsoort (bijvoorbeeld een groepskanaal) geen decodeerfout bij een peer
/// die hem nog niet kent, net als `StreamKind` en `FileOutcome`. Een onbekende tag valt
/// terug op "niet algemeen en niet aan mij gericht" — dus nooit doorsturen naar wie het
/// niet aangaat. Zie `docs/ARCHITECTURE.md`, sectie "Kanalen".
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Channel {
    tag: u8,
    peer: Option<PeerId>,
}

impl Channel {
    pub const GENERAL: Self = Self { tag: 0, peer: None };

    /// Direct bericht tussen de auteur van de op en `other`.
    pub fn dm(other: PeerId) -> Self {
        Self {
            tag: 1,
            peer: Some(other),
        }
    }

    pub fn is_general(&self) -> bool {
        self.tag == 0
    }

    /// De ander in dit DM-kanaal, als dit er een is (en we de tag kennen).
    pub fn dm_peer(&self) -> Option<PeerId> {
        (self.tag == 1).then_some(self.peer).flatten()
    }
}

impl Default for Channel {
    fn default() -> Self {
        Self::GENERAL
    }
}

impl fmt::Debug for Channel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.tag, self.peer) {
            (0, _) => write!(f, "General"),
            (1, Some(p)) => write!(f, "Dm({p:?})"),
            (tag, _) => write!(f, "Onbekend({tag})"),
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
