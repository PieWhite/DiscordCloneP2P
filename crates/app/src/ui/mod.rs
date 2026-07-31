//! De UI: het Discord-achtige ontwerp uit `docs/` verdeeld over deze weergave-modules.
//!
//! Puur een weergave: leest een momentopname van de motor en stuurt commando's terug.
//! Er wordt hier geen enkele beslissing genomen over netwerk of opslag, en er staat
//! geen state in die verloren gaat als het venster even niet tekent.

pub mod channels;
pub mod chat_pane;
pub mod dms;
pub mod modals;
pub mod rail;
pub mod settings;
pub mod statusbar;
pub mod stream_strip;
pub mod theme;
pub mod titlebar;
pub mod widgets;

use settings::SettingsTab;

use crate::engine::{self, EngineHandle, Snapshot, UiCommand};
use crate::tray;
use eframe::egui;
use fitcom_proto::{Channel, OpId, PeerId, TopicId};
use fitcom_video::Bron;
use std::collections::HashMap;
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

/// Welke van de elkaar uitsluitende hoofdweergaven getoond wordt, gestuurd door de
/// icoonrail (`ui/rail.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppView {
    Channels,
    Dms,
    Settings,
}

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
    /// Welke hoofdweergave de icoonrail net toont. Bepaalt alleen de lay-out, niet
    /// welk kanaal actief is — zie `laatste_niet_dm_kanaal`/`laatste_dm` hieronder.
    view: AppView,
    /// Laatst bekeken niet-DM-kanaal (Algemeen of een subkanaal). Zo verlies je je
    /// plek niet als je tijdelijk naar de DM-weergave wisselt en terugkomt.
    laatste_niet_dm_kanaal: Channel,
    /// Laatst geopende DM-gesprek, indien er ooit één geopend is. `None` betekent: de
    /// DM-weergave toont een lege "kies een gesprek"-staat, geen geforceerde keuze.
    laatste_dm: Option<PeerId>,
    /// Welke tab van de Instellingen-weergave getoond wordt.
    settings_tab: SettingsTab,
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
    /// Verversingsfrequenties van de schermen van deze machine, één keer opgehaald bij
    /// het openen. Alleen om te tonen wat het gevraagde tempo hier werkelijk wordt —
    /// alleen hele delers van de verversing geven gelijkmatig beeld.
    scherm_hz: Vec<u32>,
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
            view: AppView::Channels,
            laatste_niet_dm_kanaal: Channel::GENERAL,
            laatste_dm: None,
            settings_tab: SettingsTab::Account,
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
        // Bijhouden welk kanaal je in elke weergave het laatst bekeek, ook als dit
        // vroegtijdig terugkeert omdat het al het actieve kanaal is — anders zou een
        // eerste `wissel_view` naar Dms nooit een `laatste_dm` vinden als je toevallig
        // al op die DM zat vóór het wisselen van weergave.
        match kanaal.dm_peer() {
            Some(peer) => self.laatste_dm = Some(peer),
            None => self.laatste_niet_dm_kanaal = kanaal,
        }

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

    /// Wisselt van hoofdweergave (icoonrail) en herstelt daarbij waar je was: de
    /// Kanalen-weergave opent op het laatst bekeken niet-DM-kanaal, de DM-weergave op
    /// het laatst geopende gesprek — of laat het actieve kanaal met rust als er nog
    /// geen DM is geweest, zodat er geen gesprekspartner wordt afgedwongen.
    fn wissel_view(&mut self, view: AppView) {
        self.view = view;
        match view {
            AppView::Channels => self.wissel_kanaal(self.laatste_niet_dm_kanaal),
            AppView::Dms => {
                if let Some(peer) = self.laatste_dm {
                    self.wissel_kanaal(Channel::dm(peer));
                }
            }
            // Instellingen heeft geen "actief kanaal" om te herstellen — de rail vult
            // hier zelf `self.instellingen` (zie `App::open_instellingen`).
            AppView::Settings => {}
        }
    }

    /// Eigen avatar, naam, aanwezigheidsstatus en de "niet storen"-toggle — identiek
    /// onderaan zowel de Kanalen- als de DM-zijbalk (`ui/channels.rs`, `ui/dms.rs`),
    /// dus hier één keer getekend in plaats van tweemaal gedupliceerd. Levert de
    /// nieuwe "niet storen"-waarde als die net gewijzigd is.
    fn eigen_mini_kaart(&mut self, ui: &mut egui::Ui) -> Option<bool> {
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
            let avatar = widgets::avatar_square(ui, &widgets::initialen(&eigen), eigen_kleur, 32.0);
            widgets::status_badge(ui.painter(), avatar.rect, status_kleur, theme::BG_SIDEBAR);
            ui.vertical(|ui| {
                ui.label(egui::RichText::new(&eigen).strong().color(eigen_kleur));
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

        let mut niet_storen_wijziging = None;
        ui.horizontal(|ui| {
            ui.small("niet storen");
            let mut niet_storen = self.snap.niet_storen;
            if widgets::toggle_switch(ui, &mut niet_storen).changed() {
                niet_storen_wijziging = Some(niet_storen);
            }
        });
        if self.snap.voice.actief {
            let niveau = if self.snap.voice.muted {
                0.0
            } else {
                self.snap.voice.eigen_niveau
            };
            widgets::niveaubalk(ui, niveau);
        }
        niet_storen_wijziging
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

        titlebar::resize_randen(ctx);
        titlebar::titlebar(ctx);
        rail::rail(self, ctx);

        self.bronkeuze_venster(ctx);
        self.bevestig_verwijder_afbeeldingen_venster(ctx);
        self.kanaal_hernoemen_venster(ctx);
        self.bevestig_verwijder_kanaal_venster(ctx);
        self.update_beschikbaar_venster(ctx);
        self.statusbalk(ctx);
        self.overzicht_strook(ctx);
        match self.view {
            AppView::Channels => self.channels_view(ctx),
            AppView::Dms => self.dms_view(ctx),
            AppView::Settings => self.settings_view(ctx),
        }
    }
}

impl App {
    /// Wat wij delen, plus de knop om er iets bij te doen.
    ///
    /// Levert het commando en "open het keuzemenu" terug in plaats van ze meteen uit
    /// te voeren: binnen de paneelsluiting is `self` al onveranderlijk geleend.
    fn deel_bediening(&self, ui: &mut egui::Ui) -> (Option<UiCommand>, bool) {
        let mut cmd = None;

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
                    theme::STATUS_ONLINE
                } else {
                    theme::STATUS_OFFLINE
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

    /// Levert het commando dat de gebruiker aanklikte terug in plaats van het meteen
    /// te versturen: binnen de paneelsluiting is `self` al onveranderlijk geleend.
    fn voice_bediening(&self, ui: &mut egui::Ui) -> Option<UiCommand> {
        let v = &self.snap.voice;

        if !v.actief {
            if self.snap.peers.iter().any(|p| p.in_voice) {
                ui.small(
                    egui::RichText::new("er is een gesprek bezig").color(theme::STATUS_ONLINE),
                );
                ui.add_space(2.0);
            }
            ui.label(egui::RichText::new("Deelnemen aan voicechannel").strong());
            ui.add_space(4.0);
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

    /// Vult een verse bewerkkopie van de huidige video-instellingen. Aangeroepen door
    /// het tandwiel in de icoonrail (`ui/rail.rs`) bij het openen van de Instellingen-
    /// weergave, en door `ui/settings.rs` zelf na "Toepassen"/"Annuleren" op het
    /// Video-tabblad om een verse kopie te tonen.
    fn open_instellingen(&mut self) {
        // Eén keer opsommen, niet elke frame: dit vraagt Windows naar alle schermen.
        let mut scherm_hz: Vec<u32> = fitcom_video::beschikbare_bronnen()
            .unwrap_or_default()
            .iter()
            .filter(|b| b.soort == fitcom_video::BronSoort::Monitor)
            .filter_map(fitcom_video::capture::verversing_van)
            .collect();
        scherm_hz.sort_unstable();
        scherm_hz.dedup();

        self.instellingen = Some(VideoConcept {
            codec: self.snap.video.codec.clone(),
            fps: self.snap.video.fps,
            bitrate_mbit: self.snap.video.bitrate as f32 / 1_000_000.0,
            scherm_hz,
        });
    }
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
