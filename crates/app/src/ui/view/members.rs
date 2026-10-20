//! The member list (`renderMembers`, `memberRow`): the groups, one line per person (avatar,
//! name, one mono sub-line whose speaking part follows `widgets::meter_view`), and the
//! per-person "Audio for" popover a click on a peer opens. That popover is the one home of
//! every volume of somebody else: voice, screen audio and Spotify, each with its mute (mute
//! is volume 0 with the old level remembered in `v.volume_before_mute`).

use super::super::commands;
use super::super::icons::Icon;
use super::super::state::{PeerState, Presence, UiState};
use super::super::theme::{self, c};
use super::super::Ui;
use super::channels::{group_head, self_dot};
use super::widgets::{
    avatar, focus_ring, meter_view, pane_head, pane_title, popover_surface, selectable, tooltip,
    tracked, Dot,
};
use super::{format, AppHandle, AppSnapshot, PaneInit};
use gpui_kit::base::slider::{SliderEvent, SliderState};
use gpui_kit::base::{Popover, Slider as BaseSlider, SliderIndicator, SliderThumb, SliderTrack};
use gpui_kit::{
    div, prelude::FluentBuilder, px, relative, AnyElement, App, AppContext, ClickEvent, Context,
    Div, ElementId, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, Subscription, TestSupportExt, Window,
};
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

pub struct MembersPane {
    app: AppHandle,
    /// One slider state per volume in the open audio popover, made on first sight and
    /// dropped when the popover closes.
    sliders: HashMap<VolumeKey, VolumeSlider>,
}

/// One volume of somebody else's: their voice, their screen audio, their Spotify.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum VolumeKey {
    Voice(String),
    Desktop(String, u32),
    Music(String, u32),
}

impl VolumeKey {
    /// The engine's value now; `None` once that source is gone.
    fn current(&self, s: &UiState) -> Option<f32> {
        let peer = |id: &str| s.peers.iter().find(|p| p.id == id);
        match self {
            VolumeKey::Voice(id) => peer(id).map(|p| p.volume),
            VolumeKey::Desktop(id, stream) => peer(id)
                .filter(|p| p.desktop_stream == Some(*stream))
                .map(|p| p.desktop_volume.unwrap_or(1.0)),
            VolumeKey::Music(id, stream) => peer(id)
                .and_then(|p| p.music.as_ref())
                .filter(|m| m.stream == *stream)
                .map(|m| m.volume),
        }
    }

    fn send(&self, ui: &Ui, volume: f32) {
        match self {
            VolumeKey::Voice(peer) => commands::set_peer_volume(ui, peer, volume),
            VolumeKey::Desktop(peer, stream) | VolumeKey::Music(peer, stream) => {
                commands::set_stream_volume(ui, peer, *stream, volume)
            }
        }
    }
}

struct VolumeSlider {
    state: Entity<SliderState>,
    /// Between the first change and the release the thumb is the user's: a state that
    /// arrives meanwhile must not pull it back.
    dragging: Rc<Cell<bool>>,
    /// The engine's value when the thumb was last set from it. The thumb follows the
    /// engine only when that moves, so a value just let go of is not pulled back to the
    /// old one for the moment before the engine's next state confirms it.
    synced: u32,
    _changes: Subscription,
}

impl MembersPane {
    pub fn new(init: &PaneInit, _window: &mut Window, _cx: &mut Context<Self>) -> Self {
        MembersPane {
            app: init.app.clone(),
            sliders: HashMap::new(),
        }
    }

    /// The slider for `key`, following the engine's `value` (whole percents) whenever that
    /// changes and the thumb is not being dragged.
    /// Every change goes to the engine at once, as the old `input` handler sent it.
    fn slider(
        &mut self,
        key: VolumeKey,
        value: u32,
        snap: &AppSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<SliderState> {
        let entry = self.sliders.entry(key.clone()).or_insert_with(|| {
            let state = cx.new(|_| {
                SliderState::new()
                    .min(0.)
                    .max(100.)
                    .step(1.)
                    .default_value(value as f32)
            });
            let dragging = Rc::new(Cell::new(false));
            let held = dragging.clone();
            let ui = snap.ui.clone();
            let changes = cx.subscribe(&state, move |_, _, event: &SliderEvent, cx| match event {
                SliderEvent::Change(v) => {
                    held.set(true);
                    key.send(&ui, v.end() / 100.);
                    // The readout follows the thumb.
                    cx.notify();
                }
                SliderEvent::Release(_) => held.set(false),
            });
            VolumeSlider {
                state,
                dragging,
                synced: value,
                _changes: changes,
            }
        });
        if !entry.dragging.get() && entry.synced != value {
            entry.synced = value;
            entry
                .state
                .update(cx, |s, cx| s.set_value(value as f32, window, cx));
        }
        entry.state.clone()
    }
}

impl Render for MembersPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(snap) = self.app.snapshot(cx) else {
            return div().into_any_element();
        };
        let s = &snap.s;
        let me = Me {
            joined: s.voice.joined,
            dnd: s.do_not_disturb,
            sharing: snap.sharing_self(),
            camera: snap.own_camera().is_some(),
        };
        let groups = member_groups(&me, &s.peers);
        let mut used = HashSet::new();
        let mut rendered: Vec<AnyElement> = Vec::new();
        for (i, group) in groups.iter().enumerate() {
            let rows: Vec<AnyElement> = group
                .rows
                .iter()
                .map(|row| self.member_row(&snap, row, &mut used, window, cx))
                .collect();
            rendered.push(
                div()
                    .when(i > 0, |d| d.mt(px(18.)))
                    .child(group_head(group.label, group.rows.len(), None))
                    .children(rows)
                    .into_any_element(),
            );
        }
        self.sliders.retain(|k, _| used.contains(k));

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::BG_SIDEBAR)
            .border_l_1()
            .border_color(theme::BORDER)
            .child(
                pane_head()
                    .justify_between()
                    .px(px(18.))
                    .child(pane_title("Members", theme::DISPLAY_TEXT))
                    .child(
                        div()
                            .font_family(theme::MONO_FONT)
                            .text_size(theme::LABEL_TEXT)
                            .text_color(theme::TEXT_MUTED)
                            .child((snap.known_peers().count() + 1).to_string()),
                    ),
            )
            .child(
                div()
                    .id("members-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .pt(px(12.))
                    .px(px(8.))
                    .pb(px(16.))
                    .children(rendered),
            )
            .into_any_element()
    }
}

// ------------------------------------------------------------------ the groups

/// The facts about ourselves the groups need.
struct Me {
    joined: bool,
    dnd: bool,
    sharing: bool,
    camera: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Who<'a> {
    Me,
    Peer(&'a PeerState),
}

/// The second line of a row.
#[derive(Debug, Clone, PartialEq)]
enum Sub {
    None,
    Text(String),
    /// "speaking" / "in call" / "muted" / "muted for you": decided on every meters tick
    /// (`call_word`), the way `applyMeters` patched it.
    Call,
}

#[derive(Debug, Clone, PartialEq)]
struct Row<'a> {
    who: Who<'a>,
    sub: Sub,
}

struct Group<'a> {
    label: &'static str,
    rows: Vec<Row<'a>>,
}

/// `renderMembers`' grouping: In the call, Online, Offline, Not introduced yet,
/// in that order, empty groups left out.
fn member_groups<'a>(me: &Me, peers: &'a [PeerState]) -> Vec<Group<'a>> {
    let known = || peers.iter().filter(|p| !p.id.is_empty());
    let in_call: Vec<&PeerState> = known().filter(|p| p.in_call).collect();
    let call_running = !in_call.is_empty() || me.joined;

    let mut call: Vec<Row> = Vec::new();
    if me.joined {
        call.push(Row {
            who: Who::Me,
            sub: Sub::Call,
        });
    }
    call.extend(in_call.iter().map(|p| Row {
        who: Who::Peer(p),
        sub: Sub::Call,
    }));

    let mut online: Vec<Row> = Vec::new();
    if !me.joined {
        let sub = if me.dnd {
            "do not disturb"
        } else if me.sharing {
            "sharing your screen"
        } else if me.camera {
            "camera on"
        } else if call_running {
            "not in the call"
        } else {
            ""
        };
        online.push(Row {
            who: Who::Me,
            sub: text(sub),
        });
    }
    let not_in_call = if call_running { "not in the call" } else { "" };
    online.extend(
        known()
            .filter(|p| p.presence == Presence::Online && !p.in_call)
            .map(|p| Row {
                who: Who::Peer(p),
                sub: text(not_in_call),
            }),
    );

    let away = known()
        .filter(|p| matches!(p.presence, Presence::Offline | Presence::Broken))
        .map(|p| {
            let sub = match p.problem.as_deref().filter(|t| !t.is_empty()) {
                Some(problem) => problem.to_string(),
                None => format::last_seen_line(p.last_seen),
            };
            Row {
                who: Who::Peer(p),
                sub: text(&sub),
            }
        })
        .collect();
    let unknown = peers
        .iter()
        .filter(|p| p.id.is_empty())
        .map(|p| Row {
            who: Who::Peer(p),
            sub: text(&p.address),
        })
        .collect();

    [
        ("In the call", call),
        ("Online", online),
        ("Offline", away),
        ("Not introduced yet", unknown),
    ]
    .into_iter()
    .filter(|(_, rows)| !rows.is_empty())
    .map(|(label, rows)| Group { label, rows })
    .collect()
}

fn text(t: &str) -> Sub {
    if t.is_empty() {
        Sub::None
    } else {
        Sub::Text(t.to_string())
    }
}

/// The first word of an in-call sub-line. Our own mute wins over speaking, because nothing
/// is sent; a peer we muted still shows when they speak.
fn call_word(me: bool, muted: bool, speaking: bool, volume: f32) -> &'static str {
    if me {
        if muted {
            "muted"
        } else if speaking {
            "speaking"
        } else {
            "in call"
        }
    } else if speaking {
        "speaking"
    } else if volume == 0.0 {
        "muted for you"
    } else {
        "in call"
    }
}

/// The mute button: muting is volume 0 with the old level remembered; unmuting goes back
/// to that level, or full when nothing was remembered. Returns the new volume and what to
/// remember.
fn mute_toggle(volume: f32, remembered: Option<f32>) -> (f32, Option<f32>) {
    if volume == 0.0 {
        (remembered.unwrap_or(1.0), None)
    } else {
        (0.0, Some(volume))
    }
}

// ------------------------------------------------------------------ rows

/// A row's mono second line; a quiet (offline) peer's goes one tier dimmer.
fn mem_sub(content: impl IntoElement, quiet: bool) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(5.))
        .font_family(theme::MONO_FONT)
        .text_size(theme::LABEL_TEXT)
        .text_color(if quiet {
            theme::TEXT_DIM
        } else {
            theme::TEXT_MUTED
        })
        // One line: a long problem sentence ends in an ellipsis rather than growing the row.
        .child(
            div()
                .flex_1()
                .min_w_0()
                .whitespace_nowrap()
                .overflow_hidden()
                .text_ellipsis()
                .child(content),
        )
}

/// The percentage next to a slider.
fn vol_num(value: u32) -> Div {
    div()
        .flex_none()
        .min_w(px(34.))
        .flex()
        .justify_end()
        .font_family(theme::MONO_FONT)
        .text_size(theme::LABEL_TEXT)
        .text_color(theme::TEXT_MUTED)
        .child(format!("{value}%"))
}

/// A volume slider: a 6px pill (1px hairline frame included) on a lift, chrome up to the
/// value, and a 12px white thumb travelling inside the frame. The frame is its own element:
/// GPUI paints a border over the children, and the thumb must cover it.
fn range(state: &Entity<SliderState>, cx: &App) -> impl IntoElement {
    let pct = state.read(cx).percentage().end.clamp(0., 1.);
    BaseSlider::new(state)
        .flex_1()
        .min_w_0()
        .m(px(2.))
        .h(px(18.))
        .flex()
        .items_center()
        .child(
            SliderTrack::new(state)
                .w_full()
                .h(px(18.))
                .flex()
                .items_center()
                .cursor_pointer()
                .child(
                    SliderIndicator::new(state)
                        .relative()
                        .w_full()
                        .h(px(6.))
                        .child(
                            div()
                                .absolute()
                                .top_0()
                                .left_0()
                                .size_full()
                                .rounded_full()
                                .bg(c::LIFT)
                                .border_1()
                                .border_color(theme::BORDER),
                        )
                        .child(
                            div()
                                .absolute()
                                .top(px(1.))
                                .bottom(px(1.))
                                .left(px(1.))
                                .right(px(1.))
                                .child(
                                    div()
                                        .absolute()
                                        .top_0()
                                        .bottom_0()
                                        .left_0()
                                        .w(relative(pct))
                                        .rounded_full()
                                        .bg(theme::ACCENT),
                                )
                                .child(
                                    SliderThumb::new(state)
                                        .absolute()
                                        .top(px(-4.))
                                        .left(relative(pct))
                                        .ml(px(-12. * pct))
                                        .size(px(12.))
                                        .rounded_full()
                                        .bg(c::WHITE),
                                ),
                        ),
                ),
        )
}

impl MembersPane {
    fn member_row(
        &mut self,
        snap: &AppSnapshot,
        row: &Row,
        used: &mut HashSet<VolumeKey>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let s = &snap.s;
        let (key, name, hue, dot, peer) = match row.who {
            Who::Me => (
                "@self".to_string(),
                s.me.name.clone(),
                s.me.avatar,
                self_dot(s),
                None,
            ),
            Who::Peer(p) => {
                // An unintroduced peer has no id yet; its address is what tells it apart.
                let key = if p.id.is_empty() {
                    format!("@{}", p.address)
                } else {
                    p.id.clone()
                };
                (key, p.name.clone(), p.avatar, Dot::of_peer(p), Some(p))
            }
        };
        let me = peer.is_none();
        let quiet = peer.is_some_and(|p| p.presence != Presence::Online);
        let peer_id = peer.map(|p| p.id.clone()).unwrap_or_default();

        // No screen over a quiet sprite: the stylesheet meant one, but `.speak-ring::after`
        // kept it at opacity 0, so the shipped window (and every reference) never showed it.
        let ring = {
            let (name, id) = (name.clone(), peer_id.clone());
            meter_view(
                ElementId::Name(format!("mem-av-{key}").into()),
                &snap.meters,
                window,
                cx,
                move |m, _, _| {
                    let speaking = if me {
                        m.self_speaking()
                    } else {
                        m.speaking(&id)
                    };
                    avatar(&name, hue, 32., Some(dot), speaking).into_any_element()
                },
            )
        };

        // Everything after the first word of the sub-line, joined with " · ".
        let mut rest: Vec<String> = Vec::new();
        if let Some(label) = peer
            .and_then(|p| p.user_status.as_deref())
            .map(format::status_label)
            .filter(|l| !l.is_empty())
        {
            rest.push(label.to_lowercase());
        }
        // Plain text only: GPUI draws a string as glyphs, there is nothing in it to run.
        if let Some(game) = peer.and_then(|p| p.activity.as_deref()).filter(|_| !quiet) {
            rest.push(format!("playing {game}"));
        }
        let (screen, camera, spotify) = match peer {
            None => (
                snap.sharing_self(),
                snap.own_camera().is_some(),
                snap.own_music().is_some(),
            ),
            Some(p) => {
                let theirs = || {
                    s.streams
                        .iter()
                        .filter(|st| st.owner == p.id && !st.is_audio)
                };
                (
                    theirs().any(|st| !st.is_camera),
                    theirs().any(|st| st.is_camera),
                    p.music.is_some(),
                )
            }
        };
        // What they share only reads in the call; outside it our own row already says it.
        if row.sub == Sub::Call {
            for (on, what) in [(screen, "screen"), (camera, "camera"), (spotify, "Spotify")] {
                if on {
                    rest.push(what.into());
                }
            }
        }

        let sub: Option<AnyElement> = match &row.sub {
            Sub::Call => {
                let muted = s.voice.muted;
                let volume = peer.map_or(1.0, |p| p.volume);
                let id = peer_id.clone();
                let rest = rest.clone();
                Some(
                    meter_view(
                        ElementId::Name(format!("mem-sub-{key}").into()),
                        &snap.meters,
                        window,
                        cx,
                        move |m, _, _| {
                            let speaking = if me {
                                m.self_speaking()
                            } else {
                                m.speaking(&id)
                            };
                            let line = std::iter::once(call_word(me, muted, speaking, volume))
                                .chain(rest.iter().map(String::as_str))
                                .collect::<Vec<_>>()
                                .join(" \u{00b7} ");
                            mem_sub(line, quiet)
                                .when(speaking, |d| d.text_color(theme::ONLINE))
                                .into_any_element()
                        },
                    )
                    .into_any_element(),
                )
            }
            Sub::Text(t) => {
                let line = std::iter::once(t.clone())
                    .chain(rest)
                    .collect::<Vec<_>>()
                    .join(" \u{00b7} ");
                Some(mem_sub(line, quiet).into_any_element())
            }
            Sub::None if rest.is_empty() => None,
            Sub::None => Some(mem_sub(rest.join(" \u{00b7} "), quiet).into_any_element()),
        };

        // Only a known peer has volumes of their own to set.
        let popover = peer.filter(|p| !p.id.is_empty());
        let open = popover.is_some_and(|p| snap.v.audio_for.as_deref() == Some(p.id.as_str()));
        let line = selectable(
            div()
                .id(ElementId::Name(format!("mem-{key}").into()))
                .flex()
                .items_center()
                .gap(px(10.))
                .pt(px(7.))
                .px(px(10.))
                .pb(px(7.))
                .when(popover.is_some(), |d| d.cursor_pointer())
                .hover(|st| st.bg(theme::BG_HOVER)),
            open,
        )
        .child(div().flex_none().w(px(32.)).child(ring))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(
                            div()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_size(px(14.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme::TEXT_STRONG)
                                .child(name),
                        )
                        .when(me, |d| {
                            d.child(
                                div()
                                    .flex_none()
                                    .text_size(px(10.))
                                    .text_color(theme::TEXT_DIM)
                                    .child(tracked("YOU", 10., 0.05)),
                            )
                        }),
                )
                .children(sub),
        )
        .test_support();

        let Some(p) = popover else {
            return line.into_any_element();
        };
        let card = open.then(|| self.audio_card(snap, p, used, window, cx));
        let (app, id) = (snap.app.clone(), p.id.clone());
        Popover::new(ElementId::Name(format!("audio-{}", p.id).into()))
            .w_full()
            .anchor(gpui_kit::Anchor::RightCenter)
            .offset(px(12.))
            .open(open)
            .on_open_change(move |open, _, cx| {
                let id = id.clone();
                let open = *open;
                app.update(cx, |app, cx| {
                    let v = app.v_mut();
                    if open {
                        v.audio_for = Some(id);
                    } else if v.audio_for.as_deref() == Some(id.as_str()) {
                        v.audio_for = None;
                    }
                    cx.notify();
                });
            })
            .trigger_with(move |_, _, _| line.into_any_element())
            .content(move |_, _, _| card.unwrap_or_else(|| div().into_any_element()))
            .into_any_element()
    }

    /// "Audio for <name>": one row per source we hear from them, each with its slider, its
    /// mute and its percentage. The only place these volumes are set.
    fn audio_card(
        &mut self,
        snap: &AppSnapshot,
        p: &PeerState,
        used: &mut HashSet<VolumeKey>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut sources = vec![(VolumeKey::Voice(p.id.clone()), "Voice")];
        if let Some(stream) = p.desktop_stream {
            sources.push((VolumeKey::Desktop(p.id.clone(), stream), "Screen audio"));
        }
        if let Some(m) = p.music.as_ref().filter(|m| m.listening) {
            sources.push((VolumeKey::Music(p.id.clone(), m.stream), "Spotify"));
        }
        let only_voice = sources.len() == 1;
        let rows: Vec<AnyElement> = sources
            .into_iter()
            .filter_map(|(k, label)| {
                let value = k.current(&snap.s)?;
                used.insert(k.clone());
                let state = self.slider(k.clone(), format::vol_pct(value), snap, window, cx);
                Some(volume_row(
                    snap,
                    &p.name,
                    k,
                    label,
                    value == 0.0,
                    &state,
                    cx,
                ))
            })
            .collect();
        popover_surface()
            .id("audio-popover")
            .w(px(284.))
            .p(px(16.))
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(avatar(&p.name, p.avatar, 28., None, false))
                    .child(pane_title(p.name.clone(), px(21.)).flex_1())
                    .child(
                        div()
                            .flex_none()
                            .font_family(theme::MONO_FONT)
                            .text_size(px(11.))
                            .text_color(theme::TEXT_DIM)
                            .child("only for you"),
                    ),
            )
            .children(rows)
            .when(only_voice, |d| {
                d.child(
                    div()
                        .pt(px(12.))
                        .border_t_1()
                        .border_color(theme::BORDER)
                        .text_size(px(12.))
                        .text_color(theme::TEXT_MUTED)
                        .child(format!(
                            "Screen audio and Spotify appear here while {} shares them.",
                            p.name
                        )),
                )
            })
            .test_support()
            .into_any_element()
    }
}

/// One source in the audio popover: label and percentage, then mute and slider.
fn volume_row(
    snap: &AppSnapshot,
    name: &str,
    key: VolumeKey,
    label: &'static str,
    muted: bool,
    state: &Entity<SliderState>,
    cx: &App,
) -> AnyElement {
    let what = match key {
        VolumeKey::Voice(_) => name.to_string(),
        _ => format!("{name}'s {}", label.to_lowercase()),
    };
    let title = if muted {
        format!("Unmute {what} for you")
    } else {
        format!("Mute {what} for you")
    };
    let id = match &key {
        VolumeKey::Voice(p) => format!("vmute-{p}"),
        VolumeKey::Desktop(p, st) => format!("dmute-{p}-{st}"),
        VolumeKey::Music(p, st) => format!("smute-{p}-{st}"),
    };
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(
            div()
                .flex()
                .items_center()
                .text_size(px(13.))
                .child(div().flex_1().child(label))
                .child(vol_num(state.read(cx).value().end().round() as u32)),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .id(ElementId::Name(id.into()))
                        .tab_index(0)
                        .focus_visible(|s| s.shadow(focus_ring()))
                        .size(px(28.))
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .rounded(theme::RADIUS_CONTROL)
                        .cursor_pointer()
                        .text_color(if muted {
                            theme::DANGER_TEXT
                        } else {
                            theme::TEXT_MUTED
                        })
                        .when(muted, |d| d.bg(theme::DANGER_BG))
                        .when(!muted, |d| {
                            d.hover(|st| st.bg(theme::BG_HOVER).text_color(theme::TEXT_STRONG))
                        })
                        .tooltip(tooltip(title))
                        .child(if muted { Icon::VolumeOff } else { Icon::Volume }.size(15.))
                        .on_click(snap.app.listener(move |app, _: &ClickEvent, _, _| {
                            let Some(volume) = key.current(app.state()) else {
                                return;
                            };
                            let remembered = app.view_state().volume_before_mute.get(&key).copied();
                            let (volume, remember) = mute_toggle(volume, remembered);
                            if let Some(level) = remember {
                                app.v_mut().volume_before_mute.insert(key.clone(), level);
                            }
                            key.send(app.ui(), volume);
                        })),
                )
                .child(range(state, cx)),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(id: &str, name: &str, presence: Presence, in_call: bool) -> PeerState {
        PeerState {
            id: id.into(),
            name: name.into(),
            address: format!("{name}:41650"),
            avatar: 1,
            presence,
            user_status: None,
            activity: None,
            problem: None,
            path: None,
            app_version: None,
            in_call,
            volume: 1.0,
            unread: 0,
            last_seen: None,
            desktop_volume: None,
            desktop_stream: None,
            music: None,
            slot: 0,
        }
    }

    fn shape(groups: &[Group]) -> Vec<(&'static str, Vec<(String, Sub)>)> {
        groups
            .iter()
            .map(|g| {
                let rows = g
                    .rows
                    .iter()
                    .map(|r| {
                        let who = match r.who {
                            Who::Me => "me".to_string(),
                            Who::Peer(p) => p.name.clone(),
                        };
                        (who, r.sub.clone())
                    })
                    .collect();
                (g.label, rows)
            })
            .collect()
    }

    /// The `connecting` reference: me alone online, one of each trouble. Dropping the
    /// `!p.id.is_empty()` filter from `known` puts the address into Offline too.
    #[test]
    fn every_presence_lands_in_its_group() {
        let mut broken = peer("b", "Bram", Presence::Broken, false);
        broken.problem = Some("Runs protocol 4.".into());
        let peers = vec![
            peer("s", "Sander", Presence::Offline, false),
            peer("j", "Joost", Presence::Offline, false),
            broken,
            peer("", "100.88.14.2:41650", Presence::Offline, false),
        ];
        let me = Me {
            joined: false,
            dnd: false,
            sharing: false,
            camera: false,
        };
        assert_eq!(
            shape(&member_groups(&me, &peers)),
            vec![
                ("Online", vec![("me".into(), Sub::None)]),
                (
                    "Offline",
                    vec![
                        ("Sander".into(), Sub::Text("offline".into())),
                        ("Joost".into(), Sub::Text("offline".into())),
                        ("Bram".into(), Sub::Text("Runs protocol 4.".into())),
                    ]
                ),
                (
                    "Not introduced yet",
                    vec![(
                        "100.88.14.2:41650".into(),
                        Sub::Text("100.88.14.2:41650:41650".into())
                    )]
                ),
            ]
        );
    }

    /// In a call I lead the call group and leave Online; a peer outside the call reads
    /// "Not in the call". Without the `!me.joined` guard I would appear twice.
    #[test]
    fn in_a_call_i_lead_the_call_group() {
        let peers = vec![
            peer("s", "Sander", Presence::Online, true),
            peer("j", "Joost", Presence::Online, false),
        ];
        let me = Me {
            joined: true,
            dnd: false,
            sharing: true,
            camera: false,
        };
        let groups = member_groups(&me, &peers);
        assert_eq!(
            shape(&groups),
            vec![
                (
                    "In the call",
                    vec![("me".into(), Sub::Call), ("Sander".into(), Sub::Call)]
                ),
                (
                    "Online",
                    vec![("Joost".into(), Sub::Text("not in the call".into()))]
                ),
            ]
        );
    }

    /// Our mute wins over speaking, a peer's speaking over our mute of them. Swapping the
    /// first two checks of the `me` arm fails the first assertion.
    #[test]
    fn the_call_word_follows_the_meters() {
        assert_eq!(call_word(true, true, true, 1.0), "muted");
        assert_eq!(call_word(true, false, true, 1.0), "speaking");
        assert_eq!(call_word(true, false, false, 1.0), "in call");
        assert_eq!(call_word(false, false, false, 0.0), "muted for you");
        assert_eq!(call_word(false, false, true, 0.0), "speaking");
        assert_eq!(call_word(false, false, false, 0.5), "in call");
    }

    /// Mute remembers the level and unmute brings it back; returning a constant 1.0 on
    /// unmute fails the second assertion.
    #[test]
    fn mute_is_volume_zero_with_the_level_remembered() {
        assert_eq!(mute_toggle(0.4, None), (0.0, Some(0.4)));
        assert_eq!(mute_toggle(0.0, Some(0.4)), (0.4, None));
        assert_eq!(mute_toggle(0.0, None), (1.0, None));
    }
}

#[cfg(test)]
mod window_tests {
    use super::super::composer::window_tests::{Harness, SANDER};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::TestAppContext;

    /// A click on a peer opens their audio popover; a click on your own row opens nothing.
    /// Revert to see it fail: drop the `on_open_change` on the row's `Popover` (the view state
    /// never learns it opened), or wrap the own row in a `Popover` too.
    #[gpui_kit::test]
    fn a_click_on_a_peer_opens_their_audio_popover(cx: &mut TestAppContext) {
        let h = Harness::new(cx);
        h.app.update(cx, |app, cx| {
            app.v_mut().members = true;
            cx.notify();
        });
        cx.run_until_parked();
        let state = |cx: &mut TestAppContext| {
            let open = h.app.read_with(cx, |app, _| app.v.audio_for.clone());
            let card = h.with_window(cx, |window, cx| {
                // The popup places itself from its trigger's bounds of the frame before.
                window.render_frame(cx);
                window.render_frame(cx);
                window.try_find("audio-popover").is_some()
            });
            (open, card)
        };

        h.with_window(cx, |window, cx| {
            window.render_frame(cx);
            window.click("mem-@self", cx);
        });
        cx.run_until_parked();
        assert_eq!(state(cx), (None, false), "your own row opens nothing");

        let row = gpui_kit::ElementId::Name(format!("mem-{SANDER}").into());
        h.with_window(cx, move |window, cx| window.click(row, cx));
        cx.run_until_parked();
        assert_eq!(state(cx), (Some(SANDER.to_string()), true));
    }
}
