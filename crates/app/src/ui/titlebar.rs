//! Eigen titelbalk, want de OS-decoraties staan uit (`ViewportBuilder::with_decorations
//! (false)` in `main.rs`) om de mockup's titelbalk te kunnen volgen. Slepen en
//! maximaliseren/herstellen gaan via `ViewportCommand`; de sluitknop stuurt gewoon
//! `ViewportCommand::Close`, wat `close_requested()` volgende frame laat afgaan — dat
//! loopt al door `App::afsluiten_of_verbergen` (naar tray, niet direct afsluiten). Geen
//! aparte sluit-logica hier nodig.

use super::theme;
use eframe::egui;
use egui::{ResizeDirection, Sense, Stroke, ViewportCommand};

const HOOGTE: f32 = 34.0;
/// Breedte van de onzichtbare rand- en hoekzones waarmee je het venster kunt
/// vergroten/verkleinen — zonder OS-decoraties bestaat de eigen resize-grip van
/// Windows niet meer, dus die bouwen we zelf na met `ViewportCommand::BeginResize`.
const RESIZE_RAND: f32 = 6.0;
const RESIZE_HOEK: f32 = 12.0;

/// Onzichtbare grepen langs de vensterranden en -hoeken. Los van `titelbalk()` zelf
/// aangeroepen, en vóór de rest van de UI getekend, zodat deze zones altijd boven op
/// wat daaronder getekend wordt liggen — anders zou een paneel dat tot de rand
/// doorloopt de resize-greep onbereikbaar maken.
pub fn resize_randen(ctx: &egui::Context) {
    let rect = ctx.screen_rect();
    let r = RESIZE_RAND;
    let h = RESIZE_HOEK;

    let zones: [(egui::Rect, ResizeDirection, egui::CursorIcon); 8] = [
        (
            egui::Rect::from_min_max(
                egui::pos2(rect.min.x + h, rect.min.y),
                egui::pos2(rect.max.x - h, rect.min.y + r),
            ),
            ResizeDirection::North,
            egui::CursorIcon::ResizeVertical,
        ),
        (
            egui::Rect::from_min_max(
                egui::pos2(rect.min.x + h, rect.max.y - r),
                egui::pos2(rect.max.x - h, rect.max.y),
            ),
            ResizeDirection::South,
            egui::CursorIcon::ResizeVertical,
        ),
        (
            egui::Rect::from_min_max(
                egui::pos2(rect.min.x, rect.min.y + h),
                egui::pos2(rect.min.x + r, rect.max.y - h),
            ),
            ResizeDirection::West,
            egui::CursorIcon::ResizeHorizontal,
        ),
        (
            egui::Rect::from_min_max(
                egui::pos2(rect.max.x - r, rect.min.y + h),
                egui::pos2(rect.max.x, rect.max.y - h),
            ),
            ResizeDirection::East,
            egui::CursorIcon::ResizeHorizontal,
        ),
        (
            egui::Rect::from_min_size(rect.min, egui::vec2(h, h)),
            ResizeDirection::NorthWest,
            egui::CursorIcon::ResizeNwSe,
        ),
        (
            egui::Rect::from_min_size(egui::pos2(rect.max.x - h, rect.min.y), egui::vec2(h, h)),
            ResizeDirection::NorthEast,
            egui::CursorIcon::ResizeNeSw,
        ),
        (
            egui::Rect::from_min_size(egui::pos2(rect.min.x, rect.max.y - h), egui::vec2(h, h)),
            ResizeDirection::SouthWest,
            egui::CursorIcon::ResizeNeSw,
        ),
        (
            egui::Rect::from_min_size(egui::pos2(rect.max.x - h, rect.max.y - h), egui::vec2(h, h)),
            ResizeDirection::SouthEast,
            egui::CursorIcon::ResizeNwSe,
        ),
    ];

    for (i, (zone_rect, richting, cursor)) in zones.into_iter().enumerate() {
        egui::Area::new(egui::Id::new(("resize_rand", i)))
            .order(egui::Order::Foreground)
            .fixed_pos(zone_rect.min)
            .show(ctx, |ui| {
                let (_, resp) = ui.allocate_exact_size(zone_rect.size(), Sense::drag());
                if resp.hovered() {
                    ui.ctx().set_cursor_icon(cursor);
                }
                if resp.drag_started() {
                    ctx.send_viewport_cmd(ViewportCommand::BeginResize(richting));
                }
            });
    }
}

pub fn titlebar(ctx: &egui::Context) {
    egui::TopBottomPanel::top("titelbalk")
        .exact_height(HOOGTE)
        .frame(
            egui::Frame::new()
                .fill(theme::BG_DEEPEST)
                .inner_margin(egui::Margin::symmetric(10, 0))
                .stroke(Stroke::NONE),
        )
        .show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), Sense::hover());
                ui.painter().rect_filled(rect, 2.0, theme::ACCENT);
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new("FitCommunication")
                        .size(13.0)
                        .color(theme::TEXT_MUTED),
                );

                // Knoppenbreedte vooraf reserveren, zodat het slepbare middenstuk exact
                // de rest van de balk beslaat — geen overlap met de knoppen zelf.
                let knoppen_breedte = 3.0 * 30.0;
                let resterend = (ui.available_width() - knoppen_breedte).max(0.0);
                let (sleep_rect, sleep_resp) = ui.allocate_exact_size(
                    egui::vec2(resterend, ui.available_height()),
                    Sense::click_and_drag(),
                );
                if sleep_resp.drag_started() {
                    ctx.send_viewport_cmd(ViewportCommand::StartDrag);
                }
                if sleep_resp.double_clicked() {
                    let nu_gemaximaliseerd = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                    ctx.send_viewport_cmd(ViewportCommand::Maximized(!nu_gemaximaliseerd));
                }
                let _ = sleep_rect;

                ui.scope(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    if titelbalk_knop(ui, "\u{2212}").clicked() {
                        ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
                    }
                    let gemaximaliseerd = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                    let max_glyph = if gemaximaliseerd {
                        "\u{1F5D7}"
                    } else {
                        "\u{1F5D6}"
                    };
                    if titelbalk_knop(ui, max_glyph).clicked() {
                        ctx.send_viewport_cmd(ViewportCommand::Maximized(!gemaximaliseerd));
                    }
                    if titelbalk_knop_sluiten(ui).clicked() {
                        ctx.send_viewport_cmd(ViewportCommand::Close);
                    }
                });
            });
        });
}

fn titelbalk_knop(ui: &mut egui::Ui, glyph: &str) -> egui::Response {
    let knop = egui::Button::new(
        egui::RichText::new(glyph)
            .size(13.0)
            .color(theme::TEXT_MUTED),
    )
    .min_size(egui::vec2(30.0, HOOGTE))
    .corner_radius(0)
    .fill(egui::Color32::TRANSPARENT)
    .stroke(Stroke::NONE);
    ui.add(knop)
}

/// Los van `titelbalk_knop` getekend (niet via `egui::Button`): de rode hover-
/// achtergrond moet vóór het kruisje getekend worden, en `Button` tekent zijn tekst
/// al bij het aanroepen van `ui.add` — een overlay erna zou dat kruisje bedekken.
fn titelbalk_knop_sluiten(ui: &mut egui::Ui) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(30.0, HOOGTE), Sense::click());
    if ui.is_rect_visible(rect) {
        if response.hovered() {
            ui.painter().rect_filled(rect, 0.0, theme::STATUS_DND);
        }
        let kleur = if response.hovered() {
            theme::ON_ACCENT
        } else {
            theme::TEXT_MUTED
        };
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "\u{2715}",
            egui::FontId::proportional(13.0),
            kleur,
        );
    }
    response
}
