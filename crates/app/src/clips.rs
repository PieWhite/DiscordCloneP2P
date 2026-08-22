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
//! ("bewaar nu"), is instelbaar en kan herregistreerd worden zonder herstart.

use anyhow::{bail, Context, Result};

/// Wat de UI over de clipopname weet. `Option<ClipsWeergave> == None` in de snapshot
/// betekent: dit platform ondersteunt geen clips, verberg alles.
#[derive(Debug, Clone)]
pub struct ClipsWeergave {
    pub aanwezig: bool,
    pub aan: bool,
    pub venster_sec: u32,
    /// Welk scherm er opgenomen wordt (naam uit de bronlijst).
    pub monitor: Option<String>,
    /// De ingestelde sneltoets, zoals hij in de config staat.
    pub hotkey: String,
    pub map: String,
    pub laatste: Option<String>,
    pub fout: Option<String>,
}

/// Ontleedt een sneltoets zoals `F9`, `ctrl+alt+c` of `shift+f2` naar
/// `(modifier-bits, virtual key)`. De modifier-bits zijn precies Windows'
/// `HOT_KEY_MODIFIERS`: alt=1, ctrl=2, shift=4, win=8. Platform-onafhankelijk zodat
/// de config-validatie ook op een mac kan draaien.
pub fn ontled_hotkey(spec: &str) -> Result<(u32, u32)> {
    let mut mods = 0u32;
    let mut vk: Option<u32> = None;
    for deel in spec.split('+') {
        match deel.trim().to_ascii_lowercase().as_str() {
            "" => continue,
            "ctrl" | "control" => mods |= 0x2,
            "alt" => mods |= 0x1,
            "shift" => mods |= 0x4,
            "win" | "windows" => mods |= 0x8,
            toets if is_f_toets(toets) => {
                let n: u32 = toets[1..].parse().context("f-toetsnummer")?;
                vk = Some(0x70 + n - 1); // VK_F1 = 0x70, oplopend
            }
            letter if letter.chars().count() == 1 => {
                let ch = letter.to_uppercase().chars().next().unwrap();
                let code = u32::from(ch);
                if !(code as u8).is_ascii_alphanumeric() {
                    bail!("onbekende toets '{deel}'");
                }
                vk = Some(code);
            }
            _ => bail!("onbekende toets '{deel}'"),
        }
    }
    let vk = vk.context("geen toets in de sneltoets (bijv. \"F9\")")?;
    Ok((mods, vk))
}

fn is_f_toets(d: &str) -> bool {
    d.len() >= 2
        && d.starts_with('f')
        && d[1..]
            .parse::<u32>()
            .is_ok_and(|n| (1..=24).contains(&n))
}

// ---------------------------------------------------------------- windows

#[cfg(windows)]
mod backend {
    use super::{ontled_hotkey, ClipsWeergave};
    use crate::config::ClipsConfig;
    use anyhow::{bail, Context, Result};
    use fitcom_audio::loopback::LoopbackTap;
    use fitcom_audio::microfoon::MicrofoonTap;
    use fitcom_video::capture::{beschikbare_bronnen, Bron, BronSoort};
    use fitcom_video::opname::{
        AudioBronnen, ClipGebeurtenis, ClipInstellingen, OpnameHandle,
    };
    use fitcom_video::D3dContext;
    use std::path::PathBuf;
    use std::sync::mpsc;

    pub struct ClipsBeheer {
        handle: Option<OpnameHandle>,
        gebeurtenissen: mpsc::Receiver<ClipGebeurtenis>,
        /// Houdt de geluids-taps levend zolang de opname loopt; hun Drop stopt de
        /// capturedraden. Los van `handle` omdat het mislukken van het geluid de
        /// videoketen niet hoeft te blokkeren.
        tap_systeem: Option<LoopbackTap>,
        tap_microfoon: Option<MicrofoonTap>,
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
                tap_systeem: None,
                tap_microfoon: None,
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
        ///
        /// `monitor_naam` kiest welk scherm; `None` of een onbekende naam valt terug
        /// op het eerste gevonden scherm.
        #[allow(clippy::too_many_arguments)]
        pub fn zet(
            &mut self,
            aan: bool,
            venster_sec: u32,
            fps: u32,
            bitrate: u32,
            monitor_naam: Option<&str>,
            d3d: Option<&D3dContext>,
        ) -> Result<()> {
            match (aan, self.handle.is_some()) {
                (false, false) | (true, true) => return Ok(()),
                (false, true) => {
                    tracing::info!("clipopname uit");
                    self.handle = None; // Drop stopt de keten netjes achteraan
                    self.tap_systeem = None;
                    self.tap_microfoon = None;
                    return Ok(());
                }
                (true, false) => {}
            }
            let Some(d3d) = d3d else {
                bail!("geen grafische kaart voor de clipopname");
            };

            let bron = kies_monitor(monitor_naam)?;
            let instellingen = ClipInstellingen {
                fps,
                bitrate,
                venster_sec,
            };

            // Geluid erbij als het kan — systeem/spel via de loopback, je eigen stem
            // via de microfoon. Elke bron mag apart falen: een clip zonder microfoon
            // is nog steeds een clip met spelgeluid.
            self.tap_systeem = None;
            self.tap_microfoon = None;
            let mut audio = AudioBronnen::default();
            match LoopbackTap::start() {
                Ok((tap, _rate, ontvangen)) => {
                    audio.systeem = Some(ontvangen);
                    self.tap_systeem = Some(tap);
                }
                Err(e) => tracing::warn!(
                    error = %format!("{e:#}"),
                    "systeemgeluid voor clips niet beschikbaar"
                ),
            }
            match MicrofoonTap::start() {
                Ok((tap, ontvangen)) => {
                    audio.microfoon = Some(ontvangen);
                    self.tap_microfoon = Some(tap);
                }
                Err(e) => tracing::info!(
                    error = %format!("{e:#}"),
                    "microfoon voor clips niet beschikbaar"
                ),
            }
            if !audio.heeft_bron() {
                tracing::warn!("geen geluidsbron voor clips; alleen beeld");
            }

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
                Some(audio),
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
        /// signaleren. Levert het pad van een clip die déze tik klaarkwam, zodat de
        /// motor daar zijn geluidje aan kan hangen. Fouten blijven staan tot de
        /// volgende keer aanzetten.
        pub fn tik(&mut self) -> Option<PathBuf> {
            let mut net_klaar = None;
            while let Ok(ev) = self.gebeurtenissen.try_recv() {
                match ev {
                    ClipGebeurtenis::Klaar { pad } => {
                        tracing::debug!(pad = %pad.display(), "clip klaar");
                        net_klaar = Some(pad.clone());
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
                    self.tap_systeem = None;
                    self.tap_microfoon = None;
                }
            }
            net_klaar
        }

        pub fn weergave(&self, cfg: &ClipsConfig) -> ClipsWeergave {
            ClipsWeergave {
                aanwezig: true,
                aan: self.aan(),
                venster_sec: cfg.venster_sec,
                monitor: cfg.monitor.clone(),
                hotkey: cfg.hotkey.clone(),
                map: self.map.display().to_string(),
                laatste: self.laatste.as_ref().map(|p| p.display().to_string()),
                fout: self.fout.clone(),
            }
        }
    }

    /// Het gekozen scherm, op naam; zonder keuze (of bij een naam die nergens meer
    /// bij hoort — monitors veranderen) het eerste gevonden.
    fn kies_monitor(naam: Option<&str>) -> Result<Bron> {
        let bronnen = beschikbare_bronnen().context("bronnen opvragen")?;
        let monitoren: Vec<Bron> =
            bronnen.into_iter().filter(|b| b.soort == BronSoort::Monitor).collect();
        monitoren
            .iter()
            .find(|b| Some(b.naam.as_str()) == naam)
            .or_else(|| monitoren.first())
            .cloned()
            .context("geen scherm gevonden om op te nemen")
    }

    /// Een lopende hotkey-registratie. Vallen laten heft de toets op — de berichtenlus
    /// krijgt WM_QUIT en doet zelf zijn UnregisterHotKey op de juiste thread.
    pub struct HotkeyDraad {
        thread_id: u32,
    }

    impl HotkeyDraad {
        pub fn wissel_naar(
            nieuw_spec: &str,
            op_te_doen: impl Fn() + Send + 'static,
        ) -> Result<Self> {
            // Eerst ontleden; een typefout mag de oude toets niet slopen.
            let (mods_raw, vk) = ontled_hotkey(nieuw_spec)?;
            Self::start_met(mods_raw, vk, op_te_doen)
        }

        fn start_met(
            mods_raw: u32,
            vk: u32,
            op_te_doen: impl Fn() + Send + 'static,
        ) -> Result<Self> {
            use windows::Win32::System::Threading::GetCurrentThreadId;
            let (id_tx, id_rx) = mpsc::sync_channel::<u32>(1);

            std::thread::Builder::new()
                .name("fitcom-hotkey".into())
                .spawn(move || {
                    // SAFETY: thread-gebonden aanroep vóór de lus; geen parameters.
                    let thread_id = unsafe { GetCurrentThreadId() };
                    let _ = id_tx.send(thread_id);
                    hotkey_lus(mods_raw, vk, ID_HOTKEY, op_te_doen);
                })
                .context("hotkey-draad starten")?;

            let thread_id = id_rx
                .recv_timeout(std::time::Duration::from_secs(2))
                .context("geen antwoord van de hotkey-draad")?;
            Ok(Self { thread_id })
        }

        /// Een handle zonder draad: de hotkey staat niet aan, maar alles om hem heen
        /// (config, UI) blijft gewoon werken.
        pub fn dode() -> Self {
            Self { thread_id: 0 }
        }
    }

    impl Drop for HotkeyDraad {
        fn drop(&mut self) {
            if self.thread_id == 0 {
                return;
            }
            use windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW;
            use windows::Win32::UI::WindowsAndMessaging::WM_QUIT;
            // SAFETY: WM_QUIT naar de hotkey-draad; haar GetMessageW valt daarop terug
            // en unregister't haar eigen hotkey vóór ze stopt.
            unsafe {
                let _ = PostThreadMessageW(
                    self.thread_id,
                    WM_QUIT,
                    windows::Win32::Foundation::WPARAM(0),
                    windows::Win32::Foundation::LPARAM(0),
                );
            }
        }
    }

    const ID_HOTKEY: i32 = 1;

    fn hotkey_lus(
        mods_raw: u32,
        vk: u32,
        id: i32,
        op_te_doen: impl Fn() + Send + 'static,
    ) {
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS,
        };
        use windows::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, GetMessageW, TranslateMessage, MSG, WM_HOTKEY,
        };
        // SAFETY: thread-gebonden hotkey met uniek id binnen dit proces.
        unsafe {
            if RegisterHotKey(None, id, HOT_KEY_MODIFIERS(mods_raw), vk).is_err() {
                tracing::warn!(
                    spec = %super::hotkey_naam(mods_raw, vk),
                    "de clip-sneltoets is al ergens anders in gebruik"
                );
                return;
            }
        }
        let mut msg = MSG::default();
        // SAFETY: standaard berichtenlus; WM_QUIT (van de Drop van HotkeyDraad) maakt
        // hem netjes af, mét unregister op deze thread.
        unsafe {
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                if msg.message == WM_HOTKEY && msg.wParam.0 == id as usize {
                    op_te_doen();
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let _ = UnregisterHotKey(None, id);
        }
    }

    pub fn start_hotkey(
        spec: &str,
        op_te_doen: impl Fn() + Send + 'static,
    ) -> Result<HotkeyDraad> {
        HotkeyDraad::wissel_naar(spec, op_te_doen)
    }

    /// Monitoren die voor clips gekozen kunnen worden, in bronvolgorde.
    pub fn monitoren() -> Result<Vec<String>> {
        let bronnen = beschikbare_bronnen().context("bronnen opvragen")?;
        Ok(bronnen
            .into_iter()
            .filter(|b| b.soort == BronSoort::Monitor)
            .map(|b| b.naam)
            .collect())
    }
}

#[cfg(windows)]
pub use backend::{monitoren, start_hotkey, ClipsBeheer, HotkeyDraad};

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
        #[allow(clippy::too_many_arguments)]
        pub fn zet(
            &mut self,
            _aan: bool,
            _venster_sec: u32,
            _fps: u32,
            _bitrate: u32,
            _monitor_naam: Option<&str>,
            _d3d: Option<&D3dContext>,
        ) -> Result<()> {
            Ok(())
        }
        #[allow(clippy::unused_self)]
        pub fn bewaar_nu(&mut self) {}
        pub fn tik(&mut self) -> Option<std::path::PathBuf> {
            None
        }
        pub fn weergave(&self, cfg: &ClipsConfig) -> ClipsWeergave {
            ClipsWeergave {
                aanwezig: false,
                aan: false,
                venster_sec: cfg.venster_sec,
                monitor: cfg.monitor.clone(),
                hotkey: cfg.hotkey.clone(),
                map: self.map.display().to_string(),
                laatste: None,
                fout: None,
            }
        }
    }

    pub struct HotkeyDraad;

    impl HotkeyDraad {
        pub fn dode() -> Self {
            Self
        }
    }

    impl HotkeyDraad {
        pub fn wissel_naar(
            _nieuw_spec: &str,
            _op_te_doen: impl Fn() + Send + 'static,
        ) -> Result<Self> {
            Ok(Self)
        }
    }

    pub fn start_hotkey(
        _spec: &str,
        _op_te_doen: impl Fn() + Send + 'static,
    ) -> Result<HotkeyDraad> {
        Ok(HotkeyDraad)
    }

    pub fn monitoren() -> Result<Vec<String>> {
        Ok(Vec::new())
    }
}

#[cfg(not(windows))]
pub use backend::{monitoren, start_hotkey, ClipsBeheer, HotkeyDraad};

/// Leesbare vorm voor in logs: welke toets bedoelden we. Alleen voor diagnostiek.
#[allow(dead_code)]
fn hotkey_naam(mods_raw: u32, vk: u32) -> String {
    let mut delen: Vec<String> = Vec::new();
    if mods_raw & 0x2 != 0 {
        delen.push("ctrl".to_string());
    }
    if mods_raw & 0x1 != 0 {
        delen.push("alt".to_string());
    }
    if mods_raw & 0x4 != 0 {
        delen.push("shift".to_string());
    }
    if mods_raw & 0x8 != 0 {
        delen.push("win".to_string());
    }
    if (0x70..=0x87).contains(&vk) {
        delen.push(format!("F{}", vk - 0x70 + 1)); // al String
    } else if let Some(c) = char::from_u32(vk) {
        delen.push(c.to_string());
    }
    delen.join("+")
}
