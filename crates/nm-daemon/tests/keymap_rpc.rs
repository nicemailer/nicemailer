//! End to end: the core creates `keymap.toml` when it is missing, sends the
//! keys with `keymap_get`, follows the file, and refuses an invalid one on
//! its own, without touching `config.toml` (`raccourcis.md`, section 11).

mod common;

use std::time::Duration;

use common::{Client, start};
use serde_json::{Value, json};

/// Writes `file` as editors do: a temporary file renamed over it. The
/// events received before are forgotten.
fn save(c: &mut Client, dir: &std::path::Path, file: &str, text: &str) {
    c.clear_events();
    let tmp = dir.join(format!(".{file}.tmp"));
    std::fs::write(&tmp, text).unwrap();
    std::fs::rename(&tmp, dir.join(file)).unwrap();
}

/// The action of `key` in `context` of a `keymap_get` result, if listed.
fn action(keymap: &Value, context: &str, key: &str) -> Option<String> {
    keymap["contexts"][context]
        .as_array()?
        .iter()
        .find(|b| b["key"] == key)
        .map(|b| b["action"].as_str().unwrap().to_string())
}

#[test]
fn created_at_start_and_sent_with_the_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let _core = start(dir.path());
    let mut c = Client::connect(dir.path());
    let k = c.call("keymap_get", json!({})).unwrap();
    assert_eq!(k["exists"], true);
    assert!(k["path"].as_str().unwrap().ends_with("/keymap.toml"));
    assert!(k["error"].is_null());
    assert_eq!(k["warnings"], json!([]));
    assert_eq!(action(&k, "mail", "d").as_deref(), Some("mail.archive"));
    assert_eq!(action(&k, "mail", "J").as_deref(), Some("list.mark_down"));
    assert_eq!(k["menus"].as_array().unwrap().len(), 4);
    let text = std::fs::read_to_string(dir.path().join("keymap.toml")).unwrap();
    assert_eq!(text, nm_engine::keymap::template());
}

#[test]
fn a_change_is_applied_and_an_invalid_file_refused_alone() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("keymap.toml"), "[mail]\ne = \"mail.archive\"\n").unwrap();
    let _core = start(dir.path());
    let mut c = Client::connect(dir.path());
    assert_eq!(action(&c.call("keymap_get", json!({})).unwrap(), "mail", "e").as_deref(), Some("mail.archive"));
    c.call("subscribe", json!({})).unwrap();
    // FSEvents may need a moment before it reports the first change.
    std::thread::sleep(Duration::from_millis(500));

    save(&mut c, dir.path(), "keymap.toml", "[mail]\ne = \"mail.archive\"\nd = \"none\"\n");
    c.event("KeymapChanged", 15);
    let k = c.call("keymap_get", json!({})).unwrap();
    assert_eq!(action(&k, "mail", "d").as_deref(), Some("none"));

    // Invalid: refused with its line, the previous keys stay, and
    // config.toml is still applied.
    save(&mut c, dir.path(), "keymap.toml", "[mail]\nesc = \"mail.archive\"\n");
    let e = c.event("ConfigError", 15);
    assert_eq!(e["file"], "keymap.toml");
    assert_eq!(e["line"], 2);
    let k = c.call("keymap_get", json!({})).unwrap();
    assert_eq!(k["error"]["line"], 2);
    assert_eq!(action(&k, "mail", "d").as_deref(), Some("none"));
    assert!(c.call("config_status", json!({})).unwrap()["error"].is_null());

    // Fixed: announced, the error is gone.
    save(&mut c, dir.path(), "keymap.toml", "[mail]\nx = \"mail.archive\"\n");
    c.event("KeymapChanged", 15);
    let k = c.call("keymap_get", json!({})).unwrap();
    assert!(k["error"].is_null());
    assert_eq!(action(&k, "mail", "x").as_deref(), Some("mail.archive"));
    assert_eq!(action(&k, "mail", "d").as_deref(), Some("mail.archive"));
}
