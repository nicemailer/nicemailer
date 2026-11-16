//! The CLI facing a core in maintenance (`mise-a-jour.md`, section 5): a
//! clear message and the exit status 75, or with `--wait`, a wait until the
//! core serves again. The core is played by a thread of the test, which
//! answers as the real one does (`nm-daemon`, `update.rs`).

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use nm_api::{Maintenance, MaintenanceReason};
use serde_json::{Value, json};

/// A fake core on `dir`: the first `refusals` connections get `refusal` to
/// their `hello`, the next ones are served (`hello`, an empty
/// `list_folders`). Returns the count of connections.
fn fake_core(dir: &Path, refusals: usize, refusal: Maintenance) -> Arc<AtomicUsize> {
    let listener = UnixListener::bind(dir.join("core.sock")).unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let seen = count.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let n = seen.fetch_add(1, Ordering::SeqCst);
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap_or(0) > 0 {
                let req: Value = serde_json::from_str(&line).unwrap();
                line.clear();
                let answer = if n < refusals {
                    json!({"jsonrpc": "2.0", "id": req["id"], "error": refusal.error()})
                } else if req["method"] == "hello" {
                    json!({"jsonrpc": "2.0", "id": req["id"], "result": {"protocol_version": 1, "core_version": "0"}})
                } else {
                    json!({"jsonrpc": "2.0", "id": req["id"], "result": {"items": [], "as_of": {"epoch": "e", "seq": 0}}})
                };
                writeln!(stream, "{answer}").unwrap();
                if n < refusals {
                    break;
                }
            }
        }
    });
    count
}

fn maintenance(reason: MaintenanceReason) -> Maintenance {
    Maintenance { reason, from: Some(9), to: Some(13), at: Some(11), since: 1, message: Some("disk full".into()) }
}

fn cli(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_nicemailer")).arg("--data-dir").arg(dir).args(args).output().unwrap()
}

#[test]
fn a_core_in_maintenance_gives_a_message_and_status_75() {
    for (reason, said) in [
        (MaintenanceReason::Update, "NiceMailer is being updated; try again in a moment, or add --wait"),
        (MaintenanceReason::Migration, "upgrading its database (data format 9 to 13, now at 11)"),
        (MaintenanceReason::NewerSchema, "written by a newer NiceMailer (data format 9, this one reads 13)"),
        (MaintenanceReason::MigrationFailed, "failed, and the database was put back as it was: disk full"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        fake_core(dir.path(), usize::MAX, maintenance(reason));
        let out = cli(dir.path(), &["folders"]);
        let err = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(75), "{reason:?}: {err}");
        assert!(err.contains(said), "{reason:?}: {err}");
    }
}

#[test]
fn with_wait_an_update_is_waited_for() {
    let dir = tempfile::tempdir().unwrap();
    let count = fake_core(dir.path(), 2, maintenance(MaintenanceReason::Update));
    let out = cli(dir.path(), &["folders", "--wait"]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{err}");
    assert_eq!(err.matches("Waiting: NiceMailer is being updated").count(), 1, "said once: {err}");
    assert_eq!(count.load(Ordering::SeqCst), 3, "two refusals, then served");
}

#[test]
fn with_wait_a_maintenance_only_the_user_can_end_fails_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let count = fake_core(dir.path(), usize::MAX, maintenance(MaintenanceReason::NewerSchema));
    let out = cli(dir.path(), &["folders", "--wait"]);
    assert_eq!(out.status.code(), Some(75));
    assert_eq!(count.load(Ordering::SeqCst), 1, "not tried again");
}

#[test]
fn other_errors_keep_status_1() {
    let dir = tempfile::tempdir().unwrap();
    let out = cli(dir.path(), &["folders"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("is the core running?"));
}
