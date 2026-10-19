//! Meting voor fase B2: wat kost port mapping in rust? Een mesh zonder peers, met of zonder
//! port mapping, een vaste tijd lang. Processortijd en wekmomenten meet je van buitenaf
//! (zie `docs/OVERDRACHT.md` beslissing 47); dit programma meldt alleen wat de router zei.
//!
//! ```text
//! cargo run --release -p fitcom-net --example poortmeting -- aan 120 42670
//! cargo run --release -p fitcom-net --example poortmeting -- uit 120 42670
//! ```
//!
//! Vraagt de echte router om de poort en de poort+1, en geeft ze bij het afsluiten vrij.

use fitcom_net::{spawn, wegwerpsleutel, MeshConfig, MeshEvent};
use fitcom_proto::PeerId;
use std::time::Duration;

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("info,quinn=warn")
        .try_init();
    let mut args = std::env::args().skip(1);
    let aan = args.next().as_deref() == Some("aan");
    let seconden: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(120);
    let poort: u16 = args.next().and_then(|s| s.parse().ok()).unwrap_or(42670);
    println!(
        "pid {} port mapping {}",
        std::process::id(),
        if aan { "aan" } else { "uit" }
    );

    let mut mesh = spawn(MeshConfig {
        me: PeerId::new_random(),
        display_name: "meting".into(),
        control_port: poort,
        media_port: poort + 1,
        targets: Vec::new(),
        app_version: "0.0.0".into(),
        sleutel: wegwerpsleutel(),
        port_mapping: aan,
    })
    .expect("mesh starten");
    let sluiter = mesh.sluiter();
    let einde = tokio::time::sleep(Duration::from_secs(seconden));
    tokio::pin!(einde);
    loop {
        tokio::select! {
            _ = &mut einde => break,
            ev = mesh.events.recv() => match ev {
                Some(MeshEvent::Bereik(b)) => println!("router: {b}"),
                Some(MeshEvent::EigenAdressen(a)) => println!("eigen adressen: {a:?}"),
                Some(_) => {}
                None => break,
            },
        }
    }
    let begin = std::time::Instant::now();
    sluiter.sluit(Duration::from_millis(500)).await;
    println!("afgesloten in {:?}", begin.elapsed());
}
