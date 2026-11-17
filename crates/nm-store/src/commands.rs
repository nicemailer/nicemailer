//! Write commands on a thread (`dossiers.md`, "Déplacer un mail").
//!
//! v1 limits: commands are only applied to the local database. There is no
//! spool of actions to replay on the IMAP server (`pending_action`) since
//! there is no server yet. Decisions on senders and the filing they trigger
//! live in `screening.rs`.
//!
//! Every command follows the same pattern:
//! 1. validate the parameters,
//! 2. open an IMMEDIATE transaction (it takes the write lock right away,
//!    so two writers cannot both read and then conflict),
//! 3. modify the messages and write one `audit_log` row per change,
//! 4. recompute the denormalized aggregates,
//! 5. commit and return a `ChangeSet` describing what changed.

use std::collections::{BTreeSet, HashMap};

use nm_core::{FolderRole, ScreeningTarget, Source};
// `OptionalExtension` adds `.optional()` to query results: it turns the
// "no row" error into `Ok(None)`. `params!` builds a list of SQL parameters
// of different types.
use rusqlite::{OptionalExtension, TransactionBehavior, params};

// `crate::` refers to the root of this crate, i.e. `lib.rs`.
use crate::screening::{
    DecisionChange, MoveOptions, Outcome, RuleKind, audit_mark, file_address, move_message, record_auth_policy,
    record_decision, refresh_all, suspect_verdict, thread_sender, undo_token,
};
use crate::{ChangeSet, Result, Store, StoreError, refresh_folder_counts, refresh_thread};

impl Store {
    /// Returns the account of a thread, or `NotFound` if it does not exist.
    /// Also serves as an existence check at the start of each command.
    ///
    /// Rust note: `ok_or_else` turns an `Option` into a `Result`: `Some(x)`
    /// becomes `Ok(x)` and `None` becomes the error built by the closure.
    fn thread_account(&self, thread_id: i64) -> Result<i64> {
        let id = self
            .conn
            .query_row("SELECT account_id FROM thread WHERE id = ?1", [thread_id], |r| r.get(0))
            .optional()?
            .ok_or_else(|| StoreError::NotFound(format!("conversation {thread_id}")))?;
        crate::spool::refuse_read_only(&self.conn, thread_id)?;
        Ok(id)
    }

    /// Moves a thread to the folder with role `target`, and records the
    /// decision on its sender that the move implies (`dossiers.md`,
    /// "Déplacement et décision sur l'expéditeur").
    ///
    /// Only the thread's messages filed in Inbox, Todo, Waiting, Archive,
    /// Newsletter and Paperwork follow (`CONVERSATION_SCOPE`), plus those in
    /// the folder where the command is given (`from_folder_id`). So the
    /// copies in Sent never move, and a message the user deleted stays in
    /// Deleted unless the user is restoring it from there.
    ///
    /// From Drafts (only to Deleted), only the drafts move: deleting an old
    /// draft must not put the conversation it answers in the bin. A draft
    /// of NiceMailer loses its row of `draft` on the way
    /// (`draft_copies::release_for_deletion`): its row of Drafts, in
    /// Deleted, keeps its text.
    ///
    /// When the move decides (`ScreeningTarget::for_move`), the decision is
    /// recorded on the thread's sender (`screening::thread_sender`) and the
    /// sender's other mails are filed, in the same transaction. A sender
    /// whose mail is suspected of spoofing:
    /// - is not banned: Banned then concerns this mail only, so a forger
    ///   cannot get the real address banned;
    /// - gets a favorable decision only with `opts.confirm_suspect`, else
    ///   the command fails with `ConfirmationRequired` and changes nothing.
    ///
    /// Rust note: `&mut self` means the method needs exclusive access to the
    /// store, because opening a transaction requires a mutable connection.
    pub fn move_thread(
        &mut self,
        thread_id: i64,
        target: FolderRole,
        from_folder_id: Option<i64>,
        opts: MoveOptions,
        source: Source,
    ) -> Result<Outcome> {
        // The checks first, before any write: a refusal changes nothing.
        let prepared = self.prepare_move(thread_id, target, from_folder_id, opts)?;
        // From here on every statement goes through `tx`. If the function
        // returns early with an error (`?`), `tx` is dropped without commit
        // and SQLite rolls everything back automatically.
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mark = audit_mark(&tx)?;
        let mut out = Outcome::default();
        let mut threads = BTreeSet::new();
        let (decision, filed) = apply_move(&tx, &prepared, opts, source, &mut out, &mut threads)?;
        out.decision = decision;
        out.filed = filed;
        // Recompute aggregates only if something actually changed.
        refresh_all(&tx, &threads, &mut out.changes)?;
        out.undo = undo_token(&tx, mark)?;
        tx.commit()?;
        Ok(out)
    }

    /// Moves several threads to the folder with role `target`, each from
    /// the folder it is shown in, in **one** transaction (`ui.md`, "Actions
    /// groupées"): one request, one commit, one batch of events and one
    /// undo token for all, where one request per thread made the list
    /// change row by row and `z` undo them one at a time.
    ///
    /// Each thread follows the rules of `move_thread`. What would refuse
    /// the whole batch in a loop of single moves is reported instead:
    /// - a thread gone meanwhile is skipped;
    /// - a target not allowed from a thread's folder: the thread is left,
    ///   and listed in `refused`;
    /// - a sender suspected of spoofing that would get a favorable decision
    ///   without `opts.confirm_suspect`: the thread is left, and listed in
    ///   `suspects` for the client to ask about.
    pub fn move_threads(
        &mut self,
        threads: &[(i64, Option<i64>)],
        target: FolderRole,
        opts: MoveOptions,
        source: Source,
    ) -> Result<MoveBatch> {
        let mut batch = MoveBatch::default();
        let mut prepared = Vec::with_capacity(threads.len());
        for &(thread_id, from_folder_id) in threads {
            if prepared.iter().any(|p: &PreparedMove| p.thread_id == thread_id) {
                continue;
            }
            match self.prepare_move(thread_id, target, from_folder_id, opts) {
                Ok(p) => prepared.push(p),
                Err(StoreError::NotFound(_)) => {}
                Err(StoreError::InvalidMove(_)) => batch.refused.push(thread_id),
                Err(StoreError::ConfirmationRequired { addr, verdict }) => {
                    batch.suspects.push(SuspectThread { thread_id, addr, verdict })
                }
                Err(e) => return Err(e),
            }
        }
        if prepared.is_empty() {
            return Ok(batch);
        }
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mark = audit_mark(&tx)?;
        let mut touched = BTreeSet::new();
        for p in &prepared {
            let (decision, filed) = apply_move(&tx, p, opts, source, &mut batch.outcome, &mut touched)?;
            batch.outcome.filed += filed;
            if let Some(d) = decision {
                batch.decisions.push(d);
            }
        }
        batch.outcome.decision = batch.decisions.last().cloned();
        refresh_all(&tx, &touched, &mut batch.outcome.changes)?;
        batch.outcome.undo = undo_token(&tx, mark)?;
        tx.commit()?;
        batch.moved_threads = prepared.iter().map(|p| p.thread_id).collect();
        Ok(batch)
    }

    /// The checks of a move, before any write: the thread exists and can be
    /// written, the target is allowed from the folder it is shown in, and
    /// the decision the move implies (`dossiers.md`, "Déplacement et
    /// décision sur l'expéditeur") is allowed for its sender.
    fn prepare_move(
        &self,
        thread_id: i64,
        target: FolderRole,
        from_folder_id: Option<i64>,
        opts: MoveOptions,
    ) -> Result<PreparedMove> {
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
                    .ok_or_else(|| StoreError::NotFound(format!("folder {fid}")))?;
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
            return Err(StoreError::InvalidMove(
                nm_api::reason::MOVE_REFUSED.with("target", target.as_str()).with("from", from_role.map(|r| r.as_str()).unwrap_or("this folder")),
            ));
        }

        // The decision. A sender whose mail is suspected of spoofing:
        // - is not banned: Banned then concerns this mail only, so a forger
        //   cannot get the real address banned;
        // - gets a favorable decision only with `opts.confirm_suspect`, else
        //   `ConfirmationRequired`.
        let decision = match ScreeningTarget::for_move(from_role, target) {
            Some(d) => match thread_sender(&self.conn, thread_id)? {
                Some((_, _, _, true)) if d == ScreeningTarget::Banned => None,
                Some((id, addr, _, true)) if !opts.confirm_suspect => {
                    let verdict = suspect_verdict(&self.conn, thread_id, id)?;
                    return Err(StoreError::ConfirmationRequired { addr, verdict });
                }
                Some((id, addr, trigger, _)) => Some((d, id, addr, trigger)),
                None => None,
            },
            None => None,
        };
        Ok(PreparedMove { thread_id, account_id, target, from_folder_id, from_role, decision })
    }
}

/// A move checked by `prepare_move`, to apply in a transaction.
struct PreparedMove {
    thread_id: i64,
    account_id: i64,
    target: FolderRole,
    from_folder_id: Option<i64>,
    from_role: Option<FolderRole>,
    /// The decision on the sender: (target, address id, address, the mail
    /// that triggers it).
    decision: Option<(ScreeningTarget, i64, String, i64)>,
}

/// What `move_threads` did.
#[derive(Debug, Default)]
pub struct MoveBatch {
    /// Changes, mails moved and filed, the undo token of the whole batch,
    /// and the last decision taken (`decisions` lists them all).
    pub outcome: Outcome,
    pub decisions: Vec<DecisionChange>,
    /// The threads moved.
    pub moved_threads: Vec<i64>,
    /// Threads left where they are: the target is not allowed from their
    /// folder.
    pub refused: Vec<i64>,
    /// Threads left where they are: their sender is suspected of spoofing
    /// and the move would accept it; the client asks, then moves them with
    /// `confirm_suspect`.
    pub suspects: Vec<SuspectThread>,
}

impl MoveBatch {
    /// The contract's view of the batch (`move_threads`).
    pub fn view(self) -> nm_api::MoveThreadsResult {
        let one = self.outcome.view();
        nm_api::MoveThreadsResult {
            changed: one.changed,
            moved: one.moved,
            filed: one.filed,
            decisions: self.decisions.iter().map(DecisionChange::view).collect(),
            undo: one.undo,
            moved_threads: self.moved_threads,
            refused: self.refused,
            suspects: self
                .suspects
                .into_iter()
                .map(|s| nm_api::SuspectView { thread_id: s.thread_id, addr: s.addr, verdict: s.verdict })
                .collect(),
        }
    }
}

/// A thread a batch move left for a confirmation (`MoveBatch::suspects`).
#[derive(Debug, Clone)]
pub struct SuspectThread {
    pub thread_id: i64,
    pub addr: String,
    pub verdict: String,
}

/// The writes of a move checked by `prepare_move` (`dossiers.md`,
/// "Déplacer un mail"), in the caller's transaction. Adds to `out` its
/// changes and the mails moved, and to `threads` the threads to refresh;
/// returns the decision taken and the number of other mails filed.
///
/// Only the thread's messages filed in Inbox, Todo, Waiting, Archive,
/// Newsletter and Paperwork follow (`CONVERSATION_SCOPE`), plus those in
/// the folder where the command is given (`from_folder_id`). So a
/// message the user deleted stays in Deleted unless the user is
/// restoring it from there.
///
/// From Sent, the copies in Sent move only when the conversation has no
/// other mail to move (a new mail with no answer yet): otherwise they
/// stay in Sent, where the phone finds them.
///
/// From Drafts (only to Deleted), only the drafts move: deleting an old
/// draft must not put the conversation it answers in the bin. A draft
/// of NiceMailer loses its row of `draft` on the way
/// (`draft_copies::release_for_deletion`): its row of Drafts, in
/// Deleted, keeps its text.
fn apply_move(
    tx: &rusqlite::Transaction,
    p: &PreparedMove,
    opts: MoveOptions,
    source: Source,
    out: &mut Outcome,
    threads: &mut BTreeSet<i64>,
) -> Result<(Option<DecisionChange>, usize)> {
    let thread_id = p.thread_id;
    let target_id: i64 = tx
        .query_row(
            "SELECT id FROM folder WHERE account_id = ?1 AND role = ?2",
            params![p.account_id, p.target.as_str()],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(|| StoreError::NotFound(format!("folder {}", p.target.as_str())))?;

    // Messages to move, with their current folder (needed for the audit
    // log and to refresh the old folder's counters). The six roles of the
    // scope are passed as separate parameters (?3 to ?8); `-1` stands
    // for "no from_folder" since no folder has that id.
    let scope: Vec<&str> = FolderRole::CONVERSATION_SCOPE.iter().map(|r| r.as_str()).collect();
    // From Drafts, the folder of the command only (see above): ?10 off.
    let whole_conversation = p.from_role != Some(FolderRole::Drafts);
    // Rust note: a closure (`|from| ...`) is an anonymous function that
    // can read the variables around it (`tx`, `scope`...). It is called
    // twice below, with a different folder of the command.
    let select = |from_folder: i64| -> Result<Vec<(i64, i64)>> {
        let mut stmt = tx.prepare_cached(
            "SELECT m.id, m.folder_id FROM message m JOIN folder f ON f.id = m.folder_id
              WHERE m.thread_id = ?1 AND m.folder_id <> ?2
                AND ((?10 AND f.role IN (?3, ?4, ?5, ?6, ?7, ?8)) OR m.folder_id = ?9)",
        )?;
        let rows = stmt
            .query_map(
                params![
                    thread_id, target_id, scope[0], scope[1], scope[2], scope[3], scope[4], scope[5], from_folder,
                    whole_conversation
                ],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?
            // Each row is a `Result`; collecting into `Result<Vec<_>, _>`
            // stops at the first error, which the final `?` propagates.
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    };
    // From Sent, the copies in Sent stay where the phone finds them: only
    // the rest of the conversation moves. A conversation with nothing
    // else (a new mail, no answer yet) would not move at all, so its mails
    // in Sent leave instead, here and on the server: the user put a mail
    // they wrote in Waiting or Todo (`dossiers.md`, "Cibles proposées").
    let moved: Vec<(i64, i64)> = if p.from_role == Some(FolderRole::Sent) {
        let rest = select(-1)?;
        if rest.is_empty() { select(p.from_folder_id.unwrap_or(-1))? } else { rest }
    } else {
        select(p.from_folder_id.unwrap_or(-1))?
    };

    if !whole_conversation {
        for (message_id, _) in &moved {
            crate::draft_copies::release_for_deletion(tx, *message_id)?;
        }
    }

    out.moved += moved.len();
    out.changes.folders.insert(target_id);
    let none = serde_json::json!({});
    // `&moved` iterates by reference, so `message_id` and `old_folder` are
    // `&i64`; `*old_folder` reads the value behind the reference.
    for (message_id, old_folder) in &moved {
        move_message(tx, *message_id, Some(thread_id), *old_folder, target_id, source, &none, &mut out.changes, threads)?;
    }

    // Any command clears the "came back from Todo/Waiting" mark, even when
    // nothing moved: by acting on the thread, the user has dealt with it.
    // The marks are IMAP keywords: removed on the server too (spool).
    let ids: Vec<i64> = tx
        .prepare_cached("SELECT id FROM message WHERE thread_id = ?1")?
        .query_map([thread_id], |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    let unmarked = crate::reception::clear_return_marks(tx, &ids, source, &mut out.changes)?;
    if unmarked > 0 {
        threads.insert(thread_id);
    }

    let mut decision = None;
    let mut filed = 0;
    if let Some((d, address_id, addr, trigger)) = &p.decision {
        if d.is_favorable()
            && let Some(policy) = opts.auth_policy
        {
            record_auth_policy(tx, *address_id, policy, None, source)?;
        }
        let before = record_decision(tx, RuleKind::Address, *address_id, Some(*d), Some(*trigger), None, source)?;
        let mut folders = HashMap::new();
        filed = file_address(tx, *address_id, *d, false, source, &mut folders, &mut out.changes, threads)?;
        decision = Some(DecisionChange { kind: RuleKind::Address, name: addr.clone(), before, after: Some(*d) });
    }
    Ok((decision, filed))
}

impl Store {
    /// Marks every message of the thread as read or unread.
    pub fn set_thread_seen(&mut self, thread_id: i64, seen: bool, source: Source) -> Result<ChangeSet> {
        self.thread_account(thread_id)?;
        self.set_threads_seen(&[thread_id], seen, source)
    }

    /// Marks every message of the threads read or unread, in **one**
    /// transaction: `u` and `U` on 50 marked threads are one request, one
    /// commit and one batch of events, where 50 requests made the list
    /// change row by row (`ui.md`, "Actions groupées"). A thread gone
    /// meanwhile (merged, deleted by the synchronization) is skipped; a
    /// thread of a read-only account refuses the whole command, before any
    /// write.
    pub fn set_threads_seen(&mut self, thread_ids: &[i64], seen: bool, source: Source) -> Result<ChangeSet> {
        let ids = self.existing_threads(thread_ids)?;
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut all = ChangeSet::default();
        for thread_id in ids {
            // `RETURNING` (SQLite 3.35+) gives back the rows actually
            // updated, so a single statement both modifies and tells us what
            // changed. The `is_seen <> ?2` condition skips messages already
            // in the wanted state.
            let changed: Vec<(i64, i64)> = {
                let mut stmt = tx.prepare_cached(
                    "UPDATE message SET is_seen = ?2, date_modified = unixepoch(), modified_by = ?3
                      WHERE thread_id = ?1 AND is_seen <> ?2
                      RETURNING id, folder_id",
                )?;
                stmt.query_map(params![thread_id, seen, source.as_str()], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<std::result::Result<_, _>>()?
            };
            all.merge(finish_flag_change(&tx, thread_id, &changed, source, "set_flags", "is_seen", seen)?);
        }
        tx.commit()?;
        Ok(all)
    }

    /// The threads of `ids` that still exist, in their order and each once;
    /// `ReadOnly` if one belongs to a read-only account (nothing written).
    pub(crate) fn existing_threads(&self, ids: &[i64]) -> Result<Vec<i64>> {
        let mut out = Vec::with_capacity(ids.len());
        for &id in ids {
            if out.contains(&id) {
                continue;
            }
            match self.thread_account(id) {
                Ok(_) => out.push(id),
                Err(StoreError::NotFound(_)) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(out)
    }

    /// Flags the last message received in the thread, or unflags all of
    /// its messages.
    ///
    /// Flagging a single message keeps IMAP semantics (a flag belongs to a
    /// message) while the UI shows the thread as flagged when any of its
    /// messages is.
    pub fn set_thread_flagged(&mut self, thread_id: i64, flagged: bool, source: Source) -> Result<ChangeSet> {
        self.thread_account(thread_id)?;
        self.set_threads_flagged(&[thread_id], flagged, source)
    }

    /// `set_thread_flagged` for several threads, in one transaction (see
    /// `set_threads_seen`).
    pub fn set_threads_flagged(&mut self, thread_ids: &[i64], flagged: bool, source: Source) -> Result<ChangeSet> {
        let ids = self.existing_threads(thread_ids)?;
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut all = ChangeSet::default();
        for thread_id in ids {
            // In Rust `if` is an expression: both branches produce the `Vec`.
            let changed: Vec<(i64, i64)> = if flagged {
                // Last received message, excluding Sent and Drafts; if the
                // thread only has those, the most recent message overall.
                // The first ORDER BY term is 0 for received mail and 1 for
                // Sent/Drafts, so received mail sorts first.
                let target: Option<i64> = tx
                    .prepare_cached(
                        "SELECT m.id FROM message m JOIN folder f ON f.id = m.folder_id
                          WHERE m.thread_id = ?1
                          ORDER BY (coalesce(f.role, '') IN ('sent', 'drafts')), m.date_received DESC, m.id DESC
                          LIMIT 1",
                    )?
                    .query_row([thread_id], |r| r.get(0))
                    .optional()?;
                let mut stmt = tx.prepare_cached(
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
                let mut stmt = tx.prepare_cached(
                    "UPDATE message SET is_flagged = 0, date_modified = unixepoch(), modified_by = ?2
                      WHERE thread_id = ?1 AND is_flagged = 1 RETURNING id, folder_id",
                )?;
                stmt.query_map(params![thread_id, source.as_str()], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<std::result::Result<_, _>>()?
            };
            all.merge(finish_flag_change(&tx, thread_id, &changed, source, "set_flags", "is_flagged", flagged)?);
        }
        tx.commit()?;
        Ok(all)
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
    // The IMAP flag of the column, for the spool.
    let imap_flag = if flag == "is_seen" { "\\Seen" } else { "\\Flagged" };
    for (message_id, _) in changed {
        tx.execute(
            "INSERT INTO audit_log (created_by, entity, entity_id, action, detail)
             VALUES (?1, 'message', ?2, ?3,
                     json_object('before', json_object(?4, ?5), 'after', json_object(?4, ?6)))",
            params![source.as_str(), message_id, action, flag, !value, value],
        )?;
        crate::spool::enqueue_flag(tx, *message_id, imap_flag, value, source.as_str(), &mut changes)?;
        changes.messages.insert((*message_id, Some(thread_id)));
    }
    let thread_folders = refresh_thread(tx, thread_id)?;
    for f in &folders {
        refresh_folder_counts(tx, *f)?;
    }
    changes.folders = folders;
    changes.threads.push((thread_id, thread_folders));
    // A flag changes this thread only, and it is listed above: a client
    // may reload it alone (`ChangeSet::announced`). This is what happens
    // when a mail is read under the cursor, a few times a minute while
    // the user goes down a list.
    changes.announce_threads();
    Ok(changes)
}
