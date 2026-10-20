//! The titlebar: brand, drag region, window buttons, a hairline along its bottom edge.
//! Slice A owns the look; the window mechanics below are the foundation's and
//! should stay as they are.
//!
//! - **macOS**: native traffic lights (placed by `traffic_light_position` in
//!   `ui/mod.rs`), our buttons hidden, the brand inset by `MAC_TITLEBAR_INSET`. The window
//!   is `app_owns_titlebar_drag`, so the strip moves the window itself: mouse down then
//!   move starts `start_window_move`, a double click does what the system setting says.
//! - **Windows**: frameless; the strip is `WindowControlArea::Drag` and the three buttons
//!   are `Min`/`Max`/`Close`, so the OS hit-tests them (snap layouts, Aero shake, the close
//!   request going through `on_window_should_close` and thus `commands::close_window`).

use super::super::icons::Icon;
use super::super::theme;
use super::AppSnapshot;
use gpui_kit::{
    div, prelude::FluentBuilder, px, AnyElement, App, InteractiveElement, IntoElement, MouseButton,
    ParentElement, StatefulInteractiveElement, Styled, Window,
};

pub fn render(_snap: &AppSnapshot, window: &mut Window, cx: &mut App) -> AnyElement {
    let mac = cfg!(target_os = "macos");
    // "The mouse went down on the strip and has not moved yet": the gpui-kit TitleBar
    // recipe: moving with the button held hands the drag to the window manager.
    let pressed = window.use_keyed_state("titlebar-pressed", cx, |_, _| false);

    let brand = div()
        .flex()
        .items_center()
        .gap(px(10.))
        .child(div().text_color(theme::ACCENT).child(Icon::Brand.size(16.)))
        .child(
            div()
                .text_size(px(13.))
                .font_weight(gpui_kit::FontWeight::MEDIUM)
                .text_color(theme::TEXT_STRONG)
                .child("FitCommunication"),
        );

    div()
        .id("titlebar")
        .relative()
        .h(theme::TITLEBAR_H)
        .flex_none()
        .flex()
        .items_center()
        .bg(theme::BG_SIDEBAR)
        .border_b_1()
        .border_color(theme::BORDER)
        .pl(if mac {
            theme::MAC_TITLEBAR_INSET
        } else {
            px(16.)
        })
        .when(mac, |d| {
            let down = pressed.clone();
            let up = pressed.clone();
            let moved = pressed.clone();
            d.on_mouse_down(MouseButton::Left, move |_, _, cx| {
                down.update(cx, |p, _| *p = true)
            })
            .on_mouse_up(MouseButton::Left, move |_, _, cx| {
                up.update(cx, |p, _| *p = false)
            })
            .on_mouse_move(move |_, window, cx| {
                if moved.read(cx).to_owned() {
                    moved.update(cx, |p, _| *p = false);
                    window.start_window_move();
                }
            })
            .on_click(|ev, window, _| {
                if ev.click_count() == 2 {
                    window.titlebar_double_click();
                }
            })
        })
        .child(
            div()
                .id("titlebar-drag")
                .flex()
                .flex_1()
                .h_full()
                .items_center()
                .when(!mac, |d| {
                    d.window_control_area(gpui_kit::WindowControlArea::Drag)
                })
                .child(brand),
        )
        .when(!mac, |d| d.child(window_buttons()))
        .into_any_element()
}

/// 46×35, muted glyphs; hover lifts the button, the close button goes red.
/// No click handlers on purpose: the OS acts on the hit-test area.
fn window_buttons() -> impl IntoElement {
    use gpui_kit::WindowControlArea as Area;
    let button = |id: &'static str, icon: Icon, area: Area, close: bool| {
        div()
            .id(id)
            .w(px(46.))
            .h(px(35.))
            .flex()
            .items_center()
            .justify_center()
            .text_color(theme::TEXT_MUTED)
            .window_control_area(area)
            .hover(move |s| {
                if close {
                    s.bg(theme::DANGER_BG).text_color(theme::DANGER_TEXT)
                } else {
                    s.bg(theme::BG_HOVER).text_color(theme::TEXT_STRONG)
                }
            })
            .child(icon.size(16.))
    };
    div()
        .flex()
        .h_full()
        .items_start()
        .child(button("win-min", Icon::Min, Area::Min, false))
        .child(button("win-max", Icon::Max, Area::Max, false))
        .child(button("win-close", Icon::X, Area::Close, true))
}
