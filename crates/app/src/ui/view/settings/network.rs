//! The Network tab: the peers with their addresses, versions and link state, and the ports.

use super::controls::{control_row, help, help_rich, input_box, label, Body, Piece};
use super::{codes_field, SettingsPane};
use crate::ui::commands;
use crate::ui::state::{PeerState, Presence};
use crate::ui::theme::{self, c};
use crate::ui::view::format;
use crate::ui::view::widgets::{avatar, meter_view, Btn, BtnKind, Dot};
use crate::ui::view::AppSnapshot;
use gpui_kit::{
    div, prelude::FluentBuilder, px, ClipboardItem, Context, Div, IntoElement, ParentElement,
    SharedString, Styled, Window,
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

        let code = s.invite_code.clone();
        Body::new()
            .h2("Network")
            .p(concat!(
                "Peers find each other with invite codes: send yours to the others and paste ",
                "theirs below, once. Nothing goes through a server. Over the tailnet a key is ",
                "still pinned on first contact; after that, only that key gets in."
            ))
            .field(
                div()
                    .child(label("Your invite code"))
                    .child(help(concat!(
                        "Not a secret: it lets nobody in. It holds your key and the addresses this ",
                        "PC knows for itself; only a peer whose code you pasted gets in."
                    )))
                    .child(
                        control_row().child(code_block(&s.invite_code)).child(
                            Btn::new("copy-invite", "Copy")
                                .kind(BtnKind::Ghost)
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(code.clone()))
                                }),
                        ),
                    ),
            )
            .field(
                div()
                    .child(label("Add peers"))
                    .child(help(
                        "Paste one or more codes, or the whole message they came in.",
                    ))
                    .child(codes_field(&self.codes))
                    .child(
                        control_row()
                            .mt(px(8.))
                            .child(
                                Btn::new("add-codes", "Add")
                                    .kind(BtnKind::Accent)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        let text = this.codes.read(cx).value();
                                        commands::add_peer_codes(&this.ui, &text);
                                        this.codes.update(cx, |state, cx| {
                                            state.set_value(String::new(), window, cx)
                                        });
                                    })),
                            )
                            .when_some(s.code_result.clone(), |row, result| {
                                row.child(
                                    div()
                                        .max_w(px(420.))
                                        .text_size(px(12.))
                                        .text_color(c::SILVER)
                                        .whitespace_normal()
                                        .child(result),
                                )
                            }),
                    ),
            )
            .field(
                div()
                    .child(label("Peers"))
                    .child(help_rich(&[
                        Piece::Text("Everyone you added, kept in "),
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
            .field(
                div()
                    .child(label("Reachability"))
                    .child(help_rich(&[Piece::Text(
                        "Measured once, shortly after start: which port-mapping protocols the \
                         router speaks and what kind of address it shows outside. The test \
                         mapping is taken down again right after.",
                    )]))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(c::SILVER)
                            .font_family(theme::MONO_FONT)
                            .child(
                                s.router
                                    .clone()
                                    .unwrap_or_else(|| "Router: checking...".to_string()),
                            ),
                    ),
            )
    }
}

/// The invite code in the mono face, broken into blocks so it wraps. Copying goes through
/// the button, which takes the code whole.
fn code_block(code: &str) -> Div {
    let blocks: Vec<String> = code
        .as_bytes()
        .chunks(24)
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect();
    div()
        .max_w(px(420.))
        .px(px(9.))
        .py(px(6.))
        .bg(c::SPACE)
        .border_1()
        .border_color(theme::BORDER)
        .text_size(px(12.))
        .text_color(c::SKY)
        .font_family(theme::MONO_FONT)
        .child(
            div().flex().flex_row().flex_wrap().children(
                blocks
                    .into_iter()
                    .map(|b| div().whitespace_nowrap().child(b)),
            ),
        )
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
