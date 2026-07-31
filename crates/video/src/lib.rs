//! Screenshare: opnemen, coderen, versturen, ontvangen, decoderen, tonen.

pub mod capture;
pub mod codec;
pub mod d3d;
pub mod deler;
pub mod fragment;
pub mod kijker;
pub mod kleur;
pub mod mf;
pub mod venster;

pub use capture::{beschikbare_bronnen, Bron, BronSoort};
pub use codec::{Codec, Decoder, Encoder, EncoderConfig};
pub use d3d::D3dContext;
pub use deler::{deel, haalbaar_tempo, DelerConfig, DelerHandle};
pub use fragment::{Frame, Reassembler};
pub use kijker::{kijk, KijkerConfig, KijkerEvent, KijkerHandle, Miniatuur};
