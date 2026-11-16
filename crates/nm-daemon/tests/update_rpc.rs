//! End to end: the maintenance mode of the core and the update of
//! NiceMailer (`specifications/mise-a-jour.md`, section 7, "Tests
//! exigés"; `docs/updates.md`). The real core, its socket, and clients
//! declaring themselves `app` or `cli`.

mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use common::{Client, start, start_with};
use serde_json::{Value, json};

/// The `data` of a refusal for maintenance, checked to be one.
fn maintenance(e: &Value) -> nm_api::Maintenance {
    assert_eq!(e["code"], nm_api::error_code::MAINTENANCE, "{e}");
    serde_json::from_value(e["data"].clone()).unwrap()
}

fn reason(e: &Value) -> nm_api::MaintenanceReason {
    maintenance(e).reason
}

/// The schema version of the database of `dir`.
fn version(dir: &Path) -> i64 {
    let db = nm_store::rusqlite::Connection::open(dir.join("nicemailer.db")).unwrap();
    db.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap()
}

/// Between `prepare_update` and `cancel_update`, no client but the app is
/// served: a connection open before is told why and closed, a new one is
/// refused at its `hello`, and every request of the app but the update's
/// is refused. Afterwards, everyone is served again.
#[test]
fn no_client_is_served_while_an_update_is_prepared() {
    use nm_api::MaintenanceReason::Update;
    let dir = tempfile::tempdir().unwrap();
    let _core = start(dir.path());
    let mut app = Client::connect_as(dir.path(), "app");
    let mut cli = Client::connect(dir.path());
    cli.call("ping", json!({})).unwrap();

    let r = app.call("prepare_update", json!({})).unwrap();
    assert_eq!(r["timeout_secs"], 900);
    // The connection of the CLI: the error, without an id, then the end.
    let line = cli.try_line().expect("the reason of the close");
    assert_eq!(line["id"], Value::Null);
    assert_eq!(reason(&line["error"]), Update);
    assert!(cli.try_line().is_none(), "closed");
    // A new client: refused at its handshake.
    let e = Client::try_connect_as(dir.path(), "cli").err().expect("refused");
    let m = maintenance(&e);
    assert_eq!(m.reason, Update);
    assert!(m.since > 0);
    assert!(e["message"].as_str().unwrap().contains("being updated"), "{e}");
    // The app stays connected, but is served nothing else.
    assert_eq!(reason(&app.call("list_folders", json!({})).unwrap_err()), Update);
    // A new connection of the app (relaunched after a crash): let in, and
    // told at `subscribe`.
    let mut again = Client::connect_as(dir.path(), "app");
    assert_eq!(reason(&again.call("subscribe", json!({})).unwrap_err()), Update);
    // Prepared again: the timer starts over, nothing else changes.
    again.call("prepare_update", json!({})).unwrap();

    assert_eq!(again.call("cancel_update", json!({})).unwrap()["cancelled"], true);
    let mut cli = Client::connect(dir.path());
    cli.call("list_folders", json!({})).unwrap();
    app.call("list_folders", json!({})).unwrap();
    // Nothing more to cancel.
    assert_eq!(app.call("cancel_update", json!({})).unwrap()["cancelled"], false);
}

/// Only the app updates NiceMailer: the CLI cannot lock everyone out.
#[test]
fn only_the_app_may_prepare_an_update() {
    let dir = tempfile::tempdir().unwrap();
    let _core = start(dir.path());
    let mut cli = Client::connect(dir.path());
    for m in ["prepare_update", "cancel_update"] {
        assert_eq!(cli.call(m, json!({})).unwrap_err()["code"], nm_api::error_code::FORBIDDEN, "{m}");
    }
    cli.call("list_folders", json!({})).unwrap();
}

/// An update that never comes (the app crashed before Sparkle replaced
/// it): after `core.update_timeout_secs`, the core serves its clients
/// again by itself.
#[test]
fn an_abandoned_update_ends_after_its_timeout() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.toml"), "[core]\nupdate_timeout_secs = 1\n").unwrap();
    let _core = start(dir.path());
    let mut app = Client::connect_as(dir.path(), "app");
    assert_eq!(app.call("prepare_update", json!({})).unwrap()["timeout_secs"], 1);
    assert!(Client::try_connect_as(dir.path(), "cli").is_err());
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match Client::try_connect_as(dir.path(), "cli") {
            Ok(mut c) => break c.call("list_folders", json!({})).unwrap(),
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => panic!("still in maintenance: {e}"),
        };
    }
    .to_string();
    app.call("list_folders", json!({})).unwrap();
}

/// The update installed: Sparkle replaced the core's program. The core
/// sees it, closes cleanly and leaves with code 75, which launchd restarts
/// (`KeepAlive`), so that the new program runs. A new core then serves the
/// clients, whose subscription gets `ResyncRequired`: the epoch changed.
#[test]
fn a_core_whose_program_is_replaced_leaves_for_launchd_to_restart_it() {
    let dir = tempfile::tempdir().unwrap();
    let program = dir.path().join("program");
    std::fs::write(&program, "old").unwrap();
    let env = [("NICEMAILER_TEST_UPDATE_PROGRAM", program.to_str().unwrap())];
    let mut core = start_with(dir.path(), &env);
    let mut app = Client::connect_as(dir.path(), "app");
    let sub = app.call("subscribe", json!({})).unwrap();
    app.call("prepare_update", json!({})).unwrap();
    // Not replaced yet: the core waits.
    std::thread::sleep(Duration::from_millis(800));
    assert!(Client::try_connect_as(dir.path(), "cli").is_err());
    // Replaced as Sparkle does: a new file moved into place.
    let new = dir.path().join("program.new");
    std::fs::write(&new, "new").unwrap();
    std::fs::rename(&new, &program).unwrap();
    assert_eq!(core.exit_status(10).code(), Some(75));
    assert!(dir.path().join(nm_store::clean_close::FILE).exists(), "closed cleanly");

    // launchd starts the new core; the app subscribes from where it was.
    let _core = start(dir.path());
    let mut app = Client::connect_until_served(dir.path(), "app");
    let since = json!({"epoch": sub["epoch"], "seq": sub["seq"]});
    let r = app.call("subscribe", json!({"since": since})).unwrap();
    assert_eq!(r["replayed"], false);
    assert_ne!(r["epoch"], sub["epoch"]);
    let e = app.next_event();
    assert_eq!(e["method"], "ResyncRequired", "{e}");
}

/// A client that connects while the database is migrated gets the error
/// `MAINTENANCE` with the versions, and starts no core (the socket is
/// there). The migration is made slow by a write lock on the body file,
/// held by the test; a SIGTERM then stops the core with code 0, the
/// database left at its version, and the next start migrates it.
#[test]
fn a_client_is_told_of_a_migration_and_a_stop_cancels_it_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("nicemailer.db");
    nm_store::rusqlite::Connection::open(&db)
        .unwrap()
        .execute_batch(include_str!("../../nm-store/tests/fixtures/schema_v1.sql"))
        .unwrap();
    let blocker = nm_store::rusqlite::Connection::open(nm_store::body_path(&db)).unwrap();
    blocker.execute_batch("PRAGMA journal_mode = WAL; CREATE TABLE hold (x); BEGIN IMMEDIATE; INSERT INTO hold VALUES (1);").unwrap();
    let mut core = start(dir.path());
    let e = Client::try_connect_as(dir.path(), "app").err().expect("refused during the migration");
    let m = maintenance(&e);
    assert_eq!(m.reason, nm_api::MaintenanceReason::Migration);
    assert_eq!((m.from, m.to, m.at), (Some(1), Some(nm_store::SCHEMA_VERSION), Some(1)));
    assert!(m.is_transient());
    // The backup was made before anything changed.
    assert!(dir.path().join("nicemailer.db.bak-v1").exists());

    unsafe { libc::kill(core.pid() as i32, libc::SIGTERM) };
    assert!(core.exit_status(20).success());
    drop(blocker);
    assert_eq!(version(dir.path()), 1);
    let _core = start(dir.path());
    Client::connect_until_served(dir.path(), "cli").call("list_folders", json!({})).unwrap();
    assert_eq!(version(dir.path()), nm_store::SCHEMA_VERSION);
    // Kept until the next migration.
    assert!(dir.path().join("nicemailer.db.bak-v1").exists());
}

/// A migration that fails halfway: the database is put back as version 1
/// left it, and the core stays in maintenance (`migration_failed`, with
/// the error) instead of failing over and over under launchd. A stop
/// leaves with code 0.
#[test]
fn a_failed_migration_puts_the_database_back_and_stays_in_maintenance() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("nicemailer.db");
    let conn = nm_store::rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(include_str!("../../nm-store/tests/fixtures/schema_v1.sql")).unwrap();
    // Version 13 creates `ai_run`: already there, it fails.
    conn.execute_batch("CREATE TABLE ai_run (x);").unwrap();
    drop(conn);
    let mut core = start(dir.path());
    let deadline = Instant::now() + Duration::from_secs(20);
    let m = loop {
        let e = Client::try_connect_as(dir.path(), "app").err().expect("never served");
        let m = maintenance(&e);
        if m.reason == nm_api::MaintenanceReason::MigrationFailed || Instant::now() > deadline {
            break m;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    assert_eq!(m.reason, nm_api::MaintenanceReason::MigrationFailed);
    assert!(!m.is_transient());
    assert!(m.message.as_deref().unwrap().contains("ai_run"), "{m:?}");
    assert_eq!(version(dir.path()), 1);
    assert!(!dir.path().join("nicemailer.db.bak-v1").exists(), "the backup became the database");
    unsafe { libc::kill(core.pid() as i32, libc::SIGTERM) };
    assert!(core.exit_status(10).success());
}

/// A database written by a newer NiceMailer: never written, the core in
/// maintenance (`newer_schema`) until it is stopped, with code 0.
#[test]
fn a_newer_database_is_never_written() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("nicemailer.db");
    nm_store::rusqlite::Connection::open(&db).unwrap().execute_batch("PRAGMA user_version = 99;").unwrap();
    let before = std::fs::read(&db).unwrap();
    let mut core = start(dir.path());
    let e = Client::try_connect_as(dir.path(), "cli").err().expect("refused");
    let m = maintenance(&e);
    assert_eq!(m.reason, nm_api::MaintenanceReason::NewerSchema);
    assert_eq!((m.from, m.to), (Some(99), Some(nm_store::SCHEMA_VERSION)));
    assert!(e["message"].as_str().unwrap().contains("newer NiceMailer"), "{e}");
    // Still there, still refusing, a moment later.
    std::thread::sleep(Duration::from_millis(300));
    assert!(core_runs(&mut core));
    assert!(Client::try_connect_as(dir.path(), "app").is_err());
    assert_eq!(std::fs::read(&db).unwrap(), before, "not a byte written");
    assert!(!dir.path().join("nicemailer_body.db").exists(), "nothing created next to it");
    unsafe { libc::kill(core.pid() as i32, libc::SIGTERM) };
    assert!(core.exit_status(10).success());
}

/// Whether the core still runs.
fn core_runs(core: &mut common::Core) -> bool {
    unsafe { libc::kill(core.pid() as i32, 0) == 0 }
}
