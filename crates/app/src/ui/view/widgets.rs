//! Small pieces more than one area draws, in design B (see `theme`).
//!
//! | Piece | Use |
//! |---|---|
//! | [`avatar`] | monogram in the identity hue, hairline ring (green while speaking), presence dot |
//! | [`Btn`] | text buttons: ghost first, bone outline for the one primary action, red for danger |
//! | [`icon_btn`] | 28×28 glyph buttons |
//! | [`dk_btn`] | the call bar's 40×40 framed glyph buttons |
//! | [`live_pill`] | the red LIVE mark on a tile and in the voice list |
//! | [`popover_surface`], [`popover_item`] | the ground of every popover and menu, and its rows |
//! | [`badge`] | unread counts |
//! | [`selectable`] | the selected-row look: lift, hairline frame, white text |
//! | [`pane_head`], [`pane_title`], [`section_label`] | pane heads and the mono group labels |
//! | [`WindowFrame`] | every dialog's chassis: serif title, body, foot |
//! | [`modal_layer`] | the flat dim plus the centred window |
//! | [`meter_view`] | a tiny view that re-renders on a meters tick and nothing else |
//! | [`tooltip`] | hover text |
//!
//! Hover is a flat colour change and snaps; nothing here animates.

use super::super::bridge::Meters;
use super::super::icons::Icon;
use super::super::state::{PeerState, Presence, UiState};
use super::super::theme::{self, c};
use super::format;
use gpui_kit::{
    div, prelude::FluentBuilder, px, relative, AnyElement, App, ClickEvent, Context, Div,
    ElementId, Entity, FocusHandle, FontWeight, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Pixels, Refineable, Render, RenderOnce, Rgba, SharedString, Stateful,
    StatefulInteractiveElement, StyleRefinement, Styled, Window,
};
use std::rc::Rc;

// ------------------------------------------------------------------ avatar

/// What the presence square in an avatar's corner shows. A chosen status wins over the
/// connection state while it exists, as in `avatar()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dot {
    Online,
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

/// A circle with one monogram in the identity hue on a lift, a hairline ring that turns
/// green while speaking, and optionally the presence dot on its corner. `size` is the
/// diameter in px.
pub fn avatar(name: &str, hue: u8, size: f32, dot: Option<Dot>, speaking: bool) -> Div {
    let font = match size as u32 {
        0..=22 => 11.,
        23..=30 => 12.,
        31..=34 => 13.,
        _ => 15.,
    };
    div()
        .relative()
        .flex_none()
        .size(px(size))
        .child(
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(c::LIFT)
                .border_1()
                .border_color(if speaking {
                    theme::ONLINE
                } else {
                    theme::BORDER
                })
                .text_color(theme::avatar_hue(hue))
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(font))
                .line_height(px(size))
                .child(format::monogram(name)),
        )
        .when_some(dot, |d, dot| d.child(presence_dot(dot)))
}

/// A 9px dot with a 2px void ring, 2px outside the bottom-right edge.
fn presence_dot(dot: Dot) -> Div {
    let fill: Option<Rgba> = match dot {
        Dot::Online => Some(theme::ONLINE),
        Dot::Dnd | Dot::Busy => Some(theme::DND),
        Dot::Away => Some(theme::CONNECTING),
        Dot::Offline => Some(theme::OFFLINE),
        Dot::Broken => None,
    };
    div()
        .absolute()
        .right(px(-4.))
        .bottom(px(-4.))
        .size(px(13.))
        .rounded_full()
        .border_2()
        .border_color(c::VOID)
        .when_some(fill, |d, f| d.bg(f))
}

// ------------------------------------------------------------------ buttons

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BtnKind {
    /// Void ground, hairline frame, white text; hover whitens the frame.
    #[default]
    Plain,
    /// The one primary action: a bone outline instead of the hairline.
    Primary,
    /// No ground, hairline frame.
    Ghost,
    /// Red text in a red line; hover brightens the line to red.
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
    /// The dialog-foot size: 32px tall.
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
        let (bg, fg, frame, hover): (Option<Rgba>, Rgba, Rgba, Rgba) = match self.kind {
            BtnKind::Plain => (
                Some(c::VOID),
                theme::TEXT_STRONG,
                theme::BORDER,
                theme::TEXT_STRONG,
            ),
            BtnKind::Primary => (Some(c::VOID), theme::TEXT_STRONG, c::BONE, c::WHITE),
            BtnKind::Ghost => (None, theme::TEXT_STRONG, theme::BORDER, theme::TEXT_STRONG),
            BtnKind::Danger => (
                Some(c::VOID),
                theme::DANGER_TEXT,
                theme::DANGER_LINE,
                theme::DANGER,
            ),
        };
        let enabled = !self.disabled;
        div()
            .id(self.id)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .gap(px(6.))
            .h(px(if self.dialog { 32. } else { 30. }))
            .px(px(if self.dialog { 14. } else { 12. }))
            .when(self.full_width, |d| d.w_full())
            .when_some(bg, |d, bg| d.bg(bg))
            .rounded(theme::RADIUS_CONTROL)
            .border_1()
            .border_color(frame)
            .text_color(fg)
            .text_size(px(13.))
            .font_weight(FontWeight::MEDIUM)
            .whitespace_nowrap()
            .when(self.disabled, |d| d.opacity(theme::DISABLED_OPACITY))
            .when(enabled, |d| {
                d.cursor_pointer().hover(move |s| s.border_color(hover))
            })
            .when_some(self.icon, |d, icon| d.child(icon.size(14.)))
            .child(self.label)
            .when_some(self.on_click.filter(|_| enabled), |d, f| {
                d.on_click(move |ev, window, cx| f(ev, window, cx))
            })
    }
}

/// A 28×28 glyph button: muted glyph, hover lifts it; `pressed` is the selected look.
pub fn icon_btn(id: impl Into<ElementId>, icon: Icon, pressed: bool) -> Stateful<Div> {
    selectable(
        div()
            .id(id)
            .size(px(28.))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .text_color(theme::TEXT_MUTED)
            .hover(|s| s.bg(theme::BG_HOVER).text_color(theme::TEXT_STRONG)),
        pressed,
    )
    .child(icon.size(16.))
}

/// A 40×40 call-bar button: hairline frame, bone glyph, hover an iron frame and a white
/// glyph; `pressed` is the selected look.
pub fn dk_btn(id: impl Into<ElementId>, icon: Icon, pressed: bool) -> Stateful<Div> {
    div()
        .id(id)
        .tab_index(0)
        .focus_visible(|s| s.shadow(focus_ring()))
        .size(px(40.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(theme::RADIUS_CONTROL)
        .border_1()
        .border_color(theme::BORDER)
        .bg(c::VOID)
        .cursor_pointer()
        .text_color(theme::TEXT)
        .when(pressed, |d| {
            d.bg(theme::BG_SELECTED).text_color(theme::TEXT_STRONG)
        })
        .hover(|s| {
            s.border_color(theme::BORDER_HOVER)
                .text_color(theme::TEXT_STRONG)
        })
        .child(icon.size(17.))
}

/// The LIVE mark: red mono on void in a red line.
pub fn live_pill() -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .px(px(6.))
        .bg(c::VOID)
        .rounded(px(4.))
        .border_1()
        .border_color(theme::DANGER_LINE)
        .font_family(theme::MONO_FONT)
        .text_size(px(10.5))
        .text_color(theme::DANGER_TEXT)
        .child("LIVE")
}

/// A popover's ground: card corners, the hairline, the one soft shadow.
pub fn popover_surface() -> Div {
    div()
        .rounded(theme::RADIUS_CARD)
        .border_1()
        .border_color(theme::BORDER)
        .bg(theme::BG_RAISED)
        .shadow(theme::soft_shadow())
}

/// A popover row; `selected` is the `aria-selected` row in the selected look.
pub fn popover_item(id: impl Into<ElementId>, selected: bool) -> Stateful<Div> {
    selectable(
        div()
            .id(id)
            .flex()
            .items_center()
            .gap(px(9.))
            .w_full()
            .px(px(10.))
            .py(px(6.))
            .cursor_pointer()
            .text_size(px(14.))
            .text_color(theme::TEXT_MUTED)
            .hover(|s| s.bg(theme::BG_HOVER).text_color(theme::TEXT_STRONG)),
        selected,
    )
}

/// An unread count: a chrome pill with void mono figures; `quiet` is a lift with ash.
pub fn badge(count: usize, quiet: bool) -> Div {
    div()
        .min_w(px(18.))
        .h(px(18.))
        .px(px(5.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(if quiet { c::LIFT } else { theme::ACCENT })
        .text_color(if quiet {
            theme::TEXT_MUTED
        } else {
            theme::ON_ACCENT
        })
        .font_family(theme::MONO_FONT)
        .font_weight(FontWeight::MEDIUM)
        .text_size(px(11.))
        .line_height(px(18.))
        .child(count.to_string())
}

/// The row shape every list shares (6px corners, a 1px frame that is transparent until
/// selected, so selecting never shifts the label) and the selected look: a lift, the
/// hairline frame and white text.
pub fn selectable<E: Styled + FluentBuilder>(e: E, selected: bool) -> E {
    e.rounded(theme::RADIUS_CONTROL)
        .border_1()
        .border_color(gpui_kit::transparent_black())
        .when(selected, |e| {
            e.bg(theme::BG_SELECTED)
                .border_color(theme::SELECTED_EDGE)
                .text_color(theme::TEXT_STRONG)
        })
}

/// `:focus-visible { outline: 2px solid accent; outline-offset: 2px }`. GPUI has no outline,
/// so the ring is a 4px accent shadow with a 2px void shadow over it (the gap). Use as
/// `.tab_index(0).focus_visible(|s| s.shadow(focus_ring()))`.
pub fn focus_ring() -> Vec<gpui_kit::BoxShadow> {
    let ring = |colour: Rgba, spread: f32| gpui_kit::BoxShadow {
        color: colour.into(),
        offset: gpui_kit::point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius: px(spread),
        inset: false,
    };
    vec![ring(theme::ACCENT, 4.), ring(c::VOID, 2.)]
}

/// `letter-spacing: <em>em` for the short uppercase mono labels, which GPUI's text cannot
/// space: one cell per character, its advance plus the spacing wide. Every glyph of
/// JetBrains Mono advances 0.6 em, so the cell width is exact. Sets `MONO_FONT` itself.
// One element per character with the advance hardcoded; replace with a letter-spacing run
// style once GPUI's text layout grows one.
pub fn tracked(text: &str, size: f32, em: f32) -> Div {
    div()
        .flex()
        .flex_none()
        .font_family(theme::MONO_FONT)
        .children(text.chars().map(|ch| {
            div()
                .flex_none()
                .w(px(size * (0.6 + em)))
                .child(ch.to_string())
        }))
}

/// The mono uppercase group label: `IN THE CALL`, `DEVICES`.
pub fn section_label(text: &str) -> Div {
    div()
        .text_size(theme::LABEL_TEXT)
        .text_color(theme::TEXT_MUTED)
        .child(tracked(&text.to_uppercase(), 11.5, 0.04))
}

/// A pane's head: 60px of void that holds a [`pane_title`] and its buttons.
pub fn pane_head() -> Div {
    div().h(theme::HEAD_H).flex_none().flex().items_center()
}

/// The serif title of a pane, a dialog or a settings page.
pub fn pane_title(text: impl Into<SharedString>, size: Pixels) -> Div {
    div()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .font_family(theme::DISPLAY_FONT)
        .text_size(size)
        .line_height(relative(1.2))
        .text_color(theme::TEXT_STRONG)
        .child(text.into())
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

/// A dialog: void ground, hairline frame, card corners, the soft shadow, a serif title.
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
            .bg(c::VOID)
            .border_1()
            .border_color(theme::BORDER)
            .rounded(theme::RADIUS_CARD)
            .text_color(theme::TEXT)
            .shadow(theme::soft_shadow())
            .child(
                div()
                    .px(px(22.))
                    .pt(px(18.))
                    .font_family(theme::DISPLAY_FONT)
                    .text_size(theme::DISPLAY_TEXT)
                    .line_height(relative(1.2))
                    .text_color(theme::TEXT_STRONG)
                    .when(self.truncate_title, |d| d.truncate())
                    .child(self.title),
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

/// The modal layer: the window dimmed by a flat [`theme::BACKDROP`], `content` centred
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
        .bg(theme::BACKDROP)
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
