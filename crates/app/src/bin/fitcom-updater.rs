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

// Het hele mechanisme — wachten op een PID en een draaiende exe overschrijven — bestaat
// omdat Windows dat overschrijven niet toestaat. Op macOS is er (nog) geen
// auto-update; zie `engine.rs` (mac-guards) en docs/OVERDRACHT.md. De binary bestaat
// daar alleen als lege stub zodat de workspace overal bouwt.
// ponytail: mac-updater = kill(pid,0)-poll + rename in de .app-bundel + `open`,
// zodra Developer-ID-signing er is (ad-hoc her-prompt Screen Recording per update).
#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
use clap::Parser;
#[cfg(windows)]
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::time::Duration;
#[cfg(windows)]
use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
#[cfg(windows)]
use windows::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};

#[cfg(windows)]
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
    /// B-20: de BLAKE3 die de app bij het downloaden tegen het ondertekende manifest legde,
    /// als hex. Wordt hier opnieuw gelegd vlak vóór het overschrijven.
    ///
    /// Optioneel zodat een oudere app die deze vlag nog niet meestuurt blijft werken. Staat
    /// hij er niet, dan gebeurt wat er hiervóór altijd gebeurde — en dat is precies de reden
    /// dat hij er wél hoort te staan.
    #[arg(long)]
    hash: Option<String>,
}

/// B-20: klopt het bestand nog met wat de app geverifieerd heeft?
///
/// Het gat tussen "gedownload en geverifieerd" en "toegepast" is onbegrensd: de gebruiker
/// klikt wanneer het hem uitkomt, en tot die tijd staat er een exe in de updatemap die
/// straks over `fitcom.exe` heen gaat. Alles wat in dat venster in die map kan schrijven,
/// wisselt de payload om. Op een eenpersoons-PC vergt dat lokale code-uitvoering — maar
/// B-02 schreef juist naar willekeurige mappen, inclusief deze.
///
/// Een ontbrekende `--hash` is geen fout (zie het veld), een niet-kloppende wél: dan gaat er
/// niets vervangen worden.
#[cfg(windows)]
fn hash_klopt(pad: &Path, verwacht_hex: &str) -> Result<(), String> {
    let mut bestand = std::fs::File::open(pad).map_err(|e| format!("update openen: {e}"))?;
    let mut hasher = blake3::Hasher::new();
    std::io::copy(&mut bestand, &mut hasher).map_err(|e| format!("update lezen: {e}"))?;
    let werkelijk = hasher.finalize();
    let werkelijk_hex: String = werkelijk
        .as_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if !werkelijk_hex.eq_ignore_ascii_case(verwacht_hex) {
        return Err(format!(
            "de update in de wachtrij is veranderd sinds hij geverifieerd werd \
             (verwacht {verwacht_hex}, gevonden {werkelijk_hex}); er wordt niets vervangen"
        ));
    }
    Ok(())
}

/// Hoe lang we maximaal wachten tot de hoofd-app afsluit. Ruim boven wat een nette
/// afsluiting kost; loopt het hierop vast, dan is er iets anders mis en heeft nóg langer
/// wachten geen zin.
#[cfg(windows)]
const MAX_WACHTTIJD: Duration = Duration::from_secs(30);

#[cfg(windows)]
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

    // B-20: opnieuw verifiëren ná het wachten op de hoofd-app, niet ervoor — het venster dat
    // telt loopt tot vlak vóór het overschrijven, en dit is het laatste moment waarop we er
    // nog iets aan kunnen doen.
    match &args.hash {
        Some(hex) => match hash_klopt(&args.new, hex) {
            Ok(()) => log(&log_pad, "hash van de update klopt nog"),
            Err(e) => {
                log(&log_pad, &format!("BIJWERKEN AFGEBROKEN: {e}"));
                return;
            }
        },
        None => log(
            &log_pad,
            "geen --hash meegekregen; overslaan van de herverificatie (oudere app)",
        ),
    }

    if let Err(e) = vervang(&args.new, &args.target) {
        log(&log_pad, &format!("exe vervangen mislukt: {e}"));
        return;
    }
    log(&log_pad, "exe vervangen, nu opnieuw starten");

    match start_zonder_handles(&args.target) {
        Ok(()) => log(&log_pad, "nieuwe versie gestart"),
        Err(e) => log(&log_pad, &format!("nieuwe versie starten mislukt: {e}")),
    }
}

/// Het pad als nul-afgesloten UTF-16.
#[cfg(windows)]
fn wide(pad: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    pad.as_os_str().encode_wide().chain([0]).collect()
}

/// Hetzelfde, maar tussen aanhalingstekens: dit wordt de opdrachtregel van het kind, en
/// zonder aanhalingstekens valt een pad met spaties (`fitcom (1).exe`) uit elkaar.
#[cfg(windows)]
fn geciteerd_wide(pad: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    std::iter::once(b'"' as u16)
        .chain(pad.as_os_str().encode_wide())
        .chain([b'"' as u16, 0])
        .collect()
}

/// Start de bijgewerkte app **zonder er ook maar één handle aan door te geven**.
///
/// `std::process::Command` zet `bInheritHandles` op Windows altijd op TRUE — het moet wel,
/// want zo komen de stdio-handles bij het kind terecht. Gevolg: élk erfelijk handle in dit
/// proces gaat mee. Dat is precies hoe de mediapoort bezet raakte na een update: een app
/// van vóór de `HANDLE_FLAG_INHERIT`-fix in `fitcom-net` gaf zijn gekloonde mediasocket
/// door aan deze updater, en deze updater gaf hem door aan de nieuwe app, die daarna zijn
/// eigen poort niet meer kon binden (os error 10048) tot iemand hem met de hand herstartte.
///
/// Die fix dicht de bron; deze dicht de doorgeefluik. Dat is nodig omdat een geërfd handle
/// zelf ook weer erfelijk is: zonder dit blijft een zombie uit een oude generatie
/// meereizen naar elke volgende update. Met `bInheritHandles = FALSE` stopt de ketting
/// hier, wat de app die ons startte ook aan ons doorgaf.
///
/// Het kind krijgt zo geen stdio-handles. Dat is goed: de app is een GUI-programma
/// (`windows_subsystem = "windows"`) en `Command::new` gaf hem in de praktijk de console
/// van dit achtergrondproces, dus niets.
#[cfg(windows)]
fn start_zonder_handles(exe: &Path) -> Result<(), String> {
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::System::Threading::{
        CreateProcessW, PROCESS_CREATION_FLAGS, PROCESS_INFORMATION, STARTUPINFOW,
    };

    let pad = wide(exe);
    // CreateProcessW mag in deze buffer schrijven, dus hij moet van ons zijn en muteerbaar.
    let mut opdrachtregel = geciteerd_wide(exe);

    let start = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        ..Default::default()
    };
    let mut proces = PROCESS_INFORMATION::default();

    // SAFETY: beide buffers zijn nul-afgesloten en leven tot na de aanroep; `start` is
    // volledig geïnitialiseerd met zijn eigen grootte in `cb`, en `proces` is een geldige
    // uitvoerplek. Een leeg werkmappad betekent "erf die van ons", net als bij `Command`.
    unsafe {
        CreateProcessW(
            PCWSTR(pad.as_ptr()),
            Some(PWSTR(opdrachtregel.as_mut_ptr())),
            None,
            None,
            false, // hierom bestaat deze functie
            PROCESS_CREATION_FLAGS(0),
            None,
            PCWSTR::null(),
            &start,
            &mut proces,
        )
    }
    .map_err(|e| e.to_string())?;

    // De app draait nu op eigen benen; wij hoeven zijn proces- en threadhandle niet vast
    // te houden en sluiten ze meteen.
    unsafe {
        let _ = CloseHandle(proces.hProcess);
        let _ = CloseHandle(proces.hThread);
    }
    Ok(())
}

/// Wacht tot `pid` verdwenen is via `WaitForSingleObject` — geen polling-loop op
/// `tasklist` nodig, Windows kan dit rechtstreeks.
#[cfg(windows)]
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
#[cfg(windows)]
fn vervang(nieuw: &Path, doel: &Path) -> Result<(), String> {
    if std::fs::rename(nieuw, doel).is_ok() {
        return Ok(());
    }
    std::fs::copy(nieuw, doel).map_err(|e| format!("kopiëren: {e}"))?;
    let _ = std::fs::remove_file(nieuw);
    Ok(())
}

#[cfg(windows)]
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

#[cfg(windows)]
#[cfg(test)]
mod tests {
    use super::*;

    /// Het pad van de app staat bij Rick in `Downloads` en heet `fitcom (1).exe` — met
    /// spaties. Valt de opdrachtregel daar uit elkaar, dan start de nieuwe versie niet.
    #[test]
    fn een_pad_met_spaties_blijft_een_argument() {
        let w = geciteerd_wide(Path::new(r"C:\Users\rick_\Downloads\fitcom (1).exe"));
        assert_eq!(*w.last().unwrap(), 0, "CreateProcessW leest tot de nul");
        assert_eq!(
            String::from_utf16(&w[..w.len() - 1]).unwrap(),
            "\"C:\\Users\\rick_\\Downloads\\fitcom (1).exe\""
        );
    }

    /// `lpApplicationName` wil het kale pad, zonder aanhalingstekens.
    #[test]
    fn het_programmapad_gaat_er_kaal_in() {
        let w = wide(Path::new(r"C:\map\fitcom.exe"));
        assert_eq!(*w.last().unwrap(), 0);
        assert_eq!(
            String::from_utf16(&w[..w.len() - 1]).unwrap(),
            r"C:\map\fitcom.exe"
        );
    }
}
