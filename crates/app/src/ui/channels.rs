//! Kanalen-weergave: kanaal-zijbalk (Algemeen/subkanalen + stem/scherm-delen) links,
//! ledenlijst rechts, chat in het midden. Wordt in deze fase nog voor beide
//! `AppView`-waarden gebruikt vanuit `mod.rs`'s `update()` — een eigen DM-weergave met
//! een DM-lijst in plaats van een ledenlijst volgt in een latere fase.

use super::widgets;
use crate::engine::UiCommand;
use eframe::egui;
use fitcom_proto::{Channel, PeerId, TopicId};

impl super::App {
    pub(super) fn channels_view(&mut self, ctx: &egui::Context) {
        self.kanaal_zijbalk(ctx);
        self.leden_zijbalk(ctx);
        self.chat_pane(ctx);
    }

    /// Kanalenlijst, eigen stem-/scherm-delen-bediening, en de eigen mini-kaart
    /// onderaan — precies wat "dit kanaal, en wat ik zelf deel" betreft. Wie er verder
    /// meedoet en wat zij delen staat in `leden_zijbalk`.
    fn kanaal_zijbalk(&mut self, ctx: &egui::Context) {
        let mut voice_cmd: Option<UiCommand> = None;
        let mut stream_cmd: Option<UiCommand> = None;
        let mut bronnen_openen = false;
        let mut kanaal_wissel: Option<Channel> = None;
        let mut niet_storen_wijziging: Option<bool> = None;
        let mut instellingen_openen = false;

        egui::SidePanel::left("kanalen")
            .resizable(false)
            .exact_width(236.0)
            .show(ctx, |ui| {
                ui.add_space(8.0);
                ui.heading("Kanalen");
                ui.add_space(8.0);

                // Bewust begrensd: zonder dit duwt een lange kanalenlijst de
                // gebruikersbalk onderaan het paneel uit (zie
                // `App::zijbalk_onderkant_hoogte`). De lijst zelf scrollt dan gewoon.
                let lijst_hoogte =
                    (ui.available_height() - self.zijbalk_onderkant_hoogte()).max(60.0);
                egui::ScrollArea::vertical()
                    .max_height(lijst_hoogte)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let algemeen_label = if self.snap.ongelezen > 0 {
                            format!("# Algemeen ({})", self.snap.ongelezen)
                        } else {
                            "# Algemeen".to_string()
                        };
                        if ui
                            .selectable_label(self.actief_kanaal.is_general(), algemeen_label)
                            .clicked()
                        {
                            kanaal_wissel = Some(Channel::GENERAL);
                        }

                        let mut topics: Vec<(TopicId, String)> = self
                            .snap
                            .timeline
                            .topics
                            .iter()
                            .map(|(id, titel)| (*id, titel.clone()))
                            .collect();
                        // Alfabetisch, met het id als tiebreaker: zonder een vaste
                        // sortering zou de volgorde per peer kunnen verschillen
                        // (`HashMap`-iteratie is niet gegarandeerd gelijk), terwijl de
                        // inhoud van `topics` bij iedereen wel identiek is.
                        topics.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));

                        for (id, titel) in &topics {
                            let ongelezen = self.snap.ongelezen_topic.get(id).copied().unwrap_or(0);
                            let label = if ongelezen > 0 {
                                format!("# {titel} ({ongelezen})")
                            } else {
                                format!("# {titel}")
                            };
                            let actief = self.actief_kanaal.topic_id() == Some(*id);
                            ui.horizontal(|ui| {
                                if ui.selectable_label(actief, label).clicked() {
                                    kanaal_wissel = Some(Channel::topic(*id));
                                }
                                if actief
                                    && ui
                                        .small_button("\u{270E}")
                                        .on_hover_text("hernoemen")
                                        .clicked()
                                {
                                    self.kanaal_hernoemen = Some((*id, titel.clone()));
                                }
                                if actief
                                    && ui
                                        .small_button("\u{1F5D1}")
                                        .on_hover_text("verwijderen")
                                        .clicked()
                                {
                                    self.bevestig_verwijder_kanaal = Some(*id);
                                }
                            });
                        }

                        let mut nieuw_kanaal_aanmaken = false;
                        let mut nieuw_kanaal_annuleren = false;
                        if let Some(concept) = &mut self.nieuw_kanaal_titel {
                            ui.horizontal(|ui| {
                                let veld = ui.add(
                                    egui::TextEdit::singleline(concept).desired_width(120.0),
                                );
                                let enter = veld.has_focus()
                                    && ui.input(|i| i.key_pressed(egui::Key::Enter));
                                if (ui.small_button("aanmaken").clicked() || enter)
                                    && !concept.trim().is_empty()
                                {
                                    nieuw_kanaal_aanmaken = true;
                                } else if ui.small_button("\u{2715}").clicked() {
                                    nieuw_kanaal_annuleren = true;
                                }
                            });
                        } else if ui.small_button("+ nieuw kanaal").clicked() {
                            self.nieuw_kanaal_titel = Some(String::new());
                        }
                        if nieuw_kanaal_aanmaken {
                            let titel = self.nieuw_kanaal_titel.take().unwrap();
                            self.stuur(UiCommand::MaakKanaal(titel.trim().to_string()));
                        } else if nieuw_kanaal_annuleren {
                            self.nieuw_kanaal_titel = None;
                        }
                    });

                ui.add_space(4.0);
                (voice_cmd, stream_cmd, bronnen_openen, niet_storen_wijziging, instellingen_openen) =
                    self.zijbalk_onderkant(ui);
            });

        self.verwerk_zijbalk_onderkant(
            voice_cmd,
            stream_cmd,
            bronnen_openen,
            niet_storen_wijziging,
            instellingen_openen,
        );
        if let Some(kanaal) = kanaal_wissel {
            self.wissel_kanaal(kanaal);
        }
    }

    /// Voice-status, scherm-delen en de gebruikersbalk onderaan de zijbalk — zelfde
    /// blok in `kanaal_zijbalk` en `dms.rs`'s `dm_zijbalk`, alleen de lijst erboven
    /// verschilt. De lijst erboven is al begrensd op `zijbalk_onderkant_hoogte`, dus dit
    /// tekent gewoon van boven naar beneden door — geen bottom-anchoring meer nodig.
    pub(super) fn zijbalk_onderkant(
        &mut self,
        ui: &mut egui::Ui,
    ) -> (
        Option<UiCommand>,
        Option<UiCommand>,
        bool,
        Option<bool>,
        bool,
    ) {
        ui.separator();
        ui.add_space(10.0);
        self.voice_sectie(ui);
        let mut voice_cmd = self.voice_bediening(ui);
        ui.add_space(10.0);
        let (stream_cmd, bronnen_openen) = self.deel_bediening(ui);
        ui.add_space(10.0);
        ui.separator();
        ui.add_space(10.0);
        let (mute_wijziging, deafen_wijziging, niet_storen_wijziging, instellingen_openen) =
            self.gebruiker_balk(ui);
        if let Some(aan) = mute_wijziging {
            voice_cmd = Some(UiCommand::Mute(aan));
        }
        if let Some(aan) = deafen_wijziging {
            voice_cmd = Some(UiCommand::Deafen(aan));
        }

        (
            voice_cmd,
            stream_cmd,
            bronnen_openen,
            niet_storen_wijziging,
            instellingen_openen,
        )
    }

    pub(super) fn verwerk_zijbalk_onderkant(
        &mut self,
        voice_cmd: Option<UiCommand>,
        stream_cmd: Option<UiCommand>,
        bronnen_openen: bool,
        niet_storen_wijziging: Option<bool>,
        instellingen_openen: bool,
    ) {
        if let Some(cmd) = voice_cmd {
            self.stuur(cmd);
        }
        if let Some(cmd) = stream_cmd {
            self.stuur(cmd);
        }
        if bronnen_openen {
            self.open_bronkeuze();
        }
        if let Some(aan) = niet_storen_wijziging {
            self.stuur(UiCommand::NietStoren(aan));
        }
        if instellingen_openen {
            self.open_instellingen();
            self.view = super::AppView::Settings;
        }
    }

    /// Wie er meedoet: status, of ze in het gesprek zitten, en wat ze delen. Losstaand
    /// van `kanaal_zijbalk` zodat de kanalenlijst en de ledenlijst allebei hun eigen
    /// volledige hoogte houden in plaats van in één paneel te moeten concurreren om
    /// ruimte.
    fn leden_zijbalk(&mut self, ctx: &egui::Context) {
        let mut volume_wijziging: Option<(PeerId, f32)> = None;

        egui::SidePanel::right("leden")
            .resizable(false)
            .exact_width(216.0)
            .show(ctx, |ui| {
                ui.add_space(8.0);
                ui.heading("Leden");
                ui.add_space(8.0);

                if self.snap.peers.is_empty() {
                    ui.weak("Nog geen peers ingesteld.");
                    ui.small("Zet de tailnet-adressen van de anderen in config.toml.");
                }

                let in_gesprek = self.snap.voice.actief;
                for p in &self.snap.peers {
                    let naam = p
                        .peer_id
                        .and_then(|id| self.snap.timeline.nicknames.get(&id))
                        .cloned()
                        .unwrap_or_else(|| p.label.clone());
                    let scherm_live = p.peer_id.is_some_and(|id| {
                        self.snap
                            .streams
                            .iter()
                            .any(|s| s.eigenaar == id && !s.is_geluid)
                    });
                    widgets::peer_row(ui, p, &naam, scherm_live);

                    if in_gesprek && p.in_voice {
                        widgets::niveaubalk(ui, p.niveau);
                        if let Some(id) = p.peer_id {
                            let mut vol = p.volume;
                            if ui
                                .add(
                                    egui::Slider::new(&mut vol, 0.0..=2.0)
                                        .show_value(false)
                                        .text("volume"),
                                )
                                .changed()
                            {
                                volume_wijziging = Some((id, vol));
                            }
                        }
                    }

                    ui.add_space(6.0);
                }
            });

        if let Some((id, vol)) = volume_wijziging {
            self.stuur(UiCommand::Volume(id, vol));
        }
    }
}
