//! De oplog: append-only, onveranderlijk, idempotent.
//!
//! Bewust niet chat-specifiek. Chat is nu de enige gebruiker, maar bijnamen,
//! instellingen en later filemetadata lopen over hetzelfde mechanisme mee.
//!
//! # Waarom de inhoud opaak is
//!
//! `Op` bewaart zijn soort als `kind_tag` + ongeïnterpreteerde `payload`, niet als
//! getypeerde enum. Dat is essentieel voor de gossip: een peer op een oudere versie
//! moet een op die hij níét begrijpt tóch kunnen opslaan en doorsturen naar de derde
//! peer. Zou hij hem laten vallen, dan convergeert de mesh niet meer zodra er
//! versieverschil is. Decoderen naar `OpKind` gebeurt pas bij het renderen.

use crate::{Channel, OpId, PeerId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Eén operatie in de log. Nooit muteren na aanmaak.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Op {
    pub author: PeerId,
    /// Bepaalt wie deze op ooit mag zien. Staat, net als `author` en `seq`, altijd open
    /// leesbaar naast de opake `payload` — de gossip moet immers kunnen beslissen wie hem
    /// doorkrijgt zonder de soort te hoeven begrijpen.
    #[serde(default)]
    pub channel: Channel,
    /// Per (auteur, kanaal) monotoon en **dicht**: 1, 2, 3, ... zonder gaten.
    pub seq: u64,
    /// Voor totale ordening tussen auteurs.
    pub lamport: u64,
    /// Millis sinds epoch. Alleen voor weergave. Nooit voor correctheid gebruiken —
    /// de klokken van de drie PC's lopen uiteen.
    pub wall_clock: i64,
    pub kind_tag: u16,
    /// msgpack van de soort-specifieke velden. Opaak voor peers die de soort niet kennen.
    #[serde(with = "serde_bytes")]
    pub payload: Vec<u8>,
}

impl Op {
    pub fn id(&self) -> OpId {
        OpId::new(self.author, self.channel, self.seq)
    }

    /// Weergavevolgorde. `author` breekt de gelijkstand zodat alle peers dezelfde
    /// volgorde tonen bij gelijke lamport-waarde.
    pub fn order_key(&self) -> (u64, PeerId) {
        (self.lamport, self.author)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        author: PeerId,
        channel: Channel,
        seq: u64,
        lamport: u64,
        wall_clock: i64,
        kind: &OpKind,
    ) -> crate::Result<Self> {
        Ok(Self {
            author,
            channel,
            seq,
            lamport,
            wall_clock,
            kind_tag: kind.tag(),
            payload: kind.encode_payload()?,
        })
    }

    /// `Ok(None)` betekent: soort van een nieuwere peer. Opslaan en doorsturen,
    /// maar niet renderen.
    pub fn kind(&self) -> crate::Result<Option<OpKind>> {
        OpKind::decode(self.kind_tag, &self.payload)
    }
}

macro_rules! op_kinds {
    ($( $tag:literal => $variant:ident { $( $field:ident : $ty:ty ),* $(,)? } ),* $(,)?) => {
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub enum OpKind {
            $( $variant { $( $field: $ty, )* }, )*
        }

        /// Per soort een eigen struct, zodat velden per soort onafhankelijk mogen
        /// evolueren met `#[serde(default)]`.
        mod wire {
            use super::*;
            $(
                #[derive(Serialize, Deserialize)]
                pub struct $variant { $( pub $field: $ty, )* }
            )*
        }

        impl OpKind {
            pub fn tag(&self) -> u16 {
                match self { $( Self::$variant { .. } => $tag, )* }
            }

            pub fn encode_payload(&self) -> crate::Result<Vec<u8>> {
                let mut out = Vec::new();
                let mut ser = rmp_serde::Serializer::new(&mut out).with_struct_map();
                match self {
                    $( Self::$variant { $( $field, )* } => {
                        let w = wire::$variant { $( $field: $field.clone(), )* };
                        serde::Serialize::serialize(&w, &mut ser)?;
                    } )*
                }
                Ok(out)
            }

            pub fn decode(tag: u16, payload: &[u8]) -> crate::Result<Option<Self>> {
                Ok(match tag {
                    $( $tag => {
                        let w: wire::$variant = rmp_serde::from_slice(payload)?;
                        Some(Self::$variant { $( $field: w.$field, )* })
                    } )*
                    _ => None,
                })
            }
        }
    };
}

op_kinds! {
    1 => Post   { body: String },
    2 => Edit   { target: OpId, body: String },
    3 => Delete { target: OpId },
    4 => SetNick { name: String },
    // Wie aanbiedt staat niet als apart veld: dat is `op.author`, precies zoals Edit/Delete
    // hun eigenaarschap ook al via `op.author` regelen in plaats van een los veld dat uit de
    // pas zou kunnen lopen. Zie docs/ARCHITECTURE.md voor de rest van het ontwerp.
    10 => FileMeta { name: String, size: u64, hash: [u8; 32] },
    // 11+ GERESERVEERD: React, Reply. Zie TODO.md.
    // Nieuwe soorten toevoegen kost geen migratie — dat is het hele punt van deze opzet.
}

/// Lamport-klok. Ophogen bij elke eigen op, en bijwerken bij elke ontvangen op.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LamportClock(u64);

impl LamportClock {
    pub fn from_raw(v: u64) -> Self {
        Self(v)
    }

    pub fn raw(self) -> u64 {
        self.0
    }

    /// Voor een nieuwe eigen op.
    pub fn tick(&mut self) -> u64 {
        self.0 += 1;
        self.0
    }

    /// Voor elke ontvangen op, vóór opslag.
    pub fn observe(&mut self, remote: u64) {
        self.0 = self.0.max(remote);
    }
}

/// Eén regel van een `VersionVector` op de draad. Een benoemde struct in plaats van een
/// rauwe tuple: msgpack codeert een Rust-tuple altijd als vaste-lengte array, dus een
/// toekomstige extra veld zou — anders dan bij elke andere struct in dit protocol — geen
/// `#[serde(default)]` kunnen krijgen en een oudere peer zou de hele vector niet meer
/// kunnen decoderen. Met benoemde velden (als map, net als `Op`/`OpId`/`Hello`) kan dat
/// straks wél.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct VvEntry {
    author: PeerId,
    channel: Channel,
    seq: u64,
}

/// `{(auteur, kanaal) -> hoogste seq die ik heb}`.
///
/// Dit werkt alléén omdat `seq` per (auteur, kanaal) dicht is. Zodra er gaten in kunnen
/// zitten is één getal per sleutel niet meer genoeg en is dit hele mechanisme stuk. Zie
/// de moduledoc van `fitcom_store` voor waarom kanaal hier expliciet bij hoort en niet
/// per auteur alleen.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "Vec<VvEntry>", into = "Vec<VvEntry>")]
pub struct VersionVector(BTreeMap<(PeerId, Channel), u64>);

impl From<Vec<VvEntry>> for VersionVector {
    fn from(v: Vec<VvEntry>) -> Self {
        Self(
            v.into_iter()
                .map(|e| ((e.author, e.channel), e.seq))
                .collect(),
        )
    }
}

impl From<VersionVector> for Vec<VvEntry> {
    fn from(v: VersionVector) -> Self {
        v.0.into_iter()
            .map(|((author, channel), seq)| VvEntry {
                author,
                channel,
                seq,
            })
            .collect()
    }
}

impl VersionVector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Hoogste seq die we van deze auteur in dit kanaal hebben. 0 = niets.
    pub fn get(&self, author: PeerId, channel: Channel) -> u64 {
        self.0.get(&(author, channel)).copied().unwrap_or(0)
    }

    /// Alleen ophogen. Een op die we al hadden verandert hier niets — dat is de
    /// idempotentie waar de hele sync op leunt.
    pub fn observe(&mut self, author: PeerId, channel: Channel, seq: u64) {
        let e = self.0.entry((author, channel)).or_insert(0);
        *e = (*e).max(seq);
    }

    pub fn iter(&self) -> impl Iterator<Item = (PeerId, Channel, u64)> + '_ {
        self.0.iter().map(|(&(p, c), &s)| (p, c, s))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Alleen de sleutels die `viewer` ooit mag zien: het algemene kanaal, een DM-kanaal
    /// gericht aan `viewer`, en (voor het geval `viewer` eigen data kwijtraakte) alles
    /// waarvan `viewer` zelf de auteur is. Gebruikt vóór we iets van deze vector naar
    /// `viewer` sturen of ermee vergelijken wat hij nog mist — zonder dit zou een DM
    /// tussen twee andere peers gewoon meegossipt worden naar wie het niet aangaat.
    pub fn visible_to(&self, viewer: PeerId) -> VersionVector {
        VersionVector(
            self.0
                .iter()
                .filter(|((author, channel), _)| {
                    channel.is_general() || *author == viewer || channel.dm_peer() == Some(viewer)
                })
                .map(|(&k, &v)| (k, v))
                .collect(),
        )
    }

    /// Welke seq-bereiken heeft `other` nog niet, die ik wel heb?
    /// Levert `(auteur, kanaal, van_seq, tot_seq)` inclusief aan beide kanten.
    ///
    /// Dit is de kern van de inhaalslag na offline zijn: één ronde, geen onderhandeling.
    pub fn ranges_missing_in(&self, other: &VersionVector) -> Vec<(PeerId, Channel, u64, u64)> {
        self.iter()
            .filter_map(|(author, channel, mine)| {
                let theirs = other.get(author, channel);
                (mine > theirs).then_some((author, channel, theirs + 1, mine))
            })
            .collect()
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
    fn op_roundtrip_per_soort() {
        let kinds = [
            OpKind::Post { body: "hoi".into() },
            OpKind::Edit {
                target: OpId::new(peer(1), Channel::GENERAL, 7),
                body: "aangepast".into(),
            },
            OpKind::Delete {
                target: OpId::new(peer(2), Channel::GENERAL, 3),
            },
            OpKind::SetNick {
                name: "Rick".into(),
            },
            OpKind::FileMeta {
                name: "vakantiefotos.zip".into(),
                size: 123_456_789,
                hash: [0x42; 32],
            },
        ];
        for kind in kinds {
            let op = Op::new(peer(1), Channel::GENERAL, 1, 1, 0, &kind).unwrap();
            assert_eq!(op.kind().unwrap(), Some(kind));
        }
    }

    #[test]
    fn onbekende_opsoort_blijft_doorstuurbaar() {
        // Kern van de gossip: een op van een nieuwere peer moet je kunnen bewaren en
        // doorgeven aan de derde peer, ook zonder hem te begrijpen.
        let op = Op {
            author: peer(1),
            channel: Channel::GENERAL,
            seq: 1,
            lamport: 1,
            wall_clock: 0,
            kind_tag: 999,
            payload: vec![0x80],
        };
        assert_eq!(
            op.kind().unwrap(),
            None,
            "onbekende soort mag geen fout geven"
        );

        let msg = crate::ControlMsg::OpBroadcast(crate::control::OpBroadcast { op: op.clone() });
        let bytes = msg.encode().unwrap();
        match crate::ControlMsg::decode(&bytes).unwrap().unwrap() {
            crate::ControlMsg::OpBroadcast(b) => assert_eq!(b.op, op),
            other => panic!("verkeerde variant: {other:?}"),
        }
    }

    #[test]
    fn lamport_loopt_voor_op_wat_hij_zag() {
        let mut c = LamportClock::default();
        c.tick();
        c.observe(41);
        assert_eq!(c.tick(), 42);
    }

    #[test]
    fn version_vector_observeert_alleen_omhoog() {
        let mut vv = VersionVector::new();
        vv.observe(peer(1), Channel::GENERAL, 5);
        vv.observe(peer(1), Channel::GENERAL, 3); // oude op nogmaals ontvangen
        assert_eq!(vv.get(peer(1), Channel::GENERAL), 5);
        assert_eq!(vv.get(peer(2), Channel::GENERAL), 0);
    }

    #[test]
    fn dm_en_algemeen_tellen_apart_ook_voor_dezelfde_auteur() {
        let mut vv = VersionVector::new();
        vv.observe(peer(1), Channel::GENERAL, 5);
        vv.observe(peer(1), Channel::dm(peer(2)), 2);
        assert_eq!(vv.get(peer(1), Channel::GENERAL), 5);
        assert_eq!(vv.get(peer(1), Channel::dm(peer(2))), 2);
    }

    #[test]
    fn ontbrekende_bereiken_na_lang_offline() {
        // A was online en maakte 100 ops; B stond uit na 10. C heeft niets van A.
        let mut a = VersionVector::new();
        a.observe(peer(1), Channel::GENERAL, 100);
        a.observe(peer(3), Channel::GENERAL, 4);

        let mut b = VersionVector::new();
        b.observe(peer(1), Channel::GENERAL, 10);
        b.observe(peer(3), Channel::GENERAL, 4);

        assert_eq!(
            a.ranges_missing_in(&b),
            vec![(peer(1), Channel::GENERAL, 11, 100)]
        );

        let leeg = VersionVector::new();
        let mut r = a.ranges_missing_in(&leeg);
        r.sort();
        assert_eq!(
            r,
            vec![
                (peer(1), Channel::GENERAL, 1, 100),
                (peer(3), Channel::GENERAL, 1, 4),
            ]
        );
    }

    #[test]
    fn gelijke_vectoren_vragen_niets_op() {
        let mut a = VersionVector::new();
        a.observe(peer(1), Channel::GENERAL, 7);
        assert!(a.ranges_missing_in(&a.clone()).is_empty());
    }

    #[test]
    fn version_vector_overleeft_de_draad() {
        let mut vv = VersionVector::new();
        vv.observe(peer(1), Channel::GENERAL, 9);
        vv.observe(peer(2), Channel::dm(peer(1)), 4);
        let msg = crate::ControlMsg::SyncRequest(crate::control::SyncRequest { have: vv.clone() });
        let bytes = msg.encode().unwrap();
        match crate::ControlMsg::decode(&bytes).unwrap().unwrap() {
            crate::ControlMsg::SyncRequest(r) => assert_eq!(r.have, vv),
            other => panic!("verkeerde variant: {other:?}"),
        }
    }

    #[test]
    fn visible_to_laat_algemeen_kanaal_altijd_door() {
        let mut vv = VersionVector::new();
        vv.observe(peer(1), Channel::GENERAL, 5);
        let gefilterd = vv.visible_to(peer(9));
        assert_eq!(gefilterd.get(peer(1), Channel::GENERAL), 5);
    }

    #[test]
    fn visible_to_laat_dm_alleen_aan_de_geadresseerde_zien() {
        let mut vv = VersionVector::new();
        // Peer 1 z'n DM aan peer 2.
        vv.observe(peer(1), Channel::dm(peer(2)), 3);

        assert_eq!(vv.visible_to(peer(2)).get(peer(1), Channel::dm(peer(2))), 3);
        assert!(
            vv.visible_to(peer(3)).is_empty(),
            "peer 3 mag deze DM niet zien"
        );
    }

    #[test]
    fn visible_to_laat_eigen_auteurschap_altijd_terug_naar_de_auteur() {
        // Zodat een peer die eigen data kwijtraakte zijn eigen DM's kan terugkrijgen van
        // de ander in het gesprek.
        let mut vv = VersionVector::new();
        vv.observe(peer(1), Channel::dm(peer(2)), 3);
        assert_eq!(vv.visible_to(peer(1)).get(peer(1), Channel::dm(peer(2))), 3);
    }
}
