# Product

<!-- impeccable:product-schema 1 -->

## Platform

desktop

> Buiten de vier waarden die het schema kent (`web`/`ios`/`android`/`adaptive`). Dit is een
> **native Windows-desktopapplicatie**, gedistribueerd als losse `fitcom.exe` in een zip.
> Geen mobiel, geen browser, geen publieke URL.
>
> **Het weergavesubstraat is wél webtechnologie** (Tauri v2 op WebView2 — zie `## Stack`).
> CSS, design tokens, echte typografie, motion en impeccable's detector zijn dus allemaal
> van toepassing. Wat níet van toepassing is: mobiele breakpoints, touch-doelgroottes,
> browserchroom, SEO, meerdere viewports en cross-browser-compatibiliteit. Er is precies
> één engine (Chromium via WebView2) op precies één OS, in een venster dat de gebruiker
> vrij kan verslepen en maximaliseren op een scherm van 1080p tot 3440×1440.

## Stack

**Rust-kern + Tauri v2-weergave.** Besloten 2026-08-04, ter vervanging van egui/eframe.

Wat blijft: **de vijf niet-UI-crates onaangeroerd** — `proto`, `store`, `net`, `audio`,
`video`. Gemeten vóór het besluit: buiten `crates/app/src/ui/` komt egui daar alleen voor
in *doc-commentaar*; geen enkele API-aanroep. De hele wire-protocol-, sync-, voice- en
screensharelaag staat dus buiten deze wissel.

Wat vervangen wordt: `crates/app/src/ui/` (13 modules, 2.979 regels) en de
vensterbootstrap in `main.rs` (~40 regels).

Waarom dit goedkoop kan: de UI is al een pure weergave die een `Snapshot` leest en
`UiCommand`'s terugstuurt over kanalen (`crates/app/src/ui/mod.rs`, `engine.rs`). Die grens
wordt Tauri-commands en -events — grotendeels een mechanische vertaling. **Die grens is de
reden dat dit haalbaar is; hem opgeven maakt een volgende stackwissel weer duur.**

Waarom Tauri en niet iets anders:

- **Ontwerpplafond.** egui kan geen echte typografie, ritme, animatie of gelaagdheid. CSS
  wel, en het is de enige optie waar impeccable's volledige gereedschap rechtstreeks op werkt.
- **Idle wordt goedkoper, niet duurder.** egui hertekende 4× per seconde in rust en 12,5×
  tijdens een gesprek, omdat immediate-mode niet anders kan. Een event-driven weergave
  tekent alleen bij verandering. Dat werkt vóór het principe "de app in rust doet niets",
  niet ertegen — dit was een argument om te wisselen, geen concessie.
- **Twee bestaande schrammen verdwijnen.** De `GetAsyncKeyState`-omweg voor Ctrl+V
  (`docs/OVERDRACHT.md`, beslissing 15) bestond alleen omdat `egui-winit` de plakopdracht
  opslokte; een webview krijgt een echt `paste`-event met de afbeelding erin. En de
  zelfgebouwde titelbalk kostte in egui 194 regels hit-testing.
- Overwogen en afgewezen: **Dioxus 0.7** (één taal, echte CSS, maar pre-1.0-churn en een veel
  dunner ecosysteem) en **Slint 1.17** (geen webview nodig, fors beter dan egui, maar een
  eigen DSL in plaats van CSS — plafond merkbaar lager).

Wat dit kost, expliciet, zodat een latere sessie het niet als vergissing leest:

- **WebView2 is een Windows-component die jij niet in de hand hebt.** Dezelfde *soort*
  afhankelijkheid als de HEVC Video Extensions, die om precies die reden HEVC als
  standaardcodec kostte. Verschil: WebView2 zit standaard in Windows 11 en alle drie de
  machines draaien Windows 11. Zwakkere versie van dat probleem, geen herhaling. **Niet
  overstappen op de fixed-version runtime** — dat is ~180 MB en sloopt "losse exe in een zip".
- **Een tweede taal in de repo**, en een frontend-bouwstap naast `cargo build`.
- **De miniaturenstrook heeft een nieuw transport nodig.** Die krijgt nu elke 500 ms een
  BGRA-downscale rechtstreeks in een egui-textuur (`D3dContext::lees_bgra_miniatuur`). Naar
  een webview moet dat via een event of custom protocol naar een canvas. Bij 2 fps en een
  kleine afmeting is dat verwaarloosbaar, maar het is werk dat nu gratis is.
- **Het pop-out kijkvenster blijft native en verandert niet.** Het argument ervoor wordt
  zelfs sterker: eframe rendeerde via wgpu op DX12 en een gedecodeerde D3D11-textuur daarin
  krijgen vroeg al `wgpu-hal`-interop (`docs/ARCHITECTURE.md`); in een webview is dat
  moeilijker, niet makkelijker. Het hete videopad raakt de UI-stack dus nergens.
- **Eén portable exe blijft de eis.** Frontend-assets horen in de binary; geen losse
  `dist`-map naast `fitcom.exe`.

## Users

Precies drie vaste, elkaar persoonlijk kennende gebruikers (Rick + twee vrienden). Geen
accounts, geen registratie, geen onbekende gebruikers, geen groei naar meer mensen als
productdoel — de code is wél N-agnostisch en het aantal komt uit config.

Alle drie:

- Windows 11, NVIDIA Turing of nieuwer, 1 Gbit/s symmetrisch, Tailscale altijd draaiend.
- Altijd een headset (dat is de reden dat er geen echo-onderdrukking in zit).
- **Twee schermen per persoon.** Bevestigd 2026-08-04. `docs/SPEC.md` gaat nog uit van
  "1 monitor per persoon"; dat is achterhaald en heeft eerdere UX-keuzes beïnvloed (o.a.
  de onderbouwing voor het pop-out kijkvenster).

Ze zijn technisch genoeg om een `config.toml` te bewerken en een zip uit te pakken, maar
gebruiken de app in hun vrije tijd — niet als werk. Het is geen tool waar iemand voor
betaalt of op afgerekend wordt.

## Product Purpose

FitCommunication vervangt Discord voor deze drie mensen: praten, chatten, elkaars scherm
bekijken en bestanden uitwisselen, zonder dat er ook maar één server, cloud-API of account
tussen zit. Succes is dat niemand nog een reden heeft om Discord te openen, en dat de app
in rust praktisch niets doet.

## Positioning

Wat een naburig product niet waar kan maken: **nul servers en geen host-peer.** Geen
signaling, geen TURN, geen database, geen cloud-API, geen accounts. Tailscale is de enige
externe afhankelijkheid en wordt als draaiend verondersteld. Elke instantie initieert én
accepteert; geen enkele functie hangt af van "peer X is er". Eén of twee peers offline is
een gewone toestand, geen foutpad.

Daaruit volgt de tweede, even harde eis: **gamen wint.** Merkbare impact op een draaiende
game op dezelfde PC is een bug, ook als de kwaliteit daarvoor omlaag moet. Dat is geen
marketingclaim maar de meetlat waaraan features gesneuveld en teruggedraaid zijn (zie de
bitrate-verlaging van ~25 naar 12 Mbit/s in `docs/SPEC.md`).

## Operating Context

Alle gebruikssituaties hieronder zijn echt en komen alle vier voor — geen ervan is de
uitzondering:

1. **Weggeklikt in de tray tijdens gamen.** Game fullscreen op het hoofdscherm; de app
   meldt zich via een Windows-toast en het tray-icoon. Sluitknop minimaliseert naar de
   tray (`minimize_to_tray`, standaard aan) zodat synchroniseren en melden doorgaan.
2. **Actief venster tussen sessies.** De app is dan zelf de bezigheid: kletsen, bestanden
   delen, terugkijken.
3. **Permanent zichtbaar op het tweede scherm, naast een game.**
4. **Als kijkvenster voor screenshare.** De pop-out streams (eigen D3D11-swapchain,
   maximaliseerbaar, F11 voor beeldvullend) zijn dan het product; het hoofdvenster met de
   overzichtstrook is bijzaak.

**Bijna altijd 's avonds, in het donker.** Helderheid en oogbelasting zijn daarom een
echte eis, geen smaakkwestie. Het thema is één vaste donkere combinatie zonder
runtime-wisseling en zonder OS-thema-navolging — dat is bewust.

De app is portable: `config.toml`, `identity.toml`, `chat.sqlite` en logs staan naast de
exe (fallback `%APPDATA%\FitCommunication`). Distributie is een zip die met de hand naar
de andere twee gaat; sinds fase 11 kan een peer een nieuwere versie automatisch van een
andere peer ophalen, maar hij past hem pas toe na een expliciete bevestiging.

## Capabilities and Constraints

Alles hieronder is gebouwd en werkt (fase 0 t/m 11 uit `ROADMAP.md`):

- QUIC-mesh over het tailnet (chat, control, bestanden), plain UDP voor audio/video.
  Auto-reconnect met exponentiële backoff; alleen geconfigureerde peer-UUID's worden
  geaccepteerd.
- Voice: Opus 48 kHz/20 ms, open mic met VAD, RNNoise-ruisonderdrukking, adaptieve
  jitterbuffer per spreker, lokale mix, per-deelnemer volume, mute/deafen. Expliciet
  join/leave — niet altijd-aan.
- Screenshare: WGC-capture van monitor én venster, H.264 via Media Foundation,
  subscribe-on-demand, meerdere bronnen en meerdere kijkvensters tegelijk. Bureaubladgeluid
  gaat automatisch mee als aparte stream met eigen volumeschuif bij de luisteraar.
- Tekstchat met volledige geschiedenis-inhaal na offline zijn, `@username`-tags met
  autocomplete, niet-storenmodus, bewerken/verwijderen van eigen berichten.
- Kanalen: algemeen, benoembare subkanalen onder algemeen, en DM's tussen twee peers.
  **Een DM gaat nooit via een derde peer** (er is geen encryptie, dus doorsturen zou de
  derde laten meelezen) — bewuste trade-off.
- Bestanden inline in de tijdlijn: slepen-en-neerzetten, Ctrl+V voor afbeeldingen,
  hervatbaar downloaden, BLAKE3-verificatie, miniaturen via een content-adresseerbare map.

Vastliggende technische randvoorwaarden die ontwerpwerk niet mag omzeilen:

- **Protocol alleen additief wijzigen.** Nieuwe enum-varianten en structvelden aan het
  eind, nieuwe velden `#[serde(default)]`, onbekende varianten loggen en negeren.
- **Ops zijn onveranderlijk en idempotent.** Twee keer toepassen is een no-op; dat is de
  volledige conflictafhandeling.
- **`crates/proto` en `crates/store` bevatten geen Windows- of hardware-afhankelijkheden.**
- **Media-code loopt op eigen threads en praat via kanalen met de UI. Geen locks op het hot
  path.** De UI is een pure weergave van een momentopname; de chat- en sync-lus draait op de
  tokio-runtime en **nooit in de weergavelus**. Reden: een UI tekent niet als het venster
  verborgen of geminimaliseerd is, en dat is precies het moment waarop je een melding wilt
  krijgen. Dat gold voor egui en geldt onverkort voor een webview.
- **H.264 is de standaardcodec, niet HEVC** (HEVC-decode hangt op Windows aan een
  Store-uitbreiding die er niet standaard op zit). **AV1 valt af** zolang de RTX 2080 Super
  meedoet — die kan het niet encoden en niet decoden. **4:4:4 is afgewezen, niet uitgesteld:**
  geen Turing-GPU kan het hardwarematig decoderen.
- Er is nog geen instellingenscherm voor alles: peer-adressen, autostart, download- en
  picturesmap staan in `config.toml`. Video-instellingen, gebruikersnaam en
  microfoon/weergave zitten wél in het Instellingen-venster.

### Expliciet onbeslist

- **UI-taal.** De UI, de documentatie en de code-identifiers (`deler`, `kijker`, `venster`,
  `Actie`, …) zijn nu Nederlands. Rick geeft aan dat **Engels beter zou zijn**, maar dat is
  nog geen genomen beslissing en de omvang is niet bepaald: alleen zichtbare UI-strings, de
  nieuwe frontend erbij, of ook de Rust-identifiers en de docs. Er is geen vertaallaag en
  i18n is geen doel — het gaat om één taalkeuze, niet om meertaligheid. De Tauri-migratie is
  het natuurlijke moment om dit voor de weergavelaag te beslissen, want die wordt toch
  helemaal opnieuw geschreven. Niet stilzwijgend één kant op oplossen.

## Brand Commitments

- Naam: **FitCommunication**, binary `fitcom.exe`.
- **Donker is een eis, geen stijlkeuze.** Zie `## Accessibility & Inclusion`: de app wordt
  bijna altijd 's avonds in het donker gebruikt. Geen lichte modus, geen themawisselaar,
  geen OS-thema-navolging.
- Toon: nuchter en zonder marketingstem. De bestaande UI-teksten en foutmeldingen zeggen
  wat er aan de hand is en wat je eraan doet ("offline · peer reageert niet"). Dat blijft.
### De visuele wereld: de categoriestandaard, met opzet

**Vastgelegd 2026-08-04.** In een richtingsronde met zeven afgeleide werelden en vier
gepresenteerde kaarten heeft Rick de **staande uitgang** gekozen: de categoriestandaard,
recht toe recht aan. Niet bij gebrek aan alternatieven — de aangewezen richting (een
handbediende telefooncentrale: snoeren, jackveld, lampenveld) en drie uitdagers
(vertrekbord, Teletekst, Schiphol-signering) lagen er volledig uitgewerkt naast.

Dit is dus een **commitment, geen compromis**, en toekomstig werk behandelt het zo:

- **De conventionele indeling is de opdracht**, uitgevoerd zonder ironie en zonder
  eigenzinnigheid die er alsnog in gesmokkeld wordt. Icoonrail → kanaal/DM-lijst → tijdlijn
  → ledenlijst. Ronde avatars, kanaalitems als lijstrijen, één accentkleur, donkere
  kolommen. Wie van Discord komt moet het blind kunnen bedienen.
- **De kwaliteitslat is Discord en Slack.** Hun afwerkingsniveau is de meetlat: dat is waar
  het werk aan afgemeten wordt, niet aan wat er nu in egui staat.
- **Niet "anti-referentie" meer.** Ik heb dat woord eerder in dit bestand gebruikt toen de
  richting nog open stond. Het is nu onjuist: `crates/app/src/ui/theme.rs` is een vroege,
  laag-fidelity uitvoering van precies de richting die nu gekozen is. Het is een vertrekpunt
  met een te lage afwerking, geen doodlopende weg.
- **Het palet zelf blijft open.** Rick heeft bij de stackwissel voor visueel opnieuw doen
  gekozen; de teal `#3ABFC0` en de vijf grijze lagen zijn geen vastgelegde waarden. Wat
  vastligt is de *soort* wereld, niet de exacte kleuren.
- **De informatiestructuur blijft staan**, op Ricks expliciete keuze — de vier zones zijn
  uitgeprobeerd en je vindt alles blind. Materiaal, typografie, kleur, dichtheid, ritme en
  motion zijn vrij binnen de conventie.

`DESIGN.md` wordt aan het eind van de bouw geschreven, uit de gebouwde wereld, niet vooraf.

## Evidence on Hand

- Werkende app met de volledige testsuite groen (`cargo test --workspace`); zwaartepunt in
  `crates/proto` en `crates/store`.
- **Gemeten op de dev-PC:** 1080p op 55–56 fps, nul beelden onderweg kwijt, 3,1 ms tussen
  opnemen en tonen in een debug-build. Codec-ondersteuning per encoder/decoder is
  daadwerkelijk uitgevraagd, niet aangenomen (tabel in `docs/SPEC.md`).
- **Alle functionaliteit werkt in de praktijk.** Door Rick bevestigd op 2026-08-04: de
  volledige app draait tussen de echte peers — netwerklaag, tekstchat, voice, screenshare,
  bestandsdeling, kanalen, DM's, subkanalen, tags, meldingen, updates.
  `docs/TESTPLAN.md` en de statustabel in `docs/OVERDRACHT.md` staan op dit punt nog
  achter: de resterende "nog niet met echte hardware getest"-markeringen daar
  (1440p/ultrawide, de WASAPI-exclude-route, een echt versieverschil) zijn achterhaald.
  **Ontwerpwerk hoeft dus met geen enkel functioneel voorbehoud te rekenen** — er is niets
  half werkend dat een UI zou moeten verhullen of van een waarschuwing voorzien.
- Er zijn **geen** gebruikers buiten deze drie, geen testimonials, geen klanten, geen
  benchmarks tegen Discord, geen prijs, geen licentie en geen publieke distributie. Niets
  daarvan verzinnen.

## Product Principles

1. **Nul servers is het product, niet een implementatiedetail.** Elke functie die een
   server, cloud-API of account zou vragen, wordt afgewezen in plaats van uitgezonderd —
   ook als hij mooi zou zijn (zie de afgewezen YouTube-voorvertoning in `TODO.md`).
2. **Gamen wint van kwaliteit.** Bij twijfel gaat de instelling omlaag. Een gemeten
   regressie op rust/latency draait terug, ook als de vorige onderbouwing logisch klonk.
3. **Geen host, geen hiërarchie.** Alle peers zijn gelijkwaardig; niets in het ontwerp mag
   "de eigenaar", "de beheerder" of "wie het gesprek startte" bevoorrechten.
4. **Offline is een normale toestand, geen foutpad.** Een afwezige peer krijgt een rustige,
   informatieve weergave — geen alarmkleur, geen blokkade, geen dialoog.
5. **De app in rust doet niets.** Weggeklikt in de tray tijdens een game is de duurste
   toestand die hij moet halen: stil, koel, en toch synchroon en meldingsvaardig.

## Accessibility & Inclusion

- **Avondgebruik in het donker is de norm.** Helderheid en oogbelasting zijn een echte
  ontwerpeis: geen grote heldere vlakken, geen felle wit-op-donker-accenten die in een
  donkere kamer nabranden. De vaste donkere basis is daar het antwoord op en blijft.
- Geen bekende beperking bij de drie gebruikers wat betreft tekstgrootte of
  kleurwaarneming. Contrast en leesbare tekstgroottes gelden als algemene ondergrens, niet
  als specifieke eis; er is geen te halen standaard afgesproken.
