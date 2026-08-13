//! Screenshare door de echte motor heen, met echte QUIC-verbindingen over loopback.
//!
//! `streams.rs` bewijst dat de beslissingen kloppen en `fitcom-video` bewijst dat de
//! beeldketen werkt. Wat daartussen zit — de motor die de ene omzet in de andere — valt
//! daar precies tussenuit, en daar zit de bedrading die je met de hand alleen vindt door
//! twee vensters open te zetten en te hopen.
//!
//! Deze test heeft een GPU en een scherm nodig en opent kort een echt videovenster.
//! Draaien met:
//!
//! ```text
//! cargo test -p fitcom --test stream_deling -- --ignored --nocapture
//! ```

use fitcom::config::{Config, PeerConfig, VideoConfig};
use fitcom::engine::{self, EngineHandle, Snapshot, UiCommand};
use fitcom_net::{MeshConfig, PeerTarget};
use fitcom_proto::PeerId;
use fitcom_store::Store;
use std::sync::Arc;
use std::time::{Duration, Instant};

async fn vrije_poort() -> u16 {
    let s = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    s.local_addr().unwrap().port()
}

fn config(naam: &str, eigen: u16, ander: u16) -> Config {
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
        download_dir: None,
    }
}

fn start(id: PeerId, naam: &str, eigen: u16, ander: u16, dir: &std::path::Path) -> EngineHandle {
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
        config(naam, eigen, ander),
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
#[ignore = "vereist een echt scherm en een GPU, en opent kort een videovenster"]
async fn delen_en_kijken_via_de_motor() {
    let dir = std::env::temp_dir().join("fitcom-streamtest");
    std::fs::create_dir_all(&dir).unwrap();

    let (poort_a, poort_b) = (vrije_poort().await, vrije_poort().await);
    let (id_a, id_b) = (PeerId::new_random(), PeerId::new_random());
    let a = start(id_a, "deler", poort_a, poort_b, &dir);
    let b = start(id_b, "kijker", poort_b, poort_a, &dir);

    wacht(&b, "verbinding", |s| {
        s.peers.iter().any(|p| p.peer_id == Some(id_a))
    })
    .await;

    // De deler kondigt een scherm aan. Er hoort nog niets opgenomen te worden.
    let scherm = engine::deelbare_bronnen()
        .unwrap()
        .into_iter()
        .find(|br| br.soort == fitcom_video::BronSoort::Monitor)
        .expect("er moet een scherm zijn");
    println!("bron: {}", scherm.naam);
    a.commands.send(UiCommand::DeelBron(scherm)).await.unwrap();

    let snap = wacht(&a, "eigen stream", |s| !s.eigen_streams.is_empty()).await;
    assert_eq!(
        snap.eigen_streams[0].kijkers, 0,
        "er wordt opgenomen terwijl er niemand kijkt"
    );

    // De ander moet de aankondiging gezien hebben.
    let snap = wacht(&b, "aankondiging", |s| !s.streams.is_empty()).await;
    let stream = snap.streams[0].clone();
    assert_eq!(stream.eigenaar, id_a);
    assert!(!stream.kijken);
    println!(
        "aangekondigd: {} ({}×{})",
        stream.titel, stream.breedte, stream.hoogte
    );

    // Kijken. Nu pas hoort de deler te beginnen met opnemen.
    b.commands
        .send(UiCommand::Kijken(id_a, stream.stream_id))
        .await
        .unwrap();

    wacht(&a, "eerste kijker", |s| {
        s.eigen_streams.first().is_some_and(|e| e.kijkers == 1)
    })
    .await;
    wacht(&b, "kijkstatus", |s| {
        s.streams.first().is_some_and(|st| st.kijken)
    })
    .await;
    println!("deler neemt op, kijker kijkt");

    // Even laten lopen zodat er echt beeld overheen gaat.
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Stoppen met kijken moet het opnemen ook stoppen: dat is de hele reden dat je een
    // scherm gedeeld kunt laten staan zonder dat het iets kost.
    b.commands
        .send(UiCommand::StopKijken(id_a, stream.stream_id))
        .await
        .unwrap();

    wacht(&a, "laatste kijker weg", |s| {
        s.eigen_streams.first().is_some_and(|e| e.kijkers == 0)
    })
    .await;

    // En intrekken haalt hem bij de ander uit beeld.
    a.commands
        .send(UiCommand::StopDelen(stream.stream_id))
        .await
        .unwrap();
    wacht(&b, "ingetrokken", |s| s.streams.is_empty()).await;
    println!("ingetrokken en opgeruimd");
}
