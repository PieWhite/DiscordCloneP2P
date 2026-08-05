//! Het venster waarin een inkomende stream te zien is — de macOS-tegenhanger van het
//! Win32-venster met eigen swapchain.
//!
//! # Hoe dit zonder eigen renderpad kan
//!
//! AppKit is main-thread-only, maar de main-runloop van de app draait al (Tauri).
//! Het venster wordt dus via de main-queue aangemaakt; daarna raakt de kijk-thread
//! AppKit niet meer aan. Tonen loopt via `AVSampleBufferVideoRenderer.enqueue` — die
//! is thread-veilig, dus de kijk-thread en de gedeelde `Weergaveklok` blijven de
//! timingautoriteit, precies zoals op Windows. Er komt geen regel Metal aan te pas:
//! de laag schaalt en letterboxt zelf (`ResizeAspect`), en `contentAspectRatio` houdt
//! het venster in de verhouding van de stream tijdens het slepen.
//!
//! Beeldvullend gaat via de groene knop (native fullscreen); de F11/dubbelklik-route
//! van Windows bestaat hier bewust niet — het native gedrag is wat een mac-gebruiker
//! verwacht.

use crate::d3d::{afmetingen, Beeld, D3dContext};
use anyhow::{anyhow, bail, Context, Result};
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSBackingStoreType, NSWindow, NSWindowStyleMask, NSWindowWillCloseNotification,
};
use objc2_av_foundation::{
    AVLayerVideoGravityResizeAspect, AVQueuedSampleBufferRendering,
    AVQueuedSampleBufferRenderingStatus, AVSampleBufferDisplayLayer, AVSampleBufferVideoRenderer,
};
use objc2_core_foundation::{CFBoolean, CFRetained, CFString};
use objc2_core_media::{
    kCMSampleAttachmentKey_DisplayImmediately, CMSampleBuffer, CMSampleTimingInfo, CMTime,
    CMVideoFormatDescription, CMVideoFormatDescriptionCreateForImageBuffer,
};
use objc2_foundation::{
    NSNotification, NSNotificationCenter, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString,
};
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::sync_channel;
use std::sync::Arc;
use std::time::Duration;

/// AppKit-handles die alleen op de main thread aangeraakt worden, maar wel in een
/// struct op de kijk-thread wonen. Elke methode die ze gebruikt gaat via de
/// main-queue; dit omhulsel maakt dat vervoer expliciet.
struct OpMain<T>(T);
// SAFETY: het binnenveld wordt uitsluitend op de main thread gebruikt; alleen het
// eigendom verhuist tussen threads, en ObjC-refcounts zijn atomair.
unsafe impl<T> Send for OpMain<T> {}

pub struct Venster {
    venster: Option<OpMain<Retained<NSWindow>>>,
    /// Houdt de close-observer levend; opruimen doet notificatiecentrum zelf.
    _observer: OpMain<Retained<ProtocolObjectNS>>,
    renderer: OpMain<Retained<AVSampleBufferVideoRenderer>>,
    gesloten: Arc<AtomicBool>,
    afmeting: (u32, u32),
    /// Format description hoort bij de afmeting; wisselt mee met het beeld.
    formaat: Option<CFRetained<CMVideoFormatDescription>>,
}

type ProtocolObjectNS = objc2::runtime::ProtocolObject<dyn NSObjectProtocol>;

impl Venster {
    /// Maakt het venster aan vanaf de kijk-thread; het echte werk gebeurt op de
    /// main-queue en het resultaat komt via een kanaal terug.
    pub fn open(_d3d: &D3dContext, titel: &str, breedte: u32, hoogte: u32) -> Result<Self> {
        let gesloten = Arc::new(AtomicBool::new(false));
        let gesloten_voor_blok = gesloten.clone();
        let titel = titel.to_string();

        type Klaar = std::result::Result<
            (
                OpMain<Retained<NSWindow>>,
                OpMain<Retained<ProtocolObjectNS>>,
                OpMain<Retained<AVSampleBufferVideoRenderer>>,
            ),
            String,
        >;
        let (tx, rx) = sync_channel::<Klaar>(1);

        dispatch2::DispatchQueue::main().exec_async(move || {
            let Some(mtm) = MainThreadMarker::new() else {
                let _ = tx.try_send(Err("main-queue draait niet op de main thread".into()));
                return;
            };
            let kader = NSRect::new(
                NSPoint::new(120.0, 120.0),
                NSSize::new(
                    f64::from(breedte.max(320)) / 2.0,
                    f64::from(hoogte.max(180)) / 2.0,
                ),
            );
            let venster = unsafe {
                NSWindow::initWithContentRect_styleMask_backing_defer(
                    mtm.alloc(),
                    kader,
                    NSWindowStyleMask::Titled
                        | NSWindowStyleMask::Closable
                        | NSWindowStyleMask::Miniaturizable
                        | NSWindowStyleMask::Resizable,
                    NSBackingStoreType::Buffered,
                    false,
                )
            };
            // Retained beheert de levensduur; AppKit mag hem bij sluiten niet ook
            // nog eens vrijgeven.
            // SAFETY: wij houden een Retained vast tot na `close`.
            unsafe { venster.setReleasedWhenClosed(false) };
            venster.setTitle(&NSString::from_str(&titel));
            venster.setContentAspectRatio(NSSize::new(f64::from(breedte), f64::from(hoogte)));

            let laag = unsafe { AVSampleBufferDisplayLayer::new() };
            unsafe {
                if let Some(zwaartekracht) = AVLayerVideoGravityResizeAspect {
                    laag.setVideoGravity(zwaartekracht);
                }
            }
            let renderer = unsafe { laag.sampleBufferRenderer() };
            let Some(inhoud) = venster.contentView() else {
                let _ = tx.try_send(Err("venster zonder contentview".into()));
                return;
            };
            // Layer-hosting: de laag ís de inhoud en groeit vanzelf met het venster mee.
            inhoud.setLayer(Some(&laag));
            inhoud.setWantsLayer(true);

            let observer = unsafe {
                NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
                    Some(NSWindowWillCloseNotification),
                    Some(&venster),
                    None,
                    &RcBlock::new(move |_n: NonNull<NSNotification>| {
                        gesloten_voor_blok.store(true, Ordering::Relaxed);
                    }),
                )
            };

            venster.center();
            venster.makeKeyAndOrderFront(None);

            let _ = tx.try_send(Ok((OpMain(venster), OpMain(observer), OpMain(renderer))));
        });

        let (venster, observer, renderer) = rx
            .recv_timeout(Duration::from_secs(5))
            .context("videovenster reageert niet (draait de main-runloop?)")?
            .map_err(|m| anyhow!(m))?;

        tracing::info!(breedte, hoogte, "videovenster geopend");

        Ok(Self {
            venster: Some(venster),
            _observer: observer,
            renderer,
            gesloten,
            afmeting: (breedte, hoogte),
            formaat: None,
        })
    }

    pub fn afmeting(&self) -> (u32, u32) {
        self.afmeting
    }

    /// `false` betekent dat de gebruiker het venster gesloten heeft. De main-runloop
    /// van de app pompt de vensterberichten al; hier valt niets te pompen.
    pub fn pomp(&mut self) -> bool {
        !self.gesloten.load(Ordering::Relaxed)
    }

    /// Zet één beeld in de weergavewachtrij. Thread-veilig vanaf de kijk-thread.
    pub fn toon(&mut self, beeld: &Beeld) -> Result<()> {
        let maat = afmetingen(beeld);
        if maat != self.afmeting || self.formaat.is_none() {
            self.pas_maat_aan(maat, beeld)?;
        }
        let formaat = self.formaat.as_ref().context("geen beeldformaat")?;

        let mut timing = CMSampleTimingInfo {
            duration: unsafe { CMTime::new(1, 60) },
            presentationTimeStamp: unsafe { CMTime::new(0, 60) },
            decodeTimeStamp: unsafe { CMTime::new(0, 0) },
        };
        let mut sbuf: *mut CMSampleBuffer = std::ptr::null_mut();
        // SAFETY: beeld en formaat horen bij elkaar (zelfde afmetingen, zie hierboven).
        let status = unsafe {
            CMSampleBuffer::create_ready_with_image_buffer(
                None,
                beeld.cv(),
                formaat,
                NonNull::from(&mut timing),
                NonNull::from(&mut sbuf),
            )
        };
        if status != 0 {
            bail!("CMSampleBufferCreateReadyWithImageBuffer gaf {status}");
        }
        let sbuf = unsafe { CFRetained::from_raw(NonNull::new(sbuf).context("geen sample")?) };

        // Meteen tonen: de kijk-thread heeft de weergaveklok al toegepast, dus de
        // renderer hoeft niet ook nog eens te plannen.
        unsafe {
            if let Some(arr) = sbuf.sample_attachments_array(true) {
                if !arr.is_empty() {
                    let d = objc2_core_foundation::CFArray::value_at_index(&arr, 0)
                        as *mut objc2_core_foundation::CFMutableDictionary;
                    if !d.is_null() {
                        objc2_core_foundation::CFMutableDictionary::set_value(
                            Some(&*d),
                            kCMSampleAttachmentKey_DisplayImmediately as *const CFString
                                as *const c_void,
                            CFBoolean::new(true) as *const CFBoolean as *const c_void,
                        );
                    }
                }
            }

            let renderer = &self.renderer.0;
            if renderer.status() == AVQueuedSampleBufferRenderingStatus::Failed {
                // Eén kapot sample zet de renderer vast; spoelen en doorgaan.
                renderer.flush();
            }
            renderer.enqueueSampleBuffer(&sbuf);
        }
        Ok(())
    }

    fn pas_maat_aan(&mut self, maat: (u32, u32), beeld: &Beeld) -> Result<()> {
        if maat != self.afmeting {
            tracing::info!(van = ?self.afmeting, naar = ?maat, "stream van maat veranderd");
        }
        let mut desc: *const CMVideoFormatDescription = std::ptr::null();
        // SAFETY: de description beschrijft precies deze buffer.
        let status = unsafe {
            CMVideoFormatDescriptionCreateForImageBuffer(None, beeld.cv(), NonNull::from(&mut desc))
        };
        if status != 0 {
            bail!("CMVideoFormatDescriptionCreateForImageBuffer gaf {status}");
        }
        self.formaat = Some(unsafe {
            CFRetained::from_raw(
                NonNull::new(desc as *mut CMVideoFormatDescription).context("geen formaat")?,
            )
        });
        self.afmeting = maat;

        // De beeldverhouding die het venster bewaakt klopt nu niet meer.
        if let Some(venster) = &self.venster {
            let venster = OpMain(venster.0.clone());
            dispatch2::DispatchQueue::main().exec_async(move || {
                let venster = venster;
                venster
                    .0
                    .setContentAspectRatio(NSSize::new(f64::from(maat.0), f64::from(maat.1)));
            });
        }
        Ok(())
    }
}

impl Drop for Venster {
    fn drop(&mut self) {
        if let Some(venster) = self.venster.take() {
            dispatch2::DispatchQueue::main().exec_async(move || {
                let venster = venster;
                venster.0.close();
            });
        }
    }
}
