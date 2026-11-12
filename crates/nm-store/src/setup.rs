//! The database side of creating an IMAP account and of its initial
//! import (`import-initial.md`; `dossiers.md`, "A la création du compte";
//! `plan-imap.md`, phase 7). The engine reads the server; these functions
//! write what was decided.
//!
//! Order of a creation (`nm-daemon`, `setup.rs`): the account row, the
//! roles of its folders, the screening boundary of its Inbox, the
//! decisions on its correspondents, the server-side archiving queued, then
//! the password, then `accounts.toml`, which starts the account's worker. The
//! worker sends the archiving first, and waits for it before the Inbox
//! (`Store::pending_initial_archive`): each old mail is fetched once,
//! straight into its final folder.

use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};

use nm_core::{FolderRole, Source};

use crate::{ChangeSet, Result, Store, StoreError};

/// One folder of the server, and the workflow role it gets (or `None`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleAssignment {
    pub path: String,
    pub delimiter: Option<String>,
    pub role: Option<FolderRole>,
}

/// The criteria of the server-side archiving (`import-initial.md`, section
/// 6): the mails of the Inbox below the boundary, not flagged, read before
/// `read_before` or unread before `unread_before` (days, `YYYY-MM-DD`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveCriteria {
    pub from_path: String,
    pub to_path: String,
    pub boundary: u32,
    pub read_before: Option<String>,
    pub unread_before: Option<String>,
    pub batch_size: u32,
}

/// An action of the spool on the account rather than on a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountAction {
    pub id: i64,
    pub criteria: ArchiveCriteria,
}

impl Store {
    /// The row of an IMAP account about to be created, or the existing one
    /// (a creation tried again). Created active, after the other accounts.
    pub fn prepare_imap_account(&mut self, key: &str, source: Source) -> Result<i64> {
        let tx = self.transaction()?;
        let row: Option<(i64, String)> =
            tx.query_row("SELECT id, kind FROM account WHERE config_key = ?1", [key], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
        let id = match row {
            Some((_, kind)) if kind != "imap" => {
                return Err(StoreError::InvalidParams(format!("{key} is an {kind} account: choose another key")));
            }
            Some((id, _)) => {
                tx.execute("UPDATE account SET disabled_at = NULL WHERE id = ?1", [id])?;
                id
            }
            None => {
                tx.execute(
                    "INSERT INTO account (config_key, kind, created_by, modified_by, position)
                     VALUES (?1, 'imap', ?2, ?2, (SELECT coalesce(max(position), 0) + 1 FROM account))",
                    params![key, source.as_str()],
                )?;
                let id = tx.last_insert_rowid();
                tx.execute(
                    "INSERT INTO audit_log (created_by, entity, entity_id, action, detail)
                     VALUES (?1, 'account', ?2, 'create', json_object('config_key', ?3, 'kind', 'imap'))",
                    params![source.as_str(), id, key],
                )?;
                id
            }
        };
        tx.commit()?;
        Ok(id)
    }

    /// Gives the folders of an account their roles, as the user confirmed
    /// them (`dossiers.md`, "A la création du compte"). A folder not known
    /// yet gets a row; a role moves from the folder that had it; a folder
    /// listed with `None` loses its role. Each change has a `role` row in
    /// `audit_log` with the client's source.
    pub fn set_folder_roles(&mut self, account_id: i64, folders: &[RoleAssignment], source: Source) -> Result<ChangeSet> {
        let tx = self.transaction()?;
        let mut changes = ChangeSet::default();
        for f in folders {
            tx.execute(
                "INSERT INTO folder (account_id, path, delimiter, created_by, modified_by) VALUES (?1, ?2, ?3, 'imap', 'imap')
                 ON CONFLICT (account_id, path) DO NOTHING",
                params![account_id, f.path, f.delimiter],
            )?;
            let (id, before): (i64, Option<String>) =
                tx.query_row("SELECT id, role FROM folder WHERE account_id = ?1 AND path = ?2", params![account_id, f.path], |r| Ok((r.get(0)?, r.get(1)?)))?;
            let after = f.role.map(|r| r.as_str().to_string());
            if before == after {
                continue;
            }
            if let Some(role) = &after {
                // One folder per role: whoever had it loses it.
                let others: Vec<i64> = tx
                    .prepare("SELECT id FROM folder WHERE account_id = ?1 AND role = ?2 AND id <> ?3")?
                    .query_map(params![account_id, role, id], |r| r.get(0))?
                    .collect::<std::result::Result<_, _>>()?;
                for o in others {
                    set_role(&tx, o, None, Some(role), source)?;
                    changes.folders.insert(o);
                }
            }
            set_role(&tx, id, after.as_deref(), before.as_deref(), source)?;
            changes.folders.insert(id);
        }
        tx.commit()?;
        Ok(changes)
    }

    /// The people the user wrote to get the decision `inbox`, if they have
    /// none and never had one (a decision removed is a choice too), except
    /// the user's own addresses (`import-initial.md`, section 4). Unknown
    /// addresses are created, `imported_from = 'imap_initial'`. Returns how
    /// many got the decision.
    pub fn add_correspondents(&mut self, addrs: &[String], own: &[String]) -> Result<u64> {
        let tx = self.transaction()?;
        let mut n = 0;
        for addr in addrs {
            let addr = addr.trim().to_lowercase();
            if !addr.contains('@') || own.contains(&addr) {
                continue;
            }
            tx.execute(
                "INSERT INTO address (addr, imported_from, created_by, modified_by) VALUES (?1, 'imap_initial', 'core', 'core')
                 ON CONFLICT (addr) DO NOTHING",
                [&addr],
            )?;
            let updated = tx.execute(
                "UPDATE address SET screening_target = 'inbox', date_screening_modified = unixepoch(),
                        date_modified = unixepoch(), modified_by = 'core'
                  WHERE addr = ?1 AND screening_target IS NULL AND date_screening_modified IS NULL",
                [&addr],
            )?;
            if updated > 0 {
                tx.execute(
                    "INSERT INTO audit_log (created_by, entity, entity_id, action, detail)
                     SELECT 'core', 'address', id, 'screening',
                            json_object('before', NULL, 'after', 'inbox', 'reason', 'sent_correspondent')
                       FROM address WHERE addr = ?1",
                    [&addr],
                )?;
                n += 1;
            }
        }
        tx.commit()?;
        Ok(n)
    }

    /// Queues the server-side archiving of the initial import: one action
    /// on the account, without message (`import-initial.md`, section 6).
    pub fn queue_initial_archive(&mut self, account_id: i64, c: &ArchiveCriteria, source: Source) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO pending_action (account_id, kind, message_id, payload, created_by)
             VALUES (?1, 'initial_archive', NULL, ?2, ?3)",
            params![account_id, criteria_json(c).to_string(), source.as_str()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Whether an archiving of the initial import is still to run: the
    /// Inbox is not synchronized before it ends.
    pub fn pending_initial_archive(&self, account_id: i64) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM pending_action WHERE account_id = ?1 AND kind = 'initial_archive'
                             AND state IN ('queued', 'running'))",
            [account_id],
            |r| r.get(0),
        )?)
    }

    /// The archiving to run now, if due, marked `running`.
    pub fn spool_claim_account(&mut self, account_id: i64) -> Result<Vec<AccountAction>> {
        let tx = self.transaction()?;
        let rows: Vec<(i64, String)> = tx
            .prepare(
                "SELECT id, payload FROM pending_action
                  WHERE account_id = ?1 AND kind = 'initial_archive' AND state = 'queued' AND date_run_after <= unixepoch()
                  ORDER BY id",
            )?
            .query_map([account_id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?;
        let mut out = Vec::new();
        for (id, payload) in rows {
            let Some(criteria) = criteria_of(&payload) else { continue };
            tx.execute("UPDATE pending_action SET state = 'running', date_modified = unixepoch() WHERE id = ?1", [id])?;
            out.push(AccountAction { id, criteria });
        }
        tx.commit()?;
        Ok(out)
    }

    /// The archiving ended: done, with one `initial_archive` row on the
    /// account in `audit_log`, with the criteria and the number of mails
    /// moved (not one row per mail).
    pub fn initial_archive_done(&mut self, action_id: i64, moved: u64) -> Result<()> {
        let tx = self.transaction()?;
        let (account, payload, source): (i64, String, String) = tx.query_row(
            "SELECT account_id, payload, created_by FROM pending_action WHERE id = ?1",
            [action_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        tx.execute("UPDATE pending_action SET state = 'done', last_error = NULL, date_modified = unixepoch() WHERE id = ?1", [action_id])?;
        tx.execute(
            "INSERT INTO audit_log (created_by, entity, entity_id, action, detail)
             VALUES (?1, 'account', ?2, 'initial_archive', json_patch(?3, json_object('moved', ?4)))",
            params![source, account, payload, moved as i64],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// The archiving failed for good (no Archive folder, say): nothing was
    /// changed locally, there is nothing to undo; the action is `reverted`
    /// with the reason, which `sync status` shows.
    pub fn initial_archive_failed(&mut self, action_id: i64, reason: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE pending_action SET state = 'reverted', last_error = ?2, date_modified = unixepoch() WHERE id = ?1",
            params![action_id, reason],
        )?;
        Ok(())
    }

    /// A temporary failure of the archiving: tried again later, like any
    /// action; after the last attempt, failed for good.
    pub fn initial_archive_retry(&mut self, action_id: i64, error: &str) -> Result<()> {
        let attempts: i64 = self.conn.query_row("SELECT attempts + 1 FROM pending_action WHERE id = ?1", [action_id], |r| r.get(0))?;
        if attempts >= crate::spool::MAX_ATTEMPTS {
            return self.initial_archive_failed(action_id, &format!("{attempts} attempts failed, the last: {error}"));
        }
        let delay = (crate::spool::RETRY_FIRST_SECS << (attempts - 1).min(20)).min(crate::spool::RETRY_MAX_SECS);
        self.conn.execute(
            "UPDATE pending_action SET state = 'queued', attempts = ?2, last_error = ?3, date_run_after = unixepoch() + ?4,
                    date_modified = unixepoch() WHERE id = ?1",
            params![action_id, attempts, error, delay],
        )?;
        Ok(())
    }

    /// The last archiving of an account, if any: its state and error.
    pub fn initial_archive_state(&self, account_id: i64) -> Result<Option<(String, Option<String>)>> {
        Ok(self
            .conn
            .query_row(
                "SELECT state, last_error FROM pending_action WHERE account_id = ?1 AND kind = 'initial_archive' ORDER BY id DESC LIMIT 1",
                [account_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }

    /// Renames the key of an account, keeping its data
    /// (`configuration.md`, section 6): the database first, so that the file
    /// changed next finds the account under its new key.
    pub fn rename_account(&mut self, from: &str, to: &str, source: Source) -> Result<i64> {
        let tx = self.transaction()?;
        let id: i64 = tx
            .query_row("SELECT id FROM account WHERE config_key = ?1 AND kind = 'imap'", [from], |r| r.get(0))
            .optional()?
            .ok_or_else(|| StoreError::NotFound(format!("account {from}")))?;
        let taken: bool = tx.query_row("SELECT EXISTS (SELECT 1 FROM account WHERE config_key = ?1)", [to], |r| r.get(0))?;
        if taken {
            return Err(StoreError::InvalidParams(format!("an account {to} exists already")));
        }
        tx.execute(
            "UPDATE account SET config_key = ?2, date_modified = unixepoch(), modified_by = ?3 WHERE id = ?1",
            params![id, to, source.as_str()],
        )?;
        tx.execute(
            "INSERT INTO audit_log (created_by, entity, entity_id, action, detail)
             VALUES (?1, 'account', ?2, 'rename', json_object('before', ?3, 'after', ?4))",
            params![source.as_str(), id, from, to],
        )?;
        tx.commit()?;
        Ok(id)
    }

    /// Deletes an account and its data (`account remove`): folders,
    /// messages, threads, actions go with it (`ON DELETE CASCADE`). The raw
    /// files stay until the garbage collector (phase 8). Journaled on the
    /// account, which the journal outlives.
    pub fn remove_account(&mut self, key: &str, source: Source) -> Result<ChangeSet> {
        let tx = self.transaction()?;
        let id: i64 = tx
            .query_row("SELECT id FROM account WHERE config_key = ?1 AND kind = 'imap'", [key], |r| r.get(0))
            .optional()?
            .ok_or_else(|| StoreError::NotFound(format!("account {key}")))?;
        let messages: i64 = tx.query_row("SELECT count(*) FROM message WHERE account_id = ?1", [id], |r| r.get(0))?;
        tx.execute("DELETE FROM thread WHERE account_id = ?1", [id])?;
        tx.execute("DELETE FROM account WHERE id = ?1", [id])?;
        tx.execute(
            "INSERT INTO audit_log (created_by, entity, entity_id, action, detail)
             VALUES (?1, 'account', ?2, 'remove', json_object('config_key', ?3, 'messages', ?4))",
            params![source.as_str(), id, key, messages],
        )?;
        tx.commit()?;
        let mut changes = ChangeSet::default();
        changes.accounts.insert(id);
        Ok(changes)
    }
}

/// Gives a folder its role (or none) and records it. The folder of a
/// conversation follows from the roles of its mails' folders (`dossiers.md`,
/// "Dossier d'une conversation"): every thread with a mail in this folder is
/// recomputed in the same transaction. At the creation of an account the
/// folders are still empty; a role changed later, by the synchronized
/// configuration, may recompute a whole folder, once.
fn set_role(tx: &rusqlite::Transaction, folder: i64, role: Option<&str>, before: Option<&str>, source: Source) -> Result<()> {
    tx.execute(
        "UPDATE folder SET role = ?2, date_role_modified = unixepoch(), date_modified = unixepoch(), modified_by = ?3 WHERE id = ?1",
        params![folder, role, source.as_str()],
    )?;
    let threads: Vec<i64> = tx
        .prepare("SELECT DISTINCT thread_id FROM message WHERE folder_id = ?1 AND thread_id IS NOT NULL")?
        .query_map([folder], |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    for t in threads {
        crate::refresh_thread(tx, t)?;
    }
    tx.execute(
        "INSERT INTO audit_log (created_by, entity, entity_id, action, detail)
         VALUES (?1, 'folder', ?2, 'role', json_object('before', ?3, 'after', ?4))",
        params![source.as_str(), folder, before, role],
    )?;
    Ok(())
}

fn criteria_json(c: &ArchiveCriteria) -> Value {
    json!({
        "from": c.from_path, "to": c.to_path, "boundary": c.boundary,
        "read_before": c.read_before, "unread_before": c.unread_before, "batch_size": c.batch_size,
    })
}

fn criteria_of(payload: &str) -> Option<ArchiveCriteria> {
    let v: Value = serde_json::from_str(payload).ok()?;
    Some(ArchiveCriteria {
        from_path: v["from"].as_str()?.to_string(),
        to_path: v["to"].as_str()?.to_string(),
        boundary: v["boundary"].as_u64()? as u32,
        read_before: v["read_before"].as_str().map(String::from),
        unread_before: v["unread_before"].as_str().map(String::from),
        batch_size: v["batch_size"].as_u64().unwrap_or(1000).max(1) as u32,
    })
}
