//! The copies of a draft in the Drafts folder: the row the app lists, and
//! the one on the server the phone sees (`brouillons.md`, sections 3 to 6;
//! the draft itself is `drafts.rs`).
//!
//! # When
//!
//! While the reply is open, the draft lives in `draft` only: each pause of
//! typing writes there, nothing goes to the server. When it is **closed**
//! (Escape in the app, or the core restarting with a draft changed since
//! its last copy), [`Store::close_draft`]:
//!
//! 1. writes its MIME message (with `X-NiceMailer-Draft-Revision`) in the
//!    raw store, done by the caller, who has the raw store;
//! 2. replaces the draft's row of the Drafts folder by a new one, complete
//!    (`is_draft`, read), in the conversation of the mail it answers: the
//!    app lists it in Drafts at once;
//! 3. on an account synchronized with a server, queues an action
//!    `append_draft`, which the work connection sends (`nm-engine`,
//!    `sync/drafts.rs`).
//!
//! # On the server
//!
//! IMAP cannot change a message: each version is a new one. The action
//! writes the new version (`APPEND`, `\Draft \Seen`), then removes every
//! other copy of the same `Message-ID` from Drafts: the previous version,
//! and a copy left by a crash between the two steps. There is always at
//! least one copy, and replaying the action leaves exactly one.
//!
//! When the draft is sent or abandoned, its row goes, and an action
//! `delete` removes its copies from the server.
//!
//! # Changes made elsewhere (the simple cases of section 6)
//!
//! - The copy disappeared from the server, and the draft was **not**
//!   changed here since: deleted or sent on the phone. The draft goes too,
//!   when the synchronization of Drafts sees the copy gone
//!   ([`Store::drafts_gone_elsewhere`]).
//! - The copy disappeared, and the draft **was** changed here: the text is
//!   new, nothing is lost. The next copy looks for the mail in Sent first:
//!   sent elsewhere, the draft goes ([`Store::draft_sent_elsewhere`]);
//!   else it goes back to the server as a new copy.
//!
//! The adoption of a version changed on the phone, and two versions kept
//! in a conflict, are left for later (`docs/todo.md`).

use std::collections::BTreeSet;

use nm_core::Source;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::{Value, json};

use crate::insert::{Body, Flags, Inserter, NewMessage};
use crate::raw::Written;
use crate::{ChangeSet, Result, Store, StoreError};

/// Work of the Drafts folder for the work connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DraftWork {
    /// Writes this version of the draft, then removes the other copies.
    Append {
        action_id: i64,
        draft_id: i64,
        revision: i64,
        /// The MIME message of this version, in the raw store (the raw
        /// file of its row of Drafts): read by the engine.
        raw_hash: crate::raw::Hash,
        message_id_hdr: String,
        /// The UID of the last copy NiceMailer wrote, when known: if it
        /// is gone, the draft was deleted or sent elsewhere.
        previous_uid: Option<u32>,
        drafts_path: String,
        sent_path: Option<String>,
    },
    /// Removes every copy of this draft (sent, or abandoned).
    Remove { action_id: i64, message_id_hdr: String, drafts_path: String },
}

impl DraftWork {
    pub fn action_id(&self) -> i64 {
        match self {
            DraftWork::Append { action_id, .. } | DraftWork::Remove { action_id, .. } => *action_id,
        }
    }
}

fn synchronized_account(conn: &Connection, account_id: i64) -> Result<bool> {
    Ok(conn.query_row("SELECT EXISTS (SELECT 1 FROM temp.server_account WHERE id = ?1)", [account_id], |r| r.get(0))?)
}

fn drafts_folder(conn: &Connection, account_id: i64) -> Result<Option<(i64, String)>> {
    Ok(conn
        .query_row("SELECT id, path FROM folder WHERE account_id = ?1 AND role = 'drafts'", [account_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?)
}

/// The queued or running actions of the Drafts folder for this draft.
fn draft_actions(conn: &Connection, draft_id: i64) -> Result<Vec<(i64, String, String, Value)>> {
    Ok(conn
        .prepare_cached(
            "SELECT id, kind, state, payload FROM pending_action
              WHERE kind = 'append_draft' AND state IN ('queued', 'running') AND json_extract(payload, '$.draft_id') = ?1",
        )?
        .query_map([draft_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get::<_, String>(3)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?
        .into_iter()
        .map(|(id, kind, state, p)| (id, kind, state, serde_json::from_str(&p).unwrap_or(Value::Null)))
        .collect())
}

/// A row of Drafts put in Deleted (`del`, `Store::move_thread`): if it is
/// a draft of NiceMailer, its row of `draft` goes, else the next closing
/// would write it back to Drafts. The row of Drafts, moved to Deleted
/// like any mail (here and on the server), keeps its text: it was written
/// when the draft was last closed, and a draft open in the reply screen is
/// not in the list where `del` is pressed. `z` brings it back to Drafts as
/// a draft of another client (shown, not editable).
///
/// Refused, changing nothing (the caller's transaction is dropped): a draft
/// being sent, and a draft whose copy to the server is still to be made
/// (its row has no UID yet, and the copy would bring it back).
pub(crate) fn release_for_deletion(tx: &Transaction, message_id: i64) -> Result<()> {
    let row: Option<(i64, String)> =
        tx.query_row("SELECT id, state FROM draft WHERE message_id = ?1", [message_id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
    let Some((id, state)) = row else { return Ok(()) };
    if state == "sending" {
        return Err(StoreError::DraftSending);
    }
    if !draft_actions(tx, id)?.is_empty() {
        return Err(StoreError::InvalidMove("this draft is being copied to the server: try again in a moment".into()));
    }
    tx.execute("DELETE FROM draft WHERE id = ?1", [id])?;
    Ok(())
}

/// The row of the draft in Drafts deleted, with its thread and folder
/// refreshed.
fn delete_row(tx: &Transaction, message_id: i64, changes: &mut ChangeSet) -> Result<()> {
    let row: Option<(i64, Option<i64>)> =
        tx.query_row("SELECT folder_id, thread_id FROM message WHERE id = ?1", [message_id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
    let Some((folder, thread)) = row else { return Ok(()) };
    tx.execute("DELETE FROM message WHERE id = ?1", [message_id])?;
    changes.messages.insert((message_id, thread));
    changes.folders.insert(folder);
    let mut threads = BTreeSet::new();
    threads.extend(thread);
    crate::screening::refresh_all(tx, &threads, changes)?;
    Ok(())
}

/// The draft leaves Drafts (sent, abandoned, deleted elsewhere): its row
/// goes, its pending copy is cancelled, and, if a copy may be on the
/// server, an action `delete` removes it. Called in the caller's
/// transaction, before the draft row itself is deleted.
pub(crate) fn forget_copies(tx: &Transaction, draft_id: i64, source: Source, changes: &mut ChangeSet) -> Result<()> {
    let row: Option<(i64, Option<i64>, String, Option<i64>)> = tx
        .query_row("SELECT account_id, message_id, message_id_hdr, server_revision FROM draft WHERE id = ?1", [draft_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .optional()?;
    let Some((account_id, message, msgid, server_revision)) = row else { return Ok(()) };
    let mut on_server = server_revision.is_some();
    for (id, _, state, _) in draft_actions(tx, draft_id)? {
        if state == "running" {
            // On its way: its copy is to be removed once there.
            on_server = true;
        } else {
            tx.execute(
                "UPDATE pending_action SET state = 'cancelled', last_error = 'the draft left Drafts', date_modified = unixepoch()
                  WHERE id = ?1",
                [id],
            )?;
        }
    }
    if let Some(m) = message {
        delete_row(tx, m, changes)?;
    }
    if on_server
        && synchronized_account(tx, account_id)?
        && let Some((folder, _)) = drafts_folder(tx, account_id)?
    {
        tx.execute(
            "INSERT INTO pending_action (account_id, kind, message_id, payload, created_by) VALUES (?1, 'delete', NULL, ?2, ?3)",
            params![account_id, json!({"draft_id": draft_id, "folder_id": folder, "message_id_hdr": msgid}).to_string(), source.as_str()],
        )?;
        changes.spooled.insert(account_id);
    }
    Ok(())
}

impl Store {
    /// The MIME message of the draft to copy to Drafts, if it changed since
    /// its last copy (or never had one): its bytes. The caller
    /// writes the bytes in the raw store, then calls [`Store::close_draft`].
    pub fn draft_to_close(&self, id: i64, user_agent: &str) -> Result<Option<Vec<u8>>> {
        let row: Option<(i64, Option<i64>, Option<i64>, String)> = self
            .conn
            .query_row("SELECT revision, server_revision, message_id, state FROM draft WHERE id = ?1", [id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })
            .optional()?;
        let Some((revision, server_revision, message, state)) = row else {
            return Err(StoreError::NotFound(format!("brouillon {id}")));
        };
        // Being sent: it leaves Drafts anyway.
        if state == "sending" || (message.is_some() && server_revision == Some(revision)) {
            return Ok(None);
        }
        Ok(Some(crate::drafts::build_draft_mime(&self.conn, id, user_agent, Some(revision), self.outgoing.as_ref())?))
    }

    /// The drafts changed since their last copy: at start, nothing is open,
    /// so they are closed (`brouillons.md`, section 4, "Plantage").
    pub fn drafts_to_close(&self) -> Result<Vec<i64>> {
        Ok(self
            .conn
            .prepare("SELECT id FROM draft WHERE state = 'editing' AND (message_id IS NULL OR server_revision IS NOT revision) ORDER BY id")?
            .query_map([], |r| r.get(0))?
            .collect::<std::result::Result<_, _>>()?)
    }

    /// The draft is closed: its new version is the row of Drafts, and, on
    /// an account with a server, it is queued for a copy there (see the
    /// header of this file). `written`: its MIME message in the raw store
    /// ([`Store::draft_to_close`]).
    pub fn close_draft(&mut self, id: i64, written: &Written, bytes: &[u8], source: Source) -> Result<ChangeSet> {
        let mut changes = ChangeSet::default();
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row: Option<(i64, i64, Option<i64>, String)> = tx
            .query_row("SELECT account_id, revision, message_id, message_id_hdr FROM draft WHERE id = ?1", [id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })
            .optional()?;
        let Some((account_id, revision, previous, msgid)) = row else {
            return Err(StoreError::NotFound(format!("brouillon {id}")));
        };
        let Some((folder, _)) = drafts_folder(&tx, account_id)? else {
            // No Drafts folder: the draft stays in `draft`, as in v1.
            return Ok(changes);
        };
        let previous_uid: Option<u32> = match previous {
            Some(m) => tx.query_row("SELECT uid FROM message WHERE id = ?1", [m], |r| r.get(0)).optional()?.flatten(),
            None => None,
        };
        if let Some(m) = previous {
            delete_row(&tx, m, &mut changes)?;
        }
        let parsed = crate::ingest::parse(bytes);
        let mut inserter = Inserter::new(source, None);
        let m = NewMessage {
            account_id,
            folder_id: folder,
            uid: None,
            date_received: crate::drafts::now(),
            size: Some(written.size),
            modseq: None,
            flags: Flags { seen: true, draft: true, ..Flags::default() },
            auth: (nm_core::AuthResult::None, None),
            headers: &parsed,
        };
        let row_id = inserter.insert(&tx, &m, Some(Body { written, parsed: &parsed }), &mut changes)?;
        tx.execute("UPDATE draft SET message_id = ?2 WHERE id = ?1", params![id, row_id])?;
        if synchronized_account(&tx, account_id)? {
            // A copy not sent yet is replaced by this one; the previous UID
            // it would have checked is carried over.
            let mut carried = previous_uid;
            for (action, _, state, p) in draft_actions(&tx, id)? {
                if state == "queued" {
                    carried = carried.or(p["previous_uid"].as_u64().map(|u| u as u32));
                    tx.execute(
                        "UPDATE pending_action SET state = 'cancelled', last_error = 'merged: a newer version', date_modified = unixepoch()
                          WHERE id = ?1",
                        [action],
                    )?;
                }
            }
            tx.execute(
                "INSERT INTO pending_action (account_id, kind, message_id, payload, created_by) VALUES (?1, 'append_draft', ?2, ?3, ?4)",
                params![
                    account_id,
                    row_id,
                    json!({"draft_id": id, "revision": revision, "message_id_hdr": msgid, "previous_uid": carried}).to_string(),
                    source.as_str()
                ],
            )?;
            changes.spooled.insert(account_id);
        } else {
            // Nothing to send anywhere (an archive or fake account): the
            // row of Drafts is the copy.
            tx.execute("UPDATE draft SET server_revision = revision WHERE id = ?1", [id])?;
        }
        inserter.commit(tx, &mut changes)?;
        Ok(changes)
    }

    /// The work of the Drafts folder due on `account_id`, marked running.
    pub fn draft_work_claim(&mut self, account_id: i64) -> Result<Vec<DraftWork>> {
        let tx = self.transaction()?;
        let rows: Vec<(i64, String, String, Option<i64>)> = tx
            .prepare(
                "SELECT id, kind, payload, message_id FROM pending_action
                  WHERE account_id = ?1 AND kind IN ('append_draft', 'delete') AND state = 'queued' AND date_run_after <= unixepoch()
                  ORDER BY id",
            )?
            .query_map([account_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<std::result::Result<_, _>>()?;
        let drafts = drafts_folder(&tx, account_id)?;
        let sent: Option<String> =
            tx.query_row("SELECT path FROM folder WHERE account_id = ?1 AND role = 'sent'", [account_id], |r| r.get(0)).optional()?;
        let mut out = Vec::new();
        for (id, kind, payload, message) in rows {
            let p: Value = serde_json::from_str(&payload).unwrap_or(Value::Null);
            let (Some((_, drafts_path)), Some(msgid)) = (drafts.clone(), p["message_id_hdr"].as_str().map(str::to_string)) else {
                tx.execute("UPDATE pending_action SET state = 'done', last_error = 'no Drafts folder' WHERE id = ?1", [id])?;
                continue;
            };
            let work = if kind == "delete" {
                DraftWork::Remove { action_id: id, message_id_hdr: msgid, drafts_path }
            } else {
                // The MIME of this version: the raw file of the row.
                let hash: Option<Vec<u8>> = match message {
                    Some(m) => tx.query_row("SELECT raw_hash FROM message WHERE id = ?1", [m], |r| r.get(0)).optional()?.flatten(),
                    None => None,
                };
                let Some(hash) = hash.and_then(|h| <crate::raw::Hash>::try_from(h.as_slice()).ok()) else {
                    // The row is gone (a newer version, the draft sent):
                    // nothing of this version to write.
                    tx.execute("UPDATE pending_action SET state = 'done', last_error = 'version replaced' WHERE id = ?1", [id])?;
                    continue;
                };
                DraftWork::Append {
                    action_id: id,
                    draft_id: p["draft_id"].as_i64().unwrap_or(0),
                    revision: p["revision"].as_i64().unwrap_or(0),
                    raw_hash: hash,
                    message_id_hdr: msgid,
                    previous_uid: p["previous_uid"].as_u64().map(|u| u as u32),
                    drafts_path,
                    sent_path: sent.clone(),
                }
            };
            tx.execute("UPDATE pending_action SET state = 'running', date_modified = unixepoch() WHERE id = ?1", [id])?;
            out.push(work);
        }
        tx.commit()?;
        Ok(out)
    }

    /// Whether the draft still exists (not sent nor abandoned meanwhile).
    pub fn draft_exists(&self, id: i64) -> Result<bool> {
        Ok(self.conn.query_row("SELECT EXISTS (SELECT 1 FROM draft WHERE id = ?1)", [id], |r| r.get(0))?)
    }

    /// The copy is on the server, at `uid` when known: the row gets it, and
    /// the draft records the revision the server has.
    pub fn draft_appended(&mut self, action_id: i64, uid: Option<u32>) -> Result<ChangeSet> {
        let tx = self.transaction()?;
        let mut changes = ChangeSet::default();
        let row: Option<(Option<i64>, String)> =
            tx.query_row("SELECT message_id, payload FROM pending_action WHERE id = ?1", [action_id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
        if let Some((message, payload)) = row {
            let p: Value = serde_json::from_str(&payload).unwrap_or(Value::Null);
            if let (Some(m), Some(uid)) = (message, uid) {
                let n = tx.execute(
                    "UPDATE message SET uid = ?2 WHERE id = ?1 AND uid IS NULL
                       AND NOT EXISTS (SELECT 1 FROM message o WHERE o.folder_id = message.folder_id AND o.uid = ?2)",
                    params![m, uid],
                )?;
                if n > 0 {
                    let thread: Option<i64> = tx.query_row("SELECT thread_id FROM message WHERE id = ?1", [m], |r| r.get(0))?;
                    changes.messages.insert((m, thread));
                }
            }
            if let (Some(d), Some(rev)) = (p["draft_id"].as_i64(), p["revision"].as_i64()) {
                tx.execute("UPDATE draft SET server_revision = ?2 WHERE id = ?1", params![d, rev])?;
            }
        }
        tx.execute("UPDATE pending_action SET state = 'done', last_error = NULL, date_modified = unixepoch() WHERE id = ?1", [action_id])?;
        tx.commit()?;
        Ok(changes)
    }

    /// The work is done (copies removed, nothing left to do).
    pub fn draft_work_done(&mut self, action_id: i64, note: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE pending_action SET state = 'done', last_error = ?2, date_modified = unixepoch() WHERE id = ?1",
            params![action_id, note],
        )?;
        Ok(())
    }

    /// A temporary failure: tried again with the spool's delays; after the
    /// last attempt, given up (the draft stays here, its copy is not made).
    pub fn draft_work_retry(&mut self, action_id: i64, error: &str) -> Result<()> {
        let attempts: i64 = self.conn.query_row("SELECT attempts + 1 FROM pending_action WHERE id = ?1", [action_id], |r| r.get(0))?;
        if attempts >= crate::spool::MAX_ATTEMPTS {
            self.conn.execute(
                "UPDATE pending_action SET state = 'reverted', attempts = ?2, last_error = ?3, date_modified = unixepoch() WHERE id = ?1",
                params![action_id, attempts, format!("{attempts} attempts failed, the last: {error}")],
            )?;
            return Ok(());
        }
        let delay = (crate::spool::RETRY_FIRST_SECS << (attempts - 1).min(20)).min(crate::spool::RETRY_MAX_SECS);
        self.conn.execute(
            "UPDATE pending_action SET state = 'queued', attempts = ?2, last_error = ?3,
                    date_run_after = unixepoch() + ?4, date_modified = unixepoch()
              WHERE id = ?1",
            params![action_id, attempts, error, delay],
        )?;
        Ok(())
    }

    /// The copy disappeared from the server and the mail is in Sent: the
    /// draft was sent elsewhere. It goes, with its row, and the action ends.
    pub fn draft_sent_elsewhere(&mut self, action_id: i64, draft_id: i64) -> Result<ChangeSet> {
        let mut changes = ChangeSet::default();
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        drop_draft(&tx, draft_id, "sent_elsewhere", &mut changes)?;
        tx.execute(
            "UPDATE pending_action SET state = 'done', last_error = 'sent elsewhere', date_modified = unixepoch() WHERE id = ?1",
            [action_id],
        )?;
        tx.commit()?;
        Ok(changes)
    }

    /// After the synchronization removed rows of Drafts gone from the
    /// server: the drafts whose copy went, and that were not changed here
    /// since, were deleted or sent elsewhere; that choice is respected
    /// (`brouillons.md`, section 6). Returns how many went.
    pub fn drafts_gone_elsewhere(&mut self) -> Result<(usize, ChangeSet)> {
        let mut changes = ChangeSet::default();
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let n = gone_elsewhere(&tx, &mut changes)?;
        tx.commit()?;
        Ok((n, changes))
    }
}

/// [`Store::drafts_gone_elsewhere`] in a transaction of the caller (the end
/// of the synchronization of a folder, `sync.rs`).
pub(crate) fn gone_elsewhere(tx: &Transaction, changes: &mut ChangeSet) -> Result<usize> {
    {
        let ids: Vec<i64> = tx
            .prepare(
                "SELECT id FROM draft
                  WHERE message_id IS NULL AND server_revision IS NOT NULL AND server_revision = revision AND state = 'editing'",
            )?
            .query_map([], |r| r.get(0))?
            .collect::<std::result::Result<_, _>>()?;
        for id in &ids {
            drop_draft(tx, *id, "gone_elsewhere", changes)?;
        }
        Ok(ids.len())
    }
}

/// Deletes a draft that went elsewhere, with a line of history.
fn drop_draft(tx: &Transaction, draft_id: i64, reason: &str, changes: &mut ChangeSet) -> Result<()> {
    let row: Option<(Option<i64>, Option<i64>, String)> = tx
        .query_row("SELECT message_id, reply_to_message_id, message_id_hdr FROM draft WHERE id = ?1", [draft_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .optional()?;
    let Some((message, reply_to, msgid)) = row else { return Ok(()) };
    if let Some(m) = message {
        delete_row(tx, m, changes)?;
    }
    tx.execute("DELETE FROM draft WHERE id = ?1", [draft_id])?;
    tx.execute(
        "INSERT INTO audit_log (created_by, entity, entity_id, action, detail)
         VALUES ('imap', 'message', ?1, 'draft_removed', json_object('reason', ?2, 'message_id_hdr', ?3))",
        params![reply_to.unwrap_or(0), reason, msgid],
    )?;
    Ok(())
}
