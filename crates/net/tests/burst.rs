//! Wat er gebeurt als een keyframe in één stoot de deur uit gaat.
//!
//! Een keyframe van 1080p is 100 tot 260 kB en gaat in ruim tweehonderd fragmenten van
//! 1116 bytes achter elkaar de socket in. De ontvanger is op dat moment bezig: hij
//! decodeert en presenteert het vorige beeld. Past de stoot niet in de ontvangbuffer van
//! zijn socket, dan gooit het besturingssysteem de rest weg, is het beeld incompleet,
//! vraagt de kijker een nieuw keyframe, en begint hetzelfde opnieuw.
//!
//! Dit heeft geen GPU en geen tweede machine nodig: op loopback raakt onderweg niets
//! kwijt, dus alles wat hier sneuvelt sneuvelt in een buffer aan de ontvangkant.

use fitcom_net::{MediaSleutels, MediaSocket, PeerAdres, MAX_PAKKET};
use fitcom_proto::{MediaHeader, PayloadType, PeerId};
use std::net::SocketAddr;
use std::time::Duration;

/// Zo groot is een keyframe van 1080p in fragmenten. Gemeten met de meter in `deler.rs`:
/// 260 kB gedeeld door 1100 bytes payload. Sinds protocol 6 is elk fragment 28 bytes
/// groter (sessie-index, teller en tag van de AEAD); ook daarmee moet het passen.
const FRAGMENTEN: usize = 242;

#[test]
fn een_keyframe_in_een_stoot_overleeft_de_ontvangbuffer() {
    let sleutels = MediaSleutels::default();
    let (deler, kijker) = (PeerId::new_random(), PeerId::new_random());
    MediaSleutels::koppel((&sleutels, deler), (&sleutels, kijker));

    let mut ontvanger = MediaSocket::bind(0, sleutels.clone()).unwrap();
    ontvanger.zet_timeout(Duration::from_millis(50)).unwrap();
    let doel = PeerAdres {
        id: kijker,
        addr: SocketAddr::from(([127, 0, 0, 1], ontvanger.local_addr().unwrap().port())),
    };

    let zender = MediaSocket::bind(0, sleutels).unwrap();
    let payload = [7u8; 1100];
    for i in 0..FRAGMENTEN {
        let header = MediaHeader {
            stream_id: 1,
            seq: i as u32,
            timestamp: 9000,
            payload_type: PayloadType::H264,
            flags: if i + 1 == FRAGMENTEN {
                MediaHeader::FLAG_LAST_FRAGMENT
            } else {
                0
            },
            frag_index: i as u16,
        };
        zender.stuur(doel, &header, &payload).unwrap();
    }

    // Pas nu gaan we lezen. Dat is niet gemeen: de kijker zit tijdens de stoot in
    // `decode` en `toon`, en die duren samen langer dan het versturen van 242 pakketten.
    let mut buf = [0u8; MAX_PAKKET];
    let mut aangekomen = 0;
    while ontvanger.ontvang(&mut buf).unwrap().is_some() {
        aangekomen += 1;
    }

    assert_eq!(
        aangekomen,
        FRAGMENTEN,
        "{} van de {FRAGMENTEN} fragmenten weggegooid voordat de kijker ze kon lezen; \
         één ervan missen maakt het hele keyframe onbruikbaar",
        FRAGMENTEN - aangekomen
    );
}

/// Wat de extra stap door de verdeler (fase B5) kost: van `send_to` bij de deler tot het
/// pakket bij de kijker is, rechtstreeks van een eigen socket (zoals vóór B5) en via de
/// verdeler. 1080p60 op 12 Mbit/s: 24 fragmenten per beeld, drie seconden.
///
/// Een meting en geen bewaker: tijden op een drukke machine lopen uiteen (zie
/// `leestimeout.rs`). Draaien met
/// `cargo test --release -p fitcom-net --test burst -- --ignored --nocapture`.
#[test]
#[ignore = "meting; zie de doc"]
fn vertraging_door_de_verdeler() {
    use fitcom_net::Verdeler;
    use std::time::Instant;

    let begin = Instant::now();
    let stuur = |zender: MediaSocket, doel: PeerAdres| {
        std::thread::spawn(move || {
            let start = Instant::now();
            for beeld in 0..180u32 {
                let tot = start + Duration::from_micros(16_667 * u64::from(beeld) + 200_000);
                std::thread::sleep(tot.saturating_duration_since(Instant::now()));
                for i in 0..24 {
                    let mut payload = [0u8; 1100];
                    payload[..8]
                        .copy_from_slice(&(begin.elapsed().as_nanos() as u64).to_be_bytes());
                    let header = MediaHeader {
                        stream_id: 1,
                        seq: beeld * 24 + i,
                        timestamp: beeld,
                        payload_type: PayloadType::H264,
                        flags: 0,
                        frag_index: i as u16,
                    };
                    zender.stuur(doel, &header, &payload).unwrap();
                }
            }
        })
    };
    let vertraging = |payload: &[u8]| {
        let verstuurd = u64::from_be_bytes(payload[..8].try_into().unwrap());
        (begin.elapsed().as_nanos() as u64 - verstuurd) as f64 / 1e6
    };
    let samenvatting = |naam: &str, mut v: Vec<f64>| {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let p = |q: f64| v[((v.len() as f64 * q) as usize).min(v.len() - 1)];
        println!(
            "{naam}: n={} p50={:.3} p99={:.3} max={:.3} ms",
            v.len(),
            p(0.5),
            p(0.99),
            v[v.len() - 1]
        );
    };

    // Vóór B5: de kijker leest zijn eigen socket.
    let sleutels = MediaSleutels::default();
    let (deler, kijker) = (PeerId::new_random(), PeerId::new_random());
    MediaSleutels::koppel((&sleutels, deler), (&sleutels, kijker));
    let mut eigen = MediaSocket::bind(0, sleutels.clone()).unwrap();
    eigen.zet_timeout(Duration::from_millis(8)).unwrap();
    let doel = PeerAdres {
        id: kijker,
        addr: SocketAddr::from(([127, 0, 0, 1], eigen.local_addr().unwrap().port())),
    };
    let t = stuur(MediaSocket::bind(0, sleutels.clone()).unwrap(), doel);
    let mut buf = [0u8; MAX_PAKKET];
    let mut direct = Vec::new();
    while direct.len() < 180 * 24 {
        match eigen.ontvang(&mut buf).unwrap() {
            Some((_, _, p)) => direct.push(vertraging(p)),
            None if t.is_finished() => break,
            None => {}
        }
    }
    t.join().unwrap();
    samenvatting("eigen socket", direct);

    // Na B5: via de verdeler en zijn rij.
    let post = Verdeler::start(0, sleutels.clone()).unwrap();
    let rij = post.luister_beeld(deler, 1);
    let doel = PeerAdres {
        addr: SocketAddr::from(([127, 0, 0, 1], post.poort())),
        ..doel
    };
    let begin_tweede = Instant::now();
    let t = stuur(MediaSocket::bind(0, sleutels).unwrap(), doel);
    let mut via = Vec::new();
    while via.len() < 180 * 24 && begin_tweede.elapsed() < Duration::from_secs(10) {
        if let Ok(p) = rij.recv_timeout(Duration::from_millis(8)) {
            via.push(vertraging(&p.payload));
        }
    }
    t.join().unwrap();
    samenvatting("via de verdeler", via);
}
