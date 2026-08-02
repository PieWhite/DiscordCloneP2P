//! Lokaal mixen en formaat-omzettingen.
//!
//! Er is geen mix-server: iedereen stuurt zijn eigen stem naar iedereen, en elke
//! ontvanger telt op wat er binnenkomt. Bij drie deelnemers is dat triviaal werk en
//! het scheelt een centrale partij — wat het hele punt van deze app is.

/// 48 kHz is wat Opus intern gebruikt en wat vrijwel elk apparaat aankan.
pub const SAMPLE_RATE: u32 = 48_000;
/// 20 ms. De afweging tussen overhead per pakket en vertraging; dit is wat vrijwel
/// alle spraakdiensten gebruiken.
pub const FRAME_SAMPLES: usize = 960;

/// Telt de sprekers bij elkaar op met hun eigen volume.
///
/// In `f32` om ruimte te houden: drie mensen die tegelijk hard praten lopen in `i16`
/// tegen het plafond, en dan krijg je vervorming in plaats van hard geluid.
pub fn mix(sprekers: &[(&[i16], f32)], uit: &mut [i32]) {
    uit.iter_mut().for_each(|s| *s = 0);
    for (samples, volume) in sprekers {
        for (doel, &sample) in uit.iter_mut().zip(samples.iter()) {
            *doel += (f32::from(sample) * volume) as i32;
        }
    }
}

/// Begrenst naar `i16`. Klemmen is lelijk maar hoorbaar beter dan omklappen, wat je
/// bij een simpele cast zou krijgen.
pub fn klem(bron: &[i32], uit: &mut [i16]) {
    for (doel, &s) in uit.iter_mut().zip(bron.iter()) {
        *doel = s.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
    }
}

/// Mixen en begrenzen in één keer, voor het gebruikelijke geval.
pub fn mix_naar_i16(sprekers: &[(&[i16], f32)], uit: &mut [i16]) {
    let mut ruimte = vec![0i32; uit.len()];
    mix(sprekers, &mut ruimte);
    klem(&ruimte, uit);
}

/// Meerdere kanalen samenvoegen tot één. Een headsetmicrofoon levert soms stereo
/// terwijl er maar één capsule in zit.
pub fn naar_mono(verweven: &[f32], kanalen: usize, uit: &mut Vec<f32>) {
    uit.clear();
    if kanalen <= 1 {
        uit.extend_from_slice(verweven);
        return;
    }
    let deler = kanalen as f32;
    for blok in verweven.chunks_exact(kanalen) {
        uit.push(blok.iter().sum::<f32>() / deler);
    }
}

/// Eén monokanaal uitsmeren over meerdere uitvoerkanalen.
pub fn naar_kanalen(mono: &[i16], kanalen: usize, uit: &mut Vec<i16>) {
    uit.clear();
    for &s in mono {
        for _ in 0..kanalen {
            uit.push(s);
        }
    }
}

/// Lineaire herbemonstering tussen twee samplerates.
///
/// Alleen nodig als Windows het apparaat niet op 48 kHz heeft staan. Lineair is niet
/// het mooiste dat er bestaat, maar voor spraak nauwelijks hoorbaar, en het kost geen
/// extra vertraging of afhankelijkheid. Wie het beste wil, zet zijn apparaat in
/// Windows op 48000 Hz — dan wordt deze functie helemaal niet gebruikt.
pub struct Resampler {
    van: u32,
    naar: u32,
    /// Positie tussen twee invoersamples, blijft staan tussen aanroepen zodat er op
    /// de grens van een blok geen klik ontstaat.
    positie: f64,
    laatste: f32,
}

impl Resampler {
    pub fn new(van: u32, naar: u32) -> Self {
        Self {
            van,
            naar,
            positie: 0.0,
            laatste: 0.0,
        }
    }

    fn nodig(&self) -> bool {
        self.van != self.naar
    }

    pub fn verwerk(&mut self, invoer: &[f32], uit: &mut Vec<f32>) {
        if !self.nodig() {
            uit.extend_from_slice(invoer);
            return;
        }
        if invoer.is_empty() {
            return;
        }

        let stap = self.van as f64 / self.naar as f64;
        while self.positie < invoer.len() as f64 {
            let i = self.positie.floor() as usize;
            let frac = (self.positie - i as f64) as f32;
            // Het sample vóór index 0 komt uit het vorige blok.
            let a = if i == 0 { self.laatste } else { invoer[i - 1] };
            let b = invoer[i];
            uit.push(a + (b - a) * frac);
            self.positie += stap;
        }

        self.laatste = *invoer.last().unwrap();
        self.positie -= invoer.len() as f64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixen_telt_op() {
        let a = [100i16, 200, 300];
        let b = [10i16, 20, 30];
        let mut uit = [0i16; 3];
        mix_naar_i16(&[(&a[..], 1.0), (&b[..], 1.0)], &mut uit);
        assert_eq!(uit, [110, 220, 330]);
    }

    #[test]
    fn volume_per_spreker_werkt_los_van_elkaar() {
        let a = [100i16, 100];
        let b = [100i16, 100];
        let mut uit = [0i16; 2];
        mix_naar_i16(&[(&a[..], 0.5), (&b[..], 0.0)], &mut uit);
        assert_eq!(uit, [50, 50], "iemand op nul mag niets bijdragen");
    }

    #[test]
    fn hard_geluid_klemt_in_plaats_van_om_te_klappen() {
        // Dit is het geval waar het misgaat als je in i16 zou optellen: drie mensen
        // die tegelijk hard praten. Omklappen klinkt als een explosie.
        let luid = [30_000i16; 4];
        let mut uit = [0i16; 4];
        mix_naar_i16(
            &[(&luid[..], 1.0), (&luid[..], 1.0), (&luid[..], 1.0)],
            &mut uit,
        );
        assert_eq!(uit, [i16::MAX; 4]);
    }

    #[test]
    fn negatief_klemt_ook() {
        let luid = [-30_000i16; 2];
        let mut uit = [0i16; 2];
        mix_naar_i16(&[(&luid[..], 1.0), (&luid[..], 1.0)], &mut uit);
        assert_eq!(uit, [i16::MIN; 2]);
    }

    #[test]
    fn zonder_sprekers_is_het_stil() {
        let mut uit = [123i16; 3];
        mix_naar_i16(&[], &mut uit);
        assert_eq!(uit, [0, 0, 0]);
    }

    #[test]
    fn kortere_spreker_loopt_niet_buiten_de_buffer() {
        let kort = [100i16, 100];
        let mut uit = [0i16; 5];
        mix_naar_i16(&[(&kort[..], 1.0)], &mut uit);
        assert_eq!(uit, [100, 100, 0, 0, 0]);
    }

    #[test]
    fn stereo_wordt_gemiddeld_naar_mono() {
        let stereo = [1.0f32, 3.0, 10.0, 20.0];
        let mut mono = Vec::new();
        naar_mono(&stereo, 2, &mut mono);
        assert_eq!(mono, [2.0, 15.0]);
    }

    #[test]
    fn mono_wordt_over_de_kanalen_verdeeld() {
        let mono = [5i16, 6];
        let mut uit = Vec::new();
        naar_kanalen(&mono, 2, &mut uit);
        assert_eq!(uit, [5, 5, 6, 6]);
    }

    #[test]
    fn resampler_doet_niets_bij_gelijke_rate() {
        let mut r = Resampler::new(48_000, 48_000);
        assert!(!r.nodig());
        let mut uit = Vec::new();
        r.verwerk(&[1.0, 2.0, 3.0], &mut uit);
        assert_eq!(uit, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn resampler_levert_ongeveer_de_juiste_hoeveelheid() {
        // 44100 -> 48000 hoort ruwweg 8,8% meer samples op te leveren.
        let mut r = Resampler::new(44_100, 48_000);
        let invoer: Vec<f32> = (0..441).map(|i| i as f32).collect();
        let mut uit = Vec::new();
        r.verwerk(&invoer, &mut uit);
        let verwacht = 480;
        assert!(
            (uit.len() as i32 - verwacht).abs() <= 2,
            "kreeg {} samples, verwachtte ongeveer {verwacht}",
            uit.len()
        );
    }

    #[test]
    fn resampler_houdt_de_verhouding_over_meerdere_blokken() {
        // Zonder het bewaren van de positie tussen blokken loopt de telling scheef
        // en hoor je na een minuut een klik of een gat.
        let mut r = Resampler::new(44_100, 48_000);
        let blok: Vec<f32> = (0..441).map(|i| i as f32).collect();
        let mut totaal = 0usize;
        for _ in 0..100 {
            let mut uit = Vec::new();
            r.verwerk(&blok, &mut uit);
            totaal += uit.len();
        }
        let verwacht = 48_000;
        assert!(
            (totaal as i32 - verwacht).abs() <= 5,
            "na 100 blokken {totaal} samples, verwachtte ongeveer {verwacht}"
        );
    }
}
