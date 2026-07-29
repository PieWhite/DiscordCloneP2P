# Overdracht — stand van zaken

Bedoeld om in een nieuwe sessie snel weer op snelheid te komen. Wat er staat, waarom
het zo staat, waar ik tegenaan gelopen ben, en wat er nog moet.

Laatst bijgewerkt: 2026-07-30, na fase 7 (tags, meldingen, niet storen, gebruikersnaam).

---

## Status per fase

| Fase | Status | Bewezen door |
|---|---|---|
| 0 — Scaffolding | ✅ af | draait |
| 1 — Netwerklaag | ✅ af | **echt getest tussen twee PC's over Tailscale** |
| 2 — Tekstchat | ✅ af, nog niet met een echte peer getest | 19 unit/integratietests, 3 lokale instanties |
| 3 — Voice | ✅ af, nog niet met een echte peer getest | ketentests + rooktest op echte geluidskaart |
| 4 — Screenshare | ✅ af, nog niet met een echte peer getest | volledige keten op echte GPU, 55 fps op 1080p |
| 5 — Screenshare uitbreiding | ✅ af, nog niet met een echte peer getest | ketentest + motortest blijven groen na de wijziging |
| 6 — Bestandsdeling | ✅ af, nog niet met een echte peer getest | `crates/app/tests/file_deling.rs`: volledige overdracht + hash door de echte motor heen, geen GPU nodig |
| Kanalen (DM's), na fase 6 | ✅ af, nog niet met een echte peer getest | `crates/app/tests/chat_sync.rs` (drie peers, volledige mesh, echte QUIC) + `crates/store/tests/convergentie.rs` (kanaal-scoping op store-niveau) |
| 7 — Tags, meldingen, niet storen, gebruikersnaam | ✅ af, nog niet met een echte peer getest | `crates/app/src/tags.rs` (unit-tests op de tag-herkenning en cursor-parsing) + twee lokale instanties starten en verbinden schoon |

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

De UI grijst de knop uit als je niet in het gesprek zit, en verlaat je het gesprek terwijl
je geluid deelt, dan wordt dat netjes ingetrokken — anders blijven de anderen naar een
dood adres sturen.

Verdere keuzes:
- `cpal` 0.18 doet loopback vanzelf: bouw een **invoer**stroom op een **uitvoer**apparaat.
  Er is dus geen eigen WASAPI-code nodig, wat een oudere aantekening hier wel vermoedde.
- Opus in `Application::Audio` op 96 kbit/s in plaats van `Voip` op 32. Het spraakmodel
  knijpt bij muziek de hoge tonen eruit en laat percussie rammelen.
- Geen VAD, wel een lage stiltedrempel met twee seconden hangover. Een VAD zou stukken
  uit muziek knippen; de drempel zorgt alleen dat een PC waar niets speelt geen verkeer
  veroorzaakt.

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

**Wat expres ontbreekt:** een `FileRevoke`-achtige intrekking (eenmaal aangeboden blijft
voor altijd in de timeline staan, net als een bericht), en een downloadlocatie-dialoog per
bestand (vaste, instelbare map in plaats van een keuze per keer).

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
                             filtert op op.channel.is_general()
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

## Wat nog nooit met een echte peer getest is

Fase 1 is bevestigd tussen twee PC's over Tailscale. **Fase 2 t/m 6, en kanalen erna,
niet.** Zie `docs/TESTPLAN.md` voor de testgevallen die daarvoor uitgevoerd moeten worden.

Van kanalen (DM's) is de volledige keten — DM versturen, alleen bij de geadresseerde
aankomen, nooit bij de derde peer, ook niet via doorsturen — al bevestigd met drie echte
motoren over loopback-QUIC in volledige mesh (`crates/app/tests/chat_sync.rs`). Wat een
tweede en derde machine daaraan toevoegen: of de DM-knop, het kanaal-wisselen en de
ongelezen-badges in de UI in het echt doen wat ze beloven (net als bij screenshare en
bestandsdeling kon dat hier niet met de hand getest worden), en of een DM tussen twee
peers die elkaar tijdelijk niet rechtstreeks kunnen bereiken zich gedraagt zoals bedoeld
— gewoon wachten tot ze weer rechtstreeks verbinden, niet via de derde peer.

Van fase 6 is de volledige keten — aanbieden, syncen, aanvragen, streamen, hervatten,
hashen — al bevestigd via `crates/app/tests/file_deling.rs` met twee echte motoren over
loopback-QUIC, inclusief het geval waarin het bronbestand tussen aanbieden en downloaden
van schijf verdwijnt. Wat een tweede machine daaraan toevoegt: een echt netwerk met echt
pakketverlies tijdens een lopende overdracht (wordt dat netjes gemeld in plaats van
oneindig "bezig" te blijven staan?), en of de bestandsdialoog en downloadknoppen in het
echt doen wat ze beloven — net als bij screenshare kon dat hier niet met de hand getest
worden.

Fase 4 is op één machine wel volledig doorlopen, inclusief de UDP-weg over loopback en
de motor met echte QUIC-verbindingen. Wat een tweede machine daaraan toevoegt: een echt
netwerk met echt pakketverlies, een andere GPU (de RTX 2080 Super is de machine die de
codeckeuze bepaalde), en een oordeel over hoe het voelt.

Van desktop-audio is de opnamekant bevestigd op echte hardware — met geluid aan komen er
pakketten uit, zonder geluid geen enkel. Dat het aan de andere kant ook *klinkt* is niet
te controleren zonder tweede machine: op één PC tapt de loopback het geluid af dat de
andere instantie net afspeelde.

**Niet geverifieerd: de knoppen zelf.** De motor is via zijn commando's getest, maar op
"Scherm delen…" en "bekijken" is nooit echt geklikt — in deze omgeving kan een script
geen invoer naar het bureaublad sturen. Dat is dus het eerste om met de hand te doen.
Hetzelfde geldt voor fase 5: het video-instellingenscherm en de overzichtstrook zijn
alleen gecontroleerd op compileren, clippy en de bestaande ketentest/motortest (die
blijven groen na de wijziging) — niet op hoe ze er in het echt uitzien.

Van fase 7 is de tag-herkenning en de cursor-gebaseerde parsing die de autocomplete
aandrijft gedekt met unit-tests in `crates/app/src/tags.rs` (woordgrens,
hoofdletterongevoeligheid, waar de cursor precies staat). Twee lokale instanties starten en
verbinden schoon, inclusief het versturen van `SetNick` bij opstart. **Niet geverifieerd,
om dezelfde reden als bij eerdere fases:** het typen van `@` en de suggestielijst die
verschijnt, Tab/Enter die daadwerkelijk de juiste naam invult zonder een tab-teken of
nieuwe regel achter te laten, de highlight rond een getagd bericht, de niet-storenknop, en
of een Windows-melding er in het echt ook zo uitziet als bedoeld. Dat moet Rick met de hand
doen — bij voorkeur met minstens twee vensters tegelijk open, want het venster moet
verborgen/geminimaliseerd zijn voordat er überhaupt een melding komt.
