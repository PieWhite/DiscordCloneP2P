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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KijkerEvent {
    /// De gebruiker heeft het venster gesloten.
    Gesloten,
    /// We zijn de draad kwijt; de deler moet een keyframe sturen.
    KeyframeNodig,
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
                kijk_lus(venster, decoder, socket, &cfg, &staat, &event_tx);
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

    while !gedeeld.stop.load(Ordering::Relaxed) {
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

        let Some(frame) = samensteller.push(&header, payload) else {
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
        match decoder.decode(&frame.data, tijd_hns) {
            Ok(Some(beeld)) => {
                if let Err(e) = venster.toon(&beeld) {
                    tracing::error!(error = %format!("{e:#}"), "beeld tonen mislukt");
                    break;
                }
                gedeeld.beelden.fetch_add(1, Ordering::Relaxed);
                meet_vertraging(gedeeld, tijd_hns);
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
