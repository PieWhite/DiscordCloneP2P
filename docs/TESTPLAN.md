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
tussentijdse `.part`-bestand groter is dan 0 bytes vlak na de onderbreking — voor een
gewoon bestand staat dat in de downloadmap, voor een afbeelding in `Pictures` daarbinnen
(beslissing 32).*

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

**S.4 Een downloadmap op een andere schijf** — te doen na beslissing 32
Dit is de melding zelf: bij een downloadmap buiten `%APPDATA%` (het duidelijkst op een
*andere schijf*, bijv. `D:\Downloads`) mislukte élke afbeelding, met een rode tekst over
hernoemen op de kaart en geen miniatuur.

1. Zet in Settings → Files de downloadmap op een pad op een andere schijf.
2. Laat de ander een screenshot plakken (Ctrl+V in de chat).
3. De miniatuur hoort gewoon te verschijnen, en het bestand hoort te staan in
   `<die downloadmap>\Pictures\<64 hex-tekens>.png` — niet in `%APPDATA%`, en er hoort
   geen `.part` achter te blijven.
4. Kijk ook of de afbeeldingen van *vóór* de wijziging er nog staan: bij de eerste start na
   deze versie verhuizen ze van `%APPDATA%\FitCommunication\Pictures` naar de nieuwe map,
   en de oude kaarten in de tijdlijn moeten hun miniatuur houden. Het logbestand meldt
   `afbeeldingen verhuisd naar de nieuwe afbeeldingenmap` met een aantal.
5. Verzet de downloadmap daarna nóg een keer, met afbeeldingen in de tijdlijn. Ze moeten
   meeverhuizen (miniaturen blijven staan) en een oude afbeelding moet nog te downloaden
   zijn door de ánder — dat laatste bewijst dat de onthouden paden meegeschoven zijn.

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

> *Sinds 2026-10-04 is de weergave GPUI. De gevallen hieronder gingen over de Tauri-build en
> noemen Tauri-specifieke oorzaken (`thumbnail`-event, `thumb://`, Tauri-events). Voor de
> huidige build: "GPUI-weergave (2026-10-04)" verderop.*

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

**C.9 Je eigen camerabeeld (herzien 2026-08-25, beslissing 34)**
Camera aan → er komt **geen** tweede venster. In de streamstrook boven de tijdlijn staat
binnen een halve seconde een tegel "You · <cameranaam>" met je eigen beeld erin, ook als er
niemand kijkt. Controleren: staat het niet op zijn kop en niet in spiegelbeeld ten opzichte
van wat de kijker ziet? Camera uit → tegel weg.
Daarna met een kijker erbij: die kijker weg laten gaan → jouw tegel blijft beeld houden
(het LIVE-label hoort eraf te gaan, want er kijkt niemand meer) en de camera blijft aan tot
je hem zelf uitzet. Scherm delen ernaast → die tegel toont het luie icoontje tot er iemand
kijkt, en dan je eigen scherm (herzien 2026-09-01, beslissing 37); laatste kijker weg → weer
het icoontje.

**C.9c Tegels op Windows (2026-09-01, beslissing 37)**
*Sinds 2026-10-04 bestaan webview, CSP en `thumb://` niet meer; de tegels zijn opnieuw te
controleren in V.10 hieronder. Wat hier staat is de controle voor de Tauri-build.*
Dit is de controle die op de mac niet kan: op Windows liet elke tegel — eigen camera én de
stream van een ander — een donker vlak zien, omdat de webview `thumb://localhost/` vroeg waar
WebView2 `http://thumb.localhost/` verwacht, en de CSP dat adres bovendien niet toeliet.
Verwacht nu: camera aan → binnen een halve seconde je eigen beeld in de "You"-tegel, en de
tegel van een stream die je bekijkt toont het beeld van de ander. Blijft het donker, open dan
de webview-devtools (rechtsklik → Inspect in een debug-build) en kijk naar de console: een
CSP-melding of een 404 op `thumb.localhost` wijst de dader aan.

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

**W.10 De snelste wint een gelijkspel** (2026-08-30)
Zorg dat jullie tweeën de dag in **hetzelfde aantal pogingen** oplossen, en dat één van jullie
er merkbaar langer over doet (wacht een minuut tussen twee gokken). Op de kaart hoort achter
de pogingen een tijd te staan (`4/6 · 1:23`), hoort de snelste bovenaan te staan en als enige
het punt te krijgen, en hoort het scorebord dat te volgen. Controleer ook dat de dialoog na je
laatste gok **Solved in 4 of 6, in 1:23.** zegt. Sluit de app een keer af midden in een spel:
de klok hoort door te lopen vanaf je *eerste* gok, dus de tijd na herstart hoort de hele duur
te zijn en niet alleen die na het opnieuw starten.

**W.11 Een oudere build in de mesh**
Alleen te doen als er nog een instantie van vóór deze versie is: die hoort de
`WordleResult`-ops op te slaan en door te sturen zonder ze te begrijpen (er verschijnt daar
niets), en de derde peer hoort ze via hem alsnog te krijgen. Dit is de invariant "protocol
alleen additief" en er is geen protocolbump geweest.

## GPUI-weergave (2026-10-04)

De weergave is van Tauri naar GPUI gewisseld (`docs/OVERDRACHT.md` beslissing 39). Op de
Mac is elke toestand naast de oude weergave gelegd, en toetsenbord, plakken en slepen zijn
headless getest. Gebruik een **release**-build; een debug-exe tekent alleen op de pc die
hem bouwde.

### Windows

**Eerste Windows-run, 2026-10-14 (dev-PC, RTX 5070 Ti, peers offline):**
- De release-build linkte niet: CVT1100 "duplicate resource", omdat GPUI zelf al een manifest
  inbakt. Opgelost in `build.rs` (alleen nog het icoon); het manifest in de exe heeft
  PerMonitorV2 en common controls v6.
- V.1 **werkt**: donker venster met titelbalk, rail, kanalen, tijdlijn, leden en statusbalk;
  D3D11.1 op de NVIDIA-kaart, mesh luistert. Witte flits niet te zien in een PrintWindow.
- V.12, alleen het rustdeel: **0,23%** van één kern met het venster open, **0,00%** in de
  tray (gemeten over 20 s). Het deel naast een game staat nog open.
- V.3, eerste helft: `WM_CLOSE` verbergt het venster en de app blijft draaien. Terughalen
  via de tray nog niet getest.
- Bijvangst, niet nieuw: een tweede instantie op een bezette poort stopt zonder één logregel,
  want de fout gaat naar stderr en die heeft een windows-subsystem-exe niet.
- Nog met de hand te doen: V.2 t/m V.11 en het game-deel van V.12.

**Zelfde dag, 1.8.0-releasebuild, twee lokale instanties tegen elkaar (127.0.0.1):**
- Verbinden werkt beide kanten op; leden, DM-lijst, RTT in de statusbalk en `v1.8.0`
  kloppen op het scherm (PrintWindow).
- Netjes afsluiten (venster dicht met `minimize_to_tray = false`): het proces is na 158 ms
  weg en de andere kant logt 9 ms later `closed by peer: afsluiten`. Een hard gekilde
  instantie werd pas na 15 s gezien (`timed out`).
- Een tweede start op dezelfde datamap stopt meteen (exitcode 0) en het eerste venster
  blijft draaien.
- GPU-tests (`cargo test -p fitcom-video --release -- --ignored`): alles groen, inclusief
  de nieuwe `een_encoder_loslaten_midden_in_een_stream_laat_het_proces_staan` (de
  stream-stop-crash, vóór de fix ~helft van de runs) en de cliptests (7 op 7 na de
  geluidsfix in `ee9ada0`).

**V.1 Starten en tekenen**
Start `fitcom.exe`. Het venster hoort meteen donker te openen, zonder witte flits, met
titelbalk, rail, kanalen, tijdlijn, leden en statusbalk. *Blijft het venster leeg of zwart,
dan is dat het D3D11-pad van GPUI of zijn shaders, niet de motor: kijk in de log of de mesh
wel opkomt.*

**V.2 Het tray-menu**
Rechtsklik op het tray-icoon: "Openen" en "Afsluiten". "Afsluiten" sluit de app echt af (de
ander ziet je offline gaan). Dubbelklik op het icoon en "Openen" halen het venster allebei
terug. *De tray draait op een eigen Win32-thread die het venster via zijn HWND toont: werkt
"Afsluiten" wel en "Openen" niet, dan zit het in die koppeling.*

**V.3 Sluiten naar de tray, en terug**
Klik het kruisje rechtsboven (met `minimize_to_tray = true`). Het venster verdwijnt, de app
blijft in de tray en blijft synchroniseren: laat de ander een bericht sturen en haal het
venster terug, het bericht hoort er al te staan. Doe dit drie keer achter elkaar. *Komt het
venster terug zonder inhoud, of pas na een klik, zeg dat erbij.*

**V.4 Snap layouts en de titelbalk**
Hang met de muis boven de maximaliseerknop: Windows 11 hoort zijn snap layouts te tonen.
Sleep het venster aan de titelbalk, dubbelklik de titelbalk (maximaliseren en terug), sleep
het tegen een schermrand (snap), en schud het (Aero shake). *De knoppen en de titelbalk laat
GPUI door Windows zelf hit-testen via `WindowControlArea`; doet één van deze dingen niets,
dan zit het in die markering, niet in de knop.*

**V.5 Minimaliseren, maximaliseren, sluiten**
Elk van de drie knoppen één keer, en daarna Alt+F4. Minimaliseren gaat naar de taakbalk,
maximaliseren vult het scherm, en Alt+F4 doet hetzelfde als het kruisje
(naar de tray). Hover over elke knop: hij hoort op te lichten, het kruisje in koraal.

**V.6 Slepen vanuit Verkenner**
Sleep één bestand, en daarna drie tegelijk, het venster in. Tijdens het hangen hoort de
"Drop to share"-overlay te verschijnen, en na loslaten hetzelfde aanbod als via de knop, in
het open gesprek. *Komt de overlay wel en het aanbod niet (of andersom), zeg welke.*

**V.7 Een schermafdruk plakken**
Win+Shift+S, klik in het invoerveld en druk Ctrl+V. Er hoort meteen een bestandsaanbod te
verschijnen en bij de ander een inline afbeelding. Plak daarna gewone tekst: die hoort als
tekst in het invoerveld te komen. *Plakken werkt alleen met de focus in het invoerveld; dat
is bewust anders dan in de webview.*

**V.8 Een melding**
Venster naar de tray, laat iemand `@jouwnaam` sturen: de Windows-melding hoort te komen. Met
het venster op de voorgrond hoort er geen te komen, met niet-storen aan ook niet. *De motor
beslist dit; de weergave meldt alleen of het venster de focus heeft, en dat is nieuwe code.*

**V.9 Schaal 125% en 150%**
Zet in Windows-instellingen → Beeldscherm de schaal op 125%, en daarna op 150%. Tekst en
iconen horen scherp te blijven (niet wazig opgerekt), de pixelletter blijft op zijn raster,
en niets valt buiten het venster. Sleep het venster ook van het ene scherm naar het andere
als die een andere schaal hebben. *Wazig betekent dat de DPI-markering uit het manifest niet
gepakt is.*

**V.10 Streamtegels**
Laat iemand een scherm delen en kijk mee: boven de tijdlijn hoort een tegel met live beeld
te staan, ongeveer twee keer per seconde bijgewerkt. Zet je eigen camera aan: binnen een
halve seconde staat je eigen beeld in de "You"-tegel, niet doorzichtig. Kijk een paar
minuten mee en houd taakbeheer open: het GPU-geheugen van fitcom hoort niet te blijven
groeien.

**V.11 Het pop-out kijkvenster**
Open een stream in zijn eigen venster: dat werkt als voorheen (eigen venster, F11 en
dubbelklik voor beeldvullend). *Het is bij de wissel niet aangeraakt; dit is de controle
dat de nieuwe weergave er niet tussen zit.*

**V.12 In rust naast een game**
Start een game, zet de app op het tweede scherm (venster open, geen gesprek), en daarna in
de tray. Kijk in taakbeheer naar fitcom: processortijd hoort rond nul te liggen, en de game
mag niets merken. Doe het ook tijdens een gesprek. *Dit is invariant 4; de gemeten
cijfers staan in beslissing 39.*

### macOS

**V.13 Tray: Openen en Afsluiten**
Klik het icoon in de menubalk: "Openen" haalt het venster naar voren, "Afsluiten" sluit de
app echt af.

**V.14 Sluiten naar de tray, heen en terug**
Rode knop: het venster verdwijnt, de app blijft in de menubalk. Terug via "Openen" en via
het Dock-icoon. Drie keer.

**V.15 Cmd+Q en Cmd+H**
Cmd+Q sluit de app echt af (de ander ziet je offline gaan), Cmd+H verbergt hem.

**V.16 Hover**
Ga met de muis over een bericht (de acties horen te verschijnen), over een kanaal, een lid,
een knop in het spraakpaneel en de knoppen in de instellingen. Elk hoort een eigen
hovertoestand te tonen. *Dit kon tijdens de bouw niet met de hand.*

**V.17 Toetsenbordfocus**
Tab door het venster: de focusring hoort zichtbaar van knop naar knop te gaan, in een
logische volgorde. In het invoerveld: @ opent de lijst, pijltjes en Enter kiezen, Esc sluit.
*Tab zonder open @-lijst doet bewust niets.*

**V.18 Plakken en slepen**
Cmd+Ctrl+Shift+4 (schermafdruk naar het klembord), dan Cmd+V in het invoerveld: een aanbod. Sleep een
bestand vanuit de Finder het venster in: overlay, dan aanbod.

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
5. Clipmap wijzigen (Instellingen → Clips → Change…): nieuwe clips landen in de gekozen
   map, de al bewaarde clips blijven staan waar ze stonden, en de ring van de oude map is
   weg in plaats van voor altijd blijven liggen. Werkt met de opname aan (hij herstart) en
   uit.
6. Frametime-vergelijking spel met recorder aan/uit — de meetpunt-notitie in SPEC.md.

## Spotify delen (2026-10-14, beslissing 40)

Geautomatiseerd gedekt (`cargo test`): stereo blijft stereo door Opus heen, een stem komt
op beide kanalen, stereo herbemonsteren houdt links en rechts uit elkaar, de bovenste
`Spotify.exe` wordt gevonden, muziek wordt gehoord zodra je in het gesprek zit (zonder
kijken) en een nieuw nummer is een nieuwe titel en geen nieuwe stream.

Met de hand, want echte Spotify en twee machines:

1. A en B in het gesprek, Spotify draait bij A. A drukt **Share Spotify**: B hoort de
   muziek zonder iets te doen, in stereo (een nummer met duidelijk links/rechts), en ziet
   "A plays Artiest – Nummer" in het voicepaneel en een SPOTIFY-schuif onder A.
2. Geen spelgeluid, Discord of meldingen van A in de muziek, en A's eigen stem ook niet.
3. Volgend nummer bij A: binnen een paar seconden staat de nieuwe titel bij B.
4. Pauze bij A van een minuut, dan verder: de muziek komt bij B terug zonder opnieuw te
   delen. (Dit was de stiltebug; ook bureaubladgeluid na een stilte nalopen.)
5. Spotify bij A dicht en weer open, en dan afspelen: de muziek komt bij B vanzelf terug.
6. Share Spotify zonder dat Spotify draait: nette foutmelding, niets gedeeld.
7. A deelt ook zijn scherm, B kijkt: B hoort Spotify één keer, niet dubbel. Stop Spotify:
   het bureaubladgeluid van het scherm komt terug.
8. Kwaliteit: hetzelfde nummer bij B via de stream en rechtstreeks in Spotify vergelijken,
   op een koptelefoon. Verwachting: niet te onderscheiden. **Spotify's eigen volume bij A op
   100%**: de opname zit ná die schuif, dus Spotify op 30% is zachte (en minder fijne) muziek
   voor iedereen. Harder of zachter doet B met de SPOTIFY-schuif, A met het Windows-volume.
9. Gamen bij A met Spotify gedeeld: geen merkbare impact (invariant 4).
10. macOS als DJ: Screen Recording-toestemming is er al; eerste keer vraagt macOS of de app
    Spotify mag bedienen (voor de titel). Nee zeggen = de stream heet "Spotify", muziek werkt.
11. B is een 1.8.0-peer: krijgt geen muziek, maar zijn stem en de rest werken gewoon.

## Luisteraars bedienen de muziek (2026-10-15, beslissing 41)

Geautomatiseerd gedekt (`cargo test`): `MusicControl` (tag 46) komt heel over de draad en
een onbekende actie decodeert als onbekend; de DJ-regel (`mag_muziek_bedienen`) weigert een
afzender buiten het gesprek, een DJ die zelf niet in het gesprek zit, een andere stream-id en
een onbekende actie; het track-id is precies 22 tekens base62 en de linkparser pakt alleen
`open.spotify.com/[intl-xx/]track/<id>`. In fixturemodus nagekeken: de twee knoppen naast
de titel bij de luisteraar, en "Open in Spotify" onder een tracklink alleen bij de DJ.

Met de hand, want echte Spotify en twee of drie machines. A deelt Spotify, B en C luisteren:

1. B drukt de pauzeknop (naast de titel onder A in de ledenlijst): Spotify bij A pauzeert,
   binnen een halve seconde. Nog eens: hij speelt verder. Er verschijnt niets bij A behalve
   wat Spotify zelf doet.
2. C drukt Volgende: A's Spotify slaat over, en binnen een paar seconden staat de nieuwe
   titel bij B en C.
3. B verlaat het gesprek: de knoppen verdwijnen bij B (de muziek hoort hij dan ook niet
   meer). Wat B daarna nog zou sturen weigert A; dat staat als `debug`-regel
   "muziekbediening genegeerd" in A's log.
4. A stopt met delen en deelt opnieuw: knoppen bij B werken weer (nieuwe stream-id).
5. Spotify bij A staat in de **Store-versie** en in de **los geïnstalleerde versie**: beide
   reageren op pauze en volgende (de mediasessie heet anders, `SpotifyAB.SpotifyMusic_…!Spotify`
   tegenover `Spotify.exe`).
6. Spotify bij A is dicht maar A deelt nog: pauze en volgende doen niets, geen foutmelding,
   en Spotify start **niet** vanzelf op (ook niet op macOS).
7. B plakt `https://open.spotify.com/track/<id>?si=…` in de chat. A (de DJ) ziet eronder
   **Open in Spotify**; B en C zien alleen de link. A klikt: het nummer opent in A's
   Spotify-app en speelt pas als A er zelf op drukt of het in zijn wachtrij zet. Ook met
   `open.spotify.com/intl-nl/track/…`.
8. Twee keer achter elkaar op dezelfde knop: Spotify bij A springt naar dat nummer, er gaat
   niets dubbel in de wachtrij en er opent geen tweede venster.
9. Een link naar een album of afspeellijst geeft géén knop. Een kapotte link (te kort, rare
   tekens) ook niet.
10. A is een macOS-DJ: de eerste pauze vraagt mogelijk opnieuw of de app Spotify mag
    bedienen (dezelfde toestemming als voor de titel). Ja: pauze en volgende werken. Nee: ze
    doen niets, de muziek speelt door.
11. B draait 1.9.0 (zonder deze functie) en is DJ: de knoppen bij A doen niets, verder werkt
    alles. Omgekeerd ziet een 1.9.0-luisteraar gewoon geen knoppen.
12. Gamen bij A terwijl B herhaaldelijk op Volgende drukt: geen merkbare hapering
    (invariant 4).

## Push-to-talk en push-to-mute (2026-10-15, beslissing 42)

Nooit op een echte tweede machine of met een echt spel geprobeerd: de poortlogica en de
configvalback hebben unit-tests, het lezen van de toets zelf (`GetAsyncKeyState`) is alleen
op deze PC bekeken in de instellingen (fixturemodus), niet ingedrukt. Doe dit met A (de
tester) en B (luistert), A met een headset, in een gesprek. Per stap kijkt A ook naar de
eigen spreekring (de ring moet doen wat B hoort).

**Voorbereiding.** Update A naar deze build. Instellingen, tabblad Audio, sectie "Push to
talk".

1. **Standaard is spraakdetectie.** Een bestaande `config.toml` zonder `[push_to_talk]`:
   de app start, de modus staat op *Voice activity*, de toets op *Right Ctrl*, en A wordt
   gehoord zoals vroeger. Na het kiezen van iets staat er een `[push_to_talk]`-tabel in.
2. **Push-to-talk, Rechter-Ctrl.** Kies *Push to talk*. A praat zonder de toets: B hoort
   niets en A's ring blijft donker. A houdt Rechter-Ctrl vast en praat: B hoort het, de ring
   brandt. Zachte spraak en fluisteren komen ook door (de VAD geldt er niet bovenop). Los
   laten: het laatste woord wordt niet afgekapt en de ring dooft binnen ~0,2 s.
3. **Zonder te praten** de toets vasthouden: B hoort alleen stilte/ruis, geen fouten in de
   log, en na loslaten stopt het verkeer (Tailscale-teller of `netstat`).
4. **Het kan live.** Wissel van modus en toets tijdens het gesprek: het geldt zonder
   herverbinden, B hoort geen onderbreking en er klinkt geen join/leave-toon.
5. **Push-to-mute.** Kies *Push to mute*. A praat gewoon: B hoort het. A houdt de toets
   vast en praat: B hoort niets, direct (geen staart), en de ring gaat uit. Los laten: het
   gaat weer open met de VAD.
6. **Muisknoppen.** Kies *Mouse 4* en daarna *Mouse 5* (de zijknoppen): zelfde gedrag als
   stap 2. Als de muis zijn eigen software heeft (G Hub, Synapse) en het niet werkt: noteer
   welke, en probeer de knop daar op "standaard" te zetten.
7. **Met een spel vooraan (de hoofdreden).** Start een spel in **exclusieve volle-scherm-
   modus** en daarna in venster zonder rand, en doe stap 2 met Rechter-Ctrl en met een
   zijknop. Dit moet werken zonder dat A de app voor de voorgrond haalt. Kijk ook of het
   spel de toets zelf nog krijgt (dat hoort zo; kies een toets die het spel niet bindt).
8. **Een spel als administrator.** Start het spel "als administrator" en herhaal stap 2.
   Werkt het niet: dat is mogelijk de UIPI-grens van een niet-verhoogd proces; noteer het, dan
   beslissen we of er een melding of een andere aanpak voor nodig is.
9. **Gamen wint (invariant 4).** Een zwaar spel met push-to-talk aan en veel toetsen/muis
   gebruiken: geen merkbare hapering, geen extra frame-tijd ten opzichte van VAD-modus.
10. **Mute wint.** Zet A op mute (de knop in het voicepaneel), kies *Push to talk* en houd
    de toets vast: B hoort niets, de ring blijft uit. Zelfde met deafen.
11. **Apparaatwissel.** Wissel van microfoon in de instellingen tijdens het gesprek: de
    sessie herstart kort en de gekozen modus en toets gelden daarna meteen.
12. **Kapotte config.** Zet in `config.toml` `mode = "schreeuwen"` en `key = "Hyper"`: de
    app start, logt twee waarschuwingen, staat op *Voice activity* met *Right Ctrl*. Zet
    `key = "mouse4"` (kleine letters): hij staat als *Mouse 4* in de lijst.
13. **macOS** (alleen te zien, niet te testen): het tabblad toont "Push to talk is not
    available on macOS yet" en een bestaande `mode = "talk"` in de config doet niets: de
    microfoon blijft op spraakdetectie en de log meldt het één keer.

## Laatste clip delen (2026-10-15, beslissing 44)

Windows met clips aan, plus minstens één andere peer (liefst twee, waarvan één op macOS).
Instellingen > Clips: "Post clips in the chat automatically" staat standaard UIT. Na elke
clip ziet alleen de maker in #general een kaart "Clip saved" met een Share-knop.

1. **Standaard uit, ook na de update.** Een pc met een config uit 1.10.0 (met
   `auto_delen = true` erin): na de update staat de schakelaar uit. In `config.toml` is
   `auto_delen` bij de volgende opslag weg en staat er desnoods `auto_post = false`.
2. **De kaart is alleen van jou.** A zit in het gesprek (schakelaar uit), drukt F9. In
   #general verschijnt bij A een kaart "Clip saved · <grootte> · only you can see this" met
   Share. B en C zien niets.
3. **Share op de kaart.** A klikt Share: de kaart wordt "shared in the chat", zonder knop; B
   en C krijgen een bestandskaart `clip-<tijd>.mp4` (na het hashen) zonder automatische
   download. B downloadt en speelt hem af.
4. **Niet tweemaal.** De knop in Instellingen > "Share last clip" staat nu ook uit.
5. **Nieuwe clip, nieuwe kaart.** A drukt weer F9: de oude kaart maakt plaats voor één
   nieuwe, met knop. Er is nooit meer dan één kaart.
6. **Buiten een gesprek.** A is niet in het gesprek, F9: kaart met Share; klikken werkt.
7. **Auto-post aan.** Zet de schakelaar aan (blijft na herstart staan). In het gesprek F9:
   de bestandskaart verschijnt bij iedereen, en A's kaart zegt meteen "shared in the chat".
   Buiten een gesprek met schakelaar aan: niets automatisch, wel de kaart met knop.
8. **Herstart.** Na herstarten van A is er geen kaart en staat "Share last clip" uit.
9. **Clip weg.** Verwijder de laatste clip buiten de app en klik Share: een foutmelding
   "De clip staat niet meer op schijf"; er komt niets in de chat.
10. **Grote clip.** Clipvenster op 30+ minuten, F9, Share: de app blijft bruikbaar tijdens het
    hashen, een spel op de voorgrond merkt niets (invariant 4).
11. **Offline peer.** C is offline bij het posten: C ziet de kaart bij het verbinden, zonder
    automatische download.
12. **macOS:** er is geen kaart en geen Clips-tab; een Mac downloadt en opent de clip van een
    Windows-peer als elk ander bestand.

## "Speelt X" naast je naam (2026-10-15, beslissing 43)

Nooit met een echt spel of een tweede machine geprobeerd: de wisselbewaking en het
schoonmaken van de naam hebben unit-tests, het bericht heeft een roundtrip-test; de
Win32-meting (voorgrondvenster, monitor, versie-resource) is alleen gecompileerd. Doe dit met
A (speelt) en B (kijkt in de ledenlijst), beide op deze versie, en C of een peer op 1.9.0 als
je er een hebt.

1. **Standaard aan.** Bestaande `config.toml` zonder `share_activity`: de app start, Instellingen
   > Account toont de schakelaar "Show the game I'm playing" aan.
2. **Schermvullend spel.** A start een spel in *exclusieve volle-schermmodus*. Binnen ~6-9 s
   toont B onder A's naam (onder een eventuele statusregel) "Playing <spelnaam>". Herhaal met
   *venster zonder rand*.
3. **Naam.** Is het de nette naam van het spel en niet `spel.exe`? Noteer per spel wat er
   staat; bij een nietszeggende naam ("Unreal Engine", launcherachtig) is dat een kandidaat
   voor een betere keuze of voor `GEEN_SPEL`.
4. **Geen vals alarm.** A zet schermvullend vooraan: een browser (F11), een video in een speler,
   OBS, Spotify, Discord, een maximaal venster met verborgen taakbalk. Geen van allen mag een
   "Playing"-regel geven. Wel iets onverwachts: noteer de exe-naam voor `GEEN_SPEL`.
5. **Gewoon venster met rand.** Een spel in venstermodus (met titelbalk) geeft bewust niets.
6. **Alt-tab flapt niet.** A alt-tabt kort (< 30 s) naar een browser en terug: B's regel blijft
   staan. A blijft een halve minuut of langer buiten het spel: de regel verdwijnt.
   Wisselen tussen twee spellen: de regel volgt binnen ~6-9 s.
7. **Verbinding komt later op.** A speelt al; B start de app pas daarna: B ziet het spel
   zodra de verbinding er is. A of B verbreekt de verbinding: de regel verdwijnt bij de
   ander.
8. **Uitzetten.** A zet de schakelaar uit terwijl het spel draait: B's regel verdwijnt direct
   en komt niet terug zolang hij uit staat (ook niet na herstart van A; `share_activity =
   false` staat in `config.toml`). Aanzetten: de regel komt binnen ~6-9 s terug.
9. **Oudere peer.** Een peer op 1.9.0 verbindt gewoon, ziet niets en logt hooguit een
   onbekende tag (46/47 `debug`).
10. **Gamen wint (invariant 4).** Een zwaar spel met delen aan: geen merkbare hapering of
    frametijdpiek elke 3 s ten opzichte van delen uit.
11. **Rare namen.** Hernoem (testbuild of exe met aangepaste resource) naar een naam met
    stuurtekens of een heel lange naam: B ziet de opgeschoonde, ingekorte versie (max 48 tekens).
12. **macOS** (alleen te zien): de schakelaar staat er, doet niets; een Mac onder een Windows-peer
    toont wel diens "Playing"-regel.

## Tekenen op een gedeeld scherm (2026-10-15, beslissing 45)

Op deze pc geverifieerd: het model, de routering en het penseel met unit-tests; de laag bij
de deler op het echte bureaublad (BitBlt en Windows.Graphics.Capture zien hem niet, 0 tegen
450 rode pixels zonder uitsluiting); de pen in een echt kijkvenster met berichten in zijn
wachtrij (geen echte muis); de knoppen in de fixturemodus. **Nooit met twee machines, nooit
met een spel ernaast.** Doe dit met A (deelt een monitor), B en C (kijken), alle drie op deze
versie, en een peer op 1.9.0 als je er een hebt.

1. **Eerste lijn.** A deelt een monitor, B kijkt. B's venstertitel eindigt op "— right-click
   to draw". B drukt D (of rechtsklik > "Draw on the stream"): de cursor wordt een kruisje.
   B sleept een cirkel. Bij A verschijnt hij op de monitor, op de juiste plek ten opzichte
   van wat B ziet, terwijl B nog tekent (hij groeit mee). Bij B zelf verschijnt hij ook, een
   fractie later (via A). Meet hoe ver de lijn achter de muis aanloopt; noteer het.
2. **Iedereen ziet iedereen.** C kijkt ook en tekent in een andere kleur; A, B en C zien
   beide lijnen. C begint pas te kijken nadat B al tekende: C ziet B's lijnen meteen.
3. **Pen neer.** Zonder D: klikken en slepen in B's venster tekent niets. Dubbelklik
   (beeldvullend) laat geen stip achter, ook niet met de pen op.
4. **Kleur en dikte.** Rechtsklik: zes kleuren, drie diktes, een vinkje bij de huidige. Een
   keuze zet de pen op. Een nieuw kijkvenster houdt de laatst gekozen pen.
5. **Ongedaan en wissen.** B: Ctrl+Z haalt zijn laatste lijn weg, bij iedereen; Delete al
   zijn lijnen; C's lijnen blijven staan. Hetzelfde via het menu.
6. **De deler wist.** A: "Clear drawing" in Settings → Video wist alles bij iedereen. "Allow
   drawing" uit: alles weg, B's titel verliest de hint, het menu zegt "Drawing is off for this
   stream", slepen doet niets. Weer aan zet het terug. Stop en deel opnieuw, en herstart A: de
   laatste stand geldt nog (`allow_drawing` in `config.toml`, beslissing 49).
7. **Vertrek.** B sluit zijn venster: B's lijnen verdwijnen bij A en C. C verliest de
   verbinding (Tailscale uit): C's lijnen verdwijnen. A stopt met delen: alles weg.
8. **Spel vooraan (invariant 4, het belangrijkste punt).** Er staan lijnen op A's monitor.
   A start een spel in *exclusieve volle-schermmodus* op die monitor: de lijnen verdwijnen
   bij A (niet bij B en C), het spel verliest geen beelden en schiet niet naar de
   achtergrond. Kijk met een frametijdgrafiek (RTSS of de prestatie-overlay van de
   GPU-driver) of er een piek is op het moment van alt-tabben naar het spel, en of het spel
   in "Hardware: Independent Flip" blijft (PresentMon). Herhaal met *venster zonder rand*.
   Alt-tab terug naar het bureaublad: de lijnen komen terug. Teken terwijl A speelt: geen
   enkele hapering bij A.
9. **Niet in het beeld.** Er staan lijnen bij A. B ziet elke lijn één keer (niet dubbel, niet
   verschoven). Een clip die A tijdens het tekenen bewaart (F9) bevat geen lijnen.
10. **Ander scherm.** A heeft twee monitoren en deelt de tweede; de lijnen staan op de tweede.
    Een spel schermvullend op de *eerste* monitor laat de laag op de tweede staan.
11. **Venster delen.** A deelt een venster: B en C kunnen tekenen en zien het, A ziet de lijnen
    alleen op zijn eigen tegel in de app (geen laag; zo bedoeld).
12. **Tegels.** In de streamstrook staan de lijnen op B's tegel van A's scherm en op A's eigen
    tegel, binnen een halve seconde.
13. **DPI.** Kijkvenster op een monitor met 150% schaal, A op 100% (en andersom): de lijn
    staat bij A op dezelfde plek in het beeld als bij B. Ook beeldvullend (F11) bij B.
14. **Oudere peer.** A op 1.9.0 deelt: B krijgt geen hint en het menu zegt "Drawing is off".
    B op 1.9.0 kijkt naar A: hij ziet geen lijnen, verder werkt alles; A logt niets
    vreemds.
15. **Herstart van een kijker.** B tekent, sluit de app hard af (Taakbeheer) en start binnen
    15 s opnieuw, kijkt weer en tekent: de nieuwe lijn tekent niet door op een oude. Oude
    lijnen van B kunnen bij A blijven staan tot B "Clear my lines" doet of de verbinding
    echt wegvalt.
16. **macOS** (alleen te zien): een Mac die kijkt kan niet tekenen (geen menu, geen hint) maar
    ziet de lijnen op zijn tegel; een Mac die deelt heeft geen laag, zijn kijkers tekenen en
    zien elkaar, hij ziet het op zijn eigen tegel.

## Protocol 6: sleutels, getekende handshake, versleutelde media (2026-10-19, beslissing 46)

Fase A van `docs/PLAN-ZONDER-TAILSCALE.md`. **Nog niets hiervan tussen twee machines.**

Op deze pc geverifieerd:
- Met tests in `crates/net`: een doorgestuurde handtekening krijgt geen `HelloAck`; een
  onbekende sleutel krijgt een stille close; een `Hello` op 5 krijgt alleen de versie; een peer
  op 5 die sluit met "protocolversie" verschijnt als versieverschil; een andere sleutel voor
  dezelfde `PeerId` wordt geweigerd met `IdentityChanged` zonder belstorm; twee echte meshes
  leiden dezelfde mediasleutels af en een derde gaat niet open; de AEAD weigert een vervalste
  tag, een herhaling, een te oud pakket en een onbekende index (zonder ontsleutelpoging); het
  venster klopt met een referentiemodel.
- Drie motoren op loopback met echte `identity.toml`/`config.toml` (zonder venster): verse
  installaties pinnen elkaars sleutel vanzelf, na een herstart wordt niets opnieuw geleerd,
  en een peer met dezelfde `peer_id` maar een nieuwe sleutel wordt door de andere twee
  geweigerd terwijl die twee verbonden blijven.
- De routerprobe op Ricks router: PCP yes, NAT-PMP yes, UPnP no, publiek IPv4, geen IPv6 —
  hetzelfde als `tailscale netcheck` op 2026-10-18.
- De stoottest op loopback is niet slechter geworden (beslissing 46, "Gemeten").

**Vóór de release:**
- **A-vooraf:** kijk in Settings → Network (versie achter elk adres) of beide vrienden echt op
  1.10.1 zitten. Hun auto-update faalde stil zolang `fitcom-updater.exe` niet met de hand
  vervangen was. Wie op 5 blijft valt na deze release uit de mesh.
- **De configs:** staat in alle drie de `config.toml`'s bij elke peer het tailnet-adres
  (`100.x`) of de MagicDNS-naam? Een sleutel wordt alleen op het tailnet geleerd, dus een peer
  op een LAN- of Radmin-adres (peer-3 stond op een Radmin-IP) komt er na de update niet meer
  in, met "No key known for this peer".

**Bij de release: alle drie tegelijk bijwerken.**

1. **Eerste start na de update.** Alle drie op deze versie. Binnen een minuut staat iedereen
   online, zonder dat iemand iets deed. In elke `config.toml` staat nu bij elke peer een
   `public_key = "…"` naast `known_id`, en in `identity.toml` een `private_key`. In de log
   eenmaal per peer "sleutel vastgepind".
2. **Tweede start.** Alle drie herstarten: weer online, geen nieuwe "sleutel vastgepind".
3. **Iemand blijft achter.** Eén peer start nog 1.10.1. De anderen zien bij hem "Runs an older
   protocol, this build speaks 6"; hij ziet bij hen "Runs protocol 6, this build speaks 5".
   Niet "offline".
4. **Andere sleutel, zelfde peer.** Kopieer bij C `identity.toml` weg en zet er een voor in de
   plaats met dezelfde `peer_id` maar zonder `private_key` (C maakt dan een nieuwe sleutel).
   A en B tonen bij C "A different identity answered at this address." en C komt er niet in.
   Zet het origineel terug: C is weer welkom. (Echt herstellen na een herinstallatie: bij A en
   B de `known_id` en `public_key` van C uit `config.toml` halen.)
5. **Voice.** Gesprek met z'n drieën, tien minuten. Klinkt als 1.10.1: geen tikken, geen
   extra vertraging. Spreekringen kloppen.
6. **Schermdelen op 12 Mbit/s.** A deelt 1080p60, B en C kijken vijf minuten, met
   `FITCOM_SPOOR` op een map gezet (de sporen van de kijker). Vergelijk met een spoor van
   1.10.1: verlies (`verlies-<id>.csv`) en aankomst → getoond (`kijker-<id>.csv`) niet hoger.
   Let op: de meter van de deler telt nu de 28 bytes van de versleuteling mee, dus zijn
   bitrate staat ~2,5% hoger voor hetzelfde beeld. Een slechter resultaat is een blokkade voor
   de release, geen afweging.
7. **Bureaubladgeluid en Spotify.** A deelt beide; B en C horen het, met eigen volume.
8. **Camera.** Een Windows-peer deelt zijn camera; de anderen zien hem.
9. **mac ↔ Windows.** De Mac (bouwt uit de broncode) en een Windows-pc verbinden, pinnen
   elkaars sleutel, voice beide kanten op, de Mac kijkt naar een gedeeld scherm van Windows
   en Windows naar de Mac. Dit is de handshake die nog nooit op echte machines liep; de
   mac-code is vanaf Windows niet eens gecompileerd (`crates/video/examples/mac_keten.rs`
   alleen nagelezen).
10. **Herverbinden.** Tailscale bij B even uit en weer aan: B komt terug, voice en beeld
    lopen weer zonder opnieuw intekenen. In de log van A geen "verbinding geweigerd".
11. **Opnieuw deelnemen en kijken.** Verlaat het gesprek en kom terug, sluit een kijkvenster
    en open het opnieuw: geluid en beeld komen meteen terug.
12. **Peer op een niet-tailnetadres.** Zet bij één peer tijdelijk een LAN-adres in
    `config.toml` (en haal zijn `public_key` weg). Hij komt niet binnen, met "No key known for
    this peer. Keys are only learned over the tailnet." Zet het tailnetadres terug.
13. **Routerregel (A0).** Settings → Network → Reachability toont na ~10 s een regel als
    "Router: PCP yes · NAT-PMP yes · UPnP no · public IPv4 · no IPv6". **Beide vrienden sturen
    hier een screenshot van**, met het netwerk op en **zonder Tailscale-exitnode** (met een
    exitnode vindt hij de router niet). Staat er "Router: not found", wacht dan een minuut:
    hij probeert het één keer opnieuw. Kijk in de router of er na een minuut geen mapping van
    FitCom meer staat; en nog eens na de app 11 s na het starten hard af te sluiten (dan mag
    hij blijven staan tot zijn lease verloopt, hooguit twee uur).
14. **macOS:** vraagt de Mac bij de eerste start om toegang tot het lokale netwerk (UPnP zoekt
    met een multicast)? Dat is nieuw; noteer het, en of de regel daarna klopt.

## Fase B: verbinden zonder Tailscale (2026-10-19, beslissing 47)

Fase B van `docs/PLAN-ZONDER-TAILSCALE.md`: uitnodigingscodes, port mapping, bellen op
kandidaten, adressen uitwisselen, één mediasocket met roaming. Additief op protocol 6: een peer
met alleen fase A blijft werken (zie punt 11). **Nog niets hiervan tussen twee huizen.**

Op deze pc geverifieerd:
- Tests in `crates/net/tests/mesh.rs`: twee peers verbinden op alleen elkaars code via een
  LAN-adres (waar nooit een sleutel geleerd wordt), met een dood adres vooraan; twee werkende
  kandidaten tegelijk geven na de botsing één verbinding die blijft; een peer erbij terwijl de
  mesh draait; een kandidaat die QUIC haalt en stil sluit (de eigen pc) houdt de rest niet
  tegen; op het configadres blijft een andere installatie "andere identiteit"; een derde
  vertelt A waar B zit en A en B verbinden zelf.
- `crates/net/src/verdeler.rs`: stem en beeld bij hun eigen consument, twee keyframes tegelijk
  heel, het antwoord gaat na een keepalive naar het echte bronadres, een vertrokken kijker
  ruimt zijn route op. `mediasleutel.rs`: leren pas na een keepalive en alleen van echte
  pakketten.
- B5 tegen de build van vóór B5 met `hapering.rs` en `FITCOM_SPOOR` (beslissing 47, tabel).
- B2 op Ricks router met `crates/net/examples/poortmeting.rs`: zie beslissing 47 voor wat de
  router opende en wat het in rust kost.
- Willekeurige UDP op beide poorten: zie beslissing 47.

**Vóór de release** (alles hieronder met Tailscale **uit** op beide pc's, tenzij anders):

1. **Firewall precies zoals de README het zegt.** Beide vrienden zetten de firewall volgens
   `README.md` → "Firewall": fitcom op "Privé", thuisnetwerk op profiel "Privé". Controleer met
   het `Get-NetFirewallRule`-commando uit de README dat er een `Allow` voor `Private` staat en
   geen `Block`. Test daarna punt 2 **zonder** iets extra's open te zetten. Werkt het pas met de
   firewall uit, dan klopt de README niet: dat is een blokkade.
2. **Twee huizen, Tailscale uit, alleen de code.** Rick en een vriend zetten Tailscale uit
   (afsluiten, niet alleen pauzeren), wisselen codes uit via de groepsapp en plakken die. Binnen
   een halve minuut online; in Settings → Network bij de ander "direct · internet". Kijk in de
   log naar "port mapping gestart" en "control-poort gemapt" bij beiden.
3. **Voice en 12 Mbit/s schermdelen op dat pad.** Tien minuten praten, vijf minuten 1080p60
   delen met `FITCOM_SPOOR` bij de kijker. Vergelijk `verlies-<id>.csv` en `kijker-<id>.csv` met
   een spoor over het tailnet (punt 6 van "Protocol 6"). Niet slechter; anders is het een
   blokkade.
4. **Beide apps herstarten.** Weer online zonder nieuwe code. In `<data>/adressen.json` staan
   de adressen van de ander.
5. **Router herstarten (nieuw IP).** Eén van de twee herstart zijn router (bij de meeste
   providers krijg je dan een ander IP, controleer dat op een "wat is mijn IP"-site). Met de
   derde peer online: hij heelt vanzelf, binnen een paar minuten, zonder nieuwe code. Zonder
   derde peer: alleen als het onthouden adres nog klopt; anders is opnieuw codes plakken de
   bekende grens.
6. **De derde peer zonder port mapping.** Zet bij de derde `port_mapping = false` (of test bij
   iemand wiens router geen UPnP/PCP/NAT-PMP doet). Hij verbindt via B4 met de andere twee: hij
   belt naar buiten, de anderen vertellen hem hoe ze elkaar zien. Kijk in de log van de anderen
   of zijn adres als "geleerd" binnenkomt.
7. **Twee peers zonder port mapping, met een derde.** Zet bij twee peers `port_mapping = false`;
   de derde houdt hem aan. De twee verbinden met elkaar via wat de derde ze vertelt (doorprikken
   met gelijktijdig bellen). Lukt dit niet, noteer welke router en provider: dat is fase C.
8. **Router netjes achterlaten.** Kijk in de router (bij PCP/NAT-PMP soms onzichtbaar; bij UPnP
   meestal een lijst) dat er na netjes afsluiten geen mapping van FitCom meer staat. Na hard
   afsluiten mag hij blijven staan tot zijn lease verloopt.
9. **LAN.** Twee peers op hetzelfde thuisnetwerk: "direct · LAN".
10. **Met Tailscale aan.** Alles werkt nog als vroeger; het pad is "via tailnet".
11. **Eén peer nog op fase A (alleen protocol 6, zonder B).** Voice en schermdelen beide
    kanten op werken (de B-peer roamt niet naar hem: hij stuurt geen keepalive).
12. **IPv6 (B7, niet gebouwd).** Noteer bij elke vriend de A0-regel ("IPv6" of "no IPv6").

**macOS** (vanaf Windows niet te compileren: `ring` en libopus vragen de C-compiler van Xcode):
13. `cargo check --workspace --all-targets` en `cargo test -p fitcom-net -p fitcom-proto` op de
    Mac. `crates/video/examples/mac_keten.rs` draait (één mediasocket per kant).
14. Codes uitwisselen tussen Mac en Windows, Tailscale uit: verbinden, voice beide kanten op, de
    Mac kijkt naar Windows en andersom.
15. Port mapping op de Mac: vraagt macOS om toegang tot het lokale netwerk (UPnP zoekt met een
    multicast)? En klopt de Reachability-regel daarna?
16. De kijker op de Mac: beeld blijft vloeiend (de kijker slaapt nu het laatste stuk voor een
    beeld met `thread::sleep`; op macOS is dat al fijn, dus geen verschil verwacht).

## Spraakoverlay (2026-10-22, beslissing 53)

Op deze pc geverifieerd: het hele beeld van de overlay in een geheugen-DC (Inter Medium laadt
onder zijn GDI-naam, opgelichte tekst wit, gedimde grijs, pil op 85 %, doorzichtig tussen de
rijen), de rijen uit de feeds, het klemmen van de plek en de config, met unit-tests. **Het
venster zelf heeft nog nooit op een bureaublad gestaan** (Rick zat te gamen). Eén pc met een
tweede peer in het gesprek is genoeg.

1. **Aan en uit.** Settings → Audio → "In-game overlay" aanzetten. Niets te zien zolang je niet
   in het gesprek zit. Deelnemen: linksboven op het hoofdscherm een pil met je naam, en één per
   peer in het gesprek. Verlaten: weg. Uitzetten tijdens een gesprek: weg; aan: terug.
2. **Oplichten.** Praat: je eigen naam wordt wit met een groene ring, en dooft kort na je
   laatste woord. Op mute licht hij niet op, ook niet als je praat. Een peer die praat licht op,
   in de pas met de ring in de ledenlijst.
3. **Doorklikbaar en geen focus.** Klik op de overlay zonder Alt: de klik gaat naar wat
   eronder staat. Het spel verliest nooit de focus, ook niet bij oplichten.
4. **Alt + slepen.** Houd Alt vast en sleep een pil: de overlay schuift mee. Los: weer
   doorklikbaar. Herstart de app: hij staat waar je hem liet (`[overlay] x`/`y` in
   `config.toml`). Sleep hem half van het scherm af: bij het volgende oplichten springt hij
   terug binnen het scherm. Sleep hem naar een tweede monitor met een andere schaal: na het
   loslaten heeft hij de maat van die monitor.
5. **Monitor weg.** Zet hem op een tweede monitor, sluit af, koppel die monitor los, start:
   linksboven op het hoofdscherm.
6. **Niet in de opname.** Deel je scherm en maak een clip terwijl de overlay staat: de kijker
   en de clip zien hem niet.
7. **Over een spel.** Een randloos spel: de overlay staat erboven. Noteer de frametijd met en
   zonder overlay (PresentMon, of het overlay van de GPU-driver) en of het spel uit
   "independent flip" valt. Exclusief schermvullend: geen overlay, zoals afgesproken.
8. **Een spel als administrator.** Werkt Alt + slepen nog? Zo niet, noteer het spel.

## Een gedeeld scherm bedienen (2026-10-22, beslissing 54)

Op deze pc geverifieerd, zonder echte invoer: de berichten en hun grenzen (proto), wie mag
vragen en bedienen en wanneer het stopt (`streams.rs`, ook de kijkerskant), het omrekenen naar
de gedeelde monitor en het klemmen, de `INPUT`-structuren die `SendInput` zou krijgen, het
loslaten bij het einde, wat het kijkvenster van toetsen en knoppen maakt, Escape vasthouden,
en de knoppen in het hoofdvenster (headless). **Nooit is er echte invoer gezet, nooit het
echte klembord aangeraakt, nooit met twee machines.** Doe dit met A (Windows, deelt een
monitor) en B (Windows, kijkt), allebei op deze versie, plus C als derde kijker.

1. **Standaard uit.** Een verse config: in de kiezer staat "Allow remote control" uit, en B's
   venstertitel noemt geen bedienen; rechtsklik bij B zegt "Remote control is off for this
   stream". In Settings → Video staat "Allow remote control" uit.
2. **Per scherm.** A zet in de kiezer "Allow remote control" aan en deelt een *monitor*: B's
   titel eindigt op "right-click to take control" (of "draw or take control"). A deelt
   daarnaast een *venster* met dezelfde schakelaar aan: daar blijft het uit (geen pijl op de
   regel in het gesprekspaneel, B's menu zegt "off").
3. **Vragen en toestaan.** B: rechtsklik > "Take control of this screen". B's titel: "asked for
   control, waiting…". Bij A verschijnt onder de titelbalk "B asks to control your mouse,
   keyboard and clipboard text on …" met Deny en Allow, ook als A niet in het gesprek zit en in
   een ander kanaal staat. Staat A's venster op de achtergrond: een Windows-melding (niet met
   niet-storen aan). A klikt Allow: B's titel wordt "you control this screen · hold Esc to
   stop", A's balk zegt "B controls …" met Stop control.
4. **Muis.** B beweegt in zijn kijkvenster: A's muis volgt, op de juiste plek ten opzichte van
   het beeld, tot in de vier hoeken en nooit op A's andere monitor. Links-, rechts-,
   middenklik, dubbelklik (een map openen), slepen (een venster verplaatsen), het wiel en de
   zijknoppen (terug in een browser). Meet hoe ver A's muis bij B achterloopt; noteer het.
5. **Toetsen.** B typt in een tekstvak bij A, ook met Shift, AltGr-tekens (A's indeling
   bepaalt het teken), pijltjes, Home/End, Ctrl+C/Ctrl+V binnen A's pc, F-toetsen en Alt+F4
   (sluit A's venster, niet B's kijkvenster). F11, D, Delete en een dubbelklik doen in B's
   kijkvenster niets meer zelf. Alt+Tab en de Windows-toets blijven bij B (zo bedoeld).
6. **De eerste klik.** B klikt in een ander programma en daarna terug in zijn kijkvenster: die
   klik haalt alleen het venster naar voren en klikt niets aan bij A. Zolang B's venster de
   focus niet heeft beweegt A's muis niet mee.
7. **Niets blijft hangen.** B houdt Shift (of W) ingedrukt en klikt naar een ander programma:
   bij A is Shift los. B houdt de linkerknop vast in een sleep en drukt Alt+Tab: bij A is de
   knop los. B houdt W vast en trekt de netwerkkabel eruit: na hooguit 1,5 s staat W bij A los.
8. **Stoppen, alle manieren.** Elke keer moet bij A de balk verdwijnen en bij B de titel terug:
   B houdt Escape 1,5 s vast (een korte Escape gaat gewoon naar A); B sluit zijn venster; A
   klikt Stop control; A zet de pijl op de regel in het gesprekspaneel uit; A zet "Allow
   remote control" in Settings uit; A stopt met delen; B valt weg (Tailscale uit of app hard
   dicht).
9. **Eén tegelijk.** B bedient; C vraagt: C krijgt meteen nee (titel terug), A krijgt geen
   tweede vraag. A weigert een vraag van C: C's titel terug. A laat een vraag een minuut liggen:
   hij verdwijnt bij A en B's titel komt terug.
10. **Klembord, alleen tijdens het bedienen.** Vóór Allow kopieert A een tekst: B krijgt hem
    nooit, ook niet bij het toestaan. Tijdens het bedienen: B kopieert bij zich iets en plakt
    met Ctrl+V bij A (het staat er); B kopieert bij A iets (Ctrl+C in A's programma) en plakt bij
    zichzelf. Een tekst van meer dan 64 KiB gaat niet over. Na het stoppen gaat er niets meer
    over, in geen enkele richting. Geen eindeloos heen en weer (kijk of het klembord van A of B
    blijft "knipperen" in een klembordgeschiedenis, Win+V).
11. **Pen en bedienen.** B heeft de pen op en vraagt bedienen: na Allow is de pen neer. Tijdens
    het bedienen tekent slepen niet.
12. **Spel bij A.** A laat een spel lopen terwijl B niets doet: geen merkbare belasting (de
    bedienthread slaapt in `GetMessageW`). Een shooter die de muis vangt: klikken en toetsen
    werken, kijken met de muis niet (absolute muis, bekend). Een programma dat als
    administrator draait (Taakbeheer) neemt B's invoer niet aan (UIPI, bekend).
13. **DPI en monitoren.** A op 150 % met twee monitoren, de gedeelde links van het hoofdscherm:
    B's muis landt op de juiste plek, ook in de hoeken. B's kijkvenster op een andere schaal en
    beeldvullend: hetzelfde.
14. **Oudere peer en Mac.** A op een versie zonder bedienen: B ziet "off". B op een oudere versie
    kijkt naar A: niets verandert, A krijgt geen vraag. Een Mac die deelt laat zich nooit
    bedienen; een Mac die kijkt heeft geen menu om het te vragen.
