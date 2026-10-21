//! Bij de deler: muis en toetsen van één bediener in het systeem zetten, en het klembord met
//! hem delen (beslissing 54).
//!
//! Eén thread per bediend scherm, alleen zolang er bediend wordt. Hij wacht in `GetMessageW` op
//! een verborgen berichtvenster en doet dus niets zolang er niets binnenkomt: geen tik, geen
//! pollen. De motor zet invoer in een kanaal en wekt hem met `PostMessageW`; nooit `SendInput`
//! op de motortik of op de thread van het venster.
//!
//! # Waar de muis heen gaat
//!
//! De bediener stuurt een plek op het gedeelde beeld (`[0, 0]` tot `[65535, 65535]`). Die komt
//! alleen op de gedeelde monitor terecht, geklemd binnen zijn rand
//! ([`naar_bureaublad`]), via `MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK`. Het rekenen
//! gebeurt in fysieke pixels: het proces is PerMonitorV2 (het manifest van GPUI), dus
//! `GetMonitorInfoW` en `GetSystemMetrics` geven fysieke pixels. Een gedeeld *venster* bedient
//! niemand (zie de motor): een klik op "het venster" komt terecht op wat er op dat moment
//! bovenop ligt, en dat ziet de bediener niet.
//!
//! # Grenzen
//!
//! - Toetsen gaan als scancode; de toetsenbordindeling van de deler bepaalt het teken.
//! - Absolute muisbewegingen werken niet in een spel dat de muis vangt en relatief leest (een
//!   shooter): dat ziet geen beweging. Klikken en toetsen wel.
//! - Windows laat een gewoon proces geen invoer in een programma zetten dat als administrator
//!   draait (UIPI); daar gebeurt dan stil niets.
//! - Ctrl+Alt+Del bestaat hier niet, en wat de bediener met Alt+Tab of de Windows-toets doet
//!   blijft op zijn eigen PC (dat kan alleen met een hook, en die bouwen we niet).
//!
//! # Niets blijft hangen
//!
//! Wat de bediener indrukte wordt bijgehouden en losgelaten zodra het bedienen stopt (deze
//! thread sluit). Komt er anderhalve seconde niets meer binnen terwijl er een toets ingedrukt
//! staat, dan gaat die ook los: een ingedrukte toets herhaalt zich bij de bediener, dus stilte
//! betekent dat hij weg is. Een muisknop blijft wel staan (lang ingedrukt houden zonder te
//! bewegen is gewoon gebruik); die gaat los bij het einde.

use crate::klembord;
use crate::tekenlaag::monitor_kader;
use anyhow::{Context, Result};
use crossbeam_channel::{bounded, unbounded, Receiver, Sender, TryRecvError};
use fitcom_proto::control::{InputEvent, InputKind};
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::time::{Duration, Instant};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::DataExchange::{
    AddClipboardFormatListener, RemoveClipboardFormatListener,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, MOUSEEVENTF_ABSOLUTE,
    MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN,
    MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
    MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL, MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, MOUSEINPUT,
    MOUSE_EVENT_FLAGS, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    GetSystemMetrics, KillTimer, PostMessageW, RegisterClassW, SetTimer, TranslateMessage,
    HWND_MESSAGE, MSG, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
    SM_YVIRTUALSCREEN, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_CLIPBOARDUPDATE, WM_TIMER,
    WNDCLASSW,
};

const KLASSE: PCWSTR = w!("FitcomBediening");
/// "Er staat iets in het kanaal": de motor wekt de thread hiermee.
const WM_WEK: u32 = WM_APP + 1;
const LOSLAAT_TIMER: usize = 1;
/// Zo lang zonder invoer, terwijl er een toets ingedrukt staat, en dan gaat hij los.
const DODE_MAN: Duration = Duration::from_millis(1500);
/// Het grootste wiel per gebeurtenis: tien klikken. Meer is geen muis meer.
const MAX_WIEL: i16 = 1200;

enum Opdracht {
    Invoer(Vec<InputEvent>),
    Klembord(String),
}

/// Het bedienen van één gedeelde monitor door één bediener. Weg = stoppen: de thread laat dan
/// alles los wat nog ingedrukt staat en ruimt zichzelf op. Geen `join` in `Drop`: de laatste
/// referentie valt op de motor, en die mag niet wachten.
pub struct Bediener {
    tx: Option<Sender<Opdracht>>,
    /// Het berichtvenster van de thread, als getal: een `HWND` mag niet tussen threads.
    hwnd: isize,
}

impl Bediener {
    /// `monitor` is het `HMONITOR` uit `Bron::handle`. `klembord_uit` krijgt elke verandering
    /// van het klembord van de deler, op de thread van de bediening.
    pub fn start(monitor: isize, klembord_uit: impl Fn(String) + Send + 'static) -> Result<Self> {
        let (tx, rx) = unbounded();
        let (klaar_tx, klaar_rx) = bounded::<Result<isize>>(1);
        std::thread::Builder::new()
            .name("fitcom-bediening".into())
            .spawn(move || {
                lus(
                    HMONITOR(monitor as *mut _),
                    rx,
                    klaar_tx,
                    Box::new(klembord_uit),
                )
            })
            .context("bedienthread starten")?;
        let hwnd = klaar_rx
            .recv_timeout(Duration::from_secs(2))
            .context("bedienthread reageert niet")??;
        Ok(Self { tx: Some(tx), hwnd })
    }

    /// Invoer van de bediener, al gecontroleerd en afgeremd door de motor.
    pub fn invoer(&self, events: Vec<InputEvent>) {
        self.stuur(Opdracht::Invoer(events));
    }

    /// Klembordtekst van de bediener.
    pub fn klembord(&self, tekst: String) {
        self.stuur(Opdracht::Klembord(tekst));
    }

    fn stuur(&self, opdracht: Opdracht) {
        if let Some(tx) = &self.tx {
            if tx.send(opdracht).is_ok() {
                self.wek();
            }
        }
    }

    fn wek(&self) {
        // SAFETY: een venster van een andere thread; is hij al weg, dan mislukt dit gewoon.
        let _ = unsafe {
            PostMessageW(
                Some(HWND(self.hwnd as *mut _)),
                WM_WEK,
                WPARAM(0),
                LPARAM(0),
            )
        };
    }
}

impl Drop for Bediener {
    fn drop(&mut self) {
        // Het kanaal dicht en wekken: de thread ziet dat, laat alles los en stopt.
        self.tx.take();
        self.wek();
    }
}

/// Waar een verandering van het klembord van de deler heen gaat.
type KlembordUit = Box<dyn Fn(String) + Send>;

thread_local! {
    /// Eén bedienthread, één venster, één bestemming.
    static KLEMBORD_UIT: RefCell<Option<KlembordUit>> = const { RefCell::new(None) };
}

fn lus(
    monitor: HMONITOR,
    rx: Receiver<Opdracht>,
    klaar: Sender<Result<isize>>,
    klembord_uit: KlembordUit,
) {
    let hwnd = match maak_venster() {
        Ok(h) => h,
        Err(e) => {
            let _ = klaar.send(Err(e));
            return;
        }
    };
    KLEMBORD_UIT.with(|u| *u.borrow_mut() = Some(klembord_uit));
    // SAFETY: een eigen venster op deze thread.
    let luistert = unsafe { AddClipboardFormatListener(hwnd) }.is_ok();
    if !luistert {
        tracing::warn!("klembord delen niet beschikbaar: AddClipboardFormatListener mislukt");
    }
    let _ = klaar.send(Ok(hwnd.0 as isize));
    tracing::info!("bedienen gestart");

    let mut ingedrukt = Ingedrukt::default();
    let mut laatst = Instant::now();
    let mut timer = false;
    let mut geen_monitor_gemeld = false;
    loop {
        let mut msg = MSG::default();
        // SAFETY: berichten van deze thread.
        if unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 <= 0 {
            break;
        }
        match msg.message {
            WM_WEK => {
                let mut stop = false;
                loop {
                    match rx.try_recv() {
                        Ok(Opdracht::Invoer(events)) => {
                            laatst = Instant::now();
                            let Some(kader) = monitor_kader(monitor) else {
                                if !geen_monitor_gemeld {
                                    tracing::warn!("gedeelde monitor weg; invoer genegeerd");
                                    geen_monitor_gemeld = true;
                                }
                                continue;
                            };
                            let virtueel = virtueel_bureaublad();
                            let inputs: Vec<INPUT> = events
                                .iter()
                                .filter_map(|e| {
                                    ingedrukt.noteer(e);
                                    naar_input(e, kader, virtueel)
                                })
                                .collect();
                            stuur_in(&inputs);
                        }
                        Ok(Opdracht::Klembord(tekst)) => {
                            if !klembord::schrijf(hwnd, &tekst) {
                                tracing::debug!(len = tekst.len(), "klembord zetten mislukt");
                            }
                        }
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => {
                            stop = true;
                            break;
                        }
                    }
                }
                if stop {
                    break;
                }
                if !ingedrukt.toetsen.is_empty() && !timer {
                    // SAFETY: eigen venster.
                    unsafe { SetTimer(Some(hwnd), LOSLAAT_TIMER, 500, None) };
                    timer = true;
                }
            }
            WM_TIMER if msg.wParam.0 == LOSLAAT_TIMER => {
                if !ingedrukt.toetsen.is_empty() && laatst.elapsed() >= DODE_MAN {
                    tracing::info!("bediener stil met een toets ingedrukt; losgelaten");
                    stuur_in(&ingedrukt.toetsen_los());
                }
                if ingedrukt.toetsen.is_empty() {
                    // SAFETY: eigen venster en timer.
                    let _ = unsafe { KillTimer(Some(hwnd), LOSLAAT_TIMER) };
                    timer = false;
                }
            }
            _ => {
                // SAFETY: berichten van deze thread.
                unsafe {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        }
    }

    stuur_in(&ingedrukt.alles_los());
    // SAFETY: eigen venster op deze thread; daarna raakt niemand hem nog aan.
    unsafe {
        if luistert {
            let _ = RemoveClipboardFormatListener(hwnd);
        }
        let _ = DestroyWindow(hwnd);
    }
    KLEMBORD_UIT.with(|u| *u.borrow_mut() = None);
    tracing::info!("bedienen gestopt");
}

fn maak_venster() -> Result<HWND> {
    use std::sync::Once;
    static EEN_KEER: Once = Once::new();
    // SAFETY: de klasse wordt één keer per proces geregistreerd; het venster is een
    // berichtvenster (`HWND_MESSAGE`) zonder iets om te tekenen.
    unsafe {
        let instantie = GetModuleHandleW(None).context("module-handle")?;
        EEN_KEER.call_once(|| {
            let klasse = WNDCLASSW {
                lpfnWndProc: Some(venster_proc),
                hInstance: instantie.into(),
                lpszClassName: KLASSE,
                ..Default::default()
            };
            RegisterClassW(&klasse);
        });
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            KLASSE,
            PCWSTR::null(),
            WINDOW_STYLE::default(),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(instantie.into()),
            None,
        )
        .context("berichtvenster voor bedienen")
    }
}

unsafe extern "system" fn venster_proc(
    hwnd: HWND,
    bericht: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if bericht == WM_CLIPBOARDUPDATE {
        if let Some(tekst) = klembord::verandering(hwnd) {
            KLEMBORD_UIT.with(|u| {
                if let Some(uit) = &*u.borrow() {
                    uit(tekst);
                }
            });
        }
        return LRESULT(0);
    }
    // SAFETY: doorgeven aan de standaardprocedure met wat Windows ons gaf.
    unsafe { DefWindowProcW(hwnd, bericht, wparam, lparam) }
}

fn stuur_in(inputs: &[INPUT]) {
    if inputs.is_empty() {
        return;
    }
    // SAFETY: een volledig ingevulde reeks `INPUT`s met de juiste grootte erbij.
    let gezet = unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
    if gezet as usize != inputs.len() {
        tracing::debug!(gezet, van = inputs.len(), "niet alle invoer aangenomen");
    }
}

/// Het hele virtuele bureaublad (alle monitoren samen) in fysieke pixels.
fn virtueel_bureaublad() -> RECT {
    // SAFETY: kale aanroepen zonder buffers.
    unsafe {
        let (x, y) = (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
        );
        RECT {
            left: x,
            top: y,
            right: x + GetSystemMetrics(SM_CXVIRTUALSCREEN),
            bottom: y + GetSystemMetrics(SM_CYVIRTUALSCREEN),
        }
    }
}

/// Een plek op het gedeelde beeld naar de absolute coördinaten van `SendInput` met
/// `MOUSEEVENTF_VIRTUALDESK`: eerst naar een pixel binnen de monitor (de rand inbegrepen, en
/// nooit erbuiten), dan naar het midden van die pixel op de schaal 0..65535 van het hele
/// bureaublad. Windows rekent terug met `links + a * breedte / 65536`.
pub(crate) fn naar_bureaublad([x, y]: [u16; 2], monitor: RECT, virtueel: RECT) -> (i32, i32) {
    let as_ = |n: u16, lo: i32, hi: i32, vlo: i32, vhi: i32| -> i32 {
        let maat = i64::from((hi - lo).max(1));
        let px = (i64::from(lo) + (i64::from(n) * (maat - 1) + 32767) / 65535)
            .clamp(i64::from(lo), i64::from(lo) + maat - 1);
        // Het midden van de pixel: zo maakt het voor de afronding van Windows niet uit of hij
        // door 65535 of 65536 deelt.
        let vmaat = i64::from((vhi - vlo).max(1));
        ((2 * (px - i64::from(vlo)) + 1) * 65536)
            .div_euclid(2 * vmaat)
            .clamp(0, 65535) as i32
    };
    (
        as_(
            x,
            monitor.left,
            monitor.right,
            virtueel.left,
            virtueel.right,
        ),
        as_(
            y,
            monitor.top,
            monitor.bottom,
            virtueel.top,
            virtueel.bottom,
        ),
    )
}

fn muis(dx: i32, dy: i32, vlaggen: MOUSE_EVENT_FLAGS, data: u32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: data,
                dwFlags: vlaggen,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn toets(scan: u16, vk: u8, los: bool) -> INPUT {
    let mut vlaggen = if los {
        KEYEVENTF_KEYUP
    } else {
        KEYBD_EVENT_FLAGS(0)
    };
    let (wvk, wscan) = if scan != 0 {
        vlaggen |= KEYEVENTF_SCANCODE;
        if scan >> 8 == 0xE0 {
            vlaggen |= KEYEVENTF_EXTENDEDKEY;
        }
        (0, scan & 0xFF)
    } else {
        (u16::from(vk), 0)
    };
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(wvk),
                wScan: wscan,
                dwFlags: vlaggen,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// De vlag en `mouseData` voor knop 1-5, omlaag of omhoog.
fn knop(knop: u8, los: bool) -> Option<(MOUSE_EVENT_FLAGS, u32)> {
    Some(match (knop, los) {
        (1, false) => (MOUSEEVENTF_LEFTDOWN, 0),
        (1, true) => (MOUSEEVENTF_LEFTUP, 0),
        (2, false) => (MOUSEEVENTF_RIGHTDOWN, 0),
        (2, true) => (MOUSEEVENTF_RIGHTUP, 0),
        (3, false) => (MOUSEEVENTF_MIDDLEDOWN, 0),
        (3, true) => (MOUSEEVENTF_MIDDLEUP, 0),
        (4 | 5, false) => (MOUSEEVENTF_XDOWN, u32::from(knop - 3)),
        (4 | 5, true) => (MOUSEEVENTF_XUP, u32::from(knop - 3)),
        _ => return None,
    })
}

/// Eén gebeurtenis als `INPUT`. Elke muisgebeurtenis zet de muis ook op zijn plek, in dezelfde
/// `INPUT`: een klik valt altijd waar de bediener klikte, ook als een eerdere beweging wegviel.
pub(crate) fn naar_input(e: &InputEvent, monitor: RECT, virtueel: RECT) -> Option<INPUT> {
    let (dx, dy) = naar_bureaublad([e.x, e.y], monitor, virtueel);
    let plek = MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK;
    match e.kind {
        InputKind::MOVE => Some(muis(dx, dy, plek, 0)),
        InputKind::BUTTON_DOWN | InputKind::BUTTON_UP => {
            let (vlag, data) = knop(e.button, e.kind == InputKind::BUTTON_UP)?;
            Some(muis(dx, dy, plek | vlag, data))
        }
        InputKind::WHEEL | InputKind::HWHEEL => {
            let vlag = if e.kind == InputKind::WHEEL {
                MOUSEEVENTF_WHEEL
            } else {
                MOUSEEVENTF_HWHEEL
            };
            let delta = e.delta.clamp(-MAX_WIEL, MAX_WIEL);
            Some(muis(dx, dy, plek | vlag, i32::from(delta) as u32))
        }
        InputKind::KEY_DOWN | InputKind::KEY_UP if e.is_geldig() => {
            Some(toets(e.scan, e.vk, e.kind == InputKind::KEY_UP))
        }
        _ => None,
    }
}

/// Wat de bediener nu ingedrukt houdt bij de deler.
#[derive(Debug, Default)]
pub(crate) struct Ingedrukt {
    toetsen: BTreeSet<(u16, u8)>,
    /// Bit `n` voor knop `n` (1-5).
    knoppen: u8,
}

impl Ingedrukt {
    fn noteer(&mut self, e: &InputEvent) {
        match e.kind {
            InputKind::KEY_DOWN if e.is_geldig() => {
                self.toetsen.insert((e.scan, e.vk));
            }
            InputKind::KEY_UP => {
                self.toetsen.remove(&(e.scan, e.vk));
            }
            InputKind::BUTTON_DOWN if (1..=5).contains(&e.button) => self.knoppen |= 1 << e.button,
            InputKind::BUTTON_UP if (1..=5).contains(&e.button) => self.knoppen &= !(1 << e.button),
            _ => {}
        }
    }

    /// Elke ingedrukte toets los.
    fn toetsen_los(&mut self) -> Vec<INPUT> {
        std::mem::take(&mut self.toetsen)
            .into_iter()
            .map(|(scan, vk)| toets(scan, vk, true))
            .collect()
    }

    /// Alles los: toetsen en knoppen. Knoppen zonder de muis te verplaatsen.
    fn alles_los(&mut self) -> Vec<INPUT> {
        let mut uit = self.toetsen_los();
        for b in 1..=5u8 {
            if self.knoppen & (1 << b) != 0 {
                if let Some((vlag, data)) = knop(b, true) {
                    uit.push(muis(0, 0, vlag, data));
                }
            }
        }
        self.knoppen = 0;
        uit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT {
            left,
            top,
            right,
            bottom,
        }
    }

    /// Wat Windows van een absolute coördinaat maakt (`links + a * breedte / 65536`). Of hij
    /// door 65535 deelt maakt niet uit: dat moet dezelfde pixel geven.
    fn terug(a: i32, vlo: i32, vhi: i32) -> i32 {
        let p = vlo + (i64::from(a) * i64::from(vhi - vlo)).div_euclid(65536) as i32;
        let q = vlo + (i64::from(a) * i64::from(vhi - vlo)).div_euclid(65535) as i32;
        assert_eq!(p, q, "a = {a}");
        p
    }

    #[test]
    fn een_plek_komt_altijd_binnen_de_gedeelde_monitor() {
        // Twee monitoren: de gedeelde links van het hoofdscherm, op een negatieve x, en het
        // hoofdscherm hoger dan de ander.
        let gedeeld = rect(-1920, 120, 0, 1200);
        let virtueel = rect(-1920, 0, 2560, 1440);
        for x in [0u16, 1, 32767, 65534, 65535] {
            for y in [0u16, 40000, 65535] {
                let (ax, ay) = naar_bureaublad([x, y], gedeeld, virtueel);
                let (px, py) = (terug(ax, -1920, 2560), terug(ay, 0, 1440));
                assert!((-1920..0).contains(&px), "x {x} -> {px}");
                assert!((120..1200).contains(&py), "y {y} -> {py}");
            }
        }
        // De hoeken zijn de hoekpixels, niet één ernaast.
        let (ax, ay) = naar_bureaublad([0, 0], gedeeld, virtueel);
        assert_eq!((terug(ax, -1920, 2560), terug(ay, 0, 1440)), (-1920, 120));
        let (ax, ay) = naar_bureaublad([65535, 65535], gedeeld, virtueel);
        assert_eq!((terug(ax, -1920, 2560), terug(ay, 0, 1440)), (-1, 1199));
    }

    #[test]
    fn het_midden_van_een_enkel_scherm_is_het_midden() {
        let scherm = rect(0, 0, 1920, 1080);
        let (ax, ay) = naar_bureaublad([32768, 32768], scherm, scherm);
        assert_eq!((terug(ax, 0, 1920), terug(ay, 0, 1080)), (960, 540));
        // Nooit buiten 0..=65535, ook niet bij een rare (lege) rechthoek.
        let leeg = rect(5, 5, 5, 5);
        let (ax, ay) = naar_bureaublad([65535, 65535], leeg, scherm);
        assert!((0..=65535).contains(&ax) && (0..=65535).contains(&ay));
    }

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
    fn invoer_wordt_de_juiste_input() {
        let scherm = rect(0, 0, 1920, 1080);
        // SAFETY (alle drie): we lezen het veld dat bij het `type` hoort.
        let rechts = naar_input(
            &InputEvent {
                button: 2,
                ..e(InputKind::BUTTON_DOWN)
            },
            scherm,
            scherm,
        )
        .unwrap();
        assert_eq!(rechts.r#type, INPUT_MOUSE);
        let mi = unsafe { rechts.Anonymous.mi };
        assert_eq!(
            mi.dwFlags,
            MOUSEEVENTF_MOVE
                | MOUSEEVENTF_ABSOLUTE
                | MOUSEEVENTF_VIRTUALDESK
                | MOUSEEVENTF_RIGHTDOWN
        );

        let wiel = naar_input(
            &InputEvent {
                delta: -30000,
                ..e(InputKind::WHEEL)
            },
            scherm,
            scherm,
        )
        .unwrap();
        let mi = unsafe { wiel.Anonymous.mi };
        assert_eq!(mi.mouseData as i32, -1200, "geklemd");

        let ctrl = naar_input(
            &InputEvent {
                scan: 0xE01D,
                ..e(InputKind::KEY_UP)
            },
            scherm,
            scherm,
        )
        .unwrap();
        let ki = unsafe { ctrl.Anonymous.ki };
        assert_eq!(ki.wScan, 0x1D);
        assert_eq!(
            ki.dwFlags,
            KEYEVENTF_KEYUP | KEYEVENTF_SCANCODE | KEYEVENTF_EXTENDEDKEY
        );

        assert!(naar_input(&e(InputKind(9)), scherm, scherm).is_none());
        assert!(
            naar_input(&e(InputKind::KEY_DOWN), scherm, scherm).is_none(),
            "geen toets"
        );
        assert!(naar_input(
            &InputEvent {
                button: 9,
                ..e(InputKind::BUTTON_UP)
            },
            scherm,
            scherm
        )
        .is_none());
    }

    #[test]
    fn wat_ingedrukt_staat_gaat_bij_het_einde_los() {
        let mut i = Ingedrukt::default();
        let shift = InputEvent {
            scan: 0x2A,
            ..e(InputKind::KEY_DOWN)
        };
        let a = InputEvent {
            scan: 0x1E,
            ..e(InputKind::KEY_DOWN)
        };
        i.noteer(&shift);
        i.noteer(&a);
        i.noteer(&InputEvent {
            kind: InputKind::KEY_UP,
            ..a
        });
        i.noteer(&InputEvent {
            button: 1,
            ..e(InputKind::BUTTON_DOWN)
        });
        i.noteer(&InputEvent {
            button: 4,
            ..e(InputKind::BUTTON_DOWN)
        });
        let los = i.alles_los();
        assert_eq!(los.len(), 3, "Shift, links en X1; A was al los");
        let ki = unsafe { los[0].Anonymous.ki };
        assert_eq!(
            (ki.wScan, ki.dwFlags & KEYEVENTF_KEYUP),
            (0x2A, KEYEVENTF_KEYUP)
        );
        let x1 = unsafe { los[2].Anonymous.mi };
        assert_eq!((x1.dwFlags, x1.mouseData), (MOUSEEVENTF_XUP, 1));
        assert_eq!(
            unsafe { los[1].Anonymous.mi }.dwFlags & MOUSEEVENTF_ABSOLUTE,
            MOUSE_EVENT_FLAGS(0),
            "loslaten verplaatst de muis niet"
        );
        assert!(i.alles_los().is_empty(), "daarna niets meer");
    }
}
