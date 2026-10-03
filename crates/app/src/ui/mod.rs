//! The display layer: one GPUI window, drawn with gpui-kit.
//!
//! Pure display, exactly as the egui and the Tauri versions were. It reads a snapshot from
//! the engine and sends commands back; no decision about the network or the store is taken
//! here, and no state lives here that would be lost if the window stopped drawing. That
//! boundary — `Snapshot` in, `UiCommand` out — is what made two swaps of the whole UI stack
//! affordable, so it survived both on purpose. See `docs/OVERDRACHT.md`, decision 19.
//!
//! # Three kinds of traffic, deliberately separated
//!
//! The engine publishes on a fixed tick. Three tokio tasks in [`bridge`] turn that into
//! three feeds the window waits on, and each one only moves when what it carries changed:
//!
//! - the `UiState`, compared with `PartialEq` on the tokio side — an idle app wakes the
//!   window zero times a second;
//! - speaking level and RTT at 4 Hz, into the `Meters` entity that only the small views
//!   showing them observe, so a meter tick does not re-render the timeline;
//! - stream thumbnails at 2 Hz, straight from the decoder's BGRA frames into GPUI images.
//!
//! Everything else here is the window around them: the tray, close-to-tray, focus, the
//! quit paths, and the debug fixture mode.

pub mod bridge;
pub mod commands;
#[cfg(debug_assertions)]
pub mod fixture;
pub mod icons;
pub mod state;
pub mod theme;
pub mod view;

use crate::config::Config;
use crate::engine::{EngineHandle, Snapshot};
use crate::files::DownloadStatus;
use crate::tray;
use fitcom_proto::PeerId;
use gpui_kit::{
    point, px, size, AnyWindowHandle, App, AsyncApp, Bounds, QuitMode, TitlebarOptions,
    WindowBackgroundAppearance, WindowBounds, WindowOptions,
};
use state::{Constants, UiState};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How often the quit watcher looks at the two "quit now" flags. It is its own thread, as
/// before; the window is woken only when one of them is set.
const QUIT_POLL: Duration = Duration::from_millis(200);

/// The window's first size and the smallest it may be dragged to.
const WINDOW_SIZE: (f32, f32) = (1180., 720.);
const WINDOW_MIN: (f32, f32) = (780., 460.);

/// Everything the commands need. One instance for the process, shared as `Arc<Ui>` between
/// the window thread and the tokio tasks.
pub struct Ui {
    pub engine: EngineHandle,
    pub me: PeerId,
    pub fallback_name: String,
    /// Where a fetched YouTube title and thumbnail are kept. Read here rather than in the
    /// engine because the fetch is triggered by what the window is drawing — see
    /// `commands::youtube_preview` and `crate::youtube`.
    pub youtube_dir: PathBuf,
    pub minimize_to_tray: bool,
    constants: Constants,
    /// The source list handed to the picker, so "share the third one" resolves to the
    /// same source the user saw. Replaced wholesale every time the picker opens.
    sources: Mutex<Vec<fitcom_video::Bron>>,
    /// B-52: the paths from the last OS drop, kept on this side and never handed to a view.
    ///
    /// They used to travel to the webview and straight back into `offer_files`, which made
    /// "offer this file to every peer" reachable for any script running in it. The views
    /// now only ever see indices into this list — the same pattern `sources`/`share_source`
    /// uses — so no view code can name a file of its own. Replaced wholesale on every drop,
    /// so a stale index resolves to nothing instead of to an older file.
    pub dropped: Mutex<Vec<PathBuf>>,
    /// Bumped when the op log or a transfer status changes. The window refetches the open
    /// conversation on a change instead of the whole history riding on every state update.
    timeline_revision: AtomicU64,
    last_fingerprint: Mutex<u64>,
    /// When each peer was last seen online, observed while this process ran.
    last_seen: Mutex<HashMap<PeerId, i64>>,
    /// Debug builds only: the window is fed from JSON files instead of an engine.
    #[cfg(debug_assertions)]
    pub(crate) fixture: Option<fixture::Fixture>,
    /// The runtime itself lives in `run`, not here: the last `Arc<Ui>` may be dropped by a
    /// tokio task, and a runtime dropped inside its own worker panics.
    pub(crate) runtime: tokio::runtime::Handle,
}

impl Ui {
    /// The state as it is right now. The bridge builds the same thing on every snapshot;
    /// this is for the first frame.
    pub fn state(&self) -> UiState {
        #[cfg(debug_assertions)]
        if let Some(f) = &self.fixture {
            return f.state.clone();
        }
        let snap = self.engine.snapshot.borrow().clone();
        self.build_state(&snap)
    }

    fn build_state(&self, snap: &Snapshot) -> UiState {
        let seen = self.note_last_seen(snap);
        UiState::build(snap, &self.constants, self.revision(snap), &seen)
    }

    /// The `asset:` scope of the webview, as a rule in code: the window draws an image from
    /// disk only when the path lies inside the picture folder or the YouTube cache. Every
    /// view that hands a path to `img()` asks this first.
    ///
    /// The picture folder is read from the live snapshot, because it moves along with the
    /// download folder (`config::resolve_pictures_dir`). The check is lexical — absolute,
    /// no `..`, under one of the roots — which is enough for paths `state.rs` built itself
    /// and cheap enough to run on every render.
    pub fn image_path_allowed(&self, path: &Path) -> bool {
        if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
            return false;
        }
        #[cfg(debug_assertions)]
        if let Some(f) = &self.fixture {
            return path.starts_with(&f.dir);
        }
        let pictures = self.engine.snapshot.borrow().pictures_dir.clone();
        (!pictures.as_os_str().is_empty() && path.starts_with(&pictures))
            || path.starts_with(&self.youtube_dir)
    }

    /// "Last seen 22:14, 3 August" is the roster's line for a peer who is away, and the
    /// engine has nowhere to put it — a peer that is gone has no status to carry it. So
    /// it is observed here: every time a peer is up, that is the moment we remember.
    /// Not persisted, so the first run after a restart says plain "Offline" instead of
    /// making a time up.
    fn note_last_seen(&self, snap: &Snapshot) -> HashMap<PeerId, i64> {
        let mut seen = self.last_seen.lock().unwrap_or_else(|e| e.into_inner());
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
    /// Transfer status is rendered in the timeline too but lives *outside* the `Arc`
    /// (`Snapshot::files`), so it has to be folded in here as well. Without it a download
    /// that really ran left its card on "Download" for ever — the window refetches the
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
        // Same reason for the Wordle card: it rides in the timeline but its state lives
        // outside the `Arc` — the day, and how far this machine got today. Without this,
        // your own guess would not update the card until the game ended and the result op
        // moved the log.
        snap.wordle.dag.hash(&mut hasher);
        if let Some(bord) = &snap.wordle.bord {
            (bord.rijen.len(), bord.klaar).hash(&mut hasher);
        }
        let vingerafdruk = hasher.finish();

        let mut last = self
            .last_fingerprint
            .lock()
            .unwrap_or_else(|e| e.into_inner());
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

fn constants(me: PeerId, cfg: &Config) -> Constants {
    Constants {
        me,
        fallback_name: cfg.display_name.clone(),
        // Built from the engine's own list, so adding a tone set is one edit in
        // `geluid.rs` rather than one there and one in the window.
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
        autostart: cfg.autostart,
        minimize_to_tray: cfg.minimize_to_tray,
    }
}

/// Starts the window. Returns when the user really quits — closing to the tray does not
/// come back here. (On macOS it does not return at all: AppKit ends the process.)
#[allow(clippy::too_many_arguments)]
pub fn run(
    engine: EngineHandle,
    me: PeerId,
    cfg: &Config,
    youtube_dir: PathBuf,
    runtime: tokio::runtime::Runtime,
) -> anyhow::Result<()> {
    let ui = Arc::new(Ui {
        me,
        fallback_name: cfg.display_name.clone(),
        youtube_dir,
        minimize_to_tray: cfg.minimize_to_tray,
        constants: constants(me, cfg),
        sources: Mutex::new(Vec::new()),
        dropped: Mutex::new(Vec::new()),
        timeline_revision: AtomicU64::new(0),
        last_fingerprint: Mutex::new(0),
        last_seen: Mutex::new(HashMap::new()),
        engine,
        #[cfg(debug_assertions)]
        fixture: None,
        runtime: runtime.handle().clone(),
    });
    let feeds = bridge::spawn_live(&ui);
    open(ui, feeds);
    // Kept alive until the window loop has ended; see `Ui::runtime`.
    drop(runtime);
    Ok(())
}

/// Debug builds only: the window with no mesh, store or engine behind it, fed from the
/// JSON files in `dir` (see `fixture`). Commands are logged and go nowhere.
#[cfg(debug_assertions)]
pub fn run_fixture(dir: PathBuf) -> anyhow::Result<()> {
    use anyhow::Context as _;
    let fixture = fixture::Fixture::load(&dir)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(1)
        .thread_name("fitcom-fixture")
        .build()
        .context("tokio-runtime opzetten")?;

    // A real handle with nothing behind it: the snapshot never moves, and every command is
    // logged so a click in the fixture window shows what it would have asked for.
    let (snapshot_tx, snapshot_rx) = tokio::sync::watch::channel(Arc::new(Snapshot::default()));
    let (commands_tx, mut commands_rx) = tokio::sync::mpsc::channel(64);
    runtime.spawn(async move {
        let _keep = snapshot_tx;
        while let Some(cmd) = commands_rx.recv().await {
            tracing::info!(?cmd, "fixture: command");
        }
    });
    let engine = EngineHandle {
        snapshot: snapshot_rx,
        commands: commands_tx,
        voorgrond: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        afsluiten_voor_update: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };
    let s = &fixture.state;
    let me = parse_peer(&s.me.id).unwrap_or(PeerId(uuid::Uuid::nil()));
    // Only `build_state` reads these, and a fixture never builds a state; they mirror the
    // file so nothing disagrees with it.
    let constants = Constants {
        me,
        fallback_name: s.me.name.clone(),
        sound_sets: s.sound_sets.clone(),
        sound_events: s.sound_events.clone(),
        control_port: s.control_port,
        media_port: s.media_port,
        autostart: s.autostart,
        minimize_to_tray: false,
    };
    let feeds = bridge::Feeds::fixed(s.clone(), fixture.meters.clone(), fixture.thumbs.clone());
    let ui = Arc::new(Ui {
        me,
        fallback_name: s.me.name.clone(),
        youtube_dir: dir.join("youtube"),
        minimize_to_tray: false,
        constants,
        sources: Mutex::new(Vec::new()),
        dropped: Mutex::new(Vec::new()),
        timeline_revision: AtomicU64::new(0),
        last_fingerprint: Mutex::new(0),
        last_seen: Mutex::new(HashMap::new()),
        engine,
        fixture: Some(fixture),
        runtime: runtime.handle().clone(),
    });
    open(ui, feeds);
    drop(runtime);
    Ok(())
}

/// The window: opens it, hooks up tray, close, focus and quit, and runs the event loop.
fn open(ui: Arc<Ui>, feeds: bridge::Feeds) {
    let quit_for_update = ui.engine.afsluiten_voor_update.clone();
    let reopen_target: Arc<Mutex<Option<AnyWindowHandle>>> = Arc::new(Mutex::new(None));

    // Before the event loop: the muda handler is first-come, and the Windows tray thread
    // depends on it for "Openen" and "Afsluiten". See `tray::claim_menu_events`.
    #[cfg(windows)]
    tray::claim_menu_events();

    let app = gpui_kit::application().with_assets(icons::Assets);
    {
        // The Dock icon brings the window back after close-to-tray, like "Openen" does.
        let target = reopen_target.clone();
        app.on_reopen(move |cx| show_window(&target, cx));
    }

    app.run(move |cx: &mut App| {
        gpui_kit::init(cx);
        theme::install(cx);
        view::init(cx);
        // Closing the window is not quitting (minimize-to-tray); quitting is explicit,
        // through `close_window`, the tray or the updater.
        cx.set_quit_mode(QuitMode::Explicit);

        let handle = match view::open_window(cx, window_options(cx), ui.clone(), feeds) {
            Ok(h) => h,
            Err(e) => {
                tracing::error!(error = %format!("{e:#}"), "opening the window failed");
                cx.quit();
                return;
            }
        };
        *reopen_target.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);

        hook_close(&handle, ui.clone(), cx);

        #[cfg(windows)]
        start_windows_tray(&handle, cx);

        #[cfg(target_os = "macos")]
        if let Err(e) = mac_tray(reopen_target.clone(), cx) {
            tracing::warn!(error = %format!("{e:#}"), "starting the tray icon failed");
        }

        spawn_quit_watcher(quit_for_update.clone(), cx);
    });
}

fn window_options(cx: &App) -> WindowOptions {
    let (w, h) = debug_size().unwrap_or(WINDOW_SIZE);
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(w), px(h)),
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some("FitCommunication".into()),
            // Our own titlebar on both platforms. On macOS the traffic lights stay native
            // and sit in it; on Windows the caption buttons are ours, hit-tested through
            // `WindowControlArea` (see `view/titlebar.rs`).
            appears_transparent: true,
            traffic_light_position: Some(point(px(12.), px(10.))),
        }),
        // The titlebar moves the window itself (`start_window_move`), so AppKit must not
        // treat the top strip as its own drag region and delay clicks on it.
        app_owns_titlebar_drag: true,
        window_min_size: Some(size(px(WINDOW_MIN.0), px(WINDOW_MIN.1))),
        // Opaque, and the root paints the canvas colour from the first frame: the app is
        // used in a dark room, and a white flash is the one thing the theme exists to avoid.
        window_background: WindowBackgroundAppearance::Opaque,
        app_id: Some("nl.fitcommunication.app".into()),
        ..Default::default()
    }
}

/// `FITCOM_DEBUG_SIZE=980x720`: the first window size, so a narrow layout can be shot
/// without dragging. Debug builds only.
fn debug_size() -> Option<(f32, f32)> {
    #[cfg(debug_assertions)]
    {
        let v = std::env::var("FITCOM_DEBUG_SIZE").ok()?;
        let (w, h) = v.split_once('x')?;
        Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
    }
    #[cfg(not(debug_assertions))]
    None
}

/// The OS close request — the traffic light, Alt+F4, the Windows caption button — goes
/// through the same rule as our own close button.
fn hook_close(handle: &AnyWindowHandle, ui: Arc<Ui>, cx: &mut App) {
    let _ = handle.update(cx, |_, window, cx| {
        window.on_window_should_close(cx, move |_, cx| {
            if commands::close_window(&ui, cx) {
                cx.quit();
            }
            // Never let GPUI drop the window itself: quitting is `cx.quit()`, hiding keeps it.
            false
        });
    });
}

fn show_window(target: &Arc<Mutex<Option<AnyWindowHandle>>>, cx: &mut App) {
    cx.activate(true);
    if let Some(handle) = *target.lock().unwrap_or_else(|e| e.into_inner()) {
        let _ = handle.update(cx, |_, window, _| window.activate_window());
    }
}

/// The Windows tray: unchanged `tray.rs`, on its own Win32 thread, which needs the HWND to
/// show the window again even when nothing is drawing.
#[cfg(windows)]
fn start_windows_tray(handle: &AnyWindowHandle, cx: &mut App) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let _ = handle.update(cx, |_, window, _| match window.window_handle().map(|h| h.as_raw()) {
        Ok(RawWindowHandle::Win32(h)) => tray::onthoud_venster(h.hwnd.get()),
        Ok(_) => tracing::warn!("no Win32 window handle; the tray cannot show the window"),
        Err(e) => tracing::warn!(error = %e, "no window handle; the tray cannot show the window"),
    });
    // The icon has to stay alive or it drops straight out of the tray.
    match tray::start() {
        Ok(t) => std::mem::forget(t),
        Err(e) => tracing::warn!(error = %format!("{e:#}"), "starting the tray icon failed"),
    }
}

/// The macOS tray: the same `tray-icon` crate as Windows, on the main thread, with the
/// same two items. On macOS the run loop keeps pumping while the window is hidden, so the
/// Win32 detour is not needed. "Afsluiten" flips the shared flag so the quit watcher does
/// the same clean shutdown on both platforms; "Openen" is handed to the window thread over
/// a channel, so nothing polls for it.
#[cfg(target_os = "macos")]
fn mac_tray(target: Arc<Mutex<Option<AnyWindowHandle>>>, cx: &mut App) -> anyhow::Result<()> {
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::TrayIconBuilder;

    const OPENEN: &str = "fitcom-openen";
    const AFSLUITEN: &str = "fitcom-afsluiten";

    let menu = Menu::new();
    menu.append(&MenuItem::with_id(OPENEN, "Openen", true, None))?;
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&MenuItem::with_id(AFSLUITEN, "Afsluiten", true, None))?;

    let (rgba, n) = tray::icoon_rgba();
    let icon = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("FitCommunication")
        .with_icon(tray_icon::Icon::from_rgba(rgba, n, n)?)
        .build()?;
    // Same rule as on Windows: the icon has to stay alive or it drops out of the bar.
    std::mem::forget(icon);

    let (open_tx, mut open_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    MenuEvent::set_event_handler(Some(move |ev: MenuEvent| match ev.id.0.as_str() {
        OPENEN => {
            let _ = open_tx.send(());
        }
        AFSLUITEN => tray::markeer_afsluiten(),
        _ => {}
    }));
    cx.spawn(async move |cx: &mut AsyncApp| {
        while open_rx.recv().await.is_some() {
            cx.update(|cx| show_window(&target, cx));
        }
    })
    .detach();
    Ok(())
}

/// Two ways out that are not the close button: the tray's quit item, and a confirmed
/// update whose updater process has already been started. Both need the app to shut down
/// properly rather than be killed, so the connections close cleanly.
///
/// The flags are plain atomics set from other threads, so a thread of its own looks at
/// them — as before — and wakes the window thread only when one is set.
fn spawn_quit_watcher(quit_for_update: Arc<std::sync::atomic::AtomicBool>, cx: &mut App) {
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let started = std::thread::Builder::new()
        .name("fitcom-quit-watch".into())
        .spawn(move || loop {
            if tray::wil_afsluiten() || quit_for_update.load(Ordering::Relaxed) {
                let _ = tx.send(());
                return;
            }
            std::thread::sleep(QUIT_POLL);
        });
    if let Err(e) = started {
        tracing::warn!(error = %e, "the quit watcher did not start; quitting from the tray will not work");
        return;
    }
    cx.spawn(async move |cx: &mut AsyncApp| {
        if rx.await.is_ok() {
            cx.update(|cx| cx.quit());
        }
    })
    .detach();
}
