---
name: FitCommunication
description: A server-free desktop comms client for three friends, drawn as pure black chrome with hairline borders, one pale accent and a serif for titles, one owned field, one fixed dark theme.
colors:
  void: "#000000"
  lift: "#0B0E14"
  well: "#05070A"
  hairline: "#292D30"
  charcoal: "#464A4D"
  iron: "#6E727A"
  ash: "#A1A4A5"
  bone: "#F0F0F0"
  white: "#FFFFFF"
  chrome: "#D4D7DB"
  chrome-dim: "#B8BCC2"
  green: "#3AD389"
  red: "#FF9592"
  red-line: "#5A2A2A"
  red-bg: "#1A0B0B"
  amber: "#FFCA16"
  sky: "#70B8FF"
typography:
  body:
    fontFamily: "Inter, 'Segoe UI Variable Text', 'Segoe UI', system-ui, sans-serif"
    fontSize: "14px"
    fontWeight: 400
    lineHeight: 1.45
  title:
    fontFamily: "'Instrument Serif', Georgia, serif"
    fontSize: "26px"
    fontWeight: 400
    lineHeight: 1.2
  label:
    fontFamily: "'JetBrains Mono', ui-monospace, 'Cascadia Mono', Consolas, monospace"
    fontSize: "11.5px"
    fontWeight: 400
    letterSpacing: "0.08em"
  code:
    fontFamily: "'JetBrains Mono', ui-monospace, 'Cascadia Mono', Consolas, monospace"
    fontSize: "12.5px"
    fontWeight: 400
    lineHeight: 1.6
rounded:
  control: "6px"
  field: "10px"
  card: "16px"
  full: "9999px"
components:
  button-primary:
    backgroundColor: "{colors.void}"
    textColor: "{colors.white}"
    rounded: "{rounded.control}"
    height: "30px"
    padding: "0 12px"
  button-secondary:
    backgroundColor: "{colors.void}"
    textColor: "{colors.white}"
    rounded: "{rounded.control}"
    height: "30px"
    padding: "0 12px"
  button-secondary-hover:
    backgroundColor: "{colors.void}"
    textColor: "{colors.white}"
  button-danger:
    backgroundColor: "{colors.void}"
    textColor: "{colors.red}"
    rounded: "{rounded.control}"
    height: "30px"
  badge-count:
    backgroundColor: "{colors.chrome}"
    textColor: "{colors.void}"
    rounded: "{rounded.full}"
    height: "18px"
    padding: "0 5px"
  input-field:
    backgroundColor: "{colors.void}"
    textColor: "{colors.bone}"
    rounded: "{rounded.field}"
    height: "32px"
    padding: "0 10px"
---

# Design System: FitCommunication

## Overview

**Creative North Star: "Black Chrome"**

FitCommunication is pure black glass with hairlines: a void ground, one lifted tone for
what is selected or hovered, one-pixel borders, white for what matters and ash for the
rest, and a single pale-chrome accent. Titles are set in a serif, measurements in a mono,
everything else in a plain grotesque. It is quiet on purpose: the app sits next to a game
or a screenshare, so nothing glows, nothing moves and nothing competes for the eye.

This world replaced "The Sixteen-Color Evening Machine" (PC-98, seed key c9bbaaa4,
2026-08-12) on 2026-10-20, because Rick was done with retro and because the call controls
and volume sliders had become cluttered while sharing a screen (`docs/OVERDRACHT.md`,
decision 48). `design/main-window.html` and `design/shots/` are older anti-references and
document worlds that no longer apply; `design/redesign-b/` (`MainB`, `SettingsB`, `TokensB`
as `.dc.html`) holds the B mockups, and their inline styles are the source of exact
values. The four-zone layout, the Snapshot/UiCommand boundary and all behavior were
preserved; the visual world and the call-controls arrangement changed.

Product truth binds the world: no network fetch ever (fonts are bundled TTF, inside the
binary), "gaming wins" (no animation that runs on its own, zero infinite motion), offline
and reconnecting are normal states and are never alarm-colored, and the theme is fixed
dark.

**Key Characteristics:**
- Dark only: void ground, hairline borders, no hard shadows, no dither, no horizon band
- One accent, chrome; green, red, amber and sky are signals, never decoration
- Selected is a lift ground, a 1px hairline inset and white text, never an inversion
- Three faces with three jobs: Inter (everything), Instrument Serif (pane titles), JetBrains Mono (figures and labels)
- Radii on a ladder of 6, 10, 16 and full
- Every volume has exactly one home, the per-person audio popover

## Colors

One neutral ramp from void to white, one accent, five signals. Meaning comes from
placement and form first; hue never carries it alone.

### Primary
- **Chrome** (#D4D7DB): The one accent: focus ring (`2px`, offset 2px), count badges,
  slider fills, the watched-stream outline, the "NEW" divider, links' hover partner.
  **Chrome Dim** (#B8BCC2) colors links and code numbers.

### Signals
- **Green** (#3AD389): Presence, the speaking ring, success.
- **Red** (#FF9592): LIVE, errors, Leave, do-not-disturb, a pressed "mic off" toggle.
  **Red Line** (#5A2A2A) is its outline, **Red Bg** (#1A0B0B) the ground of a pressed-off
  control. There is no magenta any more, so DND is red.
- **Amber** (#FFCA16): Connecting and reconnecting (a normal state), Wordle "near".
- **Sky** (#70B8FF): Identity hue 1 and code keys.

### Neutral
- **Void** (#000000): The ground of everything, avatars, buttons and inputs.
- **Lift** (#0B0E14): Selected rows, hover, cards on the void.
- **Well** (#05070A): Message hover, code grounds, sunken fields.
- **Hairline** (#292D30): Every 1px border.
- **Charcoal** (#464A4D): Disabled glyphs, scrollbar thumbs.
- **Iron** (#6E727A): Hover borders, offline (unlit) presence, quiet icons.
- **Ash** (#A1A4A5): Muted text, sub-lines, row text at rest.
- **Bone** (#F0F0F0): Body text. **White** (#FFFFFF): headings and names.
- Identity hues (sky, amber, green for peers 1, 2, 3) colour the monogram letter and the
  author name; the avatar itself stays void with a hairline ring.

### Named Rules
**The One Accent Rule.** Chrome is the only accent. A second accent hue for an action or
a state is a bug; signals (green, red, amber, sky) mark state and identity only.

**The Offline-Is-Not-Red Rule.** Offline is an iron unlit dot, connecting is amber. Red
means LIVE, error, Leave, DND and pressed-off, never a missing peer.

**The Token Rule.** Every color is a `theme::c` value above. An inline hex, or a
translucent blend other than the modal backdrop, is a bug.

## Typography

**UI Face:** Inter, static cuts Regular 400, Medium 500, SemiBold 600, bundled locally.
**Title Face:** Instrument Serif Regular, for pane titles only (Channels, Members,
# channel, Settings, dialog titles, the popover name); 26px in panes, 32px in settings.
**Mono Face:** JetBrains Mono, for timestamps, counts, ms, UPPERCASE section labels at
11.5px with `tracked`, the status bar, and file names and sizes. Also code blocks.

**Character:** A plain grotesque that disappears, a serif that gives each pane a name,
and a mono that makes figures read as measurements. The OFL texts sit next to the font
files.

### Hierarchy
- **Body** (400, 14px, 1.45): messages and the composer; bounded at 72ch on the text
  itself, never on the row.
- **Title** (Instrument Serif, 26px, 32px in settings): white.
- **Label** (JetBrains Mono, 11.5px, uppercase, tracked): ash.
- **Weights:** names 500, buttons 500, badges 500; headings and emphasis may use 600.
- **Code** (400, 12.5px, 1.6): JetBrains Mono on well; keys sky, numbers chrome dim,
  comments ash.

### Named Rules
**The Three Jobs Rule.** Serif for titles, mono for figures and labels, Inter for the
rest. A fourth face, or a serif in running text, is a bug.

## Layout

A fixed desktop shell: titlebar / body / status bar. The body is four zones: rail,
channels, timeline, members. Widths live in `ui/theme.rs`. Separation is a 1px hairline,
not a tonal ladder: the world has void, well and lift, and the lines do the rest.

Spacing is tight and even: 2/4/6/8px inside controls, 10 to 16px pane padding, 14 to 22px
between message groups and sections. Rows are 32px; icon buttons are 28 to 46px targets.
The reading measure lives on the text (72ch), while rows, hover fields and the composer
span the column.

### Call controls (information architecture)
Each control has one home, so a screenshare is never crowded. Two deliberate second homes,
as in Discord: Watch (stream tile and voice-list hover) and stopping a screen (the share
button and that screen's own line).
- **Self panel** (bottom of the channels column): avatar, name, status, mic, deafen and
  do-not-disturb. Mic and deafen stay here always, joined or not.
- **Voice-connected panel** (only while joined): in the channels column between the voice
  roster and the self panel, stacked as in Discord. Top: "Voice connected" in green, "N
  people · worst RTT" in mono under it, Leave on the right. Under that one row of equal
  buttons: camera, share screen, Spotify.
- **Share button:** while not sharing it opens the picker; while sharing it shows the
  crossed monitor and one click stops every own screen, no menu. Spotify likewise shows the
  crossed note while on. The camera button keeps its own convention (crossed while off).
  Allow drawing and Clear drawing live in Settings → Video, one saved switch for every own
  screen (`allow_drawing`).
- **Stream <game>:** a full-width button under the call buttons while a game is in front
  and no screen is shared (Windows, with "Show the game I'm playing" on); one click shares
  that game's window, no picker.
- **Panel lines:** inside the voice-connected panel, one quiet line per own screen, "You
  share <title> · N watching" with its own stop (X), and per shared Spotify: "You share
  <title> · N listening" for the DJ, "<name> plays <title>" with play/pause and next for a
  listener. Those two music buttons live nowhere else. The "now sharing" lines are not in
  the channels column.
- **Channels column, voice section:** only who is in the call (avatar, name, LIVE and
  muted marks) plus Join the call when not joined. Hovering a peer turns LIVE into Watch
  for a stream you do not watch yet, screens before cameras. No other buttons.
- **Channels column, DMs:** none. Direct messages live only behind the @ in the rail.
- **Stream tiles:** every screen or camera of someone in the call has a tile, watched or
  not (an unwatched one shows its title on the well). A peer's tile has Watch / Stop
  watching (an eye glyph; Watch also sits on the voice-list hover) and a speaker button
  that opens that person's popover, opening the member list if it was closed. Your own
  tiles have neither.
- **Settings → Network:** a trash button per peer, behind a confirm dialog, removes that
  peer locally; the others keep listing them.
- **Members:** one line per person (avatar, name, mono sub-line), no sliders or chips
  inline. A click on a peer opens their audio popover (serif name, "only for you"): Voice,
  Screen audio (if sharing) and Spotify (if you hear it), each with a mute button and a
  percentage. Every volume slider lives there and nowhere else. Your own row opens nothing.

## Elevation & Depth

Flat. Depth is a hairline and a lift tone, not a shadow. The only shadow is one soft black
one under popovers and dialogs (`0 20px 50px rgba(0,0,0,.8)`), because a black panel on
a black ground needs something to separate it. The modal backdrop is flat void at about
85% alpha. Disabled is opacity .45.

### Named Rules
**The One Shadow Rule.** One soft black shadow, popovers and dialogs only. No hard offset
shadows, no glow, no stacked elevations.

## Shapes

Radii climb with the size of the thing: **6** controls, buttons, inputs and rows; **10**
the composer and the self panel; **16** cards (stream tiles, attachment, picture and link
cards, code blocks, popovers, dialogs, settings sections); **full** avatars, dots and
badges.

Avatars are a monogram letter in the peer's hue on void with a 1px hairline ring; the
ring turns green while the peer speaks. Presence is a small round dot: green online,
amber connecting, red DND, and offline is an unlit iron dot.

Icons are a system: 24×24 grid, stroke 1.5, `fill="none"`, round caps and joins, one
visual weight, rendered at 12 to 20px with size set by the caller. Toggling states ship as
pairs with the same silhouette plus a slash. Color is never hardcoded; icons inherit
`currentColor`.


## Logo

The wordmark is "Fit" in Instrument Serif (white) followed by "com" in JetBrains Mono (ash)
and a static chrome cursor bar: the app's two type voices in one name. It sits in the
titlebar at 20px. The app icon is a black tile (#16191E to #000, radius 36/160, hairline
edge) carrying `>Fit_`: a mono prompt in iron, the serif "Fit", the chrome cursor. At 48px
and below the prompt drops. Source: `crates/app/icons/icon.svg` (glyphs as outlines);
`icon.png`, `icon.ico` and `tray-32.png` are renders of it.

## Components

### Buttons
Ghost first: void ground, 1px hairline, white text, weight 500, 30px tall, radius 6.
Hover moves the border to iron or white. **Join the call** is the same button with a 1px
bone outline. **Danger and Leave:** red text on a red-line border. Disabled is opacity .45.
Icon buttons are 32px, transparent, ash glyph; hover is a lift ground and a white glyph.

### Inputs / Fields
- **Style:** void ground, 1px hairline, 32px tall, radius 6; the composer is radius 10.
- **Focus:** a 2px chrome ring, offset 2px; no glow.
- **Range slider:** chrome fill and thumb on a hairline track. Every slider is in the
  per-person audio popover or in settings.
- **Error:** red text and a red-line border on void, never a red fill behind text.

### Navigation (rows: channels, tabs, pickers, autocomplete)
- **Default:** ash text, 32px rows, radius 6.
- **Hover:** a lift ground and white text.
- **Current/selected:** lift ground, 1px hairline inset, white text.
- **Counts:** a chrome badge with void digits, radius full, weight 500.
- **Unread:** the name lifts to white.

### Cards and popovers
Radius 16, 1px hairline, lift or void ground. Stream tiles carry Watch (so does the
voice-list hover), and the tile of a
stream you watch has a chrome outline. The audio popover carries the peer's serif name and
one slider row per source.

### Avatars & Presence
See Shapes. The speaking ring snaps on while the peer talks; it never pulses.

### Absent peers
A peer who is away shows an iron dot and an ash name, and the name stays legible: offline
is a normal state.

## Do's and Don'ts

### Do:
- **Do** take every color from `theme::c`; chrome is the only accent.
- **Do** make selection a lift ground, a 1px hairline inset and white text.
- **Do** give every volume exactly one home, the per-person audio popover.
- **Do** keep mic and deafen in the self panel and the rest of the call controls in the
  voice-connected panel right above it while joined.
- **Do** keep icons on the 24×24 stroke 1.5 round grid, colored by `currentColor`.
- **Do** state changes instantly; the speaking ring and the typing indicator are the only
  moving things, and neither loops while nothing happens.

### Don't:
- **Don't** add a dither, a horizon band, a hard offset shadow, or an inversion as
  selection: the sixteen-color world is gone.
- **Don't** add a second accent, or let red mean offline or reconnecting.
- **Don't** run an infinite animation, ease a layout property or add a spring: "gaming
  wins" is a product invariant.
- **Don't** put a slider, chip or button inline in a member row, or a call control in the
  channels column.
- **Don't** fetch a font or any asset from the network; faces ship as bundled TTF inside
  the binary.
- **Don't** derive values from `design/main-window.html` or `design/shots/`; they are
  anti-references.
