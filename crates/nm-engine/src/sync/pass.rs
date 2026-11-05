//! One pass over the folders of an account (`modele de donnees.md`,
//! section 5; `nm-store`, `sync.rs`, "A pass, in three steps").
//!
//! 1. **List** the folders (`LIST`, with `LIST-STATUS` when the server has
//!    it, else one `STATUS` per role folder), and keep the database in
//!    line (new, renamed, missing folders).
//! 2. **Which folders changed**: the counters of `STATUS` against those of
//!    the database. A folder whose `UIDVALIDITY`, `UIDNEXT`, message count
//!    and `HIGHESTMODSEQ` are unchanged is not opened at all: a pass over
//!    twelve quiet folders costs one command.
//! 3. **Discovery** of each folder that changed, with the best the server
//!    offers:
//!    - QRESYNC: `EXAMINE … (QRESYNC (uidvalidity modseq))` gives the
//!      UIDs expunged and the flags changed since the last pass, in one
//!      command;
//!    - CONDSTORE alone: flags by `UID FETCH 1:* (FLAGS) (CHANGEDSINCE m)`,
//!      expunges by comparing `UID SEARCH ALL` with the database;
//!    - neither: expunges by `UID SEARCH ALL`; flags by `UID FETCH 1:*
//!      (FLAGS)`, about 40 bytes a message, done for the Inbox at every
//!      pass and for the other folders at each `poll_interval_secs` (open
//!      question 2 of the plan: to be measured on real servers).
//! 4. **Arrivals**: the headers of the new UIDs, in batches of 100, moves
//!    made elsewhere recognized against the expunges of step 3.
//! 5. **End** of each folder: flags, expunges, new state, one transaction.
//!
//! Every folder is opened with `EXAMINE`: read-only (see `mod.rs`).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::Instant;

use nm_api::Event;
use nm_proto::imap::{FetchItems, ImapError, ListEntry, MailboxName, MailboxStatus, Namespace, QresyncParams, SearchKey, Session, UidSet, Uids};
use nm_store::ingest::parse_headers;
use nm_store::insert::Inserter;
use nm_store::sync::{Arrival, FlagUpdate, FolderState, ListedFolder, ServerFlags, SyncFolder};
use nm_store::StoreError;

use super::{SyncHost, read, write};
use crate::folders::propose_roles;

/// Headers fetched per command, and inserted per transaction: a few
/// hundred kilobytes, and the store's lock held for a few milliseconds.
const HEADER_BATCH: usize = 100;

/// Why a pass stopped.
#[derive(Debug)]
pub(crate) enum PassError {
    /// The server or the connection. Fatal ones end the session.
    Imap(ImapError),
    Store(StoreError),
    /// The worker was asked to stop.
    Stopped,
}

impl From<ImapError> for PassError {
    fn from(e: ImapError) -> Self {
        PassError::Imap(e)
    }
}

impl From<StoreError> for PassError {
    fn from(e: StoreError) -> Self {
        PassError::Store(e)
    }
}

impl std::fmt::Display for PassError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PassError::Imap(e) => write!(f, "{e}"),
            PassError::Store(e) => write!(f, "database: {e}"),
            PassError::Stopped => write!(f, "stopped"),
        }
    }
}

/// How the server is followed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Qresync,
    Condstore,
    None,
}

impl Mode {
    pub(crate) fn of(s: &Session) -> Mode {
        if s.qresync() {
            Mode::Qresync
        } else if s.condstore() {
            Mode::Condstore
        } else {
            Mode::None
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Mode::Qresync => "qresync",
            Mode::Condstore => "condstore",
            Mode::None => "none",
        }
    }
}

/// What a pass needs that outlives it.
pub(crate) struct Ctx<'a> {
    pub host: &'a dyn SyncHost,
    pub account_id: i64,
    pub control: &'a super::Control,
    /// The address ids of the account, kept from one pass to the next.
    pub inserter: &'a mut Inserter,
    /// The personal namespace (for the role proposal).
    pub namespace: Option<Namespace>,
}

/// What a pass did.
#[derive(Debug, Default)]
pub(crate) struct PassReport {
    /// New rows (not counting moves).
    pub inserted: u64,
    pub moved: u64,
    pub deleted: u64,
    pub folders_synced: u64,
    /// Role folders the server no longer lists.
    pub missing: Vec<String>,
    /// Problems with one folder, which did not stop the pass.
    pub problems: Vec<String>,
}

/// What discovery found in one folder.
struct Work {
    folder: SyncFolder,
    uidvalidity: u32,
    uidnext: Option<u32>,
    modseq: Option<u64>,
    new_uids: Vec<u32>,
    flags: Vec<FlagUpdate>,
    /// Whether its new mail is announced (`MessageArrived`): not during the
    /// first synchronization of the folder.
    announce: bool,
}

/// One pass. `full`: also compare every flag on a server without
/// CONDSTORE (at each poll interval, and on `sync_now`).
pub(crate) async fn run(s: &mut Session, ctx: &mut Ctx<'_>, full: bool) -> Result<PassReport, PassError> {
    let mode = Mode::of(s);
    let mut report = PassReport::default();

    // 1. The folders.
    let listed: Vec<(ListEntry, Option<MailboxStatus>)> = if s.capabilities().has("LIST-STATUS") {
        s.list_with_status().await?
    } else {
        s.list().await?.into_iter().map(|e| (e, None)).collect()
    };
    let entries: Vec<ListEntry> = listed.iter().map(|(e, _)| e.clone()).collect();
    let roles = propose_roles(&entries, ctx.namespace.as_ref());
    let mut statuses: HashMap<String, MailboxStatus> = HashMap::new();
    let mut folders_listed = Vec::new();
    for ((e, st), role) in listed.into_iter().zip(roles) {
        if !e.selectable() {
            continue;
        }
        // A name whose modified UTF-7 does not come back the same once
        // decoded and encoded again cannot be opened by its stored path:
        // skipped (rare: a server writing non-canonical names).
        if MailboxName::new(&e.name.name).raw != e.name.raw {
            report.problems.push(format!("folder {:?}: name not in canonical modified UTF-7, skipped", e.name.name));
            continue;
        }
        if let Some(st) = st {
            statuses.insert(e.name.name.clone(), st);
        }
        folders_listed.push(ListedFolder {
            uidvalidity: statuses.get(&e.name.name).and_then(|s| s.uid_validity),
            path: e.name.name,
            delimiter: e.delimiter.map(String::from),
            proposed_role: role.map(|(r, _)| r),
        });
    }
    let account_id = ctx.account_id;
    let (folders, change) = write(ctx.host, |st| {
        let (f, c, changes) = st.sync_folder_list(account_id, &folders_listed)?;
        Ok(((f, c), changes))
    })?;
    report.missing = change.missing;
    // Without LIST-STATUS: one STATUS per folder to synchronize.
    for f in &folders {
        if !statuses.contains_key(&f.path) {
            match s.status(&MailboxName::new(&f.path)).await {
                Ok(st) => {
                    statuses.insert(f.path.clone(), st);
                }
                Err(e) if e.is_fatal() => return Err(e.into()),
                Err(e) => report.problems.push(format!("{}: {e}", f.path)),
            }
        }
    }

    // 2. Which changed. The folder selected on this session (the Inbox,
    // watched by IDLE) is always opened: a server may answer the STATUS of
    // the mailbox a session has selected with what that session already
    // knows, not with its current state (Dovecot does: measured).
    let selected = s.selected().map(|(m, _)| m.name.clone());
    let changed: Vec<SyncFolder> = folders
        .into_iter()
        .filter(|f| {
            let flags_compare = mode == Mode::None && (full || f.role == "inbox");
            if selected.as_deref() == Some(f.path.as_str()) {
                return true;
            }
            match statuses.get(&f.path) {
                None => true,
                Some(st) => {
                    flags_compare
                        || f.uidnext.is_none()
                        || st.uid_validity != f.uidvalidity
                        || st.uid_next != f.uidnext
                        || st.messages != Some(f.messages)
                        || (mode != Mode::None && st.highest_modseq.is_some() && st.highest_modseq != f.highestmodseq)
                }
            }
        })
        .collect();

    // 3. Discovery.
    let mut works: Vec<Work> = Vec::new();
    let mut deletions: BTreeMap<i64, BTreeSet<u32>> = BTreeMap::new();
    for f in changed {
        if ctx.control.stopped() {
            return Err(PassError::Stopped);
        }
        let flags_compare = mode == Mode::None && (full || f.role == "inbox");
        match discover(s, ctx, mode, f, flags_compare, &mut report).await {
            Ok(Some((work, gone))) => {
                deletions.insert(work.folder.id, gone);
                works.push(work);
            }
            Ok(None) => {}
            Err(PassError::Imap(e)) if !e.is_fatal() => report.problems.push(e.to_string()),
            Err(e) => return Err(e),
        }
    }

    // 4. Arrivals.
    for w in &works {
        if w.new_uids.is_empty() {
            continue;
        }
        let name = MailboxName::new(&w.folder.path);
        if let Err(e) = s.examine(&name, None).await {
            if e.is_fatal() {
                return Err(e.into());
            }
            report.problems.push(format!("{}: {e}", w.folder.path));
            continue;
        }
        let items = FetchItems { flags: true, internal_date: true, size: true, modseq: mode != Mode::None, header: true, ..FetchItems::default() };
        let total = w.new_uids.len() as u64;
        let mut done = 0u64;
        let mut last_progress = Instant::now();
        for chunk in w.new_uids.chunks(HEADER_BATCH) {
            if ctx.control.stopped() {
                return Err(PassError::Stopped);
            }
            let fetched = s.uid_fetch(&Uids::Set(UidSet::from_uids(chunk.iter().copied())), &items).await?;
            // Parsed before taking the store's lock.
            let mut batch: Vec<Arrival> = fetched
                .into_iter()
                .filter_map(|f| {
                    let uid = f.uid?;
                    Some(Arrival {
                        uid,
                        flags: ServerFlags::parse(f.flags.as_deref().unwrap_or(&[])),
                        // A server without INTERNALDATE: the Date header, else 0.
                        date_received: f.internal_date.unwrap_or(0),
                        size: f.size.map(u64::from),
                        modseq: f.modseq,
                        headers: parse_headers(f.header.as_deref().unwrap_or(b"")),
                    })
                })
                .collect();
            for a in &mut batch {
                if a.date_received == 0 {
                    a.date_received = a.headers.date_sent.unwrap_or(0);
                }
            }
            batch.sort_by_key(|a| a.uid);
            let folder_id = w.folder.id;
            let announce = w.announce;
            let inserter = &mut *ctx.inserter;
            let deletions = &mut deletions;
            let got = write(ctx.host, |st| st.insert_arrivals(inserter, account_id, folder_id, &batch, deletions, announce))?;
            report.inserted += got.inserted as u64;
            report.moved += got.moved as u64;
            done += chunk.len() as u64;
            if last_progress.elapsed().as_millis() >= 250 || done == total {
                last_progress = Instant::now();
                ctx.host.transient(Event::SyncProgress { account_id, folder_id: Some(folder_id), stage: "headers".into(), done, total });
            }
        }
    }

    // 5. End of each folder.
    for w in works {
        let gone = deletions.remove(&w.folder.id).unwrap_or_default();
        report.deleted += gone.len() as u64;
        let state = FolderState { uidvalidity: w.uidvalidity, uidnext: w.uidnext, highestmodseq: w.modseq };
        let folder_id = w.folder.id;
        write(ctx.host, |st| Ok(((), st.finish_folder(folder_id, &w.flags, &gone, state)?)))?;
        report.folders_synced += 1;
    }
    // What the server said on its own during the pass is in the database
    // now, or will be at the next pass: dropped, not kept in memory.
    s.take_unsolicited();
    Ok(report)
}

/// Step 3 for one folder: what vanished (returned apart, to fill the
/// deletions of the pass), which flags changed, which UIDs are new.
/// `None` if the folder could not be opened.
async fn discover(
    s: &mut Session,
    ctx: &mut Ctx<'_>,
    mode: Mode,
    mut f: SyncFolder,
    flags_compare: bool,
    report: &mut PassReport,
) -> Result<Option<(Work, BTreeSet<u32>)>, PassError> {
    let name = MailboxName::new(&f.path);
    let qresync = match (mode, f.uidvalidity, f.highestmodseq) {
        (Mode::Qresync, Some(uv), Some(m)) => Some(QresyncParams { uid_validity: uv, modseq: m, known_uids: None }),
        _ => None,
    };
    let sel = match s.examine(&name, qresync.as_ref()).await {
        Ok(sel) => sel,
        Err(e) if e.is_fatal() => return Err(e.into()),
        Err(e) => {
            report.problems.push(format!("{}: {e}", f.path));
            return Ok(None);
        }
    };
    let Some(uv) = sel.uid_validity else {
        report.problems.push(format!("{}: the server gives no UIDVALIDITY, folder skipped", f.path));
        return Ok(None);
    };
    let host = ctx.host;
    let mut announce = f.synced;
    let mut reset = false;
    if f.uidvalidity.is_some_and(|old| old != uv) {
        // The UIDs of the folder name other messages now: start again.
        let folder_id = f.id;
        let n = write(host, |st| st.purge_folder(folder_id, uv))?;
        report.problems.push(format!("{}: UIDVALIDITY changed, its {n} messages dropped, the folder is fetched again", f.path));
        f.uidnext = None;
        f.highestmodseq = None;
        f.messages = 0;
        announce = false;
        reset = true;
    }
    let folder_id = f.id;
    write(host, |st| Ok(((), {
        st.start_folder(folder_id, uv)?;
        nm_store::ChangeSet::default()
    })))?;
    let modseq = if mode != Mode::None && !sel.no_modseq { sel.highest_modseq } else { None };

    let mut gone = BTreeSet::new();
    let mut flags = Vec::new();
    let new_uids: Vec<u32> = if qresync.is_some() && !reset && mode == Mode::Qresync && !sel.no_modseq {
        gone.extend(sel.vanished.iter());
        flags.extend(sel.changed.iter().filter_map(|c| {
            Some(FlagUpdate { uid: c.uid?, flags: ServerFlags::parse(c.flags.as_deref()?), modseq: c.modseq })
        }));
        let from = f.uidnext.unwrap_or(1);
        if sel.uid_next.is_some_and(|n| n <= from) || sel.exists == 0 {
            Vec::new()
        } else {
            let found = s.uid_search(&SearchKey::Uid(UidSet::from_ranges([(from, u32::MAX)]))).await?;
            let mut v: Vec<u32> = found.into_iter().filter(|u| *u >= from).collect();
            v.sort_unstable();
            v
        }
    } else {
        // Compare with what the database has.
        let known = read(host, |st| st.known_messages(folder_id))?;
        let known: BTreeMap<u32, ServerFlags> = known.into_iter().map(|k| (k.uid, k.flags)).collect();
        let mut server: BTreeSet<u32> = BTreeSet::new();
        if sel.exists > 0 {
            if flags_compare && !reset {
                // UIDs and flags in one command.
                let items = FetchItems { flags: true, ..FetchItems::default() };
                for m in s.uid_fetch(&Uids::From(1), &items).await? {
                    let Some(uid) = m.uid else { continue };
                    server.insert(uid);
                    let sf = ServerFlags::parse(m.flags.as_deref().unwrap_or(&[]));
                    if known.get(&uid).is_some_and(|k| *k != sf) {
                        flags.push(FlagUpdate { uid, flags: sf, modseq: None });
                    }
                }
            } else {
                server.extend(s.uid_search(&SearchKey::All).await?);
                if mode == Mode::Condstore
                    && !reset
                    && let (Some(old), false) = (f.highestmodseq, sel.no_modseq)
                {
                    let items = FetchItems { flags: true, modseq: true, changed_since: Some(old), ..FetchItems::default() };
                    for m in s.uid_fetch(&Uids::From(1), &items).await? {
                        let Some(uid) = m.uid else { continue };
                        if let Some(fl) = &m.flags {
                            flags.push(FlagUpdate { uid, flags: ServerFlags::parse(fl), modseq: m.modseq });
                        }
                    }
                }
            }
        }
        gone.extend(known.keys().filter(|u| !server.contains(u)));
        server.iter().filter(|u| !known.contains_key(u)).copied().collect()
    };
    let uidnext = sel.uid_next.or_else(|| new_uids.last().map(|u| u + 1)).or(f.uidnext);
    Ok(Some((Work { folder: f, uidvalidity: uv, uidnext, modseq, new_uids, flags, announce }, gone)))
}
