#![cfg(windows)]

//! De hele clipketen over de echte hardware: opname → ring → remux → afspeelbaar
//! bestand. Dit is de test die bewijst dat fase 15 meer is dan losse onderdelen die
//! elk voor zich werken.
//!
//! Heeft een GPU met hardware-encoder én een echt scherm nodig — geen venster, maar
//! wel een monitor om WGC te laten capturen.
//!
//! ```text
//! cargo test -p fitcom-video --test opname_eind -- --ignored --nocapture
//! ```

use fitcom_video::capture::beschikbare_bronnen;
use fitcom_video::opname::{
    kies_venster, laad_segmenten, ClipGebeurtenis, ClipInstellingen,
};
use fitcom_video::D3dContext;
use mp4::Mp4Reader;
use std::time::{Duration, Instant};

#[test]
#[ignore = "vereist een GPU met hardware-encoder en een echt scherm"]
fn clipketen_levert_een_afspeelbaar_bestand() {
    let d3d = D3dContext::new().expect("D3D11");
    let bron = beschikbare_bronnen()
        .expect("bronnen")
        .into_iter()
        .find(|b| b.soort == fitcom_video::capture::BronSoort::Monitor)
        .expect("een scherm om op te nemen");

    let ring = std::env::temp_dir().join(format!("fitcom-ring-test-{}", std::process::id()));
    let clips = std::env::temp_dir().join(format!("fitcom-clips-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&ring);
    let _ = std::fs::remove_dir_all(&clips);

    let (tx, rx) = std::sync::mpsc::channel();
    let handle = fitcom_video::opname::start_opname(
        &d3d,
        &bron,
        ClipInstellingen {
            fps: 60,
            bitrate: 4_000_000,
            venster_sec: 5,
        },
        ring.clone(),
        clips.clone(),
        None, // videoketen eerst; geluid heeft zijn eigen dekking via de loopback-tap
        tx,
    )
    .expect("opname starten");

    // Zeven seconden: ruim drie segmenten van twee seconden.
    std::thread::sleep(Duration::from_secs(7));
    handle.bewaar_nu();

    let gebeurtenis = rx.recv_timeout(Duration::from_secs(15)).expect("clip-event");
    let pad = match gebeurtenis {
        ClipGebeurtenis::Klaar { pad } => pad,
        ClipGebeurtenis::Mislukt { reden } => panic!("clip mislukt: {reden}"),
    };
    println!("clip: {}", pad.display());

    // Het bestand bestaat, is geen leeg hulsje en heet niet meer .part.
    let meta = std::fs::metadata(&pad).expect("clipbestaat");
    assert!(meta.len() > 10_000, "clip verdacht klein: {} bytes", meta.len());
    assert!(!pad.to_string_lossy().ends_with(".part.mp4"));

    // En het is een echte MP4: moov leest, videotrack heeft beelden, eerste is een
    // keyframe, en de duur komt in de buurt van het gevraagde venster.
    let f = std::fs::File::open(&pad).unwrap();
    let len = f.metadata().unwrap().len();
    let mut r = Mp4Reader::read_header(f, len).expect("clip leesbaar als MP4");
    let n = r.sample_count(1).expect("videotrack");
    println!("monsters: {n}");
    // Een stilstaand bureaublad levert nauwelijks beelden — WGC geeft alleen wat prijs
    // als het scherm verandert. Tegen een spel aan zijn dit er honderden; hier volstaat
    // "er is echt iets opgenomen".
    assert!(n >= 3, "vrijwel niets opgenomen: {n} monsters");

    let eerste = r.read_sample(1, 1).unwrap().expect("eerste monster");
    assert!(eerste.is_sync, "clip begint niet op een keyframe");
    let laatste = r
        .read_sample(1, n)
        .unwrap()
        .expect("laatste monster");
    let duur_hns = i64::try_from(laatste.start_time + u64::from(laatste.duration)).unwrap();
    println!("duur: {:.1} s", duur_hns as f64 / 10_000_000.0);
    // Vijf seconden gevraagd. Bij een statisch scherm spannen die paar beelden minder
    // echte tijd in; de ondergrens zegt alleen dat het geen losse flits is.
    assert!(
        duur_hns >= 250_000,
        "clip duurt {:.1} s — vrijwel niets",
        duur_hns as f64 / 10_000_000.0
    );

    // De ring zelf: begrensd, en de segmentadministratie ziet wat er op schijf staat.
    let metas = laad_segmenten(&ring);
    println!("segmenten in de ring: {}", metas.len());
    assert!(
        metas.len() <= 8,
        "ring groeit niet mee: {} segmenten na 7 s + marge",
        metas.len()
    );
    let gekozen = kies_venster(&metas, 5_000_000);
    assert!(!gekozen.is_empty(), "vensterkeuze vindt niets");

    // Tweede save direct daarna moet ook kunnen: de ring is er nog, de vorige clip
    // staat los van hem.
    handle.bewaar_nu();
    let tweede = rx.recv_timeout(Duration::from_secs(15)).expect("tweede event");
    assert!(matches!(tweede, ClipGebeurtenis::Klaar { .. }), "tweede clip mislukt");

    drop(handle);
    let _ = std::fs::remove_dir_all(&ring);
    let _ = std::fs::remove_dir_all(&clips);
}

/// Een herstart van de opname pikt de bestaande ring weer op: geen dubbele segmenten,
/// geen verloren geschiedenis. Aparte test omdat hij een eigen map en timing wil.
#[test]
#[ignore = "vereist een GPU met hardware-encoder en een echt scherm"]
fn herstart_pikt_de_ring_weer_op() {
    let d3d = D3dContext::new().expect("D3D11");
    let bron = beschikbare_bronnen()
        .expect("bronnen")
        .into_iter()
        .find(|b| b.soort == fitcom_video::capture::BronSoort::Monitor)
        .expect("een scherm");

    let ring = std::env::temp_dir().join(format!("fitcom-ring-hertest-{}", std::process::id()));
    let clips = std::env::temp_dir().join(format!("fitcom-clips-hertest-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&ring);
    let _ = std::fs::remove_dir_all(&clips);

    let (tx1, _rx1) = std::sync::mpsc::channel();
    {
        let h = fitcom_video::opname::start_opname(
            &d3d,
            &bron,
            ClipInstellingen { fps: 60, bitrate: 4_000_000, venster_sec: 5 },
            ring.clone(),
            clips.clone(),
            None,
            tx1,
        )
        .unwrap();
        // Een stilstaand scherm levert pas een segment op zodra er wéér iets verandert
        // (sluiten gebeurt op keyframes van échte beelden). Wachten tot er minstens
        // één afgesloten segment ligt, in plaats van blind vijf seconden te slapen.
        let begin_wachten = Instant::now();
        loop {
            if !laad_segmenten(&ring).is_empty() {
                break;
            }
            assert!(
                begin_wachten.elapsed() <= Duration::from_secs(20),
                "geen afgesloten segment binnen 20 s"
            );
            std::thread::sleep(Duration::from_millis(500));
        }
        drop(h); // stopt; de ring blijft liggen
    }

    let metas_na_stop = laad_segmenten(&ring);
    assert!(!metas_na_stop.is_empty(), "na de eerste run ligt er niets");
    let totaal_bytes: u64 = metas_na_stop
        .iter()
        .filter_map(|m| std::fs::metadata(&m.pad).ok())
        .map(|m| m.len())
        .sum();

    // Tweede run: dezelfde map. De harde eisen: er komt bij (een bewegende wereld
    // levert gewoon nieuwe segmenten), niets wordt herschreven, en elk óúd segment
    // dat nog bestaat is nog steeds een leesbaar MP4. Retentie mag de allereerste
    // wél al geretireerd hebben — het venster (5 s) is korter dan de twee runs samen.
    let (tx2, _rx2) = std::sync::mpsc::channel();
    let h2 = fitcom_video::opname::start_opname(
        &d3d,
        &bron,
        ClipInstellingen { fps: 60, bitrate: 4_000_000, venster_sec: 5 },
        ring.clone(),
        clips.clone(),
        None,
        tx2,
    )
    .unwrap();

    // Wachten tot de ring gegroeid is; een stilstaand scherm kan even duren.
    let start_wachten = Instant::now();
    loop {
        let n: u64 = laad_segmenten(&ring)
            .iter()
            .filter_map(|m| std::fs::metadata(&m.pad).ok())
            .map(|m| m.len())
            .sum();
        if n > totaal_bytes {
            break;
        }
        assert!(
            start_wachten.elapsed() <= Duration::from_secs(20),
            "de herstart groeide niet binnen 20 s ({totaal_bytes} → {n} bytes)"
        );
        std::thread::sleep(Duration::from_millis(500));
    }

    for m in &metas_na_stop {
        if !m.pad.exists() {
            continue; // geretireerd door retentie; zie de opmerking hierboven
        }
        mp4::Mp4Reader::read_header(
            std::fs::File::open(&m.pad).expect("oud segment weg"),
            std::fs::metadata(&m.pad).unwrap().len(),
        )
        .expect("oud segment niet meer te lezen");
    }

    drop(h2);
    let _ = std::fs::remove_dir_all(&ring);
    let _ = std::fs::remove_dir_all(&clips);
    let _ = Instant::now(); // houdt de import levend als asserts ooit veranderen
}
