//! The Video tab: codec, frame rate, bitrate. Every change goes out as the whole triple, so
//! the two values it did not touch are the engine's current ones.

use super::controls::{control_row, help, label, note, readout, Body, Piece, Range, Seg};
use super::SettingsPane;
use crate::ui::commands;
use crate::ui::view::format;
use crate::ui::view::AppSnapshot;
use gpui_kit::{div, App, ClickEvent, Context, KeyDownEvent, ParentElement, Window};

/// One click on a codec or frame-rate button: the whole triple goes to the engine.
fn set_video(
    snap: &AppSnapshot,
    codec: &str,
    fps: u32,
    bitrate: u32,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    let (ui, codec) = (snap.ui.clone(), codec.to_string());
    move |_, _, _| commands::set_video_settings(&ui, &codec, fps, bitrate)
}

impl SettingsPane {
    pub(super) fn video(
        &mut self,
        snap: &AppSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Body {
        let v = &snap.s.video;
        self.bitrate
            .sync(format::mbit(v.bitrate) as f32, window, cx);
        let bitrate_now = self.bitrate.value(cx);

        let set = |codec: &str, fps: u32| set_video(snap, codec, fps, v.bitrate);

        Body::new()
            .h2("Video")
            .p(concat!(
                "Applies to every screen, window and camera you share. A running share restarts ",
                "immediately with the new settings."
            ))
            .field(
                div()
                    .child(label("Codec"))
                    .child(help(concat!(
                        "H.264 always works. HEVC decoding needs a Store extension that is not ",
                        "installed by default, so only pick it if you know both sides have it."
                    )))
                    .child(
                        Seg::new("codec")
                            .option("H.264", v.codec == "h264", set("h264", v.fps))
                            .option("HEVC", v.codec == "hevc", set("hevc", v.fps)),
                    ),
            )
            .field(
                div()
                    .child(label("Frame rate"))
                    .child(help(concat!(
                        "An upper bound, not a promise: only whole divisors of your refresh rate ",
                        "give even motion."
                    )))
                    .child(
                        Seg::new("fps")
                            .option("30", v.fps == 30, set(&v.codec, 30))
                            .option("60", v.fps == 60, set(&v.codec, 60)),
                    ),
            )
            .field(
                div()
                    .child(label("Bitrate"))
                    .child(help(concat!(
                        "12 Mbit/s is the default. Higher caused stutter in the voice of whoever ",
                        "was sharing when a viewer had a weaker connection."
                    )))
                    .child(
                        control_row()
                            .child(Range::new(&self.bitrate, 280.).on_key(cx.listener(
                                |this, ev: &KeyDownEvent, window, cx| {
                                    if let Some(v) = this.bitrate.key_target(ev, cx) {
                                        this.bitrate
                                            .state
                                            .update(cx, |s, cx| s.set_value(v, window, cx));
                                        this.commit_bitrate(v, cx);
                                        cx.stop_propagation();
                                    }
                                },
                            )))
                            .child(readout(format!("{} Mbit/s", bitrate_now.round() as u32))),
                    ),
            )
            .field(note(&[Piece::Text(concat!(
                "Desktop audio starts and stops with the share, always. Your own voice is ",
                "excluded, so nobody hears themselves back."
            ))]))
    }
}
