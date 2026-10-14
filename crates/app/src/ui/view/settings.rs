//! The settings view, seven tabs (`SettingsTab`). Switch tabs with
//! `AppView::set_settings_tab`, which also loads what a tab needs (`v.devices`,
//! `v.clip_monitors`, `v.recap`); Refresh buttons call `refresh_devices` / `refresh_recap`.
//!
//! The old window rebuilt this panel from state and skipped the rebuild while a field had
//! focus (`bezigMetInvoer`). Here every field keeps its own state in an entity of this pane,
//! so a state event never reaches what the user is typing; the engine's value is pushed in
//! only when it changed and the control is not in use (see [`controls::Choices::sync`],
//! [`controls::Ranged::sync`] and `sync_name`).

mod account;
mod audio;
mod clips;
mod controls;
mod files;
mod network;
mod recap;
mod video;

use super::super::theme::{self, c};
use super::widgets::pointer;
use super::{AppHandle, AppSnapshot, PaneInit, SettingsTab};
use crate::ui::commands;
use crate::ui::Ui;
use controls::{Choices, Ranged};
use gpui_kit::base::slider::SliderEvent;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::scroll::{Scrollbar, ScrollbarAxis};
use gpui_kit::component::select::SelectEvent;
use gpui_kit::{
    div, point, prelude::FluentBuilder, px, AppContext, Context, Div, Entity, FocusHandle,
    Focusable, InteractiveElement, IntoElement, ParentElement, Render, ScrollHandle,
    StatefulInteractiveElement, Styled, Subscription, Window,
};
use std::sync::Arc;

/// The first option of the two device selects: `None` in the engine.
const DEFAULT_DEVICE: &str = "Windows default";
/// The first option of the screen select: an empty name in the engine.
const AUTO_MONITOR: &str = "Automatic (main screen)";

pub struct SettingsPane {
    app: AppHandle,
    ui: Arc<Ui>,
    name: Entity<InputState>,
    /// The display name last copied into `name`.
    synced_name: Option<String>,
    status: Choices,
    mic: Choices,
    speaker: Choices,
    monitor: Choices,
    clip_length: Choices,
    bitrate: Ranged,
    sound_volume: Ranged,
    /// The clip shortcut box; it has focus exactly while it is listening.
    hotkey_focus: FocusHandle,
    /// Modifiers held while listening, for the "ctrl+..." text.
    pending_mods: String,
    scroll: ScrollHandle,
    shown_tab: Option<SettingsTab>,
    _subscriptions: Vec<Subscription>,
}

impl SettingsPane {
    pub fn new(init: &PaneInit, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx));
        let status = Choices::new(window, cx);
        let mic = Choices::new(window, cx);
        let speaker = Choices::new(window, cx);
        let monitor = Choices::new(window, cx);
        let clip_length = Choices::new(window, cx);
        let bitrate = Ranged::new(4., 40., cx);
        let sound_volume = Ranged::new(0., 100., cx);
        let hotkey_focus = cx.focus_handle().tab_stop(true);

        let mut subscriptions = vec![
            cx.subscribe(
                status.state(),
                |this, _, ev: &SelectEvent<Vec<String>>, _| {
                    let SelectEvent::Confirm(Some(label)) = ev else {
                        return;
                    };
                    commands::set_user_status(&this.ui, &label.to_lowercase());
                },
            ),
            cx.subscribe(mic.state(), |this, _, _: &SelectEvent<Vec<String>>, cx| {
                this.devices_chosen(cx)
            }),
            cx.subscribe(
                speaker.state(),
                |this, _, _: &SelectEvent<Vec<String>>, cx| this.devices_chosen(cx),
            ),
            cx.subscribe(
                monitor.state(),
                |this, _, ev: &SelectEvent<Vec<String>>, _| {
                    let SelectEvent::Confirm(Some(label)) = ev else {
                        return;
                    };
                    let name = if label == AUTO_MONITOR {
                        ""
                    } else {
                        label.as_str()
                    };
                    commands::set_clip_monitor(&this.ui, name);
                },
            ),
            cx.subscribe(
                clip_length.state(),
                |this, _, ev: &SelectEvent<Vec<String>>, _| {
                    let SelectEvent::Confirm(Some(label)) = ev else {
                        return;
                    };
                    if let Some(sec) = clips::length_seconds(label) {
                        commands::set_clip_length(&this.ui, sec);
                    }
                },
            ),
            // Dragging only moves the readout; the change goes out on release, once, like
            // the `change` event of a range input (so the tone does not play per pixel).
            cx.subscribe(&bitrate.state, |this, _, ev: &SliderEvent, cx| {
                if let Some(mbit) = this.bitrate.event(ev) {
                    this.commit_bitrate(mbit, cx);
                }
                cx.notify();
            }),
            cx.subscribe(&sound_volume.state, |this, _, ev: &SliderEvent, cx| {
                if let Some(percent) = this.sound_volume.event(ev) {
                    this.commit_sound_volume(percent, cx);
                }
                cx.notify();
            }),
            // Clicking away from the shortcut box ends the listening.
            cx.on_blur(&hotkey_focus, window, |this, _, cx| {
                this.pending_mods.clear();
                this.app.update(cx, |app, cx| {
                    if app.view_state().hotkey_wait {
                        app.v_mut().hotkey_wait = false;
                        cx.notify();
                    }
                });
            }),
        ];
        // The sliders redraw themselves; the pane redraws for the readout beside them.
        subscriptions.push(cx.observe(&bitrate.state, |_, _, cx| cx.notify()));
        subscriptions.push(cx.observe(&sound_volume.state, |_, _, cx| cx.notify()));

        SettingsPane {
            app: init.app.clone(),
            ui: init.ui.clone(),
            name,
            synced_name: None,
            status,
            mic,
            speaker,
            monitor,
            clip_length,
            bitrate,
            sound_volume,
            hotkey_focus,
            pending_mods: String::new(),
            scroll: ScrollHandle::new(),
            shown_tab: None,
            _subscriptions: subscriptions,
        }
    }

    /// Both device selects are sent together, as `set-in` / `set-out` were.
    fn devices_chosen(&self, cx: &mut Context<Self>) {
        let device = |label: Option<String>| label.filter(|l| l != DEFAULT_DEVICE);
        commands::set_audio_devices(
            &self.ui,
            device(self.mic.value(cx)),
            device(self.speaker.value(cx)),
        );
    }

    fn commit_bitrate(&self, mbit: f32, cx: &mut Context<Self>) {
        let Some(snap) = self.app.snapshot(cx) else {
            return;
        };
        commands::set_video_settings(
            &snap.ui,
            &snap.s.video.codec,
            snap.s.video.fps,
            mbit as u32 * 1_000_000,
        );
    }

    fn commit_sound_volume(&self, percent: f32, cx: &mut Context<Self>) {
        let Some(snap) = self.app.snapshot(cx) else {
            return;
        };
        commands::set_sound_settings(&snap.ui, &snap.s.sound.set, percent / 100.);
    }

    /// The display name goes into the input when the engine's name changed and nobody is
    /// typing in it; what is typed is never replaced.
    fn sync_name(&mut self, engine: &str, window: &mut Window, cx: &mut Context<Self>) {
        let typing = self.name.read(cx).focus_handle(cx).is_focused(window);
        if !should_pull_name(self.synced_name.as_deref(), engine, typing) {
            return;
        }
        self.synced_name = Some(engine.to_string());
        self.name.update(cx, |input, cx| {
            input.set_value(engine.to_string(), window, cx)
        });
    }

    fn nav(&self, snap: &AppSnapshot) -> Div {
        let current = snap.v.settings_tab;
        div()
            .w(theme::SETTINGS_NAV_W)
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .bg(c::INK)
            .border_r_1()
            .border_color(theme::BORDER)
            .child(
                div()
                    .h(theme::HEAD_H)
                    .flex_none()
                    .flex()
                    .items_center()
                    .px(px(14.))
                    .bg(c::AMBER)
                    .border_b_1()
                    .border_color(c::SPACE)
                    .text_color(c::SPACE)
                    .text_size(px(13.))
                    .child("SETTINGS"),
            )
            .child(
                div().flex().flex_col().px(px(8.)).pt(px(8.)).children(
                    SettingsTab::ALL
                        .into_iter()
                        .map(|tab| tab_row(snap, tab, tab == current)),
                ),
            )
    }
}

/// The engine's display name goes into the input only when it is not what the input was last
/// given and the user is not in it. Typing is never replaced.
fn should_pull_name(synced: Option<&str>, engine: &str, typing: bool) -> bool {
    !typing && synced != Some(engine)
}

/// `.set-tab`: silver row; hover a violet slab with the pointer; current inverted.
fn tab_row(snap: &AppSnapshot, tab: SettingsTab, current: bool) -> impl IntoElement {
    let group = format!("set-tab-{}", tab.id());
    div()
        .id(tab.id())
        .group(group.clone())
        .relative()
        .flex()
        .items_center()
        .gap(px(9.))
        .w_full()
        .h(px(32.))
        .px(px(8.))
        .text_size(px(13.5))
        .text_color(c::SILVER)
        .cursor_pointer()
        .when(current, |d| d.bg(c::PAPER).text_color(theme::ON_INVERT))
        .when(!current, |d| {
            d.hover(|s| s.bg(theme::BG_HOVER).text_color(theme::TEXT))
                .child(pointer().opacity(0.).group_hover(group, |s| s.opacity(1.)))
        })
        .on_click(
            snap.app
                .listener(move |app, _, _, cx| app.set_settings_tab(tab, cx)),
        )
        .child(tab.icon().size(16.))
        .child(tab.name())
}

impl Render for SettingsPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(snap) = self.app.snapshot(cx) else {
            return div().into_any_element();
        };
        let tab = snap.v.settings_tab;
        if self.shown_tab != Some(tab) {
            // A tab opens at the top, like the rebuilt DOM did.
            self.shown_tab = Some(tab);
            self.scroll.set_offset(point(px(0.), px(0.)));
            #[cfg(debug_assertions)]
            if std::env::var("FITCOM_DEBUG_SCROLL").is_ok_and(|v| v == "bottom") {
                self.scroll.scroll_to_bottom();
            }
        }
        let body = match tab {
            SettingsTab::Account => self.account(&snap, window, cx),
            SettingsTab::Audio => self.audio(&snap, window, cx),
            SettingsTab::Video => self.video(&snap, window, cx),
            SettingsTab::Clips => self.clips(&snap, window, cx),
            SettingsTab::Files => self.files(&snap),
            SettingsTab::Network => self.network(&snap, window, cx),
            SettingsTab::Recap => self.recap(&snap),
        };

        div()
            .size_full()
            .flex()
            .bg(theme::BG_CANVAS)
            .child(self.nav(&snap))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(
                        div()
                            .id("set-body")
                            .size_full()
                            .flex()
                            .flex_col()
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll)
                            .child(
                                div()
                                    .flex_none()
                                    .px(px(32.))
                                    .pt(px(28.))
                                    .pb(px(40.))
                                    .child(body.into_element()),
                            ),
                    )
                    .child(
                        div().absolute().inset_0().child(
                            Scrollbar::new(&self.scroll)
                                .id("set-scrollbar")
                                .axis(ScrollbarAxis::Vertical)
                                .viewport_from_layout(),
                        ),
                    ),
            )
            .into_any_element()
    }
}

/// The text field with the shape of `.text-input`, 240px by default.
fn text_field(input: &Entity<InputState>, width: f32) -> impl IntoElement {
    Input::new(input)
        .w(px(width))
        .h(px(32.))
        .px(px(10.))
        .bg(c::SPACE)
        .text_size(px(13.5))
}

#[cfg(test)]
mod tests {
    use super::should_pull_name;

    #[test]
    fn a_state_event_never_replaces_what_is_being_typed() {
        assert!(
            should_pull_name(None, "Rick", false),
            "first draw fills the input"
        );
        assert!(
            !should_pull_name(None, "Rick", true),
            "typing wins even before the first fill"
        );
        assert!(
            !should_pull_name(Some("Rick"), "Rick", false),
            "nothing changed"
        );
        assert!(
            !should_pull_name(Some("Rick"), "Rik", true),
            "a new name from outside waits for the focus to leave"
        );
        assert!(should_pull_name(Some("Rick"), "Rik", false));
    }
}
