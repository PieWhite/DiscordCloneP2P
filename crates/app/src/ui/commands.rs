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
use std::path::PathBuf;
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
    state::timeline_of(
        &snap,
        channel,
        ui.me,
        &ui.display_name(&snap),
        &ui.pictures_dir,
    )
}

#[tauri::command]
pub fn send_message(ui: State<'_, Ui>, channel: String, text: String) {
    let text = text.trim().to_string();
    if text.is_empty() {
        return;
    }
    send(&ui, UiCommand::Plaats(text, state::parse_channel(&channel)));
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
    *ui.sources.lock().unwrap() = sources;
    listed
}

#[tauri::command]
pub fn share_source(ui: State<'_, Ui>, index: usize) {
    let source = ui.sources.lock().unwrap().get(index).cloned();
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
#[tauri::command]
pub fn offer_files(ui: State<'_, Ui>, paths: Vec<PathBuf>, channel: String) {
    for path in paths {
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
    let extension = match extension.trim_matches('.') {
        "" => "png".to_string(),
        e => e.to_ascii_lowercase(),
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

#[tauri::command]
pub fn delete_all_images(ui: State<'_, Ui>) {
    send(&ui, UiCommand::VerwijderAlleAfbeeldingen);
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
