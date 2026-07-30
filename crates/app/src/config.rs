//! Configuratie en identiteit.
//!
//! Twee bestanden, bewust gescheiden:
//!
//! - `config.toml` — door de gebruiker te bewerken. Adressen van de andere peers,
//!   poorten, weergavenaam. Mag je kopiëren tussen machines.
//! - `identity.toml` — door de app gegenereerd. Bevat de `PeerId` van deze installatie.
//!   Mag je **niet** kopiëren; twee peers met dezelfde id breken de oplog-sync,
//!   want dan botsen hun seq-nummers.

use anyhow::{Context, Result};
use fitcom_proto::PeerId;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const DEFAULT_CONTROL_PORT: u16 = 41650;
pub const DEFAULT_MEDIA_PORT: u16 = 41651;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Hoe de andere twee jou zien. Puur cosmetisch, mag je altijd wijzigen.
    pub display_name: String,

    #[serde(default = "default_control_port")]
    pub control_port: u16,

    #[serde(default = "default_media_port")]
    pub media_port: u16,

    /// Sluitknop verbergt naar de tray in plaats van af te sluiten. De app blijft dan
    /// synchroniseren en melden terwijl je iets anders doet.
    #[serde(default = "waar")]
    pub minimize_to_tray: bool,

    /// Meestarten met Windows. Staat standaard uit; wordt bij elke start toegepast,
    /// dus je zet hem hier aan of uit en herstart de app.
    #[serde(default)]
    pub autostart: bool,

    /// Naam van de microfoon zoals Windows hem toont. Leeg = standaardapparaat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_device: Option<String>,

    /// Naam van de koptelefoon of luidsprekers. Leeg = standaardapparaat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_device: Option<String>,

    /// De andere peers. Geen limiet van 3 — de code is bewust N-agnostisch.
    #[serde(default)]
    pub peers: Vec<PeerConfig>,

    #[serde(default)]
    pub video: VideoConfig,

    /// Waar gedownloade bestanden landen. Leeg = `<data-map>/downloads`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_dir: Option<PathBuf>,
}

/// Instellingen voor screenshare. Staan hier zodat ze te wijzigen zijn zonder opnieuw
/// te bouwen; de standaardwaarden horen voor iedereen te kloppen.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoConfig {
    /// `h264` of `hevc`. **Laat dit op h264 staan** tenzij je zeker weet dat álle
    /// peers HEVC kunnen decoderen; dat loopt op Windows via een Store-uitbreiding
    /// die er niet standaard op zit. Zie `docs/SPEC.md`.
    #[serde(default = "default_codec")]
    pub codec: String,

    #[serde(default = "default_fps")]
    pub fps: u32,

    /// Bits per seconde. Op het tailnet zelf zijn bits gratis, maar een peer met een
    /// mindere eigen internetverbinding kreeg bij 25 Mbit/s meetbare audio-lag in zijn
    /// *eigen* voice — niet bij de kijker. 12 Mbit/s loste dat op zonder merkbaar
    /// kwaliteitsverlies. Zie `docs/SPEC.md`, sectie "Bitrate".
    #[serde(default = "default_bitrate")]
    pub bitrate: u32,
}

impl Default for VideoConfig {
    fn default() -> Self {
        Self {
            codec: default_codec(),
            fps: default_fps(),
            bitrate: default_bitrate(),
        }
    }
}

fn default_codec() -> String {
    "h264".into()
}

fn default_fps() -> u32 {
    60
}

fn default_bitrate() -> u32 {
    12_000_000
}

fn waar() -> bool {
    true
}

fn default_control_port() -> u16 {
    DEFAULT_CONTROL_PORT
}

fn default_media_port() -> u16 {
    DEFAULT_MEDIA_PORT
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerConfig {
    /// Tailnet-IP (`100.x.x.x`) of MagicDNS-naam. MagicDNS heeft de voorkeur: die
    /// blijft geldig als het IP verandert.
    pub address: String,

    /// Alleen voor weergave zolang we de peer nog nooit gesproken hebben. Zodra hij
    /// verbindt gebruiken we de naam die hij zelf opgeeft.
    #[serde(default)]
    pub label: String,

    /// Wordt bij het eerste contact ingevuld en daarna vastgehouden (trust-on-first-use).
    /// Verbindt er later iemand anders vanaf hetzelfde adres, dan zien we dat.
    ///
    /// We vragen de gebruiker niet om vooraf UUID's uit te wisselen — dat is
    /// onnodige wrijving op een tailnet waar alleen jullie drie op zitten.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub known_id: Option<PeerId>,

    #[serde(default = "default_control_port")]
    pub control_port: u16,
}

impl Config {
    fn template() -> Self {
        Self {
            display_name: whoami_or("gebruiker"),
            control_port: DEFAULT_CONTROL_PORT,
            media_port: DEFAULT_MEDIA_PORT,
            minimize_to_tray: true,
            autostart: false,
            input_device: None,
            output_device: None,
            peers: vec![
                PeerConfig {
                    address: "100.64.0.2".into(),
                    label: "peer-2".into(),
                    known_id: None,
                    control_port: DEFAULT_CONTROL_PORT,
                },
                PeerConfig {
                    address: "100.64.0.3".into(),
                    label: "peer-3".into(),
                    known_id: None,
                    control_port: DEFAULT_CONTROL_PORT,
                },
            ],
            video: VideoConfig::default(),
            download_dir: None,
        }
    }

    pub fn load_or_create(path: &Path) -> Result<Self> {
        if path.exists() {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("config lezen uit {}", path.display()))?;
            toml::from_str(&text).with_context(|| format!("config parsen uit {}", path.display()))
        } else {
            let cfg = Self::template();
            cfg.save(path)?;
            tracing::info!(pad = %path.display(), "voorbeeldconfig aangemaakt, pas de peer-adressen aan");
            Ok(cfg)
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = toml::to_string_pretty(self)?;
        std::fs::write(path, text)
            .with_context(|| format!("config schrijven naar {}", path.display()))
    }
}

/// De identiteit van déze installatie.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identity {
    pub peer_id: PeerId,
}

impl Identity {
    pub fn load_or_create(path: &Path) -> Result<Self> {
        if path.exists() {
            let text = std::fs::read_to_string(path)?;
            Ok(toml::from_str(&text)?)
        } else {
            let me = Self {
                peer_id: PeerId::new_random(),
            };
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(path, toml::to_string_pretty(&me)?)?;
            tracing::info!(peer_id = %me.peer_id, "nieuwe identiteit aangemaakt");
            Ok(me)
        }
    }
}

/// Waar config, database en logs staan.
///
/// Portable heeft voorrang: staat er een `data`-map naast de exe, dan gebruiken we die.
/// Dat maakt "zip uitpakken en draaien" mogelijk, en meerdere instanties naast elkaar
/// voor lokaal testen. Anders `%APPDATA%\FitCommunication`.
pub fn resolve_data_dir(override_dir: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(d) = override_dir {
        std::fs::create_dir_all(&d)?;
        return Ok(d);
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(portable) = exe.parent().map(|p| p.join("data")) {
            if portable.is_dir() {
                return Ok(portable);
            }
        }
    }

    let dir = directories::ProjectDirs::from("", "", "FitCommunication")
        .context("kon %APPDATA% niet bepalen")?
        .data_dir()
        .to_path_buf();
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Waar gedownloade bestanden landen. `cfg.download_dir` wint als de gebruiker die
/// gezet heeft; anders een map naast de rest van de data van deze installatie.
pub fn resolve_download_dir(cfg: &Config, data_dir: &Path) -> PathBuf {
    cfg.download_dir
        .clone()
        .unwrap_or_else(|| data_dir.join("downloads"))
}

/// Waar een afbeelding landt die je zelf aanbiedt of van een ander downloadt — apart van
/// `download_dir`, want dit is geen gebruikersbestand met een leesbare naam maar een
/// content-adresseerbare cache (zie `crates/app/src/files.rs::hash_bestandsnaam`) die de
/// aanbieder en elke downloadende peer op exact hetzelfde pad laat uitkomen, zodat een
/// afbeelding voor beide kanten inline te tonen is. Niet instelbaar: dit is intern
/// plumbing, geen gebruikersbestand zoals een download.
pub fn resolve_pictures_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("Pictures")
}

/// Waar een van een peer opgehaalde nieuwere exe landt (fase 11), tot hij toegepast
/// wordt. Zelfde niet-instelbare-plumbing-patroon als `resolve_pictures_dir` — geen
/// gebruikersbestand, dus geen `config.toml`-veld.
pub fn resolve_updates_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("updates")
}

fn whoami_or(fallback: &str) -> String {
    std::env::var("USERNAME")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_overleeft_roundtrip() {
        let cfg = Config::template();
        let text = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.peers.len(), cfg.peers.len());
        assert_eq!(back.control_port, cfg.control_port);
    }

    #[test]
    fn minimale_config_krijgt_defaults() {
        // Iemand die met de hand een config schrijft mag alles weglaten behalve de naam.
        let cfg: Config = toml::from_str(r#"display_name = "Rick""#).unwrap();
        assert_eq!(cfg.control_port, DEFAULT_CONTROL_PORT);
        assert_eq!(cfg.media_port, DEFAULT_MEDIA_PORT);
        assert!(cfg.peers.is_empty());
    }

    #[test]
    fn peer_zonder_known_id_is_geldig() {
        let cfg: Config = toml::from_str(
            r#"
            display_name = "Rick"
            [[peers]]
            address = "vriend-pc"
            "#,
        )
        .unwrap();
        assert_eq!(cfg.peers[0].address, "vriend-pc");
        assert!(cfg.peers[0].known_id.is_none());
    }
}
