---
name: run-peers
description: Start meerdere FitCommunication-instanties naast elkaar op deze PC om de mesh handmatig te testen, en lees hun logs. Gebruik dit wanneer je verbindingsgedrag, chat-sync of UI-gedrag tussen peers wilt zien zonder een tweede machine.
---

# Lokale peers draaien

Draait N instanties met elk een eigen datamap, poort en identiteit, allemaal via
loopback met elkaar verbonden in een volledige mesh.

## Starten

```powershell
.\scripts\run-peers.ps1 -Count 3
```

Zonder `-Count` zijn het er 2. Met `-Release` een release-build (nodig zodra je naar
latency of CPU-verbruik van media kijkt; een debug-build zegt daar niets over).

Het script sluit eerst bestaande instanties af en gooit `.localpeers\` weg, zodat je
altijd met een schone staat begint.

## Logs lezen

Elke instantie logt in zijn eigen map:

```powershell
Get-Content .\.localpeers\peer1\logs\*.log -Wait
```

Meer detail: zet `$env:FITCOM_LOG = "debug"` vóór het starten. Op `debug` zie je de
verbindingsafhandeling, inclusief botsende verbindingen en geparkeerde inkomende
verbindingen. `quinn` en `rustls` staan standaard gedempt; zet ze expliciet aan
(`FITCOM_LOG="debug,quinn=debug"`) als je in de QUIC-laag moet kijken.

## Stoppen

```powershell
.\scripts\run-peers.ps1 -Stop
```

Doe dit altijd voordat je opnieuw bouwt: een draaiende `fitcom.exe` blokkeert het
overschrijven van de exe en `cargo build` faalt dan met "Toegang geweigerd (os error 5)".

## Wanneer dit niet het juiste gereedschap is

Voor verbindingsgedrag dat je herhaald wilt kunnen controleren, schrijf een test in
`crates/net/tests/mesh.rs` in plaats van handmatig te kijken. Die tests draaien twee
meshes in één proces over loopback en dekken verbinden, wegvallen en herverbinden al.
Handmatig draaien is voor UI en media — dingen die je moet zien of horen.
