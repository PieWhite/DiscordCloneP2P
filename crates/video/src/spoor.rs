//! Tijdelijke diagnostiek: één regel per beeld, in het geheugen, aan het eind naar een
//! CSV.
//!
//! Bestaat om de periodieke microhapering te kunnen dateren. De meterregels per seconde
//! middelen precies weg waar het om gaat: één beeld dat 60 ms te laat is verdwijnt in
//! een `spreiding_ms` van een seconde met zestig beelden erin.
//!
//! Staat uit tenzij `FITCOM_SPOOR` naar een map wijst. Schrijven gebeurt pas bij
//! [`Spoor::klaar`], zodat er op het beeldpad geen bestands-I/O staat.

use std::path::PathBuf;

/// Zoveel regels houden we vast. Eén regel per beeld is bij 60 fps 216 000 regels per uur
/// van rond de 90 bytes: ~19 MB per uur per spoor, en er lopen er twee per kijker. De lijst
/// werd pas bij [`Spoor::klaar`] geleegd, dus een lange diagnosesessie liep vol (B-50).
///
/// Een miljoen regels is ruim vier uur op 60 fps en rond de 100 MB — genoeg om een
/// periodieke hapering te dateren, en met een bovengrens die vaststaat.
const MAX_REGELS: usize = 1_000_000;

pub struct Spoor {
    pad: PathBuf,
    regels: Vec<String>,
    /// Hoeveel regels er niet bewaard zijn omdat de lijst vol was. Komt in het log terecht
    /// bij [`Spoor::klaar`], want een afgekapt spoor waarvan je dat niet weet leidt tot de
    /// verkeerde conclusie.
    overgeslagen: u64,
}

impl Spoor {
    /// `None` als `FITCOM_SPOOR` niet gezet is; dan kost dit verder helemaal niets.
    pub fn nieuw(naam: &str, kop: &str) -> Option<Self> {
        let map = PathBuf::from(std::env::var_os("FITCOM_SPOOR")?);
        let _ = std::fs::create_dir_all(&map);
        let mut regels = Vec::with_capacity(64 * 1024);
        regels.push(kop.to_string());
        Some(Self {
            pad: map.join(format!("{naam}.csv")),
            regels,
            overgeslagen: 0,
        })
    }

    pub fn regel(&mut self, regel: String) {
        // Vol is vol: het begin van de sessie is waar de hapering in zit, en doorgroeien
        // is op het beeldpad geen optie (B-50).
        if self.regels.len() >= MAX_REGELS {
            self.overgeslagen += 1;
            return;
        }
        self.regels.push(regel);
    }

    pub fn klaar(&mut self) {
        if self.regels.len() <= 1 {
            return;
        }
        let inhoud = self.regels.join("\n");
        match std::fs::write(&self.pad, inhoud) {
            Ok(()) => {
                tracing::info!(pad = %self.pad.display(), regels = self.regels.len() - 1, overgeslagen = self.overgeslagen, "spoor weggeschreven")
            }
            Err(e) => tracing::warn!(error = %e, "spoor niet kunnen schrijven"),
        }
        self.regels.truncate(1);
        self.overgeslagen = 0;
    }
}

/// Handige korte vorm: doet niets als het spoor uit staat.
macro_rules! spoor {
    ($spoor:expr, $($arg:tt)*) => {
        if let Some(s) = $spoor.as_mut() {
            s.regel(format!($($arg)*));
        }
    };
}
pub(crate) use spoor;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn b50_het_spoor_groeit_niet_onbeperkt() {
        // Zat achter `FITCOM_SPOOR` en dus niet van buiten bereikbaar, maar juist aan
        // tijdens de lange sessies waarin het uitzoekwerk gebeurt.
        // Lege strings: het gaat om het aantal, niet om de inhoud.
        let mut s = Spoor {
            pad: PathBuf::from("/dev/null"),
            regels: vec![String::new(); MAX_REGELS - 1],
            overgeslagen: 0,
        };
        for i in 0..10 {
            s.regel(format!("{i}"));
        }
        assert_eq!(s.regels.len(), MAX_REGELS);
        assert_eq!(s.overgeslagen, 9);
    }
}
