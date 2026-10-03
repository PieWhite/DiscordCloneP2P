//! The Clips tab: the recording switch, which screen, the global shortcut, the folder, and
//! saving a clip right now.

use super::controls::{
    auto_width, code_inline, control_row, help, input_box, label, select, switch, Body,
};
use super::{SettingsPane, AUTO_MONITOR};
use crate::ui::commands;
use crate::ui::icons::Icon;
use crate::ui::theme::{self, c};
use crate::ui::view::widgets::{Btn, BtnKind};
use crate::ui::view::AppSnapshot;
use gpui_kit::{
    div, prelude::FluentBuilder, px, Context, InteractiveElement, KeyDownEvent, Modifiers,
    ModifiersChangedEvent, ParentElement, StatefulInteractiveElement, Styled, Window,
};
use std::rc::Rc;

/// The modifiers held, in the order the engine's shortcut format writes them. `platform`
/// (Cmd on a Mac, the Windows key on Windows) is `win` on the wire.
fn modifier_names(m: &Modifiers) -> Vec<&'static str> {
    [
        (m.control, "ctrl"),
        (m.alt, "alt"),
        (m.shift, "shift"),
        (m.platform, "win"),
    ]
    .into_iter()
    .filter_map(|(held, name)| held.then_some(name))
    .collect()
}

/// A key press as a shortcut string, or `None` for a key the wire format does not know
/// (arrows, punctuation, a lone modifier): the box keeps waiting instead of saving
/// something unusable. `key` is GPUI's unshifted lower-case key name.
pub(super) fn hotkey_name(m: &Modifiers, key: &str) -> Option<String> {
    let key = key.to_ascii_lowercase();
    let key = key.as_str();
    let function = key
        .strip_prefix('f')
        .and_then(|n| n.parse::<u8>().ok())
        .is_some_and(|n| (1..=24).contains(&n) && key.len() <= 3);
    let single = key.len() == 1
        && key
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit());
    if !(function || single) {
        return None;
    }
    let mut parts = modifier_names(m);
    parts.push(key);
    Some(parts.join("+"))
}

impl SettingsPane {
    pub(super) fn clips(
        &mut self,
        snap: &AppSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Body {
        let Some(clips) = snap.s.clips.as_ref() else {
            return Body::new()
                .h2("Clips")
                .p("Clip recording is not available on this platform.");
        };
        let hotkey = clips.hotkey.to_uppercase();

        // Before the list of screens arrived the select shows just the chosen one.
        let (items, wanted) = match &snap.v.clip_monitors {
            None => {
                let only = clips.monitor.clone().unwrap_or_else(|| "automatic".into());
                (vec![only.clone()], only)
            }
            Some(monitors) => {
                let mut items = vec![AUTO_MONITOR.to_string()];
                items.extend(monitors.iter().cloned());
                let chosen = clips.monitor.as_ref().filter(|m| monitors.contains(m));
                let wanted = chosen.cloned().unwrap_or_else(|| AUTO_MONITOR.to_string());
                (items, wanted)
            }
        };
        let monitor_width = auto_width(&items, 280.);
        self.monitor.sync(items, &wanted, window, cx);

        let waiting = snap.v.hotkey_wait;
        // The debug view sets `hotkey_wait` without a click: hand the box the keyboard.
        #[cfg(debug_assertions)]
        if waiting && snap.v.dialog.is_none() && !self.hotkey_focus.is_focused(window) {
            window.focus(&self.hotkey_focus, cx);
        }
        let shown = match (waiting, self.pending_mods.is_empty()) {
            (false, _) => hotkey.clone(),
            (true, true) => "Press keys\u{2026} Esc cancels".to_string(),
            (true, false) => format!("{}+\u{2026}", self.pending_mods),
        };
        let capture = input_box(
            "clips-hotkey-capture",
            if waiting {
                theme::ACCENT
            } else {
                theme::BORDER_INPUT
            },
        )
        .min_w(px(200.))
        .track_focus(&self.hotkey_focus)
        .cursor_pointer()
        .when(waiting, |d| {
            d.border_color(theme::ACCENT).text_color(c::CYAN)
        })
        .on_click(cx.listener(|this, _, window, cx| {
            this.pending_mods.clear();
            window.focus(&this.hotkey_focus, cx);
            this.app.update(cx, |app, cx| {
                app.v_mut().hotkey_wait = true;
                cx.notify();
            });
        }))
        .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| this.hotkey_pressed(ev, cx)))
        .on_modifiers_changed(cx.listener(|this, ev: &ModifiersChangedEvent, _, cx| {
            this.pending_mods = modifier_names(&ev.modifiers).join("+");
            cx.notify();
        }))
        .child(shown);

        let (ui, enabled) = (snap.ui.clone(), clips.enabled);
        let toggle = Rc::new(move |_: &_, _: &mut _, _: &mut _| commands::set_clips(&ui, !enabled));
        let (ui_pick, ui_now, ui_open) = (snap.ui.clone(), snap.ui.clone(), snap.ui.clone());

        let mut body = Body::new()
            .h2("Clips")
            .p(format!(
                concat!(
                    "While enabled, a screen is recorded into a rolling buffer that never leaves ",
                    "your machine. Press {} \u{2014} anywhere, also while the app sits in the tray ",
                    "\u{2014} to save the last {} seconds as a playable clip, with game sound and ",
                    "your microphone mixed in."
                ),
                hotkey, clips.window_sec
            ))
            .field(
                div()
                    .flex()
                    .items_start()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex_1()
                            .child(label("Record for clips").mb(px(2.)))
                            .child(help(concat!(
                                "Uses the same frame rate and bitrate as screen sharing. Roughly 90 MB ",
                                "of disk per minute; older buffer parts are deleted automatically."
                            ))),
                    )
                    .child(switch("clips-toggle", clips.enabled, Some(toggle))),
            )
            .field(
                div()
                    .child(label("Screen"))
                    .child(help("Which screen gets recorded while the switch is on."))
                    .child(control_row().child(select("clips-monitor", &self.monitor, monitor_width, false))),
            )
            .field(
                div()
                    .child(label("Shortcut"))
                    .child(help(concat!(
                        "Click the box and press the key or combination you want. Esc cancels. ",
                        "Applies immediately, no restart."
                    )))
                    .child(control_row().child(capture)),
            )
            .field(
                div()
                    .child(label("Clips folder"))
                    .child(help(concat!(
                        "Where saved clips land. Clips already saved stay where they are; the ",
                        "rolling buffer moves along and starts empty."
                    )))
                    .child(
                        control_row().child(code_inline(&clips.folder)).child(
                            Btn::new("btn-clips-dir", "Change\u{2026}")
                                .kind(BtnKind::Ghost)
                                .on_click(move |_, _, _| commands::pick_clips_dir(&ui_pick)),
                        ),
                    ),
            )
            .field(
                div()
                    .child(label("Save a clip now"))
                    .child(help(
                        clips.error.clone().unwrap_or_else(|| "Writes the last window to the clips folder.".into()),
                    ))
                    .child(
                        control_row()
                            .child(
                                Btn::new("clip-now", format!("Save last {}s", clips.window_sec))
                                    .kind(BtnKind::Accent)
                                    .icon(Icon::Play)
                                    .disabled(!clips.enabled)
                                    .on_click(move |_, _, _| commands::clip_now(&ui_now)),
                            )
                            .child(
                                Btn::new("open-clips-folder", "Open folder")
                                    .kind(BtnKind::Ghost)
                                    .icon(Icon::Open)
                                    .on_click(move |_, _, _| commands::open_clips_folder(&ui_open)),
                            ),
                    ),
            );
        if let Some(last) = &clips.last_clip {
            body = body.field(
                div()
                    .child(label("Last saved"))
                    .child(control_row().child(code_inline(last))),
            );
        }
        body
    }

    /// While the box listens every key is its own (nothing reaches the rest of the window);
    /// a key the shortcut format knows ends the listening and goes to the engine, which
    /// validates and registers it.
    fn hotkey_pressed(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let Some(snap) = self.app.snapshot(cx) else {
            return;
        };
        if !snap.v.hotkey_wait {
            return;
        }
        cx.stop_propagation();
        let Some(name) = hotkey_name(&ev.keystroke.modifiers, &ev.keystroke.key) else {
            return;
        };
        self.pending_mods.clear();
        commands::set_clip_hotkey(&self.ui, &name);
        self.app.update(cx, |app, cx| {
            app.v_mut().hotkey_wait = false;
            cx.notify();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mods(control: bool, alt: bool, shift: bool, platform: bool) -> Modifiers {
        Modifiers {
            control,
            alt,
            shift,
            platform,
            ..Modifiers::default()
        }
    }

    #[test]
    fn a_shortcut_is_modifiers_in_wire_order_then_one_key() {
        assert_eq!(
            hotkey_name(&mods(true, true, false, false), "c").as_deref(),
            Some("ctrl+alt+c")
        );
        assert_eq!(
            hotkey_name(&mods(false, false, true, true), "7").as_deref(),
            Some("shift+win+7")
        );
        assert_eq!(
            hotkey_name(&mods(false, false, false, false), "f13").as_deref(),
            Some("f13")
        );
    }

    #[test]
    fn keys_the_wire_format_does_not_know_keep_the_box_waiting() {
        let none = mods(false, false, false, false);
        for key in [
            "left", "enter", "space", "-", "f25", "f0", "shift", "f100", "\u{e9}",
        ] {
            assert_eq!(hotkey_name(&none, key), None, "{key}");
        }
    }
}
