//! Persistente oplog met synchronisatie tussen peers.
//!
//! Geen Windows- of hardware-afhankelijkheden: alles hier is met een database in het
//! geheugen te testen, en dat is precies waar de lastige logica zit.
//!
//! # Waarom de version vector de aaneengesloten reeks bijhoudt en niet het maximum
//!
//! Ops van één auteur worden dicht genummerd (1, 2, 3, ...) binnen hun kanaal, maar ze
//! hoeven niet in die volgorde bij ons aan te komen. Voorbeeld: we halen bij peer B de
//! ops 6 t/m 10 van auteur A op, terwijl A zelf ondertussen op 11 broadcast. Komt die 11
//! eerder binnen dan de inhaalslag, dan hebben we 1-5 en 11 — met een gat ertussen.
//!
//! Zouden we dan `MAX(seq) = 11` als version vector melden, dan zeggen we tegen de
//! andere peers "ik heb alles t/m 11" en krijgen we 6 t/m 10 nooit meer. Daarom houden
//! we per (auteur, kanaal) bij tot hoe ver de reeks *aaneengesloten* is. Een op met een
//! gat ervoor wordt wel bewaard, maar telt pas mee zodra het gat gevuld is.
//!
//! # Waarom kanaal, en niet alleen auteur
//!
//! Sinds direct-berichten (DM's) telt `seq` per **(auteur, kanaal)**, niet per auteur
//! alleen. Een DM tussen A en B mag C nooit bereiken — geen broadcast, geen doorsturen,
//! geen sync. Zou `seq` toch over alle kanalen van A heen lopen, dan zou C bij die DM een
//! permanent gat oplopen: C mag hem nooit ontvangen, dus C's aaneengesloten reeks voor A
//! zou voor altijd blijven steken vlak vóór dat gat — óók voor latere *algemene* berichten
//! van A. Door per (auteur, kanaal) te tellen bestaat dat gat voor C domweg niet: C houdt
//! helemaal geen boekhouding bij voor een kanaal dat hem niet aangaat.
//!
//! Wie welk kanaal mag zien staat in [`VersionVector::visible_to`] en wordt hier
//! toegepast via [`Store::version_vector_for`] en [`Store::ops_missing_in_for`]. De
//! algemene sync-functies (`version_vector`, `ops_missing_in`) blijven ongefilterd
//! bestaan voor lokaal gebruik (de eigen timeline opbouwen), maar mogen nooit gebruikt
//! worden om iets naar een specifieke peer te sturen — dat zou een DM alsnog lekken.

pub mod timeline;

use anyhow::{Context, Result};
use fitcom_proto::{Channel, Op, OpKind, PeerId, VersionVector};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

pub use timeline::{FileEntry, Message, Timeline};

// Doorgeven zodat de app niet ook nog een directe afhankelijkheid op `proto` nodig heeft
// voor de types die overal in de chat-code voorkomen.
pub use fitcom_proto::OpId;

/// Opgehoogd van 1 naar 2 toen `seq` per (auteur, kanaal) ging tellen in plaats van per
/// auteur alleen (DM's). **Wel een migratiepad**: `migreer_v1_naar_v2` zet een bestaande
/// database om in plaats van hem te weigeren — er staat al echte, met de hand opgebouwde
/// chatgeschiedenis in bij Rick, en die mag niet stuklopen op een schema-wijziging.
const SCHEMA_VERSION: i64 = 2;

/// 17-byte blob voor `Channel::GENERAL`: 1 tag-byte (0) + 16 nulbytes. Een bestaande
/// database van vóór de kanalen-uitbreiding kende alleen het algemene kanaal, dus alle
/// migrerende rijen krijgen deze waarde. Hardcoded in plaats van via
/// `fitcom_proto::Channel` gehaald: deze migratie mag nooit meeveranderen als de
/// blob-encodering ooit verandert — hij beschrijft precies wat v1 betekende, niet wat
/// "algemeen" nu betekent.
const ALGEMEEN_KANAAL_BLOB_V1: [u8; 17] = [0u8; 17];

/// Maximum aantal ops per `SyncResponse`. Houdt frames ruim onder `MAX_FRAME_LEN`
/// en voorkomt dat een inhaalslag na maanden offline één gigantisch bericht wordt.
pub const SYNC_BATCH: usize = 500;

pub struct Store {
    conn: Connection,
    me: PeerId,
}

impl Store {
    pub fn open(path: &Path, me: PeerId) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)
            .with_context(|| format!("database openen op {}", path.display()))?;
        Self::init(conn, me)
    }

    pub fn open_in_memory(me: PeerId) -> Result<Self> {
        Self::init(Connection::open_in_memory()?, me)
    }

    fn init(conn: Connection, me: PeerId) -> Result<Self> {
        // WAL: lezen blokkeert schrijven niet. De UI leest opnieuw bij elke wijziging.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;

        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS ops (
                author     BLOB    NOT NULL,
                channel    BLOB    NOT NULL,
                seq        INTEGER NOT NULL,
                lamport    INTEGER NOT NULL,
                wall_clock INTEGER NOT NULL,
                kind       INTEGER NOT NULL,
                payload    BLOB    NOT NULL,
                PRIMARY KEY (author, channel, seq)
            ) WITHOUT ROWID;

            CREATE INDEX IF NOT EXISTS ops_order ON ops(lamport, author);

            -- Tot hoe ver de reeks per (auteur, kanaal) aaneengesloten is. Zie de
            -- moduledocumentatie hierboven.
            CREATE TABLE IF NOT EXISTS authors (
                author     BLOB    NOT NULL,
                channel    BLOB    NOT NULL,
                contiguous INTEGER NOT NULL,
                PRIMARY KEY (author, channel)
            ) WITHOUT ROWID;

            CREATE TABLE IF NOT EXISTS meta (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            "#,
        )
        .context("schema aanmaken")?;

        let mut store = Self { conn, me };
        store.check_schema_version()?;
        Ok(store)
    }

    fn check_schema_version(&mut self) -> Result<()> {
        let found: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'schema_version'",
                [],
                |r| r.get(0),
            )
            .optional()?;

        match found {
            None => {
                self.conn.execute(
                    "INSERT INTO meta (key, value) VALUES ('schema_version', ?1)",
                    params![SCHEMA_VERSION.to_string()],
                )?;
                Ok(())
            }
            Some(v) if v.parse::<i64>().ok() == Some(SCHEMA_VERSION) => Ok(()),
            Some(v) if v.parse::<i64>().ok() == Some(1) && SCHEMA_VERSION == 2 => self
                .migreer_v1_naar_v2()
                .context("database migreren van schema 1 naar 2"),
            Some(v) => anyhow::bail!(
                "database is van schema-versie {v}, deze app verwacht {SCHEMA_VERSION}. \
                 Draai je een oudere versie van de app?"
            ),
        }
    }

    /// Zet een database van vóór de kanalen-uitbreiding om: `ops` en `authors` kregen
    /// een `channel`-kolom en een uitgebreide primary key. Alles wat er al stond was per
    /// definitie op het algemene kanaal — DM's bestonden nog niet — dus die kolom krijgt
    /// overal `ALGEMEEN_KANAAL_BLOB_V1`. Draait in één transactie: gaat er iets mis, dan
    /// blijft de oude database intact in plaats van half gemigreerd.
    fn migreer_v1_naar_v2(&mut self) -> Result<()> {
        let tx = self.conn.transaction()?;

        tx.execute_batch(
            r#"
            ALTER TABLE ops RENAME TO ops_v1;
            CREATE TABLE ops (
                author     BLOB    NOT NULL,
                channel    BLOB    NOT NULL,
                seq        INTEGER NOT NULL,
                lamport    INTEGER NOT NULL,
                wall_clock INTEGER NOT NULL,
                kind       INTEGER NOT NULL,
                payload    BLOB    NOT NULL,
                PRIMARY KEY (author, channel, seq)
            ) WITHOUT ROWID;
            "#,
        )?;
        tx.execute(
            "INSERT INTO ops (author, channel, seq, lamport, wall_clock, kind, payload)
             SELECT author, ?1, seq, lamport, wall_clock, kind, payload FROM ops_v1",
            params![ALGEMEEN_KANAAL_BLOB_V1.to_vec()],
        )?;
        tx.execute_batch(
            r#"
            DROP TABLE ops_v1;
            CREATE INDEX IF NOT EXISTS ops_order ON ops(lamport, author);

            ALTER TABLE authors RENAME TO authors_v1;
            CREATE TABLE authors (
                author     BLOB    NOT NULL,
                channel    BLOB    NOT NULL,
                contiguous INTEGER NOT NULL,
                PRIMARY KEY (author, channel)
            ) WITHOUT ROWID;
            "#,
        )?;
        tx.execute(
            "INSERT INTO authors (author, channel, contiguous)
             SELECT author, ?1, contiguous FROM authors_v1",
            params![ALGEMEEN_KANAAL_BLOB_V1.to_vec()],
        )?;
        tx.execute_batch("DROP TABLE authors_v1;")?;

        tx.execute(
            "UPDATE meta SET value = ?1 WHERE key = 'schema_version'",
            params![SCHEMA_VERSION.to_string()],
        )?;

        tx.commit()?;
        tracing::info!("database gemigreerd van schema 1 naar 2 (kanalen)");
        Ok(())
    }

    pub fn me(&self) -> PeerId {
        self.me
    }

    // -- schrijven ---------------------------------------------------------

    /// Maakt een nieuwe eigen op en slaat hem op. De teruggegeven op moet naar de
    /// andere peers gebroadcast worden — met dien verstande dat de aanroeper zelf
    /// bepaalt wíe: een `Channel::dm(x)`-op hoort alleen naar `x`, nooit breed. Zie
    /// `crates/app/src/chat.rs`.
    pub fn append_local(&mut self, channel: Channel, kind: &OpKind, wall_clock: i64) -> Result<Op> {
        // Onze eigen reeks heeft per definitie geen gaten, dus contiguous == maximum.
        let seq = self.contiguous(self.me, channel)? + 1;
        let lamport = self.max_lamport()? + 1;
        let op = Op::new(self.me, channel, seq, lamport, wall_clock, kind).context("op coderen")?;

        let tx = self.conn.transaction()?;
        insert_op(&tx, &op)?;
        advance_contiguous(&tx, op.author, op.channel)?;
        tx.commit()?;
        Ok(op)
    }

    /// Slaat een op van een andere peer op. `false` betekent: hadden we al.
    ///
    /// Tweemaal toepassen is een no-op — dat is de volledige conflictafhandeling.
    pub fn apply_remote(&mut self, op: &Op) -> Result<bool> {
        Ok(self.apply_remote_batch(std::slice::from_ref(op))? > 0)
    }

    /// Zelfde als [`Store::apply_remote`], maar voor een hele inhaalslag in één
    /// transactie. Levert het aantal ops op dat nieuw was.
    pub fn apply_remote_batch(&mut self, ops: &[Op]) -> Result<usize> {
        if ops.is_empty() {
            return Ok(0);
        }

        let tx = self.conn.transaction()?;
        let mut nieuw = 0usize;
        let mut geraakt: Vec<(PeerId, Channel)> = Vec::new();

        for op in ops {
            if insert_op(&tx, op)? {
                nieuw += 1;
                let sleutel = (op.author, op.channel);
                if !geraakt.contains(&sleutel) {
                    geraakt.push(sleutel);
                }
            }
        }

        // Eén keer per (auteur, kanaal) bijwerken in plaats van per op: bij een
        // inhaalslag van duizenden ops scheelt dat het verschil tussen lineair en
        // kwadratisch.
        for (author, channel) in geraakt {
            advance_contiguous(&tx, author, channel)?;
        }

        tx.commit()?;
        Ok(nieuw)
    }

    // -- lezen -------------------------------------------------------------

    /// `{(auteur, kanaal) -> hoogste seq waarvan we alles t/m hebben}`, **ongefilterd**.
    ///
    /// Alleen voor lokaal gebruik (de eigen timeline opbouwen). Nooit hiermee iets naar
    /// een specifieke peer sturen — gebruik daarvoor [`Store::version_vector_for`], anders
    /// lekt een DM-kanaal naar wie het niet aangaat.
    pub fn version_vector(&self) -> Result<VersionVector> {
        let mut stmt = self
            .conn
            .prepare("SELECT author, channel, contiguous FROM authors WHERE contiguous > 0")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                peer_from_row(r, 0)?,
                channel_from_row(r, 1)?,
                r.get::<_, i64>(2)? as u64,
            ))
        })?;

        let mut vv = VersionVector::new();
        for row in rows {
            let (author, channel, seq) = row?;
            vv.observe(author, channel, seq);
        }
        Ok(vv)
    }

    /// Zoals [`Store::version_vector`], maar beperkt tot wat `viewer` ooit mag zien:
    /// het algemene kanaal, DM's aan `viewer` gericht, en `viewer`'s eigen ops. Dit is
    /// wat je stuurt of vergelijkt in een `SyncRequest`/`SyncResponse` naar `viewer` toe.
    pub fn version_vector_for(&self, viewer: PeerId) -> Result<VersionVector> {
        Ok(self.version_vector()?.visible_to(viewer))
    }

    /// Ops die `theirs` mist en wij hebben, maximaal `max_ops` stuks, **ongefilterd naar
    /// kanaal**. Alleen voor tests en lokaal gebruik; zie [`Store::ops_missing_in_for`]
    /// voor het versturen naar een specifieke peer.
    pub fn ops_missing_in(&self, theirs: &VersionVector, max_ops: usize) -> Result<Vec<Op>> {
        self.ops_missing_in_raw(&self.version_vector()?, theirs, max_ops)
    }

    /// Ops die `viewer` mist en die `viewer` ooit mag zien, maximaal `max_ops` stuks.
    /// Dit is de enige variant die het netwerk in mag: hij filtert eerst onze eigen
    /// versievector op zichtbaarheid vóór hij hem vergelijkt met wat `viewer` claimt te
    /// hebben, dus een DM tussen twee andere peers komt hier nooit uit.
    pub fn ops_missing_in_for(
        &self,
        viewer: PeerId,
        theirs: &VersionVector,
        max_ops: usize,
    ) -> Result<Vec<Op>> {
        let mine = self.version_vector_for(viewer)?;
        self.ops_missing_in_raw(&mine, theirs, max_ops)
    }

    fn ops_missing_in_raw(
        &self,
        mine: &VersionVector,
        theirs: &VersionVector,
        max_ops: usize,
    ) -> Result<Vec<Op>> {
        let mut out = Vec::new();

        for (author, channel, from, to) in mine.ranges_missing_in(theirs) {
            if out.len() >= max_ops {
                break;
            }
            let ruimte = (max_ops - out.len()) as u64;
            let tot = to.min(from + ruimte - 1);
            out.extend(self.ops_range(author, channel, from, tot)?);
        }
        Ok(out)
    }

    /// Of we `theirs` nog iets te bieden hebben, ongefilterd. Zie [`Store::has_more_for`]
    /// voor de kanaal-bewuste variant die naar het netwerk mag.
    pub fn has_more_for(&self, theirs: &VersionVector) -> Result<bool> {
        Ok(!self.version_vector()?.ranges_missing_in(theirs).is_empty())
    }

    /// Of `viewer` na deze vector nog iets van ons te goed heeft, binnen wat hij mag zien.
    pub fn has_more_for_viewer(&self, viewer: PeerId, theirs: &VersionVector) -> Result<bool> {
        Ok(!self
            .version_vector_for(viewer)?
            .ranges_missing_in(theirs)
            .is_empty())
    }

    fn ops_range(&self, author: PeerId, channel: Channel, from: u64, to: u64) -> Result<Vec<Op>> {
        let mut stmt = self.conn.prepare(
            "SELECT author, channel, seq, lamport, wall_clock, kind, payload FROM ops
             WHERE author = ?1 AND channel = ?2 AND seq >= ?3 AND seq <= ?4 ORDER BY seq",
        )?;
        let rows = stmt.query_map(
            params![
                author.as_bytes().to_vec(),
                channel_to_blob(channel).to_vec(),
                from as i64,
                to as i64
            ],
            op_from_row,
        )?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Alle ops, op weergavevolgorde. Voor drie mensen blijft dit klein genoeg om in
    /// één keer te laden; de UI bouwt de timeline alleen opnieuw bij een wijziging.
    pub fn all_ops(&self) -> Result<Vec<Op>> {
        let mut stmt = self.conn.prepare(
            "SELECT author, channel, seq, lamport, wall_clock, kind, payload FROM ops
             ORDER BY lamport, author, seq",
        )?;
        let rows = stmt.query_map([], op_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn timeline(&self) -> Result<Timeline> {
        Ok(timeline::build(&self.all_ops()?))
    }

    pub fn op_count(&self) -> Result<u64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM ops", [], |r| r.get::<_, i64>(0))? as u64)
    }

    fn contiguous(&self, author: PeerId, channel: Channel) -> Result<u64> {
        Ok(self
            .conn
            .query_row(
                "SELECT contiguous FROM authors WHERE author = ?1 AND channel = ?2",
                params![
                    author.as_bytes().to_vec(),
                    channel_to_blob(channel).to_vec()
                ],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
            .unwrap_or(0) as u64)
    }

    fn max_lamport(&self) -> Result<u64> {
        Ok(self
            .conn
            .query_row("SELECT COALESCE(MAX(lamport), 0) FROM ops", [], |r| {
                r.get::<_, i64>(0)
            })? as u64)
    }
}

// -- vrije functies zodat ze ook binnen een transactie bruikbaar zijn -------

/// `false` als we deze op al hadden.
fn insert_op(conn: &Connection, op: &Op) -> Result<bool> {
    let n = conn.execute(
        "INSERT OR IGNORE INTO ops (author, channel, seq, lamport, wall_clock, kind, payload)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            op.author.as_bytes().to_vec(),
            channel_to_blob(op.channel).to_vec(),
            op.seq as i64,
            op.lamport as i64,
            op.wall_clock,
            op.kind_tag as i64,
            op.payload,
        ],
    )?;
    Ok(n > 0)
}

/// Schuift de aaneengesloten reeks van dit (auteur, kanaal)-paar zo ver mogelijk op.
fn advance_contiguous(conn: &Connection, author: PeerId, channel: Channel) -> Result<()> {
    let author_key = author.as_bytes().to_vec();
    let channel_key = channel_to_blob(channel).to_vec();
    let mut c: i64 = conn
        .query_row(
            "SELECT contiguous FROM authors WHERE author = ?1 AND channel = ?2",
            params![author_key.clone(), channel_key.clone()],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(0);

    {
        let mut stmt =
            conn.prepare("SELECT 1 FROM ops WHERE author = ?1 AND channel = ?2 AND seq = ?3")?;
        while stmt
            .query_row(
                params![author_key.clone(), channel_key.clone(), c + 1],
                |_| Ok(()),
            )
            .optional()?
            .is_some()
        {
            c += 1;
        }
    }

    conn.execute(
        "INSERT INTO authors (author, channel, contiguous) VALUES (?1, ?2, ?3)
         ON CONFLICT(author, channel) DO UPDATE SET contiguous = excluded.contiguous",
        params![author_key, channel_key, c],
    )?;
    Ok(())
}

fn peer_from_row(row: &rusqlite::Row<'_>, idx: usize) -> rusqlite::Result<PeerId> {
    let bytes: Vec<u8> = row.get(idx)?;
    let arr: [u8; 16] = bytes.try_into().map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            idx,
            rusqlite::types::Type::Blob,
            "peer-id is geen 16 bytes".into(),
        )
    })?;
    Ok(PeerId::from_bytes(arr))
}

/// `Channel` als 17-byte blob: 1 tag-byte + 16-byte peer-of-subkanaal-id (nullen als
/// afwezig). Een subkanaal-id (`TopicId`) hergebruikt dezelfde 16-byte slot als een
/// DM-peer: de twee sluiten elkaar uit (tag bepaalt welke het is), dus dit kost geen
/// bredere blob en dus geen schema-migratie. Dezelfde aanpak als de
/// bestandsoverdracht-header in `crates/net/src/filestream.rs`.
fn channel_to_blob(channel: Channel) -> [u8; 17] {
    let mut buf = [0u8; 17];
    if let Some(p) = channel.dm_peer() {
        buf[0] = 1;
        buf[1..].copy_from_slice(p.as_bytes());
    } else if let Some(t) = channel.topic_id() {
        buf[0] = 2;
        buf[1..].copy_from_slice(t.as_bytes());
    }
    buf
}

fn channel_from_blob(bytes: &[u8]) -> rusqlite::Result<Channel> {
    if bytes.len() != 17 {
        return Err(rusqlite::Error::FromSqlConversionFailure(
            bytes.len(),
            rusqlite::types::Type::Blob,
            "kanaal is geen 17 bytes".into(),
        ));
    }
    Ok(match bytes[0] {
        1 => {
            let mut peer = [0u8; 16];
            peer.copy_from_slice(&bytes[1..]);
            Channel::dm(PeerId::from_bytes(peer))
        }
        2 => {
            let mut id = [0u8; 16];
            id.copy_from_slice(&bytes[1..]);
            Channel::topic(fitcom_proto::TopicId::from_bytes(id))
        }
        _ => Channel::GENERAL,
    })
}

fn channel_from_row(row: &rusqlite::Row<'_>, idx: usize) -> rusqlite::Result<Channel> {
    let bytes: Vec<u8> = row.get(idx)?;
    channel_from_blob(&bytes)
}

fn op_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Op> {
    Ok(Op {
        author: peer_from_row(row, 0)?,
        channel: channel_from_row(row, 1)?,
        seq: row.get::<_, i64>(2)? as u64,
        lamport: row.get::<_, i64>(3)? as u64,
        wall_clock: row.get(4)?,
        kind_tag: row.get::<_, i64>(5)? as u16,
        payload: row.get(6)?,
    })
}

/// Millis sinds epoch. Alleen voor weergave — nooit voor ordening gebruiken.
pub fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Kortere schrijfwijze voor de meest voorkomende op.
pub fn post(body: impl Into<String>) -> OpKind {
    OpKind::Post { body: body.into() }
}

/// Een bestand aanbieden. De op zelf is de identificatie van de overdracht — zie
/// `fitcom_store::FileEntry` en `fitcom_proto::control::FileRequest`.
pub fn offer_file(name: impl Into<String>, size: u64, hash: [u8; 32]) -> OpKind {
    OpKind::FileMeta {
        name: name.into(),
        size,
        hash,
    }
}
