//! The spool: what NiceMailer has changed locally and must still tell the
//! server (`modele de donnees.md`, section 6; `plan-imap.md`, phase 5).
//!
//! # Principle
//!
//! A command changes the `message` rows at once, so the user sees the
//! result immediately, and in the same transaction writes an *action* in
//! `pending_action`, with what it takes to undo the change (`undo`). The
//! engine (`nm-engine`, `sync/spool.rs`) sends the actions to the server
//! later, in order, and reports here how each ended:
//!
//! - **done** ([`Store::spool_done`]): the server did it. For a move, the
//!   message's new UID is recorded when the server gave it (`COPYUID`);
//! - **temporary failure** ([`Store::spool_retry`]): tried again later,
//!   after a delay doubled at each failure (30 s to 1 h), 8 times at most;
//! - **definitive failure** ([`Store::spool_revert`]): the local change is
//!   undone, and so are the later actions on the same message (`cancelled`),
//!   and `ActionReverted` tells the user.
//!
//! Only the accounts the core synchronizes get actions: the IMAP accounts
//! of `accounts.toml`, listed in the temporary table `server_account` of the
//! writing connection ([`Store::set_server_accounts`]). The fake accounts
//! of `make seed` and the archive accounts change locally only.
//!
//! # Payloads
//!
//! | `kind` | `payload` | `undo` |
//! | --- | --- | --- |
//! | `move` | `{from_folder_id, from_uid, to_folder_id}` | `{folder_id, uid}` |
//! | `set_flags` | `{flag, set, folder_id, uid}` (`\Seen`, `\Flagged`, or a keyword; where the message was, if it had a UID) | `{set}` |
//!
//! # Merging
//!
//! A move asked while the previous move of the same message is still
//! `queued` is merged into it: Inbox → Todo, then Todo → Archive, becomes
//! Inbox → Archive, and keeps the `undo` of the first. Back to where it
//! started, the action is cancelled and the message gets its UID back. The
//! same for a flag set then cleared before being sent.

use std::collections::BTreeSet;

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::{Value, json};

use crate::{ChangeSet, Result, Store};

/// Attempts before a temporary failure becomes definitive (validated by the
/// user, `plan-imap.md`, phase 5).
pub const MAX_ATTEMPTS: i64 = 8;
/// First delay after a temporary failure, doubled at each one…
pub const RETRY_FIRST_SECS: i64 = 30;
/// …up to this.
pub const RETRY_MAX_SECS: i64 = 3600;
/// Actions ended (`done`, `reverted`, `cancelled`) are deleted after this.
pub const PURGE_AFTER_SECS: i64 = 7 * 24 * 3600;

/// An action to send, as the engine sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    pub id: i64,
    pub message_id: i64,
    pub kind: ActionKind,
    /// Its `Message-ID`, to find it on the server when its UID is unknown.
    pub message_id_hdr: Option<String>,
    pub attempts: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionKind {
    Move {
        from_folder: i64,
        from_path: String,
        /// `None`: unknown (the message had no UID when moved).
        from_uid: Option<u32>,
        to_folder: i64,
        to_path: String,
    },
    SetFlag {
        folder: i64,
        path: String,
        uid: u32,
        /// `\Seen`, `\Flagged`, or a keyword.
        flag: String,
        set: bool,
    },
}

/// Whether the message's account is one the core synchronizes.
fn synchronized(conn: &Connection, message_id: i64) -> Result<bool> {
    Ok(conn
        .prepare_cached(
            "SELECT EXISTS (SELECT 1 FROM message m JOIN temp.server_account s ON s.id = m.account_id WHERE m.id = ?1)",
        )?
        .query_row([message_id], |r| r.get(0))?)
}

/// The queued action of `kind` on a message, if any (the last one), for a
/// merge: `(id, payload, undo)`.
fn queued(conn: &Connection, message_id: i64, kind: &str) -> Result<Option<(i64, Value, Value)>> {
    let row: Option<(i64, String, Option<String>)> = conn
        .prepare_cached(
            "SELECT id, payload, undo FROM pending_action
              WHERE message_id = ?1 AND kind = ?2 AND state = 'queued' ORDER BY id DESC LIMIT 1",
        )?
        .query_row(params![message_id, kind], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .optional()?;
    Ok(row.map(|(id, p, u)| {
        (id, serde_json::from_str(&p).unwrap_or(Value::Null), u.and_then(|u| serde_json::from_str(&u).ok()).unwrap_or(Value::Null))
    }))
}

/// Records the move of a message, called by `move_message` before it
/// changes the row (`from_uid` is the UID it had). Returns the UID the
/// message gets back when the move cancels a queued one (back where it
/// was), so that the caller does not leave it without UID.
pub(crate) fn enqueue_move(
    conn: &Connection,
    message_id: i64,
    from_folder: i64,
    from_uid: Option<u32>,
    to_folder: i64,
    created_by: &str,
    changes: &mut ChangeSet,
) -> Result<Option<u32>> {
    if !synchronized(conn, message_id)? {
        return Ok(None);
    }
    let account_id: i64 = conn.prepare_cached("SELECT account_id FROM message WHERE id = ?1")?.query_row([message_id], |r| r.get(0))?;
    changes.spooled.insert(account_id);
    // A move not sent yet: merged into it, unless one is running (its
    // source is then on its way to the server, and cannot change).
    let running: bool = conn
        .prepare_cached("SELECT EXISTS (SELECT 1 FROM pending_action WHERE message_id = ?1 AND kind = 'move' AND state = 'running')")?
        .query_row([message_id], |r| r.get(0))?;
    if !running && let Some((id, mut payload, undo)) = queued(conn, message_id, "move")? {
        if payload["from_folder_id"].as_i64() == Some(to_folder) {
            // Back where it started: nothing to tell the server.
            conn.prepare_cached(
                "UPDATE pending_action SET state = 'cancelled', last_error = 'merged: back to its folder', date_modified = unixepoch()
                  WHERE id = ?1",
            )?
            .execute([id])?;
            return Ok(undo["uid"].as_u64().map(|u| u as u32));
        }
        payload["to_folder_id"] = json!(to_folder);
        conn.prepare_cached("UPDATE pending_action SET payload = ?2, date_modified = unixepoch() WHERE id = ?1")?
            .execute(params![id, payload.to_string()])?;
        return Ok(None);
    }
    conn.prepare_cached(
        "INSERT INTO pending_action (account_id, kind, message_id, payload, undo, created_by)
         VALUES (?1, 'move', ?2, ?3, ?4, ?5)",
    )?
    .execute(params![
        account_id,
        message_id,
        json!({"from_folder_id": from_folder, "from_uid": from_uid, "to_folder_id": to_folder}).to_string(),
        json!({"folder_id": from_folder, "uid": from_uid}).to_string(),
        created_by,
    ])?;
    Ok(None)
}

/// Records a flag (or keyword) set or cleared on a message, already changed
/// locally.
pub(crate) fn enqueue_flag(conn: &Connection, message_id: i64, flag: &str, set: bool, created_by: &str, changes: &mut ChangeSet) -> Result<()> {
    if !synchronized(conn, message_id)? {
        return Ok(());
    }
    let account_id: i64 = conn.prepare_cached("SELECT account_id FROM message WHERE id = ?1")?.query_row([message_id], |r| r.get(0))?;
    changes.spooled.insert(account_id);
    // The same flag queued for this message: merged.
    let same: Option<(i64, Value)> = {
        let mut stmt = conn.prepare_cached(
            "SELECT id, payload FROM pending_action WHERE message_id = ?1 AND kind = 'set_flags' AND state = 'queued' ORDER BY id",
        )?;
        let rows: Vec<(i64, String)> = stmt.query_map([message_id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<std::result::Result<_, _>>()?;
        rows.into_iter()
            .map(|(id, p)| (id, serde_json::from_str::<Value>(&p).unwrap_or(Value::Null)))
            .find(|(_, p)| p["flag"].as_str().is_some_and(|f| f.eq_ignore_ascii_case(flag)))
    };
    if let Some((id, _)) = same {
        // Set then cleared before being sent (or the reverse): the server
        // never needs to know.
        conn.prepare_cached(
            "UPDATE pending_action SET state = 'cancelled', last_error = 'merged: undone before being sent', date_modified = unixepoch()
              WHERE id = ?1",
        )?
        .execute([id])?;
        return Ok(());
    }
    // Where the message is when the flag changes: the action runs there,
    // in its turn. A move asked later must not make it wait for a UID the
    // message only gets once that move, queued after it, is done.
    let (folder, uid): (i64, Option<u32>) =
        conn.prepare_cached("SELECT folder_id, uid FROM message WHERE id = ?1")?.query_row([message_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
    let payload = match uid {
        Some(uid) => json!({"flag": flag, "set": set, "folder_id": folder, "uid": uid}),
        // No UID yet (moved, not sent): it runs where the message is once
        // the move is done and its UID known.
        None => json!({"flag": flag, "set": set}),
    };
    conn.prepare_cached(
        "INSERT INTO pending_action (account_id, kind, message_id, payload, undo, created_by)
         VALUES (?1, 'set_flags', ?2, ?3, ?4, ?5)",
    )?
    .execute(params![
        account_id,
        message_id,
        payload.to_string(),
        json!({"set": !set}).to_string(),
        created_by
    ])?;
    Ok(())
}

impl Store {
    /// The accounts the core synchronizes (the IMAP accounts of
    /// `accounts.toml`): their changes go to the spool. Kept in a temporary
    /// table of the writing connection, so that the SQL of every command
    /// can tell, without a parameter passed everywhere.
    pub fn set_server_accounts(&mut self, ids: BTreeSet<i64>) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM temp.server_account", [])?;
        for id in &ids {
            tx.execute("INSERT INTO temp.server_account (id) VALUES (?1)", [id])?;
        }
        tx.commit()?;
        self.server_accounts = ids;
        Ok(())
    }

    pub fn server_accounts(&self) -> &BTreeSet<i64> {
        &self.server_accounts
    }

    /// The accounts whose server must never be changed (`read_only = true`
    /// in `accounts.toml`): no action is queued for them (they are not among
    /// [`Store::set_server_accounts`]), and the commands that change their
    /// mail are refused, since the next synchronization would undo a local
    /// change.
    pub fn set_read_only_accounts(&mut self, ids: BTreeSet<i64>) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM temp.read_only_account", [])?;
        for id in &ids {
            tx.execute("INSERT INTO temp.read_only_account (id) VALUES (?1)", [id])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// At start: the actions that were running when the core stopped are
    /// queued again. Each can be replayed: a move whose source UID is gone
    /// was done (`modele de donnees.md`, section 6, "Reprise après
    /// plantage").
    pub fn spool_requeue_running(&mut self) -> Result<usize> {
        Ok(self.conn.execute("UPDATE pending_action SET state = 'queued', date_modified = unixepoch() WHERE state = 'running'", [])?)
    }

    /// The actions of `account_id` that can be sent now, marked `running`:
    /// due (`date_run_after`), the first of their message (actions on a
    /// message run one at a time, in order), and whose message is where the
    /// action says (a flag on a message whose UID is not known yet waits).
    pub fn spool_claim(&mut self, account_id: i64, limit: u32) -> Result<Vec<Action>> {
        let tx = self.transaction()?;
        let rows: Vec<(i64, i64, String, String, Option<String>, i64)> = tx
            .prepare(
                "SELECT a.id, a.message_id, a.kind, a.payload, m.message_id_hdr, a.attempts
                   FROM pending_action a JOIN message m ON m.id = a.message_id
                  WHERE a.account_id = ?1 AND a.state = 'queued' AND a.date_run_after <= unixepoch()
                    AND a.kind IN ('move', 'set_flags')
                    AND NOT EXISTS (SELECT 1 FROM pending_action b
                                     WHERE b.message_id = a.message_id AND b.state IN ('queued', 'running') AND b.id < a.id)
                  ORDER BY a.id LIMIT ?2",
            )?
            .query_map(params![account_id, limit], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?
            .collect::<std::result::Result<_, _>>()?;
        let mut out = Vec::new();
        for (id, message_id, kind, payload, hdr, attempts) in rows {
            let p: Value = serde_json::from_str(&payload).unwrap_or(Value::Null);
            let path = |folder: i64| -> Result<Option<String>> {
                Ok(tx.query_row("SELECT path FROM folder WHERE id = ?1", [folder], |r| r.get(0)).optional()?)
            };
            let kind = match kind.as_str() {
                "move" => {
                    let (Some(from), Some(to)) = (p["from_folder_id"].as_i64(), p["to_folder_id"].as_i64()) else { continue };
                    let (Some(from_path), Some(to_path)) = (path(from)?, path(to)?) else { continue };
                    ActionKind::Move {
                        from_folder: from,
                        from_path,
                        from_uid: p["from_uid"].as_u64().map(|u| u as u32),
                        to_folder: to,
                        to_path,
                    }
                }
                _ => {
                    // Where the message was when the flag changed; else
                    // where it is now, with its UID: none yet (moved
                    // without UIDPLUS, not attached): it waits.
                    let recorded = match (p["folder_id"].as_i64(), p["uid"].as_u64()) {
                        (Some(f), Some(u)) => path(f)?.map(|path| (f, Some(u as u32), path)),
                        _ => None,
                    };
                    let at: Option<(i64, Option<u32>, String)> = if recorded.is_some() { recorded } else { tx
                        .query_row(
                            "SELECT m.folder_id, m.uid, f.path FROM message m JOIN folder f ON f.id = m.folder_id WHERE m.id = ?1",
                            [message_id],
                            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                        )
                        .optional()? };
                    let Some((folder, Some(uid), path)) = at else { continue };
                    let (Some(flag), Some(set)) = (p["flag"].as_str(), p["set"].as_bool()) else { continue };
                    ActionKind::SetFlag { folder, path, uid, flag: flag.to_string(), set }
                }
            };
            tx.execute("UPDATE pending_action SET state = 'running', date_modified = unixepoch() WHERE id = ?1", [id])?;
            out.push(Action { id, message_id, kind, message_id_hdr: hdr, attempts });
        }
        tx.commit()?;
        Ok(out)
    }

    /// The server did it. `new_uid`: the UID of a moved message in its new
    /// folder, when the server gave it (`COPYUID`) or the engine found it.
    pub fn spool_done(&mut self, action_id: i64, new_uid: Option<u32>) -> Result<ChangeSet> {
        let tx = self.transaction()?;
        let changes = ChangeSet::default();
        let row: Option<(i64, String, String)> = tx
            .query_row("SELECT message_id, kind, payload FROM pending_action WHERE id = ?1", [action_id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .optional()?;
        tx.execute(
            "UPDATE pending_action SET state = 'done', last_error = NULL, date_modified = unixepoch() WHERE id = ?1",
            [action_id],
        )?;
        if let (Some((message_id, kind, payload)), Some(uid)) = (row, new_uid)
            && kind == "move"
        {
            let p: Value = serde_json::from_str(&payload).unwrap_or(Value::Null);
            let to = p["to_folder_id"].as_i64();
            // Still in that folder: its UID there. Moved on locally since
            // (a later move queued): the UID is the source of that move.
            let here: Option<i64> = tx.query_row("SELECT folder_id FROM message WHERE id = ?1", [message_id], |r| r.get(0)).optional()?;
            if here.is_some() && here == to {
                // A row already at that UID (attached by a pass meanwhile)
                // keeps it: never two rows for one UID.
                tx.execute(
                    "UPDATE message SET uid = ?2 WHERE id = ?1 AND uid IS NULL
                       AND NOT EXISTS (SELECT 1 FROM message o WHERE o.folder_id = message.folder_id AND o.uid = ?2)",
                    params![message_id, uid],
                )?;
            } else if let Some(to) = to {
                let next: Option<(i64, String)> = tx
                    .query_row(
                        "SELECT id, payload FROM pending_action WHERE message_id = ?1 AND kind = 'move' AND state = 'queued'
                          ORDER BY id LIMIT 1",
                        [message_id],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()?;
                if let Some((next_id, next_payload)) = next {
                    let mut np: Value = serde_json::from_str(&next_payload).unwrap_or(Value::Null);
                    if np["from_folder_id"].as_i64() == Some(to) && np["from_uid"].is_null() {
                        np["from_uid"] = json!(uid);
                        tx.execute("UPDATE pending_action SET payload = ?2 WHERE id = ?1", params![next_id, np.to_string()])?;
                    }
                }
            }
        }
        tx.commit()?;
        Ok(changes)
    }

    /// A temporary failure: tried again after a delay, or, after
    /// [`MAX_ATTEMPTS`], undone like a definitive one.
    pub fn spool_retry(&mut self, action_id: i64, error: &str) -> Result<ChangeSet> {
        let attempts: i64 =
            self.conn.query_row("SELECT attempts + 1 FROM pending_action WHERE id = ?1", [action_id], |r| r.get(0))?;
        if attempts >= MAX_ATTEMPTS {
            return self.spool_revert(action_id, nm_api::reason::ATTEMPTS_FAILED.with("attempts", attempts).with("server_text", error));
        }
        let delay = (RETRY_FIRST_SECS << (attempts - 1).min(20)).min(RETRY_MAX_SECS);
        self.conn.execute(
            "UPDATE pending_action SET state = 'queued', attempts = ?2, last_error = ?3,
                    date_run_after = unixepoch() + ?4, date_modified = unixepoch()
              WHERE id = ?1",
            params![action_id, attempts, error, delay],
        )?;
        Ok(ChangeSet::default())
    }

    /// A definitive failure: the change of this action and of every later
    /// one on the same message are undone, latest first; this action is
    /// `reverted`, the later ones `cancelled`. `ActionReverted` tells the
    /// user (`modele de donnees.md`, section 6).
    pub fn spool_revert(&mut self, action_id: i64, why: nm_api::Why) -> Result<ChangeSet> {
        // English in the database (`last_error`, the history); the event
        // carries the reason.
        let reason = why.message();
        let tx = self.transaction()?;
        let mut changes = ChangeSet::default();
        let first: Option<(i64, String)> = tx
            .query_row("SELECT message_id, kind FROM pending_action WHERE id = ?1", [action_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        let Some((message_id, kind)) = first else { return Ok(changes) };
        let actions: Vec<(i64, String, String, Option<String>)> = tx
            .prepare(
                "SELECT id, kind, payload, undo FROM pending_action
                  WHERE message_id = ?1 AND id >= ?2 AND state IN ('queued', 'running') ORDER BY id DESC",
            )?
            .query_map(params![message_id, action_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<std::result::Result<_, _>>()?;
        let mut threads = BTreeSet::new();
        for (id, kind, payload, undo) in &actions {
            undo_one(&tx, message_id, kind, payload, undo.as_deref(), &mut changes, &mut threads)?;
            let state = if *id == action_id { "reverted" } else { "cancelled" };
            tx.execute(
                "UPDATE pending_action SET state = ?2, last_error = ?3, date_modified = unixepoch() WHERE id = ?1",
                params![id, state, if *id == action_id { reason.to_string() } else { format!("after action {action_id}, undone") }],
            )?;
        }
        tx.execute(
            "INSERT INTO audit_log (created_by, entity, entity_id, action, detail)
             VALUES ('core', 'message', ?1, 'revert', json_object('kind', ?2, 'reason', ?3, 'actions', ?4))",
            params![message_id, kind, reason, actions.len() as i64],
        )?;
        crate::screening::refresh_all(&tx, &threads, &mut changes)?;
        changes.reverted.push((message_id, kind, why));
        tx.commit()?;
        Ok(changes)
    }

    /// Deletes the actions ended more than [`PURGE_AFTER_SECS`] ago.
    pub fn spool_purge(&mut self) -> Result<usize> {
        Ok(self.conn.execute(
            "DELETE FROM pending_action WHERE state IN ('done', 'reverted', 'cancelled') AND date_modified < unixepoch() - ?1",
            [PURGE_AFTER_SECS],
        )?)
    }

    /// The actions of an account not sent yet (`sync status`).
    pub fn spool_pending(&self, account_id: i64) -> Result<u64> {
        Ok(self.conn.query_row(
            "SELECT count(*) FROM pending_action WHERE account_id = ?1 AND state IN ('queued', 'running')",
            [account_id],
            |r| r.get(0),
        )?)
    }
}

/// Undoes one action on the message, from its `undo`.
fn undo_one(
    tx: &Transaction,
    message_id: i64,
    kind: &str,
    payload: &str,
    undo: Option<&str>,
    changes: &mut ChangeSet,
    threads: &mut BTreeSet<i64>,
) -> Result<()> {
    let p: Value = serde_json::from_str(payload).unwrap_or(Value::Null);
    let u: Value = undo.and_then(|u| serde_json::from_str(u).ok()).unwrap_or(Value::Null);
    let row: Option<(i64, Option<i64>)> =
        tx.query_row("SELECT folder_id, thread_id FROM message WHERE id = ?1", [message_id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
    let Some((folder, thread)) = row else { return Ok(()) };
    match kind {
        "move" => {
            let Some(back) = u["folder_id"].as_i64() else { return Ok(()) };
            // Back with its old UID, unless another row has it meanwhile.
            tx.execute(
                "UPDATE message SET folder_id = ?2, date_modified = unixepoch(), modified_by = 'core',
                        uid = CASE WHEN EXISTS (SELECT 1 FROM message o WHERE o.folder_id = ?2 AND o.uid = ?3) THEN NULL ELSE ?3 END
                  WHERE id = ?1",
                params![message_id, back, u["uid"].as_u64().map(|v| v as i64)],
            )?;
            changes.folders.insert(folder);
            changes.folders.insert(back);
        }
        _ => {
            let (Some(flag), Some(set)) = (p["flag"].as_str(), u["set"].as_bool()) else { return Ok(()) };
            match flag.to_ascii_lowercase().as_str() {
                "\\seen" => tx.execute("UPDATE message SET is_seen = ?2 WHERE id = ?1", params![message_id, set])?,
                "\\flagged" => tx.execute("UPDATE message SET is_flagged = ?2 WHERE id = ?1", params![message_id, set])?,
                _ if set => tx.execute("INSERT OR IGNORE INTO message_keyword (message_id, keyword) VALUES (?1, ?2)", params![message_id, flag])?,
                _ => tx.execute("DELETE FROM message_keyword WHERE message_id = ?1 AND keyword = ?2 COLLATE NOCASE", params![message_id, flag])?,
            };
            tx.execute("UPDATE message SET date_modified = unixepoch(), modified_by = 'core' WHERE id = ?1", [message_id])?;
            changes.folders.insert(folder);
        }
    }
    changes.messages.insert((message_id, thread));
    threads.extend(thread);
    Ok(())
}

/// Refuses a command that changes the mail of a thread of a read-only
/// account (`Store::set_read_only_accounts`).
pub(crate) fn refuse_read_only(conn: &Connection, thread_id: i64) -> Result<()> {
    let ro: bool = conn
        .prepare_cached("SELECT EXISTS (SELECT 1 FROM thread t JOIN temp.read_only_account r ON r.id = t.account_id WHERE t.id = ?1)")?
        .query_row([thread_id], |r| r.get(0))?;
    if ro {
        return Err(crate::StoreError::ReadOnly);
    }
    Ok(())
}
