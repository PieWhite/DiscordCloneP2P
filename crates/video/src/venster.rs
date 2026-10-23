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
//! niets van.
//!
//! # Het menu onder de rechterknop
//!
//! Een eigen getekend venster ([`crate::kijkmenu`]) met het geluid van de stream (een schuif
//! op het bureaubladgeluid van de deler, dezelfde sleutel als in de audio-popover van de app),
//! tekenen en bedienen. Niet modaal: het beeld loopt door terwijl het open staat. Het menu
//! post zijn muisberichten hierheen; deze procedure beslist, met de staat die het al had.
//!
//! # Het scherm van de deler bedienen (beslissing 54)
//!
//! Staat de deler het toe, dan heeft het menu "Take control of this screen". Dat vraagt het de
//! deler, en pas als zijn gebruiker toestaat zet de motor dit venster op [`Bedienstand::Actief`].
//! Dan gaan muis, knoppen, wiel en toetsen naar de deler zolang dit venster de focus heeft, en
//! doet het venster zelf niets meer met F11, D, Escape, dubbelklikken of de rechterknop: die
//! horen dan bij de andere PC. Stoppen kan door Escape anderhalve seconde vast te houden (zoals
//! Chrome uit een toetsenbordvergrendeling haalt), door het venster te sluiten, of doordat de
//! deler het intrekt. Pen en bedienen sluiten elkaar uit. Verliest het venster de focus, dan
//! gaat alles los wat via ons ingedrukt stond. Het klembord gaat alleen mee zolang er bediend
//! wordt (`crate::klembord`).

use crate::d3d::D3dContext;
use crate::kijker::MenuStijl;
use crate::kijker::{Plaatsing, Scherm};
use crate::kijkmenu::{Doel, Inhoud, Kijkmenu, WM_KIJKMENU};
use crate::klembord;
use crate::tekenlaag::Laag;
use anyhow::{Context, Result};
use fitcom_proto::bediening::{Bedienstand, MAX_INVOER_PER_BERICHT};
use fitcom_proto::control::{
    ClipboardText, Draw, DrawClear, DrawClearScope, InputEvent, InputKind, RemoteInput,
    RemoteRequest,
};
use fitcom_proto::teken::{Tekening, MAX_PUNTEN_PER_BERICHT, MAX_PUNTEN_PER_LIJN};
use fitcom_proto::ControlMsg;
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use windows::core::{w, Interface, BOOL, PCWSTR};
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
    ClientToScreen, EnumDisplayMonitors, GetMonitorInfoW, MonitorFromWindow, ScreenToClient, HDC,
    HMONITOR, MONITORINFO, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTOPRIMARY,
};
use windows::Win32::System::DataExchange::{
    AddClipboardFormatListener, RemoveClipboardFormatListener,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetCapture, GetFocus, GetKeyState, ReleaseCapture, SetCapture, VK_CONTROL, VK_D, VK_DELETE,
    VK_ESCAPE, VK_F11, VK_NUMLOCK, VK_PACKET, VK_PAUSE, VK_PROCESSKEY, VK_Z,
};
use windows::Win32::UI::WindowsAndMessaging::*;

const KLASSE: PCWSTR = w!("FitcomVideoVenster");

/// Zo lang Escape vasthouden stopt het bedienen. Escape zelf gaat wel naar de deler.
const ESC_HOUD: Duration = Duration::from_millis(1500);

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
    bedien: Bedien,
    /// De titel zonder hint.
    titel: String,
    /// Wat er laatst aan titel, luisteraar en pen gedaan is: (tekenen mag, bedienstand). De
    /// vensterprocedure zet alleen de stand om; de gevolgen (die zelf weer berichten naar het
    /// venster sturen) doen [`Venster::werk`] en [`Venster::zet_bediening`], buiten de procedure.
    toegepast: Option<(bool, Bedienstand)>,
    /// Zonder stijl geen menu (tests).
    stijl: Option<MenuStijl>,
    menu: Option<Kijkmenu>,
    /// Het volume van het geluid bij dit beeld zoals de motor het kent, `None` zonder geluid.
    volume: Option<f32>,
    /// Wat de schuif in het menu koos en nog naar de motor moet.
    volume_uit: Option<f32>,
    /// De maat die de gebruiker het venster gaf, sinds de vorige keer gevraagd.
    maat_uit: Option<(u32, u32)>,
}

/// Wat er in de titel staat, na de naam van de stream. De enige aanwijzing dat tekenen en
/// bedienen bestaan, en tijdens het bedienen hoe je stopt.
fn titel_van(basis: &str, tekenen: bool, stand: Bedienstand) -> String {
    let hint = match stand {
        Bedienstand::Actief => "you control this screen \u{00b7} hold Esc to stop",
        Bedienstand::Gevraagd => "asked for control, waiting for them to allow it",
        Bedienstand::Mag if tekenen => "right-click to draw or take control",
        Bedienstand::Mag => "right-click to take control",
        Bedienstand::Uit if tekenen => "right-click to draw",
        Bedienstand::Uit => return basis.to_string(),
    };
    format!("{basis} \u{2014} {hint}")
}

/// Bedienen vanuit dit venster.
#[derive(Default)]
struct Bedien {
    stand: Bedienstand,
    /// Alleen met de focus gaat er invoer naar de deler.
    focus: bool,
    /// Wat we bij de deler ingedrukt hebben: dat gaat los bij focusverlies of stoppen.
    toetsen: BTreeSet<(u16, u8)>,
    /// Bit `n` voor muisknop `n`.
    knoppen: u8,
    /// Laatste plek van de muis op het beeld.
    plek: [u16; 2],
    /// Invoer sinds het vorige bericht; opeenvolgende bewegingen samengevat tot één.
    invoer: Vec<InputEvent>,
    /// Klaar om naar de deler te gaan, in volgorde.
    uit: Vec<BedienUit>,
    /// Sinds wanneer Escape omlaag staat.
    esc_sinds: Option<Instant>,
    /// Het venster luistert naar het klembord (alleen tijdens het bedienen).
    luistert: bool,
}

enum BedienUit {
    Invoer(Vec<InputEvent>),
    Vraag(bool),
    Klembord(String),
}

impl Bedien {
    fn actief(&self) -> bool {
        self.stand == Bedienstand::Actief
    }

    /// Eén gebeurtenis erbij, en bijhouden wat er ingedrukt staat.
    fn duw(&mut self, e: InputEvent) {
        match e.kind {
            InputKind::KEY_DOWN => {
                self.toetsen.insert((e.scan, e.vk));
            }
            InputKind::KEY_UP => {
                self.toetsen.remove(&(e.scan, e.vk));
            }
            InputKind::BUTTON_DOWN => self.knoppen |= 1 << e.button,
            InputKind::BUTTON_UP => self.knoppen &= !(1 << e.button),
            InputKind::MOVE => {
                if let Some(vorige) = self.invoer.last_mut().filter(|v| v.kind == InputKind::MOVE) {
                    *vorige = e;
                    return;
                }
            }
            _ => {}
        }
        self.invoer.push(e);
    }

    /// Alles wat we ingedrukt hebben los, bij de deler.
    fn alles_los(&mut self) {
        for (scan, vk) in std::mem::take(&mut self.toetsen) {
            self.invoer.push(InputEvent {
                scan,
                vk,
                ..leeg(InputKind::KEY_UP)
            });
        }
        let [x, y] = self.plek;
        for knop in 1..=5u8 {
            if self.knoppen & (1 << knop) != 0 {
                self.invoer.push(InputEvent {
                    x,
                    y,
                    button: knop,
                    ..leeg(InputKind::BUTTON_UP)
                });
            }
        }
        self.knoppen = 0;
        self.esc_sinds = None;
    }

    /// Wat er aan invoer klaarstaat in de rij voor de deler.
    fn spoel(&mut self) {
        if !self.invoer.is_empty() {
            self.uit
                .push(BedienUit::Invoer(std::mem::take(&mut self.invoer)));
        }
    }

    /// Zelf stoppen: eerst alles los (dat komt nog aan zolang we bediener zijn), dan zeggen
    /// dat we stoppen.
    fn stop(&mut self) {
        self.alles_los();
        self.spoel();
        self.uit.push(BedienUit::Vraag(false));
        self.stand = Bedienstand::Mag;
    }

    /// De deler (of de motor) maakte er een eind aan: de deler laat zelf alles los, dus hier
    /// alleen vergeten.
    fn vergeet(&mut self) {
        self.toetsen.clear();
        self.knoppen = 0;
        self.invoer.clear();
        self.esc_sinds = None;
        self.uit.retain(|u| !matches!(u, BedienUit::Invoer(_)));
    }

    fn werk(&mut self, stream_id: u32) -> Vec<ControlMsg> {
        if self.actief() && self.esc_sinds.is_some_and(|t| t.elapsed() >= ESC_HOUD) {
            tracing::info!(stream = stream_id, "Escape vastgehouden; bedienen gestopt");
            self.stop();
        }
        self.spoel();
        let mut berichten = Vec::new();
        for uit in self.uit.drain(..) {
            match uit {
                BedienUit::Invoer(events) => {
                    for stuk in events.chunks(MAX_INVOER_PER_BERICHT) {
                        berichten.push(ControlMsg::RemoteInput(RemoteInput {
                            stream_id,
                            events: stuk.to_vec(),
                        }));
                    }
                }
                BedienUit::Vraag(want) => {
                    berichten.push(ControlMsg::RemoteRequest(RemoteRequest { stream_id, want }))
                }
                BedienUit::Klembord(text) => {
                    berichten.push(ControlMsg::ClipboardText(ClipboardText { stream_id, text }))
                }
            }
        }
        berichten
    }
}

fn leeg(kind: InputKind) -> InputEvent {
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

/// Een toetsbericht van Windows als invoer voor de deler: de scancode uit `lparam` (bit 24 is
/// "uitgebreid"), de virtuele toets erbij. `None` voor wat geen toets van het toetsenbord is
/// (IME, `VK_PACKET`) of niet geldig.
fn toets_van(wparam: WPARAM, lparam: LPARAM, los: bool) -> Option<InputEvent> {
    let vk = (wparam.0 & 0xFF) as u8;
    if matches!(u16::from(vk), v if v == VK_PROCESSKEY.0 || v == VK_PACKET.0) {
        return None;
    }
    let scan = ((lparam.0 >> 16) & 0xFF) as u16;
    let uitgebreid = (lparam.0 >> 24) & 1 == 1;
    // Pause en NumLock delen scancode 0x45 en zijn als scancode niet uit elkaar te houden: die
    // gaan als virtuele toets.
    let scan = if scan == 0 || matches!(u16::from(vk), v if v == VK_PAUSE.0 || v == VK_NUMLOCK.0) {
        0
    } else if uitgebreid {
        0xE000 | scan
    } else {
        scan
    };
    let e = InputEvent {
        scan,
        vk,
        ..leeg(if los {
            InputKind::KEY_UP
        } else {
            InputKind::KEY_DOWN
        })
    };
    e.is_geldig().then_some(e)
}

/// Welke muisknop een knopbericht noemt, en of hij omlaag gaat.
fn knop_van(bericht: u32, wparam: WPARAM) -> Option<(u8, bool)> {
    let x = if ((wparam.0 >> 16) & 0xFFFF) as u16 == XBUTTON2 {
        5
    } else {
        4
    };
    Some(match bericht {
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => (1, true),
        WM_LBUTTONUP => (1, false),
        WM_RBUTTONDOWN | WM_RBUTTONDBLCLK => (2, true),
        WM_RBUTTONUP => (2, false),
        WM_MBUTTONDOWN | WM_MBUTTONDBLCLK => (3, true),
        WM_MBUTTONUP => (3, false),
        WM_XBUTTONDOWN | WM_XBUTTONDBLCLK => (x, true),
        WM_XBUTTONUP => (x, false),
        _ => return None,
    })
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
    fn nieuw() -> Self {
        Self {
            tekening: Arc::new(Tekening::nieuw(false)),
            pen: false,
            lopend: None,
            uit: Vec::new(),
            laatst_verstuurd: Instant::now(),
            laag: None,
        }
    }

    fn mag(&self) -> bool {
        self.tekening.aan && self.pen
    }

    /// De titel volgt via `pas_toe`.
    fn zet(&mut self, hwnd: HWND, t: Arc<Tekening>) {
        let wissel = self.tekening.aan != t.aan;
        self.tekening = t;
        if wissel && !self.tekening.aan {
            self.pen = false;
            self.lopend = None;
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

/// Wat het menu laat zien, zoals het venster er nu voor staat.
fn inhoud(s: &Staat) -> Inhoud {
    Inhoud {
        kop: s.titel.clone(),
        volume: s.volume,
        tekenen: s.teken.tekening.aan,
        pen: s.teken.pen,
        kleur: usize::from(PEN_KLEUR.load(Ordering::Relaxed)),
        dikte: usize::from(PEN_DIKTE.load(Ordering::Relaxed)),
        bedien: s.bedien.stand,
    }
}

/// Het menu onder de rechterknop openen, waar de muis is. Tijdens het bedienen is de
/// rechterknop van de deler en komt dit menu niet.
fn open_menu(hwnd: HWND, lparam: LPARAM, staat: &mut Staat) {
    let Some(stijl) = staat.stijl else {
        return;
    };
    staat.menu = None;
    let (x, y) = muis_px(lparam);
    let mut punt = POINT { x, y };
    // SAFETY: eigen venster.
    unsafe {
        let _ = ClientToScreen(hwnd, &mut punt);
    }
    match Kijkmenu::open(hwnd, punt, stijl, &inhoud(staat)) {
        Ok(m) => staat.menu = Some(m),
        Err(e) => tracing::warn!(error = %format!("{e:#}"), "menu van het kijkvenster"),
    }
}

/// Eén muisbericht van het menu, met de plek in clientpixels van het menu. Een klik ernaast
/// of de muis kwijt sluit het; loslaten op een rij voert die uit en sluit het, behalve op de
/// schuif, die het volume zet zolang de knop omlaag is.
fn menu_bericht(staat: &mut Staat, bericht: u32, lparam: LPARAM) {
    let Some(menu) = staat.menu.as_mut() else {
        return;
    };
    let (x, y) = muis_px(lparam);
    let mut volume = None;
    match bericht {
        WM_MOUSEMOVE if menu.sleept => volume = menu.volume_op(x),
        WM_MOUSEMOVE => menu.zet_hover(menu.doel_op(x, y)),
        WM_LBUTTONDOWN if menu.doel_op(x, y) == Some(Doel::Volume) => {
            menu.sleept = true;
            volume = menu.volume_op(x);
        }
        WM_LBUTTONUP if menu.sleept => menu.sleept = false,
        WM_LBUTTONUP => match menu.doel_op(x, y) {
            Some(Doel::Volume) | None => {}
            Some(doel) => {
                staat.menu = None;
                voer_uit(staat, doel);
            }
        },
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN if !menu.raakt(x, y) => {
            staat.menu = None;
        }
        WM_CAPTURECHANGED => staat.menu = None,
        _ => {}
    }
    if let Some(v) = volume.filter(|v| staat.volume != Some(*v)) {
        staat.volume = Some(v);
        staat.volume_uit = Some(v);
        let i = inhoud(staat);
        if let Some(m) = staat.menu.as_mut() {
            m.toon(&i);
        }
    }
}

/// Wat een gekozen rij doet.
fn voer_uit(staat: &mut Staat, doel: Doel) {
    let teken = &mut staat.teken;
    match doel {
        Doel::Bedien => match staat.bedien.stand {
            Bedienstand::Mag => {
                // Pen en bedienen sluiten elkaar uit.
                teken.zet_pen(false);
                staat.bedien.uit.push(BedienUit::Vraag(true));
                staat.bedien.stand = Bedienstand::Gevraagd;
            }
            Bedienstand::Gevraagd => {
                staat.bedien.uit.push(BedienUit::Vraag(false));
                staat.bedien.stand = Bedienstand::Mag;
            }
            Bedienstand::Actief => staat.bedien.stop(),
            Bedienstand::Uit => {}
        },
        Doel::Pen => teken.zet_pen(!teken.pen),
        Doel::Kleur(k) => {
            PEN_KLEUR.store(k as u8, Ordering::Relaxed);
            teken.zet_pen(true);
        }
        Doel::Dikte(d) => {
            PEN_DIKTE.store(d as u8, Ordering::Relaxed);
            teken.zet_pen(true);
        }
        Doel::Ongedaan => teken.wis(DrawClearScope::LAST),
        Doel::Wis => teken.wis(DrawClearScope::MINE),
        Doel::Volume => {}
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
    ///
    /// `plaatsing.vanzelf`: vanzelf geopend (een nieuw gedeeld scherm). Dan gaat hij naar dat
    /// scherm en **pakt hij de focus niet**: vooraan staat misschien een spel, en een alt-tab
    /// is precies wat invariant 4 verbiedt. `plaatsing.maat`: de maat die de gebruiker de
    /// vorige keer koos, met het beeld erin gepast en nooit groter dan het scherm.
    pub fn open(
        d3d: &D3dContext,
        titel: &str,
        breedte: u32,
        hoogte: u32,
        plaatsing: Plaatsing,
    ) -> Result<Self> {
        registreer_klasse()?;

        let staat = Box::new(Staat {
            gesloten: false,
            volledig_scherm: false,
            vorige_plaatsing: WINDOWPLACEMENT {
                length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
                ..Default::default()
            },
            verhouding: breedte as f32 / hoogte.max(1) as f32,
            teken: Teken::nieuw(),
            bedien: Bedien::default(),
            titel: titel.to_string(),
            toegepast: None,
            stijl: None,
            menu: None,
            volume: None,
            volume_uit: None,
            maat_uit: None,
        });

        let naam: Vec<u16> = titel.encode_utf16().chain(std::iter::once(0)).collect();
        let beeld = (breedte.max(1) as i32, hoogte.max(1) as i32);
        // De maat van het beeld, of het beeld gepast in de maat die de gebruiker koos.
        let client = plaatsing
            .maat
            .map_or(beeld, |(b, h)| pas_in(beeld, (b as i32, h as i32)));
        // Het venster moet zijn rand daar nog omheen krijgen, anders is het beeld precies die
        // rand te klein.
        let mut kader = RECT {
            left: 0,
            top: 0,
            right: client.0,
            bottom: client.1,
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

        let rand = (
            kader.right - kader.left - client.0,
            kader.bottom - kader.top - client.1,
        );
        // SAFETY: `hwnd` is net aangemaakt.
        unsafe {
            match plaatsing.vanzelf {
                // Midden op dat scherm, zo groot als past (zonder gekozen maat: zo groot als het
                // scherm toelaat). Niet maximaliseren: dat activeert altijd, en rekt het beeld.
                Some(scherm) => {
                    if let Some(r) = werkgebied(scherm) {
                        let ruimte = (r.right - r.left - rand.0, r.bottom - r.top - rand.1);
                        let c = match plaatsing.maat {
                            Some(_) => niet_groter(client, ruimte),
                            None => pas_in(beeld, ruimte),
                        };
                        let (b, h) = (c.0 + rand.0, c.1 + rand.1);
                        let _ = SetWindowPos(
                            hwnd,
                            None,
                            r.left + (r.right - r.left - b) / 2,
                            r.top + (r.bottom - r.top - h) / 2,
                            b,
                            h,
                            SWP_NOZORDER | SWP_NOACTIVATE,
                        );
                    }
                    let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                }
                None => {
                    // Een onthouden maat kan van een groter scherm komen: dan kleiner, en
                    // binnen het scherm waar Windows hem neerzette.
                    if plaatsing.maat.is_some() {
                        if let Some(r) = werkgebied_van(hwnd) {
                            let ruimte = (r.right - r.left - rand.0, r.bottom - r.top - rand.1);
                            let c = niet_groter(client, ruimte);
                            let (b, h) = (c.0 + rand.0, c.1 + rand.1);
                            let mut nu = RECT::default();
                            let _ = GetWindowRect(hwnd, &mut nu);
                            let x = nu.left.clamp(r.left, (r.right - b).max(r.left));
                            let y = nu.top.clamp(r.top, (r.bottom - h).max(r.top));
                            let _ = SetWindowPos(hwnd, None, x, y, b, h, SWP_NOZORDER);
                        }
                    }
                    let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
                }
            }
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

    fn staat(&mut self) -> Option<&mut Staat> {
        // SAFETY: zoals bij `is_gesloten`; we draaien op de thread van het venster en de
        // vensterprocedure loopt nu niet.
        unsafe {
            let ptr = GetWindowLongPtrW(self.hwnd, GWLP_USERDATA) as *mut Staat;
            (!ptr.is_null()).then(|| &mut *ptr)
        }
    }

    #[cfg(test)]
    fn teken(&mut self) -> Option<&mut Teken> {
        self.staat().map(|s| &mut s.teken)
    }

    /// De tekening van de deler, zoals de motor hem nu kent. Alleen bij een wijziging; de
    /// laag tekent dan bij wat er veranderde.
    pub fn zet_tekening(&mut self, t: Arc<Tekening>) {
        let hwnd = self.hwnd;
        if let Some(s) = self.staat() {
            s.teken.zet(hwnd, t);
        }
        self.pas_toe();
    }

    /// Waar we staan met het bedienen van dit scherm, zoals de motor het kent. Zet de motor ons
    /// uit `Actief`, dan heeft de deler het al losgelaten; hier valt dan alleen te vergeten.
    pub fn zet_bediening(&mut self, stand: Bedienstand) {
        if let Some(s) = self.staat() {
            if s.bedien.stand == stand {
                return;
            }
            if s.bedien.actief() {
                s.bedien.vergeet();
            }
            s.bedien.stand = stand;
        }
        self.pas_toe();
    }

    /// Klembordtekst van de deler; alleen terwijl we hem bedienen.
    pub fn zet_klembord(&mut self, tekst: String) {
        let hwnd = self.hwnd;
        if self.staat().is_some_and(|s| s.bedien.actief()) && !klembord::schrijf(hwnd, &tekst) {
            tracing::debug!(len = tekst.len(), "klembord zetten mislukt");
        }
    }

    /// Hoe het menu onder de rechterknop eruitziet; zonder stijl komt er geen menu.
    pub fn zet_menu_stijl(&mut self, stijl: MenuStijl) {
        if let Some(s) = self.staat() {
            s.stijl = Some(stijl);
        }
    }

    /// Het volume van het geluid bij dit beeld, of `None` als er geen is. Een open menu volgt,
    /// behalve terwijl de schuif vastgehouden wordt: dan is die de bron, en een oudere waarde
    /// van de motor zou hem terugzetten.
    pub fn zet_volume(&mut self, volume: Option<f32>) {
        let Some(s) = self.staat() else {
            return;
        };
        if s.menu.as_ref().is_some_and(|m| m.sleept) {
            return;
        }
        s.volume = volume;
        let i = inhoud(s);
        if let Some(m) = s.menu.as_mut() {
            m.toon(&i);
        }
    }

    /// Wat de schuif in het menu koos sinds de vorige keer, voor de motor.
    pub fn volume_keuze(&mut self) -> Option<f32> {
        self.staat().and_then(|s| s.volume_uit.take())
    }

    /// De maat die de gebruiker het venster gaf sinds de vorige keer, voor de motor.
    pub fn maat_keuze(&mut self) -> Option<(u32, u32)> {
        self.staat().and_then(|s| s.maat_uit.take())
    }

    /// Wat de gebruiker tekende, wiste, vroeg of als bediener deed sinds de vorige keer, als
    /// berichten voor de deler. Elke 8 ms gevraagd.
    pub fn werk(&mut self, stream_id: u32) -> Vec<ControlMsg> {
        let mut uit = Vec::new();
        if let Some(s) = self.staat() {
            uit = s.teken.werk(stream_id);
            uit.extend(s.bedien.werk(stream_id));
        }
        self.pas_toe();
        uit
    }

    /// De gevolgen van een andere stand of een andere tekenschakelaar: titel, klembordluisteraar,
    /// pen en muisvangst. Hier en niet in de vensterprocedure, want elk van die aanroepen stuurt
    /// zelf weer berichten naar het venster.
    fn pas_toe(&mut self) {
        let hwnd = self.hwnd;
        let Some(s) = self.staat() else {
            return;
        };
        let nu = (s.teken.tekening.aan, s.bedien.stand);
        if s.toegepast == Some(nu) {
            return;
        }
        let was = s.toegepast.map(|(_, stand)| stand);
        s.toegepast = Some(nu);
        // Wat het menu liet zien klopt niet meer.
        s.menu = None;
        let titel = titel_van(&s.titel, nu.0, nu.1);
        let mut vangst_los = false;
        // SAFETY: eigen venster, eigen thread.
        unsafe {
            if nu.1 == Bedienstand::Actief && was != Some(Bedienstand::Actief) {
                // Een lijn in aanbouw stopt, en zijn muisvangst ook: anders komt het loslaten
                // van die knop nooit meer aan.
                s.teken.zet_pen(false);
                s.bedien.focus = GetFocus() == hwnd;
                s.bedien.luistert = AddClipboardFormatListener(hwnd).is_ok();
                vangst_los = true;
            } else if nu.1 != Bedienstand::Actief && was == Some(Bedienstand::Actief) {
                if s.bedien.luistert {
                    let _ = RemoveClipboardFormatListener(hwnd);
                    s.bedien.luistert = false;
                }
                vangst_los = s.teken.lopend.is_none();
            }
            let breed: Vec<u16> = titel.encode_utf16().chain(std::iter::once(0)).collect();
            let _ = SetWindowTextW(hwnd, PCWSTR(breed.as_ptr()));
            if vangst_los && GetCapture() == hwnd {
                let _ = ReleaseCapture();
            }
        }
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

/// Heeft deze pc meer dan één scherm? Goedkoop genoeg om bij elke nieuwe stream te vragen.
pub fn heeft_tweede_scherm() -> bool {
    // SAFETY: geen argumenten om fout te doen.
    unsafe { GetSystemMetrics(SM_CMONITORS) > 1 }
}

/// Het grootste dat in `ruimte` past in de verhouding van `beeld` (groter of kleiner).
fn pas_in(beeld: (i32, i32), ruimte: (i32, i32)) -> (i32, i32) {
    let schaal =
        (ruimte.0 as f32 / beeld.0.max(1) as f32).min(ruimte.1 as f32 / beeld.1.max(1) as f32);
    (
        ((beeld.0 as f32 * schaal) as i32).max(1),
        ((beeld.1 as f32 * schaal) as i32).max(1),
    )
}

/// `maat` zoals hij is als hij past, anders verkleind tot hij past.
fn niet_groter(maat: (i32, i32), ruimte: (i32, i32)) -> (i32, i32) {
    if maat.0 <= ruimte.0 && maat.1 <= ruimte.1 {
        maat
    } else {
        pas_in(maat, ruimte)
    }
}

/// Het werkgebied van het gekozen scherm; een monitor die er niet meer is wordt `Ander`.
fn werkgebied(scherm: Scherm) -> Option<RECT> {
    if let Scherm::Monitor(h) = scherm {
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        // SAFETY: een verouderde handle laat `GetMonitorInfoW` alleen falen.
        if unsafe { GetMonitorInfoW(HMONITOR(h as *mut _), &mut info) }.as_bool() {
            return Some(info.rcWork);
        }
    }
    ander_scherm()
}

/// Het werkgebied van het scherm waar `hwnd` op staat.
fn werkgebied_van(hwnd: HWND) -> Option<RECT> {
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: `hwnd` is geldig.
    unsafe { GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut info) }
        .as_bool()
        .then_some(info.rcWork)
}

/// Het werkgebied van een scherm waar het venster vooraan niet op staat, of `None` met één
/// scherm. Vooraan is waar je nu bent (je spel); het andere scherm is "je tweede scherm".
fn ander_scherm() -> Option<RECT> {
    unsafe extern "system" fn per_monitor(m: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        // SAFETY: `ander_scherm` geeft precies dit type mee achter `LPARAM`.
        let (vooraan, uit) = unsafe { &mut *(data.0 as *mut (HMONITOR, Option<RECT>)) };
        if m == *vooraan {
            return BOOL(1);
        }
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        // SAFETY: `m` komt van de enumeratie.
        if unsafe { GetMonitorInfoW(m, &mut info) }.as_bool() {
            *uit = Some(info.rcWork);
            return BOOL(0);
        }
        BOOL(1)
    }

    // SAFETY: zonder voorgrondvenster valt `MonitorFromWindow` terug op het hoofdscherm; de
    // callback krijgt een pointer naar een lokale die de hele opsomming blijft leven.
    unsafe {
        let mut zoek = (
            MonitorFromWindow(GetForegroundWindow(), MONITOR_DEFAULTTOPRIMARY),
            None,
        );
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(per_monitor),
            LPARAM(&mut zoek as *mut _ as isize),
        );
        zoek.1
    }
}

/// Beeldvullend zonder modeswitch: rand eraf en over het scherm heen waar het venster
/// op staat. Een echte fullscreen-modus zou de resolutie omgooien, en dat is precies
/// wat je niet wilt terwijl er op datzelfde scherm een game draait.
fn schakel_volledig_scherm(hwnd: HWND, staat: &mut Staat) {
    // SAFETY: `hwnd` is geldig en we draaien op de thread van het venster.
    unsafe {
        // De aanroepen hieronder sturen WM_SIZE/WM_MOVE terug naar `venster_proc`, die de
        // tekenlaag verplaatst. Dus eerst de staat bijwerken en na die aanroepen niet meer
        // aanraken.
        if staat.volledig_scherm {
            staat.volledig_scherm = false;
            let plaatsing = staat.vorige_plaatsing;
            let _ = SetWindowLongPtrW(hwnd, GWL_STYLE, WS_OVERLAPPEDWINDOW.0 as isize);
            let _ = SetWindowPlacement(hwnd, &plaatsing);
            let _ = SetWindowPos(
                hwnd,
                None,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED,
            );
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
        staat.volledig_scherm = true;
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

/// Eén vensterbericht tijdens het bedienen. `Some` = afgehandeld (niet verder door de procedure
/// en niet naar Windows), `None` = gewoon verder.
///
/// # Safety
/// Alleen vanuit de vensterprocedure, op de thread van het venster.
unsafe fn bedien_bericht(
    hwnd: HWND,
    b: &mut Bedien,
    bericht: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> Option<LRESULT> {
    let esc = VK_ESCAPE.0 as u8;
    match bericht {
        WM_SETFOCUS => {
            b.focus = true;
            None
        }
        WM_KILLFOCUS => {
            b.focus = false;
            b.alles_los();
            None
        }
        // De klik die het venster vooraan haalt gaat niet naar de deler: wie terugkomt in het
        // venster klikt dan niet per ongeluk iets aan op de andere PC. De titelbalk werkt gewoon.
        WM_MOUSEACTIVATE if (lparam.0 & 0xFFFF) as u32 == HTCLIENT => {
            Some(LRESULT(MA_ACTIVATEANDEAT as isize))
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            if b.focus {
                if let Some(e) = toets_van(wparam, lparam, false) {
                    let herhaling = (lparam.0 >> 30) & 1 == 1;
                    if e.vk == esc && !herhaling {
                        b.esc_sinds = Some(Instant::now());
                    }
                    b.duw(e);
                }
            }
            Some(LRESULT(0))
        }
        WM_KEYUP | WM_SYSKEYUP => {
            if let Some(e) = toets_van(wparam, lparam, true) {
                if e.vk == esc {
                    b.esc_sinds = None;
                }
                if b.focus || b.toetsen.contains(&(e.scan, e.vk)) {
                    b.duw(e);
                }
            }
            Some(LRESULT(0))
        }
        // Tekens maakt de deler zelf uit de scancodes; hier niets, ook geen piep bij Alt+letter.
        WM_CHAR | WM_SYSCHAR | WM_DEADCHAR | WM_SYSDEADCHAR => Some(LRESULT(0)),
        WM_MOUSEMOVE => {
            if b.focus {
                b.plek = normaliseer(hwnd, muis_px(lparam));
                let [x, y] = b.plek;
                b.duw(InputEvent {
                    x,
                    y,
                    ..leeg(InputKind::MOVE)
                });
            }
            Some(LRESULT(0))
        }
        WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
            if b.focus {
                // Bij het wiel staat de plek in schermcoördinaten.
                let (sx, sy) = muis_px(lparam);
                let mut p = POINT { x: sx, y: sy };
                let _ = ScreenToClient(hwnd, &mut p);
                b.plek = normaliseer(hwnd, (p.x, p.y));
                let [x, y] = b.plek;
                let e = InputEvent {
                    x,
                    y,
                    delta: ((wparam.0 >> 16) & 0xFFFF) as u16 as i16,
                    ..leeg(if bericht == WM_MOUSEWHEEL {
                        InputKind::WHEEL
                    } else {
                        InputKind::HWHEEL
                    })
                };
                if e.is_geldig() {
                    b.duw(e);
                }
            }
            Some(LRESULT(0))
        }
        // De muis kwijt midden in een sleep (Alt+Tab, een ander venster pakt hem): knoppen los.
        WM_CAPTURECHANGED => {
            if b.knoppen != 0 {
                let [x, y] = b.plek;
                for knop in 1..=5u8 {
                    if b.knoppen & (1 << knop) != 0 {
                        b.duw(InputEvent {
                            x,
                            y,
                            button: knop,
                            ..leeg(InputKind::BUTTON_UP)
                        });
                    }
                }
            }
            Some(LRESULT(0))
        }
        WM_CLIPBOARDUPDATE => {
            if b.luistert {
                if let Some(tekst) = klembord::verandering(hwnd) {
                    b.uit.push(BedienUit::Klembord(tekst));
                }
            }
            Some(LRESULT(0))
        }
        _ => {
            let (knop, omlaag) = knop_van(bericht, wparam)?;
            b.plek = normaliseer(hwnd, muis_px(lparam));
            let [x, y] = b.plek;
            if omlaag {
                if b.focus {
                    b.duw(InputEvent {
                        x,
                        y,
                        button: knop,
                        ..leeg(InputKind::BUTTON_DOWN)
                    });
                    SetCapture(hwnd);
                }
            } else if b.knoppen & (1 << knop) != 0 {
                b.duw(InputEvent {
                    x,
                    y,
                    button: knop,
                    ..leeg(InputKind::BUTTON_UP)
                });
                // Als laatste: `ReleaseCapture` stuurt WM_CAPTURECHANGED hierheen terug.
                if b.knoppen == 0 {
                    let _ = ReleaseCapture();
                }
            }
            // De zijknoppen willen TRUE terug als ze afgehandeld zijn.
            let x_knop = matches!(bericht, WM_XBUTTONDOWN | WM_XBUTTONUP | WM_XBUTTONDBLCLK);
            Some(LRESULT(isize::from(x_knop)))
        }
    }
}

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

        // Tijdens het bedienen gaan muis en toetsen naar de deler, en doet het venster zelf
        // niets met F11, D, Escape, dubbelklikken of de rechterknop.
        if staat.bedien.actief() {
            if let Some(r) = bedien_bericht(hwnd, &mut staat.bedien, bericht, wparam, lparam) {
                return r;
            }
        }

        match bericht {
            // Sluiten melden we alleen; het opruimen doet de eigenaar via `drop`.
            // Zou dit venster zichzelf vernietigen, dan hield de kijkende thread een
            // ongeldige handle over.
            WM_CLOSE => {
                staat.gesloten = true;
                LRESULT(0)
            }
            WM_KIJKMENU => {
                menu_bericht(staat, wparam.0 as u32, lparam);
                LRESULT(0)
            }
            // Een toets sluit het menu: wat het liet zien kan erdoor veranderen. Escape doet
            // dan verder niets.
            WM_KEYDOWN if staat.menu.take().is_some() && wparam.0 as u16 == VK_ESCAPE.0 => {
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
                open_menu(hwnd, lparam, staat);
                LRESULT(0)
            }
            WM_KILLFOCUS => {
                staat.menu = None;
                DefWindowProcW(hwnd, bericht, wparam, lparam)
            }
            // Een kruisje zolang de pen op is, zodat je ziet dat slepen tekent.
            WM_SETCURSOR if staat.teken.mag() && (lparam.0 & 0xFFFF) as u32 == HTCLIENT => {
                let _ = SetCursor(LoadCursorW(None, IDC_CROSS).ok());
                LRESULT(1)
            }
            // De laag ligt over het clientgebied en moet meeschuiven en meegroeien.
            WM_MOVE | WM_SIZE => {
                staat.menu = None;
                staat.teken.herteken(hwnd);
                DefWindowProcW(hwnd, bericht, wparam, lparam)
            }
            // Eén keer per sleep, niet per beweging: de maat voor de volgende kijkvensters.
            WM_EXITSIZEMOVE => {
                let mut r = RECT::default();
                if !staat.volledig_scherm
                    && !IsZoomed(hwnd).as_bool()
                    && !IsIconic(hwnd).as_bool()
                    && GetClientRect(hwnd, &mut r).is_ok()
                    && r.right > 0
                    && r.bottom > 0
                {
                    staat.maat_uit = Some((r.right as u32, r.bottom as u32));
                }
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
    fn onthouden_maat_past_het_beeld_in_zijn_verhouding() {
        // 16:9 in een vierkant van 800: zo breed als kan.
        assert_eq!(pas_in((1920, 1080), (800, 800)), (800, 450));
        // Een gekozen maat die op het scherm past blijft, een te grote krimpt in verhouding.
        assert_eq!(niet_groter((1280, 720), (1920, 1040)), (1280, 720));
        assert_eq!(niet_groter((2560, 1440), (1280, 1000)), (1280, 720));
    }

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
    fn de_titel_zegt_wat_kan_en_hoe_je_stopt() {
        assert_eq!(titel_van("Rick", false, Bedienstand::Uit), "Rick");
        assert!(titel_van("Rick", true, Bedienstand::Uit).ends_with("right-click to draw"));
        assert!(titel_van("Rick", true, Bedienstand::Mag).ends_with("draw or take control"));
        assert!(titel_van("Rick", false, Bedienstand::Actief).contains("hold Esc to stop"));
    }

    /// `lparam` van een toetsbericht: scancode in bit 16-23, "uitgebreid" in bit 24.
    fn toets_lparam(scan: u8, uitgebreid: bool) -> LPARAM {
        LPARAM(((scan as isize) << 16) | (isize::from(uitgebreid) << 24))
    }

    #[test]
    fn een_toets_gaat_als_scancode_met_uitgebreid() {
        let a = toets_van(WPARAM(0x41), toets_lparam(0x1E, false), false).unwrap();
        assert_eq!((a.kind, a.scan, a.vk), (InputKind::KEY_DOWN, 0x1E, 0x41));
        let rctrl = toets_van(WPARAM(0xA3), toets_lparam(0x1D, true), true).unwrap();
        assert_eq!((rctrl.kind, rctrl.scan), (InputKind::KEY_UP, 0xE01D));
        // Pause en NumLock als virtuele toets; IME en VK_PACKET helemaal niet.
        let pauze = toets_van(
            WPARAM(VK_PAUSE.0 as usize),
            toets_lparam(0x45, false),
            false,
        );
        assert_eq!(pauze.map(|e| (e.scan, e.vk)), Some((0, VK_PAUSE.0 as u8)));
        assert!(toets_van(
            WPARAM(VK_PROCESSKEY.0 as usize),
            toets_lparam(0x1E, false),
            false
        )
        .is_none());
        assert!(toets_van(WPARAM(VK_PACKET.0 as usize), toets_lparam(0, false), false).is_none());
    }

    #[test]
    fn knoppen_en_dubbelklik_zijn_gewoon_omlaag() {
        assert_eq!(knop_van(WM_LBUTTONDBLCLK, WPARAM(0)), Some((1, true)));
        assert_eq!(knop_van(WM_RBUTTONUP, WPARAM(0)), Some((2, false)));
        let x2 = WPARAM((XBUTTON2 as usize) << 16);
        assert_eq!(knop_van(WM_XBUTTONDOWN, x2), Some((5, true)));
        assert_eq!(knop_van(WM_XBUTTONUP, WPARAM(1 << 16)), Some((4, false)));
        assert_eq!(knop_van(WM_MOUSEMOVE, WPARAM(0)), None);
    }

    fn beweeg(x: u16) -> InputEvent {
        InputEvent {
            x,
            ..leeg(InputKind::MOVE)
        }
    }

    #[test]
    fn bewegingen_worden_samengevat_maar_niet_over_een_klik_heen() {
        let mut b = Bedien {
            stand: Bedienstand::Actief,
            ..Default::default()
        };
        b.duw(beweeg(1));
        b.duw(beweeg(2));
        b.duw(InputEvent {
            button: 1,
            ..leeg(InputKind::BUTTON_DOWN)
        });
        b.duw(beweeg(3));
        b.duw(beweeg(4));
        let x: Vec<(InputKind, u16)> = b.invoer.iter().map(|e| (e.kind, e.x)).collect();
        assert_eq!(
            x,
            vec![
                (InputKind::MOVE, 2),
                (InputKind::BUTTON_DOWN, 0),
                (InputKind::MOVE, 4)
            ]
        );
    }

    #[test]
    fn zelf_stoppen_laat_eerst_alles_los_en_zegt_het_dan() {
        let mut b = Bedien {
            stand: Bedienstand::Actief,
            ..Default::default()
        };
        b.duw(toets_van(WPARAM(0x10), toets_lparam(0x2A, false), false).unwrap());
        b.duw(InputEvent {
            button: 2,
            ..leeg(InputKind::BUTTON_DOWN)
        });
        // Escape is al lang genoeg ingedrukt.
        b.esc_sinds = Instant::now().checked_sub(ESC_HOUD);
        let uit = b.werk(9);
        assert_eq!(b.stand, Bedienstand::Mag);
        match uit.as_slice() {
            [ControlMsg::RemoteInput(i), ControlMsg::RemoteRequest(r)] => {
                assert_eq!(i.stream_id, 9);
                let soorten: Vec<InputKind> = i.events.iter().map(|e| e.kind).collect();
                assert_eq!(
                    soorten,
                    vec![
                        InputKind::KEY_DOWN,
                        InputKind::BUTTON_DOWN,
                        InputKind::KEY_UP,
                        InputKind::BUTTON_UP
                    ]
                );
                assert!(!r.want);
            }
            andere => panic!("verwachtte invoer en dan stoppen: {andere:?}"),
        }
        assert!(b.toetsen.is_empty() && b.knoppen == 0);
    }

    #[test]
    fn veel_invoer_gaat_in_stukken_en_vergeten_stuurt_niets_meer() {
        let mut b = Bedien {
            stand: Bedienstand::Actief,
            ..Default::default()
        };
        for i in 0..(MAX_INVOER_PER_BERICHT as u8 + 5) {
            b.duw(InputEvent {
                delta: 120,
                x: u16::from(i),
                ..leeg(InputKind::WHEEL)
            });
        }
        let uit = b.werk(1);
        assert_eq!(uit.len(), 2);
        b.duw(beweeg(1));
        b.spoel();
        b.vergeet();
        assert!(
            b.werk(1).is_empty(),
            "na intrekken door de deler gaat er niets meer heen"
        );
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

        let mut venster =
            Venster::open(&d3d, "FitCom — test", b, h, Plaatsing::default()).expect("venster");
        let tot = Instant::now() + Duration::from_secs(5);
        while Instant::now() < tot && venster.pomp() {
            venster.toon(&tex).expect("tonen");
            std::thread::sleep(Duration::from_millis(16));
        }
    }

    /// De invoer van het kijkvenster zonder echte muis: berichten rechtstreeks in de
    /// wachtrij van het venster, dus de cursor van de gebruiker blijft waar hij is. Pen op
    /// met D, een sleep met de linkerknop, Delete; daarna moet er een lijn en een
    /// wisverzoek voor de deler klaarliggen, en een tekening met lijnen geeft een laag.
    #[test]
    #[ignore = "opent een echt venster"]
    fn slepen_met_de_pen_wordt_een_lijn_voor_de_deler() {
        use fitcom_proto::PeerId;
        use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

        let d3d = D3dContext::new().expect("D3D11");
        let mut venster = Venster::open(&d3d, "FitCom — tekentest", 640, 360, Plaatsing::default())
            .expect("venster");
        let hwnd = venster.hwnd;
        let post = |msg: u32, wparam: usize, x: i32, y: i32| {
            let lparam = LPARAM(((y as isize) << 16) | (x as isize & 0xFFFF));
            // SAFETY: eigen venster; de berichten komen in onze eigen wachtrij.
            unsafe { PostMessageW(Some(hwnd), msg, WPARAM(wparam), lparam) }.unwrap();
        };
        let pomp_even = |v: &mut Venster| {
            let tot = Instant::now() + Duration::from_millis(100);
            while Instant::now() < tot {
                v.pomp();
                std::thread::sleep(Duration::from_millis(5));
            }
        };

        // Zonder toestemming van de deler doet de pen niets.
        post(WM_KEYDOWN, VK_D.0 as usize, 0, 0);
        post(WM_LBUTTONDOWN, 1, 10, 10);
        post(WM_MOUSEMOVE, 1, 300, 180);
        post(WM_LBUTTONUP, 0, 300, 180);
        pomp_even(&mut venster);
        assert!(venster.werk(7).is_empty(), "tekenen staat uit");

        venster.zet_tekening(Arc::new(Tekening::nieuw(true)));
        post(WM_KEYDOWN, VK_D.0 as usize, 0, 0);
        post(WM_LBUTTONDOWN, 1, 0, 0);
        post(WM_MOUSEMOVE, 1, 1, 1); // te klein: geen punt
        post(WM_MOUSEMOVE, 1, 320, 180);
        post(WM_MOUSEMOVE, 1, 639, 359);
        post(WM_LBUTTONUP, 0, 639, 359);
        post(WM_KEYDOWN, VK_DELETE.0 as usize, 0, 0);
        pomp_even(&mut venster);

        let werk = venster.werk(7);
        match werk.as_slice() {
            [ControlMsg::Draw(d), ControlMsg::DrawClear(c)] => {
                assert_eq!((d.stream_id, d.author, d.start), (7, None, 0));
                assert_eq!(d.points.len(), 3, "{:?}", d.points);
                assert!(d.points[0][0] < 200 && d.points[0][1] < 200, "linksboven");
                assert!(
                    d.points[2][0] > 65000 && d.points[2][1] > 65000,
                    "rechtsonder"
                );
                assert_eq!(c.scope, DrawClearScope::MINE);
            }
            andere => panic!("verwachtte een lijn en een wisverzoek: {andere:?}"),
        }

        // Wat de deler terugstuurt komt op een laag boven het beeld.
        let Some(ControlMsg::Draw(d)) = werk.first().cloned() else {
            unreachable!()
        };
        let mut t = Tekening::nieuw(true);
        assert!(t.teken(PeerId::new_random(), &d));
        venster.zet_tekening(Arc::new(t));
        assert!(venster.teken().is_some_and(|t| t.laag.is_some()));
        pomp_even(&mut venster);
    }

    /// Het menu zonder echte muis: rechtsklikken opent het en het pakt de muis, slepen op de
    /// schuif levert een volume voor de motor, en een klik ernaast sluit het en laat de muis los.
    #[test]
    #[ignore = "opent een echt venster"]
    fn het_menu_opent_zet_het_volume_en_sluit_bij_een_klik_ernaast() {
        use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

        let d3d = D3dContext::new().expect("D3D11");
        let mut venster = Venster::open(&d3d, "FitCom — menutest", 640, 360, Plaatsing::default())
            .expect("venster");
        let hwnd = venster.hwnd;
        venster.zet_menu_stijl(MenuStijl {
            grond: [0, 0, 0],
            rand: [41, 45, 48],
            hover: [11, 14, 20],
            tekst: [161, 164, 165],
            sterk: [255, 255, 255],
            gedimd: [110, 114, 122],
            accent: [212, 215, 219],
            hoek: 16.0,
            hoek_rij: 6.0,
            font: include_bytes!("../../app/src/ui/assets/fonts/Inter-Medium.ttf"),
            font_naam: "Inter Medium",
        });
        venster.zet_volume(Some(0.5));
        let lp = |x: i32, y: i32| LPARAM(((y as isize) << 16) | (x as isize & 0xFFFF));
        let post = |msg: u32, wparam: usize, lparam: LPARAM| {
            // SAFETY: eigen venster; de berichten komen in onze eigen wachtrij.
            unsafe { PostMessageW(Some(hwnd), msg, WPARAM(wparam), lparam) }.unwrap();
        };
        let pomp_even = |v: &mut Venster| {
            let tot = Instant::now() + Duration::from_millis(100);
            while Instant::now() < tot {
                v.pomp();
                std::thread::sleep(Duration::from_millis(5));
            }
        };

        post(WM_RBUTTONUP, 0, lp(100, 100));
        pomp_even(&mut venster);
        // SAFETY: kale opzoekingen.
        let vast = unsafe { GetCapture() };
        assert!(!vast.is_invalid(), "het menu heeft de muis");
        assert_eq!(unsafe { GetWindow(vast, GW_OWNER) }.ok(), Some(hwnd));

        let menu = venster.staat().and_then(|s| s.menu.as_ref()).expect("open");
        let (x, y) = (0..800)
            .step_by(2)
            .flat_map(|y| (0..600).step_by(4).map(move |x| (x, y)))
            .find(|&(x, y)| menu.doel_op(x, y) == Some(Doel::Volume))
            .expect("een schuif");
        let x_rechts = (x..x + 600)
            .find(|&x| menu.volume_op(x) == Some(1.0))
            .unwrap();
        post(WM_KIJKMENU, WM_LBUTTONDOWN as usize, lp(x, y));
        post(WM_KIJKMENU, WM_MOUSEMOVE as usize, lp(x_rechts, y));
        post(WM_KIJKMENU, WM_LBUTTONUP as usize, lp(x_rechts, y));
        pomp_even(&mut venster);
        assert_eq!(venster.volume_keuze(), Some(1.0));
        assert!(
            venster.staat().is_some_and(|s| s.menu.is_some()),
            "de schuif sluit niet"
        );

        post(WM_KIJKMENU, WM_LBUTTONDOWN as usize, lp(-10, -10));
        pomp_even(&mut venster);
        assert!(venster.staat().is_some_and(|s| s.menu.is_none()));
        assert!(unsafe { GetCapture() }.is_invalid(), "de muis is los");
    }
}
