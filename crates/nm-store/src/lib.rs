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
//! - `drafts.rs`: replies being written, and their sending (`reponse.md`),
//! - `draft_copies.rs`: their copies in the Drafts folder, local and on the server,
//! - `screening.rs`: decisions on senders, filing of their mails, undo,
//! - `raw.rs`: immutable raw message files (`stockage-brut.md`),
//! - `mbox.rs`: streaming reader of mbox files,
//! - `ingest.rs`: interpretation of a raw message (headers, bodies, attachments),
//! - `encoded_words.rs`: RFC 2047 encoded words that `mail-parser` leaves undecoded,
//! - `attachments.rs`: attachments of a thread, extraction of their files,
//! - `unsubscribe.rs`: how to unsubscribe from a mailing list,
//! - `links.rs`: the links of a message (HTML anchors, addresses of the text),
//! - `phones.rs`: phone numbers written in the text of a message,
//! - `html_text.rs`: HTML to plain text, keeping blocks apart,
//! - `import.rs`: import of an mbox file into an archive account,
//! - `reindex.rs`: reinterpretation of stored messages after a parser change,
//! - `library.rs`: history, export, tags, groups, contacts, display preference,
//! - `seed.rs`: generator of fake mail for development,
//! - `pool.rs`: read-only connections, so queries never wait for a write,
//! - `accounts.rs`: accounts of `accounts.toml` in the database (enable, disable),
//! - `migrations.rs`: upgrade of a database created by an earlier version,
//! - `inspect.rs`: what a database file is (new, current, to upgrade,
//!   unusable), without changing it.
//!
//! Rust note: `mod queries;` tells the compiler to include `src/queries.rs`
//! as a module. Without `pub`, the module is private to this crate; its
//! `impl Store` blocks still add public methods to `Store`.

pub mod accounts;
pub mod auth;
pub mod attachments;
mod commands;
pub mod clean;
pub mod draft_copies;
pub mod drafts;
pub mod css;
pub mod encoded_words;
pub mod gc;
pub mod html_text;
pub mod import;
pub mod inspect;
pub mod insert;
pub mod ingest;
pub mod library;
pub mod links;
pub mod phones;
pub mod mbox;
pub mod outgoing;
mod migrations;
pub mod pool;
mod queries;
pub mod raw;
pub mod reception;
pub mod reindex;
pub mod remote_images;
pub mod screening;
pub mod unsubscribe;
pub mod seed;
pub mod setup;
pub mod spool;
mod sql_log;
pub mod sync;

// `BTreeSet` is a sorted set without duplicates. It is used instead of
// `HashSet` so that iteration order is deterministic (events are emitted in
// a stable order, which makes tests and debugging easier).
use std::collections::BTreeSet;
use std::path::Path;

use nm_core::FolderRole;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};

// Re-export rusqlite so that crates depending on nm-store use exactly the
// same version without declaring it themselves.
pub use rusqlite;

/// Schema version expected by this code, stored in SQLite's
/// `PRAGMA user_version` (a free integer in the database header that SQLite
/// reserves for applications).
///
/// 2: indexes for large mailboxes, `thread.date_last_modified`, repaired
/// encoded words (`migrations.rs`).
/// 3: remote images (`address.remote_images`, `message_body.tracking_pixels`).
/// 4: `account.disabled_at`, accounts whose key left `accounts.toml`, and
///    `account.position`, their display order.
/// 5: `draft.after_send` accepts `stay` (the table is made again).
/// 6: `draft.forward_of_message_id`, the mail a forward carries.
/// 7: `draft.send_account_id`, the account that sends a draft when it is
///    not the draft's own.
/// 8: date indexes of `thread_folder`, for All Mail.
/// 9: the folder of a conversation in `thread` (`folder`, `has_sent`…),
///    `thread_folder` dropped, `idx_message_received` for Mail date….
pub const SCHEMA_VERSION: i64 = 9;

/// Full text of the schema, at the latest version. `include_str!` reads the
/// file at compile time and embeds it in the binary as a `&'static str`; the
/// path is relative to this source file.
const SCHEMA: &str = include_str!("../../../specifications/schema.sql");

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
    /// A command that changes the mail of a read-only account.
    #[error("{0}")]
    ReadOnly(String),
    /// The raw file of a message is corrupt or missing (`gc.rs`): the
    /// caller sets it aside (`Store::raw_damaged`), and the message is
    /// downloaded again if its server still has it. The hash, in hex.
    #[error("the raw file of this mail is damaged ({0}): it is set aside and downloaded again from the server if possible")]
    RawDamaged(String),
    #[error("déplacement refusé : {0}")]
    InvalidMove(String),
    #[error("paramètre invalide : {0}")]
    InvalidParams(String),
    /// A favorable decision on a sender whose mail is suspected of spoofing
    /// needs the user's explicit confirmation
    /// (`authentification-expediteur.md`, section 5).
    #[error("confirmation requise : usurpation probable de {addr} (verdict {verdict})")]
    ConfirmationRequired { addr: String, verdict: String },
    /// The database was created by a newer version of NiceMailer. Opening
    /// it anyway could corrupt it, so the store refuses.
    #[error("version du schéma {found} plus récente que celle du code ({expected})")]
    SchemaTooNew { found: i64, expected: i64 },
    /// A read-only connection found another schema version than the code's:
    /// the writing store must open (and migrate) the database first.
    #[error("connexion de lecture : schéma en version {found}, attendu {expected}")]
    ReaderVersion { found: i64, expected: i64 },
    /// The database was created by an earlier development build whose
    /// version-1 schema lacked tables added since (version 1 is not released
    /// yet, so it is still edited in place). It has to be recreated.
    #[error("base créée par une version de développement antérieure ({0} absent) : recréez-la (make seed)")]
    OutdatedSchema(&'static str),
    /// A file operation failed (raw message storage, mbox file).
    #[error("fichier : {0}")]
    Io(String),
    /// This mail cannot be answered or this draft sent: an archive
    /// account, a draft answered, no recipient (`reponse.md`, section 3).
    #[error("{0}")]
    CannotSend(String),
    /// The body of the mail answered is not downloaded yet.
    #[error("the body of this mail is not downloaded yet: it is asked for, try again in a moment")]
    BodyPending,
    /// `update_draft` or `send_draft` on another revision than the current
    /// one: another window or client changed the draft.
    #[error("this draft was changed elsewhere (revision {current}): reload it")]
    RevisionConflict { current: i64 },
    /// The draft is being sent and cannot be changed.
    #[error("this draft is being sent: cancel the sending first")]
    DraftSending,
    /// `cancel_send` too late.
    #[error("too late to cancel: the mail is leaving, or has left")]
    SendStarted,
}

/// Shorthand used throughout the crate: `Result<T>` means
/// `Result<T, StoreError>`.
pub type Result<T> = std::result::Result<T, StoreError>;

/// The error of reading a raw file: `RawDamaged` when the file is corrupt
/// (`InvalidData`) or missing (`NotFound`), the cases `gc.rs` repairs.
pub(crate) fn raw_error(hash: &raw::Hash, e: std::io::Error) -> StoreError {
    match e.kind() {
        std::io::ErrorKind::InvalidData | std::io::ErrorKind::NotFound => StoreError::RawDamaged(raw::hex(hash)),
        _ => StoreError::Io(e.to_string()),
    }
}

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
    /// Accounts whose settings or state changed (`AccountChanged`).
    pub accounts: BTreeSet<i64>,
    /// New mail found by the synchronization, as `(message_id, folder_id)`
    /// pairs (`MessageArrived`, which the app turns into notifications).
    pub arrived: BTreeSet<(i64, i64)>,
    /// Accounts that got actions in the spool: their worker is woken to
    /// send them (`spool.rs`).
    pub spooled: BTreeSet<i64>,
    /// Local changes undone after the server refused them for good, as
    /// `(message_id, kind, reason)` (`ActionReverted`).
    pub reverted: Vec<(i64, String, String)>,
    /// Accounts with a mail queued for sending, or a send cancelled: their
    /// sender looks at its queue again (`drafts.rs`).
    pub sending: BTreeSet<i64>,
    /// Drafts the server accepted (`DraftSent`).
    pub sent: Vec<drafts::SentDraft>,
    /// Drafts that could not be sent, for good, as `(draft_id, reply_to,
    /// reason)` (`SendFailed`).
    pub send_failed: Vec<(i64, Option<i64>, String)>,
    /// Drafts sent to some recipients only, with the others and the
    /// server's answer for each (`SendPartial`).
    pub send_partial: Vec<(i64, Vec<String>)>,
}

impl ChangeSet {
    /// Number of messages modified, returned to the client in
    /// `CommandResult::changed`.
    pub fn changed_messages(&self) -> usize {
        self.messages.len()
    }
}

/// Handle on the database. There is a single writing `Store` in the core,
/// protected by a mutex (see `nm-daemon`), so only one write happens at a
/// time. Queries run on read-only stores of a [`pool::ReadPool`].
pub struct Store {
    conn: Connection,
    /// IMAP accounts synchronized by the core, whose changes go to the
    /// spool (`spool.rs`, `set_server_accounts`).
    server_accounts: BTreeSet<i64>,
    /// Sending is pretended (`NICEMAILER_FAKE_NETWORK`, `drafts.rs`): the
    /// archive accounts may then answer too.
    fake_sending: bool,
    /// The files of mails to send (`outgoing.rs`), set by the core: a
    /// store without it (the CLI's `seed`) cannot attach files nor send
    /// a mail that has some.
    outgoing: Option<outgoing::OutgoingStore>,
}

/// Runs `analyze` (a `PRAGMA optimize`), then keeps only the averages of the
/// statistics it computed.
///
/// `ANALYZE` writes two tables. `sqlite_stat1` holds averages: how many
/// rows share a value of an index, on average. `sqlite_stat4` holds samples
/// of actual values with their counts, so that the planner can tell a big
/// account or folder from a small one. The core's SQLite is built with
/// `SQLITE_ENABLE_STAT4` (rusqlite's bundled build), so both are written.
///
/// The samples are deleted, for a reason specific to a mail database: the
/// values that matter (accounts, folders, threads) are created all the time,
/// and a value created after the last analysis is absent from the samples,
/// which SQLite reads as "almost no rows". A new account then looks empty
/// while it fills: on a real database, the threading of an mbox import read
/// every message of the account for each id cited (`insert.rs`,
/// `THREADS_CITING`), and the lists of the new account chose other plans
/// than those of the older ones. `PRAGMA optimize` does not analyze again
/// until a table has grown about tenfold, so the samples stayed wrong for
/// the whole import. The averages of `sqlite_stat1` are wrong by a little
/// in such cases, never by orders of magnitude.
///
/// Measured on a copy of a real mailbox (160,000 messages, an archive
/// account created after the last analysis): every list query of the plan
/// tests, every filter, in its folder and its account, took 411 ms in total
/// with the samples and 215 ms without; the slowest went from 31 ms to 7 ms.
///
/// `ANALYZE sqlite_schema` makes this connection load the statistics again:
/// without it, it would keep the samples it read when it opened. The
/// readers of the pool (`pool.rs`) are opened after the writing store, so
/// they read the statistics without the samples.
fn refresh_statistics(conn: &Connection, analyze: &str) -> Result<()> {
    conn.execute_batch(analyze)?;
    // The table exists only once something was analyzed.
    let has_samples: bool =
        conn.query_row("SELECT EXISTS (SELECT 1 FROM sqlite_schema WHERE name = 'sqlite_stat4')", [], |r| r.get(0))?;
    if has_samples {
        conn.execute_batch("DELETE FROM sqlite_stat4; ANALYZE sqlite_schema;")?;
    }
    Ok(())
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

    /// Opens a read-only connection to an existing database, for the read
    /// pool (`pool.rs`). The database must be at the current schema
    /// version: readers never create or migrate it, the writing store does,
    /// before the pool is opened.
    ///
    /// The file is opened read-write but the connection is set to
    /// `query_only`: a read-only *file* handle cannot create the WAL index
    /// (`-shm` file) if it is missing, while `query_only` gives the same
    /// guarantee without that limit. Any write through a reader fails with
    /// an error, so a query that writes by mistake is caught by the tests
    /// instead of racing the writer.
    pub fn open_reader(path: &Path) -> Result<Store> {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        sql_log::install(&conn);
        // Same settings as the writer (see `init`), with a smaller page
        // cache: there are several readers, and the operating system caches
        // the file for all of them anyway.
        conn.execute_batch(
            "PRAGMA query_only = ON;
             PRAGMA foreign_keys = ON;
             PRAGMA temp_store = MEMORY;
             PRAGMA busy_timeout = 5000;
             PRAGMA cache_size = -32768;",
        )?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version != SCHEMA_VERSION {
            return Err(StoreError::ReaderVersion { found: version, expected: SCHEMA_VERSION });
        }
        Ok(Store { conn, server_accounts: BTreeSet::new(), fake_sending: false, outgoing: None })
    }

    /// In-memory database, used by tests: fast and thrown away at the end.
    pub fn open_in_memory() -> Result<Store> {
        Self::init(Connection::open_in_memory()?)
    }

    /// Applies per-connection settings and creates or checks the schema.
    fn init(conn: Connection) -> Result<Store> {
        sql_log::install(&conn);
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
        // Rust note: `mut` because migrating needs `&mut Connection` (a
        // transaction borrows the connection exclusively).
        let mut conn = conn;
        match version {
            // Fresh database: create everything. schema.sql ends by setting
            // `user_version` to the latest version.
            0 => conn.execute_batch(SCHEMA)?,
            // `v if ...` is a *match guard*: the arm matches only if the
            // condition holds.
            v if v > SCHEMA_VERSION => {
                return Err(StoreError::SchemaTooNew { found: v, expected: SCHEMA_VERSION });
            }
            // An earlier version: check it is a complete one, then upgrade it.
            v => {
                check_schema(&conn, v)?;
                migrations::migrate(&mut conn, v)?;
            }
        }
        // Planner statistics (`sqlite_stat1`). Without them SQLite guesses how
        // selective each index is, and on a large mailbox it once preferred
        // an index that read a whole folder for every row of a list. The
        // queries are written to be fast without statistics; these are a
        // safety net. `0x10002` is the setting SQLite recommends when a
        // connection opens: it analyzes only the tables that need it, with a
        // bounded effort, so it stays quick on a large database.
        refresh_statistics(&conn, "PRAGMA optimize = 0x10002;")?;
        // The accounts whose changes go to the spool (`spool.rs`,
        // `set_server_accounts`): a table of this connection only, gone
        // when it closes, filled by the core from `accounts.toml`.
        conn.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS server_account (id INTEGER PRIMARY KEY);
             CREATE TEMP TABLE IF NOT EXISTS read_only_account (id INTEGER PRIMARY KEY);",
        )?;
        Ok(Store { conn, server_accounts: BTreeSet::new(), fake_sending: false, outgoing: None })
    }

    /// Updates the planner statistics after a large change (an import adds
    /// tens of thousands of rows at once). Same as at opening: only the
    /// tables whose size changed much are analyzed.
    pub fn optimize(&self) -> Result<()> {
        refresh_statistics(&self.conn, "PRAGMA optimize;")
    }

    /// Recomputes the aggregates of a thread (`refresh_thread`), in its
    /// own transaction: for tests that change `message` directly, and for
    /// a repair of a thread whose aggregates drifted.
    pub fn refresh_thread_aggregates(&mut self, thread_id: i64) -> Result<()> {
        let tx = self.transaction()?;
        refresh_thread(&tx, thread_id)?;
        tx.commit()?;
        Ok(())
    }

    /// Direct access to the connection, for the CLI and tests.
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Starts a write transaction, for the insertion of messages
    /// (`insert.rs`), which [`insert::Inserter::commit`] commits.
    ///
    /// `Immediate`: the write lock is taken at once, not at the first
    /// write, so the transaction cannot fail halfway through because a
    /// reader upgraded first (`SQLITE_BUSY`).
    pub fn transaction(&mut self) -> Result<rusqlite::Transaction<'_>> {
        Ok(self.conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?)
    }
}

/// Checks that a database at `version` really has that version's schema.
///
/// Version 1 was edited in place during development (it is not released),
/// so a database created by an early build may lack tables added since;
/// `prepare` fails on a missing table or column without running anything.
/// Such a database cannot be migrated and has to be recreated
/// (`OutdatedSchema`).
fn check_schema(conn: &Connection, version: i64) -> Result<()> {
    let mut checks = vec![
        ("SELECT kind, address FROM account", "account.kind"),
        ("SELECT raw_hash FROM message", "message.raw_hash"),
        ("SELECT hash FROM raw_blob", "raw_blob"),
        ("SELECT sync_key FROM contact_group", "contact_group"),
        ("SELECT body_format FROM address", "address.body_format"),
        ("SELECT unsubscribed_at FROM address", "address.unsubscribed_at"),
        ("SELECT one_click FROM message_unsubscribe", "message_unsubscribe"),
    ];
    if version >= 2 {
        checks.push(("SELECT date_last_modified FROM thread", "thread.date_last_modified"));
    }
    // `thread_folder` existed from the first build to version 8.
    if version < 9 {
        checks.push(("SELECT date_last_modified FROM thread_folder", "thread_folder.date_last_modified"));
    } else {
        checks.push(("SELECT folder, has_quarantine FROM thread", "thread.folder"));
    }
    if version >= 3 {
        checks.push(("SELECT remote_images FROM address", "address.remote_images"));
        checks.push(("SELECT tracking_pixels FROM message_body", "message_body.tracking_pixels"));
    }
    if version >= 4 {
        checks.push(("SELECT disabled_at, position FROM account", "account.disabled_at, account.position"));
    }
    if version >= 6 {
        checks.push(("SELECT forward_of_message_id FROM draft", "draft.forward_of_message_id"));
    }
    if version >= 7 {
        checks.push(("SELECT send_account_id FROM draft", "draft.send_account_id"));
    }
    for (sql, what) in checks {
        if conn.prepare(sql).is_err() {
            return Err(StoreError::OutdatedSchema(what));
        }
    }
    // Older builds indexed new messages with a trigger; the code now does
    // it, and the two together would index twice.
    let old_trigger: i64 = conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type = 'trigger' AND name = 'message_fts_ai'",
        [],
        |r| r.get(0),
    )?;
    if old_trigger > 0 {
        return Err(StoreError::OutdatedSchema("trigger message_fts_ai encore présent"));
    }
    Ok(())
}

/// The aggregates of a thread, as stored in `thread` (`schema.sql`,
/// version 9): its folder, its technical flags and its counters.
#[derive(Debug, Clone, PartialEq, Default)]
struct ThreadAggregates {
    /// Code of the role (`FolderRole::code`), `None` without one.
    folder: Option<&'static str>,
    has_sent: bool,
    has_draft: bool,
    has_quarantine: bool,
    has_deleted: bool,
    date_last_received: i64,
    message_count: i64,
    unread_count: i64,
    date_last_modified: i64,
}

/// One mail of a thread, as `refresh_thread` reads it.
struct ThreadMail {
    date_received: i64,
    date_modified: i64,
    seen: bool,
    role: Option<FolderRole>,
}

impl ThreadAggregates {
    /// The aggregates of the mails of a thread, **latest first** (`dossiers.md`,
    /// "Dossier d'une conversation"):
    /// - the folder by the rule of `nm_core::thread_folder`;
    /// - a flag per technical folder holding a mail;
    /// - the "live" counters, without the mails of Deleted, Junk and Banned,
    ///   or of every mail when none is live (a thread wholly in Junk still
    ///   shows its date and its count in Junk);
    /// - the latest change of any mail: deleting one is a change.
    fn of(mails: &[ThreadMail]) -> ThreadAggregates {
        let out_of_all_mail = |r: Option<FolderRole>| r.is_some_and(|r| FolderRole::OUT_OF_ALL_MAIL.contains(&r));
        let any_live = mails.iter().any(|m| !out_of_all_mail(m.role));
        // Rust note: a closure can capture `any_live`; `filter` then keeps
        // the live mails, or all of them when there is none.
        let counted = || mails.iter().filter(move |m| !any_live || !out_of_all_mail(m.role));
        let has = |r: FolderRole| mails.iter().any(|m| m.role == Some(r));
        ThreadAggregates {
            folder: nm_core::thread_folder(mails.iter().map(|m| m.role)).map(FolderRole::code),
            has_sent: has(FolderRole::Sent),
            has_draft: has(FolderRole::Drafts),
            has_quarantine: has(FolderRole::Quarantine),
            has_deleted: has(FolderRole::Deleted),
            date_last_received: counted().map(|m| m.date_received).max().unwrap_or(0),
            message_count: counted().count() as i64,
            unread_count: counted().filter(|m| !m.seen).count() as i64,
            date_last_modified: mails.iter().map(|m| m.date_modified).max().unwrap_or(0),
        }
    }
}

/// Recomputes the denormalized aggregates of a thread (`thread.folder`, its
/// flags and counters) and returns the folders where it appears, before or
/// after the change, for the events (`ThreadChanged`).
///
/// Why denormalize: listing a folder must be fast even with 100 000 mails.
/// Instead of grouping messages on every read, `thread` stores the folder,
/// date and counters of each conversation, and its indexes make every list
/// a walk that stops after a page (`schema.sql`, version 9). The price is
/// that every write must keep them in sync, which is what this function
/// does. It runs inside the same transaction as the write, so readers never
/// see inconsistent counters.
///
/// The mails of the thread are read again (a thread has few), the
/// aggregates computed in Rust, and only the columns that changed are
/// written: SQLite rewrites an index only when one of its columns is in
/// the `SET`, and `thread` has 28 of them. Marking a mail read changes
/// `unread_count` alone, which no index holds.
///
/// "Before": the folders of the account with the roles the thread had
/// (its folder and flags), since the mails have already moved when this
/// runs. "After": the folders of its mails.
///
/// Rust note: `pub(crate)` makes the function visible inside this crate
/// only (to `commands.rs`…), not to other crates.
pub(crate) fn refresh_thread(conn: &Connection, thread_id: i64) -> Result<Vec<i64>> {
    // The stored row, `None` if the thread is gone (merged into another).
    let old: Option<(i64, ThreadAggregates)> = conn
        .prepare_cached(
            "SELECT account_id, folder, has_sent, has_draft, has_quarantine, has_deleted,
                    date_last_received, message_count, unread_count, date_last_modified
               FROM thread WHERE id = ?1",
        )?
        .query_row([thread_id], |r| {
            let code: Option<String> = r.get(1)?;
            Ok((
                r.get(0)?,
                ThreadAggregates {
                    folder: code.as_deref().and_then(FolderRole::from_code).map(FolderRole::code),
                    has_sent: r.get(2)?,
                    has_draft: r.get(3)?,
                    has_quarantine: r.get(4)?,
                    has_deleted: r.get(5)?,
                    date_last_received: r.get(6)?,
                    message_count: r.get(7)?,
                    unread_count: r.get(8)?,
                    date_last_modified: r.get(9)?,
                },
            ))
        })
        .optional()?;
    let Some((account_id, old)) = old else { return Ok(vec![]) };

    // The mails, latest first: the order the folder rule reads them in.
    // `idx_message_thread (thread_id, …)` finds them; the sort is of a few rows.
    let mut folders: BTreeSet<i64> = BTreeSet::new();
    let mails: Vec<ThreadMail> = {
        let mut stmt = conn.prepare_cached(
            "SELECT m.date_received, m.date_modified, m.is_seen, f.role, m.folder_id
               FROM message m JOIN folder f ON f.id = m.folder_id
              WHERE m.thread_id = ?1
              ORDER BY m.date_received DESC, m.id DESC",
        )?;
        let rows = stmt.query_map([thread_id], |r| {
            let role: Option<String> = r.get(3)?;
            Ok((
                ThreadMail {
                    date_received: r.get(0)?,
                    date_modified: r.get(1)?,
                    seen: r.get(2)?,
                    role: role.as_deref().and_then(FolderRole::parse),
                },
                r.get::<_, i64>(4)?,
            ))
        })?;
        let mut mails = Vec::new();
        for row in rows {
            let (mail, folder) = row?;
            folders.insert(folder);
            mails.push(mail);
        }
        mails
    };
    let new = ThreadAggregates::of(&mails);

    // Folders before the change: those of the roles the thread had.
    let mut old_roles: Vec<&str> = Vec::new();
    if let Some(code) = old.folder.and_then(FolderRole::from_code) {
        old_roles.push(code.as_str());
    }
    for (flag, role) in [(old.has_sent, FolderRole::Sent), (old.has_draft, FolderRole::Drafts),
        (old.has_quarantine, FolderRole::Quarantine), (old.has_deleted, FolderRole::Deleted)]
    {
        if flag {
            old_roles.push(role.as_str());
        }
    }
    if old != new {
        let mut stmt = conn.prepare_cached("SELECT id FROM folder WHERE account_id = ?1 AND role = ?2")?;
        for role in old_roles {
            if let Some(id) = stmt.query_row(params![account_id, role], |r| r.get::<_, i64>(0)).optional()? {
                folders.insert(id);
            }
        }
        write_changed(conn, thread_id, &old, &new)?;
    }
    Ok(folders.into_iter().collect())
}

/// Writes the columns of `new` that differ from `old` (see
/// `refresh_thread`). The statement depends on which columns changed;
/// there are few combinations, each prepared once (`prepare_cached`).
fn write_changed(conn: &Connection, thread_id: i64, old: &ThreadAggregates, new: &ThreadAggregates) -> Result<()> {
    use rusqlite::types::Value;
    let mut sets: Vec<&str> = Vec::new();
    let mut values: Vec<Value> = vec![Value::Integer(thread_id)];
    // Rust note: a macro_rules! macro avoids repeating the same three lines
    // for each column: `$col` is a field name, `$sql` its SQL assignment.
    macro_rules! column {
        ($col:ident, $sql:literal, $value:expr) => {
            if old.$col != new.$col {
                sets.push($sql);
                values.push($value);
            }
        };
    }
    column!(folder, "folder = ?", new.folder.map(|c| Value::Text(c.to_string())).unwrap_or(Value::Null));
    column!(has_sent, "has_sent = ?", Value::Integer(new.has_sent.into()));
    column!(has_draft, "has_draft = ?", Value::Integer(new.has_draft.into()));
    column!(has_quarantine, "has_quarantine = ?", Value::Integer(new.has_quarantine.into()));
    column!(has_deleted, "has_deleted = ?", Value::Integer(new.has_deleted.into()));
    column!(date_last_received, "date_last_received = ?", Value::Integer(new.date_last_received));
    column!(message_count, "message_count = ?", Value::Integer(new.message_count));
    column!(unread_count, "unread_count = ?", Value::Integer(new.unread_count));
    column!(date_last_modified, "date_last_modified = ?", Value::Integer(new.date_last_modified));
    // Numbered parameters: ?1 is the id, then one per column, in order.
    let sets: Vec<String> = sets.iter().enumerate().map(|(i, s)| s.replace('?', &format!("?{}", i + 2))).collect();
    let sql = format!("UPDATE thread SET {} WHERE id = ?1", sets.join(", "));
    conn.prepare_cached(&sql)?.execute(rusqlite::params_from_iter(values))?;
    Ok(())
}

/// The aggregates of **every** thread in one set-based statement, the same
/// rule as `refresh_thread` written in SQL: for the migration to version 9
/// (`migrations.rs`) and the fake mailbox (`seed.rs`), where computing
/// thread by thread would take minutes. The test
/// `refresh_all_threads_matches_refresh_thread` (`tests/insert.rs`) checks
/// that both give the same rows.
///
/// - The latest mail of a class: `max(date_received * 1e9 + id)` with the
///   role as a "bare column", which SQLite takes from the row holding the
///   maximum (documented behaviour of `min()` and `max()` in an aggregate).
///   Message ids stay far below 10⁹, and dates below 2³¹: no overflow.
/// - The codes come from `FolderRole::code` (a `CASE` built from the enum),
///   so the SQL and the Rust rule cannot disagree on a letter.
/// - A thread without mail is left as it is (it is about to be deleted).
pub(crate) fn refresh_all_threads_sql() -> String {
    let code = FolderRole::ALL
        .iter()
        .map(|r| format!("WHEN '{}' THEN '{}'", r.as_str(), r.code()))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "WITH m AS (
             SELECT m.thread_id AS t, m.date_received * 1000000000 + m.id AS k, m.date_received AS dr,
                    m.date_modified AS dm, m.is_seen AS seen, f.role AS role,
                    coalesce(f.role NOT IN ('deleted', 'junk', 'banned'), 1) AS live
               FROM message m JOIN folder f ON f.id = m.folder_id
              WHERE m.thread_id IS NOT NULL
         ),
         live_folder AS (
             SELECT t, role, max(k) FROM m
              WHERE role IN ('inbox', 'todo', 'waiting', 'archive', 'newsletter', 'paperwork') GROUP BY t
         ),
         rejected_folder AS (
             SELECT t, role, max(k) FROM m WHERE role IN ('junk', 'banned') GROUP BY t
         ),
         agg AS (
             SELECT t, max(live) AS any_live, max(dm) AS dm,
                    max(role IS 'sent') AS hs, max(role IS 'drafts') AS hr,
                    max(role IS 'quarantine') AS hq, max(role IS 'deleted') AS hd,
                    max(CASE WHEN live THEN dr END) AS dr_live, max(dr) AS dr_all,
                    sum(live) AS n_live, count(*) AS n_all,
                    sum(live AND seen = 0) AS u_live, sum(seen = 0) AS u_all
               FROM m GROUP BY t
         )
         UPDATE thread SET
             folder = (CASE coalesce(lf.role, rf.role,
                                     CASE WHEN a.hs THEN 'sent' WHEN a.hr THEN 'drafts'
                                          WHEN a.hq THEN 'quarantine' WHEN a.hd THEN 'deleted' END)
                       {code} END),
             has_sent = a.hs, has_draft = a.hr, has_quarantine = a.hq, has_deleted = a.hd,
             date_last_received = CASE WHEN a.any_live THEN a.dr_live ELSE a.dr_all END,
             message_count = CASE WHEN a.any_live THEN a.n_live ELSE a.n_all END,
             unread_count = CASE WHEN a.any_live THEN a.u_live ELSE a.u_all END,
             date_last_modified = a.dm
           FROM agg a
           LEFT JOIN live_folder lf ON lf.t = a.t
           LEFT JOIN rejected_folder rf ON rf.t = a.t
          WHERE thread.id = a.t"
    )
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Every column `refresh_thread` maintains, for every thread.
    /// (id, folder, four flags, received, count, unread, modified).
    type Row = (i64, Option<String>, i64, i64, i64, i64, i64, i64, i64, i64);

    fn aggregates(store: &Store) -> Vec<Row> {
        store
            .conn()
            .prepare(
                "SELECT id, folder, has_sent, has_draft, has_quarantine, has_deleted,
                        date_last_received, message_count, unread_count, date_last_modified
                   FROM thread ORDER BY id",
            )
            .unwrap()
            .query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?, r.get(9)?))
            })
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    }

    /// The set-based SQL of the migration and the seed gives exactly what
    /// `refresh_thread` gives, thread by thread, on a seeded mailbox made
    /// to hold every case of the rule: mails of every folder, a rejected
    /// mail in a live thread, technical folders only, a folder without a
    /// role, ties of dates.
    #[test]
    fn refresh_all_threads_matches_refresh_thread() {
        let mut store = Store::open_in_memory().unwrap();
        crate::seed::seed(&mut store, &crate::seed::SeedOptions { threads: 600, seed: 3, now: 1_750_000_000 }).unwrap();
        // Shuffle mails between folders, deterministically: every mail whose
        // id is a multiple of 3, 5, 7 or 11 goes to another folder (a folder
        // without a role among them), some with the same date as another.
        store
            .conn()
            .execute_batch(
                "INSERT INTO folder (account_id, path, created_by, modified_by)
                     SELECT id, 'Old', 'cli', 'cli' FROM account LIMIT 1;
                 UPDATE message SET uid = NULL;
                 UPDATE message SET folder_id = (SELECT id FROM folder WHERE role = 'junk') WHERE id % 3 = 0;
                 UPDATE message SET folder_id = (SELECT id FROM folder WHERE role = 'quarantine') WHERE id % 5 = 0;
                 UPDATE message SET folder_id = (SELECT id FROM folder WHERE role = 'deleted') WHERE id % 7 = 0;
                 UPDATE message SET folder_id = (SELECT id FROM folder WHERE path = 'Old') WHERE id % 11 = 0;
                 UPDATE message SET date_received = 1700000000 WHERE id % 13 = 0;",
            )
            .unwrap();
        store.conn().execute_batch(&refresh_all_threads_sql()).unwrap();
        let by_sql = aggregates(&store);
        let threads: Vec<i64> = by_sql.iter().map(|t| t.0).collect();
        for t in threads {
            store.refresh_thread_aggregates(t).unwrap();
        }
        let by_rust = aggregates(&store);
        assert_eq!(by_sql, by_rust);
        // Every case is there.
        let has = |code: Option<&str>| by_rust.iter().any(|t| t.1.as_deref() == code);
        for code in [Some("a"), Some("i"), Some("x"), Some("s"), Some("q"), Some("d")] {
            assert!(has(code), "no thread in {code:?}");
        }
    }
}
