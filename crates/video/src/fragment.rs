//! Videoframes opknippen voor UDP en aan de andere kant weer samenstellen.
//!
//! Een keyframe op 1080p is al gauw honderd kilobyte, terwijl er per UDP-pakket maar
//! ruim duizend bytes in passen zonder dat IP zelf gaat fragmenteren. Eén verloren
//! stukje betekent dat het hele frame onbruikbaar is.
//!
//! # Hoe een fragment bij zijn frame hoort
//!
//! Alle fragmenten van één frame dragen dezelfde `timestamp`; `frag_index` geeft de
//! plek binnen het frame en het laatste fragment draagt [`MediaHeader::FLAG_LAST_FRAGMENT`].
//! Daardoor is er geen apart frame-nummer nodig in de header, en weet de ontvanger
//! hoeveel stukken hij moet hebben zodra het laatste fragment binnen is — ook als dat
//! als eerste aankomt.
//!
//! # Waarom er een pariteitsfragment achteraan gaat
//!
//! Gemeten op 2026-08-02: **één verloren fragment kostte 70 tot 156 ms bevroren beeld.**
//! Niet omdat dat ene beeld weg is — dat is 17 ms — maar omdat de kijker daarna zijn
//! decoder spoelt en alles weggooit tot er een nieuw keyframe is. Dat keyframe moet hij
//! aanvragen bij de deler, en het antwoord is een beeld van honderden kilobytes.
//!
//! Verreweg de meeste verliesmomenten zijn er precies één fragment groot. Daarvoor is een
//! terugweg over het netwerk absurd duur: met de XOR van alle stukken erbij is dat ene
//! fragment ter plekke terug te rekenen, zonder verzoek, zonder wachten, zonder keyframe.
//! Kost één pakket per beeld — bij een beeld van dertien fragmenten dus 8%, bij een
//! keyframe van driehonderd 0,3%.
//!
//! Twee of meer gaten in hetzelfde beeld blijven het oude pad volgen. Dat is de goede
//! afweging: daar is redundantie duur en zeldzaam verlies goedkoop.

use fitcom_proto::{MediaHeader, PayloadType, MAX_MEDIA_PAYLOAD, PARITEIT_PAYLOAD_LEN};
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// Zoveel frames houden we tegelijk in de lucht. Meer betekent dat we op iets wachten
/// dat toch niet meer komt, en dan is doorgaan beter dan blijven verzamelen.
const MAX_ONDERWEG: usize = 8;

/// Zoveel fragmenten mag één beeld hoogstens hebben. 1024 × 1100 B ≈ 1,1 MB.
///
/// Zonder deze grens is `frag_index` een vrije `u16` en kost één halffabrikaat tot
/// 65 536 × ~1520 = 99,6 MB werkgeheugen, gevuld door een afzender die alleen maar
/// pakketten hoeft te sturen (B-12a). Met deze grens is het hoogstens 1,1 MB per beeld
/// en, met [`MAX_ONDERWEG`], ~10 MB in totaal.
///
/// **Niet de 512 uit `docs/BEVEILIGING.md`.** 512 × 1100 = 563 kB en dat is te krap: het
/// gemeten keyframe van 371 kB was bij een budget van 8 Mbit/s, terwijl de standaard in
/// `config.toml` op 12 Mbit/s staat en vrij hoger te zetten is. Een grens die een echt
/// keyframe weigert breekt het beeld helemaal in plaats van het te beschermen. De
/// werkelijke bovengrens is [`MAX_FRAME_BYTES`]; deze grens houdt de *buffer* klein.
const MAX_FRAGMENTEN_PER_BEELD: u16 = 1024;

/// Bovengrens op een compleet beeld dat de OS-decoder in gaat. Gemeten keyframes zijn
/// 100 tot 371 kB; hierboven is het geen beeld meer maar een grote ongevalideerde invoer
/// voor een closed-source H.264-parser (B-29).
///
/// Dit is de grens die telt; [`MAX_FRAGMENTEN_PER_BEELD`] begrenst de buffer en laat er
/// net iets meer door. Wie die verhoogt of `frag_index` verbreedt loopt hier tegenaan en
/// niet in de decoder.
const MAX_FRAME_BYTES: usize = 1024 * 1024;

/// Zo lang mag een beeld onvolledig blijven liggen.
///
/// Bij 60 beelden per seconde is een beeld dat na een halve seconde nog niet compleet is
/// niet "onderweg" maar weg. Zonder deze grens blijft een halffabrikaat waarvan zowel het
/// afsluitende fragment als de pariteit nooit komt eeuwig staan (B-12b).
const HALFFABRIKAAT_TTL: Duration = Duration::from_millis(500);

/// Zoveel tikken achteruit heet nog "te laat" in plaats van "de klok is verzet".
///
/// 90 kHz × 2 s: ruim boven de weergavevoorsprong en de diepste wachtrij van de kijker,
/// en ver onder de halve omloop waarmee [`is_nieuwer`] een echte omloop van een sprong
/// onderscheidt.
const ACHTERSTAND_VENSTER: u32 = 2 * 90_000;

/// Of `ts` ná `laatste` komt, met de omloop van de 32-bits klok erin verwerkt.
///
/// Een gewone `<`-vergelijking heeft twee gaten. Eén pakket met `timestamp = u32::MAX`
/// zette `laatste` op het maximum, waarna élk legitiem pakket afketste en het beeld
/// permanent bevroor — er was nergens een pad dat `laatste` terugzette (B-11). En de
/// 90 kHz-klok loopt elke 13 uur 15 minuten zelf om; die sprong terug werd hier
/// weggegooid, waardoor `Uitvouwer` in de kijker hem nooit zag en het beeld op datzelfde
/// moment stilstond (B-30).
///
/// Modulair rekenen dekt beide: een verschil binnen een halve omloop vooruit is nieuw,
/// de rest is oud of onzin.
fn is_nieuwer(ts: u32, laatste: u32) -> bool {
    let d = ts.wrapping_sub(laatste);
    d != 0 && d < u32::MAX / 2
}

/// Knipt `data` op in stukken die elk in één UDP-pakket passen.
///
/// Levert `(frag_index, is_laatste, stuk)`. Een leeg frame levert één leeg fragment op,
/// zodat de ontvanger ook dan een compleet frame ziet in plaats van te blijven wachten.
pub fn fragmenteer(data: &[u8]) -> impl Iterator<Item = (u16, bool, &[u8])> {
    let stukken = data.len().div_ceil(MAX_MEDIA_PAYLOAD).max(1);
    (0..stukken).map(move |i| {
        let start = i * MAX_MEDIA_PAYLOAD;
        let eind = ((i + 1) * MAX_MEDIA_PAYLOAD).min(data.len());
        (
            i as u16,
            i + 1 == stukken,
            data.get(start..eind).unwrap_or(&[]),
        )
    })
}

/// De XOR van alle fragmenten van `data`, elk voorafgegaan door zijn eigen lengte en
/// aangevuld tot [`MAX_MEDIA_PAYLOAD`].
///
/// De lengte gaat mee door de XOR heen omdat het laatste fragment korter is dan de rest:
/// zo levert het terugrekenen niet alleen de bytes op maar ook hoeveel het er waren, en
/// hoeft de ontvanger geen apart geval te kennen voor "het laatste stuk is zoek".
pub fn pariteit_van(data: &[u8]) -> Vec<u8> {
    let mut uit = vec![0u8; PARITEIT_PAYLOAD_LEN];
    for (_, _, stuk) in fragmenteer(data) {
        let lengte = (stuk.len() as u16).to_le_bytes();
        uit[0] ^= lengte[0];
        uit[1] ^= lengte[1];
        for (doel, bron) in uit[2..].iter_mut().zip(stuk) {
            *doel ^= bron;
        }
    }
    uit
}

/// Bouwt de headers voor een compleet frame, met het pariteitsfragment als laatste.
/// Handig omdat de vlaggen makkelijk fout gaan.
///
/// Het pariteitsfragment draagt als `frag_index` het *aantal* stukken in plaats van een
/// plek erin. Daarmee weet de ontvanger hoeveel hij er hoort te hebben, ook als juist het
/// stuk met [`MediaHeader::FLAG_LAST_FRAGMENT`] zoek is.
pub fn headers_voor(
    stream_id: u32,
    timestamp: u32,
    payload_type: PayloadType,
    keyframe: bool,
    eerste_seq: u32,
    data: &[u8],
) -> impl Iterator<Item = (MediaHeader, Cow<'_, [u8]>)> {
    let basis = if keyframe {
        MediaHeader::FLAG_KEYFRAME
    } else {
        0
    };
    let stukken = aantal_stukken(data);

    let gegevens = fragmenteer(data).map(move |(idx, laatste, stuk)| {
        let mut flags = basis;
        if laatste {
            flags |= MediaHeader::FLAG_LAST_FRAGMENT;
        }
        (
            MediaHeader {
                stream_id,
                seq: eerste_seq.wrapping_add(u32::from(idx)),
                timestamp,
                payload_type,
                flags,
                frag_index: idx,
            },
            Cow::Borrowed(stuk),
        )
    });

    let pariteit = std::iter::once_with(move || {
        (
            MediaHeader {
                stream_id,
                seq: eerste_seq.wrapping_add(u32::from(stukken)),
                timestamp,
                payload_type,
                flags: basis | MediaHeader::FLAG_PARITEIT,
                frag_index: stukken,
            },
            Cow::Owned(pariteit_van(data)),
        )
    });

    gegevens.chain(pariteit)
}

fn aantal_stukken(data: &[u8]) -> u16 {
    data.len().div_ceil(MAX_MEDIA_PAYLOAD).max(1) as u16
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub timestamp: u32,
    pub keyframe: bool,
    pub data: Vec<u8>,
}

struct Halffabrikaat {
    stukken: BTreeMap<u16, Vec<u8>>,
    /// Bekend zodra het laatste fragment binnen is, ook als dat als eerste aankwam, en
    /// anders zodra het pariteitsfragment binnen is.
    aantal: Option<u16>,
    /// Of `aantal` uit het afsluitende fragment komt in plaats van uit de pariteit. Dan
    /// is het gezaghebbend: het staat in dezelfde stroom als de stukken zelf, terwijl een
    /// los pariteitspakket met een verkeerd aantal genoeg is om er een half beeld van te
    /// maken (B-28).
    aantal_is_hard: bool,
    /// De XOR van alle stukken, zie [`pariteit_van`].
    pariteit: Option<Vec<u8>>,
    keyframe: bool,
    /// Wanneer we hier het laatst een fragment voor kregen. Bepaalt wie er als eerste af
    /// valt; zie [`HALFFABRIKAAT_TTL`] en `Reassembler::ruim_op`.
    gezien: Instant,
}

impl Halffabrikaat {
    fn nieuw(nu: Instant) -> Self {
        Self {
            stukken: BTreeMap::new(),
            aantal: None,
            aantal_is_hard: false,
            pariteit: None,
            keyframe: false,
            gezien: nu,
        }
    }

    /// Of een pariteitspakket dat `n` stukken beweert bij dit beeld kan horen.
    ///
    /// Een pariteitspakket gaat als laatste de deur uit, dus zijn aantal hoort boven elke
    /// index te liggen die we al hebben, en gelijk te zijn aan wat het afsluitende
    /// fragment zei. Klopt dat niet, dan komt het er niet vandaan — en één zo'n pakket was
    /// genoeg om een afgekapt beeld als authentiek de decoder in te sturen (B-28).
    fn pariteit_klopt(&self, n: u16) -> bool {
        match self.aantal {
            Some(bekend) if self.aantal_is_hard => bekend == n,
            _ => self
                .stukken
                .last_key_value()
                .is_none_or(|(&hoogste, _)| hoogste < n),
        }
    }

    /// Legt het aantal stukken vast en gooit alles weg wat daar niet in past.
    ///
    /// Fragmenten die binnenkwamen vóórdat het aantal bekend was zijn niet op hun index
    /// gecontroleerd. Ze hier opruimen houdt de invariant "elke sleutel is kleiner dan
    /// `aantal`" overal geldig, en dat is waar [`Halffabrikaat::compleet`] en
    /// [`Halffabrikaat::herstel`] op leunen (B-28).
    fn zet_aantal(&mut self, n: u16, hard: bool) {
        self.aantal = Some(n);
        self.aantal_is_hard = hard;
        self.stukken.retain(|&i, _| i < n);
    }

    fn compleet(&self) -> bool {
        self.aantal.is_some_and(|n| {
            // Tellen alleen is niet genoeg: de indices 0..n moeten er ook echt zijn.
            // Anders klopt het aantal wel en zit er toch een gat in, gevuld met bytes
            // van wie het IP van de deler kan spoofen (B-28). Alle sleutels zijn
            // verschillend, dus `n` stukken met de hoogste onder `n` zijn precies 0..n.
            self.stukken.len() == usize::from(n)
                && self
                    .stukken
                    .last_key_value()
                    .is_some_and(|(&hoogste, _)| hoogste < n)
        })
    }

    /// Hoeveel bytes het samengestelde beeld zou worden.
    fn bytes(&self) -> usize {
        self.stukken.values().map(Vec::len).sum()
    }

    /// Rekent het ene ontbrekende stuk terug uit de pariteit. `true` als dat lukte; het
    /// frame is dan compleet.
    ///
    /// Doet niets bij twee of meer gaten — dan is er niets te herstellen — en niets als
    /// de pariteit zelf is wat er mist.
    fn herstel(&mut self) -> bool {
        let Some(n) = self.aantal.map(usize::from) else {
            return false;
        };
        let Some(pariteit) = &self.pariteit else {
            return false;
        };
        if self.stukken.len() + 1 != n || pariteit.len() != PARITEIT_PAYLOAD_LEN {
            return false;
        }
        // Zit er een stuk tussen dat buiten het beeld valt, dan is `len() + 1 == n`
        // toevallig en zou het terugrekenen onzin opleveren (B-28).
        if self
            .stukken
            .last_key_value()
            .is_some_and(|(&hoogste, _)| usize::from(hoogste) >= n)
        {
            return false;
        }
        let Some(ontbreekt) = (0..n as u16).find(|i| !self.stukken.contains_key(i)) else {
            return false;
        };

        let mut terug = pariteit.clone();
        for stuk in self.stukken.values() {
            let lengte = (stuk.len() as u16).to_le_bytes();
            terug[0] ^= lengte[0];
            terug[1] ^= lengte[1];
            for (doel, bron) in terug[2..].iter_mut().zip(stuk) {
                *doel ^= bron;
            }
        }

        let lengte = usize::from(u16::from_le_bytes([terug[0], terug[1]]));
        if lengte > MAX_MEDIA_PAYLOAD {
            // De pariteit zelf was beschadigd of hoorde bij een ander beeld. Terugrekenen
            // levert dan onzin op, en onzin de decoder in duwen is erger dan één beeld
            // missen.
            return false;
        }
        terug.truncate(2 + lengte);
        terug.drain(..2);
        self.stukken.insert(ontbreekt, terug);
        true
    }
}

pub struct Reassembler {
    onderweg: BTreeMap<u32, Halffabrikaat>,
    /// Tijdstempel van het laatst afgeleverde frame; alles daarvoor is te laat.
    laatste: Option<u32>,
    pub verworpen: u64,
    /// Hoe vaak een frame sneuvelde doordat er stukken misten. Boven nul is een
    /// aanleiding om een nieuw keyframe te vragen.
    pub incompleet: u64,
    /// Hoe vaak één verloren fragment uit de pariteit is teruggerekend. Elk van deze
    /// zou zonder pariteit een bevroren beeld van rond de honderd milliseconde geweest
    /// zijn, dus dit is het getal dat zegt of het werkt.
    pub hersteld: u64,
    /// Hoe vaak de tijdstempel zo ver wegsprong dat doorgaan op `laatste` geen zin meer
    /// had. Een deler die herstart doet dit één keer; loopt hij op, dan zit er iemand
    /// pakketten met rare tijdstempels in te schuiven (B-11). Staat in de meterregel van
    /// de kijker, want zonder teller is dit onzichtbaar.
    pub hersynchronisaties: u64,
}

impl Default for Reassembler {
    fn default() -> Self {
        Self::new()
    }
}

impl Reassembler {
    pub fn new() -> Self {
        Self {
            onderweg: BTreeMap::new(),
            laatste: None,
            verworpen: 0,
            incompleet: 0,
            hersteld: 0,
            hersynchronisaties: 0,
        }
    }

    pub fn onderweg(&self) -> usize {
        self.onderweg.len()
    }

    /// Levert een frame op zodra alle stukken binnen zijn.
    pub fn push(&mut self, header: &MediaHeader, payload: &[u8]) -> Option<Frame> {
        self.push_op(Instant::now(), header, payload)
    }

    /// Zoals [`Reassembler::push`], met "nu" als parameter zodat de vervaltijd en de
    /// verdringing te testen zijn zonder te slapen. Eén kloklezing per pakket, net als
    /// voorheen; het hete pad wordt hier niets duurder van.
    fn push_op(&mut self, nu: Instant, header: &MediaHeader, payload: &[u8]) -> Option<Frame> {
        if !aanvaardbaar(header, payload) {
            self.verworpen += 1;
            return None;
        }

        // Een frame dat we al gehad hebben, of waarvan we de trein gemist hebben. Een
        // pariteitsfragment dat te laat komt is geen verlies maar de normale gang van
        // zaken: hij gaat als laatste de deur uit, dus bij een compleet beeld is hij per
        // definitie overbodig.
        if let Some(l) = self.laatste {
            let achterstand = l.wrapping_sub(header.timestamp);
            if achterstand == 0 || achterstand <= ACHTERSTAND_VENSTER {
                if !header.is_pariteit() {
                    self.verworpen += 1;
                }
                return None;
            }
            if !is_nieuwer(header.timestamp, l) {
                // Verder weg dan een te laat beeld en verder dan een omloop: de klok is
                // verzet (deler herstart) of iemand schuift onzin bij. Hersynchroniseren
                // in plaats van afwijzen — anders is één pakket genoeg om de stream
                // permanent vast te zetten en is er nergens een pad terug (B-11).
                self.onderweg.clear();
                self.laatste = None;
                self.hersynchronisaties += 1;
            }
        }

        // Op leeftijd opruimen, en niet op sleutel. De oude verdringing pakte de
        // *laagste* tijdstempel, en dat is precies wat uit te buiten viel: acht
        // onvolledige beelden geparkeerd op 0xFFFFFFF8.. waren daarmee onaantastbaar
        // terwijl elk écht beeld — dat een lagere tijdstempel heeft — eruit gegooid werd
        // (B-12c). Wie fragmenten krijgt is jong; wie stil ligt valt af.
        self.ruim_op(nu);

        let deel = self
            .onderweg
            .entry(header.timestamp)
            .or_insert_with(|| Halffabrikaat::nieuw(nu));
        deel.gezien = nu;
        if header.is_keyframe() {
            deel.keyframe = true;
        }
        if header.is_pariteit() {
            // `frag_index` is hier het aantal stukken, niet een plek erin.
            if !deel.pariteit_klopt(header.frag_index) {
                self.verworpen += 1;
                return None;
            }
            if !deel.aantal_is_hard {
                deel.zet_aantal(header.frag_index, false);
            }
            deel.pariteit = Some(payload.to_vec());
        } else {
            if header.is_last_fragment() {
                // `saturating_add` en niet `+ 1`: op 0xFFFF is dat in een debugbuild een
                // paniek op de kijkerthread, die langs `KijkerEvent::Gesloten` heen
                // unwindt zodat de motor niet merkt dat de kijker dood is (B-37).
                // `aanvaardbaar` weert die index al, dit is de tweede grendel.
                deel.zet_aantal(header.frag_index.saturating_add(1), true);
            }
            deel.stukken.insert(header.frag_index, payload.to_vec());
        }

        if !deel.compleet() && deel.herstel() {
            self.hersteld += 1;
        }

        if deel.compleet() {
            if deel.bytes() > MAX_FRAME_BYTES {
                // De grens hoort hier te zitten en niet in de OS-decoder: wat hier
                // uitkomt gaat er ongefilterd in, en een H.264-parser is precies waar
                // fouten op grote ongevalideerde invoer wonen (B-29).
                self.onderweg.remove(&header.timestamp);
                self.verworpen += 1;
                return None;
            }
            let klaar = self.onderweg.remove(&header.timestamp).expect("net gezien");
            let data = klaar.stukken.into_values().flatten().collect();

            // Alles wat ouder is dan dit frame gaat nooit meer compleet worden: de
            // stukken die nog ontbraken zijn onderweg verloren gegaan. Na een omloop
            // klopt deze sleutelvergelijking niet meer — die resten ruimt de vervaltijd
            // hierboven op.
            let ouder: Vec<u32> = self
                .onderweg
                .range(..header.timestamp)
                .map(|(&t, _)| t)
                .collect();
            for t in ouder {
                self.onderweg.remove(&t);
                self.incompleet += 1;
            }

            self.laatste = Some(header.timestamp);
            return Some(Frame {
                timestamp: header.timestamp,
                keyframe: klaar.keyframe,
                data,
            });
        }

        None
    }

    /// Gooit halffabrikaten weg die vervallen zijn, en daarna de stilste tot er weer
    /// hoogstens [`MAX_ONDERWEG`] over zijn. Loopt over maximaal negen elementen en leest
    /// de klok niet: alleen integervergelijkingen op het pakketpad.
    fn ruim_op(&mut self, nu: Instant) {
        let voor = self.onderweg.len();
        self.onderweg
            .retain(|_, d| nu.duration_since(d.gezien) < HALFFABRIKAAT_TTL);
        self.incompleet += (voor - self.onderweg.len()) as u64;

        // Blijven wachten op iets dat niet meer komt levert alleen maar oplopende
        // vertraging op. Het beeld waarvan net nog een fragment binnenkwam is nooit de
        // stilste, dus een echt beeld in opbouw blijft staan.
        while self.onderweg.len() > MAX_ONDERWEG {
            let Some(stilste) = self
                .onderweg
                .iter()
                .min_by_key(|(_, d)| d.gezien)
                .map(|(&t, _)| t)
            else {
                break;
            };
            self.onderweg.remove(&stilste);
            self.incompleet += 1;
        }
    }
}

/// Of dit pakket überhaupt bij een videobeeld kan horen.
///
/// Dit staat vóór elke allocatie: wat hier afvalt raakt de buffers niet. De twee maten
/// verschillen bewust. Een pariteitsfragment is twee bytes langer dan een
/// gegevensfragment omdat de lengte van het stuk in de XOR meegaat, en zijn `frag_index`
/// is het *aantal* stukken in plaats van een plek erin — dus 1..=MAX en niet 0..MAX.
/// Beide maten door elkaar halen weigert precies het pakket dat een verloren fragment
/// terugrekent. Zie `MediaHeader::FLAG_PARITEIT` en [`MAX_FRAGMENTEN_PER_BEELD`] (B-12a).
fn aanvaardbaar(header: &MediaHeader, payload: &[u8]) -> bool {
    if header.is_pariteit() {
        payload.len() <= PARITEIT_PAYLOAD_LEN
            && header.frag_index >= 1
            && header.frag_index <= MAX_FRAGMENTEN_PER_BEELD
    } else {
        payload.len() <= MAX_MEDIA_PAYLOAD && header.frag_index < MAX_FRAGMENTEN_PER_BEELD
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Alle pakketten van een beeld, inclusief het pariteitsfragment achteraan.
    fn pakketten(data: &[u8], keyframe: bool, ts: u32) -> Vec<(MediaHeader, Vec<u8>)> {
        headers_voor(1, ts, PayloadType::HEVC, keyframe, 100, data)
            .map(|(h, s)| (h, s.to_vec()))
            .collect()
    }

    /// Alleen de gegevensfragmenten, zoals de draad er zonder pariteit uitzag.
    fn gegevens(data: &[u8], keyframe: bool, ts: u32) -> Vec<(MediaHeader, Vec<u8>)> {
        pakketten(data, keyframe, ts)
            .into_iter()
            .filter(|(h, _)| !h.is_pariteit())
            .collect()
    }

    #[test]
    fn klein_frame_past_in_een_pakket() {
        let stukken: Vec<_> = fragmenteer(b"kort").collect();
        assert_eq!(stukken.len(), 1);
        assert_eq!(stukken[0], (0, true, &b"kort"[..]));
    }

    #[test]
    fn leeg_frame_levert_toch_een_afsluitend_fragment() {
        // Anders blijft de ontvanger eeuwig wachten op iets dat nooit komt.
        let stukken: Vec<_> = fragmenteer(&[]).collect();
        assert_eq!(stukken.len(), 1);
        assert!(stukken[0].1, "moet als laatste gemarkeerd zijn");
    }

    #[test]
    fn groot_frame_wordt_opgeknipt_en_weer_heel() {
        let bron: Vec<u8> = (0..50_000u32).map(|i| (i % 251) as u8).collect();
        let pakket = pakketten(&bron, true, 9000);
        assert!(pakket.len() > 40, "50 kB hoort in veel stukken te gaan");
        assert!(pakket.iter().all(|(h, s)| s.len()
            <= if h.is_pariteit() {
                PARITEIT_PAYLOAD_LEN
            } else {
                MAX_MEDIA_PAYLOAD
            }));

        let mut r = Reassembler::new();
        let mut uit = None;
        for (h, s) in &pakket {
            if let Some(f) = r.push(h, s) {
                uit = Some(f);
            }
        }
        let f = uit.expect("frame moet compleet worden");
        assert_eq!(f.data, bron);
        assert!(f.keyframe);
        assert_eq!(f.timestamp, 9000);
    }

    #[test]
    fn omgedraaide_fragmenten_leveren_hetzelfde_frame() {
        let bron: Vec<u8> = (0..5_000u32).map(|i| i as u8).collect();
        let mut pakket = pakketten(&bron, false, 100);
        pakket.reverse(); // het afsluitende fragment komt nu als eerste

        let mut r = Reassembler::new();
        let mut uit = None;
        for (h, s) in &pakket {
            if let Some(f) = r.push(h, s) {
                uit = Some(f);
            }
        }
        assert_eq!(uit.expect("compleet").data, bron);
    }

    #[test]
    fn twee_verloren_fragmenten_leveren_geen_half_frame_op() {
        // Eén gat repareert de pariteit; twee niet. Half beeld doorgeven aan de decoder
        // geeft groene blokken, en niets doorgeven is beter — dan blijft het vorige
        // beeld staan tot er een keyframe is.
        let bron: Vec<u8> = (0..5_000u32).map(|i| i as u8).collect();
        let mut pakket = pakketten(&bron, false, 100);
        pakket.remove(3);
        pakket.remove(1);

        let mut r = Reassembler::new();
        for (h, s) in &pakket {
            assert!(
                r.push(h, s).is_none(),
                "incompleet frame mag niet naar buiten"
            );
        }
        assert_eq!(r.hersteld, 0, "met twee gaten valt er niets te herstellen");
    }

    #[test]
    fn een_verloren_fragment_wordt_uit_de_pariteit_teruggerekend() {
        // Dit is waar het allemaal om draait. Gemeten op 2026-08-02: zonder dit kostte
        // één verloren fragment 70 tot 156 ms bevroren beeld, want de kijker spoelt zijn
        // decoder en wacht op een keyframe dat hij eerst moet aanvragen.
        let bron: Vec<u8> = (0..5_000u32).map(|i| (i % 251) as u8).collect();
        for weg in 0..5usize {
            let mut pakket = pakketten(&bron, false, 100);
            assert_eq!(pakket.len(), 6, "vijf stukken plus pariteit");
            pakket.remove(weg);

            let mut r = Reassembler::new();
            let mut uit = None;
            for (h, s) in &pakket {
                if let Some(f) = r.push(h, s) {
                    uit = Some(f);
                }
            }
            let f =
                uit.unwrap_or_else(|| panic!("fragment {weg} kwijt: hoort herstelbaar te zijn"));
            assert_eq!(f.data, bron, "fragment {weg} verkeerd teruggerekend");
            assert_eq!(r.hersteld, 1);
            assert_eq!(r.incompleet, 0, "er ging niets verloren");
        }
    }

    #[test]
    fn ook_het_laatste_kortere_fragment_is_terug_te_rekenen() {
        // Het laatste stuk is korter dan de rest én draagt de vlag waaraan de ontvanger
        // ziet hoeveel stukken er zijn. Beide zijn weg als juist dat stuk sneuvelt: de
        // lengte komt daarom door de XOR mee, en het aantal uit de `frag_index` van de
        // pariteit.
        let bron: Vec<u8> = (0..2_500u32).map(|i| (i % 251) as u8).collect();
        let mut pakket = pakketten(&bron, false, 100);
        assert_eq!(pakket.len(), 4, "drie stukken plus pariteit");
        pakket.remove(2); // het laatste gegevensfragment, 300 bytes

        let mut r = Reassembler::new();
        let mut uit = None;
        for (h, s) in &pakket {
            if let Some(f) = r.push(h, s) {
                uit = Some(f);
            }
        }
        assert_eq!(uit.expect("herstelbaar").data, bron);
        assert_eq!(r.hersteld, 1);
    }

    #[test]
    fn een_verloren_pariteitsfragment_verandert_niets() {
        // De pariteit is redundant zolang alles aankomt; hem kwijtraken mag niets kosten.
        let bron: Vec<u8> = (0..5_000u32).map(|i| i as u8).collect();
        let pakket = gegevens(&bron, false, 100);
        let mut r = Reassembler::new();
        let mut uit = None;
        for (h, s) in &pakket {
            if let Some(f) = r.push(h, s) {
                uit = Some(f);
            }
        }
        assert_eq!(uit.expect("compleet").data, bron);
        assert_eq!(r.hersteld, 0);
        assert_eq!(r.incompleet, 0);
    }

    #[test]
    fn de_pariteit_die_altijd_te_laat_komt_telt_niet_als_verlies() {
        // Het pariteitsfragment gaat als laatste de deur uit, dus bij een beeld dat
        // gewoon compleet werd is hij per definitie overbodig. Zou hij als `verworpen`
        // geteld worden, dan stond die meter permanent op zestig per seconde en was hij
        // niets meer waard als signaal.
        let bron = vec![9u8; 3_000];
        let pakket = pakketten(&bron, false, 100);
        let mut r = Reassembler::new();
        for (h, s) in &pakket {
            r.push(h, s);
        }
        assert_eq!(r.verworpen, 0);
    }

    #[test]
    fn een_beeld_van_een_enkel_fragment_is_ook_herstelbaar() {
        // Kleine P-beelden zijn één of twee fragmenten. Juist die zijn het goedkoopst om
        // te verliezen en het duurst om met een keyframe te herstellen.
        let bron = vec![3u8; 200];
        let mut pakket = pakketten(&bron, false, 100);
        assert_eq!(pakket.len(), 2, "één stuk plus pariteit");
        pakket.remove(0);

        let mut r = Reassembler::new();
        let f = pakket
            .iter()
            .find_map(|(h, s)| r.push(h, s))
            .expect("herstelbaar");
        assert_eq!(f.data, bron);
    }

    #[test]
    fn een_kapot_frame_blokkeert_het_volgende_niet() {
        let bron: Vec<u8> = (0..5_000u32).map(|i| i as u8).collect();
        // Twee gaten, dus ook de pariteit redt dit beeld niet meer.
        let mut kapot = pakketten(&bron, false, 100);
        kapot.remove(2);
        kapot.remove(1);
        let heel = pakketten(&bron, false, 200);

        let mut r = Reassembler::new();
        for (h, s) in &kapot {
            r.push(h, s);
        }
        let mut uit = None;
        for (h, s) in &heel {
            if let Some(f) = r.push(h, s) {
                uit = Some(f);
            }
        }
        assert_eq!(
            uit.expect("het volgende frame moet gewoon werken")
                .timestamp,
            200
        );
        assert_eq!(r.incompleet, 1, "het kapotte frame moet geteld zijn");
        assert_eq!(r.onderweg(), 0, "de resten moeten opgeruimd zijn");
    }

    #[test]
    fn nagekomen_fragment_van_een_afgehandeld_frame_wordt_genegeerd() {
        let bron = vec![7u8; 3_000];
        let pakket = pakketten(&bron, false, 100);
        let mut r = Reassembler::new();
        for (h, s) in &pakket {
            r.push(h, s);
        }
        assert!(r.push(&pakket[0].0, &pakket[0].1).is_none());
        assert_eq!(r.verworpen, 1);
    }

    #[test]
    fn eindeloos_wachten_op_kapotte_frames_loopt_niet_vol() {
        // Slechte verbinding: er komt van elk frame maar één stukje binnen.
        let bron = vec![1u8; 5_000];
        let mut r = Reassembler::new();
        for ts in 1..100u32 {
            let pakket = pakketten(&bron, false, ts * 100);
            r.push(&pakket[0].0, &pakket[0].1);
        }
        assert!(
            r.onderweg() <= MAX_ONDERWEG + 1,
            "er blijven {} halve frames hangen",
            r.onderweg()
        );
        assert!(r.incompleet > 50, "die moeten wel geteld worden");
    }

    /// Eén los pakket, zonder van een echt beeld te komen.
    fn los(ts: u32, frag_index: u16, flags: u8) -> MediaHeader {
        MediaHeader {
            stream_id: 1,
            seq: 0,
            timestamp: ts,
            payload_type: PayloadType::H264,
            flags,
            frag_index,
        }
    }

    #[test]
    fn b11_een_pakket_met_de_hoogste_tijdstempel_bevriest_de_stream_niet() {
        // Eén pakket van 45 bytes met `timestamp = u32::MAX` en het afsluitende vlaggetje
        // is binnen één push compleet, en zette `laatste` daarmee op het maximum. Daarna
        // faalde élk legitiem pakket op `timestamp <= laatste`, vroeg de kijker elke 500
        // ms tevergeefs een keyframe, en was er nergens een pad terug: alleen het venster
        // sluiten en heropenen hielp.
        let mut r = Reassembler::new();
        let aanval = los(u32::MAX, 0, MediaHeader::FLAG_LAST_FRAGMENT);
        assert!(r.push(&aanval, b"rommel").is_some(), "gaat er als beeld in");

        let bron = vec![5u8; 3_000];
        for ts in [1_500u32, 3_000, 4_500] {
            let pakket = pakketten(&bron, true, ts);
            let uit = pakket.iter().find_map(|(h, s)| r.push(h, s));
            assert_eq!(
                uit.unwrap_or_else(|| panic!("beeld op {ts} kwam niet door"))
                    .data,
                bron
            );
        }
    }

    #[test]
    fn b30_de_tijdstempel_mag_omlopen_in_de_samensteller() {
        // De 90 kHz-klok loopt elke 13 uur 15 minuten om. Dat is een sprong terug, en die
        // gooide de samensteller weg — dus zag `Uitvouwer` in de kijker hem nooit en
        // bevroor het beeld precies als bij B-11. Zie ook de ketentest in `kijker.rs`.
        let bron = vec![2u8; 2_000];
        let mut r = Reassembler::new();

        let voor = u32::MAX - 1_500;
        assert!(pakketten(&bron, true, voor)
            .iter()
            .any(|(h, s)| r.push(h, s).is_some()));

        let na = 1_500u32; // net over de omloop heen
        let uit = pakketten(&bron, false, na)
            .iter()
            .find_map(|(h, s)| r.push(h, s));
        assert_eq!(
            uit.expect("het beeld na de omloop moet doorkomen").data,
            bron
        );
        assert_eq!(r.hersynchronisaties, 0, "een omloop is geen sprong");
    }

    #[test]
    fn b11_een_sprong_buiten_het_redelijke_hersynchroniseert() {
        // Wat geen omloop en geen te laat beeld is — een deler die herstart, of iemand die
        // onzin bijschuift — mag hoogstens één keer een hik kosten, geen permanente
        // blokkade.
        // Precies de overkant van de klok: dat is niet als omloop te lezen en ook niet als
        // te laat beeld, en juist daar hoort het opnieuw te beginnen.
        let bron = vec![4u8; 1_500];
        let mut r = Reassembler::new();
        assert!(pakketten(&bron, true, 0x8000_0000)
            .iter()
            .any(|(h, s)| r.push(h, s).is_some()));

        let uit = pakketten(&bron, true, 1_000)
            .iter()
            .find_map(|(h, s)| r.push(h, s));
        assert!(uit.is_some(), "na de sprong moet het beeld weer lopen");
        assert_eq!(r.hersynchronisaties, 1);
    }

    #[test]
    fn b12_een_te_grote_payload_wordt_geweigerd() {
        // Een datagram mag tot 1484 bytes payload bevatten, ruim boven de 1100 die een
        // gegevensfragment hoort te zijn. Aan ontvangstzijde werd dat nergens afgedwongen.
        let mut r = Reassembler::new();
        let h = los(100, 0, MediaHeader::FLAG_LAST_FRAGMENT);
        assert!(r.push(&h, &vec![0u8; MAX_MEDIA_PAYLOAD + 1]).is_none());
        assert_eq!(r.verworpen, 1);
        assert_eq!(r.onderweg(), 0, "mag geen geheugen gekost hebben");
    }

    #[test]
    fn b12_een_fragmentindex_buiten_het_beeld_wordt_geweigerd() {
        // 0xFFFF was bovendien een paniek op `frag_index + 1` (B-37): in een debugbuild
        // unwindt die langs `KijkerEvent::Gesloten` heen, zodat de motor niet merkt dat de
        // kijker dood is en de UI een actieve stream blijft tonen.
        let mut r = Reassembler::new();
        for index in [MAX_FRAGMENTEN_PER_BEELD, u16::MAX] {
            assert!(r
                .push(&los(100, index, MediaHeader::FLAG_LAST_FRAGMENT), b"x")
                .is_none());
        }
        assert_eq!(r.verworpen, 2);
        assert_eq!(r.onderweg(), 0);
    }

    #[test]
    fn b12_de_pariteit_heeft_zijn_eigen_maten_en_sneuvelt_er_niet_op() {
        // Een pariteitsfragment is twee bytes langer dan een gegevensfragment (de lengte
        // gaat door de XOR mee) en zijn `frag_index` is het *aantal* stukken. Wie voor
        // beide dezelfde grens gebruikt weigert precies het pakket dat verlies herstelt.
        let mut r = Reassembler::new();
        let pariteit = los(100, MAX_FRAGMENTEN_PER_BEELD, MediaHeader::FLAG_PARITEIT);
        assert!(r
            .push(&pariteit, &vec![0u8; PARITEIT_PAYLOAD_LEN])
            .is_none());
        assert_eq!(r.verworpen, 0, "dit is een geldig pariteitspakket");

        // Nul stukken bestaat niet, en meer dan de grens ook niet.
        for index in [0, MAX_FRAGMENTEN_PER_BEELD + 1] {
            r.push(
                &los(200, index, MediaHeader::FLAG_PARITEIT),
                &vec![0u8; PARITEIT_PAYLOAD_LEN],
            );
        }
        assert_eq!(r.verworpen, 2);
    }

    #[test]
    fn b12_een_beeld_dat_nooit_afkomt_verdwijnt_op_leeftijd() {
        // Laat het afsluitende fragment én de pariteit weg en het halffabrikaat bleef
        // eeuwig staan: acht daarvan waren samen bijna 800 MB.
        let mut r = Reassembler::new();
        let begin = Instant::now();
        r.push_op(begin, &los(100, 0, 0), b"stuk");
        assert_eq!(r.onderweg(), 1);

        r.push_op(begin + HALFFABRIKAAT_TTL, &los(200, 0, 0), b"stuk");
        assert_eq!(r.onderweg(), 1, "het oude moet vervallen zijn");
        assert_eq!(r.incompleet, 1);
    }

    #[test]
    fn b12_geparkeerde_beelden_verdringen_een_echt_beeld_niet() {
        // De oude verdringing gooide de *laagste* tijdstempel weg. Acht onvolledige
        // beelden op 0xFFFFFFF8.. waren daarmee onaantastbaar en elk echt beeld — dat een
        // lagere tijdstempel heeft — sneuvelde bij het volgende aanvallerspakket. Een
        // druppel van ~100 pakketten per seconde was genoeg om nooit meer beeld te
        // krijgen, en in het log liep alleen `incompleet` op.
        let bron = vec![6u8; 5_000];
        let mut r = Reassembler::new();
        let begin = Instant::now();
        for (i, ts) in (0xFFFF_FFF8u32..=0xFFFF_FFFF).enumerate() {
            r.push_op(
                begin + Duration::from_micros(i as u64),
                &los(ts, 0, 0),
                b"parkeer",
            );
        }
        assert_eq!(r.onderweg(), MAX_ONDERWEG);

        let nu = begin + Duration::from_millis(1);
        let uit = pakketten(&bron, true, 1_000)
            .iter()
            .find_map(|(h, s)| r.push_op(nu, h, s));
        assert_eq!(
            uit.expect("het echte beeld moet compleet worden").data,
            bron
        );
    }

    #[test]
    fn b28_een_geinjecteerd_fragment_levert_geen_half_beeld_op() {
        // `compleet()` telde alleen het aantal stukken. Wie het IP van de deler kan
        // spoofen stuurde een fragment op een index buiten het beeld, waarna het aantal
        // klopte, er een gat in zat, en het geheel als authentiek de decoder in ging.
        let bron: Vec<u8> = (0..5_000u32).map(|i| i as u8).collect();
        let mut pakket = gegevens(&bron, false, 100); // vijf stukken, geen pariteit
        pakket.remove(2);

        let mut r = Reassembler::new();
        // Injectie op index 9: buiten het beeld, maar goed voor het aantal.
        r.push(&los(100, 9, 0), &vec![0xAAu8; 1_100]);
        for (h, s) in &pakket {
            assert!(
                r.push(h, s).is_none(),
                "een beeld met een gat mag niet naar buiten"
            );
        }
    }

    #[test]
    fn b28_een_pariteit_met_een_verkeerd_aantal_kapt_het_beeld_niet_af() {
        // Een pariteitspakket gaat als laatste de deur uit, dus zijn aantal hoort boven
        // elke index te liggen die er al is. Een los pakket dat "er zijn er twee" beweert
        // terwijl er al vier stukken liggen kwam hier niet vandaan: het aannemen gooide de
        // echte stukken weg en leverde een afgekapt beeld dat als authentiek de decoder in
        // ging.
        let bron: Vec<u8> = (0..5_000u32).map(|i| i as u8).collect();
        let echt = pakketten(&bron, false, 100);
        let mut r = Reassembler::new();

        // Vier van de vijf stukken; juist het afsluitende (met de vlag) is zoek, dus het
        // aantal staat nog niet vast.
        for (h, s) in echt.iter().filter(|(h, _)| !h.is_pariteit()).take(4) {
            assert!(r.push(h, s).is_none());
        }
        assert!(r
            .push(
                &los(100, 2, MediaHeader::FLAG_PARITEIT),
                &vec![0u8; PARITEIT_PAYLOAD_LEN]
            )
            .is_none());
        assert_eq!(r.verworpen, 1, "die pariteit hoort geweigerd te zijn");

        // De echte pariteit doet daarna nog gewoon zijn werk: het vijfde stuk terugrekenen.
        let pariteit = echt
            .iter()
            .find(|(h, _)| h.is_pariteit())
            .expect("pariteit");
        let uit = r.push(&pariteit.0, &pariteit.1);
        assert_eq!(uit.expect("herstelbaar").data, bron);
        assert_eq!(r.hersteld, 1);
    }

    #[test]
    fn b28_een_pariteit_mag_het_afsluitende_fragment_niet_overrulen() {
        // Ligt het aantal al vast doordat het afsluitende fragment binnen is, dan is dat
        // gezaghebbend: het staat in dezelfde stroom als de stukken zelf.
        let bron: Vec<u8> = (0..5_000u32).map(|i| i as u8).collect();
        let echt = pakketten(&bron, false, 100);
        let mut r = Reassembler::new();
        let gegevens: Vec<_> = echt.iter().filter(|(h, _)| !h.is_pariteit()).collect();
        for (h, s) in gegevens.iter().skip(1) {
            assert!(r.push(h, s).is_none()); // stuk 0 is zoek, het afsluitende is er wel
        }
        assert!(r
            .push(
                &los(100, 3, MediaHeader::FLAG_PARITEIT),
                &vec![0u8; PARITEIT_PAYLOAD_LEN]
            )
            .is_none());
        assert_eq!(r.verworpen, 1);

        let pariteit = echt
            .iter()
            .find(|(h, _)| h.is_pariteit())
            .expect("pariteit");
        assert_eq!(
            r.push(&pariteit.0, &pariteit.1).expect("herstelbaar").data,
            bron
        );
    }

    #[test]
    fn b29_een_te_groot_beeld_gaat_niet_naar_de_decoder() {
        // Wat de samensteller oplevert ging ongefilterd naar de H.264-decoder van het
        // besturingssysteem — een grote, ongevalideerde invoer aan closed-source code, en
        // dat is precies waar parserfouten wonen. Gemeten keyframes zijn 100 tot 371 kB.
        let mut r = Reassembler::new();
        let stuk = vec![0u8; MAX_MEDIA_PAYLOAD];
        let laatste = MAX_FRAGMENTEN_PER_BEELD - 1;
        for i in 0..MAX_FRAGMENTEN_PER_BEELD {
            let vlag = if i == laatste {
                MediaHeader::FLAG_LAST_FRAGMENT
            } else {
                0
            };
            assert!(
                r.push(&los(100, i, vlag), &stuk).is_none(),
                "fragment {i}: 1,1 MB hoort nergens heen te gaan"
            );
        }
        assert_eq!(r.verworpen, 1);
        assert_eq!(r.onderweg(), 0, "de resten moeten opgeruimd zijn");
    }

    #[test]
    fn keyframe_vlag_overleeft_de_reis() {
        let bron = vec![3u8; 4_000];
        let pakket = pakketten(&bron, true, 500);
        let mut r = Reassembler::new();
        let mut uit = None;
        // Alleen het eerste fragment draagt de vlag niet per se als laatste binnenkomt;
        // de vlag moet hoe dan ook blijven staan.
        for (h, s) in pakket.iter().rev() {
            if let Some(f) = r.push(h, s) {
                uit = Some(f);
            }
        }
        assert!(uit.expect("compleet").keyframe);
    }
}
