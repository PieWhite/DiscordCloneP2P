//! De ringbuffer-recorder achter de clips (fase 15).
//!
//! ```text
//! scherm ─► WGC ─► D3D11 ─► eigen encoder ─► MP4-segmenten (~2 s) in een ringmap
//!                     systeemgeluid ─► AAC ─► tweede track per segment
//! hotkey ─► laatste N segmenten remuxen naar één afgespeelbare clip
//! ```
//!
//! Bewust een *eigen* opnameketen naast de deler, en geen SinkWriter: die bezit en
//! herstart zijn encoder per bestand, en een segmentovergang zou dan een MFT-reactivatie
//! betekenen midden in een draaiende game. Hier draait één encodersessie door en is elk
//! segment een zelfstandig, direct afspeelbaar MP4; de clip zelf is alleen een remux van
//! de laatste segmenten naar één nieuw bestand — sub-seconde, buiten het beeldpad om.
//!
//! H.264 en niet HEVC: de `mp4`-crate kan geen geldige `hvcC` schrijven (lege standaard-
//! box, geen VPS/SPS/PPS), dus HEVC-clips zouden nergens afspeelbaar zijn. H.264 speelt
//! overal af en is precies wat de kijkers van deze app toch al krijgen.
//!
//! # Tijdrekening
//!
//! Alles loopt op dezelfde klok als de deler ([`crate::deler::klok_nulpunt`]), zodat
//! video- en audiotijdstempels onderling vergelijkbaar zijn. Segmentbestanden dragen hun
//! absolute begintijd in de naam (`seg-{eerste_hns:020}.mp4`) — lexicaal sorteren ís
//! chronologisch sorteren, en na een herstart is de ring zo weer opgebouwd.
//!
//! # Crashveiligheid
//!
//! Elk segment wordt als `.part.mp4` geschreven en pas bij `sluit` hernoemd; een half
//! geschreven segment kan dus nooit tussen de levende metas zitten. Een bestand dat bij
//! het opstarten niet te lezen blijkt, wordt weggegooid in plaats van genegeerd — kapot
//! glas ruim je op, anders blijft het voor altijd liggen.
//!
//! # Segmentgrenzen
//!
//! Een segment wordt gesloten op een **geforceerd keyframe** ([`Encoder::vraag_keyframe`])
//! zodra hij ouder dan [`SEGMENT_DOEL_HNS`] is, en pas écht dicht als dat keyframe er
//! ook werkelijk doorheen komt (`CleanPoint`). Komt de IDR niet, dan loopt het segment
//! gewoon door tot de GOP-grens die de encoder bij het opzetten kreeg — een te lang
//! segment is nooit een corrupt segment.

use crate::capture::{Bron, BronSoort, Capture};
use crate::codec::{Codec, Encoder, EncoderConfig, HNS_PER_SEC};
use crate::d3d::D3dContext;
use anyhow::{anyhow, bail, Context, Result};
use bytes::Bytes;
use mp4::{
    AudioObjectType, AacConfig, AvcConfig, ChannelConfig, MediaConfig, Mp4Reader, Mp4Sample,
    Mp4Writer, SampleFreqIndex, TrackConfig, TrackType,
};
use std::collections::VecDeque;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Doorgemiddelde segmentlengte. Klein genoeg dat een clip strak op het verzoek aansluit,
/// groot genoeg dat per-segment-overhead (moov, keyframe) verwaarloosbaar is.
const SEGMENT_DOEL_HNS: i64 = 2 * HNS_PER_SEC;

/// Hoeveel extra boven het clipvenster de ring minimaal vasthoudt. Ruim boven één
/// segment, zodat het nieuwste — mogelijk nog openstaande — deel de telling nooit laat
/// kortvallen en een trage schijf nooit het oudste bewaarde beeld kost.
const RING_MARGE_HNS: i64 = 4 * HNS_PER_SEC;

/// Zelfde wachttijd als de deler gebruikt voor een stilstaand scherm: dat levert geen
/// beelden, en dat is geen fout.
const FRAME_WACHT: Duration = Duration::from_millis(100);

/// Video-tijdschaal in het MP4: honderd-nanoseconden-eenheden, exact onze eigen klok.
/// Sampleduren en starttijden zijn dan rechtstreeks hns en klopt de remux tot op het beeld.
const VIDEO_TIMESCALE: u32 = HNS_PER_SEC as u32;

/// AAC werkt in frames van 1024 samples — vastgelegt door de codec, geen keuze.
pub const AAC_FRAME_SAMPLES: u32 = 1024;

/// 192 kbit/s stereo: ruim boven wat bureaubladgeluid nodig heeft, en de hoogste
/// bytes-per-seconde die de inbox-AAC-encoder aanvaardt (24 000 × 8).
const AAC_BYTES_PER_SEC: u32 = 24_000;

// ---------------------------------------------------------------- zuivere helpers

/// Splitst een Annex-B-bytestream (startcodes) in losse NAL's, zonder codes. Valt ook
/// terug op "het geheel is één NAL" als er geen codes in zitten — precies wat de
/// sequentie-header-blob van een encoder soms blijkt te zijn.
pub fn splits_annexb(data: &[u8]) -> Vec<Vec<u8>> {
    let mut nals: Vec<Vec<u8>> = Vec::new();
    let mut start: Option<usize> = None;
    let mut i = 0usize;
    while i + 2 < data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            if let Some(s) = start {
                // Nullen vóór deze code horen bij de code zelf, niet bij de NAL ervoor.
                let mut e = i;
                while e > s && data[e - 1] == 0 {
                    e -= 1;
                }
                nals.push(data[s..e].to_vec());
            }
            start = Some(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    // Geen enkele startcode gevonden: dan is het geheel één NAL — precies hoe een
    // sequentie-header-blob zonder codes eruitziet. Niets bijten, niets weglaten.
    let Some(s) = start else {
        return if data.is_empty() {
            Vec::new()
        } else {
            vec![data.to_vec()]
        };
    };
    nals.push(data[s..].to_vec());
    nals.retain(|n| !n.is_empty());
    nals
}

/// Annex-B → AVCC: elke NAL krijgt een big-endian lengte-prefix in plaats van een
/// startcode. Dit is de vorm die in een MP4-monster hoort.
pub fn naar_avcc(data: &[u8]) -> Vec<u8> {
    let mut uit = Vec::with_capacity(data.len() + 16);
    for nal in splits_annexb(data) {
        uit.extend_from_slice(&(nal.len() as u32).to_be_bytes());
        uit.extend_from_slice(&nal);
    }
    uit
}

/// SPS (NAL-type 7) en PPS (type 8) uit een lijst NAL's — de twee parameter sets die de
/// `avcC`-box van elk segment nodig heeft.
pub fn parameter_sets_uit_nals(nals: &[Vec<u8>]) -> Option<(Vec<u8>, Vec<u8>)> {
    let sps = nals.iter().find(|n| !n.is_empty() && (n[0] & 0x1F) == 7)?;
    let pps = nals.iter().find(|n| !n.is_empty() && (n[0] & 0x1F) == 8)?;
    Some((sps.clone(), pps.clone()))
}

/// Parameter sets zoeken in een bytestroom die óf de sequentie-header-blob van de
/// encoder is, óf het eerste keyframe. Beide kunnen met of zonder startcodes komen;
/// [`splits_annexb`] behandelt allebei hetzelfde.
pub fn parameter_sets(data: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    parameter_sets_uit_nals(&splits_annexb(data))
}

/// AAC-sampling-frequentie-index zoals ADTS en de esds hem allebei gebruiken.
fn freq_index_van(sample_rate: u32) -> Option<u8> {
    Some(match sample_rate {
        96_000 => 0,
        88_200 => 1,
        64_000 => 2,
        48_000 => 3,
        44_100 => 4,
        32_000 => 5,
        24_000 => 6,
        22_050 => 7,
        16_000 => 8,
        12_000 => 9,
        11_025 => 10,
        8_000 => 11,
        _ => return None,
    })
}

/// ADTS-header (7 bytes, protection_absent): maakt van een rauw AAC-frame een
/// zelfstandig herkenbaar frame. Het MP4 zelf heeft dit niet nodig — daar staat de
/// configuratie in de esds — maar tests en foutopsporing willen het graag zien.
#[cfg(test)]
fn adts_header(lengte_inclusief: usize, freq_index: u8, chan_conf: u8) -> [u8; 7] {
    let profiel = 1u32; // veldwaarde = objecttype − 1, AAC-LC is type 2
    let lengte = lengte_inclusief as u32;
    [
        0xFF,
        0xF1, // MPEG-4, layer 0, protection_absent
        ((profiel << 6) | (u32::from(freq_index) << 2) | (u32::from(chan_conf) >> 2)) as u8,
        (((u32::from(chan_conf) & 0x03) << 6) | ((lengte >> 11) & 0x03)) as u8,
        ((lengte >> 3) & 0xFF) as u8,
        (((lengte & 0x07) << 5) | 0x1F) as u8,
        0xFC,
    ]
}

// ---------------------------------------------------------------- segmentadministratie

/// Eén afgesloten segment in de ring, met zijn absolute tijdsbereik in hns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentMeta {
    pub pad: PathBuf,
    pub eerste_hns: i64,
    pub laatste_hns: i64,
}

/// Welke segmenten samen tenminste `venster_hns` beslaan, gerekend vanaf het nieuwste.
/// Leeg als er nog niets is; anders minstens één.
pub fn kies_venster(metas: &[SegmentMeta], venster_hns: i64) -> Vec<SegmentMeta> {
    let mut gekozen: Vec<SegmentMeta> = Vec::new();
    let mut span = 0i64;
    for m in metas.iter().rev() {
        gekozen.push(m.clone());
        span += m.laatste_hns - m.eerste_hns;
        if span >= venster_hns {
            break;
        }
    }
    gekozen.reverse();
    gekozen
}

/// Welke segmenten ouder zijn dan de ring mag houden: alles dat eindigt vóór
/// `(nieuwste_einde − venster − marge)`. Puur functie, zodat het beleid testbaar is.
pub fn te_gooien(metas: &[SegmentMeta], houdt_hns: i64) -> Vec<PathBuf> {
    let Some(nieuwste) = metas.iter().map(|m| m.laatste_hns).max() else {
        return Vec::new();
    };
    let grens = nieuwste.saturating_sub(houdt_hns + RING_MARGE_HNS);
    metas
        .iter()
        .filter(|m| m.laatste_hns < grens)
        .map(|m| m.pad.clone())
        .collect()
}

/// Bouwt de ring opnieuw op uit de map: naam geeft de begintijd, de inhoud zegt hoe
/// laat het laatste beeld was. Onleesbare bestanden worden weggegooid — zie de moduledoc.
pub fn laad_segmenten(ring_dir: &Path) -> Vec<SegmentMeta> {
    let Ok(entries) = std::fs::read_dir(ring_dir) else {
        return Vec::new();
    };
    let mut metas = Vec::new();
    for entry in entries.flatten() {
        let pad = entry.path();
        let Some(naam) = pad.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(rest) = naam.strip_prefix("seg-").and_then(|r| r.strip_suffix(".mp4")) else {
            continue;
        };
        let Ok(eerste) = rest.parse::<i64>() else {
            continue;
        };
        match laatste_monster_hns(&pad) {
            Ok(rel_einde) => metas.push(SegmentMeta {
                pad,
                eerste_hns: eerste,
                laatste_hns: eerste + rel_einde,
            }),
            Err(e) => {
                tracing::warn!(pad = %pad.display(), error = %format!("{e:#}"), "kapot segment weggegooid");
                let _ = std::fs::remove_file(&pad);
            }
        }
    }
    metas.sort_by_key(|m| m.eerste_hns);
    metas
}

/// Eindtijd van het laatste videomonster in een segmentbestand, relatief aan het begin
/// van het bestand (start_time + duration van monster N).
fn laatste_monster_hns(pad: &Path) -> Result<i64> {
    let f = File::open(pad)?;
    let len = f.metadata()?.len();
    let mut r = Mp4Reader::read_header(f, len)?;
    let aantal = r.sample_count(1)?;
    let s = r
        .read_sample(1, aantal)?
        .context("videotrack zonder monsters")?;
    Ok(i64::try_from(s.start_time)? + i64::from(s.duration))
}

// ---------------------------------------------------------------- spoortypes

/// Videotrack-configuratie zoals elk segment (én de clip) hem schrijft. Vastgezet op het
/// moment dat het eerste segment opent; binnen één opnamesessie verandert er niets meer.
#[derive(Debug, Clone)]
pub struct VideoSpoor {
    pub sps: Vec<u8>,
    pub pps: Vec<u8>,
    pub breedte: u32,
    pub hoogte: u32,
    pub fps: u32,
}

#[derive(Debug, Clone)]
pub struct AudioSpoor {
    pub sample_rate: u32,
    pub kanalen: u32,
    pub bitrate: u32,
}

impl AudioSpoor {
    fn freq_index(&self) -> Result<u8> {
        freq_index_van(self.sample_rate).context("samplerate heeft geen AAC-frequentie-index")
    }

    fn chan_conf(&self) -> Result<ChannelConfig> {
        ChannelConfig::try_from(self.kanalen as u8)
            .map_err(|_| anyhow!("kanalen={} past niet in AAC", self.kanalen))
    }

    fn timescale(&self) -> u32 {
        self.sample_rate
    }
}

// ---------------------------------------------------------------- segmentschrijver

/// Schrijft één segment: een zelfstandig MP4 met video (track 1) en optioneel audio
/// (track 2).
///
/// De duur van een videomonster is pas bekend als het volgende binnenkomt, dus het
/// openstaande monster wordt bewaard en met de échte duur weggeschreven zodra die er is;
/// `sluit` geeft het allerlaatste de GOP-duur. Audiomonsters hebben hun duur in zich
/// (1024 samples, vast) en gaan direct de writer in.
struct SegmentSchrijver {
    writer: Mp4Writer<File>,
    pad: PathBuf,
    /// Absoluut nulpunt van dit segment, op onze procesklok.
    eerste_hns: i64,
    laatste_hns: i64,
    frame_duur: i64,
    /// `None` zolang er nog geen audiospoor in zit; dan doet [`Self::schrijf_audio`]
    /// niets. Anders de MP4-timescale van de audiotrack (de samplerate).
    audio_timescale: u32,
    openstaande: Option<(u64, bool, Bytes)>,
}

impl SegmentSchrijver {
    fn open(
        pad: PathBuf,
        video: &VideoSpoor,
        audio: Option<&AudioSpoor>,
        eerste_hns: i64,
    ) -> Result<Self> {
        let config = mp4::Mp4Config {
            major_brand: "isom".parse().context("mp4-brand")?,
            minor_version: 512,
            compatible_brands: vec![
                "isom".parse().unwrap(),
                "iso2".parse().unwrap(),
                "avc1".parse().unwrap(),
                "mp41".parse().unwrap(),
            ],
            timescale: VIDEO_TIMESCALE,
        };
        // Temp-naam tijdens het schrijven; pas `sluit` geeft het bestand zijn echte
        // naam. Zo is een halfgeschreven segment herkenbaar en wegwerpbaar.
        let temp = pad.with_extension("part.mp4");
        let file = File::create(&temp)
            .with_context(|| format!("segment maken: {}", temp.display()))?;
        let mut writer = Mp4Writer::write_start(file, &config)
            .with_context(|| format!("MP4-writer starten: {}", temp.display()))?;

        writer
            .add_track(&TrackConfig {
                track_type: TrackType::Video,
                timescale: VIDEO_TIMESCALE,
                language: "und".into(),
                media_conf: MediaConfig::AvcConfig(AvcConfig {
                    width: video.breedte as u16,
                    height: video.hoogte as u16,
                    seq_param_set: video.sps.clone(),
                    pic_param_set: video.pps.clone(),
                }),
            })
            .context("videospoor toevoegen")?;

        let audio_timescale = match audio {
            Some(a) => {
                writer
                    .add_track(&TrackConfig {
                        track_type: TrackType::Audio,
                        timescale: a.timescale(),
                        language: "und".into(),
                        media_conf: MediaConfig::AacConfig(AacConfig {
                            bitrate: a.bitrate,
                            profile: AudioObjectType::AacLowComplexity,
                            freq_index: SampleFreqIndex::try_from(a.freq_index()?)
                                .map_err(|e| anyhow!("frequentie-index: {e}"))?,
                            chan_conf: a.chan_conf()?,
                        }),
                    })
                    .context("audiospoor toevoegen")?;
                a.timescale()
            }
            None => 0,
        };

        Ok(Self {
            writer,
            pad,
            eerste_hns,
            laatste_hns: eerste_hns,
            frame_duur: HNS_PER_SEC / i64::from(video.fps.max(1)),
            audio_timescale,
            openstaande: None,
        })
    }

    /// Beeld toevoegen. `data` moet AVCC zijn (zie [`naar_avcc`]).
    fn schrijf_video(&mut self, abs_hns: i64, keyframe: bool, data: Vec<u8>) {
        let rel = (abs_hns - self.eerste_hns).max(0) as u64;
        if let Some((vorige_rel, sync, bytes)) = self.openstaande.take() {
            let duur = rel.saturating_sub(vorige_rel).clamp(1, u32::MAX as u64) as u32;
            let _ = self.writer.write_sample(
                1,
                &Mp4Sample {
                    start_time: vorige_rel,
                    duration: duur,
                    rendering_offset: 0,
                    is_sync: sync,
                    bytes,
                },
            );
        }
        self.openstaande = Some((rel, keyframe, Bytes::from(data)));
        self.laatste_hns = self.laatste_hns.max(abs_hns);
    }

    /// Eén AAC-frame op de audiotrack. `abs_hns` is de absolute begintijd op de
    /// procesklok; `dur_samples` is vrijwel altijd [`AAC_FRAME_SAMPLES`].
    ///
    /// Frames die vóór dit segment beginnen horen thuis in de voorganger — hier
    /// stilletjes weggooien is dus correct, mits de caller ze eerst aan die voorganger
    /// heeft aangeboden. Dat doet de opnamelus: hij schrijft de wachtrij weg vóórdat
    /// hij het segment dichttrekt.
    fn schrijf_audio(&mut self, abs_hns: i64, dur_samples: u32, data: Vec<u8>) {
        if self.audio_timescale == 0 {
            return;
        }
        let Some(delta) = abs_hns.checked_sub(self.eerste_hns) else {
            return;
        };
        if delta < 0 {
            return;
        }
        let start_ticks =
            (i128::from(delta) * i128::from(self.audio_timescale) / i128::from(HNS_PER_SEC)) as u64;
        let _ = self.writer.write_sample(
            2,
            &Mp4Sample {
                start_time: start_ticks,
                duration: dur_samples,
                rendering_offset: 0,
                is_sync: true,
                bytes: Bytes::from(data),
            },
        );
    }

    /// Rondt af: laatste beeld eruit, moov schrijven, temp → definitieve naam.
    fn sluit(mut self) -> Result<SegmentMeta> {
        if let Some((rel, sync, bytes)) = self.openstaande.take() {
            let _ = self.writer.write_sample(
                1,
                &Mp4Sample {
                    start_time: rel,
                    duration: self.frame_duur as u32,
                    rendering_offset: 0,
                    is_sync: sync,
                    bytes,
                },
            );
        }
        self.writer.write_end().context("moov schrijven")?;
        std::fs::rename(temp_naam(&self.pad), &self.pad).with_context(|| {
            format!(
                "segment hernoemen: {} → {}",
                temp_naam(&self.pad).display(),
                self.pad.display()
            )
        })?;
        Ok(SegmentMeta {
            pad: self.pad.clone(),
            eerste_hns: self.eerste_hns,
            laatste_hns: self.laatste_hns,
        })
    }
}

/// Tijdelijke naam van een segment of clip die nog aan het schrijven is.
fn temp_naam(definitief: &Path) -> PathBuf {
    definitief.with_extension("part.mp4")
}

// ---------------------------------------------------------------- remux

/// Plakt de gekozen segmenten aan elkaar tot één afgespeelbare clip.
///
/// De basis is het eerste keyframe op of ná de vensterrand — niet de rand zelf, want
/// een clip die ergens middenin begint decodeert tot het volgende IDR niet. Audiomonsters
/// die vóór dat keyframe beginnen vallen af: hooguit één AAC-frame (≈21 ms) verschil,
/// ruimschoots onder wat lip-sync waarneembaar maakt.
///
/// Geschreven wordt naar een `.part.mp4` die pas bij succes de definitieve naam krijgt.
pub fn plak_clip(
    metas: &[SegmentMeta],
    venster_hns: i64,
    uit_pad: &Path,
    video: &VideoSpoor,
    audio: Option<&AudioSpoor>,
) -> Result<()> {
    if metas.is_empty() {
        bail!("geen segmenten om te plakken");
    }
    let einde = metas.last().map(|m| m.laatste_hns).unwrap_or(0);
    let vensterrand = einde.saturating_sub(venster_hns);
    let basis = basis_tijd(metas, vensterrand)?;

    let resultaat = (|| -> Result<()> {
        let mut schrijver = SegmentSchrijver::open(uit_pad.to_path_buf(), video, audio, basis)?;

        for meta in metas {
            let f = File::open(&meta.pad).with_context(|| meta.pad.display().to_string())?;
            let len = f.metadata()?.len();
            let mut r = Mp4Reader::read_header(f, len)
                .with_context(|| format!("segment onleesbaar: {}", meta.pad.display()))?;

            // Video (track 1). Tijden zijn al hns — zelfde timescale als de clip.
            let n = r.sample_count(1)?;
            for id in 1..=n {
                let Some(s) = r.read_sample(1, id)? else { continue };
                let abs = meta.eerste_hns + i64::try_from(s.start_time)?;
                if abs < basis {
                    continue;
                }
                schrijver.schrijf_video(abs, s.is_sync, s.bytes.to_vec());
            }

            // Audio (track 2), alleen als hij erin zit én we hem meeschrijven.
            let Some(a) = audio else { continue };
            if !r.tracks().contains_key(&2) {
                continue;
            }
            let n = r.sample_count(2)?;
            for id in 1..=n {
                let Some(s) = r.read_sample(2, id)? else { continue };
                let abs = meta.eerste_hns
                    + (i128::from(s.start_time) * i128::from(HNS_PER_SEC)
                        / i128::from(a.timescale())) as i64;
                if abs < basis {
                    continue;
                }
                schrijver.schrijf_audio(abs, s.duration, s.bytes.to_vec());
            }
        }

        schrijver.sluit()?;
        Ok(())
    })();

    match resultaat {
        Ok(()) => Ok(()),
        Err(e) => {
            // Een halve clip is geen clip: weg ermee, de gebruiker probeert gewoon nog
            // een keer en dan staat de ring er netjes bij.
            let _ = std::fs::remove_file(temp_naam(uit_pad));
            Err(e)
        }
    }
}

/// Het eerste keyframe op of na de vensterrand. Loopt de segmenten vooruit; in het
/// eerste segment dat de rand raakt zoekt hij het eerste sync-monster, tenzij dat hele
/// segment al ná de rand ligt — dan is zijn eigen beginscène de basis.
fn basis_tijd(metas: &[SegmentMeta], w: i64) -> Result<i64> {
    for meta in metas {
        if meta.laatste_hns < w {
            continue;
        }
        let f = File::open(&meta.pad).with_context(|| meta.pad.display().to_string())?;
        let len = f.metadata()?.len();
        let mut r = Mp4Reader::read_header(f, len)?;
        let n = r.sample_count(1)?;
        for id in 1..=n {
            let Some(s) = r.read_sample(1, id)? else { continue };
            if !s.is_sync {
                continue;
            }
            let abs = meta.eerste_hns + i64::try_from(s.start_time)?;
            if abs >= w || meta.eerste_hns >= w {
                return Ok(abs);
            }
        }
    }
    // Onbereikbaar zolang het nieuwste segment op een keyframe begint en op of ná
    // elke denkbare rand ligt — maar bail in plaats van gokken.
    bail!("geen keyframe gevonden binnen het venster")
}

// ---------------------------------------------------------------- AAC-encoder

/// De inbox-AAC-encoder van Windows (software, sync-MFT). PCM s16 interleaved erin,
/// rauwe AAC-frames eruit — rauw wil zeggen zonder ADTS-header, want in een MP4 zit de
/// configuratie in de esds, niet per frame.
///
/// Bewust software en bewust deze encoder: hij is overal op Windows aanwezig, kost bij
/// stereo-48kHz verwaarloosbaar weinig CPU naast de hardware-videoencoder, en een
/// hardware-AAC-MFT bestaat op consumenten-GPU's sowieso niet.
pub struct AacEncoder {
    transform: windows::Win32::Media::MediaFoundation::IMFTransform,
    uitvoer_grootte: usize,
}

// SAFETY: sync-MFT, uitsluitend vanaf de opnamedraad aangeroepen; zelfde redenering als
// bij de video-encoder.
unsafe impl Send for AacEncoder {}

impl AacEncoder {
    pub fn new(sample_rate: u32, kanalen: u32) -> Result<Self> {
        crate::mf::zorg_dat_mf_draait();
        let activates = crate::mf::zoek_audio_encoder(
            windows::Win32::Media::MediaFoundation::MFAudioFormat_PCM,
            windows::Win32::Media::MediaFoundation::MFAudioFormat_AAC,
        )
        .context("AAC-encoders opzoeken")?;
        let activate = activates.first().context("geen inbox-AAC-encoder gevonden")?;
        tracing::info!(encoder = %crate::mf::naam_van(activate), "AAC-encoder gekozen");

        // SAFETY: activate komt uit MFTEnumEx en is geldig.
        let transform: windows::Win32::Media::MediaFoundation::IMFTransform =
            unsafe { activate.ActivateObject().context("AAC-encoder activeren")? };

        // Uitvoertype eerst, invoertype daarna — zelfde volgorde als bij de video-
        // encoder, om dezelfde reden: de encoder wil weten wat hij moet maken.
        unsafe {
            use windows::Win32::Media::MediaFoundation::*;
            let uit = MFCreateMediaType().context("AAC-uitvoertype maken")?;
            uit.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
            uit.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC)?;
            uit.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
            uit.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
            uit.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, sample_rate)?;
            uit.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, kanalen)?;
            uit.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, AAC_BYTES_PER_SEC)?;
            uit.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, 1)?;
            uit.SetUINT32(&MF_MT_AAC_PAYLOAD_TYPE, 0)?; // rauw, zonder ADTS
            transform
                .SetOutputType(0, &uit, 0)
                .context("AAC-uitvoertype instellen")?;

            let invoer = MFCreateMediaType().context("PCM-invoertype maken")?;
            invoer.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
            invoer.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)?;
            invoer.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
            invoer.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, sample_rate)?;
            invoer.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, kanalen)?;
            invoer.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, kanalen * 2)?;
            invoer.SetUINT32(
                &MF_MT_AUDIO_AVG_BYTES_PER_SECOND,
                sample_rate * kanalen * 2,
            )?;
            transform
                .SetInputType(0, &invoer, 0)
                .context("PCM-invoertype instellen")?;

            let info = transform
                .GetOutputStreamInfo(0)
                .context("streaminfo van de AAC-encoder")?;
            Ok(Self {
                transform,
                uitvoer_grootte: (info.cbSize as usize).max(4096),
            })
        }
    }

    /// PCM s16 interleaved erin, op de gezamenlijke procesklok. Elke invoer-chunk móét
    /// tijd én duur meedragen — een duur van nul triggert een gedocumenteerde deling
    /// door nul dieper in Media Foundation, en zonder tijd komt er überhaupt niets uit.
    pub fn voer(
        &mut self,
        pcm: &[i16],
        tijd_hns: i64,
        duur_hns: i64,
    ) -> Result<Vec<(i64, i64, Vec<u8>)>> {
        if pcm.is_empty() {
            return Ok(Vec::new());
        }
        unsafe {
            use windows::Win32::Media::MediaFoundation::*;

            let sample: IMFSample = MFCreateSample().context("PCM-sample maken")?;
            let buffer: IMFMediaBuffer =
                MFCreateMemoryBuffer((pcm.len() * 2) as u32).context("PCM-buffer maken")?;
            let mut ptr: *mut u8 = std::ptr::null_mut();
            buffer.Lock(&mut ptr, None, None).context("PCM-buffer lock")?;
            std::ptr::copy_nonoverlapping(pcm.as_ptr() as *const u8, ptr, pcm.len() * 2);
            buffer.Unlock()?;
            buffer.SetCurrentLength((pcm.len() * 2) as u32)?;
            sample.AddBuffer(&buffer)?;
            sample.SetSampleTime(tijd_hns)?;
            sample.SetSampleDuration(duur_hns)?;
            self.transform
                .ProcessInput(0, &sample, 0)
                .context("PCM invoeren in de AAC-encoder")?;
        }
        self.leeg_uitlezen()
    }

    /// Trekt de uitvoer leeg tot de encoder om nieuw invoer vraagt.
    fn leeg_uitlezen(&mut self) -> Result<Vec<(i64, i64, Vec<u8>)>> {
        use windows::Win32::Media::MediaFoundation::*;
        let mut uit = Vec::new();
        loop {
            // Eigen monster met eigen buffer meegeven: deze sync-MFT levert zelf geen
            // monsters aan (geen PROVIDES_SAMPLES-vlag).
            let sample: IMFSample = unsafe { MFCreateSample()? };
            unsafe {
                let b: IMFMediaBuffer = MFCreateMemoryBuffer(self.uitvoer_grootte as u32)?;
                sample.AddBuffer(&b)?;
            }

            let mut buffers = [MFT_OUTPUT_DATA_BUFFER {
                dwStreamID: 0,
                pSample: std::mem::ManuallyDrop::new(Some(sample)),
                pEvents: std::mem::ManuallyDrop::new(None),
                dwStatus: 0,
            }];
            let mut status = 0u32;
            // SAFETY: buffers is een geldige array van één uitvoerbuffer; de MFT is
            // netjes gesynchroniseerd (sync-MFT, één thread).
            let proces = unsafe { self.transform.ProcessOutput(0, &mut buffers, &mut status) };
            match proces {
                Ok(_) => {}
                Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => break,
                Err(e) => return Err(e).context("AAC-uitvoer halen"),
            }

            let monster = unsafe { std::mem::ManuallyDrop::take(&mut buffers[0].pSample) }
                .context("AAC-encoder leverde geen monster")?;
            let tijd = unsafe { monster.GetSampleTime() }.unwrap_or(0);
            let duur = unsafe { monster.GetSampleDuration() }.unwrap_or(0);
            unsafe {
                let buffer: IMFMediaBuffer = monster.ConvertToContiguousBuffer()?;
                let mut ptr: *mut u8 = std::ptr::null_mut();
                let mut lengte = 0u32;
                buffer.Lock(&mut ptr, None, Some(&mut lengte))?;
                uit.push((
                    tijd,
                    duur,
                    std::slice::from_raw_parts(ptr, lengte as usize).to_vec(),
                ));
                buffer.Unlock()?;
            }
        }
        Ok(uit)
    }
}

// ---------------------------------------------------------------- de opnamelus zelf

/// Instellingen voor één opnamesessie.
#[derive(Debug, Clone)]
pub struct ClipInstellingen {
    pub fps: u32,
    pub bitrate: u32,
    /// Hoeveel seconden een clip teruggaat.
    pub venster_sec: u32,
}

impl ClipInstellingen {
    fn venster_hns(&self) -> i64 {
        i64::from(self.venster_sec.max(1)) * HNS_PER_SEC
    }
    fn houdt_hns(&self) -> i64 {
        self.venster_hns()
    }
}

/// Geluid voor in de clip, al genormaliseerd naar 48 kHz stereo door de taps zelf.
/// Beide bronnen zijn optioneel — een clip zonder geluid is beter dan helemaal geen
/// clip, en welke bron er ook faalt: de videoketen draait gewoon door.
#[derive(Default)]
pub struct AudioBronnen {
    /// Systeem- en spelgeluid via de proces-exclusieve loopback-tap.
    pub systeem: Option<std::sync::mpsc::Receiver<Vec<f32>>>,
    /// De eigen microfoon via de microfoon-tap.
    pub microfoon: Option<std::sync::mpsc::Receiver<Vec<f32>>>,
}

impl AudioBronnen {
    /// Zit er überhaupt een bron bij?
    pub fn heeft_bron(&self) -> bool {
        self.systeem.is_some() || self.microfoon.is_some()
    }
}

/// Mengt beide geluidsbronnen op één tijdlijn. Bronnen leveren hun chunks met een
/// absolute begintijd (hun eigen klok, gestart op het begin van de sessie); de buffer
/// is interleaved stereo vanaf `basis_hns`. Alles wat álle bronnen tot en met hebben
/// geleverd mag veilig naar de AAC-encoder — daárvoor kan een bron nog terugkomen met
/// een monster dat eerder begint, en dat moet dan juist tussengevoegd kunnen worden.
struct Menger {
    basis_hns: i64,
    buffer: Vec<f32>,
    gecodeerd_tot_frame: i64,
    klok: [Option<i64>; 2],
    bron_einde_frame: [Option<i64>; 2],
}

impl Menger {
    fn voeg_toe(&mut self, bron: usize, start_abs_hns: i64, data: &[f32]) {
        if data.is_empty() {
            return;
        }
        let frames = i64::try_from(data.len()).unwrap_or(0) / 2;
        let offset_frames =
            ((start_abs_hns - self.basis_hns).max(0)) * 48_000 / HNS_PER_SEC;
        let nodig = (offset_frames as usize + frames as usize) * 2;
        if self.buffer.len() < nodig {
            self.buffer.resize(nodig, 0.0);
        }
        let base_i = offset_frames as usize * 2;
        for (i, s) in data.iter().enumerate() {
            self.buffer[base_i + i] += *s;
        }
        let einde = offset_frames + frames;
        self.bron_einde_frame[bron] = Some(match self.bron_einde_frame[bron] {
            Some(e) => e.max(einde),
            None => einde,
        });
    }

    /// Tot welk frame hebben álle aanwezige bronnen geleverd? Daarvóór kan niet
    /// gecodeerd worden: een latere chunk van een andere bron mag er nog in mengen.
    fn veilig_tot_frame(&self) -> i64 {
        self.bron_einde_frame.iter().flatten().copied().min().unwrap_or(0)
    }
}

/// Wat een clip-poging opleverde, via een kanaal naar de motor.
#[derive(Debug)]
pub enum ClipGebeurtenis {
    Klaar { pad: PathBuf },
    Mislukt { reden: String },
}

#[derive(Default)]
struct ClipStatus {
    stop: AtomicBool,
    gestopt: AtomicBool,
    bewaar: AtomicBool,
    fout: Mutex<Option<String>>,
}

/// Handvat op de lopende opname. Vallen laten stopt de keten netjes achteraan.
pub struct OpnameHandle {
    staat: Arc<ClipStatus>,
}

impl OpnameHandle {
    /// Vraagt om nú de laatste `venster_sec` als clip weg te schrijven.
    pub fn bewaar_nu(&self) {
        self.staat.bewaar.store(true, Ordering::Relaxed);
    }

    /// Of de keten eruit is — dan doet deze handle niets meer.
    pub fn gestopt(&self) -> bool {
        self.staat.gestopt.load(Ordering::Relaxed)
    }

    /// Waarom hij eruit is, als dat zo is.
    pub fn fout(&self) -> Option<String> {
        self.staat.fout.lock().ok().and_then(|f| f.clone())
    }
}

impl Drop for OpnameHandle {
    fn drop(&mut self) {
        self.staat.stop.store(true, Ordering::Relaxed);
        // Kort wachten zodat het lopende segment nog dicht kan. Een scherm is geen
        // exclusieve bron; dit is service richting de ring, geen noodzaak.
        let tot = Instant::now() + Duration::from_millis(1000);
        while !self.staat.gestopt.load(Ordering::Relaxed) && Instant::now() < tot {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

/// Zet de opnameketen aan. De draad leeft tot de handle valt of de gebruiker hem
/// uitzet; fouten landen op de handle en in de log, niet in een paniek.
#[allow(clippy::too_many_arguments)]
pub fn start_opname(
    d3d: &D3dContext,
    bron: &Bron,
    instellingen: ClipInstellingen,
    ring_dir: PathBuf,
    clips_dir: PathBuf,
    audio: Option<AudioBronnen>,
    gebeurtenissen: std::sync::mpsc::Sender<ClipGebeurtenis>,
) -> Result<OpnameHandle> {
    if !matches!(bron.soort, BronSoort::Monitor | BronSoort::Venster) {
        bail!("clips komen van een scherm of venster, niet van een camera");
    }
    std::fs::create_dir_all(&ring_dir)?;
    std::fs::create_dir_all(&clips_dir)?;

    let staat = Arc::new(ClipStatus::default());
    let d3d = d3d.clone();
    let bron = bron.clone();
    let staat_lus = staat.clone();
    std::thread::Builder::new()
        .name("fitcom-opname".into())
        .spawn(move || {
            let resultaat = opname_lus(
                &d3d,
                &bron,
                &instellingen,
                &ring_dir,
                &clips_dir,
                audio.as_ref(),
                gebeurtenissen,
                &staat_lus,
            );
            if let Err(e) = resultaat {
                tracing::error!(
                    error = %format!("{e:#}"),
                    "clip-opname gestopt door een fout"
                );
                if let Ok(mut f) = staat_lus.fout.lock() {
                    *f = Some(format!("{e:#}"));
                }
            }
            staat_lus.gestopt.store(true, Ordering::Relaxed);
        })
        .context("opnamedraad starten")?;
    Ok(OpnameHandle { staat })
}

fn nu_hns(begin: Instant) -> i64 {
    (begin.elapsed().as_nanos() / 100) as i64
}

/// Minimaal fps-tempo: beelden die sneller binnenkomen dan de gekozen framerate worden
/// overgeslagen, precies zoals de deler dat doet.
struct Tempo {
    interval: Duration,
    volgende: Instant,
}

impl Tempo {
    fn nieuw(fps: u32) -> Self {
        Self {
            interval: Duration::from_nanos(1_000_000_000 / u64::from(fps.max(1))),
            volgende: Instant::now(),
        }
    }
    fn laat_door(&mut self, nu: Instant) -> bool {
        if nu >= self.volgende {
            self.volgende = (self.volgende + self.interval).max(nu);
            true
        } else {
            false
        }
    }
}

fn ruim_ring_op(metas: &mut Vec<SegmentMeta>, houdt_hns: i64) {
    let weg = te_gooien(metas, houdt_hns);
    for pad in weg {
        tracing::debug!(pad = %pad.display(), "oud segment weggegooid");
        let _ = std::fs::remove_file(&pad);
    }
    metas.retain(|m| m.pad.exists());
}

#[allow(clippy::too_many_arguments)]
fn opname_lus(
    d3d: &D3dContext,
    bron: &Bron,
    instellingen: &ClipInstellingen,
    ring_dir: &Path,
    clips_dir: &Path,
    audio: Option<&AudioBronnen>,
    gebeurtenissen: std::sync::mpsc::Sender<ClipGebeurtenis>,
    staat: &ClipStatus,
) -> Result<()> {
    let mut capture = Capture::start(d3d, bron)?;
    let (breedte, hoogte) = capture.afmeting();

    let mut encoder = Encoder::new(
        d3d,
        &EncoderConfig {
            codec: Codec::H264,
            breedte,
            hoogte,
            fps: instellingen.fps,
            bitrate: instellingen.bitrate,
        },
    )
    .context("clip-encoder opzetten")?;

    // Geluid: AAC op vaste 48 kHz stereo. Zonder bronnen geen encoder — en zonder
    // encoder geen geluid, maar wel gewoon clips.
    let mut aac = match audio.filter(|a| a.heeft_bron()) {
        Some(_) => match AacEncoder::new(48_000, 2) {
            Ok(e) => Some(e),
            Err(e) => {
                tracing::warn!(
                    error = %format!("{e:#}"),
                    "AAC-encoder mislukt; clips zonder geluid"
                );
                None
            }
        },
        None => None,
    };
    let mut wachtrij: VecDeque<(i64, i64, Vec<u8>)> = VecDeque::new();

    let mut sporen: Option<(VideoSpoor, Option<AudioSpoor>)> = None;
    let mut segment: Option<SegmentSchrijver> = None;
    let mut metas = laad_segmenten(ring_dir);
    ruim_ring_op(&mut metas, instellingen.houdt_hns());

    let mut sluiten_gevraagd = false;
    let mut keyframe_gevraagd = false;
    let mut nulpunten: Option<(i64, i64)> = None;
    let mut vorige_tijd: i64 = -1;
    let mut tempo = Tempo::nieuw(instellingen.fps);
    let begin = crate::deler::klok_nulpunt();
    // Basis van de geluidstijdlijn: het begin van deze sessie. Beelden vóór het eerste
    // keyframe komen toch niet in een segment, dus die marge is gratis.
    let mut menger = aac.as_ref().map(|_| Menger {
        basis_hns: 0,
        buffer: Vec::new(),
        gecodeerd_tot_frame: 0,
        klok: [None; 2],
        bron_einde_frame: [None; 2],
    });

    loop {
        if staat.stop.load(Ordering::Relaxed) {
            break;
        }

        // Geluid binnenhalen uit beide bronnen en op één tijdlijn mengen. Elke bron
        // houdt zijn eigen klok bij (absoluut, vanaf sessiestart); de menger somt de
        // monsters op dezelfde plek op, en wat álle bronnen gevuld hebben mag naar de
        // encoder.
        if aac.is_some() {
            if menger.is_none() {
                menger = Some(Menger {
                    basis_hns: nu_hns(begin),
                    buffer: Vec::new(),
                    gecodeerd_tot_frame: 0,
                    klok: [None; 2],
                    bron_einde_frame: [None; 2],
                });
            }
            let m = menger.as_mut().expect("net aangemaakt");
            for (bron_idx, rx_opt) in [
                (0usize, audio.and_then(|a| a.systeem.as_ref())),
                (1usize, audio.and_then(|a| a.microfoon.as_ref())),
            ] {
                let Some(rx) = rx_opt else { continue };
                while let Ok(chunk) = rx.try_recv() {
                    if chunk.is_empty() {
                        continue;
                    }
                    let start = *m.klok[bron_idx].get_or_insert_with(|| nu_hns(begin));
                    let duur =
                        i64::try_from(chunk.len()).unwrap_or(0) / 2 * HNS_PER_SEC / 48_000;
                    if duur == 0 {
                        continue;
                    }
                    m.voeg_toe(bron_idx, start, &chunk);
                    m.klok[bron_idx] = Some(start + duur);
                }
            }

            // Alles wat álle bronnen tot en met leverden mag de encoder in — in
            // blokjes van ~50 ms, zodat één lange stilte niet één reuzenchunk wordt.
            if let Some(enc) = &mut aac {
                let veilig = m.veilig_tot_frame();
                const STAP_FRAMES: i64 = 2400;
                while m.gecodeerd_tot_frame < veilig {
                    let n = (veilig - m.gecodeerd_tot_frame).min(STAP_FRAMES);
                    let off = m.gecodeerd_tot_frame as usize * 2;
                    let pcm: Vec<i16> = m.buffer[off..off + (n as usize) * 2]
                        .iter()
                        .map(|s| (s.clamp(-1.0, 1.0) * 32_767.0) as i16)
                        .collect();
                    let abs =
                        m.basis_hns + m.gecodeerd_tot_frame * HNS_PER_SEC / 48_000;
                    match enc.voer(&pcm, abs, n * HNS_PER_SEC / 48_000) {
                        Ok(uit) => wachtrij.extend(uit),
                        Err(e) => {
                            tracing::warn!(error = %format!("{e:#}"), "AAC-codering mislukt");
                            break;
                        }
                    }
                    m.gecodeerd_tot_frame += n;
                }

                // De kop van de buffer is gecodeerd en kan weg; basis en administratie
                // schuiven mee zodat alle relatieve tijden kloppen blijven.
                if m.gecodeerd_tot_frame > 0 {
                    let d = (m.gecodeerd_tot_frame as usize)
                        .saturating_mul(2)
                        .min(m.buffer.len());
                    m.buffer.drain(..d);
                    m.basis_hns += m.gecodeerd_tot_frame * HNS_PER_SEC / 48_000;
                    for be in &mut m.bron_einde_frame {
                        *be = be.map(|v| (v - m.gecodeerd_tot_frame).max(0));
                    }
                    m.gecodeerd_tot_frame = 0;
                }
            }
        }

        // Bewaarverzoek: het lopende segment mag dicht zodra het keyframe er is.
        if staat.bewaar.swap(false, Ordering::Relaxed) {
            sluiten_gevraagd = true;
        }
        if sluiten_gevraagd && !keyframe_gevraagd && segment.is_some() {
            encoder.vraag_keyframe();
            keyframe_gevraagd = true;
        }

        let Some(mut opname) = capture.volgende_frame(FRAME_WACHT) else {
            continue;
        };
        while let Some(nieuwer) = capture.volgende_frame(Duration::ZERO) {
            opname = nieuwer;
        }
        if !tempo.laat_door(Instant::now()) {
            continue;
        }

        let (onul, wnul) =
            *nulpunten.get_or_insert_with(|| (opname.opgenomen_hns, nu_hns(begin)));
        let tijd = (wnul + (opname.opgenomen_hns - onul)).max(vorige_tijd + 1);
        vorige_tijd = tijd;

        let pakketten = match encoder.encode(&opname.textuur, tijd) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(error = %format!("{e:#}"), "beeld coderen mislukt");
                encoder.vraag_keyframe();
                continue;
            }
        };

        for p in pakketten {
            // Dichttrekken op een keyframe: op verzoek (bewaar), of omdat het segment
            // oud genoeg is. Komt het keyframe van de periodieke GOP in plaats van van
            // ons verzoek, dan is dichttrekken óók prima — het segment was oud genoeg.
            if let Some(s) = &segment {
                let leeftijd = p.tijd_hns - s.eerste_hns;
                if p.keyframe && (sluiten_gevraagd || leeftijd >= SEGMENT_DOEL_HNS) {
                    let gesloten = segment.take().expect("net gecontroleerd").sluit()?;
                    metas.push(gesloten);
                    metas.sort_by_key(|m| m.eerste_hns);
                    ruim_ring_op(&mut metas, instellingen.houdt_hns());
                    if sluiten_gevraagd {
                        bewaar_thread(
                            &metas,
                            instellingen,
                            clips_dir,
                            sporen.clone(),
                            gebeurtenissen.clone(),
                        );
                        sluiten_gevraagd = false;
                    }
                    keyframe_gevraagd = false;
                }
            }

            // Openen: alléén op een keyframe, want elk segment moet zelfstandig
            // decodeerbaar zijn.
            match &mut segment {
                None => {
                    if !p.keyframe {
                        continue;
                    }
                    let params = parameter_sets(encoder.sequentie_header())
                        .or_else(|| parameter_sets(&p.data))
                        .context("geen parameter sets voor het eerste segment")?;
                    let vs = VideoSpoor {
                        sps: params.0,
                        pps: params.1,
                        breedte,
                        hoogte,
                        fps: instellingen.fps,
                    };
                    // AAC is vast 48 kHz stereo — de taps normaliseren daar zelf naar.
                    let asp = if aac.is_some() {
                        Some(AudioSpoor {
                            sample_rate: 48_000,
                            kanalen: 2,
                            bitrate: AAC_BYTES_PER_SEC * 8,
                        })
                    } else {
                        None
                    };
                    sporen = Some((vs.clone(), asp.clone()));
                    let pad = ring_dir.join(format!("seg-{:020}.mp4", p.tijd_hns));
                    segment =
                        Some(SegmentSchrijver::open(pad, &vs, asp.as_ref(), p.tijd_hns)?);
                    keyframe_gevraagd = false;
                }
                Some(s) => {
                    if !keyframe_gevraagd
                        && p.tijd_hns - s.eerste_hns >= SEGMENT_DOEL_HNS
                    {
                        encoder.vraag_keyframe();
                        keyframe_gevraagd = true;
                    }
                }
            }

            let Some(s) = &mut segment else { continue };
            s.schrijf_video(p.tijd_hns, p.keyframe, naar_avcc(&p.data));
            while let Some((t, _d, _data)) = wachtrij.front() {
                if *t <= p.tijd_hns {
                    let (t, d, data) = wachtrij.pop_front().unwrap();
                    s.schrijf_audio(t, d as u32, data);
                } else {
                    break;
                }
            }
        }
    }

    // Achteraan netjes dicht; een stop is geen save.
    if let Some(s) = segment.take() {
        s.sluit()?;
    }
    Ok(())
}

/// De remux draait buiten de opnamedraad: sub-seconde normaal, maar waarom zou de
/// beeldketen er ook maar één milliseconde op moeten wachten.
fn bewaar_thread(
    metas: &[SegmentMeta],
    instellingen: &ClipInstellingen,
    clips_dir: &Path,
    sporen: Option<(VideoSpoor, Option<AudioSpoor>)>,
    gebeurtenissen: std::sync::mpsc::Sender<ClipGebeurtenis>,
) {
    let metas = metas.to_vec();
    let instellingen = instellingen.clone();
    let clips_dir = clips_dir.to_path_buf();
    let _ = std::thread::Builder::new()
        .name("fitcom-clip".into())
        .spawn(move || {
            let resultaat = (|| -> Result<PathBuf> {
                let Some((video, audio)) = sporen else {
                    bail!("er is nog geen beeld opgenomen");
                };
                let gekozen = kies_venster(&metas, instellingen.venster_hns());
                if gekozen.is_empty() {
                    bail!("er is nog niets opgenomen");
                }
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let pad = clips_dir.join(format!("clip-{stamp}.mp4"));
                plak_clip(
                    &gekozen,
                    instellingen.venster_hns(),
                    &pad,
                    &video,
                    audio.as_ref(),
                )?;
                Ok(pad)
            })();
            match resultaat {
                Ok(pad) => {
                    tracing::info!(pad = %pad.display(), "clip geschreven");
                    let _ = gebeurtenissen.send(ClipGebeurtenis::Klaar { pad });
                }
                Err(e) => {
                    tracing::warn!(error = %format!("{e:#}"), "clip mislukt");
                    let _ = gebeurtenissen.send(ClipGebeurtenis::Mislukt {
                        reden: e.to_string(),
                    });
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vier_byte_startcodes_worden_gesplitst() {
        let data = [0, 0, 0, 1, 65, 0, 0, 0, 1, 66, 67];
        let nals = splits_annexb(&data);
        assert_eq!(nals.len(), 2);
        assert_eq!(nals[0], vec![65]);
        assert_eq!(nals[1], vec![66, 67]);
    }

    #[test]
    fn drie_byte_startcodes_ok() {
        let data = [0, 0, 1, 65, 66, 0, 0, 1, 67];
        let nals = splits_annexb(&data);
        assert_eq!(nals, vec![vec![65, 66], vec![67]]);
    }

    /// De sequentie-header-blob van een encoder kan zonder startcodes aankomen; dan is
    /// het geheel één NAL en mag er niets verloren gaan.
    #[test]
    fn zonder_codes_blijft_het_geheel_een_nal() {
        let data = [0x67, 1, 2, 3];
        assert_eq!(splits_annexb(&data), vec![vec![0x67, 1, 2, 3]]);
    }

    #[test]
    fn lege_invoer_leegt_uit() {
        assert!(splits_annexb(&[]).is_empty());
        assert!(naar_avcc(&[]).is_empty());
    }

    #[test]
    fn avcc_lengtes_kloppen_met_de_inhoud() {
        let data = [0, 0, 0, 1, 10, 20, 30, 0, 0, 0, 1, 40];
        let avcc = naar_avcc(&data);
        // NAL 1: len=3 + [10 20 30]; NAL 2: len=1 + [40].
        assert_eq!(
            avcc,
            vec![0, 0, 0, 3, 10, 20, 30, 0, 0, 0, 1, 40]
        );
    }

    /// SPS/PPS herkennen moet ook werken als er een IDR (type 5) tussenzit — precies
    /// hoe het eerste keyframe van de encoder eruitziet.
    #[test]
    fn parameter_sets_vinden_sps_en_pps_tussen_andere_nals() {
        let sps = nal_met_type(7);
        let pps = nal_met_type(8);
        let idr = nal_met_type(5);
        let mut stream = Vec::new();
        for n in [&idr, &sps, &pps] {
            stream.extend_from_slice(&[0, 0, 0, 1]);
            stream.extend_from_slice(n);
        }
        let (gevonden_sps, gevonden_pps) = parameter_sets(&stream).expect("beide aanwezig");
        assert_eq!(gevonden_sps, sps);
        assert_eq!(gevonden_pps, pps);

        // Zonder PPS: geen paar, dus geen vertrouwbare avcC.
        let mut alleen_sps = Vec::new();
        alleen_sps.extend_from_slice(&[0, 0, 0, 1]);
        alleen_sps.extend_from_slice(&sps);
        assert!(parameter_sets(&alleen_sps).is_none());
    }

    fn nal_met_type(t: u8) -> Vec<u8> {
        // type zit in de lage vijf bits van de eerste byte; rest is smurf.
        vec![t | 0x60, 0xAA, 0xBB]
    }

    #[test]
    fn freq_index_tabel_kent_de_gebruikelijke_rates() {
        assert_eq!(freq_index_van(48_000), Some(3));
        assert_eq!(freq_index_van(44_100), Some(4));
        assert_eq!(freq_index_van(96_000), Some(0));
        assert_eq!(freq_index_van(12_345), None);
    }

    #[cfg(test)]
    mod adts {
        use super::*;

        #[test]
        fn header_draagt_de_lengte_in_zijn_bits() {
            // Een typische AAC-framegrootte bij 192 kbit/s: ruim onder de 13-bit grens.
            let h = adts_header(0x1234 + 7, 3, 2);
            assert_eq!(h[0], 0xFF);
            assert_eq!(h[1], 0xF1); // MPEG-4, geen CRC
                                     // Lengte zit verspreid over drie velden; teruglezend:
            let terug = ((u16::from(h[3]) & 0x03) << 11)
                | (u16::from(h[4]) << 3)
                | (u16::from(h[5]) >> 5);
            assert_eq!(terug as usize, 0x1234 + 7);
        }
    }

    fn meta(pad: &str, eerste: i64, laatste: i64) -> SegmentMeta {
        SegmentMeta {
            pad: PathBuf::from(pad),
            eerste_hns: eerste,
            laatste_hns: laatste,
        }
    }

    #[test]
    fn kies_venster_neemt_van_achternaam_tot_de_span_klopt() {
        let metas = [
            meta("a", 0, 200),
            meta("b", 200, 400),
            meta("c", 400, 600),
        ];
        // Venster van 250 hns: segment c alleen is te kort, b+c samen volstaat.
        let gekozen = kies_venster(&metas, 250);
        assert_eq!(gekozen.iter().map(|m| m.pad.clone()).collect::<Vec<_>>(), [
            PathBuf::from("b"),
            PathBuf::from("c")
        ]);
        // Venster groter dan alles: alle drie.
        assert_eq!(kies_venster(&metas, 10_000).len(), 3);
        // Lege ring blijft leeg.
        assert!(kies_venster(&[], 100).is_empty());
    }

    #[test]
    fn te_gooien_houdt_het_venster_plus_marge_over() {
        let sec = HNS_PER_SEC;
        let metas = [
            meta("oud", 0, 30 * sec),
            meta("midden", 30 * sec, 90 * sec),
            meta("vers", 90 * sec, 120 * sec),
        ];
        // Venster 50 s: grens = 120 − 50 − 4(marge) = 66 s. Alleen "oud" valt daarvoor.
        let weg = te_gooien(&metas, 50 * sec);
        assert_eq!(weg, vec![PathBuf::from("oud")]);
        // Venster groot genoeg: niets gaat weg.
        assert!(te_gooien(&metas, 500 * sec).is_empty());
        // Lege ring: niets om weg te gooien.
        assert!(te_gooien(&[], sec).is_empty());
    }
}

/// De inbox-AAC-encoder is software en bestaat op élke Windows-installatie, dus deze
/// test mag altijd mee.
#[cfg(all(test, windows))]
mod aac_toets {
    use super::*;

    #[test]
    fn aac_encoder_levert_frames_met_stijgende_tijden() {
        let mut enc = AacEncoder::new(48_000, 2).expect("inbox-AAC-encoder ontbreekt");
        let mut tijd = 0i64;
        let mut alles = Vec::new();
        for stap in 0..20 {
            // 10 ms stereo-sinus per chunk: net als de loopback-tap levert.
            let mut pcm = Vec::with_capacity(480 * 2);
            for i in 0..480 {
                let s = ((stap * 480 + i) as f32 * 0.02).sin();
                let waarde = (s * 8_000.0) as i16;
                pcm.push(waarde);
                pcm.push(waarde);
            }
            let duur = 480_i64 * HNS_PER_SEC / 48_000;
            alles.extend(enc.voer(&pcm, tijd, duur).expect("AAC-codering"));
            tijd += duur;
        }

        assert!(
            !alles.is_empty(),
            "200 ms sinus leverde geen enkel AAC-frame"
        );
        println!("{} AAC-frames uit 200 ms invoer", alles.len());
        // Frames komen met hun eigen tijdstempels uit de encoder; die moeten stijgen,
        // want de weergave plant zich hierop.
        for w in alles.windows(2) {
            assert!(w[1].0 > w[0].0, "tijden stijgen niet: {} → {}", w[0].0, w[1].0);
        }
        assert!(alles.iter().all(|(_, d, data)| *d > 0 && !data.is_empty()));
    }
}
