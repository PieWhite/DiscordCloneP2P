//! De draaiende voice-sessie: vier threads en twee geluidsstromen.
//!
//! ```text
//! microfoon ─► cpal-callback ─► opname-thread ──► UDP naar alle peers
//!                               (herbemonsteren, ruisonderdrukking, VAD, Opus)
//!
//! UDP ─► ontvang-thread ─► jitterbuffer per spreker
//!                              │
//!                              ▼
//!                          mix-thread (decoderen, optellen) ─► cpal-callback ─► koptelefoon
//! ```
//!
//! # Waarom de threads zo verdeeld zijn
//!
//! De twee cpal-callbacks draaien op threads van de geluidsdriver. Wat daar te lang
//! duurt levert een hoorbare klik op. Ze doen daarom niets anders dan samples in of uit
//! een kanaal schuiven; al het rekenwerk staat ernaast.
//!
//! De mix-thread laat zich aandrijven door hoe snel de geluidskaart de weergavebuffer
//! leegtrekt, niet door een timer. Op een timer zou hij onvermijdelijk uit de pas
//! lopen met de kaart, en dan loopt de buffer langzaam vol of leeg.

use crate::codec::{Decoder, Encoder, MAX_PAYLOAD};
use crate::jitter::{Frame, JitterBuffer};
use crate::mix::{self, Resampler, FRAME_SAMPLES, SAMPLE_RATE};
use anyhow::{bail, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{bounded, Receiver, Sender};
use fitcom_net::MediaSocket;
use fitcom_proto::{MediaHeader, PayloadType, PeerId, VOICE_STREAM_ID};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Zoveel frames blijven we nog doorsturen nadat je stopt met praten. Zonder dit kapt
/// de VAD het eind van je woorden af, wat klinkt alsof de verbinding hapert.
const HANGOVER_FRAMES: u32 = 25; // 500 ms
/// Boven deze spraakkans van RNNoise gaan we ervan uit dat er iemand praat.
const VAD_DREMPEL: f32 = 0.65;
/// Zoveel frames houden we klaar voor de geluidskaart. Meer is vertraging, minder
/// is risico op onderloop.
const WEERGAVE_DIEPTE: usize = 3;

#[derive(Debug, Clone)]
pub struct VoiceConfig {
    pub media_port: u16,
    /// Naam zoals Windows hem toont. `None` = het standaardapparaat.
    pub input_device: Option<String>,
    pub output_device: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerAdres {
    pub id: PeerId,
    pub addr: SocketAddr,
}

/// Momentopname voor de spreekindicatie in de UI.
#[derive(Debug, Clone, Default)]
pub struct Niveaus {
    pub eigen: f32,
    pub per_peer: HashMap<PeerId, f32>,
}

/// Een geluidsbron: welke peer, en welke stream van hem.
///
/// Stream 0 is zijn stem. Deelt hij zijn bureaubladgeluid, dan komt dat binnen op het
/// id dat hij daaraan gaf. Voor de mixer is dat gewoon een bron erbij — met een eigen
/// volumeschuif, want spelgeluid en een stem wil je los kunnen regelen.
pub type Bron = (PeerId, u32);

/// Wat we van ons eigen bureaubladgeluid delen, en met wie.
#[derive(Debug, Clone)]
struct Bureaublad {
    stream_id: u32,
    doelen: Vec<SocketAddr>,
}

struct Gedeeld {
    peers: Mutex<Vec<PeerAdres>>,
    volumes: Mutex<HashMap<Bron, f32>>,
    jitters: Mutex<HashMap<Bron, JitterBuffer>>,
    niveaus: Mutex<HashMap<Bron, f32>>,
    mute: AtomicBool,
    deafen: AtomicBool,
    /// `f32` via `to_bits`, zodat de UI hem zonder lock kan uitlezen.
    eigen_niveau: AtomicU32,
    stop: AtomicBool,
    /// `None` betekent: we delen geen bureaubladgeluid. De opname-thread stopt zichzelf
    /// zodra dit leeg is, dus er wordt niets opgenomen als niemand luistert.
    bureaublad: Mutex<Option<Bureaublad>>,
}

impl Gedeeld {
    fn volume_van(&self, bron: Bron) -> f32 {
        self.volumes
            .lock()
            .map(|v| v.get(&bron).copied().unwrap_or(1.0))
            .unwrap_or(1.0)
    }
}

pub struct VoiceHandle {
    gedeeld: Arc<Gedeeld>,
    /// Waar de anderen hun audio naartoe moeten sturen.
    pub media_addr: SocketAddr,
    /// Om er later een verzendkant voor bureaubladgeluid bij te kunnen zetten.
    socket: MediaSocket,
    uitvoer_apparaat: Option<String>,
}

impl VoiceHandle {
    /// Wie er in het gesprek zitten en waar hun audio heen moet. Mag op elk moment
    /// wijzigen: iemand die wegvalt of erbij komt is de normale gang van zaken.
    pub fn zet_peers(&self, peers: Vec<PeerAdres>) {
        if let Ok(mut p) = self.gedeeld.peers.lock() {
            *p = peers;
        }
    }

    pub fn zet_mute(&self, aan: bool) {
        self.gedeeld.mute.store(aan, Ordering::Relaxed);
    }

    pub fn is_mute(&self) -> bool {
        self.gedeeld.mute.load(Ordering::Relaxed)
    }

    /// Deafen zet ook de microfoon uit: als jij niemand hoort, hoort niemand jou.
    /// Alles anders is verwarrend in gebruik.
    pub fn zet_deafen(&self, aan: bool) {
        self.gedeeld.deafen.store(aan, Ordering::Relaxed);
        if aan {
            self.gedeeld.mute.store(true, Ordering::Relaxed);
        }
    }

    pub fn is_deafen(&self) -> bool {
        self.gedeeld.deafen.load(Ordering::Relaxed)
    }

    /// Volume van iemands stem.
    pub fn zet_volume(&self, peer: PeerId, volume: f32) {
        self.zet_bron_volume((peer, VOICE_STREAM_ID), volume);
    }

    /// Volume van één bron. Het bureaubladgeluid van een peer staat hiermee los van
    /// zijn stem: je wilt zijn spel zachter kunnen zetten zonder hem te dempen.
    pub fn zet_bron_volume(&self, bron: Bron, volume: f32) {
        if let Ok(mut v) = self.gedeeld.volumes.lock() {
            v.insert(bron, volume.clamp(0.0, 2.0));
        }
    }

    pub fn niveaus(&self) -> Niveaus {
        Niveaus {
            eigen: f32::from_bits(self.gedeeld.eigen_niveau.load(Ordering::Relaxed)),
            per_peer: self
                .gedeeld
                .niveaus
                .lock()
                .map(|n| {
                    // De UI toont één balkje per persoon, en dat hoort bij zijn stem.
                    n.iter()
                        .filter(|((_, stream), _)| *stream == VOICE_STREAM_ID)
                        .map(|((peer, _), niveau)| (*peer, *niveau))
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    /// Begint het geluid van deze PC mee te sturen op `stream_id`.
    ///
    /// Idempotent: staat hij al aan, dan worden alleen de luisteraars bijgewerkt. Een
    /// lege lijst luisteraars stopt de opname — er wordt niets opgenomen als niemand
    /// meeluistert.
    pub fn deel_bureaublad(&self, stream_id: u32, doelen: Vec<SocketAddr>) -> Result<()> {
        let draaide_al = self
            .gedeeld
            .bureaublad
            .lock()
            .map(|b| b.is_some())
            .unwrap_or(false);

        if doelen.is_empty() {
            self.stop_bureaublad();
            return Ok(());
        }

        if let Ok(mut b) = self.gedeeld.bureaublad.lock() {
            *b = Some(Bureaublad { stream_id, doelen });
        }
        if !draaide_al {
            start_bureaublad(
                self.gedeeld.clone(),
                self.socket.probeer_clone()?,
                self.uitvoer_apparaat.clone(),
            )?;
        }
        Ok(())
    }

    pub fn stop_bureaublad(&self) {
        if let Ok(mut b) = self.gedeeld.bureaublad.lock() {
            *b = None;
        }
    }

    /// Haalt een bron uit de mix. Zonder dit blijft de jitterbuffer van een stream waar
    /// niemand meer op ingetekend is eeuwig verliesverberging produceren.
    pub fn vergeet_bron(&self, bron: Bron) {
        if let Ok(mut j) = self.gedeeld.jitters.lock() {
            j.remove(&bron);
        }
        if let Ok(mut n) = self.gedeeld.niveaus.lock() {
            n.remove(&bron);
        }
    }

    pub fn deelt_bureaublad(&self) -> bool {
        self.gedeeld
            .bureaublad
            .lock()
            .map(|b| b.is_some())
            .unwrap_or(false)
    }

    /// De poort waarop wij audio verwachten. Hoort in `StreamSubscribe.media_port` als
    /// we op iemands bureaubladgeluid intekenen.
    pub fn media_port(&self) -> u16 {
        self.media_addr.port()
    }
}

impl Drop for VoiceHandle {
    fn drop(&mut self) {
        self.gedeeld.stop.store(true, Ordering::Relaxed);
        tracing::info!("voice gestopt");
    }
}

/// Wat de audio-thread terugmeldt zodra de apparaten open zijn.
struct Apparaten {
    invoer_rate: u32,
    uitvoer_rate: u32,
    uitvoer_kanalen: usize,
}

pub fn start(cfg: VoiceConfig) -> Result<VoiceHandle> {
    let socket = bind_met_geduld(cfg.media_port)?;
    let media_addr = socket.local_addr()?;
    // Bureaubladgeluid komt van hetzelfde apparaat als waar jij op luistert — dat is
    // per definitie wat er te horen valt.
    let uitvoer_naam = cfg.output_device.clone();

    let gedeeld = Arc::new(Gedeeld {
        peers: Mutex::new(Vec::new()),
        volumes: Mutex::new(HashMap::new()),
        jitters: Mutex::new(HashMap::new()),
        niveaus: Mutex::new(HashMap::new()),
        mute: AtomicBool::new(false),
        deafen: AtomicBool::new(false),
        eigen_niveau: AtomicU32::new(0),
        stop: AtomicBool::new(false),
        bureaublad: Mutex::new(None),
    });

    let (opname_tx, opname_rx) = bounded::<Vec<f32>>(32);
    let (weergave_tx, weergave_rx) = bounded::<Vec<f32>>(WEERGAVE_DIEPTE + 2);
    let (klaar_tx, klaar_rx) = bounded::<Result<Apparaten>>(1);

    // De cpal-streams zijn niet `Send`, dus ze moeten leven op de thread die ze
    // aanmaakt. Die thread doet verder niets dan wachten tot we stoppen.
    {
        let gedeeld = gedeeld.clone();
        std::thread::Builder::new()
            .name("fitcom-audio".into())
            .spawn(move || match open_apparaten(&cfg, opname_tx, weergave_rx) {
                Ok((apparaten, invoer, uitvoer)) => {
                    let _ = klaar_tx.send(Ok(apparaten));
                    while !gedeeld.stop.load(Ordering::Relaxed) {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    drop(invoer);
                    drop(uitvoer);
                }
                Err(e) => {
                    let _ = klaar_tx.send(Err(e));
                }
            })
            .context("audio-thread starten")?;
    }

    let apparaten = klaar_rx
        .recv_timeout(Duration::from_secs(10))
        .context("geluidsapparaten reageren niet")??;

    tracing::info!(
        invoer_hz = apparaten.invoer_rate,
        uitvoer_hz = apparaten.uitvoer_rate,
        kanalen = apparaten.uitvoer_kanalen,
        %media_addr,
        "voice gestart"
    );

    let verzender = socket.probeer_clone()?;
    let bewaard = socket.probeer_clone()?;
    start_opname(gedeeld.clone(), opname_rx, verzender, apparaten.invoer_rate)?;
    start_ontvangst(gedeeld.clone(), socket)?;
    start_mixen(gedeeld.clone(), weergave_tx, apparaten.uitvoer_rate)?;

    Ok(VoiceHandle {
        gedeeld,
        media_addr,
        socket: bewaard,
        uitvoer_apparaat: uitvoer_naam,
    })
}

/// De threads van een vorige sessie merken pas na hun timeout dat ze mogen stoppen,
/// dus vlak na "verlaten" is de poort nog even bezet. Wie meteen opnieuw deelneemt
/// zou anders een onbegrijpelijke foutmelding krijgen.
fn bind_met_geduld(port: u16) -> Result<MediaSocket> {
    let mut laatste = None;
    for _ in 0..15 {
        match MediaSocket::bind(port) {
            Ok(s) => return Ok(s),
            Err(e) => {
                laatste = Some(e);
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
    Err(laatste.expect("lus draait minstens één keer")).with_context(|| {
        format!("mediapoort {port} openen; draait er nog een instantie op deze poort?")
    })
}

// ---------------------------------------------------------------------------
// Apparaten
// ---------------------------------------------------------------------------

fn kies_apparaat<I: Iterator<Item = cpal::Device>>(
    apparaten: I,
    naam: Option<&String>,
) -> Option<cpal::Device> {
    let naam = naam?;
    apparaten.into_iter().find(|d| d.to_string() == *naam)
}

fn open_apparaten(
    cfg: &VoiceConfig,
    opname_tx: Sender<Vec<f32>>,
    weergave_rx: Receiver<Vec<f32>>,
) -> Result<(Apparaten, cpal::Stream, cpal::Stream)> {
    let host = cpal::default_host();

    let invoer = kies_apparaat(host.input_devices()?, cfg.input_device.as_ref())
        .or_else(|| host.default_input_device())
        .context("geen microfoon gevonden")?;
    let uitvoer = kies_apparaat(host.output_devices()?, cfg.output_device.as_ref())
        .or_else(|| host.default_output_device())
        .context("geen weergaveapparaat gevonden")?;

    let in_cfg = invoer
        .default_input_config()
        .context("microfoon-instellingen")?;
    let uit_cfg = uitvoer
        .default_output_config()
        .context("weergave-instellingen")?;

    let invoer_rate = in_cfg.sample_rate();
    let uitvoer_rate = uit_cfg.sample_rate();
    let in_kanalen = in_cfg.channels() as usize;
    let uit_kanalen = uit_cfg.channels() as usize;

    if invoer_rate != SAMPLE_RATE || uitvoer_rate != SAMPLE_RATE {
        // Werkt prima, maar herbemonsteren is werk dat we anders niet hoefden te doen
        // en het is niet verliesvrij. Windows staat dit meestal per apparaat in.
        tracing::warn!(
            invoer_rate,
            uitvoer_rate,
            "apparaat staat niet op 48000 Hz; zet dat in Windows in voor de beste kwaliteit"
        );
    }

    let in_stream = bouw_invoer(&invoer, &in_cfg, in_kanalen, opname_tx)?;
    let uit_stream = bouw_uitvoer(&uitvoer, &uit_cfg, uit_kanalen, weergave_rx)?;
    in_stream.play().context("microfoon starten")?;
    uit_stream.play().context("weergave starten")?;

    Ok((
        Apparaten {
            invoer_rate,
            uitvoer_rate,
            uitvoer_kanalen: uit_kanalen,
        },
        in_stream,
        uit_stream,
    ))
}

fn fout_melden(e: cpal::Error) {
    tracing::error!(error = %e, "geluidsstroom gaf een fout");
}

fn bouw_invoer(
    dev: &cpal::Device,
    cfg: &cpal::SupportedStreamConfig,
    kanalen: usize,
    tx: Sender<Vec<f32>>,
) -> Result<cpal::Stream> {
    let scfg = cfg.config();
    let doorgeven = move |mono: Vec<f32>| {
        // Volle wachtrij betekent dat de opname-thread niet bijhoudt. Weggooien is
        // dan beter dan de driver-thread laten wachten, want dat hoor je meteen.
        let _ = tx.try_send(mono);
    };

    let stream = match cfg.sample_format() {
        cpal::SampleFormat::F32 => dev.build_input_stream::<f32, _, _>(
            scfg,
            move |data, _| {
                let mut mono = Vec::new();
                mix::naar_mono(data, kanalen, &mut mono);
                doorgeven(mono);
            },
            fout_melden,
            None,
        )?,
        cpal::SampleFormat::I16 => dev.build_input_stream::<i16, _, _>(
            scfg,
            move |data: &[i16], _| {
                let drijvend: Vec<f32> = data.iter().map(|&s| f32::from(s) / 32768.0).collect();
                let mut mono = Vec::new();
                mix::naar_mono(&drijvend, kanalen, &mut mono);
                doorgeven(mono);
            },
            fout_melden,
            None,
        )?,
        andere => bail!("microfoon levert {andere:?}; alleen f32 en i16 worden ondersteund"),
    };
    Ok(stream)
}

fn bouw_uitvoer(
    dev: &cpal::Device,
    cfg: &cpal::SupportedStreamConfig,
    kanalen: usize,
    rx: Receiver<Vec<f32>>,
) -> Result<cpal::Stream> {
    let scfg = cfg.config();

    // Wat er van het vorige blok overbleef. De geluidskaart vraagt zelden precies
    // een heel frame, dus zonder dit zou elk blok een stukje stilte krijgen.
    let mut rest: Vec<f32> = Vec::new();
    let mut pos = 0usize;

    let mut volgende = move || -> f32 {
        if pos >= rest.len() {
            match rx.try_recv() {
                Ok(v) => {
                    rest = v;
                    pos = 0;
                }
                // Niets klaar: stilte is het enige juiste antwoord. Wachten zou de
                // driver-thread blokkeren.
                Err(_) => return 0.0,
            }
        }
        let s = rest.get(pos).copied().unwrap_or(0.0);
        pos += 1;
        s
    };

    let stream = match cfg.sample_format() {
        cpal::SampleFormat::F32 => dev.build_output_stream::<f32, _, _>(
            scfg,
            move |data: &mut [f32], _| {
                for blok in data.chunks_mut(kanalen) {
                    let s = volgende();
                    blok.iter_mut().for_each(|k| *k = s);
                }
            },
            fout_melden,
            None,
        )?,
        cpal::SampleFormat::I16 => dev.build_output_stream::<i16, _, _>(
            scfg,
            move |data: &mut [i16], _| {
                for blok in data.chunks_mut(kanalen) {
                    let s = (volgende() * 32767.0).clamp(-32768.0, 32767.0) as i16;
                    blok.iter_mut().for_each(|k| *k = s);
                }
            },
            fout_melden,
            None,
        )?,
        andere => bail!("weergave vraagt {andere:?}; alleen f32 en i16 worden ondersteund"),
    };
    Ok(stream)
}

// ---------------------------------------------------------------------------
// Opnemen en versturen
// ---------------------------------------------------------------------------

fn start_opname(
    gedeeld: Arc<Gedeeld>,
    rx: Receiver<Vec<f32>>,
    socket: MediaSocket,
    invoer_rate: u32,
) -> Result<()> {
    std::thread::Builder::new()
        .name("fitcom-opname".into())
        .spawn(move || {
            if let Err(e) = opname_lus(&gedeeld, rx, socket, invoer_rate) {
                tracing::error!(error = %format!("{e:#}"), "opname gestopt door een fout");
            }
        })
        .context("opname-thread starten")?;
    Ok(())
}

fn opname_lus(
    gedeeld: &Arc<Gedeeld>,
    rx: Receiver<Vec<f32>>,
    socket: MediaSocket,
    invoer_rate: u32,
) -> Result<()> {
    let mut resampler = Resampler::new(invoer_rate, SAMPLE_RATE);
    let mut ruis = nnnoiseless::DenoiseState::new();
    let mut encoder = Encoder::new()?;

    let blok = nnnoiseless::DenoiseState::FRAME_SIZE;
    let mut op48: Vec<f32> = Vec::new();
    let mut schoon: Vec<f32> = Vec::new();
    let mut uit_blok = vec![0.0f32; blok];
    let mut pakket = [0u8; MAX_PAYLOAD];

    let mut seq: u32 = 0;
    let mut timestamp: u32 = 0;
    let mut hangover: u32 = 0;

    while !gedeeld.stop.load(Ordering::Relaxed) {
        let Ok(chunk) = rx.recv_timeout(Duration::from_millis(200)) else {
            continue;
        };

        // RNNoise verwacht samples op i16-schaal, niet -1..1.
        let mut herbemonsterd = Vec::new();
        resampler.verwerk(&chunk, &mut herbemonsterd);
        op48.extend(herbemonsterd.iter().map(|s| s * 32768.0));

        while op48.len() >= blok {
            let invoer: Vec<f32> = op48.drain(..blok).collect();
            let kans = ruis.process_frame(&mut uit_blok, &invoer);
            if kans > VAD_DREMPEL {
                hangover = HANGOVER_FRAMES;
            }
            schoon.extend_from_slice(&uit_blok);
        }

        while schoon.len() >= FRAME_SAMPLES {
            let frame: Vec<f32> = schoon.drain(..FRAME_SAMPLES).collect();
            let niveau = rms(&frame) / 32768.0;
            gedeeld
                .eigen_niveau
                .store(niveau.to_bits(), Ordering::Relaxed);

            let praat = hangover > 0;
            hangover = hangover.saturating_sub(1);

            timestamp = timestamp.wrapping_add(FRAME_SAMPLES as u32);
            if gedeeld.mute.load(Ordering::Relaxed) || !praat {
                // Niets versturen. Dat is precies waarom de app in rust vrijwel geen
                // verkeer en geen CPU gebruikt.
                gedeeld
                    .eigen_niveau
                    .store(0f32.to_bits(), Ordering::Relaxed);
                continue;
            }

            let pcm: Vec<i16> = frame
                .iter()
                .map(|&s| s.clamp(-32768.0, 32767.0) as i16)
                .collect();
            let n = match encoder.encode(&pcm, &mut pakket) {
                Ok(n) => n,
                Err(e) => {
                    tracing::warn!(error = %e, "frame coderen mislukt");
                    continue;
                }
            };

            seq = seq.wrapping_add(1);
            let header = MediaHeader {
                stream_id: VOICE_STREAM_ID,
                seq,
                timestamp,
                payload_type: PayloadType::OPUS,
                flags: 0,
                frag_index: 0,
            };

            let peers = gedeeld.peers.lock().map(|p| p.clone()).unwrap_or_default();
            for peer in peers {
                if let Err(e) = socket.stuur(peer.addr, &header, &pakket[..n]) {
                    tracing::debug!(peer = ?peer.id, error = %e, "audiopakket niet verstuurd");
                }
            }
        }
    }
    Ok(())
}

fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let som: f32 = samples.iter().map(|s| s * s).sum();
    (som / samples.len() as f32).sqrt()
}

// ---------------------------------------------------------------------------
// Bureaubladgeluid
// ---------------------------------------------------------------------------

/// Onder dit niveau gaan we ervan uit dat er niets speelt. Laag gezet: een stil stuk in
/// muziek moet er niet uit geknipt worden, het gaat er alleen om dat een PC waar niets
/// op draait geen verkeer veroorzaakt.
const BUREAUBLAD_DREMPEL: f32 = 0.002;
/// Zo lang blijven we doorsturen nadat het stil werd. Ruim, want anders hoor je bij elk
/// kort stiltemoment de codec opnieuw inschakelen.
const BUREAUBLAD_HANGOVER: u32 = 100; // 2 seconden

/// Neemt op wat er uit de luidsprekers komt en stuurt het mee.
///
/// # Waarom hier geen eigen WASAPI-code staat
///
/// `cpal` doet dit al: bouw je een *invoer*stroom op een *uitvoer*apparaat, dan zet
/// WASAPI stilzwijgend loopback aan. Dat scheelt een hoop `unsafe` en een tweede
/// implementatie van dezelfde apparaatlogica.
fn start_bureaublad(
    gedeeld: Arc<Gedeeld>,
    socket: MediaSocket,
    apparaat: Option<String>,
) -> Result<()> {
    std::thread::Builder::new()
        .name("fitcom-bureaublad".into())
        .spawn(move || {
            if let Err(e) = bureaublad_lus(&gedeeld, &socket, apparaat.as_ref()) {
                tracing::error!(error = %format!("{e:#}"), "bureaubladgeluid gestopt door een fout");
            }
            // Wat er ook gebeurde: de staat moet kloppen, anders denkt de UI dat we
            // nog delen en start er nooit meer een opname.
            if let Ok(mut b) = gedeeld.bureaublad.lock() {
                *b = None;
            }
        })
        .context("bureaublad-thread starten")?;
    Ok(())
}

fn bureaublad_lus(
    gedeeld: &Arc<Gedeeld>,
    socket: &MediaSocket,
    apparaat: Option<&String>,
) -> Result<()> {
    let host = cpal::default_host();
    let dev = kies_apparaat(host.output_devices()?, apparaat)
        .or_else(|| host.default_output_device())
        .context("geen weergaveapparaat om geluid van af te tappen")?;
    let cfg = dev
        .default_output_config()
        .context("weergave-instellingen")?;
    let rate = cfg.sample_rate();
    let kanalen = cfg.channels() as usize;

    let (tx, rx) = bounded::<Vec<f32>>(32);
    // De stroom moet blijven leven op deze thread; laat je hem vallen dan stopt de
    // opname stil.
    let _stroom = bouw_invoer(&dev, &cfg, kanalen, tx)?;
    _stroom.play().context("bureaubladopname starten")?;

    tracing::info!(apparaat = %dev.to_string(), rate, kanalen, "bureaubladgeluid wordt gedeeld");

    let mut resampler = Resampler::new(rate, SAMPLE_RATE);
    let mut encoder = Encoder::voor_muziek()?;
    let mut op48: Vec<f32> = Vec::new();
    let mut pakket = [0u8; MAX_PAYLOAD];
    let mut seq: u32 = 0;
    let mut timestamp: u32 = 0;
    let mut hangover: u32 = 0;

    while !gedeeld.stop.load(Ordering::Relaxed) {
        let Some(deling) = gedeeld.bureaublad.lock().ok().and_then(|b| b.clone()) else {
            break; // niemand luistert meer
        };

        let Ok(blok) = rx.recv_timeout(Duration::from_millis(200)) else {
            continue;
        };
        let mut herbemonsterd = Vec::new();
        resampler.verwerk(&blok, &mut herbemonsterd);
        op48.extend(herbemonsterd.iter().map(|s| s * 32768.0));

        while op48.len() >= FRAME_SAMPLES {
            let frame: Vec<f32> = op48.drain(..FRAME_SAMPLES).collect();
            timestamp = timestamp.wrapping_add(FRAME_SAMPLES as u32);

            if rms(&frame) / 32768.0 > BUREAUBLAD_DREMPEL {
                hangover = BUREAUBLAD_HANGOVER;
            }
            if hangover == 0 {
                continue; // stilte kost geen bandbreedte
            }
            hangover -= 1;

            let pcm: Vec<i16> = frame
                .iter()
                .map(|&s| s.clamp(-32768.0, 32767.0) as i16)
                .collect();
            let n = match encoder.encode(&pcm, &mut pakket) {
                Ok(n) => n,
                Err(e) => {
                    tracing::warn!(error = %e, "bureaubladgeluid coderen mislukt");
                    continue;
                }
            };

            seq = seq.wrapping_add(1);
            let header = MediaHeader {
                stream_id: deling.stream_id,
                seq,
                timestamp,
                payload_type: PayloadType::OPUS,
                flags: 0,
                frag_index: 0,
            };
            for doel in &deling.doelen {
                if let Err(e) = socket.stuur(*doel, &header, &pakket[..n]) {
                    tracing::debug!(%doel, error = %e, "bureaubladgeluid niet verstuurd");
                }
            }
        }
    }

    tracing::info!("bureaubladgeluid niet meer gedeeld");
    Ok(())
}

// ---------------------------------------------------------------------------
// Ontvangen
// ---------------------------------------------------------------------------

fn start_ontvangst(gedeeld: Arc<Gedeeld>, socket: MediaSocket) -> Result<()> {
    std::thread::Builder::new()
        .name("fitcom-ontvangst".into())
        .spawn(move || {
            let mut buf = [0u8; fitcom_net::MAX_PAKKET];
            while !gedeeld.stop.load(Ordering::Relaxed) {
                let ontvangen = match socket.ontvang(&mut buf) {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::warn!(error = %format!("{e:#}"), "mediasocket gaf een fout");
                        continue;
                    }
                };
                let Some((van, header, payload)) = ontvangen else {
                    continue;
                };
                if header.payload_type != PayloadType::OPUS {
                    continue; // video; die heeft zijn eigen poort en zijn eigen thread
                }

                // Alleen van wie in het gesprek zit. Op een open UDP-poort komt
                // vroeg of laat iets anders binnen.
                let afzender = gedeeld
                    .peers
                    .lock()
                    .ok()
                    .and_then(|p| p.iter().find(|p| p.addr == van).map(|p| p.id));
                let Some(peer) = afzender else { continue };

                // Elke stream van een peer is een eigen bron met een eigen
                // jitterbuffer: zijn stem en zijn spelgeluid lopen niet gelijk op en
                // mogen elkaars volgorde niet in de war schoppen.
                if let Ok(mut j) = gedeeld.jitters.lock() {
                    j.entry((peer, header.stream_id))
                        .or_default()
                        .push(header.seq, payload.to_vec());
                }
            }
        })
        .context("ontvang-thread starten")?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Mixen en afspelen
// ---------------------------------------------------------------------------

fn start_mixen(gedeeld: Arc<Gedeeld>, tx: Sender<Vec<f32>>, uitvoer_rate: u32) -> Result<()> {
    std::thread::Builder::new()
        .name("fitcom-mix".into())
        .spawn(move || {
            let mut decoders: HashMap<Bron, Decoder> = HashMap::new();
            let mut resampler = Resampler::new(SAMPLE_RATE, uitvoer_rate);
            let mut gemixt = vec![0i16; FRAME_SAMPLES];

            while !gedeeld.stop.load(Ordering::Relaxed) {
                // Laten aandrijven door de geluidskaart: pas bijmaken als er ruimte is.
                if tx.len() >= WEERGAVE_DIEPTE {
                    std::thread::sleep(Duration::from_millis(2));
                    continue;
                }

                // Kort vasthouden: decoderen doen we buiten de lock, anders houdt de
                // mix-thread de ontvang-thread onnodig op.
                let frames: Vec<(Bron, Frame)> = match gedeeld.jitters.lock() {
                    Ok(mut j) => j.iter_mut().map(|(&id, b)| (id, b.pop())).collect(),
                    Err(_) => Vec::new(),
                };

                let mut sprekers: Vec<(Vec<i16>, f32)> = Vec::new();
                let mut niveaus: HashMap<Bron, f32> = HashMap::new();

                for (peer, frame) in frames {
                    let decoder = match decoders.entry(peer) {
                        std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                        std::collections::hash_map::Entry::Vacant(e) => match Decoder::new() {
                            Ok(d) => e.insert(d),
                            Err(err) => {
                                tracing::error!(error = %err, "decoder aanmaken mislukt");
                                continue;
                            }
                        },
                    };

                    let mut pcm = vec![0i16; FRAME_SAMPLES];
                    let gelukt = match frame {
                        Frame::Data(payload) => decoder.decode(&payload, &mut pcm).is_ok(),
                        Frame::Verloren => decoder.verberg_verlies(&mut pcm).is_ok(),
                        Frame::Stilte => false,
                    };
                    if !gelukt {
                        niveaus.insert(peer, 0.0);
                        continue;
                    }

                    let niveau =
                        rms(&pcm.iter().map(|&s| f32::from(s)).collect::<Vec<_>>()) / 32768.0;
                    niveaus.insert(peer, niveau);
                    sprekers.push((pcm, gedeeld.volume_van(peer)));
                }

                if let Ok(mut n) = gedeeld.niveaus.lock() {
                    *n = niveaus;
                }

                if gedeeld.deafen.load(Ordering::Relaxed) {
                    sprekers.clear();
                }

                let verwijzingen: Vec<(&[i16], f32)> =
                    sprekers.iter().map(|(s, v)| (s.as_slice(), *v)).collect();
                mix::mix_naar_i16(&verwijzingen, &mut gemixt);

                let drijvend: Vec<f32> = gemixt.iter().map(|&s| f32::from(s) / 32768.0).collect();
                let mut uit = Vec::new();
                resampler.verwerk(&drijvend, &mut uit);
                if tx.send(uit).is_err() {
                    break;
                }
            }
        })
        .context("mix-thread starten")?;
    Ok(())
}

/// Namen van de beschikbare microfoons en weergaveapparaten, zoals Windows ze toont.
/// Precies de tekst die in `config.toml` moet komen te staan.
pub fn apparaatnamen() -> Result<(Vec<String>, Vec<String>)> {
    let host = cpal::default_host();
    let invoer = host
        .input_devices()?
        .map(|d| d.to_string())
        .collect::<Vec<_>>();
    let uitvoer = host
        .output_devices()?
        .map(|d| d.to_string())
        .collect::<Vec<_>>();
    Ok((invoer, uitvoer))
}
