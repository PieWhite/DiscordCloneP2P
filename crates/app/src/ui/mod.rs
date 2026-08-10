//! The display layer: a Tauri v2 window on WebView2.
//!
//! Pure display, exactly as the egui version was. It reads a snapshot from the engine
//! and sends commands back; no decision about the network or the store is taken here,
//! and no state lives here that would be lost if the window stopped drawing. That
//! boundary — `Snapshot` in, `UiCommand` out — is what made swapping the whole UI stack
//! affordable, so it survived the swap on purpose. See `docs/OVERDRACHT.md`, decision 19.
//!
//! # Three kinds of traffic, deliberately separated
//!
//! - `state` carries everything structural and is emitted **only when the serialized
//!   state actually differs**. With nothing happening it fires zero times a second,
//!   where egui repainted four times a second because immediate mode cannot do
//!   otherwise. That was one of the arguments for the move, so it is measured rather
//!   than assumed.
//! - `meters` carries the two things that change while you merely look at them — speaking
//!   level and RTT — at 4 Hz, and only while a call is running or a peer is online. The
//!   frontend patches attributes with it instead of re-rendering panels.
//! - `thumbnail` carries the stream strip at 2 Hz. In egui this was an
//!   `egui::TextureHandle` per stream; here the bytes are served over a `thumb://`
//!   protocol and the event only says which tile changed.

pub mod commands;
pub mod state;

use crate::config::Config;
use crate::engine::{EngineHandle, Snapshot};
use crate::files::DownloadStatus;
use crate::tray;
use fitcom_proto::PeerId;
use state::{Constants, UiState};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{Emitter, Manager};

/// Speaking level and RTT are the only things that move while the window is idle-ish.
/// 4 Hz is the rate the old build repainted *everything* at; here it moves two numbers.
const METER_INTERVAL: Duration = Duration::from_millis(250);

/// The stream strip. `docs/OVERDRACHT.md` sets the budget at 2 fps and a small size.
const THUMBNAIL_INTERVAL: Duration = Duration::from_millis(500);

/// Everything the commands need. Managed by Tauri, so one instance for the process.
pub struct Ui {
    pub engine: EngineHandle,
    pub me: PeerId,
    pub fallback_name: String,
    pub pictures_dir: PathBuf,
    pub minimize_to_tray: bool,
    constants: Constants,
    /// The source list handed to the picker, so "share the third one" resolves to the
    /// same source the user saw. Replaced wholesale every time the picker opens.
    sources: Mutex<Vec<fitcom_video::Bron>>,
    /// Bumped when the op log or a transfer status changes. The frontend refetches the
    /// open conversation on a change instead of the whole history riding on every state
    /// event.
    timeline_revision: AtomicU64,
    last_fingerprint: Mutex<u64>,
    /// PNG bytes per stream key, served over `thumb://`.
    thumbnails: Mutex<HashMap<String, Arc<Vec<u8>>>>,
    /// When each peer was last seen online, observed while this process ran.
    last_seen: Mutex<HashMap<PeerId, i64>>,
    /// Must outlive everything below it: dropping the runtime stops the engine.
    _runtime: tokio::runtime::Runtime,
}

impl Ui {
    fn state(&self) -> UiState {
        let snap = self.engine.snapshot.borrow().clone();
        self.build_state(&snap)
    }

    fn build_state(&self, snap: &Snapshot) -> UiState {
        let seen = self.note_last_seen(snap);
        UiState::build(snap, &self.constants, self.revision(snap), &seen)
    }

    /// "Last seen 22:14, 3 August" is the roster's line for a peer who is away, and the
    /// engine has nowhere to put it — a peer that is gone has no status to carry it. So
    /// it is observed here: every time a peer is up, that is the moment we remember.
    /// Not persisted, so the first run after a restart says plain "Offline" instead of
    /// making a time up.
    fn note_last_seen(&self, snap: &Snapshot) -> HashMap<PeerId, i64> {
        let mut seen = self.last_seen.lock().unwrap();
        let now = chrono::Local::now().timestamp_millis();
        for p in &snap.peers {
            if let (Some(id), fitcom_net::PeerStatus::Online { .. }) = (p.peer_id, &p.status) {
                seen.insert(id, now);
            }
        }
        seen.clone()
    }

    /// The op log is rebuilt into a fresh `Arc<Timeline>` whenever it changes, so
    /// comparing the allocation is enough — and the previous `Arc` is kept alive by the
    /// snapshot we are holding, so the address cannot be recycled underneath us.
    ///
    /// Transfer status is rendered in that same payload but lives *outside* the `Arc`
    /// (`Snapshot::files`), so it has to be folded in here as well. Without it a download
    /// that really ran left its card on "Download" for ever — the frontend refetches the
    /// conversation only when this number changes, so progress, "Downloaded" and a failure
    /// all stayed invisible and the button looked dead.
    fn revision(&self, snap: &Snapshot) -> u64 {
        let mut hasher = DefaultHasher::new();
        (Arc::as_ptr(&snap.timeline) as usize).hash(&mut hasher);
        for f in &snap.files {
            match &f.status {
                None => 0u8.hash(&mut hasher),
                Some(DownloadStatus::Bezig { ontvangen, .. }) => (1u8, ontvangen).hash(&mut hasher),
                Some(DownloadStatus::Voltooid) => 2u8.hash(&mut hasher),
                Some(DownloadStatus::Mislukt(e)) => (3u8, e).hash(&mut hasher),
            }
        }
        let vingerafdruk = hasher.finish();

        let mut last = self.last_fingerprint.lock().unwrap();
        if *last != vingerafdruk {
            *last = vingerafdruk;
            self.timeline_revision.fetch_add(1, Ordering::Relaxed);
        }
        self.timeline_revision.load(Ordering::Relaxed)
    }

    fn display_name(&self, snap: &Snapshot) -> String {
        snap.timeline
            .nicknames
            .get(&self.me)
            .cloned()
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| self.fallback_name.clone())
    }
}

fn parse_peer(id: &str) -> Option<PeerId> {
    id.parse::<uuid::Uuid>().ok().map(PeerId)
}

/// Starts the window. Returns when the user really quits — closing to the tray does not
/// come back here.
#[allow(clippy::too_many_arguments)]
pub fn run(
    engine: EngineHandle,
    me: PeerId,
    cfg: &Config,
    pictures_dir: PathBuf,
    runtime: tokio::runtime::Runtime,
) -> anyhow::Result<()> {
    let ui = Ui {
        me,
        fallback_name: cfg.display_name.clone(),
        pictures_dir: pictures_dir.clone(),
        minimize_to_tray: cfg.minimize_to_tray,
        constants: Constants {
            me,
            fallback_name: cfg.display_name.clone(),
            // Built from the engine's own list, so adding a tone set is one edit in
            // `geluid.rs` rather than one there and one in the frontend.
            sound_sets: crate::geluid::Geluidset::ALLE
                .iter()
                .map(|s| state::SoundSetInfo {
                    id: s.naam().to_string(),
                    name: s.label().to_string(),
                    description: s.beschrijving().to_string(),
                })
                .collect(),
            sound_events: crate::geluid::Geluid::ALLE
                .iter()
                .map(|g| state::SoundEventInfo {
                    id: g.naam().to_string(),
                    name: g.label().to_string(),
                })
                .collect(),
            control_port: cfg.control_port,
            media_port: cfg.media_port,
            pictures_dir: pictures_dir.clone(),
            autostart: cfg.autostart,
            minimize_to_tray: cfg.minimize_to_tray,
        },
        sources: Mutex::new(Vec::new()),
        timeline_revision: AtomicU64::new(0),
        last_fingerprint: Mutex::new(0),
        thumbnails: Mutex::new(HashMap::new()),
        last_seen: Mutex::new(HashMap::new()),
        engine,
        _runtime: runtime,
    };

    let foreground = ui.engine.voorgrond.clone();
    let quit_for_update = ui.engine.afsluiten_voor_update.clone();
    let snapshot_rx = ui.engine.snapshot.clone();
    let to_tray = cfg.minimize_to_tray;

    // Vóór het bouwen van de Tauri-app, anders claimt Tauri de menu-gebeurtenissen van de
    // tray en doet het tray-menu niets meer. Zie `tray::claim_menu_events`. Alleen op
    // Windows: de mac-tray gebruikt Tauri's eigen tray-API, geen losse muda-events.
    #[cfg(windows)]
    tray::claim_menu_events();

    tauri::Builder::default()
        .manage(ui)
        .register_uri_scheme_protocol("thumb", move |ctx, request| {
            let app = ctx.app_handle();
            let ui: tauri::State<'_, Ui> = app.state();
            let key = request.uri().path().trim_start_matches('/').to_string();
            let png = ui.thumbnails.lock().unwrap().get(&key).cloned();
            match png {
                Some(png) => tauri::http::Response::builder()
                    .header("Content-Type", "image/png")
                    // The URL carries a revision, so a stale frame is never requested;
                    // caching it is what keeps a re-render from re-fetching.
                    .header("Cache-Control", "max-age=31536000, immutable")
                    .body(png.to_vec())
                    .unwrap_or_default(),
                None => tauri::http::Response::builder()
                    .status(404)
                    .body(Vec::new())
                    .unwrap_or_default(),
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_state,
            commands::get_timeline,
            commands::send_message,
            commands::edit_message,
            commands::delete_message,
            commands::mark_read,
            commands::dismiss_error,
            commands::set_joined,
            commands::set_muted,
            commands::set_deafened,
            commands::set_do_not_disturb,
            commands::set_peer_volume,
            commands::set_stream_volume,
            commands::set_watching,
            commands::list_sources,
            commands::share_source,
            commands::stop_sharing,
            commands::set_camera,
            commands::set_video_settings,
            commands::list_audio_devices,
            commands::set_audio_devices,
            commands::set_sound_settings,
            commands::preview_sound,
            commands::pick_download_dir,
            commands::set_display_name,
            commands::pick_and_offer_file,
            commands::offer_files,
            commands::offer_pasted_image,
            commands::download_file,
            commands::delete_all_images,
            commands::create_channel,
            commands::rename_channel,
            commands::delete_channel,
            commands::check_update,
            commands::apply_update,
            commands::ignore_update,
            commands::dismiss_update,
            commands::close_window,
            ready,
        ])
        .setup(move |app| {
            let handle = app.handle().clone();

            // Images are read straight off disk by the webview through `asset:`, so the
            // one folder they can live in is opened and nothing else is.
            app.asset_protocol_scope().allow_directory(&pictures_dir, false)?;

            #[cfg(windows)]
            {
                if let Some(window) = app.get_webview_window("main") {
                    // The tray thread talks to the window over Win32 so it can show it
                    // again even when nothing is drawing — hidden windows do not run
                    // event loops.
                    match window.hwnd() {
                        Ok(hwnd) => tray::onthoud_venster(hwnd.0 as isize),
                        Err(e) => tracing::warn!(error = %e, "no window handle; the tray cannot show the window"),
                    }
                }

                // The icon has to stay alive or it drops straight out of the tray.
                match tray::start() {
                    Ok(t) => std::mem::forget(t),
                    Err(e) => tracing::warn!(error = %format!("{e:#}"), "starting the tray icon failed"),
                }
            }

            // On macOS the NSApplication run loop keeps pumping while the window is
            // hidden, so Tauri's own tray — on the main thread — is all that is needed;
            // the Win32 detour above exists precisely because that is not true there.
            #[cfg(target_os = "macos")]
            if let Err(e) = mac_tray(app.handle()) {
                tracing::warn!(error = %format!("{e:#}"), "starting the tray icon failed");
            }

            spawn_state_pusher(handle.clone(), snapshot_rx.clone());
            spawn_meters(handle.clone(), snapshot_rx.clone());
            spawn_thumbnails(handle.clone(), snapshot_rx.clone());
            spawn_quit_watcher(handle, quit_for_update.clone());
            Ok(())
        })
        .on_window_event(move |window, event| match event {
            // The engine reads this to decide whether a message deserves a Windows
            // notification. Unfocused is not the same as hidden, and both count.
            tauri::WindowEvent::Focused(focused) => {
                foreground.store(*focused, Ordering::Relaxed);
                let _ = window.emit("focus", *focused);
            }
            tauri::WindowEvent::CloseRequested { api, .. } => {
                if to_tray {
                    api.prevent_close();
                    foreground.store(false, Ordering::Relaxed);
                    #[cfg(windows)]
                    tray::verberg_venster();
                    #[cfg(not(windows))]
                    let _ = window.hide();
                }
            }
            tauri::WindowEvent::DragDrop(drop) => match drop {
                tauri::DragDropEvent::Enter { .. } | tauri::DragDropEvent::Over { .. } => {
                    let _ = window.emit("drag", true);
                }
                tauri::DragDropEvent::Drop { paths, .. } => {
                    let _ = window.emit("drag", false);
                    let _ = window.emit("dropped", paths.clone());
                }
                _ => {
                    let _ = window.emit("drag", false);
                }
            },
            _ => {}
        })
        .run(tauri::generate_context!())
        .map_err(|e| anyhow::anyhow!("starting the window: {e}"))?;

    Ok(())
}

/// The macOS tray: Tauri's own `TrayIconBuilder` on the main thread, same two items as
/// the Windows menu. "Afsluiten" flips the shared flag so `spawn_quit_watcher` performs
/// the same clean shutdown on both platforms.
#[cfg(target_os = "macos")]
fn mac_tray(app: &tauri::AppHandle) -> anyhow::Result<()> {
    use tauri::menu::{MenuBuilder, MenuItemBuilder};
    use tauri::tray::TrayIconBuilder;

    let openen = MenuItemBuilder::with_id("openen", "Openen").build(app)?;
    let afsluiten = MenuItemBuilder::with_id("afsluiten", "Afsluiten").build(app)?;
    let menu = MenuBuilder::new(app)
        .item(&openen)
        .separator()
        .item(&afsluiten)
        .build()?;

    let (rgba, n) = tray::icoon_rgba();
    let tray = TrayIconBuilder::new()
        .icon(tauri::image::Image::new_owned(rgba, n, n))
        .tooltip("FitCommunication")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "openen" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            "afsluiten" => tray::markeer_afsluiten(),
            _ => {}
        })
        .build(app)?;
    // Same rule as on Windows: the icon has to stay alive or it drops out of the bar.
    std::mem::forget(tray);
    Ok(())
}

/// Called once the frontend has painted. The window starts hidden so the first thing on
/// screen is the finished dark shell, never a white rectangle — the app is used in a
/// dark room and a flash of white is the one thing the theme exists to avoid.
#[tauri::command]
fn ready(app: tauri::AppHandle, ui: tauri::State<'_, Ui>) -> UiState {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
    ui.state()
}

fn spawn_state_pusher(app: tauri::AppHandle, mut rx: tokio::sync::watch::Receiver<Arc<Snapshot>>) {
    tauri::async_runtime::spawn(async move {
        let mut previous = String::new();
        loop {
            if rx.changed().await.is_err() {
                return;
            }
            let snap = rx.borrow_and_update().clone();
            let ui: tauri::State<'_, Ui> = app.state();
            let json = match serde_json::to_string(&ui.build_state(&snap)) {
                Ok(j) => j,
                Err(e) => {
                    tracing::error!(error = %e, "serializing the state failed");
                    continue;
                }
            };
            // The engine publishes on a fixed tick, so most of these are identical to the
            // last one. Comparing here is what keeps an idle window at zero events —
            // volatile figures live in `meters` precisely so they cannot defeat this.
            if json == previous {
                continue;
            }
            previous = json.clone();
            let _ = app.emit("state", json);
        }
    });
}

fn spawn_meters(app: tauri::AppHandle, rx: tokio::sync::watch::Receiver<Arc<Snapshot>>) {
    tauri::async_runtime::spawn(async move {
        let mut previous = String::new();
        let mut ticker = tokio::time::interval(METER_INTERVAL);
        loop {
            ticker.tick().await;
            let snap = rx.borrow().clone();

            let mut peers = serde_json::Map::new();
            for p in &snap.peers {
                let Some(id) = p.peer_id else { continue };
                let rtt = match &p.status {
                    fitcom_net::PeerStatus::Online { rtt_ms, .. } => Some(*rtt_ms),
                    _ => None,
                };
                peers.insert(
                    id.to_string(),
                    serde_json::json!({ "rtt": rtt, "level": p.niveau }),
                );
            }
            let payload = serde_json::json!({
                "peers": peers,
                "self": { "level": snap.voice.eigen_niveau },
            })
            .to_string();

            // With everyone offline and no call running this settles into one constant
            // string, so a truly idle app emits nothing at all after the first tick.
            if payload == previous {
                continue;
            }
            previous = payload.clone();
            let _ = app.emit("meters", payload);
        }
    });
}

fn spawn_thumbnails(app: tauri::AppHandle, rx: tokio::sync::watch::Receiver<Arc<Snapshot>>) {
    tauri::async_runtime::spawn(async move {
        let mut ticker = tokio::time::interval(THUMBNAIL_INTERVAL);
        // Which frame we last encoded per stream, so an unchanged tile costs nothing.
        let mut last: HashMap<String, usize> = HashMap::new();
        let mut revision: u64 = 0;
        loop {
            ticker.tick().await;
            let snap = rx.borrow().clone();
            let ui: tauri::State<'_, Ui> = app.state();

            let mut alive: Vec<String> = Vec::new();
            for s in snap.streams.iter().filter(|s| !s.is_geluid && s.kijken) {
                let key = format!("{}-{}", s.eigenaar, s.stream_id);
                alive.push(key.clone());
                let Some(thumb) = &s.miniatuur else { continue };
                let frame = Arc::as_ptr(&thumb.data) as *const u8 as usize;
                if last.get(&key) == Some(&frame) {
                    continue;
                }
                let Some(png) = encode_thumbnail(thumb) else {
                    continue;
                };
                last.insert(key.clone(), frame);
                revision += 1;
                ui.thumbnails
                    .lock()
                    .unwrap()
                    .insert(key.clone(), Arc::new(png));
                let _ = app.emit(
                    "thumbnail",
                    serde_json::json!({ "key": key, "revision": revision }),
                );
            }

            // A stream nobody watches any more must not keep its bytes alive; the strip
            // is the only thing that ever asks for them.
            if alive.len() != last.len() {
                last.retain(|k, _| alive.contains(k));
                ui.thumbnails
                    .lock()
                    .unwrap()
                    .retain(|k, _| alive.contains(k));
            }
        }
    });
}

/// The decoder hands us BGRA; PNG wants RGBA. At 2 fps and a strip-sized image this is
/// cheaper than any of the alternatives that keep the bytes raw, and it means the
/// frontend is a plain `<img>` instead of a canvas that has to be fed.
fn encode_thumbnail(thumb: &fitcom_video::Miniatuur) -> Option<Vec<u8>> {
    let mut rgba = thumb.data.to_vec();
    if rgba.len() < (thumb.breedte as usize * thumb.hoogte as usize * 4) {
        return None;
    }
    for px in rgba.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    let buffer = image::RgbaImage::from_raw(thumb.breedte, thumb.hoogte, rgba)?;
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(buffer)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .ok()?;
    Some(png)
}

/// Two ways out that are not the close button: the tray's Quit item, and a confirmed
/// update whose updater process has already been started. Both need the app to shut down
/// properly rather than be killed, so the connections close cleanly.
fn spawn_quit_watcher(app: tauri::AppHandle, quit_for_update: Arc<std::sync::atomic::AtomicBool>) {
    std::thread::Builder::new()
        .name("fitcom-quit-watch".into())
        .spawn(move || loop {
            if tray::wil_afsluiten() || quit_for_update.load(Ordering::Relaxed) {
                app.exit(0);
                return;
            }
            std::thread::sleep(Duration::from_millis(200));
        })
        .ok();
}
