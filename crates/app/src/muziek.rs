//! Wat Spotify nu speelt, voor de titel van een gedeelde muziekstream ("Artiest – Nummer").
//!
//! Lokaal en zonder internet: op Windows uit de mediabediening van het systeem (dezelfde
//! bron als de mediaknoppen en het volumevenster), op macOS via AppleScript bij Spotify zelf.
//! Geen Spotify Web API, geen account — invariant 1 blijft heel. Lukt het lezen niet, dan
//! heet de stream gewoon "Spotify" en speelt de muziek gewoon door.
//!
//! Het lezen gebeurt op een eigen thread: de WinRT-aanroep wacht op een async-antwoord en
//! `osascript` start een proces, en geen van beide hoort op de motortik.

use fitcom_proto::control::MusicAction;
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

/// Pauze/afspelen of volgende bij onze eigen Spotify, op verzoek van een luisteraar
/// (beslissing 41). Dezelfde mediabediening als `lees`, dus geen Web API en geen account.
/// Op een eigen kortlevende thread: de WinRT-aanroep wacht en `osascript` start een
/// proces, en geen van beide hoort op de motortik. Lukt het niet, dan gebeurt er niets.
pub fn bedien(actie: MusicAction) {
    let gestart = std::thread::Builder::new()
        .name("fitcom-muziek-bediening".into())
        .spawn(move || {
            if !voer_uit(actie) {
                tracing::debug!(?actie, "Spotify bedienen lukte niet");
            }
        });
    if let Err(e) = gestart {
        tracing::warn!(error = %e, "muziekbediening-thread starten mislukt");
    }
}

#[cfg(windows)]
fn voer_uit(actie: MusicAction) -> bool {
    use windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager as Beheer;

    let Some(sessies) = Beheer::RequestAsync()
        .and_then(|b| b.join())
        .and_then(|b| b.GetSessions())
        .ok()
    else {
        return false;
    };
    for sessie in sessies {
        let is_spotify = sessie
            .SourceAppUserModelId()
            .is_ok_and(|a| a.to_string().to_ascii_lowercase().contains("spotify"));
        if !is_spotify {
            continue;
        }
        let gelukt = match actie {
            MusicAction::PLAY_PAUSE => sessie.TryTogglePlayPauseAsync(),
            MusicAction::NEXT => sessie.TrySkipNextAsync(),
            _ => return false,
        };
        return gelukt.and_then(|a| a.join()).unwrap_or(false);
    }
    false
}

/// Vaste scripts zonder iets van de draad erin; zelfde "is running"-rem als `lees`, zodat
/// een klik van een luisteraar Spotify nooit opstart.
#[cfg(target_os = "macos")]
fn voer_uit(actie: MusicAction) -> bool {
    let opdracht = match actie {
        MusicAction::PLAY_PAUSE => "playpause",
        MusicAction::NEXT => "next track",
        _ => return false,
    };
    let script = format!(
        "if application \"Spotify\" is running then tell application \"Spotify\" to {opdracht}"
    );
    std::process::Command::new("osascript")
        .args(["-e", &script])
        .status()
        .is_ok_and(|s| s.success())
}

/// Een Spotify-track-id: precies 22 tekens base62. Het gaat een URI in
/// (`spotify:track:<id>`) en komt uit een chatbericht dat een peer typte — de B-03-klasse,
/// net als `youtube::geldig_id`. Dit kan niets anders worden dan een id.
pub fn geldig_track_id(id: &str) -> bool {
    id.len() == 22 && id.bytes().all(|b| b.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::{geldig_track_id, lees, opmaak};

    #[test]
    fn track_id_is_precies_22_tekens_base62() {
        assert!(geldig_track_id("4uLU6hMCjMI75M1A2tKUQC"));
        for aanval in [
            "",
            "4uLU6hMCjMI75M1A2tKUQ",   // 21
            "4uLU6hMCjMI75M1A2tKUQCx", // 23
            "4uLU6hMCjMI75M1A2tKU-C",
            "4uLU6hMCjMI75M1A2tKU_C",
            "..\\..\\..\\Windows\\cmd",
            "4uLU6hMCjMI75M1A2tK?x=1",
            "4uLU6hMCjMI75M1A2tKUQ\n",
            "4uLU6hMCjMI75M1A2tKU Q",
            "4uLU6hMCjMI75M1A2tKUé", // 22 bytes, maar niet ASCII
        ] {
            assert!(!geldig_track_id(aanval), "geaccepteerd: {aanval:?}");
        }
    }

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
