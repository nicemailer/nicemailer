//! `nicemailer core` as a real process. Only what stops before launchd is
//! asked anything is played here: a test must never stop the core of the
//! machine it runs on. The stop and the start themselves are tested against a
//! launchd played by the test (`src/core_agent.rs`).

use std::process::Command;

#[test]
fn another_mailbox_is_refused_before_launchd_is_asked() {
    let dir = tempfile::tempdir().unwrap();
    for action in ["stop", "start"] {
        let out = Command::new(env!("CARGO_BIN_EXE_nicemailer"))
            .arg("--data-dir")
            .arg(dir.path())
            .args(["core", action])
            .output()
            .unwrap();
        let err = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(1), "{action}: {err}");
        assert!(err.contains("is not the mailbox of the app's core"), "{action}: {err}");
    }
}

#[test]
fn the_help_names_both_commands() {
    let out = Command::new(env!("CARGO_BIN_EXE_nicemailer")).args(["core", "--help"]).output().unwrap();
    let help = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success());
    assert!(help.contains("stop") && help.contains("start"), "{help}");
}
