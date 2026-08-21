"use strict";

/* FitCommunication — the window.
 *
 * Ported from the approved comp `design/main-window.html`. The comp's render functions
 * are kept shape for shape; what changed is where the state comes from. The comp seeded
 * its own and had a review harness to flip it. Here it arrives from the engine over
 * three channels, and every control sends a command back:
 *
 *   state      everything structural, emitted only when it actually changed
 *   meters     speaking level and RTT at 4 Hz, patched into attributes, never re-rendered
 *   thumbnail  the stream strip at 2 fps, as a `thumb://` URL per tile
 *
 * Nothing here decides anything. It draws a snapshot and sends commands, exactly as the
 * egui build did — that boundary is the reason this stack swap was affordable.
 */

const { invoke, convertFileSrc } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const { getCurrentWindow } = window.__TAURI__.window;

/* macOS gets native traffic lights (titleBarStyle Overlay): hide our own window
   buttons via the body class, and say ⌘ where Windows says Ctrl. */
if (navigator.platform.startsWith("Mac")) {
  document.body.classList.add("mac");
  for (const k of document.querySelectorAll(".composer-hint kbd")) {
    if (k.textContent === "Ctrl") k.textContent = "⌘";
  }
}

/* ---------------------------------------------------------------- state */

/** Last state from the engine. Replaced wholesale; never mutated. */
let S = null;
/** Timeline of the open conversation, fetched on demand. */
let TL = [];
/** Live speaking levels and RTT, from `meters`. */
let M = { peers: {}, self: { level: 0 } };

/** What this window is looking at. Purely local: none of it outlives the process. */
const V = {
  view: "channels",          // channels | dms | settings
  channel: "general",        // last non-DM channel
  dm: null,                  // last opened DM peer id
  members: true,
  settingsTab: "account",
  overlay: "none",           // none | ac | drop | plus
  editing: null,             // OpRef of the message being edited
  /** Reply being composed: { op, name, body } shown as a chip above the composer. */
  replyTo: null,
  /** OpRef whose emoji quick-bar is open, or null. */
  emojiFor: null,
  /** Last moment we told the engine we were typing; one notification per window. */
  lastTypingSent: 0,
  acIndex: 0,
  acMatches: [],
  /** Half-typed messages, per conversation. Switching away must not lose them, and must
      not carry them into the wrong conversation either. */
  drafts: {},
  /** Muting a peer is volume 0 with the old level remembered, because that is all the
      engine has — there is no separate per-peer mute on the wire. */
  volumeBeforeMute: {},
  focused: true,
};

/** The level above which a peer counts as speaking. Matches the voice VAD closely
    enough that the ring does not light on room noise. */
const SPEAKING = 0.06;

/** How long the speaking state survives a dip under the threshold. Natural speech dips
    below any level threshold between words and syllables, and the 4 Hz meters tick
    samples straight into those dips — gating the ring on a single sample made it strobe
    while someone was talking continuously. Real VADs call the cure hangover: light the
    moment the level is up, release only once it has stayed quiet this long. Two meter
    ticks plus slack, so one or two quiet samples inside a sentence never show. */
const SPEAK_HOLD_MS = 600;

/** Per key: the last moment the level was above the threshold. The self entry is keyed
    "@self" — peer ids are UUIDs, so the prefix cannot collide. */
const speakAt = {};

function heldSpeaking(key, level) {
  const now = Date.now();
  if ((level || 0) > SPEAKING) { speakAt[key] = now; return true; }
  return now - (speakAt[key] || 0) < SPEAK_HOLD_MS;
}

const $ = id => document.getElementById(id);
const esc = s => String(s).replace(/[&<>"']/g, c =>
  ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));

const ic = (id, cls = "icon", extra = "") =>
  `<svg class="${cls}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="square" stroke-linejoin="miter" ${extra}><use href="#${id}"/></svg>`;

/* ---------------------------------------------------------------- derived */

const activeChannel = () => (V.view === "dms" && V.dm ? `dm:${V.dm}` : V.channel);
const peerById = id => (S.peers || []).find(p => p.id === id);
const channelByKey = key => (S.channels || []).find(c => c.key === key);
const knownPeers = () => (S.peers || []).filter(p => p.id);
const onlinePeers = () => knownPeers().filter(p => p.presence === "online");
const callPeers = () => knownPeers().filter(p => p.in_call);
const callRunning = () => callPeers().length > 0 || S.voice.joined;
const totalDmUnread = () => knownPeers().reduce((n, p) => n + p.unread, 0);
const sharingSelf = () => (S.own_streams || []).some(s => !s.is_audio && !s.is_camera);
const watchedStreams = () => (S.streams || []).filter(s => s.watching);
const isSpeaking = id => heldSpeaking(id, M.peers[id]?.level);
/* joined/muted gate first: while muted no timestamp is refreshed, so muting mid-sentence
   drops the ring immediately instead of letting the hangover keep it lit. */
const meSpeaking = () => S.voice.joined && !S.voice.muted && heldSpeaking("@self", M.self.level);

const activeName = () => {
  if (V.view === "dms" && V.dm) return peerById(V.dm)?.name || "";
  return channelByKey(V.channel)?.name || "general";
};

/* Presence for the roster and the status bar. `self` is never offline and turns rose
   while do-not-disturb is on, which is a state, not a fault. */
const selfPresence = () => (S.do_not_disturb ? "dnd" : "online");

const fmtRtt = id => {
  const rtt = M.peers[id]?.rtt;
  return rtt === null || rtt === undefined ? "—" : `${rtt} ms`;
};

function fmtSize(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["kB", "MB", "GB", "TB"];
  let v = bytes / 1024, i = 0;
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
  return `${v < 10 ? v.toFixed(1) : Math.round(v)} ${units[i]}`;
}

const mbit = bits => Math.round(bits / 1_000_000);
/** A 0..1 volume as whole percents, which is what every slider in this window speaks. */
const volPct = v => Math.round((v || 0) * 100);

const fmtTime = ms => new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });

/* Only ever from a moment this process actually observed. A peer that has not been up
   since the app started has no time to show, and inventing one would be worse than the
   plain word. */
function lastSeenLine(p) {
  if (!p.last_seen) return "Offline";
  const d = new Date(p.last_seen);
  const day = fmtDay(p.last_seen);
  return `Last seen ${fmtTime(p.last_seen)}${day === "Today" ? "" : `, ${d.toLocaleDateString([], { day: "numeric", month: "long" })}`}`;
}

function fmtDay(ms) {
  const d = new Date(ms);
  const today = new Date();
  const same = (a, b) => a.toDateString() === b.toDateString();
  if (same(d, today)) return "Today";
  const yesterday = new Date(today.getTime() - 86400000);
  if (same(d, yesterday)) return "Yesterday";
  return d.toLocaleDateString([], { day: "numeric", month: "long" });
}

/* ---------------------------------------------------------------- avatars */

function avatar(peer, size = 32, dot = false, ring = false) {
  const initial = (peer.name || "?").trim().charAt(0).toUpperCase() || "?";
  /* A chosen status ("away", "busy") wins over the connection state while it exists —
     the engine only sends one for a peer that is online. */
  const state = peer.user_status || (peer.self ? selfPresence() : peer.presence);
  const d = dot ? `<i class="dot" data-state="${state}"></i>` : "";
  return `<span class="av-wrap${ring ? " speak-ring" : ""}"><span class="avatar avatar--${size} av-${peer.avatar}">${esc(initial)}</span>${d}</span>`;
}

/** The label a chosen status gets in rosters and tooltips. */
const statusLabel = s => ({ away: "Away", busy: "Busy" }[s] || "");

/** The engine's own peer, shaped like the others so one avatar function covers both. */
const selfPeer = () => ({ ...S.self, presence: selfPresence(), self: true });

/** Our own camera stream, or undefined. The camera button is a mirror of this. */
const ownCamera = () => (S.own_streams || []).find(s => s.is_camera);

/* ---------------------------------------------------------------- message body */

/* Chat content is Dutch and comes from another machine, so everything is escaped first
   and only then given structure. Three things get structure: fenced code, links, and
   `@name` for a name that actually exists. */
function renderBody(text, mentionsYou) {
  const parts = String(text).split(/```/);
  return parts.map((part, i) => (i % 2 === 1
    ? codeBlock(part)
    : `<div class="msg-text">${inline(part, mentionsYou)}</div>`))
    .filter((_, i) => i % 2 === 1 || parts[i].trim() !== "")
    .join("");
}

/* The header carries whatever follows the opening fence — a filename is what actually
   gets pasted here — and a Copy button, because the reason to paste a config block is
   for somebody else to use it. */
function codeBlock(part) {
  const nl = part.indexOf("\n");
  const info = (nl === -1 ? "" : part.slice(0, nl)).trim();
  const code = nl === -1 ? part : part.slice(nl + 1).replace(/\n$/, "");
  return `<div class="code">
    <div class="code-head">${esc(info || "code")}<button class="code-copy" data-copy="${esc(code)}">Copy</button></div>
    <pre>${highlight(code)}</pre>
  </div>`;
}

/* B-49: mentions are resolved BEFORE linkification, not after.
   The other way round, the @name pass ran over a string that already contained markup, so
   a mention inside a URL got rewritten inside the `href` attribute — producing a link whose
   target differs from its visible text. Not executable (the inserted markup is fixed, so
   the tag always closes on its own `>`), but misleading, which is bad enough for a link.
   Doing mentions first means linkification only ever sees a URL it produced itself. */
function inline(text, mentionsYou) {
  let html = esc(text).replace(/\n/g, "<br>");
  const names = [S.self.name, ...knownPeers().map(p => p.name)].filter(Boolean);
  for (const name of names) {
    const mine = name === S.self.name;
    const re = new RegExp(`@${name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\b`, "gi");
    html = html.replace(re, m =>
      `<span class="mention${mine && mentionsYou ? " mention--self" : ""}">${esc(m)}</span>`);
  }
  /* Skip anything already inside a tag, so a mention span cannot be re-scanned as a URL. */
  html = html.replace(/(https?:\/\/[^\s<]+)/g, u => `<a href="${u}" target="_blank" rel="noreferrer">${u}</a>`);
  return html;
}

/* The three code hues exist for the kind of thing that actually gets pasted here: a
   config file. Comments, strings and numbers, and the key before an `=`. Anything else
   stays body text — a fourth hue is not in the system. */
/* B-58: the number and comment passes must not see an HTML entity.
   `highlight()` runs over already-escaped text, so an apostrophe is `&#39;` — the digits in
   which the number rule tokenised as a number and the `#` in which the comment rule
   tokenised as a comment, mangling the entity. Purely cosmetic (all inserted markup is
   fixed), but it garbles pasted config. Fixed by splitting on entities and only
   highlighting the parts between them. */
function highlight(code) {
  const tokenise = line => line
    .replace(/^(\s*)([A-Za-z_][\w.-]*)(\s*=)/, '$1<span class="tok-key">$2</span>$3')
    .replace(/(&quot;[^&]*?&quot;)/g, '<span class="tok-str">$1</span>')
    .replace(/\b(\d[\d_.]*)\b/g, '<span class="tok-num">$1</span>')
    .replace(/(#.*)$/, '<span class="tok-com">$1</span>');

  return esc(code)
    .split("\n")
    .map(line => {
      /* Keep &...; runs out of the tokeniser's reach, then put them back verbatim. */
      const entities = [];
      const masked = line.replace(/&[a-z]+;|&#\d+;/gi, m => {
        entities.push(m);
        return `\u0000${entities.length - 1}\u0000`;
      });
      return tokenise(masked).replace(/\u0000(\d+)\u0000/g, (_, i) => entities[Number(i)]);
    })
    .join("\n");
}

/* ---------------------------------------------------------------- channel column */

function channelRow(c) {
  const active = V.view === "channels" && V.channel === c.key;
  const unread = c.unread > 0;
  return `<button class="chan-item" data-channel="${esc(c.key)}" ${active ? 'aria-current="true"' : ""} data-unread="${unread}">
    ${unread ? '<span class="unread-dot"></span>' : ""}
    ${ic("i-hash")}
    <span class="chan-name">${esc(c.name)}</span>
    ${unread ? `<span class="badge num">${c.unread}</span>` : ""}
  </button>`;
}

function dmRow(p, active) {
  const unread = p.unread > 0;
  return `<button class="chan-item" data-dm="${esc(p.id)}" ${active ? 'aria-current="true"' : ""} data-unread="${unread}">
    ${unread ? '<span class="unread-dot"></span>' : ""}
    ${avatar(p, 20, true)}
    <span class="chan-name">${esc(p.name)}</span>
    ${unread ? `<span class="badge num">${p.unread}</span>` : ""}
  </button>`;
}

function renderChannels() {
  const el = $("chan-scroll");
  const title = $("chan-head-title");
  const dms = knownPeers();

  if (V.view === "dms") {
    title.textContent = "Direct messages";
    el.innerHTML = dms.length
      ? `<div class="group">
           <div class="group-head">Conversations<span class="group-count num">${dms.length}</span></div>
           ${dms.map(p => dmRow(p, V.dm === p.id)).join("")}
         </div>`
      : `<p class="voice-hint">No peer has introduced itself yet. Identities are learned on
         first contact, so a conversation appears here as soon as somebody connects.</p>`;
    return;
  }

  title.textContent = "Channels";
  el.innerHTML = `
    <div class="group">
      <div class="group-head">
        <button id="collapse-general" title="${V.collapsed ? "Expand" : "Collapse"}"
                aria-expanded="${!V.collapsed}" style="transform:rotate(${V.collapsed ? -90 : 0}deg)">
          ${ic("i-chev", "icon", 'style="width:12px;height:12px"')}
          <span class="sr">${V.collapsed ? "Expand" : "Collapse"} the channel list</span>
        </button>
        General<span class="group-count num">${S.channels.length}</span>
      </div>
      ${V.collapsed
        ? S.channels.filter(c => c.unread > 0 || c.key === V.channel).map(channelRow).join("")
        : S.channels.map(channelRow).join("")}
    </div>
    ${dms.length ? `<div class="group">
      <div class="group-head">Direct messages<span class="group-count num">${dms.length}</span></div>
      ${dms.map(p => dmRow(p, false)).join("")}
    </div>` : ""}`;
}

/* ---------------------------------------------------------------- voice panel */

/* B-56: the names are peer-controlled (OpKind::SetNick, no length or content validation)
   and this string lands in innerHTML, so it escapes here in the sink. Unreachable today —
   this branch only runs with an empty roster, which by definition has no names in it — but
   that is one refactor away from being a real injection. */
function voiceHint() {
  const others = callPeers();
  if (others.length > 1) {
    const names = others.map(p => esc(p.name));
    return `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]} are in the call.`;
  }
  if (others.length === 1) return `${esc(others[0].name)} is in the call.`;
  if (onlinePeers().length === 0) {
    return "Nobody is online. Joining anyway is fine — the others arrive in the call when they come back.";
  }
  return "Nobody is in the call.";
}

function renderVoice() {
  const el = $("voice");
  const others = callPeers();
  const roster = S.voice.joined ? [selfPeer(), ...others] : others;
  /* Screens only: the camera has its own button and its own line, so it must not turn
     "Share screen" into "Stop sharing". */
  const sharing = (S.own_streams || []).filter(s => !s.is_audio && !s.is_camera);
  let live = "";

  if (roster.length) {
    const rows = roster.map(p => {
      const me = !!p.self;
      const muted = me ? S.voice.muted : (p.volume || 0) === 0;
      return `<div class="voice-peer" data-id="${esc(p.id)}" data-speaking="${me ? meSpeaking() : isSpeaking(p.id)}">
        ${avatar(p, 24, false, true)}
        <span class="voice-peer-name">${esc(p.name)}${me ? " (you)" : ""}</span>
        ${muted ? ic("i-mic-off", "icon", 'data-muted="1"') : ""}
      </div>`;
    }).join("");

    /* The head reports the worst link in the call, not a number of its own. Patched by
       the meters event, so it never forces a re-render. */
    live = `<div class="voice-live">
      <div class="voice-live-head">
        ${ic("i-wave")}
        ${S.voice.joined ? "Voice connected" : "Call running"}
        <span class="voice-rtt mono" id="worst-rtt">—</span>
      </div>
      <div class="voice-peers">${rows}</div>
      ${sharing.length ? `<p class="voice-share">${ic("i-monitor")}<span>Sharing <b>${esc(sharing[0].title)}</b>${sharing.length > 1 ? ` and ${sharing.length - 1} more` : ""} &middot; desktop audio follows automatically</span></p>` : ""}
      ${ownCamera() ? `<p class="voice-share">${ic("i-cam")}<span>Camera on &middot; <b>${esc(ownCamera().title)}</b></span></p>` : ""}
      ${S.voice.joined
        ? `<div class="voice-acts">
             <button class="btn btn--ghost" id="btn-share">${ic(sharing.length ? "i-x" : "i-share")}${sharing.length ? "Stop sharing" : "Share screen"}</button>
             <button class="leave" id="btn-leave">${ic("i-leave")}Leave</button>
           </div>`
        : `<button class="join" id="btn-join">${ic("i-wave")}Join the call</button>`}
    </div>`;
  } else {
    live = `<button class="join" id="btn-join">${ic("i-wave")}Join voice</button>
      <p class="voice-hint">${voiceHint()}</p>`;
  }

  const me = selfPeer();
  el.innerHTML = `${live}
    <div class="self" data-speaking="${meSpeaking()}">
      ${avatar(me, 32, true, true)}
      <div class="self-id">
        <div class="self-name">${esc(me.name)}</div>
        <div class="self-sub">${S.do_not_disturb ? "Do not disturb" : S.voice.joined ? "In the call" : "Online"}</div>
      </div>
      <div class="self-actions">
        <button class="self-btn" id="btn-mic" aria-pressed="${S.voice.muted}" title="${S.voice.muted ? "Unmute microphone" : "Mute microphone"}">
          ${ic(S.voice.muted ? "i-mic-off" : "i-mic", "icon-18 icon")}
          <span class="sr">${S.voice.muted ? "Unmute microphone" : "Mute microphone"}</span>
        </button>
        <button class="self-btn self-btn--on" id="btn-cam" aria-pressed="${!!ownCamera()}" title="${ownCamera() ? "Turn the camera off" : "Turn the camera on"}">
          ${ic(ownCamera() ? "i-cam" : "i-cam-off", "icon-18 icon")}
          <span class="sr">${ownCamera() ? "Turn the camera off" : "Turn the camera on"}</span>
        </button>
        <button class="self-btn" id="btn-deaf" aria-pressed="${S.voice.deafened}" title="${S.voice.deafened ? "Undeafen" : "Deafen"}">
          ${ic(S.voice.deafened ? "i-head-off" : "i-head", "icon-18 icon")}
          <span class="sr">${S.voice.deafened ? "Undeafen" : "Deafen"}</span>
        </button>
        <button class="self-btn" id="btn-dnd" aria-pressed="${S.do_not_disturb}" title="${S.do_not_disturb ? "Turn off do not disturb" : "Do not disturb"}">
          ${ic("i-moon", "icon-18 icon")}
          <span class="sr">Do not disturb</span>
        </button>
      </div>
    </div>`;
}

/* ---------------------------------------------------------------- timeline */

function attachmentCard(item) {
  const t = item.transfer;
  const author = peerById(item.author);
  /* "Paused" is not a state the engine has; it is a running transfer whose offerer went
     away. Deriving it here is what keeps the line from claiming progress that stopped. */
  const stalled = t.state === "running" && author && author.presence !== "online";

  if (t.state === "mine") {
    return `<div class="att">
      <span class="att-ico">${ic("i-file", "icon-18 icon")}</span>
      <span class="att-meta">
        <span class="att-name">${esc(item.name)}</span>
        <span class="att-sub num">${fmtSize(item.size)} &middot; offered by you</span>
      </span>
      ${openButton(item)}
    </div>`;
  }
  if (t.state === "done") {
    return `<div class="att">
      <span class="att-ico">${ic("i-file", "icon-18 icon")}</span>
      <span class="att-meta">
        <span class="att-name">${esc(item.name)}</span>
        <span class="att-sub num">${fmtSize(item.size)} &middot; in your download folder</span>
      </span>
      ${openButton(item) || `<button class="btn btn--ghost" disabled>${ic("i-check")}Downloaded</button>`}
    </div>`;
  }
  if (t.state === "failed") {
    return `<div class="att">
      <span class="att-ico" data-error>${ic("i-alert", "icon-18 icon")}</span>
      <span class="att-meta">
        <span class="att-name">${esc(item.name)}</span>
        <span class="att-sub" data-error>${esc(t.error)}</span>
      </span>
      <button class="btn" data-download='${opAttr(item.id)}'>${ic("i-retry")}Try again</button>
    </div>`;
  }
  if (t.state === "running") {
    const pct = t.total ? t.received / t.total : 0;
    return `<div class="att">
      <span class="att-ico">${ic("i-file", "icon-18 icon")}</span>
      <span class="att-meta">
        <span class="att-name">${esc(item.name)}</span>
        <span class="att-sub num">${stalled
          ? `Paused at ${fmtSize(t.received)} of ${fmtSize(t.total)} &middot; ${esc(author.name)} went offline`
          : `${fmtSize(t.received)} of ${fmtSize(t.total)} &middot; ${Math.round(pct * 100)}%`}</span>
        <span class="bar"${stalled ? " data-paused" : ""}><i style="transform:scaleX(${pct.toFixed(3)})"></i></span>
        ${stalled ? `<span class="att-sub">Continues from ${fmtSize(t.received)} by itself when he is back. Nothing already transferred is sent again.</span>` : ""}
      </span>
      <button class="btn" disabled>${stalled ? "Waiting" : "Downloading"}</button>
    </div>`;
  }
  return `<div class="att">
    <span class="att-ico">${ic("i-file", "icon-18 icon")}</span>
    <span class="att-meta">
      <span class="att-name">${esc(item.name)}</span>
      <span class="att-sub num">${fmtSize(item.size)}</span>
    </span>
    <button class="btn" data-download='${opAttr(item.id)}'>${ic("i-download")}Download</button>
  </div>`;
}

const opAttr = op => esc(JSON.stringify(op));

/* ------------------------------------------------------------ youtube cards

   A link to a video shows its title and its thumbnail. The fetching is the engine's job
   (`crate::youtube`) — nothing on this side ever talks to youtube.com, so a message from a
   peer cannot make this window open a connection. What arrives here is a title, a channel
   name and a path to a JPEG on our own disk.

   Everything is keyed by the eleven-character video id: one entry per video, no matter how
   many times or in how many messages the link appears.
     undefined  never asked
     "pending"  asked, waiting
     null       asked, nothing came back (offline, deleted video) — stays a plain link
     object     the card */
const YT_ID = /https?:\/\/(?:www\.|m\.|music\.)?(?:youtube\.com\/(?:watch\?(?:[\w=&%.-]*&)?v=|shorts\/|live\/|embed\/|v\/)|youtu\.be\/)([\w-]{11})/g;
const ytPreviews = {};

/** The video ids in one message body, in order, without repeats. Two at most: a card is
    tall, and a message that pastes five links should stay a message. */
function ytIds(text) {
  const ids = [...String(text).matchAll(YT_ID)].map(m => m[1]);
  return [...new Set(ids)].slice(0, 2);
}

/* A slot and not the card itself: while the answer is still coming there is nothing
   sensible to draw, and an empty box that turns into a card moves the whole log under the
   reader's eyes. The slot has no size until it is filled. */
const ytReady = id => ytPreviews[id] instanceof Object;
const ytSlots = text => ytIds(text).map(id =>
  `<div class="yt-slot" data-yt="${esc(id)}">${ytReady(id) ? ytCard(id, ytPreviews[id]) : ""}</div>`).join("");

const ytCard = (id, p) => `<a class="yt" href="https://www.youtube.com/watch?v=${esc(id)}">
    <span class="yt-thumb">
      <img src="${esc(convertFileSrc(p.thumbnail))}" alt="">
      <span class="yt-play">${ic("i-play", "icon-20 icon")}</span>
    </span>
    <span class="yt-meta">
      <span class="yt-title">${esc(p.title)}</span>
      <span class="yt-sub">YouTube${p.author ? ` &middot; ${esc(p.author)}` : ""}</span>
    </span>
  </a>`;

/** Fills the slots the last render left empty. One call per video for the life of the
    process; the engine caches on disk, so it is also one call per video ever. */
async function hydrateYt() {
  const slots = [...document.querySelectorAll(".yt-slot[data-yt]")];
  const wanted = [...new Set(slots.map(s => s.dataset.yt))]
    .filter(id => ytPreviews[id] === undefined);
  wanted.forEach(id => { ytPreviews[id] = "pending"; });

  for (const id of wanted) {
    let preview = null;
    try {
      preview = await invoke("youtube_preview", { id });
    } catch (e) {
      console.warn("youtube preview failed", e);
    }
    ytPreviews[id] = preview;
    if (!preview) continue;
    const tl = $("timeline");
    const pinned = tl ? wasPinned(tl) : false;
    document.querySelectorAll(`.yt-slot[data-yt="${CSS.escape(id)}"]`)
      .forEach(slot => { slot.innerHTML = ytCard(id, preview); });
    /* A card is ~100px tall. Landing one under a log that was scrolled to the bottom must
       not push the newest message out of sight. */
    if (tl && pinned) repin(tl, true);
  }
}

/* The download button becomes an open button once the bytes are on this machine — same
   card, same place in the log, so "where did that file go" is answered where it was
   offered instead of in a file manager.

   `local_path` is the engine's answer to "do we have it", not a path this side may act
   on: what goes back over IPC is the same `OpRef` the download used, and `open_file`
   looks the path up again on its own (see `ui/commands.rs`). Absent for a finished
   download whose file has since been moved or deleted.

   Something the system would *run* rather than open gets its folder instead, so the label
   says Show. Promising Open and then doing something else is worse than the restriction
   itself. */
const openButton = item => {
  if (!item.local_path) return "";
  if (item.opens_folder) {
    return `<button class="btn" data-open='${opAttr(item.id)}'
      title="Show it in its folder. A file the system would run is never started from here.">${ic("i-open")}Show</button>`;
  }
  return `<button class="btn" data-open='${opAttr(item.id)}'>${ic("i-open")}Open</button>`;
};

function itemContent(item) {
  if (item.kind === "message") {
    if (V.editing && sameOp(V.editing, item.id)) {
      return `<div class="msg-edit">
        <textarea id="edit-input">${esc(item.body)}</textarea>
        <p class="msg-edit-hint"><b>Enter</b> saves &middot; <b>Esc</b> cancels &middot; empty deletes</p>
      </div>`;
    }
    return renderBody(item.body, item.mentions_you) + ytSlots(item.body);
  }
  if (item.image_path) {
    /* The picture in the log is a preview at the column's width; clicking it opens the
       real thing. A button and not a bare `<img>` handler, so it is reachable from the
       keyboard like every other action here. */
    const src = convertFileSrc(item.image_path);
    return `<figure class="shot">
      <button class="shot-btn" title="Show at full size"
              data-shot='${esc(JSON.stringify({ src, name: item.name, op: item.id }))}'>
        <img src="${esc(src)}" alt="${esc(item.name)}">
      </button>
      <figcaption class="shot-cap">${esc(item.name)} &middot; <span class="num">${fmtSize(item.size)}</span></figcaption>
    </figure>`;
  }
  return attachmentCard(item);
}

/* ------------------------------------------------------- the puzzle of the day

   The card is not a message and not an op: every machine works out on its own that today
   is today, so there is nothing to tell anybody. It is placed in the log by the clock, at
   07:00 of the day it belongs to (see `ui/state.rs`).

   The squares of the others stay hidden until you have finished today's puzzle. A pattern
   is a real hint — that is why the real Wordle is shared after you play, not before — and
   this card sits in the middle of the conversation where you cannot avoid looking. */

const wordleTitle = item => item.number ? `Wordle ${item.number.toLocaleString()}` : "Wordle";

const wordleButton = item => {
  const tries = S.wordle.tries;
  switch (item.action) {
    case "play":
      return `<button class="btn btn--accent" data-wordle="${item.day}">${ic("i-play")}Play today's puzzle</button>`;
    case "continue":
      return `<button class="btn btn--accent" data-wordle="${item.day}">${ic("i-play")}Continue &middot; ${item.progress}/${tries}</button>`;
    case "done":
      return `<button class="btn btn--ghost" data-wordle="${item.day}">${ic("i-tiles")}Show the board</button>`;
    case "waiting":
      return `<button class="btn" disabled>${ic("i-tiles")}Today's word has not arrived yet</button>`;
    default:
      return `<button class="btn btn--ghost" data-wordle="${item.day}">${ic("i-tiles")}Standings</button>`;
  }
};

/** The shared squares of one result: five to a row, one row per guess made. */
const wordleGrid = pattern => {
  const cells = [...String(pattern)].filter(c => "012".includes(c));
  if (cells.length < 5) return "";
  return `<span class="wdl-grid">${cells.map(c => `<i class="wdl-pip" data-m="${c}"></i>`).join("")}</span>`;
};

function wordleCard(item) {
  const tries = S.wordle.tries;
  /* Their squares are a hint, so they wait until your own game is over. Only today's card
     can be unfinished; a past day has nothing left to spoil. */
  const spoilers = item.action === "past" || item.action === "done";
  const scores = item.results.length
    ? `<ul class="wdl-scores">${item.results.map(r => `<li class="wdl-score" data-won="${r.won}">
        <span class="wdl-who au-${r.avatar}">${esc(r.mine ? "You" : r.name)}</span>
        <span class="wdl-tries num">${r.solved ? `${r.guesses}/${tries}` : "failed"}</span>
        ${spoilers ? wordleGrid(r.pattern) : ""}
      </li>`).join("")}</ul>`
    : "";

  let note = "";
  if (!item.results.length) {
    note = item.action === "past"
      ? "Nobody played this one."
      : "Nobody has played yet today.";
  } else if (!item.scored) {
    const who = item.results[0].mine ? "You" : esc(item.results[0].name);
    note = `${who} played alone, so this day is worth no point yet. It counts as soon as somebody else joins in.`;
  } else if (!spoilers) {
    note = "The squares appear once you have finished your own game.";
  }

  /* Today's card leads with its button — that is the thing to do. A past day leads with
     the scores, because they are the thing to read, and its way into the standings sits
     underneath at its own size. */
  const past = item.action === "past";
  return `<div class="wdl">
    <div class="wdl-head">
      ${ic("i-tiles")}
      <span class="wdl-title">${esc(wordleTitle(item))}</span>
      <span class="wdl-date">${esc(fmtDay(item.at))}</span>
    </div>
    <div class="wdl-body">
      ${past ? "" : wordleButton(item)}
      ${scores}
      ${note ? `<p class="wdl-note">${note}</p>` : ""}
      ${past ? `<div class="wdl-foot">${wordleButton(item)}</div>` : ""}
    </div>
  </div>`;
}

const sameOp = (a, b) => a && b && a.author === b.author && a.channel === b.channel && a.seq === b.seq;

/** The quick-pick emoji for reactions. Small on purpose: eight covers what this chat
    actually uses, and the pill row itself takes any emoji already there. */
const QUICK_EMOJI = ["👍", "❤️", "😂", "😮", "😢", "🎉", "🔥", "👀"];

const reactionPill = (r, msgOp) => `<button class="pill${r.mine ? " pill--mine" : ""}" data-pill-op='${opAttr(msgOp)}' data-emoji="${esc(r.emoji)}"
    title="${esc(r.peers.join(", "))}">
  <span class="pill-emoji">${esc(r.emoji)}</span><span class="num">${r.count}</span>
</button>`;

function renderMessage(item, grouped, at) {
  const author = item.mine ? selfPeer() : (peerById(item.author) || { name: item.author_name, avatar: item.avatar, presence: "offline" });
  const time = at ? fmtTime(at) : "";
  /* A reply carries its context quote, so grouping it under another message would hide
     exactly the thing that makes it a reply. */
  const isReply = item.kind === "message" && item.reply_to;
  const quote = isReply ? (() => {
    const s = item.reply_snippet;
    return `<button class="msg-quote" data-jump-reply='${opAttr(item.reply_to)}'>
      ${s
        ? `<span class="msg-quote-name">${esc(s.author_name)}</span><span class="msg-quote-body">${esc(s.body)}</span>`
        : '<span class="msg-quote-body msg-quote-gone">Original message unavailable</span>'}
    </button>`;
  })() : "";
  const reactions = item.kind === "message" && item.reactions?.length
    ? `<div class="msg-reactions">${item.reactions.map(r => reactionPill(r, item.id)).join("")}
       <button class="pill pill--add" data-react-open='${opAttr(item.id)}' title="Add reaction">+</button></div>`
    : "";
  const quickBar = V.emojiFor && sameOp(V.emojiFor, item.id)
    ? `<div class="emoji-bar" role="menu">${QUICK_EMOJI.map(e =>
        `<button data-emoji="${e}" title="React with ${e}">${e}</button>`).join("")}</div>`
    : "";
  return `<article class="msg${grouped ? " msg--grouped" : " msg--start"}${item.mentions_you ? " msg--mentions" : ""}${isReply ? " msg--reply" : ""}">
    <div class="msg-gutter">
      ${grouped ? `<span class="stamp-hover">${time}</span>` : avatar(author, 40)}
    </div>
    <div class="msg-body">
      ${grouped ? "" : `<div class="msg-head">
        <span class="msg-author au-${author.avatar ?? item.avatar}">${esc(item.author_name)}</span>
        <span class="msg-stamp">${time}</span>
        ${item.edited ? '<span class="msg-edited">(edited)</span>' : ""}
      </div>`}
      ${quote}
      ${itemContent(item)}
      ${reactions}
      ${quickBar}
    </div>
    <div class="msg-actions">
      ${item.kind === "message" ? `<button data-reply-to='${opAttr(item.id)}' data-reply-name="${esc(item.author_name)}" data-reply-text="${esc(item.body.slice(0, 90))}" title="Reply">${ic("i-msg", "icon", 'style="width:15px;height:15px"')}<span class="sr">Reply to this message</span></button>` : ""}
      ${item.kind === "message" ? `<button data-react-open='${opAttr(item.id)}' title="Add reaction"><span class="act-smiley">🙂</span><span class="sr">Add reaction</span></button>` : ""}
      ${item.mine && item.kind === "message" ? `<button data-edit='${opAttr(item.id)}' title="Edit">${ic("i-edit", "icon", 'style="width:15px;height:15px"')}<span class="sr">Edit this message</span></button>` : ""}
      ${item.mine ? `<button data-danger data-delete='${opAttr(item.id)}' title="Delete">${ic("i-trash", "icon", 'style="width:15px;height:15px"')}<span class="sr">Delete this</span></button>` : ""}
      ${!item.mine && item.kind === "message" ? `<button data-copy='${esc(item.body)}' title="Copy text">${ic("i-file", "icon", 'style="width:15px;height:15px"')}<span class="sr">Copy this message</span></button>` : ""}
    </div>
  </article>`;
}

/** Grouped continuation: same author, same day, within seven minutes. */
const GROUP_WINDOW = 7 * 60 * 1000;

function renderTimeline() {
  const host = $("timeline");
  const dmPeer = V.view === "dms" && V.dm ? peerById(V.dm) : null;
  let html = "";

  if (dmPeer) {
    html += `<div class="dm-intro">
      ${avatar(dmPeer, 40)}
      <h2>${esc(dmPeer.name)}</h2>
      <p>This conversation stays between the two of you. It is never relayed through the
         third peer, because there is no encryption on the wire and relaying would mean
         letting them read it.</p>
    </div>`;
    if (!TL.length) {
      html += `<p class="dm-empty">No messages yet. ${dmPeer.presence === "online"
        ? `Anything you send reaches ${esc(dmPeer.name)} straight away.`
        : `Anything you send reaches ${esc(dmPeer.name)} as soon as he is back.`}</p>`;
      host.innerHTML = `<div class="tl-inner">${html}</div>`;
      return;
    }
  } else if (!TL.length) {
    host.innerHTML = `<div class="tl-inner"><div class="empty">
      <div class="empty-ico">${ic("i-hash", "icon-20 icon")}</div>
      <h2>#${esc(activeName())} is empty</h2>
      <p>Nothing has been posted here yet. Anything you send stays in this channel and
         reaches the others as soon as they are online.</p>
    </div></div>`;
    return;
  }

  let lastDay = null;
  let lastAuthor = null;
  let lastAt = 0;
  /* Where the "New" divider goes is decided when the conversation opens, not from the
     live counter: opening marks it read, so by the time this runs the count is already
     zero and the divider would never appear at all. */
  let firstUnreadDrawn = false;
  const unreadFrom = TL.length - (V.unreadAtOpen || 0);

  TL.forEach((item, i) => {
    /* A file carries no wall clock of its own — it is ordered by the same lamport key as
       the messages and shown at the time of whatever it was dropped beside. */
    const at = item.at || lastAt;
    const day = at ? fmtDay(at) : null;
    if (day && day !== lastDay) {
      html += `<div class="day">${esc(day)}</div>`;
      lastDay = day;
      lastAuthor = null;
    }
    if (!firstUnreadDrawn && unreadFrom > 0 && i === unreadFrom) {
      html += `<div class="newline">New</div>`;
      firstUnreadDrawn = true;
      lastAuthor = null;
    }
    if (item.kind === "wordle") {
      /* Not a message: no author, no gutter, no grouping. The next message after it starts
         a fresh block, which is what clearing `lastAuthor` does. */
      html += wordleCard(item);
      lastAuthor = null;
      lastAt = at || lastAt;
      return;
    }
    const grouped = item.author === lastAuthor
      && at && lastAt && at - lastAt < GROUP_WINDOW
      && !item.mentions_you
      && !(item.kind === "message" && item.reply_to);
    html += renderMessage(item, grouped, at);
    lastAuthor = item.author;
    lastAt = at || lastAt;
  });

  host.innerHTML = `<div class="tl-inner">${html}</div>`;
  hydrateYt();
}

function unreadCount() {
  const key = activeChannel();
  if (key.startsWith("dm:")) return peerById(key.slice(3))?.unread || 0;
  return channelByKey(key)?.unread || 0;
}

/* ---------------------------------------------------------------- members */

function memberRow(p, opts = {}) {
  const me = !!p.self;
  const quiet = !me && p.presence !== "online";
  const speaking = me ? meSpeaking() : isSpeaking(p.id);
  const shared = (S.streams || []).filter(s => s.owner === p.id);
  const muted = (p.volume || 0) === 0;
  const vol = Math.round((p.volume ?? 1) * 100);
  const deskVol = Math.round((p.desktop_volume ?? 1) * 100);

  return `<div class="mem" data-id="${esc(p.id)}" data-quiet="${quiet}" data-speaking="${speaking}">
    <div>${avatar(p, 32, true, true)}</div>
    <div class="mem-main">
      <div class="mem-top">
        <span class="mem-name">${esc(p.name)}</span>
        ${me ? '<span class="mem-you">YOU</span>' : ""}
      </div>
      ${p.user_status ? `<div class="mem-sub mem-status">${esc(statusLabel(p.user_status))}</div>` : ""}
      ${opts.sub ? `<div class="mem-sub">${esc(opts.sub)}</div>` : ""}
      ${opts.tools ? `<div class="mem-tools">
        <button class="mem-mute" aria-pressed="${muted}" data-pmute="${esc(p.id)}" title="${muted ? `Unmute ${esc(p.name)}` : `Mute ${esc(p.name)}`}">
          ${ic(muted ? "i-mic-off" : "i-mic", "icon", 'style="width:14px;height:14px"')}
          <span class="sr">${muted ? "Unmute" : "Mute"} ${esc(p.name)}</span>
        </button>
        <input type="range" min="0" max="100" value="${vol}" data-vol="${esc(p.id)}"
               style="--pct:${vol}%" aria-label="Voice volume for ${esc(p.name)}">
        <span class="vol-num num">${vol}</span>
      </div>` : ""}
      ${p.desktop_stream !== null && p.desktop_stream !== undefined ? `<div class="mem-desk">
        <span class="tool-label">Screen audio</span>
        <div class="mem-tools">
          ${ic("i-monitor", "icon", 'style="width:14px;height:14px;color:var(--text-dim);flex:none"')}
          <input type="range" min="0" max="100" value="${deskVol}" data-dvol="${esc(p.id)}" data-stream="${p.desktop_stream}"
                 style="--pct:${deskVol}%" aria-label="Desktop audio volume for ${esc(p.name)}">
          <span class="vol-num num">${deskVol}</span>
        </div>
      </div>` : ""}
      ${shared.filter(s => !s.watching).map(s =>
        `<button class="share-chip" data-watch="${esc(p.id)}" data-stream="${s.stream_id}">${ic(s.is_camera ? "i-cam" : "i-monitor")}<span>Watch</span> <span class="chip-src">${esc(s.title)}</span></button>`).join("")}
      ${shared.filter(s => s.watching).map(s =>
        `<button class="share-chip" data-unwatch="${esc(p.id)}" data-stream="${s.stream_id}">${ic("i-x")}<span>Stop watching</span> <span class="chip-src">${esc(s.title)}</span></button>`).join("")}
    </div>
  </div>`;
}

function renderMembers() {
  const el = $("members");
  const me = selfPeer();
  const inCall = callPeers();
  const inCallIds = new Set(inCall.map(p => p.id));
  const roster = S.voice.joined ? [me, ...inCall] : inCall;
  const online = knownPeers().filter(p => p.presence === "online" && !inCallIds.has(p.id));
  const connecting = knownPeers().filter(p => p.presence === "connecting");
  const away = knownPeers().filter(p => p.presence === "offline" || p.presence === "broken");
  const unknown = (S.peers || []).filter(p => !p.id);
  if (!S.voice.joined) online.unshift(me);

  const group = (label, list, opts) => list.length ? `<div class="group">
      <div class="group-head">${label}<span class="group-count num">${list.length}</span></div>
      ${list.map(p => memberRow(p, opts(p))).join("")}
    </div>` : "";

  // The member column is a titled window like its two siblings; the strip is part of
  // this render so the aside never shows content without its title bar.
  el.innerHTML =
    `<div class="members-head"><h2>Members</h2><span class="num">${knownPeers().length + 1}</span></div>` +
    `<div class="members-scroll">` +
    group("In the call", roster, p => ({
      sub: p.self
        ? (S.voice.muted ? "Microphone muted" : meSpeaking() ? "Speaking" : "Listening")
        : (isSpeaking(p.id) ? "Speaking" : (p.volume || 0) === 0 ? "Muted for you" : "Listening"),
      tools: !p.self,
    })) +
    group("Online", online, p => ({
      sub: p.self && S.do_not_disturb ? "Do not disturb"
         : p.self && sharingSelf() ? "Sharing your screen"
         : p.self && ownCamera() ? "Camera on"
         : callRunning() ? "Not in the call" : "",
    })) +
    group("Connecting", connecting, () => ({ sub: "Reconnecting" })) +
    group("Offline", away, p => ({ sub: p.problem || lastSeenLine(p) })) +
    /* B-56: no esc() here — memberRow escapes `sub` in the sink now, so escaping at the
       caller would show &amp;-noise. One place decides, and it is the place that writes
       the HTML. */
    group("Not introduced yet", unknown, p => ({ sub: p.address })) +
    `</div>`;
}

/* ---------------------------------------------------------------- strip */

function renderStrip() {
  const el = $("strip-slot");
  const tiles = [
    ...watchedStreams().map(s => ({
      key: `${s.owner}-${s.stream_id}`,
      who: s.owner_name,
      what: s.title,
      live: true,
      icon: s.is_camera ? "i-cam" : "i-monitor",
    })),
    ...(S.own_streams || []).filter(s => !s.is_audio).map(s => ({
      key: null,
      who: "You",
      what: s.title,
      live: s.viewers > 0,
      icon: s.is_camera ? "i-cam" : "i-monitor",
    })),
  ];
  if (!tiles.length) { el.innerHTML = ""; return; }

  /* A tile only becomes an image once a frame has actually arrived. Until then it shows
     the idle mark: a stream that was just opened has nothing to draw for half a second,
     and an <img> with no source is a broken-image box. */
  el.innerHTML = `<div class="strip">${tiles.map(t => `
    <button class="tile">
      ${t.live ? '<span class="live-tag"><i></i>LIVE</span>' : ""}
      ${t.key && thumbUrls[t.key]
        ? `<img id="thumb-${esc(t.key)}" src="${esc(thumbUrls[t.key])}" alt="${esc(t.who)} &middot; ${esc(t.what)}">`
        : `<span class="tile-idle">${ic(t.icon, "icon-20 icon")}</span>`}
      <span class="tile-cap">${ic(t.icon)}<span>${esc(t.who)} &middot; ${esc(t.what)}</span></span>
    </button>`).join("")}</div>`;
}

const thumbUrls = {};

/* ---------------------------------------------------------------- overlays */

function renderOverlays() {
  const ac = $("ac-slot");
  const drop = $("drop-slot");
  const input = $("composer-input");
  const open = V.overlay === "ac" && V.acMatches.length > 0;
  const plus = V.overlay === "plus";

  // Both overlays sit in the same slot above the composer and neither can be open with the
  // other, so they share it — and the + menu borrows the .ac popover styling wholesale.
  if (plus) {
    ac.innerHTML = `<div class="ac">
      <div class="ac-head" id="plus-head">Put in #general</div>
      <div role="menu" id="plus-menu" aria-labelledby="plus-head">
        <button class="ac-item" role="menuitem" data-plus="wordle">${ic("i-tiles")}Today's Wordle
          <small>everyone sees it</small></button>
      </div>
    </div>`;
  } else {
    ac.innerHTML = open ? `<div class="ac">
    <div class="ac-head" id="ac-head">Members matching @${esc(V.acQuery || "")}</div>
    <div role="listbox" id="ac-list" aria-labelledby="ac-head">
      ${V.acMatches.map((p, i) => `<button class="ac-item" role="option" id="ac-opt-${i}"
        aria-selected="${i === V.acIndex}" data-ac="${esc(p.name)}">${avatar(p, 20)}${esc(p.name)}${i === V.acIndex ? "<small>Tab</small>" : ""}</button>`).join("")}
    </div>
  </div>` : "";
  }
  const plusBtn = $("plus");
  if (plusBtn) plusBtn.setAttribute("aria-expanded", String(plus));
  input.setAttribute("aria-expanded", String(open));
  if (open) input.setAttribute("aria-activedescendant", `ac-opt-${V.acIndex}`);
  else input.removeAttribute("aria-activedescendant");

  drop.innerHTML = V.overlay === "drop" ? `<div class="drop">
    <div class="drop-inner">
      ${ic("i-share", "icon")}
      <strong>Drop to share</strong>
      <span>The file is offered to everyone in ${V.view === "dms" ? esc(activeName()) : `#${esc(activeName())}`}</span>
    </div>
  </div>` : "";
}

function renderError() {
  $("error-slot").innerHTML = S.error ? `<div class="error-bar">
    ${ic("i-alert", "icon")}
    <span>${esc(S.error)}</span>
    <button id="error-dismiss" title="Dismiss">${ic("i-x")}<span class="sr">Dismiss</span></button>
  </div>` : "";
}

/* ---------------------------------------------------------------- status bar */

function renderStatus() {
  const el = $("status");
  const link = p => `<span class="link" data-id="${esc(p.id || p.address)}" data-state="${p.presence}">
      <i></i><b>${esc(p.name)}</b><span class="num" data-rtt>${
        p.presence === "online" ? fmtRtt(p.id)
        : p.presence === "connecting" ? "reconnecting"
        : p.presence === "broken" ? "needs attention"
        : "offline"}</span></span>`;

  const u = S.update || {};
  const update = u.state === "ready"
    ? `<button class="update-chip" id="btn-update">${ic("i-download")}Update to ${esc(u.version)} is ready</button>`
    : u.state === "downloading"
    ? `<span class="update-chip" aria-live="polite">${ic("i-download")}Fetching ${esc(u.version)} — ${Math.round(100 * u.received / Math.max(1, u.total))}%</span>`
    : u.state === "checking"
    ? `<span class="update-chip" aria-live="polite">${ic("i-retry")}Checking for updates…</span>`
    /* The error text matters here: a signed manifest pointing at a release asset that is
       not there looks exactly like "nothing happens" without it. */
    : u.state === "failed"
    ? `<button class="update-chip" id="btn-update-dismiss" title="${esc(u.error || "")}">${ic("i-alert")}Update failed</button>`
    : u.state === "uptodate"
    ? `<button class="update-chip" id="btn-update-dismiss">${ic("i-check")}Up to date</button>`
    : "";

  el.innerHTML = `
    ${(S.peers || []).map(link).join("")}
    <span class="status-spacer"></span>
    ${update}
    <span class="status-meta mono">v${esc(S.app_version)} &middot; protocol ${S.protocol_version}</span>`;
}

/* ---------------------------------------------------------------- header + shell */

function renderHead() {
  const icon = $("main-head-icon");
  const title = $("main-title");
  const topic = $("main-topic");
  const input = $("composer-input");

  if (V.view === "dms" && V.dm) {
    const p = peerById(V.dm);
    icon.innerHTML = `<use href="#i-at"/>`;
    title.textContent = p ? p.name : "";
    topic.textContent = "Direct message · never relayed through the third peer";
    input.placeholder = `Message ${p ? p.name : ""}`;
  } else {
    const c = channelByKey(V.channel) || S.channels[0];
    icon.innerHTML = `<use href="#i-hash"/>`;
    title.textContent = c ? c.name : "general";
    topic.textContent = c && c.key === "general"
      ? "Everyone sees this channel"
      : "Sub-channel · everyone sees this too";
    input.placeholder = `Message #${c ? c.name : ""}`;
  }
}

function renderShell() {
  const body = $("body");
  body.dataset.view = V.view;
  body.dataset.members = V.members ? "shown" : "hidden";

  $("settings").hidden = V.view !== "settings";
  $("nav-channels").setAttribute("aria-current", V.view === "channels");
  $("nav-dms").setAttribute("aria-current", V.view === "dms");
  $("nav-settings").setAttribute("aria-current", V.view === "settings");
  $("toggle-members").setAttribute("aria-pressed", V.members);

  const n = totalDmUnread();
  const badge = $("dm-badge");
  badge.hidden = n === 0;
  badge.textContent = n || "";
  $("dm-badge-sr").textContent = n ? `Direct messages, ${n} unread` : "Direct messages";
}

/* ---------------------------------------------------------------- settings */

const SET_TABS = [
  { id: "account", name: "Account", icon: "i-users" },
  { id: "audio", name: "Audio", icon: "i-mic" },
  { id: "video", name: "Video", icon: "i-monitor" },
  { id: "files", name: "Files", icon: "i-image" },
  { id: "network", name: "Network", icon: "i-signal" },
];

/** Fetched the first time the Audio tab is shown; Refresh drops it so a headset that was
    just plugged in appears. */
let devices = null;

/** What the Account tab says next to "Check for updates". The full error is spelled out
    rather than summarised: every way this feed can break is a sentence the user needs. */
function updateWord() {
  const u = S.update || {};
  if (u.state === "checking") return "Checking…";
  if (u.state === "uptodate") return "You are on the newest version.";
  if (u.state === "downloading") return `Fetching ${esc(u.version)}…`;
  if (u.state === "ready") return `Version ${esc(u.version)} is downloaded and verified.`;
  if (u.state === "failed") return esc(u.error || "The check failed.");
  return "";
}

const deviceSelect = (id, list, chosen, label) => `
  <span class="select-wrap">
    <select id="${id}" style="min-width:300px" aria-label="${label}">
      <option value=""${chosen ? "" : " selected"}>Windows default</option>
      ${(list || []).map(d => `<option${d === chosen ? " selected" : ""}>${esc(d)}</option>`).join("")}
      ${chosen && !(list || []).includes(chosen) ? `<option selected>${esc(chosen)}</option>` : ""}
    </select>
    ${ic("i-chev")}
  </span>`;

const SET_BODY = {
  account: () => `
    <h2>Account</h2>
    <p>Your identity is a UUID generated on first start and stored locally. The display
       name is cosmetic and can change at any time; the others see the new name without
       restarting.</p>
    <div class="field">
      <label class="field-label" for="set-name">Display name</label>
      <span class="field-help">How the other two see you.</span>
      <div class="control-row">
        <input class="text-input" id="set-name" value="${esc(S.self.name)}" style="min-width:240px">
        <button class="btn btn--accent" id="save-name">Save</button>
      </div>
    </div>
    <div class="field">
      <label class="field-label" for="set-status">Status</label>
      <span class="field-help">What the others see next to your name. Volatile: after a
        restart you are plain online again.</span>
      <div class="control-row">
        <select id="set-status" class="text-input" style="min-width:240px">
          <option value="online"${S.self.status === "online" ? " selected" : ""}>Online</option>
          <option value="away"${S.self.status === "away" ? " selected" : ""}>Away</option>
          <option value="busy"${S.self.status === "busy" ? " selected" : ""}>Busy</option>
        </select>
      </div>
    </div>
    <div class="field">
      <span class="field-label">Identity</span>
      <span class="field-help">Never copy this between machines. Two peers sharing one
        identity break chat synchronisation.</span>
      <div class="control-row">
        <code class="mono code-inline">${esc(S.self.id)}</code>
        <button class="btn btn--ghost" data-copy="${esc(S.self.id)}">Copy</button>
      </div>
    </div>
    <div class="field">
      <div class="switch-row">
        <div>
          <span class="field-label">Start with Windows</span>
          <span class="field-help">Set in <code class="mono">config.toml</code>
            and applied at every start.</span>
        </div>
        <button class="switch" aria-pressed="${S.autostart}" aria-label="Start with Windows" disabled></button>
      </div>
    </div>
    <div class="field">
      <div class="switch-row">
        <div>
          <span class="field-label">Close button hides to tray</span>
          <span class="field-help">On by default, so closing the window does not stop the
            app while you are gaming. Set in <code class="mono">config.toml</code>.</span>
        </div>
        <button class="switch" aria-pressed="${S.minimize_to_tray}" aria-label="Close button hides to tray" disabled></button>
      </div>
    </div>
    <div class="field">
      <span class="field-label">Version</span>
      <span class="field-help">Updates come from the signed release feed on GitHub, checked
        every six hours. Only a build signed with the release key is ever accepted.</span>
      <div class="control-row">
        <code class="mono code-inline">v${esc(S.app_version)}</code>
        <button class="btn btn--ghost" id="btn-check-update"
                ${S.update && (S.update.state === "checking" || S.update.state === "downloading") ? "disabled" : ""}>
          ${ic("i-retry")}Check for updates</button>
        <span class="field-help" style="margin:0" aria-live="polite">${updateWord()}</span>
      </div>
    </div>`,

  audio: () => `
    <h2>Audio</h2>
    <p>Voice runs at 48 kHz with 20 ms frames. Noise suppression is always on. There is no
       echo cancellation, because everyone wears a headset.</p>
    <div class="field">
      <label class="field-label" for="set-in">Microphone</label>
      <span class="field-help">Leave on the Windows default unless you need a specific
        device. Changing it restarts a running call briefly.</span>
      <div class="control-row">
        ${deviceSelect("set-in", devices && devices[0], S.input_device, "Microphone")}
        <button class="btn btn--ghost" id="refresh-devices">${ic("i-retry")}Refresh</button>
      </div>
    </div>
    <div class="field">
      <label class="field-label" for="set-out">Output</label>
      <div class="control-row">
        ${deviceSelect("set-out", devices && devices[1], S.output_device, "Output")}
      </div>
      <div class="note">
        ${ic("i-alert")}
        <span>Set both devices to <code>48000 Hz</code> in Windows. At any other rate the
          app has to resample, and that costs quality for nothing.</span>
      </div>
    </div>
    <div class="field">
      <span class="field-label">Input level</span>
      <span class="field-help">Voice is only sent while you are actually speaking. When
        you are quiet, nothing goes over the line.</span>
      <span class="bar" style="max-width:320px;height:6px" id="input-level"><i style="transform:scaleX(0)"></i></span>
    </div>

    <h2 style="margin-top:32px">Notification sounds</h2>
    <p>Short tones when someone joins or leaves the call, or starts and stops sharing. The
       app generates them itself, so they do not go through the call and the others never
       hear them. Do not disturb silences them.</p>
    <div class="field">
      <span class="field-label">Sound set</span>
      <span class="field-help">Picking one plays it, so you hear what you chose.</span>
      <div class="sound-sets" data-sound-sets>
        ${(S.sound_sets || []).map(s => `
          <button class="sound-set" data-set="${esc(s.id)}" aria-pressed="${S.sound.set === s.id}">
            <span class="sound-set-name">${esc(s.name)}</span>
            <span class="sound-set-desc">${esc(s.description)}</span>
          </button>`).join("")}
      </div>
    </div>
    <div class="field">
      <label class="field-label" for="set-sound-vol">Volume</label>
      <span class="field-help">These tones only. Voice, desktop audio and the Windows sound
        for a message that mentions you are all left alone — those follow the Windows volume
        mixer.</span>
      <div class="control-row">
        <input type="range" min="0" max="100" value="${volPct(S.sound.volume)}"
               style="--pct:${volPct(S.sound.volume)}%;max-width:280px"
               id="set-sound-vol" aria-label="Notification sound volume">
        <span class="mono readout" id="sound-vol-readout">${volPct(S.sound.volume)}%</span>
      </div>
    </div>
    <div class="field">
      <span class="field-label">Hear them</span>
      <span class="field-help">${S.sound.volume > 0
        ? "All six, at the volume set above. Ignores do not disturb."
        : "The volume is at zero, so there is nothing to hear. Turn it up first."}</span>
      <div class="control-row" style="flex-wrap:wrap">
        ${(S.sound_events || []).map(e =>
          `<button class="btn btn--ghost" data-preview="${esc(e.id)}"
                   ${S.sound.volume > 0 ? "" : 'disabled title="The volume is at zero"'}>${ic("i-head")}${esc(e.name)}</button>`).join("")}
      </div>
    </div>`,

  video: () => `
    <h2>Video</h2>
    <p>Applies to every screen, window and camera you share. A running share restarts
       immediately with the new settings.</p>
    <div class="field">
      <span class="field-label">Codec</span>
      <span class="field-help">H.264 always works. HEVC decoding needs a Store extension
        that is not installed by default, so only pick it if you know both sides have it.</span>
      <div class="seg" data-video="codec">
        <button data-v="h264" aria-pressed="${S.video.codec === "h264"}">H.264</button>
        <button data-v="hevc" aria-pressed="${S.video.codec === "hevc"}">HEVC</button>
      </div>
    </div>
    <div class="field">
      <span class="field-label">Frame rate</span>
      <span class="field-help">An upper bound, not a promise: only whole divisors of your
        refresh rate give even motion.</span>
      <div class="seg" data-video="fps">
        <button data-v="30" aria-pressed="${S.video.fps === 30}">30</button>
        <button data-v="60" aria-pressed="${S.video.fps === 60}">60</button>
      </div>
    </div>
    <div class="field">
      <label class="field-label" for="set-br">Bitrate</label>
      <span class="field-help">12 Mbit/s is the default. Higher caused stutter in the voice
        of whoever was sharing when a viewer had a weaker connection.</span>
      <div class="control-row">
        <input type="range" min="4" max="40" value="${mbit(S.video.bitrate)}"
               style="--pct:${(mbit(S.video.bitrate) - 4) / 36 * 100}%;max-width:280px"
               id="set-br" aria-label="Bitrate in Mbit/s">
        <span class="mono readout" id="br-readout">${mbit(S.video.bitrate)} Mbit/s</span>
      </div>
    </div>
    <div class="field">
      <div class="note">
        ${ic("i-alert")}
        <span>Desktop audio starts and stops with the share, always. Your own voice is
          excluded, so nobody hears themselves back.</span>
      </div>
    </div>`,

  files: () => `
    <h2>Files</h2>
    <p>Shared files land in a fixed folder. Images go to a separate content-addressed
       folder so a thumbnail resolves to the same path for the sender and the receiver.</p>
    <div class="field">
      <span class="field-label">Download folder</span>
      <span class="field-help">Already downloaded files stay where they are — only new
        downloads land in the new folder.</span>
      <div class="control-row">
        <code class="mono code-inline">${esc(S.download_dir)}</code>
        <button class="btn btn--ghost" id="btn-download-dir">Change…</button>
      </div>
    </div>
    <div class="field">
      <span class="field-label">Images</span>
      <span class="field-help">Kept under <code class="mono">${esc(S.pictures_dir)}</code>,
        named after the content hash so both sides resolve the same file.</span>
      <div class="control-row">
        <button class="btn btn--danger" id="btn-wipe">${ic("i-trash")}Delete all images</button>
        <span class="field-help" style="margin:0">Only clears them from this machine.</span>
      </div>
    </div>`,

  network: () => `
    <h2>Network</h2>
    <p>Peers are addressed over the tailnet. Identities are learned on first contact, so
       there is nothing to exchange by hand.</p>
    <div class="field">
      <span class="field-label">Peers</span>
      <span class="field-help">Each peer lists the other two, in
        <code class="mono">config.toml</code>.</span>
      <div class="peer-table">
        ${(S.peers || []).map(p => `
          <div class="peer-row">
            ${avatar(p, 24, true)}
            <span class="peer-id">
              <span class="peer-name">${esc(p.name)}</span>
              <span class="mono peer-addr">${esc(p.address)}${p.app_version ? ` &middot; v${esc(p.app_version)}` : ""}</span>
            </span>
            <span class="link" data-id="${esc(p.id || p.address)}" data-state="${p.presence}"><i></i><span class="num" data-rtt>${
              p.presence === "online" ? fmtRtt(p.id)
              : p.presence === "connecting" ? "reconnecting"
              : p.presence === "broken" ? esc(p.problem || "needs attention")
              : "offline"}</span></span>
          </div>`).join("")}
      </div>
    </div>
    <div class="field">
      <span class="field-label">Ports</span>
      <span class="field-help">Changed in <code class="mono">config.toml</code>;
        the app reads them at start.</span>
      <div class="control-row">
        <span>
          <span class="field-help" style="margin:0 0 4px">Control (QUIC)</span>
          <input class="text-input mono" value="${S.control_port}" style="min-width:120px" aria-label="Control port" readonly>
        </span>
        <span>
          <span class="field-help" style="margin:0 0 4px">Media (UDP)</span>
          <input class="text-input mono" value="${S.media_port}" style="min-width:120px" aria-label="Media port" readonly>
        </span>
      </div>
    </div>`,
};

function renderSettings() {
  $("settings").innerHTML = `
    <nav class="set-nav" aria-label="Settings sections">
      <div class="set-nav-title">Settings</div>
      ${SET_TABS.map(t => `<button class="set-tab" data-tab="${t.id}" ${V.settingsTab === t.id ? 'aria-current="true"' : ""}>
        ${ic(t.icon)}${t.name}
      </button>`).join("")}
    </nav>
    <div class="set-body"><div class="set-inner">${SET_BODY[V.settingsTab]()}</div></div>`;
}

/* ---------------------------------------------------------------- render */

/* A chat opens on the newest message, never at the top of its history. Two rules, and the
   second is the one that gets forgotten:
   1. Switching conversation jumps to the newest message.
   2. Anything that shrinks the timeline — the stream strip appearing, the composer
      growing to three lines — must not slide the newest message out of view. */
/** Staat de focus in een veld in de instellingen dat de gebruiker aan het bijstellen is? */
function bezigMetInvoer() {
  const a = document.activeElement;
  const paneel = $("settings");
  if (!a || !paneel || !paneel.contains(a)) return false;
  return a.tagName === "INPUT" || a.tagName === "SELECT" || a.tagName === "TEXTAREA";
}

let lastConversation = null;
const PIN_SLACK = 24;
const wasPinned = tl => tl.scrollHeight - tl.clientHeight - tl.scrollTop <= PIN_SLACK;
const repin = (tl, pinned) => { if (pinned) tl.scrollTop = tl.scrollHeight; };

function render() {
  if (!S) return;
  const tlPre = $("timeline");
  const pinned = tlPre ? wasPinned(tlPre) : true;

  renderShell();
  renderChannels();
  renderVoice();
  renderHead();
  renderStrip();
  renderTimeline();
  renderMembers();
  renderOverlays();
  renderError();
  renderStatus();
  renderReplyChip();
  renderTyping();
  /* Niet hertekenen zolang de gebruiker in een veld staat. Een `change` op een schuif laat
     de motor opslaan, dat geeft een state-event, en dat bouwde het hele paneel opnieuw op —
     met het element waar de focus op stond erbij. Gevolg: pijltjestoetsen op een schuif
     werkten precies één keer, en typen in "Display name" werd bij elke tik gewist.
     Alleen voor invoervelden: een knop houdt geen toestand vast, en de kaartjes van de
     geluidssets moeten juist wél meteen bijwerken als je er een kiest. */
  if (V.view === "settings" && !bezigMetInvoer()) renderSettings();
  /* The board is state like everything else: a guess lands in the engine and comes back in
     the next event, so the window it is drawn in has to be redrawn with the rest. */
  if (wordleDlg.open) renderWordle();

  const tl = $("timeline");
  const key = `${V.view}:${activeChannel()}`;
  if (key !== lastConversation) {
    lastConversation = key;
    tl.scrollTop = tl.scrollHeight;
  } else {
    repin(tl, pinned);
  }
}

/* ---------------------------------------------------------------- dialogs */

const dlg = $("confirm");
let confirmAction = null;

function askConfirm({ title, text, ok, danger, onYes }) {
  $("confirm-title").textContent = title;
  $("confirm-text").textContent = text;
  const yes = $("confirm-yes");
  yes.textContent = ok;
  yes.className = "btn " + (danger ? "btn--danger" : "btn--accent");
  confirmAction = onYes;
  dlg.showModal();
}

$("confirm-no").addEventListener("click", () => dlg.close());
$("confirm-yes").addEventListener("click", () => {
  dlg.close();
  if (confirmAction) confirmAction();
  confirmAction = null;
});

/* The picture at full size. Two sizes and no zoom control worth the name: fitted to the
   window, or one image pixel per screen pixel with the frame scrolling. A screenshot of a
   1440p screen is the common case here, and on a 1080p window "fit" is the useful default
   while 1:1 is what you switch to for reading small text. */
const lightbox = $("lightbox");
let lightboxOp = null;

function openLightbox({ src, name, op }) {
  const img = $("lightbox-img");
  img.src = src;
  img.alt = name;
  $("lightbox-name").textContent = name;
  lightbox.classList.remove("is-full");
  $("lightbox-size").textContent = "Actual size";
  lightboxOp = op;
  lightbox.showModal();
}

$("lightbox-close").addEventListener("click", () => lightbox.close());
$("lightbox-size").addEventListener("click", () => {
  const full = lightbox.classList.toggle("is-full");
  $("lightbox-size").textContent = full ? "Fit to window" : "Actual size";
});
$("lightbox-open").addEventListener("click", () => {
  if (lightboxOp) invoke("open_file", { op: lightboxOp });
});
/* Clicking the picture toggles the two sizes; clicking beside it closes. A modal
   `<dialog>` paints its own backdrop, so a click that landed on the dialog itself or on
   the empty part of the frame is a click that missed the picture. */
$("lightbox-img").addEventListener("click", () => $("lightbox-size").click());
lightbox.addEventListener("click", e => {
  if (e.target === lightbox || e.target.id === "lightbox-frame") lightbox.close();
});

const promptDlg = $("prompt");
let promptAction = null;

function askText({ title, text, value = "", ok = "Save", onYes }) {
  $("prompt-title").textContent = title;
  $("prompt-text").textContent = text;
  $("prompt-input").value = value;
  $("prompt-yes").textContent = ok;
  promptAction = onYes;
  promptDlg.showModal();
  $("prompt-input").focus();
  $("prompt-input").select();
}

$("prompt-no").addEventListener("click", () => promptDlg.close());
$("prompt-yes").addEventListener("click", submitPrompt);
$("prompt-input").addEventListener("keydown", e => {
  if (e.key === "Enter") { e.preventDefault(); submitPrompt(); }
});

function submitPrompt() {
  const value = $("prompt-input").value.trim();
  promptDlg.close();
  if (value && promptAction) promptAction(value);
  promptAction = null;
}

/* ------------------------------------------------------- the wordle window

   The board, an on-screen keyboard and the leaderboard. Everything drawn here comes out of
   `S.wordle`, which the engine refreshes after every guess — so this repaints on the state
   event like the rest of the window, and there is no second copy of the game on this side.

   The answer is not in `S.wordle` until the game is over. Typing a guess sends the word to
   the engine and five colours come back; this side never knows the word in advance. */

const wordleDlg = $("wordle");
const KEYS = ["qwertyuiop", "asdfghjkl", "zxcvbnm"];

/** The letters typed into the row that is not sent yet. Cleared when a row lands. */
let wordleTyped = "";
let wordleRows = -1;

$("wordle-close").addEventListener("click", () => wordleDlg.close());

function openWordle() {
  renderWordle();
  if (!wordleDlg.open) wordleDlg.showModal();
}

/** The best mark each letter has earned so far, for colouring the keyboard. */
function wordleLetters(board) {
  const best = {};
  (board?.rows || []).forEach(r => {
    [...r.word.toLowerCase()].forEach((ch, i) => {
      const m = r.marks[i];
      if (best[ch] === undefined || m > best[ch]) best[ch] = m;
    });
  });
  return best;
}

function wordleBoardHtml(w) {
  const board = w.board;
  const rows = board?.rows || [];
  const live = board && !board.done ? wordleTyped : "";
  let html = "";
  for (let r = 0; r < w.tries; r++) {
    for (let c = 0; c < 5; c++) {
      if (r < rows.length) {
        html += `<span class="wdl-cell" data-m="${rows[r].marks[c]}">${esc(rows[r].word[c] || "")}</span>`;
      } else if (r === rows.length && board && !board.done) {
        html += `<span class="wdl-cell" data-live>${esc((live[c] || "").toUpperCase())}</span>`;
      } else {
        html += `<span class="wdl-cell"></span>`;
      }
    }
  }
  return `<div class="wdl-board">${html}</div>`;
}

function wordleStatus(w) {
  const board = w.board;
  if (!board) {
    // A manual attempt puts its reason here. Without this the + menu looks identical
    // whether it failed or is still going, which is the one thing the presser needs to know.
    return (w.error ? `<p class="wdl-line" data-error>${esc(w.error)}</p>` : "") +
      `<p class="wdl-line">Today's word could not be fetched yet. It is tried again every
      quarter of an hour; without it there is nothing to guess. <b>+</b> beside the message
      box tries now and puts the card in the chat for everyone.</p>`;
  }
  if (board.done && board.won) {
    return `<p class="wdl-line">Solved in ${board.rows.length} of ${w.tries}.</p>`;
  }
  if (board.done) {
    return `<p class="wdl-line">Out of guesses. The word was <b>${esc(board.solution || "")}</b>.</p>`;
  }
  if (w.error) return `<p class="wdl-line" data-error>${esc(w.error)}</p>`;
  return `<p class="wdl-line">Guess ${board.rows.length + 1} of ${w.tries}.</p>`;
}

function wordleKeys(w) {
  if (!w.board || w.board.done) return "";
  const letters = wordleLetters(w.board);
  const key = ch => {
    const m = letters[ch];
    return `<button class="wdl-key" data-wkey="${ch}"${m === undefined ? "" : ` data-m="${m}"`}>${ch}</button>`;
  };
  return `<div class="wdl-keys">
    ${KEYS.map((row, i) => `<div class="wdl-krow">
      ${i === 2 ? `<button class="wdl-key wdl-key--wide" data-wkey="enter">Enter</button>` : ""}
      ${[...row].map(key).join("")}
      ${i === 2 ? `<button class="wdl-key wdl-key--wide" data-wkey="back">Back</button>` : ""}
    </div>`).join("")}
  </div>`;
}

function wordleStandings(w) {
  if (!w.standings.length) {
    return `<div class="wdl-stand"><h3>Standings</h3>
      <p class="wdl-empty">Nobody has finished a puzzle yet. A day is worth one point to
        whoever solved it in the fewest guesses — and only when at least two of you
        played.</p></div>`;
  }
  return `<div class="wdl-stand"><h3>Standings</h3>
    <div class="wdl-row wdl-row-head"><span>Name</span><span class="wdl-pts">Pts</span>
      <span class="wdl-col">Played</span><span class="wdl-col">Solved</span></div>
    ${w.standings.map(st => `<div class="wdl-row" data-mine="${st.mine}">
      <span class="au-${st.avatar}">${esc(st.mine ? "You" : st.name)}</span>
      <span class="wdl-pts num">${st.points}</span>
      <span class="wdl-col num">${st.played}</span>
      <span class="wdl-col num">${st.solved}</span>
    </div>`).join("")}
  </div>`;
}

function renderWordle() {
  const w = S.wordle;
  /* A row that landed clears what was typed; a rejected guess keeps it, so you can fix a
     typo instead of retyping the word. Both cases come through here, and the row count is
     the only thing that tells them apart. */
  const landed = w.board ? w.board.rows.length : 0;
  if (landed !== wordleRows) {
    wordleRows = landed;
    wordleTyped = "";
  }
  const number = w.board ? ` ${w.board.number.toLocaleString()}` : "";
  $("wordle-title").textContent = `Wordle${number}`;
  $("wordle-panel").innerHTML = wordleBoardHtml(w) + wordleStatus(w) + wordleKeys(w) + wordleStandings(w);
}

function wordleKey(k) {
  const w = S.wordle;
  if (!w.board || w.board.done) return;
  if (k === "enter") {
    if (wordleTyped.length === 5) invoke("wordle_guess", { word: wordleTyped });
    return;
  }
  if (k === "back") wordleTyped = wordleTyped.slice(0, -1);
  else if (/^[a-z]$/.test(k) && wordleTyped.length < 5) wordleTyped += k;
  else return;
  renderWordle();
}

const picker = $("picker");
$("picker-cancel").addEventListener("click", () => picker.close());

async function openPicker() {
  const sources = await invoke("list_sources");
  /* Cameras are in the same list but never in this dialog: they have their own button,
     and a webcam under "Share a screen or window" reads as a mistake. */
  const screens = sources.filter(s => !s.is_camera);
  $("picker-list").innerHTML = screens.length
    ? screens.map(s => `<button class="picker-item" data-source="${s.index}">
        ${ic(s.is_window ? "i-file" : "i-monitor")}<span>${esc(s.name)}</span>
      </button>`).join("")
    : `<p class="picker-empty">Nothing was offered to capture.</p>`;
  picker.showModal();
}

/* ---------------------------------------------------------------- navigation */

function saveDraft() {
  const input = $("composer-input");
  const key = activeChannel();
  if (input.value.trim()) V.drafts[key] = input.value;
  else delete V.drafts[key];
}

function restoreDraft() {
  const input = $("composer-input");
  input.value = V.drafts[activeChannel()] || "";
  growComposer();
}

function openChannel(key) {
  saveDraft();
  V.view = "channels";
  V.channel = key;
  V.editing = null;
  V.replyTo = null;
  V.emojiFor = null;
  V.overlay = "none";
  afterConversationChange();
}

function openDm(id) {
  saveDraft();
  V.view = "dms";
  V.dm = id;
  V.editing = null;
  V.replyTo = null;
  V.emojiFor = null;
  V.overlay = "none";
  afterConversationChange();
}

async function afterConversationChange() {
  restoreDraft();
  V.unreadAtOpen = unreadCount();
  await invoke("mark_read", { channel: activeChannel() });
  await loadTimeline();
  render();
}

async function loadTimeline() {
  TL = await invoke("get_timeline", { channel: activeChannel() });
}

/* ---------------------------------------------------------------- events */

document.addEventListener("click", async e => {
  const t = e.target;

  /* An open + menu closes on any click that is not the button or one of its items, the way
     a menu is supposed to behave. First, before any of the handlers below can return early
     and leave it hanging open over the composer. The click itself still goes through. */
  if (V.overlay === "plus" && !t.closest("#plus") && !t.closest("[data-plus]")) {
    V.overlay = "none";
    renderOverlays();
  }

  /* A link goes to the system browser, never to this webview — it is the whole app, and
     there is no way back from a page it navigated to. */
  const link = t.closest("a[href]");
  if (link) {
    e.preventDefault();
    return invoke("open_link", { url: link.href });
  }

  if (t.closest("#win-min")) return getCurrentWindow().minimize();
  if (t.closest("#win-max")) return getCurrentWindow().toggleMaximize();
  if (t.closest("#win-close")) return invoke("close_window");

  const rail = t.closest("#nav-channels, #nav-dms, #nav-settings");
  if (rail) {
    if (rail.id === "nav-channels") { V.view = "channels"; return afterConversationChange(); }
    if (rail.id === "nav-dms") {
      V.view = "dms";
      if (!V.dm) V.dm = knownPeers()[0]?.id || null;
      return afterConversationChange();
    }
    V.view = "settings";
    return render();
  }

  const chan = t.closest("[data-channel]");
  if (chan) return openChannel(chan.dataset.channel);

  const dm = t.closest("[data-dm]");
  if (dm) return openDm(dm.dataset.dm);

  if (t.closest("#toggle-members")) { V.members = !V.members; return render(); }
  /* Collapsing keeps the channel you are in and anything unread — a collapse that hides
     a channel shouting at you is a collapse that loses the message. */
  if (t.closest("#collapse-general")) { V.collapsed = !V.collapsed; return renderChannels(); }
  if (t.closest("#new-channel")) {
    return askText({
      title: "New channel",
      text: "A sub-channel under the general channel. Everyone sees it, and it keeps its own message stream and its own unread count.",
      ok: "Create",
      onYes: title => invoke("create_channel", { title }),
    });
  }

  if (t.closest("#btn-join")) return invoke("set_joined", { joined: true });
  if (t.closest("#btn-leave")) return invoke("set_joined", { joined: false });
  if (t.closest("#btn-share")) {
    /* Screens only. Stopping "sharing" must not switch off the camera as a side effect —
       that has its own button. */
    const mine = (S.own_streams || []).filter(s => !s.is_audio && !s.is_camera);
    if (mine.length) return Promise.all(mine.map(s => invoke("stop_sharing", { stream: s.stream_id })));
    return openPicker();
  }
  if (t.closest("#btn-cam")) return invoke("set_camera", { on: !ownCamera() });
  if (t.closest("#btn-mic")) return invoke("set_muted", { muted: !S.voice.muted });
  if (t.closest("#btn-deaf")) return invoke("set_deafened", { deafened: !S.voice.deafened });
  if (t.closest("#btn-dnd")) return invoke("set_do_not_disturb", { on: !S.do_not_disturb });
  if (t.closest("#attach")) return invoke("pick_and_offer_file", { channel: activeChannel() });
  if (t.closest("#plus")) {
    V.overlay = V.overlay === "plus" ? "none" : "plus";
    return renderOverlays();
  }
  const plusItem = t.closest("[data-plus]");
  if (plusItem) {
    V.overlay = "none";
    renderOverlays();
    // No board opens: this is "put it in the chat", not "play it". The card lands in the
    // log for everyone, and you start it from there like any other day.
    if (plusItem.dataset.plus === "wordle") invoke("post_wordle_card");
    return;
  }
  if (t.closest("#error-dismiss")) return invoke("dismiss_error");

  const source = t.closest("[data-source]");
  if (source) {
    picker.close();
    return invoke("share_source", { index: Number(source.dataset.source) });
  }

  const watch = t.closest("[data-watch]");
  if (watch) return invoke("set_watching", { peer: watch.dataset.watch, stream: Number(watch.dataset.stream), watching: true });
  const unwatch = t.closest("[data-unwatch]");
  if (unwatch) return invoke("set_watching", { peer: unwatch.dataset.unwatch, stream: Number(unwatch.dataset.stream), watching: false });

  const pm = t.closest("[data-pmute]");
  if (pm) {
    const id = pm.dataset.pmute;
    const p = peerById(id);
    if (!p) return;
    const muted = (p.volume || 0) === 0;
    const volume = muted ? (V.volumeBeforeMute[id] ?? 1) : 0;
    if (!muted) V.volumeBeforeMute[id] = p.volume;
    return invoke("set_peer_volume", { peer: id, volume });
  }

  const dl = t.closest("[data-download]");
  if (dl) return invoke("download_file", { op: JSON.parse(dl.dataset.download) });

  const op = t.closest("[data-open]");
  if (op) return invoke("open_file", { op: JSON.parse(op.dataset.open) });

  const shot = t.closest("[data-shot]");
  if (shot) return openLightbox(JSON.parse(shot.dataset.shot));

  if (t.closest("[data-wordle]")) return openWordle();

  const wkey = t.closest("[data-wkey]");
  if (wkey) return wordleKey(wkey.dataset.wkey);

  const ed = t.closest("[data-edit]");
  if (ed) {
    V.editing = JSON.parse(ed.dataset.edit);
    render();
    const box = $("edit-input");
    if (box) { box.focus(); box.setSelectionRange(box.value.length, box.value.length); }
    return;
  }

  const del = t.closest("[data-delete]");
  if (del) {
    const op = JSON.parse(del.dataset.delete);
    return askConfirm({
      title: "Delete this?",
      text: "It disappears from the timeline for everyone. A file also stops being served — but a download that already finished stays on that machine.",
      ok: "Delete",
      danger: true,
      onYes: () => invoke("delete_message", { op }),
    });
  }

  const copy = t.closest("[data-copy]");
  if (copy) return navigator.clipboard.writeText(copy.dataset.copy);

  /* Replies, reactions and their two small popovers. The reply chip survives a re-render
     because it is its own slot; the emoji bar lives inside the message it belongs to. */
  const reply = t.closest("[data-reply-to]");
  if (reply) {
    V.replyTo = {
      op: JSON.parse(reply.dataset.replyTo),
      name: reply.dataset.replyName,
      body: reply.dataset.replyText,
    };
    renderReplyChip();
    return input.focus();
  }
  if (t.closest("#reply-cancel")) { V.replyTo = null; renderReplyChip(); return input.focus(); }

  const reactOpen = t.closest("[data-react-open]");
  if (reactOpen) {
    V.emojiFor = V.emojiFor && sameOp(V.emojiFor, JSON.parse(reactOpen.dataset.reactOpen))
      ? null
      : JSON.parse(reactOpen.dataset.reactOpen);
    return render();
  }
  const emoji = t.closest("[data-emoji]");
  if (emoji) {
    const op = V.emojiFor;
    V.emojiFor = null;
    invoke("react_message", { op, emoji: emoji.dataset.emoji });
    return render();
  }
  const pill = t.closest(".pill[data-pill-op]");
  if (pill) {
    // Toggling is decided on the engine side, which knows whether we already reacted.
    return invoke("react_message", { op: JSON.parse(pill.dataset.pillOp), emoji: pill.dataset.emoji });
  }
  if (!t.closest(".emoji-bar") && V.emojiFor) {
    V.emojiFor = null;
    render();
  }

  const acItem = t.closest("[data-ac]");
  if (acItem) return acceptSuggestion(acItem.dataset.ac);

  if (t.closest("#btn-update")) {
    return askConfirm({
      title: `Update to ${S.update.version} and restart?`,
      text: "The new version is already downloaded and verified. Applying it closes the app, replaces it, and starts it again. Your history and settings are untouched.",
      ok: "Update and restart",
      onYes: () => invoke("apply_update"),
    });
  }
  if (t.closest("#btn-update-dismiss")) return invoke("dismiss_update");
  if (t.closest("#btn-check-update")) return invoke("check_update");

  if (t.closest("#btn-wipe")) {
    return askConfirm({
      title: "Delete all images?",
      text: "This clears the image folder on this machine only. The others keep their copies, and anything still offered in the chat can be downloaded again.",
      ok: "Delete them",
      danger: true,
      onYes: () => invoke("delete_all_images"),
    });
  }

  if (t.closest("#save-name")) return invoke("set_display_name", { name: $("set-name").value });
  if (t.closest("#btn-download-dir")) return invoke("pick_download_dir");
  if (t.closest("#refresh-devices")) { devices = null; return loadDevices(); }

  const tab = t.closest("[data-tab]");
  if (tab) {
    V.settingsTab = tab.dataset.tab;
    renderSettings();
    if (V.settingsTab === "audio" && !devices) loadDevices();
    return;
  }

  const video = t.closest("[data-video] button");
  if (video) {
    const group = video.closest("[data-video]").dataset.video;
    const next = {
      codec: S.video.codec, fps: S.video.fps,
      bitrate: S.video.bitrate,
    };
    next[group] = group === "fps" ? Number(video.dataset.v) : video.dataset.v;
    return invoke("set_video_settings", next);
  }

  const soundSet = t.closest("[data-sound-sets] [data-set]");
  if (soundSet) {
    return invoke("set_sound_settings", {
      set: soundSet.dataset.set, volume: S.sound.volume,
    });
  }
  const audition = t.closest("[data-preview]");
  if (audition) return invoke("preview_sound", { sound: audition.dataset.preview });
});

/* Right-click a sub-channel for the two things you can do to it. Renaming and removing a
   channel are rare enough not to earn a control in the row, and common enough to need to
   exist. */
document.addEventListener("contextmenu", e => {
  const row = e.target.closest("[data-channel]");
  if (!row) return;
  const c = channelByKey(row.dataset.channel);
  if (!c || !c.removable) return;
  e.preventDefault();
  askText({
    title: `Rename ${c.name}`,
    text: "Everyone sees the new name. Leave it empty and cancel to remove the channel instead.",
    value: c.name,
    onYes: title => invoke("rename_channel", { channel: c.key, title }),
  });
});

document.addEventListener("input", e => {
  const desk = e.target.closest("[data-dvol]");
  if (desk) {
    desk.style.setProperty("--pct", desk.value + "%");
    desk.parentElement.querySelector(".vol-num").textContent = desk.value;
    invoke("set_stream_volume", {
      peer: desk.dataset.dvol,
      stream: Number(desk.dataset.stream),
      volume: desk.value / 100,
    });
    return;
  }
  const vol = e.target.closest("[data-vol]");
  if (vol) {
    vol.style.setProperty("--pct", vol.value + "%");
    vol.parentElement.querySelector(".vol-num").textContent = vol.value;
    invoke("set_peer_volume", { peer: vol.dataset.vol, volume: vol.value / 100 });
    return;
  }
  const br = e.target.closest("#set-br");
  if (br) {
    br.style.setProperty("--pct", ((br.value - 4) / 36 * 100) + "%");
    $("br-readout").textContent = `${br.value} Mbit/s`;
    return;
  }
  /* Only the readout while dragging; the tone itself plays on `change`, so dragging does
     not fire a beep per pixel. */
  const sv = e.target.closest("#set-sound-vol");
  if (sv) {
    sv.style.setProperty("--pct", sv.value + "%");
    $("sound-vol-readout").textContent = `${sv.value}%`;
    return;
  }
  const r = e.target.closest('input[type="range"]');
  if (r) r.style.setProperty("--pct", ((r.value - r.min) / (r.max - r.min) * 100) + "%");
});

document.addEventListener("change", e => {
  if (e.target.id === "set-br") {
    return invoke("set_video_settings", {
      codec: S.video.codec, fps: S.video.fps, bitrate: Number(e.target.value) * 1_000_000,
    });
  }
  if (e.target.id === "set-sound-vol") {
    return invoke("set_sound_settings", {
      set: S.sound.set, volume: Number(e.target.value) / 100,
    });
  }
  if (e.target.id === "set-status") {
    return invoke("set_user_status", { status: e.target.value });
  }
  if (e.target.id === "set-in" || e.target.id === "set-out") {
    return invoke("set_audio_devices", {
      input: $("set-in").value || null,
      output: $("set-out").value || null,
    });
  }
});

async function loadDevices() {
  devices = await invoke("list_audio_devices");
  if (V.view === "settings" && V.settingsTab === "audio") renderSettings();
}

/* ---------------------------------------------------------------- composer */

const input = $("composer-input");

/** The "replying to X" chip above the composer. Lives in its own slot so sending a
    state event does not have to redraw the composer. */
function renderReplyChip() {
  const el = $("reply-slot");
  if (!V.replyTo) { el.innerHTML = ""; return; }
  el.innerHTML = `<div class="reply-chip">
    <span class="reply-chip-label">Replying to</span>
    <span class="reply-chip-name">${esc(V.replyTo.name)}</span>
    <span class="reply-chip-body">${esc(V.replyTo.body)}</span>
    <button id="reply-cancel" title="Cancel reply">${ic("i-x", "icon", 'style="width:14px;height:14px"')}<span class="sr">Cancel reply</span></button>
  </div>`;
}

/** Who is typing in the open conversation. Reads the live state; called from `render`
    and from every state event, since typing arrives without a timeline change. */
function renderTyping() {
  const el = $("typing-slot");
  const ids = (S.typing || {})[activeChannel()] || [];
  const names = ids.map(id => peerById(id)?.name).filter(Boolean);
  if (!names.length) {
    if (el.innerHTML) el.innerHTML = "";
    return;
  }
  const who = names.length === 1 ? esc(names[0])
    : names.length === 2 ? `${esc(names[0])} and ${esc(names[1])}`
    : `${esc(names[0])} and ${names.length - 1} others`;
  el.innerHTML = `<div class="typing"><i></i><i></i><i></i><span>${who} ${names.length === 1 ? "is" : "are"} typing…</span></div>`;
}

function growComposer() {
  const tl = $("timeline");
  const pinned = tl ? wasPinned(tl) : true;
  input.style.height = "auto";
  input.style.height = Math.min(input.scrollHeight, 168) + "px";
  if (tl) repin(tl, pinned);
}

function acceptSuggestion(name) {
  const at = input.value.lastIndexOf("@");
  input.value = (at >= 0 ? input.value.slice(0, at) : input.value) + `@${name} `;
  V.overlay = "none";
  renderOverlays();
  input.focus();
  growComposer();
}

function updateAutocomplete() {
  const m = /@(\w*)$/.exec(input.value);
  if (!m) {
    if (V.overlay === "ac") { V.overlay = "none"; renderOverlays(); }
    return;
  }
  const q = m[1].toLowerCase();
  V.acQuery = m[1];
  V.acMatches = knownPeers().filter(p => p.name.toLowerCase().startsWith(q));
  V.acIndex = 0;
  V.overlay = "ac";
  renderOverlays();
}

input.addEventListener("input", () => {
  growComposer();
  updateAutocomplete();
  /* One "typing" per window while typing continuously; the engine throttles too, but
     this keeps the wire quiet for the common single-keystroke case as well. */
  if (input.value.trim() && Date.now() - V.lastTypingSent > 2500) {
    V.lastTypingSent = Date.now();
    invoke("notify_typing", { channel: activeChannel() });
  }
});

input.addEventListener("keydown", e => {
  if (V.overlay === "ac" && V.acMatches.length) {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const n = V.acMatches.length;
      V.acIndex = (V.acIndex + (e.key === "ArrowDown" ? 1 : n - 1)) % n;
      return renderOverlays();
    }
    if (e.key === "Tab" || e.key === "Enter") {
      e.preventDefault();
      return acceptSuggestion(V.acMatches[V.acIndex].name);
    }
  }
  if (e.key === "Enter" && !e.shiftKey) {
    e.preventDefault();
    const text = input.value.trim();
    if (!text) return;
    const replyTo = V.replyTo?.op || null;
    invoke("send_message", { channel: activeChannel(), text, replyTo });
    input.value = "";
    delete V.drafts[activeChannel()];
    V.lastTypingSent = 0;
    if (V.replyTo) { V.replyTo = null; renderReplyChip(); }
    growComposer();
    const tl = $("timeline");
    tl.scrollTop = tl.scrollHeight;
  }
});

/* A real paste event with the image in it. In egui this needed `GetAsyncKeyState`,
   because egui-winit swallowed the paste command before the app ever saw it. */
document.addEventListener("paste", async e => {
  const items = [...(e.clipboardData?.items || [])];
  const image = items.find(i => i.type.startsWith("image/"));
  if (!image) return;
  e.preventDefault();
  const file = image.getAsFile();
  if (!file) return;
  const bytes = [...new Uint8Array(await file.arrayBuffer())];
  const extension = (file.type.split("/")[1] || "png").replace("jpeg", "jpg");
  invoke("offer_pasted_image", { bytes, extension, channel: activeChannel() });
});

/* Typing goes to the board while its window is open. Physical keys and the drawn keyboard
   run through the same function, so there is one place where a letter means something. */
document.addEventListener("keydown", e => {
  if (!wordleDlg.open || e.ctrlKey || e.metaKey || e.altKey) return;
  if (e.key === "Enter") { e.preventDefault(); return wordleKey("enter"); }
  if (e.key === "Backspace") { e.preventDefault(); return wordleKey("back"); }
  if (/^[a-zA-Z]$/.test(e.key)) { e.preventDefault(); return wordleKey(e.key.toLowerCase()); }
});

document.addEventListener("keydown", e => {
  if (e.key !== "Escape") return;
  if (dlg.open || promptDlg.open || picker.open || lightbox.open || wordleDlg.open) return;   // <dialog> closes itself
  if (V.editing) { V.editing = null; return render(); }
  if (V.emojiFor) { V.emojiFor = null; return render(); }
  if (V.replyTo) { V.replyTo = null; return renderReplyChip(); }
  if (V.overlay !== "none") { V.overlay = "none"; return renderOverlays(); }
});

document.addEventListener("keydown", e => {
  if (e.target.id !== "edit-input") return;
  if (e.key === "Escape") { V.editing = null; return render(); }
  if (e.key === "Enter" && !e.shiftKey) {
    e.preventDefault();
    const op = V.editing;
    const text = e.target.value;
    V.editing = null;
    invoke("edit_message", { op, text });
  }
});

/* ---------------------------------------------------------------- engine */

function applyMeters() {
  /* Two attributes and two text nodes, never a re-render: this runs four times a second
     while a call is up, and product invariant 4 is "gaming wins". */
  document.querySelectorAll(".voice-peer[data-id], .mem[data-id]").forEach(el => {
    const id = el.dataset.id;
    el.dataset.speaking = String(id === S.self.id ? meSpeaking() : isSpeaking(id));
    const sub = el.querySelector(".mem-sub");
    if (sub && (sub.textContent === "Speaking" || sub.textContent === "Listening")) {
      sub.textContent = el.dataset.speaking === "true" ? "Speaking" : "Listening";
    }
  });
  const self = document.querySelector(".self");
  if (self) self.dataset.speaking = String(meSpeaking());

  document.querySelectorAll("[data-rtt]").forEach(el => {
    const id = el.closest("[data-id]")?.dataset.id;
    const peer = peerById(id);
    if (peer && peer.presence === "online") el.textContent = fmtRtt(id);
  });

  const worst = $("worst-rtt");
  if (worst) {
    const rtts = callPeers().map(p => M.peers[p.id]?.rtt).filter(r => r !== null && r !== undefined);
    worst.textContent = rtts.length ? `${Math.max(...rtts)} ms` : "—";
  }

  const level = $("input-level");
  if (level) level.firstElementChild.style.transform = `scaleX(${Math.min(1, M.self.level * 3).toFixed(3)})`;
}

async function applyState(next) {
  const previous = S;
  S = next;

  // A conversation that disappeared (a sub-channel someone removed) must not leave the
  // window pointing at nothing.
  if (V.view === "channels" && !channelByKey(V.channel)) V.channel = "general";
  if (V.view === "dms" && V.dm && !peerById(V.dm)) V.dm = knownPeers()[0]?.id || null;

  if (!previous || previous.timeline_revision !== S.timeline_revision) {
    await loadTimeline();
  }
  render();

  // Looking at a conversation is reading it — but only the one actually on screen, so a
  // DM cannot quietly clear the general channel's counter or the other way round.
  if (V.focused && unreadCount() > 0) invoke("mark_read", { channel: activeChannel() });
}

listen("state", e => applyState(JSON.parse(e.payload)));
listen("meters", e => { M = JSON.parse(e.payload); applyMeters(); });
listen("thumbnail", e => {
  const { key, revision } = e.payload;
  const first = !thumbUrls[key];
  thumbUrls[key] = `thumb://localhost/${key}?${revision}`;
  const img = $(`thumb-${key}`);
  if (img) img.src = thumbUrls[key];
  // The first frame turns the idle mark into an image, which needs the tile rebuilt.
  else if (first) renderStrip();
});
listen("focus", e => {
  V.focused = e.payload;
  if (V.focused && S && unreadCount() > 0) invoke("mark_read", { channel: activeChannel() });
});
listen("drag", e => {
  const next = e.payload ? "drop" : "none";
  if (V.overlay !== next && (V.overlay === "drop" || next === "drop")) {
    V.overlay = next;
    renderOverlays();
  }
});
/* The payload is a list of indices, not paths — the paths stay in Rust on purpose, so a
   script in this webview cannot name a file of its own. See `Ui::dropped`. */
listen("dropped", e => invoke("offer_files", { indices: e.payload, channel: activeChannel() }));

/* ---------------------------------------------------------------- narrow window */

/* Below 1080 the member list is in the way, so it gets closed — but as state, so the
   toggle keeps telling the truth and the user can open it again. A CSS override here made
   the button a no-op that still reported itself pressed. */
const narrow = matchMedia("(max-width: 1080px)");
let wasNarrow = null;

function applyWidth() {
  if (narrow.matches === wasNarrow) return;
  wasNarrow = narrow.matches;
  V.members = !narrow.matches;
  render();
}
narrow.addEventListener("change", applyWidth);

/* ---------------------------------------------------------------- boot */

(async () => {
  wasNarrow = narrow.matches;
  V.members = !narrow.matches;
  S = await invoke("ready");
  await loadTimeline();
  render();
  await invoke("mark_read", { channel: activeChannel() });
})();
