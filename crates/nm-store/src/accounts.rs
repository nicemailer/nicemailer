//! Accounts of `config.toml` in the database (`configuration.md`, section 6).
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
//! Every creation, disabling and enabling is written to `audit_log`
//! (`tracabilite.md`), source `core`: the core cannot know who edited the
//! file, the user's Git history does.

use std::collections::BTreeMap;

use rusqlite::{OptionalExtension, TransactionBehavior, params};

use crate::{Result, Store};

/// An account as stored in the database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountRow {
    pub id: i64,
    /// Key in `config.toml`, or name of the archive account.
    pub config_key: String,
    /// `imap` or `archive`.
    pub kind: String,
    /// Address of an archive account; `None` for an IMAP account, whose
    /// address is in the file.
    pub address: Option<String>,
    /// Whether the key is missing from the file (IMAP accounts only).
    pub disabled: bool,
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
                        "INSERT INTO account (config_key, kind, created_by, modified_by) VALUES (?1, 'imap', 'core', 'core')",
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
            "SELECT id, config_key, kind, address, disabled_at IS NOT NULL FROM account ORDER BY id",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(AccountRow {
                    id: r.get(0)?,
                    config_key: r.get(1)?,
                    kind: r.get(2)?,
                    address: r.get(3)?,
                    disabled: r.get(4)?,
                })
            })?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }

    /// Records a change of `config.toml` seen when reloading it
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
