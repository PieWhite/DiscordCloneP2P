//! Schermopname via Windows.Graphics.Capture.
//!
//! Beelden blijven op de GPU: de capture-API levert een D3D11-textuur, die we naar
//! onze eigen textuur kopiëren en rechtstreeks aan de encoder geven. Er komt geen
//! enkel frame in het werkgeheugen langs, en dat is de reden dat dit naast een
//! draaiende game kan zonder dat je het merkt.
//!
//! # Waarom `CreateFreeThreaded`
//!
//! De gewone `Create` verlangt een DispatcherQueue op de aanroepende thread — een
//! WinRT-constructie die bij een gewone Rust-thread niet bestaat. De vrije variant
//! levert frames op zijn eigen thread af en past daarmee bij hoe de rest van de
//! media-code is opgezet.

use crate::d3d::D3dContext;
use anyhow::{bail, Context, Result};
use crossbeam_channel::{bounded, Receiver};
use std::time::{Duration, Instant};
use windows::core::{Interface, BOOL};
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{
    Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::Graphics::Direct3D11::{ID3D11Texture2D, D3D11_TEXTURE2D_DESC};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_R16G16B16A16_FLOAT;
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, MonitorFromWindow, HDC, HMONITOR, MONITORINFOEXW,
    MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::WinRT::Direct3D11::IDirect3DDxgiInterfaceAccess;
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW, IsWindowVisible,
    GWL_EXSTYLE, WS_EX_TOOLWINDOW,
};

/// Twee buffers is genoeg: we halen elk frame meteen op en kopiëren het weg. Meer
/// buffers betekent alleen dat er oudere beelden blijven liggen.
const POOL_BUFFERS: i32 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BronSoort {
    Monitor,
    Venster,
    /// Een webcam. Zit niet in Windows.Graphics.Capture maar in Media Foundation, dus
    /// het opnemen loopt via [`crate::camera`]; naar buiten is het dezelfde `Bron` en
    /// hetzelfde `Opgenomen`, zodat `deler.rs` het verschil niet kent.
    Camera,
}

/// Een opgenomen beeld met de tijd waarop het gemaakt is.
///
/// Die tijd is `SystemRelativeTime` van de opname-API: wanneer het beeld op het scherm
/// stond, niet wanneer onze lus eraan toekwam. Dat verschil is het hele punt. Zet je er
/// je eigen "nu" op, dan draagt de tijdstempel de planning van onze thread in plaats van
/// de timing van het origineel, en dan kan de kijker nooit meer reconstrueren hoe het
/// bewoog. Zie [`crate::kijker`], waar hij ermee gepresenteerd wordt.
#[derive(Clone)]
pub struct Opgenomen {
    pub textuur: ID3D11Texture2D,
    /// 100-nanoseconden-eenheden op de klok van de opname-API. Alleen verschillen
    /// hiertussen zeggen iets; het nulpunt is willekeurig.
    pub opgenomen_hns: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bron {
    pub naam: String,
    pub soort: BronSoort,
    /// `HMONITOR` of `HWND`, afhankelijk van de soort.
    pub handle: isize,
}

/// Of dit Windows schermopname überhaupt toestaat. Op Windows 11 altijd; we vragen het
/// omdat een nette melding beter is dan een onbegrijpelijke fout verderop.
pub fn ondersteund() -> bool {
    GraphicsCaptureSession::IsSupported().unwrap_or(false)
}

pub fn beschikbare_bronnen() -> Result<Vec<Bron>> {
    let mut uit = monitoren()?;
    uit.extend(vensters()?);
    // Een kapotte of bezette camera mag het delen van een scherm niet in de weg staan:
    // loggen en verder, zoals overal waar hardware ontbreekt.
    match crate::camera::cameras() {
        Ok(c) => uit.extend(c),
        Err(e) => tracing::warn!(error = %format!("{e:#}"), "camera's niet op te sommen"),
    }
    Ok(uit)
}

fn monitoren() -> Result<Vec<Bron>> {
    let mut gevonden: Vec<Bron> = Vec::new();

    unsafe extern "system" fn per_monitor(
        mon: HMONITOR,
        _hdc: HDC,
        _rect: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        // SAFETY: `data` is de `&mut Vec<Bron>` die we hieronder meegeven, en blijft
        // geldig voor de duur van de enumeratie.
        let lijst = unsafe { &mut *(data.0 as *mut Vec<Bron>) };

        let mut info = MONITORINFOEXW {
            monitorInfo: windows::Win32::Graphics::Gdi::MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFOEXW>() as u32,
                ..Default::default()
            },
            ..Default::default()
        };
        // SAFETY: `info.cbSize` is correct gezet, zoals de API vereist.
        let ok = unsafe { GetMonitorInfoW(mon, &mut info.monitorInfo) }.as_bool();

        let breedte = info.monitorInfo.rcMonitor.right - info.monitorInfo.rcMonitor.left;
        let hoogte = info.monitorInfo.rcMonitor.bottom - info.monitorInfo.rcMonitor.top;
        let naam = if ok {
            format!("Scherm {}×{}", breedte, hoogte)
        } else {
            "Scherm".to_string()
        };

        // Twee identieke schermen krijgen twee identieke namen — en de clipkiezer
        // adresseert schermen op naam. Nummer ze dus bij collisions, net als
        // bestanden in een map: "#2", "#3".
        let naam = if info.monitorInfo.dwFlags & 1 != 0 {
            format!("{naam} (hoofdscherm)")
        } else {
            naam
        };
        let al_geweest = lijst
            .iter()
            .filter(|b| b.soort == BronSoort::Monitor)
            .filter(|b| b.naam == naam)
            .count();
        let naam = if al_geweest > 0 {
            format!("{naam} #{}", al_geweest + 1)
        } else {
            naam
        };

        lijst.push(Bron {
            naam,
            soort: BronSoort::Monitor,
            handle: mon.0 as isize,
        });
        BOOL(1)
    }

    // SAFETY: de callback hierboven verwacht precies dit type achter `LPARAM`.
    unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(per_monitor),
            LPARAM(&mut gevonden as *mut _ as isize),
        )
        .ok()
        .context("schermen opsommen")?;
    }
    // Het hoofdscherm vooraan. Windows somt ze op in zijn eigen volgorde, en "het eerste
    // scherm" is de standaard voor clips en het bovenste in de deelkiezer: op een pc met
    // twee schermen nam de clipopname zo het verkeerde op tot je hem elke keer koos.
    // Stabiel sorteren, dus de rest houdt zijn volgorde (en zijn "#2"-nummer).
    gevonden.sort_by_key(|b| !b.naam.ends_with("(hoofdscherm)"));
    Ok(gevonden)
}

fn vensters() -> Result<Vec<Bron>> {
    let mut gevonden: Vec<Bron> = Vec::new();

    unsafe extern "system" fn per_venster(hwnd: HWND, data: LPARAM) -> BOOL {
        // SAFETY: zie `per_monitor`.
        let lijst = unsafe { &mut *(data.0 as *mut Vec<Bron>) };

        // SAFETY: `hwnd` komt van de enumeratie en is dus geldig.
        unsafe {
            if !IsWindowVisible(hwnd).as_bool() {
                return BOOL(1);
            }
            // Werkbalkvensters en dergelijke wil niemand delen.
            if GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 != 0 {
                return BOOL(1);
            }
            let lengte = GetWindowTextLengthW(hwnd);
            if lengte <= 0 {
                return BOOL(1);
            }
            let mut buf = vec![0u16; lengte as usize + 1];
            let n = GetWindowTextW(hwnd, &mut buf);
            if n <= 0 {
                return BOOL(1);
            }
            lijst.push(Bron {
                naam: String::from_utf16_lossy(&buf[..n as usize]),
                soort: BronSoort::Venster,
                handle: hwnd.0 as isize,
            });
        }
        BOOL(1)
    }

    // SAFETY: zie `monitoren`.
    unsafe {
        EnumWindows(Some(per_venster), LPARAM(&mut gevonden as *mut _ as isize))
            .context("vensters opsommen")?;
    }
    Ok(gevonden)
}

/// Afmeting van een bron zonder er opname op te starten.
///
/// Nodig omdat we een gedeelde bron aankondigen — mét afmeting — voordat er iemand
/// naar kijkt, en tot dat moment mag er geen enkel frame opgenomen worden.
pub fn afmeting_van(bron: &Bron) -> Result<(u32, u32)> {
    // Een camera meldt zijn afmeting pas als je hem opent, en dat zou het lampje
    // aanzetten voordat er iemand kijkt — precies wat de afspraak "er wordt niets
    // opgenomen tot iemand kijkt" verbiedt. Dus een nominale maat in de aankondiging;
    // het eerste echte beeld overschrijft hem aan beide kanten van de draad, want zowel
    // het kijkvenster als de encoder gaan uit van wat er werkelijk binnenkomt.
    if bron.soort == BronSoort::Camera {
        return Ok(crate::camera::NOMINALE_AFMETING);
    }

    let interop: IGraphicsCaptureItemInterop =
        windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
            .context("capture-interop ophalen")?;
    // SAFETY: de handle hoort bij de opgegeven soort; beide komen uit onze enumeratie.
    let item: GraphicsCaptureItem = unsafe {
        match bron.soort {
            BronSoort::Monitor => interop
                .CreateForMonitor(HMONITOR(bron.handle as *mut _))
                .context("scherm openen")?,
            BronSoort::Venster => interop
                .CreateForWindow(HWND(bron.handle as *mut _))
                .context("venster openen")?,
            BronSoort::Camera => unreachable!("camera is hierboven al afgehandeld"),
        }
    };
    let grootte = item.Size().context("afmetingen opvragen")?;
    Ok((grootte.Width.max(1) as u32, grootte.Height.max(1) as u32))
}

/// Een lopende opname, van een scherm of van een camera.
///
/// Twee heel verschillende Windows-API's achter één vorm, zodat `deler.rs` niet weet
/// waar het beeld vandaan komt en er dus geen tweede deel-pad hoeft te bestaan.
pub enum Capture {
    Scherm(Schermcapture),
    Camera(crate::camera::Cameracapture),
}

impl Capture {
    pub fn start(d3d: &D3dContext, bron: &Bron) -> Result<Self> {
        match bron.soort {
            BronSoort::Camera => Ok(Self::Camera(crate::camera::Cameracapture::start(
                d3d, bron,
            )?)),
            _ => Ok(Self::Scherm(Schermcapture::start(d3d, bron)?)),
        }
    }

    pub fn afmeting(&self) -> (u32, u32) {
        match self {
            Self::Scherm(c) => c.afmeting(),
            Self::Camera(c) => c.afmeting(),
        }
    }

    /// Wacht op het volgende beeld. `None` betekent dat er binnen de tijd niets kwam;
    /// bij een scherm dat niet verandert is dat normaal.
    pub fn volgende_frame(&mut self, timeout: std::time::Duration) -> Option<Opgenomen> {
        match self {
            Self::Scherm(c) => c.volgende_frame(timeout),
            Self::Camera(c) => c.volgende_frame(timeout),
        }
    }
}

pub struct Schermcapture {
    d3d: D3dContext,
    _item: GraphicsCaptureItem,
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    /// Onze eigen textuur; de capture-API hergebruikt de zijne zodra we het frame sluiten.
    doel: ID3D11Texture2D,
    afmeting: (u32, u32),
    signaal: Receiver<()>,
    /// Waar het scherm staat dat we opnemen; voor een venster wordt dat elke controle
    /// opnieuw opgezocht, want het kan verhuizen.
    bron: Bron,
    /// Of het scherm bij de laatste controle in HDR stond (zie [`crate::hdr`]).
    in_hdr: bool,
    /// Alleen in HDR. `None` is de gewone BGRA-weg van vóór beslissing 52, met een kale
    /// `CopyResource`.
    hdr: Option<crate::hdr::Hdrnaarsdr>,
    volgende_controle: Instant,
}

/// Hoe vaak we kijken of HDR aan- of uitging. Een spel kan het scherm omzetten terwijl
/// er al gedeeld wordt, en de clipopname loopt de hele sessie.
const HDR_CONTROLE: Duration = Duration::from_secs(1);

/// Het scherm waar deze bron op staat.
fn monitor_van(bron: &Bron) -> HMONITOR {
    match bron.soort {
        BronSoort::Venster => {
            // SAFETY: een `HWND` uit onze eigen enumeratie; bestaat hij niet meer, dan
            // levert dit gewoon het dichtstbijzijnde scherm.
            unsafe { MonitorFromWindow(HWND(bron.handle as *mut _), MONITOR_DEFAULTTONEAREST) }
        }
        _ => HMONITOR(bron.handle as *mut _),
    }
}

/// De omzetter als het scherm in HDR staat, anders `None` voor de gewone weg. Lukt het
/// opzetten niet, dan liever een afgeknipt beeld dan geen beeld.
fn hdr_omzetter(d3d: &D3dContext, in_hdr: bool) -> Option<crate::hdr::Hdrnaarsdr> {
    if !in_hdr {
        return None;
    }
    match crate::hdr::Hdrnaarsdr::new(d3d) {
        Ok(h) => {
            tracing::info!("scherm staat in HDR, opname wordt omgezet naar SDR");
            Some(h)
        }
        Err(e) => {
            tracing::warn!(error = %format!("{e:#}"), "HDR-omzetting niet op te zetten");
            None
        }
    }
}

fn pixelformaat(hdr: &Option<crate::hdr::Hdrnaarsdr>) -> DirectXPixelFormat {
    if hdr.is_some() {
        DirectXPixelFormat::R16G16B16A16Float
    } else {
        DirectXPixelFormat::B8G8R8A8UIntNormalized
    }
}

// SAFETY: alle WinRT-objecten hierin zijn agile (free-threaded) en het D3D11-apparaat
// staat op multithread-protected. De struct wordt bovendien maar door één thread
// tegelijk gebruikt: de capture-thread.
unsafe impl Send for Schermcapture {}

impl Schermcapture {
    fn start(d3d: &D3dContext, bron: &Bron) -> Result<Self> {
        Self::open(d3d, bron, crate::hdr::staat_in_hdr(monitor_van(bron)))
    }

    /// `in_hdr` los, zodat de test de HDR-weg ook op een SDR-scherm kan nemen.
    fn open(d3d: &D3dContext, bron: &Bron, in_hdr: bool) -> Result<Self> {
        if !ondersteund() {
            bail!("dit Windows staat schermopname niet toe");
        }

        let interop: IGraphicsCaptureItemInterop =
            windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
                .context("capture-interop ophalen")?;

        // SAFETY: de handle hoort bij de opgegeven soort; beide komen uit onze eigen
        // enumeratie hierboven.
        let item: GraphicsCaptureItem = unsafe {
            match bron.soort {
                BronSoort::Monitor => interop
                    .CreateForMonitor(HMONITOR(bron.handle as *mut _))
                    .context("scherm openen voor opname")?,
                BronSoort::Venster => interop
                    .CreateForWindow(HWND(bron.handle as *mut _))
                    .context("venster openen voor opname")?,
                BronSoort::Camera => bail!("een camera loopt niet via schermopname"),
            }
        };

        let grootte = item.Size().context("afmetingen opvragen")?;
        let afmeting = (grootte.Width.max(1) as u32, grootte.Height.max(1) as u32);

        let hdr = hdr_omzetter(d3d, in_hdr);
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &d3d.winrt_device,
            pixelformaat(&hdr),
            POOL_BUFFERS,
            grootte,
        )
        .context("framepool aanmaken")?;

        let (tx, signaal) = bounded(1);
        pool.FrameArrived(&TypedEventHandler::new(move |_, _| {
            // Vol betekent dat we het vorige signaal nog niet verwerkt hebben; dan is
            // er niets bij te vertellen.
            let _ = tx.try_send(());
            Ok(())
        }))
        .context("frame-gebeurtenis koppelen")?;

        let session = pool.CreateCaptureSession(&item).context("opnamesessie")?;
        session
            .SetIsCursorCaptureEnabled(true)
            .context("cursor aanzetten")?;

        // De gele opnamerand kan sinds Windows 11 uit. Lukt het niet, dan is dat
        // cosmetisch en geen reden om niet te delen.
        if let Err(e) = session.SetIsBorderRequired(false) {
            tracing::debug!(error = %e, "opnamerand blijft zichtbaar");
        }

        session.StartCapture().context("opname starten")?;
        let doel = d3d.maak_textuur(afmeting.0, afmeting.1)?;

        tracing::info!(bron = %bron.naam, breedte = afmeting.0, hoogte = afmeting.1, "opname gestart");

        Ok(Self {
            d3d: d3d.clone(),
            _item: item,
            pool,
            session,
            doel,
            afmeting,
            signaal,
            bron: bron.clone(),
            in_hdr,
            hdr,
            volgende_controle: Instant::now() + HDR_CONTROLE,
        })
    }

    fn afmeting(&self) -> (u32, u32) {
        self.afmeting
    }

    /// Wacht op het volgende beeld en levert onze eigen textuur op, met de tijd waarop
    /// de opname hem maakte.
    ///
    /// `None` betekent dat er binnen de tijd niets kwam. Dat is normaal: een venster
    /// dat niet verandert levert geen frames, en dan is er ook niets te versturen.
    fn volgende_frame(&mut self, timeout: std::time::Duration) -> Option<Opgenomen> {
        self.signaal.recv_timeout(timeout).ok()?;

        if Instant::now() >= self.volgende_controle {
            self.volgende_controle = Instant::now() + HDR_CONTROLE;
            let in_hdr = crate::hdr::staat_in_hdr(monitor_van(&self.bron));
            if in_hdr != self.in_hdr {
                tracing::info!(in_hdr, "HDR van het scherm omgezet");
                self.in_hdr = in_hdr;
                self.hdr = hdr_omzetter(&self.d3d, in_hdr);
                let (b, h) = self.afmeting;
                let grootte = SizeInt32 {
                    Width: b as i32,
                    Height: h as i32,
                };
                if let Err(e) = self.herbouw(self.afmeting, grootte) {
                    tracing::error!(error = %format!("{e:#}"), "opname niet kunnen aanpassen");
                }
                return None;
            }
        }

        let frame = self.pool.TryGetNextFrame().ok()?;
        let opgenomen_hns = frame.SystemRelativeTime().ok()?.Duration;
        let grootte = frame.ContentSize().ok()?;
        let nieuw = (grootte.Width.max(1) as u32, grootte.Height.max(1) as u32);

        if nieuw != self.afmeting {
            // Resolutie gewijzigd of venster geschaald. Alles opnieuw opzetten; een
            // frame overslaan is niet erg, doorgaan met de verkeerde maat wel.
            tracing::info!(van = ?self.afmeting, naar = ?nieuw, "bron van afmeting veranderd");
            if let Err(e) = self.herbouw(nieuw, grootte) {
                tracing::error!(error = %format!("{e:#}"), "opname niet kunnen aanpassen");
            }
            let _ = frame.Close();
            return None;
        }

        let surface = frame.Surface().ok()?;
        let toegang: IDirect3DDxgiInterfaceAccess = surface.cast().ok()?;
        // SAFETY: het oppervlak van een capture-frame is altijd een ID3D11Texture2D.
        let bron: ID3D11Texture2D = unsafe { toegang.GetInterface().ok()? };

        // Overzetten naar onze eigen textuur: zodra we het frame sluiten mag de
        // capture-API zijn buffer hergebruiken voor het volgende beeld.
        if let Some(hdr) = &mut self.hdr {
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            // SAFETY: `GetDesc` vult alleen de meegegeven struct.
            unsafe { bron.GetDesc(&mut desc) };
            // Een beeld van vóór het omzetten van de pool heeft nog het oude formaat en
            // wordt overgeslagen.
            let gelukt = desc.Format == DXGI_FORMAT_R16G16B16A16_FLOAT
                && hdr
                    .zet_om(&bron, &self.doel)
                    .inspect_err(|e| tracing::warn!(error = %format!("{e:#}"), "HDR-omzetting"))
                    .is_ok();
            if !gelukt {
                let _ = frame.Close();
                return None;
            }
        } else {
            // SAFETY: beide texturen bestaan, hebben hetzelfde formaat en dezelfde maat.
            unsafe { self.d3d.context.CopyResource(&self.doel, &bron) };
        }
        let _ = frame.Close();

        Some(Opgenomen {
            textuur: self.doel.clone(),
            opgenomen_hns,
        })
    }

    fn herbouw(&mut self, nieuw: (u32, u32), grootte: SizeInt32) -> Result<()> {
        self.pool
            .Recreate(
                &self.d3d.winrt_device,
                pixelformaat(&self.hdr),
                POOL_BUFFERS,
                grootte,
            )
            .context("framepool opnieuw opzetten")?;
        self.doel = self.d3d.maak_textuur(nieuw.0, nieuw.1)?;
        self.afmeting = nieuw;
        Ok(())
    }
}

impl Drop for Schermcapture {
    fn drop(&mut self) {
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    #[ignore = "vereist een echt scherm"]
    fn bronnen_zijn_op_te_sommen() {
        let bronnen = beschikbare_bronnen().expect("bronnen");
        let schermen = bronnen
            .iter()
            .filter(|b| b.soort == BronSoort::Monitor)
            .count();
        println!("{} bronnen, waarvan {schermen} schermen", bronnen.len());
        for b in bronnen.iter().take(15) {
            println!("  {:?} {}", b.soort, b.naam);
        }
        assert!(schermen >= 1, "er moet minstens één scherm zijn");
    }

    #[test]
    #[ignore = "vereist een echt scherm"]
    fn scherm_levert_beelden() {
        let d3d = D3dContext::new().expect("D3D11");
        let scherm = beschikbare_bronnen()
            .expect("bronnen")
            .into_iter()
            .find(|b| b.soort == BronSoort::Monitor)
            .expect("scherm");

        let mut cap = Capture::start(&d3d, &scherm).expect("opname starten");
        let (b, h) = cap.afmeting();
        println!("opname {b}×{h}");
        assert!(b >= 640 && h >= 480, "onwaarschijnlijke afmeting");

        // Een stilstaand bureaublad levert weinig frames; een paar seconden geduld.
        let mut gezien = 0;
        for _ in 0..30 {
            if cap.volgende_frame(Duration::from_millis(200)).is_some() {
                gezien += 1;
            }
            if gezien >= 2 {
                break;
            }
        }
        assert!(gezien >= 1, "er kwam geen enkel beeld binnen");
        println!("{gezien} frames ontvangen");
    }

    /// De HDR-weg (FP16 + shader) tegen de gewone BGRA-weg op hetzelfde scherm: onder de
    /// knie moet een grijswaarde op afronding na dezelfde grijswaarde blijven. Per niveau
    /// de mediaan, zodat een bewegend beeld tussen de twee opnames niet meetelt. Op een
    /// HDR-scherm is dit ook de meting achter "schaal 1,0" in `hdr.rs`.
    #[test]
    #[ignore = "vereist een echt scherm"]
    fn hdr_weg_geeft_hetzelfde_beeld_als_bgra() {
        let d3d = D3dContext::new().expect("D3D11");
        let scherm = beschikbare_bronnen()
            .expect("bronnen")
            .into_iter()
            .find(|b| b.soort == BronSoort::Monitor)
            .expect("scherm");
        println!("in HDR: {}", crate::hdr::staat_in_hdr(monitor_van(&scherm)));

        let beeld = |cap: &mut Schermcapture| {
            (0..30)
                .find_map(|_| cap.volgende_frame(Duration::from_millis(200)))
                .map(|o| d3d.lees_bgra(&o.textuur).expect("uitlezen").2)
                .expect("een beeld")
        };
        let a = beeld(&mut Schermcapture::open(&d3d, &scherm, false).expect("opname"));
        let mut cap = Schermcapture::open(&d3d, &scherm, true).expect("opname");
        assert!(cap.hdr.is_some(), "omzetter niet opgezet");
        let b = beeld(&mut cap);

        // Onder de knie (0,8 lineair is sRGB 231).
        let mut per_niveau: Vec<Vec<u8>> = vec![Vec::new(); 225];
        for (pa, pb) in a.chunks(4).zip(b.chunks(4)) {
            if pa[0] == pa[1] && pa[1] == pa[2] && pa[0] < 225 {
                per_niveau[pa[0] as usize].push(pb[0]);
            }
        }
        let mut getoetst = 0;
        for (v, mut b) in per_niveau.into_iter().enumerate() {
            if b.len() < 20 {
                continue;
            }
            b.sort_unstable();
            let mediaan = b[b.len() / 2];
            println!("{v:3} -> {mediaan:3} ({} pixels)", b.len());
            assert!(mediaan.abs_diff(v as u8) <= 1, "grijs {v} werd {mediaan}");
            getoetst += 1;
        }
        assert!(getoetst > 0, "geen enkel grijsniveau om te vergelijken");
    }
}
