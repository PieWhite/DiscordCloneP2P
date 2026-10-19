//! Netwerklaag: QUIC-mesh over het tailnet.
//!
//! Zie `docs/ARCHITECTURE.md`. Tailscale doet de bereikbaarheid; wie er aan de andere kant
//! zit bewijst de handshake zelf (beslissing 46). Deze crate gaat over verbindingen
//! opzetten, in stand houden en herstellen.

pub mod bereik;
pub mod filestream;
pub mod framing;
pub mod media;
pub mod mediasleutel;
pub mod mesh;
mod tls;
pub mod verdeler;

pub use media::{MediaSocket, PeerAdres, MAX_PAKKET};
pub use mediasleutel::MediaSleutels;
pub use mesh::{
    mag_leren, spawn, wegwerpsleutel, MeshCommand, MeshConfig, MeshEvent, MeshHandle, MeshSluiter,
    PeerStatus, PeerTarget,
};
pub use verdeler::{Abonnement, Pakket, Verdeler};
// De ruwe quinn-streamtypes, zodat `fitcom-app` een bestandsoverdracht kan lezen/schrijven
// zonder zelf een directe afhankelijkheid op `quinn` te hoeven opnemen.
pub use quinn::{RecvStream, SendStream};
