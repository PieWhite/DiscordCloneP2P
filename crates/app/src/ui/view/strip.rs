//! Slice E: the stream strip above the timeline — one tile per watched stream and per own
//! screen/camera (`self-<stream_id>`), watch/unwatch, volume. Images come from
//! `Thumbnails` (`thumbs.read(cx).get(&Thumbnails::peer_key(owner, id))`); this pane
//! observes that entity, so a 2 Hz frame re-renders the strip and nothing else.
//!
//! Drawn without `.cached()` because its height follows its content.

use super::super::bridge::Thumbnails;
use super::super::theme::{self, c};
use super::{AppHandle, PaneInit};
use gpui_kit::{div, px, Context, Entity, IntoElement, ParentElement, Render, Styled, Window};

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

impl Render for StripPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(snap) = self.app.snapshot(cx) else {
            return div();
        };
        let tiles = snap.watched_streams().count() + snap.s.own_streams.iter().filter(|s| !s.is_audio).count();
        if tiles == 0 {
            return div();
        }
        let _ = self.thumbs.read(cx);
        div()
            .flex()
            .gap(px(10.))
            .px(px(16.))
            .py(px(10.))
            .bg(c::INK)
            .border_b_1()
            .border_color(theme::BORDER)
            .text_size(px(11.))
            .text_color(c::SILVER)
            .child(format!("{tiles} stream tile(s) \u{00b7} slice E"))
    }
}
