//! The Audio tab: devices, the input level, push to talk, the in-game overlay, and the
//! notification sounds.

use super::controls::{
    auto_width, control_row, help, label, note, readout, select, switch, Body, Piece, Range, Seg,
};
use super::{SettingsPane, DEFAULT_DEVICE};
use crate::ui::commands;
use crate::ui::icons::Icon;
use crate::ui::theme::{self, c};
use crate::ui::view::format;
use crate::ui::view::widgets::{meter_view, Btn, BtnKind};
use crate::ui::view::AppSnapshot;
use fitcom_audio::spreektoets;
use gpui_kit::{
    div, prelude::FluentBuilder, px, relative, Context, InteractiveElement, IntoElement,
    KeyDownEvent, ParentElement, SharedString, StatefulInteractiveElement, Styled, Window,
};

/// `deviceSelect`: the options of one device select and which one is chosen. The default
/// comes first, then what the system lists, and a device that is chosen but no longer
/// listed stays as the last option, so unplugging a headset does not silently reset it.
pub(super) fn device_options(listed: &[String], chosen: Option<&str>) -> (Vec<String>, String) {
    let mut items = vec![DEFAULT_DEVICE.to_string()];
    items.extend(listed.iter().cloned());
    if let Some(chosen) = chosen {
        if !listed.iter().any(|d| d == chosen) {
            items.push(chosen.to_string());
        }
    }
    (items, chosen.unwrap_or(DEFAULT_DEVICE).to_string())
}

/// The labels of the key select, in the order of `fitcom_audio::spreektoets::TOETSEN`.
fn ptt_key_labels() -> Vec<String> {
    spreektoets::TOETSEN
        .iter()
        .map(|t| t.label.to_string())
        .collect()
}

/// The config name behind a label of the key select.
pub(super) fn ptt_key_name(label: &str) -> Option<&'static str> {
    spreektoets::TOETSEN
        .iter()
        .find(|t| t.label == label)
        .map(|t| t.naam)
}

impl SettingsPane {
    pub(super) fn audio(
        &mut self,
        snap: &AppSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Body {
        let s = &snap.s;
        let (inputs, outputs) = snap
            .v
            .devices
            .as_ref()
            .map_or((&[][..], &[][..]), |(i, o)| (i.as_slice(), o.as_slice()));
        let (items, wanted) = device_options(inputs, s.input_device.as_deref());
        let mic_width = auto_width(&items, 300.);
        self.mic.sync(items, &wanted, window, cx);
        let (items, wanted) = device_options(outputs, s.output_device.as_deref());
        let speaker_width = auto_width(&items, 300.);
        self.speaker.sync(items, &wanted, window, cx);
        let ptt = &s.push_to_talk;
        let key_label = spreektoets::toets_van_naam(&ptt.key)
            .or_else(|| spreektoets::toets_van_naam(spreektoets::STANDAARD_TOETS))
            .map_or("", |t| t.label);
        self.ptt_key.sync(ptt_key_labels(), key_label, window, cx);
        self.sound_volume
            .sync(format::vol_pct(s.sound.volume) as f32, window, cx);

        let audible = s.sound.volume > 0.;
        let volume_now = self.sound_volume.value(cx);

        let level = meter_view("input-level", &snap.meters, window, cx, |m, _, _| {
            div()
                .relative()
                .w_full()
                .h(px(4.))
                .overflow_hidden()
                .rounded_full()
                .bg(c::LIFT)
                .child(
                    div()
                        .h_full()
                        .w(relative((m.self_level() * 3.).clamp(0., 1.)))
                        .bg(theme::ONLINE),
                )
                .into_any_element()
        });

        let overlay = &s.overlay;
        let (ui_overlay, overlay_on) = (snap.ui.clone(), overlay.enabled);
        let overlay_toggle = std::rc::Rc::new(move |_: &_, _: &mut _, _: &mut _| {
            commands::set_overlay(&ui_overlay, !overlay_on)
        });

        // A mode button sends the new mode with the key the engine has now.
        let mode_button = |mode: &'static str| {
            let (ui, key) = (snap.ui.clone(), ptt.key.clone());
            move |_: &gpui_kit::ClickEvent, _: &mut Window, _: &mut gpui_kit::App| {
                commands::set_push_to_talk(&ui, mode, &key)
            }
        };
        let sets = div()
            .flex()
            .flex_wrap()
            .gap(px(8.))
            .children(s.sound_sets.iter().map(|set| {
                let pressed = s.sound.set == set.id;
                let (ui, id, volume) = (snap.ui.clone(), set.id.clone(), s.sound.volume);
                div()
                    .id(SharedString::from(format!("sound-set-{}", set.id)))
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .min_w(px(168.))
                    .max_w(px(260.))
                    .flex_grow(1.)
                    .flex_shrink(1.)
                    .flex_basis(px(168.))
                    .pl(px(12.))
                    .pr(px(10.))
                    .py(px(10.))
                    .rounded(theme::RADIUS_FIELD)
                    .border_1()
                    .border_color(if pressed {
                        theme::ACCENT
                    } else {
                        theme::BORDER
                    })
                    .when(pressed, |d| d.bg(theme::BG_SELECTED))
                    .when(!pressed, |d| d.hover(|s| s.bg(theme::BG_HOVER)))
                    .cursor_pointer()
                    .on_click(move |_, _, _| commands::set_sound_settings(&ui, &id, volume))
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(gpui_kit::FontWeight::MEDIUM)
                            .text_color(if pressed {
                                theme::TEXT_STRONG
                            } else {
                                theme::TEXT
                            })
                            .child(set.name.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .line_height(relative(1.4))
                            .text_color(theme::TEXT_MUTED)
                            .child(set.description.clone()),
                    )
            }));

        let previews = control_row().children(s.sound_events.iter().map(|event| {
            let (ui, id) = (snap.ui.clone(), event.id.clone());
            Btn::new(
                SharedString::from(format!("preview-{}", event.id)),
                event.name.clone(),
            )
            .kind(BtnKind::Ghost)
            .icon(Icon::Head)
            .disabled(!audible)
            .on_click(move |_, _, _| commands::preview_sound(&ui, &id))
        }));

        Body::new()
            .h2("Audio")
            .p(concat!(
                "Voice runs at 48 kHz with 20 ms frames. Noise suppression is always on. There is ",
                "no echo cancellation, because everyone wears a headset."
            ))
            .field(
                div()
                    .child(label("Microphone"))
                    .child(help(concat!(
                        "Leave on the Windows default unless you need a specific device. Changing it ",
                        "restarts a running call briefly."
                    )))
                    .child(
                        control_row().child(select("set-in", &self.mic, mic_width, true)).child(
                            Btn::new("refresh-devices", "Refresh")
                                .kind(BtnKind::Ghost)
                                .icon(Icon::Retry)
                                .on_click(snap.app.listener(|app, _, _, cx| {
                                    app.refresh_devices();
                                    cx.notify();
                                })),
                        ),
                    ),
            )
            .field(
                div()
                    .child(label("Output"))
                    .child(control_row().child(select("set-out", &self.speaker, speaker_width, true)))
                    .child(note(&[
                        Piece::Text("Set both devices to "),
                        Piece::Chip("48000 Hz"),
                        Piece::Text(concat!(
                            " in Windows. At any other rate the app has to resample, and that costs ",
                            "quality for nothing."
                        )),
                    ])),
            )
            .field(
                div()
                    .child(label("Input level"))
                    .child(help(concat!(
                        "In voice activity mode, voice is only sent while you are actually ",
                        "speaking. When you are quiet, nothing goes over the line."
                    )))
                    .child(div().mt(px(6.)).max_w(px(320.)).child(level)),
            )
            .h2("Push to talk")
            .p(concat!(
                "Operate the microphone with a key instead of (or on top of) voice activity. The ",
                "key works while a game is in front, and the game still receives it too: it is ",
                "read, not taken."
            ))
            .when(!ptt.available, |b| {
                b.field(note(&[Piece::Text(concat!(
                    "Push to talk is not available on macOS yet. The microphone stays on voice ",
                    "activity."
                ))]))
            })
            .when(ptt.available, |b| {
                b.field(
                    div()
                        .child(label("Mode"))
                        .child(help(match ptt.mode.as_str() {
                            "talk" => concat!(
                                "Hold the key to talk. It overrides voice activity, so quiet speech ",
                                "goes through too, and the microphone stays open for a fifth of a ",
                                "second after you let go so the end of your last word is not cut off."
                            ),
                            "mute" => concat!(
                                "Voice activity as usual, but holding the key mutes you at once. ",
                                "Useful to cough or answer someone next to you."
                            ),
                            _ => "Only voice activity decides when you are heard.",
                        }))
                        .child(
                            Seg::new("ptt-mode")
                                .option(
                                    "Voice activity",
                                    ptt.mode == "voice",
                                    mode_button("voice"),
                                )
                                .option(
                                    "Push to talk",
                                    ptt.mode == "talk",
                                    mode_button("talk"),
                                )
                                .option(
                                    "Push to mute",
                                    ptt.mode == "mute",
                                    mode_button("mute"),
                                ),
                        ),
                )
                .field(
                    div()
                        .child(label("Key"))
                        .child(help(concat!(
                            "Right Ctrl is the default. Mouse side buttons work too. A letter key ",
                            "also types in the chat while you hold it, so a modifier, an F-key or a ",
                            "mouse button is the better choice."
                        )))
                        .child(control_row().child(select("set-ptt-key", &self.ptt_key, 300., true))),
                )
            })
            .h2("In-game overlay")
            .when(!overlay.available, |b| {
                b.field(note(&[Piece::Text(
                    "The overlay is not available on macOS.",
                )]))
            })
            .when(overlay.available, |b| {
                b.field(
                    div()
                        .flex()
                        .items_start()
                        .gap(px(12.))
                        .child(
                            div()
                                .flex_1()
                                .child(label("Show who is in the call").mb(px(2.)))
                                .child(help(concat!(
                                    "A small list on top of your game while you are in a call. A ",
                                    "name lights up while that person speaks. Hold Alt and drag ",
                                    "it to move it. It works over borderless and windowed games, ",
                                    "not over exclusive fullscreen, and never shows up in your ",
                                    "own share or clips."
                                ))),
                        )
                        .child(switch("overlay", overlay.enabled, Some(overlay_toggle))),
                )
            })
            .h2("Notification sounds")
            .p(concat!(
                "Short tones when someone joins or leaves the call, or starts and stops sharing. ",
                "The app generates them itself, so they do not go through the call and the others ",
                "never hear them. Do not disturb silences them."
            ))
            .field(
                div()
                    .child(label("Sound set"))
                    .child(help("Picking one plays it, so you hear what you chose."))
                    .child(sets),
            )
            .field(
                div()
                    .child(label("Volume"))
                    .child(help(concat!(
                        "These tones only. Voice, desktop audio and the Windows sound for a message ",
                        "that mentions you are all left alone \u{2014} those follow the Windows ",
                        "volume mixer."
                    )))
                    .child(
                        control_row()
                            .child(Range::new(&self.sound_volume, 280.).on_key(cx.listener(
                                |this, ev: &KeyDownEvent, window, cx| {
                                    if let Some(v) = this.sound_volume.key_target(ev, cx) {
                                        this.sound_volume
                                            .state
                                            .update(cx, |s, cx| s.set_value(v, window, cx));
                                        this.commit_sound_volume(v, cx);
                                        cx.stop_propagation();
                                    }
                                },
                            )))
                            .child(readout(format!("{}%", volume_now.round() as u32))),
                    ),
            )
            .field(
                div()
                    .child(label("Hear them"))
                    .child(help(if audible {
                        "All six, at the volume set above. Ignores do not disturb."
                    } else {
                        "The volume is at zero, so there is nothing to hear. Turn it up first."
                    }))
                    .child(previews),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_chosen_device_that_is_no_longer_listed_stays_selectable() {
        let (items, wanted) = device_options(&names(&["Mic A", "Mic B"]), Some("Mic Gone"));
        assert_eq!(
            items,
            names(&["Windows default", "Mic A", "Mic B", "Mic Gone"])
        );
        assert_eq!(wanted, "Mic Gone");
    }

    #[test]
    fn no_choice_means_the_default_and_a_listed_choice_is_not_doubled() {
        let (items, wanted) = device_options(&names(&["Mic A"]), None);
        assert_eq!(items, names(&["Windows default", "Mic A"]));
        assert_eq!(wanted, "Windows default");
        let (items, wanted) = device_options(&names(&["Mic A"]), Some("Mic A"));
        assert_eq!(items, names(&["Windows default", "Mic A"]));
        assert_eq!(wanted, "Mic A");
    }
}
