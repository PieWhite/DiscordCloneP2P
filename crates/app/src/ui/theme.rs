//! Kleuren en algemene stijl voor het Discord-achtige ontwerp. Eén vaste combinatie —
//! geen `Theme`-struct met methodes, er is maar één thema en geen runtime-wisseling.
//!
//! Kleurwaarden zijn overgenomen uit de aangeleverde mockup (OKLCH) en met de hand
//! omgezet naar sRGB `Color32`; ze zijn benaderingen om visueel op te finetunen, geen
//! pixel-exacte conversie.

use eframe::egui;
use egui::{Color32, CornerRadius, Stroke};

/// Donkerste laag: titelbalk, icoonrail, statusbalk. ~oklch(12-14% 0.01 250).
pub const BG_DEEPEST: Color32 = Color32::from_rgb(15, 18, 23);
/// Zijbalken: kanaallijst, DM-lijst, ledenlijst, instellingen-tabrail. ~oklch(17% 0.012 250).
pub const BG_SIDEBAR: Color32 = Color32::from_rgb(23, 27, 33);
/// Chatcanvas. ~oklch(19% 0.012 250).
pub const BG_CANVAS: Color32 = Color32::from_rgb(27, 32, 39);
/// Kaarten/invoervelden die iets lichter moeten staan dan hun achtergrond. ~oklch(23% 0.013 250).
pub const BG_CARD: Color32 = Color32::from_rgb(34, 40, 48);
/// Hover/actieve achtergrond, iets lichter dan `BG_CARD`. ~oklch(28% 0.014 250).
pub const BG_RAISED: Color32 = Color32::from_rgb(43, 50, 60);

/// Hairline-rand, één stap lichter dan de achtergrond waar hij op staat. ~oklch(24-27% 0.014 250).
pub const BORDER: Color32 = Color32::from_rgb(46, 53, 63);

/// Hoofdtekst. ~oklch(88-92% 0.005 250).
pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(224, 229, 235);
/// Gedempte tekst: microlabels, tijdstempels, hints. ~oklch(55-65% 0.01 250).
pub const TEXT_MUTED: Color32 = Color32::from_rgb(148, 156, 166);

/// Vaste accentkleur (teal), rechtstreeks uit de mockup — geen kleurkeuze, één thema.
pub const ACCENT: Color32 = Color32::from_rgb(0x3A, 0xBF, 0xC0);
/// Tekst die op een accent-gevulde achtergrond moet staan (donker, zoals de mockup's avatars).
pub const ON_ACCENT: Color32 = Color32::from_rgb(20, 24, 29);

pub const STATUS_ONLINE: Color32 = Color32::from_rgb(85, 201, 117);
pub const STATUS_CONNECTING: Color32 = Color32::from_rgb(219, 180, 74);
pub const STATUS_DND: Color32 = Color32::from_rgb(232, 88, 84);
pub const STATUS_OFFLINE: Color32 = Color32::from_rgb(96, 102, 110);

/// Ronding voor kaarten/knoppen/invoervelden.
pub const ROUNDING: CornerRadius = CornerRadius::same(8);
/// Ronding voor pillen/chips/tabs — ronder, want vaak maar ~20-26px hoog.
pub const ROUNDING_PILL: CornerRadius = CornerRadius::same(10);

pub const BORDER_STROKE: Stroke = Stroke {
    width: 1.0,
    color: BORDER,
};

/// Past het thema toe op de hele app. Wordt één keer aangeroepen vanuit `main.rs`,
/// vóór het eerste frame.
pub fn apply(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();

    visuals.window_fill = BG_SIDEBAR;
    visuals.panel_fill = BG_CANVAS;
    visuals.extreme_bg_color = BG_DEEPEST;
    visuals.faint_bg_color = BG_CARD;
    visuals.override_text_color = Some(TEXT_PRIMARY);

    visuals.widgets.noninteractive.bg_fill = BG_SIDEBAR;
    visuals.widgets.noninteractive.weak_bg_fill = BG_CANVAS;
    visuals.widgets.noninteractive.bg_stroke = BORDER_STROKE;
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT_MUTED);

    visuals.widgets.inactive.bg_fill = BG_CARD;
    visuals.widgets.inactive.weak_bg_fill = BG_CARD;
    visuals.widgets.inactive.bg_stroke = BORDER_STROKE;
    visuals.widgets.inactive.corner_radius = ROUNDING;
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT_PRIMARY);

    visuals.widgets.hovered.bg_fill = BG_RAISED;
    visuals.widgets.hovered.weak_bg_fill = BG_RAISED;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    visuals.widgets.hovered.corner_radius = ROUNDING;
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, TEXT_PRIMARY);

    visuals.widgets.active.bg_fill = BG_RAISED;
    visuals.widgets.active.weak_bg_fill = BG_RAISED;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    visuals.widgets.active.corner_radius = ROUNDING;
    visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, TEXT_PRIMARY);

    visuals.selection.bg_fill = ACCENT.gamma_multiply(0.35);
    visuals.selection.stroke = Stroke::new(1.0_f32, ACCENT);

    visuals.window_corner_radius = ROUNDING;
    visuals.window_stroke = BORDER_STROKE;
    visuals.menu_corner_radius = ROUNDING;

    visuals.hyperlink_color = ACCENT;

    ctx.set_visuals(visuals);
}
