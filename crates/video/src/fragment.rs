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

/// Zoveel frames houden we tegelijk in de lucht. Meer betekent dat we op iets wachten
/// dat toch niet meer komt, en dan is doorgaan beter dan blijven verzamelen.
const MAX_ONDERWEG: usize = 8;

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

#[derive(Default)]
struct Halffabrikaat {
    stukken: BTreeMap<u16, Vec<u8>>,
    /// Bekend zodra het laatste fragment binnen is, ook als dat als eerste aankwam, en
    /// anders zodra het pariteitsfragment binnen is.
    aantal: Option<u16>,
    /// De XOR van alle stukken, zie [`pariteit_van`].
    pariteit: Option<Vec<u8>>,
    keyframe: bool,
}

impl Halffabrikaat {
    fn compleet(&self) -> bool {
        self.aantal
            .is_some_and(|n| self.stukken.len() == usize::from(n))
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
        }
    }

    pub fn onderweg(&self) -> usize {
        self.onderweg.len()
    }

    /// Levert een frame op zodra alle stukken binnen zijn.
    pub fn push(&mut self, header: &MediaHeader, payload: &[u8]) -> Option<Frame> {
        // Een frame dat we al gehad hebben, of waarvan we de trein gemist hebben. Een
        // pariteitsfragment dat te laat komt is geen verlies maar de normale gang van
        // zaken: hij gaat als laatste de deur uit, dus bij een compleet beeld is hij per
        // definitie overbodig.
        if let Some(l) = self.laatste {
            if header.timestamp <= l {
                if !header.is_pariteit() {
                    self.verworpen += 1;
                }
                return None;
            }
        }

        let deel = self.onderweg.entry(header.timestamp).or_default();
        if header.is_keyframe() {
            deel.keyframe = true;
        }
        if header.is_pariteit() {
            // `frag_index` is hier het aantal stukken, niet een plek erin.
            deel.aantal = Some(header.frag_index);
            deel.pariteit = Some(payload.to_vec());
        } else {
            if header.is_last_fragment() {
                deel.aantal = Some(header.frag_index + 1);
            }
            deel.stukken.insert(header.frag_index, payload.to_vec());
        }

        if !deel.compleet() && deel.herstel() {
            self.hersteld += 1;
        }

        if deel.compleet() {
            let klaar = self.onderweg.remove(&header.timestamp).expect("net gezien");
            let data = klaar.stukken.into_values().flatten().collect();

            // Alles wat ouder is dan dit frame gaat nooit meer compleet worden: de
            // stukken die nog ontbraken zijn onderweg verloren gegaan.
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

        // Te veel tegelijk onderweg: de oudste gaat er af. Blijven wachten op iets dat
        // niet meer komt levert alleen maar oplopende vertraging op.
        while self.onderweg.len() > MAX_ONDERWEG {
            let oudste = *self.onderweg.keys().next().expect("niet leeg");
            self.onderweg.remove(&oudste);
            self.incompleet += 1;
        }

        None
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
