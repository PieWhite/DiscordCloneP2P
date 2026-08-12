---
version: 1
slug: "crates-app-src-ui"
primary_target: "crates/app/src/ui"
related_targets: ["crates/app/frontend"]
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
- **Verbinden / herverbinden** met exponentiële backoff, plus RTT per peer.
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

**Het zestienkleurenveld (2026-08-12).** In een tweede richtingsronde (seed `c9bbaaa4`)
heeft Rick de catalogus-uitdager **"PC-98 sixteen-color field"** gekozen, boven de
toegewezen eigen richting (tactische squad-HUD) en boven het aanhouden van de
categoriestandaard uit 2026-08-04. De vier zones en het vaste donkere thema stonden vast;
al het overige — palet, typografie, materiaal, dichtheid — is vervangen.

De wereld, kort: zestien vaste kleuren op diep indigo; elke tussentint is een geordende
2px-dither van twee paletkleuren, nooit een blend; papierkleurige tekst; blauwe titelbalken
op vierkante, dubbel-omrande vensters; magenta als dé primaire actie (Join the call en
niets anders); cyaan voor links, focus en live voortgang; DotGothic16 als bitmapletter
(één gewicht — hiërarchie is grootte, kleur en celinversie); selectie inverteert zijn
cellen; afwezig dithert naar halve dichtheid (sprite en statusregel, nooit de naam);
schaduwen zijn harde offsets; toestandswissels klikken in stappen (`steps()`), niets fadet
en niets loopt oneindig. Het faalcriterium dat Rick koos: **"moeilijker af te lezen is een
fout"** — leesbaarheid in een donkere kamer wint van elke stijlvondst.

De richtingsronde van 2026-08-04 ("de categoriestandaard, met opzet") is hiermee bewust
verlaten; de comp `design/main-window.html` en `design/shots/` documenteren die oude wereld
en zijn voor deze wereld anti-referentie, geen reproductiedoel.

**Kwaliteitslat:** de catalogkaart van de wereld (board + hero) zet het afwerkingsniveau;
Discord/Slack blijft de lat voor gedragsafwerking (toetsenbord, staten, dichtheid).

## Beslist (fase 12, 2026-08-04 — ongewijzigd geldig)

- **UI-taal: Engels** voor de weergavelaag; de motor en de vier andere crates blijven
  Nederlands; de vertaling zit op één plek, `ui/state.rs`.
- **De `Snapshot`/`UiCommand`-grens blijft staan.** Deze herstyling raakte alleen
  `frontend/` plus drie renderdetails in `app.js` (icoongrammatica, naamkleurklassen,
  ledenlijst-titelbalk); geen enkel commando of event veranderde.

## Beslist (2026-08-12, deze ronde)

- **Palet en typografie staan in `DESIGN.md`** (herschreven uit de gebouwde wereld door de
  documenter) en in `.impeccable/design.json`.
- **DotGothic16** (OFL) lokaal gebundeld in `crates/app/frontend/fonts/`; Archivo is weg,
  JetBrains Mono blijft voor code en gemeten getallen — een benoemde concessie: echte code
  in de chat vraagt een echte mono.
- **Eén decoratieve handtekening:** de horizonband (geditherde schemerstreep) — onderrand
  titelbalk, onderrand instellingenvenster, streep onder lege-staat-koppen. Nergens anders.
- **Iconen:** zelfde 29 symbolen, hertekend als grammatica: stroke 2, vierkante caps,
  verstek-joins, geen afgeronde rects. De regel staat boven het `<defs>`-blok.
