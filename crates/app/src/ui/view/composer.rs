//! Slice C: the composer window and everything around it: the error banner and typing
//! line above it, the reply chip (`v.reply_to`), the multiline auto-growing input (Enter
//! sends, Shift+Enter breaks, the 4096 limit of the old `maxlength`), drafts per
//! conversation (`swap_draft`), @-autocomplete (`v.overlay`, `v.ac_*`), the attach button
//! (`commands::pick_and_offer_file`), the `+` menu (`commands::post_wordle_card`), pasted
//! images (`commands::offer_pasted_image`), `notify_typing` throttled with
//! `v.last_typing_sent`, the drop overlay over the main column, and the in-place edit box
//! the timeline draws for `v.editing` ([`render_edit_box`]).
//!
//! Drawn without `.cached()` because its height follows its content.

use super::super::commands;
use super::super::icons::Icon;
use super::super::state::{OpRef, PeerState, UiState};
use super::super::theme::{self, c};
use super::widgets::{self, avatar, face};
use super::{active_channel, AppHandle, AppSnapshot, Overlay, PaneInit, View, ViewState};
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::{Sizable, Size};
use gpui_kit::{
    canvas, div, prelude::FluentBuilder, px, relative, AnyElement, App, AppContext, Bounds,
    ClipboardEntry, ClipboardItem, Context, Entity, Focusable, ImageFormat, InteractiveElement,
    IntoElement, KeyBinding, ParentElement, Pixels, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Task, Window,
};
use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

gpui_kit::actions!(composer, [AcNext, AcPrev, AcAccept, SwallowTab]);

/// `maxlength="4096"` on the old textarea, which counted UTF-16 units. The engine's own
/// limit (`MAX_BERICHT_LEN`) is in bytes and stays the engine's to enforce.
const MAX_LEN_UTF16: usize = 4096;
/// Seven whole rows, then the input scrolls.
const MAX_ROWS: usize = 7;
/// One "typing" notification per window while typing continuously.
const TYPING_THROTTLE: Duration = Duration::from_millis(2500);
/// The typing dots step at this rate, below the 4 Hz meters, and only while drawn.
const BLINK_STEP_MS: u64 = 275;
/// `@keyframes typing-blink` lasts 1.1s; the three dots start 0, 180 and 360 ms apart.
const BLINK_PERIOD_MS: u64 = 1100;
const BLINK_DELAYS_MS: [u64; 3] = [0, 180, 360];
/// The reference shots froze the animation at 300 ms; step 0 shows that frame.
const BLINK_OFFSET_MS: u64 = 300;

/// Key bindings. Called from `view::init` after `gpui_kit::init`: a binding at the same
/// depth registered later wins, so these out-rank the input's own Tab, arrows and Enter.
pub fn init(cx: &mut App) {
    cx.bind_keys([
        // A browser textarea moves focus on Tab; indenting a chat line is never wanted.
        KeyBinding::new("tab", SwallowTab, Some("Composer > Input")),
        KeyBinding::new("down", AcNext, Some("ComposerAc > Input")),
        KeyBinding::new("up", AcPrev, Some("ComposerAc > Input")),
        KeyBinding::new("tab", AcAccept, Some("ComposerAc > Input")),
        KeyBinding::new("enter", AcAccept, Some("ComposerAc > Input")),
    ]);
}

// ------------------------------------------------------------------ pure rules

/// `updateAutocomplete`: an `@word` at the very end of the text opens the member list with
/// every known peer whose name starts with it. Returns whether anything drawn changed.
pub fn update_autocomplete(v: &mut ViewState, s: &UiState, text: &str) -> bool {
    let before = (
        v.overlay,
        v.ac_index,
        v.ac_matches.clone(),
        v.ac_query.clone(),
    );
    match mention_query(text) {
        None => {
            if v.overlay == Overlay::Autocomplete {
                v.overlay = Overlay::None;
            }
        }
        Some(q) => {
            let lower = q.to_lowercase();
            v.ac_query = q.to_string();
            v.ac_matches = known(s)
                .filter(|p| p.name.to_lowercase().starts_with(&lower))
                .map(|p| p.id.clone())
                .collect();
            v.ac_index = 0;
            v.overlay = Overlay::Autocomplete;
        }
    }
    before
        != (
            v.overlay,
            v.ac_index,
            v.ac_matches.clone(),
            v.ac_query.clone(),
        )
}

/// `/@(\w*)$/` with JavaScript's ASCII `\w`: the word after the last `@`, if the text ends
/// in one. An earlier `@` can never match when the last one does not.
fn mention_query(text: &str) -> Option<&str> {
    let at = text.rfind('@')?;
    let word = &text[at + 1..];
    word.bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        .then_some(word)
}

/// `acceptSuggestion`: everything from the last `@` becomes `@Name `.
fn accept_into(text: &str, name: &str) -> String {
    let head = text.rfind('@').map_or(text, |at| &text[..at]);
    format!("{head}@{name} ")
}

fn known(s: &UiState) -> impl Iterator<Item = &PeerState> {
    s.peers.iter().filter(|p| !p.id.is_empty())
}

/// `renderTyping`'s sentence, or `None` when nobody types.
fn typing_text(names: &[String]) -> Option<String> {
    let who = match names {
        [] => return None,
        [one] => one.clone(),
        [a, b] => format!("{a} and {b}"),
        [first, rest @ ..] => format!("{first} and {} others", rest.len()),
    };
    let verb = if names.len() == 1 { "is" } else { "are" };
    Some(format!("{who} {verb} typing\u{2026}"))
}

fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// The old `maxlength`: an edit that would take the text past the limit keeps only as much
/// of what it inserted as fits, the rest of the text untouched. Returns the corrected text
/// and the caret (a byte offset) right after what was kept, or `None` if `new` fits.
fn clamp_edit(old: &str, new: &str, max: usize) -> Option<(String, usize)> {
    if utf16_len(new) <= max {
        return None;
    }
    let prefix = old
        .char_indices()
        .zip(new.chars())
        .find(|((_, a), b)| a != b)
        .map_or(old.len().min(new.len()), |((i, _), _)| i);
    let room_for_suffix = old.len().min(new.len()) - prefix;
    let suffix = old
        .chars()
        .rev()
        .zip(new.chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a.len_utf8())
        .scan(0, |sum, n| {
            *sum += n;
            Some(*sum)
        })
        .take_while(|&sum| sum <= room_for_suffix)
        .last()
        .unwrap_or(0);
    let (head, tail) = (&new[..prefix], &new[new.len() - suffix..]);
    let mut room = max.saturating_sub(utf16_len(head) + utf16_len(tail));
    let kept: String = new[prefix..new.len() - suffix]
        .chars()
        .take_while(|ch| {
            let fits = ch.len_utf16() <= room;
            if fits {
                room -= ch.len_utf16();
            }
            fits
        })
        .collect();
    Some((format!("{head}{kept}{tail}"), prefix + kept.len()))
}

/// What the old `paste` handler named the file: the MIME subtype, `jpeg` as `jpg`. The
/// command keeps its own allowlist (B-53) on top of this.
fn paste_extension(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Png => "png",
        ImageFormat::Jpeg => "jpg",
        ImageFormat::Webp => "webp",
        ImageFormat::Gif => "gif",
        ImageFormat::Svg => "svg+xml",
        ImageFormat::Bmp => "bmp",
        ImageFormat::Tiff => "tiff",
        _ => "png",
    }
}

/// One frame of `@keyframes typing-blink` (0/60/100%: .25, 30%: 1; linear here, `ease` in
/// CSS) for a dot started `delay` ms late, `t` ms into the loop.
fn dot_opacity(t: u64, delay: u64) -> f32 {
    let x = ((t + BLINK_PERIOD_MS - delay % BLINK_PERIOD_MS) % BLINK_PERIOD_MS) as f32
        / BLINK_PERIOD_MS as f32;
    match x {
        x if x < 0.3 => 0.25 + 0.75 * x / 0.3,
        x if x < 0.6 => 1.0 - 0.75 * (x - 0.3) / 0.3,
        _ => 0.25,
    }
}

/// The conversation a placeholder names, as `renderHead` set it.
fn placeholder(snap: &AppSnapshot) -> String {
    if snap.v.view == View::Dms {
        if let Some(dm) = &snap.v.dm {
            return format!("Message {}", snap.peer(dm).map_or("", |p| p.name.as_str()));
        }
    }
    let name = snap
        .channel(&snap.v.channel)
        .or_else(|| snap.s.channels.first())
        .map_or("", |c| c.name.as_str());
    format!("Message #{name}")
}

/// A multi-line input where Enter is "submit" and Shift+Enter a line break, `rows` tall:
/// the composer grows from one row, the edit box keeps the two of a bare `<textarea>`.
fn new_input(
    window: &mut Window,
    cx: &mut App,
    value: &str,
    rows: (usize, usize),
) -> Entity<TextareaState> {
    cx.new(|cx| {
        TextareaState::new(window, cx)
            .auto_grow(rows.0, rows.1)
            .submit_on_enter(true)
            .default_value(value.to_string())
    })
}

/// The text field itself: no frame of its own (the parent draws it), 14px on 20, the
/// kit's smallest paddings so the stylesheet's 4px/2px can be added outside.
fn textarea(input: &Entity<TextareaState>) -> Textarea {
    Textarea::new(input)
        .appearance(false)
        .bordered(false)
        .with_size(Size::XSmall)
        .text_size(theme::BASE_TEXT)
        .line_height(px(20.))
}

/// Puts `text` in `input` without it counting as typing, the caret at the end like a
/// browser does after `value = ..`.
fn put_text(input: &Entity<TextareaState>, text: &str, window: &mut Window, cx: &mut App) {
    input.update(cx, |state, cx| {
        state.set_value(text.to_string(), window, cx);
        state.set_selected_range(text.len()..text.len(), cx);
    });
}

// ------------------------------------------------------------------ the pane

pub struct ComposerPane {
    app: AppHandle,
    input: Entity<TextareaState>,
    /// The text after the last change that was let through, for the length limit.
    accepted: String,
    placeholder: String,
    /// The reply the input was last focused for; a new one focuses it, as a reply click did.
    focused_for: Option<OpRef>,
    /// Where the `+` button was last painted, so a press on it does not count as the
    /// "outside" click that closes its own menu.
    plus_bounds: Rc<Cell<Bounds<Pixels>>>,
    _subscriptions: Vec<Subscription>,
}

impl ComposerPane {
    pub fn new(init: &PaneInit, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = new_input(window, cx, "", (1, MAX_ROWS));
        let subscription = cx.subscribe_in(
            &input,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => this.on_change(window, cx),
                InputEvent::PressEnter { shift: false, .. } => this.send(window, cx),
                _ => {}
            },
        );
        ComposerPane {
            app: init.app.clone(),
            input,
            accepted: String::new(),
            placeholder: String::new(),
            focused_for: None,
            plus_bounds: Rc::new(Cell::new(Bounds::default())),
            _subscriptions: vec![subscription],
        }
    }

    /// Contract with `AppView::after_conversation_change`: put `draft` in the input and
    /// return what was in it, which `AppView` keeps as the draft of the conversation being
    /// left. Called only when the open conversation really changes; touches only the input,
    /// because `AppView` is mid-update when it calls this.
    pub fn swap_draft(
        &mut self,
        draft: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> String {
        let left = self.input.read(cx).value().to_string();
        put_text(&self.input, draft, window, cx);
        self.accepted = draft.to_string();
        cx.notify();
        left
    }

    /// Debug fixture: the input holding `text` and focused, as if it had just been typed.
    /// Like `swap_draft` it leaves `AppView` alone; the caller runs [`update_autocomplete`].
    #[cfg(debug_assertions)]
    pub fn debug_type(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        put_text(&self.input, text, window, cx);
        self.accepted = text.to_string();
        let focus = self.input.focus_handle(cx);
        window.focus(&focus, cx);
        cx.notify();
    }

    fn focus_input(&self, window: &mut Window, cx: &mut App) {
        let focus = self.input.focus_handle(cx);
        window.focus(&focus, cx);
    }

    /// The `input` event: the length limit, the member list, and the typing notification.
    fn on_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut text = self.input.read(cx).value().to_string();
        if let Some((clamped, caret)) = clamp_edit(&self.accepted, &text, MAX_LEN_UTF16) {
            self.input.update(cx, |state, cx| {
                state.set_value(clamped.clone(), window, cx);
                state.set_selected_range(caret..caret, cx);
            });
            text = clamped;
        }
        self.accepted = text.clone();
        self.app.update(cx, |app, cx| {
            let s = app.s.clone();
            let changed = update_autocomplete(app.v_mut(), &s, &text);
            // One "typing" per window; the engine throttles too, this keeps the wire quiet.
            let due = app
                .v
                .last_typing_sent
                .is_none_or(|t| t.elapsed() > TYPING_THROTTLE);
            if !text.trim().is_empty() && due {
                app.v_mut().last_typing_sent = Some(Instant::now());
                commands::notify_typing(&app.ui, &active_channel(&app.v));
            }
            // Every pane redraws on a root notify; plain typing must not cost the timeline.
            if changed {
                cx.notify();
            }
        });
    }

    /// Enter without Shift (the member list, when open, takes Enter before this).
    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }
        let Some(snap) = self.app.snapshot(cx) else {
            return;
        };
        let channel = snap.active_channel();
        commands::send_message(
            &snap.ui,
            &channel,
            &text,
            snap.v.reply_to.as_ref().map(|r| &r.op),
        );
        put_text(&self.input, "", window, cx);
        self.accepted.clear();
        self.app.update(cx, |app, cx| {
            let v = app.v_mut();
            v.drafts.remove(&channel);
            v.last_typing_sent = None;
            if v.reply_to.take().is_some() {
                cx.notify();
            }
        });
    }

    /// Takes the suggestion `name`: the text from the last `@` becomes `@name `.
    fn accept(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let text = accept_into(&self.input.read(cx).value(), name);
        put_text(&self.input, &text, window, cx);
        self.accepted = text;
        self.app.update(cx, |app, cx| {
            app.v_mut().overlay = Overlay::None;
            cx.notify();
        });
        self.focus_input(window, cx);
    }

    fn ac_step(&mut self, down: bool, cx: &mut Context<Self>) {
        self.app.update(cx, |app, cx| {
            let v = app.v_mut();
            let n = v.ac_matches.len();
            if n > 0 {
                v.ac_index = (v.ac_index + if down { 1 } else { n - 1 }) % n;
                cx.notify();
            }
        });
    }

    fn ac_accept(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(snap) = self.app.snapshot(cx) else {
            return;
        };
        let name = snap
            .v
            .ac_matches
            .get(snap.v.ac_index)
            .and_then(|id| snap.peer(id))
            .map(|p| p.name.clone());
        if let Some(name) = name {
            self.accept(&name, window, cx);
        }
    }

    fn render_popover(&self, snap: &AppSnapshot, cx: &mut Context<Self>) -> Option<AnyElement> {
        match snap.v.overlay {
            Overlay::Plus => Some(self.render_plus_menu(snap, cx)),
            Overlay::Autocomplete if !snap.v.ac_matches.is_empty() => {
                Some(self.render_ac_list(snap, cx))
            }
            _ => None,
        }
    }

    fn render_ac_list(&self, snap: &AppSnapshot, cx: &mut Context<Self>) -> AnyElement {
        let rows = snap
            .v
            .ac_matches
            .iter()
            .filter_map(|id| snap.peer(id))
            .enumerate()
            .map(|(i, p)| {
                let selected = i == snap.v.ac_index;
                let name = p.name.clone();
                widgets::popover_item(("ac-item", i), selected)
                    .child(avatar(
                        &p.name,
                        p.avatar,
                        face(&snap.ui, p.picture.as_deref()),
                        20.,
                        None,
                        false,
                    ))
                    .child(p.name.clone())
                    .when(selected, |d| d.child(item_note("Tab")))
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.accept(&name, window, cx)),
                    )
            });
        popover(format!("Members matching @{}", snap.v.ac_query))
            .id("ac-list")
            .children(rows)
            .into_any_element()
    }

    fn render_plus_menu(&self, snap: &AppSnapshot, cx: &mut Context<Self>) -> AnyElement {
        let app = snap.app.clone();
        let plus = self.plus_bounds.clone();
        popover("Put in #general".to_string())
            .id("plus-menu")
            // A press anywhere else closes it; the `+` button toggles it itself.
            .on_mouse_down_out(move |event, _, cx| {
                if !plus.get().contains(&event.position) {
                    app.update(cx, |app, cx| {
                        if app.v.overlay == Overlay::Plus {
                            app.v_mut().overlay = Overlay::None;
                            cx.notify();
                        }
                    });
                }
            })
            .child(
                widgets::popover_item("plus-wordle", false)
                    .child(Icon::Tiles.size(16.))
                    .child("Today's Wordle")
                    .child(item_note("everyone sees it"))
                    // No board opens: this puts the card in the chat for everyone.
                    .on_click(cx.listener(|this, _, _, cx| {
                        let Some(snap) = this.app.snapshot(cx) else {
                            return;
                        };
                        this.app.update(cx, |app, cx| {
                            app.v_mut().overlay = Overlay::None;
                            cx.notify();
                        });
                        commands::post_wordle_card(&snap.ui);
                    })),
            )
            .into_any_element()
    }

    fn render_frame(
        &self,
        snap: &AppSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let focused = self.input.focus_handle(cx).is_focused(window);
        let ac_open = snap.v.overlay == Overlay::Autocomplete && !snap.v.ac_matches.is_empty();
        let plus_bounds = self.plus_bounds.clone();
        let app = snap.app.clone();
        div()
            .id("composer")
            // `ComposerAc` only while the list shows, so Enter and the arrows reach the input otherwise.
            .key_context(if ac_open {
                "Composer ComposerAc"
            } else {
                "Composer"
            })
            .on_action(cx.listener(|this, _: &AcNext, _, cx| this.ac_step(true, cx)))
            .on_action(cx.listener(|this, _: &AcPrev, _, cx| this.ac_step(false, cx)))
            .on_action(cx.listener(|this, _: &AcAccept, window, cx| this.ac_accept(window, cx)))
            .on_action(|_: &SwallowTab, _, _| {})
            .flex()
            .flex_row()
            .items_end()
            .gap(px(4.))
            .pt(px(6.))
            .pr(px(6.))
            .pb(px(6.))
            .pl(px(8.))
            .bg(c::VOID)
            .rounded(theme::RADIUS_FIELD)
            .border_1()
            .border_color(if focused {
                theme::ACCENT
            } else {
                theme::BORDER
            })
            .child(
                composer_btn("composer-attach", Icon::Clip, "Share a file").on_click(
                    move |_, _, cx| {
                        if let Some(snap) = app.snapshot(cx) {
                            commands::pick_and_offer_file(&snap.ui, &snap.active_channel());
                        }
                    },
                ),
            )
            .child(
                composer_btn("composer-plus", Icon::Plus, "Put something in the chat")
                    .relative()
                    .child(
                        canvas(move |bounds, _, _| plus_bounds.set(bounds), |_, _, _, _| {})
                            .absolute()
                            .size_full(),
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.app.update(cx, |app, cx| {
                            let v = app.v_mut();
                            v.overlay = if v.overlay == Overlay::Plus {
                                Overlay::None
                            } else {
                                Overlay::Plus
                            };
                            cx.notify();
                        });
                    })),
            )
            .child(
                // The stylesheet's textarea padding is 4px 4px 4px 2px; the kit adds 4px sideways.
                div().flex_1().min_w_0().py(px(4.)).ml(px(-2.)).child(
                    textarea(&self.input).on_paste({
                        let app = self.app.clone();
                        move |item, _, cx| paste_image(&app, item, cx)
                    }),
                ),
            )
    }
}

/// An image on the clipboard is offered as a file; anything else pastes as text.
fn paste_image(app: &AppHandle, item: &ClipboardItem, cx: &mut App) -> bool {
    let Some(image) = item.entries().iter().find_map(|e| match e {
        ClipboardEntry::Image(image) => Some(image),
        _ => None,
    }) else {
        return false;
    };
    if let Some(snap) = app.snapshot(cx) {
        commands::offer_pasted_image(
            &snap.ui,
            &image.bytes,
            paste_extension(image.format),
            &snap.active_channel(),
        );
    }
    true
}

impl Render for ComposerPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(snap) = self.app.snapshot(cx) else {
            return div().into_any_element();
        };

        let wanted = placeholder(&snap);
        if wanted != self.placeholder {
            self.input.update(cx, |state, cx| {
                state.set_placeholder(wanted.clone(), window, cx)
            });
            self.placeholder = wanted;
        }

        // Starting a reply puts the caret in the composer, as the reply click did.
        let reply_op = snap.v.reply_to.as_ref().map(|r| r.op.clone());
        if reply_op != self.focused_for {
            if reply_op.is_some() {
                let focus = self.input.focus_handle(cx);
                window.defer(cx, move |window, cx| window.focus(&focus, cx));
            }
            self.focused_for = reply_op;
        }

        let names = snap.typing_names();
        let popover = self.render_popover(&snap, cx);
        div()
            .px(px(28.))
            .pb(px(18.))
            .when_some(snap.s.error.clone(), |d, error| {
                d.child(render_error(&snap, error))
            })
            .when_some(typing_text(&names), |d, text| {
                d.child(render_typing(text, &self.app, window, cx))
            })
            .child(
                div()
                    .relative()
                    .when_some(snap.v.reply_to.as_ref(), |d, reply| {
                        d.child(self.render_reply_chip(&reply.name, &reply.body, cx))
                    })
                    .child(self.render_frame(&snap, window, cx))
                    .children(popover),
            )
            .into_any_element()
    }
}

impl ComposerPane {
    /// `renderReplyChip`: "Replying to <name> <body>" with a cancel cross.
    fn render_reply_chip(
        &self,
        name: &str,
        body: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div().pb(px(4.)).child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(10.))
                .py(px(5.))
                .bg(theme::BG_RAISED)
                .rounded_t(theme::RADIUS_FIELD)
                .border_1()
                .border_b_0()
                .border_color(theme::BORDER)
                .text_size(px(12.))
                .text_color(theme::TEXT_MUTED)
                .child(
                    div()
                        .flex_none()
                        .text_size(px(10.5))
                        .text_color(theme::TEXT_DIM)
                        .child(widgets::tracked("REPLYING TO", 10.5, 0.04)),
                )
                .child(
                    div()
                        .flex_none()
                        .font_weight(gpui_kit::FontWeight::MEDIUM)
                        .text_color(theme::TEXT_STRONG)
                        .child(name.to_string()),
                )
                // `white-space: nowrap` folded line breaks into spaces; GPUI would break on them.
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(body.replace('\n', " ")),
                )
                .child(
                    div()
                        .id("reply-cancel")
                        .flex_none()
                        .p(px(2.))
                        .cursor_pointer()
                        .text_color(theme::TEXT_MUTED)
                        .hover(|s| s.text_color(theme::TEXT_STRONG))
                        .tooltip(widgets::tooltip("Cancel reply"))
                        .child(Icon::X.size(14.))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.app.update(cx, |app, cx| {
                                app.v_mut().reply_to = None;
                                cx.notify();
                            });
                            this.focus_input(window, cx);
                        })),
                ),
        )
    }
}

/// `renderError`: the engine's error line in red, with its dismiss cross.
fn render_error(snap: &AppSnapshot, error: String) -> impl IntoElement {
    let ui = snap.ui.clone();
    div()
        .flex()
        .items_center()
        .gap(px(9.))
        .mx(px(16.))
        .mb(px(8.))
        .px(px(10.))
        .py(px(8.))
        .rounded(theme::RADIUS_FIELD)
        .border_1()
        .border_color(theme::DANGER_LINE)
        .bg(theme::DANGER_BG)
        .text_color(theme::DANGER_TEXT)
        .text_size(px(13.))
        .child(div().flex_none().child(Icon::Alert.size(16.)))
        .child(div().flex_1().min_w_0().child(error))
        .child(
            div()
                .id("error-dismiss")
                .flex_none()
                .size(px(24.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(theme::RADIUS_CONTROL)
                .cursor_pointer()
                .text_color(theme::DANGER_TEXT)
                .hover(|s| s.bg(c::VOID))
                .tooltip(widgets::tooltip("Dismiss"))
                .child(Icon::X.size(13.))
                .on_click(move |_, _, _| commands::dismiss_error(&ui)),
        )
}

/// `renderTyping`: three stepping dots and the sentence. The dots are their own view with
/// their own timer, which runs only while somebody types: it stops by itself when the line
/// empties, even in a hidden window where no frame drops the view.
fn render_typing(
    text: String,
    app: &AppHandle,
    window: &mut Window,
    cx: &mut App,
) -> impl IntoElement {
    let dots = window.use_keyed_state("typing-dots", cx, |_, _| TypingDots {
        app: app.clone(),
        step: 0,
        ticker: None,
    });
    dots.update(cx, |dots, cx| dots.keep_ticking(cx));
    div()
        .flex()
        .items_center()
        .gap(px(7.))
        .px(px(14.))
        .pb(px(4.))
        .text_size(px(11.5))
        .text_color(theme::TEXT_MUTED)
        .child(dots)
        .child(text)
}

struct TypingDots {
    app: AppHandle,
    step: u64,
    /// `None` once the loop below has ended (nobody typing); the next render restarts it.
    ticker: Option<Task<()>>,
}

impl TypingDots {
    fn keep_ticking(&mut self, cx: &mut Context<Self>) {
        if self.ticker.is_some() {
            return;
        }
        self.ticker = Some(cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(BLINK_STEP_MS))
                .await;
            let typing = this.update(cx, |dots, cx| {
                let typing = dots
                    .app
                    .snapshot(cx)
                    .is_some_and(|s| !s.typing_names().is_empty());
                if typing {
                    dots.step = dots.step.wrapping_add(1);
                    cx.notify();
                } else {
                    dots.ticker = None;
                }
                typing
            });
            if !matches!(typing, Ok(true)) {
                return;
            }
        }));
    }
}

impl Render for TypingDots {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let t = (BLINK_OFFSET_MS + self.step * BLINK_STEP_MS) % BLINK_PERIOD_MS;
        div()
            .flex()
            .items_center()
            .gap(px(7.))
            .children(BLINK_DELAYS_MS.map(|delay| {
                div()
                    .size(px(5.))
                    .rounded_full()
                    .bg(theme::TEXT_MUTED)
                    .opacity(dot_opacity(t, delay))
            }))
    }
}

/// A 30×30 glyph button, muted; hover lifts it.
fn composer_btn(
    id: &'static str,
    icon: Icon,
    title: &'static str,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    div()
        .id(id)
        .size(px(30.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(theme::RADIUS_CONTROL)
        .cursor_pointer()
        .text_color(theme::TEXT_MUTED)
        .hover(|s| s.bg(theme::BG_HOVER).text_color(theme::TEXT_STRONG))
        .tooltip(widgets::tooltip(title))
        .child(icon.size(18.))
}

/// The popover over the composer that the member list and the `+` menu share: a card with
/// the soft shadow and a mono label.
fn popover(head: String) -> gpui_kit::Div {
    widgets::popover_surface()
        .absolute()
        .left(px(16.))
        .right(px(16.))
        .bottom(relative(1.))
        .mb(px(8.))
        .max_w(px(320.))
        .flex()
        .flex_col()
        .overflow_hidden()
        .p(px(6.))
        .occlude()
        .child(
            widgets::section_label(&head)
                .px(px(8.))
                .pt(px(4.))
                .pb(px(6.)),
        )
}

fn item_note(text: &'static str) -> impl IntoElement {
    div().ml_auto().text_size(px(11.)).child(text)
}

// ------------------------------------------------------------------ drop overlay

/// The "Drop to share" overlay, drawn by `AppView` over the main column while
/// `snap.drop_active` holds. No handlers and no occlusion: the drop itself lands on the
/// root's `on_drop`, which offers the files by index (B-52).
pub fn render_drop_overlay(snap: &AppSnapshot, _window: &mut Window, _cx: &mut App) -> AnyElement {
    let target = if snap.v.view == View::Dms {
        snap.active_name()
    } else {
        format!("#{}", snap.active_name())
    };
    div()
        .absolute()
        .top(px(8.))
        .left(px(8.))
        .right(px(8.))
        .bottom(px(8.))
        .bg(theme::BACKDROP)
        .rounded(theme::RADIUS_CARD)
        .border_1()
        .border_dashed()
        .border_color(theme::ACCENT)
        .child(
            div()
                .relative()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(8.))
                .child(div().text_color(theme::ACCENT).child(Icon::Share.size(30.)))
                .child(
                    div()
                        .font_family(theme::DISPLAY_FONT)
                        .text_size(theme::DISPLAY_TEXT)
                        .text_color(theme::TEXT_STRONG)
                        .child("Drop to share"),
                )
                .child(
                    div()
                        .text_size(px(12.5))
                        .text_color(theme::TEXT_MUTED)
                        .child(format!("The file is offered to everyone in {target}")),
                ),
        )
        .into_any_element()
}

// ------------------------------------------------------------------ edit in place

/// The edit box the timeline draws in place of a message body while `v.editing` points at
/// it: the body in a textarea, focused with the caret at the end. Enter saves through
/// `edit_message` (empty deletes), Shift+Enter breaks the line, Escape cancels (the root's
/// Escape order). Its state lives as long as the timeline keeps drawing it.
pub fn render_edit_box(
    snap: &AppSnapshot,
    op: &OpRef,
    body: &str,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let key = SharedString::from(format!("edit-{}-{}-{}", op.channel, op.author, op.seq));
    let (app, op, body) = (snap.app.clone(), op.clone(), body.to_string());
    window
        .use_keyed_state(key, cx, move |window, cx| {
            EditBox::new(app, op, &body, window, cx)
        })
        .into_any_element()
}

pub struct EditBox {
    app: AppHandle,
    op: OpRef,
    input: Entity<TextareaState>,
    focused_once: bool,
    _subscription: Subscription,
}

impl EditBox {
    fn new(
        app: AppHandle,
        op: OpRef,
        body: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = new_input(window, cx, body, (2, 2));
        let subscription = cx.subscribe_in(&input, window, |this, _, event: &InputEvent, _, cx| {
            if let InputEvent::PressEnter { shift: false, .. } = event {
                this.save(cx);
            }
        });
        EditBox {
            app,
            op,
            input,
            focused_once: false,
            _subscription: subscription,
        }
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value().to_string();
        let op = self.op.clone();
        self.app.update(cx, |app, cx| {
            if app.v.editing.as_ref() == Some(&op) {
                app.v_mut().editing = None;
                cx.notify();
            }
            commands::edit_message(&app.ui, &op, &text);
        });
    }
}

impl Render for EditBox {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.focused_once {
            self.focused_once = true;
            let len = self.input.read(cx).value().len();
            self.input
                .update(cx, |state, cx| state.set_selected_range(len..len, cx));
            let focus = self.input.focus_handle(cx);
            window.defer(cx, move |window, cx| window.focus(&focus, cx));
        }
        let focused = self.input.focus_handle(cx).is_focused(window);
        div()
            .w_full()
            .child(
                div()
                    .w_full()
                    .px(px(9.))
                    .py(px(7.))
                    .rounded(theme::RADIUS_FIELD)
                    .border_1()
                    .border_color(if focused {
                        theme::ACCENT
                    } else {
                        theme::BORDER
                    })
                    .bg(c::VOID)
                    .text_color(theme::TEXT)
                    .child(div().mx(px(-4.)).child(textarea(&self.input))),
            )
            .child(
                div()
                    .mt(px(5.))
                    .flex()
                    .text_size(px(11.))
                    .text_color(theme::TEXT_MUTED)
                    .child(div().text_color(theme::TEXT).child("Enter"))
                    .child("\u{00a0}saves \u{00b7} ")
                    .child(div().text_color(theme::TEXT).child("Esc"))
                    .child("\u{00a0}cancels \u{00b7} empty deletes"),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mention_query_is_the_word_after_the_last_at() {
        assert_eq!(mention_query("hoi @"), Some(""));
        assert_eq!(mention_query("hoi @Sa"), Some("Sa"));
        assert_eq!(mention_query("a@b @c_9"), Some("c_9"));
        assert_eq!(mention_query("@sander hoi"), None, "a space ends the word");
        assert_eq!(mention_query("@é"), None, "\\w is ASCII in JavaScript");
        assert_eq!(mention_query("geen at"), None);
    }

    #[test]
    fn accepting_replaces_from_the_last_at() {
        assert_eq!(accept_into("hoi @sa", "Sander"), "hoi @Sander ");
        assert_eq!(accept_into("@", "Joost"), "@Joost ");
        assert_eq!(accept_into("geen", "Bram"), "geen@Bram ");
    }

    #[test]
    fn the_typing_line_reads_like_before() {
        let n = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(typing_text(&n(&[])), None);
        assert_eq!(
            typing_text(&n(&["Joost"])).as_deref(),
            Some("Joost is typing\u{2026}")
        );
        assert_eq!(
            typing_text(&n(&["Sander", "Joost"])).as_deref(),
            Some("Sander and Joost are typing\u{2026}")
        );
        assert_eq!(
            typing_text(&n(&["Sander", "Joost", "Bram"])).as_deref(),
            Some("Sander and 2 others are typing\u{2026}")
        );
    }

    #[test]
    fn an_edit_past_the_limit_keeps_what_fits_of_the_insertion() {
        assert_eq!(clamp_edit("ab", "abc", 3), None);
        // Typing at the limit inserts nothing, wherever the caret is.
        assert_eq!(clamp_edit("abc", "abXc", 3), Some(("abc".into(), 2)));
        // A paste is cut to the room left, the text after the caret kept.
        assert_eq!(clamp_edit("ac", "a1234c", 4), Some(("a12c".into(), 3)));
        // UTF-16 units, as `maxlength` counted them: an emoji is two.
        assert_eq!(
            clamp_edit("abc", "abc\u{1F600}", 4),
            Some(("abc".into(), 3))
        );
        assert_eq!(
            clamp_edit("", "\u{e9}\u{e9}\u{e9}", 2),
            Some(("\u{e9}\u{e9}".into(), 4))
        );
        // Repeated characters: the suffix never overlaps the prefix.
        assert_eq!(clamp_edit("aa", "aaaa", 3), Some(("aaa".into(), 3)));
    }

    #[test]
    fn the_first_blink_step_is_the_reference_frame() {
        let t = BLINK_OFFSET_MS;
        let [a, b, c] = BLINK_DELAYS_MS.map(|d| dot_opacity(t, d));
        assert!(
            a > 0.9 && (0.45..0.6).contains(&b) && c == 0.25,
            "{a} {b} {c}"
        );
        // Every dot peaks somewhere in the loop of steps.
        let steps = BLINK_PERIOD_MS.div_ceil(BLINK_STEP_MS);
        for d in BLINK_DELAYS_MS {
            let peak = (0..steps)
                .map(|k| dot_opacity((t + k * BLINK_STEP_MS) % BLINK_PERIOD_MS, d))
                .fold(0., f32::max);
            assert!(peak > 0.6, "dot {d} never lights: {peak}");
        }
    }
}

/// The composer inside a real `AppView`, in a headless window, with the engine's command
/// channel kept so each test can assert what reached it.
#[cfg(test)]
pub(super) mod window_tests {
    use super::super::super::bridge::{Feeds, MeterReadings, ThumbMap};
    use super::super::super::state::{Constants, Presence};
    use super::super::super::{theme, Ui};
    use super::super::{AppView, ReplyTo};
    use super::*;
    use crate::engine::{EngineHandle, Snapshot, UiCommand};
    use fitcom_proto::{Channel, PeerId};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{
        AnyWindowHandle, ExternalPaths, FileDropEvent, TestAppContext, VisualTestContext,
    };
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicU64};
    use std::sync::{Arc, Mutex};
    use tokio::sync::mpsc;

    pub(in crate::ui::view) const SANDER: &str = "b2d4f6a8-1c3e-4a5b-8d7f-9e0a1b2c3d4e";

    pub(in crate::ui::view) struct Harness {
        pub(in crate::ui::view) app: Entity<AppView>,
        ui: Arc<Ui>,
        pub(in crate::ui::view) window: AnyWindowHandle,
        commands: mpsc::Receiver<UiCommand>,
        _runtime: tokio::runtime::Runtime,
    }

    impl Harness {
        pub(in crate::ui::view) fn new(cx: &mut TestAppContext) -> Self {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap();
            let (_snapshot_tx, snapshot) =
                tokio::sync::watch::channel(Arc::new(Snapshot::default()));
            let (commands_tx, commands) = mpsc::channel(64);
            let me = PeerId(uuid::Uuid::from_u128(1));
            let constants = || Constants {
                me,
                fallback_name: "Rick".into(),
                sound_sets: Vec::new(),
                sound_events: Vec::new(),
                control_port: 0,
                media_port: 0,
                autostart: false,
                minimize_to_tray: false,
            };
            let mut state = UiState::build(&Snapshot::default(), &constants(), 0, &HashMap::new());
            state.peers = vec![
                peer(SANDER, "Sander"),
                peer("c3e5a7b9-2d4f-4b6c-9e8a-1f2b3c4d5e6f", "Joost"),
            ];
            let ui = Arc::new(Ui {
                engine: EngineHandle {
                    snapshot,
                    commands: commands_tx,
                    voorgrond: Arc::new(AtomicBool::new(true)),
                    afsluiten_voor_update: Arc::new(AtomicBool::new(false)),
                },
                me,
                fallback_name: "Rick".into(),
                youtube_dir: PathBuf::from("/nonexistent"),
                minimize_to_tray: false,
                constants: constants(),
                sources: Mutex::new(Vec::new()),
                dropped: Mutex::new(Vec::new()),
                timeline_revision: AtomicU64::new(0),
                last_fingerprint: Mutex::new(0),
                last_seen: Mutex::new(HashMap::new()),
                #[cfg(debug_assertions)]
                fixture: None,
                runtime: runtime.handle().clone(),
            });
            cx.update(|cx| {
                gpui_kit::init(cx);
                theme::install(cx);
                super::super::init(cx);
            });
            let feeds = Feeds::fixed(state, MeterReadings::default(), ThumbMap::new());
            let built = Rc::new(std::cell::RefCell::new(None));
            let slot = built.clone();
            let ui2 = ui.clone();
            let window = cx.add_window(move |window, cx| {
                let app = cx.new(|cx| AppView::new(ui2, feeds, window, cx));
                *slot.borrow_mut() = Some(app.clone());
                gpui_kit::base::Root::new(app, window, cx)
            });
            let app = built.borrow_mut().take().unwrap();
            let mut harness = Harness {
                app,
                ui,
                window: window.into(),
                commands,
                _runtime: runtime,
            };
            harness.drain();
            harness
        }

        pub(in crate::ui::view) fn with_window<R>(
            &self,
            cx: &mut TestAppContext,
            f: impl FnOnce(&mut Window, &mut App) -> R,
        ) -> R {
            cx.update_window(self.window, |_, window, cx| f(window, cx))
                .unwrap()
        }

        fn focus_composer(&self, cx: &mut TestAppContext) {
            let pane = self.app.read_with(cx, |app, _| app.panes.composer.clone());
            self.with_window(cx, |window, cx| {
                window.render_frame(cx);
                pane.update(cx, |p, cx| p.focus_input(window, cx));
                window.render_frame(cx);
            });
        }

        fn text(&self, cx: &mut TestAppContext) -> String {
            self.app.read_with(cx, |app, cx| {
                app.panes
                    .composer
                    .read(cx)
                    .input
                    .read(cx)
                    .value()
                    .to_string()
            })
        }

        pub(in crate::ui::view) fn drain(&mut self) -> Vec<UiCommand> {
            std::iter::from_fn(|| self.commands.try_recv().ok()).collect()
        }
    }

    fn peer(id: &str, name: &str) -> PeerState {
        PeerState {
            id: id.into(),
            name: name.into(),
            address: String::new(),
            avatar: 1,
            picture: None,
            presence: Presence::Online,
            user_status: None,
            activity: None,
            problem: None,
            path: None,
            app_version: None,
            in_call: false,
            muted: false,
            deafened: false,
            volume: 1.0,
            unread: 0,
            last_seen: None,
            desktop_volume: None,
            desktop_stream: None,
            music: None,
            slot: 0,
        }
    }

    /// Revert to see it fail: drop the `PressEnter` arm in `ComposerPane::new`.
    /// A test window opens inactive, like a window started by autostart behind others. It must
    /// not count as looked at: the engine sends desktop notifications only for a window that
    /// is not in front. Dropping the seeding in `AppView::new` makes this fail.
    #[gpui_kit::test]
    fn a_window_that_opens_inactive_is_not_in_front(cx: &mut TestAppContext) {
        let h = Harness::new(cx);
        let focused = h.app.read_with(cx, |app, _| app.v.focused);
        let in_front =
            h.ui.engine
                .voorgrond
                .load(std::sync::atomic::Ordering::Relaxed);
        assert_eq!((focused, in_front), (false, false));
    }

    #[gpui_kit::test]
    fn enter_sends_the_trimmed_text_with_the_reply_and_clears(cx: &mut TestAppContext) {
        let mut h = Harness::new(cx);
        let reply = OpRef {
            author: SANDER.into(),
            channel: "general".into(),
            seq: 4,
        };
        h.app.update(cx, |app, cx| {
            app.v_mut().reply_to = Some(ReplyTo {
                op: reply.clone(),
                name: "Sander".into(),
                body: "ik start de server".into(),
            });
            cx.notify();
        });
        h.focus_composer(cx);
        h.with_window(cx, |window, cx| {
            window.input("  kom eraan ", cx);
            window.press("enter", cx);
        });

        let sent: Vec<_> = h
            .drain()
            .into_iter()
            .filter_map(|c| match c {
                UiCommand::Plaats(text, channel, reply) => Some((text, channel, reply)),
                _ => None,
            })
            .collect();
        assert_eq!(sent.len(), 1, "{sent:?}");
        let (text, channel, reply_to) = &sent[0];
        assert_eq!(text, "kom eraan");
        assert_eq!(*channel, Channel::GENERAL);
        assert_eq!(*reply_to, reply.to_op_id());
        assert_eq!(h.text(cx), "");
        assert!(
            h.app.read_with(cx, |app, _| app.v.reply_to.is_none()),
            "the chip goes away"
        );
    }

    /// Enter never becomes a line break, even with nothing to send (the old
    /// `preventDefault`). Revert to see it fail: drop `.submit_on_enter(true)` in `new_input`.
    #[gpui_kit::test]
    fn enter_on_an_empty_composer_leaves_it_empty(cx: &mut TestAppContext) {
        let mut h = Harness::new(cx);
        h.focus_composer(cx);
        h.with_window(cx, |window, cx| window.press("enter", cx));
        assert_eq!(h.text(cx), "");
        assert!(!h.drain().iter().any(|c| matches!(c, UiCommand::Plaats(..))));
    }

    /// Revert to see it fail: make `send` ignore `shift` (match `PressEnter { .. }`).
    #[gpui_kit::test]
    fn shift_enter_breaks_the_line_and_sends_nothing(cx: &mut TestAppContext) {
        let mut h = Harness::new(cx);
        h.focus_composer(cx);
        h.with_window(cx, |window, cx| {
            window.input("eerste", cx);
            window.press("shift-enter", cx);
            window.input("tweede", cx);
        });
        assert_eq!(h.text(cx), "eerste\ntweede");
        assert!(!h.drain().iter().any(|c| matches!(c, UiCommand::Plaats(..))));
    }

    /// Revert to see it fail: drop the `ComposerAc` bindings in `init`, or the
    /// `key_context` switch in `render_frame`.
    #[gpui_kit::test]
    fn the_member_list_takes_arrows_and_tab(cx: &mut TestAppContext) {
        let mut h = Harness::new(cx);
        h.focus_composer(cx);
        h.with_window(cx, |window, cx| window.input("hoi @", cx));
        let (overlay, matches) = h
            .app
            .read_with(cx, |app, _| (app.v.overlay, app.v.ac_matches.len()));
        assert_eq!((overlay, matches), (Overlay::Autocomplete, 2));

        h.with_window(cx, |window, cx| {
            window.press("down", cx);
            window.press("tab", cx);
        });
        assert_eq!(h.text(cx), "hoi @Joost ");
        assert_eq!(h.app.read_with(cx, |app, _| app.v.overlay), Overlay::None);

        // Enter takes a suggestion instead of sending. Typed in its own update: the input's
        // `Change` reaches the composer only when the effects of an update are flushed.
        h.with_window(cx, |window, cx| window.input("@s", cx));
        h.with_window(cx, |window, cx| window.press("enter", cx));
        assert_eq!(h.text(cx), "hoi @Joost @Sander ");
        assert!(!h.drain().iter().any(|c| matches!(c, UiCommand::Plaats(..))));
    }

    /// Escape typed in the composer still reaches the root's Escape order.
    /// Revert to see it fail: `.clean_on_escape()` on the composer's input state.
    #[gpui_kit::test]
    fn escape_in_the_composer_closes_the_member_list(cx: &mut TestAppContext) {
        let h = Harness::new(cx);
        h.focus_composer(cx);
        h.with_window(cx, |window, cx| window.input("@", cx));
        assert_eq!(
            h.app.read_with(cx, |app, _| app.v.overlay),
            Overlay::Autocomplete
        );
        h.with_window(cx, |window, cx| window.press("escape", cx));
        assert_eq!(h.app.read_with(cx, |app, _| app.v.overlay), Overlay::None);
        assert_eq!(h.text(cx), "@", "the text stays");
    }

    /// Starting a reply (the timeline sets `v.reply_to`) puts the caret in the composer.
    /// Revert to see it fail: drop the `window.defer(.. focus ..)` in `render`.
    #[gpui_kit::test]
    fn a_new_reply_focuses_the_composer(cx: &mut TestAppContext) {
        let h = Harness::new(cx);
        h.app.update(cx, |app, cx| {
            app.v_mut().reply_to = Some(ReplyTo {
                op: OpRef {
                    author: SANDER.into(),
                    channel: "general".into(),
                    seq: 1,
                },
                name: "Sander".into(),
                body: "hoi".into(),
            });
            cx.notify();
        });
        h.with_window(cx, |window, cx| window.render_frame(cx));
        cx.run_until_parked();
        let focus = h.app.read_with(cx, |app, cx| {
            app.panes.composer.read(cx).input.focus_handle(cx)
        });
        assert!(h.with_window(cx, |window, _| focus.is_focused(window)));
    }

    /// Revert to see it fail: make `swap_draft` return an empty string, or drop its
    /// `put_text`.
    #[gpui_kit::test]
    fn a_draft_waits_in_its_own_conversation(cx: &mut TestAppContext) {
        let h = Harness::new(cx);
        h.focus_composer(cx);
        h.with_window(cx, |window, cx| window.input("half af", cx));
        let app = h.app.clone();
        h.with_window(cx, |window, cx| {
            app.update(cx, |app, cx| app.open_dm(SANDER, window, cx))
        });
        assert_eq!(h.text(cx), "");
        h.with_window(cx, |window, cx| {
            app.update(cx, |app, cx| app.open_channel("general", window, cx))
        });
        assert_eq!(h.text(cx), "half af");
    }

    /// Typing notifies once per 2.5 s window, not per keystroke.
    /// Revert to see it fail: drop the `due` check in `on_change`.
    #[gpui_kit::test]
    fn typing_is_announced_once_per_window(cx: &mut TestAppContext) {
        let mut h = Harness::new(cx);
        h.focus_composer(cx);
        h.with_window(cx, |window, cx| window.input("abc", cx));
        let typing = h
            .drain()
            .iter()
            .filter(|c| matches!(c, UiCommand::Typing(_)))
            .count();
        assert_eq!(typing, 1);
    }

    /// The whole OS drop chain: the overlay shows during the drag, the drop keeps the
    /// paths in `Ui::dropped` and offers them by index to the open conversation.
    /// Revert to see it fail: drop the `on_drop` listener on the root.
    #[gpui_kit::test]
    fn a_dropped_file_is_offered_to_the_open_conversation(cx: &mut TestAppContext) {
        let mut h = Harness::new(cx);
        let path = PathBuf::from("/tmp/fitcom-test-drop.txt");
        let at = gpui_kit::point(px(600.), px(300.));
        let mut vcx = VisualTestContext::from_window(h.window, cx);
        vcx.simulate_event(FileDropEvent::Entered {
            position: at,
            paths: ExternalPaths(vec![path.clone()].into()),
        });
        vcx.simulate_event(FileDropEvent::Pending { position: at });
        assert!(
            h.app.read_with(cx, |app, _| app.snapshot().drop_active),
            "overlay while dragging"
        );
        let mut vcx = VisualTestContext::from_window(h.window, cx);
        vcx.simulate_event(FileDropEvent::Submit { position: at });

        let offered: Vec<_> = h
            .drain()
            .into_iter()
            .filter_map(|c| match c {
                UiCommand::BiedBestandAan(p, channel) => Some((p, channel)),
                _ => None,
            })
            .collect();
        assert_eq!(offered, vec![(path.clone(), Channel::GENERAL)]);
        assert_eq!(*h.ui.dropped.lock().unwrap(), vec![path]);
        assert!(
            !h.app.read_with(cx, |app, _| app.snapshot().drop_active),
            "overlay gone after the drop"
        );
    }

    /// Revert to see it fail: drop the `PressEnter` subscription in `EditBox::new`.
    #[gpui_kit::test]
    fn enter_in_the_edit_box_saves_and_closes_it(cx: &mut TestAppContext) {
        let mut h = Harness::new(cx);
        let op = OpRef {
            author: uuid::Uuid::from_u128(1).to_string(),
            channel: "general".into(),
            seq: 9,
        };
        h.app
            .update(cx, |app, _| app.v_mut().editing = Some(op.clone()));
        let handle = h.app.read_with(cx, |app, _| app.handle.clone());
        let (edit_op, root_op) = (op.clone(), op.clone());
        let edit_window: AnyWindowHandle = cx
            .add_window(move |window, cx| {
                let edit = cx.new(|cx| EditBox::new(handle, edit_op, "oud", window, cx));
                gpui_kit::base::Root::new(edit, window, cx)
            })
            .into();
        cx.update_window(edit_window, |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
            window.input(" nieuw", cx);
        })
        .unwrap();
        cx.update_window(edit_window, |_, window, cx| window.press("enter", cx))
            .unwrap();

        let edits: Vec<_> = h
            .drain()
            .into_iter()
            .filter_map(|c| match c {
                UiCommand::Bewerk(id, text) => Some((id, text)),
                _ => None,
            })
            .collect();
        assert_eq!(
            edits,
            vec![(root_op.to_op_id().unwrap(), "oud nieuw".to_string())]
        );
        assert!(h.app.read_with(cx, |app, _| app.v.editing.is_none()));
    }

    /// The blink costs nothing once nobody types, even if no frame is drawn to drop it.
    /// Revert to see it fail: make the ticker loop ignore `typing`.
    #[gpui_kit::test]
    fn the_typing_dots_stop_when_nobody_types(cx: &mut TestAppContext) {
        let h = Harness::new(cx);
        let set_typing = |who: Vec<String>, cx: &mut TestAppContext| {
            h.app.update(cx, |app, _| {
                let mut s = (*app.s).clone();
                s.typing.insert("general".into(), who);
                app.s = Arc::new(s);
            })
        };
        set_typing(vec![SANDER.into()], cx);
        let handle = h.app.read_with(cx, |app, _| app.handle.clone());
        let dots = cx.new(|_| TypingDots {
            app: handle,
            step: 0,
            ticker: None,
        });
        dots.update(cx, |d, cx| d.keep_ticking(cx));
        cx.executor()
            .advance_clock(Duration::from_millis(BLINK_STEP_MS * 3));
        cx.run_until_parked();
        let stepped = dots.read_with(cx, |d, _| d.step);
        assert!(stepped >= 2, "{stepped}");

        set_typing(Vec::new(), cx);
        cx.executor()
            .advance_clock(Duration::from_millis(BLINK_STEP_MS * 3));
        cx.run_until_parked();
        assert!(
            dots.read_with(cx, |d, _| d.ticker.is_none()),
            "the ticker ended"
        );
        assert!(dots.read_with(cx, |d, _| d.step) <= stepped + 1);
    }

    /// A left click at a window position, for elements the test registry does not observe.
    fn click_point(window: &mut Window, position: gpui_kit::Point<Pixels>, cx: &mut App) {
        use gpui_kit::{
            InputEvent as _, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
        };
        window.render_frame(cx);
        window.dispatch_event(
            MouseMoveEvent {
                position,
                pressed_button: None,
                modifiers: Default::default(),
            }
            .to_platform_input(),
            cx,
        );
        window.dispatch_event(
            MouseDownEvent {
                button: MouseButton::Left,
                position,
                modifiers: Default::default(),
                click_count: 1,
                first_mouse: false,
            }
            .to_platform_input(),
            cx,
        );
        window.dispatch_event(
            MouseUpEvent {
                button: MouseButton::Left,
                position,
                modifiers: Default::default(),
                click_count: 1,
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
    }

    /// The `+` button toggles its menu, and a press anywhere else closes it.
    /// Revert to see it fail: drop the `plus_bounds` check in `render_plus_menu` (the button
    /// can then never close it), or the `on_mouse_down_out`.
    #[gpui_kit::test]
    fn the_plus_menu_toggles_and_closes_on_an_outside_press(cx: &mut TestAppContext) {
        let h = Harness::new(cx);
        let overlay = |cx: &mut TestAppContext| h.app.read_with(cx, |app, _| app.v.overlay);
        // The `+` cell (30px) ends 2px left of the text field, centred on its one row.
        let input = h.app.read_with(cx, |app, cx| {
            app.panes.composer.read(cx).input.entity_id().as_u64()
        });
        let plus = h.with_window(cx, |window, cx| {
            window.render_frame(cx);
            let id = gpui_kit::ElementId::NamedInteger("input".into(), input);
            let field = window.within("composer").find(id).bounds();
            gpui_kit::point(field.left() - px(17.), field.center().y)
        });
        let outside = gpui_kit::point(px(600.), px(200.));

        h.with_window(cx, |window, cx| click_point(window, plus, cx));
        assert_eq!(overlay(cx), Overlay::Plus);
        h.with_window(cx, |window, cx| click_point(window, plus, cx));
        assert_eq!(overlay(cx), Overlay::None);
        h.with_window(cx, |window, cx| click_point(window, plus, cx));
        assert_eq!(overlay(cx), Overlay::Plus);
        h.with_window(cx, |window, cx| click_point(window, outside, cx));
        assert_eq!(overlay(cx), Overlay::None);
    }

    /// Revert to see it fail: return `false` from `paste_image` for images.
    #[gpui_kit::test]
    fn a_pasted_image_is_offered_and_text_still_pastes(cx: &mut TestAppContext) {
        let mut h = Harness::new(cx);
        h.focus_composer(cx);
        cx.write_to_clipboard(ClipboardItem::new_string("tekst".into()));
        h.with_window(cx, |window, cx| window.press("secondary-v", cx));
        assert_eq!(h.text(cx), "tekst");

        let png = vec![0x89, b'P', b'N', b'G'];
        cx.write_to_clipboard(ClipboardItem {
            entries: vec![ClipboardEntry::Image(gpui_kit::Image::from_bytes(
                ImageFormat::Png,
                png.clone(),
            ))],
        });
        h.with_window(cx, |window, cx| window.press("secondary-v", cx));
        assert_eq!(h.text(cx), "tekst", "an image does not land in the text");
        let offered = h.drain().into_iter().any(|c| match c {
            UiCommand::BiedBestandAan(p, channel) => {
                let same = std::fs::read(&p).ok() == Some(png.clone());
                let _ = std::fs::remove_file(&p);
                channel == Channel::GENERAL && same
            }
            _ => false,
        });
        assert!(offered);
    }
}
