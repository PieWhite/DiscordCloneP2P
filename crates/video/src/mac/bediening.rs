//! macOS: bedienen is niet gebouwd (beslissing 54). Een mac-deler kondigt `remote_control`
//! nooit aan, dus de motor start dit nooit; het bestaat zodat de gedeelde code niet open hoeft.
//! Bouwen zou `CGEventPost` zijn, met de Accessibility-toestemming erbij.

use fitcom_proto::control::InputEvent;

pub struct Bediener;

impl Bediener {
    pub fn start(
        _monitor: isize,
        _klembord_uit: impl Fn(String) + Send + 'static,
    ) -> anyhow::Result<Self> {
        anyhow::bail!("een scherm laten bedienen kan op macOS niet")
    }

    pub fn invoer(&self, _events: Vec<InputEvent>) {}

    pub fn klembord(&self, _tekst: String) {}
}
