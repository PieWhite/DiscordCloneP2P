//! Slice B: the timeline window: its blue head (icon, title, topic, member-list toggle)
//! and the message log: grouping (7 minutes), day lines, the unread "new" line
//! (`v.unread_at_open`), mention frames, replies, reactions and the quick emoji bar
//! (`v.emoji_for`), hover actions (reply sets `v.reply_to`, edit sets `v.editing`, delete
//! opens `Dialog::delete_message`), attachments in every transfer state, images (drawn only
//! if `ui.image_path_allowed`, lightbox via `Dialog::Lightbox`), YouTube cards
//! (`commands::youtube_preview`), Wordle cards (`wordle::render_card`) and system items.
//!
//! The log is a `gpui::list` aligned to the bottom: only the rows on screen are laid out,
//! and while it sits at the bottom it stays there when rows arrive or the composer grows.
//! A new timeline in the same conversation re-pins when the reader was within 24px of the
//! bottom (`PIN_SLACK`), as `render()` did; a conversation change always jumps to the newest.

mod attachment;
mod message;

use super::super::commands::{self, YoutubePreview};
use super::super::icons::Icon;
use super::super::state::{OpRef, PeerState, TimelineItem};
use super::super::theme::{self, c};
use super::super::Ui;
use super::widgets::tooltip;
use super::{format, markup, AppHandle, AppSnapshot, PaneInit, View};
use gpui_kit::base::{Scrollbar, ScrollbarMode};
use gpui_kit::{
    div, list, prelude::FluentBuilder, px, AnyElement, App, Context, InteractiveElement,
    IntoElement, ListAlignment, ListState, ParentElement, Pixels, Point, Render,
    StatefulInteractiveElement, Styled, WeakEntity, Window,
};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

/// `GROUP_WINDOW`: a continuation needs the same author within seven minutes.
const GROUP_WINDOW_MS: i64 = 7 * 60 * 1000;
/// `PIN_SLACK`: this close to the bottom still counts as reading the newest.
const PIN_SLACK: Pixels = px(24.);

// ------------------------------------------------------------------ rows

/// What `renderTimeline` decided for one timeline item before drawing it.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// A day line goes above this item.
    pub day: Option<String>,
    /// The "New" divider goes above this item (after the day line).
    pub new_line: bool,
    /// Drawn as a continuation: no avatar, no head, the time in the gutter on hover.
    pub grouped: bool,
    /// The time shown: the item's own, or the one before it (a file has none).
    pub at: i64,
}

fn item_at(item: &TimelineItem) -> i64 {
    match item {
        TimelineItem::Message { at, .. }
        | TimelineItem::File { at, .. }
        | TimelineItem::Wordle { at, .. } => *at,
    }
}

/// The loop of `renderTimeline`, without the drawing: day lines, the New line at
/// `len - unread_at_open`, and grouping (same author, both times known, under seven
/// minutes, not a mention of me, not a reply; a Wordle card or a separator breaks it).
pub fn build_rows(
    tl: &[TimelineItem],
    unread_at_open: usize,
    day_of: impl Fn(i64) -> String,
) -> Vec<Row> {
    let mut rows = Vec::with_capacity(tl.len());
    let mut last_day: Option<String> = None;
    let mut last_author: Option<&str> = None;
    let mut last_at = 0i64;
    let mut new_drawn = false;
    let unread_from = tl.len() as i64 - unread_at_open as i64;
    for (i, item) in tl.iter().enumerate() {
        let at = match item_at(item) {
            0 => last_at,
            at => at,
        };
        let mut row = Row {
            day: None,
            new_line: false,
            grouped: false,
            at,
        };
        if at != 0 {
            let day = day_of(at);
            if last_day.as_deref() != Some(day.as_str()) {
                row.day = Some(day.clone());
                last_day = Some(day);
                last_author = None;
            }
        }
        if !new_drawn && unread_from > 0 && i as i64 == unread_from {
            row.new_line = true;
            new_drawn = true;
            last_author = None;
        }
        let (author, mentions_you, reply) = match item {
            TimelineItem::Wordle { .. } => {
                last_author = None;
                last_at = if at != 0 { at } else { last_at };
                rows.push(row);
                continue;
            }
            TimelineItem::Message {
                author,
                mentions_you,
                reply_to,
                ..
            } => (author.as_str(), *mentions_you, reply_to.is_some()),
            TimelineItem::File { author, .. } => (author.as_str(), false, false),
        };
        row.grouped = last_author == Some(author)
            && at != 0
            && last_at != 0
            && at - last_at < GROUP_WINDOW_MS
            && !mentions_you
            && !reply;
        last_author = Some(author);
        last_at = if at != 0 { at } else { last_at };
        rows.push(row);
    }
    rows
}

/// The op of a message or file.
fn item_op(item: &TimelineItem) -> Option<&OpRef> {
    match item {
        TimelineItem::Message { id, .. } | TimelineItem::File { id, .. } => Some(id),
        TimelineItem::Wordle { .. } => None,
    }
}

fn item_mine(item: &TimelineItem) -> bool {
    matches!(
        item,
        TimelineItem::Message { mine: true, .. } | TimelineItem::File { mine: true, .. }
    )
}

// ------------------------------------------------------------------ the pane

/// A YouTube preview as the pane knows it: asked and waiting, or answered (`None` stays a
/// plain link, as before).
#[derive(Clone)]
enum Yt {
    Pending,
    Done(Option<YoutubePreview>),
}

/// What the list was last built from.
struct Shown {
    key: String,
    tl: Arc<Vec<TimelineItem>>,
    unread_at_open: usize,
    intro: bool,
}

pub struct TimelinePane {
    app: AppHandle,
    ui: Arc<Ui>,
    this: WeakEntity<Self>,
    list: ListState,
    rows: Rc<Vec<Row>>,
    shown: Option<Shown>,
    /// The row under the pointer, and the row whose action strip is under it (the strip
    /// pokes 12px into the row above, which must not take the hover from it).
    hover_row: Option<usize>,
    hover_strip: Option<usize>,
    yt: HashMap<String, Yt>,
    /// We share Spotify: track links carry the DJ's button (`spotify_button`).
    dj: bool,
    /// The press that closed the emoji bar from outside, so the click it ends on the same
    /// message's react button does not open it again (`data-react-open` toggled).
    emoji_closed_by: Option<(OpRef, Point<Pixels>)>,
    #[cfg(debug_assertions)]
    debug: Option<super::super::fixture::DebugView>,
}

impl TimelinePane {
    pub fn new(init: &PaneInit, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        TimelinePane {
            app: init.app.clone(),
            ui: init.ui.clone(),
            this: cx.entity().downgrade(),
            list: ListState::new(0, ListAlignment::Bottom, px(600.)),
            rows: Rc::new(Vec::new()),
            shown: None,
            hover_row: None,
            hover_strip: None,
            yt: HashMap::new(),
            dj: false,
            emoji_closed_by: None,
            #[cfg(debug_assertions)]
            debug: super::super::fixture::DebugView::from_env(),
        }
    }

    fn is_pinned(&self) -> bool {
        self.list.max_offset_for_scrollbar().y + self.list.scroll_px_offset_for_scrollbar().y
            <= PIN_SLACK
    }

    /// Brings the list in line with the timeline it is drawing. Runs every render; does
    /// nothing unless the conversation, the timeline or the unread mark changed.
    fn sync_list(&mut self, snap: &AppSnapshot, intro: bool, cx: &mut Context<Self>) {
        // Starting or stopping Spotify adds or drops the DJ's button under track links:
        // remeasure those rows, so one scrolled out of view does not keep its old height.
        let dj = snap.own_music().is_some();
        if dj != self.dj {
            self.dj = dj;
            if let Some(s) = &self.shown {
                let offset = usize::from(s.intro);
                for (i, item) in s.tl.iter().enumerate() {
                    if let TimelineItem::Message { body, .. } = item {
                        if !markup::spotify_tracks(body).is_empty() {
                            self.list.remeasure_items(offset + i..offset + i + 1);
                        }
                    }
                }
            }
        }
        let key = snap.active_channel();
        let unread = snap.v.unread_at_open;
        let same = self.shown.as_ref().is_some_and(|s| {
            s.key == key
                && Arc::ptr_eq(&s.tl, &snap.tl)
                && s.unread_at_open == unread
                && s.intro == intro
        });
        if same {
            return;
        }
        let rows = build_rows(&snap.tl, unread, format::fmt_day);
        let offset = usize::from(intro);
        let count = rows.len() + offset;
        match self.shown.as_ref() {
            Some(s)
                if s.key == key && s.intro == intro && !s.tl.is_empty() && !snap.tl.is_empty() =>
            {
                // The same conversation moved on: replace only what changed, so the rows
                // around keep their measured height and the reader keeps their place.
                let pinned = self.is_pinned();
                let (old_tl, new_tl) = (&s.tl, &snap.tl);
                let same_at =
                    |i: usize, j: usize| old_tl[i] == new_tl[j] && self.rows[i] == rows[j];
                let max = old_tl.len().min(new_tl.len());
                let prefix = (0..max).take_while(|&i| same_at(i, i)).count();
                let suffix = (0..max - prefix)
                    .take_while(|&k| same_at(old_tl.len() - 1 - k, new_tl.len() - 1 - k))
                    .count();
                // What I just sent lands at the end: show it, wherever the reader was, as
                // the webview did after sending (the composer cannot reach this list).
                let sent = new_tl
                    .last()
                    .filter(|i| item_mine(i))
                    .and_then(item_op)
                    .is_some_and(|op| !old_tl.iter().rev().any(|o| item_op(o) == Some(op)));
                self.list.splice(
                    offset + prefix..offset + old_tl.len() - suffix,
                    new_tl.len() - prefix - suffix,
                );
                if pinned || sent {
                    self.list.scroll_to_end();
                }
            }
            _ => {
                self.list.reset(count);
                self.hover_row = None;
                self.hover_strip = None;
            }
        }
        self.rows = Rc::new(rows);
        self.shown = Some(Shown {
            key,
            tl: snap.tl.clone(),
            unread_at_open: unread,
            intro,
        });
        self.want_previews(snap, cx);
        #[cfg(debug_assertions)]
        self.apply_debug(snap, offset);
    }

    /// `hydrateYt`: one `youtube_preview` per video id for the life of the process.
    fn want_previews(&mut self, snap: &AppSnapshot, cx: &mut Context<Self>) {
        let wanted: HashSet<String> = snap
            .tl
            .iter()
            .filter_map(|i| match i {
                TimelineItem::Message { body, .. } => Some(markup::yt_ids(body)),
                _ => None,
            })
            .flatten()
            .filter(|id| !self.yt.contains_key(id))
            .collect();
        for id in wanted {
            let ask = commands::youtube_preview(&self.ui, &id);
            // A fixture window draws one frame behind a locked screen; it reads its cache
            // synchronously so that frame already has the card. Never a real run.
            #[cfg(debug_assertions)]
            if self.ui.fixture.is_some() {
                let preview = self.ui.runtime.block_on(ask);
                self.yt.insert(id, Yt::Done(preview));
                continue;
            }
            self.yt.insert(id.clone(), Yt::Pending);
            cx.spawn(async move |this, cx| {
                let preview = ask.await;
                let _ = this.update(cx, |pane, cx| {
                    pane.yt.insert(id.clone(), Yt::Done(preview));
                    // A card is ~100px: remeasure the rows that carry it, so one scrolled
                    // out of view does not keep its old height.
                    let offset = pane.shown.as_ref().map_or(0, |s| usize::from(s.intro));
                    if let Some(s) = &pane.shown {
                        for (i, item) in s.tl.iter().enumerate() {
                            if let TimelineItem::Message { body, .. } = item {
                                if body.contains(id.as_str()) {
                                    pane.list.remeasure_items(offset + i..offset + i + 1);
                                }
                            }
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        }
    }

    /// Debug builds: `scroll=top|<item>[+<px>]` and `hover=<item>|last` from
    /// `FITCOM_DEBUG_VIEW`, applied to the first list that has rows.
    #[cfg(debug_assertions)]
    fn apply_debug(&mut self, snap: &AppSnapshot, offset: usize) {
        if snap.tl.is_empty() {
            return;
        }
        let Some(d) = self.debug.take() else { return };
        if let Some(scroll) = d.scroll.as_deref() {
            let (ix, px_in) = scroll.split_once('+').unwrap_or((scroll, "0"));
            let item_ix = if ix == "top" {
                0
            } else {
                ix.parse::<usize>().map_or(0, |i| i + offset)
            };
            self.list.scroll_to(gpui_kit::ListOffset {
                item_ix,
                offset_in_item: px(px_in.parse().unwrap_or(0.)),
            });
        }
        if let Some(hover) = d.hover.as_deref() {
            let ix = if hover == "last" {
                snap.tl
                    .iter()
                    .rposition(|i| matches!(i, TimelineItem::Message { .. }))
            } else {
                hover.parse().ok()
            };
            self.hover_row = ix.map(|i| i + offset);
        }
    }

    fn set_hover_row(&mut self, ix: usize, hovered: bool, cx: &mut Context<Self>) {
        let before = self.hover_row;
        if hovered {
            self.hover_row = Some(ix);
        } else if self.hover_row == Some(ix) {
            self.hover_row = None;
        }
        if before != self.hover_row {
            cx.notify();
        }
    }

    fn set_hover_strip(&mut self, ix: usize, hovered: bool, cx: &mut Context<Self>) {
        let before = self.hover_strip;
        if hovered {
            self.hover_strip = Some(ix);
        } else if self.hover_strip == Some(ix) {
            self.hover_strip = None;
        }
        if before != self.hover_strip {
            cx.notify();
        }
    }

    /// `data-jump-reply`: the quote is a button; bring the original into view when it is
    /// in this conversation's log.
    fn jump_to(&mut self, op: &OpRef, cx: &mut Context<Self>) {
        let Some(s) = &self.shown else { return };
        if let Some(i) = s.tl.iter().position(|i| item_op(i) == Some(op)) {
            self.list.scroll_to_reveal_item(i + usize::from(s.intro));
            cx.notify();
        }
    }
}

/// Everything one row needs, shared by every row of a render.
pub(super) struct RowCtx {
    snap: AppSnapshot,
    rows: Rc<Vec<Row>>,
    /// 1 when the DM intro is row 0.
    offset: usize,
    dm_peer: Option<PeerState>,
    hover: Option<usize>,
    yt: Rc<HashMap<String, Option<YoutubePreview>>>,
    pane: WeakEntity<TimelinePane>,
}

impl Render for TimelinePane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(snap) = self.app.snapshot(cx) else {
            return div().into_any_element();
        };
        let dm_peer = (snap.v.view == View::Dms)
            .then(|| snap.v.dm.as_deref().and_then(|id| snap.peer(id)).cloned())
            .flatten();
        let intro = dm_peer.is_some();
        self.sync_list(&snap, intro, cx);

        if snap.tl.is_empty() {
            return match &dm_peer {
                Some(peer) => message::render_dm_empty(peer).into_any_element(),
                None => message::render_empty(&snap.active_name()).into_any_element(),
            };
        }

        let yt: HashMap<String, Option<YoutubePreview>> = self
            .yt
            .iter()
            .filter_map(|(id, y)| match y {
                Yt::Done(p) => Some((id.clone(), p.clone())),
                Yt::Pending => None,
            })
            .collect();
        let ctx = Rc::new(RowCtx {
            snap,
            rows: self.rows.clone(),
            offset: usize::from(intro),
            dm_peer,
            hover: self.hover_strip.or(self.hover_row),
            yt: Rc::new(yt),
            pane: self.this.clone(),
        });
        div()
            .relative()
            .size_full()
            .child(
                list(self.list.clone(), move |ix, window, cx| {
                    message::render_row(&ctx, ix, window, cx)
                })
                .size_full()
                .pt(px(18.))
                .pb(px(12.)),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .right_0()
                    .bottom_0()
                    .w(px(12.))
                    .child(timeline_scrollbar(&self.list)),
            )
            .into_any_element()
    }
}

/// The thin steel elevator the webview drew (4px, 3px off the edge), silver under the
/// pointer. Same width in every state, so the kit has no width animation to run.
fn timeline_scrollbar(list: &ListState) -> Scrollbar {
    let thumb = |colour: gpui_kit::Rgba| {
        move |t: gpui_kit::base::ScrollbarThumbStyle| {
            t.bg(colour).width(px(4.)).inset(px(3.)).radius(px(0.))
        }
    };
    Scrollbar::vertical(list)
        .id("timeline-scrollbar")
        .mode(ScrollbarMode::Always)
        .styles(|s| {
            s.thumb(thumb(c::STEEL))
                .thumb_hover(thumb(c::SILVER))
                .thumb_active(thumb(c::SILVER))
        })
}

// ------------------------------------------------------------------ head

/// A 28px square in the blue strip. Pressed on blue is pressed *in*: the void.
fn head_btn(id: &'static str, icon: Icon, pressed: bool) -> gpui_kit::Stateful<gpui_kit::Div> {
    div()
        .id(id)
        .size(px(28.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .when(pressed, |d| d.bg(c::SPACE).text_color(c::PAPER))
        .hover(|s| s.bg(c::PAPER).text_color(theme::ON_INVERT))
        .child(icon.size(16.))
}

/// `renderHead`: the 44px blue strip on top of the timeline window.
pub fn render_head(snap: &AppSnapshot, window: &mut Window, _cx: &mut App) -> AnyElement {
    let dm = (snap.v.view == View::Dms)
        .then_some(())
        .and(snap.v.dm.as_ref());
    let (icon, title, topic) = match dm {
        Some(id) => (
            Icon::At,
            snap.peer(id).map(|p| p.name.clone()).unwrap_or_default(),
            "Direct message \u{00b7} never relayed through the third peer",
        ),
        None => {
            let channel = snap.channel(&snap.v.channel).or(snap.s.channels.first());
            let topic = if channel.is_some_and(|c| c.key == "general") {
                "Everyone sees this channel"
            } else {
                "Sub-channel \u{00b7} everyone sees this too"
            };
            (
                Icon::Hash,
                channel.map_or_else(|| "general".to_string(), |c| c.name.clone()),
                topic,
            )
        }
    };
    let narrow = window.viewport_size().width <= theme::NARROW_CHANNELS;
    div()
        .h(theme::HEAD_H)
        .flex_none()
        .flex()
        .items_center()
        .gap(px(8.))
        .pl(px(16.))
        .pr(px(8.))
        .min_w_0()
        .bg(c::BLUE)
        .border_b_1()
        .border_color(c::SPACE)
        .text_color(c::PAPER)
        .child(icon.size(18.))
        .child(
            div()
                .flex_none()
                .text_size(px(15.))
                .whitespace_nowrap()
                .child(title),
        )
        .when(!narrow, |d| {
            d.child(
                div()
                    .min_w_0()
                    .pl(px(12.))
                    .ml(px(4.))
                    .border_l_1()
                    .border_color(c::SKY)
                    .text_size(px(13.))
                    .text_color(c::PAPER)
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(topic),
            )
        })
        .child(
            div()
                .ml_auto()
                .flex()
                .flex_none()
                .gap(px(2.))
                // The webview's Search button had no handler; kept as the same dead square.
                .child(head_btn("search", Icon::Search, false).tooltip(tooltip("Search")))
                .child(
                    head_btn("toggle-members", Icon::Users, snap.v.members)
                        .tooltip(tooltip("Toggle member list"))
                        .on_click(snap.app.listener(|app, _, _, cx| app.toggle_members(cx))),
                ),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::state::{ReplySnippet, TransferState};

    fn msg(author: &str, at: i64, mentions_you: bool, reply: bool) -> TimelineItem {
        TimelineItem::Message {
            id: OpRef {
                author: author.into(),
                channel: "general".into(),
                seq: at as u64,
            },
            author: author.into(),
            author_name: author.into(),
            avatar: 1,
            body: "x".into(),
            at,
            edited: false,
            mine: false,
            mentions_you,
            reply_to: reply.then(|| OpRef {
                author: "z".into(),
                channel: "general".into(),
                seq: 1,
            }),
            reply_snippet: reply.then(|| ReplySnippet {
                author_name: "z".into(),
                body: "y".into(),
            }),
            reactions: Vec::new(),
        }
    }

    fn file(author: &str) -> TimelineItem {
        TimelineItem::File {
            id: OpRef {
                author: author.into(),
                channel: "general".into(),
                seq: 99,
            },
            author: author.into(),
            author_name: author.into(),
            avatar: 1,
            name: "f".into(),
            size: 1,
            at: 0,
            mine: false,
            transfer: TransferState::Available,
            image_path: None,
            local_path: None,
            opens_folder: false,
        }
    }

    const MIN: i64 = 60 * 1000;

    /// Each arm of the grouping rule, the day line and the New line. Drop the
    /// `!mentions_you` term and row 3 groups; drop the `0 => last_at` fallback and the file
    /// stops grouping.
    #[test]
    fn rows_follow_render_timeline() {
        let day = |ms: i64| {
            if ms < 1000 * MIN {
                "Yesterday".to_string()
            } else {
                "Today".to_string()
            }
        };
        let tl = vec![
            msg("a", 10 * MIN, false, false),
            msg("a", 16 * MIN, false, false), // within 7 minutes: grouped
            msg("a", 30 * MIN, false, false), // too late
            msg("a", 31 * MIN, true, false),  // mentions me
            msg("a", 32 * MIN, false, true),  // a reply
            file("a"),                        // no time: takes the last, groups
            msg("a", 1001 * MIN, false, false), // new day
            msg("a", 1002 * MIN, false, false), // New line breaks the group
        ];
        let rows = build_rows(&tl, 1, day);
        let grouped: Vec<bool> = rows.iter().map(|r| r.grouped).collect();
        assert_eq!(
            grouped,
            [false, true, false, false, false, true, false, false]
        );
        assert_eq!(rows[0].day.as_deref(), Some("Yesterday"));
        assert_eq!(rows[6].day.as_deref(), Some("Today"));
        assert_eq!(rows[5].at, 32 * MIN);
        let new: Vec<usize> = rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.new_line)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(new, [7]);
        assert!(build_rows(&tl, 0, day).iter().all(|r| !r.new_line));
    }

    #[test]
    fn a_wordle_card_ends_a_group() {
        let tl = vec![
            msg("a", 10 * MIN, false, false),
            TimelineItem::Wordle {
                day: 20261003,
                at: 11 * MIN,
                number: None,
                action: "past".into(),
                progress: 0,
                results: Vec::new(),
                scored: false,
            },
            msg("a", 12 * MIN, false, false),
        ];
        let rows = build_rows(&tl, 0, |_| "Today".into());
        assert!(!rows[2].grouped);
    }
}
