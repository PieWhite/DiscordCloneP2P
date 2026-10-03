//! The settings form in the grammar of `app.css`: the window-in-a-window chassis (`.set-inner`
//! with its blue heading strips and horizon band), fields, text pieces with inline code, and
//! the controls (`select`, `.text-input`, `.seg`, `.switch`, `input[type=range]`, `.note`).
//!
//! The slider, switch and segmented control are drawn here and not taken from gpui-kit's
//! styled ones: those animate (a spring on the thumb, a pill shape) and this world snaps.
//! The kit's `Select` and `Input` are used, dressed with the palette.

use crate::ui::icons::Icon;
use crate::ui::theme::{self, c};
use gpui_kit::base::slider::{
    Slider as BaseSlider, SliderEvent, SliderIndicator, SliderState, SliderThumb, SliderTrack,
};
use gpui_kit::component::select::{Select, SelectState};
use gpui_kit::{
    div, font, prelude::FluentBuilder, px, relative, AnyElement, App, AppContext, ClickEvent,
    Context, Div, ElementId, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement,
    KeyDownEvent, ParentElement, Pixels, RenderOnce, Rgba, SharedString, Stateful,
    StatefulInteractiveElement, Styled, StyledText, TextRun, Window,
};
use std::rc::Rc;

/// `max-width: Nch` in the stylesheet. The face's `0` is half an em wide. A browser lets
/// the space after the last word of a line hang past the edge; GPUI counts it, so a line
/// of exactly N glyphs would wrap. One more space of width puts the wraps where the
/// stylesheet's were.
pub fn ch(font_px: f32, n: f32) -> Pixels {
    px(font_px * 0.5 * (n + 1.) + 0.5)
}

// ------------------------------------------------------------------ the chassis

/// The content of one tab: heading strips, paragraphs and fields in `.set-inner`.
#[derive(Default)]
pub struct Body {
    items: Vec<AnyElement>,
    fields: usize,
    /// The last thing added was a paragraph: the next one's top margin collapses into its
    /// bottom margin, as margins do between siblings.
    after_p: bool,
}

impl Body {
    pub fn new() -> Self {
        Self::default()
    }

    /// `.set-body h2`: the blue strip across the whole window. A second one in the same tab
    /// sits 32px lower (`style="margin-top:32px"`).
    pub fn h2(mut self, text: &str) -> Self {
        let lower = !self.items.is_empty();
        self.after_p = false;
        self.items.push(
            div()
                .when(lower, |d| d.mt(px(32.)))
                .px(px(16.))
                .py(px(10.))
                .bg(c::BLUE)
                .border_b_1()
                .border_color(c::SPACE)
                .text_color(c::PAPER)
                .text_size(px(14.))
                .child(text.to_uppercase())
                .into_any_element(),
        );
        self
    }

    /// `.set-body > .set-inner > p`.
    pub fn p(mut self, text: impl IntoElement) -> Self {
        let collapsed = std::mem::replace(&mut self.after_p, true);
        self.items.push(
            div()
                .mx(px(20.))
                .mt(px(if collapsed { 0. } else { 14. }))
                .mb(px(22.))
                .w_full() // GPUI sizes a wrapping text block that has only a max-width a line too tall
                .max_w(ch(13.5, 62.))
                .text_size(px(13.5))
                .text_color(c::SILVER)
                .child(text)
                .into_any_element(),
        );
        self
    }

    /// `.field`: a hairline above every one but the first of the tab.
    pub fn field(mut self, content: impl IntoElement) -> Self {
        self.after_p = false;
        let first = self.fields == 0;
        self.fields += 1;
        self.items.push(
            div()
                .mx(px(20.))
                .py(px(16.))
                .when(!first, |d| d.border_t_1().border_color(theme::BORDER))
                .child(content)
                .into_any_element(),
        );
        self
    }

    /// `.set-inner`: ink ground, violet frame, hard shadow, the band flush along the bottom.
    pub fn into_element(self) -> Div {
        div()
            .w_full()
            .max_w(px(640.))
            .flex()
            .flex_col()
            .bg(c::INK)
            .border_1()
            .border_color(c::VIOLET)
            .shadow(theme::hard_shadow(3.))
            .children(self.items)
            // `::after { margin-top: 20px }` collapses into a paragraph's 22px above it.
            .child(
                div()
                    .mt(px(if self.after_p { 0. } else { 20. }))
                    .child(theme::band(px(4.))),
            )
    }
}

// ------------------------------------------------------------------ text

/// `.field-label`.
pub fn label(text: impl Into<SharedString>) -> Div {
    div()
        .mb(px(3.))
        .text_size(px(13.))
        .text_color(c::PAPER)
        .child(text.into())
}

fn help_box() -> Div {
    div()
        .mb(px(9.))
        .w_full()
        .max_w(ch(12., 58.))
        .text_size(px(12.))
        .line_height(relative(1.5))
        .text_color(c::SILVER)
}

/// `.field-help`.
pub fn help(text: impl Into<SharedString>) -> Div {
    help_box().child(text.into())
}

/// `.field-help` with inline code.
pub fn help_rich(pieces: &[Piece]) -> Div {
    help_box().child(rich(12., pieces))
}

/// `.field-help` with inline code, breaking inside hyphens (see [`rich_units`]).
pub fn help_rich_units(pieces: &[Piece]) -> Div {
    help_box().child(rich_units(12., pieces))
}

/// Text with inline code, drawn as a row of words that wraps (GPUI cannot mix two fonts in
/// one text run).
pub enum Piece<'a> {
    Text(&'a str),
    /// `<code class="mono">`: the mono face, nothing else.
    Mono(&'a str),
    /// `.note code`: a small sky-blue chip on the void.
    Chip(&'a str),
}

/// Text with inline code as one wrapped paragraph: each piece is a run with its own face.
#[derive(IntoElement)]
pub struct Rich {
    runs: Vec<(RunKind, String)>,
}

#[derive(Clone, Copy)]
enum RunKind {
    Text,
    Mono,
    Chip,
}

pub fn rich(_size: f32, pieces: &[Piece]) -> Rich {
    Rich {
        runs: pieces
            .iter()
            .map(|piece| match piece {
                Piece::Text(t) => (RunKind::Text, t.to_string()),
                Piece::Mono(t) => (RunKind::Mono, t.to_string()),
                // The chip's side padding, as hair spaces inside its background (a run has no padding).
                Piece::Chip(t) => (RunKind::Chip, format!("\u{200a}{t}\u{200a}")),
            })
            .collect(),
    }
}

impl RenderOnce for Rich {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let mut text = String::new();
        let mut runs = Vec::with_capacity(self.runs.len());
        for (kind, piece) in &self.runs {
            let (family, color, background) = match kind {
                RunKind::Text => (theme::UI_FONT, c::SILVER, None),
                RunKind::Mono => (theme::MONO_FONT, c::SILVER, None),
                RunKind::Chip => (theme::MONO_FONT, c::SKY, Some(c::SPACE)),
            };
            runs.push(TextRun {
                len: piece.len(),
                font: font(family),
                color: color.into(),
                background_color: background.map(Into::into),
                underline: None,
                strikethrough: None,
            });
            text.push_str(piece);
        }
        StyledText::new(text).with_runs(runs)
    }
}

/// The same text as a row of units that wrap like a browser does at a hyphen. A text node in
/// GPUI is ceiled to whole pixels, so this runs a little wider than `rich`; use it only where
/// a long path has to break inside its hyphens (the Files tab).
pub fn rich_units(size: f32, pieces: &[Piece]) -> Div {
    // The unit of a wrap is a group of runs with no break between them: a code span and the
    // comma that follows it are one unit, a word and its trailing space are one unit.
    let mut words: Vec<(usize, &str)> = Vec::new();
    for (i, piece) in pieces.iter().enumerate() {
        match piece {
            Piece::Text(t) | Piece::Mono(t) => {
                words.extend(break_words(t).into_iter().map(|w| (i, w)))
            }
            Piece::Chip(t) => words.push((i, t)),
        }
    }

    let mut row = div().flex().flex_row().flex_wrap().items_baseline();
    let mut unit: Option<Div> = None;
    let mut unit_closed = true;
    for (i, word) in words {
        if unit_closed {
            if let Some(done) = unit.take() {
                row = row.child(done);
            }
            unit = Some(div().flex().flex_row().whitespace_nowrap().items_baseline());
        }
        let run = match &pieces[i] {
            Piece::Text(_) => div().child(word.to_string()),
            Piece::Mono(_) => div().font_family(theme::MONO_FONT).child(word.to_string()),
            Piece::Chip(_) => div()
                .px(px(4.))
                .py(px(1.))
                .bg(c::SPACE)
                .text_color(c::SKY)
                .font_family(theme::MONO_FONT)
                .text_size(px(size * 0.92))
                .child(word.to_string()),
        };
        unit = unit.map(|u| u.child(run));
        unit_closed = word.ends_with([' ', '-']) && !matches!(&pieces[i], Piece::Chip(_));
    }
    if let Some(done) = unit {
        row = row.child(done);
    }
    row
}

/// The pieces a browser may break a line between: after a space, and after a hyphen that
/// sits between two letters or digits (`half-finished`, `4e4e-4c72`, but not `/-Users`).
pub(super) fn break_words(text: &str) -> Vec<&str> {
    let mut words = Vec::new();
    let mut start = 0;
    let mut previous: Option<char> = None;
    let mut chars = text.char_indices().peekable();
    while let Some((at, ch)) = chars.next() {
        let inside_word = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
        let breaks = ch == ' '
            || (ch == '-'
                && inside_word(previous)
                && inside_word(chars.peek().map(|(_, next)| *next)));
        previous = Some(ch);
        if breaks {
            let end = at + ch.len_utf8();
            words.push(&text[start..end]);
            start = end;
        }
    }
    if start < text.len() {
        words.push(&text[start..]);
    }
    words
}

/// `.control-row`.
pub fn control_row() -> Div {
    div()
        .flex()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(10.))
}

/// `.code-inline`: an identifier or a path in the mono face, on the void.
pub fn code_inline(text: &str) -> Div {
    div()
        .min_w_0()
        .max_w_full()
        .px(px(9.))
        .py(px(6.))
        .bg(c::SPACE)
        .border_1()
        .border_color(theme::BORDER)
        .text_size(px(12.))
        .text_color(c::SKY)
        .font_family(theme::MONO_FONT)
        .child(
            div().flex().flex_row().flex_wrap().children(
                break_words(text)
                    .into_iter()
                    .map(|w| div().whitespace_nowrap().child(w.to_string())),
            ),
        )
}

/// `.readout`.
pub fn readout(text: impl Into<SharedString>) -> Div {
    div()
        .flex_none()
        .text_size(px(13.))
        .text_color(c::PAPER)
        .font_family(theme::MONO_FONT)
        .child(text.into())
}

/// `.note`: an amber alert glyph and a small paragraph in a violet frame.
pub fn note(pieces: &[Piece]) -> Div {
    div()
        .mt(px(10.))
        .w_full()
        .max_w(px(12.5 * 0.5 * 58.))
        .flex()
        .gap(px(9.))
        .px(px(12.))
        .py(px(10.))
        .border_1()
        .border_color(theme::BORDER)
        .bg(c::INK)
        .text_size(px(12.5))
        .line_height(relative(1.5))
        .text_color(c::SILVER)
        .child(
            div()
                .flex_none()
                .mt(px(1.))
                .text_color(theme::CONNECTING)
                .child(Icon::Alert.size(16.)),
        )
        // The space after the last word of a line hangs past the edge in a browser, and the code
        // chip is a run in the text's own size (11.5px in the stylesheet): 12px of slack covers both.
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(div().mr(px(-12.)).child(rich(12.5, pieces))),
        )
}

// ------------------------------------------------------------------ boxes

/// `.text-input` / `select`: 32px, steel frame, void ground; on hover the frame goes to
/// `hover_edge` (silver, or cyan while a box is listening).
pub fn input_box(id: impl Into<ElementId>, hover_edge: Rgba) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .h(px(32.))
        .px(px(10.))
        .bg(c::SPACE)
        .border_1()
        .border_color(theme::BORDER_STRONG)
        .text_size(px(13.5))
        .text_color(theme::TEXT)
        .hover(move |s| s.border_color(hover_edge))
}

// ------------------------------------------------------------------ select

/// One gpui-kit `Select` over a list of strings, with the three rules the old window kept:
/// the option list is replaced only when it really changed, the engine's value is pushed in
/// only while the select is not being used, and a value the engine has but the list lacks
/// is the caller's to put in the list.
pub struct Choices {
    state: Entity<SelectState<Vec<String>>>,
    items: Vec<String>,
}

impl Choices {
    pub fn new<P: 'static>(window: &mut Window, cx: &mut Context<P>) -> Self {
        Choices {
            state: cx.new(|cx| SelectState::new(Vec::<String>::new(), None, window, cx)),
            items: Vec::new(),
        }
    }

    pub fn state(&self) -> &Entity<SelectState<Vec<String>>> {
        &self.state
    }

    /// The label the user sees as chosen.
    pub fn value(&self, cx: &App) -> Option<String> {
        self.state.read(cx).selected_value().cloned()
    }

    /// Make the select show `items` with `wanted` chosen (`wanted` is one of `items`).
    pub fn sync(&mut self, items: Vec<String>, wanted: &str, window: &mut Window, cx: &mut App) {
        let items_changed = self.items != items;
        if items_changed {
            self.state
                .update(cx, |s, cx| s.set_items(items.clone(), window, cx));
            self.items = items;
        }
        let in_use = self
            .state
            .read(cx)
            .focus_handle(cx)
            .contains_focused(window, cx);
        let current = self.value(cx);
        if items_changed || (!in_use && current.as_deref() != Some(wanted)) {
            self.state.update(cx, |s, cx| {
                s.set_selected_value(&wanted.to_string(), window, cx)
            });
        }
    }
}

/// The select, 32px high, `width` wide. The device selects were wrapped with a chevron
/// (`.select-wrap`); the status and screen selects were bare, so theirs is covered.
pub fn select(id: &'static str, choices: &Choices, width: f32, chevron: bool) -> Div {
    div()
        .relative()
        .w(px(width))
        .h(px(32.))
        .flex_none()
        .child(
            Select::new(choices.state())
                .id(id)
                .h(px(32.))
                .px(px(10.))
                .bg(c::SPACE)
                .text_size(px(13.5))
                .icon(Icon::Chev.size(14.)),
        )
        .when(!chevron, |d| {
            d.child(
                div()
                    .absolute()
                    .top(px(1.))
                    .right(px(1.))
                    .bottom(px(1.))
                    .w(px(28.))
                    .bg(c::SPACE),
            )
        })
}

/// A native select is as wide as its longest option; `min_w` is the stylesheet's
/// `min-width`. Padding and chevron take 40px, a glyph of the 13.5px face half its size.
/// ponytail: counts characters, not glyph widths, so a list of wide (CJK) names comes out a
/// little narrow; measuring the shaped text would need the text system here.
pub fn auto_width(items: &[String], min_w: f32) -> f32 {
    let longest = items.iter().map(|i| i.chars().count()).max().unwrap_or(0) as f32;
    min_w.max(40. + longest * 13.5 * 0.5)
}

type Handler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;
type KeyHandler = Rc<dyn Fn(&KeyDownEvent, &mut Window, &mut App)>;

// ------------------------------------------------------------------ switch

/// `.switch`: 38x22, the knob jumps 16px. A switch with no handler is the disabled kind
/// the account tab shows for the two config-file settings.
pub fn switch(id: &'static str, on: bool, on_click: Option<Handler>) -> Stateful<Div> {
    let edge = if on {
        theme::ACCENT
    } else {
        theme::BORDER_STRONG
    };
    div()
        .id(id)
        .relative()
        .flex_none()
        .w(px(38.))
        .h(px(22.))
        .bg(if on { theme::ACCENT } else { c::SPACE })
        .border_1()
        .border_color(edge)
        .child(
            div()
                .absolute()
                .top(px(2.))
                .left(px(if on { 18. } else { 2. }))
                .size(px(16.))
                .bg(if on { c::SPACE } else { c::SILVER }),
        )
        .when_some(on_click, |d, f| {
            d.cursor_pointer()
                .on_click(move |ev, window, cx| f(ev, window, cx))
        })
}

// ------------------------------------------------------------------ segmented control

/// `.seg`: buttons in one frame; the pressed one is inverted.
#[derive(IntoElement)]
pub struct Seg {
    id: &'static str,
    options: Vec<(&'static str, bool, Handler)>,
}

impl Seg {
    pub fn new(id: &'static str) -> Self {
        Seg {
            id,
            options: Vec::new(),
        }
    }

    pub fn option(
        mut self,
        label: &'static str,
        pressed: bool,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.options.push((label, pressed, Rc::new(on_click)));
        self
    }
}

impl RenderOnce for Seg {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let last = self.options.len().saturating_sub(1);
        let id = self.id;
        // `display: inline-flex`: as wide as its buttons, not as wide as its field.
        div().flex().child(
            div()
                .flex()
                .flex_none()
                .border_1()
                .border_color(theme::BORDER_STRONG)
                .children(
                    self.options
                        .into_iter()
                        .enumerate()
                        .map(|(i, (label, pressed, f))| {
                            div()
                                .id((id, i))
                                .h(px(30.))
                                .px(px(14.))
                                .flex()
                                .items_center()
                                .text_size(px(13.))
                                .text_color(c::SILVER)
                                .cursor_pointer()
                                .when(i < last, |d| {
                                    d.border_r_1().border_color(theme::BORDER_STRONG)
                                })
                                .when(pressed, |d| d.bg(c::PAPER).text_color(theme::ON_INVERT))
                                .when(!pressed, |d| {
                                    d.hover(|s| s.bg(theme::BG_HOVER).text_color(theme::TEXT))
                                })
                                .on_click(move |ev, window, cx| f(ev, window, cx))
                                .child(label)
                        }),
                ),
        )
    }
}

// ------------------------------------------------------------------ range

/// The engine's value goes into a slider unless the user has it in hand, or it is the value
/// that was pushed last (the echo of our own change must not move the thumb again).
fn should_push(dragging: bool, pushed: Option<f32>, engine: f32) -> bool {
    !dragging && pushed != Some(engine)
}

/// A slider's state plus the two facts the old window got from the DOM: whether a drag is
/// under way (the engine's value must not pull the thumb out of the user's hand) and the
/// engine value last pushed in (so the echo of our own change does not bounce the thumb).
pub struct Ranged {
    pub state: Entity<SliderState>,
    pub focus: FocusHandle,
    dragging: bool,
    pushed: Option<f32>,
}

impl Ranged {
    pub fn new<P: 'static>(min: f32, max: f32, cx: &mut Context<P>) -> Self {
        Ranged {
            state: cx.new(|_| SliderState::new().min(min).max(max).step(1.)),
            focus: cx.focus_handle().tab_stop(true),
            dragging: false,
            pushed: None,
        }
    }

    pub fn value(&self, cx: &App) -> f32 {
        self.state.read(cx).value().end()
    }

    /// The engine's value; applied when it changed and nobody is dragging.
    pub fn sync(&mut self, engine: f32, window: &mut Window, cx: &mut App) {
        if !should_push(self.dragging, self.pushed, engine) {
            return;
        }
        self.pushed = Some(engine);
        self.state
            .update(cx, |s, cx| s.set_value(engine, window, cx));
    }

    /// A drag step or its end. Returns the value to send to the engine, once, on release;
    /// the next state is then pushed in again, so a value the engine did not take is not
    /// left on the slider.
    pub fn event(&mut self, ev: &SliderEvent) -> Option<f32> {
        match ev {
            SliderEvent::Change(_) => {
                self.dragging = true;
                None
            }
            SliderEvent::Release(v) => {
                self.dragging = false;
                self.pushed = None;
                Some(v.end())
            }
        }
    }

    /// What the arrow keys, Home/End and Page keys of a native range input do.
    pub fn key_target(&self, event: &KeyDownEvent, cx: &App) -> Option<f32> {
        let state = self.state.read(cx);
        let (min, max, step) = (state.min_value(), state.max_value(), state.step_value());
        let now = state.value().end();
        let page = ((max - min) / 10.).max(step);
        let next = match event.keystroke.key.as_str() {
            "left" | "down" => now - step,
            "right" | "up" => now + step,
            "pagedown" => now - page,
            "pageup" => now + page,
            "home" => min,
            "end" => max,
            _ => return None,
        };
        Some(next.clamp(min, max))
    }
}

/// `input[type=range]`: a 6px well with a cyan fill up to the thumb, a 10x14 paper thumb
/// with a hard shadow, a ring on the thumb for keyboard focus. Takes the width the row
/// leaves it, at most `max_w`.
#[derive(IntoElement)]
pub struct Range {
    state: Entity<SliderState>,
    focus: FocusHandle,
    max_w: f32,
    on_key: Option<KeyHandler>,
}

impl Range {
    pub fn new(ranged: &Ranged, max_w: f32) -> Self {
        Range {
            state: ranged.state.clone(),
            focus: ranged.focus.clone(),
            max_w,
            on_key: None,
        }
    }

    pub fn on_key(mut self, f: impl Fn(&KeyDownEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_key = Some(Rc::new(f));
        self
    }
}

const THUMB_W: f32 = 10.;

impl RenderOnce for Range {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let pct = self.state.read(cx).percentage().end;
        let ring = self.focus.is_focused(window) && window.last_input_was_keyboard();
        // The native thumb stays inside the track: at 100% its right edge is the track's.
        let thumb_left = relative(pct);
        let thumb_nudge = px(-THUMB_W * pct);
        let state = self.state.clone();

        let fill = div()
            .absolute()
            .top_0()
            .left_0()
            .h_full()
            .w(relative(pct))
            .bg(theme::ACCENT);
        let thumb = SliderThumb::new(&state)
            .absolute()
            .top(px(-5.))
            .left(thumb_left)
            .ml(thumb_nudge)
            .w(px(THUMB_W))
            .h(px(14.))
            .bg(c::PAPER)
            .border_1()
            .border_color(c::SPACE)
            .shadow(if ring {
                vec![
                    gpui_kit::BoxShadow {
                        color: c::INK.into(),
                        offset: gpui_kit::point(px(0.), px(0.)),
                        blur_radius: px(0.),
                        spread_radius: px(2.),
                        inset: false,
                    },
                    gpui_kit::BoxShadow {
                        color: theme::ACCENT.into(),
                        offset: gpui_kit::point(px(0.), px(0.)),
                        blur_radius: px(0.),
                        spread_radius: px(4.),
                        inset: false,
                    },
                ]
            } else {
                theme::shadow_thumb()
            });

        div()
            .id(("range", self.state.entity_id()))
            .track_focus(&self.focus)
            .flex_1()
            .min_w_0()
            .max_w(px(self.max_w))
            .h(px(18.))
            // A click on the track focuses it (the thumb stops its own mouse-down).
            .capture_any_mouse_down({
                let focus = self.focus.clone();
                move |_, window, cx| window.focus(&focus, cx)
            })
            // The default margin of an `<input type=range>`.
            .m(px(2.))
            .cursor_pointer()
            .when_some(self.on_key, |d, f| {
                d.on_key_down(move |ev, window, cx| f(ev, window, cx))
            })
            .child(
                BaseSlider::new(&state)
                    .size_full()
                    .flex()
                    .items_center()
                    .child(
                        SliderTrack::new(&state)
                            .w_full()
                            .h(px(18.))
                            .flex()
                            .items_center()
                            .child(
                                SliderIndicator::new(&state)
                                    .relative()
                                    .w_full()
                                    .h(px(6.))
                                    .bg(c::SPACE)
                                    .border_1()
                                    .border_color(c::VIOLET)
                                    .child(fill)
                                    .child(thumb),
                            ),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_break_after_spaces_and_after_hyphens_inside_words() {
        assert_eq!(
            break_words("a half-finished path"),
            ["a ", "half-", "finished ", "path"]
        );
        assert_eq!(
            break_words("/tmp/claude-502/-Users-rick"),
            ["/tmp/claude-", "502/-Users-", "rick"]
        );
        assert_eq!(break_words("362fa7f5-4e4e"), ["362fa7f5-", "4e4e"]);
        assert_eq!(break_words(""), Vec::<&str>::new());
    }
}

#[cfg(test)]
mod range_tests {
    use super::should_push;

    #[test]
    fn the_engine_does_not_pull_a_slider_out_of_the_hand_or_repeat_itself() {
        assert!(should_push(false, None, 60.));
        assert!(
            !should_push(true, None, 60.),
            "mid-drag the engine value is ignored"
        );
        assert!(
            !should_push(false, Some(60.), 60.),
            "the echo of what is already shown"
        );
        assert!(
            should_push(false, Some(60.), 70.),
            "a real change from outside"
        );
    }
}
