//! The Drafts folder of the server: the copies of the drafts written in
//! NiceMailer (`brouillons.md`, sections 5 and 6; the database side is
//! `nm-store`, `draft_copies.rs`).
//!
//! Runs on the work connection with the rest of the spool. For a new
//! version of a draft:
//!
//! 1. the draft may have been sent or abandoned since: its copies are
//!    removed instead;
//! 2. the last copy NiceMailer wrote (`previous_uid`) is looked for: gone,
//!    the draft was deleted or sent elsewhere. Found in Sent: sent, the
//!    draft goes. Otherwise its text, changed here, is written again as a
//!    new copy (no text is ever lost);
//! 3. `APPEND` of the new version, `\Draft \Seen`; its UID from
//!    `APPENDUID`, or by its `Message-ID`;
//! 4. every other copy of the same `Message-ID` is removed: the previous
//!    version, and one a crash between 3 and 4 left. Replayed, the action
//!    leaves exactly one copy.
//!
//! A draft sent or abandoned: every copy of its `Message-ID` is removed.

use nm_proto::imap::{ImapError, MailboxName, SearchKey, Session, StoreOp, UidSet, Uids};
use nm_store::ChangeSet;
use nm_store::draft_copies::DraftWork;

use super::pass::PassError;
use super::spool::SpoolReport;
use super::{SyncHost, read, unix_now, write};

/// How a piece of work ended.
enum Ended {
    Done,
    /// The server refused for good (no such folder…): given up, the draft
    /// stays here.
    Refused(String),
}

/// Runs the work of the Drafts folder due on this account.
pub(crate) async fn run(s: &mut Session, host: &dyn SyncHost, account_id: i64, report: &mut SpoolReport) -> Result<(), PassError> {
    let work = write(host, |st| Ok((st.draft_work_claim(account_id)?, ChangeSet::default())))?;
    let mut pending = work.into_iter();
    while let Some(w) = pending.next() {
        let id = w.action_id();
        let result = match &w {
            DraftWork::Append { .. } => append(s, host, &w).await,
            DraftWork::Remove { message_id_hdr, drafts_path, .. } => {
                remove_copies(s, drafts_path, message_id_hdr, None).await.map(|()| Ended::Done)
            }
        };
        match result {
            Ok(Ended::Done) => report.done += 1,
            Ok(Ended::Refused(e)) => {
                eprintln!("drafts: refused by the server: {e}");
                write(host, |st| Ok((st.draft_work_done(id, Some(&e))?, ChangeSet::default())))?;
                report.reverted += 1;
            }
            Err(e) if e.is_fatal() => {
                // The connection is gone: this one and the rest again later.
                let msg = e.to_string();
                write(host, |st| {
                    st.draft_work_retry(id, &msg)?;
                    for rest in pending.by_ref() {
                        st.draft_work_retry(rest.action_id(), &msg)?;
                    }
                    Ok(((), ChangeSet::default()))
                })?;
                report.retried += 1;
                return Err(e.into());
            }
            Err(e @ (ImapError::No { .. } | ImapError::Bad { .. } | ImapError::Unsupported(_))) => {
                let msg = e.to_string();
                eprintln!("drafts: refused by the server: {msg}");
                write(host, |st| Ok((st.draft_work_done(id, Some(&msg))?, ChangeSet::default())))?;
                report.reverted += 1;
            }
            Err(e) => {
                let msg = e.to_string();
                write(host, |st| Ok((st.draft_work_retry(id, &msg)?, ChangeSet::default())))?;
                report.retried += 1;
            }
        }
    }
    Ok(())
}

async fn append(s: &mut Session, host: &dyn SyncHost, w: &DraftWork) -> Result<Ended, ImapError> {
    let DraftWork::Append { action_id, draft_id, raw_hash, message_id_hdr, previous_uid, drafts_path, sent_path, .. } = w else {
        return Ok(Ended::Done);
    };
    let (action_id, draft_id) = (*action_id, *draft_id);
    let drafts = MailboxName::new(drafts_path);
    // 1. Sent or abandoned meanwhile: its copies go.
    if !read(host, |st| st.draft_exists(draft_id)).unwrap_or(true) {
        remove_copies(s, drafts_path, message_id_hdr, None).await?;
        let _ = write(host, |st| Ok((st.draft_work_done(action_id, Some("draft gone: copies removed"))?, ChangeSet::default())));
        return Ok(Ended::Done);
    }
    // 2. The last copy written: still there?
    if let Some(uid) = previous_uid {
        s.select(&drafts, None).await?;
        let there = !s.uid_search(&SearchKey::Uid(UidSet::from_uids([*uid]))).await?.is_empty();
        if !there && let Some(sent) = sent_path {
            s.examine(&MailboxName::new(sent), None).await?;
            let sent_here = !s.uid_search(&header(message_id_hdr)).await?.is_empty();
            if sent_here {
                eprintln!("drafts: {message_id_hdr} was sent elsewhere: the draft goes");
                write(host, |st| Ok(((), st.draft_sent_elsewhere(action_id, draft_id)?)))
                    .map_err(|e| ImapError::Io(e.to_string()))?;
                return Ok(Ended::Done);
            }
        }
        if !there {
            eprintln!("drafts: the copy of {message_id_hdr} went elsewhere; changed here since: written again");
        }
    }
    // 3. The new version.
    let mime = host.raw().read(raw_hash).map_err(|e| ImapError::Io(format!("raw file of the draft: {e}")))?;
    let appended = match s.append(&drafts, &["\\Draft", "\\Seen"], Some(unix_now()), mime).await {
        Ok(a) => a,
        Err(e @ (ImapError::No { .. } | ImapError::Bad { .. })) => return Ok(Ended::Refused(e.to_string())),
        Err(e) => return Err(e),
    };
    // Its UID: given by the server, else the latest of its Message-ID.
    s.select(&drafts, None).await?;
    let uid = match appended {
        Some(a) => Some(a.uid),
        None => s.uid_search(&header(message_id_hdr)).await?.into_iter().max(),
    };
    // 4. The other copies go.
    remove_copies(s, drafts_path, message_id_hdr, uid).await?;
    write(host, |st| Ok(((), st.draft_appended(action_id, uid)?))).map_err(|e| ImapError::Io(e.to_string()))?;
    Ok(Ended::Done)
}

/// Removes from Drafts every copy of `message_id`, except `keep`. Only in
/// Drafts, and only that `Message-ID`: nothing else can be touched.
async fn remove_copies(s: &mut Session, drafts_path: &str, message_id: &str, keep: Option<u32>) -> Result<(), ImapError> {
    let drafts = MailboxName::new(drafts_path);
    if s.selected().is_none_or(|(m, read_only)| m.name != drafts.name || read_only) {
        s.select(&drafts, None).await?;
    }
    let found: Vec<u32> = s.uid_search(&header(message_id)).await?.into_iter().filter(|u| Some(*u) != keep).collect();
    if found.is_empty() {
        return Ok(());
    }
    let set = Uids::Set(UidSet::from_uids(found.iter().copied()));
    s.uid_store(&set, StoreOp::Add, &["\\Deleted"], true, None).await?;
    match s.uid_expunge(&set).await {
        Ok(()) => Ok(()),
        // Without UIDPLUS, `EXPUNGE` would also remove what the phone
        // marked deleted: the copies stay marked, and go at its next one.
        Err(ImapError::Unsupported(_)) => Ok(()),
        Err(e) => Err(e),
    }
}

fn header(message_id: &str) -> SearchKey {
    SearchKey::Header("Message-ID".into(), format!("<{message_id}>"))
}
