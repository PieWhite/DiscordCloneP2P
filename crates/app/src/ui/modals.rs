//! Kleine, kortstondige modale vensters: bronkeuze voor scherm delen, subkanaal
//! hernoemen/verwijderen bevestigen, alle afbeeldingen verwijderen bevestigen, en de
//! automatische-update-melding. Logica ongewijzigd — enkel hierheen verhuisd; de stijl
//! komt gratis mee uit `theme::apply` (`egui::Window` volgt de globale `Visuals`).

use super::theme;
use crate::engine::UiCommand;
use eframe::egui;
use fitcom_proto::{Channel, PeerId};
use fitcom_video::{Bron, BronSoort};

impl super::App {
    pub(super) fn bronkeuze_venster(&mut self, ctx: &egui::Context) {
        let Some(bronnen) = self.bronkeuze.clone() else {
            return;
        };
        let mut open = true;
        let mut gekozen: Option<Bron> = None;

        egui::Window::new("Wat wil je delen?")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(420.0)
            .show(ctx, |ui| {
                if bronnen.is_empty() {
                    ui.label("Geen bronnen gevonden.");
                    return;
                }
                ui.small("Er wordt pas opgenomen zodra iemand daadwerkelijk kijkt.");
                ui.add_space(6.0);

                egui::ScrollArea::vertical()
                    .max_height(360.0)
                    .show(ui, |ui| {
                        for soort in [BronSoort::Monitor, BronSoort::Venster] {
                            let lijst: Vec<&Bron> =
                                bronnen.iter().filter(|b| b.soort == soort).collect();
                            if lijst.is_empty() {
                                continue;
                            }
                            ui.label(egui::RichText::new(match soort {
                                BronSoort::Monitor => "Schermen",
                                BronSoort::Venster => "Vensters",
                            }))
                            .highlight();
                            for b in lijst {
                                if ui
                                    .add_sized(
                                        [ui.available_width(), 24.0],
                                        egui::Button::new(&b.naam),
                                    )
                                    .clicked()
                                {
                                    gekozen = Some(b.clone());
                                }
                            }
                            ui.add_space(8.0);
                        }
                    });
            });

        if let Some(bron) = gekozen {
            self.stuur(UiCommand::DeelBron(bron));
            self.bronkeuze = None;
        } else if !open {
            self.bronkeuze = None;
        }
    }

    /// Bevestigingsvraag vóór "Verwijder alle afbeeldingen" — een onomkeerbare
    /// schijfoperatie verdient een expliciete stap ertussen. Raakt alleen lokale
    /// schijfruimte: de berichten/kaarten blijven in de tijdlijn staan (zie
    /// `engine.rs::verwijder_alle_afbeeldingen`).
    pub(super) fn bevestig_verwijder_afbeeldingen_venster(&mut self, ctx: &egui::Context) {
        if !self.bevestig_verwijder_afbeeldingen {
            return;
        }
        let mut open = true;
        let mut bevestigd = false;
        let mut geannuleerd = false;

        egui::Window::new("Alle afbeeldingen verwijderen?")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(320.0)
            .show(ctx, |ui| {
                ui.label(
                    "Dit verwijdert alle gedeelde en gedownloade afbeeldingen van je eigen \
                     schijf. De berichten blijven staan; vraagt iemand een afbeelding later \
                     opnieuw op, dan krijgt hij netjes te horen dat hij niet meer \
                     beschikbaar is.",
                );
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Ja, verwijderen").clicked() {
                        bevestigd = true;
                    }
                    if ui.button("Annuleren").clicked() {
                        geannuleerd = true;
                    }
                });
            });

        if bevestigd {
            self.stuur(UiCommand::VerwijderAlleAfbeeldingen);
            // Bewust géén self.bijlage_texturen.clear() hier: een al geladen miniatuur
            // blijft dan zichtbaar tot de volgende herstart, ook al zijn de bytes net
            // van schijf verwijderd. Dat is prima zo — Rick wil dat expliciet zo houden.
        }
        if bevestigd || geannuleerd || !open {
            self.bevestig_verwijder_afbeeldingen = false;
        }
    }

    /// Titel van een bestaand subkanaal wijzigen. Zelfde mechanisme als aanmaken —
    /// zie `Chat::zet_kanaal_titel` — dus dit venster stuurt gewoon een nieuwe
    /// `HernoemKanaal` met hetzelfde id.
    pub(super) fn kanaal_hernoemen_venster(&mut self, ctx: &egui::Context) {
        let Some((_id, concept)) = &mut self.kanaal_hernoemen else {
            return;
        };
        let mut open = true;
        let mut opslaan = false;
        let mut annuleren = false;

        egui::Window::new("Subkanaal hernoemen")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(300.0)
            .show(ctx, |ui| {
                ui.label(egui::RichText::new("Titel").strong());
                ui.add_space(6.0);
                let veld = ui.add(egui::TextEdit::singleline(concept).desired_width(f32::INFINITY));
                let enter = veld.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.add_space(10.0);

                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!concept.trim().is_empty(), egui::Button::new("Opslaan"))
                        .clicked()
                        || (enter && !concept.trim().is_empty())
                    {
                        opslaan = true;
                    }
                    if ui.button("Annuleren").clicked() {
                        annuleren = true;
                    }
                });
            });

        if opslaan {
            let (id, titel) = self.kanaal_hernoemen.take().unwrap();
            self.stuur(UiCommand::HernoemKanaal(id, titel));
        } else if annuleren || !open {
            self.kanaal_hernoemen = None;
        }
    }

    /// Bevestigingsvraag vóór een subkanaal echt verwijderd wordt — onomkeerbaar voor
    /// iedereen, dus geen knop-per-ongeluk.
    pub(super) fn bevestig_verwijder_kanaal_venster(&mut self, ctx: &egui::Context) {
        let Some(id) = self.bevestig_verwijder_kanaal else {
            return;
        };
        let titel = self
            .snap
            .timeline
            .topics
            .get(&id)
            .cloned()
            .unwrap_or_else(|| "dit subkanaal".to_string());

        let mut open = true;
        let mut bevestigd = false;
        let mut geannuleerd = false;

        egui::Window::new("Subkanaal verwijderen?")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(320.0)
            .show(ctx, |ui| {
                ui.label(format!(
                    "Weet je zeker dat je \"{titel}\" wilt verwijderen? Dit gebeurt bij \
                     iedereen."
                ));
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Ja, verwijderen").clicked() {
                        bevestigd = true;
                    }
                    if ui.button("Annuleren").clicked() {
                        geannuleerd = true;
                    }
                });
            });

        if bevestigd {
            self.stuur(UiCommand::VerwijderKanaal(id));
            if self.actief_kanaal.topic_id() == Some(id) {
                self.wissel_kanaal(Channel::GENERAL);
            }
            self.bevestig_verwijder_kanaal = None;
        } else if geannuleerd || !open {
            self.bevestig_verwijder_kanaal = None;
        }
    }

    /// Fase 11: toont dat een peer een nieuwere versie draait, de voortgang van het
    /// automatisch ophalen, en pas een "nu bijwerken en herstarten"-knop zodra de
    /// download geverifieerd binnen is. Geen apart open/dicht-veld op `App` nodig zoals
    /// bij de andere bevestigingsvensters hierboven: de motor zelf is hier de bron van
    /// waarheid (`Snapshot::update`), dus dit venster verschijnt en verdwijnt vanzelf
    /// mee met die status.
    pub(super) fn update_beschikbaar_venster(&mut self, ctx: &egui::Context) {
        use crate::updates::UpdateStatus;

        let Some(status) = self.snap.update.clone() else {
            return;
        };

        let peer_label = |id: PeerId| -> String {
            self.snap
                .peers
                .iter()
                .find(|p| p.peer_id == Some(id))
                .map(|p| p.label.clone())
                .unwrap_or_else(|| "een peer".to_string())
        };

        let mut open = true;
        let mut toepassen = false;
        let mut wegklikken = false;

        egui::Window::new("Nieuwere versie beschikbaar")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                match &status {
                    UpdateStatus::Aangeboden { peer, hun_versie } => {
                        ui.label(format!(
                            "{} heeft versie {hun_versie}. Ophalen wordt gestart...",
                            peer_label(*peer)
                        ));
                    }
                    UpdateStatus::Bezig {
                        peer,
                        hun_versie,
                        ontvangen,
                        totaal,
                        ..
                    } => {
                        ui.label(format!(
                            "Versie {hun_versie} ophalen bij {}...",
                            peer_label(*peer)
                        ));
                        let fractie = if *totaal > 0 {
                            *ontvangen as f32 / *totaal as f32
                        } else {
                            0.0
                        };
                        ui.add(egui::ProgressBar::new(fractie).text(format!(
                            "{} / {}",
                            super::grootte_tekst(*ontvangen),
                            super::grootte_tekst(*totaal)
                        )));
                    }
                    UpdateStatus::KlaarOmToeTePassen {
                        peer, hun_versie, ..
                    } => {
                        ui.label(format!(
                            "{} heeft versie {hun_versie}. Nu bijwerken en herstarten?",
                            peer_label(*peer)
                        ));
                    }
                    UpdateStatus::Mislukt(bericht) => {
                        ui.colored_label(theme::STATUS_DND, bericht);
                    }
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if matches!(status, UpdateStatus::KlaarOmToeTePassen { .. })
                        && ui.button("Nu bijwerken en herstarten").clicked()
                    {
                        toepassen = true;
                    }
                    let knoptekst = if matches!(status, UpdateStatus::Mislukt(_)) {
                        "OK"
                    } else {
                        "Negeren"
                    };
                    if ui.button(knoptekst).clicked() {
                        wegklikken = true;
                    }
                });
            });

        if toepassen {
            self.stuur(UiCommand::PasUpdateToe);
        } else if wegklikken || !open {
            match &status {
                UpdateStatus::Mislukt(_) => self.stuur(UiCommand::WisUpdateMelding),
                other => {
                    if let Some(versie) = other.hun_versie() {
                        self.stuur(UiCommand::NegeerUpdate(versie.to_string()));
                    }
                }
            }
        }
    }
}
