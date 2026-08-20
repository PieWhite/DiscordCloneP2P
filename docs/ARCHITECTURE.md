# ARCHITECTURE

Hoe we bouwen. Requirements staan in `SPEC.md`.
**Lees dit voordat je iets aan het wire-protocol of de sync-logica wijzigt.**

## Techstack

| Laag | Keuze | Waarom |
|---|---|---|
| Taal | Rust 1.86, MSVC | Toolchain staat al; geen .NET SDK aanwezig; één statische exe; geen GC-pauzes in de audiopijplijn |
| UI | **Tauri v2** (WebView2), sinds fase 12 | Ontwerpplafond: egui kon geen echte typografie, ritme of gelaagdheid, CSS wel. Tekent alleen bij verandering in plaats van 4 fps polling. Zie `PRODUCT.md` → `## Stack` |
| Async | `tokio` | Standaard, past bij quinn |
| Control/chat | `quinn` (QUIC) | Betrouwbaar, meerdere onafhankelijke streams over één verbinding, geen head-of-line blocking tussen berichttypes, ruimte voor bulk file transfer later |
| Media | `tokio::net::UdpSocket` | Retransmits zijn schadelijk voor realtime audio/video |
| Wire-formaat | `serde` + `rmp-serde` (MessagePack) voor control; hand-rolled binaire header voor media | Compact, tolerant voor toegevoegde velden, dumpbaar als JSON voor debugging |
| Opslag | `rusqlite` (bundled) | Geen externe dependency, één bestand |
| Schermcapture | `windows` crate → Windows.Graphics.Capture + D3D11 | Microsofts eigen binding; monitor én venster; blijft op de GPU |
| Video encode/decode | Media Foundation Transform via `windows` crate, achter een trait | Geen NVIDIA SDK of CUDA nodig; vendor-agnostisch; D3D11-textuur als input |
| Audio I/O | `cpal` (WASAPI); losse WASAPI-loopback via `windows` crate voor desktop-audio | |
| Audio codec | Opus (`opus` crate) | Vereist `cmake` om libopus mee te bouwen; alleen bij het bouwen, niet bij de gebruiker |
| Noise suppression | `nnnoiseless` | Pure Rust, geen C++ build-dependency |

### Encoder achter een trait
De video-encoder zit achter `trait VideoEncoder` met een D3D11-textuur als input.
We starten met Media Foundation omdat dat nul extra dependencies kost. Als de gemeten
glass-to-glass latency tegenvalt, wisselen we naar **directe NVENC** (via
`nvidia-video-codec-sdk` of eigen FFI naar `nvEncodeAPI.dll` met
`NV_ENC_DEVICE_TYPE_DIRECTX`) zonder de rest te raken. Meetpunt zit in fase 4.

### De grens tussen UI en de rest
De UI is een pure weergave: hij leest een `watch`-momentopname (`Snapshot`) en stuurt
`UiCommand`'s terug over kanalen. Geen enkele beslissing over netwerk of opslag staat in de
weergavelaag, en er staat geen state in die verloren gaat als het venster niet tekent.

**Dat is niet alleen netjes, het is de reden dat de stackwissel naar Tauri haalbaar
was.** Gemeten vóór dat besluit en opnieuw erna: buiten `crates/app/src/ui/` komt egui
alleen voor in doc-commentaar — geen enkele API-aanroep in `proto`, `store`, `net`, `audio`
of `video`. Te vervangen viel dus alleen `crates/app/src/ui/` en de vensterbootstrap in
`main.rs`. `Snapshot`/`UiCommand` zijn Tauri-commands en -events geworden: `ui/state.rs`
vertaalt de momentopname naar JSON, `ui/commands.rs` vertaalt IPC-aanroepen terug naar een
`UiCommand`. Laat die grens intact — hem opgeven maakt een volgende wissel weer duur.

### Waarom video in een apart venster
Elke stream krijgt een eigen Win32-venster met eigen D3D11 swapchain, op een eigen thread
met eigen message pump. Volledig geïsoleerd van de UI, optimaal videopad, en een
maximaliseerbaar los venster is voor het echte kijken de betere UX — met twee schermen per
persoon kan het naast een game op het andere scherm staan.

**Dit staat los van de UI-stack en verandert niet mee.** Het oorspronkelijke argument was
dat `eframe` via wgpu (DX12) rendert en een gedecodeerde D3D11-textuur daarin
shared-handle-interop op `wgpu-hal`-niveau zou vereisen — onveilig, fragiel, en niet nodig.
Met een webview wordt een D3D11-textuur in de UI krijgen alleen maar moeilijker, dus de
conclusie wordt sterker in plaats van zwakker: het hete videopad raakt de UI-stack nergens.

Wat de UI wél nodig heeft is de overzichtstrook met verkleinde live beelden (fase 5): elke
500 ms een GPU-naar-CPU-downscale (`D3dContext::lees_bgra_miniatuur`), buiten het
eigenlijke weergavepad om. Dat was een egui-textuur en is sinds fase 12 een
`thumbnail`-event met een revisienummer plus een `thumb://`-protocol dat de PNG aan een
gewone `<img>` serveert. Bij 2 fps en een kleine afmeting verwaarloosbaar.

Het venster houdt zijn gewone rand: zonder rand valt hij niet te verplaatsen, te
vergroten of te sluiten zonder eigen hit-testing, en dat is nergens voor nodig.
Beeldvullend zit op F11 en dubbelklik, zonder modeswitch — er kan een game op datzelfde
scherm draaien. De swapchain houdt de afmeting van de stream en DXGI schaalt bij het
presenteren; het venster bewaakt zelf de beeldverhouding.

### Waarom elke kijker een eigen UDP-poort bindt
Video kan niet over de voice-poort: die is bezet zodra je in een gesprek zit. De kijker
bindt daarom per stream zijn eigen poort en zet die in `StreamSubscribe.media_port`.
Daardoor is er niets te demultiplexen — één socket, één stream, één thread, één venster —
en botsen video en voice nergens.

### NV12 naar BGRA
De decoder levert NV12, een swapchain wil BGRA. Die omzetting gaat via
`ID3D11VideoProcessor`: vaste-functie-hardware die elke GPU hiervoor heeft. Het
kleurbereik komt uit het onderhandelde uitvoertype van de decoder en niet uit een
aanname — fout gezet levert geen fout op maar een verwassen beeld, en dat zie je zelf
niet omdat je alleen je eigen scherm kent.

## Processtructuur

```
main thread            weergavelaag — pure weergave van een momentopname
tokio runtime          motor (oplog + mesh), quinn control-mesh, UDP media-sockets
capture thread(s)      WGC → D3D11 texture → encoder → UDP  (één per gedeelde bron)
render thread(s)       UDP → decoder → D3D11 swapchain      (één per bekeken stream)
audio capture thread   WASAPI → nnnoiseless → VAD → Opus → UDP
audio render thread    UDP → jitterbuffer → Opus decode → mix → WASAPI
```

Alle threads praten met de UI via kanalen. Geen gedeelde locks op het hot path.

### De UI mag stilvallen
Een verborgen of geminimaliseerd venster tekent niet. Alles wat door moet lopen terwijl je
niet kijkt — synchronisatie, herverbinden, meldingen — hoort daarom níét in de weergavelus.
De motor (`app/src/engine.rs`) bezit de mesh en de oplog en draait op de tokio-runtime; de
UI leest een `watch`-momentopname en stuurt commando's terug. Om dezelfde reden leest de
tray-thread zijn eigen gebeurtenissen: zou de tray-klik in de UI worden afgehandeld, dan
kon je een verborgen venster nooit meer terughalen.

Dit gold voor egui's `update()` en geldt onverkort voor een webview — **de reden is niet
toolkit-specifiek.** Het is een van de invarianten die de stackwissel moet overleven.

## Voice

```text
microfoon ─► cpal-callback ─► opname-thread ──► UDP naar alle deelnemers
                              (herbemonsteren, ruisonderdrukking, VAD, Opus)

UDP ─► ontvang-thread ─► jitterbuffer per spreker ─► mix-thread ─► cpal-callback ─► koptelefoon
```

- 48 kHz mono, 20 ms frames, Opus op 32 kbit/s met inband-FEC.
- **De cpal-callbacks doen niets dan samples in of uit een kanaal schuiven.** Ze draaien
  op threads van de geluidsdriver; wat daar te lang duurt is een hoorbare klik.
- **De mix-thread laat zich aandrijven door de geluidskaart**, niet door een timer: hij
  maakt pas een frame bij als de weergavebuffer ruimte heeft. Op een timer zou hij
  onvermijdelijk uit de pas lopen met de kaart en dan loopt de buffer vol of leeg.
- **Er wordt alleen verstuurd als de VAD spraak ziet**, met 500 ms hangover zodat het
  eind van woorden niet wordt afgekapt. Stil betekent geen verkeer en geen CPU.
- **Geen echo-onderdrukking.** Dat mag omdat iedereen een headset draagt; het scheelt
  een C++-bouwafhankelijkheid (WebRTC APM). Zie `SPEC.md`.
- Mixen gebeurt in `i32` en klemt daarna naar `i16`. Optellen in `i16` klapt om bij drie
  harde sprekers, en dat klinkt als een explosie in plaats van als hard geluid.

### Jitterbuffer
Telt frames, geen milliseconden — de geluidskaart *is* de klok. Daardoor is het gedrag
bij verlies en herordening exact te testen zonder te wachten.

Hij groeit snel bij problemen en krimpt langzaam bij rust. Andersom zou hij bij elke hik
terugvallen en opnieuw moeten opbouwen, wat je hoort als herhaalde onderbrekingen in
plaats van eenmalig wat extra vertraging. Loopt hij ver vol — de zender staat voor, of we
hebben gehaperd — dan gooit hij het oudste weg: één hapering is beter dan een halve
seconde vertraging die nooit meer weggaat.

## Wire-protocol

### Control (QUIC, betrouwbaar)
Length-prefixed MessagePack. Elke peer opent één QUIC-verbinding naar elke andere peer;
bij een botsing wint de verbinding waarvan de initiator de laagste UUID heeft.

```rust
enum ControlMsg {
    Hello { protocol_version: u32, peer_id: Uuid, display_name: String, app_version: String },
    HelloAck { protocol_version: u32, peer_id: Uuid, display_name: String, app_version: String },
    Ping { nonce: u64 },
    Pong { nonce: u64 },

    // chat + generieke state
    SyncRequest  { have: VersionVector },
    SyncResponse { ops: Vec<Op> },
    OpBroadcast  { op: Op },

    // media
    VoiceJoin  { media_port: u16 },
    VoiceLeave,
    StreamAnnounce   { stream_id: u32, kind: StreamKind, title: String },
    StreamRevoke     { stream_id: u32 },
    StreamSubscribe  { stream_id: u32, media_port: u16 },
    StreamUnsubscribe{ stream_id: u32 },
    StreamStats      { stream_id: u32, loss_pct: f32, rtt_ms: u32 },

    // file transfer — zie "Bestandsdeling" hieronder
    FileRequest  { file: OpId, have_bytes: u64 },
    FileResponse { file: OpId, outcome: FileOutcome },

    // automatische updates (fase 11) — zie "Automatische updates" hieronder
    UpdateRequest  { have_bytes: u64 },
    UpdateResponse { outcome: FileOutcome, size: u64, hash: [u8; 32] },
}
```

`StreamKind` is een `u8` op de draad met vier bekende waarden: `MONITOR = 1`,
`WINDOW = 2`, `DESKTOP_AUDIO = 3` en `CAMERA = 4`. Een camerastream is op de draad niet
van een gedeeld scherm te onderscheiden — dezelfde H.264 in dezelfde fragmenten — maar de
app moet het verschil weten, want bureaubladgeluid hangt aan een gedeeld *scherm* en niet
aan een camera. `CAMERA` kwam erbij **zonder `protocol_version`-bump**: een peer die hem
niet kent ziet `is_known() == false`, logt de aankondiging en tekent er niet op in. Dat is
precies het gedrag dat bij een onbekende soort hoort, dus een oude peer verliest alleen de
camera en blijft voor al het andere werken.

Regels voor uitbreiding:
- Alleen **toevoegen** aan het eind van enums en structs. Nooit varianten hernummeren of verwijderen.
- Nieuwe struct-velden krijgen `#[serde(default)]`.
- `protocol_version` alleen ophogen bij een breuk die niet met defaults op te vangen is.
- Onbekende varianten worden gelogd en genegeerd, niet als fout behandeld.

### Media (UDP, onbetrouwbaar)
Vaste 16-byte header, daarna payload. Handgeschreven, geen serde — per-pakket overhead telt.

```
offset  size  veld
0       4     stream_id   u32  (voice = 0, screenshare = 1.., desktop-audio = eigen id)
4       4     seq         u32  monotoon per stream
8       4     timestamp   u32  90 kHz voor video, 48 kHz voor audio
12      1     payload_type u8  (0=opus, 1=hevc, 2=h264)
13      1     flags       u8   bit0 = keyframe, bit1 = laatste fragment, bit2 = pariteit
14      2     frag_index  u16
```

Videoframes worden gefragmenteerd op 1100 byte payload (onder Tailscale's MTU).
Een frame is compleet als alle fragmenten 0..n binnen zijn met bit1 gezet op de laatste.
Incomplete frames worden gedropt; bij verlies van een keyframe vraagt de ontvanger via
control om een nieuwe IDR.

#### Het pariteitsfragment (bit2)

Achter elk videobeeld gaat één extra fragment aan met de XOR van alle stukken, elk
voorafgegaan door zijn eigen lengte en aangevuld tot 1100 bytes. Zijn `frag_index` is het
*aantal* stukken in plaats van een plek erin, zodat de ontvanger weet hoeveel hij er hoort
te hebben ook als juist het stuk met bit1 zoek is. Mist er precies één fragment, dan rekent
de ontvanger het terug — inclusief zijn lengte, want die zit in de XOR. Bij twee of meer
gaten gebeurt er niets en volgt het oude pad: keyframe vragen.

**Waarom dit er is.** Gemeten op 2026-08-02 met een gecontroleerde bron
(`crates/video/tests/hapering.rs`): één verloren fragment kostte **70 tot 156 ms bevroren
beeld**. Niet omdat dat ene beeld weg was — dat is 17 ms — maar omdat de kijker daarna zijn
decoder spoelt en alles weggooit tot er een nieuw keyframe is, dat hij eerst moet aanvragen
en dat honderden kilobytes groot is. Bij 1400 fragmenten per seconde is 0,013% verlies —
gewoon voor een internetpad — al genoeg voor een bevriezing per vijf seconden. Verreweg de
meeste verliesmomenten zijn precies één fragment groot, en daarvoor is een terugweg over
het netwerk absurd duur.

Kost één pakket per beeld: gemeten +6,4% fragmenten en +6,6% bytes.

**Doorgemeten alternatief dat níét werkt:** de piek van een keyframe met de encoder
begrenzen. `CODECAPI_AVEncCommonRateControlMode`, `-MeanBitRate`, `-BufferSize` en
`-MaxBitRate` geven allemaal `S_OK` op de NVIDIA-MFT en veranderen geen byte, zowel vóór
als ná `SetOutputType`, op H.264 én HEVC. Een keyframe blijft 33 tot 57× een gewoon beeld.
Wat wél kan: minder vaak (`GOP_SECONDEN` staat daarom op 10, niet op 2) en de stoot
spreiden (`deler::Verzendtempo`).

De `timestamp` van een videopakket komt uit het sample van de encoder, niet uit "nu" op
het moment van versturen. Fragmenten horen bij elkaar dóórdat ze dezelfde tijdstempel
dragen, dus die moet per beeld gelijk zijn en tussen beelden verschillen. Alle streams
van één peer delen bovendien één klok, zodat hun tijdstempels vergelijkbaar zijn.

## Chat-synchronisatie

Op-based CRDT met per-(auteur, kanaal) dichte sequentienummers. Geen vector clocks nodig
voor convergentie — de version vector volstaat omdat alleen de auteur zelf zijn eigen
ops nummert, dus er zitten nooit gaten in.

```rust
struct Op {
    author: Uuid,        // wie de op maakte
    channel: Channel,    // algemeen, een subkanaal, of een DM — zie "Kanalen" hieronder
    seq: u64,            // per (auteur, kanaal) monotoon, 1..N, geen gaten
    lamport: u64,        // voor totale ordening tussen auteurs
    wall_clock: i64,     // alleen voor weergave, nooit voor correctheid
    kind: OpKind,
}

enum OpKind {
    Post   { body: String },
    Edit   { target: OpId, body: String },
    Delete { target: OpId },
    SetNick{ name: String },
    FileMeta{ name: String, size: u64, hash: [u8; 32] },
    // later: React, Reply — nieuwe varianten, geen migratie
    SetTopicTitle{ id: TopicId, title: String },  // fase 9, zie "Kanalen"
    DeleteTopic{ id: TopicId },                    // fase 9, zie "Kanalen"
    WordleResult{ day: u32, guesses: u8, solved: bool, pattern: String },  // zie "Wordle"
}
```

- `OpId = (author, channel, seq)` is globaal uniek. Opslag is idempotent: dezelfde op
  tweemaal toepassen is een no-op. Dat is de hele conflictafhandeling.
- **Weergavevolgorde** is `(lamport, author)`. Lamport wordt bij elke ontvangen op
  bijgewerkt naar `max(local, remote) + 1`.
- `Edit`/`Delete` zijn last-writer-wins op `target`, gewonnen door de hoogste
  `(lamport, author)`. Renderen vouwt ze over de `Post` heen.
- **Version vector** = `{(author, channel) → hoogste *aaneengesloten* seq}`, niet
  `max_seq`.

  Ops worden dicht genummerd, maar komen niet per se in die volgorde binnen: we kunnen
  bij B de ops 6-10 van auteur A ophalen terwijl A zelf al 11 broadcast. Landt die 11
  eerder, dan hebben we 1-5 en 11, met een gat. Melden we dan `max = 11`, dan zeggen we
  "ik heb alles t/m 11" en krijgen we 6-10 nooit meer.

  De op met een gat ervoor wordt wél bewaard, maar telt pas mee zodra het gat gedicht is.
  Om dezelfde reden versturen we nooit ops voorbij onze eigen aaneengesloten reeks —
  anders erft de ontvanger ons gat zonder het te weten.

  Sync bij (her)verbinding:
  1. Beide kanten sturen `SyncRequest { have }`, beperkt tot wat de ontvanger ooit mag
     zien — zie "Kanalen" hieronder.
  2. Elke kant stuurt terug wat de ander mist: alle ops waarvan
     `seq > peer.have[(author, channel)]`, voor elke (auteur, kanaal)-paar dat de
     ontvanger mag zien.
  3. Klaar. Convergentie in één ronde, ook na maanden offline.
### Drie wegen waarlangs een op zich verspreidt
1. **Broadcast** bij het plaatsen — het normale geval waarin iedereen online is. Voor een
   DM-op is dit geen breed broadcast maar een `Send` naar uitsluitend de geadresseerde.
2. **Inhaalslag bij (her)verbinding** — dekt de peer die weg was.
3. **Doorsturen plus periodieke hersync** — dekt gedeeltelijke connectiviteit: A en C
   kunnen elkaar niet bereiken, B beiden wel. Ontvangen ops die nieuw voor ons waren
   sturen we door; ops die we al kenden niet, en dáármee stopt de lus vanzelf. Daarnaast
   sturen we elke 30s ongevraagd onze version vector rond. Dat kost enkele tientallen
   bytes en herstelt elke toestand die 1 en 2 gemist zouden hebben.

   **Uitzondering: een DM-op wordt nooit doorgestuurd, punt 3 slaat er niet op.** Zie
   "Kanalen" hieronder voor waarom.

Een algemene op is nooit verloren zolang één peer hem heeft. Voor een DM geldt dat
alleen voor de twee betrokkenen zelf — zie hieronder.

### Weergaveregels
- Sorteren op `(lamport, author)`. `wall_clock` mag nooit meedoen: de klokken van de
  drie PC's lopen uiteen en dan zou de volgorde per peer verschillen.
- `Edit`/`Delete` tellen alleen als `op.author == target.author` **én**
  `op.channel == target.channel`. Zonder de auteurscheck kan iedereen andermans tekst
  herschrijven; zonder de kanaalcheck zou een edit-op op het algemene kanaal (dus breed
  gesynchroniseerd) de tekst van een DM-bericht kunnen overschrijven en zo alsnog laten
  lekken. Beide zijn in een append-only log niet terug te draaien.
- Per bericht wint de `Edit`/`Delete` met de hoogste `(lamport, author)`.

### SQLite-schema
```sql
CREATE TABLE ops (
  author     BLOB    NOT NULL,     -- 16-byte uuid
  channel    BLOB    NOT NULL,     -- 17 bytes: 1 tag-byte + 16-byte peer (nullen indien afwezig)
  seq        INTEGER NOT NULL,
  lamport    INTEGER NOT NULL,
  wall_clock INTEGER NOT NULL,
  kind       INTEGER NOT NULL,
  payload    BLOB    NOT NULL,     -- MessagePack van de kind-specifieke velden
  PRIMARY KEY (author, channel, seq)
) WITHOUT ROWID;
CREATE INDEX ops_order ON ops(lamport, author);

CREATE TABLE peers (peer_id BLOB PRIMARY KEY, display_name TEXT, address TEXT, last_seen INTEGER);
CREATE TABLE meta  (key TEXT PRIMARY KEY, value BLOB);  -- eigen uuid, lamport, schema-versie
```

## Kanalen

Drie soorten. Naast het algemene kanaal (iedereen ziet alles) bestaat een subkanaal onder
het algemene kanaal (fase 9: net zo publiek als "Algemeen" zelf, alleen met een eigen
naam en een eigen berichten-/bestandenstroom — zie hieronder), en een direct bericht (DM):
een gesprek tussen de auteur van een op en precies één andere peer, die de rest van de
mesh nooit te zien krijgt.

```rust
struct Channel {
    tag: u8,               // 0 = algemeen, 1 = DM, 2 = subkanaal — als (tag, peer, topic)
    peer: Option<Uuid>,    // op de draad, net als StreamKind/FileOutcome, zodat een later
    topic: Option<TopicId>,// kanaalsoort geen decodeerfout geeft bij een peer die hem nog
}                          // niet kent. `peer` en `topic` sluiten elkaar uit (bepaald door tag).
```

`Channel::dm(other)` betekent: dit is een bericht tussen `op.author` en `other`. Een
gesprek tussen A en B bestaat dus uit twee onafhankelijke opstromen — A's eigen
`(author=A, channel=Dm(B))`-reeks en B's `(author=B, channel=Dm(A))`-reeks — precies
dezelfde opzet als bij het algemene kanaal, alleen niet gegeneraliseerd naar alle peers.

`Channel::topic(id)` betekent: een subkanaal onder "Algemeen", met een willekeurige
`TopicId` (een `Uuid`, gegenereerd bij het aanmaken) in plaats van een peer. Anders dan een
DM is een subkanaal **niet** aan één auteur of geadresseerde gebonden — elke peer kan er in
posten, en iedereen ziet het, met dezelfde `seq`-per-(auteur, kanaal)-telling als bij een
DM of het algemene kanaal. `Channel::is_public()` (`tag == 0 || tag == 2`) is de sleutel die
overal bepaalt of iets zich als "algemeen" gedraagt (zichtbaarheid, doorsturen, hersync) —
alleen een DM is daarvan uitgezonderd. De titel van een subkanaal zit niet in `Channel`
zelf, maar in een gewone op (`OpKind::SetTopicTitle { id, title }`, altijd op
`Channel::GENERAL` geplaatst), last-writer-wins per `(lamport, author)`, precies zoals een
bijnaam (`OpKind::SetNick`) — dat dekt zowel het aanmaken (eerste keer gezien) als het
hernoemen (latere keer) zonder een apart "kanaal aangemaakt"-bericht.

Verwijderen gaat via `OpKind::DeleteTopic { id }`, dezelfde `(lamport, author)`-vergelijking
als `SetTopicTitle` op hetzelfde `id` — geen apart `target`/auteurscheck zoals bij een
bericht, want elke peer mag een subkanaal aanmaken/hernoemen en dus ook verwijderen. Wint
een latere `SetTopicTitle` alsnog van de `DeleteTopic`, dan komt het subkanaal terug — net
zo'n gewone laatste-schrijver-wint-uitkomst als bij een bijnaam.

### Protocolversie: 1 → 2, 2 → 3, 3 → 4, 4 → 5

**1 → 2**, bij de kanalen-uitbreiding (DM's): `VersionVector` en de bestandsoverdracht-
header (`crates/net/src/filestream.rs`) veranderden allebei van vorm om het kanaal mee te
dragen, op een manier die een oudere peer niet zomaar kan negeren — anders dan een nieuw
`OpKind` of een nieuw `#[serde(default)]`-veld op een bestaande struct. Een peer zonder
kanaalbegrip kan een per-(auteur, kanaal) version vector fundamenteel niet correct
interpreteren (hij zou algemene en DM-entries door elkaar halen), en de bestandsheader
kreeg 17 bytes extra die een oudere peer niet verwacht. Stilzwijgend laten mislukken (zoals
bij een onbekende `ControlMsg`-tag) zou hier een corrupt gedownload bestand of een chat die
nooit meer synchroniseert opleveren, zonder duidelijk signaal. Vandaar `PROTOCOL_VERSION`
van 1 naar 2: de handshake wijst een peer op de oude versie af met de bestaande, nette
`VersionMismatch`-status in plaats van dat er iets stilletjes fout gaat.

**2 → 3**, bij subkanalen (fase 9): `Channel` kreeg tag 2 erbij. Het *wire*-decoderen
daarvan is op zichzelf onschadelijk — `Channel` is een map, geen tuple, dus een oudere peer
die tag 2 niet kent decodeert de op gewoon met `topic` als `None` genegeerd. Het echte
probleem zat een laag dieper: `channel_to_blob` in `crates/store/src/lib.rs` en
`encode_channel` in `crates/net/src/filestream.rs` kenden vóór deze bump alleen tag 0 en 1,
en zouden een onbekende tag stilzwijgend op **dezelfde opslagsleutel als het algemene
kanaal** aliasen — een botsing op dezelfde primary key (`author`, kanaal-blob, `seq`) met
een échte algemene op van diezelfde auteur, met permanent dataverlies of een verkeerd
geadresseerde bestandsoverdracht tot gevolg. Precies het patroon dat de 1→2-bump ook al
moest voorkomen, hier alleen niet in de wire-vorm maar in de lokale opslag. Gevonden door de
`protocol-reviewer`-agent vóór het committen, niet door een test. Zie `crates/proto/src/lib.rs`.

**3 → 4**, bij automatische updates (fase 11): elke uni-stream voor een bulkoverdracht
(`crates/net/src/filestream.rs`) kreeg een 1-byte kind-prefix vóór de bestaande body
(`0` = bestand, gevolgd door de bestaande 41-byte `OpId`-header; `1` = update, zonder
verdere body). Dit verandert de bytes-op-de-draad van **bestaande, al geleverde**
bestandsoverdracht: een oudere peer zou dat eerste byte anders verkeerd als onderdeel van
zijn oude header lezen — precies de klasse fout die de eerdere bumps ook moesten
voorkomen. `Hello`/`HelloAck.app_version` en `UpdateRequest`/`UpdateResponse` zijn op
zichzelf onschadelijk (map-encoded, onbekend bericht wordt genegeerd), maar horen bij
dezelfde bump: een peer zonder update-begrip verstuurt toch nooit een `UpdateRequest`, dus
beide kanten van de functionaliteit vallen sowieso samen. Zie "Automatische updates"
hieronder. Geverifieerd door de `protocol-reviewer`-agent vóór het committen.

**4 → 5**, bij het pariteitsfragment voor video (zie "Media" hierboven): een oudere
ontvanger kent bit2 niet, stopt dat fragment als gewoon stuk in zijn samensteller en komt
dan op één stuk *te veel* uit — waarna `Reassembler::compleet()` nooit meer waar wordt en
er van die deler helemaal geen beeld meer verschijnt. Stilzwijgend kapot in plaats van een
nette `VersionMismatch`, precies de klasse fout die de eerdere bumps ook moesten
voorkomen.

### Waarom `seq` per (auteur, kanaal) telt, niet per auteur

Dit is geen keuze maar een noodzaak. Zou `seq` blijven lopen per auteur, over alle
kanalen heen, dan zou een peer die een DM tussen twee anderen nooit mag zien daar een
**permanent gat** op oplopen: hij mag dat seq-nummer nooit ontvangen, dus zijn
aaneengesloten reeks voor die auteur stopt voorgoed vlak vóór dat gat — óók voor latere
*algemene* berichten van diezelfde auteur, want die reeks kan het gat nooit meer dichten.
Door per (auteur, kanaal) te tellen bestaat dat gat voor een buitenstaander domweg niet:
hij houdt helemaal geen boekhouding bij voor een kanaal dat hem niet aangaat.

### Zichtbaarheid: `VersionVector::visible_to(viewer)`

Een (auteur, kanaal)-sleutel is zichtbaar voor `viewer` als `channel.is_public()` is
(het algemene kanaal of een subkanaal daaronder — beide voor iedereen), of
`channel == Dm(viewer)`, of `viewer == author` (zodat een peer die eigen data kwijtraakte
zijn eigen DM's terug kan krijgen van de ander in het gesprek). Dit filter wordt toegepast
vóórdat er iets van de version vector naar een specifieke peer gaat — zowel wat we zelf
claimen te hebben (`SyncRequest.have`) als wat we terugsturen (`SyncResponse.ops`) —
zodat de vector zelf al geen metadata lekt over met wie we nog meer DM's hebben.

### DM's krijgen geen doorstuurhulp via een derde peer

Er is bewust **geen encryptie** van de opinhoud — alleen QUIC-transport en het tailnet
als vertrouwensgrens, zie `TODO.md`. Zou een derde peer een DM tussen twee anderen ooit
doorsturen of bufferen (het mechanisme uit punt 3 hierboven, bedoeld voor gedeeltelijke
connectiviteit), dan zou die derde peer de inhoud gewoon kunnen lezen. Daarom synchroniseren
DM's uitsluitend rechtstreeks tussen de twee betrokkenen; het doorstuur- en
periodieke-hersync-mechanisme is expliciet beperkt tot wat publiek is
(`op.channel.is_public()`) — dat geldt voor het algemene kanaal én voor elk subkanaal
daaronder, precies zoals een subkanaal ook verder overal hetzelfde behandeld wordt als het
algemene kanaal.

**Trade-off, bewust met de gebruiker afgesproken:** in een normale full-mesh (alle drie
online) merk je hier niets van. Alleen als twee DM-partners elkaar niet rechtstreeks
kunnen bereiken — terwijl een derde peer wel een pad naar beide heeft — wacht de DM tot ze
alsnog rechtstreeks verbinden, in plaats van via die derde peer te lopen. Dat is exact
hetzelfde "offline is normaal"-gedrag als bij het algemene kanaal, alleen zonder het extra
vangnet dat daar wél bestaat.

### Bestanden in een DM

`FileMeta` draagt hetzelfde `channel`-veld als elke andere op — een bestand kan dus ook
privé aan één peer aangeboden worden. Twee gevolgen:

- De aanbieding zelf verspreidt zich (net als bij het algemene kanaal) via de gewone
  sync, maar dan beperkt tot de geadresseerde — dezelfde zichtbaarheidsregel als hierboven.
- **De aanbieder controleert bij een `FileRequest` of de aanvrager wel de geadresseerde
  is.** Onder normale omstandigheden komt een aanvraag van iemand anders hier nooit
  binnen — de sync laat de `FileMeta`-op immers al niet bij hem terechtkomen — maar dit is
  de plek waar het ook zonder dat vertrouwen wordt afgedwongen. Het antwoord bij een
  ongeautoriseerde aanvraag is bewust hetzelfde `FileOutcome::NOT_AVAILABLE` als "bestaat
  niet": een apart "geweigerd"-antwoord zou juist bevestigen dát het bestand bestaat.
- De bestandsoverdracht-header in `crates/net/src/filestream.rs` draagt sindsdien ook het
  kanaal (16-byte peer-uuid + 1-byte tag + 16-byte kanaal-peer-of-subkanaal-id + 8-byte
  seq, 41 bytes in plaats van 24): zonder dat zou een algemeen bestand, een DM-bestand en
  een bestand in een subkanaal van dezelfde auteur met toevallig dezelfde `seq` niet van
  elkaar te onderscheiden zijn. Om dezelfde reden draagt de tijdelijke `.part`-bestandsnaam
  op schijf (`crates/app/src/engine.rs`) het kanaal in zijn naam.

## Bestandsdeling

Twee lagen, met een harde knip ertussen: **aanbieden** is een gewone oplog-op en gaat
gratis mee via de sync hierboven; **downloaden** is punt-naar-punt met de aanbieder en
gaat over een eigen QUIC-stream, nooit over de control-stream.

### Aanbieden = een op

`OpKind::FileMeta { name, size, hash }` is qua sync niets bijzonders — hij synchroniseert
mee zoals elke andere op, ook naar een peer die pas veel later online komt. Er is dus
geen apart `FileOffer`-bericht: dat zou alleen een tweede, overbodig verspreidingspad
naast de oplog zijn.

De op draagt geen `offered_by`-veld. `op.author` is de aanbieder, precies zoals
`Edit`/`Delete` hun eigenaarschap ook via `op.author`/`target.author` regelen in plaats
van een los veld dat uit de pas zou kunnen lopen. `hash` is een 32-byte BLAKE3-digest.

Er is geen apart `OpKind` om een aanbod in te trekken — de bestaande, generieke
`OpKind::Delete { target }` doet dat al (fase 8): `target` is een kale `OpId` zonder
onderscheid tussen "welke soort op dit was", dus dezelfde regel als bij een bericht
(alleen de auteur van het doel, alleen binnen hetzelfde kanaal) geldt hier vanzelf. Zodra
de Delete-op zich verspreidt verdwijnt de kaart uit ieders timeline, én stopt de aanbieder
zelf met serveren: `crates/app/src/engine.rs` roept bij `UiCommand::Verwijder` ook
`Files::verwijder_aanbod` aan, die het pad uit `Files::aangeboden` haalt zodat een
volgend `FileRequest` netjes op `NOT_AVAILABLE` uitkomt — zonder die stap zou de kaart wel
verdwijnen maar het bestand voor wie de `OpId` al kende gewoon downloadbaar blijven.

Dit is geen volledige intrekking. Een download die al liep op het moment van verwijderen
loopt gewoon af (dezelfde afhandeling als wanneer het bronbestand handmatig van schijf
verdwijnt, zie hieronder), en een peer die de bytes al eerder volledig downloadde houdt
zijn eigen kopie — daar is niets aan te doen zonder een vertrouwensmodel dat verder gaat
dan "leest de anderen kunnen lezen".

### Downloaden = punt-naar-punt over een eigen stream

`OpId` van de `FileMeta`-op is meteen de identificatie van de overdracht; een apart
transfer-id zou alleen een tweede naam voor hetzelfde ding zijn.

```rust
FileRequest  { file: OpId, have_bytes: u64 }   // aanvrager → aanbieder
FileResponse { file: OpId, outcome: FileOutcome }  // aanbieder → aanvrager
```

`FileOutcome` is, net als `StreamKind`, een `u8` op de draad in plaats van een kale enum:
een toekomstige derde uitkomst mag de hele `FileResponse` niet laten mislukken bij een
peer die hem nog niet kent.

Er is bewust geen `FileChunkAck`. De bytes zelf gaan over een **betrouwbare, geordende**
QUIC-stream (`conn.open_uni()` naast de bestaande control-stream), dus er valt niets te
bevestigen — anders dan bij media over UDP is hier geen pakketverlies om tegen te
beschermen. Wat `FileChunkAck` bij een UDP-aanpak had moeten oplossen, lost QUIC's eigen
transport al op.

**Hervatten na onderbreking:** de aanvrager stuurt in `FileRequest.have_bytes` hoeveel
bytes hij al op schijf heeft van een eerdere, afgebroken poging. De aanbieder seekt zijn
bronbestand naar dat punt voordat hij begint te streamen. Er is geen verificatie per
chunk: de ontvanger hasht na afloop het **hele** bestand in één keer tegen `FileMeta.hash`
en verwijdert het bij een mismatch, waarna een volgende poging vanzelf weer bij 0 begint
omdat het deelbestand er niet meer is.

**De stream zelf begint met een vaste 41-byte header** (16-byte peer-uuid + 1-byte
kanaal-tag + 16-byte kanaal-peer + 8-byte seq, dus de `OpId` zelf inclusief kanaal — zie
"Kanalen" hierboven), handgeschreven zonder msgpack — zie `crates/net/src/filestream.rs`,
zelfde stijl als de media-header in `crates/net/src/media.rs`. Die header is nodig omdat
een peer meerdere bestanden tegelijk kan downloaden van dezelfde aanbieder: zonder iets
dat de stream aan een overdracht koppelt zou de ontvanger niet weten welk aankomend
uni-stream bij welke download hoort.

**Waarom een eigen stream en niet de control-stream:** de control-stream is één
betrouwbare, geordende byte-pipe die *alle* `ControlMsg`'s multiplext — chat-sync,
screenshare-signalering, noem maar op. Een bestand van een paar honderd megabyte daar
doorheen sturen zou al het andere verkeer op die stream laten wachten tot de laatste byte
binnen is. Dit is precies de reden dat `quinn`/QUIC gekozen is in plaats van één
TCP-socket (zie de techstack-tabel): meerdere onafhankelijke streams per verbinding, geen
head-of-line blocking tussen berichttypes.

### Content-adresseerbare afbeeldingen (fase 8)

Een `FileMeta` waarvan `name` er als afbeelding uitziet (`files::is_afbeelding`, een
extensie-check) landt niet in `download_dir` maar in een aparte, content-adresseerbare
map (`pictures_dir`, standaard `<datamap>/Pictures`): `<hex(hash)>.<extensie>`.

Dit lost een asymmetrie op die met een leesbare naam niet op te lossen was. Een gewoon
bestand krijgt zijn definitieve naam pas bij de aanvrager, ná verificatie, met `" (2)"`
etc. bij een naamsbotsing (zie boven) — dat pad ligt dus per downloadende peer
verschillend en pas ná afloop vast. Voor een afbeelding wil de UI diezelfde bytes echter
inline tonen bij **beide** kanten, en dat vraagt om een pad dat van tevoren al vaststaat.
`FileMeta.hash` is precies dat: hij ligt al vóór het downloaden vast en is bij aanbieder
en ontvanger identiek. Een losse randomizer zou dit niet oplossen — die zou bij elke
peer onafhankelijk een andere waarde opleveren; alleen een deterministische, uit de
inhoud afgeleide naam werkt hier.

- **Aanbieden:** `hash_en_bied_aan` (`engine.rs`) kopieert het bestand, ná het hashen,
  zelf naar `pictures_dir` — het origineel (ergens anders op schijf, van de gebruiker)
  blijft ongemoeid.
- **Downloaden:** `download_bytes` (`engine.rs`) hernoemt na een geslaagde
  hash-verificatie naar `pictures_dir` in plaats van naar `unieke_bestandsnaam` in
  `download_dir`.
- **Weergave:** de UI berekent hetzelfde pad zelf uit `FileView.hash`/`FileView.name` en
  probeert het te laden; bestaat het nog niet (niet gedownload, of de aanbieder is nog
  aan het hashen), dan faalt dat geruisloos en toont de kaart de generieke weergave met
  een downloadknop.

**"Verwijder alle afbeeldingen"** (instellingenscherm) is puur lokale schijfruimte
opschonen: het leegt `pictures_dir`, maar raakt geen enkele op aan. De kaarten blijven in
de tijdlijn staan; een download- of uploadpoging erna krijgt dezelfde nette afhandeling
als een bronbestand dat toevallig van schijf verdwijnt (zie hieronder). Dit is dus geen
alternatief voor `OpKind::Delete` — die twee doen iets anders en kunnen allebei apart.

### Waar de bytes op deze pc staan (2026-08-20)

`files::Files` houdt twee padkaarten bij, en die staan sinds 2026-08-20 ook op schijf in
`<datamap>/bestandspaden.json`:

- `aangeboden` — het bestand van de gebruiker zelf, per `OpId` dat wij aanboden. Dit is de
  lijst die `verzoek_ontvangen` uitlevert aan een andere peer.
- `gedownload` — waar een geverifieerde download beland is.

**Waarom dit niet uit de op te halen is:** een aanbod noemt een naam, geen pad, en dat is
precies goed — het pad van de aanbieder is niets waar een andere peer iets mee te maken
heeft. Aan de ontvangende kant is de naam ook niet genoeg: `unieke_bestandsnaam` maakt er
bij een botsing `naam (2).ext` van, en dat gebeurt juist wanneer twee peers hetzelfde
bestand aanbieden. De UI heeft dat pad nodig voor de openknop, en `verzoek_ontvangen` voor
het aanbod zelf — dus wordt het onthouden.

**Puur lokaal.** Dit is geen op, het synct niet en het staat nooit op de draad. Bij het
inlezen valt elk pad af dat niet meer bestaat: dan is er geen openknop (in plaats van een
knop die niets doet) en geeft een verzoek weer een eerlijke `NOT_AVAILABLE` (in plaats van
een upload die halverwege stukloopt).

**De twee kaarten blijven gescheiden.** Wat wij downloaden bij `aangeboden` gooien zou
betekenen dat we het ook zelf gaan aanbieden. Dat is herverspreiden — een functie die
niemand gevraagd heeft, en die verandert wie welke bytes kan opvragen.

### Wat hier niet zit

- **Geen chunk-niveau voortgang van de aanbieder naar de aanvrager.** De aanvrager kent
  zijn eigen voortgang al doordat hij de bytes zelf ontvangt; er is niets dat de
  aanbieder daarover apart zou moeten melden.
- **Geen kanaal- of quotabeperking.** `FileOutcome` heeft alleen `Ready` en
  `NotAvailable`. Een derde uitkomst (bijvoorbeeld "geweigerd") kan later als nieuwe
  waarde bij, en komt bij een oudere peer gewoon als "onbekend" binnen — zie boven.
- **Geen downloadlocatie-dialoog.** Bestanden landen in een vaste map (config
  `download_dir`, standaard `<datamap>/downloads`); zie `crates/app/src/config.rs`.
  Afbeeldingen zijn de uitzondering — zie hierboven.

## Wordle van de dag (2026-08-20)

Een spelletje bovenop dezelfde oplog. Volledige onderbouwing in `docs/OVERDRACHT.md`
beslissing 31; hier staat alleen hoe het zich tot de rest van dit document verhoudt.

**Wat er reist en wat niet.** Alleen de *uitslag* is een op (`WordleResult`, tag 30,
additief toegevoegd zonder protocolbump). Het raadsel zelf haalt elke peer op bij NYT — de
derde bewuste uitzondering op "nul servers", zie invariant 1 in `CLAUDE.md` — en de kaart in
de chat is helemaal geen op.

**Waarom de kaart geen op is.** `seq` is per (auteur, kanaal) en er is geen
inhoudsgebaseerde op-identiteit, dus drie peers die om 07:00 allemaal een "hier is het
raadsel van vandaag"-op plaatsen leveren drie kaarten op die de log niet tot één kan
samenvouwen. Dat is geen tekort van de log maar het antwoord op de verkeerde vraag: de
kaart draagt geen enkel feit dat een peer niet zelf kan uitrekenen. Hij wordt daarom in
`crates/app/src/ui/state.rs` in de tijdlijn *geplaatst* in plaats van gesynchroniseerd —
op de klok, vlak voor het eerste wat er die dag na 07:00 gezegd is, omdat hij geen
`lamport` heeft om op te sorteren. Alleen op `Channel::GENERAL`.

**De sleutel is `day`, de `print_date` van het raadsel als `YYYYMMDD`.** Niet het
raadselnummer van NYT (dan zou een dag die deze pc nooit ophaalde geen datum hebben) en
niet de lokale datum van het moment van spelen (dan boeken twee peers dezelfde avond op
verschillende dagen). Een Wordle-dag loopt van 07:00 tot 07:00 lokale tijd.

**Ordening: eerste-schrijver-wint per (auteur, dag).** Dit is de enige uitzondering op de
last-writer-wins-regel onder "Weergaveregels". Een uitslag is een gebeurtenis en geen
instelling; zou de laatste op winnen, dan kon je je score bijstellen nadat je die van de
anderen gezien had. De gelijkstand gaat op `(lamport, seq)` en niet op de sorteersleutel
`(lamport, author)`: binnen één auteur kan die laatste twee ops niet scheiden, en dan zou
de uitkomst van de aankomstvolgorde afhangen. Een `Edit` of `Delete` op een uitslag doet
niets, net als bij `SetNick`.

**Het scorebord is een pure functie over `Timeline::wordle`** (`crates/app/src/wordle.rs`,
`standen`). Omdat die vector al op `(dag, auteur)` gesorteerd is, liggen de dagen als
aaneengesloten stukken naast elkaar en is het één doorloop — dat is nodig, want het wordt bij
elke momentopname opnieuw gerekend. Een dag is één punt waard voor iedereen die hem in het
laagste aantal pogingen oploste, en telt alleen als er minstens twee peers speelden.

## Automatische updates

**Fase 13 (2026-08-07) verving het P2P-updatepad door een getekende release-feed.** Het
oude pad staat onderaan deze sectie beschreven, want het protocol draagt zijn berichten
nog en een oudere peer kan ze nog sturen.

**Waar een update vandaan komt.** `crates/app/src/release.rs` haalt elke zes uur (eerste
check een minuut na start) een JSON-manifest op bij één vaste HTTPS-URL:

```json
{ "version": "0.3.0", "url": "https://…/releases/download/v0.3.0/fitcom.exe",
  "size": 24117248, "hash": "<blake3 hex>", "signature": "<ed25519 hex>" }
```

`signature` staat over `"{version}\n{hash}\n{size}"` — alle drie de velden zitten erin,
zodat geen van hen los te verwisselen is met die uit een andere, ook geldig getekende
release. De publieke sleutel is in de binary gebakken (`PUBLIEKE_SLEUTEL_HEX`); staat daar
niets, dan wordt er nooit een update geaccepteerd. Falen gaat dicht.

**De volgorde is de beveiliging.** Manifest lezen → is de versie nieuwer dan
`env!("CARGO_PKG_VERSION")` (via `fitcom_proto::is_newer`, pure tuple-vergelijking van
`MAJOR.MINOR.PATCH`, geen `semver`-dependency) → vormcontroles (`version` alleen cijfers
en punten, want hij belandt in een bestandsnaam; `url` binnen de ingebakken repo; `size`
onder 200 MB) → handtekening → **pas dan** bytes ophalen → BLAKE3 tijdens het schrijven,
en weggooien als hij niet tegen de *ondertekende* hash klopt. Geen hervatten: zonder
Range-verzoek is het één rechte lijn, en een build opnieuw halen is goedkoper dan het
hervatpunt bewaken.

**Wat dit oplost.** In fase 11 leverde de peer met de nieuwere versie zowel de bytes als
de hash waartegen die bytes gecontroleerd werden — geen controle, en de wormstap uit
`docs/BEVEILIGING.md` (B-01). Nu bewijst TLS met wie we praten en bewijst de handtekening
wie de release gemaakt heeft; de privésleutel staat op geen van de drie machines en niet
bij de host. Een gekaapt hostaccount kan wel een bestand vervangen, geen handtekening
maken. `UpdateRequest`/`UpdateResponse` en een inkomende update-uni-stream worden gelogd
en geweigerd (ook B-21: onze eigen exe gaat nergens meer heen).

**Prijs.** Dit is de enige verbinding die de app buiten het tailnet legt, en dus een
bewuste uitzondering op invariant 1. Zie `docs/OVERDRACHT.md`, beslissing 23.

**Releases uitgeven.** `crates/app/src/bin/fitcom-release.rs` maakt het sleutelpaar
(`keygen`) en tekent een build (`sign`). De `.pk8` hoort buiten de repo; raakt hij kwijt,
dan werkt iedereen één keer met de hand bij.

**Structuur is ongewijzigd.** `crates/app/src/updates.rs` (`Updates`) blijft de pure
beslislaag met dezelfde scheiding als `files.rs` — één slot tegelijk — en `engine.rs`
voert uit. Alleen het veld `peer` is eruit verdwenen: er is nog maar één bron.

**Toepassen: een los updater-procesje, geen nieuwe crate.**
`crates/app/src/bin/fitcom-updater.rs` is een tweede binary in hetzelfde `fitcom`-package
(cargo pakt alles in `src/bin/*.rs` vanzelf op). Een exe kan zichzelf niet overschrijven
terwijl hij draait op Windows, dus: `engine.rs::pas_update_toe` spawnt de updater met het
pad van de geverifieerde download, het pad van de huidige exe en het eigen PID, en sluit
de app daarna af via dezelfde weg als het tray-menu (`EngineHandle::afsluiten_voor_update`,
gecontroleerd in `ui.rs::afsluiten_of_verbergen`). De updater wacht via
`OpenProcess(SYNCHRONIZE) + WaitForSingleObject` tot dat PID echt weg is, hernoemt dan de
nieuwe exe over de oude heen (`copy`+verwijderen als terugval bij een cross-volume-fout),
start hem opnieuw op, en stopt zelf.

### Het oude P2P-pad (fase 11, sinds fase 13 dicht)

Hier voor de volledigheid: de berichten bestaan nog in het protocol (varianten
verwijderen mag niet, zie de uitbreidingsregels bij "Control"), dus een peer op een oudere
build kan ze nog sturen. Ze worden gelogd en genegeerd.

Elke peer zag in de handshake (`Hello`/`HelloAck.app_version`) of een ander verder was. De
peer met de oudere versie stuurde dan `UpdateRequest`; de ander antwoordde met
`UpdateResponse { outcome, size, hash }` en duwde zijn eigen, draaiende exe over een
QUIC-uni-stream terug. Dat kind-byte (`0` = bestand, `1` = update, zie
`crates/net/src/filestream.rs::read_kind`) is nog steeds de reden voor de
`PROTOCOL_VERSION`-bump 3 → 4 hierboven, want bestandsdeling gebruikt hem nog.

`app_version` in de handshake blijft bestaan: de UI toont welke versie een peer draait.
Het triggert alleen niets meer.

## Verbindingsbeheer
- Bij start verbindt elke peer met alle geconfigureerde adressen.
- Falende verbindingen: exponentiële backoff, 1s → 30s cap, oneindig doorproberen.
- Liveness via QUIC keep-alive (5s) en idle timeout (15s); RTT komt uit `Connection::rtt()`.
  `Ping`/`Pong` bestaat in het protocol en wordt beantwoord, maar we sturen zelf niets —
  de transportlaag doet dit beter dan wij op applicatieniveau kunnen.
- Dialen krijgt een eigen deadline van 4s. QUIC loopt over UDP, dus een peer die uit staat
  weigert niets maar zwijgt; zonder deadline blijft de UI 15s op "verbinden…" staan.
- Peer-status in de UI: `Offline` / `Connecting` / `Online` / `VersionMismatch` / `IdentityChanged`.
- Eén of twee peers offline is een normale toestand, geen foutpad.

### Botsende verbindingen
Twee peers die tegelijk starten dialen elkaar tegelijk, dus er ontstaan twee verbindingen.
De winnaar is **absoluut** bepaald: de verbinding die is opgezet door de peer met het
laagste `PeerId`. De andere wordt aan beide kanten gesloten.

Vergelijk nooit "nieuwe versus bestaande verbinding" — dat leek te werken maar hangt af
van aankomstvolgorde. Bij ongelijke volgorde houdt de ene kant dan A→B over en de andere
B→A, sluiten ze elkaars keuze, en blijft er niets werkends over.

Tijdens die wissel kan een net verstuurd bericht verloren gaan. Dat is aanvaard: ops zijn
idempotent en worden bij elke (her)verbinding opnieuw gesynchroniseerd. Bouw hier geen
aparte hertransmissie voor — de oplog ís het herstelmechanisme.

### Inkomende verbindingen koppelen
Een inkomende verbinding wordt in deze volgorde aan een geconfigureerde peer gekoppeld:

1. Op `PeerId`, als we die al kennen (trust-on-first-use uit een eerdere sessie).
2. Op het volledige bronadres, **ip én poort**. Peers dialen vanaf dezelfde socket
   waarop ze luisteren, dus de bronpoort is hun `control_port`.
3. Op alleen het IP, maar uitsluitend als dat ondubbelzinnig één peer aanwijst. Dit vangt
   het geval af waarin een NAT onderweg de bronpoort herschrijft.

Matchen op alleen het IP is niet genoeg: draaien er meerdere peers achter hetzelfde adres —
drie instanties op één PC tijdens het testen, of peers achter dezelfde exit node — dan
wordt de verbinding aan de verkeerde peer toegewezen en slaat de identiteitscontrole
daarna ten onrechte alarm.

Lukt geen van de drie, dan wordt de verbinding **geparkeerd, niet geweigerd**: het adres
van een target kennen we pas nadat onze eigen dialer het heeft opgezocht, en de ander kan
sneller zijn. Pas na 5 seconden zonder match volgt afwijzing. Zonder dit worden bij het
opstarten willekeurig legitieme verbindingen geweigerd.

### Afsluiten
`MeshHandle` zet bij `drop` alle taken stil, maar keert terug voordat de UDP-poort vrij is.
Wie op dezelfde poort opnieuw wil starten — config herladen bijvoorbeeld — moet
`shutdown().await` gebruiken.

## Crate-layout
```
crates/
  proto/     ControlMsg, Op, media-header, version vector — geen I/O, puur en testbaar
  store/     rusqlite, oplog, timeline-opbouw, sync-berekening
  net/       quinn mesh, reconnect, UDP media-sockets, uni-streams voor bestandsbytes
  audio/     capture, opus, jitterbuffer, mix, ns/vad
  video/     WGC capture, MF encoder en decoder, kleuromzetting, D3D11 render-venster,
             deler- en kijker-thread
  app/       lib + binary: weergavelaag (`ui/`), config, chat-plumbing, streambeheer,
             bestandsdeling, tray, notificaties
```
`proto` en `store` hebben geen Windows- of hardware-afhankelijkheden en zijn daarom
volledig unit-testbaar. Daar zit de subtiele logica, dus daar zitten de tests.

**`crates/app/src/ui/` is de enige map met een UI-toolkit erin** (Tauri v2, plus
`crates/app/frontend/`).
De vijf crates erboven weten niet welke toolkit er gebruikt wordt en dat moet zo blijven —
zie "De grens tussen UI en de rest".
