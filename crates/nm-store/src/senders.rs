//! People and robots in the database (`specifications/expediteurs.md`):
//! counting each received mail on its address, the verdict on an address
//! and its copy in every one of its mails, the verdict forced by the user,
//! and the catch-up of the mails stored before version 12.
//!
//! The clues and the rules themselves are in `sender_kind.rs`, pure; this
//! module only reads and writes rows.
//!
//! # Why the verdict is copied into the messages
//!
//! The list filter "human / robot" is a frequent criterion (`expediteurs.md`,
//! section 2): it must cost nothing, a column of the thread tested while the
//! list walks its index (`thread.has_person`), never a join with `address`
//! nor a set of threads built at each request (50 ms a page on 400,000
//! mails). So `message.sender_kind` holds the verdict of each mail,
//! `thread.has_person` sums it up for the conversation, and every change of
//! an address's verdict rewrites both, in the same transaction, through
//! `idx_message_from`. A verdict changes rarely (at its first mails, or when
//! the user forces it), so this write is rare too.

use std::sync::Mutex;
use std::time::Instant;

use nm_core::{SenderKind, Source};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

use crate::raw::{Hash, RawStore};
use crate::screening::Outcome;
use crate::sender_kind::{self, Counts, Facts, MACHINE, NATURE_NONE};
use crate::{Result, Store};

/// The `sender_kind` of every mail of an address that the user did not
/// write, from the verdict `?2` on the address `?1`: `robot` for an
/// automatic mail itself (`MACHINE` bits, `?3`), else the verdict. Only the
/// rows that change are written. The user's own mails (Sent, Drafts) keep
/// `person`; those whose clues are not read yet (`sender_signals IS NULL`)
/// wait for the catch-up, which sets them.
const PROPAGATE: &str = "UPDATE message SET sender_kind = CASE WHEN sender_signals & ?3 <> 0 THEN 'robot' ELSE ?2 END
  WHERE from_address_id = ?1 AND sender_signals IS NOT NULL
    AND folder_id NOT IN (SELECT id FROM folder WHERE role IN ('sent', 'drafts'))
    AND sender_kind IS NOT (CASE WHEN sender_signals & ?3 <> 0 THEN 'robot' ELSE ?2 END)";

/// `thread.has_person` of the thread `?1` from its mails (the rule of
/// `ThreadAggregates::of`), written only when it changes. Used where mails
/// change kind without `refresh_thread`: the catch-up and the propagation
/// of a verdict.
const THREAD_HAS_PERSON: &str = "UPDATE thread SET has_person = EXISTS
    (SELECT 1 FROM message WHERE thread_id = ?1 AND sender_kind = 'person')
  WHERE id = ?1 AND has_person IS NOT EXISTS
    (SELECT 1 FROM message WHERE thread_id = ?1 AND sender_kind = 'person')";

/// The same for every thread of the mails of the address `?1`.
const ADDRESS_THREADS_HAS_PERSON: &str = "UPDATE thread SET has_person = EXISTS
    (SELECT 1 FROM message m WHERE m.thread_id = thread.id AND m.sender_kind = 'person')
  WHERE id IN (SELECT thread_id FROM message WHERE from_address_id = ?1 AND thread_id IS NOT NULL)
    AND has_person IS NOT EXISTS
    (SELECT 1 FROM message m WHERE m.thread_id = thread.id AND m.sender_kind = 'person')";

/// Records the clues of a message just inserted, or read by the catch-up:
/// its `sender_signals` and `sender_kind`, and the count of its address.
///
/// `own_mail`: the message is in Sent or Drafts, written by the user. It
/// is `person`, and is not counted on its address (the user's own).
/// Otherwise the mail is counted on its sender `from`, whose verdict is
/// computed again (and copied into its other mails if it changed).
pub(crate) fn record(tx: &Connection, message_id: i64, from: Option<i64>, signals: i64, own_mail: bool) -> Result<()> {
    let kind = if own_mail {
        Some(SenderKind::Person)
    } else if let Some(address) = from {
        let mut delta = Counts::default();
        delta.add(signals);
        if delta.count > 0 {
            tx.prepare_cached(
                "UPDATE address SET sender_count = sender_count + ?2, sender_bulk = sender_bulk + ?3,
                        sender_list = sender_list + ?4, sender_human = sender_human + ?5,
                        sender_client = sender_client + ?6
                  WHERE id = ?1",
            )?
            .execute(params![address, delta.count, delta.bulk, delta.list, delta.human, delta.client])?;
        }
        let verdict = refresh_address(tx, address)?;
        sender_kind::message_kind(Some(signals), false, verdict)
    } else {
        // No sender: no verdict, the filter counts it with the robots.
        None
    };
    let thread: Option<i64> = tx
        .prepare_cached("UPDATE message SET sender_signals = ?2, sender_kind = ?3 WHERE id = ?1 RETURNING thread_id")?
        .query_row(params![message_id, signals, kind.map(SenderKind::as_str)], |r| r.get(0))
        .optional()?
        .flatten();
    if let Some(t) = thread {
        tx.prepare_cached(THREAD_HAS_PERSON)?.execute([t])?;
    }
    Ok(())
}

/// The user wrote to these addresses (the recipients of a mail of Sent):
/// `address.written_to` becomes 1, and the verdict of each one that did
/// not have it is computed again (rule 5 of `expediteurs.md`).
pub(crate) fn mark_written_to(tx: &Connection, addresses: &[i64]) -> Result<()> {
    for &id in addresses {
        let changed = tx.prepare_cached("UPDATE address SET written_to = 1 WHERE id = ?1 AND written_to = 0")?.execute([id])?;
        if changed > 0 {
            refresh_address(tx, id)?;
        }
    }
    Ok(())
}

/// Computes the verdict on the address `id` again from its counts, stores
/// it, and copies the effective verdict (the user's, else the computed
/// one) into its mails when it changed. Returns the effective verdict.
pub(crate) fn refresh_address(tx: &Connection, id: i64) -> Result<Option<SenderKind>> {
    type Row = (String, Option<String>, Option<String>, bool, Counts, bool);
    let row: Option<Row> = tx
        .prepare_cached(
            "SELECT addr, sender_kind, sender_kind_user, written_to,
                    sender_count, sender_bulk, sender_list, sender_human, sender_client,
                    EXISTS (SELECT 1 FROM account WHERE address = a.addr)
               FROM address a WHERE id = ?1",
        )?
        .query_row([id], |r| {
            let counts = Counts { count: r.get(4)?, bulk: r.get(5)?, list: r.get(6)?, human: r.get(7)?, client: r.get(8)? };
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, counts, r.get(9)?))
        })
        .optional()?;
    let Some((addr, stored, user, written_to, counts, own)) = row else { return Ok(None) };
    let stored = stored.as_deref().and_then(SenderKind::parse);
    let user = user.as_deref().and_then(SenderKind::parse);
    let computed = sender_kind::verdict(&addr, counts, Facts { written_to, own });
    if stored != Some(computed) {
        tx.prepare_cached("UPDATE address SET sender_kind = ?2 WHERE id = ?1")?.execute(params![id, computed.as_str()])?;
    }
    let before = user.or(stored);
    let after = user.unwrap_or(computed);
    if before != Some(after) {
        propagate(tx, id, after)?;
    }
    Ok(Some(after))
}

/// Copies the verdict `kind` of the address `id` into its mails (see
/// `PROPAGATE`).
fn propagate(tx: &Connection, id: i64, kind: SenderKind) -> Result<usize> {
    let n = tx.prepare_cached(PROPAGATE)?.execute(params![id, kind.as_str(), MACHINE])?;
    if n > 0 {
        tx.prepare_cached(ADDRESS_THREADS_HAS_PERSON)?.execute([id])?;
    }
    Ok(n)
}

impl Store {
    /// Forces the verdict on an address (`expediteurs.md`, section 9), or
    /// gives it back to the computation (`None`). The address's mails are
    /// rewritten in the same transaction, and the choice is journaled
    /// (`audit_log`, action `sender_kind`, with the value before, so that
    /// `undo` can restore it). The outcome carries the undo token (`None`
    /// when nothing changed); an unknown address is `NotFound`.
    ///
    /// No event is sent for the mails rewritten: their folders and threads
    /// do not change, only what a filtered list keeps, and the client that
    /// asked reloads its list (an address of 5,000 mails would otherwise
    /// make every client reload every folder).
    pub fn set_sender_kind(&mut self, address_id: i64, kind: Option<SenderKind>, source: Source) -> Result<Outcome> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mark = crate::screening::audit_mark(&tx)?;
        if force_sender_kind(&tx, address_id, kind, None, source)?.is_none() {
            return Err(crate::StoreError::NotFound(format!("adresse {address_id}")));
        }
        let undo = crate::screening::undo_token(&tx, mark)?;
        tx.commit()?;
        Ok(Outcome { undo, ..Outcome::default() })
    }
}

/// Sets `address.sender_kind_user` and journals it (`undo_of`: the
/// `audit_log` row an undo reverts). See [`Store::set_sender_kind`].
pub(crate) fn force_sender_kind(
    tx: &Transaction,
    address_id: i64,
    kind: Option<SenderKind>,
    undo_of: Option<i64>,
    source: Source,
) -> Result<Option<bool>> {
    let before: Option<Option<String>> = tx
        .prepare_cached("SELECT sender_kind_user FROM address WHERE id = ?1")?
        .query_row([address_id], |r| r.get(0))
        .optional()?;
    let Some(before) = before else { return Ok(None) };
    let after = kind.map(SenderKind::as_str);
    if before.as_deref() == after {
        return Ok(Some(false));
    }
    tx.prepare_cached(
        "UPDATE address SET sender_kind_user = ?2, date_sender_kind_modified = unixepoch(),
                date_modified = unixepoch(), modified_by = ?3 WHERE id = ?1",
    )?
    .execute(params![address_id, after, source.as_str()])?;
    tx.prepare_cached(
        "INSERT INTO audit_log (created_by, entity, entity_id, action, detail)
         VALUES (?1, 'address', ?2, 'sender_kind', json_object('before', ?3, 'after', ?4, 'undo_of', ?5))",
    )?
    .execute(params![source.as_str(), address_id, before, after, undo_of])?;
    // The effective verdict before the change: the user's previous choice,
    // else the computed one. Rewritten only if it differs now.
    let computed: Option<String> =
        tx.prepare_cached("SELECT sender_kind FROM address WHERE id = ?1")?.query_row([address_id], |r| r.get(0))?;
    let old = before.or_else(|| computed.clone()).as_deref().and_then(SenderKind::parse);
    let new = kind.or_else(|| computed.as_deref().and_then(SenderKind::parse));
    match new {
        Some(k) if old != Some(k) => {
            propagate(tx, address_id, k)?;
        }
        // Given back to a computation that never ran (no mail counted):
        // compute it now.
        None => {
            refresh_address(tx, address_id)?;
        }
        _ => {}
    }
    Ok(Some(true))
}

/// Counts, verdicts and kinds of a whole mailbox whose messages already
/// have their `sender_signals`, in a few set-based statements: for the fake
/// mailbox (`seed.rs`), which writes its rows directly. The same result as
/// recording each mail ([`record`]): the counts follow the same rule
/// (`Counts::add`, written in SQL), the verdicts come from
/// [`refresh_address`]. `thread.has_person` is left to the caller's
/// `refresh_all_threads_sql`.
pub(crate) fn classify_all(tx: &Connection) -> Result<()> {
    use crate::sender_kind::{CLIENT, NATURE_AUTO, NATURE_BULK, NATURE_HUMAN, NATURE_LIST};
    tx.execute_batch(&format!(
        "UPDATE address SET written_to = 1 WHERE id IN (
             SELECT ma.address_id FROM message_address ma
               JOIN message m ON m.id = ma.message_id JOIN folder f ON f.id = m.folder_id
              WHERE f.role = 'sent' AND ma.role IN ('to', 'cc', 'bcc'));
         WITH c AS (
             SELECT m.from_address_id AS id, count(*) AS n,
                    sum(m.sender_signals & {NATURE_BULK} <> 0) AS bulk,
                    sum(m.sender_signals & {NATURE_LIST} <> 0) AS list,
                    sum(m.sender_signals & {NATURE_HUMAN} <> 0) AS human,
                    sum(m.sender_signals & {CLIENT} <> 0) AS client
               FROM message m JOIN folder f ON f.id = m.folder_id
              WHERE m.from_address_id IS NOT NULL AND m.sender_signals IS NOT NULL
                AND m.sender_signals & {NATURE_AUTO} = 0
                AND coalesce(f.role NOT IN ('sent', 'drafts'), 1)
              GROUP BY m.from_address_id)
         UPDATE address SET sender_count = c.n, sender_bulk = c.bulk, sender_list = c.list,
                sender_human = c.human, sender_client = c.client
           FROM c WHERE address.id = c.id;
         UPDATE message SET sender_kind = 'person'
          WHERE folder_id IN (SELECT id FROM folder WHERE role IN ('sent', 'drafts'));"
    ))?;
    let ids: Vec<i64> = tx
        .prepare("SELECT id FROM address WHERE sender_count > 0 OR written_to = 1")?
        .query_map([], |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    for id in ids {
        refresh_address(tx, id)?;
    }
    Ok(())
}

// ----------------------------------------------------------------
// Catch-up of the mails stored before version 12
// ----------------------------------------------------------------

/// Messages read per batch: the slow part (decompressing the start of each
/// raw file) is done without the store lock, on every core; the writes of
/// a batch (a few statements per mail) hold it under 50 ms.
pub const BATCH_SIZE: usize = 500;

/// What a catch-up did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BackfillReport {
    /// Messages whose clues were read.
    pub read: u64,
    /// Messages without a raw file (or whose file could not be read): they
    /// get no clue (`NATURE_NONE`), and are counted as such.
    pub missing_raw: u64,
    pub seconds: f64,
}

/// Messages whose clues are not read yet: what [`backfill`] would do.
/// Counted on `idx_message_sender_pending`, which holds only them.
pub fn pending(store: &Store) -> Result<i64> {
    Ok(store.conn.query_row("SELECT count(*) FROM message WHERE sender_signals IS NULL", [], |r| r.get(0))?)
}

/// Reads the clues of every message stored before version 12
/// (`expediteurs.md`, section 10), by batches of `batch_size`, in the order
/// of their ids. Each batch is one transaction that does for its mails
/// what the insertion does for a new one ([`record`]): a crash loses at
/// most a batch, which is read again at the next start, and the counts stay
/// exact (a mail is counted in the transaction that marks it read).
///
/// `on_batch(done, total)` is called after each batch, store lock released.
pub fn backfill(store: &Mutex<Store>, raw: &RawStore, batch_size: usize, on_batch: &mut dyn FnMut(u64, u64)) -> Result<BackfillReport> {
    let started = Instant::now();
    let mut report = BackfillReport::default();
    let total = pending(&store.lock().unwrap())? as u64;
    let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    let mut after = 0i64;
    loop {
        // id, raw hash, sender, written by the user.
        type Todo = (i64, Option<Vec<u8>>, Option<i64>, bool);
        let todo: Vec<Todo> = {
            let guard = store.lock().unwrap();
            guard
                .conn
                .prepare_cached(
                    "SELECT m.id, m.raw_hash, m.from_address_id, coalesce(f.role IN ('sent', 'drafts'), 0)
                       FROM message m JOIN folder f ON f.id = m.folder_id
                      WHERE m.sender_signals IS NULL AND m.id > ?1 ORDER BY m.id LIMIT ?2",
                )?
                .query_map(params![after, batch_size.max(1) as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                .collect::<std::result::Result<_, _>>()?
        };
        let Some(last) = todo.last() else { break };
        after = last.0;
        // Read the headers in parallel, without the store lock.
        // Rust note: `std::thread::scope` lets the threads borrow `todo` and
        // `raw`; it waits for them all before returning.
        let signals: Vec<Option<i64>> = std::thread::scope(|scope| {
            let chunk = todo.len().div_ceil(workers).max(1);
            let handles: Vec<_> = todo
                .chunks(chunk)
                .map(|part| scope.spawn(move || part.iter().map(|t| read_signals(raw, t.1.as_deref())).collect::<Vec<_>>()))
                .collect();
            handles.into_iter().flat_map(|h| h.join().expect("sender backfill worker panicked")).collect()
        });
        {
            let mut guard = store.lock().unwrap();
            let tx = guard.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            for ((id, _, from, own), s) in todo.iter().zip(signals) {
                // Inserted meanwhile with its clues: already recorded.
                let still: bool = tx
                    .prepare_cached("SELECT sender_signals IS NULL FROM message WHERE id = ?1")?
                    .query_row([id], |r| r.get(0))
                    .optional()?
                    .unwrap_or(false);
                if !still {
                    continue;
                }
                match s {
                    Some(_) => report.read += 1,
                    None => report.missing_raw += 1,
                }
                record(&tx, *id, *from, s.unwrap_or(NATURE_NONE), *own)?;
            }
            tx.commit()?;
        }
        on_batch(report.read + report.missing_raw, total);
        // Let a waiting request take the lock: a `Mutex` is not fair
        // (`reindex.rs`, `RANGE`).
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    report.seconds = started.elapsed().as_secs_f64();
    Ok(report)
}

/// The clues of a stored message, from the start of its raw file; `None`
/// without a readable file.
fn read_signals(raw: &RawStore, hash: Option<&[u8]>) -> Option<i64> {
    let hash: Hash = hash?.try_into().ok()?;
    let header = raw.read_header(&hash).ok()?;
    Some(crate::ingest::parse_headers(&header).sender_signals)
}
