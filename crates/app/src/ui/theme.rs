//! Design B, "black chrome", as Rust: the palette, the alias tokens views use, the layout
//! sizes, the radii, the one soft shadow, and the fonts that ship inside the binary.
//!
//! A colour that is not one of the [`c`] values (or an alpha of one, as [`BACKDROP`]) is a
//! bug. Views use the alias tokens below where one exists, so a later palette change stays
//! one edit.
//!
//! The grammar: pure black ground, 1px hairlines, no hard shadows. Hover is [`BG_HOVER`];
//! selected is [`BG_SELECTED`] with a [`SELECTED_EDGE`] line and [`TEXT_STRONG`] text, never
//! an inversion. Disabled is [`DISABLED_OPACITY`]. Only popovers and dialogs carry
//! [`soft_shadow`]. Radii: [`RADIUS_CONTROL`] for controls and rows, [`RADIUS_FIELD`] for the
//! composer and self panel, [`RADIUS_CARD`] for cards, popovers and dialogs, `rounded_full`
//! for avatars, dots and badges.
//!
//! Type: [`UI_FONT`] (Inter) for body at 14px, [`DISPLAY_FONT`] (Instrument Serif) for pane
//! and dialog titles, [`MONO_FONT`] for timestamps, counts, RTT, file names and the
//! uppercase section labels. Weights: names, buttons and badges 500, monograms 600, the
//! rest regular. GPUI has no `letter-spacing` (see `widgets::tracked`) and no
//! `text-transform` (call `.to_uppercase()`).

use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::{point, px, App, BoxShadow, Hsla, Pixels, Rgba};
use std::borrow::Cow;

const fn hex(v: u32) -> Rgba {
    Rgba {
        r: ((v >> 16) & 0xff) as f32 / 255.0,
        g: ((v >> 8) & 0xff) as f32 / 255.0,
        b: (v & 0xff) as f32 / 255.0,
        a: 1.0,
    }
}

/// The palette; nothing else exists.
pub mod c {
    use super::{hex, Rgba};
    /// The ground of everything: panes, buttons, popovers, dialogs.
    pub const VOID: Rgba = hex(0x000000);
    /// Hover and selected rows, avatar grounds, meter tracks.
    pub const LIFT: Rgba = hex(0x0B0E14);
    /// A message under the pointer, wells, empty video tiles.
    pub const WELL: Rgba = hex(0x05070A);
    /// Every line: pane edges, card and control frames.
    pub const HAIRLINE: Rgba = hex(0x292D30);
    /// The offline ring, the unlit dot.
    pub const CHARCOAL: Rgba = hex(0x464A4D);
    /// Hover frames, timestamps, quiet glyphs.
    pub const IRON: Rgba = hex(0x6E727A);
    /// Muted text: secondary lines, section labels.
    pub const ASH: Rgba = hex(0xA1A4A5);
    /// Body text.
    pub const BONE: Rgba = hex(0xF0F0F0);
    /// Headings, names, selected rows.
    pub const WHITE: Rgba = hex(0xFFFFFF);
    /// The one accent: focus ring, badges, sliders, the watched stream, the NEW divider.
    pub const CHROME: Rgba = hex(0xD4D7DB);
    /// Links and code numbers.
    pub const CHROME_DIM: Rgba = hex(0xB8BCC2);
    /// Presence: online, in call, speaking, success.
    pub const GREEN: Rgba = hex(0x3AD389);
    /// LIVE, errors, Leave, do-not-disturb, a switched-off toggle.
    pub const RED: Rgba = hex(0xFF9592);
    /// The frame of anything red.
    pub const RED_LINE: Rgba = hex(0x5A2A2A);
    /// The ground of a switched-off toggle.
    pub const RED_BG: Rgba = hex(0x1A0B0B);
    /// Connecting, away, Wordle near.
    pub const AMBER: Rgba = hex(0xFFCA16);
    /// Identity hue 1, code keys and file names.
    pub const SKY: Rgba = hex(0x70B8FF);
}

// ------------------------------------------------------------------ alias tokens

pub const BG_CANVAS: Rgba = c::VOID;
pub const BG_SIDEBAR: Rgba = c::VOID;
pub const BG_RAISED: Rgba = c::VOID;
pub const BG_CODE: Rgba = c::VOID;
pub const BG_WELL: Rgba = c::WELL;

/// Hover is a flat lift; selected is the same lift with a hairline frame and white text.
pub const BG_HOVER: Rgba = c::LIFT;
pub const BG_SELECTED: Rgba = c::LIFT;
pub const SELECTED_EDGE: Rgba = c::HAIRLINE;
pub const BG_MSG_HOVER: Rgba = c::WELL;

pub const BORDER: Rgba = c::HAIRLINE;
/// The frame of a control under the pointer.
pub const BORDER_HOVER: Rgba = c::IRON;

/// Three text tiers plus white for what must stand out.
pub const TEXT_STRONG: Rgba = c::WHITE;
pub const TEXT: Rgba = c::BONE;
pub const TEXT_MUTED: Rgba = c::ASH;
pub const TEXT_DIM: Rgba = c::IRON;

pub const ACCENT: Rgba = c::CHROME;
/// Text on a chrome fill (badges, the drop target).
pub const ON_ACCENT: Rgba = c::VOID;
pub const LINK: Rgba = c::CHROME_DIM;

pub const ONLINE: Rgba = c::GREEN;
pub const CONNECTING: Rgba = c::AMBER;
pub const DND: Rgba = c::RED;
pub const OFFLINE: Rgba = c::CHARCOAL;

/// A message that mentions you: a lift with a chrome edge.
pub const MENTION_BG: Rgba = c::LIFT;
pub const MENTION_BORDER: Rgba = c::CHROME;

pub const DANGER: Rgba = c::RED;
pub const DANGER_TEXT: Rgba = c::RED;
pub const DANGER_LINE: Rgba = c::RED_LINE;
/// The ground of a pressed "off" toggle (mic off, deafened, do-not-disturb).
pub const DANGER_BG: Rgba = c::RED_BG;

/// Per-peer identity hues, indexed by `PeerState::avatar` (1, 2, 3).
pub const AV_1: Rgba = c::SKY;
pub const AV_2: Rgba = c::AMBER;
pub const AV_3: Rgba = c::GREEN;

/// The identity hue for an `avatar` number from the state (1..=3; anything else is 3, the
/// same fallback `state.rs` uses). It colours the monogram, not a fill.
pub fn avatar_hue(avatar: u8) -> Rgba {
    match avatar {
        1 => AV_1,
        2 => AV_2,
        _ => AV_3,
    }
}

pub const WDL_HIT: Rgba = c::GREEN;
pub const WDL_NEAR: Rgba = c::AMBER;
pub const WDL_MISS: Rgba = c::HAIRLINE;

pub const CODE_KEY: Rgba = c::SKY;
pub const CODE_STR: Rgba = c::BONE;
pub const CODE_NUM: Rgba = c::CHROME_DIM;

/// The modal dim and the drop overlay: flat void at 85%.
pub const BACKDROP: Rgba = Rgba { a: 0.85, ..c::VOID };
/// `:disabled`: everything unavailable is this transparent, never a second colour.
pub const DISABLED_OPACITY: f32 = 0.45;

// ------------------------------------------------------------------ sizes

pub const TITLEBAR_H: Pixels = px(36.);
pub const STATUS_H: Pixels = px(28.);
pub const RAIL_W: Pixels = px(60.);
pub const CHAN_W: Pixels = px(248.);
/// `@media (max-width: 820px)` in the stylesheet.
pub const CHAN_W_NARROW: Pixels = px(200.);
pub const MEMBER_W: Pixels = px(240.);
/// Every pane's head (channels, timeline, members).
pub const HEAD_H: Pixels = px(60.);
pub const SETTINGS_NAV_W: Pixels = px(220.);
/// Below this the member list closes, as state (see `AppView::apply_width`).
pub const NARROW_MEMBERS: Pixels = px(1080.);
/// Below this the channel column slims and the topic hides.
pub const NARROW_CHANNELS: Pixels = px(820.);
/// The macOS traffic lights sit in the titlebar; this keeps the brand clear of them.
pub const MAC_TITLEBAR_INSET: Pixels = px(84.);

pub const RADIUS_CONTROL: Pixels = px(6.);
pub const RADIUS_FIELD: Pixels = px(10.);
pub const RADIUS_CARD: Pixels = px(16.);

// ------------------------------------------------------------------ type

pub const UI_FONT: &str = "Inter";
pub const DISPLAY_FONT: &str = "Instrument Serif";
pub const MONO_FONT: &str = "JetBrains Mono";
/// `body { font-size: 14px; line-height: 1.45 }`.
pub const BASE_TEXT: Pixels = px(14.);
pub const BASE_LINE_HEIGHT: f32 = 1.45;
/// A pane head's title; the settings page uses [`DISPLAY_TEXT_LARGE`].
pub const DISPLAY_TEXT: Pixels = px(26.);
pub const DISPLAY_TEXT_LARGE: Pixels = px(32.);
/// The uppercase mono section labels and every small mono reading.
pub const LABEL_TEXT: Pixels = px(11.5);

/// Also the in-game overlay's font (decision 53), loaded into GDI as "Inter Medium".
pub const INTER_MEDIUM: &[u8] = include_bytes!("assets/fonts/Inter-Medium.ttf");

const FONTS: [&[u8]; 5] = [
    include_bytes!("assets/fonts/Inter-Regular.ttf"),
    INTER_MEDIUM,
    include_bytes!("assets/fonts/Inter-SemiBold.ttf"),
    include_bytes!("assets/fonts/InstrumentSerif-Regular.ttf"),
    include_bytes!("assets/fonts/JetBrainsMono-Regular.ttf"),
];

/// The pop-out viewer's right-click menu in the B tokens: the popover's ground, hairline and
/// corners, `popover_item`'s text and hover. The video crate draws it and knows no colours.
pub fn viewer_menu_style() -> fitcom_video::MenuStijl {
    let rgb = |c: Rgba| [c.r, c.g, c.b].map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u8);
    fitcom_video::MenuStijl {
        grond: rgb(BG_RAISED),
        rand: rgb(BORDER),
        hover: rgb(BG_HOVER),
        tekst: rgb(TEXT_MUTED),
        sterk: rgb(TEXT_STRONG),
        gedimd: rgb(TEXT_DIM),
        accent: rgb(ACCENT),
        hoek: f32::from(RADIUS_CARD),
        hoek_rij: f32::from(RADIUS_CONTROL),
        font: INTER_MEDIUM,
        font_naam: "Inter Medium",
    }
}

// ------------------------------------------------------------------ shadow

/// `0 20px 50px rgba(0,0,0,.8)`: the only shadow, for popovers and dialogs.
pub fn soft_shadow() -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: Hsla::from(c::VOID).opacity(0.8),
        offset: point(px(0.), px(20.)),
        blur_radius: px(50.),
        spread_radius: px(0.),
        inset: false,
    }]
}

// ------------------------------------------------------------------ installation

/// Registers the bundled fonts and dresses gpui-kit's own components in this world, so a
/// Select, Slider, Switch, Tooltip or PopupMenu from the kit arrives in black chrome. Call
/// once, after `gpui_kit::init` and before the window opens: the fonts must exist before
/// the theme names them, or the kit swaps in a fallback.
pub fn install(cx: &mut App) {
    if let Err(e) = cx
        .text_system()
        .add_fonts(FONTS.iter().map(|f| Cow::Borrowed(*f)).collect())
    {
        tracing::warn!(error = %e, "registering the bundled fonts failed");
    }

    Theme::change(ThemeMode::Dark, None, cx);
    Theme::update(cx, |t| {
        t.font_family = UI_FONT.into();
        t.font_size = BASE_TEXT;
        t.mono_font_family = MONO_FONT.into();
        t.mono_font_size = px(12.5);
        t.radius = RADIUS_CONTROL;
        t.radius_lg = RADIUS_CARD;
        t.shadow = false;

        let k = &mut t.colors;
        let h = |v: Rgba| Hsla::from(v);
        k.background = h(BG_CANVAS);
        k.foreground = h(TEXT);
        k.border = h(BORDER);
        k.input = h(BORDER);
        k.ring = h(ACCENT);
        k.caret = h(TEXT_STRONG);
        k.selection = h(c::CHROME).opacity(0.25);
        k.accent = h(BG_HOVER);
        k.accent_foreground = h(TEXT_STRONG);
        k.primary = h(c::VOID);
        k.primary_hover = h(BG_HOVER);
        k.primary_active = h(BG_SELECTED);
        k.primary_foreground = h(TEXT_STRONG);
        k.secondary = h(c::VOID);
        k.secondary_hover = h(BG_HOVER);
        k.secondary_active = h(BG_SELECTED);
        k.secondary_foreground = h(TEXT);
        k.muted = h(c::LIFT);
        k.muted_foreground = h(TEXT_MUTED);
        k.popover = h(BG_RAISED);
        k.popover_foreground = h(TEXT);
        k.danger = h(DANGER_BG);
        k.danger_hover = h(DANGER_BG);
        k.danger_active = h(DANGER_BG);
        k.danger_foreground = h(DANGER_TEXT);
        k.success = h(ONLINE);
        k.warning = h(CONNECTING);
        k.info = h(ACCENT);
        k.link = h(LINK);
        k.link_hover = h(TEXT_STRONG);
        k.link_active = h(TEXT_STRONG);
        k.list = h(c::VOID);
        k.list_hover = h(BG_HOVER);
        k.list_active = h(BG_SELECTED);
        k.list_active_border = h(SELECTED_EDGE);
        k.scrollbar = h(c::VOID).opacity(0.);
        k.scrollbar_thumb = h(c::CHARCOAL);
        k.scrollbar_thumb_hover = h(c::IRON);
        k.slider_bar = h(ACCENT);
        k.slider_thumb = h(c::WHITE);
        k.switch = h(c::LIFT);
        k.switch_thumb = h(c::ASH);
        k.progress_bar = h(ACCENT);
        k.overlay = h(BACKDROP);
        k.window_border = h(BORDER);
        k.title_bar = h(c::VOID);
        k.title_bar_border = h(BORDER);
        k.sidebar = h(BG_SIDEBAR);
        k.sidebar_foreground = h(TEXT);
        k.sidebar_border = h(BORDER);
        k.drop_target = h(c::LIFT);
        k.drag_border = h(ACCENT);
    });
}
