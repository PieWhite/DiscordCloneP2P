//! The other half of the boundary: everything the window can ask the engine to do.
//!
//! One command per `UiCommand` variant, plus the handful of things that are genuinely
//! the display layer's job (a file picker, the device list, the window buttons). Nothing
//! here decides anything — it translates an IPC call into a message on the engine's
//! channel and returns.

use super::state::{self, OpRef, TimelineItem, UiState};
use super::Ui;
use crate::engine::{self, UiCommand};
#[cfg(windows)]
use crate::tray;
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::{Manager, State};

/// Commands are fire-and-forget: the result of one is a new snapshot, which arrives as a
/// `state` event like any other change. A full channel means the engine is wedged, which
/// is worth a log line and nothing else — dropping the command is better than blocking
/// the webview's IPC thread.
fn send(ui: &Ui, cmd: UiCommand) {
    if let Err(e) = ui.engine.commands.try_send(cmd) {
        tracing::warn!(error = %e, "command not handed to the engine");
    }
}

#[tauri::command]
pub fn get_state(ui: State<'_, Ui>) -> UiState {
    ui.state()
}

#[tauri::command]
pub fn get_timeline(ui: State<'_, Ui>, channel: String) -> Vec<TimelineItem> {
    let snap = ui.engine.snapshot.borrow().clone();
    let channel = state::parse_channel(&channel);
    state::timeline_of(&snap, channel, ui.me, &ui.display_name(&snap))
}

/// Send a chat message. `reply_to` is `Some` when the composer had a reply open; the
/// engine validates that it points into this same conversation.
#[tauri::command]
pub fn send_message(ui: State<'_, Ui>, channel: String, text: String, reply_to: Option<OpRef>) {
    let text = text.trim().to_string();
    if text.is_empty() {
        return;
    }
    let reply_to = reply_to.and_then(|r| r.to_op_id());
    send(
        &ui,
        UiCommand::Plaats(text, state::parse_channel(&channel), reply_to),
    );
}

/// Add a reaction, or take it back if we already reacted with this emoji — the toggle
/// lives in the chat layer, which knows what is currently on the message.
#[tauri::command]
pub fn react_message(ui: State<'_, Ui>, op: OpRef, emoji: String) {
    let Some(id) = op.to_op_id() else { return };
    let emoji = emoji.trim().to_string();
    if emoji.is_empty() {
        return;
    }
    send(&ui, UiCommand::Reageer(id, emoji));
}

/// "I am typing in this conversation." Fire-and-forget and throttled on the engine side;
/// the frontend may call it on every keystroke.
#[tauri::command]
pub fn notify_typing(ui: State<'_, Ui>, channel: String) {
    send(&ui, UiCommand::Typing(state::parse_channel(&channel)));
}

/// Our own presence status: "online", "away" or "busy". Unknown values are dropped here
/// rather than corrected there.
#[tauri::command]
pub fn set_user_status(ui: State<'_, Ui>, status: String) {
    let status = match status.as_str() {
        "online" => 0,
        "away" => 1,
        "busy" => 2,
        _ => return,
    };
    send(&ui, UiCommand::ZetStatus(status));
}

#[tauri::command]
pub fn edit_message(ui: State<'_, Ui>, op: OpRef, text: String) {
    let text = text.trim().to_string();
    let Some(id) = op.to_op_id() else { return };
    if text.is_empty() {
        send(&ui, UiCommand::Verwijder(id));
    } else {
        send(&ui, UiCommand::Bewerk(id, text));
    }
}

#[tauri::command]
pub fn delete_message(ui: State<'_, Ui>, op: OpRef) {
    if let Some(id) = op.to_op_id() {
        send(&ui, UiCommand::Verwijder(id));
    }
}

/// Opening a conversation clears its own unread counter and nothing else — the three
/// counters (general, per sub-channel, per DM) are tracked separately on purpose.
#[tauri::command]
pub fn mark_read(ui: State<'_, Ui>, channel: String) {
    let channel = state::parse_channel(&channel);
    let cmd = if let Some(peer) = channel.dm_peer() {
        UiCommand::GelezenDm(peer)
    } else if let Some(topic) = channel.topic_id() {
        UiCommand::GelezenTopic(topic)
    } else {
        UiCommand::Gelezen
    };
    send(&ui, cmd);
}

#[tauri::command]
pub fn dismiss_error(ui: State<'_, Ui>) {
    send(&ui, UiCommand::FoutWeg);
}

#[tauri::command]
pub fn set_joined(ui: State<'_, Ui>, joined: bool) {
    send(
        &ui,
        if joined {
            UiCommand::VoiceDeelnemen
        } else {
            UiCommand::VoiceVerlaten
        },
    );
}

#[tauri::command]
pub fn set_muted(ui: State<'_, Ui>, muted: bool) {
    send(&ui, UiCommand::Mute(muted));
}

#[tauri::command]
pub fn set_deafened(ui: State<'_, Ui>, deafened: bool) {
    send(&ui, UiCommand::Deafen(deafened));
}

#[tauri::command]
pub fn set_do_not_disturb(ui: State<'_, Ui>, on: bool) {
    send(&ui, UiCommand::NietStoren(on));
}

#[tauri::command]
pub fn set_peer_volume(ui: State<'_, Ui>, peer: String, volume: f32) {
    if let Some(p) = super::parse_peer(&peer) {
        send(&ui, UiCommand::Volume(p, volume));
    }
}

#[tauri::command]
pub fn set_stream_volume(ui: State<'_, Ui>, peer: String, stream: u32, volume: f32) {
    if let Some(p) = super::parse_peer(&peer) {
        send(&ui, UiCommand::StreamVolume(p, stream, volume));
    }
}

#[tauri::command]
pub fn set_watching(ui: State<'_, Ui>, peer: String, stream: u32, watching: bool) {
    if let Some(p) = super::parse_peer(&peer) {
        send(
            &ui,
            if watching {
                UiCommand::Kijken(p, stream)
            } else {
                UiCommand::StopKijken(p, stream)
            },
        );
    }
}

#[derive(Serialize)]
pub struct SourceOption {
    pub index: usize,
    pub name: String,
    pub is_window: bool,
    pub is_camera: bool,
}

/// Fetched when the picker opens, never cached: windows come and go, and a stale list
/// offers something that is no longer there.
#[tauri::command]
pub fn list_sources(ui: State<'_, Ui>) -> Vec<SourceOption> {
    let sources = match engine::deelbare_bronnen() {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "listing shareable sources failed");
            Vec::new()
        }
    };
    let listed = sources
        .iter()
        .enumerate()
        .map(|(index, b)| SourceOption {
            index,
            name: b.naam.clone(),
            is_window: matches!(b.soort, fitcom_video::BronSoort::Venster),
            is_camera: matches!(b.soort, fitcom_video::BronSoort::Camera),
        })
        .collect();
    *ui.sources.lock().unwrap_or_else(|e| e.into_inner()) = sources;
    listed
}

#[tauri::command]
pub fn share_source(ui: State<'_, Ui>, index: usize) {
    let source = ui
        .sources
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(index)
        .cloned();
    match source {
        Some(b) => send(&ui, UiCommand::DeelBron(b)),
        None => tracing::warn!(
            index,
            "share requested for a source that is no longer listed"
        ),
    }
}

#[tauri::command]
pub fn stop_sharing(ui: State<'_, Ui>, stream: u32) {
    send(&ui, UiCommand::StopDelen(stream));
}

/// The camera on or off. The engine picks the device and reports through the usual error
/// banner if there is none, so the button needs no knowledge of what is plugged in.
#[tauri::command]
pub fn set_camera(ui: State<'_, Ui>, on: bool) {
    send(&ui, UiCommand::ZetCamera(on));
}

#[tauri::command]
pub fn set_video_settings(ui: State<'_, Ui>, codec: String, fps: u32, bitrate: u32) {
    send(
        &ui,
        UiCommand::ZetVideoInstellingen(crate::config::VideoConfig {
            codec,
            fps,
            bitrate,
        }),
    );
}

#[tauri::command]
pub fn list_audio_devices(_ui: State<'_, Ui>) -> (Vec<String>, Vec<String>) {
    engine::audio_apparaten().unwrap_or_else(|e| {
        tracing::warn!(error = %format!("{e:#}"), "listing audio devices failed");
        (Vec::new(), Vec::new())
    })
}

#[tauri::command]
pub fn set_audio_devices(ui: State<'_, Ui>, input: Option<String>, output: Option<String>) {
    send(&ui, UiCommand::ZetGeluidsapparaten(input, output));
}

/// Which set of notification tones, and how loud. Saved to `config.toml` by the engine;
/// picking a different set plays it once so you hear what you chose.
#[tauri::command]
pub fn set_sound_settings(ui: State<'_, Ui>, set: String, volume: f32) {
    send(
        &ui,
        UiCommand::ZetGeluidInstellingen(crate::config::SoundConfig { set, volume }),
    );
}

/// Play one tone so it can be judged. Ignores do-not-disturb on purpose: pressing the
/// button is asking for it.
#[tauri::command]
pub fn preview_sound(ui: State<'_, Ui>, sound: String) {
    send(&ui, UiCommand::ProefGeluid(sound));
}

/// Opens the Windows folder picker for the download folder. Same blocking-thread reason
/// as `pick_and_offer_file` — the dialog is modal.
///
/// The picture folder lives inside the download folder, so a new download folder means a
/// new picture folder, and the webview may read images from there over `asset:`. Opening
/// it here rather than at startup only: the engine moves the files, this side moves the
/// permission, and both use `config::pictures_in` so they cannot disagree.
#[tauri::command]
pub async fn pick_download_dir(app: tauri::AppHandle, ui: State<'_, Ui>) -> Result<(), ()> {
    let picked = rfd::AsyncFileDialog::new().pick_folder().await;
    if let Some(dir) = picked {
        let dir = dir.path().to_path_buf();
        if let Err(e) = app
            .asset_protocol_scope()
            .allow_directory(crate::config::pictures_in(&dir), false)
        {
            tracing::warn!(error = %e, "the new picture folder could not be opened for reading");
        }
        send(&ui, UiCommand::ZetDownloadMap(dir));
    }
    Ok(())
}

/// Opens the Windows folder picker for the clips folder. Same blocking-thread reason
/// as `pick_download_dir` — the dialog is modal.
#[tauri::command]
pub async fn pick_clips_dir(ui: State<'_, Ui>) -> Result<(), ()> {
    let picked = rfd::AsyncFileDialog::new().pick_folder().await;
    if let Some(dir) = picked {
        send(&ui, UiCommand::ZetClipMap(dir.path().to_path_buf()));
    }
    Ok(())
}

#[tauri::command]
pub fn set_display_name(ui: State<'_, Ui>, name: String) {
    let name = name.trim().to_string();
    if !name.is_empty() {
        send(&ui, UiCommand::ZetNaam(name));
    }
}

/// Opens the Windows file picker and offers whatever comes back. Runs on a blocking
/// thread: the dialog is modal and would otherwise hold up the IPC handler for as long
/// as the user browses.
#[tauri::command]
pub async fn pick_and_offer_file(ui: State<'_, Ui>, channel: String) -> Result<(), ()> {
    let picked = rfd::AsyncFileDialog::new().pick_file().await;
    if let Some(file) = picked {
        offer_path(&ui, file.path().to_path_buf(), &channel);
    }
    Ok(())
}

/// Files dropped onto the window from Explorer. Same flow as the picker — a new way in,
/// not new logic.
///
/// B-52: takes **indices** into the paths the drop handler kept in `Ui::dropped`, never
/// paths from the webview. A path coming over IPC would let any script in the webview
/// offer any file on the disk to every peer; an index can only ever name a file the user
/// physically dropped on the window. Out-of-range indices are dropped silently — a stale
/// index is a race with the next drop, not something to report.
#[tauri::command]
pub fn offer_files(ui: State<'_, Ui>, indices: Vec<usize>, channel: String) {
    let paden: Vec<PathBuf> = {
        let bewaard = ui.dropped.lock().unwrap_or_else(|e| e.into_inner());
        indices
            .iter()
            .filter_map(|&i| bewaard.get(i).cloned())
            .collect()
    };
    if paden.len() != indices.len() {
        tracing::warn!(
            gevraagd = indices.len(),
            gevonden = paden.len(),
            "drop-indices verwijzen niet allemaal naar een bewaard pad"
        );
    }
    for path in paden {
        offer_path(&ui, path, &channel);
    }
}

/// An image pasted into the composer. The webview hands over the real bytes from its own
/// `paste` event, which is the whole reason the `GetAsyncKeyState` detour from decision
/// 15 is gone: egui never let the app see this event at all.
///
/// The temporary file does not need to survive. `hash_en_bied_aan` in `engine.rs` makes
/// its own content-addressed copy in `pictures_dir` while hashing, exactly as it does for
/// a dragged or picked file whose original it also leaves alone.
#[tauri::command]
pub fn offer_pasted_image(ui: State<'_, Ui>, bytes: Vec<u8>, extension: String, channel: String) {
    // B-53: een allowlist, niet een opschoning. `trim_matches('.')` haalde punten weg maar
    // geen scheidingstekens, dus een "extensie" als `\..\..\..\Startup\evil.exe` loste op
    // naar buiten `%TEMP%` — en `bytes` is volledig door de aanroeper bepaald, dus dat was
    // een schrijfprimitief met vrije inhoud én vrije bestemming, bereikbaar voor elk script
    // in de webview. Deze vijf zijn precies wat `is_afbeelding` verderop ook accepteert.
    let extension = match extension.trim_matches('.').to_ascii_lowercase().as_str() {
        e @ ("png" | "jpg" | "jpeg" | "gif" | "bmp") => e.to_string(),
        _ => "png".to_string(),
    };
    let name = format!(
        "fitcom-paste-{}.{extension}",
        chrono::Local::now().format("%Y%m%d-%H%M%S%3f")
    );
    let path = std::env::temp_dir().join(name);
    if let Err(e) = std::fs::write(&path, bytes) {
        tracing::warn!(error = %e, path = %path.display(), "writing the pasted image failed");
        return;
    }
    offer_path(&ui, path, &channel);
}

fn offer_path(ui: &Ui, path: PathBuf, channel: &str) {
    send(
        ui,
        UiCommand::BiedBestandAan(path, state::parse_channel(channel)),
    );
}

#[tauri::command]
pub fn download_file(ui: State<'_, Ui>, op: OpRef) {
    if let Some(id) = op.to_op_id() {
        send(&ui, UiCommand::DownloadBestand(id));
    }
}

/// Opens a file we already have the bytes of — our own offer, or a finished download.
///
/// The frontend hands back the `OpRef` it was given, never a path: the path is looked up
/// in the engine's own snapshot (`FileView::local_path`), so this command cannot be talked
/// into opening something else. Same reasoning as `offer_files`, which takes indices into
/// `Ui::dropped` rather than paths (B-52): the webview names *which item*, this side
/// decides *which bytes*.
///
/// An extension the shell would execute gets the containing folder instead of the file.
/// See `files::opent_als_code`.
#[tauri::command]
pub fn open_file(ui: State<'_, Ui>, op: OpRef) {
    let Some(id) = op.to_op_id() else { return };
    let snap = ui.engine.snapshot.borrow().clone();
    let Some(path) = snap
        .files
        .iter()
        .find(|f| f.id == id)
        .and_then(|f| f.local_path.clone())
    else {
        tracing::warn!(?id, "open refused: this machine has no copy of that file");
        return;
    };
    if !path.exists() {
        tracing::warn!(path = %path.display(), "open refused: the file is gone");
        return;
    }

    // Same call the timeline made when it labelled the button, so what happens is what
    // the button said. See `files::opent_als_code`.
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    if crate::files::opent_als_code(&name) {
        tracing::info!(path = %path.display(), "executable: showing the folder instead of opening");
        reveal_in_folder(&path);
    } else {
        open_with_shell(&path);
    }
}

/// Hands a path to whatever the OS opens it with. Never used on a path that came from the
/// webview — see [`open_file`].
fn open_with_shell(path: &Path) {
    #[cfg(windows)]
    {
        use windows::core::{w, HSTRING, PCWSTR};
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        let target = HSTRING::from(path);
        // One argument, no shell parsing — same reason as `open_link`: a filename may
        // contain `&`, and through `cmd /c start` that would be a second command.
        unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                PCWSTR(target.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            );
        }
    }

    #[cfg(target_os = "macos")]
    if let Err(e) = std::process::Command::new("open").arg(path).spawn() {
        tracing::warn!(error = %e, path = %path.display(), "opening the file failed");
    }
}

/// Shows the file in Explorer/Finder with it selected, without opening it.
fn reveal_in_folder(path: &Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // `/select,<path>` is one argument, so it cannot go through `ShellExecuteW`'s verb
        // form. And it has to be `raw_arg`, not `arg`: Rust quotes an argument containing
        // spaces, which would produce `"/select,C:\dir with spaces\a.txt"` — a form
        // explorer answers by opening its default folder and selecting nothing. The
        // quotes belong around the *path* only, which is why the command line is written
        // out by hand here.
        //
        // Nothing to escape: `"` is not a legal character in a Windows filename, so the
        // quoted path cannot end early. No shell is involved either — this string goes
        // straight to `CreateProcessW`.
        let arg = format!("/select,\"{}\"", path.display());
        if let Err(e) = std::process::Command::new("explorer.exe")
            .raw_arg(arg)
            .spawn()
        {
            tracing::warn!(error = %e, path = %path.display(), "showing the folder failed");
        }
    }

    #[cfg(target_os = "macos")]
    if let Err(e) = std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .spawn()
    {
        tracing::warn!(error = %e, path = %path.display(), "showing the folder failed");
    }
}

#[tauri::command]
pub fn delete_all_images(ui: State<'_, Ui>) {
    send(&ui, UiCommand::VerwijderAlleAfbeeldingen);
}

/// Clips aan of uit (fase 15). De motor legt dit meteen vast in de config, dus de
/// schakelaar onthoudt zichzelf over herstarts.
#[tauri::command]
pub fn set_clips(ui: State<'_, Ui>, enabled: bool) {
    send(&ui, UiCommand::ZetClips(enabled));
}

/// Nú een clip schrijven. Zelfde weg als de globale hotkey Ctrl+Alt+C.
#[tauri::command]
pub fn clip_now(ui: State<'_, Ui>) {
    send(&ui, UiCommand::ClipseNu);
}

/// Opent de clipmap in de verkenner. Het pad komt uit de snapshot, nooit uit het
/// webview — zelfde regel als `open_file`.
#[tauri::command]
pub fn open_clips_folder(ui: State<'_, Ui>) {
    let snap = ui.engine.snapshot.borrow().clone();
    let Some(clips) = &snap.clips else { return };
    let path = std::path::PathBuf::from(&clips.map);
    if path.exists() {
        open_with_shell(&path);
    }
}

/// De schermen waaruit gekozen kan worden voor de clipopname.
#[tauri::command]
pub fn clip_monitors() -> Vec<String> {
    crate::clips::monitoren().unwrap_or_default()
}

/// Welk scherm er voor clips opgenomen wordt. Leeg = automatisch (eerste).
#[tauri::command]
pub fn set_clip_monitor(ui: State<'_, Ui>, name: String) {
    send(&ui, UiCommand::ZetClipMonitor(name));
}

/// De globale sneltoets wisselen, zonder herstart.
#[tauri::command]
pub fn set_clip_hotkey(ui: State<'_, Ui>, hotkey: String) {
    send(&ui, UiCommand::ZetClipsHotkey(hotkey));
}

#[tauri::command]
pub fn create_channel(ui: State<'_, Ui>, title: String) {
    let title = title.trim().to_string();
    if !title.is_empty() {
        send(&ui, UiCommand::MaakKanaal(title));
    }
}

#[tauri::command]
pub fn rename_channel(ui: State<'_, Ui>, channel: String, title: String) {
    let title = title.trim().to_string();
    if let (Some(topic), false) = (state::parse_channel(&channel).topic_id(), title.is_empty()) {
        send(&ui, UiCommand::HernoemKanaal(topic, title));
    }
}

#[tauri::command]
pub fn delete_channel(ui: State<'_, Ui>, channel: String) {
    if let Some(topic) = state::parse_channel(&channel).topic_id() {
        send(&ui, UiCommand::VerwijderKanaal(topic));
    }
}

#[tauri::command]
pub fn check_update(ui: State<'_, Ui>) {
    send(&ui, UiCommand::ZoekUpdate);
}

#[tauri::command]
pub fn apply_update(ui: State<'_, Ui>) {
    send(&ui, UiCommand::PasUpdateToe);
}

#[tauri::command]
pub fn ignore_update(ui: State<'_, Ui>, version: String) {
    send(&ui, UiCommand::NegeerUpdate(version));
}

#[tauri::command]
pub fn dismiss_update(ui: State<'_, Ui>) {
    send(&ui, UiCommand::WisUpdateMelding);
}

/// A link from the chat, handed to the system browser. The webview itself must never
/// navigate — it holds the whole app — so the frontend cancels the click and calls this.
///
/// The URL comes out of a message from another machine, so only `http(s)` gets through and
/// it is passed as one argument to the shell, never through a command line it could break
/// out of (`&` in a URL is ordinary; in `cmd /c start` it is a second command).
#[tauri::command]
pub fn open_link(url: String) {
    let schema_ok = url.starts_with("https://") || url.starts_with("http://");
    if !schema_ok || url.contains(['\n', '\r', '\0']) {
        tracing::warn!(url, "link not opened: only http(s) is followed");
        return;
    }

    #[cfg(windows)]
    {
        use windows::core::{w, HSTRING, PCWSTR};
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        let doel = HSTRING::from(&url);
        // Geen console-venster, geen shell-parsing: de URL is één argument.
        unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                PCWSTR(doel.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            );
        }
    }

    #[cfg(target_os = "macos")]
    if let Err(e) = std::process::Command::new("open").arg(&url).spawn() {
        tracing::warn!(error = %e, url, "opening the link failed");
    }
}

/// What the chat shows next to a YouTube link. `thumbnail` is a path, not an URL: the
/// frontend turns it into an `asset:` URL, the same way it does for a shared picture.
#[derive(Serialize)]
pub struct YoutubePreview {
    pub title: String,
    pub author: String,
    pub thumbnail: String,
}

/// Title and thumbnail for one video, from the cache on disk or — once, ever — from
/// YouTube. `None` means "no card": an unknown id, no internet, a video that was taken
/// down. The link stays a link, which is what it was before this existed.
///
/// The frontend passes the eleven-character id it found in the message body, never an URL,
/// and `youtube::geldig_id` checks it again on this side: the id ends up in a request URL
/// and in a filename, and it came out of a message a peer typed.
///
/// `spawn_blocking`: ureq is synchronous, and this runs while the timeline is being drawn.
/// On the IPC handler's thread the window would stall for the length of a round trip to
/// Google.
///
/// The `Result` wrapper is Tauri's requirement, not a second failure channel: an async
/// command that borrows state has to return one (same as `pick_download_dir`). Nothing
/// ever comes back as `Err`; "no card" is `Ok(None)`.
#[tauri::command]
pub async fn youtube_preview(ui: State<'_, Ui>, id: String) -> Result<Option<YoutubePreview>, ()> {
    let dir = ui.youtube_dir.clone();
    let uitkomst =
        tauri::async_runtime::spawn_blocking(move || crate::youtube::preview(&id, &dir)).await;
    Ok(match uitkomst {
        Ok(Ok(p)) => Some(YoutubePreview {
            title: p.title,
            author: p.author,
            thumbnail: p.thumbnail.display().to_string(),
        }),
        Ok(Err(e)) => {
            // Debug and not warn: being offline is a normal state here (invariant 7), and
            // every message with a link would otherwise put a line in the log.
            tracing::debug!(error = %format!("{e:#}"), "no youtube preview");
            None
        }
        Err(e) => {
            tracing::warn!(error = %e, "the youtube preview task did not finish");
            None
        }
    })
}

/// One guess at today's Wordle. Only the word travels; the answer stays in the engine and
/// the marked board comes back in the next `state` event — see `crate::wordle`.
///
/// Nothing is returned here on purpose: whether a guess was accepted, and why not, is part
/// of the state the window draws, exactly like a download's progress. That keeps the
/// `Snapshot`/`UiCommand` boundary the way decision 19 left it.
#[tauri::command]
pub fn wordle_guess(ui: State<'_, Ui>, word: String) {
    send(&ui, UiCommand::WordleGok(word));
}

/// Put today's Wordle card in #general for everyone — the rescue hatch in the + menu, for
/// when someone's automatic fetch failed and they are drawing no card at all. Fetches the
/// puzzle first if this machine is the one missing it.
#[tauri::command]
pub fn post_wordle_card(ui: State<'_, Ui>) {
    send(&ui, UiCommand::WordleInChat);
}

/// The close button. With `minimize_to_tray` on — the default — it hides the window and
/// leaves the engine running, because the state this app has to be good at is "away in
/// the tray while a game is running".
#[tauri::command]
pub fn close_window(app: tauri::AppHandle, ui: State<'_, Ui>) {
    if ui.minimize_to_tray {
        #[cfg(windows)]
        tray::verberg_venster();
        #[cfg(not(windows))]
        if let Some(w) = app.get_webview_window("main") {
            let _ = w.hide();
        }
    } else if let Some(w) = app.get_webview_window("main") {
        let _ = w.close();
    }
}
