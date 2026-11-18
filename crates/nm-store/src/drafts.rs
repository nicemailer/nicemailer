//! Replies being written, and their sending (`specifications/reponse.md`;
//! the life of a draft is `brouillons.md`, section 4).
//!
//! # The draft
//!
//! A reply is first a proposal, computed without writing
//! ([`Store::prepare_reply_draft`]), then a row of `draft`, written by
//! [`Store::create_reply_draft`] at the first pause of typing:
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

use nm_api::{AddressView, DraftAttachmentView, DraftView};
use nm_core::compose::{self, Mailbox, Original};
use nm_core::{AfterSend, AuthResult, FolderRole, Source};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::{Value, json};

use crate::insert::{Body, Flags, Inserter, NewMessage};
use crate::outgoing::{OutgoingStore, parse_hex};
use crate::raw::Written;
use crate::screening::{audit_mark, undo_token};
use crate::{ChangeSet, Result, Store, StoreError};

/// A draft sent, for the event `DraftSent`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentDraft {
    pub draft_id: i64,
    pub message_id: i64,
    pub thread_id: Option<i64>,
    /// Where the conversation of the mail answered was filed, already
    /// done and undone by `undo`; `None` when it stayed.
    pub moved_to: Option<FolderRole>,
    /// Where a new mail will be filed once its copy in Sent is made
    /// (`reponse.md`, section 8, "Nouveau mail"): decided, not done yet,
    /// and **not** covered by `undo`.
    pub filing: Option<FolderRole>,
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

/// An account that can send, with its addresses.
#[derive(Debug, Clone)]
pub struct Sender {
    pub account_id: i64,
    pub identities: Identities,
}

/// Who sends a reply or a forward (`reponse.md`, section 4 ter), as the
/// daemon found it in `accounts.toml`.
#[derive(Debug, Clone)]
pub struct Sending {
    /// The addresses of the account of the mail, never recipients of the
    /// reply. That account may be none of `senders` (an archive account).
    pub own: Vec<String>,
    /// The accounts that may send it, by preference: the mail's own account
    /// alone when it can send; else every account that can, the first one
    /// the mail was sent to winning (`compose::choose_sender`).
    pub senders: Vec<Sender>,
}

impl Sending {
    /// The mail's own account sends it.
    pub fn own_account(account_id: i64, identities: Identities) -> Sending {
        Sending { own: identities.own(), senders: vec![Sender { account_id, identities }] }
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
    /// The mail given to the SMTP server: no `Bcc` header.
    pub mime: Vec<u8>,
    /// The copy the user keeps (raw store, Sent folder of the server): the
    /// same, with `Bcc` when there are hidden recipients.
    pub copy: Vec<u8>,
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
fn recipients_json(to: &[Mailbox], cc: &[Mailbox], bcc: &[Mailbox]) -> String {
    let item = |kind: &str, m: &Mailbox| json!({"kind": kind, "addr": m.addr, "name": m.name});
    Value::Array(
        to.iter()
            .map(|m| item("to", m))
            .chain(cc.iter().map(|m| item("cc", m)))
            .chain(bcc.iter().map(|m| item("bcc", m)))
            .collect(),
    )
    .to_string()
}

/// What the user changed in a draft ([`Store::edit_draft`]); `None`: left
/// as it is. Each recipient list is given whole.
#[derive(Debug, Clone, Default)]
pub struct DraftEdit {
    pub body: Option<String>,
    pub subject: Option<String>,
    pub to: Option<Vec<Mailbox>>,
    pub cc: Option<Vec<Mailbox>>,
    pub bcc: Option<Vec<Mailbox>>,
    /// The account that sends, and the address, checked by the daemon
    /// against `accounts.toml` (`reponse.md`, section 4 ter).
    pub from: Option<(i64, Mailbox)>,
    /// `text` (text only) or `html` (formatted, the editor's HTML in
    /// `body`), `edition.md`, section 3. Anything else is refused.
    pub body_format: Option<String>,
}

/// Checks a draft's format and the size of its body: `text` or `html`,
/// and an HTML body of at most `mail_html::MAX_HTML_BYTES`
/// (`edition.md`, section 11).
fn check_body(format: &str, body: &str) -> Result<()> {
    match format {
        "text" => Ok(()),
        "html" if body.len() <= crate::mail_html::MAX_HTML_BYTES => Ok(()),
        "html" => Err(StoreError::InvalidParams(format!(
            "the formatted text is too big: {} bytes, {} at most",
            body.len(),
            crate::mail_html::MAX_HTML_BYTES
        ))),
        other => Err(StoreError::InvalidParams(format!("unknown body format: {other}"))),
    }
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
    type Row = (i64, String, Option<String>, String, String, String, Option<i64>, i64, String, Option<i64>, Option<String>, Option<i64>, i64, String);
    let row: Option<Row> = conn
        .prepare_cached(
            "SELECT d.account_id, d.from_addr, d.from_name, d.recipients, d.subject, d.body, d.reply_to_message_id,
                    d.revision, d.state, m.thread_id, d.after_send, d.forward_of_message_id,
                    coalesce(d.send_account_id, d.account_id), d.body_format
               FROM draft d LEFT JOIN message m ON m.id = d.reply_to_message_id WHERE d.id = ?1",
        )?
        .query_row([id], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
                r.get(8)?,
                r.get(9)?,
                r.get(10)?,
                r.get(11)?,
                r.get(12)?,
                r.get(13)?,
            ))
        })
        .optional()?;
    let Some((
        account_id,
        from_addr,
        from_name,
        recipients,
        subject,
        body,
        reply_to,
        revision,
        state,
        thread_id,
        after_send,
        forward_of,
        send_account_id,
        body_format,
    )) = row
    else {
        return Err(StoreError::NotFound(format!("draft {id}")));
    };
    let (to, cc, bcc) = recipients_of(&recipients);
    let after_send_target = match thread_id {
        Some(t) => crate::reception::reply_preselection(conn, account_id, t)?,
        None => None,
    };
    let send_at = if state == "sending" { send_action(conn, id)?.map(|(_, _, _, at)| at) } else { None };
    let attachments = draft_attachments(conn, id)?.into_iter().map(|(view, _)| view).collect();
    Ok(DraftView {
        id,
        account_id,
        send_account_id,
        from: AddressView { addr: from_addr, name: from_name },
        to: to.iter().map(view_of).collect(),
        cc: cc.iter().map(view_of).collect(),
        bcc: bcc.iter().map(view_of).collect(),
        subject,
        body,
        body_format,
        attachments,
        revision,
        state,
        kind: if reply_to.is_some() {
            "reply"
        } else if forward_of.is_some() {
            "forward"
        } else {
            "new"
        }
        .to_string(),
        reply_to,
        forward_of,
        thread_id,
        after_send_target,
        after_send: after_send.as_deref().and_then(AfterSend::parse),
        send_at,
    })
}

/// The attachments of a draft, in order, with the hash of their file.
fn draft_attachments(conn: &Connection, id: i64) -> Result<Vec<(DraftAttachmentView, crate::raw::Hash)>> {
    let rows: Vec<(i64, String, String, i64, Vec<u8>)> = conn
        .prepare_cached(
            "SELECT a.id, a.filename, a.mime_type, b.size, a.hash FROM draft_attachment a JOIN outgoing_blob b ON b.hash = a.hash
              WHERE a.draft_id = ?1 ORDER BY a.position, a.id",
        )?
        .query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?
        .collect::<std::result::Result<_, _>>()?;
    Ok(rows
        .into_iter()
        .filter_map(|(id, filename, mime_type, size, hash)| {
            let hash = <crate::raw::Hash>::try_from(hash.as_slice()).ok()?;
            Some((DraftAttachmentView { id, filename, mime_type, size }, hash))
        })
        .collect())
}

impl Store {
    /// The files of mails to send (`outgoing.rs`): set by the core at start.
    pub fn set_outgoing(&mut self, outgoing: OutgoingStore) {
        self.outgoing = Some(outgoing);
    }

    fn outgoing(&self) -> Result<&OutgoingStore> {
        self.outgoing.as_ref().ok_or_else(|| StoreError::Io("no store for the files to send".into()))
    }

    /// Attaches the file `staged`, put in `incoming.noindex` by the app
    /// (`brouillons.md`, section 7): the file is taken into
    /// `outgoing.noindex` ([`OutgoingStore::take`]), then the row of
    /// `draft_attachment` is added, a new revision of the draft (its next
    /// copy to Drafts carries the file). Returns the revision, the
    /// attachment, and a warning when the mail grows beyond what most
    /// providers accept.
    ///
    /// Refused: a draft being sent, a stale revision, attachments beyond
    /// `compose::ATTACHMENTS_MAX` together (the file taken is then deleted,
    /// unless another draft has it).
    #[allow(clippy::too_many_arguments)]
    pub fn add_draft_attachment(
        &mut self,
        id: i64,
        revision: i64,
        staged: &str,
        filename: &str,
        mime_type: Option<&str>,
        source: Source,
    ) -> Result<(i64, DraftAttachmentView, Option<String>)> {
        let outgoing = self.outgoing()?.clone();
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let rev = editable(&tx, id, revision)?;
        let (hash, size) = outgoing.take(staged).map_err(|e| match e.kind() {
            std::io::ErrorKind::InvalidInput | std::io::ErrorKind::NotFound => StoreError::InvalidParams(e.to_string()),
            _ => StoreError::Io(e.to_string()),
        })?;
        let (total, body_len): (i64, i64) = tx.query_row(
            "SELECT coalesce((SELECT sum(b.size) FROM draft_attachment a JOIN outgoing_blob b ON b.hash = a.hash WHERE a.draft_id = ?1), 0),
                    (SELECT length(CAST(body AS BLOB)) FROM draft WHERE id = ?1)",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let total = total as u64 + size;
        if total > compose::ATTACHMENTS_MAX {
            let kept: bool = tx.query_row("SELECT EXISTS (SELECT 1 FROM outgoing_blob WHERE hash = ?1)", [&hash[..]], |r| r.get(0))?;
            if !kept {
                outgoing.remove(&hash);
            }
            return Err(StoreError::InvalidParams(format!(
                "the attachments would weigh {} MB: {} MB at most",
                total >> 20,
                compose::ATTACHMENTS_MAX >> 20
            )));
        }
        let filename = compose::clean_filename(filename);
        let mime_type = compose::clean_mime_type(mime_type.unwrap_or(""));
        tx.execute("INSERT OR IGNORE INTO outgoing_blob (hash, size) VALUES (?1, ?2)", params![&hash[..], size as i64])?;
        tx.execute(
            "INSERT INTO draft_attachment (draft_id, hash, filename, mime_type, position)
             VALUES (?1, ?2, ?3, ?4, (SELECT coalesce(max(position) + 1, 0) FROM draft_attachment WHERE draft_id = ?1))",
            params![id, &hash[..], filename, mime_type],
        )?;
        let attachment_id = tx.last_insert_rowid();
        tx.execute(
            "UPDATE draft SET revision = revision + 1, date_modified = unixepoch(), modified_by = ?2 WHERE id = ?1",
            params![id, source.as_str()],
        )?;
        tx.commit()?;
        let weight = compose::encoded_size(total) + body_len as u64;
        let warning = (weight > compose::ATTACHMENTS_WARN).then(|| {
            format!("this mail weighs {} MB: many servers refuse more than {} MB", weight >> 20, compose::ATTACHMENTS_WARN >> 20)
        });
        Ok((rev + 1, DraftAttachmentView { id: attachment_id, filename, mime_type, size: size as i64 }, warning))
    }

    /// Removes an attachment from a draft: a new revision. Its file stays
    /// until the garbage collector finds nobody needs it.
    pub fn remove_draft_attachment(&mut self, id: i64, revision: i64, attachment_id: i64, source: Source) -> Result<i64> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let rev = editable(&tx, id, revision)?;
        let n = tx.execute("DELETE FROM draft_attachment WHERE id = ?1 AND draft_id = ?2", params![attachment_id, id])?;
        if n == 0 {
            return Err(StoreError::NotFound(format!("attachment {attachment_id}")));
        }
        tx.execute(
            "UPDATE draft SET revision = revision + 1, date_modified = unixepoch(), modified_by = ?2 WHERE id = ?1",
            params![id, source.as_str()],
        )?;
        tx.commit()?;
        Ok(rev + 1)
    }

    /// Pretended sending (`NICEMAILER_FAKE_NETWORK`, set by the core): an
    /// archive account (an mbox import, no server) may answer too, so that
    /// the reply can be tried on a real mailbox without sending anything.
    /// The daemon then records the mail as accepted itself, without network.
    pub fn set_fake_sending(&mut self, on: bool) {
        self.fake_sending = on;
    }

    /// What `r` (or `R`) on the message `reply_to` opens, without writing
    /// anything: the reply already started to it (`true`: reused), or the
    /// reply the core proposes, a [`DraftView`] with `id` 0 and `revision`
    /// 0 that no row of `draft` holds yet (`reponse.md`, section 5).
    ///
    /// Why a proposal rather than a draft: opening the reply screen must
    /// not wait for the store's write lock, which a synchronization or an
    /// import takes batch after batch, for seconds in all while tens of
    /// thousands of mails come in. Nothing needs writing to show a reply:
    /// the row is written by [`Store::create_reply_draft`] when there is
    /// something to keep (the first pause of typing, a file attached, the
    /// sending). This runs on a connection of the read pool.
    ///
    /// The same rules as [`Store::create_reply_draft`] (one computation,
    /// [`reply_plan`]), so the draft written later is the one proposed.
    /// `fake`: the fake network ([`Store::set_fake_sending`]), a setting of
    /// the writing store that the readers do not carry.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_reply_draft(
        &self,
        reply_to: i64,
        sender_only: bool,
        sending: &Sending,
        quote_header: &str,
        body: Option<&str>,
        fake: bool,
    ) -> Result<(DraftView, bool)> {
        if let Some(id) = existing_reply(&self.conn, reply_to)? {
            return Ok((draft_view(&self.conn, id)?, true));
        }
        let plan = reply_plan(&self.conn, reply_to, sender_only, sending, quote_header, body, fake)?;
        Ok((plan.view(&self.conn, Vec::new())?, false))
    }

    /// Starts a reply to all to the message `reply_to`, or gives back the
    /// one already started (`reused`), so that `r` twice never makes two
    /// drafts (`reponse.md`, section 3). The check and the insertion are in
    /// one write transaction: two clients writing their first pause of the
    /// same reply at once get the same draft.
    ///
    /// `quote_header` is the line introducing the quotation, in the
    /// client's language; `body`, when given, replaces the quotation (a
    /// draft abandoned by mistake, restored; the text typed in a reply
    /// proposed by [`Store::prepare_reply_draft`], at its first save).
    ///
    /// Refused: a message of an archive account (`CannotSend`), a draft,
    /// a message whose body is not downloaded (`BodyPending`).
    ///
    /// `sender_only`: `R`, the reply to the sender alone
    /// (`compose::reply_sender`). A reply already started to the mail is
    /// given back whatever its kind: one reply per mail.
    ///
    /// `sending`: the accounts that may send it (`reponse.md`, section 4
    /// ter); when the mail's own account is not one of them, the draft
    /// keeps the mail's account (its conversation, its Drafts folder) and
    /// records the account that sends (`draft.send_account_id`).
    #[allow(clippy::too_many_arguments)]
    pub fn create_reply_draft(
        &mut self,
        reply_to: i64,
        sender_only: bool,
        sending: &Sending,
        quote_header: &str,
        body: Option<&str>,
        source: Source,
    ) -> Result<(DraftView, bool)> {
        let fake = self.fake_sending;
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        // The reply to this mail already started, or, `reply_to` being the
        // draft's own row of Drafts, that draft (`r` in the Drafts folder).
        if let Some(id) = existing_reply(&tx, reply_to)? {
            let view = draft_view(&tx, id)?;
            tx.commit()?;
            return Ok((view, true));
        }
        let plan = reply_plan(&tx, reply_to, sender_only, sending, quote_header, body, fake)?;
        let id = plan.insert(&tx, source)?;
        let view = draft_view(&tx, id)?;
        tx.commit()?;
        Ok((view, false))
    }

    /// What `f` on the message `message` opens, without writing anything:
    /// the forward already started, or the one proposed (`id` 0, as
    /// [`Store::prepare_reply_draft`]). Its files are listed from the
    /// database, each with a negative `id` (it is not a row of
    /// `draft_attachment` yet): they are copied into `outgoing.noindex`
    /// only when the forward is written ([`Store::create_forward_draft`]).
    ///
    /// Returns the draft, whether it existed, and the names of the files
    /// that will not be forwarded (the raw file of the mail is not here,
    /// or they go beyond `compose::ATTACHMENTS_MAX`).
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_forward_draft(
        &self,
        message: i64,
        sending: &Sending,
        header: &str,
        body: Option<&str>,
        raw: &crate::raw::RawStore,
        fake: bool,
    ) -> Result<(DraftView, bool, Vec<String>)> {
        if let Some(id) = existing_forward(&self.conn, message)? {
            return Ok((draft_view(&self.conn, id)?, true, Vec::new()));
        }
        let (plan, raw_hash) = forward_plan(&self.conn, message, sending, header, body, fake)?;
        // The raw file is only looked for (a `stat`), never read: listing
        // the files must stay as fast as the rest of the proposal.
        let here = raw_hash.is_some_and(|h| raw.path(&h).exists());
        let (mut shown, mut missing, mut total) = (Vec::new(), Vec::new(), 0u64);
        for (i, f) in forward_files(&self.conn, message)?.into_iter().enumerate() {
            let size = f.size.max(0) as u64;
            if !here || total + size > compose::ATTACHMENTS_MAX {
                missing.push(f.name);
                continue;
            }
            total += size;
            shown.push(DraftAttachmentView {
                id: -(i as i64 + 1),
                filename: compose::clean_filename(&f.name),
                mime_type: compose::clean_mime_type(&f.mime_type),
                size: size as i64,
            });
        }
        Ok((plan.view(&self.conn, shown)?, false, missing))
    }

    /// Starts a forward of the message `message` (`f`), or gives back the
    /// one already started: no recipient, the subject with `Fwd: `, the
    /// block `header` written by the client then the mail's text, and the
    /// mail's files (not its inline images) copied into `outgoing.noindex`
    /// (`brouillons.md`, section 7). A forward is not a reply: no
    /// `In-Reply-To`, nothing filed (`dossiers.md`).
    ///
    /// Returns the draft, whether it existed, and the names of the files
    /// that could not be taken (a mail without raw file): the text is
    /// forwarded all the same, and the user told.
    pub fn create_forward_draft(
        &mut self,
        message: i64,
        sending: &Sending,
        header: &str,
        body: Option<&str>,
        raw: &crate::raw::RawStore,
        source: Source,
    ) -> Result<(DraftView, bool, Vec<String>)> {
        let outgoing = self.outgoing()?.clone();
        let fake = self.fake_sending;
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(id) = existing_forward(&tx, message)? {
            let view = draft_view(&tx, id)?;
            tx.commit()?;
            return Ok((view, true, Vec::new()));
        }
        let (plan, raw_hash) = forward_plan(&tx, message, sending, header, body, fake)?;
        let id = plan.insert(&tx, source)?;
        // The files of the mail: read from its raw file, kept like files
        // the user attached.
        let files = forward_files(&tx, message)?;
        let mut missing = Vec::new();
        let raw_bytes = match raw_hash {
            Some(h) if !files.is_empty() => raw.read(&h).ok(),
            _ => None,
        };
        let mut total = 0u64;
        for (position, f) in files.into_iter().enumerate() {
            let content = raw_bytes.as_deref().and_then(|b| crate::attachments::part_content(b, &f.part_id));
            let Some(content) = content.filter(|c| total + c.len() as u64 <= compose::ATTACHMENTS_MAX) else {
                missing.push(f.name);
                continue;
            };
            total += content.len() as u64;
            let hash = outgoing.write(&content).map_err(|e| StoreError::Io(e.to_string()))?;
            tx.execute("INSERT OR IGNORE INTO outgoing_blob (hash, size) VALUES (?1, ?2)", params![&hash[..], content.len() as i64])?;
            tx.execute(
                "INSERT INTO draft_attachment (draft_id, hash, filename, mime_type, position) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id, &hash[..], compose::clean_filename(&f.name), compose::clean_mime_type(&f.mime_type), position as i64],
            )?;
        }
        let view = draft_view(&tx, id)?;
        tx.commit()?;
        Ok((view, false, missing))
    }

    /// What `c` opens, without writing anything: a new mail from the
    /// account's main address, proposed (`id` 0, as
    /// [`Store::prepare_reply_draft`]). `body`: a new mail abandoned by
    /// mistake, restored.
    pub fn prepare_new_draft(&self, account_id: i64, identities: &Identities, body: Option<&str>) -> Result<DraftView> {
        new_plan(account_id, identities, body)?.view(&self.conn, Vec::new())
    }

    /// Starts a new mail (`c`) from the account's main address: nothing
    /// else, the user writes it all. `body`: a new mail abandoned by
    /// mistake, restored, or the text typed in a new mail proposed by
    /// [`Store::prepare_new_draft`], at its first save.
    pub fn create_new_draft(&mut self, account_id: i64, identities: &Identities, body: Option<&str>, source: Source) -> Result<DraftView> {
        let plan = new_plan(account_id, identities, body)?;
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id = plan.insert(&tx, source)?;
        let view = draft_view(&tx, id)?;
        tx.commit()?;
        Ok(view)
    }

    /// A new mail left without anything in it (no recipient, subject, text
    /// nor file, never copied to Drafts) is deleted instead of being kept
    /// in Drafts: `c` then Escape leaves nothing behind. Returns whether
    /// it was.
    pub fn drop_if_blank(&mut self, id: i64) -> Result<bool> {
        // A formatted draft's body is HTML: `<p></p>` is blank too. Its
        // text is what counts, so the body is read and judged here.
        let row: Option<(String, String)> =
            self.conn.query_row("SELECT body, body_format FROM draft WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
        let Some((body, format)) = row else { return Ok(false) };
        let text = if format == "html" { crate::mail_html::to_text(&body).0 } else { body };
        if !text.trim().is_empty() {
            return Ok(false);
        }
        Ok(self.conn.execute(
            "DELETE FROM draft WHERE id = ?1 AND reply_to_message_id IS NULL AND forward_of_message_id IS NULL
                AND message_id IS NULL AND state = 'editing' AND recipients = '[]' AND trim(subject) = ''
                AND NOT EXISTS (SELECT 1 FROM draft_attachment WHERE draft_id = ?1)",
            [id],
        )? > 0)
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
        self.edit_draft(id, revision, &DraftEdit { body: Some(body.to_string()), ..DraftEdit::default() }, source)
    }

    /// Saves what the user changed: the body, the subject, the recipients
    /// (`reponse.md`, sections 5 and 6). One new revision for the whole
    /// change, none if nothing changed. The recipients are cleaned by
    /// `compose::clean_recipients` (each address once across To, Cc and
    /// Bcc); an address that is not one is refused (`InvalidParams`), and
    /// nothing is saved. Returns the revision.
    pub fn edit_draft(&mut self, id: i64, revision: i64, edit: &DraftEdit, source: Source) -> Result<i64> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let rev = editable(&tx, id, revision)?;
        type Row = (String, String, String, i64, String, Option<String>, Option<i64>, String);
        let (recipients, subject, body, account_id, from_addr, from_name, send_account, body_format): Row = tx.query_row(
            "SELECT recipients, subject, body, account_id, from_addr, from_name, send_account_id, body_format FROM draft WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?)),
        )?;
        let new_recipients = if edit.to.is_some() || edit.cc.is_some() || edit.bcc.is_some() {
            let (to, cc, bcc) = recipients_of(&recipients);
            let to = edit.to.as_ref().unwrap_or(&to);
            let cc = edit.cc.as_ref().unwrap_or(&cc);
            let bcc = edit.bcc.as_ref().unwrap_or(&bcc);
            let [to, cc, bcc] = compose::clean_recipients([to, cc, bcc])
                .map_err(|bad| StoreError::InvalidParams(format!("not an address: {bad}")))?;
            recipients_json(&to, &cc, &bcc)
        } else {
            recipients
        };
        let new_subject = edit.subject.as_deref().map(compose::clean_subject).unwrap_or(subject);
        let new_body = edit.body.as_deref().unwrap_or(&body);
        let new_format = edit.body_format.as_deref().unwrap_or(&body_format);
        check_body(new_format, new_body)?;
        // The sender: NULL in `send_account_id` when the draft's own
        // account sends, so that the column only says "another account".
        let (new_send, new_from) = match &edit.from {
            Some((a, m)) => ((*a != account_id).then_some(*a), m.clone()),
            None => (send_account, Mailbox { addr: from_addr, name: from_name }),
        };
        // Rust note: `params!` borrows each value; the `UPDATE` only
        // happens when something differs, so a save of what is already
        // there keeps the revision (and the draft is not "changed here").
        // `IS NOT` compares NULLs too (`<>` with a NULL is never true).
        let n = tx.execute(
            "UPDATE draft SET recipients = ?2, subject = ?3, body = ?4, revision = revision + 1,
                    date_modified = unixepoch(), modified_by = ?5,
                    send_account_id = ?6, from_addr = ?7, from_name = ?8, body_format = ?9
              WHERE id = ?1 AND (recipients <> ?2 OR subject <> ?3 OR body <> ?4
                                 OR send_account_id IS NOT ?6 OR from_addr <> ?7 OR from_name IS NOT ?8
                                 OR body_format <> ?9)",
            params![id, new_recipients, new_subject, new_body, source.as_str(), new_send, new_from.addr, new_from.name, new_format],
        )?;
        tx.commit()?;
        Ok(if n > 0 { rev + 1 } else { rev })
    }

    /// The addresses to suggest for `query` in a recipient line
    /// (`reponse.md`, section 6, "Suggestions"). An address matches when
    /// it starts with the text, or its domain does, or its name, or a
    /// word of its name. Banned senders, the addresses of `own` and
    /// those of the accounts (an archive account's too) are never
    /// suggested.
    ///
    /// Ranked by use, **the most recent first**: the addresses written to
    /// this week come before those of this month, then this quarter, this
    /// year, longer ago, and last those never written to; inside each of
    /// those, the one written to most often, then the one most heard
    /// from. Whom one writes to changes over the years, and the address
    /// of this morning's correspondent is wanted before that of a heavy
    /// exchange long over; buckets rather than a decaying score keep the
    /// order easy to predict, which a list answered with ↓ and Return
    /// must be, and hold until a mail crosses a bucket's edge.
    ///
    /// The candidates are found by a scan of `address`, at most
    /// `CANDIDATES`; their mails are then counted and dated through the
    /// indexes `idx_message_address_address` and `idx_message_from`.
    /// Measured on a generated mailbox of 285,000 mails: 61 ms for one
    /// letter (55 ms before the dates were read), much less for more.
    /// With one letter, a frequent correspondent beyond the first 2,000
    /// matches can be missed; a second letter finds them.
    pub fn suggest_recipients(&self, query: &str, limit: u32, own: &[String]) -> Result<Vec<AddressView>> {
        const CANDIDATES: i64 = 2000;
        let q = query.trim().replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
        if q.is_empty() {
            return Ok(Vec::new());
        }
        // The date of a mail is the one it carries, else when it arrived.
        let date = "max(coalesce(m.date_sent, m.date_received))";
        let sql = format!(
            "WITH cand AS (
                 SELECT id, addr, name FROM address
                  WHERE (addr LIKE ?1 ESCAPE '\\' OR addr LIKE ?2 ESCAPE '\\'
                         OR name LIKE ?1 ESCAPE '\\' OR name LIKE ?3 ESCAPE '\\')
                    AND (screening_target IS NULL OR screening_target <> 'banned')
                    AND addr NOT IN (SELECT address FROM account WHERE address IS NOT NULL)
                  LIMIT ?4),
             -- Mails written to each candidate: how many, and how old the
             -- latest is. Grouped once over the index rather than counted
             -- candidate by candidate.
             written AS (
                 SELECT ma.address_id AS id, count(*) AS n, {age} AS age
                   FROM message_address ma
                   JOIN message m ON m.id = ma.message_id
                   JOIN folder f ON f.id = m.folder_id
                  WHERE ma.address_id IN (SELECT id FROM cand)
                    AND ma.role IN ('to', 'cc', 'bcc') AND f.role = 'sent'
                  GROUP BY ma.address_id),
             -- Mails received from each candidate: how many. No date
             -- here, on purpose: counting reads the index alone
             -- (`idx_message_from`), while dating would read every one of
             -- those mails, and received mails are most of a mailbox (it
             -- doubled the time of a suggestion on 285,000 mails).
             got AS (
                 SELECT m.from_address_id AS id, count(*) AS n
                   FROM message m
                  WHERE m.from_address_id IN (SELECT id FROM cand)
                  GROUP BY m.from_address_id)
             SELECT c.addr, c.name FROM cand c
               LEFT JOIN written w ON w.id = c.id
               LEFT JOIN got g ON g.id = c.id
              ORDER BY coalesce(w.age, {never}), coalesce(w.n, 0) DESC,
                       coalesce(g.n, 0) DESC, c.addr
              LIMIT ?5",
            age = Self::AGE_BUCKET.replace("{date}", date),
            never = Self::NEVER,
        );
        let rows: Vec<(String, Option<String>)> = self
            .conn
            .prepare_cached(&sql)?
            .query_map(
                params![
                    format!("{q}%"),
                    format!("%@{q}%"),
                    format!("% {q}%"),
                    CANDIDATES,
                    i64::from(limit) + own.len() as i64,
                    now()
                ],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows
            .into_iter()
            .filter(|(addr, _)| compose::valid_addr(addr) && !own.iter().any(|o| o.eq_ignore_ascii_case(addr)))
            .take(limit as usize)
            .map(|(addr, name)| AddressView { addr: addr.to_lowercase(), name: name.filter(|n| !n.trim().is_empty()) })
            .collect())
    }

    /// How old the latest mail of a group is, in buckets rather than in
    /// seconds, so that the order of the suggestions holds for a while and
    /// a mail one day older does not change it (`suggest_recipients`).
    /// `{date}` is the expression giving that date, `?6` is now.
    const AGE_BUCKET: &'static str = "CASE WHEN {date} >= ?6 - 604800 THEN 0
                                           WHEN {date} >= ?6 - 2592000 THEN 1
                                           WHEN {date} >= ?6 - 7776000 THEN 2
                                           WHEN {date} >= ?6 - 31536000 THEN 3
                                           ELSE 4 END";
    /// The bucket of an address never written to (or never heard from).
    const NEVER: i64 = 5;

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
    ///
    /// `after_send`, the filing chosen by the user (`⇧⌘↩`), is saved with
    /// the draft: a send cancelled with `z` keeps it for the next `⌘↩`.
    /// `None` sends with the choice saved earlier, if any; without one, the
    /// preselection is worked out when the server accepts the mail
    /// (`send_accepted`), the conversation having maybe moved meanwhile.
    #[allow(clippy::too_many_arguments)]
    pub fn send_draft(
        &mut self,
        id: i64,
        revision: i64,
        body: &str,
        body_format: Option<&str>,
        after_send: Option<AfterSend>,
        delay_secs: u64,
        user_agent: &str,
        source: Source,
    ) -> Result<(i64, ChangeSet)> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let rev = editable(&tx, id, revision)?;
        save_body(&tx, id, rev, body, body_format, source)?;
        if let Some(choice) = after_send {
            tx.execute("UPDATE draft SET after_send = ?2 WHERE id = ?1", params![id, choice.as_str()])?;
        }
        let after_send: Option<String> = tx.query_row("SELECT after_send FROM draft WHERE id = ?1", [id], |r| r.get(0))?;
        let Built { account_id, from_addr, rcpt, mime, copy, msgid, date: t, attached } = build_draft(&tx, id, user_agent, None, self.outgoing.as_ref())?;
        if rcpt.is_empty() {
            return Err(StoreError::CannotSend(nm_api::reason::DRAFT_NO_RECIPIENT.into()));
        }
        let send_at = t + delay_secs as i64;
        let mut payload = json!({
            "draft_id": id,
            "phase": "smtp",
            "from": from_addr,
            "rcpt": rcpt,
            "message_id_hdr": msgid,
            "after_send": after_send,
            "data_sent": false,
        });
        if attached {
            // With files, the message can weigh megabytes: kept as files of
            // `outgoing.noindex`, named in the payload, so that the queries
            // reading the payloads of `pending_action` stay light.
            let o = self.outgoing.as_ref().ok_or_else(|| StoreError::Io("no store for the files to send".into()))?;
            let keep = |bytes: &[u8]| -> Result<String> {
                let h = o.write(bytes).map_err(|e| StoreError::Io(e.to_string()))?;
                tx.execute("INSERT OR IGNORE INTO outgoing_blob (hash, size) VALUES (?1, ?2)", params![&h[..], bytes.len() as i64])?;
                Ok(crate::raw::hex(&h))
            };
            payload["mime_blob"] = json!(keep(&mime)?);
            payload["copy_blob"] = json!(keep(&copy)?);
        } else {
            // The MIME message is ASCII (`build_message` encodes the rest).
            payload["mime"] = json!(String::from_utf8_lossy(&mime));
            // The copy kept by the user (raw store, Sent), with `Bcc`; only
            // when it differs from the mail sent.
            if copy != mime {
                payload["mime_copy"] = json!(String::from_utf8_lossy(&copy));
            }
        }
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
        changes.sending.insert(view.send_account_id);
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
        // Rust note: cloned first (two paths): `self.transaction()` borrows
        // the whole store mutably, so no field can be read while it lives.
        let outgoing = self.outgoing.clone();
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
            let messages = message_bytes(&p, outgoing.as_ref());
            let (Some(draft_id), Some(from), Some((mime, copy)), Some(msgid)) =
                (p["draft_id"].as_i64(), p["from"].as_str(), messages, p["message_id_hdr"].as_str())
            else {
                // Unreadable (or its files gone): never sent, never claimed
                // again; the draft is editable again.
                tx.execute("UPDATE pending_action SET state = 'reverted', last_error = 'unreadable payload' WHERE id = ?1", [id])?;
                if let Some(d) = p["draft_id"].as_i64() {
                    tx.execute("UPDATE draft SET state = 'editing', modified_by = 'core' WHERE id = ?1 AND state = 'sending'", [d])?;
                }
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
                mime,
                copy,
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
        // What the draft was written from: the mail answered (its
        // conversation is the one filed), the mail forwarded (nothing is
        // filed), or neither — a new mail, which files itself (section 8,
        // "Nouveau mail").
        let (reply_to, forward_of): (Option<i64>, Option<i64>) = tx
            .query_row("SELECT reply_to_message_id, forward_of_message_id FROM draft WHERE id = ?1", [draft_id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?
            .unwrap_or((None, None));
        // The copy kept, with its `Bcc`: the row of Sent lists the hidden
        // recipients, and they get their decision like the others.
        let mime = message_bytes(&p, self.outgoing.as_ref()).map(|(_, copy)| copy).unwrap_or_default();
        let parsed = crate::ingest::parse(&mime);
        let sent_folder: Option<i64> = tx
            .query_row("SELECT id FROM folder WHERE account_id = ?1 AND role = 'sent'", [account_id], |r| r.get(0))
            .optional()?;
        let mut inserter = Inserter::new(source, None);
        let mut threads = BTreeSet::new();
        let mut sent_id = None;
        let mut moved_to = None;
        // A new mail's own filing, waiting for its copy in Sent
        // ([`Store::send_copied`]).
        let mut filing = None;
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
            // It may be of another account than the one that sent: a reply
            // to a mail of an archive account, sent from an account with a
            // server (`reponse.md`, section 4 ter). That conversation is
            // the one filed, in its own account.
            let parent: Option<(Option<i64>, i64)> = match reply_to {
                Some(r) => tx.query_row("SELECT thread_id, account_id FROM message WHERE id = ?1", [r], |r| Ok((r.get(0)?, r.get(1)?))).optional()?,
                None => None,
            };
            let parent_thread = parent.and_then(|(t, a)| t.map(|t| (t, a)));
            // A reply without `In-Reply-To` (the mail answered had no
            // Message-ID) files nothing (`dossiers.md`).
            // The filing chosen wins over the preselection; `stay` touches
            // nothing, not even the return marks: the conversation is left
            // as the reply found it.
            let after = p["after_send"].as_str().and_then(AfterSend::parse);
            if let Some((t, thread_account)) = parent_thread.or(thread.map(|t| (t, account_id))).filter(|_| parsed.in_reply_to.is_some())
                && after != Some(AfterSend::Stay)
            {
                let target = after.and_then(AfterSend::folder);
                moved_to = crate::reception::file_reply_thread(&tx, thread_account, t, id, target, source, "reply", &mut changes, &mut threads)?;
            } else if reply_to.is_none()
                && forward_of.is_none()
                && parsed.in_reply_to.is_none()
                && let Some(target) = after.and_then(AfterSend::folder)
            {
                // A new mail has no conversation to file: what the user
                // filed is the mail they wrote, so its copy in Sent moves
                // (`reponse.md`, section 8, "Nouveau mail"). Not now: that
                // copy does not exist on the server yet, and its UID is
                // unknown, so the server could not follow the move. The
                // action carries the target into its `append` phase, which
                // [`Store::send_copied`] ends.
                filing = Some(target);
            }
            crate::reception::accept_recipients(&tx, own, id, source, &mut changes, &mut threads)?;
            tx.execute(
                "UPDATE pending_action SET message_id = ?2, date_modified = unixepoch(),
                        payload = json_set(payload, '$.phase', 'append')
                  WHERE id = ?1",
                params![action_id, id],
            )?;
            if let Some(target) = filing {
                tx.execute(
                    "UPDATE pending_action SET payload = json_set(payload, '$.file_after_copy', ?2) WHERE id = ?1",
                    params![action_id, target.as_str()],
                )?;
            }
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
        // The mail answered gets `\Answered`, here and on the server, as
        // Mail and Thunderbird set it: other mail clients show it, and the
        // "answered" attribute finds the thread through the flag too
        // (`ui.md`, "Sélecteur d'attributs"). Before (November 2026),
        // nothing marked it.
        if let Some(r) = reply_to {
            mark_answered(&tx, r, source, &mut changes, &mut threads)?;
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
            changes.sent.push(SentDraft { draft_id, message_id, thread_id, moved_to, filing, undo });
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
            let why = nm_api::reason::ATTEMPTS_FAILED.with("attempts", attempts).with("server_text", error);
            let reason = why.message();
            if append {
                self.conn.execute(
                    "UPDATE pending_action SET state = 'done', attempts = ?2, last_error = ?3, date_modified = unixepoch() WHERE id = ?1",
                    params![action_id, attempts, format!("sent, but not copied to Sent: {reason}")],
                )?;
                return Ok(ChangeSet::default());
            }
            return self.send_failed(action_id, why);
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
    /// draft editable again, `SendFailed` tells the user. `why` is kept in
    /// English in the database (`last_error`, the history).
    pub fn send_failed(&mut self, action_id: i64, why: nm_api::Why) -> Result<ChangeSet> {
        let reason = why.message();
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
            changes.send_failed.push((draft_id, reply_to, why));
        }
        tx.commit()?;
        Ok(changes)
    }

    /// The copy in Sent is made (or found): the row of the mail gets its
    /// UID when known, the filing a new mail chose is applied, and the
    /// action ends. Without a UID (a server without UIDPLUS, a provider
    /// that copies itself), the synchronization of Sent attaches the row
    /// by its `Message-ID`.
    ///
    /// The filing waits for this moment: the mail is in Sent on the
    /// server from now on, so the move the user chose can be told to it
    /// (`reponse.md`, section 8, "Nouveau mail"). Replayed after a crash,
    /// the call finds the mail already out of Sent and files nothing.
    pub fn send_copied(&mut self, action_id: i64, uid: Option<u32>) -> Result<ChangeSet> {
        let tx = self.transaction()?;
        let mut changes = ChangeSet::default();
        let row: Option<(Option<i64>, i64, String, String)> = tx
            .query_row("SELECT message_id, account_id, payload, created_by FROM pending_action WHERE id = ?1", [action_id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })
            .optional()?;
        let (message, account_id, payload, created_by) = match row {
            Some(r) => r,
            // The action is gone (an old database cleaned): nothing to end.
            None => return Ok(changes),
        };
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
        // The filing of a new mail, chosen with ⇧⌘↩ and carried by the
        // action since the server accepted the mail.
        let p: Value = serde_json::from_str(&payload).unwrap_or(Value::Null);
        if let Some(id) = message
            && let Some(target) = p["file_after_copy"].as_str().and_then(FolderRole::parse)
        {
            let source = Source::parse(&created_by).unwrap_or(Source::Core);
            let mut threads = BTreeSet::new();
            crate::reception::file_sent_mail(&tx, account_id, id, target, source, &mut changes, &mut threads)?;
            crate::screening::refresh_all(&tx, &threads, &mut changes)?;
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
    /// The account that sends: its SMTP server, its Sent folder.
    account_id: i64,
    from_addr: String,
    /// Every recipient, To, Cc and Bcc: the SMTP envelope.
    rcpt: Vec<String>,
    /// The mail sent: no `Bcc` header.
    mime: Vec<u8>,
    /// The copy kept by the user, with `Bcc` (equal to `mime` without
    /// hidden recipients).
    copy: Vec<u8>,
    msgid: String,
    date: i64,
    /// The draft has attachments.
    attached: bool,
}

/// The mail sent and the copy kept, from the payload of a `send` action:
/// inline, or files of `outgoing.noindex` for a mail with attachments.
fn message_bytes(p: &Value, outgoing: Option<&OutgoingStore>) -> Option<(Vec<u8>, Vec<u8>)> {
    if let (Some(m), Some(c)) = (p["mime_blob"].as_str(), p["copy_blob"].as_str()) {
        let o = outgoing?;
        return Some((o.read(&parse_hex(m)?).ok()?, o.read(&parse_hex(c)?).ok()?));
    }
    let mime = p["mime"].as_str()?;
    Some((mime.as_bytes().to_vec(), p["mime_copy"].as_str().unwrap_or(mime).as_bytes().to_vec()))
}

/// The MIME message of a draft as it is now, its conversation headers from
/// the mail it answers. `draft_revision`: for a copy in Drafts
/// (`X-NiceMailer-Draft-Revision`), `None` for the mail sent.
fn build_draft(conn: &Connection, id: i64, user_agent: &str, draft_revision: Option<i64>, outgoing: Option<&OutgoingStore>) -> Result<Built> {
    type Row = (i64, String, Option<String>, String, String, String, String, Option<i64>, String);
    let (account_id, from_addr, from_name, recipients, subject, body, msgid, reply_to, body_format): Row = conn
        .query_row(
            "SELECT coalesce(send_account_id, account_id), from_addr, from_name, recipients, subject, body, message_id_hdr,
                    reply_to_message_id, body_format
               FROM draft WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?)),
        )
        .optional()?
        .ok_or_else(|| StoreError::NotFound(format!("draft {id}")))?;
    let (to, cc, bcc) = recipients_of(&recipients);
    let rcpt: Vec<String> = to.iter().chain(&cc).chain(&bcc).map(|m| m.addr.clone()).collect();
    // The conversation headers, from the mail answered as it is now.
    let (in_reply_to, references) = match reply_to {
        Some(p) => parent_ids(conn, p)?,
        None => (None, Vec::new()),
    };
    let t = now();
    let from = Mailbox { addr: from_addr.clone(), name: from_name };
    let headers = compose::Outgoing {
        from: &from,
        to: &to,
        cc: &cc,
        bcc: &bcc,
        subject: &subject,
        date: t,
        tz_offset_secs: local_offset(t),
        message_id: &msgid,
        in_reply_to: in_reply_to.as_deref(),
        references: &references,
        user_agent,
        draft_revision,
    };
    // The files, read from `outgoing.noindex`: a file missing there is an
    // error, never a mail sent without it.
    let listed = draft_attachments(conn, id)?;
    let mut data = Vec::with_capacity(listed.len());
    if !listed.is_empty() {
        let o = outgoing.ok_or_else(|| StoreError::Io("no store for the files to send".into()))?;
        for (a, hash) in &listed {
            data.push(o.read(hash).map_err(|e| StoreError::Io(format!("{}: {e}", a.filename)))?);
        }
    }
    let files: Vec<compose::Attachment> = listed
        .iter()
        .zip(&data)
        .map(|((a, _), bytes)| compose::Attachment { filename: &a.filename, mime_type: &a.mime_type, data: bytes })
        .collect();
    // A formatted draft (`edition.md`, section 10): its HTML cleaned and
    // styled by the core, its text drawn from it. Without any formatting,
    // it leaves as text only, exactly as a text draft.
    let (text, html) = if body_format == "html" {
        let o = crate::mail_html::outgoing(&body);
        (o.text, o.formatted.then_some(o.html))
    } else {
        (body, None)
    };
    let copy = compose::build_message_with_html(&headers, &text, html.as_deref(), &files);
    let mime = if bcc.is_empty() {
        copy.clone()
    } else {
        compose::build_message_with_html(&compose::Outgoing { bcc: &[], ..headers }, &text, html.as_deref(), &files)
    };
    Ok(Built { account_id, from_addr, rcpt, mime, copy, msgid, date: t, attached: !files.is_empty() })
}

/// The MIME message of a draft as a copy for Drafts: with its `Bcc`, so
/// that the draft resumed on another device still has them, and its files.
pub(crate) fn build_draft_mime(
    conn: &Connection,
    id: i64,
    user_agent: &str,
    revision: Option<i64>,
    outgoing: Option<&OutgoingStore>,
) -> Result<Vec<u8>> {
    Ok(build_draft(conn, id, user_agent, revision, outgoing)?.copy)
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

/// A draft as the core computes it from the mail answered or forwarded,
/// before any row exists: what [`Store::prepare_reply_draft`] shows and
/// [`Store::create_reply_draft`] writes. One computation for both, so that
/// the draft written at the first pause is exactly the one proposed.
struct Plan {
    /// The account of the draft: the mail's (its conversation, its Drafts).
    account_id: i64,
    /// The account that sends it; `account_id` unless that one cannot.
    send_account: i64,
    sender: Mailbox,
    to: Vec<Mailbox>,
    cc: Vec<Mailbox>,
    subject: String,
    body: String,
    reply_to: Option<i64>,
    forward_of: Option<i64>,
}

impl Plan {
    /// Writes the row of `draft`, with a new `Message-ID`; returns its id.
    /// Inside the caller's transaction.
    fn insert(&self, tx: &Transaction, source: Source) -> Result<i64> {
        let message_id_hdr = compose::new_message_id(rand::random::<u128>(), &self.sender.addr);
        tx.execute(
            "INSERT INTO draft (account_id, from_addr, from_name, recipients, subject, body, body_format,
                                reply_to_message_id, forward_of_message_id, message_id_hdr, created_by, modified_by,
                                send_account_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'text', ?7, ?8, ?9, ?10, ?10, ?11)",
            params![
                self.account_id,
                self.sender.addr,
                self.sender.name,
                recipients_json(&self.to, &self.cc, &[]),
                self.subject,
                self.body,
                self.reply_to,
                self.forward_of,
                message_id_hdr,
                source.as_str(),
                (self.send_account != self.account_id).then_some(self.send_account)
            ],
        )?;
        Ok(tx.last_insert_rowid())
    }

    /// The draft proposed, as [`draft_view`] would show it once written:
    /// `id` 0 and `revision` 0 say that no row holds it yet.
    fn view(&self, conn: &Connection, attachments: Vec<DraftAttachmentView>) -> Result<DraftView> {
        // The conversation of the mail answered, as `draft_view` reads it
        // through `reply_to_message_id` (a forward files nothing: none).
        let thread_id: Option<i64> = match self.reply_to {
            Some(m) => conn.query_row("SELECT thread_id FROM message WHERE id = ?1", [m], |r| r.get(0)).optional()?.flatten(),
            None => None,
        };
        let after_send_target = match thread_id {
            Some(t) => crate::reception::reply_preselection(conn, self.account_id, t)?,
            None => None,
        };
        let kind = if self.reply_to.is_some() {
            "reply"
        } else if self.forward_of.is_some() {
            "forward"
        } else {
            "new"
        };
        Ok(DraftView {
            id: 0,
            account_id: self.account_id,
            send_account_id: self.send_account,
            from: view_of(&self.sender),
            to: self.to.iter().map(view_of).collect(),
            cc: self.cc.iter().map(view_of).collect(),
            bcc: Vec::new(),
            subject: self.subject.clone(),
            body: self.body.clone(),
            body_format: "text".into(),
            attachments,
            revision: 0,
            state: "editing".into(),
            kind: kind.into(),
            reply_to: self.reply_to,
            forward_of: self.forward_of,
            thread_id,
            after_send_target,
            after_send: None,
            send_at: None,
        })
    }
}

/// The reply already started to `reply_to`, or, `reply_to` being the
/// draft's own row of Drafts, that draft (`r` in the Drafts folder).
fn existing_reply(conn: &Connection, reply_to: i64) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT id FROM draft WHERE reply_to_message_id = ?1 OR message_id = ?1 ORDER BY message_id = ?1 DESC, id LIMIT 1",
            [reply_to],
            |r| r.get(0),
        )
        .optional()?)
}

/// The forward of `message` already started.
fn existing_forward(conn: &Connection, message: i64) -> Result<Option<i64>> {
    Ok(conn.query_row("SELECT id FROM draft WHERE forward_of_message_id = ?1 ORDER BY id LIMIT 1", [message], |r| r.get(0)).optional()?)
}

/// The mail a reply or a forward starts from, refused when it cannot be:
/// `(account_id, subject, from, raw file)`.
fn source_message(conn: &Connection, id: i64, body: Option<&str>, refusal: nm_api::reason::Reason) -> Result<(i64, String, Option<Mailbox>, Option<crate::raw::Hash>)> {
    type Row = (i64, bool, i64, String, Option<String>, Option<String>, Option<Vec<u8>>);
    let row: Option<Row> = conn
        .query_row(
            "SELECT account_id, is_draft, body_state, subject, from_addr, from_name, raw_hash FROM message WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
        )
        .optional()?;
    let Some((account_id, is_draft, body_state, subject, from_addr, from_name, raw_hash)) = row else {
        return Err(StoreError::NotFound(format!("message {id}")));
    };
    if is_draft {
        return Err(StoreError::CannotSend(refusal.into()));
    }
    // The text quoted is the mail's: without its body, nothing to quote.
    // A restored body needs none.
    if body.is_none() && body_state == 0 {
        return Err(StoreError::BodyPending);
    }
    let from = from_addr.map(|a| Mailbox { addr: a, name: from_name });
    let raw_hash = raw_hash.as_deref().and_then(|h| <crate::raw::Hash>::try_from(h).ok());
    Ok((account_id, subject, from, raw_hash))
}

/// The reply to `reply_to` (`reponse.md`, sections 3 and 4): sender,
/// recipients (`compose::reply_all` or `compose::reply_sender`), subject
/// and quotation. Only reads.
#[allow(clippy::too_many_arguments)]
fn reply_plan(
    conn: &Connection,
    reply_to: i64,
    sender_only: bool,
    sending: &Sending,
    quote_header: &str,
    body: Option<&str>,
    fake: bool,
) -> Result<Plan> {
    let (account_id, subject, from, _) = source_message(conn, reply_to, body, nm_api::reason::DRAFT_CANNOT_BE_ANSWERED)?;
    let (to, cc, rt) = message_addresses(conn, reply_to)?;
    let original = Original { from: from.as_ref(), reply_to: &rt, to: &to, cc: &cc };
    let (send_account, sender, own) = pick_sender(conn, sending, &original, fake)?;
    let (r_to, r_cc) = if sender_only { (compose::reply_sender(&original, &own), Vec::new()) } else { compose::reply_all(&original, &own) };
    let body = match body {
        Some(b) => b.to_string(),
        None => {
            let quoted = compose::quote(&message_text(conn, reply_to)?, compose::QUOTE_MAX_LINES);
            compose::initial_body(quote_header, &quoted)
        }
    };
    Ok(Plan {
        account_id,
        send_account,
        sender,
        to: r_to,
        cc: r_cc,
        subject: compose::reply_subject(&subject),
        body,
        reply_to: Some(reply_to),
        forward_of: None,
    })
}

/// The forward of `message`: no recipient, `Fwd: `, the block `header`
/// then the mail's text. Also returns the raw file of the mail, where its
/// files are. Only reads.
fn forward_plan(
    conn: &Connection,
    message: i64,
    sending: &Sending,
    header: &str,
    body: Option<&str>,
    fake: bool,
) -> Result<(Plan, Option<crate::raw::Hash>)> {
    let (account_id, subject, from, raw_hash) = source_message(conn, message, body, nm_api::reason::DRAFT_CANNOT_BE_FORWARDED)?;
    let (to, cc, rt) = message_addresses(conn, message)?;
    let original = Original { from: from.as_ref(), reply_to: &rt, to: &to, cc: &cc };
    let (send_account, sender, _) = pick_sender(conn, sending, &original, fake)?;
    let body = match body {
        Some(b) => b.to_string(),
        None => compose::forward_body(header, &message_text(conn, message)?),
    };
    let plan = Plan {
        account_id,
        send_account,
        sender,
        to: Vec::new(),
        cc: Vec::new(),
        subject: compose::forward_subject(&subject),
        body,
        reply_to: None,
        forward_of: Some(message),
    };
    Ok((plan, raw_hash))
}

/// A new mail from the account's main address.
fn new_plan(account_id: i64, identities: &Identities, body: Option<&str>) -> Result<Plan> {
    let Some(sender) = identities.list.first().filter(|m| !m.addr.is_empty()) else {
        return Err(StoreError::CannotSend(nm_api::reason::NO_SENDER_ADDRESS.into()));
    };
    Ok(Plan {
        account_id,
        send_account: account_id,
        sender: sender.clone(),
        to: Vec::new(),
        cc: Vec::new(),
        subject: String::new(),
        body: body.unwrap_or("").to_string(),
        reply_to: None,
        forward_of: None,
    })
}

/// A file a forward takes from the mail (not its inline images).
struct ForwardFile {
    part_id: String,
    /// The name it is sent under (`attachments::safe_name`).
    name: String,
    mime_type: String,
    /// Bytes, as recorded at reception (0 when unknown).
    size: i64,
}

fn forward_files(conn: &Connection, message: i64) -> Result<Vec<ForwardFile>> {
    let rows: Vec<(String, Option<String>, String, i64)> = conn
        .prepare_cached(
            "SELECT part_id, filename, mime_type, coalesce(size, 0) FROM attachment
              WHERE message_id = ?1 AND is_inline = 0 ORDER BY id",
        )?
        .query_map([message], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<std::result::Result<_, _>>()?;
    Ok(rows
        .into_iter()
        .map(|(part_id, filename, mime_type, size)| {
            let name = crate::attachments::safe_name(filename.as_deref(), &mime_type, &part_id);
            ForwardFile { part_id, name, mime_type, size }
        })
        .collect())
}

/// Who sends a reply or a forward of `original` (`compose::choose_sender`):
/// the account and the address, and the addresses never to answer (those
/// of the mail's account and of the sender). An archive account has no
/// server and never sends, except with the fake network
/// (`Store::set_fake_sending`).
fn pick_sender(tx: &Connection, sending: &Sending, original: &Original, fake: bool) -> Result<(i64, Mailbox, Vec<String>)> {
    let lists: Vec<&[Mailbox]> = sending.senders.iter().map(|s| s.identities.list.as_slice()).collect();
    let Some((i, from)) = compose::choose_sender(&lists, original) else {
        return Err(StoreError::CannotSend(nm_api::reason::NO_ACCOUNT_CAN_SEND.into()));
    };
    let sender = &sending.senders[i];
    let kind: Option<String> = tx.query_row("SELECT kind FROM account WHERE id = ?1", [sender.account_id], |r| r.get(0)).optional()?;
    if kind.as_deref() == Some("archive") && !fake {
        return Err(StoreError::CannotSend(nm_api::reason::ARCHIVE_CANNOT_SEND.into()));
    }
    let mut own = sending.own.clone();
    own.extend(sender.identities.own());
    Ok((sender.account_id, from, own))
}

/// The revision of an editable draft, checked against the client's.
fn editable(tx: &Transaction, id: i64, revision: i64) -> Result<i64> {
    let row: Option<(i64, String)> =
        tx.query_row("SELECT revision, state FROM draft WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
    let Some((current, state)) = row else { return Err(StoreError::NotFound(format!("draft {id}"))) };
    if state == "sending" {
        return Err(StoreError::DraftSending);
    }
    if current != revision {
        return Err(StoreError::RevisionConflict { current });
    }
    Ok(current)
}

/// Writes the body, and its format when given, a new revision if either
/// changed; returns the revision.
fn save_body(tx: &Transaction, id: i64, rev: i64, body: &str, body_format: Option<&str>, source: Source) -> Result<i64> {
    let current: String = tx.query_row("SELECT body_format FROM draft WHERE id = ?1", [id], |r| r.get(0))?;
    let format = body_format.unwrap_or(&current);
    check_body(format, body)?;
    let n = tx.execute(
        "UPDATE draft SET body = ?2, body_format = ?4, revision = revision + 1, date_modified = unixepoch(), modified_by = ?3
          WHERE id = ?1 AND (body <> ?2 OR body_format <> ?4)",
        params![id, body, source.as_str(), format],
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

/// Marks the mail `message_id` answered (`\Answered`) after a reply to it
/// left: locally, and on the server through the spool (a mail of an
/// account without a server, an archive, is marked locally only). Nothing
/// for a mail already marked, gone, or of a read-only account (nothing is
/// changed in one). Not in the history: the `send` row says the mail was
/// answered, and a row here would fall in the undo range of the filing,
/// which undoing must not touch.
fn mark_answered(
    tx: &rusqlite::Transaction,
    message_id: i64,
    source: Source,
    changes: &mut ChangeSet,
    threads: &mut BTreeSet<i64>,
) -> Result<()> {
    let row: Option<(Option<i64>, i64)> = tx
        .query_row(
            "SELECT thread_id, folder_id FROM message m
              WHERE id = ?1 AND is_answered = 0
                AND NOT EXISTS (SELECT 1 FROM temp.read_only_account r WHERE r.id = m.account_id)",
            [message_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((thread, folder)) = row else { return Ok(()) };
    tx.execute("UPDATE message SET is_answered = 1 WHERE id = ?1", [message_id])?;
    crate::spool::enqueue_flag(tx, message_id, "\\Answered", true, source.as_str(), changes)?;
    changes.messages.insert((message_id, thread));
    changes.folders.insert(folder);
    threads.extend(thread);
    Ok(())
}
