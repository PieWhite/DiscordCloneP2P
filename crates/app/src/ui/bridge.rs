//! The three feeds between the engine and the window, and the two data entities the views
//! read the fast ones from.
//!
//! Each feed is a tokio task on the app's runtime that wakes on the engine's snapshot (or
//! its own 4 Hz / 2 Hz tick, as before), works out what the window would see, and only
//! sends when that differs from what it sent last. The window side waits on a `watch`
//! receiver, so an idle app costs the window thread nothing at all.
//!
//! | Feed | Tempo | Carries |
//! |---|---|---|
//! | state | only on a real change | the whole `UiState`, compared with `PartialEq` |
//! | meters | 4 Hz, only on change | speaking level, the speaking verdict and RTT per peer |
//! | thumbnails | 2 Hz per watched stream | one GPUI image per strip tile |
//!
//! RTT and speaking level are not in `UiState` on purpose: they move every tick and would
//! defeat the comparison that keeps the state feed quiet.
//!
//! On Windows a fourth task feeds the in-game overlay (decision 53) from the state and meters
//! feeds above: who is in the call and who is speaking, by the same rule as the speaking
//! ring. It only wakes when one of those two sends, and only hands the overlay a new list
//! when the rows differ.

use super::state::UiState;
use super::Ui;
use gpui_kit::{Context, RenderImage, Window};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::watch;

/// Speaking level and RTT are the only things that move while the window is idle-ish.
/// 4 Hz is the rate the egui build repainted *everything* at; here it moves two numbers.
const METER_INTERVAL: Duration = Duration::from_millis(250);

/// The stream strip. `docs/OVERDRACHT.md` sets the budget at 2 fps and a small size.
const THUMBNAIL_INTERVAL: Duration = Duration::from_millis(500);

/// The level above which a still picture counts as speaking. Fixture mode only: the live
/// ring follows the audio layer's own verdict (`spreekt`), which is the VAD that decides
/// whether voice is sent at all, so the ring matches what the others hear.
pub const SPEAKING: f32 = 0.06;

/// How long the ring survives a tick without voice. The sender's VAD hangover already
/// smooths the gaps between words; this only bridges one sampled jitter-buffer gap
/// (`Frame::Stilte` mid-stream). Longer double-holds with that hangover and lags the end.
pub const SPEAK_HOLD: Duration = Duration::from_millis(300);

/// One peer's live figures.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct PeerMeter {
    /// `None` while the peer is not online.
    pub rtt: Option<u32>,
    pub level: f32,
    /// The verdict with the hold applied: what the speaking ring shows.
    #[serde(default)]
    pub speaking: bool,
}

/// What the meters feed carries, keyed by peer UUID string as in `PeerState::id`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MeterReadings {
    pub peers: HashMap<String, PeerMeter>,
    pub self_level: f32,
    /// Already gated on joined-and-not-muted: while muted no timestamp is refreshed, so
    /// muting mid-sentence drops the ring at once instead of letting the hold keep it lit.
    pub self_speaking: bool,
}

/// The entity the meter-driven bits observe: speaking rings, RTT readouts, the input level
/// bar. Never observed by `AppView` itself, so a tick re-renders only those small views.
#[derive(Debug, Default, Clone)]
pub struct Meters {
    readings: MeterReadings,
}

impl Meters {
    pub fn rtt(&self, peer: &str) -> Option<u32> {
        self.readings.peers.get(peer).and_then(|m| m.rtt)
    }
    pub fn level(&self, peer: &str) -> f32 {
        self.readings.peers.get(peer).map_or(0.0, |m| m.level)
    }
    /// Is this peer speaking, with the hold applied? One rule for the voice panel and the
    /// member list (the audio layer's `spreekt`, `SPEAK_HOLD`).
    pub fn speaking(&self, peer: &str) -> bool {
        self.readings.peers.get(peer).is_some_and(|m| m.speaking)
    }
    /// Our own input level, 0..1, for the level bar in the audio settings.
    pub fn self_level(&self) -> f32 {
        self.readings.self_level
    }
    /// Am I speaking? Only true while joined and not muted.
    pub fn self_speaking(&self) -> bool {
        self.readings.self_speaking
    }
    /// The highest RTT among `peers`, for the status bar's call line.
    pub fn worst_rtt<'a>(&self, peers: impl IntoIterator<Item = &'a str>) -> Option<u32> {
        peers.into_iter().filter_map(|p| self.rtt(p)).max()
    }

    pub(crate) fn set(&mut self, readings: MeterReadings, cx: &mut Context<Self>) {
        if self.readings != readings {
            self.readings = readings;
            cx.notify();
        }
    }
}

pub type ThumbMap = HashMap<String, Arc<RenderImage>>;

/// The strip's images, one per tile key: `self-<stream_id>` for what we send ourselves,
/// `<owner-uuid>-<stream_id>` for a stream we watch. Observed by the strip only.
#[derive(Default)]
pub struct Thumbnails {
    images: Arc<ThumbMap>,
}

impl Thumbnails {
    pub fn get(&self, key: &str) -> Option<Arc<RenderImage>> {
        self.images.get(key).cloned()
    }

    /// The tile key for one of our own streams.
    pub fn own_key(stream_id: u32) -> String {
        format!("self-{stream_id}")
    }

    /// The tile key for a stream of `owner` (a peer UUID string).
    pub fn peer_key(owner: &str, stream_id: u32) -> String {
        format!("{owner}-{stream_id}")
    }

    /// Takes the new set and evicts every image that is not in it any more from the sprite
    /// atlas. Without that, 2 Hz of fresh frames would grow GPU memory for as long as a
    /// stream is watched.
    pub(crate) fn set(&mut self, next: Arc<ThumbMap>, window: &mut Window, cx: &mut Context<Self>) {
        if Arc::ptr_eq(&self.images, &next) {
            return;
        }
        for (key, old) in self.images.iter() {
            if next.get(key).is_none_or(|new| new.id != old.id) {
                let _ = window.drop_image(old.clone());
            }
        }
        self.images = next;
        cx.notify();
    }
}

/// The window's end of the three feeds.
pub struct Feeds {
    pub state: watch::Receiver<Arc<UiState>>,
    pub meters: watch::Receiver<MeterReadings>,
    pub thumbs: watch::Receiver<Arc<ThumbMap>>,
}

impl Feeds {
    /// Feeds that never move: the fixture mode's. The senders go away at once; a receiver
    /// keeps the last value, and the window's loop on it simply ends.
    #[cfg(debug_assertions)]
    pub fn fixed(state: UiState, meters: MeterReadings, thumbs: ThumbMap) -> Self {
        Feeds {
            state: watch::channel(Arc::new(state)).1,
            meters: watch::channel(meters).1,
            thumbs: watch::channel(Arc::new(thumbs)).1,
        }
    }
}

/// Starts the three producers on the app's runtime.
pub fn spawn_live(ui: &Arc<Ui>) -> Feeds {
    let (state_tx, state) = watch::channel(Arc::new(ui.state()));
    let (meters_tx, meters) = watch::channel(MeterReadings::default());
    let (thumbs_tx, thumbs) = watch::channel(Arc::new(ThumbMap::new()));

    let u = ui.clone();
    ui.runtime.spawn(async move {
        let mut rx = u.engine.snapshot.clone();
        loop {
            if rx.changed().await.is_err() {
                return;
            }
            let snap = rx.borrow_and_update().clone();
            let next = u.build_state(&snap);
            // The engine publishes on a fixed tick, so most of these equal the last one.
            // Comparing here is what keeps an idle window at zero wake-ups: volatile
            // figures live in the meters feed precisely so they cannot defeat this.
            state_tx.send_if_modified(|current| {
                if **current == next {
                    false
                } else {
                    *current = Arc::new(next);
                    true
                }
            });
        }
    });

    let rx = ui.engine.snapshot.clone();
    ui.runtime.spawn(async move {
        let mut ticker = tokio::time::interval(METER_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut speak_at: HashMap<String, Instant> = HashMap::new();
        loop {
            ticker.tick().await;
            let snap = rx.borrow().clone();
            let now = Instant::now();
            let mut held = |key: &str, now_speaking: bool, gate: bool| -> bool {
                if !gate {
                    return false;
                }
                if now_speaking {
                    speak_at.insert(key.to_string(), now);
                    return true;
                }
                speak_at
                    .get(key)
                    .is_some_and(|t| now.duration_since(*t) < SPEAK_HOLD)
            };

            let mut readings = MeterReadings::default();
            for p in &snap.peers {
                let Some(id) = p.peer_id else { continue };
                let id = id.to_string();
                let rtt = match &p.status {
                    fitcom_net::PeerStatus::Online { rtt_ms, .. } => Some(*rtt_ms),
                    _ => None,
                };
                let speaking = held(&id, p.spreekt, true);
                readings.peers.insert(
                    id,
                    PeerMeter {
                        rtt,
                        level: p.niveau,
                        speaking,
                    },
                );
            }
            readings.self_level = snap.voice.eigen_niveau;
            readings.self_speaking = held(
                "@self",
                snap.voice.eigen_spreekt,
                snap.voice.actief && !snap.voice.muted,
            );

            // With everyone offline and no call running this settles into one constant
            // value, so a truly idle app sends nothing at all after the first tick.
            meters_tx.send_if_modified(|current| {
                if *current == readings {
                    false
                } else {
                    *current = readings;
                    true
                }
            });
        }
    });

    let rx = ui.engine.snapshot.clone();
    ui.runtime.spawn(async move {
        let mut ticker = tokio::time::interval(THUMBNAIL_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // Which frame and which drawing each tile shows, so an unchanged tile costs nothing.
        let mut last: HashMap<String, (usize, usize)> = HashMap::new();
        let mut images: Arc<ThumbMap> = Arc::new(ThumbMap::new());
        loop {
            ticker.tick().await;
            let snap = rx.borrow().clone();

            // Wat wij zelf uitsturen hoort in dezelfde strook als wat we van een ander
            // bekijken, dus het loopt door dezelfde pomp. Alleen de sleutel verschilt: een
            // eigen stream heeft geen eigenaar, dus `self-` in plaats van een peer-id.
            let eigen = snap.eigen_streams.iter().filter(|s| !s.is_geluid).map(|s| {
                (
                    Thumbnails::own_key(s.stream_id),
                    s.miniatuur.as_ref(),
                    &s.tekening,
                )
            });
            let vreemd = snap
                .streams
                .iter()
                .filter(|s| !s.is_geluid && s.kijken)
                .map(|s| {
                    (
                        Thumbnails::peer_key(&s.eigenaar.to_string(), s.stream_id),
                        s.miniatuur.as_ref(),
                        &s.tekening,
                    )
                });

            let mut next: Option<ThumbMap> = None;
            let mut alive: Vec<String> = Vec::new();
            for (key, thumb, drawing) in eigen.chain(vreemd) {
                alive.push(key.clone());
                let Some(thumb) = thumb else { continue };
                // A new frame or a changed drawing: either one makes a new image.
                let shown = (
                    Arc::as_ptr(&thumb.data) as *const u8 as usize,
                    Arc::as_ptr(drawing) as usize,
                );
                if last.get(&key) == Some(&shown) {
                    continue;
                }
                let Some(image) = render_image(thumb, drawing) else {
                    continue;
                };
                last.insert(key.clone(), shown);
                next.get_or_insert_with(|| (*images).clone())
                    .insert(key, Arc::new(image));
            }

            // A stream nobody watches any more must not keep its image alive; the strip
            // is the only thing that ever asks for it.
            if alive.len() != last.len() || images.len() > alive.len() {
                last.retain(|k, _| alive.contains(k));
                let map = next.get_or_insert_with(|| (*images).clone());
                map.retain(|k, _| alive.contains(k));
            }

            if let Some(map) = next {
                images = Arc::new(map);
                let _ = thumbs_tx.send(images.clone());
            }
        }
    });

    #[cfg(windows)]
    spawn_overlay(ui, state.clone(), meters.clone());

    Feeds {
        state,
        meters,
        thumbs,
    }
}

/// The overlay's rows: ourselves first while joined, then every known peer in the call, in
/// the member list's order. Speaking is the ring's verdict, hold and mute included. Empty
/// when the overlay is off or we are not in a call.
#[cfg(windows)]
fn overlay_rows(s: &UiState, m: &MeterReadings) -> Vec<fitcom_video::spraaklaag::Spreker> {
    use fitcom_video::spraaklaag::Spreker;
    if !s.overlay.enabled || !s.voice.joined {
        return Vec::new();
    }
    // A name comes from another PC: no control, bidi or zero-width characters on screen.
    let name = |n: &str| crate::speelt::schoon(n).unwrap_or_else(|| "?".into());
    std::iter::once(Spreker {
        naam: name(&s.me.name),
        spreekt: m.self_speaking,
    })
    .chain(
        s.peers
            .iter()
            .filter(|p| !p.id.is_empty() && p.in_call)
            .map(|p| Spreker {
                naam: name(&p.name),
                spreekt: m.peers.get(&p.id).is_some_and(|x| x.speaking),
            }),
    )
    .collect()
}

/// The overlay exists only while it has rows: made on joining (with the overlay on), dropped
/// on leaving or switching it off, so outside a call it costs no thread and no window.
#[cfg(windows)]
fn spawn_overlay(
    ui: &Arc<Ui>,
    mut state: watch::Receiver<Arc<UiState>>,
    mut meters: watch::Receiver<MeterReadings>,
) {
    use super::theme::{self, c};
    use fitcom_video::spraaklaag::{Spraaklaag, Stijl};
    let rgb =
        |c: gpui_kit::Rgba| [c.r, c.g, c.b].map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u8);
    let stijl = Stijl {
        grond: rgb(theme::BACKDROP),
        grond_alfa: theme::BACKDROP.a,
        stip: rgb(c::LIFT),
        ring: rgb(theme::ONLINE),
        stil: rgb(theme::TEXT_MUTED),
        spreekt: rgb(theme::TEXT_STRONG),
        font: theme::INTER_MEDIUM,
        font_naam: "Inter Medium",
    };
    let u = ui.clone();
    ui.runtime.spawn(async move {
        let mut overlay: Option<Spraaklaag> = None;
        let mut shown = Vec::new();
        loop {
            let rows = overlay_rows(&state.borrow_and_update(), &meters.borrow_and_update());
            if rows.is_empty() {
                overlay = None;
                shown.clear();
            } else {
                if overlay.is_none() {
                    // The position as the engine has it now; after this the overlay thread
                    // keeps track itself and reports a drag back.
                    let cfg = u.engine.snapshot.borrow().overlay.clone();
                    let commands = u.engine.commands.clone();
                    let moved = move |x, y| {
                        let _ = commands.try_send(crate::engine::UiCommand::ZetOverlayPlek(x, y));
                    };
                    match Spraaklaag::start(cfg.x.zip(cfg.y), stijl, moved) {
                        Ok(o) => overlay = Some(o),
                        Err(e) => tracing::warn!(error = %format!("{e:#}"), "no overlay"),
                    }
                    shown.clear();
                }
                if let Some(o) = overlay.as_ref().filter(|_| rows != shown) {
                    o.toon(rows.clone());
                    shown = rows;
                }
            }
            tokio::select! {
                r = state.changed() => if r.is_err() { return },
                r = meters.changed() => if r.is_err() { return },
            }
        }
    });
}

/// The decoder hands us BGRA and a GPUI image is BGRA, so the bytes go across as they are:
/// no PNG, no channel swap. Only the alpha byte is fixed: it means nothing on the way here
/// (Media Foundation's RGB32 leaves it at 0), and an image honours it, so a camera tile
/// came out transparent. A thumbnail is opaque by definition.
///
/// Lines drawn on the screen (decision 45) go onto this local copy with the same brush as
/// the overlays: never into the video, and only when the frame or the drawing changed.
fn render_image(
    thumb: &fitcom_video::Miniatuur,
    drawing: &fitcom_proto::teken::Tekening,
) -> Option<RenderImage> {
    let needed = thumb.breedte as usize * thumb.hoogte as usize * 4;
    if thumb.data.len() < needed {
        return None;
    }
    let mut bgra = thumb.data[..needed].to_vec();
    for px in bgra.chunks_exact_mut(4) {
        px[3] = 255;
    }
    if !drawing.is_leeg() {
        fitcom_video::penseel::Doek {
            px: &mut bgra,
            breedte: thumb.breedte,
            hoogte: thumb.hoogte,
        }
        .teken_alles(drawing);
    }
    let buffer = image::RgbaImage::from_raw(thumb.breedte, thumb.hoogte, bgra)?;
    Some(RenderImage::new(vec![image::Frame::new(buffer)]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Nobody on screen outside a call or with the overlay off; in a call ourselves first,
    /// then the known peers in the call (not the ones outside it), each lit by the ring's
    /// verdict, with a name that cannot reorder itself on screen.
    #[cfg(windows)]
    #[test]
    fn overlay_rows_follow_the_call_and_the_ring() {
        use super::super::state::{Constants, PeerState};
        let peer = |id: &str, name: &str, in_call: bool| -> PeerState {
            serde_json::from_value(serde_json::json!({
                "id": id, "name": name, "address": "", "avatar": 1, "presence": "online",
                "problem": null, "app_version": null, "in_call": in_call, "volume": 1.0,
                "unread": 0, "last_seen": null, "desktop_volume": null, "desktop_stream": null
            }))
            .unwrap()
        };
        let constants = Constants {
            me: fitcom_proto::PeerId::new_random(),
            fallback_name: "Rick".into(),
            sound_sets: Vec::new(),
            sound_events: Vec::new(),
            control_port: 0,
            media_port: 0,
            autostart: false,
            minimize_to_tray: false,
        };
        let mut s = UiState::build(
            &crate::engine::Snapshot::default(),
            &constants,
            0,
            &HashMap::new(),
        );
        s.peers = vec![
            peer("a", "Anna\u{202E}", true),
            peer("b", "Bob", false),
            peer("", "Unknown", true),
        ];
        let mut m = MeterReadings::default();
        m.peers.insert(
            "a".into(),
            PeerMeter {
                speaking: true,
                ..Default::default()
            },
        );

        s.voice.joined = true;
        assert!(overlay_rows(&s, &m).is_empty(), "off");
        s.overlay.enabled = true;
        let rows = overlay_rows(&s, &m);
        let names: Vec<&str> = rows.iter().map(|r| r.naam.as_str()).collect();
        assert_eq!(names, ["Rick", "Anna"]);
        let lit: Vec<bool> = rows.iter().map(|r| r.spreekt).collect();
        assert_eq!(lit, [false, true]);
        m.self_speaking = true;
        assert!(overlay_rows(&s, &m)[0].spreekt);
        s.voice.joined = false;
        assert!(overlay_rows(&s, &m).is_empty(), "not in a call");
    }

    /// The one conversion on this path: bytes unchanged, alpha forced opaque. Reverting the
    /// alpha line makes a camera tile transparent again, and this fails.
    #[test]
    fn a_thumbnail_keeps_its_bgra_bytes_and_becomes_opaque() {
        let thumb = fitcom_video::Miniatuur {
            breedte: 2,
            hoogte: 1,
            data: Arc::from(vec![10u8, 20, 30, 0, 40, 50, 60, 0]),
        };
        let image = render_image(&thumb, &Default::default()).expect("image");
        assert_eq!(
            image.as_bytes(0).unwrap(),
            &[10, 20, 30, 255, 40, 50, 60, 255]
        );
    }

    #[test]
    fn a_short_frame_is_skipped_rather_than_read_past() {
        let thumb = fitcom_video::Miniatuur {
            breedte: 4,
            hoogte: 4,
            data: Arc::from(vec![0u8; 8]),
        };
        assert!(render_image(&thumb, &Default::default()).is_none());
    }

    /// Lines land on the tile and the tile stays opaque. Dropping the drawing pass, or
    /// drawing before the alpha fix, makes this fail.
    #[test]
    fn lines_are_drawn_onto_the_tile_and_it_stays_opaque() {
        use fitcom_proto::control::Draw;
        let mut drawing = fitcom_proto::teken::Tekening::nieuw(true);
        drawing.teken(
            fitcom_proto::PeerId::new_random(),
            &Draw {
                stream_id: 1,
                author: None,
                line: 1,
                color: 0,
                width: 2,
                start: 0,
                points: vec![[0, 32767], [65535, 32767]],
            },
        );
        let thumb = fitcom_video::Miniatuur {
            breedte: 32,
            hoogte: 18,
            data: Arc::from(vec![0u8; 32 * 18 * 4]),
        };
        let image = render_image(&thumb, &drawing).expect("image");
        let bytes = image.as_bytes(0).unwrap();
        assert!(bytes.chunks_exact(4).all(|p| p[3] == 255));
        assert!(bytes.chunks_exact(4).any(|p| p[2] > 200), "a red line");
    }
}
