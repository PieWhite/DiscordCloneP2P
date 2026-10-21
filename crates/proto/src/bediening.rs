//! Een gedeeld scherm bedienen (2026-10-22, beslissing 54): de grenzen, wat geldige invoer
//! is, en de rem per bediener.
//!
//! Alles wat hier binnenkomt komt van een andere PC en gaat bij de deler als muis en
//! toetsenbord zijn systeem in. De deler laat alleen de ene kijker toe die zijn gebruiker met
//! een klik toeliet (`crates/app/src/streams.rs`); deze module zegt daarbovenop welke invoer
//! zinnig is en hoeveel. Puur, zodat deler en kijker dezelfde regels gebruiken en het te testen
//! is zonder muis of toetsenbord.

use crate::control::{InputEvent, InputKind};
use std::time::Instant;

/// Zoveel gebeurtenissen hooguit in één `RemoteInput`. Het kijkvenster stuurt elke 8 ms wat er
/// bij kwam, met de muisbewegingen tot één samengevat; dit raakt alleen wie heel snel typt
/// terwijl hij klikt, en dan volgt er gewoon een tweede bericht.
pub const MAX_INVOER_PER_BERICHT: usize = 64;

/// Zoveel bytes (UTF-8) hooguit op het klembord. Genoeg voor een stuk code of een logregel;
/// groter gaat niet over, niet afgekapt: een half geplakte tekst is erger dan geen.
pub const MAX_KLEMBORD: usize = 64 * 1024;

/// De rem op invoer per bediener: gemiddeld zoveel per seconde...
pub const INVOER_PER_SECONDE: f64 = 600.0;
/// ...met zoveel als uitschieter. Een mens met een muis op 1000 Hz komt na het samenvatten op
/// hooguit ~125 bewegingen per seconde, plus wat hij typt.
pub const INVOER_VOORRAAD: u32 = 200;
/// Klembordteksten per seconde, en hoeveel als uitschieter. Kopiëren is mensenwerk.
pub const KLEMBORD_PER_SECONDE: f64 = 2.0;
pub const KLEMBORD_VOORRAAD: u32 = 4;

/// Waar een kijker staat ten opzichte van één scherm van een ander.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Bedienstand {
    /// De deler staat het niet toe: optie uit, geen monitor, een mac, of een oudere versie.
    #[default]
    Uit,
    /// Het mag; we vroegen nog niet.
    Mag,
    /// We vroegen; de gebruiker aan de andere kant moet nog klikken.
    Gevraagd,
    /// We bedienen.
    Actief,
}

impl InputEvent {
    /// Een bekende soort met velden binnen hun bereik. Een ongeldige gebeurtenis valt weg; de
    /// rest van het bericht niet.
    pub fn is_geldig(&self) -> bool {
        match self.kind {
            InputKind::MOVE => true,
            InputKind::BUTTON_DOWN | InputKind::BUTTON_UP => (1..=5).contains(&self.button),
            InputKind::WHEEL | InputKind::HWHEEL => self.delta != 0,
            InputKind::KEY_DOWN | InputKind::KEY_UP => toets_geldig(self.scan, self.vk),
            _ => false,
        }
    }

    /// Loslaten van een knop of toets. Dat wordt nooit afgeremd: een weggevallen loslaten laat
    /// een toets bij de deler hangen.
    pub fn is_loslaten(&self) -> bool {
        matches!(self.kind, InputKind::BUTTON_UP | InputKind::KEY_UP)
    }
}

/// Wat de deler van één `RemoteInput` toepast: de geldige gebeurtenissen, van de eerste
/// [`MAX_INVOER_PER_BERICHT`] alles en daarna alleen nog het loslaten. Een nieuwere peer met een
/// ruimere grens raakt zo hooguit bewegingen kwijt, nooit het loslaten van Shift of een knop.
pub fn begrens(events: &[InputEvent]) -> Vec<InputEvent> {
    events
        .iter()
        .enumerate()
        .filter(|(i, e)| e.is_geldig() && (*i < MAX_INVOER_PER_BERICHT || e.is_loslaten()))
        .map(|(_, e)| *e)
        .collect()
}

/// Een scancode uit set 1 (`0x01..=0x7F`, eventueel met `0xE0` ervoor), of anders een virtuele
/// toets. Nooit een muisknop als toets (`VK_LBUTTON` en verwanten): die horen bij de knoppen.
fn toets_geldig(scan: u16, vk: u8) -> bool {
    let vk_ok = vk != 0 && vk != 0xFF && !matches!(vk, 0x01 | 0x02 | 0x04 | 0x05 | 0x06);
    if scan == 0 {
        return vk_ok;
    }
    let (hoog, laag) = (scan >> 8, scan & 0xFF);
    (hoog == 0 || hoog == 0xE0) && (0x01..=0x7F).contains(&laag) && (vk == 0 || vk_ok)
}

/// Tekst voor of van het klembord: zonder NUL-tekens, niet leeg, hooguit [`MAX_KLEMBORD`]
/// bytes. `None` betekent: niet versturen en niet zetten.
pub fn schoon_klembord(tekst: &str) -> Option<String> {
    if tekst.len() > MAX_KLEMBORD {
        return None;
    }
    let schoon: String = tekst.chars().filter(|&c| c != '\0').collect();
    (!schoon.is_empty()).then_some(schoon)
}

/// Een rem: gemiddeld `per_seconde`, met `max` als uitschieter. De tijd komt van buiten, zodat
/// het te testen is.
#[derive(Debug, Clone)]
pub struct Emmer {
    voorraad: f64,
    max: f64,
    per_seconde: f64,
    laatst: Instant,
}

impl Emmer {
    /// Begint vol.
    pub fn nieuw(max: u32, per_seconde: f64, nu: Instant) -> Self {
        Self {
            voorraad: f64::from(max),
            max: f64::from(max),
            per_seconde,
            laatst: nu,
        }
    }

    /// Eén eruit, als er een is.
    pub fn neem(&mut self, nu: Instant) -> bool {
        let erbij = nu.saturating_duration_since(self.laatst).as_secs_f64() * self.per_seconde;
        self.voorraad = (self.voorraad + erbij).min(self.max);
        self.laatst = nu;
        if self.voorraad >= 1.0 {
            self.voorraad -= 1.0;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn e(kind: InputKind) -> InputEvent {
        InputEvent {
            kind,
            x: 0,
            y: 0,
            button: 0,
            delta: 0,
            scan: 0,
            vk: 0,
        }
    }

    #[test]
    fn alleen_zinnige_invoer_is_geldig() {
        assert!(e(InputKind::MOVE).is_geldig());
        assert!(!e(InputKind(200)).is_geldig(), "onbekende soort");

        let knop = |b| InputEvent {
            button: b,
            ..e(InputKind::BUTTON_DOWN)
        };
        assert!(knop(1).is_geldig() && knop(5).is_geldig());
        assert!(!knop(0).is_geldig() && !knop(6).is_geldig());

        let wiel = |d| InputEvent {
            delta: d,
            ..e(InputKind::WHEEL)
        };
        assert!(wiel(-120).is_geldig() && !wiel(0).is_geldig());

        let toets = |scan, vk| InputEvent {
            scan,
            vk,
            ..e(InputKind::KEY_DOWN)
        };
        assert!(toets(0x1E, 0x41).is_geldig(), "A");
        assert!(toets(0xE01D, 0).is_geldig(), "rechter Ctrl, uitgebreid");
        assert!(toets(0, 0x13).is_geldig(), "Pause: alleen een vk");
        assert!(!toets(0, 0).is_geldig(), "niets");
        assert!(
            !toets(0x80, 0).is_geldig(),
            "een loslaatcode is geen scancode"
        );
        assert!(!toets(0xE11D, 0).is_geldig(), "alleen 0xE0 als voorvoegsel");
        assert!(!toets(0, 0x01).is_geldig(), "VK_LBUTTON is geen toets");
        assert!(!toets(0, 0xFF).is_geldig());
    }

    #[test]
    fn een_te_groot_bericht_houdt_elk_loslaten() {
        let mut events = vec![e(InputKind::MOVE); MAX_INVOER_PER_BERICHT + 10];
        events.push(InputEvent {
            scan: 0x2A,
            ..e(InputKind::KEY_UP)
        });
        events.push(e(InputKind(99)));
        let over = begrens(&events);
        assert_eq!(over.len(), MAX_INVOER_PER_BERICHT + 1);
        assert_eq!(over.last().unwrap().kind, InputKind::KEY_UP, "Shift los");
        // Een ongeldige gebeurtenis vooraan valt weg, de rest blijft.
        assert_eq!(begrens(&[e(InputKind(99)), e(InputKind::MOVE)]).len(), 1);
    }

    #[test]
    fn alleen_loslaten_is_loslaten() {
        assert!(e(InputKind::KEY_UP).is_loslaten() && e(InputKind::BUTTON_UP).is_loslaten());
        assert!(!e(InputKind::KEY_DOWN).is_loslaten() && !e(InputKind::MOVE).is_loslaten());
    }

    #[test]
    fn het_klembord_gaat_zonder_nul_en_niet_boven_de_grens() {
        assert_eq!(schoon_klembord("a\0b").as_deref(), Some("ab"));
        assert_eq!(schoon_klembord("\0\0"), None, "alleen NUL is leeg");
        assert_eq!(schoon_klembord(""), None);
        let precies = "x".repeat(MAX_KLEMBORD);
        assert_eq!(
            schoon_klembord(&precies).map(|s| s.len()),
            Some(MAX_KLEMBORD)
        );
        assert_eq!(
            schoon_klembord(&"x".repeat(MAX_KLEMBORD + 1)),
            None,
            "niet afkappen"
        );
        // Bytes, niet tekens: vier bytes per emoji.
        assert_eq!(schoon_klembord(&"😀".repeat(MAX_KLEMBORD / 4 + 1)), None);
    }

    #[test]
    fn de_emmer_laat_een_uitschieter_toe_en_daarna_het_tempo() {
        let t0 = Instant::now();
        let mut emmer = Emmer::nieuw(3, 10.0, t0);
        assert!((0..3).all(|_| emmer.neem(t0)), "de voorraad");
        assert!(!emmer.neem(t0), "leeg");
        assert!(
            emmer.neem(t0 + Duration::from_millis(100)),
            "na 100 ms één erbij"
        );
        assert!(!emmer.neem(t0 + Duration::from_millis(100)));
        // Lang wachten vult niet boven de voorraad.
        let later = t0 + Duration::from_secs(60);
        assert!((0..3).all(|_| emmer.neem(later)));
        assert!(!emmer.neem(later));
    }
}
