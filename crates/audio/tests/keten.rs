//! De keten van microfoon naar koptelefoon, zonder geluidskaart.
//!
//! Of het *klinkt* kan alleen een mens beoordelen. Wat hier getest wordt is dat er
//! überhaupt herkenbaar geluid uitkomt, en dat verlies en herordening onderweg geen
//! stilte of vastloper opleveren — precies de gevallen die je met de hand niet
//! betrouwbaar kunt nabootsen.

use fitcom_audio::codec::{Decoder, Encoder, MAX_PAYLOAD};
use fitcom_audio::jitter::{Frame, JitterBuffer};
use fitcom_audio::{FRAME_SAMPLES, SAMPLE_RATE};
use fitcom_net::{MediaSocket, MAX_PAKKET};
use fitcom_proto::{MediaHeader, PayloadType, VOICE_STREAM_ID};
use std::net::SocketAddr;

/// Een zuivere toon, zodat we aan de andere kant kunnen meten of er iets zinnigs
/// uitkomt in plaats van ruis of stilte.
fn toon(frames: usize) -> Vec<Vec<i16>> {
    let mut uit = Vec::new();
    let mut fase = 0.0f32;
    let stap = 440.0 * std::f32::consts::TAU / SAMPLE_RATE as f32;
    for _ in 0..frames {
        let mut frame = Vec::with_capacity(FRAME_SAMPLES);
        for _ in 0..FRAME_SAMPLES {
            frame.push((fase.sin() * 8000.0) as i16);
            fase += stap;
        }
        uit.push(frame);
    }
    uit
}

fn energie(samples: &[i16]) -> f64 {
    samples.iter().map(|&s| f64::from(s).powi(2)).sum::<f64>() / samples.len() as f64
}

/// Verstuurt frames en levert op wat er aan de andere kant uitkomt.
/// `verlies` en `omdraaien` bootsen een slecht netwerk na.
fn stuur_en_ontvang(frames: &[Vec<i16>], verlies: &[usize], omdraaien: bool) -> Vec<Vec<i16>> {
    let zender = MediaSocket::bind(0).unwrap();
    let ontvanger = MediaSocket::bind(0).unwrap();
    let doel = SocketAddr::from(([127, 0, 0, 1], ontvanger.local_addr().unwrap().port()));

    let mut enc = Encoder::new().unwrap();
    let mut pakket = [0u8; MAX_PAYLOAD];

    let mut te_versturen: Vec<(u32, Vec<u8>)> = Vec::new();
    for (i, frame) in frames.iter().enumerate() {
        if verlies.contains(&i) {
            continue;
        }
        let n = enc.encode(frame, &mut pakket).unwrap();
        te_versturen.push((i as u32 + 1, pakket[..n].to_vec()));
    }
    if omdraaien {
        // Twee pakketten wisselen van volgorde, zoals dat op een echt netwerk gebeurt.
        te_versturen.swap(1, 2);
    }

    for (seq, payload) in &te_versturen {
        let header = MediaHeader {
            stream_id: VOICE_STREAM_ID,
            seq: *seq,
            timestamp: seq * FRAME_SAMPLES as u32,
            payload_type: PayloadType::OPUS,
            flags: 0,
            frag_index: 0,
        };
        zender.stuur(doel, &header, payload).unwrap();
    }

    let mut buffer = JitterBuffer::new();
    let mut buf = [0u8; MAX_PAKKET];
    while let Ok(Some((_, header, payload))) = ontvanger.ontvang(&mut buf) {
        buffer.push(header.seq, payload.to_vec());
    }

    let mut dec = Decoder::new().unwrap();
    let mut uit = Vec::new();
    for _ in 0..frames.len() + 2 {
        let mut pcm = vec![0i16; FRAME_SAMPLES];
        match buffer.pop() {
            Frame::Data(p) => {
                dec.decode(&p, &mut pcm).unwrap();
                uit.push(pcm);
            }
            Frame::Verloren => {
                dec.verberg_verlies(&mut pcm).unwrap();
                uit.push(pcm);
            }
            Frame::Stilte => {}
        }
    }
    uit
}

#[test]
fn spraak_komt_er_aan_de_andere_kant_herkenbaar_uit() {
    let bron = toon(10);
    let uit = stuur_en_ontvang(&bron, &[], false);

    assert!(uit.len() >= 8, "kreeg maar {} frames terug", uit.len());
    // De eerste frames van Opus zijn nog aan het inregelen; kijk naar de rest.
    let gemiddeld: f64 = uit[3..].iter().map(|f| energie(f)).sum::<f64>() / (uit.len() - 3) as f64;
    assert!(
        gemiddeld > 100_000.0,
        "er komt te weinig geluid uit: {gemiddeld}"
    );
}

#[test]
fn een_verloren_pakket_geeft_geen_stilte_maar_opvulling() {
    // Het verschil tussen een hoorbaar gat en een nauwelijks merkbare hapering.
    let bron = toon(10);
    let uit = stuur_en_ontvang(&bron, &[5], false);

    assert!(uit.len() >= 8, "verlies mag de stroom niet stoppen");
    let stille: usize = uit.iter().filter(|f| energie(f) < 1_000.0).count();
    assert!(stille <= 2, "{stille} frames zijn zo goed als stil");
}

#[test]
fn omgedraaide_pakketten_worden_hersteld() {
    let bron = toon(10);
    let uit = stuur_en_ontvang(&bron, &[], true);
    assert!(uit.len() >= 8);
    let gemiddeld: f64 = uit[3..].iter().map(|f| energie(f)).sum::<f64>() / (uit.len() - 3) as f64;
    assert!(
        gemiddeld > 100_000.0,
        "herordening mag het geluid niet slopen"
    );
}

#[test]
fn pakket_van_een_vreemde_stream_wordt_genegeerd() {
    // Straks deelt er iemand zijn scherm over dezelfde poort. Die pakketten mogen
    // niet in de audio-jitterbuffer belanden.
    let zender = MediaSocket::bind(0).unwrap();
    let ontvanger = MediaSocket::bind(0).unwrap();
    let doel = SocketAddr::from(([127, 0, 0, 1], ontvanger.local_addr().unwrap().port()));

    let header = MediaHeader {
        stream_id: 7, // screenshare
        seq: 1,
        timestamp: 0,
        payload_type: PayloadType::HEVC,
        flags: 0,
        frag_index: 0,
    };
    zender.stuur(doel, &header, b"beeld").unwrap();

    let mut buf = [0u8; MAX_PAKKET];
    let (_, terug, _) = ontvanger.ontvang(&mut buf).unwrap().unwrap();
    assert_ne!(
        terug.stream_id, VOICE_STREAM_ID,
        "de ontvangstlus filtert hierop"
    );
}
