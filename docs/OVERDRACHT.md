# Overdracht — stand van zaken

Bedoeld om in een nieuwe sessie snel weer op snelheid te komen. Wat er staat, waarom
het zo staat, waar ik tegenaan gelopen ben, en wat er nog moet.

Laatst bijgewerkt: 2026-07-29, na fase 4 deel 2.

---

## Status per fase

| Fase | Status | Bewezen door |
|---|---|---|
| 0 — Scaffolding | ✅ af | draait |
| 1 — Netwerklaag | ✅ af | **echt getest tussen twee PC's over Tailscale** |
| 2 — Tekstchat | ✅ af, nog niet met een echte peer getest | 19 unit/integratietests, 3 lokale instanties |
| 3 — Voice | ✅ af, nog niet met een echte peer getest | ketentests + rooktest op echte geluidskaart |
| 4 — Screenshare | ✅ af, nog niet met een echte peer getest | volledige keten op echte GPU, 55 fps op 1080p |

**Beeld werkt van begin tot eind**: een bron aankondigen, intekenen, opnemen, coderen,
versturen, samenstellen, decoderen en tonen. Gemeten op deze machine: 1080p op 55-56
beelden per seconde, nul beelden onderweg kwijt, 3,1 ms tussen opnemen en tonen in een
debug-build.

**Desktop-audio werkt ook**: het geluid van je PC gaat mee als eigen stream, met bij de
luisteraar een volumeschuif los van je stem.

Fase 4 is daarmee compleet op wat alleen met een tweede machine te controleren valt.
Zie `docs/TESTPLAN.md`.

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

## Wat nog nooit met een echte peer getest is

Fase 1 is bevestigd tussen twee PC's over Tailscale. **Fase 2, 3 en 4 niet.** Zie
`docs/TESTPLAN.md` voor de testgevallen die daarvoor uitgevoerd moeten worden.

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
