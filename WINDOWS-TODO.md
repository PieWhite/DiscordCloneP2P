# Voor de sessie op de Windows-PC

`main` draait sinds 2026-10-04 op GPUI in plaats van Tauri (beslissing 39 in
`docs/OVERDRACHT.md`). Op Windows is hij nog nooit gestart; vanaf de Mac is alleen
gecompileerd. Wat hier moet gebeuren:

1. `git pull`, alle fitcom-instanties sluiten, `cargo build --release -p fitcom`.
   Een release-build heeft `fxc.exe` uit de Windows SDK nodig (zit bij de C++-workload van
   de VS Build Tools); zonder faalt de build.
2. De handmatige checks V.1 t/m V.12 uit `docs/TESTPLAN.md`, sectie "GPUI-weergave
   (2026-10-04)": tray, sluiten naar tray en terug, titelbalk en snap layouts, slepen uit
   Verkenner, plakken, meldingen, DPI 125/150%, streamtegels, pop-out kijkvenster, CPU naast
   een game.
3. Wat stuk is: repareren en vastleggen in TESTPLAN. Pas daarna een release uitgeven
   (volgorde in `docs/OVERDRACHT.md` § "Een release uitgeven").

Verwijder dit bestand als het gedaan is.
