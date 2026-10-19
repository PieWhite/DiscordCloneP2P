# Verbinden zonder Tailscale — bouwplan

Opgesteld 2026-10-18 met Rick. In `ROADMAP.md` is dit fase 16.

**Status 2026-10-19:** fase A is gebouwd (A1 t/m A6) en fase B ook (B1 t/m B6), allebei nog
niet uitgegeven en nog niet tussen twee machines getest. Wat er bij het bouwen anders uitviel
dan hieronder staat, staat in `docs/OVERDRACHT.md` beslissing 46 (A) en 47 (B); de open punten
met echte machines in `docs/TESTPLAN.md`, "Protocol 6" en "Fase B". **B7 (IPv6) is niet
gebouwd**: hij wacht op de A0-regels van de vrienden. Fase C: niets gebouwd. A en B kunnen
apart of samen uit: een peer met alleen fase A werkt met een peer met B (beslissing 47).

## Waarom

De vrienden installeren geen extra app om FitCom te laten werken, en ze gaan geen poorten
openzetten in hun router. Wat ze wél willen: één keer iets kopiëren en plakken. Het doel is
dus dat de app zonder Tailscale verbindt, op publieke adressen. Geen server erbij, en
dezelfde kwaliteit als nu.

Tailscale doet nu **twee** dingen, en dit plan moet ze allebei vervangen:

1. **Bereikbaarheid.** Peers bellen elkaar op een tailnet-adres; Tailscale prikt het gat
   door de NAT.
2. **De hele beveiligingsgrens.** `crates/net/src/tls.rs` accepteert elk certificaat ("het
   verkeer loopt al door WireGuard"). Een `PeerId` is een onbewezen claim (B-05) en het
   mediapad heeft geen authenticatie en geen versleuteling (B-18). Op een publiek adres
   staat dat open voor iedereen op internet. **Daarom komt fase A vóór fase B, en is
   fase A niet optioneel.**

Wat dit plan **niet** doet: geen STUN-server, geen DHT, geen relay, geen
signaling-server. De enige partijen waar de app mee praat zijn de eigen router en de
andere peers. Invariant 1 wordt er eenvoudiger van, want Tailscale verdwijnt als externe
afhankelijkheid. De tekst van invariant 1 in `CLAUDE.md`, `PRODUCT.md` en `docs/SPEC.md`
pas je na fase B aan, in overleg.

## Wat er al bekend is

**Ricks pc, `tailscale netcheck`, 2026-10-18:**
- publiek IPv4, dus geen CGNAT
- `MappingVariesByDestIP: false`: de makkelijke soort NAT
- router doet NAT-PMP en PCP
- geen IPv6

Deze pc is dus de makkelijke kant: de app kan zijn poorten zelf openzetten. Van de twee
andere pc's is nog niets bekend; stap A0 meet dat zonder dat zij iets hoeven te doen.

**Hoe anderen het doen (onderzoek 2026-10-18):**
- **emufii** is geen voorbeeld van servervrij. Elke client heeft een WireGuard-tunnel naar
  een relay die de maker betaalt; de sessies lopen via `coord.emufii.xyz`. Hamachi-model.
- **Keet, Tox, Jami, Syncthing, iroh en libp2p** leunen allemaal op publieke
  infrastructuur, zowel om elkaar te vinden (bootstrapnodes, DHT, discoveryservers) als
  voor een relay als terugval.
- **Een gratis relay die 12 Mbit/s schermdelen draagt bestaat niet.** De relays van
  Syncthing zijn TCP op vrijwilligershardware. libp2p kapt een circuit af op 128 KiB of
  2 min. De publieke relays van iroh zijn "voor ontwikkeling".
- **Slagingskans van een direct pad:**
  - Tailscale: ruim 90% (eigen schatting)
  - libp2p DCUtR: 70% ±7 over 4,4 miljoen pogingen (IMC '26)
  - twee "harde" NAT's tegenover elkaar, zonder relay: praktisch 0

Voor drie vaste vrienden met 1 Gbit/s volstaat de router plus elkaar. De derde peer is
dan de spiegel die zegt "zo zie ik jou".

## Beslissingen

1. **Identiteit = een Ed25519-sleutel per installatie; de `PeerId` blijft de UUID.**
   - De sleutel komt naast `peer_id` in `identity.toml`. Gebruik `ring::signature`, al in
     gebruik in `release.rs`.
   - *Afwijking van BEVEILIGING blok 4 punt 16* (`PeerId = blake3(pubkey)`): elke op in de
     store draagt een UUID-auteur. Een nieuwe `PeerId` maakt de hele geschiedenis wees.
   - De koppeling `PeerId ↔ sleutel` wordt vastgepind in de config, net als `known_id` nu.

2. **TLS blijft anoniem; beide kanten tekenen de exporter.**
   - Elke kant tekent `Connection::export_keying_material` (label `fitcom/hello/v6`,
     context = rol) en stuurt sleutel plus handtekening mee in `Hello`/`HelloAck`.
   - Een man-in-the-middle heeft twee TLS-sessies met twee verschillende exporters, dus
     zijn doorgestuurde handtekening klopt nooit.
   - Kleiner en testbaarder dan raw public keys in rustls.
   - `HelloAck` gaat pas de deur uit **na** de controle (B-05a). Een onbekende sleutel
     krijgt een stille close, met hooguit één logregel per bronadres per minuut.

3. **Trust-on-first-use alleen op een tailnet-adres, nooit op een publiek adres.**
   - Tailnet-adressen zijn `100.64.0.0/10` en `fd7a:115c:a1e0::/48`. Zo pinnen de huidige
     installaties na de update elkaars sleutel vanzelf, zonder dat iemand iets doet.
   - Een sleutel voor een publiek adres komt alleen uit een uitnodigingscode.
   - De tekst "Identities are learned on first contact" in
     `ui/view/settings/network.rs` gaat dan weg.

4. **Media: eigen AEAD op de bestaande UDP-socket, niet naar QUIC-datagrams.**
   - QUIC-datagrams vallen onder quinn's congestiecontrole en pacing, en dat smoort precies
     de stoot van 200+ fragmenten van een keyframe.
   - Zonder WireGuard is versleuteling ook nodig, niet alleen een MAC; daarin wijkt dit af
     van de oplossing bij B-18, die alleen een MAC vroeg.
   - Pakket = `[4 B sessie-index][8 B teller][versleutelde header+payload][16 B tag]`,
     28 bytes overhead, met AES-256-GCM via `ring::aead`.
   - Sleutels per richting:
     `export_keying_material(label "fitcom/media/v6", context = PeerId afzender)`.
   - **Sessie-index (zoals WireGuard):** de ontvanger zoekt de sleutel op zonder te
     proberen. Rommel op een publieke poort kost dus een tabelopzoeking en geen
     cryptografie (invariant 4).
   - **Teller:** één `AtomicU64` per sleutel, geen lock (werkafspraak). Replayvenster van
     2048 over die teller.
   - `MAX_MEDIA_PAYLOAD` blijft 1100: 1100 + 16 + 28 + 48 (IPv6 40 + UDP 8) = 1192, onder
     1280.
     De ruimte die WireGuard innam is nu vrij.
   - De sleuteltabel is een `arc-swap` die de motor bijwerkt bij verbinden en verbreken;
     mediathreads lezen hem lock-vrij.

5. **Uitnodigingscode in plaats van een kaal IP.**
   - Inhoud:
     - versie
     - `PeerId` (16 B)
     - publieke sleutel (32 B)
     - weergavenaam
     - de adressen die deze pc van zichzelf kent
     - een checksum van 4 B (`blake3`)
   - Gecodeerd als `fitcom1:` plus base32 (`data-encoding`, staat al in de lockfile); zo'n
     120 tekens.
   - Niet geheim: een code geeft geen toegang. Je moet elkaars code hebben, en wie jouw
     code heeft maar jij de zijne niet, komt niet binnen.
   - `ipconfig` levert het verkeerde adres (`192.168.x.x`). De app weet zijn publieke adres
     van de router (stap B2) of van een vriend (stap B4).

6. **Port mapping met `portmapper` van n0** (0.19.3; UPnP, PCP en NAT-PMP).
   - Het is een port van de portmapper van Tailscale. Die vond op Ricks router NAT-PMP en
     PCP.
   - Hij doet zelf gateway zoeken, proberen, lease vernieuwen en extern adres volgen.
     `igd-next` + `crab_nat` laat precies dat als eigen werk liggen.
   - `default-features = false` (geen metrics).
   - Eén client per poort, dus twee: `control_port` en `media_port`.
   - Draait op de tokio-runtime van `net`, nooit op een mediathread.
   - Bij netjes afsluiten `deactivate()`, zodat de router opgeruimd achterblijft.
   - Te bewaken: wat `netwatch` op de achtergrond doet. Stap B2 meet het idle-gedrag.

7. **Eén mediasocket per proces, op `media_port`, voor alle media (zoals BUNDLE in
   WebRTC).**
   - Nu bindt de deler `bind(0)` (`video/src/deler.rs:235`) en de kijker ook
     (`video/src/kijker.rs:148`). Op een tailnet kan dat; achter een NAT is elke socket
     een eigen mapping die iemand moet doorprikken, en de kijker weet niet eens naar welke
     poort.
   - Met één socket is er per peerpaar één mapping, één keepalive en één port mapping.
   - Een ontvangthread in `net` ontsleutelt en verdeelt via `crossbeam-channel` naar
     audio en kijkers.
   - **Altijd antwoorden naar het laatste geauthenticeerde bronadres** (roaming, zoals
     WireGuard), niet naar "IP van de QUIC-verbinding plus aangekondigde poort"
     (`app/src/engine.rs:842`). Dat dekt ook een NAT die de poort herschrijft.

8. **Adressen zijn hints, nooit toegang.**
   - Peers vertellen elkaar hun eigen adressen, hoe ze de ander zien, en hoe ze de anderen
     zien ("peer als STUN").
   - Een vals adres kost hooguit een mislukte belpoging; binnenkomen gaat alleen met de
     gepinde sleutel.

9. **Tailscale blijft een kandidaat.**
   - Het bestaande `address` uit de config is gewoon één van de adressen die geprobeerd
     worden. Wie Tailscale houdt merkt niets en niets breekt tussen de releases.

## Fase A — beveiliging (release 1, protocol 5 → 6)

Hierna kan een peer zich niet meer voordoen als een ander, en is media versleuteld en
geauthenticeerd. **Iedereen moet tegelijk bijwerken**; dat is de prijs van de bump
(invariant 5). Er verandert nog niets aan hoe peers elkaar vinden: het blijft Tailscale.

**A-vooraf.** Controleer in de ledenlijst dat beide vrienden echt op 1.10.1 zitten. Volgens
de notitie van 2026-08-20 faalde hun auto-update stil zolang `fitcom-updater.exe` niet met
de hand vervangen was. Blijft één van hen achter op protocol 5, dan valt die na release 1
uit de mesh.

**A0 — Bereikbaarheid meten, alleen lezen.**
- Bij het starten één `portmapper`-probe plus een tijdelijke mapping. Uit de reactie lees
  je het externe adres af; daarna gaat de mapping meteen weer weg.
- Plus de lijst met globale IPv6-adressen van de interfaces.
- Uitkomst:
  - in de log
  - één regel in Settings → Network: "Router: PCP ✓ · NAT-PMP ✓ · UPnP ✗ · public IPv4 ·
    no IPv6", of "CGNAT", als het externe adres in `100.64.0.0/10` of een privéreeks valt.
    *(Gebouwd met "yes"/"no" in plaats van ✓/✗: de ingebedde fonts hebben die tekens niet.)*
- De vrienden sturen een screenshot; zij hoeven verder niets te doen. Daarmee is bekend of
  fase C nodig is vóórdat B gebouwd wordt.

**A1 — Sleutel.**
- `Identity` (`app/src/config.rs:507`) krijgt een Ed25519-sleutel (PKCS#8). Een bestaande
  `identity.toml` zonder sleutel krijgt er een bij; de `peer_id` blijft.
- `MeshConfig` krijgt het sleutelpaar, `PeerTarget` een `Option` met de gepinde publieke
  sleutel.
- `PeerConfig` (`config.rs:421`) krijgt `public_key`, met `#[serde(default)]`.

**A2 — Handshake.**
- `Hello` en `HelloAck` (`proto/src/control.rs:375`) krijgen er `public_key` en `signature`
  bij, achteraan en met `#[serde(default)]`.
- `PROTOCOL_VERSION` (`proto/src/lib.rs:62`) wordt 6.
- Aanpassingen in `net/src/mesh.rs`:
  - `try_dial` (`:929`) controleert na de `HelloAck`.
  - `accept_one` (`:1079`) controleert vóór de `HelloAck`.
  - `match_inbound` (`:712`) matcht alleen op een geverifieerde sleutel (B-05b).
  - Wie een sleutel gebruikt die bij een ander target hoort, wordt geweigerd.
- **Een peer op protocol 5 moet "versie 5 vs 6" zien, niet "offline".** De feedcheck loopt
  eens per 6 uur, dus tijdens het bijwerken belt een vriend op 5 nog een peer op 6. Zijn
  `Hello` heeft geen sleutel. Sluit de kant op 6 dan stil, dan ziet hij "offline · peer
  reageert niet" en weet niemand dat er bijgewerkt moet worden. Daarom:
  - Een `Hello` met een oudere `protocol_version` krijgt een `HelloAck` met alléén de
    versie: nil-`PeerId`, lege naam, poort 0, geen sleutel. Bij hem vuurt dan het bestaande
    `VersionMismatch`-pad.
  - Alleen een `Hello` op 6 met een foute handtekening of een onbekende sleutel krijgt de
    stille close.
  - Andersom (6 belt 5) werkte níét via de bestaande controle: een peer op 5 sluit in dezelfde
    tik als zijn `HelloAck`, die dus nooit aankomt. Wel zijn sluitreden "protocolversie", en
    die telt nu ook als versieverschil (zie beslissing 46).
- `tls.rs`: de moduledoc herschrijven. De verifier blijft "accepteer alles", maar de reden
  is nu de getekende exporter, niet WireGuard.

**A3 — De rand van internet.**
- In de accept-lus: `Incoming::retry()` voor adressen die nog niet gevalideerd zijn, zodat
  een gespoofde afzender geen versterking krijgt.
- Een maximum aan pogingen per bron-IP per seconde.
- Logregels voor geweigerde pogingen begrensd zoals B-25.
- De limieten van B-17 en B-26 blijven staan.

**A4 — Media-AEAD.**
- `net/src/media.rs`:
  - `stuur` (`:166`) krijgt een ontvangerhandvat met adres, sleutel, index en teller in
    plaats van een `SocketAddr`.
  - `ontvang` (`:187`) geeft de afzender-`PeerId` terug, of niets.
- Consumenten:
  - `audio/src/session.rs` (verzenden en ontvangen)
  - `video/src/deler.rs` en `kijker.rs`
  - Bij de kijker vervalt de bronpoort-heuristiek van B-28 (`kijker.rs:269`): "ontsleuteld
    met de sleutel van de deler" is de controle.
- De sleuteltabel wordt gevuld bij `PeerConnected` en geleegd bij verbreken. Pakketten in de
  vlucht rond een herverbinding mogen wegvallen: houd de vorige sleutel 5 s aan.

**A5 — Tests** (alleen waar ze fouten vangen die handmatig testen niet vindt):
- `net`:
  - handshake met verkeerde sleutel
  - een `Hello` doorgestuurd naar een andere verbinding (exporter klopt niet)
  - een `HelloAck` mag niet vertrekken vóór de controle
- `net`/`proto`:
  - een pakket met een vervalste tag
  - een herhaald pakket
  - een pakket buiten het venster
  - een onbekende index (geen crypto-aanroep)
- Bestaande loopbacktests (`tests/burst.rs`): het verlies bij keyframe-stoten mag niet
  omhooggaan.

**A6 — Documenten.**
- `docs/BEVEILIGING.md`: B-05 en B-18 opgelost, met datum.
- `docs/OVERDRACHT.md`: beslissing 46, met de afwijkingen hierboven.
- `docs/TESTPLAN.md`: mac↔Windows-handshake op echte machines.

**Klaar als:**
- alle drie de peers op protocol 6 met elkaar verbinden over het tailnet, zonder dat iemand
  iets heeft hoeven doen (TOFU op tailnet);
- een peer met een andere sleutel voor dezelfde `PeerId` geweigerd wordt;
- voice en schermdelen op 12 Mbit/s geen meetbaar extra verlies of extra vertraging hebben
  ten opzichte van 1.10.1 (vergelijk met de bestaande `spoor`-sporen);
- er van beide vrienden een A0-screenshot is.

## Fase B — verbinden op een publiek adres (release 2, additief)

Geen protocolbump: nieuwe berichten achteraan, de mediavorm ligt al vast sinds A. Na deze
fase verbinden twee peers met elkaars code, zonder Tailscale.

**B1 — Uitnodigingscodes.**
- Codering en decodering in `proto` (puur, testbaar): `fitcom1:`-tokens uit een geplakte
  tekst vissen, zodat een heel WhatsApp-bericht plakken ook werkt. Checksum en versie
  controleren.
- `UiCommand::CopyInviteCode` en `UiCommand::AddPeerCodes(String)`.
- De motor matcht op `PeerId`:
  - een bestaand target krijgt sleutel en adressen;
  - een nieuw target wordt aangemaakt;
  - een code waarvan de sleutel afwijkt van de gepinde sleutel wordt geweigerd, met een
    duidelijke melding.
- Daarna `config.toml` wegschrijven.
- `MeshCommand` krijgt "target toevoegen" en "kandidaten bijwerken". Nu liggen de targets
  vast bij `spawn` (`mesh.rs:275`).
- Settings → Network:
  - "Your invite code" + Copy
  - "Add peers" (plakveld, één of meer codes)

**B2 — Port mapping.**
- Twee `portmapper::Client`s in `net` (`control_port`, `media_port`), aan zolang de app
  draait.
- `watch_external_address` voedt de eigen kandidaten en dus ook de eigen code.
- Config `network.port_mapping`, standaard aan.
- Meten: idle-CPU en wakeups met de mapping aan versus uit. Moet onmerkbaar zijn
  (invariant 4).

**B3 — Kandidaten en bellen.**
- `PeerTarget.address` wordt een lijst kandidaten:
  - het tailnet-adres uit de config
  - de adressen uit de code
  - geleerde adressen (B4)
- `dial_forever` (`mesh.rs:870`) probeert ze gestaffeld (250 ms ertussen) vanaf hetzelfde
  endpoint. De eerste geverifieerde handshake wint; de botsingsregel blijft zoals hij is.
- `BACKOFF_MAX` gaat van 30 naar 10 s zolang er een publieke kandidaat is. Beide kanten
  bellen elkaar dan vaak genoeg terwijl de NAT-mapping van de vorige poging nog leeft.
  Op een makkelijke NAT is dat het hele doorprikken. De kosten zijn een paar
  QUIC-Initials per 10 s.
- Let op het geval waarin beide kanten elkaar via meerdere kandidaten tegelijk bereiken:
  dan ontstaan er meerdere verbindingen tegelijk. De botsingsregel lost dat al op, maar test
  het met twee kandidaten die allebei werken.

**B4 — Adressen uitwisselen.**
- Nieuw vluchtig bericht `ControlMsg::Addresses` (tag 51), single-hop, nooit een op. Bij
  verbinden en bij een wijziging. Inhoud:
  - eigen kandidaten
  - `you_are`: het bronadres van deze verbinding zoals wij het zien
  - `media_you_are`
  - per andere verbonden peer: de adressen waarop wij die zien
- Lijsten begrensd (B-17-stijl).
- Bewaren in `<data>/adressen.json`: per peer de laatste paar adressen met tijdstempel. Zo
  vindt een herstart elkaar terug zonder nieuwe code, en zolang er één peer bereikbaar
  blijft, heelt een IP-wissel vanzelf.
- Dit is ook het doorprikken voor twee peers die geen van beiden port mapping hebben: de
  derde vertelt beiden hoe hij de ander ziet, en beide gaan bellen (B3). Dat werkt voor
  elke NAT met endpoint-onafhankelijke mapping, ook voor CGNAT. Geen host-peer: elke
  verbonden peer doet dit, N-agnostisch.

**B5 — Eén mediasocket, roaming.**
- De motor opent de mediasocket bij het starten, niet meer bij deelnemen aan het gesprek
  (`audio/src/session.rs:441`).
- Een vaste ontvangthread in `net` ontsleutelt, leert het bronadres per peer en verdeelt:
  - voice, bureaubladgeluid en muziek naar de audiosessie (crossbeam);
  - video per `stream_id` naar de kijker die erop intekende (crossbeam; de lus van
    `kijker.rs` wacht op het kanaal met dezelfde timeouts);
  - keepalives naar de adrestabel.
- De deler verstuurt vanaf een kloon van deze socket in plaats van `bind(0)`.
- `StreamSubscribe.media_port` wordt de hoofdpoort (blijft als hint voor 1.x-peers).
- Een keepalive van 16 B, elke 15 s, naar elke verbonden peer. Zo staat het mediagat al
  open voordat iemand praat of deelt.
- Ontvangbuffer naar 2 MB: twee delers tegelijk moeten er allebei een keyframe in kwijt
  kunnen.
- **De vensters horen bij deze ene socket en worden niet opnieuw opgebouwd** bij opnieuw
  deelnemen aan het gesprek of opnieuw intekenen. Zolang er per stream een eigen socket is,
  vangt een hoogste-tellermerk per sleutel een herhaling naar een nieuwe socket maar ten dele
  op (de laatste ~2048 pakketten); met één vaste socket is dat gat dicht (beslissing 46).
- **Dit is de riskantste stap**, want hij raakt het hete pad van de kijker, en daar zit de
  geschiedenis van de hapering. Meet vóór en na met `spoor` (aankomst → getoond, verlies)
  en `tests/burst.rs`. Een slechter resultaat is een blokkade, geen afweging.

**B6 — Settings, documenten, firewall.**
- Settings → Network, per peer het pad:
  - "direct · internet"
  - "direct · LAN"
  - "via tailnet"
  - "not reachable"
- Eigen bereikbaarheid, uit A0, nu live.
- `README.md`:
  - Tailscale is niet meer nodig
  - de uitleg van de code
  - firewall: het advies "alleen het Tailscale-netwerk aanvinken" vervalt.
- **Firewall: de vrienden hebben waarschijnlijk blokkeerregels.** Wie in de Windows-melding
  "Privé" en "Openbaar" uitvinkte, kreeg expliciete *blokkeer*regels voor die profielen,
  niet alleen geen toestemming. Ongevraagd binnenkomend verkeer op de gemapte poorten wordt
  dan tegengehouden, en dat maakt B2 stil waardeloos voor het eerste contact.
  Tegelijk bellen (B3) komt er misschien toch door, via het antwoordverkeer van de
  stateful firewall, maar dat moet je controleren, niet aannemen.
  - In `README.md` een concrete instructie: Windows-beveiliging → Firewall → Een app
    toestaan → fitcom op "Privé" aanvinken, en het thuisnetwerk op profiel "Privé" zetten.
  - `docs/TESTPLAN.md`: testen met de firewall **precies** zoals de README het de vrienden
    liet instellen.
- `docs/BEVEILIGING.md`: het dreigingsmodel herschrijven. De app staat nu bewust aan
  internet, dus de sectie "aanname versus code" en B-09 gaan opnieuw.
- `PRODUCT.md`, `docs/SPEC.md`, `docs/ARCHITECTURE.md`: verbindingsmodel. Invariant 1 in
  `CLAUDE.md` in overleg.

**B7 — IPv6, alleen als A0 laat zien dat een paar het nodig heeft.**
- Dual-stack sockets (`[::]` met `IPV6_V6ONLY` uit, via `socket2`; Windows zet hem
  standaard aan), en globale IPv6-adressen als kandidaat.
- Bij Ricks pc zinloos (geen IPv6). Bij een vriend op KPN of Ziggo misschien de makkelijkste
  weg.

**Klaar als:**
- Rick en een vriend verbinden met Tailscale **uit**, alleen op elkaars code;
- voice en 12 Mbit/s schermdelen zijn niet slechter dan over het tailnet (`spoor`-sporen
  naast elkaar);
- beide apps herstarten verbindt weer zonder nieuwe code;
- een router herstarten (nieuw IP) heelt vanzelf via de derde peer of het onthouden adres;
- een paar minuten willekeurige UDP op beide poorten kost geen meetbare CPU en laat de log
  niet vollopen;
- de derde peer, die zelf geen port mapping heeft, verbindt via B4 met de andere twee.

## Fase C — alleen als A0 of B erom vraagt

Er komt geen plan voordat een meting laat zien dat het nodig is.

- **Introduceren met getimed bellen:** een `CallNow`-bericht via de derde peer, zodat beide
  kanten op hetzelfde moment bellen. Alleen als B3/B4 een paar laat liggen dat in principe
  wel kan.
- **Doorsturen via de derde peer**, blind, de AEAD-pakketten ongeopend, als laatste
  terugval voor twee peers met elk een symmetrische NAT. De prijs zit bij de uplink van een
  gamende pc (invariant 4) en in afhankelijkheid van die derde (de geest van invariant 2).
  Daarom niet standaard.
- **Anders:** de melding "no direct path to X" plus een concreet advies per routertype.
  Delta-glasvezel bijvoorbeeld zit standaard achter CGNAT en heeft in MijnDELTA een
  "Dynamisch IP"-optie.

## Bekende grenzen, ook na B

- Twee peers die tegelijk online zijn, geen van beiden port mapping hebben en allebei achter
  een symmetrische NAT zitten, zonder derde peer: geen direct pad. Dat is de natuurkunde
  van NAT, geen bug.
- Wijzigen alle IP's tegelijk terwijl niemand verbonden is, dan zijn de oude adressen
  waardeloos: codes opnieuw delen. Met een semi-vast IPv4 bij de meeste Nederlandse
  providers is dat zeldzaam.
- Een router die geen port mapping doet, vertelt ook het publieke IP niet. De eigen code
  bevat dan alleen LAN- en tailnet-adressen. Hij werkt nog steeds zolang de ander wel
  bereikbaar is: dan belt deze kant naar buiten en leert de ander het adres (B4).

## Wat Rick doet

- **Vóór release 1:** checken dat beide vrienden op 1.10.1 zitten (zie A-vooraf).
- **Release 1:** alle drie tegelijk bijwerken. Daarna de A0-regel van beide vrienden laten
  screenshotten.
- **Release 2:** iedereen plakt één keer de code van de anderen (in een groepsapp elk zijn
  code posten, dan twee keer plakken), en klikt bij de eerste start op "toestaan" bij de
  firewall.
- **Tests met de hand:** fase A tussen mac en Windows; fase B met Tailscale echt uit.
