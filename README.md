# FitCommunication

Servervrij alternatief voor Discord voor een kleine vaste groep. Draait over een
Tailscale-tailnet, zonder signaling-server, zonder TURN, zonder accounts, zonder cloud.
Alle peers zijn gelijkwaardig — er is geen host.

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
- [Tailscale](https://tailscale.com/), geïnstalleerd en ingelogd op elke machine, met
  alle machines in hetzelfde tailnet
- Om zelf te bouwen: Rust (stable; op Windows met de MSVC-toolchain) en `cmake`
  (libopus wordt vanuit broncode meegebouwd). Op macOS: `xcode-select --install` en
  `brew install cmake`.

## Installeren op een PC

1. Pak de zip uit in een map naar keuze.
2. Maak een lege map `data` naast `fitcom.exe`. Daarmee draait de app **portable**:
   config, database en logs blijven bij de exe. Doe je dit niet, dan gebruikt de app
   `%APPDATA%\FitCommunication`.
3. Start `fitcom.exe` één keer. Hij maakt een voorbeeld-`config.toml` aan en sluit niet af.
4. Pas `config.toml` aan (zie hieronder) en start opnieuw.

## Configuratie

`config.toml`:

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

[[peers]]
address = "vriend-pc"        # MagicDNS-naam of tailnet-IP zoals 100.64.0.2
label   = "Vriend"           # alleen zichtbaar tot hij voor het eerst verbindt

[[peers]]
address = "100.64.0.3"
label   = "Derde"
```

Elke peer zet de **andere twee** in zijn eigen `config.toml`. Verder hoef je niets af te
stemmen: identiteiten worden bij het eerste contact automatisch geleerd en vastgelegd
(`known_id` verschijnt dan vanzelf in het bestand). Je hoeft dus geen ID's uit te wisselen.

**MagicDNS-namen zijn te verkiezen boven IP's** — die blijven werken als een tailnet-IP
verandert. Staat MagicDNS uit, gebruik dan de `100.x.x.x`-adressen uit `tailscale status`.

#### Firewall: één vinkje, en het moet het juiste zijn

Windows vraagt bij de eerste start of `fitcom.exe` mag netwerken, met "Privé" en "Openbaar"
als twee losse vinkjes. **Zet alleen het Tailscale-netwerk aan, en laat "Privé" en
"Openbaar" uit.** Doe dit op alle drie de PC's en controleer het ook als je het ooit al eens
hebt weggeklikt — één verkeerd vinkje stelt de app open voor het hele subnet waar je op zit.

Dat is nu geen theorie. De app luistert op álle netwerkinterfaces (`0.0.0.0`) en de
identiteit die een peer claimt is nog niet cryptografisch aan zijn verbinding gebonden, dus
deze firewallregel is op dit moment de enige echte grens tussen "onze drie PC's" en
"iedereen op dezelfde wifi". Bij een LAN-party, in een hotel of op congres-wifi is dat het
verschil dat telt. Volledige onderbouwing: `docs/BEVEILIGING.md`, bevindingen B-09 en B-05.

Controleren of het goed staat:

```powershell
Get-NetFirewallRule -DisplayName "*fitcom*" | Get-NetFirewallAddressFilter
```

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
| `config.toml` | Door jou te bewerken. Mag je kopiëren tussen machines. |
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
of installatie nodig op de doel-PC.

## Testen met meerdere instanties op één PC

```bash
cargo run -p fitcom -- --data-dir C:\tmp\peerA
```

Geef elke instantie een eigen `--data-dir` en zet in elke `config.toml` een andere
`control_port` en `media_port`, met `address = "127.0.0.1"` en de poort van de ander.

## Problemen oplossen

| Wat je ziet | Wat het betekent |
|---|---|
| `offline · peer reageert niet` | De ander draait de app niet, of de firewall blokkeert. |
| `offline · <adres> opzoeken` | De naam is niet op te lossen. Staat MagicDNS aan? Draait Tailscale? |
| `versie X vs Y` | De twee PC's draaien verschillende versies. Kopieer dezelfde exe naar beide. |
| `microfoon of weergave: ...` | Het apparaat uit `config.toml` bestaat niet of is in gebruik. Haal de regel weg om het standaardapparaat te nemen. |
| Je hoort niemand | Zit de ander ook in het gesprek? Staat deafen aan? Klopt `media_port` en laat de firewall UDP door? |
| `andere identiteit dan verwacht` | Achter dit adres zit een andere installatie dan eerder. Klopt dat (nieuwe PC, `identity.toml` weg)? Haal dan `known_id` uit `config.toml`. |

Meer detail in de log: start met `FITCOM_LOG=debug`.

## Documentatie

- [docs/SPEC.md](docs/SPEC.md) — wat er gebouwd wordt en welke keuzes vastliggen
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) — protocol, synchronisatie, opbouw
- [ROADMAP.md](ROADMAP.md) — fasering
- [TODO.md](TODO.md) — wat bewust nog niet gebouwd is, waaronder file sharing
- [docs/OVERDRACHT.md](docs/OVERDRACHT.md) — stand van zaken, gemaakte keuzes, valkuilen
- [docs/TESTPLAN.md](docs/TESTPLAN.md) — wat er met echte machines getest moet worden
