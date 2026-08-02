//! Hoe kort is een korte leestimeout op deze machine werkelijk?
//!
//! De kijkerlus zet zijn leestimeout op 1 ms zodra er een beeld op zijn beurt wacht, en
//! rekent erop dat hij dan ook binnen die milliseconde weer aan de beurt is om te kijken
//! of het beeld getoond moet worden. Gemeten (2026-08-02) is hij gemiddeld 5,8 ms te laat
//! op zijn eigen planning, uniform verdeeld over een hele beeldtijd — precies wat je ziet
//! als die timeout in werkelijkheid veel grover is dan gevraagd. En zo was het ook: 1, 2
//! én 8 ms duurden alle drie 15,6 ms, de standaardtik van Windows.
//!
//! Gefixt met `timeBeginPeriod(1)` vanuit `MediaSocket::bind`. Deze test staat hier omdat
//! die fix één regel is die niemand mist als hij bij een opruimronde sneuvelt, en het
//! gevolg — beeld dat weer op aankomsttijd getoond wordt in plaats van op opnametijd —
//! alleen met twee machines op te merken valt.

use fitcom_net::{MediaSocket, MAX_PAKKET};
use std::time::{Duration, Instant};

fn meet(gevraagd: Duration) -> (f64, f64) {
    // `bind` zet de timerresolutie; zonder die aanroep is alles hieronder 15,6 ms.
    let sock = MediaSocket::bind(0).expect("socket");
    sock.zet_timeout(gevraagd).expect("timeout");
    let mut buf = [0u8; MAX_PAKKET];

    let mut duren = Vec::new();
    for _ in 0..100 {
        let voor = Instant::now();
        let _ = sock.ontvang(&mut buf);
        duren.push(voor.elapsed().as_secs_f64() * 1000.0);
    }
    duren.sort_by(f64::total_cmp);
    (duren[duren.len() / 2], duren[duren.len() - 1])
}

#[test]
fn korte_leestimeout_is_zo_kort_als_gevraagd() {
    let (mediaan, max) = meet(Duration::from_millis(1));
    println!("gevraagd 1 ms → mediaan {mediaan:.2} ms, langste {max:.2} ms");
    assert!(
        mediaan < 3.0,
        "een leestimeout van 1 ms duurt hier {mediaan:.1} ms; de kijkerlus kan dan niet \
         op tijd tonen en de weergaveklok doet niets"
    );
}
