//! Het venster waarin een inkomende stream te zien is.
//!
//! Eigen Win32-venster met eigen DXGI-swapchain, bediend door de thread die hem ook
//! aanmaakt. Zie `docs/ARCHITECTURE.md` → "Waarom video in een apart venster": eframe
//! rendert via wgpu op DX12, en een gedecodeerde D3D11-textuur daarin krijgen vereist
//! interop op `wgpu-hal`-niveau. Zo blijft het videopad volledig los van de UI en kan
//! de UI stilvallen zonder dat het beeld hapert.
//!
//! # Waarom er wél een vensterrand omheen zit
//!
//! De architectuur zegt "borderless". Dat is in de praktijk de verkeerde keuze: een
//! venster zonder rand kun je niet verplaatsen, niet vergroten en niet sluiten zonder
//! dat je dat allemaal zelf nabouwt met hit-testing. Wat er bedoeld werd — geen chroom
//! *over* het beeld, geen egui eromheen — geldt onverkort. Voor beeldvullend kijken
//! zit er een schakelaar op F11 en dubbelklik, en dán is hij randloos.
//!
//! # Waarom de swapchain niet meegroeit met het venster
//!
//! De swapchain houdt de afmeting van de stream en DXGI schaalt bij het presenteren.
//! Dat scheelt het opnieuw opbouwen van buffers bij elke muisbeweging tijdens het
//! slepen, en het schalen gebeurt toch al op de GPU. Om vervorming te voorkomen houdt
//! het venster zelf de beeldverhouding vast terwijl je sleept.
//!
//! # Tekenen op het beeld (beslissing 45)
//!
//! Met de pen op (D, of het menu onder de rechterknop) wordt slepen met de linkerknop een
//! lijn. Die gaat in stukjes naar de deler en komt terug zoals hij hem doorgeeft; de lijnen
//! staan op een eigen doorklikbaar venster boven het beeld ([`crate::tekenlaag`]), dat
//! alleen opnieuw tekent als de tekening verandert. De swapchain en het videopad weten er
//! niets van. Het menu is modaal: zolang het open staat wacht deze thread, net als bij
//! het slepen van het venster, en dan komt het beeld daarna via een keyframe terug.

use crate::d3d::D3dContext;
use crate::tekenlaag::Laag;
use anyhow::{Context, Result};
use fitcom_proto::control::{Draw, DrawClear, DrawClearScope};
use fitcom_proto::teken::{Tekening, DIKTES, KLEUREN, MAX_PUNTEN_PER_BERICHT, MAX_PUNTEN_PER_LIJN};
use fitcom_proto::ControlMsg;
use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use windows::core::{w, Interface, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    IDXGIDevice, IDXGIFactory2, IDXGIFactory5, IDXGISwapChain1, DXGI_FEATURE_PRESENT_ALLOW_TEARING,
    DXGI_MWA_NO_ALT_ENTER, DXGI_PRESENT, DXGI_PRESENT_ALLOW_TEARING, DXGI_SCALING_STRETCH,
    DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG, DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING,
    DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, VK_CONTROL, VK_D, VK_DELETE, VK_ESCAPE, VK_F11, VK_Z,
};
use windows::Win32::UI::WindowsAndMessaging::*;

const KLASSE: PCWSTR = w!("FitcomVideoVenster");

/// Wat de vensterprocedure bijhoudt. Leeft in een `Box` achter `GWLP_USERDATA` en
/// wordt bij `WM_NCDESTROY` weer opgeruimd.
struct Staat {
    gesloten: bool,
    volledig_scherm: bool,
    /// Waar het venster stond voordat het beeldvullend werd.
    vorige_plaatsing: WINDOWPLACEMENT,
    /// Beeldverhouding van de stream, om vervorming bij het slepen te voorkomen.
    verhouding: f32,
    teken: Teken,
}

/// Zo vaak gaat een lijn in aanbouw naar de deler: vaak genoeg dat hij bij de anderen
/// meegroeit, weinig genoeg voor een handvol kleine berichten per seconde.
const TEKEN_INTERVAL: Duration = Duration::from_millis(40);
/// Zoveel pixels moet de muis bewegen voor een nieuw punt. Een muis op 1000 Hz levert
/// anders een punt per pixel, en dat ziet niemand.
const MIN_STAP_PX: i32 = 3;

/// De gekozen kleur en dikte, voor het hele proces: een nieuw kijkvenster houdt de pen die
/// je het laatst koos.
static PEN_KLEUR: AtomicU8 = AtomicU8::new(0);
static PEN_DIKTE: AtomicU8 = AtomicU8::new(1);
/// Lijnnummers lopen op over alle vensters heen: nooit twee keer hetzelfde nummer van ons
/// op één scherm, ook niet na opnieuw openen. Nul betekent "nog niet begonnen"; zie
/// [`volgende_lijn`].
static VOLGENDE_LIJN: AtomicU32 = AtomicU32::new(0);

/// Het volgende lijnnummer. De eerste keer in dit proces begint de teller bij de klok: een
/// kijker die herstart terwijl de deler zijn oude verbinding nog vasthoudt (tot 15 s) heeft
/// daar nog lijnen staan, en met nummers vanaf 1 zou zijn nieuwe lijn op een oude
/// doortekenen.
fn volgende_lijn() -> u32 {
    let zaad = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(1, |d| (d.as_millis() as u32).max(1));
    let _ = VOLGENDE_LIJN.compare_exchange(0, zaad, Ordering::Relaxed, Ordering::Relaxed);
    VOLGENDE_LIJN.fetch_add(1, Ordering::Relaxed)
}

/// Tekenen in dit venster.
struct Teken {
    /// Zoals de deler hem doorgeeft; `aan` zegt of er getekend mag worden.
    tekening: Arc<Tekening>,
    /// De pen is opgepakt. Begint neer: een klik in het beeld wordt nooit per ongeluk een
    /// streep op het scherm van een ander.
    pen: bool,
    lopend: Option<LopendeLijn>,
    /// Klaar om naar de deler te gaan, in volgorde.
    uit: Vec<Werk>,
    laatst_verstuurd: Instant,
    laag: Option<Laag>,
    /// De titel zonder de tekenhint.
    titel: String,
}

struct LopendeLijn {
    id: u32,
    kleur: u8,
    dikte: u8,
    punten: Vec<[u16; 2]>,
    verstuurd: usize,
    /// Waar het laatste punt in clientpixels lag, voor [`MIN_STAP_PX`].
    laatst_px: (i32, i32),
}

impl LopendeLijn {
    fn nieuw(punt: [u16; 2], px: (i32, i32)) -> Self {
        Self {
            id: volgende_lijn(),
            kleur: PEN_KLEUR.load(Ordering::Relaxed),
            dikte: PEN_DIKTE.load(Ordering::Relaxed),
            punten: vec![punt],
            verstuurd: 0,
            laatst_px: px,
        }
    }

    /// Wat er sinds de vorige keer bij kwam. Een lijn van één punt gaat nooit weg: dat is
    /// een klik, bijvoorbeeld de eerste helft van een dubbelklik.
    fn stuk(&mut self) -> Option<Werk> {
        if self.punten.len() < 2 || self.punten.len() == self.verstuurd {
            return None;
        }
        let stuk = Werk::Punten {
            lijn: self.id,
            kleur: self.kleur,
            dikte: self.dikte,
            start: self.verstuurd as u32,
            punten: self.punten[self.verstuurd..].to_vec(),
        };
        self.verstuurd = self.punten.len();
        Some(stuk)
    }
}

enum Werk {
    Punten {
        lijn: u32,
        kleur: u8,
        dikte: u8,
        start: u32,
        punten: Vec<[u16; 2]>,
    },
    Wis(DrawClearScope),
}

impl Teken {
    fn nieuw(titel: &str) -> Self {
        Self {
            tekening: Arc::new(Tekening::nieuw(false)),
            pen: false,
            lopend: None,
            uit: Vec::new(),
            laatst_verstuurd: Instant::now(),
            laag: None,
            titel: titel.to_string(),
        }
    }

    fn mag(&self) -> bool {
        self.tekening.aan && self.pen
    }

    fn zet(&mut self, hwnd: HWND, t: Arc<Tekening>) {
        let wissel = self.tekening.aan != t.aan;
        self.tekening = t;
        if wissel {
            if !self.tekening.aan {
                self.pen = false;
                self.lopend = None;
            }
            let titel = if self.tekening.aan {
                format!("{} \u{2014} right-click to draw", self.titel)
            } else {
                self.titel.clone()
            };
            let breed: Vec<u16> = titel.encode_utf16().chain(std::iter::once(0)).collect();
            // SAFETY: eigen venster, eigen thread, nul-afgesloten buffer.
            unsafe {
                let _ = SetWindowTextW(hwnd, PCWSTR(breed.as_ptr()));
            }
        }
        self.herteken(hwnd);
    }

    /// De laag over het clientgebied leggen, met de tekening van nu. Doet niets als er niets
    /// veranderde; een lege tekening verbergt hem.
    fn herteken(&mut self, hwnd: HWND) {
        if self.tekening.is_leeg() {
            if let Some(l) = &mut self.laag {
                l.toon(&self.tekening, RECT::default());
            }
            return;
        }
        let Some(kader) = client_kader(hwnd) else {
            return;
        };
        if self.laag.is_none() {
            match Laag::nieuw(Some(hwnd)) {
                Ok(l) => self.laag = Some(l),
                Err(e) => {
                    tracing::warn!(error = %format!("{e:#}"), "geen tekenlaag in het kijkvenster");
                    return;
                }
            }
        }
        if let Some(l) = &mut self.laag {
            l.toon(&self.tekening, kader);
        }
    }

    fn begin(&mut self, hwnd: HWND, lparam: LPARAM) {
        let px = muis_px(lparam);
        self.lopend = Some(LopendeLijn::nieuw(normaliseer(hwnd, px), px));
    }

    fn beweeg(&mut self, hwnd: HWND, lparam: LPARAM) {
        let px = muis_px(lparam);
        let Some(l) = &mut self.lopend else {
            return;
        };
        let (dx, dy) = (px.0 - l.laatst_px.0, px.1 - l.laatst_px.1);
        if dx * dx + dy * dy < MIN_STAP_PX * MIN_STAP_PX {
            return;
        }
        l.laatst_px = px;
        let punt = normaliseer(hwnd, px);
        l.punten.push(punt);
        // Vol: deze lijn afsluiten en naadloos verder met een nieuwe vanaf hetzelfde punt.
        if l.punten.len() >= MAX_PUNTEN_PER_LIJN {
            self.uit.extend(l.stuk());
            self.lopend = Some(LopendeLijn::nieuw(punt, px));
        }
    }

    fn eind(&mut self) {
        if let Some(mut l) = self.lopend.take() {
            self.uit.extend(l.stuk());
        }
    }

    /// Wat er naar de deler moet: lijnstukken hooguit elke [`TEKEN_INTERVAL`] (of meteen
    /// bij loslaten), wisverzoeken meteen.
    fn werk(&mut self, stream_id: u32) -> Vec<ControlMsg> {
        if self.laatst_verstuurd.elapsed() >= TEKEN_INTERVAL {
            if let Some(stuk) = self.lopend.as_mut().and_then(LopendeLijn::stuk) {
                self.uit.push(stuk);
            }
        }
        if self.uit.is_empty() {
            return Vec::new();
        }
        self.laatst_verstuurd = Instant::now();
        let mut berichten = Vec::new();
        for werk in self.uit.drain(..) {
            match werk {
                Werk::Punten {
                    lijn,
                    kleur,
                    dikte,
                    start,
                    punten,
                } => {
                    for (i, stuk) in punten.chunks(MAX_PUNTEN_PER_BERICHT).enumerate() {
                        berichten.push(ControlMsg::Draw(Draw {
                            stream_id,
                            author: None,
                            line: lijn,
                            color: kleur,
                            width: dikte,
                            start: start + (i * MAX_PUNTEN_PER_BERICHT) as u32,
                            points: stuk.to_vec(),
                        }));
                    }
                }
                Werk::Wis(scope) => {
                    berichten.push(ControlMsg::DrawClear(DrawClear { stream_id, scope }))
                }
            }
        }
        berichten
    }

    fn wis(&mut self, scope: DrawClearScope) {
        if self.tekening.aan {
            self.eind();
            self.uit.push(Werk::Wis(scope));
        }
    }

    fn zet_pen(&mut self, pen: bool) {
        self.pen = pen && self.tekening.aan;
        if !self.pen {
            self.eind();
        }
    }
}

/// Muispositie uit een muisbericht, in clientpixels. Met de muis gevangen kan hij buiten
/// het venster liggen, dus negatief.
fn muis_px(lparam: LPARAM) -> (i32, i32) {
    (
        (lparam.0 & 0xFFFF) as u16 as i16 as i32,
        ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32,
    )
}

/// Clientpixels naar het raster op de draad. Het beeld vult het hele clientgebied (de
/// swapchain rekt, `DXGI_SCALING_STRETCH`), dus dat is een directe omrekening.
fn normaliseer(hwnd: HWND, (x, y): (i32, i32)) -> [u16; 2] {
    let mut r = RECT::default();
    // SAFETY: eigen venster.
    unsafe {
        let _ = GetClientRect(hwnd, &mut r);
    }
    let schaal = |v: i32, maat: i32| {
        ((v as f32 + 0.5) / maat.max(1) as f32 * 65535.0)
            .round()
            .clamp(0.0, 65535.0) as u16
    };
    [schaal(x, r.right), schaal(y, r.bottom)]
}

/// Het clientgebied in schermpixels, of `None` als het venster geminimaliseerd is.
fn client_kader(hwnd: HWND) -> Option<RECT> {
    let mut r = RECT::default();
    let mut hoek = POINT { x: 0, y: 0 };
    // SAFETY: eigen venster.
    unsafe {
        if IsIconic(hwnd).as_bool() || GetClientRect(hwnd, &mut r).is_err() {
            return None;
        }
        if !ClientToScreen(hwnd, &mut hoek).as_bool() {
            return None;
        }
    }
    (r.right > 0 && r.bottom > 0).then_some(RECT {
        left: hoek.x,
        top: hoek.y,
        right: hoek.x + r.right,
        bottom: hoek.y + r.bottom,
    })
}

/// Menu-id's onder de rechterknop.
const MENU_PEN: usize = 1;
const MENU_KLEUR: usize = 10;
const MENU_DIKTE: usize = 30;
const MENU_ONGEDAAN: usize = 40;
const MENU_WIS: usize = 41;

/// Het menu onder de rechterknop: pen op of neer, kleur, dikte, ongedaan en wissen.
fn toon_menu(hwnd: HWND, lparam: LPARAM, teken: &mut Teken) {
    let breed = |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };
    // SAFETY: een menu van deze thread, opgeruimd voor we terugkeren.
    unsafe {
        let Ok(menu) = CreatePopupMenu() else {
            return;
        };
        let voeg = |vlaggen: MENU_ITEM_FLAGS, id: usize, tekst: &str| {
            let t = breed(tekst);
            let _ = AppendMenuW(menu, vlaggen, id, PCWSTR(t.as_ptr()));
        };
        let vink = |aan: bool| if aan { MF_CHECKED } else { MF_STRING };
        if teken.tekening.aan {
            voeg(vink(teken.pen), MENU_PEN, "Draw on the stream\tD");
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            let kleur = PEN_KLEUR.load(Ordering::Relaxed) as usize;
            for (i, k) in KLEUREN.iter().enumerate() {
                voeg(vink(i == kleur), MENU_KLEUR + i, k.naam);
            }
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            let dikte = PEN_DIKTE.load(Ordering::Relaxed) as usize;
            for (i, (naam, _)) in DIKTES.iter().enumerate() {
                voeg(vink(i == dikte), MENU_DIKTE + i, naam);
            }
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            voeg(MF_STRING, MENU_ONGEDAAN, "Undo my last line\tCtrl+Z");
            voeg(MF_STRING, MENU_WIS, "Clear my lines\tDel");
        } else {
            voeg(MF_GRAYED, 0, "Drawing is off for this stream");
        }

        let mut punt = POINT {
            x: muis_px(lparam).0,
            y: muis_px(lparam).1,
        };
        let _ = ClientToScreen(hwnd, &mut punt);
        let keuze = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            punt.x,
            punt.y,
            None,
            hwnd,
            None,
        )
        .0 as usize;
        let _ = DestroyMenu(menu);

        match keuze {
            MENU_PEN => teken.zet_pen(!teken.pen),
            k if (MENU_KLEUR..MENU_KLEUR + KLEUREN.len()).contains(&k) => {
                PEN_KLEUR.store((k - MENU_KLEUR) as u8, Ordering::Relaxed);
                teken.zet_pen(true);
            }
            k if (MENU_DIKTE..MENU_DIKTE + DIKTES.len()).contains(&k) => {
                PEN_DIKTE.store((k - MENU_DIKTE) as u8, Ordering::Relaxed);
                teken.zet_pen(true);
            }
            MENU_ONGEDAAN => teken.wis(DrawClearScope::LAST),
            MENU_WIS => teken.wis(DrawClearScope::MINE),
            _ => {}
        }
    }
}

pub struct Venster {
    hwnd: HWND,
    swapchain: IDXGISwapChain1,
    d3d: D3dContext,
    tearing: bool,
    afmeting: (u32, u32),
}

impl Venster {
    /// Maakt het venster aan **op de aanroepende thread**. Een Win32-venster hoort bij
    /// de thread die hem maakt: alleen die thread krijgt zijn berichten, dus alleen
    /// die thread mag [`Venster::pomp`] aanroepen.
    pub fn open(d3d: &D3dContext, titel: &str, breedte: u32, hoogte: u32) -> Result<Self> {
        registreer_klasse()?;

        let staat = Box::new(Staat {
            gesloten: false,
            volledig_scherm: false,
            vorige_plaatsing: WINDOWPLACEMENT {
                length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
                ..Default::default()
            },
            verhouding: breedte as f32 / hoogte.max(1) as f32,
            teken: Teken::nieuw(titel),
        });

        let naam: Vec<u16> = titel.encode_utf16().chain(std::iter::once(0)).collect();
        // De opgegeven maat is die van het beeld; het venster moet zijn rand daar nog
        // omheen krijgen, anders is het beeld precies die rand te klein.
        let mut kader = RECT {
            left: 0,
            top: 0,
            right: breedte as i32,
            bottom: hoogte as i32,
        };

        // SAFETY: de klasse is geregistreerd en `staat` wordt in WM_NCCREATE overgenomen.
        let hwnd = unsafe {
            let _ = AdjustWindowRect(&mut kader, WS_OVERLAPPEDWINDOW, false);
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                KLASSE,
                PCWSTR(naam.as_ptr()),
                WS_OVERLAPPEDWINDOW,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                kader.right - kader.left,
                kader.bottom - kader.top,
                None,
                None,
                Some(GetModuleHandleW(None)?.into()),
                Some(Box::into_raw(staat) as *const _),
            )
            .context("videovenster aanmaken")?
        };

        // SAFETY: `hwnd` is net aangemaakt.
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
        }

        let (swapchain, tearing) = maak_swapchain(d3d, hwnd, breedte, hoogte)?;

        tracing::info!(titel, breedte, hoogte, tearing, "videovenster geopend");

        Ok(Self {
            hwnd,
            swapchain,
            d3d: d3d.clone(),
            tearing,
            afmeting: (breedte, hoogte),
        })
    }

    pub fn afmeting(&self) -> (u32, u32) {
        self.afmeting
    }

    /// Handelt de wachtende vensterberichten af. `false` betekent dat de gebruiker het
    /// venster gesloten heeft; dan hoort de aanroeper te stoppen met kijken.
    ///
    /// Dit blokkeert niet. Het venster hoeft niet vloeiend te blijven als er geen beeld
    /// binnenkomt — maar het moet wel te sluiten zijn, en dat is wat dit garandeert.
    pub fn pomp(&mut self) -> bool {
        let mut msg = MSG::default();
        // SAFETY: we pompen de berichten van onze eigen thread.
        unsafe {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        !self.is_gesloten()
    }

    fn is_gesloten(&self) -> bool {
        // SAFETY: de staat blijft leven tot WM_NCDESTROY, en dan is `gesloten` allang
        // gezet en heeft de aanroeper ons opgeruimd.
        unsafe {
            let ptr = GetWindowLongPtrW(self.hwnd, GWLP_USERDATA) as *const Staat;
            ptr.is_null() || (*ptr).gesloten
        }
    }

    fn teken(&mut self) -> Option<&mut Teken> {
        // SAFETY: zoals bij `is_gesloten`; we draaien op de thread van het venster en de
        // vensterprocedure loopt nu niet.
        unsafe {
            let ptr = GetWindowLongPtrW(self.hwnd, GWLP_USERDATA) as *mut Staat;
            (!ptr.is_null()).then(|| &mut (*ptr).teken)
        }
    }

    /// De tekening van de deler, zoals de motor hem nu kent. Alleen bij een wijziging; de
    /// laag tekent dan bij wat er veranderde.
    pub fn zet_tekening(&mut self, t: Arc<Tekening>) {
        let hwnd = self.hwnd;
        if let Some(teken) = self.teken() {
            teken.zet(hwnd, t);
        }
    }

    /// Wat de gebruiker tekende of wiste sinds de vorige keer, als berichten voor de deler.
    pub fn tekenwerk(&mut self, stream_id: u32) -> Vec<ControlMsg> {
        self.teken().map(|t| t.werk(stream_id)).unwrap_or_default()
    }

    /// Zet één beeld op het scherm.
    ///
    /// Wijkt de afmeting af van waarmee het venster geopend is — de deler kondigde iets
    /// anders aan dan zijn encoder uiteindelijk levert, of hij wisselde van resolutie —
    /// dan past de swapchain zich aan. Het venster zelf blijft staan waar het staat.
    pub fn toon(&mut self, beeld: &ID3D11Texture2D) -> Result<()> {
        let maat = crate::d3d::afmetingen(beeld);
        if maat != self.afmeting {
            self.pas_maat_aan(maat)?;
        }

        // SAFETY: de backbuffer hoort bij deze swapchain en heeft hetzelfde formaat en
        // dezelfde maat als het beeld.
        unsafe {
            let achter: ID3D11Texture2D = self.swapchain.GetBuffer(0).context("backbuffer")?;
            self.d3d.context.CopyResource(&achter, beeld);

            // Zonder vsync: elke gewachte vblank is zestien milliseconde die je bij het
            // wijzen naar iets op een gedeeld scherm merkt. Scheuren is hier het
            // kleinere kwaad, en het treedt alleen op als de zender uit de pas loopt
            // met de monitor van de kijker.
            let vlaggen = if self.tearing {
                DXGI_PRESENT_ALLOW_TEARING
            } else {
                DXGI_PRESENT(0)
            };
            self.swapchain
                .Present(0, vlaggen)
                .ok()
                .context("beeld tonen")?;
        }
        Ok(())
    }

    fn pas_maat_aan(&mut self, (breedte, hoogte): (u32, u32)) -> Result<()> {
        tracing::info!(van = ?self.afmeting, naar = ?(breedte, hoogte), "stream van maat veranderd");

        // SAFETY: er staat geen enkele verwijzing naar een backbuffer meer open —
        // `toon` haalt hem op en laat hem in dezelfde aanroep weer los.
        unsafe {
            self.swapchain
                .ResizeBuffers(
                    2,
                    breedte,
                    hoogte,
                    DXGI_FORMAT_B8G8R8A8_UNORM,
                    if self.tearing {
                        DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING
                    } else {
                        DXGI_SWAP_CHAIN_FLAG(0)
                    },
                )
                .context("swapchain aanpassen")?;

            // De beeldverhouding die het venster bewaakt klopt nu niet meer.
            let ptr = GetWindowLongPtrW(self.hwnd, GWLP_USERDATA) as *mut Staat;
            if !ptr.is_null() {
                (*ptr).verhouding = breedte as f32 / hoogte.max(1) as f32;
            }
        }
        self.afmeting = (breedte, hoogte);
        Ok(())
    }
}

impl Drop for Venster {
    fn drop(&mut self) {
        // SAFETY: `hwnd` is van deze thread; na DestroyWindow raakt niemand hem meer aan.
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

fn registreer_klasse() -> Result<()> {
    use std::sync::Once;
    static EEN_KEER: Once = Once::new();
    let mut uitkomst = Ok(());
    EEN_KEER.call_once(|| {
        // SAFETY: de klasse wordt precies één keer per proces geregistreerd.
        unsafe {
            let klasse = WNDCLASSW {
                style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
                lpfnWndProc: Some(venster_proc),
                hInstance: match GetModuleHandleW(None) {
                    Ok(h) => h.into(),
                    Err(e) => {
                        uitkomst = Err(e).context("module-handle");
                        return;
                    }
                },
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
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

fn maak_swapchain(
    d3d: &D3dContext,
    hwnd: HWND,
    breedte: u32,
    hoogte: u32,
) -> Result<(IDXGISwapChain1, bool)> {
    let dxgi: IDXGIDevice = d3d.device.cast().context("DXGI-apparaat")?;
    // SAFETY: het apparaat is geldig; adapter en factory horen er per definitie bij.
    let factory: IDXGIFactory2 = unsafe {
        let adapter = dxgi.GetAdapter().context("adapter")?;
        adapter.GetParent().context("DXGI-factory")?
    };

    // Scheuren toestaan kan alleen als de hele keten het ondersteunt. Op een machine
    // waar dat niet zo is werkt alles nog, alleen met iets meer vertraging.
    let tearing = factory
        .cast::<IDXGIFactory5>()
        .ok()
        .and_then(|f5| {
            let mut aan: u32 = 0;
            // SAFETY: uitvoerparameter is een geldige lokale variabele.
            unsafe {
                f5.CheckFeatureSupport(
                    DXGI_FEATURE_PRESENT_ALLOW_TEARING,
                    &mut aan as *mut _ as *mut _,
                    std::mem::size_of::<u32>() as u32,
                )
                .ok()?;
            }
            Some(aan != 0)
        })
        .unwrap_or(false);

    let desc = DXGI_SWAP_CHAIN_DESC1 {
        Width: breedte,
        Height: hoogte,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        Stereo: false.into(),
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
        // Twee buffers en het flip-model: dat is het pad zonder tussenkopie in de
        // desktopcompositor.
        BufferCount: 2,
        // Rekken naar de venstermaat. Het venster bewaakt zelf de beeldverhouding.
        Scaling: DXGI_SCALING_STRETCH,
        SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
        AlphaMode: DXGI_ALPHA_MODE_IGNORE,
        Flags: if tearing {
            DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING.0 as u32
        } else {
            0
        },
    };

    // SAFETY: `desc` is volledig ingevuld en `hwnd` is een geldig venster van deze thread.
    let swapchain = unsafe {
        let sc = factory
            .CreateSwapChainForHwnd(&d3d.device, hwnd, &desc, None, None)
            .context("swapchain aanmaken")?;
        // DXGI's eigen alt-enter zet een echte fullscreen-modus aan, met een
        // modeswitch en al. Wij doen beeldvullend zelf, zonder modeswitch.
        let _ = factory.MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER);
        sc
    };

    Ok((swapchain, tearing))
}

/// Beeldvullend zonder modeswitch: rand eraf en over het scherm heen waar het venster
/// op staat. Een echte fullscreen-modus zou de resolutie omgooien, en dat is precies
/// wat je niet wilt terwijl er op datzelfde scherm een game draait.
fn schakel_volledig_scherm(hwnd: HWND, staat: &mut Staat) {
    // SAFETY: `hwnd` is geldig en we draaien op de thread van het venster.
    unsafe {
        if staat.volledig_scherm {
            let _ = SetWindowLongPtrW(hwnd, GWL_STYLE, WS_OVERLAPPEDWINDOW.0 as isize);
            let _ = SetWindowPlacement(hwnd, &staat.vorige_plaatsing);
            let _ = SetWindowPos(
                hwnd,
                None,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED,
            );
            staat.volledig_scherm = false;
            return;
        }

        let _ = GetWindowPlacement(hwnd, &mut staat.vorige_plaatsing);
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return;
        }
        let r = info.rcMonitor;
        let _ = SetWindowLongPtrW(hwnd, GWL_STYLE, WS_POPUP.0 as isize);
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOP),
            r.left,
            r.top,
            r.right - r.left,
            r.bottom - r.top,
            SWP_NOZORDER | SWP_FRAMECHANGED,
        );
        staat.volledig_scherm = true;
    }
}

/// Houdt de beeldverhouding vast tijdens het slepen aan een rand. Zonder dit rekt het
/// beeld mee met het venster, want de swapchain schaalt zonder naar de vorm te kijken.
fn bewaak_verhouding(rand: u32, kader: &mut RECT, verhouding: f32) {
    let breedte = (kader.right - kader.left) as f32;
    let hoogte = (kader.bottom - kader.top) as f32;
    if breedte <= 0.0 || hoogte <= 0.0 {
        return;
    }

    let horizontaal = matches!(
        rand,
        WMSZ_LEFT | WMSZ_RIGHT | WMSZ_TOPLEFT | WMSZ_TOPRIGHT | WMSZ_BOTTOMLEFT | WMSZ_BOTTOMRIGHT
    );
    if horizontaal {
        let nieuw = (breedte / verhouding).round() as i32;
        if matches!(rand, WMSZ_TOPLEFT | WMSZ_TOPRIGHT) {
            kader.top = kader.bottom - nieuw;
        } else {
            kader.bottom = kader.top + nieuw;
        }
    } else {
        let nieuw = (hoogte * verhouding).round() as i32;
        if rand == WMSZ_TOP || rand == WMSZ_BOTTOM {
            kader.right = kader.left + nieuw;
        }
    }
}

const WMSZ_LEFT: u32 = 1;
const WMSZ_RIGHT: u32 = 2;
const WMSZ_TOP: u32 = 3;
const WMSZ_TOPLEFT: u32 = 4;
const WMSZ_TOPRIGHT: u32 = 5;
const WMSZ_BOTTOM: u32 = 6;
const WMSZ_BOTTOMLEFT: u32 = 7;
const WMSZ_BOTTOMRIGHT: u32 = 8;

unsafe extern "system" fn venster_proc(
    hwnd: HWND,
    bericht: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: alles hieronder draait op de thread van het venster, en `Staat` staat in
    // GWLP_USERDATA vanaf WM_NCCREATE tot WM_NCDESTROY.
    unsafe {
        if bericht == WM_NCCREATE {
            let cs = &*(lparam.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
            return DefWindowProcW(hwnd, bericht, wparam, lparam);
        }

        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Staat;
        if ptr.is_null() {
            return DefWindowProcW(hwnd, bericht, wparam, lparam);
        }
        let staat = &mut *ptr;

        match bericht {
            // Sluiten melden we alleen; het opruimen doet de eigenaar via `drop`.
            // Zou dit venster zichzelf vernietigen, dan hield de kijkende thread een
            // ongeldige handle over.
            WM_CLOSE => {
                staat.gesloten = true;
                LRESULT(0)
            }
            WM_KEYDOWN => {
                // Escape haalt je alleen uit beeldvullend; als sluitknop is hij te
                // makkelijk per ongeluk te raken terwijl je meekijkt.
                let toets = wparam.0 as u16;
                let ctrl = GetKeyState(i32::from(VK_CONTROL.0)) < 0;
                if toets == VK_F11.0 || (toets == VK_ESCAPE.0 && staat.volledig_scherm) {
                    schakel_volledig_scherm(hwnd, staat);
                } else if toets == VK_D.0 && !ctrl {
                    let pen = !staat.teken.pen;
                    staat.teken.zet_pen(pen);
                } else if toets == VK_Z.0 && ctrl {
                    staat.teken.wis(DrawClearScope::LAST);
                } else if toets == VK_DELETE.0 {
                    staat.teken.wis(DrawClearScope::MINE);
                }
                LRESULT(0)
            }
            WM_LBUTTONDBLCLK => {
                staat.teken.lopend = None;
                schakel_volledig_scherm(hwnd, staat);
                LRESULT(0)
            }
            WM_LBUTTONDOWN if staat.teken.mag() => {
                staat.teken.begin(hwnd, lparam);
                SetCapture(hwnd);
                LRESULT(0)
            }
            WM_MOUSEMOVE if staat.teken.lopend.is_some() => {
                staat.teken.beweeg(hwnd, lparam);
                LRESULT(0)
            }
            WM_LBUTTONUP if staat.teken.lopend.is_some() => {
                // Eerst afronden: `ReleaseCapture` stuurt WM_CAPTURECHANGED hierheen terug.
                staat.teken.eind();
                let _ = ReleaseCapture();
                LRESULT(0)
            }
            WM_CAPTURECHANGED => {
                staat.teken.eind();
                LRESULT(0)
            }
            WM_RBUTTONUP => {
                toon_menu(hwnd, lparam, &mut staat.teken);
                LRESULT(0)
            }
            // Een kruisje zolang de pen op is, zodat je ziet dat slepen tekent.
            WM_SETCURSOR if staat.teken.mag() && (lparam.0 & 0xFFFF) as u32 == HTCLIENT => {
                let _ = SetCursor(LoadCursorW(None, IDC_CROSS).ok());
                LRESULT(1)
            }
            // De laag ligt over het clientgebied en moet meeschuiven en meegroeien.
            WM_MOVE | WM_SIZE => {
                staat.teken.herteken(hwnd);
                DefWindowProcW(hwnd, bericht, wparam, lparam)
            }
            WM_SIZING => {
                bewaak_verhouding(
                    wparam.0 as u32,
                    &mut *(lparam.0 as *mut RECT),
                    staat.verhouding,
                );
                LRESULT(1)
            }
            // De swapchain vult het hele venster; de achtergrond wissen zou alleen maar
            // flikkeren opleveren.
            WM_ERASEBKGND => LRESULT(1),
            WM_NCDESTROY => {
                staat.gesloten = true;
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                drop(Box::from_raw(ptr));
                DefWindowProcW(hwnd, bericht, wparam, lparam)
            }
            _ => DefWindowProcW(hwnd, bericht, wparam, lparam),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verhouding_blijft_gelijk_bij_slepen() {
        // 16:9 vasthouden. Bij slepen aan een zijkant volgt de hoogte de breedte.
        let mut kader = RECT {
            left: 0,
            top: 0,
            right: 1600,
            bottom: 500,
        };
        bewaak_verhouding(WMSZ_RIGHT, &mut kader, 16.0 / 9.0);
        assert_eq!(kader.bottom - kader.top, 900);

        // Aan de bovenkant slepen verplaatst de bovenrand, niet de onderrand: anders
        // zou het venster onder je muis vandaan lopen.
        let mut kader = RECT {
            left: 0,
            top: 100,
            right: 1600,
            bottom: 1000,
        };
        bewaak_verhouding(WMSZ_TOPRIGHT, &mut kader, 16.0 / 9.0);
        assert_eq!(kader.bottom - kader.top, 900);
        assert_eq!(kader.bottom, 1000, "de onderrand hoort te blijven staan");
    }

    #[test]
    fn slepen_aan_de_onderrand_past_de_breedte_aan() {
        let mut kader = RECT {
            left: 0,
            top: 0,
            right: 100,
            bottom: 900,
        };
        bewaak_verhouding(WMSZ_BOTTOM, &mut kader, 16.0 / 9.0);
        assert_eq!(kader.right - kader.left, 1600);
    }

    #[test]
    #[ignore = "opent een echt venster"]
    fn venster_toont_een_beeld() {
        use std::time::{Duration, Instant};

        let d3d = D3dContext::new().expect("D3D11");
        let (b, h) = (640u32, 360u32);
        let pixels: Vec<u8> = (0..b * h)
            .flat_map(|i| {
                // Verloop, zodat meteen te zien is of het beeld op zijn kop staat of
                // dat de kleurkanalen omgewisseld zijn.
                let x = (i % b) as u8;
                let y = (i / b) as u8;
                [x, y, 128, 255]
            })
            .collect();
        let tex = d3d.maak_textuur_met(b, h, &pixels).expect("textuur");

        let mut venster = Venster::open(&d3d, "FitCom — test", b, h).expect("venster");
        let tot = Instant::now() + Duration::from_secs(5);
        while Instant::now() < tot && venster.pomp() {
            venster.toon(&tex).expect("tonen");
            std::thread::sleep(Duration::from_millis(16));
        }
    }
}
