# Beveiliging — bevindingen en herstelplan

Volledige beveiligingsdoorlichting van de hele codebase, uitgevoerd op 2026-08-05 op
commit `465f9d0` (versie 0.2.7). Alle zes crates, de frontend, de build en de
afhankelijkheden zijn nagelopen.

Dit document is geschreven als werkdocument, niet als aanklacht. Veel van wat hieronder
staat is een **bewuste** keuze die in `TODO.md` § Beveiliging en `docs/ARCHITECTURE.md`
netjes is opgeschreven. Wat dit onderzoek toevoegt is dit:

1. **De aannames ónder die bewuste keuzes kloppen niet.** "Het tailnet is de
   beveiligingsgrens" is de dragende aanname van het hele model — maar de code luistert
   op álle netwerkinterfaces, en de identiteitscontrole die dat zou moeten opvangen is op
   het inkomende pad onbereikbare code.
2. **De keuzes stapelen.** Los gelezen is elke keuze verdedigbaar. Achter elkaar gezet
   vormen ze een keten van "iemand die de poort kan bereiken" naar "code uitvoeren op
   alle drie de PC's", zonder dat de gebruiker iets hoeft te doen.

> **Leeswijzer.** Wil je alleen weten wat je maandag moet doen: sla door naar
> [Herstelplan](#herstelplan). Wil je weten waaróm het dringend is: lees
> [De keten](#de-keten-die-alles-verbindt). De rest is naslagwerk per bevinding.

---

> **Addendum 2026-08-07 (release-feed, fase 13):** het P2P-updatepad is vervangen door een
> ondertekende release-feed over HTTPS (`crates/app/src/release.rs`, `docs/ARCHITECTURE.md`
> § Automatische updates). Daarmee vervallen **B-01** (de hash kwam van de aanbieder zelf),
> **B-02** (padinjectie via `app_version` — de versie komt nu uit een getekend manifest en
> wordt bovendien tot cijfers en punten begrensd), **B-21** (de draaiende exe wordt nergens
> meer aangeboden) en de wormstap uit [De keten](#de-keten-die-alles-verbindt): een besmette
> peer heeft geen updatekanaal meer naar de andere twee. **B-20** (TOCTOU tussen verifiëren
> en toepassen) staat nog open — er wordt nog steeds niet opnieuw geverifieerd vlak vóór het
> spawnen van de updater. Nieuw eraan: dit is de enige verbinding buiten het tailnet, en de
> release-privésleutel is nu een vertrouwensanker dat kwijt kan raken.

> **Addendum 2026-08-13 (herstelronde uitgevoerd).** Het herstelplan onderaan dit document
> is uitgevoerd, met twee bewuste uitzonderingen. Wat er staat is geïmplementeerd én
> getest: de workspace is groen (219+ tests over `proto`, `store`, `net`, `app`, `audio` en
> `video`, waaronder ~45 nieuwe regressietests die per bevinding zijn genoemd).
>
> **Niet gedaan, en waarom:**
> - **B-05 en B-18** (identiteit cryptografisch aan de verbinding binden; sessiesleutel plus
>   MAC op het mediapad). Beide vereisen een `protocol_version`-bump, en dat is per
>   `CLAUDE.md` invariant 5 een breuk die niet zonder overleg gemaakt wordt: alle drie de
>   peers moeten dan tegelijk bij, anders kan niemand meer verbinden. Er was ook geen manier
>   om een mac↔Windows-handshake te testen. **Dit is de grootste openstaande post** — zolang
>   die er niet is, blijft de firewallregel uit blok 0 de enige echte grens (zie `README.md`).
> - **B-09** is half: er is nu een `bind_address` in `config.toml` met een waarschuwing bij
>   elke start, maar de standaard blijft `0.0.0.0`. Binden aan een tailnet-adres dat er bij
>   het starten nog niet is (Tailscale nog niet omhoog, adres gewijzigd) maakt de app
>   onbereikbaar tot een herstart — precies de stille breuk die invariant 7 verbiedt. Dit is
>   een keuze die je één keer bewust maakt en test, niet iets dat een update onder je
>   vandaan verandert.
>
> **Twee dingen die bij het uitvoeren bleken, en die dit document zelf fout had:**
> 1. **De voorgestelde fix bij B-12 zou élk pariteitsfragment weggooien.**
>    `PARITEIT_PAYLOAD_LEN` is `MAX_MEDIA_PAYLOAD + 2` en de `frag_index` van een
>    pariteitspakket *ís* het aantal, dus de voorgestelde `>=`-controles verwerpen hem
>    precies. Dat zou stilzwijgend de hele herstelweg voor één verloren fragment slopen — de
>    70-156 ms bevriezingen uit `docs/ARCHITECTURE.md`. Pariteit heeft nu eigen maten, met
>    een test die het vastpint. Om dezelfde reden is `MAX_FRAGMENTEN_PER_BEELD` 1024 en niet
>    de voorgestelde 512: 512 × 1100 = 563 kB verwerpt een echt 1440p-keyframe op de 12
>    Mbit-standaard uit `config.toml`.
> 2. **B-08 had een tweede vindplaats die hier niet genoemd stond:**
>    `crates/net/src/filestream.rs::encode_channel` aliaste elke onbekende tag op de vorm van
>    het algemene kanaal, net als `channel_to_blob`. Ook gedicht, met tests die vastleggen dat
>    tag 0/1/2 byte-identiek bleven.
>
> **B-06 is bewust gedeeltelijk.** De afzender wordt nu meegegeven tot in de store en een DM
> namens iemand anders wordt geweigerd, maar een publieke op mág legitiem via een derde peer
> binnenkomen (ARCHITECTURE, "Drie wegen"), dus de `Edit`/`Delete`-kaping op het algemene
> kanaal blijft open. Volledig sluiten kan alleen met een handtekening per op.
>
> Verder open gebleven, met reden in de code: het *decoderen* van een afbeelding (B-22 — dat
> doet WebView2 en daar komen we niet tussen), en de volledige `SocketAddr`-vergelijking van
> B-28 (de deler bindt een efemere poort die niemand aankondigt; er is nu poort-pinning met
> een re-pin-venster in plaats van zwart beeld na een herstart).

> **Addendum 2026-08-13 (1.0.0 t/m 1.2.2):** alle wijzigingen sinds het vorige addendum
> zijn nagelopen (`6ee8197..6bb3aa3`: het zestienkleurenveld-frontend, de geluidsets,
> linkafhandeling, de handmatige update-check en de updater/mediasocket-fixes). Wat het
> beeld verandert:
> - **B-55 is grotendeels gedicht (1.0.2):** de frontend vangt elke klik op een link af en
>   geeft hem via het nieuwe `open_link`-commando aan de systeembrowser. `open_link` laat
>   alleen `http(s)` door en geeft de URL als één argument door (`ShellExecuteW` op
>   Windows, `open` op macOS), dus zonder shell-parsing. Er is nog steeds geen
>   `on_navigation`-beleid aan de Rust-kant — de onderschepping leeft in de webview en dekt
>   alleen gewone klikken — dus als diepteverdediging blijft de bevinding staan.
> - **B-54 is breder geworden:** er zijn vier webview-bereikbare commando's bij
>   (`open_link`, `check_update`, `preview_sound`, `set_sound_settings`). Elk is aan de
>   Rust-kant begrensd (schema-allowlist; het zoekslot van `Updates::mag_zoeken`;
>   naam-allowlists en een volumeclamp inclusief NaN in de motor), dus geen nieuwe ernst.
> - **B-13 is nog half:** het updatepad dwingt de aangekondigde grootte inmiddels wél af
>   (`release.rs::lees_precies` plus `limit`, en het `.part`-bestand wordt bij élke fout
>   opgeruimd); de bestandsoverdracht tussen peers (`engine.rs`) is onveranderd en blijft
>   open.
> - **B-20 staat nog open:** de updater kreeg `start_zonder_handles` (geen handles meer
>   naar het kindproces — hygiëne, geen beveiligingswijziging), maar verifieert nog steeds
>   niet opnieuw vlak vóór het vervangen.
> - **Nieuw: B-60 (LAAG)** — `zoek_updater` accepteert sinds 1.1.0 elk
>   `fitcom-updater*.exe` naast de app als de exacte naam ontbreekt.
> - **Gecontroleerd en goed bevonden:** de CSP heeft de frontend-herbouw overleefd
>   (`tauri.conf.json`); de nieuwe interpolaties zijn schoon (de avatar-klasse is een
>   lokaal berekende `u8`, fouttekst en geluidsnamen gaan door `esc()`); B-56 blijft latent
>   (`voiceHint` rendert nog steeds alleen bij een leeg gesprek); de geluidjes nemen niets
>   van de draad aan (tonen zelf berekend, bestandsnaam een inhoudshash, `afplay` met vaste
>   argumenten); en de erfelijkheids-fixes op de mediasocket en in de updater zijn pure
>   winst — een kindproces krijgt geen socket meer mee.

> **Addendum 2026-08-05 (macOS-port):** dit onderzoek is van vóór de port en de
> bevindingen gelden onverkort — de portlaag voegt geen nieuwe berichten of paden toe.
> Platformscope die verschuift: B-27 (WSAEMSGSIZE) heet op macOS `EMSGSIZE`; B-35
> (`atty`) raakt macOS niet; B-46 (Control Flow Guard) is MSVC-specifiek. De
> `veilige_bestandsnaam`-fix van B-03 moet ook `/` en `:` aankunnen nu er een
> Unix-doel is. B-01/B-02/B-20/B-21 (updatepad) zijn op de mac zelf niet bereikbaar:
> die haalt nooit een exe binnen en biedt de zijne nooit aan (`engine.rs`,
> mac-guards) — voor de Windows-peers verandert er niets.

## Het dreigingsmodel: aanname versus code

Het project rust op vier aannames. Twee ervan maakt de code niet waar.

| # | Aanname | Waar vastgelegd | Maakt de code hem waar? |
|---|---|---|---|
| 1 | Tailscale/WireGuard is de beveiligingsgrens; alleen tailnet-verkeer bereikt de app | `TODO.md:37-40`, `crates/net/src/tls.rs:1-12` | **Nee.** Beide sockets binden op `0.0.0.0` — alle interfaces. Zie B-09. |
| 2 | De UUID-allowlist laat alleen bekende peers binnen | `TODO.md:38-39` | **Nee.** De `PeerId` is een onbewezen claim in een bericht, en de controle erop is inkomend dode code. Zie B-05. |
| 3 | De drie peers zijn vrienden en vertrouwen elkaar | `docs/ARCHITECTURE.md:638-640` | Ja — maar dit is nu ook "hun PC's mogen code op de jouwe draaien". Zie B-01. |
| 4 | Binaries worden met de hand gekopieerd | `crates/proto/src/lib.rs:22-24` | Niet meer waar sinds fase 11: peers duwen elkaar exe's toe. |

**Wat dat praktisch betekent.** Zolang alle drie de machines schoon zijn en er nooit een
vreemde op hetzelfde subnet zit, gebeurt er niets. Zodra één van die twee dingen wél
gebeurt — een gecompromitteerde PC van een vriend, of een avondje gamen op een LAN-party,
in een hotel of op congres-wifi — is er geen tweede verdedigingslaag.

Windows Firewall is op dit moment de énige overgebleven maatregel, en `README.md:60`
vraagt de gebruiker die zelf goed te zetten. De Windows-dialoog biedt "privé" en
"openbaar" als losse vinkjes aan; één verkeerd vinkje opent de app voor het hele subnet.
Beveiliging die afhangt van een vinkje in een dialoog die je één keer wegklikt, is geen
beveiliging.

---

## De keten die alles verbindt

Elke stap hieronder is in de code nagelopen en met regelnummers onderbouwd in de
bevindingen verderop. Geen enkele stap vereist een zwakke plek in Tailscale zelf.

1. **Bereik de poort.** `crates/net/src/mesh.rs:235` bindt op `0.0.0.0`. Op een gedeeld
   netwerk is dat genoeg; op een tailnet volstaat elk ander tailnet-apparaat. *(B-09)*
2. **Vraag de identiteit gewoon op.** `mesh.rs:901-911` stuurt `HelloAck` — met de echte
   `PeerId`, de naam, de mediapoort én het buildnummer — vóórdat er ook maar íéts
   gecontroleerd is. De "credential" is dus gratis op te halen bij het slachtoffer zelf.
   *(B-05a)*
3. **Meld je als die peer bij een ander.** `match_inbound` stap 1 (`mesh.rs:598-604`)
   matcht puur op de geclaimde `PeerId`; het bronadres doet expliciet niet mee. *(B-05)*
4. **Er gaat geen alarm af.** De `IdentityChanged`-controle in `install`
   (`mesh.rs:508-524`) is op het inkomende pad structureel onbereikbaar. *(B-05b)*
5. **Voer code uit.** Nu volstaat één van drie onafhankelijke wegen:
   - een bestandsnaam met `..\` schrijft een exe in de Startup-map — nul klikken *(B-03)*;
   - een versiestring met `..\` doet hetzelfde via het updatepad — nul klikken *(B-02)*;
   - of gewoon netjes een "nieuwere versie" aanbieden, die alleen tegen een door de
     aanvaller zélf meegestuurde hash gecontroleerd wordt — één klik *(B-01)*.

Stap 5 heeft stap 1 t/m 4 niet eens nodig als één van de drie PC's al besmet is. Dat is
precies de wormeigenschap: één besmette machine besmet de andere twee.

---

## Ernstschaal

| Niveau | Betekenis |
|---|---|
| **KRITIEK** | Code-uitvoering of volledige impersonatie, realistisch bereikbaar |
| **HOOG** | Lezen/wijzigen van andermans gegevens, of crash/DoS op afstand zonder interactie |
| **MIDDEL** | DoS met interactie of beperkt bereik, informatielek, ontbrekende diepteverdediging |
| **LAAG** | Hardening; alleen relevant als een andere aanname al gebroken is |
| **INFO** | Bewuste keuze, hier vastgelegd zodat hij bewust blíjft |

---

## Overzicht

60 bevindingen. De ID's zijn stabiel; verwijs ernaar in commits.

| ID | Ernst | Bevinding | Waar |
|---|---|---|---|
| B-01 | ~~KRITIEK~~ | **Opgelost (fase 13)** — updates komen uit een getekende feed | `app/release.rs` |
| B-02 | ~~KRITIEK~~ | **Opgelost (fase 13)** — versie komt uit het getekende manifest en is begrensd | `app/release.rs::controleer` |
| B-03 | ~~KRITIEK~~ | **Opgelost (2026-08-13)** — Padtraversal via `FileMeta.name` → willekeurig bestand schrijven | `app/engine.rs:1631-1632` |
| B-04 | ~~KRITIEK~~ | **Opgelost (2026-08-13)** — Ongevraagde bulkstreams worden geaccepteerd; de afzender wordt weggegooid | `app/engine.rs:537` |
| B-05 | KRITIEK | `PeerId` is een onbewezen claim → volledige impersonatie | `net/mesh.rs:598-604` |
| B-06 | KRITIEK | **Deels (2026-08-13)** — `op.author` wordt nooit tegen de afzender gecontroleerd | `store/lib.rs:240`, `app/chat.rs:293` |
| B-07 | ~~KRITIEK~~ | **Opgelost (2026-08-13)** — Seq-squatting: `INSERT OR IGNORE` + oplopende VV vernietigt stil echte berichten | `store/lib.rs:429-444` |
| B-08 | ~~KRITIEK~~ | **Opgelost (2026-08-13)** — Misvormd `Channel` aliast op de opslagsleutel van het algemene kanaal | `store/lib.rs:499-509` |
| B-09 | HOOG | **Deels (2026-08-13)** — Beide sockets binden op `0.0.0.0`; het tailnet is niet de feitelijke grens | `net/mesh.rs:235`, `net/media.rs:75` |
| B-10 | ~~HOOG~~ | **Opgelost (2026-08-13)** — Onbegrensde Opus-decoders per `stream_id` — 1,2 GB uit 3 MB verkeer | `audio/session.rs:964,1004` |
| B-11 | ~~HOOG~~ | **Opgelost (2026-08-13)** — Eén UDP-pakket zet een videostream permanent vast | `video/fragment.rs:243-250,287` |
| B-12 | ~~HOOG~~ | **Opgelost (2026-08-13)** — Fragmentbuffers: geen cap, geen timeout, en aanvallersbuckets zijn eviction-immuun | `video/fragment.rs:264,297-301` |
| B-13 | ~~HOOG~~ | **Opgelost (2026-08-13)** — Onbegrensde downloadgrootte bij bestandsoverdracht (het updatepad dwingt de grootte sinds fase 13 wél af) | `app/engine.rs` (download_taak) |
| B-14 | ~~HOOG~~ | **Opgelost (2026-08-13)** — `lamport` u64↔i64 → permanente, onherstelbare last-writer-wins-kaping | `store/lib.rs:417-423,437` |
| B-15 | ~~HOOG~~ | **Opgelost (2026-08-13)** — MAX_OP_LEN plus bytebudget in de store, en de schrijflus in `net` slaat over i.p.v. af te breken | `store/lib.rs:63`, `net/mesh.rs:985` |
| B-16 | ~~HOOG~~ | **Opgelost (2026-08-13)** — Onbegrensde oplog-groei; hele store per wijziging opnieuw in RAM | `store/lib.rs:383-390`, `app/chat.rs:243` |
| B-17 | ~~HOOG~~ | **Opgelost (2026-08-13)** — Pre-auth geheugenuitputting: 16 MiB per frame × onbegrensd aantal verbindingen | `net/framing.rs:39`, `net/mesh.rs:865` |
| B-18 | HOOG | Het UDP-mediapad kent geen authenticatie en geen replaybescherming | `net/media.rs:64-156` |
| B-19 | ~~HOOG~~ | **Opgelost (2026-08-13)** — `UpdateSubresource` met genegeerde bufferlengte en gestript stride-teken | `video/codec.rs:741-765` |
| B-52 | ~~HOOG~~ | **Opgelost (2026-08-13)** — `offer_files` accepteert willekeurige paden uit de webview — exfiltratieprimitief | `app/ui/commands.rs:241-246` |
| B-53 | ~~HOOG~~ | **Opgelost (2026-08-13)** — `offer_pasted_image`: extensie is niet begrensd → schrijven buiten `%TEMP%` | `app/ui/commands.rs:255-271` |
| B-20 | ~~MIDDEL~~ | **Opgelost (2026-08-13)** — TOCTOU: de update wordt niet opnieuw geverifieerd vlak vóór toepassen | `app/engine.rs:707-736` |
| B-21 | ~~MIDDEL~~ | **Opgelost (fase 13)** — de eigen exe wordt nergens meer aangeboden | `app/engine.rs` |
| B-22 | MIDDEL | **Deels (2026-08-13)** — Afbeeldingen downloaden en renderen zichzelf, zonder groottegrens | `app/engine.rs:636-658` |
| B-23 | ~~MIDDEL~~ | **Opgelost (2026-08-13)** — Geen kanaalcontrole bij ontvangst — DM's van derden worden opgeslagen | `store/lib.rs:240`, `app/chat.rs:307` |
| B-24 | ~~MIDDEL~~ | **Opgelost (2026-08-13)** — Target-desync laat een peer permanent "online" met een dode verbinding | `net/mesh.rs:658-673` |
| B-25 | ~~MIDDEL~~ | **Opgelost (2026-08-13)** — Logvervuiling vanaf een niet-geautoriseerde verbinding stalt de mesh-actor | `net/mesh.rs:632-640` |
| B-26 | ~~MIDDEL~~ | **Opgelost (2026-08-13)** — Onbegrensde `pending`-lijst voor niet-koppelbare verbindingen | `net/mesh.rs:380,476-480` |
| B-27 | ~~MIDDEL~~ | **Opgelost (2026-08-13)** — Te groot UDP-datagram is een fout i.p.v. rommel → logstorm op de mediathreads | `net/media.rs:153` |
| B-28 | MIDDEL | **Deels (2026-08-13)** — Fragmentinjectie: bronpoort genegeerd en geen index-validatie | `video/kijker.rs:300`, `video/fragment.rs:154` |
| B-29 | ~~MIDDEL~~ | **Opgelost (2026-08-13)** — Geen maximum framegrootte richting de OS-H.264-decoder | `video/kijker.rs:420` |
| B-30 | ~~MIDDEL~~ | **Opgelost (2026-08-13)** — De tijdstempel-unwrapper is onbereikbaar; beeld bevriest na 13 u 15 m | `video/kijker.rs:589-599` |
| B-31 | ~~MIDDEL~~ | **Opgelost (2026-08-13)** — Onbegrensde stream-aankondigingen en afdwingbare encoderbelasting | `app/streams.rs:342-350,372-388` |
| B-32 | ~~MIDDEL~~ | **Opgelost (2026-08-13)** — Logbestanden zonder retentielimiet | `app/main.rs:160-165` |
| B-33 | ~~MIDDEL~~ | **Opgelost (2026-08-13)** — Loginjectie via `display_name` en `app_version` | `net/mesh.rs:557-560` |
| B-34 | ~~MIDDEL~~ | **Opgelost (2026-08-13)** — `seq` u64→i64 levert 2⁶³ permanent inerte maar opgeslagen sleutels | `store/lib.rs:436` |
| B-35 | ~~MIDDEL~~ | **Opgelost (2026-08-13)** — `atty 0.2.14` — RUSTSEC-2021-0145, onbereikbaar maar aanwezig | `Cargo.lock` |
| B-36 | MIDDEL | libopus wordt uit een 2021-snapshot meegebouwd | `audio/Cargo.toml:15` |
| B-54 | MIDDEL | `apply_update` is een ongeconditioneerd IPC-commando | `app/ui/commands.rs:315-318` |
| B-55 | MIDDEL | Geen `on_navigation`-beleid; het klikpad gaat sinds 1.0.2 wél naar de systeembrowser | `app/ui/mod.rs`, `app/ui/commands.rs::open_link` |
| B-56 | ~~MIDDEL~~ | **Opgelost (2026-08-13)** — Latente XSS: escaping is de verantwoordelijkheid van de aanroeper | `frontend/app.js:252-263,531` |
| B-37 | ~~LAAG~~ | **Opgelost (2026-08-13)** — `frag_index + 1` overflow-paniek doodt de kijkerthread stil | `video/fragment.rs:262` |
| B-38 | ~~LAAG~~ | **Opgelost (2026-08-13)** — `volgende + 1` overflow-paniek maakt álle audio permanent stil | `audio/jitter.rs:128-145` |
| B-39 | LAAG | `read_kind` leest elk byte ≠ 1 als "bestand" | `net/filestream.rs:68` |
| B-40 | ~~LAAG~~ | **Opgelost (2026-08-13)** — `VersionMismatch` reset de backoff → handshake-lus van 1 Hz | `net/mesh.rs:823-836` |
| B-41 | ~~LAAG~~ | **Opgelost (2026-08-13)** — Door de peer opgegeven `media_port` wordt niet gevalideerd | `net/mesh.rs:565` |
| B-42 | ~~LAAG~~ | **Opgelost (2026-08-13)** — `wall_clock` is volledig door de afzender bepaald en wordt getoond | `store/timeline.rs:103` |
| B-43 | ~~LAAG~~ | **Opgelost (2026-08-13)** — Onbegrensde string- en collectielengtes op de draad | `proto/op.rs:126-145` |
| B-44 | ~~LAAG~~ | **Opgelost (2026-08-13)** — `unreachable!()` bereikbaar bij extreme naamcollisie | `app/engine.rs:1650` |
| B-45 | ~~LAAG~~ | **Opgelost (2026-08-13)** — `data.len() as u32` truncatie vlak vóór een volledige memcpy | `video/codec.rs:593` |
| B-46 | ~~LAAG~~ | **Opgelost (2026-08-13)** — Release-profiel: geen CFG, geen `overflow-checks`, geen `strip` | `Cargo.toml:18-21` |
| B-47 | ~~LAAG~~ | **Opgelost (2026-08-13)** — Portable modus zet schrijfbare data naast een zelf-overschrijvende exe | `app/config.rs:230-236` |
| B-48 | ~~LAAG~~ | **Opgelost (2026-08-13)** — Devtools-capability zonder buildconditie | `app/capabilities/default.json:13` |
| B-49 | ~~LAAG~~ | **Opgelost (2026-08-13)** — Mention-vervanging loopt ná linkificatie en breekt gegenereerde HTML | `frontend/app.js:162-172` |
| B-50 | ~~LAAG~~ | **Opgelost (2026-08-13)** — `Spoor` groeit onbegrensd | `video/spoor.rs:31-33` |
| B-57 | ~~LAAG~~ | **Opgelost (2026-08-13)** — Meldingen schakelen zichzelf permanent uit na één fout | `app/notify.rs:16,38-41` |
| B-58 | ~~LAAG~~ | **Opgelost (2026-08-13)** — `highlight()` verminkt ge-escapete apostrofs | `frontend/app.js:178-187` |
| B-59 | ~~LAAG~~ | **Opgelost (2026-08-13)** — `unwrap()` op een mutex in de `thumb://`-handler | `app/ui/mod.rs:168` |
| B-60 | LAAG | Elk `fitcom-updater*.exe` naast de app telt als de updater | `app/engine.rs::zoek_updater` |
| B-51 | ~~INFO~~ | **Opgelost (2026-08-13)** — `LamportClock` is dode code; de echte klok is `max_lamport()` | `proto/op.rs:150-172` |

---

# KRITIEK

## B-01 — De update-binary heeft geen authenticiteit

**Waar:** `crates/app/src/engine.rs:1914-1932` (verzender), `crates/app/src/updates.rs:115-121` en
`crates/app/src/engine.rs:2055` (ontvanger).

De hash waartegen de gedownloade exe gecontroleerd wordt, komt uit `UpdateResponse.hash` —
verstuurd door dezelfde peer die ook de bytes levert:

```rust
// updates.rs:115-121 — wat er binnenkwam is nu "de verwachte hash"
self.huidig = Some(UpdateStatus::Bezig { peer: *peer, hun_versie: hun_versie.clone(),
                                         ontvangen: 0, totaal: resp.size, hash: resp.hash });
// engine.rs:2055
let klopt = hash == verwachte_hash;
```

BLAKE3 bewijst hier dat de QUIC-stream de bytes niet verminkt heeft. Over wie ze geschreven
heeft bewijst het niets. Er is geen handtekening, geen sleutel, geen pinning, en de exe is
ook niet Authenticode-getekend (`tauri.conf.json` heeft `bundle.active: false` en er is geen
CI). De verificateur en de aanvaller zijn dezelfde partij.

**Scenario.** Een peer (of wie zich als peer voordoet, B-05) meldt versie `9.9.9`, stuurt een
willekeurige exe plus de bijbehorende hash, en de gebruiker ziet een normaal ogend
updatevenster. Eén klik en `fitcom-updater.exe` schrijft die exe over `fitcom.exe` heen en
start hem (`bin/fitcom-updater.rs:91-98`, `:59`).

**Dit staat als bewuste keuze in `docs/ARCHITECTURE.md:638-640`.** Het staat hier toch,
omdat de onderbouwing ("het tailnet + de UUID-allowlist") niet overeind blijft: B-05 laat
zien dat de allowlist niet afdwingbaar is, en B-02/B-03 laten zien dat de klik niet eens
nodig is. Wat als "wij vertrouwen onze twee vrienden" bedoeld was, is in de praktijk
"iedereen die de poort haalt, en zonder dat iemand het merkt".

**Oplossing.** Een offline Ed25519-releasesleutel, publieke helft in de binary gebakken:

```rust
// proto/src/control.rs — additief, conform de protocolregels
pub struct UpdateResponse {
    pub outcome: FileOutcome,
    pub size: u64,
    pub hash: [u8; 32],
    #[serde(default)]
    pub signature: Vec<u8>,   // Ed25519 over (canonieke versie || size || hash)
}
```

Verifieer vóór het wegschrijven én opnieuw vlak vóór het spawnen van de updater (B-20).
Een lege handtekening is een harde weigering. Dit raakt invariant 1 niet: de sleutel zit in
de binary, er wordt niets opgehaald.

Kan tekenen echt niet, dan is de ondergrens: **niet automatisch downloaden**. Vraag
bevestiging vóór de overdracht en toon in dat venster de bronpeer én de hash.

## B-02 — Padinjectie via `app_version`

**Waar:** `crates/app/src/engine.rs:669`, `:2017`, `:2062`; parser in `crates/proto/src/appversion.rs:9-16`.

De versiestring van een peer gaat rechtstreeks een bestandsnaam in:

```rust
// engine.rs:2017 en :2062
let deelpad   = updates_dir.join(format!("update-{versie}.exe.part"));
let definitief = updates_dir.join(format!("update-{versie}.exe"));
```

En de enige poort die hij passeert, `is_newer`, laat rommel achter de cijfers ongemoeid:

```rust
// appversion.rs:9-15
let mut delen = versie.split('.').map(|d| d.parse::<u64>().unwrap_or(0));
```

`"9.9.9\..\..\..\Startup\evil"` splitst op `.` in `["9", "9", "9\", …]`; het derde deel
faalt en wordt `0`. De tuple is dus `(9, 9, 0)` — ruim nieuwer dan `(0, 2, 7)` — terwijl de
**hele** string, backslashes en al, in de `format!` belandt.

Anders dan bij B-03 zit hier geen `.exists()`-rem op: `OpenOptions::create(true).append(true)`
(`engine.rs:2018`) schrijft door in een bestaand bestand en `rename` (`:2063`) overschrijft.

**Scenario — drie berichten, nul klikken.** `Hello` met die versiestring → `overweeg_update`
vuurt automatisch op `Online` (`engine.rs:501`) → `UpdateResponse` + uni-stream → de bytes
landen in de Startup-map. Code-uitvoering bij de volgende aanmelding. Het enige wat de
gebruiker ziet is een updatemelding die hij niet hoeft aan te raken.

**Oplossing.** Valideer de vorm bij het parsen en laat alleen de gecanoniseerde tuple een pad
in:

```rust
pub fn parse_strict(versie: &str) -> Option<(u64, u64, u64)> {
    let mut d = versie.split('.');
    let (a, b, c) = (d.next()?, d.next()?, d.next()?);
    if d.next().is_some() { return None; }
    Some((a.parse().ok()?, b.parse().ok()?, c.parse().ok()?))
}
```

Weiger in `overweeg_update` alles wat daar niet doorheen komt, en bouw de bestandsnaam uit
`format!("{}.{}.{}", v.0, v.1, v.2)` — dan kunnen er per constructie alleen cijfers en punten
in een pad terechtkomen. Valideer bij voorkeur al in `net`, zodat de string de app niet eens
binnenkomt.

## B-03 — Padtraversal via `FileMeta.name`

**Waar:** `crates/app/src/engine.rs:1631-1632`, aangeroepen op `:2142`.

```rust
fn unieke_bestandsnaam(dir: &Path, naam: &str) -> PathBuf {
    let kandidaat = dir.join(naam);
```

`naam` is `FileEntry.name`, letterlijk overgenomen uit de op van de peer
(`store/timeline.rs:144-157`). Er is in de hele repo geen sanitisatie: de enige
`file_name()`-aanroep (`engine.rs:1671`) zit op het *aanbied*pad, op een lokaal gekozen bestand.

`Path::join` normaliseert `..` niet, en op Windows vervangt een absoluut of rooted argument de
basis volledig. Dus `..\..\..\Startup\x.exe`, `C:\Users\...\Startup\x.exe` en `\\aanvaller\share\x`
werken alle drie.

**Wat het beperkt, eerlijk benoemd.** De `.exists()`-check op `:1633` voorkomt het
*overschrijven* van een bestaand bestand; een nieuw bestand op een gekozen pad neerzetten kan
onbeperkt, en dat is genoeg voor de Startup-map. Het **afbeeldingspad is wél veilig**:
`hash_bestandsnaam` (`files.rs:197-206`) bouwt `<hex>.<ext>` uit `Path::extension()`, dat op
`file_name()` werkt en dus nooit een separator kan bevatten, en `is_afbeelding` beperkt de
extensie tot vijf bekende waarden. Alleen de niet-afbeeldingstak is het gat.

**Oplossing.** Eén gedeelde helper, aangeroepen op het punt waar `FileEntry` gebouwd wordt
(`store/timeline.rs`), zodat elke consument hem erft:

```rust
fn veilige_bestandsnaam(naam: &str) -> String {
    let kaal = Path::new(naam).file_name().and_then(|n| n.to_str()).unwrap_or("");
    let kaal: String = kaal.chars()
        .filter(|c| !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
                    && !c.is_control())
        .collect();
    let kaal = kaal.trim_matches(|c: char| c == '.' || c == ' ');
    const GERESERVEERD: &[&str] = &["CON","PRN","AUX","NUL",
        "COM1","COM2","COM3","COM4","COM5","COM6","COM7","COM8","COM9",
        "LPT1","LPT2","LPT3","LPT4","LPT5","LPT6","LPT7","LPT8","LPT9"];
    let stam = kaal.split('.').next().unwrap_or("").to_ascii_uppercase();
    if kaal.is_empty() || GERESERVEERD.contains(&stam.as_str()) { return "bestand".into(); }
    kaal.chars().take(120).collect()
}
```

En controleer na afloop dat het resultaat nog steeds ónder de doelmap ligt
(`debug_assert!(definitief.starts_with(downloads_dir))`).

## B-04 — Ongevraagde bulkstreams worden geaccepteerd

**Waar:** `crates/app/src/engine.rs:537`, `crates/app/src/engine.rs:1858-1869`.

```rust
// engine.rs:537 — de afzender wordt expliciet weggegooid
MeshEvent::IncomingFileStream { from: _, stream } => { self.start_incoming_stream(stream); }
```

De enige controle daarna is "bestaat er een op met deze `OpId`". `Files::downloads` — de
boekhouding van wat wíj gevraagd hebben (`files.rs:45`) — wordt nooit geraadpleegd, en
`entry.author` wordt niet tegen de afzender gelegd. Voor een *update*stream is het nog losser:
die draagt helemaal geen body (`filestream.rs:53-59`), en de enige poort is één globale
"verwachten we een update" (`engine.rs:1813`), nooit "van wie".

Dit is wat B-03 van "één klik" naar "nul klikken" brengt: de aanvaller kiest gewoon de
niet-afbeeldingstak en de gebruiker klikt nergens op. Daarnaast kunnen twee streams in
hetzelfde `.part`-bestand schrijven (`engine.rs:2018-2022` opent met `append(true)`), wat een
betrouwbare download-DoS oplevert.

**Oplossing.** Geef `from` door en eis een openstaande aanvraag bij de juiste peer:

```rust
MeshEvent::IncomingFileStream { from, stream } => self.start_incoming_stream(from, stream),
// en in download_taak:
if entry.author != from { return; }
if !matches!(self.files.status(file), Some(DownloadStatus::Bezig { .. })) { return; }
// voor een update-stream: eis dat UpdateStatus::Bezig.peer == from
```

Gebruik daarnaast `create_new(true)` of een unieke tijdelijke naam per overdracht, zodat twee
streams nooit in hetzelfde bestand kunnen schrijven.

## B-05 — `PeerId` is een onbewezen claim

**Waar:** `crates/net/src/mesh.rs:598-604` (matchen), `:884-926` (accepteren), `:508-536`
(installeren), `crates/net/src/tls.rs:52,67`.

Niets koppelt een `PeerId` aan een verbinding. TLS accepteert elk certificaat
(`tls.rs:67`) en vraagt niet om een clientcertificaat (`tls.rs:52`), dus de identiteit is
puur een veld in een msgpack-bericht. En de eerste matchregel kijkt uitsluitend naar dat veld:

```rust
// mesh.rs:597-604 — de comment zegt het zelf
// 1. Kennen we deze identiteit al, dan is het adres niet interessant.
if let Some(i) = self.targets.iter().position(|t| t.known_id == Some(e.peer_id)) {
    return Some(i);
}
```

Twee deelproblemen maken het compleet:

**B-05a — de "credential" is gratis op te halen.** `accept_one` schrijft de `HelloAck` — met
de echte `PeerId`, de weergavenaam, de mediapoort én het buildnummer — *vóór* de
protocolversiecheck (`:913`) en de zelf-check (`:923`):

```rust
// mesh.rs:901-911, en pas daarna de controles
framing::write_frame(&mut send, &ControlMsg::HelloAck(HelloAck {
    protocol_version: PROTOCOL_VERSION, peer_id: cfg.me,
    display_name: cfg.display_name.clone(), media_port: cfg.media_port,
    app_version: cfg.app_version.clone(),
})).await?;
```

**B-05b — het alarm is dode code.** De `IdentityChanged`-tak in `install` (`:509-524`) kan via
het inkomende pad nooit vuren: matchregel 1 selecteert juist op gelijkheid, en regel 2 en 3
filteren op `known_id.is_none()` (`:606`), zodat altijd de TOFU-tak (`:526`) loopt — die de
geclaimde identiteit vervolgens *vastlegt*. Alleen een uitgaande dial kan het alarm nog
afgeven. De ene controle die de UI als identiteitswaarborg presenteert, doet niets op het pad
dat een aanvaller gebruikt.

**Scenario.** Haal de `PeerId` van A op met één willekeurige `Hello` (B-05a), meld je met die
`PeerId` bij B vanaf een willekeurig adres (regel 1), en je bent A — zonder alarm (B-05b).
Daarna: alle chat-sync inclusief A's DM's, berichten vervalsen, bestanden aanbieden, en via
`Hello.app_version` de updateflow starten.

En het is niet alleen lezen: de botsingsregel (`:545-552`) vergelijkt de *geclaimde* initiator,
dus zodra A's UUID lager sorteert dan de onze wint de vervalste verbinding en wordt de echte
gesloten — op commando, herhaalbaar.

**Oplossing.** Bind de identiteit cryptografisch aan de verbinding. Minimale vorm, met behoud
van de huidige structuur: geef elke installatie een vaste Ed25519-sleutel, laat `Hello` de
publieke sleutel plus een handtekening over `export_keying_material(b"fitcom-bind", …)` dragen,
en eis `PeerId == blake3(pubkey)`. Die channel-binding is wat replay van een opgevangen `Hello`
op een nieuwe verbinding onmogelijk maakt.

Kleinere tussenstap die sowieso zou moeten landen: pin bij TOFU het SPKI-hash van het
certificaat naast `known_id`, en eis in matchregel 1 óók dat `e.remote.ip()` klopt.

## B-06 — `op.author` wordt nooit tegen de afzender gecontroleerd

**Waar:** `crates/store/src/lib.rs:240-274`, bereikt vanuit `crates/app/src/chat.rs:284-296`.

`neem_over(van, ops)` krijgt de geauthenticeerde afzender binnen en gebruikt hem uitsluitend
voor een logregel (`chat.rs:316`). `apply_remote(&Op)` kent helemaal geen afzenderparameter,
dus er ís geen laag waar `op.author == van` gecontroleerd zou kunnen worden.

Erger: de auteurscontrole op `Edit`/`Delete` vergelijkt twee velden die de aanvaller allebei
zelf kiest:

```rust
// timeline.rs:113
if target.author != op.author { continue; }
```

Zet beide op A en je mag A's berichten wijzigen en verwijderen, mesh-breed en onomkeerbaar
(de oplog is append-only). Hetzelfde geldt voor `SetNick` namens A.

**Oplossing.** Geef de afzender mee tot in de store en weiger bij een mismatch. Omdat publieke
ops legitiem via een derde peer doorgestuurd worden (ARCHITECTURE, "Drie wegen"), is de eerlijke
regel: `op.author != afzender` mag alleen als `op.channel.is_public()`, en nooit voor een DM.
Volledig sluiten kan alleen met handtekeningen per op — zie het slot van dit document.

## B-07 — Seq-squatting vernietigt stil echte berichten

**Waar:** `crates/store/src/lib.rs:429-444` (`insert_op`) en `:447-480` (`advance_contiguous`).

De primaire sleutel is `(author, channel, seq)` — met B-06 alle drie door de aanvaller te
kiezen — en de insert is `INSERT OR IGNORE`. Een latere, échte op met dezelfde sleutel wordt
dus stilzwijgend weggegooid: `insert_op` geeft `false`, `apply_remote` meldt "hadden we al",
en de op wordt niet opgeslagen, niet getoond en niet doorgestuurd.

Daar bovenop schuift `advance_contiguous` de teller over de vervalste rij heen, zodat
`version_vector()` naar waarheid meldt "ik heb A's op N". A stuurt hem dus nooit opnieuw:
`ranges_missing_in` ziet niets ontbreken.

**Scenario.** Bij het eerste contact met een verse peer B injecteert C
`author=A, channel=GENERAL, seq=1..10000`. Elk van A's volgende 10 000 algemene berichten
wordt bij B permanent en geruisloos opgeslokt. Geen foutpad, geen logregel, geen UI-signaal.
De convergentietests kunnen dit niet vangen omdat geen enkele test een peer modelleert die
over auteurschap liegt.

**Oplossing.** B-06 fixen haalt de hefboom weg. Maak daarnaast een sleutelbotsing met
*afwijkende inhoud* luidruchtig: lees de bestaande rij vóór de insert en log een fout als de
payload verschilt. Een echt duplicaat is byte-identiek, dus dat geeft geen valse meldingen.

## B-08 — Misvormd `Channel` aliast op het algemene kanaal

**Waar:** `crates/proto/src/ids.rs:84-93` en `crates/store/src/lib.rs:499-509`.

`Channel` heeft een afgeleide `Deserialize` over een rauwe `tag: u8`. De velden zijn privé, dus
*Rust*-code kan geen inconsistente waarde bouwen — serde wel, en de aanvaller schrijft de
msgpack zelf. `{tag:1, peer:null}`, `{tag:0, peer:<uuid>}` en `{tag:3}` deserialiseren allemaal
schoon.

De opslagencoder is niet totaal over die invoerruimte:

```rust
// lib.rs:499-509 — alles wat geen herkenbare DM of topic is, valt door naar nullen
fn channel_to_blob(channel: Channel) -> [u8; 17] {
    let mut buf = [0u8; 17];
    if let Some(p) = channel.dm_peer() { buf[0] = 1; buf[1..].copy_from_slice(p.as_bytes()); }
    else if let Some(t) = channel.topic_id() { buf[0] = 2; buf[1..].copy_from_slice(t.as_bytes()); }
    buf
}
```

`dm_peer()` geeft alleen `Some` bij `tag == 1` **én** `peer.is_some()` (`ids.rs:127-129`). Elke
andere vorm levert de all-zero blob op — precies de blob van `Channel::GENERAL`.

Dit is exact de botsing die `docs/ARCHITECTURE.md` bij de 2→3-bump als "permanent dataverlies"
beschrijft en afgedaan acht. **De versiebump sluit alleen de versieverschil-route.** Een peer
op dezelfde protocolversie kan gewoon een vorm sturen die geen enkele eerlijke encoder
produceert.

**Scenario.** `author=A, channel={tag:1, peer:null}, seq=N`. `is_public()` is false, dus hij
wordt niet doorgestuurd en `visible_to` verbergt hem — maar hij landt op de *algemene*
sleutel `(A, nullen, N)` en schuift de algemene teller op. A's echte algemene op N is daarna
permanent onopslaanbaar. Bij de volgende `all_ops()` leest `channel_from_blob` hem bovendien
terug als `Channel::GENERAL` en verschijnt hij alsnog in het algemene kanaal.

**Oplossing.** Twee kleine wijzigingen. (1) Geef `Channel` een validerende `Deserialize`
(`#[serde(try_from = …)]`): `tag == 1` zonder `peer` is een decodeerfout, en een onbekende tag
normaliseert naar één gereserveerde vorm zodat gelijk-ogende waarden ook gelijk vergelijken.
(2) Maak `channel_to_blob` totaal en injectief door de rauwe tag altijd weg te schrijven
(`buf[0] = channel.raw_tag()`), zodat een onbekende tag zijn eigen sleutelruimte krijgt in
plaats van te botsen met het algemene kanaal.

---

# HOOG

## B-09 — Beide sockets binden op `0.0.0.0`

**Waar:** `crates/net/src/mesh.rs:235`, `crates/net/src/media.rs:75`.

```rust
let bind: SocketAddr = format!("0.0.0.0:{}", cfg.control_port)   // mesh.rs:235
let sock = UdpSocket::bind(("0.0.0.0", port))                     // media.rs:75
```

Dit is de bevinding die alle andere hun bereik geeft. `tls.rs:1-12` en `TODO.md` leggen het
hele vertrouwensmodel bij het tailnet, maar geen van beide sockets is aan de
Tailscale-interface gebonden en nergens wordt een bronadres tegen de geconfigureerde
targets gelegd vóórdat er werk gedaan wordt. Alles in dit document is dus ook bereikbaar
vanaf het LAN, vanaf hotel- of congreswifi, en vanaf internet als een router de poort
doorstuurt of UPnP hem openzet.

**Oplossing.** Bind aan het Tailscale-adres (het `100.64.0.0/10`-adres van de machine, of
een configwaarde), en weiger daarnaast vroeg:

```rust
// accept_loop, vóór het spawnen van accept_one
if !cfg.targets_resolved_ips().contains(&incoming.remote_address().ip()) {
    incoming.refuse();   // geen handshake, geen taak, geen allocatie
    continue;
}
```

De adreslijst alleen is niet genoeg (een aanvaller op het adres van een peer komt er nog
steeds door) — dit hoort samen met B-05 te landen.

## B-10 — Onbegrensde Opus-decoders per `stream_id`

**Waar:** `crates/audio/src/session.rs:964` en `:1004-1013`.

De ontvangthread gebruikt de rauwe 32-bits `stream_id` van de draad als sleutel, zonder
validatie en zonder plafond. De mixthread maakt daar vervolgens onvoorwaardelijk een
Opus-decoder bij — nog vóórdat het frame bekeken wordt.

Eén pakket van 17 bytes per `stream_id` levert één `JitterBuffer` én één decoder (~18 kB
state) op. 65 536 stream-id's is ~2,9 MB verkeer tegen ~1,18 GB geheugen: een factor 400.
`stream_id` is een `u32`, dus het proces gaat OOM lang voordat het plafond in zicht komt.

Twee bijkomende effecten: de mixthread itereert elke ~20 ms over álle buffers terwijl hij
de lock vasthoudt (`session.rs:995-998`), dus bij 65 000 buffers haalt hij zijn frameperiode
niet meer en valt de spraak weg — precies wat invariant 4 verbiedt. En `decoders` is een
thread-lokale map die nooit opgeschoond wordt: de allocatie blijft staan nadat de aanval
gestopt is.

**Oplossing.** Begrens het aantal bronnen per peer, en snoei de decodermap:

```rust
const MAX_BRONNEN_PER_PEER: usize = 8;
let sleutel = (peer, header.stream_id);
if !j.contains_key(&sleutel)
    && j.keys().filter(|(p, _)| *p == peer).count() >= MAX_BRONNEN_PER_PEER {
    tracing::warn!(peer = ?peer, "te veel audiobronnen van deze peer; genegeerd");
    continue;
}
```

Beter nog: accepteer alleen `stream_id`'s die `Streams` daadwerkelijk in een
`StreamAnnounce` van die peer gezien heeft. Voeg `decoders.retain(…)` toe per iteratie.

## B-11 — Eén pakket zet een videostream permanent vast

**Waar:** `crates/video/src/fragment.rs:243-250` en `:287`.

```rust
if let Some(l) = self.laatste {
    if header.timestamp <= l { … return None; }
}
…
self.laatste = Some(header.timestamp);
```

Een enkel-fragment-beeld is binnen één `push` compleet: `frag_index = 0` met
`FLAG_LAST_FRAGMENT` zet `aantal = Some(1)`, de insert maakt `stukken.len() == 1`, dus
`compleet()` is waar en `laatste` wordt gezet.

**Scenario.** Stuur één pakket met `timestamp = 0xFFFFFFFF` en `FLAG_LAST_FRAGMENT`.
`laatste` wordt `u32::MAX`, en daarna faalt **elk** legitiem pakket op `timestamp <= l`.
Het beeld bevriest, de kijker vraagt elke 500 ms tevergeefs een keyframe, en er is nergens
een pad dat `laatste` terugzet: alleen het venster sluiten en heropenen helpt. Kosten voor
de aanvaller: 45 bytes als de poort bekend is, ~2,9 MB om alle poorten af te gaan (de
bronpoort wordt niet gecontroleerd, zie B-28).

**Oplossing.** Behandel een grote sprong — vooruit óf achteruit — als hersynchronisatie in
plaats van als ordeschending:

```rust
fn is_nieuwer(ts: u32, laatste: u32) -> bool {
    let d = ts.wrapping_sub(laatste);
    d != 0 && d < u32::MAX / 2
}
```

Bij een sprong buiten het redelijke: `self.onderweg.clear(); self.laatste = None;`.
Dezelfde vergelijking repareert ook B-30.

## B-12 — Fragmentbuffers zonder cap, zonder timeout, met omgekeerde eviction

**Waar:** `crates/video/src/fragment.rs:264`, `:297-301`, `:154-157`.

Drie problemen die elkaar versterken:

**(a) Geen bovengrens per beeld.** `deel.stukken.insert(header.frag_index, …)` met
`frag_index: u16` en een payload tot `MAX_PAKKET - MEDIA_HEADER_LEN` = 1484 bytes (ruim
boven de `MAX_MEDIA_PAYLOAD` van 1100, die aan ontvangstzijde nergens afgedwongen wordt).
65 536 × ~1520 bytes ≈ **99,6 MB per bucket**.

**(b) Geen tijdslimiet.** Een bucket wordt alleen opgeruimd als een beeld compleet raakt of
als er een 9e tijdstempel binnenkomt. Laat `FLAG_LAST_FRAGMENT` én `FLAG_PARITEIT` weg en
hij blijft eeuwig staan. 8 buckets × 99,6 MB ≈ **797 MB**.

**(c) De eviction gooit de verkeerde weg.**

```rust
while self.onderweg.len() > MAX_ONDERWEG {
    let oudste = *self.onderweg.keys().next().expect("niet leeg");   // = laagste sleutel
    self.onderweg.remove(&oudste);
```

Parkeer 8 onvolledige buckets op `0xFFFFFFF8..=0xFFFFFFFF` en elk écht beeld — dat een
lagere tijdstempel heeft — wordt bij het volgende aanvallerspakket weggegooid. Een druppel
van ~100 pps zorgt dat geen enkel legitiem beeld ooit nog compleet wordt, en anders dan
B-11 blijft `laatste` ongemoeid, dus in het log valt alleen een oplopende `incompleet` op.

**Oplossing.** Valideer bij binnenkomst en verval op leeftijd in plaats van op sleutel:

```rust
const MAX_FRAGMENTEN_PER_BEELD: u16 = 512;   // 512 × 1100 B ≈ 563 kB, ruim boven een keyframe
if payload.len() > MAX_MEDIA_PAYLOAD || header.frag_index >= MAX_FRAGMENTEN_PER_BEELD {
    self.verworpen += 1;
    return None;
}
if let Some(n) = deel.aantal {
    if header.frag_index >= n { self.verworpen += 1; return None; }
}
```

Zet een `gezien: Instant` op `Halffabrikaat` en gooi buckets ouder dan ~500 ms weg. Dit
lost meteen B-37, B-29 en B-51 mee op.

## B-13 — Onbegrensde downloadgrootte

**Waar:** `crates/app/src/engine.rs:2030-2047` (update) en `:2092-2114` (bestand).

> **Update 2026-08-13:** de update-helft is vervallen — het updatepad loopt sinds fase 13
> via `release.rs`, en dat pad dwingt de aangekondigde grootte af (`limit` op de body plus
> `lees_precies`, dat precies `size` bytes telt) en ruimt het `.part`-bestand bij élke
> fout op. De bestands-helft hieronder staat nog open.

Beide lussen lezen tot EOF en schrijven weg zonder plafond. `entry.size` en
`UpdateResponse.size` worden opgeslagen en getoond, maar nooit vergeleken met wat er
werkelijk binnenkomt. Er is geen quotum en geen vrije-ruimtecheck. Het `.part`-bestand wordt
alleen opgeruimd bij een hash-*mismatch*, en die treedt nooit op als de aanvaller de stream
simpelweg nooit sluit: de taak blijft in `read` hangen en het bestand blijft staan.

Via B-02 is dit ook zonder enige klik bereikbaar.

**Oplossing.**

```rust
Some(n) => {
    ontvangen += n as u64;
    if ontvangen > verwacht_totaal {
        drop(bestand);
        let _ = tokio::fs::remove_file(&deelpad).await;
        anyhow::bail!("peer stuurde meer dan aangekondigd ({ontvangen} > {verwacht_totaal})");
    }
    bestand.write_all(&buf[..n]).await?;
}
```

Plus een absoluut plafond voor updates (`const MAX_UPDATE: u64 = 256 * 1024 * 1024;`) bij
`antwoord_ontvangen`, en het `.part`-bestand opruimen bij *elke* fout, niet alleen bij een
mismatch.

## B-14 — `lamport` u64↔i64 geeft een permanente LWW-kaping

**Waar:** `crates/store/src/lib.rs:417-423`, `:227`, `:437`; vergelijking in
`crates/store/src/timeline.rs:63-65`.

De lokale klok is geen `LamportClock` (dat is dode code, B-51) maar een SQL-`MAX`:

```rust
r.get::<_, i64>(0)          // lib.rs:421
…
op.lamport as i64,          // lib.rs:437
```

SQLite kent geen `u64`. Een `lamport` van `u64::MAX` wordt opgeslagen als `-1`, dus
`MAX(lamport)` ziet hem nooit: **de eerlijke klok kan nooit meer inlopen.** Maar
`timeline::build` vergelijkt in `u64`, waar `u64::MAX` élke last-writer-wins-vergelijking
wint, op elke peer, voorgoed.

**Ergste geval — `DeleteTopic` heeft geen auteurscontrole** (`timeline.rs:171-179`, bewust:
elke peer mag een subkanaal beheren). Met `lamport = u64::MAX` kan iedere peer elk
subkanaal onherroepelijk vernietigen. Het gedocumenteerde herstel ("een latere
`SetTopicTitle` wint alsnog") is onbereikbaar, want niets komt boven `u64::MAX`.

**Tweede variant, zelfs zonder kwade opzet gevaarlijk:** `lamport = i64::MAX` is wél
positief, dus die wordt opgepikt. Dan is `max_lamport() + 1` gelijk aan 2⁶³, dat als
`i64::MIN` opgeslagen wordt — waarna `MAX(lamport)` op `i64::MAX` blijft staan en élke
volgende eigen op exact dezelfde lamport krijgt. Je eigen tijdlijn is daarna niet meer
ordenbaar. Eén bericht, permanent.

**Oplossing.** Weiger bij het decoderen `lamport > i64::MAX as u64`, en begrens de sprong:
`op.lamport > lokaal_max + (1 << 32)` is in een mesh van drie peers nooit legitiem. Sla
daarnaast op wat je vergelijkt — of `lamport` end-to-end als `i64`, of als zero-padded
big-endian blob zodat SQL en Rust het eens zijn.

## B-15 — Eén te grote op sloopt de control-verbinding permanent

**Waar:** `crates/proto/src/lib.rs:65`, `crates/store/src/lib.rs:63`,
`crates/net/src/framing.rs:14`, `crates/net/src/mesh.rs:985-988`.

`MAX_FRAME_LEN` begrenst een *frame*; niets begrenst een enkele *op*, en `SYNC_BATCH = 500`
is een aantal, geen bytebudget. De invariant "wat ik kan ontvangen, kan ik doorsturen" is
daarmee gebroken.

Een aanvaller stuurt één `Post` van net onder 16 MiB. Die wordt geaccepteerd en opgeslagen.
Bij het doorsturen — of bij de volgende sync-batch van 500 — overschrijdt het frame de
limiet, `write_frame` geeft een fout, en de schrijftaak **stopt**:

```rust
if let Err(e) = framing::write_frame(&mut send, &msg).await {
    tracing::debug!(…); break;          // mesh.rs:985-988
}
```

Bij herverbinding bouwt `beantwoord_sync` dezelfde batch opnieuw op en sneuvelt de
verbinding opnieuw. Permanent verlies van alle control-connectiviteit, alleen te herstellen
door de database te verwijderen.

**Oplossing.** Begrens de op zelf (`MAX_OP_LEN`, bijv. 256 KiB) bij `apply_remote`, laat
`ops_missing_in_for` op bytes budgetteren, en laat een mislukt bericht overgeslagen worden
in plaats van de lus te breken.

## B-16 — Onbegrensde oplog-groei, en de hele store per wijziging in RAM

**Waar:** `crates/store/src/lib.rs:246-274`, `:383-390`; `crates/app/src/chat.rs:240-263`.

Er is geen limiet op ops per peer, geen sanity-grens op `seq`, geen payloadplafond en geen
rate limit. Ops met een gat ervoor worden opgeslagen maar tellen nooit mee, dus een
aanvaller kan onbeperkt sleutels vullen die nooit opgeruimd worden — er is nergens een
verwijderpad.

Twee versterkers: `Chat::refresh` roept `store.timeline()` aan na *elke* wijziging, en dat
is `all_ops()` zonder `LIMIT` — de hele opgeblazen log wordt bij elk binnenkomend bericht
opnieuw ingeladen en gesorteerd. En `beantwoord_sync` verzamelt álle batches in `uit`
vóórdat hij teruggeeft, dus één lege `SyncRequest` materialiseert de complete zichtbare
store in één keer in het geheugen.

**Oplossing.** Begrens per `(author, channel)` het venster voorbij de aaneengesloten
frontier (bijv. 1000; herordening heeft nooit meer nodig) en weiger `seq` daarbuiten.
Stream de sync-batches in plaats van ze te bufferen, en pagineer `all_ops()`/`timeline()`.

## B-17 — Pre-auth geheugenuitputting

**Waar:** `crates/net/src/framing.rs:34-43`, `crates/proto/src/lib.rs:65`,
`crates/net/src/mesh.rs:859-882`, `crates/net/src/tls.rs:30-39`.

```rust
let len = u32::from_be_bytes(len_buf) as usize;
if len > MAX_FRAME_LEN { bail!(…); }
let mut body = vec![0u8; len];        // 16 MiB, genulld, dus echt vastgelegd
```

De allocatie gebeurt op de aangekondigde lengte, vóórdat er één byte body binnen is — en
dit is dezelfde functie die de eerste, nog niet-geauthenticeerde `Hello` leest. Drie
vermenigvuldigers stapelen: er is **geen verbindingslimiet** (geen `concurrent_connections`,
geen per-IP-limiet, `refuse()` wordt nooit gebruikt), er is **geen handshake-deadline op
`accept_one`** (`try_dial` heeft er wél een, `mesh.rs:792`), en OOM in Rust is een `abort`,
geen fout. 500 verbindingen × 16 MiB ≈ 8 GB, vastgehouden door een aanvaller die alleen
keep-alives hoeft te beantwoorden.

**Oplossing.** Lees incrementeel in plaats van vooraf te alloceren (`try_reserve` met een
groeiende buffer), zet een `CONNECT_TIMEOUT` om de hele `accept_one`, en zet
`concurrent_connections` op een klein veelvoud van het aantal targets. Verlaag daarnaast
`MAX_FRAME_LEN`: `docs/ARCHITECTURE.md` zegt dat sync-antwoorden juist opgeknipt worden
zodat die grens nooit benaderd wordt, dus 256 KiB is ruim.

## B-18 — Het mediapad kent geen authenticatie en geen replaybescherming

**Waar:** `crates/net/src/media.rs:64-156`; consumenten in `crates/audio/src/session.rs:953-958`
en `crates/video/src/kijker.rs:300`.

Er is geen MAC, geen sleutel, geen sequentievenster. `MediaHeader` is een kale 16-byte
struct. De enige controle is het bronadres, en die verschilt per pad: audio vergelijkt het
volledige `SocketAddr` (goed), video alleen het IP (`van.ip() != cfg.afzender`) — de
bronpoort doet niet mee.

Wie een UDP-datagram met het bron-IP van een peer op de draad kan zetten — iemand op
hetzelfde LAN, een vierde tailnet-apparaat, of een gecompromitteerde tailnet-node — kan dus
audio in de mix injecteren die aan een legitieme peer wordt toegeschreven, of videofragmenten
injecteren (B-28). Opgenomen Opus-frames terugspelen vergt helemaal geen sleutel.

Dit is deels een aanvaarde afweging (WireGuard levert vertrouwelijkheid en integriteit) —
**maar de aanname wordt niet afgedwongen**, want de socket zit niet op de tailnet-interface
(B-09).

**Oplossing.** Bind aan het Tailscale-adres; laat `kijker.rs:300` het volledige `SocketAddr`
vergelijken zoals de audiokant al doet; en voeg een sessiesleutel plus een afgekapte MAC van
8 bytes en een replayvenster over `(stream_id, seq)` toe. De sleutel kan mee in de
`Hello`/`HelloAck` — additief conform invariant 5. 12 bytes overhead op 1100 past binnen het
MTU-budget. Invariant 1 verbiedt servers, geen cryptografie.

## B-19 — `UpdateSubresource` met genegeerde lengte en gestript stride-teken

**Waar:** `crates/video/src/codec.rs:741-765`.

```rust
Err(_) => { buffer.Lock(&mut ptr, None, Some(&mut lengte))?; breedte }   // lengte wordt nooit gebruikt
…
stap.unsigned_abs()                                                       // teken weggegooid
…
self.d3d.context.UpdateSubresource(&doel, 0, None, ptr as *const _, pitch, 0);
```

Met `pDstBox = NULL` kopieert D3D de héle subresource: `pitch × hoogte × 3/2` bytes vooruit
vanaf `ptr`. Twee ongecontroleerde aannames. Ten eerste wordt `lengte` opgehaald en nooit
vergeleken met wat er gelezen gaat worden. Ten tweede geeft `Lock2D` een **negatieve** stride
bij een bottom-up buffer, en dan wijst `ptr` naar de bovenste rij, `S·(h−1)` bytes verderop —
vooruit lezen loopt dan `S·(h−1)` bytes voorbij het einde. Op 1080p is dat ~2,07 MB
out-of-bounds.

`breedte`/`hoogte` komen uit `MF_MT_FRAME_SIZE` op het type dat de decoder uit de **SPS van
de peer** onderhandeld heeft, en dat wordt bij elke `MF_E_TRANSFORM_STREAM_CHANGE` opnieuw
gedaan — de bitstream van de aanvaller stuurt dus de leeslengte.

**Eerlijke nuance:** dit vereist dat de OS-MFT een bottom-up of te kleine buffer teruggeeft.
Dat is OS-gedrag, geen waarde die de aanvaller rechtstreeks zet — vandaar HOOG en niet
KRITIEK. Maar er is nul validatie, en een `// SAFETY:`-comment hoort een invariant te noemen
die ook echt afgedwongen wordt.

**Oplossing.** Weiger een negatieve stride expliciet, en controleer de beschikbare lengte
tegen `pitch × even(hoogte) × 3 / 2` vóór de kopie (met `Unlock2D` op beide foutpaden).

## B-52 — `offer_files` accepteert willekeurige paden uit de webview

**Waar:** `crates/app/src/ui/commands.rs:241-246`.

```rust
#[tauri::command]
pub fn offer_files(ui: State<'_, Ui>, paths: Vec<PathBuf>, channel: String) {
    for path in paths { offer_path(&ui, path, &channel); }
}
```

De frontend roept dit alleen aan met de payload van een OS-drop (`app.js:1478`), maar een
Tauri-commando is bereikbaar voor élk script in de webview. Eén regel volstaat om
`identity.toml` (de identiteit van deze installatie) of een SSH-sleutel te hashen en aan
alle peers aan te bieden.

Er is op dit moment **geen bekende XSS** om dat script binnen te krijgen (zie "Wat er goed
zit"), dus dit is vandaag geen actief lek — maar het is wel de reden dat één XSS meteen
datadiefstal zou betekenen in plaats van alleen schermschade.

**Oplossing.** Laat de paden nooit door de webview kiezen. Bewaar de drop-payload uit
`WindowEvent::DragDrop` (`ui/mod.rs:265-268`) in een kortlevende `Mutex<Vec<PathBuf>>` op
`Ui`, geef de frontend alleen ondoorzichtige indices, en laat `offer_files` een
`Vec<usize>` nemen. Precies het patroon dat `list_sources`/`share_source`
(`commands.rs:155-187`) al goed doet.

## B-53 — `offer_pasted_image` schrijft webview-bytes naar een webview-pad

**Waar:** `crates/app/src/ui/commands.rs:255-271`.

```rust
let extension = match extension.trim_matches('.') { "" => "png".to_string(), e => e.to_ascii_lowercase() };
let name = format!("fitcom-paste-{}.{extension}", …);
let path = std::env::temp_dir().join(name);
std::fs::write(&path, bytes)
```

`trim_matches('.')` haalt punten weg, maar geen separators. Een extensie als
`\..\..\..\Startup\evil.exe` lost op naar buiten `%TEMP%`, en `bytes` is volledig door de
aanroeper bepaald — een schrijfprimitief met vrije inhoud én vrije bestemming.

**Oplossing.** Een allowlist, wat hier sowieso de juiste vorm is:

```rust
let extension = match extension.trim_matches('.').to_ascii_lowercase().as_str() {
    e @ ("png" | "jpg" | "jpeg" | "gif" | "bmp") => e.to_string(),
    _ => "png".to_string(),
};
```

---

# MIDDEL

## B-20 — TOCTOU tussen verifiëren en toepassen van een update
`crates/app/src/engine.rs:707-736`, `crates/app/src/bin/fitcom-updater.rs:53,91-98`.
De hash wordt bij het downloaden gecontroleerd (`:2055`) en daarna nooit meer. Het gat tot de
klik is onbegrensd, en de updater vervangt de exe blind. Alles wat in dat venster in
`<datamap>/updates` kan schrijven, wisselt de payload om. Op een eenpersoons-PC vergt dat
lokale code-uitvoering — maar B-02 schrijft juist naar willekeurige mappen, inclusief deze.
**Oplossing:** geef de hash mee aan de updater (`--hash`) en laat die opnieuw verifiëren vlak
vóór `vervang()`. Na B-01 hoort daar ook de handtekening bij.
*Update 2026-08-13:* nog steeds open. De updater kreeg wel `start_zonder_handles`
(`bInheritHandles = FALSE` — het kind erft geen sockets meer), maar dat is hygiëne; er
wordt nog altijd niet opnieuw geverifieerd vóór het vervangen.

## B-21 — Elke peer kan op elk moment de draaiende exe ophalen
`crates/app/src/engine.rs:562-565`. Een `UpdateRequest` wordt zonder enige controle
gehonoreerd: geen "ben ik eigenlijk wel nieuwer", geen rate limit, geen deduplicatie. Elke
aanvraag spawnt een taak die de hele exe hasht en uploadt — goedkope
resource-uitputting, plus een exacte build-vingerafdruk voor wie een exploit uitzoekt.
*Wel goed:* dit pad leest uitsluitend `current_exe()`, dus het is géén willekeurige-bestandslezer.
**Oplossing:** alleen serveren als `is_newer(EIGEN_VERSIE, hun_versie)`, en maximaal één
upload tegelijk per peer.

## B-22 — Afbeeldingen downloaden en renderen zichzelf, zonder groottegrens
`crates/app/src/engine.rs:636-658`. Elk nieuw `FileMeta` van een ander waarvan de naam op
`.png/.jpg/.jpeg/.gif/.bmp` eindigt wordt direct opgehaald — live én bij het inhalen van
geschiedenis. Er is geen groottecheck in dat filter (zie B-13) en geen grens op de
gedecodeerde afmetingen: een PNG van 30000×30000 is een paar honderd kB op de draad en
gigabytes in de renderer.
*Wel goed:* de `image`-crate decodeert hier niets — die wordt alleen gebruikt om onze eigen
videominiaturen te encoderen (`ui/mod.rs:409-421`). Het decoderen doet WebView2.
**Oplossing:** alleen automatisch downloaden onder een plafond (bijv. 16 MiB), en de
afmetingen vooraf aftasten met `ImageReader::into_dimensions()` (leest alleen de header).

## B-23 — Geen kanaalcontrole bij ontvangst
`crates/store/src/lib.rs:240`, `crates/app/src/chat.rs:307-308`. `visible_to` wordt alleen op
de *verzend*kant toegepast. `apply_remote` accepteert een op voor elk kanaal, dus ook een DM
tussen twee anderen — die wordt opgeslagen, en de ongelezen-teller loopt op voor een gesprek
waar je niet in zit. *Wel goed:* de weergavelaag houdt stand (`ui/state.rs:585-594`), dus
getoond wordt het niet.
**Oplossing:** spiegel het verzendfilter bij ontvangst: weiger als
`!op.channel.is_public() && op.channel.dm_peer() != Some(me) && op.author != me`.

## B-24 — Target-desync laat een peer permanent "online" met een dode verbinding
`crates/net/src/mesh.rs:658-673`. `on_closed` zoekt de verbinding op via `active`; is die
inmiddels door een nieuwere vervangen, dan vindt hij niets en keert vroeg terug — waarna
`targets[t].bound` en `connected` blijven hangen. Bereikbaar als twee targets naar dezelfde
identiteit oplossen (misconfiguratie, of afgedwongen via B-05): de echte peer wordt nooit
meer gebeld, en de aangeleerde identiteit wordt naar de config weggeschreven.
**Oplossing:** weiger een `install` waarvan de `peer_id` al als `known_id` op een ánder target
staat, en laat `on_closed` losmaken op `targets[t].bound == Some(conn_id)` in plaats van via
een omgekeerde zoekactie in `active`.

## B-25 — Logvervuiling stalt de mesh-actor
`crates/net/src/mesh.rs:632-640`, `crates/net/src/framing.rs:47-51`. Frames van een nog niet
gekoppelde verbinding worden gelogd **op de actortaak**, met `?msg` — dus de hele
gedecodeerde `ControlMsg`, wat bij een grote `OpBroadcast` een regel van megabytes is. De
appender schrijft bewust synchroon (`main.rs:136-141`), dus peerstatus, ping/pong en
`retry_pending` staan zolang stil. Venster: 5 s per verbinding, oneindig herhaalbaar.
**Oplossing:** laat `?msg` weg (log alleen `conn_id` en de tag), rate-limit tot één regel per
verbinding, en sluit een niet-koppelbare verbinding direct na de `Hello` in plaats van er 5 s
een berichtenpomp op te laten draaien.

## B-26 — Onbegrensde `pending`-lijst
`crates/net/src/mesh.rs:380,476-480`. Elke niet-koppelbare inkomende verbinding wordt 5 s
bewaard, inclusief een levende `quinn::Connection` en zijn taken, zonder limiet op de lengte
van de lijst. `retry_pending` scant hem bovendien bij elke tik volledig na.
**Oplossing:** `MAX_PENDING` als klein veelvoud van het aantal targets; bij overschrijding de
oudste weggooien, niet de nieuwste, zodat een flood een vroege legitieme peer niet verdringt.

## B-27 — Een te groot UDP-datagram is een fout in plaats van rommel
`crates/net/src/media.rs:153`. De module negeert bewust *te korte* pakketten
("op een open UDP-poort komt vroeg of laat rommel binnen"), maar een datagram groter dan de
buffer geeft op Windows `WSAEMSGSIZE`, dat door geen van de twee opgevangen armen wordt
gedekt en dus als fout terugkomt. Beide consumenten loggen en gaan door — één synchrone
schrijfactie naar het logbestand per pakket, op precies de audio- en videothreads die
invariant 4 beschermt.
**Oplossing:** behandel `raw_os_error() == Some(10040)` net als een te kort pakket: `Ok(None)`.

## B-28 — Fragmentinjectie
`crates/video/src/kijker.rs:300`, `crates/video/src/fragment.rs:154-157`. `compleet()` telt
alleen het *aantal* stukken, niet of de indices 0..n er ook echt zijn, en het beeld wordt in
`BTreeMap`-sleutelvolgorde aan elkaar geplakt. Omdat de bronpoort niet meetelt, kan wie het
IP van de deler kan spoofen fragmenten met dezelfde tijdstempel injecteren, of een
pariteitspakket met een verkeerd aantal sturen. Het resultaat is een beeld dat deels uit
aanvallersbytes bestaat en als authentiek aan de decoder gaat.
**Oplossing:** valideer `frag_index < aantal` bij de insert (B-12), en vergelijk het volledige
`SocketAddr` zoals `session.rs:957` al doet.

## B-29 — Geen maximum framegrootte richting de OS-decoder
`crates/video/src/kijker.rs:420`. Wat de reassembler oplevert gaat ongefilterd naar de
H.264-decoder; met B-12 open is dat tot ~97 MB in één `IMFSample`. Los van de allocatie is dit
een grote, ongevalideerde invoer aan een closed-source OS-decoder — precies waar
H.264-parserfouten wonen.
**Oplossing:** weiger in `Reassembler::push` boven ~1 MB (gemeten keyframes zijn 100-371 kB).

## B-30 — De tijdstempel-unwrapper is onbereikbaar
`crates/video/src/kijker.rs:589-599` versus `crates/video/src/fragment.rs:243-250`.
`Uitvouwer` vangt de 90 kHz-omloop op door een grote sprong terug te herkennen, maar hij wordt
alleen op *complete* beelden aangeroepen — en de reassembler gooit alles met
`timestamp <= laatste` juist wég. Een omloop ís zo'n sprong terug, dus `hoog` kan nooit
oplopen. Na 2³² ÷ 90 000 = **13 uur 15 minuten** onafgebroken kijken bevriest het beeld
precies als bij B-11. De bestaande test dekt `Uitvouwer` geïsoleerd en blijft dus groen.
**Oplossing:** dezelfde wrap-bewuste vergelijking als B-11, plus een ketentest die een
omlopende reeks door `Reassembler` én `Uitvouwer` heen duwt.

## B-31 — Onbegrensde aankondigingen en afdwingbare encoderbelasting
`crates/app/src/streams.rs:342-350,372-388`. Er is geen limiet op het aantal streams dat één
peer mag aankondigen, en `titel` is een onbegrensde string die opgeslagen, gelogd (`:341`) en
getoond wordt. Elke intekening dwingt bovendien `Actie::StartDelen` af — capture plus encoder,
precies de belasting die invariant 4 wil vermijden.
*Wel goed:* `keyframe_gevraagd` (`:412-424`) honoreert alleen echte kijkers, met de motivering
erbij. Dat patroon ontbreekt bij `ingetekend`.
**Oplossing:** cap het aantal streams per peer, begrens `titel`, en overweeg een bevestiging
(of minstens een zichtbare indicatie) voordat een intekening de encoder start.

## B-32 — Logbestanden zonder retentielimiet
`crates/app/src/main.rs:160-165`. `.max_log_files(..)` wordt niet aangeroepen, dus dagbestanden
stapelen zich onbeperkt op. Met autostart aan draait dit maandenlang onbeheerd, en B-25/B-27
kunnen het volume vanaf het netwerk opdrijven. **Oplossing:** `.max_log_files(14)`.

## B-33 — Loginjectie via `display_name` en `app_version`
`crates/net/src/mesh.rs:557-560`, ook `crates/app/src/engine.rs:1430,1440`. Deze velden komen
onbegrensd en ongevalideerd van de draad en worden met de `%`-sigil gelogd, die niets escapet
(anders dan `?`). Een naam met een nieuwe regel erin kan overtuigende neplogregels
fabriceren — bijvoorbeeld een verzonnen "hash klopt niet" — precies wanneer je het log nodig
hebt om een incident te reconstrueren.
**Oplossing:** gebruik `?` voor velden van de draad, en begrens en filter `display_name` en
`app_version` al in `net`, zodat elke consument (log, UI, bestandsnaam) een schone waarde krijgt.

## B-34 — `seq` u64→i64
`crates/store/src/lib.rs:436,546`. De roundtrip is verliesvrij, maar `seq >= 2^63` wordt als
negatief opgeslagen. Zulke rijen zijn onbereikbaar voor `ops_range` en `advance_contiguous`
(die met positieve grenzen werken) en dus permanent inert — terwijl ze wel meetellen in
`op_count` en meekomen in `all_ops()`. Dat geeft een aanvaller 2⁶³ extra sleutels voor B-16.
**Oplossing:** weiger `op.seq > i64::MAX as u64` bij het decoderen.

## B-35 — `atty 0.2.14` (RUSTSEC-2021-0145)
`Cargo.lock`, via `nnnoiseless 0.5.2` → `clap 3.2.25` → `atty`. Windows is het getroffen
platform. **Praktisch onbereikbaar**: `nnnoiseless` gebruikt `clap` alleen voor zijn
binaries, niet voor de library-API die dit project aanroept. Het is wel het eerste wat
`cargo audit` gaat melden, en het sleept een verouderde `clap 3`-subboom mee.
**Oplossing:** probeer `nnnoiseless` met `default-features = false`; lukt dat niet, leg de
uitzondering vast in een `deny.toml` mét reden.

## B-36 — libopus uit een 2021-snapshot
`crates/audio/Cargo.toml:15` (`opus 0.3.1` → `audiopus_sys 0.2.2`). Dit is C-code die
audiopakketten van peers decodeert en die vier jaar aan upstream-fixes mist. Ik claim geen
specifieke CVE — controleer tegen de daadwerkelijk meegeleverde libopus-versie. De
`CMAKE_POLICY_VERSION_MINIMUM`-override in `.cargo/config.toml` is op zichzelf geen gat, maar
hij zet het signaal "deze dependency is te oud voor moderne tooling" om van een bouwfout in
stilte. **Oplossing:** een onderhouden binding of een verse libopus vendoren; minimaal een
gedateerde notitie zodat dit terugkomt.

## B-54 — `apply_update` is een ongeconditioneerd IPC-commando
`crates/app/src/ui/commands.rs:315-318`. Elk script in de webview kan het bevestigingsvenster
(`app.js:1198-1205`) overslaan en een klaarstaande update meteen toepassen. Op zichzelf
begrensd — het bestand is geverifieerd tegen wat de peer aankondigde — maar dat is precies de
tweede helft van B-01/B-02. Hetzelfde geldt voor `delete_all_images` (`commands.rs:287-290`).
**Oplossing:** zet onomkeerbare commando's achter een Rust-zijdige poort: een eenmalig token
dat `get_state` alleen uitgeeft zolang `UpdateStatus::KlaarOmToeTePassen` leeft, of een
native `rfd::MessageDialog` in `pas_update_toe` zelf.
*Update 2026-08-13:* het commando-oppervlak is gegroeid met `open_link`, `check_update`,
`preview_sound` en `set_sound_settings`. Elk is Rust-zijdig begrensd — `open_link` op
`http(s)` zonder shell-parsing, `check_update` op het zoekslot, de geluidscommando's op
naam-allowlists en een volumeclamp (inclusief NaN) — dus geen nieuwe ernst, maar het
patroon blijft: elke poort die alleen in de frontend zit, is er voor een script in de
webview niet.

## B-55 — Geen navigatiebeleid
`crates/app/src/ui/mod.rs:162-276` registreert geen `on_navigation`. De linkifier
(`app.js:164`) beperkt het schema correct tot `https?`, maar wat er bij een klik gebeurt
beslist de app niet. Lost WebView2 `target="_blank"` op als navigatie in hetzelfde venster,
dan wordt het appvenster vervangen door door de peer gekozen webinhoud — en Tauri's
initialisatiescripts (inclusief `window.__TAURI__`) worden bij elke paginalading geïnjecteerd.
Remote-origin IPC staat *niet* aan, dus commando-uitvoering blijft buiten bereik; een
phishingoppervlak binnen een vertrouwd venster is het wel.
**Oplossing:** maak de keuze expliciet met een `on_navigation`-allowlist, en open externe
links via de opener-plugin in de systeembrowser.
*Update 2026-08-13:* grotendeels gedicht in 1.0.2. De frontend annuleert elke klik op een
`<a>` en geeft de URL aan `commands.rs::open_link`, dat alleen `http(s)` doorlaat en de
URL als één argument aan `ShellExecuteW`/`open` geeft — geen shell-parsing, dus een `&`
in een URL is gewoon een `&`. Wat rest is de diepteverdediging: er is nog geen
`on_navigation`-beleid, en de onderschepping leeft in de webview en dekt alleen het
gewone klikpad (een middenklik of `window.open` valt erbuiten). De bevinding blijft
daarom staan, met minder gewicht.

## B-56 — Latente XSS: escaping is de verantwoordelijkheid van de aanroeper
`crates/app/frontend/app.js:252-263` (sink `:302`) en `:531` (aanroepers `:576-589`).
`voiceHint()` interpoleert peernamen — volledig door de peer bepaald via `OpKind::SetNick`,
zonder lengte- of inhoudsvalidatie — ongeëscaped in `innerHTML`. **Vandaag onbereikbaar**: die
tak draait alleen als de roster leeg is, en dan bevat hij juist geen namen. `memberRow`
behandelt `opts.sub` als een rauwe HTML-slot; één aanroeper escapet wel (`:589`), de andere
niet. Ook dat is vandaag veilig, omdat de betreffende waarden vaste strings met `u32`-velden
zijn. Beide zijn één refactor verwijderd van een echte injectie.
**Oplossing:** `esc()` in de sink, niet bij de aanroeper — `${esc(others[0].name)}` en
`<div class="mem-sub">${esc(opts.sub)}</div>`, en de `esc()` bij `:589` weghalen.

---

# LAAG

- **B-37** `crates/video/src/fragment.rs:262` — `header.frag_index + 1` op een `u16` overflowt
  bij `0xFFFF`. In een debugbuild (de gedocumenteerde ontwikkelflow) is dat een paniek op de
  kijkerthread, die langs `KijkerEvent::Gesloten` heen unwindt — de motor merkt dus niet dat
  de kijker dood is en de UI blijft een actieve stream tonen. Eén pakket.
  *Fix:* `saturating_add(1)`, of de indexcheck uit B-12.
- **B-38** `crates/audio/src/jitter.rs:128,133,145` — `volgende + 1` overflowt op `u32::MAX`.
  De mixthread wordt één keer gestart en nooit herstart, dus die paniek maakt **alle** audio
  permanent stil terwijl de UI een gezonde sessie toont. Twee pakketten.
  *Fix:* `wrapping_add(1)`, en maak de "te laat"-test wrap-bewust
  (`seq.wrapping_sub(v) > u32::MAX / 2`).
- **B-39** `crates/net/src/filestream.rs:68` — alles behalve `1` wordt als "bestand" gelezen.
  Nu onschadelijk (de twee waarden zijn uitputtend), maar het is exact het aliaspatroon dat
  de 2→3-bump moest voorkomen. *Fix:* expliciete `match` met een `bail!` op onbekend.
- **B-40** `crates/net/src/mesh.rs:823-836` — `VersionMismatch` keert terug als `Ok(())` en
  reset daarmee de backoff, dus tegen een peer die altijd een afwijkende versie meldt doen we
  elke seconde een volledige QUIC+TLS-handshake. *Fix:* alleen resetten na een sessie die
  echt `Online` haalde.
- **B-41** `crates/net/src/mesh.rs:565` — `media_port` komt ongevalideerd van de peer. Het IP
  komt gelukkig uit de verbinding, dus dit is geen reflectie naar derden; wel kan een peer
  onze volledige voice+screenshare op een willekeurige poort van zijn eigen host richten, en
  `media_port: 0` levert een adres op dat bij elk pakket faalt. *Fix:* weiger 0.
- **B-42** `crates/store/src/timeline.rs:103` — `wall_clock` is volledig door de afzender
  bepaald en wordt getoond. Ordening gebruikt het niet, dus dit is puur weergave: met B-06
  kan een vervalst bericht zich als oud voordoen. *Fix:* klem op ±7 dagen rond de lokale tijd.
- **B-43** `crates/proto/src/op.rs:126-145`, `crates/proto/src/control.rs:131,139,210` — geen
  enkele string of collectie op de draad heeft een lengtegrens; de enige rem is
  `MAX_FRAME_LEN` (16 MiB). Dat is het grondstofje voor B-15 en B-16.
  *Fix:* velden begrenzen bij het deserialiseren — 4 KiB voor een berichttekst, 255 voor een
  bestandsnaam, 64 voor een bijnaam of titel.
- **B-44** `crates/app/src/engine.rs:1650` — `unreachable!()` na `for i in 2u32..`. Vergt ~4
  miljard botsende namen, dus geen realistisch doelwit; `2u64..` kost niets.
- **B-45** `crates/video/src/codec.rs:593` — `MFCreateMemoryBuffer(data.len() as u32)` gevolgd
  door een `copy_nonoverlapping` over de volledige `usize`-lengte. **Nu niet bereikbaar** (de
  framegrootte is structureel begrensd op ~97 MB), maar die grens is impliciet: wie
  `MAX_PAKKET` verhoogt of `frag_index` verbreedt, maakt hier een heap-overflow van.
  *Fix:* `u32::try_from(data.len())?`.
- **B-46** `Cargo.toml:18-21` en `.cargo/config.toml` — geen Control Flow Guard (op MSVC is
  dat opt-in via `-C control-flow-guard=yes`, en deze binary linkt libopus en SQLite en
  verwerkt netwerkinvoer), geen `overflow-checks` in release (dus de tests die je draait
  testen niet de code die je uitlevert — zie B-37 en B-38), en geen `strip`, waardoor
  paniekpaden met absolute `C:\Users\...`-paden in de uitgeleverde exe blijven staan.
  *Wel goed en zo laten:* `panic` staat op unwind, niet `abort` — `abort` zou van elke paniek
  in een taak een procesbrede crash maken, tegen invariant 7 in.
- **B-47** `crates/app/src/config.rs:230-236` — in portable modus staat schrijfbare data naast
  een exe die zichzelf overschrijft. Wordt de zip uitgepakt op een plek met ruime ACL's
  (klassiek: een map in de root van `C:\`), dan kan een ander niet-admin-account daar een
  vervangende exe neerzetten. *Fix:* documenteer "pak uit onder je gebruikersprofiel".
- **B-48** `crates/app/capabilities/default.json:13` — de devtools-capability staat zonder
  buildconditie aan. In release inert (de feature wordt niet meegecompileerd), maar het
  capability-bestand is de verkeerde plek om daarop te leunen.
- **B-49** `crates/app/frontend/app.js:162-173` — de `@naam`-vervanging draait ná de
  linkificatie, over een string die al markup bevat, dus een mention binnenin een URL wordt
  in het `href`-attribuut herschreven. Uitgevoerd levert dat een `<a>` op met een afgekapte
  `href` en een rommelattribuut. **Geen scriptuitvoering**: de ingevoegde tekst is vast
  (`class="mention"`), dus de tag sluit altijd op de `>` van die span en er komt nooit een
  aanvallersgestuurde `=` in. Gevolg is een link waarvan het doel afwijkt van de zichtbare
  tekst — misleidend, niet uitvoerbaar.
  *Fix:* mentions vóór de linkificatie verwerken, of matches binnen tags overslaan.
- **B-50** `crates/video/src/spoor.rs:31-33` — `regels: Vec<String>` groeit één regel per beeld
  en wordt pas bij `klaar()` geleegd. Zit achter `FITCOM_SPOOR`, dus niet van buiten
  bereikbaar; wel juist aan tijdens lange diagnosesessies.
- **B-57** `crates/app/src/notify.rs:16,38-41` — één mislukking zet `TOASTS_WERKEN` voorgoed
  uit, dus een tijdelijke WinRT-hik of een quotum kost je alle mentionmeldingen tot een
  herstart. *Wel goed:* XML-injectie kan hier niet (de crate bouwt de toast via de
  WinRT-DOM), en de afkapping op `:26-30` telt in chars, dus geen UTF-8-paniek.
  *Fix:* uitschakelen met een tijdstempel en na een afkoelperiode opnieuw proberen.
- **B-58** `crates/app/frontend/app.js:178-187` — `highlight()` draait over al ge-escapete
  tekst, dus `&#39;` wordt deels als getal en deels als commentaar getokeniseerd. Puur
  cosmetisch; alle ingevoegde markup is vast.
- **B-59** `crates/app/src/ui/mod.rs:168` (ook `:85,:100,:173,:179`) — `.lock().unwrap()` in de
  `thumb://`-handler paniekt op een vergiftigde mutex. *Fix:* `unwrap_or_else(|e| e.into_inner())`.
- **B-60** *(sinds 1.1.0)* `crates/app/src/engine.rs::zoek_updater` — ontbreekt
  `fitcom-updater.exe`, dan telt álles in de map van de app dat met `fitcom-updater`
  begint en op `.exe` eindigt, en de eerste op alfabet wordt gestart. Bedoeld voor
  browser-hernoemingen (`fitcom-updater (1).exe`), en de map is dezelfde vertrouwensgrens
  als `fitcom.exe` zelf — maar in het zwakke-ACL-scenario van B-47 verbreedt dit wat een
  neergezet bestand mag heten. *Fix:* dit staat of valt met de B-47-afspraak ("pak uit
  onder je gebruikersprofiel"); wie meer wil, pint de updater op een hash die bij de
  release in de app gebakken is.
- **B-51** *(INFO)* `crates/proto/src/op.rs:150-172` — `LamportClock` is dode code; de enige
  verwijzing is zijn eigen unittest. De klok die echt gebruikt wordt is `max_lamport()`, en
  dáár zit de truncatie van B-14. Aansluiten of weggooien, zodat de volgende lezer niet
  aanneemt dat de veilige implementatie de levende is.

---

# Herstelplan

Op volgorde van "wat haalt de meeste risico weg per uur werk". De eerste twee blokken zijn
klein, lokaal en goed testbaar; blok 4 is het echte werk.

### Blok 0 — vandaag, zonder code (5 minuten)
Zolang er nog niets gefixt is, is dit de enige echte maatregel: controleer op alle drie de
PC's dat de Windows Firewall-regel voor `fitcom.exe` **alleen** voor het Tailscale-netwerk
geldt en niet voor "Privé" of "Openbaar". Dat maakt B-09 in de praktijk een stuk kleiner en
daarmee de hele keten. Neem dit ook op in `README.md` als expliciete stap in plaats van als
terzijde.

### Blok 1 — de drie schrijfprimitieven dichten (een dagdeel)
Dit haalt alle nul-klik-code-uitvoering weg. Elk van de drie is een gelokaliseerde wijziging
met een duidelijke test.

1. **B-02** — `parse_strict` in `appversion.rs`, weiger onbruikbare versies in
   `overweeg_update`, en bouw bestandsnamen uit de gecanoniseerde tuple.
2. **B-03** — één gedeelde `veilige_bestandsnaam`, toegepast waar `FileEntry` gebouwd wordt
   (`store/timeline.rs`) zodat elke consument hem erft.
3. **B-53** — extensie-allowlist in `offer_pasted_image`.

Regressietests horen hierbij: `crates/app/tests/file_deling.rs` zet de mesh al op, dus een
peer die een `FileMeta` met `..\` aanbiedt is een paar regels extra.

### Blok 2 — de goedkope grendels (een dagdeel)
4. **B-04** — geef `from` door en eis een openstaande aanvraag bij de juiste peer.
5. **B-13** — dwing de aangekondigde grootte af, met een absoluut plafond voor updates.
6. **B-21** — serveer `UpdateRequest` alleen als we echt nieuwer zijn, één tegelijk.
7. **B-20** — geef de hash mee aan `fitcom-updater.exe` en verifieer daar opnieuw.
8. **B-52** — laat `offer_files` indices nemen in plaats van paden.
9. **B-32/B-33** — `.max_log_files(14)` en `?` in plaats van `%` voor velden van de draad.

### Blok 3 — de mediastack stabiel maken (een dag)
10. **B-11 + B-30** — één wrap-bewuste tijdstempelvergelijking lost beide op; schrijf de
    ketentest die de omloop door `Reassembler` én `Uitvouwer` duwt.
11. **B-12** — `frag_index`- en payloadvalidatie plus verval op leeftijd. Dit sluit meteen
    B-29, B-37 en het pariteitsgat.
12. **B-10** — plafond op het aantal audiobronnen per peer, en `decoders` snoeien.
13. **B-38** — `wrapping_add` in de jitterbuffer.
14. **B-27** — `WSAEMSGSIZE` als rommel behandelen.

### Blok 4 — het vertrouwensmodel echt maken (het eigenlijke werk)
Hier zit het verschil tussen "moeilijker te misbruiken" en "niet meer misbruikbaar".

15. **B-09** — bind aan het Tailscale-adres en weiger onbekende bronadressen vroeg.
16. **B-05** — bind `PeerId` cryptografisch aan de verbinding: vaste Ed25519-sleutel per
    installatie, handtekening over `export_keying_material` in de `Hello`, en
    `PeerId == blake3(pubkey)`. Repareer tegelijk B-05a (stuur `HelloAck` pas ná de controles)
    en B-05b (controleer identiteit onafhankelijk van de matchsleutel).
17. **B-01** — Ed25519-handtekening op de update-payload, publieke sleutel in de binary.
    Kan tekenen niet, dan minimaal: niet automatisch downloaden, en bronpeer + hash in het
    bevestigingsvenster tonen.
18. **B-06/B-07/B-08/B-14** — één `validate(&Op) -> Result<()>` bij `apply_remote_batch` die
    afzender, `lamport`, `seq` en veldlengtes controleert, plus een totale en injectieve
    `channel_to_blob`.

### Blok 5 — verdediging in de diepte
19. **B-18** — sessiesleutel en afgekapte MAC op het mediapad; dit sluit de hele
    spoofingcategorie (B-28 incluis) in één keer.
20. **B-15/B-16/B-17/B-26** — bytebudgetten en plafonds op ops, frames, verbindingen en de
    `pending`-lijst.
21. **B-46** — CFG aanzetten, `overflow-checks = true` in release overwegen (dat maakt B-37 en
    B-38 in de uitgeleverde build tot een nette paniek in plaats van stille wrap).
22. **B-35/B-36** — zodra er een Windows-machine met toolchain is: `cargo deny` met een
    `deny.toml` waarin de uitzonderingen mét reden staan.

### Wat ik zou doen als er maar één ding kon
Blok 0 vandaag, en dan **B-02 en B-03** — dat zijn samen minder dan honderd regels en ze halen
allebei de nul-klik-varianten van code-uitvoering weg. B-01 is de principieel juiste fix, maar
hij is groter en hij helpt niet tegen de twee padinjecties, die de klik juist overslaan.

---

# Wat er goed zit

Dit hoort in hetzelfde document, anders leest het bovenstaande als "alles is stuk", en dat is
niet zo. Deze dingen zijn nagelopen en correct bevonden.

**Architectuur.**
- De `Snapshot`/`UiCommand`-grens houdt stand en heeft de hele Tauri-wissel overleefd. Dat is
  de reden dat de UI-laag zo klein en zo goed te auditen is.
- `proto` en `store` zijn vrij van Windows- en hardware-afhankelijkheden, precies zoals
  beloofd. Daardoor is de subtiele logica überhaupt te beoordelen.
- De pure-beslissing/uitvoering-scheiding (`files.rs`, `updates.rs`, `streams.rs` beslissen,
  `engine.rs` voert uit) maakt de beveiligingsrelevante beslissingen leesbaar op één plek.

**Concrete controles die er wél zijn, en die kloppen.**
- **DM-bestanden.** `files.rs:117-135` weigert een DM-bestand aan iedereen behalve de
  geadresseerde, en geeft bewust hetzelfde antwoord als "bestaat niet" — een onderscheidbare
  weigering zou juist bevestigen dát het bestaat. Met tests.
- **Aanbodintrekking.** `Files::verwijder_aanbod` is echt gekoppeld aan `UiCommand::Verwijder`,
  dus verwijderen stopt ook echt met serveren — de fix uit beslissing 13 doet wat hij belooft.
- **Keyframe-verzoeken.** `streams.rs:412-424` honoreert alleen echte kijkers, met de
  motivering in de comment. Precies het patroon dat elders ontbreekt.
- **DM's worden niet doorgestuurd.** `chat.rs:325` filtert op `is_public()` vóór het
  doorsturen. De verzendzijdige `visible_to` (`op.rs:254-256`) is correct, en `chat.rs`
  gebruikt consequent de `_for`-varianten.
- **Het intekenadres komt uit de verbinding**, niet uit het bericht (`streams.rs:308`), dus er
  is geen reflectie naar willekeurige derden.
- **De audiokant vergelijkt het volledige `SocketAddr`** (`session.rs:953-958`). De videokant
  zou dat moeten kopiëren (B-28), niet andersom.
- **`Halffabrikaat::herstel`** (`fragment.rs:164-199`) is de ene plek in de mediapijplijn waar
  een lengte van de draad wél gevalideerd wordt vóór gebruik.
- **De jitterbuffer-recursie uit fase 3 is echt weg** — de overloop-tak is een begrensde lus
  met een regressietest.
- **`filestream.rs`-offsets kloppen.** Ik heb `write_header` en `read_kind` byte voor byte
  nagelopen; de leesvolgorde oogt door elkaar maar de indices zijn correct en de
  `expect("8 bytes")` is per constructie onbereikbaar.

**Frontend en Tauri — dit is het sterkste deel van de codebase.**
- **Geen bereikbare XSS.** Alle 17 `innerHTML`-toekenningen zijn afzonderlijk nagelopen;
  `esc()` dekt `& < > " '` en elke attribuutinterpolatie staat tussen aanhalingstekens. Geen
  `eval`, geen `new Function`, geen `document.write`, geen `insertAdjacentHTML`.
- **`script-src 'self'` zonder `unsafe-inline` en zonder `unsafe-eval`** is de beste
  beveiligingsbeslissing in deze codebase. Laat hem staan: hij is de reden dat B-56 latent
  blijft in plaats van uitvoerbaar.
- **De linkifier beperkt het schema correct** tot `https?` — `javascript:`, `data:`,
  `vbscript:` en `file:` matchen niet.
- **De asset-scope is dicht**: leeg in config, en bij runtime versmald tot precies één
  niet-recursieve map. `image_path` lost altijd op naar `<64 hex>.<ext>` en wordt op bestaan
  gefilterd.
- **De `thumb://`-handler raakt de schijf niet** — het is een `HashMap`-lookup met een
  404-terugval, dus een geknutselde `thumb://`-URL leest niets.
- **De capability-set is minimaal**: `core:default` plus vijf vensterknoppen, gescoped op
  `windows: ["main"]`, geen `remote`, geen filesystem-, shell- of process-plugin.
- **`tags.rs` heeft geen bereikbare paniek** — alle slice-indices zijn `find`-resultaten of
  `start + 1` op een ASCII-`@`, dus altijd een char-grens. Multi-byte invoer is gedekt.
- **Toast-meldingen kennen geen XML-injectie**: de crate bouwt de toast via de WinRT-DOM, en
  het afkappen telt in chars, niet in bytes.

**Voorraadketen en build.**
- **Geen secrets in de repo of in de git-historie.** 64 commits nagelopen; geen sleutel, geen
  token, geen echt Tailscale-adres. De `100.64.0.x`-adressen in `README.md` en `config.rs`
  zijn CGNAT-voorbeelden.
- **Geen uitgaand verkeer.** Geen CDN, geen telemetrie, geen externe URL in de frontend; fonts
  zijn echt lokaal gebundeld. Invariant 1 wordt nageleefd.
- **Alle 577 dependencies komen van crates.io**, geen git-bronnen, geen `[patch]`, en
  `Cargo.lock` is gecommit — builds zijn per checksum vastgepind.
- **Geen elevatie, geen DLL-search-order-risico.** Autostart schrijft alleen naar `HKCU\…\Run`
  en zet het pad correct tussen aanhalingstekens. Het updaterpad wordt uit `current_exe()`
  afgeleid, nooit uit `PATH` of de werkmap.
- **`panic = unwind` in plaats van `abort`** is hier de juiste keuze en moet zo blijven.
- **De cryptoprimitieven zijn prima gekozen** — TLS vastgezet op 1.3, `ring` consequent
  doorgevoerd, BLAKE3 voor integriteit. Het probleem is niet de cryptografie; het is dat
  `AcceptAnyCertificate` maakt dat ze niemand authenticeren.
- **Geen chatinhoud in de logs** — alleen aantallen en peer-id's.
- **Geen SQL-injectie.** Elke query is een letterlijke string met gebonden parameters; de
  migratie gebruikt volledig statische DDL.

---

# Wat dit onderzoek niet gedekt heeft

Zodat niemand dit document leest als "hier staat alles in".

- **Er is niets uitgevoerd.** Op deze machine staat geen Rust-toolchain en het project is
  Windows-only, dus er is niet gebouwd, niet getest en geen enkele exploit daadwerkelijk
  gedraaid. Alles hierboven is codelezing met regelnummers. De exploits zijn *afgeleid*, niet
  *gedemonstreerd* — voor B-02, B-03, B-04 en B-13 is dat een kwestie van een paar regels
  bovenop `crates/app/tests/file_deling.rs`, en dat zou ik ook aanraden vóór de fix, zodat de
  test eerst rood staat.
- **`cargo audit` en `cargo deny` zijn niet gedraaid** (niet geïnstalleerd, en ik heb niets
  geïnstalleerd). B-35 is met vertrouwen benoemd, maar de volledige advieslijst moet nog een
  keer echt langs een scanner. Ik heb bewust geen advisory-ID's verzonnen: waar ik niet zeker
  was, staat dat er zo bij.
- **De C-code van libopus en de bundled SQLite zijn niet doorgelezen.** B-36 gaat over hun
  ouderdom, niet over een concrete bug daarin.
- **Windows Media Foundation en WebView2 zijn black boxes.** B-19 en B-29 gaan over wat wij
  eraan voeren, niet over wat er binnenin gebeurt.
- **Geen fuzzing.** De msgpack-decoder, de fragment-reassembler en de H.264-invoer zijn
  precies de plekken waar fuzzing dingen vindt die lezen niet vindt. `cargo-fuzz` op
  `ControlMsg::decode` en `Reassembler::push` zou ik als eerste opzetten — beide zijn puur en
  hebben geen hardware nodig, dus ze passen precies in wat `proto`/`store` testbaar maakt.
- **Geen review van Tailscale zelf**, en geen oordeel over de tailnet-ACL's. Dit document
  neemt aan dat WireGuard doet wat het belooft; het punt van B-09 is juist dat de app daar
  niet op aansluit.
- **Timing- en zijkanaalanalyse is niet gedaan** en lijkt me hier ook niet de moeite.

---

*Opgesteld 2026-08-05. Bij het oppakken van een bevinding: noem het ID in de commit, en werk
de regel in de overzichtstabel bij in plaats van hem weg te halen — dan blijft zichtbaar wat
er bewust is blijven staan.*
