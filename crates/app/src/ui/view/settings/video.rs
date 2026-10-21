//! The Video tab: codec, frame rate, bitrate, and the defaults for what viewers may do on a
//! screen you share: draw (decision 45) and ask for remote control (decision 54). The share
//! picker starts from these and can choose per screen; off here also turns it off on every
//! running share.
//! Every codec/rate change goes out as the whole triple, so the two values it did not touch
//! are the engine's current ones.

use super::controls::{control_row, help, label, note, readout, switch, Body, Piece, Range, Seg};
use super::SettingsPane;
use crate::ui::commands;
use crate::ui::view::format;
use crate::ui::view::widgets::{Btn, BtnKind};
use crate::ui::view::AppSnapshot;
use gpui_kit::{
    div, prelude::FluentBuilder, px, App, ClickEvent, Context, KeyDownEvent, ParentElement, Styled,
    Window,
};
use std::rc::Rc;

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
        let (ui_draw, drawing) = (snap.ui.clone(), snap.s.allow_drawing);
        let draw_toggle =
            Rc::new(move |_: &_, _: &mut _, _: &mut _| commands::set_drawing(&ui_draw, !drawing));
        let (ui_control, control) = (snap.ui.clone(), snap.s.allow_remote_control);
        let control_toggle = Rc::new(move |_: &_, _: &mut _, _: &mut _| {
            commands::set_remote_control(&ui_control, !control)
        });
        // Only screens carry lines; the camera never does.
        let with_lines: Vec<u32> = snap
            .s
            .own_streams
            .iter()
            .filter(|st| st.has_drawing)
            .map(|st| st.stream_id)
            .collect();

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
            .field(
                div()
                    .flex()
                    .items_start()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex_1()
                            .child(label("Allow drawing").mb(px(2.)))
                            .child(help(concat!(
                                "Viewers of a screen you share can draw on it, and everyone ",
                                "watching sees the lines. This is the default for a new share; ",
                                "turning it off also clears and stops it on running shares."
                            )))
                            .when(!with_lines.is_empty(), |d| {
                                let ui = snap.ui.clone();
                                d.child(
                                    control_row().mt(px(8.)).child(
                                        Btn::new("clear-drawing", "Clear drawing")
                                            .kind(BtnKind::Ghost)
                                            .on_click(move |_, _, _| {
                                                for stream in &with_lines {
                                                    commands::clear_drawing(&ui, *stream);
                                                }
                                            }),
                                    ),
                                )
                            }),
                    )
                    .child(switch("allow-drawing", drawing, Some(draw_toggle))),
            )
            .field(if snap.s.remote_control_available {
                div()
                    .flex()
                    .items_start()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex_1()
                            .child(label("Allow remote control").mb(px(2.)))
                            .child(help(concat!(
                                "Viewers of a screen (not a window) you share can ask to use ",
                                "your mouse and keyboard, and you allow each request. While ",
                                "they control it, copied text is shared both ways. This is the ",
                                "default for a new share; turning it off also ends control on ",
                                "running shares."
                            ))),
                    )
                    .child(switch(
                        "allow-remote-control",
                        control,
                        Some(control_toggle),
                    ))
            } else {
                div()
                    .child(label("Remote control").mb(px(2.)))
                    .child(help("Not available on macOS."))
            })
            .field(note(&[Piece::Text(concat!(
                "Desktop audio starts and stops with the share, always. Your own voice is ",
                "excluded, so nobody hears themselves back."
            ))]))
    }
}
