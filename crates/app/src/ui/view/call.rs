//! The voice-connected panel at the foot of the channel column, above the self panel, drawn
//! only while joined (decision 48, moved there as in Discord on 2026-10-20): the call's size
//! and worst link with Leave beside it, then camera, screen share and Spotify in one row, and
//! under that one quiet line per screen you share (with its own stop) and per shared Spotify. These are the only homes of those controls;
//! the defaults for drawing and remote control are in Settings → Video, and each screen line
//! has its own two switches for them (decision 54).
//!
//! [`control_banner`] is the one exception to "only while joined": a request to control your
//! screen, or the fact that someone does, shows under the title bar in every view.

use super::super::commands::{self, MusicAction};
use super::super::icons::Icon;
use super::super::state::{OwnStreamState, PeerState};
use super::super::theme;
use super::widgets::{dk_btn, meter_view, selectable, tooltip, Btn, BtnKind};
use super::{format, AppSnapshot};
use gpui_kit::{
    div, prelude::FluentBuilder, px, AnyElement, App, ClickEvent, Div, ElementId, FontWeight,
    HighlightStyle, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement,
    Styled, StyledText, TestSupportExt, Window,
};

/// The voice-connected panel. Call only while `s.voice.joined`.
pub fn render_panel(snap: &AppSnapshot, window: &mut Window, cx: &mut App) -> AnyElement {
    let others: Vec<&PeerState> = snap.call_peers().collect();
    let people = others.len() + 1;
    let call_ids: Vec<String> = others.iter().map(|p| p.id.clone()).collect();
    let worst = meter_view("worst-rtt", &snap.meters, window, cx, move |m, _, _| {
        div()
            .child(format::fmt_rtt(
                m.worst_rtt(call_ids.iter().map(String::as_str)),
            ))
            .into_any_element()
    });
    let summary = div()
        .flex()
        .items_center()
        .gap(px(4.))
        .font_family(theme::MONO_FONT)
        .text_size(px(11.5))
        .text_color(theme::TEXT_MUTED)
        .whitespace_nowrap()
        .child(format!(
            "{people} {} \u{00b7}",
            if people == 1 { "person" } else { "people" }
        ))
        .child(worst);
    let u_leave = snap.ui.clone();
    let head = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(
            div()
                .flex_none()
                .text_color(theme::ONLINE)
                .child(Icon::Signal.size(18.)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme::ONLINE)
                        .truncate()
                        .child("Voice connected"),
                )
                .child(summary),
        )
        .child(
            div()
                .id("call-leave")
                .tab_index(0)
                .focus_visible(|s| s.shadow(super::widgets::focus_ring()))
                .size(px(32.))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .rounded(theme::RADIUS_CONTROL)
                .cursor_pointer()
                .text_color(theme::DANGER_TEXT)
                .hover(|s| s.bg(theme::DANGER_BG))
                .tooltip(tooltip("Leave the call"))
                .child(Icon::Leave.size(18.))
                .on_click(move |_: &ClickEvent, _, _| commands::set_joined(&u_leave, false)),
        );

    let mut lines: Vec<AnyElement> = Vec::new();
    // Each shared screen with its own stop, as in Discord, besides the toggle above.
    for st in snap
        .s
        .own_streams
        .iter()
        .filter(|st| !st.is_audio && !st.is_camera)
    {
        let (ui, stream) = (snap.ui.clone(), st.stream_id);
        lines.push(
            line(
                Icon::Monitor,
                "You share ",
                &st.title,
                &format!(" \u{00b7} {} watching", st.viewers),
            )
            .child(option_btn(
                format!("draw-{stream}"),
                Icon::Edit,
                st.drawing,
                if st.drawing {
                    "Viewers may draw on this screen"
                } else {
                    "Let viewers draw on this screen"
                },
                {
                    let (ui, on) = (snap.ui.clone(), st.drawing);
                    move |_: &ClickEvent, _: &mut Window, _: &mut App| {
                        commands::set_stream_drawing(&ui, stream, !on)
                    }
                },
            ))
            .when(st.controllable, |d| {
                let (ui, on) = (snap.ui.clone(), st.remote_control);
                d.child(option_btn(
                    format!("control-{stream}"),
                    Icon::Pointer,
                    on,
                    if on {
                        "Viewers may ask to control this screen"
                    } else {
                        "Let viewers ask to control this screen"
                    },
                    move |_: &ClickEvent, _: &mut Window, _: &mut App| {
                        commands::set_stream_remote_control(&ui, stream, !on)
                    },
                ))
            })
            .child(
                div()
                    .id(ElementId::Name(format!("stop-share-{stream}").into()))
                    .tab_index(0)
                    .focus_visible(|s| s.shadow(super::widgets::focus_ring()))
                    .size(px(22.))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .rounded(theme::RADIUS_CONTROL)
                    .cursor_pointer()
                    .text_color(theme::DANGER_TEXT)
                    .hover(|s| s.bg(theme::DANGER_BG))
                    .tooltip(tooltip("Stop streaming"))
                    .child(Icon::X.size(13.))
                    .on_click(move |_: &ClickEvent, _, _| commands::stop_sharing(&ui, stream))
                    .test_support(),
            )
            .into_any_element(),
        );
    }
    if let Some(own) = snap.own_music() {
        lines.push(
            line(
                Icon::Music,
                "You share ",
                &own.title,
                &format!(" \u{00b7} {} listening", own.viewers),
            )
            .into_any_element(),
        );
    }
    // Everyone in the call hears it without a click, so the line only says what plays.
    for p in &others {
        let Some(m) = &p.music else { continue };
        lines.push(
            line(Icon::Music, &format!("{} plays ", p.name), &m.title, "")
                // Anyone in the call may pause or skip, at once; the DJ checks that and does
                // it in their own Spotify.
                .when(m.listening, |d| {
                    d.child(music_btn(
                        snap,
                        p,
                        m.stream,
                        MusicAction::PLAY_PAUSE,
                        Icon::PlayPause,
                        "Play / pause",
                    ))
                    .child(music_btn(
                        snap,
                        p,
                        m.stream,
                        MusicAction::NEXT,
                        Icon::Next,
                        "Next track",
                    ))
                })
                .into_any_element(),
        );
    }

    div()
        .id("call-bar")
        .flex()
        .flex_col()
        .gap(px(8.))
        .mx(px(8.))
        .mt(px(8.))
        .p(px(8.))
        .rounded(theme::RADIUS_FIELD)
        .border_1()
        .border_color(theme::BORDER)
        .child(head)
        .child(toolbar(snap))
        .when_some(game_btn(snap), |d, b| d.child(b))
        .when(!lines.is_empty(), |d| {
            d.child(div().flex().flex_col().children(lines))
        })
        .test_support()
        .into_any_element()
}

/// Camera, screen share and Spotify, in equal widths. Share and Spotify show their crossed
/// glyph while on: the next click stops. Drawing lives in Settings → Video.
fn toolbar(snap: &AppSnapshot) -> Div {
    let camera = snap.own_camera().is_some();
    let spotify = snap.own_music().is_some();
    // Screens only: the camera has its own button, so it must not count as "sharing".
    let screens: Vec<u32> = snap
        .s
        .own_streams
        .iter()
        .filter(|st| !st.is_audio && !st.is_camera)
        .map(|st| st.stream_id)
        .collect();
    let (u_cam, u_spot) = (snap.ui.clone(), snap.ui.clone());

    let share = if screens.is_empty() {
        dk_btn("call-share", Icon::Monitor, false)
            .tooltip(tooltip("Share screen"))
            .on_click(
                snap.app
                    .listener(|app, _: &ClickEvent, window, cx| app.open_picker(window, cx)),
            )
    } else {
        let ui = snap.ui.clone();
        dk_btn("call-share", Icon::MonitorOff, true)
            .tooltip(tooltip("Stop sharing"))
            .on_click(move |_: &ClickEvent, _, _| {
                for stream in &screens {
                    commands::stop_sharing(&ui, *stream);
                }
            })
    };

    let cell = |el: gpui_kit::Stateful<Div>| {
        div()
            .flex_1()
            .min_w_0()
            .child(el.w_full().h(px(36.)).test_support())
    };
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .child(cell(
            dk_btn(
                "call-cam",
                if camera { Icon::Cam } else { Icon::CamOff },
                camera,
            )
            .tooltip(tooltip(if camera {
                "Turn the camera off"
            } else {
                "Turn the camera on"
            }))
            .on_click(move |_: &ClickEvent, _, _| commands::set_camera(&u_cam, !camera)),
        ))
        .child(cell(share))
        .child(cell(
            dk_btn(
                "call-spotify",
                if spotify { Icon::MusicOff } else { Icon::Music },
                spotify,
            )
            .tooltip(tooltip(if spotify {
                "Stop sending Spotify to the call"
            } else {
                "Everyone in the call hears your Spotify, and only that"
            }))
            .on_click(move |_: &ClickEvent, _, _| commands::set_spotify(&u_spot, !spotify)),
        ))
}

/// "Stream <game>" while a game is in front and no screen is shared yet: one click shares
/// that game's window, without the picker.
fn game_btn(snap: &AppSnapshot) -> Option<impl IntoElement> {
    let game = snap.s.me.playing.as_ref()?;
    if snap.sharing_self() {
        return None;
    }
    let ui = snap.ui.clone();
    Some(
        div()
            .id("stream-game")
            .child(
                Btn::new("stream-game-btn", format!("Stream {game}"))
                    .icon(Icon::Monitor)
                    .full_width()
                    .on_click(move |_, _, _| commands::share_game(&ui)),
            )
            .test_support(),
    )
}

/// One muted line with the title in white.
fn line(icon: Icon, before: &str, title: &str, after: &str) -> Div {
    let text = format!("{before}{title}{after}");
    let bold = before.len()..before.len() + title.len();
    div()
        .flex()
        .items_center()
        .gap(px(7.))
        .min_h(px(22.))
        .text_size(px(12.))
        .text_color(theme::TEXT_MUTED)
        .child(
            div()
                .flex_none()
                .text_color(theme::TEXT_DIM)
                .child(icon.size(13.)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .child(StyledText::new(text).with_highlights(vec![(
                    bold,
                    HighlightStyle {
                        color: Some(theme::TEXT_STRONG.into()),
                        ..Default::default()
                    },
                )])),
        )
}

/// A 22px switch for one option of one shared screen: the selected look while on.
fn option_btn(
    id: String,
    icon: Icon,
    on: bool,
    tip: &'static str,
    click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    selectable(
        div()
            .id(ElementId::Name(id.into()))
            .tab_index(0)
            .focus_visible(|s| s.shadow(super::widgets::focus_ring()))
            .size(px(22.))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(theme::RADIUS_CONTROL)
            .cursor_pointer()
            .text_color(theme::TEXT_DIM)
            .hover(|st| st.bg(theme::BG_HOVER).text_color(theme::TEXT_STRONG)),
        on,
    )
    .tooltip(tooltip(tip))
    .child(icon.size(13.))
    .on_click(click)
    .test_support()
}

/// Who asks to control a screen of yours, or controls it now (decision 54). Drawn under the
/// title bar in every view and also outside a call: this is someone's hands on your PC, and
/// the person sharing may be in a game with only a glance for the app. `None` when nobody does.
pub fn control_banner(snap: &AppSnapshot) -> Option<AnyElement> {
    let rows: Vec<AnyElement> = snap
        .s
        .own_streams
        .iter()
        .filter_map(|st| control_row(snap, st))
        .collect();
    (!rows.is_empty()).then(|| {
        div()
            .id("control-banner")
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(6.))
            .px(px(16.))
            .py(px(8.))
            .bg(theme::BG_SELECTED)
            .border_b_1()
            .border_color(theme::BORDER)
            .children(rows)
            .test_support()
            .into_any_element()
    })
}

/// One line of the banner: the controller with Stop, or the request with Deny and Allow.
fn control_row(snap: &AppSnapshot, st: &OwnStreamState) -> Option<AnyElement> {
    let stream = st.stream_id;
    let ui = snap.ui.clone();
    let text = |name: &str, what: &str| {
        let full = format!("{name}{what}{}", st.title);
        let bold = 0..name.len();
        StyledText::new(full).with_highlights(vec![(
            bold,
            HighlightStyle {
                color: Some(theme::TEXT_STRONG.into()),
                font_weight: Some(FontWeight::MEDIUM),
                ..Default::default()
            },
        )])
    };
    let (message, buttons): (StyledText, Vec<AnyElement>) = if let Some(c) = &st.controller {
        (
            text(
                &c.name,
                " controls your mouse, keyboard and clipboard text on ",
            ),
            vec![banner_btn(
                format!("control-stop-{stream}"),
                "Stop control",
                BtnKind::Danger,
                move |_, _, _| commands::stop_control(&ui, stream),
            )],
        )
    } else if let Some(r) = &st.control_request {
        let (ui_deny, peer) = (ui.clone(), r.id.clone());
        (
            text(
                &r.name,
                " asks to control your mouse, keyboard and clipboard text on ",
            ),
            vec![
                banner_btn(
                    format!("control-deny-{stream}"),
                    "Deny",
                    BtnKind::Ghost,
                    move |_, _, _| commands::deny_control(&ui_deny, stream),
                ),
                banner_btn(
                    format!("control-allow-{stream}"),
                    "Allow",
                    BtnKind::Primary,
                    move |_, _, _| commands::allow_control(&ui, stream, &peer),
                ),
            ],
        )
    } else {
        return None;
    };
    Some(
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .text_size(px(13.))
            .text_color(theme::TEXT)
            .child(
                div()
                    .flex_none()
                    .text_color(theme::TEXT_STRONG)
                    .child(Icon::Pointer.size(16.)),
            )
            .child(div().flex_1().min_w_0().truncate().child(message))
            .children(buttons)
            .into_any_element(),
    )
}

/// A button on the control banner, under an id the headless tests can click.
fn banner_btn(
    id: String,
    label: &'static str,
    kind: BtnKind,
    click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(ElementId::Name(id.clone().into()))
        .flex_none()
        .child(
            Btn::new(ElementId::Name(format!("{id}-btn").into()), label)
                .kind(kind)
                .on_click(click),
        )
        .test_support()
        .into_any_element()
}

/// A listener's 22px glyph button on someone's Spotify.
fn music_btn(
    snap: &AppSnapshot,
    p: &PeerState,
    stream: u32,
    action: MusicAction,
    icon: Icon,
    label: &'static str,
) -> gpui_kit::Stateful<Div> {
    let (ui, dj) = (snap.ui.clone(), p.id.clone());
    div()
        .id(ElementId::Name(
            format!("music-{}-{}-{:?}", p.id, stream, action).into(),
        ))
        .tab_index(0)
        .focus_visible(|s| s.shadow(super::widgets::focus_ring()))
        .size(px(22.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(theme::RADIUS_CONTROL)
        .cursor_pointer()
        .text_color(theme::TEXT_MUTED)
        .hover(|st| st.bg(theme::BG_HOVER).text_color(theme::TEXT_STRONG))
        .tooltip(tooltip(label))
        .child(icon.size(13.))
        .on_click(move |_: &ClickEvent, _, _| commands::music_control(&ui, &dj, stream, action))
}

#[cfg(test)]
mod tests {
    use super::super::super::state::OwnStreamState;
    use super::super::composer::window_tests::Harness;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::TestAppContext;
    use std::sync::Arc;

    /// Revert to see it fail: drop the `.when(joined, ..)` around `call::render_panel` in
    /// `channels::voice_panel` (the panel then shows before joining).
    #[gpui_kit::test]
    fn the_call_panel_shows_only_while_joined(cx: &mut TestAppContext) {
        let h = Harness::new(cx);
        let present = |cx: &mut TestAppContext| {
            h.with_window(cx, |window, cx| {
                window.render_frame(cx);
                window.try_find("call-bar").is_some()
            })
        };
        assert!(!present(cx), "no call panel before joining");
        h.app.update(cx, |app, cx| {
            let mut s = (*app.s).clone();
            s.voice.joined = true;
            app.s = Arc::new(s);
            cx.notify();
        });
        cx.run_until_parked();
        assert!(present(cx), "the call panel once joined");
    }

    /// While a screen is shared, one click on the share button stops it; no menu.
    /// Revert to see it fail: put back the `Popover` around the button while sharing.
    #[gpui_kit::test]
    fn the_share_button_stops_sharing_on_click(cx: &mut TestAppContext) {
        let mut h = Harness::new(cx);
        h.app.update(cx, |app, cx| {
            let mut s = (*app.s).clone();
            s.voice.joined = true;
            s.own_streams = vec![OwnStreamState {
                stream_id: 6,
                title: "Screen".into(),
                viewers: 0,
                is_audio: false,
                is_camera: false,
                is_music: false,
                drawing: false,
                has_drawing: false,
                remote_control: false,
                controllable: false,
                control_request: None,
                controller: None,
            }];
            app.s = Arc::new(s);
            cx.notify();
        });
        cx.run_until_parked();
        h.with_window(cx, |window, cx| {
            window.render_frame(cx);
            window.click("call-share", cx)
        });
        cx.run_until_parked();
        assert!(matches!(
            h.drain().as_slice(),
            [crate::engine::UiCommand::StopDelen(6)]
        ));
    }

    /// Each shared screen has its own stop on its line, besides the toggle. Revert to see it
    /// fail: drop the `.child(..)` with `stop-share-` from the screen lines.
    #[gpui_kit::test]
    fn a_shared_screen_has_its_own_stop(cx: &mut TestAppContext) {
        let mut h = Harness::new(cx);
        h.app.update(cx, |app, cx| {
            let mut s = (*app.s).clone();
            s.voice.joined = true;
            s.own_streams = vec![OwnStreamState {
                stream_id: 6,
                title: "Elden Ring".into(),
                viewers: 2,
                is_audio: false,
                is_camera: false,
                is_music: false,
                drawing: false,
                has_drawing: false,
                remote_control: false,
                controllable: false,
                control_request: None,
                controller: None,
            }];
            app.s = Arc::new(s);
            cx.notify();
        });
        cx.run_until_parked();
        h.with_window(cx, |window, cx| {
            window.render_frame(cx);
            window.click("stop-share-6", cx)
        });
        cx.run_until_parked();
        assert!(matches!(
            h.drain().as_slice(),
            [crate::engine::UiCommand::StopDelen(6)]
        ));
    }

    /// In a game and not sharing: one click streams it, no picker; while sharing the
    /// button is gone. Revert to see it fail: drop the `sharing_self` check in `game_btn`.
    #[gpui_kit::test]
    fn the_game_in_front_streams_with_one_click(cx: &mut TestAppContext) {
        let mut h = Harness::new(cx);
        let set = |sharing: bool, h: &Harness, cx: &mut TestAppContext| {
            h.app.update(cx, |app, cx| {
                let mut s = (*app.s).clone();
                s.voice.joined = true;
                s.me.playing = Some("Elden Ring".into());
                s.own_streams = sharing
                    .then(|| OwnStreamState {
                        stream_id: 6,
                        title: "Elden Ring".into(),
                        viewers: 0,
                        is_audio: false,
                        is_camera: false,
                        is_music: false,
                        drawing: false,
                        has_drawing: false,
                        remote_control: false,
                        controllable: false,
                        control_request: None,
                        controller: None,
                    })
                    .into_iter()
                    .collect();
                app.s = Arc::new(s);
                cx.notify();
            });
            cx.run_until_parked();
        };
        set(false, &h, cx);
        h.with_window(cx, |window, cx| {
            window.render_frame(cx);
            window.click("stream-game", cx)
        });
        cx.run_until_parked();
        assert!(matches!(
            h.drain().as_slice(),
            [crate::engine::UiCommand::DeelSpel]
        ));
        set(true, &h, cx);
        let shown = h.with_window(cx, |window, cx| {
            window.render_frame(cx);
            window.try_find("stream-game").is_some()
        });
        assert!(!shown, "not while a screen is shared");
    }

    // -- remote control (decision 54) ----------------------------------------

    use super::super::super::state::ControlPeer;
    use super::super::composer::window_tests::SANDER;
    use crate::engine::UiCommand;

    fn own_monitor() -> OwnStreamState {
        OwnStreamState {
            stream_id: 6,
            title: "Display 1".into(),
            viewers: 1,
            is_audio: false,
            is_camera: false,
            is_music: false,
            drawing: false,
            has_drawing: false,
            remote_control: true,
            controllable: true,
            control_request: None,
            controller: None,
        }
    }

    fn sander() -> Option<ControlPeer> {
        Some(ControlPeer {
            id: SANDER.into(),
            name: "Sander".into(),
        })
    }

    fn set_own(h: &Harness, cx: &mut TestAppContext, joined: bool, st: OwnStreamState) {
        h.app.update(cx, |app, cx| {
            let mut s = (*app.s).clone();
            s.voice.joined = joined;
            s.own_streams = vec![st];
            app.s = Arc::new(s);
            cx.notify();
        });
        cx.run_until_parked();
    }

    /// A request to control your screen shows above everything, also outside a call, and
    /// Allow names the one who asked. Revert to see it fail: drop the `control_banner` child
    /// from the root in `view/mod.rs`.
    #[gpui_kit::test]
    fn a_control_request_shows_outside_a_call_and_allow_names_who_asked(cx: &mut TestAppContext) {
        let mut h = Harness::new(cx);
        let banner = |h: &Harness, cx: &mut TestAppContext| {
            h.with_window(cx, |window, cx| {
                window.render_frame(cx);
                window.try_find("control-banner").is_some()
            })
        };
        set_own(&h, cx, false, own_monitor());
        assert!(!banner(&h, cx), "nobody asks: no banner");
        set_own(
            &h,
            cx,
            false,
            OwnStreamState {
                control_request: sander(),
                ..own_monitor()
            },
        );
        assert!(banner(&h, cx), "not in a call, still shown");
        h.with_window(cx, |window, cx| {
            window.render_frame(cx);
            window.click("control-allow-6", cx)
        });
        cx.run_until_parked();
        match h.drain().as_slice() {
            [UiCommand::StaBedieningToe(6, peer)] => assert_eq!(peer.to_string(), SANDER),
            other => panic!("expected Allow for Sander: {other:?}"),
        }
        h.with_window(cx, |window, cx| {
            window.render_frame(cx);
            window.click("control-deny-6", cx)
        });
        cx.run_until_parked();
        assert!(matches!(
            h.drain().as_slice(),
            [UiCommand::WeigerBediening(6)]
        ));
    }

    /// While someone controls your screen the banner says who, with one click to stop it.
    #[gpui_kit::test]
    fn the_controller_is_named_and_can_be_stopped(cx: &mut TestAppContext) {
        let mut h = Harness::new(cx);
        set_own(
            &h,
            cx,
            false,
            OwnStreamState {
                controller: sander(),
                ..own_monitor()
            },
        );
        h.with_window(cx, |window, cx| {
            window.render_frame(cx);
            window.click("control-stop-6", cx)
        });
        cx.run_until_parked();
        assert!(matches!(
            h.drain().as_slice(),
            [UiCommand::StopBediening(6)]
        ));
    }

    /// Each shared monitor has its own switches for drawing and remote control; a window has
    /// no control switch. Revert to see it fail: drop the `.when(st.controllable, ..)` child.
    #[gpui_kit::test]
    fn a_shared_screen_has_its_own_option_switches(cx: &mut TestAppContext) {
        let mut h = Harness::new(cx);
        set_own(&h, cx, true, own_monitor());
        h.with_window(cx, |window, cx| {
            window.render_frame(cx);
            window.click("control-6", cx)
        });
        cx.run_until_parked();
        h.with_window(cx, |window, cx| {
            window.render_frame(cx);
            window.click("draw-6", cx)
        });
        cx.run_until_parked();
        assert!(matches!(
            h.drain().as_slice(),
            [
                UiCommand::ZetStreamBedienen(6, false),
                UiCommand::ZetStreamTekenen(6, true)
            ]
        ));
        set_own(
            &h,
            cx,
            true,
            OwnStreamState {
                controllable: false,
                remote_control: false,
                ..own_monitor()
            },
        );
        let shown = h.with_window(cx, |window, cx| {
            window.render_frame(cx);
            window.try_find("control-6").is_some()
        });
        assert!(!shown, "no control switch on a window share");
    }

    /// The picker's switches change the options of this one share, starting from the defaults.
    #[gpui_kit::test]
    fn the_picker_switches_flip_the_options_of_this_share(cx: &mut TestAppContext) {
        let h = Harness::new(cx);
        h.app.update(cx, |app, cx| {
            app.v_mut().dialog = Some(super::super::Dialog::Picker {
                sources: Vec::new(),
                drawing: true,
                control: false,
            });
            cx.notify();
        });
        cx.run_until_parked();
        h.with_window(cx, |window, cx| {
            window.render_frame(cx);
            window.click("picker-control", cx)
        });
        cx.run_until_parked();
        let options = h.app.read_with(cx, |app, _| match &app.v.dialog {
            Some(super::super::Dialog::Picker {
                drawing, control, ..
            }) => (*drawing, *control),
            other => panic!("picker closed: {other:?}"),
        });
        assert_eq!(options, (true, true));
    }
}
