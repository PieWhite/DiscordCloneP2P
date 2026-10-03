//! Slice C: the composer window and everything around it — the error banner and typing
//! line above it (`render_error`/typing inside this pane), the reply chip (`v.reply_to`),
//! the multiline auto-growing input (Enter sends, Shift+Enter breaks, 4096-byte limit with
//! the byte count), editing mode, @-autocomplete (`v.overlay`, `v.ac_*`), the attach button
//! (`commands::pick_and_offer_file`), the `+` menu (`commands::post_wordle_card`), pasted
//! images (`commands::offer_pasted_image`), `notify_typing` throttled with
//! `v.last_typing_sent`, and the drop overlay over the main column.
//!
//! Drawn without `.cached()` because its height follows its content.

use super::super::theme::{self, c};
use super::{AppHandle, AppSnapshot, PaneInit};
use gpui_kit::{div, px, AnyElement, App, Context, IntoElement, ParentElement, Render, Styled, Window};

pub struct ComposerPane {
    app: AppHandle,
    /// Placeholder for the input's text until the real input lands.
    text: String,
}

impl ComposerPane {
    pub fn new(init: &PaneInit, _window: &mut Window, _cx: &mut Context<Self>) -> Self {
        ComposerPane {
            app: init.app.clone(),
            text: String::new(),
        }
    }

    /// Contract with `AppView::after_conversation_change`: put `draft` in the input and
    /// return what was in it, which `AppView` keeps as the draft of the conversation being
    /// left. Called only when the open conversation really changes.
    pub fn swap_draft(&mut self, draft: &str, _window: &mut Window, cx: &mut Context<Self>) -> String {
        cx.notify();
        std::mem::replace(&mut self.text, draft.to_string())
    }
}

impl Render for ComposerPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(snap) = self.app.snapshot(cx) else {
            return div();
        };
        div().px(px(16.)).pb(px(18.)).child(
            div()
                .h(px(40.))
                .flex()
                .items_center()
                .px(px(10.))
                .bg(c::INK)
                .border_1()
                .border_color(theme::BORDER_STRONG)
                .shadow(theme::chassis_shadow(2.))
                .text_color(c::SILVER)
                .text_size(px(13.))
                .child(format!("Message {} \u{00b7} composer (slice C)", snap.active_name())),
        )
    }
}

/// The "Drop to share" overlay, drawn by `AppView` over the main column while
/// `snap.drop_active` holds.
pub fn render_drop_overlay(_snap: &AppSnapshot, _window: &mut Window, _cx: &mut App) -> AnyElement {
    div()
        .absolute()
        .top(px(8.))
        .left(px(8.))
        .right(px(8.))
        .bottom(px(8.))
        .bg(c::INDIGO)
        .child(theme::dither(theme::Dither::Half, c::SPACE, 2.))
        .border_2()
        .border_color(theme::ACCENT)
        .into_any_element()
}
