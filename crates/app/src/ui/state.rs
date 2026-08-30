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
use crate::wordle;
use fitcom_net::PeerStatus;
use fitcom_proto::{Channel, OpId, PeerId, TopicId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// How much of the parent's body rides along in a reply quote, in characters. Roughly
/// one line; the full text is one click away.
const REPLY_SNIPPET_LEN: usize = 90;

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
    /// The status this peer chose for itself ("away", "busy"), while it is online. `None`
    /// means it never sent one (or is offline): plain online, nothing extra to draw.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_status: Option<String>,
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
    /// Our own chosen status: "online", "away" or "busy".
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct VoiceState {
    pub joined: bool,
    pub muted: bool,
    pub deafened: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClipsState {
    /// Recording is on.
    pub enabled: bool,
    /// How far back a clip reaches, in seconds.
    pub window_sec: u32,
    /// Which screen is being recorded (name from the source list).
    pub monitor: Option<String>,
    /// The configured global shortcut, as written in the config.
    pub hotkey: String,
    /// Where finished clips land; for the open-folder button.
    pub folder: String,
    /// The most recent clip, if any was saved this session.
    pub last_clip: Option<String>,
    /// Last clip error that has not been replaced yet.
    pub error: Option<String>,
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
    /// Our own camera rather than a screen: the camera button reads this for its pressed
    /// state, and the sharing line leaves it out of "Sharing <screen>".
    pub is_camera: bool,
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
    /// Somebody's camera rather than their screen. Same window, same thumbnail — only the
    /// icon in the list differs.
    pub is_camera: bool,
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

/// One peer's score on one Wordle day, as the card in the log draws it.
#[derive(Debug, Clone, Serialize)]
pub struct WordleScore {
    pub peer: String,
    pub name: String,
    pub avatar: u8,
    pub mine: bool,
    /// Guesses used, 1 to 6, whether or not the word was found.
    pub guesses: u8,
    pub solved: bool,
    /// The squares, five characters per row: `0` miss, `1` near, `2` hit. Empty if the
    /// peer runs a build that did not send one.
    pub pattern: String,
    /// How long the game took, first guess to last, in whole seconds. `None` for a result
    /// from before the clock existed, or from a peer that does not send one — the window
    /// then draws no time. Equal guesses are broken by this; see `crate::wordle::winnaars`.
    pub seconds: Option<u32>,
    /// Took the point for this day. More than one means a draw, and then they all did.
    pub won: bool,
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
        /// The message this one answers, when it was sent as a reply. The frontend draws
        /// a clickable quote above the body.
        #[serde(skip_serializing_if = "Option::is_none")]
        reply_to: Option<OpRef>,
        /// Author and opening text of that parent, resolved on this side. `None` also
        /// covers a parent that is deleted or not (yet) in our log — the frontend then
        /// shows "original unavailable" instead of guessing.
        #[serde(skip_serializing_if = "Option::is_none")]
        reply_snippet: Option<ReplySnippet>,
        /// Reactions grouped per emoji, in the same order every peer folds them to.
        reactions: Vec<ReactionState>,
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
        /// Where the bytes are on this machine — our own offer, or a finished download.
        /// `Some` is what turns the download button into an open button, so it is also
        /// the answer to "may this be opened at all": the frontend never gets to name a
        /// path, it hands back the `OpRef` and `open_file` looks it up here again.
        ///
        /// For an image this is the same path as `image_path`; for everything else it is
        /// the file in the download folder under whatever name it actually got.
        local_path: Option<String>,
        /// Whether opening this one shows the folder instead of the file, because the
        /// system would execute it. The button says so rather than promising "Open" and
        /// doing something else; the decision itself is made again in `open_file`, from
        /// the same `files::opent_als_code`.
        opens_folder: bool,
    },
    /// The puzzle of the day, which shows up in #general at 07:00 (2026-08-20).
    ///
    /// Not an op and never on the wire: every peer can work out on its own that today is
    /// today, so a card carries no information anybody has to be told. Three peers each
    /// posting a "here is today's puzzle" message would be three cards, and the oplog has
    /// no way to make them one — `seq` is per author. What *does* travel is the results.
    Wordle {
        /// The puzzle's date as `YYYYMMDD`. The key everything else is grouped by.
        day: u32,
        /// 07:00 local on that day: where the card sits between the messages, and the
        /// time the log prints next to it.
        at: i64,
        /// The number the real Wordle prints above it. Only known for a day this machine
        /// fetched itself.
        number: Option<u32>,
        /// What the button does: `play`, `continue`, `done`, `waiting` (the word has not
        /// arrived), or `past` (an older day, which cannot be played any more).
        action: String,
        /// Guesses used on an unfinished game of today.
        progress: u8,
        /// Everyone who played, best score first.
        results: Vec<WordleScore>,
        /// Did this day count for points? False while only one peer has played — you get
        /// no point for playing alone. See `crate::wordle::winnaars`.
        scored: bool,
    },
}

/// An `OpId` in a shape the frontend can hand straight back to a command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpRef {
    pub author: String,
    pub channel: String,
    pub seq: u64,
}

/// The quote above a reply: who wrote the parent and how it opens. Built here so the
/// frontend never has to look an `OpRef` up in the log itself.
#[derive(Debug, Clone, Serialize)]
pub struct ReplySnippet {
    pub author_name: String,
    /// The parent's body, cut off at roughly one line.
    pub body: String,
}

/// One reaction pill under a message.
#[derive(Debug, Clone, Serialize)]
pub struct ReactionState {
    pub emoji: String,
    pub count: usize,
    /// Peer UUIDs, in the store's own order; the frontend resolves names for a tooltip.
    pub peers: Vec<String>,
    /// Did I react too? Decides whether clicking removes or adds.
    pub mine: bool,
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
    /// A check the user asked for is running. An automatic one never reaches the window.
    Checking,
    Downloading {
        version: String,
        received: u64,
        total: u64,
    },
    Ready {
        version: String,
    },
    /// Asked, answered: the feed had nothing newer.
    #[serde(rename = "uptodate")]
    UpToDate,
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

/// Which set of notification tones is picked, and how loud.
#[derive(Debug, Clone, Serialize)]
pub struct SoundSettings {
    /// The set that will actually be heard, which is not always the one in `config.toml`.
    ///
    /// A config written by a newer build may name a set this build does not have; the config
    /// keeps that name on purpose, so downgrading once does not erase the choice (see
    /// `config::SoundConfig::herstel`). The window has to show what is true rather than what
    /// is stored, otherwise the picker highlights nothing at all — so the fallback is
    /// resolved here, in the view, where showing reality is the whole job.
    pub set: String,
    /// 0.0 to 1.0. The slider shows whole percents.
    pub volume: f32,
}

/// One choosable set of tones. Comes from the engine rather than being spelled out in the
/// frontend, so adding a set is one place instead of two.
#[derive(Debug, Clone, Serialize)]
pub struct SoundSetInfo {
    pub id: String,
    pub name: String,
    pub description: String,
}

/// One event that can be auditioned, so all six can be judged instead of only the one the
/// preview button happens to play.
#[derive(Debug, Clone, Serialize)]
pub struct SoundEventInfo {
    pub id: String,
    pub name: String,
}

/// One row of the board: the word and its five colours.
#[derive(Debug, Clone, Serialize)]
pub struct WordleRow {
    pub word: String,
    /// `0` miss, `1` near, `2` hit — one per letter.
    pub marks: Vec<u8>,
}

/// Today's board, as the dialog draws it.
#[derive(Debug, Clone, Serialize)]
pub struct WordleBoard {
    pub number: u32,
    pub rows: Vec<WordleRow>,
    pub done: bool,
    pub won: bool,
    /// Only once the game is over. While it runs, the answer stays in the engine — see
    /// `crate::wordle`.
    pub solution: Option<String>,
    /// How long the finished game took, in whole seconds. Nothing ticks here while you
    /// play; it appears when the game is over. `None` for a game from before the clock.
    pub seconds: Option<u32>,
}

/// One line of the leaderboard.
#[derive(Debug, Clone, Serialize)]
pub struct WordleStanding {
    pub peer: String,
    pub name: String,
    pub avatar: u8,
    pub mine: bool,
    /// Days won, or drawn on the lowest number of guesses.
    pub points: u32,
    pub played: u32,
    pub solved: u32,
}

/// The game of the day and the leaderboard over all days.
#[derive(Debug, Clone, Serialize)]
pub struct WordleState {
    /// Today's puzzle date as `YYYYMMDD`. Rolls over at 07:00, not at midnight.
    pub day: u32,
    /// `None` while today's word has not been fetched — then there is no card either.
    pub board: Option<WordleBoard>,
    /// Why the last guess was not accepted.
    pub error: Option<String>,
    /// Highest points first. Only peers who ever played show up.
    pub standings: Vec<WordleStanding>,
    /// How many guesses a game allows, so the frontend does not carry its own copy.
    pub tries: u8,
}

/// One person's week, for the Recap panel.
#[derive(Debug, Clone, Serialize)]
pub struct RecapRow {
    pub peer: String,
    pub name: String,
    pub avatar: u8,
    pub mine: bool,
    /// Seconds in the call, as this machine saw it. Someone who talked while we were
    /// offline is not counted — see `crate::gebruik` for why that is the honest number.
    pub voice_sec: u64,
    /// Seconds with a screen, window or camera shared. Desktop audio is not counted: it
    /// rides along with a screen and would double every share.
    pub shared_sec: u64,
    pub messages: u32,
    pub files: u32,
    pub wordle_points: u32,
    pub wordle_played: u32,
    pub wordle_solved: u32,
}

/// The last seven days, fetched with `get_recap` when the panel opens.
///
/// Not part of `UiState`: every figure in here moves while a call is running, and a state
/// event fires on any change to that struct. Same reasoning as the timeline.
#[derive(Debug, Clone, Serialize)]
pub struct Recap {
    /// Length of the window in days.
    pub days: u32,
    /// First day in the window, `YYYYMMDD`.
    pub from: u32,
    /// Busiest first. Only people who did something show up.
    pub rows: Vec<RecapRow>,
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
    pub sound: SoundSettings,
    /// Constant for the life of the process; rides along so the Settings panel does not
    /// need a second call to draw the picker.
    pub sound_sets: Vec<SoundSetInfo>,
    pub sound_events: Vec<SoundEventInfo>,
    pub input_device: Option<String>,
    pub output_device: Option<String>,
    pub do_not_disturb: bool,
    /// Clip recording (fase 15). `None` where the platform has none — the UI hides
    /// everything clips-related then.
    pub clips: Option<ClipsState>,
    /// Who is typing where, keyed by conversation key ("general", "topic:<uuid>",
    /// "dm:<peer-uuid>") with the peer UUIDs. Vluchtig — rides along in `state`, which is
    /// fine: a typing event changes at most a few times a minute, and only while someone
    /// actually types.
    pub typing: HashMap<String, Vec<String>>,
    pub update: Option<UpdateState>,
    pub wordle: WordleState,
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
    /// The choosable tone sets and the events that can be auditioned. Built once from
    /// `crate::geluid`, because the frontend must not carry a second copy of that list.
    pub sound_sets: Vec<SoundSetInfo>,
    pub sound_events: Vec<SoundEventInfo>,
    pub control_port: u16,
    pub media_port: u16,
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
                    user_status: p
                        .user_status
                        .filter(|_| presence == Presence::Online)
                        .map(|s| s.to_string()),
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
                status: snap.eigen_status.to_string(),
            },
            peers,
            voice: VoiceState {
                joined: snap.voice.actief,
                muted: snap.voice.muted,
                deafened: snap.voice.deafened,
            },
            typing: {
                let mut map: HashMap<String, Vec<String>> = HashMap::new();
                for (channel, peer) in &snap.typing {
                    map.entry(channel_key(*channel))
                        .or_default()
                        .push(peer.to_string());
                }
                map
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
                    is_camera: s.is_camera,
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
                    is_camera: s.is_camera,
                    volume: s.volume,
                })
                .collect(),
            video: VideoSettings {
                codec: snap.video.codec.clone(),
                fps: snap.video.fps,
                bitrate: snap.video.bitrate,
            },
            sound: SoundSettings {
                set: crate::geluid::Geluidset::van_naam(&snap.geluid.set)
                    .unwrap_or(crate::geluid::Geluidset::STANDAARD)
                    .naam()
                    .to_string(),
                volume: snap.geluid.volume,
            },
            sound_sets: c.sound_sets.clone(),
            sound_events: c.sound_events.clone(),
            input_device: snap.input_device.clone(),
            output_device: snap.output_device.clone(),
            do_not_disturb: snap.niet_storen,
            clips: snap.clips.as_ref().map(|c| ClipsState {
                enabled: c.aan,
                window_sec: c.venster_sec,
                monitor: c.monitor.clone(),
                hotkey: c.hotkey.clone(),
                folder: c.map.clone(),
                last_clip: c.laatste.clone(),
                error: c.fout.clone(),
            }),
            wordle: wordle_state(snap, c.me, &hues),
            update: snap.update.as_ref().map(|u| match u {
                UpdateStatus::Zoeken => UpdateState::Checking,
                UpdateStatus::Actueel => UpdateState::UpToDate,
                UpdateStatus::Bezig {
                    versie,
                    ontvangen,
                    totaal,
                } => UpdateState::Downloading {
                    version: versie.clone(),
                    received: *ontvangen,
                    total: *totaal,
                },
                UpdateStatus::KlaarOmToeTePassen { versie, .. } => UpdateState::Ready {
                    version: versie.clone(),
                },
                UpdateStatus::Mislukt(e) => UpdateState::Failed { error: e.clone() },
            }),
            error: snap.fout.clone(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            protocol_version: fitcom_proto::PROTOCOL_VERSION,
            control_port: c.control_port,
            media_port: c.media_port,
            download_dir: snap.download_dir.display().to_string(),
            pictures_dir: snap.pictures_dir.display().to_string(),
            autostart: c.autostart,
            minimize_to_tray: c.minimize_to_tray,
            timeline_revision,
        }
    }
}

/// The recap the engine last computed, with names and avatars filled in.
///
/// Nothing is recalculated here — the engine owns the numbers, this only dresses them for
/// the window. That keeps the one place where the week is counted in the engine, next to
/// the measurement itself.
pub fn recap_of(snap: &Snapshot, me: PeerId) -> Recap {
    let hues = avatar_hues(me, &snap.peers);
    let o = &snap.overzicht;
    Recap {
        days: o.dagen,
        from: o.vanaf,
        rows: o
            .regels
            .iter()
            .map(|r| RecapRow {
                peer: r.peer.to_string(),
                name: display_name(snap, r.peer, &r.peer.to_string()[..8]),
                avatar: hues.get(&r.peer).copied().unwrap_or(3),
                mine: r.peer == me,
                voice_sec: r.voice_ms / 1000,
                shared_sec: r.deel_ms / 1000,
                messages: r.berichten,
                files: r.bestanden,
                wordle_points: r.punten,
                wordle_played: r.gespeeld,
                wordle_solved: r.opgelost,
            })
            .collect(),
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
) -> Vec<TimelineItem> {
    let name_of = |peer: PeerId| display_name(snap, peer, &peer.to_string()[..8]);
    let hues = avatar_hues(me, &snap.peers);
    let hue = |peer: PeerId| hues.get(&peer).copied().unwrap_or(3);

    // The wall clock rides along so the Wordle cards can be slotted in by time — see the
    // merge at the bottom. Ordering between messages and files stays `(lamport, author)`.
    let mut items: Vec<(u64, PeerId, i64, TimelineItem)> = Vec::new();

    // Parent lookup for reply quotes: only messages of this conversation can be parents
    // (the fold nulls cross-channel references), so one pass over them is enough.
    let parents: HashMap<OpId, (&fitcom_store::Message, String)> = snap
        .timeline
        .messages
        .iter()
        .filter(|m| belongs_to_channel(channel, me, m.channel, m.author))
        .map(|m| {
            let mut body = m.body.clone();
            body.truncate(REPLY_SNIPPET_LEN);
            (m.id, (m, body))
        })
        .collect();

    // Reactions per message id, in the store's own order — already sorted the same way
    // on every peer.
    let reactions_of = |id: OpId| -> Vec<ReactionState> {
        snap.timeline
            .reactions
            .iter()
            .filter(|r| r.target == id)
            .map(|r| ReactionState {
                emoji: r.emoji.clone(),
                count: r.peers.len(),
                peers: r.peers.iter().map(|p| p.to_string()).collect(),
                mine: r.peers.contains(&me),
            })
            .collect()
    };

    for m in &snap.timeline.messages {
        if !belongs_to_channel(channel, me, m.channel, m.author) {
            continue;
        }
        // A parent that is deleted or not yet known still travels as a reference; the
        // frontend shows "original unavailable" when there is no snippet.
        let snippet = m.reply_to.and_then(|p| {
            parents.get(&p).map(|(parent, body)| ReplySnippet {
                author_name: name_of(parent.author),
                body: body.clone(),
            })
        });
        items.push((
            m.lamport,
            m.author,
            m.created_at,
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
                reply_to: m.reply_to.map(OpRef::of),
                reply_snippet: snippet,
                reactions: reactions_of(m.id),
            },
        ));
    }

    for f in &snap.files {
        if !belongs_to_channel(channel, me, f.channel, f.author) {
            continue;
        }
        let image_path = files::is_afbeelding(&f.name)
            .then(|| {
                snap.pictures_dir
                    .join(files::hash_bestandsnaam(&f.hash, &f.name))
            })
            .filter(|p| p.exists())
            .map(|p| p.display().to_string());
        // An image resolves through the content-addressed path even when the engine has
        // no record of it — that path is derived, not remembered, so it survives a
        // restart on its own. Everything else leans on `FileView::local_path`.
        let local_path = image_path.clone().or_else(|| {
            f.local_path
                .as_deref()
                .filter(|p| p.exists())
                .map(|p| p.display().to_string())
        });

        items.push((
            f.lamport,
            f.author,
            0,
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
                local_path,
                opens_folder: files::opent_als_code(&f.name),
            },
        ));
    }

    items.sort_by_key(|(lamport, author, _, _)| (*lamport, *author));

    // The daily Wordle card is not an op, so it has no lamport to sort on — it is placed
    // by the clock instead, right before the first thing said after 07:00 that day. A file
    // carries no clock of its own (`at == 0`) and inherits the last one seen, the same way
    // the frontend draws it.
    let mut kaarten = if channel == Channel::GENERAL {
        wordle_cards(snap, me, &hues).into_iter().peekable()
    } else {
        Vec::new().into_iter().peekable()
    };
    let mut uit: Vec<TimelineItem> = Vec::with_capacity(items.len());
    let mut laatste = 0i64;
    for (_, _, at, item) in items {
        let at = if at > 0 { at } else { laatste };
        while kaarten.peek().is_some_and(|(op, _)| *op <= at) {
            if let Some((_, kaart)) = kaarten.next() {
                uit.push(kaart);
            }
        }
        laatste = at;
        uit.push(item);
    }
    uit.extend(kaarten.map(|(_, kaart)| kaart));
    uit
}

/// The board and the leaderboard. Kept out of `UiState::build` only because that function
/// is long enough; nothing here decides anything the engine has not already decided.
fn wordle_state(snap: &Snapshot, me: PeerId, hues: &HashMap<PeerId, u8>) -> WordleState {
    let standings = wordle::standen(&snap.timeline.wordle)
        .into_iter()
        .map(|st| WordleStanding {
            peer: st.peer.to_string(),
            name: display_name(snap, st.peer, &st.peer.to_string()[..8]),
            avatar: hues.get(&st.peer).copied().unwrap_or(3),
            mine: st.peer == me,
            points: st.punten,
            played: st.gespeeld,
            solved: st.opgelost,
        })
        .collect();

    WordleState {
        day: snap.wordle.dag,
        board: snap.wordle.bord.as_ref().map(|b| WordleBoard {
            number: b.nummer,
            rows: b
                .rijen
                .iter()
                .map(|r| WordleRow {
                    word: r.woord.clone(),
                    marks: r.tekens.iter().map(|t| *t as u8).collect(),
                })
                .collect(),
            done: b.klaar,
            won: b.gewonnen,
            solution: b.oplossing.clone(),
            seconds: b.seconden,
        }),
        error: snap.wordle.fout.clone(),
        standings,
        tries: wordle::POGINGEN,
    }
}

/// One card per Wordle day, with the time it belongs at. Sorted oldest first.
///
/// A day shows up here as soon as *either* this machine fetched its word, *or* somebody's
/// result for it arrived, *or* somebody put the card in the chat by hand (the + menu, an
/// `OpKind::WordleCard`). The second case is the day you were away and the others played;
/// the third is the day your own fetch failed, so you have no word and would otherwise draw
/// nothing at all.
///
/// Never a day beyond the current one: a peer with a fast clock must not be able to put
/// tomorrow's card in today's log.
///
/// **Where the card sits.** Normally at 07:00, the hour it belongs to. A day somebody
/// announced by hand sits at the moment they pressed the button instead — that is the whole
/// point of the button, since a card at 07:00 is buried above a day's worth of messages.
/// That time comes off another machine's clock, so it is clamped to `[07:00, now]`: a peer
/// whose clock is wrong may not fling the card to 1970 or into next week.
fn wordle_cards(
    snap: &Snapshot,
    me: PeerId,
    hues: &HashMap<PeerId, u8>,
) -> Vec<(i64, TimelineItem)> {
    let per_dag: HashMap<u32, &[fitcom_store::WordleEntry]> =
        wordle::per_dag(&snap.timeline.wordle)
            .into_iter()
            .filter_map(|groep| groep.first().map(|e| (e.day, groep)))
            .collect();

    let kaarten = &snap.timeline.wordle_cards;

    let mut dagen: Vec<u32> = snap
        .wordle
        .nummers
        .keys()
        .copied()
        .chain(per_dag.keys().copied())
        .chain(kaarten.keys().copied())
        .filter(|d| *d <= snap.wordle.dag)
        .collect();
    dagen.sort_unstable();
    dagen.dedup();

    dagen
        .into_iter()
        .map(|day| {
            let groep = per_dag.get(&day).copied().unwrap_or(&[]);
            let winners = wordle::winnaars(groep);
            let mut results: Vec<WordleScore> = groep
                .iter()
                .map(|e| WordleScore {
                    peer: e.author.to_string(),
                    name: display_name(snap, e.author, &e.author.to_string()[..8]),
                    avatar: hues.get(&e.author).copied().unwrap_or(3),
                    mine: e.author == me,
                    guesses: e.guesses,
                    solved: e.solved,
                    pattern: e.pattern.clone(),
                    seconds: e.seconds,
                    won: winners.contains(&e.author),
                })
                .collect();
            // Best first: whoever solved it, in the fewest guesses, and then the quickest
            // — the same order the point is handed out in, so the winner sits on top. An
            // unmeasured time sorts last, exactly as it scores. A peer id breaks what is
            // left over, so every machine draws the same order.
            fn sorteersleutel(s: &WordleScore) -> (bool, u8, u32, &str) {
                (!s.solved, s.guesses, s.seconds.unwrap_or(u32::MAX), &s.peer)
            }
            results.sort_by(|a, b| sorteersleutel(a).cmp(&sorteersleutel(b)));

            let vandaag = day == snap.wordle.dag;
            let action = match (vandaag, snap.wordle.bord.as_ref()) {
                (false, _) => "past",
                (true, None) => "waiting",
                (true, Some(b)) if b.klaar => "done",
                (true, Some(b)) if b.rijen.is_empty() => "play",
                (true, Some(_)) => "continue",
            };

            // 07:00, tenzij iemand de kaart met de hand in de chat zette — dan daar, maar
            // nooit vóór 07:00 en nooit in de toekomst. Zie de doc hierboven.
            // Niet `clamp`: die paniekt als de ondergrens boven de bovengrens uitkomt, en
            // dat mag nooit van een klok afhangen in de tekenlaag.
            let op_moment = kaarten.get(&day).map_or(wordle::openbaar_op(day), |k| {
                k.at.max(wordle::openbaar_op(day))
                    .min(fitcom_store::now_millis().max(wordle::openbaar_op(day)))
            });

            (
                op_moment,
                TimelineItem::Wordle {
                    day,
                    at: op_moment,
                    // Wat deze pc zelf weet gaat voor; het nummer uit de aankondiging is
                    // de terugval voor de dag die we nooit opgehaald kregen. `0` betekent
                    // daar "onbekend", en dan tekent de kaart gewoon geen nummer.
                    number: snap
                        .wordle
                        .nummers
                        .get(&day)
                        .copied()
                        .or_else(|| kaarten.get(&day).map(|k| k.number).filter(|n| *n != 0)),
                    action: action.to_string(),
                    progress: if vandaag {
                        snap.wordle.bord.as_ref().map_or(0, |b| b.rijen.len() as u8)
                    } else {
                        0
                    },
                    results,
                    scored: wordle::telt_mee(groep),
                },
            )
        })
        .collect()
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

    /// The card is not an op, so it has no lamport — it is slotted in by the clock. This
    /// is the test for that seam: 07:00 comes after anything said at 06:00 and before
    /// anything said at 08:00, on the day it belongs to.
    #[test]
    fn the_wordle_card_lands_at_seven_in_the_morning() {
        let dag = 20_260_820;
        let zeven = crate::wordle::openbaar_op(dag);
        let uur = 3_600_000;

        let bericht = |lamport: u64, at: i64| fitcom_store::Message {
            id: OpId::new(peer(1), Channel::GENERAL, lamport),
            author: peer(1),
            channel: Channel::GENERAL,
            body: format!("at {at}"),
            created_at: at,
            edited: false,
            lamport,
            reply_to: None,
        };
        let timeline = fitcom_store::Timeline {
            messages: vec![
                bericht(1, zeven - uur),
                bericht(2, zeven + uur),
                bericht(3, zeven + 2 * uur),
            ],
            wordle: vec![fitcom_store::WordleEntry {
                day: dag,
                author: peer(2),
                guesses: 3,
                solved: true,
                pattern: "2".repeat(15),
                seconds: Some(120),
            }],
            ..Default::default()
        };
        let snap = Snapshot {
            timeline: std::sync::Arc::new(timeline),
            wordle: crate::engine::WordleView {
                dag,
                nummers: [(dag, 1888)].into_iter().collect(),
                ..Default::default()
            },
            ..Default::default()
        };

        let soorten: Vec<&str> = timeline_of(&snap, Channel::GENERAL, peer(1), "me")
            .iter()
            .map(|i| match i {
                TimelineItem::Wordle { .. } => "card",
                _ => "message",
            })
            .collect();
        assert_eq!(soorten, ["message", "card", "message", "message"]);

        // And nowhere else: a DM is not where the daily puzzle shows up.
        assert!(!timeline_of(&snap, Channel::dm(peer(2)), peer(1), "me")
            .iter()
            .any(|i| matches!(i, TimelineItem::Wordle { .. })));
    }

    /// A day this machine never fetched still gets a card as soon as somebody's result
    /// arrives — that is the day you were away — but a day beyond the current one never
    /// does, however far ahead the other peer's clock runs.
    #[test]
    fn a_card_appears_for_a_missed_day_and_never_for_a_future_one() {
        let vandaag = 20_260_820;
        let timeline = fitcom_store::Timeline {
            wordle: vec![
                fitcom_store::WordleEntry {
                    day: 20_260_819,
                    author: peer(2),
                    guesses: 4,
                    solved: true,
                    pattern: String::new(),
                    seconds: None,
                },
                fitcom_store::WordleEntry {
                    day: 20_260_821,
                    author: peer(2),
                    guesses: 2,
                    solved: true,
                    pattern: String::new(),
                    seconds: None,
                },
            ],
            ..Default::default()
        };
        let snap = Snapshot {
            timeline: std::sync::Arc::new(timeline),
            wordle: crate::engine::WordleView {
                dag: vandaag,
                ..Default::default()
            },
            ..Default::default()
        };
        let dagen: Vec<u32> = timeline_of(&snap, Channel::GENERAL, peer(1), "me")
            .iter()
            .filter_map(|i| match i {
                TimelineItem::Wordle { day, .. } => Some(*day),
                _ => None,
            })
            .collect();
        assert_eq!(dagen, [20_260_819], "morgen hoort er niet bij te staan");
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
                user_status: None,
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
