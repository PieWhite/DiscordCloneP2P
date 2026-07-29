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

use fitcom_proto::{MediaHeader, PayloadType, MAX_MEDIA_PAYLOAD};
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

/// Bouwt de headers voor een compleet frame. Handig omdat de vlaggen makkelijk fout gaan.
pub fn headers_voor(
    stream_id: u32,
    timestamp: u32,
    payload_type: PayloadType,
    keyframe: bool,
    eerste_seq: u32,
    data: &[u8],
) -> impl Iterator<Item = (MediaHeader, &[u8])> {
    fragmenteer(data).map(move |(idx, laatste, stuk)| {
        let mut flags = 0u8;
        if keyframe {
            flags |= MediaHeader::FLAG_KEYFRAME;
        }
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
            stuk,
        )
    })
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
    /// Bekend zodra het laatste fragment binnen is, ook als dat als eerste aankwam.
    aantal: Option<u16>,
    keyframe: bool,
}

impl Halffabrikaat {
    fn compleet(&self) -> bool {
        self.aantal
            .is_some_and(|n| self.stukken.len() == usize::from(n))
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
        }
    }

    pub fn onderweg(&self) -> usize {
        self.onderweg.len()
    }

    /// Levert een frame op zodra alle stukken binnen zijn.
    pub fn push(&mut self, header: &MediaHeader, payload: &[u8]) -> Option<Frame> {
        // Een frame dat we al gehad hebben, of waarvan we de trein gemist hebben.
        if let Some(l) = self.laatste {
            if header.timestamp <= l {
                self.verworpen += 1;
                return None;
            }
        }

        let deel = self.onderweg.entry(header.timestamp).or_default();
        if header.is_keyframe() {
            deel.keyframe = true;
        }
        if header.is_last_fragment() {
            deel.aantal = Some(header.frag_index + 1);
        }
        deel.stukken.insert(header.frag_index, payload.to_vec());

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

    fn pakketten(data: &[u8], keyframe: bool, ts: u32) -> Vec<(MediaHeader, Vec<u8>)> {
        headers_voor(1, ts, PayloadType::HEVC, keyframe, 100, data)
            .map(|(h, s)| (h, s.to_vec()))
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
        assert!(pakket.iter().all(|(_, s)| s.len() <= MAX_MEDIA_PAYLOAD));

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
    fn een_verloren_fragment_levert_geen_half_frame_op() {
        // Half beeld doorgeven aan de decoder geeft groene blokken; niets doorgeven
        // is beter, dan blijft het vorige frame staan.
        let bron: Vec<u8> = (0..5_000u32).map(|i| i as u8).collect();
        let mut pakket = pakketten(&bron, false, 100);
        pakket.remove(2);

        let mut r = Reassembler::new();
        for (h, s) in &pakket {
            assert!(
                r.push(h, s).is_none(),
                "incompleet frame mag niet naar buiten"
            );
        }
    }

    #[test]
    fn een_kapot_frame_blokkeert_het_volgende_niet() {
        let bron: Vec<u8> = (0..5_000u32).map(|i| i as u8).collect();
        let mut kapot = pakketten(&bron, false, 100);
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
