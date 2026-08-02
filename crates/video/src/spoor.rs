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

pub struct Spoor {
    pad: PathBuf,
    regels: Vec<String>,
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
        })
    }

    pub fn regel(&mut self, regel: String) {
        self.regels.push(regel);
    }

    pub fn klaar(&mut self) {
        if self.regels.len() <= 1 {
            return;
        }
        let inhoud = self.regels.join("\n");
        match std::fs::write(&self.pad, inhoud) {
            Ok(()) => {
                tracing::info!(pad = %self.pad.display(), regels = self.regels.len() - 1, "spoor weggeschreven")
            }
            Err(e) => tracing::warn!(error = %e, "spoor niet kunnen schrijven"),
        }
        self.regels.truncate(1);
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
