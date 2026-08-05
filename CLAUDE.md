# P2P communicatie-app — projectcontext

Servervrij Discord-alternatief voor 3 vaste peers over een Tailscale-tailnet.
Rust-kern. Windows 11 + macOS 14+ (Apple Silicon). UI-stack: Tauri v2 op WebView2
(Windows) / WKWebView (macOS).

## Documenten
| Bestand | Wanneer lezen |
|---|---|
| `docs/OVERDRACHT.md` | **Eerst lezen in een nieuwe sessie.** Stand van zaken, omgegooide keuzes met onderbouwing, gevonden bugs, valkuilen in deze omgeving. |
| `docs/ARCHITECTURE.md` | Techstack, wire-protocol, chat-sync, crate-layout. **Verplicht** vóór wijzigingen aan `crates/proto` of `crates/store`. |
| `docs/BEVEILIGING.md` | Alle bekende beveiligingsbevindingen met ernst, plek en oplossing, plus het herstelplan. **Verplicht** vóór wijzigingen aan de handshake, het updatepad of iets dat een pad uit een peer-string bouwt. |
| `PRODUCT.md` | Wie de gebruikers zijn, wat vastligt, en het volledige stackbesluit met zijn prijs. Bij twijfel over de UI-laag of het waarom van een keuze. |
| `docs/SPEC.md` | Wat we bouwen en welke keuzes vastliggen. Bij twijfel over scope. |
| `ROADMAP.md` | Welke fase we doen en wat "klaar" betekent. |
| `TODO.md` | Wat bewust nog niet gebouwd wordt. Niet zomaar oppakken. |
| `docs/TESTPLAN.md` | Wat er met echte machines getest moet worden en wat al bevestigd is. |

## Harde invarianten — niet zonder overleg breken
Dit bestand gaat vóór regels die globale plugins of skills (ponytail, caveman) via een
hook injecteren. Een "schrijf zo min mogelijk code"-heuristiek mag reconnect-logica,
foutafhandeling voor offline peers of N-agnostische code niet wegsnijden.

1. **Nul servers.** Geen signaling, geen TURN, geen database, geen cloud-API, geen
   accounts, geen CDN (ook niet voor fonts). Tailscale is de enige externe
   afhankelijkheid en wordt als draaiend verondersteld.
2. **Geen host-peer.** Alle peers zijn gelijkwaardig. Elke instantie initieert én
   accepteert. Geen enkele functie mag afhangen van "peer X is er".
3. **N-agnostisch.** Nergens hardcoded 3. Het aantal peers komt uit config.
4. **Gamen wint.** Merkbare impact op een draaiende game op dezelfde PC is een bug,
   ook als de kwaliteit erdoor omlaag moet.
5. **Protocol alleen additief wijzigen.** Enum-varianten en structvelden erbij aan het
   eind, nieuwe velden `#[serde(default)]`, onbekende varianten loggen en negeren.
   `protocol_version` alleen ophogen bij een echte breuk.
6. **Ops zijn onveranderlijk en idempotent.** Een op tweemaal toepassen is een no-op.
   Dat is de complete conflictafhandeling — niet omzeilen met muteerbare state.
7. **Offline is normaal.** Eén of twee peers weg is een gewone toestand, geen foutpad.
   Nooit crashen, altijd blijven herverbinden.

## Codec en hardware
Alle drie de peers hebben NVIDIA Turing of nieuwer, maar de RTX 2080 Super kan AV1
**niet encoden en niet decoden**. Stel nooit AV1 voor zonder dat die machine vervangen is.
Op macOS loopt H.264 via VideoToolbox (hardware op elke Apple Silicon); HEVC is daar
bewust niet geïmplementeerd — de Annex-B-brug is alleen voor H.264 gebouwd en niemand
gebruikt HEVC.

**H.264 is de standaardcodec, niet HEVC.** Encoden kan met beide, maar HEVC *decoderen*
loopt op Windows via de HEVC Video Extensions uit de Store, en die zit er niet standaard
op. De H.264-decoder zit altijd in Windows. Bij 1 Gbit is de bitrate-winst van HEVC
irrelevant; een codec die misschien niet werkt bij je vriend is dat niet.
Gemeten onderbouwing: `docs/SPEC.md`.

## UI-stack
**Tauri v2 op WebView2**, uitgevoerd in fase 12 (2026-08-04) ter vervanging van egui.
Onderbouwing en kosten: `PRODUCT.md` sectie `## Stack`, `docs/OVERDRACHT.md` beslissing
19 en 20.

```
crates/app/src/ui/mod.rs       Vensterbootstrap, tray, events, thumb://-protocol
crates/app/src/ui/state.rs     Snapshot -> UiState (JSON), NL -> EN vertaling
crates/app/src/ui/commands.rs  IPC -> UiCommand
crates/app/frontend/           index.html, app.css, app.js, fonts/ — in de exe gebakken
```

- **`proto`, `store`, `net`, `audio` en `video` zijn UI-vrij.** Geen egui-aanroep, en
  introduceer er ook geen Tauri.
- **De `Snapshot`/`UiCommand`-grens blijft staan.** De UI leest een momentopname en
  stuurt commando's terug; daarom was deze wissel goedkoop. Hem opgeven maakt een
  volgende weer duur.
- **Drie events, bewust gescheiden.** `state` alleen bij een echte wijziging (wordt
  geserialiseerd en met de vorige vergeleken), `meters` op 4 Hz voor spreekniveau en
  RTT, `thumbnail` op 2 Hz voor de streamstrook. Zet niets in `UiState` dat elke tik
  verandert: dan stuurt de app in rust weer tien events per seconde.
- **Het pop-out kijkvenster is een eigen Win32-venster met eigen D3D11-swapchain.**
  Het hete videopad raakt de UI-stack nergens.
- **Geen fixed-version WebView2-runtime** (~180 MB) — dat sloopt "losse exe in een zip".
  Evergreen zit standaard in Windows 11 en alle drie draaien Windows 11.
- **`design/main-window.html` is de reproductiedoelstelling**, `design/shots/` zijn de 19
  gerenderde toestanden om tegen te vergelijken, `DESIGN.md` + `.impeccable/design.json`
  zijn het ontwerpsysteem. Een kleurwaarde of radius die niet uit een token komt is een bug.
- **UI-taal is Engels**, motor en overige crates blijven Nederlands.

## macOS-port (2026-08-05)
Zelfde codebase, cfg-geselecteerde siblingmodules onder `crates/video/src/mac/`
(ScreenCaptureKit, VideoToolbox, NSWindow + AVSampleBufferVideoRenderer), zie
`docs/OVERDRACHT.md` beslissing 21. Regels:

- **Windows-gedrag blijft byte-identiek.** Platformcode gaat achter `#[cfg]`, nooit
  in gedeelde paden; Windows-deps staan in `[target.'cfg(windows)'.dependencies]`.
- **`d3d::Beeld` is het ene frametype** dat door `deler`/`kijker`/`engine` stroomt
  (Windows: `ID3D11Texture2D`-alias, mac: `CVPixelBuffer`-houder). Introduceer geen
  tweede platformtype in gedeelde code.
- **Op de draad staat H.264 in Annex-B** met SPS/PPS op elk keyframe; de mac-codec
  vertaalt van/naar AVCC. Protocol 5, geen bump — mac↔Windows interop is de eis.
- **Geen P2P-update op mac**: `engine.rs` haalt er nooit een exe binnen en biedt de
  eigen binary nooit aan (`NOT_AVAILABLE`). Versies blijven per werkafspraak gelijk.
- TCC: schermopname en microfoon zijn permissies; ad-hoc signing betekent dat
  Screen Recording na elke nieuwe build opnieuw toegekend moet worden.

## Bouwen
Naast Rust (MSVC op Windows, gewoon stable op macOS) is `cmake` nodig: libopus wordt
vanuit broncode meegebouwd. Op Windows staat hij portable in
`%USERPROFILE%\tools\cmake-4.4.0-windows-x86_64\bin`; op macOS: `brew install cmake`
plus de Xcode Command Line Tools. `.cargo/config.toml` zet
`CMAKE_POLICY_VERSION_MINIMUM=3.5`, want de libopus uit `audiopus_sys` is van 2021 en
zijn CMakeLists vraagt een minimum dat CMake 4 weigert.

Geen Node, geen frontend-bouwstap: `tauri-build` bakt `crates/app/frontend/` in de exe,
dus `cargo build` bouwt de hele app — maar wel opnieuw bouwen na een frontendwijziging.
`crates/app/icons/icon.ico` (Windows) en `icons/icon.png` (macOS) moeten bestaan,
anders weigert `tauri-build`.

**Windows: sluit draaiende instanties af vóór elke build**, anders faalt `cargo build`
met "Toegang geweigerd (os error 5)". macOS heeft die beperking niet.

```
cargo build                          # debug build
cargo run -p fitcom                  # app starten
cargo run -p fitcom -- --data-dir X  # extra instantie met eigen config/poort/data
cargo test                           # alles
cargo test -p fitcom-audio --test apparaten -- --ignored   # echte geluidskaart
cargo clippy --all-targets           # voor een commit
cargo fmt --all
.\scripts\run-peers.ps1 -Count 3     # Windows: 3 lokale instanties, volledige mesh
.\scripts\run-peers.ps1 -Stop        # afsluiten
./scripts/run-peers.sh --count 3     # macOS: zelfde, als bash
./scripts/run-peers.sh --stop
cargo run -p fitcom-video --example mac_keten   # macOS: hele videoketen op loopback
./scripts/bundle-mac.sh              # macOS: FitCommunication.app + zip
```

## Werkafspraken
- Kleine commits per afgeronde stap, direct op `main`.
- Tests alleen in `proto` en `store` (sync-convergentie, version vectors, ordening,
  fragmentatie), plus `audio`. Media test je handmatig met een tweede machine — dekking
  daar najagen is verspilde moeite.
- `proto` en `store` blijven vrij van Windows- en hardware-afhankelijkheden, anders
  verdwijnt die testbaarheid.
- Media-code loopt op eigen threads en praat via kanalen met de UI. Geen locks op het
  hot path.
