# FitCommunication

Servervrij alternatief voor Discord voor een kleine vaste groep. De pc's vinden elkaar met
een uitnodigingscode die je één keer plakt, en praten daarna rechtstreeks met elkaar: zonder
signaling-server, zonder STUN of TURN, zonder accounts, zonder cloud. Alle peers zijn
gelijkwaardig — er is geen host. Tailscale mag, maar hoeft niet meer.

**Status:** alle geplande fasen zijn af — chat, voice, screenshare, camera,
bestandsdeling, kanalen en automatische updates. Zie [ROADMAP.md](ROADMAP.md) en
[docs/OVERDRACHT.md](docs/OVERDRACHT.md).

De camera zit naast de microfoon- en koptelefoonknop en kan tegelijk met een gedeeld
scherm aan. Er wordt niets opgenomen — en het lampje blijft uit — tot iemand er
daadwerkelijk naar kijkt, net als bij een gedeeld scherm. **Camera *uitzenden* werkt
alleen op Windows;** een mac kan wel naar de camera van een Windows-peer kijken (zie
[TODO.md](TODO.md)).

## Wat je nodig hebt

- Windows 11, of macOS 14+ op Apple Silicon
- Een gewone internetverbinding. De app vraagt de eigen router om zijn poorten open te zetten
  (UPnP, PCP of NAT-PMP, wat de router kan); kan die dat niet, dan vinden de pc's elkaar via
  een derde die wel bereikbaar is. Zie "Elkaar vinden" hieronder. Tailscale is niet meer
  nodig; wie het heeft houdt het, en dan loopt het verkeer daar ook gewoon over.
- Om zelf te bouwen: Rust (stable; op Windows met de MSVC-toolchain) en `cmake`
  (libopus wordt vanuit broncode meegebouwd). Op Windows hoort bij MSVC de Windows SDK
  (zit in de C++-workload van de VS Build Tools): een release-build haalt er `fxc.exe` uit
  voor de shaders van de weergave. Op macOS: `xcode-select --install` en
  `brew install cmake`.

## Installeren op een PC

1. Pak de zip uit in een map naar keuze.
2. Maak een lege map `data` naast `fitcom.exe`. Daarmee draait de app **portable**:
   config, database en logs blijven bij de exe. Doe je dit niet, dan gebruikt de app
   `%APPDATA%\FitCommunication`.
3. Start `fitcom.exe`. Windows vraagt of de app mag netwerken: zie "Firewall" hieronder.
4. Wissel uitnodigingscodes uit met de anderen (zie "Elkaar vinden").

## Elkaar vinden: de uitnodigingscode

In Settings → Network staat **Your invite code**, een regel die begint met `fitcom1:`. Druk op
Copy en zet hem in de groepsapp; elk zijn eigen code. Plak daarna de codes van de anderen in
**Add peers** en druk op Add. Je mag een heel bericht plakken, met tekst eromheen en meerdere
codes erin. Eén keer is genoeg: de app onthoudt de peers in `config.toml` en de adressen
waarop hij ze zag in `adressen.json`, dus na een herstart vinden ze elkaar vanzelf terug.

Wat er in een code staat: je `PeerId`, je publieke sleutel, je naam en de adressen die je pc
van zichzelf kent (het adres dat de router naar buiten opende, je LAN-adres, je tailnet-adres
als je dat hebt). **Een code is geen wachtwoord.** Hij laat niemand binnen: alleen wie jij de
code van hebt geplakt komt erin, en alleen met de sleutel uit die code. Wie jouw code heeft
maar jij de zijne niet, komt niet binnen.

Lukt verbinden niet meteen, dan is dat meestal de router: Settings → Network → Reachability
zegt wat hij kan. Zolang één van de drie wel bereikbaar is, vertelt die de andere twee hoe ze
elkaar kunnen bellen.

## Configuratie

Normaal hoef je `config.toml` niet aan te raken: peers komen erbij met hun code. Wat er in
staat:

```toml
display_name     = "Rick"    # hoe de anderen jou zien
control_port     = 41650
media_port       = 41651
minimize_to_tray = true      # sluitknop verbergt naar de tray in plaats van af te sluiten
autostart        = false     # meestarten met Windows

# Weglaten = het standaardapparaat van Windows. De naam moet exact overeenkomen met
# wat Windows toont; de app schrijft de beschikbare namen bij het starten in de log.
# input_device  = "Microfoon (BlackShark V3 Pro - Chat)"
# output_device = "Luidsprekers (BlackShark V3 Pro - Chat)"

port_mapping     = true      # de router de twee poorten laten openzetten (UPnP/PCP/NAT-PMP)

# Komt erbij als je een code plakt; met de hand bewerken hoeft niet.
[[peers]]
label      = "Vriend"
known_id   = "…"             # zijn PeerId, uit zijn code
public_key = "…"             # zijn sleutel, uit zijn code: alleen die komt binnen
addresses  = ["84.104.12.9:41650", "192.168.2.10:41650"]

# Een peer op het tailnet kan ook zonder code, met zijn tailnet-adres of MagicDNS-naam. Zijn
# sleutel wordt dan bij het eerste contact over het tailnet vastgelegd.
[[peers]]
address = "100.64.0.3"
label   = "Derde"
```

Een sleutel wordt alleen vastgelegd uit een code, of bij het eerste contact over het tailnet;
nooit op een LAN- of internetadres, want daar zou de eerste die zich meldt de identiteit kapen.
Daarna komt alleen wie die sleutel heeft binnen, vanaf welk adres ook.

#### Firewall: "Privé" aanvinken, en het thuisnetwerk op "Privé"

Zonder Tailscale komen de anderen rechtstreeks via internet binnen, op de twee poorten die de
router voor de app openzet. Windows Firewall moet dat dan ook doorlaten. Bij de eerste start
vraagt Windows of `fitcom.exe` mag netwerken, met "Privé" en "Openbaar" als twee vinkjes.

1. **Vink "Privé" aan** (en laat "Openbaar" uit). Klikte je die melding ooit weg of vinkte je
   alles uit, dan heeft Windows expliciete *blokkeer*regels gemaakt, en die winnen. Herstel:
   Windows-beveiliging → Firewall- en netwerkbeveiliging → Een app toestaan via de firewall →
   Instellingen wijzigen → zoek `fitcom` → vink **Privé** aan, **Openbaar** uit → OK.
2. **Zet je thuisnetwerk op "Privé".** Instellingen → Netwerk en internet → Wi-Fi of Ethernet
   → je netwerk → Netwerkprofieltype → **Privénetwerk**. Op een openbaar profiel (hotel,
   congres) laat de firewall de app dan bewust niet toe.

Controleren of het goed staat:

```powershell
Get-NetFirewallRule -DisplayName "*fitcom*" | Select-Object DisplayName, Enabled, Action, Profile
```

Er hoort een regel met `Action Allow` en `Profile Private` bij te staan, en geen `Block` voor
`Private`. Waarom dit veilig is terwijl de app nu aan internet staat: elke verbinding bewijst
met een getekende handshake wie hij is, en alleen vastgelegde sleutels komen binnen; alle media
is versleuteld. Zie `docs/BEVEILIGING.md`, het dreigingsmodel.

#### Waar je de zip uitpakt

Pak de release uit **onder je gebruikersprofiel** (bijvoorbeeld
`C:\Users\<jij>\FitCommunication`), niet in de root van `C:\`. De app schrijft in portable
modus zijn data naast de exe, en een map in de root van `C:\` erft ruime rechten: een ander
niet-admin-account op diezelfde PC kan daar dan een vervangende `fitcom.exe` neerzetten.
Onder je profiel bestaat dat probleem niet. Zie B-47 in `docs/BEVEILIGING.md`.

### macOS

Bouw met `./scripts/bundle-mac.sh` (of pak de zip met `FitCommunication.app` uit) en
start met rechtsklik → Open (eenmalig, vanwege Gatekeeper). De datamap staat in
`~/Library/Application Support/FitCommunication`; een `data`-map naast de app werkt
niet binnen een .app-bundel. Verder gelden dezelfde stappen als hierboven.

- **Microfoon** en **schermopname** zijn permissies: macOS vraagt om de microfoon bij
  de eerste keer deelnemen aan voice, en om Screen Recording bij de eerste keer
  delen/kiezen van een bron. Na het toekennen van Screen Recording wil macOS de app
  opnieuw gestart zien.
- De app is ad-hoc gesigneerd: na een **update** vraagt macOS de
  Screen-Recording-permissie opnieuw. Dat is de prijs van signing zonder
  Apple-Developer-account.
- Apparaatnamen in `config.toml` zijn de macOS-namen ("MacBook Pro Microphone"); de
  app schrijft de beschikbare namen bij het starten in de log.
- Automatische updates tussen peers doet de mac bewust niet mee: hij bouwt uit de
  broncode. Houd de versies gelijk.

### Bestanden in de datamap

| Bestand | |
|---|---|
| `config.toml` | Je peers en instellingen. Te bewerken, maar codes plakken in de app is makkelijker. |
| `adressen.json` | Waar de peers de laatste keer gezien zijn. Weggooien mag; dan duurt het eerste verbinden misschien wat langer. |
| `identity.toml` | Door de app gegenereerd. **Niet kopiëren** — twee peers met dezelfde identiteit breken de chat-synchronisatie. |
| `chat.sqlite` | De volledige chatgeschiedenis. Weggooien wist je geschiedenis, maar de anderen vullen hem bij de eerstvolgende verbinding weer aan. |
| `logs/fitcom.<datum>.log` | Dit bestand is wat je nodig hebt als iets niet werkt. |

## Voice

Klik links onderin op **Deelnemen**. Je hoort en spreekt dan met iedereen die ook
deelneemt; er is geen server die mixt, iedereen stuurt rechtstreeks naar iedereen.
Zie je "er is een gesprek bezig", dan zit er al iemand te wachten.

- **Open mic:** er wordt alleen verstuurd als je daadwerkelijk praat. Ben je stil, dan
  gaat er niets over de lijn en doet de app vrijwel niets.
- **Ruisonderdrukking** staat altijd aan.
- **Mute** zet je microfoon uit, **deafen** zet ook je microfoon uit — als jij niemand
  hoort, hoort niemand jou.
- Het volume per persoon stel je in met de schuif onder zijn naam.

Er zit geen echo-onderdrukking in: dat kan omdat jullie alle drie een headset gebruiken.
Speel je het geluid via luidsprekers af, dan horen de anderen zichzelf terug.

**Zet je microfoon en koptelefoon in Windows op 48000 Hz** (op macOS is 48000 Hz al
de standaard). Staat er iets anders, dan moet de app herbemonsteren en dat kost
onnodig kwaliteit. De app waarschuwt hierover in de log.

## Chat

Berichten komen aan zodra de ander online is. Was iemand weg, dan haalt hij bij de
eerstvolgende verbinding vanzelf op wat hij gemist heeft — je hoeft daar niets voor te
doen en het maakt niet uit hoe lang hij weg was. Zolang één peer een bericht heeft, komt
het uiteindelijk bij iedereen.

- **Enter** verstuurt, **shift+enter** maakt een nieuwe regel.
- Tekst tussen ` ``` ` wordt als codeblok getoond.
- Je eigen berichten kun je bewerken en verwijderen; die van anderen niet.

Staat de app niet op de voorgrond, dan krijg je een Windows-melding met een geluidje.
Met `minimize_to_tray = true` (standaard) verbergt de sluitknop het venster naar de tray
in plaats van af te sluiten: de app blijft dan synchroniseren en melden terwijl je gamet.
Dubbelklik op het tray-icoon om terug te komen, of gebruik het rechtermuismenu om echt
af te sluiten.

## Zelf bouwen

```bash
cargo build --release
```

De exe komt in `target/release/fitcom.exe`. Die is los te kopiëren; er is geen runtime
of installatie nodig op de doel-PC. Kopieer altijd een release-build: een debug-exe
compileert zijn shaders bij het starten uit de bronnen op de bouwmachine en tekent op een
andere pc niets.

## Testen met meerdere instanties op één PC

```bash
cargo run -p fitcom -- --data-dir C:\tmp\peerA
```

Geef elke instantie een eigen `--data-dir` en zet in elke `config.toml` een andere
`control_port` en `media_port`, met `address = "127.0.0.1"` en de poort van de ander (of
wissel hun codes uit), en `port_mapping = false`: anders vraagt elke instantie de router om
zijn poorten.

## Problemen oplossen

| Wat je ziet | Wat het betekent |
|---|---|
| `offline · peer reageert niet` | De ander draait de app niet, of de firewall blokkeert. Kijk bij de ander naar "Firewall" hierboven en naar Reachability in Settings → Network. |
| `offline · <adres> opzoeken` | De naam is niet op te lossen. Staat MagicDNS aan? Draait Tailscale? Zonder Tailscale: plak zijn code. |
| `No key known for this peer` | Deze peer staat in de config zonder sleutel en is niet op het tailnet. Plak zijn code. |
| Een code wordt geweigerd met "different key" | Voor die peer staat al een andere sleutel vast. Heeft hij opnieuw geïnstalleerd? Haal dan `known_id` en `public_key` van hem uit `config.toml` en plak zijn code opnieuw. |
| `versie X vs Y` | De twee PC's draaien verschillende versies. Kopieer dezelfde exe naar beide. |
| `microfoon of weergave: ...` | Het apparaat uit `config.toml` bestaat niet of is in gebruik. Haal de regel weg om het standaardapparaat te nemen. |
| Je hoort niemand | Zit de ander ook in het gesprek? Staat deafen aan? Klopt `media_port` en laat de firewall UDP door? |
| `A different identity answered at this address` | Achter dit adres zit een andere installatie dan eerder. Klopt dat (nieuwe PC, `identity.toml` weg)? Haal dan `known_id` en `public_key` uit `config.toml` en plak zijn nieuwe code. |

Meer detail in de log: start met `FITCOM_LOG=debug`.

## Documentatie

- [docs/SPEC.md](docs/SPEC.md) — wat er gebouwd wordt en welke keuzes vastliggen
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) — protocol, synchronisatie, opbouw
- [ROADMAP.md](ROADMAP.md) — fasering
- [TODO.md](TODO.md) — wat bewust nog niet gebouwd is, waaronder file sharing
- [docs/OVERDRACHT.md](docs/OVERDRACHT.md) — stand van zaken, gemaakte keuzes, valkuilen
- [docs/TESTPLAN.md](docs/TESTPLAN.md) — wat er met echte machines getest moet worden
