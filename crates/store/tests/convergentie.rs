//! Convergentie van de oplog tussen meerdere peers.
//!
//! Dit is de belangrijkste test van het project. Chat-synchronisatie faalt zelden
//! zichtbaar: hij levert een timeline op die bij de een net iets anders is dan bij de
//! ander, en dat merk je pas weken later. Handmatig testen vindt dit niet.
//!
//! Alles draait hier zonder netwerk: de "verbinding" is een functie die ops van de ene
//! store naar de andere verplaatst. Daardoor kunnen we uitval, partities en willekeurige
//! aankomstvolgorde afdwingen in plaats van erop te hopen.

use fitcom_proto::{Channel, Op, OpKind, PeerId, TopicId};
use fitcom_store::{Store, SYNC_BATCH};

fn peer(n: u8) -> PeerId {
    let mut b = [0u8; 16];
    b[0] = n;
    PeerId::from_bytes(b)
}

fn store(n: u8) -> Store {
    Store::open_in_memory(peer(n)).unwrap()
}

fn post(s: &mut Store, tekst: &str) -> Op {
    s.append_local(Channel::GENERAL, &fitcom_store::post(tekst), 0)
        .unwrap()
}

/// Eenrichtingsverkeer: alles wat `from` heeft en `to` mist.
fn push(from: &Store, to: &mut Store) {
    for _ in 0..1000 {
        let batch = from
            .ops_missing_in(&to.version_vector().unwrap(), SYNC_BATCH)
            .unwrap();
        if batch.is_empty() {
            return;
        }
        to.apply_remote_batch(&batch).unwrap();
    }
    panic!("sync komt niet tot rust — batch levert ops die de version vector niet opschuiven");
}

/// Wat er bij (her)verbinding gebeurt: beide kanten vullen elkaar aan.
fn sync(a: &mut Store, b: &mut Store) {
    push(a, b);
    let snapshot = b.all_ops().unwrap();
    a.apply_remote_batch(&snapshot).unwrap();
}

fn bodies(s: &Store) -> Vec<String> {
    s.timeline()
        .unwrap()
        .messages
        .into_iter()
        .map(|m| m.body)
        .collect()
}

#[test]
fn bericht_komt_aan_na_terugkeer_van_offline() {
    let mut a = store(1);
    let mut b = store(2);

    // B staat uit terwijl A doorpraat.
    post(&mut a, "ben je er?");
    post(&mut a, "ok, later dan");
    assert!(bodies(&b).is_empty());

    sync(&mut a, &mut b);
    assert_eq!(bodies(&b), ["ben je er?", "ok, later dan"]);
}

#[test]
fn derde_peer_krijgt_alles_via_de_tussenliggende_peer() {
    // Het geval waar de hele opzet om draait: A is dagen weg, B en C praten door,
    // en als A terugkomt is alleen B online. A moet dan tóch de berichten van C
    // krijgen — B geeft ze door, ook al zijn ze niet van hem.
    let mut a = store(1);
    let mut b = store(2);
    let mut c = store(3);

    sync(&mut b, &mut c);
    post(&mut b, "van B");
    post(&mut c, "van C");
    sync(&mut b, &mut c);
    post(&mut c, "nog een van C");
    sync(&mut b, &mut c);

    // A verbindt uitsluitend met B.
    sync(&mut a, &mut b);

    let van_a = bodies(&a);
    assert_eq!(van_a.len(), 3, "A moet ook C's berichten hebben: {van_a:?}");
    assert_eq!(van_a, bodies(&b));
    assert_eq!(van_a, bodies(&c));
}

#[test]
fn drie_peers_convergeren_bij_willekeurige_volgorde_en_partities() {
    // Deterministische pseudo-willekeur: bij een fout is de run exact te herhalen.
    let mut rng: u64 = 0x5eed_1234_9abc_def0;
    let mut volgende = || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    };

    let mut stores = [store(1), store(2), store(3)];

    for ronde in 0..200 {
        let wie = (volgende() % 3) as usize;
        match volgende() % 4 {
            // Iemand plaatst een bericht.
            0 | 1 => {
                post(&mut stores[wie], &format!("bericht {ronde} van {wie}"));
            }
            // Iemand bewerkt zijn eigen laatste bericht.
            2 => {
                let eigen: Vec<_> = stores[wie]
                    .all_ops()
                    .unwrap()
                    .into_iter()
                    .filter(|o| o.author == peer(wie as u8 + 1))
                    .filter(|o| matches!(o.kind(), Ok(Some(OpKind::Post { .. }))))
                    .collect();
                if let Some(doel) = eigen.last() {
                    stores[wie]
                        .append_local(
                            Channel::GENERAL,
                            &OpKind::Edit {
                                target: doel.id(),
                                body: format!("bewerkt in ronde {ronde}"),
                            },
                            0,
                        )
                        .unwrap();
                }
            }
            // Twee willekeurige peers krijgen even verbinding; de derde blijft weg.
            _ => {
                let ander = (wie + 1 + (volgende() % 2) as usize) % 3;
                let (lo, hi) = (wie.min(ander), wie.max(ander));
                if lo != hi {
                    let (links, rechts) = stores.split_at_mut(hi);
                    sync(&mut links[lo], &mut rechts[0]);
                }
            }
        }
    }

    // Uiteindelijk spreekt iedereen iedereen weer.
    for _ in 0..3 {
        for i in 0..3 {
            for j in (i + 1)..3 {
                let (links, rechts) = stores.split_at_mut(j);
                sync(&mut links[i], &mut rechts[0]);
            }
        }
    }

    let referentie = bodies(&stores[0]);
    assert!(!referentie.is_empty(), "test moet echt iets gedaan hebben");
    for (i, s) in stores.iter().enumerate().skip(1) {
        assert_eq!(bodies(s), referentie, "peer {i} wijkt af van peer 0");
    }
}

#[test]
fn tweemaal_toepassen_verandert_niets() {
    let mut a = store(1);
    let mut b = store(2);
    post(&mut a, "een");
    post(&mut a, "twee");

    let ops = a.all_ops().unwrap();
    assert_eq!(b.apply_remote_batch(&ops).unwrap(), 2);
    assert_eq!(
        b.apply_remote_batch(&ops).unwrap(),
        0,
        "moet een no-op zijn"
    );
    assert_eq!(b.apply_remote_batch(&ops).unwrap(), 0);
    assert_eq!(b.op_count().unwrap(), 2);
    assert_eq!(bodies(&b), ["een", "twee"]);
}

#[test]
fn version_vector_liegt_niet_bij_een_gat() {
    // Precies het geval uit de moduledocumentatie van de store: op 3 komt binnen
    // terwijl 1 en 2 nog onderweg zijn. Zouden we dan "ik heb t/m 3" melden, dan
    // krijgen we 1 en 2 nooit meer.
    let mut a = store(1);
    let mut b = store(2);
    post(&mut a, "een");
    post(&mut a, "twee");
    post(&mut a, "drie");
    let ops = a.all_ops().unwrap();

    b.apply_remote(&ops[2]).unwrap();
    assert_eq!(
        b.version_vector().unwrap().get(peer(1), Channel::GENERAL),
        0,
        "met een gat mag de version vector niets claimen"
    );

    b.apply_remote(&ops[0]).unwrap();
    assert_eq!(
        b.version_vector().unwrap().get(peer(1), Channel::GENERAL),
        1
    );

    // Het gat wordt gedicht: de reeks moet in één keer doorschuiven naar 3.
    b.apply_remote(&ops[1]).unwrap();
    assert_eq!(
        b.version_vector().unwrap().get(peer(1), Channel::GENERAL),
        3
    );
    assert_eq!(bodies(&b), ["een", "twee", "drie"]);
}

#[test]
fn een_gat_blokkeert_niet_wat_erna_komt() {
    // Als wij een gat hebben, mogen we die latere ops niet doorsturen — anders
    // erft de ontvanger ons gat zonder het te weten.
    let mut a = store(1);
    let mut b = store(2);
    let mut c = store(3);
    post(&mut a, "een");
    post(&mut a, "twee");
    post(&mut a, "drie");
    let ops = a.all_ops().unwrap();

    b.apply_remote(&ops[2]).unwrap(); // B heeft alleen nummer 3
    push(&b, &mut c);
    assert_eq!(
        c.op_count().unwrap(),
        0,
        "B mag over zijn gat heen niets leveren"
    );

    b.apply_remote_batch(&ops[..2]).unwrap();
    push(&b, &mut c);
    assert_eq!(bodies(&c), ["een", "twee", "drie"]);
}

#[test]
fn grote_inhaalslag_wordt_in_stukken_geleverd() {
    let mut a = store(1);
    let mut b = store(2);
    for i in 0..(SYNC_BATCH * 2 + 7) {
        post(&mut a, &format!("bericht {i}"));
    }

    let eerste = a
        .ops_missing_in(&b.version_vector().unwrap(), SYNC_BATCH)
        .unwrap();
    assert_eq!(eerste.len(), SYNC_BATCH, "batch moet begrensd zijn");
    assert!(a.has_more_for(&b.version_vector().unwrap()).unwrap());

    push(&a, &mut b);
    assert_eq!(b.op_count().unwrap(), (SYNC_BATCH * 2 + 7) as u64);
    assert!(!a.has_more_for(&b.version_vector().unwrap()).unwrap());
}

#[test]
fn geschiedenis_overleeft_een_herstart() {
    let dir = std::env::temp_dir().join(format!("fitcom-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let pad = dir.join("chat.sqlite");

    {
        let mut s = Store::open(&pad, peer(1)).unwrap();
        post(&mut s, "blijft staan");
        post(&mut s, "dit ook");
    }
    {
        let s = Store::open(&pad, peer(1)).unwrap();
        assert_eq!(bodies(&s), ["blijft staan", "dit ook"]);
        assert_eq!(
            s.version_vector().unwrap().get(peer(1), Channel::GENERAL),
            2
        );
    }
    // En na herstart loopt de nummering gewoon door in plaats van opnieuw te beginnen.
    {
        let mut s = Store::open(&pad, peer(1)).unwrap();
        let op = post(&mut s, "na herstart");
        assert_eq!(
            op.seq, 3,
            "seq mag na herstart geen bestaand nummer hergebruiken"
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn database_van_voor_de_kanalen_uitbreiding_wordt_gemigreerd_niet_geweigerd() {
    // Dit is geen hypothetisch geval: een echte, met de hand opgebouwde database liep
    // hier precies op vast toen schema 2 voor het eerst uitkwam. Bestaande chatgeschiedenis
    // mag niet stuklopen op een interne schema-wijziging.
    use fitcom_proto::OpKind;

    let dir = std::env::temp_dir().join(format!("fitcom-migratietest-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let pad = dir.join("chat.sqlite");

    // Precies de v1-tabelvorm van vóór de kanalen-uitbreiding: geen `channel`-kolom,
    // primary key alleen op (author, seq) resp. author.
    {
        let conn = rusqlite::Connection::open(&pad).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE ops (
                author     BLOB    NOT NULL,
                seq        INTEGER NOT NULL,
                lamport    INTEGER NOT NULL,
                wall_clock INTEGER NOT NULL,
                kind       INTEGER NOT NULL,
                payload    BLOB    NOT NULL,
                PRIMARY KEY (author, seq)
            ) WITHOUT ROWID;
            CREATE TABLE authors (
                author     BLOB    PRIMARY KEY,
                contiguous INTEGER NOT NULL
            ) WITHOUT ROWID;
            CREATE TABLE meta (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            INSERT INTO meta (key, value) VALUES ('schema_version', '1');
            "#,
        )
        .unwrap();

        let auteur = peer(1);
        let kind = OpKind::Post {
            body: "van voor de migratie".into(),
        };
        let payload = kind.encode_payload().unwrap();
        conn.execute(
            "INSERT INTO ops (author, seq, lamport, wall_clock, kind, payload)
             VALUES (?1, 1, 1, 0, ?2, ?3)",
            rusqlite::params![auteur.as_bytes().to_vec(), kind.tag() as i64, payload],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO authors (author, contiguous) VALUES (?1, 1)",
            rusqlite::params![auteur.as_bytes().to_vec()],
        )
        .unwrap();
    }

    // Openen via de echte `Store` moet dit migreren, niet weigeren.
    let mut s = Store::open(&pad, peer(1)).unwrap();
    assert_eq!(bodies(&s), ["van voor de migratie"]);
    assert_eq!(
        s.version_vector().unwrap().get(peer(1), Channel::GENERAL),
        1,
        "gemigreerde ops moeten op het algemene kanaal terechtkomen"
    );

    // En de nummering loopt door in plaats van opnieuw te beginnen.
    let nieuw = post(&mut s, "na de migratie");
    assert_eq!(nieuw.seq, 2, "seq moet doorlopen, niet opnieuw beginnen");
    assert_eq!(bodies(&s), ["van voor de migratie", "na de migratie"]);

    // En heropenen na de migratie werkt gewoon door (schema_version staat nu op 2).
    drop(s);
    let s2 = Store::open(&pad, peer(1)).unwrap();
    assert_eq!(bodies(&s2), ["van voor de migratie", "na de migratie"]);

    let _ = std::fs::remove_dir_all(&dir);
}

/// Eenrichtingsverkeer zoals `push`, maar kanaal-bewust: alleen wat `to` ooit mag zien.
/// Dit is wat de mesh-laag straks gebruikt in plaats van `push`/`ops_missing_in`.
fn push_for(from: &Store, to_id: PeerId, to: &mut Store) {
    for _ in 0..1000 {
        let batch = from
            .ops_missing_in_for(to_id, &to.version_vector().unwrap(), SYNC_BATCH)
            .unwrap();
        if batch.is_empty() {
            return;
        }
        to.apply_remote_batch(&batch).unwrap();
    }
    panic!("sync komt niet tot rust");
}

fn dm(s: &mut Store, aan: PeerId, tekst: &str) -> Op {
    s.append_local(Channel::dm(aan), &fitcom_store::post(tekst), 0)
        .unwrap()
}

#[test]
fn dm_bereikt_de_geadresseerde_maar_niet_een_derde_peer() {
    let mut a = store(1);
    let mut b = store(2);
    let mut c = store(3);

    dm(&mut a, peer(2), "dit is alleen voor jou");
    post(&mut a, "dit is voor iedereen");

    push_for(&a, peer(2), &mut b);
    push_for(&a, peer(3), &mut c);

    assert_eq!(
        bodies(&b),
        ["dit is alleen voor jou", "dit is voor iedereen"]
    );
    assert_eq!(
        bodies(&c),
        ["dit is voor iedereen"],
        "C mag de DM tussen A en B nooit ontvangen"
    );
}

#[test]
fn een_dm_blokkeert_daarna_het_algemene_kanaal_niet_voor_een_buitenstaander() {
    // De kern van de kanaal-scoping: als seq per auteur (in plaats van per
    // auteur+kanaal) zou lopen, zou C hier een permanent gat oplopen op de DM-seq en
    // nooit meer verder komen dan dat gat — ook niet voor latere algemene berichten.
    let mut a = store(1);
    let mut c = store(3);

    post(&mut a, "voor iedereen, nummer 1");
    dm(&mut a, peer(2), "stiekem tussendoor");
    post(&mut a, "voor iedereen, nummer 2");

    push_for(&a, peer(3), &mut c);

    assert_eq!(
        bodies(&c),
        ["voor iedereen, nummer 1", "voor iedereen, nummer 2"],
        "C mag de latere algemene berichten niet mislopen door de DM ertussenin"
    );
}

#[test]
fn dm_convergeert_ook_na_offline_zijn_van_de_geadresseerde() {
    let mut a = store(1);
    let mut b = store(2);

    dm(&mut a, peer(2), "ben je daar?");
    assert!(bodies(&b).is_empty());

    push_for(&a, peer(2), &mut b);
    assert_eq!(bodies(&b), ["ben je daar?"]);
}

#[test]
fn bewerken_werkt_ook_als_de_ontvanger_het_bericht_later_krijgt() {
    let mut a = store(1);
    let mut b = store(2);

    let origineel = post(&mut a, "eerste poging");
    let bewerking = a
        .append_local(
            Channel::GENERAL,
            &OpKind::Edit {
                target: origineel.id(),
                body: "verbeterd".into(),
            },
            0,
        )
        .unwrap();

    // B krijgt de bewerking eerder dan het bericht zelf.
    b.apply_remote(&bewerking).unwrap();
    assert!(bodies(&b).is_empty(), "losse bewerking toont niets");

    b.apply_remote(&origineel).unwrap();
    assert_eq!(bodies(&b), ["verbeterd"]);
}

#[test]
fn subkanaal_bereikt_alle_peers_net_als_algemeen_anders_dan_een_dm() {
    // De kern van de fase 9-uitbreiding: een subkanaal onder "Algemeen" is publiek,
    // niet DM-achtig beperkt tot de geadresseerde.
    let topic = Channel::topic(TopicId::from_bytes([0x42; 16]));
    let mut a = store(1);
    let mut b = store(2);
    let mut c = store(3);

    a.append_local(topic, &fitcom_store::post("in het subkanaal"), 0)
        .unwrap();

    push_for(&a, peer(2), &mut b);
    push_for(&a, peer(3), &mut c);

    for s in [&b, &c] {
        let berichten: Vec<String> = s
            .timeline()
            .unwrap()
            .messages
            .into_iter()
            .map(|m| m.body)
            .collect();
        assert_eq!(
            berichten,
            ["in het subkanaal"],
            "een subkanaal hoort net als algemeen bij iedereen aan te komen"
        );
    }
}

#[test]
fn subkanaal_titel_en_bericht_overleven_een_herstart() {
    // Oefent de blob-encodering van een `Channel::topic` door de echte SQLite-opslag
    // heen — niet alleen in het geheugen, zoals `channel_tests` in `proto` dat al doet.
    let topic = TopicId::from_bytes([0x77; 16]);
    let dir = std::env::temp_dir().join(format!("fitcom-subkanaal-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let pad = dir.join("chat.sqlite");

    {
        let mut s = Store::open(&pad, peer(1)).unwrap();
        s.append_local(
            Channel::GENERAL,
            &OpKind::SetTopicTitle {
                id: topic,
                title: "project x".into(),
            },
            0,
        )
        .unwrap();
        s.append_local(
            Channel::topic(topic),
            &fitcom_store::post("eerste bericht"),
            0,
        )
        .unwrap();
    }
    {
        let s = Store::open(&pad, peer(1)).unwrap();
        let t = s.timeline().unwrap();
        assert_eq!(t.topics[&topic], "project x");
        assert_eq!(t.messages[0].channel, Channel::topic(topic));
        assert_eq!(t.messages[0].body, "eerste bericht");
    }

    let _ = std::fs::remove_dir_all(&dir);
}
