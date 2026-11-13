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
            3 => to_4(&tx)?,
            4 => to_5(&tx)?,
            5 => to_6(&tx)?,
            6 => to_7(&tx)?,
            7 => to_8(&tx)?,
            8 => to_9(&tx)?,
            9 => to_10(&tx)?,
            10 => to_11(&tx)?,
            11 => to_12(&tx)?,
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

/// Version 4, two columns of `account`, added at the end of a small table:
/// instant.
/// - `disabled_at`, set when the key of an IMAP account leaves `accounts.toml`
///   (`configuration.md`, section 6: an account missing from the file is
///   disabled, not deleted). Every existing account starts active; the core
///   disables the ones missing from the file when it reads it, right after.
/// - `position`, the display order of the accounts (account picker,
///   sidebar). Existing accounts keep the order they were shown in until
///   now, their creation order: `position = id`.
fn to_4(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "ALTER TABLE account ADD COLUMN disabled_at INTEGER;
         ALTER TABLE account ADD COLUMN position INTEGER NOT NULL DEFAULT 0;
         UPDATE account SET position = id;",
    )?;
    Ok(())
}

/// Version 5: `draft.after_send` accepts `stay`, the trial option "leave
/// the conversation where it is" of the filing chosen at sending
/// (`reponse.md`, section 8).
///
/// SQLite cannot change a `CHECK` constraint in place: the table is made
/// again, by the procedure of its documentation ("Making Other Kinds Of
/// Table Schema Changes"): a new table, the rows copied, the old one
/// dropped, the new one renamed. `draft` holds a few rows (the replies being
/// written), so this is instant.
///
/// The foreign keys stay on (they cannot be switched off inside a
/// transaction), so dropping `draft` deletes the rows of `draft_attachment`
/// that point to it (`ON DELETE CASCADE`): they are kept aside in a
/// temporary table and put back once the new `draft` has the same ids.
fn to_5(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE draft_new (
             id                  INTEGER PRIMARY KEY,
             account_id          INTEGER NOT NULL REFERENCES account(id) ON DELETE CASCADE,
             from_addr           TEXT NOT NULL,
             from_name           TEXT,
             recipients          TEXT NOT NULL DEFAULT '[]',
             subject             TEXT NOT NULL DEFAULT '',
             body                TEXT NOT NULL DEFAULT '',
             body_format         TEXT NOT NULL DEFAULT 'text' CHECK (body_format IN ('text','html')),
             reply_to_message_id INTEGER REFERENCES message(id) ON DELETE SET NULL,
             after_send          TEXT CHECK (after_send IN ('archive','waiting','todo','stay')),
             message_id_hdr      TEXT NOT NULL,
             message_id          INTEGER REFERENCES message(id) ON DELETE SET NULL,
             revision            INTEGER NOT NULL DEFAULT 1,
             server_revision     INTEGER,
             state               TEXT NOT NULL DEFAULT 'editing' CHECK (state IN ('editing','sending')),
             date_created        INTEGER NOT NULL DEFAULT (unixepoch()),
             date_modified       INTEGER NOT NULL DEFAULT (unixepoch()),
             created_by          TEXT NOT NULL CHECK (created_by  IN ('core','app','cli','tui','web','mcp','config_sync','imap')),
             modified_by         TEXT NOT NULL CHECK (modified_by IN ('core','app','cli','tui','web','mcp','config_sync','imap')),
             UNIQUE (message_id_hdr)
         ) STRICT;
         INSERT INTO draft_new SELECT * FROM draft;
         CREATE TEMP TABLE draft_attachment_kept AS SELECT * FROM draft_attachment;
         DROP TABLE draft;
         ALTER TABLE draft_new RENAME TO draft;
         CREATE INDEX idx_draft_message ON draft (message_id) WHERE message_id IS NOT NULL;
         INSERT INTO draft_attachment SELECT * FROM temp.draft_attachment_kept;
         DROP TABLE temp.draft_attachment_kept;",
    )?;
    Ok(())
}

/// Version 6: `draft.forward_of_message_id`, the mail a forward carries
/// (`reponse.md`, `f`). A column added at the end of a small table:
/// instant. Existing drafts are replies or nothing: NULL.
fn to_6(conn: &Connection) -> Result<()> {
    conn.execute_batch("ALTER TABLE draft ADD COLUMN forward_of_message_id INTEGER REFERENCES message(id) ON DELETE SET NULL;")?;
    Ok(())
}

/// Version 7: `draft.send_account_id`, the account that sends a draft when
/// it is not the draft's own (`reponse.md`, section 4 ter: a reply to a
/// mail of an archive account, or of an account without `[smtp]`, sent
/// from another account). A column added at the end of a small table:
/// instant. Existing drafts are sent by their own account: NULL.
fn to_7(conn: &Connection) -> Result<()> {
    conn.execute_batch("ALTER TABLE draft ADD COLUMN send_account_id INTEGER REFERENCES account(id) ON DELETE SET NULL;")?;
    Ok(())
}

/// Version 8: the virtual folder "All Mail" (`queries.rs`,
/// `list_all_mail`) walks every folder's rows of `thread_folder` in date
/// order at once, newest first, and stops after a page. Without these two
/// indexes it read each folder on its own until a page was found in it: a
/// search found in few folders read them all (160 ms a page instead of 20
/// on 90,000 conversations). Built from `thread_folder`, a row per thread
/// and folder: about a second on a large mailbox.
fn to_8(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE INDEX idx_thread_folder_received ON thread_folder (date_last_received DESC, thread_id DESC);
         CREATE INDEX idx_thread_folder_date_modified ON thread_folder (date_last_modified DESC, thread_id DESC);",
    )?;
    Ok(())
}

/// Version 9: the folder of a conversation is a column of `thread`, and the
/// materialized view `thread_folder` goes (`modele de donnees.md`,
/// "`thread`"; `dossiers.md`, "Dossier d'une conversation").
///
/// 1. Five columns added at the end of `thread`: instant, no rewrite.
/// 2. The folder, flags and live counters of every thread, in one
///    set-based statement (`refresh_all_threads_sql`, the rule of
///    `refresh_thread`): a few seconds on 400,000 mails, where thread by
///    thread would take minutes.
/// 3. `thread_folder` dropped, with its indexes.
/// 4. The indexes of the lists created after the values are written: SQLite
///    builds an index faster in one sort than row by row.
fn to_9(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "ALTER TABLE thread ADD COLUMN folder TEXT CHECK (folder IN ('i','t','w','a','n','p','x','b','s','r','q','d'));
         ALTER TABLE thread ADD COLUMN has_sent INTEGER NOT NULL DEFAULT 0 CHECK (has_sent IN (0,1));
         ALTER TABLE thread ADD COLUMN has_draft INTEGER NOT NULL DEFAULT 0 CHECK (has_draft IN (0,1));
         ALTER TABLE thread ADD COLUMN has_quarantine INTEGER NOT NULL DEFAULT 0 CHECK (has_quarantine IN (0,1));
         ALTER TABLE thread ADD COLUMN has_deleted INTEGER NOT NULL DEFAULT 0 CHECK (has_deleted IN (0,1));",
    )?;
    conn.execute_batch(&crate::refresh_all_threads_sql(false))?;
    conn.execute_batch(
        "DROP TABLE thread_folder;

         CREATE INDEX idx_thread_role_received ON thread (folder, date_last_received, id);
         CREATE INDEX idx_thread_role_account_received ON thread (folder, account_id, date_last_received, id);
         CREATE INDEX idx_thread_role_modified ON thread (folder, date_last_modified, id);
         CREATE INDEX idx_thread_role_account_modified ON thread (folder, account_id, date_last_modified, id);

         CREATE INDEX idx_thread_live_received ON thread (date_last_received, id) WHERE folder NOT IN ('d','x','b');
         CREATE INDEX idx_thread_live_account_received ON thread (account_id, date_last_received, id) WHERE folder NOT IN ('d','x','b');
         CREATE INDEX idx_thread_live_modified ON thread (date_last_modified, id) WHERE folder NOT IN ('d','x','b');
         CREATE INDEX idx_thread_live_account_modified ON thread (account_id, date_last_modified, id) WHERE folder NOT IN ('d','x','b');

         CREATE INDEX idx_thread_sent_received ON thread (date_last_received, id) WHERE has_sent = 1;
         CREATE INDEX idx_thread_sent_account_received ON thread (account_id, date_last_received, id) WHERE has_sent = 1;
         CREATE INDEX idx_thread_sent_modified ON thread (date_last_modified, id) WHERE has_sent = 1;
         CREATE INDEX idx_thread_sent_account_modified ON thread (account_id, date_last_modified, id) WHERE has_sent = 1;
         CREATE INDEX idx_thread_draft_received ON thread (date_last_received, id) WHERE has_draft = 1;
         CREATE INDEX idx_thread_draft_account_received ON thread (account_id, date_last_received, id) WHERE has_draft = 1;
         CREATE INDEX idx_thread_draft_modified ON thread (date_last_modified, id) WHERE has_draft = 1;
         CREATE INDEX idx_thread_draft_account_modified ON thread (account_id, date_last_modified, id) WHERE has_draft = 1;
         CREATE INDEX idx_thread_quarantine_received ON thread (date_last_received, id) WHERE has_quarantine = 1;
         CREATE INDEX idx_thread_quarantine_account_received ON thread (account_id, date_last_received, id) WHERE has_quarantine = 1;
         CREATE INDEX idx_thread_quarantine_modified ON thread (date_last_modified, id) WHERE has_quarantine = 1;
         CREATE INDEX idx_thread_quarantine_account_modified ON thread (account_id, date_last_modified, id) WHERE has_quarantine = 1;
         CREATE INDEX idx_thread_deleted_received ON thread (date_last_received, id) WHERE has_deleted = 1;
         CREATE INDEX idx_thread_deleted_account_received ON thread (account_id, date_last_received, id) WHERE has_deleted = 1;
         CREATE INDEX idx_thread_deleted_modified ON thread (date_last_modified, id) WHERE has_deleted = 1;
         CREATE INDEX idx_thread_deleted_account_modified ON thread (account_id, date_last_modified, id) WHERE has_deleted = 1;

         CREATE INDEX idx_message_received ON message (date_received);",
    )?;
    Ok(())
}

/// Version 10: the bodies and the search index move to the second file,
/// `nicemailer_body.db` (`schema_body.sql`; `modele de donnees.md`, "Deux
/// fichiers"), attached as `body` and given its tables before the
/// migrations run (`lib.rs`, `init`).
///
/// - The bodies: copied row by row (`INSERT … SELECT`), about 7 GB on a
///   real mailbox, a matter of seconds since nothing is parsed.
/// - The search index: its five shadow tables are copied as they are. An
///   FTS5 table keeps all its state in them (`_data` the index, `_idx`
///   its segments, `_content` the text, `_docsize` the sizes, `_config` its
///   settings), and the new table was created with the same settings, so
///   the copy is that index, without tokenizing 400,000 mails again.
/// - The old tables and their triggers are dropped; the triggers that tie
///   the two files are now TEMP (`lib.rs`, `TEMP_TRIGGERS`).
///
/// The rows of the new file are deleted first: if a previous attempt
/// committed the body file but not the main one (two files commit
/// separately), the migration runs again from the start without
/// duplicates. The main file is vacuumed afterwards, outside the
/// transaction (`lib.rs`).
fn to_10(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "DELETE FROM body.message_body;
         INSERT INTO body.message_body
                (message_id, text_plain, html_sanitized, has_remote_content, parser_version, date_fetched, tracking_pixels)
         SELECT message_id, text_plain, html_sanitized, has_remote_content, parser_version, date_fetched, tracking_pixels
           FROM main.message_body;

         DELETE FROM body.message_fts_data;
         DELETE FROM body.message_fts_idx;
         DELETE FROM body.message_fts_content;
         DELETE FROM body.message_fts_docsize;
         DELETE FROM body.message_fts_config;
         INSERT INTO body.message_fts_data SELECT * FROM main.message_fts_data;
         INSERT INTO body.message_fts_idx SELECT * FROM main.message_fts_idx;
         INSERT INTO body.message_fts_content SELECT * FROM main.message_fts_content;
         INSERT INTO body.message_fts_docsize SELECT * FROM main.message_fts_docsize;
         INSERT INTO body.message_fts_config SELECT * FROM main.message_fts_config;

         DROP TRIGGER main.message_fts_au;
         DROP TRIGGER main.message_fts_ad;
         DROP TRIGGER main.message_body_fts_au;
         DROP TABLE main.message_fts;
         DROP TABLE main.message_body;",
    )?;
    Ok(())
}

/// Version 11: the partial indexes of the virtual folder Work (Inbox, Todo
/// and Waiting together, `ui.md`, "Dossiers virtuels"). They hold only the
/// conversations to deal with: instant.
fn to_11(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE INDEX idx_thread_work_received ON thread (date_last_received, id) WHERE folder IN ('i','t','w');
         CREATE INDEX idx_thread_work_account_received ON thread (account_id, date_last_received, id) WHERE folder IN ('i','t','w');
         CREATE INDEX idx_thread_work_modified ON thread (date_last_modified, id) WHERE folder IN ('i','t','w');
         CREATE INDEX idx_thread_work_account_modified ON thread (account_id, date_last_modified, id) WHERE folder IN ('i','t','w');",
    )?;
    Ok(())
}

/// Version 12: people and robots (`expediteurs.md`). Columns added at the
/// end of `address`, `message` and `thread`, which SQLite does without
/// rewriting the tables, and the partial index of the mails whose clues are
/// still to read (all of them at first).
///
/// `address.written_to` is filled here, in one statement over the mails of
/// Sent (a few thousand rows of `message_address`). The rest needs the
/// headers of every stored mail, read from the raw files: that is the
/// catch-up done in the background (`senders.rs`), so the app opens
/// at once.
fn to_12(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "ALTER TABLE address ADD COLUMN sender_kind TEXT CHECK (sender_kind IN ('person','robot'));
         ALTER TABLE address ADD COLUMN sender_kind_user TEXT CHECK (sender_kind_user IN ('person','robot'));
         ALTER TABLE address ADD COLUMN date_sender_kind_modified INTEGER;
         ALTER TABLE address ADD COLUMN written_to INTEGER NOT NULL DEFAULT 0 CHECK (written_to IN (0,1));
         ALTER TABLE address ADD COLUMN sender_count INTEGER NOT NULL DEFAULT 0;
         ALTER TABLE address ADD COLUMN sender_bulk INTEGER NOT NULL DEFAULT 0;
         ALTER TABLE address ADD COLUMN sender_list INTEGER NOT NULL DEFAULT 0;
         ALTER TABLE address ADD COLUMN sender_human INTEGER NOT NULL DEFAULT 0;
         ALTER TABLE address ADD COLUMN sender_client INTEGER NOT NULL DEFAULT 0;
         ALTER TABLE message ADD COLUMN sender_signals INTEGER;
         ALTER TABLE message ADD COLUMN sender_kind TEXT CHECK (sender_kind IN ('person','robot'));
         ALTER TABLE thread ADD COLUMN has_person INTEGER NOT NULL DEFAULT 0 CHECK (has_person IN (0,1));
         CREATE INDEX idx_message_sender_pending ON message (id) WHERE sender_signals IS NULL;
         UPDATE address SET written_to = 1 WHERE id IN (
             SELECT ma.address_id FROM message_address ma
               JOIN message m ON m.id = ma.message_id
               JOIN folder f ON f.id = m.folder_id
              WHERE f.role = 'sent' AND ma.role IN ('to', 'cc', 'bcc'));",
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
