//! Voice chat: opnemen, coderen, versturen, ontvangen, mixen, afspelen.

pub mod codec;
pub mod jitter;
#[cfg(windows)]
pub mod loopback;
#[cfg(windows)]
pub mod microfoon;
pub mod mix;
pub mod session;
pub mod spotify;
pub mod spreektoets;

/// Eén stuk van een clip-tap: interleaved 48 kHz stereo, met het moment waarop het van het
/// apparaat gelezen werd.
#[cfg(windows)]
pub type TapStuk = (std::time::Instant, Vec<f32>);

pub use jitter::{Frame, JitterBuffer};
pub use mix::{FRAME_SAMPLES, SAMPLE_RATE};
pub use session::{start, Bron, Niveaus, PeerAdres, VoiceConfig, VoiceHandle};
pub use spreektoets::Modus as Spreekmodus;
