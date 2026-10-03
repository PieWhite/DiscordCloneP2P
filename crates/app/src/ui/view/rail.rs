//! Slice A: the icon rail — channels, direct messages (with the unread badge), settings.
//!
//! Foundation placeholder: the three glyphs without behaviour. Navigation goes through
//! `AppView::show_channels`, `show_dms` and `show_settings`.

use super::super::icons::Icon;
use super::super::theme::{self, c};
use super::{AppSnapshot, View};
use gpui_kit::{div, prelude::FluentBuilder, px, AnyElement, App, IntoElement, ParentElement, Styled, Window};

pub fn render(snap: &AppSnapshot, _window: &mut Window, _cx: &mut App) -> AnyElement {
    let slot = |icon: Icon, current: bool| {
        div()
            .size(px(40.))
            .flex()
            .items_center()
            .justify_center()
            .text_color(c::SILVER)
            .when(current, |d| d.bg(c::PAPER).text_color(theme::ON_INVERT))
            .child(icon.size(20.))
    };
    div()
        .w(theme::RAIL_W)
        .flex_none()
        .h_full()
        .flex()
        .flex_col()
        .items_center()
        .py(px(8.))
        .gap(px(4.))
        .bg(c::INK)
        .border_r_1()
        .border_color(theme::BORDER)
        .child(slot(Icon::Hash, snap.v.view == View::Channels))
        .child(slot(Icon::At, snap.v.view == View::Dms))
        .child(div().flex_1())
        .child(div().w(px(24.)).h(px(1.)).my(px(4.)).bg(theme::BORDER))
        .child(slot(Icon::Gear, snap.v.view == View::Settings))
        .into_any_element()
}
