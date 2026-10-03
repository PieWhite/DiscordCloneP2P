//! Slice B: the timeline window — its blue head (icon, title, topic, member-list toggle
//! via `AppView::toggle_members`) and the message log: grouping (7 minutes), day lines, the
//! unread "new" line (`v.unread_at_open`), mention frames, replies, reactions and the quick
//! emoji bar (`v.emoji_for`), hover actions (reply sets `v.reply_to`, edit sets `v.editing`,
//! delete opens `Dialog::delete_message`), attachments in every transfer state, images
//! (drawn only if `ui.image_path_allowed`, lightbox via `Dialog::Lightbox`), YouTube cards
//! (`commands::youtube_preview`), Wordle cards (`wordle::render_card`) and system items.
//!
//! The log reads `snap.tl`, which `AppView` refetches by itself. Stay pinned to the bottom
//! unless the user scrolled up more than 24px; jump to the newest on a conversation change.

use super::super::icons::Icon;
use super::super::theme::{self, c};
use super::widgets::placeholder;
use super::{AppHandle, AppSnapshot, PaneInit, View};
use gpui_kit::{div, px, AnyElement, App, Context, IntoElement, ParentElement, Render, Styled, Window};

pub struct TimelinePane {
    app: AppHandle,
}

impl TimelinePane {
    pub fn new(init: &PaneInit, _window: &mut Window, _cx: &mut Context<Self>) -> Self {
        TimelinePane { app: init.app.clone() }
    }
}

impl Render for TimelinePane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(snap) = self.app.snapshot(cx) else {
            return div().into_any_element();
        };
        div()
            .size_full()
            .child(placeholder(
                "",
                None,
                &format!("{} timeline items in {} \u{00b7} slice B", snap.tl.len(), snap.active_channel()),
            ))
            .into_any_element()
    }
}

/// The 44px blue strip on top of the timeline window.
pub fn render_head(snap: &AppSnapshot, _window: &mut Window, _cx: &mut App) -> AnyElement {
    let dm = snap.v.view == View::Dms && snap.v.dm.is_some();
    div()
        .h(theme::HEAD_H)
        .flex_none()
        .flex()
        .items_center()
        .gap(px(8.))
        .pl(px(16.))
        .pr(px(8.))
        .bg(c::BLUE)
        .border_b_1()
        .border_color(c::SPACE)
        .text_color(c::PAPER)
        .child((if dm { Icon::At } else { Icon::Hash }).size(18.))
        .child(div().text_size(px(15.)).child(snap.active_name()))
        .child(div().text_size(px(12.)).child("\u{00b7} head (slice B)"))
        .into_any_element()
}
