//! Slice B: message bodies — fenced code blocks with highlighting, links (opened through
//! `commands::open_link`), `@name` mentions — as styled text runs. Ported from `renderBody`,
//! `inline`, `codeBlock` and `highlight` in `app.js`.
//!
//! Foundation placeholder: the plain text.

use super::AppSnapshot;
use gpui_kit::{div, AnyElement, App, IntoElement, ParentElement, Window};

pub fn render_body(_snap: &AppSnapshot, text: &str, _mentions_you: bool, _window: &mut Window, _cx: &mut App) -> AnyElement {
    div().child(text.to_string()).into_any_element()
}
