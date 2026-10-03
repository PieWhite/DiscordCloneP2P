//! The Network tab: the peers with their addresses, versions and link state, and the ports.

use super::controls::{control_row, help_rich, input_box, label, Body, Piece};
use super::SettingsPane;
use crate::ui::state::{PeerState, Presence};
use crate::ui::theme::{self, c};
use crate::ui::view::format;
use crate::ui::view::widgets::{avatar, meter_view, Dot};
use crate::ui::view::AppSnapshot;
use gpui_kit::{
    div, prelude::FluentBuilder, px, Context, Div, IntoElement, ParentElement, SharedString,
    Styled, Window,
};

impl SettingsPane {
    pub(super) fn network(
        &mut self,
        snap: &AppSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Body {
        let s = &snap.s;
        let rows = s
            .peers
            .iter()
            .enumerate()
            .map(|(i, p)| peer_row(snap, i, p, window, cx));
        let table = div()
            .max_w(px(520.))
            .border_1()
            .border_color(theme::BORDER)
            .overflow_hidden()
            .children(rows);

        let port = |id: &'static str, title: &'static str, value: u16| {
            div()
                .child(
                    div()
                        .mb(px(4.))
                        .text_size(px(12.))
                        .text_color(c::SILVER)
                        .child(title),
                )
                .child(
                    // An `<input>` is 20 characters wide whatever `min-width` says.
                    input_box(id, theme::BORDER_INPUT)
                        .w(px(190.))
                        .font_family(theme::MONO_FONT)
                        .child(value.to_string()),
                )
        };

        Body::new()
            .h2("Network")
            .p(concat!(
                "Peers are addressed over the tailnet. Identities are learned on first contact, so ",
                "there is nothing to exchange by hand."
            ))
            .field(
                div()
                    .child(label("Peers"))
                    .child(help_rich(&[
                        Piece::Text("Each peer lists the other two, in "),
                        Piece::Mono("config.toml"),
                        Piece::Text("."),
                    ]))
                    .child(table),
            )
            .field(
                div()
                    .child(label("Ports"))
                    .child(help_rich(&[
                        Piece::Text("Changed in "),
                        Piece::Mono("config.toml"),
                        Piece::Text("; the app reads them at start."),
                    ]))
                    .child(
                        control_row()
                            .child(port("port-control", "Control (QUIC)", s.control_port))
                            .child(port("port-media", "Media (UDP)", s.media_port)),
                    ),
            )
    }
}

/// `.peer-row`: avatar, name over address and version, and the link on the right.
fn peer_row(
    snap: &AppSnapshot,
    index: usize,
    p: &PeerState,
    window: &mut Window,
    cx: &mut Context<SettingsPane>,
) -> Div {
    let address = match &p.app_version {
        Some(v) => format!("{} \u{b7} v{v}", p.address),
        None => p.address.clone(),
    };
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .px(px(12.))
        .py(px(10.))
        .bg(c::INK)
        .when(index > 0, |d| d.border_t_1().border_color(theme::BORDER))
        .child(avatar(&p.name, p.avatar, 24., Some(Dot::of_peer(p)), false))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(div().text_size(px(13.)).child(p.name.clone()))
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(c::SILVER)
                        .font_family(theme::MONO_FONT)
                        .child(address),
                ),
        )
        .child(link(snap, p, window, cx))
}

/// `.link`: the state square and its word. Only the RTT of an online peer moves with the
/// meters, so only that cell is a meter view. The problem text of a broken peer wraps in a
/// column of its own instead of running over the address (a defect of the old window).
fn link(
    snap: &AppSnapshot,
    p: &PeerState,
    window: &mut Window,
    cx: &mut Context<SettingsPane>,
) -> Div {
    let square = |fill: Option<gpui_kit::Rgba>, ring: bool| {
        div()
            .flex_none()
            .mt(px(7.))
            .size(px(6.))
            .when_some(fill, |d, f| d.bg(f))
            .when(ring, |d| d.border(px(1.5)).border_color(theme::OFFLINE))
    };
    let (dot, text) = match p.presence {
        Presence::Online => {
            let id = p.id.clone();
            let key = SharedString::from(format!(
                "net-rtt-{}",
                if p.id.is_empty() { &p.address } else { &p.id }
            ));
            let rtt = meter_view(key, &snap.meters, window, cx, move |m, _, _| {
                div().child(format::fmt_rtt(m.rtt(&id))).into_any_element()
            });
            (square(Some(theme::ONLINE), false), div().child(rtt))
        }
        Presence::Connecting => (
            square(Some(theme::CONNECTING), false),
            div().child("reconnecting"),
        ),
        Presence::Broken => (
            square(None, false),
            div().child(
                p.problem
                    .clone()
                    .unwrap_or_else(|| "needs attention".into()),
            ),
        ),
        Presence::Offline => (square(None, true), div().child("offline")),
    };
    div()
        .flex()
        .flex_none()
        .items_start()
        .gap(px(6.))
        .max_w(px(230.))
        .text_color(c::SILVER)
        .child(dot)
        .child(
            text.min_w_0()
                .flex_shrink(1.)
                .whitespace_normal()
                .text_right(),
        )
}
