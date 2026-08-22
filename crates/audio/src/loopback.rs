//! Losse bureaubladgeluid-tap voor de cliprecorder (fase 15).
//!
//! Zelfde proces-exclusieve WASAPI-loopback als het bureaubladgeluid van screenshare
//! (zie `session::wasapi_capture`), maar als zelfstandige tap met een eigen stopvlag:
//! clips draaien óók als er niet gedeeld wordt en niemand in gesprek is, dus ze kunnen
//! niet op de voice-sessie meeliften.
//!
//! De eigen stem van deze app zit er bewust niet bij — dezelfde exclude-modus als bij
//! het delen, om dezelfde reden: wie zijn eigen voice vertraagd terughoort in zijn clip
//! hoort een echo die er niet was.
//!
//! De chunks zijn interleaved f32 op [`crate::mix::SAMPLE_RATE`]; frames tellen is de
//! klok. Dat is sample-exact en loopt nooit uit de pas met wat de AAC-encoder er later
//! van maakt.

#[cfg(windows)]
use anyhow::{bail, Context, Result};
#[cfg(windows)]
use std::collections::VecDeque;
#[cfg(windows)]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(windows)]
use std::sync::Arc;
#[cfg(windows)]
use std::time::Duration;

/// Vast op stereo, om dezelfde reden als bij het delen: `WaveFormat` dwingt WASAPI via
/// `autoconvert` zelf tot deze indeling.
#[cfg(windows)]
pub const KANALEN: usize = 2;
#[cfg(windows)]
const BYTES_PER_FRAME: usize = KANALEN * 4; // 32-bit float per kanaal

/// Een lopende geluids-tap. Vallen laten stopt de capturedraad netjes.
#[cfg(windows)]
pub struct LoopbackTap {
    stop: Arc<AtomicBool>,
}

#[cfg(windows)]
impl LoopbackTap {
    /// Start de tap op zijn eigen draad en wacht tot bekend is of het gelukt is — een
    /// Windows-versie zonder application-loopback (vóór build 20348) faalt hier dus
    /// direct en leesbaar, in plaats van stilletjes clips zonder geluid op te leveren.
    pub fn start() -> Result<(Self, u32, std::sync::mpsc::Receiver<Vec<f32>>)> {
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = std::sync::mpsc::channel();
        let stop_draad = stop.clone();
        let (klaar_tx, klaar_rx) = std::sync::mpsc::sync_channel::<Result<()>>(1);

        std::thread::Builder::new()
            .name("fitcom-clip-geluid".into())
            .spawn(move || {
                let res = capture_lus(&stop_draad, &tx);
                let _ = klaar_tx.send(res);
            })
            .context("geluidsdraad voor clips starten")?;

        match klaar_rx.recv_timeout(Duration::from_secs(2)) {
            Ok(res) => {
                res?;
                Ok((Self { stop }, crate::mix::SAMPLE_RATE, rx))
            }
            Err(_) => bail!("geen antwoord van de geluidsdraad binnen 2 seconden"),
        }
    }
}

#[cfg(windows)]
impl Drop for LoopbackTap {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[cfg(windows)]
fn capture_lus(stop: &AtomicBool, tx: &std::sync::mpsc::Sender<Vec<f32>>) -> Result<()> {
    wasapi::initialize_mta().ok()?;
    let formaat = wasapi::WaveFormat::new(
        32,
        32,
        &wasapi::SampleType::Float,
        crate::mix::SAMPLE_RATE as usize,
        KANALEN,
        None,
    );
    // include_tree: false = alles behalve dit proces (en zijn kinderen).
    let mut client =
        wasapi::AudioClient::new_application_loopback_client(std::process::id(), false)
            .context("proces-exclusieve loopback niet beschikbaar")?;
    let modus = wasapi::StreamMode::EventsShared {
        autoconvert: true,
        buffer_duration_hns: 0,
    };
    client
        .initialize_client(&formaat, &wasapi::Direction::Capture, &modus)
        .context("wasapi-client initialiseren")?;
    let event = client.set_get_eventhandle().context("wasapi-eventhandle")?;
    let capture_client = client
        .get_audiocaptureclient()
        .context("wasapi-captureclient")?;
    client.start_stream().context("wasapi-stream starten")?;

    let mut bytes: VecDeque<u8> = VecDeque::new();
    while !stop.load(Ordering::Relaxed) {
        if tx.send(chunks_lezen(&capture_client, &mut bytes)).is_err() {
            break; // ontvanger weg: recorder gestopt
        }
        if event.wait_for_event(500).is_err() {
            break;
        }
    }
    let _ = client.stop_stream();
    Ok(())
}

/// Leest alle pakketten die er nu klaarstaan en geeft ze als één chunk terug.
#[cfg(windows)]
fn chunks_lezen(
    capture_client: &wasapi::AudioCaptureClient,
    bytes: &mut VecDeque<u8>,
) -> Vec<f32> {
    while matches!(capture_client.get_next_packet_size(), Ok(Some(n)) if n > 0) {
        if capture_client.read_from_device_to_deque(bytes).is_err() {
            break;
        }
    }
    let frames = bytes.len() / BYTES_PER_FRAME;
    let mut verweven = Vec::with_capacity(frames * KANALEN);
    for _ in 0..frames {
        let b0 = bytes.pop_front().unwrap_or(0);
        let b1 = bytes.pop_front().unwrap_or(0);
        let b2 = bytes.pop_front().unwrap_or(0);
        let b3 = bytes.pop_front().unwrap_or(0);
        verweven.push(f32::from_le_bytes([b0, b1, b2, b3]));
    }
    verweven
}
