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

    /// Op welk adres de control- en mediapoort luisteren. B-09.
    ///
    /// **Waarom dit bestaat.** Het hele vertrouwensmodel van deze app rust op "alleen
    /// tailnet-verkeer bereikt ons", maar beide sockets binden op `0.0.0.0` — álle
    /// interfaces. Daarmee is elke andere bevinding in `docs/BEVEILIGING.md` ook bereikbaar
    /// vanaf het LAN, vanaf hotel- of congreswifi, en vanaf internet als een router de poort
    /// doorstuurt. De firewallregel is op dit moment de enige echte grens (zie `README.md`).
    ///
    /// **Waarom de standaard tóch `0.0.0.0` is.** Een vast adres hier is niet gratis: staat
    /// Tailscale nog niet omhoog als de app start, of verandert het tailnet-adres van deze
    /// machine, dan bindt de app aan een adres dat er niet is en is hij onbereikbaar tot
    /// iemand hem herstart. Dat is precies het soort stille breuk dat invariant 7 verbiedt.
    /// Deze waarde is daarom een keuze die je bewust maakt en één keer test, niet iets dat
    /// onder je vandaan verandert bij een update.
    ///
    /// **Wat je hier wilt invullen** is het `100.x.y.z`-adres van deze machine uit
    /// `tailscale ip -4`. Dat maakt B-09 in één regel dicht voor deze machine.
    #[serde(default = "default_bind_address")]
    pub bind_address: String,

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

    #[serde(default)]
    pub sound: SoundConfig,

    /// Waar gedownloade bestanden landen. Leeg = `<data-map>/downloads`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_dir: Option<PathBuf>,
}

/// Welke tonen de app zelf maakt bij deelnemen, verlaten en delen, en hoe hard.
///
/// Los van de geluidsapparaten hierboven: die gaan over het *gesprek*, dit over de app.
/// Deze tonen lopen ook niet via de voice-mixer (zie `crate::geluid`), dus ze hebben hun
/// eigen volume nodig — de volumemixer van Windows kan de app alleen als geheel zachter
/// zetten, en dan gaat je vriend mee.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SoundConfig {
    /// Welke set tonen. Zie `crate::geluid::Geluidset::van_naam`; een onbekende naam valt
    /// terug op de standaard in plaats van te weigeren, zodat een config van een nieuwere
    /// build een oudere niet laat struikelen.
    #[serde(default = "default_sound_set")]
    pub set: String,

    /// 0.0 tot 1.0. Nul is stil; dan wordt er niets afgespeeld in plaats van iets
    /// onhoorbaars.
    #[serde(default = "default_sound_volume")]
    pub volume: f32,
}

impl Default for SoundConfig {
    fn default() -> Self {
        Self {
            set: default_sound_set(),
            volume: default_sound_volume(),
        }
    }
}

impl SoundConfig {
    /// Zet met de hand geschreven waarden terug binnen wat ze kunnen betekenen.
    ///
    /// **TOML kent `nan` en `inf` als geldige floats.** `volume = nan` parseert dus gewoon,
    /// en zou daarna elke sample op nul zetten: alle geluidjes stil, zonder foutmelding en
    /// zonder dat er iets in de log staat. Dat is het soort stilte waar je een avond naar
    /// zoekt. Een waarde buiten 0..1 valt hier ook onder — die klopt niet, maar hij hoort
    /// afgekapt te worden en niet de app te weigeren.
    ///
    /// **Een onbekende `set` wordt hier bewust níet rechtgezet.** Dat is verleidelijk — met
    /// een onbekende naam staat er in de kiezer niets geselecteerd — maar het zou de belofte
    /// twee velden hierboven breken: een config die door een nieuwere build geschreven is
    /// mag zijn keuze niet verliezen doordat je één keer een oudere versie start. Dezelfde
    /// afspraak geldt voor `video.codec`. Het afspelen valt terug op de standaard (zie
    /// `engine.rs::geluidset`) en de *weergave* laat zien wat er werkelijk klinkt
    /// (`ui/state.rs`); de config houdt de bedoeling vast.
    pub fn herstel(&mut self) {
        // Alleen `nan` valt terug op de standaard: die betekent niets, dus er is niets uit
        // op te maken. `inf` en `-inf` hebben wél een richting ("zo hard/zacht mogelijk") en
        // die wordt gewoon afgekapt, net als 3.0 of −0.5.
        if self.volume.is_nan() {
            tracing::warn!("geluidvolume in de config is geen getal; standaard gebruikt");
            self.volume = default_sound_volume();
        }
        self.volume = self.volume.clamp(0.0, 1.0);
    }
}

fn default_sound_set() -> String {
    crate::geluid::Geluidset::STANDAARD.naam().to_string()
}

/// Niet 1.0: deze tonen komen langs terwijl er gegamed wordt, en het is prettiger om ze
/// harder te moeten zetten dan om de eerste keer te schrikken.
fn default_sound_volume() -> f32 {
    0.7
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

    /// Bovengrens, geen belofte: er wordt elk N-de schermbeeld verstuurd met N een heel
    /// getal, want alleen hele delers van de verversing geven gelijkmatig beeld. Op
    /// 144 Hz levert 60 dus 48 op en 72 er 72. Zie `fitcom_video::haalbaar_tempo` en
    /// `docs/OVERDRACHT.md`.
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

/// B-09: alle interfaces, zoals het altijd was. Zie het veld voor waarom dit de standaard
/// blijft en wat je in plaats daarvan zou invullen.
pub const ALLE_INTERFACES: &str = "0.0.0.0";

fn default_bind_address() -> String {
    ALLE_INTERFACES.to_string()
}

impl Config {
    /// Het adres waarop we luisteren, met een waarschuwing als dat alles is.
    ///
    /// De waarschuwing staat er omdat "we binden op alles" anders nergens zichtbaar is: de
    /// app werkt prima, en dat je bereikbaar bent vanaf de wifi van het hotel merk je pas
    /// als iemand het gebruikt. Eén regel per start, op het moment dat er toch al gelogd
    /// wordt.
    pub fn bind_ip(&self) -> String {
        if self.bind_address == ALLE_INTERFACES {
            tracing::warn!(
                "luistert op alle interfaces (B-09): op een gedeeld netwerk is de app dan \
                 ook buiten het tailnet bereikbaar. Zet `bind_address` in config.toml op je \
                 tailnet-adres (`tailscale ip -4`) om dat te sluiten."
            );
        } else {
            tracing::info!(adres = %self.bind_address, "luistert op één adres (B-09)");
        }
        self.bind_address.clone()
    }
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
            bind_address: default_bind_address(),
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
            sound: SoundConfig::default(),
            download_dir: None,
        }
    }

    pub fn load_or_create(path: &Path) -> Result<Self> {
        if path.exists() {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("config lezen uit {}", path.display()))?;
            let mut cfg: Self = toml::from_str(&text)
                .with_context(|| format!("config parsen uit {}", path.display()))?;
            cfg.sound.herstel();
            Ok(cfg)
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

/// Waar een afbeelding landt die je zelf aanbiedt of van een ander downloadt: een eigen
/// map *in de downloadmap*. De bestandsnaam is de inhoudshash
/// (`crates/app/src/files.rs::hash_bestandsnaam`) en geen leesbare naam, zodat de
/// aanbieder en elke downloadende peer op exact hetzelfde pad uitkomen en een afbeelding
/// voor beide kanten inline te tonen is — daarom een eigen map en niet los tussen de
/// downloads.
///
/// **In de downloadmap en niet in de datamap** (dat was het tot 2026-08-20): wie zijn
/// downloadmap naar een andere schijf verzet, kreeg geen enkele afbeelding meer binnen.
/// Het halve bestand stond dan op de ene schijf en de eindbestemming op de andere, en
/// `rename` kan niet over een schijfgrens heen — op Windows `os error 17`, op unix
/// `EXDEV`. Zie `docs/OVERDRACHT.md` beslissing 32.
pub fn resolve_pictures_dir(cfg: &Config, data_dir: &Path) -> PathBuf {
    pictures_in(&resolve_download_dir(cfg, data_dir))
}

/// Dezelfde regel als [`resolve_pictures_dir`], maar op een downloadmap die nog niet in
/// de config staat — de map die de gebruiker net in de kiezer heeft aangewezen. Eén
/// functie voor de regel, zodat de motor en het venster niet uit elkaar kunnen lopen.
pub fn pictures_in(download_dir: &Path) -> PathBuf {
    download_dir.join("Pictures")
}

/// Waar de titel en de miniatuur van een YouTube-link blijven staan zodra ze één keer
/// zijn opgehaald. Zelfde niet-instelbare-plumbing-patroon als `resolve_pictures_dir`.
/// Weggooien mag altijd: dan wordt er bij de volgende link opnieuw één keer opgehaald.
pub fn resolve_youtube_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("youtube")
}

/// Waar een van een peer opgehaalde nieuwere exe landt (fase 11), tot hij toegepast
/// wordt. Zelfde niet-instelbare-plumbing-patroon als `resolve_pictures_dir` — geen
/// gebruikersbestand, dus geen `config.toml`-veld.
pub fn resolve_updates_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("updates")
}

fn whoami_or(fallback: &str) -> String {
    // Windows zet USERNAME, macOS/Unix zet USER; beide proberen kost niets.
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
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

    /// De afbeeldingenmap hoort in de downloadmap te zitten, ook — juist — als de
    /// gebruiker die naar een andere schijf verzet heeft. Anders staat het halve bestand
    /// op de ene schijf en de eindbestemming op de andere, en dat is precies de
    /// hernoeming die niet kan (beslissing 32).
    #[test]
    fn de_afbeeldingenmap_volgt_de_downloadmap() {
        let data = Path::new("/data/fitcom");

        let standaard = Config {
            download_dir: None,
            ..Config::template()
        };
        assert_eq!(
            resolve_pictures_dir(&standaard, data),
            resolve_download_dir(&standaard, data).join("Pictures")
        );

        let eigen = Config {
            download_dir: Some(PathBuf::from("/volumes/schijf-d/Downloads")),
            ..Config::template()
        };
        assert_eq!(
            resolve_pictures_dir(&eigen, data),
            Path::new("/volumes/schijf-d/Downloads/Pictures"),
            "een eigen downloadmap moet de afbeeldingen meenemen"
        );
    }

    #[test]
    fn minimale_config_krijgt_defaults() {
        // Iemand die met de hand een config schrijft mag alles weglaten behalve de naam.
        let cfg: Config = toml::from_str(r#"display_name = "Rick""#).unwrap();
        assert_eq!(cfg.control_port, DEFAULT_CONTROL_PORT);
        assert_eq!(cfg.media_port, DEFAULT_MEDIA_PORT);
        assert!(cfg.peers.is_empty());
    }

    /// De reden dat dit een eigen test heeft: een config die vóór 1.0.1 geschreven is
    /// heeft geen `[sound]`-tabel, en die moet gewoon de standaardwaarden krijgen in
    /// plaats van de app te laten weigeren op te starten. Dat is precies één keer eerder
    /// misgegaan (de schema-bump van de kanalen-uitbreiding, zie docs/OVERDRACHT.md).
    #[test]
    fn config_van_voor_de_geluidsinstellingen_krijgt_de_standaardset() {
        let cfg: Config = toml::from_str(
            r#"
            display_name = "Rick"
            [video]
            codec = "h264"
            fps = 60
            bitrate = 12000000
            "#,
        )
        .unwrap();
        assert_eq!(cfg.sound, SoundConfig::default());
    }

    #[test]
    fn een_halve_geluidstabel_vult_zichzelf_aan() {
        let cfg: Config = toml::from_str(
            r#"
            display_name = "Rick"
            [sound]
            volume = 0.25
            "#,
        )
        .unwrap();
        assert_eq!(cfg.sound.volume, 0.25);
        assert_eq!(
            cfg.sound.set,
            default_sound_set(),
            "set hoort de standaard te zijn"
        );
    }

    /// `nan` en `inf` zijn geldige TOML-floats, dus een met de hand geschreven config kan ze
    /// bevatten — en dan zou alles stil zijn zonder dat er iets misgegaan lijkt.
    #[test]
    fn een_onmogelijk_volume_in_de_config_wordt_hersteld() {
        for (tekst, verwacht) in [
            ("nan", default_sound_volume()),
            ("inf", 1.0),
            ("-inf", 0.0),
            ("-0.5", 0.0),
            ("3.0", 1.0),
            ("0.4", 0.4),
        ] {
            let mut s: SoundConfig =
                toml::from_str(&format!("volume = {tekst}")).expect("moet parsen");
            s.herstel();
            assert_eq!(s.volume, verwacht, "volume = {tekst}");
        }
    }

    /// De tegenhanger van de volume-test: een setnaam die deze build niet kent blijft staan.
    /// Dat is voorwaartse compatibiliteit — één keer een oudere versie starten mag je keuze
    /// niet wissen — en geen vergeten geval.
    #[test]
    fn een_onbekende_geluidset_blijft_in_de_config_staan() {
        let mut s = SoundConfig {
            set: "belletjes-uit-een-latere-build".into(),
            volume: 0.5,
        };
        s.herstel();
        assert_eq!(s.set, "belletjes-uit-een-latere-build");
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
