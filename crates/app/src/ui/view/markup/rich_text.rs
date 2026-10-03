//! A run of styled text that takes part in the window-wide text selection, with
//! clickable links and the mention plates. gpui-kit's own `SelectableText` takes plain text
//! only; this is its element with `StyledText::with_highlights` inside, plus what the
//! webview's `<a>` and `.mention` did.
//!
//! The timeline is a cached pane, so a selection change must notify the view that painted
//! the run (the `TextSelectionHandle::subscribe` below), or a replayed frame would keep
//! stale selection quads. Same reason for the link hover: the underline turns solid under
//! the pointer, which needs a repaint of that view.

use super::super::super::commands;
use super::super::super::theme::{self, c};
use gpui_kit::base::{TextSelection, TextSelectionEvent, TextSelectionHandle, TextSelectionRegistration, TextSelectionRun};
use gpui_kit::{
    point, px, size, App, BorderStyle, Bounds, Corners, CursorStyle, Edges, Element, ElementId, EntityId,
    GlobalElementId, HighlightStyle, Hitbox, HitboxBehavior, Hsla, InspectorElementId, IntoElement, LayoutId,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, SharedString,
    StyledText, Subscription, TextLayout, Window,
};
use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::rc::Rc;


pub struct RichText {
    id: ElementId,
    text: SharedString,
    styled: StyledText,
    links: Rc<Vec<(Range<usize>, SharedString)>>,
    plates: Vec<Range<usize>>,
    plate_color: Hsla,
    /// Byte offsets of characters that are layout only (the mention padding) and are left
    /// out of what Copy puts on the clipboard.
    no_copy: Rc<Vec<usize>>,
}

impl RichText {
    pub fn new(id: impl Into<ElementId>, text: impl Into<SharedString>) -> Self {
        let text = text.into();
        RichText {
            id: id.into(),
            styled: StyledText::new(text.clone()),
            text,
            links: Rc::new(Vec::new()),
            plates: Vec::new(),
            plate_color: c::SPACE.into(),
            no_copy: Rc::new(Vec::new()),
        }
    }

    /// Colours per byte range; sorted, disjoint, on char boundaries.
    pub fn highlights(mut self, highlights: Vec<(Range<usize>, HighlightStyle)>) -> Self {
        self.styled = StyledText::new(self.text.clone()).with_highlights(highlights);
        self
    }

    /// Ranges that open their URL on a click (not on the end of a drag-select).
    pub fn links(mut self, links: Vec<(Range<usize>, SharedString)>) -> Self {
        self.links = Rc::new(links);
        self
    }

    /// Ranges drawn on a filled plate, like `.mention`.
    pub fn plates(mut self, plates: Vec<Range<usize>>, color: impl Into<Hsla>) -> Self {
        self.plates = plates;
        self.plate_color = color.into();
        self
    }

    /// Characters (by byte offset) that are drawn but never copied.
    pub fn no_copy(mut self, offsets: Vec<usize>) -> Self {
        self.no_copy = Rc::new(offsets);
        self
    }
}

/// What survives between frames: the selection participant, the subscription that
/// repaints our view when the selection moves, and the link a press started on.
struct Retained {
    handle: TextSelectionHandle,
    _repaint: Subscription,
    pressed: Rc<Cell<Option<usize>>>,
    copy: Rc<RefCell<String>>,
}

pub struct Layout {
    handle: TextSelectionHandle,
    pressed: Rc<Cell<Option<usize>>>,
    copy: Rc<RefCell<String>>,
}

pub struct Prepaint {
    hitbox: Hitbox,
    links: Rc<Vec<(usize, Hitbox, Bounds<Pixels>)>>,
    plates: Vec<Bounds<Pixels>>,
    view: EntityId,
}

impl IntoElement for RichText {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

/// The boxes a byte range covers, one per visual line.
fn line_rects(layout: &TextLayout, text: &str, range: Range<usize>) -> Vec<Bounds<Pixels>> {
    let line_height = layout.line_height();
    let mut rects: Vec<Bounds<Pixels>> = Vec::new();
    let mut last_width = px(0.);
    let mut indices: Vec<usize> = text[range.clone()].char_indices().map(|(i, _)| range.start + i).collect();
    indices.push(range.end);
    for pair in indices.windows(2) {
        let (Some(a), Some(b)) = (layout.position_for_index(pair[0]), layout.position_for_index(pair[1])) else {
            continue;
        };
        // A char that ends a wrapped line has no right neighbour on its line: give it the
        // width of the one before.
        let right = if b.y == a.y {
            last_width = b.x - a.x;
            b.x
        } else {
            a.x + last_width
        };
        match rects.last_mut() {
            Some(r) if r.origin.y == a.y => r.size.width = right - r.origin.x,
            _ => rects.push(Bounds::new(a, size(right - a.x, line_height))),
        }
    }
    rects
}

fn quad(bounds: Bounds<Pixels>, color: Hsla) -> PaintQuad {
    PaintQuad {
        bounds,
        corner_radii: Corners::default(),
        background: color.into(),
        border_widths: Edges::default(),
        border_color: gpui_kit::transparent_black(),
        border_style: BorderStyle::default(),
    }
}

/// The selection quads of one range, the way `SelectableText` lays them out.
fn selection_quads(layout: &TextLayout, range: Range<usize>, bounds: Bounds<Pixels>) -> Vec<Bounds<Pixels>> {
    let (Some(start), Some(end)) = (layout.position_for_index(range.start), layout.position_for_index(range.end)) else {
        return Vec::new();
    };
    let lh = layout.line_height();
    if start.y == end.y {
        return vec![Bounds::from_corners(start, point(end.x, end.y + lh))];
    }
    let mut quads = vec![Bounds::from_corners(start, point(bounds.right(), start.y + lh))];
    if end.y > start.y + lh {
        quads.push(Bounds::from_corners(point(bounds.left(), start.y + lh), point(bounds.right(), end.y)));
    }
    quads.push(Bounds::from_corners(point(bounds.left(), end.y), point(end.x, end.y + lh)));
    quads
}

impl Element for RichText {
    type RequestLayoutState = Layout;
    type PrepaintState = Prepaint;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Layout) {
        let view = window.current_view();
        let text = self.text.clone();
        let layout_state = window.with_element_state(
            global_id.expect("RichText has an id"),
            |retained: Option<Retained>, _| {
                let retained = retained.unwrap_or_else(|| {
                    let handle = TextSelectionHandle::new(text.to_string(), cx);
                    let repaint = handle.subscribe(
                        move |event, cx| {
                            if matches!(event, TextSelectionEvent::SelectionChanged(_)) {
                                cx.notify(view);
                            }
                        },
                        cx,
                    );
                    // What this run contributes to a copy, worked out at paint from the
                    // selected ranges minus the layout-only characters.
                    let copy = Rc::new(RefCell::new(String::new()));
                    let source = copy.clone();
                    handle.copy_with(move |_| source.borrow().clone(), cx);
                    Retained {
                        handle,
                        _repaint: repaint,
                        pressed: Rc::new(Cell::new(None)),
                        copy,
                    }
                });
                retained.handle.set_fallback_copy_text(text.to_string(), cx);
                let state = Layout {
                    handle: retained.handle.clone(),
                    pressed: retained.pressed.clone(),
                    copy: retained.copy.clone(),
                };
                (state, retained)
            },
        );
        let (layout_id, ()) = self.styled.request_layout(global_id, inspector_id, window, cx);
        (layout_id, layout_state)
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Layout,
        window: &mut Window,
        cx: &mut App,
    ) -> Prepaint {
        self.styled.prepaint(global_id, inspector_id, bounds, &mut (), window, cx);
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let layout = self.styled.layout().clone();
        let links = self
            .links
            .iter()
            .enumerate()
            .flat_map(|(ix, (range, _))| {
                line_rects(&layout, &self.text, range.clone())
                    .into_iter()
                    .map(move |r| (ix, r))
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|(ix, r)| (ix, window.insert_hitbox(r, HitboxBehavior::Normal), r))
            .collect();
        let plates = self
            .plates
            .iter()
            .flat_map(|r| line_rects(&layout, &self.text, r.clone()))
            .collect();
        let registration = TextSelectionRegistration::new(hitbox.clone(), bounds)
            .with_text_bounds(vec![bounds])
            .with_rendered_element(&state.handle, window, cx);
        state.handle.register(registration, window, cx);
        Prepaint {
            hitbox,
            links: Rc::new(links),
            plates,
            view: window.current_view(),
        }
    }

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Layout,
        prepaint: &mut Prepaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        let layout = self.styled.layout().clone();
        let before = TextSelection::selected_text(window, cx);
        let projection = state
            .handle
            .update_runs(&[TextSelectionRun::new(self.text.clone(), layout.clone(), bounds)], cx);
        if before != TextSelection::selected_text(window, cx) {
            window.refresh();
        }
        let copied: String = projection
            .ranges()
            .iter()
            .flatten()
            .flat_map(|r| self.text[r.clone()].char_indices().map(move |(i, ch)| (r.start + i, ch)))
            .filter(|(i, _)| !self.no_copy.contains(i))
            .map(|(_, ch)| ch)
            .collect();
        *state.copy.borrow_mut() = copied;

        for plate in &prepaint.plates {
            window.paint_quad(quad(*plate, self.plate_color));
        }
        // `::selection` inverted the cells; a translucent paper wash is the closest a quad
        // under unchanged glyph colours gets.
        let wash = Hsla::from(c::PAPER).opacity(0.35);
        for range in projection.ranges().iter().flatten().cloned() {
            for b in selection_quads(&layout, range, bounds) {
                window.paint_quad(quad(b, wash));
            }
        }
        self.styled.paint(global_id, inspector_id, bounds, &mut (), &mut (), window, cx);

        window.set_cursor_style(CursorStyle::IBeam, &prepaint.hitbox);
        if prepaint.links.is_empty() {
            return;
        }

        // `border-bottom: 1px dotted` under a link, solid while hovered. It sits at the
        // bottom of the glyphs' box, not on the baseline.
        let font_size = window.text_style().font_size.to_pixels(window.rem_size());
        let line_height = layout.line_height();
        let drop = (line_height + font_size) / 2. + px(0.5);
        let accent: Hsla = theme::ACCENT.into();
        let mut painted = Vec::with_capacity(prepaint.links.len());
        for (_, hitbox, rect) in prepaint.links.iter() {
            window.set_cursor_style(CursorStyle::PointingHand, hitbox);
            let hovered = hitbox.is_hovered(window);
            painted.push(hovered);
            let y = (rect.origin.y + drop).floor();
            if hovered {
                window.paint_quad(quad(Bounds::new(point(rect.origin.x, y), size(rect.size.width, px(1.))), accent));
            } else {
                let mut x = rect.origin.x;
                while x < rect.right() {
                    window.paint_quad(quad(Bounds::new(point(x, y), size(px(1.), px(1.))), accent));
                    x += px(2.);
                }
            }
        }

        let view = prepaint.view;
        let boxes = prepaint.links.clone();
        window.on_mouse_event(move |_: &MouseMoveEvent, phase, window, cx| {
            if phase.bubble() && boxes.iter().zip(painted.iter()).any(|((_, hb, _), was)| hb.is_hovered(window) != *was) {
                cx.notify(view);
            }
        });
        let boxes = prepaint.links.clone();
        let pressed = state.pressed.clone();
        window.on_mouse_event(move |e: &MouseDownEvent, phase, window, _| {
            if phase.bubble() && e.button == MouseButton::Left {
                pressed.set(boxes.iter().find(|(_, hb, _)| hb.is_hovered(window)).map(|(ix, _, _)| *ix));
            }
        });
        let boxes = prepaint.links.clone();
        let pressed = state.pressed.clone();
        let links = self.links.clone();
        window.on_mouse_event(move |e: &MouseUpEvent, phase, window, cx| {
            if !phase.bubble() || e.button != MouseButton::Left {
                return;
            }
            let Some(ix) = pressed.take() else { return };
            let still_on_it = boxes.iter().any(|(i, hb, _)| *i == ix && hb.is_hovered(window));
            // The end of a drag that selected text is not a click on the link.
            if still_on_it && !TextSelection::has_selection(window, cx) {
                if let Some((_, url)) = links.get(ix) {
                    commands::open_link(url);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::RichText;
    use gpui_kit::base::{TextSelection, TextSelectionLayer};
    use gpui_kit::{
        div, px, Context, IntoElement, Modifiers, MouseButton, ParentElement, Render, Styled, TestAppContext, Window,
    };

    struct View;

    impl Render for View {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let text = "\u{2009}\u{200A}@Sander\u{2009}\u{200A} hoi";
            let plate = std::iter::once(0..text.len() - 4).collect();
            div().size_full().child(TextSelectionLayer).child(
                div().w(px(600.)).h(px(40.)).child(
                    RichText::new("run", text)
                        .plates(plate, gpui_kit::black())
                        .no_copy(vec![0, 3, 13, 16]),
                ),
            )
        }
    }

    /// Dragging across a padded mention copies the words without the padding. Drop the
    /// `no_copy` filter in `paint` and the thin spaces end up on the clipboard.
    #[gpui_kit::test]
    fn a_selection_copies_without_the_mention_padding(cx: &mut TestAppContext) {
        let (_, cx) = cx.add_window_view(|_, _| View);
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.simulate_mouse_down(gpui_kit::point(px(0.), px(10.)), MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_move(gpui_kit::point(px(590.), px(10.)), Some(MouseButton::Left), Modifiers::default());
        cx.simulate_mouse_up(gpui_kit::point(px(590.), px(10.)), MouseButton::Left, Modifiers::default());
        cx.update(|window, cx| {
            let _ = window.draw(cx);
            assert_eq!(TextSelection::selected_text(window, cx), "@Sander hoi");
        });
    }
}
