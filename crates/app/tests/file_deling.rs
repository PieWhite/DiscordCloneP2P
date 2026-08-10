//! Bestandsoverdracht door de echte motor heen, met echte QUIC-verbindingen over loopback.
//!
//! `files.rs` bewijst dat de beslissingen kloppen (wie krijgt welk antwoord, wanneer
//! start een upload). Wat daartussen zit — de bytes daadwerkelijk over een eigen
//! QUIC-stream sturen, hashen en op de juiste plek op schijf zetten — valt daar precies
//! tussenuit. Anders dan `stream_deling.rs` is hier geen GPU of scherm voor nodig, dus
//! dit draait gewoon mee met `cargo test`.

use fitcom::config::{Config, PeerConfig, VideoConfig};
use fitcom::engine::{self, EngineHandle, Snapshot, UiCommand};
use fitcom_net::{MeshConfig, PeerTarget};
use fitcom_proto::{Channel, OpId, PeerId};
use fitcom_store::Store;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

async fn free_port() -> u16 {
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};
    static UITGEDEELD: OnceLock<Mutex<HashSet<u16>>> = OnceLock::new();
    let uitgedeeld = UITGEDEELD.get_or_init(Default::default);

    for _ in 0..100 {
        let s = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let p = s.local_addr().unwrap().port();
        if uitgedeeld.lock().unwrap().insert(p) {
            return p;
        }
    }
    panic!("geen vrije poort gevonden");
}

fn config(naam: &str, eigen: u16, ander: u16, downloads: &Path) -> Config {
    Config {
        display_name: naam.to_string(),
        control_port: eigen,
        media_port: 0,
        minimize_to_tray: false,
        autostart: false,
        input_device: None,
        output_device: None,
        peers: vec![PeerConfig {
            address: "127.0.0.1".into(),
            label: "ander".into(),
            known_id: None,
            control_port: ander,
        }],
        video: VideoConfig::default(),
        sound: Default::default(),
        download_dir: Some(downloads.to_path_buf()),
    }
}

fn start(
    id: PeerId,
    naam: &str,
    eigen: u16,
    ander: u16,
    dir: &Path,
    downloads: &Path,
) -> EngineHandle {
    let mesh = fitcom_net::spawn(MeshConfig {
        me: id,
        display_name: naam.to_string(),
        control_port: eigen,
        media_port: 0,
        app_version: "0.1.0".to_string(),
        targets: vec![PeerTarget {
            address: "127.0.0.1".into(),
            label: "ander".into(),
            known_id: None,
            control_port: ander,
        }],
    })
    .unwrap();

    engine::spawn(
        mesh,
        Store::open_in_memory(id).unwrap(),
        config(naam, eigen, ander, downloads),
        dir.join(format!("{naam}.toml")),
    )
    .unwrap()
}

/// Wacht tot de momentopname aan een voorwaarde voldoet. Levert hem op, of niets als
/// het binnen de tijd niet gebeurt.
async fn wacht(
    handle: &EngineHandle,
    wat: &str,
    voorwaarde: impl Fn(&Snapshot) -> bool,
) -> Arc<Snapshot> {
    let tot = Instant::now() + Duration::from_secs(15);
    while Instant::now() < tot {
        let snap = handle.snapshot.borrow().clone();
        if voorwaarde(&snap) {
            return snap;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("wachten op '{wat}' liep af");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn aanbieden_en_downloaden_via_de_motor() {
    let dir = tempdir();
    let (poort_a, poort_b) = (free_port().await, free_port().await);
    let (id_a, id_b) = (PeerId::new_random(), PeerId::new_random());

    let downloads_a = dir.join("downloads-a");
    let downloads_b = dir.join("downloads-b");
    let a = start(id_a, "aanbieder", poort_a, poort_b, &dir, &downloads_a);
    let b = start(id_b, "downloader", poort_b, poort_a, &dir, &downloads_b);

    wacht(&b, "verbinding", |s| {
        s.peers.iter().any(|p| p.peer_id == Some(id_a))
    })
    .await;

    // Een bestand met wat inhoud om te delen. Groot genoeg om over meerdere leesbuffers
    // (64 KiB) te lopen, klein genoeg om de test snel te houden.
    let bron = dir.join("vakantiefotos.zip");
    let inhoud = testinhoud(200_000);
    std::fs::write(&bron, &inhoud).unwrap();

    a.commands
        .send(UiCommand::BiedBestandAan(bron, Channel::GENERAL))
        .await
        .unwrap();

    let snap = wacht(&a, "eigen aanbod", |s| !s.files.is_empty()).await;
    assert!(snap.files[0].is_mine);
    assert_eq!(snap.files[0].name, "vakantiefotos.zip");
    assert_eq!(snap.files[0].size, inhoud.len() as u64);

    // De aanbieding is een gewone op en moet dus vanzelf bij B aankomen, ook zonder dat
    // iemand iets downloadt.
    let snap = wacht(&b, "aanbieding gezien", |s| !s.files.is_empty()).await;
    let file: OpId = snap.files[0].id;
    assert!(!snap.files[0].is_mine);
    assert_eq!(snap.files[0].size, inhoud.len() as u64);

    b.commands
        .send(UiCommand::DownloadBestand(file))
        .await
        .unwrap();

    wacht(&b, "download voltooid", |s| {
        matches!(
            s.files
                .iter()
                .find(|f| f.id == file)
                .and_then(|f| f.status.as_ref()),
            Some(fitcom::files::DownloadStatus::Voltooid)
        )
    })
    .await;

    let gedownload = std::fs::read(downloads_b.join("vakantiefotos.zip"))
        .expect("het gedownloade bestand moet onder zijn oorspronkelijke naam staan");
    assert_eq!(gedownload, inhoud, "de bytes moeten exact overeenkomen");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn downloaden_van_een_ingetrokken_bestand_geeft_een_nette_fout() {
    // De aanbieder heeft het bestand nooit lokaal gehad onder dit id — simuleert een
    // aanbod waarvan het originele bestand intussen weg is. Geen crash, een duidelijke
    // status.
    let dir = tempdir();
    let (poort_a, poort_b) = (free_port().await, free_port().await);
    let (id_a, id_b) = (PeerId::new_random(), PeerId::new_random());

    let downloads_a = dir.join("downloads-a");
    let downloads_b = dir.join("downloads-b");
    let a = start(id_a, "aanbieder", poort_a, poort_b, &dir, &downloads_a);
    let b = start(id_b, "downloader", poort_b, poort_a, &dir, &downloads_b);

    wacht(&b, "verbinding", |s| {
        s.peers.iter().any(|p| p.peer_id == Some(id_a))
    })
    .await;

    let bron = dir.join("weer-weg.bin");
    std::fs::write(&bron, testinhoud(1_000)).unwrap();
    a.commands
        .send(UiCommand::BiedBestandAan(bron.clone(), Channel::GENERAL))
        .await
        .unwrap();
    let snap = wacht(&b, "aanbieding gezien", |s| !s.files.is_empty()).await;
    let file = snap.files[0].id;

    // Het bronbestand verdwijnt voordat er iemand om vraagt.
    std::fs::remove_file(&bron).unwrap();

    b.commands
        .send(UiCommand::DownloadBestand(file))
        .await
        .unwrap();

    wacht(&b, "download mislukt", |s| {
        matches!(
            s.files
                .iter()
                .find(|f| f.id == file)
                .and_then(|f| f.status.as_ref()),
            Some(fitcom::files::DownloadStatus::Mislukt(_))
        )
    })
    .await;
}

fn testinhoud(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

fn tempdir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fitcom-filetest-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
