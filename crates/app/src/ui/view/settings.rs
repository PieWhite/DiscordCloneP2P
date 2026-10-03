//! Slice D: the settings view, seven tabs (`SettingsTab`). Switch tabs with
//! `AppView::set_settings_tab`, which also loads what a tab needs (`v.devices`,
//! `v.clip_monitors`, `v.recap`); Refresh buttons call `refresh_devices` / `refresh_recap`.
//! The clip hotkey capture uses `v.hotkey_wait` (Escape is already handled at the root).
//! The input level meter reads `Meters::self_level` through `widgets::meter_view`.

use super::super::theme::{self, c};
use super::widgets::placeholder;
use super::{AppHandle, PaneInit};
use gpui_kit::{div, Context, IntoElement, ParentElement, Render, Styled, Window};

pub struct SettingsPane {
    app: AppHandle,
}

impl SettingsPane {
    pub fn new(init: &PaneInit, _window: &mut Window, _cx: &mut Context<Self>) -> Self {
        SettingsPane { app: init.app.clone() }
    }
}

impl Render for SettingsPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(snap) = self.app.snapshot(cx) else {
            return div().into_any_element();
        };
        div()
            .size_full()
            .flex()
            .child(
                div()
                    .w(theme::SETTINGS_NAV_W)
                    .flex_none()
                    .h_full()
                    .bg(c::INK)
                    .border_r_1()
                    .border_color(theme::BORDER)
                    .child(placeholder("Settings", Some(c::AMBER), "tabs \u{00b7} slice D")),
            )
            .child(div().flex_1().child(placeholder(
                "",
                None,
                &format!("{} tab \u{00b7} slice D", snap.v.settings_tab.name()),
            )))
            .into_any_element()
    }
}
