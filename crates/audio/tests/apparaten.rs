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
fn bureaubladgeluid_kan_worden_afgetapt() {
    // De aanname onder desktop-audio: `cpal` zet loopback aan zodra je een invoerstroom
    // op een uitvoerapparaat bouwt. Klopt dat niet op deze machine, dan valt dit hier
    // om in plaats van pas als er iemand meeluistert.
    use fitcom_net::MediaSocket;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    let sessie = fitcom_audio::start(config()).expect("voice moet kunnen starten");
    let luisteraar = MediaSocket::bind(0).expect("luisterpoort");
    let doel = SocketAddr::new(
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        luisteraar.local_addr().unwrap().port(),
    );

    sessie
        .deel_bureaublad(7, vec![doel])
        .expect("bureaubladgeluid moet te delen zijn");
    assert!(sessie.deelt_bureaublad());

    // Er hoeft geen geluid te spelen: dit bewijst dat de opname draait en de keten
    // opgezet is. Speelt er wél iets af, dan komen er pakketten binnen.
    std::thread::sleep(Duration::from_secs(2));
    let mut buf = [0u8; fitcom_net::MAX_PAKKET];
    let mut pakketten = 0;
    for _ in 0..10 {
        if let Ok(Some((_, header, _))) = luisteraar.ontvang(&mut buf) {
            assert_eq!(header.stream_id, 7);
            pakketten += 1;
        }
    }
    println!(
        "{pakketten} pakketten opgevangen (nul is goed als er niets speelt: stilte hoort geen verkeer te kosten)"
    );

    sessie.stop_bureaublad();
    assert!(!sessie.deelt_bureaublad());
    std::thread::sleep(Duration::from_millis(400));
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

/// De microfoon-tap voor clips levert écht chunks, en op de snelheid van de klok.
///
/// Dit is geen theoretische test. De tap bouwde zijn `cpal::Stream` in een hulpfunctie en
/// gaf hem niet terug, dus de stream viel weg op de regel ná `play()`. `start()` meldde
/// netjes Ok, er kwam geen enkele fout in de log, en er kwam nooit één sample: clips met
/// spelgeluid maar zonder je eigen stem. Alleen een test die de chunks daadwerkelijk
/// opvangt ziet dat verschil.
///
/// Bewust geen eis aan het volume — een stille kamer of een gedempte microfoon is geen
/// kapotte tap. Wat hier telt is dat er samples komen, op ongeveer 48 kHz stereo.
#[test]
#[ignore = "vereist een echte microfoon"]
fn microfoon_tap_levert_chunks() {
    let (tap, rx) = fitcom_audio::microfoon::MicrofoonTap::start(None).expect("microfoon-tap");

    let duur = Duration::from_millis(1500);
    let begin = std::time::Instant::now();
    let mut chunks = 0usize;
    let mut samples = 0usize;
    let mut piek = 0f32;
    while begin.elapsed() < duur {
        if let Ok(chunk) = rx.recv_timeout(Duration::from_millis(200)) {
            chunks += 1;
            samples += chunk.len();
            for s in chunk {
                piek = piek.max(s.abs());
            }
        }
    }
    drop(tap);

    let seconden = samples as f64 / 2.0 / 48_000.0;
    println!("{chunks} chunks, {samples} samples = {seconden:.2} s geluid, piek {piek:.3}");
    assert!(chunks > 0, "de microfoon-tap leverde geen enkele chunk");
    // Ruime band: de driver bepaalt de blokgrootte en de eerste chunk kan laat zijn.
    assert!(
        seconden > 0.5,
        "maar {seconden:.2} s geluid in 1,5 s — de tap loopt niet op kloksnelheid"
    );
    assert!(
        samples.is_multiple_of(2),
        "geen stereo: oneven aantal samples"
    );
}

/// Voorkomt dat de compiler het type wegoptimaliseert als de tests overgeslagen worden.
#[allow(dead_code)]
fn typecheck(_: &VoiceHandle) {}
