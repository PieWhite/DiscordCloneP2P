# TODO / Backlog

Bewust niet in v1. De architectuur moet deze items kunnen opnemen zonder herontwerp.

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
