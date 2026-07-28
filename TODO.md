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
- 4:4:4-chroma-modus voor extra scherpe tekst in screenshare (fase 5).
- Grid-weergave van meerdere streams in het hoofdvenster (fase 5).
- Directe NVENC in plaats van Media Foundation, als de latencymeting in fase 4 daarom vraagt.

## Beveiliging
- QUIC gebruikt nu self-signed certs die niet geverifieerd worden; authenticatie gebeurt
  op de UUID-allowlist ná de handshake. Het tailnet is de echte beveiligingsgrens.
  Certificaat-pinning per peer zou dat kunnen aanscherpen als dat ooit nodig is.
