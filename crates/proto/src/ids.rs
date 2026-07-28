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

/// Globaal unieke identiteit van één operatie.
///
/// `seq` is per auteur dicht (1, 2, 3, ... zonder gaten) omdat alleen de auteur zelf
/// zijn eigen ops nummert. Dat is precies de eigenschap waar de version-vector-sync op
/// leunt: "ik heb tot en met seq N van jou" impliceert dan dat er niets tussenin mist.
/// Ga hier nooit gaten introduceren.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct OpId {
    pub author: PeerId,
    pub seq: u64,
}

impl OpId {
    pub fn new(author: PeerId, seq: u64) -> Self {
        Self { author, seq }
    }
}

impl fmt::Debug for OpId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}#{}", self.author, self.seq)
    }
}
