//! What the core does when started by the agent of NiceMailer.app
//! (`docs/app-bundle.md`): a second core on the same directory leaves with
//! code 0, so that launchd does not start it again and again, and the core
//! writes its messages in `core.log` of its data directory.

mod common;

use std::process::Command;
use std::time::{Duration, Instant};

use common::{Client, start_with};

#[test]
fn a_second_core_leaves_quietly_and_the_log_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let _first = start_with(dir.path(), &[("NICEMAILER_LOG_IN_DATA_DIR", "1")]);
    let _c = Client::connect(dir.path());
    // The second one, on the same directory: code 0, at once.
    let started = Instant::now();
    let status = Command::new(env!("CARGO_BIN_EXE_nicemailer-core"))
        .arg("--data-dir")
        .arg(dir.path())
        .arg("--config")
        .arg(dir.path().join("config.toml"))
        .arg("--keychain")
        .arg(dir.path().join("test.keychain-db"))
        .status()
        .unwrap();
    assert!(status.success(), "{status}");
    assert!(started.elapsed() < Duration::from_secs(10));
    // The first one logs to its data directory.
    let log = std::fs::read_to_string(dir.path().join("core.log")).unwrap();
    assert!(log.contains("--- nicemailer-core"), "{log}");
    assert!(log.contains("socket"), "{log}");
}
