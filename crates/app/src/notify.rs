//! Windows-meldingen.
//!
//! Een melding die niet lukt mag nooit iets breken: op sommige systemen zijn toasts
//! uitgezet of geblokkeerd door "focus assist". We loggen dat en gaan door.

use std::sync::atomic::{AtomicBool, Ordering};

/// Zonder een geregistreerde snelkoppeling accepteert Windows geen eigen app-id voor
/// toasts. We lenen daarom die van PowerShell, wat de gangbare oplossing is voor een
/// app die je zelf naast de exe uitpakt in plaats van installeert.
const APP_ID: &str =
    "{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\WindowsPowerShell\\v1.0\\powershell.exe";

/// Blijft `false` zodra bleek dat toasts hier niet werken, zodat we het niet elk
/// bericht opnieuw proberen en de log niet vervuilen.
static TOASTS_WERKEN: AtomicBool = AtomicBool::new(true);

pub fn nieuw_bericht(van: &str, tekst: &str) {
    speel_geluid();

    if !TOASTS_WERKEN.load(Ordering::Relaxed) {
        return;
    }

    // Lange berichten passen niet in een toast en maken hem onleesbaar.
    let kort: String = if tekst.chars().count() > 120 {
        tekst.chars().take(117).collect::<String>() + "…"
    } else {
        tekst.to_string()
    };

    let resultaat = tauri_winrt_notification::Toast::new(APP_ID)
        .title(van)
        .text1(&kort)
        .duration(tauri_winrt_notification::Duration::Short)
        .show();

    if let Err(e) = resultaat {
        tracing::warn!(error = %e, "meldingen werken niet op dit systeem; verder zonder");
        TOASTS_WERKEN.store(false, Ordering::Relaxed);
    }
}

/// Het standaard Windows-meldingsgeluid. Geen eigen wav-bestand nodig, en het klinkt
/// zoals de gebruiker het van zijn systeem gewend is — inclusief zijn eigen
/// volume-instellingen en "niet storen".
fn speel_geluid() {
    use windows::Win32::System::Diagnostics::Debug::MessageBeep;
    use windows::Win32::UI::WindowsAndMessaging::MB_ICONASTERISK;

    // SAFETY: MessageBeep neemt alleen een constante en heeft geen neveneffecten
    // buiten het afspelen van een systeemgeluid.
    unsafe {
        let _ = MessageBeep(MB_ICONASTERISK);
    }
}
