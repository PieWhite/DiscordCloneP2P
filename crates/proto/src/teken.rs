//! Tekenen op een gedeeld scherm (2026-10-15, beslissing 45): het palet, de grenzen, en de
//! toestand die elke kant uit dezelfde berichten opbouwt.
//!
//! De deler van een scherm houdt de echte tekening bij; elke kijker een kopie die hij alleen
//! van de deler aanneemt. Beide passen de berichten toe met [`Tekening`], dus met dezelfde
//! grenzen in dezelfde volgorde — de deler stuurt alles door in de volgorde waarin hij het
//! zelf toepaste. Zolang er niets onderweg verloren gaat lopen ze dus niet uit elkaar, en na
//! elk wissen vervangt een `DrawState` de kopie toch helemaal. Gaat er wél iets verloren (een
//! volle verzendwachtrij in de mesh, een verbinding die vervangen wordt), dan ziet de kijker
//! dat aan een gat ([`Tekening::heeft_gat`]) en haalt hij de hele tekening opnieuw.
//!
//! Alles wat hier binnenkomt komt van een andere PC. De grenzen zijn er om geheugen en
//! tekenwerk te begrenzen, niet om fouten van een eerlijke peer op te vangen: die blijft er
//! met de invoer van het venster ruim onder.

use crate::control::{Draw, DrawLine, DrawState};
use crate::PeerId;
use std::sync::Arc;

/// Eén kleur van het palet. De naam staat in het menu van het kijkvenster.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kleur {
    pub naam: &'static str,
    /// `[r, g, b]`, dekkend.
    pub rgb: [u8; 3],
}

/// Het palet, één keer gedefinieerd voor kijker, deler en de tegels in het hoofdvenster.
/// Fel en verzadigd: een lijn moet opvallen op een willekeurig spelbeeld. Op de draad staat
/// de index; een kleur erbij is een regel erbij, achteraan.
pub const KLEUREN: [Kleur; 6] = [
    Kleur {
        naam: "Red",
        rgb: [0xFF, 0x3B, 0x3B],
    },
    Kleur {
        naam: "Yellow",
        rgb: [0xFF, 0xD5, 0x00],
    },
    Kleur {
        naam: "Green",
        rgb: [0x2E, 0xE6, 0x6B],
    },
    Kleur {
        naam: "Cyan",
        rgb: [0x2E, 0xC8, 0xFF],
    },
    Kleur {
        naam: "Magenta",
        rgb: [0xFF, 0x4F, 0xD8],
    },
    Kleur {
        naam: "White",
        rgb: [0xFF, 0xFF, 0xFF],
    },
];

/// Dun, middel, dik: de lijndikte als deel van de beeldhoogte, zodat een lijn op elk
/// scherm en in elk venster even dik oogt ten opzichte van het beeld. Bij 1080 regels is
/// dat 4, 8 en 15 pixels.
pub const DIKTES: [(&str, f32); 3] = [("Thin", 0.0035), ("Medium", 0.007), ("Thick", 0.014)];

/// Zoveel punten hooguit in één `Draw`. Een kijker stuurt elke ~40 ms wat er bij kwam, dus
/// dit raakt hij alleen als de muis heel snel gaat; dan volgt er gewoon een tweede bericht.
pub const MAX_PUNTEN_PER_BERICHT: usize = 64;
/// Zoveel punten hooguit in één lijn. Het venster begint daarna vanzelf een nieuwe lijn.
pub const MAX_PUNTEN_PER_LIJN: usize = 512;
/// Zoveel lijnen houdt één tekenaar hooguit op één scherm; de oudste valt eraf.
pub const MAX_LIJNEN_PER_TEKENAAR: usize = 32;
/// Zoveel lijnen hooguit op één scherm, van iedereen samen. Een nieuwe lijn daarboven wordt
/// genegeerd. Geen grens op het aantal tekenaars: dit begrenst ze al, voor elke N.
pub const MAX_LIJNEN: usize = 256;

/// Eén lijn: wie, welke, hoe, en de punten in genormaliseerde coördinaten.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lijn {
    pub tekenaar: PeerId,
    pub id: u32,
    pub kleur: u8,
    pub dikte: u8,
    pub punten: Vec<[u16; 2]>,
}

impl Lijn {
    pub fn rgb(&self) -> [u8; 3] {
        KLEUREN[usize::from(self.kleur).min(KLEUREN.len() - 1)].rgb
    }

    /// Diameter als deel van de beeldhoogte.
    pub fn dikte(&self) -> f32 {
        DIKTES[usize::from(self.dikte).min(DIKTES.len() - 1)].1
    }
}

/// Alles wat er op één scherm getekend staat, plus of dat mag.
///
/// De lijnen staan achter een `Arc`: de motor geeft de hele tekening na elke wijziging door
/// aan een venster op een andere thread, en dan kost een kopie alleen de lijn die groeide.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tekening {
    /// Mag er getekend worden. Bij de deler is dit zijn schakelaar, bij een kijker wat de
    /// deler aankondigde. Uit betekent ook: niets te zien.
    pub aan: bool,
    /// In de volgorde waarin ze begonnen; later begonnen staat erbovenop.
    lijnen: Vec<Arc<Lijn>>,
}

impl Tekening {
    pub fn nieuw(aan: bool) -> Self {
        Self {
            aan,
            lijnen: Vec::new(),
        }
    }

    pub fn lijnen(&self) -> &[Arc<Lijn>] {
        &self.lijnen
    }

    pub fn is_leeg(&self) -> bool {
        self.lijnen.is_empty()
    }

    /// Punten erbij van `tekenaar`. `false` als er niets veranderde: tekenen staat uit, een
    /// onbekende kleur of dikte (van een nieuwere peer), te veel punten in één bericht, een
    /// gat in de punten, of een grens. Tweemaal hetzelfde bericht is een no-op.
    pub fn teken(&mut self, tekenaar: PeerId, d: &Draw) -> bool {
        if d.points.len() > MAX_PUNTEN_PER_BERICHT {
            return false;
        }
        self.voeg_toe(tekenaar, d.line, d.color, d.width, d.start, &d.points)
    }

    /// Of `d` verder gaat dan wat wij van die lijn hebben. De deler geeft alleen door wat hij
    /// zelf aannam, dus bij een kijker betekent dit dat er onderweg een bericht verloren ging
    /// (een volle verzendwachtrij, een vervangen verbinding): dan klopt de kopie niet meer en
    /// moet hij opnieuw de hele tekening halen.
    pub fn heeft_gat(&self, tekenaar: PeerId, d: &Draw) -> bool {
        let heeft = self
            .lijnen
            .iter()
            .find(|l| l.tekenaar == tekenaar && l.id == d.line)
            .map_or(0, |l| l.punten.len());
        d.start as usize > heeft
    }

    fn voeg_toe(
        &mut self,
        tekenaar: PeerId,
        id: u32,
        kleur: u8,
        dikte: u8,
        start: u32,
        punten: &[[u16; 2]],
    ) -> bool {
        if !self.aan
            || punten.is_empty()
            || usize::from(kleur) >= KLEUREN.len()
            || usize::from(dikte) >= DIKTES.len()
        {
            return false;
        }
        let start = start as usize;

        if let Some(lijn) = self
            .lijnen
            .iter_mut()
            .find(|l| l.tekenaar == tekenaar && l.id == id)
        {
            let heeft = lijn.punten.len();
            // Een gat: er ging onderweg iets verloren. De lijn houden zoals hij is; een
            // kijker haalt dan de hele tekening opnieuw (zie `heeft_gat`).
            if start > heeft {
                return false;
            }
            let nieuw = &punten[(heeft - start).min(punten.len())..];
            let ruimte = MAX_PUNTEN_PER_LIJN.saturating_sub(heeft);
            if nieuw.is_empty() || ruimte == 0 {
                return false;
            }
            Arc::make_mut(lijn)
                .punten
                .extend_from_slice(&nieuw[..nieuw.len().min(ruimte)]);
            return true;
        }

        // Een lijn begint bij punt nul. Een vervolg op een lijn die we niet (meer) hebben —
        // ongedaan gemaakt, of van vóór we keken — negeren we.
        if start != 0 {
            return false;
        }
        let van_hem = self
            .lijnen
            .iter()
            .filter(|l| l.tekenaar == tekenaar)
            .count();
        if van_hem >= MAX_LIJNEN_PER_TEKENAAR {
            if let Some(oudste) = self.lijnen.iter().position(|l| l.tekenaar == tekenaar) {
                self.lijnen.remove(oudste);
            }
        }
        if self.lijnen.len() >= MAX_LIJNEN {
            return false;
        }
        self.lijnen.push(Arc::new(Lijn {
            tekenaar,
            id,
            kleur,
            dikte,
            punten: punten[..punten.len().min(MAX_PUNTEN_PER_LIJN)].to_vec(),
        }));
        true
    }

    /// De laatst begonnen lijn van `tekenaar` weg.
    pub fn ongedaan(&mut self, tekenaar: PeerId) -> bool {
        match self.lijnen.iter().rposition(|l| l.tekenaar == tekenaar) {
            Some(i) => {
                self.lijnen.remove(i);
                true
            }
            None => false,
        }
    }

    /// Alle lijnen van `tekenaar` weg.
    pub fn wis_van(&mut self, tekenaar: PeerId) -> bool {
        let voor = self.lijnen.len();
        self.lijnen.retain(|l| l.tekenaar != tekenaar);
        self.lijnen.len() != voor
    }

    /// Alles weg.
    pub fn wis(&mut self) -> bool {
        let had = !self.lijnen.is_empty();
        self.lijnen.clear();
        had
    }

    /// Tekenen aan of uit. Uit wist ook alles: wie het uitzet wil de lijnen kwijt. `true`
    /// als er iets veranderde.
    pub fn zet_aan(&mut self, aan: bool) -> bool {
        let wissel = self.aan != aan;
        self.aan = aan;
        let gewist = !aan && self.wis();
        wissel || gewist
    }

    /// De hele tekening als bericht, voor een nieuwe kijker of na wissen.
    pub fn als_toestand(&self, stream_id: u32) -> DrawState {
        DrawState {
            stream_id,
            lines: self
                .lijnen
                .iter()
                .map(|l| DrawLine {
                    author: l.tekenaar,
                    line: l.id,
                    color: l.kleur,
                    width: l.dikte,
                    points: l.punten.clone(),
                })
                .collect(),
        }
    }

    /// Een ontvangen `DrawState`, door dezelfde grenzen als losse lijnen. Een lijn met te
    /// veel punten wordt afgekapt, een onbekende kleur valt weg, en boven de grenzen houdt
    /// het op — precies wat de deler zelf ook gedaan zou hebben.
    pub fn uit_toestand(aan: bool, s: &DrawState) -> Self {
        let mut t = Self::nieuw(aan);
        for l in &s.lines {
            t.voeg_toe(l.author, l.line, l.color, l.width, 0, &l.points);
        }
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lijn(id: u32, start: u32, punten: &[[u16; 2]]) -> Draw {
        Draw {
            stream_id: 1,
            author: None,
            line: id,
            color: 0,
            width: 1,
            start,
            points: punten.to_vec(),
        }
    }

    fn punten(t: &Tekening, wie: PeerId, id: u32) -> Option<usize> {
        t.lijnen()
            .iter()
            .find(|l| l.tekenaar == wie && l.id == id)
            .map(|l| l.punten.len())
    }

    #[test]
    fn een_lijn_groeit_en_tweemaal_toepassen_is_een_no_op() {
        let wie = PeerId::new_random();
        let mut t = Tekening::nieuw(true);
        assert!(t.teken(wie, &lijn(1, 0, &[[0, 0], [10, 10]])));
        assert!(t.teken(wie, &lijn(1, 2, &[[20, 20], [30, 30]])));
        assert_eq!(punten(&t, wie, 1), Some(4));

        let voor = t.clone();
        assert!(!t.teken(wie, &lijn(1, 2, &[[20, 20], [30, 30]])), "dubbel");
        assert!(
            !t.teken(wie, &lijn(1, 0, &[[0, 0], [10, 10]])),
            "dubbel begin"
        );
        assert_eq!(t, voor);

        // Overlap: alleen het stuk dat nieuw is komt erbij.
        assert!(t.teken(wie, &lijn(1, 3, &[[30, 30], [40, 40]])));
        assert_eq!(punten(&t, wie, 1), Some(5));
    }

    #[test]
    fn een_gat_of_een_vervolg_zonder_begin_wordt_genegeerd() {
        let wie = PeerId::new_random();
        let mut t = Tekening::nieuw(true);
        assert!(
            !t.teken(wie, &lijn(1, 5, &[[0, 0]])),
            "vervolg zonder begin"
        );
        assert!(t.teken(wie, &lijn(1, 0, &[[0, 0], [1, 1]])));
        assert!(!t.teken(wie, &lijn(1, 3, &[[9, 9]])), "gat");
        assert_eq!(punten(&t, wie, 1), Some(2));

        // Het gat is te zien, een dubbel bericht niet.
        assert!(t.heeft_gat(wie, &lijn(1, 3, &[[9, 9]])));
        assert!(
            t.heeft_gat(wie, &lijn(7, 1, &[[9, 9]])),
            "lijn die we missen"
        );
        assert!(!t.heeft_gat(wie, &lijn(1, 2, &[[9, 9]])));
        assert!(!t.heeft_gat(wie, &lijn(1, 0, &[[0, 0], [1, 1]])));
    }

    #[test]
    fn onbekende_kleur_of_dikte_en_te_grote_berichten_worden_genegeerd() {
        let wie = PeerId::new_random();
        let mut t = Tekening::nieuw(true);
        let mut d = lijn(1, 0, &[[0, 0], [1, 1]]);
        d.color = KLEUREN.len() as u8;
        assert!(!t.teken(wie, &d));
        let mut d = lijn(1, 0, &[[0, 0], [1, 1]]);
        d.width = DIKTES.len() as u8;
        assert!(!t.teken(wie, &d));
        let d = lijn(1, 0, &vec![[0, 0]; MAX_PUNTEN_PER_BERICHT + 1]);
        assert!(!t.teken(wie, &d));
        assert!(!t.teken(wie, &lijn(1, 0, &[])), "leeg");
        assert!(t.is_leeg());
    }

    #[test]
    fn uit_neemt_niets_aan_en_uitzetten_wist() {
        let wie = PeerId::new_random();
        let mut t = Tekening::nieuw(false);
        assert!(!t.teken(wie, &lijn(1, 0, &[[0, 0], [1, 1]])));
        assert!(t.zet_aan(true));
        assert!(t.teken(wie, &lijn(1, 0, &[[0, 0], [1, 1]])));
        assert!(t.zet_aan(false));
        assert!(t.is_leeg() && !t.aan);
        assert!(!t.zet_aan(false), "al uit en leeg");
    }

    #[test]
    fn een_lijn_stopt_bij_zijn_grens() {
        let wie = PeerId::new_random();
        let mut t = Tekening::nieuw(true);
        let stuk = vec![[7u16, 7]; MAX_PUNTEN_PER_BERICHT];
        let mut start = 0;
        while t.teken(wie, &lijn(1, start, &stuk)) {
            start += MAX_PUNTEN_PER_BERICHT as u32;
        }
        assert_eq!(punten(&t, wie, 1), Some(MAX_PUNTEN_PER_LIJN));
    }

    #[test]
    fn de_oudste_lijn_van_een_tekenaar_valt_eraf_en_de_ander_blijft() {
        let a = PeerId::new_random();
        let b = PeerId::new_random();
        let mut t = Tekening::nieuw(true);
        assert!(t.teken(b, &lijn(1, 0, &[[0, 0], [1, 1]])));
        for id in 1..=MAX_LIJNEN_PER_TEKENAAR as u32 + 1 {
            assert!(t.teken(a, &lijn(id, 0, &[[0, 0], [1, 1]])));
        }
        assert_eq!(punten(&t, a, 1), None, "de oudste van a is weg");
        assert_eq!(punten(&t, a, 2), Some(2));
        assert_eq!(punten(&t, b, 1), Some(2), "b heeft er niets mee te maken");
    }

    #[test]
    fn boven_het_totaal_komt_er_geen_lijn_meer_bij() {
        let mut t = Tekening::nieuw(true);
        let mut n = 0;
        while n < MAX_LIJNEN {
            let wie = PeerId::new_random();
            for id in 0..MAX_LIJNEN_PER_TEKENAAR as u32 {
                if n == MAX_LIJNEN {
                    break;
                }
                assert!(t.teken(wie, &lijn(id, 0, &[[0, 0], [1, 1]])));
                n += 1;
            }
        }
        assert!(!t.teken(PeerId::new_random(), &lijn(1, 0, &[[0, 0], [1, 1]])));
        assert_eq!(t.lijnen().len(), MAX_LIJNEN);
    }

    #[test]
    fn ongedaan_en_wissen_raken_alleen_de_eigen_lijnen() {
        let a = PeerId::new_random();
        let b = PeerId::new_random();
        let mut t = Tekening::nieuw(true);
        t.teken(a, &lijn(1, 0, &[[0, 0], [1, 1]]));
        t.teken(b, &lijn(1, 0, &[[0, 0], [1, 1]]));
        t.teken(a, &lijn(2, 0, &[[0, 0], [1, 1]]));

        assert!(t.ongedaan(a));
        assert_eq!(punten(&t, a, 2), None, "de laatste van a");
        assert_eq!(punten(&t, a, 1), Some(2));
        assert!(t.wis_van(a));
        assert!(!t.wis_van(a), "niets meer");
        assert!(!t.ongedaan(a));
        assert_eq!(t.lijnen().len(), 1, "b blijft staan");
        assert!(t.wis());
        assert!(t.is_leeg());
    }

    #[test]
    fn een_toestand_overleeft_de_heen_en_terugweg_en_wordt_gecontroleerd() {
        let a = PeerId::new_random();
        let mut t = Tekening::nieuw(true);
        t.teken(a, &lijn(1, 0, &[[0, 0], [1, 1]]));
        t.teken(a, &lijn(2, 0, &[[5, 5], [6, 6], [7, 7]]));
        let terug = Tekening::uit_toestand(true, &t.als_toestand(1));
        assert_eq!(terug, t);

        // Wat een deler nooit zou sturen, komt er ook bij de kijker niet in.
        let mut s = t.als_toestand(1);
        s.lines[0].color = 99;
        s.lines[1].points = vec![[1, 1]; MAX_PUNTEN_PER_LIJN + 10];
        let terug = Tekening::uit_toestand(true, &s);
        assert_eq!(terug.lijnen().len(), 1);
        assert_eq!(terug.lijnen()[0].punten.len(), MAX_PUNTEN_PER_LIJN);

        assert!(Tekening::uit_toestand(false, &t.als_toestand(1)).is_leeg());
    }

    #[test]
    fn de_grootste_toestand_past_in_een_control_frame() {
        // `fitcom-net` weigert frames boven 2 MiB (`framing::MAX_FRAME_LEN`). Een volle
        // tekening moet daar ruim onder blijven, anders sloopt hij de verbinding.
        let mut t = Tekening::nieuw(true);
        let vol = vec![[65535u16, 65535]; MAX_PUNTEN_PER_LIJN];
        'vol: loop {
            let wie = PeerId::new_random();
            for id in 0..MAX_LIJNEN_PER_TEKENAAR as u32 {
                if t.lijnen().len() == MAX_LIJNEN {
                    break 'vol;
                }
                t.voeg_toe(wie, u32::MAX - id, 0, 0, 0, &vol);
            }
        }
        let bytes = crate::ControlMsg::DrawState(t.als_toestand(u32::MAX))
            .encode()
            .unwrap();
        assert!(bytes.len() < 1536 * 1024, "{} bytes", bytes.len());
    }
}
