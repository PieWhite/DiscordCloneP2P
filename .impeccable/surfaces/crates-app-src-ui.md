---
version: 1
slug: "crates-app-src-ui"
primary_target: "crates/app/src/ui"
related_targets: []
---

# Hoofdvenster — FitCommunication

## Scope en modus

Het hoofdvenster van de desktopapp: icoonrail, kanaal/DM-lijst, chattijdlijn, ledenlijst,
eigen titelbalk, statusbalk, instellingen en modals. **Modus: Operate** — de gebruiker maakt
een taak af, dus scanbaarheid, consistentie en verwachte affordances gaan vóór expressie.

Buiten scope: het pop-out kijkvenster voor streams. Dat is een eigen Win32-venster met eigen
D3D11-swapchain, staat buiten de UI-stack en verandert niet mee.

## Publiek en taak

Drie vaste gebruikers die elkaar persoonlijk kennen, 's avonds, in het donker, met een
headset, twee schermen, vaak een game ernaast. Ze komen van Discord.

Twee hoofdtaken in dit venster, door Rick bevestigd:

1. **Chat lezen en schrijven** — berichten, codeblokken, afbeeldingen, bestanden inline in
   één chronologische tijdlijn. Dit is het grootste vlak en verdient de meeste aandacht.
2. **Zien wie er is en erbij springen** — wie is online, wie zit in het gesprek, wie deelt.
   De primaire actie van het venster is deelnemen aan het gesprek.

Níet de hoofdtaak: streams monitoren (de overzichtstrook is bijzaak) en terugzoeken.

## Belangrijke toestanden

Deze zijn geen randgevallen maar de normale toestand, en de UI moet ze rustig tonen:

- **Eén of twee peers offline.** Invariant 7: dit is gewoon, geen foutpad. Geen alarmkleur,
  geen dialoog, geen blokkade.
- **Verbinden / herverbinden** met exponentiële backoff, plus RTT en verlies per peer.
- **Ongelezen per kanaal, per subkanaal en per DM**, los van elkaar geteld.
- **Een `@jouwnaam`-tag** — gemarkeerd in de tijdlijn, en het enige wat een Windows-melding
  rechtvaardigt. Ook in een DM: geen melding zonder tag.
- **Niet-storen** onderdrukt alle meldingen, ook een directe tag.
- **In gesprek**: spreekindicatie per deelnemer, volume per persoon, mute, deafen.
- **Delen actief**: bureaubladgeluid gaat automatisch mee, alleen een passieve statusregel.
- **Bestandsoverdracht**: voortgang, hervatten, mislukt-met-opnieuw-proberen.
- **Lege staten**: leeg subkanaal, nog nooit een DM geopend, niemand online.
- **Update beschikbaar**: bevestigingsvraag vóór toepassen, nooit stilzwijgend.

## Gekozen richting

**De categoriestandaard, met opzet.** Op 2026-08-04 gekozen boven vier volledig uitgewerkte
alternatieven (handbediende telefooncentrale, vertrekbord, Teletekst, Schiphol-signering).
De conventionele Discord-indeling is de opdracht, uitgevoerd zonder ironie en zonder
eigenzinnigheid die er alsnog in gesmokkeld wordt.

**Kwaliteitslat: Discord en Slack.** Hun afwerkingsniveau is de meetlat.

Vier zones blijven staan waar ze staan — expliciete keuze, ze zijn uitgeprobeerd en blind te
bedienen. Materiaal, typografie, kleur, dichtheid, ritme en motion zijn vrij binnen de
conventie. Donker is een eis, geen keuze: avondgebruik in het donker.

**Het memorabele moment moet in de precisie zitten, niet in een vondst.** Bij een gekozen
conventie is dat de enige plek waar het verschil gemaakt wordt: uitlijning die klopt bij elke
dichtheid, statusovergangen die niet springen, een tijdlijn die na duizend berichten nog
ritme heeft, en toetsenbordgedrag dat nergens hapert.

## Onopgeloste beslissingen

- **UI-taal.** Nederlands of Engels voor de nieuwe weergavelaag. De laag wordt toch helemaal
  opnieuw geschreven, dus dit is nu gratis en later niet. Zie `PRODUCT.md`.
- **Het palet.** De teal `#3ABFC0` en de vijf grijze lagen uit `ui/theme.rs` zijn geen
  vastgelegde waarden; alleen de soort wereld ligt vast.
- **Typografie.** Nog niet gekozen. Operate-modus wordt goed gediend door workhorse-faces.
