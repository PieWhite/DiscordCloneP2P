//! Microfoon-tap voor de clips (fase 15).
//!
//! Los van de voice-sessie opgezet, want clips draaien ook als er geen gesprek is en
//! ook als je gemutet staat tegenover de anderen: wat in de clip hoort is wat jij zei
//! tijdens het gamen, niet wat er verstuurd werd.
//!
//! De chunks komen als 48 kHz STEREO f32 uit deze tap — dezelfde vorm als wat de
//! loopback levert — zodat de menger in `fitcom_video::opname` geen tweede
//! samplerate-hoofd hoeft te zijn. Het apparaat levert wat hij levert; naar-mono,
//! hersampling en dupliceren gebeuren allemaal hier, in de driver-callback.

#[cfg(windows)]
use anyhow::{bail, Context, Result};
#[cfg(windows)]
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
#[cfg(windows)]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(windows)]
use std::sync::Arc;
#[cfg(windows)]
use std::time::Duration;

use crate::mix::{naar_mono, Resampler, SAMPLE_RATE};

/// Een lopende microfoon-tap. Vallen laten stopt de capturedraad netjes.
#[cfg(windows)]
pub struct MicrofoonTap {
    stop: Arc<AtomicBool>,
}

#[cfg(windows)]
impl MicrofoonTap {
    /// Start de capture op een eigen draad; signaal ná het bouwen én spelen van de
    /// stream, vóór de wachtlus — zelfde les als bij de loopback-tap.
    ///
    /// `naam` is de gekozen microfoon uit `config.toml` (leeg = standaardapparaat).
    /// Dezelfde keuze als het gesprek gebruikt: wie in de instellingen een microfoon
    /// aanwijst verwacht die ook in zijn clips terug te horen.
    pub fn start(naam: Option<&str>) -> Result<(Self, std::sync::mpsc::Receiver<Vec<f32>>)> {
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = std::sync::mpsc::channel();
        let stop_draad = stop.clone();
        let naam = naam.map(str::to_owned);
        let (klaar_tx, klaar_rx) = std::sync::mpsc::sync_channel::<Result<()>>(1);

        std::thread::Builder::new()
            .name("fitcom-clip-microfoon".into())
            .spawn(move || {
                // De cpal-stream leeft in deze scope en valt pas weg als de wachtlus
                // hieronder eindigt — een stream zonder eigenaar stopt met leveren.
                // `bouw_en_speel` moet hem dus teruggeven en niet zelf vasthouden: deed
                // hij dat wel, dan viel de stream weg op de regel ná `play()` en leverde
                // de tap nooit één chunk. Geen fout, geen logregel, alleen een clip
                // zonder jouw stem erin.
                match bouw_en_speel(&tx, naam.as_deref()) {
                    Err(e) => {
                        let _ = klaar_tx.send(Err(e));
                    }
                    Ok(stroom) => {
                        let _ = klaar_tx.send(Ok(()));
                        while !stop_draad.load(Ordering::Relaxed) {
                            std::thread::sleep(Duration::from_millis(100));
                        }
                        drop(stroom);
                    }
                }
            })
            .context("microfoondraad voor clips starten")?;

        match klaar_rx.recv_timeout(Duration::from_secs(2)) {
            Ok(res) => {
                res?;
                Ok((Self { stop }, rx))
            }
            Err(_) => bail!("geen antwoord van de microfoondraad binnen 2 seconden"),
        }
    }
}

#[cfg(windows)]
impl Drop for MicrofoonTap {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Bouwt de invoerstroom op het standaardapparaat, zet hem aan en **geeft hem terug**.
/// De beller houdt hem vast zolang er opgenomen wordt; een `cpal::Stream` die valt is een
/// stream die stopt met leveren, en dat gebeurt zonder foutmelding.
fn bouw_en_speel(
    tx: &std::sync::mpsc::Sender<Vec<f32>>,
    naam: Option<&str>,
) -> Result<cpal::Stream> {
    use anyhow::Context as _;
    let host = cpal::default_host();
    let device = crate::session::kies_apparaat(host.input_devices()?, naam)
        .or_else(|| host.default_input_device())
        .context("geen microfoon gevonden")?;
    let cfg = device.default_input_config().context("microfoon-config")?;
    // Welke microfoon dit is hoort in de log: "mijn stem staat niet in de clip" is
    // meestal een apparaatkwestie, en dan wil je die naam zien zonder te hoeven raden.
    tracing::info!(
        apparaat = %device,
        rate = cfg.sample_rate(),
        kanalen = cfg.channels(),
        "microfoon voor clips"
    );
    let rate = cfg.sample_rate();
    let kanalen = usize::from(cfg.channels());
    let scfg = cfg.config();

    let fout_melden = |e| tracing::debug!(error = %e, "microfoon-callback fout");

    // Elke arm bouwt zijn eigen conversieketen; slechts één arm wordt ooit gebouwd,
    // dus de moves hieronder lopen nooit tegen elkaar op.
    let stroom = match cfg.sample_format() {
        cpal::SampleFormat::F32 => {
            let mut resampler = Resampler::new(rate, SAMPLE_RATE);
            let zender = tx.clone();
            device.build_input_stream::<f32, _, _>(
                scfg,
                move |data: &[f32], _| {
                    let mut mono = Vec::new();
                    naar_mono(data, kanalen, &mut mono);
                    if mono.is_empty() {
                        return;
                    }
                    let mut op48 = Vec::new();
                    if rate == SAMPLE_RATE {
                        op48 = mono;
                    } else {
                        resampler.verwerk(&mono, &mut op48);
                    }
                    let mut stereo = Vec::with_capacity(op48.len() * 2);
                    for s in op48 {
                        stereo.push(s);
                        stereo.push(s);
                    }
                    let _ = zender.send(stereo);
                },
                fout_melden,
                None,
            )?
        }
        cpal::SampleFormat::I16 => {
            let mut resampler = Resampler::new(rate, SAMPLE_RATE);
            let zender = tx.clone();
            device.build_input_stream::<i16, _, _>(
                scfg,
                move |data: &[i16], _| {
                    let drijvend: Vec<f32> = data.iter().map(|&s| f32::from(s) / 32768.0).collect();
                    let mut mono = Vec::new();
                    naar_mono(&drijvend, kanalen, &mut mono);
                    if mono.is_empty() {
                        return;
                    }
                    let mut op48 = Vec::new();
                    if rate == SAMPLE_RATE {
                        op48 = mono;
                    } else {
                        resampler.verwerk(&mono, &mut op48);
                    }
                    let mut stereo = Vec::with_capacity(op48.len() * 2);
                    for s in op48 {
                        stereo.push(s);
                        stereo.push(s);
                    }
                    let _ = zender.send(stereo);
                },
                fout_melden,
                None,
            )?
        }
        andere => bail!("microfoon levert {andere:?}; alleen f32 en i16 worden ondersteund"),
    };
    stroom.play().context("microfoon-stream starten")?;
    Ok(stroom)
}
