//! The Recap tab: the last week, counted on this machine.

use super::controls::{control_row, help, label, rich, Body, Piece};
use super::SettingsPane;
use crate::ui::icons::Icon;
use crate::ui::state::RecapRow;
use crate::ui::theme;
use crate::ui::view::format;
use crate::ui::view::widgets::{avatar, face, Btn, BtnKind};
use crate::ui::view::AppSnapshot;
use crate::ui::Ui;
use gpui_kit::{div, prelude::FluentBuilder, px, Div, ParentElement, Styled};

/// The second line of a recap row: everything except the call time, which is the figure on
/// the right. Zeroes are left out rather than printed: "0 files" is noise.
pub(super) fn recap_line(r: &RecapRow) -> String {
    let plural = |n: u32, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    let mut bits = Vec::new();
    if r.messages > 0 {
        bits.push(plural(r.messages, "message", "messages"));
    }
    if r.files > 0 {
        bits.push(plural(r.files, "file", "files"));
    }
    if r.shared_sec > 0 {
        bits.push(format!("{} sharing", format::fmt_span(r.shared_sec)));
    }
    if r.wordle_played > 0 {
        let mut wordle = format!("Wordle {}/{}", r.wordle_solved, r.wordle_played);
        if r.wordle_points > 0 {
            wordle += &format!(", {}", plural(r.wordle_points, "point", "points"));
        }
        bits.push(wordle);
    }
    bits.join(" \u{b7} ")
}

impl SettingsPane {
    pub(super) fn recap(&mut self, snap: &AppSnapshot) -> Body {
        let Some(recap) = snap.v.recap.as_ref() else {
            return Body::new().h2("Recap").p("Working out the week\u{2026}");
        };
        let since = match format::fmt_day_num(recap.from) {
            day if day.is_empty() => String::new(),
            day => format!(", since {day}"),
        };
        let mut body = Body::new().h2("Recap").p(format!(
            concat!(
                "The last {} days{}. Counted on this machine and kept here \u{2014} nothing about ",
                "your week is sent to the others. Time someone spent in a call while you were ",
                "offline is not in these numbers, because this app was not there to see it."
            ),
            recap.days, since
        ));
        if let Some(mine) = recap.rows.iter().find(|r| r.mine && r.voice_sec > 0) {
            body = body.p(rich(
                13.5,
                &[
                    Piece::Text("You were in the call for "),
                    Piece::Text(&format::fmt_span(mine.voice_sec)),
                    Piece::Text("."),
                ],
            ));
        }

        let everyone = if recap.rows.is_empty() {
            // A bare `<p>` in the old markup: the body size and colour, a 15px gap above.
            div()
                .mt(px(6.))
                .mb(px(15.))
                .child("Nothing yet. The count starts at the first call, message or file.")
        } else {
            div()
                .max_w(px(520.))
                .rounded(theme::RADIUS_FIELD)
                .border_1()
                .border_color(theme::BORDER)
                .overflow_hidden()
                .children(
                    recap
                        .rows
                        .iter()
                        .enumerate()
                        .map(|(i, r)| recap_row(&snap.ui, i, r)),
                )
        };

        body.field(
            div()
                .child(label("Everyone"))
                .child(help(concat!(
                    "Longest in the call first. Screen time counts a shared screen, window or ",
                    "camera \u{2014} not the desktop sound that rides along with it."
                )))
                .child(everyone),
        )
        .field(
            div()
                .child(label("Refresh"))
                .child(help(concat!(
                    "The engine recalculates once a minute, so a call you just left may still be ",
                    "a moment behind."
                )))
                .child(
                    control_row().child(
                        Btn::new("refresh-recap", "Refresh")
                            .kind(BtnKind::Ghost)
                            .icon(Icon::Retry)
                            .on_click(snap.app.listener(|app, _, _, cx| {
                                app.refresh_recap();
                                cx.notify();
                            })),
                    ),
                ),
        )
    }
}

fn recap_row(ui: &Ui, index: usize, r: &RecapRow) -> Div {
    let line = recap_line(r);
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .px(px(12.))
        .py(px(10.))
        .when(index > 0, |d| d.border_t_1().border_color(theme::BORDER))
        .child(avatar(
            &r.name,
            r.avatar,
            face(ui, r.picture.as_deref()),
            24.,
            None,
            false,
        ))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(gpui_kit::FontWeight::MEDIUM)
                        .text_color(theme::TEXT_STRONG)
                        .child(format!("{}{}", r.name, if r.mine { " (you)" } else { "" })),
                )
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(theme::TEXT_MUTED)
                        .child(if line.is_empty() {
                            "quiet week".to_string()
                        } else {
                            line
                        }),
                ),
        )
        .child(div().flex_none().child(format::fmt_span(r.voice_sec)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> RecapRow {
        RecapRow {
            peer: "p".into(),
            name: "Rick".into(),
            avatar: 1,
            picture: None,
            mine: true,
            voice_sec: 0,
            shared_sec: 0,
            messages: 0,
            files: 0,
            wordle_points: 0,
            wordle_played: 0,
            wordle_solved: 0,
        }
    }

    #[test]
    fn zeroes_are_left_out_and_singulars_are_singular() {
        assert_eq!(recap_line(&row()), "");
        let r = RecapRow {
            messages: 1,
            files: 1,
            ..row()
        };
        assert_eq!(recap_line(&r), "1 message \u{b7} 1 file");
    }

    #[test]
    fn a_full_row_reads_like_the_old_tab() {
        let r = RecapRow {
            messages: 64,
            files: 9,
            shared_sec: 2 * 3600 + 5 * 60,
            wordle_played: 7,
            wordle_solved: 7,
            wordle_points: 3,
            ..row()
        };
        assert_eq!(
            recap_line(&r),
            "64 messages \u{b7} 9 files \u{b7} 2h 05m sharing \u{b7} Wordle 7/7, 3 points"
        );
        let one = RecapRow {
            wordle_played: 4,
            wordle_solved: 4,
            wordle_points: 1,
            ..row()
        };
        assert_eq!(recap_line(&one), "Wordle 4/4, 1 point");
    }
}
