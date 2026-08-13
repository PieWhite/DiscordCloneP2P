//! Invoercontrole op ops die van het netwerk komen.
//!
//! De convergentietests hiernaast modelleren peers die zich aan de regels houden en zijn
//! daarom blind voor deze klasse fouten: geen enkele test daar liegt over auteurschap,
//! stuurt een kanaalvorm die geen eerlijke encoder produceert, of zet een `lamport` neer
//! die de opslag niet kan bewaren. Elke test hier hoort bij een bevinding uit
//! `docs/BEVEILIGING.md` en noemt hem in zijn naam, zodat de link terug blijft bestaan.
//!
//! De ops worden hier met de hand in elkaar gezet in plaats van via `append_local`: de
//! velden van `Op` zijn `pub`, en dat is precies wat een aanvaller ook heeft — hij schrijft
//! zijn eigen msgpack.

use fitcom_proto::{Channel, Op, OpKind, PeerId};
use fitcom_store::{Store, MAX_LAMPORT_SPRONG, MAX_SEQ_VOORUIT, SYNC_BATCH};

fn peer(n: u8) -> PeerId {
    let mut b = [0u8; 16];
    b[0] = n;
    PeerId::from_bytes(b)
}

fn store(n: u8) -> Store {
    Store::open_in_memory(peer(n)).unwrap()
}

/// Een op zoals een peer hem op de draad zet, zonder enige controle onderweg.
fn rauw(author: PeerId, channel: Channel, seq: u64, lamport: u64, kind: &OpKind) -> Op {
    Op {
        author,
        channel,
        seq,
        lamport,
        wall_clock: 0,
        kind_tag: kind.tag(),
        payload: kind.encode_payload().unwrap(),
    }
}

fn post(author: PeerId, channel: Channel, seq: u64, lamport: u64, tekst: &str) -> Op {
    rauw(
        author,
        channel,
        seq,
        lamport,
        &OpKind::Post { body: tekst.into() },
    )
}

fn bodies(s: &Store) -> Vec<String> {
    s.timeline()
        .unwrap()
        .messages
        .into_iter()
        .map(|m| m.body)
        .collect()
}

// -- B-06: op.author tegen de geauthenticeerde afzender --------------------

#[test]
fn b06_een_dm_namens_iemand_anders_wordt_geweigerd() {
    // C verbindt met B en stuurt een op die zegt van A te zijn, in het DM-kanaal tussen A
    // en B. Er is geen enkele legitieme reden waarom die op via C zou komen: een DM wordt
    // nooit doorgestuurd (ARCHITECTURE, "DM's krijgen geen doorstuurhulp via een derde
    // peer"), dus de afzender ís daar altijd de auteur.
    let mut b = store(2);
    let vervalst = post(peer(1), Channel::dm(peer(2)), 1, 1, "dit zei A nooit");

    assert!(
        !b.apply_remote_from(peer(3), &vervalst).unwrap(),
        "een DM namens een ander hoort geweigerd te worden"
    );
    assert_eq!(b.op_count().unwrap(), 0);
    assert!(bodies(&b).is_empty());

    // En van A zelf komt exact dezelfde op er wel in.
    assert!(b.apply_remote_from(peer(1), &vervalst).unwrap());
    assert_eq!(bodies(&b), ["dit zei A nooit"]);
}

#[test]
fn b06_een_publieke_op_mag_wel_via_een_derde_peer_binnenkomen() {
    // Dit is de reden dat de regel niet simpelweg `op.author == afzender` is: het
    // doorstuurpad uit ARCHITECTURE ("Drie wegen waarlangs een op zich verspreidt", punt 3)
    // dekt gedeeltelijke connectiviteit, en dan komt A's algemene op legitiem via B binnen.
    // Voor een publiek kanaal blijft auteurschap dus onbewijsbaar zonder een handtekening
    // per op — zie het slot van docs/BEVEILIGING.md.
    let mut c = store(3);
    let van_a = post(peer(1), Channel::GENERAL, 1, 1, "hallo allemaal");
    assert!(c.apply_remote_from(peer(2), &van_a).unwrap());
    assert_eq!(bodies(&c), ["hallo allemaal"]);

    // Hetzelfde geldt voor een subkanaal: dat is net zo publiek als "Algemeen" zelf.
    let topic = Channel::topic(fitcom_proto::TopicId::from_bytes([7; 16]));
    let in_subkanaal = post(peer(1), topic, 1, 2, "in het subkanaal");
    assert!(c.apply_remote_from(peer(2), &in_subkanaal).unwrap());
}

#[test]
fn b06_een_op_op_een_onbekend_kanaal_geldt_niet_als_publiek() {
    // Een kanaalsoort die deze build niet kent is niet `is_public()`, dus hij mag net als
    // een DM alleen van de auteur zelf komen. Anders zou een onbekende tag een gaatje in
    // B-06 zijn.
    let mut b = store(2);
    let op = post(peer(1), Channel::onbekend(3), 1, 1, "onbekend kanaal");
    assert!(!b.apply_remote_from(peer(3), &op).unwrap());
    assert!(b.apply_remote_from(peer(1), &op).unwrap());
}

// -- B-07: seq-squatting ---------------------------------------------------

#[test]
fn b07_sleutelbotsing_met_afwijkende_inhoud_wordt_geteld_niet_verzwegen() {
    // Het scenario uit de bevinding: bij het eerste contact bezet C de sleutels van A. Elk
    // van A's echte berichten wordt daarna door `INSERT OR IGNORE` opgeslokt — geen
    // foutpad, geen logregel, geen UI-signaal. De op zelf blijft (terecht) buiten de
    // opslag, want de sleutel is bezet en de log is append-only; wat hier veranderd is, is
    // dat het niet meer stil gebeurt.
    let mut b = store(2);
    let bezet = post(peer(1), Channel::GENERAL, 1, 1, "vervalst door C");
    b.apply_remote(&bezet).unwrap();
    assert_eq!(b.botsingen(), 0, "een verse sleutel is geen botsing");

    let echt = post(peer(1), Channel::GENERAL, 1, 1, "het echte bericht van A");
    assert!(
        !b.apply_remote(&echt).unwrap(),
        "de sleutel is bezet, dus dit is geen nieuwe op"
    );
    assert_eq!(
        b.botsingen(),
        1,
        "een botsing met afwijkende inhoud moet zichtbaar zijn"
    );
    assert_eq!(bodies(&b), ["vervalst door C"]);
}

#[test]
fn b07_een_echt_duplicaat_geeft_geen_valse_melding() {
    // Dezelfde op tweemaal toepassen is de normaalste gang van zaken: broadcast,
    // inhaalslag en periodieke hersync leveren hem alle drie. Een echt duplicaat is
    // byte-identiek, dus dat mag nooit als botsing meetellen — anders is het signaal
    // waardeloos.
    let mut b = store(2);
    let op = post(peer(1), Channel::GENERAL, 1, 1, "eenmaal");
    assert!(b.apply_remote(&op).unwrap());
    assert!(!b.apply_remote(&op).unwrap());
    assert!(!b.apply_remote(&op).unwrap());
    assert_eq!(b.botsingen(), 0);
}

// -- B-08: misvormd kanaal op de opslagsleutel ----------------------------

#[test]
fn b08_een_onbekend_kanaal_krijgt_zijn_eigen_opslagsleutel() {
    // Vóór de fix schreef `channel_to_blob` alles wat geen herkenbare DM of subkanaal was
    // weg als 17 nulbytes — de blob van het algemene kanaal. Een op op een onbekend kanaal
    // botste daarmee op de primary key met A's échte algemene op van hetzelfde seq, en
    // kwam bij het teruglezen bovendien in het algemene kanaal terecht.
    let mut b = store(2);
    let onbekend = post(peer(1), Channel::onbekend(9), 1, 1, "niet algemeen");
    let algemeen = post(peer(1), Channel::GENERAL, 1, 2, "wel algemeen");

    assert!(b.apply_remote(&onbekend).unwrap());
    assert!(
        b.apply_remote(&algemeen).unwrap(),
        "de algemene op hoort niet te botsen met een op op een ander kanaal"
    );
    assert_eq!(b.op_count().unwrap(), 2);
    assert_eq!(b.botsingen(), 0);

    // En het kanaal komt terug zoals het erin ging, niet als `GENERAL`.
    let kanalen: Vec<Channel> = b.all_ops().unwrap().iter().map(|o| o.channel).collect();
    assert!(kanalen.contains(&Channel::onbekend(9)));
    assert!(kanalen.contains(&Channel::GENERAL));

    // De tellers lopen ook apart, dus de onbekende op schuift de algemene reeks niet op.
    let vv = b.version_vector().unwrap();
    assert_eq!(vv.get(peer(1), Channel::GENERAL), 1);
    assert_eq!(vv.get(peer(1), Channel::onbekend(9)), 1);
}

#[test]
fn b08_de_blobs_van_de_bekende_kanalen_zijn_niet_veranderd() {
    // De opslagsleutel is on-disk formaat: verandert die voor tag 0, 1 of 2, dan is een
    // bestaande database van Rick stil onleesbaar geworden. Deze test schrijft met de
    // huidige code en leest terug met een expliciete, hardgecodeerde verwachting van de
    // 17-byte blob.
    let dir = std::env::temp_dir().join(format!("fitcom-b08-blob-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let pad = dir.join("chat.sqlite");

    {
        let mut s = Store::open(&pad, peer(1)).unwrap();
        s.append_local(Channel::GENERAL, &fitcom_store::post("algemeen"), 0)
            .unwrap();
        s.append_local(Channel::dm(peer(2)), &fitcom_store::post("dm"), 0)
            .unwrap();
        s.append_local(
            Channel::topic(fitcom_proto::TopicId::from_bytes([0x33; 16])),
            &fitcom_store::post("subkanaal"),
            0,
        )
        .unwrap();
    }

    let conn = rusqlite::Connection::open(&pad).unwrap();
    let mut stmt = conn
        .prepare("SELECT channel FROM ops ORDER BY lamport")
        .unwrap();
    let blobs: Vec<Vec<u8>> = stmt
        .query_map([], |r| r.get::<_, Vec<u8>>(0))
        .unwrap()
        .map(|b| b.unwrap())
        .collect();

    let mut algemeen = vec![0u8; 17];
    algemeen[0] = 0;
    let mut dm = vec![1u8];
    dm.extend_from_slice(peer(2).as_bytes());
    let mut sub = vec![2u8];
    sub.extend_from_slice(&[0x33; 16]);

    assert_eq!(blobs, vec![algemeen, dm, sub]);

    drop(stmt);
    drop(conn);
    let _ = std::fs::remove_dir_all(&dir);
}

// -- B-14 en B-34: getallen die de opslag niet kan bewaren -----------------

#[test]
fn b14_een_onmogelijke_lamport_sprong_wordt_geweigerd() {
    // `lamport = i64::MAX` is positief en wordt dus wél opgepikt door `MAX(lamport)`.
    // Daarna is `max_lamport() + 1` gelijk aan 2⁶³, dat als `i64::MIN` opgeslagen wordt,
    // waarna elke volgende eigen op exact dezelfde lamport krijgt en de eigen tijdlijn
    // niet meer ordenbaar is. Eén bericht, permanent.
    let mut b = store(2);
    let bom = post(peer(1), Channel::GENERAL, 1, i64::MAX as u64, "lamport-bom");
    assert!(!b.apply_remote_from(peer(1), &bom).unwrap());
    assert_eq!(b.op_count().unwrap(), 0);

    // Onze eigen klok is dus ongemoeid en loopt gewoon door.
    let eigen = b
        .append_local(Channel::GENERAL, &fitcom_store::post("gewoon door"), 0)
        .unwrap();
    assert_eq!(eigen.lamport, 1);

    // Een gewone voorsprong is geen probleem: peers lopen normaal uit elkaar.
    let normaal = post(
        peer(1),
        Channel::GENERAL,
        1,
        5_000,
        "ik was lang aan het praten",
    );
    assert!(b.apply_remote_from(peer(1), &normaal).unwrap());

    // De grens zit op de sprong, niet op de absolute waarde.
    let net_te_ver = post(
        peer(1),
        Channel::GENERAL,
        2,
        5_000 + MAX_LAMPORT_SPRONG + 1,
        "te ver",
    );
    assert!(!b.apply_remote_from(peer(1), &net_te_ver).unwrap());
}

#[test]
fn b34_een_seq_boven_i64_max_komt_de_opslag_niet_in() {
    // Zulke rijen zijn na opslag onbereikbaar voor `ops_range` en `advance_contiguous`
    // (die met positieve grenzen werken) maar tellen wel mee in `op_count()` en komen mee
    // in `all_ops()`: 2⁶³ permanent inerte sleutels. `proto` weigert hem al bij het
    // decoderen; dit is de tweede lijn, voor een op die uit een oudere database komt.
    let mut b = store(2);
    let op = post(peer(1), Channel::GENERAL, u64::MAX, 1, "onbereikbaar");
    assert!(!b.apply_remote_from(peer(1), &op).unwrap());
    assert_eq!(b.op_count().unwrap(), 0);
}

// -- B-15: één te grote op ------------------------------------------------

#[test]
fn b15_een_op_die_niet_meer_doorstuurbaar_is_wordt_geweigerd() {
    // De invariant is "wat ik kan ontvangen, kan ik doorsturen". Werd deze op geaccepteerd,
    // dan sneuvelde bij het doorsturen (of bij de volgende sync-batch) `write_frame` en
    // brak de schrijftaak af — en bij herverbinding opnieuw, permanent.
    let mut b = store(2);
    let kind = OpKind::Post {
        body: "a".repeat(400 * 1024),
    };
    let te_groot = rauw(peer(1), Channel::GENERAL, 1, 1, &kind);
    assert!(!b.apply_remote_from(peer(1), &te_groot).unwrap());
    assert_eq!(b.op_count().unwrap(), 0);
}

#[test]
fn b15_een_sync_batch_budgetteert_op_bytes_en_niet_alleen_op_aantal() {
    // `SYNC_BATCH` is een aantal, geen bytebudget: 500 grote ops passen samen niet in een
    // control-frame. De batch moet dus eerder ophouden dan bij 500 stuks.
    let mut a = store(1);
    let mut b = store(2);
    let tekst = "a".repeat(4000);
    for _ in 0..400 {
        a.append_local(Channel::GENERAL, &fitcom_store::post(&tekst), 0)
            .unwrap();
    }

    let batch = a
        .ops_missing_in(&b.version_vector().unwrap(), SYNC_BATCH)
        .unwrap();
    assert!(!batch.is_empty(), "er valt genoeg te sturen");
    assert!(
        batch.len() < 400,
        "het bytebudget hoort eerder te knijpen dan het aantal ({} ops)",
        batch.len()
    );
    let bytes: usize = batch.iter().map(|o| o.payload.len()).sum();
    assert!(
        bytes < fitcom_proto::MAX_FRAME_LEN,
        "een batch moet in een frame passen"
    );

    // En de sync komt nog steeds tot rust: het budget knijpt, het blokkeert niet.
    for _ in 0..100 {
        let batch = a
            .ops_missing_in(&b.version_vector().unwrap(), SYNC_BATCH)
            .unwrap();
        if batch.is_empty() {
            break;
        }
        b.apply_remote_batch(&batch).unwrap();
    }
    assert_eq!(b.op_count().unwrap(), 400);
}

// -- B-16: onbegrensde oplog-groei ---------------------------------------

#[test]
fn b16_een_seq_ver_voorbij_de_frontier_wordt_geweigerd() {
    // Ops met een gat ervoor worden bewaard maar tellen nooit mee, en er is nergens een
    // verwijderpad — dus zonder venster kan een peer onbeperkt sleutels vullen die nooit
    // opgeruimd worden.
    let mut b = store(2);
    let ver_weg = post(
        peer(1),
        Channel::GENERAL,
        MAX_SEQ_VOORUIT + 2,
        1,
        "ver voorbij de frontier",
    );
    assert!(!b.apply_remote_from(peer(1), &ver_weg).unwrap());
    assert_eq!(b.op_count().unwrap(), 0);

    // Binnen het venster mag herordening wel: dat is het normale geval waarin een live
    // broadcast de inhaalslag inhaalt.
    let vooruit = post(peer(1), Channel::GENERAL, 5, 1, "kwam eerst aan");
    assert!(b.apply_remote_from(peer(1), &vooruit).unwrap());
    assert_eq!(
        b.version_vector().unwrap().get(peer(1), Channel::GENERAL),
        0,
        "een op met een gat ervoor telt nog niet mee"
    );

    // Een seq van 0 bestaat niet — de reeks is 1-gebaseerd.
    let nul = post(peer(1), Channel::GENERAL, 0, 1, "bestaat niet");
    assert!(!b.apply_remote_from(peer(1), &nul).unwrap());
}

#[test]
fn b16_een_aaneengesloten_inhaalslag_groter_dan_het_venster_komt_er_wel_in() {
    // Het venster mag geen echte inhaalslag afknijpen: een aaneengesloten reeks schuift de
    // frontier bij elke op mee op, dus 1..N in één batch hoort te werken ook als N groter
    // is dan `MAX_SEQ_VOORUIT`.
    let mut b = store(2);
    let n = MAX_SEQ_VOORUIT + 500;
    let ops: Vec<Op> = (1..=n)
        .map(|i| post(peer(1), Channel::GENERAL, i, i, &format!("bericht {i}")))
        .collect();

    assert_eq!(
        b.apply_remote_batch_from(peer(1), &ops).unwrap(),
        n as usize
    );
    assert_eq!(
        b.version_vector().unwrap().get(peer(1), Channel::GENERAL),
        n
    );
}

#[test]
fn b16_de_tijdlijn_leest_nooit_meer_dan_het_plafond_in() {
    // `timeline()` wordt na élke wijziging opnieuw opgebouwd uit `all_ops()`. Zonder
    // plafond wordt bij een opgeblazen log de complete log per binnenkomend bericht
    // opnieuw ingeladen en gesorteerd.
    let mut a = store(1);
    for i in 0..50 {
        a.append_local(Channel::GENERAL, &fitcom_store::post(format!("{i}")), 0)
            .unwrap();
    }

    // Het plafond houdt de *nieuwste* ops, want dat is wat de UI toont.
    let laatste = a.all_ops_limited(10).unwrap();
    assert_eq!(laatste.len(), 10);
    assert_eq!(laatste.first().unwrap().seq, 41);
    assert_eq!(laatste.last().unwrap().seq, 50);

    // En de volgorde is nog steeds de weergavevolgorde, niet omgekeerd.
    let mut op_volgorde = laatste.clone();
    op_volgorde.sort_by_key(|o| o.order_key());
    assert_eq!(op_volgorde, laatste);

    // Zonder plafond in zicht verandert er niets aan het bestaande gedrag.
    assert_eq!(a.all_ops().unwrap().len(), 50);
}
