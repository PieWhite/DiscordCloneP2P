# Overdracht — stand van zaken

Bedoeld om in een nieuwe sessie snel weer op snelheid te komen. Wat er staat, waarom
het zo staat, waar ik tegenaan gelopen ben, en wat er nog moet.

Laatst bijgewerkt: 2026-08-20. Alle geplande fasen uit `ROADMAP.md` zijn af (t/m fase 12,
de UI-stack van egui naar Tauri v2).

**Wat er op 2026-08-20 bijkwam:** bestanden openen vanuit de chat en YouTube-previews
(beslissing 29 en 30), en daarna een **Wordle van de dag** met een scorebord —
beslissing 31, en de derde bewuste uitzondering op invariant 1. Lees die vóór je aan
`crates/app/src/wordle.rs`, de kaart in de tijdlijn of de puntenregel iets verandert.

**Wat er daarvoor gebeurd was:** de **camera** erbij (Windows; op macOS bewust overgeslagen,
zie beslissing 22 en `TODO.md`) — een derde `BronSoort` door de bestaande deelketen, dus
tegelijk met een gedeeld scherm. Daarvoor is de app **geport naar macOS** (14+, Apple
Silicon) — zelfde codebase, zelfde protocol 5, volledige featurepariteit behalve
P2P-auto-update en camera-opname. Lees beslissing 21 hieronder vóór je iets aan de
platformlaag doet. De vorige mijlpaal (de
weergavelaag in Tauri v2, naar de goedgekeurde comp `design/main-window.html`) staat in
beslissing 19 en 20; er is geen egui meer in de repo behalve in doc-commentaar.

**Waar de UI nu staat:**

```
crates/app/src/ui/mod.rs       Vensterbootstrap, tray, de drie events, thumb://-protocol
crates/app/src/ui/state.rs     Snapshot → JSON (de ene plek waar Nederlands Engels wordt)
crates/app/src/ui/commands.rs  IPC-aanroep → UiCommand
crates/app/frontend/           index.html + app.css + app.js + fonts/ (in de exe gebakken)
```

---

## Status per fase

| Fase | Status | Bewezen door |
|---|---|---|
| 0 — Scaffolding | ✅ af | draait |
| 1 — Netwerklaag | ✅ af | **echt getest tussen twee PC's over Tailscale** |
| 2 — Tekstchat | ✅ af, **bevestigd met een echte peer** | 19 unit/integratietests, 3 lokale instanties, `docs/TESTPLAN.md` 2.1 t/m 2.9 |
| 3 — Voice | ✅ af, **bevestigd met een echte peer** | ketentests + rooktest op echte geluidskaart, `docs/TESTPLAN.md` 3.1 t/m 3.10 |
| 4 — Screenshare | ✅ af, **bevestigd met een echte peer** | volledige keten op echte GPU, 55 fps op 1080p, `docs/TESTPLAN.md` 4.1 t/m 4.12 |
| 5 — Screenshare uitbreiding | ✅ af, **bevestigd met een echte peer** | ketentest + motortest blijven groen na de wijziging, `docs/TESTPLAN.md` 5.1 t/m 5.4 |
| 6 — Bestandsdeling | ✅ af, **bevestigd met een echte peer** | `crates/app/tests/file_deling.rs`: volledige overdracht + hash door de echte motor heen, geen GPU nodig; `docs/TESTPLAN.md` 6.1 t/m 6.7 |
| Kanalen (DM's), na fase 6 | ✅ af, **bevestigd met een echte peer** | `crates/app/tests/chat_sync.rs` (drie peers, volledige mesh, echte QUIC) + `crates/store/tests/convergentie.rs` (kanaal-scoping op store-niveau); `docs/TESTPLAN.md` K.1 t/m K.6 |
| 7 — Tags, meldingen, niet storen, gebruikersnaam | ✅ af, **bevestigd met een echte peer** | `crates/app/src/tags.rs` (unit-tests op de tag-herkenning en cursor-parsing); `docs/TESTPLAN.md` 7.1 t/m 7.7 |
| 8 — Chat verrijking: bestanden inline, plakken, links | ✅ af, grotendeels bevestigd (Ctrl+V-plakken bevestigd, zie beslissing 15) | volledige testsuite blijft groen (o.a. `crates/app/tests/file_deling.rs`, `crates/store/src/timeline.rs`); slepen-en-neerzetten, de miniatuur in de tijdlijn en het Instellingenscherm nog niet met de hand bekeken |
| 9 — Algemeen: subkanalen met een eigen titel | ✅ af, nog niet met een echte peer getest | volledige testsuite blijft groen (nieuwe tests in `crates/proto`, `crates/store`, `crates/app/src/ui.rs`) + `protocol-reviewer`-agent vóór het committen + twee lokale instanties starten en verbinden schoon; zie `docs/TESTPLAN.md`, fase 9 |
| 10 — Resoluties, bitrate, gecombineerd delen | ✅ af, **nog niet met echte hardware getest** | bugfix heeft een regressietest (`crates/app/src/streams.rs`); resolutie bleek al parametrisch (audit, geen codewijziging); bitrate is een configwaarde; wasapi-exclude-route + terugval compileert en start schoon in twee lokale instanties, maar het capturen/uitsluiten zelf kan alleen met echte speakers/koptelefoon gecontroleerd worden — zie `docs/TESTPLAN.md`, fase 10 |
| 11 — Automatische updates tussen peers | ✅ af, **nog niet met een echt versieverschil getest** | volledige testsuite blijft groen (nieuwe tests in `crates/proto`: `Hello`-veld, `is_newer`, `UpdateRequest`/`UpdateResponse`-roundtrip; `crates/app/src/updates.rs`: 12 unit-tests op de pure beslislogica) + `protocol-reviewer`-agent vóór het committen (protocolversie 3→4) + twee lokale instanties starten en verbinden schoon; een echt versieverschil, het bevestigingsvenster en het toepassen door `fitcom-updater.exe` kan alleen met een tweede, oudere build getest worden — zie `docs/TESTPLAN.md`, fase 11 |
| 12 — UI-stack naar Tauri v2 | ✅ af, **de media- en sleep/plak-paden nog niet met echte hardware getest** | volledige testsuite blijft groen (de vier `hoort_bij_kanaal`-tests zijn meeverhuisd naar `ui/state.rs` als `belongs_to_channel`); app gebouwd, gestart en gescreenshot tegen `design/shots/` op 1440×900; `cargo clippy --all-targets` schoon; idle-CPU gemeten, zie fase 12 in `ROADMAP.md` |

**Fase 5 was kleiner dan gepland.** Venster-capture, meerdere bronnen tegelijk delen en
meerdere inkomende streams tegelijk bekijken bleken al in fase 4 meegebouwd — zie
TESTPLAN 4.7/4.8. Er kwamen twee dingen bij: een instellingenscherm voor codec/fps/bitrate
in de UI (`ZetVideoInstellingen`, herstart lopende delers meteen), en een overzichtstrook
boven de chat met een levend verkleind beeld per bekeken stream (`Miniatuur`, elke 500 ms
via een gerichte GPU-naar-CPU-downscale op het al gedecodeerde beeld —
`D3dContext::lees_bgra_miniatuur`, niet het eigenlijke weergavepad). De geplande optionele
4:4:4-modus is geschrapt, zie de omgegooide beslissingen hieronder.

**Beeld werkt van begin tot eind**: een bron aankondigen, intekenen, opnemen, coderen,
versturen, samenstellen, decoderen en tonen. Gemeten op deze machine: 1080p op 55-56
beelden per seconde, nul beelden onderweg kwijt, 3,1 ms tussen opnemen en tonen in een
debug-build.

**Desktop-audio werkt ook**: het geluid van je PC gaat mee als eigen stream, met bij de
luisteraar een volumeschuif los van je stem.

Fase 4 is daarmee compleet op wat alleen met een tweede machine te controleren valt.
Zie `docs/TESTPLAN.md`.

**Fase 6 is het hoofdbacklog-item: bestanden delen.** Aanbieden is een gewone oplog-op
(`OpKind::FileMeta`) en verspreidt zich dus gratis mee via de bestaande sync, ook naar een
peer die pas later online komt. Downloaden gaat punt-naar-punt met de aanbieder over een
**eigen QUIC-uni-stream** naast de control-stream — nooit erover, want dat zou chat en
screenshare-signalering laten wachten op een bulkoverdracht. Hervatten na onderbreking
werkt via een hervatpunt dat de aanvrager meestuurt; verificatie is een BLAKE3-hash over
het hele bestand na afloop, met weggooien-en-opnieuw bij een mismatch. Zie "Hoe
bestandsdeling in elkaar zit" verderop en `docs/ARCHITECTURE.md` voor het volledige
ontwerp, inclusief waarom het anders is dan TODO.md's oorspronkelijke schets.

---

## Waar wat staat

```
crates/proto/   Wire-protocol. Puur, geen I/O. ControlMsg, Op, VersionVector, mediaheader.
crates/store/   SQLite-oplog, timeline-opbouw, sync-berekening. Puur, geen Windows.
crates/net/     QUIC-mesh (async, tokio) + MediaSocket voor UDP (blokkerend, geen tokio).
crates/audio/   Voice: jitterbuffer, mixer, Opus, cpal-sessie.
crates/video/   Screenshare: fragmentatie, D3D11, WGC-capture, MF-encoder en -decoder,
                kleuromzetting, weergavevenster, deler- en kijker-thread.
crates/app/     lib + binary. engine.rs is de motor; streams.rs beslist over
                screenshare; ui/ is een pure weergave (Tauri v2 + frontend/).
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

### 5. Het kijkvenster heeft wél een rand (fase 4)
De ARCHITECTURE zei "borderless". Dat is in de praktijk de verkeerde keuze: een venster
zonder rand kun je niet verplaatsen, niet vergroten en niet sluiten zonder dat je dat
allemaal zelf nabouwt met hit-testing. Wat er bedoeld werd — geen chroom *over* het
beeld, geen egui eromheen, een eigen swapchain — geldt onverkort.

Beeldvullend zit op F11 en dubbelklik, en dán is hij randloos. Zonder modeswitch, want
er kan een game op datzelfde scherm draaien.

### 6. Elke kijker bindt zijn eigen UDP-poort (fase 4)
Video kan niet over de voice-poort: die is bezet zodra je in een gesprek zit. De kijker
bindt daarom per stream een eigen poort en zet die in zijn `StreamSubscribe` — precies
waar dat veld voor bedoeld was. Gevolg: geen demultiplexen, geen gedeelde socket, en één
thread per bekeken stream die zijn eigen socket, decoder en venster bezit.

### 7. Eén tijdklok per proces in plaats van per deler (fase 4)
De tijdstempels op de draad hingen eerst aan een klok per deel-thread. Nu aan één klok
per proces. Dat maakt de tijdstempels van al je streams onderling vergelijkbaar, en het
maakt de vertraging van de hele keten meetbaar zodra deler en kijker in hetzelfde proces
draaien — wat in `crates/video/tests/keten.rs` het geval is.

**Tussen twee machines zegt dat getal niets**: die klokken lopen niet gelijk. Daarom
staat het nergens in de UI.

### 8. Geen 4:4:4-modus (fase 5)
De SPEC en ROADMAP noemden 4:4:4 als latere optionele toggle "voor scherpere tekst",
met de aanname dat alle drie de peers het zouden ondersteunen. Uitgezocht vóór er iets
gebouwd werd, net als bij de HEVC-beslissing in fase 4: NVIDIA's eigen GPU-supportmatrix
zet H.264-4:4:4- én HEVC-4:4:4-*decode* op "nee" voor de hele Turing-generatie, dus ook
voor de RTX 2080 Super. Encoderen naar 4:4:4 kan wél (`eAVEncH264VProfile_444` bestaat
als gedocumenteerd Codec-API-profiel), maar zonder dat iemand het kan terugzien is dat
zinloos. Anders dan bij HEVC-decode (die hangt aan een ontbrekende Store-uitbreiding,
dus is in theorie op te lossen) zit hier geen hardwarepad achter — geen enkele
software-oplossing daarvoor die niet op zichzelf al een probleem zou zijn
(CPU-belasting naast een game, of een GPU→CPU→GPU-omweg). Zie `TODO.md` voor waar dit
staat als "afgewezen, niet uitgesteld".

### 9. Geen FileOffer, geen FileChunkAck, geen los offered_by-veld (fase 6)
TODO.md schetste `FileOffer`/`FileAccept`/`FileChunkAck` als gereserveerde
control-berichten en `FileMeta { name, size, hash, offered_by }` als op. Bij het bouwen
bleken twee van de drie berichten overbodig en het veld redundant:

- **Geen `FileOffer`.** Het aanbod ís de `FileMeta`-op zelf; die synchroniseert al gratis
  mee via de bestaande version-vector-sync. Een apart broadcast-bericht ernaast zou een
  tweede, overbodig verspreidingspad zijn.
- **Geen `FileChunkAck`.** De bytes gaan over een betrouwbare, geordende QUIC-stream
  (`conn.open_uni()`), niet over UDP zoals media. Er is dus geen pakketverlies om tegen
  te beschermen, en dus niets om per chunk te bevestigen. Hervatten na onderbreking werkt
  met één getal (`FileRequest.have_bytes`), niet met chunk-boekhouding.
- **Geen `offered_by`.** `op.author` van de `FileMeta`-op ís de aanbieder — een apart veld
  zou alleen kunnen gaan liegen, precies zoals `Edit`/`Delete` hun eigenaarschap ook via
  `op.author`/`target.author` regelen in plaats van een los veld.

`FileAccept` heet in de code `FileRequest` (dezelfde rol, andere naam) en kreeg een
antwoord terug (`FileResponse` met `FileOutcome::READY`/`NOT_AVAILABLE`) dat TODO.md niet
noemde — nodig omdat de aanbieder pas ten tijde van de aanvraag weet of het bronbestand
nog bestaat. `FileOutcome` is net als `StreamKind` een getagd `u8` in plaats van een kale
enum, zodat een latere derde uitkomst een oudere peer niet laat struikelen over de hele
`FileResponse`. Zie `docs/ARCHITECTURE.md` (sectie "Bestandsdeling") voor het volledige
ontwerp.

### 10. DM's krijgen geen doorstuurhulp via een derde peer (kanalen, na fase 6)
TODO.md boekte "meerdere chatkanalen" af als bewust uitgesteld. Op verzoek van Rick
alsnog opgepakt, maar pas na een expliciet gesprek over één ontwerpkeuze: profiteert een
DM van dezelfde doorstuur-/hersync-robuustheid als het algemene kanaal (punt 3 onder
"Drie wegen waarlangs een op zich verspreidt" in `docs/ARCHITECTURE.md`)?

Antwoord: nee, bewust niet. Er is geen encryptie van de opinhoud — alleen QUIC-transport
en het tailnet als vertrouwensgrens. Zou een derde peer een DM tussen twee anderen ooit
doorsturen, dan kán hij de inhoud gewoon lezen. Dat conflicteert direct met wat een DM
belooft. DM's synchroniseren daarom uitsluitend rechtstreeks tussen de twee betrokkenen;
in een normale full-mesh (alle drie online) merk je daar niets van, en alleen bij
gedeeltelijke connectiviteit tussen precies de twee DM-partners mis je het vangnet dat
het algemene kanaal wel heeft.

Technische consequentie, geen keuze: `seq` moest van "per auteur" naar "per (auteur,
kanaal)" — anders loopt een buitenstaander een permanent gat op bij een DM die hij nooit
mag ontvangen, en blokkeert dat gat zijn hele reeks voor die auteur, óók voor latere
algemene berichten. Zie `docs/ARCHITECTURE.md`, sectie "Kanalen", voor de volledige
redenering en `crates/store/tests/convergentie.rs::een_dm_blokkeert_daarna_het_algemene_kanaal_niet_voor_een_buitenstaander`
voor de test die dit vastlegt.

**`PROTOCOL_VERSION` ging van 1 naar 2.** Eerste versie liet dit ongemoeid in de aanname
dat de wijziging additief was, maar dat bleek niet zo: `VersionVector` codeerde zijn
regels als rauwe tuples, en msgpack codeert een tuple altijd als vaste-lengte array —
een 2-tuple en een 3-tuple zijn dus niet over en weer decodeerbaar, in tegenstelling tot
elke andere struct in dit protocol (die als map gecodeerd worden en dus wél
`#[serde(default)]` kunnen gebruiken). Zonder versiebump zou een oude en een nieuwe peer
elkaars `SyncRequest` stilzwijgend niet meer kunnen lezen — geen crash, gewoon een
warn-regel in het log en een chat die nooit meer synchroniseert. Gefixt door (a) de
versie op te hogen, zodat de bestaande `VersionMismatch`-afhandeling dit nu netjes meldt,
en (b) de tuple sowieso te vervangen door een benoemde struct (`VvEntry`), zodat een
volgende uitbreiding hier niet opnieuw tegenaan loopt. Gevonden door de
`protocol-reviewer`-agent, niet door een test — zie hieronder.

### 11. Een DM meldt zich niet vanzelf — alleen bij een expliciete tag (fase 7)
De ROADMAP liet in het midden of een DM een Windows-melding waard is zonder dat de tekst
letterlijk `@jouwnaam` bevat — een DM is immers al aan jou persoonlijk gericht. Voorgelegd
aan Rick vóór het bouwen, met als aanbeveling "DM's melden altijd". Rick koos het
tegenovergestelde: **ook een DM meldt zich alleen bij een expliciete tag**, dezelfde regel
als het algemene kanaal. Geen uitzondering voor DM's ingebouwd.

Gevolg voor de implementatie: `Engine::overweeg_melding` (`crates/app/src/engine.rs`) kijkt
nooit naar `Channel::is_general()` om te beslissen of er gemeld wordt — alleen naar
`tags::bevat_tag`. Het kanaal is uitsluitend nog relevant om te bepalen welke
"ongelezen"-teller (algemeen of per DM-partner) gebruikt wordt om vast te stellen of een
op daadwerkelijk nieuw was, niet om de meldingsbeslissing zelf.

### 12. Content-adresseerbare afbeeldingen in plaats van een sessie-lokale padcache (fase 8)
ROADMAP.md vroeg om "een miniatuurweergave in plaats van een generieke bestandskaart" bij
Ctrl+V-plakken. Eerste versie loste dat te bekrompen op: een `App::eigen_afbeeldingen`
(`HashMap<naam, pad>`) die alleen het lokale pad van je **eigen** aanbod onthield, met als
onderbouwing dat een gedownload bestand pas ná hash-verificatie zijn leesbare naam krijgt
(met `" (2)"` bij een botsing), dus de UI vooraf nooit het exacte pad van een ontvangen
afbeelding zou kennen. Een ontvangen afbeelding kreeg daardoor nooit een miniatuur, ook niet
na downloaden — een asymmetrie die Rick terecht niet begreep en die ook niet hoefde.

Rick's eigen voorstel loste het echt op: gebruik niet een sessie-lokale boekhouding, maar
de **inhoudshash die er al lag** (`FileMeta.hash`, ook gebruikt voor verificatie). Die ligt
al vóór het downloaden vast en is bij aanbieder en ontvanger identiek — in tegenstelling tot
een losse randomizer, die bij elke peer onafhankelijk een andere waarde zou opleveren en dus
niets zou oplossen. Elke afbeelding (aangeboden of gedownload) landt daarom nu onder
`<hex(hash)>.<extensie>` in een eigen map (`pictures_dir`; sinds beslissing 32
`<downloadmap>/Pictures`, daarvóór `<datamap>/Pictures`),
apart van de gewone downloadmap:

- **Aanbieden:** `hash_en_bied_aan` (`engine.rs`) kopieert, ná het hashen, zelf een kopie
  naar `pictures_dir`. Het origineel (het bestand van de gebruiker, ergens anders op
  schijf) blijft ongemoeid.
- **Downloaden:** `download_bytes` (`engine.rs`) hernoemt na een geslaagde
  hash-verificatie naar `pictures_dir` in plaats van naar de gebruikelijke
  `unieke_bestandsnaam` in `download_dir`.
- **Weergave:** `ui.rs` berekent hetzelfde pad uit `FileView.hash`/`.name` (nu een gedeelde
  pure functie, `files::hash_bestandsnaam`) en probeert simpelweg te laden. Staat het
  bestand er nog niet, dan faalt dat geruisloos en toont de kaart de generieke weergave —
  precies zoals eerst, maar nu voor **beide** kanten in plaats van alleen de aanbieder.

`App::eigen_afbeeldingen` is daarmee volledig vervallen: er is geen sessie-lokale
boekhouding meer nodig, het pad is overal deterministisch af te leiden. Zie
`docs/ARCHITECTURE.md`, sectie "Bestandsdeling", voor het volledige ontwerp.

**Terzijde, in dezelfde ronde:** Ctrl+V-plakken werkte niet, gemeld door Rick. Eerste gok was
dat de check aan `output.response.has_focus()` vastzat — losgemaakt van focus, met alleen een
uitzondering als er een ander modaal venster open staat. Dat was nodig maar niet voldoende;
de echte oorzaak zat dieper en staat in beslissing 15.

### 13. Een bestand verwijderen moest ook echt stoppen met serveren (fase 8)
Rick vroeg dat een zelf aangeboden bestand of foto net als een bericht te verwijderen moet
zijn. `OpKind::Delete { target: OpId }` bleek daar al generiek genoeg voor — `target` maakt
geen onderscheid tussen "een bericht" en "een bestandsaanbod", dus `crates/store/src/timeline.rs::build()`
hoefde alleen dezelfde `changes`-toepassing die al voor `messages` bestond, ook op `files`
toe te passen. Zelfde regel (alleen de auteur van het doel, alleen binnen hetzelfde kanaal),
geen protocolwijziging, geen `protocol_version`-bump — bevestigd door de
`protocol-reviewer`-agent vóór het committen.

Die agent vond wel een echt gat, niet in `store` maar in `crates/app`: de Delete-op liet
alleen de kaart uit de timeline verdwijnen. `Files::aangeboden` (welk lokaal pad bij welke
`OpId` hoort) werd nergens opgeschoond, dus kon een peer die de `OpId` al kende het bestand
na "verwijderen" gewoon nog steeds downloaden — schijnzekerheid in plaats van een echte
intrekking. Gefixt met `Files::verwijder_aanbod` (`crates/app/src/files.rs`), aangeroepen
vanuit `UiCommand::Verwijder` in `engine.rs` naast de bestaande `chat.verwijder_bericht`-aanroep.
Een no-op als het doel geen eigen bestandsaanbod is (bijvoorbeeld een gewoon bericht), dus
geen aparte tak nodig om te onderscheiden wat er verwijderd wordt.

**Nog steeds geen volledige intrekking**, en dat kán ook niet zonder een vertrouwensmodel
dat verder gaat dan dit tailnet biedt: een download die al liep op het moment van
verwijderen loopt gewoon af, en een peer die de bytes al eerder volledig binnenhad houdt
zijn eigen kopie. Zie `docs/ARCHITECTURE.md`, sectie "Bestandsdeling", voor de volledige
uitleg.

### 14. Eén algemeen instellingenscherm in plaats van los "video-instellingen" (fase 8)
De nieuwe knop "Verwijder alle afbeeldingen" (met bevestigingsvraag, want onomkeerbaar)
had geen voor de hand liggende plek: er was alleen een video-specifiek instellingenscherm
en het profielvenster, geen algemeen scherm. Voorgelegd aan Rick met drie opties (nieuw
algemeen scherm, bijplakken in video-instellingen, of een losse knop in de statusbalk).
Rick koos voor een nieuw algemeen "Instellingen"-scherm, met de bestaande
video-instellingen als eerste sectie erin en "Afbeeldingen" (de nieuwe knop) als tweede.
De statusbalk-knop heet nu "instellingen" in plaats van "video-instellingen".
Niet-storen en naam wijzigen zijn bewust blijven staan waar ze al stonden
(deelnemerspaneel) — dat zijn live bedieningen, geen instellingen, en waren geen deel van
de vraag.

### 15. Ctrl+V via `GetAsyncKeyState`, niet via egui's eigen toetsenbordevents (fase 8)

> **Vervallen in fase 12 (beslissing 19), de code is weg.** Deze hele omweg bestond alleen
> omdat `egui-winit` de plakopdracht opslokte vóórdat de app hem zag; de webview krijgt een
> echt `paste`-event met de afbeelding erin, dat via `offer_pasted_image` in dezelfde
> aanbiedflow terechtkomt als een gesleept of gekozen bestand. Blijft hier staan omdat de
> onderliggende les — welke laag het OS-klembord bezit, en dat een logbestand dat sneller
> aanwijst dan raden — nog steeds geldt. Niet terughalen.

De focus-fix uit beslissing 12 loste Ctrl+V niet op — Rick meldde dat het nog steeds niet
werkte. Het logbestand (met `FITCOM_LOG=debug`) liet zien dat `egui_winit::clipboard` zelf
een `arboard paste error` gooide bij elke Ctrl+V, maar geen enkele eigen debug-regel van
`plak_afbeelding` verscheen — dus mijn eigen check werd nooit bereikt, ook niet nadat de
focus-eis weg was.

De oorzaak lag in `egui-winit` zelf (`lib.rs`, `is_paste_command`): zodra het een
toetsaanslag herkent als de OS-plakopdracht (Ctrl+V op Windows), leest het zelf de
klembordtekst, voegt hoogstens een `Event::Paste(tekst)` toe, en **stuurt daarna nooit een
gewone `Key::V`-toetsaanslag door** (een vroege `return` in de match). Bevat het klembord
alleen een afbeelding — geen tekst — dan komt er dus helemaal niets in `ctx.input()`
terecht: geen `Event::Paste` (leeg/mislukt) én geen `Key::V`-event om op te reageren.
`ui.input(|i| i.key_pressed(egui::Key::V))` kán in dat geval nooit `true` worden, ongeacht
focus. Dat is precies wat de `egui_winit::clipboard`-foutmeldingen in het logbestand ook
lieten zien: dat was `egui-winit`'s eigen, mislukte poging — niet de onze, want die
startte nooit.

Oplossing: `App::ctrl_v_zojuist_ingedrukt` vraagt de fysieke toetsstatus rechtstreeks op
bij Windows via `GetAsyncKeyState(VK_CONTROL)`/`GetAsyncKeyState(VK_V)`
(`Win32_UI_Input_KeyboardAndMouse`, een nieuwe feature op de al aanwezige `windows`-crate),
volledig langs egui's eigen event-vertaling. Met randdetectie (`App::ctrl_v_ingedrukt`
onthoudt vorige frame) zodat het niet elke frame opnieuw triggert zolang de toetsen
ingedrukt blijven. Bewust **wel** gebonden aan `ctx.input(|i| i.focused)`:
`GetAsyncKeyState` kijkt naar de fysieke toetsstatus ongeacht welk venster de OS-focus
heeft, dus zonder die check zou Ctrl+V in een andere toepassing hier ook een bestand
aanbieden.

**Door Rick bevestigd met een echte screenshot en het klembord van deze Windows-machine** —
de enige manier waarop dit te testen was, want dit hele probleem zat in hoe Windows en egui
onderling met het klembord omgaan, niet in iets dat een geautomatiseerde test kan simuleren.

### 16. Subkanalen onder het algemene kanaal in plaats van onder een DM (fase 9)

`ROADMAP.md` schetste fase 9 aanvankelijk als meerdere benoembare gesprekken **binnen één
DM** — "algemeen" en "project X" als sub-gesprekken met dezelfde ene peer. Rick draaide dit
tijdens het bouwen om: de subkanalen horen bij **het algemene kanaal**, niet bij een DM. Het
resultaat is dichter bij Discord-kanalen binnen één server dan bij sub-DM's — een
subkanaal is een extra, voor **iedereen** zichtbare gespreksstroom naast "Algemeen", geen
uitbreiding van een privégesprek tussen twee peers.

Technisch schelen de twee ontwerpen weinig: in beide gevallen krijgt `Channel` een derde
soort met een eigen identifier, en werkt `seq` per (auteur, kanaal) al generiek genoeg om
dat zonder verdere aanpassing te dragen. Het verschil zit in de zichtbaarheidsregel: een
DM-subkanaal zou de DM-beperking (alleen de twee betrokkenen, geen doorstuurhulp) moeten
erven, een subkanaal onder "Algemeen" juist niet — die moet zich in alles gedragen als het
algemene kanaal zelf. Vandaar `Channel::is_public()` (`tag == 0 || tag == 2`) als de ene
plek die overal bepaalt of iets zich als "algemeen" gedraagt, in plaats van het bestaande
`is_general()` overal aan te passen: `is_general()` blijft "is dit letterlijk het
hoofdkanaal", `is_public()` wordt "is dit voor iedereen zichtbaar en met doorstuurhulp".

**Protocolversie moest opnieuw omhoog (2 → 3)**, net als bij de DM-uitbreiding, maar om een
andere reden: niet omdat het wire-decoderen van de nieuwe `Channel`-tag een probleem was
(dat is een map, geen tuple, dus onschadelijk voor een oudere peer), maar omdat de lokale
opslag (`channel_to_blob` in `crates/store/src/lib.rs`, `encode_channel` in
`crates/net/src/filestream.rs`) een onbekende tag stilzwijgend op dezelfde sleutel als het
algemene kanaal zou aliasen — zie de bug hieronder, gevonden door de `protocol-reviewer`-
agent vóór het committen.

### 17. Bureaubladgeluid via `wasapi`-crate met terugval, niet via raw COM (fase 10)

Automatisch geluid meesturen bij scherm delen kan alleen zonder dat een peer zijn eigen
stem terugkrijgt — anders capturet de gewone `cpal`-loopback ook de eigen voice-weergave
van deze app. Vooraf uitgezocht (`media-research`-agent, niet aangenomen): Windows heeft
sinds build 20348 (in de praktijk Windows 11) een proces-exclusieve loopback-capture
(`AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS` met `PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE`),
geen Store-uitbreiding nodig, GPU-onafhankelijk.

Zelf de COM-completion-handler en `PROPVARIANT`-inpakking met raw WASAPI bouwen was niet
nodig: de **`wasapi`-crate** (0.23, MIT, hangt al af van dezelfde `windows` 0.62-lijn als
de rest van de workspace) heeft `AudioClient::new_application_loopback_client(pid,
include_tree)` kant-en-klaar, met `include_tree: false` als de exclude-modus — precies de
usecase uit het eigen voorbeeld van die crate (`examples/record_application.rs`).

Belangrijkste valkuil: `AudioClient`/`AudioCaptureClient`/`Handle` zijn niet `Send`. Alles
opzetten én gebruiken moet dus op dezelfde OS-thread — de nieuwe `wasapi_capture`-submodule
in `crates/audio/src/session.rs` doet dat door de hele opzet (COM-MTA, activeren, formaat,
eventhandle, captureclient, stream starten) pas ná het spawnen van de thread uit te voeren,
nooit ervoor.

**Niet gegarandeerd op elke installatie, dus met terugval, niet met een harde aanname.**
`bureaublad_lus` probeert de exclude-route eerst; lukt dat niet (oudere Windows-versie, of
een andere onbekende reden), dan valt hij terug op de bestaande `cpal`-loopback — het oude
gedrag, inclusief het eigen-stem-risico, als ondergrens in plaats van een crash of een
compleet uitgevallen functie. Beide routes leveren mono `f32`-samples op hetzelfde kanaal
af, dus de resample/encode/verstuur-lus erna (ongewijzigd) ziet geen verschil.

**Nog niet met echte hardware bevestigd** of de exclude-modus daadwerkelijk aanslaat op de
testmachines. Zie `docs/TESTPLAN.md`, fase 10.

### 18. Kind-byte op elke uni-stream in plaats van een sentinel-`OpId`, en `PROTOCOL_VERSION` opnieuw omhoog (fase 11)

Een update-overdracht heeft, anders dan een bestand, geen `OpId` om de inkomende
uni-stream aan te herkennen — er is geen `FileMeta`-op. Overwogen: een gereserveerde
sentinel-waarde in het bestaande `OpId`-veld (bijvoorbeeld een all-nul auteur). Verworpen
vóór er iets gebouwd werd: dat is precies het soort impliciete aliasing dat in fase 9 al
een echte bug opleverde (een onbekende kanaal-tag die stilzwijgend op dezelfde
opslagsleutel als het algemene kanaal terechtkwam, zie de bug hieronder). In plaats
daarvan een expliciet 1-byte kind vóór elke stream (`0` = bestand, `1` = update) —
duidelijk in plaats van toevallig, met dezelfde motivatie als destijds.

Dat kind-byte wijzigt het wire-formaat van de **bestaande** bestandsoverdracht, dus
`PROTOCOL_VERSION` ging opnieuw omhoog, van 3 naar 4 — derde keer dat dit gebeurt, zie
"Protocolversie: 1 → 2, 2 → 3, 3 → 4" in `docs/ARCHITECTURE.md`. Vooraf voorgelegd aan de
`protocol-reviewer`-agent (niet pas ná een gevonden bug, zoals bij de 2→3-bump): die
bevestigde dat de bump nodig en compleet is, en vond geen resterend pad waarop een oudere
peer het nieuwe byte verkeerd zou kunnen lezen — een peer op protocolversie 3 verstuurt
sowieso nooit een `UpdateRequest`, dus de twee kanten van deze functionaliteit vallen altijd
samen.

Het updater-procesje (`fitcom-updater.exe`) is bewust een tweede binary in hetzelfde
`fitcom`-package (`crates/app/src/bin/`) geworden, niet een nieuwe workspace-crate: cargo
pakt `src/bin/*.rs` vanzelf op, en een aparte crate voor iets dat alleen wacht, hernoemt en
herstart zou alleen extra `Cargo.toml`-boekhouding zijn zonder een echt voordeel.

### 19. UI-stack van egui naar Tauri v2, en visueel opnieuw beginnen (fase 12)

**Uitgevoerd op 2026-08-04**, dezelfde dag als het besluit. Wat hieronder staat is de
onderbouwing; hoe het geworden is staat onder "Hoe de weergavelaag in elkaar zit"
verderop en in `ROADMAP.md`, fase 12.

`docs/SPEC.md` legde "Rust + egui/eframe" vast, en dat was vier jaar techstack-keuzes lang
de goede afweging: alles in één proces, snel te bouwen, laag idle-verbruik. Wat er niet in
stond is dat het ontwerpplafond laag is. egui kan geen echte typografie, geen ritme, geen
gelaagdheid, geen animatie zonder handwerk. Voor een app die je elke avond openhebt is dat
op een gegeven moment de bindende beperking geworden, niet het netwerk of de codec.

**Waarom dit goedkoop kan.** Eerst gemeten, toen besloten. Buiten `crates/app/src/ui/` komt
egui in de hele workspace alleen voor in *doc-commentaar* — `video/venster.rs`,
`video/kijker.rs`, `app/engine.rs` en `app/tray.rs` noemen het, maar gebruiken geen enkele
API ervan. De echte egui-code:

| | Regels |
|---|---|
| `crates/app/src/ui/` (13 modules) | 2.979 |
| `main.rs` vensterbootstrap | ~40 |
| `proto`, `store`, `net`, `audio`, `video` | 0 |

Dat is geen geluk: de UI is sinds beslissing 3 een pure weergave die een `Snapshot` leest
en `UiCommand`'s terugstuurt over kanalen. Die grens wordt Tauri-commands en -events, wat
grotendeels een mechanische vertaling is. **Die grens intact houden is de prijs van een
volgende goedkope wissel.**

**Twee dingen die tegen de intuïtie in gaan.** Ten eerste: idle wordt *goedkoper*.
`ui/mod.rs` hertekent nu 4× per seconde in rust en 12,5× tijdens een gesprek
(`IDLE_REPAINT`/`VOICE_REPAINT`), omdat immediate-mode niet anders kan; een event-driven
weergave tekent alleen bij verandering. Dat werkt vóór invariant 4 ("gamen wint"), niet
ertegen — dit was een argument om te wisselen, geen concessie. Ten tweede: **beslissing 15
vervalt hiermee.** De `GetAsyncKeyState`-omweg voor Ctrl+V bestond uitsluitend omdat
`egui-winit` de plakopdracht opslokte vóórdat de app hem zag; een webview krijgt een echt
`paste`-event met de afbeelding erin. Ook de zelfgebouwde titelbalk (194 regels, het
egui-dichtste bestand in de repo) wordt in CSS een fractie daarvan.

**Overwogen en afgewezen.** Dioxus 0.7.10 — Rust van voor tot achter met echte CSS en één
taal in de repo, maar pre-1.0 en een veel dunner ecosysteem dan Tauri; voor een codebase
die verder stabiel is weegt volwassenheid zwaarder dan taaleenheid. Slint 1.17.1 — geen
webview, geen externe Windows-component, GPU-gerenderd en fors beter dan egui, maar een
eigen DSL in plaats van CSS, waarmee het plafond merkbaar lager blijft liggen.

**Wat het kost, expliciet, zodat dit later niet als vergissing gelezen wordt.**

- **WebView2 is een Windows-component die wij niet in de hand hebben** — dezelfde *soort*
  afhankelijkheid als de HEVC Video Extensions, en dát argument kostte HEVC zijn plek als
  standaardcodec. Het verschil: WebView2 Evergreen zit standaard in Windows 11 en alle drie
  de machines draaien Windows 11. Zwakkere versie van dat probleem, geen herhaling ervan.
  **Niet uitwijken naar de fixed-version runtime** (~180 MB): dat sloopt "losse exe in een
  zip", en dat is de hele distributiestrategie.
- **Een tweede taal in de repo** en een frontend-bouwstap; `cargo build` bouwt daarna niet
  meer de hele app.
- **De miniaturenstrook heeft een nieuw transport nodig.** Zie "Hoe screenshare in elkaar
  zit" verderop: de UI cachet nu een `egui::TextureHandle` per stream. Dat wordt een canvas
  gevoed via een event of custom protocol. Bij 2 fps verwaarloosbaar, maar nu gratis.
- **De sectie "Hoe chat-verrijking in elkaar zit" beschrijft de egui-implementatie** van
  plakken, slepen en de autocomplete in de chatbox. Die blijft correct tot de migratie en is
  daarna verslag, niet beschrijving. Bewust niet weggehaald: het probleem dat eronder zat
  (welke laag het klembord bezit) komt in een webview in andere vorm terug.

**Het pop-out kijkvenster verandert niet.** Zie `docs/ARCHITECTURE.md`: het argument voor een
eigen Win32-venster met eigen swapchain wordt met een webview sterker, niet zwakker.

**Visueel opnieuw uitvoeren, binnen de categoriestandaard.** Rick heeft er tegelijk voor
gekozen het bestaande ontwerp niet één-op-één over te zetten. In een richtingsronde daarna
(zeven afgeleide werelden, vier gepresenteerde kaarten) heeft hij bewust de staande uitgang
genomen: **de categoriestandaard, recht toe recht aan** — de Discord-indeling zoals iedereen
hem kent, met **Discord en Slack als kwaliteitslat**. De aangewezen richting (handbediende
telefooncentrale) en drie uitdagers (vertrekbord, Teletekst, Schiphol-signering) lagen er
volledig uitgewerkt naast; dit is dus een keuze, geen gebrek aan alternatief.

Gevolg voor de uitvoering: de conventie is de opdracht, zonder ironie en zonder
eigenzinnigheid die er alsnog in gesmokkeld wordt. `ui/theme.rs` is daarmee **geen
anti-referentie** — het is een vroege, laag-fidelity uitvoering van precies de richting die
nu gekozen is, met een te lage afwerking. Het palet zelf staat wel open: de teal `#3ABFC0`
en de vijf grijze lagen zijn geen vastgelegde waarden. Donker blíjft een eis — de app wordt
bijna altijd 's avonds in het donker gebruikt, dat is geen smaak. En de informatiestructuur
(icoonrail → kanaal/DM-lijst → tijdlijn → ledenlijst, plus eigen titelbalk en statusbalk)
blijft staan op Ricks expliciete keuze: die is uitgeprobeerd en je vindt alles blind.

Volledige onderbouwing en de productcontext eromheen staan in `PRODUCT.md` (nieuw, sectie
`## Stack`).

### 20. De UI-taal wordt Engels — maar alleen wat in fase 12 herschreven werd (fase 12)

`PRODUCT.md` had de UI-taal expliciet als onbeslist staan, met drie mogelijke omvangen:
alleen zichtbare strings, de nieuwe frontend erbij, of ook de Rust-identifiers en de docs.
Voorgelegd aan Rick bij het begin van fase 12, omdat de weergavelaag toch helemaal
opnieuw geschreven werd en het daar dus gratis was. **Rick koos Engels, zonder beperking.**

Wat dat concreet geworden is:

- **De hele weergavelaag is Engels.** Alle zichtbare strings (dat stond al zo in de comp),
  én de Rust-identifiers in `crates/app/src/ui/` — `UiState`, `belongs_to_channel`,
  `timeline_of`, `PeerState`, `Presence`. De JSON-veldnamen die de frontend leest ook.
- **De vertaling zit op één plek: `ui/state.rs`.** Daar wordt `Snapshot.ongelezen` tot
  `unread`, `eigen_streams` tot `own_streams`, `niet_storen` tot `do_not_disturb`. Eén
  bestand, niet uitgesmeerd.
- **De motor en de vier andere crates zijn níét hernoemd.** Dat is geen halfheid maar de
  harde randvoorwaarde van deze fase: `proto`, `store`, `net`, `audio` en `video` blijven
  onaangeroerd, en `engine.rs`/`streams.rs`/`files.rs` meebeslepen in een repo-brede
  hernoeming zou een diff van duizenden regels zijn die niets met de stackwissel te maken
  heeft en elke `git blame` op de subtiele sync-logica onleesbaar maakt.
- **De docs blijven Nederlands.** Ze zijn een gesprek met Rick, niet een API.

Een repo-brede hernoeming van de resterende Nederlandse identifiers is daarmee toegestaan
maar niet gedaan. Wie hem alsnog wil: dat is eigen werk met een eigen commit, niet iets om
en passant mee te nemen.

### 21. De macOS-port: cfg-siblingmodules, geen traits (2026-08-05)

De SPEC zei "Windows only". Rick vroeg om een volledige port naar macOS (en iOS; dat
laatste is na de analyse bewust uitgesteld — op iOS kan een app geen scherm van andere
apps opnemen, en zonder servers is er geen push, dus een iPhone-client kan alleen iets
betekenen terwijl hij open staat. Als iOS ooit komt is het een gereduceerde
kijk/luister/chat-client met eigen spikes vooraf).

Hoe de port in elkaar zit, en waarom zo:

- **Cfg-geselecteerde siblingmodules, geen trait-hiërarchie.** Elke Windows-module
  (`capture`, `codec`, `d3d`, `venster`) houdt naam en publieke API; macOS heeft
  twinbestanden onder `crates/video/src/mac/`, gekozen met `#[cfg]` + `#[path]` in
  `lib.rs`. Geen enkele Windows-regel is verplaatst of hernoemd; Windows-deps staan in
  `[target.'cfg(windows)'.dependencies]`. Windows-gedrag is daardoor byte-identiek.
- **Eén opaak frametype dicht het ene lek.** `ID3D11Texture2D` stond als parameter- en
  returntype in de publieke API van zes modules. `d3d::Beeld` (Windows: type-alias van
  de textuur; mac: houder om een IOSurface-gebackte `CVPixelBuffer`) laat `deler.rs`,
  `kijker.rs`, `fragment.rs` en heel `engine.rs` op beide platforms ongewijzigd
  compileren — de getunede pacing-, FEC- en weergaveklok-logica is niet aangeraakt.
- **De mac-videobackend:** ScreenCaptureKit voor opname (opsommen gaat synchroon via
  CoreGraphics, want `list_sources` is een synchroon Tauri-commando en
  `SCShareableContent` bestaat alleen async), VideoToolbox voor H.264
  (`VTCompressionSession`/`VTDecompressionSession`, synchroon gehouden met
  `CompleteFrames` en de callback-op-aanroepende-thread), en het kijkvenster is een
  `NSWindow` met `AVSampleBufferVideoRenderer` — geen regel Metal; de laag schaalt en
  letterboxt zelf en `contentAspectRatio` bewaakt de verhouding.
- **De Annex-B-brug is de wire-kritieke regel.** Media Foundation spreekt Annex-B
  (startcodes, SPS/PPS inline op keyframes); VideoToolbox spreekt AVCC. De mac-codec
  vertaalt beide richtingen. HEVC is op mac bewust niet geïmplementeerd; H.264 is de
  standaard en `kan_decoderen(Hevc)` zegt daar eerlijk `false`.
- **Kleurconversie en MF-bootstrap hebben geen mac-tegenhanger nodig**: de VT-decoder
  levert op verzoek rechtstreeks BGRA (`destinationImageBufferAttributes`), dus
  `kleur.rs` en `mf.rs` zijn Windows-interne details gebleven.
- **Bureaubladgeluid**: audio-only `SCStream` met `excludesCurrentProcessAudio`
  (macOS 14+) — hetzelfde doel als de proces-exclusieve WASAPI-loopback, zelfde
  threadcontract (`session.rs::sck_capture` naast `wasapi_capture`). Er is geen
  cpal-terugval: de loopback-truc (invoerstroom op een uitvoerapparaat) bestaat alleen
  op WASAPI.
- **Geen P2P-auto-update op mac.** Twee guards in `engine.rs`: mac haalt nooit een
  peer-exe binnen, en biedt de eigen binary nooit aan (`NOT_AVAILABLE` — bestaand,
  netjes afgehandeld antwoord). Zonder die tweede guard zou een Windows-peer die een
  "nieuwere" mac-versie ziet een Mach-O over zijn `fitcom.exe` heen zetten. Versies
  blijven per werkafspraak gelijk op; de mac bouwt uit de broncode.
- **Tray en meldingen:** de Win32-omweg (eigen pollingthread, want een verborgen
  venster pompt daar zijn events niet) is op macOS onnodig — de NSApplication-runloop
  pompt door. De mac-tray is Tauri's eigen tray-API op de main thread
  (`ui/mod.rs::mac_tray`); meldingen gaan via `osascript` (nul deps, respecteert
  Focus). `tray.rs` deelt alleen nog de afsluitvlag en het icoon.
- **Distributie:** `scripts/bundle-mac.sh` bouwt een ad-hoc-gesigneerde
  `FitCommunication.app` in een zip — de mac-tweeling van "losse exe in een zip",
  zonder tauri-cli of Node. De .app is nodig omdat TCC-permissies aan een
  bundel-identiteit plakken. **Prijs van ad-hoc:** elke nieuwe build heeft een andere
  cdhash, dus macOS vraagt Screen Recording na elke update opnieuw;
  Developer-ID-signing lost dat op en dat besluit is uitgesteld.
- **Bindings:** de objc2-familie 0.3.2 (objc2 0.6.4, block2 0.6.2, dispatch2 0.3.1),
  gepind na een spike. Twee valkuilen om te onthouden: `CVPixelBuffer` is in die
  bindings een type-alias van `CVBuffer`, en Rust 2021's disjuncte closure-capture kan
  een `Send`-newtype omzeilen door alleen het binnenveld te vangen — vandaar dat
  `Beeld` zijn veld privé houdt.
- **De ene echte bug uit de integratie:** SCK-tijdstempels zijn hosttijd (nanoseconden
  sinds opstarten); `value × 10^7` satureerde in i64, alle beelden kregen dezelfde
  tijdstempel en de weergaveklok moest elke twee seconden opnieuw ijken. In i128
  rekenen loste het op — gemeten daarna: 5,1 ms van opnemen tot tonen op loopback
  (3456×2234), 231 van 233 beelden getoond, 0 gesneuveld
  (`cargo run -p fitcom-video --example mac_keten`; als voorbeeldprogramma omdat het
  venster de main-runloop nodig heeft en de cargo-testharnas die niet pompt).

---

## Hoe de weergavelaag in elkaar zit

```
crates/app/src/ui/mod.rs       Vensterbootstrap (Tauri v2 + WebView2), tray-koppeling,
                               de drie events, en het thumb://-protocol.
crates/app/src/ui/state.rs     Snapshot → UiState (JSON). Puur, met eigen tests.
crates/app/src/ui/commands.rs  Eén #[tauri::command] per UiCommand-variant.
crates/app/frontend/           index.html, app.css, app.js, fonts/. Door tauri-build in
                               de exe gebakken; er staat geen dist-map naast fitcom.exe.
```

**Drie soorten verkeer, bewust gescheiden.** Dat is de kern van waarom dit in rust
goedkoper is dan egui, niet duurder:

| Event | Tempo | Wat |
|---|---|---|
| `state` | alleen bij een echte wijziging | Alles structureels. De motor publiceert zijn `Snapshot` op een vaste tik, maar `ui/mod.rs` serialiseert en **vergelijkt met de vorige**; is hij gelijk, dan gaat er niets de brug over. |
| `meters` | 4 Hz, en alleen als hij verandert | Spreekniveau en RTT — de twee dingen die bewegen terwijl je er alleen naar kijkt. De frontend werkt hiermee attributen bij; er wordt geen paneel hertekend. |
| `thumbnail` | 2 Hz per bekeken stream | Alleen een sleutel plus een revisienummer. De PNG zelf komt over `thumb://` als een gewone `<img>`. |

RTT en spreekniveau staan daarom **niet** in `UiState`: zaten ze erin, dan zou elke tik
een verschil opleveren en zou de vergelijking hierboven niets meer tegenhouden. Datzelfde
geldt voor "laatst gezien", dat alleen meegaat voor een peer die *niet* online is.

**Gemeten in rust op deze machine** (debug-build, één peer geconfigureerd en offline, geen
gesprek): 31 ms processortijd over 60 seconden voor het hele procesboompje — fitcom plus
zijn zes WebView2-processen — oftewel 0,05% van één kern. Het geheugen is wél duurder:
381 MB werkset voor dat hele boompje, tegen de tientallen MB's die egui gebruikte. Dat is
de eerlijke prijs van een webview en hij was in `PRODUCT.md` niet expliciet benoemd; hij
is de moeite waard omdat invariant 4 over processortijd naast een game gaat, niet over
werkgeheugen, en 381 MB is op deze machines geen schaars goed.

**De tijdlijn rijdt niet mee in `state`.** Hij is het enige onbegrensde deel van de staat,
en een hele geschiedenis over de IPC-brug duwen bij elke wijziging zou precies de winst
hierboven weggooien. In plaats daarvan draagt `state` een `timeline_revision` (opgehoogd
zodra de oplog een nieuwe `Arc<Timeline>` oplevert) en haalt de frontend het open gesprek
op met `get_timeline`.

**Beslissing 15 is vervallen.** De `GetAsyncKeyState`-omweg voor Ctrl+V is weg: de webview
krijgt een echt `paste`-event met de bytes van de afbeelding erin, en die gaan via
`offer_pasted_image` naar dezelfde aanbiedflow als een gesleept of gekozen bestand.

### 22. De camera is een derde `BronSoort`, niet een tweede pijplijn (2026-08-06)

Rick wilde de camera aan kunnen zetten, ook tegelijk met een gedeeld scherm. De hele
deelketen — encoder, pacing, `Verzendtempo`, fragmentatie met pariteit, keyframe-op-verzoek,
kijker, weergaveklok, miniaturen — is al generiek over `Bron`. Dus kwam er **geen tweede
pijplijn**: `BronSoort::Camera` erbij, `Capture` werd een enum met twee varianten, en
`deler.rs`, `kijker.rs`, `fragment.rs` en de `Actie`-laag zijn niet aangeraakt. Dat
meerdere eigen streams naast elkaar kunnen, was ook al zo (`engine.delers` is een map op
`stream_id`), dus "camera tegelijk met scherm" vroeg geen enkele wijziging in de
streamlogica.

Wat er wél aan vastzat:

- **`StreamKind::CAMERA = 4`, additief, geen `protocol_version`-bump.** Op de draad is een
  camerastream niet van een gedeeld scherm te onderscheiden. De app moet het verschil toch
  weten, want bureaubladgeluid hangt aan een *scherm*: je webcam aanzetten mag niet
  stilzwijgend je Spotify de kamer in sturen. Daarvoor zijn `StreamKind::is_scherm()` en
  `is_beeld()` bijgekomen, en `stem_geluid_af_op_beeld` filtert nu op `is_scherm()` in
  plaats van "alles wat geen geluid is". Regressietest:
  `streams.rs::tests::naar_een_camera_kijken_haalt_geen_bureaubladgeluid_binnen`.
- **Windows: Media Foundation, en dit pad gaat wél door het werkgeheugen.** Bij een scherm
  is "nooit een kopie naar RAM" een harde eis (invariant 4). Bij een camera is dat een
  andere afweging: 720p30 is een orde kleiner dan 1080p60, en een webcam levert MJPEG of
  YUY2 aan, dus er staat hoe dan ook een omzetting in het pad. Met
  `MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING` doet MF die omzetting zelf naar
  RGB32 en is één `memcpy` naar een textuur (`maak_textuur_met`) goedkoper dan zelf een
  NV12-tussenstap met de GPU-videoprocessor optuigen. Upgradepad staat als
  `ponytail:`-commentaar in `camera.rs`: NV12 in een DXGI-buffer vragen en
  `kleur::Kleuromzetter` ertussen, als er ooit een 4K60-webcam komt.
- **De reader staat op zijn eigen thread.** `ReadSample` blokkeert, en de deel-lus wil een
  timeout kunnen stellen om te kunnen kijken of hij nog door moet. Kanaal ertussen, zelfde
  patroon als de mac-SCK-uitvoer.
- **RGB32 uit MF staat standaard onderstboven** (DIB-indeling, negatieve stride). Een
  omgekeerd beeld zie je zelf niet, alleen je vriend. We zetten daarom expliciet een
  positieve `MF_MT_DEFAULT_STRIDE` op het uitvoertype zodra de afmeting bekend is, en
  lezen daarna alsnog de werkelijk geldende stride terug: is die negatief, dan klapt de
  kopieerlus de rijen om. Beide takken staan in de code omdat niet elke camera de
  strideset overneemt. **Dit is op geen echte camera getest** — zie de testlijst.
- **De camera wordt aangekondigd met een nominale 1280×720.** Zijn echte afmeting vraag je
  pas als je hem opent, en dat zou het lampje aanzetten voordat er iemand kijkt — precies
  wat "er wordt niets opgenomen tot iemand kijkt" verbiedt. Het eerste echte beeld
  overschrijft de maat aan beide kanten: de encoder gebruikt `capture.afmeting()` en het
  kijkvenster past zich al aan (`Venster::pas_maat_aan`, en op mac de laagformaat-cache).
- **Eén schakelaar, niet de bronkiezer.** `UiCommand::ZetCamera(bool)` zoekt zelf de eerste
  camera en meldt via de bestaande foutbalk als er geen is. De kiezer filtert camera's er
  juist uit: een webcam onder "Share a screen or window" leest als een fout. De knop zit
  naast mute/deafen, maar met een eigen `self-btn--on`-stijl — die drie zijn "iets staat
  uit" (rood), camera-aan is het tegenovergestelde (accent).
- **macOS: bewust overgeslagen** (op Ricks verzoek). `BronSoort::Camera` bestaat daar wel,
  zodat de gedeelde app-code niet open hoeft; `beschikbare_bronnen` noemt geen camera's en
  `Capture::start` weigert er een met een leesbare melding. **Kijken naar de camera van een
  Windows-peer werkt op de mac wel** — dat is dezelfde H.264 in hetzelfde venster. Het
  bouwplan staat in `TODO.md`.

**Windows-code is hier niet te compileren.** `cargo check --target x86_64-pc-windows-msvc`
loopt op deze Mac stuk op `ring` (C-code, via `fitcom-net`). De omweg die wél werkt en
waarmee `camera.rs` is nagelopen: een losse crate met alleen `windows`, `anyhow`,
`crossbeam-channel` en `tracing` als deps, die de echte bestanden met
`#[path = ".../crates/video/src/camera.rs"] pub mod camera;` insluit. Daarmee zijn
`cargo check` én `cargo clippy` voor de Windows-target schoon te krijgen zonder Windows.
Dat vangt elke API-vormfout; het vangt geen gedrag.

### 23. Updates komen uit een getekende release-feed, niet meer van een peer (2026-08-07)

Rick wilde af van "een peer duwt je een exe toe", maar zonder ergens een server te moeten
draaien en zonder maandelijkse kosten. Dat kan: een statisch JSON-manifest plus de exe als
release-asset op GitHub. Gratis, niets om te beheren, en de app haalt het alleen op als
hij zelf gaat kijken.

**Wat de echte winst is, en dat is niet het transport.** Het oude pad liet dezelfde peer
zowel de bytes als de hash leveren waartegen die bytes gecontroleerd werden (B-01). Dat
is geen controle, en het maakte van één besmette machine een worm richting de andere twee.
De feed lost dat op met twee onafhankelijke bewijzen: TLS zegt met wíe we praten,
Ed25519 zegt wie de release gemáákt heeft. De privésleutel staat op geen van de drie PC's
en niet bij GitHub, dus een gekaapt account kan wel het bestand vervangen maar geen
geldige handtekening maken. Zonder ingebakken publieke sleutel weigert het hele pad —
falen gaat dicht, niet open.

- **Het manifest tekent versie, hash én grootte samen** (`"{version}\n{hash}\n{size}"`).
  Alle drie los tekenen zou toestaan dat iemand velden tussen twee geldige releases
  omwisselt.
- **De vormcontroles blijven, ook al is het getekend.** `version` belandt in een
  bestandsnaam, dus alleen cijfers en punten (B-02 kwam ooit precies langs die weg
  binnen); `url` moet binnen de ingebakken repo vallen; `size` onder 200 MB. Signing is
  geen reden om de goedkope controles weg te laten.
- **Hervatten is eruit.** `have_bytes`, het `.part`-hervatpunt en de
  cross-stream-synchronisatie (`update_verwachting_tx`, `wacht_op_update_verwachting`)
  waren er alleen omdat een peer-verbinding halverwege kon wegvallen. Een HTTPS-GET die
  faalt begin je gewoon opnieuw; dat is ~130 regels minder motor.
- **`ureq`, geen `reqwest`.** Blokkerend in een `spawn_blocking`, want dit is geen heet
  pad. Het gebruikt dezelfde `rustls` 0.23 en dezelfde `ring` als quinn, dus er komt geen
  tweede TLS-stack in de binary.
- **Wat er bewust níet is:** een handmatige "check nu"-knop. De tik doet het elke zes uur
  en de eerste een minuut na start; een knop is frontendwerk voor iets waar niemand op
  wacht.
- **macOS blijft uitgesloten.** `fitcom-updater` is daar een lege stub, dus een opgehaalde
  build zou nergens heen kunnen. De feed maakt het mogelijk, het toepassen nog niet.

**De prijs, expliciet.** Dit is een uitzondering op invariant 1 (nul servers, geen CDN).
De app werkt zonder internet volledig door — alleen updaten niet. De tweede prijs is
menselijk: raakt `release-key.pk8` kwijt, dan moet iedereen één keer met de hand
bijwerken. Dat is inherent aan een vertrouwensanker; daarom staat hij buiten de repo.

**Wat er nog open staat.** B-20 (TOCTOU: er wordt niet opnieuw geverifieerd vlak vóór het
spawnen van de updater) is met deze wijziging niet aangeraakt.

> **Eén punt hierboven is herroepen:** "wat er bewust níet is: een handmatige check
> nu-knop". Die is er nu wel, en om een reden die pas bleek toen het pad kapot was — zie
> beslissing 24 hieronder.

### 24. Bijwerken vanaf GitHub deed één leesbeurt te veel (2026-08-10)

Rick meldde dat het bijwerken niet werkt. Het zat niet in de handtekening, niet in TLS en
niet in de beslislogica, maar in vier regels leeslus — en het faalde **elke keer**, ook bij
een volstrekt correcte release:

```rust
let mut lezer = antwoord.body_mut().with_config().limit(rel.size).reader();
loop {
    let n = lezer.read(&mut buf)?;   // <- na de laatste byte: Err, niet Ok(0)
    if n == 0 { break; }
```

De begrensde lezer van ureq (`ureq::body::limit::LimitReader`) geeft bij een `read` nadat
zijn teller op nul staat geen einde-bestand terug maar `Error::BodyExceedsLimit`: hij kan
"body is precies zo groot als aangekondigd" niet onderscheiden van "de host stuurt meer dan
hij aankondigde". De grens stond op exact `rel.size`, dus de lus liep altijd door tot die
ene leesbeurt te veel en elke download eindigde in "bytes ontvangen: body exceeds limit".
Er is nooit een update binnengekomen.

Gefixt door de lus zelf te laten aftellen (`while ontvangen < grootte`, en nooit meer dan
het restant opvragen) in plaats van op einde-bestand te wachten. De grens blijft staan als
tweede net. Regressietest:
`release.rs::tests::een_body_van_exact_de_aangekondigde_grootte_komt_er_heel_door`, met een
nabootsing van die lezer — zonder die nabootsing is dit alleen met een echte HTTP-server te
zien, en dan zou dit bestand een testserver nodig hebben voor een bug van vier regels.

**Twee dingen eromheen, want alleen de leeslus repareren maakt dit niet zichtbaar
werkend:**

- **Er is nu wél een "Check for updates"-knop** (Settings → Account), en die antwoordt
  altijd: "Checking…", "You are on the newest version", of de letterlijke fout. Dat was
  eerder bewust weggelaten (beslissing 23), en dat blijkt precies de verkeerde keuze bij
  een pad dat stil faalt: de tik van zes uur meldt niets als de feed onbereikbaar is —
  terecht, offline is normaal — dus was er geen enkele manier om "hij werkt niet" van
  "er is niets nieuws" te onderscheiden. Een handmatige check meldt daarom élke uitkomst,
  een automatische blijft stil (`Updates::zoeken_gestart(handmatig)`).
- **`fitcom-release check`** doet vanaf de uitgeefmachine precies wat een gebruikersmachine
  doet: manifest ophalen bij `MANIFEST_URL`, handtekening tegen de sleutel van deze build,
  én controleren dat de exe waar het manifest naar wíjst er werkelijk staat. Dat laatste is
  geen luxe: `sign --url` pint de download vast op zijn eigen tag, dus tekenen vóórdat die
  tag bestaat levert een manifest op dat perfect klopt boven een download die 404't.
  **Dat is precies de toestand waarin de live feed op 2026-08-10 stond**: het manifest op
  release `v0.3.2` kondigt versie 0.3.3 aan met
  `…/releases/download/v0.3.3/fitcom.exe`, en een release `v0.3.3` bestaat niet. Zelfs met
  de leeslus gerepareerd komt daar niets binnen; er moet één keer opnieuw uitgegeven
  worden (zie "Een release uitgeven" verderop).

Verder een leesbare melding als `fitcom-updater.exe` niet naast `fitcom.exe` staat. Dat
gaf eerst "Het systeem kan het opgegeven bestand niet vinden", waar niemand uit opmaakt dat
er een tweede bestand uit de release naast de app hoort.

### 25. Eén COM-apartment per *thread*, en de camera-thread wordt afgewacht (2026-08-10)

Rick meldde dat de app crasht bij het uitzetten van de camera. Twee dingen op dat pad waren
fout, en het tweede verklaart waarom het juist bij het *uitzetten* stukliep.

**`zorg_dat_mf_draait` initialiseerde COM één keer per proces in plaats van per thread.**
Het eigen modulecommentaar van `crates/video/src/mf.rs` zei het al goed — "verwacht dat de
aanroepende thread een COM-apartment heeft" — maar `CoInitializeEx` stond samen met
`MFStartup` in dezelfde `Once`. `MFStartup` is per proces; een apartment is **per thread**.
Gevolg: precies één thread in het hele proces zat in een apartment en alle andere
media-threads werkten zonder. Voor het aanmaken van een MFT kom je daar meestal mee weg;
voor het openen en vooral het **vrijgeven** van een apparaatbron — een camera loopt via een
KS-proxy in de driverlaag — is dat geen fout meer maar een crash. De camera-leesthread was
de enige thread die zijn eigen `zorg_dat_mf_draait` nooit aanriep: `Cameracapture::start`
deed dat op de thread van de aanroeper en spawnde daarna de thread die de reader
werkelijk opent en sluit.

Nu: de `Once` doet alleen `MFStartup`, en een thread-local doet het apartment. Elke thread
die MF aanraakt roept het op zichzelf aan, inclusief de camera-thread als eerste statement.

**Er hoort geen `CoUninitialize` tegenover te staan, en dat is opzet.** De nette
tegenhanger is hier gevaarlijker dan de ziekte: het multithreaded apartment bestaat zolang
er minstens één thread in zit, dus als media-threads het bij hun einde netjes verlaten kan
de laatste vertrekker het hele MTA opdoeken terwijl `D3dContext::winrt_device` en de
WinRT-capture-objecten nog bij de motor leven. Eén apartmentverwijzing per media-thread
laten staan is precies het gedrag dat we willen. Staat als eigen sectie in de docstring,
zodat niemand dit later "opruimt".

**En `Cameracapture::drop` wacht nu op zijn leesthread.** Hij zette alleen een vlag en
ging verder, dus het uitzetten van de camera liep door terwijl die thread nog een reader en
een geopende apparaatbron vasthield: het lampje bleef aan, meteen weer aanzetten gaf "in
gebruik door iets anders", en het vrijgeven van die bron gebeurde náást alles wat de motor
intussen aan het opruimen was. Eén beeldtijd wachten is de hele prijs, op een thread die
toch aan het afsluiten is. Om dezelfde reden wacht `DelerHandle::drop` op zijn deel-thread
— maar **alleen voor een exclusieve bron** (een camera): een scherm mag je twee keer
tegelijk opnemen, dus daar blijft het gedrag ongewijzigd en kost een stoppende stream niets.

**Eerlijk over wat hier bewezen is:** deze twee fouten zijn met de hand gevonden door het
pad na te lezen, niet gereproduceerd — er staat hier geen Windows. Beide zijn
zelfstandig verkeerd en beide zitten precies op het pad dat crasht. Of de crash er
daadwerkelijk mee weg is, moet C.7/C.8 in `docs/TESTPLAN.md` uitwijzen.

### 26. Je eigen camera hangt aan de deler, niet aan een tweede opname (2026-08-10)

> **Deels ingehaald door beslissing 34 (2026-08-25):** het venster hieronder bestaat niet
> meer — de terugblik is een tegel in de streamstrook geworden. Wat hier over de
> *levensduur* van de deler staat (bestaat ook zonder kijkers, codeert niet zonder kijkers,
> het lampje gaat aan bij het aanzetten) geldt onverkort; lees 34 voor wat er van het
> venster in de plaats gekomen is en waarom.

Rick wilde zichzelf kunnen zien als hij de camera aanzet. De verleiding is een tweede
opname met een eigen venster ernaast, en dat kán niet: Media Foundation geeft een camera aan
één iemand tegelijk uit, dus die tweede opname zou de deler het apparaat afpakken (of
andersom). Het venster hangt daarom aan de **deel-thread**, die de textuur toch al in
handen heeft op weg naar de encoder: `DelerConfig::voorbeeld: Option<String>` zet het aan en
geeft het zijn titel, en het is dezelfde `crate::venster::Venster` als het kijkvenster —
dus ook op mac compileerbaar, met F11 en dubbelklik voor beeldvullend.

Wat daarvoor moest wijken, en dat is de echte wijziging:

- **Een deler bestaat nu ook zonder kijkers.** "Er wordt pas opgenomen als er iemand
  kijkt" gold tot nu toe absoluut. Voor een camera is *jij* die iemand. `zet_camera(true)`
  start daarom meteen een deler met een lege kijkerslijst, en `Actie::StopDelen` ruimt een
  stream met een voorbeeldvenster niet op maar zet alleen zijn kijkerslijst leeg.
  Het echte opruimen zit in `stop_met_delen`, expliciet, zodat het niet afhangt van de
  vraag of er iemand kéék.
- **Zonder kijkers wordt er niet gecodeerd.** De `kijkers.is_empty()`-controle is vóór
  `encoder.encode` gaan staan in plaats van erna. Voor een gedeeld scherm scheelt dat de
  paar beelden tussen de laatste kijker en het opruimen; voor een camera met alleen een
  voorbeeldvenster is het de normale toestand. Eén GPU-kopie per beeld op 720p30, geen
  encoder, geen socket.
- **`DelerHandle::gestopt()` en `::fout()`** zijn er bijgekomen omdat de motor een dode
  deler nu wél moet kunnen herkennen, en dat is een direct gevolg van het punt hierboven:
  zodra de opname bij het *aanzetten* begint, kan het aanzetten zelf mislukken — de camera
  is in gebruik door Teams, de encoder wil niet. Dat is precies het moment waarop iemand
  staat te wachten, en de deler heeft geen kanaal terug naar de motor. Dus legt hij zijn
  reden neer en haalt `Engine::ruim_gestopte_camera_op` die op de tik van 100 ms op: fout in
  de foutbalk, knop terug op uit. Dat vangt tegelijk het nette geval — voorbeeldvenster
  gesloten terwijl niemand kijkt — waar niets mis is maar de knop wel hoort terug te
  springen in plaats van "aan" te blijven staan boven iets dat niet meer draait.
  **Sluit je het venster terwijl er wél iemand kijkt, dan blijft het delen doorlopen**,
  alleen zonder venster; de camera gaat dan uit zodra ook die laatste kijker weg is.
  Alleen voor een camera: een gedeeld scherm heeft geen voorbeeldvenster en zijn
  aankondiging blijft staan als een deler eruit klapt, precies zoals eerst.
- **Het voorbeeldvenster niet kunnen openen is fataal voor de lus**, niet iets om
  overheen te lopen. "Camera aan, maar je ziet niets" is van de drie uitkomsten de
  slechtste; nu komt er een leesbare fout en springt de knop terug.

**De prijs, expliciet: het lampje gaat nu aan zodra je de camera aanzet**, niet pas als
iemand kijkt. Dat is geen verslapping van de regel maar de betekenis van wat er gevraagd
is — een terugblik zonder opname bestaat niet. Voor een **scherm** verandert er niets:
`voorbeeld` is daar `None` (naar je eigen scherm kijk je al), en de regel geldt onverkort.

### 27. Geluidjes worden gemaakt, niet meegeleverd (2026-08-10)

Er komt nu een korte toon bij het komen en gaan van iemand in het gesprek en bij een stream
of camera die aan of uit gaat. Drie keuzes daarin:

- **De tonen worden gerekend, niet meegeleverd.** Een wav naast de exe breekt "losse exe in
  een zip"; zes wavs in de repo bakken levert zes bestandjes op die niemand kan nalezen.
  `geluid.rs` schrijft ze bij het eerste gebruik zelf: sinus, korte in- en uitregeling
  tegen klikken, en de noten staan als noten in de broncode.
- **Niet via de voice-mixer.** Die bestaat alleen tijdens een gesprek, en het eerste
  geluidje dat je wilt horen is dat van je eigen deelname. Dus rechtstreeks naar het
  standaardapparaat: `PlaySound` met `SND_MEMORY | SND_ASYNC` op Windows (de bytes zijn
  daarom `'static` — `SND_ASYNC` leest ze ná de aanroep nog), `afplay` op macOS. Zelfde
  afweging als bij `notify.rs`: nul afhankelijkheden.
- **Een stream-geluidje hangt aan een *verandering*, niet aan een bericht.** Een
  `StreamAnnounce` komt bij elke herverbinding opnieuw langs voor een stream die we al
  kenden. De motor telt daarom hoeveel zichtbare streams een peer heeft vóór en ná het
  bericht (`zichtbare_streams_van`) en piept op het verschil. Bureaubladgeluid telt niet
  mee: dat gaat automatisch met een scherm mee en is geen eigen gebeurtenis.

Niet-storen zet ze uit, dezelfde regel als voor meldingen en om dezelfde reden. Mute en
deafen doen hier níéts: die gaan over het gesprek, niet over de app.

### 28. Vier geluidsets met een eigen volume, en hoe ze ontworpen zijn zonder ze te horen (1.0.1, 2026-08-10)

Rick vond de tonen uit beslissing 27 niet fijn genoeg en vroeg om drie alternatieven, in te
stellen en te bewaren, met een eigen volume dat ook bewaard blijft.

**Het probleem is niet de code maar het oordeel.** Deze machine kan geen geluid beoordelen,
en aan een parametertabel is niet te hóren of hij klopt. Dus is het ontwerp niet door één
partij bedacht: vier onafhankelijke voorstellen (elk drie sets) vanuit vier verschillende
invalshoeken — psychoakoestiek, wat bestaande producten feitelijk doen, implementeerbaarheid,
en de luistersituatie zelf (avond, headset, game eronder) — daarna vier juryrondes met elk
één lens (onderscheidend vermogen, klinkt-het-prettig, implementeerbaar-zonder-gokken, past
het bij dít product), en pas daarna één samenvoeging.

Wat dat opleverde, en het is de reden om het zo te doen: **alle vier de invalshoeken kwamen
onafhankelijk op dezelfde drie families uit** — aangeslagen glas, aangeslagen hout, en een
geblazen/ademende klank. Die convergentie is een sterker signaal dan welk enkel voorstel ook,
en ze wees ook iets áf: de ademende familie werd door meerdere lenzen als de zwakste
beoordeeld, en is dus niet de derde set geworden. Glas stond bij alle vier de jury's in de
top drie.

De drie sets zijn daarmee **Glass**, **Wood** en **Keys**, met **Classic** als vierde:

| Set | Waar de klank vandaan komt | Waarom hij van de andere verschilt |
|---|---|---|
| Classic | Kale sinus, vlakke omhulling | De set uit 1.0.0. Staat, dooft niet uit. |
| Glass | Modes 1 : 2,76 : 5,40 (aangeslagen buis) | Ónhele partialen, dus geen toonhoogte-gevoel maar een voorwerp; hoge modes vallen 2-6× sneller weg, dus hij verkleurt van helder naar warm; twee grondtonen 2,5 Hz uiteen geven een langzame zweving. |
| Wood | Modes 1 : 3,93 : 9,55 (vrij opgelegde balk) | De stemming van een marimba, een heel andere reeks dan glas. Kort en droog, met een ruistik van elf milliseconde als hamercontact eronder. |
| Keys | FM, modulator 1:1, index 2,3 die in 48 ms wegvalt | Zijbanden op hele veelvouden, dus hij hóudt een toonhoogte waar glas dat juist niet doet. Hol bij de aanslag, vrijwel zuivere sinus als hij wegvalt. Het laagste register van de vier; peer-gebeurtenissen krijgen een dovere modulator (`TOETS_FM_DONKER`). |

Binnen elke set: twee overlappende klanken voor je eigen gebeurtenissen en één voor die van
iemand anders, zodat het *aantal* klanken de eerste aanwijzing is, nog vóór de toonhoogte.
Erbij stijgt, eraf daalt. Het aan- en uitzetten van een stream staat een register hoger met
een ánder interval — bij Wood is het zelfs geen slag meer maar één gebogen toon zonder
hamertik. Dat is opzet: een stream is geen persoon die binnenkomt, dus hij mag ook niet als
dezelfde klank een terts hoger klinken.

**De samenvoegstap heeft het niet afgemaakt.** De vierde fase van die workflow (één agent die
de winnaars samenvoegt) is na een kwartier nog bezig geweest en is afgebroken; de merge is
daarna met de hand gedaan, uit dezelfde vier voorstellen en vier juryrapporten die die agent
ook had. Dat staat hier omdat het het verschil uitmaakt tussen "dit is door een panel
vastgesteld" en "dit is door mij gekozen op basis van wat een panel opleverde", en het tweede
is wat het is.

**Wat er van dat oordeel objectief na te meten valt, is nagemeten.** Ik kan ze niet horen,
maar ik kan de wav-bestanden lezen. Alle 24 zijn langs: duur, piek, luidheid, of de eerste
sample precies nul is en de laatste vrijwel, en twee dingen die een eerste poging fout deed:

- **Klikken.** Niet "is er een sprong groter dan X tussen twee samples" — een partiaal op
  5 kHz geeft bij deze amplitude legitiem sprongen van een kwart. Een klik is een
  *losstaande* discontinuïteit, dus de grootste sprong wordt vergeleken met de 99,9e
  percentiel van alle sprongen. Bij alle 24 zit die verhouding op 1,0 tot 1,3 — geen enkele
  uitschieter.
- **Of het gebaar de goede kant op gaat.** Niet "welke toon is het luidst in de eerste
  helft": bij overlappende klanken klinken beide toonhoogtes de hele tijd door, en dan meet
  je niets (de eerste versie van deze meting zei van een stijgende klank dat hij vlak was).
  Wat het gebaar máákt is hoe de verhouding tussen de twee toonhoogtes over de tijd
  verschuift. Zo gemeten stijgt elke join en daalt elke leave, in alle vier de sets.

Er is een `#[ignore]`-test (`geluid.rs::schrijf_alle_geluidjes_weg`) die alle sets naar een
map schrijft, precies zodat dit — en beluisteren door een mens — kan.

**Het apparaat eronder.** Eén synthesefunctie voor alle sets; een set is een tabel.

- `Partiaal { ratio, offset_hz, amp, aanslag_ms, tau_deel }`. `ratio` mag onheel zijn: dat is
  het verschil tussen een klok en een orgelpijp. `offset_hz` is er voor zweving en staat in
  hertz en niet als ratio — met een ratio zweeft een hoge noot sneller dan een lage en valt
  de familie uit elkaar. `tau_deel` laat hoge partialen sneller wegvallen dan lage, en dat is
  wat elk aangeslagen voorwerp doet.
- `Omhulling::Vlak` (een toon die *staat* — de klassieke set) of `Aanslag { tau, release }`
  (een toon die *wegvalt*). Die release is niet cosmetisch: een exponent bereikt nooit nul,
  en de sprong van "wat er nog staat" naar stilte is een tik.
- `Ruis` als optionele laag onder een toon: het geluid van het *contact*, niet van de toon.
  Volgorde is dwingend — filteren, dán op piek 1 normaliseren, dán de omhulling erover.
  Andersom is `amp` niet meer te lezen als "hoe hard het contact is" (de filterversterking
  hangt van de afsnijfrequentie af) én is de eerste sample niet nul.
- `Glijden::Naartoe` doet `f(t) = doel + (start − doel)·e^(−t/τ)`: hij schiet erheen en valt
  op zijn plek. Lineair glijden klinkt als een sirene.
- **De fase wordt doorgeteld, nooit `sin(2π f t)` met een veranderende `f`.** Dat laatste
  geeft een fasesprong op elke frequentiewijziging, en dus een tik. Dit is de valkuil waar
  drie van de vier voorstellen expliciet voor waarschuwden.

**Genormaliseerd op luidheid, niet op de piek — en dat is gemeten, niet bedacht.** Eerst
stond er één piekgrens van 0,22 voor alles, met een gewicht per gebeurtenis eroverheen. Dat
klinkt eerlijk en is het niet: nameten gaf dat de glas-set bij dezelfde piek **5 tot 9 dB
zachter** uitkwam dan de klassieke, want een aangeslagen klank van 380 ms zit maar de eerste
honderd milliseconde in de buurt van zijn piek. Van set wisselen zou dan voelen alsof de
geluidjes bijna weg waren — precies wat je niet wilt bij een instelling die je uitnodigt om
te wisselen.

Dus wordt er nu genormaliseerd op **luidheid**: de hoogste RMS over een schuivend venster van
200 ms, ongeveer de integratietijd van het oor. Niet de piek (één sample kan die zetten, en
hij zegt niets over hoe hard iets klinkt) en niet de RMS over het hele bestand (dan maakt een
lange stille nagalm het begin "zachter"). Gemeten na de wijziging: alle vier de sets komen op
0,00 dB van elkaar uit op gelijk gewicht, en het gewenste verschil binnen een set staat exact
waar het hoort — −2,3 dB voor wat een ander doet, −3,4 dB voor een stream.

Er staat nu een piek*plafond* van 0,6 in plaats van een piekdoel: een uitdovende klank heeft
een hogere piek nodig om even luid te klinken (de hoogste van de 24 is 0,49), en het plafond
garandeert alleen dat er nooit iets vervormt, wat er ook in een tabel gezet wordt. Vier tests
bewaken het geheel: de luidheid staat precies op zijn gewicht, twee sets met hetzelfde gewicht
klinken even luid, geen tabel raakt het plafond, en **de klassieke set houdt het niveau van
1.0.0** — die is al goedgekeurd en mag door al dit normaliseren niet verschuiven.

**Het volume is een eigen instelling omdat het niet anders kan.** Deze tonen gaan bewust
langs de voice-mixer heen (beslissing 27), dus de enige andere knop zou de volumemixer van
Windows zijn — en die zet de hele app zachter, inclusief de stem van je vriend. Dus
`[sound] volume` in `config.toml`, met `set` ernaast. Alles `#[serde(default)]`: een config
van 1.0.0 heeft geen `[sound]`-tabel en hoort gewoon te starten. Eigen test, want dat is
precies wat bij de kanalen-uitbreiding één keer misging.

**Wat de review erna opleverde.** Vijf lenzen over het resultaat, elke bevinding daarna door
een aparte agent die hem probeerde te weerleggen. Zeven claims, twee bevestigd, vijf
weerlegd — en twee van die weerleggingen waren nuttiger dan de bevinding die ze afwezen.
(Eén lens, de frontend-lens, is halverwege op een API-fout gestrand en heeft niets
opgeleverd; die kant is dus alleen door mij nagelezen.)

- **Bevestigd:** op macOS hing de naam van het tijdelijke wav-bestand aan de gebeurtenis, dus
  "het bestand bestaat" gold als bewijs dat het het júiste bestand was. Na een wijziging in
  een tonentabel bleef de vorige build klinken. De naam komt nu uit een hash van de inhoud, en
  het schrijven gaat via `.part` + hernoemen zodat een tweede instantie hem nooit halfaf ziet.
  Nagelopen op deze Mac (`het_mac_pad_schrijft_een_bestand_dat_bij_de_inhoud_hoort`).
- **Weerlegd, maar met een beter voorstel:** de vergiftigde-slot-tak in `onthoud` liet de
  buffer vallen terwijl `PlaySound` er nog een verwijzing naar had. Onbereikbaar (er wordt
  niets gedaan dan pushen en verwijderen, dus er kan niets paniceren), dus formeel geen
  defect — maar de reviewer wees op `lock().unwrap_or_else(|e| e.into_inner())` als de nettere
  oplossing dan mijn eerste poging, die de buffer lekte. Overgenomen.
- **Bevestigd, en dit was de nuttigste bevinding van de hele ronde:** in Keys waren de eerste
  120 ms van "iemand komt erbij" meetbaar *dezelfde golf* als die van je eigen deelname —
  genormaliseerde correlatie **0,9999**, tegen onder 0,15 bij de andere drie sets. Oorzaak:
  Keys gebruikte voor alle zes gebeurtenissen één partialentabel en één modulator, en
  peer-join begon op de grondtoon van eigen-join. Het hele onderscheid hing dus aan een
  tweede noot die 120 ms later niet komt. Bovendien beweerde het commentaar een octaafverschil
  dat er alleen bij de leave was.
  Gerepareerd zoals Glas het al deed: één dovere klank op de **aankomsttoon** van het eigen
  gebaar (`TOETS_FM_DONKER`, en F4/C4 in plaats van C4/F3), dus drie aanwijzingen die alle
  drie vanaf de eerste sample gelden — één noot in plaats van twee, dover, en een andere
  toonhoogte. Nu 0,007. Een octaaf naar beneden was de verleiding en zou fout zijn geweest:
  `luidheid` weegt niet naar frequentie, dus dezelfde RMS een octaaf lager klinkt merkbaar
  zachter dan de rest van de set — die valkuil kwam uit de weerlegging, niet uit de bevinding.
  Er staat nu een test op (`een_eigen_gebeurtenis_klinkt_vanaf_het_begin_anders_dan_die_van_een_ander`);
  aan een parametertabel is dit niet te zien, en horen kan deze machine niet.
- **Weerlegd, en dat heeft een wijziging van mij tegengehouden.** Ik had `herstel()` een
  onbekende setnaam laten terugzetten op de standaard, omdat er anders niets geselecteerd
  staat in de kiezer. Dat breekt de belofte die twee velden hogerop in datzelfde bestand
  staat: een config van een nieuwere build mag zijn keuze niet verliezen doordat je één keer
  een oudere versie start — en met zes `Config::save`-plekken in `engine.rs` zou die naam
  daarna ook echt overschreven worden. Teruggedraaid; de kiezer laat nu in `ui/state.rs` zien
  wat er *werkelijk klinkt*, terwijl de config de bedoeling vasthoudt. De weergave hoort de
  werkelijkheid te tonen, de opslag de bedoeling.

**De standaard blijft de klassieke set.** Wie bijwerkt hoort te horen wat hij gewend is; de
nieuwe sets staan één klik verderop, met een kaartje per set en zes proefknoppen om ze te
beoordelen. Van set wisselen speelt hem meteen één keer — anders kies je op een naam.
De proefknoppen negeren niet-storen: wie erop drukt vraagt erom, en een knop die niets doet
leest als een stukke knop.

### 29. Een gedownload bestand is te openen waar het aangeboden werd (2026-08-20)

Rick: als je een bestand of afbeelding gedownload hebt, moet die op dezelfde plek in de
chat klikbaar zijn — de downloadknop wordt een openknop. Afbeeldingen gaan in een modaal
venster op ware grootte open.

**Het pad was er niet meer.** `DownloadStatus::Voltooid` zei dát het gelukt was, niet
waar het bestand stond, en dat is niet te herleiden: `engine::unieke_bestandsnaam` maakt er
bij een naambotsing `naam (2).ext` van, en dat gebeurt precies wanneer twee peers hetzelfde
bestand aanbieden. Er is dus een nieuwe padkaart in `files::Files` bij gekomen
(`gedownload`), naast de bestaande `aangeboden`.

**Bewust twee kaarten en geen één.** `aangeboden` is de lijst die `verzoek_ontvangen` aan
andere peers uitlevert. Wat wij downloaden daarin gooien zou betekenen dat we het ook zelf
gaan aanbieden — dat is een functie (herverspreiden) die niemand gevraagd heeft, en die
verandert wie welke bytes kan opvragen. `lokaal_pad` leest ze allebei; verder blijven ze
gescheiden.

**Beide kaarten staan nu op schijf** (`<data>/bestandspaden.json`, puur lokaal, nooit een
op, nooit op de draad). Dat moest voor de openknop, en het loste iets op wat al kapot was
zonder dat het opviel: `aangeboden` werd alleen gevuld door `FileEvent::NieuwAanbod`, dus
**een peer kon een bestand dat jij aanbood niet meer ophalen zodra jij de app één keer had
herstart** — hij kreeg `NOT_AVAILABLE` terug voor een kaart die er nog gewoon stond. Bij het
inlezen valt elk pad af dat niet meer bestaat: liever geen knop dan een knop die niets doet,
en liever een eerlijke `NOT_AVAILABLE` dan een upload die halverwege stukloopt.

**Een openknop op een bestand van een ander is een uitvoeringspad.** Daarom opent de knop
bij een uitvoerbare extensie niet het bestand maar de map eromheen
(`files::opent_als_code`), en zegt hij dan ook "Show" in plaats van "Open" — een knop die
iets anders doet dan hij belooft is erger dan de beperking zelf. Het vertrouwensmodel zegt
dat de drie peers vrienden zijn, maar B-01 is juist gesloten omdat "hun pc mag code op de
mijne draaien" daar geen aanvaardbare lezing van is. Geen viruscontrole en niet bedoeld als
een: het punt is dat één klik in de tijdlijn nooit "start wat een ander mij stuurde" is.

**Het pad reist niet door de webview.** De frontend geeft dezelfde `OpRef` terug die de
downloadknop ook gebruikt; `open_file` zoekt het pad opnieuw op in de momentopname van de
motor. Zelfde patroon als B-52 (`offer_files` met indices in plaats van paden): de webview
zegt *welk item*, deze kant beslist *welke bytes*.

**Onderweg gevonden:** een afbeelding in de tijdlijn had helemaal geen CSS-regel. De comp
had daar alleen SVG-plaatshouders, dus `.shot svg` bestond en `.shot img` niet — een
1080p-schermafdruk werd op zijn eigen pixelmaat getekend en door `overflow: hidden` van de
kaart afgekapt op 420 px. Je zag de linkerbovenhoek en verder niets.

### 30. YouTube-previews: de motor haalt ze op, de webview blijft dicht (2026-08-20)

Rick: een YouTube-link in de chat moet zijn titel en miniatuur laten zien, zonder dat het
geld kost.

**Dit is de tweede bewuste uitzondering op invariant 1 (nul servers)**, na de release-feed
uit fase 13. Een titel en een plaatje kunnen alleen bij YouTube vandaan komen, dus de vraag
is niet *of* er een verbinding buiten het tailnet gelegd wordt maar wie hem legt en hoe
vaak. Rick heeft de afweging gemaakt met deze drie opties op tafel; dit is de gekozen vorm.

**`https://www.youtube.com/oembed`**: publiek, gratis, geen sleutel en geen account. De
YouTube Data API zou een key vragen, en een key in een exe die bij drie mensen op de pc
staat is geen key.

**Het ophalen zit in de motor, niet in een `<img src="https://i.ytimg.com/...">`.** Dat is
het hele verschil:

- De CSP blijft dicht. Er komt geen host bij in `img-src`, dus een bericht van een peer kan
  nooit een verbinding uit het venster laten vertrekken. De miniatuur komt over `asset:`
  van de eigen schijf, net als een gedeelde afbeelding.
- Geen cookies, geen referrer, geen request bij elk hertekenen. Eén keer per video, ooit,
  daarna van schijf (`<data>/youtube/<id>.json` + `.jpg`).
- Mislukt het, dan blijft het een gewone link. Invariant 7 (offline is normaal) geldt ook
  hier: dit is versiering en mag nooit een foutmelding opleveren. Daarom `tracing::debug`
  en niet `warn` — anders zet elk bericht met een link een regel in het log.

**De miniatuur-URL wordt zelf samengesteld** (`i.ytimg.com/vi/<id>/hqdefault.jpg`) en komt
**niet** uit `thumbnail_url` in het antwoord. Een URL uit een respons is een URL die je moet
gaan valideren; `hqdefault.jpg` bestaat voor elke video.

**Het video-id is het enige dat van buiten komt en het gaat twee injectiepaden in**: een
URL (queryparameter én padsegment) en een bestandsnaam in de cachemap — dat laatste is de
B-03-klasse. `youtube::geldig_id` eist precies elf tekens uit `[A-Za-z0-9_-]`, en dat kan
geen van beide iets anders worden. De frontend zoekt het id op met een eigen regex; deze
kant controleert het opnieuw, want de frontend is niet de plek waar dat vaststaat.

**De echte URL-vormen zijn niet offline na te kijken**, en fout betekent hier "de kaart
verschijnt nooit" zonder dat iemand ziet waarom. Daarom staat er één `#[ignore]`-test die
echt met YouTube praat — zelfde patroon als de rooktest op de echte geluidskaart:
`cargo test -p fitcom --lib youtube -- --ignored --nocapture`.

### 31. Wordle: elke peer haalt het echte woord zelf op, de kaart is geen op (2026-08-20)

Rick: een Wordle-spelletje in de app, met het **echte** woord van de dag, een scorebord van
wie er die dag won met de minste pogingen, en elke ochtend om 07:00 vanzelf een kaart in de
chat.

**Dit is de derde bewuste uitzondering op invariant 1 (nul servers)**, na de release-feed
(fase 13) en de YouTube-previews (beslissing 30). Apart voorgelegd, want `CLAUDE.md` zegt
erbij dat twee uitzonderingen geen precedent voor een derde zijn. Het echte woord kán
alleen bij NYT vandaan komen: de klassieke offline-berekening (index op de datum in een
meegebakken lijst) klopt sinds de overname niet meer, want NYT redigeert de lijst met de
hand. Rick heeft gekozen met drie opties op tafel — zelf ophalen, één peer die het rondstuurt,
of een eigen woordenlijst zonder netwerk.

**`https://www.nytimes.com/svc/wordle/v2/<datum>.json`**: publiek, gratis, geen sleutel en
geen account. Eén GET per dag per peer, daarna van schijf (`<data>/wordle.json`). Levert
`solution`, `print_date` en `days_since_launch` — precies wat er nodig is en niets meer.

**Iedere peer haalt het zelf op, en dat is een ontwerpkeuze en geen luiheid.** Eén peer die
het voor de anderen ophaalt en via een op rondstuurt zou:
- de dag laten afhangen van wie er om 07:00 online was — dat wringt met invariant 2 (geen
  host-peer);
- het antwoord op de draad zetten, terwijl er nu alleen *uitslagen* over de mesh gaan;
- de uitzondering niet kleiner maken, want er gaat nog steeds een verzoek naar NYT.
Dat alle drie hetzelfde woord krijgen is gratis: het staat op de datum en niet op wie vraagt.

**Het ophalen zit in de motor, niet in de webview** — zelfde reden als bij de
YouTube-previews: de CSP blijft dicht, er komt geen host bij in `connect-src`, en een
bericht van een peer kan nooit een verbinding uit het venster laten vertrekken.

**De oplossing gaat pas naar het venster als het spel klaar is.** De webview stuurt een gok
en krijgt vijf kleuren terug; `UiState` bevat het woord alleen als er niets meer te
verklappen valt. Niet tegen een aanvaller — het is je eigen spel — maar omdat de hele grap
eruit loopt als het antwoord in de eerste `get_state` staat.

**De kaart van de dag is géén op.** Dat was de eerste ingeving en het is fout: `seq` is per
(auteur, kanaal), dus drie peers die allemaal "hier is het raadsel van vandaag" plaatsen
zijn drie ops die de log niet tot één kan maken — er is geen inhoudsgebaseerde
op-identiteit om op te dedupliceren. En het hoeft ook niet: de kaart draagt geen enkel feit
dat een peer niet zelf kan uitrekenen. Hij wordt daarom lokaal in de tijdlijn gezet, op de
klok, vlak voor het eerste wat er die dag na 07:00 gezegd is. Wat wél reist zijn de
uitslagen (`OpKind::WordleResult`, tag 30, additief, geen protocolbump).

**De dag loopt van 07:00 tot 07:00 en de sleutel is de `print_date` van het raadsel.** Wie
om 00:30 nog aan "gisteren" zit te puzzelen scoort op de dag waar het raadsel bij hoort, en
niet op de stand van zijn eigen klok. Dat maakt het ook onmogelijk dat twee peers dezelfde
avond op verschillende dagen boeken.

**De uitslag is onveranderlijk: per (auteur, dag) wint de *eerste* op, niet de laatste.** Dit
is de enige plek in `timeline::build` waar niet last-writer-wins geldt. Zou de laatste
winnen, dan kon je je score bijstellen nadat je die van de anderen gezien had. Een `Delete`
doet er om dezelfde reden niets: er is geen eigenaarschap over iets dat al gebeurd is.
Een dag naspelen kan niet — alleen het huidige raadsel neemt gokken aan.

**Puntenregel, zoals Rick hem vroeg en N-agnostisch gemaakt.** Hij zei "een punt als je
wint of gelijkspeelt, en alleen als beide spelers gespeeld hebben". Met drie peers is
"beide" onbepaald; de regel is nu **minstens twee deelnemers** (`MIN_SPELERS`). Zo krijg je
geen punt voor alleen spelen, maar legt één peer op vakantie de competitie niet stil.
Verder: een punt voor iedereen met het laagste aantal pogingen onder de oplossers (dus
gelijkspel = allebei een punt), en heeft niemand het woord gevonden dan scoort niemand.

**De 14.855 toegestane gokwoorden staan in de repo** (`crates/app/src/wordle_woorden.txt`,
89 kB, via `include_str!` in de exe). Rick koos expliciet voor "alleen echte woorden" boven
"elke vijf letters". Het formaat is strikt en de code leunt erop: vijf ASCII-kleine letters
plus een newline per rij, gesorteerd, dus zes bytes per rij en binair zoeken zonder
allocatie. Eén test bewaakt dat formaat, anders is die aanname een tijdbom. **De oplossing
van NYT is altijd toegestaan, ook als hij niet in de lijst staat** — die lijst is een
afdruk, en zonder die uitzondering zou zo'n dag onoplosbaar zijn.

**De vierkantjes van de anderen blijven verborgen tot je eigen spel klaar is.** Een patroon
is een echte hint; het echte Wordle wordt ook pas ná het spelen gedeeld, en deze kaart staat
midden in het gesprek waar je niet omheen kunt kijken. Het *aantal* pogingen is wel meteen
te zien: dat is het getal waar de competitie om gaat.

**De prijs, expliciet.** Zonder internet is er die dag geen kaart. Dat is `debug` en geen
`warn` in het log en het levert nooit een foutmelding op — invariant 7 (offline is normaal)
geldt ook voor een spelletje. Elke vijftien minuten wordt het opnieuw geprobeerd.

**Wat er bewust niet is:** een geluidje of een melding bij een nieuwe kaart (het is geen
bericht dat op je wacht), een aparte plek in het menu (de kaart van vandaag staat onderaan
`#general`), en een eigen subkanaal (dan zou de app ongevraagd een `SetTopicTitle` moeten
plaatsen).

### 32. De afbeeldingenmap hoort in de downloadmap, niet in de datamap (2026-08-20)

**Gemeld door Rick:** "wanneer ik een screenshot post, krijgt mijn vriend vaak de error dat
hij de naam moet aanpassen ofzo". Zijn eigen vermoeden was dat de twee kanten een
verschillende naam voor dezelfde afbeelding verzinnen, met als voorstel er een UUID aan te
hangen die aan beide kanten hetzelfde is.

**Dat was het niet, en dat is het goede nieuws:** die gedeelde naam bestaat al sinds
beslissing 12 en is de blake3-hash van de inhoud. Ik heb de hele weg van een geplakt
plaatje nagelopen — aanbieder en downloader komen aantoonbaar op exact dezelfde
bestandsnaam uit. Een UUID zou er niets aan toevoegen; per peer opnieuw getrokken zou hij
zelfs precies het probleem terugbrengen dat beslissing 12 oploste.

**Waar het wél op stuk liep, was de láátste stap.** Het halve bestand (`.part`) stond in de
downloadmap, de eindbestemming in `<datamap>/Pictures`, en tussen die twee zat één kale
`tokio::fs::rename`. Rick's vriend heeft zijn downloadmap verzet naar een pad buiten
`%APPDATA%` — en `rename` kan niet over een schijfgrens heen (`os error 17` op Windows,
`EXDEV` op unix). Dus mislukte bij hem *elke* afbeelding, altijd, met de tekst
"bestand hernoemen naar definitieve naam: …" op de kaart in de chat. Die tekst is precies
wat hij las als "ik moet de naam aanpassen". Gewone bestanden bleven werken, want die
verhuizen nergens naartoe: hun `.part` staat al in de map waar ze blijven liggen.

**De regel is nu: de afbeeldingen staan in de downloadmap.** `<downloadmap>/Pictures`
(`config::resolve_pictures_dir`, één functie voor de regel). Dat lost twee dingen in één
keer op — de map staat waar de gebruiker zijn downloads wil hebben, in plaats van
onaangekondigd in `%APPDATA%`, en de eindbestemming staat per definitie op dezelfde schijf
als het halve bestand. Het `.part` van een afbeelding staat er ook al in (`deelpad_van`),
dus de laatste stap is een hernoeming *binnen één map*.

**Afbeeldingen verhuizen mee, downloads niet.** Dat lijkt inconsequent en is het niet: het
pad van een gewone download wordt *onthouden* (`bestandspaden.json`), dat van een
afbeelding wordt *afgeleid* uit de hash en de huidige map. Laat je de bestanden staan waar
ze stonden, dan rekent niemand dat pad nog uit en verdwijnt elke afbeelding van gisteren
uit de tijdlijn. Vandaar `verhuis_afbeeldingen` — bij de eerste start na deze wijziging
(van `<datamap>/Pictures` naar de nieuwe plek) en bij het kiezen van een andere
downloadmap. Kopiëren-en-weggooien als `rename` niet lukt, want dit is de ene plek waar de
verhuizing wél over een schijfgrens gaat. De onthouden paden schuiven mee (`verhuisd_pad`),
anders kan een peer een afbeelding die wij aanbieden niet meer ophalen.

**Twee dingen die onderweg meekwamen, allebei uit dezelfde regel code:**

- `zet_op_zijn_plek` in plaats van een kale `rename`: staat het doel er al mét de juiste
  grootte, dan is het klaar. Bij een afbeelding is de naam de hash, dus hetzelfde pad
  bewijst dezelfde bytes — en op Windows *mislukt* het vervangen van een bestand dat een
  ander proces net leest (de webview die het in de tijdlijn tekent, een virusscanner).
  Dat gaf een foutmelding voor een afbeelding die al gewoon binnen was. Verder een paar
  korte pogingen, voor de scanner die een net weggeschreven bestand een fractie van een
  seconde vasthoudt. Op de mac bestaat die klasse fouten niet — daar vervangt `rename` een
  open bestand gewoon — en dat is waarom dit alleen bij hem gebeurde en niet bij Rick.
- `download_bestand` weigert een tweede download voor iets dat al `Bezig` is. Twee klikken
  vóór de eerste toestandswijziging het venster bereikt leverden twee uploadstreams op die
  in hetzelfde halve bestand schreven. De knop staat tijdens een download al uit, dus dit
  is een smalle race — maar het is drie regels.

**Het gat dat dit mogelijk maakte:** de hele afbeeldingsweg had geen enkele test. Er staat
er nu één in `crates/app/tests/file_deling.rs` die een `.png` door twee echte motoren over
loopback duwt en controleert dat hij bij beiden onder dezelfde hashnaam in
`<downloadmap>/Pictures` landt, plus unittests voor de verhuizing, voor `zet_op_zijn_plek`
en voor waar het `.part` van een afbeelding hoort te staan.

### 33. Het `+`-menu: de kaart is tóch een op, maar alleen met de hand (2026-08-20)

Rick zag geen Wordle-kaart in zijn chat en vroeg om een `+` naast de paperclip: een menu
(voor nu één regel) waarmee je de kaart van vandaag alsnog de chat in stuurt, "voor beide
partijen", als noodgreep wanneer het automatische pad iets laat liggen.

**Wat er die dag werkelijk aan de hand was, en waarom de knop er tóch komt.** Zijn kaart
ontbrak niet: `wordle.json` had dag 20260820 gewoon staan. De kaart wordt geplaatst op
`openbaar_op(dag)` = 07:00, en dat is bij een dag met gesprek erin ver boven de onderkant
van de tijdlijn — hij zat in de scrollback. Dat is dus geen mislukte ophaal maar een
plaatsingsprobleem. Maar het gat waar Rick op mikte bestaat wél, en is erger: mislukt de
ophaal bij één peer, dan heeft die peer geen raadsel, tekent hij helemaal geen kaart, en
was er geen enkele manier om dat vanuit de app te herstellen. Beide klachten hebben dezelfde
oplossing, en dat is waarom de knop dit doet en niet iets anders.

**Beslissing 31 zei "de kaart is geen op", en dat klopt nog steeds — voor de automatische
kaart.** De aanname eronder was "elke peer kan de kaart zelf uitrekenen", en die houdt
precies zolang ieders ophaal lukt. Voor de peer bij wie dat niet lukte is de aanname onwaar,
en dan is een op het enige dat helpt. `WordleCard` (tag 31, additief, geen bump) draagt
alleen `day` en `number` — **nooit een woord**, dus wie de kaart zo krijgt kan hem pas
spelen als zijn eigen ophaal alsnog lukt. Dat is precies goed: de oplossing hoort niet op de
draad.

**Het bezwaar uit 31 — drie peers, drie onsamenvoegbare kaarten — wordt opgevangen en niet
omzeild.** `fitcom_store::timeline` houdt per dag de **eerste** aankondiging op
`(lamport, author)`; de rest is een no-op (invariant 6). `(lamport, author)` en niet
`(lamport, seq)`, want dit gaat over auteurs heen en `seq` telt per auteur — daar is dat
geen ordening. Drie tests in `timeline.rs` bewaken het, inclusief convergentie bij een
andere aankomstvolgorde.

**Een handmatige kaart staat op het moment van drukken, niet op 07:00.** Anders lost hij
het probleem niet op waarvoor hij bestaat. Die tijd komt van de klok van een andere machine
en bepaalt hier de *plek* en niet alleen een label, dus hij wordt tweemaal begrensd:
`klem_wall_clock` (B-42) in de store, en in `ui/state.rs` nog eens naar
`[07:00 van die dag, nu]`. Geen `clamp` daar — die paniekt als de grenzen elkaar passeren,
en dat mag in de tekenlaag nooit van een klok afhangen.

**Wat de knop niet doet:** het spel openen. Rick was daar expliciet over — vanaf de `+`
stuur je hem de chat in, spelen doe je vanaf de kaart, net als op elke andere dag. En hij
haalt het raadsel van morgen niet vóór 07:00 op: `datum_van` bepaalt óók wanneer de kaart
bij de anderen verschijnt, dus dat vooruithalen zou het spel scheeftrekken.

---

### 33. De clip-ring is van één sessie, niet van de vorige (2026-08-23)

Rick, derde testronde op fase 15: de sneltoets werkt, er komt een bestand uit, maar het is
elke keer de laatste minuut van een sessie van de dag ervoor. "Hij neemt niets nieuws meer
op." Dat klopte — en het stond zo in de eisen. `ROADMAP.md` had *"een herstart pikt de
bestaande ring op zonder te herschrijven"* als afvinkpunt en `TESTPLAN.md` punt 4 zei
*"herstart met aan = ring wordt verder gebruikt, niet gewist"*. Allebei omgedraaid.

**Waarom het niet kan.** Een segment draagt zijn begintijd in zijn naam
(`seg-{eerste_hns:020}.mp4`) en die tijd komt van `deler::klok_nulpunt`: de procesklok, die
bij **elke** start weer op nul begint (beslissing 7 — één klok per proces). De ring bleef
tussen sessies op schijf staan en werd bij het opstarten ingelezen. Segmenten van gisteren
dragen dus tijden uit een sessie die al een tijd liep, en die sorteren *na* de verse van
vandaag. Twee gevolgen, allebei precies wat Rick zag:

- `kies_venster` rekent terug vanaf het nieuwste segment. Het nieuwste wás het oudste, dus
  een clip bestond uit beelden van de vorige sessie.
- `te_gooien` rekent de retentiegrens vanaf het hoogste eindpunt. Had de vorige sessie
  langer gedraaid dan het clipvenster, dan lag die grens boven álles wat er vandaag
  bijkwam: elk vers segment werd gewist binnen een seconde na het schrijven. Vandaar
  "hij neemt niets meer op" — er wérd opgenomen, het werd alleen meteen weer opgeruimd.

**Waarom geen enkele test dit zag.** `herstart_pikt_de_ring_weer_op` deed precies dit
scenario — maar startte beide runs in hetzelfde proces, en daar is de klok per definitie
dezelfde. De test bewees dus iets anders dan waar hij naar heette te kijken. Hij heet nu
`herstart_begint_met_een_schone_ring` en wacht op *beide* helften van de nieuwe toestand:
alles van run 1 weg én minstens één vers segment. Dat wachten is niet optioneel — "de map
is niet leeg" is aan het begin van run 2 nog steeds waar, met precies de oude bestanden.

**Waarom niet gewoon de wandklok in de naam.** Dat is de andere voor de hand liggende
oplossing en hij is ook klein, maar hij lost het verkeerde probleem op. Segmenten uit twee
sessies naast elkaar in één spoor plakken betekent monsters uit twee **encodersessies**
in één `avcC`: eigen SPS/PPS, mogelijk een andere resolutie, mogelijk een ander scherm.
`plak_clip` schrijft één trackconfiguratie en kopieert de monsters ongewijzigd — dat is
per definitie een kapotte clip, alleen dan eentje die wél afspeelt tot hij dat niet meer
doet. En een wandklok kan achteruit springen (NTP), waarmee de sortering opnieuw stuk is.
**Gekozen: een startende opname veegt de ringmap leeg** (`leeg_ring`, alleen `seg-*.mp4`
en `seg-*.part.mp4`, de rest van de map blijft met rust). Dat ruimt en passant de halve
`.part`-bestanden op die na een harde stop bleven liggen en die niemand ooit opruimde.

**Twee eenheidsfouten in het geluid zaten hieronder verstopt.** Ze waren niet te zien
zolang elke clip toch uit oud materiaal kwam:

- **De duur ging in hns naar een spoor dat in samples telt.** De MFT levert
  `GetSampleDuration` in honderd-nanoseconden (213 333 per AAC-frame), de audiotrack rekent
  in samples (1024). Ongewisseld doorgeven is een factor 208. Gemeten aan Ricks clip van
  22 augustus: videospoor 11,81 s met 13 beelden, geluidsspoor dat beweerde **3000,00 s**
  te duren. Geen speler die daar geluid uit haalt — en dat is de "clips hebben geen geluid"
  uit ronde één en twee. Nu via `hns_naar_samples`, afgerond en niet afgekapt: afkappen
  scheelt één sample per frame en dat loopt over een minuut zichtbaar uit de pas.
- **De menger begon zijn tijdlijn op seconde nul.** Hij werd vóór de lus aangemaakt met
  `basis_hns: 0`, terwijl de taps hun chunks aanleveren met de tijd sinds *procesbegin*.
  Ging clips pas een tijd na het opstarten aan — de normale gang van zaken, want de
  schakelaar zit in de instellingen — dan bouwde hij de buffer vanaf nul op: honderden
  megabytes stilte, een AAC-encoder die zich daar eerst doorheen moest werken terwijl de
  opnamedraad stilstond, en monsters die daarna allemaal vóór het eerste segment lagen en
  dus wegvielen. De lus maakt hem nu zelf aan, op de klok van dat moment.

**En de sneltoets kon zichzelf opheffen.** `self.hotkey_draad = start_hotkey(..)` maakt de
rechterkant vóór Rust de oude waarde laat vallen, en `RegisterHotKey` weigert een toets die
nog geregistreerd staat. Dezelfde sneltoets opnieuw instellen liet je dus zonder sneltoets
achter, met alleen een `warn` in het logbestand. De oude draad gaat er nu eerst uit *en
wordt gejoind* (WM_QUIT → `UnregisterHotKey` op de eigen thread → einde), en een mislukte
registratie komt als fout terug in de UI in plaats van alleen in de log.

**De prijs, expliciet.** Na een herstart van de app is er even geen geschiedenis: wie
tien seconden na het opstarten op de sneltoets drukt krijgt tien seconden. De ring vult
zich daarna in segmenten van ~2 s. Dat is de goede kant om fout te zitten — een clip die
korter is dan gevraagd is duidelijk, een clip met beelden van gisteren erin is dat niet.

**Bewezen op echte hardware**, in Ricks eigen datamap met de vastgelopen ring erin:
`ring van een vorige sessie opgeruimd aantal=11`, daarna F9 → bestand binnen 100 ms.

| | videospoor | geluidsspoor |
|---|---|---|
| clip 22 aug (kapot) | 11,81 s / 13 monsters | **3000,00 s** / 675 monsters |
| clip 23 aug (na de fix) | 8,06 s / 222 monsters | 7,89 s / 370 monsters |

Plus drie tests in `crates/video/tests/opname_eind.rs` (`--ignored`, GPU + scherm), waarvan
één nieuwe die de opname bewust drie seconden ná procesbegin start: dát is de test die de
twee geluidsfouten vangt, want met de opname meteen bij het opstarten valt het verschil
tussen "sinds procesbegin" en "sinds opnamebegin" weg.

### 34. De terugblik op je eigen camera is een tegel, niet een venster (2026-08-25)

Rick: "als ik mijn camera aanzet wil ik geen tweede venster, ik wil mezelf in dat kleine
voorbeeldvakje zien." Dat vakje bestond al — de streamstrook boven de tijdlijn zette voor
je eigen stream een tegel neer — maar die tegel had nooit een beeld: hij kreeg
`key: null` in `renderStrip`, dus er was niets om een `thumb://`-URL van te maken en er
stond permanent het luie icoontje in. Het echte beeld ging naar een los Win32-venster
(beslissing 26).

**Wat er weg is.** `DelerConfig::voorbeeld` was `Option<String>` — een venstertitel — en is
nu een `bool`. De deel-lus opent geen `Venster` meer, pompt geen berichten en kan er dus
ook niet meer op afknappen. Weg daarmee:

- Het fatale foutpad "voorbeeldvenster niet te openen". Er is niets meer te openen.
- Het pad "venster dicht en niemand kijkt, dus stoppen" (en `geen_kijkers`). De camera loopt
  nu tot je hem uitzet of tot de deler eruit klapt. `ruim_gestopte_camera_op` blijft nodig
  voor precies dat tweede geval — camera in gebruik door Teams, encoder wil niet — en
  daarmee is die functie terug tot waar hij voor bedoeld was.
- De titel `"You — <camera>"`. De tegel zegt al "You".

**Wat ervoor terugkomt is het pad dat er al lag.** De deel-lus roept twee keer per seconde
`kijker::maak_miniatuur` aan op de textuur die de encoder toch al krijgt — letterlijk
dezelfde functie, dezelfde 192 pixels breed, hetzelfde tempo als de tegel van iemand
anders — en legt het resultaat in `Gedeeld::miniatuur`. De motor haalt hem op zijn tik op
via `DelerHandle::miniatuur()` en zet hem in `EigenStreamView`; `spawn_thumbnails` pompt
eigen en vreemde streams nu door dezelfde lus en `renderStrip` geeft de eigen tegel de
sleutel `self-<stream_id>`.

**Waarom dit goedkoper is dan wat het vervangt**, en dat is de reden dat het geen afweging
was: het venster kostte een tweede D3D11-swapchain plus een volledige `CopyResource` van
elk opgenomen beeld (720p30 = dertig kopieën per seconde) plus een `Present`. De tegel kost
één geschaalde GPU-naar-CPU-uitlezing van 192 px breed, twee keer per seconde. Invariant 4
("gamen wint") gaat er dus op vooruit.

**De `Mutex` in `Gedeeld` is geen inbreuk op "geen locks op het hete pad."** Het beeld zelf
raakt hem niet: de lus zet er twee keer per seconde een `Arc` in en laat het slot meteen
weer los, net zoals `fout` en `kijkers` er al stonden.

**Wat blijft staan uit beslissing 26**, want dat was het echte punt daar en niet het
venster: een camera-deler bestaat óók zonder kijkers (jij bent de kijker), zonder kijkers
wordt er niet gecodeerd, en het lampje gaat aan zodra je de camera aanzet. `heeft_voorbeeld`
heet nog zo en betekent nog hetzelfde — "deze eigen stream levert een terugblik op" — maar
dat is nu een miniatuur in plaats van een venster.

**Niet geverifieerd op deze machine, en dat kan hier ook niet:** macOS neemt bewust geen
camera op (`TODO.md`), dus `voorbeeld` staat er nooit aan. Getypecheckt, `clippy` schoon,
`cargo build` op mac. Wat Rick op Windows moet zien: camera aan → geen tweede venster,
en binnen een halve seconde staat er beeld in de "You"-tegel boven de tijdlijn, ook als er
niemand kijkt. Camera uit → tegel weg. Zie `docs/TESTPLAN.md` C.9.

### 35. De terugblik meet lokaal en gaat nooit de draad op (2026-08-26)

Een weekoverzicht: hoe lang je in het gesprek zat, wie er deelde, wie wat stuurde, en het
Wordle-scorebord van die week. `crates/app/src/gebruik.rs` is de hele motorkant.

**Tijd in het gesprek staat nergens.** `VoiceJoin`/`VoiceLeave` zijn vluchtige
control-berichten, net als `Typing` en `UserStatus` — na een herstart is er geen spoor van.
De verleiding is er een op van te maken, en dat is precies de val uit beslissing 31: drie
peers die elke minuut een regel in een append-only log schrijven, voor een feit waar
niemand het over eens hoeft te zijn. Dus meet elke peer op zijn eigen tik wie er in het
gesprek zit en wie er deelt, en dat blijft in `<data>/gebruik.json` staan. **Niets hiervan
gaat over de draad, en er komt geen op bij.**

De prijs, en die is bewust: het overzicht is "zoals deze pc het zag". Was je een avond
offline, dan telt die avond bij jou niet mee en bij de anderen wel. Dat staat ook zo in de
UI-tekst — het weg willen poetsen zou betekenen dat er alsnog iets over de draad moet.

**Wat je niet mag omdraaien:**

- **Tellen gaat over de rauwe ops, niet over de tijdlijn.** `timeline::build` klemt
  `wall_clock` op ±7 dagen rond nu (B-42), dus élk bericht ouder dan een week krijgt daar
  exact dezelfde tijdstempel. Voor een venster van zeven dagen valt dat toevallig goed uit;
  zou iemand het venster op een maand zetten, dan telt hij stilzwijgend de hele
  geschiedenis mee. `Chat::alle_ops` bestaat precies hiervoor.
- **De puntenregel wordt hergebruikt, niet nagebouwd.** `wordle::standen` op alleen de dagen
  in het venster. Een tweede telling hier gaat vroeg of laat uit de pas lopen met het
  scorebord in de chat, en dan is er geen manier om te zien welke van de twee klopt.
- **Het overzicht zit niet in `UiState`.** Elk getal erin loopt op zolang er een gesprek
  draait, en `spawn_state_pusher` vergelijkt de geserialiseerde `UiState`: erin zetten
  betekent tien `state`-events per seconde, precies wat de scheiding met `meters` moet
  voorkomen. Hij hangt in `Snapshot`, wordt eens per minuut uitgerekend, en de UI haalt hem
  met `get_recap` op wanneer het paneel opengaat — hetzelfde patroon als `get_timeline`.
- **Eén tik telt hoogstens vijf seconden bij** (`MAX_STAP`). De motor tikt tien keer per
  seconde; alles daarboven is een pc die geslapen heeft. Zonder die grens boekt één keer
  dichtklappen van de laptop acht uur "in gesprek".
- **Bureaubladgeluid telt niet als delen.** Het komt sinds fase 10 vanzelf met een scherm
  mee, dus meetellen is elke gedeelde monitor dubbel tellen.

**Waar het staat:** motor in `gebruik.rs` (met de meetstap in `Engine::meet_gebruik`),
weergavelaag in `ui/state.rs::recap_of` + `commands::get_recap`, en een tabblad "Recap" in
het instellingenscherm — het enige bestaande paneeloppervlak; een eigen plek in de linkerbalk
kost drie keer zoveel en levert hetzelfde.

**Getest:** zes unit-tests in `gebruik.rs` op de dingen die stilzwijgend fout gaan — boeken
op de juiste peer en dag, de slaapstandgrens, dagen buiten het venster, de volgorde, en een
rondje naar schijf en terug inclusief het snoeien van te oude dagen. **Niet geverifieerd:**
hoe het paneel eruitziet en of de getallen na een echt gesprek kloppen — dat vraagt een
tweede machine en een gesprek van enige lengte, net als bij elke andere mediafunctie.

## Bugs die de tests eruit haalden

Allemaal dingen die met handmatig testen niet betrouwbaar te vinden waren.

**Onbekende kanaal-tag aliasde stilzwijgend naar het algemene kanaal (fase 9).** Niet door
een test gevonden, maar door de `protocol-reviewer`-agent vóór het committen. De eerste
versie van de subkanalen-uitbreiding voegde tag 2 toe aan `Channel` zonder
`PROTOCOL_VERSION` op te hogen, met als redenering dat het wire-decoderen additief en dus
veilig was (`Channel` is een map, nieuwe velden krijgen `#[serde(default)]`). Dat klopte
voor het decoderen, maar niet voor wat er daarna met de op gebeurde: `channel_to_blob` en
`encode_channel` kenden alleen tag 0 (algemeen) en 1 (DM), en lieten elke andere tag —
dus ook de nieuwe subkanaal-tag — stilzwijgend terugvallen op tag 0. Een subkanaal-op en
een algemene op van dezelfde auteur met toevallig dezelfde `seq` (elk kanaal telt `seq`
onafhankelijk, dus dat is een gewone samenloop, geen randgeval) kwamen bij een peer die de
subkanaal-tag nog niet kende op **dezelfde primary key** in de `ops`-tabel terecht: welke
van de twee won hing af van aankomstvolgorde, en de andere verdween voorgoed zonder fout of
logregel. Hetzelfde gat zat in de bestandsoverdracht-header. Precies het patroon dat de
1→2-bump (zie beslissing 10) ook al moest voorkomen — alleen ditmaal niet zichtbaar in het
wire-formaat, maar in de lokale opslag eronder. Gefixt door `PROTOCOL_VERSION` alsnog naar 3
op te hogen: een peer zonder subkanaalbegrip wordt nu bij de handshake netjes met
`VersionMismatch` geweigerd, in plaats van dat de opslag stilletjes twee ops door elkaar
haalt. Zie `docs/ARCHITECTURE.md`, sectie "Kanalen", "Protocolversie: 1 → 2, 2 → 3".

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

**Tijdstempel per pakket in plaats van per beeld (fase 4).** De encoder leverde eerst
kale bytes op, en de deler zette er "nu" als tijdstempel op. Fragmenten van hetzelfde
beeld horen bij elkaar doordat ze dezelfde tijdstempel dragen, dus dat werkte alleen
zolang alle fragmenten in dezelfde milliseconde de deur uit gingen. De encoder levert nu
de tijd van het sample zelf, plus of het een keyframe is — dat laatste heeft de ontvanger
nodig om te weten waar hij kan aanhaken.

**Logbestand bleef leeg (fase 2).** `tracing_appender::non_blocking` buffert tot het
proces netjes eindigt — precies verkeerd voor het bestand dat je opvraagt als er iets
crasht. Schrijft nu direct.

**Testhelper deelde poorten dubbel uit (fase 2/3).** Parallelle tests binnen één binary
kregen soms hetzelfde poortnummer van het besturingssysteem. Gaf ongeveer één op de vijf
runs een onverklaarbare fout. De helper onthoudt nu wat hij uitgedeeld heeft.

**DM-venster toonde nooit het antwoord van de ander (kanalen, na fase 6).** Niet door een
test gevonden, maar door de `protocol-reviewer`-agent vóór het committen. `Channel::dm(x)`
betekent "de auteur DM'de naar x" — mijn eigen berichten aan X dragen dus `Dm(X)`, maar
X's antwoorden aan mij dragen `Dm(mij)`, niet `Dm(X)`. De UI filterde het DM-venster met
een simpele `channel == actief_kanaal`-vergelijking, die alleen mijn eigen kant van het
gesprek matchte. Het gesprek en het bestandenpaneel toonden dus permanent maar de helft.
Dit was een weergavefout, geen sync- of opslagfout: de op kwam gewoon goed binnen. Gefixt
met een predicate die beide kanaalwaarden van een gesprek herkent
(`crates/app/src/ui.rs::hoort_bij_kanaal`), nu met een eigen test
(`ui.rs::kanaal_tests::dm_toont_beide_kanten_van_het_gesprek`).

**`chat.sqlite`-schemabump weigerde Ricks échte database (kanalen, na fase 6).** De
`SCHEMA_VERSION`-bump (1→2, voor de `channel`-kolom op `ops`/`authors`) ging ervan uit dat
er nog geen echte database bestond om rekening mee te houden. Fout: er stond al 56 ops
aan echte chatgeschiedenis in `%APPDATA%\FitCommunication\data\chat.sqlite`, met een
echte, ingevulde `config.toml` (twee tailnet-peers). De app weigerde na de bump te
starten met "database is van schema-versie 1, deze app verwacht 2" — geen crash, maar
ook geen migratie, dus effectief ontoegankelijke data. Gevonden doordat de app bij Rick
niet meer opstartte, niet door een test (er was geen test die een écht bestaande
database met inhoud simuleerde). Gefixt met een echte migratie
(`Store::migreer_v1_naar_v2`): hernoemt de oude tabellen, zet ze over naar de nieuwe vorm
met `channel` overal op het algemene kanaal (dat was het enige dat vóór deze uitbreiding
bestond), en verhoogt `schema_version` — allemaal in één transactie, zodat een mislukte
poging de oude database intact laat. Getest tegen een met de hand opgezette v1-database
(`crates/store/tests/convergentie.rs::database_van_voor_de_kanalen_uitbreiding_wordt_gemigreerd_niet_geweigerd`)
én tegen Ricks eigen bestand (met een backup ervan gemaakt vóórdat de nieuwe build erop
losging).

**Stoppen met bureaubladgeluid deelde stopte het delen niet echt (fase 10).** Al langer
bekend bij Rick, opgepakt als het losstaande bugfix-item van fase 10. `Engine::voer_uit`
(`crates/app/src/engine.rs`) besliste bij `Actie::StopDelen` via `self.is_geluid(stream_id)`
of `stop_bureaublad()` aangeroepen moest worden — maar die functie zoekt de stream op in
`streams.eigen()`, en `Streams::stop_delen()` had de stream daar op dat moment al uit
verwijderd (`self.eigen.remove(plek)`, vóórdat de actie teruggegeven wordt). `is_geluid`
gaf dus bij een expliciete stop altijd `false`, ongeacht wat er gedeeld werd, en de
opnamethread aan de kant van de deler bleef gewoon doorsturen. Het generieke video-stoppad
merkte dit nooit, want dat vraagt nooit naar de soort stream — `self.delers.remove(&id)` is
toch een no-op voor een stream-id dat nooit in `delers` heeft gestaan.

Niet gevonden door een geautomatiseerde test (er was er nog geen voor dit pad), wel meteen
zichtbaar zodra je het naleest: `uitgetekend()` (de laatste kijker die zelf wegvalt) gaf
altijd het juiste antwoord, want die verwijdert de stream niet uit `eigen` vóór de actie.
Gefixt door de vlag in de actie zelf te leggen — `Actie::StopDelen` draagt nu `is_geluid:
bool`, bepaald op elk van de drie plekken in `streams.rs` die hem opbouwen (`stop_delen`,
`bij_verbreking`, `uitgetekend`) terwijl de stream nog bestaat — hetzelfde patroon dat
`Actie::StartKijken.is_geluid` al gebruikte. Eigen regressietest:
`crates/app/src/streams.rs::tests::expliciet_stoppen_met_bureaubladgeluid_is_gemarkeerd_als_geluid`.

---

## Valkuilen in deze omgeving

- **Een draaiende `fitcom.exe` blokkeert `cargo build`** met "Toegang geweigerd
  (os error 5)". Altijd eerst `.\scripts\run-peers.ps1 -Stop`.
- **`backgroundColor` in `tauri.conf.json` sloopt het venster op deze machine.** Met
  `"backgroundColor": "#0E1013"` erin weigert WebView2 te starten:
  `failed to create webview: WebView2 error: WindowsError(HRESULT(0x80070057))` — "de
  parameter is onjuist" — en dan draait de app wel maar krijg je nooit een venster te
  zien. Zonder die sleutel start hij gewoon. De reden om hem te willen (geen witte flits
  bij het openen) is anders opgelost: het venster staat op `"visible": false` en de
  frontend roept `ready` aan zodra hij getekend heeft.
- **De frontend heeft géén bouwstap.** `crates/app/frontend/` is gewone HTML, CSS en JS
  zonder bundler; `tauri-build` bakt de map in de exe. `cargo build` bouwt dus nog steeds
  de hele app en er is geen Node nodig — anders dan `PRODUCT.md` bij het besluit
  verwachtte. Een frontendwijziging vereist wel opnieuw bouwen: de assets zitten in de
  binary, niet ernaast.
- **`crates/app/icons/icon.ico` moet bestaan**, anders weigert `tauri-build` met
  "`icons/icon.ico` not found". Het bestand is het merkteken uit de titelbalk.
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
- **De H.264-decoder van Windows gebruikt DXVA.** `Microsoft H264 Video Decoder MFT` is
  formeel een software-MFT, maar zodra je hem ons D3D11-apparaat geeft levert hij zijn
  beelden als GPU-textuur. De keten raakt het werkgeheugen dus nergens. Het pad voor een
  decoder die dat níét doet zit er wel in (`Decoder::op_gpu` meldt welk pad actief is),
  maar is op deze machine nooit uitgevoerd en dus ongetest.
- **De vensterlijst bevat rommel.** `EnumWindows` levert ook onzichtbare hulpvensters op;
  er wordt al gefilterd op zichtbaarheid, titel en `WS_EX_TOOLWINDOW`, maar er blijven
  dubbelingen in staan (twee keer "Mail", twee keer "Instellingen"). Cosmetisch.
- **Een `cpal::Stream` die niemand vasthoudt stopt met leveren, zonder fout.** Bouw je hem
  in een hulpfunctie en geef je hem niet terug, dan valt hij op de regel ná `play()` en
  komt er nooit één sample binnen — `start()` meldt gewoon Ok. Dat was precies de reden dat
  clips wel spelgeluid hadden en niet je eigen stem (`microfoon.rs`, 2026-08-23). De
  voice-sessie heeft het goed (`open_apparaten` geeft beide streams terug); een tap die dat
  niet doet ziet er van buiten identiek uit. Test hierop door de chunks écht op te vangen,
  niet door te kijken of het starten lukte.
- **Een paniek in een spawned thread staat níét in het logbestand.** Hij gaat naar stderr,
  en de app schrijft zijn log ergens anders heen — dus een draad die meteen omvalt ziet er
  van buiten uit als een draad die niets te doen heeft. Dat kostte een avond bij de clips
  (een lege ring, geen enkele foutregel, terwijl de opnamedraad al vóór het eerste segment
  gesneuveld was op een index). Recept: start de instantie met stderr naar een bestand
  (`Start-Process ... -RedirectStandardError`) en lees dát erbij. Elke draad die op eigen
  benen staat verdient bovendien een diagnoseregel die ook logt wanneer er *niets* gebeurt.

---

## Hoe screenshare in elkaar zit

```
crates/app/src/streams.rs   Wie deelt wat en wie kijkt waarnaar. Pure toestandslogica,
                            raakt geen GPU, scherm of socket aan. Levert `Actie`s op.
crates/app/src/engine.rs    Voert die `Actie`s uit: threads starten en stoppen.
crates/video/src/deler.rs   opnemen → coderen → fragmenteren → UDP  (één per bron)
crates/video/src/kijker.rs  UDP → samenstellen → decoderen → venster (één per stream)
crates/video/src/venster.rs Win32-venster met eigen swapchain
crates/video/src/kleur.rs   NV12 → BGRA via ID3D11VideoProcessor
```

De splitsing tussen `streams.rs` en `engine.rs` is dezelfde als bij de chat en om
dezelfde reden: de beslissingen zijn zonder hardware te testen, het uitvoeren niet.
Vijftien tests in `streams.rs` dekken de gevallen die met de hand niet betrouwbaar te
vinden zijn — de tweede kijker mag de encoder niet opnieuw starten, de laatste die
weggaat moet hem stoppen, een peer die wegvalt telt als weggaan, opnieuw aankondigen bij
een herverbinding mag een open venster niet dichtgooien.

**De regel die alles bij elkaar houdt: er wordt pas opgenomen en gecodeerd als er iemand
kijkt.** Een aangekondigde bron kost niets — geen capture, geen encoder, geen verkeer.
Dat is waarom je een scherm gedeeld kunt laten staan terwijl je gamet, en het is het
eerste wat je moet controleren als je hier iets verandert.

Twee tests dekken de rest:
```
cargo test -p fitcom-video --test keten     -- --ignored --nocapture   # de beeldketen
cargo test -p fitcom      --test stream_deling -- --ignored --nocapture # via de motor
```

### Screenshare meten zonder tweede machine

`deler.rs` en `kijker.rs` schrijven elk **één `info`-regel per seconde per stream**. Dat
is het enige gereedschap dat uitspraken over haperend beeld boven het gokniveau tilt:
opnemen, coderen en versturen zijn alle drie verdachte, en alleen de verhouding ertussen
wijst de dader aan.

```
deler   opgenomen_fps verstuurd_fps mbit keyframes grootste_kb frag_per_s niet_verstuurd
kijker  getoond_fps mbit frag_per_s incompleet verworpen keyframe_verzoeken decode_ms toon_ms
```

`niet_verstuurd` telt fragmenten die de socket weigerde — boven nul zit het verlies aan
de verzendkant, niet op de lijn. `incompleet` en `keyframe_verzoeken` die samen oplopen
betekenen de lus waarin elk keyframe zelf een burst is die weer verlies veroorzaakt.

Twee manieren om eraan te komen, allebei op één PC:

```
cargo test -p fitcom-video --test encoder_gedrag -- --ignored --nocapture
KETEN_SECONDEN=30 KETEN_BITRATE=8000000 cargo test -p fitcom-video --test keten -- --ignored --nocapture
.\scripts\run-peers.ps1 -Count 2   →   .\scripts\run-peers.ps1 -Logs
```

`encoder_gedrag` is de enige waar je aan kunt *rekenen*: vast patroon, vast tempo, geen
scherm nodig. De ketentest deelt het echte bureaublad en is dus nooit twee keer hetzelfde
— een stilstaand scherm levert vier beelden per seconde en een bewegend spel tweehonderd.
Gebruik hem om te zien dát de keten loopt, niet om twee runs te vergelijken. Twee
instanties met `-Logs` is de echte app; deel daar een **venster** en geen monitor, want
een monitor zet het kijkvenster in zijn eigen opname.

**Wat loopback niet nadoet:** geen MTU van 1280, geen jitter, geen verlies op de lijn.
`quinn_udp: sendmsg error 10040` gaat lokaal dus nooit af. En encoder plus decoder op
dezelfde GPU is extra last die een echte deler niet heeft. Hapering die je lokaal ziet is
geen bewijs; hapering die je lokaal *niet* ziet ook niet.

### Onderzoek 2026-07-31: de keyframe-storm, gevonden en opgelost op één machine

Eerste run met de meters, 1920×1080, budget 8 Mbit:

```
deler  opgenomen_fps=188 verstuurd_fps=188 mbit=16.6 keyframes=3 grootste_kb=260
kijker getoond_fps=189  incompleet=0 verworpen=0 keyframe_verzoeken=0
```

**De hoofdoorzaak is de ontvangbuffer van de kijker.** `crates/net/tests/burst.rs` stuurt
242 fragmenten — één keyframe van 1080p — achter elkaar naar een socket waarvan de lezer
even bezig is, precies zoals de kijker tijdens `decode` en `toon`. Van de 242 kwamen er
**59** aan. Dat is geen toeval: 59 × 1116 bytes is 64 kB, de standaard-`SO_RCVBUF` van
Windows. Eén gemist fragment maakt het hele keyframe onbruikbaar, dus de kijker vraagt een
nieuw keyframe, en die stoot sneuvelt op exact dezelfde manier. Dat is de zichzelf in
stand houdende lus uit het eerdere onderzoek, hier gereproduceerd **op loopback, waar
onderweg niets kwijt kan raken**. Het netwerk had er nooit iets mee te maken.

Drie dingen gerepareerd, elk met de meting die hem aanwees:

1. **Ontvangbuffer op 1 MB** (`net/src/media.rs`). Daarmee komen alle 242 aan. Groter is
   geen gratis winst: wat daar in de rij staat is beeld dat al te laat is.
2. **Frame pacing** (`deler.rs::Pacer`). `opgenomen_fps == verstuurd_fps` liet zien dat
   `cfg.fps` nergens werd gebruikt; WGC levert op monitortempo. Op 144-165 Hz ging er ruim
   3× zoveel de draad op. Twee valkuilen zitten in de unittests vastgelegd: de deadline
   telt op bij de vórige deadline (anders zakt 144 Hz naar 48 fps in plaats van 60), en
   bij achterstand schuift hij naar `nu` en niet naar `nu + interval` (anders gooi je
   onder het doeltempo alsnog beelden weg). De deler trekt daarna de opnamewachtrij leeg
   en codeert alleen het verste beeld — de rest is al te laat.
3. **GOP expliciet op 2 seconden** (`codec.rs`). De keyframes kwamen niet van de kijker:
   `keyframe_verzoeken=0` en er gingen er toch 3 per seconde uit. De driver koos, en die
   telt in *beelden*, niet in seconden — 60, wat bij een deler die niet paceert neerkomt
   op 3/s. **`MF_MT_MAX_KEYFRAME_SPACING` op het uitvoertype doet niets bij de
   NVIDIA-MFT**, en `CODECAPI_AVEncMPVGOPSize` werkt alleen als je hem vóór
   `SetOutputType` zet. Verkeerd gezet meldt hij niets en blijft het gewoon 1/s.

Na afloop, gemeten met `tests/encoder_gedrag.rs` (vast patroon, vast tempo, geen scherm
nodig): 0,5 keyframes/s, 3,4 Mbit op een budget van 8, en de encoder houdt precies **één**
beeld vast — 17 ms op 60 fps. Die pijplijndiepte telt in beelden en niet in tijd, en dat
verklaart waarom de ketentest op een stilstaand bureaublad 100 ms meet: bij 10 beelden per
seconde is één beeld achterstand 100 ms. Op een bewegend scherm is het 17 ms.

### Het spoor van de verversingsfrequentie, en waarom het een omweg was

Na bovenstaande drie fixes hapert het nog steeds. Wat nooit verklaard was: het gebeurt maar
één kant op. Peer 2 deelt op 180 Hz en hapert niet; Rick deelt op 144 en 165 Hz en wel.
180 ÷ 60 = 3, maar 144 ÷ 60 = 2,4 en 165 ÷ 60 = 2,75.

Uit 144 Hz zijn geen gelijkmatige zestig beelden per seconde te halen. Je krijgt er wel
zestig, met afstanden die springen tussen 13,9 en 20,8 ms:

```
120 Hz -> 60/s, spreiding 0.00 ms
144 Hz -> 60/s, spreiding 3.40 ms   <- Rick
165 Hz -> 60/s, spreiding 2.62 ms   <- Rick
180 Hz -> 60/s, spreiding 0.32 ms   <- peer 2
240 Hz -> 60/s, spreiding 0.24 ms
```

Dat leidde tot een pacer die het tempo vastklikte op een heel aantal schermbeelden: op
144 Hz elk derde, dus 48 per seconde. **Dat was een omweg, en hij is weer weg.** De reden
staat hieronder: het probleem zat niet in de verzendtiming maar in het ontbreken van een
weergaveklok. Zodra die er is, is ongelijk verzonden beeld geen probleem meer — en is
vastklikken zelfs schadelijk, want 48 monsters van een filmpje met 60 beelden gooit er
twaalf per seconde weg.

Wat van dit spoor overeind blijft: het is een goede verklaring van de asymmetrie in de
*oude* situatie, en de meetmethode (spreiding in plaats van fps) is wat het onderzoek
verder hielp.

### De echte oorzaak: er was geen weergaveklok

Het vastklikken op de verversing hielp, maar het bleef aan de zendkant sleutelen. Wat er
werkelijk ontbrak: **de kijker deed niets met de tijdstempel.** Hij toonde elk beeld zodra
het compleet was. Daarmee bepaalt de *reis* wanneer een beeld op het scherm komt —
netwerk, planning van threads, hoe vol de ontvangbuffer net zat — en niet het moment
waarop het is opgenomen. Gelijkmatig opgenomen beeld komt er dan ongelijkmatig uit, zonder
dat er één beeld verloren gaat en met een fps-teller die netjes zestig meldt.

Audio had dit al (`crates/audio/src/jitter.rs`). Video niet. Dat verschil is precies
waarom het geluid vloeiend was terwijl het beeld schokte.

En de zender maakte het erger: de tijdstempel kwam van `begin.elapsed()` op het moment van
*encoderen*, niet van het moment van opnemen. De planningsjitter van onze eigen lus zat er
dus al in voordat het pakket de deur uit ging.

Beide gerepareerd:

- `capture::Opgenomen` draagt nu `SystemRelativeTime` van de opname-API mee, en `deler.rs`
  zet die op de draad — het nulpunt van die klok wordt bij het eerste beeld naast het onze
  gelegd. Beide komen van dezelfde QPC, dus ze lopen niet uit elkaar.
- `kijker::Weergaveklok` plant elk beeld op `basis + verstreken + 30 ms`. `basis` is het
  **minimum** van `aankomst − verstreken` over een venster van twee emmers van 2 s: het
  beeld dat er het snelst over deed had de minste reistijd en bepaalt dus de beste
  schatting van hoe de twee klokken zich verhouden. Een gemiddelde zou met elke vertraging
  meelopen. Er wordt niets gesynchroniseerd tussen de machines — dat zou een tijdserver
  vragen die we niet hebben en niet willen.

Een unittest voert 300 beelden aan die gelijkmatig zijn opgenomen maar er tussen 4 en 26 ms
over deden. Zonder klok staat dat zo op het scherm; met klok is de spreiding **< 0,01 ms**.

De meter van de kijker meldt nu `spreiding_ms`: de standaardafwijking van de afstand tussen
twee getoonde beelden. **Dat is het getal dat zegt of het hapert** — `getoond_fps` kan
kloppen terwijl dit hoog staat, en dan schokt het.

Twee dingen om te weten bij het afstellen:

- `WEERGAVE_VOORSPRONG` (30 ms) is de knop. Kleiner is minder vertraging maar dan is elk
  beeld dat een paar milliseconden later binnenkomt al te laat en zijn we terug bij
  tonen-zodra-het-kan.
- De klok maakt de *reis* onzichtbaar, niet de bemonstering. Neem je 48 beelden per seconde
  van een filmpje dat er zestig heeft, dan gooi je er twaalf per seconde weg en dát zie je,
  hoe gelijkmatig die 48 ook staan. Zet `fps` daarom gelijk aan of boven het tempo van wat
  je deelt. De verversingsfrequentie van je scherm doet er níét toe — dat was de omweg
  hierboven.

**Nog open, uit het eerdere onderzoek en hier niet aangeraakt:** de heraankondiging na een
herverbinding die niet opnieuw intekent (wit kijkvenster, geen foutmelding), en
`quinn_udp: sendmsg error 10040` op datagrammen boven de tun-MTU van 1280. Die twee gaan
over verbindingen die wegvallen, niet over haperend beeld, en geen van beide is op
loopback te reproduceren.

### Onderzoek 2026-08-02: de periodieke microhapering, gevonden en gerepareerd

Rick meldde: beeld ziet er glad uit, en dan om de vijf à zes seconden één korte stotter.
Alle eerdere ronden zochten in de verkeerde hoek omdat de meters het wegmiddelen — een
`spreiding_ms` over een seconde met zestig beelden verstopt één beeld dat 150 ms te laat
is, en `incompleet=1` in een logregel ziet eruit als ruis.

**Nieuw gereedschap, en de enige reden dat dit gevonden is.** `crates/video/tests/hapering.rs`
zet een eigen venster op dat op een exact tijdraster van 60 Hz van inhoud wisselt en stuurt
dat door de échte deler en kijker. Alles wat er ongelijkmatig uitkomt hebben wij toegevoegd.
`crates/video/src/spoor.rs` schrijft daarbij één CSV-regel per beeld aan beide kanten (aan
via `FITCOM_SPOOR=<map>`). Zonder die twee blijft elke uitspraak hierover een gok.

**Wat er aan de hand was, in twee losse oorzaken.**

1. **Eén verloren UDP-fragment kostte 70 tot 156 ms bevroren beeld.** Niet omdat dat ene
   beeld weg is — dat is 17 ms — maar omdat `kijk_lus` daarna de decoder spoelt en élk
   volgend beeld weggooit tot er een keyframe is, dat eerst aangevraagd moet worden en dan
   honderden kilobytes groot is. Bij 1400 fragmenten per seconde is 0,013% verlies al
   genoeg voor een bevriezing per vijf seconden, en dat is een doodgewoon internetpad.
   *Niet* spoelen is trouwens erger: gemeten met de flush uit gaven dezelfde verliezen
   gaten van 1,2 tot 1,9 seconde, want de decoder levert daarna stil niets meer tot de
   volgende IDR. Doodlopend spoor.

2. **Elke gevraagde timeout duurde 15,6 ms.** De kijkerlus zet zijn leestimeout op 1 ms
   zodra er een beeld op zijn beurt staat, maar Windows tikt standaard op 15,6 ms en
   rondt daarheen af — gemeten: 1, 2 én 8 ms duurden alle drie 15,6 ms. Gevolg: de lus
   kwam alleen wakker als er een pakket binnenviel, dus elk beeld werd getoond op het
   moment dat het *vólgende* binnenkwam. **De weergaveklok deed daardoor helemaal niets**;
   gemeten 5,8 ms te laat, gelijkmatig verdeeld over een hele beeldtijd, en 5% van de
   beelden kwam nooit op het scherm. Gefixt met `timeBeginPeriod(1)` vanuit
   `MediaSocket::bind` (`crates/net/src/media.rs`), vastgelegd in
   `crates/net/tests/leestimeout.rs`.

**Wat er gebouwd is.**

- **Pariteitsfragment per beeld** (`fragment.rs`, `MediaHeader::FLAG_PARITEIT`): de XOR van
  alle stukken, met de lengte erin verweven zodat ook het kortere laatste stuk terug te
  rekenen is. Eén gat repareert zichzelf ter plekke — geen verzoek, geen wachten, geen
  keyframe. Twee of meer gaten volgen het oude pad. Kost 6,4% meer fragmenten.
  **`PROTOCOL_VERSION` 4 → 5**, want een oudere ontvanger stopt dat fragment als gewoon
  stuk in zijn samensteller en komt op één te veel uit: dan verschijnt er van die deler
  helemaal geen beeld meer.
- **`GOP_SECONDEN` 2 → 10** (`codec.rs`). Een keyframe is 33 tot 57× een gewoon beeld
  (371 kB tegen 6 kB op 1080p) en dat is niet te temmen — zie het doodlopende spoor
  hieronder — dus blijft alleen: minder vaak. Scheelt bijna een vijfde van het bitrate-
  budget en 5× zoveel stoten.
- **Verzendtempo** (`deler.rs`): een emmer met een gat op twintig keer het budget, met
  48 kB speling. Een gewoon beeld wacht nooit; een keyframe wordt over ongeveer één
  beeldtijd gespreid in plaats van in 1,7 ms de socket in gestampt (momentaan 1,75 Gbit/s).
- **Keyframe-verzoeken van de 100 ms-tik af** (`engine.rs`): `lees_kijkers` is
  `lees_kijker` geworden, gevoed door een doorgeefthread per kijkvenster die rechtstreeks
  in de select-lus valt. Die 100 ms zat in elke bevriezing.
- **`VersionMismatch` heeft een eigen tekst in de UI** (`ui/widgets.rs`). Zag er eerst uit
  als gewoon "offline", en met een protocolbump op komst is dat precies de verwarring die
  je niet wilt.

**Gemeten resultaat, 150 seconden, dezelfde bron, loopback:**

| | vóór | na |
|---|---|---|
| beelden die nooit getoond werden | 5,0% | 0,0% |
| spreiding tussen getoonde beelden | sd 7,44 ms | sd 2,74 ms |
| grootste gat | 137,3 ms | 31,7 ms |
| gaten boven 40 ms | 3 | **0** |
| te laat t.o.v. eigen planning | 5,80 ms | 1,06 ms |
| keyframes | 78 | 15 |
| verloren fragmenten | 3 (alle 3 een bevriezing) | 32 (alle 32 hersteld, `kapot=0`) |

Die 32 verliezen in 150 seconden zijn er één per 4,7 seconden — precies de cadans die Rick
beschreef, en nu allemaal onzichtbaar.

**Wat overblijft**, en waarschijnlijk inherent is: de tijdstempel op de draad is de
*compositietijd* van de deler, niet het moment waarop de inhoud veranderde. Voor 60 Hz-
inhoud op een 165 Hz-scherm levert dat afstanden van 18,2 en 12,1 ms op in een verhouding
3:1, en de kijker speelt dat getrouw na. Dat is de hele resterende sd van 2,7 ms. Er is
geen goedkope manier om te weten wanneer de inhoud écht veranderde — WGC geeft die
informatie niet — en de tijdstempels naar een vast raster afronden zou echte, onregelmatige
beweging (een game) juist gaan uitsmeren.

**Doodlopende sporen uit deze ronde, niet opnieuw proberen:**
- *De piek van een keyframe met de encoder begrenzen.* `CODECAPI_AVEncCommonRateControlMode`,
  `-MeanBitRate`, `-BufferSize` en `-MaxBitRate` geven allemaal `S_OK` op de NVIDIA-MFT en
  veranderen geen byte, zowel vóór als ná `SetOutputType`, op H.264 én HEVC, op drie
  resoluties. Zelfde categorie als `MF_MT_MAX_KEYFRAME_SPACING`.
- *Het wisselen van de leestimeout in `kijk_lus` als oorzaak van verlies.* Loopback
  verliest altijd het eerste datagram van een stoot na een stilte (negen van de negen);
  kale UDP zonder onze code doet exact hetzelfde, met en zonder dat wisselen. Een
  eigenschap van de Windows-loopbackstack, geen bug van ons.
- *Niet spoelen bij verlies.* Zie oorzaak 1 hierboven: erger, niet beter.

**Let op bij het uitrollen.** De protocolbump betekent dat alle drie de peers tegelijk over
moeten. Een peer op protocolversie 4 wordt bij de handshake afgewezen vóórdat er een
sessie draait, dus de automatische update van fase 11 kan hier niet overheen helpen — die
heeft een werkende verbinding nodig. Handmatig bijwerken bij alle drie.

### Miniaturen voor de overzichtstrook (fase 5)
`kijker.rs` stuurt elke 500 ms een `KijkerEvent::Miniatuur` met een verkleind BGRA-beeld,
afgeleid van het net getoonde frame via `D3dContext::lees_bgra_miniatuur` (gerichte
GPU→CPU-downscale, geen volledige framekopie). De motor bewaart de laatste per
`(PeerId, stream_id)` in `Engine::miniaturen` en publiceert hem mee in de `Snapshot`.

**Het transport naar de UI is in fase 12 vervangen.** Waar de egui-UI een
`egui::TextureHandle` per stream cachete, kijkt `ui/mod.rs` nu elke 500 ms of de
`Arc`-pointer van de data veranderd is (dezelfde vergelijking, dezelfde reden: op de
pointer en niet op de inhoud), codeert alleen dan een PNG, en stuurt een
`thumbnail`-event met een sleutel en een revisienummer. De frontend zet dat als
`thumb://localhost/<peer>-<stream>?<revisie>` op een gewone `<img>`; het protocol serveert
de bytes. Geen base64 in de JSON en geen canvas dat gevoed moet worden.

**Dit raakt het echte weergavepad niet.** De swapchain van het kijkvenster blijft het
gedecodeerde beeld rechtstreeks tonen; de miniatuur is een aftakking ernaast, niet een
omweg ervoor.

---

## Hoe desktop-audio in elkaar zit

Aankondigen en intekenen gaan via dezelfde `Streams` als screenshare, met
`StreamKind::DESKTOP_AUDIO`. Het geluid zelf gaat **over de voice-verbinding**: de
zender stuurt het vanaf zijn voice-socket, de luisteraar zet zijn *voice*-poort in de
`StreamSubscribe`, en bij de ontvanger telt de bestaande mixer het er als extra bron bij
op. De sleutel van `jitters`, `volumes` en `niveaus` is daarom `(PeerId, stream_id)` in
plaats van `PeerId`.

**Gevolg: je moet in het gesprek zitten om meegedeeld geluid te horen of te delen.** Dat
is een bewuste afweging. De alternatieven waren een eigen poort per stream (consequent
met video, maar dan heeft de ontvangende kant een tweede volledig weergavepad nodig los
van de voice-sessie) of de weergave losknippen van de microfoon (grondiger, maar dat
raakt code die werkt en al met een rooktest bevestigd is). De beperking valt in de
praktijk weg: je deelt je scherm tijdens een gesprek.

Verlaat je het gesprek terwijl je geluid deelt, dan wordt dat netjes ingetrokken — anders
blijven de anderen naar een dood adres sturen.

Verdere keuzes:
- Opus in `Application::Audio` op 96 kbit/s in plaats van `Voip` op 32. Het spraakmodel
  knijpt bij muziek de hoge tonen eruit en laat percussie rammelen.
- Geen VAD, wel een lage stiltedrempel met twee seconden hangover. Een VAD zou stukken
  uit muziek knippen; de drempel zorgt alleen dat een PC waar niets speelt geen verkeer
  veroorzaakt.

### Automatisch mee met scherm delen, en eigen stem uitgesloten (fase 10)

Geen losse aan/uit-knop meer: `Engine::deel_bron` roept `deel_bureaubladgeluid()` aan
zodra de eerste monitor of het eerste venster gedeeld wordt (en bij het joinen van een
gesprek terwijl er al gedeeld wordt), en `UiCommand::StopDelen` roept
`stop_bureaubladgeluid()` aan zodra de laatste weg is (`Engine::deelt_scherm_of_venster`).
`deel_bureaubladgeluid()` was al idempotent (niets doen als er al gedeeld wordt), dus dat
hoefde niet te veranderen. De UI toont alleen nog een passieve statusregel, niets om op
te klikken — zie beslissing 17.

**`cpal`'s gewone loopback volstond niet meer.** Die vangt ook de eigen voice-weergave
van deze app mee, en dat is onschuldig zolang je zelf moet klikken (je merkt het en zet
het uit), maar niet meer zodra het automatisch aan staat: een luisteraar zou dan zijn
eigen stem vertraagd terug horen via jouw gedeelde geluid. `bureaublad_lus`
(`crates/audio/src/session.rs`) probeert daarom eerst een proces-exclusieve
WASAPI-loopback (submodule `wasapi_capture`, via de `wasapi`-crate,
`AudioClient::new_application_loopback_client(pid, include_tree: false)`) die dit proces
expliciet uitsluit, en valt bij een fout terug op de oude `cpal`-route. Zie beslissing 17
voor het volledige ontwerp en de reden dat dit niet met raw WASAPI/COM gebouwd is.

---

## Hoe bestandsdeling in elkaar zit

```
crates/proto/src/op.rs           OpKind::FileMeta { name, size, hash } — tag 10
crates/proto/src/control.rs      FileRequest, FileResponse, FileOutcome — tags 40/41
crates/net/src/filestream.rs     24-byte header (OpId) vóór de bulkbytes op de uni-stream
crates/net/src/mesh.rs           OpenUploadStream-commando, IncomingFileStream-event
crates/store/src/timeline.rs     FileEntry, opgebouwd uit alle FileMeta-ops
crates/app/src/files.rs          Wie biedt wat aan, wie downloadt wat — pure beslislogica
crates/app/src/engine.rs         hash_en_bied_aan / upload_taak / download_taak — de I/O
crates/app/src/ui.rs             bestanden_paneel — lijst, downloadknop, voortgangsbalk
```

Dezelfde splitsing als bij chat en screenshare: `files.rs` beslist zonder schijf of
netwerk aan te raken (en is dus met unit-tests te controleren), `engine.rs` voert uit.

**Aanbieden.** De UI opent een native bestandsdialoog (`rfd`), stuurt het gekozen pad naar
de motor. Die leest en hasht het bestand op een losse tokio-taak (`hash_en_bied_aan`) —
nooit op de UI-thread, want bij een groot bestand kan hashen seconden duren. Pas als de
hash er is, wordt de `FileMeta`-op vastgelegd en gebroadcast, exact zoals een chatbericht.

**Downloaden.** De motor onthoudt bij zichzelf welk lokaal pad bij welke `OpId` hoort
(alleen voor bestanden die *wij* aanbieden — een andere peer die hetzelfde bestand
aanbiedt heeft zijn eigen entry met zijn eigen pad). Een download begint met een
`FileRequest` naar de aanbieder; die antwoordt met `FileResponse` en opent, als hij het
bestand nog heeft, een nieuwe uni-stream waar eerst de 24-byte header overheen gaat en
dan de bytes zelf, vanaf het opgegeven hervatpunt.

**Waar bestanden landen.** Tijdens het downloaden staat een deelbestand
(`<auteur-uuid>-<seq>.part`) in de downloadmap; de bestandsnaam draagt bewust niet de
leesbare naam, want twee peers kunnen hetzelfde bestand met dezelfde naam aanbieden.
Pas na een geslaagde hash-verificatie wordt het hernoemd naar zijn eigen, leesbare naam
(met `" (2)"` etc. bij een botsing). Mislukt de hash, dan verdwijnt het deelbestand en telt
een volgende poging weer vanaf 0 — er wordt niets gedeeltelijk bewaard dat niet
geverifieerd is.

**Verwijderen kan wel** (sinds fase 8, zie beslissing 13), maar niet via een apart
`FileRevoke`-bericht: de generieke `OpKind::Delete` die al voor berichten bestond, verbergt
sindsdien ook een `FileEntry` uit de timeline, en `Files::verwijder_aanbod` stopt de
aanbieder ook echt met serveren. Dit is bewust géén volledige intrekking — een download die
al liep loopt af, en wie de bytes al eerder volledig had houdt zijn kopie.

**Wat verder expres ontbreekt:** een downloadlocatie-dialoog per bestand (vaste, instelbare
map in plaats van een keuze per keer).

---

## Hoe kanalen (DM's) in elkaar zitten

```
crates/proto/src/ids.rs      Channel — algemeen of Dm(PeerId), getagd (u8, Option<PeerId>)
crates/proto/src/ids.rs      OpId, Op — kregen een channel-veld
crates/proto/src/op.rs       VersionVector — nu BTreeMap<(PeerId, Channel), u64> + visible_to()
crates/store/src/lib.rs      ops/authors-tabellen op (author, channel, seq); version_vector_for /
                             ops_missing_in_for passen visible_to() toe vóór er iets naar een
                             specifieke peer gaat
crates/store/src/timeline.rs Message/FileEntry dragen channel; Edit/Delete matchen nu ook
                             target.channel == op.channel
crates/app/src/chat.rs       stuur_op() routeert een DM naar alleen de geadresseerde; doorsturen
                             filtert op op.channel.is_public() (sinds fase 9: algemeen én
                             subkanalen, alleen een DM is uitgezonderd)
crates/app/src/files.rs      verzoek_ontvangen() weigert een DM-bestand aan iemand anders dan
                             de geadresseerde
crates/app/src/ui.rs         actief_kanaal + DM-knop per peer met ongelezen-badge
```

**De kern staat in `docs/ARCHITECTURE.md`, sectie "Kanalen"**, inclusief waarom `seq` per
(auteur, kanaal) moest gaan tellen en waarom een DM bewust geen doorstuurhulp van een
derde peer krijgt (zie ook beslissing 10 hierboven).

**Wat hier het meest kon misgaan, en dus getest is:**
- Dat een DM een derde peer nooit bereikt, óók niet via het bestaande
  doorstuurmechanisme, terwijl alle drie de peers volledig met elkaar verbonden zijn —
  `crates/app/tests/chat_sync.rs::dm_komt_aan_bij_de_geadresseerde_en_nooit_bij_de_derde_peer`.
  Dit draait door de echte mesh met echte QUIC-verbindingen over loopback, niet alleen
  op store-niveau.
- Dat een DM tussenin de aaneengesloten reeks van het algemene kanaal niet blokkeert voor
  een buitenstaander — `crates/store/tests/convergentie.rs::een_dm_blokkeert_daarna_het_algemene_kanaal_niet_voor_een_buitenstaander`.
- Dat een `Edit`/`Delete` een DM-bericht niet via het algemene kanaal kan overschrijven —
  `crates/store/src/timeline.rs::edit_in_een_ander_kanaal_dan_het_origineel_wordt_genegeerd`.
- Dat een aanvraag voor een DM-bestand door iemand anders dan de geadresseerde wordt
  geweigerd — `crates/app/src/files.rs::dm_bestand_wordt_geweigerd_aan_iemand_anders_dan_de_geadresseerde`.

**Bewust niet gedaan:** groepskanalen (meer dan twee, maar niet iedereen) — `Channel` is
al zo getagd dat dat later als nieuwe waarde bij kan zonder protocolbreuk, zie `TODO.md`.

---

## Hoe subkanalen onder het algemene kanaal in elkaar zitten (fase 9)

```
crates/proto/src/ids.rs      TopicId (nieuw, een Uuid-newtype); Channel kreeg tag 2 +
                              topic: Option<TopicId>; is_public() = tag 0 || tag 2
crates/proto/src/op.rs       OpKind::SetTopicTitle { id, title } (tag 20),
                              OpKind::DeleteTopic { id } (tag 21); visible_to() gebruikt
                              nu is_public() in plaats van is_general()
crates/proto/src/lib.rs      PROTOCOL_VERSION 2 → 3 — zie beslissing 16 en de bug hierboven
crates/store/src/lib.rs      channel_to_blob/from_blob: tag 2 hergebruikt dezelfde 16-byte
                              slot als een DM-peer (sluiten elkaar uit), dus geen
                              schema-migratie nodig
crates/store/src/timeline.rs Timeline.topics: HashMap<TopicId, String>, opgebouwd uit
                              SetTopicTitle net als nicknames uit SetNick; DeleteTopic
                              concurreert op dezelfde (lamport, author)-sleutel, dus een
                              latere hernoeming laat het subkanaal gewoon terugkomen
crates/net/src/filestream.rs encode_channel/decode_channel: tag 2 in de bestandsheader
crates/app/src/chat.rs       zet_kanaal_titel() (aanmaken én hernoemen, zelfde op);
                              ongelezen_topic: HashMap<TopicId, usize>, los van ongelezen_dm
crates/app/src/engine.rs     UiCommand::MaakKanaal/HernoemKanaal/GelezenTopic;
                              deelbestand_naam() kreeg een topic-tak (anders zou een
                              subkanaal-bestand dezelfde tijdelijke naam kunnen krijgen als
                              een algemeen bestand van dezelfde auteur met dezelfde seq)
crates/app/src/ui.rs         Sidebar toont subkanalen onder "# Algemeen" (aanmaken/
                              hernoemen/verwijderen/wisselen), verwijderen achter een
                              bevestigingsvraag; hoort_bij_kanaal() kreeg een derde tak
```

**De kern staat in `docs/ARCHITECTURE.md`, sectie "Kanalen"**, inclusief waarom dit als
`Channel::is_public()` naast `is_general()` is toegevoegd in plaats van `is_general()` zelf
op te rekken, en de volledige uitleg van de protocolversie-bump.

**Geen apart "kanaal aangemaakt"-bericht.** Zowel het aanmaken als het hernoemen van een
subkanaal is dezelfde `SetTopicTitle`-op, last-writer-wins op `(lamport, author)` per
`TopicId` — identiek aan hoe een bijnaam werkt. Er is dus ook geen "verwijder subkanaal":
dat was niet gevraagd en is niet gebouwd, net zomin als een bijnaam ooit "verwijderd"
wordt.

**Wat hier het meest kon misgaan, en dus getest is:**
- Dat een subkanaal, anders dan een DM, wél bij alle peers aankomt en via een derde peer
  doorgestuurd wordt — `crates/store/tests/convergentie.rs::subkanaal_bereikt_alle_peers_net_als_algemeen_anders_dan_een_dm`.
- Dat een subkanaal-titel en -bericht een herstart overleven, inclusief de blob-encodering
  door de echte SQLite-opslag heen — `crates/store/tests/convergentie.rs::subkanaal_titel_en_bericht_overleven_een_herstart`.
- Dat de UI een subkanaal-bericht niet met het algemene kanaal of een ander subkanaal door
  elkaar haalt — `crates/app/src/ui.rs::kanaal_tests::subkanaal_toont_alleen_zijn_eigen_berichten`.
- De kanaal-roundtrip van de bestandsoverdracht-header voor tag 2 —
  `crates/net/src/filestream.rs::tests::kanaal_roundtrip`.

---

## Hoe tags en meldingen in elkaar zitten

```
crates/app/src/tags.rs      Puur tekstwerk: bevat_tag, actieve_tag, tag_suggesties.
                             Geen state, geen UI — daarom met unit-tests gedekt.
crates/app/src/engine.rs    overweeg_melding / meld_nieuw_bericht — de meldingsbeslissing.
                             UiCommand::ZetNaam en ::NietStoren.
crates/app/src/ui.rs        Autocomplete in de chatbox, highlight van een getagd bericht,
                             het profielvenster en de niet-storenknop.
```

**Geen protocol- of storewijziging nodig.** `OpKind::SetNick` en de nickname-map in de
timeline bestonden al sinds fase 2; deze fase voegt alleen een UI-ingang toe
(`UiCommand::ZetNaam`) om er zelf een te versturen, plus tekstherkenning bovenop berichten
die er al waren. `crates/proto` en `crates/store` zijn dus ongemoeid gebleven — geen
protocol-reviewer-ronde nodig, in tegenstelling tot fase 9 die dat straks wel weer wordt.

**Wie geldt als "getagd".** `tags::bevat_tag(body, naam)` zoekt `@naam` als los woord
(hoofdletterongevoelig, met een woordgrens van niet-alfanumerieke tekens aan beide kanten)
— dat voorkomt dat `@Rick` ook `@Rickie` of een e-mailadres raakt. De naam waartegen
gecontroleerd wordt is steeds de actuele weergavenaam uit de timeline, niet een naam die bij
het opstarten is meegegeven: verandert iemand zijn naam tijdens de sessie, dan geldt de
nieuwe naam voor tags meteen, zonder herstart.

**Live versus inhaalsync, zonder aparte status.** Zie beslissing 11 hierboven en de
motivatie in `ROADMAP.md` fase 7: het onderscheid zit al in het berichttype
(`OpBroadcast` = live, `SyncResponse` = inhaalslag), dus `Engine::op_mesh_event` hoeft alleen
te kijken naar *hoe* een op binnenkwam, niet naar een los bijgehouden "peer is bijgewerkt"-
vlaggetje. Om een dubbel bezorgde broadcast geen tweede melding te laten geven, wordt vóór
en ná het verwerken de relevante ongelezen-teller (`chat.ongelezen` of `chat.ongelezen_dm`)
vergeleken — alleen een echte toename telt als "nieuw".

**Autocomplete in de chatbox (`ui.rs`).** egui's multiline `TextEdit` verwerkt Tab en Enter
zelf al tijdens `.show()` — Tab voegt een tab-teken in, Enter (zonder shift) een nieuwe
regel — vóórdat de eigen code de kans krijgt om te zien dat er een tag-suggestie
afgerond moet worden. Vandaar `App::tag_actief`: onthoudt of er vórige frame een
suggestielijst open stond, en zo ja, worden Tab/Enter dit frame uit de
toetsenbordgebeurtenissen gehaald (`ui.input_mut`) vóórdat de `TextEdit` ze ziet. De eigen
beslissing of Tab/Enter een tag moest afronden is dan al genomen (`ui.input` gelezen vóór
het strippen), dus dat gaat niet verloren. Na het invullen van een suggestie wordt de
cursor van de `TextEdit` expliciet verplaatst naar net na de ingevoegde naam
(`TextEditState::cursor::set_char_range` + `store`) — anders blijft de oude
cursorpositie hangen op een plek die na het vervangen van de tekst niet meer klopt.

**Niet-storenmodus en de profielnaam zijn sessiestate, geen config.** Net als mute/deafen:
`Engine::niet_storen` en de zojuist bewerkte naam in het profielvenster overleven geen
herstart als los concept — de naam zelf wordt wel meteen naar `config.toml` geschreven
(zodat hij bij de volgende start meteen weer klopt), maar de niet-storenschakelaar staat bij
elke start weer uit.

---

## Hoe chat-verrijking (bestanden inline, plakken) in elkaar zit

> **De UI-kant hieronder is verslag, geen beschrijving meer.** `crates/app/src/ui.rs`
> bestaat niet meer; fase 12 verving die laag. Wat er nu voor in de plaats staat:
> `ui/state.rs::timeline_of` voegt berichten en bestanden samen op dezelfde
> `(lamport, author)`-sleutel, en de drie invoerwegen komen samen in `offer_path`
> (`ui/commands.rs`) — de bestandsdialoog via `pick_and_offer_file`, slepen-en-neerzetten
> via Tauri's `DragDrop`-event, en plakken via het echte `paste`-event van de webview.
> De rest van deze sectie — waarom `lamport` op `Message` en `FileEntry` zit, en waarom
> het pad van een afbeelding uit zijn inhoudshash volgt — geldt onverkort.

```
crates/store/src/timeline.rs   Message en FileEntry kregen een lamport-veld (sorteersleutel
                                van hun eigen op) en Delete werkt nu ook op FileEntry.
crates/app/src/config.rs       resolve_pictures_dir / pictures_in — <downloadmap>/Pictures.
crates/app/src/files.rs        is_afbeelding, hash_bestandsnaam (puur), verwijder_aanbod.
crates/app/src/engine.rs       FileView kreeg lamport en hash. hash_en_bied_aan kopieert een
                                aangeboden afbeelding naar pictures_dir; download_bytes landt
                                een gedownloade afbeelding daar ook, i.p.v. in download_dir.
                                UiCommand::VerwijderAlleAfbeeldingen leegt pictures_dir.
crates/app/src/ui.rs           ChatItem (Bericht/Bestand) — de samengevoegde, gesorteerde
                                tijdlijn; App::bied_bestand_aan/verwerk_gedropte_bestanden/
                                plak_afbeelding/bijlage_texture — de drie invoerwegen en de
                                miniatuurweergave; algemeen Instellingen-venster.
```

**Waarom een `lamport`-veld op `Message` en `FileEntry` in plaats van een nieuw
`TimelineItem`-type in `store`.** `Timeline::build()` hield `messages` en `files` al apart
en al elk intern gesorteerd op `(lamport, author)` — maar zodra ze de store verlaten is dat
sorteersleutel-veld nergens meer terug te vinden op de items zelf, alleen op de `Op` waaruit
ze gebouwd zijn. Om ze in `ui.rs` te kunnen interleaven zonder de scheiding tussen store
(puur, geen UI-kennis) en app (beslist hoe het getoond wordt) te doorbreken, is het simpelst
om dat ene veld gewoon mee te geven in plaats van in `store` zelf al een gecombineerd,
UI-vormig type te bouwen. `store` blijft zo onwetend van hoe de UI berichten en bestanden
naast elkaar toont.

**`App::bied_bestand_aan` is de ene plek waar alle drie de invoerwegen samenkomen.** De
bestandsdialoog, slepen-en-neerzetten (`ctx.input(|i| i.raw.dropped_files)`) en Ctrl+V-plakken
leiden alle drie naar dezelfde functie, die zelf niets nieuws doet — hij stuurt hetzelfde
`UiCommand::BiedBestandAan` dat al sinds fase 6 bestaat. Dat is precies wat ROADMAP.md
vroeg: "alleen een nieuwe invoerweg, geen nieuwe logica". Sinds beslissing 12 hoeft deze
functie ook niets meer te onthouden over de plek van een afbeelding — dat is
`hash_en_bied_aan` in `engine.rs` gaan doen, deterministisch via de hash.

**Ctrl+V onderscheidt zich niet via een apart toetsen-event, maar via wat er op het
klembord staat.** egui levert voor tekst-plakken al een kant-en-klaar `Event::Paste`, maar
niets vergelijkbaars voor een afbeelding — het OS-klembord zit daarvoor los van wat egui via
`i.events`/`i.raw` aanbiedt. `App::plak_afbeelding` leest daarom zelf `Ctrl+V` uit de
ruwe toetsenbordstatus (los van welk veld focus heeft, zie beslissing 12) en probeert via
`arboard` een afbeelding van het klembord te lezen. Staat er geen afbeelding op (gewone
tekst, of niets), dan levert dat `None` op en gebeurt er verder niets — egui's eigen
tekst-plakken in de `TextEdit` is daarmee nooit in de weg gezeten, want die twee
klembord-inhouden (tekst met een `Event::Paste`, een afbeelding zonder) sluiten elkaar uit.
Het weggeschreven bestand staat in de OS-tijdelijke map, niet in de eigen datamap — een
blijvende plek is niet nodig, want `hash_en_bied_aan` maakt er zelf meteen een duurzame,
content-adresseerbare kopie van in `pictures_dir`.

**De miniatuur-textuurcache (`App::bijlage_texturen`) hoeft nooit ververst te worden**, in
tegenstelling tot `miniatuur_cache` voor screenshare-thumbnails: de bytes op een
content-adresseerbaar pad veranderen per definitie nooit (een andere inhoud geeft een
andere hash, dus een ander pad), dus is een simpele "eenmaal geladen, blijft geladen"-cache
op `OpId` genoeg. Het pad zelf wordt wel elke frame opnieuw berekend uit `FileView.hash`
— goedkoop, en zo is er geen aparte "is dit al gedownload"-status nodig: bestaat het
bestand nog niet, dan faalt het laden gewoon en valt de kaart terug op de generieke
weergave.

---

## Hoe automatische updates in elkaar zitten (fase 11)

> **Verouderd sinds fase 13 (2026-08-07).** Het ophaalpad hieronder is vervangen door een
> getekende release-feed over HTTPS; zie beslissing 23 en `docs/ARCHITECTURE.md`
> § Automatische updates. Wat nog klopt: `is_newer`, `app_version` in de handshake (nu
> alleen om te tónen), `fitcom-updater` en `afsluiten_voor_update`. Wat weg is:
> `overweeg_update`, `update_upload_taak`, `download_update_taak`,
> `update_verwachting_tx`, en het hervatten met `have_bytes`. De protocolvarianten
> bestaan nog maar worden gelogd en geweigerd.

```
crates/proto/src/appversion.rs   is_newer(theirs, ours) — pure tuple-vergelijking,
                                   geen semver-dependency.
crates/proto/src/control.rs      Hello/HelloAck kregen app_version; UpdateRequest/
                                   UpdateResponse (tags 42/43).
crates/proto/src/lib.rs          PROTOCOL_VERSION 3 → 4 — zie hieronder.
crates/net/src/filestream.rs     1-byte kind-prefix op elke uni-stream (0 = bestand,
                                   1 = update); read_kind/write_update_header.
crates/net/src/mesh.rs           app_version door MeshConfig/Established/Active/
                                   PeerStatus::Online.
crates/app/src/updates.rs        Updates (nieuw) — pure beslislogica, zelfde opzet als
                                   files.rs. Eén slot tegelijk, niet per peer.
crates/app/src/engine.rs         overweeg_update, update_upload_taak/download_update_taak
                                   (mirror van hash_en_bied_aan/upload_taak/download_taak),
                                   pas_update_toe, EngineHandle::afsluiten_voor_update.
crates/app/src/bin/fitcom-updater.rs (nieuw) los updater-procesje, tweede binary in
                                   hetzelfde package.
crates/app/src/ui.rs             update_beschikbaar_venster — bevestigingsvraag/voortgang.
```

**De kern staat in `docs/ARCHITECTURE.md`, sectie "Automatische updates"**, inclusief de
volledige onderbouwing van de `PROTOCOL_VERSION`-bump 3 → 4.

**Geen `FileMeta`-achtige oplog-op voor een update.** Dat was de eerste vraag bij het
ontwerp: waarom niet gewoon hetzelfde mechanisme als bestandsdeling, op-en-op? Een update
is geen chatgeschiedenis — hij hoeft niet te convergeren, niet te overleven bij een derde
peer die hem nooit zag, en betekent niets buiten "wie draait er op dit moment welke
versie". Dat is per-peer, vluchtige status, net als een RTT-meting. Vandaar twee kleine,
losstaande control-berichten in plaats van een oplog-op.

**Eén slot, niet per peer.** `Updates` houdt maar één "huidige aanbieding/download" bij.
Biedt een tweede peer dezelfde versie aan terwijl de eerste al bezig is, dan wordt dat
genegeerd — geen reden om twee keer hetzelfde te downloaden. Een nog nieuwere versie die
langskomt tijdens een lopende download wint wél, want die is per definitie beter. Een
mislukte poging wordt niet actief opnieuw geprobeerd: dat gebeurt vanzelf zodra die peer
opnieuw `Online` gaat (dezelfde "reconnect is de natuurlijke retry"-redenering als bij de
oplog-sync).

**Het kind-byte was de enige echte ontwerpvraag.** `MeshEvent::IncomingFileStream` levert
een kale `RecvStream` — er is op mesh-niveau niets dat een bestandsoverdracht van een
update-overdracht onderscheidt. De drie opties die overwogen zijn: (1) een sentinel-waarde
in het bestaande `OpId`-veld (verworpen — precies het soort impliciete aliasing dat in
fase 9 al een bug opleverde), (2) een volledig apart transportkanaal (te zwaar voor wat
uiteindelijk maar één extra byte hoeft te zijn), (3) een 1-byte kind-prefix vóór elke
stream. Optie 3 is gekozen, met als consequentie dat het bestaande bestandsoverdracht-
formaat verandert — vandaar de `PROTOCOL_VERSION`-bump, bevestigd door de
`protocol-reviewer`-agent als terecht en compleet.

**De updater is een tweede binary in `fitcom`, geen nieuwe workspace-crate.** Cargo pakt
alles in `src/bin/*.rs` vanzelf op als extra binary in hetzelfde package — dat scheelde een
`[workspace] members`-wijziging en een nieuwe `Cargo.toml` voor iets dat maar drie dingen
doet: wachten op een PID, een bestand hernoemen, opnieuw starten.

**Wat hier het meest kon misgaan, en dus getest is:**
- Dat een oude, onleesbare of ontbrekende `app_version`-string nooit een update triggert in
  plaats van te paniceren — `crates/proto/src/appversion.rs::tests::onleesbare_versie_van_een_peer_crasht_niet_en_telt_als_ouder`.
- Dat een oudere peer die het `Hello`-veld nog niet kent gewoon op `"0.0.0"` uitkomt —
  `crates/proto/src/control.rs::tests::hello_zonder_app_versie_valt_terug_op_onbekend`.
- De volledige beslislogica van `Updates` (aanbieden, negeren, een nog nieuwere versie die
  wint, voortgang, mislukking, wegklikken) — twaalf pure unit-tests in
  `crates/app/src/updates.rs`, zonder netwerk of schijf.

---

## Hoe clips in elkaar zitten (fase 15)

Een altijd-draaiende ringbuffer van het scherm dat *deze* pc opneemt; één toets schrijft de
laatste ~60 seconden weg als een MP4 die buiten de app afspeelt. **Volledig lokaal**: er
gaat niets over de draad, er hoeft niemand te kijken, en er is geen enkele peer bij
betrokken. Windows-only — `crates/app/src/clips.rs` is elders een leeg beheerobject dat
"afwezig" meldt, en de UI verbergt dan alles wat met clips te maken heeft.

```text
scherm ─► WGC ─► D3D11 ─► eigen Encoder ─► MP4-segmenten (~2 s) in <data>/clips/ring/
        loopback + microfoon ─► Menger ─► AAC (inbox-MFT) ─► tweede track per segment
sneltoets ─► laatste N segmenten remuxen ─► <data>/clips/clip-<tijdstempel>.mp4
```

**Een tweede `Encoder`, geen `IMFSinkWriter`.** De SinkWriter bezit en herstart zijn eigen
encoder per bestand; elke segmentovergang zou dan een MFT-deactivatie zijn midden in een
draaiende game (invariant 4). Hier draait één encodersessie door en is elk segment een
zelfstandig, direct afspeelbaar MP4. Muxing gaat met de `mp4`-crate (0.14). Directe NVENC
is bekeken en afgevallen: laat de muxing net zo hard aan jou over, en heeft nul ecosysteem.

**H.264, ook al kan de encoder HEVC.** De `mp4`-crate schrijft een lege standaard-`hvcC`
zonder VPS/SPS/PPS en kan die niet via de publieke API vullen — HEVC-clips initialiseren
nergens een decoder. Zelfde uitkomst als beslissing 1, om een andere reden.

**Een segment opent alleen op een keyframe en sluit alleen op een keyframe.** Openen op
iets anders maakt het segment niet zelfstandig decodeerbaar. Sluiten gebeurt zodra het
segment ouder is dan `SEGMENT_DOEL_HNS` (2 s) én het gevraagde keyframe er ook werkelijk
doorheen komt; komt de IDR niet, dan loopt het segment door tot de GOP-grens van de encoder
— een te lang segment is nooit een corrupt segment. Het verzoek (`vraag_keyframe`) wordt
elke tik herhaald zolang er op gewacht wordt: sommige encoder-MFT's slikken een eenmalig
verzoek midden in een GOP stilzwijgend in.

**Schrijven gaat naar `.part.mp4` en pas `sluit` hernoemt.** Een half geschreven segment
kan dus nooit tussen de levende metas terechtkomen. Wat er van een vorige sessie blijft
liggen gaat bij de volgende start hoe dan ook weg — zie beslissing 33, dat is de kern van
de hele tijdrekening hier.

**De ring houdt venster + `RING_MARGE_HNS` (4 s).** `te_gooien` is een pure functie over de
metadata, zodat het retentiebeleid testbaar is zonder GPU. De marge is er zodat het
nieuwste — mogelijk nog openstaande — segment de telling nooit laat kortvallen.

**Bewaren is een remux, geen her-encodering.** `bewaar_thread` draait op een eigen draad:
de beeldketen wacht er geen milliseconde op, en in de praktijk staat het bestand er binnen
een halve seconde. De basis is het **eerste keyframe op of ná de vensterrand**, niet de rand
zelf — een clip die middenin een GOP begint decodeert tot het volgende IDR niet.

**Twee tijdschalen, en dat is waar het fout ging.** De videotrack staat in hns (10 MHz,
exact onze eigen klok), de audiotrack in samples (48 kHz). Beide fouten uit beslissing 33
zaten op die grens. Wie hier iets aanraakt: `abs_hns` is altijd hns, `dur_samples` is altijd
samples, en er staat één omrekening tussen (`hns_naar_samples`).

**Geluid komt uit twee taps en wordt op één tijdlijn gemengd.** `LoopbackTap` (WASAPI,
systeem- en spelgeluid) en `MicrofoonTap` (cpal, je eigen stem), allebei genormaliseerd naar
48 kHz stereo door de tap zelf. De microfoon volgt de keuze uit de instellingen (dezelfde
`kies_apparaat` als het gesprek gebruikt) en wisselt mee zodra je hem daar aanpast — anders
neem je stilzwijgend iets anders op dan de anderen van je horen. Elke bron houdt zijn eigen klok bij; `Menger` telt ze op
dezelfde plek in de buffer op en geeft alleen vrij wat **álle** aanwezige bronnen geleverd
hebben — daarvóór kan een bron nog terugkomen met een monster dat eerder begint. Elke bron
mag apart falen: een clip zonder microfoon is nog steeds een clip met spelgeluid, en zonder
enige bron zijn het gewoon clips zonder geluid.

**De AAC-monsters wachten in een rij tot het beeld ze inhaalt.** Ze gaan het segment in
zodra er een videopakket met een latere tijd langskomt, en een monster dat vóór het huidige
segment begint wordt weggegooid — dat hoorde bij de voorganger en is daar ook aangeboden.

**De sneltoets is een eigen Win32-draad met `RegisterHotKey` en een berichtenlus.** Globaal,
dus hij werkt met de app in de tray en een spel op de voorgrond. Hij doet precies één ding:
één seintje over een kanaal naar de motor, dezelfde weg die de knop in de UI
(`UiCommand::ClipseNu`) ook neemt. Instelbaar zonder herstart; standaard F9 (één toets, ver
van de meeste gamebinds, met één hand te halen). Zie beslissing 33 voor de volgorde waarin
de oude draad eruit moet.

**Instellingen staan in `config.toml` onder `[clips]`** (`enabled`, `venster_sec`, `monitor`,
`hotkey`, `map`), alles met `#[serde(default)]`. `map` is waar de clips landen; leeg is
`<data-map>/clips`. Zelfde vorm als `download_dir` — een clip is een gebruikersbestand dat
je terugvindt en deelt, en een minuut 1080p is ~90 MB, dus "op die andere schijf" is een
redelijke wens. De ring hangt eronder (`<map>/ring`) en verhuist mee: bij het wisselen
wordt de ring van de oude map opgeruimd en begint de nieuwe leeg, precies zoals bij elke
start. Clips die er al staan blijven staan waar ze staan. Van scherm wisselen terwijl de opname loopt kan de
keten niet, dus dat herstart hem gewoon — een paar seconden opnieuw opbouwen is genoeg
troost. Monitornamen worden uniek gemaakt (`#2`, `#3`), anders is de keuze op naam ambigu
bij twee identieke schermen.

**Diagnostiek, met reden.** De opnamelus logt elke vijf seconden op `debug` wat hij tot dan
toe zag (beelden, pakketten, keyframes, of er een segment openstaat), bovenin de lus zodat
hij ook meldt wanneer er *niets* binnenkomt. Dat is er niet voor de sier: **een paniek in
een spawned thread verdwijnt naar stderr en staat niet in het logbestand**, waardoor een
dode opnamedraad zich precies zo gedraagt als een lege ring. Dat heeft één avond gekost;
zie ook "Valkuilen in deze omgeving".

De bestanden: `crates/video/src/opname.rs` (de hele keten, ring en remux),
`crates/app/src/clips.rs` (beheer, sneltoets, schermkeuze), `crates/audio/src/loopback.rs`
en `crates/audio/src/microfoon.rs` (de taps). Handmatige testpunten staan in
`docs/TESTPLAN.md`, sectie "Clips".

---

## Een release uitgeven

De volgorde hier is niet vrij: `sign --url` pint de download vast op een tag, en die tag
moet er zijn vóórdat het manifest gepubliceerd is. Precies dát ging op 2026-08-10 mis (zie
beslissing 24): op release `v0.3.2` stond een manifest dat 0.3.3 aankondigde met een URL
naar een `v0.3.3` die niet bestond. Handtekening perfect, download 404, app zwijgt.

1. Versie in `Cargo.toml` (`workspace.package.version`) op de nieuwe waarde zetten en
   committen. Dat getal is `EIGEN_VERSIE`, dus dit moet vóór het bouwen.
2. `cargo build --release` op Windows. Er komen twee bestanden uit die beide mee moeten:
   `fitcom.exe` **en** `fitcom-updater.exe`.
3. `fitcom-release sign --key <pad>\release-key.pk8 --exe target\release\fitcom.exe
   --version <X.Y.Z> --url https://github.com/PieWhite/DiscordCloneP2P/releases/download/v<X.Y.Z>/fitcom.exe`
4. `fitcom-release verify` — legt het net geschreven manifest langs de sleutel die *deze*
   build meedraagt. Dit is het enige dat een verkeerd geplakte publieke sleutel aan het
   licht brengt vóórdat iedereen stilletjes geen updates meer krijgt.
5. **Release `v<X.Y.Z>` op GitHub aanmaken met `fitcom.exe`, `fitcom-updater.exe` én
   `latest.json` erin.** De tag in stap 3 en die van de release moeten letterlijk gelijk
   zijn.
6. `fitcom-release check` — doet nu wat een gebruikersmachine doet: manifest bij
   `MANIFEST_URL` ophalen (die volgt `releases/latest`), handtekening controleren, en
   ophalen of de exe waar het manifest naar wijst er werkelijk staat. **Zolang deze stap
   geen HTTP 200 meldt, krijgt niemand de update** — en niemand ziet waarom, want een
   onbereikbare feed is voor de app een normale toestand.

`latest.json` in de repo-root is een afdruk van de laatst uitgegeven release, geen bron:
de app leest hem nooit, hij komt van de release-asset. Loopt hij achter, dan zegt dat niets
over wat er live staat — daarvoor is stap 6.

---

## Wat nog nooit met een echte peer getest is

**Fase 1 t/m 7, en kanalen (DM's) erna, zijn inmiddels allemaal door Rick met een echte
tweede (en waar relevant derde) peer bevestigd** — zie `docs/TESTPLAN.md` voor de volledige
lijst afgevinkte gevallen (2.1 t/m 2.9, 3.1 t/m 3.10, 4.1 t/m 4.12, 5.1 t/m 5.4, 6.1 t/m
6.7, K.1 t/m K.6, 7.1 t/m 7.7). Dat dekt onder meer: of de DM-knop, het kanaal-wisselen en
de ongelezen-badges in de UI echt doen wat ze beloven; of een DM tussen twee peers die
elkaar tijdelijk niet rechtstreeks bereiken zich gedraagt zoals bedoeld; een echt netwerk
met echt pakketverlies tijdens een lopende bestandsoverdracht; scherm delen op een andere
GPU (de RTX 2080 Super); of desktop-audio aan de ontvangstkant ook echt klinkt; en het
typen van `@`, de suggestielijst, de highlight, de niet-storenknop en een echte
Windows-melding. **Alleen fase 8 (deels) en fase 9 (nog helemaal) niet — zie hieronder.**

Van fase 8 zijn de bestaande geautomatiseerde ketentests (`file_deling.rs`, `chat_sync.rs`,
`timeline.rs`, plus nieuwe unit-tests op `Files::verwijder_aanbod` en het verwijderen van een
`FileEntry` in `timeline.rs`) blijven groen na het samenvoegen van berichten en bestanden tot
één tijdlijn en na het generiek maken van `Delete` — dat raakt hoe `ui.rs` de al bestaande
`Snapshot` presenteert en hoe `timeline::build()` een bekende soort op interpreteert, niet de
motor of de sync zelf. De store-wijziging is bovendien door de `protocol-reviewer`-agent
gecontroleerd vóór het committen. Twee lokale instanties starten en verbinden schoon.

**Ctrl+V-plakken is met de hand bevestigd door Rick** — pas na twee ronden, want het echte
probleem (`egui-winit` dat de toetsaanslag zelf al inslikt vóórdat de app hem ziet, zie
beslissing 15) was alleen op een echte Windows-machine met een echt klembord te vinden.
Precies het soort fout dat in deze omgeving niet te simuleren was.

**Niet geverifieerd, om dezelfde reden als bij eerdere fases — invoer naar het bureaublad kan
een script hier niet sturen:** slepen-en-neerzetten vanuit de Verkenner, of een bestandskaart
en een miniatuur er in de tijdlijn ook zo uitzien als bedoeld tussen de berichten door, het
nieuwe algemene Instellingenscherm, en de bevestigingsvraag bij "Verwijder alle
afbeeldingen". Dat moet Rick met de hand doen.

Van fase 9 (subkanalen onder het algemene kanaal) is de sync- en opslaglaag gedekt door
nieuwe tests op store- en protocolniveau (zie "Hoe subkanalen onder het algemene kanaal in
elkaar zitten" hierboven), en de `protocol-reviewer`-agent vond en de fix bevestigde de
`PROTOCOL_VERSION`-noodzaak vóór het committen. Twee lokale instanties starten en verbinden
schoon. **Niet geverifieerd, om dezelfde reden als bij eerdere fases:** het aanmaken en
hernoemen van een subkanaal via de UI, of de sidebar-lijst en de ongelezen-badge per
subkanaal er in het echt uitzien en bijwerken zoals bedoeld. Dat moet Rick met de hand doen.

**Sidenote, los van fase 9: afbeeldingen downloaden zichzelf nu automatisch, voor
iedereen.** Op verzoek van Rick tijdens dezelfde ronde: een `FileMeta`-op waarvan de naam er
als afbeelding uitziet (`files::is_afbeelding`) triggert nu meteen een download zodra hij
binnenkomt — zowel live als bij het inhalen van gemiste geschiedenis — in plaats van te
wachten tot iemand op de downloadknop klikt. Alleen bestanden die je zelf aanbiedt worden
overgeslagen (die staan al op schijf via `hash_en_bied_aan`). Elk ander bestandstype blijft
gewoon achter de bestaande bevestigingswal staan: de kaart met downloadknop, precies zoals
voorheen. Zie `Engine::op_mesh_event` in `crates/app/src/engine.rs`. **Niet geverifieerd:**
of dit in de praktijk voelt zoals bedoeld — bijvoorbeeld of een handvol afbeeldingen tegelijk
inhalen bij het opstarten niet hinderlijk aanvoelt. Dat moet Rick met de hand doen.

Van fase 10 zijn twee van de vier onderdelen pure code-/configwijzigingen zonder iets om
handmatig te verifiëren (de bugfix heeft een regressietest; resolutie bleek al overal
parametrisch, dus geaudit in plaats van gebouwd). De twee overige onderdelen raken
precies het soort gedrag dat deze omgeving niet kan simuleren:

- **Bitrate naar 12 Mbit/s** is een configwaarde; of dat de audio-lag bij een peer met een
  matige verbinding daadwerkelijk weg blijft nemen kan alleen Rick met zijn eigen twee
  machines opnieuw controleren, net als de oorspronkelijke meting.
- **Automatisch bureaubladgeluid met de proces-exclusieve WASAPI-loopback** (beslissing 17)
  compileert, en twee lokale instanties starten en verbinden schoon — maar of de
  exclude-route hier daadwerkelijk aanslaat (in plaats van stil terug te vallen op cpal),
  of het geluid bij de luisteraar goed klinkt, en of een peer zijn eigen stem echt niet
  terughoort, is met geen enkele geautomatiseerde test te dekken. Dat moet Rick met een
  tweede machine en een koptelefoon doen — zet `FITCOM_LOG=debug` aan en kijk of de regel
  "bureaubladgeluid via proces-exclusieve WASAPI-loopback" verschijnt (gelukt) of
  "terugval op gewone loopback" (mislukt, en dan graag de foutmelding erbij).
- **1440p en 3440×1440 scherp en zonder framedrops** kan alleen op de echte
  1440p-hoofdschermen gecontroleerd worden, niet in deze omgeving.

Van fase 11 (automatische updates) is de sync-onafhankelijke beslislogica volledig
unit-getest en de protocolwijziging door de `protocol-reviewer`-agent gecontroleerd, maar
alles wat hier écht om draait vergt een echt versieverschil: `run-peers.ps1` start overal
dezelfde build, dus er is geen manier om lokaal twee verschillende versies te laten praten.
**Niet geverifieerd, voor Rick met een tweede, oudere build:**
- Dat een oudere peer de nieuwere versie automatisch aangeboden krijgt zodra hij verbindt.
- Het bevestigingsvenster zelf (de voortgangsbalk, de tekst, de knoppen).
- Dat `fitcom-updater.exe` na bevestiging echt wacht tot de hoofd-app dicht is, de exe
  vervangt en de nieuwe versie start — dit is het enige stuk van deze fase dat sowieso
  nooit door een geautomatiseerde test gedekt kan worden, want het draait per definitie
  na het afsluiten van het proces dat de test erop zou controleren.
