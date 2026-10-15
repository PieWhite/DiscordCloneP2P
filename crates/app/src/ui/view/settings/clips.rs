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

/// The clip lengths on offer, in seconds. Up to two hours; the buffer on disk holds as
/// much as the clip is long.
const LENGTHS: [u32; 10] = [30, 60, 120, 300, 600, 900, 1800, 3600, 5400, 7200];

/// "30 seconds", "1 minute", "15 minutes".
pub(super) fn length_label(sec: u32) -> String {
    match sec {
        s if !s.is_multiple_of(60) => format!("{s} seconds"),
        60 => "1 minute".into(),
        s => format!("{} minutes", s / 60),
    }
}

/// The other way round, for the select's choice. Accepts any label `length_label` makes,
/// so a length set by hand in `config.toml` survives being re-chosen.
pub(super) fn length_seconds(label: &str) -> Option<u32> {
    let (n, unit) = label.split_once(' ')?;
    let n: u32 = n.parse().ok()?;
    Some(if unit.starts_with("second") {
        n
    } else {
        n * 60
    })
}

/// "30s", "5 min": short enough for the button.
fn length_short(sec: u32) -> String {
    if sec.is_multiple_of(60) {
        format!("{} min", sec / 60)
    } else {
        format!("{sec}s")
    }
}

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

        // A length from `config.toml` that is not on the list still shows, as its own item.
        let mut lengths: Vec<u32> = LENGTHS.to_vec();
        if !lengths.contains(&clips.window_sec) {
            lengths.push(clips.window_sec);
            lengths.sort_unstable();
        }
        let length_items: Vec<String> = lengths.iter().map(|&s| length_label(s)).collect();
        let length_width = auto_width(&length_items, 160.);
        self.clip_length
            .sync(length_items, &length_label(clips.window_sec), window, cx);

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
        let (ui_share, ui_auto, auto_share) = (snap.ui.clone(), snap.ui.clone(), clips.auto_share);
        let auto_toggle = Rc::new(move |_: &_, _: &mut _, _: &mut _| {
            commands::set_clips_auto_share(&ui_auto, !auto_share)
        });

        let mut body = Body::new()
            .h2("Clips")
            .p(format!(
                concat!(
                    "While enabled, a screen is recorded into a rolling buffer that never leaves ",
                    "your machine. Press {} \u{2014} anywhere, also while the app sits in the tray ",
                    "\u{2014} to save the last {} as a playable clip, with game sound and ",
                    "your microphone mixed in."
                ),
                hotkey,
                length_label(clips.window_sec)
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
                    .child(label("Clip length"))
                    .child(help(concat!(
                        "How far back a saved clip goes. The rolling buffer keeps that much on ",
                        "disk: about 90 MB per minute, so 2.7 GB at 30 minutes and 10.8 GB at ",
                        "2 hours. Changing it restarts the recording with an empty buffer."
                    )))
                    .child(control_row().child(select("clips-length", &self.clip_length, length_width, false))),
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
                    .flex()
                    .items_start()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex_1()
                            .child(label("Post clips in the chat automatically").mb(px(2.)))
                            .child(help(concat!(
                                "Only while you are in a voice call. A saved clip is offered in the ",
                                "general channel; nobody downloads it unless they click. Outside a ",
                                "call nothing is posted, but \"Share last clip\" always works."
                            ))),
                    )
                    .child(switch("clips-auto-share", auto_share, Some(auto_toggle))),
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
                                Btn::new("clip-now", format!("Save last {}", length_short(clips.window_sec)))
                                    .kind(BtnKind::Accent)
                                    .icon(Icon::Play)
                                    .disabled(!clips.enabled)
                                    .on_click(move |_, _, _| commands::clip_now(&ui_now)),
                            )
                            .child(
                                Btn::new("clip-share", "Share last clip")
                                    .kind(BtnKind::Ghost)
                                    .icon(Icon::Clip)
                                    .disabled(!clips.can_share)
                                    .on_click(move |_, _, _| commands::share_last_clip(&ui_share)),
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
    fn a_clip_length_label_reads_back_to_its_seconds() {
        for sec in LENGTHS.into_iter().chain([45, 90, 3600]) {
            assert_eq!(length_seconds(&length_label(sec)), Some(sec), "{sec}");
        }
        assert_eq!(length_label(1800), "30 minutes");
        assert_eq!(length_label(7200), "120 minutes");
        assert_eq!(length_label(60), "1 minute");
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
