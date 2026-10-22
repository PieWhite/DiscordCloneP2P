//! Het menu onder de rechterknop in het kijkvenster: het geluid van de stream, tekenen en
//! bedienen. Een eigen layered venster, getekend met [`crate::penseel`] en GDI-tekst als
//! masker, net als de spraakoverlay, in de tokens van de app ([`MenuStijl`]).
//!
//! Niet modaal, anders dan het `TrackPopupMenu` dat het verving: het beeld loopt door terwijl
//! het open staat. Het venster hoort bij het kijkvenster (eigenaar), pakt nooit de focus en
//! houdt de muis vast (`SetCapture`), zodat een klik ernaast het sluit zoals bij een gewoon
//! menu; verliest het de muis (Alt+Tab, een ander programma), dan gaat het dicht. Het beslist
//! zelf niets: zijn vensterprocedure post elk muisbericht door naar het kijkvenster
//! ([`WM_KIJKMENU`]), dat de staat heeft. Posten en niet sturen, zodat de procedure van het
//! kijkvenster nooit in zichzelf terugkomt.

use crate::kijker::MenuStijl;
use crate::penseel::{Doek, Kader};
use crate::spraaklaag::{dpi_van, laad_font, maak_font, schaal};
use crate::tekenlaag::{registreer_klasse, Laag};
use anyhow::Result;
use fitcom_proto::bediening::Bedienstand;
use fitcom_proto::teken::{DIKTES, KLEUREN};
use std::sync::Once;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    DeleteObject, DrawTextW, GdiFlush, GetMonitorInfoW, MonitorFromPoint, SelectObject, SetBkMode,
    SetTextColor, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_RIGHT, DT_SINGLELINE, DT_VCENTER, HDC,
    HFONT, MONITORINFO, MONITOR_DEFAULTTONEAREST, TRANSPARENT,
};
use windows::Win32::UI::Input::KeyboardAndMouse::SetCapture;
use windows::Win32::UI::WindowsAndMessaging::{
    DefWindowProcW, GetWindow, GetWindowLongW, LoadCursorW, PostMessageW, SetCursor,
    SetWindowLongW, GWL_EXSTYLE, GW_OWNER, IDC_ARROW, MA_NOACTIVATE, WM_APP, WM_CAPTURECHANGED,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_RBUTTONDOWN,
    WS_EX_TRANSPARENT,
};

const KLASSE: PCWSTR = w!("FitcomKijkmenu");

/// Een muisbericht van het menu, gepost naar het kijkvenster: `wparam` is het oorspronkelijke
/// bericht, `lparam` de plek in clientpixels van het menu (met de muis vast ook daarbuiten,
/// dus negatief kan).
pub(crate) const WM_KIJKMENU: u32 = WM_APP + 1;

// Maten bij 96 dpi; geschaald met de dpi van de monitor waar het opengaat.
const BREEDTE: i32 = 264;
/// Tussen de rand van het menu en de rijen.
const BINNEN: i32 = 6;
const KOP: i32 = 28;
const RIJ: i32 = 32;
const SCHUIF: i32 = 24;
const CEL: i32 = 34;
/// Een haarlijn met lucht erboven en eronder.
const SCHEIDING: i32 = 9;
/// Tekst en schuif binnen een rij.
const TEKST_X: i32 = 10;
/// Ruimte voor een sneltoets of percentage rechts in een rij.
const RECHTS_B: i32 = 48;
const LETTER: i32 = 14;
const KLEIN: i32 = 12;
const STIP: i32 = 7;
const SCHAKELAAR: (i32, i32) = (28, 16);

/// Wat het menu laat zien, uit de staat van het kijkvenster.
pub(crate) struct Inhoud {
    pub kop: String,
    /// Het volume van het geluid bij dit beeld, of `None` als er geen is.
    pub volume: Option<f32>,
    /// Tekenen mag op dit scherm.
    pub tekenen: bool,
    pub pen: bool,
    pub kleur: usize,
    pub dikte: usize,
    pub bedien: Bedienstand,
}

/// Wat een klik in het menu raakt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Doel {
    Volume,
    Pen,
    Kleur(usize),
    Dikte(usize),
    Ongedaan,
    Wis,
    Bedien,
}

#[derive(Debug, Clone, PartialEq)]
enum Soort {
    Kop(String),
    /// Een gewone rij. `aan` maakt er een schakelaar van.
    Rij {
        tekst: &'static str,
        rechts: &'static str,
        aan: Option<bool>,
    },
    /// Tekst met rechts een waarde, zonder iets om te klikken.
    Label {
        tekst: &'static str,
        rechts: String,
    },
    /// Gedimd: het kan hier niet.
    Uit(&'static str),
    Schuif(f32),
    Kleur {
        i: usize,
        gekozen: bool,
    },
    Dikte {
        i: usize,
        gekozen: bool,
    },
    Lijn,
}

#[derive(Debug, Clone, PartialEq)]
struct Vak {
    kader: RECT,
    soort: Soort,
    doel: Option<Doel>,
}

/// De rijen van het menu op hun plek, in pixels bij `dpi`, vanaf de linkerbovenhoek.
#[derive(Debug)]
struct Opmaak {
    dpi: u32,
    vakken: Vec<Vak>,
    maat: (i32, i32),
}

impl Opmaak {
    fn nieuw(inh: &Inhoud, dpi: u32) -> Self {
        let s = |v: i32| schaal(v, dpi);
        let mut b = Bouw {
            vakken: Vec::new(),
            links: s(BINNEN),
            rechts: s(BREEDTE - BINNEN),
            y: s(BINNEN),
        };
        b.rij(s(KOP), Soort::Kop(inh.kop.clone()), None);
        match inh.volume {
            Some(v) => {
                b.rij(
                    s(RIJ),
                    Soort::Label {
                        tekst: "Stream audio",
                        rechts: format!("{}%", (v * 100.0).round() as u32),
                    },
                    None,
                );
                b.rij(s(SCHUIF), Soort::Schuif(v), Some(Doel::Volume));
            }
            None => b.rij(s(RIJ), Soort::Uit("No stream audio"), None),
        }
        b.rij(s(SCHEIDING), Soort::Lijn, None);
        if inh.tekenen {
            b.rij(
                s(RIJ),
                Soort::Rij {
                    tekst: "Draw on the stream",
                    rechts: "D",
                    aan: Some(inh.pen),
                },
                Some(Doel::Pen),
            );
            b.cellen(s(CEL), KLEUREN.len(), |i| {
                let gekozen = i == inh.kleur;
                (Soort::Kleur { i, gekozen }, Doel::Kleur(i))
            });
            b.cellen(s(CEL), DIKTES.len(), |i| {
                let gekozen = i == inh.dikte;
                (Soort::Dikte { i, gekozen }, Doel::Dikte(i))
            });
            for (tekst, rechts, doel) in [
                ("Undo my last line", "Ctrl+Z", Doel::Ongedaan),
                ("Clear my lines", "Del", Doel::Wis),
            ] {
                let soort = Soort::Rij {
                    tekst,
                    rechts,
                    aan: None,
                };
                b.rij(s(RIJ), soort, Some(doel));
            }
        } else {
            b.rij(s(RIJ), Soort::Uit("Drawing is off"), None);
        }
        b.rij(s(SCHEIDING), Soort::Lijn, None);
        let bedien = match inh.bedien {
            Bedienstand::Mag => Some("Take control of this screen"),
            Bedienstand::Gevraagd => Some("Cancel the control request"),
            Bedienstand::Actief => Some("Stop controlling"),
            Bedienstand::Uit => None,
        };
        match bedien {
            Some(tekst) => b.rij(
                s(RIJ),
                Soort::Rij {
                    tekst,
                    rechts: "",
                    aan: None,
                },
                Some(Doel::Bedien),
            ),
            None => b.rij(s(RIJ), Soort::Uit("Remote control is off"), None),
        }
        let hoogte = b.y + s(BINNEN);
        Self {
            dpi,
            vakken: b.vakken,
            maat: (s(BREEDTE), hoogte),
        }
    }

    fn s(&self, v: i32) -> i32 {
        schaal(v, self.dpi)
    }

    fn doel_op(&self, x: i32, y: i32) -> Option<Doel> {
        self.vakken
            .iter()
            .find(|v| binnen(v.kader, x, y))
            .and_then(|v| v.doel)
    }

    /// Waar de schuif loopt, van links naar rechts in pixels.
    fn baan(&self) -> Option<(f32, f32, f32)> {
        let v = self
            .vakken
            .iter()
            .find(|v| matches!(v.soort, Soort::Schuif(_)))?;
        let midden = (v.kader.top + v.kader.bottom) as f32 / 2.0;
        Some((
            (v.kader.left + self.s(TEKST_X)) as f32,
            (v.kader.right - self.s(TEKST_X)) as f32,
            midden,
        ))
    }

    /// Het volume bij muis-x `x`, in hele procenten.
    fn volume_op(&self, x: i32) -> Option<f32> {
        let (x0, x1, _) = self.baan()?;
        let v = ((x as f32 - x0) / (x1 - x0).max(1.0)).clamp(0.0, 1.0);
        Some((v * 100.0).round() / 100.0)
    }
}

struct Bouw {
    vakken: Vec<Vak>,
    links: i32,
    rechts: i32,
    y: i32,
}

impl Bouw {
    fn rij(&mut self, h: i32, soort: Soort, doel: Option<Doel>) {
        self.vakken.push(Vak {
            kader: RECT {
                left: self.links,
                top: self.y,
                right: self.rechts,
                bottom: self.y + h,
            },
            soort,
            doel,
        });
        self.y += h;
    }

    /// `n` cellen naast elkaar over de hele breedte.
    fn cellen(&mut self, h: i32, n: usize, cel: impl Fn(usize) -> (Soort, Doel)) {
        let b = self.rechts - self.links;
        for i in 0..n {
            let (soort, doel) = cel(i);
            self.vakken.push(Vak {
                kader: RECT {
                    left: self.links + b * i as i32 / n as i32,
                    top: self.y,
                    right: self.links + b * (i as i32 + 1) / n as i32,
                    bottom: self.y + h,
                },
                soort,
                doel: Some(doel),
            });
        }
        self.y += h;
    }
}

fn binnen(r: RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

fn kader_van(r: RECT) -> Kader {
    Kader {
        x0: r.left,
        y0: r.top,
        x1: r.right,
        y1: r.bottom,
    }
}

/// De linkerbovenhoek van een menu van `maat` dat bij `plek` opengaat: rechtsonder de muis,
/// en waar dat niet past ernaast gespiegeld, altijd helemaal binnen `werk`.
fn plaats(plek: POINT, maat: (i32, i32), werk: RECT) -> POINT {
    let as_ = |p: i32, m: i32, lo: i32, hi: i32| {
        let p = if p + m <= hi { p } else { p - m };
        p.min(hi - m).max(lo)
    };
    POINT {
        x: as_(plek.x, maat.0, werk.left, werk.right),
        y: as_(plek.y, maat.1, werk.top, werk.bottom),
    }
}

struct Fonts {
    groot: HFONT,
    klein: HFONT,
}

impl Fonts {
    fn nieuw(stijl: &MenuStijl, dpi: u32) -> Self {
        Self {
            groot: maak_font(stijl.font_naam, LETTER, dpi),
            klein: maak_font(stijl.font_naam, KLEIN, dpi),
        }
    }
}

impl Drop for Fonts {
    fn drop(&mut self) {
        // SAFETY: eigen objecten; `schilder` zet na gebruik de oude selectie terug.
        unsafe {
            let _ = DeleteObject(self.groot.into());
            let _ = DeleteObject(self.klein.into());
        }
    }
}

/// Eén stuk tekst: waar, wat, in welke maat en kleur.
struct Tekst {
    kader: RECT,
    tekst: Vec<u16>,
    klein: bool,
    rechts: bool,
    kleur: [u8; 3],
}

/// De teksten van alle vakken, in hun kleur: onder de muis sterk, gedimd wat niet kan.
fn teksten(o: &Opmaak, hover: Option<Doel>, st: &MenuStijl) -> Vec<Tekst> {
    let mut uit = Vec::new();
    let mut zet = |kader: RECT, tekst: &str, klein: bool, rechts: bool, kleur: [u8; 3]| {
        uit.push(Tekst {
            kader,
            tekst: tekst.encode_utf16().collect(),
            klein,
            rechts,
            kleur,
        })
    };
    for v in &o.vakken {
        let r = v.kader;
        let links = RECT {
            left: r.left + o.s(TEKST_X),
            right: r.right - o.s(TEKST_X),
            ..r
        };
        // Links en rechts mogen elkaar niet raken: elke tekst kleurt zijn eigen kader.
        let met_rechts = |extra: i32| {
            let rechts = RECT {
                left: links.right - extra - o.s(RECHTS_B),
                right: links.right - extra,
                ..r
            };
            (
                RECT {
                    right: rechts.left - o.s(4),
                    ..links
                },
                rechts,
            )
        };
        let onder = v.doel.is_some() && v.doel == hover;
        match &v.soort {
            Soort::Kop(t) => zet(links, t, true, false, st.gedimd),
            Soort::Uit(t) => zet(links, t, false, false, st.gedimd),
            Soort::Label { tekst, rechts } => {
                let (l, r) = met_rechts(0);
                zet(l, tekst, false, false, st.tekst);
                zet(r, rechts, true, true, st.gedimd);
            }
            Soort::Rij { tekst, rechts, aan } => {
                let kleur = if onder { st.sterk } else { st.tekst };
                if rechts.is_empty() {
                    zet(links, tekst, false, false, kleur);
                } else {
                    let schakelaar = aan.map_or(0, |_| o.s(SCHAKELAAR.0 + 8));
                    let (l, r) = met_rechts(schakelaar);
                    zet(l, tekst, false, false, kleur);
                    zet(r, rechts, true, true, st.gedimd);
                }
            }
            _ => {}
        }
    }
    uit
}

/// Het hele menu op `doek` (dat in `dc` geselecteerd is): eerst alle tekst wit op zwart als
/// masker, dan de grond en de vormen, dan de tekst in zijn eigen kleur erover.
fn schilder(doek: &mut Doek, dc: HDC, f: &Fonts, o: &Opmaak, hover: Option<Doel>, st: &MenuStijl) {
    let teksten = teksten(o, hover, st);
    doek.wis();
    // SAFETY: eigen DC met het beeld erin; lettertypes eigen, daarna de oude terug.
    unsafe {
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, COLORREF(0x00FF_FFFF));
        let oud = SelectObject(dc, f.groot.into());
        for t in &teksten {
            SelectObject(dc, if t.klein { f.klein } else { f.groot }.into());
            let mut r = t.kader;
            let kant = if t.rechts { DT_RIGHT } else { DT_LEFT };
            DrawTextW(
                dc,
                &mut t.tekst.clone(),
                &mut r,
                kant | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
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
    let s = |v: f32| v * o.dpi as f32 / 96.0;
    let heel = Kader {
        x0: 0,
        y0: 0,
        x1: o.maat.0,
        y1: o.maat.1,
    };
    let hoek = s(st.hoek);
    doek.afgerond(heel, hoek, st.rand, 1.0);
    let rand = o.s(1);
    let binnen = Kader {
        x0: rand,
        y0: rand,
        x1: o.maat.0 - rand,
        y1: o.maat.1 - rand,
    };
    doek.afgerond(binnen, hoek - rand as f32, st.grond, 1.0);

    for v in &o.vakken {
        let k = kader_van(v.kader);
        let onder = v.doel.is_some() && v.doel == hover;
        let midden = ((k.x0 + k.x1) as f32 / 2.0, (k.y0 + k.y1) as f32 / 2.0);
        match &v.soort {
            Soort::Rij { aan, .. } => {
                if onder {
                    doek.afgerond(k, s(st.hoek_rij), st.hover, 1.0);
                }
                if let Some(aan) = *aan {
                    let (b, h) = (o.s(SCHAKELAAR.0) as f32, o.s(SCHAKELAAR.1) as f32);
                    let x1 = (k.x1 - o.s(TEKST_X)) as f32;
                    let (r, y) = (h / 2.0, midden.1);
                    let (a, z) = ((x1 - b + r, y), (x1 - r, y));
                    let knop = s(STIP as f32 - 1.5);
                    if aan {
                        doek.capsule(a, z, r, st.accent, 1.0);
                        doek.capsule(z, z, knop, st.grond, 1.0);
                    } else {
                        doek.capsule(a, z, r, st.rand, 1.0);
                        doek.capsule(a, z, r - s(1.0), st.hover, 1.0);
                        doek.capsule(a, a, knop, st.gedimd, 1.0);
                    }
                }
            }
            Soort::Kleur { i, gekozen } => {
                if onder {
                    doek.afgerond(k, s(st.hoek_rij), st.hover, 1.0);
                }
                let r = s(STIP as f32);
                if *gekozen {
                    doek.capsule(midden, midden, r + s(3.5), st.sterk, 1.0);
                    doek.capsule(midden, midden, r + s(2.0), st.grond, 1.0);
                }
                doek.capsule(midden, midden, r, KLEUREN[*i].rgb, 1.0);
            }
            Soort::Dikte { i, gekozen } => {
                if onder || *gekozen {
                    doek.afgerond(k, s(st.hoek_rij), st.hover, 1.0);
                }
                // Even dik ten opzichte van elkaar als op het scherm, de dikste 4 px.
                let dikst = DIKTES[DIKTES.len() - 1].1;
                let r = s((DIKTES[*i].1 / dikst * 4.0).max(1.0));
                let half = s(11.0);
                let kleur = if *gekozen || onder {
                    st.sterk
                } else {
                    st.tekst
                };
                doek.capsule(
                    (midden.0 - half, midden.1),
                    (midden.0 + half, midden.1),
                    r,
                    kleur,
                    1.0,
                );
            }
            Soort::Schuif(waarde) => {
                if let Some((x0, x1, y)) = o.baan() {
                    let r = s(3.0);
                    let x = x0 + (x1 - x0) * waarde.clamp(0.0, 1.0);
                    doek.capsule((x0, y), (x1, y), r, st.rand, 1.0);
                    doek.capsule((x0, y), (x1, y), r - s(1.0), st.hover, 1.0);
                    if *waarde > 0.0 {
                        doek.capsule((x0, y), (x, y), r - s(1.0), st.accent, 1.0);
                    }
                    doek.capsule((x, y), (x, y), s(6.0), st.sterk, 1.0);
                }
            }
            Soort::Lijn => {
                doek.capsule(
                    (k.x0 as f32, midden.1),
                    (k.x1 as f32, midden.1),
                    s(0.5),
                    st.rand,
                    1.0,
                );
            }
            Soort::Kop(_) | Soort::Label { .. } | Soort::Uit(_) => {}
        }
    }

    for t in &teksten {
        doek.bedek(&masker, kader_van(t.kader), t.kleur);
    }
}

/// Het open menu. Weg met deze waarde: venster dicht, en de muis los.
pub(crate) struct Kijkmenu {
    laag: Laag,
    stijl: MenuStijl,
    fonts: Fonts,
    hoek: POINT,
    o: Opmaak,
    hover: Option<Doel>,
    /// De linkerknop ging omlaag op de schuif en is nog niet los.
    pub(crate) sleept: bool,
}

impl Kijkmenu {
    /// Opent het menu bij `plek` (schermpixels) boven `eigenaar`, en pakt de muis.
    pub(crate) fn open(
        eigenaar: HWND,
        plek: POINT,
        stijl: MenuStijl,
        inhoud: &Inhoud,
    ) -> Result<Self> {
        static EEN_KEER: Once = Once::new();
        registreer_klasse(KLASSE, menu_proc, &EEN_KEER)?;
        let laag = Laag::maak(KLASSE, Some(eigenaar))?;
        // SAFETY: eigen venster, net gemaakt en nog verborgen; alleen de doorklik-stijl gaat eraf.
        // Hij wordt gelezen als het venster verschijnt, dus geen FRAMECHANGED nodig.
        unsafe {
            let ex = GetWindowLongW(laag.hwnd(), GWL_EXSTYLE) as u32;
            SetWindowLongW(laag.hwnd(), GWL_EXSTYLE, (ex & !WS_EX_TRANSPARENT.0) as i32);
        }
        laad_font(stijl.font);
        // SAFETY: kale opzoeking, geen eigendom.
        let monitor = unsafe { MonitorFromPoint(plek, MONITOR_DEFAULTTONEAREST) };
        let dpi = dpi_van(monitor);
        let o = Opmaak::nieuw(inhoud, dpi);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        // SAFETY: `cbSize` is gezet zoals de API vraagt.
        let werk = if unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
            info.rcWork
        } else {
            RECT {
                left: plek.x,
                top: plek.y,
                right: plek.x + o.maat.0,
                bottom: plek.y + o.maat.1,
            }
        };
        let mut menu = Self {
            laag,
            fonts: Fonts::nieuw(&stijl, dpi),
            stijl,
            hoek: plaats(plek, o.maat, werk),
            o,
            hover: None,
            sleept: false,
        };
        menu.teken();
        // SAFETY: eigen venster van deze thread, die net de rechterknop losliet: hij is
        // vooraan, dus de muis komt hierheen tot het menu dichtgaat.
        unsafe {
            SetCapture(menu.laag.hwnd());
            let _ = SetCursor(LoadCursorW(None, IDC_ARROW).ok());
        }
        Ok(menu)
    }

    /// Nieuwe inhoud (een ander volume): opnieuw tekenen op dezelfde plek.
    pub(crate) fn toon(&mut self, inhoud: &Inhoud) {
        self.o = Opmaak::nieuw(inhoud, self.o.dpi);
        self.teken();
    }

    fn teken(&mut self) {
        let kader = RECT {
            left: self.hoek.x,
            top: self.hoek.y,
            right: self.hoek.x + self.o.maat.0,
            bottom: self.hoek.y + self.o.maat.1,
        };
        let Self {
            laag,
            stijl,
            fonts,
            o,
            hover,
            ..
        } = self;
        let hover = *hover;
        laag.toon_beeld(kader, |doek, dc| schilder(doek, dc, fonts, o, hover, stijl));
    }

    /// Ligt `(x, y)` (clientpixels van het menu) op het menu?
    pub(crate) fn raakt(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && x < self.o.maat.0 && y < self.o.maat.1
    }

    pub(crate) fn doel_op(&self, x: i32, y: i32) -> Option<Doel> {
        self.o.doel_op(x, y)
    }

    pub(crate) fn volume_op(&self, x: i32) -> Option<f32> {
        self.o.volume_op(x)
    }

    /// De rij onder de muis; tekent alleen opnieuw als die verandert.
    pub(crate) fn zet_hover(&mut self, hover: Option<Doel>) {
        if self.hover != hover {
            self.hover = hover;
            self.teken();
        }
    }
}

/// Muis door naar het kijkvenster; nooit de focus pakken.
unsafe extern "system" fn menu_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP | WM_RBUTTONDOWN | WM_MBUTTONDOWN
        | WM_CAPTURECHANGED => {
            // SAFETY: de eigenaar is het kijkvenster van deze thread; posten wacht nergens op.
            unsafe {
                if let Ok(eigenaar) = GetWindow(hwnd, GW_OWNER) {
                    let _ = PostMessageW(Some(eigenaar), WM_KIJKMENU, WPARAM(msg as usize), lparam);
                }
            }
            LRESULT(0)
        }
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        // SAFETY: doorgeven aan de standaardprocedure met wat Windows ons gaf.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inhoud(volume: Option<f32>, tekenen: bool, bedien: Bedienstand) -> Inhoud {
        Inhoud {
            kop: "Sander \u{2014} Display 1".into(),
            volume,
            tekenen,
            pen: false,
            kleur: 2,
            dikte: 1,
            bedien,
        }
    }

    /// Elke knop is te raken waar hij getekend wordt, de schuif rekent van zijn linker- naar
    /// zijn rechteruiteinde, en wat uit staat raakt niets.
    #[test]
    fn de_rijen_liggen_waar_ze_te_raken_zijn() {
        let o = Opmaak::nieuw(&inhoud(Some(0.5), true, Bedienstand::Mag), 96);
        let midden = |d: Doel| {
            let v = o
                .vakken
                .iter()
                .find(|v| v.doel == Some(d))
                .expect("aanwezig");
            (
                (v.kader.left + v.kader.right) / 2,
                (v.kader.top + v.kader.bottom) / 2,
            )
        };
        for d in [
            Doel::Volume,
            Doel::Pen,
            Doel::Kleur(0),
            Doel::Kleur(KLEUREN.len() - 1),
            Doel::Dikte(1),
            Doel::Ongedaan,
            Doel::Wis,
            Doel::Bedien,
        ] {
            let (x, y) = midden(d);
            assert_eq!(o.doel_op(x, y), Some(d));
        }
        assert_eq!(o.doel_op(BREEDTE / 2, BINNEN + 2), None, "de kop");
        let (x0, x1, _) = o.baan().unwrap();
        assert_eq!(o.volume_op(x0 as i32 - 50), Some(0.0));
        assert_eq!(o.volume_op(x1 as i32 + 50), Some(1.0));
        assert_eq!(o.volume_op(((x0 + x1) / 2.0) as i32), Some(0.5));

        let uit = Opmaak::nieuw(&inhoud(None, false, Bedienstand::Uit), 96);
        assert!(uit.vakken.iter().all(|v| v.doel.is_none()));
        assert!(uit.volume_op(10).is_none());
        assert!(uit.maat.1 < o.maat.1);
    }

    /// Bij 150 % schaalt alles mee.
    #[test]
    fn het_menu_schaalt_met_de_dpi() {
        let i = inhoud(Some(1.0), true, Bedienstand::Mag);
        let (a, b) = (Opmaak::nieuw(&i, 96), Opmaak::nieuw(&i, 144));
        assert_eq!(b.maat.0, BREEDTE * 3 / 2);
        assert!((b.maat.1 - a.maat.1 * 3 / 2).abs() <= 2);
    }

    #[test]
    fn het_menu_blijft_op_het_scherm() {
        let werk = RECT {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1040,
        };
        let p = |x, y| POINT { x, y };
        let maat = (264, 400);
        let at = |q: POINT| (q.x, q.y);
        assert_eq!(at(plaats(p(100, 100), maat, werk)), (100, 100));
        // Rechtsonder: gespiegeld naar links en omhoog.
        assert_eq!(at(plaats(p(1900, 1000), maat, werk)), (1636, 600));
        // Te hoog voor beide kanten: tegen de bovenrand, helemaal zichtbaar.
        assert_eq!(at(plaats(p(10, 300), (264, 900), werk)), (10, 0));
    }

    /// Zonder venster, in een eigen geheugen-DC: de grond is dekkend met ronde hoeken, de
    /// gekozen kleur staat er, en de rij onder de muis licht op.
    #[test]
    fn het_menu_wordt_getekend_zonder_venster() {
        use windows::Win32::Graphics::Gdi::{
            CreateCompatibleDC, CreateDIBSection, DeleteDC, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
            DIB_RGB_COLORS,
        };
        let stijl = MenuStijl {
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
        };
        laad_font(stijl.font);
        let f = Fonts::nieuw(&stijl, 96);
        let o = Opmaak::nieuw(&inhoud(Some(0.5), true, Bedienstand::Mag), 96);
        let (b, h) = (o.maat.0 as u32, o.maat.1 as u32);
        let pen = o
            .vakken
            .iter()
            .find(|v| v.doel == Some(Doel::Pen))
            .unwrap()
            .kader;
        // SAFETY: eigen DC en bitmap, opgeruimd voor we terugkeren.
        let px = unsafe {
            let dc = CreateCompatibleDC(None);
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
            schilder(&mut doek, dc, &f, &o, Some(Doel::Pen), &stijl);
            let px = doek.px.to_vec();
            SelectObject(dc, oud);
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(dc);
            px
        };
        let at = |x: i32, y: i32| px[((y as u32 * b + x as u32) * 4) as usize..][..4].to_vec();
        assert_eq!(at(0, 0)[3], 0, "ronde hoek");
        assert_eq!(
            at(o.maat.0 / 2, 0)[..3],
            [48, 45, 41],
            "haarlijn bovenaan (BGR)"
        );
        // Links in de rij onder de muis, voor de tekst: de hover-grond.
        assert_eq!(at(pen.left + 3, pen.top + 3), vec![20, 14, 11, 255]);
        // De gekozen kleur (Green, index 2) in het midden van zijn cel.
        let k = o
            .vakken
            .iter()
            .find(|v| v.doel == Some(Doel::Kleur(2)))
            .unwrap()
            .kader;
        let [r, g, bl] = KLEUREN[2].rgb;
        assert_eq!(
            at((k.left + k.right) / 2, (k.top + k.bottom) / 2),
            vec![bl, g, r, 255]
        );
        // Witte tekst in de rij onder de muis.
        assert!((pen.left..pen.right)
            .flat_map(|x| (pen.top..pen.bottom).map(move |y| (x, y)))
            .any(|(x, y)| at(x, y)[..3] == [255, 255, 255]));
        assert!(px
            .chunks_exact(4)
            .all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]));
    }
}
