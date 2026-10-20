//! The window's views: `AppView` (the root) and one file per area.
//!
//! # Shape
//!
//! `AppView` owns everything the window draws from: `s` (the last `UiState`), `tl` (the
//! timeline of the open conversation) and `v` (`ViewState`, the `V` object of the old
//! `app.js`, field for field), plus the `Meters` and `Thumbnails` entities. It renders the
//! shell skeleton and the light areas (titlebar, rail, main head, status bar) as plain
//! functions, and the heavy areas as *panes*: entities of their own, defined in the area's
//! file, drawn with `.cached(..)` so a meter tick or a keystroke does not rebuild the
//! timeline. Panes observe `AppView`, so every change to it reaches them.
//!
//! # Rules for the area files
//!
//! - Read state through an [`AppSnapshot`] (`snap.s`, `snap.tl`, `snap.v`, plus the
//!   derived helpers on it). A pane gets one with `self.app.snapshot(cx)`; a function gets
//!   it as its first argument. Cheap: it is a handful of `Arc`/`Rc` clones.
//! - Change state only through `AppView`: `snap.app.listener(|app, ev, window, cx| ..)` for
//!   an event handler, `snap.app.update(cx, |app, cx| ..)` otherwise. Mutate `app.v_mut()`
//!   and call `cx.notify()`, or use one of the navigation methods below.
//! - Talk to the engine only through `commands::*`, with `&snap.ui`.
//! - A timeline refetch is never asked for: it follows `s.timeline_revision` and the open
//!   conversation by itself (`AppView::apply_state`, `after_conversation_change`).
//! - Read speaking level and RTT only from `snap.meters` (see `widgets::meter_view`), never
//!   from `s`, and never observe `Meters` from a pane's root.

pub mod call;
pub mod channels;
pub mod composer;
pub mod dialogs;
pub mod format;
pub mod markup;
pub mod members;
pub mod rail;
pub mod settings;
pub mod status;
pub mod strip;
pub mod timeline;
pub mod titlebar;
pub mod widgets;
pub mod wordle;

use super::bridge::{Feeds, MeterReadings, Meters, ThumbMap, Thumbnails};
use super::commands::{self, SourceOption};
use super::icons::Icon;
use super::state::{
    ChannelState, OpRef, OwnStreamState, PeerState, Presence, Recap, StreamState, TimelineItem,
    UiState,
};
use super::theme;
use super::Ui;
use gpui_kit::{
    deferred, div, prelude::FluentBuilder, relative, AnyWindowHandle, App, AppContext, Context,
    Entity, ExternalPaths, FocusHandle, InteractiveElement, IntoElement, KeyBinding, ParentElement,
    Render, StyleRefinement, Styled, Task, WeakEntity, Window,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

gpui_kit::actions!(fitcom, [Escape, Quit, HideApp]);

/// The key context of the root, where Escape is handled.
const KEY_CONTEXT: &str = "FitApp";

/// Key bindings, and on macOS the application menu Tauri used to provide (Hide, Quit, and
/// their Cmd shortcuts). Called once before the window opens.
pub fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("escape", Escape, Some(KEY_CONTEXT))]);
    composer::init(cx);
    #[cfg(target_os = "macos")]
    {
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-h", HideApp, None),
        ]);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_action(|_: &HideApp, cx| cx.hide());
        cx.set_menus([gpui_kit::Menu {
            name: "FitCommunication".into(),
            items: vec![
                gpui_kit::MenuItem::action("Hide FitCommunication", HideApp),
                gpui_kit::MenuItem::separator(),
                gpui_kit::MenuItem::action("Quit FitCommunication", Quit),
            ],
            disabled: false,
        }]);
    }
}

/// Opens the main window with `AppView` inside gpui-kit's `Root` (which hosts the kit's
/// tooltips, popovers and notifications).
pub fn open_window(
    cx: &mut App,
    options: gpui_kit::WindowOptions,
    ui: Arc<Ui>,
    feeds: Feeds,
) -> anyhow::Result<AnyWindowHandle> {
    let (handle, _) = gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| AppView::new(ui, feeds, window, cx))
    })?;
    Ok(handle)
}

// ------------------------------------------------------------------ view state

/// Which zone the rail points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum View {
    #[default]
    Channels,
    Dms,
    Settings,
}

/// The seven settings tabs, in the order of the old `SET_TABS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsTab {
    #[default]
    Account,
    Audio,
    Video,
    Clips,
    Files,
    Network,
    Recap,
}

impl SettingsTab {
    pub const ALL: [SettingsTab; 7] = [
        SettingsTab::Account,
        SettingsTab::Audio,
        SettingsTab::Video,
        SettingsTab::Clips,
        SettingsTab::Files,
        SettingsTab::Network,
        SettingsTab::Recap,
    ];

    pub fn id(self) -> &'static str {
        match self {
            SettingsTab::Account => "account",
            SettingsTab::Audio => "audio",
            SettingsTab::Video => "video",
            SettingsTab::Clips => "clips",
            SettingsTab::Files => "files",
            SettingsTab::Network => "network",
            SettingsTab::Recap => "recap",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            SettingsTab::Account => "Account",
            SettingsTab::Audio => "Audio",
            SettingsTab::Video => "Video",
            SettingsTab::Clips => "Clips",
            SettingsTab::Files => "Files",
            SettingsTab::Network => "Network",
            SettingsTab::Recap => "Recap",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            SettingsTab::Account => Icon::Users,
            SettingsTab::Audio => Icon::Mic,
            SettingsTab::Video => Icon::Monitor,
            SettingsTab::Clips => Icon::Play,
            SettingsTab::Files => Icon::Image,
            SettingsTab::Network => Icon::Signal,
            SettingsTab::Recap => Icon::Wave,
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.id() == id)
    }
}

/// What sits over the composer. At most one at a time, as before.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Overlay {
    #[default]
    None,
    /// The @-member list.
    Autocomplete,
    /// Files are being dragged over the window.
    Drop,
    /// The `+` menu.
    Plus,
}

/// The chip above the composer while a reply is being written.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplyTo {
    pub op: OpRef,
    pub name: String,
    pub body: String,
}

/// The modal windows. One at a time; `AppView::open_dialog` / `close_dialog`.
#[derive(Debug, Clone, PartialEq)]
pub enum Dialog {
    Confirm {
        title: String,
        text: String,
        ok_label: String,
        danger: bool,
        action: ConfirmAction,
    },
    Prompt {
        title: String,
        text: String,
        value: String,
        ok_label: String,
        action: PromptAction,
    },
    /// Share a screen or window. Cameras are already filtered out (they have their own
    /// button).
    Picker {
        sources: Vec<SourceOption>,
    },
    /// A picture at full size. `src` must pass `Ui::image_path_allowed` before it is drawn.
    Lightbox {
        src: PathBuf,
        name: String,
        op: Option<OpRef>,
        actual_size: bool,
    },
    Wordle,
}

/// What "Confirm" does. Every irreversible action of the old `askConfirm` call sites.
#[derive(Debug, Clone, PartialEq)]
pub enum ConfirmAction {
    ApplyUpdate,
    DeleteAllImages,
    DeleteMessage(OpRef),
}

impl ConfirmAction {
    pub fn run(&self, ui: &Ui) {
        match self {
            ConfirmAction::ApplyUpdate => commands::apply_update(ui),
            ConfirmAction::DeleteAllImages => commands::delete_all_images(ui),
            ConfirmAction::DeleteMessage(op) => commands::delete_message(ui, op),
        }
    }
}

/// What "Save" in the text prompt does.
#[derive(Debug, Clone, PartialEq)]
pub enum PromptAction {
    NewChannel,
    RenameChannel(String),
}

impl PromptAction {
    /// An empty value does nothing, as `submitPrompt` did.
    pub fn run(&self, ui: &Ui, value: &str) {
        let value = value.trim();
        if value.is_empty() {
            return;
        }
        match self {
            PromptAction::NewChannel => commands::create_channel(ui, value),
            PromptAction::RenameChannel(key) => commands::rename_channel(ui, key, value),
        }
    }
}

impl Dialog {
    /// The texts of the old `askConfirm` / `askText` call sites live here, once, so the
    /// area that opens a dialog does not need to know the dialog's words.
    pub fn delete_message(op: OpRef) -> Self {
        Dialog::Confirm {
            title: "Delete this?".into(),
            text: "It disappears from the timeline for everyone. A file also stops being served \u{2014} but a download that already finished stays on that machine.".into(),
            ok_label: "Delete".into(),
            danger: true,
            action: ConfirmAction::DeleteMessage(op),
        }
    }

    pub fn apply_update(version: &str) -> Self {
        Dialog::Confirm {
            title: format!("Update to {version} and restart?"),
            text: "The new version is already downloaded and verified. Applying it closes the app, replaces it, and starts it again. Your history and settings are untouched.".into(),
            ok_label: "Update and restart".into(),
            danger: false,
            action: ConfirmAction::ApplyUpdate,
        }
    }

    pub fn delete_all_images() -> Self {
        Dialog::Confirm {
            title: "Delete all images?".into(),
            text: "This clears the image folder on this machine only. The others keep their copies, and anything still offered in the chat can be downloaded again.".into(),
            ok_label: "Delete them".into(),
            danger: true,
            action: ConfirmAction::DeleteAllImages,
        }
    }

    pub fn new_channel() -> Self {
        Dialog::Prompt {
            title: "New channel".into(),
            text: "A sub-channel under the general channel. Everyone sees it, and it keeps its own message stream and its own unread count.".into(),
            value: String::new(),
            ok_label: "Create".into(),
            action: PromptAction::NewChannel,
        }
    }

    pub fn rename_channel(channel: &ChannelState) -> Self {
        Dialog::Prompt {
            title: format!("Rename {}", channel.name),
            text: "Everyone sees the new name. Leave it empty and cancel to remove the channel instead.".into(),
            value: channel.name.clone(),
            ok_label: "Save".into(),
            action: PromptAction::RenameChannel(channel.key.clone()),
        }
    }
}

/// What this window is looking at: the `V` object of `app.js`, field for field, plus the
/// module-level `let`s that were view state in disguise. Purely local; none of it outlives
/// the process.
#[derive(Debug, Clone)]
pub struct ViewState {
    pub view: View,
    /// The last non-DM channel.
    pub channel: String,
    /// The last opened DM peer id.
    pub dm: Option<String>,
    pub members: bool,
    pub settings_tab: SettingsTab,
    pub overlay: Overlay,
    /// The message being edited in place.
    pub editing: Option<OpRef>,
    pub reply_to: Option<ReplyTo>,
    /// The message whose quick-emoji bar is open.
    pub emoji_for: Option<OpRef>,
    /// Last moment we told the engine we were typing; one notification per 2.5 s window.
    pub last_typing_sent: Option<Instant>,
    pub ac_index: usize,
    /// Peer ids matching the @-query, in roster order.
    pub ac_matches: Vec<String>,
    pub ac_query: String,
    /// Half-typed messages of the conversations that are *not* open, per channel key. The
    /// open one's text lives in the composer itself.
    pub drafts: HashMap<String, String>,
    /// Muting a volume is setting it to 0 with the old level remembered here, because that
    /// is all the engine has: there is no separate per-peer mute on the wire.
    pub volume_before_mute: HashMap<members::VolumeKey, f32>,
    /// The peer whose "Audio for" popover is open: the one home of their volumes.
    pub audio_for: Option<String>,
    /// The call panel's share menu is open. Only while a screen is shared.
    pub share_menu: bool,
    pub focused: bool,
    /// Unread count of the conversation at the moment it was opened, for the "new" line.
    pub unread_at_open: usize,
    /// The general channel group is collapsed.
    pub collapsed: bool,
    /// The clip hotkey box is listening for a combination.
    pub hotkey_wait: bool,
    pub dialog: Option<Dialog>,
    /// Letters typed into the Wordle row that is not sent yet.
    pub wordle_typed: String,
    /// The number of landed rows when `wordle_typed` was last cleared; a change clears it.
    pub wordle_rows: Option<usize>,
    /// Loaded on demand by `AppView::set_settings_tab`; `None` until then.
    pub devices: Option<(Vec<String>, Vec<String>)>,
    pub clip_monitors: Option<Vec<String>>,
    pub recap: Option<Recap>,
}

impl Default for ViewState {
    fn default() -> Self {
        ViewState {
            view: View::Channels,
            channel: "general".into(),
            dm: None,
            members: true,
            settings_tab: SettingsTab::Account,
            overlay: Overlay::None,
            editing: None,
            reply_to: None,
            emoji_for: None,
            last_typing_sent: None,
            ac_index: 0,
            ac_matches: Vec::new(),
            ac_query: String::new(),
            drafts: HashMap::new(),
            volume_before_mute: HashMap::new(),
            audio_for: None,
            share_menu: false,
            focused: true,
            unread_at_open: 0,
            collapsed: false,
            hotkey_wait: false,
            dialog: None,
            wordle_typed: String::new(),
            wordle_rows: None,
            devices: None,
            clip_monitors: None,
            recap: None,
        }
    }
}

// ------------------------------------------------------------------ snapshot + handle

/// A weak handle to the root, for event handlers in the area files.
#[derive(Clone)]
pub struct AppHandle(WeakEntity<AppView>);

impl AppHandle {
    /// The state to draw from. `None` only while the window is going away.
    pub fn snapshot(&self, cx: &App) -> Option<AppSnapshot> {
        self.0.upgrade().map(|app| app.read(cx).snapshot())
    }

    /// Run `f` on the root. Call `cx.notify()` inside when it changed something.
    pub fn update<R>(
        &self,
        cx: &mut App,
        f: impl FnOnce(&mut AppView, &mut Context<AppView>) -> R,
    ) -> Option<R> {
        self.0.update(cx, f).ok()
    }

    /// An event handler that runs on the root, like `cx.listener` does for the current
    /// entity: `.on_click(snap.app.listener(|app, _, window, cx| app.open_channel(key, window, cx)))`.
    pub fn listener<E: ?Sized + 'static>(
        &self,
        f: impl Fn(&mut AppView, &E, &mut Window, &mut Context<AppView>) + 'static,
    ) -> impl Fn(&E, &mut Window, &mut App) + 'static {
        let weak = self.0.clone();
        move |event, window, cx| {
            let _ = weak.update(cx, |app, cx| f(app, event, window, cx));
        }
    }
}

/// Everything an area draws from, cloned out of `AppView` for one render.
#[derive(Clone)]
pub struct AppSnapshot {
    pub ui: Arc<Ui>,
    pub s: Arc<UiState>,
    pub tl: Arc<Vec<TimelineItem>>,
    pub v: Rc<ViewState>,
    pub meters: Entity<Meters>,
    pub thumbs: Entity<Thumbnails>,
    pub app: AppHandle,
    /// Files are being dragged over the window right now (draw the drop overlay).
    pub drop_active: bool,
    /// The focus handle the dialog layer tracks; `open_dialog` focuses it so typing goes to
    /// the dialog and not to the composer behind it.
    pub dialog_focus: FocusHandle,
}

/// The derived helpers of `app.js`, one for one.
impl AppSnapshot {
    /// `activeChannel()`: the key of the conversation on screen.
    pub fn active_channel(&self) -> String {
        active_channel(&self.v)
    }
    pub fn peer(&self, id: &str) -> Option<&PeerState> {
        self.s.peers.iter().find(|p| p.id == id)
    }
    pub fn channel(&self, key: &str) -> Option<&ChannelState> {
        self.s.channels.iter().find(|c| c.key == key)
    }
    /// Peers whose identity we know (`knownPeers`).
    pub fn known_peers(&self) -> impl Iterator<Item = &PeerState> {
        self.s.peers.iter().filter(|p| !p.id.is_empty())
    }
    pub fn online_peers(&self) -> impl Iterator<Item = &PeerState> {
        self.known_peers()
            .filter(|p| p.presence == Presence::Online)
    }
    pub fn call_peers(&self) -> impl Iterator<Item = &PeerState> {
        self.known_peers().filter(|p| p.in_call)
    }
    pub fn call_running(&self) -> bool {
        self.call_peers().next().is_some() || self.s.voice.joined
    }
    pub fn total_dm_unread(&self) -> usize {
        self.known_peers().map(|p| p.unread).sum()
    }
    /// A screen or window of ours is shared (not the camera, not desktop audio).
    pub fn sharing_self(&self) -> bool {
        self.s
            .own_streams
            .iter()
            .any(|s| !s.is_audio && !s.is_camera)
    }
    pub fn own_camera(&self) -> Option<&OwnStreamState> {
        self.s.own_streams.iter().find(|s| s.is_camera)
    }
    pub fn own_music(&self) -> Option<&OwnStreamState> {
        self.s.own_streams.iter().find(|s| s.is_music)
    }
    pub fn watched_streams(&self) -> impl Iterator<Item = &StreamState> {
        self.s.streams.iter().filter(|s| s.watching)
    }
    /// `activeName()`: the peer's name in a DM, the channel's name otherwise.
    pub fn active_name(&self) -> String {
        if self.v.view == View::Dms {
            if let Some(dm) = &self.v.dm {
                return self.peer(dm).map(|p| p.name.clone()).unwrap_or_default();
            }
        }
        self.channel(&self.v.channel)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| "general".into())
    }
    /// `unreadCount()`: unread messages in the conversation on screen.
    pub fn unread_count(&self) -> usize {
        unread_count(&self.s, &self.v)
    }
    /// Names of the peers typing in the open conversation.
    pub fn typing_names(&self) -> Vec<String> {
        self.s
            .typing
            .get(&self.active_channel())
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.peer(id).map(|p| p.name.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }
}

fn active_channel(v: &ViewState) -> String {
    match (&v.view, &v.dm) {
        (View::Dms, Some(dm)) => format!("dm:{dm}"),
        _ => v.channel.clone(),
    }
}

/// Whether the open conversation is really on screen: the window in front and not on a
/// settings tab. In settings the last channel is still "active" but not visible, and
/// marking it read there swallowed messages and their notifications.
fn looking(v: &ViewState) -> bool {
    v.focused && v.view != View::Settings
}

fn unread_count(s: &UiState, v: &ViewState) -> usize {
    let key = active_channel(v);
    match key.strip_prefix("dm:") {
        Some(id) => s.peers.iter().find(|p| p.id == id).map_or(0, |p| p.unread),
        None => s
            .channels
            .iter()
            .find(|c| c.key == key)
            .map_or(0, |c| c.unread),
    }
}

/// What a pane constructor gets. Panes are built while `AppView` itself is being built,
/// so they cannot read it yet; everything they may need at construction is in here.
#[derive(Clone)]
pub struct PaneInit {
    pub app: AppHandle,
    pub ui: Arc<Ui>,
    pub meters: Entity<Meters>,
    pub thumbs: Entity<Thumbnails>,
}

struct Panes {
    channels: Entity<channels::ChannelsPane>,
    members: Entity<members::MembersPane>,
    strip: Entity<strip::StripPane>,
    timeline: Entity<timeline::TimelinePane>,
    composer: Entity<composer::ComposerPane>,
    settings: Entity<settings::SettingsPane>,
}

// ------------------------------------------------------------------ the root

pub struct AppView {
    ui: Arc<Ui>,
    s: Arc<UiState>,
    tl: Arc<Vec<TimelineItem>>,
    v: Rc<ViewState>,
    meters: Entity<Meters>,
    thumbs: Entity<Thumbnails>,
    handle: AppHandle,
    panes: Panes,
    focus: FocusHandle,
    dialog_focus: FocusHandle,
    /// The conversation whose text is in the composer right now.
    composer_key: String,
    /// The width side of the narrow rule last applied (`applyWidth`).
    was_narrow: Option<bool>,
    drop_hover: bool,
    _tasks: Vec<Task<()>>,
    _subscriptions: Vec<gpui_kit::Subscription>,
}

impl AppView {
    fn new(ui: Arc<Ui>, feeds: Feeds, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let me = cx.entity();
        let handle = AppHandle(me.downgrade());
        let meters = cx.new(|_| Meters::default());
        let thumbs = cx.new(|_| Thumbnails::default());
        let init = PaneInit {
            app: handle.clone(),
            ui: ui.clone(),
            meters: meters.clone(),
            thumbs: thumbs.clone(),
        };

        let panes = Panes {
            channels: observing_pane(&me, cx, |cx| channels::ChannelsPane::new(&init, window, cx)),
            members: observing_pane(&me, cx, |cx| members::MembersPane::new(&init, window, cx)),
            strip: observing_pane(&me, cx, |cx| strip::StripPane::new(&init, window, cx)),
            timeline: observing_pane(&me, cx, |cx| timeline::TimelinePane::new(&init, window, cx)),
            composer: observing_pane(&me, cx, |cx| composer::ComposerPane::new(&init, window, cx)),
            settings: observing_pane(&me, cx, |cx| settings::SettingsPane::new(&init, window, cx)),
        };

        let Feeds {
            state: mut state_rx,
            meters: mut meters_rx,
            thumbs: mut thumbs_rx,
        } = feeds;
        let s = state_rx.borrow_and_update().clone();
        let first_meters: MeterReadings = meters_rx.borrow_and_update().clone();
        let first_thumbs: Arc<ThumbMap> = thumbs_rx.borrow_and_update().clone();
        meters.update(cx, |m, cx| m.set(first_meters, cx));
        thumbs.update(cx, |t, cx| t.set(first_thumbs, window, cx));

        let mut tasks = Vec::new();
        tasks.push(cx.spawn_in(window, async move |this, cx| {
            while state_rx.changed().await.is_ok() {
                let next = state_rx.borrow_and_update().clone();
                if this
                    .update_in(cx, |this, window, cx| this.apply_state(next, window, cx))
                    .is_err()
                {
                    return;
                }
            }
        }));
        tasks.push(cx.spawn_in(window, async move |this, cx| {
            while meters_rx.changed().await.is_ok() {
                let next = meters_rx.borrow_and_update().clone();
                if this
                    .update(cx, |this, cx| {
                        this.meters.update(cx, |m, cx| m.set(next, cx))
                    })
                    .is_err()
                {
                    return;
                }
            }
        }));
        tasks.push(cx.spawn_in(window, async move |this, cx| {
            while thumbs_rx.changed().await.is_ok() {
                let next = thumbs_rx.borrow_and_update().clone();
                let applied = this.update_in(cx, |this, window, cx| {
                    this.thumbs.update(cx, |t, cx| t.set(next, window, cx))
                });
                if applied.is_err() {
                    return;
                }
            }
        }));

        let subscriptions = vec![
            // Looking at a conversation is reading it, and the engine sends a desktop
            // notification only for a window that is not in front.
            cx.observe_window_activation(window, |this, window, cx| {
                let active = window.is_window_active();
                this.ui
                    .engine
                    .voorgrond
                    .store(active, std::sync::atomic::Ordering::Relaxed);
                this.v_mut().focused = active;
                if looking(&this.v) && unread_count(&this.s, &this.v) > 0 {
                    commands::mark_read(&this.ui, &active_channel(&this.v));
                }
                cx.notify();
            }),
            cx.observe_window_bounds(window, |this, window, cx| {
                if this.apply_width(window) {
                    cx.notify();
                }
            }),
        ];

        let focus = cx.focus_handle();
        let mut this = AppView {
            ui,
            s,
            tl: Arc::new(Vec::new()),
            v: Rc::new(ViewState::default()),
            meters,
            thumbs,
            handle,
            panes,
            focus: focus.clone(),
            dialog_focus: cx.focus_handle(),
            composer_key: "general".into(),
            was_narrow: None,
            drop_hover: false,
            _tasks: tasks,
            _subscriptions: subscriptions,
        };
        this.apply_width(window);
        // The window can open without becoming active (autostart at login, a launch from the
        // background), and the observer above only hears changes: start from the real state,
        // or a window nobody looks at swallows unread counts and desktop notifications.
        let active = window.is_window_active();
        this.ui
            .engine
            .voorgrond
            .store(active, std::sync::atomic::Ordering::Relaxed);
        this.v_mut().focused = active;
        this.load_timeline();
        // The boot sequence of `app.js`: draw, then mark the open conversation read, but only
        // when someone is looking: an autostarted window that opens behind the game must not
        // swallow what came in overnight.
        if looking(&this.v) {
            commands::mark_read(&this.ui, &active_channel(&this.v));
        }
        window.focus(&focus, cx);

        #[cfg(debug_assertions)]
        if let Some(script) = super::fixture::DebugView::from_env() {
            this.apply_debug_view(script, window, cx);
        }
        this
    }

    /// Cheap clones of everything an area draws from.
    pub fn snapshot(&self) -> AppSnapshot {
        AppSnapshot {
            ui: self.ui.clone(),
            s: self.s.clone(),
            tl: self.tl.clone(),
            v: self.v.clone(),
            meters: self.meters.clone(),
            thumbs: self.thumbs.clone(),
            app: self.handle.clone(),
            drop_active: self.drop_hover || self.v.overlay == Overlay::Drop,
            dialog_focus: self.dialog_focus.clone(),
        }
    }

    pub fn ui(&self) -> &Arc<Ui> {
        &self.ui
    }

    pub fn state(&self) -> &UiState {
        &self.s
    }

    pub fn view_state(&self) -> &ViewState {
        &self.v
    }

    /// The view state, for changing it. Call `cx.notify()` afterwards.
    pub fn v_mut(&mut self) -> &mut ViewState {
        Rc::make_mut(&mut self.v)
    }

    /// `applyState`: a new state from the engine.
    fn apply_state(&mut self, next: Arc<UiState>, window: &mut Window, cx: &mut Context<Self>) {
        let previous = std::mem::replace(&mut self.s, next);
        let before = active_channel(&self.v);

        // A conversation that disappeared (a sub-channel someone removed) must not leave
        // the window pointing at nothing.
        let channel_gone = self.v.view == View::Channels
            && !self.s.channels.iter().any(|c| c.key == self.v.channel);
        if channel_gone {
            self.v_mut().channel = "general".into();
        }
        let dm_gone = self.v.view == View::Dms
            && self
                .v
                .dm
                .as_ref()
                .is_some_and(|dm| !self.s.peers.iter().any(|p| &p.id == dm));
        if dm_gone {
            let first = self
                .s
                .peers
                .iter()
                .find(|p| !p.id.is_empty())
                .map(|p| p.id.clone());
            self.v_mut().dm = first;
        }

        // A share that ended elsewhere (window closed, call left) takes its menu along, so
        // the next share does not open with it.
        if self.v.share_menu && !(self.s.voice.joined && self.snapshot().sharing_self()) {
            self.v_mut().share_menu = false;
        }

        if active_channel(&self.v) != before {
            self.after_conversation_change(window, cx);
        } else if previous.timeline_revision != self.s.timeline_revision {
            self.load_timeline();
        }
        cx.notify();

        // Looking at a conversation is reading it, but only the one actually on screen,
        // so a DM cannot quietly clear the general channel's counter or the other way round.
        if looking(&self.v) && unread_count(&self.s, &self.v) > 0 {
            commands::mark_read(&self.ui, &active_channel(&self.v));
        }
    }

    fn load_timeline(&mut self) {
        self.tl = Arc::new(commands::get_timeline(&self.ui, &active_channel(&self.v)));
    }

    /// `afterConversationChange`: the composer swaps to the new conversation's draft, the
    /// unread mark is taken, the conversation is marked read and its timeline loaded.
    fn after_conversation_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = active_channel(&self.v);
        if key != self.composer_key {
            // Here and not only in `open_channel`/`open_dm`: the rail buttons and a vanished
            // channel come through here too. A reply that moved along to another
            // conversation made Enter drop the text with an error banner.
            self.leave_conversation_state();
            let draft = self.v_mut().drafts.remove(&key).unwrap_or_default();
            let left = self
                .panes
                .composer
                .update(cx, |c, cx| c.swap_draft(&draft, window, cx));
            let old = std::mem::replace(&mut self.composer_key, key.clone());
            if !left.trim().is_empty() {
                self.v_mut().drafts.insert(old, left);
            }
        }
        let unread = unread_count(&self.s, &self.v);
        self.v_mut().unread_at_open = unread;
        commands::mark_read(&self.ui, &key);
        self.load_timeline();
        cx.notify();
    }

    fn leave_conversation_state(&mut self) {
        let v = self.v_mut();
        v.editing = None;
        v.reply_to = None;
        v.emoji_for = None;
        v.overlay = Overlay::None;
    }

    /// `openChannel(key)`.
    pub fn open_channel(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.leave_conversation_state();
        let v = self.v_mut();
        v.view = View::Channels;
        v.channel = key.to_string();
        self.after_conversation_change(window, cx);
    }

    /// `openDm(id)`.
    pub fn open_dm(&mut self, peer: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.leave_conversation_state();
        let v = self.v_mut();
        v.view = View::Dms;
        v.dm = Some(peer.to_string());
        self.after_conversation_change(window, cx);
    }

    /// The rail's channels button.
    pub fn show_channels(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.v_mut().view = View::Channels;
        self.after_conversation_change(window, cx);
    }

    /// The rail's DM button: the last DM, or the first known peer.
    pub fn show_dms(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let first = self
            .s
            .peers
            .iter()
            .find(|p| !p.id.is_empty())
            .map(|p| p.id.clone());
        let v = self.v_mut();
        v.view = View::Dms;
        if v.dm.is_none() {
            v.dm = first;
        }
        self.after_conversation_change(window, cx);
    }

    /// The rail's settings button. The composer keeps its text; it is only hidden.
    pub fn show_settings(&mut self, cx: &mut Context<Self>) {
        self.v_mut().view = View::Settings;
        self.load_for_tab(self.v.settings_tab);
        cx.notify();
    }

    /// A settings tab, with what it needs loaded the way the old tab click did: devices
    /// and monitors once, the recap every time (its figures move while you look).
    pub fn set_settings_tab(&mut self, tab: SettingsTab, cx: &mut Context<Self>) {
        self.v_mut().settings_tab = tab;
        self.load_for_tab(tab);
        cx.notify();
    }

    fn load_for_tab(&mut self, tab: SettingsTab) {
        match tab {
            SettingsTab::Audio if self.v.devices.is_none() => self.refresh_devices(),
            SettingsTab::Clips if self.v.clip_monitors.is_none() && self.s.clips.is_some() => {
                let monitors = commands::clip_monitors(&self.ui);
                self.v_mut().clip_monitors = Some(monitors);
            }
            SettingsTab::Recap => self.refresh_recap(),
            _ => {}
        }
    }

    /// The audio tab's Refresh: a headset that was just plugged in appears.
    pub fn refresh_devices(&mut self) {
        let devices = commands::list_audio_devices(&self.ui);
        self.v_mut().devices = Some(devices);
    }

    pub fn refresh_recap(&mut self) {
        let recap = commands::get_recap(&self.ui);
        self.v_mut().recap = Some(recap);
    }

    /// Opens a peer's audio popover. It hangs off their member row, so the member list
    /// opens with it (a speaker button on a stream tile can ask while the list is closed).
    pub fn open_audio_for(&mut self, peer: &str, cx: &mut Context<Self>) {
        let v = self.v_mut();
        v.audio_for = Some(peer.to_string());
        v.members = true;
        cx.notify();
    }

    pub fn toggle_members(&mut self, cx: &mut Context<Self>) {
        let v = self.v_mut();
        v.members = !v.members;
        cx.notify();
    }

    /// `openPicker`: the source list is fetched now and never cached.
    pub fn open_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let sources = commands::list_sources(&self.ui)
            .into_iter()
            .filter(|s| !s.is_camera)
            .collect();
        self.open_dialog(Dialog::Picker { sources }, window, cx);
    }

    pub fn open_dialog(&mut self, dialog: Dialog, window: &mut Window, cx: &mut Context<Self>) {
        let v = self.v_mut();
        v.overlay = Overlay::None;
        v.dialog = Some(dialog);
        window.focus(&self.dialog_focus, cx);
        cx.notify();
    }

    pub fn close_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.v_mut().dialog = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Below 1080 the member list is in the way, so it closes, and as state, so the toggle
    /// keeps telling the truth and the user can open it again. Only on crossing the line,
    /// like the old `matchMedia` listener. Returns whether it changed anything.
    fn apply_width(&mut self, window: &Window) -> bool {
        let narrow = window.viewport_size().width <= theme::NARROW_MEMBERS;
        if self.was_narrow == Some(narrow) {
            return false;
        }
        self.was_narrow = Some(narrow);
        self.v_mut().members = !narrow;
        true
    }

    /// Escape closes the topmost thing, in the order of the old keydown handler.
    fn on_escape(&mut self, _: &Escape, window: &mut Window, cx: &mut Context<Self>) {
        if self.v.dialog.is_some() {
            return self.close_dialog(window, cx);
        }
        let v = self.v_mut();
        if v.hotkey_wait {
            v.hotkey_wait = false;
        } else if v.editing.is_some() {
            v.editing = None;
        } else if v.emoji_for.is_some() {
            v.emoji_for = None;
        } else if v.reply_to.is_some() {
            v.reply_to = None;
        } else if v.overlay != Overlay::None {
            v.overlay = Overlay::None;
        } else {
            cx.propagate();
            return;
        }
        cx.notify();
    }

    fn on_drag_files(&mut self, cx: &mut Context<Self>) {
        // Settings shows no conversation to share into.
        if self.v.view == View::Settings {
            return;
        }
        if !self.drop_hover {
            self.drop_hover = true;
            self.v_mut().overlay = Overlay::Drop;
            cx.notify();
        }
    }

    /// B-52: the paths stay in `Ui::dropped`; `offer_files` gets indices into them.
    fn on_drop_files(&mut self, paths: &ExternalPaths, _: &mut Window, cx: &mut Context<Self>) {
        // See `on_drag_files`: without this a file let go over settings went, with no
        // overlay, to everyone in the last opened channel.
        if self.v.view == View::Settings {
            return;
        }
        self.drop_hover = false;
        if self.v.overlay == Overlay::Drop {
            self.v_mut().overlay = Overlay::None;
        }
        let indices: Vec<usize> = {
            let mut kept = self.ui.dropped.lock().unwrap_or_else(|e| e.into_inner());
            *kept = paths.paths().to_vec();
            (0..kept.len()).collect()
        };
        commands::offer_files(&self.ui, &indices, &active_channel(&self.v));
        cx.notify();
    }

    #[cfg(debug_assertions)]
    fn apply_debug_view(
        &mut self,
        d: super::fixture::DebugView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use super::fixture::DebugDialog;
        if let Some(view) = d.view {
            match view {
                View::Channels => self.show_channels(window, cx),
                View::Dms => self.show_dms(window, cx),
                View::Settings => self.show_settings(cx),
            }
        }
        if let Some(channel) = &d.channel {
            self.open_channel(channel, window, cx);
        }
        if let Some(dm) = &d.dm {
            let id = if dm == "first" {
                self.s
                    .peers
                    .iter()
                    .find(|p| !p.id.is_empty())
                    .map(|p| p.id.clone())
            } else {
                Some(dm.clone())
            };
            if let Some(id) = id {
                self.open_dm(&id, window, cx);
            }
        }
        if let Some(tab) = d.tab {
            self.show_settings(cx);
            self.set_settings_tab(tab, cx);
        }
        let last_message = self.tl.iter().rev().find_map(|i| match i {
            TimelineItem::Message {
                id,
                author_name,
                body,
                mine,
                ..
            } => Some((id.clone(), author_name.clone(), body.clone(), *mine)),
            _ => None,
        });
        let last_own = self.tl.iter().rev().find_map(|i| match i {
            TimelineItem::Message { id, mine: true, .. } => Some(id.clone()),
            _ => None,
        });
        let known: Vec<(String, String)> = self
            .s
            .peers
            .iter()
            .filter(|p| !p.id.is_empty())
            .map(|p| (p.id.clone(), p.name.clone()))
            .collect();
        let channel_of = |s: &UiState, key: &str| s.channels.iter().find(|c| c.key == key).cloned();
        let removable = self.s.channels.iter().find(|c| c.removable).cloned();
        let update_version = match &self.s.update {
            Some(super::state::UpdateState::Ready { version }) => version.clone(),
            _ => "1.0.0".into(),
        };

        {
            let v = self.v_mut();
            if let Some(m) = d.members {
                v.members = m;
            }
            v.collapsed |= d.collapsed;
            if let Some(o) = d.overlay {
                v.overlay = o;
            }
            if let Some(q) = &d.ac_query {
                v.ac_query = q.clone();
                let ql = q.to_lowercase();
                v.ac_matches = known
                    .iter()
                    .filter(|(_, name)| name.to_lowercase().starts_with(&ql))
                    .map(|(id, _)| id.clone())
                    .collect();
                v.ac_index = 0;
                v.overlay = Overlay::Autocomplete;
            }
            if let Some(w) = &d.wordle_typed {
                v.wordle_typed = w
                    .chars()
                    .filter(|c| c.is_ascii_lowercase())
                    .take(5)
                    .collect();
            }
            if let (true, Some((op, name, body, _))) = (d.reply_last, &last_message) {
                v.reply_to = Some(ReplyTo {
                    op: op.clone(),
                    name: name.clone(),
                    body: body.chars().take(90).collect(),
                });
            }
            if d.edit_last {
                v.editing = last_own.clone();
            }
            if let (true, Some((op, ..))) = (d.emoji_last, &last_message) {
                v.emoji_for = Some(op.clone());
            }
            v.hotkey_wait |= d.hotkey_wait;
            if let Some(f) = d.focused {
                v.focused = f;
            }
        }

        let dialog = match d.dialog {
            Some(DebugDialog::Ready(Dialog::Lightbox {
                src,
                name,
                op,
                actual_size,
            })) => {
                let src = if src.is_relative() {
                    self.ui
                        .fixture
                        .as_ref()
                        .map(|f| f.dir.join(&src))
                        .unwrap_or(src)
                } else {
                    src
                };
                Some(Dialog::Lightbox {
                    src,
                    name,
                    op,
                    actual_size,
                })
            }
            Some(DebugDialog::Ready(dialog)) => Some(dialog),
            Some(DebugDialog::Update) => Some(Dialog::apply_update(&update_version)),
            Some(DebugDialog::Wipe) => Some(Dialog::delete_all_images()),
            Some(DebugDialog::DeleteLast) => {
                last_message.map(|(op, ..)| Dialog::delete_message(op))
            }
            Some(DebugDialog::NewChannel) => Some(Dialog::new_channel()),
            Some(DebugDialog::Rename) => removable.as_ref().map(Dialog::rename_channel),
            Some(DebugDialog::Picker) => {
                self.open_picker(window, cx);
                None
            }
            None => None,
        };
        if let Some(dialog) = dialog {
            self.open_dialog(dialog, window, cx);
        }
        if d.share_menu {
            self.v_mut().share_menu = true;
        }
        if let Some(peer) = &d.audio_for {
            let first = self
                .s
                .peers
                .iter()
                .find(|p| p.in_call)
                .map(|p| p.id.clone());
            if let Some(id) = if peer == "first" {
                first
            } else {
                Some(peer.clone())
            } {
                self.open_audio_for(&id, cx);
            }
        }
        if let Some(text) = &d.composer_text {
            self.panes
                .composer
                .update(cx, |c, cx| c.debug_type(text, window, cx));
            let s = self.s.clone();
            composer::update_autocomplete(self.v_mut(), &s, text);
        }
        let _ = channel_of;
        cx.notify();
    }
}

/// A pane: an entity of its own that re-renders whenever `root` notifies. Built while the
/// root itself is still being constructed (`root` is the reserved handle), which GPUI
/// allows; the test below pins that the observation still fires afterwards.
fn observing_pane<R: 'static, P: 'static>(
    root: &Entity<R>,
    cx: &mut Context<R>,
    build: impl FnOnce(&mut Context<P>) -> P,
) -> Entity<P> {
    let root = root.clone();
    cx.new(|cx| {
        cx.observe(&root, |_, _, cx| cx.notify()).detach();
        build(cx)
    })
}

/// A pane drawn as a cached view: it re-renders only when it (or something it observes)
/// notified, never because a sibling did.
fn cached<V: Render>(pane: &Entity<V>) -> impl IntoElement {
    pane.clone().cached(StyleRefinement::default().size_full())
}

impl Render for AppView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // An external drag that left the window, or was cancelled, ends without a drop.
        if self.drop_hover && !cx.has_active_drag() {
            self.drop_hover = false;
            if self.v.overlay == Overlay::Drop {
                self.v_mut().overlay = Overlay::None;
            }
        }

        let snap = self.snapshot();
        let width = window.viewport_size().width;
        let chan_w = if width <= theme::NARROW_CHANNELS {
            theme::CHAN_W_NARROW
        } else {
            theme::CHAN_W
        };
        let settings = snap.v.view == View::Settings;

        let main = div()
            .id("main")
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .bg(theme::BG_CANVAS)
            .child(timeline::render_head(&snap, window, cx))
            .child(self.panes.strip.clone())
            .child(div().flex_1().min_h_0().child(cached(&self.panes.timeline)))
            .child(self.panes.composer.clone())
            .when(snap.drop_active, |d| {
                d.child(composer::render_drop_overlay(&snap, window, cx))
            });

        let body = div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_row()
            .child(rail::render(&snap, window, cx))
            .when(!settings, |d| {
                d.child(
                    div()
                        .w(chan_w)
                        .flex_none()
                        .h_full()
                        .child(cached(&self.panes.channels)),
                )
                .child(main)
                .when(snap.v.members, |d| {
                    d.child(
                        div()
                            .w(theme::MEMBER_W)
                            .flex_none()
                            .h_full()
                            .child(cached(&self.panes.members)),
                    )
                })
            })
            .when(settings, |d| {
                d.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .child(cached(&self.panes.settings)),
                )
            });

        let dialog = snap.v.dialog.clone();
        div()
            .id("fitcom")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_escape))
            .on_drag_move::<ExternalPaths>(cx.listener(|this, _, _, cx| this.on_drag_files(cx)))
            .on_drop(cx.listener(Self::on_drop_files))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::BG_CANVAS)
            .text_color(theme::TEXT)
            .font_family(theme::UI_FONT)
            .text_size(theme::BASE_TEXT)
            .line_height(relative(theme::BASE_LINE_HEIGHT))
            .child(titlebar::render(&snap, window, cx))
            .child(body)
            .child(status::render(&snap, window, cx))
            .when_some(dialog, |d, dialog| {
                d.child(
                    deferred(dialogs::render_dialog(&snap, &dialog, window, cx)).with_priority(2),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::TestAppContext;
    use std::cell::Cell;

    struct Root {
        pane: Entity<Pane>,
        value: usize,
    }

    struct Pane {
        root: WeakEntity<Root>,
        renders: Rc<Cell<usize>>,
        seen: Rc<Cell<usize>>,
    }

    impl Render for Pane {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            self.renders.set(self.renders.get() + 1);
            self.seen
                .set(self.root.upgrade().map_or(0, |r| r.read(cx).value));
            div().size_full()
        }
    }

    impl Render for Root {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(cached(&self.pane))
        }
    }

    /// The pane model rests on this: a cached pane, observing a root that did not exist
    /// yet when the observation was made, redraws with the root's new data once the root
    /// notifies. Dropping the `observe` in `observing_pane` makes it fail.
    #[gpui_kit::test]
    fn a_cached_pane_redraws_when_its_root_changes(cx: &mut TestAppContext) {
        let renders = Rc::new(Cell::new(0));
        let seen = Rc::new(Cell::new(0));
        let (r, s) = (renders.clone(), seen.clone());
        let window = cx.add_window(move |_, cx| {
            let me = cx.entity();
            let pane = observing_pane(&me, cx, |_| Pane {
                root: me.downgrade(),
                renders: r,
                seen: s,
            });
            Root { pane, value: 0 }
        });
        let draw = |cx: &mut TestAppContext| {
            cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))
                .unwrap();
        };
        draw(cx);
        let before = renders.get();

        window
            .update(cx, |root, _, cx| {
                root.value = 7;
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        draw(cx);

        assert!(renders.get() > before, "the cached pane did not redraw");
        assert_eq!(seen.get(), 7);
    }

    #[test]
    fn the_active_channel_follows_the_rail() {
        let mut v = ViewState::default();
        assert_eq!(active_channel(&v), "general");
        v.view = View::Dms;
        assert_eq!(
            active_channel(&v),
            "general",
            "a DM view without a peer keeps the channel"
        );
        v.dm = Some("abc".into());
        assert_eq!(active_channel(&v), "dm:abc");
        v.view = View::Settings;
        assert_eq!(active_channel(&v), "general");
    }
}
