//! Read queries: display-ready views, paginated by cursor.
//!
//! These methods take `&self` (shared access): they never modify the
//! database.

use nm_api::{AddressView, FolderView, MessageView, SearchHit, SegmentView, ThreadFilter, ThreadSort, ThreadSummary};
use rusqlite::types::Value;

/// A row of `thread_folder` joined with its thread: (thread id, subject,
/// last received, last modified, message count, unread count).
type ListRow = (i64, String, i64, i64, i64, i64);
use nm_core::{FolderRole, KW_WAS_TODO, KW_WAS_WAITING};
use rusqlite::{OptionalExtension, Row, params};

use crate::{Result, Store, StoreError};

/// The messages of a thread in one folder, oldest first: the details of a
/// list row (`thread_summary`). It must use `idx_message_thread (thread_id,
/// folder_id, …)`; through `idx_message_list (folder_id, …)`, which gives
/// the order for free, SQLite read the whole folder for every row of the
/// list (1.5 s for a page of a folder of 75 000 threads).
const THREAD_MESSAGES_IN_FOLDER: &str = "SELECT from_name, from_addr, snippet, has_attachments
       FROM message WHERE thread_id = ?1 AND folder_id = ?2
      ORDER BY date_received, id";

/// Whether a thread has a flagged message, in any folder (read from the
/// partial index `idx_message_thread_flagged`).
const THREAD_IS_FLAGGED: &str = "SELECT EXISTS (SELECT 1 FROM message WHERE thread_id = ?1 AND is_flagged = 1)";

/// Parses a list cursor: `<date_last_received>:<thread_id>` of the last row
/// of the previous page.
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

impl Store {
    /// Every folder shown in the sidebar, with its counters and the move
    /// targets it offers.
    pub fn list_folders(&self) -> Result<Vec<FolderView>> {
        // The `config` folder stores the synced configuration
        // (`synchronisation-configuration.md`); it is technical and hidden.
        let mut stmt = self.conn.prepare_cached(
            "SELECT f.id, f.role, f.path, f.total_count, f.unread_count,
                    (SELECT count(*) FROM thread_folder tf WHERE tf.folder_id = f.id),
                    a.id, a.config_key, a.kind
               FROM folder f JOIN account a ON a.id = f.account_id
              WHERE f.subscribed = 1 AND (f.role IS NULL OR f.role <> 'config')",
        )?;
        let mut items = stmt
            .query_map([], |r| {
                let role = role_of(r.get(1)?);
                let path: String = r.get(2)?;
                Ok(FolderView {
                    id: r.get(0)?,
                    role,
                    // Workflow folders use their standard name; any other
                    // folder falls back to its IMAP path.
                    name: role.map(|r| r.display_name().to_string()).unwrap_or(path),
                    letter: role.map(|r| r.letter().to_string()),
                    category: role.map(|r| r.category()),
                    total_count: r.get(3)?,
                    unread_count: r.get(4)?,
                    thread_count: r.get(5)?,
                    // `unwrap_or_default` gives an empty `Vec` when there is no role.
                    move_targets: role.map(|r| r.move_targets()).unwrap_or_default(),
                    account_id: r.get(6)?,
                    account_name: r.get(7)?,
                    account_kind: r.get(8)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        // Grouped by account (oldest account first), then workflow folders in
        // the order of the spec, then the others by name. `sort_by_key`
        // sorts on a computed key; tuples compare element by element, so the
        // account decides first, then the position, and the name breaks
        // ties. Folders without a role get `usize::MAX` and end up last.
        items.sort_by_key(|f| {
            (
                f.account_id,
                f.role
                    .and_then(|r| FolderRole::ALL.iter().position(|x| *x == r))
                    .unwrap_or(usize::MAX),
                f.name.clone(),
            )
        });
        Ok(items)
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
    /// is the last page).
    pub fn list_threads(
        &self,
        folder_id: i64,
        cursor: Option<&str>,
        limit: u32,
        sort: ThreadSort,
    ) -> Result<(Vec<ThreadSummary>, Option<String>)> {
        self.list_threads_in(&[folder_id], cursor, limit, sort, &ThreadFilter::default())
    }

    /// One page of the threads of several folders together, newest first:
    /// the unified view ("all") lists the folders of the same role in every
    /// account. (The virtual folder "Everything" has its own query,
    /// `list_everything`.) A thread in several of the folders is listed once
    /// (see `once` below).
    ///
    /// Each folder is read with its own index, `limit + 1` rows at most,
    /// then the pages are merged: the cost depends on the page size, not on
    /// the size of the folders (sorting the union of two folders of 40 000
    /// threads on every page would not).
    pub fn list_threads_in(
        &self,
        folder_ids: &[i64],
        cursor: Option<&str>,
        limit: u32,
        sort: ThreadSort,
        filter: &ThreadFilter,
    ) -> Result<(Vec<ThreadSummary>, Option<String>)> {
        if folder_ids.is_empty() {
            return Ok((vec![], None));
        }
        let mut roles = Vec::with_capacity(folder_ids.len());
        let mut accounts = Vec::with_capacity(folder_ids.len());
        for f in folder_ids {
            let (role, account) = self.folder_role_and_account(*f)?;
            roles.push((*f, role));
            accounts.push(account);
        }
        // Fetch one extra row: if it exists, there is a next page. This
        // avoids a separate `count(*)` query.
        let fetch = i64::from(limit) + 1;
        // The sort column. Both have their own index
        // (idx_thread_folder_list, idx_thread_folder_modified). It is
        // inserted into the SQL text, never taken from the client: only
        // these two constants are possible, so there is no injection risk.
        let key = match sort {
            ThreadSort::Received => "tf.date_last_received",
            ThreadSort::Modified => "tf.date_last_modified",
        };
        let after = cursor.map(parse_cursor).transpose()?;
        let mut rows: Vec<(i64, ListRow)> = Vec::new();
        // Closure that converts a row into a tuple, shared by both queries.
        let map = |r: &Row| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?));
        // The listed folders, as integers from the store written into the
        // SQL text (no user input).
        let set = folder_ids.iter().map(|f| f.to_string()).collect::<Vec<_>>().join(",");
        // Attribute filters: extra conditions, with their values bound after
        // the fixed parameters (`?1`… are folder, cursor and limit).
        let tags = self.tag_ids(filter)?;
        let (filter_sql, filter_values) = filter_clause(filter, &Target::folders(&set), &tags);
        // A thread belongs to one account, so it can only be in two listed
        // folders if two of them belong to the same account (never in the
        // unified view, which lists one folder per account). Then it is
        // listed once, in the folder of its latest message among them (ties:
        // the smallest folder id). Its rows in the other folders are
        // skipped, on every page, so pagination stays consistent.
        let shared_account = (1..accounts.len()).any(|i| accounts[..i].contains(&accounts[i]));
        let once = if shared_account {
            let col = key.trim_start_matches("tf.");
            format!(
                " AND NOT EXISTS (SELECT 1 FROM thread_folder tf2
                                   WHERE tf2.thread_id = tf.thread_id AND tf2.folder_id IN ({set})
                                     AND (tf2.{col} > tf.{col} OR (tf2.{col} = tf.{col} AND tf2.folder_id < tf.folder_id)))"
            )
        } else {
            String::new()
        };
        for &folder_id in folder_ids {
            let (sql, values) = folder_query(key, &once, &filter_sql, &filter_values, folder_id, fetch, after);
            let mut stmt = self.conn.prepare_cached(&sql)?;
            let page: Vec<ListRow> =
                stmt.query_map(rusqlite::params_from_iter(values), map)?.collect::<std::result::Result<_, _>>()?;
            rows.extend(page.into_iter().map(|r| (folder_id, r)));
        }
        // Merge: newest first on the sort key, then the thread id, like the
        // index order. A single folder is already in that order.
        if folder_ids.len() > 1 {
            let sort_key = |r: &(i64, ListRow)| {
                let (_, (id, _, received, modified, _, _)) = r;
                (if sort == ThreadSort::Modified { *modified } else { *received }, *id)
            };
            rows.sort_by_key(|r| std::cmp::Reverse(sort_key(r)));
        }

        // `as i64` is an explicit numeric conversion; Rust never converts
        // between integer types implicitly.
        let has_more = rows.len() as i64 > i64::from(limit);
        let mut items = Vec::with_capacity(rows.len().min(limit as usize));
        // `into_iter` consumes `rows`; `take(limit)` drops the extra rows.
        // The tuple is destructured directly in the `for` pattern.
        for (folder_id, (thread_id, subject, received, modified, count, unread)) in rows.into_iter().take(limit as usize) {
            let role = roles.iter().find(|(f, _)| *f == folder_id).and_then(|(_, r)| *r);
            items.push(self.thread_summary(thread_id, folder_id, role, subject, (received, modified), count, unread)?);
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

    /// One page of the virtual folder "Everything" (`ui.md`): every folder,
    /// of one account or of all of them, newest first.
    ///
    /// Every folder means whole threads, so the list is read from `thread`
    /// itself, whose aggregates (`date_last_received`,
    /// `date_last_modified`, counters) span all folders: one walk of
    /// `idx_thread_received` (or its per-account and "modified" variants),
    /// whatever the number of accounts and folders. Listing it folder by
    /// folder, as `list_threads_in` does, ran one query per folder (156 for
    /// thirteen accounts) and had to check, for every row, that the thread
    /// was not also in a more recent folder.
    ///
    /// Each thread is shown in the folder of its latest message (on the
    /// sort key; ties: the smallest folder id), with that folder's counters,
    /// exactly as when Everything was listed folder by folder.
    pub fn list_everything(
        &self,
        account_id: Option<i64>,
        cursor: Option<&str>,
        limit: u32,
        sort: ThreadSort,
        filter: &ThreadFilter,
    ) -> Result<(Vec<ThreadSummary>, Option<String>)> {
        let fetch = i64::from(limit) + 1;
        let after = cursor.map(parse_cursor).transpose()?;
        let tags = self.tag_ids(filter)?;
        let (sql, values) = everything_query(account_id, after, fetch, sort, filter, &tags);
        // Constant of ours, never client input (see `list_threads_in`).
        let tf_key = match sort {
            ThreadSort::Received => "tf.date_last_received",
            ThreadSort::Modified => "tf.date_last_modified",
        };
        let rows: Vec<(i64, String, i64)> = self
            .conn
            .prepare_cached(&sql)?
            .query_map(rusqlite::params_from_iter(values), |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<std::result::Result<_, _>>()?;

        let has_more = rows.len() as i64 > i64::from(limit);
        // The folder each thread is shown in (`shown_in_sql`).
        let mut shown_in = self.conn.prepare_cached(&shown_in_sql(tf_key))?;
        let mut items = Vec::with_capacity(rows.len().min(limit as usize));
        let mut last_key = None;
        for (thread_id, subject, sort_value) in rows.into_iter().take(limit as usize) {
            last_key = Some((sort_value, thread_id));
            let row = shown_in
                .query_row([thread_id], |r| {
                    Ok((r.get::<_, i64>(0)?, role_of(r.get(1)?), r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))
                })
                .optional()?;
            // A thread with messages always has a folder row; skip rather
            // than fail if an inconsistency ever slips in.
            let Some((folder_id, role, received, modified, count, unread)) = row else { continue };
            items.push(self.thread_summary(thread_id, folder_id, role, subject, (received, modified), count, unread)?);
        }
        // The cursor is the sort key of the thread itself (equal to that of
        // the folder it is shown in), so the next page continues the walk of
        // the index where this one stopped.
        let next_cursor = if has_more { last_key.map(|(d, id)| format!("{d}:{id}")) } else { None };
        Ok((items, next_cursor))
    }

    /// Completes a list row with the details that `thread_folder` does not
    /// store: participants, snippet, attachments, flag and return mark.
    ///
    /// This runs a few small queries per thread (an "N+1" pattern). With a
    /// page of 50 threads and indexed lookups, it costs a few milliseconds;
    /// if it ever shows up in profiling, these fields can be denormalized.
    ///
    /// `#[allow(...)]` silences the clippy lint about functions with more
    /// than 7 parameters.
    ///
    /// `dates` is (last received, last modified) in the folder.
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
    ) -> Result<ThreadSummary> {
        // The thread's messages in this folder, oldest first.
        let mut stmt = self.conn.prepare_cached(THREAD_MESSAGES_IN_FOLDER)?;
        let mut participants: Vec<String> = Vec::new();
        let mut snippet = None;
        let mut has_attachments = false;
        // `query` gives a cursor over rows; `while let` loops as long as
        // `rows.next()?` returns `Some(row)`.
        let mut rows = stmt.query(params![thread_id, folder_id])?;
        while let Some(r) = rows.next()? {
            let name: Option<String> = r.get(0)?;
            let addr: Option<String> = r.get(1)?;
            // `or` keeps the name if present, otherwise the address.
            if let Some(p) = name.or(addr) {
                // Remove a previous occurrence then append: each participant
                // appears once, at the position of their latest message.
                participants.retain(|x| x != &p);
                participants.push(p);
            }
            // Keep the snippet of the latest message that has one.
            snippet = r.get::<_, Option<String>>(2)?.or(snippet);
            has_attachments |= r.get::<_, bool>(3)?;
        }

        // The flag is looked up across the whole thread, not only this
        // folder: a flagged message in Sent still flags the conversation.
        let is_flagged: bool = self.conn.prepare_cached(THREAD_IS_FLAGGED)?
        .query_row([thread_id], |r| r.get(0))?;

        // The return mark only matters in the Inbox, where the thread landed
        // after leaving Todo or Waiting.
        let returned_from = if role == Some(FolderRole::Inbox) {
            let mut stmt = self.conn.prepare_cached(
                "SELECT DISTINCT mk.keyword FROM message m
                   JOIN message_keyword mk ON mk.message_id = m.id
                  WHERE m.thread_id = ?1 AND m.folder_id = ?2
                    AND mk.keyword COLLATE NOCASE IN (?3, ?4)",
            )?;
            let kws: Vec<String> = stmt
                .query_map(params![thread_id, folder_id, KW_WAS_TODO, KW_WAS_WAITING], |r| r.get(0))?
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

    /// Every message of a thread, across all folders, oldest first.
    /// Returns the thread's subject and its messages.
    pub fn get_thread(&self, thread_id: i64) -> Result<(String, Vec<MessageView>)> {
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
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        // Recipients of each message, in header order.
        let mut addr_stmt = self.conn.prepare_cached(
            "SELECT ma.role, a.addr, a.name FROM message_address ma
               JOIN address a ON a.id = ma.address_id
              WHERE ma.message_id = ?1 AND ma.role IN ('to', 'cc')
              ORDER BY ma.role, ma.position",
        )?;
        // `&mut messages` iterates with mutable references, so each `m` can
        // be modified in place.
        for m in &mut messages {
            let mut rows = addr_stmt.query([m.id])?;
            while let Some(r) = rows.next()? {
                let role: String = r.get(0)?;
                let view = AddressView { addr: r.get(1)?, name: r.get(2)? };
                if role == "to" { m.to.push(view) } else { m.cc.push(view) }
            }
        }
        // Attachments of the whole thread in one query, then dealt out to
        // their messages (`attachments.rs`).
        for (message_id, a) in self.thread_attachments(thread_id)? {
            if let Some(m) = messages.iter_mut().find(|m| m.id == message_id) {
                m.attachments.push(a);
            }
        }
        present_thread(&mut messages);
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
        // `snippet(...)` is an FTS5 function returning an excerpt around the
        // matches: column -1 means "best column", '[' and ']' wrap the
        // matched terms, '…' marks cut text, 12 is the excerpt length in
        // tokens. `rank` is FTS5's relevance score (BM25), lower is better.
        let mut stmt = self.conn.prepare_cached(
            "SELECT m.id, m.thread_id, m.folder_id, f.role, m.subject, m.from_name, m.from_addr,
                    m.date_received, snippet(message_fts, -1, '[', ']', '…', 12)
               FROM message_fts
               JOIN message m ON m.id = message_fts.rowid
               JOIN folder f ON f.id = m.folder_id
              WHERE message_fts MATCH ?1
              ORDER BY rank
              LIMIT ?2",
        )?;
        let hits = stmt
            .query_map(params![fts_query, limit], |r| {
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

/// The list query of one folder of `list_threads_in` and its parameters:
/// `once` and `filter_sql` are the extra conditions, `filter_values` the
/// values of the filters.
fn folder_query(
    key: &str,
    once: &str,
    filter_sql: &str,
    filter_values: &[Value],
    folder_id: i64,
    fetch: i64,
    after: Option<(i64, i64)>,
) -> (String, Vec<Value>) {
    let select = format!(
        "SELECT tf.thread_id, t.subject, tf.date_last_received, tf.date_last_modified,
                tf.message_count, tf.unread_count
           FROM thread_folder tf JOIN thread t ON t.id = tf.thread_id
          WHERE tf.folder_id = ?1{once}{filter_sql}"
    );
    // Two separate queries rather than one with `?2 IS NULL OR ...`: such a
    // condition would prevent SQLite from using the index for the row-value
    // comparison, and every page would scan the folder. Fixed parameters
    // first, then the filter values (numbered from `?10`, see
    // `filter_clause`).
    let (sql, mut values): (String, Vec<Value>) = match after {
        None => (
            format!("{select} ORDER BY {key} DESC, tf.thread_id DESC LIMIT ?2"),
            vec![Value::Integer(folder_id), Value::Integer(fetch)],
        ),
        // `(a, b) < (x, y)` is a row-value comparison: true when a < x, or
        // a = x and b < y. It matches the sort order.
        Some((d, id)) => (
            format!("{select} AND ({key}, tf.thread_id) < (?3, ?4) ORDER BY {key} DESC, tf.thread_id DESC LIMIT ?2"),
            vec![Value::Integer(folder_id), Value::Integer(fetch), Value::Integer(d), Value::Integer(id)],
        ),
    };
    // SQLite counts parameters up to the highest number used: pad the
    // unused numbers (up to `?9`) only when filters follow.
    if !filter_values.is_empty() {
        values.resize(9, Value::Null);
        values.extend(filter_values.iter().cloned());
    }
    (sql, values)
}

/// The list query of `list_everything` and its parameters.
fn everything_query(
    account_id: Option<i64>,
    after: Option<(i64, i64)>,
    fetch: i64,
    sort: ThreadSort,
    filter: &ThreadFilter,
    tags: &[i64],
) -> (String, Vec<Value>) {
    // Constants of ours, never client input (see `list_threads_in`).
    let key = match sort {
        ThreadSort::Received => "t.date_last_received",
        ThreadSort::Modified => "t.date_last_modified",
    };
    let (filter_sql, filter_values) = filter_clause(filter, &Target::everything(), tags);
    // `message_count > 0`: a thread whose messages were all deleted keeps
    // its row until it is cleaned up, and has nothing to show.
    let mut sql = format!("SELECT t.id, t.subject, {key} FROM thread t WHERE t.message_count > 0{filter_sql}");
    // Fixed parameters: ?1 account, ?2 limit, ?3 and ?4 cursor; the filter
    // values start at ?10. SQLite counts parameters up to the highest number
    // used, so unused numbers are bound to NULL.
    let mut values = vec![Value::Null, Value::Integer(fetch)];
    if let Some(a) = account_id {
        sql.push_str(" AND t.account_id = ?1");
        values[0] = Value::Integer(a);
    }
    if let Some((d, id)) = after {
        sql.push_str(&format!(" AND ({key}, t.id) < (?3, ?4)"));
        values.extend([Value::Integer(d), Value::Integer(id)]);
    }
    sql.push_str(&format!(" ORDER BY {key} DESC, t.id DESC LIMIT ?2"));
    if !filter_values.is_empty() {
        values.resize(9, Value::Null);
        values.extend(filter_values);
    }
    (sql, values)
}

/// The folder a thread of Everything is shown in, with its counters there:
/// the row of `thread_folder` with the latest date on the sort key `tf_key`
/// (ties: the smallest folder id), read through its primary key (a thread
/// is in a handful of folders).
fn shown_in_sql(tf_key: &str) -> String {
    format!(
        "SELECT tf.folder_id, f.role, tf.date_last_received, tf.date_last_modified, tf.message_count, tf.unread_count
           FROM thread_folder tf JOIN folder f ON f.id = tf.folder_id
          WHERE tf.thread_id = ?1
          ORDER BY {tf_key} DESC, tf.folder_id LIMIT 1"
    )
}

/// Simplified presentation of a thread, computed by the core so every
/// client shows the same thing (`clean.rs`, `docs/thread-display.md`):
/// segments of each message, messages folded by default, recipients only
/// when they change.
fn present_thread(messages: &mut [MessageView]) {
    let inputs: Vec<(Option<&str>, &str)> =
        messages.iter().map(|m| (m.from_addr.as_deref(), m.body_text.as_deref().unwrap_or(""))).collect();
    let cleaned = crate::clean::clean_thread(&inputs);
    let last = messages.len().saturating_sub(1);
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
        m.show_recipients = match &previous {
            None => recipients.len() > 1,
            Some(p) => *p != recipients,
        };
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

/// One FTS5 phrase: the word quoted, as a prefix, behind a column filter
/// (`{subject body} : ` restricts it to those columns; empty: every
/// column).
fn fts_phrase(word: &str, columns: &str) -> String {
    format!("{columns}\"{word}\"*")
}

/// What the attribute filters apply to: the columns of the list query, and
/// the folders whose messages count.
///
/// A folder list (`list_threads_in`) reads `thread_folder tf JOIN thread t`;
/// Everything (`list_everything`) reads `thread t` alone. The column names
/// are constants of ours written into the SQL text, never client input.
struct Target<'a> {
    /// The thread id of the row.
    thread: &'static str,
    /// Unread messages of the row (in the folder, or in the whole thread).
    unread: &'static str,
    /// Date of the latest message of the row.
    received: &'static str,
    /// The listed folders (comma-separated ids), for the criteria that only
    /// look at the messages of those folders (`ui.md`, "Négation": From,
    /// To and attachments). `None` in Everything: every folder is listed,
    /// so the whole thread counts.
    folders: Option<&'a str>,
}

impl<'a> Target<'a> {
    fn folders(set: &'a str) -> Target<'a> {
        Target { thread: "tf.thread_id", unread: "tf.unread_count", received: "tf.date_last_received", folders: Some(set) }
    }

    fn everything() -> Target<'static> {
        Target { thread: "t.id", unread: "t.unread_count", received: "t.date_last_received", folders: None }
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
        sql.push_str(&format!(" AND {thread} IN ({})", in_fts(bind(Value::Text(q)))));
    }
    if !excluded.is_empty() {
        // Any excluded word leaves the thread out: the words are OR-ed.
        let q = excluded.iter().map(|w| fts_phrase(w, columns)).collect::<Vec<_>>().join(" OR ");
        sql.push_str(&format!(" AND {thread} NOT IN ({})", in_fts(bind(Value::Text(q)))));
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
fn filter_clause(f: &ThreadFilter, target: &Target, tags: &[i64]) -> (String, Vec<Value>) {
    let mut sql = String::new();
    let mut values: Vec<Value> = Vec::new();
    let thread = target.thread;
    // " AND <thread> IN (<set>)", or NOT IN for `false` / an exclusion.
    let member = |yes: bool, set: &str| format!(" AND {thread} {}IN ({set})", if yes { "" } else { "NOT " });
    if let Some(seen) = f.seen {
        let unread = target.unread;
        sql.push_str(&if seen { format!(" AND {unread} = 0") } else { format!(" AND {unread} > 0") });
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
    let received = target.received;
    if let Some(after) = f.after {
        let p = bind(Value::Integer(after));
        sql.push_str(&format!(" AND {received} >= {p}"));
    }
    if let Some(before) = f.before {
        let p = bind(Value::Integer(before));
        sql.push_str(&format!(" AND {received} < {p}"));
    }
    (sql, values)
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
            tag: Some("Clients".into()),
            group_id: Some(1),
            address_id: Some(2),
        };
        let mut out = vec![("none", ThreadFilter::default())];
        out.push(("seen", ThreadFilter { seen: Some(false), ..Default::default() }));
        out.push(("flagged", ThreadFilter { flagged: Some(true), ..Default::default() }));
        out.push(("not flagged", ThreadFilter { flagged: Some(false), ..Default::default() }));
        out.push(("answered", ThreadFilter { answered: Some(true), ..Default::default() }));
        out.push(("attachments", ThreadFilter { attachments: Some(true), ..Default::default() }));
        out.push(("from", ThreadFilter { from: v("alice"), ..Default::default() }));
        out.push(("not from", ThreadFilter { not_from: v("@acme.fr"), ..Default::default() }));
        out.push(("to", ThreadFilter { to: v("bob"), ..Default::default() }));
        out.push(("contains", ThreadFilter { contains: Some("devis".into()), ..Default::default() }));
        out.push(("dates", ThreadFilter { after: Some(1), before: Some(2), ..Default::default() }));
        out.push(("tag", ThreadFilter { tag: Some("Clients".into()), ..Default::default() }));
        out.push(("group", ThreadFilter { group_id: Some(1), ..Default::default() }));
        out.push(("contact", ThreadFilter { address_id: Some(2), ..Default::default() }));
        all.conversation = Some(true);
        out.push(("all", all));
        out
    }

    /// The steps a list query may take. A full read (`SCAN`) is only
    /// allowed on what is small or already narrowed: the list's own index,
    /// the address table (a LIKE needs to read it), the full-text index
    /// (it answers a MATCH itself) and the partial index of the messages
    /// with attachments. A correlated subquery (one run per row of the list)
    /// is never allowed, nor reading a whole folder through
    /// `idx_message_list`.
    fn check(what: &str, steps: &[String]) {
        for step in steps {
            assert!(!step.contains("CORRELATED"), "{what}: per-row subquery: {steps:?}");
            assert!(!step.contains("idx_message_list"), "{what}: reads a whole folder: {steps:?}");
            if step.starts_with("SCAN ") {
                let allowed = ["SCAN t USING INDEX idx_thread_", "SCAN address", "SCAN message_fts VIRTUAL TABLE",
                    "SCAN m USING COVERING INDEX idx_message_thread_attachments", "SCAN CONSTANT ROW"];
                assert!(allowed.iter().any(|a| step.starts_with(a)), "{what}: full scan `{step}`: {steps:?}");
            }
        }
    }

    #[test]
    fn row_details_use_the_thread_and_folder_index() {
        let store = Store::open_in_memory().unwrap();
        let steps = plan(&store, THREAD_MESSAGES_IN_FOLDER, &[Value::Integer(1), Value::Integer(1)]);
        assert_eq!(steps, ["SEARCH message USING INDEX idx_message_thread (thread_id=? AND folder_id=?)"]);
        let steps = plan(&store, THREAD_IS_FLAGGED, &[Value::Integer(1)]);
        assert!(steps.iter().any(|s| s.contains("idx_message_thread_flagged (thread_id=?)")), "{steps:?}");
        for key in ["tf.date_last_received", "tf.date_last_modified"] {
            let steps = plan(&store, &shown_in_sql(key), &[Value::Integer(1)]);
            assert_eq!(steps[0], "SEARCH tf USING PRIMARY KEY (thread_id=?)");
            check("shown_in", &steps);
        }
    }

    #[test]
    fn everything_walks_a_thread_index_with_every_filter() {
        let store = Store::open_in_memory().unwrap();
        for (name, f) in filters() {
            for account in [None, Some(1)] {
                for sort in [ThreadSort::Received, ThreadSort::Modified] {
                    for after in [None, Some((1, 1))] {
                        let (sql, values) = everything_query(account, after, 61, sort, &f, &[1, 2]);
                        let steps = plan(&store, &sql, &values);
                        let what = format!("everything {name} account={account:?} {sort:?} cursor={}", after.is_some());
                        check(&what, &steps);
                        // The list itself: a thread index, or the threads
                        // of a criterion's set, by primary key.
                        assert!(
                            steps[0].contains("USING INDEX idx_thread_") || steps[0] == "SEARCH t USING INTEGER PRIMARY KEY (rowid=?)",
                            "{what}: {steps:?}"
                        );
                    }
                }
            }
        }
        // Without filter, the index of the sort and the scope, nothing else.
        let first = |account, sort| {
            let (sql, values) = everything_query(account, None, 61, sort, &ThreadFilter::default(), &[]);
            plan(&store, &sql, &values)
        };
        assert_eq!(first(None, ThreadSort::Received), ["SCAN t USING INDEX idx_thread_received"]);
        assert_eq!(first(None, ThreadSort::Modified), ["SCAN t USING INDEX idx_thread_modified"]);
        assert_eq!(first(Some(1), ThreadSort::Received), ["SEARCH t USING INDEX idx_thread_account_received (account_id=?)"]);
        assert_eq!(first(Some(1), ThreadSort::Modified), ["SEARCH t USING INDEX idx_thread_account_modified (account_id=?)"]);
    }

    #[test]
    fn a_folder_list_walks_its_index_with_every_filter() {
        let store = Store::open_in_memory().unwrap();
        for (name, f) in filters() {
            for key in ["tf.date_last_received", "tf.date_last_modified"] {
                for after in [None, Some((1, 1))] {
                    let (filter_sql, filter_values) = filter_clause(&f, &Target::folders("1,2"), &[1, 2]);
                    let (sql, values) = folder_query(key, "", &filter_sql, &filter_values, 1, 61, after);
                    let steps = plan(&store, &sql, &values);
                    let what = format!("folder {name} {key} cursor={}", after.is_some());
                    check(&what, &steps);
                    assert!(steps[0].starts_with("SEARCH tf USING INDEX idx_thread_folder_"), "{what}: {steps:?}");
                }
            }
        }
    }
}
