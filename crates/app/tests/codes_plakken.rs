//! Uitnodigingscodes door de echte motor heen (fase B1): twee installaties zonder peers plakken
//! elkaars code, verbinden, en vinden elkaar na een herstart terug uit `config.toml` alleen.
//!
//! Over het LAN-adres van deze pc en niet over loopback: op loopback mag een sleutel ook zonder
//! code geleerd worden (trust-on-first-use), op een LAN-adres nooit. Zo bewijst dit dat de
//! sleutel echt uit de code komt.

use fitcom::config::{ClipsConfig, Config, VideoConfig};
use fitcom::engine::{self, EngineHandle, Snapshot, UiCommand};
use fitcom_net::{MeshConfig, PeerStatus, PeerTarget};
use fitcom_proto::PeerId;
use fitcom_store::Store;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn lan_ip() -> Option<std::net::IpAddr> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?;
    let ip = s.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
}

fn vrije_poort() -> u16 {
    std::net::UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn config(naam: &str, poort: u16, downloads: &Path) -> Config {
    Config {
        display_name: naam.to_string(),
        control_port: poort,
        media_port: 0,
        bind_address: fitcom::config::ALLE_INTERFACES.to_string(),
        port_mapping: false,
        minimize_to_tray: false,
        autostart: false,
        input_device: None,
        output_device: None,
        peers: Vec::new(),
        video: VideoConfig::default(),
        sound: Default::default(),
        clips: ClipsConfig::default(),
        push_to_talk: Default::default(),
        share_activity: false,
        allow_drawing: true,
        allow_remote_control: false,
        auto_popout: false,
        overlay: Default::default(),
        download_dir: Some(downloads.to_path_buf()),
    }
}

/// Start zoals `main.rs` dat doet: de targets uit de peers van de config, met hun code-adressen.
/// Probeert een paar seconden als de poort van een vorige start nog niet vrij is.
async fn start(
    id: PeerId,
    sleutel: Arc<ring::signature::Ed25519KeyPair>,
    cfg: Config,
    dir: &Path,
) -> EngineHandle {
    let targets = cfg
        .peers
        .iter()
        .map(|p| PeerTarget {
            address: p.address.clone(),
            label: p.label.clone(),
            known_id: p.known_id,
            control_port: p.control_port,
            public_key: p.vastgepinde_sleutel(),
            kandidaten: p.addresses.clone(),
        })
        .collect();
    let mesh_cfg = MeshConfig {
        me: id,
        display_name: cfg.display_name.clone(),
        control_port: cfg.control_port,
        media_port: 0,
        app_version: "0.1.0".to_string(),
        sleutel,
        port_mapping: false,
        targets,
    };
    let tot = Instant::now() + Duration::from_secs(8);
    let mesh = loop {
        match fitcom_net::spawn(mesh_cfg.clone()) {
            Ok(m) => break m,
            Err(_) if Instant::now() < tot => tokio::time::sleep(Duration::from_millis(100)).await,
            Err(e) => panic!("mesh starten: {e:#}"),
        }
    };
    engine::spawn(
        mesh,
        Store::open_in_memory(id).unwrap(),
        cfg,
        dir.join("config.toml"),
    )
    .unwrap()
}

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
    panic!("{wat}: niet binnen 15 s");
}

fn online(snap: &Snapshot, peer: PeerId) -> bool {
    snap.peers
        .iter()
        .any(|p| matches!(p.status, PeerStatus::Online { peer_id, .. } if peer_id == peer))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn twee_installaties_verbinden_op_elkaars_code_en_na_een_herstart_weer() {
    let Some(ip) = lan_ip() else {
        eprintln!("geen netwerk; overgeslagen");
        return;
    };
    let basis = std::env::temp_dir().join(format!("fitcom-codes-{}", PeerId::new_random()));
    let (dir_a, dir_b) = (basis.join("a"), basis.join("b"));
    for d in [&dir_a, &dir_b] {
        std::fs::create_dir_all(d.join("downloads")).unwrap();
    }
    let (ida, idb) = (PeerId::new_random(), PeerId::new_random());
    let (ka, kb) = (fitcom_net::wegwerpsleutel(), fitcom_net::wegwerpsleutel());
    let (pa, pb) = (vrije_poort(), vrije_poort());

    let a = start(
        ida,
        ka.clone(),
        config("A", pa, &dir_a.join("downloads")),
        &dir_a,
    )
    .await;
    let b = start(
        idb,
        kb.clone(),
        config("B", pb, &dir_b.join("downloads")),
        &dir_b,
    )
    .await;
    let code = |s: &Snapshot| !s.uitnodiging.is_empty() && s.uitnodiging.len() > 100;
    let code_a = wacht(&a, "code van A", code).await.uitnodiging.clone();
    let code_b = wacht(&b, "code van B", code).await.uitnodiging.clone();
    assert!(
        fitcom_proto::uitnodiging::Uitnodiging::lees(&code_a)
            .unwrap()
            .adressen
            .contains(&std::net::SocketAddr::new(ip, pa)),
        "de code van A noemt zijn LAN-adres"
    );

    // Zoals uit een groepsapp: tekst eromheen.
    a.commands
        .send(UiCommand::VoegCodesToe(format!(
            "hier is de mijne: {code_b} groet"
        )))
        .await
        .unwrap();
    b.commands
        .send(UiCommand::VoegCodesToe(code_a))
        .await
        .unwrap();
    let snap = wacht(&a, "A ziet B", |s| online(s, idb)).await;
    let uitslag = snap.codes_uitslag.clone().unwrap();
    assert_eq!(uitslag.toegevoegd, vec!["B".to_string()]);
    wacht(&b, "B ziet A", |s| online(s, ida)).await;

    // Wat er in de config van A staat.
    let opgeslagen =
        Config::load_or_create(&dir_a.join("config.toml")).expect("config van A is te lezen");
    assert_eq!(opgeslagen.peers.len(), 1);
    assert_eq!(opgeslagen.peers[0].known_id, Some(idb));
    assert!(opgeslagen.peers[0].vastgepinde_sleutel().is_some());

    // Beide herstarten, uit hun eigen config, zonder nieuwe code.
    drop(a);
    drop(b);
    let cfg_a = Config::load_or_create(&dir_a.join("config.toml")).unwrap();
    let cfg_b = Config::load_or_create(&dir_b.join("config.toml")).unwrap();
    let a = start(ida, ka, cfg_a, &dir_a).await;
    let b = start(idb, kb, cfg_b, &dir_b).await;
    wacht(&a, "A ziet B na de herstart", |s| online(s, idb)).await;
    wacht(&b, "B ziet A na de herstart", |s| online(s, ida)).await;
    let _ = std::fs::remove_dir_all(&basis);
}
