//! De macOS-versie van de ketentest uit `tests/keten.rs`, als voorbeeldprogramma.
//!
//! ```text
//! scherm ─► SCK ─► VT-encoder ─► fragmenten ─► UDP ─► samenstellen ─► VT-decoder ─► venster
//! ```
//!
//! Waarom geen `#[test]`: het kijkvenster op macOS heeft de main-runloop nodig (AppKit
//! is main-thread-only), en de testharnas van cargo pompt die niet. Dit programma runt
//! de keten op een werkthread — exact zoals de app dat doet — en pompt zelf de runloop.
//!
//! ```text
//! cargo run -p fitcom-video --example mac_keten
//! KETEN_SECONDEN=10 cargo run -p fitcom-video --example mac_keten
//! ```
//!
//! Vereist de Screen-Recording-permissie. Zelfde kanttekeningen als de Windows-test:
//! het venster filmt zichzelf (meer beweging dan echt) en loopback heeft geen verlies.

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("dit voorbeeld is alleen voor macOS");
}

#[cfg(target_os = "macos")]
fn main() {
    use fitcom_video::capture::{afmeting_van, BronSoort};
    use fitcom_video::{
        beschikbare_bronnen, deel, kijk, Codec, D3dContext, DelerConfig, KijkerConfig,
    };
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    fn env_u32(naam: &str, standaard: u32) -> u32 {
        std::env::var(naam)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(standaard)
    }

    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .try_init();

    // AppKit klaarzetten vóór er om vensters gevraagd wordt.
    let mtm = objc2::MainThreadMarker::new().expect("main() draait op de main thread");
    let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(objc2_app_kit::NSApplicationActivationPolicy::Regular);

    let klaar = Arc::new(AtomicBool::new(false));
    let klaar_werk = klaar.clone();
    let mislukt = Arc::new(AtomicBool::new(false));
    let mislukt_werk = mislukt.clone();

    // De hele keten op een werkthread, zoals de motor dat ook doet.
    std::thread::spawn(move || {
        let uitkomst = (|| -> anyhow::Result<()> {
            let d3d = D3dContext::new()?;
            let scherm = beschikbare_bronnen()?
                .into_iter()
                .find(|b| b.soort == BronSoort::Monitor)
                .ok_or_else(|| anyhow::anyhow!("er moet een scherm zijn"))?;
            let (breedte, hoogte) = afmeting_van(&scherm)?;
            println!("bron: {} ({breedte}×{hoogte})", scherm.naam);

            // Eerst de kijker: in zijn abonnement staat de poort waarop hij luistert.
            let kijker = kijk(
                &d3d,
                KijkerConfig {
                    stream_id: 1,
                    titel: "FitCom — mac-ketentest".into(),
                    breedte,
                    hoogte,
                    codec: Codec::H264,
                    afzender: IpAddr::V4(Ipv4Addr::LOCALHOST),
                },
            )?;
            let doel = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), kijker.poort);
            println!("kijker luistert op {doel}");

            let deler = deel(
                &d3d,
                DelerConfig {
                    stream_id: 1,
                    bron: scherm,
                    codec: Codec::H264,
                    fps: env_u32("KETEN_FPS", 60),
                    bitrate: env_u32("KETEN_BITRATE", 25_000_000),
                    voorbeeld: true,
                },
                vec![doel],
            )?;

            let duur = Duration::from_secs(u64::from(env_u32("KETEN_SECONDEN", 5)));
            let tot = Instant::now() + duur;
            while Instant::now() < tot {
                std::thread::sleep(Duration::from_millis(100));
            }

            let (getoond, kapot) = kijker.tellers();
            let seconden = duur.as_secs_f64();
            println!(
                "verstuurd: {} beelden, getoond: {getoond} ({:.0}/s), onderweg gesneuveld: {kapot}",
                deler.beelden(),
                getoond as f64 / seconden
            );
            println!(
                "vertraging van opnemen tot tonen: {:?}",
                kijker.vertraging()
            );

            anyhow::ensure!(getoond >= 20, "maar {getoond} beelden; de keten valt stil");

            // De terugblik op wat je zelf deelt: dezelfde miniatuur als de tegel in de
            // streamstrook. Er moet er een liggen, en hij mag niet leeg (zwart) zijn.
            let mini = deler
                .miniatuur()
                .ok_or_else(|| anyhow::anyhow!("de deler legde geen eigen miniatuur neer"))?;
            println!("eigen miniatuur: {}×{}", mini.breedte, mini.hoogte);
            anyhow::ensure!(
                mini.data.len() == (mini.breedte * mini.hoogte * 4) as usize,
                "miniatuur heeft {} bytes voor {}×{}",
                mini.data.len(),
                mini.breedte,
                mini.hoogte
            );
            anyhow::ensure!(
                mini.data
                    .chunks_exact(4)
                    .any(|px| px[0] > 16 || px[1] > 16 || px[2] > 16),
                "de eigen miniatuur is helemaal zwart"
            );
            anyhow::ensure!(
                kapot * 4 < getoond,
                "te veel kapot onderweg: {kapot} tegen {getoond}"
            );
            Ok(())
        })();

        if let Err(e) = uitkomst {
            eprintln!("KETEN MISLUKT: {e:#}");
            mislukt_werk.store(true, Ordering::Relaxed);
        } else {
            println!("KETEN GESLAAGD");
        }
        klaar_werk.store(true, Ordering::Relaxed);
    });

    // De main-runloop pompen tot de werkthread klaar is; dit is wat Tauri in de echte
    // app doet.
    let lus = objc2_foundation::NSRunLoop::mainRunLoop();
    while !klaar.load(Ordering::Relaxed) {
        lus.runUntilDate(&objc2_foundation::NSDate::dateWithTimeIntervalSinceNow(
            0.05,
        ));
    }
    std::process::exit(if mislukt.load(Ordering::Relaxed) {
        1
    } else {
        0
    });
}
