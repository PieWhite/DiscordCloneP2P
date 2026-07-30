//! Kanalen-weergave: kanaal-zijbalk (Algemeen/subkanalen + stem/scherm-delen) links,
//! ledenlijst rechts, chat in het midden. Wordt in deze fase nog voor beide
//! `AppView`-waarden gebruikt vanuit `mod.rs`'s `update()` — een eigen DM-weergave met
//! een DM-lijst in plaats van een ledenlijst volgt in een latere fase.

use super::{widgets, AppView};
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

        egui::SidePanel::left("kanalen")
            .resizable(false)
            .exact_width(236.0)
            .show(ctx, |ui| {
                ui.add_space(8.0);
                ui.heading("Kanalen");
                ui.add_space(8.0);

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
                // Alfabetisch, met het id als tiebreaker: zonder een vaste sortering
                // zou de volgorde per peer kunnen verschillen (`HashMap`-iteratie is
                // niet gegarandeerd gelijk), terwijl de inhoud van `topics` bij
                // iedereen wel identiek is.
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
                        let veld = ui.add(egui::TextEdit::singleline(concept).desired_width(120.0));
                        let enter =
                            veld.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
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

                ui.add_space(4.0);
                ui.separator();
                ui.add_space(8.0);
                voice_cmd = self.voice_bediening(ui);

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(6.0);
                let (cmd, openen) = self.deel_bediening(ui);
                stream_cmd = cmd;
                bronnen_openen = openen;

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(6.0);
                if let Some(aan) = self.eigen_mini_kaart(ui) {
                    niet_storen_wijziging = Some(aan);
                }
            });

        if let Some(cmd) = voice_cmd {
            self.stuur(cmd);
        }
        if let Some(cmd) = stream_cmd {
            self.stuur(cmd);
        }
        if bronnen_openen {
            self.open_bronkeuze();
        }
        if let Some(kanaal) = kanaal_wissel {
            self.wissel_kanaal(kanaal);
        }
        if let Some(aan) = niet_storen_wijziging {
            self.stuur(UiCommand::NietStoren(aan));
        }
    }

    /// Wie er meedoet: status, of ze in het gesprek zitten, en wat ze delen. Losstaand
    /// van `kanaal_zijbalk` zodat de kanalenlijst en de ledenlijst allebei hun eigen
    /// volledige hoogte houden in plaats van in één paneel te moeten concurreren om
    /// ruimte.
    fn leden_zijbalk(&mut self, ctx: &egui::Context) {
        let mut volume_wijziging: Option<(PeerId, f32)> = None;
        let mut stream_cmd: Option<UiCommand> = None;
        let mut naar_dm: Option<PeerId> = None;

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
                    widgets::peer_row(ui, p, &naam);

                    if let Some(id) = p.peer_id {
                        let ongelezen = self.snap.ongelezen_dm.get(&id).copied().unwrap_or(0);
                        let label = if ongelezen > 0 {
                            format!("\u{1F4AC} DM ({ongelezen})")
                        } else {
                            "\u{1F4AC} DM".to_string()
                        };
                        if ui
                            .selectable_label(self.actief_kanaal.dm_peer() == Some(id), label)
                            .clicked()
                        {
                            naar_dm = Some(id);
                        }
                    }

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

                    // Wat deze peer deelt, direct onder zijn naam: daar zoek je het.
                    if let Some(id) = p.peer_id {
                        for s in self.snap.streams.iter().filter(|s| s.eigenaar == id) {
                            ui.horizontal(|ui| {
                                ui.small(if s.is_geluid {
                                    "\u{1F50A}"
                                } else {
                                    "\u{1F5B5}"
                                });
                                let label = ui.small(&s.titel);
                                if !s.is_geluid {
                                    label.on_hover_text(format!("{}×{}", s.breedte, s.hoogte));
                                }
                            });

                            let knop = match (s.is_geluid, s.kijken) {
                                (true, true) => "niet meer luisteren",
                                (true, false) => "meeluisteren",
                                (false, true) => "sluiten",
                                (false, false) => "bekijken",
                            };
                            if ui.small_button(knop).clicked() {
                                stream_cmd = Some(if s.kijken {
                                    UiCommand::StopKijken(id, s.stream_id)
                                } else {
                                    UiCommand::Kijken(id, s.stream_id)
                                });
                            }

                            // Meegedeeld geluid staat los van de stem: je wilt zijn
                            // spel zachter kunnen zetten zonder hem te dempen.
                            if s.is_geluid && s.kijken {
                                let mut vol = s.volume;
                                if ui
                                    .add(
                                        egui::Slider::new(&mut vol, 0.0..=2.0)
                                            .show_value(false)
                                            .text("geluid"),
                                    )
                                    .changed()
                                {
                                    stream_cmd =
                                        Some(UiCommand::StreamVolume(id, s.stream_id, vol));
                                }
                            }
                        }
                    }
                    ui.add_space(6.0);
                }
            });

        if let Some((id, vol)) = volume_wijziging {
            self.stuur(UiCommand::Volume(id, vol));
        }
        if let Some(cmd) = stream_cmd {
            self.stuur(cmd);
        }
        if let Some(id) = naar_dm {
            // Rechtstreeks het gekozen gesprek zetten in plaats van via `wissel_view`
            // (die het *laatst* geopende gesprek zou herstellen — hier weten we al
            // precies welke DM het moet worden).
            self.wissel_kanaal(Channel::dm(id));
            self.view = AppView::Dms;
        }
    }
}
