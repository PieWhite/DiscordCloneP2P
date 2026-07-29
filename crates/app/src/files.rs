//! Bestanden aanbieden en downloaden.
//!
//! Aanbieden is een gewone oplog-op (`OpKind::FileMeta` via `fitcom_store::offer_file`),
//! dus dat verspreidt zich gratis mee via dezelfde sync als chat — ook naar een peer die
//! pas veel later online komt. Wat hier staat is alleen het puntje dat daarna nog moet:
//! de daadwerkelijke bytes ophalen bij de aanbieder.
//!
//! Net als `streams.rs` neemt deze module de beslissingen zonder zelf schijf of netwerk
//! aan te raken; `engine.rs` voert uit (bestand lezen/schrijven, hashen, de uni-stream
//! openen). Zie `docs/ARCHITECTURE.md` voor waarom de bulkbytes over een eigen QUIC-stream
//! gaan in plaats van over de control-stream.

use fitcom_net::MeshCommand;
use fitcom_proto::control::{FileOutcome, FileRequest, FileResponse};
use fitcom_proto::{ControlMsg, OpId, PeerId};
use fitcom_store::FileEntry;
use std::collections::HashMap;
use std::path::PathBuf;

/// Toestand van een download, voor de UI.
#[derive(Debug, Clone, PartialEq)]
pub enum DownloadStatus {
    Bezig { ontvangen: u64, totaal: u64 },
    Voltooid,
    Mislukt(String),
}

/// Wat de motor moet doen als gevolg van een binnengekomen verzoek: de upload starten.
/// Er is maar één soort actie, dus geen aparte enum eromheen — dat zou hier alleen ruis zijn.
#[derive(Debug, Clone)]
pub struct StartUpload {
    pub naar: PeerId,
    pub file: OpId,
    pub pad: PathBuf,
    pub vanaf: u64,
}

#[derive(Debug, Default)]
pub struct Files {
    /// Bestanden die wij aanbieden: waar het originele bestand op deze pc staat. Alleen
    /// bekend voor bestanden die wíj hebben aangeboden — een andere peer die hetzelfde
    /// bestand aanbiedt heeft zijn eigen entry met een ander pad.
    aangeboden: HashMap<OpId, PathBuf>,
    /// Downloads die wij aan het doen zijn of gedaan hebben.
    downloads: HashMap<OpId, DownloadStatus>,
}

impl Files {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn biedt_aan(&mut self, file: OpId, pad: PathBuf) {
        self.aangeboden.insert(file, pad);
    }

    pub fn status(&self, file: OpId) -> Option<&DownloadStatus> {
        self.downloads.get(&file)
    }

    pub fn zet_status(&mut self, file: OpId, status: DownloadStatus) {
        self.downloads.insert(file, status);
    }

    /// Werkt alleen het ontvangen-teller bij van een lopende download. Doet niets als
    /// de download niet (meer) `Bezig` is — bijvoorbeeld als hij intussen al mislukt is.
    pub fn zet_voortgang(&mut self, file: OpId, ontvangen: u64) {
        if let Some(DownloadStatus::Bezig { ontvangen: o, .. }) = self.downloads.get_mut(&file) {
            *o = ontvangen;
        }
    }

    /// De gebruiker klikt op downloaden (of hervatten). `bestaand` is wat er al op schijf
    /// staat van een eerdere, onderbroken poging — `0` voor een verse download.
    pub fn download_aanvragen(&mut self, entry: &FileEntry, bestaand: u64) -> MeshCommand {
        self.downloads.insert(
            entry.id,
            DownloadStatus::Bezig {
                ontvangen: bestaand,
                totaal: entry.size,
            },
        );
        MeshCommand::Send {
            to: entry.author,
            msg: ControlMsg::FileRequest(FileRequest {
                file: entry.id,
                have_bytes: bestaand,
            }),
        }
    }

    /// Een `FileRequest` van een ander binnengekomen. Levert het antwoord dat terug moet,
    /// plus — als we het bestand nog hebben — de opdracht om de upload te starten.
    pub fn verzoek_ontvangen(
        &self,
        van: PeerId,
        req: &FileRequest,
    ) -> (MeshCommand, Option<StartUpload>) {
        match self.aangeboden.get(&req.file) {
            Some(pad) => (
                MeshCommand::Send {
                    to: van,
                    msg: ControlMsg::FileResponse(FileResponse {
                        file: req.file,
                        outcome: FileOutcome::READY,
                    }),
                },
                Some(StartUpload {
                    naar: van,
                    file: req.file,
                    pad: pad.clone(),
                    vanaf: req.have_bytes,
                }),
            ),
            None => (
                MeshCommand::Send {
                    to: van,
                    msg: ControlMsg::FileResponse(FileResponse {
                        file: req.file,
                        outcome: FileOutcome::NOT_AVAILABLE,
                    }),
                },
                None,
            ),
        }
    }

    /// Antwoord van de aanbieder op onze aanvraag. `Ready` betekent: er komt zo een
    /// uni-stream aan, en die wordt los afgehandeld zodra hij binnenkomt. `NotAvailable`
    /// is meteen het einde van deze poging.
    pub fn antwoord_ontvangen(&mut self, resp: &FileResponse) {
        if resp.outcome == FileOutcome::NOT_AVAILABLE {
            self.downloads.insert(
                resp.file,
                DownloadStatus::Mislukt("bestand is niet meer beschikbaar bij de aanbieder".into()),
            );
        }
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

    fn entry(author: PeerId) -> FileEntry {
        FileEntry {
            id: OpId::new(author, 1),
            author,
            name: "test.bin".into(),
            size: 1000,
            hash: [0u8; 32],
        }
    }

    #[test]
    fn download_aanvragen_stuurt_naar_de_aanbieder_met_het_hervatpunt() {
        let mut f = Files::new();
        let e = entry(peer(1));
        let cmd = f.download_aanvragen(&e, 400);
        match cmd {
            MeshCommand::Send {
                to,
                msg: ControlMsg::FileRequest(r),
            } => {
                assert_eq!(to, peer(1));
                assert_eq!(r.file, e.id);
                assert_eq!(r.have_bytes, 400);
            }
            other => panic!("verkeerd commando: {other:?}"),
        }
        assert_eq!(
            f.status(e.id),
            Some(&DownloadStatus::Bezig {
                ontvangen: 400,
                totaal: 1000
            })
        );
    }

    #[test]
    fn verzoek_voor_een_bestand_dat_we_niet_aanbieden_krijgt_not_available() {
        let f = Files::new();
        let van = peer(2);
        let req = FileRequest {
            file: OpId::new(peer(1), 1),
            have_bytes: 0,
        };
        let (cmd, actie) = f.verzoek_ontvangen(van, &req);
        assert!(actie.is_none());
        match cmd {
            MeshCommand::Send {
                to,
                msg: ControlMsg::FileResponse(r),
            } => {
                assert_eq!(to, van);
                assert_eq!(r.outcome, FileOutcome::NOT_AVAILABLE);
            }
            other => panic!("verkeerd commando: {other:?}"),
        }
    }

    #[test]
    fn verzoek_voor_een_bestand_dat_we_aanbieden_start_de_upload_vanaf_het_hervatpunt() {
        let mut f = Files::new();
        let file = OpId::new(peer(1), 3);
        let pad = PathBuf::from("C:/data/vakantiefotos.zip");
        f.biedt_aan(file, pad.clone());

        let van = peer(2);
        let req = FileRequest {
            file,
            have_bytes: 250,
        };
        let (cmd, actie) = f.verzoek_ontvangen(van, &req);

        match cmd {
            MeshCommand::Send {
                to,
                msg: ControlMsg::FileResponse(r),
            } => {
                assert_eq!(to, van);
                assert_eq!(r.outcome, FileOutcome::READY);
            }
            other => panic!("verkeerd commando: {other:?}"),
        }
        let actie = actie.expect("upload had moeten starten");
        assert_eq!(actie.naar, van);
        assert_eq!(actie.file, file);
        assert_eq!(actie.pad, pad);
        assert_eq!(actie.vanaf, 250);
    }

    #[test]
    fn voortgang_raakt_een_niet_lopende_download_niet_aan() {
        let mut f = Files::new();
        let e = entry(peer(1));
        // Nog niet gestart: er is niets om bij te werken.
        f.zet_voortgang(e.id, 500);
        assert_eq!(f.status(e.id), None);

        f.download_aanvragen(&e, 0);
        f.zet_voortgang(e.id, 500);
        assert_eq!(
            f.status(e.id),
            Some(&DownloadStatus::Bezig {
                ontvangen: 500,
                totaal: 1000
            })
        );

        f.zet_status(e.id, DownloadStatus::Voltooid);
        f.zet_voortgang(e.id, 999); // te laat, telt niet meer
        assert_eq!(f.status(e.id), Some(&DownloadStatus::Voltooid));
    }

    #[test]
    fn not_available_antwoord_zet_de_download_op_mislukt() {
        let mut f = Files::new();
        let e = entry(peer(1));
        f.download_aanvragen(&e, 0);
        f.antwoord_ontvangen(&FileResponse {
            file: e.id,
            outcome: FileOutcome::NOT_AVAILABLE,
        });
        assert!(matches!(f.status(e.id), Some(DownloadStatus::Mislukt(_))));
    }

    #[test]
    fn ready_antwoord_laat_de_bezig_status_ongemoeid() {
        // De voortgang wordt bijgewerkt zodra de bytes zelf binnenkomen, niet door dit
        // antwoord — dat zegt alleen "er komt een stream aan".
        let mut f = Files::new();
        let e = entry(peer(1));
        f.download_aanvragen(&e, 0);
        f.antwoord_ontvangen(&FileResponse {
            file: e.id,
            outcome: FileOutcome::READY,
        });
        assert_eq!(
            f.status(e.id),
            Some(&DownloadStatus::Bezig {
                ontvangen: 0,
                totaal: 1000
            })
        );
    }
}
