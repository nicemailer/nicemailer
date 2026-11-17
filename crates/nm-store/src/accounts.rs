//! Accounts of `accounts.toml` in the database (`configuration.md`, section 6).
//!
//! The file holds the user's intentions (addresses, servers), the database
//! holds the state (folders, messages, detected values). The `account` table
//! links the two: one row per key of `[accounts.<key>]`, whose id is what
//! folders, messages and threads refer to.
//!
//! The rules implemented here:
//! - a key that appears in the file gets a row, created the first time;
//! - a key that leaves the file disables its account (`disabled_at`), and
//!   nothing is deleted: going back to an older version of the file in Git,
//!   or taking the file of another Mac, must not erase gigabytes of cached
//!   mail;
//! - a key that comes back finds its account intact, enabled again;
//! - archive accounts (`kind = 'archive'`, created by an mbox import) are
//!   never in the file, and are left alone.
//!
//! Accounts also have a display order, `account.position`: the order of
//! the account picker (`@`) and of the sidebar, which follow the folder
//! list (`list_folders`). A new account goes after the others; the user
//! changes the order with [`Store::set_account_order`]. It is a state of the
//! interface, not an intention about the servers, hence in the database and
//! not in the file.
//!
//! Every creation, disabling and enabling is written to `audit_log`
//! (`tracabilite.md`), source `core`: the core cannot know who edited the
//! file, the user's Git history does.

use std::collections::BTreeMap;

use nm_core::Source;
use rusqlite::{OptionalExtension, TransactionBehavior, params};

use crate::{ChangeSet, Result, Store, StoreError};

/// An account as stored in the database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountRow {
    pub id: i64,
    /// Key in `accounts.toml`, or name of the archive account.
    pub config_key: String,
    /// `imap` or `archive`.
    pub kind: String,
    /// Address of an archive account; `None` for an IMAP account, whose
    /// address is in the file.
    pub address: Option<String>,
    /// Whether the key is missing from the file (IMAP accounts only).
    pub disabled: bool,
    /// Display order: rows are listed by `position`, then `id`.
    pub position: i64,
}

/// What [`Store::sync_config_accounts`] changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccountSync {
    /// Id of the account of each key of the file, except the clashes.
    pub ids: BTreeMap<String, i64>,
    /// Accounts created by this call.
    pub created: Vec<i64>,
    /// Accounts enabled again: their key came back.
    pub enabled: Vec<i64>,
    /// Accounts disabled: their key left the file.
    pub disabled: Vec<i64>,
    /// Keys of the file already used by an archive account. They get no
    /// row: `config_key` is unique, and the archive account must not
    /// become an IMAP one by accident. The user renames one of the two.
    pub clashes: Vec<String>,
}

impl Store {
    /// Brings the `account` table in line with the keys of the IMAP accounts
    /// of the file, in one transaction.
    pub fn sync_config_accounts(&mut self, keys: &[&str]) -> Result<AccountSync> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut out = AccountSync::default();
        let audit = |tx: &rusqlite::Transaction, id: i64, action: &str, key: &str| {
            tx.execute(
                "INSERT INTO audit_log (created_by, entity, entity_id, action, detail)
                 VALUES ('core', 'account', ?1, ?2, json_object('config_key', ?3))",
                params![id, action, key],
            )
        };
        for &key in keys {
            // Rust note: `optional()` turns "no row" into `Ok(None)` instead
            // of an error, so a missing account is an ordinary case.
            let row: Option<(i64, String, Option<i64>)> = tx
                .query_row("SELECT id, kind, disabled_at FROM account WHERE config_key = ?1", [key], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })
                .optional()?;
            match row {
                Some((_, kind, _)) if kind != "imap" => out.clashes.push(key.to_string()),
                Some((id, _, disabled_at)) => {
                    if disabled_at.is_some() {
                        tx.execute(
                            "UPDATE account SET disabled_at = NULL, date_modified = unixepoch(), modified_by = 'core'
                              WHERE id = ?1",
                            [id],
                        )?;
                        audit(&tx, id, "enable", key)?;
                        out.enabled.push(id);
                    }
                    out.ids.insert(key.to_string(), id);
                }
                None => {
                    tx.execute(
                        // After the other accounts in the display order. Keys
                        // created by the same reading come in the order of
                        // `keys`, sorted by name.
                        "INSERT INTO account (config_key, kind, created_by, modified_by, position)
                         VALUES (?1, 'imap', 'core', 'core', (SELECT coalesce(max(position), 0) + 1 FROM account))",
                        [key],
                    )?;
                    let id = tx.last_insert_rowid();
                    audit(&tx, id, "create", key)?;
                    out.created.push(id);
                    out.ids.insert(key.to_string(), id);
                }
            }
        }
        // Active IMAP accounts whose key is not in the file any more.
        let gone: Vec<(i64, String)> = {
            let mut stmt =
                tx.prepare("SELECT id, config_key FROM account WHERE kind = 'imap' AND disabled_at IS NULL")?;
            let active: Vec<(i64, String)> =
                stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<std::result::Result<_, _>>()?;
            active.into_iter().filter(|(_, k)| !keys.contains(&k.as_str())).collect()
        };
        for (id, key) in gone {
            tx.execute(
                "UPDATE account SET disabled_at = unixepoch(), date_modified = unixepoch(), modified_by = 'core'
                  WHERE id = ?1",
                [id],
            )?;
            audit(&tx, id, "disable", &key)?;
            out.disabled.push(id);
        }
        tx.commit()?;
        Ok(out)
    }

    /// Every account, IMAP and archive, by id.
    pub fn list_account_rows(&self) -> Result<Vec<AccountRow>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, config_key, kind, address, disabled_at IS NOT NULL, position FROM account ORDER BY position, id",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(AccountRow {
                    id: r.get(0)?,
                    config_key: r.get(1)?,
                    kind: r.get(2)?,
                    address: r.get(3)?,
                    disabled: r.get(4)?,
                    position: r.get(5)?,
                })
            })?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }

    /// Puts the accounts `ids` first, in this order, and the others after
    /// them in their current order; positions become 1, 2, 3…
    ///
    /// A partial list is accepted on purpose: a client that shows only some
    /// accounts (not the disabled ones, say) can reorder what it shows
    /// without knowing the rest. An unknown or repeated id is refused, so a
    /// stale client cannot reorder by mistake.
    ///
    /// Each account whose position changes gets a line in `audit_log`
    /// (`reorder`, with before and after) and its folders are reported as
    /// changed: the order of the folder list, which the app's account
    /// picker follows, is what changed for the clients.
    pub fn set_account_order(&mut self, ids: &[i64], source: Source) -> Result<ChangeSet> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current: Vec<(i64, i64)> = {
            let mut stmt = tx.prepare("SELECT id, position FROM account ORDER BY position, id")?;
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<std::result::Result<_, _>>()?
        };
        for (i, id) in ids.iter().enumerate() {
            if !current.iter().any(|(c, _)| c == id) {
                return Err(StoreError::NotFound(format!("account {id}")));
            }
            if ids[..i].contains(id) {
                return Err(StoreError::InvalidParams(format!("account {id} given twice")));
            }
        }
        let order: Vec<(i64, i64)> = ids
            .iter()
            .map(|id| *current.iter().find(|(c, _)| c == id).expect("checked above"))
            .chain(current.iter().filter(|(c, _)| !ids.contains(c)).copied())
            .collect();
        let mut changes = ChangeSet::default();
        for (i, (id, before)) in order.into_iter().enumerate() {
            let after = i as i64 + 1;
            if before == after {
                continue;
            }
            tx.execute(
                "UPDATE account SET position = ?2, date_modified = unixepoch(), modified_by = ?3 WHERE id = ?1",
                params![id, after, source.as_str()],
            )?;
            tx.execute(
                "INSERT INTO audit_log (created_by, entity, entity_id, action, detail)
                 VALUES (?1, 'account', ?2, 'reorder', json_object('before', ?3, 'after', ?4))",
                params![source.as_str(), id, before, after],
            )?;
            changes.accounts.insert(id);
            let mut stmt = tx.prepare_cached("SELECT id FROM folder WHERE account_id = ?1")?;
            for f in stmt.query_map([id], |r| r.get::<_, i64>(0))? {
                changes.folders.insert(f?);
            }
        }
        tx.commit()?;
        Ok(changes)
    }

    /// Records that the password of an account was saved, for which server,
    /// and by which client (`tracabilite.md`). Never the password.
    pub fn record_password_saved(&self, account_id: i64, host: &str, username: &str, source: Source) -> Result<()> {
        self.conn.execute(
            "INSERT INTO audit_log (created_by, entity, entity_id, action, detail)
             VALUES (?1, 'account', ?2, 'set_password', json_object('host', ?3, 'username', ?4))",
            params![source.as_str(), account_id, host, username],
        )?;
        Ok(())
    }

    /// Records a change of `config.toml` or `accounts.toml` seen when reloading it
    /// (`configuration.md`, section 10): action `reload`, source `core`,
    /// with the keys that changed. Only the keys: the values may include a
    /// password (development builds), which never goes to the database.
    pub fn record_config_reload(&self, keys: &[String]) -> Result<()> {
        let detail = serde_json::json!({ "keys": keys }).to_string();
        self.conn.execute(
            "INSERT INTO audit_log (created_by, entity, entity_id, action, detail) VALUES ('core', 'config', 0, 'reload', ?1)",
            [detail],
        )?;
        Ok(())
    }
}
