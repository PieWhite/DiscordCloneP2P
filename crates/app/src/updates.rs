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
    /// Er wordt nu bij de feed gekeken. Alleen na een druk op de knop: een automatische
    /// check hoort onzichtbaar te zijn.
    Zoeken,
    Bezig {
        versie: String,
        ontvangen: u64,
        totaal: u64,
    },
    KlaarOmToeTePassen {
        versie: String,
        pad: PathBuf,
        /// B-20: de hash waartegen de download geverifieerd is, zodat de updater hem vlak
        /// vóór het vervangen nóg een keer kan leggen. Het gat tussen "geverifieerd" en
        /// "toegepast" is onbegrensd — de gebruiker klikt wanneer het hem uitkomt — en
        /// alles wat in dat venster in de updatemap kan schrijven wisselt anders de payload
        /// om zonder dat iemand het merkt.
        hash: [u8; 32],
    },
    /// De feed was bereikbaar en had niets nieuwers. Ook alleen na een druk op de knop —
    /// zonder dit antwoord lijkt de knop stuk.
    Actueel,
    Mislukt(String),
}

impl UpdateStatus {
    pub fn versie(&self) -> Option<&str> {
        match self {
            Self::Bezig { versie, .. } | Self::KlaarOmToeTePassen { versie, .. } => Some(versie),
            Self::Zoeken | Self::Actueel | Self::Mislukt(_) => None,
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
    /// Of de lopende check op verzoek van de gebruiker is. Bepaalt of "niets gevonden"
    /// een antwoord verdient of stilzwijgend voorbijgaat.
    handmatig: bool,
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

    /// `handmatig` betekent: de gebruiker heeft erom gevraagd, dus elke uitkomst — ook
    /// "niets nieuws" en "feed onbereikbaar" — hoort zichtbaar te worden. Een
    /// automatische check laat het slot verder ongemoeid.
    pub fn zoeken_gestart(&mut self, handmatig: bool) {
        self.bezig = true;
        self.handmatig = handmatig;
        if handmatig {
            self.huidig = Some(UpdateStatus::Zoeken);
        }
    }

    /// Of de lopende check er een is waar de gebruiker om gevraagd heeft.
    pub fn is_handmatig(&self) -> bool {
        self.handmatig
    }

    /// Wat de check-taak moet overslaan.
    pub fn genegeerde_versies(&self) -> HashSet<String> {
        self.genegeerd.clone()
    }

    /// De feed had een nieuwere, getekende versie en de download is begonnen.
    pub fn gestart(&mut self, versie: String, totaal: u64) {
        self.handmatig = false;
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

    pub fn klaar(&mut self, pad: PathBuf, hash: [u8; 32]) {
        self.bezig = false;
        self.handmatig = false;
        if let Some(UpdateStatus::Bezig { versie, .. }) = &self.huidig {
            self.huidig = Some(UpdateStatus::KlaarOmToeTePassen {
                versie: versie.clone(),
                pad,
                hash,
            });
        }
    }

    pub fn mislukt(&mut self, bericht: String) {
        self.bezig = false;
        self.handmatig = false;
        self.huidig = Some(UpdateStatus::Mislukt(bericht));
    }

    /// De feed was bereikbaar maar had niets nieuws (of alleen iets genegeerds).
    ///
    /// Bij een automatische check laat dit een eerdere melding staan en gaat alleen het
    /// slot weer open. Bij een handmatige check is "niets nieuws" het antwoord waar de
    /// gebruiker op wacht, dus dan komt het in beeld.
    pub fn niets_gevonden(&mut self) {
        self.bezig = false;
        if std::mem::take(&mut self.handmatig) {
            self.huidig = Some(UpdateStatus::Actueel);
        } else if self.huidig == Some(UpdateStatus::Zoeken) {
            self.huidig = None;
        }
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
        u.zoeken_gestart(false);
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
        u.klaar(pad.clone(), [7u8; 32]);
        assert_eq!(
            u.status(),
            Some(&UpdateStatus::KlaarOmToeTePassen {
                versie: "0.3.0".into(),
                pad,
                // B-20: de hash reist mee tot aan de updater.
                hash: [7u8; 32],
            })
        );
    }

    #[test]
    fn klaarstaande_update_blokkeert_een_nieuwe_check() {
        let mut u = Updates::new();
        u.zoeken_gestart(false);
        u.gestart("0.3.0".into(), 10);
        u.klaar(PathBuf::from("x"), [0u8; 32]);
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
        u.zoeken_gestart(false);
        u.mislukt("feed onbereikbaar".into());
        assert!(matches!(u.status(), Some(UpdateStatus::Mislukt(_))));
        assert!(u.mag_zoeken(), "een volgende tik mag het opnieuw proberen");
    }

    #[test]
    fn niets_gevonden_geeft_het_slot_vrij_zonder_status() {
        let mut u = Updates::new();
        u.zoeken_gestart(false);
        u.niets_gevonden();
        assert_eq!(u.status(), None);
        assert!(u.mag_zoeken());
    }

    #[test]
    fn negeren_leegt_het_slot_en_belandt_in_de_overslaglijst() {
        let mut u = Updates::new();
        u.zoeken_gestart(false);
        u.gestart("0.3.0".into(), 10);
        u.klaar(PathBuf::from("x"), [0u8; 32]);

        u.negeer("0.3.0");
        assert_eq!(u.status(), None);
        assert!(u.genegeerde_versies().contains("0.3.0"));
        assert!(u.mag_zoeken());
    }

    #[test]
    fn een_handmatige_check_antwoordt_ook_als_er_niets_nieuws_is() {
        // Zonder dit doet de knop "Check for updates" niets zichtbaars, en dat is niet te
        // onderscheiden van "de knop is stuk" — precies de klacht die dit moet oplossen.
        let mut u = Updates::new();
        u.zoeken_gestart(true);
        assert_eq!(u.status(), Some(&UpdateStatus::Zoeken));
        u.niets_gevonden();
        assert_eq!(u.status(), Some(&UpdateStatus::Actueel));
        assert!(u.mag_zoeken(), "nog een keer kijken mag altijd");
    }

    #[test]
    fn een_automatische_check_zonder_nieuws_blijft_onzichtbaar() {
        let mut u = Updates::new();
        u.zoeken_gestart(false);
        u.niets_gevonden();
        assert_eq!(u.status(), None);
    }

    #[test]
    fn een_automatische_check_wist_het_antwoord_van_een_handmatige_niet() {
        // De tik van zes uur mag "je bent bij" niet zomaar van het scherm halen, maar hij
        // mag er ook geen stale "Zoeken" laten staan.
        let mut u = Updates::new();
        u.zoeken_gestart(true);
        u.niets_gevonden();
        u.zoeken_gestart(false);
        assert_eq!(
            u.status(),
            Some(&UpdateStatus::Actueel),
            "een automatische check hoort het beeld niet te veranderen"
        );
        u.niets_gevonden();
        assert_eq!(u.status(), Some(&UpdateStatus::Actueel));
    }

    #[test]
    fn wis_melding_negeert_de_versie_niet() {
        let mut u = Updates::new();
        u.zoeken_gestart(false);
        u.mislukt("stuk".into());
        u.wis_melding();
        assert_eq!(u.status(), None);
        assert!(u.genegeerde_versies().is_empty());
    }
}
