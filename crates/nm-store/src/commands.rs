//! Write commands on a thread (`dossiers.md`, "Déplacer un mail").
//!
//! v1 limits: commands are only applied to the local database. There is no
//! spool of actions to replay on the IMAP server (`pending_action`) since
//! there is no server yet, and no screening decision on the sender either
//! (moving a newsletter to Newsletter does not file the sender's future
//! mail there).
//!
//! Every command follows the same pattern:
//! 1. validate the parameters,
//! 2. open an IMMEDIATE transaction (it takes the write lock right away,
//!    so two writers cannot both read and then conflict),
//! 3. modify the messages and write one `audit_log` row per change,
//! 4. recompute the denormalized aggregates,
//! 5. commit and return a `ChangeSet` describing what changed.

use std::collections::BTreeSet;

use nm_core::{FolderRole, KW_WAS_TODO, KW_WAS_WAITING, Source};
// `OptionalExtension` adds `.optional()` to query results: it turns the
// "no row" error into `Ok(None)`. `params!` builds a list of SQL parameters
// of different types.
use rusqlite::{OptionalExtension, TransactionBehavior, params};

// `crate::` refers to the root of this crate, i.e. `lib.rs`.
use crate::{ChangeSet, Result, Store, StoreError, refresh_folder_counts, refresh_thread};

impl Store {
    /// Returns the account of a thread, or `NotFound` if it does not exist.
    /// Also serves as an existence check at the start of each command.
    ///
    /// Rust note: `ok_or_else` turns an `Option` into a `Result`: `Some(x)`
    /// becomes `Ok(x)` and `None` becomes the error built by the closure.
    fn thread_account(&self, thread_id: i64) -> Result<i64> {
        self.conn
            .query_row("SELECT account_id FROM thread WHERE id = ?1", [thread_id], |r| r.get(0))
            .optional()?
            .ok_or_else(|| StoreError::NotFound(format!("conversation {thread_id}")))
    }

    /// Moves a thread to the folder with role `target`.
    ///
    /// Only the thread's messages filed in Inbox, Todo, Waiting, Archive,
    /// Newsletter and Paperwork follow (`CONVERSATION_SCOPE`), plus those in
    /// the folder where the command is given (`from_folder_id`). So the
    /// copies in Sent never move, and a message the user deleted stays in
    /// Deleted unless the user is restoring it from there.
    ///
    /// Rust note: `&mut self` means the method needs exclusive access to the
    /// store, because opening a transaction requires a mutable connection.
    pub fn move_thread(
        &mut self,
        thread_id: i64,
        target: FolderRole,
        from_folder_id: Option<i64>,
        source: Source,
    ) -> Result<ChangeSet> {
        let account_id = self.thread_account(thread_id)?;

        // Role of the folder where the command is given, if any.
        let from_role = match from_folder_id {
            Some(fid) => {
                // `role` is nullable in the schema, hence `Option<String>`.
                // The query itself may also find no row, hence `optional()`
                // followed by `ok_or_else`.
                let role: Option<String> = self
                    .conn
                    .query_row("SELECT role FROM folder WHERE id = ?1", [fid], |r| r.get(0))
                    .optional()?
                    .ok_or_else(|| StoreError::NotFound(format!("dossier {fid}")))?;
                // `as_deref` turns `Option<String>` into `Option<&str>`;
                // `and_then` applies `parse`, which itself returns an Option.
                role.as_deref().and_then(FolderRole::parse)
            }
            None => None,
        };
        let allowed = match from_role {
            Some(r) => r.move_targets().contains(&target),
            // Without context: any command target, but never Sent, Drafts or
            // Quarantine, which only the core may fill.
            None => !matches!(target, FolderRole::Sent | FolderRole::Drafts | FolderRole::Quarantine),
        };
        if !allowed {
            return Err(StoreError::InvalidMove(format!(
                "{} n'est pas une cible depuis {}",
                target.as_str(),
                from_role.map(|r| r.as_str()).unwrap_or("ce dossier")
            )));
        }

        // From here on every statement goes through `tx`. If the function
        // returns early with an error (`?`), `tx` is dropped without commit
        // and SQLite rolls everything back automatically.
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let target_id: i64 = tx
            .query_row(
                "SELECT id FROM folder WHERE account_id = ?1 AND role = ?2",
                params![account_id, target.as_str()],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound(format!("dossier {}", target.as_str())))?;

        // Messages to move, with their current folder (needed for the audit
        // log and to refresh the old folder's counters). The six roles of the
        // scope are passed as separate parameters (?3 to ?8); `-1` stands
        // for "no from_folder" since no folder has that id.
        let scope: Vec<&str> = FolderRole::CONVERSATION_SCOPE.iter().map(|r| r.as_str()).collect();
        let moved: Vec<(i64, i64)> = {
            let mut stmt = tx.prepare(
                "SELECT m.id, m.folder_id FROM message m JOIN folder f ON f.id = m.folder_id
                  WHERE m.thread_id = ?1 AND m.folder_id <> ?2
                    AND (f.role IN (?3, ?4, ?5, ?6, ?7, ?8) OR m.folder_id = ?9)",
            )?;
            stmt.query_map(
                params![
                    thread_id, target_id, scope[0], scope[1], scope[2], scope[3], scope[4], scope[5],
                    from_folder_id.unwrap_or(-1)
                ],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?
            // Each row is a `Result`; collecting into `Result<Vec<_>, _>`
            // stops at the first error, which the final `?` propagates.
            .collect::<std::result::Result<_, _>>()?
        };

        let mut changes = ChangeSet::default();
        changes.folders.insert(target_id);
        // `&moved` iterates by reference, so `message_id` and `old_folder` are
        // `&i64`; `*old_folder` reads the value behind the reference.
        for (message_id, old_folder) in &moved {
            // The IMAP UID is only valid within a given server folder, so it
            // is cleared on move; the future IMAP sync will learn the new one.
            tx.execute(
                "UPDATE message SET folder_id = ?2, uid = NULL,
                        date_modified = unixepoch(), modified_by = ?3
                  WHERE id = ?1",
                params![message_id, target_id, source.as_str()],
            )?;
            tx.execute(
                "INSERT INTO audit_log (created_by, entity, entity_id, action, detail)
                 VALUES (?1, 'message', ?2, 'move',
                         json_object('before', json_object('folder_id', ?3),
                                     'after',  json_object('folder_id', ?4)))",
                params![source.as_str(), message_id, old_folder, target_id],
            )?;
            changes.folders.insert(*old_folder);
            changes.messages.insert((*message_id, Some(thread_id)));
        }

        // Any command clears the "came back from Todo/Waiting" mark, even when
        // nothing moved: by acting on the thread, the user has dealt with it.
        // `COLLATE NOCASE` because IMAP keywords are case-insensitive.
        let unmarked = tx.execute(
            "DELETE FROM message_keyword
              WHERE keyword COLLATE NOCASE IN (?2, ?3)
                AND message_id IN (SELECT id FROM message WHERE thread_id = ?1)",
            params![thread_id, KW_WAS_TODO, KW_WAS_WAITING],
        )?;

        // Recompute aggregates only if something actually changed.
        if !moved.is_empty() || unmarked > 0 {
            let folders = refresh_thread(&tx, thread_id)?;
            for f in &changes.folders {
                refresh_folder_counts(&tx, *f)?;
            }
            changes.threads.push((thread_id, folders));
        }
        tx.commit()?;
        Ok(changes)
    }

    /// Marks every message of the thread as read or unread.
    pub fn set_thread_seen(&mut self, thread_id: i64, seen: bool, source: Source) -> Result<ChangeSet> {
        self.thread_account(thread_id)?;
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        // `RETURNING` (SQLite 3.35+) gives back the rows actually updated, so
        // a single statement both modifies and tells us what changed. The
        // `is_seen <> ?2` condition skips messages already in the wanted state.
        let changed: Vec<(i64, i64)> = {
            let mut stmt = tx.prepare(
                "UPDATE message SET is_seen = ?2, date_modified = unixepoch(), modified_by = ?3
                  WHERE thread_id = ?1 AND is_seen <> ?2
                  RETURNING id, folder_id",
            )?;
            stmt.query_map(params![thread_id, seen, source.as_str()], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<std::result::Result<_, _>>()?
        };
        let changes = finish_flag_change(&tx, thread_id, &changed, source, "set_flags", "is_seen", seen)?;
        tx.commit()?;
        Ok(changes)
    }

    /// Flags the last message received in the thread, or unflags all of
    /// its messages.
    ///
    /// Flagging a single message keeps IMAP semantics (a flag belongs to a
    /// message) while the UI shows the thread as flagged when any of its
    /// messages is.
    pub fn set_thread_flagged(&mut self, thread_id: i64, flagged: bool, source: Source) -> Result<ChangeSet> {
        self.thread_account(thread_id)?;
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        // In Rust `if` is an expression: both branches produce the `Vec`.
        let changed: Vec<(i64, i64)> = if flagged {
            // Last received message, excluding Sent and Drafts; if the thread
            // only has those, the most recent message overall. The first
            // ORDER BY term is 0 for received mail and 1 for Sent/Drafts, so
            // received mail sorts first.
            let target: Option<i64> = tx
                .query_row(
                    "SELECT m.id FROM message m JOIN folder f ON f.id = m.folder_id
                      WHERE m.thread_id = ?1
                      ORDER BY (coalesce(f.role, '') IN ('sent', 'drafts')), m.date_received DESC, m.id DESC
                      LIMIT 1",
                    [thread_id],
                    |r| r.get(0),
                )
                .optional()?;
            let mut stmt = tx.prepare(
                "UPDATE message SET is_flagged = 1, date_modified = unixepoch(), modified_by = ?2
                  WHERE id = ?1 AND is_flagged = 0 RETURNING id, folder_id",
            )?;
            match target {
                Some(id) => stmt
                    .query_map(params![id, source.as_str()], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<std::result::Result<_, _>>()?,
                None => vec![],
            }
        } else {
            let mut stmt = tx.prepare(
                "UPDATE message SET is_flagged = 0, date_modified = unixepoch(), modified_by = ?2
                  WHERE thread_id = ?1 AND is_flagged = 1 RETURNING id, folder_id",
            )?;
            stmt.query_map(params![thread_id, source.as_str()], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<std::result::Result<_, _>>()?
        };
        let changes = finish_flag_change(&tx, thread_id, &changed, source, "set_flags", "is_flagged", flagged)?;
        tx.commit()?;
        Ok(changes)
    }
}

/// Common end of the flag commands (seen, flagged): writes the audit log,
/// refreshes the aggregates and builds the `ChangeSet`.
///
/// `changed` holds the `(message_id, folder_id)` pairs actually modified.
/// `flag` is the name of the column, used as the JSON key in the audit log.
///
/// Rust note: `&[(i64, i64)]` is a *slice*, a borrowed view on a sequence of
/// pairs. A `&Vec<...>` converts to it automatically.
fn finish_flag_change(
    tx: &rusqlite::Transaction,
    thread_id: i64,
    changed: &[(i64, i64)],
    source: Source,
    action: &str,
    flag: &str,
    value: bool,
) -> Result<ChangeSet> {
    let mut changes = ChangeSet::default();
    // Nothing changed: no audit, no refresh, no events.
    if changed.is_empty() {
        return Ok(changes);
    }
    // Distinct folders touched by the change.
    let folders: BTreeSet<i64> = changed.iter().map(|(_, f)| *f).collect();
    for (message_id, _) in changed {
        tx.execute(
            "INSERT INTO audit_log (created_by, entity, entity_id, action, detail)
             VALUES (?1, 'message', ?2, ?3,
                     json_object('before', json_object(?4, ?5), 'after', json_object(?4, ?6)))",
            params![source.as_str(), message_id, action, flag, !value, value],
        )?;
        changes.messages.insert((*message_id, Some(thread_id)));
    }
    let thread_folders = refresh_thread(tx, thread_id)?;
    for f in &folders {
        refresh_folder_counts(tx, *f)?;
    }
    changes.folders = folders;
    changes.threads.push((thread_id, thread_folders));
    Ok(changes)
}
