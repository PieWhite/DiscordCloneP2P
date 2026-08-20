---
name: FitCommunication
description: A server-free desktop comms client for three friends, drawn as a sixteen-color Japanese PC screen — one owned field, one fixed dark theme.
colors:
  space: "#070810"
  ink: "#10122A"
  indigo: "#191D3A"
  violet: "#2E3464"
  steel: "#4A5490"
  silver: "#A7AECC"
  paper: "#EFE9D5"
  blue: "#3B58C4"
  cyan: "#5FD3E0"
  sky: "#8FB8F0"
  magenta: "#D460A2"
  coral: "#F0716B"
  orange: "#E08A48"
  amber: "#E7BC5D"
  lime: "#C4DC78"
  green: "#62CE82"
typography:
  body:
    fontFamily: "DotGothic16, 'Segoe UI Variable Text', 'Segoe UI', system-ui, sans-serif"
    fontSize: "15px"
    fontWeight: 400
    lineHeight: 1.45
  title:
    fontFamily: "DotGothic16, 'Segoe UI Variable Text', 'Segoe UI', system-ui, sans-serif"
    fontSize: "13px"
    fontWeight: 400
    lineHeight: 1.45
    letterSpacing: "0.08em"
  label:
    fontFamily: "DotGothic16, 'Segoe UI Variable Text', 'Segoe UI', system-ui, sans-serif"
    fontSize: "11px"
    fontWeight: 400
    letterSpacing: "0.08em"
  code:
    fontFamily: "'JetBrains Mono', ui-monospace, 'Cascadia Mono', Consolas, monospace"
    fontSize: "12.5px"
    fontWeight: 400
    lineHeight: 1.6
rounded:
  none: "0"
components:
  button-primary:
    backgroundColor: "{colors.magenta}"
    textColor: "{colors.space}"
    rounded: "{rounded.none}"
    height: "34px"
  button-secondary:
    backgroundColor: "{colors.space}"
    textColor: "{colors.paper}"
    rounded: "{rounded.none}"
    height: "28px"
    padding: "0 11px"
  button-secondary-hover:
    backgroundColor: "{colors.paper}"
    textColor: "{colors.space}"
  button-danger:
    backgroundColor: "{colors.space}"
    textColor: "{colors.coral}"
    rounded: "{rounded.none}"
  button-danger-hover:
    backgroundColor: "{colors.coral}"
    textColor: "{colors.space}"
  badge-count:
    backgroundColor: "{colors.paper}"
    textColor: "{colors.space}"
    rounded: "{rounded.none}"
    height: "18px"
    padding: "0 5px"
  input-field:
    backgroundColor: "{colors.space}"
    textColor: "{colors.paper}"
    rounded: "{rounded.none}"
    height: "32px"
    padding: "0 10px"
---

# Design System: FitCommunication

## Overview

**Creative North Star: "The Sixteen-Color Evening Machine"**

FitCommunication is drawn as a Japanese sixteen-color PC screen of the early nineties
(PC-98 lineage, seed key c9bbaaa4, 2026-08-12): a locked sixteen-color palette on deep
indigo, paper text, tinted title strips on square double-bordered windows, and a machine
that answers instantly — state changes snap like a palette swap, nothing fades, nothing
pulses. This world replaced "The Well-Made Standard" (the deliberate category-standard
direction of 2026-08-04) wholesale; `design/main-window.html` and `design/shots/`
document the *replaced* world and are anti-reference for this one. The four-zone layout,
the Snapshot/UiCommand boundary, and all behavior were preserved; only the visual world
changed.

The material logic is the pixel grid. There is no translucency, no blur, no gradient
blend anywhere in the resting system: every mid-tone is an ordered 2px dither of exactly
two palette colors, the modal backdrop is a
checker of the void, and "unavailable" is a half-density dither laid over the thing
itself. Depth is a hard offset shadow in the void, the way a sprite casts one. Selection
is a full cell inversion — paper ground, space ink — inherited from the source machines'
one selection device.

Product truth binds the world: no network fetch ever (fonts are bundled woff2), "gaming
wins" (no layout-property transitions, zero infinite animations), offline and
reconnecting are normal states and are never alarm-colored, and the theme is fixed dark
with `color-scheme: dark` so native chrome (scrollbars, selects, caret) stays in-world.

**Key Characteristics:**
- Sixteen locked colors; every other value is an alias into the table or a 2px ordered dither of two entries
- Square everything: all radius tokens exist and are all zero
- Tinted title strips make each zone a titled window — magenta channels, blue timeline, green members, amber settings nav — the chrome is the composition
- One bitmap face at one weight; hierarchy is size, color and inversion
- Hard offset shadows, no blur; motion in `steps()`, never eased fades
- One decorative signature: the full-strength evening-sky "horizon band"

## Colors

Sixteen colors, top to bottom, and nothing else exists; hues are shared across jobs by
necessity, so meaning comes from placement and form, never from hue alone.

### Primary
- **Magenta** (#D460A2): The primary action — the one solid magenta fill in the resting
  window is Join the call. Elsewhere magenta appears only as small state marks: the LIVE
  tag's dot and border, do-not-disturb, the pressed "switched-off" tint on mute/deafen,
  and the new-messages divider line.

### Secondary
- **Cyan** (#5FD3E0): The working accent — links (dotted underline), focus rings
  (`2px solid`, offset 2px), progress fills, the caret, the update chip, share chips,
  and the "camera on / working" pressed state. Cyan is what you *use*; magenta is what
  you *commit*.
- **Blue** (#3B58C4): The timeline's title strip and the content-window headings
  (settings pane, dialog, autocomplete). The only strip dark enough to carry paper
  text; every strip's bottom edge is a 1px line of space.

### Tertiary (signal colors)
- **Green** (#62CE82): Presence — online, in call, the speaking frame — and the member
  window's title strip (space ink), which is the presence column wearing its own color.
- **Amber** (#E7BC5D): Connecting/reconnecting (a normal state, not a warning), code
  numbers, the note icon, and the settings nav's title strip (space ink).
- **Coral** (#F0716B): Failure, and only failure — error bars, delete/leave actions,
  the close button's hover. Coral and magenta must never share a value: DND is not danger.
- **Orange** (#E08A48): The mention frame, identity hue 3, and the warm stop of the band.

### Neutral
- **Space** (#070810): The void — wells, code grounds, hard shadows, ink on inverted
  cells, the dark outer line of every double frame.
- **Ink** (#10122A): Window frames, sidebars, cards, message hover.
- **Indigo** (#191D3A): The canvas, the largest field.
- **Violet** (#2E3464): Structure — hairline borders (`--border`) and the flat hover
  slab (`--bg-hover`).
- **Steel** (#4A5490): Interactive edges (`--border-strong`), disabled glyphs, the
  offline (unlit) presence square, scrollbar thumbs.
- **Silver** (#A7AECC): Secondary text — the only quiet tier.
- **Paper** (#EFE9D5): Primary text, the ground of every inverted cell, badge fills, the
  frame of the primary button.
- **Sky** (#8FB8F0): Identity hue 1, code keys; **Lime** (#C4DC78): identity hue 2,
  code strings. Identity hues (`--av-1` sky, `--av-2` lime, `--av-3` orange) are keyed
  per peer for the life of the install and color both the avatar sprite and the author
  name (`au-1/2/3`); identity is told apart by form — a filled square with a monogram —
  never by hue alone.

### Named Rules
**The Sixteen-Color Lock.** Every color on screen is one of the sixteen `--c-*` values
or an ordered 2px dither of exactly two of them. A color that is not — an rgba(), a
blend, a seventeenth hue — is a bug, not a variation. The build purged its three
formerly-rgba blends to get here.

**The One Magenta Fill Rule.** Join the call is the only solid magenta fill in the
resting window. Counts and badges are inverted paper cells; DND, LIVE and the pressed
"off" toggles use magenta only as small marks. A second magenta fill demotes the first.

**The Dither-Not-Blend Rule.** A mid-tone is `repeating-conic-gradient(A 0% 25%, B 25% 50%)`
at 2px: the modal backdrop is space-on-window cells, primary-button hover flips a
quarter of its cells to paper, an absent peer's sprite and status dither to half density.
No alpha, no gradient blends, no `color-mix`.

## Typography

**UI Face:** DotGothic16 (with Segoe UI Variable Text, system-ui fallback) — bundled
locally, latin + latin-ext, weight 400 only.
**Code/Figures Face:** JetBrains Mono (with ui-monospace, Consolas fallback) — a named
concession: a chat between developers has real code in it, and legibility there is
product truth, not styling. Also carries measured figures via `tnum`.

**Character:** A genuine Japanese bitmap revival at one weight. `font-synthesis: none`
is set globally because a synthesized bold smears the pixels; hierarchy is built from
size, color, casing and cell inversion — never weight. The bitmap face runs wider than
a grotesque; labels give way before targets do.

### Hierarchy
- **Body** (400, 15px, 1.45–1.5): messages and the composer; message text is paper,
  bounded at 72ch on the text itself, never on the row.
- **Title** (400, 13–15px, uppercase, .06–.08em tracking): window title strips —
  paper on blue.
- **Label** (400, 10–11.5px, uppercase where sectional, .04–.08em): group heads, day
  dividers, hints, sub-lines — silver.
- **Code** (400, 12.5px, 1.6): JetBrains Mono in bordered code windows on the void;
  syntax colors are sky (keys), lime (strings), amber (numbers), silver (comments).

### Named Rules
**The One Weight Rule.** There is no bold. `font-weight: 400` everywhere, `<b>` elements
are explicitly reset to 400 and speak through color (paper against silver). Emphasis is
size, color, casing or inversion.

**The Two Tiers Rule.** Text is paper or silver — the old four-step grey ladder
collapsed to two. A third grey is a bug.

## Layout

A fixed desktop shell: `32px` titlebar / `1fr` body / `26px` status bar. The body is
four fixed zones as four titled windows — rail (`--rail-w: 56px`), channels
(`--chan-w: 240px`), timeline (`1fr`), members (`--member-w: 232px`) — each column
window opening with a `44px` blue title strip that never scrolls away. Separation is a
1px violet border and a title strip, not a tonal ladder: the world has three darks
(space, ink, indigo) and structure does the rest.

Spacing rhythm is tight and even: 2/4/6/8px inside controls, 10–16px pane padding,
14–22px between message groups and sections. Rows are 28–32px tall; icon buttons are
24–46px square targets. The timeline sits *on* the composer (`justify-content: flex-end`
in a min-height:100% inner), so a short thread rests at the bottom, not hanging from the
header. The reading measure lives on the text (72ch), while rows, hover fields and the
composer span the full column.

Responsive: at ≤820px the channel column narrows to 200px, the topic drops, and self
toggles slim to 24px (the label gives way, never the target). Below 1080px the member
list is dropped by *setting state* in the script (matchMedia), never by a CSS override
that would desync `aria-pressed`.

## Elevation & Depth

No soft shadows, no blur, no glow. Elevation is a hard offset block of the void
(`--c-space`), the way a sprite casts a shadow, and it belongs only to surfaces that
float above the plane: hover toolbars, popovers, modals, thumbs, the settings card and
the primary button. Flat chrome (columns, strips, rows) has no shadow at all. The modal
backdrop dims by dithering: an ordered space-on-window checker at 4px, never an alpha
veil.

### Shadow Vocabulary
- **Thumb** (`1px 1px 0 var(--c-space)`): the range-slider thumb.
- **Bar** (`3px 3px 0 var(--c-space)`): floating toolbars (message actions), the
  settings card, the empty-state icon block.
- **Popover** (`4px 4px 0 var(--c-space)`): autocomplete and other anchored popovers.
- **Modal** (`8px 8px 0 var(--c-space)`): dialogs.
- **Button chassis** (`0 0 0 1px var(--c-space), 3px 3px 0 var(--c-space)`): the primary
  button's dark outer line plus its cast shadow; `:active` translates 1px,1px and
  shortens the offset to 2px — the press is physical.

### Named Rules
**The Hard Offset Rule.** Every shadow is `Xpx Xpx 0` in space — zero blur, and only on
floating surfaces. A blurred or ambient shadow does not exist in this world.

## Shapes

Square is the world. Every radius token (`--r-inner` through `--r-full`) exists and is
zero — they stay as tokens so a future world can turn corners back on without hunting
literals, but in this one nothing is rounded: not badges, not avatars, not presence
dots, not switches, not scrollbar thumbs.

The signature silhouette is the **double-bordered system window**: a paper or steel
frame set in a 1px dark outer line of space (`border` + `0 0 0 1px var(--c-space)`),
title strip on blue, hard shadow behind. The primary button, the composer, the confirm
dialog and its footer buttons all carry this chassis. Borders are 1px violet for
structure, 1px steel for interactive edges, 2px paper for the heaviest frames.

The one decorative signature is the **horizon band**: a hard-stop evening sky running
cold to warm at full strength (cyan → sky → blue → magenta → coral → orange, `--band`),
edge to edge — never dimmed and never started in a dark hue, which made the first
version fade into the ground. It appears in exactly three places: the
titlebar's bottom edge (3px), the settings window's bottom edge (4px), and the
empty-state heading's underline (4px × 96px). It is a signature, not a utility — do not
put it on new surfaces without demoting one of these.

Icons are a system, not 31 loose drawings: 24×24 grid on whole and half pixels,
stroke 2, `fill="none"`, square caps, miter joins, no rounded rects, one visual weight
for the whole set, rendered at 12–20px with size set by the caller. Toggling states ship
as pairs with the same silhouette plus a slash from (3.5,3.5) to (20.5,20.5). Color is
never hardcoded — icons inherit `currentColor` from the element they sit in.

## Components

### Buttons
- **Shape:** square (0 radius), all variants.
- **Primary (Join the call):** solid magenta fill, space text, 34px tall, in the full
  chassis — 2px paper frame, 1px space outer line, 3px hard shadow. Hover brightens by
  dither (a quarter of the cells flip to paper at 4px); active presses in (translate
  1px,1px, shadow shortens). The one saturated fill at rest.
- **Standard (.btn):** space fill, 1px steel border, paper text, 28px, 12.5px. Hover
  inverts: paper fill, space text. Ghost variant is the same with a transparent ground.
- **Danger:** space fill, coral text and border; hover fills coral with space text.
  Reserved for destructive/leave actions only.
- **Disabled:** `opacity: .45`, hover suppressed. (Doctrine is half-density dither; see
  Do's and Don'ts.)
- **Focus:** cyan `outline: 2px solid`, offset 2px — drawn inside (offset −2px, with a
  paper fill behind it) only where the ring has nowhere to go, e.g. the floating message
  toolbar.

### Chips
- **Style:** outlined capsules-without-the-capsule: square, space ground, 1px border and
  text in the accent that owns them (cyan for share/update chips), 11–11.5px.
- **State:** hover inverts into the accent (cyan fill, space text).

### Cards / Containers (titled windows)
- **Corner Style:** square.
- **Background:** ink on the indigo canvas; wells and code grounds are space.
- **Title:** a 44px (inline: ~34px) blue strip, paper uppercase text, 1px space bottom
  edge. A column *is* a window because it has this strip.
- **Border:** 1px violet (structure) or steel (interactive); dialogs wear 2px paper.
- **Shadow:** only if floating (see Elevation).
- **Internal Padding:** 10–20px.

### Link cards in the log (attachment, picture, video)
Three things sit in the message column that are not text, and they share one chassis: a
440px measure (the picture card 420px), 1px violet border, ink ground, `6px 0 2px` margin.
- **Attachment:** 34px space icon plate, name over a silver sub-line, one `.btn` on the
  right. The button is the state — Download, Downloading, Try again, Open, or Show.
- **Picture:** the image scaled to the card width with an ink caption strip under it
  (11px silver). It is a preview, not the picture: clicking opens the lightbox.
- **Video (`.yt`):** a 160×90 thumbnail cropped with `cover` out of a 4:3 source, a play
  glyph on a space plate with a paper frame (a thumbnail is somebody else's colours, so
  the mark needs its own ground), then the title clamped to two lines over a silver
  `YouTube · channel` line. Hover moves the border to cyan — it is a link.

### Lightbox (a picture at full size)
Same chassis as the confirm dialog — 2px paper frame, blue title strip, modal shadow,
dithered backdrop — but sized by its content rather than to a measure. Two states and no
zoom control: fitted (bounded by the window, no scrolling) and `is-full` (one image pixel
per screen pixel, the frame scrolls). Centring gives way in `is-full`, because a centred
child that overflows its scroll container puts its own top-left corner out of reach.

### Inputs / Fields
- **Style:** space ground, 1px steel border, square, 32px tall, 13.5px; the composer is
  an ink window with the double chassis (`0 0 0 1px space, 2px 2px 0 space`), cyan caret.
- **Focus:** border switches to cyan (`focus-within` on the composer); no glow.
- **Range slider:** square track (6px, violet border) with a hard two-stop fill (cyan
  progress against space — a hard stop, not a blend), paper thumb 10×14 with the thumb
  shadow; focus states with a cyan ring of its own.
- **Switch:** 38×22 square, space ground, steel border; the 16px silver knob *steps*
  across (`steps(3)`), and on it becomes cyan ground with a space knob.
- **Error:** coral text/border on a space ground (`.error-bar`, attachment errors) —
  never a red fill behind text.

### Navigation (rows: channels, tabs, pickers, autocomplete)
- **Default:** silver text, steel icons, 32px rows.
- **Hover:** a flat violet slab (`--bg-hover`) with a 5×8px cyan ▶ marker clipped in at
  the row's left edge — the source world's menu pointer; text lifts to paper. The
  dithered hover was tried and read as mud on a real screen (2026-08-12).
- **Current/selected:** full cell inversion — paper ground, space text/icons
  (`aria-current`, `aria-selected`, `aria-pressed` all share this grammar).
- **Counts:** paper badge with space digits; on an inverted current row the badge
  re-inverts (space ground, paper digits) or it would vanish. Quiet counts sit on violet.
- **Unread:** text lifts to paper, guarded with `:not([aria-current])`; countless unread
  is a 4×8 paper block left of the row.
- **Rail:** 40px square buttons; the active marker is a 3px cyan block scaled on the
  compositor in `steps(2)`, never an animated height.

### Avatars & Presence (signature)
Avatars are sprites: filled squares (20/24/32/40px) in the peer's identity hue with a
one-letter space-ink monogram. Presence is a lit square notched into the corner (9px,
2px ink bezel): green online, amber connecting, magenta DND, and offline is an *unlit*
square (ink fill, steel inset line) — an off pixel, not a warning. The speaking frame is
a 2px green cell border that snaps on around the sprite (`steps(2)`, 120ms) while the
peer talks — an entrance, never a heartbeat; it does not pulse.

### Absent peers (signature)
A peer who is away dithers to half density — an ordered ink checker laid over the sprite
and the status word only. The name stays full-density silver: offline is a normal state,
and a name you cannot read in a dark room fails the build's chosen failure test. Hover
lifts the screen off the whole row.

## Do's and Don'ts

### Do:
- **Do** build every color from the sixteen `--c-*` tokens; a new mid-tone is an ordered
  2px `repeating-conic-gradient` of exactly two of them.
- **Do** make selection an inversion (paper ground, space ink) and re-invert anything
  that sits on an inverted row.
- **Do** snap state changes with `steps()` (90–400ms) and keep every transition on
  compositor properties (`transform`, `opacity`); honor `prefers-reduced-motion`.
- **Do** give a new surface the window grammar: a tinted title strip in the zone's own
  color (bright fills carry space ink, blue carries paper), 1px space under-edge,
  violet/steel border, square corners, hard offset shadow only if it floats.
- **Do** keep icons on the 24×24 stroke-2 square-cap miter grid, colored by
  `currentColor`; draw a missing glyph to these rules rather than substituting.
- **Do** keep JetBrains Mono for code blocks and tabular figures — the one non-bitmap
  face, by concession.

### Don't:
- **Don't** use alpha, blur, gradient blends, or `color-mix` — no translucency exists in
  this world; dim by dithering (the modal backdrop is the model).
- **Don't** add a second solid magenta fill to the resting window, and don't let magenta
  (DND, LIVE, primary) and coral (failure) trade jobs — they must never share a value.
- **Don't** round a corner, blur a shadow, or synthesize a bold — all radii are 0, all
  shadows are hard offsets in space, and the face has one weight.
- **Don't** run an infinite animation or transition a layout property — "gaming wins" is
  a product invariant; the speaking frame appears, it never pulses.
- **Don't** dither an absent peer's *name* — sprite and status word only; the name stays
  legible in a dark room.
- **Don't** alarm-color offline or reconnecting: offline is an unlit steel square,
  connecting is amber, and neither is coral.
- **Don't** derive values from `design/main-window.html` or `design/shots/` — they
  document the replaced 2026-08-04 world and are anti-reference here.
- **Don't** fetch a font or any asset from the network; faces ship as bundled woff2.
