//! Tray-icoon, verbergen naar de tray, en autostart.
//!
//! # Waarom de tray-events op een eigen thread worden afgehandeld
//!
//! Verberg je het venster, dan tekent de weergavelaag niet meer. Zou de tray-klik daar
//! afgehandeld worden, dan kon je het venster nooit meer terughalen — precies wanneer je
//! het nodig hebt. Een losse thread leest de tray-gebeurtenissen en praat rechtstreeks
//! met het venster via Win32.

use anyhow::{Context, Result};
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder, TrayIconEvent};

/// Het venster van deze app. 0 zolang het nog niet bekend is.
static HWND: AtomicIsize = AtomicIsize::new(0);
/// Wordt gezet als de gebruiker in de tray op "afsluiten" klikt.
static AFSLUITEN: AtomicBool = AtomicBool::new(false);

pub fn onthoud_venster(hwnd: isize) {
    HWND.store(hwnd, Ordering::Relaxed);
}

pub fn wil_afsluiten() -> bool {
    AFSLUITEN.load(Ordering::Relaxed)
}

/// Het icoon moet blijven leven zolang de app draait; valt het weg, dan verdwijnt het
/// uit de tray.
pub struct Tray {
    _icon: TrayIcon,
}

pub fn start() -> Result<Tray> {
    let menu = Menu::new();
    let openen = MenuItem::new("Openen", true, None);
    let afsluiten = MenuItem::new("Afsluiten", true, None);
    menu.append(&openen).context("menu-item toevoegen")?;
    menu.append(&tray_icon::menu::PredefinedMenuItem::separator())?;
    menu.append(&afsluiten).context("menu-item toevoegen")?;

    let openen_id = openen.id().clone();
    let afsluiten_id = afsluiten.id().clone();

    let icon = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("FitCommunication")
        .with_icon(maak_icoon()?)
        .build()
        .context("tray-icoon aanmaken")?;

    std::thread::Builder::new()
        .name("fitcom-tray".into())
        .spawn(move || {
            let menu_events = MenuEvent::receiver();
            let tray_events = TrayIconEvent::receiver();

            loop {
                if let Ok(ev) = menu_events.try_recv() {
                    if ev.id == openen_id {
                        toon_venster();
                    } else if ev.id == afsluiten_id {
                        // Eerst tonen: pas als de app weer draait kan hij zichzelf netjes
                        // afsluiten. Zouden we hier het proces killen, dan sluiten de
                        // verbindingen niet netjes af.
                        toon_venster();
                        AFSLUITEN.store(true, Ordering::Relaxed);
                        return;
                    }
                }

                if let Ok(TrayIconEvent::DoubleClick { .. }) = tray_events.try_recv() {
                    toon_venster();
                }

                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        })
        .context("tray-thread starten")?;

    Ok(Tray { _icon: icon })
}

pub fn verberg_venster() {
    use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
    if let Some(h) = venster() {
        // SAFETY: geldige HWND van ons eigen venster.
        unsafe {
            let _ = ShowWindow(h, SW_HIDE);
        }
    }
}

pub fn toon_venster() {
    use windows::Win32::UI::WindowsAndMessaging::{
        SetForegroundWindow, ShowWindow, SW_RESTORE, SW_SHOW,
    };
    if let Some(h) = venster() {
        // SAFETY: geldige HWND van ons eigen venster.
        unsafe {
            let _ = ShowWindow(h, SW_SHOW);
            let _ = ShowWindow(h, SW_RESTORE);
            let _ = SetForegroundWindow(h);
        }
    }
}

fn venster() -> Option<windows::Win32::Foundation::HWND> {
    match HWND.load(Ordering::Relaxed) {
        0 => None,
        h => Some(windows::Win32::Foundation::HWND(h as *mut std::ffi::c_void)),
    }
}

/// Een gevulde cirkel met zachte rand. Klein genoeg om in de code te houden; een
/// los .ico-bestand zou meegekopieerd moeten worden en dat botst met "één exe".
fn maak_icoon() -> Result<tray_icon::Icon> {
    const N: u32 = 32;
    let mut rgba = Vec::with_capacity((N * N * 4) as usize);
    let midden = (N as f32 - 1.0) / 2.0;
    let straal = midden - 1.0;

    for y in 0..N {
        for x in 0..N {
            let dx = x as f32 - midden;
            let dy = y as f32 - midden;
            let afstand = (dx * dx + dy * dy).sqrt();
            // Anti-aliasing over één pixel, anders oogt het icoon rafelig.
            let dekking = ((straal - afstand).clamp(0.0, 1.0) * 255.0) as u8;
            rgba.extend_from_slice(&[80, 200, 120, dekking]);
        }
    }

    tray_icon::Icon::from_rgba(rgba, N, N).context("icoon opbouwen")
}

/// Zet of verwijdert de autostart-vermelding voor de huidige gebruiker.
///
/// Alleen `HKEY_CURRENT_USER` — dat vereist geen beheerdersrechten en raakt de andere
/// gebruikers van de PC niet.
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
