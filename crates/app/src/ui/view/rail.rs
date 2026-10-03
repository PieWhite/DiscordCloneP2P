//! The icon rail: channels, direct messages (with the unread badge), settings. `.rail` and
//! `.rail-btn` in the old stylesheet; the clicks are the old `#nav-*` handler.

use super::super::icons::Icon;
use super::super::theme::{self, c};
use super::widgets::tooltip;
use super::{AppSnapshot, View};
use gpui_kit::{
    div, prelude::FluentBuilder, px, AnyElement, App, ClickEvent, Context, Div, InteractiveElement,
    IntoElement, ParentElement, SharedString, Stateful, StatefulInteractiveElement, Styled, Window,
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
        .py(px(8.))
        .gap(px(4.))
        .bg(c::INK)
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

/// `.rail-btn`: 40×40; hover is the violet slab, current the paper inversion. The cyan
/// marker sits on the rail's left edge: full height when current, a stub on hover. It
/// snaps; the stylesheet's two-step transition is not worth a timer (invariant 4).
fn rail_btn(
    id: &'static str,
    icon: Icon,
    current: bool,
    title: &'static str,
    badge: usize,
) -> Stateful<Div> {
    let group = SharedString::from(id);
    let marker = |h: f32| {
        div()
            .absolute()
            .left(px(-8.))
            .top(px((40. - h) / 2.))
            .w(px(3.))
            .h(px(h))
            .bg(c::CYAN)
    };
    div()
        .id(id)
        .group(group.clone())
        .relative()
        .flex_none()
        .size(px(40.))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .text_color(c::SILVER)
        .tooltip(tooltip(title))
        .when(current, |d| {
            d.bg(c::PAPER)
                .text_color(theme::ON_INVERT)
                .child(marker(22.))
        })
        .when(!current, |d| {
            d.hover(|s| s.bg(theme::BG_HOVER).text_color(theme::TEXT))
                .child(
                    marker(10.)
                        .opacity(0.)
                        .group_hover(group, |s| s.opacity(1.)),
                )
        })
        .child(icon.size(20.))
        // `.rail-badge`: a paper cell with ink digits in a 2px ink frame, so it reads as cut
        // out of whatever the button behind it shows.
        .when(badge > 0, |d| {
            d.child(
                div()
                    .absolute()
                    .right(px(2.))
                    .bottom(px(2.))
                    .min_w(px(28.))
                    .h(px(20.))
                    .px(px(4.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(c::PAPER)
                    .border_2()
                    .border_color(c::INK)
                    .text_color(c::SPACE)
                    .text_size(px(10.))
                    .line_height(px(16.))
                    .child(badge.to_string()),
            )
        })
}
