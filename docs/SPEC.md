# SPEC — P2P communicatie-app (3 peers, Tailscale, geen servers)

Vastgelegd 2026-07-28. Vervangt `OPEN-QUESTIONS.md`.
Dit document is de bron van waarheid voor *wat* we bouwen. Het *hoe* staat in `ARCHITECTURE.md`.

## Doel
Lichtgewicht, servervrij alternatief voor Discord voor precies 3 vaste gebruikers.
Kernwaarde: **het mag geen merkbare impact hebben op gamen op dezelfde PC.**
Dat is de belangrijkste niet-functionele eis en wint van kwaliteit waar ze botsen.

## Deelnemers en hardware
| Peer | GPU | Rol |
|---|---|---|
| Dev-PC (deze machine) | RTX 5070 Ti (Blackwell) + AMD Radeon iGPU | peer 1, dev + test |
| Vriend | RTX 3090 (Ampere) of RTX 2080 Super (Turing) | peer 2, testpartner |
| Derde | de andere van bovenstaande twee | peer 3 |

- 1 monitor per persoon. Uitgangspunt was 1080p @ 60 Hz; de capture-, codec- en
  weergavelaag zijn resolutie-onafhankelijk (geen hardcoded 1920×1080 ergens in de
  keten) en zijn sinds fase 10 ook bevestigd op 1440p en 3440×1440 (ultrawide) — zie
  `ROADMAP.md`, fase 10.
- Iedereen gebruikt altijd een headset.
- Alle drie: 1 Gbit/s symmetrisch, Tailscale.
- Windows only.

**Codecgevolg:** alle drie zijn NVIDIA Turing of nieuwer. AV1 valt af (2080 Super kan
AV1 niet encoden én niet decoden).

Bij het bouwen van fase 4 bleek de eerdere keuze voor HEVC niet houdbaar en is hij
omgedraaid naar **H.264 als standaard, HEVC als optie**. Gemeten op de dev-PC:

| | Encoder | Decoder |
|---|---|---|
| HEVC | `NVIDIA HEVC Encoder MFT` ✅ | `HEVCVideoExtension` — Store-uitbreiding, **niet standaard aanwezig** |
| H.264 | `NVIDIA H.264 Encoder MFT` ✅ | `Microsoft H264 Video Decoder MFT` — zit altijd in Windows ✅ |

Encoden kan met beide; het probleem zit aan de ontvangstkant. Zonder de HEVC Video
Extensions kan een peer een HEVC-stream simpelweg niet decoderen, en dat is een
onvoorspelbare afhankelijkheid op de PC's van de anderen.

De reden om HEVC te willen was betere kwaliteit per bit — maar bij 1 Gbit symmetrisch
zijn bits gratis. Een codec die misschien niet werkt op de PC van je vriend is een veel
groter probleem dan een bitrate die niemand merkt.

HEVC blijft instelbaar (`codec = "hevc"` in de config) voor wie weet dat beide kanten
de uitbreiding hebben.

### Bitrate

De redenering hierboven ("bij 1 Gbit zijn bits gratis") klopt nog steeds **voor het
tailnet zelf** — die blijft de reden dat er geen volwaardige congestion control nodig
is. Ze zei alleen niets over wat er gebeurt als de bits bij een peer aankomen die zelf
geen 1 Gbit heeft.

Sinds fase 10 is de standaardbitrate **12 Mbit/s** in plaats van de oorspronkelijke
~25 Mbit/s. Rick heeft gemeten dat de 25 Mbit-stream bij een kijkende peer met een
mindere eigen internetverbinding lag veroorzaakte in de audio van **degene die
streamt** (niet bij de kijker zelf) — een regressie op de belangrijkste eis van dit
document ("geen merkbare impact op gamen/voice"). Bij 12 Mbit viel die lag weg zonder
merkbaar kwaliteitsverlies. Het precieze mechanisme (vermoedelijk audio die achter
dezelfde verbinding of CPU wacht als de videostream) is niet verder uitgezocht, alleen
het symptoom en de oplossing zijn bevestigd.

**Dit is geen vergissing die teruggedraaid moet worden** als er ooit weer met hogere
bitrates geëxperimenteerd wordt — het is een gemeten regressie, geen esthetische keuze.
Voor 1440p en 3440×1440 geldt dezelfde waarschuwing: niet zomaar omhoog schalen naar
"wat gebruikelijk is voor die resolutie" zonder opnieuw te meten bij een peer met een
matige verbinding.

## In scope
1. P2P-netwerklaag over het tailnet, geen signaling-server.
2. Voice chat, full mesh, lokaal gemixt.
3. Screenshare 1080p60, hardware-encoded, lage latency.
4. Meerdere bronnen tegelijk delen (monitor + vensters) en meerdere streams tegelijk bekijken.
5. Tekstchat met volledige geschiedenis-inhaal na offline zijn.
6. Bestanden delen tussen de peers, met hervatten na onderbreking en hash-verificatie.
7. Directe berichten (DM's) naast het algemene kanaal: een gesprek tussen twee peers dat
   de derde nooit te zien krijgt. Zie `docs/ARCHITECTURE.md`, sectie "Kanalen", voor het
   ontwerp en de trade-off die daarbij hoort.

## Buiten scope (backlog, zie `TODO.md`)
- Remote input control (Moonlight-stijl).
- Reacties, replies, afbeeldingen plakken in chat.

## Vastgelegde beslissingen

### Netwerk
- **N-agnostische** code, 3 peers in config. Geen hardcoded 3.
- Identiteit: random UUID bij eerste start, lokaal opgeslagen + door gebruiker gekozen
  displaynaam. UUID = auteursidentiteit in de chat, naam is cosmetisch en wijzigbaar.
- Transport: **QUIC (quinn)** voor control + chat + toekomstige file transfer.
  **Plain UDP** voor audio/video.
- Peers adresseren via tailnet-IP of MagicDNS-naam, configureerbaar.
- Vaste default-poort, overschrijfbaar (nodig voor meerdere instanties op één PC).
- Protocolversie in de handshake; mismatch geeft nette melding, geen crash.
- Alleen verbindingen van geconfigureerde peer-UUID's worden geaccepteerd.

### Screenshare
- Capture: **Windows.Graphics.Capture** (monitor én venster, cursor-toggle, rand uitschakelbaar).
- Codec: **H.264 4:2:0 8-bit** default, 60 fps, ~12 Mbit (zie "Bitrate" hierboven), in
  de UI aan te passen. Resolutie volgt de bron (monitor of venster) en is niet
  vastgelegd op 1080p. HEVC staat aan als optie maar hangt op Windows af van een
  Store-uitbreiding om te decoderen; zie `docs/OVERDRACHT.md`.
  4:4:4 (scherpere tekst) is **geen haalbare uitbreiding, niet alleen uitgesteld**: geen
  enkele Turing-GPU — ook de RTX 2080 Super niet — kan H.264- of HEVC-4:4:4
  hardwarematig decoderen. Zie `docs/OVERDRACHT.md` en `TODO.md`.
- Fan-out: **subscribe-on-demand**. De deler kondigt een bron aan; encoden start pas
  als minstens één peer kijkt. Eén encoder-sessie, dezelfde encoded stream naar alle kijkers.
- Alle drie mogen tegelijk delen; meerdere bronnen per persoon toegestaan.
- Weergave: **pop-out venster per stream** (eigen D3D11 swapchain), maximaliseerbaar,
  plús een overzichtstrook met verkleinde live beelden in het hoofdvenster (fase 5) zodat
  je niet tussen losse vensters hoeft te zoeken. Met één 1080p-monitor per persoon is een
  maximaliseerbaar los venster voor het echte kijken de betere UX.
- Bitrate vast met simpele loss/RTT-feedback. Geen volwaardige congestion control.
- **Desktop-audio** gaat mee als aparte stream met eigen volumeslider per luisteraar.

### Voice
- Discord-model: expliciet join/leave. Niet altijd-aan (eis: laag verbruik in rust).
- Opus, 48 kHz, 20 ms frames.
- **Geen AEC** — headset is verplicht. Wel noise suppression via `nnnoiseless` (RNNoise).
- Open mic met VAD. Push-to-talk niet in v1, wel voorzien in het ontwerp.
- Per-deelnemer volume, mute, deafen.

### Chat
- Datamodel vanaf dag één: `Post`, `Edit`, `Delete`, `SetNick`. UI in v1: versturen,
  eigen bericht bewerken/verwijderen, markdown-codeblokken renderen.
- Reacties/replies/afbeeldingen zijn later nieuwe op-kinds — geen migratie nodig.
- Alles voor altijd bewaren.
- Windows toast + geluid bij achtergrond, tray-icoon, minimaliseren naar tray.
  Autostart met Windows staat standaard uit. Het is een regel in `config.toml` geworden
  in plaats van een vinkje in de UI: er is nog geen instellingenscherm, en je bewerkt
  dat bestand toch al voor de peer-adressen.
- Generieke append-only oplog, niet chat-specifiek, zodat nicknames/settings/file-metadata
  later over hetzelfde sync-mechanisme kunnen.

### Bestandsdeling
- Aanbieden is een gewone oplog-op (`OpKind::FileMeta`), niet chat-specifiek: hetzelfde
  version-vector-mechanisme als tekstchat, dus ook een aanbod terugvinden na lang offline
  zijn kost geen aparte inhaalslag.
- Downloaden is **punt-naar-punt** met de aanbieder, over een eigen QUIC-stream naast de
  control-stream — een bestand mag chat of screenshare-signalering nooit laten wachten.
- Hervatten na onderbreking: de aanvrager meldt hoeveel hij al heeft, de aanbieder seekt
  zijn bronbestand daarnaartoe. Geen chunk-niveau bevestiging nodig — de stream zelf is al
  betrouwbaar en geordend.
- Verificatie: BLAKE3-hash over het hele bestand na afloop. Klopt hij niet, dan wordt het
  weggegooid; een volgende poging begint vanzelf weer bij 0.
- Bestanden landen in een vaste, instelbare downloadmap. Geen locatiedialoog per download.
- Zie `docs/ARCHITECTURE.md` (sectie "Bestandsdeling") voor het volledige ontwerp.

### Opslag en distributie
- Portable: SQLite + config naast de exe, fallback `%APPDATA%`.
- Distributie: zip met exe, handmatig naar de andere twee.

### Proces
- Techstack: **Rust-kern + Tauri v2 (WebView2) voor de weergave.** Oorspronkelijk
  egui/eframe; op 2026-08-04 omgedraaid omdat het ontwerpplafond van egui te laag bleek.
  De migratie is nog niet uitgevoerd. De vijf niet-UI-crates (`proto`, `store`, `net`,
  `audio`, `video`) blijven onaangeroerd — die bevatten geen egui-aanroepen. Volledige
  onderbouwing en de kosten (WebView2 als Windows-component, tweede taal, nieuw transport
  voor de miniaturenstrook) staan in `PRODUCT.md`, sectie `## Stack`.
- Kleine commits per afgeronde stap, direct op `main`.
- Tests waar ze het werk aantoonbaar sneller/veiliger maken (protocol, sync, jitterbuffer).
  Geen testdekking als doel op zich. Media handmatig testen met peer 2.
