//! 60px icoonrail langs de linkerkant, zoals Discord's serverlijst: ronde iconen met
//! een accentstreep links van de actieve knop. Alleen DM- en kanaal-weergave — het
//! instellingen-tandwiel woont sinds de Discord-indeling in de gebruikersbalk onderaan
//! de zijbalk (zie `App::gebruiker_balk` in `mod.rs`), niet meer hier.

use super::{theme, widgets, App, AppView};
use eframe::egui;

const BREEDTE: f32 = 60.0;

pub fn rail(app: &mut App, ctx: &egui::Context) {
    let mut nieuwe_view: Option<AppView> = None;

    egui::SidePanel::left("icoonrail")
        .resizable(false)
        .exact_width(BREEDTE)
        .frame(
            egui::Frame::new()
                .fill(theme::BG_DEEPEST)
                .inner_margin(egui::Margin::symmetric(0, 10)),
        )
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                let dm = app.view == AppView::Dms;
                let resp = widgets::rail_button(ui, dm, "DM").on_hover_text("Directe berichten");
                if dm {
                    teken_indicator(ui, resp.rect);
                }
                if resp.clicked() {
                    nieuwe_view = Some(AppView::Dms);
                }
                ui.add_space(8.0);
                let (rect, _) = ui.allocate_exact_size(egui::vec2(28.0, 1.0), egui::Sense::hover());
                ui.painter()
                    .hline(rect.x_range(), rect.center().y, theme::BORDER_STROKE);
                ui.add_space(8.0);
                let kanalen = app.view == AppView::Channels || app.view == AppView::Settings;
                let resp =
                    widgets::rail_button(ui, kanalen, "F").on_hover_text("FitCommunication");
                if kanalen {
                    teken_indicator(ui, resp.rect);
                }
                if resp.clicked() {
                    nieuwe_view = Some(AppView::Channels);
                }
            });
        });

    if let Some(view) = nieuwe_view {
        app.wissel_view(view);
    }
}

/// De witte/accentstreep links van een geselecteerde rail-knop — Discord's manier om
/// te tonen welk server-icoon actief is. Relatief aan de knop-rect getekend, dus
/// onafhankelijk van waar het paneel zelf op het scherm staat.
fn teken_indicator(ui: &egui::Ui, knop_rect: egui::Rect) {
    let streep = egui::Rect::from_center_size(
        egui::pos2(knop_rect.left() - 5.0, knop_rect.center().y),
        egui::vec2(4.0, 22.0),
    );
    ui.painter()
        .rect_filled(streep, egui::CornerRadius::same(2), theme::ACCENT);
}
