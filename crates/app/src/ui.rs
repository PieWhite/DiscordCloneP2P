//! De UI. Bewust functioneel gehouden — styling volgt in een latere fase.
//!
//! De UI-thread doet geen netwerkwerk en houdt geen locks vast. Alles komt binnen via
//! `MeshEvent` en gaat eruit via `MeshCommand`.

use crate::config::{Config, Identity};
use eframe::egui;
use fitcom_net::{MeshEvent, MeshHandle, PeerStatus};
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
    /// Moet in leven blijven zolang de app draait, anders stopt de netwerklaag.
    _runtime: tokio::runtime::Runtime,
    rows: Vec<PeerRow>,
    /// Laatste fout bij het wegschrijven van de config, om te tonen in plaats van te slikken.
    config_error: Option<String>,
}

struct PeerRow {
    label: String,
    address: String,
    status: PeerStatus,
}

impl App {
    pub fn new(
        cfg: Config,
        identity: Identity,
        config_path: PathBuf,
        data_dir: PathBuf,
        mesh: MeshHandle,
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
            _runtime: runtime,
            rows,
            config_error: None,
        }
    }

    fn drain_events(&mut self) {
        while let Ok(event) = self.mesh.events.try_recv() {
            match event {
                MeshEvent::Status { target, status } => {
                    if let Some(row) = self.rows.get_mut(target) {
                        // Zodra de peer zichzelf voorstelt gebruiken we zijn eigen naam
                        // in plaats van het label uit onze config.
                        if let PeerStatus::Online { display_name, .. } = &status {
                            row.label = display_name.clone();
                        }
                        row.status = status;
                    }
                }
                MeshEvent::LearnedIdentity { target, peer_id } => {
                    if let Some(p) = self.cfg.peers.get_mut(target) {
                        p.known_id = Some(peer_id);
                        // Vastleggen zodat we bij een volgende start kunnen zien of er
                        // echt dezelfde peer achter dit adres zit.
                        if let Err(e) = self.cfg.save(&self.config_path) {
                            self.config_error = Some(format!("{e:#}"));
                        }
                    }
                }
                MeshEvent::Message { from, msg } => {
                    // Fase 2 hangt chat hier aan. Nu alleen loggen zodat we tijdens het
                    // testen zien dat de verbinding echt tweerichtingsverkeer is.
                    tracing::debug!(peer = ?from, ?msg, "bericht ontvangen");
                }
            }
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_events();
        ctx.request_repaint_after(IDLE_REPAINT);

        egui::SidePanel::left("deelnemers")
            .resizable(false)
            .exact_width(260.0)
            .show(ctx, |ui| {
                ui.add_space(8.0);
                ui.heading("Deelnemers");
                ui.add_space(8.0);

                // Jezelf: altijd bovenaan, altijd "online".
                ui.horizontal(|ui| {
                    ui.colored_label(egui::Color32::from_rgb(80, 200, 120), "\u{25CF}");
                    ui.label(egui::RichText::new(&self.cfg.display_name).strong());
                    ui.weak("(jij)");
                });
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);

                if self.rows.is_empty() {
                    ui.weak("Nog geen peers ingesteld.");
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(
                            "Zet de tailnet-adressen van de anderen in config.toml.",
                        )
                        .small(),
                    );
                }

                for row in &self.rows {
                    peer_row(ui, row);
                    ui.add_space(6.0);
                }
            });

        egui::TopBottomPanel::bottom("statusbalk").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.small(format!("id {}", &self.identity.peer_id.to_string()[..8]));
                ui.separator();
                ui.small(format!("poort {}", self.cfg.control_port));
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
                if let Some(err) = &self.config_error {
                    ui.separator();
                    ui.colored_label(
                        egui::Color32::from_rgb(220, 90, 90),
                        format!("config: {err}"),
                    );
                }
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(12.0);
            ui.heading("Chat");
            ui.add_space(8.0);
            ui.weak("Komt in fase 2. De netwerklaag eronder draait al.");
        });
    }
}

fn peer_row(ui: &mut egui::Ui, row: &PeerRow) {
    let (color, text) = describe(&row.status);

    ui.horizontal(|ui| {
        ui.colored_label(color, "\u{25CF}");
        ui.vertical(|ui| {
            ui.label(egui::RichText::new(&row.label).strong());
            ui.small(egui::RichText::new(text).color(color));
        });
    })
    .response
    .on_hover_text(&row.address);
}

fn describe(status: &PeerStatus) -> (egui::Color32, String) {
    const GROEN: egui::Color32 = egui::Color32::from_rgb(80, 200, 120);
    const GEEL: egui::Color32 = egui::Color32::from_rgb(220, 180, 70);
    const GRIJS: egui::Color32 = egui::Color32::from_rgb(130, 130, 130);
    const ROOD: egui::Color32 = egui::Color32::from_rgb(220, 90, 90);

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
