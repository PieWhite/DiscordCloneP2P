# Product

<!-- impeccable:product-schema 1 -->

## Platform

desktop

> Buiten de vier waarden die het schema kent (`web`/`ios`/`android`/`adaptive`). Dit is een
> **native desktopapplicatie voor Windows en macOS**: op Windows een losse `fitcom.exe`
> in een zip, op macOS een `FitCommunication.app` in een zip (`scripts/bundle-mac.sh`).
> Geen mobiel, geen browser, geen publieke URL. iOS is op 2026-08-05 overwogen en
> bewust uitgesteld: geen scherm delen mogelijk, en zonder servers geen push.
>
> **Twee dingen kan alleen de Windows-kant:** de P2P-auto-update (bewust — een mac mag
> nooit een Mach-O over `fitcom.exe` heen zetten) en het *uitzenden* van de camera
> (2026-08-06, op verzoek uitgesteld; `TODO.md`). Kijken naar de camera van een
> Windows-peer werkt op de mac wel. Al het andere is featurepariteit.
>
> **Het weergavesubstraat is sinds 2026-10-04 geen webtechnologie meer** maar GPUI (zie
> `## Stack`): Rust-views die zelf tekenen, via Metal op macOS en DirectX 11 op Windows.
> Design tokens, echte typografie en de motion-regels uit `DESIGN.md` gelden onverkort, nu
> in Rust (`crates/app/src/ui/theme.rs`); CSS en de twee browserengines zijn weg. Wat níet
> van toepassing is: mobiele breakpoints, touch-doelgroottes, browserchroom, SEO en
> meerdere viewports. Het venster is vrij te verslepen en te maximaliseren op een scherm van
> 1080p tot 3440×1440, met een minimum van 780×460. *(Van 2026-08-04 tot 2026-10-04 was
> het Tauri v2, met Chromium via WebView2 op Windows en WebKit via WKWebView op macOS.)*

## Stack

**Rust-kern + GPUI-weergave (gpui-kit 0.7).** Uitgevoerd 2026-10-04, ter vervanging van
Tauri v2, op Ricks verzoek: de app moet minder kosten om open te hebben. Zie
`docs/OVERDRACHT.md`, beslissing 39. De afweging van fase 12 staat hieronder als historie;
de reden van toen (het ontwerpplafond van egui) blijft gelden, en het zestienkleurenveld was
in GPUI na te tekenen (sinds 2026-10-20 vervangen door B, beslissing 48).

Wat gebleven is: **de vijf niet-UI-crates en de motor onaangeroerd**, en de
`Snapshot`/`UiCommand`-grens. `ui/state.rs` is hetzelfde contract, `ui/commands.rs` heeft
dezelfde bodies als gewone Rust-functies. Opnieuw geschreven is wat de webview was:
`crates/app/frontend/` werd `crates/app/src/ui/view/`.

Wat de wissel oplevert:

- **Geen webview, dus ook niet zijn prijs.** De 381 MB werkset van fitcom plus zes
  WebView2-processen, WebView2 als Windows-component die jij niet in de hand hebt, en een
  tweede taal in de repo: alle drie weg. De weergave is Rust, in hetzelfde proces als de
  motor. De gemeten cijfers van GPUI staan in beslissing 39.
- **De app in rust doet nog steeds niets.** De drie feeds blijven en worden pas wakker als er iets
  verschilt. Daarbovenop hertekent een meters-tik alleen de kleine meterweergaven, en zijn
  de zware panelen (kanalen, leden, tijdlijn, instellingen) gecachet.
- **Geen script, geen HTML.** De beveiligingsbevindingen die aan de webview hingen (B-52
  t/m B-56) hebben geen aanvalsoppervlak meer; de regels erachter staan nu gewoon in code.

Wat het kost, expliciet:

- **Windows is nog door niemand gestart.** Al het verifiëren gebeurde op de Mac:
  fixturemodus, 97 referentietoestanden, headless venstertests, en een typecheck voor
  Windows. Tot iemand `docs/TESTPLAN.md`, "GPUI-weergave (2026-10-04)", heeft afgelopen,
  is het op Windows onbewezen.
- **Afbeeldingen worden in ons eigen proces gedecodeerd** in plaats van in de sandbox van
  WebView2. Begrensd op 8192 px per zijde; zie B-61 in `docs/BEVEILIGING.md`.
- **Kleine verschillen met de webview:** geen letter-spacing (benaderd), keuzelijsten in
  DotGothic16, de emojibalk in de berichtstroom, een selectie die alleen de zichtbare rijen
  kopieert, en plakken alleen met de focus in de composer. Volledige lijst in beslissing 39.
- **Een vastgepinde momentopname.** GPUI komt als `gpui-pre` 0.3.7 (Zeds GPUI op
  crates.io), vastgepind door gpui-kit 0.7, en is pre-1.0.
- **Het pop-out kijkvenster verandert niet.** Eigen Win32-venster, eigen D3D11-swapchain;
  het hete videopad raakt de UI-stack nog steeds nergens.

### Historie: Tauri v2 (2026-08-04 tot 2026-10-04)

**Rust-kern + Tauri v2-weergave.** Besloten én uitgevoerd 2026-08-04, ter vervanging van
egui/eframe. Zie `ROADMAP.md`, fase 12, en `docs/OVERDRACHT.md`, beslissing 19 en 20.

Wat gebleven is: **de vijf niet-UI-crates onaangeroerd** — `proto`, `store`, `net`,
`audio`, `video`. Gemeten vóór het besluit en opnieuw erna: buiten `crates/app/src/ui/`
komt egui daar alleen voor in *doc-commentaar*; geen enkele API-aanroep. De hele
wire-protocol-, sync-, voice- en screensharelaag stond en staat buiten deze wissel.

Wat vervangen is: `crates/app/src/ui/` (was 13 egui-modules, 2.979 regels; nu drie
Rust-modules plus `crates/app/frontend/`) en de vensterbootstrap in `main.rs`.

Waarom dit goedkoop kon: de UI was al een pure weergave die een `Snapshot` leest en
`UiCommand`'s terugstuurt over kanalen. Die grens is Tauri-commands en -events geworden en
staat er nog: `ui/state.rs` vertaalt de `Snapshot` naar JSON, `ui/commands.rs` vertaalt
IPC-aanroepen terug naar `UiCommand`. **Die grens is de reden dat dit haalbaar was; hem
opgeven maakt een volgende stackwissel weer duur.**

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
- **Een tweede taal in de repo.** De verwachte frontend-bouwstap is er níét gekomen: de
  frontend is gewone HTML, CSS en JS zonder bundler, en `tauri-build` bakt de map in de
  exe. `cargo build` bouwt dus nog steeds de hele app en er is geen Node nodig.
- **Geheugen.** In rust gemeten op de dev-PC: 381 MB werkset voor het hele procesboompje
  (de app plus zes WebView2-processen), tegen tientallen MB's met egui. Processortijd is
  juist omlaag gegaan (31 ms over 60 seconden, 0,05% van één kern), en invariant 4 gaat
  over wat een game merkt — dat is de processor, niet het werkgeheugen.
- **De miniaturenstrook heeft een nieuw transport gekregen.** Was een BGRA-downscale
  rechtstreeks in een egui-textuur; is nu een `thumbnail`-event met een revisienummer plus
  een `thumb://`-protocol dat de PNG serveert aan een gewone `<img>`. Bij 2 fps en een
  kleine afmeting verwaarloosbaar, maar het was werk dat eerst gratis was.
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

- Windows 11, NVIDIA Turing of nieuwer, 1 Gbit/s symmetrisch. Tailscale hoeft niet meer
  (fase 16): de vrienden installeren geen extra app en zetten geen poorten open, ze plakken één
  keer elkaars uitnodigingscode.
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
signaling, geen STUN of TURN, geen relay, geen database, geen cloud-API, geen accounts. De
pc's vinden elkaar met een uitnodigingscode die je één keer plakt; de eigen router zet de poorten
open, en wie dat niet kan wordt via een andere peer gevonden. Tailscale mag, maar is geen
afhankelijkheid meer. Elke instantie initieert én
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

- QUIC-mesh op publieke adressen of het tailnet (chat, control, bestanden), versleutelde UDP
  over één mediasocket voor audio/video. Auto-reconnect met exponentiële backoff; alleen peers
  met een vastgepinde sleutel (uit hun code, of geleerd over het tailnet) komen binnen.
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
  krijgen. Dat gold voor egui, gold voor de webview, en geldt onverkort voor GPUI.
- **H.264 is de standaardcodec, niet HEVC** (HEVC-decode hangt op Windows aan een
  Store-uitbreiding die er niet standaard op zit). **AV1 valt af** zolang de RTX 2080 Super
  meedoet — die kan het niet encoden en niet decoden. **4:4:4 is afgewezen, niet uitgesteld:**
  geen Turing-GPU kan het hardwarematig decoderen.
- Er is nog geen instellingenscherm voor alles: peer-adressen, autostart, download- en
  picturesmap staan in `config.toml`. Video-instellingen, gebruikersnaam en
  microfoon/weergave zitten wél in het Instellingen-venster.

### Beslist in fase 12

- **UI-taal: Engels.** Voorgelegd aan Rick bij het begin van fase 12 en zonder beperking
  goedgekeurd. Uitgevoerd voor alles wat die fase herschreef: de zichtbare strings (die
  stonden al zo in de comp) én de Rust-identifiers en JSON-veldnamen in
  `crates/app/src/ui/`. De vertaling zit op één plek, `ui/state.rs`. De motor en de vier
  andere crates zijn **niet** hernoemd — dat viel buiten de fase, en een repo-brede
  hernoeming van `deler`, `kijker`, `venster` en `Actie` is toegestaan maar is eigen werk
  met een eigen commit. De docs blijven Nederlands: die zijn een gesprek, geen API. Er is
  geen vertaallaag en i18n is nog steeds geen doel. Zie `docs/OVERDRACHT.md`,
  beslissing 20.

## Brand Commitments

- Naam: **FitCommunication**, binary `fitcom.exe`.
- **Donker is een eis, geen stijlkeuze.** Zie `## Accessibility & Inclusion`: de app wordt
  bijna altijd 's avonds in het donker gebruikt. Geen lichte modus, geen themawisselaar,
  geen OS-thema-navolging.
- Toon: nuchter en zonder marketingstem. De bestaande UI-teksten en foutmeldingen zeggen
  wat er aan de hand is en wat je eraan doet ("offline · peer reageert niet"). Dat blijft.
### De visuele wereld: het zestienkleurenveld

> **Vervangen op 2026-10-20 door B, "Black chrome"** (`docs/OVERDRACHT.md` beslissing 48,
> `DESIGN.md`). Wat hieronder staat over dither, horizonband, DotGothic16 en inversie is
> historie; de informatiestructuur en het faalcriterium gelden nog.

**Vastgelegd 2026-08-12, ter vervanging van de keuze van 2026-08-04.** Rick vroeg om een
frissere, eigenzinnigere uitstraling en er is opnieuw een richtingsronde gedraaid (seed
`c9bbaaa4`). Uit die ronde koos Rick de catalogus-uitdager **"PC-98 sixteen-color field"**
— het Japanse zestienkleuren-computerscherm van begin jaren negentig — boven de toegewezen
eigen richting (tactische squad-HUD) en boven het aanhouden van de categoriestandaard van
2026-08-04 (die als vaste terugvaloptie op tafel lag).

Wat vastligt en wat toekomstig werk zo behandelt:

- **De informatiestructuur blijft staan**, op Ricks expliciete keuze: icoonrail →
  kanaal/DM-lijst → tijdlijn → ledenlijst. Wie van Discord komt bedient het nog blind.
  Donker blijft een eis. Het faalcriterium dat Rick voor deze ronde koos: **"moeilijker
  af te lezen is een fout"** — leesbaarheid in een donkere kamer wint van elke vondst.
- **De wereld zelf staat in `DESIGN.md`**, opgeschreven uit de gebouwde frontend:
  zestien vaste kleuren op diep indigo, elke tussentint een geordende dither van twee
  paletkleuren, papieren tekst, blauwe titelbalken op vierkante vensters, selectie die
  zijn cellen inverteert, magenta als dé ene primaire-actievulling, DotGothic16 als
  bitmapletter (lokaal gebundeld), harde offset-schaduwen, motion die klikt in `steps()`.
- **De kwaliteitslat is dubbel:** de catalogkaart van de wereld voor de visuele
  toewijding, Discord en Slack voor de gedragsafwerking (toetsenbord, staten, dichtheid).
- **De oude wereld is anti-referentie geworden.** De comp `design/main-window.html` en
  `design/shots/` documenteren de vervangen "Well-Made Standard" van 2026-08-04 en zijn
  geen reproductiedoel meer; waarden daaruit terugkopiëren (de teal `#2FB3AE`, de zes
  grijze lagen, afgeronde hoeken) is een fout, geen huisstijl.

`DESIGN.md` is aan het eind van deze bouw herschreven uit de gebouwde wereld, na een
finish-review met eindoordeel *ship*; `.impeccable/design.json` is meegegenereerd.

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
- **De GPUI-weergave (2026-10-04) is alleen op macOS bekeken**, in de fixturemodus en met
  headless tests. Op Windows heeft nog niemand hem gestart; de bevestiging hierboven ging
  over de Tauri-build. Zie `docs/OVERDRACHT.md`, beslissing 39.
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
