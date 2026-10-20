//! Uitnodigingscodes in de app (fase B1 van `docs/PLAN-ZONDER-TAILSCALE.md`).
//!
//! Het formaat staat in `fitcom_proto::uitnodiging`; hier staat alleen de regel welke peer
//! uit `config.toml` een geplakte code is. Pure beslislogica: de motor voert uit.
//!
//! - Eerst op `PeerId`, dan op de sleutel, dan op een adres in de code dat het configadres van
//!   een peer zonder `PeerId` is (een tailnet-peer die nog nooit verbonden was).
//! - Een code waarvan de sleutel of `PeerId` afwijkt van wat er vastgepind staat wordt
//!   geweigerd: de app kan een gestolen identiteit niet van een herinstallatie onderscheiden
//!   (zie `docs/BEVEILIGING.md`, addendum 2026-10-19). Wie echt opnieuw installeerde haalt
//!   eerst `known_id` en `public_key` uit `config.toml`.
//! - De eigen code wordt geweigerd: dan zou de mesh zichzelf bellen.

use crate::config::PeerConfig;
use fitcom_proto::uitnodiging::{CodeFout, Uitnodiging};
use fitcom_proto::PeerId;
use std::net::IpAddr;

/// Bij welke peer een code hoort.
#[derive(Debug, PartialEq, Eq)]
pub enum Koppeling {
    /// Deze peer staat al in de config, op deze plek.
    Bestaand(usize),
    /// Nog niet: er komt een peer bij.
    Nieuw,
}

/// Waarom een code niet gebruikt is. Het venster maakt er Engelse zinnen van.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeWeigering {
    Onleesbaar(CodeFout),
    EigenCode,
    /// Deze peer staat al vast met een andere sleutel of `PeerId`.
    SleutelWijkt,
    /// De netwerklaag nam het niet aan (zijn wachtrij was vol); opnieuw plakken helpt.
    Bezet,
}

/// Wat er van de laatste keer plakken terechtkwam.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CodesUitslag {
    pub toegevoegd: Vec<String>,
    pub bijgewerkt: Vec<String>,
    /// De naam uit de code als die te lezen was, anders leeg.
    pub geweigerd: Vec<(String, CodeWeigering)>,
    /// Er stond nergens `fitcom1:` in.
    pub geen_code: bool,
}

pub fn koppel(
    peers: &[PeerConfig],
    me: PeerId,
    u: &Uitnodiging,
) -> Result<Koppeling, CodeWeigering> {
    if u.peer_id == me {
        return Err(CodeWeigering::EigenCode);
    }
    // Een peer zonder `PeerId` en zonder sleutel (nooit verbonden, of de gebruiker haalde ze
    // weg na een herinstallatie): herkenbaar aan een adres dat ook in de code staat.
    let op_adres = |p: &PeerConfig| {
        p.known_id.is_none()
            && p.vastgepinde_sleutel().is_none()
            && (p
                .address
                .parse::<IpAddr>()
                .is_ok_and(|ip| u.adressen.iter().any(|a| a.ip() == ip))
                || p.addresses.iter().any(|a| u.adressen.contains(a)))
    };
    let plek = peers
        .iter()
        .position(|p| p.known_id == Some(u.peer_id))
        .or_else(|| {
            peers
                .iter()
                .position(|p| p.vastgepinde_sleutel() == Some(u.public_key))
        })
        .or_else(|| peers.iter().position(op_adres));
    let Some(i) = plek else {
        return Ok(Koppeling::Nieuw);
    };
    let p = &peers[i];
    if p.vastgepinde_sleutel().is_some_and(|k| k != u.public_key)
        || p.known_id.is_some_and(|k| k != u.peer_id)
    {
        return Err(CodeWeigering::SleutelWijkt);
    }
    Ok(Koppeling::Bestaand(i))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::release::bytes_naar_hex;

    fn peer(address: &str, id: Option<PeerId>, sleutel: Option<[u8; 32]>) -> PeerConfig {
        PeerConfig {
            address: address.into(),
            label: String::new(),
            known_id: id,
            control_port: 41650,
            public_key: sleutel.map(|k| bytes_naar_hex(&k)),
            addresses: Vec::new(),
            verwijderd: false,
        }
    }

    fn code(id: PeerId, sleutel: [u8; 32], adres: &str) -> Uitnodiging {
        Uitnodiging {
            peer_id: id,
            public_key: sleutel,
            naam: "Sam".into(),
            adressen: vec![adres.parse().unwrap()],
        }
    }

    #[test]
    fn een_code_hoort_bij_zijn_peer_op_id_sleutel_of_tailnetadres() {
        let (ik, sam, kim) = (
            PeerId::new_random(),
            PeerId::new_random(),
            PeerId::new_random(),
        );
        let peers = vec![
            peer("100.64.0.2", Some(sam), Some([1; 32])),
            peer("100.64.0.3", None, None),
        ];
        assert_eq!(
            koppel(&peers, ik, &code(sam, [1; 32], "84.1.2.3:41650")),
            Ok(Koppeling::Bestaand(0))
        );
        // Een tailnet-peer die nog nooit verbonden was: zijn code noemt zijn tailnet-adres.
        assert_eq!(
            koppel(&peers, ik, &code(kim, [2; 32], "100.64.0.3:41650")),
            Ok(Koppeling::Bestaand(1))
        );
        assert_eq!(
            koppel(&peers, ik, &code(kim, [2; 32], "84.1.2.4:41650")),
            Ok(Koppeling::Nieuw)
        );
    }

    #[test]
    fn na_een_herinstallatie_wordt_hij_dezelfde_peer_en_geen_tweede() {
        // De gebruiker deed wat de melding zei: `known_id` en `public_key` weg. De peer kwam
        // uit een code, dus hij heeft geen configadres, wel zijn adressen van toen.
        let ik = PeerId::new_random();
        let mut oud = peer("", None, None);
        oud.addresses = vec!["84.1.2.3:41650".parse().unwrap()];
        assert_eq!(
            koppel(
                &[oud],
                ik,
                &code(PeerId::new_random(), [5; 32], "84.1.2.3:41650")
            ),
            Ok(Koppeling::Bestaand(0))
        );
    }

    /// Na verwijderen is zijn code, ook met een nieuwe sleutel, een nieuwe peer: de
    /// grafsteen ervoor wordt nooit gekozen. Revert om te zien falen: laat `PeerConfig::vergeet`
    /// de pin staan (dan `SleutelWijkt`, of `Bestaand(0)` op de grafsteen).
    #[test]
    fn een_verwijderde_peer_komt_terug_als_nieuwe() {
        let (ik, sam) = (PeerId::new_random(), PeerId::new_random());
        let mut weg = peer("100.64.0.2", Some(sam), Some([1; 32]));
        weg.addresses = vec!["84.1.2.3:41650".parse().unwrap()];
        weg.vergeet();
        let peers = [weg];
        for sleutel in [[1; 32], [9; 32]] {
            assert_eq!(
                koppel(&peers, ik, &code(sam, sleutel, "100.64.0.2:41650")),
                Ok(Koppeling::Nieuw)
            );
        }
    }

    #[test]
    fn een_andere_sleutel_of_de_eigen_code_wordt_geweigerd() {
        let (ik, sam) = (PeerId::new_random(), PeerId::new_random());
        let peers = vec![peer("", Some(sam), Some([1; 32]))];
        assert_eq!(
            koppel(&peers, ik, &code(sam, [9; 32], "84.1.2.3:41650")),
            Err(CodeWeigering::SleutelWijkt),
            "zelfde peer, nieuwe sleutel: niet stil overschrijven"
        );
        assert_eq!(
            koppel(
                &peers,
                ik,
                &code(PeerId::new_random(), [1; 32], "84.1.2.3:41650")
            ),
            Err(CodeWeigering::SleutelWijkt),
            "zijn sleutel onder een andere PeerId"
        );
        assert_eq!(
            koppel(&peers, ik, &code(ik, [3; 32], "84.1.2.3:41650")),
            Err(CodeWeigering::EigenCode)
        );
    }
}
