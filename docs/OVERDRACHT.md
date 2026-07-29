# Overdracht — stand van zaken

Bedoeld om in een nieuwe sessie snel weer op snelheid te komen. Wat er staat, waarom
het zo staat, waar ik tegenaan gelopen ben, en wat er nog moet.

Laatst bijgewerkt: 2026-07-29, na fase 4 deel 1.

---

## Status per fase

| Fase | Status | Bewezen door |
|---|---|---|
| 0 — Scaffolding | ✅ af | draait |
| 1 — Netwerklaag | ✅ af | **echt getest tussen twee PC's over Tailscale** |
| 2 — Tekstchat | ✅ af, nog niet met een echte peer getest | 19 unit/integratietests, 3 lokale instanties |
| 3 — Voice | ✅ af, nog niet met een echte peer getest | ketentests + rooktest op echte geluidskaart |
| 4 — Screenshare | 🟡 **deels**: capture en encoder werken, de rest niet | tests op echte GPU |

Wat nog open staat in fase 4 staat onderaan onder "Wat er nog moet".

---

## Waar wat staat

```
crates/proto/   Wire-protocol. Puur, geen I/O. ControlMsg, Op, VersionVector, mediaheader.
crates/store/   SQLite-oplog, timeline-opbouw, sync-berekening. Puur, geen Windows.
crates/net/     QUIC-mesh (async, tokio) + MediaSocket voor UDP (blokkerend, geen tokio).
crates/audio/   Voice: jitterbuffer, mixer, Opus, cpal-sessie.
crates/video/   Screenshare: fragmentatie, D3D11, WGC-capture, MF-encoder.
crates/app/     lib + binary. engine.rs is de motor; ui.rs is een pure weergave.
```

De belangrijkste structurele regel: **`proto` en `store` bevatten geen Windows- of
hardware-afhankelijkheden.** Daar zit de subtiele logica, en die moet testbaar blijven
zonder GPU of geluidskaart. Houd dat zo.

---

## Beslissingen die tijdens het bouwen zijn omgegooid

Deze staan hier omdat ze tegen de oorspronkelijke afspraak in gaan. Ze zijn allemaal
gebaseerd op een meting, niet op een voorkeur.

### 1. H.264 in plaats van HEVC (fase 4)
De SPEC zei HEVC. Gemeten op de dev-PC:

- HEVC **encoden**: `NVIDIA HEVC Encoder MFT` — aanwezig.
- HEVC **decoderen**: alleen `HEVCVideoExtension`, een Store-uitbreiding die niet
  standaard op Windows zit.
- H.264 decoderen: `Microsoft H264 Video Decoder MFT` — zit altijd in Windows.

De reden om HEVC te willen was kwaliteit per bit, maar bij 1 Gbit zijn bits gratis.
Een codec die misschien niet werkt op de PC van een van de anderen is een veel groter
risico. HEVC blijft instelbaar voor wie weet dat beide kanten hem hebben.

### 2. Geen kleurconversie in het videopad (fase 4)
Ik ging ervan uit dat de encoder NV12 wil en dat we dus BGRA→NV12 moesten omzetten op
de GPU. Gemeten: de encoders accepteren `ARGB32` rechtstreeks — precies wat de
schermopname levert. Die hele stap is geschrapt.

### 3. De motor los van de UI (fase 2)
Chat en synchronisatie zaten in `update()` van egui. egui tekent geen frames zolang het
venster verborgen of geminimaliseerd is, dus de synchronisatie viel stil op precies het
moment waarop je een melding wilt: tijdens het gamen, app op de achtergrond. De motor
draait nu op de tokio-runtime; de UI leest een `watch`-momentopname en mag stilvallen.

Om dezelfde reden leest de tray-thread zijn eigen gebeurtenissen. Zat dat in de UI, dan
kon je een verborgen venster nooit meer terughalen.

### 4. Geen echo-onderdrukking (fase 3)
Conform de afspraak dat iedereen een headset draagt. Dat scheelde WebRTC APM, een
C++-bouwafhankelijkheid. Gebruikt iemand luidsprekers, dan horen de anderen zichzelf.

---

## Bugs die de tests eruit haalden

Allemaal dingen die met handmatig testen niet betrouwbaar te vinden waren.

**Botsende QUIC-verbindingen (fase 1).** Twee peers die tegelijk starten dialen elkaar
tegelijk. Ik vergeleek de nieuwe verbinding met de bestaande — dat hangt af van
aankomstvolgorde, dus bij een bepaalde timing hield A verbinding B→A en B verbinding
A→B, sloten ze elkaars keuze en bleef er niets over. De winnaar moet **absoluut**
bepaald zijn: de verbinding opgezet door de peer met het laagste `PeerId`.

**Koppelen op alleen het IP (fase 1).** Drie instanties op één PC delen hetzelfde
loopback-adres, dus inkomende verbindingen werden aan de verkeerde peer toegewezen en
de identiteitscontrole sloeg ten onrechte alarm. Nu op IP én poort, met IP-only als
ondubbelzinnige terugval. Gevonden door pas bij drie peers te testen, niet bij twee.

**Version vector die loog (fase 2).** Zie `docs/ARCHITECTURE.md`. Ops komen niet altijd
op volgorde binnen; meldde de version vector het maximum in plaats van de hoogste
*aaneengesloten* seq, dan claimden we ops te hebben die we misten en kregen we ze nooit
meer.

**Stack overflow in de jitterbuffer (fase 3).** De overloop-tak riep zichzelf aan zonder
de buffer te verkleinen. Op de audio-thread is dat geen hapering maar een crash.

**Encoder stond stil na één frame (fase 4).** Hardware-MFT's zijn asynchroon en melden
via gebeurtenissen wanneer ze invoer willen. Mijn lus wachtte daar alleen de eerste keer
blokkerend op en polste daarna; alle volgende `NeedInput`-gebeurtenissen werden gemist.
Nu: blokkerend wachten tot hij om invoer vraagt, daarna alleen polsen naar uitvoer.

**Logbestand bleef leeg (fase 2).** `tracing_appender::non_blocking` buffert tot het
proces netjes eindigt — precies verkeerd voor het bestand dat je opvraagt als er iets
crasht. Schrijft nu direct.

**Testhelper deelde poorten dubbel uit (fase 2/3).** Parallelle tests binnen één binary
kregen soms hetzelfde poortnummer van het besturingssysteem. Gaf ongeveer één op de vijf
runs een onverklaarbare fout. De helper onthoudt nu wat hij uitgedeeld heeft.

---

## Valkuilen in deze omgeving

- **Een draaiende `fitcom.exe` blokkeert `cargo build`** met "Toegang geweigerd
  (os error 5)". Altijd eerst `.\scripts\run-peers.ps1 -Stop`.
- **`cmake` is nodig om te bouwen** (libopus). Staat portable in
  `%USERPROFILE%\tools\cmake-4.4.0-windows-x86_64\bin`, op het gebruikers-PATH.
  `.cargo/config.toml` zet `CMAKE_POLICY_VERSION_MINIMUM=3.5` omdat de meegeleverde
  libopus uit 2021 is en CMake 4 zijn `CMakeLists` anders weigert.
- **Tests die echte hardware nodig hebben staan op `#[ignore]`.** Draai ze met de hand:
  ```
  cargo test -p fitcom-audio --test apparaten -- --ignored --nocapture
  cargo test -p fitcom-video --lib -- --ignored --nocapture --test-threads=1
  ```
- **De `windows` crate is versiegevoelig.** In 0.62 zit `BOOL` in `windows::core`,
  niet in `Win32::Foundation`; `VARIANT` vereist zowel `Win32_System_Com` als
  `Win32_System_Ole`. Bij een foutmelding "found an item that was configured out" ontbreekt
  er een feature, niet een import.
- **De dev-PC heeft twee monitoren** (2560×1440 hoofdscherm + 1920×1080), terwijl de
  SPEC uitgaat van één 1080p-scherm per persoon. Multi-monitor is dus relevanter dan
  gedacht.

---

## Wat er nog moet in fase 4

De volgorde hieronder is ook de aanbevolen bouwvolgorde: elke stap is los te verifiëren.

### 1. Decoder (`crates/video/src/codec.rs`)
Spiegelbeeld van `Encoder`. Aandachtspunten:
- Zoeken met `zoek_transform_met(false, ...)` en **alle** vlaggen, niet alleen
  `HARDWARE` — de H.264-decoder van Windows is geen hardware-MFT.
- Software-MFT's zijn synchroon: dan geen gebeurtenislus maar gewoon `ProcessInput`
  gevolgd door `ProcessOutput` tot `MF_E_TRANSFORM_NEED_MORE_INPUT`. Controleer met
  `MFT_ENUM_FLAG_ASYNCMFT` welke van de twee je hebt en ondersteun beide.
- Uitvoer is NV12. Voor weergave moet dat naar BGRA: `ID3D11VideoProcessor`
  (`VideoProcessorBlt`) doet dat op de GPU.
- **Verificatie:** roundtrip-test. Encodeer een textuur met bekende inhoud, decodeer,
  controleer afmetingen en dat er beeld uitkomt. Dat kan zonder iets te zien.

### 2. Weergavevenster (`crates/video/src/venster.rs`)
Borderless Win32-venster met eigen DXGI-swapchain, op een eigen thread met eigen message
pump. Zie `docs/ARCHITECTURE.md` → "Waarom video in een apart venster": eframe rendert via
wgpu/DX12 en een D3D11-textuur daarin krijgen vereist fragiele `wgpu-hal`-interop.
Alleen visueel te verifiëren.

### 3. Streambeheer (`crates/app/src/engine.rs`)
De protocolberichten bestaan al en zijn ongebruikt: `StreamAnnounce`, `StreamRevoke`,
`StreamSubscribe`, `StreamUnsubscribe`, `StreamStats`, `RequestKeyframe`.

- Deler kondigt een bron aan met `StreamAnnounce`.
- **Encoden start pas bij de eerste `StreamSubscribe`** en stopt bij de laatste
  `StreamUnsubscribe`. Dat is de reden dat dit niets kost als niemand kijkt.
- Bij `RequestKeyframe` → `Encoder::vraag_keyframe()` (bestaat al).
- De ontvanger vraagt een keyframe zodra `Reassembler::incompleet` oploopt.
- `stream_id` 0 is voice; screenshare krijgt 1 en hoger, toegekend door de deler.

Dit deel is grotendeels toestandslogica en dus **testbaar zonder GPU** — trek het uit
elkaar zoals bij de chat: beslissingen in een pure module, plumbing in de motor.

### 4. Desktop-audio
WASAPI-loopback opnemen (`cpal` kan dit mogelijk niet; dan rechtstreeks via de `windows`
crate met `AUDCLNT_STREAMFLAGS_LOOPBACK`). Verder identiek aan de bestaande voice-keten:
Opus, eigen `stream_id`, bij de ontvanger een eigen volumeschuif los van de stemmen.
De mixer in `crates/audio/src/mix.rs` kan dit al aan — het is gewoon een extra bron.

### 5. Meetpunt latency
Uit de ROADMAP: meet glass-to-glass. Valt het tegen, dan de encoder omzetten naar
directe NVENC. De `Encoder` is daarvoor al een afzonderlijke module met een smalle API
(`new` + `encode` + `vraag_keyframe`), dus dat raakt de rest niet.

---

## Wat nog nooit met een echte peer getest is

Fase 1 is bevestigd tussen twee PC's over Tailscale. **Fase 2 en 3 niet.** Zie
`docs/TESTPLAN.md` voor de testgevallen die daarvoor uitgevoerd moeten worden.
