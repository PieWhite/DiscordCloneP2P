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
    let (mut gemeten_incompleet, mut gemeten_verworpen, mut gemeten_hersteld) = (0u64, 0u64, 0u64);

    let mut klok = Weergaveklok::nieuw();
    let mut uitvouwer = Uitvouwer::default();
    let mut wachtrij: std::collections::VecDeque<(Instant, Instant, crate::fragment::Frame)> =
        std::collections::VecDeque::new();
    let mut korte_timeout = false;

    let begin = Instant::now();
    let mut spoor = crate::spoor::Spoor::nieuw(
        &format!("kijker-{}", cfg.stream_id),
        "getoond_ms,ts90k,aankomst_ms,gepland_ms,keyframe,bytes,decode_us,toon_us,mini_us,wachtrij,incompleet",
    );
    let mut verlies = crate::spoor::Spoor::nieuw(
        &format!("verlies-{}", cfg.stream_id),
        "ms,verwacht_seq,gekregen_seq,gemist,ts90k,keyframe,frag_index,stil_us",
    );
    let mut vorige_seq: Option<u32> = None;
    let mut vorig_pakket = Instant::now();

    while !gedeeld.stop.load(Ordering::Relaxed) {
        meter.tik();

        // Wacht er beeld op zijn beurt, dan moet de lus fijner tikken dan de 8 ms die
        // voor een leeg venster prima is — anders komt elk beeld tot 8 ms te laat en is
        // de hele planning voor niets geweest.
        let wil_kort = !wachtrij.is_empty();
        if wil_kort != korte_timeout {
            korte_timeout = wil_kort;
            let _ = socket.zet_timeout(if wil_kort {
                Duration::from_millis(1)
            } else {
                ONTVANG_TIMEOUT
            });
        }

        // Alles waarvan de tijd gekomen is decoderen; alleen het laatste tonen. Meer dan
        // één tegelijk betekent dat we achterlopen, en dan is de nieuwste de juiste.
        while wachtrij
            .front()
            .is_some_and(|(op, _, _)| *op <= Instant::now())
        {
            let (gepland, aankomst, frame) = wachtrij.pop_front().expect("net gezien");
            let laatste_die_toe_was = !wachtrij
                .front()
                .is_some_and(|(op, _, _)| *op <= Instant::now());
            let sporen = Sporen {
                begin,
                gepland,
                aankomst,
                wachtrij: wachtrij.len(),
                incompleet: samensteller.incompleet,
            };
            if !toon_beeld(
                &mut decoder,
                &mut venster,
                d3d,
                gedeeld,
                events,
                &mut meter,
                &mut laatste_miniatuur,
                &frame,
                laatste_die_toe_was,
                &mut spoor,
                &sporen,
            ) {
                decoder.spoel();
                wacht_op_keyframe = true;
                laatst_gevraagd = Instant::now() - KEYFRAME_PAUZE;
            }
        }

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

        // Gaten in `seq` zijn het enige harde bewijs van verlies: de deler telt hem per
        // fragment door, dus een sprong betekent dat er iets tussenuit is.
        if let Some(v) = vorige_seq {
            let verwacht = v.wrapping_add(1);
            if header.seq != verwacht {
                crate::spoor::spoor!(
                    verlies,
                    "{:.3},{verwacht},{},{},{},{},{},{}",
                    Instant::now().duration_since(begin).as_secs_f64() * 1000.0,
                    header.seq,
                    header.seq.wrapping_sub(verwacht),
                    header.timestamp,
                    u8::from(header.is_keyframe()),
                    header.frag_index,
                    vorig_pakket.elapsed().as_micros() as u64
                );
            }
        }
        vorige_seq = Some(header.seq);
        vorig_pakket = Instant::now();

        let klaar = samensteller.push(&header, payload);

        meter.incompleet += samensteller.incompleet - gemeten_incompleet;
        meter.verworpen += samensteller.verworpen - gemeten_verworpen;
        meter.hersteld += samensteller.hersteld - gemeten_hersteld;
        gemeten_incompleet = samensteller.incompleet;
        gemeten_verworpen = samensteller.verworpen;
        gemeten_hersteld = samensteller.hersteld;

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

        if frame.keyframe {
            tracing::debug!(stream = cfg.stream_id, "keyframe ontvangen");
        }

        if wacht_op_keyframe {
            if !frame.keyframe {
                continue;
            }
            wacht_op_keyframe = false;
        }

        // Niet meteen tonen: inplannen op de tijd waarop dit beeld is opgenomen. Dat is
        // het verschil tussen "zo snel mogelijk" en "op tijd", en alleen het tweede ziet
        // eruit als vloeiend.
        let aankomst = Instant::now();
        let toon_op = klok.plan(uitvouwer.uitvouwen(frame.timestamp), aankomst);
        wachtrij.push_back((toon_op, aankomst, frame));

        if wachtrij.len() > MAX_WACHTEND {
            // De klok is ergens weggesprongen — deler herstart, stream opnieuw begonnen.
            // Opnieuw ijken en wat er ligt tonen; achterlopen heeft geen zin.
            tracing::info!(stream = cfg.stream_id, "weergaveklok opnieuw geijkt");
            klok.opnieuw_ijken();
            let nu = Instant::now();
            for (op, _, _) in wachtrij.iter_mut() {
                *op = nu;
            }
        }
    }

    if let Some(s) = spoor.as_mut() {
        s.klaar();
    }
    if let Some(s) = verlies.as_mut() {
        s.klaar();
    }
}

/// Alleen voor het diagnostische spoor: wat er over dit beeld te vertellen valt buiten
/// het beeld zelf.
struct Sporen {
    begin: Instant,
    gepland: Instant,
    aankomst: Instant,
    wachtrij: usize,
    incompleet: u64,
}

/// Decodeert één beeld en toont het, tenzij er nog een verser exemplaar achteraan staat.
/// Levert `false` op als de decoder struikelde; dan is een nieuw keyframe het herstel.
#[allow(clippy::too_many_arguments)]
fn toon_beeld(
    decoder: &mut Decoder,
    venster: &mut Venster,
    d3d: &D3dContext,
    gedeeld: &Arc<Gedeeld>,
    events: &Sender<KijkerEvent>,
    meter: &mut Meter,
    laatste_miniatuur: &mut Instant,
    frame: &crate::fragment::Frame,
    tonen: bool,
    spoor: &mut Option<crate::spoor::Spoor>,
    sporen: &Sporen,
) -> bool {
    let tijd_hns = naar_hns(frame.timestamp);
    let voor_decode = Instant::now();
    let uit_decoder = decoder.decode(&frame.data, tijd_hns);
    let decode_us = voor_decode.elapsed().as_micros() as u64;
    meter.decode_us += decode_us;
    let mut mini_us = 0u64;

    match uit_decoder {
        Ok(Some(beeld)) => {
            // Elk beeld gaat wél door de decoder — die heeft ze nodig als referentie voor
            // de volgende — maar alleen het laatste dat aan de beurt was gaat het scherm
            // op. De rest zou je toch niet zien.
            if !tonen {
                return true;
            }
            let voor_toon = Instant::now();
            if let Err(e) = venster.toon(&beeld) {
                tracing::error!(error = %format!("{e:#}"), "beeld tonen mislukt");
                return true;
            }
            let toon_us = voor_toon.elapsed().as_micros() as u64;
            meter.toon_us += toon_us;
            meter.toonde(voor_toon);
            gedeeld.beelden.fetch_add(1, Ordering::Relaxed);
            meet_vertraging(gedeeld, tijd_hns);

            if laatste_miniatuur.elapsed() >= MINIATUUR_INTERVAL {
                *laatste_miniatuur = Instant::now();
                let voor_mini = Instant::now();
                match maak_miniatuur(d3d, &beeld) {
                    Ok(m) => {
                        let _ = events.try_send(KijkerEvent::Miniatuur(m));
                    }
                    Err(e) => {
                        tracing::debug!(error = %format!("{e:#}"), "miniatuur maken mislukt");
                    }
                }
                mini_us = voor_mini.elapsed().as_micros() as u64;
            }
            crate::spoor::spoor!(
                spoor,
                "{:.3},{},{:.3},{:.3},{},{},{decode_us},{toon_us},{mini_us},{},{}",
                voor_toon.duration_since(sporen.begin).as_secs_f64() * 1000.0,
                frame.timestamp,
                sporen.aankomst.duration_since(sporen.begin).as_secs_f64() * 1000.0,
                sporen.gepland.duration_since(sporen.begin).as_secs_f64() * 1000.0,
                u8::from(frame.keyframe),
                frame.data.len(),
                sporen.wachtrij,
                sporen.incompleet
            );
            true
        }
        Ok(None) => true,
        Err(e) => {
            // Een beschadigd beeld laat de decoder struikelen. Opnieuw beginnen bij het
            // volgende keyframe is hier het herstel, niet stoppen.
            tracing::warn!(error = %format!("{e:#}"), "beeld decoderen mislukt");
            false
        }
    }
}

/// Hoeveel voorsprong we nemen op de weergave.
///
/// Dit is de prijs voor gelijkmatig beeld: een beeld dat eerder klaar is dan zijn beurt
/// wacht even. Te klein en elk beeld dat een paar milliseconden later binnenkomt is al te
/// laat, en dan zijn we terug bij tonen-zodra-het-kan. Te groot en je kijkt naar het
/// verleden. Op een tailnet met 9 ms heen en weer is dit ruim; bij zichtbare vertraging is
/// dit de knop.
const WEERGAVE_VOORSPRONG: Duration = Duration::from_millis(30);

/// Diagnostisch: boven dit gat tussen twee getoonde beelden loggen we meteen, in plaats
/// van te wachten op de per-seconde `spreiding_ms`. Voor het uitzoeken van de periodieke
/// microhapering — die aggregatie verbergt precies wanneer en hoe vaak het gebeurt.
const GROTE_SPRONG_MS: f64 = 40.0;

/// Zoveel beelden mogen er hoogstens wachten. Meer betekent dat de klok ergens is
/// weggesprongen — een herstart van de deler, een stream die opnieuw begint — en dan is
/// opnieuw ijken beter dan een halve seconde achterlopen.
const MAX_WACHTEND: usize = 16;

/// Wanneer een binnengekomen beeld getoond moet worden.
///
/// # Waarom dit er moet zijn
///
/// De tijdstempel op de draad zegt wanneer het beeld is *opgenomen*. Zonder deze klok
/// toont de kijker elk beeld zodra het compleet is, en dan bepaalt de reis — netwerk,
/// planning van threads, hoe vol de ontvangbuffer net zat — wanneer het op het scherm
/// komt. Beelden die gelijkmatig zijn opgenomen komen dan ongelijkmatig in beeld, en dat
/// is precies wat je ziet als haperen, ook als er geen enkel beeld verloren gaat en de
/// teller keurig zestig per seconde meldt.
///
/// Audio heeft dit al (`fitcom-audio::jitter`), video niet. Dat verschil is de reden dat
/// het geluid vloeiend was terwijl het beeld schokte.
///
/// # Hoe de twee klokken aan elkaar geknoopt worden
///
/// De deler en de kijker delen geen klok en gaan die ook niet synchroniseren — dat zou
/// een tijdserver vragen die we niet hebben en niet willen. Wat wel kan: het beeld dat er
/// het *snelst* over deed, deed er de minste reistijd over, en dat beeld bepaalt dus de
/// beste schatting van hoe de twee klokken zich verhouden. Vandaar het minimum van
/// `aankomst − verstreken` over een venster, en niet het gemiddelde: dat zou meelopen met
/// elke vertraging.
///
/// Het venster loopt door in twee emmers, zodat een eenmalige uitschieter er vanzelf weer
/// uit valt en een klok die langzaam wegloopt gevolgd wordt.
struct Weergaveklok {
    eerste_ts: Option<u64>,
    huidig_min: Option<Instant>,
    vorig_min: Option<Instant>,
    emmer_begin: Instant,
}

/// Na zoveel tijd schuift de lopende emmer door. Twee emmers samen zijn dus het venster
/// waarover het minimum genomen wordt.
const EMMER: Duration = Duration::from_secs(2);

impl Weergaveklok {
    fn nieuw() -> Self {
        Self {
            eerste_ts: None,
            huidig_min: None,
            vorig_min: None,
            emmer_begin: Instant::now(),
        }
    }

    fn opnieuw_ijken(&mut self) {
        *self = Self::nieuw();
    }

    /// `ts` is de uitgevouwen tijdstempel van de draad in 90 kHz-tikken.
    fn plan(&mut self, ts: u64, aangekomen: Instant) -> Instant {
        let eerste = *self.eerste_ts.get_or_insert(ts);
        let verstreken = Duration::from_nanos(ts.saturating_sub(eerste) * 1_000_000_000 / 90_000);

        // Waar het nulpunt van de deler ligt op onze klok, als dit beeld er niets over
        // gedaan had.
        let kandidaat = aangekomen - verstreken;

        if self.emmer_begin.elapsed() >= EMMER {
            self.emmer_begin = Instant::now();
            self.vorig_min = self.huidig_min.take();
        }
        self.huidig_min = Some(match self.huidig_min {
            Some(m) => m.min(kandidaat),
            None => kandidaat,
        });

        let basis = match (self.huidig_min, self.vorig_min) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) | (None, Some(a)) => a,
            (None, None) => kandidaat,
        };

        basis + verstreken + WEERGAVE_VOORSPRONG
    }
}

/// Vouwt de 32-bits tijdstempel van de draad uit naar een doorlopende teller.
///
/// Op 90 kHz loopt hij elke dertien uur om. Dat is zeldzaam en juist daarom gevaarlijk:
/// zonder dit springt de planning er een halve dag naast en staat het beeld stil.
#[derive(Default)]
struct Uitvouwer {
    hoog: u64,
    vorige: Option<u32>,
}

impl Uitvouwer {
    fn uitvouwen(&mut self, ts: u32) -> u64 {
        if let Some(v) = self.vorige {
            // Een sprong terug over meer dan de helft van het bereik is een omloop, geen
            // beeld dat te laat is.
            if v > ts && v - ts > u32::MAX / 2 {
                self.hoog += 1;
            }
        }
        self.vorige = Some(ts);
        (self.hoog << 32) | u64::from(ts)
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
    /// Verloren fragmenten die uit de pariteit teruggerekend zijn. Elk daarvan zou
    /// zonder pariteit een bevroren beeld van rond de honderd milliseconde geweest zijn,
    /// dus dit is het getal dat zegt of dat werkt. Zie `crate::fragment`.
    hersteld: u64,
    keyframe_verzoeken: u32,
    decode_us: u64,
    toon_us: u64,
    /// Afstanden tussen twee getoonde beelden, als som en som-van-kwadraten in
    /// milliseconden. Daar komt de standaardafwijking uit, en **dat is het getal dat
    /// zegt of het hapert**: het aantal beelden per seconde kan kloppen terwijl ze
    /// ongelijk uit elkaar staan, en dan ziet het er alsnog uit als schokken.
    vorige_toon: Option<Instant>,
    afstand_som: f64,
    afstand_som_kwadraat: f64,
    afstanden: u32,
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
            hersteld: 0,
            keyframe_verzoeken: 0,
            decode_us: 0,
            toon_us: 0,
            vorige_toon: None,
            afstand_som: 0.0,
            afstand_som_kwadraat: 0.0,
            afstanden: 0,
        }
    }

    /// Eén beeld op het scherm gezet.
    fn toonde(&mut self, nu: Instant) {
        if let Some(vorige) = self.vorige_toon {
            let ms = nu.duration_since(vorige).as_secs_f64() * 1000.0;
            self.afstand_som += ms;
            self.afstand_som_kwadraat += ms * ms;
            self.afstanden += 1;
            if ms > GROTE_SPRONG_MS {
                tracing::warn!(
                    stream = self.stream_id,
                    gap_ms = (ms * 10.0).round() / 10.0,
                    "grote sprong tussen twee getoonde beelden"
                );
            }
        }
        self.vorige_toon = Some(nu);
        self.getoond += 1;
    }

    fn spreiding_ms(&self) -> f64 {
        if self.afstanden < 2 {
            return 0.0;
        }
        let n = f64::from(self.afstanden);
        let gem = self.afstand_som / n;
        (self.afstand_som_kwadraat / n - gem * gem).max(0.0).sqrt()
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
            spreiding_ms = (self.spreiding_ms() * 100.0).round() / 100.0,
            mbit = ((self.bytes as f64 * 8.0 / s / 1e5).round() / 10.0),
            frag_per_s = (self.fragmenten as f64 / s).round() as u32,
            incompleet = self.incompleet,
            verworpen = self.verworpen,
            hersteld = self.hersteld,
            keyframe_verzoeken = self.keyframe_verzoeken,
            decode_ms = per_beeld(self.decode_us),
            toon_ms = per_beeld(self.toon_us),
            "kijker"
        );
        let vorige_toon = self.vorige_toon;
        *self = Meter::nieuw(self.stream_id);
        // De afstand over de secondegrens heen telt gewoon door; anders mis je juist de
        // hapering die precies daar valt.
        self.vorige_toon = vorige_toon;
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
fn maak_miniatuur(d3d: &D3dContext, beeld: &crate::d3d::Beeld) -> Result<Miniatuur> {
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

    /// Een reis die elke keer anders lang duurt: netwerk, planning van threads, een
    /// ontvangbuffer die net vol zat. Vaste reeks, zodat de test elke keer hetzelfde doet.
    const REISTIJD_MS: [u64; 10] = [4, 11, 5, 19, 4, 7, 26, 4, 9, 13];

    #[test]
    fn gelijkmatig_opgenomen_beeld_wordt_gelijkmatig_getoond() {
        // Dit is waar het allemaal om draait. De deler neemt keurig op 60 beelden per
        // seconde op; onderweg wordt die regelmaat verpest. Toont de kijker zodra een
        // beeld compleet is, dan staan ze op het scherm zoals ze aankwamen — met
        // uitschieters van 20 ms, en dát is wat je ziet als haperen. Plant hij op de
        // tijdstempel, dan komt de regelmaat van de opname terug.
        let mut klok = Weergaveklok::nieuw();
        let begin = Instant::now();
        let stap_ticks = 90_000 / 60;

        let gepland: Vec<Instant> = (0..300u64)
            .map(|n| {
                let ts = n * stap_ticks;
                let ideaal = begin + Duration::from_nanos(n * 1_000_000_000 / 60);
                let aangekomen =
                    ideaal + Duration::from_millis(REISTIJD_MS[n as usize % REISTIJD_MS.len()]);
                klok.plan(ts, aangekomen)
            })
            .collect();

        let afstanden: Vec<f64> = gepland
            .windows(2)
            .map(|w| w[1].duration_since(w[0]).as_secs_f64() * 1000.0)
            .collect();
        let gem = afstanden.iter().sum::<f64>() / afstanden.len() as f64;
        let spreiding = (afstanden.iter().map(|a| (a - gem).powi(2)).sum::<f64>()
            / afstanden.len() as f64)
            .sqrt();

        assert!(
            spreiding < 0.01,
            "{spreiding:.3} ms ongelijk terwijl de opname gelijkmatig was; de reistijd \
             lekt door naar het scherm"
        );
        assert!(
            (16.6..=16.7).contains(&gem),
            "gemiddeld {gem:.2} ms tussen twee beelden; verwacht 16,67"
        );
    }

    #[test]
    fn een_beeld_dat_te_laat_is_wordt_niet_vooruit_gepland() {
        // Eén beeld doet er veel langer over dan de rest. Dat mag de klok niet meeslepen:
        // het minimum over het venster blijft staan, dus de rest houdt zijn plek.
        let mut klok = Weergaveklok::nieuw();
        let begin = Instant::now();
        let stap = Duration::from_nanos(1_000_000_000 / 60);

        let eerste = klok.plan(0, begin);
        let tweede = klok.plan(1500, begin + stap + Duration::from_millis(200));
        let derde = klok.plan(3000, begin + stap * 2);

        assert_eq!(
            derde.duration_since(eerste).as_millis(),
            33,
            "het derde beeld hoort twee stappen na het eerste te staan"
        );
        assert!(
            tweede < derde,
            "de volgorde moet kloppen, ook als er eentje laat is"
        );
    }

    #[test]
    fn de_tijdstempel_mag_omlopen() {
        // Op 90 kHz gebeurt dat elke dertien uur. Zonder uitvouwen springt de planning
        // er een halve dag naast en staat het beeld stil.
        let mut u = Uitvouwer::default();
        assert_eq!(u.uitvouwen(u32::MAX - 1), u64::from(u32::MAX - 1));
        assert_eq!(u.uitvouwen(u32::MAX), u64::from(u32::MAX));
        assert_eq!(u.uitvouwen(3), (1u64 << 32) + 3, "dit is een omloop");
        assert_eq!(u.uitvouwen(9), (1u64 << 32) + 9);
    }

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
