//! Opus-codec met de instellingen die voor spraak in een klein gezelschap kloppen.

use anyhow::{Context, Result};
use opus::{Application, Channels};

use crate::mix::{FRAME_SAMPLES, SAMPLE_RATE};

/// 32 kbit/s mono is voor spraak ruim voldoende. Hoger zou bij 1 Gbit niets kosten,
/// maar levert hoorbaar niets op en maakt de pakketten alleen groter.
const BITRATE: i32 = 32_000;
/// Opus mag hiermee redundante informatie meesturen zodat één verloren pakket te
/// herstellen is. Kost enkele procenten bitrate en scheelt hoorbaar op een hik.
const VERWACHT_VERLIES: i32 = 5;

/// De grootste payload die een mediapakket draagt. Opus houdt zich aan de buffer die hij
/// krijgt, dus dit is ook het plafond per frame: 1100 bytes per 20 ms is 440 kbit/s.
pub const MAX_PAYLOAD: usize = fitcom_proto::MAX_MEDIA_PAYLOAD;

/// Spelgeluid en muziek zijn geen spraak: Opus' spraakmodel knijpt daar de hoge tonen
/// uit en laat percussie rammelen. Stereo, het audio-model en ruim boven de grens waar
/// Opus in luistertests niet meer van het origineel te onderscheiden is (~256 kbit/s
/// stereo); 384 past met marge in één pakket. Echt verliesvrij (PCM, 1,5 Mbit/s) zou
/// fragmenteren en zonder verliesverberging (Opus' PLC) een tik per verloren pakket geven, en
/// Spotify zelf levert meestal al gecomprimeerd aan — de laatste paar procent is hoorbaar
/// slechter, niet beter. 0,4 Mbit/s per luisteraar kost bij 1 Gbit niets.
const BUREAUBLAD_BITRATE: i32 = 384_000;

pub struct Encoder {
    inner: opus::Encoder,
    kanalen: usize,
}

impl Encoder {
    pub fn new() -> Result<Self> {
        let mut inner = opus::Encoder::new(SAMPLE_RATE, Channels::Mono, Application::Voip)
            .context("Opus-encoder aanmaken")?;
        inner.set_bitrate(opus::Bitrate::Bits(BITRATE))?;
        inner.set_inband_fec(true)?;
        inner.set_packet_loss_perc(VERWACHT_VERLIES)?;
        Ok(Self { inner, kanalen: 1 })
    }

    /// Voor bureaubladgeluid en Spotify: muziek en spelgeluid in stereo in plaats van
    /// spraak. Een oudere peer met een monodecoder kan dit nog steeds decoderen — Opus
    /// mengt dan zelf terug naar mono.
    pub fn voor_muziek() -> Result<Self> {
        let mut inner = opus::Encoder::new(SAMPLE_RATE, Channels::Stereo, Application::Audio)
            .context("Opus-encoder aanmaken")?;
        inner.set_bitrate(opus::Bitrate::Bits(BUREAUBLAD_BITRATE))?;
        inner.set_inband_fec(true)?;
        inner.set_packet_loss_perc(VERWACHT_VERLIES)?;
        Ok(Self { inner, kanalen: 2 })
    }

    /// Codeert precies één frame: [`FRAME_SAMPLES`] samples per kanaal, verweven.
    pub fn encode(&mut self, frame: &[i16], uit: &mut [u8]) -> Result<usize> {
        debug_assert_eq!(frame.len(), FRAME_SAMPLES * self.kanalen);
        Ok(self.inner.encode(frame, uit)?)
    }
}

/// Altijd stereo: een stereodecoder speelt een monopakket (een stem) af op beide kanalen,
/// dus één soort decoder voor elke bron en de mixer hoeft niet te weten wat er binnenkomt.
/// `uit` is verweven en moet plaats hebben voor [`FRAME_SAMPLES`] × 2; het antwoord is het
/// aantal samples *per kanaal*.
pub struct Decoder {
    inner: opus::Decoder,
}

impl Decoder {
    pub fn new() -> Result<Self> {
        Ok(Self {
            inner: opus::Decoder::new(SAMPLE_RATE, Channels::Stereo).context("Opus-decoder")?,
        })
    }

    pub fn decode(&mut self, payload: &[u8], uit: &mut [i16]) -> Result<usize> {
        Ok(self.inner.decode(payload, uit, false)?)
    }

    /// Verzint een frame na verlies. Klinkt hoorbaar beter dan een gat: Opus zet de
    /// laatste klank uitdovend voort in plaats van abrupt naar stilte te springen.
    pub fn verberg_verlies(&mut self, uit: &mut [i16]) -> Result<usize> {
        Ok(self.inner.decode(&[], uit, false)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Een toon van 440 Hz, in het bereik van `i16`.
    fn toon(n: usize) -> Vec<i16> {
        (0..n)
            .map(|i| {
                let t = i as f32 / SAMPLE_RATE as f32;
                ((t * 440.0 * std::f32::consts::TAU).sin() * 8000.0) as i16
            })
            .collect()
    }

    #[test]
    fn coderen_en_decoderen_levert_herkenbaar_geluid() {
        let mut enc = Encoder::new().unwrap();
        let mut dec = Decoder::new().unwrap();
        let bron = toon(FRAME_SAMPLES);

        let mut pakket = [0u8; MAX_PAYLOAD];
        let n = enc.encode(&bron, &mut pakket).unwrap();
        assert!(
            n > 0 && n < MAX_PAYLOAD,
            "pakketgrootte {n} is onwaarschijnlijk"
        );

        let mut uit = vec![0i16; FRAME_SAMPLES * 2];
        let samples = dec.decode(&pakket[..n], &mut uit).unwrap();
        assert_eq!(samples, FRAME_SAMPLES);

        // Opus is lossy, dus we vergelijken energie en niet sample voor sample.
        let energie: f64 = uit.iter().map(|&s| f64::from(s).powi(2)).sum();
        assert!(energie > 0.0, "decoder leverde stilte");
    }

    #[test]
    fn verlies_verbergen_levert_een_volledig_frame() {
        let mut enc = Encoder::new().unwrap();
        let mut dec = Decoder::new().unwrap();
        let mut pakket = [0u8; MAX_PAYLOAD];
        let mut uit = vec![0i16; FRAME_SAMPLES * 2];

        // Eerst iets echts, anders heeft de concealment niets om op voort te borduren.
        let n = enc.encode(&toon(FRAME_SAMPLES), &mut pakket).unwrap();
        dec.decode(&pakket[..n], &mut uit).unwrap();

        let hersteld = dec.verberg_verlies(&mut uit).unwrap();
        assert_eq!(hersteld, FRAME_SAMPLES, "moet een heel frame opleveren");
    }

    #[test]
    fn stilte_levert_een_klein_pakket() {
        // Opus hoort bij stilte bijna niets te versturen. Zou dit groot zijn, dan
        // sturen we onnodig verkeer terwijl er niemand praat.
        let mut enc = Encoder::new().unwrap();
        let mut pakket = [0u8; MAX_PAYLOAD];
        let n = enc.encode(&vec![0i16; FRAME_SAMPLES], &mut pakket).unwrap();
        assert!(n < 30, "stiltepakket van {n} bytes is onverwacht groot");
    }

    #[test]
    fn stereo_blijft_stereo() {
        // Links een toon, rechts stilte: komt dat er na Opus nog zo uit, dan is de keten
        // echt stereo en niet stilletjes gemiddeld.
        let mut enc = Encoder::voor_muziek().unwrap();
        let mut dec = Decoder::new().unwrap();
        let links = toon(FRAME_SAMPLES * 5);
        let mut pakket = [0u8; MAX_PAYLOAD];
        let mut uit = vec![0i16; FRAME_SAMPLES * 2];
        for blok in links.chunks(FRAME_SAMPLES) {
            let verweven: Vec<i16> = blok.iter().flat_map(|&s| [s, 0]).collect();
            let n = enc.encode(&verweven, &mut pakket).unwrap();
            dec.decode(&pakket[..n], &mut uit).unwrap();
        }
        let energie = |k: usize| -> f64 {
            uit.iter()
                .skip(k)
                .step_by(2)
                .map(|&s| f64::from(s).powi(2))
                .sum()
        };
        assert!(
            energie(0) > 100.0 * energie(1).max(1.0),
            "links {} rechts {}",
            energie(0),
            energie(1)
        );
    }

    #[test]
    fn stereodecoder_speelt_een_stem_op_beide_kanalen() {
        let mut enc = Encoder::new().unwrap();
        let mut dec = Decoder::new().unwrap();
        let mut pakket = [0u8; MAX_PAYLOAD];
        let mut uit = vec![0i16; FRAME_SAMPLES * 2];
        for blok in toon(FRAME_SAMPLES * 3).chunks(FRAME_SAMPLES) {
            let n = enc.encode(blok, &mut pakket).unwrap();
            assert_eq!(dec.decode(&pakket[..n], &mut uit).unwrap(), FRAME_SAMPLES);
        }
        assert!(uit.chunks(2).all(|lr| lr[0] == lr[1]));
        assert!(uit.iter().any(|&s| s != 0));
    }
}
