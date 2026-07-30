//! 60px icoonrail langs de linkerkant: DM-weergave, kanaal-weergave, en onderaan het
//! instellingen-tandwiel — zoals de mockup. Instellingen is in deze fase nog het
//! bestaande modale venster (zie `App::open_instellingen`); een volle Instellingen-
//! weergave komt in een latere fase.

use super::{theme, widgets, App, AppView};
use eframe::egui;

const BREEDTE: f32 = 60.0;

pub fn rail(app: &mut App, ctx: &egui::Context) {
    let mut nieuwe_view: Option<AppView> = None;
    let mut instellingen_openen = false;

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
                if widgets::rail_button(ui, app.view == AppView::Dms, "DM")
                    .on_hover_text("Directe berichten")
                    .clicked()
                {
                    nieuwe_view = Some(AppView::Dms);
                }
                ui.add_space(8.0);
                let (rect, _) = ui.allocate_exact_size(egui::vec2(28.0, 1.0), egui::Sense::hover());
                ui.painter()
                    .hline(rect.x_range(), rect.center().y, theme::BORDER_STROKE);
                ui.add_space(8.0);
                if widgets::rail_button(ui, app.view == AppView::Channels, "F")
                    .on_hover_text("FitCommunication")
                    .clicked()
                {
                    nieuwe_view = Some(AppView::Channels);
                }
            });

            ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                ui.add_space(10.0);
                if widgets::rail_button(ui, false, "\u{2699}")
                    .on_hover_text("Instellingen")
                    .clicked()
                {
                    instellingen_openen = true;
                }
            });
        });

    if let Some(view) = nieuwe_view {
        app.wissel_view(view);
    }
    if instellingen_openen {
        app.open_instellingen();
    }
}
