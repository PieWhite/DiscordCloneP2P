//! The voice-connected panel at the foot of the channel column, above the self panel, drawn
//! only while joined (decision 48, moved there as in Discord on 2026-10-20): the call's size
//! and worst link with Leave beside it, then camera, screen share and Spotify in one row, and
//! under that one quiet line per screen you share (with its own stop) and per shared Spotify. These are the only homes of those controls;
//! the drawing switch for your shared screens is in Settings → Video.

use super::super::commands::{self, MusicAction};
use super::super::icons::Icon;
use super::super::state::PeerState;
use super::super::theme;
use super::widgets::{dk_btn, meter_view, tooltip, Btn};
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
}
