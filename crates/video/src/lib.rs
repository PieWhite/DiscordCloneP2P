//! Screenshare: opnemen, coderen, versturen, ontvangen, decoderen, tonen.

pub mod capture;
pub mod codec;
pub mod d3d;
pub mod fragment;
pub mod kleur;
pub mod mf;
pub mod venster;

pub use codec::{Codec, Decoder, Encoder, EncoderConfig};
pub use d3d::D3dContext;
pub use fragment::{Frame, Reassembler};
