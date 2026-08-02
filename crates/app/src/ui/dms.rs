//! DM-weergave: een lijst van peers om direct mee te chatten, links, en het gekozen
//! gesprek in het midden. Voice/scherm-delen zijn app-breed (niet per gesprek), dus die
//! bediening staat hier — net als in `ui/channels.rs` — onderaan de zijbalk; er is geen
//! ledenlijst zoals bij Kanalen, want in een DM is de "peer" al de titel van het gesprek.

use super::{theme, widgets};
use eframe::egui;
use fitcom_proto::{Channel, PeerId};

impl super::App {
    pub(super) fn dms_view(&mut self, ctx: &egui::Context) {
        self.dm_zijbalk(ctx);
        if self.actief_kanaal.dm_peer().is_some() {
            self.chat_pane(ctx);
        } else {
            self.geen_dm_gekozen(ctx);
        }
    }

    fn dm_zijbalk(&mut self, ctx: &egui::Context) {
        let mut voice_cmd = None;
        let mut stream_cmd = None;
        let mut bronnen_openen = false;
        let mut naar_dm: Option<PeerId> = None;
        let mut niet_storen_wijziging: Option<bool> = None;

        egui::SidePanel::left("dms")
            .resizable(false)
            .exact_width(236.0)
            .show(ctx, |ui| {
                ui.add_space(8.0);
                ui.heading("Directe berichten");
                ui.add_space(8.0);

                if self.snap.peers.is_empty() {
                    ui.weak("Nog geen peers ingesteld.");
                    ui.small("Zet de tailnet-adressen van de anderen in config.toml.");
                }

                for p in &self.snap.peers {
                    let Some(id) = p.peer_id else { continue };
                    let naam = self
                        .snap
                        .timeline
                        .nicknames
                        .get(&id)
                        .cloned()
                        .unwrap_or_else(|| p.label.clone());
                    let ongelezen = self.snap.ongelezen_dm.get(&id).copied().unwrap_or(0);
                    let label = if ongelezen > 0 {
                        format!("{naam} ({ongelezen})")
                    } else {
                        naam.clone()
                    };
                    let actief = self.actief_kanaal.dm_peer() == Some(id);

                    ui.horizontal(|ui| {
                        let avatar_kleur = widgets::kleur_van(id);
                        let avatar = widgets::avatar_square(
                            ui,
                            &widgets::initialen(&naam),
                            avatar_kleur,
                            28.0,
                        );
                        let (status_kleur, _) = widgets::describe(&p.status);
                        widgets::status_badge(
                            ui.painter(),
                            avatar.rect,
                            status_kleur,
                            theme::BG_SIDEBAR,
                        );
                        if ui.selectable_label(actief, label).clicked() {
                            naar_dm = Some(id);
                        }
                    });
                    ui.add_space(4.0);
                }

                ui.add_space(8.0);
                (voice_cmd, stream_cmd, bronnen_openen, niet_storen_wijziging) =
                    self.zijbalk_onderkant(ui);
            });

        self.verwerk_zijbalk_onderkant(
            voice_cmd,
            stream_cmd,
            bronnen_openen,
            niet_storen_wijziging,
        );
        if let Some(id) = naar_dm {
            self.wissel_kanaal(Channel::dm(id));
        }
    }

    /// Nog geen gesprek gekozen: geen kanaal om te tonen, dus geen `chat_pane` (die zou
    /// zonder gekozen DM de inhoud van een willekeurig ander kanaal tonen). Zelfde
    /// toon als de bestaande lege-staat-teksten elders in de app.
    fn geen_dm_gekozen(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.weak("Kies een gesprek.");
                ui.small("Klik links op een peer om een DM te openen.");
            });
        });
    }
}
