//! Webcam-opname via Media Foundation.
//!
//! ```text
//! camera ─► IMFSourceReader (RGB32) ─► werkgeheugen ─► D3D11-textuur ─► encoder
//! ```
//!
//! # Waarom dit pad wél door het werkgeheugen gaat
//!
//! Bij een gedeeld scherm is "nooit een kopie naar het werkgeheugen" een harde eis:
//! 1080p60 is drie megabyte zestig keer per seconde, en dat naast een draaiende game is
//! precies de merkbare last die `CLAUDE.md` verbiedt. Een webcam is een andere orde:
//! 720p op 30 beelden per seconde is 2,7 MB per beeld, en camera's leveren hun beeld
//! bovendien vrijwel altijd als MJPEG of YUY2 aan — dus er staat hoe dan ook een
//! omzetting in het pad. Media Foundation doet die omzetting met zijn eigen
//! geoptimaliseerde Video Processor zodra we om RGB32 vragen, en dan is één `memcpy`
//! naar een textuur goedkoper dan zelf een NV12-tussenstap met de GPU-videoprocessor
//! optuigen.
//!
//! ponytail: bij een 4K-webcam op 60 fps wordt die kopie wel voelbaar. Het upgradepad is
//! de reader om NV12 in een DXGI-buffer vragen (`MF_SOURCE_READER_D3D_MANAGER` staat er
//! dan al) en `crate::kleur::Kleuromzetter` ertussen zetten — die doet NV12 → BGRA al op
//! de GPU voor de decoder. Niemand in de groep heeft zo'n camera, dus niet gebouwd.
//!
//! # Waarom de reader op zijn eigen thread staat
//!
//! `ReadSample` blokkeert tot er een beeld is, en de deel-lus wil een timeout kunnen
//! stellen (`volgende_frame(FRAME_WACHT)`) om te kunnen kijken of hij nog door moet gaan.
//! Een eigen thread die in een kanaal duwt geeft precies dat, en het is hetzelfde patroon
//! als de macOS-kant gebruikt.

use crate::capture::{Bron, BronSoort, Opgenomen};
use crate::d3d::D3dContext;
use anyhow::{bail, Context, Result};
use crossbeam_channel::{bounded, Receiver};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use windows::core::PWSTR;
use windows::Win32::Media::MediaFoundation::{
    IMFActivate, IMFMediaSource, IMFSourceReader, MFCreateAttributes, MFCreateMediaType,
    MFCreateSourceReaderFromMediaSource, MFEnumDeviceSources, MFMediaType_Video,
    MFVideoFormat_RGB32, MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
    MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID, MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_SIZE,
    MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE, MF_SOURCE_READERF_ENDOFSTREAM,
    MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, MF_SOURCE_READER_FIRST_VIDEO_STREAM,
};

/// Twee beelden in de rij, net als bij de schermopname: de deel-lus pakt het nieuwste en
/// gooit de rest weg, dus meer buffers betekent alleen oudere beelden bewaren.
const KANAAL_DIEPTE: usize = 2;

/// De maat die in de aankondiging staat zolang de camera nog niet loopt. Zie
/// `capture::afmeting_van`: de camera opendraaien om zijn echte maat te vragen zou het
/// lampje aanzetten voordat er iemand kijkt. 720p is wat de meeste webcams standaard
/// leveren; het eerste echte beeld corrigeert het aan beide kanten.
pub const NOMINALE_AFMETING: (u32, u32) = (1280, 720);

/// Zo lang wacht `Cameracapture::start` op het eerste beeld. Een camera met een lampje
/// heeft even nodig om te belichten; korter dan dit meldt "camera doet niets" terwijl hij
/// nog aan het opstarten is.
const EERSTE_BEELD_WACHT: Duration = Duration::from_secs(5);

/// De camera's die deze machine heeft, in de vorm die de bronkiezer verwacht.
///
/// De `handle` is de plek in deze lijst. Dat is genoeg omdat `Cameracapture::start` eerst
/// op naam zoekt en de index alleen als terugvaloptie gebruikt: tussen het openklappen
/// van de kiezer en het klikken kan er een camera bijkomen of weggaan, en dan is de index
/// van gisteren de verkeerde camera van vandaag.
pub fn cameras() -> Result<Vec<Bron>> {
    crate::mf::zorg_dat_mf_draait();
    let namen = camera_namen()?;
    Ok(namen
        .into_iter()
        .enumerate()
        .map(|(i, naam)| Bron {
            naam,
            soort: BronSoort::Camera,
            handle: i as isize,
        })
        .collect())
}

/// De vriendelijke namen van alle videovastlegapparaten, in de volgorde die Media
/// Foundation aanhoudt.
fn camera_namen() -> Result<Vec<String>> {
    // SAFETY: het attribuut wordt volledig gevuld voordat het gebruikt wordt, en de
    // array uit `MFEnumDeviceSources` wordt hieronder netjes vrijgegeven.
    unsafe {
        let mut attrs = None;
        MFCreateAttributes(&mut attrs, 1).context("attributen voor apparaatlijst")?;
        let attrs = attrs.context("MFCreateAttributes gaf niets")?;
        attrs
            .SetGUID(
                &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
                &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
            )
            .context("apparaatsoort op video zetten")?;

        let mut lijst: *mut Option<IMFActivate> = std::ptr::null_mut();
        let mut aantal: u32 = 0;
        MFEnumDeviceSources(&attrs, &mut lijst, &mut aantal).context("camera's opsommen")?;

        let mut uit = Vec::with_capacity(aantal as usize);
        for i in 0..aantal as usize {
            // Elke plek in de array is een losse IMFActivate die wij moeten laten gaan;
            // `take` haalt hem eruit en `Option`'s Drop doet de Release.
            if let Some(activate) = (*lijst.add(i)).take() {
                uit.push(naam_van(&activate).unwrap_or_else(|| format!("Camera {}", i + 1)));
            }
        }
        if !lijst.is_null() {
            windows::Win32::System::Com::CoTaskMemFree(Some(lijst as *const _));
        }
        Ok(uit)
    }
}

/// De vriendelijke naam van één apparaat. `None` als hij er geen heeft — dan verzint de
/// aanroeper er een, want een bron zonder naam is in de kiezer onbruikbaar.
fn naam_van(activate: &IMFActivate) -> Option<String> {
    let mut ptr = PWSTR::null();
    let mut lengte = 0u32;
    // SAFETY: MF vult `ptr` met een string die wij moeten vrijgeven; dat doet
    // `to_string` niet, dus daarna expliciet.
    unsafe {
        activate
            .GetAllocatedString(&MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, &mut ptr, &mut lengte)
            .ok()?;
        let naam = ptr.to_string().ok();
        windows::Win32::System::Com::CoTaskMemFree(Some(ptr.0 as *const _));
        naam.filter(|n| !n.is_empty())
    }
}

/// De camera die bij deze bron hoort openen: eerst op naam, anders op plek in de lijst.
fn open_apparaat(bron: &Bron) -> Result<IMFMediaSource> {
    // SAFETY: zie `camera_namen`; hetzelfde patroon, maar nu houden we er één.
    unsafe {
        let mut attrs = None;
        MFCreateAttributes(&mut attrs, 1).context("attributen voor apparaatlijst")?;
        let attrs = attrs.context("MFCreateAttributes gaf niets")?;
        attrs.SetGUID(
            &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,
            &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID,
        )?;

        let mut lijst: *mut Option<IMFActivate> = std::ptr::null_mut();
        let mut aantal: u32 = 0;
        MFEnumDeviceSources(&attrs, &mut lijst, &mut aantal).context("camera's opsommen")?;

        let mut apparaten: Vec<(String, IMFActivate)> = Vec::with_capacity(aantal as usize);
        for i in 0..aantal as usize {
            if let Some(activate) = (*lijst.add(i)).take() {
                let naam = naam_van(&activate).unwrap_or_else(|| format!("Camera {}", i + 1));
                apparaten.push((naam, activate));
            }
        }
        if !lijst.is_null() {
            windows::Win32::System::Com::CoTaskMemFree(Some(lijst as *const _));
        }

        if apparaten.is_empty() {
            bail!("deze pc heeft geen camera");
        }

        // Naam eerst: de lijst kan verschoven zijn sinds de kiezer hem ophaalde.
        let index = apparaten
            .iter()
            .position(|(naam, _)| *naam == bron.naam)
            .or_else(|| {
                let i = usize::try_from(bron.handle).ok()?;
                (i < apparaten.len()).then_some(i)
            })
            .with_context(|| format!("camera \"{}\" is er niet meer", bron.naam))?;

        let (naam, activate) = &apparaten[index];
        if *naam != bron.naam {
            tracing::info!(gevraagd = %bron.naam, gekozen = %naam, "camera op plek gekozen; de naam klopte niet meer");
        }
        activate
            .ActivateObject::<IMFMediaSource>()
            .with_context(|| format!("camera \"{naam}\" openen (in gebruik door iets anders?)"))
    }
}

/// Zet een source reader op die BGRA levert, en geeft ook de afmeting terug.
///
/// `MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING` is wat dit kort houdt: daarmee
/// zet Media Foundation zelf de nodige decoder en kleuromzetter in het pad, dus we
/// vragen om RGB32 en hoeven ons niets aan te trekken van of de camera MJPEG, YUY2 of
/// NV12 uitspuugt.
fn open_reader(bron: &Bron) -> Result<(IMFSourceReader, u32, u32, i32)> {
    let bron_media = open_apparaat(bron)?;

    // SAFETY: alle types worden volledig gevuld voordat ze gezet worden.
    unsafe {
        let mut attrs = None;
        MFCreateAttributes(&mut attrs, 1).context("attributen voor de reader")?;
        let attrs = attrs.context("MFCreateAttributes gaf niets")?;
        attrs
            .SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1)
            .context("kleuromzetting aanzetten")?;

        let reader = MFCreateSourceReaderFromMediaSource(&bron_media, &attrs)
            .context("source reader voor de camera")?;

        let stroom = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
        let gewenst = MFCreateMediaType().context("uitvoertype aanmaken")?;
        gewenst.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        gewenst.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
        reader
            .SetCurrentMediaType(stroom, None, &gewenst)
            .context("camera op BGRA zetten")?;

        let huidig = reader
            .GetCurrentMediaType(stroom)
            .context("onderhandeld type opvragen")?;
        let maat = huidig
            .GetUINT64(&MF_MT_FRAME_SIZE)
            .context("afmeting van de camera opvragen")?;
        let breedte = (maat >> 32) as u32;
        let hoogte = (maat & 0xffff_ffff) as u32;
        if breedte == 0 || hoogte == 0 {
            bail!("camera meldt een afmeting van {breedte}×{hoogte}");
        }

        // RGB32 is bij Media Foundation een DIB-indeling, en die staat *onderstboven*:
        // rij 0 is de onderste, aangegeven met een negatieve stride. Een omgekeerd beeld
        // is precies het soort fout dat je pas ziet als je vriend ernaar kijkt, dus we
        // vragen expliciet om bovenaf door zelf een positieve stride te zetten — nu de
        // afmeting bekend is, kan dat.
        let recht = MFCreateMediaType().context("uitvoertype aanmaken")?;
        if huidig.CopyAllItems(&recht).is_ok()
            && recht.SetUINT32(&MF_MT_DEFAULT_STRIDE, breedte * 4).is_ok()
            && reader.SetCurrentMediaType(stroom, None, &recht).is_ok()
        {
            tracing::debug!("camera levert het beeld bovenaf");
        } else {
            tracing::debug!("camera houdt zijn eigen rij-indeling; die volgen we");
        }

        // Hoe het ook uitpakte: de stride uit het *werkelijk* geldende type is de
        // waarheid, en zijn teken zegt of we rijen moeten omklappen.
        let stride = reader
            .GetCurrentMediaType(stroom)
            .and_then(|t| t.GetUINT32(&MF_MT_DEFAULT_STRIDE))
            .map(|s| s as i32)
            .unwrap_or((breedte * 4) as i32);
        if stride < 0 {
            tracing::info!("camera levert onderstboven; beeld wordt omgeklapt");
        }

        Ok((reader, breedte, hoogte, stride))
    }
}

pub struct Cameracapture {
    frames: Receiver<(Vec<u8>, i64)>,
    stop: Arc<AtomicBool>,
    d3d: D3dContext,
    afmeting: (u32, u32),
}

impl Cameracapture {
    pub fn start(d3d: &D3dContext, bron: &Bron) -> Result<Self> {
        crate::mf::zorg_dat_mf_draait();

        // De reader wordt op de leesthread aangemaakt en blijft daar: MF-objecten zijn
        // niet thread-affien zolang je ze niet vanaf twee threads tegelijk aanroept, en
        // op deze manier is dat structureel onmogelijk.
        let (klaar_tx, klaar_rx) = bounded::<Result<(u32, u32)>>(1);
        let (tx, frames) = bounded(KANAAL_DIEPTE);
        let stop = Arc::new(AtomicBool::new(false));

        let bron_kopie = bron.clone();
        let stop_lees = stop.clone();
        std::thread::Builder::new()
            .name("fitcom-camera".into())
            .spawn(move || {
                let (reader, breedte, hoogte, stride) = match open_reader(&bron_kopie) {
                    Ok(v) => v,
                    Err(e) => {
                        let _ = klaar_tx.send(Err(e));
                        return;
                    }
                };
                let _ = klaar_tx.send(Ok((breedte, hoogte)));

                let stroom = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
                let rij = breedte as usize * 4;
                while !stop_lees.load(Ordering::Relaxed) {
                    let mut vlaggen = 0u32;
                    let mut tijd = 0i64;
                    let mut sample = None;
                    // SAFETY: alle uitvoerparameters zijn geldige, in leven gehouden
                    // plekken; `reader` hoort bij deze thread.
                    let uitkomst = unsafe {
                        reader.ReadSample(
                            stroom,
                            0,
                            None,
                            Some(&mut vlaggen),
                            Some(&mut tijd),
                            Some(&mut sample),
                        )
                    };
                    if let Err(e) = uitkomst {
                        tracing::error!(error = %e, "camera lezen mislukt; opname stopt");
                        return;
                    }
                    let Some(sample) = sample else {
                        // Een leeg antwoord is normaal: een timeout of een
                        // stroomwijziging. Doorgaan, tenzij de camera echt klaar is —
                        // dat gebeurt als iemand de USB-stekker eruit trekt.
                        if vlaggen & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                            tracing::info!("camera meldt einde van de stroom");
                            return;
                        }
                        continue;
                    };

                    // SAFETY: het sample komt net uit de reader; `Lock` geeft een
                    // aaneengesloten buffer die tot `Unlock` geldig blijft.
                    let gekopieerd = unsafe {
                        let buffer = match sample.ConvertToContiguousBuffer() {
                            Ok(b) => b,
                            Err(e) => {
                                tracing::warn!(error = %e, "camerabeeld niet aaneengesloten te krijgen");
                                continue;
                            }
                        };
                        let mut basis: *mut u8 = std::ptr::null_mut();
                        let mut lengte = 0u32;
                        if let Err(e) = buffer.Lock(&mut basis, None, Some(&mut lengte)) {
                            tracing::warn!(error = %e, "camerabuffer niet te lezen");
                            continue;
                        }
                        let nodig = rij * hoogte as usize;
                        let uit = if (lengte as usize) < nodig || basis.is_null() {
                            tracing::warn!(lengte, nodig, "camerabeeld is te klein; overgeslagen");
                            None
                        } else if stride < 0 {
                            // Onderstboven: rij voor rij achterstevoren kopiëren, want
                            // de encoder en de kijker verwachten rij 0 bovenaan.
                            let mut uit = Vec::with_capacity(nodig);
                            for y in (0..hoogte as usize).rev() {
                                uit.extend_from_slice(std::slice::from_raw_parts(
                                    basis.add(y * rij),
                                    rij,
                                ));
                            }
                            Some(uit)
                        } else {
                            Some(std::slice::from_raw_parts(basis, nodig).to_vec())
                        };
                        let _ = buffer.Unlock();
                        uit
                    };

                    if let Some(pixels) = gekopieerd {
                        // Vol kanaal betekent dat de deel-lus achterloopt; het oudste
                        // beeld is dan toch al niet meer actueel.
                        if tx.try_send((pixels, tijd)).is_err() {
                            tracing::trace!("camerabeeld weggegooid; deel-lus loopt achter");
                        }
                    }
                }
            })
            .context("camera-thread starten")?;

        let (breedte, hoogte) = klaar_rx
            .recv_timeout(EERSTE_BEELD_WACHT)
            .context("camera reageert niet")??;

        tracing::info!(bron = %bron.naam, breedte, hoogte, "camera-opname gestart");

        Ok(Self {
            frames,
            stop,
            d3d: d3d.clone(),
            afmeting: (breedte, hoogte),
        })
    }

    pub fn afmeting(&self) -> (u32, u32) {
        self.afmeting
    }

    /// Wacht op het volgende camerabeeld en zet het in een textuur. `None` betekent dat
    /// er binnen de tijd niets kwam — bij een camera ongewoon, maar geen fout.
    pub fn volgende_frame(&mut self, timeout: Duration) -> Option<Opgenomen> {
        let (pixels, opgenomen_hns) = self.frames.recv_timeout(timeout).ok()?;
        // ponytail: een verse textuur per beeld. Bij 30 fps is dat niets; wordt het ooit
        // een 4K-camera op 60, dan is een ring van drie texturen het upgradepad.
        match self
            .d3d
            .maak_textuur_met(self.afmeting.0, self.afmeting.1, &pixels)
        {
            Ok(textuur) => Some(Opgenomen {
                textuur,
                opgenomen_hns,
            }),
            Err(e) => {
                tracing::warn!(error = %format!("{e:#}"), "camerabeeld niet naar de GPU gekregen");
                None
            }
        }
    }
}

impl Drop for Cameracapture {
    fn drop(&mut self) {
        // De leesthread hangt in `ReadSample` en merkt dit pas bij het volgende beeld.
        // Dat is één beeldtijd, en daarna sluit hij de reader zelf via zijn Drop.
        self.stop.store(true, Ordering::Relaxed);
    }
}
