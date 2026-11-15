//! Read queries: display-ready views, paginated by cursor.
//!
//! These methods take `&self` (shared access): they never modify the
//! database.
//!
//! Every thread list reads the table `thread` alone (`schema.sql`, version
//! 9): its folder, flags and counters are columns, and each kind of list
//! walks one of its indexes, newest first, and stops after a page.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use nm_api::{AddressView, FolderView, MessageView, SearchHit, SegmentView, ThreadFilter, ThreadSort, ThreadSummary};
use nm_core::{FolderRole, KW_WAS_TODO, KW_WAS_WAITING, SenderKind};
use rusqlite::types::Value;
use rusqlite::{OptionalExtension, Row, params};

use crate::{Result, Store, StoreError};

/// A row of a list query: (thread id, subject, last received, last
/// modified, message count, unread count, folder code, account).
type ListRow = (i64, String, i64, i64, i64, i64, Option<String>, i64);

/// The mails of a thread, oldest first, with the role of their folder: the
/// details of a list row (`thread_summary`). Found through
/// `idx_message_thread (thread_id, …)`; the sort is of a handful of rows.
const THREAD_MESSAGES: &str = "SELECT m.from_name, m.from_addr, m.snippet, m.has_attachments, f.role, m.id, m.sender_kind
       FROM message m JOIN folder f ON f.id = m.folder_id
      WHERE m.thread_id = ?1
      ORDER BY m.date_received, m.id";

/// The first recipient of a mail: To before Cc before Bcc, in header
/// order (the correspondent of a conversation the user wrote alone,
/// `thread_summary`). Found through the primary key of `message_address`
/// `(message_id, …)`; the sort is of a handful of rows.
const FIRST_RECIPIENT: &str = "SELECT a.name, a.addr FROM message_address ma
       JOIN address a ON a.id = ma.address_id
      WHERE ma.message_id = ?1 AND ma.role IN ('to', 'cc', 'bcc')
      ORDER BY ma.role = 'to' DESC, ma.role = 'cc' DESC, ma.position
      LIMIT 1";

/// Whether a thread has a flagged message, in any folder (read from the
/// partial index `idx_message_thread_flagged`).
const THREAD_IS_FLAGGED: &str = "SELECT EXISTS (SELECT 1 FROM message WHERE thread_id = ?1 AND is_flagged = 1)";

/// The condition of All Mail, written exactly as the `WHERE` of its partial
/// indexes (`idx_thread_live_received`…, `schema.sql`): SQLite uses a
/// partial index only when the query holds its condition, and compares the
/// expressions, not their meaning.
const ALL_MAIL: &str = "t.folder NOT IN ('d','x','b')";

/// The condition of Work (Inbox, Todo, Waiting together), written exactly
/// as the `WHERE` of its partial indexes (`idx_thread_work_received`…),
/// for the same reason as `ALL_MAIL`. With `folder IN (…)` alone, SQLite
/// would read the three ranges of `idx_thread_role_received` and sort them.
const WORK: &str = "t.folder IN ('i','t','w')";

/// Parses a list cursor: `<date>:<thread_id>` of the last row of the
/// previous page, the date being the sort key.
///
/// Why a cursor rather than `OFFSET`: with `OFFSET 500`, SQLite still reads
/// and skips 500 rows, and if a thread arrives meanwhile, every row shifts
/// and the user sees a duplicate. A cursor says "continue after this row",
/// which is both fast (the index jumps straight there) and stable.
/// The thread id breaks ties between threads with the same date.
fn parse_cursor(cursor: &str) -> Result<(i64, i64)> {
    // `split_once` returns `Some((before, after))` or `None` if there is no ':'.
    let (d, id) = cursor
        .split_once(':')
        .ok_or_else(|| StoreError::InvalidParams(format!("curseur invalide : {cursor}")))?;
    // A closure stored in a variable, used twice below. It captures `cursor`
    // from the surrounding scope to build the error message.
    let parse = |s: &str| {
        s.parse::<i64>()
            .map_err(|_| StoreError::InvalidParams(format!("curseur invalide : {cursor}")))
    };
    Ok((parse(d)?, parse(id)?))
}

/// Converts the nullable `role` column into a `FolderRole`. Unknown roles
/// become `None` rather than an error, so a newer database does not break
/// the listing.
fn role_of(s: Option<String>) -> Option<FolderRole> {
    s.as_deref().and_then(FolderRole::parse)
}

/// A folder in `thread_counts`: (account, role, 0), or (account, `None`,
/// folder id) for a folder without a role.
type FolderKey = (i64, Option<FolderRole>, i64);

/// The column of `thread` flagging a technical folder (`schema.sql`).
fn flag_column(role: FolderRole) -> &'static str {
    match role {
        FolderRole::Sent => "has_sent",
        FolderRole::Drafts => "has_draft",
        FolderRole::Quarantine => "has_quarantine",
        // Only the four technical roles reach here (`Scope::of`).
        _ => "has_deleted",
    }
}

/// Which threads a list reads from `thread`.
///
/// Rust note: `#[derive(Clone, Copy)]` lets a small value be copied freely
/// instead of moved; `PartialEq, Eq, PartialOrd, Ord` let it be a key of a
/// `BTreeMap` (the groups of `list_threads_in`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Scope {
    /// A real folder (Inbox… Banned): `folder = '<code>'`.
    Role(FolderRole),
    /// A technical folder (Sent, Drafts, Quarantine, Deleted): its flag.
    Flag(FolderRole),
    /// All Mail: every thread but those in Deleted, Junk and Banned.
    AllMail,
    /// Work: the threads of Inbox, Todo and Waiting.
    Work,
    /// Everything: every thread with a mail.
    Everything,
    /// A folder without a role: the threads with a mail in it. Such a
    /// folder is not synchronized (`dossiers.md`), so it only holds mail
    /// left there when it lost its role; the list reads its mails, a small
    /// set, and walks the Everything index.
    Unmanaged(i64),
}

impl Scope {
    /// The scope of a folder of this role.
    fn of(role: FolderRole) -> Scope {
        if role.is_technical() { Scope::Flag(role) } else { Scope::Role(role) }
    }

    /// The SQL condition. The codes and column names are constants of
    /// ours, and the folder id an integer from the store: no client input
    /// is written into the SQL text.
    fn condition(self) -> String {
        match self {
            Scope::Role(r) => format!("t.folder = '{}'", r.code()),
            Scope::Flag(r) => format!("t.{} = 1", flag_column(r)),
            Scope::AllMail => ALL_MAIL.to_string(),
            Scope::Work => WORK.to_string(),
            Scope::Everything => "t.message_count > 0".to_string(),
            Scope::Unmanaged(f) => {
                format!("t.id IN (SELECT thread_id FROM message WHERE folder_id = {f} AND thread_id IS NOT NULL)")
            }
        }
    }
}

/// The accounts a list covers.
#[derive(Debug, Clone, PartialEq)]
enum Accounts {
    /// Every account: no condition, the "all accounts" index.
    All,
    /// One account: the "one account" index (`account_id = ?1`).
    One(i64),
    /// Several accounts, not all of them (the unified view without a
    /// disabled account): the "all accounts" index walked, the others
    /// skipped (comma-separated ids).
    Some(String),
}

impl Store {
    /// Every folder shown in the sidebar, with its counters and the move
    /// targets it offers.
    pub fn list_folders(&self) -> Result<Vec<FolderView>> {
        // The `config` folder stores the synced configuration
        // (`synchronisation-configuration.md`); it is technical and hidden.
        let mut stmt = self.conn.prepare_cached(
            "SELECT f.id, f.role, f.path, f.total_count, f.unread_count,
                    a.id, a.config_key, a.kind, a.position
               FROM folder f JOIN account a ON a.id = f.account_id
              WHERE f.subscribed = 1 AND (f.role IS NULL OR f.role <> 'config')",
        )?;
        // Each folder with the position of its account, for the sort below.
        let mut items = stmt
            .query_map([], |r| {
                let position: i64 = r.get(8)?;
                let role = role_of(r.get(1)?);
                let path: String = r.get(2)?;
                Ok((position, FolderView {
                    id: r.get(0)?,
                    role,
                    // Workflow folders use their standard name; any other
                    // folder falls back to its IMAP path.
                    name: role.map(|r| r.display_name().to_string()).unwrap_or(path),
                    letter: role.map(|r| r.letter().to_string()),
                    category: role.map(|r| r.category()),
                    total_count: r.get(3)?,
                    unread_count: r.get(4)?,
                    // `unwrap_or_default` gives an empty `Vec` when there is no role.
                    move_targets: role.map(|r| r.move_targets()).unwrap_or_default(),
                    account_id: r.get(5)?,
                    account_name: r.get(6)?,
                    account_kind: r.get(7)?,
                }))
            })?
            .collect::<std::result::Result<Vec<(i64, FolderView)>, _>>()?;
        // Grouped by account, in the order of `account.position` (the order
        // of the account picker, `set_account_order`; the oldest account
        // first on a tie), then workflow folders in
        // the order of the spec, then the others by name. `sort_by_key`
        // sorts on a computed key; tuples compare element by element, so the
        // rank of the account decides first, then the place of the folder's
        // role, and the name breaks ties. Folders without a role get `usize::MAX` and end up last.
        items.sort_by_key(|(position, f)| {
            (
                *position,
                f.account_id,
                f.role
                    .and_then(|r| FolderRole::ALL.iter().position(|x| *x == r))
                    .unwrap_or(usize::MAX),
                f.name.clone(),
            )
        });
        // The number of conversations of each folder is not sent: no
        // interface shows it, and counting them read the list index of
        // every conversation of the mailbox, the longest part of this
        // request (0.6 to 1.4 s just after the core started, on a real
        // mailbox), at every refresh of the sidebar.
        Ok(items.into_iter().map(|(_, f)| f).collect())
    }

    /// The number of conversations listed in the folder `folder_id`, by
    /// the rule of the lists (`dossiers.md`, "Dossier d'une conversation"):
    /// for the tests that check the lists against it. No request sends it.
    pub fn folder_thread_count(&self, folder_id: i64) -> Result<i64> {
        let (role, account) = self.folder_role_and_account(folder_id)?;
        let key = match role {
            Some(r) => (account, Some(r), 0),
            None => (account, None, folder_id),
        };
        Ok(self.thread_counts()?.get(&key).copied().unwrap_or(0))
    }

    /// The number of conversations of each folder, keyed by (account, role,
    /// 0), or (account, `None`, folder id) for a folder without a role: the
    /// threads whose `folder` is the role, or whose flag is set for a
    /// technical folder (`dossiers.md`, "Dossier d'une conversation").
    ///
    /// One pass over each index rather than a count per folder: a
    /// `GROUP BY` on the covering index of the real folders, and one on the
    /// small partial index of each flag.
    fn thread_counts(&self) -> Result<HashMap<FolderKey, i64>> {
        let mut out = HashMap::new();
        let mut stmt = self.conn.prepare_cached(
            "SELECT account_id, folder, count(*) FROM thread WHERE folder IS NOT NULL GROUP BY folder, account_id",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))?;
        for row in rows {
            let (account, code, n) = row?;
            // A technical code is the fallback folder of a thread; its list
            // is that of the flag, counted below.
            if let Some(role) = FolderRole::from_code(&code).filter(|r| !r.is_technical()) {
                out.insert((account, Some(role), 0), n);
            }
        }
        for role in FolderRole::TECHNICAL {
            let sql = format!(
                "SELECT account_id, count(*) FROM thread WHERE {} = 1 GROUP BY account_id",
                flag_column(role)
            );
            let mut stmt = self.conn.prepare_cached(&sql)?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?;
            for row in rows {
                let (account, n) = row?;
                out.insert((account, Some(role), 0), n);
            }
        }
        // Folders without a role: normally empty (they are not synchronized).
        let mut stmt = self.conn.prepare_cached(
            "SELECT f.account_id, f.id, (SELECT count(DISTINCT m.thread_id) FROM message m WHERE m.folder_id = f.id)
               FROM folder f WHERE f.role IS NULL",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)))?;
        for row in rows {
            let (account, folder, n) = row?;
            out.insert((account, None, folder), n);
        }
        Ok(out)
    }

    /// Every folder with a role, by (account, role): the folder a thread
    /// of Everything or All Mail is shown in, from its `folder` code.
    fn role_folders(&self) -> Result<HashMap<(i64, FolderRole), i64>> {
        let mut stmt = self.conn.prepare_cached("SELECT account_id, role, id FROM folder WHERE role IS NOT NULL")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))?;
        let mut out = HashMap::new();
        for row in rows {
            let (account, role, id) = row?;
            if let Some(role) = FolderRole::parse(&role) {
                out.insert((account, role), id);
            }
        }
        Ok(out)
    }

    /// Ids of the active tags named like the `tag` criterion of `filter`,
    /// in every account (each account has its own tags, and the unified
    /// view shows "Clients" once). Names are compared in Rust, without
    /// case: SQLite's NOCASE only folds ASCII, so "Équipe" and "équipe"
    /// would differ (the same rule as `tag_thread`). An account has few
    /// tags, reading them all is cheap. Empty without a `tag` criterion.
    fn tag_ids(&self, filter: &ThreadFilter) -> Result<Vec<i64>> {
        let Some(wanted) = filter.tag.as_deref().map(|t| t.trim().to_lowercase()) else { return Ok(vec![]) };
        let mut stmt = self.conn.prepare_cached("SELECT id, name FROM tag WHERE date_deleted IS NULL")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        let mut ids = Vec::new();
        for row in rows {
            let (id, name) = row?;
            if name.to_lowercase() == wanted {
                ids.push(id);
            }
        }
        Ok(ids)
    }

    /// Role and account of a folder, or `NotFound` if the folder does not
    /// exist.
    fn folder_role_and_account(&self, folder_id: i64) -> Result<(Option<FolderRole>, i64)> {
        let row: Option<(Option<String>, i64)> = self
            .conn
            .query_row("SELECT role, account_id FROM folder WHERE id = ?1", [folder_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        match row {
            None => Err(StoreError::NotFound(format!("dossier {folder_id}"))),
            Some((r, account)) => Ok((role_of(r), account)),
        }
    }

    /// One page of a folder's threads, newest first, by reception date or by
    /// modification date (`sort`).
    ///
    /// Returns the threads and the cursor of the next page (`None` when this
    /// is the last page). Given no address of the mailbox, it tells the
    /// user's mails by their folder alone (Sent, Drafts): for the CLI and
    /// tests, where that is enough.
    pub fn list_threads(
        &self,
        folder_id: i64,
        cursor: Option<&str>,
        limit: u32,
        sort: ThreadSort,
    ) -> Result<(Vec<ThreadSummary>, Option<String>)> {
        self.list_threads_in(&[folder_id], cursor, limit, sort, &ThreadFilter::default(), &[])
    }

    /// One page of the threads of several folders together, newest first:
    /// the unified view ("all") lists the folders of the same role in every
    /// account. (The virtual folders have their own methods,
    /// `list_everything` and `list_all_mail`.)
    ///
    /// The folders are grouped by role. A role common to the folders of
    /// every account is **one** query: one walk of the index of the role
    /// (`idx_thread_role_received`, or the partial index of a technical
    /// flag), whatever the number of accounts, which stops after a page.
    /// Only folders of different roles together (Inbox and Sent, which the
    /// CLI may ask for) are several queries, one per role, whose pages are
    /// merged on the sort key, each thread once: every query reads `thread`,
    /// so the key of a thread is the same in all of them.
    ///
    /// `own`: the addresses of the mailbox, which `accounts.toml` holds and
    /// only the caller knows, as for `get_thread`. They tell the user's
    /// mails, so a row names the person the conversation is with rather
    /// than the user (`ThreadSummary::correspondent`). The other lists
    /// take them for the same reason.
    pub fn list_threads_in(
        &self,
        folder_ids: &[i64],
        cursor: Option<&str>,
        limit: u32,
        sort: ThreadSort,
        filter: &ThreadFilter,
        own: &[String],
    ) -> Result<(Vec<ThreadSummary>, Option<String>)> {
        if folder_ids.is_empty() {
            return Ok((vec![], None));
        }
        // The listed folders, grouped by scope: for each, the folder of
        // each account, which a row of that account is shown in.
        let mut groups: BTreeMap<Scope, BTreeMap<i64, i64>> = BTreeMap::new();
        for &f in folder_ids {
            let (role, account) = self.folder_role_and_account(f)?;
            let scope = match role {
                Some(r) => Scope::of(r),
                None => Scope::Unmanaged(f),
            };
            groups.entry(scope).or_default().insert(account, f);
        }
        let after = cursor.map(parse_cursor).transpose()?;
        let fetch = i64::from(limit) + 1;
        // Attribute filters: their criteria look at the mails of every
        // listed folder (`ui.md`, "Négation"). The ids are integers from
        // the store written into the SQL text (no user input).
        let set = folder_ids.iter().map(|f| f.to_string()).collect::<Vec<_>>().join(",");
        let tags = self.tag_ids(filter)?;
        let unread = self.unread_plan(filter, Some(folder_ids), None)?;
        let filter = filter_clause(filter, &Target::folders(&set), &tags, unread);

        let mut rows: Vec<(Scope, ListRow)> = Vec::new();
        for (scope, folders) in &groups {
            let accounts = match scope {
                Scope::Role(r) | Scope::Flag(r) => self.accounts_of(*r, folders)?,
                _ => Accounts::All,
            };
            let (sql, values) = list_query(*scope, &accounts, after, fetch, sort, &filter);
            rows.extend(self.list_rows(&sql, values)?.into_iter().map(|r| (*scope, r)));
        }
        // Several groups: merge on the sort key then the id, like the index
        // order, and keep each thread once (a thread of the Inbox the user
        // answered is in the Inbox and in Sent).
        if groups.len() > 1 {
            let key = |r: &ListRow| (if sort == ThreadSort::Modified { r.3 } else { r.2 }, r.0);
            rows.sort_by_key(|(_, r)| std::cmp::Reverse(key(r)));
            let mut seen = std::collections::HashSet::new();
            rows.retain(|(_, r)| seen.insert(r.0));
        }
        self.page(rows, limit, sort, own, |scope, row| {
            let account = row.7;
            Ok(match scope {
                // The listed folder of the row's account, with its role: a
                // row of Sent is moved from Sent (it has no targets).
                Scope::Role(r) | Scope::Flag(r) => groups[&scope].get(&account).map(|f| (*f, Some(r))),
                Scope::Unmanaged(f) => Some((f, None)),
                _ => None,
            })
        })
    }

    /// The accounts of a list of the folders `folders` (account → folder)
    /// of role `role`: every account when the folders of that role of all
    /// accounts are listed (the unified view), else one, else several.
    fn accounts_of(&self, role: FolderRole, folders: &BTreeMap<i64, i64>) -> Result<Accounts> {
        if folders.len() == 1 {
            return Ok(Accounts::One(*folders.keys().next().unwrap_or(&0)));
        }
        let all: Vec<i64> = self
            .conn
            .prepare_cached("SELECT account_id FROM folder WHERE role = ?1")?
            .query_map([role.as_str()], |r| r.get(0))?
            .collect::<std::result::Result<_, _>>()?;
        if all.iter().all(|a| folders.contains_key(a)) {
            Ok(Accounts::All)
        } else {
            Ok(Accounts::Some(folders.keys().map(i64::to_string).collect::<Vec<_>>().join(",")))
        }
    }

    /// How the list finds its unread conversations (`UnreadPlan`), from
    /// the unread mails of the listed folders (`folders`), or of every
    /// folder of `account_id` (of every account when `None`) in Everything.
    /// `folder.unread_count` is the sidebar's counter, kept up to date by
    /// every change: one indexed read of a few hundred rows, and never
    /// stale. Without the unread criterion, nothing is read.
    ///
    /// The counter includes the mails of Deleted, Junk and Banned, which
    /// `thread.unread_count` leaves out: it only chooses the method, both
    /// give the same rows.
    fn unread_plan(&self, filter: &ThreadFilter, folders: Option<&[i64]>, account_id: Option<i64>) -> Result<UnreadPlan> {
        if filter.seen != Some(false) {
            return Ok(UnreadPlan::Walk);
        }
        let unread: i64 = match folders {
            // Integers from the store written into the SQL text.
            Some(ids) => {
                let set = ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
                self.conn.query_row(&format!("SELECT coalesce(sum(unread_count), 0) FROM folder WHERE id IN ({set})"), [], |r| r.get(0))?
            }
            None => self.conn.prepare_cached(
                "SELECT coalesce(sum(unread_count), 0) FROM folder WHERE ?1 IS NULL OR account_id = ?1",
            )?
            .query_row([account_id], |r| r.get(0))?,
        };
        Ok(UnreadPlan::for_count(unread))
    }

    /// Runs a list query and reads its rows.
    fn list_rows(&self, sql: &str, values: Vec<Value>) -> Result<Vec<ListRow>> {
        let map = |r: &Row| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?));
        Ok(self.conn.prepare_cached(sql)?.query_map(rusqlite::params_from_iter(values), map)?.collect::<std::result::Result<_, _>>()?)
    }

    /// Turns the rows of a list (one more than `limit` when there is a next
    /// page) into a page of summaries and the cursor of the next page.
    /// `shown_in` gives the folder a row is shown in and its role, or
    /// `None` to take the thread's own folder (Everything, All Mail).
    ///
    /// Rust note: `impl Fn(...)` takes any closure with that signature; the
    /// caller's closure borrows its local maps.
    fn page(
        &self,
        rows: Vec<(Scope, ListRow)>,
        limit: u32,
        sort: ThreadSort,
        own: &[String],
        shown_in: impl Fn(Scope, &ListRow) -> Result<Option<(i64, Option<FolderRole>)>>,
    ) -> Result<(Vec<ThreadSummary>, Option<String>)> {
        // `as i64` is an explicit numeric conversion; Rust never converts
        // between integer types implicitly.
        let has_more = rows.len() as i64 > i64::from(limit);
        let role_folders = self.role_folders()?;
        // Read once per page: a handful of rows (`own_addresses`).
        let own = self.own_addresses(own)?;
        let mut items = Vec::with_capacity(rows.len().min(limit as usize));
        // `into_iter` consumes `rows`; `take(limit)` drops the extra row.
        for (scope, row) in rows.into_iter().take(limit as usize) {
            let (folder_id, role) = match shown_in(scope, &row)? {
                Some(x) => x,
                None => self.own_folder(&row, &role_folders)?,
            };
            let (thread_id, subject, received, modified, count, unread, _, _) = row;
            items.push(self.thread_summary(thread_id, folder_id, role, subject, (received, modified), count, unread, &own)?);
        }
        let next_cursor = if has_more {
            items.last().map(|t| {
                let d = if sort == ThreadSort::Modified { t.date_last_modified } else { t.date_last_received };
                format!("{d}:{}", t.id)
            })
        } else {
            None
        };
        Ok((items, next_cursor))
    }

    /// The folder a thread is in (`dossiers.md`, "Dossier d'une
    /// conversation"): that of its role in its account. A thread without a
    /// role (its mails all in folders without one) is shown in the folder
    /// of its latest mail.
    fn own_folder(&self, row: &ListRow, role_folders: &HashMap<(i64, FolderRole), i64>) -> Result<(i64, Option<FolderRole>)> {
        let role = row.6.as_deref().and_then(FolderRole::from_code);
        if let Some(r) = role
            && let Some(f) = role_folders.get(&(row.7, r))
        {
            return Ok((*f, Some(r)));
        }
        let folder: i64 = self
            .conn
            .prepare_cached("SELECT folder_id FROM message WHERE thread_id = ?1 ORDER BY date_received DESC, id DESC LIMIT 1")?
            .query_row([row.0], |r| r.get(0))
            .optional()?
            .unwrap_or(0);
        Ok((folder, None))
    }

    /// One page of the virtual folder "Everything" (`ui.md`): every
    /// conversation, of one account or of all of them, newest first, each
    /// in its folder. One walk of `idx_thread_received` (or its per-account
    /// and "modified" variants), whatever the number of accounts and
    /// folders. Its criteria look at every mail of the thread.
    pub fn list_everything(
        &self,
        account_id: Option<i64>,
        cursor: Option<&str>,
        limit: u32,
        sort: ThreadSort,
        filter: &ThreadFilter,
        own: &[String],
    ) -> Result<(Vec<ThreadSummary>, Option<String>)> {
        let tags = self.tag_ids(filter)?;
        let unread = self.unread_plan(filter, None, account_id)?;
        let filter = filter_clause(filter, &Target::everything(), &tags, unread);
        self.list_virtual(Scope::Everything, account_id, cursor, limit, sort, &filter, own)
    }

    /// The virtual folder "All Mail" (`ui.md`, "Sélecteur d'attributs"):
    /// every conversation of one account, or of every account, but those
    /// in Deleted, Junk and Banned (`FolderRole::OUT_OF_ALL_MAIL`). A
    /// thread whose latest mail was deleted keeps its place: its date is
    /// that of its latest mail left (the "live" date of `thread`). The
    /// attributes look at the mails of the folders of All Mail only.
    ///
    /// One walk of the partial index `idx_thread_live_received` (or its
    /// variants), stopping after a page.
    pub fn list_all_mail(
        &self,
        account_id: Option<i64>,
        cursor: Option<&str>,
        limit: u32,
        sort: ThreadSort,
        filter: &ThreadFilter,
        own: &[String],
    ) -> Result<(Vec<ThreadSummary>, Option<String>)> {
        let out: Vec<&str> = FolderRole::OUT_OF_ALL_MAIL.iter().map(|r| r.as_str()).collect();
        let folders: Vec<i64> = self
            .conn
            .prepare_cached(
                "SELECT id FROM folder
                  WHERE (?1 IS NULL OR account_id = ?1) AND coalesce(role, '') NOT IN (?2, ?3, ?4) ORDER BY id",
            )?
            .query_map(params![account_id, out[0], out[1], out[2]], |r| r.get(0))?
            .collect::<std::result::Result<_, _>>()?;
        self.list_virtual_of(Scope::AllMail, &folders, account_id, cursor, limit, sort, filter, own)
    }

    /// The virtual folder "Work" (`ui.md`, "Dossiers virtuels"): the
    /// conversations of Inbox, Todo and Waiting together, of one account or
    /// of every account, each in its folder. One walk of the partial index
    /// `idx_thread_work_received` (or its variants). The attributes look at
    /// the mails of those three folders.
    pub fn list_work(
        &self,
        account_id: Option<i64>,
        cursor: Option<&str>,
        limit: u32,
        sort: ThreadSort,
        filter: &ThreadFilter,
        own: &[String],
    ) -> Result<(Vec<ThreadSummary>, Option<String>)> {
        let roles: Vec<&str> = FolderRole::WORK.iter().map(|r| r.as_str()).collect();
        let folders: Vec<i64> = self
            .conn
            .prepare_cached(
                "SELECT id FROM folder WHERE (?1 IS NULL OR account_id = ?1) AND role IN (?2, ?3, ?4) ORDER BY id",
            )?
            .query_map(params![account_id, roles[0], roles[1], roles[2]], |r| r.get(0))?
            .collect::<std::result::Result<_, _>>()?;
        self.list_virtual_of(Scope::Work, &folders, account_id, cursor, limit, sort, filter, own)
    }

    /// A page of a virtual folder whose criteria look at the mails of
    /// `folders` (All Mail, Work).
    #[allow(clippy::too_many_arguments)]
    fn list_virtual_of(
        &self,
        scope: Scope,
        folders: &[i64],
        account_id: Option<i64>,
        cursor: Option<&str>,
        limit: u32,
        sort: ThreadSort,
        filter: &ThreadFilter,
        own: &[String],
    ) -> Result<(Vec<ThreadSummary>, Option<String>)> {
        if folders.is_empty() {
            return Ok((vec![], None));
        }
        // The folders, as integers from the store written into the SQL
        // text (no user input).
        let set = folders.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
        let tags = self.tag_ids(filter)?;
        let unread = self.unread_plan(filter, Some(folders), None)?;
        let filter = filter_clause(filter, &Target::folders(&set), &tags, unread);
        self.list_virtual(scope, account_id, cursor, limit, sort, &filter, own)
    }

    /// A page of Everything or All Mail: one query, each row in its folder.
    #[allow(clippy::too_many_arguments)]
    fn list_virtual(
        &self,
        scope: Scope,
        account_id: Option<i64>,
        cursor: Option<&str>,
        limit: u32,
        sort: ThreadSort,
        filter: &FilterSql,
        own: &[String],
    ) -> Result<(Vec<ThreadSummary>, Option<String>)> {
        let after = cursor.map(parse_cursor).transpose()?;
        let accounts = account_id.map(Accounts::One).unwrap_or(Accounts::All);
        let (sql, values) = list_query(scope, &accounts, after, i64::from(limit) + 1, sort, filter);
        let rows = self.list_rows(&sql, values)?.into_iter().map(|r| (scope, r)).collect();
        self.page(rows, limit, sort, own, |_, _| Ok(None))
    }

    /// Completes a list row with the details that `thread` does not store:
    /// participants, snippet, attachments, flag and return mark.
    ///
    /// They are read from the mails counted by the row: those outside
    /// Deleted, Junk and Banned, or all of them when none is (the rule of
    /// the counters, `schema.sql`).
    ///
    /// This runs a few small queries per thread (an "N+1" pattern). With a
    /// page of 50 threads and indexed lookups, it costs a few milliseconds;
    /// if it ever shows up in profiling, these fields can be denormalized.
    ///
    /// `#[allow(...)]` silences the clippy lint about functions with more
    /// than 7 parameters.
    ///
    /// `dates` is (last received, last modified) of the thread.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn thread_summary(
        &self,
        thread_id: i64,
        folder_id: i64,
        role: Option<FolderRole>,
        subject: String,
        dates: (i64, i64),
        message_count: i64,
        unread_count: i64,
        own: &BTreeSet<String>,
    ) -> Result<ThreadSummary> {
        // The thread's messages, oldest first, with their folder's role.
        let mails: Vec<ListMail> = self
            .conn
            .prepare_cached(THREAD_MESSAGES)?
            .query_map([thread_id], |r| {
                Ok(ListMail {
                    name: r.get(0)?,
                    addr: r.get(1)?,
                    snippet: r.get(2)?,
                    has_attachments: r.get(3)?,
                    role: role_of(r.get(4)?),
                    id: r.get(5)?,
                    // A value the schema does not know (it has a CHECK)
                    // reads as no verdict rather than failing the list.
                    kind: r.get::<_, Option<String>>(6)?.as_deref().and_then(SenderKind::parse),
                })
            })?
            .collect::<std::result::Result<_, _>>()?;
        let rejected = |r: Option<FolderRole>| r.is_some_and(|r| FolderRole::OUT_OF_ALL_MAIL.contains(&r));
        let any_live = mails.iter().any(|m| !rejected(m.role));
        // The mails the row counts, as the counters do.
        let counted: Vec<&ListMail> = mails.iter().filter(|m| !(any_live && rejected(m.role))).collect();
        let mut participants: Vec<String> = Vec::new();
        let mut snippet = None;
        let mut has_attachments = false;
        for m in &counted {
            if let Some(p) = label(&m.name, &m.addr) {
                // Remove a previous occurrence then append: each participant
                // appears once, at the position of their latest message.
                participants.retain(|x| x != &p);
                participants.push(p);
            }
            // Keep the snippet of the latest message that has one.
            // Rust note: `m.snippet` sits behind a shared reference (`&`),
            // so it is cloned rather than moved out.
            snippet = m.snippet.clone().or(snippet);
            has_attachments |= m.has_attachments;
        }

        // Who the conversation is with, and whether the user spoke last
        // (`ui.md`, "Liste"). Pure, so tested on its own (`correspondent`).
        let (correspondent, correspondent_kind, last_is_mine) = match correspondent(&counted, own) {
            Correspondent::Author(author, mine) => {
                // Rust note: `unzip` splits an `Option<(A, B)>` into an
                // `(Option<A>, Option<B>)`; `flatten` then turns the kind's
                // `Option<Option<SenderKind>>` into one level.
                let (name, kind) = author.unzip();
                (name, kind.flatten(), mine)
            }
            // Every mail is the user's: the person they wrote to, read
            // only for such threads (one indexed lookup).
            Correspondent::RecipientOf(id) => (
                self.conn
                    .prepare_cached(FIRST_RECIPIENT)?
                    .query_row([id], |r| Ok(label(&r.get(0)?, &r.get(1)?)))
                    .optional()?
                    .flatten(),
                // The user wrote every mail: no received mail carries a
                // verdict, and none is shown (`expediteurs.md` judges
                // senders, not recipients).
                None,
                true,
            ),
        };

        // The flag is looked up across the whole thread: a flagged message
        // in Sent still flags the conversation.
        let is_flagged: bool = self.conn.prepare_cached(THREAD_IS_FLAGGED)?
        .query_row([thread_id], |r| r.get(0))?;

        // The return mark only matters in the Inbox, where the thread landed
        // after leaving Todo or Waiting.
        let returned_from = if role == Some(FolderRole::Inbox) {
            let mut stmt = self.conn.prepare_cached(
                "SELECT DISTINCT mk.keyword FROM message m
                   JOIN message_keyword mk ON mk.message_id = m.id
                  WHERE m.thread_id = ?1
                    AND mk.keyword COLLATE NOCASE IN (?2, ?3)",
            )?;
            let kws: Vec<String> = stmt
                .query_map(params![thread_id, KW_WAS_TODO, KW_WAS_WAITING], |r| r.get(0))?
                .collect::<std::result::Result<_, _>>()?;
            // Both marks present: `todo` wins, since there is still something
            // to do.
            if kws.iter().any(|k| k.eq_ignore_ascii_case(KW_WAS_TODO)) {
                Some(FolderRole::Todo)
            } else if kws.iter().any(|k| k.eq_ignore_ascii_case(KW_WAS_WAITING)) {
                Some(FolderRole::Waiting)
            } else {
                None
            }
        } else {
            None
        };

        // Tags of the thread: keywords of its messages that are declared in
        // `tag` (other keywords are technical, `modele de donnees.md`).
        let tags: Vec<String> = self
            .conn
            .prepare_cached(
                "SELECT DISTINCT t.name FROM message m
                   JOIN message_keyword mk ON mk.message_id = m.id
                   JOIN tag t ON t.account_id = m.account_id AND t.keyword = mk.keyword AND t.date_deleted IS NULL
                  WHERE m.thread_id = ?1
                  ORDER BY t.name",
            )?
            .query_map([thread_id], |r| r.get(0))?
            .collect::<std::result::Result<_, _>>()?;

        // Rust note: `subject` alone is shorthand for `subject: subject`.
        Ok(ThreadSummary {
            id: thread_id,
            subject,
            participants,
            correspondent,
            correspondent_kind,
            last_is_mine,
            snippet,
            date_last_received: dates.0,
            date_last_modified: dates.1,
            folder_id,
            tags,
            message_count,
            unread_count,
            is_flagged,
            has_attachments,
            returned_from,
            reply_target: role.and_then(|r| r.reply_target(returned_from)),
        })
    }

    /// Where the message `message_id` is: its thread, folder and account
    /// (`locate_message`, deep links `nicemailer://m/<id>`). `NotFound`
    /// when it is not in the database any more (deleted, or the database
    /// was rebuilt and ids changed).
    pub fn locate_message(&self, message_id: i64) -> Result<nm_api::MessageLocation> {
        self.conn
            .query_row(
                "SELECT thread_id, folder_id, account_id FROM message WHERE id = ?1",
                [message_id],
                |r| Ok(nm_api::MessageLocation { message_id, thread_id: r.get(0)?, folder_id: r.get(1)?, account_id: r.get(2)? }),
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound(format!("message {message_id}")))
    }

    /// The addresses of the mailbox, lowercase: those the caller gives
    /// (`accounts.toml`, which the database does not hold) and those of
    /// the archive accounts, which it does (`account.address`). A handful
    /// of rows, read once per thread opened.
    fn own_addresses(&self, given: &[String]) -> Result<BTreeSet<String>> {
        let mut out: BTreeSet<String> = given.iter().map(|a| a.trim().to_lowercase()).filter(|a| !a.is_empty()).collect();
        let mut stmt = self.conn.prepare_cached("SELECT address FROM account WHERE address IS NOT NULL")?;
        for a in stmt.query_map([], |r| r.get::<_, String>(0))? {
            out.insert(a?.trim().to_lowercase());
        }
        Ok(out)
    }

    /// Every message of a thread, across all folders, oldest first.
    /// Returns the thread's subject and its messages.
    ///
    /// `own`: the addresses of the mailbox, which `accounts.toml` holds
    /// and only the caller knows (the daemon). They decide which messages
    /// are the user's own, and so always show their recipients
    /// (`present_thread`); the addresses of the archive accounts, which
    /// the database holds, are added here.
    pub fn get_thread(&self, thread_id: i64, own: &[String]) -> Result<(String, Vec<MessageView>)> {
        let subject: String = self
            .conn
            .query_row("SELECT subject FROM thread WHERE id = ?1", [thread_id], |r| r.get(0))
            .optional()?
            .ok_or_else(|| StoreError::NotFound(format!("conversation {thread_id}")))?;

        // LEFT JOIN on the body: a message whose body has not been downloaded
        // yet still appears, with `body_text` set to NULL.
        let mut stmt = self.conn.prepare_cached(
            "SELECT m.id, m.folder_id, f.role, m.subject, m.from_name, m.from_addr,
                    m.date_sent, m.date_received, b.text_plain,
                    m.is_seen, m.is_flagged, m.is_answered, m.has_attachments, m.auth_result,
                    b.html_sanitized, coalesce(b.has_remote_content, 0),
                    -- The images mode is stored as 'html' plus remote_images
                    -- (schema.sql, table address).
                    CASE WHEN a.body_format = 'html' AND a.remote_images = 1 THEN 'images' ELSE a.body_format END,
                    u.url, u.mailto, u.one_click, u.source, a.unsubscribed_at,
                    coalesce(b.tracking_pixels, 0)
               FROM message m
               JOIN folder f ON f.id = m.folder_id
               LEFT JOIN message_body b ON b.message_id = m.id
               LEFT JOIN address a ON a.id = m.from_address_id
               LEFT JOIN message_unsubscribe u ON u.message_id = m.id
              WHERE m.thread_id = ?1
              ORDER BY m.date_received, m.id",
        )?;
        let mut messages = stmt
            .query_map([thread_id], |r| {
                Ok(MessageView {
                    id: r.get(0)?,
                    folder_id: r.get(1)?,
                    folder_role: role_of(r.get(2)?),
                    subject: r.get(3)?,
                    from_name: r.get(4)?,
                    from_addr: r.get(5)?,
                    // Filled by the second query below.
                    to: vec![],
                    cc: vec![],
                    reply_to: vec![],
                    date_sent: r.get(6)?,
                    date_received: r.get(7)?,
                    body_text: r.get(8)?,
                    is_seen: r.get(9)?,
                    is_flagged: r.get(10)?,
                    is_answered: r.get(11)?,
                    has_attachments: r.get(12)?,
                    auth_result: r.get(13)?,
                    body_html: r.get(14)?,
                    has_remote_content: r.get(15)?,
                    from_body_format: r.get(16)?,
                    // Filled below, once the whole thread is known.
                    segments: vec![],
                    only_quotes: false,
                    expanded: false,
                    show_recipients: false,
                    attachments: vec![],
                    unsubscribe: unsubscribe_view(r.get(17)?, r.get(18)?, r.get(19)?, r.get(20)?, r.get(21)?),
                    tracking_pixels: r.get(22)?,
                    // Filled below, from the bodies just read.
                    links: vec![],
                    verification_code: None,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        // Recipients of each message, in header order.
        let mut addr_stmt = self.conn.prepare_cached(
            "SELECT ma.role, a.addr, a.name FROM message_address ma
               JOIN address a ON a.id = ma.address_id
              WHERE ma.message_id = ?1 AND ma.role IN ('to', 'cc', 'reply_to')
              ORDER BY ma.role, ma.position",
        )?;
        // `&mut messages` iterates with mutable references, so each `m` can
        // be modified in place.
        for m in &mut messages {
            let mut rows = addr_stmt.query([m.id])?;
            while let Some(r) = rows.next()? {
                let role: String = r.get(0)?;
                let view = AddressView { addr: r.get(1)?, name: r.get(2)? };
                match role.as_str() {
                    "to" => m.to.push(view),
                    "cc" => m.cc.push(view),
                    _ => m.reply_to.push(view),
                }
            }
        }
        // Attachments of the whole thread in one query, then dealt out to
        // their messages (`attachments.rs`).
        let attachments = self.thread_attachments(thread_id)?;
        for (message_id, a) in attachments.listed {
            if let Some(m) = messages.iter_mut().find(|m| m.id == message_id) {
                m.attachments.push(a);
            }
        }
        // The links of `l`, read from the HTML: the text the core makes
        // from it has no address left (`links.rs`). One scan per body,
        // already in memory: computed here rather than stored.
        for m in &mut messages {
            m.links = crate::links::find(m.body_html.as_deref(), m.body_text.as_deref())
                .into_iter()
                .map(|l| nm_api::LinkView { url: l.url, text: l.text, target: l.target, other_texts: l.other_texts })
                .collect();
            // Its verification code, read the same way: from the subject
            // and the bodies already in memory (`codes.rs`).
            m.verification_code = crate::codes::find(&m.subject, m.body_text.as_deref(), m.body_html.as_deref());
        }
        // Embedded images get a source the reader can load (`nm-cid:`,
        // `attachments.rs`). Done here rather than stored: the attachment
        // ids are only known once the message is inserted, after its body
        // is sanitized.
        for m in &mut messages {
            let mut images = attachments.inline.iter().filter(|i| i.message_id == m.id).peekable();
            if images.peek().is_some()
                && let Some(html) = &m.body_html
            {
                m.body_html = Some(crate::attachments::resolve_inline_images(html, images));
            }
        }
        present_thread(&mut messages, &self.own_addresses(own)?);
        Ok((subject, messages))
    }

    /// Full-text search, sorted by relevance. Each word typed is searched as
    /// a prefix: "fact dup" finds "facture Dupont". Accents are ignored by
    /// the FTS5 tokenizer (`remove_diacritics 2` in schema.sql), so
    /// "electricite" finds "électricité".
    pub fn search(&self, query: &str, limit: u32) -> Result<Vec<SearchHit>> {
        // Build an FTS5 query: each word is wrapped in double quotes (so
        // characters like `-` or `:` are taken literally instead of as FTS
        // operators) and followed by `*` (prefix match). Double quotes typed
        // by the user are removed, as they would break the syntax.
        // Words separated by spaces are implicitly ANDed by FTS5.
        let fts_query = fts_query(query);
        if fts_query.is_empty() {
            return Ok(vec![]);
        }
        // Words with punctuation, confirmed as written (`is_literal`).
        let mut values: Vec<Value> = vec![Value::Text(fts_query), Value::Integer(i64::from(limit))];
        let mut literal = String::new();
        for w in fts_words(query).0.iter().filter(|w| is_literal(w)) {
            let mut bind = |v: Value| {
                values.push(v);
                format!("?{}", values.len())
            };
            literal.push_str(&format!(" AND {}", literal_condition(w, "", &mut bind)));
        }
        // `snippet(...)` is an FTS5 function returning an excerpt around the
        // matches: column -1 means "best column", '[' and ']' wrap the
        // matched terms, '…' marks cut text, 12 is the excerpt length in
        // tokens. `rank` is FTS5's relevance score (BM25), lower is better.
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT m.id, m.thread_id, m.folder_id, f.role, m.subject, m.from_name, m.from_addr,
                    m.date_received, snippet(message_fts, -1, '[', ']', '…', 12)
               FROM message_fts
               JOIN message m ON m.id = message_fts.rowid
               JOIN folder f ON f.id = m.folder_id
              WHERE message_fts MATCH ?1{literal}
              ORDER BY rank
              LIMIT ?2"
        ))?;
        let hits = stmt
            .query_map(rusqlite::params_from_iter(values), |r| {
                Ok(SearchHit {
                    message_id: r.get(0)?,
                    thread_id: r.get(1)?,
                    folder_id: r.get(2)?,
                    folder_role: role_of(r.get(3)?),
                    subject: r.get(4)?,
                    from_name: r.get(5)?,
                    from_addr: r.get(6)?,
                    date_received: r.get(7)?,
                    highlight: r.get(8)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(hits)
    }
}

/// The list query of a scope and its parameters: the threads of `scope`
/// and `accounts`, newest first on the sort key, after the cursor, `fetch`
/// rows at most, with the attribute conditions `filter`.
///
/// One statement whatever the scope: the table `thread` alone, walked on
/// one of its indexes (`schema.sql`, version 9). The plan tests below check
/// which one for every scope, filter, sort and cursor.
fn list_query(
    scope: Scope,
    accounts: &Accounts,
    after: Option<(i64, i64)>,
    fetch: i64,
    sort: ThreadSort,
    filter: &FilterSql,
) -> (String, Vec<Value>) {
    // The sort column: a constant of ours, never client input.
    let key = match sort {
        ThreadSort::Received => "t.date_last_received",
        ThreadSort::Modified => "t.date_last_modified",
    };
    // Work: its partial index is named. Without statistics, SQLite prefers
    // to read the three ranges of `idx_thread_role_received` and sort them
    // (the plan tests caught it). Work holds what is still to deal with, a
    // small set: walking its index is right even with a rare criterion.
    //
    // The set of the unread threads (`UnreadPlan::Set`): no index of
    // `thread` at all (`NOT INDEXED`), only its primary key, so the threads
    // of the set are read one by one then sorted. Without statistics (a new
    // database, before `ANALYZE`) SQLite walks the date index instead and
    // only tests each row against the set: the whole mailbox again.
    let indexed = if filter.by_set {
        " NOT INDEXED".to_string()
    } else if scope == Scope::Work {
        let account = if matches!(accounts, Accounts::One(_)) { "_account" } else { "" };
        let sort_name = if sort == ThreadSort::Modified { "modified" } else { "received" };
        format!(" INDEXED BY idx_thread_work{account}_{sort_name}")
    } else {
        String::new()
    };
    let mut sql = format!(
        "SELECT t.id, t.subject, t.date_last_received, t.date_last_modified, t.message_count, t.unread_count,
                t.folder, t.account_id
           FROM thread t{indexed} WHERE {}",
        scope.condition()
    );
    // Fixed parameters: ?1 account, ?2 limit, ?3 and ?4 cursor; the filter
    // values start at ?10. SQLite counts parameters up to the highest number
    // used, so unused numbers are bound to NULL.
    let mut values = vec![Value::Null, Value::Integer(fetch)];
    match accounts {
        Accounts::All => {}
        Accounts::One(a) => {
            sql.push_str(" AND t.account_id = ?1");
            values[0] = Value::Integer(*a);
        }
        // The unary `+` keeps SQLite from reading the rows account by
        // account (several ranges, then a sort of all of them): the index
        // of all accounts is walked, the rows of the others skipped.
        Accounts::Some(ids) => sql.push_str(&format!(" AND +t.account_id IN ({ids})")),
    }
    sql.push_str(&filter.sql);
    // Two shapes rather than one with `?3 IS NULL OR ...`: such a condition
    // would prevent SQLite from using the index for the row-value
    // comparison, and every page would scan from the top. `(a, b) < (x, y)`
    // is a row-value comparison: true when a < x, or a = x and b < y. It
    // matches the sort order.
    if let Some((d, id)) = after {
        sql.push_str(&format!(" AND ({key}, t.id) < (?3, ?4)"));
        values.extend([Value::Integer(d), Value::Integer(id)]);
    }
    sql.push_str(&format!(" ORDER BY {key} DESC, t.id DESC LIMIT ?2"));
    if !filter.values.is_empty() {
        values.resize(9, Value::Null);
        values.extend(filter.values.iter().cloned());
    }
    (sql, values)
}

/// Whether the user wrote a mail: its sender is one of the addresses of
/// the mailbox (`own`, lowercase), or it sits in Sent or Drafts (an
/// account whose address `accounts.toml` no longer declares). It stays
/// theirs once filed in Waiting or Todo (`reponse.md`, section 8). The one
/// rule of the reader (`present_thread`) and of the list
/// (`correspondent`).
fn is_mine(from_addr: Option<&str>, role: Option<FolderRole>, own: &BTreeSet<String>) -> bool {
    from_addr.is_some_and(|a| own.contains(&a.trim().to_lowercase()))
        || matches!(role, Some(FolderRole::Sent) | Some(FolderRole::Drafts))
}

/// A mail of a thread, as a list row reads it (`THREAD_MESSAGES`).
struct ListMail {
    id: i64,
    name: Option<String>,
    addr: Option<String>,
    snippet: Option<String>,
    has_attachments: bool,
    role: Option<FolderRole>,
    /// `message.sender_kind`: a person or a robot wrote this mail
    /// (`expediteurs.md`, section 6); `None` without a readable sender or
    /// before the catch-up has read the mail.
    kind: Option<SenderKind>,
}

/// What a list shows of a person: their name, else their address; a
/// blank name counts as none.
///
/// Rust note: `&Option<String>` borrows the option without taking it;
/// `as_deref` turns it into an `Option<&str>` to look at, and `to_string`
/// makes the owned copy returned.
fn label(name: &Option<String>, addr: &Option<String>) -> Option<String> {
    let name = name.as_deref().map(str::trim).filter(|n| !n.is_empty());
    let addr = addr.as_deref().map(str::trim).filter(|a| !a.is_empty());
    name.or(addr).map(str::to_string)
}

/// Who a conversation is with, as `correspondent` finds it.
#[derive(Debug, PartialEq)]
enum Correspondent {
    /// The latest author who is not the user, with the verdict of the mail
    /// that names them (or `None`: no mail names one), and whether the
    /// user wrote the latest mail.
    Author(Option<(String, Option<SenderKind>)>, bool),
    /// The user wrote every mail: the correspondent is the first recipient
    /// of this one, the latest, which the caller reads from the database.
    RecipientOf(i64),
}

/// Who the conversation of `mails` (oldest first, those its row counts) is
/// with, for the sender column of the list (`ui.md`, "Liste").
///
/// The author of the latest mail alone told nothing when the user had
/// answered: the row showed their own name. The latest author who is not
/// the user is the person the conversation waits on, or who waits on the
/// user; the second value says which ("→ Alice": the user spoke last).
///
/// Drafts are left out unless the thread has nothing else: a reply being
/// written has not been sent, and the conversation still waits on the
/// user.
fn correspondent(mails: &[&ListMail], own: &BTreeSet<String>) -> Correspondent {
    let draft = |m: &ListMail| m.role == Some(FolderRole::Drafts);
    let only_drafts = mails.iter().all(|m| draft(m));
    // Rust note: `copied` turns the iterator's `&&ListMail` into the
    // `&ListMail` the slice holds; no mail is copied, only references.
    let sent: Vec<&ListMail> = mails.iter().copied().filter(|m| only_drafts || !draft(m)).collect();
    let mine = |m: &ListMail| is_mine(m.addr.as_deref(), m.role, own);
    let Some(last) = sent.last() else { return Correspondent::Author(None, false) };
    if sent.iter().all(|m| mine(m)) {
        return Correspondent::RecipientOf(last.id);
    }
    // The latest mail of someone else that names its author, and the
    // verdict of that very mail: the icon of Quarantine must describe the
    // name shown beside it, not another mail of the conversation.
    let author =
        sent.iter().rev().filter(|m| !mine(m)).find_map(|m| label(&m.name, &m.addr).map(|name| (name, m.kind)));
    Correspondent::Author(author, mine(last))
}

/// Simplified presentation of a thread, computed by the core so every
/// client shows the same thing (`clean.rs`, `docs/thread-display.md`):
/// segments of each message, messages folded by default, recipients only
/// when they change — always for the user's own mails, whose recipients
/// are the whole point (`own`, the addresses of the mailbox).
fn present_thread(messages: &mut [MessageView], own: &BTreeSet<String>) {
    let inputs: Vec<(Option<&str>, &str)> =
        messages.iter().map(|m| (m.from_addr.as_deref(), m.body_text.as_deref().unwrap_or(""))).collect();
    let cleaned = crate::clean::clean_thread(&inputs);
    let last = messages.len().saturating_sub(1);
    // A mail alone in its conversation has no previous message to compare
    // with: its recipients are always shown (`ui.md`, "En-têtes allégés").
    let single = messages.len() == 1;
    let mut previous: Option<Vec<String>> = None;
    for (i, (m, c)) in messages.iter_mut().zip(cleaned).enumerate() {
        if m.body_text.is_some() {
            m.segments = c
                .segments
                .into_iter()
                .map(|s| SegmentView {
                    kind: s.segment.kind.as_str().to_string(),
                    text: s.segment.text,
                    depth: s.segment.depth,
                    hidden: s.hidden,
                    in_thread: s.in_thread,
                })
                .collect();
            m.only_quotes = c.only_quotes;
        }
        // Gmail's rule: the latest message and the unread ones are open; a
        // message that only quotes stays folded unless it is the latest.
        m.expanded = i == last || (!m.is_seen && !m.only_quotes);
        let mut recipients: Vec<String> = m.to.iter().chain(m.cc.iter()).map(|a| a.addr.to_lowercase()).collect();
        recipients.sort();
        // A mail the user wrote: its sender is one of their addresses, or
        // it sits in Sent or Drafts (an account whose address the file no
        // longer declares). Showing "from me" tells nothing; who it went
        // to is the information, and a mail sent to one person would
        // otherwise show nobody (`docs/thread-display.md`).
        let mine = is_mine(m.from_addr.as_deref(), m.folder_role, own);
        m.show_recipients = !recipients.is_empty()
            && (mine
                || match &previous {
                    // The first message of a conversation: shown when it
                    // went to several people, or when it is the only one.
                    None => single || recipients.len() > 1,
                    Some(p) => *p != recipients,
                });
        previous = Some(recipients);
    }
}

/// An FTS5 query from words typed by the user: each word wrapped in double
/// quotes (so characters like `-` or `:` are taken literally instead of as
/// FTS operators) and followed by `*` (prefix match). Double quotes typed by
/// the user are removed, as they would break the syntax. Words separated by
/// spaces are implicitly ANDed by FTS5.
fn fts_query(query: &str) -> String {
    fts_words(query).0.into_iter().map(|w| fts_phrase(&w, "")).collect::<Vec<_>>().join(" ")
}

/// The words typed in a text criterion, split into the wanted ones and the
/// excluded ones (written `-word`, `ui.md`, "Négation"), double quotes
/// removed. A lone `-` is no word; a dash inside a word (`e-mail`) is part
/// of it.
fn fts_words(query: &str) -> (Vec<String>, Vec<String>) {
    let mut wanted = Vec::new();
    let mut excluded = Vec::new();
    for w in query.split_whitespace().map(|w| w.replace('"', "")) {
        match w.strip_prefix('-') {
            Some(rest) if !rest.is_empty() => excluded.push(rest.to_string()),
            Some(_) => {}
            None if !w.is_empty() => wanted.push(w),
            None => {}
        }
    }
    (wanted, excluded)
}

/// Whether a typed word must also be found as it is written: it holds a
/// character the tokenizer splits on (`.` `@` `/` `:` `_`…). FTS5 reads
/// `paris.fr` as the phrase "paris" then "fr", and as a prefix (`*`) "paris"
/// then a word starting with "fr": "75016 PARIS / France" matched, 14,542
/// mails of a real mailbox for a handful that contain "paris.fr". Such a
/// word is still looked up in the index (it finds the candidates, with a
/// word boundary before `paris`), then confirmed with `LIKE` on the text of
/// those candidates (`literal_condition`). Hyphens and apostrophes do not
/// count: they join the words of prose (`rendez-vous`, `l'eau`, often typed
/// `l’eau` in a mail), where the index alone is the right answer.
fn is_literal(word: &str) -> bool {
    word.chars().any(|c| !c.is_alphanumeric() && !matches!(c, '-' | '\'' | '’'))
}

/// The columns of `message_fts` a column filter of `fts_phrase` covers
/// (`""`: every column).
fn literal_columns(columns: &str) -> &'static [&'static str] {
    match columns {
        "subject : " => &["subject"],
        "{subject body} : " => &["subject", "body"],
        _ => &["subject", "sender", "recipients", "body"],
    }
}

/// `(subject LIKE ? OR body LIKE ? …)` on `message_fts` for a literal word:
/// the word as written, anywhere in one of the columns. `LIKE` ignores ASCII
/// case; `%` and `_` typed are escaped. Accents are not folded here, unlike
/// in the index: `eleve.fr` does not find `élève.fr`.
fn literal_condition(word: &str, columns: &str, bind: &mut dyn FnMut(Value) -> String) -> String {
    let escaped = word.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
    let p = bind(Value::Text(format!("%{escaped}%")));
    let ors: Vec<String> =
        literal_columns(columns).iter().map(|c| format!("message_fts.{c} LIKE {p} ESCAPE '\\'")).collect();
    format!("({})", ors.join(" OR "))
}

/// One FTS5 phrase: the word quoted, as a prefix, behind a column filter
/// (`{subject body} : ` restricts it to those columns; empty: every
/// column).
fn fts_phrase(word: &str, columns: &str) -> String {
    format!("{columns}\"{word}\"*")
}

/// How the criterion "unread" finds its conversations.
///
/// A list walks its date index and stops after a page. A test on
/// `thread.unread_count` during that walk costs the rows walked before a
/// page is found: nothing when unread mail is common, the whole mailbox
/// when it is rare, and the most when there is none at all (the walk goes
/// to the end). The set of the unread conversations, read from the partial
/// index `idx_message_unread` (only the unread mails), costs the opposite:
/// nothing when it is small, a sort of every unread conversation when it
/// is large.
///
/// Measured on a real mailbox of 400,000 mails and 296,000 conversations
/// (one page of 50, Everything and the archives alike):
///
/// | unread mails | walk     | set     |
/// | ------------ | -------- | ------- |
/// | 0            | 64 ms    | 0.0 ms  |
/// | 100          | 32 ms    | 0.1 ms  |
/// | 1,000        | 4 ms     | 0.7 ms  |
/// | 5,000        | 0.8 ms   | 7–13 ms |
/// | 50,000       | 0.1 ms   | 74 ms   |
/// | 150,000      | 0.0 ms   | 145 ms  |
///
/// Hence the switch at [`UNREAD_SET_MAX`], decided for each query on the
/// unread mails of the listed folders, not of the mailbox: 100 unread mails
/// all in the Inbox leave none in the archives, whose walk would go to the
/// end. Both plans give the same rows: the set only narrows the threads,
/// and `t.unread_count > 0` still decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnreadPlan {
    /// Test `t.unread_count` during the walk of the list's index.
    Walk,
    /// Read the set of the unread conversations first.
    Set,
}

/// The most unread mails in the listed folders for which the list reads
/// the set of their conversations (`UnreadPlan`). Both plans cost a few
/// milliseconds there.
const UNREAD_SET_MAX: i64 = 2_000;

impl UnreadPlan {
    fn for_count(unread_mails: i64) -> UnreadPlan {
        if unread_mails <= UNREAD_SET_MAX { UnreadPlan::Set } else { UnreadPlan::Walk }
    }
}

/// What the attribute filters apply to: the folders whose messages count.
///
/// Every list reads `thread t`; the column names are constants of ours
/// written into the SQL text, never client input.
struct Target<'a> {
    /// The listed folders (comma-separated ids), for the criteria that only
    /// look at the messages of those folders (`ui.md`, "Négation": From,
    /// To, attachments and Mail date). `None` in Everything: every folder
    /// is listed, so the whole thread counts.
    folders: Option<&'a str>,
}

impl<'a> Target<'a> {
    fn folders(set: &'a str) -> Target<'a> {
        Target { folders: Some(set) }
    }

    fn everything() -> Target<'static> {
        Target { folders: None }
    }

    /// " AND m.folder_id IN (…)" on the messages of a criterion, or nothing
    /// in Everything.
    fn in_folders(&self, alias: &str) -> String {
        self.folders.map(|set| format!(" AND {alias}.folder_id IN ({set})")).unwrap_or_default()
    }
}

/// SQL conditions for one text criterion over `columns` of `message_fts`
/// (`ui.md`, "Sélecteur d'attributs"): the thread has a message holding
/// every wanted word, and no message holding an excluded word. Each part
/// runs its full-text query once (not correlated), whatever the number of
/// threads.
fn text_clause(query: &str, columns: &str, thread: &str, bind: &mut dyn FnMut(Value) -> String) -> String {
    let (wanted, excluded) = fts_words(query);
    let mut sql = String::new();
    // `thread_id IS NOT NULL`: in `x NOT IN (set)`, a single NULL in the set
    // makes the result NULL (unknown), which would leave out every thread.
    let in_fts = |p: String| {
        format!(
            "SELECT mc.thread_id FROM message_fts JOIN message mc ON mc.id = message_fts.rowid
              WHERE message_fts MATCH {p} AND mc.thread_id IS NOT NULL"
        )
    };
    if !wanted.is_empty() {
        let q = wanted.iter().map(|w| fts_phrase(w, columns)).collect::<Vec<_>>().join(" ");
        let mut set = in_fts(bind(Value::Text(q)));
        // Words with punctuation: in the same mail, as written.
        for w in wanted.iter().filter(|w| is_literal(w)) {
            set.push_str(&format!(" AND {}", literal_condition(w, columns, bind)));
        }
        sql.push_str(&format!(" AND {thread} IN ({set})"));
    }
    // Any excluded word leaves the thread out. The plain words are OR-ed in
    // one query; each literal one has its own, confirmed as written.
    let (literal, plain): (Vec<&String>, Vec<&String>) = excluded.iter().partition(|w| is_literal(w));
    if !plain.is_empty() {
        let q = plain.iter().map(|w| fts_phrase(w, columns)).collect::<Vec<_>>().join(" OR ");
        sql.push_str(&format!(" AND {thread} NOT IN ({})", in_fts(bind(Value::Text(q)))));
    }
    for w in literal {
        let set = format!("{} AND {}", in_fts(bind(Value::Text(fts_phrase(w, columns)))), literal_condition(w, columns, bind));
        sql.push_str(&format!(" AND {thread} NOT IN ({set})"));
    }
    sql
}

/// The addresses matching one From / To value, as conditions on the
/// `address` table (`ui.md`, "Sélecteur d'attributs"). A value starting
/// with `@` is a domain: the address ends with `@domain` or `.domain` (its
/// subdomains), and the name is not looked at. Otherwise the value is part
/// of the address or of its name.
///
/// The name is the one of the address (`address.name`), not the display
/// name of each mail: a sender is a person, and a mail sent on someone's
/// behalf by a shared address (`notifications@github.com` showing
/// "thomas-serverhouse") is not from them. It also keeps the search fast:
/// the address table is small, and the mails are then reached through
/// indexes (`idx_message_from`, `idx_message_address_address`).
///
/// LIKE ignores ASCII case, and addresses are stored in lower case.
fn address_condition(value: &str, bind: &mut dyn FnMut(Value) -> String) -> String {
    // `%` and `_` are LIKE wildcards: escape them in the user's text.
    let escape = |s: &str| s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
    let v = value.trim();
    match v.strip_prefix('@').map(str::trim).filter(|d| !d.is_empty()) {
        Some(domain) => {
            let d = escape(&domain.to_lowercase());
            let at = bind(Value::Text(format!("%@{d}")));
            let sub = bind(Value::Text(format!("%.{d}")));
            format!("addr LIKE {at} ESCAPE '\\' OR addr LIKE {sub} ESCAPE '\\'")
        }
        None => {
            let p = bind(Value::Text(format!("%{}%", escape(v))));
            format!("addr LIKE {p} ESCAPE '\\' OR name LIKE {p} ESCAPE '\\'")
        }
    }
}

/// The attribute conditions of a list (`filter_clause`).
#[derive(Debug, Default)]
struct FilterSql {
    /// Conditions, each starting with " AND ".
    sql: String,
    /// Their values, numbered from `?10`.
    values: Vec<Value>,
    /// The unread criterion reads the set of the unread threads first
    /// (`UnreadPlan::Set`), or the list asks for given threads
    /// (`ThreadFilter::thread_ids`): `list_query` then reads `thread` by
    /// primary key, and sorts the few rows found.
    by_set: bool,
}

/// SQL conditions (each starting with " AND ") for the attribute filters,
/// and their values, numbered from `?10` (the first nine numbers are kept
/// for the fixed parameters of the list query).
///
/// Every criterion on the messages of a thread is a set of thread ids,
/// computed once through an index and tested with `IN` / `NOT IN`, never a
/// subquery run for each row of the list. A list walks its index until it
/// has a page of matching rows: with a rare criterion (a sender seen in 2009
/// only, no flagged mail at all), a per-row subquery ran for every thread of
/// the mailbox, several seconds on 200 000 threads.
fn filter_clause(f: &ThreadFilter, target: &Target, tags: &[i64], unread: UnreadPlan) -> FilterSql {
    // Given threads: read by primary key, never by walking a date index
    // and testing each row (SQLite would choose that walk to avoid a
    // sort, and read the whole list for a handful of rows).
    let unread_set = f.seen == Some(false) && unread == UnreadPlan::Set;
    let by_set = unread_set || !f.thread_ids.is_empty();
    let mut sql = String::new();
    let mut values: Vec<Value> = Vec::new();
    if !f.thread_ids.is_empty() {
        // Integers, written into the SQL text: no client text there.
        let ids = f.thread_ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
        sql.push_str(&format!(" AND t.id IN ({ids})"));
    }
    let thread = "t.id";
    // " AND <thread> IN (<set>)", or NOT IN for `false` / an exclusion.
    let member = |yes: bool, set: &str| format!(" AND {thread} {}IN ({set})", if yes { "" } else { "NOT " });
    // Unread: `thread.unread_count` decides (it leaves out the mails of
    // Deleted, Junk and Banned), narrowed first by the set of the threads
    // with an unread mail when they are few (`UnreadPlan`). Read: the common
    // case, a test during the walk stops early.
    if let Some(seen) = f.seen {
        if unread_set {
            sql.push_str(&member(true, "SELECT thread_id FROM message WHERE is_seen = 0 AND thread_id IS NOT NULL"));
        }
        sql.push_str(if seen { " AND t.unread_count = 0" } else { " AND t.unread_count > 0" });
    }
    // Flag and "answered" look at the whole thread, attachments at the
    // listed folders. Each set is read from a partial index that holds only
    // the marked messages (`idx_message_thread_flagged`…, schema.sql).
    if let Some(yes) = f.flagged {
        sql.push_str(&member(yes, "SELECT thread_id FROM message WHERE is_flagged = 1 AND thread_id IS NOT NULL"));
    }
    if let Some(yes) = f.answered {
        sql.push_str(&member(yes, "SELECT thread_id FROM message WHERE is_answered = 1 AND thread_id IS NOT NULL"));
    }
    if let Some(yes) = f.attachments {
        let set = format!(
            "SELECT m.thread_id FROM message m WHERE m.has_attachments = 1 AND m.thread_id IS NOT NULL{}",
            target.in_folders("m")
        );
        sql.push_str(&member(yes, &set));
    }
    // Human / robot (`expediteurs.md`, section 8): a column of the thread
    // (`thread.has_person`), tested during the walk like the conversation
    // criterion. A set of threads built at each request cost 50 ms a page
    // on 400,000 mails; the column costs nothing.
    if let Some(yes) = f.human {
        sql.push_str(if yes { " AND t.has_person = 1" } else { " AND t.has_person = 0" });
    }
    if let Some(yes) = f.conversation {
        sql.push_str(if yes { " AND t.message_count > 1" } else { " AND t.message_count = 1" });
    }
    // From, and To / Cc: a message of the thread in the listed folders is
    // (or, for the exclusions, none is) from / to one of the values, OR-ed.
    // The listed folders, not only the one the thread is shown in: in a
    // folder list of several folders, "not from @acme.fr" must see acme.fr's
    // mail in any of them.
    let address_set = |list: &[String], sender: bool, bind: &mut dyn FnMut(Value) -> String| -> Option<String> {
        let ors: Vec<String> =
            list.iter().filter(|v| !v.trim().is_empty()).map(|v| address_condition(v, bind)).collect();
        if ors.is_empty() {
            return None;
        }
        let ids = format!("SELECT id FROM address WHERE {}", ors.join(" OR "));
        let folders = target.in_folders("m");
        Some(if sender {
            format!(
                "SELECT m.thread_id FROM message m
                  WHERE m.from_address_id IN ({ids}) AND m.thread_id IS NOT NULL{folders}"
            )
        } else {
            format!(
                "SELECT m.thread_id FROM message_address ma JOIN message m ON m.id = ma.message_id
                  WHERE ma.address_id IN ({ids}) AND ma.role IN ('to', 'cc') AND m.thread_id IS NOT NULL{folders}"
            )
        })
    };
    // Rust note: `bind` borrows `values` mutably, so it is a closure created
    // here and passed down as `&mut dyn FnMut`: one numbering of the
    // parameters for every condition.
    let mut bind = |v: Value| -> String {
        values.push(v);
        format!("?{}", 9 + values.len())
    };
    for (list, yes, sender) in [(&f.from, true, true), (&f.not_from, false, true), (&f.to, true, false), (&f.not_to, false, false)] {
        if let Some(set) = address_set(list, sender, &mut bind) {
            sql.push_str(&member(yes, &set));
        }
    }
    // A tag: a message of the thread carries the keyword of one of the tags
    // of that name (`tags`, resolved by `tag_ids`), in that tag's account.
    // Read from `idx_message_keyword_kw`. Like the flag, it looks at the
    // whole thread. A name no tag has matches nothing.
    if f.tag.is_some() {
        if tags.is_empty() {
            sql.push_str(" AND 0");
        } else {
            let ids = tags.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
            sql.push_str(&member(
                true,
                &format!(
                    "SELECT m.thread_id FROM tag tg
                       JOIN message_keyword mk ON mk.keyword = tg.keyword
                       JOIN message m ON m.id = mk.message_id AND m.account_id = tg.account_id
                      WHERE tg.id IN ({ids}) AND m.thread_id IS NOT NULL"
                ),
            ));
        }
    }
    // A group or a contact: a message of the thread in the listed folders
    // is from one of the addresses or to them (To, Cc) (`groupes.md`). One
    // set, the union of the two, each read through its index.
    let people = |ids: &str| {
        let folders = target.in_folders("m");
        format!(
            "SELECT m.thread_id FROM message m
              WHERE m.from_address_id IN ({ids}) AND m.thread_id IS NOT NULL{folders}
             UNION ALL
             SELECT m.thread_id FROM message_address ma JOIN message m ON m.id = ma.message_id
              WHERE ma.address_id IN ({ids}) AND ma.role IN ('to', 'cc') AND m.thread_id IS NOT NULL{folders}"
        )
    };
    if let Some(group) = f.group_id {
        let g = bind(Value::Integer(group));
        let members = format!("SELECT address_id FROM contact_group_member WHERE group_id = {g} AND date_removed IS NULL");
        sql.push_str(&member(true, &people(&members)));
    }
    if let Some(address) = f.address_id {
        let a = bind(Value::Integer(address));
        sql.push_str(&member(true, &people(&a)));
    }
    // Text criteria: `/` (every column), Text… (subject and body), Subject….
    for (query, columns) in [(&f.contains, ""), (&f.text, "{subject body} : "), (&f.subject, "subject : ")] {
        if let Some(q) = query.as_deref() {
            sql.push_str(&text_clause(q, columns, thread, &mut bind));
        }
    }
    // Conversation date…: the date of the thread, the one the list is
    // sorted on (`ui.md`, "Deux critères de date"). Nothing to compute: a
    // bound of the walk of the index.
    if let Some(after) = f.after {
        let p = bind(Value::Integer(after));
        sql.push_str(&format!(" AND t.date_last_received >= {p}"));
    }
    if let Some(before) = f.before {
        let p = bind(Value::Integer(before));
        sql.push_str(&format!(" AND t.date_last_received < {p}"));
    }
    // Mail date…: a mail of the thread in the listed folders was received
    // in the interval. A set computed once, through
    // `idx_message_received (date_received)` in Everything, or
    // `idx_message_list (folder_id, date_received)` in a list of folders:
    // its cost follows the number of mails in the interval, which is why
    // it is a criterion of its own.
    if f.mail_after.is_some() || f.mail_before.is_some() {
        let mut range = Vec::new();
        if let Some(after) = f.mail_after {
            range.push(format!("m.date_received >= {}", bind(Value::Integer(after))));
        }
        if let Some(before) = f.mail_before {
            range.push(format!("m.date_received < {}", bind(Value::Integer(before))));
        }
        let set = format!(
            "SELECT m.thread_id FROM message m WHERE {} AND m.thread_id IS NOT NULL{}",
            range.join(" AND "),
            target.in_folders("m")
        );
        sql.push_str(&member(true, &set));
    }
    FilterSql { sql, values, by_set }
}

/// The view of a `message_unsubscribe` row (all `None` when the message has
/// none): the method follows from what the row holds.
fn unsubscribe_view(
    url: Option<String>,
    mailto: Option<String>,
    one_click: Option<bool>,
    source: Option<String>,
    unsubscribed_at: Option<i64>,
) -> Option<nm_api::UnsubscribeView> {
    let source = source?;
    let method = if one_click == Some(true) {
        "one_click"
    } else if url.is_some() {
        "browser"
    } else {
        "mail"
    };
    Some(nm_api::UnsubscribeView {
        method: method.into(),
        source,
        url,
        mailto,
        sender_unsubscribed_at: unsubscribed_at,
    })
}

/// Query plans of the list queries (`EXPLAIN QUERY PLAN`), checked on an
/// empty database, that is without planner statistics: the case where a
/// missing index once made a page of a large folder take 1.5 s. Timing a
/// query in a test would be slow and flaky; its plan is deterministic and
/// says whether the cost grows with the page or with the mailbox.
#[cfg(test)]
mod correspondent_tests {
    use super::*;

    /// A mail from `addr` (named `name`) in a folder of `role`.
    fn mail(id: i64, name: Option<&str>, addr: Option<&str>, role: Option<FolderRole>) -> ListMail {
        ListMail {
            id,
            name: name.map(str::to_string),
            addr: addr.map(str::to_string),
            snippet: None,
            has_attachments: false,
            role,
            kind: None,
        }
    }

    fn own() -> BTreeSet<String> {
        BTreeSet::from(["me@x.org".to_string()])
    }

    fn of(mails: &[ListMail]) -> Correspondent {
        correspondent(&mails.iter().collect::<Vec<_>>(), &own())
    }

    fn author(name: &str, mine: bool) -> Correspondent {
        Correspondent::Author(Some((name.to_string(), None)), mine)
    }

    const INBOX: Option<FolderRole> = Some(FolderRole::Inbox);

    #[test]
    fn the_latest_other_author_and_who_spoke_last() {
        let alice = mail(1, Some("Alice"), Some("alice@x.org"), INBOX);
        let me = mail(2, None, Some("ME@x.org "), INBOX);
        let bob = mail(3, Some("Bob"), Some("bob@x.org"), INBOX);
        assert_eq!(of(&[mail(1, Some("Alice"), Some("alice@x.org"), INBOX)]), author("Alice", false));
        assert_eq!(of(&[alice, me]), author("Alice", true));
        let alice = mail(1, Some("Alice"), Some("alice@x.org"), INBOX);
        let me = mail(2, None, Some("me@x.org"), INBOX);
        assert_eq!(of(&[alice, bob, me]), author("Bob", true));
    }

    #[test]
    fn sent_mail_is_the_users_whatever_its_address() {
        // An account `accounts.toml` no longer declares: its Sent says so.
        let alice = mail(1, Some("Alice"), Some("alice@x.org"), INBOX);
        let sent = mail(2, None, Some("old@x.org"), Some(FolderRole::Sent));
        assert_eq!(of(&[alice, sent]), author("Alice", true));
    }

    #[test]
    fn the_users_mails_alone_ask_for_the_latest_ones_recipient() {
        let first = mail(1, None, Some("me@x.org"), Some(FolderRole::Sent));
        let second = mail(2, None, Some("me@x.org"), Some(FolderRole::Waiting));
        assert_eq!(of(&[first, second]), Correspondent::RecipientOf(2));
    }

    #[test]
    fn a_draft_reply_does_not_count_until_sent() {
        // Alice asked and the user is writing: the conversation still
        // waits on them.
        let alice = mail(1, Some("Alice"), Some("alice@x.org"), INBOX);
        let draft = mail(2, None, Some("me@x.org"), Some(FolderRole::Drafts));
        assert_eq!(of(&[alice, draft]), author("Alice", false));
        // A new mail being written: its recipient, like a sent one.
        let draft = mail(3, None, Some("me@x.org"), Some(FolderRole::Drafts));
        assert_eq!(of(&[draft]), Correspondent::RecipientOf(3));
    }

    #[test]
    fn a_mail_without_sender_names_nobody_but_still_counts() {
        // No `From`: not the user's, but nothing to name; an earlier
        // author is named instead.
        let alice = mail(1, Some(" "), Some("alice@x.org"), INBOX);
        let anonymous = mail(2, None, None, INBOX);
        assert_eq!(of(&[alice, anonymous]), author("alice@x.org", false));
        assert_eq!(of(&[mail(1, None, None, INBOX)]), Correspondent::Author(None, false));
        assert_eq!(of(&[]), Correspondent::Author(None, false));
    }

    #[test]
    fn the_kind_is_that_of_the_mail_naming_the_correspondent() {
        // Bob, a robot, wrote last; Alice before him. The row names Bob,
        // so its kind is his mail's, not Alice's.
        let robot = |mut m: ListMail| {
            m.kind = Some(SenderKind::Robot);
            m
        };
        let person = |mut m: ListMail| {
            m.kind = Some(SenderKind::Person);
            m
        };
        let alice = person(mail(1, Some("Alice"), Some("alice@x.org"), INBOX));
        let bob = robot(mail(2, Some("Bob"), Some("bob@x.org"), INBOX));
        assert_eq!(of(&[alice, bob]), Correspondent::Author(Some(("Bob".into(), Some(SenderKind::Robot))), false));
        // A mail without sender is skipped: the kind follows the name.
        let alice = person(mail(1, Some("Alice"), Some("alice@x.org"), INBOX));
        let anonymous = robot(mail(2, None, None, INBOX));
        assert_eq!(
            of(&[alice, anonymous]),
            Correspondent::Author(Some(("Alice".into(), Some(SenderKind::Person))), false)
        );
        // The user's own reply carries no kind of the correspondent.
        let alice = person(mail(1, Some("Alice"), Some("alice@x.org"), INBOX));
        let me = person(mail(2, None, Some("me@x.org"), Some(FolderRole::Sent)));
        assert_eq!(of(&[alice, me]), Correspondent::Author(Some(("Alice".into(), Some(SenderKind::Person))), true));
    }
}

#[cfg(test)]
mod plan_tests {
    use super::*;
    use crate::Store;

    /// The plan of `sql`, one line per step ("SEARCH t USING INDEX …").
    fn plan(store: &Store, sql: &str, values: &[Value]) -> Vec<String> {
        let mut stmt = store.conn().prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
        stmt.query_map(rusqlite::params_from_iter(values.iter()), |r| r.get::<_, String>(3))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap()
    }

    /// Every filter of the attribute selector, one at a time, plus all of
    /// them together.
    fn filters() -> Vec<(&'static str, ThreadFilter)> {
        let v = |s: &str| vec![s.to_string()];
        let mut all = ThreadFilter {
            seen: Some(false),
            flagged: Some(true),
            attachments: Some(true),
            answered: Some(true),
            conversation: Some(true),
            from: v("alice"),
            not_from: v("@acme.fr"),
            to: v("bob"),
            not_to: v("@x.org"),
            contains: Some("devis -spam".into()),
            text: Some("rénovation".into()),
            subject: Some("facture".into()),
            after: Some(1),
            before: Some(2),
            mail_after: Some(1),
            mail_before: Some(2),
            tag: Some("Clients".into()),
            group_id: Some(1),
            address_id: Some(2),
            human: Some(true),
            thread_ids: vec![],
        };
        let mut out = vec![("none", ThreadFilter::default())];
        out.push(("seen", ThreadFilter { seen: Some(false), ..Default::default() }));
        out.push(("flagged", ThreadFilter { flagged: Some(true), ..Default::default() }));
        out.push(("not flagged", ThreadFilter { flagged: Some(false), ..Default::default() }));
        out.push(("answered", ThreadFilter { answered: Some(true), ..Default::default() }));
        out.push(("attachments", ThreadFilter { attachments: Some(true), ..Default::default() }));
        out.push(("human", ThreadFilter { human: Some(true), ..Default::default() }));
        out.push(("robot", ThreadFilter { human: Some(false), ..Default::default() }));
        out.push(("from", ThreadFilter { from: v("alice"), ..Default::default() }));
        out.push(("not from", ThreadFilter { not_from: v("@acme.fr"), ..Default::default() }));
        out.push(("to", ThreadFilter { to: v("bob"), ..Default::default() }));
        out.push(("contains", ThreadFilter { contains: Some("devis".into()), ..Default::default() }));
        out.push(("dates", ThreadFilter { after: Some(1), before: Some(2), ..Default::default() }));
        out.push(("mail dates", ThreadFilter { mail_after: Some(1), mail_before: Some(2), ..Default::default() }));
        out.push(("tag", ThreadFilter { tag: Some("Clients".into()), ..Default::default() }));
        out.push(("group", ThreadFilter { group_id: Some(1), ..Default::default() }));
        out.push(("contact", ThreadFilter { address_id: Some(2), ..Default::default() }));
        all.conversation = Some(true);
        out.push(("all", all));
        out
    }

    /// The steps a list query may take. A full read (`SCAN`) is only
    /// allowed on what is small or already narrowed: the list's own index
    /// of `thread`, walked newest first and stopped after a page, the
    /// address table (a LIKE needs to read it), the full-text index (it
    /// answers a MATCH itself) and the partial index of the messages with
    /// attachments, and that of the unread mails. A correlated subquery (one run per row of the list) is
    /// never allowed, nor reading a whole folder through `idx_message_list`
    /// (a range of dates in it, for Mail date…, is fine).
    fn check(what: &str, steps: &[String]) {
        for step in steps {
            assert!(!step.contains("CORRELATED"), "{what}: per-row subquery: {steps:?}");
            assert!(
                !step.contains("idx_message_list") || step.contains("date_received>") || step.contains("date_received<"),
                "{what}: reads a whole folder: {steps:?}"
            );
            if step.starts_with("SCAN ") {
                let allowed = ["SCAN t USING INDEX idx_thread_", "SCAN address", "SCAN message_fts VIRTUAL TABLE",
                    "SCAN m USING COVERING INDEX idx_message_thread_attachments", "SCAN CONSTANT ROW",
                    // The unread mails only, read when they are few
                    // (`UnreadPlan::Set`).
                    "SCAN message USING INDEX idx_message_unread"];
                assert!(allowed.iter().any(|a| step.starts_with(a)), "{what}: full scan `{step}`: {steps:?}");
            }
        }
    }

    #[test]
    fn row_details_use_the_thread_index() {
        let store = Store::open_in_memory().unwrap();
        let steps = plan(&store, THREAD_MESSAGES, &[Value::Integer(1)]);
        assert_eq!(steps[0], "SEARCH m USING INDEX idx_message_thread (thread_id=?)", "{steps:?}");
        let steps = plan(&store, FIRST_RECIPIENT, &[Value::Integer(1)]);
        assert!(steps[0].contains("ma USING PRIMARY KEY (message_id=?"), "{steps:?}");
        let steps = plan(&store, THREAD_IS_FLAGGED, &[Value::Integer(1)]);
        assert!(steps.iter().any(|s| s.contains("idx_message_thread_flagged (thread_id=?)")), "{steps:?}");
    }

    /// Every scope of a list, with the index it must walk without
    /// criterion, for all accounts, one account and some accounts.
    fn scopes() -> Vec<(Scope, [&'static str; 3])> {
        use FolderRole::*;
        vec![
            (Scope::Role(Inbox), ["idx_thread_role_received", "idx_thread_role_account_received", "idx_thread_role_received"]),
            (Scope::Role(Junk), ["idx_thread_role_received", "idx_thread_role_account_received", "idx_thread_role_received"]),
            (Scope::Flag(Sent), ["idx_thread_sent_received", "idx_thread_sent_account_received", "idx_thread_sent_received"]),
            (Scope::Flag(Drafts), ["idx_thread_draft_received", "idx_thread_draft_account_received", "idx_thread_draft_received"]),
            (Scope::Flag(Quarantine), ["idx_thread_quarantine_received", "idx_thread_quarantine_account_received", "idx_thread_quarantine_received"]),
            (Scope::Flag(Deleted), ["idx_thread_deleted_received", "idx_thread_deleted_account_received", "idx_thread_deleted_received"]),
            (Scope::AllMail, ["idx_thread_live_received", "idx_thread_live_account_received", "idx_thread_live_received"]),
            (Scope::Work, ["idx_thread_work_received", "idx_thread_work_account_received", "idx_thread_work_received"]),
            (Scope::Everything, ["idx_thread_received", "idx_thread_account_received", "idx_thread_received"]),
        ]
    }

    /// Every list, every filter, sort, account scope and cursor: the steps
    /// follow the rules of `check`, and the list itself is a walk of an
    /// index of `thread` (or the threads of a criterion's set, by primary
    /// key).
    #[test]
    fn every_list_walks_an_index_of_thread_with_every_filter() {
        let store = Store::open_in_memory().unwrap();
        let accounts = [Accounts::All, Accounts::One(1), Accounts::Some("1,2".into())];
        for (scope, _) in scopes() {
            for target in [Target::folders("1,2,3"), Target::everything()] {
                for ((name, f), unread) in filters().into_iter().flat_map(|x| [(x.clone(), UnreadPlan::Walk), (x, UnreadPlan::Set)]) {
                    let filter = filter_clause(&f, &target, &[1, 2], unread);
                    for a in &accounts {
                        for sort in [ThreadSort::Received, ThreadSort::Modified] {
                            for after in [None, Some((1, 1))] {
                                let (sql, values) = list_query(scope, a, after, 61, sort, &filter);
                                let steps = plan(&store, &sql, &values);
                                let what = format!("{scope:?} {name} {unread:?} {a:?} {sort:?} cursor={}", after.is_some());
                                check(&what, &steps);
                                assert!(
                                    steps[0].contains("t USING INDEX idx_thread_")
                                        || steps[0].contains("t USING COVERING INDEX idx_thread_")
                                        || steps[0] == "SEARCH t USING INTEGER PRIMARY KEY (rowid=?)",
                                    "{what}: {steps:?}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    /// Without criterion, each list is one walk of its own index, in the
    /// order of the sort: no other step, no sort of the rows.
    #[test]
    fn each_list_walks_its_own_index() {
        let store = Store::open_in_memory().unwrap();
        let accounts = [Accounts::All, Accounts::One(1), Accounts::Some("1,2".into())];
        for (scope, indexes) in scopes() {
            for (a, index) in accounts.iter().zip(indexes) {
                for sort in [ThreadSort::Received, ThreadSort::Modified] {
                    let index = if sort == ThreadSort::Modified { index.replace("_received", "_modified") } else { index.to_string() };
                    let (sql, values) = list_query(scope, a, None, 61, sort, &FilterSql::default());
                    let steps = plan(&store, &sql, &values);
                    assert_eq!(steps.len(), 1, "{scope:?} {a:?} {sort:?}: {steps:?}");
                    assert!(steps[0].contains(&format!("USING INDEX {index}")), "{scope:?} {a:?} {sort:?}: {steps:?}");
                }
            }
        }
    }

    /// Given threads (`ThreadFilter::thread_ids`, the reload of the
    /// conversations that changed): read by primary key in every list,
    /// whatever the scope, sort or cursor, never by a walk of a date
    /// index; and the criterion "read" does not bring in the set of the
    /// unread threads.
    #[test]
    fn given_threads_are_read_by_primary_key() {
        let store = Store::open_in_memory().unwrap();
        for seen in [None, Some(true)] {
            let f = ThreadFilter { thread_ids: vec![3, 5, 8], seen, ..Default::default() };
            let filter = filter_clause(&f, &Target::everything(), &[], UnreadPlan::Set);
            assert!(filter.by_set);
            assert!(filter.sql.contains("t.id IN (3,5,8)"), "{}", filter.sql);
            assert!(!filter.sql.contains("is_seen = 0"), "{}", filter.sql);
            for (scope, _) in scopes() {
                for a in [Accounts::All, Accounts::One(1), Accounts::Some("1,2".into())] {
                    for sort in [ThreadSort::Received, ThreadSort::Modified] {
                        let (sql, values) = list_query(scope, &a, None, 4, sort, &filter);
                        let steps = plan(&store, &sql, &values);
                        let what = format!("{scope:?} {a:?} {sort:?} seen={seen:?}");
                        assert!(steps[0].starts_with("SEARCH t USING INTEGER PRIMARY KEY"), "{what}: {steps:?}");
                    }
                }
            }
        }
    }

    /// With few unread mails (`UnreadPlan::Set`), every list reads the set
    /// of the unread threads first, then those threads by primary key,
    /// whatever the scope, sort or cursor: never a walk of a date index,
    /// which would go to the end of the mailbox when nothing is unread.
    #[test]
    fn few_unread_mails_are_read_from_their_set() {
        let store = Store::open_in_memory().unwrap();
        let f = ThreadFilter { seen: Some(false), ..Default::default() };
        for (scope, _) in scopes() {
            for a in [Accounts::All, Accounts::One(1), Accounts::Some("1,2".into())] {
                for sort in [ThreadSort::Received, ThreadSort::Modified] {
                    for after in [None, Some((1, 1))] {
                        let filter = filter_clause(&f, &Target::everything(), &[], UnreadPlan::Set);
                        assert!(filter.by_set);
                        let (sql, values) = list_query(scope, &a, after, 61, sort, &filter);
                        let steps = plan(&store, &sql, &values);
                        let what = format!("{scope:?} {a:?} {sort:?} cursor={}", after.is_some());
                        assert_eq!(steps[0], "SEARCH t USING INTEGER PRIMARY KEY (rowid=?)", "{what}: {steps:?}");
                        assert!(steps.iter().any(|s| s.contains("idx_message_unread")), "{what}: {steps:?}");
                    }
                }
            }
        }
        // Another criterion than unread keeps the walk: the set is only for
        // "unread".
        let read = ThreadFilter { seen: Some(true), ..Default::default() };
        assert!(!filter_clause(&read, &Target::everything(), &[], UnreadPlan::Set).by_set);
        assert!(!filter_clause(&ThreadFilter::default(), &Target::everything(), &[], UnreadPlan::Set).by_set);
    }

    /// The switch follows the unread mails of the listed folders.
    #[test]
    fn the_plan_switches_at_the_threshold() {
        assert_eq!(UnreadPlan::for_count(0), UnreadPlan::Set);
        assert_eq!(UnreadPlan::for_count(UNREAD_SET_MAX), UnreadPlan::Set);
        assert_eq!(UnreadPlan::for_count(UNREAD_SET_MAX + 1), UnreadPlan::Walk);
    }

    /// The decision reads the unread counters of the listed folders only,
    /// or of the folders of the account in Everything; without the unread
    /// criterion it is always the walk.
    #[test]
    fn the_plan_reads_the_counters_of_the_listed_folders() {
        let mut store = Store::open_in_memory().unwrap();
        crate::seed::seed(&mut store, &crate::seed::SeedOptions { threads: 50, seed: 3, now: 1_790_000_000 }).unwrap();
        let id = |role: &str| -> i64 {
            store.conn().query_row("SELECT id FROM folder WHERE role = ?1", [role], |r| r.get(0)).unwrap()
        };
        let (inbox, archive) = (id("inbox"), id("archive"));
        let unread = ThreadFilter { seen: Some(false), ..Default::default() };
        // Many unread mails in the Inbox, none in the archives.
        store.conn().execute("UPDATE folder SET unread_count = 0", []).unwrap();
        store.conn().execute("UPDATE folder SET unread_count = 5000 WHERE id = ?1", [inbox]).unwrap();
        assert_eq!(store.unread_plan(&unread, Some(&[inbox]), None).unwrap(), UnreadPlan::Walk);
        assert_eq!(store.unread_plan(&unread, Some(&[archive]), None).unwrap(), UnreadPlan::Set);
        assert_eq!(store.unread_plan(&unread, None, None).unwrap(), UnreadPlan::Walk);
        assert_eq!(store.unread_plan(&unread, None, Some(999)).unwrap(), UnreadPlan::Set);
        // Another criterion: nothing to choose.
        assert_eq!(store.unread_plan(&ThreadFilter::default(), Some(&[archive]), None).unwrap(), UnreadPlan::Walk);
    }

    /// Both plans give the same rows, page after page, in every scope: the
    /// set only narrows the threads, `t.unread_count` still decides (an
    /// unread mail in Deleted does not make a live thread unread).
    #[test]
    fn both_plans_give_the_same_pages() {
        let mut store = Store::open_in_memory().unwrap();
        crate::seed::seed(&mut store, &crate::seed::SeedOptions { threads: 400, seed: 7, now: 1_790_000_000 }).unwrap();
        // An unread mail in Deleted, in a thread whose other mails are read.
        store.conn().execute_batch(
            "UPDATE message SET is_seen = 0
              WHERE id = (SELECT m.id FROM message m JOIN folder f ON f.id = m.folder_id
                           WHERE f.role = 'deleted' ORDER BY m.id LIMIT 1)",
        ).unwrap();
        let f = ThreadFilter { seen: Some(false), ..Default::default() };
        let pages = |scope: Scope, unread: UnreadPlan| -> Vec<i64> {
            let filter = filter_clause(&f, &Target::everything(), &[], unread);
            let mut out = Vec::new();
            let mut after = None;
            loop {
                let (sql, values) = list_query(scope, &Accounts::All, after, 8, ThreadSort::Received, &filter);
                let rows = store.list_rows(&sql, values).unwrap();
                out.extend(rows.iter().map(|r| r.0));
                match rows.last() {
                    Some(r) if rows.len() == 8 => after = Some((r.2, r.0)),
                    _ => return out,
                }
            }
        };
        for (scope, _) in scopes() {
            let walk = pages(scope, UnreadPlan::Walk);
            assert_eq!(walk, pages(scope, UnreadPlan::Set), "{scope:?}");
        }
        assert!(!pages(Scope::Everything, UnreadPlan::Walk).is_empty());
    }

    /// Mail date… reads the mails of its interval through a date index,
    /// never a whole folder.
    #[test]
    fn mail_date_reads_a_range_of_dates() {
        let store = Store::open_in_memory().unwrap();
        let f = ThreadFilter { mail_after: Some(1), mail_before: Some(2), ..Default::default() };
        for target in [Target::folders("1"), Target::everything()] {
            let filter = filter_clause(&f, &target, &[], UnreadPlan::Walk);
            let (sql, values) = list_query(Scope::Everything, &Accounts::All, None, 61, ThreadSort::Received, &filter);
            let steps = plan(&store, &sql, &values);
            check("mail date", &steps);
            assert!(steps.iter().any(|s| s.contains("date_received>?")), "{steps:?}");
        }
    }
}
