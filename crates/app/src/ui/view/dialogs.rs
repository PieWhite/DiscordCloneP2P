//! Slice E: the dialog layer and the channel context menu, both drawn by `AppView` as
//! `deferred` layers above everything.
//!
//! `render_dialog` draws `Dialog` inside `widgets::modal_layer` (dithered backdrop, focus on
//! `snap.dialog_focus`) with a `widgets::WindowFrame`: confirm (`ConfirmAction::run`),
//! prompt (`PromptAction::run`), source picker (`commands::share_source`), lightbox (fit /
//! actual size, open via `commands::open_file`, the image only if
//! `ui.image_path_allowed`), and Wordle (`wordle::render_board`). Close with
//! `AppView::close_dialog`; Escape already does.

use super::super::theme::c;
use super::widgets::{modal_layer, Btn, BtnKind, WindowFrame};
use super::{AppSnapshot, ContextMenu, Dialog};
use gpui_kit::{
    div, px, AnyElement, App, InteractiveElement, IntoElement, MouseButton, ParentElement, Styled, Window,
};

pub fn render_dialog(snap: &AppSnapshot, dialog: &Dialog, _window: &mut Window, _cx: &mut App) -> AnyElement {
    let title = match dialog {
        Dialog::Confirm { title, .. } | Dialog::Prompt { title, .. } => title.clone(),
        Dialog::Picker { .. } => "Share a screen or window".into(),
        Dialog::Lightbox { name, .. } => name.clone(),
        Dialog::Wordle => "Wordle".into(),
    };
    let close = Btn::new("dialog-close", "Close")
        .kind(BtnKind::Accent)
        .dialog()
        .on_click(snap.app.listener(|app, _, window, cx| app.close_dialog(window, cx)));
    let frame = WindowFrame::new(title)
        .child(
            div()
                .px(px(22.))
                .pt(px(16.))
                .text_size(px(13.5))
                .child(format!("{dialog:?}").chars().take(160).collect::<String>()),
        )
        .foot(vec![close.into_any_element()]);
    modal_layer(frame, &snap.dialog_focus, None).into_any_element()
}

/// Right-click on a removable sub-channel: rename (`Dialog::rename_channel`) and delete
/// (`Dialog::delete_channel`). A click anywhere else closes it.
pub fn render_context_menu(snap: &AppSnapshot, menu: &ContextMenu, _window: &mut Window, _cx: &mut App) -> AnyElement {
    div()
        .id("context-menu-layer")
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .on_mouse_down(MouseButton::Left, snap.app.listener(|app, _, _, cx| app.close_context_menu(cx)))
        .child(
            div()
                .absolute()
                .left(menu.position.x)
                .top(menu.position.y)
                .p(px(8.))
                .bg(c::INK)
                .border_1()
                .border_color(c::STEEL)
                .text_size(px(12.))
                .child(format!("menu for {} \u{00b7} slice E", menu.channel)),
        )
        .into_any_element()
}
