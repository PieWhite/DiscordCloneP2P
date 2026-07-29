# ROADMAP

Fasen worden op volgorde afgemaakt. Elke fase eindigt in iets dat draait en te testen is
met een echte tweede peer, niet in een half werkend tussenproduct.

## Fase 0 — Scaffolding ✅
Cargo workspace met de crates uit `ARCHITECTURE.md`, config-laden/opslaan,
logging (`tracing` naar bestand + console), leeg eframe-venster.
**Klaar.**

## Fase 1 — Netwerklaag ✅
QUIC-mesh over het tailnet. Handshake met protocolversiecheck, UUID-allowlist,
auto-reconnect met exponentiële backoff, RTT-meting, peerstatus in de UI.
**Klaar:** bevestigd tussen twee echte PC's over het tailnet.

## Fase 2 — Tekstchat ✅
SQLite-oplog, version-vector sync, `OpBroadcast`, chat-UI met codeblokken,
eigen berichten bewerken/verwijderen, tray + toast-notificaties.
**Klaar:** getest met drie lokale instanties en met een echte peer over het tailnet.
Een peer die later opstart haalt de geschiedenis vanzelf in; alles overleeft herstart.

Eén ding week af van het oorspronkelijke plan: de chat- en sync-lus draait op de
tokio-runtime, niet in `update()` van de UI. egui tekent niet als het venster verborgen
of geminimaliseerd is, en dan zou de synchronisatie stilvallen op precies het moment dat
je een melding zou willen krijgen. De UI is nu een pure weergave van een momentopname.

## Fase 3 — Voice ✅
WASAPI-capture via `cpal`, `nnnoiseless` ruisonderdrukking + VAD, Opus over UDP,
adaptieve jitterbuffer per spreker, lokale mix, per-deelnemer volume, mute/deafen,
expliciet join/leave.
**Klaar:** de keten van coderen tot decoderen is getest inclusief pakketverlies en
herordening; de apparaatlaag is met een rooktest op echte hardware gecontroleerd.
Of het *klinkt* moet met een tweede machine beoordeeld worden — dat kan geen test.

Geen echo-onderdrukking, conform de afspraak dat iedereen een headset gebruikt. Dat
scheelde een C++-bouwafhankelijkheid (WebRTC APM).

## Fase 4 — Screenshare, eerste versie ✅
WGC-capture van één monitor, H.264-encode via Media Foundation, UDP-fragmentatie,
decode, D3D11-render in een pop-out venster, subscribe-on-demand, desktop-audio
als aparte stream met eigen volume.

**Af:** de hele beeldketen. Aankondigen, intekenen, opnemen, coderen, versturen,
samenstellen, decoderen, tonen — plus het streambeheer eromheen en de knoppen in de UI.
Op deze machine gemeten: 1080p op 55-56 beelden per seconde, geen enkel beeld onderweg
kwijt, scherp leesbare tekst.

**Desktop-audio** gaat mee als eigen stream over de voice-verbinding, met bij de
luisteraar een volumeschuif los van de stem. Gevolg: je moet in het gesprek zitten om
mee te luisteren of mee te sturen. De afweging staat in `docs/OVERDRACHT.md`.

**Meetpunt: gedaan.** 3,1 ms tussen opnemen en tonen, in een debug-build. Dat is ver
onder alles wat opvalt, dus de encoder hoeft niet naar directe NVENC. Wat de monitor er
zelf nog bij optelt zit daar niet in en is met software ook niet te meten.

Anders dan gepland: **H.264 in plaats van HEVC** (decoderen van HEVC hangt op Windows aan
een Store-uitbreiding die er niet standaard op zit), en het kijkvenster heeft **wel een
rand** — een randloos venster kun je niet verplaatsen of sluiten zonder dat zelf na te
bouwen. Beeldvullend zit op F11. Beide staan onderbouwd in `docs/OVERDRACHT.md`.

**Klaar als:** peer B ziet peer A's scherm op 1080p60, tekst is scherp leesbaar, en
peer A merkt geen framedrops in een draaiende game. *Dat laatste kan alleen met een
tweede machine; zie `docs/TESTPLAN.md`.*

## Fase 5 — Screenshare uitbreiding
Venster-capture, meerdere bronnen tegelijk delen, meerdere inkomende streams tegelijk
bekijken, grid-weergave in het hoofdvenster, kwaliteitsinstellingen in de UI,
optionele 4:4:4-modus voor extra scherpe tekst.

## Fase 6 — Backlog
Zie `TODO.md`. Niet nu bouwen.
