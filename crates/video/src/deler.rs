//! De kant die deelt: opnemen, coderen, versturen.
//!
//! ```text
//! scherm ─► WGC ─► D3D11-textuur ─► encoder ─► fragmenteren ─► UDP naar elke kijker
//! ```
//!
//! Alles op één thread, en die thread bestaat alleen zolang er een reden voor is. Dat is de
//! kern van de afspraak dat delen niets kost als niemand meekijkt: er wordt dan niet
//! opgenomen, niet gecodeerd en niets verstuurd.
//!
//! **Eén uitzondering, en die is expliciet:** met [`DelerConfig::voorbeeld`] gezet legt de
//! lus twee keer per seconde een miniatuur neer van wat hij opneemt, en dan is de kijker
//! die de thread rechtvaardigt de gebruiker zelf — hij ziet zichzelf terug in de
//! streamstrook van het hoofdvenster. De camera gebruikt dat; een scherm nooit — daar kijk
//! je al naar. Zonder kijkers wordt er in beide gevallen niet gecodeerd en niets
//! verstuurd: de lus stopt vóór de encoder, niet erna.
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

/// Hoe lang het opruimen van een deler met een exclusieve bron (een camera) op zijn thread
/// wacht. Ruim boven [`FRAME_WACHT`] plus één beeldtijd van de camera, en laag genoeg dat
/// een apparaat dat niet meer reageert de motor niet vasthoudt.
const STOP_WACHT: Duration = Duration::from_millis(500);

/// Hoeveel sneller dan het bitrate-budget er momentaan verstuurd mag worden.
///
/// Gemeten op 2026-08-02: een keyframe van 1080p is 371 kB tegen 6 kB voor een gewoon
/// beeld, en ging er als 346 datagrammen achter elkaar uit in 1,7 ms — momentaan
/// 1,75 Gbit/s op een budget van 8 Mbit/s. Zo'n stoot is precies wat een echt
/// internetpad laat druppelen, en elk gedruppeld pakket kostte honderd milliseconde
/// bevroren beeld.
///
/// Twintig keer het budget laat een gewoon beeld ongemoeid (die past in de speling
/// hieronder) en spreidt een keyframe over ongeveer één beeldtijd in plaats van over
/// niets. Lager zou het keyframe zelf te laat maken; hoger is geen spreiding meer.
const PIEK_FACTOR: u32 = 20;

/// Zoveel bytes mogen er hoe dan ook aaneengesloten de deur uit voordat er geremd wordt.
/// Ruim boven een gewoon beeld, zodat de spreiding alleen keyframes raakt.
const SPELING_BYTES: u64 = 48 * 1024;

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
    /// Een terugblik op wat je zelf uitstuurt: de lus legt twee keer per seconde een
    /// [`Miniatuur`](crate::kijker::Miniatuur) neer die de motor via
    /// [`DelerHandle::miniatuur`] ophaalt en het
    /// hoofdvenster als tegel in de streamstrook toont. `false` is de gewone gang van
    /// zaken voor een gedeeld scherm — daar kijk je al naar.
    ///
    /// Voor een camera staat dit aan: zonder terugblik weet je niet of je in beeld zit, of
    /// het licht klopt, of dat je camera de lensdop nog voorheeft. Hij komt van de textuur
    /// die de encoder toch al krijgt, dus er komt geen tweede opname en geen tweede
    /// decoder aan te pas — dat is ook het enige dat kán, want Media Foundation geeft een
    /// camera niet twee keer uit.
    pub voorbeeld: bool,
}

struct Gedeeld {
    kijkers: Mutex<Vec<SocketAddr>>,
    keyframe_gevraagd: AtomicBool,
    stop: AtomicBool,
    /// De deel-lus is klaar: gestopt op verzoek, of eruit geklapt op een fout. Zonder dit
    /// kan de motor een dode deler niet van een levende onderscheiden, en dat is gaan
    /// tellen zodra een deler ook zonder kijkers bestaat.
    gestopt: AtomicBool,
    /// Waaróp de lus eruit klapte, als hij eruit klapte. De reden waarom dit niet alleen
    /// in de log staat: de opname van een camera begint nu bij het aanzetten, en "camera
    /// in gebruik door Teams" is dan iets waar de gebruiker meteen op wacht. Er is geen
    /// kanaal terug naar de motor voor de deler, dus die leest het hier op zijn tik.
    fout: Mutex<Option<String>>,
    /// Voor de UI: hoeveel beelden we tot nu toe verstuurd hebben.
    beelden: std::sync::atomic::AtomicU64,
    /// De laatste terugblik op wat we opnemen, als [`DelerConfig::voorbeeld`] aan staat.
    /// Twee keer per seconde vervangen; de motor haalt hem op zijn eigen tik op. Een
    /// `Mutex` op het beeldpad zou hier niet mogen, maar dit is het pad niet: de lus zet
    /// er twee keer per seconde een `Arc` in en laat het slot meteen weer los.
    miniatuur: Mutex<Option<crate::kijker::Miniatuur>>,
}

pub struct DelerHandle {
    gedeeld: Arc<Gedeeld>,
    afmeting: (u32, u32),
    /// Of het opruimen op de deel-thread moet wachten. Zie [`DelerHandle::drop`].
    exclusieve_bron: bool,
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

    /// Of de deel-lus er niet meer is. `true` betekent dat deze handle niets meer doet:
    /// er hoeft niet gewacht te worden, hij hoort weggegooid of opnieuw opgezet.
    pub fn gestopt(&self) -> bool {
        self.gedeeld.gestopt.load(Ordering::Relaxed)
    }

    /// Waarop hij eruit klapte, als dat gebeurd is. `None` bij een gewone stop — dan is er
    /// niets te melden.
    pub fn fout(&self) -> Option<String> {
        self.gedeeld.fout.lock().ok().and_then(|f| f.clone())
    }

    /// De laatste terugblik op wat we uitsturen, of `None` als deze deler er geen maakt
    /// ([`DelerConfig::voorbeeld`] uit) of het eerste beeld nog niet binnen is. Kopiëren
    /// kost een refcount: de pixels zitten achter een `Arc`.
    pub fn miniatuur(&self) -> Option<crate::kijker::Miniatuur> {
        self.gedeeld.miniatuur.lock().ok().and_then(|m| m.clone())
    }
}

impl Drop for DelerHandle {
    fn drop(&mut self) {
        self.gedeeld.stop.store(true, Ordering::Relaxed);

        // Een scherm mag je twee keer tegelijk opnemen; een camera geeft Media Foundation
        // maar aan één iemand uit. Voor die tweede soort is "vlag gezet, tot ziens" niet
        // genoeg: wie meteen daarna opnieuw begint — de camera uit en weer aan, of een
        // instellingenwijziging die lopende delers herstart — krijgt dan "in gebruik door
        // iets anders" van zijn eigen vorige thread. Dus wachten tot de lus er echt uit
        // is, met een plafond zodat een vastgelopen apparaat de motor niet meesleept.
        if self.exclusieve_bron {
            let tot = Instant::now() + STOP_WACHT;
            while !self.gedeeld.gestopt.load(Ordering::Relaxed) {
                if Instant::now() >= tot {
                    tracing::warn!("deel-thread stopte niet binnen {STOP_WACHT:?}");
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        tracing::info!("delen gestopt");
    }
}

/// Start opnemen en coderen. De thread stopt zodra de handle wordt losgelaten.
pub fn deel(d3d: &D3dContext, cfg: DelerConfig, kijkers: Vec<SocketAddr>) -> Result<DelerHandle> {
    let afmeting = crate::capture::afmeting_van(&cfg.bron)?;
    let exclusieve_bron = cfg.bron.soort == crate::capture::BronSoort::Camera;

    let gedeeld = Arc::new(Gedeeld {
        kijkers: Mutex::new(kijkers),
        keyframe_gevraagd: AtomicBool::new(false),
        stop: AtomicBool::new(false),
        gestopt: AtomicBool::new(false),
        fout: Mutex::new(None),
        beelden: std::sync::atomic::AtomicU64::new(0),
        miniatuur: Mutex::new(None),
    });

    let d3d = d3d.clone();
    let staat = gedeeld.clone();
    std::thread::Builder::new()
        .name(format!("fitcom-deel-{}", cfg.stream_id))
        .spawn(move || {
            if let Err(e) = deel_lus(&d3d, &cfg, &staat) {
                let bericht = format!("{e:#}");
                tracing::error!(error = %bericht, stream = cfg.stream_id, "delen gestopt door een fout");
                if let Ok(mut f) = staat.fout.lock() {
                    *f = Some(bericht);
                }
            }
            // Op elk pad, ook het foutpad: hierna doet deze deler niets meer. Ná het
            // wegschrijven van de fout, zodat wie `gestopt` ziet hem ook al kan lezen.
            staat.gestopt.store(true, Ordering::Relaxed);
        })
        .context("deel-thread starten")?;

    Ok(DelerHandle {
        gedeeld,
        afmeting,
        exclusieve_bron,
    })
}

fn deel_lus(d3d: &D3dContext, cfg: &DelerConfig, gedeeld: &Arc<Gedeeld>) -> Result<()> {
    let socket = MediaSocket::bind(0).context("uitgaande mediapoort")?;
    let mut capture = Capture::start(d3d, &cfg.bron)?;
    let (breedte, hoogte) = capture.afmeting();

    let encoder_cfg = EncoderConfig {
        codec: cfg.codec,
        breedte,
        hoogte,
        fps: cfg.fps,
        bitrate: cfg.bitrate,
    };
    // Pas aanmaken als er echt iemand kijkt. Een hardware-encodersessie is een schaars
    // goed — op Turing staat de driver er maar een paar tegelijk toe — en een camera met
    // alleen een voorbeeldvenster codeert niets. Zonder dit zou je webcam aanzetten een
    // sessie bezet houden die je bij twee gedeelde schermen nodig hebt.
    //
    // Voor een gedeeld scherm verandert dit niets: dat begint altijd mét een kijker, dus
    // hij staat er bij het eerste beeld.
    let mut encoder: Option<Encoder> = None;

    let payload_type = cfg.codec.payload_type();
    let begin = klok_nulpunt();
    let mut seq: u32 = 0;
    let mut meter = Meter::nieuw(cfg.stream_id, cfg.bron.naam.clone());
    // Het nulpunt van de opnameklok naast dat van de onze, gezet bij het eerste beeld.
    let mut nulpunten: Option<(i64, i64)> = None;
    let mut vorige_tijd: i64 = -1;

    let mut pacer = Pacer::nieuw(cfg.fps);
    let mut tempo = Verzendtempo::nieuw(cfg.bitrate, Instant::now());

    // Wanneer we voor het laatst een terugblik neergelegd hebben. In het verleden gezet,
    // zodat het eerste beeld er meteen een oplevert en de tegel niet een halve seconde
    // leeg blijft.
    let mut laatste_voorbeeld = Instant::now() - crate::kijker::MINIATUUR_INTERVAL;

    let spoor_begin = Instant::now();
    let mut spoor = crate::spoor::Spoor::nieuw(
        &format!("deler-{}", cfg.stream_id),
        "opgenomen_ms,binnen_ms,verstuurd_ms,gedropt,tijd_hns,bytes,keyframe,encode_us,stuur_us",
    );

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
        let mut gedropt = 0u32;
        while let Some(nieuwer) = capture.volgende_frame(Duration::ZERO) {
            opname = nieuwer;
            meter.opgenomen += 1;
            gedropt += 1;
        }
        let binnen = Instant::now();

        // Jezelf terugzien loopt op hetzelfde tempo als de tegel van iemand anders:
        // dezelfde `maak_miniatuur` op dezelfde 2 Hz, alleen van de opnametextuur in
        // plaats van van een gedecodeerd beeld. Eén GPU-naar-CPU-kopie van 192 pixels
        // breed, twee keer per seconde — geen tweede opname, geen encoder, geen socket.
        // Vóór de pacer, zodat een lage `fps` de terugblik niet stilzet.
        if cfg.voorbeeld && laatste_voorbeeld.elapsed() >= crate::kijker::MINIATUUR_INTERVAL {
            laatste_voorbeeld = Instant::now();
            match crate::kijker::maak_miniatuur(d3d, &opname.textuur) {
                Ok(m) => {
                    if let Ok(mut slot) = gedeeld.miniatuur.lock() {
                        *slot = Some(m);
                    }
                }
                Err(e) => {
                    tracing::debug!(error = %format!("{e:#}"), "eigen miniatuur maken mislukt")
                }
            }
        }

        if !pacer.laat_door(Instant::now()) {
            continue;
        }

        let kijkers = gedeeld
            .kijkers
            .lock()
            .map(|k| k.clone())
            .unwrap_or_default();
        if kijkers.is_empty() {
            // Niemand aan de andere kant. Coderen voor de leegte heeft geen zin — bij een
            // camera met alleen een voorbeeldvenster is dat de normale toestand, en bij
            // een gedeeld scherm het korte moment tussen de laatste kijker en het
            // opruimen door de motor.
            continue;
        }

        // Er kijkt iemand, dus nu is de encoder nodig. Mislukt dat, dan stopt deze deler
        // met een leesbare reden in plaats van beeld te blijven opnemen dat nergens
        // heen kan.
        let encoder = match &mut encoder {
            Some(e) => e,
            geen => {
                let nieuw = Encoder::new(d3d, &encoder_cfg).context("encoder opzetten")?;
                tracing::info!(stream = cfg.stream_id, "eerste kijker; encoder erbij");
                geen.insert(nieuw)
            }
        };

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

        let voor_encode = Instant::now();
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

        let encode_us = voor_encode.elapsed().as_micros() as u64;

        for pakket in pakketten {
            meter.verstuurd += 1;
            meter.keyframes += u32::from(pakket.keyframe);
            meter.grootste = meter.grootste.max(pakket.data.len());
            let voor_stuur = Instant::now();

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
                let op_de_draad = fitcom_proto::MEDIA_HEADER_LEN + stuk.len();
                meter.bytes += op_de_draad as u64;

                // Spreiden in plaats van stoten: zie `Verzendtempo`. Een gewoon beeld
                // wacht hier nooit.
                let wacht = tempo.wachttijd(Instant::now(), op_de_draad * kijkers.len());
                if !wacht.is_zero() {
                    meter.geremd_us += wacht.as_micros() as u64;
                    std::thread::sleep(wacht);
                }

                for &kijker in &kijkers {
                    if let Err(e) = socket.stuur(kijker, &header, &stuk) {
                        meter.niet_verstuurd += 1;
                        tracing::debug!(%kijker, error = %e, "videofragment niet verstuurd");
                    }
                }
            }
            gedeeld.beelden.fetch_add(1, Ordering::Relaxed);
            crate::spoor::spoor!(
                spoor,
                "{:.3},{:.3},{:.3},{gedropt},{},{},{},{encode_us},{}",
                (opname.opgenomen_hns - nulpunten.map(|n| n.0).unwrap_or(0)) as f64 / 10_000.0,
                binnen.duration_since(spoor_begin).as_secs_f64() * 1000.0,
                voor_stuur.duration_since(spoor_begin).as_secs_f64() * 1000.0,
                pakket.tijd_hns,
                pakket.data.len(),
                u8::from(pakket.keyframe),
                voor_stuur.elapsed().as_micros() as u64
            );
        }
    }

    if let Some(s) = spoor.as_mut() {
        s.klaar();
    }
    Ok(())
}

/// Houdt de momentane verzendsnelheid onder een plafond, zonder een gewoon beeld op te
/// houden.
///
/// Een emmer met een gat: elke byte kost tijd, en zolang de achterstand binnen de speling
/// blijft wordt er niet gewacht. Een beeld van dertien fragmenten past helemaal in de
/// speling en gaat dus net zo hard de deur uit als voorheen; een keyframe van
/// driehonderd fragmenten wordt gespreid.
///
/// Er wordt pas gewacht zodra het de moeite waard is ([`MIN_WACHT`]) — anders zou er per
/// pakket vijftig microseconde gepauzeerd worden, en dat is op Windows niet te slapen
/// maar wel te verspillen aan wachtlussen.
struct Verzendtempo {
    per_byte: Duration,
    speling: Duration,
    /// Tot wanneer alles wat tot nu toe aangeboden is de draad op gaat.
    klaar: Instant,
}

/// Korter dan dit niet wachten: dan kost het pauzeren meer dan het oplevert.
const MIN_WACHT: Duration = Duration::from_millis(1);

impl Verzendtempo {
    fn nieuw(bitrate: u32, nu: Instant) -> Self {
        let bytes_per_s = u64::from(bitrate.max(1)) / 8 * u64::from(PIEK_FACTOR);
        let per_byte = Duration::from_nanos(1_000_000_000 / bytes_per_s.max(1));
        Self {
            per_byte,
            speling: per_byte * SPELING_BYTES as u32,
            klaar: nu,
        }
    }

    /// Hoelang er gewacht moet worden voordat deze bytes de deur uit mogen. Boekt ze
    /// meteen in, dus precies één keer aanroepen per pakket.
    fn wachttijd(&mut self, nu: Instant, bytes: usize) -> Duration {
        if self.klaar < nu {
            self.klaar = nu;
        }
        let achterstand = self.klaar.saturating_duration_since(nu + self.speling);
        self.klaar += self.per_byte * bytes as u32;
        if achterstand >= MIN_WACHT {
            achterstand
        } else {
            Duration::ZERO
        }
    }
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
    /// Hoeveel er in deze seconde gewacht is om de stoot van een keyframe te spreiden.
    geremd_us: u64,
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
            geremd_us: 0,
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
            geremd_ms = self.geremd_us / 1000,
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
    fn een_gewoon_beeld_wordt_nooit_opgehouden() {
        // De spreiding is er voor keyframes. Zou hij een gewoon beeld van een paar
        // fragmenten ook remmen, dan kost hij vertraging op elk beeld in plaats van op
        // één per tien seconden.
        let mut tempo = Verzendtempo::nieuw(8_000_000, Instant::now());
        let nu = Instant::now();
        for beeld in 0..600 {
            // Twintig fragmenten, ruim boven de mediaan van veertien kB per beeld.
            let moment = nu + Duration::from_nanos(beeld * 1_000_000_000 / 60);
            for _ in 0..20 {
                assert!(
                    tempo.wachttijd(moment, 1116).is_zero(),
                    "een gewoon beeld hoort er zonder wachten uit te gaan"
                );
            }
        }
    }

    #[test]
    fn een_keyframe_wordt_gespreid_over_ongeveer_een_beeldtijd() {
        // 371 kB ging er eerst in 1,7 ms uit: momentaan 1,75 Gbit/s op een budget van
        // 8 Mbit/s. Dat is de stoot die op een echt pad verlies veroorzaakt.
        let begin = Instant::now();
        let mut tempo = Verzendtempo::nieuw(8_000_000, begin);
        let mut nu = begin;
        let mut gewacht = Duration::ZERO;
        for _ in 0..(371 * 1024 / 1116) {
            let w = tempo.wachttijd(nu, 1116);
            gewacht += w;
            nu += w;
        }
        assert!(
            (Duration::from_millis(10)..=Duration::from_millis(30)).contains(&gewacht),
            "een keyframe werd over {gewacht:?} gespreid; verwacht rond een beeldtijd"
        );
    }

    #[test]
    fn na_een_stille_periode_mag_er_weer_vol_gestuurd_worden() {
        // De emmer loopt leeg als er niets verstuurd wordt. Zonder dat zou een deler die
        // even niets te doen had daarna alsnog geremd worden.
        let begin = Instant::now();
        let mut tempo = Verzendtempo::nieuw(8_000_000, begin);
        for _ in 0..400 {
            tempo.wachttijd(begin, 1116);
        }
        let later = begin + Duration::from_secs(1);
        assert!(
            tempo.wachttijd(later, 1116).is_zero(),
            "na een seconde stilte hoort de speling weer vol te zijn"
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
