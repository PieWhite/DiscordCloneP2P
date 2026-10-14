//! Push-to-talk en push-to-mute: één toets die de microfoonpoort bedient.
//!
//! # Waarom pollen en geen sneltoets of hook
//!
//! De clip-sneltoets (`crates/app/src/clips.rs`) is een `RegisterHotKey`. Dat is hier het
//! verkeerde model: het meldt alleen het *indrukken* (push-to-talk moet ook het loslaten
//! weten), het neemt de toets af van het spel dat vooraan staat, en een kale Rechter-Ctrl
//! is er niet mee te registreren. Een `WH_KEYBOARD_LL`-hook lost dat op, maar draait op
//! elke toetsaanslag van het hele systeem en legt zo een callback in het invoerpad van de
//! game: invariant 4 (gamen wint).
//!
//! In plaats daarvan vraagt de opname-thread eens per frame (20 ms) `GetAsyncKeyState`.
//! Die thread tikt toch al op elk microfoonblok, ook als de VAD dicht staat, dus er wordt
//! niets extra wakker en het spel krijgt de toets gewoon. Het kost ~50 aanroepen per
//! seconde, alleen tijdens een gesprek.
//!
//! Het zijn dezelfde virtuele toetscodes voor toetsenbord en muis: `VK_XBUTTON1/2` zijn de
//! zijknoppen, `VK_RCONTROL` is Rechter-Ctrl.
//!
//! # macOS
//!
//! Niet gebouwd (bewust): `BESCHIKBAAR` is daar `false`, `ingedrukt` is een stub, en de
//! sessie rekent de modus dan terug naar spraakdetectie. Een push-to-talk met een toets die
//! nooit "ingedrukt" is zou een permanente mute zijn.

/// Hoe de microfoonpoort bepaald wordt. Staat als `u8` in een atomic (`Modus::als_u8`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Modus {
    /// Spraakdetectie alleen, zoals het altijd was.
    #[default]
    Spraak,
    /// Toets ingedrukt = praten. Gaat vóór de VAD: zolang de toets ingedrukt is staat de
    /// poort open, ook voor zachte spraak die de VAD zou missen.
    PushToTalk,
    /// Toets ingedrukt = stil. Daarbuiten bepaalt de VAD.
    PushToMute,
}

impl Modus {
    /// De namen zoals ze in `config.toml` staan.
    pub const ALLE: [Modus; 3] = [Modus::Spraak, Modus::PushToTalk, Modus::PushToMute];

    pub fn naam(self) -> &'static str {
        match self {
            Modus::Spraak => "voice",
            Modus::PushToTalk => "talk",
            Modus::PushToMute => "mute",
        }
    }

    pub fn van_naam(naam: &str) -> Option<Modus> {
        Self::ALLE
            .into_iter()
            .find(|m| m.naam().eq_ignore_ascii_case(naam.trim()))
    }

    pub fn als_u8(self) -> u8 {
        match self {
            Modus::Spraak => 0,
            Modus::PushToTalk => 1,
            Modus::PushToMute => 2,
        }
    }

    pub fn van_u8(n: u8) -> Modus {
        match n {
            1 => Modus::PushToTalk,
            2 => Modus::PushToMute,
            _ => Modus::Spraak,
        }
    }
}

/// Of dit platform een globale toets kan lezen. Zo niet, dan blijft het bij spraakdetectie.
pub const BESCHIKBAAR: bool = cfg!(windows);

/// Zo lang blijft de poort open nadat je de toets loslaat in push-to-talk. Zonder dit knipt
/// het loslaten het einde van je laatste woord af (je laat de toets los op het moment dat
/// het woord nog uitklinkt). 200 ms is kort genoeg om niet als "open microfoon" te voelen
/// en ruim genoeg voor een uitlopende klank. Push-to-mute heeft bewust geen staart: stil
/// moet direct stil zijn.
pub const STAART_FRAMES: u32 = 10; // 200 ms

/// Een toets die gekozen kan worden: config-naam, wat de gebruiker leest, en de Windows
/// virtuele toetscode. `0` is niet gekozen.
pub struct Toets {
    pub naam: &'static str,
    pub label: &'static str,
    pub vk: u16,
}

const fn t(naam: &'static str, label: &'static str, vk: u16) -> Toets {
    Toets { naam, label, vk }
}

/// De toetsen die te kiezen zijn. Eén plek: config, motor en instellingenscherm lezen
/// allemaal deze tabel. De codes zijn de Windows `VK_*`-waarden; op macOS worden ze niet
/// gebruikt (zie `BESCHIKBAAR`).
pub const TOETSEN: &[Toets] = &[
    t("RightCtrl", "Right Ctrl", 0xA3),
    t("LeftCtrl", "Left Ctrl", 0xA2),
    t("RightShift", "Right Shift", 0xA1),
    t("LeftShift", "Left Shift", 0xA0),
    t("RightAlt", "Right Alt", 0xA5),
    t("LeftAlt", "Left Alt", 0xA4),
    t("CapsLock", "Caps Lock", 0x14),
    t("Backquote", "` (backtick)", 0xC0),
    t("Tab", "Tab", 0x09),
    t("Mouse3", "Mouse 3 (wheel click)", 0x04),
    t("Mouse4", "Mouse 4 (side, back)", 0x05),
    t("Mouse5", "Mouse 5 (side, forward)", 0x06),
    t("F1", "F1", 0x70),
    t("F2", "F2", 0x71),
    t("F3", "F3", 0x72),
    t("F4", "F4", 0x73),
    t("F5", "F5", 0x74),
    t("F6", "F6", 0x75),
    t("F7", "F7", 0x76),
    t("F8", "F8", 0x77),
    t("F9", "F9", 0x78),
    t("F10", "F10", 0x79),
    t("F11", "F11", 0x7A),
    t("F12", "F12", 0x7B),
    t("F13", "F13", 0x7C),
    t("F14", "F14", 0x7D),
    t("F15", "F15", 0x7E),
    t("F16", "F16", 0x7F),
    t("F17", "F17", 0x80),
    t("F18", "F18", 0x81),
    t("F19", "F19", 0x82),
    t("F20", "F20", 0x83),
    t("F21", "F21", 0x84),
    t("F22", "F22", 0x85),
    t("F23", "F23", 0x86),
    t("F24", "F24", 0x87),
    t("Insert", "Insert", 0x2D),
    t("Delete", "Delete", 0x2E),
    t("Home", "Home", 0x24),
    t("End", "End", 0x23),
    t("PageUp", "Page Up", 0x21),
    t("PageDown", "Page Down", 0x22),
    t("ScrollLock", "Scroll Lock", 0x91),
    t("Pause", "Pause", 0x13),
    t("Numpad0", "Numpad 0", 0x60),
    t("Numpad1", "Numpad 1", 0x61),
    t("Numpad2", "Numpad 2", 0x62),
    t("Numpad3", "Numpad 3", 0x63),
    t("Numpad4", "Numpad 4", 0x64),
    t("Numpad5", "Numpad 5", 0x65),
    t("Numpad6", "Numpad 6", 0x66),
    t("Numpad7", "Numpad 7", 0x67),
    t("Numpad8", "Numpad 8", 0x68),
    t("Numpad9", "Numpad 9", 0x69),
    t("A", "A", 0x41),
    t("B", "B", 0x42),
    t("C", "C", 0x43),
    t("D", "D", 0x44),
    t("E", "E", 0x45),
    t("F", "F", 0x46),
    t("G", "G", 0x47),
    t("H", "H", 0x48),
    t("I", "I", 0x49),
    t("J", "J", 0x4A),
    t("K", "K", 0x4B),
    t("L", "L", 0x4C),
    t("M", "M", 0x4D),
    t("N", "N", 0x4E),
    t("O", "O", 0x4F),
    t("P", "P", 0x50),
    t("Q", "Q", 0x51),
    t("R", "R", 0x52),
    t("S", "S", 0x53),
    t("T", "T", 0x54),
    t("U", "U", 0x55),
    t("V", "V", 0x56),
    t("W", "W", 0x57),
    t("X", "X", 0x58),
    t("Y", "Y", 0x59),
    t("Z", "Z", 0x5A),
];

/// De standaardtoets: Rechter-Ctrl, de toets die bijna geen spel bindt.
pub const STANDAARD_TOETS: &str = "RightCtrl";

/// Zoekt een toets op config-naam, zonder op hoofdletters te letten.
pub fn toets_van_naam(naam: &str) -> Option<&'static Toets> {
    let naam = naam.trim();
    TOETSEN.iter().find(|t| t.naam.eq_ignore_ascii_case(naam))
}

/// Of de toets nu fysiek omlaag staat. `vk == 0` (niet gekozen) is nooit ingedrukt.
///
/// Alleen de hoogste bit van `GetAsyncKeyState` telt: de laagste ("ingedrukt sinds de vorige
/// aanroep") wordt door elk ander programma dat dezelfde toets pollt mee gewist en is dus
/// onbetrouwbaar.
#[cfg(windows)]
pub fn ingedrukt(vk: u16) -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
    vk != 0 && (unsafe { GetAsyncKeyState(i32::from(vk)) } as u16 & 0x8000) != 0
}

#[cfg(not(windows))]
pub fn ingedrukt(_vk: u16) -> bool {
    false
}

/// De poort voor één frame: mag dit frame de deur uit?
///
/// De poort kent twee getallen die van frame tot frame meegaan: `staart` (frames die push-
/// to-talk nog openblijft na het loslaten) en wat de VAD zegt. Mute valt hier buiten: die
/// wint in de opname-lus boven elke modus.
#[derive(Debug, Default)]
pub struct Poort {
    staart: u32,
}

impl Poort {
    /// `toets`: staat de gekozen toets nu omlaag. `vad`: zegt de spraakdetectie (met haar
    /// eigen nahouden) dat er gepraat wordt.
    pub fn open(&mut self, modus: Modus, toets: bool, vad: bool) -> bool {
        match modus {
            Modus::Spraak => {
                self.staart = 0;
                vad
            }
            // De toets gaat vóór de VAD: een ingedrukte toets is een besluit van de
            // gebruiker, de VAD is een gok. Ook zachte spraak gaat dus mee.
            Modus::PushToTalk => {
                if toets {
                    self.staart = STAART_FRAMES;
                    return true;
                }
                let open = self.staart > 0;
                self.staart = self.staart.saturating_sub(1);
                open
            }
            Modus::PushToMute => {
                self.staart = 0;
                vad && !toets
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toetsnamen_zijn_uniek_en_vinden_zichzelf_terug() {
        for (i, a) in TOETSEN.iter().enumerate() {
            assert_ne!(a.vk, 0, "{} heeft geen toetscode", a.naam);
            assert_eq!(toets_van_naam(a.naam).map(|t| t.vk), Some(a.vk));
            for b in &TOETSEN[i + 1..] {
                assert!(!a.naam.eq_ignore_ascii_case(b.naam), "dubbel: {}", a.naam);
                assert_ne!(a.vk, b.vk, "{} en {} delen een toets", a.naam, b.naam);
                assert_ne!(a.label, b.label, "{} en {} delen een label", a.naam, b.naam);
            }
        }
        assert!(toets_van_naam(STANDAARD_TOETS).is_some());
        assert_eq!(toets_van_naam(" rightctrl ").map(|t| t.vk), Some(0xA3));
        assert!(toets_van_naam("Hyper").is_none());
        assert!(toets_van_naam("").is_none());
    }

    #[test]
    fn modus_overleeft_naam_en_getal() {
        for m in Modus::ALLE {
            assert_eq!(Modus::van_naam(m.naam()), Some(m));
            assert_eq!(Modus::van_u8(m.als_u8()), m);
        }
        assert_eq!(Modus::van_naam("TALK"), Some(Modus::PushToTalk));
        assert_eq!(Modus::van_naam("shout"), None);
        // Een onbekend getal valt terug op spraakdetectie, nooit op een poort die dicht blijft.
        assert_eq!(Modus::van_u8(200), Modus::Spraak);
    }

    #[test]
    fn spraakmodus_volgt_alleen_de_vad() {
        let mut p = Poort::default();
        assert!(p.open(Modus::Spraak, false, true));
        // Een ingedrukte toets doet in deze modus niets.
        assert!(p.open(Modus::Spraak, true, true));
        assert!(!p.open(Modus::Spraak, true, false));
    }

    #[test]
    fn push_to_talk_gaat_voor_de_vad_en_houdt_een_korte_staart() {
        let mut p = Poort::default();
        // Niets ingedrukt: dicht, ook al hoort de VAD spraak.
        assert!(!p.open(Modus::PushToTalk, false, true));
        // Ingedrukt: open, ook al hoort de VAD niets.
        assert!(p.open(Modus::PushToTalk, true, false));
        // Los: nog precies STAART_FRAMES frames open, dan dicht.
        for _ in 0..STAART_FRAMES {
            assert!(p.open(Modus::PushToTalk, false, false));
        }
        assert!(!p.open(Modus::PushToTalk, false, false));
        // Een nieuwe druk begint de staart opnieuw.
        assert!(p.open(Modus::PushToTalk, true, false));
        assert!(p.open(Modus::PushToTalk, false, false));
    }

    #[test]
    fn push_to_mute_is_direct_stil_en_laat_de_vad_verder_beslissen() {
        let mut p = Poort::default();
        assert!(p.open(Modus::PushToMute, false, true));
        assert!(!p.open(Modus::PushToMute, false, false));
        // Ingedrukt: meteen dicht, zonder staart.
        assert!(!p.open(Modus::PushToMute, true, true));
        assert!(p.open(Modus::PushToMute, false, true));
    }

    #[test]
    fn een_staart_van_push_to_talk_lekt_niet_naar_een_andere_modus() {
        let mut p = Poort::default();
        assert!(p.open(Modus::PushToTalk, true, false));
        // Wisselen naar spraak midden in de staart: de VAD beslist, niet de rest van de staart.
        assert!(!p.open(Modus::Spraak, false, false));
        assert!(!p.open(Modus::PushToTalk, false, false));
    }
}
