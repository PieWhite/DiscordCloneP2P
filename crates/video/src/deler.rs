//! De kant die deelt: opnemen, coderen, versturen.
//!
//! ```text
//! scherm ─► WGC ─► D3D11-textuur ─► encoder ─► fragmenteren ─► UDP naar elke kijker
//! ```
//!
//! Alles op één thread, en die thread bestaat alleen zolang er iemand kijkt. Dat is de
//! kern van de afspraak dat delen niets kost als niemand meekijkt: er wordt dan niet
//! opgenomen, niet gecodeerd en niets verstuurd.
//!
//! # Waarom er nergens een lock op het beeldpad zit
//!
//! Het beeld blijft van begin tot eind op de GPU en gaat via een enkele thread naar
//! buiten. Wat van buiten kan veranderen — wie er kijkt, of er een keyframe nodig is —
//! staat in een klein stukje gedeelde staat dat één keer per beeld gelezen wordt.

use crate::capture::{Bron, Capture};
use crate::codec::{Codec, Encoder, EncoderConfig, HNS_PER_SEC};
use crate::d3d::D3dContext;
use crate::fragment::headers_voor;
use anyhow::{Context, Result};
use fitcom_net::MediaSocket;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Zo lang wachten we op een volgend beeld voordat we even kijken of we nog door
/// moeten gaan. Een stilstaand scherm levert geen frames, en dat is geen fout.
const FRAME_WACHT: Duration = Duration::from_millis(100);

/// De tijdrekening op de draad voor video, zoals in `docs/ARCHITECTURE.md`.
const KLOK_HZ: i64 = 90_000;

/// Het nulpunt van de tijdstempels, één keer per proces.
///
/// Eén klok voor alle streams van deze peer, niet één per deler. Dat maakt de
/// tijdstempels onderling vergelijkbaar, en het maakt de latency van de hele keten
/// meetbaar zodra deler en kijker in hetzelfde proces draaien — zie
/// `crates/video/tests/keten.rs`. **Tussen twee machines zegt dit niets**: die klokken
/// lopen niet gelijk, en daar synchroniseren zou een tijdserver vragen die we niet
/// hebben en niet willen.
pub fn klok_nulpunt() -> Instant {
    static NULPUNT: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    *NULPUNT.get_or_init(Instant::now)
}

#[derive(Debug, Clone)]
pub struct DelerConfig {
    pub stream_id: u32,
    pub bron: Bron,
    pub codec: Codec,
    pub fps: u32,
    pub bitrate: u32,
}

struct Gedeeld {
    kijkers: Mutex<Vec<SocketAddr>>,
    keyframe_gevraagd: AtomicBool,
    stop: AtomicBool,
    /// Voor de UI: hoeveel beelden we tot nu toe verstuurd hebben.
    beelden: std::sync::atomic::AtomicU64,
}

pub struct DelerHandle {
    gedeeld: Arc<Gedeeld>,
    afmeting: (u32, u32),
}

impl DelerHandle {
    /// Waar het beeld heen moet. Mag op elk moment wijzigen: een kijker die erbij komt
    /// of wegvalt is de normale gang van zaken.
    pub fn zet_kijkers(&self, doelen: Vec<SocketAddr>) {
        if let Ok(mut k) = self.gedeeld.kijkers.lock() {
            *k = doelen;
        }
    }

    /// Het volgende beeld als keyframe versturen. Een kijker die de draad kwijt is
    /// blijft anders naar vlekken kijken tot de volgende periodieke IDR.
    pub fn vraag_keyframe(&self) {
        self.gedeeld
            .keyframe_gevraagd
            .store(true, Ordering::Relaxed);
    }

    pub fn afmeting(&self) -> (u32, u32) {
        self.afmeting
    }

    pub fn beelden(&self) -> u64 {
        self.gedeeld.beelden.load(Ordering::Relaxed)
    }
}

impl Drop for DelerHandle {
    fn drop(&mut self) {
        self.gedeeld.stop.store(true, Ordering::Relaxed);
        tracing::info!("delen gestopt");
    }
}

/// Start opnemen en coderen. De thread stopt zodra de handle wordt losgelaten.
pub fn deel(d3d: &D3dContext, cfg: DelerConfig, kijkers: Vec<SocketAddr>) -> Result<DelerHandle> {
    let afmeting = crate::capture::afmeting_van(&cfg.bron)?;

    let gedeeld = Arc::new(Gedeeld {
        kijkers: Mutex::new(kijkers),
        keyframe_gevraagd: AtomicBool::new(false),
        stop: AtomicBool::new(false),
        beelden: std::sync::atomic::AtomicU64::new(0),
    });

    let d3d = d3d.clone();
    let staat = gedeeld.clone();
    std::thread::Builder::new()
        .name(format!("fitcom-deel-{}", cfg.stream_id))
        .spawn(move || {
            if let Err(e) = deel_lus(&d3d, &cfg, &staat) {
                tracing::error!(error = %format!("{e:#}"), stream = cfg.stream_id, "delen gestopt door een fout");
            }
        })
        .context("deel-thread starten")?;

    Ok(DelerHandle { gedeeld, afmeting })
}

fn deel_lus(d3d: &D3dContext, cfg: &DelerConfig, gedeeld: &Arc<Gedeeld>) -> Result<()> {
    let socket = MediaSocket::bind(0).context("uitgaande mediapoort")?;
    let mut capture = Capture::start(d3d, &cfg.bron)?;
    let (breedte, hoogte) = capture.afmeting();

    let mut encoder = Encoder::new(
        d3d,
        &EncoderConfig {
            codec: cfg.codec,
            breedte,
            hoogte,
            fps: cfg.fps,
            bitrate: cfg.bitrate,
        },
    )?;

    let payload_type = cfg.codec.payload_type();
    let begin = klok_nulpunt();
    let mut seq: u32 = 0;
    let mut meter = Meter::nieuw(cfg.stream_id, cfg.bron.naam.clone());

    let mut pacer = Pacer::nieuw(cfg.fps, crate::capture::verversing_van(&cfg.bron));

    while !gedeeld.stop.load(Ordering::Relaxed) {
        meter.tik();
        let Some(mut beeld) = capture.volgende_frame(FRAME_WACHT) else {
            continue;
        };
        meter.opgenomen += 1;

        // Staat er meer klaar, dan liepen we achter en is alles behalve het laatste oud
        // nieuws. Het verste beeld coderen scheelt precies die achterstand aan
        // vertraging, en de beelden ertussen zou de kijker toch nooit los zien.
        while let Some(nieuwer) = capture.volgende_frame(Duration::ZERO) {
            beeld = nieuwer;
            meter.opgenomen += 1;
        }

        if !pacer.laat_door(Instant::now()) {
            continue;
        }

        if gedeeld.keyframe_gevraagd.swap(false, Ordering::Relaxed) {
            encoder.vraag_keyframe();
        }

        let tijd_hns = (begin.elapsed().as_nanos() / 100) as i64;
        let pakketten = match encoder.encode(&beeld, tijd_hns) {
            Ok(p) => p,
            Err(e) => {
                // Eén mislukt beeld is geen reden om te stoppen met delen; de encoder
                // vangt zichzelf op zodra er weer een keyframe langskomt.
                tracing::warn!(error = %format!("{e:#}"), "beeld coderen mislukt");
                encoder.vraag_keyframe();
                continue;
            }
        };

        let kijkers = gedeeld
            .kijkers
            .lock()
            .map(|k| k.clone())
            .unwrap_or_default();
        if kijkers.is_empty() {
            // Niemand meer aan de andere kant. De motor ruimt ons zo op; tot die tijd
            // heeft versturen geen zin.
            continue;
        }

        for pakket in pakketten {
            meter.verstuurd += 1;
            meter.keyframes += u32::from(pakket.keyframe);
            meter.grootste = meter.grootste.max(pakket.data.len());

            let tijdstempel = naar_klok(pakket.tijd_hns);
            for (header, stuk) in headers_voor(
                cfg.stream_id,
                tijdstempel,
                payload_type,
                pakket.keyframe,
                seq,
                &pakket.data,
            ) {
                seq = seq.wrapping_add(1);
                meter.fragmenten += 1;
                meter.bytes += (fitcom_proto::MEDIA_HEADER_LEN + stuk.len()) as u64;
                for &kijker in &kijkers {
                    if let Err(e) = socket.stuur(kijker, &header, stuk) {
                        meter.niet_verstuurd += 1;
                        tracing::debug!(%kijker, error = %e, "videofragment niet verstuurd");
                    }
                }
            }
            gedeeld.beelden.fetch_add(1, Ordering::Relaxed);
        }
    }

    Ok(())
}

/// Houdt het aantal beelden per seconde op `cfg.fps`.
///
/// WGC levert op het tempo van de monitor en trekt zich niets van `fps` aan. Zonder dit
/// gaat er op een 144-165 Hz-scherm ruim twee tot drie keer zoveel de draad op, en dan
/// klopt niets meer: de bitrate niet, en de afstand tussen keyframes ook niet — de
/// encoder telt die in beelden, dus meer beelden per seconde is ook meer keyframes per
/// seconde, elk een stoot van honderden kilobytes.
///
/// # Waarom het doeltempo niet altijd gehaald wordt, en dat goed is
///
/// Uit 144 Hz zijn geen gelijkmatige 60 beelden per seconde te halen: 144 ÷ 60 is 2,4 en
/// je kunt geen 2,4 schermbeelden overslaan. Je krijgt er dan wel zestig, maar met
/// afstanden die heen en weer springen tussen 13,9 en 20,8 ms. Het *aantal* klopt en het
/// **oogt toch als haperen**, en dat is precies waarom elk onderzoek naar fps en bitrate
/// hier langs is gelopen. De kijker heeft geen jitterbuffer voor beeld — die toont elk
/// beeld zodra het compleet is — dus die ongelijkheid komt onverdund op het scherm.
///
/// Daarom klikt het interval vast op een heel veelvoud van de schermperiode. Op 144 Hz
/// wordt dat elk derde beeld: 48 per seconde, exact 20,8 ms uit elkaar. Gelijkmatige 48
/// ziet er beter uit dan ongelijkmatige 60. Op 180 Hz (drie keer 60) verandert er niets.
///
/// Twee valkuilen, allebei in de tests hieronder vastgelegd:
///
/// 1. De volgende deadline telt op bij de *vorige*, niet bij `nu`. Bij `nu` rondt elk
///    interval naar boven af op een schermperiode en zakt het tempo nog verder weg.
/// 2. Bij een achterstand schuift de deadline naar `nu` en niet naar `nu + interval`.
///    Anders raak je beelden kwijt die je juist wilde hebben: staat het scherm stil en
///    komt er daarna een tweetal beelden vlak achter elkaar, dan is dat tweede het
///    verste beeld en gooi je dat weg terwijl je onder het doeltempo zit.
struct Pacer {
    interval: Duration,
    deadline: Instant,
}

impl Pacer {
    /// `scherm_hz` komt van [`crate::capture::verversing_van`]. Is die onbekend, dan
    /// houden we het gevraagde tempo aan en accepteren we de ongelijkmatigheid — beter
    /// dan gokken.
    fn nieuw(fps: u32, scherm_hz: Option<u32>) -> Self {
        let doel = Duration::from_nanos(1_000_000_000 / u64::from(fps.max(1)));
        let interval = match scherm_hz.filter(|hz| *hz > 0) {
            Some(hz) => {
                let periode = Duration::from_nanos(1_000_000_000 / u64::from(hz));
                // Twee procent speling voordat we naar boven afronden. Zonder dat wordt
                // 180 Hz, waar drie schermbeelden exact 60 per seconde zijn, door een
                // afrondingsrest van een nanoseconde alsnog vier beelden en dus 45.
                let ondergrens = doel.as_nanos() * 98 / 100;
                let stappen = ondergrens.div_ceil(periode.as_nanos()).max(1);
                periode * stappen as u32
            }
            None => doel,
        };

        tracing::info!(
            scherm_hz = scherm_hz.unwrap_or(0),
            gevraagd_fps = fps,
            beelden_per_s = (1.0 / interval.as_secs_f64()).round() as u32,
            "tempo vastgeklikt op de schermverversing"
        );

        Self {
            interval,
            deadline: Instant::now(),
        }
    }

    fn laat_door(&mut self, nu: Instant) -> bool {
        if nu < self.deadline {
            return false;
        }
        self.deadline += self.interval;
        if self.deadline + self.interval < nu {
            self.deadline = nu;
        }
        true
    }
}

/// Eén regel per seconde per stream, op `info`. Zonder deze getallen blijft elke
/// uitspraak over haperend beeld een gok: opnemen, coderen en versturen zijn alle drie
/// verdachte, en alleen de verhouding ertussen wijst de dader aan.
///
/// Let op `niet_verstuurd`: dat zijn fragmenten die de socket weigerde. Boven nul is de
/// verzendkant de bron van het verlies, niet de lijn.
struct Meter {
    stream_id: u32,
    bron: String,
    sinds: Instant,
    opgenomen: u32,
    verstuurd: u32,
    keyframes: u32,
    fragmenten: u32,
    bytes: u64,
    grootste: usize,
    niet_verstuurd: u32,
}

impl Meter {
    fn nieuw(stream_id: u32, bron: String) -> Self {
        Self {
            stream_id,
            bron,
            sinds: Instant::now(),
            opgenomen: 0,
            verstuurd: 0,
            keyframes: 0,
            fragmenten: 0,
            bytes: 0,
            grootste: 0,
            niet_verstuurd: 0,
        }
    }

    fn tik(&mut self) {
        let dt = self.sinds.elapsed();
        if dt < Duration::from_secs(1) {
            return;
        }
        let s = dt.as_secs_f64();
        tracing::info!(
            stream = self.stream_id,
            bron = %self.bron,
            opgenomen_fps = (self.opgenomen as f64 / s).round() as u32,
            verstuurd_fps = (self.verstuurd as f64 / s).round() as u32,
            mbit = ((self.bytes as f64 * 8.0 / s / 1e5).round() / 10.0),
            keyframes = self.keyframes,
            grootste_kb = self.grootste / 1024,
            frag_per_s = (self.fragmenten as f64 / s).round() as u32,
            niet_verstuurd = self.niet_verstuurd,
            "deler"
        );
        *self = Meter::nieuw(self.stream_id, std::mem::take(&mut self.bron));
    }
}

/// Van de 100-nanoseconden-klok van Media Foundation naar de 90 kHz-klok op de draad.
pub(crate) fn naar_klok(tijd_hns: i64) -> u32 {
    (tijd_hns.max(0) as u64)
        .wrapping_mul(KLOK_HZ as u64)
        .wrapping_div(HNS_PER_SEC as u64) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Speelt een scherm van `bron_hz` af en levert de tijdstippen van de doorgelaten
    /// beelden. Dit is de enige manier om aan de pacer te rekenen zonder scherm.
    fn speel_af(doel_fps: u32, bron_hz: u64, seconden: u64) -> (Instant, Vec<Instant>) {
        let mut pacer = Pacer::nieuw(doel_fps, Some(bron_hz as u32));
        let begin = Instant::now();
        let stap = Duration::from_nanos(1_000_000_000 / bron_hz);
        pacer.deadline = begin;

        let mut door = Vec::new();
        for n in 0..bron_hz * seconden {
            let nu = begin + stap * (n as u32);
            if pacer.laat_door(nu) {
                door.push(nu);
            }
        }
        (begin, door)
    }

    /// Standaardafwijking van de afstand tussen opeenvolgende beelden, in milliseconden.
    /// Dit getal is waar het om gaat: het aantal beelden kan kloppen terwijl dit hoog is,
    /// en dan ziet het er alsnog uit als haperen.
    fn spreiding_ms(door: &[Instant]) -> f64 {
        let intervallen: Vec<f64> = door
            .windows(2)
            .map(|w| w[1].duration_since(w[0]).as_secs_f64() * 1000.0)
            .collect();
        let gem = intervallen.iter().sum::<f64>() / intervallen.len() as f64;
        (intervallen.iter().map(|i| (i - gem).powi(2)).sum::<f64>() / intervallen.len() as f64)
            .sqrt()
    }

    /// Hoeveel beelden er doorheen komen als de opname op `bron_hz` aanlevert.
    fn door_bij(doel_fps: u32, bron_hz: u64, seconden: u64) -> u32 {
        speel_af(doel_fps, bron_hz, seconden).1.len() as u32
    }

    #[test]
    fn beelden_staan_gelijkmatig_uit_elkaar_op_elke_verversing() {
        // Dit is de test die ertoe doet. Ongelijkmatige 60 beelden per seconde zien er
        // uit als haperen terwijl elke fps-meting zegt dat het goed gaat — precies wat
        // het onderzoek naar de screenshare-hapering zo lang heeft opgehouden. Zonder
        // vastklikken op de schermperiode staat 144 Hz hier op 3,4 ms.
        for hz in [120u64, 144, 165, 180, 240] {
            let (_, door) = speel_af(60, hz, 10);
            let spreiding = spreiding_ms(&door);
            assert!(
                spreiding < 0.5,
                "op {hz} Hz staan de beelden {spreiding:.2} ms ongelijk uit elkaar"
            );
        }
    }

    #[test]
    fn het_tempo_klikt_vast_op_een_heel_aantal_schermbeelden() {
        // 144 ÷ 60 is 2,4 en je kunt geen 2,4 schermbeelden overslaan. Elk derde beeld
        // is 48 per seconde: minder dan gevraagd, maar wél gelijkmatig. Boven het
        // doeltempo uitkomen mag nooit, want dat is bitrate die niemand gevraagd heeft.
        for (hz, verwacht) in [(120u64, 60u32), (144, 48), (165, 55), (180, 60), (240, 60)] {
            let door = door_bij(60, hz, 10) / 10;
            assert_eq!(door, verwacht, "op {hz} Hz");
            assert!(door <= 60, "op {hz} Hz gaat {door} boven het doeltempo");
        }
    }

    #[test]
    fn onder_het_doeltempo_gaat_alles_door() {
        // Een rustig scherm levert minder dan 60 beelden per seconde. Daar hoort niets
        // van weggegooid te worden.
        assert_eq!(door_bij(60, 12, 5), 60, "12 Hz moet volledig doorgaan");
        assert_eq!(door_bij(60, 30, 5), 150, "30 Hz moet volledig doorgaan");
    }

    #[test]
    fn na_stilstand_geen_stoot_beelden() {
        let mut pacer = Pacer::nieuw(60, Some(60));
        let begin = Instant::now();
        pacer.deadline = begin;
        assert!(pacer.laat_door(begin));

        // Vijf seconden niets, en dan ineens tien beelden binnen een milliseconde.
        let na = begin + Duration::from_secs(5);
        let door = (0..10)
            .filter(|n| pacer.laat_door(na + Duration::from_micros(100 * n)))
            .count();
        assert!(
            door <= 2,
            "{door} van de 10 beelden uit één stoot doorgelaten; dat is de achterstand \
             inhalen met een piek en precies wat we niet willen"
        );
    }

    #[test]
    fn klok_rekent_naar_negentig_kilohertz() {
        assert_eq!(naar_klok(0), 0);
        assert_eq!(naar_klok(HNS_PER_SEC), 90_000, "één seconde");
        // Een beeld op 60 fps duurt 166666 eenheden — de deling laat een rest achter,
        // dus 1499 in plaats van 1500. Dat mag: de tijdstempel hoeft niet exact te
        // zijn, hij moet uniek en oplopend zijn.
        assert_eq!(naar_klok(HNS_PER_SEC / 60), 1499);
    }

    #[test]
    fn opeenvolgende_beelden_krijgen_verschillende_tijdstempels() {
        // Vallen twee beelden op dezelfde tijdstempel, dan plakt de ontvanger hun
        // fragmenten aan elkaar tot één onzinnig beeld.
        let stap = HNS_PER_SEC / 60;
        let stempels: Vec<u32> = (0..120).map(|i| naar_klok(i * stap)).collect();
        assert!(stempels.windows(2).all(|w| w[1] > w[0]));
    }
}
