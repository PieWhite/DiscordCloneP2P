//! Strook boven de chat met een levend, verkleind beeld van elke stream die we
//! bekijken — logica ongewijzigd, alleen hierheen verhuisd zodat `mod.rs` niet elk
//! onderdeel van de app hoeft te bevatten. De stijl komt gratis mee uit `theme::apply`.

use eframe::egui;
use fitcom_proto::PeerId;
use fitcom_video::Miniatuur;
use std::collections::HashSet;
use std::sync::Arc;

impl super::App {
    /// Bestaat om niet tussen meerdere losse kijkvensters te hoeven zoeken zodra er
    /// meer dan één tegelijk open staat — "meerdere inkomende streams tegelijk
    /// bekijken" uit fase 5. Toont niets zolang er niets bekeken wordt, net als de rest
    /// van screenshare pas iets kost zodra het ergens toe dient.
    pub(super) fn overzicht_strook(&mut self, ctx: &egui::Context) {
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
