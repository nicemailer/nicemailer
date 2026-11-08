//! End to end: a core that cannot run on its data directory says why in
//! `core-error.json`, where the app reads it; a core that starts removes
//! it (`docs/app-bundle.md`, "When the core cannot start").

mod common;

use std::path::Path;

use common::{Client, start};

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
