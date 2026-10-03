//! Slice A: the channel column — channel list with unread badges, sub-channel rows (right
//! click opens `AppView::open_context_menu`), the DM list, and the voice panel at the
//! bottom (join/leave, peers in the call with the speaking ring, mute/deafen/DND, the self
//! row and its status menu, camera and share buttons, own streams, the hints).

use super::super::theme::{self, c};
use super::widgets::placeholder;
use super::{AppHandle, PaneInit, View};
use gpui_kit::{div, Context, IntoElement, ParentElement, Render, Styled, Window};

pub struct ChannelsPane {
    app: AppHandle,
}

impl ChannelsPane {
    pub fn new(init: &PaneInit, _window: &mut Window, _cx: &mut Context<Self>) -> Self {
        ChannelsPane { app: init.app.clone() }
    }
}

impl Render for ChannelsPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(snap) = self.app.snapshot(cx) else {
            return div().into_any_element();
        };
        let title = if snap.v.view == View::Dms { "Direct messages" } else { "Channels" };
        div()
            .size_full()
            .bg(c::INK)
            .border_r_1()
            .border_color(theme::BORDER)
            .child(placeholder(
                title,
                Some(c::MAGENTA),
                &format!("{} channels \u{00b7} slice A", snap.s.channels.len()),
            ))
            .into_any_element()
    }
}
