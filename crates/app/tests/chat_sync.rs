//! De koppeling tussen oplog en mesh, met echte QUIC-verbindingen over loopback.
//!
//! `fitcom-store` bewijst dat de synchronisatie-logica convergeert; `fitcom-net` bewijst
//! dat de verbindingen werken. Wat daartussen zit — wie stuurt wanneer welk bericht naar
//! wie — valt daar precies tussenuit, en dat is nou net waar een lus of een gemiste
//! inhaalslag ontstaat.

use fitcom::chat::Chat;
use fitcom_net::{MeshCommand, MeshConfig, MeshEvent, MeshHandle, PeerStatus, PeerTarget};
use fitcom_proto::PeerId;
use fitcom_store::Store;
use std::time::{Duration, Instant};

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

fn config(me: PeerId, naam: &str, eigen: u16, ander: u16) -> MeshConfig {
    MeshConfig {
        me,
        display_name: naam.to_string(),
        control_port: eigen,
        media_port: 0,
        targets: vec![PeerTarget {
            address: "127.0.0.1".to_string(),
            label: naam.to_string(),
            known_id: None,
            control_port: ander,
        }],
    }
}

/// Eén peer: mesh plus chat, met dezelfde bedrading als de UI gebruikt.
struct Peer {
    mesh: MeshHandle,
    chat: Chat,
    verbonden: Vec<PeerId>,
}

impl Peer {
    fn nieuw(id: PeerId, naam: &str, eigen: u16, ander: u16) -> Self {
        Self {
            mesh: fitcom_net::spawn(config(id, naam, eigen, ander)).unwrap(),
            chat: Chat::new(Store::open_in_memory(id).unwrap()).unwrap(),
            verbonden: Vec::new(),
        }
    }

    /// Precies wat `ui::App::drain_events` doet, zonder venster.
    fn pomp(&mut self) {
        let mut net_online = Vec::new();

        while let Ok(ev) = self.mesh.events.try_recv() {
            match ev {
                MeshEvent::Status {
                    status: PeerStatus::Online { peer_id, .. },
                    ..
                } => {
                    if !self.verbonden.contains(&peer_id) {
                        self.verbonden.push(peer_id);
                        net_online.push(peer_id);
                    }
                }
                MeshEvent::Status { .. }
                | MeshEvent::LearnedIdentity { .. }
                | MeshEvent::IncomingFileStream { .. } => {}
                MeshEvent::Message { from, msg } => {
                    let cmds = self.chat.bij_bericht(from, msg).unwrap();
                    self.stuur(cmds);
                }
            }
        }

        for peer in net_online {
            let cmds = self.chat.bij_verbinding(peer).unwrap();
            self.stuur(cmds);
        }

        let verbonden = self.verbonden.clone();
        let cmds = self.chat.tick(&verbonden).unwrap();
        self.stuur(cmds);

        self.chat.refresh();
    }

    fn stuur(&self, cmds: Vec<MeshCommand>) {
        for c in cmds {
            self.mesh.commands.try_send(c).unwrap();
        }
    }

    fn zeg(&mut self, tekst: &str) {
        let cmds = self.chat.plaats_bericht(tekst).unwrap();
        self.stuur(cmds);
        self.chat.refresh();
    }

    fn berichten(&mut self) -> Vec<String> {
        self.chat.refresh();
        self.chat
            .timeline()
            .messages
            .iter()
            .map(|m| m.body.clone())
            .collect()
    }
}

/// Draait alle peers tot de voorwaarde klopt, of faalt na `secs`.
async fn tot(
    peers: &mut [&mut Peer],
    wat: &str,
    secs: u64,
    mut klaar: impl FnMut(&mut [&mut Peer]) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        for p in peers.iter_mut() {
            p.pomp();
        }
        if klaar(peers) {
            return;
        }
        if Instant::now() > deadline {
            panic!("timeout na {secs}s bij wachten op: {wat}");
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bericht_komt_live_aan_bij_de_ander() {
    let (pa, pb) = (free_port().await, free_port().await);
    let mut a = Peer::nieuw(PeerId::new_random(), "A", pa, pb);
    let mut b = Peer::nieuw(PeerId::new_random(), "B", pb, pa);

    tot(&mut [&mut a, &mut b], "verbinding", 20, |p| {
        !p[0].verbonden.is_empty() && !p[1].verbonden.is_empty()
    })
    .await;

    a.zeg("hallo B");
    tot(&mut [&mut a, &mut b], "B ziet het bericht", 20, |p| {
        p[1].berichten() == ["hallo B"]
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn wie_later_opstart_haalt_de_gemiste_berichten_op() {
    // Het scenario waar de hele fase om draait: iemand was er niet, komt terug, en
    // hoeft niets te doen om bij te zijn.
    let (pa, pb) = (free_port().await, free_port().await);
    let mut a = Peer::nieuw(PeerId::new_random(), "A", pa, pb);

    // B bestaat nog niet. A praat tegen niemand.
    for i in 0..5 {
        a.zeg(&format!("bericht {i}"));
        a.pomp();
    }

    let mut b = Peer::nieuw(PeerId::new_random(), "B", pb, pa);

    tot(
        &mut [&mut a, &mut b],
        "B haalt de geschiedenis in",
        30,
        |p| p[1].berichten().len() == 5,
    )
    .await;

    assert_eq!(a.berichten(), b.berichten());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn doorsturen_zingt_niet_eindeloos_rond() {
    // Ontvangen ops worden doorgestuurd zodat een peer die maar één van de twee
    // anderen kan bereiken toch alles krijgt. Dat mag geen lus worden: een op die we
    // al kenden hoort nergens meer heen te gaan.
    let (pa, pb) = (free_port().await, free_port().await);
    let mut a = Peer::nieuw(PeerId::new_random(), "A", pa, pb);
    let mut b = Peer::nieuw(PeerId::new_random(), "B", pb, pa);

    tot(&mut [&mut a, &mut b], "verbinding", 20, |p| {
        !p[0].verbonden.is_empty() && !p[1].verbonden.is_empty()
    })
    .await;

    a.zeg("eenmalig");
    tot(&mut [&mut a, &mut b], "B ziet het bericht", 20, |p| {
        p[1].berichten() == ["eenmalig"]
    })
    .await;

    // Een lus zou zich hier verraden: de oplog blijft groeien of het bericht wordt
    // dubbel getoond.
    for _ in 0..40 {
        a.pomp();
        b.pomp();
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    assert_eq!(a.berichten(), ["eenmalig"]);
    assert_eq!(b.berichten(), ["eenmalig"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bewerken_en_verwijderen_komen_ook_over() {
    let (pa, pb) = (free_port().await, free_port().await);
    let mut a = Peer::nieuw(PeerId::new_random(), "A", pa, pb);
    let mut b = Peer::nieuw(PeerId::new_random(), "B", pb, pa);

    tot(&mut [&mut a, &mut b], "verbinding", 20, |p| {
        !p[0].verbonden.is_empty() && !p[1].verbonden.is_empty()
    })
    .await;

    a.zeg("typfuot");
    a.zeg("weg hiermee");
    tot(&mut [&mut a, &mut b], "beide berichten bij B", 20, |p| {
        p[1].berichten().len() == 2
    })
    .await;

    let ids: Vec<_> = a.chat.timeline().messages.iter().map(|m| m.id).collect();
    let cmds = a.chat.bewerk_bericht(ids[0], "typfout").unwrap();
    a.stuur(cmds);
    let cmds = a.chat.verwijder_bericht(ids[1]).unwrap();
    a.stuur(cmds);

    tot(
        &mut [&mut a, &mut b],
        "B verwerkt de wijzigingen",
        20,
        |p| p[1].berichten() == ["typfout"],
    )
    .await;

    assert_eq!(a.berichten(), ["typfout"]);
}
