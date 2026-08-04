//! The view model the webview reads.
//!
//! This is one half of the `Snapshot`/`UiCommand` boundary that survived the move off
//! egui (see `docs/OVERDRACHT.md`, decision 19). The engine still publishes an
//! immutable `Snapshot`; this module turns it into JSON the frontend can render, and
//! nothing else. No decision about the network, the store or the media threads is taken
//! here, and no state that matters lives here — the window may stop drawing entirely
//! without anything going wrong.
//!
//! Field names are English, as is everything written for this layer. The engine keeps
//! its Dutch identifiers; the translation happens exactly here, in one file, instead of
//! being smeared across a rename of five crates that this phase is not allowed to touch.

use crate::engine::{PeerView, Snapshot};
use crate::files::{self, DownloadStatus};
use crate::tags;
use crate::updates::UpdateStatus;
use fitcom_net::PeerStatus;
use fitcom_proto::{Channel, OpId, PeerId, TopicId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// How a `Channel` travels to the frontend and back: `general`, `topic:<uuid>` or
/// `dm:<uuid>`. A string rather than a nested object because it is used as an object key
/// and a `data-` attribute on nearly every row in the window.
pub fn channel_key(channel: Channel) -> String {
    if let Some(peer) = channel.dm_peer() {
        format!("dm:{peer}")
    } else if let Some(topic) = channel.topic_id() {
        format!("topic:{}", topic.0)
    } else {
        "general".to_string()
    }
}

/// Inverse of [`channel_key`]. An unparsable key falls back to the general channel
/// rather than failing: the frontend can only produce keys this layer handed it, so a
/// miss means a bug here, not input to validate.
pub fn parse_channel(key: &str) -> Channel {
    match key.split_once(':') {
        Some(("dm", id)) => id
            .parse::<uuid::Uuid>()
            .map(|u| Channel::dm(PeerId(u)))
            .unwrap_or(Channel::GENERAL),
        Some(("topic", id)) => id
            .parse::<uuid::Uuid>()
            .map(|u| Channel::topic(TopicId(u)))
            .unwrap_or(Channel::GENERAL),
        _ => Channel::GENERAL,
    }
}

/// The three avatar fills from `DESIGN.md` are identity, not state: one hue per peer for
/// the life of the install.
///
/// Assigned by position in the sorted set of known peers rather than by hashing the id.
/// A hash is stable too, but it collides — two of three peers came out the same green on
/// the first run — and two people wearing one colour defeats the point of the hue.
/// Sorting is just as stable for a fixed group and is collision-free up to three.
fn avatar_hues(me: PeerId, peers: &[PeerView]) -> HashMap<PeerId, u8> {
    let mut ids: Vec<PeerId> = std::iter::once(me)
        .chain(peers.iter().filter_map(|p| p.peer_id))
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids.into_iter()
        .enumerate()
        .map(|(i, id)| (id, (i % 3) as u8 + 1))
        .collect()
}

/// One line of presence vocabulary, shared by the roster, the status bar and the peer
/// table in settings. Deliberately not a colour: `offline` is drawn as an unlit ring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Presence {
    Online,
    Connecting,
    Offline,
    /// Protocol or identity trouble. Rare, and the only peer state that is a real fault
    /// rather than a normal absence.
    Broken,
}

#[derive(Debug, Clone, Serialize)]
pub struct PeerState {
    /// Empty for a peer whose identity we have not learned yet.
    pub id: String,
    pub name: String,
    pub address: String,
    pub avatar: u8,
    pub presence: Presence,
    /// Set when `presence` is `broken`: what is wrong, in one sentence.
    pub problem: Option<String>,
    pub app_version: Option<String>,
    pub in_call: bool,
    pub volume: f32,
    pub unread: usize,
    /// When this peer was last seen online, in millis since epoch. Observed by this
    /// process, not remembered across restarts — so it is `None` for a peer that has not
    /// been up since this app started, and the roster then says plain "Offline" rather
    /// than inventing a time.
    pub last_seen: Option<i64>,
    /// Does this peer share desktop audio we are listening to?
    pub desktop_volume: Option<f32>,
    pub desktop_stream: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SelfState {
    pub id: String,
    pub name: String,
    pub avatar: u8,
}

#[derive(Debug, Clone, Serialize)]
pub struct VoiceState {
    pub joined: bool,
    pub muted: bool,
    pub deafened: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChannelState {
    pub key: String,
    pub name: String,
    pub unread: usize,
    /// Only a sub-channel can be renamed or removed; the general channel cannot.
    pub removable: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct OwnStreamState {
    pub stream_id: u32,
    pub title: String,
    pub viewers: usize,
    pub is_audio: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct StreamState {
    pub owner: String,
    pub owner_name: String,
    pub stream_id: u32,
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub watching: bool,
    pub is_audio: bool,
    pub volume: f32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum TransferState {
    /// Offered by someone else and not fetched yet.
    Available,
    Running {
        received: u64,
        total: u64,
    },
    Done,
    Failed {
        error: String,
    },
    /// Ours. There is nothing to download.
    Mine,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum TimelineItem {
    Message {
        id: OpRef,
        author: String,
        author_name: String,
        avatar: u8,
        body: String,
        at: i64,
        edited: bool,
        mine: bool,
        mentions_you: bool,
    },
    File {
        id: OpRef,
        author: String,
        author_name: String,
        avatar: u8,
        name: String,
        size: u64,
        at: i64,
        mine: bool,
        transfer: TransferState,
        /// Absolute path in the content-addressed picture folder, for an image whose
        /// bytes are actually on this machine. The frontend turns it into an `asset:`
        /// URL. `None` covers both "not an image" and "not downloaded yet", and the card
        /// falls back to the generic form — the same rule the egui build used.
        image_path: Option<String>,
    },
}

/// An `OpId` in a shape the frontend can hand straight back to a command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpRef {
    pub author: String,
    pub channel: String,
    pub seq: u64,
}

impl OpRef {
    fn of(id: OpId) -> Self {
        Self {
            author: id.author.to_string(),
            channel: channel_key(id.channel),
            seq: id.seq,
        }
    }

    pub fn to_op_id(&self) -> Option<OpId> {
        Some(OpId::new(
            PeerId(self.author.parse().ok()?),
            parse_channel(&self.channel),
            self.seq,
        ))
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum UpdateState {
    Offered {
        peer: String,
        version: String,
    },
    Downloading {
        peer: String,
        version: String,
        received: u64,
        total: u64,
    },
    Ready {
        peer: String,
        version: String,
    },
    Failed {
        error: String,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct VideoSettings {
    pub codec: String,
    pub fps: u32,
    pub bitrate: u32,
}

/// Everything the window needs to draw itself, minus the timeline.
///
/// The timeline is fetched per conversation with `get_timeline` instead of riding along
/// here. It is the only unbounded part of the state, and pushing a whole history through
/// the IPC bridge on every tick would undo the reason for moving off immediate mode.
#[derive(Debug, Clone, Serialize)]
pub struct UiState {
    #[serde(rename = "self")]
    pub me: SelfState,
    pub peers: Vec<PeerState>,
    pub voice: VoiceState,
    pub channels: Vec<ChannelState>,
    pub own_streams: Vec<OwnStreamState>,
    pub streams: Vec<StreamState>,
    pub video: VideoSettings,
    pub input_device: Option<String>,
    pub output_device: Option<String>,
    pub do_not_disturb: bool,
    pub update: Option<UpdateState>,
    pub error: Option<String>,
    pub app_version: String,
    pub protocol_version: u32,
    pub control_port: u16,
    pub media_port: u16,
    pub download_dir: String,
    pub pictures_dir: String,
    pub autostart: bool,
    pub minimize_to_tray: bool,
    /// Changes whenever the op log did. The frontend refetches the open conversation
    /// when it moves, so a message that arrives while you are reading shows up without
    /// the whole history travelling on every tick.
    pub timeline_revision: u64,
}

/// The constant half of the state: what the config and the build say, not what the
/// engine is currently doing.
pub struct Constants {
    pub me: PeerId,
    pub fallback_name: String,
    pub control_port: u16,
    pub media_port: u16,
    pub download_dir: std::path::PathBuf,
    pub pictures_dir: std::path::PathBuf,
    pub autostart: bool,
    pub minimize_to_tray: bool,
}

fn display_name(snap: &Snapshot, peer: PeerId, fallback: &str) -> String {
    snap.timeline
        .nicknames
        .get(&peer)
        .cloned()
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

/// RTT is deliberately not here: it moves every tick and lives in the `meters` event, so
/// a measurement ticking over cannot force a full state push. See `ui/mod.rs`.
fn peer_presence(view: &PeerView) -> (Presence, Option<String>, Option<String>) {
    match &view.status {
        PeerStatus::Online { app_version, .. } => {
            (Presence::Online, None, Some(app_version.clone()))
        }
        PeerStatus::Connecting => (Presence::Connecting, None, None),
        PeerStatus::Offline { .. } => (Presence::Offline, None, None),
        PeerStatus::VersionMismatch { theirs, ours } => (
            Presence::Broken,
            Some(format!(
                "Runs protocol {theirs}, this build speaks {ours}. One of you has to update."
            )),
            None,
        ),
        PeerStatus::IdentityChanged { .. } => (
            Presence::Broken,
            Some("A different identity answered at this address.".into()),
            None,
        ),
    }
}

impl UiState {
    pub fn build(
        snap: &Snapshot,
        c: &Constants,
        timeline_revision: u64,
        last_seen: &HashMap<PeerId, i64>,
    ) -> Self {
        let my_name = display_name(snap, c.me, &c.fallback_name);
        let hues = avatar_hues(c.me, &snap.peers);

        // Desktop audio rides along as its own stream. It belongs to the peer's row in
        // the member list, not in the strip, so it is folded into the peer here.
        let desktop = |owner: PeerId| {
            snap.streams
                .iter()
                .find(|s| s.eigenaar == owner && s.is_geluid && s.kijken)
                .map(|s| (s.stream_id, s.volume))
        };

        let peers = snap
            .peers
            .iter()
            .map(|p| {
                let (presence, problem, app_version) = peer_presence(p);
                let id = p.peer_id;
                let desk = id.and_then(desktop);
                PeerState {
                    id: id.map(|p| p.to_string()).unwrap_or_default(),
                    name: match id {
                        Some(peer) => display_name(snap, peer, &p.label),
                        None => p.label.clone(),
                    },
                    address: p.address.clone(),
                    avatar: id.and_then(|i| hues.get(&i).copied()).unwrap_or(3),
                    presence,
                    problem,
                    app_version,
                    in_call: p.in_voice,
                    volume: p.volume,
                    unread: id
                        .and_then(|peer| snap.ongelezen_dm.get(&peer).copied())
                        .unwrap_or(0),
                    // Only while they are away. Carrying it for an online peer would
                    // move on every tick and turn every tick into a state event — the
                    // one thing the change-detection in `ui/mod.rs` exists to avoid.
                    last_seen: (presence != Presence::Online)
                        .then(|| id.and_then(|i| last_seen.get(&i).copied()))
                        .flatten(),
                    desktop_volume: desk.map(|d| d.1),
                    desktop_stream: desk.map(|d| d.0),
                }
            })
            .collect();

        let mut channels = vec![ChannelState {
            key: "general".into(),
            name: "general".into(),
            unread: snap.ongelezen,
            removable: false,
        }];
        let mut topics: Vec<(&TopicId, &String)> = snap.timeline.topics.iter().collect();
        topics.sort_by_key(|(_, title)| title.to_lowercase());
        channels.extend(topics.into_iter().map(|(id, title)| ChannelState {
            key: channel_key(Channel::topic(*id)),
            name: title.clone(),
            unread: snap.ongelezen_topic.get(id).copied().unwrap_or(0),
            removable: true,
        }));

        let name_of = |peer: PeerId| display_name(snap, peer, &peer.to_string()[..8]);

        Self {
            me: SelfState {
                id: c.me.to_string(),
                name: my_name,
                avatar: hues.get(&c.me).copied().unwrap_or(1),
            },
            peers,
            voice: VoiceState {
                joined: snap.voice.actief,
                muted: snap.voice.muted,
                deafened: snap.voice.deafened,
            },
            channels,
            own_streams: snap
                .eigen_streams
                .iter()
                .map(|s| OwnStreamState {
                    stream_id: s.stream_id,
                    title: s.titel.clone(),
                    viewers: s.kijkers,
                    is_audio: s.is_geluid,
                })
                .collect(),
            streams: snap
                .streams
                .iter()
                .filter(|s| !s.is_geluid)
                .map(|s| StreamState {
                    owner: s.eigenaar.to_string(),
                    owner_name: name_of(s.eigenaar),
                    stream_id: s.stream_id,
                    title: s.titel.clone(),
                    width: s.breedte,
                    height: s.hoogte,
                    watching: s.kijken,
                    is_audio: false,
                    volume: s.volume,
                })
                .collect(),
            video: VideoSettings {
                codec: snap.video.codec.clone(),
                fps: snap.video.fps,
                bitrate: snap.video.bitrate,
            },
            input_device: snap.input_device.clone(),
            output_device: snap.output_device.clone(),
            do_not_disturb: snap.niet_storen,
            update: snap.update.as_ref().map(|u| match u {
                UpdateStatus::Aangeboden { peer, hun_versie } => UpdateState::Offered {
                    peer: name_of(*peer),
                    version: hun_versie.clone(),
                },
                UpdateStatus::Bezig {
                    peer,
                    hun_versie,
                    ontvangen,
                    totaal,
                    ..
                } => UpdateState::Downloading {
                    peer: name_of(*peer),
                    version: hun_versie.clone(),
                    received: *ontvangen,
                    total: *totaal,
                },
                UpdateStatus::KlaarOmToeTePassen {
                    peer, hun_versie, ..
                } => UpdateState::Ready {
                    peer: name_of(*peer),
                    version: hun_versie.clone(),
                },
                UpdateStatus::Mislukt(e) => UpdateState::Failed { error: e.clone() },
            }),
            error: snap.fout.clone(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            protocol_version: fitcom_proto::PROTOCOL_VERSION,
            control_port: c.control_port,
            media_port: c.media_port,
            download_dir: c.download_dir.display().to_string(),
            pictures_dir: c.pictures_dir.display().to_string(),
            autostart: c.autostart,
            minimize_to_tray: c.minimize_to_tray,
            timeline_revision,
        }
    }
}

/// Messages and files of one conversation, merged into the single chronological order
/// the timeline draws. `lamport` is the sort key both carry for exactly this reason —
/// wall clocks differ per machine and would order the thread differently per peer.
pub fn timeline_of(
    snap: &Snapshot,
    channel: Channel,
    me: PeerId,
    my_name: &str,
    pictures_dir: &Path,
) -> Vec<TimelineItem> {
    let name_of = |peer: PeerId| display_name(snap, peer, &peer.to_string()[..8]);
    let hues = avatar_hues(me, &snap.peers);
    let hue = |peer: PeerId| hues.get(&peer).copied().unwrap_or(3);

    let mut items: Vec<(u64, PeerId, TimelineItem)> = Vec::new();

    for m in &snap.timeline.messages {
        if !belongs_to_channel(channel, me, m.channel, m.author) {
            continue;
        }
        items.push((
            m.lamport,
            m.author,
            TimelineItem::Message {
                id: OpRef::of(m.id),
                author: m.author.to_string(),
                author_name: name_of(m.author),
                avatar: hue(m.author),
                body: m.body.clone(),
                at: m.created_at,
                edited: m.edited,
                mine: m.author == me,
                mentions_you: m.author != me && tags::bevat_tag(&m.body, my_name),
            },
        ));
    }

    for f in &snap.files {
        if !belongs_to_channel(channel, me, f.channel, f.author) {
            continue;
        }
        let image_path = files::is_afbeelding(&f.name)
            .then(|| pictures_dir.join(files::hash_bestandsnaam(&f.hash, &f.name)))
            .filter(|p| p.exists())
            .map(|p| p.display().to_string());

        items.push((
            f.lamport,
            f.author,
            TimelineItem::File {
                id: OpRef::of(f.id),
                author: f.author.to_string(),
                author_name: name_of(f.author),
                avatar: hue(f.author),
                name: f.name.clone(),
                size: f.size,
                // Files carry no wall clock of their own; the timeline groups them with
                // the message they were dropped next to, which is what `lamport` orders.
                at: 0,
                mine: f.is_mine,
                transfer: match (&f.status, f.is_mine) {
                    (_, true) => TransferState::Mine,
                    (None, _) => TransferState::Available,
                    (Some(DownloadStatus::Bezig { ontvangen, totaal }), _) => {
                        TransferState::Running {
                            received: *ontvangen,
                            total: *totaal,
                        }
                    }
                    (Some(DownloadStatus::Voltooid), _) => TransferState::Done,
                    (Some(DownloadStatus::Mislukt(e)), _) => {
                        TransferState::Failed { error: e.clone() }
                    }
                },
                image_path,
            },
        ));
    }

    items.sort_by_key(|(lamport, author, _)| (*lamport, *author));
    items.into_iter().map(|(_, _, item)| item).collect()
}

/// Whether a message or file belongs to the conversation on screen.
///
/// For a public channel that is plain equality. For a DM it is subtler: `Channel::dm(x)`
/// means "the author DM'd x", so *my* messages to X carry `Dm(X)` while X's replies to me
/// carry `Dm(me)` — not `Dm(X)`. One conversation is therefore two channel values, one
/// per participant, and comparing against the open channel alone shows only your own half
/// of it. That was a real bug once; the tests below are what keep it fixed.
pub fn belongs_to_channel(open: Channel, me: PeerId, channel: Channel, author: PeerId) -> bool {
    match open.dm_peer() {
        Some(other) => match channel.dm_peer() {
            // Mine to them, or theirs to me. Anything else is a different conversation.
            Some(target) => (author == me && target == other) || (author == other && target == me),
            None => false,
        },
        None => channel == open,
    }
}

#[cfg(test)]
mod tests {
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
    fn a_subchannel_shows_only_its_own_messages() {
        let open = Channel::topic(topic(1));
        assert!(belongs_to_channel(open, peer(1), open, peer(2)));
        assert!(!belongs_to_channel(
            open,
            peer(1),
            Channel::topic(topic(2)),
            peer(2)
        ));
        assert!(!belongs_to_channel(
            open,
            peer(1),
            Channel::GENERAL,
            peer(2)
        ));
    }

    #[test]
    fn the_general_channel_shows_no_subchannel_messages() {
        assert!(!belongs_to_channel(
            Channel::GENERAL,
            peer(1),
            Channel::topic(topic(1)),
            peer(2)
        ));
        assert!(belongs_to_channel(
            Channel::GENERAL,
            peer(1),
            Channel::GENERAL,
            peer(2)
        ));
    }

    #[test]
    fn a_dm_shows_both_sides_of_the_conversation() {
        let me = peer(1);
        let other = peer(2);
        let open = Channel::dm(other);
        // Mine, addressed to them.
        assert!(belongs_to_channel(open, me, Channel::dm(other), me));
        // Theirs, addressed to me — the half a plain equality check would drop.
        assert!(belongs_to_channel(open, me, Channel::dm(me), other));
    }

    #[test]
    fn a_dm_shows_no_messages_from_another_conversation() {
        let me = peer(1);
        let open = Channel::dm(peer(2));
        assert!(!belongs_to_channel(open, me, Channel::dm(peer(3)), me));
        assert!(!belongs_to_channel(open, me, Channel::dm(me), peer(3)));
        assert!(!belongs_to_channel(open, me, Channel::GENERAL, peer(2)));
    }

    #[test]
    fn channel_keys_survive_a_round_trip() {
        for c in [
            Channel::GENERAL,
            Channel::dm(peer(7)),
            Channel::topic(topic(9)),
        ] {
            assert_eq!(parse_channel(&channel_key(c)), c);
        }
    }

    #[test]
    fn every_peer_gets_its_own_avatar_hue() {
        let peers: Vec<PeerView> = [2u8, 3, 4]
            .iter()
            .map(|n| PeerView {
                label: String::new(),
                address: String::new(),
                peer_id: Some(peer(*n)),
                status: fitcom_net::PeerStatus::Connecting,
                in_voice: false,
                niveau: 0.0,
                volume: 1.0,
            })
            .collect();
        // Three peers plus me is four identities, so one hue repeats — but never within
        // the three the hashing version collided on.
        let hues = avatar_hues(peer(1), &peers[..2]);
        let mut seen: Vec<u8> = hues.values().copied().collect();
        seen.sort_unstable();
        assert_eq!(seen, vec![1, 2, 3]);
        // Stable: the same set gives the same assignment every time.
        assert_eq!(avatar_hues(peer(1), &peers), avatar_hues(peer(1), &peers));
    }
}
