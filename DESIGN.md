---
name: FitCommunication
description: A server-free desktop comms client for three friends — the category standard, executed straight, in one fixed dark theme.
colors:
  bg-window: "#0E1013"
  bg-sidebar: "#14171B"
  bg-canvas: "#1A1E23"
  bg-raised: "#22272E"
  bg-hover: "#2A3038"
  bg-active: "#313944"
  bg-msg-hover: "#1E232A"
  bg-mention-hover: "#302915"
  bg-code: "#12151A"
  bg-well: "#0A0C0E"
  border: "#2E353E"
  border-strong: "#3A424D"
  border-input: "#4A5460"
  text: "#E6E9ED"
  text-mid: "#B3BAC4"
  text-muted: "#98A1AC"
  text-dim: "#8E97A2"
  accent: "#2FB3AE"
  accent-hover: "#3ECCC6"
  accent-text: "#5FD9D3"
  on-accent: "#05201F"
  online: "#3FB950"
  connecting: "#D29922"
  dnd: "#C77DAF"
  offline: "#6B747F"
  mention-bg: "#2A2418"
  mention-border: "#6B551E"
  danger: "#E5534B"
  danger-bg: "#2A1A19"
  danger-text: "#F0908A"
  danger-hi: "#FFB3AD"
  danger-line: "#4A2724"
  danger-hov: "#3A211F"
  close-hover: "#C4342B"
  on-signal: "#FFFFFF"
  av-1: "#4C6FB5"
  av-2: "#57896B"
  av-3: "#9A5F6E"
  code-key: "#8FC7E8"
  code-str: "#A5D6A7"
  code-num: "#E8C07D"
typography:
  display:
    fontFamily: "Archivo, 'Segoe UI Variable Text', 'Segoe UI', system-ui, sans-serif"
    fontSize: "20px"
    fontWeight: 700
    lineHeight: 1.45
    letterSpacing: "-0.01em"
  headline-intro:
    fontFamily: "Archivo, 'Segoe UI Variable Text', 'Segoe UI', system-ui, sans-serif"
    fontSize: "19px"
    fontWeight: 700
    lineHeight: 1.45
    letterSpacing: "-0.01em"
  headline:
    fontFamily: "Archivo, 'Segoe UI Variable Text', 'Segoe UI', system-ui, sans-serif"
    fontSize: "16px"
    fontWeight: 600
    lineHeight: 1.45
    letterSpacing: "normal"
  title:
    fontFamily: "Archivo, 'Segoe UI Variable Text', 'Segoe UI', system-ui, sans-serif"
    fontSize: "15px"
    fontWeight: 600
    lineHeight: 1.45
    letterSpacing: "normal"
  body:
    fontFamily: "Archivo, 'Segoe UI Variable Text', 'Segoe UI', system-ui, sans-serif"
    fontSize: "14px"
    fontWeight: 400
    lineHeight: 1.5
    letterSpacing: "normal"
  body-sm:
    fontFamily: "Archivo, 'Segoe UI Variable Text', 'Segoe UI', system-ui, sans-serif"
    fontSize: "13px"
    fontWeight: 400
    lineHeight: 1.55
    letterSpacing: "normal"
  label:
    fontFamily: "Archivo, 'Segoe UI Variable Text', 'Segoe UI', system-ui, sans-serif"
    fontSize: "11px"
    fontWeight: 600
    lineHeight: 1.45
    letterSpacing: "0.07em"
  micro:
    fontFamily: "Archivo, 'Segoe UI Variable Text', 'Segoe UI', system-ui, sans-serif"
    fontSize: "10px"
    fontWeight: 700
    lineHeight: 1.6
    letterSpacing: "0.05em"
  monogram:
    fontFamily: "Archivo, 'Segoe UI Variable Text', 'Segoe UI', system-ui, sans-serif"
    fontSize: "9px"
    fontWeight: 600
    lineHeight: 1
    letterSpacing: "0.01em"
  code:
    fontFamily: "'JetBrains Mono', ui-monospace, 'Cascadia Mono', Consolas, monospace"
    fontSize: "12.5px"
    fontWeight: 400
    lineHeight: 1.6
    fontFeature: "'tnum' 1"
rounded:
  inner: "3px"
  sm: "4px"
  md: "6px"
  lg: "10px"
  full: "999px"
spacing:
  "1": "2px"
  "2": "4px"
  "3": "6px"
  "4": "8px"
  "5": "10px"
  "6": "12px"
  "7": "14px"
  "8": "16px"
  "9": "18px"
  "10": "22px"
  "12": "28px"
  "14": "32px"
components:
  join-primary:
    backgroundColor: "{colors.accent}"
    textColor: "{colors.on-accent}"
    typography: "{typography.body-sm}"
    rounded: "{rounded.md}"
    height: "34px"
  join-primary-hover:
    backgroundColor: "{colors.accent-hover}"
  button-neutral:
    backgroundColor: "{colors.bg-hover}"
    textColor: "{colors.text}"
    rounded: "{rounded.sm}"
    padding: "0 11px"
    height: "28px"
  button-neutral-hover:
    backgroundColor: "{colors.bg-active}"
  button-accent:
    backgroundColor: "{colors.accent}"
    textColor: "{colors.on-accent}"
    rounded: "{rounded.sm}"
    padding: "0 11px"
    height: "28px"
  button-ghost:
    textColor: "{colors.text}"
    rounded: "{rounded.sm}"
    padding: "0 11px"
    height: "28px"
  button-danger:
    backgroundColor: "{colors.danger-bg}"
    textColor: "{colors.danger-text}"
    rounded: "{rounded.sm}"
    padding: "0 11px"
    height: "28px"
  button-danger-hover:
    backgroundColor: "{colors.danger-hov}"
    textColor: "{colors.danger-hi}"
  icon-button:
    textColor: "{colors.text-muted}"
    rounded: "{rounded.sm}"
    size: "28px"
  icon-button-pressed:
    backgroundColor: "{colors.bg-active}"
    textColor: "{colors.text}"
  rail-button:
    textColor: "{colors.text-muted}"
    rounded: "{rounded.md}"
    size: "40px"
  rail-button-current:
    backgroundColor: "{colors.bg-active}"
    textColor: "{colors.text}"
  list-row:
    textColor: "{colors.text-muted}"
    rounded: "{rounded.sm}"
    padding: "0 8px"
    height: "32px"
  list-row-current:
    backgroundColor: "{colors.bg-active}"
    textColor: "{colors.text}"
  composer:
    backgroundColor: "{colors.bg-raised}"
    textColor: "{colors.text}"
    typography: "{typography.body}"
    rounded: "{rounded.lg}"
    padding: "6px 6px 6px 8px"
  text-field:
    backgroundColor: "{colors.bg-raised}"
    textColor: "{colors.text}"
    rounded: "{rounded.sm}"
    padding: "0 10px"
    height: "32px"
  segmented-button:
    textColor: "{colors.text-muted}"
    rounded: "{rounded.inner}"
    padding: "0 14px"
    height: "30px"
  segmented-button-pressed:
    backgroundColor: "{colors.bg-active}"
    textColor: "{colors.text}"
  badge-count:
    backgroundColor: "{colors.dnd}"
    textColor: "{colors.on-signal}"
    rounded: "{rounded.full}"
    padding: "0 5px"
    height: "18px"
  chip-accent:
    backgroundColor: "rgba(47,179,174,.15)"
    textColor: "{colors.accent-text}"
    rounded: "{rounded.sm}"
    padding: "0 8px"
    height: "19px"
  card-attachment:
    backgroundColor: "{colors.bg-raised}"
    textColor: "{colors.text}"
    rounded: "{rounded.md}"
    padding: "9px 11px"
  dialog-confirm:
    backgroundColor: "{colors.bg-raised}"
    textColor: "{colors.text}"
    rounded: "{rounded.lg}"
    width: "min(420px, calc(100vw - 48px))"
---

# Design System: FitCommunication

Recorded from the shipped comp `design/main-window.html` (self-contained HTML/CSS/JS, 18 rendered
states in `design/shots/`). The comp is the reproduction target for the Tauri v2 frontend that
replaces `crates/app/src/ui/`. Every value below is present in that file; where the direction
contract and the build disagree, the build is recorded and the divergence is named.

## Overview

**Creative North Star: "The Well-Made Standard"**

The arrangement is the category standard — icon rail, channel/DM column, timeline, member list,
own titlebar, status bar — and that conventionality is the commitment, chosen over four fully
worked alternative worlds. The quality bar is Discord and Slack: the difference this system is
allowed to make is not in the layout but in the execution. Hairlines land on whole pixels,
presence has one vocabulary, the timeline keeps its rhythm at every density, and nothing
animates that would cost a frame on a machine that is also running a game.

The world is neutral-cool near-black in six ground layers, separated by one 1px hairline value
and one step of tonal lift rather than by shadow. There is exactly one accent, a tuned teal, and
it is spent on the primary action, the live progress fill, links and the focus ring — nowhere
decorative. Presence and alert are deliberately different families: green/amber/rose/unlit for
who is around, one loud red reserved for failure. Density is high and calm: 32px list rows,
28px controls, 11px uppercase group labels, and a 9–20px type ramp on a half-pixel grid.

Dark is not a style choice. The app is used almost every evening in a dark room, so the theme is
one fixed dark combination — no light mode, no theme switch, no OS-theme following — and
`color-scheme: dark` is declared so UA chrome (scrollbars, `select` popups, caret, selection)
comes back dark too rather than punching a bright hole in the window.

**Key Characteristics:**
- Six near-black ground layers, one hairline value, tonal lift instead of depth
- One teal accent; a second, separate rose for do-not-disturb and a third, loud red for failure
- Archivo for UI, JetBrains Mono for code and every measured figure
- 29 authored 24×24 icons at 1.5px stroke, round caps and joins, `currentColor` only
- Transform-only motion, 90–260ms, zero infinite animations
- Four fixed columns: 56px rail, 240px channels, fluid timeline, 232px members

## Colors

Neutral-cool near-blacks with a single tuned teal, warm enough in the greys not to read as
blue-grey plastic and dim enough overall that no surface burns in a dark room.

### Primary
- **Tuned Teal** (`{colors.accent}`): The only accent. Primary action (Join the call), transfer
  progress fill, the toggle switch when on, the drop-zone dash, and the focus ring. Its
  brightened sibling `accent-hover` is the hover of accent-filled controls only.
- **Teal On Text** (`{colors.accent-text}`): The accent as *text* — links in messages, accent
  chips (update available, Watch screen), the share hint icon. Lifted from the fill value
  because the fill is too dark to read as type on dark ground.
- **Teal Ink** (`{colors.on-accent}`): The near-black that sits *on* accent fills — button
  label, switch knob. Never used as a background.

### Secondary
- **Presence Green** (`{colors.online}`): Peer is online, voice is connected, the speaking ring.
- **Retry Amber** (`{colors.connecting}`): Connecting/reconnecting, and the informational note
  icon in settings. Reconnecting is a normal state, so amber is as loud as it gets.
- **Muted Rose** (`{colors.dnd}`): Do-not-disturb, unread count badges, the "new messages"
  divider, the LIVE tag dot, and a muted peer's mic glyph. A state colour, not an alarm.
- **Unlit Grey** (`{colors.offline}`): Only ever as a 1.5–2px inset ring on the panel ground.

### Tertiary
- **Mention Field** (`{colors.mention-bg}` / `{colors.mention-border}` / `{colors.bg-mention-hover}`):
  The amber-black field and hairline behind a message that mentions you.
- **Code Hues** (`{colors.code-key}` / `{colors.code-str}` / `{colors.code-num}`): Blue keys,
  green strings, amber numbers inside code blocks. Comments reuse `text-dim`; the code block has
  no fourth hue.
- **Danger Family** (`{colors.danger}`, `-bg`, `-text`, `-hi`, `-line`, `-hov`, `close-hover`):
  A full six-value family for failure: fill, hairline, text, hover-text, hover-fill, plus the
  distinct red that only the titlebar close button uses.

### Signal Ink and Identity
- **Signal Ink** (`{colors.on-signal}`): Pure white, and only as text or a glyph sitting on a
  *filled signal colour* — count badges on rose, the LIVE tag on its scrim, the titlebar close
  button on its red. Never on a neutral ground; the primary text tier owns that.
- **Peer Identity** (`{colors.av-1}` / `{colors.av-2}` / `{colors.av-3}`): The three avatar
  fills, keyed to a peer for the life of the install. These are identity, not state: never reuse
  them for presence, selection, or anything a user could mistake for a status.

### Neutral
- **Window Black** (`{colors.bg-window}`): Titlebar, rail, voice panel, status bar — the frame.
- **Panel Black** (`{colors.bg-sidebar}`): Channel column, member list, stream strip, settings nav.
- **Canvas** (`{colors.bg-canvas}`): The timeline and settings body — the largest field.
- **Raised** (`{colors.bg-raised}`): Anything that reads as an object on the canvas: composer,
  attachment cards, notes, selects, dialog, autocomplete, empty-state icon disc.
- **Hover / Active** (`{colors.bg-hover}` / `{colors.bg-active}`): The two interaction steps —
  hover, then current/pressed. `bg-active` is also the scrollbar thumb.
- **Row Hover** (`{colors.bg-msg-hover}`): The timeline's own hover, one step quieter than
  `bg-hover`, because a full-width row at panel-hover strength reads as a selection.
- **Well** (`{colors.bg-well}`) and **Code Black** (`{colors.bg-code}`): Below the canvas —
  video tiles, screenshots, code blocks. The only layers darker than the window frame.
- **Hairline** (`{colors.border}`): Every 1px structural line: column edges, headers, day
  dividers, card outlines. One value, everywhere.
- **Hairline Strong** (`{colors.border-strong}`) / **Input Line** (`{colors.border-input}`):
  Interactive edges — control borders, focused composer, floating-surface outlines; and the
  hovered form-control edge.
- **Text tiers** (`{colors.text}` → `{colors.text-mid}` → `{colors.text-muted}` → `{colors.text-dim}`):
  Primary/emphasis, running body text, secondary labels and idle icons, quiet metadata.

### Named Rules

**The One Teal Rule.** One accent, and it means "this is the action, or this is live". If a
surface needs a second colour to be understood, it needs a presence value or a danger value, not
a second accent.

**The DND-Is-Not-Danger Rule.** `dnd` and `danger` must never be the same value. They were, and
that made the do-not-disturb dot literally the error colour. Rose is a state; red is a failure.
Audit test: search the sheet for the danger red outside the danger family, the close button and
`att-sub[data-error]` — any other hit is a bug.

**The Unlit-Dot Rule.** Offline is drawn as an inset ring on the ground it sits on, never as a
warning colour and never as a red dot. One or two peers away is a normal state, so nothing about
it may read as an error — this holds in the roster, the status bar, and any future surface.

**The Quiet-Tier Floor Rule.** The quietest text tier must clear 4.5:1 against `bg-raised`, the
lightest ground it ever lands on. `#7C8590` measured 4.02 there — legible enough to look fine
and not legible enough to be right — and was raised to the recorded value. A quiet tier is
lightened, never dimmed further, to fix a legibility finding.

## Typography

**Display Font:** Archivo (400/500/600/700), with `Segoe UI Variable Text` → `Segoe UI` →
`system-ui` fallback
**Body Font:** Archivo, same stack — one UI family, no separate display face
**Label/Mono Font:** JetBrains Mono (400/500), with `ui-monospace` → `Cascadia Mono` → Consolas

**Character:** A grotesque workhorse doing all the UI work, and a mono that appears only where a
number or an identifier must be read exactly. Nothing is set in a display face; the largest type
in the product is a 20px settings heading.

### Hierarchy
- **Display** (700, 20px, -0.01em): Settings page heading. The single largest type in the app.
- **Headline intro** (700, 19px, -0.01em): The DM introduction heading, and only that.
- **Headline** (600/700, 16px): Empty-state and dialog headings.
- **Title** (600, 15px / 600, 14px): Channel header title; channel-column heading, message author.
- **Body** (400, 14px, 1.5, max 72ch): Message text and the composer. The only place a reading
  measure is bounded.
- **Body small** (400, 13–13.5px, 1.55): Descriptive prose — settings help, empty-state copy,
  dialog body, DM intro; capped at 58–62ch in settings, 44ch in empty states.
- **Label** (600, 11px, 0.07em, uppercase): Group headings — CHANNELS, DIRECT MESSAGES, ONLINE,
  IN THE CALL — plus day dividers and the autocomplete header (700, 10.5px).
- **Micro** (700, 9.5–11px, 0.04–0.06em, uppercase where labelling): Count badges, the YOU tag,
  the LIVE tag, slider labels, keyboard hint `kbd` chips.
- **Monogram** (600, 9px, line-height 1): The single initial inside a 20px avatar. A letter, not
  text — the smallest glyph in the window, and the only thing set below the micro step.
- **Code** (400, 12.5px, 1.6): Code blocks; 11.5–12px for inline code and the version readout.

The ramp is dense and lands on a half-pixel grid (9, 9.5, 10, 10.5, 11, 11.5, 12, 12.5, 13,
13.5, 14, 15, 16, 19, 20). That is the build's actual shape: fifteen steps for a window that must
fit four columns of status at once. The two ends of that ramp each have exactly one owner, and both are declared roles rather than
loose values: `monogram` (9px) is the initial inside a 20px avatar — a single letter, not text, and
10px overflows the circle — and `headline-intro` (19px) is the DM introduction heading. Neither is
a general step; don't reach for them for anything else.

Weight does most of the hierarchy work — 400 body, 600 for
nearly all emphasis, 700 only for headings and micro-labels; 500 appears twice.

### Named Rules

**The Tabular Figures Rule.** Every number that changes while you look at it is set with tabular
figures: RTT and loss in the status bar, per-peer volume readouts, timestamps, unread counts,
group counts, file sizes. The body default explicitly disables `tnum` so prose is proportional;
`.num` and `.mono` turn it back on. A figure that shifts its neighbours as it ticks is a defect.

**The Measure-On-The-Text Rule.** The 72ch cap lives on the message text and the code block —
nowhere else. Capping the message row, the hover highlight or the composer was built and
reverted: it put a visible edge mid-column while the header hairline and the stream strip ran on
to the member list. Rows span the column; only the words are bounded.

**The Mono-Means-Measured Rule.** JetBrains Mono marks code, identifiers, paths, versions and
protocol numbers. It is never used for emphasis or for a label that happens to be short.

## Layout

**Shell.** A three-row grid: 32px titlebar, fluid body, 26px status bar. The body is a four-column
grid — `56px` rail, `240px` channel column, `1fr` timeline, `232px` member list — and the three
fixed widths are tokens (`--rail-w`, `--chan-w`, `--member-w`). Every column is
`min-height: 0` / `min-width: 0` so the shell never grows past the window; `html, body` are
`overflow: hidden` and six inner surfaces scroll independently.

**Column internals.** Channel column: 44px header, scrolling list, voice panel pinned to the
bottom as an `auto` row. Main column: 48px header, optional stream strip, timeline, composer.
Settings replaces the last three columns with a two-column pane (208px nav + body, `max-width:
640px` on the content measure) by switching `.body[data-view="settings"]`.

**Spacing rhythm.** A 2px-based scale, dense at the row level and generous at the field level:
2/4/6 for intra-control gaps, 8/10/12 for control padding and card interiors, 14/16 for the
timeline gutter and column padding, 18/22 between message groups and day dividers, 28/32 for
settings page padding. Timeline rows are 2px apart within a group and 14px apart between groups —
grouping is expressed as vertical rhythm, not as a rule line.

**Responsive.** This is a resizable desktop window from roughly 780px to 3440px on one engine
(WebView2), never a phone: no touch targets, no mobile breakpoints, no fluid type. Two rules
only. Below 1080px the member list closes — *by setting state through `matchMedia`*, not by a CSS
override, so the toggle button keeps telling the truth and the user can reopen it. Below 820px
the channel column narrows to 200px and the channel topic is dropped from the header. Everything
above 1080px absorbs the extra width into the timeline.

### Named Rules

**The Truthful-Toggle Rule.** A responsive change that a control also owns is made in state, not
in CSS. A media query that overrode the member list turned its toggle into a no-op that still
reported `aria-pressed="true"`. If a breakpoint and a button can set the same thing, the
breakpoint sets the state.

**The Thread-Sits-On-The-Composer Rule.** The timeline's inner wrapper is `min-height: 100%` with
`justify-content: flex-end`, so a three-message channel rests on the composer instead of hanging
from the header, and still scrolls normally once it overflows. The view opens pinned to the newest
message and stays pinned when the stream strip appears or the composer grows.

**The Hidden-Means-Gone Rule.** `[hidden] { display: none !important }` is load-bearing, not
hygiene. The settings pane declares `display: grid`, which outranks the `hidden` attribute;
without the override the hidden pane stayed a grid item and stole 245px of height from the four
real columns. Any component with an explicit `display` needs this to exist.

## Elevation & Depth

Depth is tonal first. The six ground layers plus one hairline value do all the structural
separation — column edges, headers, cards and code blocks are a background step and a 1px line,
never a shadow. Shadows exist, and they exist for exactly one purpose: an element that floats
above the plane it belongs to. All four are offset-plus-blur, black, and increasing with distance
from the surface. There are no hard offset shadows and no glows.

### Shadow Vocabulary

Four whole shadows, each a token named for its job, plus the modal backdrop. Geometry and colour
travel together in one custom property; there is no shadow colour token, by design.

- **`--shadow-thumb`** (`0 1px 3px rgba(0,0,0,.5)`): The slider thumb — enough to read as a
  grabbable object on its own track.
- **`--shadow-bar`** (`0 4px 12px rgba(0,0,0,.4)`): The message action bar, which sits over the
  message above it inside a scroll container.
- **`--shadow-popover`** (`0 10px 28px rgba(0,0,0,.5)`): The mention autocomplete.
- **`--shadow-modal`** (`0 18px 48px rgba(0,0,0,.62)`): The confirm dialog, with `--backdrop`
  (`rgba(6,8,10,.68)`) behind it.
- Two scrims remain inline because each has exactly one user and no depth role:
  `rgba(12,15,18,.86)` for the drop overlay and `rgba(10,12,14,.82)` behind the LIVE tag.

### Named Rules

**The Rings-Are-Not-Depth Rule.** Every zero-offset `box-shadow` in this system is a ring, not
elevation: the speaking ring, the offline dot's inset ring, the titlebar button's inset focus
ring, and the slider thumb's two-stop focus ring. Read `0 0 0 Npx` as a stroke that a border could not draw, and never as a lift.

**The Whole-Shadow Rule.** A shadow is used as a whole token or not at all. Its alpha is
inseparable from its offset and blur, so there is no shadow *colour* token to reach for and no way
to pair one shadow's darkness with another's geometry. Adding a fifth shadow means adding a fifth
job, not a fifth size.

**The Float-Only Rule.** A shadow is earned by leaving the plane. Cards, panels, headers and
inputs sit *in* the layout and get a tonal step plus a hairline; only the four floating things
above get a shadow.

## Shapes

Five radii and nothing between them. **3px** is the inner radius, and it exists for one job:
the corner of a child inside a 1px-bordered group. It is the working radius minus the border, so
a segmented button's corner sits concentric inside its container instead of proud of it. Use it
only there — the segmented control's end children and the keyboard-hint chip. **4px** is the working radius: list rows, icon buttons,
badges' square siblings, chips, form controls, segmented groups, inline code, mention pills.
**6px** is the object radius: rail buttons, cards, code blocks, attachments, notes, video tiles,
the message action bar, the member row, the mention-highlighted message. **10px** is reserved for
the two largest containers a user talks to: the composer and the confirm dialog. **999px** is
circles and pills only: avatars, presence dots, count badges, progress track and fill, the switch
and its knob, scrollbar thumbs.

Corners on grouped controls come from the end children, never from `overflow: hidden` on the
group — clipping the segmented control cut the focus outline off every button inside it. Groups
that legitimately clip (code blocks, tiles, screenshots, the peer table, the dialog) contain no
focusable child whose ring could be lost.

Two silhouettes recur. The **left-edge nub**: a pill 3px wide and 22px tall with only its right corners
rounded to full, marking the current rail item, and a 4px × 8px version marking an unread row — both
positioned outside the row, so no row shifts to show state. The **ringed avatar**: a circle with
a 2px presence dot notched into its bottom-right, bordered in the panel colour so the dot reads
as cut into the surface.

## Components

### Buttons
- **Shape:** 4px on the standard button, 6px on the two panel-width actions (Join, Leave).
- **Primary (Join the call):** Accent fill with near-black ink, 34px tall, full panel width less
  its 10px margin, icon + label centred. The only accent-filled button in the resting window.
- **Neutral:** `bg-hover` fill, primary text, 28px tall, 11px side padding — the default for
  inline actions in the timeline (Download, Try again).
- **Ghost:** No fill, `border-strong` hairline, fills to `bg-hover` on hover. Used for the
  secondary half of a pair (Cancel, Share screen).
- **Danger (Leave, Delete):** `danger-bg` fill, `danger-line` hairline, `danger-text` label;
  hover brightens both fill and label. Never a solid red fill.
- **Hover / Focus:** Background and colour transition 90ms linear; `:active` on the primary drops
  1px via transform. Focus is a 2px accent outline offset 2px. Disabled is `opacity: .45` with
  the hover suppressed.
- **Icon buttons:** Three sizes for three densities — 28px (headers, self actions, composer 30px),
  40px (rail), 24px (per-peer mute), all with the icon at 14–20px and `text-muted` at rest.

### Chips
- **Accent chip** (update available, Watch screen): 15% accent tint, 30% accent hairline,
  `accent-text` label, 4px radius, 19–24px tall. Tint deepens on hover.
- **Count badge:** Rose pill, white 700 label, tabular figures, 18px (list) or 16px (rail
  overlay, with a 2px window-coloured border so it reads as cut out of the icon).
- **Quiet badge:** `bg-hover` fill with `text-mid` — a count that is not asking for attention.
- **Keyboard chip (`kbd`):** UI font, not mono, 10.5px 600, `bg-hover` on a hairline, inner radius (3px) because it is a bordered chip.

### Cards / Containers
- **Corner:** 6px. **Background:** `bg-raised` for objects you act on (attachments, notes, peer
  table rows); `bg-code`/`bg-well` for things that are content (code, screenshots, video tiles).
- **Border:** Always a 1px hairline. **Shadow:** none — see Elevation.
- **Internal padding:** 9–12px, with the caption strips at 5px.
- **Progress:** A 4px full-radius track with a full-width fill scaled on `transform: scaleX()`,
  400ms. Paused turns the fill `text-dim` and keeps the geometry.

### Inputs / Fields
- **Text field / select:** 32px tall, `bg-raised` on a `border-strong` hairline, 4px radius,
  `border-input` on hover. `select` has its native arrow suppressed and an authored chevron
  absolutely positioned in a 30px right pad.
- **Composer:** `bg-raised`, hairline, 10px radius, auto-growing textarea from 26px to 168px with
  `resize: none` and no inner outline; the container's border goes `border-strong` on
  `:focus-within`, which is the field's focus signal.
- **Range slider:** Native input, appearance stripped: 4px full-radius track painted with a
  linear-gradient stop driven by a `--pct` custom property, 12px white-ish thumb with the lifted
  shadow. Focus draws its own two-stop ring on the thumb
  (`0 0 0 2px var(--bg-sidebar), 0 0 0 4px var(--accent)`) because a native thumb cannot carry an
  outline — this is the second and last exception to the outline rule. `min-width: 0` is mandatory — as a `flex: 1` item it otherwise refuses to shrink below
  intrinsic width and pushed the volume readouts 19px outside the member panel.
- **Switch:** 38×22 pill, `bg-active` off / accent on, 16px knob translated 16px on
  `transform`, 160–200ms.
- **Error:** The attachment card carries failure in its own text (`danger-text`) and icon, with
  the row geometry unchanged. No field-level error styling exists in this build.

### Navigation
- **Rail (56px):** 40px icon buttons at 6px radius, `text-muted` at rest, `bg-active` +
  `text` when current, with the left-edge nub scaled to 1. Hover previews the nub at 0.45.
- **Channel/DM rows:** 32px, 4px radius, 8px gutter between icon and 14px name; `text-muted` →
  `text-mid` on hover → `text` on current with `bg-active`. Unread turns the row's label
  600-weight `text` and adds the outside dot plus a count badge. DM rows carry a 20px avatar with
  its presence dot instead of an icon.
- **Settings tabs:** 32px rows, 208px column, same three-step colour treatment, 600-weight when
  current.
- **Group headings:** 11px 600 uppercase with 0.07em tracking, count pushed right in tabular
  figures. Every list in the app — channels, DMs, member groups, settings nav — uses this one
  header form.

### Timeline (signature)
A 40px avatar gutter and a fluid body, 12px apart, 16px from the column edges. A message that
starts a group gets 14px of air and shows avatar, author, timestamp; a grouped continuation is
1–2px tall in margin and shows its timestamp only on hover, in the avatar column, in tabular
figures. Full-row hover uses the quiet `bg-msg-hover`. Day dividers are a hairline with a
centred uppercase label; the unread divider is the same form in rose at 50% opacity, with the
right rule cut to 24px so the label sits left.

A message that mentions you becomes an amber-black field with a `mention-border` hairline, 6px
radius, inset 12px from the column — and its left padding drops to 3px so the avatar column stays
on exactly the same x as every other message. A highlighted row that moves its own avatar reads
as a rendering bug, not as emphasis.

The hover action bar floats at `top: -12px` in a 6px-radius chip on `bg-raised` and
`border-strong`, revealed by `:hover` on the row *or* `:focus-within` on the bar itself — an
opacity-0 button is still focusable, and without that second selector Edit and Delete were
mouse-only.

### Voice panel (signature)
Bottom of the channel column on the window-black frame. When a call is live: an uppercase green
head with the waveform icon and the *worst* RTT in the call set in mono, a 28px row per
participant with a 24px avatar, and either the accent Join button or the ghost/danger action pair.
When nobody is in the call: the Join button plus one plain sentence derived from real peer state.
Below the divider, the persistent self row — 32px avatar with your own presence dot, name,
state line, and three 28px toggles (mic, deafen, do-not-disturb) which turn rose on a
`danger-bg` fill when engaged.

### Member list (signature)
232px, grouped IN THE CALL / ONLINE / CONNECTING / OFFLINE, each row a 32px avatar beside a
name, a derived sub-line ("Listening", "Retrying in 8 s", "Last seen 22:14, 3 August"), and — for
peers in the call — a mute button, a voice slider and a tabular readout on one line. When a peer
shares desktop audio, its slider gets its own labelled line rather than a second unlabelled
slider: a 14px icon cannot distinguish two sliders in a 232px column. Rows for peers who are not
online sit at `opacity: .62` and come back to full on hover.

## Do's and Don'ts

### Do:
- **Do** take every colour, radius, shadow, easing curve and column width from the custom properties on
  `:root` — including `on-signal`, the three peer-identity hues and the inner radius. The Tauri
  reproduction has to reach this surface from tokens alone, so a literal in a rule is a bug.
- **Do** animate with `transform`, `opacity`, `background-color`, `border-color` and `color`
  only, in the 90–260ms band on `cubic-bezier(.16,.84,.44,1)` (or 90ms linear for a plain
  colour swap).
- **Do** draw focus as `outline: 2px solid var(--accent)` with `outline-offset: 2px`, so the ring
  shows on whatever ground it lands on. The one exception is the message action bar's buttons,
  which use a negative offset plus a `bg-active` fill because the bar sits at `top: -12px` inside
  a scroll container where an outward ring has nowhere to go.
- **Do** derive every state string from real state. Two strings were caught claiming a peer was
  offline while the roster beside them showed a lit dot; the DM empty line and the paused-transfer
  line both branch on peer state now.
- **Do** keep UI strings English and chat content Dutch. The three users are Dutch; the mix is a
  decision, not drift.
- **Do** draw icons as inline SVG at 24×24, `fill="none"`, `stroke="currentColor"`,
  `stroke-width="1.5"`, round caps and joins, rendered at 12–20px. 29 symbols are defined once in
  a hidden `<defs>` and referenced with `<use>`.
- **Do** honour `prefers-reduced-motion: reduce` by collapsing all durations to 0.01ms.

### Don't:
- **Don't** transition or animate a layout property — no `width`, `height`, `top`, `margin`,
  `padding`. Product invariant 4 is "gaming wins": perceptible impact on a game on the same PC is
  a bug. The rail nub scales on `scaleY`, the progress bar on `scaleX`.
- **Don't** ship an `infinite` animation. There are zero in this build; an infinite `box-shadow`
  pulse on the speaking ring was removed as a non-composited repaint on the idle path. The
  authored moment is the entrance, not a heartbeat. Don't leave `will-change` parked on anything
  either.
- **Don't** add a light theme, a theme switcher or OS-theme following, and don't remove
  `color-scheme: dark` — UA chrome comes back light without it.
- **Don't** use the danger red for presence, and don't give offline a colour at all.
- **Don't** cap the width of a message row, the hover highlight or the composer. Bound the text.
- **Don't** put `overflow: hidden` on a control group that contains focusable children.
- **Don't** add a shadow to something that sits in the layout, and don't introduce a second
  accent, a fourth code hue, or a radius outside the five.
- **Don't** load fonts over the network in the shipping app. The comp links
  `fonts.googleapis.com` because a comp is opened by hand; "zero servers, no cloud API" is
  product invariant 1, so the Tauri build must bundle Archivo and JetBrains Mono locally and
  keep the same family names and weights (400/500/600/700 and 400/500).

### Recorded gaps in the comp — all three closed in the shipped frontend

These were recorded against `design/main-window.html`. `crates/app/frontend/` is the built
reproduction and fixes all three; the comp itself is untouched, so this list stays as the
record of what not to copy back.

- **Two mechanisms for one signal.** *Closed.* The speaking ring was gated by
  `[data-speaking="true"]` in the voice panel and by the presence of the `.speak-ring` class in
  the member list. Both places now carry the class and both are gated on the attribute, so the
  speaking level from the engine flips exactly one thing.
- **The icon set is 29 symbols, not the 43 the brief anticipated.** *Closed as a system, not as a
  count.* The set is still 29 and still complete for this surface; what was missing was the rule
  it was drawn to. That rule is now written above the `<defs>` block in
  `crates/app/frontend/index.html`: 24×24 grid, 1.5 stroke, round caps and joins, `currentColor`
  only, one visual weight, a toggled state ships as a pair sharing a silhouette plus the same
  slash, and the size is set by the caller. A new surface needs drawings that follow it, not a
  near-enough substitute from this set.
- **Fonts are remote in the comp.** *Closed.* Archivo and JetBrains Mono ship as four woff2 files
  in `crates/app/frontend/fonts/`, declared in `fonts/faces.css`. Both families are variable, so
  one file per subset (latin, latin-ext) covers every weight the system uses instead of the eight
  byte-identical files a naive download produces. Product invariant 1 is "zero servers": the
  shipping build must never fetch a font.

### Where the shipped frontend departs from the comp, and why

The comp is the reproduction target and the CSS is taken from it verbatim. Five things
nevertheless differ, each because the comp had no engine behind it:

- **Peer identity hues are `av-1`/`av-2`/`av-3`, not `av-r`/`av-s`/`av-j`.** The comp hardcoded
  one class per demo peer. Peers come from configuration, so the hue is an index assigned by
  position in the sorted set of known peer ids — stable for the life of the install, and
  collision-free for three, which hashing the id was not.
- **No packet-loss figure in the status bar.** The comp shows `0.4% loss` beside the RTT. The
  engine measures RTT and does not measure loss, and "derive every state string from real state"
  outranks reproducing a number there is no source for. The RTT stays.
- **"Last seen 22:14, 3 August" is only shown when it is true.** The time is observed while the
  process runs and is not persisted, so a peer who has not been up since this start reads plain
  "Offline" instead of a made-up time.
- **The code block's header carries the fence info string**, not a fixed filename, and its Copy
  button works. A bare fence gets the header with the word `code`.
- **Four surfaces the comp did not have to draw**: the share-source picker, the two text prompts
  for sub-channels, in-place message editing, and the engine's own error line. All four are built
  from the tokens above and reuse the comp's own components; they are in the one marked section at
  the bottom of `app.css`.

Three gaps recorded in the first pass are now closed in the artifact and are documented above as
system values rather than as defects: `--focus` no longer dangles (the slider thumb declares its
own two-stop ring, `0 0 0 2px var(--bg-sidebar), 0 0 0 4px var(--accent)`, because a native thumb
cannot carry an outline), `#fff` is `on-signal`, and the three avatar hues are `av-1`–`av-3`.

The four shadows are now tokens too — `--shadow-thumb`, `--shadow-bar`, `--shadow-popover`,
`--shadow-modal`, plus `--backdrop` — named by job and carrying geometry and colour together. That
is the stronger form of the rule I first recorded as a non-tokenization: "copy a whole shadow or
don't use one" is now mechanically true rather than something a future author has to remember.

**Not tokenized, deliberately.** The review harness's own chrome — its two off-token greys and its
`rgba(0,0,0,.6)` shadow — is a state-toggling scaffold that does not ship, so it is not part of
the system and should not be reproduced.
