#![cfg(windows)]

//! Hoe groot mag één beeld worden, en houdt de encoder zich aan het budget?
//!
//! De bitrate staat op 8 Mbit/s bij 60 beelden per seconde: gemiddeld 16 kB per beeld.
//! De vraag die deze test beantwoordt is wat het *grootste* beeld kost, want dat is het
//! beeld dat in één stoot de socket in gaat.
//!
//! ```text
//! cargo test -p fitcom-video --test piek -- --ignored --nocapture
//! $env:FITCOM_RC = "cbr"; cargo test -p fitcom-video --test piek -- --ignored --nocapture
//! ```

use fitcom_video::codec::HNS_PER_SEC;
use fitcom_video::{Codec, D3dContext, Encoder, EncoderConfig};
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;

const FPS: u32 = 60;
const BITRATE: u32 = 8_000_000;
const FASEN: usize = 12;

fn patroon(breedte: u32, hoogte: u32, fase: usize) -> Vec<u8> {
    let mut pixels = vec![0u8; (breedte * hoogte * 4) as usize];
    let balk = (breedte as usize / FASEN) * fase;
    for y in 0..hoogte as usize {
        for x in 0..breedte as usize {
            let i = (y * breedte as usize + x) * 4;
            let ruis = ((x * 7 + y * 13) % 256) as u8;
            let op_balk = x >= balk && x < balk + breedte as usize / FASEN;
            pixels[i] = if op_balk { 255 } else { ruis / 4 };
            pixels[i + 1] = if op_balk { 200 } else { ruis / 4 };
            pixels[i + 2] = if op_balk { 40 } else { ruis / 4 };
            pixels[i + 3] = 255;
        }
    }
    pixels
}

fn fasen(d3d: &D3dContext, breedte: u32, hoogte: u32) -> Vec<ID3D11Texture2D> {
    (0..FASEN)
        .map(|f| {
            d3d.maak_textuur_met(breedte, hoogte, &patroon(breedte, hoogte, f))
                .expect("textuur")
        })
        .collect()
}

fn meet(d3d: &D3dContext, codec: Codec, breedte: u32, hoogte: u32) {
    let beelden = fasen(d3d, breedte, hoogte);
    let mut encoder = Encoder::new(
        d3d,
        &EncoderConfig {
            codec,
            breedte,
            hoogte,
            fps: FPS,
            bitrate: BITRATE,
        },
    )
    .expect("encoder");

    let aantal = 360usize; // zes seconden op 60 fps: drie GOP's
    let (mut totaal, mut n, mut grootste_kf, mut grootste_p, mut keyframes) =
        (0usize, 0usize, 0usize, 0usize, 0usize);
    for i in 0..aantal {
        let tijd = (i as i64) * HNS_PER_SEC / i64::from(FPS);
        for p in encoder.encode(&beelden[i % FASEN], tijd).expect("coderen") {
            totaal += p.data.len();
            n += 1;
            if p.keyframe {
                keyframes += 1;
                grootste_kf = grootste_kf.max(p.data.len());
            } else {
                grootste_p = grootste_p.max(p.data.len());
            }
        }
    }
    let seconden = aantal as f64 / f64::from(FPS);
    let gemiddeld = totaal / n.max(1);
    println!(
        "{codec:?} {breedte}x{hoogte}: {:.1} Mbit/s (budget {:.0}), {keyframes} keyframes, \
         gemiddeld beeld {} kB, grootste P {} kB, grootste keyframe {} kB = {}x het gemiddelde, \
         {} UDP-fragmenten in één stoot",
        totaal as f64 * 8.0 / seconden / 1e6,
        f64::from(BITRATE) / 1e6,
        gemiddeld / 1024,
        grootste_p / 1024,
        grootste_kf / 1024,
        grootste_kf / gemiddeld.max(1),
        grootste_kf.div_ceil(1100),
    );
}

#[test]
#[ignore = "vereist een GPU met een hardware-encoder"]
fn hoe_groot_wordt_een_keyframe() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .try_init();
    let d3d = D3dContext::new().expect("D3D11");
    for (b, h) in [(1280u32, 720u32), (1920, 1080), (2560, 1440)] {
        meet(&d3d, Codec::Hevc, b, h);
        meet(&d3d, Codec::H264, b, h);
    }
}
