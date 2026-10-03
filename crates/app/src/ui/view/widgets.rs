//! Small pieces more than one area draws, in the PC-98 grammar of `app.css`.
//!
//! | Piece | CSS | Use |
//! |---|---|---|
//! | [`avatar`] | `.avatar`, `.dot`, `.speak-ring` | every roster row, message gutter, DM row |
//! | [`Btn`] | `.btn`, `--accent`, `--ghost`, `--danger`, dialog foot | text buttons |
//! | [`icon_btn`] | `.icon-btn` | 28×28 square glyph buttons |
//! | [`badge`] | `.badge`, `.badge--quiet` | unread counts |
//! | [`pointer`] | the ▶ on menu-row hover | channel rows, settings tabs, picker rows |
//! | [`WindowFrame`] | `dialog.confirm` | every dialog's chassis: paper frame, blue strip, foot |
//! | [`modal_layer`] | `::backdrop` | the dithered dim plus the centred window |
//! | [`meter_view`] | - | a tiny view that re-renders on a meters tick and nothing else |
//! | [`tooltip`] | `title=""` | hover text |
//!
//! Hover is a flat colour change and snaps; nothing here animates.

use super::super::bridge::Meters;
use super::super::icons::Icon;
use super::super::state::{PeerState, Presence, UiState};
use super::super::theme::{self, c, Dither};
use super::format;
use gpui_kit::{
    div, prelude::FluentBuilder, px, AnyElement, App, ClickEvent, Context, Div, ElementId, Entity,
    FocusHandle, InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels, Refineable,
    Render, RenderOnce, Rgba, SharedString, Stateful, StatefulInteractiveElement, StyleRefinement,
    Styled, Window,
};
use std::rc::Rc;

// ------------------------------------------------------------------ avatar

/// What the presence square in an avatar's corner shows. A chosen status wins over the
/// connection state while it exists, as in `avatar()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dot {
    Online,
    Connecting,
    /// Our own do-not-disturb.
    Dnd,
    Away,
    Busy,
    /// An unlit square, not a warning.
    Offline,
    /// Protocol or identity trouble; the stylesheet had no fill for it, only the frame.
    Broken,
}

impl Dot {
    pub fn of_peer(p: &PeerState) -> Dot {
        match p.user_status.as_deref() {
            Some("away") => return Dot::Away,
            Some("busy") => return Dot::Busy,
            _ => {}
        }
        match p.presence {
            Presence::Online => Dot::Online,
            Presence::Connecting => Dot::Connecting,
            Presence::Offline => Dot::Offline,
            Presence::Broken => Dot::Broken,
        }
    }

    /// Ourselves: never offline; do-not-disturb is a state, not a fault.
    pub fn of_self(s: &UiState) -> Dot {
        match s.me.status.as_str() {
            "away" => Dot::Away,
            "busy" => Dot::Busy,
            _ if s.do_not_disturb => Dot::Dnd,
            _ => Dot::Online,
        }
    }
}

/// A sprite: a filled square in the identity hue with one monogram in space ink, optionally
/// with the presence square notched into its corner and the speaking frame around it.
/// `size` is one of 20, 24, 32, 40 (the font size follows the stylesheet).
pub fn avatar(name: &str, hue: u8, size: f32, dot: Option<Dot>, speaking: bool) -> Div {
    let font = match size as u32 {
        0..=20 => 10.,
        21..=24 => 11.,
        25..=32 => 13.,
        _ => 16.,
    };
    let square = div()
        .size(px(size))
        .flex()
        .items_center()
        .justify_center()
        .bg(theme::avatar_hue(hue))
        .text_color(c::SPACE)
        .text_size(px(font))
        .line_height(px(size))
        .child(format::monogram(name));

    div()
        .relative()
        .flex_none()
        .size(px(size))
        .child(square)
        // `.speak-ring::after`: inset -3px, a 2px ring outside that. It snaps on and off.
        .when(speaking, |d| {
            d.child(
                div()
                    .absolute()
                    .top(px(-5.))
                    .left(px(-5.))
                    .size(px(size + 10.))
                    .border_2()
                    .border_color(theme::ONLINE),
            )
        })
        .when_some(dot, |d, dot| d.child(presence_dot(dot)))
}

/// `.dot`: 9×9 with a 2px ink frame, sitting 2px outside the bottom-right corner.
fn presence_dot(dot: Dot) -> Div {
    let fill: Option<Rgba> = match dot {
        Dot::Online => Some(theme::ONLINE),
        Dot::Connecting => Some(theme::CONNECTING),
        Dot::Dnd | Dot::Busy => Some(theme::DND),
        Dot::Away => Some(c::AMBER),
        Dot::Offline => Some(c::INK),
        Dot::Broken => None,
    };
    div()
        .absolute()
        .right(px(-2.))
        .bottom(px(-2.))
        .size(px(13.))
        .border_2()
        .border_color(c::INK)
        .when_some(fill, |d, f| d.bg(f))
        .when(dot == Dot::Offline, |d| {
            d.child(div().size_full().border_2().border_color(theme::OFFLINE))
        })
}

// ------------------------------------------------------------------ buttons

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BtnKind {
    /// `.btn`: space ground, steel frame; hover inverts to paper.
    #[default]
    Plain,
    /// `.btn--accent`: magenta in a paper frame; hover flips a quarter of the cells.
    Accent,
    /// `.btn--ghost`: no ground.
    Ghost,
    /// `.btn--danger`: coral text and frame; hover fills coral.
    Danger,
}

type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// A text button in the stylesheet's grammar.
///
/// ```ignore
/// Btn::new("confirm-yes", "Delete").kind(BtnKind::Danger).dialog().on_click(handler)
/// ```
#[derive(IntoElement)]
pub struct Btn {
    id: ElementId,
    label: SharedString,
    icon: Option<Icon>,
    kind: BtnKind,
    dialog: bool,
    disabled: bool,
    full_width: bool,
    on_click: Option<ClickHandler>,
}

impl Btn {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Btn {
            id: id.into(),
            label: label.into(),
            icon: None,
            kind: BtnKind::Plain,
            dialog: false,
            disabled: false,
            full_width: false,
            on_click: None,
        }
    }
    pub fn kind(mut self, kind: BtnKind) -> Self {
        self.kind = kind;
        self
    }
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }
    /// The dialog-foot chassis: 32px tall, 2px frame, dark outer line plus a 2px drop.
    pub fn dialog(mut self) -> Self {
        self.dialog = true;
        self
    }
    /// `:disabled`: 45% opacity and no hover, no click.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
    pub fn full_width(mut self) -> Self {
        self.full_width = true;
        self
    }
    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for Btn {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let (bg, fg, frame): (Option<Rgba>, Rgba, Rgba) = match self.kind {
            BtnKind::Plain => (Some(c::SPACE), theme::TEXT, theme::BORDER_STRONG),
            BtnKind::Accent => (Some(theme::PRIMARY), theme::ON_PRIMARY, c::PAPER),
            BtnKind::Ghost => (None, theme::TEXT, theme::BORDER_STRONG),
            BtnKind::Danger => (Some(c::SPACE), theme::DANGER_TEXT, theme::DANGER_LINE),
        };
        // GPUI paints a box shadow under the element too, where CSS clips it to the outside:
        // a ghost dialog button would read as void-filled. Dialogs are ink, so fill with ink.
        let bg = bg.or(self.dialog.then_some(c::INK));
        let group: SharedString = format!("btn-{:?}", self.id).into();
        let enabled = !self.disabled;
        let kind = self.kind;
        div()
            .id(self.id)
            .group(group.clone())
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .gap(px(6.))
            .h(px(if self.dialog { 32. } else { 28. }))
            .px(px(if self.dialog { 14. } else { 11. }))
            .when(self.full_width, |d| d.w_full())
            .when_some(bg, |d, bg| d.bg(bg))
            .border(px(if self.dialog { 2. } else { 1. }))
            .border_color(frame)
            .text_color(fg)
            .text_size(px(12.5))
            .when(self.dialog, |d| d.shadow(theme::chassis_shadow(2.)))
            .when(self.disabled, |d| d.opacity(0.45))
            .when(enabled, |d| {
                d.cursor_pointer().hover(move |s| match kind {
                    BtnKind::Plain | BtnKind::Ghost => s
                        .bg(c::PAPER)
                        .border_color(c::PAPER)
                        .text_color(theme::ON_INVERT),
                    BtnKind::Danger => s
                        .bg(c::CORAL)
                        .border_color(c::CORAL)
                        .text_color(theme::ON_SIGNAL),
                    BtnKind::Accent => s,
                })
            })
            // The accent hover is a dither, not a colour: a quarter of the cells to paper.
            .when(enabled && kind == BtnKind::Accent, |d| {
                d.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .opacity(0.)
                        .group_hover(group.clone(), |s| s.opacity(1.))
                        .child(theme::dither(Dither::Quarter, c::PAPER, 2.)),
                )
            })
            .when_some(self.icon, |d, icon| d.child(icon.size(14.)))
            .child(div().relative().child(self.label))
            .when_some(self.on_click.filter(|_| enabled), |d, f| {
                d.on_click(move |ev, window, cx| f(ev, window, cx))
            })
    }
}

/// `.icon-btn`: a 28×28 square glyph button; hover and pressed invert to paper.
pub fn icon_btn(id: impl Into<ElementId>, icon: Icon, pressed: bool) -> Stateful<Div> {
    div()
        .id(id)
        .size(px(28.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .when(pressed, |d| d.bg(c::PAPER).text_color(theme::ON_INVERT))
        .hover(|s| s.bg(c::PAPER).text_color(theme::ON_INVERT))
        .child(icon.size(16.))
}

/// `.badge`: an inverted cell with the count; `quiet` is the violet variant.
pub fn badge(count: usize, quiet: bool) -> Div {
    div()
        .min_w(px(18.))
        .h(px(18.))
        .px(px(5.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .bg(if quiet { c::VIOLET } else { c::PAPER })
        .text_color(if quiet { c::SILVER } else { c::SPACE })
        .text_size(px(11.))
        .line_height(px(18.))
        .child(count.to_string())
}

/// `:focus-visible { outline: 2px solid accent; outline-offset: 2px }`. GPUI has no outline,
/// so the ring is a 4px accent shadow with a 2px `ground` shadow over it (the gap shows the
/// ground the element sits on), around the element's own `base` shadows. Use as
/// `.tab_index(0).focus_visible(|s| s.shadow(focus_ring(c::INK, vec![])))`.
pub fn focus_ring(ground: Rgba, base: Vec<gpui_kit::BoxShadow>) -> Vec<gpui_kit::BoxShadow> {
    let ring = |colour: Rgba, spread: f32| gpui_kit::BoxShadow {
        color: colour.into(),
        offset: gpui_kit::point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius: px(spread),
        inset: false,
    };
    // Later shadows paint over earlier ones; a frame line of the chassis (no offset) goes
    // back on top so the ring only replaces what the outline covered.
    let lines: Vec<gpui_kit::BoxShadow> = base
        .iter()
        .filter(|s| s.offset.x == px(0.))
        .cloned()
        .collect();
    let mut all = base;
    all.extend([ring(theme::ACCENT, 4.), ring(ground, 2.)]);
    all.extend(lines);
    all
}

/// `letter-spacing: <em>em` for the short uppercase labels, which GPUI's text cannot space:
/// one cell per character, its advance plus the spacing wide. Every ASCII glyph of the UI
/// face advances half an em, so the cell width is exact (a measured single glyph would be
/// rounded up a pixel each). ASCII labels in `UI_FONT` at `size` px only.
// ponytail: one element per character and the half-em advance hardcoded; replace with a
// letter-spacing run style once GPUI's text layout grows one.
pub fn tracked(text: &str, size: f32, em: f32) -> Div {
    div().flex().flex_none().children(text.chars().map(|ch| {
        div()
            .flex_none()
            .w(px(size * (0.5 + em)))
            .child(ch.to_string())
    }))
}

const POINTER_SVG: &[u8] =
    br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 5 8"><path d="M0 0 5 4 0 8z" fill="#000"/></svg>"##;

/// The ▶ of a hovered menu row: a 5×8 cyan block at the row's left edge. Put it in a
/// `relative()` row and show it on hover with a group (`.opacity(0.).group_hover(..)`).
pub fn pointer() -> Div {
    div()
        .absolute()
        .left(px(1.))
        .top_0()
        .bottom_0()
        .flex()
        .items_center()
        .text_color(c::CYAN)
        .child(
            gpui_kit::component::Icon::default()
                .data(POINTER_SVG)
                .w(px(5.))
                .h(px(8.)),
        )
}

/// Hover text, the way `title=""` worked: `.tooltip(tooltip("Close to tray"))` on an
/// element with an id.
pub fn tooltip(
    text: impl Into<SharedString>,
) -> impl Fn(&mut Window, &mut App) -> gpui_kit::AnyView {
    let text: SharedString = text.into();
    move |window, cx| gpui_kit::component::tooltip::Tooltip::new(text.clone()).build(window, cx)
}

// ------------------------------------------------------------------ dialog chassis

/// A system window: paper frame, blue title strip, hard shadow in the void.
///
/// ```ignore
/// WindowFrame::new("Delete this?").width(420.)
///     .child(div().px(px(22.)).pt(px(16.)).child(text))
///     .foot(vec![cancel.into_any_element(), ok.into_any_element()])
/// ```
#[derive(IntoElement)]
pub struct WindowFrame {
    title: SharedString,
    width: Option<Pixels>,
    truncate_title: bool,
    children: Vec<AnyElement>,
    foot: Vec<AnyElement>,
    style: StyleRefinement,
}

impl WindowFrame {
    pub fn new(title: impl Into<SharedString>) -> Self {
        WindowFrame {
            title: title.into(),
            width: Some(px(420.)),
            truncate_title: false,
            children: Vec::new(),
            foot: Vec::new(),
            style: StyleRefinement::default(),
        }
    }
    /// One line with an ellipsis instead of wrapping (the lightbox's file name).
    pub fn truncate_title(mut self) -> Self {
        self.truncate_title = true;
        self
    }
    /// `None` sizes the window by its content (the lightbox).
    pub fn width(mut self, width: Option<f32>) -> Self {
        self.width = width.map(px);
        self
    }
    /// The buttons, right-aligned in `.confirm-foot`.
    pub fn foot(mut self, buttons: Vec<AnyElement>) -> Self {
        self.foot = buttons;
        self
    }
}

/// Refines the window's root: a max height for the Wordle window, a fixed width.
impl Styled for WindowFrame {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl ParentElement for WindowFrame {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for WindowFrame {
    fn render(self, window: &mut Window, _: &mut App) -> impl IntoElement {
        // `min(420px, 100vw - 48px)`.
        let max = window.viewport_size().width - px(48.);
        let mut root = div()
            .flex()
            .flex_col()
            .when_some(self.width, |d, w| d.w(w.min(max)))
            .bg(c::INK)
            .border_2()
            .border_color(c::PAPER)
            .text_color(theme::TEXT)
            .shadow(theme::shadow_modal())
            .child(
                div()
                    .px(px(14.))
                    .py(px(8.))
                    .bg(c::BLUE)
                    .border_b_1()
                    .border_color(c::SPACE)
                    .text_color(c::PAPER)
                    .text_size(px(14.))
                    .when(self.truncate_title, |d| d.truncate())
                    .child(self.title.to_uppercase()),
            )
            .children(self.children)
            .when(!self.foot.is_empty(), |d| {
                d.child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(8.))
                        .px(px(22.))
                        .pt(px(18.))
                        .pb(px(20.))
                        .children(self.foot),
                )
            });
        root.style().refine(&self.style);
        root
    }
}

/// The modal layer: the window dimmed by an ordered dither of the void, `content` centred
/// on it, and every click and scroll stopped from reaching what is underneath. Tracks
/// `focus` so keys typed while it is open go to it (attach `on_key_down` to `content`).
/// `on_backdrop` runs for a click beside the window (the lightbox closes that way; the
/// confirm dialogs do not).
pub type BackdropHandler = Box<dyn Fn(&mut Window, &mut App)>;

pub fn modal_layer(
    content: impl IntoElement,
    focus: &FocusHandle,
    on_backdrop: Option<BackdropHandler>,
) -> Stateful<Div> {
    let on_backdrop = on_backdrop.map(Rc::new);
    div()
        .id("modal-layer")
        .track_focus(focus)
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .occlude()
        .child(theme::dither(Dither::Half, c::SPACE, 2.))
        .child(
            div()
                .id("modal-backdrop")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .when_some(on_backdrop, |d, f| {
                    d.on_mouse_down(MouseButton::Left, move |_, window, cx| f(window, cx))
                })
                .child(
                    div()
                        .id("modal-content")
                        // Clicks inside the window are not clicks on the backdrop.
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(content),
                ),
        )
}

// ------------------------------------------------------------------ meters

type MeterRender = Rc<dyn Fn(&Meters, &mut Window, &mut App) -> AnyElement>;

/// A view of its own that observes `Meters`, so a 4 Hz tick re-renders only this bit (and
/// the cached pane around it), never the timeline. Create it in render with a stable id;
/// the closure is refreshed on every parent render, so it may capture current data.
///
/// ```ignore
/// meter_view(("rtt", peer.id.clone()), &snap.meters, window, cx, move |m, _, _| {
///     div().child(format::fmt_rtt(m.rtt(&id))).into_any_element()
/// })
/// ```
pub fn meter_view(
    id: impl Into<ElementId>,
    meters: &Entity<Meters>,
    window: &mut Window,
    cx: &mut App,
    render: impl Fn(&Meters, &mut Window, &mut App) -> AnyElement + 'static,
) -> Entity<MeterView> {
    let meters = meters.clone();
    let view = window.use_keyed_state(id, cx, |_, cx| MeterView::new(meters, cx));
    let render: MeterRender = Rc::new(render);
    view.update(cx, |v, _| v.render = Some(render));
    view
}

pub struct MeterView {
    meters: Entity<Meters>,
    render: Option<MeterRender>,
    _observe: gpui_kit::Subscription,
}

impl MeterView {
    fn new(meters: Entity<Meters>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&meters, |_, _, cx| cx.notify());
        MeterView {
            meters,
            render: None,
            _observe: observe,
        }
    }
}

impl Render for MeterView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(render) = self.render.clone() else {
            return div().into_any_element();
        };
        // `render` needs `&mut App` while a read borrows it; the readings are a handful of
        // floats per peer, so a copy is cheaper than any way around that.
        let copy = self.meters.read(cx).clone();
        render(&copy, window, cx)
    }
}

// ------------------------------------------------------------------ placeholders

/// Foundation stand-in for an area a slice has not drawn yet: the zone's title strip in its
/// tint, and a label. Remove from a file once its real render lands.
pub fn placeholder(title: &str, tint: Option<Rgba>, label: &str) -> Div {
    div()
        .size_full()
        .flex()
        .flex_col()
        .when_some(tint, |d, tint| {
            d.child(
                div()
                    .h(theme::HEAD_H)
                    .flex_none()
                    .flex()
                    .items_center()
                    .px(px(14.))
                    .bg(tint)
                    .border_b_1()
                    .border_color(c::SPACE)
                    .text_color(if tint == c::BLUE { c::PAPER } else { c::SPACE })
                    .text_size(px(13.))
                    .child(title.to_uppercase()),
            )
        })
        .child(
            div()
                .flex_1()
                .p(px(14.))
                .text_color(c::SILVER)
                .text_size(px(12.))
                .child(label.to_string()),
        )
}
