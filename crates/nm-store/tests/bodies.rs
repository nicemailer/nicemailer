//! The second database file, `nicemailer_body.db` (`schema_body.sql`,
//! schema version 10): the bodies and the search index live there,
//! attached to every connection, and can be rebuilt from the raw files.

use std::path::Path;
use std::sync::Mutex;

use nm_core::Source;
use nm_store::import::{ImportOptions, LOCK_BUDGET, import_mbox};
use nm_store::raw::RawStore;
use nm_store::reindex::{pending, reindex};
use nm_store::{Store, body_path};

/// Two mails, with words found only in their bodies.
const MBOX: &str = "From a@x Mon Jun  1 10:00:00 2020
From: Alice <alice@a.example>
To: Me <me@hey.example>
Subject: Devis toiture
Message-ID: <m1@a.example>

Bonjour, voici le devis pour la charpente.

From b@x Mon Jun  1 11:00:00 2020
From: Bob <bob@b.example>
To: Me <me@hey.example>
Subject: Rendez-vous
Message-ID: <m2@b.example>

Rendez-vous mardi pour le carrelage.

";

/// A database file in `dir` holding the two mails, imported with their raw
/// files.
fn imported(dir: &Path) -> (std::path::PathBuf, RawStore) {
    let db = dir.join("nicemailer.db");
    let raw = RawStore::open(dir).unwrap();
    let mbox = dir.join("m.mbox");
    std::fs::write(&mbox, MBOX).unwrap();
    let store = Mutex::new(Store::open(&db).unwrap());
    let opts = ImportOptions {
        path: mbox,
        account: "hey".into(),
        address: "me@hey.example".into(),
        mark_correspondents: false,
        source: Source::Cli,
        batch_size: 10,
        lock_budget: LOCK_BUDGET,
    };
    import_mbox(&store, &raw, &opts, &mut |_, _, _| {}).unwrap();
    (db, raw)
}

fn hits(store: &Store, words: &str) -> usize {
    store.search(words, 10).unwrap().len()
}

fn count(store: &Store, sql: &str) -> i64 {
    store.conn().query_row(sql, [], |r| r.get(0)).unwrap()
}

#[test]
fn the_bodies_live_in_their_own_file() {
    let dir = tempfile::tempdir().unwrap();
    let (db, _) = imported(dir.path());
    assert!(body_path(&db).ends_with("nicemailer_body.db"));
    assert!(body_path(&db).exists());
    let store = Store::open(&db).unwrap();
    // The main file has no body table; the body file has them.
    assert_eq!(count(&store, "SELECT count(*) FROM main.sqlite_master WHERE name IN ('message_body', 'message_fts')"), 0);
    assert_eq!(count(&store, "SELECT count(*) FROM body.message_body"), 2);
    // Queries name the tables without the file: search and bodies work.
    assert_eq!(hits(&store, "charpente"), 1);
    let thread: i64 = count(&store, "SELECT thread_id FROM message WHERE message_id_hdr = 'm1@a.example'");
    let (_, messages) = store.get_thread(thread).unwrap();
    assert!(messages[0].body_text.as_deref().unwrap_or("").contains("charpente"));
    // A reader of the pool sees them too.
    let reader = Store::open_reader(&db).unwrap();
    assert_eq!(hits(&reader, "carrelage"), 1);
}

#[test]
fn deleting_a_message_deletes_its_body_and_its_search_row() {
    let dir = tempfile::tempdir().unwrap();
    let (db, _) = imported(dir.path());
    let store = Store::open(&db).unwrap();
    let id = count(&store, "SELECT id FROM message WHERE message_id_hdr = 'm1@a.example'");
    store.conn().execute("DELETE FROM message WHERE id = ?1", [id]).unwrap();
    assert_eq!(count(&store, &format!("SELECT count(*) FROM message_body WHERE message_id = {id}")), 0);
    assert_eq!(count(&store, &format!("SELECT count(*) FROM message_fts_docsize WHERE id = {id}")), 0);
    assert_eq!(hits(&store, "charpente"), 0);
    // A change of subject follows in the search index.
    store.conn().execute("UPDATE message SET subject = 'Planning chantier' WHERE message_id_hdr = 'm2@b.example'", []).unwrap();
    assert_eq!(hits(&store, "planning"), 1);
}

#[test]
fn a_deleted_body_file_is_rebuilt_from_the_raw_files() {
    let dir = tempfile::tempdir().unwrap();
    let (db, raw) = imported(dir.path());
    // Rust note: the `-wal` and `-shm` files go with the database file.
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", body_path(&db).display()));
    }
    let store = Store::open(&db).unwrap();
    // At opening, every mail gets its row of the search index back, from
    // the main file: subject, sender and recipients are found at once.
    assert_eq!(hits(&store, "toiture"), 1);
    assert_eq!(hits(&store, "alice"), 1);
    assert_eq!(hits(&store, "charpente"), 0);
    // The bodies are the reinterpretation's to rebuild.
    assert_eq!(pending(&store).unwrap(), 2);
    let store = Mutex::new(store);
    let report = reindex(&store, &raw, 10, &mut |_, _| {}).unwrap();
    assert_eq!(report.reparsed, 2);
    let store = store.into_inner().unwrap();
    assert_eq!(pending(&store).unwrap(), 0);
    assert_eq!(hits(&store, "charpente"), 1);
    assert_eq!(hits(&store, "carrelage"), 1);
}

#[test]
fn rows_left_by_a_crash_between_the_two_commits_are_cleaned_at_opening() {
    let dir = tempfile::tempdir().unwrap();
    let (db, _) = imported(dir.path());
    {
        let store = Store::open(&db).unwrap();
        // As if the body file had committed a mail the main one did not.
        store
            .conn()
            .execute_batch(
                "INSERT INTO body.message_body (message_id, text_plain, parser_version) VALUES (999, 'fantôme', 1);
                 INSERT INTO body.message_fts (rowid, subject, sender, recipients, body) VALUES (999, 'fantome', '', '', '');",
            )
            .unwrap();
        // And as if a search row of the main file's mail were lost.
        let id = count(&store, "SELECT id FROM message WHERE message_id_hdr = 'm2@b.example'");
        store.conn().execute("DELETE FROM message_fts WHERE rowid = ?1", [id]).unwrap();
    }
    let store = Store::open(&db).unwrap();
    assert_eq!(count(&store, "SELECT count(*) FROM message_body WHERE message_id = 999"), 0);
    assert_eq!(count(&store, "SELECT count(*) FROM message_fts_docsize WHERE id = 999"), 0);
    // The row lost is made again, body included.
    assert_eq!(hits(&store, "carrelage"), 1);
    assert_eq!(count(&store, "SELECT count(*) FROM message_fts_docsize"), 2);
}

#[test]
fn a_new_database_empties_a_body_file_left_behind() {
    let dir = tempfile::tempdir().unwrap();
    let (db, _) = imported(dir.path());
    // The main file deleted, the body file kept: the ids start again at 1,
    // the old bodies must not answer for the new mails.
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", db.display()));
    }
    let store = Store::open(&db).unwrap();
    assert_eq!(count(&store, "SELECT count(*) FROM message_body"), 0);
    assert_eq!(count(&store, "SELECT count(*) FROM message_fts_docsize"), 0);
    assert_eq!(hits(&store, "charpente"), 0);
}
