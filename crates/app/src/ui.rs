//! De UI. Bewust functioneel gehouden — styling volgt in een latere fase.
//!
//! Puur een weergave: leest een momentopname van de motor en stuurt commando's terug.
//! Er wordt hier geen enkele beslissing genomen over netwerk of opslag, en er staat
//! geen state in die verloren gaat als het venster even niet tekent.

use crate::engine::{self, EngineHandle, PeerView, Snapshot, UiCommand};
use crate::tray;
use eframe::egui;
use fitcom_net::PeerStatus;
use fitcom_proto::{OpId, PeerId};
use fitcom_video::{Bron, BronSoort};
use std::path::PathBuf;
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
    /// Moet in leven blijven zolang de app draait, anders stopt alles eronder.
    _runtime: tokio::runtime::Runtime,
    invoer: String,
    bewerkt: Option<OpId>,
    vorig_aantal: usize,
    naar_tray: bool,
    /// `Some` zolang het keuzemenu voor te delen bronnen open staat. De lijst wordt bij
    /// het openen opgehaald: vensters komen en gaan, dus hem bewaren zou hem verouderen.
    bronkeuze: Option<Vec<Bron>>,
}

impl App {
    pub fn new(
        engine: EngineHandle,
        mij: PeerId,
        eigen_naam: String,
        control_port: u16,
        data_dir: PathBuf,
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
            _runtime: runtime,
            invoer: String::new(),
            bewerkt: None,
            vorig_aantal: 0,
            naar_tray,
            bronkeuze: None,
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
            None => self.stuur(UiCommand::Plaats(tekst)),
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

    /// Levert `true` als er deze frame niets meer getekend hoeft te worden.
    ///
    /// De sluitknop verbergt naar de tray in plaats van af te sluiten: de motor loopt
    /// door, dus je blijft berichten ontvangen en een melding krijgen terwijl je iets
    /// anders doet. Echt afsluiten gaat via het tray-menu.
    fn afsluiten_of_verbergen(&mut self, ctx: &egui::Context) -> bool {
        if tray::wil_afsluiten() {
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
        if voorgrond && self.snap.ongelezen > 0 {
            self.stuur(UiCommand::Gelezen);
        }

        self.deelnemers_paneel(ctx);
        self.bronkeuze_venster(ctx);
        self.statusbalk(ctx);
        self.chat_paneel(ctx);
    }
}

impl App {
    fn deelnemers_paneel(&mut self, ctx: &egui::Context) {
        let mut volume_wijziging: Option<(PeerId, f32)> = None;
        let mut voice_cmd: Option<UiCommand> = None;
        let mut stream_cmd: Option<UiCommand> = None;
        let mut bronnen_openen = false;

        egui::SidePanel::left("deelnemers")
            .resizable(false)
            .exact_width(240.0)
            .show(ctx, |ui| {
                ui.add_space(8.0);
                ui.heading("Deelnemers");
                ui.add_space(8.0);

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
                        egui::RichText::new(eigen)
                            .strong()
                            .color(kleur_van(self.mij)),
                    );
                    ui.weak("(jij)");
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
                                ui.small("\u{1F5B5}");
                                ui.small(&s.titel)
                                    .on_hover_text(format!("{}×{}", s.breedte, s.hoogte));
                            });
                            let knop = if s.kijken { "sluiten" } else { "bekijken" };
                            if ui.small_button(knop).clicked() {
                                stream_cmd = Some(if s.kijken {
                                    UiCommand::StopKijken(id, s.stream_id)
                                } else {
                                    UiCommand::Kijken(id, s.stream_id)
                                });
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
    }

    /// Wat wij delen, plus de knop om er iets bij te doen.
    ///
    /// Levert het commando en "open het keuzemenu" terug in plaats van ze meteen uit
    /// te voeren: binnen de paneelsluiting is `self` al onveranderlijk geleend.
    fn deel_bediening(&self, ui: &mut egui::Ui) -> (Option<UiCommand>, bool) {
        let mut cmd = None;
        ui.label(egui::RichText::new("Scherm delen").strong());
        ui.add_space(4.0);

        for s in &self.snap.eigen_streams {
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

        let label = if self.snap.eigen_streams.is_empty() {
            "Scherm delen…"
        } else {
            "Nog een bron delen…"
        };
        let openen = ui
            .add_sized([ui.available_width(), 26.0], egui::Button::new(label))
            .clicked();
        (cmd, openen)
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

                ui.horizontal(|ui| {
                    let knop = if self.bewerkt.is_some() {
                        "opslaan"
                    } else {
                        "versturen"
                    };
                    let breedte = (ui.available_width() - 90.0).max(80.0);

                    let veld = ui.add_sized(
                        [breedte, 24.0],
                        egui::TextEdit::multiline(&mut self.invoer)
                            .desired_rows(1)
                            .hint_text("bericht… (shift+enter voor een nieuwe regel)"),
                    );

                    // Enter verstuurt, shift+enter maakt een nieuwe regel. De TextEdit
                    // heeft de enter al ingevoegd, dus die halen we er weer uit.
                    let enter = veld.has_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);

                    if ui.button(knop).clicked() || enter {
                        if enter {
                            if let Some(p) = self.invoer.rfind('\n') {
                                self.invoer.truncate(p);
                            }
                        }
                        self.versturen();
                        veld.request_focus();
                    }
                });
                ui.add_space(6.0);
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            let mut te_bewerken: Option<(OpId, String)> = None;
            let mut te_verwijderen: Option<OpId> = None;

            // Alleen naar beneden springen als er echt iets bij is gekomen; anders kun
            // je niet terugscrollen in de geschiedenis terwijl de RTT blijft tikken.
            let gegroeid = self.snap.timeline.messages.len() != self.vorig_aantal;
            self.vorig_aantal = self.snap.timeline.messages.len();

            egui::ScrollArea::vertical()
                .stick_to_bottom(gegroeid)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if self.snap.timeline.messages.is_empty() {
                        ui.add_space(20.0);
                        ui.vertical_centered(|ui| {
                            ui.weak("Nog geen berichten.");
                            ui.small(
                                "Wat je hier typt wordt bewaard en komt aan zodra de \
                                 anderen online zijn.",
                            );
                        });
                    }

                    for msg in &self.snap.timeline.messages {
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
                                            te_bewerken = Some((msg.id, msg.body.clone()));
                                        }
                                    },
                                );
                            }
                        });

                        toon_tekst(ui, &msg.body);
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
        });
    }
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
