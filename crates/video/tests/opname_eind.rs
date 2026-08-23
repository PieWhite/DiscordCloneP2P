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
    kies_venster, laad_segmenten, AudioBronnen, ClipGebeurtenis, ClipInstellingen,
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

    let gebeurtenis = rx
        .recv_timeout(Duration::from_secs(15))
        .expect("clip-event");
    let pad = match gebeurtenis {
        ClipGebeurtenis::Klaar { pad } => pad,
        ClipGebeurtenis::Mislukt { reden } => panic!("clip mislukt: {reden}"),
    };
    println!("clip: {}", pad.display());

    // Het bestand bestaat, is geen leeg hulsje en heet niet meer .part.
    let meta = std::fs::metadata(&pad).expect("clipbestaat");
    assert!(
        meta.len() > 10_000,
        "clip verdacht klein: {} bytes",
        meta.len()
    );
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
    let laatste = r.read_sample(1, n).unwrap().expect("laatste monster");
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
    let tweede = rx
        .recv_timeout(Duration::from_secs(15))
        .expect("tweede event");
    assert!(
        matches!(tweede, ClipGebeurtenis::Klaar { .. }),
        "tweede clip mislukt"
    );

    drop(handle);
    let _ = std::fs::remove_dir_all(&ring);
    let _ = std::fs::remove_dir_all(&clips);
}

/// Een herstart begint met een schone ring: alles van de vorige sessie is weg, en er
/// komt vers materiaal voor in de plaats.
///
/// Dit was ooit precies andersom bedoeld ("de ring wordt weer opgepakt") en dat was de
/// bug: segmentnamen dragen tijden van een procesklok die bij elke start op nul begint,
/// dus oude segmenten kwamen ná de nieuwe te liggen. `kies_venster` gaf dan de beelden
/// van de vórige sessie terug, en de retentie ruimde juist de verse segmenten op.
/// In één proces viel dat nooit op — daar is de klok immers dezelfde.
#[test]
#[ignore = "vereist een GPU met hardware-encoder en een echt scherm"]
fn herstart_begint_met_een_schone_ring() {
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
            ClipInstellingen {
                fps: 60,
                bitrate: 4_000_000,
                venster_sec: 5,
            },
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

    // Tweede run: dezelfde map. De harde eisen: geen enkel segment van run 1 overleeft,
    // en er komt vers materiaal voor in de plaats.
    let (tx2, _rx2) = std::sync::mpsc::channel();
    let h2 = fitcom_video::opname::start_opname(
        &d3d,
        &bron,
        ClipInstellingen {
            fps: 60,
            bitrate: 4_000_000,
            venster_sec: 5,
        },
        ring.clone(),
        clips.clone(),
        None,
        tx2,
    )
    .unwrap();

    // Wachten op de nieuwe toestand: alles van run 1 weg én minstens één vers segment.
    // Niet enkel "de map is niet leeg" — dat is hij aan het begin van run 2 nog steeds,
    // met precies de oude bestanden erin. Een stilstaand scherm mag hier even over doen.
    let start_wachten = Instant::now();
    loop {
        let nu = laad_segmenten(&ring);
        let oud_weg = metas_na_stop.iter().all(|m| !m.pad.exists());
        let vers = nu
            .iter()
            .any(|m| !metas_na_stop.iter().any(|oud| oud.pad == m.pad));
        if oud_weg && vers {
            break;
        }
        assert!(
            start_wachten.elapsed() <= Duration::from_secs(25),
            "na 25 s nog geen schone ring met vers materiaal \
             (oud weg: {oud_weg}, vers segment: {vers})"
        );
        std::thread::sleep(Duration::from_millis(500));
    }

    drop(h2);
    let _ = std::fs::remove_dir_all(&ring);
    let _ = std::fs::remove_dir_all(&clips);
}

/// Geluid komt in de clip, óók als de opname pas een tijd ná het opstarten begint.
///
/// Dat "ná" is de hele test. De geluidsbronnen leveren hun chunks met een tijd sinds
/// procesbegin; de menger werd daarnaast opgezet met basis nul. Ging clips pas na een
/// paar minuten aan, dan bouwde hij de tijdlijn vanaf seconde nul op — honderden
/// megabytes stilte, een AAC-encoder die zich daar eerst doorheen moest werken, en
/// audiomonsters die vervolgens allemaal vóór het eerste segment lagen en dus wegvielen.
/// Hier volstaan een paar seconden voorsprong om het verschil te zien.
///
/// De geluidsbron is synthetisch: 10 ms stereo op 48 kHz, precies wat de loopback-tap
/// levert. Dat houdt de test los van welke geluidskaart er in de machine zit.
#[test]
#[ignore = "vereist een GPU met hardware-encoder en een echt scherm"]
fn geluid_komt_in_de_clip_ook_als_de_opname_later_begint() {
    let d3d = D3dContext::new().expect("D3D11");
    let bron = beschikbare_bronnen()
        .expect("bronnen")
        .into_iter()
        .find(|b| b.soort == fitcom_video::capture::BronSoort::Monitor)
        .expect("een scherm om op te nemen");

    // De voorsprong waar het om draait.
    std::thread::sleep(Duration::from_secs(3));

    let ring = std::env::temp_dir().join(format!("fitcom-ring-audio-{}", std::process::id()));
    let clips = std::env::temp_dir().join(format!("fitcom-clips-audio-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&ring);
    let _ = std::fs::remove_dir_all(&clips);

    let (pcm_tx, pcm_rx) = std::sync::mpsc::channel::<Vec<f32>>();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop_draad = stop.clone();
    let geluid = std::thread::spawn(move || {
        let mut fase = 0f32;
        while !stop_draad.load(std::sync::atomic::Ordering::Relaxed) {
            let mut chunk = Vec::with_capacity(480 * 2);
            for _ in 0..480 {
                let s = (fase * 2.0 * std::f32::consts::PI).sin() * 0.2;
                fase = (fase + 440.0 / 48_000.0).fract();
                chunk.push(s);
                chunk.push(s);
            }
            if pcm_tx.send(chunk).is_err() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    });

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
        Some(AudioBronnen {
            systeem: Some(pcm_rx),
            microfoon: None,
        }),
        tx,
    )
    .expect("opname starten");

    std::thread::sleep(Duration::from_secs(7));
    handle.bewaar_nu();
    let pad = match rx
        .recv_timeout(Duration::from_secs(15))
        .expect("clip-event")
    {
        ClipGebeurtenis::Klaar { pad } => pad,
        ClipGebeurtenis::Mislukt { reden } => panic!("clip mislukt: {reden}"),
    };
    drop(handle);
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = geluid.join();

    let f = std::fs::File::open(&pad).unwrap();
    let len = f.metadata().unwrap().len();
    let mut r = Mp4Reader::read_header(f, len).expect("clip leesbaar als MP4");
    assert!(r.tracks().contains_key(&2), "clip heeft geen geluidsspoor");

    let nv = r.sample_count(1).expect("videotrack");
    let laatste_v = r.read_sample(1, nv).unwrap().expect("laatste beeld");
    let video_eind = (laatste_v.start_time + u64::from(laatste_v.duration)) as f64 / 10_000_000.0;

    let na = r.sample_count(2).expect("audiotrack");
    assert!(na > 0, "geluidsspoor zonder monsters");
    let eerste_a = r.read_sample(2, 1).unwrap().expect("eerste geluidsmonster");
    let laatste_a = r
        .read_sample(2, na)
        .unwrap()
        .expect("laatste geluidsmonster");
    let audio_begin = eerste_a.start_time as f64 / 48_000.0;
    let audio_eind = (laatste_a.start_time + u64::from(laatste_a.duration)) as f64 / 48_000.0;
    println!(
        "video 0..{video_eind:.2} s, geluid {audio_begin:.2}..{audio_eind:.2} s ({na} monsters)"
    );

    // Het geluid moet het beeld dekken: even lang, en niet ernaast. Twee oude fouten
    // vallen hier door de mand — de menger die zijn tijdlijn op seconde nul begon (dan
    // blijft audio_eind ver achter), en de duur die in hns in plaats van in samples werd
    // weggeschreven (dan beweert een spoor van vier seconden dertien minuten te duren).
    assert!(audio_begin < 0.5, "geluid begint pas op {audio_begin:.2} s");
    assert!(
        (audio_eind - video_eind).abs() < 1.0,
        "geluid loopt tot {audio_eind:.2} s en beeld tot {video_eind:.2} s — die horen          binnen een seconde van elkaar te liggen"
    );

    let _ = std::fs::remove_dir_all(&ring);
    let _ = std::fs::remove_dir_all(&clips);
}
