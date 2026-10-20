//! The icon rail: channels, direct messages (with the unread badge), settings. `.rail` and
//! `.rail-btn` in the old stylesheet; the clicks are the old `#nav-*` handler.

use super::super::icons::Icon;
use super::super::theme;
use super::widgets::{badge, focus_ring, selectable, tooltip};
use super::{AppSnapshot, View};
use gpui_kit::{
    div, prelude::FluentBuilder, px, AnyElement, App, ClickEvent, Context, Div, InteractiveElement,
    IntoElement, ParentElement, Stateful, StatefulInteractiveElement, Styled, Window,
};

pub fn render(snap: &AppSnapshot, _window: &mut Window, _cx: &mut App) -> AnyElement {
    let view = snap.v.view;
    let unread = snap.total_dm_unread();
    let app = &snap.app;

    div()
        .w(theme::RAIL_W)
        .flex_none()
        .h_full()
        .flex()
        .flex_col()
        .items_center()
        .py(px(12.))
        .gap(px(8.))
        .bg(theme::BG_SIDEBAR)
        .border_r_1()
        .border_color(theme::BORDER)
        .child(
            rail_btn(
                "nav-channels",
                Icon::Hash,
                view == View::Channels,
                "Channels",
                0,
            )
            .on_click(
                app.listener(|app, _: &ClickEvent, window, cx| app.show_channels(window, cx)),
            ),
        )
        .child(
            rail_btn(
                "nav-dms",
                Icon::At,
                view == View::Dms,
                "Direct messages",
                unread,
            )
            .on_click(app.listener(|app, _: &ClickEvent, window, cx| app.show_dms(window, cx))),
        )
        .child(div().flex_1())
        .child(div().w(px(24.)).h(px(1.)).my(px(4.)).bg(theme::BORDER))
        .child(
            rail_btn(
                "nav-settings",
                Icon::Gear,
                view == View::Settings,
                "Settings",
                0,
            )
            .on_click(
                app.listener(|app, _: &ClickEvent, _, cx: &mut Context<_>| app.show_settings(cx)),
            ),
        )
        .into_any_element()
}

/// A 40×40 rail button: muted glyph, hover lifts it, current is the selected look. The
/// unread count is a badge on its top-right corner.
fn rail_btn(
    id: &'static str,
    icon: Icon,
    current: bool,
    title: &'static str,
    unread: usize,
) -> Stateful<Div> {
    selectable(
        div()
            .id(id)
            .tab_index(0)
            .focus_visible(|s| s.shadow(focus_ring()))
            .relative()
            .flex_none()
            .size(px(40.))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .text_color(theme::TEXT_MUTED)
            .tooltip(tooltip(title)),
        current,
    )
    .when(!current, |d| {
        d.hover(|s| s.bg(theme::BG_HOVER).text_color(theme::TEXT_STRONG))
    })
    .child(icon.size(18.))
    .when(unread > 0, |d| {
        d.child(badge(unread, false).absolute().top(px(-4.)).right(px(-6.)))
    })
}
