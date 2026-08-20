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

pub use timeline::{FileEntry, Message, Timeline, WordleEntry};

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

/// Bytebudget per `SyncResponse`, náást [`SYNC_BATCH`] (B-15). Een aantal is geen
/// bytebudget: 500 ops van elk net onder `MAX_OP_LEN` is 128 MiB en dus ver boven
/// `MAX_FRAME_LEN` (16 MiB). `write_frame` gaf dan een fout en de schrijftaak brak af —
/// bij herverbinding bouwde `beantwoord_sync` dezelfde batch opnieuw op en sneuvelde de
/// verbinding opnieuw, permanent. 1 MiB laat ruimte voor msgpack-overhead en is nog altijd
/// tien keer meer dan een normale inhaalslag van 500 chatberichten.
pub const SYNC_BATCH_BYTES: usize = 1024 * 1024;

/// Bovengrens voor één op (B-15). "Wat ik kan ontvangen, kan ik doorsturen" is de
/// invariant die dit bewaakt: een op die geaccepteerd wordt maar niet meer in een frame
/// past is een permanente breuk van de control-verbinding. Ruim boven [`MAX_BERICHT_LEN`]
/// uit `proto` (4 KiB), zodat er plek is voor toekomstige soorten met meer inhoud.
pub const MAX_OP_LEN: usize = 256 * 1024;

/// Wat een op buiten `payload` kost op de draad: twee UUID's, drie getallen, een tag en de
/// msgpack-veldnamen. Ruim naar boven afgerond — dit is een budget, geen boekhouding.
const OP_VASTE_OVERHEAD: usize = 128;

/// Hoogste `seq`/`lamport` die in een `i64` past, en dus in SQLite. Wordt door `proto` al
/// bij het decoderen geweigerd (B-14, B-34); hier nog een keer, want een op kan ook uit
/// een oudere database komen.
const MAX_OPSLAG_GETAL: u64 = i64::MAX as u64;

/// Hoeveel een ontvangen `lamport` boven onze eigen hoogste mag liggen (B-14). In een mesh
/// van drie peers is een sprong van 2³² nooit legitiem — dat zijn vier miljard berichten
/// die wij gemist zouden hebben. Zonder deze grens kan één op met `lamport = i64::MAX` de
/// eigen klok voorgoed vastzetten: `max_lamport() + 1` is dan 2⁶³, dat als `i64::MIN`
/// opgeslagen wordt, waarna `MAX(lamport)` op `i64::MAX` blijft staan en élke volgende
/// eigen op exact dezelfde lamport krijgt. Eén bericht, permanent onordenbare tijdlijn.
pub const MAX_LAMPORT_SPRONG: u64 = 1 << 32;

/// Hoe ver voorbij de aaneengesloten frontier een `seq` mag liggen (B-16). Ops met een gat
/// ervoor worden bewaard maar tellen nooit mee, en er is nergens een verwijderpad — dus
/// zonder deze grens kan een peer onbeperkt sleutels vullen die nooit opgeruimd worden.
/// Herordening heeft nooit meer dan een handvol nodig: de inhaalslag levert altijd vanaf
/// `have + 1` en alleen een live broadcast kan vooruitlopen.
pub const MAX_SEQ_VOORUIT: u64 = 1000;

/// Plafond op wat [`Store::all_ops`] in één keer in het geheugen zet (B-16). `timeline()`
/// wordt na *elke* wijziging opnieuw opgebouwd, dus dit is de rem op "hele opgeblazen log
/// bij elk binnenkomend bericht opnieuw inladen en sorteren". Voor drie mensen is 100 000
/// ops onbereikbaar in normaal gebruik; wordt de grens toch geraakt, dan valt de *oudste*
/// geschiedenis buiten beeld en staat er een waarschuwing in het log.
pub const MAX_TIMELINE_OPS: usize = 100_000;

/// Waarom een ontvangen op geweigerd is. Alleen voor het log en voor de tests — een
/// afwijzing is geen fout die omhoog hoort te bubbelen: een liegende of kapotte peer mag de
/// verbinding niet slopen (invariant 7). Zie `docs/BEVEILIGING.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Afwijzing {
    /// B-06: `op.author` is niet de geauthenticeerde afzender, en dit is geen publiek
    /// kanaal waar doorsturen door een derde peer legitiem is.
    VerkeerdeAfzender,
    /// B-34: `seq` past niet in een `i64`.
    SeqTeGroot,
    /// B-14: `lamport` past niet in een `i64`.
    LamportTeGroot,
    /// B-14: `lamport` ligt onmogelijk ver boven de onze.
    LamportSprong,
    /// B-16: `seq` ligt buiten het venster voorbij de aaneengesloten frontier, of is 0.
    SeqBuitenVenster,
    /// B-15: de op past niet meer in een control-frame en zou dus niet doorstuurbaar zijn.
    OpTeGroot,
}

impl std::fmt::Display for Afwijzing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::VerkeerdeAfzender => {
                "auteur is niet de afzender en het kanaal is geen publiek kanaal (B-06)"
            }
            Self::SeqTeGroot => "seq past niet in een i64 (B-34)",
            Self::LamportTeGroot => "lamport past niet in een i64 (B-14)",
            Self::LamportSprong => "lamport springt onmogelijk ver vooruit (B-14)",
            Self::SeqBuitenVenster => {
                "seq ligt buiten het venster voorbij de aaneengesloten reeks (B-16)"
            }
            Self::OpTeGroot => "op is te groot om nog door te kunnen sturen (B-15)",
        };
        f.write_str(s)
    }
}

pub struct Store {
    conn: Connection,
    me: PeerId,
    /// Hoeveel sleutelbotsingen met *afwijkende* inhoud we gezien hebben (B-07). Een echt
    /// duplicaat is byte-identiek, dus alles wat hier meetelt is een peer die een
    /// `(auteur, kanaal, seq)` bezet met andere inhoud dan de eigenaar er neerzette.
    botsingen: u64,
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

        let mut store = Self {
            conn,
            me,
            botsingen: 0,
        };
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
        let vorige = self.max_lamport()?;
        // Onbereikbaar zolang `keur_op` de lamport-sprong begrenst — een ontvangen op kan
        // hem niet meer hierheen tillen. Blijft staan als vangnet voor een database die van
        // vóór die grens komt: doorgaan zou 2⁶³ als `i64::MIN` opslaan en daarna elke eigen
        // op dezelfde lamport geven. Zie B-14.
        if vorige >= MAX_OPSLAG_GETAL {
            anyhow::bail!(
                "lamport-klok staat op {vorige} en kan niet verder; de oplog is niet meer \
                 ordenbaar (B-14). Verwijder de op met die lamport of begin met een nieuwe \
                 database."
            );
        }
        let op =
            Op::new(self.me, channel, seq, vorige + 1, wall_clock, kind).context("op coderen")?;

        let tx = self.conn.transaction()?;
        insert_op(&tx, &op)?;
        advance_contiguous(&tx, op.author, op.channel)?;
        tx.commit()?;
        Ok(op)
    }

    /// Slaat een op van een andere peer op. `false` betekent: hadden we al, of geweigerd.
    ///
    /// Tweemaal toepassen is een no-op — dat is de volledige conflictafhandeling.
    ///
    /// **Zonder afzender, dus zonder de auteurscontrole van B-06.** Gebruik
    /// [`Store::apply_remote_from`] zodra je weet wie het stuurde; dit blijft bestaan voor
    /// lokaal gebruik en voor de tests, en doet alle overige controles wel.
    pub fn apply_remote(&mut self, op: &Op) -> Result<bool> {
        Ok(self.apply_remote_batch(std::slice::from_ref(op))? > 0)
    }

    /// Zelfde als [`Store::apply_remote`], maar met de geauthenticeerde afzender erbij, dus
    /// mét de auteurscontrole van B-06.
    ///
    /// `false` betekent: hadden we al, óf de op is geweigerd. Dat is precies wat de
    /// aanroeper nodig heeft — hij stuurt alleen door wat nieuw was, en een geweigerde op
    /// mag hij niet doorsturen.
    pub fn apply_remote_from(&mut self, van: PeerId, op: &Op) -> Result<bool> {
        Ok(self.apply_remote_batch_from(van, std::slice::from_ref(op))? > 0)
    }

    /// Zelfde als [`Store::apply_remote`], maar voor een hele inhaalslag in één
    /// transactie. Levert het aantal ops op dat nieuw was.
    pub fn apply_remote_batch(&mut self, ops: &[Op]) -> Result<usize> {
        self.apply_remote_intern(None, ops)
    }

    /// Zelfde als [`Store::apply_remote_batch`], maar mét de auteurscontrole van B-06.
    pub fn apply_remote_batch_from(&mut self, van: PeerId, ops: &[Op]) -> Result<usize> {
        self.apply_remote_intern(Some(van), ops)
    }

    /// Hoeveel sleutelbotsingen met afwijkende inhoud deze store gezien heeft (B-07).
    /// Loopt alleen op bij een peer die een `(auteur, kanaal, seq)` van iemand anders bezet
    /// hield; een echt duplicaat is byte-identiek en telt niet mee.
    pub fn botsingen(&self) -> u64 {
        self.botsingen
    }

    fn apply_remote_intern(&mut self, van: Option<PeerId>, ops: &[Op]) -> Result<usize> {
        if ops.is_empty() {
            return Ok(0);
        }

        // Eén keer per batch opvragen in plaats van per op: de sprong die we bewaken is
        // 2³², dus of we tegen de stand van vóór of tijdens de batch vergelijken maakt
        // geen praktisch verschil.
        let lokaal_max_lamport = self.max_lamport()?;

        let tx = self.conn.transaction()?;
        let mut nieuw = 0usize;
        let mut botsingen = 0u64;
        let mut geweigerd = 0usize;
        let mut geraakt: Vec<(PeerId, Channel)> = Vec::new();
        // De aaneengesloten frontier per (auteur, kanaal), bijgehouden binnen de batch:
        // een query per paar in plaats van per op, en hij schuift mee met elke op die het
        // gat sluit. Dat laatste is nodig omdat een inhaalslag van 1..N in één batch kan
        // komen en dan groter is dan `MAX_SEQ_VOORUIT`. Alleen `frontier + 1` schuift op —
        // meebewegen met élke geaccepteerde seq zou het venster tot een glijdend venster
        // maken en dan is B-16 weer open.
        let mut frontier: Vec<((PeerId, Channel), u64)> = Vec::new();

        for op in ops {
            let sleutel = (op.author, op.channel);
            let idx = match frontier.iter().position(|(k, _)| *k == sleutel) {
                Some(i) => i,
                None => {
                    let c = contiguous_in(&tx, op.author, op.channel)?;
                    frontier.push((sleutel, c));
                    frontier.len() - 1
                }
            };

            if let Err(reden) = keur_op(van, op, lokaal_max_lamport, frontier[idx].1) {
                tracing::warn!(afzender = ?van, op = ?op.id(), %reden, "op geweigerd");
                geweigerd += 1;
                continue;
            }

            match insert_op(&tx, op)? {
                Insert::Nieuw => {
                    nieuw += 1;
                    if !geraakt.contains(&sleutel) {
                        geraakt.push(sleutel);
                    }
                }
                Insert::Duplicaat => {}
                Insert::Botsing => botsingen += 1,
            }

            if op.seq == frontier[idx].1 + 1 {
                frontier[idx].1 += 1;
            }
        }

        // Eén keer per (auteur, kanaal) bijwerken in plaats van per op: bij een
        // inhaalslag van duizenden ops scheelt dat het verschil tussen lineair en
        // kwadratisch.
        for (author, channel) in geraakt {
            advance_contiguous(&tx, author, channel)?;
        }

        tx.commit()?;
        self.botsingen += botsingen;
        if geweigerd > 0 {
            tracing::warn!(
                afzender = ?van,
                geweigerd,
                aangeboden = ops.len(),
                "ops geweigerd bij de invoercontrole"
            );
        }
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

    /// Budgetteert op **aantal én bytes** (B-15). Op alleen een aantal budgetteren geeft bij
    /// grote ops een frame boven `MAX_FRAME_LEN`, en dan breekt de schrijftaak in `net` af —
    /// bij herverbinding wordt dezelfde batch opnieuw opgebouwd en breekt hij opnieuw af.
    /// Er komt altijd minstens één op uit als er iets te sturen valt, ook als die op zelf
    /// al over het bytebudget gaat: anders zou een enkele te grote op (uit een database van
    /// vóór `MAX_OP_LEN`) de sync voorgoed laten stilstaan.
    fn ops_missing_in_raw(
        &self,
        mine: &VersionVector,
        theirs: &VersionVector,
        max_ops: usize,
    ) -> Result<Vec<Op>> {
        let mut out = Vec::new();
        let mut bytes = 0usize;

        for (author, channel, from, to) in mine.ranges_missing_in(theirs) {
            if out.len() >= max_ops || bytes >= SYNC_BATCH_BYTES {
                break;
            }
            let ruimte = (max_ops - out.len()) as u64;
            let tot = to.min(from + ruimte - 1);
            for op in self.ops_range(author, channel, from, tot, SYNC_BATCH_BYTES - bytes)? {
                bytes += op_wire_len(&op);
                out.push(op);
            }
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

    /// `budget_bytes` stopt de reeks zodra hij vol is, maar levert altijd minstens één op —
    /// zie [`Store::ops_missing_in_raw`]. De rij-iterator van rusqlite is lui, dus wat we
    /// niet meenemen wordt ook niet uit de database gehaald.
    fn ops_range(
        &self,
        author: PeerId,
        channel: Channel,
        from: u64,
        to: u64,
        budget_bytes: usize,
    ) -> Result<Vec<Op>> {
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

        let mut out = Vec::new();
        let mut bytes = 0usize;
        for row in rows {
            let op = row?;
            bytes += op_wire_len(&op);
            out.push(op);
            if bytes >= budget_bytes {
                break;
            }
        }
        Ok(out)
    }

    /// Alle ops, op weergavevolgorde — begrensd op [`MAX_TIMELINE_OPS`]. Voor drie mensen
    /// blijft dit klein genoeg om in één keer te laden; de UI bouwt de timeline alleen
    /// opnieuw bij een wijziging.
    pub fn all_ops(&self) -> Result<Vec<Op>> {
        self.all_ops_limited(MAX_TIMELINE_OPS)
    }

    /// De **nieuwste** `limit` ops, op weergavevolgorde (B-16). `timeline()` wordt na elke
    /// wijziging opnieuw opgebouwd uit dit resultaat; zonder plafond wordt bij een
    /// opgeblazen log de complete log per binnenkomend bericht opnieuw ingeladen en
    /// gesorteerd. De nieuwste houden en niet de oudste is de enige zinnige kant: de UI
    /// toont het recente gesprek.
    pub fn all_ops_limited(&self, limit: usize) -> Result<Vec<Op>> {
        let mut stmt = self.conn.prepare(
            "SELECT author, channel, seq, lamport, wall_clock, kind, payload FROM ops
             ORDER BY lamport DESC, author DESC, seq DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], op_from_row)?;
        let mut ops = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        // DESC gelezen zodat het plafond de nieuwste houdt; omdraaien geeft weer precies
        // `ORDER BY lamport, author, seq`.
        ops.reverse();
        if ops.len() >= limit {
            tracing::warn!(
                limit,
                totaal = self.op_count().unwrap_or(0),
                "oplog is groter dan het plafond; de oudste geschiedenis valt buiten de \
                 tijdlijn (B-16)"
            );
        }
        Ok(ops)
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
        contiguous_in(&self.conn, author, channel)
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

/// Invoercontrole op een op die van het netwerk komt. `van` is de geauthenticeerde
/// afzender, of `None` als die niet bekend is (lokaal gebruik en tests).
///
/// Alles hier weigert vormen die een eerlijke peer nooit produceert. Zie
/// `docs/BEVEILIGING.md` bij de genoemde bevindingen voor het waarom per regel.
fn keur_op(
    van: Option<PeerId>,
    op: &Op,
    lokaal_max_lamport: u64,
    frontier: u64,
) -> std::result::Result<(), Afwijzing> {
    // B-06. Een *publieke* op mag legitiem van een derde peer komen: dat is het
    // doorstuurmechanisme uit ARCHITECTURE, "Drie wegen waarlangs een op zich verspreidt".
    // Voor een DM bestaat dat mechanisme bewust niet, dus daar is de afzender altijd de
    // auteur. Volledig sluiten kan alleen met een handtekening per op.
    if let Some(van) = van {
        if op.author != van && !op.channel.is_public() {
            return Err(Afwijzing::VerkeerdeAfzender);
        }
    }

    // B-34 en B-14: boven `i64::MAX` slaat SQLite een negatief getal op, en dan zijn de
    // rij en de vergelijking in Rust het niet meer eens over wat er staat.
    if op.seq > MAX_OPSLAG_GETAL {
        return Err(Afwijzing::SeqTeGroot);
    }
    if op.lamport > MAX_OPSLAG_GETAL {
        return Err(Afwijzing::LamportTeGroot);
    }
    if op.lamport > lokaal_max_lamport.saturating_add(MAX_LAMPORT_SPRONG) {
        return Err(Afwijzing::LamportSprong);
    }

    // B-16. `seq` is 1-gebaseerd en dicht; 0 bestaat niet en een gat groter dan het venster
    // wordt nooit meer gedicht, dus die rij zou voor altijd blijven staan zonder mee te
    // tellen.
    if op.seq == 0 || op.seq > frontier.saturating_add(MAX_SEQ_VOORUIT) {
        return Err(Afwijzing::SeqBuitenVenster);
    }

    // B-15.
    if op.payload.len().saturating_add(OP_VASTE_OVERHEAD) > MAX_OP_LEN {
        return Err(Afwijzing::OpTeGroot);
    }

    Ok(())
}

/// Wat een insert opleverde. `Botsing` is de stille datavernietiging van B-07: de sleutel
/// was al bezet door iets ánders dan deze op.
enum Insert {
    Nieuw,
    Duplicaat,
    Botsing,
}

fn insert_op(conn: &Connection, op: &Op) -> Result<Insert> {
    let author_key = op.author.as_bytes().to_vec();
    let channel_key = channel_to_blob(op.channel).to_vec();
    let n = conn.execute(
        "INSERT OR IGNORE INTO ops (author, channel, seq, lamport, wall_clock, kind, payload)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            author_key.clone(),
            channel_key.clone(),
            op.seq as i64,
            op.lamport as i64,
            op.wall_clock,
            op.kind_tag as i64,
            op.payload,
        ],
    )?;
    if n > 0 {
        return Ok(Insert::Nieuw);
    }

    // B-07: `INSERT OR IGNORE` gooide een latere, échte op met dezelfde sleutel
    // stilzwijgend weg — geen foutpad, geen logregel, geen UI-signaal, en
    // `advance_contiguous` schoof daarna over de vervalste rij heen zodat de version vector
    // naar waarheid meldde "ik heb hem", waarna de eigenaar hem nooit meer opnieuw stuurt.
    // Een echt duplicaat is byte-identiek, dus vergelijken kost geen valse meldingen. Deze
    // extra query loopt alleen bij een botsing: een nieuwe op is met de insert al klaar.
    let bestaand: Option<(i64, i64, i64, Vec<u8>)> = conn
        .query_row(
            "SELECT lamport, wall_clock, kind, payload FROM ops
             WHERE author = ?1 AND channel = ?2 AND seq = ?3",
            params![author_key, channel_key, op.seq as i64],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;

    let Some((lamport, wall_clock, kind, payload)) = bestaand else {
        // Niet ingevoegd én niet te vinden: dat kan alleen als de rij tussen de twee
        // queries verdween, en er is geen verwijderpad. Melden in plaats van negeren.
        tracing::error!(op = ?op.id(), "insert genegeerd maar de rij bestaat niet");
        return Ok(Insert::Duplicaat);
    };

    if lamport == op.lamport as i64
        && wall_clock == op.wall_clock
        && kind == op.kind_tag as i64
        && payload == op.payload
    {
        return Ok(Insert::Duplicaat);
    }

    tracing::error!(
        op = ?op.id(),
        bestaande_lamport = lamport,
        nieuwe_lamport = op.lamport,
        bestaande_kind = kind,
        nieuwe_kind = op.kind_tag,
        "sleutelbotsing met afwijkende inhoud: deze op wordt niet opgeslagen (B-07). \
         Iemand heeft deze (auteur, kanaal, seq) eerder met andere inhoud bezet."
    );
    Ok(Insert::Botsing)
}

/// Tot hoe ver de reeks van dit (auteur, kanaal)-paar aaneengesloten is. Vrije functie
/// zodat de invoercontrole hem ook binnen een lopende transactie kan opvragen.
fn contiguous_in(conn: &Connection, author: PeerId, channel: Channel) -> Result<u64> {
    Ok(conn
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

/// Wat deze op op de draad kost, ruim geschat. Gebruikt voor het bytebudget van B-15;
/// `payload` is het enige veld dat in grootte varieert.
fn op_wire_len(op: &Op) -> usize {
    op.payload.len().saturating_add(OP_VASTE_OVERHEAD)
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
///
/// **Altijd de rauwe tag wegschrijven, ook een tag die deze build niet kent (B-08).** De
/// oude vorm schreef alleen tag 1 en 2 en liet al het andere door naar 17 nulbytes — precies
/// de blob van `Channel::GENERAL`. Een op op een onbekend kanaal landde daarmee op de
/// *algemene* sleutel `(auteur, nullen, seq)`, botste daar met een échte algemene op van
/// diezelfde auteur (`INSERT OR IGNORE`, dus stil) en schoof de algemene teller op. Deze
/// functie moet totaal en injectief zijn over álles wat `Channel` kan zijn; dat is wat de
/// primary key `(author, channel, seq)` van de ops-tabel eist.
///
/// De bytes voor tag 0, 1 en 2 zijn onveranderd, dus bestaande databases blijven leesbaar.
fn channel_to_blob(channel: Channel) -> [u8; 17] {
    let mut buf = [0u8; 17];
    buf[0] = channel.raw_tag();
    if let Some(p) = channel.dm_peer() {
        buf[1..].copy_from_slice(p.as_bytes());
    } else if let Some(t) = channel.topic_id() {
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
        0 => Channel::GENERAL,
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
        // Kanaalsoort van een nieuwere peer. Terugvallen op `GENERAL` zou de op alsnog in
        // het algemene kanaal laten opduiken (B-08) én de rondgang blob → Channel → blob
        // niet-injectief maken, waardoor het teruglezen op een andere sleutel uitkomt dan
        // waar hij staat. Zie `Channel::onbekend`.
        tag => Channel::onbekend(tag),
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

/// De uitslag van één Wordle-dag vastleggen. `day` is de `print_date` van het raadsel als
/// `YYYYMMDD`; zie `OpKind::WordleResult` voor waarom dat de sleutel is en niet de dag
/// waarop je speelde.
pub fn wordle_result(day: u32, guesses: u8, solved: bool, pattern: impl Into<String>) -> OpKind {
    OpKind::WordleResult {
        day,
        guesses,
        solved,
        pattern: pattern.into(),
    }
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
