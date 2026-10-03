//! Slice A: the member list — online/offline groups, speaking state and RTT (through
//! `widgets::meter_view`), last seen, status, per-peer volume (mute is volume 0 with the
//! old level in `v.volume_before_mute`), desktop-audio volume.

use super::super::theme::{self, c};
use super::widgets::placeholder;
use super::{AppHandle, PaneInit};
use gpui_kit::{div, Context, IntoElement, ParentElement, Render, Styled, Window};

pub struct MembersPane {
    app: AppHandle,
}

impl MembersPane {
    pub fn new(init: &PaneInit, _window: &mut Window, _cx: &mut Context<Self>) -> Self {
        MembersPane { app: init.app.clone() }
    }
}

impl Render for MembersPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(snap) = self.app.snapshot(cx) else {
            return div().into_any_element();
        };
        div()
            .size_full()
            .bg(c::INK)
            .border_l_1()
            .border_color(theme::BORDER)
            .child(placeholder(
                "Members",
                Some(c::GREEN),
                &format!("{} known peers \u{00b7} slice A", snap.known_peers().count()),
            ))
            .into_any_element()
    }
}
