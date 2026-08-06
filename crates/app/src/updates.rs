//! Beslislogica voor automatische updates uit de release-feed (fase 13).
//!
//! Zelfde opzet als `files.rs`: puur, geen schijf of netwerk hier. `engine.rs` voert uit
//! (de feed ophalen, de handtekening controleren, downloaden, het updater-proces
//! starten). Het ophalen en verifiëren zelf staat in `crate::release`.
//!
//! Eén slot tegelijk: het gaat om "is er een nieuwere versie om naartoe te gaan", niet om
//! een lijst aanbieders. Vóór fase 13 kwam die versie van een peer; sinds de wisseling
//! naar een getekende feed is er nog maar één bron en is `peer` uit dit bestand verdwenen
//! (zie `docs/BEVEILIGING.md`, B-01).
//!
//! Een mislukte poging wordt niet actief opnieuw geprobeerd — dat gebeurt vanzelf bij de
//! volgende periodieke check, net als bij de oplog-sync.

use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub enum UpdateStatus {
    Bezig {
        versie: String,
        ontvangen: u64,
        totaal: u64,
    },
    KlaarOmToeTePassen {
        versie: String,
        pad: PathBuf,
    },
    Mislukt(String),
}

impl UpdateStatus {
    pub fn versie(&self) -> Option<&str> {
        match self {
            Self::Bezig { versie, .. } | Self::KlaarOmToeTePassen { versie, .. } => Some(versie),
            Self::Mislukt(_) => None,
        }
    }
}

#[derive(Debug, Default)]
pub struct Updates {
    huidig: Option<UpdateStatus>,
    /// Versies die de gebruiker expliciet weggeklikt heeft. Sessie-lokaal, geen config —
    /// zelfde soort keuze als niet-storen/mute: verdwijnt bij een herstart, en dan wordt
    /// een nog steeds nieuwere versie gewoon opnieuw aangeboden.
    genegeerd: HashSet<String>,
    /// Er loopt een check of download. Voorkomt dat de periodieke tik er een tweede
    /// naast start terwijl de eerste nog bezig is.
    bezig: bool,
}

impl Updates {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn status(&self) -> Option<&UpdateStatus> {
        self.huidig.as_ref()
    }

    /// Mag er nu een check starten? Niet als er al één loopt, en niet als er al een
    /// gecontroleerde update klaarstaat — die moet eerst toegepast of weggeklikt worden.
    pub fn mag_zoeken(&self) -> bool {
        !self.bezig && !matches!(self.huidig, Some(UpdateStatus::KlaarOmToeTePassen { .. }))
    }

    pub fn zoeken_gestart(&mut self) {
        self.bezig = true;
    }

    /// Wat de check-taak moet overslaan.
    pub fn genegeerde_versies(&self) -> HashSet<String> {
        self.genegeerd.clone()
    }

    /// De feed had een nieuwere, getekende versie en de download is begonnen.
    pub fn gestart(&mut self, versie: String, totaal: u64) {
        self.huidig = Some(UpdateStatus::Bezig {
            versie,
            ontvangen: 0,
            totaal,
        });
    }

    pub fn voortgang(&mut self, ontvangen: u64) {
        if let Some(UpdateStatus::Bezig { ontvangen: o, .. }) = &mut self.huidig {
            *o = ontvangen;
        }
    }

    pub fn klaar(&mut self, pad: PathBuf) {
        self.bezig = false;
        if let Some(UpdateStatus::Bezig { versie, .. }) = &self.huidig {
            self.huidig = Some(UpdateStatus::KlaarOmToeTePassen {
                versie: versie.clone(),
                pad,
            });
        }
    }

    pub fn mislukt(&mut self, bericht: String) {
        self.bezig = false;
        self.huidig = Some(UpdateStatus::Mislukt(bericht));
    }

    /// De feed was bereikbaar maar had niets nieuws (of alleen iets genegeerds). Laat een
    /// eerdere melding staan; alleen het slot voor een volgende check gaat weer open.
    pub fn niets_gevonden(&mut self) {
        self.bezig = false;
    }

    /// De gebruiker klikt "negeren": deze versie wordt niet meer vanzelf aangeboden
    /// deze sessie, en het huidige slot (als het over deze versie ging) wordt geleegd.
    pub fn negeer(&mut self, versie: &str) {
        self.genegeerd.insert(versie.to_string());
        if self.huidig.as_ref().and_then(|s| s.versie()) == Some(versie) {
            self.huidig = None;
        }
    }

    /// Een mislukte melding wegklikken. Anders dan `negeer` geen versie om te
    /// onthouden — een `Mislukt`-status draagt er geen — dus dit leegt gewoon het slot;
    /// de volgende periodieke check biedt hem dan opnieuw aan.
    pub fn wis_melding(&mut self) {
        self.huidig = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_loopt_van_bezig_naar_klaar() {
        let mut u = Updates::new();
        assert!(u.mag_zoeken());
        u.zoeken_gestart();
        assert!(!u.mag_zoeken(), "geen tweede check naast een lopende");

        u.gestart("0.3.0".into(), 1000);
        u.voortgang(500);
        assert_eq!(
            u.status(),
            Some(&UpdateStatus::Bezig {
                versie: "0.3.0".into(),
                ontvangen: 500,
                totaal: 1000,
            })
        );

        let pad = PathBuf::from("C:/data/updates/update-0.3.0.exe");
        u.klaar(pad.clone());
        assert_eq!(
            u.status(),
            Some(&UpdateStatus::KlaarOmToeTePassen {
                versie: "0.3.0".into(),
                pad
            })
        );
    }

    #[test]
    fn klaarstaande_update_blokkeert_een_nieuwe_check() {
        let mut u = Updates::new();
        u.zoeken_gestart();
        u.gestart("0.3.0".into(), 10);
        u.klaar(PathBuf::from("x"));
        assert!(
            !u.mag_zoeken(),
            "eerst toepassen of wegklikken, anders halen we hem er twee keer bij"
        );
    }

    #[test]
    fn voortgang_zonder_lopende_download_doet_niets() {
        let mut u = Updates::new();
        u.voortgang(500);
        assert_eq!(u.status(), None);
    }

    #[test]
    fn mislukking_geeft_het_slot_weer_vrij() {
        let mut u = Updates::new();
        u.zoeken_gestart();
        u.mislukt("feed onbereikbaar".into());
        assert!(matches!(u.status(), Some(UpdateStatus::Mislukt(_))));
        assert!(u.mag_zoeken(), "een volgende tik mag het opnieuw proberen");
    }

    #[test]
    fn niets_gevonden_geeft_het_slot_vrij_zonder_status() {
        let mut u = Updates::new();
        u.zoeken_gestart();
        u.niets_gevonden();
        assert_eq!(u.status(), None);
        assert!(u.mag_zoeken());
    }

    #[test]
    fn negeren_leegt_het_slot_en_belandt_in_de_overslaglijst() {
        let mut u = Updates::new();
        u.zoeken_gestart();
        u.gestart("0.3.0".into(), 10);
        u.klaar(PathBuf::from("x"));

        u.negeer("0.3.0");
        assert_eq!(u.status(), None);
        assert!(u.genegeerde_versies().contains("0.3.0"));
        assert!(u.mag_zoeken());
    }

    #[test]
    fn wis_melding_negeert_de_versie_niet() {
        let mut u = Updates::new();
        u.zoeken_gestart();
        u.mislukt("stuk".into());
        u.wis_melding();
        assert_eq!(u.status(), None);
        assert!(u.genegeerde_versies().is_empty());
    }
}
