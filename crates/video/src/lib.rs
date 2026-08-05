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
//!
//! # Camera
//!
//! `camera` bestaat alleen op Windows (Media Foundation). Een camera is naar buiten toe
//! gewoon een derde `BronSoort`, dus hij loopt door dezelfde encoder, fragmentatie en
//! kijker als een gedeeld scherm. Op macOS is het *opnemen* bewust niet gebouwd — zie
//! `TODO.md` — maar *kijken* naar de camera van een Windows-peer werkt daar wel: op de
//! draad is dat niet van een gedeeld scherm te onderscheiden.

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

/// Webcam-opname. Alleen Windows; op macOS listet `capture` geen camera's en weigert
/// `Capture::start` er een te openen.
#[cfg(windows)]
pub mod camera;
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
