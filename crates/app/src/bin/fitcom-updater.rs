//! Los updater-procesje (fase 11): een exe kan zichzelf niet overschrijven terwijl hij
//! draait op Windows, dus dit kleine, aparte proces wacht tot de hoofd-app echt
//! afgesloten is, vervangt de exe, en start hem opnieuw op. Zie `docs/ARCHITECTURE.md`,
//! sectie "Automatische updates".
//!
//! Gespawnd door `crates/app/src/engine.rs::pas_update_toe`, nooit handmatig. Logt naar
//! `updater.log` naast de doel-exe — dit proces heeft geen `tracing`-opzet nodig voor
//! wat één keer draait en meteen weer stopt, maar een mislukking hier betekent wel dat
//! de gebruiker vastzit op de oude versie zonder duidelijke reden, dus loggen we hem toch.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use clap::Parser;
use std::path::{Path, PathBuf};
use std::time::Duration;
use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};

#[derive(Parser)]
struct Args {
    /// De gedownloade, al geverifieerde exe.
    #[arg(long)]
    new: PathBuf,
    /// Waar hij naartoe moet — de exe van de draaiende app.
    #[arg(long)]
    target: PathBuf,
    /// PID van de hoofd-app, om op te wachten vóór het overschrijven.
    #[arg(long)]
    pid: u32,
}

/// Hoe lang we maximaal wachten tot de hoofd-app afsluit. Ruim boven wat een nette
/// afsluiting kost; loopt het hierop vast, dan is er iets anders mis en heeft nóg langer
/// wachten geen zin.
const MAX_WACHTTIJD: Duration = Duration::from_secs(30);

fn main() {
    let args = Args::parse();
    let log_pad = args
        .target
        .parent()
        .map(|p| p.join("updater.log"))
        .unwrap_or_else(|| PathBuf::from("updater.log"));

    log(&log_pad, "updater gestart");

    if let Err(e) = wacht_op_afsluiten(args.pid) {
        log(&log_pad, &format!("wachten op hoofd-app mislukt: {e}"));
        return;
    }
    log(&log_pad, "hoofd-app is afgesloten");

    if let Err(e) = vervang(&args.new, &args.target) {
        log(&log_pad, &format!("exe vervangen mislukt: {e}"));
        return;
    }
    log(&log_pad, "exe vervangen, nu opnieuw starten");

    match std::process::Command::new(&args.target).spawn() {
        Ok(_) => log(&log_pad, "nieuwe versie gestart"),
        Err(e) => log(&log_pad, &format!("nieuwe versie starten mislukt: {e}")),
    }
}

/// Wacht tot `pid` verdwenen is via `WaitForSingleObject` — geen polling-loop op
/// `tasklist` nodig, Windows kan dit rechtstreeks.
fn wacht_op_afsluiten(pid: u32) -> Result<(), String> {
    let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) };
    let Ok(handle) = handle else {
        // Proces bestaat al niet meer (bijvoorbeeld al afgesloten voordat we hier
        // kwamen) — dat is precies wat we wilden, dus geen fout.
        return Ok(());
    };
    let millis: u32 = MAX_WACHTTIJD.as_millis().try_into().unwrap_or(u32::MAX);
    let resultaat = unsafe { WaitForSingleObject(handle, millis) };
    unsafe {
        let _ = CloseHandle(handle);
    }
    if resultaat == WAIT_OBJECT_0 {
        Ok(())
    } else {
        Err(format!(
            "timeout of fout tijdens wachten (code {})",
            resultaat.0
        ))
    }
}

/// `rename` op hetzelfde volume; `copy` + verwijderen als terugval bij een cross-volume-
/// fout (bijvoorbeeld een `--data-dir` op een andere schijf dan de installatiemap).
fn vervang(nieuw: &Path, doel: &Path) -> Result<(), String> {
    if std::fs::rename(nieuw, doel).is_ok() {
        return Ok(());
    }
    std::fs::copy(nieuw, doel).map_err(|e| format!("kopiëren: {e}"))?;
    let _ = std::fs::remove_file(nieuw);
    Ok(())
}

fn log(pad: &Path, regel: &str) {
    use std::io::Write;
    let tijd = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    if let Ok(mut bestand) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(pad)
    {
        let _ = writeln!(bestand, "[{tijd}] {regel}");
    }
}
