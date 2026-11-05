//! End to end: the core reads `config.toml`, watches it, and reports the
//! accounts and the state of the file (`configuration.md`, section 12).
//!
//! By default the test binary is built without the `dev-passwords`
//! feature, like the shipped core: a password in the file is not read, and
//! the account says so. `cargo test -p nm-daemon --features dev-passwords`
//! checks the other case.

mod common;

use std::time::Duration;

use common::{Client, start};
use serde_json::{Value, json};

const ACCOUNT: &str = "[accounts.perso]\nemail = \"me@example.com\"\ndisplay_name = \"Me\"\n\
                       [accounts.perso.imap]\nhost = \"127.0.0.1\"\nport = 31993\npassword = \"nicemailer\"\n";

fn accounts(c: &mut Client) -> Vec<Value> {
    c.call("list_accounts", json!({})).unwrap()["items"].as_array().unwrap().clone()
}

/// Writes the file as editors do: a temporary file renamed over it.
fn save(dir: &std::path::Path, text: &str) {
    let tmp = dir.join(".config.toml.tmp");
    std::fs::write(&tmp, text).unwrap();
    std::fs::rename(&tmp, dir.join("config.toml")).unwrap();
}

#[test]
fn without_a_file_the_core_starts_with_no_account() {
    let dir = tempfile::tempdir().unwrap();
    let _core = start(dir.path());
    let mut c = Client::connect(dir.path());
    assert!(accounts(&mut c).is_empty());
    let status = c.call("config_status", json!({})).unwrap();
    assert_eq!(status["exists"], false);
    assert!(status["error"].is_null());
    assert!(status["path"].as_str().unwrap().ends_with("config.toml"));
}

#[test]
fn accounts_and_changes_of_the_file_are_reported() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.toml"), ACCOUNT).unwrap();
    let _core = start(dir.path());
    let mut c = Client::connect(dir.path());

    let list = accounts(&mut c);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["key"], "perso");
    assert_eq!(list[0]["email"], "me@example.com");
    assert_eq!(list[0]["display_name"], "Me");
    assert_eq!(list[0]["enabled"], true);
    if cfg!(feature = "dev-passwords") {
        // `cargo test --features dev-passwords`: the password is read.
        assert!(list[0]["error"].is_null(), "{}", list[0]);
    } else {
        assert!(list[0]["error"].as_str().unwrap().contains("dev-passwords"), "{}", list[0]);
    }
    let status = c.call("config_status", json!({})).unwrap();
    assert_eq!(status["warnings"][0]["key"], "accounts.perso.imap.password");
    let id = list[0]["id"].as_i64().unwrap();

    c.call("subscribe", json!({})).unwrap();
    // FSEvents may need a moment before it reports the first change.
    std::thread::sleep(Duration::from_millis(500));

    // An invalid file: refused, with its line; the account stays.
    save(dir.path(), &format!("{ACCOUNT}[sync]\npoll_interval_secs = \"soon\"\n"));
    let e = c.event("ConfigError", 15);
    assert_eq!(e["line"], 9);
    assert_eq!(e["key"], "sync.poll_interval_secs");
    let status = c.call("config_status", json!({})).unwrap();
    assert_eq!(status["error"]["line"], 9);
    assert_eq!(accounts(&mut c)[0]["enabled"], true);

    // Fixed: applied.
    save(dir.path(), &format!("{ACCOUNT}[sync]\npoll_interval_secs = 600\n"));
    let e = c.event("ConfigChanged", 15);
    assert!(e["keys"].as_array().unwrap().contains(&json!("sync.poll_interval_secs")), "{e}");
    assert!(c.call("config_status", json!({})).unwrap()["error"].is_null());

    // The account removed: disabled, not deleted.
    save(dir.path(), "");
    let e = c.event("AccountChanged", 15);
    assert_eq!(e["account_id"], id);
    let list = accounts(&mut c);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["enabled"], false);

    // And back, with the same id.
    save(dir.path(), ACCOUNT);
    c.event("AccountChanged", 15);
    let list = accounts(&mut c);
    assert_eq!(list[0]["id"], id);
    assert_eq!(list[0]["enabled"], true);

    // The history has the reloads and the account's life.
    let history = c.call("list_history", json!({"limit": 50})).unwrap();
    let actions: Vec<String> =
        history["items"].as_array().unwrap().iter().map(|h| format!("{}:{}", h["entity"].as_str().unwrap_or(""), h["action"].as_str().unwrap_or(""))).collect();
    for expected in ["account:create", "config:reload", "account:disable", "account:enable"] {
        assert!(actions.iter().any(|a| a == expected), "{expected} missing from {actions:?}");
    }
}

#[test]
fn an_invalid_file_at_start_is_reported_by_config_status() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.toml"), "[accounts.perso]\nemail = 3\n").unwrap();
    let _core = start(dir.path());
    let mut c = Client::connect(dir.path());
    let status = c.call("config_status", json!({})).unwrap();
    assert_eq!(status["error"]["line"], 2);
    assert_eq!(status["error"]["key"], "accounts.perso.email");
    assert!(accounts(&mut c).is_empty(), "nothing of an invalid file is applied");
}
