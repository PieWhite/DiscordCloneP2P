//! Statusbalk onderaan: id/poort/berichtenteller, snelkoppeling naar de datamap, en een
//! wegklikbare foutmelding. De "instellingen"-knop die hier ooit stond is vervallen —
//! dat gaat nu via het tandwiel in de icoonrail (`ui/rail.rs`).

use super::theme;
use crate::engine::UiCommand;
use eframe::egui;

impl super::App {
    pub(super) fn statusbalk(&mut self, ctx: &egui::Context) {
        let mut fout_weg = false;

        egui::TopBottomPanel::bottom("statusbalk").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.small(format!("id {}", &self.mij.to_string()[..8]));
                ui.separator();
                ui.small(format!("poort {}", self.control_port));
                ui.separator();
                ui.small(format!("{} berichten", self.snap.timeline.messages.len()));
                ui.separator();
                if ui
                    .small_button("map openen")
                    .on_hover_text(self.data_dir.display().to_string())
                    .clicked()
                {
                    let _ = std::process::Command::new("explorer")
                        .arg(&self.data_dir)
                        .spawn();
                }
                if let Some(err) = &self.snap.fout {
                    ui.separator();
                    if ui
                        .add(egui::Label::new(
                            egui::RichText::new(format!("⚠ {err}")).color(theme::STATUS_DND),
                        ))
                        .on_hover_text("klik om te verbergen")
                        .clicked()
                    {
                        fout_weg = true;
                    }
                }
            });
        });

        if fout_weg {
            self.stuur(UiCommand::FoutWeg);
        }
    }
}
