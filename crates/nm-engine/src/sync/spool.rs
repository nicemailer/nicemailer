//! Sends the spool to the server (`plan-imap.md`, phase 5; `modele de
//! donnees.md`, section 6; the database side is `nm-store`, `spool.rs`).
//!
//! Runs on the work connection, before each pass, and at once when a
//! command queued actions (`Syncer::wake`). Each round claims the actions
//! that can run (the first of each message, due), groups them, and sends
//! each group in one command:
//!
//! - moves with the same source and target folders: one `UID MOVE` (or,
//!   without `MOVE`, `UID COPY`, `\Deleted`, `UID EXPUNGE`), whose
//!   `COPYUID` gives the new UID of each message;
//! - flags with the same folder, flag and value: one `UID STORE … .SILENT`.
//!
//! How each action ends:
//!
//! | What happened | Action |
//! | --- | --- |
//! | The server did it | `done`, with the new UID of a moved message when known |
//! | The source UID is gone (moved or deleted elsewhere, or a move already done before a crash) | `done`: not a failure. The message is looked for in the target by `Message-ID` |
//! | The server refused (`NO`: folder missing, quota) | `reverted`: the local change is undone, `ActionReverted` |
//! | Connection lost, time limit | tried again later (30 s to 1 h, 8 times at most) |
//! | A keyword in a folder that keeps none (`PERMANENTFLAGS` without `\*`) | `done` without sending: the tag stays local |
//!
//! Folders are opened with `SELECT` here (the synchronization uses
//! `EXAMINE`): the spool is the one part of NiceMailer that writes on the
//! server.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use nm_proto::imap::{ImapError, MailboxName, SearchKey, Session, StoreOp, UidSet, Uids};
use nm_store::ChangeSet;
use nm_store::spool::{Action, ActionKind};

use super::pass::PassError;
use super::{SyncHost, write};

/// Actions claimed per round.
const ROUND: u32 = 500;

/// How one action ended.
#[derive(Debug)]
enum Outcome {
    Done(Option<u32>),
    Retry(String),
    Revert(String),
}

/// What a run did.
#[derive(Debug, Default)]
pub(crate) struct SpoolReport {
    pub done: u64,
    pub reverted: u64,
    pub retried: u64,
}

/// Sends every action that can run now. `keywords`: whether each folder
/// keeps keywords, learnt at its `SELECT`, for the synchronization
/// (`pass.rs`), which must not erase local tags of a folder that keeps none.
pub(crate) async fn run(
    s: &mut Session,
    host: &dyn SyncHost,
    account_id: i64,
    keywords: &mut HashMap<String, bool>,
) -> Result<SpoolReport, PassError> {
    let mut report = SpoolReport::default();
    write(host, |st| Ok((st.spool_purge()?, ChangeSet::default())))?;
    initial_archive(s, host, account_id, &mut report).await?;
    // The copies of the drafts in Drafts (`drafts.rs`).
    super::drafts::run(s, host, account_id, &mut report).await?;
    loop {
        let actions = write(host, |st| Ok((st.spool_claim(account_id, ROUND)?, ChangeSet::default())))?;
        if actions.is_empty() {
            return Ok(report);
        }
        let mut outcomes: Vec<(i64, Outcome)> = Vec::new();
        let result = send(s, &actions, keywords, &mut outcomes).await;
        // Claimed but not ended (the connection broke in the middle): tried
        // again later, like any temporary failure.
        let ended: BTreeSet<i64> = outcomes.iter().map(|(id, _)| *id).collect();
        if let Err(e) = &result {
            for a in &actions {
                if !ended.contains(&a.id) {
                    outcomes.push((a.id, Outcome::Retry(e.to_string())));
                }
            }
        }
        for (_, o) in &outcomes {
            match o {
                Outcome::Done(_) => report.done += 1,
                Outcome::Retry(_) => report.retried += 1,
                Outcome::Revert(_) => report.reverted += 1,
            }
        }
        write(host, |st| {
            let mut all = ChangeSet::default();
            for (id, o) in &outcomes {
                let c = match o {
                    Outcome::Done(uid) => st.spool_done(*id, *uid)?,
                    Outcome::Retry(e) => st.spool_retry(*id, e)?,
                    Outcome::Revert(e) => st.spool_revert(*id, nm_api::reason::SERVER_REFUSED_CHANGE.with("server_text", e.as_str()))?,
                };
                merge(&mut all, c);
            }
            Ok(((), all))
        })?;
        if let Err(e) = result {
            return Err(e.into());
        }
    }
}

/// Sends the actions of one round, group by group; records how each ended.
/// An error is one that ends the session: the caller retries the rest.
async fn send(
    s: &mut Session,
    actions: &[Action],
    keywords: &mut HashMap<String, bool>,
    outcomes: &mut Vec<(i64, Outcome)>,
) -> Result<(), ImapError> {
    let mut moves: BTreeMap<(String, String), Vec<&Action>> = BTreeMap::new();
    let mut flags: BTreeMap<(String, String, bool), Vec<&Action>> = BTreeMap::new();
    for a in actions {
        match &a.kind {
            ActionKind::Move { from_path, to_path, .. } => moves.entry((from_path.clone(), to_path.clone())).or_default().push(a),
            ActionKind::SetFlag { path, flag, set, .. } => flags.entry((path.clone(), flag.clone(), *set)).or_default().push(a),
        }
    }
    for ((from, to), group) in moves {
        move_group(s, &from, &to, &group, outcomes).await?;
    }
    for ((path, flag, set), group) in flags {
        flag_group(s, &path, &flag, set, &group, keywords, outcomes).await?;
    }
    Ok(())
}

/// What a refusal of the server means for the actions: `Err` for an error
/// that ends the session, else the outcome of every action of the group.
fn refusal(e: ImapError) -> Result<Outcome, ImapError> {
    match e {
        e if e.is_fatal() => Err(e),
        // The server said no for good: the target folder does not exist,
        // the quota is exceeded, the command is not possible here.
        e @ (ImapError::No { .. } | ImapError::Bad { .. } | ImapError::Unsupported(_)) => Ok(Outcome::Revert(e.to_string())),
        e => Ok(Outcome::Retry(e.to_string())),
    }
}

async fn move_group(
    s: &mut Session,
    from: &str,
    to: &str,
    group: &[&Action],
    outcomes: &mut Vec<(i64, Outcome)>,
) -> Result<(), ImapError> {
    if let Err(e) = s.select(&MailboxName::new(from), None).await {
        // The source folder cannot be opened: its messages are no longer
        // there for this connection; looked for in the target.
        refusal(e)?;
        return find_in_target(s, to, group, outcomes).await;
    }
    // The source UID of each action; found by `Message-ID` when unknown.
    let mut by_uid: BTreeMap<u32, &Action> = BTreeMap::new();
    let mut gone: Vec<&Action> = Vec::new();
    for a in group {
        let ActionKind::Move { from_uid, .. } = &a.kind else { continue };
        let uid = match from_uid {
            Some(u) => Some(*u),
            None => match &a.message_id_hdr {
                Some(id) => one(s.uid_search(&SearchKey::Header("Message-ID".into(), format!("<{id}>"))).await?),
                None => None,
            },
        };
        match uid {
            Some(u) => {
                by_uid.insert(u, a);
            }
            None => gone.push(a),
        }
    }
    // Which are still there: the others were moved or deleted elsewhere,
    // or by this very action before a crash.
    let present: BTreeSet<u32> = if by_uid.is_empty() {
        BTreeSet::new()
    } else {
        s.uid_search(&SearchKey::Uid(UidSet::from_uids(by_uid.keys().copied()))).await?.into_iter().collect()
    };
    for (uid, a) in &by_uid {
        if !present.contains(uid) {
            gone.push(a);
        }
    }
    if !present.is_empty() {
        match s.uid_move(&Uids::Set(UidSet::from_uids(present.iter().copied())), &MailboxName::new(to)).await {
            Ok(r) => {
                crash_point();
                let new: HashMap<u32, u32> = r.copy_uids.map(|c| c.pairs.into_iter().collect()).unwrap_or_default();
                for uid in &present {
                    outcomes.push((by_uid[uid].id, Outcome::Done(new.get(uid).copied())));
                }
            }
            Err(e) => {
                let o = refusal(e)?;
                // Refused: maybe because a message went meanwhile (another
                // Mac moved it between the search and the move). Those are
                // not failures; only the ones still here are.
                let still: BTreeSet<u32> = if matches!(o, Outcome::Revert(_)) {
                    s.uid_search(&SearchKey::Uid(UidSet::from_uids(present.iter().copied()))).await?.into_iter().collect()
                } else {
                    present.clone()
                };
                for uid in &present {
                    if still.contains(uid) {
                        outcomes.push((by_uid[uid].id, clone(&o)));
                    } else {
                        gone.push(by_uid[uid]);
                    }
                }
            }
        }
    }
    if !gone.is_empty() {
        find_in_target(s, to, &gone, outcomes).await?;
    }
    Ok(())
}

/// Messages no longer in their source: done, with their UID in the target
/// if found there by `Message-ID` (`modele de donnees.md`, section 6,
/// "Message disparu du serveur").
async fn find_in_target(s: &mut Session, to: &str, group: &[&Action], outcomes: &mut Vec<(i64, Outcome)>) -> Result<(), ImapError> {
    let opened = match s.examine(&MailboxName::new(to), None).await {
        Ok(_) => true,
        Err(e) if e.is_fatal() => return Err(e),
        Err(_) => false,
    };
    for a in group {
        let uid = match (&a.message_id_hdr, opened) {
            (Some(id), true) => one(s.uid_search(&SearchKey::Header("Message-ID".into(), format!("<{id}>"))).await?),
            _ => None,
        };
        outcomes.push((a.id, Outcome::Done(uid)));
    }
    Ok(())
}

async fn flag_group(
    s: &mut Session,
    path: &str,
    flag: &str,
    set: bool,
    group: &[&Action],
    keywords: &mut HashMap<String, bool>,
    outcomes: &mut Vec<(i64, Outcome)>,
) -> Result<(), ImapError> {
    let selected_here = s.selected().is_some_and(|(m, read_only)| m.name == path && !read_only);
    if !selected_here {
        match s.select(&MailboxName::new(path), None).await {
            Ok(sel) => {
                let any = sel.permanent_flags.iter().any(|f| f == "\\*");
                keywords.insert(path.to_string(), any);
            }
            Err(e) => {
                let o = refusal(e)?;
                for a in group {
                    outcomes.push((a.id, clone(&o)));
                }
                return Ok(());
            }
        }
    }
    // A keyword in a folder that keeps none: the tag stays local.
    if !flag.starts_with('\\') && keywords.get(path) == Some(&false) {
        for a in group {
            outcomes.push((a.id, Outcome::Done(None)));
        }
        return Ok(());
    }
    let uids: Vec<u32> = group.iter().filter_map(|a| match &a.kind {
        ActionKind::SetFlag { uid, .. } => Some(*uid),
        _ => None,
    }).collect();
    let op = if set { StoreOp::Add } else { StoreOp::Remove };
    // A UID gone meanwhile is ignored by the server: the change of a
    // message deleted elsewhere is done, as it should be.
    match s.uid_store(&Uids::Set(UidSet::from_uids(uids)), op, &[flag], true, None).await {
        Ok(_) => {
            for a in group {
                outcomes.push((a.id, Outcome::Done(None)));
            }
        }
        Err(e) => {
            let o = refusal(e)?;
            for a in group {
                outcomes.push((a.id, clone(&o)));
            }
        }
    }
    Ok(())
}

/// The single UID of a search, if exactly one.
fn one(found: Vec<u32>) -> Option<u32> {
    match found[..] {
        [u] => Some(u),
        _ => None,
    }
}

fn clone(o: &Outcome) -> Outcome {
    match o {
        Outcome::Done(u) => Outcome::Done(*u),
        Outcome::Retry(e) => Outcome::Retry(e.clone()),
        Outcome::Revert(e) => Outcome::Revert(e.clone()),
    }
}

/// Merges the changes of one write into those of a batch.
pub(crate) fn merge(into: &mut ChangeSet, from: ChangeSet) {
    into.announced = into.merged_announced(&from);
    into.messages.extend(from.messages);
    for t in from.threads {
        if !into.threads.iter().any(|(id, _)| *id == t.0) {
            into.threads.push(t);
        }
    }
    into.folders.extend(from.folders);
    into.accounts.extend(from.accounts);
    into.arrived.extend(from.arrived);
    into.spooled.extend(from.spooled);
    into.reverted.extend(from.reverted);
}

/// Tests only: `NICEMAILER_TEST_CRASH_AFTER_MOVE` makes the core die right
/// after the server moved messages, before the actions are marked done,
/// as a crash or a power cut would (`plan-imap.md`, phase 5: "plantage
/// simulé entre le MOVE et le passage à done").
fn crash_point() {
    if std::env::var_os("NICEMAILER_TEST_CRASH_AFTER_MOVE").is_some() {
        eprintln!("spool: NICEMAILER_TEST_CRASH_AFTER_MOVE: stopping right after the MOVE");
        std::process::abort();
    }
}

/// The archiving of the initial import (`import-initial.md`, section 6):
/// the Inbox's old mail moved to Archive on the server, by batches of
/// `import.batch_size`, before the Inbox is synchronized. Replayable: after
/// an interruption, the search no longer finds the mails moved already.
async fn initial_archive(s: &mut Session, host: &dyn SyncHost, account_id: i64, report: &mut SpoolReport) -> Result<(), PassError> {
    let actions = write(host, |st| Ok((st.spool_claim_account(account_id)?, ChangeSet::default())))?;
    for a in actions {
        let c = &a.criteria;
        let result: Result<u64, ImapError> = async {
            let (read, unread) =
                crate::setup::archive_uids(s, c.boundary, c.read_before.as_deref(), c.unread_before.as_deref()).await?;
            let mut uids: Vec<u32> = read.into_iter().chain(unread).collect();
            uids.sort_unstable();
            uids.dedup();
            s.select(&MailboxName::new(&c.from_path), None).await?;
            let mut moved = 0u64;
            for chunk in uids.chunks(c.batch_size as usize) {
                s.uid_move(&Uids::Set(UidSet::from_uids(chunk.iter().copied())), &MailboxName::new(&c.to_path)).await?;
                moved += chunk.len() as u64;
                archive_crash_point();
            }
            Ok(moved)
        }
        .await;
        match result {
            Ok(moved) => {
                write(host, |st| Ok(((), {
                    st.initial_archive_done(a.id, moved)?;
                    ChangeSet::default()
                })))?;
                eprintln!("spool: initial archiving: {moved} mails moved to {}", c.to_path);
                report.done += 1;
            }
            Err(e) if e.is_fatal() => {
                let msg = e.to_string();
                write(host, |st| Ok(((), {
                    st.initial_archive_retry(a.id, &msg)?;
                    ChangeSet::default()
                })))?;
                report.retried += 1;
                return Err(e.into());
            }
            Err(e) => {
                let o = refusal(e)?;
                write(host, |st| Ok(((), {
                    match &o {
                        Outcome::Revert(r) => st.initial_archive_failed(a.id, r)?,
                        Outcome::Retry(r) => st.initial_archive_retry(a.id, r)?,
                        Outcome::Done(_) => {}
                    }
                    ChangeSet::default()
                })))?;
                report.reverted += 1;
            }
        }
    }
    Ok(())
}

/// Tests only: `NICEMAILER_TEST_CRASH_DURING_ARCHIVE` makes the core die
/// after the first batch of the initial archiving, as a crash would.
fn archive_crash_point() {
    if std::env::var_os("NICEMAILER_TEST_CRASH_DURING_ARCHIVE").is_some() {
        eprintln!("spool: NICEMAILER_TEST_CRASH_DURING_ARCHIVE: stopping after a batch");
        std::process::abort();
    }
}
