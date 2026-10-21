//! De spraakoverlay (beslissing 53): een klein, doorklikbaar venster bovenop alles met wie er
//! in het gesprek zit. Elke naam staat gedimd en licht op terwijl die persoon spreekt.
//!
//! Hetzelfde soort venster als de tekenlaag ([`crate::tekenlaag`]): layered, bovenop alles,
//! zonder focus, niet in alt-tab, buiten de schermopname (`WDA_EXCLUDEFROMCAPTURE`; lukt dat
//! niet, dan is er geen overlay), en alleen opnieuw getekend als de inhoud verandert, hooguit
//! eens per [`MIN_HERTEKEN`]. Pillen en stippen komen uit [`crate::penseel`]; alleen de tekst
//! is GDI, als grijswaarden in hetzelfde beeld en daarna met de eigen kleur erover gelegd
//! (GDI schrijft geen alfa, en ClearType geeft gekleurde randen).
//!
//! Geen injectie in een spel, nooit: dit is een gewoon venster van de compositor. Het staat
//! dus over randloze en venster-spellen, niet over exclusief schermvullend. Anders dan de
//! tekenlaag wijkt hij **niet** voor een schermvullend venster vooraan; dat is juist waar hij
//! voor is. De prijs staat in de moduledoc van de tekenlaag: een venster bovenop een randloos
//! spel kan dat spel uit independent flip halen.
//!
//! # Verplaatsen met Alt + slepen
//!
//! Zolang de overlay staat, kijkt de thread elke [`ALT_INTERVAL`] met `GetAsyncKeyState` of
//! Alt omlaag is (geen hook, geen `RegisterHotKey`: beslissing 42). Dan gaat
//! `WS_EX_TRANSPARENT` eraf, is het venster raakbaar en zegt het overal "titelbalk"
//! (`HTCAPTION`), zodat Windows zelf het slepen doet. Na het loslaten gaat hij weer
//! doorklikbaar, en een nieuwe plek gaat één keer naar de aanroeper om te bewaren. Alleen de
//! pillen zijn raakbaar: een pixel met alfa 0 laat een klik altijd door.

use crate::penseel::{Doek, Kader, MIN_HERTEKEN};
use crate::tekenlaag::{monitor_kader, registreer_klasse, Laag};
use anyhow::{Context, Result};
use crossbeam_channel::{unbounded, Receiver, RecvTimeoutError, Sender};
use std::sync::{Once, OnceLock};
use std::time::{Duration, Instant};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    AddFontMemResourceEx, CreateFontW, DeleteObject, DrawTextW, GdiFlush, GetTextExtentPoint32W,
    MonitorFromPoint, SelectObject, SetBkMode, SetTextColor, ANTIALIASED_QUALITY,
    CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX,
    DT_SINGLELINE, DT_VCENTER, FW_NORMAL, HDC, HFONT, HMONITOR, MONITOR_DEFAULTTONULL,
    MONITOR_DEFAULTTOPRIMARY, OUT_TT_PRECIS, TRANSPARENT,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_MENU};
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, DispatchMessageW, GetWindowLongW, GetWindowRect, PeekMessageW, SetWindowLongW,
    SetWindowPos, TranslateMessage, GWL_EXSTYLE, HTCAPTION, HWND_TOPMOST, MA_NOACTIVATE, MSG,
    PM_REMOVE, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
    WM_MOUSEACTIVATE, WM_NCHITTEST, WS_EX_TRANSPARENT,
};

const KLASSE: PCWSTR = w!("FitcomSpraaklaag");

/// Hoe vaak de thread naar Alt kijkt en berichten pompt, alleen zolang de overlay staat.
/// Een handvol goedkope aanroepen; slepen begint hooguit zo veel later.
const ALT_INTERVAL: Duration = Duration::from_millis(50);

// Maten bij 96 dpi; geschaald met de dpi van de monitor waar hij staat.
const RIJ: i32 = 28;
const TUSSEN: i32 = 4;
const STIP: i32 = 20;
const STIP_LINKS: i32 = 4;
const RING: i32 = 2;
const NAAM_X: i32 = 32;
const RECHTS: i32 = 12;
const NAAM_MAX: i32 = 200;
const LETTER: i32 = 14;
const MONOGRAM: i32 = 11;
/// Afstand tot de rand van het hoofdscherm zolang er nog geen eigen plek is.
const MARGE: i32 = 16;

/// Eén rij: een naam, en of die persoon nu spreekt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spreker {
    pub naam: String,
    pub spreekt: bool,
}

/// Kleuren en lettertype, van de aanroeper: de tokens staan in de app, niet hier.
#[derive(Clone, Copy)]
pub struct Stijl {
    /// De pil achter elke rij, met zijn dekking.
    pub grond: [u8; 3],
    pub grond_alfa: f32,
    /// De stip met de beginletter.
    pub stip: [u8; 3],
    /// De ring om de stip terwijl iemand spreekt.
    pub ring: [u8; 3],
    /// Tekst gedimd en opgelicht.
    pub stil: [u8; 3],
    pub spreekt: [u8; 3],
    /// Een TrueType-bestand en zijn GDI-familienaam (naam-id 1, bv. "Inter Medium").
    pub font: &'static [u8],
    pub font_naam: &'static str,
}

/// De overlay. Leeft op een eigen thread zolang deze waarde bestaat; de aanroeper maakt hem
/// als er iets te tonen is en laat hem vallen als dat niet meer zo is.
pub struct Spraaklaag {
    tx: Sender<Vec<Spreker>>,
}

impl Spraaklaag {
    /// `plek`: de bewaarde linkerbovenhoek, of `None` voor linksboven op het hoofdscherm.
    /// `verplaatst` krijgt de nieuwe hoek na elke keer slepen, op de overlay-thread.
    pub fn start(
        plek: Option<(i32, i32)>,
        stijl: Stijl,
        verplaatst: impl Fn(i32, i32) + Send + 'static,
    ) -> Result<Self> {
        let (tx, rx) = unbounded();
        std::thread::Builder::new()
            .name("fitcom-spraaklaag".into())
            .spawn(move || lus(rx, plek, stijl, verplaatst))
            .context("spraaklaag-thread starten")?;
        Ok(Self { tx })
    }

    /// Wie er nu in het gesprek zit, in de volgorde van tonen.
    pub fn toon(&self, sprekers: Vec<Spreker>) {
        let _ = self.tx.send(sprekers);
    }
}

// Geen `Drop` met `join`: de aanroeper zit op een tokio-worker, en de thread kan midden in een
// sleep van Windows zitten. Het kanaal dicht is genoeg; de thread ruimt zijn venster zelf op.

fn lus(
    rx: Receiver<Vec<Spreker>>,
    mut plek: Option<(i32, i32)>,
    stijl: Stijl,
    verplaatst: impl Fn(i32, i32),
) {
    static EEN_KEER: Once = Once::new();
    let laag =
        registreer_klasse(KLASSE, spraak_proc, &EEN_KEER).and_then(|_| Laag::maak(KLASSE, None));
    let mut laag = match laag {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!(error = %format!("{e:#}"), "geen spraakoverlay");
            for _ in rx {}
            return;
        }
    };
    laad_font(&stijl);
    let mut fonts: Option<Fonts> = None;
    let mut sprekers: Vec<Spreker> = Vec::new();
    let mut vuil = false;
    let mut laatst = Instant::now()
        .checked_sub(MIN_HERTEKEN)
        .unwrap_or_else(Instant::now);
    // Waar hij nu staat, zoals wij hem neerzetten; een verschil is slepen.
    let mut kader: Option<RECT> = None;
    let mut grijpbaar = false;
    loop {
        let mut msg = MSG::default();
        // SAFETY: berichten van deze thread. Slepen draait hierin, tot de knop los is.
        unsafe {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        if grijpbaar {
            if let (Some(k), Some(r)) = (kader, venster_kader(laag.hwnd())) {
                if (r.left, r.top) != (k.left, k.top) {
                    plek = Some((r.left, r.top));
                    kader = Some(r);
                    verplaatst(r.left, r.top);
                    vuil = true;
                }
            }
        }
        match rx.recv_timeout(ALT_INTERVAL) {
            Ok(s) => {
                sprekers = rx.try_iter().last().unwrap_or(s);
                vuil = true;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        // SAFETY: kale toetsstatus, geen eigendom.
        let alt = unsafe { GetAsyncKeyState(i32::from(VK_MENU.0)) } < 0;
        if alt != grijpbaar {
            zet_grijpbaar(laag.hwnd(), alt);
            grijpbaar = alt;
        }
        if vuil && laatst.elapsed() >= MIN_HERTEKEN {
            if sprekers.is_empty() {
                laag.verberg();
                kader = None;
            } else {
                kader = teken(&mut laag, &mut fonts, &sprekers, plek, &stijl);
            }
            vuil = false;
            laatst = Instant::now();
        }
    }
}

/// Het eigen lettertype, één keer per proces en alleen voor dit proces. Mislukt het, dan
/// kiest GDI zelf iets dat erop lijkt.
fn laad_font(stijl: &Stijl) {
    static GELADEN: OnceLock<()> = OnceLock::new();
    GELADEN.get_or_init(|| {
        let mut aantal = 0u32;
        // SAFETY: de bytes zijn `'static`; Windows kopieert ze.
        let h = unsafe {
            AddFontMemResourceEx(
                stijl.font.as_ptr().cast(),
                stijl.font.len() as u32,
                None,
                (&raw mut aantal).cast_const(),
            )
        };
        if h.is_invalid() {
            tracing::debug!("spraaklaag: lettertype niet geladen, GDI kiest er een");
        }
    });
}

struct Fonts {
    dpi: u32,
    naam: HFONT,
    monogram: HFONT,
}

impl Fonts {
    fn nieuw(stijl: &Stijl, dpi: u32) -> Self {
        let naam: Vec<u16> = stijl.font_naam.encode_utf16().chain([0]).collect();
        let maak = |px: i32| {
            // SAFETY: `naam` eindigt op 0 en leeft tot na de aanroep.
            unsafe {
                CreateFontW(
                    -schaal(px, dpi),
                    0,
                    0,
                    0,
                    FW_NORMAL.0 as i32,
                    0,
                    0,
                    0,
                    DEFAULT_CHARSET,
                    OUT_TT_PRECIS,
                    CLIP_DEFAULT_PRECIS,
                    ANTIALIASED_QUALITY,
                    0,
                    PCWSTR(naam.as_ptr()),
                )
            }
        };
        Self {
            dpi,
            naam: maak(LETTER),
            monogram: maak(MONOGRAM),
        }
    }
}

impl Drop for Fonts {
    fn drop(&mut self) {
        // SAFETY: eigen objecten, nergens meer geselecteerd (`Opmaak::nieuw` en `schilder`
        // zetten de oude terug).
        unsafe {
            let _ = DeleteObject(self.naam.into());
            let _ = DeleteObject(self.monogram.into());
        }
    }
}

fn schaal(v: i32, dpi: u32) -> i32 {
    (v * dpi as i32 + 48) / 96
}

/// Tekent de rijen en zet het venster neer. Levert waar het nu staat.
fn teken(
    laag: &mut Laag,
    fonts: &mut Option<Fonts>,
    sprekers: &[Spreker],
    plek: Option<(i32, i32)>,
    stijl: &Stijl,
) -> Option<RECT> {
    // SAFETY: kale opzoekingen, geen eigendom.
    let (bij_plek, hoofd) = unsafe {
        let bij_plek = plek
            .map(|(x, y)| MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONULL))
            .filter(|m| !m.is_invalid());
        (
            bij_plek,
            MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY),
        )
    };
    let hoofd_kader = monitor_kader(hoofd)?;
    let scherm = bij_plek.and_then(monitor_kader);
    let dpi = dpi_van(bij_plek.unwrap_or(hoofd));
    if fonts.as_ref().is_none_or(|f| f.dpi != dpi) {
        *fonts = Some(Fonts::nieuw(stijl, dpi));
    }
    let f = fonts.as_ref()?;
    let o = Opmaak::nieuw(laag.dc(), f, sprekers);
    let (x, y) = plaats(plek, o.maat, scherm, hoofd_kader, schaal(MARGE, dpi));
    let kader = RECT {
        left: x,
        top: y,
        right: x + o.maat.0,
        bottom: y + o.maat.1,
    };
    laag.toon_beeld(kader, |doek, dc| schilder(doek, dc, f, sprekers, &o, stijl));
    // Een spel dat na ons bovenop ging staan zou hem anders begraven. Alleen bij een
    // wijziging, dus gratis.
    // SAFETY: eigen venster; niet verplaatsen, niet vergroten, geen focus.
    unsafe {
        let _ = SetWindowPos(
            laag.hwnd(),
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
    Some(kader)
}

/// Wat er per rij staat en hoe groot het geheel wordt, in pixels bij `Fonts::dpi`.
struct Opmaak {
    dpi: u32,
    namen: Vec<Vec<u16>>,
    letters: Vec<Vec<u16>>,
    /// De breedte van elke naam, afgekapt op `NAAM_MAX` (daar komt een beletselteken).
    breedtes: Vec<i32>,
    maat: (i32, i32),
}

impl Opmaak {
    /// `dc`: een DC om de namen in te meten; het lettertype gaat er even in en weer uit.
    fn nieuw(dc: HDC, f: &Fonts, sprekers: &[Spreker]) -> Self {
        let s = |v: i32| schaal(v, f.dpi);
        let namen: Vec<Vec<u16>> = sprekers
            .iter()
            .map(|p| p.naam.encode_utf16().collect())
            .collect();
        let letters = sprekers
            .iter()
            .map(|p| {
                p.naam
                    .chars()
                    .next()
                    .map(|c| c.to_uppercase().collect::<String>())
                    .unwrap_or_default()
                    .encode_utf16()
                    .collect()
            })
            .collect();
        // SAFETY: het lettertype tijdelijk in `dc`, en daarna de oude terug.
        let breedtes: Vec<i32> = unsafe {
            let oud = SelectObject(dc, f.naam.into());
            let b = namen
                .iter()
                .map(|n| {
                    let mut maat = SIZE::default();
                    let _ = GetTextExtentPoint32W(dc, n, &mut maat);
                    maat.cx.min(s(NAAM_MAX))
                })
                .collect();
            SelectObject(dc, oud);
            b
        };
        let maat = (
            breedtes
                .iter()
                .map(|&b| s(NAAM_X) + b + s(RECHTS))
                .max()
                .unwrap_or(1),
            sprekers.len() as i32 * s(RIJ + TUSSEN) - s(TUSSEN),
        );
        Self {
            dpi: f.dpi,
            namen,
            letters,
            breedtes,
            maat,
        }
    }

    fn s(&self, v: i32) -> i32 {
        schaal(v, self.dpi)
    }

    fn rij(&self, i: usize) -> i32 {
        i as i32 * self.s(RIJ + TUSSEN)
    }

    fn stip(&self, i: usize) -> RECT {
        let (rij, s) = (self.rij(i), |v| self.s(v));
        RECT {
            left: s(STIP_LINKS),
            top: rij + (s(RIJ) - s(STIP)) / 2,
            right: s(STIP_LINKS) + s(STIP),
            bottom: rij + (s(RIJ) + s(STIP)) / 2,
        }
    }

    fn naam(&self, i: usize) -> RECT {
        RECT {
            left: self.s(NAAM_X),
            top: self.rij(i),
            right: self.s(NAAM_X) + self.breedtes[i],
            bottom: self.rij(i) + self.s(RIJ),
        }
    }
}

/// Het hele beeld op `doek` (dat in `dc` geselecteerd is): eerst alle tekst wit op zwart
/// als masker, dan de pillen en stippen, dan de tekst in zijn eigen kleur erover.
fn schilder(doek: &mut Doek, dc: HDC, f: &Fonts, sprekers: &[Spreker], o: &Opmaak, stijl: &Stijl) {
    doek.wis();
    // SAFETY: eigen DC met het beeld erin; lettertypes eigen, daarna de oude terug.
    unsafe {
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, COLORREF(0x00FF_FFFF));
        let oud = SelectObject(dc, f.naam.into());
        for (i, n) in o.namen.iter().enumerate() {
            let mut r = o.naam(i);
            DrawTextW(
                dc,
                &mut n.clone(),
                &mut r,
                DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
            );
        }
        SelectObject(dc, f.monogram.into());
        for (i, l) in o.letters.iter().enumerate() {
            let mut r = o.stip(i);
            DrawTextW(
                dc,
                &mut l.clone(),
                &mut r,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
            );
        }
        SelectObject(dc, oud);
        let _ = GdiFlush();
    }
    let masker: Vec<u8> = doek
        .px
        .chunks_exact(4)
        .map(|p| p[0].max(p[1]).max(p[2]))
        .collect();

    doek.wis();
    let half = o.s(RIJ) as f32 / 2.0;
    for (i, p) in sprekers.iter().enumerate() {
        let midden = o.rij(i) as f32 + half;
        let rechts = (o.s(NAAM_X) + o.breedtes[i] + o.s(RECHTS)) as f32 - half;
        doek.capsule(
            (half, midden),
            (rechts, midden),
            half,
            stijl.grond,
            stijl.grond_alfa,
        );
        let st = o.stip(i);
        let c = (
            (st.left + st.right) as f32 / 2.0,
            (st.top + st.bottom) as f32 / 2.0,
        );
        let r = o.s(STIP) as f32 / 2.0;
        if p.spreekt {
            doek.capsule(c, c, r, stijl.ring, 1.0);
            doek.capsule(c, c, r - o.s(RING) as f32, stijl.stip, 1.0);
        } else {
            doek.capsule(c, c, r, stijl.stip, 1.0);
        }
        let kleur = if p.spreekt { stijl.spreekt } else { stijl.stil };
        for r in [o.naam(i), st] {
            doek.bedek(&masker, kader_van(r), kleur);
        }
    }
}

fn kader_van(r: RECT) -> Kader {
    Kader {
        x0: r.left,
        y0: r.top,
        x1: r.right,
        y1: r.bottom,
    }
}

fn dpi_van(monitor: HMONITOR) -> u32 {
    let (mut x, mut y) = (96u32, 96u32);
    // SAFETY: twee eigen uitvoervelden.
    match unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y) } {
        Ok(()) if x > 0 => x,
        _ => 96,
    }
}

fn venster_kader(hwnd: HWND) -> Option<RECT> {
    let mut r = RECT::default();
    // SAFETY: eigen venster, eigen uitvoerveld.
    unsafe { GetWindowRect(hwnd, &mut r) }.ok().map(|_| r)
}

/// Met Alt omlaag raakbaar, anders doorklikbaar.
fn zet_grijpbaar(hwnd: HWND, aan: bool) {
    // SAFETY: eigen venster op eigen thread; alleen de doorklik-stijl wisselt.
    unsafe {
        let stijl = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        let nieuw = if aan {
            stijl & !WS_EX_TRANSPARENT.0
        } else {
            stijl | WS_EX_TRANSPARENT.0
        };
        SetWindowLongW(hwnd, GWL_EXSTYLE, nieuw as i32);
        // Windows leest een gewijzigde stijl pas na een SetWindowPos met FRAMECHANGED.
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }
}

/// Overal "titelbalk", zodat Windows zelf sleept; klikken pakt nooit de focus. Zolang
/// `WS_EX_TRANSPARENT` erop staat komt hier geen muisbericht.
unsafe extern "system" fn spraak_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_NCHITTEST => LRESULT(HTCAPTION as isize),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        // SAFETY: doorgeven aan de standaardprocedure met wat Windows ons gaf.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Waar de overlay van `maat` komt. `scherm` is de monitor onder de bewaarde `plek`; is die
/// er niet (meer), of is er nog geen plek, dan linksboven op `hoofd` met `marge`. Altijd
/// helemaal binnen dat scherm.
fn plaats(
    plek: Option<(i32, i32)>,
    maat: (i32, i32),
    scherm: Option<RECT>,
    hoofd: RECT,
    marge: i32,
) -> (i32, i32) {
    let (p, s) = match (plek, scherm) {
        (Some(p), Some(s)) => (p, s),
        _ => ((hoofd.left + marge, hoofd.top + marge), hoofd),
    };
    (
        p.0.min(s.right - maat.0).max(s.left),
        p.1.min(s.bottom - maat.1).max(s.top),
    )
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

    /// Zonder venster, in een eigen geheugen-DC: het lettertype uit de app laadt onder zijn
    /// GDI-naam, de tekst landt als masker en krijgt zijn kleur, de pil is halfdoorzichtig,
    /// en tussen de rijen blijft alles doorzichtig (daar klikt een muis altijd doorheen).
    #[test]
    fn de_rijen_worden_getekend_zonder_venster() {
        use windows::Win32::Graphics::Gdi::{
            CreateCompatibleDC, CreateDIBSection, DeleteDC, GetTextFaceW, BITMAPINFO,
            BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
        };
        let stijl = Stijl {
            grond: [0, 0, 0],
            grond_alfa: 0.85,
            stip: [11, 14, 20],
            ring: [58, 211, 137],
            stil: [161, 164, 165],
            spreekt: [255, 255, 255],
            font: include_bytes!("../../app/src/ui/assets/fonts/Inter-Medium.ttf"),
            font_naam: "Inter Medium",
        };
        laad_font(&stijl);
        let f = Fonts::nieuw(&stijl, 96);
        let sprekers = [
            Spreker {
                naam: "Rick".into(),
                spreekt: false,
            },
            Spreker {
                naam: "Anna met een heel lange naam die nooit past in de overlay".into(),
                spreekt: true,
            },
        ];
        // SAFETY: eigen DC en bitmap, opgeruimd voor we terugkeren.
        unsafe {
            let dc = CreateCompatibleDC(None);
            let o = Opmaak::nieuw(dc, &f, &sprekers);
            assert_eq!(o.maat.1, 2 * RIJ + TUSSEN);
            assert!(
                o.maat.0 <= NAAM_X + NAAM_MAX + RECHTS,
                "lange naam afgekapt"
            );

            let oud = SelectObject(dc, f.naam.into());
            let mut face = [0u16; 32];
            GetTextFaceW(dc, Some(&mut face));
            let n = face.iter().position(|&c| c == 0).unwrap_or(face.len());
            SelectObject(dc, oud);
            let face = String::from_utf16_lossy(&face[..n]);
            assert_eq!(face, "Inter Medium");

            let (b, h) = (o.maat.0 as u32, o.maat.1 as u32);
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: b as i32,
                    biHeight: -(h as i32),
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            let bitmap =
                CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0).unwrap();
            let oud = SelectObject(dc, bitmap.into());
            let px = std::slice::from_raw_parts_mut(bits.cast::<u8>(), (b * h * 4) as usize);
            let mut doek = Doek {
                px,
                breedte: b,
                hoogte: h,
            };
            schilder(&mut doek, dc, &f, &sprekers, &o, &stijl);
            let px = doek.px.to_vec();
            SelectObject(dc, oud);
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(dc);

            let at = |x: i32, y: i32| px[((y as u32 * b + x as u32) * 4) as usize..][..4].to_vec();
            let in_rect = |r: RECT| {
                (r.top..r.bottom)
                    .flat_map(move |y| (r.left..r.right).map(move |x| (x, y)))
                    .collect::<Vec<_>>()
            };
            // Opgelicht: witte tekst in de naam van rij 2. Gedimd: niets wits in rij 1.
            let wit = |(x, y): (i32, i32)| at(x, y)[..3] == [255, 255, 255];
            assert!(in_rect(o.naam(1)).into_iter().any(wit), "opgelichte naam");
            assert!(!in_rect(o.naam(0)).into_iter().any(wit), "gedimde naam");
            assert!(
                in_rect(o.naam(0))
                    .into_iter()
                    .any(|(x, y)| at(x, y)[0] > 100),
                "gedimde naam staat er wel"
            );
            // De pil: zwart op 85 %, rechts van de lange naam waar geen tekst staat.
            assert_eq!(at(o.maat.0 - RECHTS / 2, RIJ + TUSSEN + RIJ / 2)[3], 217);
            // Tussen de rijen: doorzichtig.
            assert_eq!(at(o.maat.0 / 2, RIJ + TUSSEN / 2)[3], 0);
            // Voorvermenigvuldigd, overal.
            assert!(px
                .chunks_exact(4)
                .all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]));
        }
    }

    #[test]
    fn de_plek_blijft_op_een_zichtbaar_scherm() {
        let hoofd = rect(0, 0, 1920, 1080);
        let links = rect(-2560, 0, 0, 1440);
        // Nog geen plek: linksboven op het hoofdscherm.
        assert_eq!(plaats(None, (200, 100), None, hoofd, 16), (16, 16));
        // Een plek op het linkerscherm blijft staan.
        assert_eq!(
            plaats(Some((-2500, 40)), (200, 100), Some(links), hoofd, 16),
            (-2500, 40)
        );
        // Over de rand: terug naar binnen, ook als de overlay groeit.
        assert_eq!(
            plaats(Some((1800, 1050)), (200, 100), Some(hoofd), hoofd, 16),
            (1720, 980)
        );
        // Het scherm onder de plek is weg: linksboven op het hoofdscherm.
        assert_eq!(
            plaats(Some((-2500, 40)), (200, 100), None, hoofd, 16),
            (16, 16)
        );
        // Groter dan het scherm: de linkerbovenhoek blijft zichtbaar.
        assert_eq!(
            plaats(Some((10, 10)), (3000, 100), Some(hoofd), hoofd, 16),
            (0, 10)
        );
    }
}
