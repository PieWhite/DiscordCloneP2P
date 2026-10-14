# TODO / Backlog

Bewust niet in v1. De architectuur moet deze items kunnen opnemen zonder herontwerp.

## Gekozen door Rick, nog te bouwen (2026-10-15)
Volgorde is een voorstel: 1 bouwt voort op Spotify delen, 2 is wat je tijdens het gamen
het meest gebruikt. Per item eerst de context dichttimmeren, dan pas bouwen.

1. **Luisteraars bedienen de muziek.** Pauze/volgende-knoppen bij de luisteraars, en een
   Spotify-link in de chat die de DJ in zijn wachtrij kan zetten. Pauze en volgende gaan
   lokaal bij de DJ via dezelfde mediabediening waar `muziek.rs` de titel al uit leest
   (`TryTogglePlayPauseAsync`, `TrySkipNextAsync`); een `open.spotify.com/track/…`-link
   opent bij de DJ als `spotify:track:…`. Geen Web API, invariant 1 blijft heel. Nieuw
   control-bericht (additief) van luisteraar naar DJ; de DJ voert alleen uit voor wie in
   het gesprek zit, en het track-id gaat net als het YouTube-id eerst door een strikte
   controle (B-03-klasse). Open vraag: mag iedereen meteen skippen, of eerst stemmen?
2. **Push-to-talk en push-to-mute.** Een globale toets, ook terwijl een spel vooraan
   staat. Twee standen: ingedrukt = praten (push-to-talk), of ingedrukt = stil
   (push-to-mute). De clip-hotkey laat al zien hoe een globale toets binnenkomt. Toets en
   stand in `config.toml`, `#[serde(default)]`. Raakt de VAD-poort in `opname_lus`: de toets
   gaat vóór de VAD, en de spreekring moet blijven volgen wat er echt verstuurd wordt.
3. **"Speelt X" naast je naam.** Het spel dat vooraan staat, als regel onder je naam bij de
   anderen. Lokaal bepaald (vensternaam of exe van het voorgrondvenster, alleen als hij
   schermvullend is, en een korte lijst bekende niet-spellen eruit), meegestuurd zoals de
   eigen status. Nooit vaker dan bij een echte wissel over de draad, en niet in `UiState`
   als hij elke tik kan veranderen. Uit te zetten in de instellingen.
4. **Laatste clip in één klik delen.** Een knop "Share last clip" (en eventueel een
   instelling "clip meteen in de chat zetten") die de net bewaarde clip via de gewone
   bestandsdeling aanbiedt. Clips en bestandsdeling bestaan allebei al; dit is de koppeling,
   geen nieuwe pijplijn. Let op de grootte: een clip van 120 minuten is gigabytes, dus
   alleen aanbieden, nooit automatisch downloaden zoals bij afbeeldingen.
5. **Tekenen op een gedeeld scherm.** Kijkers tekenen op de stream ("ga daarheen") en de
   deler ziet het. Lijnen als kleine control-berichten (punten, kleur, wie), niet in het
   videobeeld gebakken. Bij de kijker tekent het kijkvenster ze zelf over het beeld. Bij de
   deler is een doorklikbaar, transparant venster bovenop het scherm nodig. Dat is het
   grootste werk en het gevoeligste punt voor invariant 4: een spel in exclusive fullscreen
   laat geen venster bovenop toe, en het mag geen frames kosten. Lijnen vervagen vanzelf na
   een paar seconden.

## Overig
- **Camera-opname op macOS.** De camera is op Windows gebouwd (Media Foundation, zie
  `crates/video/src/camera.rs`); op macOS bewust niet. Een mac kan wél naar de camera van
  een Windows-peer *kijken* — op de draad is dat hetzelfde als een gedeeld scherm — maar
  zelf niets uitzenden: `beschikbare_bronnen` noemt daar geen camera's en `Capture::start`
  weigert er een. Bouwen betekent `mac/camera.rs` met AVFoundation
  (`AVCaptureSession` + `AVCaptureVideoDataOutput` op BGRA → `Beeld`, delegate via
  `define_class!` net als de SCK-uitvoer), `NSCameraUsageDescription` in
  `scripts/bundle-mac.sh` en de camera-TCC-prompt. `BronSoort::Camera` bestaat op mac al,
  dus de gedeelde code hoeft er niet voor open.
- **Een gemiste Wordle-dag naspelen.** Bewust dicht: alleen het huidige raadsel neemt gokken
  aan. Kon je een oude dag alsnog spelen, dan haal je een punt op een dag waarop de anderen
  al klaar waren (en met het woord er dan al bij). Zie `docs/OVERDRACHT.md` beslissing 31.
- **Een geluidje of melding bij de Wordle-kaart of bij de uitslag van een ander.** Bewust
  niet: de kaart is geen bericht dat op je wacht, en `geluid.rs` erop aansluiten zou een
  zevende toon betekenen voor iets dat elke ochtend afgaat.
- Remote input control (muis/toetsenbord overnemen, Moonlight-stijl).
- Directe NVENC in plaats van Media Foundation, als de latencymeting in fase 4 daarom vraagt.
- Subkanalen binnen een DM (het oorspronkelijke plan voor fase 9, vóór Rick het omdraaide
  naar subkanalen onder "Algemeen"). Zou dezelfde `TopicId`-aanpak kunnen hergebruiken,
  alleen genest onder `Channel::dm(peer)` in plaats van onder het algemene kanaal.

## Afgewezen, niet alleen uitgesteld
- **YouTube-linkvoorvertoning (thumbnail/titel/kanaalnaam).** Zou alleen kunnen door de
  app een verzoek te laten doen naar een Google/YouTube-API — een uitzondering op de
  vastgelegde regel "nul servers, geen cloud-API". Rick heeft dit expliciet afgewezen
  toen het als vraag voorgelegd werd bij het plannen van fase 8: een gedeelde
  YouTube-link blijft een kale klikbare tekstlink, zonder extern verzoek. Zie
  `ROADMAP.md` fase 8.
- **4:4:4-chroma voor scherpere tekst.** Encoderen kan op Turing (H.264 en HEVC), maar
  geen enkele Turing-GPU — ook de RTX 2080 Super niet — kan H.264- of HEVC-4:4:4
  hardwarematig *decoderen* (NVIDIA's eigen supportmatrix zet dat op "nee" voor de hele
  generatie). Encoderen zonder dat iemand het kan terugzien is nutteloos. Dit is geen
  Store-uitbreiding-probleem zoals bij HEVC-4:2:0-decode: er is geen hardwarepad, punt.
  Alleen heroverwegen als er een GPU-generatie bijkomt die dit wel kan. Zie
  `docs/OVERDRACHT.md`.

## Beveiliging
- QUIC gebruikt nu self-signed certs die niet geverifieerd worden; authenticatie gebeurt
  op de UUID-allowlist ná de handshake. Het tailnet is de echte beveiligingsgrens.
  Certificaat-pinning per peer zou dat kunnen aanscherpen als dat ooit nodig is.
