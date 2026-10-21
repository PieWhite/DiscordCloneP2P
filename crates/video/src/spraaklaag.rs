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

use crate::penseel::{Kader, MIN_HERTEKEN};
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
    DT_SINGLELINE, DT_VCENTER, FW_NORMAL, HFONT, HMONITOR, MONITOR_DEFAULTTONULL,
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
        // SAFETY: eigen objecten, nergens meer geselecteerd (`teken` zet de oude terug).
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
    let s = |v: i32| schaal(v, dpi);

    let namen: Vec<Vec<u16>> = sprekers
        .iter()
        .map(|p| p.naam.encode_utf16().collect())
        .collect();
    let letters: Vec<Vec<u16>> = sprekers
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
    // SAFETY: het lettertype tijdelijk in de DC van de laag, en daarna de oude terug.
    let breedtes: Vec<i32> = unsafe {
        let oud = SelectObject(laag.dc(), f.naam.into());
        let b = namen
            .iter()
            .map(|n| {
                let mut maat = SIZE::default();
                let _ = GetTextExtentPoint32W(laag.dc(), n, &mut maat);
                maat.cx.min(s(NAAM_MAX))
            })
            .collect();
        SelectObject(laag.dc(), oud);
        b
    };
    let rij_breedte = |b: i32| s(NAAM_X) + b + s(RECHTS);
    let maat = (
        breedtes.iter().map(|&b| rij_breedte(b)).max().unwrap_or(1),
        sprekers.len() as i32 * s(RIJ + TUSSEN) - s(TUSSEN),
    );
    let (x, y) = plaats(plek, maat, scherm, hoofd_kader, s(MARGE));
    let kader = RECT {
        left: x,
        top: y,
        right: x + maat.0,
        bottom: y + maat.1,
    };

    laag.toon_beeld(kader, |doek, dc| {
        let rij = |i: usize| i as i32 * s(RIJ + TUSSEN);
        let stip = |i: usize| RECT {
            left: s(STIP_LINKS),
            top: rij(i) + (s(RIJ) - s(STIP)) / 2,
            right: s(STIP_LINKS) + s(STIP),
            bottom: rij(i) + (s(RIJ) + s(STIP)) / 2,
        };
        let naam = |i: usize| RECT {
            left: s(NAAM_X),
            top: rij(i),
            right: s(NAAM_X) + breedtes[i],
            bottom: rij(i) + s(RIJ),
        };

        // Eerst alle tekst wit op zwart: dat is het masker.
        doek.wis();
        // SAFETY: eigen DC met het beeld erin; lettertypes eigen, daarna de oude terug.
        unsafe {
            SetBkMode(dc, TRANSPARENT);
            SetTextColor(dc, COLORREF(0x00FF_FFFF));
            let oud = SelectObject(dc, f.naam.into());
            for (i, n) in namen.iter().enumerate() {
                let mut r = naam(i);
                let mut n = n.clone();
                DrawTextW(
                    dc,
                    &mut n,
                    &mut r,
                    DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
                );
            }
            SelectObject(dc, f.monogram.into());
            for (i, l) in letters.iter().enumerate() {
                let mut r = stip(i);
                let mut l = l.clone();
                DrawTextW(
                    dc,
                    &mut l,
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
        let half = s(RIJ) as f32 / 2.0;
        for (i, p) in sprekers.iter().enumerate() {
            let midden = rij(i) as f32 + half;
            let rechts = rij_breedte(breedtes[i]) as f32 - half;
            doek.capsule(
                (half, midden),
                (rechts, midden),
                half,
                stijl.grond,
                stijl.grond_alfa,
            );
            let st = stip(i);
            let c = (
                (st.left + st.right) as f32 / 2.0,
                (st.top + st.bottom) as f32 / 2.0,
            );
            let r = s(STIP) as f32 / 2.0;
            if p.spreekt {
                doek.capsule(c, c, r, stijl.ring, 1.0);
                doek.capsule(c, c, r - s(RING) as f32, stijl.stip, 1.0);
            } else {
                doek.capsule(c, c, r, stijl.stip, 1.0);
            }
            let kleur = if p.spreekt { stijl.spreekt } else { stijl.stil };
            for r in [naam(i), st] {
                doek.bedek(&masker, kader_van(r), kleur);
            }
        }
    });
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
