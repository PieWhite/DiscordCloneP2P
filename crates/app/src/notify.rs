//! Systeemmeldingen.
//!
//! Een melding die niet lukt mag nooit iets breken: op sommige systemen zijn toasts
//! uitgezet of geblokkeerd door "focus assist"/"niet storen". We loggen dat en gaan door.

use std::sync::atomic::{AtomicBool, Ordering};

/// Zonder een geregistreerde snelkoppeling accepteert Windows geen eigen app-id voor
/// toasts. We lenen daarom die van PowerShell, wat de gangbare oplossing is voor een
/// app die je zelf naast de exe uitpakt in plaats van installeert.
#[cfg(windows)]
const APP_ID: &str =
    "{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\WindowsPowerShell\\v1.0\\powershell.exe";

/// Blijft `false` zodra bleek dat meldingen hier niet werken, zodat we het niet elk
/// bericht opnieuw proberen en de log niet vervuilen.
static TOASTS_WERKEN: AtomicBool = AtomicBool::new(true);

/// Lange berichten passen niet in een melding en maken hem onleesbaar.
fn kort(tekst: &str) -> String {
    if tekst.chars().count() > 120 {
        tekst.chars().take(117).collect::<String>() + "…"
    } else {
        tekst.to_string()
    }
}

#[cfg(windows)]
pub fn nieuw_bericht(van: &str, tekst: &str) {
    speel_geluid();

    if !TOASTS_WERKEN.load(Ordering::Relaxed) {
        return;
    }

    let resultaat = tauri_winrt_notification::Toast::new(APP_ID)
        .title(van)
        .text1(&kort(tekst))
        .duration(tauri_winrt_notification::Duration::Short)
        .show();

    if let Err(e) = resultaat {
        tracing::warn!(error = %e, "meldingen werken niet op dit systeem; verder zonder");
        TOASTS_WERKEN.store(false, Ordering::Relaxed);
    }
}

/// Op macOS via `osascript`: nul afhankelijkheden, werkt gebundeld én als losse
/// binary, en respecteert Focus/Niet storen vanzelf. Het geluid ("Ping") zit in
/// dezelfde aanroep.
/// ponytail: upgradepad is UNUserNotificationCenter zodra er echte codesigning is —
/// die API weigert zonder geldige bundel-identiteit.
#[cfg(target_os = "macos")]
pub fn nieuw_bericht(van: &str, tekst: &str) {
    if !TOASTS_WERKEN.load(Ordering::Relaxed) {
        return;
    }

    // AppleScript-strings: alleen backslash en dubbele quote hoeven escaped.
    fn esc(s: &str) -> String {
        s.replace('\\', "\\\\").replace('"', "\\\"")
    }
    let script = format!(
        "display notification \"{}\" with title \"{}\" sound name \"Ping\"",
        esc(&kort(tekst)),
        esc(van)
    );

    let resultaat = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();

    if let Err(e) = resultaat {
        tracing::warn!(error = %e, "meldingen werken niet op dit systeem; verder zonder");
        TOASTS_WERKEN.store(false, Ordering::Relaxed);
    }
}

/// Het standaard Windows-meldingsgeluid. Geen eigen wav-bestand nodig, en het klinkt
/// zoals de gebruiker het van zijn systeem gewend is — inclusief zijn eigen
/// volume-instellingen en "niet storen".
#[cfg(windows)]
fn speel_geluid() {
    use windows::Win32::System::Diagnostics::Debug::MessageBeep;
    use windows::Win32::UI::WindowsAndMessaging::MB_ICONASTERISK;

    // SAFETY: MessageBeep neemt alleen een constante en heeft geen neveneffecten
    // buiten het afspelen van een systeemgeluid.
    unsafe {
        let _ = MessageBeep(MB_ICONASTERISK);
    }
}
