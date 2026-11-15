//! The clean close (`nm_store::clean_close`): the opening skips the
//! reconciliation of the two files after it, and only after it.

use nm_store::{Store, body_path, clean_close};
use std::path::Path;

/// The steps of an opening of `path`, by name.
fn opening_steps(path: &Path) -> Vec<&'static str> {
    let (_, steps) = Store::open_timed(path).unwrap();
    steps.steps.iter().map(|(name, _)| *name).collect()
}

fn reconciled(steps: &[&str]) -> bool {
    steps.contains(&"reconcile")
}

/// A database opened once, closed cleanly.
fn closed_cleanly(dir: &Path) -> std::path::PathBuf {
    let path = dir.join("nicemailer.db");
    let store = Store::open(&path).unwrap();
    assert!(store.close_cleanly().unwrap().marked);
    drop(store);
    path
}

/// After a clean close, the next opening skips the reconciliation, and
/// takes the mark away: the one after reconciles again.
#[test]
fn a_clean_close_spares_the_next_opening_only() {
    let dir = tempfile::tempdir().unwrap();
    let path = closed_cleanly(dir.path());
    assert!(dir.path().join(clean_close::FILE).exists());
    let steps = opening_steps(&path);
    assert!(!reconciled(&steps), "{steps:?}");
    assert!(steps.contains(&"clean close, no reconcile"), "{steps:?}");
    assert!(!dir.path().join(clean_close::FILE).exists());
    // No clean close since: as after a crash.
    assert!(reconciled(&opening_steps(&path)));
}

/// Without any mark (a first start, a crash, a kill): reconciled.
#[test]
fn no_mark_reconciles() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nicemailer.db");
    drop(Store::open(&path).unwrap());
    assert!(reconciled(&opening_steps(&path)));
}

/// A body file written after the clean close (an orphan body, as a stop
/// in the middle of a write would leave): reconciled, and the orphan
/// removed.
#[test]
fn a_file_written_after_the_close_reconciles() {
    let dir = tempfile::tempdir().unwrap();
    let path = closed_cleanly(dir.path());
    let body = rusqlite::Connection::open(body_path(&path)).unwrap();
    body.execute(
        "INSERT INTO message_body (message_id, text_plain, parser_version) VALUES (424242, 'orphan', 1)",
        [],
    )
    .unwrap();
    drop(body);
    let (store, steps) = Store::open_timed(&path).unwrap();
    assert!(steps.steps.iter().any(|(n, _)| *n == "reconcile"));
    drop(store);
    let left: i64 = rusqlite::Connection::open(body_path(&path))
        .unwrap()
        .query_row("SELECT count(*) FROM message_body WHERE message_id = 424242", [], |r| r.get(0))
        .unwrap();
    assert_eq!(left, 0);
}

/// The body file deleted or replaced after the clean close: reconciled.
#[test]
fn a_file_replaced_after_the_close_reconciles() {
    let dir = tempfile::tempdir().unwrap();
    let path = closed_cleanly(dir.path());
    std::fs::remove_file(body_path(&path)).unwrap();
    assert!(reconciled(&opening_steps(&path)));

    let path = closed_cleanly(dir.path());
    // A copy put back in its place (a backup restored): same content,
    // another inode.
    let copy = dir.path().join("copy.db");
    std::fs::copy(&path, &copy).unwrap();
    std::fs::rename(&copy, &path).unwrap();
    assert!(reconciled(&opening_steps(&path)));
}

/// A mark that is not one (damaged, from an older version): reconciled,
/// and removed.
#[test]
fn a_damaged_mark_reconciles() {
    let dir = tempfile::tempdir().unwrap();
    let path = closed_cleanly(dir.path());
    std::fs::write(dir.path().join(clean_close::FILE), b"{not json").unwrap();
    assert!(reconciled(&opening_steps(&path)));
    assert!(!dir.path().join(clean_close::FILE).exists());
}

/// A database in memory has no file to mark.
#[test]
fn a_database_in_memory_is_never_marked() {
    let store = Store::open_in_memory().unwrap();
    assert!(!store.close_cleanly().unwrap().marked);
}

/// A reader still in a read when the core stops: the log cannot be
/// emptied, so no mark (the next start reconciles), and the close does not
/// wait long for it: launchd kills a core that takes 20 s to stop.
#[test]
fn a_busy_reader_leaves_no_mark_and_no_long_wait() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nicemailer.db");
    let store = Store::open(&path).unwrap();
    // Something in the log, then a reader that holds its snapshot.
    store.conn().execute_batch("CREATE TABLE IF NOT EXISTS probe (x); INSERT INTO probe VALUES (1);").unwrap();
    let reader = rusqlite::Connection::open(&path).unwrap();
    reader.execute_batch("BEGIN; SELECT count(*) FROM probe;").unwrap();
    let started = std::time::Instant::now();
    let report = store.close_cleanly().unwrap();
    assert!(!report.marked);
    assert!(started.elapsed() < std::time::Duration::from_secs(2), "{:?}", started.elapsed());
    assert!(!dir.path().join(clean_close::FILE).exists());
    reader.execute_batch("COMMIT;").unwrap();
}
