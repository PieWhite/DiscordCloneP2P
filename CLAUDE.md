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
   **Drie bewuste uitzonderingen, allemaal apart afgesproken. Dit is geen precedent voor
   een vierde:**
   - *Fase 13:* `crates/app/src/release.rs` haalt de update-feed op bij een vaste
     HTTPS-URL. Alleen tijdens een check, en de app werkt zonder internet volledig door.
     Onderbouwing: `docs/OVERDRACHT.md` beslissing 23.
   - *2026-08-20:* `crates/app/src/youtube.rs` haalt titel en miniatuur op bij
     `youtube.com/oembed` en `i.ytimg.com`, één keer per video, daarna van schijf
     (`<data>/youtube/`). **Het ophalen zit in de motor en nooit in de webview**: de CSP
     blijft dicht, dus een bericht van een peer kan geen verbinding uit het venster laten
     vertrekken. Mislukt het, dan blijft het een gewone link. Onderbouwing: beslissing 30.
   - *2026-08-20:* `crates/app/src/wordle.rs` haalt het woord van de dag op bij
     `nytimes.com/svc/wordle/v2/<datum>.json`. **Eén GET per dag per peer**, daarna van
     schijf (`<data>/wordle.json`). Elke peer haalt het zelf op: geen peer die het voor de
     anderen doet (invariant 2) en het antwoord staat nooit op de draad. Ook hier zit het
     ophalen in de motor en niet in de webview. Mislukt het, dan is er die dag geen kaart
     en werkt de rest door. Onderbouwing: beslissing 31.
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
- **Een URL naar een eigen protocol (`thumb`, `asset`) bouw je met `convertFileSrc`**, nooit
  met de hand: WKWebView wil `thumb://localhost/`, WebView2 `http://thumb.localhost/`. De CSP
  in `tauri.conf.json` moet beide vormen noemen. Met de hand gebouwd was de streamstrook op
  Windows een rij donkere vlakken (beslissing 37).
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
- **Geen automatische update op mac**: `fitcom-updater` is daar een lege stub, dus
  `engine.rs::zoek_update` slaat de release-feed over. Versies blijven per werkafspraak
  gelijk; de mac bouwt uit de broncode.
- **Geen camera-opname op mac** (bewust, 2026-08-06). `BronSoort::Camera` bestaat er wel
  zodat gedeelde code niet open hoeft, maar `beschikbare_bronnen` noemt er geen en
  `Capture::start` weigert. Naar de camera van een Windows-peer *kijken* werkt wel.
  Bouwplan in `TODO.md`.
- TCC: schermopname en microfoon zijn permissies; ad-hoc signing betekent dat
  Screen Recording na elke nieuwe build opnieuw toegekend moet worden.

## Bestanden openen en YouTube-previews (2026-08-20)
Beslissing 29 en 30 in `docs/OVERDRACHT.md`. Wat je niet mag omdraaien:

- **De webview noemt nooit een pad.** `open_file` krijgt dezelfde `OpRef` als de
  downloadknop en zoekt het pad zelf op in de momentopname van de motor. Zelfde patroon als
  B-52 (`offer_files` met indices).
- **Een uitvoerbare extensie krijgt de map, niet het bestand** (`files::opent_als_code`),
  en de knop zegt dan "Show". Eén klik in de tijdlijn mag nooit "start wat een ander mij
  stuurde" zijn.
- **`aangeboden` en `gedownload` blijven twee kaarten.** Ze samenvoegen betekent dat je
  gaat herverspreiden wat je gedownload hebt, en dat verandert wie welke bytes kan
  opvragen. Beide staan in `<data>/bestandspaden.json`; een pad dat niet meer bestaat valt
  bij het inlezen af.
- **Een video-id is elf tekens uit `[A-Za-z0-9_-]` en wordt in Rust opnieuw gecontroleerd.**
  Het gaat een URL *en* een bestandsnaam in — dat is de B-03-klasse.

## Afbeeldingen: één map, in de downloadmap (2026-08-20)
Beslissing 32 in `docs/OVERDRACHT.md`. `config::resolve_pictures_dir` is de enige plek waar
de regel staat: `<downloadmap>/Pictures`, bestandsnaam is de inhoudshash.

- **Het halve bestand van een afbeelding staat in dezelfde map als zijn eindbestemming**
  (`deelpad_van`). Dat is geen opruimkwestie: `rename` kan niet over een schijfgrens heen,
  en een downloadmap op een andere schijf dan de datamap liet zo elke afbeelding stuklopen.
  Zet het `.part` van een afbeelding nooit terug in de downloadmap.
- **De afbeeldingen verhuizen mee met de downloadmap, gewone downloads niet.** Het pad van
  een afbeelding is *afgeleid* (uit de hash) en niet onthouden; laten staan betekent uit de
  tijdlijn verdwijnen. `verhuis_afbeeldingen` + `verhuisd_pad` voor de onthouden paden.
- **Het venster leest de map uit `Snapshot::pictures_dir`**, niet uit `ui::Constants`: hij
  kan tijdens het draaien wijzigen. `pick_download_dir` opent de nieuwe map ook voor het
  `asset:`-protocol, anders laadt de webview er geen enkele afbeelding uit.
- **De laatste stap is `zet_op_zijn_plek` en geen kale `rename`.** Staat het doel er al met
  de juiste grootte, dan zijn dat dezelfde bytes (de naam is de hash) en is er niets te
  vervangen — op Windows mislukt vervangen van een bestand dat een ander proces net leest.

## Wordle van de dag (2026-08-20)
Beslissing 31 in `docs/OVERDRACHT.md`. `crates/app/src/wordle.rs` is de hele motorkant.
Wat je niet mag omdraaien:

- **De kaart van de dag is geen op en gaat nooit over de draad.** `seq` is per (auteur,
  kanaal), dus drie peers die allemaal een "hier is het raadsel"-op plaatsen zijn drie
  kaarten die de log niet tot één kan maken. De kaart draagt ook geen enkel feit dat een
  peer niet zelf kan uitrekenen; hij wordt lokaal in de tijdlijn gezet, op de klok. Wat wél
  reist zijn de uitslagen (`OpKind::WordleResult`, tag 30).
- **De oplossing blijft in de motor tot het spel klaar is.** De webview stuurt een gok en
  krijgt vijf kleuren terug. Zet het woord nooit in `UiState` zolang `klaar == false`.
- **Een Wordle-dag loopt van 07:00 tot 07:00 en de sleutel is de `print_date` van het
  raadsel**, niet de lokale datum van het moment van spelen. Anders boeken twee peers
  dezelfde avond op verschillende dagen.
- **Per (auteur, dag) wint de *eerste* op, niet de laatste.** Enige plek in
  `timeline::build` waar niet last-writer-wins geldt: een uitslag is een gebeurtenis, geen
  instelling. `Delete` doet er niets, en een oudere dag naspelen kan niet.
- **De puntenregel is N-agnostisch:** minstens twee deelnemers (`MIN_SPELERS`), een punt
  voor iedereen met het laagste aantal pogingen onder de oplossers, en niemand opgelost is
  niemand een punt. Nooit `peers.len()`, nooit een 3.
- **Bij gelijke pogingen wint de kortste speeltijd** (2026-08-30, beslissing 36). De klok
  loopt van je eerste gok tot je laatste — niet vanaf het openen van het bord, want dat
  moment kent de motor niet en een bord dat blijft openstaan is geen speeltijd. De duur
  reist als `seconds` (optioneel, additief) mee met `OpKind::WordleResult`. Een uitslag
  **zonder** tijd telt als traagst denkbaar en verliest dus elk gelijkspel; nooit `0`
  invullen voor "onbekend", dat zou hem juist elk gelijkspel laten winnen.
- **`wordle_woorden.txt` heeft een strikt formaat** dat de code gebruikt: vijf ASCII-kleine
  letters plus newline per rij, gesorteerd, zes bytes per rij, zodat er binair op gezocht
  kan worden zonder allocatie. Een test bewaakt het. De oplossing van NYT is altijd
  toegestaan, ook als hij niet in de lijst staat.

## Terugblik van de week (2026-08-26)
Beslissing 35 in `docs/OVERDRACHT.md`. `crates/app/src/gebruik.rs` is de hele motorkant.
Wat je niet mag omdraaien:

- **Gemeten tijd blijft lokaal en wordt nooit een op.** `VoiceJoin`/`VoiceLeave` zijn
  vluchtig, dus tijd in het gesprek staat nergens; elke peer meet op zijn eigen tik wat hij
  ziet en zet dat in `<data>/gebruik.json`. Er een op van maken is dezelfde val als bij de
  Wordle-kaart: drie peers die hetzelfde feit in een append-only log schrijven.
- **Tellen gaat over `Chat::alle_ops`, niet over de tijdlijn.** `timeline::build` klemt
  `wall_clock` op ±7 dagen (B-42): alles ouder komt daar op dezelfde tijdstempel uit, dus
  een venster groter dan een week zou stilzwijgend de hele geschiedenis meetellen.
- **De Wordle-punten komen uit `wordle::standen`** op de dagen in het venster — nooit een
  tweede telling, anders loopt het uit de pas met het scorebord in de chat.
- **Het overzicht zit in `Snapshot` en niet in `UiState`**, wordt eens per minuut
  uitgerekend en de UI haalt hem met `get_recap` op. Erin zetten is tien `state`-events per
  seconde zodra er een gesprek loopt.

## Camera (2026-08-06)
Een camera is een derde `BronSoort`, geen tweede pijplijn: `crates/video/src/camera.rs`
(Media Foundation, alleen Windows) levert dezelfde `Opgenomen` als de schermopname, dus
`deler`/`kijker`/`fragment` zijn onaangeraakt. `StreamKind::CAMERA = 4` is additief
toegevoegd zonder protocolbump. Zie `docs/OVERDRACHT.md` beslissing 22.

- **Bureaubladgeluid hangt aan een *scherm*, niet aan beeld.** Gebruik
  `StreamKind::is_scherm()`, nooit "alles wat geen geluid is" — anders stuurt een webcam
  ongevraagd je systeemgeluid mee.
- **Een camera is een exclusieve bron.** Media Foundation geeft hem aan één iemand tegelijk
  uit. Dus nooit een tweede opname ernaast zetten, en bij het stoppen wachten tot de vorige
  thread er echt uit is (`Cameracapture::drop`, `DelerHandle::drop`). Zie beslissing 25.
- **`crates/video/src/mf.rs`: een COM-apartment is per thread, `MFStartup` per proces.**
  Elke thread die MF aanraakt roept `zorg_dat_mf_draait()` op *zichzelf* aan. Er staat
  bewust geen `CoUninitialize` tegenover; de docstring legt uit waarom en dat is geen
  omissie om op te ruimen. Beslissing 25.
- **De camera heeft een eigen terugblik** (`DelerConfig::voorbeeld`), en daarom bestaat er
  voor een camera een deler zonder kijkers. "Er wordt pas opgenomen als er iemand kijkt"
  geldt onverkort voor een **scherm**; bij een camera ben jij die iemand. Beslissing 26.
  Die terugblik is **een tegel in de streamstrook, geen venster** (beslissing 34): de
  deel-lus legt twee keer per seconde een `kijker::maak_miniatuur` in `Gedeeld::miniatuur`,
  de motor haalt hem op via `DelerHandle::miniatuur()` en de tegel heet `self-<stream_id>`.
  Zet er geen tweede venster naast terug — dat kostte een swapchain plus een volle
  `CopyResource` per beeld, en invariant 4 gaat vóór.
- **Windows-code is op de Mac te typechecken** met een losse crate die `camera.rs` via
  `#[path]` insluit (`cargo check --target x86_64-pc-windows-msvc` op de workspace zelf
  loopt stuk op `ring`). Recept in beslissing 22 — gebruik dit vóór je Windows-code
  aanraakt zonder Windows. Werkt ook voor `mf.rs`, `venster.rs` en `app/src/geluid.rs`.

## Geluidjes (2026-08-10, sets erbij in 1.0.1)
`crates/app/src/geluid.rs` rekent de zes korte tonen zelf uit en speelt ze buiten de
voice-mixer om (`PlaySound` met de bytes in het geheugen; `afplay` op mac). Niets om mee te
leveren, niets om te bundelen. Niet-storen onderdrukt ze; mute en deafen niet. Een
stream-geluidje hangt aan een *verandering* in het aantal zichtbare streams van een peer,
nooit aan een `StreamAnnounce` — die komt bij elke herverbinding opnieuw langs.
Beslissing 27 en 28.

- **Een set is een parametertabel, geen tweede codepad.** `Geluidset::tonen` levert
  `Vec<Toon>`; er is één `samples`-functie voor alle sets. Een set erbij is een tabel erbij
  plus een variant — nergens anders iets.
- **Genormaliseerd op luidheid, niet op de piek.** `DOEL_LUIDHEID × Geluidset::gewicht(g)`,
  met de luidheid als hoogste RMS over 200 ms. Op de piek normaliseren gaf 5-9 dB verschil
  tussen sets (gemeten); `PIEK_PLAFOND` is er alleen om vervorming onmogelijk te maken. Vier
  tests bewaken dit, waaronder dat de klassieke set het niveau van 1.0.0 houdt.
- **Fase doortellen, nooit `sin(2π f t)` met een veranderende `f`.** Dat laatste geeft een
  fasesprong op elke frequentiewijziging, en een fasesprong is een tik.
- **De lijst sets komt uit de motor** (`Constants::sound_sets`), niet uit de frontend. Eén
  plek, anders lopen ze uit elkaar.
- **Gekozen set en volume staan in `config.toml` onder `[sound]`**, met `#[serde(default)]`
  op alles: een config van vóór 1.0.1 hoort gewoon te starten. Eigen test.

## Releases uitgeven
Het manifest pint zijn download vast op een tag, dus **de release moet bestaan vóór het
manifest live gaat**. De volgorde staat in `docs/OVERDRACHT.md` § "Een release uitgeven";
`fitcom-release check` is de laatste stap en die moet HTTP 200 melden, anders krijgt niemand
de update en ziet niemand waarom. `latest.json` in de repo-root is een afdruk, geen bron.

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
