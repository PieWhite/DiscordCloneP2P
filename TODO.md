# TODO / Backlog

Bewust niet in v1. De architectuur moet deze items kunnen opnemen zonder herontwerp.

## File sharing tussen de drie peers — hoofdbacklog-item
Expliciet uitgesteld, expliciet voorzien in het ontwerp.

**Aanhaakpunten die al bestaan:**
- `ControlMsg` heeft gereserveerde varianten `FileOffer` / `FileAccept` / `FileChunkAck`
  (zie `docs/ARCHITECTURE.md`). Toevoegen aan het eind van de enum breekt niets.
- QUIC ondersteunt meerdere onafhankelijke streams per verbinding. Bulk file-bytes gaan
  over een eigen stream en blokkeren chat of control dus niet. Dit is precies de reden
  dat we QUIC gekozen hebben in plaats van één TCP-socket.
- De oplog is generiek, niet chat-specifiek. Filemetadata wordt een nieuwe `OpKind`
  (`FileMeta { name, size, hash, offered_by }`) en synchroniseert dan gratis mee via
  hetzelfde version-vector-mechanisme. Geen schema-migratie nodig.

**Wat er dan nog moet gebeuren:** resume na onderbreking, hash-verificatie,
voortgangs-UI, en een keuze waar bestanden landen.

## Overig
- Remote input control (muis/toetsenbord overnemen, Moonlight-stijl).
- Chat: reacties, replies, afbeeldingen plakken. Worden nieuwe `OpKind`-varianten.
- Meerdere chatkanalen.
- Push-to-talk met globale hotkey. Voorzien in het ontwerp, niet gebouwd.
- Directe NVENC in plaats van Media Foundation, als de latencymeting in fase 4 daarom vraagt.

## Afgewezen, niet alleen uitgesteld
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
