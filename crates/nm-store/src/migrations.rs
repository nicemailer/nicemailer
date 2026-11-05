//! Schema migrations: bring a database created by an earlier version up to
//! `SCHEMA_VERSION`.
//!
//! A fresh database is created directly at the latest version by
//! `specifications/schema.sql`. An existing one is upgraded here, one
//! version at a time, each step in its own transaction: if a step fails
//! (disk full, crash), SQLite rolls it back and the database stays at the
//! previous version, ready to be migrated again at the next start.
//!
//! The test `migrating_version_1_gives_the_latest_schema`
//! (`tests/migrations.rs`) checks that a migrated database has exactly the
//! tables, columns and indexes of a fresh one, so `schema.sql` and this file
//! cannot drift apart unnoticed.

use rusqlite::{Connection, TransactionBehavior, params};

use crate::Result;
use crate::encoded_words::decode_leftovers;

/// Upgrades the database from `version` to `SCHEMA_VERSION`.
pub(crate) fn migrate(conn: &mut Connection, mut version: i64) -> Result<()> {
    while version < crate::SCHEMA_VERSION {
        // Rust note: `TransactionBehavior::Immediate` takes the write lock
        // at once (`BEGIN IMMEDIATE`), so no other connection can write
        // between our reads and our writes.
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        match version {
            1 => to_2(&tx)?,
            2 => to_3(&tx)?,
            _ => unreachable!("aucune migration depuis la version {version}"),
        }
        version += 1;
        // `PRAGMA user_version` takes no bound parameter: the number is
        // written into the SQL text (an integer of ours, not user input).
        tx.execute_batch(&format!("PRAGMA user_version = {version}"))?;
        tx.commit()?;
    }
    Ok(())
}

/// Version 2: indexes for large mailboxes (`modele de donnees.md`,
/// "Index"), the date of last modification of a thread for Everything, and
/// the repair of names and subjects with encoded words glued inside a word
/// (`encoded_words.rs`).
fn to_2(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "DROP INDEX idx_message_thread;
         CREATE INDEX idx_message_thread ON message (thread_id, folder_id, date_received, id);
         CREATE INDEX idx_message_thread_flagged ON message (thread_id) WHERE is_flagged = 1;
         CREATE INDEX idx_message_thread_answered ON message (thread_id) WHERE is_answered = 1;
         CREATE INDEX idx_message_thread_attachments ON message (folder_id, thread_id) WHERE has_attachments = 1;

         ALTER TABLE thread ADD COLUMN date_last_modified INTEGER NOT NULL DEFAULT 0;
         UPDATE thread SET date_last_modified =
             coalesce((SELECT max(date_last_modified) FROM thread_folder WHERE thread_id = thread.id), 0);
         CREATE INDEX idx_thread_received ON thread (date_last_received, id);
         CREATE INDEX idx_thread_account_received ON thread (account_id, date_last_received, id);
         CREATE INDEX idx_thread_modified ON thread (date_last_modified, id);
         CREATE INDEX idx_thread_account_modified ON thread (account_id, date_last_modified, id);",
    )?;
    repair_encoded_words(conn)?;
    fill_empty_address_names(conn)?;
    Ok(())
}

/// Version 3: remote images (`ui.md`, "Images distantes"): the choice to
/// load them for a sender, and the count of tracking pixels removed from a
/// body. Two columns added at the end of their tables, which SQLite does
/// without rewriting them: instant on any mailbox. The bodies themselves
/// are brought up to date by the reinterpretation (`PARSER_VERSION` 8),
/// in the background.
fn to_3(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "ALTER TABLE address ADD COLUMN remote_images INTEGER NOT NULL DEFAULT 0 CHECK (remote_images IN (0,1));
         ALTER TABLE message_body ADD COLUMN tracking_pixels INTEGER NOT NULL DEFAULT 0;",
    )?;
    Ok(())
}

/// Decodes the encoded words left in the names and subjects stored before
/// `encoded_words.rs` existed: in `message`, `thread`, `address` and the
/// search index. Only rows containing `=?` are read, a few thousand at most.
fn repair_encoded_words(conn: &Connection) -> Result<()> {
    // Messages: sender name and subject, then their row of the search index,
    // whose `sender` column is "name address" (as written by `import.rs`).
    let rows: Vec<(i64, Option<String>, Option<String>, String)> = conn
        .prepare("SELECT id, from_name, from_addr, subject FROM message WHERE from_name LIKE '%=?%' OR subject LIKE '%=?%'")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<std::result::Result<_, _>>()?;
    let mut update_message = conn.prepare("UPDATE message SET from_name = ?2, subject = ?3 WHERE id = ?1")?;
    let mut update_fts = conn.prepare("UPDATE message_fts SET sender = ?2, subject = ?3 WHERE rowid = ?1")?;
    for (id, name, addr, subject) in rows {
        let name = name.map(|n| decode_leftovers(&n).trim().to_string()).filter(|n| !n.is_empty());
        let subject = decode_leftovers(&subject).into_owned();
        let sender = format!("{} {}", name.as_deref().unwrap_or(""), addr.as_deref().unwrap_or(""));
        update_message.execute(params![id, name, subject])?;
        update_fts.execute(params![id, sender.trim(), subject])?;
    }
    // Recipient names only exist in the search index.
    let rows: Vec<(i64, String)> = conn
        .prepare("SELECT rowid, recipients FROM message_fts WHERE recipients LIKE '%=?%'")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?;
    let mut update = conn.prepare("UPDATE message_fts SET recipients = ?2 WHERE rowid = ?1")?;
    for (id, recipients) in rows {
        update.execute(params![id, decode_leftovers(&recipients).as_ref()])?;
    }
    // Threads take the subject of their first message; addresses keep a name.
    for (select, update) in [
        ("SELECT id, subject FROM thread WHERE subject LIKE '%=?%'", "UPDATE thread SET subject = ?2 WHERE id = ?1"),
        ("SELECT id, name FROM address WHERE name LIKE '%=?%'", "UPDATE address SET name = ?2 WHERE id = ?1"),
    ] {
        let rows: Vec<(i64, String)> = conn
            .prepare(select)?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<std::result::Result<_, _>>()?;
        let mut stmt = conn.prepare(update)?;
        for (id, text) in rows {
            stmt.execute(params![id, decode_leftovers(&text).trim()])?;
        }
    }
    Ok(())
}

/// Gives an address without a name the name of the oldest mail it sent
/// that carries one: the rule of the ingestion ("the first encounter that
/// has a name", `schema.sql`, table `address`), applied to the mail already
/// stored. Recipient names are not stored per message, so only senders can
/// be used here; the ingestion fills the others as new mail arrives.
fn fill_empty_address_names(conn: &Connection) -> Result<()> {
    conn.execute(
        "UPDATE address SET
            name = (SELECT m.from_name FROM message m
                     WHERE m.from_address_id = address.id AND m.from_name <> ''
                     ORDER BY m.date_received, m.id LIMIT 1),
            date_modified = unixepoch(), modified_by = 'core'
          WHERE coalesce(name, '') = ''
            AND EXISTS (SELECT 1 FROM message m WHERE m.from_address_id = address.id AND m.from_name <> '')",
        [],
    )?;
    Ok(())
}
