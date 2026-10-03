//! Slice E: the stream strip above the timeline (`renderStrip`): one tile per watched stream
//! and per own screen/camera (`self-<stream_id>`). Images come from `Thumbnails`; this pane
//! observes that entity, so a 2 Hz frame re-renders the strip and nothing else.
//!
//! The tiles have no action, as in the old strip: watching, unwatching and the desktop-audio
//! volume are the member list's share chips and sliders (`set_watching`, `set_stream_volume`).
//!
//! Drawn without `.cached()` because its height follows its content. The row scrolls
//! sideways instead of widening the main column (the old strip pushed the column past the
//! window at 780px with three tiles).

use super::super::bridge::Thumbnails;
use super::super::icons::Icon;
use super::super::state::{OwnStreamState, StreamState};
use super::super::theme::{self, c};
use super::{AppHandle, PaneInit};
use gpui_kit::{
    div, img, prelude::FluentBuilder, px, Context, Entity, InteractiveElement, IntoElement, ObjectFit,
    ParentElement, Render, RenderImage, SharedString, StatefulInteractiveElement, Styled, StyledImage, Window,
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

/// One tile of the strip, in the old order: watched streams first, then our own.
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
}

/// `renderStrip`'s tile list.
pub fn tiles(streams: &[StreamState], own_streams: &[OwnStreamState]) -> Vec<Tile> {
    let watched = streams.iter().filter(|st| st.watching).map(|st| Tile {
        key: Thumbnails::peer_key(&st.owner, st.stream_id),
        who: st.owner_name.clone(),
        what: st.title.clone(),
        live: true,
        camera: st.is_camera,
    });
    let own = own_streams.iter().filter(|st| !st.is_audio).map(|st| Tile {
        key: Thumbnails::own_key(st.stream_id),
        who: "You".into(),
        what: st.title.clone(),
        live: st.viewers > 0,
        camera: st.is_camera,
    });
    watched.chain(own).collect()
}

impl Render for StripPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(snap) = self.app.snapshot(cx) else {
            return div().into_any_element();
        };
        let tiles = tiles(&snap.s.streams, &snap.s.own_streams);
        if tiles.is_empty() {
            return div().into_any_element();
        }
        let thumbs = self.thumbs.read(cx);
        let rendered: Vec<_> = tiles
            .into_iter()
            .map(|t| {
                let image = thumbs.get(&t.key);
                render_tile(t, image)
            })
            .collect();
        div()
            .id("strip")
            .flex()
            .flex_none()
            .min_w_0()
            .gap(px(10.))
            .px(px(16.))
            .py(px(10.))
            .bg(c::INK)
            .border_b_1()
            .border_color(theme::BORDER)
            .overflow_x_scroll()
            .children(rendered)
            .into_any_element()
    }
}

/// `.tile` is 176px including its 1px frame.
const TILE_INNER_W: f32 = 174.;

fn render_tile(t: Tile, image: Option<Arc<RenderImage>>) -> impl IntoElement {
    let icon = if t.camera { Icon::Cam } else { Icon::Monitor };
    let caption: SharedString = format!("{} \u{00b7} {}", t.who, t.what).into();
    // A tile becomes an image only once a frame has arrived; until then the idle mark.
    let picture = match image {
        // An explicit height: `img` turns an auto height into the frame's own, not 16:9.
        Some(image) => img(image)
            .w(px(TILE_INNER_W))
            .h(px(TILE_INNER_W * 9. / 16.))
            .object_fit(ObjectFit::Cover)
            .bg(c::SPACE)
            .into_any_element(),
        None => div()
            .w(px(TILE_INNER_W))
            .h(px(TILE_INNER_W * 9. / 16.))
            .flex()
            .items_center()
            .justify_center()
            .text_color(c::SILVER)
            .child(icon.size(20.))
            .into_any_element(),
    };
    div()
        .id(SharedString::from(format!("tile-{}", t.key)))
        .relative()
        .w(px(176.))
        .flex_none()
        .overflow_hidden()
        .border_1()
        .border_color(theme::BORDER)
        .bg(c::SPACE)
        .hover(|s| s.border_color(c::CYAN))
        .child(picture)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(7.))
                .py(px(5.))
                .bg(c::INK)
                .text_size(px(11.))
                .text_color(c::SILVER)
                .child(icon.size(12.))
                .child(div().min_w_0().truncate().child(caption)),
        )
        .when(t.live, |d| d.child(live_tag()))
}

/// `.live-tag`: a void cell with a magenta frame and square. No letter-spacing in GPUI.
fn live_tag() -> impl IntoElement {
    div()
        .absolute()
        .top(px(6.))
        .left(px(6.))
        .flex()
        .items_center()
        .gap(px(4.))
        .pt(px(2.))
        .pb(px(2.))
        .pl(px(5.))
        .pr(px(6.))
        .bg(c::SPACE)
        .border_1()
        .border_color(c::MAGENTA)
        .text_size(px(10.))
        .text_color(c::PAPER)
        .child(div().size(px(5.)).bg(c::MAGENTA))
        .child("LIVE")
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
        }
    }

    /// Watched streams first, then our own without desktop audio; our own tile is LIVE only
    /// with a viewer. Dropping the `!is_audio` filter or the viewer rule makes it fail.
    #[test]
    fn the_strip_lists_watched_then_own_visual_streams() {
        let streams = vec![stream("p1", 1, true, false), stream("p1", 2, false, false)];
        let own_streams = vec![own(3, 0, false, false), own(4, 0, true, false), own(5, 1, false, true)];
        let t = tiles(&streams, &own_streams);
        let keys: Vec<_> = t.iter().map(|t| (t.key.as_str(), t.live, t.camera)).collect();
        assert_eq!(keys, vec![("p1-1", true, false), ("self-3", false, false), ("self-5", true, true)]);
        assert_eq!(t[1].who, "You");
    }
}
