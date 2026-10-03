//! The member list (`renderMembers`, `memberRow`): the five groups, speaking state through
//! `widgets::meter_view`, status, last seen, the per-peer voice volume (mute is volume 0
//! with the old level remembered in `v.volume_before_mute`), the desktop-audio volume, and
//! the Watch / Stop watching chips.

use super::super::commands;
use super::super::icons::Icon;
use super::super::state::{PeerState, Presence};
use super::super::theme::{self, c, Dither};
use super::channels::self_dot;
use super::widgets::{avatar, meter_view, tooltip, tracked, Dot};
use super::{format, AppHandle, AppSnapshot, PaneInit};
use gpui_kit::base::slider::{SliderEvent, SliderState};
use gpui_kit::base::{Slider as BaseSlider, SliderIndicator, SliderThumb, SliderTrack};
use gpui_kit::{
    div, prelude::FluentBuilder, px, relative, AnyElement, App, AppContext, ClickEvent, Context,
    Div, ElementId, Entity, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Window,
};
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

pub struct MembersPane {
    app: AppHandle,
    /// One slider state per voice or desktop-audio volume on screen, made on first sight
    /// and dropped when its row goes.
    sliders: HashMap<SliderKey, VolumeSlider>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum SliderKey {
    Voice(String),
    Desktop(String, u32),
}

struct VolumeSlider {
    state: Entity<SliderState>,
    /// Between the first change and the release the thumb is the user's: a state that
    /// arrives meanwhile must not pull it back.
    dragging: Rc<Cell<bool>>,
    _changes: Subscription,
}

impl MembersPane {
    pub fn new(init: &PaneInit, _window: &mut Window, _cx: &mut Context<Self>) -> Self {
        MembersPane {
            app: init.app.clone(),
            sliders: HashMap::new(),
        }
    }

    /// The slider for `key`, showing `value` (whole percents) unless it is being dragged.
    /// Every change goes to the engine at once, as the old `input` handler sent it.
    fn slider(
        &mut self,
        key: SliderKey,
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
                    let volume = v.end() / 100.;
                    match &key {
                        SliderKey::Voice(peer) => commands::set_peer_volume(&ui, peer, volume),
                        SliderKey::Desktop(peer, stream) => {
                            commands::set_stream_volume(&ui, peer, *stream, volume)
                        }
                    }
                    // The readout follows the thumb.
                    cx.notify();
                }
                SliderEvent::Release(_) => held.set(false),
            });
            VolumeSlider {
                state,
                dragging,
                _changes: changes,
            }
        });
        if !entry.dragging.get() && entry.state.read(cx).value().end().round() as u32 != value {
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
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .px(px(6.))
                            .pb(px(6.))
                            .text_size(px(11.))
                            .text_color(c::SILVER)
                            .child(tracked(&group.label.to_uppercase(), 11. * 0.08))
                            .child(div().ml_auto().child(group.rows.len().to_string())),
                    )
                    .children(rows)
                    .into_any_element(),
            );
        }
        self.sliders.retain(|k, _| used.contains(k));

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(c::INK)
            .border_l_1()
            .border_color(theme::BORDER)
            .child(
                div()
                    .h(theme::HEAD_H)
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(px(14.))
                    .bg(c::GREEN)
                    .border_b_1()
                    .border_color(c::SPACE)
                    .text_color(c::SPACE)
                    .child(
                        div()
                            .text_size(px(13.))
                            .child(tracked("MEMBERS", 13. * 0.08)),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
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
    /// "Speaking" / "Listening" / "Microphone muted" / "Muted for you": decided on every
    /// meters tick (`call_sub`), the way `applyMeters` patched it.
    Call,
}

#[derive(Debug, Clone, PartialEq)]
struct Row<'a> {
    who: Who<'a>,
    sub: Sub,
    /// The mute button and the voice volume slider.
    tools: bool,
}

struct Group<'a> {
    label: &'static str,
    rows: Vec<Row<'a>>,
}

/// `renderMembers`' grouping: In the call, Online, Connecting, Offline, Not introduced yet,
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
            tools: false,
        });
    }
    call.extend(in_call.iter().map(|p| Row {
        who: Who::Peer(p),
        sub: Sub::Call,
        tools: true,
    }));

    let mut online: Vec<Row> = Vec::new();
    if !me.joined {
        let sub = if me.dnd {
            "Do not disturb"
        } else if me.sharing {
            "Sharing your screen"
        } else if me.camera {
            "Camera on"
        } else if call_running {
            "Not in the call"
        } else {
            ""
        };
        online.push(Row {
            who: Who::Me,
            sub: text(sub),
            tools: false,
        });
    }
    let not_in_call = if call_running { "Not in the call" } else { "" };
    online.extend(
        known()
            .filter(|p| p.presence == Presence::Online && !p.in_call)
            .map(|p| Row {
                who: Who::Peer(p),
                sub: text(not_in_call),
                tools: false,
            }),
    );

    let connecting = known()
        .filter(|p| p.presence == Presence::Connecting)
        .map(|p| Row {
            who: Who::Peer(p),
            sub: text("Reconnecting"),
            tools: false,
        })
        .collect();
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
                tools: false,
            }
        })
        .collect();
    let unknown = peers
        .iter()
        .filter(|p| p.id.is_empty())
        .map(|p| Row {
            who: Who::Peer(p),
            sub: text(&p.address),
            tools: false,
        })
        .collect();

    [
        ("In the call", call),
        ("Online", online),
        ("Connecting", connecting),
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

/// The in-call second line, as the old render and its meter patch decided it together.
fn call_sub(me: bool, muted: bool, speaking: bool, volume: f32) -> &'static str {
    if me {
        if muted {
            "Microphone muted"
        } else if speaking {
            "Speaking"
        } else {
            "Listening"
        }
    } else if speaking {
        "Speaking"
    } else if volume == 0.0 {
        "Muted for you"
    } else {
        "Listening"
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

/// `.mem[data-quiet]`: half the cells of the status words go to ink, in 1px cells; hovering
/// the row lifts it.
fn quiet_screen(group: SharedString) -> Div {
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .group_hover(group, |s| s.opacity(0.))
        .child(theme::dither(Dither::Half, c::INK, 1.))
}

/// `.mem-sub`.
fn mem_sub(content: impl IntoElement, quiet: Option<SharedString>) -> Div {
    div()
        .relative()
        .flex()
        .items_center()
        .gap(px(5.))
        .text_size(px(11.5))
        .text_color(c::SILVER)
        .child(content)
        .when_some(quiet, |d, g| d.child(quiet_screen(g)))
}

/// `.vol-num`.
fn vol_num(value: u32) -> Div {
    div()
        .flex_none()
        .min_w(px(26.))
        .flex()
        .justify_end()
        .text_size(px(11.5))
        .text_color(c::SILVER)
        .child(value.to_string())
}

/// `input[type=range]` as Chromium drew it: 2px of margin, a 6px track (1px violet frame
/// included) that is cyan up to the value and void after it, and a 10×14 paper thumb with
/// a one-pixel drop, travelling inside the track's frame. The frame is its own element:
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
                                .bg(c::SPACE)
                                .border_1()
                                .border_color(c::VIOLET),
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
                                        .bg(theme::ACCENT),
                                )
                                .child(
                                    SliderThumb::new(state)
                                        .absolute()
                                        .top(px(-5.))
                                        .left(relative(pct))
                                        .ml(px(-10. * pct))
                                        .w(px(10.))
                                        .h(px(14.))
                                        .bg(c::PAPER)
                                        .border_1()
                                        .border_color(c::SPACE)
                                        .shadow(theme::shadow_thumb()),
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
        used: &mut HashSet<SliderKey>,
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
        let group = SharedString::from(format!("mem-{key}"));
        let quiet_group = quiet.then(|| group.clone());
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

        let status = peer
            .and_then(|p| p.user_status.as_deref())
            .map(format::status_label)
            .filter(|l| !l.is_empty());
        let sub: Option<AnyElement> = match &row.sub {
            Sub::None => None,
            Sub::Text(t) => Some(mem_sub(t.clone(), quiet_group.clone()).into_any_element()),
            Sub::Call => {
                let muted = s.voice.muted;
                let volume = peer.map_or(1.0, |p| p.volume);
                let id = peer_id.clone();
                let quiet_group = quiet_group.clone();
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
                            mem_sub(call_sub(me, muted, speaking, volume), quiet_group.clone())
                                .into_any_element()
                        },
                    )
                    .into_any_element(),
                )
            }
        };

        let tools = match peer.filter(|_| row.tools) {
            Some(p) => {
                let k = SliderKey::Voice(p.id.clone());
                used.insert(k.clone());
                let state = self.slider(k, format::vol_pct(p.volume), snap, window, cx);
                let muted = p.volume == 0.0;
                let id = p.id.clone();
                let label = if muted {
                    format!("Unmute {}", p.name)
                } else {
                    format!("Mute {}", p.name)
                };
                Some(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .mt(px(7.))
                        .child(
                            div()
                                .id(ElementId::Name(format!("pmute-{}", p.id).into()))
                                .size(px(24.))
                                .flex()
                                .flex_none()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .text_color(if muted { theme::DND } else { c::SILVER })
                                .hover(move |st| {
                                    st.bg(c::PAPER).text_color(if muted {
                                        theme::DND
                                    } else {
                                        theme::ON_INVERT
                                    })
                                })
                                .tooltip(tooltip(label))
                                .child(if muted { Icon::MicOff } else { Icon::Mic }.size(14.))
                                .on_click(snap.app.listener(move |app, _: &ClickEvent, _, _| {
                                    let Some(p) = app.state().peers.iter().find(|p| p.id == id)
                                    else {
                                        return;
                                    };
                                    let remembered =
                                        app.view_state().volume_before_mute.get(&id).copied();
                                    let (volume, remember) = mute_toggle(p.volume, remembered);
                                    if let Some(level) = remember {
                                        app.v_mut().volume_before_mute.insert(id.clone(), level);
                                    }
                                    commands::set_peer_volume(app.ui(), &id, volume);
                                })),
                        )
                        .child(range(&state, cx))
                        .child(vol_num(state.read(cx).value().end().round() as u32)),
                )
            }
            None => None,
        };

        let desk = match peer.and_then(|p| p.desktop_stream.map(|st| (p, st))) {
            Some((p, stream)) => {
                let k = SliderKey::Desktop(p.id.clone(), stream);
                used.insert(k.clone());
                let state = self.slider(
                    k,
                    format::vol_pct(p.desktop_volume.unwrap_or(1.0)),
                    snap,
                    window,
                    cx,
                );
                Some(
                    div()
                        .mt(px(6.))
                        .child(
                            div()
                                .text_size(px(9.5))
                                .line_height(relative(1.1))
                                .text_color(c::SILVER)
                                .child(tracked("SCREEN AUDIO", 9.5 * 0.04)),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .mt(px(2.))
                                .child(
                                    div()
                                        .flex_none()
                                        .text_color(theme::TEXT_DIM)
                                        .child(Icon::Monitor.size(14.)),
                                )
                                .child(range(&state, cx))
                                .child(vol_num(state.read(cx).value().end().round() as u32)),
                        ),
                )
            }
            None => None,
        };

        let chips: Vec<AnyElement> = match peer {
            Some(p) if !p.id.is_empty() => {
                let shared: Vec<_> = s.streams.iter().filter(|st| st.owner == p.id).collect();
                shared
                    .iter()
                    .filter(|st| !st.watching)
                    .chain(shared.iter().filter(|st| st.watching))
                    .map(|st| {
                        let watching = st.watching;
                        let (icon, verb) = match (watching, st.is_camera) {
                            (true, _) => (Icon::X, "Stop watching"),
                            (false, true) => (Icon::Cam, "Watch"),
                            (false, false) => (Icon::Monitor, "Watch"),
                        };
                        let (ui, owner, stream) = (snap.ui.clone(), p.id.clone(), st.stream_id);
                        share_chip(
                            ElementId::Name(format!("chip-{}-{}", p.id, st.stream_id).into()),
                            icon,
                            verb,
                            &st.title,
                        )
                        .on_click(move |_: &ClickEvent, _, _| {
                            commands::set_watching(&ui, &owner, stream, !watching)
                        })
                        .into_any_element()
                    })
                    .collect()
            }
            _ => Vec::new(),
        };

        div()
            .id(ElementId::Name(group.clone()))
            .group(group)
            .relative()
            .flex()
            .gap(px(9.))
            .pt(px(6.))
            .px(px(6.))
            .pb(px(7.))
            .hover(|st| st.bg(theme::BG_HOVER))
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
                                    .text_size(px(13.5))
                                    .child(name),
                            )
                            .when(me, |d| {
                                d.child(
                                    div()
                                        .flex_none()
                                        .text_size(px(10.))
                                        .text_color(c::SILVER)
                                        .child(tracked("YOU", 10. * 0.05)),
                                )
                            }),
                    )
                    .when_some(status, |d, label| {
                        d.child(mem_sub(label, quiet_group.clone()))
                    })
                    .children(sub)
                    .children(tools)
                    .children(desk)
                    .children(chips),
            )
            .into_any_element()
    }
}

/// `.share-chip`: an accent-framed chip whose verb and source break apart, never inside
/// the source; a very long source ellipsises on its own line.
fn share_chip(
    id: ElementId,
    icon: Icon,
    verb: &'static str,
    source: &str,
) -> gpui_kit::Stateful<Div> {
    div()
        .id(id)
        .mt(px(7.))
        .flex()
        .flex_wrap()
        .items_center()
        .gap_x(px(5.))
        .gap_y(px(1.))
        .max_w_full()
        .pt(px(3.))
        .pr(px(8.))
        .pb(px(3.))
        .pl(px(6.))
        .bg(c::SPACE)
        .text_color(theme::ACCENT)
        .border_1()
        .border_color(theme::ACCENT)
        .text_size(px(11.5))
        .cursor_pointer()
        .hover(|s| s.bg(theme::ACCENT).text_color(theme::ON_ACCENT))
        .child(icon.size(12.))
        .child(div().whitespace_nowrap().child(verb))
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(source.to_string()),
        )
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
            problem: None,
            app_version: None,
            in_call,
            volume: 1.0,
            unread: 0,
            last_seen: None,
            desktop_volume: None,
            desktop_stream: None,
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
    /// `!p.id.is_empty()` filter from `known` puts the address into Connecting too.
    #[test]
    fn every_presence_lands_in_its_group() {
        let mut broken = peer("b", "Bram", Presence::Broken, false);
        broken.problem = Some("Runs protocol 4.".into());
        let peers = vec![
            peer("s", "Sander", Presence::Connecting, false),
            peer("j", "Joost", Presence::Offline, false),
            broken,
            peer("", "100.88.14.2:41650", Presence::Connecting, false),
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
                    "Connecting",
                    vec![("Sander".into(), Sub::Text("Reconnecting".into()))]
                ),
                (
                    "Offline",
                    vec![
                        ("Joost".into(), Sub::Text("Offline".into())),
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
                    vec![("Joost".into(), Sub::Text("Not in the call".into()))]
                ),
            ]
        );
        assert!(groups[0].rows[1].tools && !groups[0].rows[0].tools);
    }

    #[test]
    fn the_call_line_follows_the_meters() {
        assert_eq!(call_sub(true, true, true, 1.0), "Microphone muted");
        assert_eq!(call_sub(true, false, true, 1.0), "Speaking");
        assert_eq!(call_sub(false, false, false, 0.0), "Muted for you");
        assert_eq!(call_sub(false, false, true, 0.0), "Speaking");
        assert_eq!(call_sub(false, false, false, 0.5), "Listening");
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
