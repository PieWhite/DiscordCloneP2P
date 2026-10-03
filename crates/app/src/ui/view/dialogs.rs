//! Slice E: the dialog layer, drawn by `AppView` as a `deferred` layer above everything.
//! Every dialog is the same system window (`widgets::WindowFrame`: paper frame, blue title
//! strip, hard shadow) on the dithered backdrop (`widgets::modal_layer`):
//!
//! | Dialog | Old code | Does |
//! |---|---|---|
//! | Confirm | `askConfirm` | Cancel, or close and `ConfirmAction::run` |
//! | Prompt | `askText` / `submitPrompt` | field focused with its text selected; Enter or the button closes and runs `PromptAction::run`, Escape cancels |
//! | Picker | `openPicker` | one row per screen/window, a click closes and `commands::share_source` |
//! | Lightbox | `openLightbox` | fitted or actual size (click the picture to toggle), Open via `commands::open_file`, a click beside it closes |
//! | Wordle | `openWordle` | `wordle::render_panel`, keys through `wordle::press` |
//!
//! Escape is the root's (`AppView::on_escape`); a focused field lets it through.

use super::super::commands::SourceOption;
use super::super::icons::Icon;
use super::super::state::OpRef;
use super::super::theme::{self, c};
use super::widgets::{modal_layer, pointer, BackdropHandler, Btn, BtnKind, WindowFrame};
use super::wordle::{self, WordleKey};
use super::{AppHandle, AppSnapshot, ConfirmAction, Dialog, PromptAction};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{
    div, img, prelude::FluentBuilder, px, relative, size, AnyElement, App, AppContext, ClickEvent,
    Context, Entity, Focusable, InteractiveElement, IntoElement, KeyDownEvent, ObjectFit,
    ParentElement, Pixels, SharedString, Size, StatefulInteractiveElement, Styled, StyledImage,
    Subscription, Window,
};
use std::path::{Path, PathBuf};

pub fn render_dialog(
    snap: &AppSnapshot,
    dialog: &Dialog,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    match dialog {
        Dialog::Confirm {
            title,
            text,
            ok_label,
            danger,
            action,
        } => confirm(snap, title, text, ok_label, *danger, action),
        Dialog::Prompt {
            title,
            text,
            value,
            ok_label,
            action,
        } => prompt(snap, title, text, value, ok_label, action, window, cx),
        Dialog::Picker { sources } => picker(snap, sources),
        Dialog::Lightbox {
            src,
            name,
            op,
            actual_size,
        } => lightbox(snap, src, name, op.as_ref(), *actual_size, window, cx),
        Dialog::Wordle => wordle_dialog(snap, window, cx),
    }
}

fn close_button(
    snap: &AppSnapshot,
    id: &'static str,
    label: &'static str,
    kind: BtnKind,
) -> AnyElement {
    Btn::new(id, label)
        .kind(kind)
        .dialog()
        .on_click(
            snap.app
                .listener(|app, _: &ClickEvent, window, cx| app.close_dialog(window, cx)),
        )
        .into_any_element()
}

/// `.confirm-body p`.
fn paragraph(text: impl Into<SharedString>) -> impl IntoElement {
    div()
        .px(px(22.))
        .pt(px(16.))
        .pb(px(4.))
        .text_size(px(13.5))
        .text_color(c::PAPER)
        .line_height(relative(1.55))
        .child(text.into())
}

// ------------------------------------------------------------------ confirm

fn confirm(
    snap: &AppSnapshot,
    title: &str,
    text: &str,
    ok: &str,
    danger: bool,
    action: &ConfirmAction,
) -> AnyElement {
    let action = action.clone();
    let yes = Btn::new("confirm-yes", ok.to_string())
        .kind(if danger {
            BtnKind::Danger
        } else {
            BtnKind::Accent
        })
        .dialog()
        .on_click(snap.app.listener(move |app, _: &ClickEvent, window, cx| {
            app.close_dialog(window, cx);
            let ui = app.ui().clone();
            action.run(&ui);
        }));
    let frame = WindowFrame::new(title.to_string())
        .child(paragraph(text.to_string()))
        .foot(vec![
            close_button(snap, "confirm-no", "Cancel", BtnKind::Ghost),
            yes.into_any_element(),
        ]);
    modal_layer(frame, &snap.dialog_focus, None).into_any_element()
}

// ------------------------------------------------------------------ prompt

/// The prompt's text field, kept across renders while the dialog is open and dropped with
/// it, so the next prompt starts fresh.
struct PromptField {
    input: Entity<InputState>,
    /// Focus and select-all happen once, after the field has been drawn the first time.
    primed: bool,
    _enter: Subscription,
}

/// `submitPrompt`: close first, then act; an empty value does nothing (`PromptAction::run`).
fn submit_prompt(
    app: &AppHandle,
    action: &PromptAction,
    value: &str,
    window: &mut Window,
    cx: &mut App,
) {
    app.update(cx, |app, cx| {
        app.close_dialog(window, cx);
        let ui = app.ui().clone();
        action.run(&ui, value);
    });
}

#[allow(clippy::too_many_arguments)]
fn prompt(
    snap: &AppSnapshot,
    title: &str,
    text: &str,
    value: &str,
    ok: &str,
    action: &PromptAction,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let key = match action {
        PromptAction::NewChannel => SharedString::from("prompt-new"),
        PromptAction::RenameChannel(channel) => format!("prompt-rename-{channel}").into(),
    };
    let field = window.use_keyed_state(key, cx, |window, cx: &mut Context<PromptField>| {
        let value = value.to_string();
        let input = cx.new(|cx| InputState::new(window, cx).default_value(value));
        let (app, action) = (snap.app.clone(), action.clone());
        let enter = cx.subscribe_in(
            &input,
            window,
            move |_, input, ev: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = ev {
                    let value = input.read(cx).value().to_string();
                    submit_prompt(&app, &action, &value, window, cx);
                }
            },
        );
        PromptField {
            input,
            primed: false,
            _enter: enter,
        }
    });
    let (input, primed) = {
        let f = field.read(cx);
        (f.input.clone(), f.primed)
    };
    if !primed {
        field.update(cx, |f, _| f.primed = true);
        let target = input.clone();
        window.defer(cx, move |window, cx| {
            target.update(cx, |s, cx| {
                s.focus(window, cx);
                s.select_all(window, cx);
            })
        });
    }
    let focused = input.read(cx).focus_handle(cx).is_focused(window);

    // `.text-input`: a void well with a steel edge, silver on hover, cyan with focus.
    let well = div()
        .id("prompt-input")
        .mx(px(22.))
        .mt(px(12.))
        .mb(px(4.))
        .h(px(32.))
        .flex()
        .items_center()
        .px(px(10.))
        .border_1()
        .border_color(if focused {
            theme::ACCENT
        } else {
            theme::BORDER_STRONG
        })
        .when(!focused, |d| {
            d.hover(|s| s.border_color(theme::BORDER_INPUT))
        })
        .bg(c::SPACE)
        .child(
            Input::new(&input)
                .appearance(false)
                .text_size(px(13.5))
                .text_color(theme::TEXT)
                .px_0()
                .py_0(),
        );

    let (app, submit_action) = (snap.app.clone(), action.clone());
    let save_input = input.clone();
    let yes = Btn::new("prompt-yes", ok.to_string())
        .kind(BtnKind::Accent)
        .dialog()
        .on_click(move |_, window, cx| {
            let value = save_input.read(cx).value().to_string();
            submit_prompt(&app, &submit_action, &value, window, cx);
        });
    let frame = WindowFrame::new(title.to_string())
        .child(paragraph(text.to_string()))
        .child(well)
        .foot(vec![
            close_button(snap, "prompt-no", "Cancel", BtnKind::Ghost),
            yes.into_any_element(),
        ]);
    modal_layer(frame, &snap.dialog_focus, None).into_any_element()
}

// ------------------------------------------------------------------ picker

fn picker(snap: &AppSnapshot, sources: &[SourceOption]) -> AnyElement {
    let list = if sources.is_empty() {
        div()
            .px(px(22.))
            .pt(px(12.))
            .pb(px(16.))
            .text_size(px(13.))
            .text_color(c::SILVER)
            .child("Nothing was offered to capture.")
            .into_any_element()
    } else {
        div()
            .id("picker-list")
            .max_h(px(320.))
            .overflow_y_scroll()
            .px(px(12.))
            .pt(px(2.))
            .pb(px(12.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .children(sources.iter().map(|s| {
                let index = s.index;
                let group: SharedString = format!("picker-{index}").into();
                div()
                    .id(("picker-item", index))
                    .group(group.clone())
                    .relative()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(8.))
                    .w_full()
                    .h(px(32.))
                    .px(px(8.))
                    .text_size(px(14.))
                    .text_color(c::SILVER)
                    .cursor_pointer()
                    .hover(|st| st.bg(theme::BG_HOVER).text_color(theme::TEXT))
                    .child(
                        pointer()
                            .opacity(0.)
                            .group_hover(group, |st| st.opacity(1.)),
                    )
                    .child(
                        (if s.is_window {
                            Icon::File
                        } else {
                            Icon::Monitor
                        })
                        .size(15.),
                    )
                    .child(div().min_w_0().truncate().child(s.name.clone()))
                    .on_click(snap.app.listener(move |app, _: &ClickEvent, window, cx| {
                        app.close_dialog(window, cx);
                        let ui = app.ui().clone();
                        super::super::commands::share_source(&ui, index);
                    }))
            }))
            .into_any_element()
    };
    let frame = WindowFrame::new("Share a screen or window")
        .child(paragraph(
            "Nothing is captured until somebody actually watches. A shared source that nobody opened costs no encoder and no traffic.",
        ))
        .child(list)
        .foot(vec![close_button(snap, "picker-cancel", "Cancel", BtnKind::Ghost)]);
    modal_layer(frame, &snap.dialog_focus, None).into_any_element()
}

// ------------------------------------------------------------------ lightbox

/// The picture's size scaled down (never up) to fit `bounds` in both dimensions. The old
/// stylesheet bounded only the width, so a tall picture was cropped and scrolled.
pub fn fit_within(image: Size<Pixels>, bounds: Size<Pixels>) -> Size<Pixels> {
    if image.width <= px(0.) || image.height <= px(0.) {
        return bounds;
    }
    let scale = (bounds.width / image.width)
        .min(bounds.height / image.height)
        .min(1.);
    size(image.width * scale, image.height * scale)
}

fn toggle_size(app: &AppHandle) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    app.listener(|app, _: &ClickEvent, _, cx| {
        cx.stop_propagation();
        if let Some(Dialog::Lightbox { actual_size, .. }) = &mut app.v_mut().dialog {
            *actual_size = !*actual_size;
        }
        cx.notify();
    })
}

fn lightbox(
    snap: &AppSnapshot,
    src: &Path,
    name: &str,
    op: Option<&OpRef>,
    actual_size: bool,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let viewport = window.viewport_size();
    let bounds = size(viewport.width - px(52.), viewport.height - px(150.));
    let allowed = snap.ui.image_path_allowed(src);
    // The header only, once per opened picture.
    let natural = allowed
        .then(|| {
            let path: PathBuf = src.to_path_buf();
            let key = SharedString::from(format!("lightbox-size-{}", path.display()));
            *window
                .use_keyed_state(key, cx, move |_, _| {
                    image::image_dimensions(&path)
                        .ok()
                        .map(|(w, h)| size(px(w as f32), px(h as f32)))
                })
                .read(cx)
        })
        .flatten();

    let picture = allowed.then(|| {
        let image = img(src.to_path_buf())
            .id("lightbox-img")
            .flex_none()
            .on_click(toggle_size(&snap.app));
        match (natural, actual_size) {
            (Some(n), true) => image.w(n.width).h(n.height),
            (Some(n), false) => {
                let fitted = fit_within(n, bounds);
                image.w(fitted.width).h(fitted.height)
            }
            (None, _) => image
                .w(bounds.width)
                .h(bounds.height)
                .object_fit(ObjectFit::Contain),
        }
    });

    // Clicking the frame beside the picture closes, like a click on the backdrop.
    let close_beside = snap
        .app
        .listener(|app, _: &ClickEvent, window, cx| app.close_dialog(window, cx));
    let frame = div()
        .id("lightbox-frame")
        .flex()
        .max_w(bounds.width)
        .max_h(bounds.height)
        .bg(c::SPACE)
        .when(actual_size, |d| {
            d.overflow_scroll().items_start().justify_start()
        })
        .when(!actual_size, |d| d.items_center().justify_center())
        .on_click(close_beside)
        .children(picture);

    let open_op = op.cloned();
    let foot = div()
        .flex()
        .justify_end()
        .gap(px(8.))
        .px(px(14.))
        .py(px(12.))
        .child(
            Btn::new(
                "lightbox-size",
                if actual_size {
                    "Fit to window"
                } else {
                    "Actual size"
                },
            )
            .kind(BtnKind::Ghost)
            .dialog()
            .on_click(toggle_size(&snap.app)),
        )
        .child(
            Btn::new("lightbox-open", "Open")
                .kind(BtnKind::Ghost)
                .dialog()
                .on_click(snap.app.listener(move |app, _: &ClickEvent, _, _| {
                    if let Some(op) = &open_op {
                        super::super::commands::open_file(app.ui(), op);
                    }
                })),
        )
        .child(close_button(
            snap,
            "lightbox-close",
            "Close",
            BtnKind::Accent,
        ));

    let window_frame = WindowFrame::new(name.to_string())
        .width(None)
        .truncate_title()
        .max_w(viewport.width - px(48.))
        .child(frame)
        .child(foot);
    let backdrop: BackdropHandler = Box::new({
        let app = snap.app.clone();
        move |window, cx| {
            app.update(cx, |app, cx| app.close_dialog(window, cx));
        }
    });
    modal_layer(window_frame, &snap.dialog_focus, Some(backdrop)).into_any_element()
}

// ------------------------------------------------------------------ wordle

/// The board, the keyboard and the standings are taller than the smallest window, so the
/// window is a column whose middle scrolls; the title strip and Close stay put.
fn wordle_dialog(snap: &AppSnapshot, window: &mut Window, cx: &mut App) -> AnyElement {
    let viewport = window.viewport_size();
    let panel = div()
        .id("wordle-panel")
        .px(px(16.))
        .pt(px(14.))
        .min_h_0()
        .overflow_y_scroll()
        .child(wordle::render_panel(snap, window, cx));
    let frame = WindowFrame::new(wordle::dialog_title(&snap.s.wordle))
        .width(Some(340.))
        .max_h(viewport.height - px(32.))
        .child(panel)
        .foot(vec![close_button(
            snap,
            "wordle-close",
            "Close",
            BtnKind::Accent,
        )]);
    // Typing goes to the board while its window is open; the drawn keys take the same path.
    modal_layer(frame, &snap.dialog_focus, None)
        .on_key_down(snap.app.listener(|app, ev: &KeyDownEvent, _, cx| {
            if let Some(key) = WordleKey::from_key_down(ev) {
                cx.stop_propagation();
                wordle::press(app, key, cx);
            }
        }))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "Fit" bounds both dimensions and never enlarges. Dropping the height term (the old
    /// stylesheet's bug) fails the portrait case.
    #[test]
    fn fit_bounds_both_dimensions_and_never_enlarges() {
        let bounds = size(px(1128.), px(570.));
        let landscape = fit_within(size(px(1440.), px(810.)), bounds);
        assert!(
            (f32::from(landscape.height) - 570.).abs() < 0.01 && f32::from(landscape.width) < 1128.
        );
        let portrait = fit_within(size(px(900.), px(1200.)), bounds);
        assert!((f32::from(portrait.height) - 570.).abs() < 0.01);
        assert!((f32::from(portrait.width) - 427.5).abs() < 0.01);
        let small = fit_within(size(px(200.), px(100.)), bounds);
        assert_eq!(
            (f32::from(small.width), f32::from(small.height)),
            (200., 100.)
        );
    }
}
