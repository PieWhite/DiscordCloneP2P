//! The channel column (`renderChannels`, `renderVoice`): the channel list with unread badges
//! and the collapsible general group, the DM list, and at the bottom the voice section (who
//! is in the call, Join), the voice-connected panel while joined (`call.rs`) and the self
//! panel (mic, deafen, do-not-disturb), stacked as in Discord.
//!
//! Right-click on a removable sub-channel opens the rename prompt straight away, as the old
//! `contextmenu` handler did; there is no menu in between.

use super::super::commands;
use super::super::icons::Icon;
use super::super::state::{ChannelState, PeerState, UiState};
use super::super::theme;
use super::widgets::{
    avatar, badge, focus_ring, icon_btn, live_pill, meter_view, pane_head, pane_title, selectable,
    tooltip, tracked, Btn, BtnKind, Dot,
};
use super::{AppHandle, AppSnapshot, Dialog, PaneInit, View};
use gpui_kit::{
    div, prelude::FluentBuilder, px, radians, AnyElement, App, ClickEvent, Context, Div, ElementId,
    FontWeight, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement,
    Render, SharedString, Stateful, StatefulInteractiveElement, Styled, TestSupportExt, Window,
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
            .bg(theme::BG_SIDEBAR)
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

/// The pane head: the serif title with the New channel button. The button stays in the
/// DM view too, as it did.
fn head(snap: &AppSnapshot, title: &str) -> Div {
    pane_head()
        .justify_between()
        .pl(px(18.))
        .pr(px(12.))
        .child(pane_title(title.to_string(), theme::DISPLAY_TEXT))
        .child(
            icon_btn("new-channel", Icon::Plus, false)
                .tab_index(0)
                .focus_visible(|s| s.shadow(focus_ring()))
                .tooltip(tooltip("New channel"))
                .on_click(snap.app.listener(|app, _: &ClickEvent, window, cx| {
                    app.open_dialog(Dialog::new_channel(), window, cx)
                })),
        )
}

/// The mono uppercase group label with its count pushed right.
pub(super) fn group_head(label: &str, count: usize, lead: Option<AnyElement>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(4.))
        .px(px(10.))
        .pb(px(8.))
        .font_family(theme::MONO_FONT)
        .text_size(theme::LABEL_TEXT)
        .text_color(theme::TEXT_MUTED)
        .children(lead)
        .child(tracked(&label.to_uppercase(), 11.5, 0.04))
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
        .focus_visible(|s| s.shadow(focus_ring()))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(|s| s.text_color(theme::TEXT_STRONG))
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
    // Direct messages live only behind the @ in the rail.
    groups(vec![general])
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
            .text_color(theme::TEXT_MUTED)
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

/// A 32px menu row. Hover lifts it; the current row is [`selectable`]'s selected look;
/// unread lifts the name to white and puts a small mark on the left edge.
fn menu_row(id: ElementId, current: bool, unread: usize) -> Stateful<Div> {
    selectable(
        div()
            .id(id)
            .tab_index(0)
            .focus_visible(|s| s.shadow(focus_ring()))
            .relative()
            .w_full()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(10.))
            .h(px(32.))
            .px(px(10.))
            .cursor_pointer()
            .text_color(if unread > 0 {
                theme::TEXT_STRONG
            } else {
                theme::TEXT_MUTED
            }),
        current,
    )
    .when(!current, |d| {
        d.hover(|s| s.bg(theme::BG_HOVER).text_color(theme::TEXT_STRONG))
    })
    .when(unread > 0, |d| {
        d.child(
            div()
                .absolute()
                .left(px(-8.))
                .top(px(11.))
                .w(px(4.))
                .h(px(8.))
                .rounded_full()
                .bg(theme::TEXT_STRONG),
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

fn channel_row(snap: &AppSnapshot, ch: &ChannelState) -> AnyElement {
    let current = snap.v.view == View::Channels && snap.v.channel == ch.key;
    let group = SharedString::from(format!("chan-{}", ch.key));
    let key = ch.key.clone();
    menu_row(ElementId::Name(group), current, ch.unread)
        .child(
            div()
                .text_color(theme::TEXT_DIM)
                .child(Icon::Hash.size(16.)),
        )
        .child(row_name(&ch.name))
        .when(ch.unread > 0, |d| d.child(badge(ch.unread, false)))
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
    menu_row(ElementId::Name(group), current, p.unread)
        .child(avatar(&p.name, p.avatar, 20., Some(Dot::of_peer(p)), false))
        .child(row_name(&p.name))
        .when(p.unread > 0, |d| d.child(badge(p.unread, false)))
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

/// The voice section: who is in the call, Join while you are not, and the call controls
/// in the voice-connected panel while you are (decision 48).
fn voice_panel(snap: &AppSnapshot, narrow: bool, window: &mut Window, cx: &mut App) -> Div {
    let s = &snap.s;
    let joined = s.voice.joined;
    let others: Vec<&PeerState> = snap.call_peers().collect();
    let mut rows: Vec<AnyElement> = Vec::new();
    if joined {
        rows.push(voice_peer(snap, VoicePeer::Me, window, cx));
    }
    for p in &others {
        rows.push(voice_peer(snap, VoicePeer::Peer(p), window, cx));
    }
    let count = rows.len();
    let join = |label: &'static str| {
        let ui = snap.ui.clone();
        div().px(px(10.)).pt(px(4.)).child(
            Btn::new("btn-join", label)
                .kind(BtnKind::Primary)
                .icon(Icon::Wave)
                .full_width()
                .on_click(move |_, _, _| commands::set_joined(&ui, true)),
        )
    };
    div()
        .flex_none()
        .flex()
        .flex_col()
        .pt(px(12.))
        .border_t_1()
        .border_color(theme::BORDER)
        .child(group_head("Voice", count, None))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .px(px(8.))
                .children(rows),
        )
        .when(!joined && !others.is_empty(), |d| {
            d.child(join("Join the call"))
        })
        .when(!joined && others.is_empty(), |d| {
            d.child(join("Join voice")).child(
                div()
                    .mt(px(8.))
                    .px(px(12.))
                    .text_size(px(12.))
                    .text_color(theme::TEXT_MUTED)
                    .child(voice_hint(&others, snap.online_peers().next().is_some())),
            )
        })
        .when(joined, |d| {
            d.child(super::call::render_panel(snap, window, cx))
        })
        .child(self_row(snap, narrow, window, cx))
}

enum VoicePeer<'a> {
    Me,
    Peer(&'a PeerState),
}

/// One person in the call: avatar with the speaking ring, name, LIVE while they show a
/// screen or camera, the mic-off glyph when muted. Hovering a peer's LIVE turns it into
/// Watch for a stream you are not watching yet (screens before cameras), as in Discord. The
/// whole row is a meter view: the ring and the name's white both follow the speaking level.
fn voice_peer(snap: &AppSnapshot, who: VoicePeer, window: &mut Window, cx: &mut App) -> AnyElement {
    let s = &snap.s;
    let (key, name, hue, muted, live, watch) = match who {
        VoicePeer::Me => (
            None,
            s.me.name.clone(),
            s.me.avatar,
            s.voice.muted,
            s.own_streams.iter().any(|st| !st.is_audio),
            None,
        ),
        VoicePeer::Peer(p) => (
            Some(p.id.clone()),
            p.name.clone(),
            p.avatar,
            p.volume == 0.0,
            s.streams.iter().any(|st| st.owner == p.id && !st.is_audio),
            s.streams
                .iter()
                .filter(|st| st.owner == p.id && !st.is_audio && !st.watching)
                .min_by_key(|st| st.is_camera)
                .map(|st| st.stream_id),
        ),
    };
    let group: SharedString = format!("voice-row-{}", key.as_deref().unwrap_or("@self")).into();
    let ui = snap.ui.clone();
    let id: ElementId =
        ElementId::Name(format!("voice-peer-{}", key.as_deref().unwrap_or("@self")).into());
    meter_view(id, &snap.meters, window, cx, move |m, _, _| {
        let speaking = match &key {
            None => m.self_speaking(),
            Some(id) => m.speaking(id),
        };
        div()
            .id(ElementId::Name(group.clone()))
            .flex()
            .items_center()
            .gap(px(10.))
            .h(px(32.))
            .px(px(10.))
            .text_size(px(14.))
            .text_color(if speaking {
                theme::TEXT_STRONG
            } else {
                theme::TEXT
            })
            .group(group.clone())
            .child(avatar(&name, hue, 22., None, speaking))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(name.clone()),
            )
            .when(live, |d| match (&key, watch) {
                (Some(owner), Some(stream)) => d.child(
                    div()
                        .relative()
                        .flex_none()
                        .child(live_pill().group_hover(group.clone(), |s| s.invisible()))
                        .child(watch_btn(&ui, owner, stream, &group)),
                ),
                _ => d.child(live_pill()),
            })
            .when(muted, |d| {
                d.child(
                    div()
                        .text_color(theme::TEXT_DIM)
                        .child(Icon::MicOff.size(14.)),
                )
            })
            .test_support()
            .into_any_element()
    })
    .into_any_element()
}

/// Watch, laid over the LIVE pill and shown only while the row is hovered. Opens the
/// stream the way the tile's Watch does.
fn watch_btn(
    ui: &std::sync::Arc<super::super::Ui>,
    owner: &str,
    stream: u32,
    group: &SharedString,
) -> impl IntoElement {
    let (ui, owner) = (ui.clone(), owner.to_string());
    div()
        .id(ElementId::Name(format!("watch-{owner}-{stream}").into()))
        .absolute()
        .top_0()
        .right_0()
        .h_full()
        .flex()
        .items_center()
        .px(px(6.))
        .bg(theme::c::VOID)
        .rounded(px(4.))
        .border_1()
        .border_color(theme::BORDER_HOVER)
        .font_family(theme::MONO_FONT)
        .text_size(px(10.5))
        .text_color(theme::TEXT_STRONG)
        .whitespace_nowrap()
        .cursor_pointer()
        .invisible()
        .group_hover(group.clone(), |s| s.visible())
        .hover(|s| s.border_color(theme::TEXT_STRONG))
        .tooltip(tooltip("Watch stream"))
        .child("WATCH")
        .on_click(move |_: &ClickEvent, _, _| commands::set_watching(&ui, &owner, stream, true))
        .test_support()
}

/// The self panel: who you are, how you are, and the mic, deafen and do-not-disturb toggles,
/// in a hairline field. Mic and deafen live here whether or not you are in a call.
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
        "In call"
    } else {
        "Online"
    };
    let muted = s.voice.muted;
    let deafened = s.voice.deafened;
    let dnd = s.do_not_disturb;
    let ui = snap.ui.clone();
    let btn = |id: &'static str, icon: Icon, pressed: bool, title: &'static str| {
        div()
            .id(id)
            .tab_index(0)
            .focus_visible(|s| s.shadow(focus_ring()))
            .w(px(if narrow { 24. } else { 26. }))
            .h(px(28.))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(theme::RADIUS_CONTROL)
            .border_1()
            .border_color(gpui_kit::transparent_black())
            .cursor_pointer()
            .text_color(theme::TEXT_MUTED)
            .tooltip(tooltip(title))
            // Pressed is "something is switched off", in red.
            .when(pressed, |d| {
                d.text_color(theme::DANGER_TEXT)
                    .bg(theme::DANGER_BG)
                    .border_color(theme::DANGER_LINE)
            })
            .when(!pressed, |d| {
                d.hover(|s| s.bg(theme::BG_HOVER).text_color(theme::TEXT_STRONG))
            })
            .child(icon.size(18.))
    };
    let (u1, u3, u4) = (ui.clone(), ui.clone(), ui);
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .m(px(8.))
        .p(px(8.))
        .when(narrow, |d| d.pr(px(6.)))
        .rounded(theme::RADIUS_FIELD)
        .border_1()
        .border_color(theme::BORDER)
        .child(ring)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme::TEXT_STRONG)
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(s.me.name.clone()),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(theme::TEXT_MUTED)
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
                        "btn-deaf",
                        if deafened { Icon::HeadOff } else { Icon::Head },
                        deafened,
                        if deafened { "Undeafen" } else { "Deafen" },
                    )
                    .on_click(move |_: &ClickEvent, _, _| commands::set_deafened(&u3, !deafened)),
                )
                .child(
                    btn(
                        "btn-dnd",
                        Icon::Moon,
                        dnd,
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
pub(super) mod tests {
    use super::super::super::state::Presence;
    use super::*;

    pub(in crate::ui::view) fn peer(name: &str, presence: Presence, in_call: bool) -> PeerState {
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
            slot: 0,
        }
    }

    /// Hovering a streamer in the voice list offers Watch for the stream not watched yet,
    /// screen before camera. Revert to see it fail: drop `!st.watching` from the `watch`
    /// filter (then it offers the screen that is already open).
    #[gpui_kit::test]
    fn a_streamer_in_the_call_offers_watch(cx: &mut gpui_kit::TestAppContext) {
        use super::super::super::state::StreamState;
        use gpui_kit::test::TestWindowExt;
        let mut h = super::super::composer::window_tests::Harness::new(cx);
        let id = "6f1c2c1e-3b1a-4c55-9a43-0d7c5e0f2a11";
        let stream = |stream_id, watching, is_camera| StreamState {
            owner: id.into(),
            owner_name: "Sam".into(),
            stream_id,
            title: "Screen".into(),
            width: 1920,
            height: 1080,
            watching,
            is_audio: false,
            is_camera,
            volume: 1.0,
        };
        h.app.update(cx, |app, cx| {
            let mut s = (*app.s).clone();
            s.voice.joined = true;
            s.peers = vec![PeerState {
                id: id.into(),
                ..peer("Sam", Presence::Online, true)
            }];
            s.streams = vec![
                stream(3, false, true),
                stream(4, true, false),
                stream(5, false, false),
            ];
            app.s = std::sync::Arc::new(s);
            cx.notify();
        });
        cx.run_until_parked();
        let watch = format!("watch-{id}-5");
        let visible = |cx: &mut gpui_kit::TestAppContext| {
            h.with_window(cx, |window, cx| {
                window.render_frame(cx);
                window.find(watch.clone()).visible()
            })
        };
        assert!(!visible(cx), "only on hover");
        h.with_window(cx, |window, cx| window.hover(format!("voice-row-{id}"), cx));
        assert!(visible(cx));
        h.with_window(cx, |window, cx| window.click(watch.clone(), cx));
        cx.run_until_parked();
        assert!(matches!(
            h.drain().as_slice(),
            [crate::engine::UiCommand::Kijken(_, 5)]
        ));
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
