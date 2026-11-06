//! Reinterpretation of stored messages after an improvement of the parser
//! (`stockage-brut.md`, section 8, "Reconstruction (`reindex`)").
//!
//! `message_body` is an interpretation of the raw message, tagged with the
//! version of the parser that produced it (`ingest::PARSER_VERSION`). When
//! the parser improves, the core starts this reindex in the background: it
//! reads the raw file of every body of an older version again, parses it,
//! and rewrites what the parser produces from the body:
//! - `message_body` (text, sanitized HTML, remote content, tracking
//!   pixels), created if the first parse failed;
//! - `message.snippet`;
//! - the `body` column of the search index;
//! - `message_unsubscribe`;
//! - the name of an address that has none (`schema.sql`, table `address`):
//!   the mail's sender and recipients give theirs, as at ingestion.
//!
//! A version that can be applied to what is stored does without the raw
//! files: version 7 (entities encoded twice) corrects the stored text and
//! HTML directly (`repair_stored`). And a version declares which bodies it
//! cannot change, marked without being read again (`skip_unchanged`;
//! `skip_without_remote_images` for version 8, which keeps the URL of remote
//! images: the stored HTML no longer has it, so the others are read again).
//!
//! Not rewritten: the headers already interpreted (threads, addresses,
//! references), the attachments (their ids and extracted files are in use),
//! the authentication verdict, folders and flags. The raw file is never
//! modified.
//!
//! Like the import, the work runs in batches: the slow part (reading,
//! decompressing and parsing) is done without the store lock, on every
//! core; only the writes of a batch hold it, in one transaction, so the app
//! keeps being answered.

use std::sync::Mutex;
use std::time::Instant;

use rusqlite::{OptionalExtension, TransactionBehavior, params};

use crate::ingest::{self, PARSER_VERSION, ParsedMessage};
use crate::raw::{Hash, RawStore};
use crate::{Result, Store};

/// Messages reparsed per batch. The writes of a batch hold the store lock
/// (rewriting a row of the search index is the slow part): with 200, a list
/// asked by the app meanwhile waited up to 0.4 s; with 50, under 0.1 s.
pub const BATCH_SIZE: usize = 50;

/// Message ids per step of the preparation (marking the bodies that cannot
/// change, listing the others). Done in one statement on a large mailbox,
/// marking read every body with the lock held, and the app waited 49 s for
/// its first list; 500 ids at a time keep each step under 0.1 s.
const RANGE: i64 = 500;

/// Text written per transaction, in bytes. Rewriting the body of a mail of
/// 10 MB (a log sent as text) and its row of the search index takes about
/// 1 s; 50 such mails in one transaction held the lock 40 s. The writes of
/// a batch are committed whenever this much text has been written, the lock
/// released in between: a huge mail is then written alone.
const WRITE_BYTES: usize = 2_000_000;

/// What a reindex did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReindexReport {
    /// Bodies whose raw file was read and parsed again.
    pub reparsed: u64,
    /// Bodies brought to the current version without being reparsed: the
    /// new versions are known to give the same result for them, or a
    /// correction of what is stored is enough (`skip_unchanged`,
    /// `repair_stored`, `skip_without_remote_images`, `skip_without_html`).
    pub unchanged: u64,
    /// Bodies corrected from what is stored, without reading the raw file
    /// (`repair_stored`). Those a later version reads again anyway (an
    /// HTML body, for version 9) are counted here and in `reparsed`; the
    /// others here and in `unchanged`.
    pub repaired: u64,
    /// Messages whose raw file is missing or corrupt: left as they were.
    pub missing_raw: u64,
    /// Messages the parser still cannot read.
    pub parse_errors: u64,
    /// Names given to addresses that had none.
    pub names_filled: u64,
    pub seconds: f64,
}

/// Messages without a body (their first parse failed): read from the
/// index `idx_message_body_parser`, which holds every body's id. The
/// condition `parser_version > -1` makes SQLite read that index: with a
/// plain `IN (SELECT message_id FROM message_body)` it looks every message
/// up in the table itself, whose pages hold the whole text of the mails
/// (gigabytes to read on a large mailbox).
const WITHOUT_BODY: &str = "SELECT m.id, m.raw_hash FROM message m
    WHERE m.raw_hash IS NOT NULL
      AND m.id NOT IN (SELECT message_id FROM message_body WHERE parser_version > -1)";

/// Number of messages whose body is older than the current parser, or
/// missing: what `reindex` would do (0 means nothing to do). Read from
/// indexes only, in a fraction of a second: never from the bodies (a join
/// with `message_body` read all of them, 30 s on a large mailbox).
pub fn pending(store: &Store) -> Result<i64> {
    let older: i64 = store.conn.query_row(
        "SELECT count(*) FROM message_body WHERE parser_version < ?1",
        [PARSER_VERSION],
        |r| r.get(0),
    )?;
    let missing: i64 = store.conn.query_row(&format!("SELECT count(*) FROM ({WITHOUT_BODY})"), [], |r| r.get(0))?;
    Ok(older + missing)
}

/// Reinterprets every message whose body is older than the current parser.
/// `on_batch(done, total)` is called after each batch, store lock released.
pub fn reindex(store: &Mutex<Store>, raw: &RawStore, batch_size: usize, on_batch: &mut dyn FnMut(u64, u64)) -> Result<ReindexReport> {
    let started = Instant::now();
    let mut report = ReindexReport::default();
    // First mark the bodies that cannot change (`skip_unchanged`), by
    // ranges of ids: this reads their text, gigabytes on a large mailbox,
    // so the lock is released between two ranges (`RANGE`).
    let max_id: i64 = store.lock().unwrap().conn.query_row("SELECT coalesce(max(id), 0) FROM message", [], |r| r.get(0))?;
    let mut start = 0;
    while start <= max_id {
        let end = start + RANGE;
        {
            let guard = store.lock().unwrap();
            // Each step brings what it can one version up, without the raw
            // files; a body goes through as many as apply, and stops where
            // a version needs its raw file (read below). What reached the
            // current version is counted once, by difference, whatever the
            // path it took.
            let current = || -> Result<i64> {
                Ok(guard.conn.prepare_cached(
                    "SELECT count(*) FROM message_body WHERE message_id >= ?1 AND message_id < ?2 AND parser_version = ?3",
                )?
                .query_row(params![start, end, PARSER_VERSION], |r| r.get(0))?)
            };
            let before = current()?;
            skip_unchanged(&guard, start, end)?;
            let (_, repaired) = repair_stored(&guard, start, end)?;
            report.repaired += repaired;
            skip_without_remote_images(&guard, start, end)?;
            skip_without_html(&guard, start, end)?;
            report.unchanged += (current()? - before) as u64;
        }
        start = end;
        // Let a waiting request take the lock. A `Mutex` is not fair: a
        // thread that releases it and asks again at once usually gets it
        // back, and the app's requests would wait for the end of this loop.
        // The batches below need no pause: their reading and parsing,
        // without the lock, leave room.
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    // Then the work list: ids and raw hashes only (a few megabytes for a
    // whole mailbox), read from the index of the parser versions, never
    // from the bodies. A message imported meanwhile is already current.
    let todo: Vec<(i64, Vec<u8>)> = {
        let guard = store.lock().unwrap();
        let mut todo: Vec<(i64, Vec<u8>)> = guard
            .conn
            .prepare(
                "SELECT m.id, m.raw_hash FROM message_body b JOIN message m ON m.id = b.message_id
                  WHERE b.parser_version < ?1 AND m.raw_hash IS NOT NULL",
            )?
            .query_map([PARSER_VERSION], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?;
        // Rust note: the statement must outlive the rows it yields, hence
        // its own variable rather than a temporary.
        let mut stmt = guard.conn.prepare(WITHOUT_BODY)?;
        for row in stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))? {
            todo.push(row?);
        }
        todo.sort_unstable_by_key(|(id, _)| *id);
        todo
    };
    let total = todo.len() as u64;
    let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    let mut done = 0;
    for batch in todo.chunks(batch_size.max(1)) {
        // Read and parse in parallel, without the store lock.
        // Rust note: `std::thread::scope` lets the threads borrow `batch` and
        // `raw`; it waits for them all before returning.
        let parsed: Vec<(i64, Option<ParsedMessage>)> = std::thread::scope(|scope| {
            let chunk = batch.len().div_ceil(workers);
            let handles: Vec<_> = batch
                .chunks(chunk.max(1))
                .map(|part| scope.spawn(move || part.iter().map(|(id, hash)| (*id, parse_raw(raw, hash))).collect::<Vec<_>>()))
                .collect();
            handles.into_iter().flat_map(|h| h.join().expect("reindex worker panicked")).collect()
        });
        let mut parsed = parsed.into_iter().peekable();
        while parsed.peek().is_some() {
            let mut guard = store.lock().unwrap();
            let tx = guard.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let mut written = 0;
            // Rust note: `by_ref` borrows the iterator, so the loop can stop
            // early and the next transaction go on from there.
            for (id, parsed) in parsed.by_ref() {
                match parsed {
                    None => report.missing_raw += 1,
                    Some(m) if !m.parsed => report.parse_errors += 1,
                    Some(m) => {
                        let w = write(&tx, id, &m)?;
                        report.names_filled += w.names_filled;
                        written += w.bytes;
                        report.reparsed += 1;
                    }
                }
                if written >= WRITE_BYTES {
                    break;
                }
            }
            tx.commit()?;
            drop(guard);
            if parsed.peek().is_some() {
                // Let a waiting request take the lock (see `RANGE`).
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
        done += batch.len() as u64;
        on_batch(done, total);
    }
    report.seconds = started.elapsed().as_secs_f64();
    Ok(report)
}

/// Marks the version 5 bodies the parser of version 6 would leave
/// unchanged as version 6, so that only the others are read again (those
/// marked then go through `repair_stored`, which brings them to version 7).
/// Each version that changes the parser says which bodies it can change;
/// by default, all of them.
///
/// Version 6 only changes a text part holding an HTML entity or HTML
/// markup (`ingest.rs`, `parse`; `html_text::has_entity`,
/// `has_html_markup`). A version 5 body whose text holds none of them was
/// either a text part version 6 keeps as is, or the conversion of an HTML
/// part, which is unchanged. The conditions below are wider than those
/// marks (any `&…;`, any `</`…; LIKE ignores ASCII case, as
/// `has_html_markup` does), so no body that may change is skipped. On a
/// real mailbox this leaves about one body in ten to read again.
fn skip_unchanged(store: &Store, start: i64, end: i64) -> Result<u64> {
    let n = store.conn.prepare_cached(
        "UPDATE message_body SET parser_version = 6
          WHERE message_id >= ?1 AND message_id < ?2 AND parser_version = 5
            AND (text_plain IS NULL
                 OR (text_plain NOT LIKE '%&%;%' AND text_plain NOT LIKE '%</%' AND text_plain NOT LIKE '%<br%'
                     AND text_plain NOT LIKE '% style=\"%' AND text_plain NOT LIKE '% target=\"%'))",
    )?
    .execute(params![start, end])?;
    Ok(n as u64)
}

/// Brings the version 6 bodies of the ids `start..end` to version 7 from
/// what is stored, without reading the raw files: version 7 only removes
/// the entities a sender encoded twice (`&amp;nbsp;`), which the stored text
/// and HTML show as they are (`html_text::decode_all_entities`,
/// `undouble_entities`). Returns (bodies only marked, bodies corrected).
///
/// Every version 6 body of the range is read, text and HTML (the ranges are
/// small, `RANGE`); only a body that changes is written, with its snippet
/// and its row of the search index.
fn repair_stored(store: &Store, start: i64, end: i64) -> Result<(u64, u64)> {
    debug_assert!(PARSER_VERSION >= 7, "repair_stored brings bodies from version 6 to 7");
    let rows: Vec<(i64, Option<String>, Option<String>)> = store
        .conn
        .prepare_cached(
            "SELECT message_id, text_plain, html_sanitized FROM message_body
              WHERE message_id >= ?1 AND message_id < ?2 AND parser_version = 6",
        )?
        .query_map(params![start, end], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<std::result::Result<_, _>>()?;
    let (mut marked, mut repaired) = (0, 0);
    for (id, text, html) in rows {
        let new_text = text.as_deref().map(crate::html_text::decode_all_entities);
        let new_html = html.as_deref().map(crate::html_text::undouble_entities);
        let text_changed = matches!(new_text, Some(std::borrow::Cow::Owned(_)));
        let html_changed = matches!(new_html, Some(std::borrow::Cow::Owned(_)));
        if text_changed {
            let t = new_text.as_deref().unwrap_or("");
            store
                .conn
                .prepare_cached("UPDATE message_body SET text_plain = ?2 WHERE message_id = ?1")?
                .execute(params![id, t])?;
            let snippet = Some(ingest::snippet(t)).filter(|s| !s.is_empty());
            store.conn.prepare_cached("UPDATE message SET snippet = ?2 WHERE id = ?1")?.execute(params![id, snippet])?;
            store.conn.prepare_cached("UPDATE message_fts SET body = ?2 WHERE rowid = ?1")?.execute(params![id, t])?;
        }
        if html_changed {
            store
                .conn
                .prepare_cached("UPDATE message_body SET html_sanitized = ?2 WHERE message_id = ?1")?
                .execute(params![id, new_html.as_deref()])?;
        }
        store
            .conn
            .prepare_cached("UPDATE message_body SET parser_version = 7 WHERE message_id = ?1")?
            .execute([id])?;
        if text_changed || html_changed { repaired += 1 } else { marked += 1 }
    }
    Ok((marked, repaired))
}

/// Marks the version 7 bodies without remote image as version 8, so that
/// only the others are read again. Version 8 only changes the sources of
/// remote images (`remote_images.rs`); a body without any
/// (`has_remote_content = 0`, which version 7 set for any `<img>` with an
/// `http:`, `https:` or `//` source, the same test as version 8) comes out
/// the same, with no tracking pixel. On a real mailbox this leaves about
/// two bodies in three to read again: most HTML mails have remote images.
/// Returns how many were marked.
fn skip_without_remote_images(store: &Store, start: i64, end: i64) -> Result<u64> {
    let n = store
        .conn
        .prepare_cached(
            "UPDATE message_body SET parser_version = 8
              WHERE message_id >= ?1 AND message_id < ?2 AND parser_version = 7 AND has_remote_content = 0",
        )?
        .execute(params![start, end])?;
    Ok(n as u64)
}

/// Marks the bodies of version 8 or later without HTML as current, so
/// that only the HTML mails are read again. Versions 9 (layout attributes,
/// `ingest::LAYOUT_ATTRIBUTES`), 10 (the sender's CSS, `css.rs`) and 11
/// (the hidden elements of the HTML out of its text, `html_text::convert`)
/// only change what is made from the HTML part; a body without one comes
/// out the same.
/// Returns how many were marked.
fn skip_without_html(store: &Store, start: i64, end: i64) -> Result<u64> {
    let n = store
        .conn
        .prepare_cached(
            "UPDATE message_body SET parser_version = ?3
              WHERE message_id >= ?1 AND message_id < ?2 AND parser_version >= 8 AND parser_version < ?3
                AND html_sanitized IS NULL",
        )?
        .execute(params![start, end, PARSER_VERSION])?;
    Ok(n as u64)
}

/// The raw message `hash`, parsed; `None` if the file is missing or
/// corrupt.
fn parse_raw(raw: &RawStore, hash: &[u8]) -> Option<ParsedMessage> {
    let hash: Hash = hash.try_into().ok()?;
    let bytes = raw.read(&hash).ok()?;
    Some(ingest::parse(&bytes))
}

/// What `write` did.
struct Written {
    /// Bytes of text written (0 when the body was already right).
    bytes: usize,
    /// Addresses that received a name.
    names_filled: u64,
}

/// Writes what the parser produced for the message `id`.
///
/// The body and its row of the search index are only rewritten when the
/// text or the HTML changed: the bodies reread are chosen widely (any
/// `&…;`, see `skip_unchanged`), many come out the same, and rewriting a row
/// of the search index makes FTS5 tokenize the old text and the new one.
fn write(tx: &rusqlite::Transaction, id: i64, m: &ParsedMessage) -> Result<Written> {
    // `IS` compares with NULL too (two NULLs are the same).
    let same: Option<bool> = tx
        .prepare_cached("SELECT text_plain IS ?2 AND html_sanitized IS ?3 FROM message_body WHERE message_id = ?1")?
        .query_row(params![id, m.text, m.html], |r| r.get(0))
        .optional()?;
    let mut written = Written { bytes: 0, names_filled: 0 };
    if same == Some(true) {
        tx.prepare_cached(
            "UPDATE message_body SET parser_version = ?2, has_remote_content = ?3, tracking_pixels = ?4
              WHERE message_id = ?1",
        )?
        .execute(params![id, PARSER_VERSION, m.has_remote_content, m.tracking_pixels])?;
    } else {
        written.bytes = m.text.as_ref().map_or(0, String::len) + m.html.as_ref().map_or(0, String::len);
        write_body(tx, id, m)?;
    }
    tx.prepare_cached("DELETE FROM message_unsubscribe WHERE message_id = ?1")?.execute([id])?;
    if let Some(u) = &m.unsubscribe {
        tx.prepare_cached(
            "INSERT INTO message_unsubscribe (message_id, url, mailto, one_click, source) VALUES (?1, ?2, ?3, ?4, ?5)",
        )?
        .execute(params![id, u.url, u.mailto, u.one_click, u.source.as_str()])?;
    }
    // Names of addresses that have none, as at ingestion. The condition is
    // in SQL: a name set by the user is never replaced.
    for a in m.from.iter().chain(&m.to).chain(&m.cc) {
        if let Some(name) = &a.name {
            written.names_filled += tx
                .prepare_cached(
                    "UPDATE address SET name = ?2, date_modified = unixepoch(), modified_by = 'core'
                      WHERE addr = ?1 AND coalesce(name, '') = ''",
                )?
                .execute(params![a.addr, name])? as u64;
        }
    }
    Ok(written)
}

/// Rewrites the body of the message `id`, its snippet and the body of its
/// row of the search index.
fn write_body(tx: &rusqlite::Transaction, id: i64, m: &ParsedMessage) -> Result<()> {
    tx.prepare_cached(
        "INSERT INTO message_body (message_id, text_plain, html_sanitized, has_remote_content, tracking_pixels,
                                   parser_version)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (message_id) DO UPDATE SET
            text_plain = excluded.text_plain, html_sanitized = excluded.html_sanitized,
            has_remote_content = excluded.has_remote_content, tracking_pixels = excluded.tracking_pixels,
            parser_version = excluded.parser_version",
    )?
    .execute(params![id, m.text, m.html, m.has_remote_content, m.tracking_pixels, PARSER_VERSION])?;
    tx.prepare_cached("UPDATE message SET snippet = ?2 WHERE id = ?1")?.execute(params![id, m.snippet])?;
    // The search index has one row per message (`import.rs`); only its body
    // comes from the text.
    let in_index: Option<i64> =
        tx.prepare_cached("SELECT rowid FROM message_fts WHERE rowid = ?1")?.query_row([id], |r| r.get(0)).optional()?;
    if in_index.is_some() {
        tx.prepare_cached("UPDATE message_fts SET body = ?2 WHERE rowid = ?1")?
            .execute(params![id, m.text.as_deref().unwrap_or("")])?;
    }
    Ok(())
}
