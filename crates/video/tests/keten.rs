//! De hele screenshare-keten in één stuk, over een echte UDP-socket.
//!
//! ```text
//! scherm ─► capture ─► encoder ─► fragmenten ─► UDP ─► samenstellen ─► decoder ─► venster
//! ```
//!
//! Dit heeft een GPU en een scherm nodig en staat daarom op `#[ignore]`. Draaien met:
//!
//! ```text
//! cargo test -p fitcom-video --test keten -- --ignored --nocapture
//! ```
//!
//! # Als debugmiddel
//!
//! Deler en kijker schrijven elk een meterregel per seconde; die komen hier op de
//! console. Dit is de snelste manier om aan screenshare te meten zonder tweede machine.
//! Instelbaar zonder opnieuw te bouwen:
//!
//! ```text
//! $env:KETEN_SECONDEN = 30; $env:KETEN_BITRATE = 8000000; $env:KETEN_FPS = 60
//! ```
//!
//! **Twee dingen die deze opstelling niet nadoet.** Het kijkvenster staat op hetzelfde
//! scherm dat gedeeld wordt, dus de kijker filmt zichzelf: dat geeft veel meer beweging
//! dan een echte deler ooit heeft, en dus een hogere bitrate. En loopback heeft geen
//! MTU van 1280, geen jitter en geen verlies op de lijn. Verlies dat je hier ziet komt
//! van buffers aan deze kant, niet van het netwerk.

use fitcom_video::capture::{afmeting_van, BronSoort};
use fitcom_video::{beschikbare_bronnen, deel, kijk, Codec, D3dContext, DelerConfig, KijkerConfig};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

fn env_u32(naam: &str, standaard: u32) -> u32 {
    std::env::var(naam)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(standaard)
}

#[test]
#[ignore = "vereist een echt scherm en een GPU"]
fn scherm_komt_via_udp_in_het_venster_terecht() {
    // Zonder abonnee gaan de meterregels van deler en kijker nergens heen.
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .try_init();

    let d3d = D3dContext::new().expect("D3D11");
    let scherm = beschikbare_bronnen()
        .expect("bronnen")
        .into_iter()
        .find(|b| b.soort == BronSoort::Monitor)
        .expect("er moet een scherm zijn");
    let (breedte, hoogte) = afmeting_van(&scherm).expect("afmeting");
    println!("bron: {} ({breedte}×{hoogte})", scherm.naam);

    // Eerst de kijker: in zijn abonnement staat de poort waarop hij luistert, en pas
    // daarna mag de deler beginnen. Precies de volgorde die de motor ook aanhoudt.
    let kijker = kijk(
        &d3d,
        KijkerConfig {
            stream_id: 1,
            titel: "FitCom — ketentest".into(),
            breedte,
            hoogte,
            codec: Codec::H264,
            afzender: IpAddr::V4(Ipv4Addr::LOCALHOST),
        },
    )
    .expect("kijker starten");

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
            // Geen terugblik: deze test meet de keten, en een miniatuur per halve
            // seconde ernaast zou meten wat er niet gemeten wordt.
            voorbeeld: false,
        },
        vec![doel],
    )
    .expect("deler starten");

    // Een paar seconden laten lopen, niet stoppen bij het eerste beeld: pas over meer
    // beelden zie je of de keten blijft draaien of na een paar frames vastloopt.
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

    // Het meetpunt uit de ROADMAP. Dit is opnemen → coderen → versturen → samenstellen
    // → decoderen → presenteren; wat de monitor er daarna zelf nog bij optelt zit er
    // niet in en is met software ook niet te meten. Valt dit tegen, dan is de encoder
    // omzetten naar directe NVENC de volgende stap — die zit al achter een smalle API.
    println!(
        "vertraging van opnemen tot tonen: {:?}",
        kijker.vertraging()
    );

    assert!(
        getoond > 0,
        "er kwam geen enkel beeld aan; de keten is ergens onderbroken"
    );
    // Een stilstaand bureaublad levert weinig beelden, maar helemaal stilvallen mag
    // niet: dat was precies de fout die de encoder eerder had.
    assert!(
        getoond >= 20,
        "maar {getoond} beelden in {seconden:.0} seconden; de keten valt stil"
    );
    assert!(
        kapot * 4 < getoond,
        "te veel beelden sneuvelen onderweg: {kapot} kapot tegen {getoond} goed"
    );
}
