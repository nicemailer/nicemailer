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

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use nm_core::{FolderRole, Source};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

use crate::ingest::{self, PARSER_VERSION, ParsedAddress, ParsedMessage};
use crate::mbox::MboxReader;
use crate::raw::{RawStore, Written};
use crate::{ChangeSet, Result, Store, StoreError, refresh_folder_counts, refresh_thread};

/// Number of messages written per transaction. Large enough to amortize the
/// cost of a commit, small enough to release the store lock every few
/// hundred milliseconds.
pub const BATCH_SIZE: usize = 500;

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
            return Err(StoreError::InvalidParams(format!("adresse invalide : « {address} »")));
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
                    "INSERT INTO account (config_key, kind, address, created_by, modified_by)
                     VALUES (?1, 'archive', ?2, ?3, ?3)",
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
            .ok_or_else(|| StoreError::NotFound(format!("dossier {}", role.as_str())))
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
        source: opts.source,
        addresses: HashMap::new(),
        report: ImportReport { account_id: account.id, ..Default::default() },
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

        let written = writer.run(store, raw, &prep_rx, opts.batch_size.max(1), bytes_read, on_batch);
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

/// The slow, parallel part of the pipeline for one message.
fn prepare(raw: &RawStore, seq: u64, m: crate::mbox::MboxMessage) -> std::io::Result<Prepared> {
    let written = raw.write(&m.bytes)?;
    let parsed = ingest::parse(&m.bytes);
    Ok(Prepared { seq, written, parsed, delivered_at: m.delivered_at })
}

fn io_error(path: &std::path::Path, e: std::io::Error) -> StoreError {
    StoreError::InvalidParams(format!("{} : {e}", path.display()))
}

/// State of the single writer thread.
struct Writer {
    account: ArchiveAccount,
    address: String,
    source: Source,
    /// Address ids already looked up, by lowercase address. Most messages
    /// involve the same few hundred people, so this saves most queries.
    addresses: HashMap<String, i64>,
    report: ImportReport,
}

impl Writer {
    /// Receives prepared messages and writes them in batches.
    fn run(
        &mut self,
        store: &Mutex<Store>,
        raw: &RawStore,
        rx: &Receiver<std::io::Result<Prepared>>,
        batch_size: usize,
        bytes_read: &AtomicU64,
        on_batch: &mut dyn FnMut(&Store, &ChangeSet, &ImportProgress),
    ) -> Result<()> {
        let mut batch: Vec<Prepared> = Vec::with_capacity(batch_size);
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
                batch.push(p);
                if batch.len() >= batch_size {
                    self.flush(store, raw, &mut batch, bytes_read, on_batch)?;
                }
            }
        }
        // Every worker is done, so nothing can be missing any more.
        debug_assert!(ahead.is_empty());
        self.flush(store, raw, &mut batch, bytes_read, on_batch)
    }

    /// Writes one batch in one transaction, then reports it.
    fn flush(
        &mut self,
        store: &Mutex<Store>,
        raw: &RawStore,
        batch: &mut Vec<Prepared>,
        bytes_read: &AtomicU64,
        on_batch: &mut dyn FnMut(&Store, &ChangeSet, &ImportProgress),
    ) -> Result<()> {
        if batch.is_empty() {
            return Ok(());
        }
        // The batch's raw files must be durable before the rows that
        // reference them are committed (`raw.rs`, "Durability on macOS").
        // Done before taking the lock: the flush takes milliseconds.
        raw.barrier().map_err(|e| StoreError::Io(format!("stockage du brut : {e}")))?;
        let mut guard = store.lock().unwrap();
        let mut changes = ChangeSet::default();
        {
            let tx = guard.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let mut threads: BTreeSet<i64> = BTreeSet::new();
            // `drain(..)` moves the messages out of the batch, leaving it
            // empty and ready to be refilled (its memory is reused).
            for p in batch.drain(..) {
                self.insert(&tx, p, &mut threads, &mut changes)?;
            }
            for t in &threads {
                tx.execute(
                    "UPDATE thread SET subject = coalesce(
                        (SELECT subject FROM message WHERE thread_id = ?1 ORDER BY date_received, id LIMIT 1), '')
                      WHERE id = ?1",
                    [t],
                )?;
                for f in refresh_thread(&tx, *t)? {
                    changes.folders.insert(f);
                }
            }
            changes.folders.insert(self.account.archive_folder);
            changes.folders.insert(self.account.sent_folder);
            for f in &changes.folders {
                refresh_folder_counts(&tx, *f)?;
            }
            tx.commit()?;
        }
        self.report.progress.bytes_read = bytes_read.load(Ordering::Relaxed);
        on_batch(&guard, &changes, &self.report.progress);
        Ok(())
    }

    /// Inserts one message, unless the account already has these exact bytes.
    fn insert(&mut self, tx: &Transaction, p: Prepared, threads: &mut BTreeSet<i64>, changes: &mut ChangeSet) -> Result<()> {
        let account_id = self.account.id;
        let hash = &p.written.hash[..];
        let duplicate = tx
            .prepare_cached("SELECT 1 FROM message WHERE raw_hash = ?1 AND account_id = ?2 LIMIT 1")?
            .exists(params![hash, account_id])?;
        if duplicate {
            self.report.progress.skipped += 1;
            return Ok(());
        }
        tx.prepare_cached("INSERT OR IGNORE INTO raw_blob (hash, size, stored_size) VALUES (?1, ?2, ?3)")?
            .execute(params![hash, p.written.size as i64, p.written.stored_size as i64])?;

        let m = &p.parsed;
        let sent = m.from.as_ref().is_some_and(|f| f.addr == self.address);
        let folder_id = if sent { self.account.sent_folder } else { self.account.archive_folder };
        let from_id = match &m.from {
            Some(a) => Some(self.address_id(tx, a)?),
            None => None,
        };
        // Reception date: the mbox From_ line (when the exporting server
        // received it, like IMAP's INTERNALDATE), else the Date header.
        let date_received = p.delivered_at.or(m.date_sent).unwrap_or(0);
        let thread_id = self.thread_for(tx, m, changes)?;
        threads.insert(thread_id);

        tx.prepare_cached(
            "INSERT INTO message (account_id, folder_id, uid, thread_id, message_id_hdr, in_reply_to, subject,
                from_address_id, from_name, from_addr, date_sent, date_received, size, snippet,
                is_seen, has_attachments, auth_result, body_state, raw_hash, created_by, modified_by)
             VALUES (?1, ?2, NULL, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                     1, ?14, 'unknown', 1, ?15, ?16, ?16)",
        )?
        .execute(params![
            account_id,
            folder_id,
            thread_id,
            m.message_id,
            m.in_reply_to,
            m.subject,
            from_id,
            m.from.as_ref().and_then(|a| a.name.as_deref()),
            m.from.as_ref().map(|a| a.addr.as_str()),
            m.date_sent,
            date_received,
            p.written.size as i64,
            m.snippet,
            m.has_attachments(),
            hash,
            self.source.as_str(),
        ])?;
        let message_id = tx.last_insert_rowid();

        for (pos, r) in m.references.iter().enumerate() {
            tx.prepare_cached("INSERT OR IGNORE INTO message_reference (message_id, position, ref_msgid) VALUES (?1, ?2, ?3)")?
                .execute(params![message_id, pos as i64, r])?;
        }
        let mut recipients = String::new();
        for (role, list) in [("to", &m.to), ("cc", &m.cc), ("bcc", &m.bcc), ("reply_to", &m.reply_to)] {
            for (pos, a) in list.iter().enumerate() {
                let aid = self.address_id(tx, a)?;
                tx.prepare_cached(
                    "INSERT OR IGNORE INTO message_address (message_id, role, position, address_id) VALUES (?1, ?2, ?3, ?4)",
                )?
                .execute(params![message_id, role, pos as i64, aid])?;
                if role != "reply_to" {
                    recipients.push_str(a.name.as_deref().unwrap_or(""));
                    recipients.push(' ');
                    recipients.push_str(&a.addr);
                    recipients.push(' ');
                }
            }
        }
        if m.parsed {
            tx.prepare_cached(
                "INSERT INTO message_body (message_id, text_plain, html_sanitized, has_remote_content, parser_version)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?
            .execute(params![message_id, m.text, m.html, m.has_remote_content, PARSER_VERSION])?;
        } else {
            self.report.progress.parse_errors += 1;
        }
        // Search index: one row per message, written once with everything
        // (no trigger, see the comment above `message_fts` in schema.sql).
        let sender = m.from.as_ref().map(|a| format!("{} {}", a.name.as_deref().unwrap_or(""), a.addr)).unwrap_or_default();
        tx.prepare_cached("INSERT INTO message_fts (rowid, subject, sender, recipients, body) VALUES (?1, ?2, ?3, ?4, ?5)")?
            .execute(params![message_id, m.subject, sender.trim(), recipients.trim_end(), m.text.as_deref().unwrap_or("")])?;
        for a in &m.attachments {
            tx.prepare_cached(
                "INSERT OR IGNORE INTO attachment (message_id, part_id, filename, mime_type, size, content_id, is_inline)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?
            .execute(params![message_id, a.part_id, a.filename, a.mime_type, a.size, a.content_id, a.is_inline])?;
        }

        if sent {
            self.report.sent += 1;
        } else {
            self.report.archived += 1;
        }
        self.report.progress.imported += 1;
        Ok(())
    }

    /// Id of the `address` row, created if needed with `imported_from =
    /// 'mbox'`. An address that already exists keeps its name and its
    /// `imported_from` (`import-initial.md`, section 7).
    fn address_id(&mut self, tx: &Transaction, a: &ParsedAddress) -> Result<i64> {
        if let Some(id) = self.addresses.get(&a.addr) {
            return Ok(*id);
        }
        let existing: Option<i64> = tx
            .prepare_cached("SELECT id FROM address WHERE addr = ?1")?
            .query_row([&a.addr], |r| r.get(0))
            .optional()?;
        let id = match existing {
            Some(id) => id,
            None => {
                tx.prepare_cached(
                    "INSERT INTO address (addr, name, imported_from, created_by, modified_by)
                     VALUES (?1, ?2, 'mbox', ?3, ?3)",
                )?
                .execute(params![a.addr, a.name, self.source.as_str()])?;
                tx.last_insert_rowid()
            }
        };
        self.addresses.insert(a.addr.clone(), id);
        Ok(id)
    }

    /// Finds the thread of a message, creating or merging threads as needed.
    ///
    /// A message belongs to the thread of any message of the same account
    /// that it references, that references it, or that has the same
    /// `Message-ID` (another copy). Messages arrive in any order, so a reply
    /// can be imported before the message it answers: when the parent
    /// arrives, it finds the reply through `message_reference`. When a
    /// message links several existing threads, they are merged into the
    /// oldest one.
    ///
    /// This is the linking part of the JWZ algorithm (`modele de
    /// donnees.md`); messages without any reference are not grouped by
    /// subject, which would wrongly merge unrelated mails such as monthly
    /// invoices with the same subject.
    fn thread_for(&mut self, tx: &Transaction, m: &ParsedMessage, changes: &mut ChangeSet) -> Result<i64> {
        let account_id = self.account.id;
        let mut found: BTreeSet<i64> = BTreeSet::new();
        {
            let mut by_id = tx.prepare_cached(
                "SELECT DISTINCT thread_id FROM message
                  WHERE account_id = ?1 AND message_id_hdr = ?2 AND thread_id IS NOT NULL",
            )?;
            for r in m.references.iter().chain(m.message_id.iter()) {
                for t in by_id.query_map(params![account_id, r], |row| row.get::<_, i64>(0))? {
                    found.insert(t?);
                }
            }
        }
        if let Some(id) = &m.message_id {
            let mut referencing = tx.prepare_cached(
                "SELECT DISTINCT m.thread_id FROM message_reference r JOIN message m ON m.id = r.message_id
                  WHERE r.ref_msgid = ?1 AND m.account_id = ?2 AND m.thread_id IS NOT NULL",
            )?;
            for t in referencing.query_map(params![id, account_id], |row| row.get::<_, i64>(0))? {
                found.insert(t?);
            }
        }
        // `pop_first` removes and returns the smallest element: the oldest
        // thread, which survives the merge.
        let Some(keep) = found.pop_first() else {
            tx.prepare_cached("INSERT INTO thread (account_id, subject) VALUES (?1, ?2)")?
                .execute(params![account_id, m.subject])?;
            return Ok(tx.last_insert_rowid());
        };
        for other in found {
            // Folders of the absorbed thread must be refreshed too.
            {
                let mut stmt = tx.prepare_cached("SELECT folder_id FROM thread_folder WHERE thread_id = ?1")?;
                for f in stmt.query_map([other], |r| r.get::<_, i64>(0))? {
                    changes.folders.insert(f?);
                }
            }
            tx.prepare_cached("UPDATE message SET thread_id = ?1 WHERE thread_id = ?2")?.execute(params![keep, other])?;
            // `thread_folder` rows go with the thread (ON DELETE CASCADE).
            tx.prepare_cached("DELETE FROM thread WHERE id = ?1")?.execute([other])?;
        }
        Ok(keep)
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
        let mut changes = ChangeSet::default();
        changes.folders.insert(acc.archive_folder);
        changes.folders.insert(acc.sent_folder);
        on_batch(&guard, &changes, &self.report.progress);
        Ok(())
    }
}
