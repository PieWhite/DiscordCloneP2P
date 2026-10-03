//! The sixteen-colour world, as Rust: every token from `frontend/app.css` `:root`, the
//! layout sizes, the hard shadows, the two fonts, and the three things CSS drew that GPUI
//! has no primitive for — the ordered 2px dither, the hard-stop horizon band, and fonts
//! that ship inside the binary.
//!
//! A colour that is not one of the sixteen [`c`] values (or a [`dither`] of two of them)
//! is a bug, exactly as the stylesheet said. Views use the alias tokens below where CSS
//! used an alias, so a later palette change stays one edit.
//!
//! Not portable from CSS, on purpose: `letter-spacing` (GPUI has none; the uppercase
//! strips read a little tighter) and `text-transform` (call `.to_uppercase()` on the
//! string). One weight only: never set `font_weight`, the face has no bold and GPUI would
//! pick the regular cut anyway.

use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::{
    canvas, div, point, px, size, App, Bounds, BoxShadow, Corners, Div, IntoElement, ParentElement,
    Pixels, RenderImage, Rgba, Styled, Window,
};
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

const fn hex(v: u32) -> Rgba {
    Rgba {
        r: ((v >> 16) & 0xff) as f32 / 255.0,
        g: ((v >> 8) & 0xff) as f32 / 255.0,
        b: (v & 0xff) as f32 / 255.0,
        a: 1.0,
    }
}

/// The sixteen colours, top to bottom; nothing else exists.
pub mod c {
    use super::{hex, Rgba};
    /// 00 the void: wells, code grounds, hard shadows.
    pub const SPACE: Rgba = hex(0x070810);
    /// 01 window frames, sidebars, cards.
    pub const INK: Rgba = hex(0x10122A);
    /// 02 the canvas — the largest field.
    pub const INDIGO: Rgba = hex(0x191D3A);
    /// 03 structure: hairlines, hover half-step.
    pub const VIOLET: Rgba = hex(0x2E3464);
    /// 04 interactive edges, disabled glyphs.
    pub const STEEL: Rgba = hex(0x4A5490);
    /// 05 secondary text — the only quiet tier.
    pub const SILVER: Rgba = hex(0xA7AECC);
    /// 06 primary text, and the ground of an inverted cell.
    pub const PAPER: Rgba = hex(0xEFE9D5);
    /// 07 title strips.
    pub const BLUE: Rgba = hex(0x3B58C4);
    /// 08 the working accent: links, focus, live progress.
    pub const CYAN: Rgba = hex(0x5FD3E0);
    /// 09 code keys, identity hue 1.
    pub const SKY: Rgba = hex(0x8FB8F0);
    /// 0A the primary action, badges, do-not-disturb, LIVE.
    pub const MAGENTA: Rgba = hex(0xD460A2);
    /// 0B failure, and only failure.
    pub const CORAL: Rgba = hex(0xF0716B);
    /// 0C the mention frame, identity hue 3, the warm band.
    pub const ORANGE: Rgba = hex(0xE08A48);
    /// 0D connecting, code numbers.
    pub const AMBER: Rgba = hex(0xE7BC5D);
    /// 0E code strings, identity hue 2.
    pub const LIME: Rgba = hex(0xC4DC78);
    /// 0F presence: online, in call, speaking.
    pub const GREEN: Rgba = hex(0x62CE82);
}

// ------------------------------------------------------------------ alias tokens
// One constant per custom property in `app.css` `:root`, same name in SCREAMING_CASE.

pub const BG_WINDOW: Rgba = c::INK;
pub const BG_SIDEBAR: Rgba = c::INK;
pub const BG_CANVAS: Rgba = c::INDIGO;
pub const BG_RAISED: Rgba = c::INK;
pub const BG_CODE: Rgba = c::SPACE;
pub const BG_WELL: Rgba = c::SPACE;

/// Hover is a flat violet slab (with the ▶ marker on menu rows); current/pressed is a full
/// cell inversion: paper ground, space ink.
pub const BG_HOVER: Rgba = c::VIOLET;
pub const BG_ACTIVE: Rgba = c::PAPER;
pub const ON_INVERT: Rgba = c::SPACE;
pub const BG_MSG_HOVER: Rgba = c::INK;
pub const BG_MENTION_HOVER: Rgba = c::INK;

pub const BORDER: Rgba = c::VIOLET;
pub const BORDER_STRONG: Rgba = c::STEEL;
pub const BORDER_INPUT: Rgba = c::SILVER;

/// Two text tiers, not four: paper and silver.
pub const TEXT: Rgba = c::PAPER;
pub const TEXT_MID: Rgba = c::PAPER;
pub const TEXT_MUTED: Rgba = c::SILVER;
pub const TEXT_DIM: Rgba = c::SILVER;

pub const ACCENT: Rgba = c::CYAN;
pub const ACCENT_HOVER: Rgba = c::CYAN;
pub const ACCENT_TEXT: Rgba = c::CYAN;
pub const ON_ACCENT: Rgba = c::SPACE;

/// Magenta with a paper frame — the one saturated fill in the resting window.
pub const PRIMARY: Rgba = c::MAGENTA;
pub const ON_PRIMARY: Rgba = c::SPACE;

pub const ONLINE: Rgba = c::GREEN;
pub const CONNECTING: Rgba = c::AMBER;
pub const DND: Rgba = c::MAGENTA;
pub const OFFLINE: Rgba = c::STEEL;

pub const MENTION_BG: Rgba = c::SPACE;
pub const MENTION_BORDER: Rgba = c::ORANGE;

pub const DANGER: Rgba = c::CORAL;
pub const DANGER_BG: Rgba = c::SPACE;
pub const DANGER_TEXT: Rgba = c::CORAL;
pub const DANGER_HI: Rgba = c::PAPER;
pub const DANGER_LINE: Rgba = c::CORAL;
pub const DANGER_HOV: Rgba = c::CORAL;
pub const CLOSE_HOVER: Rgba = c::CORAL;
/// Text that sits on a filled signal colour (badges, the close button, LIVE).
pub const ON_SIGNAL: Rgba = c::SPACE;

/// Per-peer identity hues, indexed by `PeerState::avatar` (1, 2, 3).
pub const AV_1: Rgba = c::SKY;
pub const AV_2: Rgba = c::LIME;
pub const AV_3: Rgba = c::ORANGE;

/// The identity hue for an `avatar` number from the state (1..=3; anything else is 3, the
/// same fallback `state.rs` uses).
pub fn avatar_hue(avatar: u8) -> Rgba {
    match avatar {
        1 => AV_1,
        2 => AV_2,
        _ => AV_3,
    }
}

pub const WDL_HIT: Rgba = c::GREEN;
pub const WDL_NEAR: Rgba = c::AMBER;
pub const WDL_MISS: Rgba = c::STEEL;

pub const CODE_KEY: Rgba = c::SKY;
pub const CODE_STR: Rgba = c::LIME;
pub const CODE_NUM: Rgba = c::AMBER;

// ------------------------------------------------------------------ sizes

pub const TITLEBAR_H: Pixels = px(32.);
pub const STATUS_H: Pixels = px(26.);
pub const RAIL_W: Pixels = px(56.);
pub const CHAN_W: Pixels = px(240.);
/// `@media (max-width: 820px)` in the stylesheet.
pub const CHAN_W_NARROW: Pixels = px(200.);
pub const MEMBER_W: Pixels = px(232.);
/// Every zone's title strip (channels, timeline, members, settings nav).
pub const HEAD_H: Pixels = px(44.);
pub const SETTINGS_NAV_W: Pixels = px(208.);
/// Below this the member list closes, as state (see `AppView::apply_width`).
pub const NARROW_MEMBERS: Pixels = px(1080.);
/// Below this the channel column slims and the topic hides.
pub const NARROW_CHANNELS: Pixels = px(820.);
/// The macOS traffic lights sit in the titlebar; this keeps the brand clear of them.
pub const MAC_TITLEBAR_INSET: Pixels = px(84.);

/// Square corners are the world. Kept as a token so a later world can turn them back on.
pub const RADIUS: Pixels = px(0.);

// ------------------------------------------------------------------ type

pub const UI_FONT: &str = "DotGothic16";
pub const MONO_FONT: &str = "JetBrains Mono";
/// `body { font-size: 15px; line-height: 1.45 }`.
pub const BASE_TEXT: Pixels = px(15.);
pub const BASE_LINE_HEIGHT: f32 = 1.45;

const FONT_UI: &[u8] = include_bytes!("assets/fonts/DotGothic16-Regular.ttf");
const FONT_MONO: &[u8] = include_bytes!("assets/fonts/JetBrainsMono-Regular.ttf");

// ------------------------------------------------------------------ shadows

/// A hard offset in the void — no blur, no glow. `--shadow-thumb` is 1, `--shadow-bar` 3,
/// `--shadow-popover` 4, `--shadow-modal` 8.
pub fn hard_shadow(offset: f32) -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: c::SPACE.into(),
        offset: point(px(offset), px(offset)),
        blur_radius: px(0.),
        spread_radius: px(0.),
        inset: false,
    }]
}

pub fn shadow_thumb() -> Vec<BoxShadow> {
    hard_shadow(1.)
}
pub fn shadow_bar() -> Vec<BoxShadow> {
    hard_shadow(3.)
}
pub fn shadow_popover() -> Vec<BoxShadow> {
    hard_shadow(4.)
}
pub fn shadow_modal() -> Vec<BoxShadow> {
    hard_shadow(8.)
}

/// The button chassis: `box-shadow: 0 0 0 1px space, <offset>px <offset>px 0 space` — a dark
/// outer line plus a hard drop. Used by `.join`, the dialog buttons and the composer frame.
pub fn chassis_shadow(offset: f32) -> Vec<BoxShadow> {
    vec![
        BoxShadow {
            color: c::SPACE.into(),
            offset: point(px(0.), px(0.)),
            blur_radius: px(0.),
            spread_radius: px(1.),
            inset: false,
        },
        BoxShadow {
            color: c::SPACE.into(),
            offset: point(px(offset), px(offset)),
            blur_radius: px(0.),
            spread_radius: px(0.),
            inset: false,
        },
    ]
}

// ------------------------------------------------------------------ the band

/// The horizon band: a hard-stop evening sky, cold to warm, at full strength across the
/// whole width. Six flat segments in the stylesheet's proportions; GPUI's gradients have
/// two stops and would blend, which is exactly what this world does not do.
pub fn band(height: Pixels) -> Div {
    const SEGMENTS: [(Rgba, f32); 6] = [
        (c::CYAN, 0.17),
        (c::SKY, 0.17),
        (c::BLUE, 0.16),
        (c::MAGENTA, 0.17),
        (c::CORAL, 0.17),
        (c::ORANGE, 0.16),
    ];
    div()
        .flex()
        .flex_row()
        .w_full()
        .h(height)
        .flex_none()
        .children(SEGMENTS.iter().map(|(colour, share)| {
            div()
                .h_full()
                .flex_basis(gpui_kit::relative(*share))
                .bg(*colour)
        }))
}

// ------------------------------------------------------------------ the dither

/// Which cells of each 2×2 block are lit, after the two conic gradients in the stylesheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dither {
    /// `--dither-half` / `--backdrop`: top-right and bottom-left lit — a checkerboard. What
    /// "unavailable", the modal dim and the drop overlay look like.
    Half,
    /// The 4px paper hover on `.join` and `.btn--accent`: only the top-right cell — a quarter
    /// of the cells flip.
    Quarter,
}

impl Dither {
    fn lit(self, cx: u32, cy: u32) -> bool {
        match self {
            Dither::Half => (cx + cy) % 2 == 1,
            Dither::Quarter => cx % 2 == 1 && cy.is_multiple_of(2),
        }
    }
}

/// Logical size of one cached tile. Big enough that the whole-window backdrop is a few
/// dozen sprites, small enough that the atlas entry is a few hundred kilobytes.
const TILE: f32 = 128.;

/// (pattern, colour, cell in device pixels, scale bits) -> (tile, logical edge).
type TileCache = HashMap<(Dither, u32, u32, u32), (Arc<RenderImage>, f32)>;

thread_local! {
    static TILES: RefCell<TileCache> = RefCell::new(HashMap::new());
}

fn colour_key(colour: Rgba) -> u32 {
    u32::from(colour)
}

/// The tile for one pattern, colour, cell size and scale factor, built once at device
/// resolution so every cell lands on whole device pixels. Returns the tile and its logical
/// edge length.
fn tile(pattern: Dither, colour: Rgba, cell: f32, scale: f32) -> (Arc<RenderImage>, f32) {
    let cell_dev = (cell * scale).round().max(1.) as u32;
    let key = (pattern, colour_key(colour), cell_dev, scale.to_bits());
    TILES.with(|tiles| {
        tiles
            .borrow_mut()
            .entry(key)
            .or_insert_with(|| {
                // A whole number of 2×2 blocks, so the pattern continues across tiles.
                let blocks = ((TILE * scale) / (2 * cell_dev) as f32).round().max(1.) as u32;
                let edge = blocks * 2 * cell_dev;
                let [r, g, b] = [colour.r, colour.g, colour.b].map(|v| (v * 255.).round() as u8);
                let mut bgra = vec![0u8; (edge * edge * 4) as usize];
                for y in 0..edge {
                    for x in 0..edge {
                        if pattern.lit(x / cell_dev, y / cell_dev) {
                            let i = ((y * edge + x) * 4) as usize;
                            bgra[i..i + 4].copy_from_slice(&[b, g, r, 255]);
                        }
                    }
                }
                let buffer = image::RgbaImage::from_raw(edge, edge, bgra)
                    .expect("buffer is edge*edge*4 by construction");
                let image = Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]));
                (image, edge as f32 / scale)
            })
            .clone()
    })
}

/// An ordered dither of `colour` over whatever is underneath, filling its parent. Put it as
/// the first child of a `relative()` element; the lit cells are `colour`, the rest show the
/// parent's own background. `cell` is 2 for every surface in the stylesheet except the
/// member list's quiet rows, which use 1.
///
/// Costs nothing when nothing changes: the tile is built once per (pattern, colour, cell,
/// scale) and painted as a handful of atlas sprites only when a frame is drawn anyway.
pub fn dither(pattern: Dither, colour: Rgba, cell: f32) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window: &mut Window, _cx: &mut App| {
            paint_dither(bounds, pattern, colour, cell, window);
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

fn paint_dither(
    bounds: Bounds<Pixels>,
    pattern: Dither,
    colour: Rgba,
    cell: f32,
    window: &mut Window,
) {
    let (image, edge) = tile(pattern, colour, cell, window.scale_factor());
    let edge = px(edge);
    let mut y = bounds.origin.y;
    while y < bounds.bottom() {
        let mut x = bounds.origin.x;
        while x < bounds.right() {
            let tile_bounds = Bounds::new(point(x, y), size(edge, edge));
            // `paint_image` draws the intersection with `bounds`, so edge tiles clip.
            let _ = window.paint_image(
                bounds,
                tile_bounds,
                Corners::default(),
                image.clone(),
                0,
                false,
            );
            x += edge;
        }
        y += edge;
    }
}

// ------------------------------------------------------------------ installation

/// Registers the two fonts and dresses gpui-kit's own components in this world, so a
/// Select, Slider, Switch, Tooltip or PopupMenu from the kit arrives with the sixteen
/// colours and square corners. Call once, after `gpui_kit::init` and before the window
/// opens: the fonts must exist before the theme names them, or the kit swaps in a fallback.
pub fn install(cx: &mut App) {
    if let Err(e) = cx
        .text_system()
        .add_fonts(vec![Cow::Borrowed(FONT_UI), Cow::Borrowed(FONT_MONO)])
    {
        tracing::warn!(error = %e, "registering the bundled fonts failed");
    }

    Theme::change(ThemeMode::Dark, None, cx);
    Theme::update(cx, |t| {
        t.font_family = UI_FONT.into();
        t.font_size = BASE_TEXT;
        t.mono_font_family = MONO_FONT.into();
        t.mono_font_size = px(12.5);
        t.radius = RADIUS;
        t.radius_lg = RADIUS;
        t.shadow = false;

        let k = &mut t.colors;
        let h = |v: Rgba| gpui_kit::Hsla::from(v);
        k.background = h(BG_CANVAS);
        k.foreground = h(TEXT);
        k.border = h(BORDER_STRONG);
        k.input = h(BORDER_STRONG);
        k.ring = h(ACCENT);
        k.caret = h(ACCENT);
        k.selection = h(c::PAPER).opacity(0.35);
        k.accent = h(BG_HOVER);
        k.accent_foreground = h(TEXT);
        k.primary = h(PRIMARY);
        k.primary_hover = h(PRIMARY);
        k.primary_active = h(PRIMARY);
        k.primary_foreground = h(ON_PRIMARY);
        k.secondary = h(c::SPACE);
        k.secondary_hover = h(c::PAPER);
        k.secondary_active = h(c::PAPER);
        k.secondary_foreground = h(TEXT);
        k.muted = h(c::INK);
        k.muted_foreground = h(TEXT_MUTED);
        k.popover = h(BG_RAISED);
        k.popover_foreground = h(TEXT);
        k.danger = h(DANGER);
        k.danger_hover = h(DANGER);
        k.danger_active = h(DANGER);
        k.danger_foreground = h(ON_SIGNAL);
        k.success = h(ONLINE);
        k.warning = h(CONNECTING);
        k.info = h(ACCENT);
        k.link = h(ACCENT);
        k.link_hover = h(ACCENT);
        k.link_active = h(ACCENT);
        k.list = h(c::INK);
        k.list_hover = h(BG_HOVER);
        k.list_active = h(BG_ACTIVE);
        k.list_active_border = h(BG_ACTIVE);
        k.scrollbar = h(c::SPACE).opacity(0.);
        k.scrollbar_thumb = h(c::STEEL);
        k.scrollbar_thumb_hover = h(c::SILVER);
        k.slider_bar = h(ACCENT);
        k.slider_thumb = h(c::PAPER);
        k.switch = h(c::SPACE);
        k.switch_thumb = h(c::SILVER);
        k.progress_bar = h(ACCENT);
        k.overlay = h(c::SPACE).opacity(0.5);
        k.window_border = h(c::VIOLET);
        k.title_bar = h(c::INK);
        k.title_bar_border = h(c::INK);
        k.sidebar = h(BG_SIDEBAR);
        k.sidebar_foreground = h(TEXT);
        k.sidebar_border = h(BORDER);
        k.drop_target = h(c::INDIGO);
        k.drag_border = h(ACCENT);
    });
}
