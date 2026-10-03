//! One row of the log: the separators above it, then a message or file (`renderMessage`)
//! or a Wordle card; and the conversation-level blocks (`dm-intro`, `dm-empty`, `.empty`).

use super::super::super::icons::Icon;
use super::super::super::state::{OpRef, PeerState, ReactionState, TimelineItem};
use super::super::super::theme::{self, c, Dither};
use super::super::widgets::{avatar, tooltip};
use super::super::{commands, format, markup, wordle, Dialog, ReplyTo};
use super::{attachment, Row, RowCtx};
use gpui_kit::{
    div, prelude::FluentBuilder, px, relative, AnyElement, App, ClickEvent, ClipboardItem, Div,
    ElementId, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window,
};

/// `QUICK_EMOJI`: eight cover what this chat uses; the pill row takes any emoji already there.
pub const QUICK_EMOJI: [&str; 8] = ["👍", "❤️", "😂", "😮", "😢", "🎉", "🔥", "👀"];

/// The grey smiley of the react button (`.act-smiley`, `filter: grayscale(1)`): a mask, so
/// the eyes and mouth are holes. The colour emoji takes its place under the pointer.
const SMILEY_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path fill-rule="evenodd" fill="#000" d="M8 1.2a6.8 6.8 0 1 1 0 13.6A6.8 6.8 0 0 1 8 1.2zM5.4 5.3h1.3v2.3H5.4zm3.9 0h1.3v2.3H9.3zM4.5 9.2c.8 1.6 2 2.4 3.5 2.4s2.7-.8 3.5-2.4l-1-.5c-.6 1.2-1.4 1.8-2.5 1.8s-1.9-.6-2.5-1.8z"/></svg>"##;

fn op_key(op: &OpRef) -> SharedString {
    format!("{}:{}:{}", op.author, op.channel, op.seq).into()
}

/// The list's row `ix`: the DM intro, or a timeline item with its separators.
pub(super) fn render_row(ctx: &RowCtx, ix: usize, window: &mut Window, cx: &mut App) -> AnyElement {
    if ix < ctx.offset {
        return match &ctx.dm_peer {
            Some(peer) => render_dm_intro(peer).into_any_element(),
            None => div().into_any_element(),
        };
    }
    let i = ix - ctx.offset;
    let (Some(item), Some(row)) = (ctx.snap.tl.get(i), ctx.rows.get(i)) else {
        return div().into_any_element();
    };
    div()
        .w_full()
        .flex()
        .flex_col()
        .when_some(row.day.clone(), |d, day| d.child(day_line(day)))
        .when(row.new_line, |d| d.child(new_line()))
        .child(match item {
            // Not a message: no author, no gutter, flush with the log's left edge.
            TimelineItem::Wordle { .. } => div()
                .w_full()
                .max_w(px(440.))
                .mt(px(6.))
                .mb(px(2.))
                .child(wordle::render_card(&ctx.snap, item, window, cx))
                .into_any_element(),
            _ => render_message(ctx, ix, item, row, window, cx),
        })
        .into_any_element()
}

/// `.day`: the date between two hairlines.
fn day_line(day: String) -> Div {
    let rule = || div().flex_1().h(px(1.)).bg(theme::BORDER);
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .mt(px(22.))
        .mb(px(18.))
        .mx(px(16.))
        .text_size(px(11.))
        .text_color(c::SILVER)
        .child(rule())
        .child(day.to_uppercase())
        .child(rule())
}

/// `.newline`: "New" in magenta between two dithered 2px rules, the right one 24px.
fn new_line() -> Div {
    let rule = || {
        div()
            .relative()
            .h(px(2.))
            .bg(c::MAGENTA)
            .child(theme::dither(Dither::Half, c::SPACE, 2.))
    };
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .mt(px(22.))
        .mb(px(18.))
        .mx(px(16.))
        .text_size(px(11.))
        .text_color(c::MAGENTA)
        .child(rule().flex_1())
        .child("NEW")
        .child(rule().w(px(24.)).flex_none())
}

struct Who {
    name: String,
    avatar: u8,
}

/// `renderMessage`'s `author`: myself, the peer as the roster knows it now, or what the
/// item carried.
fn who(ctx: &RowCtx, author: &str, author_name: &str, avatar_no: u8, mine: bool) -> Who {
    if mine {
        return Who {
            name: ctx.snap.s.me.name.clone(),
            avatar: ctx.snap.s.me.avatar,
        };
    }
    match ctx.snap.peer(author) {
        Some(p) => Who {
            name: p.name.clone(),
            avatar: p.avatar,
        },
        None => Who {
            name: author_name.to_string(),
            avatar: avatar_no,
        },
    }
}

fn render_message(
    ctx: &RowCtx,
    ix: usize,
    item: &TimelineItem,
    row: &Row,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let (id, author, author_name, avatar_no, mine, mentions_you, edited) = match item {
        TimelineItem::Message {
            id,
            author,
            author_name,
            avatar,
            mine,
            mentions_you,
            edited,
            ..
        } => (
            id,
            author,
            author_name,
            *avatar,
            *mine,
            *mentions_you,
            *edited,
        ),
        TimelineItem::File {
            id,
            author,
            author_name,
            avatar,
            mine,
            ..
        } => (id, author, author_name, *avatar, *mine, false, false),
        TimelineItem::Wordle { .. } => return div().into_any_element(),
    };
    let key = op_key(id);
    let who = who(ctx, author, author_name, avatar_no, mine);
    let grouped = row.grouped;
    let hovered = ctx.hover == Some(ix);
    let time = if row.at != 0 {
        format::fmt_time(row.at)
    } else {
        String::new()
    };
    let pane = ctx.pane.clone();

    let gutter = div()
        .w(px(40.))
        .flex_none()
        .flex()
        .flex_col()
        .items_center()
        .map(|d| {
            if grouped {
                d.pt(px(4.))
                    .text_size(px(10.))
                    .text_color(c::SILVER)
                    .when(!hovered, |d| d.invisible())
                    .child(time.clone())
            } else {
                d.child(avatar(&who.name, who.avatar, 40., None, false))
            }
        });

    let mut body = div().flex_1().min_w_0().relative().flex().flex_col();
    if let TimelineItem::Message {
        reply_to: Some(_), ..
    } = item
    {
        body = body.pt(px(2.));
    }
    if !grouped {
        body = body.child(
            div()
                .flex()
                .items_baseline()
                .gap(px(8.))
                .mb(px(1.))
                .child(
                    div()
                        .text_size(px(14.))
                        .text_color(theme::avatar_hue(who.avatar))
                        .child(author_name.clone()),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(c::SILVER)
                        .child(time.clone()),
                )
                .when(edited, |d| d.child(edited_mark())),
        );
    }
    if let TimelineItem::Message {
        reply_to: Some(parent),
        reply_snippet,
        ..
    } = item
    {
        body = body.child(quote(ctx, &key, parent.clone(), reply_snippet.as_ref()));
    }
    body = body.children(content(ctx, item, &key, window, cx));
    // Fixed here (webview bug 5): a grouped message has no head, so its marker goes under
    // the text.
    if grouped && edited {
        body = body.child(edited_mark());
    }
    if let TimelineItem::Message { reactions, .. } = item {
        if !reactions.is_empty() {
            body = body.child(reaction_row(ctx, &key, id, reactions));
        }
        if ctx.snap.v.emoji_for.as_ref() == Some(id) {
            body = body.child(emoji_bar(ctx, &key, id));
        }
    }

    div()
        .id(ElementId::Name(key.clone()))
        .relative()
        .flex()
        .flex_row()
        .gap(px(12.))
        .px(px(16.))
        .py(px(if grouped { 1. } else { 2. }))
        .when(!grouped, |d| d.mt(px(14.)))
        .when(hovered, |d| d.bg(theme::BG_MSG_HOVER))
        // Inset by 12px so the inner padding is 4px: the avatar stays on the same x.
        .when(mentions_you, |d| {
            d.bg(theme::MENTION_BG)
                .border_1()
                .border_color(theme::MENTION_BORDER)
                .mt(px(14.))
                .mx(px(12.))
                .mb(px(2.))
                .pt(px(8.))
                .pr(px(12.))
                .pb(px(10.))
                .pl(px(3.))
        })
        .on_hover(move |hovered, _, cx| {
            let _ = pane.update(cx, |p, cx| p.set_hover_row(ix, *hovered, cx));
        })
        .child(gutter)
        .child(body)
        .when(hovered, |d| d.child(actions(ctx, ix, item, &key)))
        .into_any_element()
}

fn edited_mark() -> Div {
    div()
        .text_size(px(10.))
        .text_color(c::SILVER)
        .child("(edited)")
}

/// `.msg-quote`: who wrote the parent and how it opens, or that it is gone. A click brings
/// the parent into view (the webview drew it as a button without a handler).
fn quote(
    ctx: &RowCtx,
    key: &SharedString,
    parent: OpRef,
    snippet: Option<&super::super::super::state::ReplySnippet>,
) -> AnyElement {
    let pane = ctx.pane.clone();
    div()
        .id(ElementId::Name(format!("{key}-quote").into()))
        // A `<button>`: as wide as its content, up to 480px.
        .self_start()
        .flex()
        // Both runs are 12px, so centre is the baseline; taffy's baseline pass also
        // measured the text unconstrained, which defeated the ellipsis.
        .items_center()
        .gap(px(6.))
        .max_w(px(480.))
        .mb(px(4.))
        .pt(px(3.))
        .pb(px(3.))
        .pr(px(10.))
        .pl(px(8.))
        .border_l_2()
        .border_color(theme::BORDER_STRONG)
        .text_color(c::SILVER)
        .text_size(px(12.))
        .cursor_pointer()
        .hover(|s| s.bg(theme::BG_HOVER))
        .on_click(move |_, _, cx| {
            let _ = pane.update(cx, |p, cx| p.jump_to(&parent, cx));
        })
        .map(|d| match snippet {
            Some(s) => d
                .child(
                    div()
                        .flex_none()
                        .text_color(c::PAPER)
                        .child(s.author_name.clone()),
                )
                // `white-space: nowrap` folded the parent's line breaks into spaces.
                .child(ellipsis_line(
                    s.body.split_whitespace().collect::<Vec<_>>().join(" "),
                )),
            None => d.child(ellipsis_line("Original message unavailable".into())),
        })
        .into_any_element()
}

/// One line cut off with an ellipsis. GPUI truncates only text whose width the layout
/// hands it, so the box grows from zero (`flex_1`) instead of from its content.
fn ellipsis_line(text: String) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .child(text)
}

/// `itemContent`: the edit field, the body plus its YouTube cards, or the file.
fn content(
    ctx: &RowCtx,
    item: &TimelineItem,
    key: &SharedString,
    window: &mut Window,
    cx: &mut App,
) -> Vec<AnyElement> {
    match item {
        TimelineItem::Message {
            id,
            body,
            mentions_you,
            ..
        } => {
            if ctx.snap.v.editing.as_ref() == Some(id) {
                return vec![super::super::composer::render_edit_box(
                    &ctx.snap, id, body, window, cx,
                )];
            }
            let mut out = markup::render_body(&ctx.snap, key, body, *mentions_you);
            out.extend(
                markup::yt_ids(body)
                    .into_iter()
                    .filter_map(|yt| ctx.yt.get(&yt).cloned().flatten().map(|p| (yt, p)))
                    .filter_map(|(yt, p)| attachment::youtube_card(ctx, key, &yt, &p)),
            );
            out
        }
        TimelineItem::File { .. } => vec![attachment::render_file(ctx, item, key)],
        TimelineItem::Wordle { .. } => Vec::new(),
    }
}

/// The names behind a pill, for its tooltip (the webview showed the raw peer ids).
fn reaction_names(ctx: &RowCtx, r: &ReactionState) -> String {
    r.peers
        .iter()
        .map(|id| {
            if *id == ctx.snap.s.me.id {
                ctx.snap.s.me.name.clone()
            } else {
                ctx.snap
                    .peer(id)
                    .map_or_else(|| id.clone(), |p| p.name.clone())
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// `.pill`: a small cell under the body; the add pill opens the quick bar.
fn pill(id: ElementId, mine: bool) -> gpui_kit::Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(5.))
        .px(px(8.))
        .py(px(1.))
        .border_1()
        .border_color(if mine { c::STEEL } else { theme::BORDER })
        .bg(if mine { c::VIOLET } else { theme::BG_CANVAS })
        .text_color(if mine { c::PAPER } else { c::SILVER })
        .text_size(px(12.))
        .line_height(px(18.))
        .cursor_pointer()
        .hover(|s| s.bg(theme::BG_HOVER))
}

fn reaction_row(
    ctx: &RowCtx,
    key: &SharedString,
    op: &OpRef,
    reactions: &[ReactionState],
) -> AnyElement {
    let ui = ctx.snap.ui.clone();
    div()
        .flex()
        .flex_wrap()
        .gap(px(4.))
        .mt(px(4.))
        .children(reactions.iter().enumerate().map(|(k, r)| {
            let ui = ui.clone();
            let op = op.clone();
            let emoji = r.emoji.clone();
            pill(
                ElementId::NamedInteger(format!("{key}-pill").into(), k as u64),
                r.mine,
            )
            .tooltip(tooltip(reaction_names(ctx, r)))
            // Toggling is decided on the engine side, which knows whether we reacted.
            .on_click(move |_, _, _| commands::react_message(&ui, &op, &emoji))
            .child(
                div()
                    .text_size(px(14.))
                    .line_height(relative(1.))
                    .child(r.emoji.clone()),
            )
            .child(r.count.to_string())
        }))
        .child(
            pill(ElementId::Name(format!("{key}-pill-add").into()), false)
                .tooltip(tooltip("Add reaction"))
                .on_click(react_open(ctx, op))
                .child("+"),
        )
        .into_any_element()
}

/// `data-react-open`: toggles the quick bar of this message. A press that already closed
/// it from outside (`on_mouse_down_out` below) must not reopen it on release.
fn react_open(ctx: &RowCtx, op: &OpRef) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    let pane = ctx.pane.clone();
    let app = ctx.snap.app.clone();
    let op = op.clone();
    move |ev, _, cx| {
        let closed_now = pane
            .update(cx, |p, _| {
                let closed = p.emoji_closed_by.take();
                matches!((closed, ev), (Some((o, at)), ClickEvent::Mouse(m)) if o == op && at == m.down.position)
            })
            .unwrap_or(false);
        let op = op.clone();
        app.update(cx, move |app, cx| {
            let v = app.v_mut();
            v.emoji_for = if closed_now || v.emoji_for.as_ref() == Some(&op) {
                None
            } else {
                Some(op)
            };
            cx.notify();
        });
    }
}

/// `.emoji-bar`: the quick picks, in the flow under the message (the webview floated it).
fn emoji_bar(ctx: &RowCtx, key: &SharedString, op: &OpRef) -> AnyElement {
    let pane = ctx.pane.clone();
    let app = ctx.snap.app.clone();
    let closing = op.clone();
    div()
        .id(ElementId::Name(format!("{key}-emoji-bar").into()))
        .self_start()
        .flex()
        .gap(px(2.))
        .mt(px(2.))
        .p(px(4.))
        .bg(theme::BG_RAISED)
        .border_1()
        .border_color(theme::BORDER_STRONG)
        .shadow(theme::shadow_popover())
        .on_mouse_down_out(move |ev, _, cx| {
            let _ = pane.update(cx, |p, _| {
                p.emoji_closed_by = Some((closing.clone(), ev.position))
            });
            app.update(cx, |app, cx| {
                app.v_mut().emoji_for = None;
                cx.notify();
            });
        })
        .children(QUICK_EMOJI.iter().map(|e| {
            let ui = ctx.snap.ui.clone();
            let app = ctx.snap.app.clone();
            let op = op.clone();
            div()
                .id(ElementId::Name(format!("{key}-emoji-{e}").into()))
                .px(px(6.))
                .py(px(4.))
                .text_size(px(16.))
                .cursor_pointer()
                .hover(|s| s.bg(theme::BG_HOVER))
                .tooltip(tooltip(format!("React with {e}")))
                .on_click(move |_, _, cx| {
                    commands::react_message(&ui, &op, e);
                    app.update(cx, |app, cx| {
                        app.v_mut().emoji_for = None;
                        cx.notify();
                    });
                })
                .child(*e)
        }))
        .into_any_element()
}

/// `.msg-actions`: the strip on the row's top edge, half over the row above.
fn actions(ctx: &RowCtx, ix: usize, item: &TimelineItem, key: &SharedString) -> AnyElement {
    let (id, mine, message_body, author_name) = match item {
        TimelineItem::Message {
            id,
            mine,
            body,
            author_name,
            ..
        } => (id, *mine, Some(body.clone()), author_name.clone()),
        TimelineItem::File {
            id,
            mine,
            author_name,
            ..
        } => (id, *mine, None, author_name.clone()),
        TimelineItem::Wordle { .. } => return div().into_any_element(),
    };
    let button = |name: &str| {
        div()
            .id(ElementId::Name(format!("{key}-act-{name}").into()))
            .w(px(30.))
            .h(px(28.))
            .flex()
            .items_center()
            .justify_center()
            .text_color(c::SILVER)
            .cursor_pointer()
    };
    let plain =
        |d: gpui_kit::Stateful<Div>| d.hover(|s| s.bg(c::PAPER).text_color(theme::ON_INVERT));
    let pane = ctx.pane.clone();
    let mut strip = div()
        .id(ElementId::Name(format!("{key}-actions").into()))
        .absolute()
        .top(px(-12.))
        .right(px(16.))
        .flex()
        .bg(c::INK)
        .border_1()
        .border_color(theme::BORDER_STRONG)
        .shadow(theme::shadow_bar())
        .block_mouse_except_scroll()
        .on_hover(move |hovered, _, cx| {
            let _ = pane.update(cx, |p, cx| p.set_hover_strip(ix, *hovered, cx));
        });

    if let Some(body) = &message_body {
        let app = ctx.snap.app.clone();
        let reply = ReplyTo {
            op: id.clone(),
            name: author_name.clone(),
            body: body.chars().take(90).collect(),
        };
        strip = strip.child(
            plain(button("reply"))
                .tooltip(tooltip("Reply"))
                .on_click(move |_, _, cx| {
                    let reply = reply.clone();
                    app.update(cx, move |app, cx| {
                        app.v_mut().reply_to = Some(reply);
                        cx.notify();
                    });
                })
                .child(Icon::Msg.size(15.)),
        );
        let group: SharedString = format!("{key}-act-react").into();
        strip = strip.child(
            plain(button("react"))
                .group(group.clone())
                .relative()
                .tooltip(tooltip("Add reaction"))
                .on_click(react_open(ctx, id))
                .child(
                    div().group_hover(group.clone(), |s| s.invisible()).child(
                        gpui_kit::component::Icon::default()
                            .data(SMILEY_SVG)
                            .size(px(13.)),
                    ),
                )
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(13.))
                        .line_height(px(15.))
                        .invisible()
                        .group_hover(group, |s| s.visible())
                        .child("🙂"),
                ),
        );
    }
    if mine && message_body.is_some() {
        let app = ctx.snap.app.clone();
        let op = id.clone();
        strip = strip.child(
            plain(button("edit"))
                .tooltip(tooltip("Edit"))
                .on_click(move |_, _, cx| {
                    let op = op.clone();
                    app.update(cx, move |app, cx| {
                        app.v_mut().editing = Some(op);
                        cx.notify();
                    });
                })
                .child(Icon::Edit.size(15.)),
        );
    }
    if mine {
        let op = id.clone();
        strip = strip.child(
            button("delete")
                .hover(|s| s.bg(c::CORAL).text_color(theme::ON_SIGNAL))
                .tooltip(tooltip("Delete"))
                .on_click(ctx.snap.app.listener(move |app, _, window, cx| {
                    app.open_dialog(Dialog::delete_message(op.clone()), window, cx)
                }))
                .child(Icon::Trash.size(15.)),
        );
    }
    if let (false, Some(body)) = (mine, message_body) {
        strip = strip.child(
            plain(button("copy"))
                .tooltip(tooltip("Copy text"))
                .on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(body.clone()))
                })
                .child(Icon::File.size(15.)),
        );
    }
    strip.into_any_element()
}

// ------------------------------------------------------------------ conversation blocks

/// `.dm-intro`: whose DM this is and why it is never relayed.
pub(super) fn render_dm_intro(peer: &PeerState) -> Div {
    div()
        .pt(px(6.))
        .px(px(16.))
        .pb(px(4.))
        .child(avatar(&peer.name, peer.avatar, 40., None, false))
        .child(div().mt(px(10.)).mb(px(4.)).text_size(px(19.)).child(peer.name.clone()))
        .child(
            // `max-width: 60ch` at 13px, plus the trailing space GPUI's wrapper counts.
            div()
                .max_w(px(397.))
                .text_size(px(13.))
                .line_height(relative(1.55))
                .text_color(c::SILVER)
                .child("This conversation stays between the two of you. It is never relayed through the third peer, because there is no encryption on the wire and relaying would mean letting them read it."),
        )
}

/// A DM with no messages: the intro, then who it reaches and when, on the composer.
pub(super) fn render_dm_empty(peer: &PeerState) -> Div {
    let when = if peer.presence == super::super::super::state::Presence::Online {
        "straight away"
    } else {
        "as soon as he is back"
    };
    div()
        .size_full()
        .flex()
        .flex_col()
        .justify_end()
        .pt(px(18.))
        .pb(px(12.))
        .child(render_dm_intro(peer))
        .child(
            div()
                .mt(px(20.))
                .mx(px(16.))
                .text_size(px(13.))
                .text_color(c::SILVER)
                .child(format!(
                    "No messages yet. Anything you send reaches {} {when}.",
                    peer.name
                )),
        )
}

/// `.empty`: a channel nothing was posted in.
pub(super) fn render_empty(name: &str) -> Div {
    div().size_full().pt(px(18.)).pb(px(12.)).flex().items_center().justify_center().child(
        div()
            .p(px(40.))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(6.))
            .child(
                div()
                    .size(px(48.))
                    .mb(px(6.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(c::INK)
                    .border_1()
                    .border_color(c::VIOLET)
                    .shadow(theme::hard_shadow(3.))
                    .text_color(c::SILVER)
                    .child(Icon::Hash.size(20.)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .child(div().text_size(px(16.)).child(format!("#{name} is empty")))
                    .child(div().mt(px(8.)).w(px(96.)).child(theme::band(px(4.)))),
            )
            .child(
                // `max-width: 44ch` at 13px, plus the trailing space GPUI's wrapper counts.
                div()
                    .max_w(px(293.))
                    .text_size(px(13.))
                    .line_height(relative(1.55))
                    .text_color(c::SILVER)
                    .text_center()
                    .child("Nothing has been posted here yet. Anything you send stays in this channel and reaches the others as soon as they are online."),
            ),
    )
}
