//! Korte tonen bij het komen en gaan van anderen, en bij een stream die aan of uit gaat.
//!
//! # Waarom de tonen hier gemaakt worden en niet meegeleverd
//!
//! Een wav-bestand naast de exe zou "losse exe in een zip" breken, en hem in de binary
//! bakken zou zes bestandjes in de repo betekenen die niemand kan nalezen of aanpassen.
//! Zes sinustonen met een fade zijn veertig regels rekenwerk, gebeuren één keer bij het
//! eerste gebruik, en staan hieronder als de noten waar ze uit bestaan.
//!
//! # Waarom niet via de voice-uitvoer
//!
//! Die bestaat alleen tijdens een gesprek, en het eerste geluidje dat je wilt horen is
//! precies dat van je eigen deelname. Dus langs de mixer heen, rechtstreeks naar het
//! standaardapparaat van het systeem: op Windows `PlaySound` met de bytes in het geheugen,
//! op macOS `afplay`. Zelfde afweging als bij `notify.rs`: nul afhankelijkheden.
//!
//! Het volume volgt de systeemmixer, dus wie ze te hard vindt zet de app in de
//! volumemixer van Windows zachter. Niet-storen onderdrukt ze; dat besluit staat in
//! `engine.rs::geluid`, want alleen de motor weet in welke stand hij staat.

use std::sync::OnceLock;

/// 24 kHz is voor een piepje van twee tonen ruim: de hoogste toon hieronder zit op
/// 1,3 kHz, een factor negen onder Nyquist.
const SAMPLERATE: u32 = 24_000;

/// Bewust laag. Deze tonen komen langs terwijl er gegamed wordt; ze moeten opvallen zonder
/// over de game heen te gaan.
const AMPLITUDE: f32 = 0.22;

/// Een sinus die abrupt begint of eindigt klikt. Zes milliseconde in- en uitregelen haalt
/// dat eruit zonder de toon merkbaar korter te maken.
const FADE_MS: u32 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Geluid {
    /// Jij neemt deel aan het gesprek. Oplopend: er komt iets bij.
    EigenJoin,
    /// Jij verlaat het gesprek. Hetzelfde interval, aflopend.
    EigenLeave,
    /// Iemand anders komt erbij. Eén toon, zodat het niet met je eigen deelname te
    /// verwarren is.
    PeerJoin,
    /// Iemand anders gaat eruit.
    PeerLeave,
    /// Iemand zet een scherm of camera aan. Een octaaf hoger dan de stemgeluidjes: het
    /// gaat over iets anders, dus het klinkt ook anders.
    StreamAan,
    /// Iemand zet dat weer uit.
    StreamUit,
}

impl Geluid {
    /// De noten: (frequentie in Hz, duur in ms).
    fn tonen(self) -> &'static [(f32, u32)] {
        match self {
            Self::EigenJoin => &[(587.0, 90), (880.0, 130)],
            Self::EigenLeave => &[(880.0, 90), (587.0, 130)],
            Self::PeerJoin => &[(880.0, 120)],
            Self::PeerLeave => &[(587.0, 120)],
            Self::StreamAan => &[(1046.0, 60), (1318.0, 90)],
            Self::StreamUit => &[(1318.0, 60), (1046.0, 90)],
        }
    }

    fn naam(self) -> &'static str {
        match self {
            Self::EigenJoin => "eigen-join",
            Self::EigenLeave => "eigen-leave",
            Self::PeerJoin => "peer-join",
            Self::PeerLeave => "peer-leave",
            Self::StreamAan => "stream-aan",
            Self::StreamUit => "stream-uit",
        }
    }

    const ALLE: [Self; 6] = [
        Self::EigenJoin,
        Self::EigenLeave,
        Self::PeerJoin,
        Self::PeerLeave,
        Self::StreamAan,
        Self::StreamUit,
    ];

    fn plek(self) -> usize {
        Self::ALLE.iter().position(|g| *g == self).unwrap_or(0)
    }
}

/// De zes wav-bestanden, één keer gemaakt en daarna blijvend. Dat `'static` is geen
/// gemak maar een eis: `PlaySound` met `SND_ASYNC | SND_MEMORY` speelt ná de aanroep nog
/// door en leest dan nog uit deze bytes.
fn wav_van(g: Geluid) -> &'static [u8] {
    static CACHE: OnceLock<Vec<Vec<u8>>> = OnceLock::new();
    &CACHE.get_or_init(|| Geluid::ALLE.iter().map(|g| bouw_wav(g.tonen())).collect())[g.plek()]
}

/// Een compleet wav-bestand (16-bits PCM, mono) met de opgegeven tonen achter elkaar.
fn bouw_wav(tonen: &[(f32, u32)]) -> Vec<u8> {
    let mut samples: Vec<i16> = Vec::new();
    for (hz, ms) in tonen {
        let n = (SAMPLERATE as u64 * u64::from(*ms) / 1000) as usize;
        let fade = ((SAMPLERATE * FADE_MS / 1000) as usize).min(n / 2).max(1);
        for i in 0..n {
            let t = i as f32 / SAMPLERATE as f32;
            let omhoog = (i as f32 / fade as f32).min(1.0);
            let omlaag = ((n - i) as f32 / fade as f32).min(1.0);
            let waarde = (std::f32::consts::TAU * hz * t).sin() * AMPLITUDE * omhoog * omlaag;
            samples.push((waarde * i16::MAX as f32) as i16);
        }
    }

    let data_bytes = (samples.len() * 2) as u32;
    let mut uit = Vec::with_capacity(44 + samples.len() * 2);
    uit.extend_from_slice(b"RIFF");
    uit.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    uit.extend_from_slice(b"WAVEfmt ");
    uit.extend_from_slice(&16u32.to_le_bytes()); // lengte van dit blok
    uit.extend_from_slice(&1u16.to_le_bytes()); // PCM
    uit.extend_from_slice(&1u16.to_le_bytes()); // mono
    uit.extend_from_slice(&SAMPLERATE.to_le_bytes());
    uit.extend_from_slice(&(SAMPLERATE * 2).to_le_bytes()); // bytes per seconde
    uit.extend_from_slice(&2u16.to_le_bytes()); // bytes per blok
    uit.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    uit.extend_from_slice(b"data");
    uit.extend_from_slice(&data_bytes.to_le_bytes());
    for s in samples {
        uit.extend_from_slice(&s.to_le_bytes());
    }
    uit
}

/// Speelt het geluidje. Lukt dat niet, dan is dat geen fout die iemand hoeft te zien:
/// een gesprek zonder piepje werkt nog steeds.
#[cfg(windows)]
pub fn speel(g: Geluid) {
    use windows::core::PCWSTR;
    use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};

    let bytes = wav_van(g);
    // SAFETY: met `SND_MEMORY` is de eerste parameter een verwijzing naar de wav-bytes in
    // plaats van naar een bestandsnaam, en die bytes leven voor de rest van het proces
    // (zie `wav_van`) — wat `SND_ASYNC` vereist. `SND_NODEFAULT` voorkomt dat Windows er
    // een standaardpiep van maakt als er iets niet klopt.
    let ok = unsafe {
        PlaySoundW(
            PCWSTR(bytes.as_ptr() as *const u16),
            None,
            SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
        )
    };
    if !ok.as_bool() {
        tracing::debug!(geluid = g.naam(), "geluidje niet afgespeeld");
    }
}

/// Op macOS via `afplay`, dezelfde soort keuze als de `osascript`-melding in `notify.rs`:
/// nul afhankelijkheden en het werkt zowel gebundeld als als losse binary. `afplay` wil
/// een bestand, dus de bytes gaan één keer naar de tijdelijke map.
#[cfg(target_os = "macos")]
pub fn speel(g: Geluid) {
    let pad = std::env::temp_dir().join(format!("fitcom-{}.wav", g.naam()));
    if !pad.exists() && std::fs::write(&pad, wav_van(g)).is_err() {
        tracing::debug!(geluid = g.naam(), "geluidje niet weg te schrijven");
        return;
    }
    let gestart = std::process::Command::new("afplay")
        .arg(&pad)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    match gestart {
        // Niet op afronden wachten en de zombie niet laten hangen: afplay is klaar binnen
        // een kwart seconde en de motor mag daar niet op staan wachten.
        Ok(mut kind) => {
            std::thread::spawn(move || {
                let _ = kind.wait();
            });
        }
        Err(e) => tracing::debug!(error = %e, geluid = g.naam(), "afplay niet te starten"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lees_u32(b: &[u8], op: usize) -> u32 {
        u32::from_le_bytes(b[op..op + 4].try_into().unwrap())
    }

    /// Een verkeerd samengestelde header levert op Windows geen fout op maar stilte, en
    /// dat is precies het soort bug dat je nooit vindt. Dus hier vastgelegd.
    #[test]
    fn elke_wav_heeft_een_kloppende_header() {
        for g in Geluid::ALLE {
            let w = wav_van(g);
            assert_eq!(&w[0..4], b"RIFF", "{}", g.naam());
            assert_eq!(&w[8..12], b"WAVE", "{}", g.naam());
            assert_eq!(&w[12..16], b"fmt ", "{}", g.naam());
            assert_eq!(&w[36..40], b"data", "{}", g.naam());
            assert_eq!(
                lees_u32(w, 4) as usize,
                w.len() - 8,
                "RIFF-lengte klopt niet voor {}",
                g.naam()
            );
            assert_eq!(
                lees_u32(w, 40) as usize,
                w.len() - 44,
                "data-lengte klopt niet voor {}",
                g.naam()
            );
        }
    }

    #[test]
    fn de_duur_komt_overeen_met_de_noten() {
        for g in Geluid::ALLE {
            let ms: u32 = g.tonen().iter().map(|(_, ms)| ms).sum();
            let samples = (wav_van(g).len() - 44) / 2;
            assert_eq!(
                samples,
                (SAMPLERATE as u64 * u64::from(ms) / 1000) as usize,
                "{} duurt niet wat er staat",
                g.naam()
            );
        }
    }

    /// Een sinus die op volle amplitude begint klikt hoorbaar, en een geluidje dat klikt
    /// klinkt als een fout in de app.
    #[test]
    fn elk_geluidje_regelt_in_en_uit() {
        for g in Geluid::ALLE {
            let w = wav_van(g);
            let eerste = i16::from_le_bytes(w[44..46].try_into().unwrap());
            let laatste = i16::from_le_bytes(w[w.len() - 2..].try_into().unwrap());
            assert_eq!(eerste, 0, "{} begint met een klik", g.naam());
            assert!(laatste.abs() < 400, "{} eindigt met een klik", g.naam());
        }
    }

    #[test]
    fn geen_geluidje_gaat_over_de_gekozen_amplitude() {
        let plafond = (AMPLITUDE * i16::MAX as f32) as i16 + 2;
        for g in Geluid::ALLE {
            let w = wav_van(g);
            let piek = w[44..]
                .chunks_exact(2)
                .map(|c| i16::from_le_bytes(c.try_into().unwrap()).saturating_abs())
                .max()
                .unwrap_or(0);
            assert!(piek <= plafond, "{} piekt op {piek}", g.naam());
        }
    }
}
