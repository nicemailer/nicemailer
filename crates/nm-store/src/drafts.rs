//! Replies being written, and their sending (`specifications/reponse.md`;
//! the life of a draft is `brouillons.md`, section 4).
//!
//! # The draft
//!
//! A reply is a row of `draft`, written by [`Store::create_reply_draft`]:
//! the recipients, the subject and the quotation are prepared here, by the
//! rules of `nm_core::compose`, from what the database knows of the mail
//! answered (its headers, its addresses, its references, its text). The
//! client sends the body at each pause of typing ([`Store::update_draft`]);
//! the revision keeps two windows from writing over each other.
//!
//! In v1 a draft lives here only: no copy goes to the Drafts folder of the
//! server (`reponse.md`, section 5). Its `server_revision` stays NULL, so
//! the version that brings `append_draft` will find which drafts were never
//! sent to the server.
//!
//! # Sending
//!
//! ```text
//! send_draft ──▶ action `send` (phase smtp), due in 5 s ──▶ SMTP accepts ──▶ send_accepted
//!   │                         ▲                                              │ (Sent row, filing,
//!   └─ cancel_send ───────────┘ (back to editing)                            │  decisions, draft gone)
//!                                                                            ▼
//!                                                      phase append ──▶ copy in Sent ──▶ send_copied (done)
//! ```
//!
//! [`Store::send_draft`] freezes the draft (`sending`), builds the MIME
//! message and queues an action `send` in `pending_action`, due after the
//! undo delay. The engine's sender (`nm-engine`, `sync/send.rs`) claims it
//! when it is due, talks to the SMTP server, and reports here:
//!
//! - accepted ([`Store::send_accepted`]): in one transaction, the mail
//!   becomes a row of Sent, the conversation is filed, the recipients get
//!   their decision, the draft row goes; the action goes on in phase
//!   `append`, the copy in the Sent folder of the server;
//! - temporary failure ([`Store::send_retry`]): tried again later, like the
//!   rest of the spool (30 s, doubled, 8 attempts);
//! - definitive failure ([`Store::send_failed`]): the draft is editable
//!   again, and `SendFailed` tells the user.
//!
//! # Payload of a `send` action
//!
//! `{draft_id, phase, from, rcpt, mime, message_id_hdr, after_send,
//! data_sent}`: everything the sender needs, so that it never reads the
//! draft again (the user may not change it while it is `sending`).
//! `data_sent` is set just before the server is told the message is
//! complete: from then on the mail may have left even if no answer came
//! back, and a new attempt first looks for it in Sent ("Double envoi").

use std::collections::BTreeSet;

use nm_api::{AddressView, DraftView};
use nm_core::compose::{self, Mailbox, Original};
use nm_core::{AuthResult, FolderRole, Source};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::{Value, json};

use crate::insert::{Body, Flags, Inserter, NewMessage};
use crate::raw::Written;
use crate::screening::{audit_mark, undo_token};
use crate::{ChangeSet, Result, Store, StoreError};

/// A draft sent, for the event `DraftSent`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentDraft {
    pub draft_id: i64,
    pub message_id: i64,
    pub thread_id: Option<i64>,
    pub moved_to: Option<FolderRole>,
    pub undo: Option<String>,
}

/// What the store needs to know of the account to write a reply: its
/// addresses, main one first, with their names (`accounts.toml`).
#[derive(Debug, Clone)]
pub struct Identities {
    pub list: Vec<Mailbox>,
}

impl Identities {
    /// The addresses, lowercase: never recipients of a reply, never decided on.
    pub fn own(&self) -> Vec<String> {
        self.list.iter().map(|m| m.addr.to_lowercase()).collect()
    }
}

/// Where a `send` action is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendPhase {
    /// Not accepted by the SMTP server yet.
    Smtp,
    /// Sent; the copy in the Sent folder of the server is to be made.
    Append,
}

/// A `send` action claimed by the sender.
#[derive(Debug, Clone)]
pub struct SendJob {
    pub action_id: i64,
    pub account_id: i64,
    pub draft_id: i64,
    pub phase: SendPhase,
    /// The envelope: sender and every recipient.
    pub from: String,
    pub rcpt: Vec<String>,
    pub mime: Vec<u8>,
    pub message_id_hdr: String,
    /// The server may have the mail already (see the header of this file).
    pub data_sent: bool,
    pub attempts: i64,
}

/// The time zone offset of the Mac at `t`, in seconds east of UTC, for the
/// `Date` of a mail.
///
/// Rust note: `unsafe` because `localtime_r` is a C function the compiler
/// cannot check. Its contract is simple (it writes the `tm` given), and
/// `localtime_r`, unlike `localtime`, is safe to call from several threads.
pub fn local_offset(t: i64) -> i32 {
    let time = t as libc::time_t;
    // SAFETY: `tm` is a plain C struct, valid when zeroed; `localtime_r`
    // only reads `time` and writes `tm`.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let ok = unsafe { !libc::localtime_r(&time, &mut tm).is_null() };
    if ok { tm.tm_gmtoff as i32 } else { 0 }
}

pub(crate) fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// The JSON of the recipients column: `[{kind, addr, name}]`.
fn recipients_json(to: &[Mailbox], cc: &[Mailbox]) -> String {
    let item = |kind: &str, m: &Mailbox| json!({"kind": kind, "addr": m.addr, "name": m.name});
    Value::Array(to.iter().map(|m| item("to", m)).chain(cc.iter().map(|m| item("cc", m))).collect()).to_string()
}

/// The recipients of a draft, `(to, cc, bcc)`.
fn recipients_of(json: &str) -> (Vec<Mailbox>, Vec<Mailbox>, Vec<Mailbox>) {
    let v: Value = serde_json::from_str(json).unwrap_or(Value::Null);
    let (mut to, mut cc, mut bcc) = (Vec::new(), Vec::new(), Vec::new());
    for r in v.as_array().into_iter().flatten() {
        let Some(addr) = r["addr"].as_str() else { continue };
        let m = Mailbox::new(addr, r["name"].as_str());
        match r["kind"].as_str() {
            Some("cc") => cc.push(m),
            Some("bcc") => bcc.push(m),
            _ => to.push(m),
        }
    }
    (to, cc, bcc)
}

fn view_of(m: &Mailbox) -> AddressView {
    AddressView { addr: m.addr.clone(), name: m.name.clone() }
}

/// The `send` action of a draft not ended: `(id, state, payload)`.
fn send_action(conn: &Connection, draft_id: i64) -> Result<Option<(i64, String, Value, i64)>> {
    let row: Option<(i64, String, String, i64)> = conn
        .prepare_cached(
            "SELECT id, state, payload, date_run_after FROM pending_action
              WHERE kind = 'send' AND state IN ('queued', 'running') AND json_extract(payload, '$.draft_id') = ?1
              ORDER BY id DESC LIMIT 1",
        )?
        .query_row([draft_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .optional()?;
    Ok(row.map(|(id, state, p, at)| (id, state, serde_json::from_str(&p).unwrap_or(Value::Null), at)))
}

/// The view of a draft, with its preselected filing and, if it is being
/// sent, when it leaves.
pub(crate) fn draft_view(conn: &Connection, id: i64) -> Result<DraftView> {
    type Row = (i64, String, Option<String>, String, String, String, Option<i64>, i64, String, Option<i64>);
    let row: Option<Row> = conn
        .prepare_cached(
            "SELECT d.account_id, d.from_addr, d.from_name, d.recipients, d.subject, d.body, d.reply_to_message_id,
                    d.revision, d.state, m.thread_id
               FROM draft d LEFT JOIN message m ON m.id = d.reply_to_message_id WHERE d.id = ?1",
        )?
        .query_row([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?, r.get(9)?)))
        .optional()?;
    let Some((account_id, from_addr, from_name, recipients, subject, body, reply_to, revision, state, thread_id)) = row else {
        return Err(StoreError::NotFound(format!("brouillon {id}")));
    };
    let (to, cc, _) = recipients_of(&recipients);
    let after_send_target = match thread_id {
        Some(t) => crate::reception::reply_preselection(conn, account_id, t)?,
        None => None,
    };
    let send_at = if state == "sending" { send_action(conn, id)?.map(|(_, _, _, at)| at) } else { None };
    Ok(DraftView {
        id,
        account_id,
        from: AddressView { addr: from_addr, name: from_name },
        to: to.iter().map(view_of).collect(),
        cc: cc.iter().map(view_of).collect(),
        subject,
        body,
        revision,
        state,
        reply_to,
        thread_id,
        after_send_target,
        send_at,
    })
}

impl Store {
    /// Pretended sending (`NICEMAILER_FAKE_NETWORK`, set by the core): an
    /// archive account (an mbox import, no server) may answer too, so that
    /// the reply can be tried on a real mailbox without sending anything.
    /// The daemon then records the mail as accepted itself, without network.
    pub fn set_fake_sending(&mut self, on: bool) {
        self.fake_sending = on;
    }

    /// Starts a reply to all to the message `reply_to`, or gives back the
    /// one already started (`reused`), so that `r` twice never makes two
    /// drafts (`reponse.md`, section 3).
    ///
    /// `quote_header` is the line introducing the quotation, in the
    /// client's language; `body`, when given, replaces the quotation (a
    /// draft abandoned by mistake, restored).
    ///
    /// Refused: a message of an archive account (`CannotSend`), a draft,
    /// a message whose body is not downloaded (`BodyPending`).
    pub fn create_reply_draft(
        &mut self,
        reply_to: i64,
        identities: &Identities,
        quote_header: &str,
        body: Option<&str>,
        source: Source,
    ) -> Result<(DraftView, bool)> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        // The reply to this mail already started, or, `reply_to` being the
        // draft's own row of Drafts, that draft (`r` in the Drafts folder).
        let existing: Option<i64> = tx
            .query_row(
                "SELECT id FROM draft WHERE reply_to_message_id = ?1 OR message_id = ?1 ORDER BY message_id = ?1 DESC, id LIMIT 1",
                [reply_to],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id) = existing {
            let view = draft_view(&tx, id)?;
            tx.commit()?;
            return Ok((view, true));
        }
        type Parent = (i64, String, bool, i64, Option<String>, Option<String>, String, Option<String>, Option<String>);
        let parent: Option<Parent> = tx
            .query_row(
                "SELECT m.account_id, a.kind, m.is_draft, m.body_state, m.message_id_hdr, m.in_reply_to, m.subject,
                        m.from_addr, m.from_name
                   FROM message m JOIN account a ON a.id = m.account_id WHERE m.id = ?1",
                [reply_to],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?)),
            )
            .optional()?;
        let Some((account_id, kind, is_draft, body_state, _msgid, _irt, subject, from_addr, from_name)) = parent else {
            return Err(StoreError::NotFound(format!("message {reply_to}")));
        };
        if kind == "archive" && !self.fake_sending {
            return Err(StoreError::CannotSend("this mail is in an archive account, which cannot send".into()));
        }
        if is_draft {
            return Err(StoreError::CannotSend("a draft cannot be answered".into()));
        }
        if body.is_none() && body_state == 0 {
            return Err(StoreError::BodyPending);
        }
        let (to, cc, rt) = message_addresses(&tx, reply_to)?;
        let from = from_addr.map(|a| Mailbox { addr: a, name: from_name });
        let original = Original { from: from.as_ref(), reply_to: &rt, to: &to, cc: &cc };
        let own = identities.own();
        let (r_to, r_cc) = compose::reply_all(&original, &own);
        let sender = compose::sender_identity(&identities.list, &original);
        if sender.addr.is_empty() {
            return Err(StoreError::CannotSend("the account has no address to send from".into()));
        }
        let text = match body {
            Some(b) => b.to_string(),
            None => {
                let quoted = compose::quote(&message_text(&tx, reply_to)?, compose::QUOTE_MAX_LINES);
                compose::initial_body(quote_header, &quoted)
            }
        };
        let message_id_hdr = compose::new_message_id(rand::random::<u128>(), &sender.addr);
        tx.execute(
            "INSERT INTO draft (account_id, from_addr, from_name, recipients, subject, body, body_format,
                                reply_to_message_id, message_id_hdr, created_by, modified_by)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'text', ?7, ?8, ?9, ?9)",
            params![
                account_id,
                sender.addr,
                sender.name,
                recipients_json(&r_to, &r_cc),
                compose::reply_subject(&subject),
                text,
                reply_to,
                message_id_hdr,
                source.as_str()
            ],
        )?;
        let id = tx.last_insert_rowid();
        let view = draft_view(&tx, id)?;
        tx.commit()?;
        Ok((view, false))
    }

    /// A draft, as the reply screen shows it.
    pub fn get_draft(&self, id: i64) -> Result<DraftView> {
        draft_view(&self.conn, id)
    }

    /// The draft of a reply to `reply_to`, if one was started.
    pub fn find_reply_draft(&self, reply_to: i64) -> Result<Option<DraftView>> {
        let id: Option<i64> = self
            .conn
            .query_row("SELECT id FROM draft WHERE reply_to_message_id = ?1 ORDER BY id LIMIT 1", [reply_to], |r| r.get(0))
            .optional()?;
        id.map(|id| draft_view(&self.conn, id)).transpose()
    }

    /// Saves the body typed. `revision` must be the draft's current one
    /// (`RevisionConflict` otherwise); a body unchanged changes nothing.
    /// Returns the new revision.
    pub fn update_draft(&mut self, id: i64, revision: i64, body: &str, source: Source) -> Result<i64> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let rev = editable(&tx, id, revision)?;
        let rev = save_body(&tx, id, rev, body, source)?;
        tx.commit()?;
        Ok(rev)
    }

    /// Abandons a draft, and returns it as it was: the client keeps it to
    /// restore it (`z`, through `create_reply_draft` with its body). A
    /// draft being sent is refused: `cancel_send` first.
    pub fn discard_draft(&mut self, id: i64, source: Source) -> Result<(DraftView, ChangeSet)> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let view = draft_view(&tx, id)?;
        if view.state == "sending" {
            return Err(StoreError::DraftSending);
        }
        // Its row of Drafts goes, and its copy on the server.
        let mut changes = ChangeSet::default();
        crate::draft_copies::forget_copies(&tx, id, source, &mut changes)?;
        tx.execute("DELETE FROM draft WHERE id = ?1", [id])?;
        tx.commit()?;
        Ok((view, changes))
    }

    /// Queues the draft for sending, due in `delay_secs` (the undo delay,
    /// `reponse.md`, section 7). The body is saved first, at `revision`.
    /// The MIME message is built now: it is what will leave, whatever
    /// happens to the draft. Returns when it leaves.
    pub fn send_draft(
        &mut self,
        id: i64,
        revision: i64,
        body: &str,
        after_send: Option<FolderRole>,
        delay_secs: u64,
        user_agent: &str,
        source: Source,
    ) -> Result<(i64, ChangeSet)> {
        if let Some(t) = after_send
            && !matches!(t, FolderRole::Archive | FolderRole::Waiting | FolderRole::Todo)
        {
            return Err(StoreError::InvalidParams(format!("after_send: archive, waiting or todo, not {}", t.as_str())));
        }
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let rev = editable(&tx, id, revision)?;
        save_body(&tx, id, rev, body, source)?;
        let Built { account_id, from_addr, rcpt, mime, msgid, date: t } = build_draft(&tx, id, user_agent, None)?;
        if rcpt.is_empty() {
            return Err(StoreError::CannotSend("this draft has no recipient".into()));
        }
        let send_at = t + delay_secs as i64;
        let payload = json!({
            "draft_id": id,
            "phase": "smtp",
            "from": from_addr,
            "rcpt": rcpt,
            // The MIME message is ASCII (`build_message` encodes the rest).
            "mime": String::from_utf8_lossy(&mime),
            "message_id_hdr": msgid,
            "after_send": after_send.map(|r| r.as_str()),
            "data_sent": false,
        });
        tx.execute(
            "INSERT INTO pending_action (account_id, kind, message_id, payload, created_by, date_run_after)
             VALUES (?1, 'send', NULL, ?2, ?3, ?4)",
            params![account_id, payload.to_string(), source.as_str(), send_at],
        )?;
        tx.execute("UPDATE draft SET state = 'sending', date_modified = unixepoch(), modified_by = ?2 WHERE id = ?1", params![id, source.as_str()])?;
        tx.commit()?;
        let mut changes = ChangeSet::default();
        changes.sending.insert(account_id);
        Ok((send_at, changes))
    }

    /// Cancels a send during the undo delay: the action is `cancelled`, the
    /// draft editable again. Too late (`SendStarted`) once the sender has
    /// started on it, or once the server may have the mail.
    pub fn cancel_send(&mut self, id: i64, source: Source) -> Result<(DraftView, ChangeSet)> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let view = draft_view(&tx, id)?;
        if view.state != "sending" {
            return Err(StoreError::InvalidParams(format!("draft {id} is not being sent")));
        }
        match send_action(&tx, id)? {
            Some((action, state, p, _)) if state == "queued" && p["phase"] == "smtp" && p["data_sent"] != true => {
                tx.execute(
                    "UPDATE pending_action SET state = 'cancelled', last_error = 'cancelled by the user', date_modified = unixepoch()
                      WHERE id = ?1",
                    [action],
                )?;
            }
            // No action at all: a draft left `sending` by a bug; editable.
            None => {}
            Some(_) => return Err(StoreError::SendStarted),
        }
        tx.execute("UPDATE draft SET state = 'editing', date_modified = unixepoch(), modified_by = ?2 WHERE id = ?1", params![id, source.as_str()])?;
        let view = draft_view(&tx, id)?;
        tx.commit()?;
        let mut changes = ChangeSet::default();
        changes.sending.insert(view.account_id);
        Ok((view, changes))
    }

    /// At start: a draft `sending` whose action is gone (cancelled, purged)
    /// would stay frozen for ever: it is editable again.
    pub fn repair_drafts(&mut self) -> Result<usize> {
        Ok(self.conn.execute(
            "UPDATE draft SET state = 'editing', date_modified = unixepoch(), modified_by = 'core'
              WHERE state = 'sending' AND NOT EXISTS (
                    SELECT 1 FROM pending_action a WHERE a.kind = 'send' AND a.state IN ('queued', 'running')
                       AND json_extract(a.payload, '$.draft_id') = draft.id)",
            [],
        )?)
    }

    // -----------------------------------------------------------------
    // The sender's side (`nm-engine`, `sync/send.rs`)
    // -----------------------------------------------------------------

    /// The `send` actions of `account_id` due now, marked `running`.
    pub fn send_claim(&mut self, account_id: i64) -> Result<Vec<SendJob>> {
        let tx = self.transaction()?;
        let rows: Vec<(i64, String, i64)> = tx
            .prepare(
                "SELECT id, payload, attempts FROM pending_action
                  WHERE account_id = ?1 AND kind = 'send' AND state = 'queued' AND date_run_after <= unixepoch()
                  ORDER BY id",
            )?
            .query_map([account_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<std::result::Result<_, _>>()?;
        let mut jobs = Vec::new();
        for (id, payload, attempts) in rows {
            let p: Value = serde_json::from_str(&payload).unwrap_or(Value::Null);
            let (Some(draft_id), Some(from), Some(mime), Some(msgid)) =
                (p["draft_id"].as_i64(), p["from"].as_str(), p["mime"].as_str(), p["message_id_hdr"].as_str())
            else {
                // Unreadable: never sent, never claimed again.
                tx.execute("UPDATE pending_action SET state = 'reverted', last_error = 'unreadable payload' WHERE id = ?1", [id])?;
                continue;
            };
            tx.execute("UPDATE pending_action SET state = 'running', date_modified = unixepoch() WHERE id = ?1", [id])?;
            jobs.push(SendJob {
                action_id: id,
                account_id,
                draft_id,
                phase: if p["phase"] == "append" { SendPhase::Append } else { SendPhase::Smtp },
                from: from.to_string(),
                rcpt: p["rcpt"].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_string)).collect(),
                mime: mime.as_bytes().to_vec(),
                message_id_hdr: msgid.to_string(),
                data_sent: p["data_sent"] == true,
                attempts,
            });
        }
        tx.commit()?;
        Ok(jobs)
    }

    /// When the next `send` action of `account_id` is due, Unix seconds.
    pub fn send_next_due(&self, account_id: i64) -> Result<Option<i64>> {
        Ok(self.conn.query_row(
            "SELECT min(date_run_after) FROM pending_action WHERE account_id = ?1 AND kind = 'send' AND state = 'queued'",
            [account_id],
            |r| r.get(0),
        )?)
    }

    /// The sender is about to tell the server the message is complete:
    /// from now on it may have left, whatever happens.
    pub fn send_data_started(&mut self, action_id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE pending_action SET payload = json_set(payload, '$.data_sent', json('true')) WHERE id = ?1",
            [action_id],
        )?;
        Ok(())
    }

    /// The SMTP server accepted the mail (`reponse.md`, section 7, "Après
    /// l'acceptation"), in one transaction:
    ///
    /// 1. the mail becomes a row of the Sent folder, complete (its raw file,
    ///    `written`, is what was sent), without UID until the copy on the
    ///    server is made;
    /// 2. its conversation is filed (section 8);
    /// 3. its recipients without decision get `inbox` (section 10);
    /// 4. the draft row is deleted;
    /// 5. a `send` row in `audit_log`; `DraftSent` with the undo token of
    ///    the filing and the decisions.
    ///
    /// The action goes on in phase `append`. `refused`: recipients the
    /// server refused, the others got the mail (`SendPartial`). `own`: the
    /// account's addresses, never decided on.
    pub fn send_accepted(&mut self, action_id: i64, written: &Written, refused: &[String], own: &[String]) -> Result<ChangeSet> {
        let mut changes = ChangeSet::default();
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row: Option<(i64, String, String)> = tx
            .query_row("SELECT account_id, payload, created_by FROM pending_action WHERE id = ?1", [action_id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .optional()?;
        let Some((account_id, payload, created_by)) = row else { return Ok(changes) };
        let p: Value = serde_json::from_str(&payload).unwrap_or(Value::Null);
        if p["phase"] == "append" {
            // Already recorded (the call is replayed after a crash).
            return Ok(changes);
        }
        let source = Source::parse(&created_by).unwrap_or(Source::Core);
        let draft_id = p["draft_id"].as_i64().unwrap_or(0);
        let reply_to: Option<i64> =
            tx.query_row("SELECT reply_to_message_id FROM draft WHERE id = ?1", [draft_id], |r| r.get(0)).optional()?.flatten();
        let mime = p["mime"].as_str().unwrap_or("").as_bytes().to_vec();
        let parsed = crate::ingest::parse(&mime);
        let sent_folder: Option<i64> = tx
            .query_row("SELECT id FROM folder WHERE account_id = ?1 AND role = 'sent'", [account_id], |r| r.get(0))
            .optional()?;
        let mut inserter = Inserter::new(source, None);
        let mut threads = BTreeSet::new();
        let mut sent_id = None;
        let mut moved_to = None;
        let mark: i64;
        if let Some(folder) = sent_folder {
            let m = NewMessage {
                account_id,
                folder_id: folder,
                uid: None,
                date_received: now(),
                size: Some(written.size),
                modseq: None,
                flags: Flags { seen: true, ..Flags::default() },
                // One's own mail: nothing to verify.
                auth: (AuthResult::None, None),
                headers: &parsed,
            };
            let id = inserter.insert(&tx, &m, Some(Body { written, parsed: &parsed }), &mut changes)?;
            sent_id = Some(id);
            // The `send` row before the undo range: undoing the filing never
            // pretends to undo the sending.
            send_audit(&tx, source, id, &p, refused)?;
            mark = audit_mark(&tx)?;
            let thread: Option<i64> = tx.query_row("SELECT thread_id FROM message WHERE id = ?1", [id], |r| r.get(0))?;
            // The conversation of the mail answered: the reply joined it by
            // its references, unless the mail answered had no Message-ID.
            let parent_thread: Option<i64> = match reply_to {
                Some(r) => tx.query_row("SELECT thread_id FROM message WHERE id = ?1", [r], |r| r.get(0)).optional()?.flatten(),
                None => None,
            };
            // A reply without `In-Reply-To` (the mail answered had no
            // Message-ID) files nothing (`dossiers.md`).
            if let Some(t) = parent_thread.or(thread).filter(|_| parsed.in_reply_to.is_some()) {
                let after = p["after_send"].as_str().and_then(FolderRole::parse);
                moved_to = crate::reception::file_reply_thread(&tx, account_id, t, id, after, source, "reply", &mut changes, &mut threads)?;
            }
            crate::reception::accept_recipients(&tx, own, id, source, &mut changes, &mut threads)?;
            tx.execute(
                "UPDATE pending_action SET message_id = ?2, date_modified = unixepoch(),
                        payload = json_set(payload, '$.phase', 'append')
                  WHERE id = ?1",
                params![action_id, id],
            )?;
        } else {
            // No Sent folder known: the mail left, nothing to copy.
            send_audit(&tx, source, reply_to.unwrap_or(0), &p, refused)?;
            mark = audit_mark(&tx)?;
            tx.execute(
                "UPDATE pending_action SET state = 'done', last_error = 'no Sent folder: no copy kept', date_modified = unixepoch()
                  WHERE id = ?1",
                [action_id],
            )?;
        }
        // Its row of Drafts goes, and its copy on the server.
        crate::draft_copies::forget_copies(&tx, draft_id, source, &mut changes)?;
        tx.execute("DELETE FROM draft WHERE id = ?1", [draft_id])?;
        crate::screening::refresh_all(&tx, &threads, &mut changes)?;
        let undo = undo_token(&tx, mark)?;
        let thread_id = match sent_id {
            Some(id) => tx.query_row("SELECT thread_id FROM message WHERE id = ?1", [id], |r| r.get(0))?,
            None => None,
        };
        if let Some(message_id) = sent_id {
            changes.sent.push(SentDraft { draft_id, message_id, thread_id, moved_to, undo });
        }
        if !refused.is_empty() {
            changes.send_partial.push((draft_id, refused.to_vec()));
        }
        inserter.commit(tx, &mut changes)?;
        Ok(changes)
    }

    /// A temporary failure of a `send` action: tried again later, with the
    /// delays of the spool. After the last attempt: in phase `smtp`, a
    /// definitive failure ([`Store::send_failed`]); in phase `append`, the
    /// mail left anyway, the action ends without a copy on the server.
    pub fn send_retry(&mut self, action_id: i64, error: &str) -> Result<ChangeSet> {
        let (attempts, payload): (i64, String) =
            self.conn.query_row("SELECT attempts + 1, payload FROM pending_action WHERE id = ?1", [action_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        let append = serde_json::from_str::<Value>(&payload).is_ok_and(|p| p["phase"] == "append");
        if attempts >= crate::spool::MAX_ATTEMPTS {
            let reason = format!("{attempts} attempts failed, the last: {error}");
            if append {
                self.conn.execute(
                    "UPDATE pending_action SET state = 'done', attempts = ?2, last_error = ?3, date_modified = unixepoch() WHERE id = ?1",
                    params![action_id, attempts, format!("sent, but not copied to Sent: {reason}")],
                )?;
                return Ok(ChangeSet::default());
            }
            return self.send_failed(action_id, &reason);
        }
        let delay = (crate::spool::RETRY_FIRST_SECS << (attempts - 1).min(20)).min(crate::spool::RETRY_MAX_SECS);
        self.conn.execute(
            "UPDATE pending_action SET state = 'queued', attempts = ?2, last_error = ?3,
                    date_run_after = unixepoch() + ?4, date_modified = unixepoch()
              WHERE id = ?1",
            params![action_id, attempts, error, delay],
        )?;
        Ok(ChangeSet::default())
    }

    /// The mail cannot be sent, for good: the action is `reverted`, the
    /// draft editable again, `SendFailed` tells the user.
    pub fn send_failed(&mut self, action_id: i64, reason: &str) -> Result<ChangeSet> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut changes = ChangeSet::default();
        let payload: Option<String> =
            tx.query_row("SELECT payload FROM pending_action WHERE id = ?1", [action_id], |r| r.get(0)).optional()?;
        let Some(payload) = payload else { return Ok(changes) };
        let p: Value = serde_json::from_str(&payload).unwrap_or(Value::Null);
        tx.execute(
            "UPDATE pending_action SET state = 'reverted', last_error = ?2, date_modified = unixepoch() WHERE id = ?1",
            params![action_id, reason],
        )?;
        if let Some(draft_id) = p["draft_id"].as_i64() {
            tx.execute("UPDATE draft SET state = 'editing', date_modified = unixepoch(), modified_by = 'core' WHERE id = ?1", [draft_id])?;
            let reply_to: Option<i64> =
                tx.query_row("SELECT reply_to_message_id FROM draft WHERE id = ?1", [draft_id], |r| r.get(0)).optional()?.flatten();
            tx.execute(
                "INSERT INTO audit_log (created_by, entity, entity_id, action, detail)
                 VALUES ('core', 'message', ?1, 'send_failed', json_object('message_id_hdr', ?2, 'reason', ?3))",
                params![reply_to.unwrap_or(0), p["message_id_hdr"].as_str(), reason],
            )?;
            changes.send_failed.push((draft_id, reply_to, reason.to_string()));
        }
        tx.commit()?;
        Ok(changes)
    }

    /// The copy in Sent is made (or found): the row of the mail gets its
    /// UID when known, and the action ends. Without a UID (a server without
    /// UIDPLUS, a provider that copies itself), the synchronization of Sent
    /// attaches the row by its `Message-ID`.
    pub fn send_copied(&mut self, action_id: i64, uid: Option<u32>) -> Result<ChangeSet> {
        let tx = self.transaction()?;
        let mut changes = ChangeSet::default();
        let message: Option<i64> =
            tx.query_row("SELECT message_id FROM pending_action WHERE id = ?1", [action_id], |r| r.get(0)).optional()?.flatten();
        if let (Some(id), Some(uid)) = (message, uid) {
            // A pass may have attached it meanwhile: never two rows on one UID.
            let n = tx.execute(
                "UPDATE message SET uid = ?2 WHERE id = ?1 AND uid IS NULL
                   AND NOT EXISTS (SELECT 1 FROM message o WHERE o.folder_id = message.folder_id AND o.uid = ?2)",
                params![id, uid],
            )?;
            if n > 0 {
                let thread: Option<i64> = tx.query_row("SELECT thread_id FROM message WHERE id = ?1", [id], |r| r.get(0))?;
                changes.messages.insert((id, thread));
            }
        }
        tx.execute("UPDATE pending_action SET state = 'done', last_error = NULL, date_modified = unixepoch() WHERE id = ?1", [action_id])?;
        tx.commit()?;
        Ok(changes)
    }

    /// The Sent folder of an account: `(id, path)`.
    pub fn sent_folder(&self, account_id: i64) -> Result<Option<(i64, String)>> {
        Ok(self
            .conn
            .query_row("SELECT id, path FROM folder WHERE account_id = ?1 AND role = 'sent'", [account_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?)
    }

    /// Whether the database has the mail of this `Message-ID` in the Sent
    /// folder of the account: sent already (`Double envoi`).
    pub fn sent_has(&self, account_id: i64, message_id_hdr: &str) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM message m JOIN folder f ON f.id = m.folder_id
                             WHERE m.account_id = ?1 AND f.role = 'sent' AND m.message_id_hdr = ?2)",
            params![account_id, message_id_hdr],
            |r| r.get(0),
        )?)
    }
}

/// A draft made a MIME message: what `send_draft` queues and what
/// `close_draft` copies to Drafts.
struct Built {
    account_id: i64,
    from_addr: String,
    /// Every recipient, To, Cc and Bcc: the SMTP envelope.
    rcpt: Vec<String>,
    mime: Vec<u8>,
    msgid: String,
    date: i64,
}

/// The MIME message of a draft as it is now, its conversation headers from
/// the mail it answers. `draft_revision`: for a copy in Drafts
/// (`X-NiceMailer-Draft-Revision`), `None` for the mail sent.
fn build_draft(conn: &Connection, id: i64, user_agent: &str, draft_revision: Option<i64>) -> Result<Built> {
    type Row = (i64, String, Option<String>, String, String, String, String, Option<i64>);
    let (account_id, from_addr, from_name, recipients, subject, body, msgid, reply_to): Row = conn
        .query_row(
            "SELECT account_id, from_addr, from_name, recipients, subject, body, message_id_hdr, reply_to_message_id
               FROM draft WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?)),
        )
        .optional()?
        .ok_or_else(|| StoreError::NotFound(format!("brouillon {id}")))?;
    let (to, cc, bcc) = recipients_of(&recipients);
    let rcpt: Vec<String> = to.iter().chain(&cc).chain(&bcc).map(|m| m.addr.clone()).collect();
    // The conversation headers, from the mail answered as it is now.
    let (in_reply_to, references) = match reply_to {
        Some(p) => parent_ids(conn, p)?,
        None => (None, Vec::new()),
    };
    let t = now();
    let from = Mailbox { addr: from_addr.clone(), name: from_name };
    let mime = compose::build_message(
        &compose::Outgoing {
            from: &from,
            to: &to,
            cc: &cc,
            subject: &subject,
            date: t,
            tz_offset_secs: local_offset(t),
            message_id: &msgid,
            in_reply_to: in_reply_to.as_deref(),
            references: &references,
            user_agent,
            draft_revision,
        },
        &body,
    );
    Ok(Built { account_id, from_addr, rcpt, mime, msgid, date: t })
}

/// The MIME message of a draft as a copy for Drafts.
pub(crate) fn build_draft_mime(conn: &Connection, id: i64, user_agent: &str, revision: Option<i64>) -> Result<Vec<u8>> {
    Ok(build_draft(conn, id, user_agent, revision)?.mime)
}

/// The `send` row of `audit_log`, on the mail sent (or, without Sent
/// folder, the mail answered).
fn send_audit(tx: &Transaction, source: Source, entity: i64, p: &Value, refused: &[String]) -> Result<()> {
    tx.execute(
        "INSERT INTO audit_log (created_by, entity, entity_id, action, detail)
         VALUES (?1, 'message', ?2, 'send', json_object('message_id_hdr', ?3, 'recipients', json(?4), 'refused', json(?5)))",
        params![
            source.as_str(),
            entity,
            p["message_id_hdr"].as_str(),
            p["rcpt"].to_string(),
            serde_json::to_string(refused).unwrap_or_else(|_| "[]".into())
        ],
    )?;
    Ok(())
}

/// The revision of an editable draft, checked against the client's.
fn editable(tx: &Transaction, id: i64, revision: i64) -> Result<i64> {
    let row: Option<(i64, String)> =
        tx.query_row("SELECT revision, state FROM draft WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
    let Some((current, state)) = row else { return Err(StoreError::NotFound(format!("brouillon {id}"))) };
    if state == "sending" {
        return Err(StoreError::DraftSending);
    }
    if current != revision {
        return Err(StoreError::RevisionConflict { current });
    }
    Ok(current)
}

/// Writes the body, a new revision if it changed; returns the revision.
fn save_body(tx: &Transaction, id: i64, rev: i64, body: &str, source: Source) -> Result<i64> {
    let n = tx.execute(
        "UPDATE draft SET body = ?2, revision = revision + 1, date_modified = unixepoch(), modified_by = ?3
          WHERE id = ?1 AND body <> ?2",
        params![id, body, source.as_str()],
    )?;
    Ok(if n > 0 { rev + 1 } else { rev })
}

/// The recipients of a message, `(to, cc, reply_to)`.
fn message_addresses(conn: &Connection, message_id: i64) -> Result<(Vec<Mailbox>, Vec<Mailbox>, Vec<Mailbox>)> {
    let rows: Vec<(String, String, Option<String>)> = conn
        .prepare_cached(
            "SELECT ma.role, a.addr, a.name FROM message_address ma JOIN address a ON a.id = ma.address_id
              WHERE ma.message_id = ?1 ORDER BY ma.role, ma.position",
        )?
        .query_map([message_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<std::result::Result<_, _>>()?;
    let (mut to, mut cc, mut rt) = (Vec::new(), Vec::new(), Vec::new());
    for (role, addr, name) in rows {
        let m = Mailbox { addr, name };
        match role.as_str() {
            "to" => to.push(m),
            "cc" => cc.push(m),
            "reply_to" => rt.push(m),
            _ => {}
        }
    }
    Ok((to, cc, rt))
}

/// The text of a message, as the reader shows it: the text part, or the
/// HTML made text.
fn message_text(conn: &Connection, message_id: i64) -> Result<String> {
    let row: Option<(Option<String>, Option<String>)> = conn
        .query_row("SELECT text_plain, html_sanitized FROM message_body WHERE message_id = ?1", [message_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    Ok(match row {
        Some((Some(t), _)) if !t.trim().is_empty() => t,
        Some((_, Some(h))) => crate::html_text::html_to_text(&h),
        _ => String::new(),
    })
}

/// `In-Reply-To` and `References` of a reply to `parent`.
fn parent_ids(conn: &Connection, parent: i64) -> Result<(Option<String>, Vec<String>)> {
    let row: Option<(Option<String>, Option<String>)> = conn
        .query_row("SELECT message_id_hdr, in_reply_to FROM message WHERE id = ?1", [parent], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    let Some((msgid, irt)) = row else { return Ok((None, Vec::new())) };
    let refs: Vec<String> = conn
        .prepare_cached("SELECT ref_msgid FROM message_reference WHERE message_id = ?1 ORDER BY position")?
        .query_map([parent], |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    let msgid = msgid.filter(|m| compose::valid_msgid(m));
    let references = if msgid.is_some() { compose::references(&refs, irt.as_deref(), msgid.as_deref()) } else { Vec::new() };
    Ok((msgid, references))
}
