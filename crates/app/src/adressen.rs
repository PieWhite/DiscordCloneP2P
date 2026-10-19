//! Onthouden waar peers gezien zijn (fase B4 van `docs/PLAN-ZONDER-TAILSCALE.md`).
//!
//! `<data>/adressen.json`: per peer de laatste paar control-adressen waarop hij gezien is,
//! met het moment. Zo vinden twee apps elkaar na een herstart terug zonder nieuwe code, en
//! zolang er één peer bereikbaar blijft heelt een IP-wissel vanzelf (de mesh leert het nieuwe
//! adres via een derde). Puur lokaal, nooit op de draad; weggooien mag altijd.
//!
//! Adressen zijn hints: dit bestand kan niemand binnenlaten, want binnenkomen gaat alleen met
//! de sleutel in `config.toml`.

use fitcom_proto::PeerId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::Path;

/// Ouder dan dit is geen hint meer maar een gok.
const HOUDBAAR_S: u64 = 60 * 24 * 3600;
/// Per peer; meer onthouden helpt niet, het kost alleen belpogingen.
const PER_PEER: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct Gezien {
    adres: SocketAddr,
    /// Seconden sinds 1970.
    op: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Bestand {
    #[serde(default)]
    peers: HashMap<PeerId, Vec<Gezien>>,
}

fn nu() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn lees_bestand(pad: &Path) -> Bestand {
    std::fs::read_to_string(pad)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// Wat er per peer onthouden is, de nieuwste eerst, zonder wat te oud is. Een kapot of
/// ontbrekend bestand is gewoon leeg (invariant 7).
pub fn lees(pad: &Path) -> HashMap<PeerId, Vec<SocketAddr>> {
    let grens = nu().saturating_sub(HOUDBAAR_S);
    lees_bestand(pad)
        .peers
        .into_iter()
        .map(|(peer, mut lijst)| {
            lijst.sort_by_key(|g| std::cmp::Reverse(g.op));
            let adressen = lijst
                .into_iter()
                .filter(|g| g.op >= grens)
                .take(PER_PEER)
                .map(|g| g.adres)
                .collect();
            (peer, adressen)
        })
        .collect()
}

/// `adressen` (de nieuwste eerst) zijn nu gezien voor `peer`. Wat er al stond schuift op.
pub fn bewaar(pad: &Path, peer: PeerId, adressen: &[SocketAddr]) -> anyhow::Result<()> {
    let mut b = lees_bestand(pad);
    let t = nu();
    let lijst = b.peers.entry(peer).or_default();
    // Niets nieuws: niet schrijven. De momenten verversen bij de volgende echte wijziging, en
    // na een herstart is de eerste melding er altijd een.
    if !adressen.is_empty() && adressen.iter().all(|a| lijst.iter().any(|g| g.adres == *a)) {
        return Ok(());
    }
    // Volgorde vasthouden: de eerste krijgt het nieuwste moment.
    for (i, &adres) in adressen.iter().enumerate().rev() {
        lijst.retain(|g| g.adres != adres);
        lijst.push(Gezien {
            adres,
            op: t + (adressen.len() - i) as u64,
        });
    }
    lijst.sort_by_key(|g| std::cmp::Reverse(g.op));
    lijst.truncate(PER_PEER);
    std::fs::write(pad, serde_json::to_string_pretty(&b)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn onthouden_overleeft_een_herstart_en_blijft_begrensd() {
        let map = std::env::temp_dir().join(format!("fitcom-adressen-{}", PeerId::new_random()));
        std::fs::create_dir_all(&map).unwrap();
        let pad = map.join("adressen.json");
        assert!(lees(&pad).is_empty(), "geen bestand is leeg, geen fout");

        let peer = PeerId::new_random();
        let a: SocketAddr = "84.104.12.9:41650".parse().unwrap();
        let b: SocketAddr = "192.168.2.5:41650".parse().unwrap();
        bewaar(&pad, peer, &[a, b]).unwrap();
        assert_eq!(lees(&pad)[&peer], vec![a, b]);

        // Een nieuw adres komt vooraan; meer dan PER_PEER valt van achteren af.
        let nieuw: Vec<SocketAddr> = (1..=10)
            .map(|i| SocketAddr::from(([10, 0, 0, i], 41650)))
            .collect();
        bewaar(&pad, peer, &nieuw).unwrap();
        let terug = lees(&pad)[&peer].clone();
        assert_eq!(terug.len(), PER_PEER);
        assert_eq!(terug[0], nieuw[0]);

        std::fs::write(&pad, "{ kapot").unwrap();
        assert!(lees(&pad).is_empty());
        let _ = std::fs::remove_dir_all(&map);
    }
}
