//! Jitterbuffer per spreker.
//!
//! Pakketten komen over UDP binnen: door elkaar, te laat, of helemaal niet. De
//! geluidskaart daarentegen vraagt onverbiddelijk elke 20 ms om een frame. Deze buffer
//! zit daartussen.
//!
//! # Waarom er geen klok in zit
//!
//! De buffer telt frames, geen milliseconden. De weergave wordt aangedreven door de
//! geluidskaart, dus "één frame" *is* de tijdseenheid. Daardoor is het gedrag bij
//! verlies, herordening en te laat aankomen exact te testen zonder te wachten of te
//! slapen — en drift tussen de klok van de zender en die van de ontvanger komt vanzelf
//! naar boven als groeiende of leeglopende buffer, precies waar we op reageren.

use std::collections::BTreeMap;

/// Minimale buffer. Twee frames = 40 ms speling; daaronder klapt het bij de kleinste hik.
pub const MIN_DIEPTE: usize = 2;
/// Bovengrens. Meer dan dit is geen jitter meer maar een structureel probleem, en de
/// vertraging zou merkbaar worden.
pub const MAX_DIEPTE: usize = 10;
/// Zoveel frames boven de streefdiepte gooien we weg om opgelopen vertraging in te lopen.
const OVERLOOP: usize = MAX_DIEPTE + 4;

#[derive(Debug, PartialEq, Eq)]
pub enum Frame {
    /// Een pakket dat op zijn beurt is.
    Data(Vec<u8>),
    /// Er hoorde hier iets te zijn maar het is er niet. De decoder maakt er iets van
    /// dat minder opvalt dan een gat (packet loss concealment).
    Verloren,
    /// Niets te spelen: de spreker is stil of nog aan het opbouwen.
    Stilte,
}

pub struct JitterBuffer {
    frames: BTreeMap<u32, Vec<u8>>,
    /// Het volgnummer dat we nu horen te spelen. `None` = nog niet begonnen.
    volgende: Option<u32>,
    streefdiepte: usize,
    /// Verlies en te-laat sinds de laatste aanpassing van de streefdiepte.
    misser_teller: u32,
    schoon_teller: u32,
    pub verloren_totaal: u64,
    pub te_laat_totaal: u64,
}

impl Default for JitterBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl JitterBuffer {
    pub fn new() -> Self {
        Self {
            frames: BTreeMap::new(),
            volgende: None,
            streefdiepte: MIN_DIEPTE,
            misser_teller: 0,
            schoon_teller: 0,
            verloren_totaal: 0,
            te_laat_totaal: 0,
        }
    }

    pub fn aantal(&self) -> usize {
        self.frames.len()
    }

    pub fn streefdiepte(&self) -> usize {
        self.streefdiepte
    }

    pub fn push(&mut self, seq: u32, payload: Vec<u8>) {
        // Al voorbij: dit pakket komt te laat om nog iets mee te kunnen. Wel meetellen,
        // want structureel te laat betekent dat de buffer dieper moet.
        //
        // Modulair vergelijken en niet met `<`: `volgende` loopt met `wrapping_add` door,
        // dus vlak na de omloop van de 32-bits teller staat hij lager dan alles wat er nog
        // binnenkomt. Met een gewone `<` gold dan élk pakket als te laat en viel de spreker
        // stil (B-38).
        if let Some(v) = self.volgende {
            if seq.wrapping_sub(v) > u32::MAX / 2 {
                self.te_laat_totaal += 1;
                self.misser_teller += 1;
                return;
            }
        }
        self.frames.insert(seq, payload);
    }

    /// Bewust iteratief en niet recursief: dit draait op de audio-thread, en een
    /// tak die zichzelf aanroept zonder de buffer te verkleinen loopt daar de stack
    /// over in plaats van een hoorbaar hapertje te geven.
    pub fn pop(&mut self) -> Frame {
        let Some(&eerste) = self.frames.keys().next() else {
            // Niets in huis. De volgende keer beginnen we opnieuw met opbouwen,
            // anders zouden we na een stilte meteen weer happen naar lucht.
            self.volgende = None;
            return Frame::Stilte;
        };

        // Opbouwen: pas beginnen als er genoeg speling is. Zonder dit hoor je bij de
        // eerste de beste vertraging meteen een gat.
        let volgende = match self.volgende {
            Some(v) => v,
            None => {
                if self.frames.len() < self.streefdiepte {
                    return Frame::Stilte;
                }
                self.volgende = Some(eerste);
                eerste
            }
        };

        // Te ver opgelopen: de zender loopt voor of we hebben te lang gebufferd.
        // Het oudste weggooien kost een hoorbare hapering, maar structureel een halve
        // seconde vertraging is in een gesprek erger.
        if self.frames.len() > OVERLOOP {
            let weg = self.frames.len() - self.streefdiepte;
            tracing::debug!(aantal = self.frames.len(), weg, "jitterbuffer loopt over");
            for _ in 0..weg {
                let oudste = *self.frames.keys().next().expect("buffer is niet leeg");
                self.frames.remove(&oudste);
            }
            let nu = *self
                .frames
                .keys()
                .next()
                .expect("streefdiepte is minstens 2");
            let payload = self.frames.remove(&nu).expect("net opgezocht");
            self.volgende = Some(nu.wrapping_add(1));
            return Frame::Data(payload);
        }

        if let Some(payload) = self.frames.remove(&volgende) {
            self.volgende = Some(volgende.wrapping_add(1));
            self.schoon_teller += 1;
            self.pas_diepte_aan();
            return Frame::Data(payload);
        }

        // Het gat kan nog gevuld worden zolang we speling over hebben. Pas als de
        // buffer diep genoeg is geven we hem op.
        if self.frames.len() < self.streefdiepte {
            return Frame::Stilte;
        }

        self.volgende = Some(volgende.wrapping_add(1));
        self.verloren_totaal += 1;
        self.misser_teller += 1;
        self.pas_diepte_aan();
        Frame::Verloren
    }

    /// Groeit snel bij problemen, krimpt langzaam bij rust.
    ///
    /// Andersom zou de buffer bij elke hik terugvallen en meteen weer moeten opbouwen,
    /// wat je hoort als herhaalde onderbrekingen in plaats van één keer wat extra
    /// vertraging.
    fn pas_diepte_aan(&mut self) {
        if self.misser_teller >= 2 {
            self.streefdiepte = (self.streefdiepte + 1).min(MAX_DIEPTE);
            self.misser_teller = 0;
            self.schoon_teller = 0;
            return;
        }
        // 500 frames = 10 seconden zonder problemen.
        if self.schoon_teller >= 500 {
            self.streefdiepte = self.streefdiepte.saturating_sub(1).max(MIN_DIEPTE);
            self.schoon_teller = 0;
            self.misser_teller = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vul(b: &mut JitterBuffer, seqs: &[u32]) {
        for &s in seqs {
            b.push(s, vec![s as u8]);
        }
    }

    #[test]
    fn speelt_pas_af_na_opbouwen() {
        let mut b = JitterBuffer::new();
        b.push(1, vec![1]);
        assert_eq!(b.pop(), Frame::Stilte, "één frame is te weinig speling");
        b.push(2, vec![2]);
        assert_eq!(b.pop(), Frame::Data(vec![1]));
    }

    #[test]
    fn herstelt_de_volgorde() {
        let mut b = JitterBuffer::new();
        vul(&mut b, &[3, 1, 2]);
        assert_eq!(b.pop(), Frame::Data(vec![1]));
        assert_eq!(b.pop(), Frame::Data(vec![2]));
        assert_eq!(b.pop(), Frame::Data(vec![3]));
    }

    #[test]
    fn meldt_verlies_zodat_de_decoder_het_kan_verbergen() {
        let mut b = JitterBuffer::new();
        vul(&mut b, &[1, 2, 4, 5]); // 3 raakt kwijt
        assert_eq!(b.pop(), Frame::Data(vec![1]));
        assert_eq!(b.pop(), Frame::Data(vec![2]));
        assert_eq!(b.pop(), Frame::Verloren);
        assert_eq!(b.pop(), Frame::Data(vec![4]));
        assert_eq!(b.verloren_totaal, 1);
    }

    #[test]
    fn wacht_op_een_nakomer_voordat_hij_hem_opgeeft() {
        let mut b = JitterBuffer::new();
        vul(&mut b, &[1, 2]);
        assert_eq!(b.pop(), Frame::Data(vec![1]));
        // 2 is weg uit de buffer; 3 ontbreekt nog maar kan nog komen.
        assert_eq!(b.pop(), Frame::Data(vec![2]));
        assert_eq!(b.pop(), Frame::Stilte, "nog niet opgeven");
        b.push(3, vec![3]);
        b.push(4, vec![4]);
        assert_eq!(b.pop(), Frame::Data(vec![3]));
    }

    #[test]
    fn een_pakket_dat_te_laat_is_wordt_niet_alsnog_gespeeld() {
        let mut b = JitterBuffer::new();
        vul(&mut b, &[1, 2, 3]);
        assert_eq!(b.pop(), Frame::Data(vec![1]));
        assert_eq!(b.pop(), Frame::Data(vec![2]));
        b.push(1, vec![99]); // arriveert alsnog, veel te laat
        assert_eq!(b.te_laat_totaal, 1);
        assert_eq!(
            b.pop(),
            Frame::Data(vec![3]),
            "de volgorde mag niet omvallen"
        );
    }

    #[test]
    fn buffer_wordt_dieper_na_herhaald_verlies() {
        let mut b = JitterBuffer::new();
        let begin = b.streefdiepte();
        // Twee gaten achter elkaar.
        vul(&mut b, &[1, 2, 4, 6, 7, 8]);
        for _ in 0..6 {
            b.pop();
        }
        assert!(
            b.streefdiepte() > begin,
            "streefdiepte moet meegroeien met de problemen"
        );
    }

    #[test]
    fn buffer_krimpt_weer_als_alles_goed_gaat() {
        let mut b = JitterBuffer::new();
        // Twee gaten: één is niet genoeg om de buffer te laten groeien, en dan meet
        // deze test niets.
        vul(&mut b, &[1, 2, 4, 6, 7, 8]);
        for _ in 0..8 {
            b.pop();
        }
        let diep = b.streefdiepte();
        assert!(diep > MIN_DIEPTE, "eerst moet hij gegroeid zijn");

        // Lange schone periode, aansluitend op wat er al lag: een nieuw gat zou de
        // teller weer resetten.
        for s in 9..600u32 {
            b.push(s, vec![0]);
            b.pop();
        }
        assert!(b.streefdiepte() < diep, "moet weer krimpen bij rust");
    }

    #[test]
    fn spoelt_vooruit_als_de_vertraging_oploopt() {
        // Zender loopt voor of de ontvanger heeft gehaperd: er stapelt zich veel op.
        // Blijven afspelen zou een halve seconde vertraging inbouwen die nooit meer
        // weggaat, en dat is in een gesprek erger dan één hapering.
        let mut b = JitterBuffer::new();
        for s in 1..=(OVERLOOP as u32 + 5) {
            b.push(s, vec![s as u8]);
        }
        let voor = b.aantal();

        match b.pop() {
            Frame::Data(p) => assert!(
                p[0] > 1,
                "moet oude frames overgeslagen hebben, kreeg {}",
                p[0]
            ),
            andere => panic!("verwachtte geluid, kreeg {andere:?}"),
        }

        assert!(
            b.aantal() < voor - 1,
            "meer dan één frame moet gesneuveld zijn"
        );
        assert!(
            b.aantal() <= MAX_DIEPTE,
            "moet terug binnen de grenzen zijn"
        );
    }

    #[test]
    fn vooruitspoelen_loopt_niet_vast() {
        // Deze tak riep zichzelf eerder aan zonder de buffer te verkleinen. Op de
        // audio-thread is dat geen hapering maar een stack overflow.
        let mut b = JitterBuffer::new();
        for s in 1..=500u32 {
            b.push(s, vec![0]);
        }
        for _ in 0..600 {
            b.pop();
        }
        assert!(b.aantal() <= MAX_DIEPTE);
    }

    #[test]
    fn b38_de_volgnummers_mogen_omlopen() {
        // `volgende + 1` overflowde op `u32::MAX`. De mixthread wordt één keer gestart en
        // nooit herstart, dus die paniek maakte *alle* audio permanent stil terwijl de UI
        // een gezonde sessie bleef tonen. Twee pakketten waren genoeg.
        let mut b = JitterBuffer::new();
        vul(&mut b, &[u32::MAX - 1, u32::MAX]);
        assert_eq!(b.pop(), Frame::Data(vec![(u32::MAX - 1) as u8]));
        assert_eq!(b.pop(), Frame::Data(vec![u32::MAX as u8]));

        // `volgende` staat nu op 0: de teller is omgelopen. Wat daarna komt is nieuw en
        // niet iets van vroeger — met een gewone `<`-vergelijking gold hier élk pakket
        // als te laat en viel de spreker stil.
        b.push(0, vec![10]);
        b.push(1, vec![11]);
        assert_eq!(b.te_laat_totaal, 0, "0 is na de omloop het eerstvolgende");
        assert_eq!(b.pop(), Frame::Data(vec![10]));
        assert_eq!(b.pop(), Frame::Data(vec![11]));
    }

    #[test]
    fn stilte_zet_de_opbouw_terug() {
        let mut b = JitterBuffer::new();
        vul(&mut b, &[1, 2]);
        assert_eq!(b.pop(), Frame::Data(vec![1]));
        assert_eq!(b.pop(), Frame::Data(vec![2]));
        assert_eq!(b.pop(), Frame::Stilte);
        assert_eq!(b.pop(), Frame::Stilte);

        // Spreker begint opnieuw met een heel ander volgnummer (nieuwe sessie).
        b.push(500, vec![50]);
        assert_eq!(b.pop(), Frame::Stilte, "eerst weer opbouwen");
        b.push(501, vec![51]);
        assert_eq!(b.pop(), Frame::Data(vec![50]));
    }
}
