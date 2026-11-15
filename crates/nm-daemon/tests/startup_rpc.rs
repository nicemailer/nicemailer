//! End to end: a core that cannot run on its data directory says why in
//! `core-error.json`, where the app reads it; a core that starts removes
//! it (`docs/app-bundle.md`, "When the core cannot start").

mod common;

use std::path::Path;

use common::{Client, start, start_logged};
use serde_json::json;

fn report(dir: &Path) -> nm_api::StartupError {
    let text = std::fs::read_to_string(dir.join("core-error.json")).expect("core-error.json written");
    serde_json::from_str(&text).unwrap()
}

#[test]
fn an_early_development_database_is_reported_then_cleared() {
    let dir = tempfile::tempdir().unwrap();
    // A version 1 that lacks what the migrations assume: refused.
    nm_store::rusqlite::Connection::open(dir.path().join("nicemailer.db"))
        .unwrap()
        .execute_batch("CREATE TABLE account (id INTEGER PRIMARY KEY); PRAGMA user_version = 1;")
        .unwrap();
    let mut core = start(dir.path());
    assert!(!core.exit_status(20).success());
    let r = report(dir.path());
    assert_eq!(r.kind, "outdated");
    assert_eq!(r.missing.as_deref(), Some("account.kind"));
    assert!(r.message.contains("nicemailer.db"), "{}", r.message);
    assert!(r.at > 0);
    assert!(!dir.path().join(".core-error.json.tmp").exists(), "no temporary file left");

    // A database the core can open: it starts, and the reason goes away.
    std::fs::remove_file(dir.path().join("nicemailer.db")).unwrap();
    let _core = start(dir.path());
    Client::connect(dir.path());
    assert!(!dir.path().join("core-error.json").exists());
}

#[test]
fn a_database_of_a_newer_version_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    nm_store::rusqlite::Connection::open(dir.path().join("nicemailer.db"))
        .unwrap()
        .execute_batch("PRAGMA user_version = 99;")
        .unwrap();
    let mut core = start(dir.path());
    assert!(!core.exit_status(20).success());
    let r = report(dir.path());
    assert_eq!((r.kind.as_str(), r.found, r.expected), ("too_new", Some(99), Some(nm_store::SCHEMA_VERSION)));
}

/// A database of an earlier version that migrates slowly: the body file is
/// locked by another connection, so the migration waits on it. Meanwhile
/// `core-upgrading.json` says so (the app shows "upgrading" rather than
/// "unreachable"), and a SIGTERM stops the core with code 0: launchd does
/// not start it again, where a core killed in the middle of a migration was
/// restarted and started the migration over (`docs/app-bundle.md`).
#[test]
fn an_upgrade_is_announced_and_a_stop_cancels_it_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("nicemailer.db");
    nm_store::rusqlite::Connection::open(&db)
        .unwrap()
        .execute_batch(include_str!("../../nm-store/tests/fixtures/schema_v1.sql"))
        .unwrap();
    // A write lock on the body file, held by this test.
    let blocker = nm_store::rusqlite::Connection::open(nm_store::body_path(&db)).unwrap();
    blocker.execute_batch("PRAGMA journal_mode = WAL; CREATE TABLE hold (x); BEGIN IMMEDIATE; INSERT INTO hold VALUES (1);").unwrap();
    let mut core = start(dir.path());
    let status_path = dir.path().join("core-upgrading.json");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !status_path.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let status: nm_api::UpgradeStatus = serde_json::from_str(&std::fs::read_to_string(&status_path).unwrap()).unwrap();
    assert_eq!((status.from, status.to, status.pid), (1, nm_store::SCHEMA_VERSION, core.pid()));
    assert!(status.started_at > 0);
    // Stopped while it waits: code 0, the file gone, the database still at
    // version 1 (the migration rolled back).
    unsafe { libc::kill(core.pid() as i32, libc::SIGTERM) };
    assert!(core.exit_status(20).success());
    assert!(!status_path.exists());
    drop(blocker);
    let version: i64 = nm_store::rusqlite::Connection::open(&db).unwrap().query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
    assert_eq!(version, 1);
    // Started again without the lock: it upgrades, opens, and the file goes.
    let _core = start(dir.path());
    Client::connect(dir.path());
    assert!(!status_path.exists());
}

/// A core stopped by SIGTERM (launchd, "Stop the Core") closes cleanly and
/// marks its files: the next start skips the reconciliation of the two
/// files (`nm_store::clean_close`), the longest step of its opening on a
/// real mailbox. A core killed instead leaves no mark: the start after it
/// reconciles.
#[test]
fn a_clean_stop_spares_the_next_start_the_reconciliation() {
    let dir = tempfile::tempdir().unwrap();
    let log = |n: u32| dir.path().join(format!("core{n}.log"));
    let read = |n: u32| std::fs::read_to_string(log(n)).unwrap();
    let mut core = start_logged(dir.path(), &[], Some(&log(1)));
    Client::connect(dir.path()).call("ping", json!({})).unwrap();
    unsafe { libc::kill(core.pid() as i32, libc::SIGTERM) };
    assert!(core.exit_status(10).success());
    assert!(read(1).contains("clean close: the next start skips"), "{}", read(1));
    assert!(dir.path().join(nm_store::clean_close::FILE).exists());

    // Started again: no reconciliation, and the mark is gone.
    let mut core = start_logged(dir.path(), &[], Some(&log(2)));
    Client::connect(dir.path()).call("ping", json!({})).unwrap();
    assert!(read(2).contains("clean close, no reconcile"), "{}", read(2));
    assert!(!dir.path().join(nm_store::clean_close::FILE).exists());
    // Killed, as by a crash: no mark, and the next start reconciles.
    unsafe { libc::kill(core.pid() as i32, libc::SIGKILL) };
    core.exit_status(10);
    assert!(!dir.path().join(nm_store::clean_close::FILE).exists());
    let _core = start_logged(dir.path(), &[], Some(&log(3)));
    Client::connect(dir.path()).call("ping", json!({})).unwrap();
    assert!(read(3).contains(", reconcile "), "{}", read(3));
}
