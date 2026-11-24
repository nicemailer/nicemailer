//! End to end: the core creates the files of the built-in themes, sends the
//! theme `ui.theme` names with `theme_get`, follows the directory, refuses
//! an invalid file on its own, and writes `ui.theme` for the Preferences
//! (`themes.md`, section 11).

mod common;

use std::path::Path;
use std::time::Duration;

use common::{Client, start};
use serde_json::json;

/// Writes `file` (relative to `dir`) as editors do: a temporary file
/// renamed over it. The events received before are forgotten.
fn save(c: &mut Client, dir: &Path, file: &str, text: &str) {
    c.clear_events();
    let path = dir.join(file);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text).unwrap();
    std::fs::rename(&tmp, &path).unwrap();
}

#[test]
fn the_builtin_themes_are_created_and_geek_is_sent() {
    let dir = tempfile::tempdir().unwrap();
    let _core = start(dir.path());
    let mut c = Client::connect(dir.path());
    for name in ["geek", "clean", "regular", "minimal"] {
        let text = std::fs::read_to_string(dir.path().join(format!("themes/{name}.toml"))).unwrap();
        assert_eq!(text, nm_engine::theme::template(name).unwrap());
    }
    let t = c.call("theme_get", json!({})).unwrap();
    assert_eq!(t["name"], "geek");
    assert_eq!(t["chain"], json!(["geek"]));
    assert!(t["path"].as_str().unwrap().ends_with("/themes"));
    assert_eq!(t["layout"]["row_height"], 20);
    assert_eq!(t["colors"]["background"], json!({"light": "#FAFAF8", "dark": "#1B1D23"}));
    assert!(t["error"].is_null());

    let l = c.call("theme_list", json!({})).unwrap();
    assert_eq!(l["selected"], "geek");
    let names: Vec<_> = l["themes"].as_array().unwrap().iter().map(|t| (t["name"].clone(), t["origin"].clone())).collect();
    // The files created change nothing: the themes are still the built-in
    // ones, not "modified".
    assert_eq!(
        names,
        vec![
            (json!("geek"), json!("builtin")),
            (json!("clean"), json!("builtin")),
            (json!("regular"), json!("builtin")),
            (json!("minimal"), json!("builtin")),
        ]
    );
}

#[test]
fn the_theme_of_config_toml_is_followed() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.toml"), "[ui]\ntheme = \"clean\"\n").unwrap();
    let _core = start(dir.path());
    let mut c = Client::connect(dir.path());
    assert_eq!(c.call("theme_get", json!({})).unwrap()["name"], "clean");
    assert_eq!(c.call("config_status", json!({})).unwrap()["ui"]["theme"], "clean");
    c.call("subscribe", json!({})).unwrap();
    std::thread::sleep(Duration::from_millis(500));

    save(&mut c, dir.path(), "config.toml", "[ui]\ntheme = \"regular\"\n");
    c.event("ThemeChanged", 15);
    let t = c.call("theme_get", json!({})).unwrap();
    assert_eq!(t["window"]["bar_type"], "toolbar");
    assert_eq!(t["colors"]["unread"], json!({"system": "accent"}));

    // A name that names no theme: geek, with a warning; config.toml is
    // still applied (its accounts stay on).
    save(&mut c, dir.path(), "config.toml", "[ui]\ntheme = \"nowhere\"\n");
    c.event("ThemeChanged", 15);
    let t = c.call("theme_get", json!({})).unwrap();
    assert_eq!(t["name"], "geek");
    assert!(t["warnings"].as_array().unwrap().iter().any(|w| w["key"] == "ui.theme"), "{t}");
    assert!(c.call("config_status", json!({})).unwrap()["error"].is_null());
}

#[test]
fn a_theme_file_is_followed_and_refused_alone() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.toml"), "[ui]\ntheme = \"sepia\"\n").unwrap();
    std::fs::create_dir(dir.path().join("themes")).unwrap();
    std::fs::write(dir.path().join("themes/sepia.toml"), "extends = \"clean\"\n[layout]\nzebra = true\n").unwrap();
    let _core = start(dir.path());
    let mut c = Client::connect(dir.path());
    let t = c.call("theme_get", json!({})).unwrap();
    assert_eq!(t["chain"], json!(["sepia", "clean"]));
    assert_eq!(t["layout"]["zebra"], true);
    c.call("subscribe", json!({})).unwrap();
    std::thread::sleep(Duration::from_millis(500));

    save(&mut c, dir.path(), "themes/sepia.toml", "extends = \"clean\"\n[layout]\nrow_height = 40\n");
    c.event("ThemeChanged", 15);
    let t = c.call("theme_get", json!({})).unwrap();
    assert_eq!(t["layout"]["row_height"], 40);
    assert_eq!(t["layout"]["zebra"], false);

    // Invalid: refused with its line, the previous version stays, the
    // other files are still applied.
    save(&mut c, dir.path(), "themes/sepia.toml", "extends = \"clean\"\n[layout]\nrow_height = \"tall\"\n");
    let e = c.event("ConfigError", 15);
    assert_eq!(e["file"], "themes/sepia.toml");
    assert_eq!(e["line"], 3);
    let t = c.call("theme_get", json!({})).unwrap();
    assert_eq!(t["layout"]["row_height"], 40);
    assert_eq!(t["error"]["file"], "themes/sepia.toml");
    assert!(c.call("config_status", json!({})).unwrap()["error"].is_null());
    assert!(c.call("keymap_get", json!({})).unwrap()["error"].is_null());

    // Fixed: announced, the error is gone.
    save(&mut c, dir.path(), "themes/sepia.toml", "extends = \"clean\"\n[layout]\nrow_height = 36\n");
    c.event("ThemeChanged", 15);
    let t = c.call("theme_get", json!({})).unwrap();
    assert!(t["error"].is_null());
    assert_eq!(t["layout"]["row_height"], 36);

    // A theme added to the directory is listed.
    save(&mut c, dir.path(), "themes/amber.toml", "extends = \"regular\"\n");
    std::thread::sleep(Duration::from_millis(1500));
    let l = c.call("theme_list", json!({})).unwrap();
    assert!(l["themes"].as_array().unwrap().iter().any(|t| t["name"] == "amber" && t["parent"] == "regular"), "{l}");
}

#[test]
fn config_set_theme_writes_the_file_and_applies_it() {
    let dir = tempfile::tempdir().unwrap();
    let mine = "# my settings\n[ui]\ntext_size = 14   # my eyes\n";
    std::fs::write(dir.path().join("config.toml"), mine).unwrap();
    let _core = start(dir.path());
    let mut c = Client::connect(dir.path());
    c.call("subscribe", json!({})).unwrap();

    let t = c.call("config_set_theme", json!({"name": "clean"})).unwrap();
    assert_eq!(t["name"], "clean", "the theme now in effect is returned");
    c.event("ThemeChanged", 15);
    let text = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
    assert_eq!(text, format!("{mine}theme = \"clean\"\n"), "the rest of the file is kept");
    assert_eq!(c.call("theme_get", json!({})).unwrap()["name"], "clean");

    // An unknown theme: refused, nothing written.
    let e = c.call("config_set_theme", json!({"name": "nowhere"})).unwrap_err();
    assert_eq!(e["data"]["reason"], "theme_unknown");
    assert_eq!(std::fs::read_to_string(dir.path().join("config.toml")).unwrap(), text);

    // A config.toml refused as it is: not touched.
    std::fs::write(dir.path().join("config.toml"), "[sync]\npoll_interval_secs = 1\n").unwrap();
    let e = c.call("config_set_theme", json!({"name": "regular"})).unwrap_err();
    assert_eq!(e["data"]["reason"], "config_not_written");
    assert_eq!(std::fs::read_to_string(dir.path().join("config.toml")).unwrap(), "[sync]\npoll_interval_secs = 1\n");
}

#[test]
fn config_set_compose_format_writes_the_file_and_applies_it() {
    // edition.md, section 3: ⇧⌘T makes the mode it converts a draft to the
    // mode of the next mails.
    let dir = tempfile::tempdir().unwrap();
    let mine = "# my settings\n[compose]\nundo_send_seconds = 5   # time to think\n";
    std::fs::write(dir.path().join("config.toml"), mine).unwrap();
    let _core = start(dir.path());
    let mut c = Client::connect(dir.path());
    assert_eq!(c.call("compose_formats", json!({})).unwrap()["default_format"], "text");

    let f = c.call("config_set_compose_format", json!({"format": "html"})).unwrap();
    assert_eq!(f["default_format"], "html", "the new default is returned");
    assert!(f["fonts"].as_array().is_some_and(|a| !a.is_empty()), "with the lists of the formatted mode");
    let text = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
    assert_eq!(text, format!("{mine}format = \"html\"\n"), "the rest of the file is kept");
    assert_eq!(c.call("compose_formats", json!({})).unwrap()["default_format"], "html");

    // Back to text: the same line, replaced.
    c.call("config_set_compose_format", json!({"format": "text"})).unwrap();
    let text = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
    assert_eq!(text, format!("{mine}format = \"text\"\n"));

    // A mode the editor does not have: refused, nothing written.
    let e = c.call("config_set_compose_format", json!({"format": "rtf"})).unwrap_err();
    assert_eq!(e["data"]["reason"], "compose_format_unknown");
    assert_eq!(std::fs::read_to_string(dir.path().join("config.toml")).unwrap(), text);

    // A config.toml refused as it is: not touched.
    std::fs::write(dir.path().join("config.toml"), "[sync]\npoll_interval_secs = 1\n").unwrap();
    let e = c.call("config_set_compose_format", json!({"format": "html"})).unwrap_err();
    assert_eq!(e["data"]["reason"], "config_not_written");
    assert_eq!(std::fs::read_to_string(dir.path().join("config.toml")).unwrap(), "[sync]\npoll_interval_secs = 1\n");
}
