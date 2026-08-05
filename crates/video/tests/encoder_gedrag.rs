#![cfg(windows)]

//! Wat de encoder doet als je hem precies weet wat je hem voert.
//!
//! De ketentest deelt het echte scherm, en dat maakt hem ongeschikt om aan te meten: een
//! stilstaand bureaublad levert vier beelden per seconde en een bewegend spel tweehonderd,
//! dus twee runs zijn nooit met elkaar te vergelijken. Hier gaat er een vast patroon in
//! op een vast tempo, en dan zeggen de getallen die eruit komen wel iets.
//!
//! Heeft een GPU nodig, maar geen scherm en geen venster.
//!
//! ```text
//! cargo test -p fitcom-video --test encoder_gedrag -- --ignored --nocapture
//! ```

use fitcom_video::codec::HNS_PER_SEC;
use fitcom_video::{Codec, D3dContext, Encoder, EncoderConfig};
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;

const BREEDTE: u32 = 1920;
const HOOGTE: u32 = 1080;
const FPS: u32 = 60;
const BITRATE: u32 = 8_000_000;
/// Genoeg fasen dat de encoder elk beeld echt iets ziet veranderen, weinig genoeg dat we
/// er geen halve gigabyte videogeheugen aan kwijt zijn.
const FASEN: usize = 12;

/// Een balk die over het beeld schuift, met ruis eronder zodat er ook echt iets te
/// coderen valt. Een egaal vlak comprimeert tot niets en meet dus niets.
fn patroon(fase: usize) -> Vec<u8> {
    let mut pixels = vec![0u8; (BREEDTE * HOOGTE * 4) as usize];
    let balk = (BREEDTE as usize / FASEN) * fase;
    for y in 0..HOOGTE as usize {
        for x in 0..BREEDTE as usize {
            let i = (y * BREEDTE as usize + x) * 4;
            let ruis = ((x * 7 + y * 13) % 256) as u8;
            let op_balk = x >= balk && x < balk + BREEDTE as usize / FASEN;
            pixels[i] = if op_balk { 255 } else { ruis / 4 };
            pixels[i + 1] = if op_balk { 200 } else { ruis / 4 };
            pixels[i + 2] = if op_balk { 40 } else { ruis / 4 };
            pixels[i + 3] = 255;
        }
    }
    pixels
}

fn fasen(d3d: &D3dContext) -> Vec<ID3D11Texture2D> {
    (0..FASEN)
        .map(|f| {
            d3d.maak_textuur_met(BREEDTE, HOOGTE, &patroon(f))
                .expect("textuur")
        })
        .collect()
}

/// Codeert `aantal` beelden en telt wat eruit komt.
fn meet(d3d: &D3dContext, codec: Codec, aantal: usize) -> (usize, usize, usize, usize) {
    let beelden = fasen(d3d);
    let mut encoder = Encoder::new(
        d3d,
        &EncoderConfig {
            codec,
            breedte: BREEDTE,
            hoogte: HOOGTE,
            fps: FPS,
            bitrate: BITRATE,
        },
    )
    .expect("encoder");

    let (mut keyframes, mut pakketten, mut bytes, mut grootste) = (0usize, 0usize, 0usize, 0usize);
    for n in 0..aantal {
        let tijd = (n as i64) * HNS_PER_SEC / i64::from(FPS);
        for p in encoder
            .encode(&beelden[n % FASEN], tijd)
            .expect("beeld coderen")
        {
            pakketten += 1;
            bytes += p.data.len();
            grootste = grootste.max(p.data.len());
            keyframes += usize::from(p.keyframe);
        }
    }
    (keyframes, pakketten, bytes, grootste)
}

#[test]
#[ignore = "vereist een GPU met een hardware-encoder"]
fn keyframe_afstand_komt_uit_de_config_en_niet_van_de_driver() {
    // Dit is de meting die het keyframe-onderzoek beslist. De driver telt zijn GOP in
    // beelden; wij willen weten dat wij hem in seconden vastzetten. Tien seconden bij 60
    // fps is één keyframe per 600 beelden.
    //
    // Beide codecs, want het zijn twee losse transforms en niets garandeert dat de
    // NVIDIA HEVC-MFT dezelfde instelling aanneemt als de H.264-MFT. Rick's eigen config
    // stond op HEVC toen de haperingen gemeten werden.
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .try_init();

    let d3d = D3dContext::new().expect("D3D11");
    let aantal = 900usize; // vijftien seconden op 60 fps
    let seconden = aantal as f64 / f64::from(FPS);

    for codec in [Codec::H264, Codec::Hevc] {
        let (keyframes, pakketten, bytes, grootste) = meet(&d3d, codec, aantal);
        println!(
            "{}: {pakketten} beelden, {keyframes} keyframes ({:.2}/s), {:.1} Mbit/s op een budget van {:.1}, grootste beeld {} kB",
            codec.naam(),
            keyframes as f64 / seconden,
            bytes as f64 * 8.0 / seconden / 1e6,
            f64::from(BITRATE) / 1e6,
            grootste / 1024
        );

        // Vijftien seconden op één keyframe per tien: die van de start plus één. Meer dan
        // drie betekent dat de driver zijn eigen afstand aanhoudt, en dan komt de stoot
        // van honderden kilobytes vaker dan afgesproken.
        assert!(
            (1..=3).contains(&keyframes),
            "{}: {keyframes} keyframes in {seconden:.0} seconden; verwacht er 2",
            codec.naam()
        );
    }
}

#[test]
#[ignore = "vereist een GPU met een hardware-encoder"]
fn de_encoder_houdt_hoogstens_een_beeld_vast() {
    // Waar de gevoelde vertraging vandaan komt, los van het netwerk. Een hardware-MFT is
    // een pijplijn: je stopt beeld N erin en krijgt beeld N-k eruit. Die k is vertraging
    // die je nergens meer inhaalt, en hij telt in *beelden* — op een rustig scherm dat
    // maar 5 beelden per seconde levert is k=2 dus 400 ms.
    //
    // Dit is ook de reden dat `deler.rs` de opnamewachtrij leegtrekt en alleen het
    // verste beeld codeert: alles wat daarachter aan staat is al te laat.
    let d3d = D3dContext::new().expect("D3D11");
    let beelden = fasen(&d3d);

    let mut encoder = Encoder::new(
        &d3d,
        &EncoderConfig {
            codec: Codec::H264,
            breedte: BREEDTE,
            hoogte: HOOGTE,
            fps: FPS,
            bitrate: BITRATE,
        },
    )
    .expect("encoder");

    let stap = HNS_PER_SEC / i64::from(FPS);
    let mut achterstand = Vec::new();
    for n in 0..180i64 {
        let tijd = n * stap;
        for p in encoder
            .encode(&beelden[n as usize % FASEN], tijd)
            .expect("beeld coderen")
        {
            achterstand.push((tijd - p.tijd_hns) / stap);
        }
    }

    let grootste = achterstand.iter().copied().max().unwrap_or(0);
    let gemiddeld = achterstand.iter().sum::<i64>() as f64 / achterstand.len() as f64;
    println!(
        "pijplijn: gemiddeld {gemiddeld:.2} beeld achterstand, grootste {grootste} \
         (op {FPS} fps is dat {:.0} ms gemiddeld)",
        gemiddeld * 1000.0 / f64::from(FPS)
    );

    assert!(
        grootste <= 2,
        "de encoder houdt {grootste} beelden vast; dat is vertraging die nergens meer \
         in te halen is"
    );
}
