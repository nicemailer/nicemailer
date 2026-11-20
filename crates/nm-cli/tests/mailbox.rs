//! `nicemailer mailbox check | upgrade | backup` (`docs/migrations.md`,
//! `docs/backup.md`): each works on the standard data directory by
//! default, on the folder `--data-dir` names when given, and never takes
//! the folder as a positional argument.
//!
//! Each test gives the CLI its own `HOME`, so the "standard" data directory
//! (`~/Library/Application Support/NiceMailer`) is a temporary folder and
//! never the user's real mailbox.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

use nm_store::rusqlite::Connection;

/// The standard data directory under `home`, as `paths::default_data_dir`
/// builds it.
fn standard_dir(home: &Path) -> PathBuf {
    home.join("Library/Application Support/NiceMailer")
}

/// A small `nicemailer.db` in `dir` holding one row that says `label`, so a
/// test can tell which database a copy came from.
fn make_db(dir: &Path, label: &str) {
    std::fs::create_dir_all(dir).unwrap();
    let conn = Connection::open(dir.join("nicemailer.db")).unwrap();
    conn.execute_batch("PRAGMA journal_mode = WAL; CREATE TABLE t (label TEXT);").unwrap();
    conn.execute("INSERT INTO t VALUES (?1)", [label]).unwrap();
}

fn label_of(db: &Path) -> String {
    Connection::open(db).unwrap().query_row("SELECT label FROM t", [], |r| r.get(0)).unwrap()
}

/// Runs the CLI with `home` as its `HOME` and no `NICEMAILER_DATA_DIR`
/// (which would otherwise replace the standard folder).
fn cli(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_nicemailer"))
        .env("HOME", home)
        .env_remove("NICEMAILER_DATA_DIR")
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn without_data_dir_the_standard_folder_is_backed_up() {
    let home = tempfile::tempdir().unwrap();
    make_db(&standard_dir(home.path()), "standard");
    let dest = home.path().join("backup.db");

    let out = cli(home.path(), &["mailbox", "backup", dest.to_str().unwrap()]);

    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(label_of(&dest), "standard");
}

#[test]
fn data_dir_names_another_folder() {
    let home = tempfile::tempdir().unwrap();
    make_db(&standard_dir(home.path()), "standard");
    let other = home.path().join("elsewhere");
    make_db(&other, "elsewhere");
    let dest = home.path().join("backup.db");

    let out = cli(home.path(), &["mailbox", "backup", "--data-dir", other.to_str().unwrap(), dest.to_str().unwrap()]);

    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(label_of(&dest), "elsewhere");
}

#[test]
fn an_existing_destination_is_refused_and_kept() {
    let home = tempfile::tempdir().unwrap();
    make_db(&standard_dir(home.path()), "standard");
    let dest = home.path().join("backup.db");
    std::fs::write(&dest, "keep me").unwrap();

    let out = cli(home.path(), &["mailbox", "backup", dest.to_str().unwrap()]);

    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("already exists"));
    assert_eq!(std::fs::read_to_string(&dest).unwrap(), "keep me");
}

#[test]
fn a_folder_without_database_is_refused() {
    let home = tempfile::tempdir().unwrap();
    let dest = home.path().join("backup.db");

    let out = cli(home.path(), &["mailbox", "backup", dest.to_str().unwrap()]);

    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("no database in this folder"));
    assert!(!dest.exists());
}

/// The old form of `backup`, with the folder before DEST, no longer parses: clap
/// rejects the extra argument instead of guessing which one is which.
#[test]
fn the_folder_is_no_longer_a_positional_argument() {
    let home = tempfile::tempdir().unwrap();
    make_db(&standard_dir(home.path()), "standard");
    let dest = home.path().join("backup.db");

    let out = cli(home.path(), &["mailbox", "backup", standard_dir(home.path()).to_str().unwrap(), dest.to_str().unwrap()]);

    assert!(!out.status.success());
    assert!(!dest.exists());
}

/// The JSON `check --json` prints, parsed.
fn check_json(out: &Output) -> Value {
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn check_without_data_dir_reads_the_standard_folder() {
    let home = tempfile::tempdir().unwrap();
    let standard = standard_dir(home.path());
    std::fs::create_dir_all(&standard).unwrap();

    let report = check_json(&cli(home.path(), &["mailbox", "check", "--json"]));

    assert_eq!(report["path"], standard.display().to_string());
    assert_eq!(report["state"], "new");
}

#[test]
fn check_with_data_dir_reads_that_folder() {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(standard_dir(home.path())).unwrap();
    let other = home.path().join("elsewhere");
    make_db(&other, "elsewhere");

    let report = check_json(&cli(home.path(), &["mailbox", "check", "--data-dir", other.to_str().unwrap(), "--json"]));

    assert_eq!(report["path"], other.display().to_string());
    // A table of our own and no NiceMailer schema: not the "new" of the
    // empty standard folder, which shows the other folder was read.
    assert_ne!(report["state"], "new");
}

#[test]
fn check_no_longer_takes_the_folder_as_an_argument() {
    let home = tempfile::tempdir().unwrap();
    let out = cli(home.path(), &["mailbox", "check", home.path().to_str().unwrap()]);
    assert!(!out.status.success());
}

#[test]
fn upgrade_works_on_the_standard_folder_or_the_data_dir() {
    let home = tempfile::tempdir().unwrap();
    let standard = standard_dir(home.path());
    std::fs::create_dir_all(&standard).unwrap();
    let other = home.path().join("elsewhere");
    std::fs::create_dir_all(&other).unwrap();

    // Neither folder has a database: the error names the folder worked on.
    let out = cli(home.path(), &["mailbox", "upgrade"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains(&format!("{}: no database to upgrade", standard.display())));

    let out = cli(home.path(), &["mailbox", "upgrade", "--data-dir", other.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains(&format!("{}: no database to upgrade", other.display())));
}

#[test]
fn upgrade_no_longer_takes_the_folder_as_an_argument() {
    let home = tempfile::tempdir().unwrap();
    let out = cli(home.path(), &["mailbox", "upgrade", home.path().to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("unexpected argument"));
}
