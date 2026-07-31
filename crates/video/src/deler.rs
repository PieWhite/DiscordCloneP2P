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
    // Het nulpunt van de opnameklok naast dat van de onze, gezet bij het eerste beeld.
    let mut nulpunten: Option<(i64, i64)> = None;
    let mut vorige_tijd: i64 = -1;

    let mut pacer = Pacer::nieuw(cfg.fps);

    while !gedeeld.stop.load(Ordering::Relaxed) {
        meter.tik();
        let Some(mut opname) = capture.volgende_frame(FRAME_WACHT) else {
            continue;
        };
        meter.opgenomen += 1;

        // Staat er meer klaar, dan liepen we achter en is alles behalve het laatste oud
        // nieuws. Het verste beeld coderen scheelt precies die achterstand aan
        // vertraging, en de beelden ertussen zou de kijker toch nooit los zien — maar ze
        // tellen wel mee, want het zijn schermbeelden die echt langs zijn gekomen.
        while let Some(nieuwer) = capture.volgende_frame(Duration::ZERO) {
            opname = nieuwer;
            meter.opgenomen += 1;
        }

        if !pacer.laat_door(Instant::now()) {
            continue;
        }

        if gedeeld.keyframe_gevraagd.swap(false, Ordering::Relaxed) {
            encoder.vraag_keyframe();
        }

        // De tijd van de *opname*, niet van nu. Onze lus komt er een wisselend aantal
        // milliseconden later aan toe, en die vertraging hoort niet in de tijdstempel
        // terecht te komen: de kijker plant zijn weergave hierop, dus elke
        // onnauwkeurigheid die we hier instoppen ziet hij terug als haperen.
        //
        // Het nulpunt van de opname-API is willekeurig, dus we leggen het bij het eerste
        // beeld naast het onze. Beide klokken komen van dezelfde QPC en lopen dus niet
        // uit elkaar; alleen het nulpunt verschilt.
        let (opname_nul, ons_nul) = *nulpunten.get_or_insert_with(|| {
            (
                opname.opgenomen_hns,
                (begin.elapsed().as_nanos() / 100) as i64,
            )
        });
        // Strikt oplopend: gelijke tijdstempels plakt de ontvanger tot één beeld aan
        // elkaar. De opnametijden lopen al op, dit is de vangnetregel.
        let tijd_hns = (ons_nul + (opname.opgenomen_hns - opname_nul)).max(vorige_tijd + 1);
        vorige_tijd = tijd_hns;

        let pakketten = match encoder.encode(&opname.textuur, tijd_hns) {
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
/// # Waarom een tijdraster genoeg is, en de verversing er niet toe doet
///
/// Er is hier een tijd lang geprobeerd om het tempo vast te klikken op een heel aantal
/// schermbeelden, omdat uit 144 Hz geen gelijkmatige 60 per seconde te halen zijn:
/// 144 ÷ 60 is 2,4. Dat leverde 48 op, en het was een omweg om een probleem dat ergens
/// anders zat.
///
/// **Ongelijk opgenomen beeld dat ongelijk wordt getoond is niet fout — dat is juist.**
/// Veranderde het scherm op moment t, dan hoort dat beeld op moment t te verschijnen. De
/// kijker doet dat sinds [`crate::kijker::Weergaveklok`]; hij plant op de tijdstempel in
/// plaats van te tonen zodra er iets binnenkomt. Haperen ontstaat door het *verschil*
/// tussen opnametijd en weergavetijd, niet door ongelijkheid op zich.
///
/// Wat dan overblijft is één eis: **vaak genoeg bemonsteren**. Neem je 48 monsters van een
/// filmpje met 60 beelden, dan gooi je er twaalf per seconde weg, en dát zie je — hoe
/// gelijkmatig die 48 ook staan. Vastklikken op de verversing maakte het dus actief
/// erger: dat is precies de regel die 48 van 60 pakt.
///
/// Dus: een gelijkmatig tijdraster van 1/fps, het verste beeld dat er op dat moment is,
/// en de echte opnametijd erop. Zet `fps` hoger dan het tempo van wat je deelt en er gaat
/// niets verloren.
///
/// Twee valkuilen, allebei in de tests hieronder vastgelegd:
///
/// 1. De volgende deadline telt op bij de *vorige*, niet bij `nu`. Bij `nu` rondt elk
///    interval naar boven af op een schermperiode en zakt 144 Hz naar 48 in plaats van 60.
/// 2. Bij een achterstand schuift de deadline naar `nu` en niet naar `nu + interval`.
///    Anders raak je beelden kwijt die je juist wilde hebben: staat het scherm stil en
///    komt er daarna een tweetal beelden vlak achter elkaar, dan is dat tweede het verste
///    en gooi je dat weg terwijl je onder het doeltempo zit.
struct Pacer {
    interval: Duration,
    deadline: Instant,
}

impl Pacer {
    fn nieuw(fps: u32) -> Self {
        Self {
            interval: Duration::from_nanos(1_000_000_000 / u64::from(fps.max(1))),
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
    fn speel_af(doel_fps: u32, bron_hz: u64, seconden: u64) -> Vec<Instant> {
        let mut pacer = Pacer::nieuw(doel_fps);
        let begin = Instant::now();
        let stap = Duration::from_nanos(1_000_000_000 / bron_hz);
        pacer.deadline = begin;

        (0..bron_hz * seconden)
            .map(|n| begin + stap * (n as u32))
            .filter(|nu| pacer.laat_door(*nu))
            .collect()
    }

    /// Hoeveel beelden er per seconde doorheen komen als de opname op `bron_hz` levert.
    fn door_bij(doel_fps: u32, bron_hz: u64, seconden: u64) -> u32 {
        speel_af(doel_fps, bron_hz, seconden).len() as u32 / seconden as u32
    }

    #[test]
    fn het_doeltempo_wordt_gehaald_op_elke_verversing() {
        // De valkuil: rekende de deadline vanaf `nu`, dan rondt elk interval naar boven
        // af op een schermperiode en houd je op 144 Hz 48 over in plaats van 60.
        //
        // Dat die zestig niet precies even ver uit elkaar staan is géén probleem meer:
        // ze dragen hun echte opnametijd en de kijker plant daarop. Wat wél telt is dat
        // er niet stiekem beelden verdwijnen, want elk gemist beeld is er een die de
        // kijker nooit ziet.
        for hz in [120u64, 144, 165, 180, 240] {
            let door = door_bij(60, hz, 10);
            assert!(
                (59..=61).contains(&door),
                "op {hz} Hz komen er {door} per seconde door; verwacht 60"
            );
        }
    }

    #[test]
    fn nooit_meer_dan_gevraagd() {
        // Boven het doeltempo uitkomen is bitrate die niemand gevraagd heeft.
        for hz in [120u64, 144, 165, 180, 240, 360] {
            for fps in [30u32, 60, 72, 90] {
                let door = door_bij(fps, hz, 5);
                assert!(
                    door <= fps + 1,
                    "{fps} fps gevraagd op {hz} Hz, maar er komen er {door} door"
                );
            }
        }
    }

    #[test]
    fn onder_het_doeltempo_gaat_alles_door() {
        // Een rustig scherm levert minder dan 60 beelden per seconde. Daar hoort niets
        // van weggegooid te worden.
        assert_eq!(door_bij(60, 12, 5), 12, "12 Hz moet volledig doorgaan");
        assert_eq!(door_bij(60, 30, 5), 30, "30 Hz moet volledig doorgaan");
    }

    #[test]
    fn na_stilstand_geen_stoot_beelden() {
        // Vijf seconden niets, en dan tien beelden binnen een milliseconde. De
        // achterstand inhalen met een piek is precies wat we niet willen: dat is een
        // stoot bijna identieke beelden op de draad.
        let mut pacer = Pacer::nieuw(60);
        let begin = Instant::now();
        pacer.deadline = begin;
        assert!(pacer.laat_door(begin));

        let na = begin + Duration::from_secs(5);
        let door = (0..10)
            .filter(|n| pacer.laat_door(na + Duration::from_micros(100 * n)))
            .count();
        assert!(
            door <= 2,
            "{door} van de 10 beelden uit één stoot doorgelaten"
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
