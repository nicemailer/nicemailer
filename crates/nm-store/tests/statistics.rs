//! Planner statistics (`lib.rs`, `refresh_statistics`): the store keeps the
//! averages of `ANALYZE` (`sqlite_stat1`) and deletes its samples
//! (`sqlite_stat4`), which describe accounts and folders as they were at the
//! last analysis and make a newer account look empty.
//!
//! The query checked is the shape that was slow on a real database: the
//! messages of an account that cite an id, written with a plain `JOIN`, so
//! that the planner is free to choose the order of the tables. (The
//! threading itself no longer gives it that choice: `insert.rs`,
//! `THREADS_CITING`.)

use nm_core::Source;
use nm_store::ingest::parse_headers;
use nm_store::insert::{Flags, Inserter, NewMessage};
use nm_store::{ChangeSet, Store};

const CITING: &str = "SELECT DISTINCT m.thread_id FROM message_reference r JOIN message m ON m.id = r.message_id
  WHERE r.ref_msgid = ?1 AND m.account_id = ?2 AND m.thread_id IS NOT NULL";

/// A new account with its Inbox: (account id, folder id).
fn account(store: &Store, name: &str) -> (i64, i64) {
    let c = store.conn();
    c.execute("INSERT INTO account (config_key, kind, created_by, modified_by) VALUES (?1, 'imap', 'core', 'core')", [name])
        .unwrap();
    let account = c.last_insert_rowid();
    c.execute(
        "INSERT INTO folder (account_id, path, delimiter, role, created_by, modified_by)
         VALUES (?1, 'INBOX', '/', 'inbox', 'core', 'core')",
        [account],
    )
    .unwrap();
    (account, c.last_insert_rowid())
}

/// `n` messages in `folder`, each citing the four before it. Their ids are
/// `<a{account}m{i}@x.example>`.
fn fill(store: &mut Store, account: i64, folder: i64, n: usize) {
    let mut ins = Inserter::new(Source::Imap, None);
    let mut changes = ChangeSet::default();
    let tx = store.transaction().unwrap();
    for i in 0..n {
        let refs: String = (i.saturating_sub(4)..i).map(|j| format!("<a{account}m{j}@x.example> ")).collect();
        let raw = format!(
            "Message-ID: <a{account}m{i}@x.example>\r\nFrom: p{}@x.example\r\nSubject: s{i}\r\nReferences: {refs}\r\n\r\n",
            i % 50
        );
        let h = parse_headers(raw.as_bytes());
        let m = NewMessage {
            account_id: account,
            folder_id: folder,
            uid: Some(i as u32 + 1),
            date_received: i as i64,
            size: None,
            modseq: None,
            flags: Flags::default(),
            auth: (nm_core::AuthResult::Unknown, None),
            headers: &h,
        };
        ins.insert(&tx, &m, None, &mut changes).unwrap();
    }
    ins.commit(tx, &mut changes).unwrap();
}

/// The plan of `CITING` for an id of `account`, values bound: with samples,
/// the planner looks at the values.
fn plan(store: &Store, account: i64) -> Vec<String> {
    let cited = format!("a{account}m10@x.example");
    let mut stmt = store.conn().prepare(&format!("EXPLAIN QUERY PLAN {CITING}")).unwrap();
    stmt.query_map(nm_store::rusqlite::params![cited, account], |r| r.get(3)).unwrap().map(|r| r.unwrap()).collect()
}

fn count(store: &Store, sql: &str) -> i64 {
    store.conn().query_row(sql, [], |r| r.get(0)).unwrap()
}

/// The good plan: the few messages citing the id, then each by its key.
fn assert_citations_first(steps: &[String]) {
    assert!(steps[0].starts_with("SEARCH r USING COVERING INDEX idx_message_reference_ref"), "{steps:?}");
    assert!(steps[1].starts_with("SEARCH m USING INTEGER PRIMARY KEY"), "{steps:?}");
}

/// An account filled after the last analysis, as by an mbox import: with
/// the samples, the planner reads all its messages for each lookup;
/// `Store::optimize` (run at the end of an import) removes them.
#[test]
fn an_account_newer_than_the_statistics_is_not_taken_for_empty() {
    let mut store = Store::open_in_memory().unwrap();
    let (one, inbox_one) = account(&store, "one");
    fill(&mut store, one, inbox_one, 1000);
    // What a database analyzed by an older core holds: samples.
    store.conn().execute_batch("ANALYZE").unwrap();
    let (two, inbox_two) = account(&store, "two");
    fill(&mut store, two, inbox_two, 1000);
    assert!(count(&store, "SELECT count(*) FROM sqlite_stat4") > 0);
    // The trap itself, so that the test fails if SQLite changes and the
    // scenario no longer shows what it is meant to.
    let steps = plan(&store, two);
    assert!(steps[0].starts_with("SEARCH m USING INDEX idx_message_msgid (account_id=?)"), "{steps:?}");

    store.optimize().unwrap();
    assert_eq!(count(&store, "SELECT count(*) FROM sqlite_stat4"), 0);
    assert!(count(&store, "SELECT count(*) FROM sqlite_stat1 WHERE tbl = 'message'") > 0, "the averages are kept");
    assert_citations_first(&plan(&store, two));
    assert_citations_first(&plan(&store, one));
}

/// Opening a database removes the samples an older core left.
#[test]
fn opening_a_database_removes_the_samples() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nicemailer.db");
    let two = {
        let mut store = Store::open(&path).unwrap();
        let (one, inbox_one) = account(&store, "one");
        fill(&mut store, one, inbox_one, 1000);
        store.conn().execute_batch("ANALYZE").unwrap();
        let (two, inbox_two) = account(&store, "two");
        fill(&mut store, two, inbox_two, 1000);
        two
    };
    let store = Store::open(&path).unwrap();
    assert_eq!(count(&store, "SELECT count(*) FROM sqlite_stat4"), 0);
    assert_citations_first(&plan(&store, two));
    // The readers of the pool open after the writing store: no samples.
    let reader = Store::open_reader(&path).unwrap();
    assert_citations_first(&plan(&reader, two));
}
