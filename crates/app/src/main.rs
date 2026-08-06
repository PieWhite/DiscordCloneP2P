//! FitCommunication — servervrije communicatie-app voor een kleine vaste groep.
//!
//! Zie `README.md` voor opstarten en `docs/ARCHITECTURE.md` voor het ontwerp.

// Geen consolevenster bij een release-build; in debug houden we hem wel zodat logs
// direct zichtbaar zijn tijdens ontwikkelen.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use fitcom::{config, engine, tray, ui};

use anyhow::{Context, Result};
use clap::Parser;
use config::{Config, Identity};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "fitcom", about = "FitCommunication")]
struct Args {
    /// Map voor config, database en logs. Zonder deze vlag: `data` naast de exe als
    /// die bestaat, anders `%APPDATA%\FitCommunication`.
    ///
    /// Meerdere instanties op één PC starten voor een lokale test doe je hiermee:
    /// elke instantie een eigen map, en in die config een eigen poort.
    #[arg(long)]
    data_dir: Option<PathBuf>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let data_dir = config::resolve_data_dir(args.data_dir)?;
    init_logging(&data_dir)?;

    tracing::info!(
        versie = env!("CARGO_PKG_VERSION"),
        protocol = fitcom_proto::PROTOCOL_VERSION,
        dir = %data_dir.display(),
        "FitCommunication start"
    );

    let config_path = data_dir.join("config.toml");
    let identity_path = data_dir.join("identity.toml");

    let cfg = Config::load_or_create(&config_path)?;
    let identity = Identity::load_or_create(&identity_path)?;

    tracing::info!(
        me = %identity.peer_id,
        naam = %cfg.display_name,
        peers = cfg.peers.len(),
        "configuratie geladen"
    );

    // De namen exact zoals ze in `config.toml` moeten komen te staan. Zonder dit moet
    // je gokken hoe Windows jouw headset noemt.
    match engine::audio_apparaten() {
        Ok((invoer, uitvoer)) => {
            tracing::info!(microfoons = ?invoer, weergave = ?uitvoer, "geluidsapparaten");
        }
        Err(e) => tracing::warn!(error = %format!("{e:#}"), "geluidsapparaten niet op te vragen"),
    }

    // De vensterlus wil de hoofdthread. Tokio krijgt daarom een eigen runtime op
    // achtergrondthreads; de UI praat er via kanalen mee, nooit via locks.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(2)
        .thread_name("fitcom-net")
        .build()
        .context("tokio-runtime opzetten")?;

    let mesh_cfg = fitcom_net::MeshConfig {
        me: identity.peer_id,
        display_name: cfg.display_name.clone(),
        control_port: cfg.control_port,
        media_port: cfg.media_port,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        targets: cfg
            .peers
            .iter()
            .map(|p| fitcom_net::PeerTarget {
                address: p.address.clone(),
                label: if p.label.is_empty() {
                    p.address.clone()
                } else {
                    p.label.clone()
                },
                known_id: p.known_id,
                control_port: p.control_port,
            })
            .collect(),
    };

    let mesh = {
        let _enter = runtime.enter();
        fitcom_net::spawn(mesh_cfg).context("netwerklaag starten")?
    };

    let store = fitcom_store::Store::open(&data_dir.join("chat.sqlite"), identity.peer_id)
        .context("chat-database openen")?;
    tracing::info!(ops = store.op_count().unwrap_or(0), "oplog geladen");

    // De motor draait op de tokio-runtime, niet op de UI-thread: chat en synchronisatie
    // moeten doorlopen terwijl het venster geminimaliseerd is of naar de tray staat.
    let pictures_dir = config::resolve_pictures_dir(&data_dir);

    if let Err(e) = tray::zet_autostart(cfg.autostart) {
        // Geen reden om de app niet te starten; alleen melden.
        tracing::warn!(error = %format!("{e:#}"), "autostart instellen mislukt");
    }

    // De motor bezit de config vanaf hier; de UI leest er alleen vaste waarden uit
    // (poorten, weergavenaam, autostart) en krijgt daarom een kopie.
    let cfg_voor_ui = cfg.clone();

    let engine = {
        let _enter = runtime.enter();
        engine::spawn(mesh, store, cfg, config_path).context("motor starten")?
    };

    // Het venster: Tauri v2 op WebView2. Eigen titelbalk volgens het ontwerp, dus de
    // OS-decoraties staan uit (`tauri.conf.json`). Alles eromheen — tray, vensterhandle,
    // gebeurtenissen — zit in `ui::run`.
    ui::run(
        engine,
        identity.peer_id,
        &cfg_voor_ui,
        pictures_dir,
        runtime,
    )
}

/// Logt naar een dagelijks roterend bestand in `<data>/logs` en naar de console.
///
/// Bewust **niet** via `tracing_appender::non_blocking`: die buffert tot het proces
/// netjes eindigt. Crasht de app of wordt hij hard afgesloten — precies wanneer je het
/// logbestand nodig hebt — dan is het leeg. We loggen weinig genoeg (verbindingen, geen
/// media-frames) dat direct schrijven niets kost.
///
/// Regel voor de rest van de app: nooit per frame of per audiopakket loggen.
fn init_logging(data_dir: &std::path::Path) -> Result<()> {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let log_dir = data_dir.join("logs");
    std::fs::create_dir_all(&log_dir)?;

    // Quinn en rustls zijn op debug-niveau extreem spraakzaam; die dempen we standaard.
    let filter = tracing_subscriber::EnvFilter::try_from_env("FITCOM_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,quinn=warn,rustls=warn"));

    tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(
                    // Levert `fitcom.2026-07-29.log`. De kant-en-klare `rolling::daily`
                    // plakt de datum áchter de extensie, wat zoeken op *.log breekt.
                    tracing_appender::rolling::Builder::new()
                        .rotation(tracing_appender::rolling::Rotation::DAILY)
                        .filename_prefix("fitcom")
                        .filename_suffix("log")
                        .build(&log_dir)
                        .context("logbestand aanmaken")?,
                )
                .with_ansi(false),
        )
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .init();

    Ok(())
}
