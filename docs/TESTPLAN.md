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

Al gedaan: twee PC's zagen elkaar over Tailscale. Onderstaande twee zijn inmiddels ook
door Rick met de hand bevestigd.

**1.1 Herverbinden na wegvallen** ✅ bevestigd
Sluit bij je vriend de app af. Jouw scherm moet binnen ~15 s "offline" tonen. Start hem
weer; binnen enkele seconden moet hij vanzelf weer "online" staan zonder dat iemand iets
doet. *Faalt dit, dan werkt de backoff-lus niet of blokkeert de firewall het opnieuw
verbinden.*

**1.2 Alle drie tegelijk** ✅ bevestigd
Zodra de derde persoon meedoet: alle drie moeten elkaar zien. Niemand mag "andere
identiteit dan verwacht" tonen. *Die melding betekent dat het koppelen van inkomende
verbindingen misgaat — precies de bug die met twee peers onzichtbaar was.*

---

## Fase 2 — Tekstchat ✅ volledig bevestigd door Rick

**2.1 Bericht komt aan** ✅ bevestigd
Stuur over en weer. Moet direct verschijnen.

**2.2 Inhaalslag na offline — het belangrijkste geval** ✅ bevestigd
1. Laat je vriend de app **afsluiten**.
2. Stuur vijf berichten.
3. Laat hem opstarten.

Alle vijf moeten er binnen een seconde staan, in dezelfde volgorde als bij jou, zonder
dat iemand iets doet. *Dit is de kern van fase 2. Ontbreekt er iets, dan liegt de version
vector; staat de volgorde anders, dan wordt er ergens op wall-clock gesorteerd in plaats
van op lamport.*

**2.3 Beide kanten tegelijk offline geweest** ✅ bevestigd
Sluit allebei af. Start allebei op. Beide geschiedenissen moeten identiek zijn.

**2.4 Overleeft herstart** ✅ bevestigd
Sluit af en start opnieuw. Alle berichten moeten er nog staan.

**2.5 Bewerken en verwijderen** ✅ bevestigd
Bewerk een eigen bericht → bij hem moet de tekst wijzigen met "(bewerkt)". Verwijder er
een → moet bij hem verdwijnen. Bij zijn berichten mag jij die knoppen niet zien.

**2.6 Melding op de achtergrond** ✅ bevestigd
Minimaliseer of ga naar de tray. Laat hem iets sturen. Je hoort een geluidje en ziet een
Windows-melding. *Blijft die uit, dan is de motor met de UI meegestopt — precies wat hij
niet mag doen.*

**2.7 Tray** ✅ bevestigd
Sluitknop → venster verdwijnt, app draait door. Dubbelklik op het tray-icoon → terug.
Rechtermuisknop → Afsluiten → echt weg. Controleer daarna dat berichten die tijdens het
"weg zijn" gestuurd zijn, alsnog binnenkomen.

**2.8 Codeblok** ✅ bevestigd
Stuur iets met ``` eromheen. Moet als codeblok verschijnen.

**2.9 Naam wijzigen** ✅ bevestigd
Pas `display_name` aan in `config.toml`, herstart. Bij hem moet je nieuwe naam
verschijnen zonder dat hij iets doet.

---

## Fase 3 — Voice ✅ volledig bevestigd door Rick

**3.1 Gesprek opzetten** ✅ bevestigd
Klik allebei op **Deelnemen**. Je hoort elkaar. *Hoor je niets: zit hij ook echt in het
gesprek (staat er een balkje onder zijn naam)? Laat de firewall UDP door op `media_port`?*

**3.2 Vertraging** ✅ bevestigd
Tel om de beurt hardop. De vertraging moet nauwelijks merkbaar zijn — vergelijkbaar met
Discord. *Voelt het traag, noteer hoeveel; dat stuurt of de jitterbuffer te diep staat.*

**3.3 Open mic** ✅ bevestigd
Wees stil. De ander mag geen achtergrondruis horen en jouw balkje moet leeg blijven.
Praat weer: het moet meteen doorkomen, en het **eind van je zin mag niet afgekapt worden**.
*Wordt het laatste woord ingeslikt, dan is de hangover te kort.*

**3.4 Ruisonderdrukking** ✅ bevestigd
Zet iets luidruchtigs aan (ventilator, toetsenbord). Hij hoort jou wel, de ruis niet of
nauwelijks.

**3.5 Mute en deafen** ✅ bevestigd
Mute → hij hoort je niet, jij hem wel. Deafen → jij hoort niets én hij hoort jou niet.

**3.6 Volume per persoon** ✅ bevestigd
Schuif zijn volume omlaag; hij wordt zachter. Op nul hoor je hem niet meer.

**3.7 Verlaten en opnieuw deelnemen** ✅ bevestigd
Verlaat, klik meteen weer op Deelnemen. Moet gewoon werken. *Faalt dit met een fout over
de mediapoort, dan is de wachtlus bij het opnieuw binden te kort.*

**3.8 Met z'n drieën** ✅ bevestigd
Zodra de derde meedoet: iedereen hoort iedereen, en je kunt aan de balkjes zien wie er
praat. Als twee mensen tegelijk praten mag het niet vervormen.

**3.9 Spreken tijdens gamen — de eis die bovenaan staat** ✅ bevestigd
Start een game, ga in gesprek. Let op je framerate. *Merkbare impact is een bug, geen
compromis.* Noteer wat je ziet.

**3.10 Iemand valt weg tijdens het gesprek** ✅ bevestigd
Trek bij je vriend de netwerkkabel eruit of sluit de app af. Jouw kant mag niet
vastlopen of blijven kraken; hij verdwijnt gewoon uit het gesprek.

---

## Fase 4 — Screenshare ✅ volledig bevestigd door Rick

Beeld werkt van begin tot eind. Op één machine is bevestigd: 1080p op 55-56 beelden per
seconde, geen enkel beeld onderweg kwijt, scherp leesbare tekst, en 3,1 ms tussen
opnemen en tonen in een debug-build. Wat een tweede machine daaraan toevoegt — een echt
netwerk, een andere GPU, en een oordeel over hoe het voelt — is inmiddels ook bevestigd,
zie 4.1 t/m 4.12 hieronder.

Van desktop-audio is de opnamekant bevestigd: met geluid aan komen er pakketten uit,
zonder geluid geen enkel. Of het aan de andere kant ook klínkt is met een tweede machine
gecontroleerd (4.9) — op één PC tapt de loopback het geluid af dat de andere instantie net
afspeelde, dus dat kon niet eerder.

**4.1** ✅ Deel je scherm; hij ziet het. Cursor is zichtbaar.
**4.2** ✅ Tekst en code zijn scherp leesbaar, geen wazige randen om gekleurde letters.
*Zijn ze grauw of juist te contrastrijk, dan staat het kleurbereik verkeerd; zie
`crates/video/src/kleur.rs`.*
**4.3** ✅ Glass-to-glass vertraging: zwaai met een venster en kijk hoeveel je achterloopt.
*Lokaal gemeten zit er 3 ms tussen opnemen en tonen, dus alles wat je hier merkt komt van
het netwerk of van de monitor zelf.*
**4.4** ✅ Delen tijdens gamen mag je framerate niet merkbaar raken.
**4.5** ✅ Niemand kijkt → geen CPU/GPU-verbruik en geen netwerkverkeer. *Kondig een bron
aan en laat hem staan zonder dat iemand op "bekijken" klikt. In de deelnemerslijst staat
dan "niemand kijkt" met een grijze stip. Zie je toch verkeer of GPU-gebruik, dan is de
belangrijkste eigenschap van deze fase stuk.*
**4.6** ✅ Kijker sluit het venster en opent het opnieuw → beeld komt binnen een seconde
terug. *Bij het opnieuw openen wordt er om een keyframe gevraagd; blijft het venster
zwart, dan komt dat verzoek niet aan of wordt het niet gehonoreerd.*
**4.7** ✅ Venster delen in plaats van scherm; alleen dat venster is zichtbaar.
**4.8** ✅ Twee mensen delen tegelijk; beide beelden zijn te zien.
**4.9** ✅ Desktop-audio komt mee en is los in volume te regelen van de stemmen. *Jullie
moeten allebei in het gesprek zitten; het geluid gaat over de voice-verbinding mee.
Zonder gesprek staat de knop uitgegrijsd. Zet ook de schuif op nul: dan hoor je zijn
spel niet meer maar hem nog wel.*
**4.10** ✅ Bij de peer met de RTX 2080 Super: werkt decoderen daar ook? *Dit is de machine
die de codeckeuze bepaalde.*
**4.11** ✅ F11 of dubbelklik in het kijkvenster → beeldvullend, Escape of F11 → terug. Dit
mag geen resolutiewissel geven; draait er een game op datzelfde scherm, dan moet die
gewoon doorlopen.
**4.12** ✅ Deler valt weg terwijl er gekeken wordt (app afsluiten of kabel eruit). Het
kijkvenster mag niet vastlopen; het beeld bevriest en dat is genoeg.

---

## Fase 5 — Screenshare uitbreiding ✅ volledig bevestigd door Rick

Venster-capture, meerdere bronnen tegelijk delen en meerdere streams tegelijk bekijken
zaten al in fase 4 en zijn daar getest (4.7, 4.8). Dit zijn alleen de twee dingen die in
fase 5 zijn toegevoegd.

**5.1 Video-instellingen aanpassen** ✅ bevestigd
Open "video-instellingen" in de statusbalk, zet de bitrate flink lager, klik toepassen
terwijl je al aan het delen bent. Bij de kijker moet het beeld merkbaar minder scherp
worden zonder dat de stream opnieuw opgezet hoeft te worden aan zijn kant. *Verandert er
niets, dan herstart `herstart_lopende_delers` de deler niet echt.*

**5.2 HEVC-waarschuwing** ✅ bevestigd
Zet de codec op HEVC in het instellingenscherm. Er moet een waarschuwing verschijnen dat
niet iedereen dit kan decoderen. Deel een scherm; bij de peer met de RTX 2080 Super moet
kijken **mislukken** met een foutmelding, niet een bevroren of grauw beeld.

**5.3 Overzichtstrook** ✅ bevestigd
Bekijk twee streams tegelijk (van dezelfde of verschillende peers). Boven de chat moet
een strook verschijnen met een levend, klein beeld van allebei, met de titel eronder.
Sluit een van de twee kijkvensters → zijn tegel verdwijnt uit de strook. Sluit de laatste
→ de hele strook verdwijnt.
**5.4** ✅ Niemand bekijkt iets → de overzichtstrook neemt geen ruimte in. *Dit is dezelfde
eigenschap als 4.5, maar dan voor de strook: hij hoort niet te bestaan als er niets te
tonen is.*

---

## Fase 6 — Bestandsdeling ✅ volledig bevestigd door Rick

De volledige keten — aanbieden, syncen, aanvragen, streamen, hervatten, hashen — is al
bevestigd met een geautomatiseerde test door de echte motor heen, over loopback-QUIC
(`crates/app/tests/file_deling.rs`, geen GPU nodig, draait gewoon mee met `cargo test`).
Wat een tweede machine daaraan toevoegt — een echt netwerk met echt pakketverlies, en of de
bestandsdialoog en downloadknoppen in het echt doen wat ze beloven — is inmiddels ook
bevestigd, zie 6.1 t/m 6.7 hieronder.

**6.1 Aanbieden komt aan zonder dat iemand downloadt** ✅ bevestigd
Klik "Bestand delen…", kies een bestand. Bij je vriend moet het meteen in het
bestandenpaneel verschijnen, met de juiste naam en grootte, zonder dat hij iets doet.
*Dit is de kern van fase 6: het aanbod is een gewone oplog-op en moet zich dus precies zo
gedragen als een chatbericht.*

**6.2 Downloaden levert een identiek bestand op** ✅ bevestigd
Download het aangeboden bestand. Vergelijk het resultaat met het origineel (grootte,
en bij twijfel een checksum met de hand). *Klopt de hash niet, dan had de test in
`file_deling.rs` dat op deze machine ook al moeten laten zien — meld dat als een
regressie, niet als iets dat alleen "in het echt" fout gaat.*

**6.3 Groot bestand tijdens een gesprek en/of screenshare** ✅ bevestigd
Deel iets van een paar honderd MB tot een paar GB terwijl je in gesprek bent of je scherm
deelt. Spraak en beeld mogen geen hapering vertonen. *De bulkbytes gaan over een eigen
QUIC-stream naast de control-stream — precies om dit te voorkomen. Merk je toch hapering,
dan is dat een aanwijzing dat er ergens alsnog gedeeld verkeer optreedt.*

**6.4 Hervatten na een onderbreking** ✅ bevestigd
Start een download van een groter bestand, sluit tijdens de overdracht bij je vriend de
app af (of trek de netwerkkabel eruit). Herstart hem en klik nogmaals downloaden (of
"opnieuw proberen" als de status al op mislukt staat). De overdracht moet verdergaan
vanaf ongeveer waar hij was, niet vanaf 0. *Bevestig dit ook door te kijken of het
tussentijdse `.part`-bestand in de downloadmap groter is dan 0 bytes vlak na de
onderbreking.*

**6.5 Aanbieder heeft het bestand niet meer** ✅ bevestigd
Bied een bestand aan, verwijder of verplaats het daarna van schijf bij de aanbieder, en
probeer het dan bij de andere kant te downloaden. Moet netjes mislukken met een duidelijke
status ("mislukt: ...") en een knop om het later opnieuw te proberen — geen hang op
"bezig" die nooit meer verandert.

**6.6 Twee bestanden tegelijk van dezelfde aanbieder** ✅ bevestigd
Bied twee verschillende bestanden aan en download ze allebei tegelijk. Beide moeten
correct en compleet aankomen zonder dat de bytes door elkaar raken. *Dit test de header
op de uni-stream die de twee overdrachten uit elkaar houdt.*

**6.7 Downloadmap** ✅ bevestigd
Controleer dat het gedownloade bestand terechtkomt in de map die `config.toml`'s
`download_dir` aangeeft (of `<datamap>/downloads` als die leeg is), en dat "map openen"
in de UI daadwerkelijk die map opent.

---

## Kanalen (DM's) ✅ volledig bevestigd door Rick

De volledige keten — DM versturen, alleen bij de geadresseerde aankomen, nooit bij de
derde peer ook niet via doorsturen — is al bevestigd met drie echte motoren over
loopback-QUIC in volledige mesh (`crates/app/tests/chat_sync.rs`). Wat een tweede en
derde machine daaraan toevoegen — of de knoppen in het echt doen wat ze beloven, en het
geval waarin twee DM-partners elkaar niet rechtstreeks kunnen bereiken — is inmiddels ook
bevestigd, zie K.1 t/m K.6 hieronder.

**K.1 DM komt aan, alleen bij de geadresseerde** ✅ bevestigd
Klik bij je vriend op de DM-knop naast jouw naam en stuur iets. Bij jou moet het
verschijnen zodra je op jouw beurt de DM-knop naast zijn naam opent. Bij de derde peer
mag het **nergens** verschijnen — niet in het algemene kanaal, niet in een DM-venster met
iemand anders. *Dit is de kern van deze uitbreiding: zie je het bericht toch bij de
derde, dan lekt er iets in de kanaal-filtering.*

**K.2 Ongelezen-badge** ✅ bevestigd
Laat je vriend je een DM sturen terwijl je in het algemene kanaal zit. Er moet een apart
getal op zijn DM-knop verschijnen, los van de teller op "# Algemeen". Open de DM → de
badge verdwijnt, het algemene kanaal blijft ongemoeid (en andersom).

**K.3 Bewerken en verwijderen in een DM** ✅ bevestigd
Bewerk en verwijder een eigen DM-bericht. Moet bij de ander bijwerken, precies als in het
algemene kanaal. Bij zijn berichten mag jij die knoppen niet zien.

**K.4 Bestand delen in een DM** ✅ bevestigd
Open een DM-venster en klik "Bestand delen…" daarbinnen. Het bestand moet alleen in dát
DM-venster verschijnen bij de geadresseerde — niet in het algemene bestandenpaneel, en
niet bij de derde peer. Download het bij de geadresseerde: moet identiek aankomen, net
als bij fase 6.

**K.5 Geschiedenis-inhaal geldt ook voor DM's** ✅ bevestigd
Stuur een vriend een DM terwijl hij offline is. Laat hem opstarten: de DM moet er staan
zodra jullie weer verbinden, zonder dat iemand iets doet.

**K.6 DM tussen twee peers die elkaar niet rechtstreeks bereiken** ✅ bevestigd
Lastigste geval, alleen te proberen als je de mesh handmatig kunt opbreken (bijvoorbeeld
met een firewallregel tussen twee van de drie machines): als A en B geen directe
verbinding hebben maar allebei wel met C, dan mag een DM tussen A en B **niet** via C
aankomen — hij moet gewoon wachten tot A en B elkaar weer rechtstreeks kunnen bereiken.
*Dit is de bewuste trade-off uit `docs/ARCHITECTURE.md` (sectie "Kanalen"): een DM
profiteert niet van de doorstuurhulp die het algemene kanaal wel heeft. Komt de DM tóch
via C aan, dan is de kanaal-filtering in het doorstuurpad kapot — dat zou C in staat
stellen de inhoud te lezen, en dat is precies wat dit ontwerp voorkomt.*

---

## Fase 7 — Tags, meldingen, niet storen, gebruikersnaam ✅ volledig bevestigd door Rick

De tag-herkenning zelf (woordgrens, hoofdletterongevoeligheid, waar de cursor precies
staat voor de autocomplete) is met unit-tests gedekt (`crates/app/src/tags.rs`). Wat
alleen met de hand te controleren was — hoe het typen voelt, en of een Windows-melding er
in het echt ook verschijnt — is inmiddels ook bevestigd, zie 7.1 t/m 7.7 hieronder.

**7.1 Autocomplete** ✅ bevestigd
Typ `@` in de chatbox. Er moet een lijstje met peernamen verschijnen dat meefiltert
terwijl je verder typt. Pijltjes omhoog/omlaag verplaatsen de markering, Tab of Enter
vult de gemarkeerde naam in (met een spatie erachter) zonder dat er een tab-teken of een
nieuwe regel achterblijft. Klikken op een suggestie moet hetzelfde doen.

**7.2 Highlight bij een tag naar jezelf** ✅ bevestigd
Laat je vriend `@jouwnaam` in een bericht zetten. Dat bericht moet bij jou opvallen
(gekleurd kader) tussen de rest van de geschiedenis, ook als je later terugscrollt. Een
bericht met `@` gevolgd door iets dat niet op een bestaande naam lijkt (bijvoorbeeld
`@Rickie` als jij `Rick` heet) mag **niet** highlighten.

**7.3 Melding alleen bij een tag, en alleen als het venster verborgen is** ✅ bevestigd
Minimaliseer je venster. Laat je vriend eerst een gewoon bericht sturen (geen tag) — geen
melding. Laat hem daarna `@jouwnaam` sturen — nu wel een Windows-melding met geluid.
Herhaal met het venster **op de voorgrond**: ook met een tag mag er dan geen melding
komen. *Dit dekt precies de twee voorwaarden uit fase 7: verborgen én getagd, niet
"of".*

**7.4 Geen melding voor ingehaalde geschiedenis** ✅ bevestigd
Sluit je app af. Laat je vriend een paar berichten sturen, waaronder eentje met
`@jouwnaam`. Start je app weer op (venster mag gerust verborgen staan of naar de tray
gaan). De ingehaalde berichten moeten gewoon verschijnen — inclusief de highlight op het
getagde bericht — maar er mag **geen** Windows-melding voor komen. *Dit is het
onderscheid tussen live binnenkomen en een inhaalslag; zie `docs/OVERDRACHT.md`.*

**7.5 DM meldt zich ook alleen bij een tag** ✅ bevestigd
Stuur jezelf (via je vriend) een gewoon DM-bericht zonder tag terwijl je venster
verborgen is — geen melding, ondanks dat het een DM is. Stuur daarna een DM mét
`@jouwnaam` — wel een melding. *Bewust zo gekozen, zie beslissing 11 in
`docs/OVERDRACHT.md`.*

**7.6 Niet storen** ✅ bevestigd
Zet "niet storen" aan, minimaliseer, laat je vriend `@jouwnaam` sturen. Geen melding,
geen geluid. Zet niet storen weer uit en herhaal — nu wel. Herstart de app: niet storen
moet weer standaard uit staan.

**7.7 Gebruikersnaam wijzigen** ✅ bevestigd
Open het profielvenster, wijzig je naam, sla op. Bij jezelf verandert je naam meteen
overal waar hij getoond wordt (deelnemerslijst, eigen berichten in de geschiedenis). Bij
je vriend moet de nieuwe naam verschijnen zonder dat hij iets doet. Herstart je app: de
nieuwe naam moet blijven staan (staat nu in `config.toml`).

---

## Fase 9 — Algemeen: subkanalen met een eigen titel

Nieuw deze ronde, nog niet met een echte peer getest. De sync- en opslaglaag is gedekt
door geautomatiseerde tests (`crates/store/tests/convergentie.rs`,
`crates/proto/src/op.rs`, `crates/app/src/ui.rs`) en de `protocol-reviewer`-agent heeft de
protocolwijziging vóór het committen gecontroleerd (zie `docs/OVERDRACHT.md`, beslissing
16). Wat alleen met de hand te controleren is: of de UI voor het aanmaken, hernoemen en
wisselen van een subkanaal in het echt doet wat hij belooft.

**9.1 Subkanaal aanmaken verschijnt bij iedereen**
Klik "+ nieuw kanaal" in de zijbalk onder "# Algemeen", geef het een titel. Bij je vriend
moet het subkanaal vanzelf verschijnen in zijn zijbalk, met dezelfde titel, zonder dat hij
iets doet. *Dit is de kern van fase 9: een subkanaal is net zo publiek als het algemene
kanaal, dus moet zich hetzelfde verspreiden als een chatbericht.*

**9.2 Berichten en bestanden blijven per subkanaal gescheiden**
Stuur een bericht en deel een bestand in het subkanaal. Ze mogen niet verschijnen in "#
Algemeen" of in een ander subkanaal, en andersom: iets uit "# Algemeen" hoort niet in het
subkanaal thuis.

**9.3 Ongelezen-badge per subkanaal**
Laat je vriend iets in het subkanaal sturen terwijl jij naar "# Algemeen" kijkt. Er moet
een apart getal op het subkanaal in de zijbalk verschijnen, los van de teller op "#
Algemeen" en los van een eventuele DM-badge. Open het subkanaal → de badge verdwijnt, de
andere tellers blijven ongemoeid.

**9.4 Hernoemen verschijnt bij iedereen**
Hernoem een bestaand subkanaal (potlood-knopje naast de titel terwijl je erin zit). Bij je
vriend moet de nieuwe titel verschijnen zonder dat hij iets doet, en de geschiedenis van
het subkanaal blijft gewoon staan.

**9.5 Geschiedenis-inhaal geldt ook voor een subkanaal**
Laat je vriend de app afsluiten, stuur intussen iets in een subkanaal, laat hem opstarten:
het bericht moet er staan zodra jullie weer verbinden, zonder dat iemand iets doet — net
als bij het algemene kanaal.

**9.6 Een subkanaal profiteert wél van doorsturen (anders dan een DM)**
Alleen te proberen met drie peers en een handmatig opgebroken mesh (zoals K.6): als A en C
elkaar niet rechtstreeks bereiken maar allebei wel B, dan moet een bericht van A in een
subkanaal bij C aankomen via B — in tegenstelling tot een DM. Komt het niet aan, dan is
`Channel::is_public()` ergens niet toegepast waar `is_general()` eerder stond.

**9.7 Subkanaal verwijderen, met bevestiging**
Klik het prullenbak-knopje naast de titel van een subkanaal waar je in zit. Er moet een
"Weet je zeker?"-vraag verschijnen. Annuleren doet niets. Bevestigen verwijdert het
subkanaal bij jou én bij je vriend, zonder dat hij iets doet, en je springt zelf terug naar
"# Algemeen".

---

## Sidenote: afbeeldingen downloaden zichzelf automatisch

Los van fase 9, op verzoek van Rick tijdens dezelfde ronde: een gedeelde afbeelding hoeft
niet meer aangeklikt te worden om te downloaden — dat gebeurt nu voor iedereen vanzelf,
zowel live als bij het inhalen van gemiste geschiedenis. Andere bestandstypen blijven
gewoon achter de downloadknop staan, zoals nu al het geval is.

**S.1 Afbeelding downloadt zichzelf**
Deel een foto (png/jpg/gif/bmp). Bij je vriend moet de miniatuur vanzelf verschijnen,
zonder dat hij op een downloadknop klikt.

**S.2 Een gewoon bestand downloadt niet vanzelf**
Deel iets dat geen afbeelding is (bijvoorbeeld een zip). Bij je vriend moet de kaart met
downloadknop verschijnen zoals altijd — geen automatische download.

**S.3 Ingehaalde afbeeldingen downloaden ook vanzelf**
Sluit je vriend zijn app, deel intussen een paar foto's, laat hem opstarten: de
miniaturen moeten er vanzelf staan zodra hij weer verbindt, net als bij een live gedeelde
foto.

---

## Fase 10 — Resoluties, bitrate, gecombineerd delen

Nieuw deze ronde, nog niet met een echte peer getest. De bugfix (bureaubladgeluid stopte
niet echt bij een expliciete stop) heeft een regressietest en de resolutie-ondersteuning
bleek al overal parametrisch — daar is niets gebouwd om te breken. Wat overblijft is
precies het soort gedrag dat alleen met echte hardware te controleren is: hoe het beeld
eruitziet op 1440p/ultrawide, of de bitrate-verlaging de audio-lag echt oplost, en of de
nieuwe automatische geluidsdeling zonder eigen-stem-echo werkt.

**10.1 1440p scherp en zonder framedrops**
Deel je 1440p-hoofdscherm. Bij je vriend moet tekst scherp leesbaar zijn, zonder haperingen
in een normaal gesprek. *Faalt dit, dan zit er toch ergens een aanname op 1080p — kijk in
`crates/video/src/codec.rs` en `crates/video/src/kleur.rs`.*

**10.2 Ultrawide (3440×1440) zonder vervorming**
Zelfde als 10.1, maar met een ultrawide-scherm of -venster. Let vooral op de
beeldverhouding in het kijkvenster (moet 21:9 blijven, niet uitgerekt naar 16:9). *Faalt
dit, dan klopt `verhouding` in `crates/video/src/venster.rs` niet, of de
video-instellingen-UI dwingt ergens stilzwijgend 16:9 af.*

**10.3 12 Mbit/s lost de audio-lag op**
Herhaal de oorspronkelijke situatie: jij deelt je scherm op 1080p60, je vriend kijkt mee
terwijl zijn eigen internetverbinding matig is. Zijn kant moet nu **geen** hoorbare
audio-lag meer geven bij jou (de streamer). *Dit is de kern van de bitrate-wijziging — zie
`docs/SPEC.md`, sectie "Bitrate". Blijft de lag, dan zit het probleem niet bij de bitrate
en moet de aanname in die sectie herzien worden.*

**10.4 Scherm delen start geluid automatisch**
Neem deel aan het gesprek, deel daarna een monitor of venster. Zonder op een knop te
klikken moet je vriend nu ook je bureaubladgeluid horen (zet iets af dat geluid maakt).
Zet `FITCOM_LOG=debug` aan: in het logbestand moet "bureaubladgeluid via
proces-exclusieve WASAPI-loopback" verschijnen. *Staat er in plaats daarvan "terugval op
gewone loopback", dan is de exclude-route niet beschikbaar op deze Windows-versie — dat is
op zich geen bug (de terugval is bedoeld), maar dan geldt bij 10.5 het verhoogde risico dat
je je eigen stem terughoort.*

**10.5 Eigen stem komt niet terug via het gedeelde geluid**
Terwijl je scherm en geluid deelt (10.4) en gewoon praat: je vriend mag jouw stem niet
via het gedeelde bureaubladgeluid dubbel/vertraagd terughoren — alleen via de normale
voice-verbinding. *Dit is precies waarom de proces-exclusieve route gebouwd is; hoort hij
je toch dubbel, dan sluit de exclude-modus je eigen proces niet goed uit, of de terugval
naar cpal is geactiveerd (zie 10.4) en dat is dan een bekende beperking, geen nieuwe bug.*

**10.6 Stoppen met scherm delen stopt het geluid ook echt**
Stop met delen (laatste scherm/venster weg) terwijl je geluid deelde. Bij je vriend moet
het geluid meteen stil zijn — geen resterend geluid, geen "stoppen"-knop meer nodig (die
is er niet meer). *Dit dekt zowel de losstaande bugfix als de nieuwe auto-koppeling.*

**10.7 Eén scherm stoppen terwijl er nog een tweede gedeeld wordt**
Deel twee bronnen (bijv. twee vensters) en stop er één. Het geluid moet **doorgaan** —
pas als ook de laatste bron stopt, stopt het geluid. *Faalt dit, dan telt
`deelt_scherm_of_venster()` verkeerd.*

---

## Fase 11 — Automatische updates tussen peers

Nieuw deze ronde, nog niet met een echt versieverschil getest. `run-peers.ps1` start overal
dezelfde build, dus dit kan niet met de gebruikelijke lokale opzet — je hebt hiervoor een
**tweede, oudere build** nodig (bijvoorbeeld: bouw een keer, kopieer `target\debug\fitcom.exe`
en `fitcom-updater.exe` naar een aparte map als "oude versie", verhoog daarna
`workspace.package.version` in `Cargo.toml` met een patch-nummer, bouw opnieuw, en start de
oude exe als de "peer met de oudere versie"). Zet `FITCOM_LOG=debug` aan op beide kanten.

**11.1 Een nieuwere versie wordt automatisch aangeboden**
Start de oude build en de nieuwe build als twee peers die met elkaar verbinden. Bij de oude
peer moet vanzelf — zonder klikken — een venster "Nieuwere versie beschikbaar" verschijnen
zodra hij verbindt. *Verschijnt er niets, controleer in het log van de oude peer of
`Hello`/`HelloAck` een `app_version` meekreeg en of die hoger is dan de eigen versie.*

**11.2 De download komt binnen en wordt geverifieerd**
Laat 11.1 doorlopen: de voortgangsbalk moet oplopen tot 100% en het venster moet vanzelf
overgaan naar een "Nu bijwerken en herstarten"-knop. *Blijft hij hangen op "bezig", dan is
de uni-stream niet aangekomen of niet als update herkend — kijk naar `read_kind` in
`crates/net/src/filestream.rs`. Faalt de hash-verificatie, dan staat dat expliciet in het
log ("update is corrupt geraakt").*

**11.3 Bevestigen werkt: de oude peer wordt de nieuwe versie**
Klik "Nu bijwerken en herstarten" op de oude peer. De app moet zichzelf afsluiten, en na
hooguit een paar seconden moet er een nieuw venster verschijnen dat nu de nieuwe versie
draait (zie de titelbalk-log-regel "FitCommunication start versie=..."). *Gebeurt er niets,
kijk in `updater.log` naast de exe — die zegt exact waar het is blijven steken (wachten op
het oude proces, hernoemen, of opnieuw starten).*

**11.4 Onderbreken en hervatten**
Herhaal 11.1, maar sluit de oude peer af (of verbreek het netwerk) terwijl de download nog
bezig is. Start hem daarna opnieuw: de download moet hervatten vanaf waar hij was, niet
opnieuw vanaf 0. *Kijk naar de grootte van `updates\update-<versie>.exe.part` in de datamap
vlak vóór het herstarten.*

**11.5 Negeren houdt op met vragen, deze sessie**
Klik "Negeren" op het aanbod. Het venster moet verdwijnen en niet vanzelf terugkomen zolang
de peer verbonden blijft. Herstart de oude peer: het aanbod mag dan wél weer verschijnen
(negeren is sessie-lokaal, geen instelling).

---

## Periodieke microhapering bij screenshare (2026-08-02)

> Heette hier eerder "fase 12". Dat botste met de echte fase 12 uit `ROADMAP.md`
> (de UI-stack), die hieronder staat. De testnummers 12.1 t/m 12.6 zijn ongewijzigd.

Alles hieronder is op één machine gemeten en gerepareerd (zie `docs/OVERDRACHT.md`,
"Onderzoek 2026-08-02"). Wat daar niet te controleren viel is of het verlies op een écht
internetpad dezelfde vorm heeft — daar zit ook verlies van twee of meer fragmenten tegelijk
in, en dat repareert de pariteit niet.

**Vóóraf, en dit is geen optie:** `PROTOCOL_VERSION` ging van 4 naar 5. **Alle drie de
peers moeten dezelfde build draaien.** Een peer op de oude versie wordt bij de handshake
geweigerd vóórdat er een sessie draait, dus de automatische update van fase 11 kan hier
niet overheen helpen. Zie 12.5.

**12.1 De hapering zelf is weg**
Deel een venster met een filmpje erin, laat het minstens een paar minuten lopen en kijk aan
de andere kant. Het gaat om precies één ding: die korte stotter om de vijf à zes seconden.
*Als hij er nog is: hoe vaak, en gebeurt het aan beide kanten?*

**12.2 De meter aan de kijkkant vertelt of het werkt**
Zet `FITCOM_LOG=info` (standaard) en kijk in `data\logs\` naar de `kijker`-regels. Wat je
wilt zien: `hersteld` af en toe boven nul, en `incompleet` en `keyframe_verzoeken` op nul.
*Loopt `incompleet` op terwijl `hersteld` nul blijft, dan raakt er meer dan één fragment
per beeld zoek en helpt de pariteit niet — stuur die regels door.*

**12.3 Het beeld is niet trager geworden**
Het pariteitsfragment en de spreiding kosten allebei iets. Het moet niet merkbaar later
zijn dan voorheen als je met de muis ergens naar wijst op een gedeeld scherm. *Voelt het
traag: dat is `WEERGAVE_VOORSPRONG` in `kijker.rs`, nu 30 ms.*

**12.4 Aanhaken duurt niet langer**
`GOP_SECONDEN` staat van 2 op 10. Een nieuwe kijker vraagt zelf om een keyframe, dus het
venster hoort net zo snel beeld te tonen als voorheen. *Blijft het venster seconden zwart
bij het openen, dan komt dat verzoek niet aan.*

**12.5 Een peer op de oude versie is als zodanig herkenbaar**
Laat één peer bewust de oude build draaien. In het deelnemerspaneel hoort daar nu
"andere versie (protocol 4, wij 5)" te staan en niet gewoon "offline". *Staat er offline,
dan ga je het netwerk zitten uitzoeken terwijl er niets mis is met het netwerk.*

**12.6 Naast een draaiende game**
`MediaSocket::bind` zet de timerresolutie van dit proces op 1 ms zodra er beeld of geluid
loopt. Sinds Windows 10 2004 werkt dat per proces, dus een game hoort er niets van te
merken — maar dat is een aanname uit documentatie, geen meting. *Deel je scherm terwijl je
gamet en kijk of de frametimes van de game er anders uitzien dan voorheen.*

---

## Fase 12 — UI-stack naar Tauri v2

De weergavelaag is vervangen; functioneel hoort er niets veranderd te zijn. Wat hieronder
staat kan alleen met echte hardware of een tweede machine, want daar zit precies het deel
dat de migratie een nieuw transport of een nieuw invoerpad gaf.

Wat wél al met de hand gecontroleerd is op de dev-PC (geen actie nodig): het venster start
en toont chat, kanalen, DM's, het lege kanaal, het deelnemerspaneel, de statusbalk en alle
vijf de instellingen-tabbladen; de tijdlijn toont berichten, codeblokken, links, tags en
bestandskaarten; en de app verbruikt in rust 0,05% van één kern.

**U.1 De miniaturenstrook toont echt beeld**
Laat iemand een scherm delen en kijk mee. Boven de chat hoort een strook met een tegel per
bekeken stream te verschijnen, met **live beeld** dat ongeveer twee keer per seconde
bijwerkt, een LIVE-markering en de naam eronder. *Blijft de tegel het grijze monitor-icoon
tonen, dan komt er wel een stream binnen maar geen miniatuur — dat is het nieuwe
`thumbnail`-event of het `thumb://`-protocol, niet de stream zelf.*

**U.2 Ctrl+V met een echte afbeelding**
Maak een schermafdruk (PrtSc of Win+Shift+S), klik in het invoerveld en druk Ctrl+V. Er
hoort meteen een bestandsaanbod te verschijnen en bij de ander een inline afbeelding.
*Dit is het pad dat in egui via `GetAsyncKeyState` moest — beslissing 15 — en nu een echt
`paste`-event is. Werkt het niet, dan is dat een regressie op precies dat punt.*

**U.3 Slepen en neerzetten vanuit Verkenner**
Sleep een bestand het venster in. Terwijl je erboven hangt hoort er een overlay te
verschijnen ("Drop to share"), en na loslaten hetzelfde aanbod als via de knop. *De overlay
en het neerzetten zijn twee aparte Tauri-events; als de overlay wel komt en het aanbod niet
(of andersom), zeg welke van de twee.*

**U.4 Spreekindicatie tijdens een gesprek**
Zit met z'n tweeën of drieën in het gesprek en let op de groene ring om de avatar in het
spraakpaneel én in de ledenlijst, en op de regel "Speaking"/"Listening". Die moeten
tegelijk aan en uit gaan. *In de comp werd dit door twee mechanismen aangestuurd; dat is
in de port één mechanisme geworden, dus als de ene plek wel meebeweegt en de andere niet is
dat een echte bug.*

**U.5 RTT in de statusbalk beweegt en springt niet**
Kijk onderin naar de milliseconden per peer. Die horen rustig bij te werken (viermaal per
seconde) zonder dat de rest van het venster knippert of verspringt. *De hele reden dat RTT
in een apart event zit is dat het venster er niet van hoeft te hertekenen.*

**U.6 Een scherm kiezen om te delen**
Klik "Share screen" in het spraakpaneel. Er hoort een lijst te komen met je monitoren én
je open vensters, en na kiezen begint het delen. *Deze keuzelijst stond niet in de comp en
is nieuw gebouwd; let vooral op of de lijst klopt met wat er op dat moment open staat.*

**U.7 Bureaubladgeluid en volume per stream**
Deel een scherm met geluid. Bij de luisteraar hoort onder de peer een aparte, gelabelde
schuif "Screen audio" te staan, los van zijn stemvolume. *Beide schuiven moeten
onafhankelijk werken; één schuif die allebei stuurt is de bug om op te letten.*

**U.8 Sluiten naar de tray, en terug**
Klik het kruisje. Het venster hoort te verdwijnen en de app moet blijven synchroniseren en
melden. Dubbelklik het tray-icoon om hem terug te halen, en probeer ook "Afsluiten" in het
tray-menu. *Dit ging door een compleet nieuwe vensterlus heen; dat het icoon het venster
nog terugvindt is niet vanzelfsprekend.*

**U.9 Een melding tijdens het gamen**
Zet het venster naar de tray, laat iemand `@jouwnaam` sturen en kijk of de Windows-melding
komt. Zet daarna niet-storen aan en laat het opnieuw doen: dan hoort er niets te komen.
*De motor beslist dit, niet de UI — maar de UI vertelt de motor of het venster de voorgrond
heeft, en dat is nieuwe code.*

**U.10 Een update bevestigen**
Alleen te doen met een tweede, oudere build (zie fase 11). De chip rechtsonder hoort te
verschijnen en een bevestigingsvenster te openen vóór er iets gebeurt. *Nooit stilzwijgend
toepassen.*

---

## macOS-port (2026-08-05) — wat er met een echte Windows-peer getest moet worden

Op de mac zelf is al bevestigd (zie `docs/OVERDRACHT.md` beslissing 21): bouwen, alle
tests, mesh van twee lokale instanties (QUIC, protocol 5), SCK-opname op
Retina-resolutie, en de hele videoketen op loopback (5,1 ms opnemen→tonen, 231/233
beelden). Wat alleen met een echte Windows-peer over Tailscale kan:

**M.1 Chat mac↔Windows**
Berichten beide kanten op, bewerken/verwijderen convergeert, geschiedenis-inhaal na
offline zijn, bestand heen en terug, geplakte afbeelding inline aan beide kanten.

**M.2 Voice mac↔Windows**
Duplex-audio, VAD (stilte verstuurt niets), mute/deafen, volume per peer, RTT in de
meterregel. Eerste keer: macOS vraagt om de microfoon. *Klinkt de mac-kant robotisch
of te snel/langzaam, kijk dan naar de herbemonstering (log meldt de apparaat-rate).*

**M.3 Windows deelt, mac kijkt**
Stream verschijnt in de strook (miniatuur!), kijkvenster opent, beeld loopt vloeiend
(meterregel: `getoond_fps` ≈ bron, `spreiding_ms` laag), venster sluiten stopt het
abonnement (deler-meter: kijkers → 0), keyframe-herstel na verlies (even de wifi/kabel
knijpen).

**M.4 Mac deelt, Windows kijkt**
Bronkiezer toont schermen én vensters (eerste keer: Screen-Recording-permissie +
herstart), Windows-decoder pakt de stream (Annex-B/SPS-PPS-brug — dít is de
wire-kritieke test), 60 fps-pacing op de ProMotion-mac (120 Hz-bron → 60 op de draad),
laat-instappende tweede kijker krijgt binnen een halve seconde beeld
(keyframe-verzoek).

**M.5 Desktopgeluid vanaf de mac**
Mac deelt scherm + zit in voice → bureaubladgeluid-stream verschijnt vanzelf; de
Windows-peer hoort het mac-systeemgeluid maar **niet zijn eigen stem terug**
(`excludesCurrentProcessAudio` — speel muziek af terwijl de Windows-peer praat).
Stilte stopt de pakketten (2 s hangover).

**M.6 Update-gedrag met versieverschil**
Bouw de mac één patchversie hoger dan de Windows-peer. De Windows-peer mag *geen*
update-banner tonen of iets binnenhalen (de mac antwoordt NOT_AVAILABLE); in de
Windows-log hoort de aanvraag als mislukt/niet-beschikbaar te eindigen, waarna alles
gewoon doorwerkt.

**M.7 Tray en meldingen op de mac**
Sluitknop → venster weg, app blijft (menubalk-icoon), Openen haalt hem terug met
focus, Afsluiten sluit netjes af (peer ziet offline binnen ~15 s). Bericht terwijl de
app verborgen is → macOS-melding met geluid; niet-storen onderdrukt hem.

**M.8 Slapen en ontwaken**
Mac dichtklappen tijdens een gesprek → peers zien offline; openklappen → binnen ~30 s
vanzelf weer online, chat haalt in, stream-abonnementen zijn netjes opgeruimd.

## Camera (2026-08-06) — nog op geen enkele echte camera getest

Op de mac is bevestigd: bouwen, alle tests (inclusief de nieuwe streams-test), clippy
schoon voor **beide** targets, en de app start met de camera-knop erin. Wat niet te
bevestigen was: er is geen camera-opname op macOS, dus **alles hieronder moet op de
Windows-machine**. `crates/video/src/camera.rs` is nagelopen met een typecheck tegen de
echte `windows`-crate (zie `docs/OVERDRACHT.md` beslissing 22) — dat vangt API-fouten,
geen gedrag.

**C.1 Camera aanzetten**
Knop naast mute/deafen (camera-icoon). Klik → knop licht op in de accentkleur, en er komt
een regel "Camera on · <naam>" bij. **Het lampje van de camera hoort nog uit te zijn**:
er wordt niets opgenomen tot iemand kijkt. Geen camera aangesloten → foutbalk "geen camera
gevonden", geen crash.

**C.2 Peer kijkt naar de camera**
De peer ziet "Watch <cameranaam>" met een camera-icoontje. Kijken → venster opent, beeld
loopt, miniatuur in de strook. **Let op de oriëntatie: staat het beeld op zijn kop, dan is
de stride-afhandeling fout** (zoek in de log naar "camera levert onderstboven" /
"bovenaf"). Ook checken: het venster begint op 1280×720 (de nominale maat) en springt naar
de echte resolutie van de camera zodra het eerste beeld binnen is.

**C.3 Camera én scherm tegelijk**
Camera aan + scherm delen. Twee losse streams, twee vensters bij de kijker, beide vloeiend
(meterregels per stream vergelijken). "Stop sharing" stopt alleen het scherm en laat de
camera aan; de camera-knop stopt alleen de camera.

**C.4 Bureaubladgeluid hoort níét bij de camera**
Alleen de camera aan, in het gesprek → er komt **geen** bureaubladgeluid-stream. Scherm
erbij → die komt er wel. Scherm weer weg (camera blijft aan) → geluid gaat weer uit. Dit
is de regel die de nieuwe unittest afdekt; dit is de controle dat hij in het echt ook zo
uitpakt.

**C.5 Camera bezet of eruit getrokken**
Start Teams/Zoom met de camera en zet hem dan hier aan → nette foutmelding, geen hang.
USB-camera eruit trekken terwijl iemand kijkt → log meldt einde van de stroom, de app
blijft leven, aankondiging intrekken werkt nog.

**C.6 Mac kijkt naar een Windows-camera**
Op de mac hoort de camera-knop een foutbalk te geven (geen camera-opname op macOS), maar
**kijken naar de camera van de Windows-peer moet gewoon werken** — venster, miniatuur,
alles. Dit is de test dat het overslaan van de mac-kant niets anders gebroken heeft.

## Camera, geluidjes en bijwerken (2026-08-10)

Alle vier de wijzigingen van deze ronde zijn **op een Mac geschreven zonder Windows in de
buurt**. De Windows-only bestanden zijn wel getypecheckt voor `x86_64-pc-windows-msvc` (de
scratch-crate-omweg uit beslissing 22), en dat vangt elke API-vormfout — geen enkel gedrag.
Dit zijn de gevallen die alleen op de echte machine iets bewijzen.

**C.7 De camera uitzetten crasht niet meer** — de klacht die dit moest oplossen.
Camera aan, camera uit. Dan met een kijker erbij: camera aan, iemand laten kijken, camera
uit terwijl hij kijkt. Beide kanten moeten blijven leven. Daarna vijf keer snel aan/uit
achter elkaar: geen crash, geen "in gebruik door iets anders", en het lampje moet echt
uitgaan. In de log horen `camera-opname gestopt` en `delen gestopt` bij elkaar te staan.

**C.8 Camera uit en meteen weer aan**
Uit en binnen een halve seconde weer aan. Dit is het geval dat vóór deze ronde niet kón
werken (de leesthread hield het apparaat nog vast) en waar nu maximaal een halve seconde op
gewacht wordt. Werkt hij niet, kijk dan in de log naar `deel-thread stopte niet binnen`.

**C.9 Je eigen camerabeeld**
Camera aan → er komt een venster "You — <cameranaam>" met je eigen beeld, ook als er
niemand kijkt. Controleren: staat het niet op zijn kop en niet in spiegelbeeld ten opzichte
van wat de kijker ziet? Camera uit → venster sluit. F11 en dubbelklik doen beeldvullend,
Escape eruit.
Daarna: het voorbeeldvenster met de sluitknop dichtdoen terwijl er niemand kijkt → de
opname hoort te stoppen (lampje uit) **en de camera-knop hoort terug op uit te springen**,
binnen een tik. En met een kijker erbij: venster dicht → de kijker blijft gewoon beeld
krijgen, en de camera gaat pas uit als ook die laatste kijker weg is.

**C.9b Camera aanzetten terwijl hij bezet is**
Teams of Zoom met de camera open laten staan en dan hier de camera aanzetten. Verwacht: een
leesbare foutbalk ("camera: …") en de knop die terugspringt naar uit — niet een knop die
"aan" blijft staan boven iets dat niet draait. Dit is het geval dat er pas kán zijn sinds de
opname bij het aanzetten begint in plaats van bij de eerste kijker.

**C.10 Gedeeld scherm is niets veranderd**
Scherm delen, iemand laat kijken, laatste kijker weg, opnieuw kijken. Dit is de controle dat
de deler-wijzigingen (geen encoder zonder kijkers, wachten bij het stoppen) alleen de camera
raken: een scherm hoort zich exact als voorheen te gedragen, inclusief bureaubladgeluid dat
mee aan- en uitgaat.

**G.1 Geluidjes**
Zes gevallen, en ze horen alle zes verschillend te klinken: zelf deelnemen (oplopend), zelf
verlaten (aflopend), de ander komt erbij, de ander gaat eruit, de ander zet een scherm of
camera aan, en weer uit. Let vooral op wat er **niet** hoort te klinken: bij een peer die
opnieuw verbindt terwijl hij al deelt of al in het gesprek zit hoort het stil te blijven
(dat is de her-aankondiging), en met niet-storen aan hoort er niets te komen. Te hard? De
app staat in de volumemixer van Windows.

**U.1 Bijwerken vanaf GitHub, met een echt versieverschil**
Nog nooit gelukt (beslissing 24), dus dit is de eerste keer dat dit hele pad in het echt
loopt. Volg "Een release uitgeven" in `docs/OVERDRACHT.md` — inclusief stap 6, en pas
verder als die HTTP 200 zegt. Daarna op een machine met een óudere versie: Settings →
Account → "Check for updates". Verwacht: "Fetching …" met een percentage dat oploopt, dan de
chip in de statusbalk, dan "Update and restart" → app sluit, komt terug op de nieuwe versie,
geschiedenis en instellingen intact. In `data\logs\` en in `updater.log` naast de exe staat
wat er gebeurde.

**U.2 Bijwerken als er niets nieuws is, en als de feed stuk is**
Zelfde knop op de nieuwste versie → "You are on the newest version." Daarna met Tailscale
en internet uit → de knop hoort de echte fout te melden, niet stil te blijven. Dat
onderscheid is de reden dat die knop er is.

## Geluidsets en hun volume (1.0.1)

De tonen zijn op een Mac ontworpen en daar alleen *nagemeten*, niet beoordeeld. Wat er
gemeten is, per geluidje: duur, piek, luidheid, of het op stil begint en eindigt, of de
grootste sprong tussen twee samples eruit springt ten opzichte van de rest (dát is wat een
tik hoorbaar maakt, niet de sprong op zich — een partiaal op 5 kHz geeft legitiem grote
sprongen), en of een stijgend motief werkelijk stijgt: niet "welke toon is het luidst in de
eerste helft", want bij overlappende klanken klinken beide toonhoogtes de hele tijd door,
maar of de verhouding tussen de twee toonhoogtes de goede kant op schuift. Alle 24 kwamen
schoon door. Of ze prettig klínken kan alleen een mens.

**S.1 Alle sets, alle zes**
Instellingen → Audio → Notification sounds. Per set alle zes proefknoppen langs. Waar het om
gaat: (a) klinkt het prettig en niet als een foutmelding, (b) hoor je zonder nadenken welke
van de zes het is, (c) klinkt "erbij" hoorbaar anders dan "eraf", (d) is je eigen deelname te
onderscheiden van die van iemand anders, (e) zit er geen tik, klik of ratel in.

**S.2 Van set wisselen verandert het volume niet**
Kies een set → hij speelt zichzelf één keer. Ga langs alle vier en let op of de ene set
merkbaar harder is dan de andere. Dat hoort niet: elk geluidje wordt genormaliseerd op zijn
*luidheid* (hoogste RMS over 200 ms, ongeveer de integratietijd van het oor) en niet op zijn
piek. Op de piek genormaliseerd was dit verschil 5 tot 9 dB — een uitdovende klank is bij
gelijke piek veel zachter dan een staande toon — en dat is precies waarom het nu op luidheid
gaat. Klinkt er één set toch duidelijk harder of zachter, dan is de meting niet
representatief voor wat jij hoort; meld welke set en welke kant op.

Binnen een set hoort er wel verschil te zijn, en dat is bedoeld: wat een ander doet is
2,3 dB zachter dan je eigen deelname, en een stream 3,4 dB.

**S.3 Volume, en dat het bewaard blijft**
De schuif naar 20%, 60% en 100% en bij elk een proefknop. Dan de app afsluiten en opnieuw
starten: de gekozen set én het volume horen te staan waar je ze liet. In
`data\config.toml` staat dan `[sound]` met `set` en `volume`.
Schuif op 0 → er hoort niets te komen, ook niet zachtjes.

**S.4 Het volume raakt alleen deze tonen**
Met het volume op 20%: je eigen stem, de stem van de anderen en bureaubladgeluid horen
onveranderd hard te blijven. En de Windows-melding bij een bericht dat jou tagt is Windows'
eigen geluid — die verandert hier bewust niet mee.

**S.5 In het echt, met een tweede machine**
De vier echte gebeurtenissen langs: iemand komt in het gesprek, gaat eruit, zet een scherm
of camera aan, en weer uit. Let op wat er níét hoort te klinken: bij een peer die opnieuw
verbindt terwijl hij al deelt of al in het gesprek zit hoort het stil te blijven, en met
niet-storen aan hoort er niets te komen behalve de proefknoppen.

**S.6 Een config van 1.0.0 blijft werken**
Start 1.0.1 met de `config.toml` van 1.0.0 (zonder `[sound]`-tabel). Hij hoort gewoon te
starten, op de klassieke set en 70% volume, en pas iets weg te schrijven als je iets wijzigt.
Dit is de test die de kanalen-uitbreiding indertijd niet had; er is nu ook een unittest voor
(`config.rs::config_van_voor_de_geluidsinstellingen_krijgt_de_standaardset`).

## Bestanden openen en YouTube-previews (2026-08-20)

De motorkant is met de hand niet meer te breken dan de tests al doen: `file_deling.rs`
loopt een echte overdracht door en controleert dat het pad van de download én van een eigen
aanbod in de momentopname staat en een herstart overleeft, en `youtube.rs` heeft een
`#[ignore]`-test die echt met YouTube praat. Wat er overblijft, is wat alleen een mens kan
zien: dat het openen op jouw pc met jouw programma's ook echt gebeurt.

**O.1 Een gedownload bestand openen**
Laat de ander een pdf of een zip aanbieden, download hem, en klik dan **Open** op dezelfde
kaart in de chat. Hij hoort in het programma te openen dat Windows er standaard voor
gebruikt. De kaart blijft staan waar hij stond.

**O.2 Je eigen aanbod openen**
Bied zelf een bestand aan en klik **Open** op je eigen kaart. Dat opent het *originele*
bestand op de plek waar je het vandaan sleepte, niet een kopie.

**O.3 Na een herstart werkt de knop nog**
Sluit de app helemaal af (ook uit de tray) en start hem opnieuw. Bij de kaarten uit O.1 en
O.2 hoort **Open** er nog te staan. Verplaats daarna het gedownloade bestand naar een andere
map en herstart nog eens: dan hoort de knop weg te zijn in plaats van niets te doen.
Onderdeel van dezelfde zaak: laat de ander na jouw herstart een bestand ophalen dat jij vóór
die herstart aanbood — dat werkte niet en hoort nu wel te werken.

**O.4 Een uitvoerbaar bestand krijgt de map**
Bied een `.exe` aan (bijvoorbeeld de installer van iets) en klik de knop bij de ander. De
knop hoort **Show** te heten en Verkenner te openen met het bestand geselecteerd — hij mag
het bestand *niet* starten. Ditzelfde geldt voor `.bat`, `.ps1`, `.msi` en `.lnk`.

**O.5 Een afbeelding op ware grootte**
Klik een afbeelding in de chat aan. Er hoort een venster open te gaan met de afbeelding
passend in het scherm, met de bestandsnaam in de titelbalk. **Actual size** zet hem op één
beeldpunt per schermpunt en het kader gaat scrollen — controleer dat je de linkerbovenhoek
en de rechteronderhoek allebei kunt bereiken. Klikken op de afbeelding wisselt tussen die
twee. **Open** opent hem in je gewone afbeeldingsprogramma. Esc, **Close** en een klik
naast de afbeelding sluiten hem allemaal.
Doe dit één keer met een 1440p-schermafdruk: de kaart in de chat hoort de hele afbeelding
verkleind te tonen (dat was afgekapt op 420 px) en het venster de echte maat.

**O.6 Een YouTube-link**
Stuur `https://www.youtube.com/watch?v=dQw4w9WgXcQ`. Onder het bericht hoort binnen een
seconde een kaart te komen met de miniatuur, de titel en de kanaalnaam; de link zelf blijft
gewoon staan. Klikken op de kaart opent de video in je browser, niet in de app.
Probeer ook `youtu.be/...`, een `shorts/`-link en een link met `&t=42` erachter.

**O.7 De preview komt maar één keer van internet**
Wissel van kanaal en terug, of herstart de app: de kaart hoort er meteen te staan zonder te
knipperen. In `data\youtube\` staan dan een `.json` en een `.jpg` per video. Gooi die map
weg → bij het volgende bericht wordt hij opnieuw gevuld.

**O.8 Zonder internet blijft het een gewone link**
Zet Wi-Fi/ethernet uit (Tailscale mag weg, de peers hoeven niet te werken voor deze test) en
stuur een YouTube-link. Er hoort **geen** foutmelding en geen leeg kader te komen — alleen
de link. Met `FITCOM_LOG=debug` staat er één regel `no youtube preview` in het log.

**O.9 Een link die geen video is**
`https://www.youtube.com/playlist?list=...` en een gewone link naar een andere site horen
géén kaart te krijgen.

## Wordle van de dag (2026-08-20)

De regels zelf zitten in tests: het kleuren van dubbele letters, de dagovergang om 07:00,
de puntenregel, de plek van de kaart in de tijdlijn, en één `#[ignore]`-test die echt met
NYT praat (`cargo test -p fitcom --lib wordle -- --ignored --nocapture`). Wat overblijft is
wat twee echte machines en een echte ochtend moeten uitwijzen.

**W.1 De kaart staat er, met het echte woord**
Start de app na 07:00. Onderaan `#general` hoort een kaart **WORDLE `<nummer>`** te staan met
**Play today's puzzle**. Controleer het nummer en het woord tegen het echte Wordle van die
dag (`<data>\wordle.json` bevat de oplossing — pas op, dat verklapt hem). In het log staat
één regel `wordle van vandaag binnen`.

**W.2 Spelen**
Typ een woord van vijf letters en druk Enter. De rij hoort meteen te kleuren: groen op de
plek, amber erin-maar-niet-daar, grijs niet erin. Het toetsenbord onder het bord hoort
dezelfde kleuren te krijgen. Probeer ook: vier letters (Enter doet niets), en een niet-woord
zoals `qwrtz` — dan komt er **Not in the word list.** en blijft wat je typte staan, zodat je
het kunt herstellen.

**W.3 Halverwege afsluiten**
Sluit de app helemaal af (ook uit de tray) terwijl je op poging 3 zit. Na het opnieuw starten
horen die drie rijen er nog te staan en zegt de kaart **Continue · 3/6**.

**W.4 De uitslag komt bij de ander aan**
Speel de dag uit terwijl de ander online is. Bij hem hoort binnen enkele seconden je uitslag
op de kaart van vandaag te verschijnen (naam + `4/6`), en jouw regel in het scorebord één
punt te veranderen zodra jullie er beiden op staan. Doe dit ook een keer met de ander
**offline**: zet hem daarna aan, en de uitslag hoort via de gewone inhaalslag alsnog binnen
te komen zonder dat iemand iets doet.

**W.5 Alleen spelen levert niets op**
Speel een dag uit die de anderen niet spelen. De kaart hoort te zeggen dat je alleen speelde
en dat de dag pas meetelt als iemand anders meedoet; je punten mogen niet omhoog.

**W.6 De vierkantjes van de anderen blijven verborgen**
Laat de ander eerst spelen en kijk dan naar de kaart van vandaag vóórdat je zelf klaar bent:
je hoort zijn *aantal* pogingen te zien, maar geen vierkantjes. Zodra je eigen spel klaar is
horen ze te verschijnen — bij hem en bij jou.

**W.7 Zonder internet**
Gooi `<data>\wordle.json` weg, zet Wi-Fi/ethernet uit en start de app. Er hoort **geen**
kaart en **geen** foutmelding te komen; met `FITCOM_LOG=debug` staat er één regel
`geen wordle van vandaag`. Zet het internet weer aan: binnen een kwartier hoort de kaart er
vanzelf te staan zonder herstart.

**W.8 Een dag die je gemist hebt**
Laat de app een dag uit staan terwijl de anderen spelen. Bij het opstarten hoort er een
kaart voor die dag in de tijdlijn te komen met hun uitslagen erop, zonder knop — je kunt hem
niet meer naspelen.

**W.9 De dag wisselt om 07:00 en niet om middernacht**
Speel een keer laat op de avond en kijk om 00:30 nog eens: de kaart van "vandaag" hoort nog
steeds die van gisteravond te zijn, en een dag die je om 00:30 afmaakt hoort op de dag van
gisteren geboekt te worden (kijk in `wordle.json`, en bij de ander in het scorebord).

**W.10 Een oudere build in de mesh**
Alleen te doen als er nog een instantie van vóór deze versie is: die hoort de
`WordleResult`-ops op te slaan en door te sturen zonder ze te begrijpen (er verschijnt daar
niets), en de derde peer hoort ze via hem alsnog te krijgen. Dit is de invariant "protocol
alleen additief" en er is geen protocolbump geweest.

## Wat je terugkoppelt

Per geval genoeg aan: **nummer + werkt / werkt niet + wat je zag**. Bij audio- of
beeldproblemen is de log van beide kanten waardevol; die staat in `data\logs\`.

## Clips (2026-08-22, fase 15)

Geautomatiseerd gedekt (`cargo test -p fitcom-video -- --ignored`, GPU + scherm nodig):
de hele keten tot afspeelbaar MP4, een herstart die met een schone ring begint, geluid dat
het beeld dekt ook als de opname pas ná het opstarten aangaat, AAC-frames met stijgende
tijden, en de zuivere regels (avcc-conversie, vensterkeuze, ring-retentie, ring legen).

Met de hand, want UI en tweede machine:

1. Instellingen → Clips: aanzetten. Statusbalk blijft rustig; taakbeheer toont een
   extra GPU-process. Ring groeit in `<data>/clips/ring/` tot ~venster+marge en stopt
   met groeien.
2. Tijdens gamen Ctrl+Alt+C drukken (venster mag in de tray): binnen enkele seconden
   verschijnt `clip-<tijdstempel>.mp4` in `<data>/clips/` en speelt hij buiten de app
   af — beeld én systeemgeluid, lip-sync over de hele minuut.
3. Clip maken vlak nadat de recorder aanging: nette foutmelding, geen crash, geen leeg
   bestand.
4. Uitzetten: geluidsdraad stopt. Weer aanzetten (of de app herstarten) = de ring wordt
   leeggeveegd en opnieuw opgebouwd; een clip vlak daarna is korter dan het venster en
   bevat nooit beelden van vóór het aanzetten. Zie OVERDRACHT beslissing 33 — dit punt
   stond hier eerst andersom en dát was de bug.
5. Frametime-vergelijking spel met recorder aan/uit — de meetpunt-notitie in SPEC.md.
