//! Integration tests of the schema migrations (`migrations.rs`).
//!
//! `fixtures/schema_v1.sql` is the schema of version 1, as released before
//! version 2. A database created with it and filled with a few rows must,
//! once opened by the current code, have exactly the schema of a fresh
//! database (`specifications/schema.sql`) and repaired data.

use std::collections::BTreeMap;
use std::path::Path;

use nm_store::rusqlite::Connection;
use nm_store::{SCHEMA_VERSION, Store, StoreError};

const SCHEMA_V1: &str = include_str!("fixtures/schema_v1.sql");

/// A version 1 database at `path`, with one account, two folders (Inbox
/// and Sent), and one thread of two mails across them:
/// - mail 1, received, from "Achille Ber=?ISO-8859-1?B?6A==?=s" (an
///   encoded word glued inside a word), subject "2=?ISO-8859-1?B?6A==?=me
///   rappel", modified at 500;
/// - mail 2, sent later, to an address without a name, modified at 900.
///
/// The address of mail 1's sender has no name either.
fn version_1(path: &Path) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(SCHEMA_V1).unwrap();
    conn.execute_batch(
        "INSERT INTO account (id, config_key, kind, address, created_by, modified_by)
              VALUES (1, 'perso', 'archive', 'me@x.org', 'cli', 'cli');
         INSERT INTO folder (id, account_id, path, role, created_by, modified_by) VALUES
              (1, 1, 'Inbox', 'inbox', 'core', 'core'), (2, 1, 'Sent', 'sent', 'core', 'core');
         INSERT INTO address (id, addr, name, created_by, modified_by) VALUES
              (1, 'achille@x.org', NULL, 'core', 'core'),
              (2, 'bob@x.org', '', 'core', 'core'),
              (3, 'carol@x.org', 'Ang=?ISO-8859-1?B?6Q==?=lique', 'core', 'core');
         INSERT INTO thread (id, account_id, subject, date_last_received, message_count, unread_count)
              VALUES (1, 1, '2=?ISO-8859-1?B?6A==?=me rappel', 200, 2, 1);
         INSERT INTO message (id, account_id, folder_id, thread_id, subject, from_address_id, from_name, from_addr,
                              date_received, is_seen, date_modified, created_by, modified_by) VALUES
              (1, 1, 1, 1, '2=?ISO-8859-1?B?6A==?=me rappel', 1, 'Achille Ber=?ISO-8859-1?B?6A==?=s', 'achille@x.org',
               100, 0, 500, 'core', 'core'),
              (2, 1, 2, 1, 'Re: 2ème rappel', NULL, 'Me', 'me@x.org', 200, 1, 900, 'core', 'core');
         INSERT INTO message_address (message_id, role, position, address_id) VALUES (2, 'to', 0, 2);
         INSERT INTO thread_folder (thread_id, folder_id, date_last_received, date_last_modified, message_count, unread_count)
              VALUES (1, 1, 100, 500, 1, 1), (1, 2, 200, 900, 1, 0);
         INSERT INTO message_fts (rowid, subject, sender, recipients, body) VALUES
              (1, '2=?ISO-8859-1?B?6A==?=me rappel', 'Achille Ber=?ISO-8859-1?B?6A==?=s achille@x.org', 'Me me@x.org', 'Bonjour'),
              (2, 'Re: 2ème rappel', 'Me me@x.org', 'Herv=?ISO-8859-1?B?6SA=?=Mainguet bob@x.org', 'Merci');",
    )
    .unwrap();
}

/// Tables with their columns, and indexes with their definition, of the
/// database at `path`, comparable between two databases. Columns are read
/// with `table_xinfo` (name, type, not null, default, primary key): the
/// text of `CREATE TABLE` differs after `ALTER TABLE ADD COLUMN` even when
/// the columns are the same. Index definitions are compared with their
/// whitespace collapsed.
fn schema_of(path: &Path) -> BTreeMap<String, String> {
    let conn = Connection::open(path).unwrap();
    let mut out = BTreeMap::new();
    let objects: Vec<(String, String, Option<String>)> = conn
        .prepare("SELECT type, name, sql FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for (kind, name, sql) in objects {
        let description = if kind == "table" {
            conn.prepare(&format!("PRAGMA table_xinfo(\"{name}\")"))
                .unwrap()
                .query_map([], |r| {
                    Ok(format!(
                        "{} {} {} {:?} {}",
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, Option<String>>(4)?,
                        r.get::<_, i64>(5)?
                    ))
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
                .join(", ")
        } else {
            sql.unwrap_or_default().split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
        };
        out.insert(format!("{kind} {name}"), description);
    }
    out
}

#[test]
fn migrating_version_1_gives_the_latest_schema() {
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("old.db");
    version_1(&old);
    drop(Store::open(&old).unwrap());
    let fresh = dir.path().join("fresh.db");
    drop(Store::open(&fresh).unwrap());
    let (migrated, created) = (schema_of(&old), schema_of(&fresh));
    // Compared one object at a time, for a readable failure.
    for (name, def) in &created {
        assert_eq!(migrated.get(name), Some(def), "{name}");
    }
    assert_eq!(migrated.keys().collect::<Vec<_>>(), created.keys().collect::<Vec<_>>());
    let version: i64 = Connection::open(&old).unwrap().query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
    assert_eq!(version, SCHEMA_VERSION);
}

#[test]
fn migration_repairs_names_and_subjects_and_fills_thread_dates() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    version_1(&path);
    let store = Store::open(&path).unwrap();
    let conn = store.conn();
    let text = |sql: &str| -> Option<String> { conn.query_row(sql, [], |r| r.get(0)).unwrap() };
    let int = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap() };
    // Version 4: existing accounts are active, in their creation order.
    assert_eq!(int("SELECT position FROM account WHERE id = 1"), 1);
    assert_eq!(int("SELECT count(*) FROM account WHERE disabled_at IS NOT NULL"), 0);
    // Encoded words glued inside words are decoded everywhere.
    assert_eq!(text("SELECT from_name FROM message WHERE id = 1").as_deref(), Some("Achille Berès"));
    assert_eq!(text("SELECT subject FROM message WHERE id = 1").as_deref(), Some("2ème rappel"));
    assert_eq!(text("SELECT subject FROM thread WHERE id = 1").as_deref(), Some("2ème rappel"));
    assert_eq!(text("SELECT name FROM address WHERE id = 3").as_deref(), Some("Angélique"));
    assert_eq!(text("SELECT sender FROM message_fts WHERE rowid = 1").as_deref(), Some("Achille Berès achille@x.org"));
    assert_eq!(text("SELECT subject FROM message_fts WHERE rowid = 1").as_deref(), Some("2ème rappel"));
    assert_eq!(text("SELECT recipients FROM message_fts WHERE rowid = 2").as_deref(), Some("Hervé Mainguet bob@x.org"));
    // The search index finds the decoded words.
    assert_eq!(int("SELECT count(*) FROM message_fts WHERE message_fts MATCH 'beres'"), 1);
    assert_eq!(int("SELECT count(*) FROM message_fts WHERE message_fts MATCH 'mainguet'"), 1);
    // An address without a name gets the one of the mail it sent; one that
    // never sent a mail with a name keeps none.
    assert_eq!(text("SELECT name FROM address WHERE id = 1").as_deref(), Some("Achille Berès"));
    assert_eq!(text("SELECT modified_by FROM address WHERE id = 1").as_deref(), Some("core"));
    assert_eq!(text("SELECT name FROM address WHERE id = 2").as_deref(), Some(""));
    // The thread's last modification spans its folders.
    assert_eq!(int("SELECT date_last_modified FROM thread WHERE id = 1"), 900);
    drop(store);
    // Opening it again (now at version 2) changes nothing.
    let store = Store::open(&path).unwrap();
    let name: String = store.conn().query_row("SELECT from_name FROM message WHERE id = 1", [], |r| r.get(0)).unwrap();
    assert_eq!(name, "Achille Berès");
}

#[test]
fn a_migrated_database_lists_everything() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    version_1(&path);
    let store = Store::open(&path).unwrap();
    let (items, _) = store
        .list_everything(None, None, 10, nm_api::ThreadSort::Modified, &nm_api::ThreadFilter::default())
        .unwrap();
    assert_eq!(items.len(), 1);
    // Shown in the folder of its latest mail (Sent), with that folder's
    // counters.
    assert_eq!((items[0].folder_id, items[0].subject.as_str(), items[0].unread_count), (2, "2ème rappel", 0));
    // The From filter goes through the address, whose name was filled in.
    let from = nm_api::ThreadFilter { from: vec!["berès".into()], ..Default::default() };
    let (items, _) = store.list_everything(None, None, 10, nm_api::ThreadSort::Received, &from).unwrap();
    assert_eq!(items.len(), 1);
}

#[test]
fn an_incomplete_version_1_is_refused_not_migrated() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    version_1(&path);
    Connection::open(&path).unwrap().execute_batch("DROP TABLE message_unsubscribe;").unwrap();
    assert!(matches!(Store::open(&path), Err(StoreError::OutdatedSchema("message_unsubscribe"))));
    let version: i64 = Connection::open(&path).unwrap().query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
    assert_eq!(version, 1);
}

#[test]
fn a_newer_database_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("new.db");
    drop(Store::open(&path).unwrap());
    Connection::open(&path).unwrap().execute_batch(&format!("PRAGMA user_version = {};", SCHEMA_VERSION + 1)).unwrap();
    assert!(matches!(Store::open(&path), Err(StoreError::SchemaTooNew { .. })));
}
