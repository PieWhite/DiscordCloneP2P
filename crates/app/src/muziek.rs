//! Wat Spotify nu speelt, voor de titel van een gedeelde muziekstream ("Artiest – Nummer").
//!
//! Lokaal en zonder internet: op Windows uit de mediabediening van het systeem (dezelfde
//! bron als de mediaknoppen en het volumevenster), op macOS via AppleScript bij Spotify zelf.
//! Geen Spotify Web API, geen account — invariant 1 blijft heel. Lukt het lezen niet, dan
//! heet de stream gewoon "Spotify" en speelt de muziek gewoon door.
//!
//! Het lezen gebeurt op een eigen thread: de WinRT-aanroep wacht op een async-antwoord en
//! `osascript` start een proces, en geen van beide hoort op de motortik.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Vaak genoeg dat een nieuw nummer er binnen een paar seconden staat, zelden genoeg dat
/// het niets kost.
const INTERVAL: Duration = Duration::from_secs(2);

/// Leest zolang hij leeft. Laten vallen stopt de thread (na hoogstens één interval).
pub struct NuSpeelt {
    laatste: Arc<Mutex<Option<String>>>,
    stop: Arc<AtomicBool>,
}

impl NuSpeelt {
    pub fn start() -> Self {
        let laatste = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let (l, s) = (laatste.clone(), stop.clone());
        let gestart = std::thread::Builder::new()
            .name("fitcom-nu-speelt".into())
            .spawn(move || {
                while !s.load(Ordering::Relaxed) {
                    let nu = lees();
                    if let Ok(mut l) = l.lock() {
                        *l = nu;
                    }
                    std::thread::sleep(INTERVAL);
                }
            });
        if let Err(e) = gestart {
            tracing::warn!(error = %e, "nu-speelt-thread starten mislukt; de stream heet gewoon Spotify");
        }
        Self { laatste, stop }
    }

    /// "Artiest – Nummer", of `None` als er (nog) niets bekend is.
    pub fn titel(&self) -> Option<String> {
        self.laatste.lock().ok().and_then(|l| l.clone())
    }
}

impl Drop for NuSpeelt {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn opmaak(artiest: &str, nummer: &str) -> Option<String> {
    match (artiest.trim(), nummer.trim()) {
        ("", "") => None,
        ("", n) => Some(n.to_string()),
        (a, "") => Some(a.to_string()),
        (a, n) => Some(format!("{a} \u{2013} {n}")),
    }
}

/// De mediasessie van Spotify uit de systeembrede mediabediening. Spotify meldt zich daar
/// als `Spotify.exe` (los geïnstalleerd) of als `SpotifyAB.SpotifyMusic_…!Spotify` (Store).
#[cfg(windows)]
fn lees() -> Option<String> {
    use windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager as Beheer;

    let beheer = Beheer::RequestAsync().ok()?.join().ok()?;
    let sessies = beheer.GetSessions().ok()?;
    for sessie in sessies {
        let app = sessie.SourceAppUserModelId().ok()?.to_string();
        if !app.to_ascii_lowercase().contains("spotify") {
            continue;
        }
        let p = sessie.TryGetMediaPropertiesAsync().ok()?.join().ok()?;
        return opmaak(&p.Artist().ok()?.to_string(), &p.Title().ok()?.to_string());
    }
    None
}

/// "is running" eerst, anders start AppleScript Spotify zelf op. De eerste keer vraagt
/// macOS of FitCommunication Spotify mag bedienen (`NSAppleEventsUsageDescription`); nee
/// zeggen betekent alleen dat de stream "Spotify" heet.
#[cfg(target_os = "macos")]
fn lees() -> Option<String> {
    const SCRIPT: &str = r#"if application "Spotify" is running then
    tell application "Spotify"
        if player state is not stopped then return (artist of current track) & tab & (name of current track)
    end tell
end if
return """#;
    let uit = std::process::Command::new("osascript")
        .args(["-e", SCRIPT])
        .output()
        .ok()?;
    let tekst = String::from_utf8_lossy(&uit.stdout);
    let (artiest, nummer) = tekst.trim_end().split_once('\t')?;
    opmaak(artiest, nummer)
}

#[cfg(test)]
mod tests {
    use super::{lees, opmaak};

    /// `cargo test -p fitcom muziek -- --ignored --nocapture` met Spotify open: toont wat
    /// de mediabediening van het systeem over hem zegt (ook op pauze).
    #[test]
    #[ignore = "vereist een draaiende Spotify-desktopapp"]
    fn leest_wat_spotify_speelt() {
        let titel = lees();
        println!("nu speelt: {titel:?}");
        assert!(titel.is_some(), "Spotify draait maar meldt geen nummer");
    }

    #[test]
    fn titel_uit_artiest_en_nummer() {
        assert_eq!(
            opmaak("Daft Punk", "Veridis Quo").as_deref(),
            Some("Daft Punk – Veridis Quo")
        );
        assert_eq!(opmaak("  ", "Nummer").as_deref(), Some("Nummer"));
        assert_eq!(opmaak("", ""), None);
    }
}
