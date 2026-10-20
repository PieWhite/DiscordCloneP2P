//! The other half of the boundary: everything the window can ask the engine to do.
//!
//! One function per `UiCommand` variant, plus the handful of things that are genuinely
//! the display layer's job (a file picker, the device list, the window buttons). Nothing
//! here decides anything: it translates a call from a view into a message on the
//! engine's channel and returns. Views call these and nothing else: no view touches
//! `engine.commands` itself.
//!
//! These used to be Tauri commands, and the synchronous ones ran on the main thread there
//! too, so they stay synchronous. The four that wait on a person or the network
//! (`pick_download_dir`, `pick_clips_dir`, `pick_and_offer_file`, `youtube_preview`) run
//! on the tokio runtime instead and never hold up a frame.

use super::state::{self, OpRef, Recap, TimelineItem, UiState};
use super::Ui;
use crate::engine::{self, UiCommand};
#[cfg(windows)]
use crate::tray;
pub use fitcom_proto::control::MusicAction;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Commands are fire-and-forget: the result of one is a new snapshot, which arrives as a
/// state update like any other change. A full channel means the engine is wedged, which
/// is worth a log line and nothing else — dropping the command is better than blocking
/// the window's thread.
fn send(ui: &Ui, cmd: UiCommand) {
    if let Err(e) = ui.engine.commands.try_send(cmd) {
        tracing::warn!(error = %e, "command not handed to the engine");
    }
}

pub fn get_state(ui: &Ui) -> UiState {
    ui.state()
}

pub fn get_timeline(ui: &Ui, channel: &str) -> Vec<TimelineItem> {
    #[cfg(debug_assertions)]
    if let Some(f) = &ui.fixture {
        return f.timeline(channel);
    }
    let snap = ui.engine.snapshot.borrow().clone();
    let channel = state::parse_channel(channel);
    state::timeline_of(&snap, channel, ui.me, &ui.display_name(&snap))
}

/// The last seven days. Fetched when the Recap panel opens rather than pushed with the
/// state, because every figure in it moves while a call is running.
pub fn get_recap(ui: &Ui) -> Recap {
    #[cfg(debug_assertions)]
    if let Some(f) = &ui.fixture {
        return f.recap.clone();
    }
    let snap = ui.engine.snapshot.borrow().clone();
    state::recap_of(&snap, ui.me)
}

/// Send a chat message. `reply_to` is `Some` when the composer had a reply open; the
/// engine validates that it points into this same conversation.
pub fn send_message(ui: &Ui, channel: &str, text: &str, reply_to: Option<&OpRef>) {
    let text = text.trim().to_string();
    if text.is_empty() {
        return;
    }
    let reply_to = reply_to.and_then(|r| r.to_op_id());
    send(
        ui,
        UiCommand::Plaats(text, state::parse_channel(channel), reply_to),
    );
}

/// Add a reaction, or take it back if we already reacted with this emoji — the toggle
/// lives in the chat layer, which knows what is currently on the message.
pub fn react_message(ui: &Ui, op: &OpRef, emoji: &str) {
    let Some(id) = op.to_op_id() else { return };
    let emoji = emoji.trim().to_string();
    if emoji.is_empty() {
        return;
    }
    send(ui, UiCommand::Reageer(id, emoji));
}

/// "I am typing in this conversation." Fire-and-forget and throttled on the engine side;
/// the composer may call it on every keystroke.
pub fn notify_typing(ui: &Ui, channel: &str) {
    send(ui, UiCommand::Typing(state::parse_channel(channel)));
}

/// Our own presence status: "online", "away" or "busy". Unknown values are dropped here
/// rather than corrected there.
pub fn set_user_status(ui: &Ui, status: &str) {
    let status = match status {
        "online" => 0,
        "away" => 1,
        "busy" => 2,
        _ => return,
    };
    send(ui, UiCommand::ZetStatus(status));
}

pub fn edit_message(ui: &Ui, op: &OpRef, text: &str) {
    let text = text.trim().to_string();
    let Some(id) = op.to_op_id() else { return };
    if text.is_empty() {
        send(ui, UiCommand::Verwijder(id));
    } else {
        send(ui, UiCommand::Bewerk(id, text));
    }
}

pub fn delete_message(ui: &Ui, op: &OpRef) {
    if let Some(id) = op.to_op_id() {
        send(ui, UiCommand::Verwijder(id));
    }
}

/// Opening a conversation clears its own unread counter and nothing else — the three
/// counters (general, per sub-channel, per DM) are tracked separately on purpose.
pub fn mark_read(ui: &Ui, channel: &str) {
    let channel = state::parse_channel(channel);
    let cmd = if let Some(peer) = channel.dm_peer() {
        UiCommand::GelezenDm(peer)
    } else if let Some(topic) = channel.topic_id() {
        UiCommand::GelezenTopic(topic)
    } else {
        UiCommand::Gelezen
    };
    send(ui, cmd);
}

pub fn dismiss_error(ui: &Ui) {
    send(ui, UiCommand::FoutWeg);
}

pub fn set_joined(ui: &Ui, joined: bool) {
    send(
        ui,
        if joined {
            UiCommand::VoiceDeelnemen
        } else {
            UiCommand::VoiceVerlaten
        },
    );
}

pub fn set_muted(ui: &Ui, muted: bool) {
    send(ui, UiCommand::Mute(muted));
}

pub fn set_deafened(ui: &Ui, deafened: bool) {
    send(ui, UiCommand::Deafen(deafened));
}

pub fn set_do_not_disturb(ui: &Ui, on: bool) {
    send(ui, UiCommand::NietStoren(on));
}

pub fn set_peer_volume(ui: &Ui, peer: &str, volume: f32) {
    if let Some(p) = super::parse_peer(peer) {
        send(ui, UiCommand::Volume(p, volume));
    }
}

pub fn set_stream_volume(ui: &Ui, peer: &str, stream: u32, volume: f32) {
    if let Some(p) = super::parse_peer(peer) {
        send(ui, UiCommand::StreamVolume(p, stream, volume));
    }
}

/// Play/pause or next on the Spotify `peer` shares. The DJ decides whether to do it
/// (only for someone in the call); nothing comes back but the next title.
pub fn music_control(ui: &Ui, peer: &str, stream: u32, action: MusicAction) {
    if let Some(p) = super::parse_peer(peer) {
        send(ui, UiCommand::BedienMuziek(p, stream, action));
    }
}

pub fn set_watching(ui: &Ui, peer: &str, stream: u32, watching: bool) {
    if let Some(p) = super::parse_peer(peer) {
        send(
            ui,
            if watching {
                UiCommand::Kijken(p, stream)
            } else {
                UiCommand::StopKijken(p, stream)
            },
        );
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceOption {
    pub index: usize,
    pub name: String,
    pub is_window: bool,
    pub is_camera: bool,
}

/// Fetched when the picker opens, never cached: windows come and go, and a stale list
/// offers something that is no longer there.
pub fn list_sources(ui: &Ui) -> Vec<SourceOption> {
    #[cfg(debug_assertions)]
    if let Some(f) = &ui.fixture {
        return f.sources.clone();
    }
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

pub fn share_source(ui: &Ui, index: usize) {
    let source = ui
        .sources
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(index)
        .cloned();
    match source {
        Some(b) => send(ui, UiCommand::DeelBron(b)),
        None => tracing::warn!(
            index,
            "share requested for a source that is no longer listed"
        ),
    }
}

/// Forget a peer on this machine: no more connections, pin and addresses gone.
pub fn remove_peer(ui: &Ui, slot: usize) {
    send(ui, UiCommand::VerwijderPeer(slot));
}

/// Share the window of the game we are playing, as Discord's in-game stream button does.
pub fn share_game(ui: &Ui) {
    send(ui, UiCommand::DeelSpel);
}

pub fn stop_sharing(ui: &Ui, stream: u32) {
    send(ui, UiCommand::StopDelen(stream));
}

/// May viewers draw on the screens we share, now and later (decision 45). Saved to
/// `config.toml`; off also wipes what is there.
pub fn set_drawing(ui: &Ui, on: bool) {
    send(ui, UiCommand::ZetTekenen(on));
}

/// Wipe every line on this screen of ours, whoever drew it.
pub fn clear_drawing(ui: &Ui, stream: u32) {
    send(ui, UiCommand::WisTekening(stream));
}

/// The camera on or off. The engine picks the device and reports through the usual error
/// banner if there is none, so the button needs no knowledge of what is plugged in.
pub fn set_camera(ui: &Ui, on: bool) {
    send(ui, UiCommand::ZetCamera(on));
}

/// Share Spotify's sound with the call, or stop. Same as the camera: the engine finds
/// Spotify and reports through the error banner when it is not running.
pub fn set_spotify(ui: &Ui, on: bool) {
    send(ui, UiCommand::DeelSpotify(on));
}

pub fn set_video_settings(ui: &Ui, codec: &str, fps: u32, bitrate: u32) {
    send(
        ui,
        UiCommand::ZetVideoInstellingen(crate::config::VideoConfig {
            codec: codec.to_string(),
            fps,
            bitrate,
        }),
    );
}

pub fn list_audio_devices(ui: &Ui) -> (Vec<String>, Vec<String>) {
    #[cfg(debug_assertions)]
    if let Some(f) = &ui.fixture {
        return f.devices.clone();
    }
    let _ = ui;
    engine::audio_apparaten().unwrap_or_else(|e| {
        tracing::warn!(error = %format!("{e:#}"), "listing audio devices failed");
        (Vec::new(), Vec::new())
    })
}

pub fn set_audio_devices(ui: &Ui, input: Option<String>, output: Option<String>) {
    send(ui, UiCommand::ZetGeluidsapparaten(input, output));
}

/// Which set of notification tones, and how loud. Saved to `config.toml` by the engine;
/// picking a different set plays it once so you hear what you chose.
pub fn set_sound_settings(ui: &Ui, set: &str, volume: f32) {
    send(
        ui,
        UiCommand::ZetGeluidInstellingen(crate::config::SoundConfig {
            set: set.to_string(),
            volume,
        }),
    );
}

/// How the microphone gate works and which key operates it. Saved to `config.toml` by the
/// engine and live at once, also in a running call. `mode` is `voice`, `talk` or `mute`;
/// `key` is a name from `fitcom_audio::spreektoets::TOETSEN`.
pub fn set_push_to_talk(ui: &Ui, mode: &str, key: &str) {
    send(
        ui,
        UiCommand::ZetSpreektoets(crate::config::PushToTalkConfig {
            mode: mode.to_string(),
            key: key.to_string(),
        }),
    );
}

/// Whether the game in front is shared as "Playing X" under your name. Saved to
/// `config.toml` by the engine; off also clears what the others still show.
pub fn set_share_activity(ui: &Ui, on: bool) {
    send(ui, UiCommand::ZetActiviteitDelen(on));
}

/// Play one tone so it can be judged. Ignores do-not-disturb on purpose: pressing the
/// button is asking for it.
pub fn preview_sound(ui: &Ui, sound: &str) {
    send(ui, UiCommand::ProefGeluid(sound.to_string()));
}

/// Opens the folder picker for the download folder, on the tokio runtime: the dialog is
/// modal, and the window must keep drawing while the user browses.
///
/// The picture folder lives inside the download folder, so a new download folder means a
/// new picture folder. There is no permission to move along with it any more: what the
/// window may draw from disk is decided per image by [`Ui::image_path_allowed`], which
/// reads the folder from the live snapshot, so the engine moving the files is enough.
pub fn pick_download_dir(ui: &Arc<Ui>) {
    ui.runtime.spawn({
        let ui = ui.clone();
        async move {
            if let Some(dir) = rfd::AsyncFileDialog::new().pick_folder().await {
                send(&ui, UiCommand::ZetDownloadMap(dir.path().to_path_buf()));
            }
        }
    });
}

/// Opens the folder picker for the clips folder. Same reason as `pick_download_dir`.
pub fn pick_clips_dir(ui: &Arc<Ui>) {
    ui.runtime.spawn({
        let ui = ui.clone();
        async move {
            if let Some(dir) = rfd::AsyncFileDialog::new().pick_folder().await {
                send(&ui, UiCommand::ZetClipMap(dir.path().to_path_buf()));
            }
        }
    });
}

/// Pasted text with one or more invite codes; the engine fishes them out (phase B1).
pub fn add_peer_codes(ui: &Ui, text: &str) {
    if !text.trim().is_empty() {
        send(ui, UiCommand::VoegCodesToe(text.to_string()));
    }
}

pub fn set_display_name(ui: &Ui, name: &str) {
    let name = name.trim().to_string();
    if !name.is_empty() {
        send(ui, UiCommand::ZetNaam(name));
    }
}

/// Opens the file picker and offers whatever comes back. Runs on the tokio runtime: the
/// dialog is modal and would otherwise hold up the window for as long as the user browses.
pub fn pick_and_offer_file(ui: &Arc<Ui>, channel: &str) {
    let channel = channel.to_string();
    ui.runtime.spawn({
        let ui = ui.clone();
        async move {
            if let Some(file) = rfd::AsyncFileDialog::new().pick_file().await {
                offer_path(&ui, file.path().to_path_buf(), &channel);
            }
        }
    });
}

/// Files dropped onto the window from Explorer or Finder. Same flow as the picker: a new
/// way in, not new logic.
///
/// B-52: takes **indices** into the paths the drop handler kept in `Ui::dropped`, never
/// paths from a view. There is no webview script any more to abuse a path parameter, but
/// the rule stays: an index can only ever name a file the user physically dropped on the
/// window, so no view code can be talked into offering an arbitrary file to every peer.
/// Out-of-range indices are dropped silently: a stale index is a race with the next
/// drop, not something to report.
pub fn offer_files(ui: &Ui, indices: &[usize], channel: &str) {
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
        offer_path(ui, path, channel);
    }
}

/// An image pasted into the composer. The composer hands over the real bytes from the
/// clipboard, which is the whole reason the `GetAsyncKeyState` detour from decision 15 is
/// gone: egui never let the app see this event at all.
///
/// The temporary file does not need to survive. `hash_en_bied_aan` in `engine.rs` makes
/// its own content-addressed copy in `pictures_dir` while hashing, exactly as it does for
/// a dragged or picked file whose original it also leaves alone.
pub fn offer_pasted_image(ui: &Ui, bytes: &[u8], extension: &str, channel: &str) {
    // B-53: een allowlist, niet een opschoning. `trim_matches('.')` haalde punten weg maar
    // geen scheidingstekens, dus een "extensie" als `\..\..\..\Startup\evil.exe` loste op
    // naar buiten `%TEMP%` — en `bytes` is volledig door de aanroeper bepaald, dus dat was
    // een schrijfprimitief met vrije inhoud én vrije bestemming, bereikbaar voor elk script
    // in de toenmalige webview. Deze vijf zijn precies wat `is_afbeelding` verderop ook
    // accepteert.
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
    offer_path(ui, path, channel);
}

fn offer_path(ui: &Ui, path: PathBuf, channel: &str) {
    send(
        ui,
        UiCommand::BiedBestandAan(path, state::parse_channel(channel)),
    );
}

pub fn download_file(ui: &Ui, op: &OpRef) {
    if let Some(id) = op.to_op_id() {
        send(ui, UiCommand::DownloadBestand(id));
    }
}

/// Opens a file we already have the bytes of — our own offer, or a finished download.
///
/// The view hands back the `OpRef` it was given, never a path: the path is looked up
/// in the engine's own snapshot (`FileView::local_path`), so this command cannot be talked
/// into opening something else. Same reasoning as `offer_files`, which takes indices into
/// `Ui::dropped` rather than paths (B-52): the view names *which item*, this side
/// decides *which bytes*.
///
/// An extension the shell would execute gets the containing folder instead of the file.
/// See `files::opent_als_code`.
pub fn open_file(ui: &Ui, op: &OpRef) {
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

/// Hands a path to whatever the OS opens it with. Never used on a path that came from a
/// view: see [`open_file`].
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

pub fn delete_all_images(ui: &Ui) {
    send(ui, UiCommand::VerwijderAlleAfbeeldingen);
}

/// Clips aan of uit (fase 15). De motor legt dit meteen vast in de config, dus de
/// schakelaar onthoudt zichzelf over herstarts.
pub fn set_clips(ui: &Ui, enabled: bool) {
    send(ui, UiCommand::ZetClips(enabled));
}

/// Nú een clip schrijven. Zelfde weg als de globale hotkey Ctrl+Alt+C.
pub fn clip_now(ui: &Ui) {
    send(ui, UiCommand::ClipseNu);
}

/// Offers the last saved clip in the general channel. No path: the engine knows which
/// file that is (same rule as `open_file`).
pub fn share_last_clip(ui: &Ui) {
    send(ui, UiCommand::DeelLaatsteClip);
}

/// "Post clips in the chat automatically" on or off.
pub fn set_clips_auto_share(ui: &Ui, on: bool) {
    send(ui, UiCommand::ZetClipsAutoDelen(on));
}

/// Opent de clipmap in de verkenner. Het pad komt uit de snapshot, nooit uit een
/// view: zelfde regel als `open_file`.
pub fn open_clips_folder(ui: &Ui) {
    let snap = ui.engine.snapshot.borrow().clone();
    let Some(clips) = &snap.clips else { return };
    let path = std::path::PathBuf::from(&clips.map);
    if path.exists() {
        open_with_shell(&path);
    }
}

/// De schermen waaruit gekozen kan worden voor de clipopname.
pub fn clip_monitors(ui: &Ui) -> Vec<String> {
    #[cfg(debug_assertions)]
    if let Some(f) = &ui.fixture {
        return f.clip_monitors.clone();
    }
    let _ = ui;
    crate::clips::monitoren().unwrap_or_default()
}

/// Welk scherm er voor clips opgenomen wordt. Leeg = automatisch (eerste).
pub fn set_clip_monitor(ui: &Ui, name: &str) {
    send(ui, UiCommand::ZetClipMonitor(name.to_string()));
}

/// Hoe lang een clip teruggaat, in seconden.
pub fn set_clip_length(ui: &Ui, seconds: u32) {
    send(ui, UiCommand::ZetClipLengte(seconds));
}

/// De globale sneltoets wisselen, zonder herstart.
pub fn set_clip_hotkey(ui: &Ui, hotkey: &str) {
    send(ui, UiCommand::ZetClipsHotkey(hotkey.to_string()));
}

pub fn create_channel(ui: &Ui, title: &str) {
    let title = title.trim().to_string();
    if !title.is_empty() {
        send(ui, UiCommand::MaakKanaal(title));
    }
}

pub fn rename_channel(ui: &Ui, channel: &str, title: &str) {
    let title = title.trim().to_string();
    if let (Some(topic), false) = (state::parse_channel(channel).topic_id(), title.is_empty()) {
        send(ui, UiCommand::HernoemKanaal(topic, title));
    }
}

pub fn delete_channel(ui: &Ui, channel: &str) {
    if let Some(topic) = state::parse_channel(channel).topic_id() {
        send(ui, UiCommand::VerwijderKanaal(topic));
    }
}

pub fn check_update(ui: &Ui) {
    send(ui, UiCommand::ZoekUpdate);
}

pub fn apply_update(ui: &Ui) {
    send(ui, UiCommand::PasUpdateToe);
}

pub fn ignore_update(ui: &Ui, version: &str) {
    send(ui, UiCommand::NegeerUpdate(version.to_string()));
}

pub fn dismiss_update(ui: &Ui) {
    send(ui, UiCommand::WisUpdateMelding);
}

/// A link from the chat, handed to the system browser.
///
/// The URL comes out of a message from another machine, so only `http(s)` gets through and
/// it is passed as one argument to the shell, never through a command line it could break
/// out of (`&` in a URL is ordinary; in `cmd /c start` it is a second command).
pub fn open_link(url: &str) {
    let schema_ok = url.starts_with("https://") || url.starts_with("http://");
    if !schema_ok || url.contains(['\n', '\r', '\0']) {
        tracing::warn!(url, "link not opened: only http(s) is followed");
        return;
    }
    shell_open(url);
}

/// A Spotify track from the chat, opened in this machine's Spotify app as
/// `spotify:track:<id>` — the DJ's button on an `open.spotify.com/track/…` link
/// (decision 41). Spotify shows the track; playing or queueing it is the DJ's own click
/// there. Without the Web API there is no "add to queue" from outside the app.
///
/// The id came out of a message a peer typed, and it goes into a URI: it is checked again
/// here (`muziek::geldig_track_id`, 22 characters base62), whatever the view found. If
/// Windows reports that nothing took the URI, the web player gets the same id instead — a
/// safety net only: the DJ cannot share without the app running.
pub fn open_spotify_track(id: &str) {
    if !crate::muziek::geldig_track_id(id) {
        tracing::warn!(id, "not a Spotify track id; not opened");
        return;
    }
    if !shell_open(&format!("spotify:track:{id}")) {
        shell_open(&format!("https://open.spotify.com/track/{id}"));
    }
}

/// Hands one URI to the system, as one argument and never through a command line.
/// `false` when nothing took it (on Windows: no handler registered for the scheme).
fn shell_open(target: &str) -> bool {
    #[cfg(windows)]
    {
        use windows::core::{w, HSTRING, PCWSTR};
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        let doel = HSTRING::from(target);
        // Geen console-venster, geen shell-parsing: de URL is één argument.
        let h = unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                PCWSTR(doel.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        // ShellExecute's documented success test: a value above 32.
        let gelukt = h.0 as isize > 32;
        if !gelukt {
            tracing::warn!(target, code = h.0 as isize, "opening failed");
        }
        gelukt
    }

    #[cfg(target_os = "macos")]
    {
        // Spawned, not awaited: the click must not wait on an app launch. So `true` only
        // means `open` started, not that an app took the URI.
        match std::process::Command::new("open").arg(target).spawn() {
            Ok(_) => true,
            Err(e) => {
                tracing::warn!(error = %e, target, "opening failed");
                false
            }
        }
    }
}

/// What the chat shows next to a YouTube link. `thumbnail` is a path on our own disk, in
/// the YouTube cache; draw it only through [`Ui::image_path_allowed`].
#[derive(Debug, Clone, PartialEq)]
pub struct YoutubePreview {
    pub title: String,
    pub author: String,
    pub thumbnail: PathBuf,
}

/// Title and thumbnail for one video, from the cache on disk or — once, ever — from
/// YouTube. `None` means "no card": an unknown id, no internet, a video that was taken
/// down. The link stays a link, which is what it was before this existed.
///
/// The view passes the eleven-character id it found in the message body, never an URL,
/// and `youtube::geldig_id` checks it again on this side: the id ends up in a request URL
/// and in a filename, and it came out of a message a peer typed.
///
/// `spawn_blocking` on the tokio runtime: ureq is synchronous, and this is asked for while
/// the timeline is being drawn. The returned future is executor-agnostic, so a view awaits
/// it in `cx.spawn` and updates itself when it resolves.
pub fn youtube_preview(ui: &Ui, id: &str) -> impl Future<Output = Option<YoutubePreview>> {
    let id = id.to_string();
    let dir = ui.youtube_dir.clone();
    // A fixture never reaches the network: only what is already in its cache gets a card.
    #[cfg(debug_assertions)]
    let cache_only = ui.fixture.is_some();
    #[cfg(not(debug_assertions))]
    let cache_only = false;
    let taak = ui.runtime.spawn_blocking(move || {
        if cache_only {
            // Same id check as `preview`: the id becomes a filename here too.
            crate::youtube::geldig_id(&id)
                .then(|| crate::youtube::uit_cache(&id, &dir))
                .flatten()
                .ok_or_else(|| anyhow::anyhow!("not in the fixture cache"))
        } else {
            crate::youtube::preview(&id, &dir)
        }
    });
    async move {
        match taak.await {
            Ok(Ok(p)) => Some(YoutubePreview {
                title: p.title,
                author: p.author,
                thumbnail: p.thumbnail,
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
        }
    }
}

/// One guess at today's Wordle. Only the word travels; the answer stays in the engine and
/// the marked board comes back in the next state update: see `crate::wordle`.
///
/// Nothing is returned here on purpose: whether a guess was accepted, and why not, is part
/// of the state the window draws, exactly like a download's progress. That keeps the
/// `Snapshot`/`UiCommand` boundary the way decision 19 left it.
pub fn wordle_guess(ui: &Ui, word: &str) {
    send(ui, UiCommand::WordleGok(word.to_string()));
}

/// Put today's Wordle card in #general for everyone — the rescue hatch in the + menu, for
/// when someone's automatic fetch failed and they are drawing no card at all. Fetches the
/// puzzle first if this machine is the one missing it.
pub fn post_wordle_card(ui: &Ui) {
    send(ui, UiCommand::WordleInChat);
}

/// The close button, and the OS close request (the traffic light, Alt+F4, the Windows
/// caption button) all land here. With `minimize_to_tray` on (the default) it hides the
/// window and leaves the engine running, because the state this app has to be good at is
/// "away in the tray while a game is running". Returns whether the app should really quit.
pub fn close_window(ui: &Ui, cx: &mut gpui_kit::App) -> bool {
    if !ui.minimize_to_tray {
        return true;
    }
    // Hidden is not in the foreground: the engine decides on notifications with this.
    ui.engine
        .voorgrond
        .store(false, std::sync::atomic::Ordering::Relaxed);
    #[cfg(windows)]
    {
        let _ = cx;
        tray::verberg_venster();
    }
    // AppKit has no per-window hide that the Dock and the tray agree on; hiding the app is
    // the honest equivalent, and "Openen" in the tray activates it again.
    #[cfg(not(windows))]
    cx.hide();
    false
}
