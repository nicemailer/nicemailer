//! The database side of the IMAP synchronization (`plan-imap.md`, phase 4;
//! `modele de donnees.md`, section 5): what the engine (`nm-engine`,
//! `sync/`) found on the server, written in the database.
//!
//! This module talks to no server: the engine reads the server, then calls
//! these functions with what it read. That keeps every rule here testable
//! without a network, and every write short (the store's lock is held for
//! milliseconds, never while waiting for a server).
//!
//! # A pass, in three steps
//!
//! The engine synchronizes the folders of an account in *passes*:
//!
//! 1. **Discovery**, folder by folder: what disappeared (`VANISHED`, or a
//!    comparison of the UIDs), which flags changed, which UIDs are new. Not
//!    written yet.
//! 2. **Arrivals** ([`Store::insert_arrivals`]), folder by folder, in
//!    batches: the new messages, each either a message moved elsewhere
//!    (found in the deletions of step 1) or a new row.
//! 3. **End of folder** ([`Store::finish_folder`]): flags, deletions, and
//!    the folder's new state (`uidvalidity`, `uidnext`, `highestmodseq`),
//!    in one transaction.
//!
//! Why deletions wait for step 3: a mail moved by the phone from Inbox to
//! Archive vanishes from Inbox and appears in Archive. If the Inbox were
//! written first, its row would be gone when Archive's copy arrives, and
//! the message would come back as a new one: new id, tags, history and
//! consultation lost. With the deletions of the whole pass known before any
//! arrival, the row is moved instead (`plan-imap.md`, "Rapprochement des
//! déplacements faits ailleurs").
//!
//! # Crash safety
//!
//! `uidnext` advances with each batch of arrivals, in the batch's
//! transaction: after a crash, the next pass fetches from the first UID
//! not written, never twice (`UNIQUE (folder_id, uid)`). `highestmodseq`
//! only advances in step 3, with the flags and deletions: after a crash
//! before it, the server reports the same changes again, and applying them
//! twice is harmless.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use nm_core::Source;
use rusqlite::{OptionalExtension, Transaction, params};

use crate::ingest::ParsedMessage;
use crate::insert::{Flags, Inserter, NewMessage};
use crate::screening::refresh_all;
use crate::{ChangeSet, Result, Store};

/// The order in which the folders of an account are synchronized: the
/// Inbox first (new mail), then the folders the user works in, then the
/// rest.
const ROLE_ORDER: &[&str] = &[
    "inbox", "todo", "waiting", "sent", "drafts", "archive", "newsletter", "paperwork", "quarantine", "junk",
    "banned", "deleted",
];

/// A folder of the server, as listed (`LIST`), with the role NiceMailer
/// proposes for it (`nm-engine`, `folders.rs`).
#[derive(Debug, Clone)]
pub struct ListedFolder {
    /// The decoded name (modified UTF-7 decoded), as stored in
    /// `folder.path`.
    pub path: String,
    pub delimiter: Option<String>,
    pub proposed_role: Option<&'static str>,
    /// Its `UIDVALIDITY`, when `LIST-STATUS` or `STATUS` gave it: what
    /// recognizes a folder renamed on the server.
    pub uidvalidity: Option<u32>,
}

/// A folder to synchronize, with what the database knows of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncFolder {
    pub id: i64,
    pub path: String,
    pub role: String,
    pub uidvalidity: Option<u32>,
    pub uidnext: Option<u32>,
    pub highestmodseq: Option<u64>,
    /// Messages of the folder in the database. 0 until
    /// [`Store::fill_message_counts`] has counted them.
    pub messages: u32,
    /// `uid_screening_start` (Inbox): the mails at or above it are screened.
    pub screening_start: Option<u32>,
    /// Synchronized completely at least once (`date_last_sync`): its new
    /// mail is new for the user (`MessageArrived`). A first
    /// synchronization interrupted and resumed is still a first one.
    pub synced: bool,
}

/// What [`Store::sync_folder_list`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FolderListChange {
    /// Paths of the folders new in the database.
    pub created: Vec<String>,
    /// Folders renamed on the server, `(old path, new path)`.
    pub renamed: Vec<(String, String)>,
    /// Folders with a role that the server no longer lists. Kept, with
    /// their mail: a folder deleted by mistake, or a server that lists
    /// badly for a moment, must not delete thousands of mails. Not
    /// synchronized until it comes back or gets another folder.
    pub missing: Vec<String>,
    /// Roles given by the first listing of an account (see
    /// [`Store::sync_folder_list`]), `(path, role)`.
    pub roles_given: Vec<(String, String)>,
}

/// The flags of a message on the server: system flags and keywords.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServerFlags {
    pub flags: Flags,
    /// Keywords as the server spells them (`$Forwarded`, `$NmWasTodo`…),
    /// sorted, without duplicates (compared without case).
    pub keywords: Vec<String>,
}

impl ServerFlags {
    /// From the flags of a `FETCH` (`\Seen`, `$Junk`…). `\Recent` is a
    /// session flag, never stored (and `nm-proto` already drops it).
    pub fn parse<S: AsRef<str>>(list: &[S]) -> ServerFlags {
        let mut out = ServerFlags::default();
        for f in list {
            let f = f.as_ref();
            match f.to_ascii_lowercase().as_str() {
                "\\seen" => out.flags.seen = true,
                "\\flagged" => out.flags.flagged = true,
                "\\answered" => out.flags.answered = true,
                "\\draft" => out.flags.draft = true,
                "\\deleted" => out.flags.deleted = true,
                "\\recent" => {}
                // Another system flag (`\Something`): not a keyword.
                s if s.starts_with('\\') => {}
                _ => {
                    if !out.keywords.iter().any(|k| k.eq_ignore_ascii_case(f)) {
                        out.keywords.push(f.to_string());
                    }
                }
            }
        }
        out.keywords.sort_by_key(|k| k.to_ascii_lowercase());
        out
    }
}

/// A new message of a folder: what the server gave with its headers.
#[derive(Debug, Clone)]
pub struct Arrival {
    pub uid: u32,
    pub flags: ServerFlags,
    /// INTERNALDATE, Unix seconds.
    pub date_received: i64,
    /// RFC822.SIZE.
    pub size: Option<u64>,
    pub modseq: Option<u64>,
    /// `ingest::parse_headers` of its header.
    pub headers: ParsedMessage,
}

/// A change of flags reported by the server for a message already known.
#[derive(Debug, Clone)]
pub struct FlagUpdate {
    pub uid: u32,
    pub flags: ServerFlags,
    pub modseq: Option<u64>,
}

/// What the server says of a folder after a pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FolderState {
    pub uidvalidity: u32,
    pub uidnext: Option<u32>,
    pub highestmodseq: Option<u64>,
}

/// What [`Store::insert_arrivals`] did.
#[derive(Debug, Clone, Default)]
pub struct Arrived {
    /// New rows.
    pub inserted: u32,
    /// Rows moved from another folder (a move made elsewhere).
    pub moved: u32,
    /// Rows moved by NiceMailer that got their UID in this folder.
    pub attached: u32,
    /// New rows the screening moved out of the Inbox.
    pub screened: u32,
}

/// A message of a folder in the database, for comparisons with the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownMessage {
    pub id: i64,
    pub uid: u32,
    pub flags: ServerFlags,
}

impl Store {
    /// Brings the folders of an IMAP account in line with the server's
    /// listing, and returns the folders to synchronize (those with a role,
    /// listed by the server), Inbox first.
    ///
    /// - A folder listed for the first time gets a row (`created_by =
    ///   'imap'`), without role: NiceMailer does not manage folders without
    ///   role (`dossiers.md`).
    /// - Exception, provisional until the account creation panel (phase 7):
    ///   the first listing of an account that has no role at all gives each
    ///   folder the role proposed for it (`folders.rs`: INBOX, special-use,
    ///   then names). Without it, nothing would be synchronized. Recorded
    ///   in `audit_log`.
    /// - A folder with a role that the server no longer lists, while a new
    ///   folder has its `UIDVALIDITY`, was renamed on the server: the row
    ///   follows, with its mail and its role. Servers keep `UIDVALIDITY` on
    ///   a rename (Dovecot keeps the whole mailbox).
    /// - A folder with a role no longer listed, not renamed: kept
    ///   ([`FolderListChange::missing`]). A folder without role no longer
    ///   listed: deleted (it holds no mail).
    pub fn sync_folder_list(
        &mut self,
        account_id: i64,
        listed: &[ListedFolder],
    ) -> Result<(Vec<SyncFolder>, FolderListChange, ChangeSet)> {
        let tx = self.transaction()?;
        let mut change = FolderListChange::default();
        let mut changes = ChangeSet::default();
        // Rust note: a `BTreeMap` keyed by path; `collect` builds it from
        // an iterator of `(key, value)` pairs.
        let existing: BTreeMap<String, (i64, Option<String>, Option<u32>)> = {
            let mut stmt = tx.prepare("SELECT path, id, role, uidvalidity FROM folder WHERE account_id = ?1")?;
            stmt.query_map([account_id], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?, r.get(3)?))))?
                .collect::<std::result::Result<_, _>>()?
        };
        let has_roles = existing.values().any(|(_, role, _)| role.is_some());
        let listed_paths: BTreeSet<&str> = listed.iter().map(|l| l.path.as_str()).collect();
        // Role folders gone from the listing: candidates for a rename.
        let mut gone: Vec<(&String, i64, Option<u32>)> = existing
            .iter()
            .filter(|(p, (_, role, _))| role.is_some() && !listed_paths.contains(p.as_str()))
            .map(|(p, (id, _, uv))| (p, *id, *uv))
            .collect();

        for l in listed {
            if existing.contains_key(&l.path) {
                continue;
            }
            // Renamed: a role folder gone whose UIDVALIDITY this one has.
            if let Some(uv) = l.uidvalidity
                && let Some(pos) = gone.iter().position(|(_, _, old)| *old == Some(uv))
            {
                let (old, id, _) = gone.remove(pos);
                tx.execute(
                    "UPDATE folder SET path = ?2, delimiter = ?3, date_modified = unixepoch(), modified_by = 'imap' WHERE id = ?1",
                    params![id, l.path, l.delimiter],
                )?;
                change.renamed.push((old.clone(), l.path.clone()));
                changes.folders.insert(id);
                continue;
            }
            let role = if has_roles { None } else { l.proposed_role };
            tx.execute(
                "INSERT INTO folder (account_id, path, delimiter, role, date_role_modified, created_by, modified_by)
                 VALUES (?1, ?2, ?3, ?4, CASE WHEN ?4 IS NULL THEN NULL ELSE unixepoch() END, 'imap', 'imap')",
                params![account_id, l.path, l.delimiter, role],
            )?;
            let id = tx.last_insert_rowid();
            if let Some(role) = role {
                tx.execute(
                    "INSERT INTO audit_log (created_by, entity, entity_id, action, detail)
                     VALUES ('imap', 'folder', ?1, 'role', json_object('before', NULL, 'after', ?2, 'reason', 'proposed'))",
                    params![id, role],
                )?;
                change.roles_given.push((l.path.clone(), role.to_string()));
            }
            change.created.push(l.path.clone());
            changes.folders.insert(id);
        }
        for (path, (id, role, _)) in &existing {
            if listed_paths.contains(path.as_str()) || change.renamed.iter().any(|(old, _)| old == path) {
                continue;
            }
            if role.is_some() {
                change.missing.push(path.clone());
            } else {
                tx.execute("DELETE FROM folder WHERE id = ?1", [id])?;
                changes.folders.insert(*id);
            }
        }
        let folders = role_folders(&tx, account_id, &listed_paths, &change)?;
        tx.commit()?;
        Ok((folders, change, changes))
    }

    /// Counts the messages of these folders in the database.
    ///
    /// Why apart from [`Store::sync_folder_list`]: counting reads the index
    /// of every message of the folder, thousands of pages from the disk on
    /// a large mailbox when they are not in memory. The listing runs at each
    /// pass with the store's write lock held; this runs afterwards, on a
    /// reading connection, where that wait holds up no one. A count made a
    /// moment after the listing can only open one folder more than needed.
    pub fn fill_message_counts(&self, folders: &mut [SyncFolder]) -> Result<()> {
        let mut stmt = self.conn.prepare_cached("SELECT count(*) FROM message WHERE folder_id = ?1")?;
        for f in folders {
            f.messages = stmt.query_row([f.id], |r| r.get(0))?;
        }
        Ok(())
    }

    /// The Inbox's screening boundary, set once (provisional until the
    /// account creation of phase 7): its `UIDNEXT` when its synchronization
    /// first starts, so that the mails already there are not screened and
    /// the next ones are (`import-initial.md`, section 5).
    pub fn set_screening_start(&mut self, folder_id: i64, uid: u32) -> Result<()> {
        self.conn.execute("UPDATE folder SET uid_screening_start = ?2 WHERE id = ?1 AND uid_screening_start IS NULL", params![folder_id, uid])?;
        Ok(())
    }

    /// Sets the boundary of the screening again, whatever it was: for an
    /// account made writable (`activate_account`), the mails that arrived
    /// while it was only looked at are not screened either, as those of a
    /// creation (`import-initial.md`, section 5). [`Store::set_screening_start`]
    /// sets it only once.
    pub fn reset_screening_start(&mut self, folder_id: i64, uid: u32) -> Result<()> {
        self.conn.execute("UPDATE folder SET uid_screening_start = ?2 WHERE id = ?1", params![folder_id, uid])?;
        Ok(())
    }

    /// The highest UIDs of a folder, at most `n`.
    pub fn top_uids(&self, folder_id: i64, n: u32) -> Result<Vec<u32>> {
        let mut stmt = self.conn.prepare_cached("SELECT uid FROM message WHERE folder_id = ?1 AND uid IS NOT NULL ORDER BY uid DESC LIMIT ?2")?;
        let rows = stmt.query_map(params![folder_id, n], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }

    /// The `authserv-id` detected for an account (`auth.rs`).
    pub fn account_authserv_id(&self, account_id: i64) -> Result<Option<String>> {
        Ok(self.conn.query_row("SELECT authserv_id FROM account WHERE id = ?1", [account_id], |r| r.get(0)).optional()?.flatten())
    }

    pub fn set_account_authserv_id(&mut self, account_id: i64, id: &str) -> Result<()> {
        self.conn.execute("UPDATE account SET authserv_id = ?2 WHERE id = ?1", params![account_id, id])?;
        Ok(())
    }

    /// What the account's server taught of the pace of its logins
    /// (`docs/sync.md`, "Pace of the connections"); all `None` for a
    /// server that never blocked a login.
    pub fn account_login_pace(&self, account_id: i64) -> Result<LoginPaceRow> {
        Ok(self
            .conn
            .query_row(
                "SELECT login_limit, login_blocked_at, login_pause_until, login_ok_at FROM account WHERE id = ?1",
                [account_id],
                |r| Ok(LoginPaceRow { limit: r.get(0)?, blocked_at: r.get(1)?, pause_until: r.get(2)?, ok_at: r.get(3)? }),
            )
            .optional()?
            .unwrap_or_default())
    }

    /// A block seen: the limit learnt, when, and the pause after it.
    /// `limit` `None` forgets what was learnt.
    pub fn set_account_login_block(&mut self, account_id: i64, limit: Option<i64>, blocked_at: Option<i64>, pause_until: Option<i64>) -> Result<()> {
        self.conn.execute(
            "UPDATE account SET login_limit = ?2, login_blocked_at = ?3, login_pause_until = ?4 WHERE id = ?1",
            params![account_id, limit, blocked_at, pause_until],
        )?;
        Ok(())
    }

    /// A new IMAP password was saved: it has not worked yet, so a refusal
    /// of it is a wrong password, not a lock; and a pause taken after a
    /// refusal of the old one ends (`docs/sync.md`, "Pace of the
    /// connections"). The limit learnt stays: it is the server's.
    pub fn login_password_changed(&mut self, account_id: i64) -> Result<()> {
        self.conn.execute("UPDATE account SET login_ok_at = NULL, login_pause_until = NULL WHERE id = ?1", [account_id])?;
        Ok(())
    }

    /// A login worked (`at`, Unix seconds); `None`: the password changed,
    /// and has not worked yet.
    pub fn set_account_login_ok(&mut self, account_id: i64, at: Option<i64>) -> Result<()> {
        self.conn.execute("UPDATE account SET login_ok_at = ?2 WHERE id = ?1", params![account_id, at])?;
        Ok(())
    }

    /// The synchronization of a folder starts: records its `UIDVALIDITY` if
    /// the database has none yet (first synchronization). From then on its
    /// mail counts as server mail (`refuse_imap_thread`).
    pub fn start_folder(&mut self, folder_id: i64, uidvalidity: u32) -> Result<()> {
        self.conn.execute("UPDATE folder SET uidvalidity = ?2 WHERE id = ?1 AND uidvalidity IS NULL", params![folder_id, uidvalidity])?;
        Ok(())
    }

    /// The messages of a folder, by UID, with their flags: what the
    /// comparison with the server needs on a server without QRESYNC.
    pub fn known_messages(&self, folder_id: i64) -> Result<Vec<KnownMessage>> {
        let mut keywords: HashMap<i64, Vec<String>> = HashMap::new();
        {
            let mut stmt = self.conn.prepare_cached(
                "SELECT k.message_id, k.keyword FROM message_keyword k JOIN message m ON m.id = k.message_id
                  WHERE m.folder_id = ?1 ORDER BY k.message_id, k.keyword COLLATE NOCASE",
            )?;
            for row in stmt.query_map([folder_id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))? {
                let (id, k) = row?;
                keywords.entry(id).or_default().push(k);
            }
        }
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, uid, is_seen, is_flagged, is_answered, is_draft, is_deleted FROM message
              WHERE folder_id = ?1 AND uid IS NOT NULL ORDER BY uid",
        )?;
        let rows = stmt.query_map([folder_id], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, u32>(1)?, Flags {
                seen: r.get(2)?,
                flagged: r.get(3)?,
                answered: r.get(4)?,
                draft: r.get(5)?,
                deleted: r.get(6)?,
            }))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, uid, flags) = row?;
            let mut kw = keywords.remove(&id).unwrap_or_default();
            kw.sort_by_key(|k| k.to_ascii_lowercase());
            out.push(KnownMessage { id, uid, flags: ServerFlags { flags, keywords: kw } });
        }
        Ok(out)
    }

    /// `UIDVALIDITY` changed: the UIDs of the folder name other messages
    /// now. Deletes every message of the folder (bodies, attachments,
    /// keywords go with them) and resets its state, so that the next steps
    /// fetch it again from the start (`modele de donnees.md`, section 5).
    pub fn purge_folder(&mut self, folder_id: i64, uidvalidity: u32) -> Result<(u32, ChangeSet)> {
        let tx = self.transaction()?;
        let mut changes = ChangeSet::default();
        let ids: Vec<(i64, Option<i64>)> = {
            let mut stmt = tx.prepare("SELECT id, thread_id FROM message WHERE folder_id = ?1")?;
            stmt.query_map([folder_id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<std::result::Result<_, _>>()?
        };
        let threads: BTreeSet<i64> = ids.iter().filter_map(|(_, t)| *t).collect();
        tx.execute("DELETE FROM message WHERE folder_id = ?1", [folder_id])?;
        // `uid_screening_start` too: what is there now counts as imported
        // (`import-initial.md`, section 5); the pass sets it again.
        tx.execute(
            "UPDATE folder SET uidvalidity = ?2, uidnext = NULL, highestmodseq = NULL, date_last_sync = NULL,
                    uid_screening_start = NULL WHERE id = ?1",
            params![folder_id, uidvalidity],
        )?;
        changes.folders.insert(folder_id);
        refresh_threads_after_delete(&tx, &threads, &mut changes)?;
        tx.commit()?;
        Ok((ids.len() as u32, changes))
    }

    /// Step 2 of a pass: a batch of new messages of `folder_id`, in UID
    /// order. Each is either a message moved there from another folder of
    /// the account, found in `deletions` (the UIDs that vanished in this
    /// pass, by folder), or a new row (headers only, its body to come).
    ///
    /// A move is recognized only when it is certain: same `Message-ID`,
    /// exactly one row of the account with it outside this folder, of the
    /// same size (or, once the body is stored, the same Subject and Date:
    /// Exchange announces another size than the bytes it sends), and whose
    /// UID vanished in this pass. The row then moves
    /// (`folder_id`, `uid`, flags) and keeps its id, thread, tags, history;
    /// its UID leaves `deletions`. Anything else is a new row: a copy (the
    /// original stayed), or a move that cannot be told from one.
    ///
    /// `announce`: whether the new rows are new mail for the user
    /// (`MessageArrived`), which the first synchronization of a folder is
    /// not (the whole folder would be "new").
    ///
    /// `uidnext` of the folder becomes the UID after the last one of the
    /// batch, in the same transaction.
    #[allow(clippy::too_many_arguments)]
    pub fn insert_arrivals(
        &mut self,
        inserter: &mut Inserter,
        account_id: i64,
        folder_id: i64,
        batch: &[Arrival],
        deletions: &mut BTreeMap<i64, BTreeSet<u32>>,
        rc: &crate::reception::Reception,
    ) -> Result<(Arrived, ChangeSet)> {
        let keywords_local = rc.keywords_local;
        let mut changes = ChangeSet::default();
        let mut arrived = Arrived::default();
        let tx = self.transaction()?;
        let result = (|| -> Result<()> {
            let mut threads = BTreeSet::new();
            for a in batch {
                // Already known here: a message NiceMailer moved here, whose
                // UID the server gave (`COPYUID`), past the folder's old
                // `uidnext`. Nothing new; and never two rows for one UID.
                let known = tx
                    .prepare_cached("SELECT 1 FROM message WHERE folder_id = ?1 AND uid = ?2")?
                    .exists(params![folder_id, a.uid])?;
                if known {
                    continue;
                }
                // A row of this folder without UID, with the same
                // Message-ID: moved here by NiceMailer, by a server without
                // UIDPLUS (no `COPYUID`), or by the phone when this row's
                // own move found it gone. It gets its UID; no new row.
                if let Some((id, thread)) = attach_here(&tx, folder_id, a)? {
                    tx.prepare_cached("UPDATE message SET uid = ?2, modseq = ?3 WHERE id = ?1")?
                        .execute(params![id, a.uid, a.modseq.map(|m| m as i64)])?;
                    if !pending_flags(&tx, id)? {
                        write_flags(&tx, id, &a.flags, false, keywords_local)?;
                    }
                    changes.messages.insert((id, thread));
                    arrived.attached += 1;
                    continue;
                }
                if let Some(MovedFrom { id, folder: from, uid: from_uid, thread }) = moved_here(&tx, account_id, folder_id, a, deletions)? {
                    tx.prepare_cached(
                        "UPDATE message SET folder_id = ?2, uid = ?3, modseq = ?4, date_modified = unixepoch(), modified_by = 'imap'
                          WHERE id = ?1",
                    )?
                    .execute(params![id, folder_id, a.uid, a.modseq.map(|m| m as i64)])?;
                    // No `audit_log` row: changes seen on the server are not
                    // journaled (`tracabilite.md`, "Ce qui n'est pas
                    // journalisé"); `modified_by = 'imap'` says where it came from.
                    write_flags(&tx, id, &a.flags, false, keywords_local)?;
                    if let Some(set) = deletions.get_mut(&from) {
                        set.remove(&from_uid);
                    }
                    // A mail marked as back from Todo or Waiting that left
                    // the Inbox elsewhere loses its mark, else it would show
                    // again, wrongly, if it came back one day (`dossiers.md`).
                    let from_inbox: bool = tx
                        .prepare_cached("SELECT role = 'inbox' FROM folder WHERE id = ?1")?
                        .query_row([from], |r| r.get::<_, Option<bool>>(0))?
                        .unwrap_or(false);
                    if from_inbox && rc.role != "inbox" {
                        crate::reception::clear_return_marks(&tx, &[id], nm_core::Source::Core, &mut changes)?;
                    }
                    changes.folders.insert(from);
                    changes.messages.insert((id, thread));
                    threads.extend(thread);
                    arrived.moved += 1;
                    continue;
                }
                let m = NewMessage {
                    account_id,
                    folder_id,
                    uid: Some(a.uid),
                    date_received: a.date_received,
                    size: a.size,
                    modseq: a.modseq,
                    flags: a.flags.flags,
                    auth: crate::reception::verdict_of(a, rc),
                    headers: &a.headers,
                };
                let id = inserter.insert(&tx, &m, None, &mut changes)?;
                write_flags(&tx, id, &a.flags, true, false)?;
                // Screening, return of a conversation, effects of Sent
                // (`reception.rs`), in this same transaction.
                let to = crate::reception::after_insert(&tx, rc, account_id, folder_id, id, a, m.auth.0, &mut changes, &mut threads)?;
                if to != folder_id {
                    arrived.screened += 1;
                }
                if rc.announce {
                    changes.arrived.insert((id, to));
                }
                arrived.inserted += 1;
            }
            if let Some(last) = batch.last() {
                tx.prepare_cached("UPDATE folder SET uidnext = max(coalesce(uidnext, 0), ?2) WHERE id = ?1")?
                    .execute(params![folder_id, last.uid as i64 + 1])?;
            }
            changes.folders.insert(folder_id);
            // The threads of moved rows: their folders changed. The new
            // rows' threads are refreshed by the inserter's commit.
            for t in &threads {
                let folders = crate::refresh_thread(&tx, *t)?;
                changes.folders.extend(folders.iter().copied());
                changes.threads.push((*t, folders));
            }
            Ok(())
        })();
        match result {
            Ok(()) => inserter.commit(tx, &mut changes)?,
            Err(e) => {
                inserter.abandon();
                return Err(e);
            }
        }
        Ok((arrived, changes))
    }

    /// Step 3 of a pass: the flags that changed, the messages that vanished
    /// (those not recognized as moved by [`Store::insert_arrivals`]), and
    /// the folder's new state, in one transaction.
    pub fn finish_folder(
        &mut self,
        folder_id: i64,
        flags: &[FlagUpdate],
        deleted: &BTreeSet<u32>,
        state: FolderState,
        keywords_local: bool,
    ) -> Result<ChangeSet> {
        Ok(self.finish_folder_counted(folder_id, flags, deleted, state, keywords_local)?.0)
    }

    /// [`Store::finish_folder`], with the number of rows deleted: the UIDs
    /// gone include those of messages NiceMailer itself moved away, whose
    /// rows already followed them.
    pub fn finish_folder_counted(
        &mut self,
        folder_id: i64,
        flags: &[FlagUpdate],
        deleted: &BTreeSet<u32>,
        state: FolderState,
        keywords_local: bool,
    ) -> Result<(ChangeSet, u32)> {
        let tx = self.transaction()?;
        let mut changes = ChangeSet::default();
        let mut threads = BTreeSet::new();
        for u in flags {
            let row: Option<(i64, Option<i64>)> = tx
                .prepare_cached("SELECT id, thread_id FROM message WHERE folder_id = ?1 AND uid = ?2")?
                .query_row(params![folder_id, u.uid], |r| Ok((r.get(0)?, r.get(1)?)))
                .optional()?;
            // A UID unknown here: arrived after the discovery, it will come
            // with the next pass's arrivals.
            let Some((id, thread)) = row else { continue };
            if let Some(m) = u.modseq {
                tx.prepare_cached("UPDATE message SET modseq = ?2 WHERE id = ?1")?.execute(params![id, m as i64])?;
            }
            // A flag changed in NiceMailer and not sent yet: the local
            // value stays until the spool sends it (`modele de donnees.md`,
            // section 6, "Synchro et spool").
            if pending_flags(&tx, id)? {
                continue;
            }
            if write_flags(&tx, id, &u.flags, false, keywords_local)? {
                changes.messages.insert((id, thread));
                threads.extend(thread);
            }
        }
        let mut gone = 0i64;
        for uid in deleted {
            let row: Option<(i64, Option<i64>)> = tx
                .prepare_cached("DELETE FROM message WHERE folder_id = ?1 AND uid = ?2 RETURNING id, thread_id")?
                .query_row(params![folder_id, uid], |r| Ok((r.get(0)?, r.get(1)?)))
                .optional()?;
            if let Some((id, thread)) = row {
                changes.messages.insert((id, thread));
                threads.extend(thread);
                gone += 1;
            }
        }
        // Rows of this folder still without UID and without action pending:
        // their move was done, but the message was not found here (moved
        // or deleted elsewhere meanwhile). It shows again through the
        // folder it really is in (`modele de donnees.md`, section 6,
        // "Message disparu du serveur").
        //
        // Except a mail NiceMailer sent less than an hour ago (`created_by`
        // a client, not `imap`): its copy in Sent may not be there yet (a
        // provider that copies it itself, a server without UIDPLUS); the
        // pass that sees it attaches the row by its Message-ID
        // (`reponse.md`, section 9).
        {
            let orphans: Vec<(i64, Option<i64>)> = tx
                .prepare_cached(
                    "DELETE FROM message WHERE folder_id = ?1 AND uid IS NULL
                       AND NOT EXISTS (SELECT 1 FROM pending_action a WHERE a.message_id = message.id AND a.state IN ('queued', 'running'))
                       AND NOT (created_by <> 'imap' AND date_created > unixepoch() - 3600)
                     RETURNING id, thread_id",
                )?
                .query_map([folder_id], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<std::result::Result<_, _>>()?;
            for (id, thread) in orphans {
                changes.messages.insert((id, thread));
                threads.extend(thread);
            }
        }
        tx.execute(
            "UPDATE folder SET uidvalidity = ?2, uidnext = coalesce(?3, uidnext), highestmodseq = ?4, date_last_sync = unixepoch()
              WHERE id = ?1",
            params![folder_id, state.uidvalidity, state.uidnext, state.highestmodseq.map(|m| m as i64)],
        )?;
        changes.folders.insert(folder_id);
        refresh_threads_after_delete(&tx, &threads, &mut changes)?;
        // Drafts whose copy went from the server, not changed here since:
        // deleted or sent elsewhere (`draft_copies.rs`).
        if gone > 0 {
            crate::draft_copies::gone_elsewhere(&tx, &mut changes)?;
        } else {
            // Every thread whose flags changed is in `threads`, now in
            // `changes.threads`: the folders may be reloaded by
            // conversation (`ChangeSet::announced`). Not after mails
            // vanished: the drafts above may change folders without
            // naming their conversations, and an announced folder must
            // hold no change left unsaid.
            changes.announce_threads();
        }
        tx.commit()?;
        Ok((changes, gone as u32))
    }

    /// The messages of an account still without body, in the order they
    /// are downloaded (`plan-imap.md`, section 2, "Corps"): the Inbox
    /// first, then the other folders, the most recent first.
    pub fn bodies_missing(&self, account_id: i64) -> Result<Vec<MissingBody>> {
        let mut stmt = self.conn.prepare(
            "SELECT m.id, m.folder_id, f.path, m.uid, m.size FROM message m JOIN folder f ON f.id = m.folder_id
              WHERE m.account_id = ?1 AND m.body_state = 0 AND m.uid IS NOT NULL AND f.role IS NOT NULL
              ORDER BY f.role <> 'inbox', m.date_received DESC, m.id DESC",
        )?;
        let rows = stmt
            .query_map([account_id], |r| {
                Ok(MissingBody { id: r.get(0)?, folder_id: r.get(1)?, path: r.get(2)?, uid: r.get(3)?, size: r.get(4)? })
            })?
            .collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }

    /// One message still without body, by id (a message the user opened).
    pub fn missing_body(&self, message_id: i64) -> Result<Option<(i64, MissingBody)>> {
        Ok(self
            .conn
            .query_row(
                "SELECT m.account_id, m.id, m.folder_id, f.path, m.uid, m.size FROM message m JOIN folder f ON f.id = m.folder_id
                  WHERE m.id = ?1 AND m.body_state = 0 AND m.uid IS NOT NULL",
                [message_id],
                |r| Ok((r.get(0)?, MissingBody { id: r.get(1)?, folder_id: r.get(2)?, path: r.get(3)?, uid: r.get(4)?, size: r.get(5)? })),
            )
            .optional()?)
    }

    /// The UIDs of a folder that moves of the spool, not sent yet, take
    /// away: the server still shows them there, the database already has
    /// the message elsewhere. The synchronization ignores them, rather than
    /// insert the message again (`modele de donnees.md`, section 6).
    pub fn pending_source_uids(&self, folder_id: i64) -> Result<BTreeSet<u32>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT json_extract(payload, '$.from_uid') FROM pending_action
              WHERE kind = 'move' AND state IN ('queued', 'running') AND json_extract(payload, '$.from_folder_id') = ?1
                AND json_extract(payload, '$.from_uid') IS NOT NULL",
        )?;
        let rows = stmt.query_map([folder_id], |r| r.get::<_, i64>(0))?;
        let mut out = BTreeSet::new();
        for r in rows {
            out.insert(r? as u32);
        }
        Ok(out)
    }

    /// The complete messages of a folder with their raw file, by UID, and
    /// the number still without body (`sync verify`).
    pub fn folder_bodies(&self, folder_id: i64) -> Result<(Vec<(u32, crate::raw::Hash)>, u64)> {
        let mut stmt = self.conn.prepare(
            "SELECT uid, raw_hash FROM message WHERE folder_id = ?1 AND uid IS NOT NULL AND body_state = 1 ORDER BY uid",
        )?;
        let rows: Vec<(u32, Vec<u8>)> = stmt.query_map([folder_id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<std::result::Result<_, _>>()?;
        let mut out = Vec::with_capacity(rows.len());
        for (uid, h) in rows {
            if let Ok(h) = <crate::raw::Hash>::try_from(h.as_slice()) {
                out.push((uid, h));
            }
        }
        let missing: u64 =
            self.conn.query_row("SELECT count(*) FROM message WHERE folder_id = ?1 AND body_state = 0", [folder_id], |r| r.get(0))?;
        Ok((out, missing))
    }

    /// The role folders of an account as stored, Inbox first, without
    /// asking the server (`sync status`, `sync verify`).
    pub fn account_sync_folders(&self, account_id: i64) -> Result<Vec<(SyncFolder, Option<i64>, u64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT f.id, f.path, f.role, f.uidvalidity, f.uidnext, f.highestmodseq,
                    (SELECT count(*) FROM message m WHERE m.folder_id = f.id), f.date_last_sync IS NOT NULL, f.date_last_sync,
                    (SELECT count(*) FROM message m WHERE m.folder_id = f.id AND m.body_state = 0), f.uid_screening_start
               FROM folder f WHERE f.account_id = ?1 AND f.role IS NOT NULL AND f.role <> 'config'",
        )?;
        let mut out: Vec<(SyncFolder, Option<i64>, u64)> = stmt
            .query_map([account_id], |r| {
                Ok((
                    SyncFolder {
                        id: r.get(0)?,
                        path: r.get(1)?,
                        role: r.get(2)?,
                        uidvalidity: r.get(3)?,
                        uidnext: r.get(4)?,
                        highestmodseq: r.get::<_, Option<i64>>(5)?.map(|m| m as u64),
                        messages: r.get(6)?,
                        synced: r.get(7)?,
                        screening_start: r.get(10)?,
                    },
                    r.get(8)?,
                    r.get(9)?,
                ))
            })?
            .collect::<std::result::Result<_, _>>()?;
        let rank = |role: &str| ROLE_ORDER.iter().position(|r| *r == role).unwrap_or(ROLE_ORDER.len());
        out.sort_by(|a, b| (rank(&a.0.role), &a.0.path).cmp(&(rank(&b.0.role), &b.0.path)));
        Ok(out)
    }

    /// The messages of a thread still without body, with their account:
    /// opened by the user, they are downloaded first.
    pub fn thread_bodies_missing(&self, thread_id: i64) -> Result<Vec<(i64, i64)>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT account_id, id FROM message WHERE thread_id = ?1 AND body_state = 0 AND uid IS NOT NULL",
        )?;
        let rows = stmt.query_map([thread_id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<std::result::Result<_, _>>()?;
        Ok(rows)
    }

    /// The account of an IMAP account key, if the database has it.
    pub fn imap_account_id(&self, key: &str) -> Result<Option<i64>> {
        Ok(self
            .conn
            .query_row("SELECT id FROM account WHERE config_key = ?1 AND kind = 'imap'", [key], |r| r.get(0))
            .optional()?)
    }
}

/// The pace of the logins learnt from an account's server (`account`,
/// version 16), Unix seconds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LoginPaceRow {
    /// Logins per hour at most; `None`: no limit.
    pub limit: Option<i64>,
    pub blocked_at: Option<i64>,
    /// No login before.
    pub pause_until: Option<i64>,
    /// The last login that worked.
    pub ok_at: Option<i64>,
}

/// A message without body, to download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingBody {
    pub id: i64,
    pub folder_id: i64,
    pub path: String,
    pub uid: u32,
    pub size: Option<i64>,
}

/// The role folders listed by the server, in synchronization order.
fn role_folders(tx: &Transaction, account_id: i64, listed: &BTreeSet<&str>, change: &FolderListChange) -> Result<Vec<SyncFolder>> {
    let mut stmt = tx.prepare(
        "SELECT f.id, f.path, f.role, f.uidvalidity, f.uidnext, f.highestmodseq,
                f.date_last_sync IS NOT NULL, f.uid_screening_start
           FROM folder f WHERE f.account_id = ?1 AND f.role IS NOT NULL AND f.role <> 'config'",
    )?;
    let mut out: Vec<SyncFolder> = stmt
        .query_map([account_id], |r| {
            Ok(SyncFolder {
                id: r.get(0)?,
                path: r.get(1)?,
                role: r.get(2)?,
                uidvalidity: r.get(3)?,
                uidnext: r.get(4)?,
                highestmodseq: r.get::<_, Option<i64>>(5)?.map(|m| m as u64),
                // Counted without the lock (`Store::fill_message_counts`).
                messages: 0,
                synced: r.get(6)?,
                screening_start: r.get(7)?,
            })
        })?
        .collect::<std::result::Result<_, _>>()?;
    out.retain(|f| listed.contains(f.path.as_str()) || change.renamed.iter().any(|(_, new)| *new == f.path));
    let rank = |role: &str| ROLE_ORDER.iter().position(|r| *r == role).unwrap_or(ROLE_ORDER.len());
    out.sort_by(|a, b| (rank(&a.role), &a.path).cmp(&(rank(&b.role), &b.path)));
    Ok(out)
}

/// The row a message moved elsewhere was (see [`moved_here`]).
struct MovedFrom {
    id: i64,
    folder: i64,
    uid: u32,
    thread: Option<i64>,
}

/// A row with a given `Message-ID`: id, folder, UID, size, thread, whether
/// it has its body, subject, `Date` header.
type Candidate = (i64, i64, Option<u32>, Option<i64>, Option<i64>, bool, String, Option<i64>);

/// A move made elsewhere into `folder_id`: the row it was, if certain (see
/// [`Store::insert_arrivals`]).
fn moved_here(
    tx: &Transaction,
    account_id: i64,
    folder_id: i64,
    a: &Arrival,
    deletions: &BTreeMap<i64, BTreeSet<u32>>,
) -> Result<Option<MovedFrom>> {
    let Some(msgid) = a.headers.message_id.as_deref() else { return Ok(None) };
    let rows: Vec<Candidate> = tx
        .prepare_cached(
            "SELECT id, folder_id, uid, size, thread_id, body_state = 1, subject, date_sent FROM message
              WHERE account_id = ?1 AND message_id_hdr = ?2 AND folder_id <> ?3 LIMIT 2",
        )?
        .query_map(params![account_id, msgid, folder_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?))
        })?
        .collect::<std::result::Result<_, _>>()?;
    // Rust note: a slice pattern; `ref` borrows the subject out of the
    // vector instead of moving it (a `String` is not `Copy`).
    let [(id, from, Some(uid), size, thread, has_body, ref subject, date_sent)] = rows[..] else { return Ok(None) };
    // The size tells a different mail that has the same Message-ID (the
    // copy a mailing list sends back, with its footer). The size of the
    // row is RFC822.SIZE while its body is missing, then the bytes
    // stored. Those are what a server that counts right announces, but
    // not Exchange Online, which announces another size for almost every
    // mail (`docs/sync.md`, "Bodies"): every move made on the phone
    // would then be a deletion and a new mail, which loses the tags and
    // the history and downloads the body again. For a row with its body,
    // a size that differs is therefore not enough to refuse: the Subject
    // and the Date headers decide, which a list that changes the mail
    // usually changes too (a `[list]` prefix).
    if size.map(|s| s as u64) != a.size && !(has_body && *subject == a.headers.subject && date_sent == a.headers.date_sent) {
        return Ok(None);
    }
    if !deletions.get(&from).is_some_and(|d| d.contains(&uid)) {
        return Ok(None);
    }
    Ok(Some(MovedFrom { id, folder: from, uid, thread }))
}

/// A row of `folder_id` without UID with the `Message-ID` of `a`, the
/// oldest first: `(id, thread)`.
fn attach_here(tx: &Transaction, folder_id: i64, a: &Arrival) -> Result<Option<(i64, Option<i64>)>> {
    let Some(msgid) = a.headers.message_id.as_deref() else { return Ok(None) };
    Ok(tx
        .prepare_cached("SELECT id, thread_id FROM message WHERE folder_id = ?1 AND uid IS NULL AND message_id_hdr = ?2 ORDER BY id LIMIT 1")?
        .query_row(params![folder_id, msgid], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?)
}

/// Whether a message has a flag change not sent yet.
fn pending_flags(tx: &Transaction, id: i64) -> Result<bool> {
    Ok(tx
        .prepare_cached("SELECT EXISTS (SELECT 1 FROM pending_action WHERE message_id = ?1 AND kind = 'set_flags' AND state IN ('queued', 'running'))")?
        .query_row([id], |r| r.get(0))?)
}

/// Writes the flags and keywords of a message; returns whether anything
/// changed. A change from the server is a modification of the mail
/// (`tracabilite.md`): `date_modified`, `modified_by = 'imap'`, unless
/// `new` (a row just inserted, already dated).
///
/// `keywords_local`: the folder keeps no keyword (`PERMANENTFLAGS` without
/// `\*`): tags and marks stay local, the server's (empty) list of
/// keywords does not replace them.
fn write_flags(tx: &Transaction, id: i64, f: &ServerFlags, new: bool, keywords_local: bool) -> Result<bool> {
    let n = tx
        .prepare_cached(
            "UPDATE message SET is_seen = ?2, is_flagged = ?3, is_answered = ?4, is_draft = ?5, is_deleted = ?6,
                    date_modified = CASE WHEN ?7 THEN date_modified ELSE unixepoch() END,
                    modified_by = CASE WHEN ?7 THEN modified_by ELSE 'imap' END
              WHERE id = ?1 AND (is_seen, is_flagged, is_answered, is_draft, is_deleted) IS NOT (?2, ?3, ?4, ?5, ?6)",
        )?
        .execute(params![id, f.flags.seen, f.flags.flagged, f.flags.answered, f.flags.draft, f.flags.deleted, new])?;
    let current: Vec<String> = tx
        .prepare_cached("SELECT keyword FROM message_keyword WHERE message_id = ?1 ORDER BY keyword COLLATE NOCASE")?
        .query_map([id], |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    let same = keywords_local
        || current.len() == f.keywords.len() && current.iter().zip(&f.keywords).all(|(a, b)| a.eq_ignore_ascii_case(b));
    if !same {
        tx.prepare_cached("DELETE FROM message_keyword WHERE message_id = ?1")?.execute([id])?;
        for k in &f.keywords {
            tx.prepare_cached("INSERT OR IGNORE INTO message_keyword (message_id, keyword) VALUES (?1, ?2)")?
                .execute(params![id, k])?;
        }
        if !new {
            tx.prepare_cached("UPDATE message SET date_modified = unixepoch(), modified_by = 'imap' WHERE id = ?1")?
                .execute([id])?;
        }
    }
    Ok(n > 0 || !same)
}

/// Threads whose messages were deleted or changed: their aggregates
/// recomputed, and those left empty deleted.
fn refresh_threads_after_delete(tx: &Transaction, threads: &BTreeSet<i64>, changes: &mut ChangeSet) -> Result<()> {
    refresh_all(tx, threads, changes)?;
    for t in threads {
        let empty: bool = tx.query_row("SELECT NOT EXISTS (SELECT 1 FROM message WHERE thread_id = ?1)", [t], |r| r.get(0))?;
        if empty {
            tx.execute("DELETE FROM thread WHERE id = ?1", [t])?;
        }
    }
    Ok(())
}

/// `Source` of the synchronization, for the rows it creates.
pub const SOURCE: Source = Source::Imap;
