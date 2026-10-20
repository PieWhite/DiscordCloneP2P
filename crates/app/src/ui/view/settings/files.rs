//! The Files tab: where downloads and pictures land, and clearing the pictures.

use super::controls::{code_inline, control_row, help, help_rich_units, label, rich, Body, Piece};
use super::SettingsPane;
use crate::ui::commands;
use crate::ui::icons::Icon;
use crate::ui::theme;
use crate::ui::view::widgets::{Btn, BtnKind};
use crate::ui::view::{AppSnapshot, Dialog};
use gpui_kit::{div, px, ParentElement, Styled};

impl SettingsPane {
    pub(super) fn files(&mut self, snap: &AppSnapshot) -> Body {
        let s = &snap.s;
        let ui = snap.ui.clone();
        Body::new()
            .h2("Files")
            .p(rich(
                13.5,
                &[
                    Piece::Text("Shared files land in the folder you pick. Images go to a "),
                    Piece::Mono("Pictures"),
                    Piece::Text(concat!(
                        " folder inside it, named after the content hash, so a thumbnail resolves to ",
                        "the same path for the sender and the receiver."
                    )),
                ],
            ))
            .field(
                div()
                    .child(label("Download folder"))
                    .child(help(concat!(
                        "Already downloaded files stay where they are \u{2014} only new downloads ",
                        "land in the new folder. Images do move along: their path is worked out from ",
                        "the folder, so leaving them behind would drop them from the timeline."
                    )))
                    .child(
                        control_row().child(code_inline(&s.download_dir)).child(
                            Btn::new("btn-download-dir", "Change\u{2026}")
                                .kind(BtnKind::Ghost)
                                .on_click(move |_, _, _| commands::pick_download_dir(&ui)),
                        ),
                    ),
            )
            .field(
                div()
                    .child(label("Images"))
                    .child(help_rich_units(&[
                        Piece::Text("Kept under "),
                        Piece::Mono(&s.pictures_dir),
                        Piece::Text(concat!(
                            ", named after the content hash so both sides resolve the same file. ",
                            "Inside the download folder on purpose: a half-finished download cannot be ",
                            "moved across a drive boundary when it is done."
                        )),
                    ]))
                    .child(
                        control_row()
                            .child(
                                Btn::new("btn-wipe", "Delete all images")
                                    .kind(BtnKind::Danger)
                                    .icon(Icon::Trash)
                                    .on_click(snap.app.listener(|app, _, window, cx| {
                                        app.open_dialog(Dialog::delete_all_images(), window, cx)
                                    })),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(theme::TEXT_MUTED)
                                    .child("Only clears them from this machine."),
                            ),
                    ),
            )
    }
}
