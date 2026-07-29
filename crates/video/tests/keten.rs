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

use fitcom_video::capture::{afmeting_van, BronSoort};
use fitcom_video::{beschikbare_bronnen, deel, kijk, Codec, D3dContext, DelerConfig, KijkerConfig};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

#[test]
#[ignore = "vereist een echt scherm en een GPU"]
fn scherm_komt_via_udp_in_het_venster_terecht() {
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
            fps: 60,
            bitrate: 25_000_000,
        },
        vec![doel],
    )
    .expect("deler starten");

    // Een paar seconden laten lopen, niet stoppen bij het eerste beeld: pas over meer
    // beelden zie je of de keten blijft draaien of na een paar frames vastloopt.
    let duur = Duration::from_secs(5);
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
