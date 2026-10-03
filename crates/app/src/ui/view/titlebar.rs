//! The titlebar: brand, drag region, window buttons, and the 3px horizon band along its
//! bottom edge. Slice A owns the look; the window mechanics below are the foundation's and
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
use super::super::theme::{self, c};
use super::AppSnapshot;
use gpui_kit::{
    div, prelude::FluentBuilder, px, AnyElement, App, InteractiveElement, IntoElement, MouseButton,
    ParentElement, StatefulInteractiveElement, Styled, Window,
};

pub fn render(_snap: &AppSnapshot, window: &mut Window, cx: &mut App) -> AnyElement {
    let mac = cfg!(target_os = "macos");
    // "The mouse went down on the strip and has not moved yet" — the gpui-kit TitleBar
    // recipe: moving with the button held hands the drag to the window manager.
    let pressed = window.use_keyed_state("titlebar-pressed", cx, |_, _| false);

    let brand = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(div().text_color(c::CYAN).child(Icon::Brand.size(16.)))
        .child(
            div()
                .text_size(px(12.))
                .text_color(c::SILVER)
                .child("FITCOMMUNICATION"),
        );

    div()
        .id("titlebar")
        .relative()
        .h(theme::TITLEBAR_H)
        .flex_none()
        .flex()
        .items_center()
        .bg(c::INK)
        .pl(if mac { theme::MAC_TITLEBAR_INSET } else { px(10.) })
        .when(mac, |d| {
            let down = pressed.clone();
            let up = pressed.clone();
            let moved = pressed.clone();
            d.on_mouse_down(MouseButton::Left, move |_, _, cx| down.update(cx, |p, _| *p = true))
                .on_mouse_up(MouseButton::Left, move |_, _, cx| up.update(cx, |p, _| *p = false))
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
                .when(!mac, |d| d.window_control_area(gpui_kit::WindowControlArea::Drag))
                .child(brand),
        )
        .when(!mac, |d| d.child(window_buttons()))
        .child(
            div()
                .absolute()
                .left_0()
                .bottom_0()
                .w_full()
                .child(theme::band(px(3.))),
        )
        .into_any_element()
}

/// `.win-btn`: 46×29, silver glyphs; hover inverts to paper, the close button to coral.
/// No click handlers on purpose — the OS acts on the hit-test area.
fn window_buttons() -> impl IntoElement {
    use gpui_kit::WindowControlArea as Area;
    let button = |id: &'static str, icon: Icon, area: Area, close: bool| {
        div()
            .id(id)
            .w(px(46.))
            .h(px(29.))
            .flex()
            .items_center()
            .justify_center()
            .text_color(c::SILVER)
            .window_control_area(area)
            .hover(move |s| {
                if close {
                    s.bg(theme::CLOSE_HOVER).text_color(theme::ON_SIGNAL)
                } else {
                    s.bg(c::PAPER).text_color(theme::ON_INVERT)
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
