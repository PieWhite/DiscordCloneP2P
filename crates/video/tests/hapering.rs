//! De hele keten met een bron waarvan we precies weten hoe hij beweegt.
//!
//! `keten.rs` deelt het bureaublad en filmt daarmee zijn eigen kijkvenster: nooit twee
//! keer hetzelfde, dus je kunt er niet aan rekenen. Hier is de bron een eigen venster dat
//! op een exact tijdraster van 60 beelden per seconde van inhoud wisselt — precies wat een
//! filmpje doet. Alles wat er in de weergave *niet* gelijkmatig uitkomt, hebben wij
//! toegevoegd.
//!
//! ```text
//! $env:FITCOM_SPOOR = "C:\pad\naar\map"; $env:HAPER_SECONDEN = "40"
//! cargo test -p fitcom-video --test hapering -- --ignored --nocapture
//! ```
//!
//! Levert `deler-1.csv` en `kijker-1.csv` in `FITCOM_SPOOR`: één regel per beeld, aan
//! beide kanten. Daar is de periodieke microhapering in te dateren; de meterregels per
//! seconde middelen hem juist weg.

use fitcom_video::capture::{Bron, BronSoort};
use fitcom_video::venster::Venster;
use fitcom_video::{beschikbare_bronnen, deel, kijk, Codec, D3dContext, DelerConfig, KijkerConfig};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const BRON_TITEL: &str = "FITCOM-BRON-60";
/// Zoveel verschillende beelden draaien er rond. Genoeg beweging dat de encoder echt
/// werk heeft, weinig genoeg dat ze allemaal in het videogeheugen passen.
const FASEN: usize = 24;

fn env(naam: &str, standaard: u64) -> u64 {
    std::env::var(naam)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(standaard)
}

fn breedte() -> u32 {
    env("HAPER_BREEDTE", 1280) as u32
}

fn hoogte() -> u32 {
    env("HAPER_HOOGTE", 720) as u32
}

/// Een verticale balk die per fase opschuift, op een ruisachtige achtergrond. De balk
/// geeft beweging die je in de codec terugziet; de ruis zorgt dat er echt bits nodig zijn.
fn fase(n: usize) -> Vec<u8> {
    let (b, h) = (breedte(), hoogte());
    let balk = (n * b as usize / FASEN) as u32;
    let mut uit = vec![0u8; (b * h * 4) as usize];
    for y in 0..h {
        for x in 0..b {
            let i = ((y * b + x) * 4) as usize;
            let dicht_bij = x.abs_diff(balk) < 40;
            // Goedkope pseudo-ruis; hoeft niet mooi te zijn, wel onvoorspelbaar.
            let r = ((x.wrapping_mul(2654435761) ^ y.wrapping_mul(40503)) >> 13) as u8;
            uit[i] = if dicht_bij { 250 } else { r / 3 };
            uit[i + 1] = if dicht_bij { 220 } else { r / 4 + 20 };
            uit[i + 2] = if dicht_bij { 40 } else { r / 5 + 40 };
            uit[i + 3] = 255;
        }
    }
    uit
}

/// Wacht tot `tot`, eerst slapend en de laatste twee milliseconden draaiend. `sleep`
/// alleen is op Windows te grof voor een raster van 16,7 ms.
fn wacht_tot(tot: Instant) {
    loop {
        let rest = tot.saturating_duration_since(Instant::now());
        if rest.is_zero() {
            return;
        }
        if rest > Duration::from_millis(2) {
            std::thread::sleep(rest - Duration::from_millis(2));
        } else {
            std::hint::spin_loop();
        }
    }
}

#[test]
#[ignore = "vereist een echt scherm en een GPU"]
fn gelijkmatige_bron_hoort_gelijkmatig_aan_te_komen() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .try_init();

    let seconden = env("HAPER_SECONDEN", 40);
    let fps = env("HAPER_FPS", 60) as u32;
    let bitrate = env("HAPER_BITRATE", 8_000_000) as u32;
    let codec = match std::env::var("HAPER_CODEC").as_deref() {
        Ok("h264") => Codec::H264,
        _ => Codec::Hevc,
    };
    let (b, h) = (breedte(), hoogte());
    println!(
        "{seconden}s, {b}x{h}, {fps} fps, {} Mbit, {codec:?}",
        bitrate / 1_000_000
    );

    let d3d = D3dContext::new().expect("D3D11");

    // --- de bron: een eigen venster dat op 60 Hz van inhoud wisselt -------------------
    let stop = Arc::new(AtomicBool::new(false));
    let bron_stop = stop.clone();
    let bron_d3d = d3d.clone();
    let bron_thread = std::thread::Builder::new()
        .name("bron".into())
        .spawn(move || {
            let (b, h) = (breedte(), hoogte());
            let texturen: Vec<_> = (0..FASEN)
                .map(|n| {
                    bron_d3d
                        .maak_textuur_met(b, h, &fase(n))
                        .expect("brontextuur")
                })
                .collect();
            let mut venster = Venster::open(&bron_d3d, BRON_TITEL, b, h).expect("bronvenster");

            let stap = Duration::from_nanos(1_000_000_000 / 60);
            let mut deadline = Instant::now();
            let mut n = 0usize;
            while !bron_stop.load(Ordering::Relaxed) {
                wacht_tot(deadline);
                deadline += stap;
                venster.toon(&texturen[n % FASEN]).expect("bron tonen");
                n += 1;
                if !venster.pomp() {
                    break;
                }
            }
            println!("bron: {n} beelden gepresenteerd");
        })
        .expect("bron-thread");

    // Het venster moet bestaan voordat het op te zoeken is. Op 1080p duurt het aanmaken
    // van de brontexturen een paar seconden, dus wachten tot hij er is in plaats van een
    // vaste pauze.
    let bron: Bron = (0..100)
        .find_map(|_| {
            std::thread::sleep(Duration::from_millis(200));
            beschikbare_bronnen()
                .ok()?
                .into_iter()
                .find(|b| b.soort == BronSoort::Venster && b.naam == BRON_TITEL)
        })
        .expect("het bronvenster moet in de lijst verschijnen");

    // --- de keten ---------------------------------------------------------------------
    let kijker = kijk(
        &d3d,
        KijkerConfig {
            stream_id: 1,
            titel: "FitCom — haperingstest".into(),
            breedte: b,
            hoogte: h,
            codec,
            afzender: IpAddr::V4(Ipv4Addr::LOCALHOST),
        },
    )
    .expect("kijker starten");
    let doel = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), kijker.poort);

    let deler = deel(
        &d3d,
        DelerConfig {
            stream_id: 1,
            bron,
            codec,
            fps,
            bitrate,
            voorbeeld: false,
        },
        vec![doel],
    )
    .expect("deler starten");

    // De kijker vraagt om een keyframe zodra hij aanhaakt; zonder motor eromheen moeten
    // wij dat doorgeven. Dit is precies wat `Engine::lees_kijkers` doet, op dezelfde tik.
    let tot = Instant::now() + Duration::from_secs(seconden);
    while Instant::now() < tot {
        std::thread::sleep(Duration::from_millis(100));
        while let Ok(ev) = kijker.events.try_recv() {
            if matches!(ev, fitcom_video::KijkerEvent::KeyframeNodig) {
                deler.vraag_keyframe();
            }
        }
    }

    let (getoond, kapot) = kijker.tellers();
    println!(
        "verstuurd {} / getoond {getoond} / kapot {kapot}",
        deler.beelden()
    );

    stop.store(true, Ordering::Relaxed);
    drop(deler);
    drop(kijker);
    std::thread::sleep(Duration::from_millis(500));
    let _ = bron_thread.join();

    assert!(getoond > 0, "er kwam geen enkel beeld aan");
}
