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
//! - `sender_kind.rs`: people and robots, the clues of a message and the verdict on an address (`expediteurs.md`),
//! - `senders.rs`: the database side of people and robots: counts, verdicts, the catch-up of stored mail,
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
pub mod ai_runs;
pub mod auth;
pub mod attachments;
pub mod attachment_text;
mod commands;
pub mod clean;
pub mod clean_close;
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
pub mod link_key;
pub mod links;
pub mod mail_html;
pub mod phones;
pub mod codes;
pub mod mbox;
pub mod outgoing;
pub mod paragraphs;
mod migrations;
pub mod pool;
mod queries;
pub mod raw;
pub mod reception;
pub mod record;
pub mod reindex;
pub mod remote_images;
pub mod screening;
pub mod unsubscribe;
pub mod upgrade;
pub mod seed;
pub mod senders;
pub mod sender_kind;
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
/// 10: the bodies and the search index in a second file,
///     `nicemailer_body.db` (`schema_body.sql`).
/// 11: the partial indexes of the virtual folder Work.
/// 12: people and robots (`expediteurs.md`): the clues and the verdict of
///     each message, the verdict and the counts of each address.
/// 13: the AI assistance (`ia.md`, section 9): `ai_run`, the history of
///     the runs of prompts, and `ai_suggestion`, the targets suggested.
/// 14: `idx_message_reply`, for the "answered" attribute.
/// 15: `message.link_key`, the key of the link to a mail, the same on
///     every Mac (`link_key.rs`).
/// 16: the pace of the logins learnt from each server (`account.login_*`,
///     `docs/sync.md`, "Pace of the connections").
pub const SCHEMA_VERSION: i64 = 16;

/// Full text of the schema, at the latest version. `include_str!` reads the
/// file at compile time and embeds it in the binary as a `&'static str`; the
/// path is relative to this source file.
const SCHEMA: &str = include_str!("../../../specifications/schema.sql");

/// Schema of the second file, `nicemailer_body.db`: the bodies and the
/// full-text index (`specifications/schema_body.sql`, schema version 10).
const BODY_SCHEMA: &str = include_str!("../../../specifications/schema_body.sql");

/// The name under which the body file is attached to every connection
/// (`ATTACH DATABASE … AS body`). Queries name `message_body` and
/// `message_fts` without it: SQLite looks a table up in `main`, then in the
/// attached files, and `main` has no such table since version 10.
pub const BODY_ALIAS: &str = "body";

/// The triggers that tie the two files together (`schema_body.sql`): a
/// trigger stored in a file may only touch the tables of that file, so
/// these are created `TEMP`, on each connection that writes, when it
/// opens. A message's subject or sender changed: its row of the search
/// index follows. A message deleted: its body and its row of the search
/// index go (what `ON DELETE CASCADE` did within one file).
const TEMP_TRIGGERS: &str = "
    CREATE TEMP TRIGGER IF NOT EXISTS message_fts_au AFTER UPDATE OF subject, from_name, from_addr ON main.message BEGIN
        UPDATE message_fts
           SET subject = new.subject,
               sender  = coalesce(new.from_name,'') || ' ' || coalesce(new.from_addr,'')
         WHERE rowid = new.id;
    END;
    CREATE TEMP TRIGGER IF NOT EXISTS message_body_ad AFTER DELETE ON main.message BEGIN
        DELETE FROM message_fts WHERE rowid = old.id;
        DELETE FROM message_body WHERE message_id = old.id;
    END;";

/// The body file next to the database at `path`: `nicemailer.db` gives
/// `nicemailer_body.db` (`<stem>_body.db` for any other name, as in tests).
pub fn body_path(path: &Path) -> std::path::PathBuf {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "nicemailer".into());
    path.with_file_name(format!("{stem}_body.db"))
}

/// Attaches the body file (`None`: an in-memory one, for tests) to `conn`.
/// Rust note: `Option<&Path>` is a path that may be absent.
fn attach_body(conn: &Connection, body: Option<&Path>) -> Result<()> {
    let target = body.map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| ":memory:".into());
    conn.execute(&format!("ATTACH DATABASE ?1 AS {BODY_ALIAS}"), [target])?;
    Ok(())
}

/// Creates the tables of the body file if it is new (its `user_version` is
/// still 0), and sets it in WAL mode, like the main file.
fn create_body_schema(conn: &Connection) -> Result<()> {
    let version: i64 = conn.query_row(&format!("PRAGMA {BODY_ALIAS}.user_version"), [], |r| r.get(0))?;
    if version == 0 {
        // The statements of the file are written for a database of their
        // own: run them with `body` as the schema of every name, by
        // prefixing the `CREATE` statements. The pragmas are set apart.
        conn.execute_batch(&format!("PRAGMA {BODY_ALIAS}.journal_mode = WAL;"))?;
        let ddl = BODY_SCHEMA
            .replace("PRAGMA journal_mode = WAL;", "")
            .replace("PRAGMA user_version = 1;", "")
            .replace("CREATE TABLE message_body", &format!("CREATE TABLE {BODY_ALIAS}.message_body"))
            .replace("CREATE INDEX idx_message_body_parser", &format!("CREATE INDEX {BODY_ALIAS}.idx_message_body_parser"))
            .replace("CREATE VIRTUAL TABLE message_fts", &format!("CREATE VIRTUAL TABLE {BODY_ALIAS}.message_fts"))
            .replace("CREATE TRIGGER message_body_fts_au", &format!("CREATE TRIGGER {BODY_ALIAS}.message_body_fts_au"));
        conn.execute_batch(&format!("BEGIN; {ddl} PRAGMA {BODY_ALIAS}.user_version = 1; COMMIT;"))?;
    }
    Ok(())
}

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
/// The messages are in English, for the logs and the CLI. An error a user
/// may read carries a reason (`nm_api::Why`, `localisation.md`, section
/// 3.1), which each client writes in the user's language; `NotFound` and
/// `InvalidParams` are mistakes of a client's program, whose text is a
/// detail for the logs.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("not found: {0}")]
    NotFound(String),
    /// A command that changes the mail of a read-only account.
    #[error("{}", nm_api::reason::ACCOUNT_READ_ONLY.english)]
    ReadOnly,
    /// The raw file of a message is corrupt or missing (`gc.rs`): the
    /// caller sets it aside (`Store::raw_damaged`), and the message is
    /// downloaded again if its server still has it. The hash, in hex.
    #[error("the raw file of this mail is damaged ({0}): it is set aside and downloaded again from the server if possible")]
    RawDamaged(String),
    /// A move the workflow rules refuse.
    #[error("{0}")]
    InvalidMove(nm_api::Why),
    /// A mistake of the client's program: the text says which.
    #[error("invalid parameters: {0}")]
    InvalidParams(String),
    /// A value the user gave that cannot be used (a tag name too long, a
    /// name taken): its reason.
    #[error("{0}")]
    Invalid(nm_api::Why),
    /// A favorable decision on a sender whose mail is suspected of spoofing
    /// needs the user's explicit confirmation
    /// (`authentification-expediteur.md`, section 5).
    #[error("confirmation required: probable spoofing of {addr} (verdict {verdict})")]
    ConfirmationRequired { addr: String, verdict: String },
    /// The database was created by a newer version of NiceMailer. Opening
    /// it anyway could corrupt it, so the store refuses.
    #[error("schema version {found} newer than the code's ({expected})")]
    SchemaTooNew { found: i64, expected: i64 },
    /// A read-only connection found another schema version than the code's:
    /// the writing store must open (and migrate) the database first.
    #[error("read connection: schema version {found}, expected {expected}")]
    ReaderVersion { found: i64, expected: i64 },
    /// The database was created by an earlier development build whose
    /// version-1 schema lacked tables added since (version 1 is not released
    /// yet, so it is still edited in place). It has to be recreated.
    #[error("database created by an earlier development build ({0} missing): create it again (make seed)")]
    OutdatedSchema(&'static str),
    /// A file operation failed (raw message storage, mbox file).
    #[error("file: {0}")]
    Io(String),
    /// This mail cannot be answered or this draft sent: an archive
    /// account, a draft answered, no recipient (`reponse.md`, section 3).
    #[error("{0}")]
    CannotSend(nm_api::Why),
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
    /// Those of `folders` whose every conversation that changed is in
    /// `threads` (`announce_threads`): a client that shows such a folder
    /// may reload these conversations alone instead of the whole list
    /// (`FolderChanged.threads_announced`, `evenements.md`). Empty by
    /// default, which is always right: a folder not announced is reloaded
    /// whole, as before. A change that touches a folder without saying
    /// which conversations (an import, a first synchronization) never
    /// announces it.
    pub announced: BTreeSet<i64>,
    /// Accounts whose settings or state changed (`AccountChanged`).
    pub accounts: BTreeSet<i64>,
    /// New mail found by the synchronization, as `(message_id, folder_id)`
    /// pairs (`MessageArrived`, which the app turns into notifications).
    pub arrived: BTreeSet<(i64, i64)>,
    /// Accounts that got actions in the spool: their worker is woken to
    /// send them (`spool.rs`).
    pub spooled: BTreeSet<i64>,
    /// Local changes undone after the server refused them for good, as
    /// `(message_id, kind, why)` (`ActionReverted`).
    pub reverted: Vec<(i64, String, nm_api::Why)>,
    /// Accounts with a mail queued for sending, or a send cancelled: their
    /// sender looks at its queue again (`drafts.rs`).
    pub sending: BTreeSet<i64>,
    /// Drafts the server accepted (`DraftSent`).
    pub sent: Vec<drafts::SentDraft>,
    /// Drafts that could not be sent, for good, as `(draft_id, reply_to,
    /// why)` (`SendFailed`).
    pub send_failed: Vec<(i64, Option<i64>, nm_api::Why)>,
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

    /// Says that every conversation that changed in `folders` is in
    /// `threads`. Called only where that was checked: a change of flags,
    /// a move of known conversations.
    pub fn announce_threads(&mut self) {
        self.announced = self.folders.clone();
    }

    /// The folders announced once `other` is added (`announced`): a folder
    /// stays announced only if every change that touched it announced it.
    /// An empty change set is neutral, so a batch built by merging into
    /// `ChangeSet::default()` keeps what its parts said.
    pub fn merged_announced(&self, other: &ChangeSet) -> BTreeSet<i64> {
        self.folders
            .union(&other.folders)
            .copied()
            .filter(|f| {
                (!self.folders.contains(f) || self.announced.contains(f))
                    && (!other.folders.contains(f) || other.announced.contains(f))
            })
            .collect()
    }

    /// Adds the changes of `other`: a command on several threads, run in
    /// one transaction, reports them in one `ChangeSet` (one batch of
    /// events). A thread already listed keeps its first entry, with the
    /// folders of both.
    pub fn merge(&mut self, other: ChangeSet) {
        // Before `other.threads` is moved out below (Rust note: a value
        // partly moved can no longer be borrowed whole).
        self.announced = self.merged_announced(&other);
        self.messages.extend(other.messages);
        for (thread, folders) in other.threads {
            match self.threads.iter_mut().find(|(t, _)| *t == thread) {
                Some((_, mine)) => {
                    for f in folders {
                        if !mine.contains(&f) {
                            mine.push(f);
                        }
                    }
                }
                None => self.threads.push((thread, folders)),
            }
        }
        self.folders.extend(other.folders);
        self.accounts.extend(other.accounts);
        self.arrived.extend(other.arrived);
        self.spooled.extend(other.spooled);
        self.reverted.extend(other.reverted);
        self.sending.extend(other.sending);
        self.sent.extend(other.sent);
        self.send_failed.extend(other.send_failed);
        self.send_partial.extend(other.send_partial);
    }
}

/// What [`Store::close_cleanly`] did: whether it marked the files, and
/// how long the checkpoint of each took (the core writes it in its log).
#[derive(Debug, Clone, Default)]
pub struct CloseReport {
    pub marked: bool,
    pub main: std::time::Duration,
    pub body: std::time::Duration,
}

/// The steps of [`Store::open_timed`], each with its duration, in order.
#[derive(Debug, Clone)]
pub struct OpeningSteps {
    last: std::time::Instant,
    pub steps: Vec<(&'static str, std::time::Duration)>,
}

impl OpeningSteps {
    fn new() -> OpeningSteps {
        OpeningSteps { last: std::time::Instant::now(), steps: Vec::new() }
    }

    /// Ends the step `name`, which began at the previous mark.
    fn mark(&mut self, name: &'static str) {
        let now = std::time::Instant::now();
        self.steps.push((name, now - self.last));
        self.last = now;
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
    /// The bodies and the search index live in a second file next to it,
    /// `nicemailer_body.db` (`body_path`), created if missing: a mailbox
    /// whose body file was deleted gets an empty one, and the
    /// reinterpretation fills it again from the raw files (`reindex.rs`).
    pub fn open(path: &Path) -> Result<Store> {
        Ok(Self::open_timed(path)?.0)
    }

    /// [`Store::open`], and how long each step of the opening took, in
    /// order: the core writes them in its log at start (`startup: …`), so
    /// that a slow start can be read where it happens rather than guessed.
    pub fn open_timed(path: &Path) -> Result<(Store, OpeningSteps)> {
        Self::open_migrating(path, &|_| {})
    }

    /// [`Store::open_timed`], calling `on_version` with each schema version
    /// reached while it migrates the database (`migrations::migrate`): the
    /// progress the core announces during a migration (`mise-a-jour.md`,
    /// section 4).
    pub fn open_migrating(path: &Path, on_version: &dyn Fn(i64)) -> Result<(Store, OpeningSteps)> {
        let mut steps = OpeningSteps::new();
        // Before anything opens the files: whether they are as the last
        // clean close left them (`clean_close`). The mark goes either way.
        let clean = clean_close::take(path, &body_path(path));
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )?;
        steps.mark("open");
        let store = Self::init(conn, Some(&body_path(path)), clean, &mut steps, on_version)?;
        Ok((store, steps))
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
        // The body file, created by the writing store before the pool.
        attach_body(&conn, Some(&body_path(path)))?;
        Ok(Store { conn, server_accounts: BTreeSet::new(), fake_sending: false, outgoing: None })
    }

    /// In-memory database, used by tests: fast and thrown away at the end.
    /// Its body file is in memory too.
    pub fn open_in_memory() -> Result<Store> {
        Self::init(Connection::open_in_memory()?, None, false, &mut OpeningSteps::new(), &|_| {})
    }

    /// Applies per-connection settings and creates or checks the schema.
    /// `clean`: the files are as a clean close left them
    /// (`clean_close::take`), so the reconciliation of the two is skipped.
    fn init(
        conn: Connection,
        body: Option<&Path>,
        clean: bool,
        steps: &mut OpeningSteps,
        on_version: &dyn Fn(i64),
    ) -> Result<Store> {
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
        steps.mark("version");
        // The body file, attached before the migrations: version 10 moves
        // the bodies into it. A new or deleted file gets its tables here.
        attach_body(&conn, body)?;
        create_body_schema(&conn)?;
        steps.mark("body file");
        // Rust note: `mut` because migrating needs `&mut Connection` (a
        // transaction borrows the connection exclusively).
        let mut conn = conn;
        match version {
            // Fresh database: create everything. schema.sql ends by setting
            // `user_version` to the latest version.
            // A body file already there belongs to a database that is gone
            // (its main file deleted, `seed --reset`): the ids start again at
            // 1, so its rows would answer for other mails. Emptied.
            0 => {
                conn.execute_batch(SCHEMA)?;
                conn.execute_batch("DELETE FROM body.message_body; DELETE FROM body.message_fts;")?;
            }
            // `v if ...` is a *match guard*: the arm matches only if the
            // condition holds.
            v if v > SCHEMA_VERSION => {
                return Err(StoreError::SchemaTooNew { found: v, expected: SCHEMA_VERSION });
            }
            // An earlier version: check it is a complete one, then upgrade it.
            v => {
                check_schema(&conn, v)?;
                migrations::migrate(&mut conn, v, on_version)?;
                // The bodies just left the main file (version 10): give its
                // pages back to the disk, once. `VACUUM` rewrites only the
                // pages in use (about a tenth of the file on a real
                // mailbox), outside any transaction.
                if v < 10 {
                    conn.execute_batch("VACUUM main;")?;
                }
            }
        }
        steps.mark("schema");
        conn.execute_batch(TEMP_TRIGGERS)?;
        // The two files are not committed together (`schema_body.sql`):
        // make them agree after a crash, or after the body file was deleted.
        // Not after a clean close, which left them in step: on a real
        // mailbox, this is most of the opening (0.9 s, the body file cold).
        if clean {
            steps.mark("clean close, no reconcile");
        } else {
            reconcile_bodies(&conn)?;
            steps.mark("reconcile");
        }
        // Planner statistics (`sqlite_stat1`). Without them SQLite guesses how
        // selective each index is, and on a large mailbox it once preferred
        // an index that read a whole folder for every row of a list. The
        // queries are written to be fast without statistics; these are a
        // safety net. `0x10002` is the setting SQLite recommends when a
        // connection opens: it analyzes only the tables that need it, with a
        // bounded effort, so it stays quick on a large database.
        refresh_statistics(&conn, "PRAGMA optimize = 0x10002;")?;
        steps.mark("statistics");
        // The accounts whose changes go to the spool (`spool.rs`,
        // `set_server_accounts`): a table of this connection only, gone
        // when it closes, filled by the core from `accounts.toml`.
        conn.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS server_account (id INTEGER PRIMARY KEY);
             CREATE TEMP TABLE IF NOT EXISTS read_only_account (id INTEGER PRIMARY KEY);",
        )?;
        Ok(Store { conn, server_accounts: BTreeSet::new(), fake_sending: false, outgoing: None })
    }

    /// The clean close of the core (`clean_close`): empties both
    /// write-ahead logs into their files, then marks them, so that the next
    /// opening may skip the reconciliation. Returns whether the mark was
    /// written: not when a log could not be emptied (a reader still in a
    /// read), nor for a database in memory. The caller holds the store
    /// lock, so that nothing writes between the checkpoint and the mark; a
    /// write after it changes the files, and the next opening reconciles.
    pub fn close_cleanly(&self) -> Result<CloseReport> {
        let mut report = CloseReport::default();
        // A checkpoint that empties a log waits until no reader is in a
        // read: with the usual timeout (5 s), once per file, a stop took
        // 11.7 s, and launchd kills a core that does not stop within 20 s.
        // A reader still busy after 200 ms: no mark, the next start
        // reconciles. Restored after, for the tests that go on.
        self.conn.execute_batch("PRAGMA busy_timeout = 200;")?;
        // `wal_checkpoint(TRUNCATE)` answers (busy, log pages, pages
        // checkpointed); busy = 1 when a reader kept it from finishing.
        let checkpoint = |schema: &str| -> Result<(bool, std::time::Duration)> {
            let started = std::time::Instant::now();
            let busy = self.conn.query_row(&format!("PRAGMA {schema}.wal_checkpoint(TRUNCATE)"), [], |r| r.get::<_, i64>(0))? != 0;
            Ok((busy, started.elapsed()))
        };
        let result = (|| -> Result<()> {
            let (busy, took) = checkpoint("main")?;
            report.main = took;
            if busy {
                return Ok(());
            }
            let (busy, took) = checkpoint("body")?;
            report.body = took;
            if busy {
                return Ok(());
            }
            if let Some(path) = self.conn.path().filter(|p| !p.is_empty()).map(Path::new) {
                report.marked = clean_close::write(path, &body_path(path));
            }
            Ok(())
        })();
        self.conn.execute_batch("PRAGMA busy_timeout = 5000;")?;
        result.map(|()| report)
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

/// Makes the body file agree with the main one (`schema_body.sql`), after a
/// crash between the commits of the two files, or a body file deleted to be
/// rebuilt (`docs/backup.md`):
/// - bodies and rows of the search index of messages that no longer exist
///   are deleted;
/// - a message without a row in the search index gets one, from what the
///   main file knows (subject, sender, recipients) and its body if there
///   is one; a missing body is the reinterpretation's to rebuild from the
///   raw file (`reindex.rs`, `WITHOUT_BODY`), which then fills that row.
///
/// Only indexes and small tables are read, never the text: the ids of the
/// bodies come from `idx_message_body_parser`, those of the search index
/// from its table of sizes (`message_fts_docsize`, one short row per
/// message), not from the 2 GB of its copy of the text. Returns the rows
/// deleted and the rows added.
fn reconcile_bodies(conn: &Connection) -> Result<(usize, usize)> {
    let mut deleted = conn.execute(
        "DELETE FROM message_body WHERE message_id IN (
             SELECT message_id FROM message_body INDEXED BY idx_message_body_parser
              WHERE parser_version > -1 AND message_id NOT IN (SELECT id FROM main.message))",
        [],
    )?;
    let orphans: Vec<i64> = conn
        .prepare("SELECT id FROM message_fts_docsize WHERE id NOT IN (SELECT id FROM main.message)")?
        .query_map([], |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    for id in &orphans {
        conn.execute("DELETE FROM message_fts WHERE rowid = ?1", [id])?;
    }
    deleted += orphans.len();
    // The recipients as the insertion writes them: "name address" for To,
    // Cc and Bcc, in header order (`insert.rs`). The name is the
    // contact's, the closest the main file keeps.
    let added = conn.execute(
        "INSERT INTO message_fts (rowid, subject, sender, recipients, body)
         SELECT m.id, m.subject, trim(coalesce(m.from_name, '') || ' ' || coalesce(m.from_addr, '')),
                coalesce((SELECT group_concat(coalesce(a.name, '') || ' ' || a.addr, ' ')
                            FROM (SELECT ma.address_id FROM message_address ma
                                   WHERE ma.message_id = m.id AND ma.role IN ('to', 'cc', 'bcc')
                                   ORDER BY CASE ma.role WHEN 'to' THEN 0 WHEN 'cc' THEN 1 ELSE 2 END, ma.position) x
                            JOIN address a ON a.id = x.address_id), ''),
                coalesce((SELECT b.text_plain FROM message_body b WHERE b.message_id = m.id), '')
           FROM main.message m
          WHERE m.id NOT IN (SELECT id FROM message_fts_docsize)",
        [],
    )?;
    Ok((deleted, added))
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
        // From version 10 the bodies are in the second file, whose tables
        // `create_body_schema` makes when missing: not a reason to refuse.
        if version < 10 {
            checks.push(("SELECT tracking_pixels FROM message_body", "message_body.tracking_pixels"));
        }
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
    if version >= 16 {
        checks.push(("SELECT login_limit, login_blocked_at, login_pause_until, login_ok_at FROM account", "account.login_limit"));
    }
    if version >= 13 {
        checks.push(("SELECT prompt_hash, billed FROM ai_run", "ai_run"));
        checks.push(("SELECT run_id, thread_id FROM ai_suggestion", "ai_suggestion"));
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
    /// A mail of the thread, in any folder, was written by a person
    /// (`message.sender_kind`, `expediteurs.md`, section 8).
    has_person: bool,
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
    person: bool,
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
            has_person: mails.iter().any(|m| m.person),
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
                    date_last_received, message_count, unread_count, date_last_modified, has_person
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
                    has_person: r.get(10)?,
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
            "SELECT m.date_received, m.date_modified, m.is_seen, f.role, m.folder_id, m.sender_kind IS 'person'
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
                    person: r.get(5)?,
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
    column!(has_person, "has_person = ?", Value::Integer(new.has_person.into()));
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
/// - `with_person`: also `has_person` (schema version 12). The migration to
///   version 9 runs on a database that does not have the column yet.
pub(crate) fn refresh_all_threads_sql(with_person: bool) -> String {
    let (person_of, person) =
        if with_person { ("m.sender_kind IS 'person'", " has_person = a.hp,") } else { ("0", "") };
    let code = FolderRole::ALL
        .iter()
        .map(|r| format!("WHEN '{}' THEN '{}'", r.as_str(), r.code()))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "WITH m AS (
             SELECT m.thread_id AS t, m.date_received * 1000000000 + m.id AS k, m.date_received AS dr,
                    m.date_modified AS dm, m.is_seen AS seen, f.role AS role,
                    {person_of} AS person,
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
                    max(role IS 'quarantine') AS hq, max(role IS 'deleted') AS hd, max(person) AS hp,
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
             has_sent = a.hs, has_draft = a.hr, has_quarantine = a.hq, has_deleted = a.hd,{person}
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

    /// A folder stays announced only if every change that touched it
    /// announced it; an empty change set is neutral (`ChangeSet::merge`).
    #[test]
    fn announced_folders_merge_conservatively() {
        let announced = |folders: &[i64]| {
            let mut c = ChangeSet::default();
            c.folders.extend(folders);
            c.announce_threads();
            c
        };
        let silent = |folders: &[i64]| {
            let mut c = ChangeSet::default();
            c.folders.extend(folders);
            c
        };
        // Into an empty set: kept.
        let mut c = ChangeSet::default();
        c.merge(announced(&[1, 2]));
        assert_eq!(c.announced, [1, 2].into());
        // Folder 2 touched by a change that does not announce it: lost;
        // folder 3, announced by the second, kept.
        c.merge(silent(&[2]));
        c.merge(announced(&[3]));
        assert_eq!(c.announced, [1, 3].into());
        assert_eq!(c.folders, [1, 2, 3].into());
        // A silent change first, then an announced one: still silent.
        let mut d = silent(&[4]);
        d.merge(announced(&[4]));
        assert!(d.announced.is_empty());
    }
    use super::*;

    /// Every column `refresh_thread` maintains, for every thread.
    /// (id, folder, five flags, received, count, unread, modified).
    type Row = (i64, Option<String>, i64, i64, i64, i64, i64, i64, i64, i64, i64);

    fn aggregates(store: &Store) -> Vec<Row> {
        store
            .conn()
            .prepare(
                "SELECT id, folder, has_sent, has_draft, has_quarantine, has_deleted,
                        date_last_received, message_count, unread_count, date_last_modified, has_person
                   FROM thread ORDER BY id",
            )
            .unwrap()
            .query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?, r.get(9)?, r.get(10)?))
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
                 UPDATE message SET date_received = 1700000000 WHERE id % 13 = 0;
                 UPDATE message SET sender_kind = 'person' WHERE id % 4 = 0;
                 UPDATE message SET sender_kind = 'robot' WHERE id % 4 = 1;",
            )
            .unwrap();
        store.conn().execute_batch(&refresh_all_threads_sql(true)).unwrap();
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
