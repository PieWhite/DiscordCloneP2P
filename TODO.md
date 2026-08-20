# TODO / Backlog

Bewust niet in v1. De architectuur moet deze items kunnen opnemen zonder herontwerp.

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
- Chat: reacties, replies. Worden nieuwe `OpKind`-varianten. (Afbeeldingen plakken staat
  nu in `ROADMAP.md` fase 8, niet meer hier.)
- Push-to-talk met globale hotkey. Voorzien in het ontwerp, niet gebouwd.
- Directe NVENC in plaats van Media Foundation, als de latencymeting in fase 4 daarom vraagt.
- Groepskanalen (meer dan twee peers, maar niet iedereen). Dit is iets anders dan de
  naamgevbare subkanalen onder "Algemeen" uit fase 9: die zijn voor iedereen zichtbaar,
  net als het algemene kanaal zelf. Hier gaat het om een kanaal met een subset van de
  peers die niet iedereen omvat — vergelijkbaar met een DM, maar met meer dan twee
  deelnemers. `Channel` is al getagd (`tag`, net als `StreamKind`/`FileOutcome`) zodat dit
  later als nieuwe waarde bij kan zonder een protocolbreuk — zie `docs/ARCHITECTURE.md`,
  sectie "Kanalen".
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
