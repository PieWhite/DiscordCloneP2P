//! The channel column (`renderChannels`, `renderVoice`): the channel list with unread badges
//! and the collapsible general group, the DM list, and the voice panel at the bottom
//! (join/leave, the people in the call with their speaking frame, share and camera lines,
//! the self row with mute, camera, deafen and do-not-disturb).
//!
//! Right-click on a removable sub-channel opens the rename prompt straight away, as the old
//! `contextmenu` handler did; there is no menu in between.

use super::super::commands;
use super::super::icons::Icon;
use super::super::state::{ChannelState, PeerState, UiState};
use super::super::theme::{self, c, Dither};
use super::widgets::{
    avatar, badge, focus_ring, icon_btn, meter_view, pointer, tooltip, tracked, Btn, BtnKind, Dot,
};
use super::{AppHandle, AppSnapshot, Dialog, PaneInit, View};
use gpui_kit::{
    div, prelude::FluentBuilder, px, radians, relative, AnyElement, App, ClickEvent, Context, Div,
    ElementId, HighlightStyle, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    ParentElement, Render, SharedString, Stateful, StatefulInteractiveElement, Styled, StyledText,
    Window,
};

pub struct ChannelsPane {
    app: AppHandle,
}

impl ChannelsPane {
    pub fn new(init: &PaneInit, _window: &mut Window, _cx: &mut Context<Self>) -> Self {
        ChannelsPane {
            app: init.app.clone(),
        }
    }
}

impl Render for ChannelsPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(snap) = self.app.snapshot(cx) else {
            return div().into_any_element();
        };
        // `@media (max-width: 820px)`: the self row's toggles slim before the name does.
        let narrow = window.viewport_size().width <= theme::NARROW_CHANNELS;
        let dms = snap.v.view == View::Dms;

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(c::INK)
            .border_r_1()
            .border_color(theme::BORDER)
            .child(head(
                &snap,
                if dms { "Direct messages" } else { "Channels" },
            ))
            .child(
                div()
                    .id("chan-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .pt(px(10.))
                    .px(px(8.))
                    .pb(px(14.))
                    .children(if dms {
                        dm_list(&snap)
                    } else {
                        channel_list(&snap)
                    }),
            )
            .child(voice_panel(&snap, narrow, window, cx))
            .into_any_element()
    }
}

// ------------------------------------------------------------------ head + lists

/// `.chan-head`: the magenta title strip with the New channel button. The button stays in
/// the DM view too, as it did.
fn head(snap: &AppSnapshot, title: &str) -> Div {
    div()
        .h(theme::HEAD_H)
        .flex_none()
        .flex()
        .items_center()
        .justify_between()
        .pl(px(14.))
        .pr(px(8.))
        .bg(c::MAGENTA)
        .border_b_1()
        .border_color(c::SPACE)
        .text_color(c::SPACE)
        .child(
            div()
                .text_size(px(13.))
                .child(tracked(&title.to_uppercase(), 13., 0.08)),
        )
        .child(
            icon_btn("new-channel", Icon::Plus, false)
                .tab_index(0)
                .focus_visible(|s| s.shadow(focus_ring(c::MAGENTA, vec![])))
                .tooltip(tooltip("New channel"))
                .on_click(snap.app.listener(|app, _: &ClickEvent, window, cx| {
                    app.open_dialog(Dialog::new_channel(), window, cx)
                })),
        )
}

/// `.group-head`: the small uppercase label with its count pushed right.
fn group_head(label: &str, count: usize, lead: Option<AnyElement>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(4.))
        .px(px(6.))
        .pb(px(6.))
        .text_size(px(11.))
        .text_color(c::SILVER)
        .children(lead)
        .child(tracked(&label.to_uppercase(), 11., 0.08))
        .child(div().ml_auto().child(count.to_string()))
}

/// `.group + .group { margin-top: 18px }`.
fn groups(list: Vec<Div>) -> Vec<AnyElement> {
    list.into_iter()
        .enumerate()
        .map(|(i, g)| g.when(i > 0, |g| g.mt(px(18.))).into_any_element())
        .collect()
}

fn channel_list(snap: &AppSnapshot) -> Vec<AnyElement> {
    let v = &snap.v;
    let collapsed = v.collapsed;
    let toggle = div()
        .id("collapse-general")
        .tab_index(0)
        .focus_visible(|s| s.shadow(focus_ring(c::INK, vec![])))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(|s| s.text_color(theme::TEXT))
        .tooltip(tooltip(if collapsed { "Expand" } else { "Collapse" }))
        .child(Icon::Chev.size(12.).when(collapsed, |i| {
            i.rotate(radians(-std::f32::consts::FRAC_PI_2))
        }))
        .on_click(snap.app.listener(|app, _: &ClickEvent, _, cx| {
            let v = app.v_mut();
            v.collapsed = !v.collapsed;
            cx.notify();
        }));

    // Collapsing keeps the channel you are in and anything unread: a collapse that hides a
    // channel shouting at you is a collapse that loses the message.
    let rows = snap
        .s
        .channels
        .iter()
        .filter(|c| !collapsed || c.unread > 0 || c.key == v.channel)
        .map(|c| channel_row(snap, c));
    let general = div()
        .child(group_head(
            "General",
            snap.s.channels.len(),
            Some(toggle.into_any_element()),
        ))
        .children(rows);

    let known: Vec<&PeerState> = snap.known_peers().collect();
    let mut list = vec![general];
    if !known.is_empty() {
        list.push(
            div()
                .child(group_head("Direct messages", known.len(), None))
                .children(known.iter().map(|p| dm_row(snap, p, false))),
        );
    }
    groups(list)
}

fn dm_list(snap: &AppSnapshot) -> Vec<AnyElement> {
    let known: Vec<&PeerState> = snap.known_peers().collect();
    if known.is_empty() {
        // A `<p class="voice-hint">` in the old markup: its own margins plus the padding.
        return vec![div()
            .my(px(12.))
            .px(px(12.))
            .pb(px(10.))
            .text_size(px(12.))
            .text_color(c::SILVER)
            .child(
                "No peer has introduced itself yet. Identities are learned on first contact, so a \
                 conversation appears here as soon as somebody connects.",
            )
            .into_any_element()];
    }
    let current = snap.v.dm.as_deref();
    groups(vec![div()
        .child(group_head("Conversations", known.len(), None))
        .children(
            known
                .iter()
                .map(|p| dm_row(snap, p, current == Some(p.id.as_str()))),
        )])
}

/// `.chan-item`: 32px menu row. Hover is the violet slab with the ▶ marker; the current row
/// inverts its cells instead and drops the marker; unread lifts the name to paper and puts
/// a block on the left edge.
fn menu_row(id: ElementId, group: SharedString, current: bool, unread: usize) -> Stateful<Div> {
    div()
        .id(id)
        .tab_index(0)
        .focus_visible(|s| s.shadow(focus_ring(c::INK, vec![])))
        .group(group.clone())
        .relative()
        .w_full()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(8.))
        .h(px(32.))
        .px(px(8.))
        .cursor_pointer()
        .text_color(if current {
            theme::ON_INVERT
        } else if unread > 0 {
            theme::TEXT
        } else {
            c::SILVER
        })
        .when(current, |d| d.bg(theme::BG_ACTIVE))
        .when(!current, |d| {
            d.hover(|s| s.bg(theme::BG_HOVER).text_color(theme::TEXT))
                .child(pointer().opacity(0.).group_hover(group, |s| s.opacity(1.)))
        })
        .when(unread > 0, |d| {
            d.child(
                div()
                    .absolute()
                    .left(px(-8.))
                    .top(px(12.))
                    .w(px(4.))
                    .h(px(8.))
                    .bg(c::PAPER),
            )
        })
}

fn row_name(name: &str) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .text_size(px(14.))
        .child(name.to_string())
}

/// The count on a row; on the inverted current row it re-inverts, or paper would vanish.
fn row_badge(count: usize, current: bool) -> Div {
    badge(count, false).when(current, |b| b.bg(c::SPACE).text_color(c::PAPER))
}

fn channel_row(snap: &AppSnapshot, ch: &ChannelState) -> AnyElement {
    let current = snap.v.view == View::Channels && snap.v.channel == ch.key;
    let group = SharedString::from(format!("chan-{}", ch.key));
    let icon_colour = if current {
        theme::ON_INVERT
    } else if ch.unread > 0 {
        c::SILVER
    } else {
        c::STEEL
    };
    let key = ch.key.clone();
    menu_row(
        ElementId::Name(group.clone()),
        group.clone(),
        current,
        ch.unread,
    )
    .child(
        div()
            .text_color(icon_colour)
            .when(!current, |d| {
                d.group_hover(group, |s| s.text_color(c::SILVER))
            })
            .child(Icon::Hash.size(16.)),
    )
    .child(row_name(&ch.name))
    .when(ch.unread > 0, |d| d.child(row_badge(ch.unread, current)))
    .on_click(snap.app.listener({
        let key = key.clone();
        move |app, _: &ClickEvent, window, cx| app.open_channel(&key, window, cx)
    }))
    .when(ch.removable, |d| {
        d.on_mouse_down(
            MouseButton::Right,
            snap.app
                .listener(move |app, _: &MouseDownEvent, window, cx| {
                    let Some(ch) = app
                        .state()
                        .channels
                        .iter()
                        .find(|c| c.key == key && c.removable)
                    else {
                        return;
                    };
                    let dialog = Dialog::rename_channel(ch);
                    app.open_dialog(dialog, window, cx);
                }),
        )
    })
    .into_any_element()
}

fn dm_row(snap: &AppSnapshot, p: &PeerState, current: bool) -> AnyElement {
    let group = SharedString::from(format!("dm-{}", p.id));
    let id = p.id.clone();
    menu_row(ElementId::Name(group.clone()), group, current, p.unread)
        .child(avatar(&p.name, p.avatar, 20., Some(Dot::of_peer(p)), false))
        .child(row_name(&p.name))
        .when(p.unread > 0, |d| d.child(row_badge(p.unread, current)))
        .on_click(
            snap.app
                .listener(move |app, _: &ClickEvent, window, cx| app.open_dm(&id, window, cx)),
        )
        .into_any_element()
}

// ------------------------------------------------------------------ voice panel

/// `selfPresence()` as `avatar()` used it for ourselves: our chosen status is not drawn
/// here (it never was), only do-not-disturb.
pub(super) fn self_dot(s: &UiState) -> Dot {
    if s.do_not_disturb {
        Dot::Dnd
    } else {
        Dot::Online
    }
}

/// `voiceHint()`, as written. The first branch cannot run (the hint shows only with an
/// empty roster); it stays so the logic reads the same as before.
fn voice_hint(call: &[&PeerState], anyone_online: bool) -> String {
    if call.len() > 1 {
        let names: Vec<&str> = call.iter().map(|p| p.name.as_str()).collect();
        let (last, rest) = names.split_last().expect("len > 1");
        return format!("{} and {last} are in the call.", rest.join(", "));
    }
    if let [one] = call {
        return format!("{} is in the call.", one.name);
    }
    if !anyone_online {
        return "Nobody is online. Joining anyway is fine \u{2014} the others arrive in the call when they come back."
            .into();
    }
    "Nobody is in the call.".into()
}

fn voice_panel(snap: &AppSnapshot, narrow: bool, window: &mut Window, cx: &mut App) -> Div {
    let s = &snap.s;
    let joined = s.voice.joined;
    let others: Vec<&PeerState> = snap.call_peers().collect();
    // Screens only: the camera has its own button and line, so it must not turn "Share
    // screen" into "Stop sharing".
    let sharing: Vec<(u32, String)> = s
        .own_streams
        .iter()
        .filter(|st| !st.is_audio && !st.is_camera)
        .map(|st| (st.stream_id, st.title.clone()))
        .collect();
    let screens = || {
        s.own_streams
            .iter()
            .filter(|st| !st.is_audio && !st.is_camera)
    };
    let drawing = screens().any(|st| st.drawing);
    let has_drawing = screens().any(|st| st.has_drawing);

    let panel = div()
        .flex_none()
        .flex()
        .flex_col()
        .border_t_1()
        .border_color(theme::BORDER)
        .bg(c::INK);
    let panel = if joined || !others.is_empty() {
        let mut rows: Vec<AnyElement> = Vec::new();
        if joined {
            rows.push(voice_peer(snap, VoicePeer::Me, window, cx));
        }
        for p in &others {
            rows.push(voice_peer(snap, VoicePeer::Peer(p), window, cx));
        }
        // The head reports the worst link in the call, not a number of its own.
        let call_ids: Vec<String> = others.iter().map(|p| p.id.clone()).collect();
        let worst = meter_view("worst-rtt", &snap.meters, window, cx, move |m, _, _| {
            div()
                .child(super::format::fmt_rtt(
                    m.worst_rtt(call_ids.iter().map(String::as_str)),
                ))
                .into_any_element()
        });
        let live = div()
            .flex()
            .flex_col()
            .pt(px(10.))
            .px(px(10.))
            .pb(px(8.))
            .border_b_1()
            .border_color(theme::BORDER)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .mb(px(8.))
                    .text_size(px(11.))
                    .text_color(theme::ONLINE)
                    .child(Icon::Wave.size(13.))
                    .child(tracked(
                        if joined {
                            "VOICE CONNECTED"
                        } else {
                            "CALL RUNNING"
                        },
                        11.,
                        0.08,
                    ))
                    .child(
                        div()
                            .ml_auto()
                            .text_color(c::SILVER)
                            .font_family(theme::MONO_FONT)
                            .child(worst),
                    ),
            )
            .child(div().flex().flex_col().gap(px(2.)).children(rows))
            .when_some(sharing.first(), |d, (_, title)| {
                let more = if sharing.len() > 1 {
                    format!(" and {} more", sharing.len() - 1)
                } else {
                    String::new()
                };
                d.child(share_line(
                    Icon::Monitor,
                    "Sharing ",
                    title,
                    &format!("{more} \u{00b7} desktop audio follows automatically"),
                ))
                .child(draw_row(snap, &sharing, drawing, has_drawing))
            })
            .when_some(snap.own_camera(), |d, cam| {
                d.child(share_line(Icon::Cam, "Camera on \u{00b7} ", &cam.title, ""))
            })
            .when_some(snap.own_music(), |d, m| {
                d.child(share_line(
                    Icon::Music,
                    "Sharing ",
                    &m.title,
                    &format!(" \u{00b7} {} listening", m.viewers),
                ))
            })
            // Everyone in the call hears it without a click, so the line only says what.
            .children(others.iter().filter_map(|p| {
                p.music
                    .as_ref()
                    .map(|m| share_line(Icon::Music, &format!("{} plays ", p.name), &m.title, ""))
            }));
        let live = if joined {
            live.child(voice_acts(snap, &sharing))
        } else {
            live.child(join_button(snap, "btn-join", "Join the call"))
        };
        panel.child(live)
    } else {
        panel
            .child(join_button(snap, "btn-join", "Join voice"))
            // The hint is a `<p>`: its 12px top margin collapsed with the button's 10px.
            .child(
                div()
                    .mt(px(2.))
                    .mb(px(12.))
                    .px(px(12.))
                    .pb(px(10.))
                    .text_size(px(12.))
                    .text_color(c::SILVER)
                    .child(voice_hint(&others, snap.online_peers().next().is_some())),
            )
    };
    panel.child(self_row(snap, narrow, window, cx))
}

enum VoicePeer<'a> {
    Me,
    Peer(&'a PeerState),
}

/// `.voice-peer`: one person in the call. The whole row is a meter view: the frame and
/// the name's paper both follow the speaking level.
fn voice_peer(snap: &AppSnapshot, who: VoicePeer, window: &mut Window, cx: &mut App) -> AnyElement {
    let s = &snap.s;
    let (key, name, hue, muted) = match who {
        VoicePeer::Me => (
            None,
            format!("{} (you)", s.me.name),
            s.me.avatar,
            s.voice.muted,
        ),
        VoicePeer::Peer(p) => (
            Some(p.id.clone()),
            p.name.clone(),
            p.avatar,
            p.volume == 0.0,
        ),
    };
    let monogram_of = match who {
        VoicePeer::Me => s.me.name.clone(),
        VoicePeer::Peer(p) => p.name.clone(),
    };
    let id: ElementId =
        ElementId::Name(format!("voice-peer-{}", key.as_deref().unwrap_or("@self")).into());
    meter_view(id, &snap.meters, window, cx, move |m, _, _| {
        let speaking = match &key {
            None => m.self_speaking(),
            Some(id) => m.speaking(id),
        };
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .h(px(28.))
            .px(px(6.))
            .text_size(px(13.))
            .text_color(if speaking { theme::TEXT } else { c::SILVER })
            .child(avatar(&monogram_of, hue, 24., None, speaking))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(name.clone()),
            )
            .when(muted, |d| {
                d.child(div().text_color(theme::DND).child(Icon::MicOff.size(14.)))
            })
            .into_any_element()
    })
    .into_any_element()
}

/// `.voice-share`: an accent glyph and a wrapping line with one name in paper.
fn share_line(icon: Icon, before: &str, title: &str, after: &str) -> Div {
    let text = format!("{before}{title}{after}");
    let bold = before.len()..before.len() + title.len();
    div()
        .flex()
        .items_start()
        .gap(px(7.))
        .mt(px(9.))
        .text_size(px(11.5))
        .line_height(relative(1.45))
        .text_color(c::SILVER)
        .child(
            div()
                .flex_none()
                .mt(px(1.))
                .text_color(theme::ACCENT)
                .child(icon.size(13.)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(StyledText::new(text).with_highlights(vec![(
                    bold,
                    HighlightStyle {
                        color: Some(c::PAPER.into()),
                        ..Default::default()
                    },
                )])),
        )
}

/// Under "Sharing": whether viewers may draw on your screen, and wiping what they drew
/// (decision 45). One switch for every screen you share; blocking also wipes.
fn draw_row(snap: &AppSnapshot, sharing: &[(u32, String)], on: bool, has_lines: bool) -> Div {
    let streams: Vec<u32> = sharing.iter().map(|(id, _)| *id).collect();
    let (ui, ui_clear) = (snap.ui.clone(), snap.ui.clone());
    let clear_streams = streams.clone();
    let toggle = Btn::new(
        "btn-drawing",
        if on { "Block drawing" } else { "Allow drawing" },
    )
    .kind(BtnKind::Ghost)
    .icon(Icon::Edit)
    .full_width()
    .on_click(move |_, _, _| {
        for stream in &streams {
            commands::set_drawing(&ui, *stream, !on);
        }
    });
    let clear = Btn::new("btn-clear-drawing", "Clear")
        .kind(BtnKind::Ghost)
        .icon(Icon::Trash)
        .full_width()
        .disabled(!has_lines)
        .on_click(move |_, _, _| {
            for stream in &clear_streams {
                commands::clear_drawing(&ui_clear, *stream);
            }
        });
    div()
        .flex()
        .gap(px(6.))
        .mt(px(7.))
        .child(div().flex_1().min_w_0().child(toggle))
        .child(div().flex_1().min_w_0().child(clear))
}

/// `.join`: the one saturated fill in the resting window, magenta in a paper frame on the
/// button chassis. Hover flips a quarter of the cells to paper; pressed sinks by a pixel.
fn join_button(snap: &AppSnapshot, id: &'static str, label: &'static str) -> Stateful<Div> {
    let ui = snap.ui.clone();
    div()
        .id(id)
        .tab_index(0)
        .focus_visible(|s| s.shadow(focus_ring(c::INK, theme::chassis_shadow(3.))))
        .group(id)
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap(px(7.))
        .m(px(10.))
        .h(px(34.))
        .bg(theme::PRIMARY)
        .text_color(theme::ON_PRIMARY)
        .border_2()
        .border_color(c::PAPER)
        .shadow(theme::chassis_shadow(3.))
        .text_size(px(13.))
        .cursor_pointer()
        .active(|s| s.top(px(1.)).left(px(1.)).shadow(theme::chassis_shadow(2.)))
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .opacity(0.)
                .group_hover(id, |s| s.opacity(1.))
                .child(theme::dither(Dither::Quarter, c::PAPER, 2.)),
        )
        .child(div().relative().child(Icon::Wave.size(16.)))
        .child(div().relative().child(label))
        .on_click(move |_: &ClickEvent, _, _| commands::set_joined(&ui, true))
}

/// `.voice-acts`: Share screen / Stop sharing and Leave, side by side.
fn voice_acts(snap: &AppSnapshot, sharing: &[(u32, String)]) -> Div {
    let streams: Vec<u32> = sharing.iter().map(|(id, _)| *id).collect();
    let is_sharing = !streams.is_empty();
    let ui = snap.ui.clone();
    let share = div()
        .id("btn-share")
        .tab_index(0)
        .focus_visible(|s| s.shadow(focus_ring(c::INK, vec![])))
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .h(px(30.))
        .px(px(8.))
        .border_1()
        .border_color(theme::BORDER_STRONG)
        .text_color(theme::TEXT)
        .text_size(px(12.5))
        .whitespace_nowrap()
        .cursor_pointer()
        .hover(|s| {
            s.bg(c::PAPER)
                .border_color(c::PAPER)
                .text_color(theme::ON_INVERT)
        })
        .child(if is_sharing { Icon::X } else { Icon::Share }.size(14.))
        .child(if is_sharing {
            "Stop sharing"
        } else {
            "Share screen"
        })
        // Screens only: stopping "sharing" must not switch the camera off as a side effect.
        .on_click(snap.app.listener(move |app, _: &ClickEvent, window, cx| {
            if streams.is_empty() {
                app.open_picker(window, cx);
            } else {
                for stream in &streams {
                    commands::stop_sharing(app.ui(), *stream);
                }
            }
        }));
    let leave = div()
        .id("btn-leave")
        .tab_index(0)
        .focus_visible(|s| s.shadow(focus_ring(c::INK, vec![])))
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .gap(px(7.))
        .h(px(30.))
        .bg(c::SPACE)
        .text_color(theme::DANGER_TEXT)
        .border_1()
        .border_color(theme::DANGER_LINE)
        .text_size(px(13.))
        .whitespace_nowrap()
        .cursor_pointer()
        .hover(|s| s.bg(c::CORAL).text_color(theme::ON_SIGNAL))
        .child(Icon::Leave.size(16.))
        .child("Leave")
        .on_click(move |_: &ClickEvent, _, _| commands::set_joined(&ui, false));
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .mt(px(9.))
        .child(div().flex().gap(px(6.)).child(share).child(leave))
        .child(spotify_button(snap))
}

/// Share Spotify / Stop Spotify: only Spotify's sound, for everyone in the call.
fn spotify_button(snap: &AppSnapshot) -> Stateful<Div> {
    let on = snap.own_music().is_some();
    let ui = snap.ui.clone();
    div()
        .id("btn-spotify")
        .tab_index(0)
        .focus_visible(|s| s.shadow(focus_ring(c::INK, vec![])))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .h(px(30.))
        .px(px(8.))
        .border_1()
        .border_color(theme::BORDER_STRONG)
        .text_color(theme::TEXT)
        .text_size(px(12.5))
        .whitespace_nowrap()
        .cursor_pointer()
        .tooltip(tooltip(if on {
            "Stop sending Spotify to the call"
        } else {
            "Everyone in the call hears your Spotify, and only that"
        }))
        .hover(|s| {
            s.bg(c::PAPER)
                .border_color(c::PAPER)
                .text_color(theme::ON_INVERT)
        })
        .child(if on { Icon::X } else { Icon::Music }.size(14.))
        .child(if on { "Stop Spotify" } else { "Share Spotify" })
        .on_click(move |_: &ClickEvent, _, _| commands::set_spotify(&ui, !on))
}

/// `.self`: who you are, how you are, and the four toggles.
fn self_row(snap: &AppSnapshot, narrow: bool, window: &mut Window, cx: &mut App) -> Div {
    let s = &snap.s;
    let name = s.me.name.clone();
    let hue = s.me.avatar;
    let dot = self_dot(s);
    let ring = meter_view("self-avatar", &snap.meters, window, cx, move |m, _, _| {
        avatar(&name, hue, 32., Some(dot), m.self_speaking()).into_any_element()
    });
    let sub = if s.do_not_disturb {
        "Do not disturb"
    } else if s.voice.joined {
        "In the call"
    } else {
        "Online"
    };
    let muted = s.voice.muted;
    let deafened = s.voice.deafened;
    let dnd = s.do_not_disturb;
    let camera = snap.own_camera().is_some();
    let ui = snap.ui.clone();
    let btn = |id: &'static str, icon: Icon, pressed: bool, live: bool, title: &'static str| {
        div()
            .id(id)
            .tab_index(0)
            .focus_visible(|s| s.shadow(focus_ring(c::INK, vec![])))
            .w(px(if narrow { 24. } else { 26. }))
            .h(px(28.))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .text_color(c::SILVER)
            .tooltip(tooltip(title))
            // Pressed is "something is switched off" in rose; the camera opts out, because
            // a camera that is on is working, so it takes the live accent.
            .when(pressed, |d| {
                d.text_color(if live { theme::ACCENT } else { theme::DND })
                    .bg(c::SPACE)
                    .border_1()
                    .border_color(c::VIOLET)
            })
            .when(!pressed, |d| {
                d.hover(|s| s.bg(theme::BG_HOVER).text_color(theme::TEXT))
            })
            .child(icon.size(18.))
    };
    let (u1, u2, u3, u4) = (ui.clone(), ui.clone(), ui.clone(), ui);
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .p(px(8.))
        .when(narrow, |d| d.pr(px(6.)))
        .child(ring)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_size(px(13.))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(s.me.name.clone()),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(c::SILVER)
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(sub),
                ),
        )
        .child(
            div()
                .flex()
                .flex_none()
                .gap(px(2.))
                .child(
                    btn(
                        "btn-mic",
                        if muted { Icon::MicOff } else { Icon::Mic },
                        muted,
                        false,
                        if muted {
                            "Unmute microphone"
                        } else {
                            "Mute microphone"
                        },
                    )
                    .on_click(move |_: &ClickEvent, _, _| commands::set_muted(&u1, !muted)),
                )
                .child(
                    btn(
                        "btn-cam",
                        if camera { Icon::Cam } else { Icon::CamOff },
                        camera,
                        true,
                        if camera {
                            "Turn the camera off"
                        } else {
                            "Turn the camera on"
                        },
                    )
                    .on_click(move |_: &ClickEvent, _, _| commands::set_camera(&u2, !camera)),
                )
                .child(
                    btn(
                        "btn-deaf",
                        if deafened { Icon::HeadOff } else { Icon::Head },
                        deafened,
                        false,
                        if deafened { "Undeafen" } else { "Deafen" },
                    )
                    .on_click(move |_: &ClickEvent, _, _| commands::set_deafened(&u3, !deafened)),
                )
                .child(
                    btn(
                        "btn-dnd",
                        Icon::Moon,
                        dnd,
                        false,
                        if dnd {
                            "Turn off do not disturb"
                        } else {
                            "Do not disturb"
                        },
                    )
                    .on_click(move |_: &ClickEvent, _, _| commands::set_do_not_disturb(&u4, !dnd)),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::super::super::state::Presence;
    use super::*;

    fn peer(name: &str, presence: Presence, in_call: bool) -> PeerState {
        PeerState {
            id: format!("id-{name}"),
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
        }
    }

    /// The four answers of `voiceHint`; swapping the online check for `!call.is_empty()`
    /// breaks the third.
    #[test]
    fn the_voice_hint_reads_like_before() {
        let (s, j) = (
            peer("Sander", Presence::Online, true),
            peer("Joost", Presence::Online, true),
        );
        assert_eq!(
            voice_hint(&[&s, &j], true),
            "Sander and Joost are in the call."
        );
        assert_eq!(voice_hint(&[&s], true), "Sander is in the call.");
        assert_eq!(
            voice_hint(&[], false),
            "Nobody is online. Joining anyway is fine \u{2014} the others arrive in the call when they come back."
        );
        assert_eq!(voice_hint(&[], true), "Nobody is in the call.");
    }
}
