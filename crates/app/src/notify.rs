//! Systeemmeldingen.
//!
//! Een melding die niet lukt mag nooit iets breken: op sommige systemen zijn toasts
//! uitgezet of geblokkeerd door "focus assist"/"niet storen". We loggen dat en gaan door.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Zonder een geregistreerde snelkoppeling accepteert Windows geen eigen app-id voor
/// toasts. We lenen daarom die van PowerShell, wat de gangbare oplossing is voor een
/// app die je zelf naast de exe uitpakt in plaats van installeert.
#[cfg(windows)]
const APP_ID: &str =
    "{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\WindowsPowerShell\\v1.0\\powershell.exe";

/// B-57: hoe lang we meldingen laten rusten na een mislukking.
///
/// Dit was een `AtomicBool` die één keer omging en daarna nooit meer terug: één tijdelijke
/// WinRT-hik of een quotum kostte je *alle* mentionmeldingen tot een herstart, terwijl de
/// oorzaak meestal binnen een minuut voorbij is. Een afkoelperiode houdt de oorspronkelijke
/// bedoeling — niet elk bericht opnieuw proberen en de log niet vervuilen — zonder de app
/// permanent stil te zetten.
const AFKOELPERIODE: Duration = Duration::from_secs(300);

/// Millis sinds procesbegin van de laatste mislukking, of 0 als er nog niets misging.
/// Een `AtomicU64` in plaats van een `Mutex<Instant>`: dit zit op het meldingspad en hoeft
/// niemand te laten wachten.
static LAATSTE_MISLUKKING_MS: AtomicU64 = AtomicU64::new(0);

/// Vast startpunt om `Instant` als getal te kunnen bewaren.
fn sinds_start() -> u64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// Of we het nu mogen proberen. Vlak na een mislukking niet; daarna weer wel.
fn toasts_mogen() -> bool {
    let laatste = LAATSTE_MISLUKKING_MS.load(Ordering::Relaxed);
    laatste == 0 || sinds_start().saturating_sub(laatste) >= AFKOELPERIODE.as_millis() as u64
}

fn meld_mislukking() {
    LAATSTE_MISLUKKING_MS.store(sinds_start().max(1), Ordering::Relaxed);
}

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

    if !toasts_mogen() {
        return;
    }

    let resultaat = tauri_winrt_notification::Toast::new(APP_ID)
        .title(van)
        .text1(&kort(tekst))
        .duration(tauri_winrt_notification::Duration::Short)
        .show();

    if let Err(e) = resultaat {
        tracing::warn!(error = %e, "melding mislukt; even niet opnieuw proberen");
        meld_mislukking();
    }
}

/// Op macOS via `osascript`: nul afhankelijkheden, werkt gebundeld én als losse
/// binary, en respecteert Focus/Niet storen vanzelf. Het geluid ("Ping") zit in
/// dezelfde aanroep.
/// ponytail: upgradepad is UNUserNotificationCenter zodra er echte codesigning is —
/// die API weigert zonder geldige bundel-identiteit.
#[cfg(target_os = "macos")]
pub fn nieuw_bericht(van: &str, tekst: &str) {
    if !toasts_mogen() {
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
        tracing::warn!(error = %e, "melding mislukt; even niet opnieuw proberen");
        meld_mislukking();
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
