//! Integratietests voor de mesh over loopback.
//!
//! Deze bestaan omdat de interessante gevallen — botsende verbindingen, wegvallen,
//! herverbinden — met de hand testen betekent: twee vensters openen, er eentje
//! afschieten en naar logs turen. Dat is te traag om elke wijziging mee te controleren.

use fitcom_net::{
    spawn, wegwerpsleutel, MediaSocket, MeshCommand, MeshConfig, MeshEvent, MeshHandle, PeerAdres,
    PeerStatus, PeerTarget,
};
use fitcom_proto::control::OpBroadcast;
use fitcom_proto::{Channel, ControlMsg, Op, OpKind, PeerId};
use ring::signature::KeyPair;
use std::time::Duration;
use tokio::time::timeout;

/// Zet `FITCOM_LOG=debug` om te zien wat de mesh doet tijdens een falende test.
fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("FITCOM_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("off")),
        )
        .with_test_writer()
        .try_init();
}

/// Een vrije poort claimen en meteen weer loslaten.
///
/// De uitgedeelde poorten worden onthouden: tests in dit binary draaien parallel, en
/// zonder dit kan het besturingssysteem tweemaal hetzelfde nummer teruggeven aan twee
/// tests tegelijk. Dat leverde precies één op de zoveel runs een onverklaarbare fout op.
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

fn config(me: PeerId, name: &str, my_port: u16, peer_port: u16) -> MeshConfig {
    config_multi(me, name, my_port, &[peer_port])
}

fn config_multi(me: PeerId, name: &str, my_port: u16, peer_ports: &[u16]) -> MeshConfig {
    MeshConfig {
        me,
        display_name: name.to_string(),
        control_port: my_port,
        media_port: 0,
        app_version: "0.1.0".to_string(),
        sleutel: wegwerpsleutel(),
        targets: peer_ports
            .iter()
            .map(|&p| PeerTarget {
                address: "127.0.0.1".to_string(),
                label: format!("peer-{p}"),
                known_id: None,
                control_port: p,
                public_key: None,
                kandidaten: Vec::new(),
            })
            .collect(),
    }
}

/// Wacht tot een gebeurtenis aan de voorwaarde voldoet, of faal na `secs`.
async fn wait_for<T>(
    h: &mut MeshHandle,
    what: &str,
    secs: u64,
    mut pred: impl FnMut(&MeshEvent) -> Option<T>,
) -> T {
    let found = timeout(Duration::from_secs(secs), async {
        while let Some(ev) = h.events.recv().await {
            if let Some(v) = pred(&ev) {
                return Some(v);
            }
        }
        None
    })
    .await;

    match found {
        Ok(Some(v)) => v,
        Ok(None) => panic!("kanaal sloot terwijl we wachtten op: {what}"),
        Err(_) => panic!("timeout na {secs}s bij wachten op: {what}"),
    }
}

fn online(ev: &MeshEvent) -> Option<PeerId> {
    match ev {
        MeshEvent::Status {
            status: PeerStatus::Online { peer_id, .. },
            ..
        } => Some(*peer_id),
        _ => None,
    }
}

fn offline(ev: &MeshEvent) -> Option<String> {
    match ev {
        MeshEvent::Status {
            status: PeerStatus::Offline { reason },
            ..
        } => Some(reason.clone()),
        _ => None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn twee_peers_vinden_elkaar_en_wisselen_berichten_uit() {
    init_tracing();
    let (pa, pb) = (free_port().await, free_port().await);
    let (ida, idb) = (PeerId::new_random(), PeerId::new_random());

    let mut a = spawn(config(ida, "A", pa, pb)).unwrap();
    let mut b = spawn(config(idb, "B", pb, pa)).unwrap();

    // Beide kanten dialen tegelijk. Er ontstaan twee verbindingen en de botsingsregel
    // moet er één overhouden — aan beide kanten dezelfde.
    let seen_by_a = wait_for(&mut a, "A ziet B online", 15, online).await;
    let seen_by_b = wait_for(&mut b, "B ziet A online", 15, online).await;
    assert_eq!(seen_by_a, idb);
    assert_eq!(seen_by_b, ida);

    let op = Op::new(
        ida,
        Channel::GENERAL,
        1,
        1,
        0,
        &OpKind::Post {
            body: "hallo vanaf A".into(),
            reply_to: None,
        },
    )
    .unwrap();

    // Herhaald versturen, niet één keer. Bij gelijktijdig dialen bestaan er heel even
    // twee verbindingen; de verliezer wordt gesloten en wat daar net op verstuurd is
    // gaat verloren. Dat is geen defect maar het ontwerp: ops zijn idempotent en
    // worden bij elke (her)verbinding opnieuw gesynchroniseerd. Deze lus doet hier
    // hetzelfde als de sync-laag straks in de app doet.
    let repeater = {
        let cmds = a.commands.clone();
        let op = op.clone();
        tokio::spawn(async move {
            loop {
                let msg = ControlMsg::OpBroadcast(OpBroadcast { op: op.clone() });
                if cmds.send(MeshCommand::Broadcast(msg)).await.is_err() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        })
    };

    let received = wait_for(&mut b, "B ontvangt de op van A", 10, |ev| match ev {
        MeshEvent::Message {
            from,
            msg: ControlMsg::OpBroadcast(bc),
        } => Some((*from, bc.op.clone())),
        _ => None,
    })
    .await;

    repeater.abort();
    assert_eq!(received.0, ida);
    assert_eq!(received.1, op);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn peer_die_wegvalt_komt_vanzelf_terug() {
    let (pa, pb) = (free_port().await, free_port().await);
    let (ida, idb) = (PeerId::new_random(), PeerId::new_random());

    let cfg_a = config(ida, "A", pa, pb);
    let a = spawn(cfg_a.clone()).unwrap();
    let mut b = spawn(config(idb, "B", pb, pa)).unwrap();

    wait_for(&mut b, "eerste verbinding", 15, online).await;

    // A gaat weg. Dat is een normale toestand, geen fout: B moet blijven draaien.
    a.shutdown().await;
    wait_for(&mut b, "B merkt dat A weg is", 20, offline).await;

    // A komt terug op dezelfde poort met dezelfde identiteit — en dus dezelfde sleutel, die
    // B inmiddels vastgepind heeft. Niemand hoeft iets te doen.
    let _a2 = spawn(cfg_a).unwrap();
    let terug = wait_for(&mut b, "B verbindt vanzelf opnieuw", 40, online).await;
    assert_eq!(terug, ida, "B moet dezelfde peer terugzien");
}

/// `shutdown` keert pas terug als de poort vrij is. Op één thread is dat deterministisch te
/// zien: quinn's endpoint-taak en onze sessietaken ruimen pas op als `shutdown` zelf wacht.
/// Met alleen `wait_idle` was de poort hier elke keer nog bezet; met vier threads liet de test
/// hierboven dat maar af en toe zien.
#[tokio::test(flavor = "current_thread")]
async fn shutdown_geeft_de_poort_vrij() {
    let (pa, pb) = (free_port().await, free_port().await);
    let a = spawn(config(PeerId::new_random(), "A", pa, pb)).unwrap();
    let mut b = spawn(config(PeerId::new_random(), "B", pb, pa)).unwrap();
    wait_for(&mut b, "verbinding", 15, online).await;

    a.shutdown().await;
    assert!(
        std::net::UdpSocket::bind(("0.0.0.0", pa)).is_ok(),
        "poort {pa} nog bezet na shutdown"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn drie_peers_vormen_een_volledige_mesh() {
    // De echte doeltopologie. Met twee peers blijft verborgen dat het koppelen van een
    // inkomende verbinding aan een geconfigureerde peer op méér dan het IP moet gaan:
    // hier delen alle drie hetzelfde loopback-adres, net als drie peers achter één NAT.
    init_tracing();
    let ports = [free_port().await, free_port().await, free_port().await];
    let ids: Vec<PeerId> = (0..3).map(|_| PeerId::new_random()).collect();

    let mut meshes: Vec<MeshHandle> = (0..3)
        .map(|i| {
            let others: Vec<u16> = (0..3).filter(|&j| j != i).map(|j| ports[j]).collect();
            spawn(config_multi(ids[i], &format!("peer{i}"), ports[i], &others)).unwrap()
        })
        .collect();

    // Elke peer moet de andere twee zien — en precies die twee, geen dubbelen.
    for (i, h) in meshes.iter_mut().enumerate() {
        let mut seen = std::collections::HashSet::new();
        let expected: std::collections::HashSet<PeerId> = ids
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(_, id)| *id)
            .collect();

        wait_for(h, &format!("peer{i} ziet beide anderen"), 25, |ev| {
            if let Some(id) = online(ev) {
                seen.insert(id);
            }
            (seen == expected).then_some(())
        })
        .await;
    }

    // En niemand mag ten onrechte concluderen dat er een andere peer achter een
    // bekend adres zit — dat was precies de fout die het IP-only matchen opleverde.
    for h in meshes.iter_mut() {
        while let Ok(ev) = h.events.try_recv() {
            if let MeshEvent::Status {
                status: PeerStatus::IdentityChanged { expected, got },
                ..
            } = ev
            {
                panic!("onterechte identiteitswissel gemeld: {expected:?} -> {got:?}");
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn abrupt_verdwijnen_wordt_ook_opgemerkt() {
    // Het realistische geval: stekker eruit, blauw scherm, proces gekild. Geen nette
    // afsluiting, dus B moet dit uit het uitblijven van verkeer afleiden.
    let (pa, pb) = (free_port().await, free_port().await);
    let (ida, idb) = (PeerId::new_random(), PeerId::new_random());

    let a = spawn(config(ida, "A", pa, pb)).unwrap();
    let mut b = spawn(config(idb, "B", pb, pa)).unwrap();

    wait_for(&mut b, "eerste verbinding", 15, online).await;

    drop(a);
    let reason = wait_for(&mut b, "B merkt het abrupte wegvallen", 30, offline).await;
    assert!(!reason.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn peer_zonder_tegenhanger_blijft_proberen_zonder_te_crashen() {
    // De normale situatie bij opstarten: jij bent er, de anderen nog niet.
    let (mine, dead) = (free_port().await, free_port().await);
    let mut a = spawn(config(PeerId::new_random(), "A", mine, dead)).unwrap();

    let reason = wait_for(&mut a, "offline-melding", 15, offline).await;
    assert!(!reason.is_empty(), "de UI moet iets kunnen tonen");

    // En blijft daarna gewoon leven.
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(!a.commands.is_closed(), "mesh mag niet omvallen");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn een_andere_sleutel_voor_dezelfde_peer_komt_er_niet_in() {
    // Klaar-als van fase A: B heeft de PeerId van A vastgepind met een sleutel die A niet
    // heeft. Wat A ook doet, bellen of gebeld worden, bij B komt hij niet online, en B
    // meldt waarom.
    init_tracing();
    let (pa, pb) = (free_port().await, free_port().await);
    let (ida, idb) = (PeerId::new_random(), PeerId::new_random());

    let mut a = spawn(config(ida, "A", pa, pb)).unwrap();
    let mut cfg_b = config(idb, "B", pb, pa);
    cfg_b.targets[0].known_id = Some(ida);
    cfg_b.targets[0].public_key = Some([0x42; 32]);
    let mut b = spawn(cfg_b).unwrap();

    let found = timeout(Duration::from_secs(12), async {
        while let Some(ev) = b.events.recv().await {
            match ev {
                MeshEvent::Status {
                    status: PeerStatus::Online { .. },
                    ..
                } => return Err("B liet A met een andere sleutel binnen"),
                MeshEvent::Status {
                    status: PeerStatus::IdentityChanged { expected, got },
                    ..
                } => {
                    assert_eq!((expected, got), (ida, ida));
                    return Ok(());
                }
                MeshEvent::LearnedIdentity { .. } => {
                    return Err("B leerde een sleutel over een pin heen")
                }
                _ => {}
            }
        }
        Err("kanaal dicht")
    })
    .await
    .expect("B had de identiteitswissel moeten melden");
    found.unwrap();

    // En ook daarna niet, terwijl A blijft proberen.
    let later = timeout(Duration::from_secs(4), async {
        while let Some(ev) = b.events.recv().await {
            if online(&ev).is_some() {
                return true;
            }
        }
        false
    })
    .await;
    assert!(!matches!(later, Ok(true)), "A kwam alsnog online");

    // A heeft B intussen wél geleerd en laat hem binnen; B gooit hem er bij elke poging
    // meteen weer uit. Dat mag geen belstorm worden (B-40): telde zo'n afgewezen sessie als
    // sessie, dan begon B's backoff steeds opnieuw bij één seconde en zag A elke seconde
    // een volledige online/offline-ronde.
    let mut rondes = 0;
    let _ = timeout(Duration::from_secs(10), async {
        // Alleen de overgangen naar online: zolang een verbinding staat stuurt de mesh elke
        // seconde een verse RTT mee, en dat is geen nieuwe ronde.
        let mut was_online = false;
        while let Some(ev) = a.events.recv().await {
            if let MeshEvent::Status { status, .. } = ev {
                let nu = matches!(status, PeerStatus::Online { .. });
                rondes += usize::from(nu && !was_online);
                was_online = nu;
            }
        }
    })
    .await;
    // A's events stonden sinds de start in de rij, dus dit telt de hele test (~14 s). Met
    // een oplopende backoff belt B op 0, 1, 3, 7 en 15 s; zonder belde hij elke seconde.
    eprintln!("A zag B {rondes} keer online");
    assert!(
        rondes <= 6,
        "A zag B {rondes} keer online in ~14 s: B belt te vaak"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn verbonden_peers_delen_mediasleutels_en_niemand_anders() {
    // De mesh-actor zet bij verbinden de sleutels per richting, uit de exporter van de
    // verbinding en de sessie-index die elke kant in zijn Hello zette. Klopt daar één kant
    // niet van, dan gaat er geen enkel pakket open; dat zie je alleen met twee echte meshes.
    init_tracing();
    let (pa, pb) = (free_port().await, free_port().await);
    let (ida, idb) = (PeerId::new_random(), PeerId::new_random());
    let mut a = spawn(config(ida, "A", pa, pb)).unwrap();
    let mut b = spawn(config(idb, "B", pb, pa)).unwrap();
    wait_for(&mut a, "A ziet B", 15, online).await;
    wait_for(&mut b, "B ziet A", 15, online).await;
    // Na een botsing hoort de winnende verbinding aan beide kanten even te landen.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Via de mediasocket van elke mesh (fase B5): A verstuurt vanaf de zijne, B luistert op de
    // zijne. Een losse socket ernaast zou na de eerste keepalive omzeild worden, want dan gaat
    // alles naar het geleerde adres van B's mediasocket.
    let (post_a, post_b) = (a.post.clone(), b.post.clone());
    let uitkomst = tokio::task::spawn_blocking(move || {
        let zender = post_a.zender().unwrap();
        let rij = post_b.luister_geluid();
        let naar_b = PeerAdres {
            id: idb,
            addr: std::net::SocketAddr::from(([127, 0, 0, 1], post_b.poort())),
        };
        let header = fitcom_proto::MediaHeader {
            stream_id: 0,
            seq: 1,
            timestamp: 0,
            payload_type: fitcom_proto::PayloadType::OPUS,
            flags: 0,
            frag_index: 0,
        };
        zender.stuur(naar_b, &header, b"hoi").unwrap();
        let ontvangen = rij
            .recv_timeout(Duration::from_secs(2))
            .ok()
            .map(|p| (p.van, p.header.seq, p.payload));

        // Een derde zonder verbinding heeft geen sleutel voor B, en wat hij met een eigen
        // tabel stuurt gaat bij B niet open.
        let vreemd = fitcom_net::MediaSleutels::default();
        fitcom_net::MediaSleutels::koppel((&vreemd, PeerId::new_random()), (&vreemd, idb));
        let indringer = MediaSocket::bind(0, vreemd).unwrap();
        indringer.stuur(naar_b, &header, b"nep").unwrap();
        let geweigerd = rij.recv_timeout(Duration::from_millis(300)).is_err();
        (ontvangen, geweigerd)
    })
    .await
    .unwrap();

    assert_eq!(uitkomst.0, Some((ida, 1, b"hoi".to_vec())));
    assert!(uitkomst.1, "een pakket met een vreemde sleutel ging open");
}
/// Het LAN-adres van deze pc (geen pakket: alleen de route kiezen), of `None` zonder netwerk.
fn lan_ip() -> Option<std::net::IpAddr> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?;
    let ip = s.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
}

/// Een peer die alleen uit een uitnodigingscode komt: geen tailnet-adres, wel sleutel,
/// `PeerId` en kandidaten.
fn uit_code(id: PeerId, sleutel: [u8; 32], kandidaten: Vec<std::net::SocketAddr>) -> PeerTarget {
    PeerTarget {
        address: String::new(),
        label: "code".into(),
        known_id: Some(id),
        control_port: 0,
        public_key: Some(sleutel),
        kandidaten,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn twee_peers_verbinden_op_alleen_elkaars_code_buiten_het_tailnet() {
    // Fase B: geen tailnet. Het adres is een LAN-adres, waar nooit een sleutel geleerd wordt;
    // binnenkomen kan dus alleen met de sleutel uit de code. De eerste kandidaat is een dood
    // adres: gestaffeld bellen moet de tweede vinden.
    init_tracing();
    let Some(ip) = lan_ip() else {
        eprintln!("geen netwerk; overgeslagen");
        return;
    };
    let (pa, pb) = (free_port().await, free_port().await);
    let (ida, idb) = (PeerId::new_random(), PeerId::new_random());
    let (ka, kb) = (wegwerpsleutel(), wegwerpsleutel());
    let sleutel = |k: &ring::signature::Ed25519KeyPair| -> [u8; 32] {
        k.public_key().as_ref().try_into().unwrap()
    };
    let dood = std::net::SocketAddr::new(ip, free_port().await);
    let mut cfg_a = config_multi(ida, "A", pa, &[]);
    cfg_a.sleutel = ka.clone();
    cfg_a.targets = vec![uit_code(
        idb,
        sleutel(&kb),
        vec![dood, std::net::SocketAddr::new(ip, pb)],
    )];
    let mut cfg_b = config_multi(idb, "B", pb, &[]);
    cfg_b.sleutel = kb.clone();
    cfg_b.targets = vec![uit_code(
        ida,
        sleutel(&ka),
        vec![std::net::SocketAddr::new(ip, pa)],
    )];

    let mut a = spawn(cfg_a).unwrap();
    let mut b = spawn(cfg_b).unwrap();
    assert_eq!(wait_for(&mut a, "A ziet B", 15, online).await, idb);
    assert_eq!(wait_for(&mut b, "B ziet A", 15, online).await, ida);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn twee_werkende_kandidaten_tegelijk_geven_een_verbinding_die_blijft() {
    // B3: beide kanten bellen elkaar tegelijk, elk op twee adressen die allebei werken
    // (loopback en het LAN-adres). Per poging gaat er maar één `Hello` uit, en de
    // botsingsregel kiest daarna één verbinding; die moet blijven staan.
    init_tracing();
    let Some(ip) = lan_ip() else {
        eprintln!("geen netwerk; overgeslagen");
        return;
    };
    let (pa, pb) = (free_port().await, free_port().await);
    let (ida, idb) = (PeerId::new_random(), PeerId::new_random());
    let (ka, kb) = (wegwerpsleutel(), wegwerpsleutel());
    let sleutel = |k: &ring::signature::Ed25519KeyPair| -> [u8; 32] {
        k.public_key().as_ref().try_into().unwrap()
    };
    let beide = |p: u16| {
        vec![
            std::net::SocketAddr::from(([127, 0, 0, 1], p)),
            std::net::SocketAddr::new(ip, p),
        ]
    };
    let mut cfg_a = config_multi(ida, "A", pa, &[]);
    cfg_a.sleutel = ka.clone();
    cfg_a.targets = vec![uit_code(idb, sleutel(&kb), beide(pb))];
    let mut cfg_b = config_multi(idb, "B", pb, &[]);
    cfg_b.sleutel = kb.clone();
    cfg_b.targets = vec![uit_code(ida, sleutel(&ka), beide(pa))];

    let mut a = spawn(cfg_a).unwrap();
    let mut b = spawn(cfg_b).unwrap();
    wait_for(&mut a, "A ziet B", 15, online).await;
    wait_for(&mut b, "B ziet A", 15, online).await;
    // Tijdens de botsing kan een kant heel even offline gaan: de verliezer wordt gesloten
    // voordat de winnaar daar geïnstalleerd is. Dat kon met één adres ook al. Daarna moet er
    // aan beide kanten één verbinding staan die blijft.
    for h in [&mut a, &mut b] {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let mut laatst = None;
        while let Ok(ev) = h.events.try_recv() {
            if let MeshEvent::Status { status, .. } = ev {
                laatst = Some(status);
            }
        }
        if let Some(s) = laatst {
            assert!(
                matches!(s, PeerStatus::Online { .. }),
                "na de botsing: {s:?}"
            );
        }
    }
    let rust = timeout(Duration::from_secs(4), async {
        loop {
            tokio::select! {
                Some(ev) = a.events.recv() => if let Some(r) = offline(&ev) { return r },
                Some(ev) = b.events.recv() => if let Some(r) = offline(&ev) { return r },
            }
        }
    })
    .await;
    assert!(rust.is_err(), "verbinding viel weg na de botsing: {rust:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn een_peer_uit_een_code_erbij_terwijl_de_mesh_draait() {
    // B1: beide starten zonder peers; daarna plakt elk de code van de ander.
    init_tracing();
    let Some(ip) = lan_ip() else {
        eprintln!("geen netwerk; overgeslagen");
        return;
    };
    let (pa, pb) = (free_port().await, free_port().await);
    let (ida, idb) = (PeerId::new_random(), PeerId::new_random());
    let mut cfg_a = config_multi(ida, "A", pa, &[]);
    let mut cfg_b = config_multi(idb, "B", pb, &[]);
    cfg_a.targets.clear();
    cfg_b.targets.clear();
    let mut a = spawn(cfg_a).unwrap();
    let mut b = spawn(cfg_b).unwrap();
    for (h, id, sleutel, poort) in [(&a, idb, b.public_key, pb), (&b, ida, a.public_key, pa)] {
        h.commands
            .send(MeshCommand::VoegToe(uit_code(
                id,
                sleutel,
                vec![std::net::SocketAddr::new(ip, poort)],
            )))
            .await
            .unwrap();
    }
    assert_eq!(wait_for(&mut a, "A ziet B", 15, online).await, idb);
    assert_eq!(wait_for(&mut b, "B ziet A", 15, online).await, ida);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn een_kandidaat_die_snel_opneemt_maar_niets_wordt_houdt_de_rest_niet_tegen() {
    // De eerste kandidaat is A's eigen poort (een oud LAN-adres dat nu de eigen pc is): die
    // haalt QUIC in een milliseconde en sluit dan stil. B belt A niet terug. Toch moet A via
    // de tweede kandidaat bij B komen, en niet elke ronde opnieuw op zichzelf stuklopen.
    init_tracing();
    let Some(ip) = lan_ip() else {
        eprintln!("geen netwerk; overgeslagen");
        return;
    };
    let (pa, pb) = (free_port().await, free_port().await);
    let (ida, idb) = (PeerId::new_random(), PeerId::new_random());
    let (ka, kb) = (wegwerpsleutel(), wegwerpsleutel());
    let sleutel = |k: &ring::signature::Ed25519KeyPair| -> [u8; 32] {
        k.public_key().as_ref().try_into().unwrap()
    };
    let mut cfg_a = config_multi(ida, "A", pa, &[]);
    cfg_a.sleutel = ka.clone();
    cfg_a.targets = vec![uit_code(
        idb,
        sleutel(&kb),
        vec![
            std::net::SocketAddr::new(ip, pa),
            std::net::SocketAddr::new(ip, pb),
        ],
    )];
    let mut cfg_b = config_multi(idb, "B", pb, &[]);
    cfg_b.sleutel = kb.clone();
    // B kent A wel (anders sluit hij stil), maar heeft geen adres om hem te bellen.
    cfg_b.targets = vec![uit_code(ida, sleutel(&ka), Vec::new())];

    let mut a = spawn(cfg_a).unwrap();
    let _b = spawn(cfg_b).unwrap();
    assert_eq!(wait_for(&mut a, "A ziet B", 10, online).await, idb);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn een_ander_op_het_configadres_blijft_een_alarm() {
    // Fase A: op het (tailnet-)adres uit de config neemt een andere installatie op, met een
    // andere `PeerId` en sleutel (een herinstallatie). Dat moet "andere identiteit" blijven
    // en niet stil "verbinden..." worden, ook nu een kandidaat die niet klopt gewoon wordt
    // overgeslagen.
    init_tracing();
    let (pa, pc) = (free_port().await, free_port().await);
    let (ida, idb, idc) = (
        PeerId::new_random(),
        PeerId::new_random(),
        PeerId::new_random(),
    );
    let mut cfg_a = config(ida, "A", pa, pc);
    cfg_a.targets[0].known_id = Some(idb);
    cfg_a.targets[0].public_key = Some([7; 32]);
    let mut a = spawn(cfg_a).unwrap();
    let _c = spawn(config(idc, "C", pc, pa)).unwrap();
    let (expected, got) = wait_for(&mut a, "A meldt een andere identiteit", 10, |ev| match ev {
        MeshEvent::Status {
            status: PeerStatus::IdentityChanged { expected, got },
            ..
        } => Some((*expected, *got)),
        _ => None,
    })
    .await;
    assert_eq!((expected, got), (idb, idc));
}
