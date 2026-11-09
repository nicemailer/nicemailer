//! The sender of one account: the mails queued by `send_draft`, sent
//! through the account's SMTP server, then copied to its Sent folder
//! (`specifications/reponse.md`, sections 7 and 9; the database side is
//! `nm-store`, `drafts.rs`).
//!
//! # Its own thread
//!
//! The sender is a third thread of the account's worker, with its own
//! connections: an SMTP one for each mail, and an IMAP one for the copy in
//! Sent. It never waits for the synchronization, which may be in the
//! middle of downloading 200,000 bodies, and the synchronization never
//! waits for it.
//!
//! ```text
//! ┌─▶ claim the due mails ─▶ for each: SMTP ─▶ accepted ─▶ copy in Sent ─┐
//! └── sleep until the next one is due, or until woken (send_draft, stop) ◀┘
//! ```
//!
//! A mail is due once its undo delay has passed (`date_run_after`): the
//! sender sleeps until then, and is woken when a mail is queued or
//! cancelled, to look at its queue again.
//!
//! # Failures
//!
//! | What happened | What follows |
//! | --- | --- |
//! | The server accepted | the mail in Sent, the conversation filed (`send_accepted`), then the copy |
//! | Network, time limit, `4xx` | tried again later, 30 s doubled, 8 times (`send_retry`) |
//! | Login refused, `5xx`, too large, no recipient accepted, no password | the draft goes back to the user (`send_failed`, `SendFailed`) |
//! | Some recipients refused | sent to the others, `SendPartial` |
//! | The copy in Sent fails | tried again later; the mail is sent whatever happens |
//!
//! # Never twice
//!
//! Just before telling the server the message is complete, the sender
//! records it (`data_sent`). A mail whose sending was interrupted after
//! that point (connection lost while waiting for the answer, core stopped)
//! may have left: before sending it again, the sender looks for its
//! `Message-ID` in the Sent folder of the server. This covers the
//! providers that copy to Sent themselves; elsewhere, a crash between the
//! server's acceptance and NiceMailer's record can still send it twice
//! (`brouillons.md`, "Double envoi").

use std::time::Duration;

use nm_core::compose;
use nm_proto::imap::{ImapError, MailboxName, SearchKey, Security, ServerSpec, Session, Timeouts};
use nm_proto::smtp::{Client, SmtpError};
use nm_store::ChangeSet;
use nm_store::drafts::{SendJob, SendPhase};

use super::{AccountSpec, Control, SyncHost, read, unix_now, write};
use crate::config;
use crate::probe::imap_spec;
use crate::secrets::{self, Refusal, Secrets};

/// Longest sleep without a mail due: the sender looks at its queue at
/// least this often, in case a wake-up was missed.
const IDLE_CHECK: Duration = Duration::from_secs(600);
/// How long the copy made by the provider itself is looked for in Sent
/// before the row is left to the synchronization: 3 tries, 2 s apart.
const PROVIDER_COPY_TRIES: u32 = 3;
const PROVIDER_COPY_WAIT: Duration = Duration::from_secs(2);

/// What the sender needs.
pub(super) struct Sender<'a> {
    pub spec: &'a AccountSpec,
    pub host: &'a dyn SyncHost,
    pub secrets: &'a dyn Secrets,
    pub control: &'a Control,
}

/// How a step ended.
enum Step {
    /// Go on with the next step (the copy in Sent).
    Next,
    /// Finished (done, failed for good, or queued again).
    End,
}

/// The loop of the sender's thread.
pub(super) async fn run(sx: Sender<'_>) {
    let account_id = sx.spec.account_id;
    loop {
        if sx.control.stopped() {
            return;
        }
        let jobs = match write(sx.host, |st| Ok((st.send_claim(account_id)?, ChangeSet::default()))) {
            Ok(j) => j,
            Err(e) => {
                eprintln!("send {}: {e}", sx.spec.key);
                Vec::new()
            }
        };
        for job in jobs {
            // A job claimed when the core stops stays `running`: queued
            // again at the next start, and replayed safely.
            if sx.control.stopped() {
                return;
            }
            send_one(&sx, job).await;
        }
        let next = read(sx.host, |st| st.send_next_due(account_id)).ok().flatten();
        let wait = match next {
            // A quarter of a second more than the delay: the database
            // counts in whole seconds.
            Some(t) => Duration::from_secs((t - unix_now()).max(0) as u64) + Duration::from_millis(250),
            None => IDLE_CHECK,
        };
        tokio::select! {
            _ = tokio::time::sleep(wait.min(IDLE_CHECK)) => {}
            _ = sx.control.send.notified() => {}
        }
    }
}

async fn send_one(sx: &Sender<'_>, job: SendJob) {
    let step = match job.phase {
        SendPhase::Smtp => smtp(sx, &job).await,
        SendPhase::Append => Step::Next,
    };
    if let Step::Next = step {
        copy_to_sent(sx, &job).await;
    }
}

/// Phase `smtp`: the mail to the server.
async fn smtp(sx: &Sender<'_>, job: &SendJob) -> Step {
    let cfg = &sx.spec.config;
    let key = &sx.spec.key;
    if job.data_sent {
        // It may have left already ("Never twice").
        match find_in_sent(sx, &job.message_id_hdr).await {
            Ok(Some(_)) => {
                eprintln!("send {key}: {} found in Sent, not sent again", job.message_id_hdr);
                return accepted(sx, job, &[]);
            }
            Ok(None) => {}
            Err(e) => return retry(sx, job, &format!("checking Sent before sending again: {e}")),
        }
    }
    let Some(smtp) = cfg.smtp.as_ref() else {
        return failed(sx, job, &Refusal::NoSmtp.message(key, cfg));
    };
    let password = match secrets::smtp_password(sx.secrets, key, cfg, nm_store::auth::organizational_domain) {
        Ok(p) => p,
        Err(r @ Refusal::Unavailable(_)) => return retry(sx, job, &r.message(key, cfg)),
        Err(r) => return failed(sx, job, &r.message(key, cfg)),
    };
    let result = async {
        let mut c = Client::connect(&smtp_spec(smtp), Timeouts::default()).await?;
        c.login(&smtp.username, password.expose()).await?;
        let action = job.action_id;
        let sent = c
            .send(&job.from, &job.rcpt, &job.mime, || {
                if let Err(e) = write(sx.host, |st| Ok((st.send_data_started(action)?, ChangeSet::default()))) {
                    eprintln!("send {key}: {e}");
                }
            })
            .await?;
        c.quit().await;
        Ok::<_, SmtpError>(sent)
    }
    .await;
    match result {
        Ok(sent) => {
            let refused: Vec<String> = sent.refused.iter().map(|r| format!("{} ({} {})", r.addr, r.code, r.text)).collect();
            if !refused.is_empty() {
                eprintln!("send {key}: refused by the server: {}", refused.join(", "));
            }
            accepted(sx, job, &refused)
        }
        Err(e) if e.is_permanent() => failed(sx, job, &e.to_string()),
        Err(e) => retry(sx, job, &e.to_string()),
    }
}

/// The server has the mail: its raw file, then the database.
fn accepted(sx: &Sender<'_>, job: &SendJob, refused: &[String]) -> Step {
    let raw = sx.host.raw();
    // The copy the user keeps, with its `Bcc` (`SendJob::copy`).
    let written = match raw.write(&job.copy).and_then(|w| raw.barrier().map(|()| w)) {
        Ok(w) => w,
        Err(e) => {
            // The mail left; without its file, it is not recorded here.
            // Tried again (the next attempt finds it in Sent).
            eprintln!("send {}: the mail left but could not be stored: {e}", sx.spec.key);
            return retry(sx, job, &format!("storing the sent mail: {e}"));
        }
    };
    let own = own(&sx.spec.config);
    match sx.host.write(&mut |st| st.send_accepted(job.action_id, &written, refused, &own)) {
        Ok(()) => {
            // The filing of the conversation queued moves in the spool:
            // the work connection sends them now, leaving IDLE.
            sx.control.spool.store(true, std::sync::atomic::Ordering::SeqCst);
            sx.control.work.notify_one();
            Step::Next
        }
        Err(e) => {
            eprintln!("send {}: {e}", sx.spec.key);
            Step::End
        }
    }
}

fn retry(sx: &Sender<'_>, job: &SendJob, error: &str) -> Step {
    eprintln!("send {}: {error}; new attempt later", sx.spec.key);
    if let Err(e) = sx.host.write(&mut |st| st.send_retry(job.action_id, error)) {
        eprintln!("send {}: {e}", sx.spec.key);
    }
    Step::End
}

fn failed(sx: &Sender<'_>, job: &SendJob, reason: &str) -> Step {
    eprintln!("send {}: not sent: {reason}", sx.spec.key);
    if let Err(e) = sx.host.write(&mut |st| st.send_failed(job.action_id, reason)) {
        eprintln!("send {}: {e}", sx.spec.key);
    }
    Step::End
}

/// Phase `append`: the copy in the Sent folder of the server (`reponse.md`,
/// section 9), or, for a provider that makes it itself, its UID.
async fn copy_to_sent(sx: &Sender<'_>, job: &SendJob) {
    let key = &sx.spec.key;
    let account_id = sx.spec.account_id;
    let sent = match read(sx.host, |st| st.sent_folder(account_id)) {
        Ok(Some((_, path))) => path,
        Ok(None) => {
            let _ = sx.host.write(&mut |st| st.send_copied(job.action_id, None));
            return;
        }
        Err(e) => {
            eprintln!("send {key}: {e}");
            return;
        }
    };
    let result = async {
        let mut s = imap_session(sx).await?;
        let gmail = s.capabilities().has("X-GM-EXT-1");
        let smtp_host = sx.spec.config.smtp.as_ref().map(|s| s.host.as_str()).unwrap_or("");
        let mailbox = MailboxName::new(&sent);
        let uid = if compose::must_copy_to_sent(sx.spec.config.save_sent.rule(), smtp_host, gmail) {
            // Replayed (a crash after the APPEND), or found on the server
            // before sending again: the copy may be there already.
            let replay = job.phase == SendPhase::Append || job.data_sent;
            let found = if replay { search(&mut s, &mailbox, &job.message_id_hdr).await? } else { None };
            match found {
                Some(uid) => Some(uid),
                None => match s.append(&mailbox, &["\\Seen"], Some(unix_now()), job.copy.clone()).await {
                    Ok(a) => a.map(|a| a.uid),
                    // Refused (no such folder, quota): the mail is sent,
                    // and stays without copy on the server.
                    Err(e @ (ImapError::No { .. } | ImapError::Bad { .. })) => {
                        eprintln!("send {key}: the copy in Sent was refused: {e}");
                        None
                    }
                    Err(e) => return Err(e),
                },
            }
        } else {
            // The provider makes the copy: its UID, once it shows.
            let mut uid = None;
            for i in 0..PROVIDER_COPY_TRIES {
                if i > 0 {
                    tokio::time::sleep(PROVIDER_COPY_WAIT).await;
                }
                uid = search(&mut s, &mailbox, &job.message_id_hdr).await?;
                if uid.is_some() {
                    break;
                }
            }
            uid
        };
        s.logout().await;
        Ok::<_, ImapError>(uid)
    }
    .await;
    match result {
        Ok(uid) => {
            if let Err(e) = sx.host.write(&mut |st| st.send_copied(job.action_id, uid)) {
                eprintln!("send {key}: {e}");
            }
        }
        Err(e) => {
            let _ = retry(sx, job, &format!("copy in Sent: {e}"));
        }
    }
}

/// The UID of the mail of this `Message-ID` in the Sent folder of the
/// server, if it is there.
async fn find_in_sent(sx: &Sender<'_>, message_id: &str) -> Result<Option<u32>, ImapError> {
    let account_id = sx.spec.account_id;
    let Ok(Some((_, path))) = read(sx.host, |st| st.sent_folder(account_id)) else { return Ok(None) };
    let mut s = imap_session(sx).await?;
    let uid = search(&mut s, &MailboxName::new(&path), message_id).await?;
    s.logout().await;
    Ok(uid)
}

async fn search(s: &mut Session, mailbox: &MailboxName, message_id: &str) -> Result<Option<u32>, ImapError> {
    s.examine(mailbox, None).await?;
    let uids = s.uid_search(&SearchKey::Header("Message-ID".into(), format!("<{message_id}>"))).await?;
    Ok(uids.into_iter().max())
}

/// An IMAP session of the account, logged in.
async fn imap_session(sx: &Sender<'_>) -> Result<Session, ImapError> {
    let cfg = &sx.spec.config;
    let password = secrets::imap_password(sx.secrets, &sx.spec.key, cfg).map_err(|r| ImapError::AuthFailed(r.message(&sx.spec.key, cfg)))?;
    let mut s = Session::connect(&imap_spec(cfg), Timeouts::default()).await?;
    s.login(&cfg.imap.username, password.expose()).await?;
    Ok(s)
}

fn smtp_spec(smtp: &config::ServerConfig) -> ServerSpec {
    ServerSpec {
        host: smtp.host.clone(),
        port: smtp.port,
        security: match smtp.security {
            config::Security::Tls => Security::Tls,
            config::Security::Starttls => Security::Starttls,
            config::Security::None => Security::None,
        },
        ca_file: smtp.ca_file.clone(),
    }
}

/// The account's own addresses, lowercase.
pub(crate) fn own(cfg: &config::AccountConfig) -> Vec<String> {
    std::iter::once(cfg.email.to_lowercase()).chain(cfg.identities.iter().map(|i| i.email.to_lowercase())).collect()
}
