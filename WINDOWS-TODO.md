# Voor de sessie op de Windows-PC

`main` draait sinds 2026-10-04 op GPUI in plaats van Tauri (beslissing 39 in
`docs/OVERDRACHT.md`). Stand op 2026-10-14:

1. ~~Release-build op Windows~~: gedaan; de eerste build liep stuk op een dubbel manifest
   (opgelost in `build.rs`).
2. Van de handmatige checks uit `docs/TESTPLAN.md`, sectie "GPUI-weergave (2026-10-04)",
   zijn V.1, het rustdeel van V.12 en de eerste helft van V.3 gedaan (zie de resultaten
   daar). **Nog open: V.2 t/m V.11 en het game-deel van V.12**: tray-menu, terug uit de
   tray, titelbalk en snap layouts, slepen uit Verkenner, plakken, meldingen, DPI 125/150%,
   streamtegels, pop-out kijkvenster, CPU naast een game.
3. **1.8.0 is op Rick's verzoek al uitgegeven vóór die checks.** Wat er bij V.2-V.11 stuk
   blijkt: repareren, vastleggen in TESTPLAN, en een 1.8.1 uitgeven (volgorde in
   `docs/OVERDRACHT.md` § "Een release uitgeven").

De openstaande bevindingen van de code-audit staan in `docs/AUDIT-2026-10.md`.

Verwijder dit bestand als punt 2 gedaan is.
