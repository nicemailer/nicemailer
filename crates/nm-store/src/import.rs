//! Import of a whole mailbox from an mbox file into an archive account
//! (`import-initial.md`, section 9).
//!
//! What the import does:
//! - creates the archive account and its workflow folders if needed;
//! - files every message in Archive, or in Sent when the account's own
//!   address sent it; every message is marked as read;
//! - keeps the raw message (`raw.rs`) and stores its interpretation
//!   (`ingest.rs`) in SQLite;
//! - groups messages into threads using `Message-ID`, `In-Reply-To` and
//!   `References`;
//! - marks the address of every new contact with `imported_from = 'mbox'`,
//!   and, optionally, gives the screening decision `inbox` to the people the
//!   user wrote to (`import-initial.md`, section 4).
//!
//! Re-running the same import is safe: a message whose raw bytes are already
//! in the account is skipped. An interrupted import is resumed by running it
//! again.
//!
//! ## Pipeline
//!
//! The test file is 9.7 GB, so speed matters. Three kinds of threads work at
//! the same time, connected by bounded channels (a full channel makes the
//! sender wait, which caps memory use):
//!
//! ```text
//! reader (1)  ──messages──▶  workers (one per core)  ──parsed──▶  writer (1)
//! splits the mbox            hash, compress, write                SQLite, in
//!                            the raw file, parse MIME,            batches of
//!                            sanitize HTML                        500 messages
//! ```
//!
//! Only the writer touches the database. It takes the store lock for one
//! batch at a time, so the app and the CLI keep being served between batches,
//! and each batch is one transaction: readers never see half a batch.
//!
//! A batch is bounded twice: at most [`BATCH_SIZE`] messages, and at most
//! [`LOCK_BUDGET`] of writing. The count is what amortizes the commits when
//! writing is fast; the time is what protects everyone else when it is not.
//! The synchronization, the app's requests and the events all wait for the
//! store lock, so a batch that takes 20 s stops the core for 20 s. It
//! happened: with a query planned badly, 500 messages took 15 to 20 s, the
//! synchronization waited up to 21 s for the lock, and its IMAP connections
//! were cut meanwhile.

use std::collections::{BTreeMap, VecDeque};
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nm_core::{FolderRole, Source};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

use crate::ingest::{self, ParsedMessage};
use crate::insert::{Body, Flags, Inserter, NewMessage};
use crate::mbox::MboxReader;
use crate::raw::{RawStore, Written};
use crate::{ChangeSet, Result, Store, StoreError};

/// Number of messages written per transaction. Large enough to amortize the
/// cost of a commit, small enough to release the store lock every few
/// hundred milliseconds.
pub const BATCH_SIZE: usize = 500;

/// Longest time a batch writes while holding the store lock. Past it, the
/// batch is committed with the messages written so far, and the rest go in
/// the next one. The commit itself comes on top: the limit counts the
/// writing of the messages. On a real mailbox (2,355 messages, 400 MB), a
/// batch of 500 held the lock 145 ms at most, commit included, so the limit
/// only cuts batches that are slow for a reason, and keeps the core
/// answering its clients meanwhile. At least one message is written per
/// batch, so the import always progresses.
pub const LOCK_BUDGET: Duration = Duration::from_millis(200);

/// Parameters of an import.
#[derive(Debug, Clone)]
pub struct ImportOptions {
    /// The mbox file. Must be an absolute path: the core does not share the
    /// working directory of the CLI.
    pub path: PathBuf,
    /// Name of the archive account (`account.config_key`).
    pub account: String,
    /// Address of the archived mailbox: messages from it go to Sent.
    pub address: String,
    /// Give the decision `inbox` to recipients of sent messages that have no
    /// decision yet (`import-initial.md`, section 4).
    pub mark_correspondents: bool,
    /// Client that asked for the import, recorded as `created_by`.
    pub source: Source,
    /// Messages per transaction ([`BATCH_SIZE`] in production; tests use
    /// smaller batches to exercise batch boundaries).
    pub batch_size: usize,
    /// Longest time a transaction may write ([`LOCK_BUDGET`] in production;
    /// tests use zero to get one message per transaction).
    pub lock_budget: Duration,
}

/// Progress, reported after each batch.
#[derive(Debug, Clone, Default)]
pub struct ImportProgress {
    pub bytes_read: u64,
    pub total_bytes: u64,
    /// Messages added to the database.
    pub imported: u64,
    /// Messages skipped because they were already in the account.
    pub skipped: u64,
    /// Messages kept (raw file and headers from the mbox) but that the MIME
    /// parser could not interpret.
    pub parse_errors: u64,
}

/// Final result of an import.
#[derive(Debug, Clone, Default)]
pub struct ImportReport {
    pub account_id: i64,
    pub progress: ImportProgress,
    /// Imported messages filed in Sent.
    pub sent: u64,
    /// Imported messages filed in Archive.
    pub archived: u64,
    /// Addresses that received the decision `inbox` as correspondents.
    pub correspondents: u64,
    pub seconds: f64,
}

/// The archive account an import writes into.
#[derive(Debug, Clone, Copy)]
pub struct ArchiveAccount {
    pub id: i64,
    pub archive_folder: i64,
    pub sent_folder: i64,
}

/// A message ready to be written: everything the slow part of the pipeline
/// (hashing, compression, parsing) produced.
struct Prepared {
    /// Position of the message in the file (0, 1, 2…). Workers finish in
    /// any order; the writer uses this number to write messages in file
    /// order, so that an import always produces the same database (same
    /// ids, same display name kept for an address).
    seq: u64,
    written: Written,
    /// Keeps the raw file from the garbage collector until the batch is
    /// committed (`raw.rs`, `Pin`).
    pin: Option<crate::raw::Pin>,
    parsed: ParsedMessage,
    /// Date of the mbox From_ line.
    delivered_at: Option<i64>,
}

impl Store {
    /// Creates the archive account `name` with its workflow folders, or
    /// returns it if it already exists with the same address.
    ///
    /// Refused: an invalid name or address, a name already used by an IMAP
    /// account, or an existing archive account with another address (the
    /// address decides what goes to Sent, so it cannot change between runs).
    pub fn prepare_archive_account(&mut self, name: &str, address: &str, source: Source) -> Result<ArchiveAccount> {
        let name = name.trim();
        // Account names end up as keys and in file names later: keep them
        // simple.
        if name.is_empty()
            || name.len() > 64
            || !name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        {
            return Err(StoreError::InvalidParams(format!(
                "nom de compte invalide : « {name} » (lettres, chiffres, - _ . ; 64 caractères au plus)"
            )));
        }
        let address = address.trim().to_lowercase();
        if address.len() < 3 || !address.contains('@') || address.contains(char::is_whitespace) {
            return Err(StoreError::Invalid(nm_api::reason::ADDRESS_INVALID.with("address", address)));
        }

        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<(i64, String, Option<String>)> = tx
            .query_row("SELECT id, kind, address FROM account WHERE config_key = ?1", [name], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .optional()?;
        let account_id = match existing {
            Some((_, kind, _)) if kind != "archive" => {
                return Err(StoreError::InvalidParams(format!(
                    "« {name} » est un compte {kind} : choisissez un autre nom pour le compte d'archives"
                )));
            }
            Some((_, _, addr)) if addr.as_deref() != Some(address.as_str()) => {
                return Err(StoreError::InvalidParams(format!(
                    "le compte d'archives « {name} » a l'adresse {}, pas {address}",
                    addr.unwrap_or_default()
                )));
            }
            Some((id, _, _)) => id,
            None => {
                tx.execute(
                    // After the other accounts in the display order.
                    "INSERT INTO account (config_key, kind, address, created_by, modified_by, position)
                     VALUES (?1, 'archive', ?2, ?3, ?3, (SELECT coalesce(max(position), 0) + 1 FROM account))",
                    params![name, address, source.as_str()],
                )?;
                let id = tx.last_insert_rowid();
                tx.execute(
                    "INSERT INTO audit_log (created_by, entity, entity_id, action, detail)
                     VALUES (?1, 'account', ?2, 'create',
                             json_object('after', json_object('config_key', ?3, 'kind', 'archive', 'address', ?4)))",
                    params![source.as_str(), id, name, address],
                )?;
                id
            }
        };
        // Every workflow folder, so the user can file imported mail anywhere
        // (moves are local: there is no server). `INSERT OR IGNORE` relies
        // on UNIQUE (account_id, path) and also repairs a missing folder.
        for role in FolderRole::ALL {
            tx.execute(
                "INSERT OR IGNORE INTO folder (account_id, path, delimiter, role, created_by, modified_by)
                 VALUES (?1, ?2, '/', ?3, ?4, ?4)",
                params![account_id, role.display_name(), role.as_str(), source.as_str()],
            )?;
        }
        let folder = |role: FolderRole| -> Result<i64> {
            tx.query_row(
                "SELECT id FROM folder WHERE account_id = ?1 AND role = ?2",
                params![account_id, role.as_str()],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound(format!("folder {}", role.as_str())))
        };
        let account = ArchiveAccount {
            id: account_id,
            archive_folder: folder(FolderRole::Archive)?,
            sent_folder: folder(FolderRole::Sent)?,
        };
        tx.commit()?;
        Ok(account)
    }
}

/// Runs a whole import. Blocking: call it from a dedicated thread.
///
/// `store` is the shared store of the core; the lock is taken once per
/// batch. After each committed batch, `on_batch` is called *while the lock
/// is still held*, with what changed and the progress so far: the core
/// emits its events there, so they are ordered exactly like the writes
/// (`evenements.md`).
///
/// Rust note: `&mut dyn FnMut(...)` is a borrowed closure that may modify
/// its captured state (`FnMut`), called through dynamic dispatch (`dyn`):
/// the import does not need to know the closure's concrete type.
pub fn import_mbox(
    store: &Mutex<Store>,
    raw: &RawStore,
    opts: &ImportOptions,
    on_batch: &mut dyn FnMut(&Store, &ChangeSet, &ImportProgress),
) -> Result<ImportReport> {
    let started = Instant::now();
    let file = File::open(&opts.path).map_err(|e| io_error(&opts.path, e))?;
    let total_bytes = file.metadata().map_err(|e| io_error(&opts.path, e))?.len();
    let account = store.lock().unwrap().prepare_archive_account(&opts.account, &opts.address, opts.source)?;
    let address = opts.address.trim().to_lowercase();

    let bytes_read = AtomicU64::new(0);
    let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    let mut writer = Writer {
        account,
        address,
        inserter: Inserter::new(opts.source, Some("mbox")),
        report: ImportReport { account_id: account.id, ..Default::default() },
        imported_at: unix_now(),
    };
    writer.report.progress.total_bytes = total_bytes;

    // Rust note: `std::thread::scope` lets the threads borrow local
    // variables (`raw`, `bytes_read`, the receivers): the scope does not
    // return before every thread it spawned has finished, so the borrows
    // cannot outlive the data. Without it, every shared value would need an
    // `Arc`.
    let outcome: Result<()> = std::thread::scope(|scope| {
        // Bounded channels: at most a few messages per worker in flight.
        let (msg_tx, msg_rx) = sync_channel::<(u64, crate::mbox::MboxMessage)>(workers * 4);
        let (prep_tx, prep_rx) = sync_channel::<std::io::Result<Prepared>>(workers * 4);

        // Reader thread: splits the mbox into messages.
        let bytes_read = &bytes_read;
        let reader = scope.spawn(move || -> std::io::Result<()> {
            let mut mbox = MboxReader::new(BufReader::with_capacity(1 << 20, file));
            let mut seq = 0;
            while let Some(m) = mbox.next_message()? {
                bytes_read.store(mbox.position(), Ordering::Relaxed);
                // A send error means the writer stopped (error): stop too.
                if msg_tx.send((seq, m)).is_err() {
                    break;
                }
                seq += 1;
            }
            bytes_read.store(mbox.position(), Ordering::Relaxed);
            Ok(())
        });

        // Worker threads share one receiver. `std::sync::mpsc` receivers
        // cannot be cloned, hence the `Mutex` around it: each worker locks
        // it just long enough to take the next message.
        let msg_rx = Arc::new(Mutex::new(msg_rx));
        for _ in 0..workers {
            let msg_rx = msg_rx.clone();
            let prep_tx: SyncSender<_> = prep_tx.clone();
            scope.spawn(move || {
                loop {
                    // The lock guard is dropped at the end of this statement,
                    // before the slow work below.
                    let next = msg_rx.lock().unwrap().recv();
                    let Ok((seq, m)) = next else { break };
                    let prepared = prepare(raw, seq, m);
                    if prep_tx.send(prepared).is_err() {
                        break;
                    }
                }
            });
        }
        // Drop the originals kept by this thread. Once every worker is done,
        // the parsed channel closes and the writer loop below ends. And if
        // the writer stops early, the workers exit, the last receiver of
        // messages goes away, and the reader's next `send` fails instead of
        // waiting forever on a full channel.
        drop(prep_tx);
        drop(msg_rx);

        let limits = Limits { messages: opts.batch_size.max(1), time: opts.lock_budget };
        let written = writer.run(store, raw, &prep_rx, limits, bytes_read, on_batch);
        // If the writer failed, dropping the receiver makes the workers'
        // next `send` fail, so they stop, and then the reader stops too.
        drop(prep_rx);
        let read = reader.join().expect("reader thread panicked");
        written?;
        read.map_err(|e| io_error(&opts.path, e))
    });
    outcome?;

    writer.finish(store, opts, on_batch)?;
    writer.report.seconds = started.elapsed().as_secs_f64();
    Ok(writer.report)
}

/// Longest plausible delay between the `Date` header of a message (set by
/// the sender's client when sending) and its delivery. SMTP servers give up
/// retrying after about 5 days, so a real delivery never comes later than
/// that; 7 days leaves a margin for clocks set wrong.
pub const MAX_DELIVERY_DELAY_SECS: i64 = 7 * 24 * 3600;

/// Reception date of an imported message (`import-initial.md`, section 9,
/// "Autres attributs"), in Unix seconds.
///
/// The date of the mbox `From ` line is preferred: when the exporting server
/// received the message, like IMAP's INTERNALDATE. But some tools do not know
/// that date and write the time of the export instead. Python's `mailbox`
/// module does this, and so did `scripts/export-apple-mail.py`: every message
/// then appeared as received on the day of the export. Since the core cannot
/// tell who wrote the file, it checks the date instead and falls back to the
/// `Date` header when the `From ` line date cannot be a delivery date:
/// - it is after the start of the import (a date in the future);
/// - it is more than `MAX_DELIVERY_DELAY_SECS` after the `Date` header.
///
/// A message whose `From ` line date is *before* its `Date` header keeps the
/// `From ` line date: that only means the sender's clock was ahead. Without
/// any usable date, the message gets 0 (1970), which sorts it last.
pub fn reception_date(delivered_at: Option<i64>, date_sent: Option<i64>, imported_at: i64) -> i64 {
    // Rust note: `Option::filter` keeps the value only if the closure returns
    // true, and `or` then falls back to the other `Option`. `is_none_or`
    // is true for `None`, or when the closure is true for the value.
    delivered_at
        .filter(|&d| d <= imported_at)
        .filter(|&d| date_sent.is_none_or(|s| d - s <= MAX_DELIVERY_DELAY_SECS))
        .or(date_sent)
        .unwrap_or(0)
}

/// Current time in Unix seconds.
fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// The slow, parallel part of the pipeline for one message.
fn prepare(raw: &RawStore, seq: u64, m: crate::mbox::MboxMessage) -> std::io::Result<Prepared> {
    let (written, pin) = raw.write_pinned(&m.bytes)?;
    let parsed = ingest::parse(&m.bytes);
    Ok(Prepared { seq, written, pin: Some(pin), parsed, delivered_at: m.delivered_at })
}

fn io_error(path: &std::path::Path, e: std::io::Error) -> StoreError {
    StoreError::InvalidParams(format!("{} : {e}", path.display()))
}

/// How much one batch may hold: [`ImportOptions::batch_size`] and
/// [`ImportOptions::lock_budget`].
#[derive(Debug, Clone, Copy)]
struct Limits {
    messages: usize,
    time: Duration,
}

/// State of the single writer thread.
struct Writer {
    account: ArchiveAccount,
    address: String,
    /// The insertion path shared with the IMAP synchronization
    /// (`insert.rs`): rows, addresses, threading.
    inserter: Inserter,
    report: ImportReport,
    /// When the import started, in Unix seconds: no message can have been
    /// delivered after it (see `reception_date`).
    imported_at: i64,
}

impl Writer {
    /// Receives prepared messages and writes them in batches.
    fn run(
        &mut self,
        store: &Mutex<Store>,
        raw: &RawStore,
        rx: &Receiver<std::io::Result<Prepared>>,
        limits: Limits,
        bytes_read: &AtomicU64,
        on_batch: &mut dyn FnMut(&Store, &ChangeSet, &ImportProgress),
    ) -> Result<()> {
        // A queue: a batch cut by the time limit leaves its last messages at
        // the front, written first by the next batch, so the file order is
        // kept.
        let mut batch: VecDeque<Prepared> = VecDeque::with_capacity(limits.messages);
        // Messages that arrived ahead of their turn, by position. Its size
        // is bounded by the number of messages in flight in the channels.
        let mut ahead: BTreeMap<u64, Prepared> = BTreeMap::new();
        let mut next_seq = 0;
        // `rx.iter()` yields messages until every sender is gone.
        for item in rx.iter() {
            let p = item.map_err(|e| StoreError::Io(format!("stockage du brut : {e}")))?;
            ahead.insert(p.seq, p);
            // Move every message whose turn has come into the batch.
            while let Some(p) = ahead.remove(&next_seq) {
                next_seq += 1;
                batch.push_back(p);
                if batch.len() >= limits.messages {
                    self.flush(store, raw, &mut batch, limits, bytes_read, on_batch)?;
                }
            }
        }
        // Every worker is done, so nothing can be missing any more.
        debug_assert!(ahead.is_empty());
        // The time limit may need several batches for what is left.
        while !batch.is_empty() {
            self.flush(store, raw, &mut batch, limits, bytes_read, on_batch)?;
        }
        Ok(())
    }

    /// Writes one batch in one transaction, then reports it: the messages
    /// of `batch` from the front, until there are none left or the writing
    /// has taken `limits.time`. What is left stays in `batch`.
    fn flush(
        &mut self,
        store: &Mutex<Store>,
        raw: &RawStore,
        batch: &mut VecDeque<Prepared>,
        limits: Limits,
        bytes_read: &AtomicU64,
        on_batch: &mut dyn FnMut(&Store, &ChangeSet, &ImportProgress),
    ) -> Result<()> {
        if batch.is_empty() {
            return Ok(());
        }
        // The batch's raw files must be durable before the rows that
        // reference them are committed (`raw.rs`, "Durability on macOS").
        // Done before taking the lock: the flush takes milliseconds. It also
        // leaves the lock free for a moment between two batches, so that a
        // thread waiting for it gets it.
        raw.barrier().map_err(|e| StoreError::Io(format!("stockage du brut : {e}")))?;
        let mut guard = store.lock().unwrap();
        let started = Instant::now();
        let mut changes = ChangeSet::default();
        {
            let tx = guard.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            // The pins live until the commit below. `pop_front` moves each
            // message out of the queue; the queue keeps its memory, ready to
            // be refilled.
            let mut pins = Vec::with_capacity(batch.len());
            // Rust note: `while let` loops as long as the pattern matches:
            // here, as long as the queue gives a message. The time is checked
            // after each message, so a batch has at least one.
            while let Some(mut p) = batch.pop_front() {
                pins.extend(p.pin.take());
                if let Err(e) = self.insert(&tx, p, &mut changes) {
                    // The transaction is rolled back when `tx` is dropped.
                    self.inserter.abandon();
                    return Err(e);
                }
                if started.elapsed() >= limits.time {
                    break;
                }
            }
            changes.folders.insert(self.account.archive_folder);
            changes.folders.insert(self.account.sent_folder);
            self.inserter.commit(tx, &mut changes)?;
            drop(pins);
        }
        self.report.progress.bytes_read = bytes_read.load(Ordering::Relaxed);
        on_batch(&guard, &changes, &self.report.progress);
        Ok(())
    }

    /// Inserts one message, unless the account already has these exact bytes.
    fn insert(&mut self, tx: &Transaction, p: Prepared, changes: &mut ChangeSet) -> Result<()> {
        let account_id = self.account.id;
        let duplicate = tx
            .prepare_cached("SELECT 1 FROM message WHERE raw_hash = ?1 AND account_id = ?2 LIMIT 1")?
            .exists(params![&p.written.hash[..], account_id])?;
        if duplicate {
            self.report.progress.skipped += 1;
            return Ok(());
        }
        let m = &p.parsed;
        let sent = m.from.as_ref().is_some_and(|f| f.addr == self.address);
        let message = NewMessage {
            account_id,
            folder_id: if sent { self.account.sent_folder } else { self.account.archive_folder },
            uid: None,
            date_received: reception_date(p.delivered_at, m.date_sent, self.imported_at),
            size: Some(p.written.size),
            modseq: None,
            // Archived mail has been dealt with: read.
            flags: Flags { seen: true, ..Flags::default() },
            auth: (nm_core::AuthResult::Unknown, None),
            headers: m,
        };
        self.inserter.insert(tx, &message, Some(Body { written: &p.written, parsed: m }), changes)?;
        if !m.parsed {
            self.report.progress.parse_errors += 1;
        }
        if sent {
            self.report.sent += 1;
        } else {
            self.report.archived += 1;
        }
        self.report.progress.imported += 1;
        Ok(())
    }

    /// Last steps, once every message is written: answered flags,
    /// correspondents and the audit log entry of the import.
    fn finish(
        &mut self,
        store: &Mutex<Store>,
        opts: &ImportOptions,
        on_batch: &mut dyn FnMut(&Store, &ChangeSet, &ImportProgress),
    ) -> Result<()> {
        let mut guard = store.lock().unwrap();
        let tx = guard.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let acc = self.account;
        // A received message answered by one of the user's sent messages is
        // marked as answered, as IMAP's \Answered flag would have shown it.
        tx.execute(
            "UPDATE message SET is_answered = 1
              WHERE account_id = ?1 AND folder_id <> ?2 AND is_answered = 0
                AND message_id_hdr IN (SELECT in_reply_to FROM message
                                        WHERE account_id = ?1 AND folder_id = ?2 AND in_reply_to IS NOT NULL)",
            params![acc.id, acc.sent_folder],
        )?;

        if opts.mark_correspondents {
            // People the user wrote to are real correspondents: they get the
            // decision `inbox` if they have none (`import-initial.md`,
            // section 4). An address whose decision was removed by the user
            // (NULL target with a date: tombstone) is left alone, so the
            // import never undoes a choice of the user.
            let ids: Vec<i64> = {
                let mut stmt = tx.prepare(
                    "SELECT DISTINCT a.id FROM message m
                       JOIN message_address ma ON ma.message_id = m.id AND ma.role IN ('to', 'cc', 'bcc')
                       JOIN address a ON a.id = ma.address_id
                      WHERE m.account_id = ?1 AND m.folder_id = ?2
                        AND a.screening_target IS NULL AND a.date_screening_modified IS NULL
                        AND a.addr <> ?3
                      ORDER BY a.id",
                )?;
                stmt.query_map(params![acc.id, acc.sent_folder, self.address], |r| r.get(0))?
                    .collect::<std::result::Result<_, _>>()?
            };
            for id in &ids {
                tx.execute(
                    "UPDATE address SET screening_target = 'inbox', date_screening_modified = unixepoch(),
                            date_modified = unixepoch(), modified_by = 'core'
                      WHERE id = ?1",
                    [id],
                )?;
                tx.execute(
                    "INSERT INTO audit_log (created_by, entity, entity_id, action, detail)
                     VALUES ('core', 'address', ?1, 'screening',
                             json_object('before', NULL, 'after', 'inbox', 'reason', 'sent_correspondent'))",
                    [id],
                )?;
            }
            self.report.correspondents = ids.len() as u64;
        }

        let p = &self.report.progress;
        tx.execute(
            "INSERT INTO audit_log (created_by, entity, entity_id, action, detail)
             VALUES (?1, 'account', ?2, 'import_mbox',
                     json_object('file', ?3, 'imported', ?4, 'skipped', ?5, 'parse_errors', ?6,
                                 'sent', ?7, 'archived', ?8, 'correspondents', ?9))",
            params![
                opts.source.as_str(),
                acc.id,
                opts.path.display().to_string(),
                p.imported as i64,
                p.skipped as i64,
                p.parse_errors as i64,
                self.report.sent as i64,
                self.report.archived as i64,
                self.report.correspondents as i64
            ],
        )?;
        tx.commit()?;
        // Tens of thousands of new rows change the planner statistics.
        guard.optimize()?;
        let mut changes = ChangeSet::default();
        changes.folders.insert(acc.archive_folder);
        changes.folders.insert(acc.sent_folder);
        on_batch(&guard, &changes, &self.report.progress);
        Ok(())
    }
}
