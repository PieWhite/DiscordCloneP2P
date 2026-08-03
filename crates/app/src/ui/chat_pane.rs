//! De chat-kern: berichten-/bestandenlijst en de invoerbalk met @tag-autocomplete.
//! Gedeeld door de Kanalen- en (vanaf een latere fase) de DM-weergave — beide werken
//! op `App::actief_kanaal`, dat bij het wisselen van weergave al op het juiste kanaal
//! staat (zie `App::wissel_view`), dus dit hoeft zelf geen kanaal-parameter te dragen.

use super::{grootte_tekst, theme, widgets};
use crate::engine::{FileView, UiCommand};
use crate::files::{hash_bestandsnaam, is_afbeelding, DownloadStatus};
use crate::tags;
use eframe::egui;
use fitcom_proto::OpId;
use fitcom_store::Message;
use std::sync::Arc;

impl super::App {
    pub(super) fn chat_pane(&mut self, ctx: &egui::Context) {
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

                let (knop_tekst, knop_symbool) = if self.bewerkt.is_some() {
                    ("opslaan", "\u{2713}")
                } else {
                    ("versturen", "\u{27A1}")
                };

                // Links het "+"-icoon voor een bijlage, rechts versturen — zoals
                // Discord's invoerbalk, in plaats van de tekstknoppen die dit
                // voorheen waren.
                let (mut output, verstuur_geklikt, bestand_geklikt) = ui
                    .horizontal(|ui| {
                        let bestand = widgets::icon_button(ui, "\u{2795}")
                            .on_hover_text("bestand delen")
                            .clicked();
                        let breedte = (ui.available_width() - 40.0).max(80.0);
                        let output = egui::TextEdit::multiline(&mut self.invoer)
                            .desired_rows(1)
                            .desired_width(breedte)
                            .hint_text(
                                "bericht… (shift+enter voor een nieuwe regel, sleep of plak \
                                 een bestand)",
                            )
                            .show(ui);
                        let geklikt = widgets::icon_button(ui, knop_symbool)
                            .on_hover_text(knop_tekst)
                            .clicked();
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
                                    let blok = ui.scope(|ui| {
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                egui::RichText::new(self.naam_van(msg.author))
                                                    .strong()
                                                    .color(widgets::kleur_van(msg.author)),
                                            );
                                            ui.small(
                                                egui::RichText::new(tijd(msg.created_at)).weak(),
                                            );
                                            if msg.edited {
                                                ui.small(egui::RichText::new("(bewerkt)").weak());
                                            }
                                        });

                                        toon_tekst(ui, &msg.body);
                                    });

                                    // Alleen tonen bij hover: anders staat er achter elk
                                    // bericht permanent "bewerk verwijder", wat bij een
                                    // lange geschiedenis alleen maar ruis is.
                                    if msg.author == self.mij && blok.response.hovered() {
                                        ui.scope_builder(
                                            egui::UiBuilder::new().max_rect(
                                                egui::Rect::from_min_size(
                                                    blok.response.rect.right_top()
                                                        - egui::vec2(96.0, 0.0),
                                                    egui::vec2(96.0, 18.0),
                                                ),
                                            ),
                                            |ui| {
                                                ui.with_layout(
                                                    egui::Layout::right_to_left(
                                                        egui::Align::Center,
                                                    ),
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
                                            },
                                        );
                                    }
                                };

                                // Een tag naar jezelf springt eruit met een gekleurd
                                // kader — subtiel genoeg om niet als foutmelding te
                                // lezen, opvallend genoeg om in een lange geschiedenis
                                // terug te vinden.
                                if getagd {
                                    egui::Frame::group(ui.style())
                                        .fill(theme::MENTION_BG)
                                        .stroke(egui::Stroke::new(1.0_f32, theme::MENTION_BORDER))
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
                                    .color(widgets::kleur_van(f.author)),
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
                                                                    theme::STATUS_ONLINE,
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
                                                                .color(theme::STATUS_DND),
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
