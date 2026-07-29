//! Van oplog naar wat je op het scherm ziet.
//!
//! Volledig puur: een `&[Op]` erin, een `Timeline` eruit. Geen database, geen tijd,
//! geen willekeur. Daardoor is het gedrag bij door elkaar aankomende ops en gelijktijdige
//! bewerkingen exact te testen.

use fitcom_proto::{Op, OpId, OpKind, PeerId};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub id: OpId,
    pub author: PeerId,
    pub body: String,
    /// Millis sinds epoch, van de klok van de auteur. Alleen voor weergave.
    pub created_at: i64,
    pub edited: bool,
}

/// Een aangeboden bestand. `id` is de `OpId` van de `FileMeta`-op zelf — die is al
/// globaal uniek en dient meteen als overdracht-identificatie in `FileRequest`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub id: OpId,
    /// Wie aanbiedt. Gelijk aan `id.author` — geen apart veld nodig, precies zoals
    /// `Edit`/`Delete` hun eigenaarschap ook via `op.author` regelen.
    pub author: PeerId,
    pub name: String,
    pub size: u64,
    pub hash: [u8; 32],
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Timeline {
    /// Op weergavevolgorde: `(lamport, author)`.
    pub messages: Vec<Message>,
    pub nicknames: HashMap<PeerId, String>,
    /// Alle bekende aanbiedingen, op weergavevolgorde. Geen bewerken of intrekken in v1
    /// — zie `TODO.md`.
    pub files: Vec<FileEntry>,
}

/// Sorteersleutel die op alle peers dezelfde uitkomst geeft. `wall_clock` mag hier
/// nooit in meedoen — de klokken van de drie PC's lopen uiteen en dan zou de volgorde
/// per peer verschillen.
fn key(op: &Op) -> (u64, PeerId) {
    (op.lamport, op.author)
}

pub fn build(ops: &[Op]) -> Timeline {
    let mut sorted: Vec<&Op> = ops.iter().collect();
    sorted.sort_by_key(|o| key(o));

    let mut posts: Vec<(OpId, Message)> = Vec::new();
    let mut files: Vec<FileEntry> = Vec::new();

    // Laatste bewerking of verwijdering per bericht, met de sleutel die won.
    enum Change {
        Edit(String),
        Delete,
    }
    let mut changes: HashMap<OpId, ((u64, PeerId), Change)> = HashMap::new();
    let mut nicknames: HashMap<PeerId, ((u64, PeerId), String)> = HashMap::new();

    for op in sorted {
        // Onbekende soort: van een nieuwere peer. We bewaren en verspreiden hem wel,
        // maar tonen kunnen we hem niet.
        let Ok(Some(kind)) = op.kind() else { continue };

        match kind {
            OpKind::Post { body } => {
                posts.push((
                    op.id(),
                    Message {
                        id: op.id(),
                        author: op.author,
                        body,
                        created_at: op.wall_clock,
                        edited: false,
                    },
                ));
            }
            OpKind::Edit { target, body } => {
                // Alleen je eigen berichten. Zonder deze regel kan iedereen andermans
                // tekst herschrijven, en dat is niet te herstellen in een append-only log.
                if target.author != op.author {
                    continue;
                }
                record(&mut changes, target, key(op), Change::Edit(body));
            }
            OpKind::Delete { target } => {
                if target.author != op.author {
                    continue;
                }
                record(&mut changes, target, key(op), Change::Delete);
            }
            OpKind::SetNick { name } => {
                let k = key(op);
                match nicknames.get(&op.author) {
                    Some((prev, _)) if *prev >= k => {}
                    _ => {
                        nicknames.insert(op.author, (k, name));
                    }
                }
            }
            OpKind::FileMeta { name, size, hash } => {
                files.push(FileEntry {
                    id: op.id(),
                    author: op.author,
                    name,
                    size,
                    hash,
                });
            }
        }
    }

    fn record(
        changes: &mut HashMap<OpId, ((u64, PeerId), Change)>,
        target: OpId,
        k: (u64, PeerId),
        change: Change,
    ) {
        // Laatste schrijver wint. Twee peers kunnen niet tegelijk hetzelfde bericht
        // bewerken — alleen de auteur mag dat — maar dezelfde auteur kan het vanaf
        // twee installaties doen, en dan moet de uitkomst overal gelijk zijn.
        match changes.get(&target) {
            Some((prev, _)) if *prev >= k => {}
            _ => {
                changes.insert(target, (k, change));
            }
        }
    }

    let mut messages: Vec<Message> = Vec::with_capacity(posts.len());
    for (id, mut msg) in posts {
        match changes.get(&id) {
            Some((_, Change::Delete)) => continue,
            Some((_, Change::Edit(body))) => {
                msg.body = body.clone();
                msg.edited = true;
            }
            None => {}
        }
        messages.push(msg);
    }

    Timeline {
        messages,
        nicknames: nicknames.into_iter().map(|(p, (_, n))| (p, n)).collect(),
        files,
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

    fn op(author: PeerId, seq: u64, lamport: u64, kind: OpKind) -> Op {
        Op::new(author, seq, lamport, 0, &kind).unwrap()
    }

    #[test]
    fn volgorde_komt_van_lamport_niet_van_de_klok() {
        // B's PC loopt een uur achter. Dat mag de volgorde niet bepalen.
        let mut eerst = op(peer(1), 1, 1, OpKind::Post { body: "een".into() });
        eerst.wall_clock = 9_999_999;
        let mut daarna = op(
            peer(2),
            1,
            2,
            OpKind::Post {
                body: "twee".into(),
            },
        );
        daarna.wall_clock = 0;

        let t = build(&[daarna, eerst]);
        let bodies: Vec<&str> = t.messages.iter().map(|m| m.body.as_str()).collect();
        assert_eq!(bodies, ["een", "twee"]);
    }

    #[test]
    fn gelijke_lamport_wordt_op_auteur_beslist() {
        let a = op(peer(1), 1, 5, OpKind::Post { body: "a".into() });
        let b = op(peer(2), 1, 5, OpKind::Post { body: "b".into() });
        // Beide aankomstvolgordes moeten hetzelfde resultaat geven.
        let t1 = build(&[a.clone(), b.clone()]);
        let t2 = build(&[b, a]);
        assert_eq!(t1, t2);
    }

    #[test]
    fn bewerking_vervangt_de_tekst() {
        let post = op(
            peer(1),
            1,
            1,
            OpKind::Post {
                body: "typfuot".into(),
            },
        );
        let edit = op(
            peer(1),
            2,
            2,
            OpKind::Edit {
                target: post.id(),
                body: "typfout".into(),
            },
        );
        let t = build(&[post, edit]);
        assert_eq!(t.messages.len(), 1);
        assert_eq!(t.messages[0].body, "typfout");
        assert!(t.messages[0].edited);
    }

    #[test]
    fn verwijderd_bericht_verdwijnt() {
        let post = op(
            peer(1),
            1,
            1,
            OpKind::Post {
                body: "oeps".into(),
            },
        );
        let del = op(peer(1), 2, 2, OpKind::Delete { target: post.id() });
        assert!(build(&[post, del]).messages.is_empty());
    }

    #[test]
    fn laatste_wijziging_wint_ongeacht_aankomstvolgorde() {
        let post = op(peer(1), 1, 1, OpKind::Post { body: "v1".into() });
        let e1 = op(
            peer(1),
            2,
            2,
            OpKind::Edit {
                target: post.id(),
                body: "v2".into(),
            },
        );
        let e2 = op(
            peer(1),
            3,
            7,
            OpKind::Edit {
                target: post.id(),
                body: "v3".into(),
            },
        );
        for volgorde in [
            vec![post.clone(), e1.clone(), e2.clone()],
            vec![e2.clone(), e1.clone(), post.clone()],
            vec![e1.clone(), post.clone(), e2.clone()],
        ] {
            let t = build(&volgorde);
            assert_eq!(t.messages[0].body, "v3", "hoogste lamport hoort te winnen");
        }
    }

    #[test]
    fn verwijderen_wint_van_een_eerdere_bewerking() {
        let post = op(peer(1), 1, 1, OpKind::Post { body: "v1".into() });
        let edit = op(
            peer(1),
            2,
            2,
            OpKind::Edit {
                target: post.id(),
                body: "v2".into(),
            },
        );
        let del = op(peer(1), 3, 3, OpKind::Delete { target: post.id() });
        assert!(build(&[post, edit, del]).messages.is_empty());
    }

    #[test]
    fn je_mag_andermans_bericht_niet_herschrijven() {
        let post = op(
            peer(1),
            1,
            1,
            OpKind::Post {
                body: "van peer 1".into(),
            },
        );
        let kaping = op(
            peer(2),
            1,
            5,
            OpKind::Edit {
                target: post.id(),
                body: "gekaapt".into(),
            },
        );
        let sloop = op(peer(2), 2, 6, OpKind::Delete { target: post.id() });

        let t = build(&[post, kaping, sloop]);
        assert_eq!(t.messages.len(), 1, "bericht mag niet verwijderd zijn");
        assert_eq!(t.messages[0].body, "van peer 1");
        assert!(!t.messages[0].edited);
    }

    #[test]
    fn bewerking_zonder_bijbehorend_bericht_doet_niets() {
        // Kan gebeuren als de bewerking eerder aankomt dan het bericht zelf.
        let edit = op(
            peer(1),
            2,
            2,
            OpKind::Edit {
                target: OpId::new(peer(1), 1),
                body: "zweeft".into(),
            },
        );
        assert!(build(&[edit]).messages.is_empty());
    }

    #[test]
    fn onbekende_opsoort_wordt_overgeslagen_bij_renderen() {
        let post = op(
            peer(1),
            1,
            1,
            OpKind::Post {
                body: "gewoon".into(),
            },
        );
        let toekomstig = Op {
            author: peer(2),
            seq: 1,
            lamport: 2,
            wall_clock: 0,
            kind_tag: 4242,
            payload: vec![0x80],
        };
        let t = build(&[post, toekomstig]);
        assert_eq!(t.messages.len(), 1);
    }

    #[test]
    fn aangeboden_bestand_komt_in_de_timeline_met_zijn_eigen_op_id_als_identificatie() {
        let aanbod = op(
            peer(1),
            1,
            1,
            OpKind::FileMeta {
                name: "vakantiefotos.zip".into(),
                size: 42,
                hash: [0x11; 32],
            },
        );
        let t = build(std::slice::from_ref(&aanbod));
        assert_eq!(t.files.len(), 1);
        assert_eq!(t.files[0].id, aanbod.id());
        assert_eq!(t.files[0].author, peer(1));
        assert_eq!(t.files[0].name, "vakantiefotos.zip");
        assert_eq!(t.files[0].size, 42);
        assert_eq!(t.files[0].hash, [0x11; 32]);
    }

    #[test]
    fn laatste_bijnaam_wint() {
        let a = op(
            peer(1),
            1,
            1,
            OpKind::SetNick {
                name: "Rick".into(),
            },
        );
        let b = op(
            peer(1),
            2,
            9,
            OpKind::SetNick {
                name: "Rick2".into(),
            },
        );
        let t = build(&[b, a]);
        assert_eq!(t.nicknames[&peer(1)], "Rick2");
    }
}
