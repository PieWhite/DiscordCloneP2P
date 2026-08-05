//! Hardware-encoder en -decoder via VideoToolbox — de macOS-tegenhanger van de
//! Media Foundation-module, met dezelfde publieke API.
//!
//! # Waarom hier een Annex-B-brug in zit
//!
//! Op de draad staat H.264 in Annex-B: startcodes, met SPS/PPS inline op elk
//! keyframe. Dat is wat de Media Foundation-codecs op Windows uitspreken en
//! verwachten. VideoToolbox spreekt AVCC (4-byte lengteprefixen, parametersets in de
//! format description). De encoder vertaalt dus AVCC → Annex-B en plakt SPS/PPS voor
//! elk keyframe; de decoder haalt SPS/PPS uit de stroom en vertaalt Annex-B → AVCC.
//! Zonder die brug decodeert mac↔Windows nooit — geverifieerd in de fase 0-spike.
//!
//! # Waarom de API synchroon kan blijven
//!
//! `VTCompressionSessionCompleteFrames` keert pas terug als de uitvoercallback voor
//! dat beeld gedraaid heeft, en een decode zonder async-vlag draait zijn callback op
//! de aanroepende thread vóór `DecodeFrame` terugkeert. Daarmee houden `encode` en
//! `decode` exact de vorm die `deler.rs` en `kijker.rs` verwachten.

use crate::d3d::{afmetingen, bgra_attrs, Beeld, D3dContext};
use anyhow::{anyhow, bail, Context, Result};
use objc2_core_foundation::{CFBoolean, CFDictionary, CFNumber, CFRetained, CFString, CFType};
use objc2_core_media::{
    kCMSampleAttachmentKey_NotSync, kCMVideoCodecType_H264, CMBlockBuffer, CMFormatDescription,
    CMSampleBuffer, CMSampleTimingInfo, CMTime, CMVideoFormatDescription,
    CMVideoFormatDescriptionCreateFromH264ParameterSets,
    CMVideoFormatDescriptionGetH264ParameterSetAtIndex,
};
use objc2_core_video::{CVImageBuffer, CVPixelBuffer};
use objc2_video_toolbox::{
    kVTCompressionPropertyKey_AllowFrameReordering, kVTCompressionPropertyKey_AverageBitRate,
    kVTCompressionPropertyKey_ExpectedFrameRate, kVTCompressionPropertyKey_MaxKeyFrameInterval,
    kVTCompressionPropertyKey_ProfileLevel, kVTCompressionPropertyKey_RealTime,
    kVTEncodeFrameOptionKey_ForceKeyFrame, kVTProfileLevel_H264_Main_AutoLevel,
    VTCompressionSession, VTDecodeFrameFlags, VTDecodeInfoFlags,
    VTDecompressionOutputCallbackRecord, VTDecompressionSession, VTEncodeInfoFlags, VTSession,
    VTSessionSetProperty,
};
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// 100-nanoseconden-eenheden — de tijdrekening van Media Foundation, en dus van het
/// hele gedeelde pad. VideoToolbox rekent in `CMTime` met deze schaal.
pub const HNS_PER_SEC: i64 = 10_000_000;

/// Zoveel seconden tussen twee periodieke keyframes; zelfde waarde en zelfde
/// onderbouwing als op Windows (zie `codec.rs` daar).
const GOP_SECONDEN: u32 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    H264,
    Hevc,
}

impl Codec {
    pub fn payload_type(self) -> fitcom_proto::PayloadType {
        match self {
            Codec::H264 => fitcom_proto::PayloadType::H264,
            Codec::Hevc => fitcom_proto::PayloadType::HEVC,
        }
    }

    /// De omgekeerde richting: welke codec er bij een payload-type op de draad hoort.
    /// `None` voor alles wat geen video is (Opus) of wat we nog niet kennen.
    pub fn van_payload(pt: fitcom_proto::PayloadType) -> Option<Self> {
        match pt {
            fitcom_proto::PayloadType::H264 => Some(Codec::H264),
            fitcom_proto::PayloadType::HEVC => Some(Codec::Hevc),
            _ => None,
        }
    }

    pub fn naam(self) -> &'static str {
        match self {
            Codec::H264 => "h264",
            Codec::Hevc => "hevc",
        }
    }

    pub fn van_naam(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "h264" | "avc" => Some(Codec::H264),
            "hevc" | "h265" => Some(Codec::Hevc),
            _ => None,
        }
    }

    /// Of deze machine hem kan decoderen. Elke Apple Silicon-Mac decodeert H.264 én
    /// HEVC in hardware, maar de Annex-B-brug hieronder is alleen voor H.264 gebouwd —
    /// HEVC draagt zijn parametersets anders (VPS/SPS/PPS) en niemand in de groep
    /// gebruikt hem. H.264 is de standaardcodec; zie CLAUDE.md.
    pub fn kan_decoderen(self) -> bool {
        matches!(self, Codec::H264)
    }
}

#[derive(Debug, Clone)]
pub struct EncoderConfig {
    pub codec: Codec,
    pub breedte: u32,
    pub hoogte: u32,
    pub fps: u32,
    pub bitrate: u32,
}

/// Eén gecodeerd beeld zoals het de encoder verlaat. Identiek aan de Windows-kant.
#[derive(Debug, Clone)]
pub struct Pakket {
    /// Tijd van de encoder in 100-nanoseconden-eenheden.
    pub tijd_hns: i64,
    pub keyframe: bool,
    pub data: Vec<u8>,
}

// ---------------------------------------------------------------------------
// Annex-B <-> AVCC
// ---------------------------------------------------------------------------

/// AVCC (4-byte lengteprefixen) achter `uit` plakken als Annex-B (startcodes).
fn avcc_naar_annexb(data: &[u8], uit: &mut Vec<u8>) {
    let mut i = 0usize;
    while i + 4 <= data.len() {
        let len = u32::from_be_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]) as usize;
        i += 4;
        if len == 0 || i + len > data.len() {
            break;
        }
        uit.extend_from_slice(&[0, 0, 0, 1]);
        uit.extend_from_slice(&data[i..i + len]);
        i += len;
    }
}

/// Splitst een Annex-B-stroom in NAL-eenheden (zonder startcodes). Accepteert drie-
/// en vierbyte-startcodes, want beide komen in het wild voor.
fn annexb_nals(data: &[u8]) -> Vec<&[u8]> {
    let mut nals = Vec::new();
    let mut begin: Option<usize> = None;
    let mut i = 0usize;
    while i + 3 <= data.len() {
        let (sc, sc_len) = if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            (true, 3)
        } else if i + 4 <= data.len()
            && data[i] == 0
            && data[i + 1] == 0
            && data[i + 2] == 0
            && data[i + 3] == 1
        {
            (true, 4)
        } else {
            (false, 0)
        };
        if sc {
            if let Some(b) = begin {
                nals.push(&data[b..i]);
            }
            i += sc_len;
            begin = Some(i);
        } else {
            i += 1;
        }
    }
    if let Some(b) = begin {
        nals.push(&data[b..]);
    }
    nals
}

fn nal_soort(nal: &[u8]) -> u8 {
    nal.first().map(|b| b & 0x1f).unwrap_or(0)
}

fn osstatus(naam: &str, status: i32) -> Result<()> {
    if status == 0 {
        Ok(())
    } else {
        bail!("{naam} gaf OSStatus {status}");
    }
}

fn zet_eigenschap(sessie: &VTSession, sleutel: &CFString, waarde: &CFType) -> Result<()> {
    // SAFETY: sleutel en waarde zijn geldige CF-objecten van het type dat de
    // eigenschap verwacht.
    osstatus("VTSessionSetProperty", unsafe {
        VTSessionSetProperty(sessie, sleutel, Some(waarde))
    })
}

// ---------------------------------------------------------------------------
// Encoder
// ---------------------------------------------------------------------------

/// Wat de uitvoercallback per beeld verzamelt; leeft in een `Box` zodat het adres
/// stabiel is zolang de sessie bestaat.
struct EncoderUitvoer {
    pakketten: Mutex<Vec<Pakket>>,
}

pub struct Encoder {
    sessie: CFRetained<VTCompressionSession>,
    uitvoer: Box<EncoderUitvoer>,
    frame_duur: CMTime,
    keyframe_gevraagd: AtomicBool,
}

// SAFETY: de sessie wordt uitsluitend vanaf de deel-thread gebruikt; VideoToolbox is
// thread-veilig zolang aanroepen niet gelijktijdig vanaf meerdere threads komen.
unsafe impl Send for Encoder {}

/// De uitvoercallback van de compressiesessie: AVCC → Annex-B, SPS/PPS voor elk
/// keyframe, en het resultaat in de verzamelbak.
unsafe extern "C-unwind" fn encode_klaar(
    refcon: *mut c_void,
    _bron: *mut c_void,
    status: i32,
    _vlaggen: VTEncodeInfoFlags,
    sbuf: *mut CMSampleBuffer,
) {
    if status != 0 || sbuf.is_null() || refcon.is_null() {
        return;
    }
    // SAFETY: `refcon` is de `EncoderUitvoer` in de Box van de encoder, en die leeft
    // zolang de sessie leeft; `sbuf` is net op null gecontroleerd.
    let uitvoer = unsafe { &*(refcon as *const EncoderUitvoer) };
    let sbuf = unsafe { &*sbuf };

    // Keyframe: het ontbreken van NotSync in de sample-attachments.
    let keyframe = unsafe {
        sbuf.sample_attachments_array(false)
            .map(|arr| {
                if arr.is_empty() {
                    return true;
                }
                let d = arr.value_at_index(0) as *const CFDictionary;
                if d.is_null() {
                    return true;
                }
                let v =
                    (*d).value(kCMSampleAttachmentKey_NotSync as *const CFString as *const c_void);
                v.is_null()
            })
            .unwrap_or(true)
    };

    let Some(blok) = (unsafe { sbuf.data_buffer() }) else {
        return;
    };
    let lengte = unsafe { blok.data_length() };
    if lengte == 0 {
        return;
    }
    let mut avcc = vec![0u8; lengte];
    // SAFETY: de bestemming is precies `lengte` bytes groot.
    let _ = unsafe {
        blok.copy_data_bytes(
            0,
            lengte,
            NonNull::new_unchecked(avcc.as_mut_ptr() as *mut c_void),
        )
    };

    let mut annexb = Vec::with_capacity(avcc.len() + 128);
    if keyframe {
        // SPS/PPS inline op elk keyframe — precies wat de Windows-decoder verwacht om
        // midden in een stroom te kunnen aanhaken.
        if let Some(desc) = unsafe { sbuf.format_description() } {
            let mut telling = 0usize;
            // SAFETY: alleen de telling opvragen; uitvoerpointers mogen null zijn.
            let status = unsafe {
                CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
                    &desc,
                    0,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    &mut telling,
                    std::ptr::null_mut(),
                )
            };
            if status == 0 {
                for i in 0..telling {
                    let mut ptr: *const u8 = std::ptr::null();
                    let mut maat = 0usize;
                    // SAFETY: de pointer wijst in de format description en blijft
                    // geldig zolang `desc` vastgehouden wordt — en dat is nu.
                    let status = unsafe {
                        CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
                            &desc,
                            i,
                            &mut ptr,
                            &mut maat,
                            std::ptr::null_mut(),
                            std::ptr::null_mut(),
                        )
                    };
                    if status == 0 && !ptr.is_null() {
                        annexb.extend_from_slice(&[0, 0, 0, 1]);
                        annexb.extend_from_slice(unsafe { std::slice::from_raw_parts(ptr, maat) });
                    }
                }
            }
        }
    }
    avcc_naar_annexb(&avcc, &mut annexb);

    let tijd = unsafe { sbuf.presentation_time_stamp() };
    // In i128, om dezelfde reden als in `capture.rs`: grote tijdwaarden × 1e7 passen
    // niet in i64 en saturatie plakt alle beelden op één tijdstempel.
    let tijd_hns = if tijd.timescale > 0 {
        (i128::from(tijd.value) * i128::from(HNS_PER_SEC) / i128::from(tijd.timescale)) as i64
    } else {
        0
    };

    if let Ok(mut p) = uitvoer.pakketten.lock() {
        p.push(Pakket {
            tijd_hns,
            keyframe,
            data: annexb,
        });
    }
}

impl Encoder {
    pub fn new(_d3d: &D3dContext, cfg: &EncoderConfig) -> Result<Self> {
        if cfg.codec != Codec::H264 {
            bail!(
                "codec {} wordt op macOS niet ondersteund; h264 is de standaard",
                cfg.codec.naam()
            );
        }

        let uitvoer = Box::new(EncoderUitvoer {
            pakketten: Mutex::new(Vec::new()),
        });

        let mut sessie: *mut VTCompressionSession = std::ptr::null_mut();
        // SAFETY: de callback en refcon horen bij elkaar; de Box leeft zolang de
        // sessie leeft omdat beide in dezelfde struct terechtkomen.
        let status = unsafe {
            VTCompressionSession::create(
                None,
                cfg.breedte as i32,
                cfg.hoogte as i32,
                kCMVideoCodecType_H264,
                None,
                Some(bgra_attrs().as_opaque()),
                None,
                Some(encode_klaar),
                &*uitvoer as *const EncoderUitvoer as *mut c_void,
                NonNull::from(&mut sessie),
            )
        };
        osstatus("VTCompressionSessionCreate", status)?;
        let sessie = unsafe {
            CFRetained::from_raw(NonNull::new(sessie).context("VT gaf geen encodersessie")?)
        };

        // SAFETY: VTSessionRef is in C een alias van dezelfde opaque pointer.
        let vt: &VTSession =
            unsafe { &*(&*sessie as *const VTCompressionSession as *const VTSession) };
        let fps = cfg.fps.max(1);
        // Realtime en zonder B-frames: één pakket per beeld, in volgorde — de vorm
        // waar de fragmentatie en de kijker op rekenen.
        zet_eigenschap(
            vt,
            unsafe { kVTCompressionPropertyKey_RealTime },
            CFBoolean::new(true).as_ref(),
        )?;
        zet_eigenschap(
            vt,
            unsafe { kVTCompressionPropertyKey_AllowFrameReordering },
            CFBoolean::new(false).as_ref(),
        )?;
        zet_eigenschap(
            vt,
            unsafe { kVTCompressionPropertyKey_AverageBitRate },
            CFNumber::new_i64(i64::from(cfg.bitrate)).as_ref(),
        )?;
        zet_eigenschap(
            vt,
            unsafe { kVTCompressionPropertyKey_ExpectedFrameRate },
            CFNumber::new_i64(i64::from(fps)).as_ref(),
        )?;
        zet_eigenschap(
            vt,
            unsafe { kVTCompressionPropertyKey_MaxKeyFrameInterval },
            CFNumber::new_i64(i64::from(fps * GOP_SECONDEN)).as_ref(),
        )?;
        if let Err(e) = zet_eigenschap(
            vt,
            unsafe { kVTCompressionPropertyKey_ProfileLevel },
            unsafe { kVTProfileLevel_H264_Main_AutoLevel }.as_ref(),
        ) {
            tracing::debug!(error = %e, "H.264-profiel niet in te stellen; VideoToolbox kiest zelf");
        }

        // SAFETY: de sessie is net aangemaakt en geconfigureerd.
        let status = unsafe { sessie.prepare_to_encode_frames() };
        osstatus("PrepareToEncodeFrames", status)?;

        tracing::info!(
            encoder = "VideoToolbox",
            codec = cfg.codec.naam(),
            "encoder gekozen"
        );

        Ok(Self {
            sessie,
            uitvoer,
            frame_duur: unsafe { CMTime::new(HNS_PER_SEC / i64::from(fps), HNS_PER_SEC as i32) },
            keyframe_gevraagd: AtomicBool::new(false),
        })
    }

    /// Vraagt het volgende beeld als keyframe — op VideoToolbox een eigenschap van
    /// het eerstvolgende `encode_frame`, dus hier alleen onthouden.
    pub fn vraag_keyframe(&self) {
        self.keyframe_gevraagd.store(true, Ordering::Relaxed);
    }

    /// Codeert één beeld. Levert nul of meer pakketten op, net als op Windows —
    /// al dwingt `complete_frames` hier af dat het er vrijwel altijd precies één is.
    pub fn encode(&mut self, beeld: &Beeld, tijd_hns: i64) -> Result<Vec<Pakket>> {
        let pts = unsafe { CMTime::new(tijd_hns, HNS_PER_SEC as i32) };

        let opties = if self.keyframe_gevraagd.swap(false, Ordering::Relaxed) {
            let sleutels: [&CFString; 1] = [unsafe { kVTEncodeFrameOptionKey_ForceKeyFrame }];
            let waarden: [&CFType; 1] = [CFBoolean::new(true).as_ref()];
            Some(CFDictionary::<CFString, CFType>::from_slices(
                &sleutels, &waarden,
            ))
        } else {
            None
        };

        let cv: &CVImageBuffer = beeld.cv();
        let mut vlaggen = VTEncodeInfoFlags::empty();
        // SAFETY: het beeld blijft geldig zolang de aanroeper het vasthoudt; VT houdt
        // het zelf vast zolang het nodig is.
        let status = unsafe {
            self.sessie.encode_frame(
                cv,
                pts,
                self.frame_duur,
                opties.as_deref().map(|d| d.as_opaque()),
                std::ptr::null_mut(),
                &mut vlaggen,
            )
        };
        osstatus("VTCompressionSessionEncodeFrame", status)?;
        // Blokkeert tot de callback voor dit beeld gedraaid heeft: dat houdt de API
        // synchroon en de latency op nul beelden.
        // SAFETY: zelfde sessie, geldig pts.
        let status = unsafe { self.sessie.complete_frames(pts) };
        osstatus("VTCompressionSessionCompleteFrames", status)?;

        let mut pakketten = self
            .uitvoer
            .pakketten
            .lock()
            .map_err(|_| anyhow!("encoderuitvoer vergrendeld door een panic"))?;
        Ok(std::mem::take(&mut *pakketten))
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        // SAFETY: na invalidate roept VT de callback niet meer aan, dus de Box mag
        // daarna pas opgeruimd worden — en dat gebeurt door de veldvolgorde vanzelf.
        unsafe { self.sessie.invalidate() };
    }
}

// ---------------------------------------------------------------------------
// Decoder
// ---------------------------------------------------------------------------

/// Waar de decodecallback zijn beeld neerlegt; in een `Box` om het adres stabiel te
/// houden zolang de sessie leeft.
struct DecoderUitvoer {
    beeld: Mutex<Option<Beeld>>,
}

/// Spiegelbeeld van [`Encoder`]. Neemt complete Annex-B-frames aan en levert
/// BGRA-beelden — de kleuromzetting die Windows apart moet doen zit hier in de
/// decoder zelf (`destinationImageBufferAttributes`).
///
/// # Waarom de sessie lui wordt aangemaakt
///
/// VideoToolbox kan pas een decoder bouwen als hij SPS/PPS heeft, en die komen op de
/// draad pas met het eerste keyframe mee. `new` onthoudt dus alleen de wensen; het
/// eerste keyframe bouwt de echte sessie, en een parametersetwissel bouwt hem opnieuw.
pub struct Decoder {
    sessie: Option<CFRetained<VTDecompressionSession>>,
    formaat: Option<CFRetained<CMFormatDescription>>,
    sps: Option<Vec<u8>>,
    pps: Option<Vec<u8>>,
    uitvoer: Box<DecoderUitvoer>,
    afmeting: Option<(u32, u32)>,
    frame_duur: CMTime,
}

// SAFETY: alleen gebruikt vanaf de kijk-thread van één stream, net als op Windows.
unsafe impl Send for Decoder {}

/// De uitvoercallback van de decodersessie. Draait — zonder async-vlag — op de
/// aanroepende thread, vóór `decode_frame` terugkeert.
unsafe extern "C-unwind" fn decode_klaar(
    refcon: *mut c_void,
    _bron: *mut c_void,
    status: i32,
    _vlaggen: VTDecodeInfoFlags,
    beeld: *mut CVImageBuffer,
    _pts: CMTime,
    _duur: CMTime,
) {
    if status != 0 || beeld.is_null() || refcon.is_null() {
        return;
    }
    // SAFETY: `refcon` is de `DecoderUitvoer` in de Box van de decoder; het beeld is
    // een CVPixelBuffer (wij vroegen om BGRA) en wordt hier geretaind.
    let uitvoer = unsafe { &*(refcon as *const DecoderUitvoer) };
    let pb = unsafe { CFRetained::retain(NonNull::new_unchecked(beeld as *mut CVPixelBuffer)) };
    if let Ok(mut b) = uitvoer.beeld.lock() {
        *b = Some(Beeld::nieuw(pb));
    }
}

impl Decoder {
    pub fn new(_d3d: &D3dContext, codec: Codec, _breedte: u32, _hoogte: u32) -> Result<Self> {
        if codec != Codec::H264 {
            bail!(
                "codec {} wordt op macOS niet ondersteund; h264 is de standaard",
                codec.naam()
            );
        }
        tracing::info!(
            decoder = "VideoToolbox",
            codec = codec.naam(),
            "decoder gekozen"
        );
        Ok(Self {
            sessie: None,
            formaat: None,
            sps: None,
            pps: None,
            uitvoer: Box::new(DecoderUitvoer {
                beeld: Mutex::new(None),
            }),
            afmeting: None,
            frame_duur: unsafe { CMTime::new(HNS_PER_SEC / 60, HNS_PER_SEC as i32) },
        })
    }

    /// Decodeert één compleet frame. Levert het nieuwste beeld op, of `None` zolang
    /// er nog geen keyframe met parametersets langs is gekomen.
    pub fn decode(&mut self, data: &[u8], tijd_hns: i64) -> Result<Option<Beeld>> {
        // Parametersets uit de stroom vissen; een wissel betekent een nieuwe sessie.
        let mut avcc: Vec<u8> = Vec::with_capacity(data.len());
        for nal in annexb_nals(data) {
            match nal_soort(nal) {
                7 => {
                    if self.sps.as_deref() != Some(nal) {
                        self.sps = Some(nal.to_vec());
                        self.sessie = None;
                    }
                }
                8 => {
                    if self.pps.as_deref() != Some(nal) {
                        self.pps = Some(nal.to_vec());
                        self.sessie = None;
                    }
                }
                _ => {
                    avcc.extend_from_slice(&(nal.len() as u32).to_be_bytes());
                    avcc.extend_from_slice(nal);
                }
            }
        }

        if self.sessie.is_none() {
            if self.sps.is_none() || self.pps.is_none() {
                // Nog geen keyframe gezien; de kijker wacht daar toch op.
                return Ok(None);
            }
            self.bouw_sessie()?;
        }
        if avcc.is_empty() {
            return Ok(None);
        }

        let (sessie, formaat) = match (&self.sessie, &self.formaat) {
            (Some(s), Some(f)) => (s, f),
            _ => return Ok(None),
        };

        // Het frame in een CMSampleBuffer verpakken.
        let mut blok: *mut CMBlockBuffer = std::ptr::null_mut();
        // SAFETY: CoreMedia beheert het geheugen zelf (memory_block = null) en de
        // inhoud gaat er hieronder met `replace_data_bytes` in.
        let status = unsafe {
            CMBlockBuffer::create_with_memory_block(
                None,
                std::ptr::null_mut(),
                avcc.len(),
                None,
                std::ptr::null(),
                0,
                avcc.len(),
                0,
                NonNull::from(&mut blok),
            )
        };
        osstatus("CMBlockBufferCreateWithMemoryBlock", status)?;
        let blok = unsafe { CFRetained::from_raw(NonNull::new(blok).context("geen blockbuffer")?) };
        // SAFETY: bron en bestemming zijn beide `avcc.len()` bytes.
        let status = unsafe {
            CMBlockBuffer::replace_data_bytes(
                NonNull::new_unchecked(avcc.as_ptr() as *mut c_void),
                &blok,
                0,
                avcc.len(),
            )
        };
        osstatus("CMBlockBufferReplaceDataBytes", status)?;

        let timing = CMSampleTimingInfo {
            duration: self.frame_duur,
            presentationTimeStamp: unsafe { CMTime::new(tijd_hns, HNS_PER_SEC as i32) },
            decodeTimeStamp: unsafe { CMTime::new(0, 0) },
        };
        let maat = avcc.len();
        let mut sbuf: *mut CMSampleBuffer = std::ptr::null_mut();
        // SAFETY: alle pointers wijzen naar geldige lokale waarden.
        let status = unsafe {
            CMSampleBuffer::create_ready(
                None,
                Some(&blok),
                Some(formaat),
                1,
                1,
                &timing,
                1,
                &maat,
                NonNull::from(&mut sbuf),
            )
        };
        osstatus("CMSampleBufferCreateReady", status)?;
        let sbuf =
            unsafe { CFRetained::from_raw(NonNull::new(sbuf).context("geen samplebuffer")?) };

        // SAFETY: zonder async-vlag draait de callback op deze thread vóór terugkeer.
        let mut vlaggen = VTDecodeInfoFlags::empty();
        let status = unsafe {
            sessie.decode_frame(
                &sbuf,
                VTDecodeFrameFlags::empty(),
                std::ptr::null_mut(),
                &mut vlaggen,
            )
        };
        osstatus("VTDecompressionSessionDecodeFrame", status)?;

        let beeld = self
            .uitvoer
            .beeld
            .lock()
            .map_err(|_| anyhow!("decoderuitvoer vergrendeld door een panic"))?
            .take();
        if let Some(b) = &beeld {
            let maat = afmetingen(b);
            if self.afmeting != Some(maat) {
                tracing::info!(breedte = maat.0, hoogte = maat.1, "decoder levert beeld");
                self.afmeting = Some(maat);
            }
        }
        Ok(beeld)
    }

    fn bouw_sessie(&mut self) -> Result<()> {
        let sps = self.sps.as_deref().context("geen SPS")?;
        let pps = self.pps.as_deref().context("geen PPS")?;

        let mut desc: *const CMFormatDescription = std::ptr::null();
        let ptrs = [
            NonNull::new(sps.as_ptr() as *mut u8).context("lege SPS")?,
            NonNull::new(pps.as_ptr() as *mut u8).context("lege PPS")?,
        ];
        let maten = [sps.len(), pps.len()];
        // SAFETY: twee parametersets, viervoudige NAL-lengteprefix — het formaat dat
        // de AVCC-verpakking hierboven gebruikt.
        let status = unsafe {
            CMVideoFormatDescriptionCreateFromH264ParameterSets(
                None,
                2,
                NonNull::new(ptrs.as_ptr() as *mut NonNull<u8>).expect("stackpointer"),
                NonNull::new(maten.as_ptr() as *mut usize).expect("stackpointer"),
                4,
                NonNull::from(&mut desc),
            )
        };
        osstatus(
            "CMVideoFormatDescriptionCreateFromH264ParameterSets",
            status,
        )?;
        let formaat = unsafe {
            CFRetained::from_raw(
                NonNull::new(desc as *mut CMFormatDescription).context("geen formaat")?,
            )
        };

        let record = VTDecompressionOutputCallbackRecord {
            decompressionOutputCallback: Some(decode_klaar),
            decompressionOutputRefCon: &*self.uitvoer as *const DecoderUitvoer as *mut c_void,
        };
        // SAFETY: CMVideoFormatDescription is dezelfde opaque als CMFormatDescription.
        let video_desc: &CMVideoFormatDescription = unsafe {
            &*(&*formaat as *const CMFormatDescription as *const CMVideoFormatDescription)
        };
        let mut sessie: *mut VTDecompressionSession = std::ptr::null_mut();
        // SAFETY: BGRA als bestemming — geen aparte kleuromzetting nodig; de callback
        // en refcon leven zolang de sessie leeft.
        let status = unsafe {
            VTDecompressionSession::create(
                None,
                video_desc,
                None,
                Some(bgra_attrs().as_opaque()),
                &record,
                NonNull::from(&mut sessie),
            )
        };
        osstatus("VTDecompressionSessionCreate", status)?;
        self.sessie = Some(unsafe {
            CFRetained::from_raw(NonNull::new(sessie).context("VT gaf geen decodersessie")?)
        });
        self.formaat = Some(formaat);
        Ok(())
    }

    /// Afmeting van het beeld zoals de decoder het aflevert. `None` tot het eerste
    /// beeld binnen is.
    pub fn afmeting(&self) -> Option<(u32, u32)> {
        self.afmeting
    }

    /// VideoToolbox levert IOSurface-gebackte buffers; het beeld blijft dus buiten
    /// het gewone werkgeheugenpad, net als het GPU-pad op Windows.
    pub fn op_gpu(&self) -> bool {
        true
    }

    /// Gooit weg wat er nog in de decoder zit. De sessie gaat mee: het volgende
    /// keyframe draagt zijn eigen parametersets en bouwt hem opnieuw op — doorgaan op
    /// oude referentiebeelden levert toch alleen vlekken op.
    pub fn spoel(&mut self) {
        if let Some(s) = &self.sessie {
            // SAFETY: mag op elk moment tussen twee frames door.
            unsafe {
                let _ = s.wait_for_asynchronous_frames();
                s.invalidate();
            }
        }
        self.sessie = None;
        self.sps = None;
        self.pps = None;
        if let Ok(mut b) = self.uitvoer.beeld.lock() {
            *b = None;
        }
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        if let Some(s) = &self.sessie {
            // SAFETY: na invalidate komt er geen callback meer.
            unsafe { s.invalidate() };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Een effen vlak van één kleur, net als in de Windows-ketentest: de codec
    /// bewaart dat vrijwel verliesloos, dus de vergelijking is streng.
    fn effen(breedte: u32, hoogte: u32, b: u8, g: u8, r: u8) -> Vec<u8> {
        (0..breedte * hoogte).flat_map(|_| [b, g, r, 255]).collect()
    }

    #[test]
    fn payload_type_en_codec_zijn_elkaars_omkering() {
        for codec in [Codec::H264, Codec::Hevc] {
            assert_eq!(Codec::van_payload(codec.payload_type()), Some(codec));
        }
        assert_eq!(Codec::van_payload(fitcom_proto::PayloadType::OPUS), None);
    }

    #[test]
    fn annexb_en_avcc_zijn_elkaars_omkering() {
        let nal1 = [0x67u8, 1, 2, 3]; // SPS
        let nal2 = [0x41u8, 9, 8, 7, 6]; // beeld
        let mut avcc = Vec::new();
        avcc.extend_from_slice(&(nal1.len() as u32).to_be_bytes());
        avcc.extend_from_slice(&nal1);
        avcc.extend_from_slice(&(nal2.len() as u32).to_be_bytes());
        avcc.extend_from_slice(&nal2);

        let mut annexb = Vec::new();
        avcc_naar_annexb(&avcc, &mut annexb);
        let nals = annexb_nals(&annexb);
        assert_eq!(nals, vec![&nal1[..], &nal2[..]]);
        assert_eq!(nal_soort(nals[0]), 7);
    }

    #[test]
    fn driebyte_startcodes_worden_ook_gelezen() {
        let mut annexb = vec![0, 0, 1, 0x67, 1, 2];
        annexb.extend_from_slice(&[0, 0, 0, 1, 0x41, 3, 4]);
        let nals = annexb_nals(&annexb);
        assert_eq!(nals.len(), 2);
        assert_eq!(nal_soort(nals[0]), 7);
        assert_eq!(nal_soort(nals[1]), 1);
    }

    #[test]
    #[ignore = "vereist VideoToolbox-hardware"]
    fn beeld_overleeft_de_hele_keten() {
        // Mac-versie van de Windows-ketentest: coderen, via Annex-B terug, decoderen,
        // en de kleur in het midden controleren.
        const B: u32 = 1280;
        const H: u32 = 720;
        let (blauw, groen, rood) = (40u8, 140u8, 220u8);

        let d3d = D3dContext::new().expect("context");
        let bron = d3d
            .maak_textuur_met(B, H, &effen(B, H, blauw, groen, rood))
            .expect("bronbeeld");

        let cfg = EncoderConfig {
            codec: Codec::H264,
            breedte: B,
            hoogte: H,
            fps: 60,
            bitrate: 20_000_000,
        };
        let mut enc = Encoder::new(&d3d, &cfg).expect("encoder");
        enc.vraag_keyframe();
        let mut dec = Decoder::new(&d3d, Codec::H264, B, H).expect("decoder");

        let mut beeld = None;
        for i in 0..30 {
            let tijd = i as i64 * (HNS_PER_SEC / 60);
            for pakket in enc.encode(&bron, tijd).expect("encoderen") {
                if let Some(t) = dec
                    .decode(&pakket.data, pakket.tijd_hns)
                    .expect("decoderen")
                {
                    beeld = Some(t);
                }
            }
            if beeld.is_some() {
                break;
            }
        }

        let beeld = beeld.expect("er kwam geen enkel beeld uit de decoder");
        let (b, h, pixels) = d3d.lees_bgra(&beeld).expect("uitlezen");
        assert_eq!((b, h), (B, H), "afmeting veranderd onderweg");

        let midden = ((h / 2 * b + b / 2) * 4) as usize;
        let (gb, gg, gr) = (pixels[midden], pixels[midden + 1], pixels[midden + 2]);
        for (naam, wil, kreeg) in [
            ("blauw", blauw, gb),
            ("groen", groen, gg),
            ("rood", rood, gr),
        ] {
            let afwijking = (i32::from(wil) - i32::from(kreeg)).abs();
            assert!(
                afwijking <= 12,
                "{naam} wijkt {afwijking} af; kanalen omgewisseld of kleurbereik verkeerd"
            );
        }
    }
}
