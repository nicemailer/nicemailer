//! End to end: the core creates `config.toml` and `accounts.toml` when
//! they are missing, reads them, watches them, and reports the accounts and
//! the state of the files (`configuration.md`, section 12).
//!
//! The core uses a throwaway keychain file of the test (`common::start`).

mod common;

use std::time::Duration;

use common::{Client, start, write_config};
use serde_json::{Value, json};

const ACCOUNT: &str = "[accounts.perso]\nemail = \"me@example.com\"\ndisplay_name = \"Me\"\n\
                       [accounts.perso.imap]\nhost = \"127.0.0.1\"\nport = 31993\n";

fn accounts(c: &mut Client) -> Vec<Value> {
    c.call("list_accounts", json!({})).unwrap()["items"].as_array().unwrap().clone()
}

/// Writes `file` (`config.toml` or `accounts.toml`) as editors do: a
/// temporary file renamed over it. The events received before are
/// forgotten, so the test waits for those of this save.
fn save(c: &mut Client, dir: &std::path::Path, file: &str, text: &str) {
    c.clear_events();
    let tmp = dir.join(format!(".{file}.tmp"));
    std::fs::write(&tmp, text).unwrap();
    std::fs::rename(&tmp, dir.join(file)).unwrap();
}

#[test]
fn missing_files_are_created_at_start_and_change_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let _core = start(dir.path());
    let mut c = Client::connect(dir.path());
    assert!(accounts(&mut c).is_empty());
    let status = c.call("config_status", json!({})).unwrap();
    assert_eq!(status["exists"], true, "created");
    assert_eq!(status["accounts_exists"], true, "created");
    assert!(status["error"].is_null());
    assert_eq!(status["warnings"], json!([]));
    assert!(status["path"].as_str().unwrap().ends_with("/config.toml"));
    assert!(status["accounts_path"].as_str().unwrap().ends_with("/accounts.toml"));
    // Every setting listed, commented out: the defaults apply. The
    // window's title bar: the theme's (`themes.md`, section 9), and the
    // theme clean, the default of a new install.
    assert!(status["ui"]["title_bar"].is_null());
    assert_eq!(status["ui"]["theme"], "clean");
    // The app does not confirm calls for the user unless asked.
    assert_eq!(status["ui"]["auto_confirm_calls"], false);
    // The text sizes of the app and of its status bar: 16 and 18 pt.
    assert_eq!(status["ui"]["text_size"], 16);
    assert_eq!(status["ui"]["status_bar_text_size"], 18);
    // The folders keep their English names.
    assert_eq!(status["folder_names"]["waiting"], "Waiting");
    assert_eq!(status["folder_names"]["all_mail"], "All Mail");
    let config = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
    assert_eq!(config, nm_engine::config::config_template());
    assert!(config.contains("# poll_interval_secs = 300"), "{config}");
    let accounts = std::fs::read_to_string(dir.path().join("accounts.toml")).unwrap();
    assert_eq!(accounts, nm_engine::config::accounts_template());
}

#[test]
fn existing_files_are_kept_as_they_are() {
    let dir = tempfile::tempdir().unwrap();
    // An empty config.toml is the user's choice; only accounts.toml is
    // missing.
    write_config(dir.path(), "");
    let _core = start(dir.path());
    let mut c = Client::connect(dir.path());
    c.call("config_status", json!({})).unwrap();
    assert_eq!(std::fs::read_to_string(dir.path().join("config.toml")).unwrap(), "");
    assert_eq!(std::fs::read_to_string(dir.path().join("accounts.toml")).unwrap(), nm_engine::config::accounts_template());
}

/// An update from a version before the themes (`themes.md`, "The default
/// theme"): `config.toml` there, `themes/` not. The core writes geek, the
/// look the user had, into the file, keeping the rest; and only once.
#[test]
fn an_install_from_before_the_themes_keeps_geek() {
    let dir = tempfile::tempdir().unwrap();
    let mine = "# my settings\n[ui]\ntext_size = 14\n";
    std::fs::write(dir.path().join("config.toml"), mine).unwrap();
    {
        let _core = start(dir.path());
        let mut c = Client::connect(dir.path());
        assert_eq!(c.call("config_status", json!({})).unwrap()["ui"]["theme"], "geek");
        assert_eq!(c.call("theme_get", json!({})).unwrap()["name"], "geek");
        let text = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert_eq!(text, format!("{mine}theme = \"geek\"\n"), "the rest of the file is kept");
        assert!(dir.path().join("themes/clean.toml").exists(), "themes/ created after");
    }
    // The user removes the line to get the default: the next start keeps it so.
    std::fs::write(dir.path().join("config.toml"), mine).unwrap();
    let _core = start(dir.path());
    let mut c = Client::connect(dir.path());
    assert_eq!(c.call("config_status", json!({})).unwrap()["ui"]["theme"], "clean");
    assert_eq!(std::fs::read_to_string(dir.path().join("config.toml")).unwrap(), mine);
}

#[test]
fn the_title_bar_setting_reaches_the_app_and_follows_the_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.toml"), "[ui]\ntitle_bar = \"hidden\"\n").unwrap();
    let _core = start(dir.path());
    let mut c = Client::connect(dir.path());
    let status = c.call("config_status", json!({})).unwrap();
    assert_eq!(status["ui"]["title_bar"], "hidden");
    // Still applied while written, with a warning: the theme says it now.
    assert_eq!(status["warnings"][0]["key"], "ui.title_bar");
    c.call("subscribe", json!({})).unwrap();
    std::thread::sleep(Duration::from_millis(500));
    save(&mut c, dir.path(), "config.toml", "[ui]\ntitle_bar = \"shown\"\n");
    let e = c.event("ConfigChanged", 15);
    assert!(e["keys"].as_array().unwrap().contains(&json!("ui.title_bar")), "{e}");
    assert_eq!(c.call("config_status", json!({})).unwrap()["ui"]["title_bar"], "shown");
}

#[test]
fn the_call_confirmation_setting_reaches_the_app_and_follows_the_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.toml"), "[ui]\nauto_confirm_calls = true\n").unwrap();
    let _core = start(dir.path());
    let mut c = Client::connect(dir.path());
    assert_eq!(c.call("config_status", json!({})).unwrap()["ui"]["auto_confirm_calls"], true);
    c.call("subscribe", json!({})).unwrap();
    std::thread::sleep(Duration::from_millis(500));
    save(&mut c, dir.path(), "config.toml", "[ui]\nauto_confirm_calls = false\n");
    let e = c.event("ConfigChanged", 15);
    assert!(e["keys"].as_array().unwrap().contains(&json!("ui.auto_confirm_calls")), "{e}");
    assert_eq!(c.call("config_status", json!({})).unwrap()["ui"]["auto_confirm_calls"], false);
}

/// The app applies the text sizes at once (`docs/geek-ui.md`, "Text
/// size"): `config_status` gives them, and follows an edit of the file.
#[test]
fn the_text_sizes_reach_the_app_and_follow_the_file() {
    let dir = tempfile::tempdir().unwrap();
    write_config(dir.path(), "[ui]\ntext_size = 18\nstatus_bar_text_size = 12\n");
    let _core = start(dir.path());
    let mut c = Client::connect(dir.path());
    let ui = &c.call("config_status", json!({})).unwrap()["ui"];
    assert_eq!((ui["text_size"].clone(), ui["status_bar_text_size"].clone()), (json!(18), json!(12)));
    c.call("subscribe", json!({})).unwrap();
    std::thread::sleep(Duration::from_millis(500));
    save(&mut c, dir.path(), "config.toml", "[ui]\ntext_size = 16\nstatus_bar_text_size = 12\n");
    let e = c.event("ConfigChanged", 15);
    assert_eq!(e["keys"], json!(["ui.text_size"]), "{e}");
    assert_eq!(c.call("config_status", json!({})).unwrap()["ui"]["text_size"], 16);
}

/// The app shows the names of `[folder_names]` (`configuration.md`, "Noms
/// des dossiers"): `config_status` gives every role, English unless named,
/// and follows an edit of the file; `list_folders` keeps the English names,
/// which the CLI and the MCP agent show.
#[test]
fn the_folder_names_reach_the_app_and_follow_the_file() {
    let dir = tempfile::tempdir().unwrap();
    write_config(dir.path(), "[folder_names]\nwaiting = \"En attente\"\n");
    let _core = start(dir.path());
    let mut c = Client::connect(dir.path());
    let names = c.call("config_status", json!({})).unwrap()["folder_names"].clone();
    assert_eq!(names.as_object().unwrap().len(), 15, "the 12 roles and the 3 virtual folders: {names}");
    assert_eq!((names["waiting"].clone(), names["inbox"].clone()), (json!("En attente"), json!("Inbox")));
    c.call("subscribe", json!({})).unwrap();
    std::thread::sleep(Duration::from_millis(500));
    save(&mut c, dir.path(), "config.toml", "[folder_names]\nwaiting = \"En attente\"\ninbox = \"Réception\"\n");
    let e = c.event("ConfigChanged", 15);
    assert_eq!(e["keys"], json!(["folder_names.inbox"]), "{e}");
    assert_eq!(c.call("config_status", json!({})).unwrap()["folder_names"]["inbox"], "Réception");
    // A clash refuses the file and keeps the names in effect.
    save(&mut c, dir.path(), "config.toml", "[folder_names]\nwaiting = \"Inbox\"\n");
    let e = c.event("ConfigError", 15);
    assert_eq!(e["key"], "folder_names.waiting", "{e}");
    let status = c.call("config_status", json!({})).unwrap();
    assert_eq!((status["folder_names"]["inbox"].clone(), status["folder_names"]["waiting"].clone()), (json!("Réception"), json!("En attente")));
}

#[test]
fn the_unsubscribe_in_banned_setting_reaches_the_app_and_follows_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let _core = start(dir.path());
    let mut c = Client::connect(dir.path());
    assert_eq!(c.call("config_status", json!({})).unwrap()["unsubscribe"]["in_banned"], false, "off by default");
    c.call("subscribe", json!({})).unwrap();
    std::thread::sleep(Duration::from_millis(500));
    save(&mut c, dir.path(), "config.toml", "[unsubscribe]\nin_banned = true\n");
    let e = c.event("ConfigChanged", 15);
    assert!(e["keys"].as_array().unwrap().contains(&json!("unsubscribe.in_banned")), "{e}");
    assert_eq!(c.call("config_status", json!({})).unwrap()["unsubscribe"]["in_banned"], true);
}

#[test]
fn accounts_and_changes_of_the_file_are_reported() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("accounts.toml"), ACCOUNT).unwrap();
    let _core = start(dir.path());
    let mut c = Client::connect(dir.path());

    let list = accounts(&mut c);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["key"], "perso");
    assert_eq!(list[0]["email"], "me@example.com");
    assert_eq!(list[0]["display_name"], "Me");
    assert_eq!(list[0]["enabled"], true);
    assert!(list[0]["error"].as_str().unwrap().contains("nicemailer account password perso"), "{}", list[0]);
    let id = list[0]["id"].as_i64().unwrap();

    c.call("subscribe", json!({})).unwrap();
    // FSEvents may need a moment before it reports the first change.
    std::thread::sleep(Duration::from_millis(500));

    // An invalid file: refused, with its file and line; the account stays.
    save(&mut c, dir.path(), "config.toml", "[sync]\npoll_interval_secs = \"soon\"\n");
    let e = c.event("ConfigError", 15);
    assert_eq!(e["file"], "config.toml");
    assert_eq!(e["line"], 2);
    assert_eq!(e["key"], "sync.poll_interval_secs");
    let status = c.call("config_status", json!({})).unwrap();
    assert_eq!(status["error"]["file"], "config.toml");
    assert_eq!(status["error"]["line"], 2);
    assert_eq!(accounts(&mut c)[0]["enabled"], true);

    // Fixed: applied.
    save(&mut c, dir.path(), "config.toml", "[sync]\npoll_interval_secs = 600\n");
    let e = c.event("ConfigChanged", 15);
    assert!(e["keys"].as_array().unwrap().contains(&json!("sync.poll_interval_secs")), "{e}");
    assert!(c.call("config_status", json!({})).unwrap()["error"].is_null());

    // The account removed: disabled, not deleted.
    save(&mut c, dir.path(), "accounts.toml", "");
    let e = c.event("AccountChanged", 15);
    assert_eq!(e["account_id"], id);
    let list = accounts(&mut c);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["enabled"], false);

    // And back, with the same id.
    save(&mut c, dir.path(), "accounts.toml", ACCOUNT);
    c.event("AccountChanged", 15);
    let list = accounts(&mut c);
    assert_eq!(list[0]["id"], id);
    assert_eq!(list[0]["enabled"], true);

    // Display order: an archive account, created after, goes last; the
    // order can be changed, and is announced.
    let mbox = dir.path().join("old.mbox");
    std::fs::write(&mbox, "From x@example.com Mon Jan  1 00:00:00 2024\nFrom: x@example.com\nSubject: hi\nMessage-ID: <1@x>\n\nhello\n").unwrap();
    c.call("import_mbox", json!({"path": mbox.to_string_lossy(), "account": "old", "address": "old@example.com"})).unwrap();
    c.event("ImportFinished", 15);
    let list = accounts(&mut c);
    let keys: Vec<&str> = list.iter().map(|a| a["key"].as_str().unwrap()).collect();
    assert_eq!(keys, vec!["perso", "old"]);
    let archive = list[1]["id"].as_i64().unwrap();
    c.clear_events();
    c.call("set_account_order", json!({"account_ids": [archive]})).unwrap();
    let e = c.event("AccountChanged", 5);
    assert!(e["account_id"] == archive || e["account_id"] == id, "{e}");
    let list = accounts(&mut c);
    assert_eq!(list[0]["key"], "old");
    assert_eq!(list[0]["position"], 1);
    assert_eq!(list[1]["position"], 2);
    let folders = c.call("list_folders", json!({})).unwrap();
    assert_eq!(folders["items"][0]["account_id"], archive, "the folder list follows the order");
    let err = c.call("set_account_order", json!({"account_ids": [archive, archive]})).unwrap_err();
    assert!(err["message"].as_str().unwrap().contains("given twice"), "{err}");
    assert_eq!(err["data"]["reason"], "invalid_params", "{err}");

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
    std::fs::write(dir.path().join("accounts.toml"), "[accounts.perso]\nemail = 3\n").unwrap();
    let _core = start(dir.path());
    let mut c = Client::connect(dir.path());
    let status = c.call("config_status", json!({})).unwrap();
    assert_eq!(status["error"]["file"], "accounts.toml");
    assert_eq!(status["error"]["line"], 2);
    assert_eq!(status["error"]["key"], "accounts.perso.email");
    assert!(accounts(&mut c).is_empty(), "nothing of an invalid file is applied");
}

#[test]
fn a_password_saved_by_the_cli_goes_to_the_keychain() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("accounts.toml"), ACCOUNT).unwrap();
    let _core = start(dir.path());
    let mut c = Client::connect(dir.path());
    c.call("subscribe", json!({})).unwrap();
    let id = accounts(&mut c)[0]["id"].clone();

    // An agent may not save a password (`configuration.md`, section 6).
    let mut agent = Client::connect_as(dir.path(), "mcp");
    let err = agent.call("set_password", json!({"account": "perso", "password": "x"})).unwrap_err();
    assert_eq!(err["code"], 1007);
    // Nor anyone for an account the file does not have.
    let err = c.call("set_password", json!({"account": "nobody", "password": "x"})).unwrap_err();
    assert_eq!(err["code"], 1001);

    c.clear_events();
    c.call("set_password", json!({"account": "perso", "password": "nicemailer"})).unwrap();
    assert_eq!(c.event("AccountChanged", 5)["account_id"], id);
    assert!(accounts(&mut c)[0]["error"].is_null());

    // In the keychain of the core, for the server of the file.
    let out = std::process::Command::new("/usr/bin/security")
        .args(["find-generic-password", "-s", "NiceMailer", "-a", "imap:perso"])
        .arg(dir.path().join("test.keychain-db"))
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("127.0.0.1"));

    // The file points the account at another server: refused until the
    // password is saved again.
    c.clear_events();
    save(&mut c, dir.path(), "accounts.toml", &ACCOUNT.replace("127.0.0.1", "imap.example.com"));
    // The account's worker may report its own errors meanwhile (it tries
    // to connect once a password is saved, and nothing answers there):
    // wait for the one of the binding.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        let e = c.event("AccountError", 15);
        if e["error"].as_str().unwrap().contains("save the password again") {
            assert_eq!(e["why"]["reason"], "password_server_changed", "{e}");
            break;
        }
        assert!(std::time::Instant::now() < deadline, "no binding error, last: {e}");
    }
    c.call("set_password", json!({"account": "perso", "password": "nicemailer"})).unwrap();
    assert!(accounts(&mut c)[0]["error"].is_null());

    // Traced, without the password.
    let history = c.call("list_history", json!({"limit": 50})).unwrap().to_string();
    assert!(history.contains("set_password"), "{history}");
    assert!(!history.contains("nicemailer\""), "the password is not in the history");
}
