//! Debug builds only: run the window without mesh, store or engine, fed from JSON, so any
//! state can be drawn and screenshotted without a second machine.
//!
//! ```text
//! FITCOM_UI_FIXTURE=<dir>        the files below; only state.json is required
//! FITCOM_DEBUG_VIEW=<script>     initial view state, see `DebugView`
//! FITCOM_DEBUG_SIZE=980x720      first window size
//! ```
//!
//! `<dir>` holds `state.json` (a serialized `UiState`), `timeline/<key>.json` (a list of
//! `TimelineItem`, `<key>` being the channel key with `:` optionally written as `_`, so
//! `general.json`, `topic_<uuid>.json`, `dm_<uuid>.json`), `meters.json`
//! (`{"peers": {"<id>": {"rtt": 12, "level": 0.2}}, "self": {"level": 0.1}}`, the old
//! `meters` event), `recap.json`, `sources.json` (list of `SourceOption`), `devices.json`
//! (`[[inputs], [outputs]]` or `{"input": [..], "output": [..]}`), `monitors.json`, images
//! under `files/`, strip frames as `thumbs/<tile-key>.png`, and `meta.json` whose `now_ms`
//! freezes the clock (`format::now_ms`). Relative image paths in the JSON are resolved
//! against `<dir>`.

use super::bridge::{MeterReadings, PeerMeter, ThumbMap, SPEAKING};
use super::commands::SourceOption;
use super::state::{Recap, TimelineItem, UiState};
use super::view::{Dialog, Overlay, SettingsTab, View};
use anyhow::{Context, Result};
use gpui_kit::RenderImage;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct Fixture {
    pub dir: PathBuf,
    pub state: UiState,
    timelines: HashMap<String, Vec<TimelineItem>>,
    pub recap: Recap,
    pub sources: Vec<SourceOption>,
    pub devices: (Vec<String>, Vec<String>),
    pub clip_monitors: Vec<String>,
    pub meters: MeterReadings,
    pub thumbs: ThumbMap,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Devices {
    Pair(Vec<String>, Vec<String>),
    Named { input: Vec<String>, output: Vec<String> },
}

/// `meta.json`: the frozen clock the reference screenshots were taken with.
#[derive(Deserialize)]
struct Meta {
    now_ms: Option<i64>,
}

#[derive(Deserialize, Default)]
struct MetersFile {
    #[serde(default)]
    peers: HashMap<String, PeerMeter>,
    #[serde(default, rename = "self")]
    me: SelfMeter,
}

#[derive(Deserialize, Default)]
struct SelfMeter {
    #[serde(default)]
    level: f32,
}

fn read<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<Option<T>> {
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let value = serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    Ok(Some(value))
}

fn absolute(dir: &Path, p: &str) -> String {
    let path = Path::new(p);
    if path.is_absolute() || p.is_empty() {
        p.to_string()
    } else {
        dir.join(path).display().to_string()
    }
}

impl Fixture {
    pub fn load(dir: &Path) -> Result<Self> {
        let dir = dir
            .canonicalize()
            .with_context(|| format!("fixture directory {}", dir.display()))?;
        if let Some(now) = read::<Meta>(&dir.join("meta.json"))?.and_then(|m| m.now_ms) {
            super::view::format::freeze_clock(now);
        }
        let mut state: UiState = read(&dir.join("state.json"))?.context("state.json is required")?;
        state.pictures_dir = absolute(&dir, &state.pictures_dir);
        state.download_dir = absolute(&dir, &state.download_dir);

        let mut timelines = HashMap::new();
        if let Ok(entries) = std::fs::read_dir(dir.join("timeline")) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                    continue;
                };
                let key = ["dm", "topic"]
                    .iter()
                    .find_map(|p| {
                        stem.strip_prefix(p)
                            .and_then(|rest| rest.strip_prefix(['_', '-', ':']))
                            .map(|id| format!("{p}:{id}"))
                    })
                    .unwrap_or_else(|| stem.to_string());
                let mut items: Vec<TimelineItem> = read(&path)?.unwrap_or_default();
                for item in &mut items {
                    if let TimelineItem::File { image_path, local_path, .. } = item {
                        for p in [image_path, local_path].into_iter().flatten() {
                            *p = absolute(&dir, p);
                        }
                    }
                }
                timelines.insert(key, items);
            }
        }

        let devices = match read::<Devices>(&dir.join("devices.json"))? {
            Some(Devices::Pair(i, o)) | Some(Devices::Named { input: i, output: o }) => (i, o),
            None => (Vec::new(), Vec::new()),
        };

        let meters_file: MetersFile = read(&dir.join("meters.json"))?.unwrap_or_default();
        let meters = MeterReadings {
            peers: meters_file
                .peers
                .into_iter()
                .map(|(id, mut m)| {
                    // A still picture has no hold: speaking is the level, as the old
                    // frontend decided it on the first sample.
                    m.speaking = m.speaking || m.level > SPEAKING;
                    (id, m)
                })
                .collect(),
            self_level: meters_file.me.level,
            self_speaking: state.voice.joined && !state.voice.muted && meters_file.me.level > SPEAKING,
        };

        let mut thumbs = ThumbMap::new();
        if let Ok(entries) = std::fs::read_dir(dir.join("thumbs")) {
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(key) = path.file_stem().and_then(|s| s.to_str()).map(str::to_string) else {
                    continue;
                };
                match image::open(&path) {
                    Ok(img) => {
                        let mut rgba = img.to_rgba8();
                        // GPUI images are BGRA.
                        for px in rgba.chunks_exact_mut(4) {
                            px.swap(0, 2);
                        }
                        thumbs.insert(key, Arc::new(RenderImage::new(vec![image::Frame::new(rgba)])));
                    }
                    Err(e) => tracing::warn!(error = %e, path = %path.display(), "fixture thumbnail unreadable"),
                }
            }
        }

        Ok(Fixture {
            recap: read(&dir.join("recap.json"))?.unwrap_or(Recap {
                days: 7,
                from: 0,
                rows: Vec::new(),
            }),
            sources: read(&dir.join("sources.json"))?.unwrap_or_default(),
            clip_monitors: read(&dir.join("monitors.json"))?.unwrap_or_default(),
            devices,
            meters,
            thumbs,
            timelines,
            state,
            dir,
        })
    }

    pub fn timeline(&self, channel: &str) -> Vec<TimelineItem> {
        self.timelines.get(channel).cloned().unwrap_or_default()
    }
}

/// `FITCOM_DEBUG_VIEW`: what the window looks at when it opens, so any state can be shot
/// without a click (Accessibility is not granted to the screenshot tooling). Items are
/// separated by `;`, each `key=value`:
///
/// | Item | Effect |
/// |---|---|
/// | `view=channels\|dms\|settings` | the rail |
/// | `channel=<key>` | open a channel (`general`, `topic:<uuid>`) |
/// | `dm=<peer-uuid>\|first` | open a DM |
/// | `tab=account\|audio\|video\|clips\|files\|network\|recap` | settings tab (implies `view=settings`) |
/// | `members=0\|1` | member list |
/// | `collapsed=1` | collapse the channel group |
/// | `overlay=ac\|drop\|plus` | composer overlay; `ac=<query>` sets the @-query |
/// | `dialog=wordle\|picker\|update\|wipe\|delete\|new-channel\|rename\|delete-channel\|lightbox:<path>` | open a dialog |
/// | `wordle=<letters>` | letters typed into the board |
/// | `reply=last` / `edit=last` / `emoji=last` | reply chip, inline edit, quick-emoji bar on the last message (edit: last own) |
/// | `menu=<channel-key>` | channel context menu |
/// | `hotkey=1` | clip hotkey capture waiting |
/// | `focused=0` | window treated as unfocused |
/// | `scroll=top\|<item>[+<px>]` | timeline scrolled to that timeline item (plus pixels into it) |
/// | `hover=<item>\|last` | timeline row drawn hovered (the last message for `last`) |
#[derive(Debug, Default, Clone)]
pub struct DebugView {
    pub view: Option<View>,
    pub channel: Option<String>,
    pub dm: Option<String>,
    pub tab: Option<SettingsTab>,
    pub members: Option<bool>,
    pub collapsed: bool,
    pub overlay: Option<Overlay>,
    pub ac_query: Option<String>,
    pub dialog: Option<DebugDialog>,
    pub wordle_typed: Option<String>,
    pub reply_last: bool,
    pub edit_last: bool,
    pub emoji_last: bool,
    pub menu: Option<String>,
    pub hotkey_wait: bool,
    pub focused: Option<bool>,
    pub scroll: Option<String>,
    pub hover: Option<String>,
}

/// A dialog named in the script. Resolved against the state when it is applied, because
/// most dialogs carry data (the update version, the message to delete).
#[derive(Debug, Clone)]
pub enum DebugDialog {
    Ready(Dialog),
    Update,
    Wipe,
    DeleteLast,
    NewChannel,
    Rename,
    DeleteChannel,
    Picker,
}

impl DebugView {
    pub fn from_env() -> Option<Self> {
        std::env::var("FITCOM_DEBUG_VIEW").ok().map(|s| Self::parse(&s))
    }

    pub fn parse(script: &str) -> Self {
        let mut d = DebugView::default();
        for item in script.split(';').map(str::trim).filter(|i| !i.is_empty()) {
            let (key, value) = item.split_once('=').unwrap_or((item, ""));
            let flag = matches!(value, "1" | "true" | "yes" | "");
            match key {
                "view" => {
                    d.view = match value {
                        "channels" => Some(View::Channels),
                        "dms" => Some(View::Dms),
                        "settings" => Some(View::Settings),
                        _ => None,
                    }
                }
                "channel" => d.channel = Some(value.to_string()),
                "dm" => d.dm = Some(value.to_string()),
                "tab" => d.tab = SettingsTab::parse(value),
                "members" => d.members = Some(flag),
                "collapsed" => d.collapsed = flag,
                "overlay" => {
                    d.overlay = match value {
                        "ac" => Some(Overlay::Autocomplete),
                        "drop" => Some(Overlay::Drop),
                        "plus" => Some(Overlay::Plus),
                        _ => Some(Overlay::None),
                    }
                }
                "ac" => d.ac_query = Some(value.to_string()),
                "dialog" => {
                    d.dialog = match value.split_once(':') {
                        Some(("lightbox", path)) => Some(DebugDialog::Ready(Dialog::Lightbox {
                            src: PathBuf::from(path),
                            name: Path::new(path)
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_default(),
                            op: None,
                            actual_size: false,
                        })),
                        _ => match value {
                            "wordle" => Some(DebugDialog::Ready(Dialog::Wordle)),
                            "picker" => Some(DebugDialog::Picker),
                            "update" => Some(DebugDialog::Update),
                            "wipe" => Some(DebugDialog::Wipe),
                            "delete" => Some(DebugDialog::DeleteLast),
                            "new-channel" => Some(DebugDialog::NewChannel),
                            "rename" => Some(DebugDialog::Rename),
                            "delete-channel" => Some(DebugDialog::DeleteChannel),
                            other => {
                                tracing::warn!(dialog = other, "FITCOM_DEBUG_VIEW: unknown dialog");
                                None
                            }
                        },
                    }
                }
                "wordle" => d.wordle_typed = Some(value.to_ascii_lowercase()),
                "reply" => d.reply_last = value == "last",
                "edit" => d.edit_last = value == "last",
                "emoji" => d.emoji_last = value == "last",
                "menu" => d.menu = Some(value.to_string()),
                "hotkey" => d.hotkey_wait = flag,
                "focused" => d.focused = Some(flag),
                "scroll" => d.scroll = Some(value.to_string()),
                "hover" => d.hover = Some(value.to_string()),
                other => tracing::warn!(item = other, "FITCOM_DEBUG_VIEW: unknown item"),
            }
        }
        d
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_script_sets_what_it_names() {
        let d = DebugView::parse("tab=audio; members=0;overlay=ac;ac=ri;dialog=lightbox:files/a.png");
        assert_eq!(d.tab, Some(SettingsTab::Audio));
        assert_eq!(d.members, Some(false));
        assert_eq!(d.overlay, Some(Overlay::Autocomplete));
        assert_eq!(d.ac_query.as_deref(), Some("ri"));
        match d.dialog {
            Some(DebugDialog::Ready(Dialog::Lightbox { name, .. })) => assert_eq!(name, "a.png"),
            other => panic!("{other:?}"),
        }
    }
}
