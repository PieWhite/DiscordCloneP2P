# P2P communicatie-app — projectcontext

Servervrij Discord-alternatief voor 3 vaste peers over een Tailscale-tailnet.
Rust-kern. Windows 11 + macOS 14+ (Apple Silicon). UI-stack: GPUI via gpui-kit 0.7
(DirectX 11 op Windows, Metal op macOS).

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
     (`<data>/youtube/`). **Het ophalen zit in de motor en nooit in het venster**: het
     venster tekent alleen van schijf (`Ui::image_path_allowed`), dus een bericht van een
     peer kan geen verbinding uit het venster laten vertrekken. Mislukt het, dan blijft het
     een gewone link. Onderbouwing: beslissing 30.
   - *2026-08-20:* `crates/app/src/wordle.rs` haalt het woord van de dag op bij
     `nytimes.com/svc/wordle/v2/<datum>.json`. **Eén GET per dag per peer**, daarna van
     schijf (`<data>/wordle.json`). Elke peer haalt het zelf op: geen peer die het voor de
     anderen doet (invariant 2) en het antwoord staat nooit op de draad. Ook hier zit het
     ophalen in de motor en niet in het venster. Mislukt het, dan is er die dag geen kaart
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
**GPUI via gpui-kit 0.7**, sinds 2026-10-04, ter vervanging van Tauri v2 (dat in fase 12
egui verving). DirectX 11 + DirectComposition op Windows, Metal op macOS; geen webview, geen
script, geen HTML. Onderbouwing en prijs: `docs/OVERDRACHT.md` beslissing 39 (en 19 voor de
vorige wissel), `PRODUCT.md` sectie `## Stack`.

```
crates/app/src/ui/mod.rs       Bootstrap: app + venster, tray, sluiten-naar-tray, afsluiten,
                               focus, bestanden slepen, Ui::image_path_allowed
crates/app/src/ui/bridge.rs    De drie feeds: state, meters, miniaturen
crates/app/src/ui/state.rs     Snapshot -> UiState, NL -> EN vertaling
crates/app/src/ui/commands.rs  Eén functie per UiCommand, plus kiezers en vensterknoppen
crates/app/src/ui/theme.rs     De zestien kleuren en alle tokens, maten, schaduwen, fonts
crates/app/src/ui/icons.rs     Iconen als ingebedde SVG; bestanden in ui/assets/
crates/app/src/ui/fixture.rs   Alleen debug: het venster gevoed uit JSON
crates/app/src/ui/view/        AppView (de wortel) en één bestand per gebied
```

- **`proto`, `store`, `net`, `audio` en `video` zijn UI-vrij.** Geen GPUI-aanroep, en
  introduceer er ook geen andere toolkit.
- **De `Snapshot`/`UiCommand`-grens blijft staan.** De UI leest een momentopname en
  stuurt commando's terug; daarom waren beide wissels goedkoop. Hem opgeven maakt een
  volgende weer duur.
- **Drie feeds, bewust gescheiden** (`ui/bridge.rs`). `state` alleen bij een echte
  wijziging (`UiState` vergeleken met `PartialEq`), `meters` op 4 Hz en alleen bij
  verandering, voor spreekniveau en RTT, en die tik hertekent alleen de kleine
  `meter_view`s. Miniaturen op 2 Hz, rechtstreeks van BGRA naar `RenderImage`. Zet niets in
  `UiState` dat elke tik verandert: dan wordt het venster in rust weer tien keer per seconde
  wakker.
- **Kanalen, leden, tijdlijn en instellingen zijn gecachte views** (`.cached(..)`): ze
  hertekenen alleen als hun invoer verandert. Niets animeert door of veert (invariant 4);
  de typindicator knippert op een getrapte timer, en alleen terwijl iemand typt.
- **Het pop-out kijkvenster is een eigen Win32-venster met eigen D3D11-swapchain.**
  Het hete videopad raakt de UI-stack nergens.
- **Een afbeelding van schijf gaat altijd door `Ui::image_path_allowed`**: absoluut, geen
  `..`, binnen de levende `pictures_dir` of de YouTube-cache. Dat is de `asset:`-scope van
  de webview, nu als regel in code. Het venster haalt nooit iets van internet.
- **Er staat geen runtime naast de exe.** WebView2 Evergreen was de prijs van Tauri; met
  GPUI is er niets meer om te installeren of vast te pinnen, en "losse exe in een zip" blijft.
- **Het ontwerp is het zestienkleurenveld:** `DESIGN.md` + `.impeccable/design.json`, in
  Rust `ui/theme.rs`. Een kleurwaarde of radius die niet uit een token komt is een bug.
  `design/main-window.html` en `design/shots/` zijn de oudere grijze wereld en geen
  referentie meer; vergelijken doe je in de fixturemodus (zie Bouwen).
- **Venstergedrag heeft headless tests** (gpui-kit `test-support`, naast de views):
  toetsenbord, plakken, slepen. `cargo test -p fitcom` draait ze mee.
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
  vertaalt van/naar AVCC. Geen bump bij de port (toen protocol 5) — mac↔Windows interop is
  de eis.
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

- **Een view noemt nooit een pad.** `open_file` krijgt dezelfde `OpRef` als de
  downloadknop en zoekt het pad zelf op in de momentopname van de motor. Zelfde patroon als
  B-52 (`offer_files` met indices in `Ui::dropped`). Er is geen script meer om dit te
  misbruiken; de regel blijft, want hij houdt ook een fout in een view tegen.
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
  kan tijdens het draaien wijzigen. `Ui::image_path_allowed` leest hem daarom bij elke
  aanroep uit de levende momentopname; een kopie van bij het starten zou na het verzetten
  van de downloadmap elke afbeelding weigeren.
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
- **De oplossing blijft in de motor tot het spel klaar is.** Het venster stuurt een gok en
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

## Spotify delen (2026-10-14)
Beslissing 40 in `docs/OVERDRACHT.md`. Eén DJ, alleen Spotify's geluid, stereo, naar
iedereen in het gesprek. Wat je niet mag omdraaien:

- **Het is het geluid van één app, geen gesynchroniseerde Spotify.** Geen Web API, geen
  login, geen Premium: dat zou een vierde uitzondering op invariant 1 zijn. Luisteraars
  hebben geen Spotify nodig.
- **Alleen Spotify's procesboom** (`new_application_loopback_client(pid, true)`, op mac een
  SCK-filter met alleen de app). Geen terugval op de gewone loopback: die neemt je spel en
  je stem mee, en "alleen Spotify" is de belofte.
- **`StreamKind::MUSIC` hoort iedereen in het gesprek, los van beeld.** Hem op
  `DESKTOP_AUDIO` laten meeliften maakt hem afhankelijk van naar je scherm kijken.
- **Bureaubladgeluid uit zolang muziek gedeeld wordt**, anders hoort je kijker Spotify
  dubbel (`deel_bureaubladgeluid` checkt `eigen_muziek`).
- **Wat er speelt is de titel van de stream** en gaat alleen opnieuw de draad op bij een
  ander nummer (`Streams::zet_titel`). Niet in een eigen bericht en niet elke tik.
- **De geluidsketen is stereo** behalve de microfoon: decoders, mix en weergave zijn
  verweven stereo. Een `FRAME_SAMPLES`-buffer voor gedecodeerde audio is daarom
  `FRAME_SAMPLES * 2`. Muziek en bureaubladgeluid zijn 384 kbit/s, het plafond van één
  pakket (`MAX_PAYLOAD = MAX_MEDIA_PAYLOAD`); hoger kan alleen met fragmenteren.
- **Een time-out van de loopback is stilte, geen einde.** Nooit `break` op
  `wait_for_event`; pauze in Spotify mag het delen niet stoppen.

## Luisteraars bedienen de muziek (2026-10-15)
Beslissing 41 in `docs/OVERDRACHT.md`. Pauze/volgende bij de luisteraar, een knop op een
Spotify-link bij de DJ. Wat je niet mag omdraaien:

- **De DJ beslist, niet de luisteraar.** `MusicControl` (tag 46) wordt alleen uitgevoerd als
  de `stream_id` de eigen muziekstream is, de DJ en de afzender (`from` van de verbinding,
  nooit uit de payload) allebei in het gesprek zitten en de actie bekend is
  (`mag_muziek_bedienen`). Geen stemming, geen host: iedereen in het gesprek mag meteen.
- **Vluchtig en single-hop, nooit een op.** Een klik op pauze is geen geschiedenis.
- **Uitvoeren via de mediabediening van het systeem** (`muziek::bedien`), op een eigen
  thread, nooit op de motortik. Op macOS een vast script zonder iets van de draad erin,
  achter "is running": een klik van een ander start Spotify nooit op.
- **De Spotify-link stuurt niets over de draad.** Hij staat al in de chat; alleen wie Spotify
  deelt krijgt een knop die `spotify:track:<id>` opent. Niets speelt of komt in de wachtrij
  zonder de klik van de DJ in zijn eigen Spotify. Echt "aan de wachtrij toevoegen" kan niet
  zonder Web API — dat blijft nee (invariant 1).
- **Het track-id is 22 tekens base62 en wordt in Rust opnieuw gecontroleerd**
  (`muziek::geldig_track_id`, in `open_spotify_track`) voordat het een URI in gaat. B-03-klasse,
  net als het YouTube-id. `open_link` blijft alleen `http(s)` doorlaten; verruim hem niet.
- **Eén knop voor afspelen én pauzeren.** De luisteraar kent de afspeeltoestand niet; zet
  die niet op de draad om een knop te laten wisselen.

## Push-to-talk en push-to-mute (2026-10-15)
Beslissing 42 in `docs/OVERDRACHT.md`. Eén toets, drie modi, `crates/audio/src/spreektoets.rs`.
Wat je niet mag omdraaien:

- **Pollen, geen `RegisterHotKey` en geen low-level hook.** `GetAsyncKeyState` per 20 ms-frame
  in `opname_lus`. Een hotkey ziet het loslaten niet en neemt de toets van het spel af; een
  `WH_KEYBOARD_LL`/`WH_MOUSE_LL` draait op elke toetsaanslag van het systeem, in het
  invoerpad van de game (invariant 4). Zet er ook geen eigen pollthread of timer naast: de
  opname-lus tikt al op elk microfoonblok, ook met een dichte VAD.
- **De toets gaat vóór de VAD.** `Poort::open` is de ene plek die dat beslist. In
  push-to-talk geldt de VAD er niet bovenop; push-to-mute is "VAD, tenzij de toets omlaag
  staat" en heeft geen staart. Push-to-talk houdt 200 ms staart (`STAART_FRAMES`), anders
  knipt het loslaten je laatste woord af.
- **Mute wint van elke modus, en de ring volgt wat er verstuurd wordt.**
  `eigen_spreekt = poort && !mute`, in dezelfde regel als de verzendbeslissing (zie
  `df7a975`). Een tweede, aparte berekening voor de ring loopt vroeg of laat uit de pas.
- **Elke wijziging geldt meteen, zonder de sessie te herstarten.** Modus en toets zijn
  atomics in `Gedeeld`; `deelnemen` zet ze na het starten opnieuw uit de config.
- **Zonder leesbare toets blijft het spraakdetectie.** Op een platform waar
  `spreektoets::BESCHIKBAAR` `false` is (macOS) rekent `zet_spreektoets` elke modus terug naar
  spraakdetectie. Push-to-talk met een toets die nooit ingedrukt is, is een permanente mute.
- **De toetsenlijst is `spreektoets::TOETSEN`, en alleen dat.** Config, motor en scherm
  lezen die tabel; een toets erbij is één rij, en een test bewaakt dat naam, label en
  toetscode uniek zijn.
- **Een kapotte configregel valt terug, weigert niet.** `PushToTalkConfig::herstel`: onbekende
  modus wordt `voice`, onbekende toets `RightCtrl`, met een waarschuwing. De standaard na de
  update is spraakdetectie: wie niets instelt merkt niets.
- **De toets wordt niet ingeslikt.** Dat is de prijs van pollen en het hele punt ervan: de
  game die vooraan staat krijgt hem ook. Beloof in de UI dus niet dat hij "gereserveerd" is.

## "Speelt X" naast je naam (2026-10-15)
Beslissing 43 in `docs/OVERDRACHT.md`. `crates/app/src/speelt.rs` is de meting,
`ControlMsg::Activity` (tag 47) het bericht. Wat je niet mag omdraaien:

- **Het is een vluchtig, single-hop bericht en nooit een op.** Zelfde reden als status en
  typindicatie: wat iemand nu speelt is geen geschiedenis, en drie peers die dat in een
  append-only log schrijven laten hem groeien. Lege naam betekent "niets (meer)".
- **Gemeten op de motor, op een trage tik (3 s), nooit in het venster.** Een handvol goedkope
  Win32-aanroepen; de versie-resource van een exe één keer en dan onthouden. Het spel op
  dezelfde PC wint (invariant 4): geen hooks, geen procesopsomming, geen vensterlijst.
- **Over de draad alleen bij een bevestigde wissel** (`speelt::Rust`: 2 metingen voor een
  nieuw spel, 10 voor "geen spel"). Niet bij elke meting en niet bij elke alt-tab.
- **`speelt::schoon` voor alles wat een naam is, eigen en vreemd.** Lengte, stuurtekens,
  bidi en zero-width eruit; getekend als gewone tekst. De naam komt van een andere PC.
- **Een voorgrondvenster telt alleen als spel zonder titelbalk én over de hele monitor én
  niet op `GEEN_SPEL`.** Een vals alarm los je op in die lijst, niet met een extra heuristiek
  of een spellendatabase (invariant 1: geen server, geen lookup).
- **De naam is `FileDescription`, anders de exe-naam, nooit de venstertitel.** Een titel kan
  een documentnaam zijn en gaat naar alle peers, met het delen standaard aan.
- **Standaard aan, uit via `Config::share_activity`.** Uit = niet eens meten, niets sturen,
  en een lege naam naar de anderen zodat hun regel verdwijnt.
- **Staat in `UiState` (`PeerState::activity`) alleen omdat het alleen op een wissel
  verandert.** Zet hier nooit iets neven dat per tik beweegt.
- **macOS:** `Zoeker::voorgrond_spel` is er `None`. Geen Accessibility-/Screen Recording-
  vraag voor een regel tekst.

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
  De hele workspace kan sinds beslissing 39 ook, met mingw-w64; het recept staat daar.

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

## Clip delen (2026-10-15)
Beslissing 44 in `docs/OVERDRACHT.md`. De koppeling tussen clips en bestandsdeling; geen
nieuwe pijplijn en geen protocolwijziging. Wat je niet mag omdraaien:

- **Een clip is alleen een aanbod.** Nooit automatisch downloaden, ook niet onder een
  grootteplafond zoals bij afbeeldingen: `files::is_afbeelding` blijft onwaar voor een mp4.
- **Aanbieden pas na `ClipGebeurtenis::Klaar`**, nooit eerder; dat event komt na het sluiten
  van het bestand.
- **Automatisch posten staat standaard UIT** en gebeurt alleen terwijl je in een gesprek
  zit (`voice.is_some()` op het moment van `Klaar`), in het algemene kanaal. De config-
  sleutel heet `clips.auto_post`; **zet hem nooit terug op `auto_delen`**: 1.10.0 leverde
  die sleutel met standaard aan, en de oude waarde moet genegeerd blijven. De knop werkt
  altijd.
- **De deelkaart is lokaal.** Na elke clip staat er in #general, alleen bij de maker, een
  kaart "Clip saved" met een Share-knop (`TimelineItem::Clip`, uit `ClipsWeergave::kaart`).
  Geen op, nooit over de draad, zelfde patroon als de Wordle-kaart: ze wordt in
  `timeline_of` op de klok tussen de berichten gezet. Alleen de laatste clip heeft een kaart
  en na een herstart is ze weg. Maak er nooit een op van.
- **De view noemt nooit het pad.** `DeelLaatsteClip` draagt niets; de motor kent
  `ClipsBeheer::laatste()`. Dezelfde clip gaat niet tweemaal de chat in (`clip_aangeboden`).

## Tekenen op een gedeeld scherm (2026-10-15)
Beslissing 45 in `docs/OVERDRACHT.md`. Kijkers tekenen in het kijkvenster; de deler en alle
kijkers van dat scherm zien het. `crates/proto/src/teken.rs` is het model, `streams.rs` de
routering, `crates/video/src/tekenlaag.rs` de lagen, `penseel.rs` het tekenen. Wat je niet
mag omdraaien:

- **Lijnen zijn berichten, nooit beeld en nooit een op.** `Draw`/`DrawClear`/`DrawState`
  (tags 48-50) zijn vluchtig; het videobeeld en de opname van de deler bevatten geen lijn.
  Een lijn in de log of in het beeld bakken is de fout die dit ontwerp vermijdt.
- **De deler is het middelpunt van zijn eigen scherm.** Een kijker stuurt alleen naar de
  deler; de deler neemt alleen aan van wie nu intekende (`kijkers`), met de tekenaar uit de
  verbinding en nooit uit het bericht, en geeft door aan al zijn kijkers in de volgorde
  waarin hij zelf toepaste. Een kijker neemt tekenwerk alleen aan van de eigenaar van een
  stream waar hij naar kijkt. Geen broadcast naar iedereen met filteren achteraf: dan is er
  geen plek meer waar de schakelaar en de grenzen gelden.
- **Geen lokale echo.** Ook je eigen lijn komt terug via de deler. Eén weg, dus iedereen ziet
  hetzelfde; de prijs is één rondgang (een paar tientallen ms). Een `Draw` zónder tekenaar is
  "op jouw scherm", mét tekenaar "op het scherm van de afzender": stream-id's zijn alleen per
  eigenaar uniek, en dat veld is het onderscheid.
- **Deler en kijker passen dezelfde `Tekening` toe**, met dezelfde grenzen (64 punten per
  bericht, 512 per lijn, 32 lijnen per tekenaar met de oudste eraf, 256 per scherm). Twee
  sets regels lopen uit elkaar. Een kopie met een gat (`heeft_gat`) tekent opnieuw in op
  dezelfde poort, hooguit eens per 2 s, en de deler antwoordt op elke intekening van een
  scherm met de hele `DrawState`, ook als die leeg is.
- **Of het mag staat op `StreamAnnounce::drawing`** (`#[serde(default)]`), en uit wist alles.
  Een oudere deler stuurt het niet en krijgt dus geen pen. Geen apart bericht voor de
  schakelaar.
- **Lijnen leven zolang de tekenaar kijkt.** Uittekenen of wegvallen wist ze bij de deler
  (`vergeet_tekenaar`), en stoppen met delen of intrekken neemt alles mee.
- **Tekenen kost niets per beeld.** De lagen zijn layered vensters die alleen bij een
  wijziging opnieuw tekenen (alleen het stuk dat erbij kwam, `penseel::aanvulling`), hooguit
  eens per 30 ms (`MIN_HERTEKEN`). De swapchain en het videopad van het kijkvenster weten er
  niets van; het venster pakt de tekening op zijn bestaande pomptempo, nooit per pakket.
- **De laag bij de deler wijkt voor alles wat schermvullend vooraan staat op die monitor**
  (`tekenlaag::schermvullend_vooraan`, dezelfde regel als "Speelt X"). Een venster bovenop
  een schermvullend spel kost beelden of een alt-tab (invariant 4). De lijnen staan dan nog
  bij de kijkers en op de eigen tegel. Maak hem niet "slim genoeg om toch te tonen".
- **De laag bij de deler blijft buiten de opname** (`WDA_EXCLUDEFROMCAPTURE`). Lukt dat niet,
  dan is er geen laag; anders staan de lijnen dubbel bij de kijkers en in elke clip.
- **Alleen een gedeelde monitor krijgt een laag.** Een venster delen heeft er geen: een laag
  die een ander venster volgt is een tweede bouwwerk. Daar staan de lijnen op de tegel.
- **De pen begint neer.** Slepen tekent pas na D of het menu onder de rechterknop; een klik in
  het beeld is nooit per ongeluk een streep op het scherm van een ander. Een lijn van één
  punt gaat niet weg (de eerste klik van een dubbelklik).
- **In `UiState` alleen `drawing` en `has_drawing`**, die wisselen bij de schakelaar en bij
  de eerste of laatste lijn. De lijnen zelf gaan via de miniaturenpomp de tegels in (2 Hz),
  nooit via `UiState`.
- **macOS:** alleen de tegels. Geen pen in het kijkvenster en geen laag bij de deler
  (`mac/tekenlaag.rs` is een lege `Deellaag`).

## Sleutels en versleutelde media (protocol 6, 2026-10-19)
Beslissing 46 in `docs/OVERDRACHT.md`, fase A van `docs/PLAN-ZONDER-TAILSCALE.md`. Tailscale is
niet langer de beveiligingsgrens: de handshake bewijst wie er belt en alle media is versleuteld.
Wat je niet mag omdraaien:

- **De `PeerId` blijft de UUID; de sleutel staat ernaast** (`identity.toml`, en per peer
  `known_id` + `public_key` in `config.toml`). Nooit `PeerId = hash(sleutel)`: elke op in de
  store draagt een UUID-auteur, en een nieuwe `PeerId` maakt de hele geschiedenis wees.
- **TLS blijft anoniem; de handtekening over de exporter is de controle**
  (`fitcom_proto::control::Handtekening`, rol `client`/`server`). Ga niet "voor de zekerheid"
  ook certificaten pinnen: één plek voor vertrouwen, en die is getest.
- **Geen `HelloAck` vóór het oordeel van de actor** (B-05a). Wie niet binnenkomt krijgt een
  stille close, zonder reden — ook bij een `Hello` met onze eigen `PeerId`. Alleen een peer op
  een andere protocolversie krijgt iets: `HelloAck::alleen_versie()`, met niets erin dan de
  versie.
- **Eén beoordeling voor bellen en opnemen** (`Actor::beoordeel`). Twee sets regels lopen uit
  elkaar, en dan is één van de twee paden de achterdeur.
- **Een sleutel leren gebeurt alleen op een tailnet- of loopbackadres** (`mesh::mag_leren`),
  inkomend bovendien alleen vanaf het opgezochte IP van dat target. Verruim dat nooit naar een
  LAN- of publiek adres: daar is de eerste die zich meldt de eigenaar. Een sleutel voor een
  publiek adres komt in fase B uit een uitnodigingscode.
- **Adressen zijn hints, nooit toegang.** Een gepinde sleutel komt binnen vanaf elk adres; een
  ongepinde nergens buiten het tailnet.
- **Een korte sessie is een afwijzing** (`KORTE_SESSIE`, B-40): anders belt een peer met een
  verkeerde pin elke seconde, met bij de ander elke seconde een online/offline-ronde.
- **Media gaat altijd door de AEAD, zonder onversleutelde terugval** (`mediasleutel.rs`). De
  teller zit in de nonce: een sleutel krijgt nooit een verse teller (`MediaSleutels::zet`
  vervangt een zendsleutel van dezelfde verbinding niet), en twee richtingen delen nooit een
  sleutel (de context is de `PeerId` van de afzender).
- **Sleutels leven alleen in de tabel van de mesh-actor.** Nooit in `PeerStatus`, `Snapshot`,
  `UiState` of een logregel; `MediaSleutels` en `Identity` hebben bewust geen `Debug` die
  sleutels toont. `MediaSleutels::koppel` is voor tests en voorbeelden, nooit naast een mesh.
- **Pas markeren ná geslaagd ontsleutelen**, en een onbekende index kost geen cryptografie.
  Het replayvenster is per ontvangende socket, plus een hoogste-tellermerk per sleutel over
  alle sockets; haal dat merk niet weg "omdat er al een venster is": dan neemt een nieuwe
  socket (opnieuw deelnemen, opnieuw kijken) elk oud pakket nog eens aan.
- **Een consument vertrouwt de `PeerId` uit `MediaSocket::ontvang`, nooit een bronadres.**
  Breng geen bronadres- of bronpoortcontrole terug als "extra" laag; dat was B-28.
- **`MAX_MEDIA_PAYLOAD` blijft 1100** zolang het grootste pakket met envelop, IPv6 en UDP
  onder de 1280 blijft; een test in `proto/src/media.rs` bewaakt dat.
- **De routerprobe (A0) meet één keer, 10 s na het starten, op een eigen runtime op een
  blokkeerthread** (portmapper zoekt de gateway synchroon), nooit op de mesh-threads of een
  mediathread. De tijdelijke mapping gaat er meteen weer af, met een tweede probe als
  wachtpost (anders breekt het weggooien van de client het vrijgeven af). "Geen router
  gevonden" is een eigen uitkomst met één nieuwe poging, nooit "router kan niets". Geen
  interfacelijst via netwatch (WMI op Windows) en geen ✓/✗ in de UI-regel (niet in de fonts).

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

Geen Node en geen frontend-bouwstap: de weergave is Rust, en fonts en iconen uit
`crates/app/src/ui/assets/` zitten in de exe. `cargo build` bouwt de hele app.

- **macOS:** de Xcode Command Line Tools volstaan. gpui-kit zet `runtime_shaders` aan,
  dus er is geen Metal-compiler nodig. `bundle-mac.sh` haalt het icoon uit `icons/icon.png`.
- **Windows:** een *release*-build compileert de HLSL-shaders van GPUI met `fxc.exe` uit de
  Windows SDK (via `GPUI_FXC_PATH`, `PATH` of het register; zonder paniekt de build). Die
  SDK komt mee met de C++-workload van de VS Build Tools die MSVC toch al vraagt. Een
  debug-build compileert de shaders pas bij het starten, uit de cargo-registry van de
  bouwmachine: zo'n exe tekent alleen daar. Uitdelen gaat altijd met een release-build.
- **Windows:** `build.rs` bakt `icons/icon.ico` in de exe via `winresource`. Het manifest
  (common controls v6, PerMonitorV2-DPI) komt van GPUI's feature `windows-manifest`; een
  tweede ernaast geeft bij het linken CVT1100 "duplicate resource".

**Fixturemodus** (alleen debug): het venster zonder mesh, store en motor, gevoed uit JSON
(`state.json` verplicht, de rest staat in `ui/fixture.rs`). Zo is elke toestand te tekenen en
te screenshotten zonder tweede machine en zonder klikken.

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
FITCOM_UI_FIXTURE=<map> cargo run -p fitcom    # fixturemodus, alleen debug
FITCOM_DEBUG_VIEW="view=settings;tab=audio" FITCOM_DEBUG_SIZE=980x720 FITCOM_UI_FIXTURE=<map> cargo run -p fitcom
                                     # zelfde, met beginstand (tab, dialoog, scroll) en maat
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
