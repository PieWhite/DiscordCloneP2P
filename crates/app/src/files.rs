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

    /// Stopt met een eigen aanbod serveren — bijvoorbeeld omdat de gebruiker het net
    /// als een bericht heeft "verwijderd" (zie `Chat::verwijder_bericht`, generiek voor
    /// elke soort op). Zonder dit zou de `Delete`-op alleen de kaart uit de timeline
    /// laten verdwijnen, terwijl `verzoek_ontvangen` het bestand daarna gewoon nog
    /// levert aan wie er zelf al de `OpId` van kende — schijnzekerheid in plaats van een
    /// echte intrekking. Doet niets als `file` niet iets is dat wij aanbieden (bijv. een
    /// verwijderd bericht, of andermans bestand) — dat is dan gewoon een no-op.
    pub fn verwijder_aanbod(&mut self, file: OpId) {
        self.aangeboden.remove(&file);
    }

    pub fn status(&self, file: OpId) -> Option<&DownloadStatus> {
        self.downloads.get(&file)
    }

    /// B-04: welke downloads wíj hebben aangevraagd en nog lopen.
    ///
    /// Bestaat omdat een inkomende bulkstream verder niets bewijst: hij werd geaccepteerd
    /// zodra er ergens een op met die `OpId` bestond, en de afzender werd weggegooid. Een
    /// peer kon dus ongevraagd een bestand op onze schijf zetten — en dat is precies wat
    /// B-03 van "één klik" naar "nul klikken" bracht. De ontvangtaak legt hier tegenaan of
    /// dit een overdracht is waar wij om gevraagd hebben.
    pub fn lopende_downloads(&self) -> std::collections::HashSet<OpId> {
        self.downloads
            .iter()
            .filter(|(_, s)| matches!(s, DownloadStatus::Bezig { .. }))
            .map(|(id, _)| *id)
            .collect()
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
    ///
    /// Is dit bestand aangeboden op een DM-kanaal, dan mag alleen de geadresseerde het
    /// krijgen. Onder normale omstandigheden komt een aanvraag van iemand anders hier
    /// nooit binnen — de sync laat de `FileMeta`-op zelf al niet bij hem terecht komen —
    /// maar dit is de plek waar we het ook zonder dat vertrouwen zouden afdwingen. Het
    /// antwoord is bewust hetzelfde als "niet meer beschikbaar": een afwijzing die zich
    /// onderscheidt van "bestaat niet" zou juist bevestigen dát het bestaat.
    pub fn verzoek_ontvangen(
        &self,
        van: PeerId,
        req: &FileRequest,
    ) -> (MeshCommand, Option<StartUpload>) {
        if let Some(geadresseerde) = req.file.channel.dm_peer() {
            if geadresseerde != van {
                tracing::warn!(
                    ?van,
                    file = ?req.file,
                    "verzoek om een DM-bestand van iemand anders dan de geadresseerde genegeerd"
                );
                return (
                    MeshCommand::Send {
                        to: van,
                        msg: ControlMsg::FileResponse(FileResponse {
                            file: req.file,
                            outcome: FileOutcome::NOT_AVAILABLE,
                        }),
                    },
                    None,
                );
            }
        }

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

/// Grove extensie-check: alleen de formaten die de `image`-crate-features in
/// `Cargo.toml` ook daadwerkelijk kunnen decoderen.
pub fn is_afbeelding(naam: &str) -> bool {
    let laag = naam.to_ascii_lowercase();
    [".png", ".jpg", ".jpeg", ".gif", ".bmp"]
        .iter()
        .any(|ext| laag.ends_with(ext))
}

/// Content-adresseerbare bestandsnaam voor de `Pictures`-map: de hash die toch al voor
/// verificatie gebruikt wordt (zie `FileEntry::hash`), als hex, met de originele
/// extensie erachter.
///
/// De aanbieder en elke downloadende peer komen zo, zonder iets extra af te spreken, op
/// exact hetzelfde pad uit — dat lost de asymmetrie op die een leesbare naam met
/// `" (2)"`-deduplicatie niet kan oplossen: die naam ligt bij de aanbieder al vóór het
/// downloaden vast, bij de ontvanger pas ná een geslaagde verificatie (zie
/// `docs/OVERDRACHT.md`).
pub fn hash_bestandsnaam(hash: &[u8; 32], oorspronkelijke_naam: &str) -> String {
    let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
    match std::path::Path::new(oorspronkelijke_naam)
        .extension()
        .and_then(|e| e.to_str())
    {
        Some(ext) => format!("{hex}.{}", ext.to_ascii_lowercase()),
        None => hex,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fitcom_proto::Channel;

    fn peer(n: u8) -> PeerId {
        let mut b = [0u8; 16];
        b[0] = n;
        PeerId::from_bytes(b)
    }

    fn entry(author: PeerId) -> FileEntry {
        entry_in(Channel::GENERAL, author)
    }

    fn entry_in(channel: Channel, author: PeerId) -> FileEntry {
        FileEntry {
            id: OpId::new(author, channel, 1),
            author,
            channel,
            name: "test.bin".into(),
            size: 1000,
            hash: [0u8; 32],
            lamport: 1,
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
            file: OpId::new(peer(1), Channel::GENERAL, 1),
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
        let file = OpId::new(peer(1), Channel::GENERAL, 3);
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
    fn dm_bestand_wordt_geweigerd_aan_iemand_anders_dan_de_geadresseerde() {
        let mut f = Files::new();
        let geadresseerde = peer(2);
        let file = OpId::new(peer(1), Channel::dm(geadresseerde), 1);
        f.biedt_aan(file, PathBuf::from("C:/data/prive.zip"));

        let indringer = peer(3);
        let req = FileRequest {
            file,
            have_bytes: 0,
        };
        let (cmd, actie) = f.verzoek_ontvangen(indringer, &req);
        assert!(actie.is_none(), "geen upload naar wie het niet aangaat");
        match cmd {
            MeshCommand::Send {
                to,
                msg: ControlMsg::FileResponse(r),
            } => {
                assert_eq!(to, indringer);
                assert_eq!(
                    r.outcome,
                    FileOutcome::NOT_AVAILABLE,
                    "zelfde antwoord als 'bestaat niet' — anders bevestig je dat het bestaat"
                );
            }
            other => panic!("verkeerd commando: {other:?}"),
        }
    }

    #[test]
    fn verwijder_aanbod_laat_een_volgend_verzoek_not_available_krijgen() {
        let mut f = Files::new();
        let file = OpId::new(peer(1), Channel::GENERAL, 3);
        f.biedt_aan(file, PathBuf::from("C:/data/vakantiefotos.zip"));

        f.verwijder_aanbod(file);

        let van = peer(2);
        let req = FileRequest {
            file,
            have_bytes: 0,
        };
        let (cmd, actie) = f.verzoek_ontvangen(van, &req);
        assert!(
            actie.is_none(),
            "een ingetrokken aanbod mag niets meer serveren"
        );
        match cmd {
            MeshCommand::Send {
                msg: ControlMsg::FileResponse(r),
                ..
            } => assert_eq!(r.outcome, FileOutcome::NOT_AVAILABLE),
            other => panic!("verkeerd commando: {other:?}"),
        }
    }

    #[test]
    fn verwijder_aanbod_van_iets_dat_we_niet_aanbieden_is_een_no_op() {
        let mut f = Files::new();
        f.verwijder_aanbod(OpId::new(peer(1), Channel::GENERAL, 1)); // mag niet paniceren
    }

    #[test]
    fn dm_bestand_wordt_wel_geleverd_aan_de_geadresseerde() {
        let mut f = Files::new();
        let geadresseerde = peer(2);
        let file = OpId::new(peer(1), Channel::dm(geadresseerde), 1);
        let pad = PathBuf::from("C:/data/prive.zip");
        f.biedt_aan(file, pad.clone());

        let req = FileRequest {
            file,
            have_bytes: 0,
        };
        let (_, actie) = f.verzoek_ontvangen(geadresseerde, &req);
        let actie = actie.expect("de geadresseerde moet de upload wel krijgen");
        assert_eq!(actie.pad, pad);
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
    fn is_afbeelding_kijkt_naar_de_extensie_hoofdletterongevoelig() {
        assert!(is_afbeelding("vakantie.PNG"));
        assert!(is_afbeelding("foto.jpg"));
        assert!(!is_afbeelding("document.pdf"));
        assert!(!is_afbeelding("archief.zip"));
    }

    #[test]
    fn hash_bestandsnaam_is_hex_plus_originele_extensie() {
        let hash = [0x11u8; 32];
        assert_eq!(
            hash_bestandsnaam(&hash, "vakantie.PNG"),
            format!("{}.png", "11".repeat(32))
        );
    }

    #[test]
    fn hash_bestandsnaam_zonder_extensie_is_kale_hex() {
        let hash = [0xabu8; 32];
        assert_eq!(hash_bestandsnaam(&hash, "zondereigentijd"), "ab".repeat(32));
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
