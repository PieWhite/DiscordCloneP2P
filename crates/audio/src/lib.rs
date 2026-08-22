//! Voice chat: opnemen, coderen, versturen, ontvangen, mixen, afspelen.

pub mod codec;
pub mod jitter;
#[cfg(windows)]
pub mod loopback;
pub mod mix;
pub mod session;

pub use jitter::{Frame, JitterBuffer};
pub use mix::{FRAME_SAMPLES, SAMPLE_RATE};
pub use session::{start, Bron, Niveaus, PeerAdres, VoiceConfig, VoiceHandle};
