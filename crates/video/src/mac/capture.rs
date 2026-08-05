//! Schermopname via ScreenCaptureKit — de macOS-tegenhanger van
//! Windows.Graphics.Capture, met dezelfde publieke API.
//!
//! # Waarom het opsommen niet via ScreenCaptureKit gaat
//!
//! `list_sources` is een synchroon Tauri-commando: de UI vraagt en wil meteen
//! antwoord. `SCShareableContent` bestaat alleen met een completion handler, en de
//! main thread daarop laten wachten is een hang wachten op een moment. CoreGraphics
//! kan hetzelfde synchroon (`CGGetActiveDisplayList`, `CGWindowListCopyWindowInfo`),
//! dus het opsommen loopt daar. Pas bij het echte opnemen — op de deel-thread, die
//! mag blokkeren — komt ScreenCaptureKit erbij.
//!
//! # Permissie
//!
//! Zowel de vensternamen in de opsomming als de opname zelf vereisen de
//! Screen-Recording-permissie (TCC). De eerste aanroep vraagt hem aan; macOS wil
//! daarna vaak een herstart van de app voordat de toekenning doorwerkt.

use crate::d3d::{afmetingen, Beeld, D3dContext};
use anyhow::{anyhow, bail, Context, Result};
use block2::RcBlock;
use crossbeam_channel::{bounded, Receiver, Sender};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{define_class, AllocAnyThread, DefinedClass};
use objc2_core_foundation::{CFArray, CFDictionary, CFNumber, CFRetained, CFString};
use objc2_core_graphics::{
    kCGWindowLayer, kCGWindowName, kCGWindowNumber, kCGWindowOwnerName, CGDisplayCopyDisplayMode,
    CGDisplayMode, CGDisplayPixelsHigh, CGDisplayPixelsWide, CGGetActiveDisplayList,
    CGMainDisplayID, CGPreflightScreenCaptureAccess, CGRequestScreenCaptureAccess,
    CGWindowListCopyWindowInfo, CGWindowListOption,
};
use objc2_core_video::{kCVPixelFormatType_32BGRA, CVPixelBuffer};
use objc2_foundation::{NSArray, NSError, NSObject, NSObjectProtocol};
use objc2_screen_capture_kit::{
    SCContentFilter, SCShareableContent, SCStream, SCStreamConfiguration, SCStreamOutput,
    SCStreamOutputType,
};
use std::ffi::c_void;
use std::sync::mpsc::sync_channel;
use std::time::Duration;

/// Twee frames in de rij is genoeg: de deel-lus haalt elk frame meteen op en houdt
/// alleen het nieuwste; meer buffers betekent alleen oudere beelden bewaren.
const KANAAL_DIEPTE: usize = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BronSoort {
    Monitor,
    Venster,
}

/// Een opgenomen beeld met de tijd waarop het gemaakt is — de presentatietijd van
/// ScreenCaptureKit, niet het moment waarop onze lus eraan toekwam. Zie de
/// Windows-kant voor waarom dat verschil het hele punt is.
#[derive(Clone)]
pub struct Opgenomen {
    pub textuur: Beeld,
    /// 100-nanoseconden-eenheden; alleen verschillen zeggen iets.
    pub opgenomen_hns: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bron {
    pub naam: String,
    pub soort: BronSoort,
    /// `CGDirectDisplayID` of `CGWindowID`, afhankelijk van de soort.
    pub handle: isize,
}

/// ScreenCaptureKit zit in elke macOS die wij ondersteunen (14+). We vragen wel vast
/// de permissie aan, zodat de eerste echte opname niet koud tegen TCC aanloopt.
pub fn ondersteund() -> bool {
    vraag_permissie();
    true
}

/// Vraagt de Screen-Recording-permissie precies één keer aan. Meer dan vragen kan
/// niet: de gebruiker moet hem in Systeeminstellingen toekennen, en macOS laat de
/// toekenning vaak pas na een herstart van de app doorwerken.
fn vraag_permissie() {
    static EEN_KEER: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    EEN_KEER.get_or_init(|| {
        if !CGPreflightScreenCaptureAccess() {
            tracing::warn!(
                "geen Screen-Recording-permissie; macOS vraagt er nu om — na toekennen de app herstarten"
            );
            let _ = CGRequestScreenCaptureAccess();
        }
    });
}

pub fn beschikbare_bronnen() -> Result<Vec<Bron>> {
    vraag_permissie();
    let mut uit = monitoren()?;
    uit.extend(vensters()?);
    Ok(uit)
}

/// Verhouding tussen echte pixels en punten op het hoofdscherm (2.0 op Retina).
fn hoofdscherm_schaal() -> f64 {
    let hoofd = CGMainDisplayID();
    let punten = CGDisplayPixelsWide(hoofd).max(1);
    let pixels = CGDisplayCopyDisplayMode(hoofd)
        .map(|m| CGDisplayMode::pixel_width(Some(&m)))
        .unwrap_or(punten);
    pixels as f64 / punten as f64
}

/// Pixelafmetingen van een scherm, via de actieve weergavemodus — de "pixels" van
/// `CGDisplayPixelsWide` zijn op Retina namelijk punten.
fn scherm_pixels(id: u32) -> (u32, u32) {
    match CGDisplayCopyDisplayMode(id) {
        Some(modus) => (
            CGDisplayMode::pixel_width(Some(&modus)) as u32,
            CGDisplayMode::pixel_height(Some(&modus)) as u32,
        ),
        None => (
            CGDisplayPixelsWide(id) as u32,
            CGDisplayPixelsHigh(id) as u32,
        ),
    }
}

fn monitoren() -> Result<Vec<Bron>> {
    let mut ids = [0u32; 16];
    let mut aantal = 0u32;
    // SAFETY: de uitvoerbuffer is 16 groot en dat geven we ook op.
    let status = unsafe { CGGetActiveDisplayList(16, ids.as_mut_ptr(), &mut aantal) };
    if status != objc2_core_graphics::CGError::Success {
        bail!("schermen opsommen gaf {status:?}");
    }
    let hoofd = CGMainDisplayID();

    Ok(ids[..aantal as usize]
        .iter()
        .map(|&id| {
            let (b, h) = scherm_pixels(id);
            let naam = if id == hoofd {
                format!("Scherm {b}×{h} (hoofdscherm)")
            } else {
                format!("Scherm {b}×{h}")
            };
            Bron {
                naam,
                soort: BronSoort::Monitor,
                handle: id as isize,
            }
        })
        .collect())
}

/// Leest één waarde uit een CFDictionary van de vensterlijst.
unsafe fn dict_waarde<T>(d: &CFDictionary, sleutel: &CFString) -> Option<*const T> {
    let v = d.value(sleutel as *const CFString as *const c_void);
    if v.is_null() {
        None
    } else {
        Some(v as *const T)
    }
}

fn vensters() -> Result<Vec<Bron>> {
    let optie = CGWindowListOption::OptionOnScreenOnly | CGWindowListOption::ExcludeDesktopElements;
    let lijst = CGWindowListCopyWindowInfo(optie, 0)
        .context("vensters opsommen (Screen-Recording-permissie?)")?;

    let mut uit = Vec::new();
    for i in 0..lijst.len() {
        // SAFETY: elk element van deze lijst is per documentatie een CFDictionary.
        let d = unsafe {
            let p = CFArray::value_at_index(&lijst, i as isize) as *const CFDictionary;
            if p.is_null() {
                continue;
            }
            &*p
        };

        // Alleen gewone vensters (laag 0): menubalk, dock en overlays wil niemand delen.
        let laag = unsafe { dict_waarde::<CFNumber>(d, kCGWindowLayer) }
            .and_then(|n| unsafe { (*n).as_i64() })
            .unwrap_or(0);
        if laag != 0 {
            continue;
        }
        let Some(nummer) = (unsafe { dict_waarde::<CFNumber>(d, kCGWindowNumber) })
            .and_then(|n| unsafe { (*n).as_i64() })
        else {
            continue;
        };
        // Zonder Screen-Recording-permissie ontbreekt de naam; zo'n regel in de lijst
        // heeft geen zin.
        let titel = unsafe { dict_waarde::<CFString>(d, kCGWindowName) }
            .map(|s| unsafe { (*s).to_string() })
            .filter(|s| !s.is_empty());
        let eigenaar = unsafe { dict_waarde::<CFString>(d, kCGWindowOwnerName) }
            .map(|s| unsafe { (*s).to_string() })
            .filter(|s| !s.is_empty());
        let naam = match (eigenaar, titel) {
            (Some(e), Some(t)) if e != t => format!("{e} — {t}"),
            (_, Some(t)) => t,
            (Some(e), None) => e,
            (None, None) => continue,
        };

        uit.push(Bron {
            naam,
            soort: BronSoort::Venster,
            handle: nummer as isize,
        });
    }
    Ok(uit)
}

/// Afmeting van een bron zonder er opname op te starten — nodig omdat een gedeelde
/// bron mét afmeting aangekondigd wordt voordat er iemand kijkt.
pub fn afmeting_van(bron: &Bron) -> Result<(u32, u32)> {
    match bron.soort {
        BronSoort::Monitor => {
            let (b, h) = scherm_pixels(bron.handle as u32);
            Ok((b.max(1), h.max(1)))
        }
        BronSoort::Venster => {
            let optie = CGWindowListOption::OptionIncludingWindow;
            let lijst = CGWindowListCopyWindowInfo(optie, bron.handle as u32)
                .context("venster opzoeken")?;
            if lijst.is_empty() {
                bail!("venster bestaat niet meer");
            }
            // SAFETY: element 0 is een CFDictionary met een kCGWindowBounds-woordenboek.
            let kader = unsafe {
                let d = CFArray::value_at_index(&lijst, 0) as *const CFDictionary;
                let d = d.as_ref().context("lege vensterinfo")?;
                let bounds = dict_waarde::<CFDictionary>(d, objc2_core_graphics::kCGWindowBounds)
                    .context("venster zonder afmetingen")?;
                let mut rect = objc2_core_foundation::CGRect::default();
                if !objc2_core_graphics::CGRectMakeWithDictionaryRepresentation(
                    Some(&*bounds),
                    &mut rect,
                ) {
                    bail!("vensterafmetingen niet te lezen");
                }
                rect
            };
            // De vensterlijst rekent in punten; het beeld in pixels. De schaal van het
            // hoofdscherm is de beste synchrone benadering — wijkt het echte beeld af,
            // dan wint dat zodra de opname loopt, net als op Windows.
            let schaal = hoofdscherm_schaal();
            Ok((
                ((kader.size.width * schaal).round() as u32).max(1),
                ((kader.size.height * schaal).round() as u32).max(1),
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// De opname zelf
// ---------------------------------------------------------------------------

struct UitvoerIvars {
    tx: Sender<(Beeld, i64)>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "FitcomSchermUitvoer"]
    #[ivars = UitvoerIvars]
    struct Schermuitvoer;

    unsafe impl NSObjectProtocol for Schermuitvoer {}

    unsafe impl SCStreamOutput for Schermuitvoer {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        fn sample_binnen(
            &self,
            _stream: &SCStream,
            sbuf: &objc2_core_media::CMSampleBuffer,
            soort: SCStreamOutputType,
        ) {
            if soort != SCStreamOutputType::Screen {
                return;
            }
            // Een statusframe zonder beeld (niets veranderd) heeft geen imagebuffer.
            let Some(beeld) = (unsafe { sbuf.image_buffer() }) else {
                return;
            };
            let pts = unsafe { sbuf.presentation_time_stamp() };
            let hns = if pts.timescale > 0 {
                pts.value.saturating_mul(super::codec::HNS_PER_SEC) / i64::from(pts.timescale)
            } else {
                0
            };
            // SAFETY: schermframes zijn CVPixelBuffers; CVImageBuffer is dezelfde
            // opaque. We retainen hem, dus hij overleeft de callback.
            let pb = unsafe {
                CFRetained::retain(std::ptr::NonNull::from(
                    &*(&*beeld as *const objc2_core_video::CVImageBuffer as *const CVPixelBuffer),
                ))
            };
            // Vol kanaal betekent dat de deel-lus achterloopt; het oudste beeld is
            // dan toch al niet meer actueel.
            let _ = self.ivars().tx.try_send((Beeld::nieuw(pb), hns));
        }
    }
);

pub struct Capture {
    stream: Retained<SCStream>,
    _uitvoer: Retained<Schermuitvoer>,
    frames: Receiver<(Beeld, i64)>,
    afmeting: (u32, u32),
}

// SAFETY: de struct wordt door één thread tegelijk gebruikt (de deel-thread); de
// SCK-objecten zelf zijn thread-veilig via hun eigen wachtrij.
unsafe impl Send for Capture {}

impl Capture {
    /// Start de opname. Mag blokkeren: dit draait op de deel-thread, en de
    /// completion-handlers van ScreenCaptureKit worden via kanalen synchroon gemaakt.
    pub fn start(_d3d: &D3dContext, bron: &Bron) -> Result<Self> {
        vraag_permissie();

        let inhoud = deelbare_inhoud()?;

        // Filter en (punt)afmetingen van de gekozen bron.
        let (filter, naam) = match bron.soort {
            BronSoort::Monitor => {
                let schermen = unsafe { inhoud.displays() };
                let scherm = schermen
                    .iter()
                    .find(|s| unsafe { s.displayID() } as isize == bron.handle)
                    .context("scherm bestaat niet meer")?;
                let filter = unsafe {
                    SCContentFilter::initWithDisplay_excludingWindows(
                        SCContentFilter::alloc(),
                        &scherm,
                        &NSArray::new(),
                    )
                };
                (filter, bron.naam.clone())
            }
            BronSoort::Venster => {
                let vensters = unsafe { inhoud.windows() };
                let venster = vensters
                    .iter()
                    .find(|v| unsafe { v.windowID() } as isize == bron.handle)
                    .context("venster bestaat niet meer")?;
                let filter = unsafe {
                    SCContentFilter::initWithDesktopIndependentWindow(
                        SCContentFilter::alloc(),
                        &venster,
                    )
                };
                (filter, bron.naam.clone())
            }
        };

        // Pixels = punten × schaal; het filter weet allebei.
        let kader = unsafe { filter.contentRect() };
        let schaal = f64::from(unsafe { filter.pointPixelScale() });
        let afmeting = (
            ((kader.size.width * schaal).round() as u32).max(2) & !1,
            ((kader.size.height * schaal).round() as u32).max(2) & !1,
        );

        let cfg = unsafe { SCStreamConfiguration::new() };
        unsafe {
            cfg.setWidth(afmeting.0 as usize);
            cfg.setHeight(afmeting.1 as usize);
            cfg.setPixelFormat(kCVPixelFormatType_32BGRA);
            cfg.setShowsCursor(true);
            cfg.setQueueDepth(3);
            // Bron en doel even groot houden: geen schaling, wel letterbox bij een
            // venster dat later van maat verandert.
            cfg.setPreservesAspectRatio(true);
        }

        let (tx, frames) = bounded(KANAAL_DIEPTE);
        let uitvoer = Schermuitvoer::alloc().set_ivars(UitvoerIvars { tx });
        let uitvoer: Retained<Schermuitvoer> = unsafe { objc2::msg_send![super(uitvoer), init] };

        let stream = unsafe {
            SCStream::initWithFilter_configuration_delegate(SCStream::alloc(), &filter, &cfg, None)
        };
        let wachtrij = dispatch2::DispatchQueue::new("fitcom.capture.sck", None);
        unsafe {
            stream
                .addStreamOutput_type_sampleHandlerQueue_error(
                    ProtocolObject::from_ref(&*uitvoer),
                    SCStreamOutputType::Screen,
                    Some(&wachtrij),
                )
                .map_err(|e| anyhow!(e.localizedDescription().to_string()))
                .context("schermuitvoer aan SCK-stream koppelen")?;
        }

        let (start_tx, start_rx) = sync_channel::<std::result::Result<(), String>>(1);
        let blok = RcBlock::new(move |fout: *mut NSError| {
            let uitkomst = if fout.is_null() {
                Ok(())
            } else {
                // SAFETY: net op null gecontroleerd.
                Err(unsafe { (*fout).localizedDescription() }.to_string())
            };
            let _ = start_tx.try_send(uitkomst);
        });
        unsafe { stream.startCaptureWithCompletionHandler(Some(&blok)) };
        start_rx
            .recv_timeout(Duration::from_secs(10))
            .context("startCapture kwam niet")?
            .map_err(|m| anyhow!(m).context("opname starten (Screen-Recording-permissie?)"))?;

        tracing::info!(bron = %naam, breedte = afmeting.0, hoogte = afmeting.1, "opname gestart");

        Ok(Self {
            stream,
            _uitvoer: uitvoer,
            frames,
            afmeting,
        })
    }

    pub fn afmeting(&self) -> (u32, u32) {
        self.afmeting
    }

    /// Wacht op het volgende beeld. `None` betekent dat er binnen de tijd niets
    /// kwam — normaal voor een venster dat niet verandert.
    pub fn volgende_frame(&mut self, timeout: Duration) -> Option<Opgenomen> {
        let (textuur, opgenomen_hns) = self.frames.recv_timeout(timeout).ok()?;
        let maat = afmetingen(&textuur);
        if maat != self.afmeting {
            // SCK levert de geconfigureerde maat, dus dit hoort niet voor te komen;
            // meegeven wat er echt is — het echte beeld wint, net als op Windows.
            tracing::info!(van = ?self.afmeting, naar = ?maat, "bron van afmeting veranderd");
            self.afmeting = maat;
        }
        Some(Opgenomen {
            textuur,
            opgenomen_hns,
        })
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        let (stop_tx, stop_rx) = sync_channel::<()>(1);
        let blok = RcBlock::new(move |_fout: *mut NSError| {
            let _ = stop_tx.try_send(());
        });
        unsafe { self.stream.stopCaptureWithCompletionHandler(Some(&blok)) };
        let _ = stop_rx.recv_timeout(Duration::from_secs(3));
    }
}

/// Haalt de deelbare inhoud op — async API, synchroon gemaakt via een kanaal.
fn deelbare_inhoud() -> Result<Retained<SCShareableContent>> {
    let (tx, rx) = sync_channel::<std::result::Result<Retained<SCShareableContent>, String>>(1);
    let blok = RcBlock::new(move |inhoud: *mut SCShareableContent, fout: *mut NSError| {
        let uitkomst = if inhoud.is_null() {
            Err(if fout.is_null() {
                "onbekende fout".to_string()
            } else {
                // SAFETY: net op null gecontroleerd.
                unsafe { (*fout).localizedDescription() }.to_string()
            })
        } else {
            // SAFETY: net op null gecontroleerd.
            Ok(unsafe { Retained::retain(inhoud) }.expect("niet null"))
        };
        let _ = tx.try_send(uitkomst);
    });
    unsafe { SCShareableContent::getShareableContentWithCompletionHandler(&blok) };
    rx.recv_timeout(Duration::from_secs(10))
        .context("SCShareableContent kwam niet")?
        .map_err(|m| anyhow!(m).context("deelbare bronnen opvragen (Screen-Recording-permissie?)"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "vereist een echt scherm en Screen-Recording-permissie"]
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
    #[ignore = "vereist een echt scherm en Screen-Recording-permissie"]
    fn scherm_levert_beelden() {
        let d3d = D3dContext::new().expect("context");
        let scherm = beschikbare_bronnen()
            .expect("bronnen")
            .into_iter()
            .find(|b| b.soort == BronSoort::Monitor)
            .expect("scherm");

        let mut cap = Capture::start(&d3d, &scherm).expect("opname starten");
        let (b, h) = cap.afmeting();
        println!("opname {b}×{h}");
        assert!(b >= 640 && h >= 480, "onwaarschijnlijke afmeting");

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
}
