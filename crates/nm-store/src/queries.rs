//! Read queries: display-ready views, paginated by cursor.
//!
//! These methods take `&self` (shared access): they never modify the
//! database.

use nm_api::{AddressView, FolderView, MessageView, SearchHit, SegmentView, ThreadSort, ThreadSummary};

/// A row of `thread_folder` joined with its thread: (thread id, subject,
/// last received, last modified, message count, unread count).
type ListRow = (i64, String, i64, i64, i64, i64);
use nm_core::{FolderRole, KW_WAS_TODO, KW_WAS_WAITING};
use rusqlite::{OptionalExtension, Row, params};

use crate::{Result, Store, StoreError};

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

    /// Role of a folder, or `NotFound` if the folder does not exist.
    fn folder_role(&self, folder_id: i64) -> Result<Option<FolderRole>> {
        // Two levels of Option: the outer one says whether a row was found,
        // the inner one whether its `role` column is NULL.
        let role: Option<Option<String>> = self
            .conn
            .query_row("SELECT role FROM folder WHERE id = ?1", [folder_id], |r| r.get(0))
            .optional()?;
        match role {
            None => Err(StoreError::NotFound(format!("dossier {folder_id}"))),
            Some(r) => Ok(role_of(r)),
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
        let role = self.folder_role(folder_id)?;
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
        // Two separate queries rather than one with `?2 IS NULL OR ...`: such
        // a condition would prevent SQLite from using the index for the
        // row-value comparison, and every page would scan the folder.
        let rows: Vec<ListRow> = {
            // Closure that converts a row into a tuple, shared by both queries.
            let map = |r: &Row| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?));
            let select = "SELECT tf.thread_id, t.subject, tf.date_last_received, tf.date_last_modified,
                                 tf.message_count, tf.unread_count
                            FROM thread_folder tf JOIN thread t ON t.id = tf.thread_id
                           WHERE tf.folder_id = ?1";
            match cursor {
                None => {
                    let sql = format!("{select} ORDER BY {key} DESC, tf.thread_id DESC LIMIT ?2");
                    let mut stmt = self.conn.prepare_cached(&sql)?;
                    stmt.query_map(params![folder_id, fetch], map)?.collect::<std::result::Result<_, _>>()?
                }
                Some(c) => {
                    let (d, id) = parse_cursor(c)?;
                    // `(a, b) < (x, y)` is a row-value comparison: true when
                    // a < x, or a = x and b < y. It matches the sort order.
                    let sql = format!(
                        "{select} AND ({key}, tf.thread_id) < (?2, ?3) ORDER BY {key} DESC, tf.thread_id DESC LIMIT ?4"
                    );
                    let mut stmt = self.conn.prepare_cached(&sql)?;
                    stmt.query_map(params![folder_id, d, id, fetch], map)?
                        .collect::<std::result::Result<_, _>>()?
                }
            }
        };

        // `as i64` is an explicit numeric conversion; Rust never converts
        // between integer types implicitly.
        let has_more = rows.len() as i64 > i64::from(limit);
        let mut items = Vec::with_capacity(rows.len().min(limit as usize));
        // `into_iter` consumes `rows`; `take(limit)` drops the extra row.
        // The tuple is destructured directly in the `for` pattern.
        for (thread_id, subject, received, modified, count, unread) in rows.into_iter().take(limit as usize) {
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
        let mut stmt = self.conn.prepare_cached(
            "SELECT from_name, from_addr, snippet, has_attachments
               FROM message WHERE thread_id = ?1 AND folder_id = ?2
              ORDER BY date_received, id",
        )?;
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
        let is_flagged: bool = self.conn.prepare_cached(
            "SELECT EXISTS (SELECT 1 FROM message WHERE thread_id = ?1 AND is_flagged = 1)",
        )?
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
                    b.html_sanitized, coalesce(b.has_remote_content, 0), a.body_format
               FROM message m
               JOIN folder f ON f.id = m.folder_id
               LEFT JOIN message_body b ON b.message_id = m.id
               LEFT JOIN address a ON a.id = m.from_address_id
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
        let fts_query = query
            .split_whitespace()
            .map(|w| w.replace('"', ""))
            .filter(|w| !w.is_empty())
            .map(|w| format!("\"{w}\"*"))
            .collect::<Vec<_>>()
            .join(" ");
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
