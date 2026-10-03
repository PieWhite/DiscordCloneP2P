//! The non-text rows of a message: file cards in every transfer state (`attachmentCard`,
//! `openButton`), pictures (`.shot`) and YouTube cards (`ytCard`).
//!
//! Security, unchanged from the webview: a button hands back the item's `OpRef`, never a
//! path (`download_file`, `open_file` resolve it in the engine), a file the system would run
//! says Show and gets its folder (`opens_folder`), and an image is drawn only from a path
//! `Ui::image_path_allowed` accepts.

use super::super::super::commands::{self, YoutubePreview};
use super::super::super::icons::Icon;
use super::super::super::state::{Presence, TimelineItem, TransferState};
use super::super::super::theme::{self, c};
use super::super::super::Ui;
use super::super::widgets::{tooltip, Btn, BtnKind};
use super::super::{format, Dialog};
use super::RowCtx;
use gpui_kit::{
    div, img, px, relative, AnyElement, Div, ElementId, ImageSource, InteractiveElement,
    IntoElement, ObjectFit, ParentElement, SharedString, StatefulInteractiveElement, Styled, StyledImage,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

thread_local! {
    /// Width and height per picture, read from the file header once, so the row has its
    /// final height in the first layout and pinning does not land above the newest message.
    static SIZES: RefCell<HashMap<PathBuf, Option<(u32, u32)>>> = RefCell::new(HashMap::new());
}

fn picture_size(path: &Path) -> Option<(u32, u32)> {
    SIZES.with(|s| {
        *s.borrow_mut()
            .entry(path.to_path_buf())
            .or_insert_with(|| image::image_dimensions(path).ok().filter(|(w, h)| *w > 0 && *h > 0))
    })
}

/// A picture from disk for `img()`, or `None` when the path is not one this window may
/// read. Debug fixture runs decode it right here: their window draws a single frame behind
/// a locked screen, so an asynchronous load would never show up. Never in a real run.
pub(super) fn picture(ui: &Ui, path: &Path) -> Option<ImageSource> {
    if !ui.image_path_allowed(path) {
        return None;
    }
    #[cfg(debug_assertions)]
    if ui.fixture.is_some() {
        return fixture_picture(path);
    }
    Some(ImageSource::from(path.to_path_buf()))
}

#[cfg(debug_assertions)]
fn fixture_picture(path: &Path) -> Option<ImageSource> {
    use gpui_kit::RenderImage;
    use std::sync::Arc;
    thread_local! {
        static DECODED: RefCell<HashMap<PathBuf, Option<Arc<RenderImage>>>> = RefCell::new(HashMap::new());
    }
    DECODED.with(|d| {
        d.borrow_mut()
            .entry(path.to_path_buf())
            .or_insert_with(|| {
                let mut rgba = image::open(path).ok()?.to_rgba8();
                // GPUI images are BGRA.
                for px in rgba.chunks_exact_mut(4) {
                    px.swap(0, 2);
                }
                Some(Arc::new(RenderImage::new(vec![image::Frame::new(rgba)])))
            })
            .clone()
            .map(ImageSource::from)
    })
}

/// `.att`: one bordered row with the file glyph, name, sub line and an action.
fn card(icon: Div, name: &str, sub: Vec<AnyElement>, action: Option<AnyElement>) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .mt(px(6.))
        .mb(px(2.))
        .px(px(11.))
        .py(px(9.))
        .max_w(px(440.))
        .border_1()
        .border_color(theme::BORDER)
        .bg(c::INK)
        .child(icon)
        .child(
            div()
                .min_w_0()
                .flex_1()
                .flex()
                .flex_col()
                .child(
                    div()
                        .text_size(px(13.))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(name.to_string()),
                )
                .children(sub),
        )
        .children(action)
        .into_any_element()
}

/// `.att-ico`: a 34px well; coral when the transfer failed.
fn glyph(icon: Icon, error: bool) -> Div {
    div()
        .size(px(34.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .bg(c::SPACE)
        .border_1()
        .border_color(c::VIOLET)
        .text_color(if error { theme::DANGER_TEXT } else { c::SILVER })
        .child(icon.size(18.))
}

/// `.att-sub`.
fn sub(text: impl Into<SharedString>, error: bool) -> AnyElement {
    div()
        .text_size(px(11.5))
        .line_height(relative(1.4))
        .text_color(if error { theme::DANGER_TEXT } else { c::SILVER })
        .child(text.into())
        .into_any_element()
}

/// `.bar`: the cells of a running transfer; silver while it waits for its sender.
fn bar(pct: f64, paused: bool) -> AnyElement {
    div()
        .mt(px(6.))
        .h(px(6.))
        .bg(c::SPACE)
        .border_1()
        .border_color(c::VIOLET)
        .overflow_hidden()
        .child(
            div()
                .h_full()
                .w(relative(pct.clamp(0., 1.) as f32))
                .bg(if paused { c::SILVER } else { theme::ACCENT }),
        )
        .into_any_element()
}

/// `openButton`: once the bytes are here the card opens them, or shows their folder for
/// something the system would run.
fn open_button(ctx: &RowCtx, key: &SharedString, item: &TimelineItem) -> Option<AnyElement> {
    let TimelineItem::File { id, local_path: Some(_), opens_folder, .. } = item else {
        return None;
    };
    let ui = ctx.snap.ui.clone();
    let op = id.clone();
    let btn = Btn::new(ElementId::Name(format!("{key}-open").into()), if *opens_folder { "Show" } else { "Open" })
        .icon(Icon::Open)
        .on_click(move |_, _, _| commands::open_file(&ui, &op));
    Some(if *opens_folder {
        div()
            .id(ElementId::Name(format!("{key}-open-tip").into()))
            .tooltip(tooltip("Show it in its folder. A file the system would run is never started from here."))
            .child(btn)
            .into_any_element()
    } else {
        btn.into_any_element()
    })
}

fn download_button(ctx: &RowCtx, key: &SharedString, item: &TimelineItem, label: &'static str, icon: Icon) -> AnyElement {
    let TimelineItem::File { id, .. } = item else {
        return div().into_any_element();
    };
    let ui = ctx.snap.ui.clone();
    let op = id.clone();
    Btn::new(ElementId::Name(format!("{key}-download").into()), label)
        .icon(icon)
        .on_click(move |_, _, _| commands::download_file(&ui, &op))
        .into_any_element()
}

/// `attachmentCard`, or the picture when its bytes are here.
pub(super) fn render_file(ctx: &RowCtx, item: &TimelineItem, key: &SharedString) -> AnyElement {
    let TimelineItem::File { id, author, name, size, transfer, image_path, .. } = item else {
        return div().into_any_element();
    };
    if let Some(path) = image_path.as_deref().map(Path::new) {
        if let Some(shot) = render_shot(ctx, key, id.clone(), path, name, *size) {
            return shot;
        }
    }
    let author_peer = ctx.snap.peer(author);
    match transfer {
        TransferState::Mine => card(
            glyph(Icon::File, false),
            name,
            vec![sub(format!("{} \u{00b7} offered by you", format::fmt_size(*size)), false)],
            open_button(ctx, key, item),
        ),
        TransferState::Done => card(
            glyph(Icon::File, false),
            name,
            vec![sub(format!("{} \u{00b7} in your download folder", format::fmt_size(*size)), false)],
            Some(open_button(ctx, key, item).unwrap_or_else(|| {
                Btn::new(ElementId::Name(format!("{key}-downloaded").into()), "Downloaded")
                    .icon(Icon::Check)
                    .kind(BtnKind::Ghost)
                    .disabled(true)
                    .into_any_element()
            })),
        ),
        TransferState::Failed { error } => card(
            glyph(Icon::Alert, true),
            name,
            vec![sub(error.clone(), true)],
            Some(download_button(ctx, key, item, "Try again", Icon::Retry)),
        ),
        TransferState::Running { received, total } => {
            // "Paused" is no engine state: a running transfer whose offerer went away.
            let stalled = author_peer.filter(|p| p.presence != Presence::Online);
            let pct = if *total > 0 { *received as f64 / *total as f64 } else { 0. };
            let mut lines = vec![
                sub(
                    match stalled {
                        Some(p) => format!(
                            "Paused at {} of {} \u{00b7} {} went offline",
                            format::fmt_size(*received),
                            format::fmt_size(*total),
                            p.name
                        ),
                        None => format!(
                            "{} of {} \u{00b7} {}%",
                            format::fmt_size(*received),
                            format::fmt_size(*total),
                            (pct * 100.).round()
                        ),
                    },
                    false,
                ),
                bar(pct, stalled.is_some()),
            ];
            if stalled.is_some() {
                lines.push(sub(
                    format!(
                        "Continues from {} by itself when he is back. Nothing already transferred is sent again.",
                        format::fmt_size(*received)
                    ),
                    false,
                ));
            }
            card(
                glyph(Icon::File, false),
                name,
                lines,
                Some(
                    Btn::new(ElementId::Name(format!("{key}-busy").into()), if stalled.is_some() { "Waiting" } else { "Downloading" })
                        .disabled(true)
                        .into_any_element(),
                ),
            )
        }
        TransferState::Available => card(
            glyph(Icon::File, false),
            name,
            vec![sub(format::fmt_size(*size), false)],
            Some(download_button(ctx, key, item, "Download", Icon::Download)),
        ),
    }
}

/// `.shot`: the picture at the card's width (420px), its name and size under it; a click
/// opens the lightbox.
fn render_shot(
    ctx: &RowCtx,
    key: &SharedString,
    op: super::super::super::state::OpRef,
    path: &Path,
    name: &str,
    size: u64,
) -> Option<AnyElement> {
    let source = picture(&ctx.snap.ui, path)?;
    let (w, h) = picture_size(path)?;
    let src = path.to_path_buf();
    let lightbox_name = name.to_string();
    Some(
        div()
            .mt(px(6.))
            .mb(px(2.))
            .max_w(px(420.))
            .border_1()
            .border_color(theme::BORDER)
            .bg(c::SPACE)
            .overflow_hidden()
            .child(
                div()
                    .id(ElementId::Name(format!("{key}-shot").into()))
                    .w_full()
                    .cursor_pointer()
                    .tooltip(tooltip("Show at full size"))
                    .on_click(ctx.snap.app.listener(move |app, _, window, cx| {
                        app.open_dialog(
                            Dialog::Lightbox {
                                src: src.clone(),
                                name: lightbox_name.clone(),
                                op: Some(op.clone()),
                                actual_size: false,
                            },
                            window,
                            cx,
                        )
                    }))
                    // The box takes the card's width and the picture's ratio; the image
                    // fills it. On the image itself a relative width makes GPUI fall back
                    // to the picture's own pixel height.
                    .child(
                        div().relative().w_full().aspect_ratio(w as f32 / h as f32).child(
                            img(source)
                                .absolute()
                                .top_0()
                                .left_0()
                                .size_full()
                                .aspect_ratio(w as f32 / h as f32)
                                .object_fit(ObjectFit::Fill),
                        ),
                    ),
            )
            .child(
                div()
                    .px(px(9.))
                    .py(px(5.))
                    .text_size(px(11.))
                    .text_color(c::SILVER)
                    .bg(c::INK)
                    .child(format!("{name} \u{00b7} {}", format::fmt_size(size))),
            )
            .into_any_element(),
    )
}

/// `ytCard`: thumbnail with the play plate, title (two lines at most) and channel. The
/// whole card is the link.
pub(super) fn youtube_card(ctx: &RowCtx, key: &SharedString, id: &str, p: &YoutubePreview) -> Option<AnyElement> {
    let thumb = picture(&ctx.snap.ui, &p.thumbnail)?;
    let url = format!("https://www.youtube.com/watch?v={id}");
    Some(
        div()
            .id(ElementId::Name(format!("{key}-yt-{id}").into()))
            .flex()
            .mt(px(6.))
            .mb(px(2.))
            .max_w(px(440.))
            .border_1()
            .border_color(theme::BORDER)
            .bg(c::INK)
            .text_color(theme::TEXT)
            .cursor_pointer()
            .hover(|s| s.border_color(theme::ACCENT))
            .on_click(move |_, _, _| commands::open_link(&url))
            .child(
                div()
                    .relative()
                    .flex_none()
                    .w(px(160.))
                    .h(px(90.))
                    .overflow_hidden()
                    .bg(c::SPACE)
                    // An explicit ratio, or the image's own 4:3 overrides the 90px height.
                    .child(img(thumb).w(px(160.)).h(px(90.)).aspect_ratio(160. / 90.).object_fit(ObjectFit::Cover))
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                div()
                                    .w(px(38.))
                                    .h(px(28.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .bg(c::SPACE)
                                    .border_1()
                                    .border_color(c::PAPER)
                                    .text_color(c::PAPER)
                                    .child(Icon::Play.size(20.)),
                            ),
                    ),
            )
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .justify_center()
                    .gap(px(4.))
                    .px(px(11.))
                    .py(px(8.))
                    .child(div().text_size(px(13.)).line_height(relative(1.35)).line_clamp(2).child(p.title.clone()))
                    .child(
                        div().text_size(px(11.5)).text_color(c::SILVER).child(if p.author.is_empty() {
                            "YouTube".to_string()
                        } else {
                            format!("YouTube \u{00b7} {}", p.author)
                        }),
                    ),
            )
            .into_any_element(),
    )
}
