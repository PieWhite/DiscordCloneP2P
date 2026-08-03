//! Instellingenweergave: een volledig scherm in plaats van een modal venster, met een
//! tab-rail zoals de mockup. Eigen tabnamen in plaats van de mockup's letterlijke
//! "My Account"/"Appearance"/"Notifications"/"About": Appearance (accentkleur,
//! compact-modus) en Notifications horen bij functionaliteit die wij niet hebben, dus
//! die tabs worden niet gebouwd — Video en Opslag zijn wat wij daadwerkelijk hebben.

use super::{theme, widgets};
use crate::config::VideoConfig;
use crate::engine::UiCommand;
use eframe::egui;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsTab {
    Account,
    Geluid,
    Video,
    Opslag,
    Over,
}

impl super::App {
    pub(super) fn settings_view(&mut self, ctx: &egui::Context) {
        let mut gekozen_tab = self.settings_tab;

        egui::SidePanel::left("instellingen_tabs")
            .resizable(false)
            .exact_width(200.0)
            .show(ctx, |ui| {
                ui.add_space(8.0);
                widgets::section_label(ui, "Instellingen");
                ui.add_space(8.0);
                for (tab, label) in [
                    (SettingsTab::Account, "Account"),
                    (SettingsTab::Geluid, "Geluid"),
                    (SettingsTab::Video, "Video"),
                    (SettingsTab::Opslag, "Opslag"),
                    (SettingsTab::Over, "Over"),
                ] {
                    if widgets::pill_tab(ui, self.settings_tab == tab, label).clicked() {
                        gekozen_tab = tab;
                    }
                    ui.add_space(2.0);
                }
            });
        self.settings_tab = gekozen_tab;

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(20.0);
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(20, 0))
                .show(ui, |ui| match self.settings_tab {
                    SettingsTab::Account => self.instellingen_account(ui),
                    SettingsTab::Geluid => self.instellingen_geluid(ui),
                    SettingsTab::Video => self.instellingen_video(ui),
                    SettingsTab::Opslag => self.instellingen_opslag(ui),
                    SettingsTab::Over => self.instellingen_over(ui),
                });
        });
    }

    /// Eigen naam wijzigen (bewerkt `self.profiel` als kopie, zoals voorheen de aparte
    /// `profiel_venster`-modal deed) en de "niet storen"-toggle.
    fn instellingen_account(&mut self, ui: &mut egui::Ui) {
        ui.heading("Account");
        ui.add_space(16.0);

        let eigen = self
            .snap
            .timeline
            .nicknames
            .get(&self.mij)
            .cloned()
            .unwrap_or_else(|| self.eigen_naam.clone());
        let eigen_kleur = widgets::kleur_van(self.mij);
        let status_kleur = if self.snap.niet_storen {
            theme::STATUS_DND
        } else {
            theme::STATUS_ONLINE
        };

        ui.horizontal(|ui| {
            let avatar = widgets::avatar_square(ui, &widgets::initialen(&eigen), eigen_kleur, 64.0);
            widgets::status_badge(ui.painter(), avatar.rect, status_kleur, theme::BG_CANVAS);
            ui.add_space(12.0);
            ui.vertical(|ui| {
                ui.label(egui::RichText::new(&eigen).size(18.0).strong());
                ui.small(
                    egui::RichText::new(if self.snap.niet_storen {
                        "Niet storen"
                    } else {
                        "Online"
                    })
                    .color(status_kleur),
                );
            });
        });
        ui.add_space(20.0);

        let mut opslaan = false;
        let mut annuleren = false;
        if let Some(concept) = &mut self.profiel {
            ui.label(egui::RichText::new("Weergavenaam").strong());
            ui.small("Zichtbaar voor de andere peers, overal waar jouw naam getoond wordt.");
            ui.add_space(6.0);
            let veld = ui.add(egui::TextEdit::singleline(concept).desired_width(300.0));
            let enter = veld.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.add_space(8.0);
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
        } else if ui.button("Naam wijzigen").clicked() {
            self.profiel = Some(eigen.clone());
        }
        if opslaan {
            let naam = self.profiel.take().unwrap();
            self.stuur(UiCommand::ZetNaam(naam));
        } else if annuleren {
            self.profiel = None;
        }

        ui.add_space(24.0);
        widgets::section_label(ui, "Status");
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.small("Niet storen");
            let mut niet_storen = self.snap.niet_storen;
            if widgets::toggle_switch(ui, &mut niet_storen).changed() {
                self.stuur(UiCommand::NietStoren(niet_storen));
            }
        });
    }

    /// Microfoon- en weergavekeuze, plus een meter op je eigen microfoon.
    ///
    /// De meter is het punt van dit scherm: hij toont wat er daadwerkelijk verstuurd
    /// wordt. Slaat hij uit terwijl je niets zegt, dan neemt je "microfoon" het geluid
    /// van deze pc op — een loopback-apparaat (Stereo Mix, VB-Cable, Voicemeeter) of
    /// speakers die je mic weer inpikt. Er zit geen echo-onderdrukking in de keten, dus
    /// dat gaat één op één naar de anderen toe.
    fn instellingen_geluid(&mut self, ui: &mut egui::Ui) {
        ui.heading("Geluid");
        ui.add_space(6.0);

        if self.audio_apparaten.is_none() {
            match crate::engine::audio_apparaten() {
                Ok(lijsten) => self.audio_apparaten = Some(lijsten),
                Err(e) => {
                    ui.colored_label(theme::STATUS_DND, format!("apparaten opvragen: {e:#}"));
                    return;
                }
            }
        }
        let (invoer, uitvoer) = self.audio_apparaten.clone().unwrap_or_default();

        let mut keuze_in = self.snap.input_device.clone();
        let mut keuze_uit = self.snap.output_device.clone();

        ui.label(egui::RichText::new("Microfoon").strong());
        apparaat_keuze(ui, "invoerapparaat", &invoer, &mut keuze_in);
        ui.add_space(10.0);

        ui.label(egui::RichText::new("Weergave").strong());
        apparaat_keuze(ui, "uitvoerapparaat", &uitvoer, &mut keuze_uit);
        ui.small("Ook waar het gedeelde bureaubladgeluid vanaf getapt wordt.");
        ui.add_space(4.0);

        if ui.button("Lijst vernieuwen").clicked() {
            self.audio_apparaten = None;
        }

        if keuze_in != self.snap.input_device || keuze_uit != self.snap.output_device {
            self.stuur(UiCommand::ZetGeluidsapparaten(keuze_in, keuze_uit));
        }

        ui.add_space(20.0);
        widgets::section_label(ui, "Microfoontest");
        ui.add_space(8.0);
        if self.snap.voice.actief {
            widgets::niveaubalk(ui, self.snap.voice.eigen_niveau);
            ui.add_space(4.0);
            ui.small(
                "De balk beweegt alleen als er ook daadwerkelijk iets naar de anderen \
                 gaat. Slaat hij uit terwijl jij niets zegt — bij muziek, een spel of \
                 als iemand anders praat — dan neemt dit apparaat het geluid van je pc \
                 op in plaats van alleen jou. Kies dan een ander apparaat of gebruik een \
                 koptelefoon; er zit geen echo-onderdrukking in.",
            );
        } else {
            ui.small("Neem deel aan het gesprek om je microfoon te testen.");
        }
    }

    /// Codec/fps/bitrate. Bewerkt een kopie (`self.instellingen`, gevuld door
    /// `App::open_instellingen` bij het openen van deze weergave) zodat weg-navigeren
    /// zonder "Toepassen" niets verandert — de volgende keer dat je hier komt, vult
    /// `open_instellingen` een verse kopie uit de motor.
    fn instellingen_video(&mut self, ui: &mut egui::Ui) {
        let mut toepassen = false;
        let mut annuleren = false;

        if let Some(concept) = &mut self.instellingen {
            ui.heading("Video");
            ui.add_space(6.0);
            ui.label(egui::RichText::new("Codec").strong());
            ui.horizontal(|ui| {
                ui.selectable_value(&mut concept.codec, "h264".to_string(), "H.264");
                ui.selectable_value(&mut concept.codec, "hevc".to_string(), "HEVC");
            });
            if concept.codec == "hevc" {
                ui.small(
                    "HEVC decoderen loopt op Windows via een Store-uitbreiding die er niet \
                     standaard op zit. Zet dit alleen aan als je zeker weet dat alle peers \
                     hem kunnen decoderen.",
                );
            } else {
                ui.small("Aanbevolen: zit altijd in Windows, bij iedereen.");
            }
            ui.add_space(10.0);

            ui.label(egui::RichText::new("Beelden per seconde").strong());
            ui.add(egui::Slider::new(&mut concept.fps, 15..=240));
            ui.small(
                "Zet dit gelijk aan of hoger dan het tempo van wat je deelt: een filmpje \
                 van 60 bemonster je met 60 of meer, anders mis je beelden. Hoger kost \
                 evenredig meer bitrate.",
            );
            ui.add_space(10.0);

            ui.label(egui::RichText::new("Bitrate").strong());
            ui.add(
                egui::Slider::new(&mut concept.bitrate_mbit, 2.0..=50.0)
                    .suffix(" Mbit/s")
                    .fixed_decimals(0),
            );
            ui.small("Op een gigabitnetwerk zijn bits gratis; hoger geeft scherpere tekst.");
            ui.add_space(14.0);

            ui.horizontal(|ui| {
                if ui.button("Toepassen").clicked() {
                    toepassen = true;
                }
                if ui.button("Annuleren").clicked() {
                    annuleren = true;
                }
            });
            ui.add_space(4.0);
            ui.small("Geldt voor lopende en nieuw gestarte deelsessies.");
        } else {
            // Zou niet moeten voorkomen: `open_instellingen` vult dit altijd bij het
            // binnengaan van deze weergave. Voor de zekerheid geen paniek, gewoon
            // opnieuw vullen.
            self.open_instellingen();
        }

        if toepassen {
            let concept = self.instellingen.take().unwrap();
            self.stuur(UiCommand::ZetVideoInstellingen(VideoConfig {
                codec: concept.codec,
                fps: concept.fps,
                bitrate: (concept.bitrate_mbit * 1_000_000.0).round() as u32,
            }));
            self.open_instellingen();
        } else if annuleren {
            self.open_instellingen();
        }
    }

    fn instellingen_opslag(&mut self, ui: &mut egui::Ui) {
        ui.heading("Opslag");
        ui.add_space(6.0);
        ui.small(
            "Afbeeldingen die je zelf deelt of downloadt staan apart van je gewone \
             downloads, zodat ze inline in de chat getoond kunnen worden.",
        );
        ui.add_space(10.0);
        if ui.button("Verwijder alle afbeeldingen").clicked() {
            self.bevestig_verwijder_afbeeldingen = true;
        }
    }

    fn instellingen_over(&mut self, ui: &mut egui::Ui) {
        ui.heading("Over");
        ui.add_space(10.0);
        ui.label(format!("FitCommunication {}", env!("CARGO_PKG_VERSION")));
        ui.small(format!("protocol {}", fitcom_proto::PROTOCOL_VERSION));
        ui.add_space(6.0);
        ui.small(format!("id {}", &self.mij.to_string()[..8]));
        ui.small(format!("poort {}", self.control_port));
    }
}

/// Uitklaplijst met apparaten, met "Standaardapparaat" (`None`) als eerste keuze.
///
/// `None` betekent hetzelfde als in de config: laat Windows kiezen. Staat een eerder
/// gekozen apparaat er niet meer tussen (hernoemd, losgekoppeld), dan komt hij als
/// gemarkeerde regel onderaan te staan — anders zou de lijst een keuze tonen die
/// stilzwijgend niet gebruikt wordt.
fn apparaat_keuze(ui: &mut egui::Ui, id: &str, namen: &[String], keuze: &mut Option<String>) {
    const STANDAARD: &str = "Standaardapparaat (Windows)";

    egui::ComboBox::from_id_salt(id)
        .width(380.0)
        .selected_text(keuze.clone().unwrap_or_else(|| STANDAARD.to_string()))
        .show_ui(ui, |ui| {
            ui.selectable_value(keuze, None, STANDAARD);
            for naam in namen {
                ui.selectable_value(keuze, Some(naam.clone()), naam);
            }
        });

    if let Some(naam) = keuze.clone() {
        if !namen.contains(&naam) {
            ui.small(
                egui::RichText::new(
                    "Dit apparaat is er nu niet; de app gebruikt zolang het standaardapparaat.",
                )
                .color(theme::STATUS_CONNECTING),
            );
        }
    }
}
