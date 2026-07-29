# Testplan — wat er met echte machines getest moet worden

De geautomatiseerde tests dekken de logica: convergentie van de chat, verlies en
herordening van audiopakketten, fragmentatie van beeld. Wat ze **niet** kunnen: horen
of het goed klinkt, zien of het beeld klopt, en voelen of het snel genoeg is.

Dit zijn de gevallen die jij en je vriend moeten doorlopen. Per geval staat erbij
waarom hij er staat — als iets faalt is dat de aanwijzing waar je moet kijken.

**Vooraf:**
- Zet je microfoon en koptelefoon in Windows op **48000 Hz** (geluidsinstellingen →
  apparaateigenschappen → geavanceerd). Anders moet de app herbemonsteren.
- Zorg dat beide PC's **dezelfde exe** draaien. Bij verschil zie je "versie X vs Y".
- Log bij problemen: `data\logs\fitcom.<datum>.log`. Meer detail met `FITCOM_LOG=debug`.

---

## Fase 1 — Netwerklaag ✅ al bevestigd

Al gedaan: twee PC's zagen elkaar over Tailscale. Deze twee zijn nog niet gedaan:

**1.1 Herverbinden na wegvallen**
Sluit bij je vriend de app af. Jouw scherm moet binnen ~15 s "offline" tonen. Start hem
weer; binnen enkele seconden moet hij vanzelf weer "online" staan zonder dat iemand iets
doet. *Faalt dit, dan werkt de backoff-lus niet of blokkeert de firewall het opnieuw
verbinden.*

**1.2 Alle drie tegelijk**
Zodra de derde persoon meedoet: alle drie moeten elkaar zien. Niemand mag "andere
identiteit dan verwacht" tonen. *Die melding betekent dat het koppelen van inkomende
verbindingen misgaat — precies de bug die met twee peers onzichtbaar was.*

---

## Fase 2 — Tekstchat

**2.1 Bericht komt aan**
Stuur over en weer. Moet direct verschijnen.

**2.2 Inhaalslag na offline — het belangrijkste geval**
1. Laat je vriend de app **afsluiten**.
2. Stuur vijf berichten.
3. Laat hem opstarten.

Alle vijf moeten er binnen een seconde staan, in dezelfde volgorde als bij jou, zonder
dat iemand iets doet. *Dit is de kern van fase 2. Ontbreekt er iets, dan liegt de version
vector; staat de volgorde anders, dan wordt er ergens op wall-clock gesorteerd in plaats
van op lamport.*

**2.3 Beide kanten tegelijk offline geweest**
Sluit allebei af. Start allebei op. Beide geschiedenissen moeten identiek zijn.

**2.4 Overleeft herstart**
Sluit af en start opnieuw. Alle berichten moeten er nog staan.

**2.5 Bewerken en verwijderen**
Bewerk een eigen bericht → bij hem moet de tekst wijzigen met "(bewerkt)". Verwijder er
een → moet bij hem verdwijnen. Bij zijn berichten mag jij die knoppen niet zien.

**2.6 Melding op de achtergrond**
Minimaliseer of ga naar de tray. Laat hem iets sturen. Je hoort een geluidje en ziet een
Windows-melding. *Blijft die uit, dan is de motor met de UI meegestopt — precies wat hij
niet mag doen.*

**2.7 Tray**
Sluitknop → venster verdwijnt, app draait door. Dubbelklik op het tray-icoon → terug.
Rechtermuisknop → Afsluiten → echt weg. Controleer daarna dat berichten die tijdens het
"weg zijn" gestuurd zijn, alsnog binnenkomen.

**2.8 Codeblok**
Stuur iets met ``` eromheen. Moet als codeblok verschijnen.

**2.9 Naam wijzigen**
Pas `display_name` aan in `config.toml`, herstart. Bij hem moet je nieuwe naam
verschijnen zonder dat hij iets doet.

---

## Fase 3 — Voice

**3.1 Gesprek opzetten**
Klik allebei op **Deelnemen**. Je hoort elkaar. *Hoor je niets: zit hij ook echt in het
gesprek (staat er een balkje onder zijn naam)? Laat de firewall UDP door op `media_port`?*

**3.2 Vertraging**
Tel om de beurt hardop. De vertraging moet nauwelijks merkbaar zijn — vergelijkbaar met
Discord. *Voelt het traag, noteer hoeveel; dat stuurt of de jitterbuffer te diep staat.*

**3.3 Open mic**
Wees stil. De ander mag geen achtergrondruis horen en jouw balkje moet leeg blijven.
Praat weer: het moet meteen doorkomen, en het **eind van je zin mag niet afgekapt worden**.
*Wordt het laatste woord ingeslikt, dan is de hangover te kort.*

**3.4 Ruisonderdrukking**
Zet iets luidruchtigs aan (ventilator, toetsenbord). Hij hoort jou wel, de ruis niet of
nauwelijks.

**3.5 Mute en deafen**
Mute → hij hoort je niet, jij hem wel. Deafen → jij hoort niets én hij hoort jou niet.

**3.6 Volume per persoon**
Schuif zijn volume omlaag; hij wordt zachter. Op nul hoor je hem niet meer.

**3.7 Verlaten en opnieuw deelnemen**
Verlaat, klik meteen weer op Deelnemen. Moet gewoon werken. *Faalt dit met een fout over
de mediapoort, dan is de wachtlus bij het opnieuw binden te kort.*

**3.8 Met z'n drieën**
Zodra de derde meedoet: iedereen hoort iedereen, en je kunt aan de balkjes zien wie er
praat. Als twee mensen tegelijk praten mag het niet vervormen.

**3.9 Spreken tijdens gamen — de eis die bovenaan staat**
Start een game, ga in gesprek. Let op je framerate. *Merkbare impact is een bug, geen
compromis.* Noteer wat je ziet.

**3.10 Iemand valt weg tijdens het gesprek**
Trek bij je vriend de netwerkkabel eruit of sluit de app af. Jouw kant mag niet
vastlopen of blijven kraken; hij verdwijnt gewoon uit het gesprek.

---

## Fase 4 — Screenshare

Beeld werkt van begin tot eind. Op één machine is bevestigd: 1080p op 55-56 beelden per
seconde, geen enkel beeld onderweg kwijt, scherp leesbare tekst, en 3,1 ms tussen
opnemen en tonen in een debug-build. Wat een tweede machine daaraan toevoegt is een echt
netwerk, een andere GPU, en een oordeel over hoe het voelt.

Desktop-audio (4.9) is nog niet gebouwd — dat is het laatste wat in deze fase openstaat.

**4.1** Deel je scherm; hij ziet het. Cursor is zichtbaar.
**4.2** Tekst en code zijn scherp leesbaar, geen wazige randen om gekleurde letters.
*Zijn ze grauw of juist te contrastrijk, dan staat het kleurbereik verkeerd; zie
`crates/video/src/kleur.rs`.*
**4.3** Glass-to-glass vertraging: zwaai met een venster en kijk hoeveel je achterloopt.
*Lokaal gemeten zit er 3 ms tussen opnemen en tonen, dus alles wat je hier merkt komt van
het netwerk of van de monitor zelf.*
**4.4** Delen tijdens gamen mag je framerate niet merkbaar raken.
**4.5** Niemand kijkt → geen CPU/GPU-verbruik en geen netwerkverkeer. *Kondig een bron
aan en laat hem staan zonder dat iemand op "bekijken" klikt. In de deelnemerslijst staat
dan "niemand kijkt" met een grijze stip. Zie je toch verkeer of GPU-gebruik, dan is de
belangrijkste eigenschap van deze fase stuk.*
**4.6** Kijker sluit het venster en opent het opnieuw → beeld komt binnen een seconde
terug. *Bij het opnieuw openen wordt er om een keyframe gevraagd; blijft het venster
zwart, dan komt dat verzoek niet aan of wordt het niet gehonoreerd.*
**4.7** Venster delen in plaats van scherm; alleen dat venster is zichtbaar.
**4.8** Twee mensen delen tegelijk; beide beelden zijn te zien.
**4.9** Desktop-audio komt mee en is los in volume te regelen van de stemmen.
**4.10** Bij de peer met de RTX 2080 Super: werkt decoderen daar ook? *Dit is de machine
die de codeckeuze bepaalde.*
**4.11** F11 of dubbelklik in het kijkvenster → beeldvullend, Escape of F11 → terug. Dit
mag geen resolutiewissel geven; draait er een game op datzelfde scherm, dan moet die
gewoon doorlopen.
**4.12** Deler valt weg terwijl er gekeken wordt (app afsluiten of kabel eruit). Het
kijkvenster mag niet vastlopen; het beeld bevriest en dat is genoeg.

---

## Wat je terugkoppelt

Per geval genoeg aan: **nummer + werkt / werkt niet + wat je zag**. Bij audio- of
beeldproblemen is de log van beide kanten waardevol; die staat in `data\logs\`.
