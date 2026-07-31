//! De kant die kijkt: ontvangen, samenstellen, decoderen, tonen.
//!
//! ```text
//! UDP ─► reassembler ─► decoder ─► kleuromzetting ─► swapchain van het venster
//! ```
//!
//! Eén thread per bekeken stream, met een eigen UDP-poort. Die poort staat in de
//! `StreamSubscribe` die de motor verstuurt, en daarom hoeft dit nergens te concurreren
//! met de voice-poort: die is bezet zodra je in een gesprek zit.
//!
//! # Waarom er op een keyframe gewacht wordt
//!
//! Een H.264-stroom is alleen te volgen vanaf een keyframe. Wie halverwege aanhaakt en
//! toch begint te decoderen krijgt vlekken die pas bij het volgende keyframe weggaan.
//! Beter niets tonen dan iets kapots tonen: het venster blijft leeg tot er beeld is dat
//! klopt.

use crate::codec::{Codec, Decoder};
use crate::d3d::D3dContext;
use crate::fragment::Reassembler;
use crate::venster::Venster;
use anyhow::{Context, Result};
use crossbeam_channel::{bounded, Receiver, Sender};
use fitcom_net::MediaSocket;
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Kort genoeg dat het venster op muis en toetsen blijft reageren als er geen beeld
/// binnenkomt, lang genoeg dat we niet zinloos wakker worden.
const ONTVANG_TIMEOUT: Duration = Duration::from_millis(8);
/// Niet vaker dan dit om een keyframe vragen. Bij aanhoudend verlies zou een verzoek
/// per kapot beeld de deler bedelven en het probleem alleen maar erger maken.
const KEYFRAME_PAUZE: Duration = Duration::from_millis(500);
/// Hoe vaak er een miniatuur voor het overzicht in het hoofdvenster wordt afgeleid.
/// Dit is geen weergavepad — twee keer per seconde is ruim genoeg om levend te ogen en
/// te weinig om ook maar iets te merken van de GPU-naar-CPU-kopie die het kost.
const MINIATUUR_INTERVAL: Duration = Duration::from_millis(500);
/// Breedte van de miniatuur; de hoogte volgt de beeldverhouding van de bron.
const MINIATUUR_BREEDTE: u32 = 192;

#[derive(Debug, Clone)]
pub enum KijkerEvent {
    /// De gebruiker heeft het venster gesloten.
    Gesloten,
    /// We zijn de draad kwijt; de deler moet een keyframe sturen.
    KeyframeNodig,
    /// Een verkleind beeld voor het overzicht in het hoofdvenster. BGRA, net als de
    /// textuur waar het uit komt — de UI zet dat zelf om naar wat egui verwacht.
    Miniatuur(Miniatuur),
}

#[derive(Debug, Clone)]
pub struct Miniatuur {
    pub breedte: u32,
    pub hoogte: u32,
    /// BGRA, `breedte * hoogte * 4` bytes. Achter een `Arc` omdat dit elke tik opnieuw
    /// de motor in en de snapshot in gekopieerd wordt; zonder dat zou elke publicatie
    /// een paar honderd kilobyte per bekeken stream kopiëren in plaats van een
    /// refcount op te hogen.
    pub data: Arc<[u8]>,
}

#[derive(Debug, Clone)]
pub struct KijkerConfig {
    pub stream_id: u32,
    pub titel: String,
    /// Aangekondigde afmeting. Wijkt het echte beeld af, dan wint het echte beeld.
    pub breedte: u32,
    pub hoogte: u32,
    pub codec: Codec,
    /// Alleen pakketten van dit adres tellen mee. Op een open UDP-poort komt vroeg of
    /// laat iets anders binnen, en dat mag nooit in de decoder terechtkomen.
    pub afzender: IpAddr,
}

struct Gedeeld {
    stop: AtomicBool,
    beelden: AtomicU64,
    kapot: AtomicU64,
    /// Tijd tussen opnemen en tonen, in microseconden. Zie [`KijkerHandle::vertraging`].
    vertraging_us: AtomicU64,
}

pub struct KijkerHandle {
    gedeeld: Arc<Gedeeld>,
    /// De poort waarop deze kijker beeld verwacht; hoort in `StreamSubscribe`.
    pub poort: u16,
    pub events: Receiver<KijkerEvent>,
}

impl KijkerHandle {
    /// Aantal getoonde beelden en het aantal dat onderweg sneuvelde. Voor `StreamStats`
    /// en om in de UI te kunnen zien of de verbinding het aankan.
    pub fn tellers(&self) -> (u64, u64) {
        (
            self.gedeeld.beelden.load(Ordering::Relaxed),
            self.gedeeld.kapot.load(Ordering::Relaxed),
        )
    }

    /// Tijd tussen het opnemen van een beeld en het tonen ervan: opnemen, coderen,
    /// versturen, samenstellen, decoderen, presenteren.
    ///
    /// **Alleen geldig als deler en kijker in hetzelfde proces draaien.** De tijdstempel
    /// op de draad hangt aan de klok van de deler, en tussen twee machines loopt die
    /// niet gelijk. Dit is dus een meetinstrument voor de ketentest, geen waarde om in
    /// de UI te zetten.
    pub fn vertraging(&self) -> Duration {
        Duration::from_micros(self.gedeeld.vertraging_us.load(Ordering::Relaxed))
    }
}

impl Drop for KijkerHandle {
    fn drop(&mut self) {
        self.gedeeld.stop.store(true, Ordering::Relaxed);
        tracing::info!("kijken gestopt");
    }
}

/// Opent een venster en begint te luisteren. De poort in de handle moet naar de deler,
/// anders komt er nooit beeld.
pub fn kijk(d3d: &D3dContext, cfg: KijkerConfig) -> Result<KijkerHandle> {
    let socket = MediaSocket::bind(0).context("mediapoort voor video")?;
    socket.zet_timeout(ONTVANG_TIMEOUT)?;
    let poort = socket.local_addr()?.port();

    let gedeeld = Arc::new(Gedeeld {
        stop: AtomicBool::new(false),
        beelden: AtomicU64::new(0),
        kapot: AtomicU64::new(0),
        vertraging_us: AtomicU64::new(0),
    });

    // Het venster en de decoder moeten leven op de thread die ze bedient, dus we
    // wachten hier af of dat gelukt is. Zo krijgt de gebruiker een nette melding in
    // plaats van een venster dat er nooit komt.
    let (klaar_tx, klaar_rx) = bounded::<Result<()>>(1);
    let (event_tx, events) = bounded(16);

    let d3d = d3d.clone();
    let staat = gedeeld.clone();
    let stream_id = cfg.stream_id;
    std::thread::Builder::new()
        .name(format!("fitcom-kijk-{stream_id}"))
        .spawn(move || match opzetten(&d3d, &cfg) {
            Ok((venster, decoder)) => {
                let _ = klaar_tx.send(Ok(()));
                kijk_lus(venster, decoder, &d3d, socket, &cfg, &staat, &event_tx);
                let _ = event_tx.send(KijkerEvent::Gesloten);
            }
            Err(e) => {
                let _ = klaar_tx.send(Err(e));
            }
        })
        .context("kijk-thread starten")?;

    klaar_rx
        .recv_timeout(Duration::from_secs(10))
        .context("videovenster reageert niet")??;

    tracing::info!(stream = stream_id, poort, "kijken gestart");

    Ok(KijkerHandle {
        gedeeld,
        poort,
        events,
    })
}

fn opzetten(d3d: &D3dContext, cfg: &KijkerConfig) -> Result<(Venster, Decoder)> {
    let decoder = Decoder::new(d3d, cfg.codec, cfg.breedte, cfg.hoogte)?;
    let venster = Venster::open(d3d, &cfg.titel, cfg.breedte, cfg.hoogte)?;
    Ok((venster, decoder))
}

fn kijk_lus(
    mut venster: Venster,
    mut decoder: Decoder,
    d3d: &D3dContext,
    socket: MediaSocket,
    cfg: &KijkerConfig,
    gedeeld: &Arc<Gedeeld>,
    events: &Sender<KijkerEvent>,
) {
    let mut samensteller = Reassembler::new();
    let mut buf = [0u8; fitcom_net::MAX_PAKKET];

    // Aanhaken kan alleen op een keyframe. Bij de start is dat er nog niet, dus we
    // vragen er meteen om: als de deler al voor iemand anders bezig was, staat het
    // volgende keyframe anders misschien seconden verderop.
    let mut wacht_op_keyframe = true;
    let mut laatst_gevraagd = Instant::now() - KEYFRAME_PAUZE;
    let mut laatste_incompleet = 0u64;
    let mut laatste_pomp = Instant::now();
    let mut laatste_miniatuur = Instant::now() - MINIATUUR_INTERVAL;

    // Eigen ijkpunten voor de meter: `laatste_incompleet` hierboven stuurt het
    // keyframe-herstel aan en wordt alleen bijgewerkt als een beeld compleet werd. De
    // meter moet elke verandering zien, ook die van beelden die nooit afkwamen.
    let mut meter = Meter::nieuw(cfg.stream_id);
    let (mut gemeten_incompleet, mut gemeten_verworpen) = (0u64, 0u64);

    while !gedeeld.stop.load(Ordering::Relaxed) {
        meter.tik();

        // Het venster bedienen mag niet bij elk pakket: bij 1080p60 zijn dat er
        // duizenden per seconde en dan doen we niets anders meer.
        if laatste_pomp.elapsed() >= Duration::from_millis(8) {
            laatste_pomp = Instant::now();
            if !venster.pomp() {
                break;
            }
        }

        if wacht_op_keyframe && laatst_gevraagd.elapsed() >= KEYFRAME_PAUZE {
            laatst_gevraagd = Instant::now();
            meter.keyframe_verzoeken += 1;
            let _ = events.try_send(KijkerEvent::KeyframeNodig);
        }

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
        if header.stream_id != cfg.stream_id || van.ip() != cfg.afzender {
            continue;
        }
        meter.fragmenten += 1;
        meter.bytes += (fitcom_proto::MEDIA_HEADER_LEN + payload.len()) as u64;

        let klaar = samensteller.push(&header, payload);

        meter.incompleet += samensteller.incompleet - gemeten_incompleet;
        meter.verworpen += samensteller.verworpen - gemeten_verworpen;
        gemeten_incompleet = samensteller.incompleet;
        gemeten_verworpen = samensteller.verworpen;

        let Some(frame) = klaar else {
            continue;
        };

        // Kapotte beelden tellen op; loopt dat op, dan missen we stukken en heeft
        // doorgaan geen zin tot er een nieuw keyframe is.
        if samensteller.incompleet > laatste_incompleet {
            gedeeld
                .kapot
                .store(samensteller.incompleet, Ordering::Relaxed);
            laatste_incompleet = samensteller.incompleet;
            if !wacht_op_keyframe {
                wacht_op_keyframe = true;
                decoder.spoel();
                laatst_gevraagd = Instant::now() - KEYFRAME_PAUZE;
            }
        }

        if wacht_op_keyframe {
            if !frame.keyframe {
                continue;
            }
            wacht_op_keyframe = false;
        }

        let tijd_hns = naar_hns(frame.timestamp);
        let voor_decode = Instant::now();
        let uit_decoder = decoder.decode(&frame.data, tijd_hns);
        meter.decode_us += voor_decode.elapsed().as_micros() as u64;
        match uit_decoder {
            Ok(Some(beeld)) => {
                let voor_toon = Instant::now();
                if let Err(e) = venster.toon(&beeld) {
                    tracing::error!(error = %format!("{e:#}"), "beeld tonen mislukt");
                    break;
                }
                meter.toon_us += voor_toon.elapsed().as_micros() as u64;
                meter.getoond += 1;
                gedeeld.beelden.fetch_add(1, Ordering::Relaxed);
                meet_vertraging(gedeeld, tijd_hns);

                if laatste_miniatuur.elapsed() >= MINIATUUR_INTERVAL {
                    laatste_miniatuur = Instant::now();
                    match maak_miniatuur(d3d, &beeld) {
                        Ok(m) => {
                            let _ = events.try_send(KijkerEvent::Miniatuur(m));
                        }
                        Err(e) => {
                            tracing::debug!(error = %format!("{e:#}"), "miniatuur maken mislukt");
                        }
                    }
                }
            }
            Ok(None) => {}
            Err(e) => {
                // Een beschadigd beeld laat de decoder struikelen. Opnieuw beginnen bij
                // het volgende keyframe is hier het herstel, niet stoppen.
                tracing::warn!(error = %format!("{e:#}"), "beeld decoderen mislukt");
                decoder.spoel();
                wacht_op_keyframe = true;
            }
        }
    }
}

/// De tegenhanger van de meter bij de deler: één regel per seconde, op `info`.
///
/// Dit is de kant die de deler niet kan zien. `incompleet` boven nul betekent dat er
/// fragmenten onderweg sneuvelen; `keyframe_verzoeken` telt hoe vaak we de deler daarom
/// om een nieuw keyframe vragen. Lopen die twee samen op, dan zit je in de lus waarbij
/// elk keyframe een burst is die zelf weer verlies veroorzaakt.
struct Meter {
    stream_id: u32,
    sinds: Instant,
    fragmenten: u32,
    bytes: u64,
    getoond: u32,
    incompleet: u64,
    verworpen: u64,
    keyframe_verzoeken: u32,
    decode_us: u64,
    toon_us: u64,
}

impl Meter {
    fn nieuw(stream_id: u32) -> Self {
        Self {
            stream_id,
            sinds: Instant::now(),
            fragmenten: 0,
            bytes: 0,
            getoond: 0,
            incompleet: 0,
            verworpen: 0,
            keyframe_verzoeken: 0,
            decode_us: 0,
            toon_us: 0,
        }
    }

    fn tik(&mut self) {
        let dt = self.sinds.elapsed();
        if dt < Duration::from_secs(1) {
            return;
        }
        let s = dt.as_secs_f64();
        let per_beeld = |us: u64| {
            if self.getoond == 0 {
                0.0
            } else {
                (us as f64 / self.getoond as f64 / 100.0).round() / 10.0
            }
        };
        tracing::info!(
            stream = self.stream_id,
            getoond_fps = (self.getoond as f64 / s).round() as u32,
            mbit = ((self.bytes as f64 * 8.0 / s / 1e5).round() / 10.0),
            frag_per_s = (self.fragmenten as f64 / s).round() as u32,
            incompleet = self.incompleet,
            verworpen = self.verworpen,
            keyframe_verzoeken = self.keyframe_verzoeken,
            decode_ms = per_beeld(self.decode_us),
            toon_ms = per_beeld(self.toon_us),
            "kijker"
        );
        *self = Meter::nieuw(self.stream_id);
    }
}

/// Hoe lang dit beeld erover deed van opnemen tot tonen.
///
/// Alleen zinnig als de deler dezelfde klok gebruikt als wij — dus in hetzelfde proces.
/// Tussen twee machines levert dit onzin op, en daarom staat het nergens in de UI.
fn meet_vertraging(gedeeld: &Arc<Gedeeld>, opgenomen_hns: i64) {
    let nu_hns = (crate::deler::klok_nulpunt().elapsed().as_nanos() / 100) as i64;
    let verschil = nu_hns - opgenomen_hns;
    if verschil <= 0 || verschil > 10 * crate::codec::HNS_PER_SEC {
        return; // klokken van verschillende machines; niets te meten
    }
    // Voortschrijdend gemiddelde: één beeld dat toevallig achterliep zegt niets, het
    // gaat om waar de keten gemiddeld op uitkomt.
    let nieuw = (verschil / 10) as u64;
    let oud = gedeeld.vertraging_us.load(Ordering::Relaxed);
    let gemiddeld = if oud == 0 {
        nieuw
    } else {
        (oud * 7 + nieuw) / 8
    };
    gedeeld.vertraging_us.store(gemiddeld, Ordering::Relaxed);
}

/// Van de 90 kHz-klok op de draad terug naar de eenheden van Media Foundation.
fn naar_hns(tijdstempel: u32) -> i64 {
    (u64::from(tijdstempel) * crate::codec::HNS_PER_SEC as u64 / 90_000) as i64
}

/// Verkleint het getoonde beeld tot een miniatuur voor het overzicht in het
/// hoofdvenster. Loopt via `D3dContext::lees_bgra_miniatuur`, dat alleen de nodige
/// pixels uit de uitleestextuur bemonstert in plaats van het hele beeld te kopiëren.
fn maak_miniatuur(
    d3d: &D3dContext,
    beeld: &windows::Win32::Graphics::Direct3D11::ID3D11Texture2D,
) -> Result<Miniatuur> {
    let (bron_b, bron_h) = crate::d3d::afmetingen(beeld);
    let hoogte = ((MINIATUUR_BREEDTE as u64 * bron_h as u64) / bron_b.max(1) as u64).max(1) as u32;
    let data = d3d.lees_bgra_miniatuur(beeld, MINIATUUR_BREEDTE, hoogte)?;
    Ok(Miniatuur {
        breedte: MINIATUUR_BREEDTE,
        hoogte,
        data: data.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tijdstempel_overleeft_de_heen_en_terugweg() {
        // De deler rekent naar 90 kHz, de kijker terug. Loopt dat uit de pas, dan
        // krijgt de decoder tijden die niet oplopen en gaat hij beelden weggooien.
        for beeld in 0..600i64 {
            let hns = beeld * (crate::codec::HNS_PER_SEC / 60);
            let heen = crate::deler::naar_klok(hns);
            let terug = naar_hns(heen);
            assert!(
                (terug - hns).abs() <= crate::codec::HNS_PER_SEC / 90_000,
                "beeld {beeld}: {hns} werd {terug}"
            );
        }
    }
}
