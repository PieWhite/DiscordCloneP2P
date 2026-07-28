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

- 1 monitor per persoon, 1080p @ 60 Hz.
- Iedereen gebruikt altijd een headset.
- Alle drie: 1 Gbit/s symmetrisch, Tailscale.
- Windows only.

**Codecgevolg:** alle drie zijn NVIDIA Turing of nieuwer. AV1 valt af (2080 Super kan
AV1 niet encoden én niet decoden). **HEVC (H.265)** is de grootste gemene deler voor
hardware-encode én -decode. H.264 als fallback.

## In scope
1. P2P-netwerklaag over het tailnet, geen signaling-server.
2. Voice chat, full mesh, lokaal gemixt.
3. Screenshare 1080p60, hardware-encoded, lage latency.
4. Meerdere bronnen tegelijk delen (monitor + vensters) en meerdere streams tegelijk bekijken.
5. Tekstchat met volledige geschiedenis-inhaal na offline zijn.

## Buiten scope (backlog, zie `TODO.md`)
- File sharing — architectuur moet dit zonder herontwerp kunnen opnemen.
- Remote input control (Moonlight-stijl).
- Reacties, replies, afbeeldingen plakken in chat.
- Meer dan één chatkanaal.

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
- Codec: **HEVC 4:2:0 8-bit**, 1080p60, ~25 Mbit default. H.264 fallback.
  4:4:4 (scherpere tekst) is een latere optionele toggle — alle drie ondersteunen het,
  maar het is een minder betreden codepad, dus niet in v1.
- Fan-out: **subscribe-on-demand**. De deler kondigt een bron aan; encoden start pas
  als minstens één peer kijkt. Eén encoder-sessie, dezelfde encoded stream naar alle kijkers.
- Alle drie mogen tegelijk delen; meerdere bronnen per persoon toegestaan.
- Weergave: **pop-out venster per stream** (eigen D3D11 swapchain), maximaliseerbaar.
  Grid-in-hoofdvenster komt in fase 5. Met één 1080p-monitor per persoon is een
  maximaliseerbaar los venster de betere UX.
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
  Autostart met Windows als optioneel vinkje, standaard uit.
- Generieke append-only oplog, niet chat-specifiek, zodat nicknames/settings/file-metadata
  later over hetzelfde sync-mechanisme kunnen.

### Opslag en distributie
- Portable: SQLite + config naast de exe, fallback `%APPDATA%`.
- Distributie: zip met exe, handmatig naar de andere twee.

### Proces
- Techstack: **Rust + egui/eframe**.
- Kleine commits per afgeronde stap, direct op `main`.
- Tests waar ze het werk aantoonbaar sneller/veiliger maken (protocol, sync, jitterbuffer).
  Geen testdekking als doel op zich. Media handmatig testen met peer 2.
