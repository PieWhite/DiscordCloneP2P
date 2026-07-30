# ROADMAP

Fasen worden op volgorde afgemaakt. Elke fase eindigt in iets dat draait en te testen is
met een echte tweede peer, niet in een half werkend tussenproduct.

## Fase 0 — Scaffolding ✅
Cargo workspace met de crates uit `ARCHITECTURE.md`, config-laden/opslaan,
logging (`tracing` naar bestand + console), leeg eframe-venster.
**Klaar.**

## Fase 1 — Netwerklaag ✅
QUIC-mesh over het tailnet. Handshake met protocolversiecheck, UUID-allowlist,
auto-reconnect met exponentiële backoff, RTT-meting, peerstatus in de UI.
**Klaar:** bevestigd tussen twee echte PC's over het tailnet.

## Fase 2 — Tekstchat ✅
SQLite-oplog, version-vector sync, `OpBroadcast`, chat-UI met codeblokken,
eigen berichten bewerken/verwijderen, tray + toast-notificaties.
**Klaar:** getest met drie lokale instanties en met een echte peer over het tailnet.
Een peer die later opstart haalt de geschiedenis vanzelf in; alles overleeft herstart.

Eén ding week af van het oorspronkelijke plan: de chat- en sync-lus draait op de
tokio-runtime, niet in `update()` van de UI. egui tekent niet als het venster verborgen
of geminimaliseerd is, en dan zou de synchronisatie stilvallen op precies het moment dat
je een melding zou willen krijgen. De UI is nu een pure weergave van een momentopname.

## Fase 3 — Voice ✅
WASAPI-capture via `cpal`, `nnnoiseless` ruisonderdrukking + VAD, Opus over UDP,
adaptieve jitterbuffer per spreker, lokale mix, per-deelnemer volume, mute/deafen,
expliciet join/leave.
**Klaar:** de keten van coderen tot decoderen is getest inclusief pakketverlies en
herordening; de apparaatlaag is met een rooktest op echte hardware gecontroleerd.
Of het *klinkt* moet met een tweede machine beoordeeld worden — dat kan geen test.

Geen echo-onderdrukking, conform de afspraak dat iedereen een headset gebruikt. Dat
scheelde een C++-bouwafhankelijkheid (WebRTC APM).

## Fase 4 — Screenshare, eerste versie ✅
WGC-capture van één monitor, H.264-encode via Media Foundation, UDP-fragmentatie,
decode, D3D11-render in een pop-out venster, subscribe-on-demand, desktop-audio
als aparte stream met eigen volume.

**Af:** de hele beeldketen. Aankondigen, intekenen, opnemen, coderen, versturen,
samenstellen, decoderen, tonen — plus het streambeheer eromheen en de knoppen in de UI.
Op deze machine gemeten: 1080p op 55-56 beelden per seconde, geen enkel beeld onderweg
kwijt, scherp leesbare tekst.

**Desktop-audio** gaat mee als eigen stream over de voice-verbinding, met bij de
luisteraar een volumeschuif los van de stem. Gevolg: je moet in het gesprek zitten om
mee te luisteren of mee te sturen. De afweging staat in `docs/OVERDRACHT.md`.

**Meetpunt: gedaan.** 3,1 ms tussen opnemen en tonen, in een debug-build. Dat is ver
onder alles wat opvalt, dus de encoder hoeft niet naar directe NVENC. Wat de monitor er
zelf nog bij optelt zit daar niet in en is met software ook niet te meten.

Anders dan gepland: **H.264 in plaats van HEVC** (decoderen van HEVC hangt op Windows aan
een Store-uitbreiding die er niet standaard op zit), en het kijkvenster heeft **wel een
rand** — een randloos venster kun je niet verplaatsen of sluiten zonder dat zelf na te
bouwen. Beeldvullend zit op F11. Beide staan onderbouwd in `docs/OVERDRACHT.md`.

**Klaar als:** peer B ziet peer A's scherm op 1080p60, tekst is scherp leesbaar, en
peer A merkt geen framedrops in een draaiende game. *Dat laatste kan alleen met een
tweede machine; zie `docs/TESTPLAN.md`.*

## Fase 5 — Screenshare uitbreiding ✅
Venster-capture, meerdere bronnen tegelijk delen en meerdere inkomende streams tegelijk
bekijken bleken al in fase 4 meegebouwd — de architectuur (`BronSoort::Monitor | Venster`,
`Streams` met `Vec<EigenStream>`/`Vec<VreemdeStream>`) was daar al op ingericht en
TESTPLAN 4.7/4.8 dekt het al. Wat er in fase 5 nog bij kwam:

- **Kwaliteitsinstellingen in de UI.** Codec, fps en bitrate zijn nu in de app zelf
  aan te passen ("video-instellingen" in de statusbalk) in plaats van alleen via
  `config.toml`. Lopende deelsessies herstarten meteen met de nieuwe instellingen.
- **Grid-weergave in het hoofdvenster.** Een overzichtstrook boven de chat toont een
  levend, verkleind beeld van elke stream die je bekijkt, zodat je niet tussen losse
  kijkvensters hoeft te zoeken zodra er meerdere tegelijk open staan. Blijft leeg
  zolang er niets bekeken wordt.

**Geschrapt: optionele 4:4:4-modus.** Onderzocht en afgewezen, niet uitgesteld: geen
enkele Turing-GPU — ook de RTX 2080 Super niet — kan H.264- of HEVC-4:4:4 hardwarematig
*decoderen*. Encoderen zou wel kunnen, maar dan kan niemand het terugzien. Zie
`docs/OVERDRACHT.md` en `TODO.md`.

## Fase 6 — Bestandsdeling ✅
Het hoofdbacklog-item uit `TODO.md`: bestanden delen tussen de peers.

- **Aanbieden is een gewone oplog-op** (`OpKind::FileMeta`), dus dat verspreidt zich
  gratis mee via de bestaande sync — ook naar een peer die pas veel later online komt.
  Geen apart `FileOffer`-bericht nodig.
- **Downloaden is punt-naar-punt** met de aanbieder, over een eigen QUIC-uni-stream naast
  de control-stream (`FileRequest`/`FileResponse`, tags 40/41). Een groot bestand kan zo
  nooit chat of screenshare-signalering laten wachten.
- **Hervatten na onderbreking:** de aanvrager meldt hoeveel bytes hij al heeft
  (`have_bytes`), de aanbieder seekt zijn bronbestand daarnaartoe.
- **Hash-verificatie** met BLAKE3 over het hele bestand na afloop; bij een mismatch wordt
  het weggegooid en begint een volgende poging vanzelf weer bij 0.
- **Voortgangs-UI** in een nieuw bestandenpaneel, met downloadknop, voortgangsbalk en
  "opnieuw proberen" bij een mislukte overdracht.
- Bestanden landen in een vaste, instelbare downloadmap (`download_dir` in
  `config.toml`, standaard `<datamap>/downloads`).

**Anders dan TODO.md's oorspronkelijke schets:** geen `FileOffer`/`FileChunkAck` en geen
los `offered_by`-veld — zie `docs/ARCHITECTURE.md` (sectie "Bestandsdeling") voor de
onderbouwing. Geen downloadlocatie-dialoog per download; dat is een vaste config-map,
net als de andere instellingen die nog geen eigen scherm hebben.

**Klaar als:** een aangeboden bestand verschijnt bij de andere peers zonder dat iemand
iets download, en downloaden levert byte-voor-byte hetzelfde bestand op als het origineel
— bevestigd met een geautomatiseerde test door de echte motor heen
(`crates/app/tests/file_deling.rs`, geen GPU nodig). Wat alleen met een tweede machine te
controleren is: hoe een overdracht zich gedraagt bij echt pakketverlies en een normale
netwerkonderbreking. Zie `docs/TESTPLAN.md`.

## Kanalen (DM's) — na fase 6 toegevoegd ✅
Was in `TODO.md` afgeboekt als backlog ("meer dan één chatkanaal"), maar alsnog
opgepakt: directe berichten tussen twee peers, naast het bestaande algemene kanaal.

- **`Op`/`OpId` kregen een `channel`-veld** (algemeen, of een DM met een specifieke
  peer). `seq` telt voortaan per **(auteur, kanaal)** in plaats van per auteur alleen —
  zie `docs/ARCHITECTURE.md`, sectie "Kanalen", voor waarom dat geen keuze maar een
  noodzaak is.
- **Een DM gaat nooit via een derde peer.** Het algemene kanaal profiteert van
  doorsturen bij gedeeltelijke connectiviteit; een DM bewust niet, want er is geen
  encryptie en een derde peer zou de inhoud anders kunnen lezen als hij hem doorgaf.
  Bewuste trade-off, afgestemd met de gebruiker.
- **Bestanden kunnen ook privé** aan één peer aangeboden worden — hetzelfde
  `channel`-veld op `FileMeta`, met een aanbieder die een download door iemand anders
  dan de geadresseerde weigert.
- **UI:** elke peer heeft een DM-knop met ongelezen-badge, los van de algemene
  ongelezen-teller. Bestanden- en berichtenpaneel tonen het actieve kanaal.

**Klaar als:** een DM tussen twee peers komt aan bij de geadresseerde en nooit bij de
derde, ook niet na doorstuurpogingen — bevestigd met een test door de echte mesh heen met
alle drie de peers volledig verbonden (`crates/app/tests/chat_sync.rs`), plus
kanaal-scoping op store-niveau (`crates/store/tests/convergentie.rs`).

---

## Geplande fasen (nog te doen)

Op 2026-07-30 met Rick doorgesproken en in fases gezet. Volgorde is de uitvoeringsvolgorde;
elke fase is los op te leveren en te testen zoals de fases hiervoor. Twee open vragen zijn
al beantwoord vóór het plannen: auto-update (fase 11) haalt de nieuwe versie automatisch op
maar past hem pas toe ná bevestiging, en YouTube-links (fase 8) krijgen geen voorvertoning
via een externe API — zie `TODO.md`, sectie "Afgewezen".

### Fase 7 — Tags, notificaties, niet storen, gebruikersnaam ✅
Alles rond wie wanneer een melding krijgt, en je eigen identiteit aanpasbaar maken.

- **`@username`-tags**: autocomplete in de chatbox op basis van de bestaande peerlijst
  (typen van `@` opent een gefilterde lijst, Tab/Enter vult aan). Een bericht met een
  geldige tag naar jezelf wordt in de chat gemarkeerd/gehighlight.
- **Windows-melding alleen bij een tag naar jezelf**, niet meer bij elk bericht, en alleen
  als het venster geminimaliseerd/verborgen is — dat mechanisme (tray leest zijn eigen
  events, los van de UI) staat er al, dit is een filter erbovenop.
- **Geen melding voor ingehaalde geschiedenis.** Een tag in een bericht dat binnenkomt
  tijdens het inhalen van gemiste geschiedenis (je was zelf net offline) mag geen melding
  geven — alleen een tag in een bericht dat binnenkomt terwijl je al verbonden/online was
  telt. Dit onderscheid (live broadcast vs. inhaalsync) bestaat al impliciet in hoe sync
  werkt; hier moet een moment "initiële sync met deze peer is klaar" expliciet gemarkeerd
  worden zodat de meldingslaag weet wat "live" is.
- **Niet-storenmodus**: een schakelaar (net als mute/deafen) die alle Windows-meldingen
  onderdrukt, ook een directe tag naar jezelf.
- **Gebruikersnaam wijzigen via de UI.** `OpKind::SetNick` bestaat al in `crates/proto` en
  wordt al verwerkt in `crates/app/src/chat.rs` — er is alleen nog geen UI-ingang om er zelf
  een te versturen. Toevoegen: een veld in de instellingen dat een `SetNick`-op broadcast;
  de naam verandert dan overal waar een auteur getoond wordt, bij alle peers.

**Klaar als:** een tag naar jezelf highlight in de chat, geeft een Windows-melding alleen
als het venster verborgen is én het bericht live binnenkwam, blijft stil in niet-storenmodus,
en een naamswijziging bij de ene peer verschijnt bij de andere twee zonder herstart.

**Eén beslissing afgestemd met Rick tijdens het bouwen:** een DM meldt zich, net als het
algemene kanaal, uitsluitend bij een expliciete `@jouwnaam`-tag — niet automatisch omdat
het een DM is. Dat maakt de regel overal hetzelfde: geen melding zonder een tag, ook niet
in een gesprek dat maar twee mensen zien.

**Live vs. inhaalsync zonder aparte status.** Het onderscheid dat de meldingslaag nodig
heeft ("is dit bericht net binnengekomen, of onderdeel van het inhalen van gemiste
geschiedenis") zit al in het berichttype: een `OpBroadcast` is per definitie live, een
`SyncResponse` per definitie een inhaalslag — zowel bij een verse verbinding als bij de
periodieke hersync. Er is dus geen apart "sync met deze peer is klaar"-vlaggetje nodig; zie
`docs/OVERDRACHT.md` voor de onderbouwing.

**Getest:** de tag-herkenning (woordgrens, hoofdletterongevoeligheid, de cursor-gebaseerde
autocomplete-parsing) heeft units-tests in `crates/app/src/tags.rs` — precies het soort
randgevallen dat handmatig testen mist. Beide instanties startten schoon op en verbonden
zonder fouten; **niet geverifieerd:** het typen van `@`, de suggestielijst, de highlight en
de niet-storenknop zelf aanklikken, om dezelfde reden als bij eerdere fases — dat kan in
deze omgeving niet automatisch, en moet Rick met de hand doen.

### Fase 8 — Chat verrijking: bestanden inline, plakken, links ✅
Bestanden en afbeeldingen horen in de conversatie te zitten, niet in een los paneel.

- **Slepen-en-neerzetten**: een bestand vanaf Windows naar het venster slepen start dezelfde
  aanbiedflow als de bestandsdialoog (`hash_en_bied_aan` in `engine.rs`, aangeroepen via de
  nieuwe `App::bied_bestand_aan`) — alleen een nieuwe invoerweg, geen nieuwe logica. Een
  lichte overlay ("Zet hier neer om te delen") verschijnt zolang er iets over het venster
  hangt.
- **Bestanden inline in de chat, geen apart paneel meer.** Berichten en aangeboden bestanden
  worden in `ui.rs` samengevoegd tot één chronologische lijst (`ChatItem::Bericht` /
  `ChatItem::Bestand`), gesorteerd op dezelfde `(lamport, author)`-sleutel die de store al
  per lijst aanhield. Een bestand verschijnt zo op zijn eigen plek tussen de berichten: naam,
  grootte, voortgangsbalk tijdens downloaden, downloadknop als het nog niet binnen is. Het
  losse `bestanden_paneel` is vervallen; DM-bestanden scopen nog steeds op hun kanaal, zoals
  nu. De "Bestand delen…"-knop staat voortaan naast de chatbox in plaats van in een side panel.
- **Ctrl+V afbeelding plakken**: een afbeelding op het klembord in de chatbox plakken (via
  `arboard`) schrijft hem als PNG weg in `<datamap>/geplakt` en biedt hem meteen aan via
  dezelfde flow als hierboven. Bevat het klembord geen afbeelding (gewone tekst, of niets),
  dan gebeurt er niets en blijft egui's eigen tekst-plakken in de `TextEdit` intact.
- **YouTube-links blijven kale links.** Rick heeft de externe-API-uitzondering afgewezen
  (zie `TODO.md`) — geen thumbnail, titel of kanaalnaam, gewoon een klikbare tekstlink zoals
  nu al het geval is.

**Miniatuurweergave werkt symmetrisch, voor eigen én ontvangen afbeeldingen** — een
herziening tijdens het bouwen, zie beslissing 12 in `docs/OVERDRACHT.md`. Elke afbeelding
(aangeboden of gedownload) landt onder een naam afgeleid van zijn `FileMeta.hash` in een
eigen, content-adresseerbare map (`pictures_dir`, standaard `<datamap>/Pictures`) in plaats
van in de gewone downloadmap. Aanbieder en ontvanger komen zo, zonder iets extra af te
spreken, op exact hetzelfde pad uit — de UI laadt een miniatuur simpelweg zodra het bestand
daar staat, ongeacht wie hem erheen zette.

**Ctrl+V gaat via `GetAsyncKeyState`, niet via egui's eigen toetsenbordevents.** Bleek pas
via het logbestand: `egui-winit` herkent Ctrl+V zelf al als de OS-plakopdracht en stuurt in
dat geval nooit een gewone toetsaanslag door — bevat het klembord alleen een afbeelding
(geen tekst om te plakken), dan komt er dus helemaal niets in `ctx.input()` terecht om op te
reageren, ongeacht focus. Zie beslissing 15 in `docs/OVERDRACHT.md`.

**Bestanden en foto's die je zelf aanbood zijn nu ook te verwijderen**, net als een
bericht — dezelfde generieke `OpKind::Delete` als bij chat, nu ook toegepast op
`FileEntry` in `crates/store/src/timeline.rs`. De motor stopt dan ook echt met serveren
(`Files::verwijder_aanbod`), niet alleen de kaart uit de tijdlijn; zie
`docs/ARCHITECTURE.md`, sectie "Bestandsdeling", voor wat dit wel en niet dekt (geen
terugroepen van bytes die een ander al binnen had).

**Instellingenscherm geconsolideerd.** Video-instellingen (codec/fps/bitrate) en het
nieuwe "Verwijder alle afbeeldingen" (met bevestigingsvraag, leegt alleen `pictures_dir`
op schijf) zitten nu samen in één algemeen "Instellingen"-venster in plaats van een los
video-scherm.

**Klaar als:** een gesleept, geplakt of via de dialoog gekozen bestand verschijnt als kaart op
zijn eigen plek in de tijdlijn met voortgang en downloadknop, een afbeelding toont een
miniatuur zodra de bytes er staan (bij beide kanten), een eigen bestand of foto is te
verwijderen, en er is geen apart bestandenpaneel meer in de UI. Bevestigd met de volledige
testsuite (`cargo test --workspace`, inclusief `crates/app/tests/file_deling.rs` en
`crates/store/src/timeline.rs`), een protocol-reviewer-ronde over de store-wijziging, en twee
lokale instanties die schoon opstarten en verbinden.

**Ctrl+V-plakken is met de hand bevestigd door Rick, na twee ronden.** Eerste poging (de
check aan focus van de chatbox binden) loste het niet op; het logbestand liet zien dat
`egui-winit` een Ctrl+V die het zelf als OS-plakopdracht herkent nooit als gewone
toetsaanslag doorstuurt — bevat het klembord alleen een afbeelding (geen tekst om te
plakken), dan komt er dus niets in `ctx.input()` terecht om op te reageren. Opgelost met
`GetAsyncKeyState` (rechtstreeks bij Windows, langs egui's eigen toetsenbordvertaling om).
Zie beslissing 15 in `docs/OVERDRACHT.md`.

**Niet geverifieerd, om dezelfde reden als bij eerdere fases:** het slepen zelf, en hoe een
miniatuur er in de tijdlijn daadwerkelijk uitziet — dat moet Rick nog met de hand doen.

### Fase 9 — Algemeen: meerdere subkanalen met een eigen titel ✅
Oorspronkelijk gepland als meerdere benoembare gesprekken **binnen één DM**. Op verzoek
van Rick omgedraaid tijdens het bouwen: de subkanalen horen bij **het algemene kanaal**,
niet bij een DM. Net als Discord-kanalen binnen één server — "algemeen" en "project X" als
aparte, voor iedereen zichtbare gesprekken naast het bestaande hoofdkanaal. Een DM blijft
een enkel gesprek tussen twee peers, zoals hij nu is.

- **Protocolwijziging, additief**: `Channel` krijgt een derde soort naast algemeen en DM —
  `Channel::topic(id)`, met een eigen `TopicId` (een `Uuid`, net als `PeerId`). Dezelfde
  aanpak als bij de vorige kanalen-uitbreiding: `seq` telt hierdoor ook per (auteur,
  subkanaal), en dat werkt zonder verdere wijziging omdat `seq` al per (auteur, kanaal)
  telde, niet per auteur alleen. Zie `docs/ARCHITECTURE.md`, sectie "Kanalen".
- **Een subkanaal is net zo zichtbaar als het algemene kanaal**: geen aparte
  toestemming, geen doorstuurbeperking. Het is bewust géén DM-achtige uitzondering — het
  is een extra, voor iedereen open gespreksstroom onder "Algemeen", en profiteert dus van
  dezelfde doorstuur- en hersync-robuustheid.
- **Titel via een gewone op**, net als een bijnaam: `OpKind::SetTopicTitle { id, title }`
  legt zowel het aanmaken (eerste keer gezien) als het hernoemen (latere keer, hoogste
  `(lamport, author)` wint) vast — geen apart "kanaal aangemaakt"-bericht nodig.
- **UI**: binnen "Algemeen" een lijst subkanalen, een nieuwe aanmaken met een titel,
  een bestaande hernoemen, en ertussen wisselen; berichten en bestanden blijven per
  subkanaal gescheiden, net als nu tussen algemeen en een DM. Eigen ongelezen-teller per
  subkanaal, los van het hoofdkanaal en van DM's.
- Dit moet, net als bij de vorige kanalen-uitbreiding, langs de `protocol-reviewer`-agent
  vóór het committen — dit soort wijzigingen in `crates/proto`/`crates/store` is precies
  waar eerder een reken- en een schema-fout binnenslopen (zie `docs/OVERDRACHT.md`,
  beslissing 10 en de bugs eronder).

**Klaar als:** alle peers dezelfde set subkanalen en titels zien, berichten en bestanden in
een subkanaal bij iedereen aankomen (ook bij een peer die pas later online komt) net als in
het algemene kanaal, en het aanmaken/hernoemen/wisselen in de UI werkt.

### Fase 10 — Beeld en geluid: resoluties, bitrate, gecombineerd delen
- **Resolutieondersteuning 2560×1440 en 3440×1440 (ultrawide).** SPEC ging uit van
  1080p@60Hz per persoon; de dev-PC heeft in de praktijk al een 1440p-hoofdscherm (zie
  "Valkuilen" in `docs/OVERDRACHT.md`). Uitzoeken en wegnemen van elke plek die stilzwijgend
  1920×1080 aanneemt: video-instellingen-UI, encoder/decoder-init, kijkvenster-formaat,
  miniaturen-downscale. WGC capture zelf is al resolutie-onafhankelijk.
- **Bitrate-standaarden herzien**: 12 Mbit/s als standaard voor 1080p@60fps (nu ~25 Mbit/s).
  Dit gaat over de vastgelegde onderbouwing in SPEC.md heen ("bij 1 Gbit zijn bits gratis"),
  maar niet omdat die redenering fout was voor de lokale tailnet-verbinding — het probleem
  zit bij een peer met een minder goede eigen internetverbinding. Rick heeft gemeten dat de
  ~25 Mbit-stream bij zo'n peer lag veroorzaakte in de audio van **degene die streamt**
  (niet bij de kijker zelf), en dat die lag wegviel bij 12 Mbit zonder merkbaar
  kwaliteitsverlies. Dus geen esthetische keuze maar een gemeten regressie op precies de
  eis die bovenaan SPEC.md staat ("geen merkbare impact op een draaiende game/voice
  daarnaast"). SPEC.md moet bij het bouwen van deze fase een nieuwe passage krijgen die dit
  vastlegt — anders leest een latere sessie de oude "bits zijn gratis"-redenering en denkt
  dat 12 Mbit een vergissing is. Voor 1440p@60fps en 3440×1440@60fps geldt dezelfde
  waarschuwing: niet zomaar omhoog schalen naar "wat gebruikelijk is voor die resolutie"
  zonder opnieuw te meten bij een peer met een matige verbinding — het mechanisme achter de
  lag (waarschijnlijk audio die achter dezelfde verbinding/CPU wacht als de videostream) is
  nog niet uitgezocht, alleen het symptoom is bevestigd.
- **Geluid delen ingebouwd bij scherm delen.** Geen losse aan/uit-knop meer: desktop-audio
  start automatisch zodra je een monitor/venster deelt en stopt automatisch zodra je stopt.
  Blijft technisch een aparte stream met eigen volumeschuif bij de luisteraar, zoals nu.
  - **Eigen stem niet laten terugkomen.** Als de gedeelde desktop-audio de eigen
    voice-chat-weergave van de app zou meecapturen, hoort een peer zijn eigen stem
    vertraagd terug via jouw gedeelde geluid. Windows heeft sinds versie 2004 een
    proces-specifieke loopback-capture die een proces juist kán *uitsluiten*; of die
    exclude-modus op alle drie de machines beschikbaar is, is nog niet uitgezocht — dit is
    een onderzoekspunt (`media-research`-agent) vóór er iets gebouwd wordt, geen aanname.
- **Bugfix, los van de rest oppakbaar**: specifiek stoppen met audio delen laat de andere
  partij nu nog gewoon geluid horen. Vermoedelijk mist het stop-pad een
  unsubscribe/teardown-stap specifiek voor `StreamKind::DESKTOP_AUDIO`, terwijl het
  video-stoppad dat al wel goed doet. Kleinste, meest losstaande item in deze fase — kan
  het eerst.

**Klaar als:** beide nieuwe resoluties scherp en zonder framedrops getoond worden, de
nieuwe bitrate-defaults in SPEC.md staan met hun onderbouwing, scherm delen automatisch
geluid meeneemt zonder een eigen knop, stoppen met delen ook echt stil is bij de
ontvanger, en een peer zijn eigen stem niet hoort terugkomen via gedeeld bureaubladgeluid.

### Fase 11 — Automatische updates tussen peers
- **Versievergelijking bij de handshake**: naast de bestaande `protocol_version` een
  app-versie (semver) uitwisselen zodat een peer kan zien dat een ander een nieuwere build
  heeft.
- **Automatisch ophalen, pas toepassen na bevestiging** — dit is expliciet met Rick
  afgestemd (zie boven): zodra een nieuwere versie bij een peer gezien wordt, wordt de
  nieuwe exe op de achtergrond gedownload via het bestaande bestandsdelingsmechanisme
  (punt-naar-punt, hervatbaar, BLAKE3-geverifieerd — vrijwel hergebruik van fase 6). Pas als
  hij volledig binnen en geverifieerd is, vraagt de app: "Peer X heeft versie Y, nu
  bijwerken en herstarten?". Alleen bij bevestiging wordt hij toegepast.
- **Toepassen**: een exe kan zichzelf niet overschrijven terwijl hij draait op Windows. Een
  klein los updater-proces wacht tot de hoofd-app afgesloten is, vervangt de exe en start
  hem opnieuw op.
- **Vertrouwensgrens verschuift.** Dit is de eerste functie waarbij "een peer vertrouwen"
  betekent "code van een peer uitvoeren", niet alleen zijn chat/bestanden lezen. Dat past
  binnen de bestaande aanname (tailnet + UUID-allowlist is de beveiligingsgrens, zie
  `TODO.md`, sectie "Beveiliging"), maar is wel een principieel andere stap dan de fases
  hiervoor — expliciet benoemd zodat dat niet stilzwijgend gebeurt.

**Klaar als:** een peer met een oudere versie krijgt de nieuwe automatisch aangeboden zodra
hij weer online komt, ziet een duidelijke bevestigingsvraag, en draait na akkoord de nieuwe
versie zonder dat iemand handmatig een zip hoeft uit te pakken.
