//! Rooktest op de échte geluidsapparaten van deze machine.
//!
//! Staat op `#[ignore]` omdat hij een werkende microfoon en weergave nodig heeft: op
//! een machine zonder geluid zou hij terecht falen zonder dat er iets mis is met de
//! code. Draai hem met de hand na wijzigingen aan `session.rs`:
//!
//! ```text
//! cargo test -p fitcom-audio --test apparaten -- --ignored --nocapture
//! ```

use fitcom_audio::{VoiceConfig, VoiceHandle};
use std::time::Duration;

fn config() -> VoiceConfig {
    VoiceConfig {
        media_port: 0, // laat het systeem een vrije poort kiezen
        input_device: None,
        output_device: None,
    }
}

#[test]
#[ignore = "vereist een echte geluidskaart"]
fn sessie_start_en_stopt_netjes() {
    let sessie = fitcom_audio::start(config()).expect("voice moet kunnen starten");
    println!("mediapoort: {}", sessie.media_addr);
    assert_ne!(sessie.media_addr.port(), 0, "poort moet toegekend zijn");

    // Even laten lopen zodat alle vier de threads echt aan het werk zijn geweest.
    std::thread::sleep(Duration::from_millis(500));
    let niveaus = sessie.niveaus();
    println!("eigen niveau: {:.4}", niveaus.eigen);

    drop(sessie);
    // Threads mogen tot hun timeout doen over het opmerken; daarna moet alles los zijn.
    std::thread::sleep(Duration::from_millis(500));
}

#[test]
#[ignore = "vereist een echte geluidskaart"]
fn opnieuw_deelnemen_op_dezelfde_poort_lukt() {
    // Het gebruikelijke geval waarin dit misgaat: verlaten en meteen weer deelnemen,
    // terwijl de vorige sessie de UDP-poort nog vasthoudt.
    let eerste = fitcom_audio::start(config()).expect("eerste sessie");
    let poort = eerste.media_addr.port();
    drop(eerste);

    let tweede = fitcom_audio::start(VoiceConfig {
        media_port: poort,
        ..config()
    })
    .expect("meteen opnieuw deelnemen moet lukken");
    assert_eq!(tweede.media_addr.port(), poort);
}

#[test]
#[ignore = "vereist een echte geluidskaart"]
fn apparaten_zijn_op_te_vragen() {
    let (invoer, uitvoer) = fitcom_audio::session::apparaatnamen().unwrap();
    println!("microfoons:");
    for n in &invoer {
        println!("  {n}");
    }
    println!("weergave:");
    for n in &uitvoer {
        println!("  {n}");
    }
    assert!(!uitvoer.is_empty(), "geen enkel weergaveapparaat gevonden");
}

/// Voorkomt dat de compiler het type wegoptimaliseert als de tests overgeslagen worden.
#[allow(dead_code)]
fn typecheck(_: &VoiceHandle) {}
