//! Wire-protocol voor FitCommunication.
//!
//! Deze crate is puur: geen I/O, geen Windows-API's, geen hardware. Dat is opzet —
//! hier zit de subtiele logica en die moet volledig unit-testbaar blijven.
//! Zie `docs/ARCHITECTURE.md` voor het ontwerp en de compatibiliteitsregels.

pub mod control;
pub mod ids;
pub mod media;
pub mod op;

pub use control::{ControlMsg, StreamKind};
pub use ids::{OpId, PeerId};
pub use media::{MediaHeader, PayloadType, MEDIA_HEADER_LEN};
pub use op::{Op, OpKind, VersionVector};

/// Verhoog dit alleen bij een breuk die niet met `#[serde(default)]` of het negeren
/// van een onbekende tag op te vangen is. Peers draaien handmatig gekopieerde binaries,
/// dus een bump dwingt iedereen tot updaten voordat er weer iets werkt.
pub const PROTOCOL_VERSION: u32 = 1;

/// Bovengrens voor een control-frame. Grote sync-antwoorden worden opgeknipt door de
/// netwerklaag, zodat we hier bij normaal gebruik nooit tegenaan lopen.
pub const MAX_FRAME_LEN: usize = 16 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum ProtoError {
    #[error("frame van {0} bytes overschrijdt de limiet van {MAX_FRAME_LEN}")]
    FrameTooLarge(usize),
    #[error("frame is te kort")]
    FrameTruncated,
    #[error("msgpack encode mislukt: {0}")]
    Encode(#[from] rmp_serde::encode::Error),
    #[error("msgpack decode mislukt: {0}")]
    Decode(#[from] rmp_serde::decode::Error),
}

pub type Result<T> = std::result::Result<T, ProtoError>;
