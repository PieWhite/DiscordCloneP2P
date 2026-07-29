# P2P communicatie-app — projectcontext

Servervrij Discord-alternatief voor 3 vaste peers over een Tailscale-tailnet.
Rust + egui. Windows only.

## Documenten
| Bestand | Wanneer lezen |
|---|---|
| `docs/SPEC.md` | Wat we bouwen en welke keuzes vastliggen. Bij twijfel over scope. |
| `docs/ARCHITECTURE.md` | Techstack, wire-protocol, chat-sync, crate-layout. **Verplicht** vóór wijzigingen aan `crates/proto` of `crates/store`. |
| `ROADMAP.md` | Welke fase we doen en wat "klaar" betekent. |
| `TODO.md` | Wat bewust nog niet gebouwd wordt. Niet zomaar oppakken. |

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

## Bouwvereisten
Naast Rust + MSVC is `cmake` nodig: libopus wordt vanuit broncode meegebouwd. Op deze
machine staat hij portable in `%USERPROFILE%\tools\cmake-4.4.0-windows-x86_64\bin`,
toegevoegd aan het gebruikers-PATH.

`.cargo/config.toml` zet `CMAKE_POLICY_VERSION_MINIMUM=3.5`: de libopus die met
`audiopus_sys` meekomt is uit 2021 en zijn CMakeLists vraagt om een minimumversie die
CMake 4 niet meer zonder meer accepteert.

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
