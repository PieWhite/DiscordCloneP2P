//! Koppelt de ringbuffer-recorder (`fitcom_video::opname`) aan de motor (fase 15).
//!
//! Bewust een eigen module en niet verspreid over `engine.rs`: alles hier is
//! Windows-only — de recorder leunt op WASAPI-loopback voor het bureaubladgeluid, en
//! die heeft geen macOS-tegenhanger in deze fase (zie ROADMAP, fase 15). Op andere
//! platforms is dit een leeg beheerobject dat niets doet en zichzelf als afwezig meldt;
//! de motor maakt dezelfde aanroepen op beide platforms en de UI verbergt alles wat met
//! clips te maken heeft zodra de snapshot zegt dat ze er niet zijn.
//!
//! De globale hotkey hoort hier thuis en nergens anders: hij doet precies één ding
//! ("bewaar nu") en bestaat pas sinds deze functie. Push-to-talk wil later ook een
//! globale toets — die mag hiervan kopiëren, maar bouwt zijn eigen registratie.

/// Wat de UI over de clipopname weet. `Option<ClipsWeergave> == None` in de snapshot
/// betekent: dit platform ondersteunt geen clips, verberg alles.
#[derive(Debug, Clone)]
pub struct ClipsWeergave {
    pub aanwezig: bool,
    pub aan: bool,
    pub venster_sec: u32,
    pub map: String,
    pub laatste: Option<String>,
    pub fout: Option<String>,
}

// ---------------------------------------------------------------- windows

#[cfg(windows)]
mod backend {
    use super::ClipsWeergave;
    use anyhow::{bail, Context, Result};
    use fitcom_audio::loopback::{LoopbackTap, KANALEN};
    use fitcom_video::capture::{beschikbare_bronnen, Bron, BronSoort};
    use fitcom_video::opname::{ClipGebeurtenis, ClipInstellingen, OpnameHandle};
    use fitcom_video::D3dContext;
    use std::path::PathBuf;
    use std::sync::mpsc;

    pub struct ClipsBeheer {
        handle: Option<OpnameHandle>,
        gebeurtenissen: mpsc::Receiver<ClipGebeurtenis>,
        /// Houdt de geluids-tap levend zolang de opname loopt; zijn Drop stopt de
        /// capturedraad. Los van `handle` omdat het mislukken van het geluid de
        /// videoketen niet hoeft te blokkeren.
        geluid: Option<LoopbackTap>,
        laatste: Option<PathBuf>,
        fout: Option<String>,
        map: PathBuf,
    }

    impl ClipsBeheer {
        pub fn nieuw(map: PathBuf) -> Self {
            let (_dode_tx, rx) = mpsc::channel();
            Self {
                handle: None,
                gebeurtenissen: rx,
                geluid: None,
                laatste: None,
                fout: None,
                map,
            }
        }

        pub fn aanwezig(&self) -> bool {
            true
        }

        pub fn aan(&self) -> bool {
            self.handle.is_some()
        }

        /// Aan of uit, idempotent. Uitzetten gaat via de Drop van de handle; de ring
        /// blijft op schijf en wordt bij een herstart gewoon weer opgepakt.
        pub fn zet(
            &mut self,
            aan: bool,
            venster_sec: u32,
            fps: u32,
            bitrate: u32,
            d3d: Option<&D3dContext>,
        ) -> Result<()> {
            match (aan, self.handle.is_some()) {
                (false, false) | (true, true) => return Ok(()),
                (false, true) => {
                    tracing::info!("clipopname uit");
                    self.handle = None; // Drop stopt de keten netjes achteraan
                    self.geluid = None;
                    return Ok(());
                }
                (true, false) => {}
            }
            let Some(d3d) = d3d else {
                bail!("geen grafische kaart voor de clipopname");
            };

            let bron = eerste_monitor()?;
            let instellingen = ClipInstellingen {
                fps,
                bitrate,
                venster_sec,
            };

            // Geluid erbij als het kan; mislukt dat, dan gaan de clips zonder
            // audiospoor door — beter dan helemaal geen clip.
            self.geluid = None;
            let audio_bron = match LoopbackTap::start() {
                Ok((tap, sample_rate, ontvangen)) => {
                    self.geluid = Some(tap);
                    Some(fitcom_video::opname::AudioBron {
                        ontvangen,
                        sample_rate,
                        kanalen: KANALEN,
                    })
                }
                Err(e) => {
                    tracing::warn!(
                        error = %format!("{e:#}"),
                        "bureaubladgeluid voor clips niet beschikbaar"
                    );
                    None
                }
            };

            let (tx, rx) = mpsc::channel();
            self.gebeurtenissen = rx;
            self.fout = None;
            let ring_dir = self.map.join("ring");
            let handle = fitcom_video::opname::start_opname(
                d3d,
                &bron,
                instellingen,
                ring_dir,
                self.map.clone(),
                audio_bron,
                tx,
            )
            .context("clipopname starten")?;
            tracing::info!(bron = %bron.naam, "clipopname aan");
            self.handle = Some(handle);
            Ok(())
        }

        pub fn bewaar_nu(&mut self) {
            match &self.handle {
                Some(h) => h.bewaar_nu(),
                None => self.fout = Some("Clips staan uit.".into()),
            }
        }

        /// De laatste opgetreden clipfout, als die er nog staat.
        pub fn fout(&self) -> Option<String> {
            self.fout.clone()
        }

        /// Even vaak als de motortik: events binnenhalen en een doodlopende keten
        /// signaleren. Fouten blijven staan tot de volgende keer aanzetten.
        pub fn tik(&mut self) {
            while let Ok(ev) = self.gebeurtenissen.try_recv() {
                match ev {
                    ClipGebeurtenis::Klaar { pad } => {
                        tracing::debug!(pad = %pad.display(), "clip klaar");
                        self.laatste = Some(pad);
                    }
                    ClipGebeurtenis::Mislukt { reden } => {
                        tracing::warn!(reden = %reden, "clip mislukt");
                        self.fout = Some(reden);
                    }
                }
            }
            if let Some(h) = &self.handle {
                if h.gestopt() {
                    if let Some(f) = h.fout() {
                        self.fout = Some(f);
                    }
                    self.handle = None;
                    self.geluid = None;
                }
            }
        }

        pub fn weergave(&self, venster_sec: u32) -> ClipsWeergave {
            ClipsWeergave {
                aanwezig: true,
                aan: self.aan(),
                venster_sec,
                map: self.map.display().to_string(),
                laatste: self.laatste.as_ref().map(|p| p.display().to_string()),
                fout: self.fout.clone(),
            }
        }
    }

    fn eerste_monitor() -> Result<Bron> {
        let bronnen = beschikbare_bronnen().context("bronnen opvragen")?;
        bronnen.into_iter()
            .find(|b| b.soort == BronSoort::Monitor)
            .context("geen scherm gevonden om op te nemen")
    }

    /// Globale hotkey Ctrl+Alt+C: één ding — "bewaar nu". Werkt óók als het venster in
    /// de tray zit, juist dan: gamen doe je zonder dit venster voor je gezicht.
    ///
    /// De registratie is thread-gebonden (NULL-hwnd) en de draad bestaat voor de rest
    /// van het proces; bij afsluiten ruimt Windows de hotkey zelf op.
    pub fn start_hotkey(op_te_doen: impl Fn() + Send + 'static) -> Result<()> {
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            RegisterHotKey, MOD_ALT, MOD_CONTROL,
        };
        use windows::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, GetMessageW, TranslateMessage, MSG, WM_HOTKEY,
        };
        const ID: i32 = 1;
        const VK_C: u32 = 0x43;

        std::thread::Builder::new()
            .name("fitcom-hotkey".into())
            .spawn(move || {
                // SAFETY: thread-gebonden hotkey met uniek id binnen dit proces; alle
                // uitvoerparameters zijn geldig.
                unsafe {
                    if RegisterHotKey(None, ID, MOD_CONTROL | MOD_ALT, VK_C).is_err() {
                        tracing::warn!(
                            "Ctrl+Alt+C is al ergens anders in gebruik; de clip-hotkey doet niets"
                        );
                        return;
                    }
                }
                let mut msg = MSG::default();
                // SAFETY: standaard berichtenlus. GetMessageW geeft pas FALSE bij
                // WM_QUIT, en dat bericht komt hier nooit — het proces eindigt hard.
                unsafe {
                    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                        if msg.message == WM_HOTKEY && msg.wParam.0 == ID as usize {
                            op_te_doen();
                        }
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
            })?;
        Ok(())
    }
}

#[cfg(windows)]
pub use backend::{start_hotkey, ClipsBeheer};

// ---------------------------------------------------------------- andere platforms

#[cfg(not(windows))]
mod backend {
    use super::ClipsWeergave;
    use crate::config::ClipsConfig;
    use anyhow::Result;
    use fitcom_video::D3dContext;
    use std::path::PathBuf;

    pub struct ClipsBeheer {
        map: PathBuf,
    }

    impl ClipsBeheer {
        pub fn nieuw(map: PathBuf) -> Self {
            Self { map }
        }
        pub fn aanwezig(&self) -> bool {
            false
        }
        #[allow(clippy::unused_self)]
        pub fn aan(&self) -> bool {
            false
        }
        #[allow(clippy::needless_pass_by_ref_mut)]
        pub fn zet(
            &mut self,
            _aan: bool,
            _venster_sec: u32,
            _fps: u32,
            _bitrate: u32,
            _d3d: Option<&D3dContext>,
        ) -> Result<()> {
            Ok(())
        }
        #[allow(clippy::unused_self)]
        pub fn bewaar_nu(&mut self) {}
        pub fn tik(&mut self) {}
        #[allow(clippy::unused_self)]
        pub fn weergave(&self, cfg: &ClipsConfig) -> ClipsWeergave {
            ClipsWeergave {
                aanwezig: false,
                aan: false,
                venster_sec: cfg.venster_sec,
                map: self.map.display().to_string(),
                laatste: None,
                fout: None,
            }
        }
    }

    #[allow(clippy::needless_pass_by_unit_val)]
    pub fn start_hotkey(_op_te_doen: impl Fn() + Send + 'static) -> Result<()> {
        Ok(())
    }
}

#[cfg(not(windows))]
pub use backend::{start_hotkey, ClipsBeheer};
