//! The status bar (`renderStatus`): one link per configured peer with its RTT, the update
//! chip, version and protocol. RTT comes from `snap.meters` through `widgets::meter_view`,
//! never from `s`, so a meters tick redraws only those numbers.

use super::super::commands;
use super::super::icons::Icon;
use super::super::state::{PeerState, Presence, UpdateState};
use super::super::theme::{self, c};
use super::widgets::{focus_ring, meter_view, tooltip};
use super::{format, AppSnapshot, Dialog};
use gpui_kit::{
    div, prelude::FluentBuilder, px, AnyElement, App, ClickEvent, InteractiveElement, IntoElement,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, Window,
};

pub fn render(snap: &AppSnapshot, window: &mut Window, cx: &mut App) -> AnyElement {
    let links: Vec<AnyElement> = snap
        .s
        .peers
        .iter()
        .enumerate()
        .map(|(i, p)| link(snap, i, p, window, cx))
        .collect();

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
        .overflow_hidden()
        .children(links)
        .child(div().flex_1())
        .when_some(update_chip(snap), |d, chip| d.child(chip))
        .child(
            div()
                .flex_none()
                .whitespace_nowrap()
                .font_family(theme::MONO_FONT)
                .child(format!(
                    "v{} \u{00b7} protocol {}",
                    snap.s.app_version, snap.s.protocol_version
                )),
        )
        .into_any_element()
}

/// `.link`: the 6px presence square, the name in paper, and what the link is doing.
fn link(
    snap: &AppSnapshot,
    index: usize,
    p: &PeerState,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let square = div().size(px(6.)).flex_none();
    let square = match p.presence {
        Presence::Online => square.bg(theme::ONLINE),
        Presence::Offline => square.border(px(1.5)).border_color(theme::OFFLINE),
        // The stylesheet has no rule for a broken link: the square's place stays empty.
        Presence::Broken => square,
    };
    let state: AnyElement = match p.presence {
        Presence::Online => {
            let id = p.id.clone();
            // Keyed by position too: unintroduced peers all have an empty id.
            meter_view(
                ("status-rtt", index),
                &snap.meters,
                window,
                cx,
                move |m, _, _| div().child(format::fmt_rtt(m.rtt(&id))).into_any_element(),
            )
            .into_any_element()
        }
        Presence::Broken => div().child("needs attention").into_any_element(),
        Presence::Offline => div().child("offline").into_any_element(),
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.))
        .whitespace_nowrap()
        .child(square)
        .child(div().text_color(c::PAPER).child(p.name.clone()))
        .child(state)
        .into_any_element()
}

/// `.update-chip`, in the five shapes the update state can take. Only "ready" and the two
/// answers are buttons; the running states are plain chips (hover still inverts them, as
/// the stylesheet's rule did not care).
fn update_chip(snap: &AppSnapshot) -> Option<AnyElement> {
    let pct =
        |received: u64, total: u64| (100.0 * received as f64 / total.max(1) as f64).round() as u64;
    let (icon, text, click, title): (Icon, String, Option<Click>, Option<String>) =
        match snap.s.update.as_ref()? {
            UpdateState::Ready { version } => (
                Icon::Download,
                format!("Update to {version} is ready"),
                Some(Click::Apply(version.clone())),
                None,
            ),
            UpdateState::Downloading {
                version,
                received,
                total,
            } => (
                Icon::Download,
                format!("Fetching {version} \u{2014} {}%", pct(*received, *total)),
                None,
                None,
            ),
            UpdateState::Checking => (
                Icon::Retry,
                "Checking for updates\u{2026}".into(),
                None,
                None,
            ),
            // The error text matters: a manifest pointing at a missing release asset looks
            // exactly like "nothing happens" without it.
            UpdateState::Failed { error } => (
                Icon::Alert,
                "Update failed".into(),
                Some(Click::Dismiss),
                Some(error.clone()),
            ),
            UpdateState::UpToDate => (Icon::Check, "Up to date".into(), Some(Click::Dismiss), None),
        };

    let id = SharedString::from(match click {
        Some(Click::Apply(_)) => "btn-update",
        Some(Click::Dismiss) => "btn-update-dismiss",
        None => "update-chip",
    });
    let chip = div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.))
        .h(px(19.))
        .px(px(8.))
        .bg(c::SPACE)
        .border_1()
        .border_color(theme::ACCENT)
        .text_color(theme::ACCENT)
        .text_size(px(11.))
        .whitespace_nowrap()
        .hover(|s| s.bg(theme::ACCENT).text_color(theme::ON_ACCENT))
        .child(icon.size(12.))
        .child(text)
        .when_some(title.filter(|t| !t.is_empty()), |d, t| {
            d.tooltip(tooltip(t))
        })
        .when_some(click, |d, click| {
            let app = snap.app.clone();
            let d = d
                .tab_index(0)
                .focus_visible(|s| s.shadow(focus_ring(c::INK, vec![])));
            let ui = snap.ui.clone();
            d.cursor_pointer()
                .on_click(move |_: &ClickEvent, window, cx| match &click {
                    Click::Apply(version) => {
                        let dialog = Dialog::apply_update(version);
                        app.update(cx, |app, cx| app.open_dialog(dialog, window, cx));
                    }
                    Click::Dismiss => commands::dismiss_update(&ui),
                })
        });
    Some(chip.into_any_element())
}

enum Click {
    Apply(String),
    Dismiss,
}
