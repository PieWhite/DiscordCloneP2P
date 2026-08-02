//! Kleine, herbruikbare bouwstenen die de mockup steeds opnieuw gebruikt en waar egui
//! zelf geen kant-en-klaar widget voor heeft: vierkante avatars met initialen,
//! statusstippen, microlabels, een toggle-switch en pil-vormige tabs/knoppen.
//!
//! Ook de bestaande kleur-/tekenhulpjes wonen hier: `niveaubalk`, `peer_row`,
//! `describe`, `kleur_van` en `hsv_naar_rgb` zijn gedeelde kleur- en teken-logica,
//! geen weergave-specifieke code, en horen dus niet in `mod.rs` thuis.

use super::theme;
use eframe::egui;
use egui::{Color32, Sense, Stroke, Vec2};
use fitcom_net::PeerStatus;
use fitcom_proto::PeerId;

use crate::engine::PeerView;

/// Initialen uit een naam voor gebruik in een `avatar_square`: eerste letter van
/// hoogstens de eerste twee woorden, in hoofdletters — zelfde conventie als de mockup.
pub fn initialen(naam: &str) -> String {
    naam.split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .flat_map(char::to_uppercase)
        .collect()
}

/// Rond-afgerond vierkant avatar met initialen erin, zoals de mockup's peer-/
/// gebruikersavatars.
pub fn avatar_square(
    ui: &mut egui::Ui,
    initials: &str,
    color: Color32,
    size: f32,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect_filled(rect, theme::ROUNDING, color);
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            initials,
            egui::FontId::proportional(size * 0.4),
            theme::ON_ACCENT,
        );
    }
    response
}

/// Statusstip als badge rechtsonder aan een net getekend avatar-vierkant, met een
/// kleine "uitsparing" in de achtergrondkleur eromheen zodat hij los oogt van het
/// avatar erachter — zoals de mockup's ledenlijst.
pub fn status_badge(
    painter: &egui::Painter,
    avatar_rect: egui::Rect,
    color: Color32,
    achtergrond: Color32,
) {
    let r = (avatar_rect.width() * 0.16).max(3.0);
    let center = avatar_rect.right_bottom() - Vec2::splat(r * 0.7);
    painter.circle_filled(center, r * 1.7, achtergrond);
    painter.circle_filled(center, r, color);
}

/// 11px hoofdletter-microlabel voor sectiekopjes, zoals de mockup's "TEXT CHANNELS".
pub fn section_label(ui: &mut egui::Ui, text: &str) {
    ui.label(
        egui::RichText::new(text.to_uppercase())
            .size(11.0)
            .color(theme::TEXT_MUTED)
            .strong(),
    );
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let lerp = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(lerp(a.r(), b.r()), lerp(a.g(), b.g()), lerp(a.b(), b.b()))
}

/// Aan/uit-schakelaar: 36×20 afgeronde track, geanimeerd tussen uit (donker) en aan
/// (accentkleur), met een schuivend rond knopje.
pub fn toggle_switch(ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
    let size = Vec2::new(36.0, 20.0);
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let t = ui.ctx().animate_bool(response.id, *on);
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let track = lerp_color(theme::BG_DEEPEST, theme::ACCENT, t);
        painter.rect_filled(rect, theme::ROUNDING_PILL, track);
        let knop_x = egui::lerp((rect.left() + 10.0)..=(rect.right() - 10.0), t);
        painter.circle_filled(
            egui::pos2(knop_x, rect.center().y),
            8.0,
            theme::TEXT_PRIMARY,
        );
    }
    response
}

/// Pil-vormige tab/knop: instellingen-tabs en een actief-kanaal-rij delen deze stijl —
/// accent-getint en -omrand als geselecteerd, anders gedempt.
pub fn pill_tab(ui: &mut egui::Ui, selected: bool, text: &str) -> egui::Response {
    let kleur = if selected {
        theme::ACCENT
    } else {
        theme::TEXT_MUTED
    };
    let knop = egui::Button::new(egui::RichText::new(text).size(13.0).color(kleur))
        .corner_radius(theme::ROUNDING_PILL)
        .fill(if selected {
            theme::ACCENT.gamma_multiply(0.22)
        } else {
            Color32::TRANSPARENT
        })
        .stroke(if selected {
            Stroke::new(1.0_f32, theme::ACCENT)
        } else {
            Stroke::NONE
        });
    ui.add(knop)
}

/// Vierkante 40×40 icoonknop voor de rail: accent-getint en -omrand als geselecteerd,
/// anders een gedempt vierkant — zoals de mockup's DM-/server-/instellingen-knoppen.
pub fn rail_button(ui: &mut egui::Ui, selected: bool, text: &str) -> egui::Response {
    let kleur = if selected {
        theme::ACCENT
    } else {
        theme::TEXT_MUTED
    };
    let knop = egui::Button::new(egui::RichText::new(text).size(13.0).strong().color(kleur))
        .min_size(Vec2::splat(40.0))
        .corner_radius(theme::ROUNDING)
        .fill(if selected {
            theme::ACCENT.gamma_multiply(0.22)
        } else {
            theme::BG_SIDEBAR
        })
        .stroke(if selected {
            Stroke::new(1.0_f32, theme::ACCENT)
        } else {
            Stroke::NONE
        });
    ui.add(knop)
}

/// Smalle balk die meebeweegt met hoe hard iemand praat.
///
/// Logaritmisch geschaald: spraak zit qua energie laag ten opzichte van het maximum,
/// en lineair zou de balk nauwelijks bewegen.
pub fn niveaubalk(ui: &mut egui::Ui, niveau: f32) {
    let deel = if niveau <= 0.0005 {
        0.0
    } else {
        ((niveau.log10() * 20.0 + 60.0) / 60.0).clamp(0.0, 1.0)
    };

    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width().min(200.0), 4.0),
        Sense::hover(),
    );
    let schilder = ui.painter();
    schilder.rect_filled(rect, 2.0, theme::BG_DEEPEST);
    if deel > 0.0 {
        let mut gevuld = rect;
        gevuld.set_width(rect.width() * deel);
        schilder.rect_filled(gevuld, 2.0, theme::STATUS_ONLINE);
    }
}

/// Peer-rij met avatar (initialen + statusbadge), naam en statustekst.
pub fn peer_row(ui: &mut egui::Ui, p: &PeerView, naam: &str) {
    let (kleur, tekst) = describe(&p.status);

    ui.horizontal(|ui| {
        let avatar_kleur = p.peer_id.map(kleur_van).unwrap_or(theme::ACCENT);
        let (avatar_rect, _) = {
            let resp = avatar_square(ui, &initialen(naam), avatar_kleur, 28.0);
            (resp.rect, resp)
        };
        status_badge(ui.painter(), avatar_rect, kleur, theme::BG_SIDEBAR);

        ui.vertical(|ui| {
            let naam_kleur = p
                .peer_id
                .map(kleur_van)
                .unwrap_or(ui.visuals().text_color());
            ui.label(egui::RichText::new(naam).strong().color(naam_kleur));
            ui.small(egui::RichText::new(tekst).color(kleur));
        });
    })
    .response
    .on_hover_text(&p.address);
}

pub fn describe(status: &PeerStatus) -> (Color32, String) {
    match status {
        PeerStatus::Online { .. } => (theme::STATUS_ONLINE, "online".into()),
        // Apart van "offline", want het is geen storing en wachten helpt niet: één van
        // beiden moet bijwerken. Als dit er als gewoon offline uitziet ga je het
        // netwerk zitten uitzoeken terwijl er niets mis is met het netwerk.
        PeerStatus::VersionMismatch { theirs, ours } => (
            theme::STATUS_OFFLINE,
            format!("andere versie (protocol {theirs}, wij {ours})"),
        ),
        _ => (theme::STATUS_OFFLINE, "offline".into()),
    }
}

/// Stabiele kleur per peer, zodat je in de chat aan de kleur ziet wie wat zei.
///
/// N-agnostisch met opzet: een continue hue-hoek uit een hash van de peer-bytes, geen
/// vaste lijst met kleuren — die zou bij meer peers dan lijst-items gaan botsen.
pub fn kleur_van(peer: PeerId) -> Color32 {
    let b = peer.as_bytes();
    let tint = (u16::from(b[0]) << 8) | u16::from(b[1]);
    let hoek = f32::from(tint) / 65535.0 * 360.0;
    // Vaste verzadiging en helderheid, bijgesteld richting de mockup's zachtere
    // pastelkleuren — nog steeds goed leesbaar in het donkere thema.
    let (r, g, bl) = hsv_naar_rgb(hoek, 0.6, 0.88);
    Color32::from_rgb(r, g, bl)
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
