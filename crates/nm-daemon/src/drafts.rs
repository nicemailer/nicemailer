//! Replies and their sending, on the socket (`specifications/reponse.md`,
//! section 11). The rules are in `nm-core` (`compose.rs`) and `nm-store`
//! (`drafts.rs`); this module adds what only the core knows: the account
//! of `accounts.toml` the mail belongs to (its addresses, whether it has an
//! SMTP server), the undo delay, and waking the account's sender.
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
    CreateDraftParams, CreateDraftResult, DraftIdParams, DraftView, RpcError, SendDraftParams, SendDraftResult,
    UpdateDraftParams, UpdateDraftResult, error_code,
};
use nm_core::Source;
use nm_core::compose::Mailbox;
use nm_store::drafts::Identities;

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

/// Only a human's client sends, in v1.
fn human(source: Source) -> Result<(), RpcError> {
    if matches!(source, Source::App | Source::Cli | Source::Tui) {
        Ok(())
    } else {
        Err(RpcError::new(error_code::FORBIDDEN, "sending mail is reserved to the app, the TUI and the CLI"))
    }
}

/// `create_draft`: a reply to all to a message, or the one already started.
pub async fn create(core: &Arc<Core>, p: CreateDraftParams, source: Source) -> Result<CreateDraftResult, RpcError> {
    human(source)?;
    let reply_to = p.reply_to;
    let account: Option<i64> = with_reader(core, move |s| {
        Ok(s.conn().query_row("SELECT account_id FROM message WHERE id = ?1", [reply_to], |r| r.get(0)).ok())
    })
    .await?;
    let Some(account_id) = account else {
        return Err(RpcError::new(error_code::NOT_FOUND, format!("no message {reply_to}")));
    };
    let s = sendable(core, account_id).await?;
    let result = with_store(core, move |st| st.create_reply_draft(reply_to, &s.identities, &p.quote_header, p.body.as_deref(), source)).await;
    if let Err(e) = &result
        && e.code == error_code::BODY_PENDING
    {
        // The body the reply quotes: downloaded first.
        sync::prioritize(core, vec![(account_id, reply_to)]);
    }
    let (draft, reused) = result?;
    Ok(CreateDraftResult { draft, reused })
}

/// `open_draft`: the latest version of a draft. (v1: nothing to mark, the
/// draft is never sent to the server while open; `brouillons.md`.)
pub async fn open(core: &Arc<Core>, p: DraftIdParams) -> Result<DraftView, RpcError> {
    with_reader(core, move |s| s.get_draft(p.id)).await
}

pub async fn update(core: &Arc<Core>, p: UpdateDraftParams, source: Source) -> Result<UpdateDraftResult, RpcError> {
    human(source)?;
    let revision = with_store(core, move |s| s.update_draft(p.id, p.revision, &p.body, source)).await?;
    Ok(UpdateDraftResult { revision })
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
    let account: i64 = with_reader(core, move |s| s.get_draft(id)).await?.account_id;
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
                let Ok(written) = core.raw.write(&job.mime) else { continue };
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
