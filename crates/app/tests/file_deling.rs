//! Bestandsoverdracht door de echte motor heen, met echte QUIC-verbindingen over loopback.
//!
//! `files.rs` bewijst dat de beslissingen kloppen (wie krijgt welk antwoord, wanneer
//! start een upload). Wat daartussen zit — de bytes daadwerkelijk over een eigen
//! QUIC-stream sturen, hashen en op de juiste plek op schijf zetten — valt daar precies
//! tussenuit. Anders dan `stream_deling.rs` is hier geen GPU of scherm voor nodig, dus
//! dit draait gewoon mee met `cargo test`.

use fitcom::config::{ClipsConfig, Config, PeerConfig, VideoConfig};
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
        bind_address: fitcom::config::ALLE_INTERFACES.to_string(),
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
        clips: ClipsConfig::default(),
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

/// Zelfde motor, maar met een oplog op schijf, zodat een tweede start dezelfde
/// aanbiedingen terugziet. Alleen nodig voor de herstart-test.
fn start_op_schijf(
    id: PeerId,
    naam: &str,
    eigen: u16,
    dir: &Path,
    downloads: &Path,
) -> EngineHandle {
    let mesh = fitcom_net::spawn(MeshConfig {
        me: id,
        display_name: naam.to_string(),
        control_port: eigen,
        media_port: 0,
        app_version: "0.1.0".to_string(),
        targets: vec![],
    })
    .unwrap();

    engine::spawn(
        mesh,
        Store::open(&dir.join("chat.sqlite"), id).unwrap(),
        Config {
            peers: vec![],
            ..config(naam, eigen, eigen, downloads)
        },
        dir.join("config.toml"),
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

    // Waar het beland is, is wat de openknop in de tijdlijn nodig heeft. Zonder dit pad
    // wordt de downloadknop nooit een openknop — en het is niet uit `name` te herleiden,
    // want bij een tweede download van dezelfde naam wordt het `naam (2).zip`.
    let snap = b.snapshot.borrow().clone();
    let view = snap.files.iter().find(|f| f.id == file).unwrap();
    assert_eq!(
        view.local_path.as_deref(),
        Some(downloads_b.join("vakantiefotos.zip").as_path()),
        "de motor moet weten waar de download staat"
    );

    // Bij de aanbieder wijst hetzelfde veld naar zijn eigen bestand; die hoeft niets te
    // downloaden om het te kunnen openen.
    let snap = a.snapshot.borrow().clone();
    assert_eq!(
        snap.files[0].local_path.as_deref(),
        Some(dir.join("vakantiefotos.zip").as_path())
    );
}

/// Het pad waar de openknop op staat, moet een herstart overleven — anders is een bestand
/// van gisteren morgen weer alleen een downloadknop, en kan een andere peer een bestand
/// dat jij aanbood niet meer ophalen zodra jij de app één keer hebt herstart.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn het_pad_van_een_aanbod_overleeft_een_herstart() {
    let dir = tempdir();
    let downloads = dir.join("downloads");
    let id = PeerId::new_random();

    let bron = dir.join("notulen.pdf");
    std::fs::write(&bron, testinhoud(1_000)).unwrap();

    {
        let e = start_op_schijf(id, "solo", free_port().await, &dir, &downloads);
        e.commands
            .send(UiCommand::BiedBestandAan(bron.clone(), Channel::GENERAL))
            .await
            .unwrap();
        wacht(&e, "eigen aanbod", |s| {
            s.files
                .first()
                .and_then(|f| f.local_path.as_ref())
                .is_some()
        })
        .await;
    }
    assert!(
        dir.join("bestandspaden.json").exists(),
        "het aanbod hoort op schijf te staan"
    );

    // Nieuwe motor, zelfde datamap. Andere poort: de vorige socket kan nog even blijven
    // hangen, en dat heeft niets met deze vraag te maken.
    let e = start_op_schijf(id, "solo", free_port().await, &dir, &downloads);
    let snap = wacht(&e, "aanbod terug", |s| !s.files.is_empty()).await;
    assert_eq!(
        snap.files[0].local_path.as_deref(),
        Some(bron.as_path()),
        "na een herstart moet het pad er nog zijn"
    );

    // En een pad dat niet meer bestaat valt af, in plaats van een knop die niets doet.
    drop(e);
    std::fs::remove_file(&bron).unwrap();
    let e = start_op_schijf(id, "solo", free_port().await, &dir, &downloads);
    let snap = wacht(&e, "aanbod terug", |s| !s.files.is_empty()).await;
    assert_eq!(snap.files[0].local_path, None);
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

/// Een afbeelding gaat een andere weg dan een gewoon bestand: hij landt onder zijn
/// inhoudshash in `<downloadmap>/Pictures`, aan *beide* kanten op precies hetzelfde pad,
/// zodat de kaart in de tijdlijn bij de aanbieder én bij de ontvanger een miniatuur kan
/// laden. En hij haalt zichzelf op, zonder klik.
///
/// Deze weg had geen enkele test, en dat is precies waar het misging: de afbeeldingenmap
/// stond in de datamap en het halve bestand in de downloadmap, dus bij iedereen die zijn
/// downloadmap naar een andere schijf verzet had, mislukte de laatste stap — `rename` kan
/// niet over een schijfgrens. Zie `docs/OVERDRACHT.md` beslissing 32.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn een_afbeelding_landt_bij_beiden_op_hetzelfde_pad_in_de_downloadmap() {
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

    // De inhoud doet niet mee: niets in de motor decodeert een afbeelding, het is de
    // extensie die de weg bepaalt (`files::is_afbeelding`).
    let bron = dir.join("schermafdruk.png");
    let inhoud = testinhoud(120_000);
    std::fs::write(&bron, &inhoud).unwrap();

    a.commands
        .send(UiCommand::BiedBestandAan(bron.clone(), Channel::GENERAL))
        .await
        .unwrap();

    let snap = wacht(&a, "eigen aanbod", |s| !s.files.is_empty()).await;
    let aanbod = snap.files[0].clone();
    let naam = fitcom::files::hash_bestandsnaam(&aanbod.hash, &aanbod.name);

    // Bij de aanbieder: een eigen kopie onder de hash, en het origineel ongemoeid.
    assert_eq!(
        aanbod.local_path.as_deref(),
        Some(downloads_a.join("Pictures").join(&naam).as_path()),
        "de aanbieder hoort zijn kopie in <downloadmap>/Pictures te hebben"
    );
    assert!(
        bron.exists(),
        "het bestand van de gebruiker blijft waar het is"
    );

    // Bij de ontvanger: niemand klikt, de afbeelding haalt zichzelf op.
    wacht(&b, "afbeelding vanzelf binnen", |s| {
        matches!(
            s.files
                .iter()
                .find(|f| f.id == aanbod.id)
                .and_then(|f| f.status.as_ref()),
            Some(fitcom::files::DownloadStatus::Voltooid)
        )
    })
    .await;

    let bij_b = downloads_b.join("Pictures").join(&naam);
    assert_eq!(
        std::fs::read(&bij_b).expect("de afbeelding hoort onder zijn hash te staan"),
        inhoud
    );
    assert_eq!(
        std::fs::read_dir(&downloads_b)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.path().is_file())
            .count(),
        0,
        "een afbeelding hoort niet los in de downloadmap te belanden, ook geen .part"
    );
    // Hetzelfde pad aan beide kanten, op de map na — dat is wat de miniatuur mogelijk maakt.
    assert_eq!(bij_b.file_name(), Some(std::ffi::OsStr::new(&naam)));
}

/// De eenmalige verhuizing bij het starten: tot 2026-08-20 stonden de afbeeldingen in de
/// datamap. Ze horen bij de eerste start onder de downloadmap terecht te komen, want hun
/// pad wordt afgeleid en niet onthouden — laten staan is uit de tijdlijn verdwijnen.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn afbeeldingen_uit_de_oude_map_verhuizen_bij_het_starten() {
    let dir = tempdir();
    let downloads = dir.join("downloads");

    // Zoals een 1.3.0-installatie het achterlaat: <datamap>/Pictures.
    let oud = dir.join("Pictures");
    std::fs::create_dir_all(&oud).unwrap();
    let naam = format!("{}.png", "ab".repeat(32));
    std::fs::write(oud.join(&naam), testinhoud(64)).unwrap();

    let e = start_op_schijf(
        PeerId::new_random(),
        "solo",
        free_port().await,
        &dir,
        &downloads,
    );
    // De momentopname vertelt het venster waar hij ze moet zoeken; dat is dezelfde map.
    let snap = wacht(&e, "afbeeldingenmap in de momentopname", |s| {
        s.pictures_dir == downloads.join("Pictures")
    })
    .await;
    assert_eq!(snap.pictures_dir, downloads.join("Pictures"));

    assert_eq!(
        std::fs::read(downloads.join("Pictures").join(&naam)).unwrap(),
        testinhoud(64),
        "de afbeelding hoort mee verhuisd te zijn"
    );
    assert!(!oud.exists(), "de oude map hoort opgeruimd te zijn");
}

fn testinhoud(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

fn tempdir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fitcom-filetest-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
