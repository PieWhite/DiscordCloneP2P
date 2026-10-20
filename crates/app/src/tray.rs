//! Tray-icoon, verbergen naar de tray, en autostart.
//!
//! # Waarom de tray-events op Windows op een eigen thread worden afgehandeld
//!
//! Verberg je het venster, dan tekent de weergavelaag niet meer. Zou de tray-klik daar
//! afgehandeld worden, dan kon je het venster nooit meer terughalen — precies wanneer je
//! het nodig hebt. Een losse thread leest de tray-gebeurtenissen en praat rechtstreeks
//! met het venster via Win32.
//!
//! Op macOS is die reden er niet: de main-runloop van `NSApplication` blijft pompen
//! terwijl het venster verborgen is. Daar draait `ui/mod.rs::mac_tray` dezelfde
//! `tray-icon`-crate gewoon op de main thread; deze module levert alleen de gedeelde
//! afsluitvlag, het icoon en de autostart-schakelaar.

use anyhow::Result;
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(windows)]
use anyhow::Context;
#[cfg(windows)]
use std::sync::atomic::AtomicIsize;
#[cfg(windows)]
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
#[cfg(windows)]
use tray_icon::{TrayIcon, TrayIconBuilder, TrayIconEvent};

/// Het venster van deze app. 0 zolang het nog niet bekend is.
#[cfg(windows)]
static HWND: AtomicIsize = AtomicIsize::new(0);
/// Wordt gezet als de gebruiker in de tray op "afsluiten" klikt.
static AFSLUITEN: AtomicBool = AtomicBool::new(false);

#[cfg(windows)]
pub fn onthoud_venster(hwnd: isize) {
    HWND.store(hwnd, Ordering::Relaxed);
}

pub fn wil_afsluiten() -> bool {
    AFSLUITEN.load(Ordering::Relaxed)
}

/// Voor de macOS-tray in `ui/mod.rs`: zelfde vlag, zelfde afsluitroute via
/// `spawn_quit_watcher`.
pub fn markeer_afsluiten() {
    AFSLUITEN.store(true, Ordering::Relaxed);
}

/// Het icoon moet blijven leven zolang de app draait; valt het weg, dan verdwijnt het
/// uit de tray.
#[cfg(windows)]
pub struct Tray {
    _icon: TrayIcon,
}

#[cfg(windows)]
const OPENEN_ID: &str = "fitcom-openen";
#[cfg(windows)]
const AFSLUITEN_ID: &str = "fitcom-afsluiten";

/// Zet de globale menu-handler van `muda`: zo komen "Openen" en "Afsluiten" binnen, op de
/// thread die het menu toont, ook als het venster niet tekent. muda stuurt een gebeurtenis
/// naar de handler *of* naar `MenuEvent::receiver()`, nooit naar allebei.
///
/// `set_event_handler` schrijft in een `OnceCell`: wie eerst is, wint. Onder Tauri was dat
/// een race met Tauri's eigen event-loop; GPUI gebruikt muda niet, maar de regel blijft:
/// aanroepen vóór de event-loop start (`ui::open`). De app heeft geen andere menu's, dus
/// alle menu-gebeurtenissen zijn van ons.
#[cfg(windows)]
pub fn claim_menu_events() {
    MenuEvent::set_event_handler(Some(|ev: MenuEvent| match ev.id.0.as_str() {
        OPENEN_ID => toon_venster(),
        AFSLUITEN_ID => {
            // Eerst tonen: pas als de app weer draait kan hij zichzelf netjes afsluiten.
            // Zouden we hier het proces killen, dan sluiten de verbindingen niet netjes af.
            toon_venster();
            AFSLUITEN.store(true, Ordering::Relaxed);
        }
        _ => {}
    }));
}

#[cfg(windows)]
pub fn start() -> Result<Tray> {
    let menu = Menu::new();
    let openen = MenuItem::with_id(OPENEN_ID, "Openen", true, None);
    let afsluiten = MenuItem::with_id(AFSLUITEN_ID, "Afsluiten", true, None);
    menu.append(&openen).context("menu-item toevoegen")?;
    menu.append(&tray_icon::menu::PredefinedMenuItem::separator())?;
    menu.append(&afsluiten).context("menu-item toevoegen")?;

    let icon = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("FitCommunication")
        .with_icon(maak_icoon()?)
        .build()
        .context("tray-icoon aanmaken")?;

    std::thread::Builder::new()
        .name("fitcom-tray".into())
        .spawn(move || {
            // Blokkerend en alles leeglezen: tray-icon zet élke muisbeweging boven het
            // icoon in deze onbegrensde wachtrij. Eén per 100 ms ophalen liet een
            // dubbelklik achter een rij Move-events wachten, soms seconden, en het venster
            // kwam dan naar voren terwijl je alweer in je game zat.
            let tray_events = TrayIconEvent::receiver();
            while let Ok(ev) = tray_events.recv() {
                if let TrayIconEvent::DoubleClick { .. } = ev {
                    toon_venster();
                }
            }
        })
        .context("tray-thread starten")?;

    Ok(Tray { _icon: icon })
}

#[cfg(windows)]
pub fn verberg_venster() {
    use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
    if let Some(h) = venster() {
        // SAFETY: geldige HWND van ons eigen venster.
        unsafe {
            let _ = ShowWindow(h, SW_HIDE);
        }
    }
}

#[cfg(windows)]
pub fn toon_venster() {
    use windows::Win32::UI::WindowsAndMessaging::{
        IsIconic, SetForegroundWindow, ShowWindow, SW_RESTORE, SW_SHOW,
    };
    if let Some(h) = venster() {
        // SAFETY: geldige HWND van ons eigen venster.
        unsafe {
            let _ = ShowWindow(h, SW_SHOW);
            // Alleen als hij geminimaliseerd is: `SW_RESTORE` haalt een gemaximaliseerd
            // venster ook terug naar zijn gewone maat.
            if IsIconic(h).as_bool() {
                let _ = ShowWindow(h, SW_RESTORE);
            }
            let _ = SetForegroundWindow(h);
        }
    }
}

#[cfg(windows)]
fn venster() -> Option<windows::Win32::Foundation::HWND> {
    match HWND.load(Ordering::Relaxed) {
        0 => None,
        h => Some(windows::Win32::Foundation::HWND(h as *mut std::ffi::c_void)),
    }
}

/// Het tray-icoon als rauwe RGBA (32x32): `icons/tray-32.png`, de kleine snede van
/// `icons/icon.svg` zonder de prompt. Ingebakken, want een los bestand botst met "één exe".
/// Beide platforms bouwen hun tray-icoon hieruit op.
pub fn icoon_rgba() -> (Vec<u8>, u32) {
    let png = image::load_from_memory(include_bytes!("../icons/tray-32.png"))
        .expect("ingebakken tray-icoon is een geldige PNG")
        .into_rgba8();
    let n = png.width();
    debug_assert_eq!(n, png.height(), "tray-icoon is niet vierkant");
    (png.into_raw(), n)
}

#[cfg(windows)]
fn maak_icoon() -> Result<tray_icon::Icon> {
    let (rgba, n) = icoon_rgba();
    tray_icon::Icon::from_rgba(rgba, n, n).context("icoon opbouwen")
}

/// Zet of verwijdert de autostart-vermelding voor de huidige gebruiker.
///
/// Alleen `HKEY_CURRENT_USER` — dat vereist geen beheerdersrechten en raakt de andere
/// gebruikers van de PC niet.
#[cfg(windows)]
pub fn zet_autostart(aan: bool) -> Result<()> {
    use windows::core::{w, PCWSTR};
    use windows::Win32::System::Registry::{
        RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
        KEY_SET_VALUE, REG_SZ,
    };

    let exe = std::env::current_exe().context("eigen pad bepalen")?;
    let mut key = HKEY::default();

    // SAFETY: alle pointers wijzen naar geldige, in leven gehouden buffers.
    unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run"),
            None,
            KEY_SET_VALUE,
            &mut key,
        )
        .ok()
        .context("autostart-sleutel openen")?;

        let naam: Vec<u16> = "FitCommunication\0".encode_utf16().collect();
        let resultaat = if aan {
            let waarde: Vec<u16> = format!("\"{}\"\0", exe.display()).encode_utf16().collect();
            let bytes = std::slice::from_raw_parts(
                waarde.as_ptr() as *const u8,
                std::mem::size_of_val(&waarde[..]),
            );
            RegSetValueExW(key, PCWSTR(naam.as_ptr()), None, REG_SZ, Some(bytes)).ok()
        } else {
            // Niet bestaan is prima: dan stond hij al uit.
            let _ = RegDeleteValueW(key, PCWSTR(naam.as_ptr()));
            Ok(())
        };

        let _ = RegCloseKey(key);
        resultaat.context("autostart instellen")?;
    }

    Ok(())
}

/// Nog niet op macOS: een LaunchAgent-plist is het upgradepad, maar niemand heeft er
/// tot nu toe om gevraagd. De schakelaar in de instellingen doet hier dus niets.
#[cfg(target_os = "macos")]
pub fn zet_autostart(aan: bool) -> Result<()> {
    if aan {
        tracing::info!("autostart is op macOS nog niet ondersteund; instelling genegeerd");
    }
    Ok(())
}

/// Draait er al een FitCommunication op deze datamap? Dan wordt dat venster naar voren
/// gehaald en geeft dit `true`: de aanroeper hoort dan te stoppen.
///
/// Zonder dit liep een tweede exemplaar (een dubbelklik op de exe terwijl de eerste in de
/// tray staat, autostart plus een handmatige start) vast op de bezette poort en stopte het
/// zonder één melding: "poort al in gebruik" waar niemand een tweede app zag. Per datamap,
/// zodat `--data-dir` voor lokale testpeers blijft werken.
#[cfg(windows)]
pub fn al_actief(data_dir: &std::path::Path) -> bool {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::Win32::UI::WindowsAndMessaging::FindWindowW;

    // Een backslash is in een mutexnaam een naamruimte-scheiding.
    let sleutel: String = data_dir
        .to_string_lossy()
        .to_lowercase()
        .chars()
        .map(|c| if c == '\\' || c == ':' { '_' } else { c })
        .collect();
    let naam = HSTRING::from(format!("Local\\FitCommunication-{sleutel}"));
    // SAFETY: geldige naam; de handle blijft bewust open tot het proces eindigt, want
    // zolang hij bestaat bestaat de mutex.
    let bestond = unsafe {
        match CreateMutexW(None, false, &naam) {
            Ok(_) => GetLastError() == ERROR_ALREADY_EXISTS,
            Err(_) => false,
        }
    };
    if bestond {
        // SAFETY: alleen lezen; een venster dat er niet (meer) is levert een fout op.
        if let Ok(h) = unsafe { FindWindowW(None, &HSTRING::from("FitCommunication")) } {
            HWND.store(h.0 as isize, Ordering::Relaxed);
            toon_venster();
        }
    }
    bestond
}

/// Een macOS-app is vanzelf één exemplaar: Launch Services start hem geen tweede keer.
#[cfg(not(windows))]
pub fn al_actief(_data_dir: &std::path::Path) -> bool {
    false
}

/// Een fout bij het opstarten zichtbaar maken. Een release-exe heeft geen console, dus
/// zonder dit stopte de app bij een kapotte config of een bezette poort zonder één spoor.
#[cfg(windows)]
pub fn toon_fout(tekst: &str) {
    use windows::core::HSTRING;
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
    // SAFETY: geldige strings, geen ouder-venster.
    unsafe {
        MessageBoxW(
            None,
            &HSTRING::from(tekst),
            &HSTRING::from("FitCommunication kon niet starten"),
            MB_OK | MB_ICONERROR,
        );
    }
}

#[cfg(not(windows))]
pub fn toon_fout(tekst: &str) {
    eprintln!("FitCommunication kon niet starten: {tekst}");
}

#[cfg(test)]
mod tests {
    #[test]
    fn het_ingebakken_tray_icoon_is_32_bij_32_rgba() {
        let (rgba, n) = super::icoon_rgba();
        assert_eq!(n, 32);
        assert_eq!(rgba.len(), 32 * 32 * 4);
    }
}
