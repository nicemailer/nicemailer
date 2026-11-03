//! Local storage of NiceMailer: SQLite database, paginated queries and
//! mutations.
//!
//! The schema is `specifications/schema.sql`, loaded verbatim at compile
//! time: the specification remains the single source of truth for the
//! schema, and there is no risk of the code and the spec drifting apart.
//!
//! The crate is split into modules (one file each):
//! - `queries.rs`: read-only queries returning API views,
//! - `clean.rs`: simplified display of a thread (quotes, signatures, copies),
//! - `commands.rs`: write commands (move, read/unread, flag),
//! - `raw.rs`: immutable raw message files (`stockage-brut.md`),
//! - `mbox.rs`: streaming reader of mbox files,
//! - `ingest.rs`: interpretation of a raw message (headers, bodies, attachments),
//! - `attachments.rs`: attachments of a thread, extraction of their files,
//! - `html_text.rs`: HTML to plain text, keeping blocks apart,
//! - `import.rs`: import of an mbox file into an archive account,
//! - `library.rs`: history, export, tags, groups, contacts, display preference,
//! - `seed.rs`: generator of fake mail for development.
//!
//! Rust note: `mod queries;` tells the compiler to include `src/queries.rs`
//! as a module. Without `pub`, the module is private to this crate; its
//! `impl Store` blocks still add public methods to `Store`.

pub mod attachments;
mod commands;
pub mod clean;
pub mod html_text;
pub mod import;
pub mod ingest;
pub mod library;
pub mod mbox;
mod queries;
pub mod raw;
pub mod seed;

// `BTreeSet` is a sorted set without duplicates. It is used instead of
// `HashSet` so that iteration order is deterministic (events are emitted in
// a stable order, which makes tests and debugging easier).
use std::collections::BTreeSet;
use std::path::Path;

use rusqlite::{Connection, OpenFlags};

// Re-export rusqlite so that crates depending on nm-store use exactly the
// same version without declaring it themselves.
pub use rusqlite;

/// Schema version expected by this code, stored in SQLite's
/// `PRAGMA user_version` (a free integer in the database header that SQLite
/// reserves for applications).
pub const SCHEMA_VERSION: i64 = 1;

/// Full text of the schema. `include_str!` reads the file at compile time
/// and embeds it in the binary as a `&'static str`; the path is relative to
/// this source file.
const SCHEMA_V1: &str = include_str!("../../../specifications/schema.sql");

/// Errors returned by the store.
///
/// Rust note: Rust has no exceptions. A function that can fail returns a
/// `Result<T, E>`, which is either `Ok(value)` or `Err(error)`. The `?`
/// operator, used everywhere below, means "if this is an error, return it
/// from the current function right away; otherwise unwrap the value".
///
/// `thiserror::Error` generates the boilerplate for a proper error type:
/// - `#[error("...")]` defines the message shown when printing the error;
///   `{0}` refers to the first field, `{found}` to a named field.
/// - `#[from]` generates a conversion from `rusqlite::Error`, so `?` can
///   turn a SQLite error into a `StoreError::Sqlite` automatically.
///
/// The messages stay in French because they reach the user.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("base de données : {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("introuvable : {0}")]
    NotFound(String),
    #[error("déplacement refusé : {0}")]
    InvalidMove(String),
    #[error("paramètre invalide : {0}")]
    InvalidParams(String),
    /// The database was created by a newer version of NiceMailer. Opening
    /// it anyway could corrupt it, so the store refuses.
    #[error("version du schéma {found} plus récente que celle du code ({expected})")]
    SchemaTooNew { found: i64, expected: i64 },
    /// The database was created by an earlier development build whose
    /// version-1 schema lacked tables added since (version 1 is not released
    /// yet, so it is still edited in place). It has to be recreated.
    #[error("base créée par une version de développement antérieure ({0} absent) : recréez-la (make seed)")]
    OutdatedSchema(&'static str),
    /// A file operation failed (raw message storage, mbox file).
    #[error("fichier : {0}")]
    Io(String),
}

/// Shorthand used throughout the crate: `Result<T>` means
/// `Result<T, StoreError>`.
pub type Result<T> = std::result::Result<T, StoreError>;

/// What a command modified, so that the core can emit the matching events.
/// The store knows nothing about events or sockets; it only reports what
/// changed, and the daemon decides what to broadcast.
#[derive(Debug, Default, Clone)]
pub struct ChangeSet {
    /// Modified messages, as `(message_id, thread_id)` pairs.
    pub messages: BTreeSet<(i64, Option<i64>)>,
    /// Modified threads, each with the folders it appeared in before or
    /// after the change.
    pub threads: Vec<(i64, Vec<i64>)>,
    /// Folders whose counters or content changed.
    pub folders: BTreeSet<i64>,
}

impl ChangeSet {
    /// Number of messages modified, returned to the client in
    /// `CommandResult::changed`.
    pub fn changed_messages(&self) -> usize {
        self.messages.len()
    }
}

/// Handle on the database. There is a single `Store` in the core, protected
/// by a mutex (see `nm-daemon`), so only one write happens at a time.
pub struct Store {
    conn: Connection,
}

impl Store {
    /// Opens (or creates) the database at `path` and applies migrations.
    ///
    /// Rust note: `&Path` is a borrowed path: the function reads it but does
    /// not take ownership, so the caller keeps its `PathBuf`.
    pub fn open(path: &Path) -> Result<Store> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )?;
        Self::init(conn)
    }

    /// In-memory database, used by tests: fast and thrown away at the end.
    pub fn open_in_memory() -> Result<Store> {
        Self::init(Connection::open_in_memory()?)
    }

    /// Applies per-connection settings and creates or checks the schema.
    fn init(conn: Connection) -> Result<Store> {
        // These pragmas are not stored in the database file: they must be
        // set on every new connection (see the header of schema.sql).
        // - foreign_keys: SQLite ignores FOREIGN KEY constraints unless asked.
        // - synchronous = NORMAL: with WAL mode, safe and much faster than FULL.
        // - temp_store = MEMORY: temporary tables and indexes stay in RAM.
        // - busy_timeout: wait up to 5 s for a lock instead of failing at once.
        // - cache_size = -65536: keep up to 64 MiB of database pages in
        //   memory (negative means KiB). The default, 2 MiB, is far smaller
        //   than the indexes of a real mailbox, and every miss is a read.
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA synchronous = NORMAL;
             PRAGMA temp_store = MEMORY;
             PRAGMA busy_timeout = 5000;
             PRAGMA cache_size = -65536;",
        )?;
        // `query_row` runs a query expected to return one row and passes it to
        // the closure `|r| r.get(0)`, which reads column 0.
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        match version {
            // Fresh database: create everything. schema.sql ends by setting
            // `user_version` to 1.
            0 => conn.execute_batch(SCHEMA_V1)?,
            // Same version number: check that the schema really is the
            // current one (see `OutdatedSchema`). `prepare` fails if a
            // column or table is missing, without running anything.
            SCHEMA_VERSION => {
                for (sql, what) in [
                    ("SELECT kind, address FROM account", "account.kind"),
                    ("SELECT raw_hash FROM message", "message.raw_hash"),
                    ("SELECT hash FROM raw_blob", "raw_blob"),
                    ("SELECT sync_key FROM contact_group", "contact_group"),
                    ("SELECT date_last_modified FROM thread_folder", "thread_folder.date_last_modified"),
                    ("SELECT body_format FROM address", "address.body_format"),
                ] {
                    if conn.prepare(sql).is_err() {
                        return Err(StoreError::OutdatedSchema(what));
                    }
                }
                // Older builds indexed new messages with a trigger; the code
                // now does it, and the two together would index twice.
                let old_trigger: i64 = conn.query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type = 'trigger' AND name = 'message_fts_ai'",
                    [],
                    |r| r.get(0),
                )?;
                if old_trigger > 0 {
                    return Err(StoreError::OutdatedSchema("trigger message_fts_ai encore présent"));
                }
            }
            // `v if ...` is a *match guard*: the arm matches only if the
            // condition holds.
            v if v > SCHEMA_VERSION => {
                return Err(StoreError::SchemaTooNew { found: v, expected: SCHEMA_VERSION });
            }
            // Future migrations (version 1 -> 2, ...) will go here. For now no
            // version between 0 and 1 can exist; `unreachable!` panics if the
            // impossible happens.
            _ => unreachable!("pas de migration antérieure à la version 1"),
        }
        Ok(Store { conn })
    }

    /// Direct access to the connection, for the CLI and tests.
    pub fn conn(&self) -> &Connection {
        &self.conn
    }
}

/// Recomputes the denormalized aggregates of a thread (tables `thread` and
/// `thread_folder`) and returns the folders where it appears, before or
/// after the change.
///
/// Why denormalize: listing a folder must be fast even with 100 000 mails.
/// Instead of grouping messages on every read, `thread_folder` stores one
/// row per (thread, folder) with its date and counters, and an index on it
/// makes the list query cheap. The price is that every write must keep those
/// rows in sync, which is what this function does. It runs inside the same
/// transaction as the write, so readers never see inconsistent counters.
///
/// The simplest correct approach is used: delete the thread's rows and
/// recompute them from `message`. A thread has few messages, so it is cheap.
///
/// Rust note: `pub(crate)` makes the function visible inside this crate
/// only (to `commands.rs`), not to other crates.
pub(crate) fn refresh_thread(conn: &Connection, thread_id: i64) -> Result<Vec<i64>> {
    let mut folders: BTreeSet<i64> = BTreeSet::new();
    // Folders before the change. The extra `{ }` block limits the lifetime of
    // `stmt`: a prepared statement borrows the connection, and the borrow
    // must end before we run other statements below.
    {
        // `prepare_cached` keeps the compiled statement for reuse, which saves
        // parsing the SQL on every call.
        let mut stmt = conn.prepare_cached("SELECT folder_id FROM thread_folder WHERE thread_id = ?1")?;
        // `query_map` returns an iterator over rows; each item is itself a
        // `Result` because reading a row can fail, hence `f?`.
        // `r.get::<_, i64>(0)` reads column 0 as an `i64`; `_` lets the
        // compiler infer the index type.
        for f in stmt.query_map([thread_id], |r| r.get::<_, i64>(0))? {
            folders.insert(f?);
        }
    }
    conn.execute("DELETE FROM thread_folder WHERE thread_id = ?1", [thread_id])?;
    // `sum(is_seen = 0)` counts unread messages: in SQLite a comparison
    // yields 1 or 0.
    conn.execute(
        "INSERT INTO thread_folder (thread_id, folder_id, date_last_received, date_last_modified, message_count, unread_count)
         SELECT thread_id, folder_id, max(date_received), max(date_modified), count(*), sum(is_seen = 0)
           FROM message WHERE thread_id = ?1 GROUP BY folder_id",
        [thread_id],
    )?;
    conn.execute(
        "UPDATE thread SET
            date_last_received = coalesce((SELECT max(date_received) FROM message WHERE thread_id = ?1), 0),
            message_count      = (SELECT count(*) FROM message WHERE thread_id = ?1),
            unread_count       = (SELECT count(*) FROM message WHERE thread_id = ?1 AND is_seen = 0)
          WHERE id = ?1",
        [thread_id],
    )?;
    // Folders after the change.
    {
        let mut stmt = conn.prepare_cached("SELECT folder_id FROM thread_folder WHERE thread_id = ?1")?;
        for f in stmt.query_map([thread_id], |r| r.get::<_, i64>(0))? {
            folders.insert(f?);
        }
    }
    // Convert the set into a `Vec` (already sorted, without duplicates).
    Ok(folders.into_iter().collect())
}

/// Recomputes the message counters of a folder (`folder.total_count` and
/// `folder.unread_count`), shown in the sidebar.
pub(crate) fn refresh_folder_counts(conn: &Connection, folder_id: i64) -> Result<()> {
    conn.execute(
        "UPDATE folder SET
            total_count  = (SELECT count(*) FROM message WHERE folder_id = ?1),
            unread_count = (SELECT count(*) FROM message WHERE folder_id = ?1 AND is_seen = 0)
          WHERE id = ?1",
        [folder_id],
    )?;
    // `()` is the "unit" type: the function succeeds without returning data.
    Ok(())
}
