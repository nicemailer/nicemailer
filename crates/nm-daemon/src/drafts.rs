//! Replies and their sending, on the socket (`specifications/reponse.md`,
//! section 11). The rules are in `nm-core` (`compose.rs`) and `nm-store`
//! (`drafts.rs`); this module adds what only the core knows: the account
//! of `accounts.toml` the mail belongs to (its addresses, whether it has an
//! SMTP server), the undo delay, and waking the account's sender.
//!
//! # Sending from another account
//!
//! Any draft can be sent from any account that can send (`list_senders`,
//! `update_draft` with `from`; `reponse.md`, section 4 ter). A reply to, or
//! a forward of, a mail of an account that cannot send (an archive account,
//! an IMAP account without `[smtp]` or read-only) starts from the first
//! account that can, by preference one the mail was sent to
//! (`compose::choose_sender`). The draft stays in the mail's account (its
//! conversation, its Drafts folder); the account that sends queues the
//! mail, sends it with its SMTP server and keeps it in its Sent folder.
//!
//! Sending is refused to agents (MCP) and to the web server in v1: a mail
//! an agent asks to send must be confirmed in the app (`architecture.md`,
//! section 9), which does not exist yet. The app, the TUI and the CLI may.
//!
//! # The UI tests
//!
//! With `NICEMAILER_FAKE_NETWORK` (the UI scenarios, `scripts/ui-test.sh`,
//! or trying the reply on a real mailbox), an account the file does not
//! declare (the fake mail of `make seed`, an archive account imported from
//! mbox) replies from its own address, and its mails "leave" without any network:
//! once the undo delay is over, the core records them as accepted, exactly
//! as after a real SMTP server (`fake_delivery`). Nothing ever leaves the
//! machine.

use std::sync::Arc;

use nm_api::{
    AddDraftAttachmentParams, AddDraftAttachmentResult, AddressView, CreateDraftParams, CreateDraftResult, DraftIdParams, DraftKind, DraftView,
    ListSendersResult, ParseAddressesParams, ParseAddressesResult, RemoveDraftAttachmentParams, RpcError, SendDraftParams, SendDraftResult,
    SenderView, SuggestRecipientsParams, SuggestRecipientsResult, UpdateDraftParams, UpdateDraftResult, error_code,
};
use nm_core::Source;
use nm_core::compose::Mailbox;
use nm_store::drafts::{DraftEdit, Identities, Sender, Sending};

use crate::{Core, sync, with_reader, with_store};

/// What the core knows of the account of a mail, from `accounts.toml`.
struct Sendable {
    identities: Identities,
    /// A fake account of the UI tests (see the header of this file).
    fake: bool,
}

pub fn fake_network() -> bool {
    std::env::var_os("NICEMAILER_FAKE_NETWORK").is_some()
}

/// The account of `account_id` as `accounts.toml` declares it, if it can
/// send: an IMAP account of the file, with `[smtp]`, not read-only.
async fn sendable(core: &Arc<Core>, account_id: i64) -> Result<Sendable, RpcError> {
    if fake_network() {
        let declared = {
            let engine = core.engine.lock().unwrap();
            engine.config().accounts.keys().any(|key| engine.account_id(key) == Some(account_id))
        };
        if !declared {
            let addr: Option<String> = with_reader(core, move |s| {
                Ok(s.conn()
                    .query_row("SELECT address FROM account WHERE id = ?1", [account_id], |r| r.get(0))
                    .ok()
                    .flatten())
            })
            .await?;
            if let Some(addr) = addr {
                return Ok(Sendable { identities: Identities { list: vec![Mailbox::new(&addr, None)] }, fake: true });
            }
        }
    }
    let engine = core.engine.lock().unwrap();
    let found = engine.config().accounts.iter().find(|(key, _)| engine.account_id(key) == Some(account_id));
    let Some((_, cfg)) = found else {
        return Err(RpcError::new(error_code::CANNOT_SEND, "this mail is not in an account of accounts.toml (an archive account?): it cannot be answered from here"));
    };
    if cfg.read_only {
        return Err(RpcError::new(error_code::CANNOT_SEND, "this account is read-only (read_only = true in accounts.toml): it cannot send"));
    }
    if cfg.smtp.is_none() {
        return Err(RpcError::new(error_code::CANNOT_SEND, "this account has no [smtp] server in accounts.toml: it cannot send"));
    }
    let main_name = cfg.display_name.as_deref();
    let list = std::iter::once(Mailbox::new(&cfg.email, main_name))
        .chain(cfg.identities.iter().map(|i| Mailbox::new(&i.email, i.name.as_deref().or(main_name))))
        .collect();
    Ok(Sendable { identities: Identities { list }, fake: false })
}

/// An account that can send, for `list_senders` and the choice of the
/// account that sends a reply.
struct Candidate {
    account_id: i64,
    key: String,
    sendable: Sendable,
}

/// Every account that can send, in display order (`account.position`):
/// those of `accounts.toml` with `[smtp]`, not read-only; with the fake
/// network, the accounts the file does not declare too (the fake mail of
/// `seed` is such an account, disabled since the file lacks it). Outside
/// the fake network a disabled account (its key left the file) never
/// sends: `sendable` does not find it in the file.
async fn senders(core: &Arc<Core>) -> Result<Vec<Candidate>, RpcError> {
    let rows: Vec<(i64, String)> = with_reader(core, |s| {
        Ok(s
            .conn()
            .prepare("SELECT id, config_key FROM account ORDER BY position, id")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?)
    })
    .await?;
    let mut out = Vec::new();
    for (account_id, key) in rows {
        if let Ok(sendable) = sendable(core, account_id).await {
            out.push(Candidate { account_id, key, sendable });
        }
    }
    Ok(out)
}

/// The addresses of an account, lowercase: its address and identities in
/// `accounts.toml`, else its address in the database (an archive account).
/// Never recipients of a reply to one of its mails.
async fn own_addresses(core: &Arc<Core>, account_id: i64) -> Result<Vec<String>, RpcError> {
    let declared: Option<Vec<String>> = {
        let engine = core.engine.lock().unwrap();
        engine.config().accounts.iter().find(|(key, _)| engine.account_id(key) == Some(account_id)).map(|(_, a)| {
            std::iter::once(a.email.to_lowercase()).chain(a.identities.iter().map(|i| i.email.to_lowercase())).collect()
        })
    };
    if let Some(own) = declared {
        return Ok(own);
    }
    let addr: Option<String> = with_reader(core, move |s| {
        Ok(s.conn().query_row("SELECT address FROM account WHERE id = ?1", [account_id], |r| r.get(0)).ok().flatten())
    })
    .await?;
    Ok(addr.map(|a| vec![a.to_lowercase()]).unwrap_or_default())
}

/// Who may send a reply to, or a forward of, a mail of `account_id`: that
/// account when it can; else every account that can (`reponse.md`,
/// section 4 ter). None: the reason the mail's account cannot, and that no
/// other account can either.
async fn sending_for(core: &Arc<Core>, account_id: i64) -> Result<Sending, RpcError> {
    let refused = match sendable(core, account_id).await {
        Ok(s) => return Ok(Sending::own_account(account_id, s.identities)),
        Err(e) if e.code == error_code::CANNOT_SEND => e,
        Err(e) => return Err(e),
    };
    let senders: Vec<Sender> =
        senders(core).await?.into_iter().map(|c| Sender { account_id: c.account_id, identities: c.sendable.identities }).collect();
    if senders.is_empty() {
        return Err(RpcError::new(
            error_code::CANNOT_SEND,
            format!("{}, and no other account can: add an [smtp] server to one in accounts.toml", refused.message),
        ));
    }
    Ok(Sending { own: own_addresses(core, account_id).await?, senders })
}

/// `list_senders`: the addresses a draft can be sent from, account by
/// account, main address first.
pub async fn list_senders(core: &Arc<Core>) -> Result<ListSendersResult, RpcError> {
    let items = senders(core)
        .await?
        .into_iter()
        .flat_map(|c| {
            let (account_id, key) = (c.account_id, c.key);
            c.sendable.identities.list.into_iter().map(move |m| SenderView { account_id, account: key.clone(), addr: m.addr, name: m.name })
        })
        .collect();
    Ok(ListSendersResult { items })
}

/// Only a human's client sends, in v1.
fn human(source: Source) -> Result<(), RpcError> {
    if matches!(source, Source::App | Source::Cli | Source::Tui) {
        Ok(())
    } else {
        Err(RpcError::new(error_code::FORBIDDEN, "sending mail is reserved to the app, the TUI and the CLI"))
    }
}

/// `create_draft`: a reply (to all, `r`, or to the sender, `R`), a forward
/// (`f`) or a new mail (`c`); for a reply or a forward, the one already
/// started to the same mail if any.
pub async fn create(core: &Arc<Core>, p: CreateDraftParams, source: Source) -> Result<CreateDraftResult, RpcError> {
    human(source)?;
    if p.kind == DraftKind::New {
        // The account of the view, if it can send; else the first that can.
        let chosen = match p.account {
            Some(a) => sendable(core, a).await.ok().map(|s| (a, s)),
            None => None,
        };
        let (account_id, s) = match chosen {
            Some(c) => c,
            None => default_account(core).await?,
        };
        let draft = with_store(core, move |st| st.create_new_draft(account_id, &s.identities, p.body.as_deref(), source)).await?;
        return Ok(CreateDraftResult { draft, reused: false, warning: None });
    }
    let Some(reply_to) = p.reply_to else {
        return Err(RpcError::new(error_code::INVALID_PARAMS, "reply_to: the mail answered or forwarded"));
    };
    let account: Option<i64> = with_reader(core, move |s| {
        Ok(s.conn().query_row("SELECT account_id FROM message WHERE id = ?1", [reply_to], |r| r.get(0)).ok())
    })
    .await?;
    let Some(account_id) = account else {
        return Err(RpcError::new(error_code::NOT_FOUND, format!("no message {reply_to}")));
    };
    let sending = sending_for(core, account_id).await?;
    let raw = core.raw.clone();
    let result = with_store(core, move |st| match p.kind {
        DraftKind::Forward => st.create_forward_draft(reply_to, &sending, &p.quote_header, p.body.as_deref(), &raw, source),
        kind => st
            .create_reply_draft(reply_to, kind == DraftKind::ReplySender, &sending, &p.quote_header, p.body.as_deref(), source)
            .map(|(d, reused)| (d, reused, Vec::new())),
    })
    .await;
    if let Err(e) = &result
        && e.code == error_code::BODY_PENDING
    {
        // The body the draft quotes: downloaded first.
        sync::prioritize(core, vec![(account_id, reply_to)]);
    }
    let (draft, reused, missing) = result?;
    let warning = (!missing.is_empty()).then(|| format!("not forwarded (not downloaded): {}", missing.join(", ")));
    Ok(CreateDraftResult { draft, reused, warning })
}

/// The account a new mail is sent from when the client names none, or
/// one that cannot send: the first account that can, in display order
/// (with the fake network, of the UI tests, any account of the database).
async fn default_account(core: &Arc<Core>) -> Result<(i64, Sendable), RpcError> {
    match senders(core).await?.into_iter().next() {
        Some(c) => Ok((c.account_id, c.sendable)),
        None => Err(RpcError::new(error_code::CANNOT_SEND, "no account of accounts.toml can send: add an [smtp] server to one")),
    }
}

/// `open_draft`: the latest version of a draft. (v1: nothing to mark, the
/// draft is never sent to the server while open; `brouillons.md`.)
pub async fn open(core: &Arc<Core>, p: DraftIdParams) -> Result<DraftView, RpcError> {
    with_reader(core, move |s| s.get_draft(p.id)).await
}

/// `update_draft`: what changed since the last save, the body at each pause,
/// the recipients and the subject when edited.
pub async fn update(core: &Arc<Core>, p: UpdateDraftParams, source: Source) -> Result<UpdateDraftResult, RpcError> {
    human(source)?;
    let list = |l: Option<Vec<AddressView>>| l.map(|l| l.into_iter().map(|a| Mailbox { addr: a.addr, name: a.name }).collect());
    // The sender: an address of an account that can send, now; its name
    // is the one `accounts.toml` gives it.
    let from = match p.from {
        Some(f) => {
            let s = sendable(core, f.account_id).await?;
            let Some(m) = s.identities.list.into_iter().find(|m| m.addr.eq_ignore_ascii_case(f.addr.trim())) else {
                return Err(RpcError::new(error_code::INVALID_PARAMS, format!("{} is not an address of this account", f.addr)));
            };
            Some((f.account_id, m))
        }
        None => None,
    };
    let edit = DraftEdit { body: p.body, subject: p.subject, to: list(p.to), cc: list(p.cc), bcc: list(p.bcc), from };
    let revision = with_store(core, move |s| s.edit_draft(p.id, p.revision, &edit, source)).await?;
    Ok(UpdateDraftResult { revision })
}

/// `add_draft_attachment`: the file the app put in `incoming.noindex`,
/// taken by the core (`nm_store::outgoing`).
pub async fn add_attachment(core: &Arc<Core>, p: AddDraftAttachmentParams, source: Source) -> Result<AddDraftAttachmentResult, RpcError> {
    human(source)?;
    let (revision, attachment, warning) = with_store(core, move |s| {
        s.add_draft_attachment(p.id, p.revision, &p.staged, &p.filename, p.mime_type.as_deref(), source)
    })
    .await?;
    Ok(AddDraftAttachmentResult { revision, attachment, warning })
}

pub async fn remove_attachment(core: &Arc<Core>, p: RemoveDraftAttachmentParams, source: Source) -> Result<UpdateDraftResult, RpcError> {
    human(source)?;
    let revision = with_store(core, move |s| s.remove_draft_attachment(p.id, p.revision, p.attachment_id, source)).await?;
    Ok(UpdateDraftResult { revision })
}

/// `parse_addresses`: a pure function of the text (`compose::parse_address_list`),
/// on the socket so that the app does not repeat the rule.
pub fn parse_addresses(p: ParseAddressesParams) -> ParseAddressesResult {
    let (addresses, invalid) = nm_core::compose::parse_address_list(&p.text);
    ParseAddressesResult { addresses: addresses.into_iter().map(|m| AddressView { addr: m.addr, name: m.name }).collect(), invalid }
}

/// `suggest_recipients`: the addresses of every account of `accounts.toml`
/// are left out (one does not write to oneself).
pub async fn suggest(core: &Arc<Core>, p: SuggestRecipientsParams) -> Result<SuggestRecipientsResult, RpcError> {
    let own: Vec<String> = {
        let engine = core.engine.lock().unwrap();
        engine
            .config()
            .accounts
            .values()
            .flat_map(|a| std::iter::once(a.email.clone()).chain(a.identities.iter().map(|i| i.email.clone())))
            .map(|a| a.to_lowercase())
            .collect()
    };
    let limit = p.limit.clamp(1, 50);
    let items = with_reader(core, move |s| s.suggest_recipients(&p.query, limit, &own)).await?;
    Ok(SuggestRecipientsResult { items })
}

pub async fn discard(core: &Arc<Core>, p: DraftIdParams, source: Source) -> Result<DraftView, RpcError> {
    human(source)?;
    let core2 = core.clone();
    with_store(core, move |s| {
        let (view, changes) = s.discard_draft(p.id, source)?;
        core2.events.lock().unwrap().emit(&changes);
        sync::wake(&core2, &changes);
        Ok(view)
    })
    .await
}

/// `close_draft`: the reply was left (Escape). If it changed since its last
/// copy, it becomes the row of Drafts, and goes to the server's Drafts
/// (`nm-store`, `draft_copies.rs`).
pub async fn close(core: &Arc<Core>, p: DraftIdParams, source: Source) -> Result<(), RpcError> {
    human(source)?;
    let core2 = core.clone();
    with_store(core, move |s| close_now(&core2, s, p.id, source)).await
}

/// Closes a draft with the store's lock held: its MIME message in the raw
/// store, then the row of Drafts and the copy queued. Also used at start
/// for the drafts left open by a stop or a crash.
pub fn close_now(core: &Core, s: &mut nm_store::Store, id: i64, source: Source) -> Result<(), nm_store::StoreError> {
    // A new mail left empty is not worth a place in Drafts.
    if s.drop_if_blank(id)? {
        return Ok(());
    }
    let Some(mime) = s.draft_to_close(id, USER_AGENT)? else { return Ok(()) };
    let written = core.raw.write(&mime).and_then(|w| core.raw.barrier().map(|()| w)).map_err(|e| nm_store::StoreError::Io(e.to_string()))?;
    let changes = s.close_draft(id, &written, &mime, source)?;
    core.events.lock().unwrap().emit(&changes);
    sync::wake(core, &changes);
    Ok(())
}

/// The `User-Agent` of the mails and drafts written by NiceMailer.
pub const USER_AGENT: &str = concat!("NiceMailer/", env!("CARGO_PKG_VERSION"));

/// `send_draft`: queued, due after `compose.undo_send_seconds`; the
/// account's sender is woken to sleep until then.
pub async fn send(core: &Arc<Core>, p: SendDraftParams, source: Source) -> Result<SendDraftResult, RpcError> {
    human(source)?;
    let id = p.id;
    // The account that sends, which may not be the draft's own.
    let account: i64 = with_reader(core, move |s| s.get_draft(id)).await?.send_account_id;
    // Checked again now: the file may have changed since the draft began.
    let s = sendable(core, account).await?;
    let delay = core.engine.lock().unwrap().config().compose.undo_send_seconds;
    let ua = USER_AGENT;
    let core2 = core.clone();
    let send_at = with_store(core, move |s| {
        let (at, changes) = s.send_draft(p.id, p.revision, &p.body, p.after_send, delay, ua, source)?;
        core2.events.lock().unwrap().emit(&changes);
        sync::wake(&core2, &changes);
        Ok(at)
    })
    .await?;
    if s.fake {
        fake_delivery(core.clone(), account, s.identities.own(), send_at);
    }
    Ok(SendDraftResult { send_at })
}

/// The UI tests: once due, the mails of a fake account are recorded as
/// accepted by a server that does not exist, and copied to nowhere.
fn fake_delivery(core: Arc<Core>, account_id: i64, own: Vec<String>, send_at: i64) {
    tokio::spawn(async move {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        tokio::time::sleep(std::time::Duration::from_millis(((send_at - now).max(0) * 1000 + 300) as u64)).await;
        let _ = tokio::task::spawn_blocking(move || {
            let mut store = core.store.lock().unwrap();
            let Ok(jobs) = store.send_claim(account_id) else { return };
            for job in jobs {
                let Ok(written) = core.raw.write(&job.copy) else { continue };
                if let Ok(changes) = store.send_accepted(job.action_id, &written, &[], &own) {
                    core.events.lock().unwrap().emit(&changes);
                }
                if let Ok(changes) = store.send_copied(job.action_id, None) {
                    core.events.lock().unwrap().emit(&changes);
                }
            }
        })
        .await;
    });
}

/// `cancel_send`: the draft editable again, if the mail has not started
/// leaving.
pub async fn cancel(core: &Arc<Core>, p: DraftIdParams, source: Source) -> Result<DraftView, RpcError> {
    human(source)?;
    let core2 = core.clone();
    with_store(core, move |s| {
        let (view, changes) = s.cancel_send(p.id, source)?;
        core2.events.lock().unwrap().emit(&changes);
        sync::wake(&core2, &changes);
        Ok(view)
    })
    .await
}
