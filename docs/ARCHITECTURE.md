# ARCHITECTURE

Hoe we bouwen. Requirements staan in `SPEC.md`.
**Lees dit voordat je iets aan het wire-protocol of de sync-logica wijzigt.**

## Techstack

| Laag | Keuze | Waarom |
|---|---|---|
| Taal | Rust 1.86, MSVC | Toolchain staat al; geen .NET SDK aanwezig; één statische exe; geen GC-pauzes in de audiopijplijn |
| UI | `eframe` / `egui` | Alles in één proces, snel te bouwen, laag idle-verbruik |
| Async | `tokio` | Standaard, past bij quinn |
| Control/chat | `quinn` (QUIC) | Betrouwbaar, meerdere onafhankelijke streams over één verbinding, geen head-of-line blocking tussen berichttypes, ruimte voor bulk file transfer later |
| Media | `tokio::net::UdpSocket` | Retransmits zijn schadelijk voor realtime audio/video |
| Wire-formaat | `serde` + `rmp-serde` (MessagePack) voor control; hand-rolled binaire header voor media | Compact, tolerant voor toegevoegde velden, dumpbaar als JSON voor debugging |
| Opslag | `rusqlite` (bundled) | Geen externe dependency, één bestand |
| Schermcapture | `windows` crate → Windows.Graphics.Capture + D3D11 | Microsofts eigen binding; monitor én venster; blijft op de GPU |
| Video encode/decode | Media Foundation Transform via `windows` crate, achter een trait | Geen NVIDIA SDK of CUDA nodig; vendor-agnostisch; D3D11-textuur als input |
| Audio I/O | `cpal` (WASAPI); losse WASAPI-loopback via `windows` crate voor desktop-audio | |
| Audio codec | Opus (`audiopus`) | |
| Noise suppression | `nnnoiseless` | Pure Rust, geen C++ build-dependency |

### Encoder achter een trait
De video-encoder zit achter `trait VideoEncoder` met een D3D11-textuur als input.
We starten met Media Foundation omdat dat nul extra dependencies kost. Als de gemeten
glass-to-glass latency tegenvalt, wisselen we naar **directe NVENC** (via
`nvidia-video-codec-sdk` of eigen FFI naar `nvEncodeAPI.dll` met
`NV_ENC_DEVICE_TYPE_DIRECTX`) zonder de rest te raken. Meetpunt zit in fase 4.

### Waarom video in een apart venster
`eframe` rendert via wgpu (DX12). Een gedecodeerde D3D11-textuur daarin krijgen vereist
shared-handle-interop op `wgpu-hal`-niveau — onveilig, fragiel, en niet nodig.
In plaats daarvan: elke stream krijgt een eigen borderless Win32-venster met eigen
D3D11 swapchain, op een eigen thread met eigen message pump. Volledig geïsoleerd van
de UI-thread, optimale videopad, en een maximaliseerbaar venster is op één 1080p-monitor
sowieso de betere UX. Grid-in-hoofdvenster is fase 5.

## Processtructuur

```
main thread            eframe/egui  — pure weergave van een momentopname
tokio runtime          motor (oplog + mesh), quinn control-mesh, UDP media-sockets
capture thread(s)      WGC → D3D11 texture → encoder → UDP  (één per gedeelde bron)
render thread(s)       UDP → decoder → D3D11 swapchain      (één per bekeken stream)
audio capture thread   WASAPI → nnnoiseless → VAD → Opus → UDP
audio render thread    UDP → jitterbuffer → Opus decode → mix → WASAPI
```

Alle threads praten met de UI via kanalen. Geen gedeelde locks op het hot path.

### De UI mag stilvallen
egui tekent geen frames zolang het venster verborgen of geminimaliseerd is. Alles wat
door moet lopen terwijl je niet kijkt — synchronisatie, herverbinden, meldingen — hoort
daarom níét in `update()`. De motor (`app/src/engine.rs`) bezit de mesh en de oplog en
draait op de tokio-runtime; de UI leest een `watch`-momentopname en stuurt commando's
terug. Om dezelfde reden leest de tray-thread zijn eigen gebeurtenissen: zou de
tray-klik in de UI worden afgehandeld, dan kon je een verborgen venster nooit meer
terughalen.

## Wire-protocol

### Control (QUIC, betrouwbaar)
Length-prefixed MessagePack. Elke peer opent één QUIC-verbinding naar elke andere peer;
bij een botsing wint de verbinding waarvan de initiator de laagste UUID heeft.

```rust
enum ControlMsg {
    Hello { protocol_version: u32, peer_id: Uuid, display_name: String },
    HelloAck { protocol_version: u32, peer_id: Uuid, display_name: String },
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

    // GERESERVEERD — nog niet implementeren, zie TODO.md
    // FileOffer { .. }, FileAccept { .. }, FileChunkAck { .. },
}
```

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
13      1     flags       u8   bit0 = keyframe, bit1 = laatste fragment van frame
14      2     frag_index  u16
```

Videoframes worden gefragmenteerd op ~1200 byte payload (onder Tailscale's MTU).
Een frame is compleet als alle fragmenten 0..n binnen zijn met bit1 gezet op de laatste.
Incomplete frames worden gedropt; bij verlies van een keyframe vraagt de ontvanger via
control om een nieuwe IDR.

## Chat-synchronisatie

Op-based CRDT met per-auteur dichte sequentienummers. Geen vector clocks nodig voor
convergentie — de version vector volstaat omdat alleen de auteur zelf zijn eigen
ops nummert, dus er zitten nooit gaten in.

```rust
struct Op {
    author: Uuid,        // wie de op maakte
    seq: u64,            // per auteur monotoon, 1..N, geen gaten
    lamport: u64,        // voor totale ordening tussen auteurs
    wall_clock: i64,     // alleen voor weergave, nooit voor correctheid
    kind: OpKind,
}

enum OpKind {
    Post   { body: String },
    Edit   { target: OpId, body: String },
    Delete { target: OpId },
    SetNick{ name: String },
    // later: React, Reply, FileMeta — nieuwe varianten, geen migratie
}
```

- `OpId = (author, seq)` is globaal uniek. Opslag is idempotent: dezelfde op tweemaal
  toepassen is een no-op. Dat is de hele conflictafhandeling.
- **Weergavevolgorde** is `(lamport, author)`. Lamport wordt bij elke ontvangen op
  bijgewerkt naar `max(local, remote) + 1`.
- `Edit`/`Delete` zijn last-writer-wins op `target`, gewonnen door de hoogste
  `(lamport, author)`. Renderen vouwt ze over de `Post` heen.
- **Version vector** = `{author → hoogste *aaneengesloten* seq}`, niet `max_seq`.

  Ops worden dicht genummerd, maar komen niet per se in die volgorde binnen: we kunnen
  bij B de ops 6-10 van auteur A ophalen terwijl A zelf al 11 broadcast. Landt die 11
  eerder, dan hebben we 1-5 en 11, met een gat. Melden we dan `max = 11`, dan zeggen we
  "ik heb alles t/m 11" en krijgen we 6-10 nooit meer.

  De op met een gat ervoor wordt wél bewaard, maar telt pas mee zodra het gat gedicht is.
  Om dezelfde reden versturen we nooit ops voorbij onze eigen aaneengesloten reeks —
  anders erft de ontvanger ons gat zonder het te weten.

  Sync bij (her)verbinding:
  1. Beide kanten sturen `SyncRequest { have }`.
  2. Elke kant stuurt terug wat de ander mist: alle ops waarvan
     `seq > peer.have[author]`, voor elke auteur.
  3. Klaar. Convergentie in één ronde, ook na maanden offline.
### Drie wegen waarlangs een op zich verspreidt
1. **Broadcast** bij het plaatsen — het normale geval waarin iedereen online is.
2. **Inhaalslag bij (her)verbinding** — dekt de peer die weg was.
3. **Doorsturen plus periodieke hersync** — dekt gedeeltelijke connectiviteit: A en C
   kunnen elkaar niet bereiken, B beiden wel. Ontvangen ops die nieuw voor ons waren
   sturen we door; ops die we al kenden niet, en dáármee stopt de lus vanzelf. Daarnaast
   sturen we elke 30s ongevraagd onze version vector rond. Dat kost enkele tientallen
   bytes en herstelt elke toestand die 1 en 2 gemist zouden hebben.

Een op is nooit verloren zolang één peer hem heeft.

### Weergaveregels
- Sorteren op `(lamport, author)`. `wall_clock` mag nooit meedoen: de klokken van de
  drie PC's lopen uiteen en dan zou de volgorde per peer verschillen.
- `Edit`/`Delete` tellen alleen als `op.author == target.author`. Zonder die regel kan
  iedereen andermans tekst herschrijven, en in een append-only log is dat niet terug
  te draaien.
- Per bericht wint de `Edit`/`Delete` met de hoogste `(lamport, author)`.

### SQLite-schema
```sql
CREATE TABLE ops (
  author     BLOB    NOT NULL,     -- 16-byte uuid
  seq        INTEGER NOT NULL,
  lamport    INTEGER NOT NULL,
  wall_clock INTEGER NOT NULL,
  kind       INTEGER NOT NULL,
  payload    BLOB    NOT NULL,     -- MessagePack van de kind-specifieke velden
  PRIMARY KEY (author, seq)
) WITHOUT ROWID;
CREATE INDEX ops_order ON ops(lamport, author);

CREATE TABLE peers (peer_id BLOB PRIMARY KEY, display_name TEXT, address TEXT, last_seen INTEGER);
CREATE TABLE meta  (key TEXT PRIMARY KEY, value BLOB);  -- eigen uuid, lamport, schema-versie
```

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
  net/       quinn mesh, reconnect, UDP media-sockets
  audio/     capture, opus, jitterbuffer, mix, ns/vad
  video/     WGC capture, encoder-trait, MF impl, decoder, D3D11 render-venster
  app/       lib + binary: eframe UI, config, chat-plumbing, tray, notificaties
```
`proto` en `store` hebben geen Windows- of hardware-afhankelijkheden en zijn daarom
volledig unit-testbaar. Daar zit de subtiele logica, dus daar zitten de tests.
