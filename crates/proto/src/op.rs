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

use crate::{OpId, PeerId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Eén operatie in de log. Nooit muteren na aanmaak.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Op {
    pub author: PeerId,
    /// Per auteur monotoon en **dicht**: 1, 2, 3, ... zonder gaten.
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
        OpId::new(self.author, self.seq)
    }

    /// Weergavevolgorde. `author` breekt de gelijkstand zodat alle peers dezelfde
    /// volgorde tonen bij gelijke lamport-waarde.
    pub fn order_key(&self) -> (u64, PeerId) {
        (self.lamport, self.author)
    }

    pub fn new(
        author: PeerId,
        seq: u64,
        lamport: u64,
        wall_clock: i64,
        kind: &OpKind,
    ) -> crate::Result<Self> {
        Ok(Self {
            author,
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
    // 10+ GERESERVEERD: React, Reply, FileMeta. Zie TODO.md.
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

/// `{auteur -> hoogste seq die ik heb}`.
///
/// Dit werkt alléén omdat `seq` per auteur dicht is. Zodra er gaten in kunnen zitten
/// is één getal per auteur niet meer genoeg en is dit hele mechanisme stuk.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "Vec<(PeerId, u64)>", into = "Vec<(PeerId, u64)>")]
pub struct VersionVector(BTreeMap<PeerId, u64>);

impl From<Vec<(PeerId, u64)>> for VersionVector {
    fn from(v: Vec<(PeerId, u64)>) -> Self {
        Self(v.into_iter().collect())
    }
}

impl From<VersionVector> for Vec<(PeerId, u64)> {
    fn from(v: VersionVector) -> Self {
        v.0.into_iter().collect()
    }
}

impl VersionVector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Hoogste seq die we van deze auteur hebben. 0 = niets.
    pub fn get(&self, author: PeerId) -> u64 {
        self.0.get(&author).copied().unwrap_or(0)
    }

    /// Alleen ophogen. Een op die we al hadden verandert hier niets — dat is de
    /// idempotentie waar de hele sync op leunt.
    pub fn observe(&mut self, author: PeerId, seq: u64) {
        let e = self.0.entry(author).or_insert(0);
        *e = (*e).max(seq);
    }

    pub fn iter(&self) -> impl Iterator<Item = (PeerId, u64)> + '_ {
        self.0.iter().map(|(&p, &s)| (p, s))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Welke seq-bereiken heeft `other` nog niet, die ik wel heb?
    /// Levert `(auteur, van_seq, tot_seq)` inclusief aan beide kanten.
    ///
    /// Dit is de kern van de inhaalslag na offline zijn: één ronde, geen onderhandeling.
    pub fn ranges_missing_in(&self, other: &VersionVector) -> Vec<(PeerId, u64, u64)> {
        self.iter()
            .filter_map(|(author, mine)| {
                let theirs = other.get(author);
                (mine > theirs).then_some((author, theirs + 1, mine))
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
                target: OpId::new(peer(1), 7),
                body: "aangepast".into(),
            },
            OpKind::Delete {
                target: OpId::new(peer(2), 3),
            },
            OpKind::SetNick {
                name: "Rick".into(),
            },
        ];
        for kind in kinds {
            let op = Op::new(peer(1), 1, 1, 0, &kind).unwrap();
            assert_eq!(op.kind().unwrap(), Some(kind));
        }
    }

    #[test]
    fn onbekende_opsoort_blijft_doorstuurbaar() {
        // Kern van de gossip: een op van een nieuwere peer moet je kunnen bewaren en
        // doorgeven aan de derde peer, ook zonder hem te begrijpen.
        let op = Op {
            author: peer(1),
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
        vv.observe(peer(1), 5);
        vv.observe(peer(1), 3); // oude op nogmaals ontvangen
        assert_eq!(vv.get(peer(1)), 5);
        assert_eq!(vv.get(peer(2)), 0);
    }

    #[test]
    fn ontbrekende_bereiken_na_lang_offline() {
        // A was online en maakte 100 ops; B stond uit na 10. C heeft niets van A.
        let mut a = VersionVector::new();
        a.observe(peer(1), 100);
        a.observe(peer(3), 4);

        let mut b = VersionVector::new();
        b.observe(peer(1), 10);
        b.observe(peer(3), 4);

        assert_eq!(a.ranges_missing_in(&b), vec![(peer(1), 11, 100)]);

        let leeg = VersionVector::new();
        let mut r = a.ranges_missing_in(&leeg);
        r.sort();
        assert_eq!(r, vec![(peer(1), 1, 100), (peer(3), 1, 4)]);
    }

    #[test]
    fn gelijke_vectoren_vragen_niets_op() {
        let mut a = VersionVector::new();
        a.observe(peer(1), 7);
        assert!(a.ranges_missing_in(&a.clone()).is_empty());
    }

    #[test]
    fn version_vector_overleeft_de_draad() {
        let mut vv = VersionVector::new();
        vv.observe(peer(1), 9);
        vv.observe(peer(2), 4);
        let msg = crate::ControlMsg::SyncRequest(crate::control::SyncRequest { have: vv.clone() });
        let bytes = msg.encode().unwrap();
        match crate::ControlMsg::decode(&bytes).unwrap().unwrap() {
            crate::ControlMsg::SyncRequest(r) => assert_eq!(r.have, vv),
            other => panic!("verkeerde variant: {other:?}"),
        }
    }
}
