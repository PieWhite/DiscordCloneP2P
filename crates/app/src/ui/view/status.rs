//! Slice A: the status bar — one link per peer with its RTT, the update chip, version and
//! protocol. RTT comes from `snap.meters` through `widgets::meter_view`, never from `s`.
//!
//! Foundation placeholder: the bar and the version line.

use super::super::theme::{self, c};
use super::AppSnapshot;
use gpui_kit::{div, px, AnyElement, App, IntoElement, ParentElement, Styled, Window};

pub fn render(snap: &AppSnapshot, _window: &mut Window, _cx: &mut App) -> AnyElement {
    div()
        .h(theme::STATUS_H)
        .flex_none()
        .flex()
        .items_center()
        .gap(px(14.))
        .px(px(12.))
        .bg(c::INK)
        .border_t_1()
        .border_color(theme::BORDER)
        .text_size(px(11.5))
        .text_color(c::SILVER)
        .child(format!("{} peers \u{00b7} status bar (slice A)", snap.s.peers.len()))
        .child(div().flex_1())
        .child(
            div()
                .font_family(theme::MONO_FONT)
                .child(format!("v{} \u{00b7} protocol {}", snap.s.app_version, snap.s.protocol_version)),
        )
        .into_any_element()
}
