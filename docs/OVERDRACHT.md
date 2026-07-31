# Overdracht — stand van zaken

Bedoeld om in een nieuwe sessie snel weer op snelheid te komen. Wat er staat, waarom
het zo staat, waar ik tegenaan gelopen ben, en wat er nog moet.

Laatst bijgewerkt: 2026-07-30, na fase 11 (automatische updates tussen peers) — de laatste
geplande fase uit `ROADMAP.md`.

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
                screenshare; ui.rs is een pure weergave.
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
`<hex(hash)>.<extensie>` in een eigen map (`pictures_dir`, standaard `<datamap>/Pictures`),
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

---

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

### Miniaturen voor de overzichtstrook (fase 5)
`kijker.rs` stuurt elke 500 ms een `KijkerEvent::Miniatuur` met een verkleind BGRA-beeld,
afgeleid van het net getoonde frame via `D3dContext::lees_bgra_miniatuur` (gerichte
GPU→CPU-downscale, geen volledige framekopie). De motor bewaart de laatste per
`(PeerId, stream_id)` in `Engine::miniaturen` en publiceert hem mee in de `Snapshot`. De
UI cachet zelf een `egui::TextureHandle` per stream en vergelijkt op de `Arc`-pointer van
de data, niet op de inhoud — zo wordt een ongewijzigde miniatuur niet elke frame opnieuw
naar de GPU geüpload.

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

```
crates/store/src/timeline.rs   Message en FileEntry kregen een lamport-veld (sorteersleutel
                                van hun eigen op) en Delete werkt nu ook op FileEntry.
crates/app/src/config.rs       resolve_pictures_dir — <datamap>/Pictures, naast download_dir.
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
