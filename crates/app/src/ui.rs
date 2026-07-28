//! De UI. Bewust functioneel gehouden — styling volgt in een latere fase.
//!
//! De UI-thread doet geen netwerkwerk, geen database-werk op de achtergrond en houdt
//! geen locks vast. Alles komt binnen via `MeshEvent` en gaat eruit via `MeshCommand`.

use crate::chat::Chat;
use crate::config::{Config, Identity};
use eframe::egui;
use fitcom_net::{MeshCommand, MeshEvent, MeshHandle, PeerStatus};
use fitcom_proto::{OpId, PeerId};
use std::path::PathBuf;
use std::time::Duration;

/// 4 fps als er niets gebeurt. Genoeg om statuswijzigingen direct te tonen, en
/// verwaarloosbaar qua CPU — de app moet in rust vrijwel niets doen.
const IDLE_REPAINT: Duration = Duration::from_millis(250);

pub struct App {
    cfg: Config,
    identity: Identity,
    config_path: PathBuf,
    data_dir: PathBuf,
    mesh: MeshHandle,
    chat: Chat,
    /// Moet in leven blijven zolang de app draait, anders stopt de netwerklaag.
    _runtime: tokio::runtime::Runtime,
    rows: Vec<PeerRow>,
    verbonden: Vec<PeerId>,
    invoer: String,
    bewerkt: Option<OpId>,
    scroll_naar_beneden: bool,
    fout: Option<String>,
}

struct PeerRow {
    label: String,
    address: String,
    peer_id: Option<PeerId>,
    status: PeerStatus,
}

impl App {
    pub fn new(
        cfg: Config,
        identity: Identity,
        config_path: PathBuf,
        data_dir: PathBuf,
        mesh: MeshHandle,
        chat: Chat,
        runtime: tokio::runtime::Runtime,
    ) -> Self {
        let rows = cfg
            .peers
            .iter()
            .map(|p| PeerRow {
                label: if p.label.is_empty() {
                    p.address.clone()
                } else {
                    p.label.clone()
                },
                address: p.address.clone(),
                peer_id: p.known_id,
                status: PeerStatus::Offline {
                    reason: "nog niet verbonden".into(),
                },
            })
            .collect();

        Self {
            cfg,
            identity,
            config_path,
            data_dir,
            mesh,
            chat,
            _runtime: runtime,
            rows,
            verbonden: Vec::new(),
            invoer: String::new(),
            bewerkt: None,
            scroll_naar_beneden: true,
            fout: None,
        }
    }

    fn verstuur(&mut self, cmds: Vec<MeshCommand>) {
        for cmd in cmds {
            if let Err(e) = self.mesh.commands.try_send(cmd) {
                tracing::warn!(error = %e, "commando niet verstuurd");
            }
        }
    }

    fn meld(&mut self, r: anyhow::Result<Vec<MeshCommand>>) {
        match r {
            Ok(cmds) => self.verstuur(cmds),
            Err(e) => {
                tracing::error!(error = %format!("{e:#}"), "chat-actie mislukt");
                self.fout = Some(format!("{e:#}"));
            }
        }
    }

    fn drain_events(&mut self) {
        let mut acties = Vec::new();

        while let Ok(event) = self.mesh.events.try_recv() {
            match event {
                MeshEvent::Status { target, status } => {
                    let net_online = matches!(status, PeerStatus::Online { .. })
                        && !matches!(
                            self.rows.get(target).map(|r| &r.status),
                            Some(PeerStatus::Online { .. })
                        );

                    if let PeerStatus::Online { peer_id, .. } = &status {
                        if !self.verbonden.contains(peer_id) {
                            self.verbonden.push(*peer_id);
                        }
                        if net_online {
                            acties.push(*peer_id);
                        }
                    } else if let Some(id) = self.rows.get(target).and_then(|r| r.peer_id) {
                        self.verbonden.retain(|p| *p != id);
                    }

                    if let Some(row) = self.rows.get_mut(target) {
                        if let PeerStatus::Online {
                            peer_id,
                            display_name,
                            ..
                        } = &status
                        {
                            row.peer_id = Some(*peer_id);
                            row.label = display_name.clone();
                        }
                        row.status = status;
                    }
                }
                MeshEvent::LearnedIdentity { target, peer_id } => {
                    if let Some(p) = self.cfg.peers.get_mut(target) {
                        p.known_id = Some(peer_id);
                        if let Err(e) = self.cfg.save(&self.config_path) {
                            self.fout = Some(format!("config opslaan: {e:#}"));
                        }
                    }
                }
                MeshEvent::Message { from, msg } => {
                    let r = self.chat.bij_bericht(from, msg);
                    self.meld(r);
                }
            }
        }

        // Een net verbonden peer vraagt om een inhaalslag.
        for peer in acties {
            let r = self.chat.bij_verbinding(peer);
            self.meld(r);
        }

        let verbonden = self.verbonden.clone();
        let r = self.chat.tick(&verbonden);
        self.meld(r);

        if self.chat.refresh() {
            self.scroll_naar_beneden = true;
        }
    }

    fn versturen_klaar(&mut self) {
        let tekst = self.invoer.trim().to_string();
        if tekst.is_empty() {
            self.invoer.clear();
            self.bewerkt = None;
            return;
        }

        let r = match self.bewerkt.take() {
            Some(doel) => self.chat.bewerk_bericht(doel, &tekst),
            None => self.chat.plaats_bericht(&tekst),
        };
        self.meld(r);

        self.invoer.clear();
        self.chat.refresh();
        self.scroll_naar_beneden = true;
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_events();
        ctx.request_repaint_after(IDLE_REPAINT);

        // Kijkt de gebruiker ernaar, dan is het gelezen. De teller is straks ook de
        // trigger voor de toast-notificatie als het venster níét op de voorgrond staat.
        if ctx.input(|i| i.focused) {
            self.chat.markeer_gelezen();
        }

        self.deelnemers_paneel(ctx);
        self.statusbalk(ctx);
        self.chat_paneel(ctx);
    }
}

impl App {
    fn deelnemers_paneel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("deelnemers")
            .resizable(false)
            .exact_width(240.0)
            .show(ctx, |ui| {
                ui.add_space(8.0);
                ui.heading("Deelnemers");
                ui.add_space(8.0);

                let eigen_naam = self
                    .chat
                    .timeline()
                    .nicknames
                    .get(&self.chat.me())
                    .cloned()
                    .unwrap_or_else(|| self.cfg.display_name.clone());

                ui.horizontal(|ui| {
                    ui.colored_label(GROEN, "\u{25CF}");
                    ui.label(
                        egui::RichText::new(eigen_naam)
                            .strong()
                            .color(kleur_van(self.identity.peer_id)),
                    );
                    ui.weak("(jij)");
                });
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);

                if self.rows.is_empty() {
                    ui.weak("Nog geen peers ingesteld.");
                    ui.small("Zet de tailnet-adressen van de anderen in config.toml.");
                }

                for row in &self.rows {
                    let naam = row
                        .peer_id
                        .and_then(|id| self.chat.timeline().nicknames.get(&id))
                        .cloned()
                        .unwrap_or_else(|| row.label.clone());
                    peer_row(ui, row, &naam);
                    ui.add_space(6.0);
                }
            });
    }

    fn statusbalk(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("statusbalk").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.small(format!("id {}", &self.identity.peer_id.to_string()[..8]));
                ui.separator();
                ui.small(format!("poort {}", self.cfg.control_port));
                ui.separator();
                ui.small(format!("{} berichten", self.chat.timeline().messages.len()));
                if self.chat.ongelezen > 0 {
                    ui.small(
                        egui::RichText::new(format!("{} nieuw", self.chat.ongelezen)).color(GROEN),
                    );
                }
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
                if let Some(err) = self.fout.clone() {
                    ui.separator();
                    if ui
                        .add(egui::Label::new(
                            egui::RichText::new(format!("⚠ {err}")).color(ROOD),
                        ))
                        .on_hover_text("klik om te verbergen")
                        .clicked()
                    {
                        self.fout = None;
                    }
                }
            });
        });
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
                    let breedte = ui.available_width() - 90.0;

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
                        self.versturen_klaar();
                        veld.request_focus();
                    }
                });
                ui.add_space(6.0);
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            let mut te_bewerken: Option<(OpId, String)> = None;
            let mut te_verwijderen: Option<OpId> = None;

            egui::ScrollArea::vertical()
                .stick_to_bottom(self.scroll_naar_beneden)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if self.chat.timeline().messages.is_empty() {
                        ui.add_space(20.0);
                        ui.vertical_centered(|ui| {
                            ui.weak("Nog geen berichten.");
                            ui.small(
                                "Wat je hier typt wordt bewaard en komt aan zodra de \
                                 anderen online zijn.",
                            );
                        });
                    }

                    let mij = self.chat.me();
                    let nicks = self.chat.timeline().nicknames.clone();

                    for msg in &self.chat.timeline().messages {
                        let naam = nicks
                            .get(&msg.author)
                            .cloned()
                            .unwrap_or_else(|| korte_id(msg.author));

                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(naam)
                                    .strong()
                                    .color(kleur_van(msg.author)),
                            );
                            ui.small(egui::RichText::new(tijd(msg.created_at)).weak());
                            if msg.edited {
                                ui.small(egui::RichText::new("(bewerkt)").weak());
                            }

                            if msg.author == mij {
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
                let r = self.chat.verwijder_bericht(id);
                self.meld(r);
                self.chat.refresh();
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

fn peer_row(ui: &mut egui::Ui, row: &PeerRow, naam: &str) {
    let (color, text) = describe(&row.status);

    ui.horizontal(|ui| {
        ui.colored_label(color, "\u{25CF}");
        ui.vertical(|ui| {
            let naam_kleur = row
                .peer_id
                .map(kleur_van)
                .unwrap_or(ui.visuals().text_color());
            ui.label(egui::RichText::new(naam).strong().color(naam_kleur));
            ui.small(egui::RichText::new(text).color(color));
        });
    })
    .response
    .on_hover_text(&row.address);
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
    let tint = u16::from(b[0]) << 8 | u16::from(b[1]);
    let hoek = f32::from(tint) / 65535.0;
    // Vaste verzadiging en helderheid: elke peer krijgt een goed leesbare kleur,
    // ook in een donker thema.
    let (r, g, bl) = hsv_naar_rgb(hoek * 360.0, 0.55, 0.95);
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

fn korte_id(peer: PeerId) -> String {
    peer.to_string()[..8].to_string()
}

fn tijd(millis: i64) -> String {
    use chrono::{Local, TimeZone};
    match Local.timestamp_millis_opt(millis).single() {
        Some(t) => t.format("%H:%M").to_string(),
        None => String::new(),
    }
}
