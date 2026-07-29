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
pub use ids::{Channel, OpId, PeerId};
pub use media::{MediaHeader, PayloadType, MAX_MEDIA_PAYLOAD, MEDIA_HEADER_LEN, VOICE_STREAM_ID};
pub use op::{Op, OpKind, VersionVector};

/// Verhoog dit alleen bij een breuk die niet met `#[serde(default)]` of het negeren
/// van een onbekende tag op te vangen is. Peers draaien handmatig gekopieerde binaries,
/// dus een bump dwingt iedereen tot updaten voordat er weer iets werkt.
///
/// 1 → 2 bij de kanalen-uitbreiding (DM's): `VersionVector` en de bestandsoverdracht-
/// header (`crates/net/src/filestream.rs`) kregen er een `Channel` bij op een manier die
/// een oudere peer niet stilzwijgend kan negeren — een peer zonder kanaalbegrip kan een
/// per-(auteur, kanaal) version vector fundamenteel niet correct interpreteren, dus
/// stilletjes doorsyncen zou erger zijn dan een nette `VersionMismatch`. Zie
/// `docs/ARCHITECTURE.md`, sectie "Kanalen".
pub const PROTOCOL_VERSION: u32 = 2;

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
