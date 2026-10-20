//! Slice E: the puzzle of the day. The card in the log (`render_card`, called by the
//! timeline for `TimelineItem::Wordle`) and the inside of the Wordle dialog (`render_panel`:
//! board, status line, on-screen keyboard, standings), plus the one function every key goes
//! through (`press`), physical or drawn.
//!
//! Typed letters live in `v.wordle_typed`. A row that lands clears them, a rejected guess
//! keeps them (`typed_row`, `apply_key`); the old `renderWordle` did that while drawing, which
//! a GPUI render may not. The solution is never in view state while the game runs: it only
//! appears in `s.wordle.board.solution` once it is done.

use super::super::commands;
use super::super::icons::Icon;
use super::super::state::{TimelineItem, WordleBoard, WordleScore, WordleState};
use super::super::theme::{self, c};
use super::format;
use super::widgets::{Btn, BtnKind};
use super::{AppSnapshot, AppView, Dialog, ViewState};
use gpui_kit::{
    div, prelude::FluentBuilder, px, relative, AnyElement, App, ClickEvent, Context, Div,
    HighlightStyle, InteractiveElement, IntoElement, KeyDownEvent, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, StyledText, Window,
};
use std::collections::HashMap;

const KEYS: [&str; 3] = ["qwertyuiop", "asdfghjkl", "zxcvbnm"];
const WORD_LEN: usize = 5;

// ------------------------------------------------------------------ keys

/// One press on the board, from the physical keyboard or the drawn one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WordleKey {
    Letter(char),
    Enter,
    Back,
}

impl WordleKey {
    /// The physical keys the board takes: a letter, Enter, Backspace, never with Ctrl, Alt or
    /// Cmd held (Shift is fine), as in the old document keydown handler.
    pub fn from_key_down(ev: &KeyDownEvent) -> Option<Self> {
        let m = &ev.keystroke.modifiers;
        if m.control || m.alt || m.platform {
            return None;
        }
        match ev.keystroke.key.as_str() {
            "enter" => Some(WordleKey::Enter),
            "backspace" => Some(WordleKey::Back),
            k => {
                let mut chars = k.chars();
                match (chars.next(), chars.next()) {
                    (Some(ch), None) if ch.is_ascii_alphabetic() => {
                        Some(WordleKey::Letter(ch.to_ascii_lowercase()))
                    }
                    _ => None,
                }
            }
        }
    }
}

/// The letters the live row shows: what was typed, unless a row has landed since it was
/// typed. `None` (never reconciled) counts as current, so a fresh window shows its text.
pub fn typed_row(v: &ViewState, landed: usize) -> &str {
    if v.wordle_rows.is_none_or(|rows| rows == landed) {
        &v.wordle_typed
    } else {
        ""
    }
}

/// `wordleKey` on plain data: reconciles with the landed row count first (a landed row
/// clears the typed letters), then applies `key`. Returns the guess to send, if any, and
/// whether the typed letters changed.
pub fn apply_key(
    typed: &mut String,
    rows: &mut Option<usize>,
    landed: usize,
    key: WordleKey,
) -> (Option<String>, bool) {
    let mut changed = false;
    if *rows != Some(landed) {
        if rows.is_some() && !typed.is_empty() {
            typed.clear();
            changed = true;
        }
        *rows = Some(landed);
    }
    match key {
        WordleKey::Enter => (Some(typed.clone()).filter(|t| t.len() == WORD_LEN), changed),
        WordleKey::Back => (None, typed.pop().is_some() || changed),
        WordleKey::Letter(ch) if typed.len() < WORD_LEN => {
            typed.push(ch);
            (None, true)
        }
        WordleKey::Letter(_) => (None, changed),
    }
}

/// A key on the board. Nothing happens without a running game.
pub fn press(app: &mut AppView, key: WordleKey, cx: &mut Context<AppView>) {
    let landed = match &app.state().wordle.board {
        Some(board) if !board.done => board.rows.len(),
        _ => return,
    };
    let ui = app.ui().clone();
    let v = app.v_mut();
    let (guess, changed) = apply_key(&mut v.wordle_typed, &mut v.wordle_rows, landed, key);
    if let Some(word) = guess {
        commands::wordle_guess(&ui, &word);
    }
    if changed {
        cx.notify();
    }
}

// ------------------------------------------------------------------ the card

/// `wordleTitle`: `Wordle 1,932`, or bare `Wordle` for a day this machine did not fetch.
fn card_title(number: Option<u32>) -> String {
    match number {
        Some(n) if n > 0 => format!("Wordle {}", format::fmt_thousands(n)),
        _ => "Wordle".into(),
    }
}

/// `wordleResult`: `4/6 · 1:23`, `failed · 4:05`, or without a time when the peer sent none.
fn result_line(r: &WordleScore, tries: u8) -> String {
    let score = if r.solved {
        format!("{}/{}", r.guesses, tries)
    } else {
        "failed".into()
    };
    match r.seconds {
        Some(sec) => format!("{score} \u{00b7} {}", format::fmt_clock(sec)),
        None => score,
    }
}

/// `wordleGrid`: the shared squares, five to a row, one row per guess; nothing for a
/// pattern shorter than one row.
fn pips(pattern: &str) -> Option<Div> {
    let cells: Vec<u8> = pattern
        .bytes()
        .filter(|b| matches!(b, b'0'..=b'2'))
        .map(|b| b - b'0')
        .collect();
    if cells.len() < WORD_LEN {
        return None;
    }
    let rows = cells.chunks(WORD_LEN).map(|row| {
        div().flex().gap(px(2.)).children(
            row.iter()
                .map(|m| div().size(px(8.)).rounded(px(2.)).bg(mark_colour(*m))),
        )
    });
    // 6px margin plus the 3px the old inline-grid gained from sitting on a line box.
    Some(
        div()
            .mt(px(9.))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(2.))
            .children(rows),
    )
}

fn mark_colour(m: u8) -> gpui_kit::Rgba {
    match m {
        2 => theme::WDL_HIT,
        1 => theme::WDL_NEAR,
        _ => theme::WDL_MISS,
    }
}

/// The letter on a marked square: void on green and amber, white on the hairline miss.
fn on_mark(m: u8) -> gpui_kit::Rgba {
    if m == 0 {
        theme::TEXT_STRONG
    } else {
        c::VOID
    }
}

/// The card's one button (`wordleButton`). Every action that is not "waiting" opens the
/// dialog, whatever day the card is: the dialog always shows today.
fn card_button(snap: &AppSnapshot, day: u32, action: &str, progress: u8) -> Btn {
    let tries = snap.s.wordle.tries;
    let open = snap
        .app
        .listener(|app, _: &ClickEvent, window, cx| app.open_dialog(Dialog::Wordle, window, cx));
    let btn = |label: String, icon: Icon, kind: BtnKind| {
        Btn::new(("wdl-btn", day), label).icon(icon).kind(kind)
    };
    match action {
        "play" => btn("Play today's puzzle".into(), Icon::Play, BtnKind::Primary).on_click(open),
        "continue" => btn(
            format!("Continue \u{00b7} {progress}/{tries}"),
            Icon::Play,
            BtnKind::Primary,
        )
        .on_click(open),
        "done" => btn("Show the board".into(), Icon::Tiles, BtnKind::Ghost).on_click(open),
        "waiting" => btn(
            "Today's word has not arrived yet".into(),
            Icon::Tiles,
            BtnKind::Plain,
        )
        .disabled(true),
        _ => btn("Standings".into(), Icon::Tiles, BtnKind::Ghost).on_click(open),
    }
}

/// The puzzle-of-the-day card in the log (`wordleCard`). Today's card leads with its
/// button; a past day leads with the scores and keeps a small Standings button under them.
/// The others' squares stay hidden until your own game of today is over.
pub fn render_card(
    snap: &AppSnapshot,
    item: &TimelineItem,
    _window: &mut Window,
    _cx: &mut App,
) -> AnyElement {
    let TimelineItem::Wordle {
        day,
        at,
        number,
        action,
        progress,
        results,
        scored,
    } = item
    else {
        return div().into_any_element();
    };
    let (day, tries) = (*day, snap.s.wordle.tries);
    let past = action == "past";
    let spoilers = past || action == "done";

    let scores = (!results.is_empty()).then(|| {
        div().flex().gap(px(6.)).children(results.iter().map(|r| {
            let won = r.won;
            div()
                .flex_1()
                .min_w_0()
                .p(px(6.))
                .rounded(theme::RADIUS_FIELD)
                .border_1()
                .border_color(if won { theme::ONLINE } else { theme::BORDER })
                .when(won, |d| d.bg(theme::BG_SELECTED))
                .text_color(if won { theme::TEXT_STRONG } else { theme::TEXT })
                .flex()
                .flex_col()
                .items_center()
                .text_size(px(11.5))
                .child(div().w_full().text_center().truncate().child(if r.mine {
                    "You".to_string()
                } else {
                    r.name.clone()
                }))
                .child(
                    div()
                        .mt(px(1.))
                        .font_family(theme::MONO_FONT)
                        .text_color(theme::TEXT_MUTED)
                        .child(result_line(r, tries)),
                )
                .when_some(spoilers.then(|| pips(&r.pattern)).flatten(), |d, grid| {
                    d.child(grid)
                })
        }))
    });

    let note: Option<String> = if results.is_empty() {
        Some(
            if past {
                "Nobody played this one."
            } else {
                "Nobody has played yet today."
            }
            .into(),
        )
    } else if !scored {
        let first = &results[0];
        let who = if first.mine {
            "You".to_string()
        } else {
            first.name.clone()
        };
        Some(format!(
            "{who} played alone, so this day is worth no point yet. It counts as soon as somebody else joins in."
        ))
    } else if !spoilers {
        Some("The squares appear once you have finished your own game.".into())
    } else {
        None
    };

    let head = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(14.))
        .py(px(8.))
        .border_b_1()
        .border_color(theme::BORDER)
        .child(div().text_color(theme::ACCENT).child(Icon::Tiles.size(14.)))
        .child(
            div()
                .text_size(px(13.))
                .font_weight(gpui_kit::FontWeight::MEDIUM)
                .text_color(theme::TEXT_STRONG)
                .child(card_title(*number)),
        )
        .child(
            div()
                .ml_auto()
                .font_family(theme::MONO_FONT)
                .text_size(theme::LABEL_TEXT)
                .text_color(theme::TEXT_MUTED)
                .child(format::fmt_day(*at)),
        );

    let body = div()
        .p(px(12.))
        .flex()
        .flex_col()
        .when(!past, |d| {
            d.child(card_button(snap, day, action, *progress).full_width())
        })
        .when_some(scores, |d, scores| {
            d.child(scores.when(!past, |s| s.mt(px(10.))))
        })
        .when_some(note, |d, note| {
            d.child(
                div()
                    .mt(px(8.))
                    .text_size(px(11.5))
                    .text_color(theme::TEXT_MUTED)
                    .line_height(relative(1.45))
                    .child(note),
            )
        })
        .when(past, |d| {
            d.child(
                div()
                    .mt(px(8.))
                    .flex()
                    .child(card_button(snap, day, action, *progress)),
            )
        });

    div()
        .id(("wdl-card", day))
        .mt(px(6.))
        .mb(px(2.))
        .max_w(px(440.))
        .rounded(theme::RADIUS_CARD)
        .overflow_hidden()
        .border_1()
        .border_color(theme::BORDER)
        .child(head)
        .child(body)
        .into_any_element()
}

// ------------------------------------------------------------------ the dialog

/// The dialog's title: `Wordle 1,932`, bare `Wordle` while the word has not arrived.
pub fn dialog_title(w: &WordleState) -> String {
    match &w.board {
        Some(board) => format!("Wordle {}", format::fmt_thousands(board.number)),
        None => "Wordle".into(),
    }
}

/// `wordleLetters`: the best mark each letter has earned so far.
fn best_marks(board: Option<&WordleBoard>) -> HashMap<char, u8> {
    let mut best: HashMap<char, u8> = HashMap::new();
    for row in board.map(|b| b.rows.as_slice()).unwrap_or_default() {
        for (i, ch) in row.word.to_lowercase().chars().enumerate() {
            let m = row.marks.get(i).copied().unwrap_or(0);
            best.entry(ch).and_modify(|b| *b = (*b).max(m)).or_insert(m);
        }
    }
    best
}

/// A board cell: empty, typed (iron edge) or marked (a filled square, the colour is the
/// answer).
fn cell(letter: String, mark: Option<u8>, live: bool) -> Div {
    let base = div()
        .size(px(46.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(theme::RADIUS_CONTROL)
        .border_1()
        .text_size(px(20.))
        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
        .line_height(px(20.));
    match mark {
        Some(m) => base
            .bg(mark_colour(m))
            .border_color(mark_colour(m))
            .text_color(on_mark(m))
            .child(letter),
        None => base
            .bg(c::VOID)
            .border_color(if live {
                theme::BORDER_HOVER
            } else {
                theme::BORDER
            })
            .text_color(theme::TEXT)
            .child(letter),
    }
}

fn board(w: &WordleState, typed: &str) -> Div {
    let board = w.board.as_ref();
    let rows = board.map(|b| b.rows.as_slice()).unwrap_or_default();
    let running = board.is_some_and(|b| !b.done);
    let live: Vec<char> = if running {
        typed.chars().collect()
    } else {
        Vec::new()
    };
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(5.))
        .children((0..w.tries as usize).map(|r| {
            div().flex().gap(px(5.)).children((0..WORD_LEN).map(|col| {
                if let Some(row) = rows.get(r) {
                    let letter = row
                        .word
                        .chars()
                        .nth(col)
                        .map(String::from)
                        .unwrap_or_default();
                    cell(
                        letter,
                        Some(row.marks.get(col).copied().unwrap_or(0)),
                        false,
                    )
                } else if r == rows.len() && running {
                    let letter = live
                        .get(col)
                        .map(|ch| ch.to_ascii_uppercase().to_string())
                        .unwrap_or_default();
                    cell(letter, None, true)
                } else {
                    cell(String::new(), None, false)
                }
            }))
        }))
}

/// One `.wdl-line` paragraph, optionally with one stretch in white (the `<b>` of the old
/// markup).
fn line(text: String, bold: Option<std::ops::Range<usize>>, error: bool) -> Div {
    let styled = StyledText::new(text).with_highlights(bold.map(|range| {
        (
            range,
            HighlightStyle {
                color: Some(theme::TEXT_STRONG.into()),
                ..Default::default()
            },
        )
    }));
    div()
        .mt(px(12.))
        .min_h(px(18.))
        .text_size(px(12.5))
        .text_center()
        .text_color(if error {
            theme::DANGER_TEXT
        } else {
            theme::TEXT_MUTED
        })
        .child(styled)
}

/// `wordleStatus`.
fn status(w: &WordleState) -> Vec<Div> {
    let Some(board) = &w.board else {
        let mut lines = Vec::new();
        if let Some(e) = &w.error {
            lines.push(line(e.clone(), None, true));
        }
        let text = "Today's word could not be fetched yet. It is tried again every quarter of an hour; without it there is nothing to guess. + beside the message box tries now and puts the card in the chat for everyone.";
        let plus = text.find("+ beside").map(|i| i..i + 1);
        lines.push(line(text.into(), plus, false));
        return lines;
    };
    if board.done && board.won {
        let clock = board
            .seconds
            .map(|s| format!(", in {}", format::fmt_clock(s)))
            .unwrap_or_default();
        return vec![line(
            format!("Solved in {} of {}{clock}.", board.rows.len(), w.tries),
            None,
            false,
        )];
    }
    if board.done {
        let solution = board.solution.clone().unwrap_or_default();
        let prefix = "Out of guesses. The word was ";
        let range = prefix.len()..prefix.len() + solution.len();
        return vec![line(format!("{prefix}{solution}."), Some(range), false)];
    }
    if let Some(e) = &w.error {
        return vec![line(e.clone(), None, true)];
    }
    vec![line(
        format!("Guess {} of {}.", board.rows.len() + 1, w.tries),
        None,
        false,
    )]
}

/// `wordleKeys`: the on-screen keyboard, only while a game runs.
fn keyboard(snap: &AppSnapshot, w: &WordleState) -> Option<Div> {
    let board = w.board.as_ref().filter(|b| !b.done)?;
    let best = best_marks(Some(board));
    let key = |id: SharedString, label: String, mark: Option<u8>, wide: bool, k: WordleKey| {
        let d = div()
            .id(id)
            .relative()
            .min_w(px(if wide { 46. } else { 26. }))
            .h(px(34.))
            .px(px(4.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(theme::RADIUS_CONTROL)
            .border_1()
            .text_size(px(if wide { 11. } else { 12.5 }))
            .cursor_pointer()
            .on_click(
                snap.app
                    .listener(move |app, _: &ClickEvent, _, cx| press(app, k, cx)),
            );
        let d = match mark {
            // A ruled-out letter stays readable: a hairline fill with dim glyphs.
            Some(0) => d
                .bg(theme::WDL_MISS)
                .border_color(theme::WDL_MISS)
                .text_color(theme::TEXT_DIM),
            Some(m) => d
                .bg(mark_colour(m))
                .border_color(mark_colour(m))
                .text_color(on_mark(m)),
            None => d
                .bg(c::VOID)
                .border_color(theme::BORDER)
                .text_color(theme::TEXT),
        };
        d.hover(|s| s.border_color(theme::TEXT_STRONG))
            .child(div().relative().child(label.to_uppercase()))
    };
    let rows = KEYS.iter().enumerate().map(|(i, row)| {
        div()
            .flex()
            .gap(px(4.))
            .when(i == 2, |d| {
                d.child(key(
                    "wkey-enter".into(),
                    "Enter".into(),
                    None,
                    true,
                    WordleKey::Enter,
                ))
            })
            .children(row.chars().map(|ch| {
                key(
                    format!("wkey-{ch}").into(),
                    ch.to_string(),
                    best.get(&ch).copied(),
                    false,
                    WordleKey::Letter(ch),
                )
            }))
            .when(i == 2, |d| {
                d.child(key(
                    "wkey-back".into(),
                    "Back".into(),
                    None,
                    true,
                    WordleKey::Back,
                ))
            })
    });
    Some(
        div()
            .mt(px(12.))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(4.))
            .children(rows),
    )
}

/// `wordleStandings`.
fn standings(w: &WordleState) -> Div {
    let wrap = div()
        .mt(px(16.))
        .pt(px(10.))
        .border_t_1()
        .border_color(theme::BORDER)
        .child(
            div()
                .mb(px(6.))
                .text_size(px(11.))
                .text_color(theme::TEXT_MUTED)
                .child("STANDINGS"),
        );
    if w.standings.is_empty() {
        return wrap.child(
            div()
                .my(px(12.))
                .text_size(px(12.))
                .text_color(theme::TEXT_MUTED)
                .line_height(relative(1.5))
                .child(
                    "Nobody has finished a puzzle yet. A day is worth one point to whoever solved it in the fewest guesses, and on a tie to whoever was quickest \u{2014} and only when at least two of you played.",
                ),
        );
    }
    let row = |name: String, pts: String, played: String, solved: String| {
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(6.))
            .py(px(4.))
            .child(div().flex_1().min_w_0().child(name))
            .child(div().min_w(px(26.)).flex().justify_end().child(pts))
            .child(
                div()
                    .min_w(px(46.))
                    .flex()
                    .justify_end()
                    .text_color(theme::TEXT_MUTED)
                    .child(played),
            )
            .child(
                div()
                    .min_w(px(46.))
                    .flex()
                    .justify_end()
                    .text_color(theme::TEXT_MUTED)
                    .child(solved),
            )
    };
    wrap.child(
        row(
            "NAME".into(),
            "PTS".into(),
            "PLAYED".into(),
            "SOLVED".into(),
        )
        .text_size(px(10.5))
        .text_color(theme::TEXT_MUTED),
    )
    .children(w.standings.iter().map(|st| {
        row(
            if st.mine {
                "You".into()
            } else {
                st.name.clone()
            },
            st.points.to_string(),
            st.played.to_string(),
            st.solved.to_string(),
        )
        .text_size(px(12.))
        .when(st.mine, |d| d.bg(theme::BG_HOVER))
    }))
}

/// The scrolling middle of the Wordle dialog: board, status line, keyboard, standings.
pub fn render_panel(snap: &AppSnapshot, _window: &mut Window, _cx: &mut App) -> AnyElement {
    let w = &snap.s.wordle;
    let landed = w.board.as_ref().map_or(0, |b| b.rows.len());
    let typed = typed_row(&snap.v, landed);
    div()
        .flex()
        .flex_col()
        .child(board(w, typed))
        .children(status(w))
        .children(keyboard(snap, w))
        .child(standings(w))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(
        typed: &mut String,
        rows: &mut Option<usize>,
        landed: usize,
        ks: &str,
    ) -> Option<String> {
        let mut sent = None;
        for ch in ks.chars() {
            let k = match ch {
                '>' => WordleKey::Enter,
                '<' => WordleKey::Back,
                ch => WordleKey::Letter(ch),
            };
            sent = apply_key(typed, rows, landed, k).0.or(sent);
        }
        sent
    }

    /// Five letters at most, Back removes one, Enter sends only a full row. Making Enter
    /// send a short word, or dropping the length cap, fails it.
    #[test]
    fn a_row_takes_five_letters_and_enter_sends_only_a_full_row() {
        let (mut typed, mut rows) = (String::new(), None);
        assert_eq!(keys(&mut typed, &mut rows, 2, "cra>"), None);
        assert_eq!(keys(&mut typed, &mut rows, 2, "nxx<es"), None);
        assert_eq!(typed, "crane");
        assert_eq!(keys(&mut typed, &mut rows, 2, ">"), Some("crane".into()));
    }

    /// A row that landed clears what was typed; a rejected guess (same row count) keeps it
    /// so a typo can be fixed. Removing the reconcile step fails the first half.
    #[test]
    fn a_landed_row_clears_the_typed_letters_and_a_rejected_guess_keeps_them() {
        let (mut typed, mut rows) = ("cranx".to_string(), Some(2));
        let mut v = ViewState {
            wordle_typed: typed.clone(),
            wordle_rows: Some(2),
            ..Default::default()
        };
        assert_eq!(typed_row(&v, 2), "cranx", "rejected: still the same row");
        assert_eq!(typed_row(&v, 3), "", "a row landed");

        keys(&mut typed, &mut rows, 3, "s");
        assert_eq!(typed, "s");
        assert_eq!(rows, Some(3));

        v.wordle_rows = None;
        assert_eq!(
            typed_row(&v, 3),
            "cranx",
            "never reconciled counts as current"
        );
    }
}
