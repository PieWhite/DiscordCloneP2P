//! Een doorklikbaar, doorzichtig venster met de lijnen erop (beslissing 45): over het beeld
//! in een kijkvenster, en bij de deler over zijn gedeelde monitor.
//!
//! Een *layered* venster dat alleen opnieuw getekend wordt als de tekening verandert, met
//! `UpdateLayeredWindowIndirect` en alleen het stuk dat erbij kwam. De compositor van
//! Windows legt het over wat eronder staat; het videopad merkt er niets van, en niets aan
//! deze laag draait per beeld. Klikken gaan erdoorheen (`WS_EX_TRANSPARENT`), hij pakt nooit
//! de focus (`WS_EX_NOACTIVATE`) en staat niet in alt-tab of de taakbalk
//! (`WS_EX_TOOLWINDOW`).
//!
//! # De laag bij de deler en invariant 4
//!
//! Een venster bovenop een spel dat schermvullend draait haalt dat spel uit zijn eigen
//! presentatiepad (independent flip, of echte exclusieve modus): de compositor moet dan
//! elk beeld samenstellen, en dat kost beelden of een alt-tab. Dat is niet gratis te maken,
//! dus de laag wijkt: zolang er op die monitor een venster vooraan staat dat de hele
//! monitor bedekt zonder titelbalk ([`schermvullend_vooraan`]), is hij verborgen. De lijnen
//! staan dan nog wel bij de kijkers en op de eigen tegel in de app.
//!
//! Hij moet ook buiten de schermopname blijven (`WDA_EXCLUDEFROMCAPTURE`): anders neemt de
//! opname de lijnen mee in het beeld, en dan staan ze bij elke kijker dubbel en in elke clip.
//! Lukt dat niet (Windows ouder dan 10 2004), dan is er geen laag.

use crate::penseel::{aanvulling, Doek, Kader, MIN_HERTEKEN};
use anyhow::{Context, Result};
use crossbeam_channel::{unbounded, Receiver, RecvTimeoutError, Sender};
use fitcom_proto::teken::Tekening;
use std::sync::Arc;
use std::time::{Duration, Instant};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetMonitorInfoW,
    MonitorFromWindow, SelectObject, AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ, HMONITOR, MONITORINFO,
    MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClassNameW,
    GetForegroundWindow, GetWindowLongW, GetWindowRect, IsIconic, PeekMessageW, RegisterClassW,
    SetWindowDisplayAffinity, ShowWindow, TranslateMessage, UpdateLayeredWindowIndirect, GWL_STYLE,
    MSG, PM_REMOVE, SW_HIDE, SW_SHOWNOACTIVATE, ULW_ALPHA, UPDATELAYEREDWINDOWINFO,
    WDA_EXCLUDEFROMCAPTURE, WNDCLASSW, WS_CAPTION, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

const KLASSE: PCWSTR = w!("FitcomTekenlaag");

/// Hoe vaak de laag bij de deler kijkt of hij moet wijken. Alt-tabben naar een spel zet hem
/// dus binnen deze tijd weg; verder kost het een handvol Win32-aanroepen, en alleen zolang
/// er lijnen staan.
const WIJK_INTERVAL: Duration = Duration::from_millis(50);

/// Het beeld van een laag: een DIB-sectie in een eigen geheugen-DC.
struct Dib {
    bitmap: HBITMAP,
    oud: HGDIOBJ,
    bits: *mut u8,
    breedte: u32,
    hoogte: u32,
}

pub(crate) struct Laag {
    hwnd: HWND,
    dc: HDC,
    beeld: Option<Dib>,
    /// Wat er nu in `beeld` staat; daartegen wordt bepaald wat er bij kwam.
    getoond: Option<Arc<Tekening>>,
    kader: RECT,
    zichtbaar: bool,
}

impl Laag {
    /// `eigenaar`: het kijkvenster waar de laag bij hoort. Hij blijft er dan vanzelf boven,
    /// verdwijnt mee bij minimaliseren, en staat niet boven andere programma's. Zonder
    /// eigenaar staat hij bovenop alles en buiten de schermopname: de laag bij de deler.
    pub(crate) fn nieuw(eigenaar: Option<HWND>) -> Result<Self> {
        registreer_klasse()?;
        let mut stijl = WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW;
        if eigenaar.is_none() {
            stijl |= WS_EX_TOPMOST;
        }
        // SAFETY: de klasse is geregistreerd; een leeg pop-upvenster zonder eigen staat.
        let hwnd = unsafe {
            CreateWindowExW(
                stijl,
                KLASSE,
                w!(""),
                WS_POPUP,
                0,
                0,
                1,
                1,
                eigenaar,
                None,
                Some(GetModuleHandleW(None)?.into()),
                None,
            )
        }
        .context("tekenlaag aanmaken")?;

        if eigenaar.is_none() {
            // SAFETY: `hwnd` is net aangemaakt door deze thread.
            if let Err(e) = unsafe { SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) } {
                // SAFETY: zelfde venster; daarna raakt niemand het meer aan.
                unsafe {
                    let _ = DestroyWindow(hwnd);
                }
                return Err(e).context("tekenlaag buiten de schermopname houden");
            }
        }

        // SAFETY: een geheugen-DC zonder voorbeeld-DC is altijd geldig.
        let dc = unsafe { CreateCompatibleDC(None) };
        Ok(Self {
            hwnd,
            dc,
            beeld: None,
            getoond: None,
            kader: RECT::default(),
            zichtbaar: false,
        })
    }

    /// Toont `t` over `kader` (schermpixels). Tekent alleen wat er bijkwam als dat kan, en
    /// niets als er niets veranderde. Een lege tekening verbergt de laag en geeft zijn
    /// geheugen terug.
    pub(crate) fn toon(&mut self, t: &Arc<Tekening>, kader: RECT) {
        if t.is_leeg() {
            self.verberg();
            self.laat_beeld_los();
            return;
        }
        let breedte = (kader.right - kader.left).max(1) as u32;
        let hoogte = (kader.bottom - kader.top).max(1) as u32;
        let zelfde_maat = self
            .beeld
            .as_ref()
            .is_some_and(|d| d.breedte == breedte && d.hoogte == hoogte);
        let zelfde = self.getoond.as_ref().is_some_and(|g| Arc::ptr_eq(g, t));
        if zelfde_maat && zelfde && self.zichtbaar && self.kader == kader {
            return;
        }
        if !zelfde_maat {
            self.laat_beeld_los();
            if let Err(e) = self.maak_beeld(breedte, hoogte) {
                tracing::warn!(error = %format!("{e:#}"), "tekenlaag: geen beeld");
                return;
            }
        }
        let Some(dib) = self.beeld.as_ref() else {
            return;
        };

        // SAFETY: `bits` wijst naar `breedte * hoogte * 4` bytes van de DIB-sectie, die leeft
        // tot `laat_beeld_los`; niemand anders schrijft erin.
        let px = unsafe {
            std::slice::from_raw_parts_mut(dib.bits, breedte as usize * hoogte as usize * 4)
        };
        let mut doek = Doek {
            px,
            breedte,
            hoogte,
        };
        let vuil = match self.getoond.as_deref().and_then(|oud| aanvulling(oud, t)) {
            Some(nieuw) => doek.teken_aanvulling(t, &nieuw),
            None => {
                doek.wis();
                doek.teken_alles(t);
                Some(Kader {
                    x0: 0,
                    y0: 0,
                    x1: breedte as i32,
                    y1: hoogte as i32,
                })
            }
        };
        self.getoond = Some(t.clone());

        // Verplaatst of net zichtbaar: het hele beeld, anders alleen wat er bijkwam.
        let heel = self.kader != kader || !self.zichtbaar;
        let vuil = vuil.map(|k| RECT {
            left: k.x0,
            top: k.y0,
            right: k.x1,
            bottom: k.y1,
        });
        if vuil.is_none() && !heel {
            return;
        }
        let plek = POINT {
            x: kader.left,
            y: kader.top,
        };
        let maat = SIZE {
            cx: breedte as i32,
            cy: hoogte as i32,
        };
        let bron = POINT { x: 0, y: 0 };
        let menging = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let info = UPDATELAYEREDWINDOWINFO {
            cbSize: std::mem::size_of::<UPDATELAYEREDWINDOWINFO>() as u32,
            hdcDst: HDC::default(),
            pptDst: &plek,
            psize: &maat,
            hdcSrc: self.dc,
            pptSrc: &bron,
            crKey: COLORREF(0),
            pblend: &menging,
            dwFlags: ULW_ALPHA,
            prcDirty: match (&vuil, heel) {
                (Some(r), false) => r,
                _ => std::ptr::null(),
            },
        };
        // SAFETY: alle verwijzingen in `info` leven tot na de aanroep; de DIB zit in `dc`.
        let gelukt = unsafe { UpdateLayeredWindowIndirect(self.hwnd, &info) }.as_bool();
        if !gelukt {
            tracing::debug!(
                error = %windows::core::Error::from_thread(),
                "tekenlaag bijwerken mislukt"
            );
            self.getoond = None;
            return;
        }
        self.kader = kader;
        if !self.zichtbaar {
            // SAFETY: eigen venster, eigen thread; SW_SHOWNOACTIVATE pakt geen focus.
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            }
            self.zichtbaar = true;
        }
    }

    pub(crate) fn verberg(&mut self) {
        if self.zichtbaar {
            // SAFETY: eigen venster, eigen thread.
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            }
            self.zichtbaar = false;
        }
    }

    fn maak_beeld(&mut self, breedte: u32, hoogte: u32) -> Result<()> {
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: breedte as i32,
                // Negatief: van boven naar beneden, zoals `Doek` het verwacht.
                biHeight: -(hoogte as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        // SAFETY: `info` beschrijft een 32-bits top-down DIB; `bits` krijgt zijn geheugen.
        let bitmap =
            unsafe { CreateDIBSection(Some(self.dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) }
                .context("DIB-sectie voor de tekenlaag")?;
        if bits.is_null() {
            // SAFETY: net aangemaakt, nergens geselecteerd.
            unsafe {
                let _ = DeleteObject(bitmap.into());
            }
            anyhow::bail!("DIB-sectie zonder geheugen");
        }
        // SAFETY: eigen DC en eigen bitmap.
        let oud = unsafe { SelectObject(self.dc, bitmap.into()) };
        self.beeld = Some(Dib {
            bitmap,
            oud,
            bits: bits.cast(),
            breedte,
            hoogte,
        });
        self.getoond = None;
        Ok(())
    }

    fn laat_beeld_los(&mut self) {
        if let Some(d) = self.beeld.take() {
            // SAFETY: de oude selectie terug, daarna is de bitmap van niemand meer.
            unsafe {
                SelectObject(self.dc, d.oud);
                let _ = DeleteObject(d.bitmap.into());
            }
        }
        self.getoond = None;
    }
}

impl Drop for Laag {
    fn drop(&mut self) {
        self.laat_beeld_los();
        // SAFETY: eigen venster en DC, op de thread die ze maakte.
        unsafe {
            let _ = DeleteDC(self.dc);
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

fn registreer_klasse() -> Result<()> {
    use std::sync::Once;
    static EEN_KEER: Once = Once::new();
    let mut uitkomst = Ok(());
    EEN_KEER.call_once(|| {
        // SAFETY: één keer per proces; de standaardprocedure is genoeg voor een venster dat
        // niets ontvangt.
        unsafe {
            let klasse = WNDCLASSW {
                lpfnWndProc: Some(laag_proc),
                hInstance: match GetModuleHandleW(None) {
                    Ok(h) => h.into(),
                    Err(e) => {
                        uitkomst = Err(e).context("module-handle");
                        return;
                    }
                },
                lpszClassName: KLASSE,
                ..Default::default()
            };
            if RegisterClassW(&klasse) == 0 {
                uitkomst = Err(windows::core::Error::from_thread()).context("vensterklasse");
            }
        }
    });
    uitkomst
}

unsafe extern "system" fn laag_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: doorgeven aan de standaardprocedure met wat Windows ons gaf.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// Het hele gebied van een monitor in schermpixels, of `None` als hij er niet meer is.
pub(crate) fn monitor_kader(monitor: HMONITOR) -> Option<RECT> {
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: `cbSize` is gezet zoals de API vraagt.
    unsafe { GetMonitorInfoW(monitor, &mut info) }
        .as_bool()
        .then_some(info.rcMonitor)
}

/// Het venster dat vooraan staat, als het zijn hele monitor bedekt zonder titelbalk
/// (schermvullend of randloos schermvullend, zoals een spel), met die monitor. Het
/// bureaublad telt niet mee. Eén plek voor de regel: de laag bij de deler wijkt ervoor, en
/// "Speelt X" (`fitcom::speelt`) noemt het een spel als het programma niet op zijn lijst
/// met niet-spellen staat.
pub fn schermvullend_vooraan() -> Option<(HWND, HMONITOR)> {
    // SAFETY: kale Win32-aanroepen met eigen buffers.
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_invalid() || IsIconic(hwnd).as_bool() {
            return None;
        }
        // Een titelbalk betekent een gewoon (zelfs gemaximaliseerd) venster.
        let stijl = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        if stijl & WS_CAPTION.0 == WS_CAPTION.0 {
            return None;
        }
        // Het bureaublad bedekt zijn monitor ook en heeft geen titelbalk.
        let mut klasse = [0u16; 16];
        let n = GetClassNameW(hwnd, &mut klasse).max(0) as usize;
        let klasse = String::from_utf16_lossy(&klasse[..n]);
        if klasse == "Progman" || klasse == "WorkerW" {
            return None;
        }
        let mut venster = RECT::default();
        GetWindowRect(hwnd, &mut venster).ok()?;
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let m = monitor_kader(monitor)?;
        let bedekt = venster.left <= m.left
            && venster.top <= m.top
            && venster.right >= m.right
            && venster.bottom >= m.bottom;
        bedekt.then_some((hwnd, monitor))
    }
}

/// De laag over een gedeelde monitor bij de deler. Leeft op een eigen thread, alleen zolang
/// er lijnen staan: de motor maakt hem bij de eerste lijn en laat hem vallen als alles gewist
/// is.
pub struct Deellaag {
    tx: Option<Sender<Arc<Tekening>>>,
    draad: Option<std::thread::JoinHandle<()>>,
}

impl Deellaag {
    /// `monitor` is het `HMONITOR` uit `Bron::handle`.
    pub fn start(monitor: isize) -> Result<Self> {
        let (tx, rx) = unbounded();
        let draad = std::thread::Builder::new()
            .name("fitcom-tekenlaag".into())
            .spawn(move || deel_lus(HMONITOR(monitor as *mut _), rx))
            .context("tekenlaag-thread starten")?;
        Ok(Self {
            tx: Some(tx),
            draad: Some(draad),
        })
    }

    /// De tekening is veranderd.
    pub fn toon(&self, t: Arc<Tekening>) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(t);
        }
    }
}

impl Drop for Deellaag {
    fn drop(&mut self) {
        // Het kanaal dicht laat de lus stoppen; die ruimt zijn venster zelf op.
        self.tx.take();
        if let Some(d) = self.draad.take() {
            let _ = d.join();
        }
    }
}

fn deel_lus(monitor: HMONITOR, rx: Receiver<Arc<Tekening>>) {
    let mut laag = match Laag::nieuw(None) {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!(
                error = %format!("{e:#}"),
                "geen tekenlaag over het gedeelde scherm; de lijnen staan alleen in de app"
            );
            for _ in rx {}
            return;
        }
    };
    let mut tekening: Option<Arc<Tekening>> = None;
    let mut laatst = Instant::now()
        .checked_sub(MIN_HERTEKEN)
        .unwrap_or_else(Instant::now);
    loop {
        let mut msg = MSG::default();
        // SAFETY: berichten van deze thread.
        unsafe {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        match rx.recv_timeout(WIJK_INTERVAL) {
            Ok(t) => tekening = Some(rx.try_iter().last().unwrap_or(t)),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        let wijk = schermvullend_vooraan().is_some_and(|(_, m)| m == monitor);
        match (&tekening, monitor_kader(monitor)) {
            (Some(t), Some(kader)) if !wijk => {
                if laatst.elapsed() >= MIN_HERTEKEN {
                    laag.toon(t, kader);
                    laatst = Instant::now();
                }
            }
            _ => laag.verberg(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fitcom_proto::control::Draw;
    use fitcom_proto::PeerId;
    use windows::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, GetDC, GetDIBits, ReleaseDC, CAPTUREBLT, SRCCOPY,
    };
    use windows::Win32::UI::WindowsAndMessaging::WDA_NONE;

    /// Wat een schermopname op `kader` ziet, als BGRA.
    fn opname(kader: RECT) -> Vec<u8> {
        let (b, h) = (kader.right - kader.left, kader.bottom - kader.top);
        // SAFETY: eigen DC's en bitmap, opgeruimd voor we terugkeren.
        unsafe {
            let scherm = GetDC(None);
            let dc = CreateCompatibleDC(Some(scherm));
            let bitmap = CreateCompatibleBitmap(scherm, b, h);
            let oud = SelectObject(dc, bitmap.into());
            BitBlt(
                dc,
                0,
                0,
                b,
                h,
                Some(scherm),
                kader.left,
                kader.top,
                SRCCOPY | CAPTUREBLT,
            )
            .unwrap();
            SelectObject(dc, oud);
            let mut info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: b,
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut px = vec![0u8; (b * h * 4) as usize];
            GetDIBits(
                dc,
                bitmap,
                0,
                h as u32,
                Some(px.as_mut_ptr().cast()),
                &mut info,
                DIB_RGB_COLORS,
            );
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(dc);
            ReleaseDC(None, scherm);
            px
        }
    }

    /// Pixels waar rood duidelijk de boventoon voert. Niet "puur rood": een testproces zonder
    /// DPI-bewustzijn krijgt op een geschaalde monitor een verkleinde opname, en dan is een
    /// dunne lijn half gemengd met wat eronder staat.
    fn rode_pixels(px: &[u8]) -> usize {
        px.chunks_exact(4)
            .filter(|p| {
                i32::from(p[2]) > i32::from(p[1]) + 60 && i32::from(p[2]) > i32::from(p[0]) + 60
            })
            .count()
    }

    /// Op een echt bureaublad: de laag bij de deler tekent, en een schermopname ziet hem
    /// niet (`WDA_EXCLUDEFROMCAPTURE`) — anders stonden de lijnen dubbel bij de kijkers en
    /// in elke clip. Daarna dezelfde laag zonder uitsluiting, als bewijs dat er wél iets te
    /// zien was. Een strook van 240 bij 90 pixels linksboven, een seconde of twee,
    /// doorklikbaar en zonder focus; overgeslagen als er iets schermvullend vooraan staat.
    #[test]
    #[ignore = "toont kort een venster op het bureaublad"]
    fn de_laag_tekent_en_blijft_buiten_de_opname() {
        if schermvullend_vooraan().is_some() {
            eprintln!("iets schermvullend vooraan; overgeslagen");
            return;
        }
        let kader = RECT {
            left: 0,
            top: 0,
            right: 240,
            bottom: 90,
        };
        let mut t = Tekening::nieuw(true);
        t.teken(
            PeerId::new_random(),
            &Draw {
                stream_id: 1,
                author: None,
                line: 1,
                color: 0,
                width: 2,
                start: 0,
                points: vec![[2000, 32767], [63000, 32767]],
            },
        );
        let t = Arc::new(t);

        // Ook met de opname die de deler echt gebruikt: Windows.Graphics.Capture op de
        // monitor, de linkerbovenhoek in fysieke pixels (ruim genomen voor DPI-schaling).
        let d3d = crate::d3d::D3dContext::new().expect("D3D11");
        // SAFETY: een punt opzoeken; geen eigendom.
        let monitor = unsafe {
            windows::Win32::Graphics::Gdi::MonitorFromPoint(
                POINT { x: 0, y: 0 },
                MONITOR_DEFAULTTONEAREST,
            )
        };
        let wgc = || {
            let bron = crate::capture::Bron {
                naam: "test".into(),
                soort: crate::capture::BronSoort::Monitor,
                handle: monitor.0 as isize,
            };
            let mut c = crate::capture::Capture::start(&d3d, &bron).expect("opname");
            let beeld = c.volgende_frame(Duration::from_secs(2)).expect("een beeld");
            let (b, _, px) = d3d.lees_bgra(&beeld.textuur).expect("uitlezen");
            let hoek: Vec<u8> = px
                .chunks_exact(b as usize * 4)
                .take(300)
                .flat_map(|rij| rij[..600 * 4].to_vec())
                .collect();
            rode_pixels(&hoek)
        };

        let mut laag = Laag::nieuw(None).expect("laag buiten de opname");
        laag.toon(&t, kader);
        std::thread::sleep(Duration::from_millis(250));
        let buiten = rode_pixels(&opname(kader));
        let buiten_wgc = wgc();

        // SAFETY: eigen venster, eigen thread.
        unsafe { SetWindowDisplayAffinity(laag.hwnd, WDA_NONE) }.unwrap();
        std::thread::sleep(Duration::from_millis(250));
        let binnen = rode_pixels(&opname(kader));
        let binnen_wgc = wgc();

        // Een tweede lijn erbij: dat is de weg die elke 40 ms loopt terwijl iemand tekent,
        // alleen het nieuwe stuk met een vuil gebied (`prcDirty`), niet het hele beeld.
        let mut meer = (*t).clone();
        assert!(meer.teken(
            PeerId::new_random(),
            &Draw {
                stream_id: 1,
                author: None,
                line: 2,
                color: 0,
                width: 2,
                start: 0,
                points: vec![[2000, 12000], [63000, 12000]],
            },
        ));
        assert!(
            crate::penseel::aanvulling(&t, &meer).is_some(),
            "alleen bijgekomen"
        );
        laag.toon(&Arc::new(meer), kader);
        std::thread::sleep(Duration::from_millis(250));
        let aangevuld = rode_pixels(&opname(kader));
        drop(laag);
        eprintln!(
            "rode pixels, met uitsluiting / zonder: BitBlt {buiten} / {binnen}, WGC {buiten_wgc} / {binnen_wgc}; na een tweede lijn {aangevuld}"
        );

        assert!(
            binnen > 100 && binnen_wgc > 100,
            "de lijn hoort te zien te zijn zonder uitsluiting: {binnen}, {binnen_wgc}"
        );
        assert!(
            aangevuld > binnen + 100,
            "de tweede lijn hoort erbij te komen zonder alles opnieuw: {aangevuld} na {binnen}"
        );
        assert!(
            buiten < binnen / 10 && buiten_wgc < binnen_wgc / 10,
            "met uitsluiting ziet de opname de lijn toch: {buiten}, {buiten_wgc}"
        );
    }
}
