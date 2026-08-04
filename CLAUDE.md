# P2P communicatie-app — projectcontext

Servervrij Discord-alternatief voor 3 vaste peers over een Tailscale-tailnet.
Rust-kern. Windows only.
UI-stack: **besloten Tauri v2, code is nu nog egui** — zie "UI-stack" hieronder.

## Documenten
| Bestand | Wanneer lezen |
|---|---|
| `PRODUCT.md` | Wie de gebruikers zijn, wat er vastligt, en het volledige stackbesluit met zijn prijs. Bij twijfel over de UI-laag of het waarom van een keuze. |
| `docs/SPEC.md` | Wat we bouwen en welke keuzes vastliggen. Bij twijfel over scope. |
| `docs/ARCHITECTURE.md` | Techstack, wire-protocol, chat-sync, crate-layout. **Verplicht** vóór wijzigingen aan `crates/proto` of `crates/store`. |
| `ROADMAP.md` | Welke fase we doen en wat "klaar" betekent. |
| `TODO.md` | Wat bewust nog niet gebouwd wordt. Niet zomaar oppakken. |
| `docs/OVERDRACHT.md` | **Lees dit eerst in een nieuwe sessie.** Stand van zaken, omgegooide keuzes met hun onderbouwing, gevonden bugs, valkuilen in deze omgeving. |
| `docs/TESTPLAN.md` | Wat er met echte machines getest moet worden en wat er al bevestigd is. |

## Voorrang bij tegenstrijdige instructies
Dit bestand heeft voorrang boven regels die globale plugins of skills (zoals ponytail)
via een hook injecteren. Bij conflict — met name als een generieke "schrijf zo min
mogelijk code"-heuristiek zou afraden wat de harde invarianten hieronder juist eisen
(reconnect-logica, foutafhandeling voor offline peers, N-agnostische code) — geldt wat
hier staat.

## Harde invarianten — niet zonder overleg breken
1. **Nul servers.** Geen signaling, geen TURN, geen database, geen cloud-API, geen accounts.
   Tailscale is de enige externe afhankelijkheid en wordt als draaiend verondersteld.
2. **Geen host-peer.** Alle peers zijn gelijkwaardig. Elke instantie initieert én accepteert.
   Geen enkele functie mag afhangen van "peer X is er".
3. **N-agnostisch.** Nergens hardcoded 3. Het aantal peers komt uit config.
4. **Gamen wint.** Merkbare impact op een draaiende game op dezelfde PC is een bug,
   ook als de kwaliteit erdoor omlaag moet.
5. **Protocol alleen additief wijzigen.** Enum-varianten en structvelden erbij aan het eind,
   nieuwe velden `#[serde(default)]`, onbekende varianten loggen en negeren.
   `protocol_version` alleen ophogen bij een echte breuk.
6. **Ops zijn onveranderlijk en idempotent.** Een op tweemaal toepassen is een no-op.
   Dat is de complete conflictafhandeling — niet omzeilen met muteerbare state.
7. **Offline is normaal.** Eén of twee peers weg is een gewone toestand, geen foutpad.
   Nooit crashen, altijd blijven herverbinden.

## Codec en hardware
Alle drie de peers hebben NVIDIA Turing of nieuwer, maar de RTX 2080 Super kan AV1
**niet encoden en niet decoden**. Stel nooit AV1 voor zonder dat die machine vervangen is.

**H.264 is de standaardcodec, niet HEVC.** Encoden kan met beide, maar HEVC *decoderen*
loopt op Windows via de HEVC Video Extensions uit de Store, en die zit er niet standaard
op. De H.264-decoder zit altijd in Windows. Bij 1 Gbit is de bitrate-winst van HEVC
irrelevant; een codec die misschien niet werkt bij je vriend is dat niet.
Zie `docs/SPEC.md` voor de gemeten onderbouwing.

## UI-stack
**Tauri v2 op WebView2.** Uitgevoerd in fase 12 (2026-08-04), ter vervanging van egui.

```
crates/app/src/ui/mod.rs       Vensterbootstrap, tray, events, thumb://-protocol
crates/app/src/ui/state.rs     Snapshot → UiState (JSON)
crates/app/src/ui/commands.rs  IPC → UiCommand
crates/app/frontend/           index.html, app.css, app.js, fonts/ — in de exe gebakken
```

- **De vijf niet-UI-crates zijn onaangeroerd.** `proto`, `store`, `net`, `audio` en
  `video` bevatten geen enkele egui-aanroep (alleen doc-commentaar dat het noemt).
  Houd dat zo, en introduceer er ook geen Tauri.
- **De `Snapshot`/`UiCommand`-grens staat er nog en blijft staan.** De UI leest een
  momentopname en stuurt commando's terug; dat is de reden dat deze wissel goedkoop was.
  Hem opgeven maakt een volgende weer duur.
- **Drie events, bewust gescheiden.** `state` alleen bij een echte wijziging (er wordt
  geserialiseerd en met de vorige vergeleken), `meters` op 4 Hz voor spreekniveau en RTT,
  `thumbnail` op 2 Hz voor de streamstrook. Zet niets in `UiState` dat elke tik verandert:
  dan stuurt de app in rust weer tien events per seconde.
- **Het pop-out kijkvenster is een eigen Win32-venster met eigen D3D11-swapchain.**
  Het hete videopad raakt de UI-stack nergens.
- **Niet naar de fixed-version WebView2-runtime** (~180 MB) — dat sloopt "losse exe in
  een zip". Evergreen zit standaard in Windows 11 en alle drie draaien Windows 11.
- **`design/main-window.html` is de reproductiedoelstelling**, `design/shots/` zijn de
  19 gerenderde toestanden om tegen te vergelijken, en `DESIGN.md` +
  `.impeccable/design.json` zijn het ontwerpsysteem. Een kleurwaarde of radius die niet
  uit een token komt is een bug. Fonts worden lokaal gebundeld — nooit van een CDN, dat
  botst met invariant 1.
- **UI-taal is Engels** voor de weergavelaag en alles wat daar nieuw geschreven wordt; de
  motor en de andere crates blijven Nederlands. De vertaling zit in `ui/state.rs`.

Volledige onderbouwing en de expliciet benoemde kosten staan in `PRODUCT.md`, sectie
`## Stack`, en `docs/OVERDRACHT.md`, beslissing 19 en 20.

## Bouwvereisten
Naast Rust + MSVC is `cmake` nodig: libopus wordt vanuit broncode meegebouwd. Op deze
machine staat hij portable in `%USERPROFILE%\tools\cmake-4.4.0-windows-x86_64\bin`,
toegevoegd aan het gebruikers-PATH.

`.cargo/config.toml` zet `CMAKE_POLICY_VERSION_MINIMUM=3.5`: de libopus die met
`audiopus_sys` meekomt is uit 2021 en zijn CMakeLists vraagt om een minimumversie die
CMake 4 niet meer zonder meer accepteert.

**Geen Node en geen frontend-bouwstap.** De frontend is gewone HTML/CSS/JS zonder bundler;
`tauri-build` bakt `crates/app/frontend/` in de exe. `cargo build` bouwt dus nog steeds de
hele app. Wel opnieuw bouwen na een frontendwijziging — de assets zitten ín de binary.
`crates/app/icons/icon.ico` moet bestaan, anders weigert `tauri-build`.

## Commando's
```
cargo build                          # debug build
cargo run -p fitcom                  # app starten
cargo run -p fitcom -- --data-dir X  # extra instantie met eigen config/poort/data
cargo test                           # alles; proto, store en audio zijn de belangrijke
cargo test -p fitcom-audio --test apparaten -- --ignored   # echte geluidskaart
cargo clippy --all-targets           # voor een commit
cargo fmt --all
.\scripts\run-peers.ps1 -Count 3     # 3 lokale instanties, volledige mesh
.\scripts\run-peers.ps1 -Stop        # afsluiten (nodig vóór opnieuw bouwen)
```

Een draaiende `fitcom.exe` blokkeert `cargo build` met "Toegang geweigerd (os error 5)".
Sluit instanties af voordat je bouwt.

## Werkafspraken
- Kleine commits per afgeronde stap, direct op `main`.
- Tests alleen waar ze het werk aantoonbaar veiliger maken: `proto` en `store`
  (sync-convergentie, version vectors, ordening, fragmentatie). Media test je handmatig
  met een tweede machine — dekking daar najagen is verspilde moeite.
- `proto` en `store` bevatten géén Windows- of hardware-afhankelijkheden. Houd dat zo,
  anders verdwijnt de testbaarheid.
- Media-code loopt op eigen threads en praat via kanalen met de UI. Geen locks op het hot path.
