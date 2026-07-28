# ROADMAP

Fasen worden op volgorde afgemaakt. Elke fase eindigt in iets dat draait en te testen is
met een echte tweede peer, niet in een half werkend tussenproduct.

## Fase 0 — Scaffolding
Cargo workspace met de crates uit `ARCHITECTURE.md`, config-laden/opslaan,
logging (`tracing` naar bestand + console), leeg eframe-venster.
**Klaar als:** `cargo run` opent een venster en schrijft een logbestand.

## Fase 1 — Netwerklaag
QUIC-mesh over het tailnet. Handshake met protocolversiecheck, UUID-allowlist,
auto-reconnect met exponentiële backoff, RTT-meting, peerstatus in de UI.
**Klaar als:** twee PC's zien elkaar als online, één afsluiten geeft `Offline` zonder
crash, opnieuw starten geeft binnen enkele seconden weer `Online`.

## Fase 2 — Tekstchat
SQLite-oplog, version-vector sync, `OpBroadcast`, chat-UI met markdown-codeblokken,
eigen berichten bewerken/verwijderen, tray + toast-notificaties.
**Klaar als:** peer A stuurt berichten terwijl peer B uit staat; B start op en heeft
binnen een seconde de volledige geschiedenis. Idem andersom, en na herstart van beide.

## Fase 3 — Voice
WASAPI-capture, `nnnoiseless` noise suppression + VAD, Opus, UDP-transport,
jitterbuffer, lokale mix van meerdere sprekers, per-deelnemer volume, mute/deafen,
expliciet join/leave.
**Klaar als:** twee peers voeren een gesprek zonder hoorbare vertraging of drop-outs;
CPU-verbruik blijft laag; app in rust (niet in voice) doet vrijwel niets.

## Fase 4 — Screenshare, eerste versie
WGC-capture van één monitor, HEVC-encode via Media Foundation, UDP-fragmentatie,
decode, D3D11-render in een pop-out venster, subscribe-on-demand, desktop-audio
als aparte stream met eigen volume.
**Meetpunt:** glass-to-glass latency meten. Valt die tegen, dan de encoder-trait
omzetten naar directe NVENC voordat we door gaan.
**Klaar als:** peer B ziet peer A's scherm op 1080p60, tekst is scherp leesbaar, en
peer A merkt geen framedrops in een draaiende game.

## Fase 5 — Screenshare uitbreiding
Venster-capture, meerdere bronnen tegelijk delen, meerdere inkomende streams tegelijk
bekijken, grid-weergave in het hoofdvenster, kwaliteitsinstellingen in de UI,
optionele 4:4:4-modus voor extra scherpe tekst.

## Fase 6 — Backlog
Zie `TODO.md`. Niet nu bouwen.
