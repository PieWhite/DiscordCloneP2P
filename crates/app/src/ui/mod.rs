//! De UI. Bewust functioneel gehouden — styling volgt in een latere fase.
//!
//! Puur een weergave: leest een momentopname van de motor en stuurt commando's terug.
//! Er wordt hier geen enkele beslissing genomen over netwerk of opslag, en er staat
//! geen state in die verloren gaat als het venster even niet tekent.

pub mod theme;

use crate::config::VideoConfig;
use crate::engine::{self, EngineHandle, FileView, PeerView, Snapshot, UiCommand};
use crate::files::{hash_bestandsnaam, is_afbeelding, DownloadStatus};
use crate::tags;
use crate::tray;
use eframe::egui;
use fitcom_net::PeerStatus;
use fitcom_proto::{Channel, OpId, PeerId, TopicId};
use fitcom_store::Message;
use fitcom_video::{Bron, BronSoort, Miniatuur};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

/// 4 fps als er niets gebeurt. Genoeg om wijzigingen direct te tonen, en
/// verwaarloosbaar qua CPU — de app moet in rust vrijwel niets doen.
const IDLE_REPAINT: Duration = Duration::from_millis(250);
/// Tijdens een gesprek vaker: een spreekindicatie die vier keer per seconde bijwerkt
/// oogt traag. Dit kost pas iets als er daadwerkelijk gepraat wordt.
const VOICE_REPAINT: Duration = Duration::from_millis(80);

pub struct App {
    engine: EngineHandle,
    snap: Arc<Snapshot>,
    mij: PeerId,
    eigen_naam: String,
    control_port: u16,
    data_dir: PathBuf,
    downloads_dir: PathBuf,
    /// Content-adresseerbare map voor afbeeldingen, zowel eigen aanbod als gedownload —
    /// zie `files::hash_bestandsnaam`. Apart van `downloads_dir`: dit zijn geen
    /// gebruikersbestanden met een leesbare naam.
    pictures_dir: PathBuf,
    /// Moet in leven blijven zolang de app draait, anders stopt alles eronder.
    _runtime: tokio::runtime::Runtime,
    invoer: String,
    bewerkt: Option<OpId>,
    vorig_aantal: usize,
    /// Het algemene kanaal, of een DM — bepaalt wat de chat- en bestandenpanelen tonen
    /// en waar een nieuw bericht naartoe gaat.
    actief_kanaal: Channel,
    naar_tray: bool,
    /// `Some` zolang het keuzemenu voor te delen bronnen open staat. De lijst wordt bij
    /// het openen opgehaald: vensters komen en gaan, dus hem bewaren zou hem verouderen.
    bronkeuze: Option<Vec<Bron>>,
    /// `Some` zolang het algemene instellingenscherm open staat. Een kopie van de
    /// video-instellingen om in te bewerken, zodat "annuleren" niets hoeft terug te
    /// draaien — geldt alleen voor de video-sectie van het scherm.
    instellingen: Option<VideoConcept>,
    /// Staat de bevestigingsvraag voor "Verwijder alle afbeeldingen" open? Los van
    /// `instellingen`, zodat annuleren van de bevestiging het instellingenscherm zelf
    /// niet sluit.
    bevestig_verwijder_afbeeldingen: bool,
    /// `Some` zolang het profielvenster open staat. Bewerkbare kopie van de naam, zodat
    /// "annuleren" niets hoeft terug te draaien.
    profiel: Option<String>,
    /// `Some` zolang het invoerveld voor een nieuw subkanaal open staat, met de al
    /// getypte titel erin.
    nieuw_kanaal_titel: Option<String>,
    /// `Some` zolang het hernoem-venster voor een subkanaal open staat: welk subkanaal,
    /// en een bewerkbare kopie van zijn titel.
    kanaal_hernoemen: Option<(TopicId, String)>,
    /// `Some` zolang de bevestigingsvraag voor "subkanaal verwijderen" open staat, met
    /// welk subkanaal het betreft.
    bevestig_verwijder_kanaal: Option<TopicId>,
    /// Welke suggestie in de @tag-autocomplete gemarkeerd is. Reset zodra de getypte
    /// tag verandert, zie `chat_paneel`.
    tag_selectie: usize,
    /// Stond er vorige frame een suggestielijst open? Bepaalt of Tab/Enter dit frame
    /// vóór het tekenen van het invoerveld uit de toetsenbordgebeurtenissen gehaald
    /// moeten worden — anders voegt een multiline `TextEdit` zelf al een tab-teken of
    /// nieuwe regel in vóórdat onze eigen code de tag kan afronden. Zie `chat_paneel`.
    tag_actief: bool,
    /// Geladen teksturen voor het overzicht, met de pointer van de laatst geüploade
    /// `Arc` erbij. Zo hoeft een miniatuur die niet ververst is niet elke frame opnieuw
    /// naar de GPU; alleen een echt nieuwe `Arc` (van de kijk-thread) triggert dat.
    miniatuur_cache: HashMap<(PeerId, u32), (usize, egui::TextureHandle)>,
    /// Geladen miniatuurteksturen van aangeboden afbeeldingen, per `OpId`. Het pad zelf
    /// is altijd deterministisch af te leiden uit `FileView::hash` (zie
    /// `files::hash_bestandsnaam`) — zowel voor wat wij aanbieden als voor wat we
    /// gedownload hebben — dus is er geen aparte boekhouding per bestandsnaam nodig
    /// zoals eerder wel het geval was. De bytes van een aangeboden bestand veranderen
    /// nooit meer, dus dit hoeft nooit ververst te worden zoals `miniatuur_cache` dat
    /// wel moet.
    bijlage_texturen: HashMap<OpId, egui::TextureHandle>,
    /// Was Ctrl+V vorige frame al ingedrukt? Voor randdetectie op `GetAsyncKeyState` —
    /// zie `App::ctrl_v_zojuist_ingedrukt` voor waarom dit niet via egui's eigen
    /// toetsenbordevents kan.
    ctrl_v_ingedrukt: bool,
}

/// Bewerkbare kopie van de video-instellingen. Bitrate in Mbit/s voor de schuif —
/// niemand denkt in bits per seconde.
struct VideoConcept {
    codec: String,
    fps: u32,
    bitrate_mbit: f32,
}

impl App {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        engine: EngineHandle,
        mij: PeerId,
        eigen_naam: String,
        control_port: u16,
        data_dir: PathBuf,
        downloads_dir: PathBuf,
        pictures_dir: PathBuf,
        naar_tray: bool,
        runtime: tokio::runtime::Runtime,
    ) -> Self {
        let snap = engine.snapshot.borrow().clone();
        Self {
            engine,
            snap,
            mij,
            eigen_naam,
            control_port,
            data_dir,
            downloads_dir,
            pictures_dir,
            _runtime: runtime,
            invoer: String::new(),
            bewerkt: None,
            vorig_aantal: 0,
            actief_kanaal: Channel::GENERAL,
            naar_tray,
            bronkeuze: None,
            instellingen: None,
            bevestig_verwijder_afbeeldingen: false,
            profiel: None,
            nieuw_kanaal_titel: None,
            kanaal_hernoemen: None,
            bevestig_verwijder_kanaal: None,
            tag_selectie: 0,
            tag_actief: false,
            miniatuur_cache: HashMap::new(),
            bijlage_texturen: HashMap::new(),
            ctrl_v_ingedrukt: false,
        }
    }

    fn stuur(&self, cmd: UiCommand) {
        if let Err(e) = self.engine.commands.try_send(cmd) {
            tracing::warn!(error = %e, "commando niet doorgegeven aan de motor");
        }
    }

    fn versturen(&mut self) {
        let tekst = self.invoer.trim().to_string();
        if tekst.is_empty() {
            self.invoer.clear();
            self.bewerkt = None;
            return;
        }
        match self.bewerkt.take() {
            Some(doel) => self.stuur(UiCommand::Bewerk(doel, tekst)),
            None => self.stuur(UiCommand::Plaats(tekst, self.actief_kanaal)),
        }
        self.invoer.clear();
    }

    fn naam_van(&self, peer: PeerId) -> String {
        self.snap
            .timeline
            .nicknames
            .get(&peer)
            .cloned()
            .unwrap_or_else(|| peer.to_string()[..8].to_string())
    }

    /// Of een bericht of bestand bij het actief bekeken kanaal hoort.
    ///
    /// Voor het algemene kanaal is dat een gewone gelijkheid. Voor een DM ligt het
    /// subtieler: `Channel::dm(x)` betekent "de auteur DM'de naar x", dus *mijn* eigen
    /// berichten aan X dragen `Dm(X)`, maar X's antwoorden aan mij dragen `Dm(mij)` — niet
    /// `Dm(X)`. Een DM-gesprek met X bestaat dus uit twee verschillende kanaalwaarden, één
    /// per gespreksdeelnemer. Simpelweg vergelijken met `self.actief_kanaal` (wat altijd
    /// `Dm(X)` is) laat daardoor alleen je eigen kant van het gesprek zien en nooit de
    /// antwoorden van de ander.
    fn hoort_bij_actief_kanaal(&self, kanaal: Channel, auteur: PeerId) -> bool {
        hoort_bij_kanaal(self.actief_kanaal, self.mij, kanaal, auteur)
    }

    /// Wisselt van kanaal. Een half getypt bericht of een lopende bewerking hoort niet
    /// per ongeluk in het verkeerde gesprek terecht te komen, dus die vervallen hierbij.
    fn wissel_kanaal(&mut self, kanaal: Channel) {
        if self.actief_kanaal == kanaal {
            return;
        }
        self.actief_kanaal = kanaal;
        self.invoer.clear();
        self.bewerkt = None;
        if let Some(peer) = kanaal.dm_peer() {
            self.stuur(UiCommand::GelezenDm(peer));
        } else if let Some(topic) = kanaal.topic_id() {
            self.stuur(UiCommand::GelezenTopic(topic));
        } else {
            self.stuur(UiCommand::Gelezen);
        }
    }

    /// Levert `true` als er deze frame niets meer getekend hoeft te worden.
    ///
    /// De sluitknop verbergt naar de tray in plaats van af te sluiten: de motor loopt
    /// door, dus je blijft berichten ontvangen en een melding krijgen terwijl je iets
    /// anders doet. Echt afsluiten gaat via het tray-menu.
    fn afsluiten_of_verbergen(&mut self, ctx: &egui::Context) -> bool {
        if tray::wil_afsluiten() || self.engine.afsluiten_voor_update.load(Ordering::Relaxed) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return true;
        }

        if ctx.input(|i| i.viewport().close_requested()) {
            if !self.naar_tray {
                return false; // gewoon afsluiten
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            // De motor moet weten dat we niet meer kijken, anders blijven meldingen uit.
            self.engine.voorgrond.store(false, Ordering::Relaxed);
            tray::verberg_venster();
            return true;
        }

        false
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.snap = self.engine.snapshot.borrow_and_update().clone();
        ctx.request_repaint_after(if self.snap.voice.actief {
            VOICE_REPAINT
        } else {
            IDLE_REPAINT
        });

        if self.afsluiten_of_verbergen(ctx) {
            return;
        }

        // De motor gebruikt dit om te bepalen of er een Windows-melding moet komen.
        let voorgrond = ctx.input(|i| i.focused);
        self.engine.voorgrond.store(voorgrond, Ordering::Relaxed);
        if voorgrond {
            // Alleen het kanaal dat je daadwerkelijk bekijkt telt als gelezen: zit je
            // in een DM, dan mag dat het algemene kanaal niet stilletjes wegstrepen,
            // en andersom.
            match (self.actief_kanaal.dm_peer(), self.actief_kanaal.topic_id()) {
                (Some(p), _) if self.snap.ongelezen_dm.get(&p).copied().unwrap_or(0) > 0 => {
                    self.stuur(UiCommand::GelezenDm(p));
                }
                (None, Some(t)) if self.snap.ongelezen_topic.get(&t).copied().unwrap_or(0) > 0 => {
                    self.stuur(UiCommand::GelezenTopic(t));
                }
                (None, None) if self.snap.ongelezen > 0 => self.stuur(UiCommand::Gelezen),
                _ => {}
            }
        }

        self.verwerk_gedropte_bestanden(ctx);

        self.deelnemers_paneel(ctx);
        self.bronkeuze_venster(ctx);
        self.instellingen_venster(ctx);
        self.bevestig_verwijder_afbeeldingen_venster(ctx);
        self.profiel_venster(ctx);
        self.kanaal_hernoemen_venster(ctx);
        self.bevestig_verwijder_kanaal_venster(ctx);
        self.update_beschikbaar_venster(ctx);
        self.statusbalk(ctx);
        self.overzicht_strook(ctx);
        self.chat_paneel(ctx);
    }
}

impl App {
    fn deelnemers_paneel(&mut self, ctx: &egui::Context) {
        let mut volume_wijziging: Option<(PeerId, f32)> = None;
        let mut voice_cmd: Option<UiCommand> = None;
        let mut stream_cmd: Option<UiCommand> = None;
        let mut bronnen_openen = false;
        let mut kanaal_wissel: Option<Channel> = None;
        let mut niet_storen_wijziging: Option<bool> = None;

        egui::SidePanel::left("deelnemers")
            .resizable(false)
            .exact_width(240.0)
            .show(ctx, |ui| {
                ui.add_space(8.0);
                ui.heading("Deelnemers");
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
                ui.add_space(4.0);

                let eigen = self
                    .snap
                    .timeline
                    .nicknames
                    .get(&self.mij)
                    .cloned()
                    .unwrap_or_else(|| self.eigen_naam.clone());

                ui.horizontal(|ui| {
                    ui.colored_label(GROEN, "\u{25CF}");
                    ui.label(
                        egui::RichText::new(&eigen)
                            .strong()
                            .color(kleur_van(self.mij)),
                    );
                    ui.weak("(jij)");
                });
                ui.horizontal(|ui| {
                    if ui.small_button("naam wijzigen").clicked() {
                        self.profiel = Some(eigen.clone());
                    }
                    if ui
                        .selectable_label(self.snap.niet_storen, "\u{1F515} niet storen")
                        .clicked()
                    {
                        niet_storen_wijziging = Some(!self.snap.niet_storen);
                    }
                });
                if self.snap.voice.actief {
                    let niveau = if self.snap.voice.muted {
                        0.0
                    } else {
                        self.snap.voice.eigen_niveau
                    };
                    niveaubalk(ui, niveau);
                }
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);

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
                    peer_row(ui, p, &naam);

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
                            kanaal_wissel = Some(Channel::dm(id));
                        }
                    }

                    if in_gesprek && p.in_voice {
                        niveaubalk(ui, p.niveau);
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

                ui.add_space(8.0);
                ui.separator();
                ui.add_space(8.0);
                voice_cmd = self.voice_bediening(ui);

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(6.0);
                let (cmd, openen) = self.deel_bediening(ui);
                stream_cmd = stream_cmd.take().or(cmd);
                bronnen_openen = openen;
            });

        if let Some((id, vol)) = volume_wijziging {
            self.stuur(UiCommand::Volume(id, vol));
        }
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

    /// Wat wij delen, plus de knop om er iets bij te doen.
    ///
    /// Levert het commando en "open het keuzemenu" terug in plaats van ze meteen uit
    /// te voeren: binnen de paneelsluiting is `self` al onveranderlijk geleend.
    fn deel_bediening(&self, ui: &mut egui::Ui) -> (Option<UiCommand>, bool) {
        let mut cmd = None;
        ui.label(egui::RichText::new("Scherm delen").strong());
        ui.add_space(4.0);

        let schermen: Vec<_> = self
            .snap
            .eigen_streams
            .iter()
            .filter(|s| !s.is_geluid)
            .collect();

        for s in &schermen {
            ui.horizontal(|ui| {
                // Delen kost pas iets zodra er iemand kijkt, en dat is precies wat je
                // hier wilt kunnen zien als er een game draait.
                let kleur = if s.kijkers > 0 {
                    GROEN
                } else {
                    egui::Color32::GRAY
                };
                ui.colored_label(kleur, "\u{25CF}");
                ui.small(&s.titel);
            });
            ui.horizontal(|ui| {
                ui.small(match s.kijkers {
                    0 => "niemand kijkt".to_string(),
                    1 => "1 kijker".to_string(),
                    n => format!("{n} kijkers"),
                });
                if ui.small_button("stoppen").clicked() {
                    cmd = Some(UiCommand::StopDelen(s.stream_id));
                }
            });
            ui.add_space(4.0);
        }

        let label = if schermen.is_empty() {
            "Scherm delen…"
        } else {
            "Nog een bron delen…"
        };
        let openen = ui
            .add_sized([ui.available_width(), 26.0], egui::Button::new(label))
            .clicked();

        // Geen eigen knop meer (fase 10): geluid van deze pc gaat automatisch mee zodra
        // er een scherm of venster gedeeld wordt, en stopt automatisch met de laatste.
        // Alleen nog een passieve statusregel, niets om op te klikken.
        if self.snap.eigen_streams.iter().any(|s| s.is_geluid) {
            ui.add_space(4.0);
            ui.small("\u{1F50A} geluid van deze pc gaat automatisch mee");
        }
        (cmd, openen)
    }

    /// Centrale plek waar een lokaal bestand de aanbiedflow ingaat, ongeacht of het via
    /// de bestandsdialoog, slepen-en-neerzetten of Ctrl+V-plakken binnenkwam — alleen een
    /// nieuwe invoerweg, geen nieuwe logica (zie `ROADMAP.md`, fase 8). Is het een
    /// afbeelding, dan kopieert de motor hem zelf naar `pictures_dir` onder een naam op
    /// basis van zijn inhoudshash (`hash_en_bied_aan` in `engine.rs`) — de UI hoeft hier
    /// dus zelf niets te onthouden.
    fn bied_bestand_aan(&mut self, pad: PathBuf) {
        self.stuur(UiCommand::BiedBestandAan(pad, self.actief_kanaal));
    }

    /// Een bestand vanaf Windows in het venster gesleept: start dezelfde aanbiedflow als
    /// de bestandsdialoog.
    fn verwerk_gedropte_bestanden(&mut self, ctx: &egui::Context) {
        let gedropt: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        for pad in gedropt {
            self.bied_bestand_aan(pad);
        }

        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            egui::Area::new(egui::Id::new("sleep_hint"))
                .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 8.0))
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.label("Zet hier neer om te delen");
                    });
                });
        }
    }

    /// Leest een afbeelding van het klembord en schrijft hem als PNG weg in een
    /// tijdelijk bestand. `None` als het klembord geen afbeelding bevat (bijvoorbeeld
    /// gewone tekst, of niets) — dan laat dit egui's eigen tekst-plakken in de
    /// `TextEdit` met rust; die twee klembord-inhouden gaan nooit tegelijk over
    /// hetzelfde pad.
    ///
    /// Dit hoeft geen permanente plek te zijn: `hash_en_bied_aan` in `engine.rs` maakt
    /// er zelf een blijvende, content-adresseerbare kopie van in `pictures_dir` zodra
    /// hij hasht. Dit bestand is daarna niet meer nodig — het opruimen ervan laten we
    /// aan Windows' eigen tijdelijke-bestandenbeheer over, net als bij een gesleept of
    /// via de dialoog gekozen bestand, waarvan het origineel ook niet door de app wordt
    /// aangeraakt.
    /// Of Ctrl+V dit frame *net* is ingedrukt (overgang van los naar ingedrukt).
    ///
    /// Kan niet via egui's eigen toetsenbordevents: `egui-winit` herkent Ctrl+V zelf al
    /// als de OS-plakopdracht (`is_paste_command` in zijn `lib.rs`) en stuurt in dat
    /// geval **geen gewone `Key::V`-event** door — hij probeert zelf tekst van het
    /// klembord te lezen en stopt daarna (`return`), met of zonder succes. Bevat het
    /// klembord alleen een afbeelding (geen tekst), dan komt er dus helemaal niets in
    /// `ctx.input()` terecht om op te reageren — `i.key_pressed(egui::Key::V)` blijft
    /// voor altijd `false`, wat precies verklaart waarom de eerdere aanpak niets deed.
    /// Dit is bevestigd via de app-log: alleen `egui_winit::clipboard`'s eigen (tekst-)
    /// klembordpoging verschijnt daar, nooit een eigen gedetecteerde toetsaanslag.
    ///
    /// In plaats daarvan wordt de fysieke toetsstatus rechtstreeks bij Windows
    /// opgevraagd (`GetAsyncKeyState`), los van egui's eigen event-vertaling. Alleen
    /// als het venster ook daadwerkelijk de voorgrond heeft: deze functie is anders
    /// niet gebonden aan welk venster de OS-focus heeft, en zonder die check zou een
    /// Ctrl+V in een willekeurige andere toepassing hier ook een bestand aanbieden.
    fn ctrl_v_zojuist_ingedrukt(&mut self, ctx: &egui::Context) -> bool {
        use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL, VK_V};

        if !ctx.input(|i| i.focused) {
            self.ctrl_v_ingedrukt = false;
            return false;
        }

        let ingedrukt = |vk: u16| unsafe { GetAsyncKeyState(i32::from(vk)) as u16 & 0x8000 != 0 };
        let nu = ingedrukt(VK_CONTROL.0) && ingedrukt(VK_V.0);
        let net_ingedrukt = nu && !self.ctrl_v_ingedrukt;
        self.ctrl_v_ingedrukt = nu;
        net_ingedrukt
    }

    fn plak_afbeelding(&self) -> Option<PathBuf> {
        // Uitgebreid gelogd (op debug-niveau, dus alleen zichtbaar met
        // `$env:FITCOM_LOG = "debug"`): dit stuk faalde bij Rick zonder duidelijke
        // reden, en "geen afbeelding op het klembord" (heel normaal bij een gewone
        // tekst-plak, want dit loopt nu voor elke Ctrl+V, niet alleen met een
        // afbeelding erop) moet te onderscheiden zijn van een echte bug hieronder.
        let mut klembord = match arboard::Clipboard::new() {
            Ok(k) => k,
            Err(e) => {
                tracing::warn!(error = %e, "klembord openen mislukt bij Ctrl+V");
                return None;
            }
        };
        let beeld = match klembord.get_image() {
            Ok(b) => b,
            Err(e) => {
                tracing::debug!(error = %e, "geen afbeelding op het klembord");
                return None;
            }
        };
        tracing::debug!(
            breedte = beeld.width,
            hoogte = beeld.height,
            bytes = beeld.bytes.len(),
            "afbeelding van het klembord gelezen"
        );

        let breedte = beeld.width as u32;
        let hoogte = beeld.height as u32;
        let bytes = beeld.bytes.into_owned();
        let Some(buffer) = image::RgbaImage::from_raw(breedte, hoogte, bytes) else {
            tracing::warn!(
                breedte,
                hoogte,
                "klembordafbeelding paste niet in breedte×hoogte×4 bytes"
            );
            return None;
        };

        let naam = format!(
            "fitcom-plak-{}.png",
            chrono::Local::now().format("%Y%m%d-%H%M%S%3f")
        );
        let pad = std::env::temp_dir().join(naam);
        if let Err(e) = buffer.save(&pad) {
            tracing::warn!(error = %e, pad = %pad.display(), "klembordafbeelding als PNG wegschrijven mislukt");
            return None;
        }
        tracing::debug!(pad = %pad.display(), "klembordafbeelding weggeschreven");
        Some(pad)
    }

    /// Laadt een eigen aangeboden afbeelding als egui-textuur, of levert de al geladen
    /// textuur terug. Faalt geruisloos (bijvoorbeeld een sindsdien verplaatst pad) —
    /// de aanroeper valt dan terug op de generieke bestandskaart.
    fn bijlage_texture(
        &mut self,
        ctx: &egui::Context,
        file: OpId,
        pad: &Path,
    ) -> Option<(egui::TextureId, egui::Vec2)> {
        if let Some(handle) = self.bijlage_texturen.get(&file) {
            return Some((handle.id(), handle.size_vec2()));
        }

        let beeld = image::open(pad).ok()?.into_rgba8();
        let (breedte, hoogte) = beeld.dimensions();
        let kleur = egui::ColorImage::from_rgba_unmultiplied(
            [breedte as usize, hoogte as usize],
            beeld.as_raw(),
        );
        let handle = ctx.load_texture(
            format!("bijlage-{file:?}"),
            kleur,
            egui::TextureOptions::LINEAR,
        );
        let resultaat = (handle.id(), handle.size_vec2());
        self.bijlage_texturen.insert(file, handle);
        Some(resultaat)
    }

    fn open_bronkeuze(&mut self) {
        match engine::deelbare_bronnen() {
            Ok(b) => self.bronkeuze = Some(b),
            Err(e) => {
                tracing::error!(error = %format!("{e:#}"), "bronnen opvragen mislukt");
                self.bronkeuze = Some(Vec::new());
            }
        }
    }

    fn bronkeuze_venster(&mut self, ctx: &egui::Context) {
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

    /// Strook boven de chat met een levend, verkleind beeld van elke stream die we
    /// bekijken. Bestaat om niet tussen meerdere losse kijkvensters te hoeven zoeken
    /// zodra er meer dan één tegelijk open staat — "meerdere inkomende streams tegelijk
    /// bekijken" uit fase 5. Toont niets zolang er niets bekeken wordt, net als de rest
    /// van screenshare pas iets kost zodra het ergens toe dient.
    fn overzicht_strook(&mut self, ctx: &egui::Context) {
        // Eerst loskoppelen van `self.snap`: zodra we teksturen laden hebben we `self`
        // weer mutabel nodig, en dat mag niet terwijl er nog uit `self.snap` geleend
        // wordt.
        let actief: Vec<(PeerId, u32, String, Option<Miniatuur>)> = self
            .snap
            .streams
            .iter()
            .filter(|s| s.kijken && !s.is_geluid)
            .map(|s| {
                (
                    s.eigenaar,
                    s.stream_id,
                    s.titel.clone(),
                    s.miniatuur.clone(),
                )
            })
            .collect();

        let sleutels: HashSet<(PeerId, u32)> = actief.iter().map(|(p, id, ..)| (*p, *id)).collect();
        self.miniatuur_cache.retain(|k, _| sleutels.contains(k));

        if actief.is_empty() {
            return;
        }

        let tegels: Vec<(String, Option<egui::TextureId>, f32)> = actief
            .into_iter()
            .map(|(peer, id, titel, miniatuur)| match miniatuur {
                Some(m) => {
                    let verhouding = m.breedte as f32 / (m.hoogte.max(1) as f32);
                    let tex = self.miniatuur_texture(ctx, (peer, id), &m);
                    (titel, Some(tex), verhouding)
                }
                None => (titel, None, 16.0 / 9.0),
            })
            .collect();

        egui::TopBottomPanel::top("overzicht")
            .resizable(false)
            .exact_height(148.0)
            .show(ctx, |ui| {
                ui.add_space(4.0);
                egui::ScrollArea::horizontal()
                    .id_salt("overzicht_scroll")
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            for (titel, tex, verhouding) in &tegels {
                                ui.vertical(|ui| {
                                    let hoogte = 108.0;
                                    let breedte = hoogte * verhouding;
                                    match tex {
                                        Some(id) => {
                                            ui.image((*id, egui::vec2(breedte, hoogte)));
                                        }
                                        None => {
                                            let (rect, _) = ui.allocate_exact_size(
                                                egui::vec2(breedte, hoogte),
                                                egui::Sense::hover(),
                                            );
                                            ui.painter().rect_filled(
                                                rect,
                                                4.0,
                                                ui.visuals().extreme_bg_color,
                                            );
                                        }
                                    }
                                    ui.set_max_width(breedte.max(80.0));
                                    ui.small(titel);
                                });
                            }
                        });
                    });
            });
    }

    /// Zet een miniatuur om naar een egui-textuur, of levert de al geladen textuur
    /// terug als de data sinds de vorige frame niet ververst is. Vergelijkt op de
    /// `Arc`-pointer in plaats van de inhoud: die is alleen anders als de kijk-thread
    /// echt een nieuw beeld stuurde, en dan hoeven we geen paar honderd kilobyte te
    /// vergelijken om dat te weten.
    fn miniatuur_texture(
        &mut self,
        ctx: &egui::Context,
        sleutel: (PeerId, u32),
        m: &Miniatuur,
    ) -> egui::TextureId {
        let ptr = Arc::as_ptr(&m.data) as *const u8 as usize;
        if let Some((oude_ptr, handle)) = self.miniatuur_cache.get(&sleutel) {
            if *oude_ptr == ptr {
                return handle.id();
            }
        }

        let rgba = bgra_naar_rgba(&m.data);
        let kleur = egui::ColorImage::from_rgba_unmultiplied(
            [m.breedte as usize, m.hoogte as usize],
            &rgba,
        );
        let naam = format!("miniatuur-{}-{}", sleutel.0, sleutel.1);
        let handle = ctx.load_texture(naam, kleur, egui::TextureOptions::LINEAR);
        let id = handle.id();
        self.miniatuur_cache.insert(sleutel, (ptr, handle));
        id
    }

    /// Levert het commando dat de gebruiker aanklikte terug in plaats van het meteen
    /// te versturen: binnen de paneelsluiting is `self` al onveranderlijk geleend.
    fn voice_bediening(&self, ui: &mut egui::Ui) -> Option<UiCommand> {
        let v = &self.snap.voice;

        if !v.actief {
            if self.snap.peers.iter().any(|p| p.in_voice) {
                ui.small(egui::RichText::new("er is een gesprek bezig").color(GROEN));
                ui.add_space(2.0);
            }
            return ui
                .add_sized([ui.available_width(), 28.0], egui::Button::new("Deelnemen"))
                .clicked()
                .then_some(UiCommand::VoiceDeelnemen);
        }

        let mut cmd = None;
        ui.horizontal(|ui| {
            if ui.selectable_label(v.muted, "mute").clicked() {
                cmd = Some(UiCommand::Mute(!v.muted));
            }
            if ui.selectable_label(v.deafened, "deafen").clicked() {
                cmd = Some(UiCommand::Deafen(!v.deafened));
            }
        });
        ui.add_space(4.0);
        if ui
            .add_sized([ui.available_width(), 24.0], egui::Button::new("Verlaten"))
            .clicked()
        {
            cmd = Some(UiCommand::VoiceVerlaten);
        }
        cmd
    }

    fn statusbalk(&mut self, ctx: &egui::Context) {
        let mut fout_weg = false;
        let mut instellingen_openen = false;

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
                ui.separator();
                if ui.small_button("instellingen").clicked() {
                    instellingen_openen = true;
                }
                if let Some(err) = &self.snap.fout {
                    ui.separator();
                    if ui
                        .add(egui::Label::new(
                            egui::RichText::new(format!("⚠ {err}")).color(ROOD),
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
        if instellingen_openen {
            self.instellingen = Some(VideoConcept {
                codec: self.snap.video.codec.clone(),
                fps: self.snap.video.fps,
                bitrate_mbit: self.snap.video.bitrate as f32 / 1_000_000.0,
            });
        }
    }

    /// Algemeen instellingenscherm: video (codec/fps/bitrate) en beheer van de lokale
    /// `Pictures`-map. Video bewerkt een kopie, zodat "annuleren" niets hoeft terug te
    /// draaien — pas "toepassen" stuurt iets naar de motor. Lopende deelsessies
    /// herstarten daar meteen mee; zie `engine.rs`.
    fn instellingen_venster(&mut self, ctx: &egui::Context) {
        let Some(concept) = &mut self.instellingen else {
            return;
        };
        let mut open = true;
        let mut toepassen = false;
        let mut annuleren = false;
        let mut verwijder_afbeeldingen_geklikt = false;

        egui::Window::new("Instellingen")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(360.0)
            .show(ctx, |ui| {
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
                ui.add(egui::Slider::new(&mut concept.fps, 15..=60));
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

                ui.add_space(16.0);
                ui.separator();
                ui.add_space(10.0);

                ui.heading("Afbeeldingen");
                ui.add_space(6.0);
                ui.small(
                    "Afbeeldingen die je zelf deelt of downloadt staan apart van je gewone \
                     downloads, zodat ze inline in de chat getoond kunnen worden.",
                );
                ui.add_space(6.0);
                if ui.button("Verwijder alle afbeeldingen").clicked() {
                    verwijder_afbeeldingen_geklikt = true;
                }
            });

        if toepassen {
            let concept = self.instellingen.take().unwrap();
            self.stuur(UiCommand::ZetVideoInstellingen(VideoConfig {
                codec: concept.codec,
                fps: concept.fps,
                bitrate: (concept.bitrate_mbit * 1_000_000.0).round() as u32,
            }));
        } else if annuleren || !open {
            self.instellingen = None;
        }
        if verwijder_afbeeldingen_geklikt {
            self.bevestig_verwijder_afbeeldingen = true;
        }
    }

    /// Bevestigingsvraag vóór "Verwijder alle afbeeldingen" — een onomkeerbare
    /// schijfoperatie verdient een expliciete stap ertussen. Raakt alleen lokale
    /// schijfruimte: de berichten/kaarten blijven in de tijdlijn staan (zie
    /// `engine.rs::verwijder_alle_afbeeldingen`).
    fn bevestig_verwijder_afbeeldingen_venster(&mut self, ctx: &egui::Context) {
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

    /// Eigen weergavenaam wijzigen. Bewerkt een kopie zodat "annuleren" niets hoeft
    /// terug te draaien; pas "opslaan" stuurt een `SetNick`-op naar de motor, die hem
    /// ook meteen in `config.toml` bewaart.
    fn profiel_venster(&mut self, ctx: &egui::Context) {
        let Some(concept) = &mut self.profiel else {
            return;
        };
        let mut open = true;
        let mut opslaan = false;
        let mut annuleren = false;

        egui::Window::new("Profiel")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(300.0)
            .show(ctx, |ui| {
                ui.label(egui::RichText::new("Weergavenaam").strong());
                ui.small("Zichtbaar voor de andere peers, overal waar jouw naam getoond wordt.");
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
            let naam = self.profiel.take().unwrap();
            self.stuur(UiCommand::ZetNaam(naam));
        } else if annuleren || !open {
            self.profiel = None;
        }
    }

    /// Titel van een bestaand subkanaal wijzigen. Zelfde mechanisme als aanmaken —
    /// zie `Chat::zet_kanaal_titel` — dus dit venster stuurt gewoon een nieuwe
    /// `HernoemKanaal` met hetzelfde id.
    fn kanaal_hernoemen_venster(&mut self, ctx: &egui::Context) {
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
    fn bevestig_verwijder_kanaal_venster(&mut self, ctx: &egui::Context) {
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
    fn update_beschikbaar_venster(&mut self, ctx: &egui::Context) {
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
                            grootte_tekst(*ontvangen),
                            grootte_tekst(*totaal)
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
                        ui.colored_label(egui::Color32::from_rgb(200, 60, 60), bericht);
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

    fn chat_paneel(&mut self, ctx: &egui::Context) {
        // Invoer eerst vastzetten, zodat de berichtenlijst de rest van de hoogte krijgt
        // en niet onder het invoerveld doorloopt.
        egui::TopBottomPanel::bottom("invoer")
            .resizable(false)
            .show(ctx, |ui| {
                ui.add_space(6.0);

                if self.bewerkt.is_some() {
                    ui.horizontal(|ui| {
                        ui.small("bericht bewerken");
                        if ui.small_button("annuleren").clicked() {
                            self.bewerkt = None;
                            self.invoer.clear();
                        }
                    });
                }

                // Tab en Enter (zonder shift) horen een openstaande tag-suggestie af te
                // ronden, niet een tab-teken of nieuwe regel in te voegen. Een multiline
                // `TextEdit` doet dat laatste zelf al tijdens `.show()`, vóórdat onze
                // eigen code de kans krijgt de tag te herkennen — dus als er vorige frame
                // een suggestielijst open stond, halen we die toetsen er hier al uit.
                let tab_gedrukt = ui.input(|i| i.key_pressed(egui::Key::Tab));
                let enter_zonder_shift_gedrukt =
                    ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);
                if self.tag_actief {
                    ui.input_mut(|i| i.events.retain(|e| !is_tag_toets(e)));
                }

                let knop = if self.bewerkt.is_some() {
                    "opslaan"
                } else {
                    "versturen"
                };

                let (mut output, verstuur_geklikt, bestand_geklikt) = ui
                    .horizontal(|ui| {
                        let breedte = (ui.available_width() - 90.0 - 30.0).max(80.0);
                        let output = egui::TextEdit::multiline(&mut self.invoer)
                            .desired_rows(1)
                            .desired_width(breedte)
                            .hint_text(
                                "bericht… (shift+enter voor een nieuwe regel, sleep of plak \
                                 een bestand)",
                            )
                            .show(ui);
                        let bestand = ui
                            .button("\u{1F4CE}")
                            .on_hover_text("bestand delen")
                            .clicked();
                        let geklikt = ui.button(knop).clicked();
                        (output, geklikt, bestand)
                    })
                    .inner;

                if bestand_geklikt {
                    // Blokkeert kort op de native dialoog — normaal voor een
                    // bestandskeuze en raakt de motor niet: die draait op zijn eigen
                    // tokio-runtime.
                    if let Some(pad) = rfd::FileDialog::new().pick_file() {
                        self.bied_bestand_aan(pad);
                    }
                }

                // Ctrl+V met een afbeelding op het klembord gaat via dezelfde
                // aanbiedflow als een bestand kiezen of slepen, in plaats van als tekst
                // in de invoer terecht te komen. Staat er geen afbeelding op het
                // klembord (bijvoorbeeld gewone tekst), dan gebeurt hier niets en blijft
                // egui's eigen tekst-plakken in de `TextEdit` intact.
                //
                // Bewust *niet* gebonden aan focus op de chatbox: na een screenshot
                // (Win+Shift+S) alt-tab je terug naar het venster en druk je meteen
                // Ctrl+V, zonder eerst ergens in te klikken. Alleen als er een ander
                // modaal venster open staat (bijvoorbeeld het profiel, waar je gewoon
                // tekst wilt kunnen plakken) doet dit niets — anders zou een
                // klembord-afbeelding daar een verrassend bestand aanbieden.
                //
                // Zie `App::ctrl_v_zojuist_ingedrukt` voor waarom dit via
                // `GetAsyncKeyState` gaat en niet via egui's eigen `key_pressed`.
                let geen_modaal_venster_open = self.profiel.is_none()
                    && self.instellingen.is_none()
                    && self.bronkeuze.is_none()
                    && self.kanaal_hernoemen.is_none()
                    && self.nieuw_kanaal_titel.is_none()
                    && self.bevestig_verwijder_kanaal.is_none();
                if geen_modaal_venster_open && self.ctrl_v_zojuist_ingedrukt(ctx) {
                    tracing::debug!(
                        "ctrl+v gezien, klembord wordt gecontroleerd op een afbeelding"
                    );
                    if let Some(pad) = self.plak_afbeelding() {
                        self.bied_bestand_aan(pad);
                    }
                }

                // Welke tag er nog getypt wordt, op basis van de cursor. Alleen relevant
                // zolang het veld focus heeft — anders zou een klik ergens anders de
                // laatst gebruikte tag-positie laten "hangen".
                let actieve_tag = if output.response.has_focus() {
                    output.cursor_range.and_then(|c| {
                        let cursor_byte = char_naar_byte(&self.invoer, c.primary.index);
                        tags::actieve_tag(&self.invoer, cursor_byte)
                            .map(|(start, query)| (start, query.to_string()))
                    })
                } else {
                    None
                };

                let namen: Vec<String> = self.snap.timeline.nicknames.values().cloned().collect();
                let suggesties: Vec<String> = match &actieve_tag {
                    Some((_, query)) => tags::tag_suggesties(&namen, query)
                        .into_iter()
                        .map(String::from)
                        .collect(),
                    None => Vec::new(),
                };
                self.tag_actief = !suggesties.is_empty();
                if suggesties.is_empty() {
                    self.tag_selectie = 0;
                } else {
                    self.tag_selectie = self.tag_selectie.min(suggesties.len() - 1);
                    if ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
                        self.tag_selectie = (self.tag_selectie + 1) % suggesties.len();
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
                        self.tag_selectie =
                            (self.tag_selectie + suggesties.len() - 1) % suggesties.len();
                    }
                }

                let mut te_voltooien: Option<String> = None;
                if !suggesties.is_empty() && (tab_gedrukt || enter_zonder_shift_gedrukt) {
                    te_voltooien = Some(suggesties[self.tag_selectie].clone());
                }

                if !suggesties.is_empty() {
                    ui.add_space(2.0);
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        for (i, naam) in suggesties.iter().enumerate() {
                            if ui.selectable_label(i == self.tag_selectie, naam).clicked() {
                                te_voltooien = Some(naam.clone());
                            }
                        }
                    });
                }

                if let (Some((start, query)), Some(naam)) = (&actieve_tag, &te_voltooien) {
                    let eind = start + 1 + query.len();
                    let ingevoegd = format!("@{naam} ");
                    self.invoer.replace_range(*start..eind, &ingevoegd);
                    let nieuwe_cursor = self.invoer[..start + ingevoegd.len()].chars().count();
                    output
                        .state
                        .cursor
                        .set_char_range(Some(egui::text::CCursorRange::one(
                            egui::text::CCursor::new(nieuwe_cursor),
                        )));
                    output.state.store(ui.ctx(), output.response.id);
                    output.response.request_focus();
                    self.tag_selectie = 0;
                    self.tag_actief = false;
                }

                // Enter verstuurt alleen als hij niet net een tag heeft afgerond — dat
                // is al hierboven verwerkt.
                let enter_voor_versturen = te_voltooien.is_none()
                    && output.response.has_focus()
                    && enter_zonder_shift_gedrukt;

                if verstuur_geklikt || enter_voor_versturen {
                    // Stond er geen tag-popup open, dan heeft de TextEdit de enter al
                    // als nieuwe regel verwerkt — die halen we er weer uit.
                    if enter_voor_versturen {
                        if let Some(p) = self.invoer.rfind('\n') {
                            self.invoer.truncate(p);
                        }
                    }
                    self.versturen();
                    output.response.request_focus();
                }
                ui.add_space(6.0);
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            let mut te_bewerken: Option<(OpId, String)> = None;
            let mut te_verwijderen: Option<OpId> = None;
            let mut te_downloaden: Option<OpId> = None;
            let mut open_downloads = false;

            ui.horizontal(|ui| {
                match (self.actief_kanaal.dm_peer(), self.actief_kanaal.topic_id()) {
                    (Some(p), _) => {
                        ui.label(
                            egui::RichText::new(format!("DM met {}", self.naam_van(p))).strong(),
                        );
                        ui.weak("alleen jij en deze peer zien dit gesprek");
                    }
                    (None, Some(t)) => {
                        let titel = self
                            .snap
                            .timeline
                            .topics
                            .get(&t)
                            .cloned()
                            .unwrap_or_else(|| "onbekend subkanaal".to_string());
                        ui.label(egui::RichText::new(format!("# {titel}")).strong());
                    }
                    (None, None) => {
                        ui.label(egui::RichText::new("# Algemeen").strong());
                    }
                }
            });
            ui.separator();

            // Onafhankelijke kopie van de `Arc`: zo blijft `items` hieronder niet aan
            // `self` geleend, en kan er verderop in de lus alsnog een `&mut self`-methode
            // (`bijlage_texture`) aangeroepen worden om een miniatuur te laden. Kost geen
            // kopie van de geschiedenis, alleen een refcount — zie `Snapshot` in
            // `engine.rs`.
            let snap = Arc::clone(&self.snap);

            // Berichten en bestanden op hun eigen plek in de tijdlijn, chronologisch
            // geïnterleaved. Beide hebben al een `lamport`-sleutel van hun oorspronkelijke
            // op, dus is dit dezelfde sortering als de timeline zelf al per lijst
            // aanhoudt — hier alleen samengevoegd. Zie `ROADMAP.md`, fase 8.
            let mut items: Vec<ChatItem> = snap
                .timeline
                .messages
                .iter()
                .filter(|m| self.hoort_bij_actief_kanaal(m.channel, m.author))
                .map(ChatItem::Bericht)
                .chain(
                    snap.files
                        .iter()
                        .filter(|f| self.hoort_bij_actief_kanaal(f.channel, f.author))
                        .map(ChatItem::Bestand),
                )
                .collect();
            items.sort_by_key(|item| match item {
                ChatItem::Bericht(m) => (m.lamport, m.author),
                ChatItem::Bestand(f) => (f.lamport, f.author),
            });

            // Alleen naar beneden springen als er echt iets bij is gekomen; anders kun
            // je niet terugscrollen in de geschiedenis terwijl de RTT blijft tikken.
            let gegroeid = items.len() != self.vorig_aantal;
            self.vorig_aantal = items.len();

            // Waarop een tag naar "jezelf" gecontroleerd wordt: dezelfde naam die ook
            // getoond wordt, dus exact wat een ander zou typen om jou te taggen.
            let eigen_naam = self.naam_van(self.mij);

            egui::ScrollArea::vertical()
                .stick_to_bottom(gegroeid)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if items.is_empty() {
                        ui.add_space(20.0);
                        ui.vertical_centered(|ui| {
                            ui.weak("Nog geen berichten of bestanden.");
                            ui.small(
                                "Wat je hier plaatst wordt bewaard en komt aan zodra de \
                                 anderen online zijn.",
                            );
                        });
                    }

                    for item in items {
                        match item {
                            ChatItem::Bericht(msg) => {
                                let getagd = tags::bevat_tag(&msg.body, &eigen_naam);

                                let mut teken = |ui: &mut egui::Ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            egui::RichText::new(self.naam_van(msg.author))
                                                .strong()
                                                .color(kleur_van(msg.author)),
                                        );
                                        ui.small(egui::RichText::new(tijd(msg.created_at)).weak());
                                        if msg.edited {
                                            ui.small(egui::RichText::new("(bewerkt)").weak());
                                        }

                                        if msg.author == self.mij {
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    if ui.small_button("verwijder").clicked() {
                                                        te_verwijderen = Some(msg.id);
                                                    }
                                                    if ui.small_button("bewerk").clicked() {
                                                        te_bewerken =
                                                            Some((msg.id, msg.body.clone()));
                                                    }
                                                },
                                            );
                                        }
                                    });

                                    toon_tekst(ui, &msg.body);
                                };

                                // Een tag naar jezelf springt eruit met een gekleurd
                                // kader — subtiel genoeg om niet als foutmelding te
                                // lezen, opvallend genoeg om in een lange geschiedenis
                                // terug te vinden.
                                if getagd {
                                    egui::Frame::group(ui.style())
                                        .fill(TAG_ACHTERGROND)
                                        .stroke(egui::Stroke::new(1.0_f32, TAG_RAND))
                                        .inner_margin(6.0)
                                        .show(ui, teken);
                                } else {
                                    teken(ui);
                                }
                            }
                            ChatItem::Bestand(f) => {
                                ui.label(
                                    egui::RichText::new(if f.is_mine {
                                        "jij".to_string()
                                    } else {
                                        self.naam_van(f.author)
                                    })
                                    .strong()
                                    .color(kleur_van(f.author)),
                                );

                                // Content-adresseerbaar: de aanbieder én elke
                                // downloadende peer komen op exact hetzelfde pad uit
                                // (zie `files::hash_bestandsnaam`), dus dit werkt voor
                                // eigen én ontvangen afbeeldingen. Staat het bestand er
                                // nog niet (niet gedownload, of nog niet gehasht), dan
                                // faalt dit geruisloos en valt de kaart terug op de
                                // generieke weergave.
                                let miniatuur = if is_afbeelding(&f.name) {
                                    let pad =
                                        self.pictures_dir.join(hash_bestandsnaam(&f.hash, &f.name));
                                    self.bijlage_texture(ui.ctx(), f.id, &pad)
                                } else {
                                    None
                                };

                                match miniatuur {
                                    Some((tex, natuurlijk)) => {
                                        let schaal = (240.0 / natuurlijk.x).min(1.0);
                                        ui.image((tex, natuurlijk * schaal));
                                        ui.horizontal(|ui| {
                                            ui.small(&f.name);
                                            if f.is_mine && ui.small_button("verwijder").clicked() {
                                                te_verwijderen = Some(f.id);
                                            }
                                        });
                                    }
                                    None => {
                                        egui::Frame::group(ui.style()).inner_margin(6.0).show(
                                            ui,
                                            |ui| {
                                                ui.label(egui::RichText::new(&f.name).strong());
                                                ui.small(grootte_tekst(f.size));

                                                if f.is_mine {
                                                    ui.horizontal(|ui| {
                                                        ui.small(
                                                            egui::RichText::new(
                                                                "aangeboden door jou",
                                                            )
                                                            .weak(),
                                                        );
                                                        if ui.small_button("verwijder").clicked() {
                                                            te_verwijderen = Some(f.id);
                                                        }
                                                    });
                                                } else {
                                                    match &f.status {
                                                        None => {
                                                            if ui
                                                                .small_button("downloaden")
                                                                .clicked()
                                                            {
                                                                te_downloaden = Some(f.id);
                                                            }
                                                        }
                                                        Some(DownloadStatus::Bezig {
                                                            ontvangen,
                                                            totaal,
                                                        }) => {
                                                            let deel = if *totaal > 0 {
                                                                *ontvangen as f32 / *totaal as f32
                                                            } else {
                                                                0.0
                                                            };
                                                            ui.add(
                                                                egui::ProgressBar::new(deel).text(
                                                                    format!(
                                                                        "{} / {}",
                                                                        grootte_tekst(*ontvangen),
                                                                        grootte_tekst(*totaal)
                                                                    ),
                                                                ),
                                                            );
                                                        }
                                                        Some(DownloadStatus::Voltooid) => {
                                                            ui.horizontal(|ui| {
                                                                ui.colored_label(
                                                                    GROEN,
                                                                    "\u{2713} gedownload",
                                                                );
                                                                if ui
                                                                    .small_button("map openen")
                                                                    .clicked()
                                                                {
                                                                    open_downloads = true;
                                                                }
                                                            });
                                                        }
                                                        Some(DownloadStatus::Mislukt(bericht)) => {
                                                            ui.small(
                                                                egui::RichText::new(format!(
                                                                    "mislukt: {bericht}"
                                                                ))
                                                                .color(ROOD),
                                                            );
                                                            if ui
                                                                .small_button("opnieuw proberen")
                                                                .clicked()
                                                            {
                                                                te_downloaden = Some(f.id);
                                                            }
                                                        }
                                                    }
                                                }
                                            },
                                        );
                                    }
                                }
                            }
                        }
                        ui.add_space(8.0);
                    }
                });

            if let Some((id, body)) = te_bewerken {
                self.bewerkt = Some(id);
                self.invoer = body;
            }
            if let Some(id) = te_verwijderen {
                self.stuur(UiCommand::Verwijder(id));
            }
            if let Some(id) = te_downloaden {
                self.stuur(UiCommand::DownloadBestand(id));
            }
            if open_downloads {
                let _ = std::process::Command::new("explorer")
                    .arg(&self.downloads_dir)
                    .spawn();
            }
        });
    }
}

/// Eén plek in de chronologische tijdlijn: een bericht of een aangeboden bestand. Beide
/// dragen al een `lamport`-sleutel van hun oorspronkelijke op, dus zijn ze op dezelfde
/// manier te sorteren als de timeline zelf — hier alleen samengevoegd zodat een
/// aangeboden bestand op zijn eigen plek tussen de berichten verschijnt in plaats van in
/// een los paneel. Zie `ROADMAP.md`, fase 8.
enum ChatItem<'a> {
    Bericht(&'a Message),
    Bestand(&'a FileView),
}

/// Rendert de tekst met herkenbare codeblokken. Bewust minimaal: we kijken samen naar
/// code, dus ``` moet werken — de rest van markdown is nu niet nodig.
fn toon_tekst(ui: &mut egui::Ui, body: &str) {
    let mut in_code = false;
    for deel in body.split("```") {
        if !deel.is_empty() {
            if in_code {
                egui::Frame::group(ui.style())
                    .inner_margin(6.0)
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(deel.trim_matches('\n')).monospace(),
                            )
                            .wrap(),
                        );
                    });
            } else {
                ui.add(egui::Label::new(deel.trim_matches('\n')).wrap());
            }
        }
        in_code = !in_code;
    }
}

fn peer_row(ui: &mut egui::Ui, p: &PeerView, naam: &str) {
    let (color, text) = describe(&p.status);

    ui.horizontal(|ui| {
        ui.colored_label(color, "\u{25CF}");
        ui.vertical(|ui| {
            let naam_kleur = p
                .peer_id
                .map(kleur_van)
                .unwrap_or(ui.visuals().text_color());
            ui.label(egui::RichText::new(naam).strong().color(naam_kleur));
            ui.small(egui::RichText::new(text).color(color));
        });
    })
    .response
    .on_hover_text(&p.address);
}

const GROEN: egui::Color32 = egui::Color32::from_rgb(80, 200, 120);
const GEEL: egui::Color32 = egui::Color32::from_rgb(220, 180, 70);
const GRIJS: egui::Color32 = egui::Color32::from_rgb(130, 130, 130);
const ROOD: egui::Color32 = egui::Color32::from_rgb(220, 90, 90);
/// Zacht genoeg om niet als foutmelding te lezen, zowel licht als donker thema.
const TAG_ACHTERGROND: egui::Color32 = egui::Color32::from_rgba_premultiplied(90, 75, 20, 40);
const TAG_RAND: egui::Color32 = GEEL;

fn describe(status: &PeerStatus) -> (egui::Color32, String) {
    match status {
        PeerStatus::Online { rtt_ms, .. } => (GROEN, format!("online · {rtt_ms} ms")),
        PeerStatus::Connecting => (GEEL, "verbinden…".into()),
        PeerStatus::Offline { reason } => (GRIJS, format!("offline · {reason}")),
        PeerStatus::VersionMismatch { theirs, ours } => (
            ROOD,
            format!("versie {theirs} vs {ours} — één van beiden moet updaten"),
        ),
        PeerStatus::IdentityChanged { .. } => {
            (ROOD, "andere identiteit dan verwacht op dit adres".into())
        }
    }
}

/// Stabiele kleur per peer, zodat je in de chat aan de kleur ziet wie wat zei.
fn kleur_van(peer: PeerId) -> egui::Color32 {
    let b = peer.as_bytes();
    let tint = (u16::from(b[0]) << 8) | u16::from(b[1]);
    let hoek = f32::from(tint) / 65535.0 * 360.0;
    // Vaste verzadiging en helderheid: elke peer krijgt een goed leesbare kleur,
    // ook in een donker thema.
    let (r, g, bl) = hsv_naar_rgb(hoek, 0.55, 0.95);
    egui::Color32::from_rgb(r, g, bl)
}

fn hsv_naar_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match h as u32 / 60 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    (
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    )
}

/// D3D11 levert BGRA, egui verwacht RGBA. Alleen de eerste en derde byte per pixel
/// wisselen; alfa en groen staan al goed.
fn bgra_naar_rgba(data: &[u8]) -> Vec<u8> {
    let mut uit = data.to_vec();
    for pixel in uit.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    uit
}

/// Leesbare bestandsgrootte. Bewust grof (één decimaal): niemand telt hier mee.
fn grootte_tekst(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

/// Zet een egui-cursorpositie (teken-index) om naar een byte-offset in `s`. egui telt
/// in tekens, `tags::actieve_tag` in bytes — nodig voor niet-ASCII namen.
fn char_naar_byte(s: &str, char_index: usize) -> usize {
    s.char_indices()
        .nth(char_index)
        .map(|(b, _)| b)
        .unwrap_or(s.len())
}

/// Of dit een toets is die we uit de invoer moeten halen zolang er een tag-suggestie
/// openstaat: Tab (voegt anders een tab-teken in) en Enter zonder shift (voegt anders
/// een nieuwe regel in). Shift+Enter blijft gewoon een nieuwe regel geven.
fn is_tag_toets(e: &egui::Event) -> bool {
    matches!(
        e,
        egui::Event::Key {
            key: egui::Key::Tab,
            pressed: true,
            ..
        }
    ) || matches!(
        e,
        egui::Event::Key {
            key: egui::Key::Enter,
            pressed: true,
            modifiers,
            ..
        } if !modifiers.shift
    )
}

fn tijd(millis: i64) -> String {
    use chrono::{Local, TimeZone};
    match Local.timestamp_millis_opt(millis).single() {
        Some(t) => t.format("%H:%M").to_string(),
        None => String::new(),
    }
}

/// Smalle balk die meebeweegt met hoe hard iemand praat.
///
/// Logaritmisch geschaald: spraak zit qua energie laag ten opzichte van het maximum,
/// en lineair zou de balk nauwelijks bewegen.
fn niveaubalk(ui: &mut egui::Ui, niveau: f32) {
    let deel = if niveau <= 0.0005 {
        0.0
    } else {
        ((niveau.log10() * 20.0 + 60.0) / 60.0).clamp(0.0, 1.0)
    };

    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width().min(200.0), 4.0),
        egui::Sense::hover(),
    );
    let schilder = ui.painter();
    schilder.rect_filled(rect, 2.0, ui.visuals().extreme_bg_color);
    if deel > 0.0 {
        let mut gevuld = rect;
        gevuld.set_width(rect.width() * deel);
        schilder.rect_filled(gevuld, 2.0, GROEN);
    }
}

/// Of een op met dit `(kanaal, auteur)`-paar hoort bij wat je nu bekijkt (`actief`, vanuit
/// je eigen standpunt `mij`).
///
/// Losstaand van `App` zodat dit zonder een hele `EngineHandle` te testen is. Zie
/// `App::hoort_bij_actief_kanaal` voor de uitleg van de valkuil die dit voorkomt: een
/// DM-gesprek met X bestaat uit twee kanaalwaarden (`Dm(X)` voor jouw berichten, `Dm(mij)`
/// voor die van X), dus simpelweg vergelijken met `actief` (altijd `Dm(X)`) laat de helft
/// van het gesprek verdwijnen. Voor het algemene kanaal en een subkanaal geldt die valkuil
/// niet — daar draagt elke op, van wie dan ook, precies dezelfde kanaalwaarde — dus is een
/// gewone gelijkheid genoeg.
fn hoort_bij_kanaal(actief: Channel, mij: PeerId, kanaal: Channel, auteur: PeerId) -> bool {
    match actief.dm_peer() {
        Some(partner) => {
            (auteur == mij && kanaal == Channel::dm(partner))
                || (auteur == partner && kanaal == Channel::dm(mij))
        }
        None => kanaal == actief,
    }
}

#[cfg(test)]
mod kanaal_tests {
    use super::*;

    fn peer(n: u8) -> PeerId {
        let mut b = [0u8; 16];
        b[0] = n;
        PeerId::from_bytes(b)
    }

    fn topic(n: u8) -> TopicId {
        TopicId::from_bytes([n; 16])
    }

    #[test]
    fn subkanaal_toont_alleen_zijn_eigen_berichten() {
        let (mij, ander) = (peer(1), peer(2));
        let a = Channel::topic(topic(1));
        let b = Channel::topic(topic(2));

        assert!(hoort_bij_kanaal(a, mij, a, ander), "eigen subkanaal");
        assert!(
            !hoort_bij_kanaal(a, mij, b, ander),
            "een ander subkanaal hoort er niet bij"
        );
        assert!(
            !hoort_bij_kanaal(a, mij, Channel::GENERAL, ander),
            "het algemene kanaal hoort niet bij een subkanaal"
        );
    }

    #[test]
    fn algemeen_toont_geen_berichten_uit_een_subkanaal() {
        let (mij, ander) = (peer(1), peer(2));
        assert!(!hoort_bij_kanaal(
            Channel::GENERAL,
            mij,
            Channel::topic(topic(1)),
            ander
        ));
    }

    #[test]
    fn algemeen_toont_alleen_algemene_berichten() {
        let (mij, ander) = (peer(1), peer(2));
        assert!(hoort_bij_kanaal(
            Channel::GENERAL,
            mij,
            Channel::GENERAL,
            ander
        ));
        assert!(!hoort_bij_kanaal(
            Channel::GENERAL,
            mij,
            Channel::dm(mij),
            ander
        ));
    }

    #[test]
    fn dm_toont_beide_kanten_van_het_gesprek() {
        // Precies de bug die de reviewer vond: mijn eigen berichten aan X dragen Dm(X),
        // maar X's antwoorden aan mij dragen Dm(mij), niet Dm(X).
        let (mij, x) = (peer(1), peer(2));
        let actief = Channel::dm(x);

        assert!(
            hoort_bij_kanaal(actief, mij, Channel::dm(x), mij),
            "mijn eigen bericht aan X moet zichtbaar zijn"
        );
        assert!(
            hoort_bij_kanaal(actief, mij, Channel::dm(mij), x),
            "X's antwoord aan mij moet zichtbaar zijn"
        );
    }

    #[test]
    fn dm_toont_geen_berichten_uit_een_ander_gesprek() {
        let (mij, x, derde) = (peer(1), peer(2), peer(3));
        let actief = Channel::dm(x);

        assert!(!hoort_bij_kanaal(actief, mij, Channel::dm(derde), mij));
        assert!(!hoort_bij_kanaal(actief, mij, Channel::dm(mij), derde));
        assert!(!hoort_bij_kanaal(actief, mij, Channel::GENERAL, x));
    }
}
