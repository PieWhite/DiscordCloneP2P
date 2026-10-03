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

/// The level above which a peer counts as speaking. Matches the voice VAD closely enough
/// that the ring does not light on room noise.
pub const SPEAKING: f32 = 0.06;

/// How long the speaking state survives a dip under the threshold. Natural speech dips
/// below any level threshold between words, and the 4 Hz tick samples straight into those
/// dips — gating the ring on a single sample made it strobe. Light the moment the level is
/// up, release only once it has stayed quiet this long (two ticks plus slack).
pub const SPEAK_HOLD: Duration = Duration::from_millis(600);

/// One peer's live figures.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct PeerMeter {
    /// `None` while the peer is not online.
    pub rtt: Option<u32>,
    pub level: f32,
    /// The verdict with the hold applied — what the speaking ring shows.
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
    /// member list (`SPEAKING`, `SPEAK_HOLD`).
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
            // Comparing here is what keeps an idle window at zero wake-ups — volatile
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
            let mut held = |key: &str, level: f32, gate: bool| -> bool {
                if !gate {
                    return false;
                }
                if level > SPEAKING {
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
                let speaking = held(&id, p.niveau, true);
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
                snap.voice.eigen_niveau,
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
        // Which frame each tile shows, so an unchanged tile costs nothing.
        let mut last: HashMap<String, usize> = HashMap::new();
        let mut images: Arc<ThumbMap> = Arc::new(ThumbMap::new());
        loop {
            ticker.tick().await;
            let snap = rx.borrow().clone();

            // Wat wij zelf uitsturen hoort in dezelfde strook als wat we van een ander
            // bekijken, dus het loopt door dezelfde pomp. Alleen de sleutel verschilt: een
            // eigen stream heeft geen eigenaar, dus `self-` in plaats van een peer-id.
            let eigen = snap
                .eigen_streams
                .iter()
                .filter(|s| !s.is_geluid)
                .map(|s| (Thumbnails::own_key(s.stream_id), s.miniatuur.as_ref()));
            let vreemd = snap
                .streams
                .iter()
                .filter(|s| !s.is_geluid && s.kijken)
                .map(|s| {
                    (
                        Thumbnails::peer_key(&s.eigenaar.to_string(), s.stream_id),
                        s.miniatuur.as_ref(),
                    )
                });

            let mut next: Option<ThumbMap> = None;
            let mut alive: Vec<String> = Vec::new();
            for (key, thumb) in eigen.chain(vreemd) {
                alive.push(key.clone());
                let Some(thumb) = thumb else { continue };
                let frame = Arc::as_ptr(&thumb.data) as *const u8 as usize;
                if last.get(&key) == Some(&frame) {
                    continue;
                }
                let Some(image) = render_image(thumb) else {
                    continue;
                };
                last.insert(key.clone(), frame);
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

    Feeds {
        state,
        meters,
        thumbs,
    }
}

/// The decoder hands us BGRA and a GPUI image is BGRA, so the bytes go across as they are —
/// no PNG, no channel swap. Only the alpha byte is fixed: it means nothing on the way here
/// (Media Foundation's RGB32 leaves it at 0), and an image honours it, so a camera tile
/// came out transparent. A thumbnail is opaque by definition.
fn render_image(thumb: &fitcom_video::Miniatuur) -> Option<RenderImage> {
    let needed = thumb.breedte as usize * thumb.hoogte as usize * 4;
    if thumb.data.len() < needed {
        return None;
    }
    let mut bgra = thumb.data[..needed].to_vec();
    for px in bgra.chunks_exact_mut(4) {
        px[3] = 255;
    }
    let buffer = image::RgbaImage::from_raw(thumb.breedte, thumb.hoogte, bgra)?;
    Some(RenderImage::new(vec![image::Frame::new(buffer)]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The one conversion on this path: bytes unchanged, alpha forced opaque. Reverting the
    /// alpha line makes a camera tile transparent again, and this fails.
    #[test]
    fn a_thumbnail_keeps_its_bgra_bytes_and_becomes_opaque() {
        let thumb = fitcom_video::Miniatuur {
            breedte: 2,
            hoogte: 1,
            data: Arc::from(vec![10u8, 20, 30, 0, 40, 50, 60, 0]),
        };
        let image = render_image(&thumb).expect("image");
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
        assert!(render_image(&thumb).is_none());
    }
}
