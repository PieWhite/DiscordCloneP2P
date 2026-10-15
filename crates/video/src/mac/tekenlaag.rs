//! macOS: geen laag over het gedeelde scherm (beslissing 45). Een doorklikbaar venster
//! boven alles is daar een eigen bouwwerk (NSWindow op `.screenSaver`-niveau, buiten de
//! ScreenCaptureKit-opname gehouden) en is niet gebouwd. De deler ziet de lijnen op zijn
//! eigen tegel in de app; de kijkers zien ze gewoon.

use fitcom_proto::teken::Tekening;
use std::sync::Arc;

pub struct Deellaag;

impl Deellaag {
    pub fn start(_monitor: isize) -> anyhow::Result<Self> {
        Ok(Self)
    }

    pub fn toon(&self, _t: Arc<Tekening>) {}
}
