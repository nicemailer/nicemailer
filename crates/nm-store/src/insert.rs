//! The one way a message enters the database, shared by the mbox import
//! and the IMAP synchronization (`plan-imap.md`, phase 3).
//!
//! Two steps, because IMAP gives the headers long before the body: a new
//! mail must appear in the list, be threaded and screened at once, while
//! its body (and 200,000 others, after a new account) downloads in the
//! background.
//!
//! 1. [`Inserter::insert`] with the headers: the `message` row
//!    (`body_state = 0`, no `raw_hash`), its addresses and references, its
//!    thread, its search entry (subject, sender, recipients; no body yet).
//! 2. [`Inserter::complete`] when the whole message is stored in the raw
//!    store: `raw_blob`, `message_body`, attachments, how to unsubscribe,
//!    snippet, the body in the search index, `body_state = 1`.
//!
//! The mbox import has the whole message at once: it calls `insert` with
//! the body, which does both steps in one.
//!
//! # Threading
//!
//! A message joins the thread of any message of the same account that it
//! references, that references it, that references one of the messages it
//! references, or that has the same `Message-ID`. Messages arrive in any
//! order, so a reply can come before the message it answers: the parent
//! then finds the reply through `message_reference`.
//!
//! The third link is JWZ's "phantom parent": two replies to a message the
//! database does not have (deleted long ago, or in a folder NiceMailer
//! does not synchronize, `dossiers.md`) are one conversation, which they
//! say themselves by citing the same message. Without it they would stay
//! apart until the parent arrives, if ever.
//!
//! Every link is between two messages present, whichever came first: the
//! threads depend on which messages are there, never on their order.
//! When a message links several threads, they are merged into the oldest
//! one. This is the linking part of the JWZ algorithm (`modele de
//! donnees.md`); messages without any reference are not grouped by subject,
//! which would wrongly merge unrelated mails such as monthly invoices with
//! the same subject.
//!
//! # Transactions
//!
//! The caller opens the transaction, inserts any number of messages, then
//! hands the transaction to [`Inserter::commit`]: it recomputes what the
//! batch changed (thread subjects and folders, folder counters) once for
//! the whole batch, and commits. A batch is one transaction, so a reader
//! never sees half of it.

use std::collections::{BTreeSet, HashMap};

use nm_core::Source;
use rusqlite::{OptionalExtension, Transaction, params};

use crate::ingest::{PARSER_VERSION, ParsedAddress, ParsedMessage};
use crate::raw::Written;
use crate::{ChangeSet, Result, refresh_folder_counts, refresh_thread};

/// The IMAP flags a message arrives with. An mbox import marks everything
/// as read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Flags {
    pub seen: bool,
    pub flagged: bool,
    pub answered: bool,
    pub draft: bool,
    pub deleted: bool,
}

/// A message to insert: where it goes, what the server says of it, and its
/// headers.
#[derive(Debug, Clone, Copy)]
pub struct NewMessage<'a> {
    pub account_id: i64,
    pub folder_id: i64,
    /// UID on the server; `None` in an archive account.
    pub uid: Option<u32>,
    /// Reception date (IMAP INTERNALDATE), Unix seconds: the sort key.
    pub date_received: i64,
    /// Size of the whole message (RFC822.SIZE), if known.
    pub size: Option<u64>,
    /// CONDSTORE modification sequence, if the server has one.
    pub modseq: Option<u64>,
    pub flags: Flags,
    /// The headers ([`crate::ingest::parse_headers`]), or the whole message
    /// parsed ([`crate::ingest::parse`]): only the header fields are read.
    pub headers: &'a ParsedMessage,
}

/// The whole message, stored in the raw store, and parsed.
#[derive(Debug, Clone, Copy)]
pub struct Body<'a> {
    pub written: &'a Written,
    pub parsed: &'a ParsedMessage,
}

/// What [`Inserter::complete`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Completed {
    /// The message is complete now.
    Done,
    /// It already was (the body was downloaded twice): nothing changed.
    Already,
    /// No such message any more (deleted while its body was downloading):
    /// nothing changed. The raw file stays, harmless; the garbage collector
    /// removes it (`stockage-brut.md`, section 7).
    Missing,
}

/// Inserts messages, one transaction after another. Keeps a cache of the
/// address ids it has seen: most mails involve the same few hundred people,
/// so it saves most of the address lookups.
pub struct Inserter {
    source: Source,
    /// `address.imported_from` of the addresses it creates: `mbox`,
    /// `imap_initial`, or `None` for a mail that simply arrived.
    imported_from: Option<&'static str>,
    /// Address ids by lowercase address, with whether the address has a
    /// name; valid across transactions, since they were committed.
    addresses: HashMap<String, (i64, bool)>,
    /// The same, for the transaction in progress: merged into `addresses`
    /// by [`Inserter::commit`], dropped by [`Inserter::abandon`]. An id
    /// created in a transaction that was rolled back does not exist, and
    /// must never be used again.
    pending: HashMap<String, (i64, bool)>,
    /// Threads the transaction in progress touched.
    threads: BTreeSet<i64>,
}

impl Inserter {
    pub fn new(source: Source, imported_from: Option<&'static str>) -> Inserter {
        Inserter { source, imported_from, addresses: HashMap::new(), pending: HashMap::new(), threads: BTreeSet::new() }
    }

    /// Inserts a message and returns its id. With `body`, the message is
    /// complete at once (both steps); without, its body is to come
    /// ([`Inserter::complete`]).
    pub fn insert(&mut self, tx: &Transaction, m: &NewMessage, body: Option<Body>, changes: &mut ChangeSet) -> Result<i64> {
        let h = m.headers;
        let from_id = match &h.from {
            Some(a) => Some(self.address_id(tx, a)?),
            None => None,
        };
        let thread_id = self.thread_for(tx, m.account_id, h, changes)?;
        self.threads.insert(thread_id);
        changes.folders.insert(m.folder_id);

        // With the body, the raw file's row first: `message.raw_hash`
        // references it.
        if let Some(b) = &body {
            insert_raw_blob(tx, b.written)?;
        }
        let (snippet, has_attachments, raw_hash, size) = match &body {
            Some(b) => (b.parsed.snippet.as_deref(), b.parsed.has_attachments(), Some(&b.written.hash[..]), Some(b.written.size)),
            None => (None, false, None, m.size),
        };
        tx.prepare_cached(
            "INSERT INTO message (account_id, folder_id, uid, thread_id, message_id_hdr, in_reply_to, subject,
                from_address_id, from_name, from_addr, date_sent, date_received, size, snippet,
                is_seen, is_flagged, is_answered, is_draft, is_deleted, has_attachments, auth_result, modseq,
                body_state, raw_hash, created_by, modified_by)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                     ?15, ?16, ?17, ?18, ?19, ?20, 'unknown', ?21, ?22, ?23, ?24, ?24)",
        )?
        .execute(params![
            m.account_id,
            m.folder_id,
            m.uid,
            thread_id,
            h.message_id,
            h.in_reply_to,
            h.subject,
            from_id,
            h.from.as_ref().and_then(|a| a.name.as_deref()),
            h.from.as_ref().map(|a| a.addr.as_str()),
            h.date_sent,
            m.date_received,
            // The bytes stored are the truth, whatever RFC822.SIZE said.
            size.map(|s| s as i64),
            snippet,
            m.flags.seen,
            m.flags.flagged,
            m.flags.answered,
            m.flags.draft,
            m.flags.deleted,
            has_attachments,
            m.modseq.map(|s| s as i64),
            // `body_state = 1` and `raw_hash` go together: a complete
            // message has its raw file (`schema.sql`).
            raw_hash.is_some(),
            raw_hash,
            self.source.as_str(),
        ])?;
        let message_id = tx.last_insert_rowid();

        for (pos, r) in h.references.iter().enumerate() {
            tx.prepare_cached("INSERT OR IGNORE INTO message_reference (message_id, position, ref_msgid) VALUES (?1, ?2, ?3)")?
                .execute(params![message_id, pos as i64, r])?;
        }
        let mut recipients = String::new();
        for (role, list) in [("to", &h.to), ("cc", &h.cc), ("bcc", &h.bcc), ("reply_to", &h.reply_to)] {
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
        // Search index: one row per message, written with everything known
        // in one statement (no trigger, see the comment above `message_fts`
        // in schema.sql). The body, if known; else filled in by `complete`.
        let sender = h.from.as_ref().map(|a| format!("{} {}", a.name.as_deref().unwrap_or(""), a.addr)).unwrap_or_default();
        let text = body.as_ref().and_then(|b| b.parsed.text.as_deref()).unwrap_or("");
        tx.prepare_cached("INSERT INTO message_fts (rowid, subject, sender, recipients, body) VALUES (?1, ?2, ?3, ?4, ?5)")?
            .execute(params![message_id, h.subject, sender.trim(), recipients.trim_end(), text])?;

        if let Some(b) = body {
            write_body(tx, message_id, b.parsed)?;
        }
        // No `changes.messages` entry: a new message is not a change of a
        // message a client shows. The folder counters say it is there
        // (`changes.folders`), and the synchronization announces it with
        // its own event (`MessageArrived`, phase 4). An import of 50,000
        // mails then sends one event per folder and batch, not 50,000.
        Ok(message_id)
    }

    /// Second step: the whole message of `message_id` is in the raw store.
    pub fn complete(&mut self, tx: &Transaction, message_id: i64, body: Body, changes: &mut ChangeSet) -> Result<Completed> {
        let row: Option<(i64, Option<i64>)> = tx
            .prepare_cached("SELECT body_state, thread_id FROM message WHERE id = ?1")?
            .query_row([message_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        let Some((state, thread_id)) = row else { return Ok(Completed::Missing) };
        if state == 1 {
            return Ok(Completed::Already);
        }
        let m = body.parsed;
        insert_raw_blob(tx, body.written)?;
        tx.prepare_cached(
            "UPDATE message SET raw_hash = ?2, body_state = 1, size = ?3, snippet = ?4, has_attachments = ?5 WHERE id = ?1",
        )?
        .execute(params![message_id, &body.written.hash[..], body.written.size as i64, m.snippet, m.has_attachments()])?;
        if let Some(text) = m.text.as_deref().filter(|t| !t.is_empty()) {
            tx.prepare_cached("UPDATE message_fts SET body = ?2 WHERE rowid = ?1")?.execute(params![message_id, text])?;
        }
        write_body(tx, message_id, m)?;
        // A client showing this message wants its body now.
        changes.messages.insert((message_id, thread_id));
        Ok(Completed::Done)
    }

    /// Ends the batch: recomputes what the messages inserted changed
    /// (subject and folders of each thread touched, counters of each folder
    /// in `changes`), commits, and keeps the new address ids for the next
    /// batches.
    ///
    /// If the batch has bodies, call [`crate::raw::RawStore::barrier`]
    /// first: the raw files must be on stable storage before the rows that
    /// reference them are committed (`raw.rs`, "Durability on macOS"). It
    /// is left to the caller so that it can flush before taking the lock
    /// of the store, as the mbox import does.
    ///
    /// Rust note: `tx: Transaction` is taken by value: once committed, the
    /// transaction cannot be used by mistake, the compiler forbids it.
    pub fn commit(&mut self, tx: Transaction, changes: &mut ChangeSet) -> Result<()> {
        let result = self.finish(&tx, changes).and_then(|_| tx.commit().map_err(Into::into));
        match result {
            Ok(()) => {
                // `drain` empties `pending` and moves its entries over.
                self.addresses.extend(self.pending.drain());
                Ok(())
            }
            Err(e) => {
                self.abandon();
                Err(e)
            }
        }
    }

    /// The transaction was rolled back (dropped without `commit`): forgets
    /// what it created.
    pub fn abandon(&mut self) {
        self.pending.clear();
        self.threads.clear();
    }

    fn finish(&mut self, tx: &Transaction, changes: &mut ChangeSet) -> Result<()> {
        // `std::mem::take` leaves an empty set in `self.threads` and gives
        // us the old one.
        for t in std::mem::take(&mut self.threads) {
            // A thread may have been merged into another after a message
            // joined it: then it is gone, nothing to refresh.
            let exists = tx.prepare_cached("SELECT 1 FROM thread WHERE id = ?1")?.exists([t])?;
            if !exists {
                continue;
            }
            // The subject of a thread is the one of its first message.
            tx.prepare_cached(
                "UPDATE thread SET subject = coalesce(
                    (SELECT subject FROM message WHERE thread_id = ?1 ORDER BY date_received, id LIMIT 1), '')
                  WHERE id = ?1",
            )?
            .execute([t])?;
            for f in refresh_thread(tx, t)? {
                changes.folders.insert(f);
            }
        }
        for f in &changes.folders {
            refresh_folder_counts(tx, *f)?;
        }
        Ok(())
    }

    /// Id of the `address` row, created if needed. An address that already
    /// exists keeps its name and its `imported_from` (`import-initial.md`,
    /// section 7), except that an address without a name gets the one this
    /// message gives it: the name is "filled in at the first encounter that
    /// has one" (`schema.sql`, table `address`). A recipient added to a
    /// group, or a sender decided on before any of their mail with a name
    /// arrived, would otherwise stay nameless forever, and the From / To
    /// filters look names up there.
    fn address_id(&mut self, tx: &Transaction, a: &ParsedAddress) -> Result<i64> {
        if let Some(&(id, named)) = self.pending.get(&a.addr).or_else(|| self.addresses.get(&a.addr)) {
            if !named && a.name.is_some() {
                self.fill_name(tx, id, a)?;
            }
            return Ok(id);
        }
        let existing: Option<(i64, Option<String>)> = tx
            .prepare_cached("SELECT id, name FROM address WHERE addr = ?1")?
            .query_row([&a.addr], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        let (id, named) = match existing {
            Some((id, name)) if name.as_deref().is_some_and(|n| !n.is_empty()) => (id, true),
            Some((id, _)) => {
                if a.name.is_some() {
                    self.fill_name(tx, id, a)?;
                }
                (id, a.name.is_some())
            }
            None => {
                tx.prepare_cached(
                    "INSERT INTO address (addr, name, imported_from, created_by, modified_by) VALUES (?1, ?2, ?3, ?4, ?4)",
                )?
                .execute(params![a.addr, a.name, self.imported_from, self.source.as_str()])?;
                (tx.last_insert_rowid(), a.name.is_some())
            }
        };
        self.pending.insert(a.addr.clone(), (id, named));
        Ok(id)
    }

    /// Gives the address `id`, which has no name, the name of `a`. The
    /// condition on the name is repeated in SQL so that a name set in the
    /// meantime (by the user, between two batches) is never overwritten.
    fn fill_name(&mut self, tx: &Transaction, id: i64, a: &ParsedAddress) -> Result<()> {
        tx.prepare_cached(
            "UPDATE address SET name = ?2, date_modified = unixepoch(), modified_by = 'core'
              WHERE id = ?1 AND coalesce(name, '') = ''",
        )?
        .execute(params![id, a.name])?;
        self.pending.insert(a.addr.clone(), (id, true));
        Ok(())
    }

    /// Finds the thread of a message, creating or merging threads as needed
    /// (see "Threading" above).
    fn thread_for(&mut self, tx: &Transaction, account_id: i64, m: &ParsedMessage, changes: &mut ChangeSet) -> Result<i64> {
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
        {
            // Messages that cite what this one cites, and messages that cite
            // this one.
            let mut referencing = tx.prepare_cached(
                "SELECT DISTINCT m.thread_id FROM message_reference r JOIN message m ON m.id = r.message_id
                  WHERE r.ref_msgid = ?1 AND m.account_id = ?2 AND m.thread_id IS NOT NULL",
            )?;
            for r in m.references.iter().chain(m.message_id.iter()) {
                for t in referencing.query_map(params![r, account_id], |row| row.get::<_, i64>(0))? {
                    found.insert(t?);
                }
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
            // Messages of the absorbed thread already in `changes` would
            // name a thread that no longer exists: they name the kept one.
            let moved: Vec<i64> = changes.messages.iter().filter(|(_, t)| *t == Some(other)).map(|(id, _)| *id).collect();
            for id in moved {
                changes.messages.remove(&(id, Some(other)));
                changes.messages.insert((id, Some(keep)));
            }
            tx.prepare_cached("UPDATE message SET thread_id = ?1 WHERE thread_id = ?2")?.execute(params![keep, other])?;
            // `thread_folder` rows go with the thread (ON DELETE CASCADE).
            tx.prepare_cached("DELETE FROM thread WHERE id = ?1")?.execute([other])?;
            self.threads.remove(&other);
        }
        Ok(keep)
    }
}

/// The row of a raw file; already there if another message has the same
/// bytes (two folders, two accounts).
fn insert_raw_blob(tx: &Transaction, w: &Written) -> Result<()> {
    tx.prepare_cached("INSERT OR IGNORE INTO raw_blob (hash, size, stored_size) VALUES (?1, ?2, ?3)")?
        .execute(params![&w.hash[..], w.size as i64, w.stored_size as i64])?;
    Ok(())
}

/// The rows that come from the body: body, attachments, how to unsubscribe.
fn write_body(tx: &Transaction, message_id: i64, m: &ParsedMessage) -> Result<()> {
    // A message the MIME parser could not read keeps its raw file and gets
    // no body: `reindex` creates it once the parser can (`stockage-brut.md`,
    // section 5, "Échec de l'analyse MIME").
    if m.parsed {
        tx.prepare_cached(
            "INSERT INTO message_body (message_id, text_plain, html_sanitized, has_remote_content, tracking_pixels,
                                       parser_version)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?
        .execute(params![message_id, m.text, m.html, m.has_remote_content, m.tracking_pixels, PARSER_VERSION])?;
    }
    for a in &m.attachments {
        tx.prepare_cached(
            "INSERT OR IGNORE INTO attachment (message_id, part_id, filename, mime_type, size, content_id, is_inline)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?
        .execute(params![message_id, a.part_id, a.filename, a.mime_type, a.size, a.content_id, a.is_inline])?;
    }
    if let Some(u) = &m.unsubscribe {
        tx.prepare_cached(
            "INSERT INTO message_unsubscribe (message_id, url, mailto, one_click, source) VALUES (?1, ?2, ?3, ?4, ?5)",
        )?
        .execute(params![message_id, u.url, u.mailto, u.one_click, u.source.as_str()])?;
    }
    Ok(())
}
