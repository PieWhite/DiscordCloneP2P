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

/// Ruim boven wat een 20 ms mono-frame ooit oplevert.
pub const MAX_PAYLOAD: usize = 512;

/// Spelgeluid en muziek zijn geen spraak: Opus' spraakmodel knijpt daar de hoge tonen
/// uit en laat percussie rammelen. Meer bits en het audio-model lossen dat op, en bij
/// 1 Gbit kost dat niets.
const BUREAUBLAD_BITRATE: i32 = 96_000;

pub struct Encoder {
    inner: opus::Encoder,
}

impl Encoder {
    pub fn new() -> Result<Self> {
        let mut inner = opus::Encoder::new(SAMPLE_RATE, Channels::Mono, Application::Voip)
            .context("Opus-encoder aanmaken")?;
        inner.set_bitrate(opus::Bitrate::Bits(BITRATE))?;
        inner.set_inband_fec(true)?;
        inner.set_packet_loss_perc(VERWACHT_VERLIES)?;
        Ok(Self { inner })
    }

    /// Voor bureaubladgeluid: muziek en spelgeluid in plaats van spraak.
    pub fn voor_muziek() -> Result<Self> {
        let mut inner = opus::Encoder::new(SAMPLE_RATE, Channels::Mono, Application::Audio)
            .context("Opus-encoder aanmaken")?;
        inner.set_bitrate(opus::Bitrate::Bits(BUREAUBLAD_BITRATE))?;
        inner.set_inband_fec(true)?;
        inner.set_packet_loss_perc(VERWACHT_VERLIES)?;
        Ok(Self { inner })
    }

    /// Codeert precies één frame van [`FRAME_SAMPLES`] samples.
    pub fn encode(&mut self, frame: &[i16], uit: &mut [u8]) -> Result<usize> {
        debug_assert_eq!(frame.len(), FRAME_SAMPLES);
        Ok(self.inner.encode(frame, uit)?)
    }
}

pub struct Decoder {
    inner: opus::Decoder,
}

impl Decoder {
    pub fn new() -> Result<Self> {
        Ok(Self {
            inner: opus::Decoder::new(SAMPLE_RATE, Channels::Mono).context("Opus-decoder")?,
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

        let mut uit = vec![0i16; FRAME_SAMPLES];
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
        let mut uit = vec![0i16; FRAME_SAMPLES];

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
}
