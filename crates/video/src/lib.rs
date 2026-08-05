//! Screenshare: opnemen, coderen, versturen, ontvangen, decoderen, tonen.
//!
//! # Twee backends, één API
//!
//! De platformgebonden modules (`capture`, `codec`, `d3d`, `venster`) bestaan twee
//! keer: de Windows-bestanden hier, de macOS-tegenhangers onder `mac/`. Ze houden
//! dezelfde modulenamen en dezelfde publieke API, gekozen met `#[cfg]` — geen traits,
//! geen wrappers. Het ene type dat door de gedeelde code stroomt is `d3d::Beeld`
//! (Windows: `ID3D11Texture2D`, macOS: een `CVPixelBuffer`-houder), waardoor
//! `deler.rs`, `kijker.rs` en `fragment.rs` op beide platforms ongewijzigd compileren.
//! `kleur` en `mf` zijn interne details van de Windows-codec en hebben geen
//! mac-tegenhanger: VideoToolbox levert rechtstreeks BGRA en heeft geen bootstrap.

#[cfg(windows)]
pub mod capture;
#[cfg(target_os = "macos")]
#[path = "mac/capture.rs"]
pub mod capture;

#[cfg(windows)]
pub mod codec;
#[cfg(target_os = "macos")]
#[path = "mac/codec.rs"]
pub mod codec;

#[cfg(windows)]
pub mod d3d;
#[cfg(target_os = "macos")]
#[path = "mac/d3d.rs"]
pub mod d3d;

#[cfg(windows)]
pub mod venster;
#[cfg(target_os = "macos")]
#[path = "mac/venster.rs"]
pub mod venster;

pub mod deler;
pub mod fragment;
pub mod kijker;
#[cfg(windows)]
pub mod kleur;
#[cfg(windows)]
pub mod mf;
pub mod spoor;

pub use capture::{beschikbare_bronnen, Bron, BronSoort};
pub use codec::{Codec, Decoder, Encoder, EncoderConfig};
pub use d3d::D3dContext;
pub use deler::{deel, DelerConfig, DelerHandle};
pub use fragment::{Frame, Reassembler};
pub use kijker::{kijk, KijkerConfig, KijkerEvent, KijkerHandle, Miniatuur};
