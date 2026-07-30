//! Beslislogica voor automatische updates tussen peers (fase 11).
//!
//! Zelfde opzet als `files.rs`: puur, geen schijf of netwerk hier. `engine.rs` voert uit
//! (de eigen exe hashen, de upload-/downloadtaak spawnen, het updater-proces starten).
//! Zie `docs/ARCHITECTURE.md`, sectie "Automatische updates".
//!
//! Eén slot tegelijk, niet per peer: het gaat om "is er een nieuwere versie om naartoe
//! te gaan", niet om een lijst van aanbieders. Ziet de motor een nog nieuwere versie
//! langskomen dan wat al onderweg is, dan wint die. Een mislukte poging wordt niet actief
//! opnieuw geprobeerd — dat gebeurt vanzelf zodra die peer opnieuw `Online` gaat, net als
//! bij de oplog-sync.

use fitcom_net::MeshCommand;
use fitcom_proto::control::{FileOutcome, UpdateRequest, UpdateResponse};
use fitcom_proto::{ControlMsg, PeerId};
use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub enum UpdateStatus {
    Aangeboden {
        peer: PeerId,
        hun_versie: String,
    },
    Bezig {
        peer: PeerId,
        hun_versie: String,
        ontvangen: u64,
        totaal: u64,
        /// BLAKE3-hash van de aangekondigde update, om de download straks tegen te
        /// verifiëren — zie `engine.rs::download_update_bytes`.
        hash: [u8; 32],
    },
    KlaarOmToeTePassen {
        peer: PeerId,
        hun_versie: String,
        pad: PathBuf,
    },
    Mislukt(String),
}

impl UpdateStatus {
    pub fn hun_versie(&self) -> Option<&str> {
        match self {
            Self::Aangeboden { hun_versie, .. }
            | Self::Bezig { hun_versie, .. }
            | Self::KlaarOmToeTePassen { hun_versie, .. } => Some(hun_versie),
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
}

impl Updates {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn status(&self) -> Option<&UpdateStatus> {
        self.huidig.as_ref()
    }

    /// Een peer bleek net (of nog steeds) een nieuwere versie te draaien dan wijzelf.
    /// Levert het `UpdateRequest` op dat verstuurd moet worden, tenzij we deze versie al
    /// aan het ophalen zijn, al hebben liggen, of de gebruiker hem negeerde.
    ///
    /// `bestaand_op_schijf` is wat er al van een eerdere, onderbroken poging aan precies
    /// deze versie op schijf staat — `0` voor een verse aanvraag.
    pub fn nieuwere_versie_gezien(
        &mut self,
        peer: PeerId,
        hun_versie: &str,
        bestaand_op_schijf: u64,
    ) -> Option<MeshCommand> {
        if self.genegeerd.contains(hun_versie) {
            return None;
        }
        // Al bezig met (of klaar met) exact deze versie: geen tweede aanvraag.
        if self.huidig.as_ref().and_then(|s| s.hun_versie()) == Some(hun_versie) {
            return None;
        }

        self.huidig = Some(UpdateStatus::Aangeboden {
            peer,
            hun_versie: hun_versie.to_string(),
        });
        Some(MeshCommand::Send {
            to: peer,
            msg: ControlMsg::UpdateRequest(UpdateRequest {
                have_bytes: bestaand_op_schijf,
            }),
        })
    }

    /// Antwoord van de peer met de nieuwere versie. `Ready` betekent: er komt zo een
    /// uni-stream aan; die grootte/hash worden bewaard om straks tegen te verifiëren.
    pub fn antwoord_ontvangen(&mut self, resp: &UpdateResponse) {
        let Some(UpdateStatus::Aangeboden { peer, hun_versie }) = &self.huidig else {
            return; // niet (meer) waar we op wachtten
        };
        if resp.outcome == FileOutcome::NOT_AVAILABLE {
            self.huidig = Some(UpdateStatus::Mislukt(
                "peer kon zijn eigen exe niet meer lezen".into(),
            ));
            return;
        }
        self.huidig = Some(UpdateStatus::Bezig {
            peer: *peer,
            hun_versie: hun_versie.clone(),
            ontvangen: 0,
            totaal: resp.size,
            hash: resp.hash,
        });
    }

    pub fn voortgang(&mut self, ontvangen: u64) {
        if let Some(UpdateStatus::Bezig { ontvangen: o, .. }) = &mut self.huidig {
            *o = ontvangen;
        }
    }

    pub fn klaar(&mut self, pad: PathBuf) {
        if let Some(UpdateStatus::Bezig {
            peer, hun_versie, ..
        }) = &self.huidig
        {
            self.huidig = Some(UpdateStatus::KlaarOmToeTePassen {
                peer: *peer,
                hun_versie: hun_versie.clone(),
                pad,
            });
        }
    }

    pub fn mislukt(&mut self, bericht: String) {
        self.huidig = Some(UpdateStatus::Mislukt(bericht));
    }

    /// De gebruiker klikt "negeren": deze versie wordt niet meer vanzelf aangeboden
    /// deze sessie, en het huidige slot (als het over deze versie ging) wordt geleegd.
    pub fn negeer(&mut self, versie: &str) {
        self.genegeerd.insert(versie.to_string());
        if self.huidig.as_ref().and_then(|s| s.hun_versie()) == Some(versie) {
            self.huidig = None;
        }
    }

    /// Een mislukte melding wegklikken. Anders dan `negeer` geen versie om te
    /// onthouden — een `Mislukt`-status draagt er geen — dus dit leegt gewoon het slot;
    /// een volgende `Online` van die peer biedt hem dan opnieuw aan.
    pub fn wis_melding(&mut self) {
        self.huidig = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(n: u8) -> PeerId {
        let mut b = [0u8; 16];
        b[0] = n;
        PeerId::from_bytes(b)
    }

    #[test]
    fn nieuwere_versie_stuurt_update_request_en_zet_status_aangeboden() {
        let mut u = Updates::new();
        let cmd = u.nieuwere_versie_gezien(peer(1), "0.2.0", 0);
        match cmd {
            Some(MeshCommand::Send {
                to,
                msg: ControlMsg::UpdateRequest(r),
            }) => {
                assert_eq!(to, peer(1));
                assert_eq!(r.have_bytes, 0);
            }
            other => panic!("verkeerd commando: {other:?}"),
        }
        assert_eq!(
            u.status(),
            Some(&UpdateStatus::Aangeboden {
                peer: peer(1),
                hun_versie: "0.2.0".into()
            })
        );
    }

    #[test]
    fn zelfde_versie_nogmaals_gezien_stuurt_geen_tweede_aanvraag() {
        let mut u = Updates::new();
        u.nieuwere_versie_gezien(peer(1), "0.2.0", 0);
        let tweede = u.nieuwere_versie_gezien(peer(2), "0.2.0", 0);
        assert!(tweede.is_none(), "eerste peer wint, geen dubbele aanvraag");
    }

    #[test]
    fn genegeerde_versie_wordt_niet_meer_aangeboden() {
        let mut u = Updates::new();
        u.negeer("0.2.0");
        let cmd = u.nieuwere_versie_gezien(peer(1), "0.2.0", 0);
        assert!(cmd.is_none());
        assert_eq!(u.status(), None);
    }

    #[test]
    fn ready_antwoord_zet_status_bezig_met_grootte() {
        let mut u = Updates::new();
        u.nieuwere_versie_gezien(peer(1), "0.2.0", 0);
        u.antwoord_ontvangen(&UpdateResponse {
            outcome: FileOutcome::READY,
            size: 1000,
            hash: [0u8; 32],
        });
        assert_eq!(
            u.status(),
            Some(&UpdateStatus::Bezig {
                peer: peer(1),
                hun_versie: "0.2.0".into(),
                ontvangen: 0,
                totaal: 1000,
                hash: [0u8; 32],
            })
        );
    }

    #[test]
    fn not_available_antwoord_zet_status_mislukt() {
        let mut u = Updates::new();
        u.nieuwere_versie_gezien(peer(1), "0.2.0", 0);
        u.antwoord_ontvangen(&UpdateResponse {
            outcome: FileOutcome::NOT_AVAILABLE,
            size: 0,
            hash: [0u8; 32],
        });
        assert!(matches!(u.status(), Some(UpdateStatus::Mislukt(_))));
    }

    #[test]
    fn voortgang_werkt_alleen_bezig_status_bij() {
        let mut u = Updates::new();
        u.voortgang(500); // niets aan de gang: geen effect, geen paniek
        assert_eq!(u.status(), None);

        u.nieuwere_versie_gezien(peer(1), "0.2.0", 0);
        u.antwoord_ontvangen(&UpdateResponse {
            outcome: FileOutcome::READY,
            size: 1000,
            hash: [0u8; 32],
        });
        u.voortgang(500);
        assert_eq!(
            u.status(),
            Some(&UpdateStatus::Bezig {
                peer: peer(1),
                hun_versie: "0.2.0".into(),
                ontvangen: 500,
                totaal: 1000,
                hash: [0u8; 32],
            })
        );
    }

    #[test]
    fn klaar_zet_status_klaar_om_toe_te_passen_met_pad() {
        let mut u = Updates::new();
        u.nieuwere_versie_gezien(peer(1), "0.2.0", 0);
        u.antwoord_ontvangen(&UpdateResponse {
            outcome: FileOutcome::READY,
            size: 1000,
            hash: [0u8; 32],
        });
        let pad = PathBuf::from("C:/data/updates/update-0.2.0.exe");
        u.klaar(pad.clone());
        assert_eq!(
            u.status(),
            Some(&UpdateStatus::KlaarOmToeTePassen {
                peer: peer(1),
                hun_versie: "0.2.0".into(),
                pad
            })
        );
    }

    #[test]
    fn negeren_van_de_lopende_versie_leegt_het_slot() {
        let mut u = Updates::new();
        u.nieuwere_versie_gezien(peer(1), "0.2.0", 0);
        u.negeer("0.2.0");
        assert_eq!(u.status(), None);

        // En daarna komt hij niet vanzelf terug.
        let cmd = u.nieuwere_versie_gezien(peer(1), "0.2.0", 0);
        assert!(cmd.is_none());
    }

    #[test]
    fn wis_melding_leegt_een_mislukte_status_zonder_de_versie_te_negeren() {
        let mut u = Updates::new();
        u.nieuwere_versie_gezien(peer(1), "0.2.0", 0);
        u.antwoord_ontvangen(&UpdateResponse {
            outcome: FileOutcome::NOT_AVAILABLE,
            size: 0,
            hash: [0u8; 32],
        });
        u.wis_melding();
        assert_eq!(u.status(), None);

        // Geen negeer-effect: dezelfde versie mag opnieuw aangeboden worden.
        let cmd = u.nieuwere_versie_gezien(peer(1), "0.2.0", 0);
        assert!(cmd.is_some());
    }

    #[test]
    fn nieuwere_versie_dan_de_lopende_wint() {
        let mut u = Updates::new();
        u.nieuwere_versie_gezien(peer(1), "0.2.0", 0);
        let cmd = u.nieuwere_versie_gezien(peer(2), "0.3.0", 0);
        assert!(
            cmd.is_some(),
            "een nog nieuwere versie moet alsnog aangevraagd worden"
        );
        assert_eq!(
            u.status(),
            Some(&UpdateStatus::Aangeboden {
                peer: peer(2),
                hun_versie: "0.3.0".into()
            })
        );
    }
}
