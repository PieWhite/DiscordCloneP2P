//! Persistente oplog met synchronisatie tussen peers.
//!
//! Geen Windows- of hardware-afhankelijkheden: alles hier is met een database in het
//! geheugen te testen, en dat is precies waar de lastige logica zit.
//!
//! # Waarom de version vector de aaneengesloten reeks bijhoudt en niet het maximum
//!
//! Ops van één auteur worden dicht genummerd (1, 2, 3, ...), maar ze hoeven niet in
//! die volgorde bij ons aan te komen. Voorbeeld: we halen bij peer B de ops 6 t/m 10
//! van auteur A op, terwijl A zelf ondertussen op 11 broadcast. Komt die 11 eerder
//! binnen dan de inhaalslag, dan hebben we 1-5 en 11 — met een gat ertussen.
//!
//! Zouden we dan `MAX(seq) = 11` als version vector melden, dan zeggen we tegen de
//! andere peers "ik heb alles t/m 11" en krijgen we 6 t/m 10 nooit meer. Daarom houden
//! we per auteur bij tot hoe ver de reeks *aaneengesloten* is. Een op met een gat
//! ervoor wordt wel bewaard, maar telt pas mee zodra het gat gevuld is.

pub mod timeline;

use anyhow::{Context, Result};
use fitcom_proto::{Op, OpKind, PeerId, VersionVector};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

pub use timeline::{FileEntry, Message, Timeline};

// Doorgeven zodat de app niet ook nog een directe afhankelijkheid op `proto` nodig heeft
// voor de types die overal in de chat-code voorkomen.
pub use fitcom_proto::{OpId, VersionVector as SyncState};

const SCHEMA_VERSION: i64 = 1;

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
                seq        INTEGER NOT NULL,
                lamport    INTEGER NOT NULL,
                wall_clock INTEGER NOT NULL,
                kind       INTEGER NOT NULL,
                payload    BLOB    NOT NULL,
                PRIMARY KEY (author, seq)
            ) WITHOUT ROWID;

            CREATE INDEX IF NOT EXISTS ops_order ON ops(lamport, author);

            -- Tot hoe ver de reeks per auteur aaneengesloten is. Zie de moduledocumentatie.
            CREATE TABLE IF NOT EXISTS authors (
                author     BLOB    PRIMARY KEY,
                contiguous INTEGER NOT NULL
            ) WITHOUT ROWID;

            CREATE TABLE IF NOT EXISTS meta (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            "#,
        )
        .context("schema aanmaken")?;

        let store = Self { conn, me };
        store.check_schema_version()?;
        Ok(store)
    }

    fn check_schema_version(&self) -> Result<()> {
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
            Some(v) => anyhow::bail!(
                "database is van schema-versie {v}, deze app verwacht {SCHEMA_VERSION}. \
                 Draai je een oudere versie van de app?"
            ),
        }
    }

    pub fn me(&self) -> PeerId {
        self.me
    }

    // -- schrijven ---------------------------------------------------------

    /// Maakt een nieuwe eigen op en slaat hem op. De teruggegeven op moet naar de
    /// andere peers gebroadcast worden.
    pub fn append_local(&mut self, kind: &OpKind, wall_clock: i64) -> Result<Op> {
        // Onze eigen reeks heeft per definitie geen gaten, dus contiguous == maximum.
        let seq = self.contiguous(self.me)? + 1;
        let lamport = self.max_lamport()? + 1;
        let op = Op::new(self.me, seq, lamport, wall_clock, kind).context("op coderen")?;

        let tx = self.conn.transaction()?;
        insert_op(&tx, &op)?;
        advance_contiguous(&tx, op.author)?;
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
        let mut geraakt: Vec<PeerId> = Vec::new();

        for op in ops {
            if insert_op(&tx, op)? {
                nieuw += 1;
                if !geraakt.contains(&op.author) {
                    geraakt.push(op.author);
                }
            }
        }

        // Eén keer per auteur bijwerken in plaats van per op: bij een inhaalslag van
        // duizenden ops scheelt dat het verschil tussen lineair en kwadratisch.
        for author in geraakt {
            advance_contiguous(&tx, author)?;
        }

        tx.commit()?;
        Ok(nieuw)
    }

    // -- lezen -------------------------------------------------------------

    /// `{auteur -> hoogste seq waarvan we alles t/m hebben}`.
    pub fn version_vector(&self) -> Result<VersionVector> {
        let mut stmt = self
            .conn
            .prepare("SELECT author, contiguous FROM authors WHERE contiguous > 0")?;
        let rows = stmt.query_map([], |r| {
            Ok((peer_from_row(r, 0)?, r.get::<_, i64>(1)? as u64))
        })?;

        let mut vv = VersionVector::new();
        for row in rows {
            let (author, seq) = row?;
            vv.observe(author, seq);
        }
        Ok(vv)
    }

    /// Ops die `theirs` mist en wij hebben, maximaal `max_ops` stuks.
    ///
    /// Levert nooit een op voorbij onze eigen aaneengesloten reeks: dan zou de
    /// ontvanger een gat krijgen en zou zijn version vector gaan liegen.
    pub fn ops_missing_in(&self, theirs: &VersionVector, max_ops: usize) -> Result<Vec<Op>> {
        let mine = self.version_vector()?;
        let mut out = Vec::new();

        for (author, from, to) in mine.ranges_missing_in(theirs) {
            if out.len() >= max_ops {
                break;
            }
            let ruimte = (max_ops - out.len()) as u64;
            let tot = to.min(from + ruimte - 1);
            out.extend(self.ops_range(author, from, tot)?);
        }
        Ok(out)
    }

    /// Of we `theirs` nog iets te bieden hebben.
    pub fn has_more_for(&self, theirs: &VersionVector) -> Result<bool> {
        Ok(!self.version_vector()?.ranges_missing_in(theirs).is_empty())
    }

    fn ops_range(&self, author: PeerId, from: u64, to: u64) -> Result<Vec<Op>> {
        let mut stmt = self.conn.prepare(
            "SELECT author, seq, lamport, wall_clock, kind, payload FROM ops
             WHERE author = ?1 AND seq >= ?2 AND seq <= ?3 ORDER BY seq",
        )?;
        let rows = stmt.query_map(
            params![author.as_bytes().to_vec(), from as i64, to as i64],
            op_from_row,
        )?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Alle ops, op weergavevolgorde. Voor drie mensen blijft dit klein genoeg om in
    /// één keer te laden; de UI bouwt de timeline alleen opnieuw bij een wijziging.
    pub fn all_ops(&self) -> Result<Vec<Op>> {
        let mut stmt = self.conn.prepare(
            "SELECT author, seq, lamport, wall_clock, kind, payload FROM ops
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

    fn contiguous(&self, author: PeerId) -> Result<u64> {
        Ok(self
            .conn
            .query_row(
                "SELECT contiguous FROM authors WHERE author = ?1",
                params![author.as_bytes().to_vec()],
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
        "INSERT OR IGNORE INTO ops (author, seq, lamport, wall_clock, kind, payload)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            op.author.as_bytes().to_vec(),
            op.seq as i64,
            op.lamport as i64,
            op.wall_clock,
            op.kind_tag as i64,
            op.payload,
        ],
    )?;
    Ok(n > 0)
}

/// Schuift de aaneengesloten reeks van deze auteur zo ver mogelijk op.
fn advance_contiguous(conn: &Connection, author: PeerId) -> Result<()> {
    let key = author.as_bytes().to_vec();
    let mut c: i64 = conn
        .query_row(
            "SELECT contiguous FROM authors WHERE author = ?1",
            params![key.clone()],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(0);

    {
        let mut stmt = conn.prepare("SELECT 1 FROM ops WHERE author = ?1 AND seq = ?2")?;
        while stmt
            .query_row(params![key.clone(), c + 1], |_| Ok(()))
            .optional()?
            .is_some()
        {
            c += 1;
        }
    }

    conn.execute(
        "INSERT INTO authors (author, contiguous) VALUES (?1, ?2)
         ON CONFLICT(author) DO UPDATE SET contiguous = excluded.contiguous",
        params![key, c],
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

fn op_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Op> {
    Ok(Op {
        author: peer_from_row(row, 0)?,
        seq: row.get::<_, i64>(1)? as u64,
        lamport: row.get::<_, i64>(2)? as u64,
        wall_clock: row.get(3)?,
        kind_tag: row.get::<_, i64>(4)? as u16,
        payload: row.get(5)?,
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
