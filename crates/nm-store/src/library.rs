//! Features around the mail itself, used by the screens of the app
//! (`ui.md`): display preference per address, export of a message to a
//! file, history (audit log), tags, contact groups and contacts. The
//! threads of a tag, a group or a contact are criteria of the thread lists
//! (`ThreadFilter`, `queries.rs`).

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use nm_api::{ContactView, ExportFormat, GroupView, HistoryEntry, HistoryGroup, TagView, ThreadLabels};
use nm_core::Source;
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use serde_json::Value;

use crate::raw::RawStore;

/// What `unsubscribe_target` finds for a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsubscribeTarget {
    pub url: Option<String>,
    pub mailto: Option<String>,
    /// RFC 8058: one POST to `url` unsubscribes.
    pub one_click: bool,
    /// `header` or `body`.
    pub source: String,
}
use crate::{ChangeSet, Result, Store, StoreError, refresh_thread};

/// A message as the exports show it (`Store::message_parts`).
struct MessageParts {
    subject: String,
    /// `Name <addr>`, or the bare address.
    from: String,
    /// Recipients, `Name <addr>` joined by ", "; empty when none.
    to: String,
    cc: String,
    /// `YYYY-MM-DD HH:MM`, local time (`format_datetime`).
    when: String,
    /// The stored text body, as is.
    body: String,
    /// Hash of the raw message, for `.eml`.
    hash: Option<Vec<u8>>,
}

/// The header lines of the text export and of the clipboard text, each
/// ended by a line break; `Cc` only when there is one.
fn text_headers(from: &str, to: &str, cc: &str, when: &str, subject: &str) -> String {
    let mut s = format!("From: {from}\nTo: {to}\n");
    if !cc.is_empty() {
        s.push_str(&format!("Cc: {cc}\n"));
    }
    s.push_str(&format!("Date: {when}\nSubject: {subject}\n"));
    s
}

/// Splits a `<number>:<number>` cursor (date and id of the last row).
fn parse_pair(cursor: &str) -> Result<(i64, i64)> {
    let bad = || StoreError::InvalidParams(format!("invalid cursor: {cursor}"));
    let (a, b) = cursor.split_once(':').ok_or_else(bad)?;
    Ok((a.parse().map_err(|_| bad())?, b.parse().map_err(|_| bad())?))
}

/// What `list_history_groups` gives.
#[derive(Debug, Clone, Default)]
pub struct HistoryGroupQuery {
    /// Also the mails marked as read.
    pub include_read: bool,
    /// The lines between these two ids, each on its own: the detail of a
    /// group.
    pub range: Option<(i64, i64)>,
    /// Only the lines of these sources (`Source::as_str`): who did it, as
    /// the screen's filter chooses; every source when empty.
    pub sources: Vec<String>,
}

/// Two lines further apart than this are two actions, even if they do the
/// same thing: a command writes its lines at once, in one transaction.
const HISTORY_GROUP_GAP_SECS: i64 = 2;

/// What a line of the log did: a code for the client to word, and what it
/// was done with (`HistoryGroup::kind`, `HistoryGroup::value`). Pure, so that
/// the words of the screen can be tested from the log's own lines.
///
/// `folder` names a folder id; it is a parameter so that the scan of the log
/// reads each folder once.
fn history_change(
    action: &str,
    detail: &Value,
    folder: &mut impl FnMut(i64) -> Result<String>,
) -> Result<(String, Option<String>)> {
    let text = |v: &Value| v.as_str().map(str::to_string);
    Ok(match action {
        "move" => ("move".into(), Some(folder(detail["after"]["folder_id"].as_i64().unwrap_or(-1))?)),
        "set_flags" => {
            let flag = detail["after"].as_object().and_then(|o| o.iter().next());
            let on = flag.is_some_and(|(_, v)| v.as_i64() == Some(1) || v.as_bool() == Some(true));
            let kind = match flag.map(|(k, _)| k.as_str()) {
                Some("is_seen") if on => "read",
                Some("is_seen") => "unread",
                Some("is_flagged") if on => "flagged",
                Some("is_flagged") => "unflagged",
                _ => action,
            };
            (kind.into(), None)
        }
        "set_tags" => {
            let kind = if detail.get("remove").is_some() { "tag_removed" } else { "tag_added" };
            (kind.into(), text(&detail["tag"]))
        }
        "screening" => match text(&detail["after"]) {
            Some(target) => ("decision".into(), Some(target)),
            None => ("decision_removed".into(), None),
        },
        "screen" => ("screen".into(), text(&detail["folder"])),
        "add_member" | "remove_member" => (action.into(), text(&detail["addr"])),
        "import_mbox" => (action.into(), detail["imported"].as_i64().map(|n| n.to_string())),
        _ => (action.into(), None),
    })
}

/// A group of the log being read, newest line first.
struct GroupBuilder {
    group: HistoryGroup,
    detail: Value,
    /// The date of its oldest line so far.
    oldest: i64,
}

impl GroupBuilder {
    /// Whether the line `id`, written at `date`, continues this action.
    fn continues(&self, id: i64, date: i64, source: &str, entity: &str, kind: &str, value: &Option<String>) -> bool {
        let g = &self.group;
        g.first_id == id + 1
            && self.oldest - date <= HISTORY_GROUP_GAP_SECS
            && g.source == source
            && g.entity == entity
            && g.kind == kind
            && g.value == *value
    }
}

/// What `list_history_filtered` keeps; everything by default.
#[derive(Debug, Clone, Default)]
pub struct HistoryFilter {
    /// Lines written at this Unix time or later.
    pub since: Option<i64>,
    /// Lines of these sources (`Source::as_str`); every source when empty.
    pub sources: Vec<String>,
}

impl Store {
    // ----------------------------------------------------------------
    // Display preference
    // ----------------------------------------------------------------

    /// Remembers how the mails of `addr` are displayed: `"text"`, `"html"`,
    /// `"images"` (the HTML with its remote images loaded, `ui.md`, "Images
    /// distantes"), or `None` for the client's default. A local display
    /// preference: not synced, not journaled, `date_modified` untouched
    /// (schema.sql).
    ///
    /// `"images"` is stored as `body_format = 'html'` with `remote_images =
    /// 1`: a column added to the table rather than a new value of
    /// `body_format`, whose CHECK constraint SQLite can only change by
    /// rebuilding the table.
    pub fn set_body_format(&mut self, addr: &str, format: Option<&str>) -> Result<()> {
        let (stored, images) = match format {
            None => (None, false),
            Some("text") => (Some("text"), false),
            Some("html") => (Some("html"), false),
            Some("images") => (Some("html"), true),
            Some(f) => return Err(StoreError::InvalidParams(format!("unknown format: {f} (text, html or images)"))),
        };
        let n = self.conn.execute(
            "UPDATE address SET body_format = ?2, remote_images = ?3 WHERE addr = ?1",
            params![addr.trim().to_lowercase(), stored, images],
        )?;
        if n == 0 {
            return Err(StoreError::NotFound(format!("address {addr}")));
        }
        Ok(())
    }

    // ----------------------------------------------------------------
    // Export
    // ----------------------------------------------------------------

    /// Writes one message into `dir` and returns the path of the file.
    ///
    /// - `eml`: the raw message, byte for byte, read back from the raw
    ///   storage and checked against its hash;
    /// - `txt` and `md`: the main headers and the text body.
    ///
    /// The file is named `<date> <subject>.<ext>`; an existing file is never
    /// overwritten, " (2)", " (3)"… is added instead.
    pub fn export_message(&self, raw: &RawStore, message_id: i64, format: ExportFormat, dir: &Path) -> Result<PathBuf> {
        let MessageParts { subject, from, to, cc, when, body, hash } = self.message_parts(message_id)?;
        let (bytes, ext): (Vec<u8>, &str) = match format {
            ExportFormat::Eml => {
                let hash = hash.ok_or_else(|| StoreError::NotFound(format!("raw file of message {message_id}")))?;
                let hash: crate::raw::Hash =
                    hash.try_into().map_err(|_| StoreError::Io("hash de brut invalide".into()))?;
                (raw.read(&hash).map_err(|e| crate::raw_error(&hash, e))?, "eml")
            }
            ExportFormat::Txt => {
                let mut s = text_headers(&from, &to, &cc, &when, &subject);
                s.push_str(&format!("\n{body}\n"));
                (s.into_bytes(), "txt")
            }
            ExportFormat::Md => {
                let mut s = format!("# {}\n\n- **From:** {from}\n- **To:** {}\n", md_inline(&subject), to);
                if !cc.is_empty() {
                    s.push_str(&format!("- **Cc:** {cc}\n"));
                }
                s.push_str(&format!("- **Date:** {when}\n\n{body}\n"));
                (s.into_bytes(), "md")
            }
        };
        let base = format!("{} {}", &when[..10], file_safe(&subject));
        write_new_file(dir, &base, ext, &bytes)
    }

    /// The text of one message for the clipboard (`ui.md`, menu `s`, "c
    /// to clipboard"): the headers of the text file, then the text body
    /// with each paragraph on one line (`paragraphs::unwrap`), so that it
    /// pastes into a document without the line breaks of the sender's
    /// mail app. No line break at the end.
    pub fn message_text(&self, message_id: i64) -> Result<String> {
        let MessageParts { subject, from, to, cc, when, body, .. } = self.message_parts(message_id)?;
        let mut s = text_headers(&from, &to, &cc, &when, &subject);
        let body = crate::paragraphs::unwrap(&body);
        if body.is_empty() {
            // No body: the headers alone, without the empty line after them.
            s.pop();
        } else {
            s.push('\n');
            s.push_str(&body);
        }
        Ok(s)
    }

    /// What the exports of a message are made of, read in one query plus
    /// one per recipient role.
    fn message_parts(&self, message_id: i64) -> Result<MessageParts> {
        type Row = (String, Option<String>, Option<String>, i64, Option<Vec<u8>>, Option<String>);
        let (subject, from_name, from_addr, date, hash, text): Row = self
            .conn
            .query_row(
                "SELECT m.subject, m.from_name, m.from_addr, m.date_received, m.raw_hash, b.text_plain
                   FROM message m LEFT JOIN message_body b ON b.message_id = m.id WHERE m.id = ?1",
                [message_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound(format!("message {message_id}")))?;
        let recipients = |role: &str| -> Result<String> {
            let mut stmt = self.conn.prepare_cached(
                "SELECT a.name, a.addr FROM message_address ma JOIN address a ON a.id = ma.address_id
                  WHERE ma.message_id = ?1 AND ma.role = ?2 ORDER BY ma.position",
            )?;
            let list: Vec<String> = stmt
                .query_map(params![message_id, role], |r| {
                    let (name, addr): (Option<String>, String) = (r.get(0)?, r.get(1)?);
                    Ok(match name {
                        Some(n) => format!("{n} <{addr}>"),
                        None => addr,
                    })
                })?
                .collect::<std::result::Result<_, _>>()?;
            Ok(list.join(", "))
        };
        let from = match (&from_name, &from_addr) {
            (Some(n), Some(a)) => format!("{n} <{a}>"),
            (None, Some(a)) => a.clone(),
            _ => String::new(),
        };
        Ok(MessageParts {
            subject,
            from,
            to: recipients("to")?,
            cc: recipients("cc")?,
            when: format_datetime(date),
            body: text.unwrap_or_default(),
            hash,
        })
    }

    // ----------------------------------------------------------------
    // Unsubscribing (`ui.md`, "Désabonnement")
    // ----------------------------------------------------------------

    /// How to unsubscribe from the list of `message_id`, as found at
    /// reception (`unsubscribe.rs`). Refused for a message in Junk,
    /// Quarantine or Banned: unsubscribing from a spam only tells the
    /// spammer that the address is read. `in_banned`
    /// (`unsubscribe.in_banned` of `config.toml`) opens Banned, never the
    /// other two.
    pub fn unsubscribe_target(&self, message_id: i64, in_banned: bool) -> Result<UnsubscribeTarget> {
        type Row = (Option<String>, Option<String>, bool, String, Option<String>);
        let (url, mailto, one_click, source, role): Row = self
            .conn
            .query_row(
                "SELECT u.url, u.mailto, u.one_click, u.source, f.role
                   FROM message m JOIN folder f ON f.id = m.folder_id
                   JOIN message_unsubscribe u ON u.message_id = m.id
                  WHERE m.id = ?1",
                [message_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound(format!("unsubscribe link of message {message_id}")))?;
        let closed = match role.as_deref() {
            Some("junk" | "quarantine") => true,
            Some("banned") => !in_banned,
            _ => false,
        };
        if closed {
            return Err(StoreError::InvalidParams(
                "no unsubscribe from Junk, Quarantine or Banned: it would confirm the address to the spammer \
                 (unsubscribe.in_banned in config.toml allows it in Banned)"
                    .into(),
            ));
        }
        Ok(UnsubscribeTarget { url, mailto, one_click, source })
    }

    /// Records that the user unsubscribed with one click from the sender of
    /// `message_id`: `address.unsubscribed_at`, and a line in the audit
    /// log. The sender's screening decision and its mails stay as they are.
    pub fn mark_unsubscribed(&mut self, message_id: i64, source: Source) -> Result<ChangeSet> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (address, thread, folder, url): (Option<i64>, Option<i64>, i64, Option<String>) = tx
            .query_row(
                "SELECT m.from_address_id, m.thread_id, m.folder_id, u.url
                   FROM message m LEFT JOIN message_unsubscribe u ON u.message_id = m.id WHERE m.id = ?1",
                [message_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound(format!("message {message_id}")))?;
        let (entity, entity_id) = match address {
            Some(a) => {
                tx.execute("UPDATE address SET unsubscribed_at = unixepoch() WHERE id = ?1", [a])?;
                ("address", a)
            }
            // No readable sender: the log still keeps a trace, on the message.
            None => ("message", message_id),
        };
        tx.execute(
            "INSERT INTO audit_log (created_by, entity, entity_id, action, detail)
             VALUES (?1, ?2, ?3, 'unsubscribe', json_object('message_id', ?4, 'method', 'one_click', 'url', ?5))",
            params![source.as_str(), entity, entity_id, message_id, url],
        )?;
        tx.commit()?;
        // The thread is shown again by clients, with "unsubscribed on".
        let mut changes = ChangeSet::default();
        changes.messages.insert((message_id, thread));
        if let Some(t) = thread {
            changes.threads.push((t, vec![folder]));
        }
        Ok(changes)
    }

    // ----------------------------------------------------------------
    // History
    // ----------------------------------------------------------------

    /// One page of the audit log, newest first (`tracabilite.md`).
    pub fn list_history(&self, cursor: Option<&str>, limit: u32) -> Result<(Vec<HistoryEntry>, Option<String>)> {
        self.list_history_filtered(cursor, limit, &HistoryFilter::default())
    }

    /// One page of the audit log, newest first, of the lines `filter` keeps:
    /// written since a date, by some sources (`tracabilite.md`, section 6;
    /// `mcp.md`, `agent_history`).
    ///
    /// The SQL is built with only the clauses needed, rather than one
    /// statement with `?1 IS NULL OR …` for each: SQLite would then read the
    /// cursor as a filter, not as the place to start in the date index, and
    /// every page would read the log again from its newest line. There are
    /// at most eight variants, each prepared once (`prepare_cached`).
    ///
    /// A filter on the sources reads the date index and skips the other
    /// sources' lines: no index on `created_by`, which would need a new
    /// schema version (and the upgrade of every mailbox) for a list read
    /// rarely; `since` bounds what is read.
    pub fn list_history_filtered(
        &self,
        cursor: Option<&str>,
        limit: u32,
        filter: &HistoryFilter,
    ) -> Result<(Vec<HistoryEntry>, Option<String>)> {
        let fetch = i64::from(limit) + 1;
        type Row = (i64, i64, String, String, i64, String, Option<String>);
        let map = |r: &rusqlite::Row| -> rusqlite::Result<Row> {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?))
        };
        // Rust note: `Box<dyn ToSql>` holds values of different types
        // (integers, a string) in one list of SQL parameters.
        let mut clauses: Vec<&str> = Vec::new();
        let mut values: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(c) = cursor {
            let (d, id) = parse_pair(c)?;
            clauses.push("(date_created, id) < (?, ?)");
            values.push(Box::new(d));
            values.push(Box::new(id));
        }
        if let Some(since) = filter.since {
            clauses.push("date_created >= ?");
            values.push(Box::new(since));
        }
        if !filter.sources.is_empty() {
            for s in &filter.sources {
                if Source::parse(s).is_none() {
                    return Err(StoreError::InvalidParams(format!("unknown source: {s}")));
                }
            }
            clauses.push("created_by IN (SELECT value FROM json_each(?))");
            values.push(Box::new(serde_json::to_string(&filter.sources).unwrap_or_default()));
        }
        let where_clause = if clauses.is_empty() { String::new() } else { format!("WHERE {}", clauses.join(" AND ")) };
        values.push(Box::new(fetch));
        let sql = format!(
            "SELECT id, date_created, created_by, entity, entity_id, action, detail
               FROM audit_log {where_clause} ORDER BY date_created DESC, id DESC LIMIT ?"
        );
        let rows: Vec<Row> = self
            .conn
            .prepare_cached(&sql)?
            .query_map(rusqlite::params_from_iter(values.iter()), map)?
            .collect::<std::result::Result<_, _>>()?;
        let has_more = rows.len() as i64 > i64::from(limit);
        let mut items = Vec::new();
        for (id, date, source, entity, entity_id, action, detail) in rows.into_iter().take(limit as usize) {
            let detail: Value = detail.and_then(|d| serde_json::from_str(&d).ok()).unwrap_or(Value::Null);
            let summary = self.history_summary(&entity, entity_id, &action, &detail)?;
            items.push(HistoryEntry { id, date, source, entity, entity_id, action, summary, detail });
        }
        let next = if has_more { items.last().map(|e| format!("{}:{}", e.date, e.id)) } else { None };
        Ok((items, next))
    }

    /// The audit log as the History screen shows it, newest first: the
    /// consecutive lines of one action make one group (`tracabilite.md`,
    /// section 6). With `query.range`, the lines of one group, each on its
    /// own.
    ///
    /// The groups are made here, while reading the log in date order, and
    /// not by the client: one action on 300 mails is 300 lines, and a client
    /// would have to fetch them all to show one row. The scan stops as soon
    /// as the page is full. Hiding the mails marked as read may read many
    /// lines to fill a page, so consecutive lines with the same `detail`,
    /// the usual case in such a run, are decoded only once.
    pub fn list_history_groups(
        &self,
        cursor: Option<&str>,
        limit: u32,
        query: &HistoryGroupQuery,
    ) -> Result<(Vec<HistoryGroup>, Option<String>)> {
        let mut clauses: Vec<&str> = Vec::new();
        let mut values: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(c) = cursor {
            let (d, id) = parse_pair(c)?;
            clauses.push("(date_created, id) < (?, ?)");
            values.push(Box::new(d));
            values.push(Box::new(id));
        }
        if let Some((first, last)) = query.range {
            clauses.push("id BETWEEN ? AND ?");
            values.push(Box::new(first));
            values.push(Box::new(last));
        }
        // Filtered in SQL, before the groups are made: the lines of the
        // other sources never reach the scan. Grouping is not changed by it:
        // a group needs consecutive ids, and a line of another source
        // between two runs of one keeps them apart, filtered or not. As in
        // `list_history_filtered`, the date index is read and the other
        // sources' lines skipped (no index on `created_by`): a rare source
        // may read much of the log to fill a page, once, as the list loads
        // as it scrolls.
        if !query.sources.is_empty() {
            for s in &query.sources {
                if Source::parse(s).is_none() {
                    return Err(StoreError::InvalidParams(format!("unknown source: {s}")));
                }
            }
            clauses.push("created_by IN (SELECT value FROM json_each(?))");
            values.push(Box::new(serde_json::to_string(&query.sources).unwrap_or_default()));
        }
        let where_clause = if clauses.is_empty() { String::new() } else { format!("WHERE {}", clauses.join(" AND ")) };
        let sql = format!(
            "SELECT id, date_created, created_by, entity, entity_id, action, detail
               FROM audit_log {where_clause} ORDER BY date_created DESC, id DESC"
        );
        let mut folders: HashMap<i64, String> = HashMap::new();
        let mut folder = |id: i64| -> Result<String> {
            if let Some(name) = folders.get(&id) {
                return Ok(name.clone());
            }
            let name = self.folder_name(id)?;
            folders.insert(id, name.clone());
            Ok(name)
        };
        let keeps = |g: &GroupBuilder| query.range.is_some() || query.include_read || g.group.kind != "read";

        let mut stmt = self.conn.prepare_cached(&sql)?;
        let mut rows = stmt.query(rusqlite::params_from_iter(values.iter()))?;
        let mut done: Vec<GroupBuilder> = Vec::new();
        let mut current: Option<GroupBuilder> = None;
        // The last line decoded, to reuse its result on the next identical one.
        let mut decoded: Option<(String, Option<String>, Value, String, Option<String>)> = None;
        let mut has_more = false;
        while let Some(r) = rows.next()? {
            let (id, date): (i64, i64) = (r.get(0)?, r.get(1)?);
            let (source, entity): (String, String) = (r.get(2)?, r.get(3)?);
            let entity_id: i64 = r.get(4)?;
            let action: String = r.get(5)?;
            let raw: Option<String> = r.get(6)?;
            let same = decoded.as_ref().is_some_and(|(a, d, ..)| *a == action && *d == raw);
            if !same {
                let detail: Value = raw.as_deref().and_then(|d| serde_json::from_str(d).ok()).unwrap_or(Value::Null);
                let (kind, value) = history_change(&action, &detail, &mut folder)?;
                decoded = Some((action.clone(), raw, detail, kind, value));
            }
            let Some((_, _, detail, kind, value)) = decoded.as_ref() else { continue };
            if query.range.is_none()
                && let Some(g) = current.as_mut().filter(|g| g.continues(id, date, &source, &entity, kind, value))
            {
                g.group.first_id = id;
                g.group.count += 1;
                g.oldest = date;
                continue;
            }
            if let Some(g) = current.take().filter(&keeps) {
                done.push(g);
                if done.len() == limit as usize {
                    has_more = true;
                    break;
                }
            }
            current = Some(GroupBuilder {
                group: HistoryGroup {
                    first_id: id,
                    last_id: id,
                    count: 1,
                    date,
                    source,
                    entity,
                    entity_id,
                    kind: kind.clone(),
                    value: value.clone(),
                    title: None,
                },
                detail: detail.clone(),
                oldest: date,
            });
        }
        drop(rows);
        if !has_more && let Some(g) = current.take().filter(&keeps) {
            done.push(g);
        }
        // Read last: only the rows of the page need their titles.
        let next = if has_more { done.last().map(|g| format!("{}:{}", g.oldest, g.group.first_id)) } else { None };
        let mut items = Vec::with_capacity(done.len());
        for mut g in done {
            g.group.title = self.history_title(&g.group.entity, g.group.entity_id, &g.detail)?;
            items.push(g.group);
        }
        Ok((items, next))
    }

    /// The role of a folder, else its path; `?` for a folder gone.
    fn folder_name(&self, id: i64) -> Result<String> {
        Ok(self
            .conn
            .prepare_cached("SELECT coalesce(role, path) FROM folder WHERE id = ?1")?
            .query_row([id], |r| r.get::<_, String>(0))
            .optional()?
            .unwrap_or_else(|| "?".into()))
    }

    /// What an entry of the log is about: the subject of the mail, the
    /// address, the name. An item deleted since keeps the name its line
    /// recorded, if any (`tracabilite.md`: the log survives its entities).
    fn history_title(&self, entity: &str, id: i64, detail: &Value) -> Result<Option<String>> {
        let one = |sql: &str| -> Result<Option<String>> {
            Ok(self.conn.prepare_cached(sql)?.query_row([id], |r| r.get::<_, Option<String>>(0)).optional()?.flatten())
        };
        let found = match entity {
            // A mail without a subject exists: its title is empty, not gone.
            "message" => {
                let row: Option<Option<String>> = self
                    .conn
                    .prepare_cached("SELECT subject FROM message WHERE id = ?1")?
                    .query_row([id], |r| r.get(0))
                    .optional()?;
                row.map(Option::unwrap_or_default)
            }
            "address" => one("SELECT addr FROM address WHERE id = ?1")?,
            "domain" => one("SELECT domain FROM domain WHERE id = ?1")?,
            "account" => one("SELECT config_key FROM account WHERE id = ?1")?,
            "contact_group" => one("SELECT name FROM contact_group WHERE id = ?1")?,
            "folder" => one("SELECT coalesce(role, path) FROM folder WHERE id = ?1")?,
            "tag" => one("SELECT name FROM tag WHERE id = ?1")?,
            _ => None,
        };
        let recorded = || {
            [&detail["before"]["name"], &detail["after"]["name"]].into_iter().find_map(|v| v.as_str().map(str::to_string))
        };
        Ok(found.or_else(recorded))
    }

    /// One readable line for an audit entry: what the entity is (subject,
    /// address, name) and, for the common actions, what changed.
    fn history_summary(&self, entity: &str, id: i64, action: &str, detail: &Value) -> Result<String> {
        let what = self.history_title(entity, id, detail)?.unwrap_or_else(|| format!("{entity} {id}"));
        let folder_role = |v: &Value| self.folder_name(v.as_i64().unwrap_or(-1));
        let change = match action {
            "move" => format!(" → {}", folder_role(&detail["after"]["folder_id"])?),
            "screening" | "auth_policy" => format!(" → {}", detail["after"].as_str().unwrap_or("—")),
            "set_flags" => detail["after"]
                .as_object()
                .and_then(|o| o.iter().next())
                .map(|(k, v)| format!(" {k} = {v}"))
                .unwrap_or_default(),
            "set_tags" | "add_member" | "remove_member" => {
                detail["tag"].as_str().or(detail["addr"].as_str()).map(|t| format!(" {t}")).unwrap_or_default()
            }
            "import_mbox" => format!(" {} imported", detail["imported"]),
            _ => String::new(),
        };
        Ok(format!("{what}{change}"))
    }

    // ----------------------------------------------------------------
    // Tags
    // ----------------------------------------------------------------

    /// Tags not deleted, with the number of messages carrying each.
    pub fn list_tags(&self, account_id: Option<i64>) -> Result<Vec<TagView>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT t.id, t.account_id, t.name, t.color,
                    (SELECT count(*) FROM message_keyword mk JOIN message m ON m.id = mk.message_id
                      WHERE mk.keyword = t.keyword AND m.account_id = t.account_id)
               FROM tag t
              WHERE t.date_deleted IS NULL AND (?1 IS NULL OR t.account_id = ?1)
              ORDER BY t.name COLLATE NOCASE",
        )?;
        let items = stmt
            .query_map([account_id], |r| {
                Ok(TagView { id: r.get(0)?, account_id: r.get(1)?, name: r.get(2)?, color: r.get(3)?, message_count: r.get(4)? })
            })?
            .collect::<std::result::Result<_, _>>()?;
        Ok(items)
    }

    /// Adds (or removes) the tag `name` on every message of a thread. The
    /// tag is created in the thread's account if it does not exist (a
    /// deleted tag of the same name is revived).
    ///
    /// A tag is an IMAP keyword (`modele de donnees.md`, `tag`): the
    /// keyword is derived from the name by [`tag_keyword`].
    pub fn tag_thread(&mut self, thread_id: i64, name: &str, remove: bool, source: Source) -> Result<ChangeSet> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 64 {
            return Err(StoreError::Invalid(nm_api::reason::TAG_NAME_INVALID.into()));
        }
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changes = tag_one(&tx, thread_id, name, remove, source)?;
        tx.commit()?;
        Ok(changes)
    }

    /// `tag_thread` for several threads, in one transaction: `t` on 50
    /// marked threads is one request and one batch of events (`ui.md`,
    /// "Actions groupées"). Threads gone meanwhile are skipped; a thread
    /// of a read-only account refuses the whole command.
    pub fn tag_threads(&mut self, thread_ids: &[i64], name: &str, remove: bool, source: Source) -> Result<ChangeSet> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 64 {
            return Err(StoreError::Invalid(nm_api::reason::TAG_NAME_INVALID.into()));
        }
        let ids = self.existing_threads(thread_ids)?;
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut all = ChangeSet::default();
        for id in ids {
            all.merge(tag_one(&tx, id, name, remove, source)?);
        }
        tx.commit()?;
        Ok(all)
    }

    /// Renames the tag `name` to `new_name`, in every account that has it:
    /// the user thinks of a tag by its name (`ui.md`, "Écrans Tags et
    /// Groupes"), and `;T` lists the tags of every account as one row per
    /// name. Returns how many tags (one per account) were renamed; 0 when
    /// the name does not change.
    ///
    /// Only the displayed name changes, never the IMAP keyword
    /// (`modele de donnees.md`, `tag`): the keyword is what the server
    /// stores on each message, and changing it would rewrite every tagged
    /// message on the server (a STORE per message, thousands for a big
    /// tag), for a name the user reads in NiceMailer only. The price:
    /// another mail client (Thunderbird) keeps showing the old keyword.
    ///
    /// Refused when another tag of one of these accounts already has the
    /// new name: merging two tags would mean moving one keyword to the
    /// other on the server. A change of case only ("vip" to "VIP") is a
    /// rename of the same tag.
    pub fn rename_tag(&mut self, name: &str, new_name: &str, source: Source) -> Result<usize> {
        let new_name = new_name.trim();
        if new_name.is_empty() || new_name.chars().count() > 64 {
            return Err(StoreError::Invalid(nm_api::reason::TAG_NAME_INVALID.into()));
        }
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let rows = all_tags(&tx)?;
        let (old, new) = (name.trim().to_lowercase(), new_name.to_lowercase());
        let renamed: Vec<&TagRow> = rows.iter().filter(|t| t.active && t.name.to_lowercase() == old).collect();
        if renamed.is_empty() {
            return Err(StoreError::NotFound(format!("tag {name}")));
        }
        for t in &renamed {
            if rows.iter().any(|o| o.active && o.account_id == t.account_id && o.id != t.id && o.name.to_lowercase() == new) {
                return Err(StoreError::Invalid(nm_api::reason::TAG_EXISTS.with("tag", new_name)));
            }
        }
        let mut count = 0;
        for t in renamed {
            if t.name == new_name {
                continue;
            }
            // A deleted tag of the account may hold the new name: the name
            // is unique per account (`UNIQUE (account_id, name)`, deleted
            // rows included), and reviving it by that name later must find
            // the renamed tag, not the old one. Its tombstone keeps its
            // keyword under a name no one types.
            tx.execute(
                "UPDATE tag SET name = name || ' #' || id WHERE account_id = ?1 AND date_deleted IS NOT NULL AND name = ?2",
                params![t.account_id, new_name],
            )?;
            tx.execute(
                "UPDATE tag SET name = ?2, date_modified = unixepoch(), modified_by = ?3 WHERE id = ?1",
                params![t.id, new_name, source.as_str()],
            )?;
            audit(
                &tx,
                source,
                "tag",
                t.id,
                "update",
                serde_json::json!({"before": {"name": t.name}, "after": {"name": new_name}}),
            )?;
            count += 1;
        }
        tx.commit()?;
        Ok(count)
    }

    /// Deletes the tag `name` in every account that has it: the tag leaves
    /// every message (on the server too, through the spool, as `t` does
    /// when it removes a tag), then its row becomes a tombstone
    /// (`date_deleted`, `modele de donnees.md`). Removing the keyword first
    /// is what makes the deletion complete: a tombstone alone would leave
    /// the keyword on the messages, and creating a tag of the same name
    /// later would bring them all back under it.
    ///
    /// A read-only account (`read_only = true`) is left as it is, its tag
    /// included: nothing is changed in it.
    ///
    /// One transaction for every thread of the tag, found through
    /// `idx_message_keyword_kw`: a tag of 2,000 mails is one request.
    pub fn delete_tag(&mut self, name: &str, source: Source) -> Result<ChangeSet> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let wanted = name.trim().to_lowercase();
        let found: Vec<TagRow> = all_tags(&tx)?.into_iter().filter(|t| t.active && t.name.to_lowercase() == wanted).collect();
        if found.is_empty() {
            return Err(StoreError::NotFound(format!("tag {name}")));
        }
        let mut all = ChangeSet::default();
        for t in found {
            let read_only: bool = tx
                .prepare_cached("SELECT EXISTS (SELECT 1 FROM temp.read_only_account WHERE id = ?1)")?
                .query_row([t.account_id], |r| r.get(0))?;
            if read_only {
                continue;
            }
            let threads: Vec<i64> = tx
                .prepare_cached(
                    "SELECT DISTINCT m.thread_id FROM message_keyword mk JOIN message m ON m.id = mk.message_id
                      WHERE mk.keyword = ?1 AND m.account_id = ?2 AND m.thread_id IS NOT NULL",
                )?
                .query_map(params![t.keyword, t.account_id], |r| r.get(0))?
                .collect::<std::result::Result<_, _>>()?;
            for thread_id in threads {
                // The removal of `t`, with its spool entries and history.
                all.merge(tag_one(&tx, thread_id, &t.name, true, source)?);
            }
            tx.execute(
                "UPDATE tag SET date_deleted = unixepoch(), date_modified = unixepoch(), modified_by = ?2 WHERE id = ?1",
                params![t.id, source.as_str()],
            )?;
            audit(&tx, source, "tag", t.id, "delete", serde_json::json!({"before": {"name": t.name}}))?;
        }
        tx.commit()?;
        Ok(all)
    }
}

/// One row of `tag`, as `rename_tag` and `delete_tag` compare them.
struct TagRow {
    id: i64,
    account_id: i64,
    name: String,
    keyword: String,
    active: bool,
}

/// Every tag, deleted ones included. Names are compared in Rust, Unicode
/// aware (see `tag_one`); the table holds a few rows per account.
fn all_tags(tx: &rusqlite::Transaction) -> Result<Vec<TagRow>> {
    let rows = tx
        .prepare_cached("SELECT id, account_id, name, keyword, date_deleted IS NULL FROM tag ORDER BY id")?
        .query_map([], |r| {
            Ok(TagRow { id: r.get(0)?, account_id: r.get(1)?, name: r.get(2)?, keyword: r.get(3)?, active: r.get(4)? })
        })?
        .collect::<std::result::Result<_, _>>()?;
    Ok(rows)
}

/// The body of `tag_thread`, in the caller's transaction.
fn tag_one(tx: &rusqlite::Transaction, thread_id: i64, name: &str, remove: bool, source: Source) -> Result<ChangeSet> {
    {
        let account_id: i64 = tx
            .query_row("SELECT account_id FROM thread WHERE id = ?1", [thread_id], |r| r.get(0))
            .optional()?
            .ok_or_else(|| StoreError::NotFound(format!("conversation {thread_id}")))?;
        crate::spool::refuse_read_only(tx, thread_id)?;
        // Names are compared without case in Rust: SQLite's NOCASE only folds
        // ASCII, so "Équipe" and "équipe" would be two tags. An account has
        // few tags, reading them all is cheap.
        let existing: Option<(i64, String, Option<i64>)> = {
            let mut stmt = tx.prepare_cached("SELECT id, keyword, date_deleted, name FROM tag WHERE account_id = ?1")?;
            let rows: Vec<(i64, String, Option<i64>, String)> = stmt
                .query_map([account_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                .collect::<std::result::Result<_, _>>()?;
            let wanted = name.to_lowercase();
            // An active tag first, else a deleted one to revive.
            let mut matching: Vec<_> = rows.into_iter().filter(|r| r.3.to_lowercase() == wanted).collect();
            matching.sort_by_key(|r| r.2.is_some());
            matching.into_iter().next().map(|(id, kw, del, _)| (id, kw, del))
        };
        let keyword = match existing {
            Some((_, kw, None)) => kw,
            Some(_) if remove => return Ok(ChangeSet::default()),
            Some((id, kw, Some(_))) => {
                tx.execute(
                    "UPDATE tag SET date_deleted = NULL, date_modified = unixepoch(), modified_by = ?2 WHERE id = ?1",
                    params![id, source.as_str()],
                )?;
                audit(tx, source, "tag", id, "create", serde_json::json!({"after": {"name": name}}))?;
                kw
            }
            None if remove => return Ok(ChangeSet::default()),
            None => {
                let kw = unique_keyword(tx, account_id, &tag_keyword(name))?;
                tx.execute(
                    "INSERT INTO tag (account_id, name, keyword, created_by, modified_by) VALUES (?1, ?2, ?3, ?4, ?4)",
                    params![account_id, name, kw, source.as_str()],
                )?;
                let id = tx.last_insert_rowid();
                audit(tx, source, "tag", id, "create", serde_json::json!({"after": {"name": name, "keyword": kw}}))?;
                kw
            }
        };

        let messages: Vec<(i64, i64)> = tx
            .prepare("SELECT id, folder_id FROM message WHERE thread_id = ?1")?
            .query_map([thread_id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?;
        let mut changes = ChangeSet::default();
        for (message_id, folder_id) in messages {
            let n = if remove {
                tx.execute(
                    "DELETE FROM message_keyword WHERE message_id = ?1 AND keyword = ?2 COLLATE NOCASE",
                    params![message_id, keyword],
                )?
            } else {
                tx.execute(
                    "INSERT OR IGNORE INTO message_keyword (message_id, keyword) VALUES (?1, ?2)",
                    params![message_id, keyword],
                )?
            };
            if n == 0 {
                continue;
            }
            // Tags are a user-visible attribute of the message (tracabilite.md).
            tx.execute(
                "UPDATE message SET date_modified = unixepoch(), modified_by = ?2 WHERE id = ?1",
                params![message_id, source.as_str()],
            )?;
            let op = if remove { "remove" } else { "add" };
            audit(tx, source, "message", message_id, "set_tags", serde_json::json!({op: name, "tag": name}))?;
            // A tag is an IMAP keyword: the server gets it through the spool.
            crate::spool::enqueue_flag(tx, message_id, &keyword, !remove, source.as_str(), &mut changes)?;
            changes.messages.insert((message_id, Some(thread_id)));
            changes.folders.insert(folder_id);
        }
        if !changes.messages.is_empty() {
            let folders = refresh_thread(tx, thread_id)?;
            changes.threads.push((thread_id, folders));
        }
        Ok(changes)
    }
}

impl Store {
    // ----------------------------------------------------------------
    // Contact groups (groupes.md)
    // ----------------------------------------------------------------

    /// The active group named `name` (compared without case, Unicode
    /// aware, as in `set_group_member`), with its name as stored.
    fn active_group(tx: &rusqlite::Transaction, name: &str) -> Result<(i64, String)> {
        let wanted = name.trim().to_lowercase();
        let rows: Vec<(i64, String)> = tx
            .prepare_cached("SELECT id, name FROM contact_group WHERE date_deleted IS NULL ORDER BY id")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?;
        rows.into_iter()
            .find(|(_, n)| n.to_lowercase() == wanted)
            .ok_or_else(|| StoreError::NotFound(format!("group {name}")))
    }

    /// Renames the group `name` to `new_name` (`groupes.md`, section 6).
    /// Its members, position and colour stay; the filters that name it
    /// follow, since they hold its id. Returns whether the name changed.
    ///
    /// Refused when another active group has the new name, without case
    /// (`idx_contact_group_name`): merging two groups is not a rename. A
    /// deleted group of that name stays a tombstone: the unique index only
    /// covers active groups, and `set_group_member` prefers an active one.
    pub fn rename_group(&mut self, name: &str, new_name: &str, source: Source) -> Result<bool> {
        let new_name = new_name.trim();
        if new_name.is_empty() || new_name.chars().count() > 64 {
            return Err(StoreError::Invalid(nm_api::reason::GROUP_NAME_INVALID.into()));
        }
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (id, old) = Self::active_group(&tx, name)?;
        if old == new_name {
            return Ok(false);
        }
        if let Ok((other, _)) = Self::active_group(&tx, new_name) {
            if other != id {
                return Err(StoreError::Invalid(nm_api::reason::GROUP_EXISTS.with("group", new_name)));
            }
        }
        tx.execute(
            "UPDATE contact_group SET name = ?2, date_modified = unixepoch(), modified_by = ?3 WHERE id = ?1",
            params![id, new_name, source.as_str()],
        )?;
        audit(&tx, source, "contact_group", id, "update", serde_json::json!({"before": {"name": old}, "after": {"name": new_name}}))?;
        tx.commit()?;
        Ok(true)
    }

    /// Deletes the group `name`: a tombstone (`date_deleted`), its member
    /// rows kept and no longer read (`groupes.md`, section 3, "Règles"). No
    /// mail changes: a group is a list of addresses kept by NiceMailer,
    /// nothing on the server. Returns the group's id and how many members
    /// it had.
    pub fn delete_group(&mut self, name: &str, source: Source) -> Result<(i64, i64)> {
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (id, old) = Self::active_group(&tx, name)?;
        let members: i64 = tx.query_row(
            "SELECT count(*) FROM contact_group_member WHERE group_id = ?1 AND date_removed IS NULL",
            [id],
            |r| r.get(0),
        )?;
        tx.execute(
            "UPDATE contact_group SET date_deleted = unixepoch(), date_modified = unixepoch(), modified_by = ?2 WHERE id = ?1",
            params![id, source.as_str()],
        )?;
        audit(&tx, source, "contact_group", id, "delete", serde_json::json!({"before": {"name": old, "members": members}}))?;
        tx.commit()?;
        Ok((id, members))
    }

    /// Groups not deleted, in display order, with their member count, their
    /// latest change and the mails received from their members.
    ///
    /// One pass over the memberships, read through the primary key
    /// `(group_id, address_id)`, and one row of `address` per active member
    /// for its `sender_count`: the cost follows the number of members,
    /// never the number of mails (on 275,000 mails, the count of a group's
    /// mails takes 150 ms; this, a few hundred microseconds). The removed
    /// members count for the date (a removal is a change of the group), not
    /// for the other two.
    pub fn list_groups(&self) -> Result<Vec<GroupView>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT g.id, g.name, g.color, count(a.id), coalesce(sum(a.sender_count), 0),
                    max(g.date_modified, coalesce(max(gm.date_modified), 0))
               FROM contact_group g
               LEFT JOIN contact_group_member gm ON gm.group_id = g.id
               LEFT JOIN address a ON a.id = gm.address_id AND gm.date_removed IS NULL
              WHERE g.date_deleted IS NULL
              GROUP BY g.id
              ORDER BY g.position, g.name COLLATE NOCASE",
        )?;
        let items = stmt
            .query_map([], |r| {
                Ok(GroupView {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    color: r.get(2)?,
                    member_count: r.get(3)?,
                    mail_count: r.get(4)?,
                    date_modified: r.get(5)?,
                })
            })?
            .collect::<std::result::Result<_, _>>()?;
        Ok(items)
    }

    /// What the header of the open thread shows besides its subject: the
    /// thread's tags, the person it is with and the groups that person is
    /// in (`ui.md`, "Tags et groupes dans l'en-tête").
    ///
    /// The person is the one `set_group_member` takes for this thread
    /// (`thread_correspondent_addr`): the header then names the groups that
    /// `g` can remove them from, never the groups of someone else of the
    /// thread. `own` as for `set_group_member`.
    ///
    /// A few indexed reads (the thread's messages, then the person's
    /// memberships through `idx_contact_group_member_address`), done once
    /// per thread opened.
    pub fn thread_labels(&self, thread_id: i64, own: &[String]) -> Result<ThreadLabels> {
        let own = self.own_addresses(own)?;
        let tags = crate::queries::thread_tag_names(&self.conn, thread_id)?;
        let correspondent = crate::queries::thread_correspondent_addr(&self.conn, thread_id, &own)?;
        let groups = match &correspondent {
            None => vec![],
            Some(addr) => self
                .conn
                .prepare_cached(
                    "SELECT g.name FROM address a
                       JOIN contact_group_member gm ON gm.address_id = a.id AND gm.date_removed IS NULL
                       JOIN contact_group g ON g.id = gm.group_id AND g.date_deleted IS NULL
                      WHERE a.addr = ?1
                      ORDER BY g.position, g.name COLLATE NOCASE",
                )?
                .query_map([addr], |r| r.get(0))?
                .collect::<std::result::Result<_, _>>()?,
        };
        Ok(ThreadLabels { thread_id, tags, correspondent, groups })
    }

    /// Removes the user's addresses (`own`, as for `set_group_member`) from
    /// every group: a tombstone, as any removal, recorded in the history.
    /// Called with each reading of `accounts.toml`, so that an address
    /// declared after it was put in a group, or put there before the rule
    /// (`groupes.md`, "Adresses de l'utilisateur"), leaves it. Returns the
    /// number of memberships removed; usually none, at the cost of one
    /// indexed lookup per address.
    pub fn remove_own_from_groups(&mut self, own: &[String], source: Source) -> Result<usize> {
        let own: Vec<String> = self.own_addresses(own)?.into_iter().collect();
        if own.is_empty() {
            return Ok(0);
        }
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let marks = vec!["?"; own.len()].join(", ");
        let found: Vec<(i64, i64, String)> = tx
            .prepare(&format!(
                "SELECT gm.group_id, gm.address_id, a.addr FROM address a
                   JOIN contact_group_member gm ON gm.address_id = a.id AND gm.date_removed IS NULL
                  WHERE a.addr IN ({marks})"
            ))?
            .query_map(rusqlite::params_from_iter(&own), |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<std::result::Result<_, _>>()?;
        for (group_id, address_id, addr) in &found {
            tx.execute(
                "UPDATE contact_group_member SET date_removed = unixepoch(), date_modified = unixepoch(), modified_by = ?3
                  WHERE group_id = ?1 AND address_id = ?2",
                params![group_id, address_id, source.as_str()],
            )?;
            audit(&tx, source, "contact_group", *group_id, "remove_member", serde_json::json!({"addr": addr}))?;
        }
        tx.commit()?;
        Ok(found.len())
    }

    /// Adds `addr`, or the person `thread_id` is with, to the group `group`
    /// (created if needed), or removes it. Returns (group id, address,
    /// whether anything changed).
    ///
    /// Rules of `groupes.md`, section 3: a deleted group recreated under the
    /// same name reuses its row and starts empty; removing a member is a
    /// tombstone (`date_removed`), adding it back clears it.
    ///
    /// `own`: the addresses `accounts.toml` declares, which only the caller
    /// (the daemon) knows; those of the archive accounts are added. None of
    /// them can be added (`groupes.md`, "Adresses de l'utilisateur"): a
    /// group is people the user writes with, and the user in a group makes
    /// its filter match every mail. Removing one stays possible.
    pub fn set_group_member(
        &mut self,
        group: &str,
        addr: Option<&str>,
        thread_id: Option<i64>,
        remove: bool,
        own: &[String],
        source: Source,
    ) -> Result<(i64, String, bool)> {
        let group = group.trim();
        if group.is_empty() || group.chars().count() > 64 {
            return Err(StoreError::Invalid(nm_api::reason::GROUP_NAME_INVALID.into()));
        }
        // Read before the transaction: it borrows the connection mutably.
        let own = self.own_addresses(own)?;
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        // The address: given, or the person the thread is with.
        let addr = match (addr, thread_id) {
            (Some(a), _) => {
                let a = a.trim().to_lowercase();
                if !a.contains('@') || a.contains(char::is_whitespace) {
                    return Err(StoreError::Invalid(nm_api::reason::ADDRESS_INVALID.with("address", a)));
                }
                a
            }
            (None, Some(t)) => crate::queries::thread_correspondent_addr(&tx, t, &own)?
                .ok_or_else(|| StoreError::NotFound(format!("correspondent of conversation {t}")))?,
            (None, None) => return Err(StoreError::InvalidParams("addr or thread_id required".into())),
        };
        if !remove && own.contains(&addr) {
            return Err(StoreError::InvalidParams(format!(
                "{addr} est une adresse de l'utilisateur : elle ne peut pas être membre d'un groupe"
            )));
        }
        let address_id: i64 = match tx
            .query_row("SELECT id FROM address WHERE addr = ?1", [&addr], |r| r.get(0))
            .optional()?
        {
            Some(id) => id,
            None if remove => return Ok((0, addr, false)),
            None => {
                tx.execute(
                    "INSERT INTO address (addr, created_by, modified_by) VALUES (?1, ?2, ?2)",
                    params![addr, source.as_str()],
                )?;
                tx.last_insert_rowid()
            }
        };

        // The group: active, deleted (revived empty), or new.
        // Same Unicode-aware comparison as for tags (see `tag_thread`).
        let found: Option<(i64, Option<i64>)> = {
            let mut stmt = tx.prepare_cached("SELECT id, date_deleted, name FROM contact_group ORDER BY id")?;
            let rows: Vec<(i64, Option<i64>, String)> =
                stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<std::result::Result<_, _>>()?;
            let wanted = group.to_lowercase();
            let mut matching: Vec<_> = rows.into_iter().filter(|r| r.2.to_lowercase() == wanted).collect();
            matching.sort_by_key(|r| r.1.is_some());
            matching.into_iter().next().map(|(id, del, _)| (id, del))
        };
        let group_id = match found {
            Some((id, None)) => id,
            Some(_) | None if remove => return Ok((0, addr, false)),
            Some((id, Some(_))) => {
                tx.execute(
                    "UPDATE contact_group SET date_deleted = NULL, name = ?2, date_modified = unixepoch(), modified_by = ?3
                      WHERE id = ?1",
                    params![id, group, source.as_str()],
                )?;
                tx.execute(
                    "UPDATE contact_group_member SET date_removed = unixepoch(), date_modified = unixepoch(), modified_by = ?2
                      WHERE group_id = ?1 AND date_removed IS NULL",
                    params![id, source.as_str()],
                )?;
                audit(&tx, source, "contact_group", id, "create", serde_json::json!({"after": {"name": group}}))?;
                id
            }
            None => {
                let key = format!("g_{:016x}", rand::random::<u64>());
                tx.execute(
                    "INSERT INTO contact_group (sync_key, name, position, created_by, modified_by)
                     VALUES (?1, ?2, (SELECT coalesce(max(position), -1) + 1 FROM contact_group), ?3, ?3)",
                    params![key, group, source.as_str()],
                )?;
                let id = tx.last_insert_rowid();
                audit(&tx, source, "contact_group", id, "create", serde_json::json!({"after": {"name": group}}))?;
                id
            }
        };

        let current: Option<Option<i64>> = tx
            .query_row(
                "SELECT date_removed FROM contact_group_member WHERE group_id = ?1 AND address_id = ?2",
                params![group_id, address_id],
                |r| r.get(0),
            )
            .optional()?;
        let changed = match (current, remove) {
            // Already a member / already absent: nothing to do.
            (Some(None), false) | (Some(Some(_)), true) | (None, true) => false,
            (Some(Some(_)), false) => {
                tx.execute(
                    "UPDATE contact_group_member SET date_removed = NULL, date_modified = unixepoch(), modified_by = ?3
                      WHERE group_id = ?1 AND address_id = ?2",
                    params![group_id, address_id, source.as_str()],
                )?;
                true
            }
            (None, false) => {
                tx.execute(
                    "INSERT INTO contact_group_member (group_id, address_id, created_by, modified_by) VALUES (?1, ?2, ?3, ?3)",
                    params![group_id, address_id, source.as_str()],
                )?;
                true
            }
            (Some(None), true) => {
                tx.execute(
                    "UPDATE contact_group_member SET date_removed = unixepoch(), date_modified = unixepoch(), modified_by = ?3
                      WHERE group_id = ?1 AND address_id = ?2",
                    params![group_id, address_id, source.as_str()],
                )?;
                true
            }
        };
        if changed {
            let action = if remove { "remove_member" } else { "add_member" };
            audit(&tx, source, "contact_group", group_id, action, serde_json::json!({"addr": addr}))?;
        }
        tx.commit()?;
        Ok((group_id, addr, changed))
    }

    // ----------------------------------------------------------------
    // Contacts
    // ----------------------------------------------------------------

    /// Known addresses, sorted by address, with an optional filter on the
    /// address or the name. Cursor: the last address of the previous page.
    pub fn list_contacts(&self, query: Option<&str>, cursor: Option<&str>, limit: u32) -> Result<(Vec<ContactView>, Option<String>)> {
        // `%` and `_` are LIKE wildcards: escape them in the user's text.
        let pattern = query
            .map(str::trim)
            .filter(|q| !q.is_empty())
            .map(|q| format!("%{}%", q.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")));
        let fetch = i64::from(limit) + 1;
        // id, addr, name, screening_target, auth_policy, sender kind
        // (computed, forced).
        type Row = (i64, String, Option<String>, Option<String>, String, Option<String>, Option<String>);
        let rows: Vec<Row> = self
            .conn
            .prepare_cached(
                "SELECT id, addr, name, screening_target, auth_policy, sender_kind, sender_kind_user FROM address
                  WHERE (?1 IS NULL OR addr > ?1)
                    AND (?2 IS NULL OR addr LIKE ?2 ESCAPE '\\' OR name LIKE ?2 ESCAPE '\\')
                  ORDER BY addr LIMIT ?3",
            )?
            .query_map(params![cursor, pattern, fetch], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?))
            })?
            .collect::<std::result::Result<_, _>>()?;
        let has_more = rows.len() as i64 > i64::from(limit);
        let mut items = Vec::new();
        for (id, addr, name, screening_target, auth_policy, computed, forced) in rows.into_iter().take(limit as usize) {
            // The user's choice wins over the computation (`expediteurs.md`).
            let sender_kind_forced = forced.is_some();
            let sender_kind = forced.or(computed).as_deref().and_then(nm_core::SenderKind::parse);
            let groups: Vec<String> = self
                .conn
                .prepare_cached(
                    "SELECT g.name FROM contact_group_member gm JOIN contact_group g ON g.id = gm.group_id
                      WHERE gm.address_id = ?1 AND gm.date_removed IS NULL AND g.date_deleted IS NULL
                      ORDER BY g.position, g.name",
                )?
                .query_map([id], |r| r.get(0))?
                .collect::<std::result::Result<_, _>>()?;
            // Counted through idx_message_from: a quick index range per row.
            let message_count: i64 = self
                .conn
                .prepare_cached("SELECT count(*) FROM message WHERE from_address_id = ?1")?
                .query_row([id], |r| r.get(0))?;
            let domain_target = crate::screening::domain_target(&self.conn, &addr)?.map(|t| t.as_str().to_string());
            items.push(ContactView {
                id,
                addr,
                name,
                screening_target,
                domain_target,
                auth_policy,
                groups,
                message_count,
                sender_kind,
                sender_kind_forced,
            });
        }
        let next = if has_more { items.last().map(|c| c.addr.clone()) } else { None };
        Ok((items, next))
    }
}

/// One `audit_log` line.
fn audit(tx: &Transaction, source: Source, entity: &str, id: i64, action: &str, detail: Value) -> Result<()> {
    tx.execute(
        "INSERT INTO audit_log (created_by, entity, entity_id, action, detail) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![source.as_str(), entity, id, action, detail.to_string()],
    )?;
    Ok(())
}

/// IMAP keyword for a tag name. A keyword is an ASCII atom without spaces
/// or special characters (`modele de donnees.md`, `tag`): accents are
/// removed, other characters become `_`. It never starts with `$`, so it
/// cannot use the `$Nm` prefix reserved to the core.
///
/// "Équipe Paris" → "Equipe_Paris", "Clients & co" → "Clients_co".
pub fn tag_keyword(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        let mapped = match c {
            'à' | 'â' | 'ä' | 'á' | 'ã' | 'å' => "a",
            'À' | 'Â' | 'Ä' | 'Á' | 'Ã' | 'Å' => "A",
            'é' | 'è' | 'ê' | 'ë' => "e",
            'É' | 'È' | 'Ê' | 'Ë' => "E",
            'î' | 'ï' | 'í' | 'ì' => "i",
            'Î' | 'Ï' | 'Í' | 'Ì' => "I",
            'ô' | 'ö' | 'ó' | 'ò' | 'õ' => "o",
            'Ô' | 'Ö' | 'Ó' | 'Ò' | 'Õ' => "O",
            'ù' | 'û' | 'ü' | 'ú' => "u",
            'Ù' | 'Û' | 'Ü' | 'Ú' => "U",
            'ç' => "c",
            'Ç' => "C",
            'ñ' => "n",
            'Ñ' => "N",
            'œ' => "oe",
            'Œ' => "OE",
            'æ' => "ae",
            'Æ' => "AE",
            'ß' => "ss",
            c if c.is_ascii_alphanumeric() || c == '-' => {
                out.push(c);
                continue;
            }
            _ => "_",
        };
        // Collapse runs of `_`.
        if mapped == "_" && (out.is_empty() || out.ends_with('_')) {
            continue;
        }
        out.push_str(mapped);
    }
    let out = out.trim_end_matches('_').to_string();
    if out.is_empty() { "tag".to_string() } else { out }
}

/// `keyword`, or `keyword_2`, `keyword_3`… if another tag of the account
/// already uses it (two names can map to the same keyword: "Café", "Cafe").
fn unique_keyword(tx: &Transaction, account_id: i64, keyword: &str) -> Result<String> {
    let mut candidate = keyword.to_string();
    let mut n = 2;
    while tx
        .prepare_cached("SELECT 1 FROM tag WHERE account_id = ?1 AND keyword = ?2 COLLATE NOCASE")?
        .exists(params![account_id, candidate])?
    {
        candidate = format!("{keyword}_{n}");
        n += 1;
    }
    Ok(candidate)
}

/// `YYYY-MM-DD HH:MM UTC` for a Unix timestamp.
fn format_datetime(ts: i64) -> String {
    let days = ts.div_euclid(86_400);
    let secs = ts.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02} UTC", secs / 3600, secs % 3600 / 60)
}

/// Calendar date of a day count since 1970-01-01 (inverse of
/// `mbox::days_from_civil`; Howard Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// A subject turned into a file name: no `/`, `:` or control characters,
/// at most 80 characters, "message" if nothing is left.
fn file_safe(subject: &str) -> String {
    let cleaned: String = subject
        .chars()
        .map(|c| if c == '/' || c == ':' || c == '\\' || c.is_control() { ' ' } else { c })
        .collect();
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let cleaned: String = cleaned.trim_start_matches('.').chars().take(80).collect();
    if cleaned.trim().is_empty() { "message".to_string() } else { cleaned.trim().to_string() }
}

/// Escapes the characters that would turn a Markdown title into markup.
fn md_inline(s: &str) -> String {
    s.replace('\\', "\\\\").replace('*', "\\*").replace('_', "\\_").replace('#', "\\#")
}

/// Creates `<dir>/<base>.<ext>`, or `<base> (2).<ext>`… if it exists.
/// `create_new` makes the check and the creation one atomic step: two
/// exports at the same moment cannot overwrite each other.
fn write_new_file(dir: &Path, base: &str, ext: &str, bytes: &[u8]) -> Result<PathBuf> {
    if !dir.is_dir() {
        return Err(StoreError::Invalid(nm_api::reason::FOLDER_MISSING.with("path", dir.display().to_string())));
    }
    for n in 1..1000 {
        let name = if n == 1 { format!("{base}.{ext}") } else { format!("{base} ({n}).{ext}") };
        let path = dir.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut f) => {
                f.write_all(bytes).map_err(|e| StoreError::Io(e.to_string()))?;
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(StoreError::Io(format!("{} : {e}", path.display()))),
        }
    }
    Err(StoreError::Io(format!("trop de fichiers {base}.{ext} dans {}", dir.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_keywords_are_ascii_atoms() {
        assert_eq!(tag_keyword("Équipe Paris"), "Equipe_Paris");
        assert_eq!(tag_keyword("Clients & co"), "Clients_co");
        assert_eq!(tag_keyword("$NmWasTodo"), "NmWasTodo");
        assert_eq!(tag_keyword("!!!"), "tag");
        assert_eq!(tag_keyword("cœur"), "coeur");
    }

    #[test]
    fn formats_dates_and_file_names() {
        assert_eq!(format_datetime(1_592_940_056), "2020-06-23 19:20 UTC");
        assert_eq!(format_datetime(0), "1970-01-01 00:00 UTC");
        assert_eq!(format_datetime(1_709_208_000), "2024-02-29 12:00 UTC");
        assert_eq!(file_safe("Re: devis/v2"), "Re devis v2");
        assert_eq!(file_safe("   "), "message");
        assert_eq!(file_safe("..hidden"), "hidden");
    }
}
