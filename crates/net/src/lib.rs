//! Netwerklaag: QUIC-mesh over het tailnet.
//!
//! Zie `docs/ARCHITECTURE.md`. Tailscale doet de encryptie en de bereikbaarheid;
//! deze crate gaat alleen over verbindingen opzetten, in stand houden en herstellen.

pub mod framing;
pub mod mesh;
mod tls;

pub use mesh::{spawn, MeshCommand, MeshConfig, MeshEvent, MeshHandle, PeerStatus, PeerTarget};
