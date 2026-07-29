# FitCommunication

Servervrij alternatief voor Discord voor een kleine vaste groep. Draait over een
Tailscale-tailnet, zonder signaling-server, zonder TURN, zonder accounts, zonder cloud.
Alle peers zijn gelijkwaardig — er is geen host.

**Status:** fase 2 af — netwerklaag en tekstchat werken, inclusief het inhalen van
berichten die je gemist hebt terwijl je offline was. Voice en screenshare volgen.
Zie [ROADMAP.md](ROADMAP.md).

## Wat je nodig hebt

- Windows 11
- [Tailscale](https://tailscale.com/), geïnstalleerd en ingelogd op elke PC, met alle
  PC's in hetzelfde tailnet
- Om zelf te bouwen: Rust (stable) met de MSVC-toolchain

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

Zorg dat de firewall `fitcom.exe` toestaat op het Tailscale-netwerk. Windows vraagt dit
meestal bij de eerste start.

### Bestanden in de datamap

| Bestand | |
|---|---|
| `config.toml` | Door jou te bewerken. Mag je kopiëren tussen machines. |
| `identity.toml` | Door de app gegenereerd. **Niet kopiëren** — twee peers met dezelfde identiteit breken de chat-synchronisatie. |
| `chat.sqlite` | De volledige chatgeschiedenis. Weggooien wist je geschiedenis, maar de anderen vullen hem bij de eerstvolgende verbinding weer aan. |
| `logs/fitcom.<datum>.log` | Dit bestand is wat je nodig hebt als iets niet werkt. |

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
| `andere identiteit dan verwacht` | Achter dit adres zit een andere installatie dan eerder. Klopt dat (nieuwe PC, `identity.toml` weg)? Haal dan `known_id` uit `config.toml`. |

Meer detail in de log: start met `FITCOM_LOG=debug`.

## Documentatie

- [docs/SPEC.md](docs/SPEC.md) — wat er gebouwd wordt en welke keuzes vastliggen
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) — protocol, synchronisatie, opbouw
- [ROADMAP.md](ROADMAP.md) — fasering
- [TODO.md](TODO.md) — wat bewust nog niet gebouwd is, waaronder file sharing
