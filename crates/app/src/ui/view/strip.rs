//! Slice E: the stream strip above the timeline (`renderStrip`): one tile per watched stream,
//! per unwatched screen or camera of someone in the call, and per own screen/camera
//! (`self-<stream_id>`). Images come from `Thumbnails`; this pane observes that entity, so a
//! 2 Hz frame re-renders the strip and nothing else.
//!
//! A peer's tile is the one home of Watch / Stop watching (`set_watching`), and its speaker
//! button opens that person's audio popover, where the screen-audio volume lives.
//!
//! Drawn without `.cached()` because its height follows its content. The row scrolls
//! sideways instead of widening the main column (the old strip pushed the column past the
//! window at 780px with three tiles).

use super::super::bridge::Thumbnails;
use super::super::commands;
use super::super::icons::Icon;
use super::super::state::{OwnStreamState, StreamState};
use super::super::theme::{self, c};
use super::widgets::{icon_btn, live_pill, tooltip};
use super::{AppHandle, AppSnapshot, PaneInit};
use gpui_kit::{
    div, img, prelude::FluentBuilder, px, ClickEvent, Context, Entity, InteractiveElement,
    IntoElement, ObjectFit, ParentElement, Render, RenderImage, SharedString,
    StatefulInteractiveElement, Styled, StyledImage, TestSupportExt, Window,
};
use std::sync::Arc;

pub struct StripPane {
    app: AppHandle,
    thumbs: Entity<Thumbnails>,
}

impl StripPane {
    pub fn new(init: &PaneInit, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&init.thumbs, |_, _, cx| cx.notify()).detach();
        StripPane {
            app: init.app.clone(),
            thumbs: init.thumbs.clone(),
        }
    }
}

/// One tile of the strip: watched streams first, then the unwatched ones of the call, then
/// our own.
#[derive(Debug, Clone, PartialEq)]
pub struct Tile {
    /// The `Thumbnails` key: `<owner>-<id>` or `self-<id>`.
    pub key: String,
    pub who: String,
    pub what: String,
    /// The LIVE tag: always for a stream we watch; for our own only while someone watches,
    /// because a screen is captured only then.
    pub live: bool,
    pub camera: bool,
    /// We receive its frames; the tile wears the accent outline.
    pub watching: bool,
    /// Owner id and stream id of somebody else's stream; `None` for our own.
    pub peer: Option<(String, u32)>,
}

/// `renderStrip`'s tile list. `call` holds the ids of the peers in the call: only their
/// unwatched streams get a tile, since a stream we watch always has one.
pub fn tiles(streams: &[StreamState], own_streams: &[OwnStreamState], call: &[&str]) -> Vec<Tile> {
    let peer_tile = |st: &StreamState| Tile {
        key: Thumbnails::peer_key(&st.owner, st.stream_id),
        who: st.owner_name.clone(),
        what: st.title.clone(),
        live: st.watching,
        camera: st.is_camera,
        watching: st.watching,
        peer: Some((st.owner.clone(), st.stream_id)),
    };
    let watched = streams.iter().filter(|st| st.watching).map(peer_tile);
    let unwatched = streams
        .iter()
        .filter(|st| !st.watching && !st.is_audio && call.contains(&st.owner.as_str()))
        .map(peer_tile);
    let own = own_streams.iter().filter(|st| !st.is_audio).map(|st| Tile {
        key: Thumbnails::own_key(st.stream_id),
        who: "You".into(),
        what: st.title.clone(),
        live: st.viewers > 0,
        camera: st.is_camera,
        watching: false,
        peer: None,
    });
    watched.chain(unwatched).chain(own).collect()
}

impl Render for StripPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(snap) = self.app.snapshot(cx) else {
            return div().into_any_element();
        };
        let call: Vec<&str> = snap.call_peers().map(|p| p.id.as_str()).collect();
        let tiles = tiles(&snap.s.streams, &snap.s.own_streams, &call);
        if tiles.is_empty() {
            return div().into_any_element();
        }
        let thumbs = self.thumbs.read(cx);
        let rendered: Vec<_> = tiles
            .into_iter()
            .map(|t| {
                // An unwatched stream sends no frames; an old one would be a lie.
                let image = thumbs
                    .get(&t.key)
                    .filter(|_| t.watching || t.peer.is_none());
                render_tile(&snap, t, image)
            })
            .collect();
        div()
            .id("strip")
            .flex()
            .flex_none()
            .min_w_0()
            .gap(px(12.))
            .px(px(28.))
            .py(px(14.))
            .border_b_1()
            .border_color(theme::BORDER)
            .overflow_x_scroll()
            .children(rendered)
            .test_support()
            .into_any_element()
    }
}

/// `.tile` is 240px including its 1px frame.
const TILE_INNER_W: f32 = 238.;

fn render_tile(snap: &AppSnapshot, t: Tile, image: Option<Arc<RenderImage>>) -> impl IntoElement {
    let icon = if t.camera { Icon::Cam } else { Icon::Monitor };
    let kind = if t.camera { "camera" } else { "screen" };
    let well = || {
        div()
            .w(px(TILE_INNER_W))
            .h(px(TILE_INNER_W * 9. / 16.))
            .bg(theme::BG_WELL)
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(6.))
            .px(px(12.))
            .text_size(px(12.))
            .text_color(theme::TEXT_DIM)
            .child(icon.size(20.))
    };
    // A tile becomes an image only once a frame has arrived; until then the idle mark, and
    // a stream we do not watch says what it shows.
    let picture = match image {
        // An explicit height: `img` turns an auto height into the frame's own, not 16:9.
        Some(image) => img(image)
            .w(px(TILE_INNER_W))
            .h(px(TILE_INNER_W * 9. / 16.))
            .object_fit(ObjectFit::Cover)
            .bg(theme::BG_WELL)
            .into_any_element(),
        None if t.watching || t.peer.is_none() => well().into_any_element(),
        None => well()
            .child(div().max_w_full().truncate().child(t.what.clone()))
            .into_any_element(),
    };
    let actions = t.peer.clone().map(|(owner, stream)| {
        let watching = t.watching;
        let ui = snap.ui.clone();
        let who = owner.clone();
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.))
            .child(
                icon_btn(
                    SharedString::from(format!("tile-audio-{}", t.key)),
                    Icon::Volume,
                    false,
                )
                .tooltip(tooltip(format!("Audio for {}", t.who)))
                .on_click(
                    snap.app
                        .listener(move |app, _: &ClickEvent, _, cx| app.open_audio_for(&who, cx)),
                ),
            )
            .child(
                icon_btn(
                    SharedString::from(format!("tile-watch-{}", t.key)),
                    if watching { Icon::EyeOff } else { Icon::Eye },
                    false,
                )
                .tooltip(tooltip(if watching { "Stop watching" } else { "Watch" }))
                .on_click(move |_: &ClickEvent, _, _| {
                    commands::set_watching(&ui, &owner, stream, !watching)
                }),
            )
    });
    div()
        .id(SharedString::from(format!("tile-{}", t.key)))
        .relative()
        .w(px(TILE_INNER_W + 2.))
        .flex_none()
        .overflow_hidden()
        .rounded(theme::RADIUS_CARD)
        .border_1()
        .border_color(if t.watching {
            theme::ACCENT
        } else {
            theme::BORDER
        })
        .bg(c::VOID)
        .hover(|s| s.border_color(theme::TEXT_STRONG))
        .child(picture)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .h(px(46.))
                .pl(px(12.))
                .pr(px(8.))
                .border_t_1()
                .border_color(theme::BORDER)
                .text_size(px(13.))
                .text_color(theme::TEXT)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .gap(px(4.))
                        // The name wins the room; the kind gives way first.
                        .child(
                            div()
                                .flex_none()
                                .max_w(px(140.))
                                .truncate()
                                .child(t.who.clone()),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_color(theme::TEXT_DIM)
                                .child(format!("/ {kind}")),
                        ),
                )
                .children(actions),
        )
        .when(t.live, |d| {
            d.child(
                div()
                    .absolute()
                    .top(px(10.))
                    .left(px(10.))
                    .child(live_pill()),
            )
        })
        .test_support()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(owner: &str, id: u32, watching: bool, is_audio: bool) -> StreamState {
        StreamState {
            owner: owner.into(),
            owner_name: "Sander".into(),
            stream_id: id,
            title: "Counter-Strike 2".into(),
            width: 1920,
            height: 1080,
            watching,
            is_audio,
            is_camera: false,
            volume: 1.0,
        }
    }

    fn own(id: u32, viewers: usize, is_audio: bool, is_camera: bool) -> OwnStreamState {
        OwnStreamState {
            stream_id: id,
            title: "Scherm".into(),
            viewers,
            is_audio,
            is_camera,
            is_music: false,
            drawing: false,
            has_drawing: false,
        }
    }

    /// Watched streams first, then our own without desktop audio; our own tile is LIVE only
    /// with a viewer. Dropping the `!is_audio` filter or the viewer rule makes it fail.
    #[test]
    fn the_strip_lists_watched_then_own_visual_streams() {
        let streams = vec![stream("p1", 1, true, false), stream("p1", 2, false, false)];
        let own_streams = vec![
            own(3, 0, false, false),
            own(4, 0, true, false),
            own(5, 1, false, true),
        ];
        let t = tiles(&streams, &own_streams, &[]);
        let keys: Vec<_> = t
            .iter()
            .map(|t| (t.key.as_str(), t.live, t.camera))
            .collect();
        assert_eq!(
            keys,
            vec![
                ("p1-1", true, false),
                ("self-3", false, false),
                ("self-5", true, true)
            ]
        );
        assert_eq!(t[1].who, "You");
    }

    /// A call peer's stream we do not watch still gets a tile, unwatched, between the watched
    /// ones and our own; a stream of someone outside the call does not. Dropping the
    /// `unwatched` chain in `tiles` makes it fail.
    #[test]
    fn an_unwatched_stream_of_the_call_gets_an_unwatched_tile() {
        let streams = vec![
            stream("p2", 2, false, false),
            stream("p1", 1, true, false),
            stream("p9", 9, false, false),
        ];
        let own_streams = vec![own(3, 0, false, false)];
        let t = tiles(&streams, &own_streams, &["p1", "p2"]);
        let keys: Vec<_> = t
            .iter()
            .map(|t| (t.key.as_str(), t.watching, t.live, t.peer.clone()))
            .collect();
        assert_eq!(
            keys,
            vec![
                ("p1-1", true, true, Some(("p1".to_string(), 1))),
                ("p2-2", false, false, Some(("p2".to_string(), 2))),
                ("self-3", false, false, None),
            ]
        );
    }

    /// More tiles than the column holds scroll sideways instead of being cut off for good.
    /// Revert to see it fail: drop `.overflow_x_scroll()` from the strip.
    #[gpui_kit::test]
    fn the_strip_scrolls_sideways(cx: &mut gpui_kit::TestAppContext) {
        use super::super::composer::window_tests::Harness;
        use gpui_kit::test::TestWindowExt;
        use gpui_kit::{point, ScrollDelta};
        let h = Harness::new(cx);
        h.app.update(cx, |app, cx| {
            let mut s = (*app.s).clone();
            s.own_streams = (1..=8).map(|id| own(id, 0, false, false)).collect();
            app.s = std::sync::Arc::new(s);
            cx.notify();
        });
        cx.run_until_parked();
        let first_x = |cx: &mut gpui_kit::TestAppContext| {
            h.with_window(cx, |window, cx| {
                window.render_frame(cx);
                window.find("tile-self-1").bounds().origin.x
            })
        };
        let before = first_x(cx);
        h.with_window(cx, |window, cx| {
            window.scroll("strip", ScrollDelta::Pixels(point(px(-400.), px(0.))), cx)
        });
        cx.run_until_parked();
        assert!(first_x(cx) < before, "the first tile moved left");
    }
}
