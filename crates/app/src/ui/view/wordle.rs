//! Slice E: the puzzle of the day — the card in the log (`render_card`, called by the
//! timeline for `TimelineItem::Wordle`) and the board inside the Wordle dialog
//! (`render_board`: board, on-screen keyboard, physical keys, standings, time). Typed
//! letters live in `v.wordle_typed` (cleared when `v.wordle_rows` sees a row land); a
//! guess goes out with `commands::wordle_guess`. The solution is never in view state
//! while the game runs: it only appears in `s.wordle.board.solution` once it is done.

use super::super::state::TimelineItem;
use super::super::theme::{self, c};
use super::AppSnapshot;
use gpui_kit::{div, px, AnyElement, App, IntoElement, ParentElement, Styled, Window};

pub fn render_card(_snap: &AppSnapshot, item: &TimelineItem, _window: &mut Window, _cx: &mut App) -> AnyElement {
    let day = match item {
        TimelineItem::Wordle { day, .. } => *day,
        _ => 0,
    };
    div()
        .p(px(10.))
        .bg(c::INK)
        .border_1()
        .border_color(theme::BORDER)
        .text_size(px(12.))
        .child(format!("Wordle card {day} \u{00b7} slice E"))
        .into_any_element()
}

pub fn render_board(snap: &AppSnapshot, _window: &mut Window, _cx: &mut App) -> AnyElement {
    div()
        .p(px(16.))
        .text_size(px(12.))
        .child(format!("Wordle board, day {} \u{00b7} slice E", snap.s.wordle.day))
        .into_any_element()
}
