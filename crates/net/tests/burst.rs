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

use fitcom_net::{MediaSocket, MAX_PAKKET};
use fitcom_proto::{MediaHeader, PayloadType};
use std::net::SocketAddr;
use std::time::Duration;

/// Zo groot is een keyframe van 1080p in fragmenten. Gemeten met de meter in `deler.rs`:
/// 260 kB gedeeld door 1100 bytes payload.
const FRAGMENTEN: usize = 242;

#[test]
fn een_keyframe_in_een_stoot_overleeft_de_ontvangbuffer() {
    let ontvanger = MediaSocket::bind(0).unwrap();
    ontvanger.zet_timeout(Duration::from_millis(50)).unwrap();
    let doel = SocketAddr::from(([127, 0, 0, 1], ontvanger.local_addr().unwrap().port()));

    let zender = MediaSocket::bind(0).unwrap();
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
