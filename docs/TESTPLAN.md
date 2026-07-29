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

Van desktop-audio is de opnamekant bevestigd: met geluid aan komen er pakketten uit,
zonder geluid geen enkel. Of het aan de andere kant ook klínkt kan alleen met een tweede
machine — op één PC tapt de loopback het geluid af dat de andere instantie net afspeelde.

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
**4.9** Desktop-audio komt mee en is los in volume te regelen van de stemmen. *Jullie
moeten allebei in het gesprek zitten; het geluid gaat over de voice-verbinding mee.
Zonder gesprek staat de knop uitgegrijsd. Zet ook de schuif op nul: dan hoor je zijn
spel niet meer maar hem nog wel.*
**4.10** Bij de peer met de RTX 2080 Super: werkt decoderen daar ook? *Dit is de machine
die de codeckeuze bepaalde.*
**4.11** F11 of dubbelklik in het kijkvenster → beeldvullend, Escape of F11 → terug. Dit
mag geen resolutiewissel geven; draait er een game op datzelfde scherm, dan moet die
gewoon doorlopen.
**4.12** Deler valt weg terwijl er gekeken wordt (app afsluiten of kabel eruit). Het
kijkvenster mag niet vastlopen; het beeld bevriest en dat is genoeg.

---

## Fase 5 — Screenshare uitbreiding

Venster-capture, meerdere bronnen tegelijk delen en meerdere streams tegelijk bekijken
zaten al in fase 4 en zijn daar getest (4.7, 4.8). Dit zijn alleen de twee dingen die in
fase 5 zijn toegevoegd.

**5.1 Video-instellingen aanpassen**
Open "video-instellingen" in de statusbalk, zet de bitrate flink lager, klik toepassen
terwijl je al aan het delen bent. Bij de kijker moet het beeld merkbaar minder scherp
worden zonder dat de stream opnieuw opgezet hoeft te worden aan zijn kant. *Verandert er
niets, dan herstart `herstart_lopende_delers` de deler niet echt.*

**5.2 HEVC-waarschuwing**
Zet de codec op HEVC in het instellingenscherm. Er moet een waarschuwing verschijnen dat
niet iedereen dit kan decoderen. Deel een scherm; bij de peer met de RTX 2080 Super moet
kijken **mislukken** met een foutmelding, niet een bevroren of grauw beeld.

**5.3 Overzichtstrook**
Bekijk twee streams tegelijk (van dezelfde of verschillende peers). Boven de chat moet
een strook verschijnen met een levend, klein beeld van allebei, met de titel eronder.
Sluit een van de twee kijkvensters → zijn tegel verdwijnt uit de strook. Sluit de laatste
→ de hele strook verdwijnt.
**5.4** Niemand bekijkt iets → de overzichtstrook neemt geen ruimte in. *Dit is dezelfde
eigenschap als 4.5, maar dan voor de strook: hij hoort niet te bestaan als er niets te
tonen is.*

---

## Fase 6 — Bestandsdeling

De volledige keten — aanbieden, syncen, aanvragen, streamen, hervatten, hashen — is al
bevestigd met een geautomatiseerde test door de echte motor heen, over loopback-QUIC
(`crates/app/tests/file_deling.rs`, geen GPU nodig, draait gewoon mee met `cargo test`).
Wat een tweede machine daaraan toevoegt: een echt netwerk met echt pakketverlies, en of de
bestandsdialoog en downloadknoppen in het echt doen wat ze beloven.

**6.1 Aanbieden komt aan zonder dat iemand downloadt**
Klik "Bestand delen…", kies een bestand. Bij je vriend moet het meteen in het
bestandenpaneel verschijnen, met de juiste naam en grootte, zonder dat hij iets doet.
*Dit is de kern van fase 6: het aanbod is een gewone oplog-op en moet zich dus precies zo
gedragen als een chatbericht.*

**6.2 Downloaden levert een identiek bestand op**
Download het aangeboden bestand. Vergelijk het resultaat met het origineel (grootte,
en bij twijfel een checksum met de hand). *Klopt de hash niet, dan had de test in
`file_deling.rs` dat op deze machine ook al moeten laten zien — meld dat als een
regressie, niet als iets dat alleen "in het echt" fout gaat.*

**6.3 Groot bestand tijdens een gesprek en/of screenshare**
Deel iets van een paar honderd MB tot een paar GB terwijl je in gesprek bent of je scherm
deelt. Spraak en beeld mogen geen hapering vertonen. *De bulkbytes gaan over een eigen
QUIC-stream naast de control-stream — precies om dit te voorkomen. Merk je toch hapering,
dan is dat een aanwijzing dat er ergens alsnog gedeeld verkeer optreedt.*

**6.4 Hervatten na een onderbreking**
Start een download van een groter bestand, sluit tijdens de overdracht bij je vriend de
app af (of trek de netwerkkabel eruit). Herstart hem en klik nogmaals downloaden (of
"opnieuw proberen" als de status al op mislukt staat). De overdracht moet verdergaan
vanaf ongeveer waar hij was, niet vanaf 0. *Bevestig dit ook door te kijken of het
tussentijdse `.part`-bestand in de downloadmap groter is dan 0 bytes vlak na de
onderbreking.*

**6.5 Aanbieder heeft het bestand niet meer**
Bied een bestand aan, verwijder of verplaats het daarna van schijf bij de aanbieder, en
probeer het dan bij de andere kant te downloaden. Moet netjes mislukken met een duidelijke
status ("mislukt: ...") en een knop om het later opnieuw te proberen — geen hang op
"bezig" die nooit meer verandert.

**6.6 Twee bestanden tegelijk van dezelfde aanbieder**
Bied twee verschillende bestanden aan en download ze allebei tegelijk. Beide moeten
correct en compleet aankomen zonder dat de bytes door elkaar raken. *Dit test de header
op de uni-stream die de twee overdrachten uit elkaar houdt.*

**6.7 Downloadmap**
Controleer dat het gedownloade bestand terechtkomt in de map die `config.toml`'s
`download_dir` aangeeft (of `<datamap>/downloads` als die leeg is), en dat "map openen"
in de UI daadwerkelijk die map opent.

---

## Kanalen (DM's)

De volledige keten — DM versturen, alleen bij de geadresseerde aankomen, nooit bij de
derde peer ook niet via doorsturen — is al bevestigd met drie echte motoren over
loopback-QUIC in volledige mesh (`crates/app/tests/chat_sync.rs`). Wat een tweede en
derde machine daaraan toevoegen: of de knoppen in het echt doen wat ze beloven, en het
geval waarin twee DM-partners elkaar niet rechtstreeks kunnen bereiken.

**K.1 DM komt aan, alleen bij de geadresseerde**
Klik bij je vriend op de DM-knop naast jouw naam en stuur iets. Bij jou moet het
verschijnen zodra je op jouw beurt de DM-knop naast zijn naam opent. Bij de derde peer
mag het **nergens** verschijnen — niet in het algemene kanaal, niet in een DM-venster met
iemand anders. *Dit is de kern van deze uitbreiding: zie je het bericht toch bij de
derde, dan lekt er iets in de kanaal-filtering.*

**K.2 Ongelezen-badge**
Laat je vriend je een DM sturen terwijl je in het algemene kanaal zit. Er moet een apart
getal op zijn DM-knop verschijnen, los van de teller op "# Algemeen". Open de DM → de
badge verdwijnt, het algemene kanaal blijft ongemoeid (en andersom).

**K.3 Bewerken en verwijderen in een DM**
Bewerk en verwijder een eigen DM-bericht. Moet bij de ander bijwerken, precies als in het
algemene kanaal. Bij zijn berichten mag jij die knoppen niet zien.

**K.4 Bestand delen in een DM**
Open een DM-venster en klik "Bestand delen…" daarbinnen. Het bestand moet alleen in dát
DM-venster verschijnen bij de geadresseerde — niet in het algemene bestandenpaneel, en
niet bij de derde peer. Download het bij de geadresseerde: moet identiek aankomen, net
als bij fase 6.

**K.5 Geschiedenis-inhaal geldt ook voor DM's**
Stuur een vriend een DM terwijl hij offline is. Laat hem opstarten: de DM moet er staan
zodra jullie weer verbinden, zonder dat iemand iets doet.

**K.6 DM tussen twee peers die elkaar niet rechtstreeks bereiken**
Lastigste geval, alleen te proberen als je de mesh handmatig kunt opbreken (bijvoorbeeld
met een firewallregel tussen twee van de drie machines): als A en B geen directe
verbinding hebben maar allebei wel met C, dan mag een DM tussen A en B **niet** via C
aankomen — hij moet gewoon wachten tot A en B elkaar weer rechtstreeks kunnen bereiken.
*Dit is de bewuste trade-off uit `docs/ARCHITECTURE.md` (sectie "Kanalen"): een DM
profiteert niet van de doorstuurhulp die het algemene kanaal wel heeft. Komt de DM tóch
via C aan, dan is de kanaal-filtering in het doorstuurpad kapot — dat zou C in staat
stellen de inhoud te lezen, en dat is precies wat dit ontwerp voorkomt.*

---

## Wat je terugkoppelt

Per geval genoeg aan: **nummer + werkt / werkt niet + wat je zag**. Bij audio- of
beeldproblemen is de log van beide kanten waardevol; die staat in `data\logs\`.
