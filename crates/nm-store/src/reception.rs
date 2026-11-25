//! What happens when a mail arrives (`plan-imap.md`, phase 6): its
//! authentication verdict, the screening of the Inbox, the return of a
//! conversation from Todo or Waiting, and the effects of a mail found in
//! Sent. Called by [`crate::Store::insert_arrivals`], in the transaction
//! that inserts the mail: the mail never shows in the Inbox before it is
//! screened, and a crash leaves either both or neither.
//!
//! # Screening (`authentification-expediteur.md`, section 4)
//!
//! For a mail arriving in the Inbox, from address `A` of domain `D`:
//!
//! 1. `A` banned, or `A` without rule and `D` banned: **Banned**;
//! 2. `A` has a favorable rule (Inbox, Newsletter, Paperwork): its target
//!    if `A.auth_policy` accepts the verdict, else **Quarantine**, suspect;
//! 3. else `D` has a favorable rule: its target if the verdict is `pass`,
//!    else **Quarantine**, suspect;
//! 4. else, a reply in a known conversation: **Inbox** (no decision on
//!    `A`) unless the verdict is `fail` (**Quarantine**, suspect);
//! 5. else **Quarantine**.
//!
//! Then `A.auth_seen` becomes 1 if the verdict is `pass`. The mails of the
//! user's own addresses stay in the Inbox, whatever their rule: the
//! addresses of every account, IMAP (with its identities) or archive, not
//! only of the account that receives ([`is_own`]). Only the mails at or above the
//! Inbox's `uid_screening_start` are screened: those below were there when
//! the account was created (`import-initial.md`, section 5).
//!
//! A mail moved by the screening goes through the spool, like a command
//! (source `core`), and gets a `screen` row in `audit_log`.

use std::collections::{BTreeSet, HashMap};

use nm_core::{AuthPolicy, AuthResult, FolderRole, ScreeningTarget, Source};
use rusqlite::{OptionalExtension, Transaction, params};
use serde_json::json;

use crate::screening::{domain_target, file_address, move_message};
use crate::sync::Arrival;
use crate::{ChangeSet, Result};

/// The return marks (`dossiers.md`, "Marque de retour").
use nm_core::{KW_WAS_TODO as WAS_TODO, KW_WAS_WAITING as WAS_WAITING};

/// What the arrivals of one folder need to know.
#[derive(Debug, Clone, Copy)]
pub struct Reception<'a> {
    /// Role of the folder (`inbox`, `sent`…).
    pub role: &'a str,
    /// Whether new rows are new mail for the user (`MessageArrived`), and
    /// a mail found in Sent was sent since the last pass: not during the
    /// first synchronization of the folder.
    pub announce: bool,
    /// The folder keeps no keyword (`PERMANENTFLAGS` without `\*`).
    pub keywords_local: bool,
    /// The account's `authserv-id`: the file's, else the detected one.
    pub authserv_id: Option<&'a str>,
    /// The user's own addresses, lowercase: those of every IMAP account
    /// and its identities (`accounts.toml`, which the database does not
    /// hold). The addresses of the archive accounts, which it does, are
    /// added by [`is_own`].
    pub own: &'a [String],
    /// Inbox only: mails at or above this UID are screened. `None`: the
    /// screening of the account is suspended (account being created).
    pub screen_from_uid: Option<u32>,
    /// A read-only account: the screening says where each mail would go
    /// (`screen` row with `dry_run`), and moves nothing; a mail found in
    /// Sent has no effect.
    pub dry_run: bool,
}

impl Reception<'_> {
    /// A folder with no reception rule: no screening, no verdict source.
    pub fn plain(announce: bool) -> Reception<'static> {
        Reception { role: "", announce, keywords_local: false, authserv_id: None, own: &[], screen_from_uid: None, dry_run: false }
    }
}

/// The verdict of an arriving mail.
pub fn verdict_of(a: &Arrival, rc: &Reception) -> (AuthResult, Option<&'static str>) {
    let from = a.headers.from.as_ref().map(|f| f.addr.as_str());
    crate::auth::verdict(&a.headers.auth_results, rc.authserv_id, from, a.headers.from_count)
}

/// After the row of `message_id` was inserted in `folder_id`: screening,
/// return of a conversation, effects of Sent. Returns the folder the mail
/// ends in.
#[allow(clippy::too_many_arguments)]
pub(crate) fn after_insert(
    tx: &Transaction,
    rc: &Reception,
    account_id: i64,
    folder_id: i64,
    message_id: i64,
    a: &Arrival,
    verdict: AuthResult,
    changes: &mut ChangeSet,
    threads: &mut BTreeSet<i64>,
) -> Result<i64> {
    let mut folder = folder_id;
    if rc.role == "inbox" && rc.screen_from_uid.is_some_and(|start| a.uid >= start) {
        folder = screen(tx, rc, account_id, folder_id, message_id, a, verdict, changes, threads)?;
    }
    if rc.role == "sent" && rc.announce && !rc.dry_run {
        sent_effects(tx, rc, account_id, message_id, changes, threads)?;
    }
    // Trust on first use: a sender seen authenticated must stay so.
    if verdict == AuthResult::Pass
        && let Some(f) = &a.headers.from
    {
        tx.prepare_cached("UPDATE address SET auth_seen = 1 WHERE addr = ?1 AND auth_seen = 0")?.execute([&f.addr])?;
    }
    Ok(folder)
}

/// Where the screening sends a mail, why, and whether it is suspect.
struct Route {
    target: FolderRole,
    rule: &'static str,
    suspect: bool,
}

fn route(tx: &Transaction, account_id: i64, message_id: i64, a: &Arrival, verdict: AuthResult) -> Result<Route> {
    let quarantine = |rule, suspect| Route { target: FolderRole::Quarantine, rule, suspect };
    let Some(from) = a.headers.from.as_ref().filter(|_| a.headers.from_count == 1) else {
        // No sender, or several: no rule can apply.
        return Ok(quarantine("no_sender", false));
    };
    let row: Option<(Option<String>, String, bool)> = tx
        .prepare_cached("SELECT screening_target, auth_policy, auth_seen FROM address WHERE addr = ?1")?
        .query_row([&from.addr], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .optional()?;
    let (own_rule, policy, seen) = match row {
        Some((t, p, s)) => (t.as_deref().and_then(ScreeningTarget::parse), AuthPolicy::parse(&p).unwrap_or_default(), s),
        None => (None, AuthPolicy::default(), false),
    };
    let domain_rule = domain_target(tx, &from.addr)?;
    let folder_of = |t: ScreeningTarget| match t {
        ScreeningTarget::Inbox => FolderRole::Inbox,
        ScreeningTarget::Newsletter => FolderRole::Newsletter,
        ScreeningTarget::Paperwork => FolderRole::Paperwork,
        ScreeningTarget::Banned => FolderRole::Banned,
    };
    // 1. A restrictive rule always applies.
    if own_rule == Some(ScreeningTarget::Banned) {
        return Ok(Route { target: FolderRole::Banned, rule: "address", suspect: false });
    }
    if own_rule.is_none() && domain_rule == Some(ScreeningTarget::Banned) {
        return Ok(Route { target: FolderRole::Banned, rule: "domain", suspect: false });
    }
    // 2. A favorable rule of the address, if the verdict is enough.
    if let Some(t) = own_rule {
        return Ok(if policy.accepts(verdict, seen) {
            Route { target: folder_of(t), rule: "address", suspect: false }
        } else {
            quarantine("address", true)
        });
    }
    // 3. A favorable rule of the domain: authenticated mail only.
    if let Some(t) = domain_rule {
        return Ok(if nm_core::domain_rule_accepts(verdict) {
            Route { target: folder_of(t), rule: "domain", suspect: false }
        } else {
            quarantine("domain", true)
        });
    }
    // 4. A reply in a known conversation (`dossiers.md`): one of the ids it
    // cites is a mail of the account kept in a folder where the user deals
    // with mail. A "reply" to a newsletter, or to the Quarantine, is not.
    let known: bool = tx
        .prepare_cached(
            "SELECT EXISTS (SELECT 1 FROM message_reference r
                              JOIN message m ON m.message_id_hdr = r.ref_msgid AND m.account_id = ?2 AND m.id <> ?1
                              JOIN folder f ON f.id = m.folder_id
                             WHERE r.message_id = ?1 AND f.role IN ('inbox', 'todo', 'waiting', 'archive', 'sent'))",
        )?
        .query_row(params![message_id, account_id], |r| r.get(0))?;
    if known {
        return Ok(if verdict == AuthResult::Fail {
            quarantine("conversation", true)
        } else {
            Route { target: FolderRole::Inbox, rule: "conversation", suspect: false }
        });
    }
    // 5. An unknown sender.
    Ok(quarantine("unknown_sender", false))
}

/// The folder of `role` in the account, if it has one.
fn folder_of_role(tx: &Transaction, account_id: i64, role: FolderRole) -> Result<Option<i64>> {
    Ok(tx
        .prepare_cached("SELECT id FROM folder WHERE account_id = ?1 AND role = ?2")?
        .query_row(params![account_id, role.as_str()], |r| r.get(0))
        .optional()?)
}

fn thread_of(tx: &Transaction, message_id: i64) -> Result<Option<i64>> {
    Ok(tx.prepare_cached("SELECT thread_id FROM message WHERE id = ?1")?.query_row([message_id], |r| r.get(0))?)
}

#[allow(clippy::too_many_arguments)]
fn screen(
    tx: &Transaction,
    rc: &Reception,
    account_id: i64,
    inbox: i64,
    message_id: i64,
    a: &Arrival,
    verdict: AuthResult,
    changes: &mut ChangeSet,
    threads: &mut BTreeSet<i64>,
) -> Result<i64> {
    let audit = |tx: &Transaction, detail: serde_json::Value| -> Result<()> {
        tx.prepare_cached(
            "INSERT INTO audit_log (created_by, entity, entity_id, action, detail) VALUES ('core', 'message', ?1, 'screen', ?2)",
        )?
        .execute(params![message_id, detail.to_string()])?;
        Ok(())
    };
    // The user's own mail (a copy to oneself): it stays, and brings no
    // conversation back from Todo or Waiting (`dossiers.md`).
    let from_own = match &a.headers.from {
        Some(f) => is_own(tx, rc.own, &f.addr)?,
        None => false,
    };
    if from_own {
        audit(tx, json!({"folder": "inbox", "rule": "self", "verdict": verdict.as_str(), "dry_run": rc.dry_run}))?;
        return Ok(inbox);
    }
    let r = route(tx, account_id, message_id, a, verdict)?;
    if r.suspect && !rc.dry_run {
        tx.prepare_cached("UPDATE message SET auth_suspect = 1 WHERE id = ?1")?.execute([message_id])?;
    }
    // A target folder the account does not have (phase 7 creates the
    // missing ones): the mail stays in the Inbox, and the audit says so.
    let to = if r.target == FolderRole::Inbox { Some(inbox) } else { folder_of_role(tx, account_id, r.target)? };
    audit(
        tx,
        json!({"folder": r.target.as_str(), "rule": r.rule, "verdict": verdict.as_str(), "suspect": r.suspect,
               "folder_missing": to.is_none(), "dry_run": rc.dry_run}),
    )?;
    if rc.dry_run {
        // Said, not done.
        return Ok(inbox);
    }
    let thread = thread_of(tx, message_id)?;
    match to {
        Some(to) if to != inbox => {
            move_message(tx, message_id, thread, inbox, to, Source::Core, &json!({"reason": "screen"}), changes, threads)?;
            Ok(to)
        }
        _ => {
            if let Some(t) = thread {
                return_conversation(tx, account_id, t, message_id, inbox, changes, threads)?;
            }
            Ok(inbox)
        }
    }
}

/// A mail delivered to the Inbox, in a conversation with mails in Todo or
/// Waiting: those come back to the Inbox, marked with where they came
/// from (`dossiers.md`, "Nouveau mail dans une conversation en Todo ou en
/// Waiting"). The new mail gets no mark.
fn return_conversation(
    tx: &Transaction,
    account_id: i64,
    thread: i64,
    received: i64,
    inbox: i64,
    changes: &mut ChangeSet,
    threads: &mut BTreeSet<i64>,
) -> Result<()> {
    let waiting: Vec<(i64, i64, String)> = tx
        .prepare_cached(
            "SELECT m.id, m.folder_id, f.role FROM message m JOIN folder f ON f.id = m.folder_id
              WHERE m.thread_id = ?1 AND m.account_id = ?2 AND f.role IN ('todo', 'waiting') ORDER BY m.id",
        )?
        .query_map(params![thread, account_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<std::result::Result<_, _>>()?;
    for (id, folder, role) in waiting {
        move_message(tx, id, Some(thread), folder, inbox, Source::Core, &json!({"reason": "returned", "message_id": received}), changes, threads)?;
        let mark = if role == "todo" { WAS_TODO } else { WAS_WAITING };
        tx.prepare_cached("INSERT OR IGNORE INTO message_keyword (message_id, keyword) VALUES (?1, ?2)")?.execute(params![id, mark])?;
        crate::spool::enqueue_flag(tx, id, mark, true, Source::Core.as_str(), changes)?;
    }
    Ok(())
}

/// Removes the return marks of these messages, locally and on the server
/// (`dossiers.md`, "Retrait"). Returns how many lost one.
pub(crate) fn clear_return_marks(tx: &Transaction, message_ids: &[i64], source: Source, changes: &mut ChangeSet) -> Result<usize> {
    let mut n = 0;
    for id in message_ids {
        let marks: Vec<String> = tx
            .prepare_cached(
                "DELETE FROM message_keyword WHERE message_id = ?1 AND keyword COLLATE NOCASE IN ('$NmWasTodo', '$NmWasWaiting')
                 RETURNING keyword",
            )?
            .query_map([id], |r| r.get(0))?
            .collect::<std::result::Result<_, _>>()?;
        for m in &marks {
            crate::spool::enqueue_flag(tx, *id, m, false, source.as_str(), changes)?;
        }
        n += marks.len();
    }
    Ok(n)
}

/// A mail found in Sent, sent since the last pass (from the phone, the
/// webmail): what sending from NiceMailer does too (`nm-store`,
/// `drafts.rs`).
/// - Its recipients without decision get the decision `inbox` ("Écrire à
///   quelqu'un vaut décision Inbox"): their mails still in Quarantine go
///   to the Inbox.
/// - A reply files its conversation as the reply screen would have
///   preselected ("Répondre à une conversation", "Réponse envoyée
///   ailleurs").
fn sent_effects(
    tx: &Transaction,
    rc: &Reception,
    account_id: i64,
    message_id: i64,
    changes: &mut ChangeSet,
    threads: &mut BTreeSet<i64>,
) -> Result<()> {
    accept_recipients(tx, rc.own, message_id, Source::Imap, changes, threads)?;
    file_reply(tx, account_id, message_id, changes, threads)
}

/// "Écrire à quelqu'un vaut décision Inbox" (`dossiers.md`): the
/// recipients (To, Cc, Bcc) of the sent mail `message_id` that have no
/// decision get `inbox`, and their mails still in Quarantine go to the
/// Inbox. An existing decision is kept, a decision removed too (its date
/// stays: the user chose to have none). `own`: the account's addresses,
/// lowercase, never decided on. Returns how many addresses were decided.
pub(crate) fn accept_recipients(
    tx: &Transaction,
    own: &[String],
    message_id: i64,
    source: Source,
    changes: &mut ChangeSet,
    threads: &mut BTreeSet<i64>,
) -> Result<usize> {
    let recipients: Vec<(i64, String)> = tx
        .prepare_cached(
            "SELECT DISTINCT a.id, a.addr FROM message_address ma JOIN address a ON a.id = ma.address_id
              WHERE ma.message_id = ?1 AND ma.role IN ('to', 'cc', 'bcc')
                AND a.screening_target IS NULL AND a.date_screening_modified IS NULL",
        )?
        .query_map([message_id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?;
    let mut folders = HashMap::new();
    let mut n = 0;
    for (id, addr) in recipients {
        if is_own(tx, own, &addr)? {
            continue;
        }
        tx.prepare_cached(
            "UPDATE address SET screening_target = 'inbox', date_screening_modified = unixepoch(),
                    date_modified = unixepoch(), modified_by = ?2 WHERE id = ?1",
        )?
        .execute(params![id, source.as_str()])?;
        tx.prepare_cached(
            "INSERT INTO audit_log (created_by, entity, entity_id, action, detail)
             VALUES (?3, 'address', ?1, 'screening',
                     json_object('before', NULL, 'after', 'inbox', 'reason', 'sent_to', 'message_id', ?2))",
        )?
        .execute(params![id, message_id, source.as_str()])?;
        file_address(tx, id, ScreeningTarget::Inbox, false, source, &mut folders, changes, threads)?;
        n += 1;
    }
    Ok(n)
}

/// A reply sent elsewhere files its conversation as preselected: the
/// conversation of the mail it answers (`In-Reply-To`), its mails in the
/// Inbox, Todo and Waiting.
fn file_reply(tx: &Transaction, account_id: i64, sent: i64, changes: &mut ChangeSet, threads: &mut BTreeSet<i64>) -> Result<()> {
    let thread: Option<i64> = tx
        .prepare_cached(
            "SELECT o.thread_id FROM message s
               JOIN message o ON o.message_id_hdr = s.in_reply_to AND o.account_id = s.account_id AND o.id <> s.id
               JOIN folder f ON f.id = o.folder_id AND coalesce(f.role, '') <> 'sent'
              WHERE s.id = ?1 LIMIT 1",
        )?
        .query_row([sent], |r| r.get(0))
        .optional()?
        .flatten();
    let Some(thread) = thread else { return Ok(()) };
    file_reply_thread(tx, account_id, thread, sent, None, Source::Core, "reply_elsewhere", changes, threads)?;
    Ok(())
}

type OpenMails = Vec<(i64, i64, String, bool, bool)>;

/// Where a conversation of `account_id` is, for the filing of a reply:
/// its mails in the Inbox, Todo and Waiting, with their return marks, and
/// the folder whose rule applies (`dossiers.md`, "Répondre à une
/// conversation"): the Inbox if any mail is there (with where it came back
/// from), else Todo, else Waiting. `None`: the conversation is in none of
/// them, and stays where it is.
fn reply_position(conn: &rusqlite::Connection, account_id: i64, thread: i64) -> Result<Option<(OpenMails, FolderRole, Option<FolderRole>)>> {
    let open: OpenMails = conn
        .prepare_cached(
            "SELECT m.id, m.folder_id, f.role,
                    EXISTS (SELECT 1 FROM message_keyword k WHERE k.message_id = m.id AND k.keyword = '$NmWasTodo' COLLATE NOCASE),
                    EXISTS (SELECT 1 FROM message_keyword k WHERE k.message_id = m.id AND k.keyword = '$NmWasWaiting' COLLATE NOCASE)
               FROM message m JOIN folder f ON f.id = m.folder_id
              WHERE m.thread_id = ?1 AND m.account_id = ?2 AND f.role IN ('inbox', 'todo', 'waiting')",
        )?
        .query_map(params![thread, account_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?
        .collect::<std::result::Result<_, _>>()?;
    let in_inbox: Vec<&(i64, i64, String, bool, bool)> = open.iter().filter(|m| m.2 == "inbox").collect();
    let (here, returned_from) = if !in_inbox.is_empty() {
        // Todo wins when both marks are there: something is left to do.
        let back = if in_inbox.iter().any(|m| m.3) {
            Some(FolderRole::Todo)
        } else if in_inbox.iter().any(|m| m.4) {
            Some(FolderRole::Waiting)
        } else {
            None
        };
        (FolderRole::Inbox, back)
    } else if open.iter().any(|m| m.2 == "todo") {
        (FolderRole::Todo, None)
    } else if open.iter().any(|m| m.2 == "waiting") {
        (FolderRole::Waiting, None)
    } else {
        return Ok(None);
    };
    Ok(Some((open, here, returned_from)))
}

/// The folder a reply would file the conversation in, as preselected now
/// (`ThreadSummary.reply_target`, the "after sending" line of the reply
/// screen); `None` when it stays where it is.
pub(crate) fn reply_preselection(conn: &rusqlite::Connection, account_id: i64, thread: i64) -> Result<Option<FolderRole>> {
    Ok(reply_position(conn, account_id, thread)?.and_then(|(_, here, back)| here.reply_target(back)))
}

/// Files the conversation `thread` after the reply `sent` (`dossiers.md`,
/// "Répondre à une conversation"): to `target`, or to the preselection when
/// `None`; its mails in the Inbox, Todo and Waiting follow. Filing is a
/// command: it removes the return marks, even when nothing moved (a
/// reminder from Waiting). Returns the folder the conversation went to,
/// `None` when nothing moved.
#[allow(clippy::too_many_arguments)]
pub(crate) fn file_reply_thread(
    tx: &Transaction,
    account_id: i64,
    thread: i64,
    sent: i64,
    target: Option<FolderRole>,
    source: Source,
    reason: &str,
    changes: &mut ChangeSet,
    threads: &mut BTreeSet<i64>,
) -> Result<Option<FolderRole>> {
    let Some((open, here, returned_from)) = reply_position(tx, account_id, thread)? else { return Ok(None) };
    let Some(target) = target.or_else(|| here.reply_target(returned_from)) else { return Ok(None) };
    let Some(to) = folder_of_role(tx, account_id, target)? else { return Ok(None) };
    let mut moved = false;
    for (id, folder, ..) in &open {
        if *folder != to {
            move_message(tx, *id, Some(thread), *folder, to, source, &json!({"reason": reason, "message_id": sent}), changes, threads)?;
            moved = true;
        }
    }
    let ids: Vec<i64> = open.iter().map(|m| m.0).collect();
    if clear_return_marks(tx, &ids, source, changes)? > 0 {
        threads.insert(thread);
    }
    Ok(moved.then_some(target))
}

/// Files a mail the user wrote, once its copy in Sent is made
/// (`reponse.md`, section 8, "Nouveau mail"): the copy leaves Sent for
/// `target`, here and on the server. A new mail answers nothing, so its
/// conversation is made of that copy alone; moving it is what a move
/// given from Sent does for such a conversation (`commands::apply_move`,
/// `dossiers.md`, "Cibles proposées").
///
/// Only the mail sent moves, never another copy its conversation may
/// hold: the user filed *this* mail. Returns the folder it went to;
/// `None` when the account has no such folder, or the mail already left
/// Sent (moved by hand meanwhile, or this call replayed after a crash).
pub(crate) fn file_sent_mail(
    tx: &Transaction,
    account_id: i64,
    sent: i64,
    target: FolderRole,
    source: Source,
    changes: &mut ChangeSet,
    threads: &mut BTreeSet<i64>,
) -> Result<Option<FolderRole>> {
    let Some(to) = folder_of_role(tx, account_id, target)? else { return Ok(None) };
    // Where the mail is now, and whether that folder is still Sent.
    let row: Option<(i64, Option<i64>, Option<String>)> = tx
        .prepare_cached(
            "SELECT m.folder_id, m.thread_id, f.role FROM message m JOIN folder f ON f.id = m.folder_id WHERE m.id = ?1",
        )?
        .query_row([sent], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .optional()?;
    let Some((from, thread, role)) = row else { return Ok(None) };
    if role.as_deref() != Some(FolderRole::Sent.as_str()) || from == to {
        return Ok(None);
    }
    move_message(tx, sent, thread, from, to, source, &json!({"reason": "sent_new"}), changes, threads)?;
    Ok(Some(target))
}

/// Whether `addr` (lowercase) is one of the user's: in `own` (the IMAP
/// accounts and their identities), or the address of an archive account
/// (`account.address`). The user's own mail is never screened, even when
/// the address got a rule: an address of one account banned by mistake
/// would otherwise send to Banned the mail the user writes to another of
/// their accounts. A handful of rows, read per mail that arrives.
fn is_own(tx: &Transaction, own: &[String], addr: &str) -> Result<bool> {
    if own.iter().any(|o| o == addr) {
        return Ok(true);
    }
    Ok(tx
        .prepare_cached("SELECT 1 FROM account WHERE address = ?1 COLLATE NOCASE LIMIT 1")?
        .query_row([addr], |_| Ok(()))
        .optional()?
        .is_some())
}
