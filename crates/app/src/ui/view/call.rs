//! The voice-connected panel at the foot of the channel column, above the self panel, drawn
//! only while joined (decision 48, moved there as in Discord on 2026-10-20): the call's size
//! and worst link with Leave beside it, then camera, screen share and Spotify in one row, and
//! under that one quiet line per shared Spotify. These are the only homes of those controls;
//! the drawing switch for your shared screens is in Settings → Video.

use super::super::commands::{self, MusicAction};
use super::super::icons::Icon;
use super::super::state::PeerState;
use super::super::theme;
use super::widgets::{dk_btn, meter_view, tooltip};
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

    let mut music: Vec<AnyElement> = Vec::new();
    if let Some(own) = snap.own_music() {
        music.push(
            music_line(
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
        music.push(
            music_line(&format!("{} plays ", p.name), &m.title, "")
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
        .when(!music.is_empty(), |d| {
            d.child(div().flex().flex_col().children(music))
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

/// One muted line with the title in white.
fn music_line(before: &str, title: &str, after: &str) -> Div {
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
                .child(Icon::Music.size(13.)),
        )
        .child(
            div()
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
}
