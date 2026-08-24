//! Integratietests voor de mesh over loopback.
//!
//! Deze bestaan omdat de interessante gevallen — botsende verbindingen, wegvallen,
//! herverbinden — met de hand testen betekent: twee vensters openen, er eentje
//! afschieten en naar logs turen. Dat is te traag om elke wijziging mee te controleren.

use fitcom_net::{spawn, MeshCommand, MeshConfig, MeshEvent, MeshHandle, PeerStatus, PeerTarget};
use fitcom_proto::control::OpBroadcast;
use fitcom_proto::{Channel, ControlMsg, Op, OpKind, PeerId};
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
        targets: peer_ports
            .iter()
            .map(|&p| PeerTarget {
                address: "127.0.0.1".to_string(),
                label: format!("peer-{p}"),
                known_id: None,
                control_port: p,
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

    let a = spawn(config(ida, "A", pa, pb)).unwrap();
    let mut b = spawn(config(idb, "B", pb, pa)).unwrap();

    wait_for(&mut b, "eerste verbinding", 15, online).await;

    // A gaat weg. Dat is een normale toestand, geen fout: B moet blijven draaien.
    a.shutdown().await;
    wait_for(&mut b, "B merkt dat A weg is", 20, offline).await;

    // A komt terug op dezelfde poort met dezelfde identiteit. Niemand hoeft iets te doen.
    let _a2 = spawn(config(ida, "A", pa, pb)).unwrap();
    let terug = wait_for(&mut b, "B verbindt vanzelf opnieuw", 40, online).await;
    assert_eq!(terug, ida, "B moet dezelfde peer terugzien");
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
