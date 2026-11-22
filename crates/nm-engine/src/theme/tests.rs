//! Tests of the themes (`themes.md`, section 11).

use super::catalog::{BUILTINS, KEYS};
use super::*;

fn ok(name: &str, text: &str) -> Layer {
    parse(name, text).unwrap_or_else(|p| panic!("refused: {p}"))
}

/// The problem refusing `text` as the theme `name`, which must say `says`,
/// on `line`.
fn refused(name: &str, text: &str, line: Option<u32>, says: &str) -> Problem {
    let p = parse(name, text).expect_err("accepted");
    assert!(p.to_string().contains(says), "{p}");
    assert_eq!(p.file, file_label(name));
    assert_eq!(p.line, line, "{p}");
    p
}

fn layers(files: &[(&str, &str)]) -> BTreeMap<String, Layer> {
    files.iter().map(|(n, t)| (n.to_string(), ok(n, t))).collect()
}

fn color(values: &Values, key: &str) -> Color {
    match &values[key] {
        Value::Color(c) => c.clone(),
        other => panic!("{key}: {other:?}"),
    }
}

fn pair(light: &str, dark: &str) -> Color {
    Color::Pair { light: light.into(), dark: dark.into() }
}

// --- The built-in themes ---------------------------------------------------

#[test]
fn each_builtin_theme_gives_every_key_a_value() {
    for name in BUILTINS {
        let values = builtin(name).unwrap();
        for key in KEYS {
            assert!(values.contains_key(&key.dotted()), "{name}: {}", key.dotted());
        }
        assert_eq!(values.len(), KEYS.len(), "{name} has keys outside the catalog");
    }
}

#[test]
fn a_builtin_theme_resolves_alone() {
    for name in BUILTINS {
        let r = resolve(name, &BTreeMap::new()).unwrap();
        assert_eq!(r.chain, vec![name.to_string()]);
        assert_eq!(&r.values, builtin(name).unwrap());
        assert!(r.warnings.is_empty());
    }
}

#[test]
fn clean_and_regular_are_readable() {
    // `themes.md`, section 6.5: the shipped themes reach WCAG AA. regular
    // uses the colors of macOS for its texts, which are not checked.
    for name in ["clean", "regular"] {
        let w = contrast_warnings(name, builtin(name).unwrap());
        assert!(w.is_empty(), "{name}: {w:?}");
    }
}

#[test]
fn geek_is_readable_too() {
    // Its secondary text (dates, counts) was below AA on its background
    // (3.3:1 light, 3.6:1 dark); darkened in November 2026, it reaches it.
    let w = contrast_warnings("geek", builtin("geek").unwrap());
    assert!(w.is_empty(), "{w:?}");
}

#[test]
fn the_values_of_the_specification() {
    // A few values of `themes.md`, section 6, one per table, read from the
    // embedded files.
    let geek = builtin("geek").unwrap();
    assert_eq!(geek["layout.row_height"], Value::Int(20));
    assert_eq!(geek["window.title_bar"], Value::Text("shown".into()));
    assert_eq!(color(geek, "colors.selection"), pair("#D3E0CE", "#33432F"));
    let clean = builtin("clean").unwrap();
    assert_eq!(clean["motion.sidebar_ms"], Value::Int(120));
    assert_eq!(clean["window.key_hints"], Value::Bool(false));
    assert_eq!(clean["fonts.interface"], Value::Text("system".into()));
    let regular = builtin("regular").unwrap();
    assert_eq!(regular["window.chrome"], Value::Text("toolbar".into()));
    assert_eq!(color(regular, "colors.unread"), Color::System("accent".into()));
    assert_eq!(regular["layout.avatars"], Value::Bool(true));
    match &regular["window.blocks"] {
        Value::Blocks(b) => assert_eq!(b.first().map(String::as_str), Some("account")),
        other => panic!("{other:?}"),
    }
}

// --- Reading a file --------------------------------------------------------

#[test]
fn an_empty_file_says_nothing() {
    let l = ok("mine", "");
    assert!(l.values.is_empty() && l.extends.is_none() && l.warnings.is_empty());
    assert!(ok("mine", "version = 1\n").values.is_empty());
}

#[test]
fn the_forms_of_a_color() {
    let l = ok(
        "mine",
        "[colors]\ntext = \"#abcdef\"\nmuted = \"#AbCdEf80\"\nbackground = { light = \"#ffffff\", dark = \"#000000\" }\naccent = \"system.accent\"\n\
         [colors.flag]\nlight = \"#111111\"\ndark = \"#222222\"\n",
    );
    assert_eq!(color(&l.values, "colors.text"), pair("#ABCDEF", "#ABCDEF"));
    assert_eq!(color(&l.values, "colors.muted"), pair("#ABCDEF80", "#ABCDEF80"));
    assert_eq!(color(&l.values, "colors.background"), pair("#FFFFFF", "#000000"));
    assert_eq!(color(&l.values, "colors.accent"), Color::System("accent".into()));
    assert_eq!(color(&l.values, "colors.flag"), pair("#111111", "#222222"));
}

#[test]
fn a_wrong_color_is_refused_with_its_line() {
    refused("mine", "[colors]\ntext = \"red\"\n", Some(2), "expected \"#RRGGBB\"");
    refused("mine", "[colors]\ntext = \"#abc\"\n", Some(2), "expected \"#RRGGBB\"");
    refused("mine", "\n[colors]\ntext = \"system.pink\"\n", Some(3), "unknown color of the system");
    refused("mine", "[colors]\ntext = { light = \"#ffffff\" }\n", Some(2), "dark is missing");
    refused("mine", "[colors]\ntext = { light = \"#ffffff\", dark = \"system.label\" }\n", Some(2), "dark: expected");
    refused("mine", "[colors]\ntext = { light = \"#ffffff\", dark = \"#000000\", hover = \"#111111\" }\n", Some(2), "unknown key hover");
    refused("mine", "[colors]\ntext = 3\n", Some(2), "expected");
}

#[test]
fn numbers_choices_and_switches_are_checked() {
    let l = ok("mine", "[fonts]\nreading_scale = 1\n[layout]\nreader_max_width = 0\n");
    assert_eq!(l.values["fonts.reading_scale"], Value::Float(1.0));
    assert_eq!(l.values["layout.reader_max_width"], Value::Int(0));
    refused("mine", "[fonts]\nreading_scale = 3.0\n", Some(2), "must be from 0.75 to 1.5");
    refused("mine", "[layout]\nrow_height = 100\n", Some(2), "must be from 14 to 48");
    refused("mine", "[layout]\nrow_height = 20.5\n", Some(2), "expected an integer");
    refused("mine", "[layout]\nreader_max_width = 100\n", Some(2), "must be 0, or from 320 to 1200");
    refused("mine", "[layout]\nzebra = \"yes\"\n", Some(2), "expected true or false");
    refused("mine", "[window]\nchrome = \"ribbon\"\n", Some(2), "must be one of: status_bar, toolbar");
    refused("mine", "[window]\ndefault_columns = 4\n", Some(2), "must be from 1 to 3");
}

#[test]
fn a_font_is_a_name() {
    let l = ok("mine", "[fonts]\nreading = \"  Charter \"\ninterface = \"system-rounded\"\n");
    assert_eq!(l.values["fonts.reading"], Value::Text("Charter".into()));
    refused("mine", "[fonts]\nreading = \"\"\n", Some(2), "expected the name of a font");
    refused("mine", "[fonts]\nreading = 12\n", Some(2), "expected the name of a font");
}

#[test]
fn the_blocks_are_known_and_listed_once() {
    let l = ok("mine", "[window]\nblocks = [\"account\", \"|\", \"folder\", \"|\", \"->\", \"help\"]\n");
    assert_eq!(l.values["window.blocks"], Value::Blocks(vec!["account".into(), "|".into(), "folder".into(), "|".into(), "->".into(), "help".into()]));
    // An empty list hides every block: the keys still work.
    assert_eq!(ok("mine", "[window]\nblocks = []\n").values["window.blocks"], Value::Blocks(vec![]));
    refused("mine", "[window]\nblocks = [\"account\", \"weather\"]\n", Some(2), "unknown block \"weather\"");
    refused("mine", "[window]\nblocks = [\"account\", \"account\"]\n", Some(2), "listed twice");
    refused("mine", "[window]\nblocks = [\"->\", \"help\", \"->\"]\n", Some(2), "listed twice");
    refused("mine", "[window]\nblocks = \"account\"\n", Some(2), "expected a list of blocks");
}

#[test]
fn unknown_keys_are_warnings_unknown_tables_errors() {
    let l = ok("mine", "turbo = true\n[layout]\nwobble = 3\nzebra = true\n");
    let keys: Vec<_> = l.warnings.iter().map(|w| (w.key.clone().unwrap(), w.line)).collect();
    assert_eq!(keys, vec![("turbo".to_string(), Some(1)), ("layout.wobble".to_string(), Some(3))]);
    assert_eq!(l.values["layout.zebra"], Value::Bool(true));
    refused("mine", "[sounds]\nbeep = true\n", Some(1), "unknown table");
    refused("mine", "layout = 3\n", Some(1), "expected a table");
}

#[test]
fn version_and_syntax() {
    refused("mine", "version = 2\n", Some(1), "expected 1");
    refused("mine", "[colors\n", Some(1), "");
}

#[test]
fn extends_names_another_theme() {
    assert_eq!(ok("mine", "extends = \"clean\"\n").extends, Some(("clean".into(), Some(1))));
    refused("mine", "extends = \"mine\"\n", Some(1), "cannot extend itself");
    refused("mine", "extends = \"Clean\"\n", Some(1), "lowercase letters");
    refused("mine", "extends = 3\n", Some(1), "expected the name of a theme");
    // A file of a built-in theme modifies it: no other parent.
    refused("clean", "extends = \"geek\"\n", Some(1), "modifies the built-in theme clean");
    assert!(ok("clean", "extends = \"clean\"\n").extends.is_some());
}

// --- Inheritance -----------------------------------------------------------

#[test]
fn a_theme_of_the_user_is_laid_over_its_parent() {
    let ls = layers(&[("sepia", "extends = \"clean\"\n[layout]\nzebra = true\n[colors]\ntext = \"#3B3025\"\n")]);
    let r = resolve("sepia", &ls).unwrap();
    assert_eq!(r.chain, vec!["sepia", "clean"]);
    assert_eq!(r.values["layout.zebra"], Value::Bool(true));
    assert_eq!(color(&r.values, "colors.text"), pair("#3B3025", "#3B3025"));
    // What it does not say comes from clean.
    assert_eq!(r.values["motion.sidebar_ms"], Value::Int(120));
    assert_eq!(r.values.len(), KEYS.len());
}

#[test]
fn a_chain_of_files_rests_on_a_builtin_theme_modified_by_the_user() {
    let ls = layers(&[
        ("clean", "[motion]\nsidebar_ms = 0\n"),
        ("soft", "extends = \"clean\"\n[layout]\nrow_height = 26\n"),
        ("softer", "extends = \"soft\"\n[layout]\nrow_radius = 12\n"),
    ]);
    let r = resolve("softer", &ls).unwrap();
    assert_eq!(r.chain, vec!["softer", "soft", "clean"]);
    assert_eq!(r.values["layout.row_radius"], Value::Int(12));
    assert_eq!(r.values["layout.row_height"], Value::Int(26));
    // The user's change of clean reaches the themes that extend it.
    assert_eq!(r.values["motion.sidebar_ms"], Value::Int(0));
    // And clean itself is modified.
    assert_eq!(resolve("clean", &ls).unwrap().values["motion.sidebar_ms"], Value::Int(0));
}

#[test]
fn the_blocks_are_replaced_whole() {
    let ls = layers(&[("bare", "extends = \"geek\"\n[window]\nblocks = [\"help\"]\n")]);
    assert_eq!(resolve("bare", &ls).unwrap().values["window.blocks"], Value::Blocks(vec!["help".into()]));
}

#[test]
fn without_extends_the_parent_is_geek_with_a_warning() {
    let r = resolve("plain", &layers(&[("plain", "[layout]\nzebra = true\n")])).unwrap();
    assert_eq!(r.chain, vec!["plain", "geek"]);
    assert_eq!(r.warnings.len(), 1);
    assert_eq!(r.warnings[0].key.as_deref(), Some("extends"));
}

#[test]
fn broken_chains_are_refused_where_they_break() {
    let p = resolve("a", &layers(&[("a", "\nextends = \"nowhere\"\n")])).unwrap_err();
    assert_eq!((p.file.as_str(), p.line), ("themes/a.toml", Some(2)));
    assert!(p.message.contains("no theme named nowhere"), "{p}");

    let p = resolve("a", &layers(&[("a", "extends = \"b\"\n"), ("b", "extends = \"a\"\n")])).unwrap_err();
    assert!(p.message.contains("circular inheritance: a -> b -> a"), "{p}");

    let mut files: Vec<(String, String)> = (0..9).map(|i| (format!("t{i}"), format!("extends = \"t{}\"\n", i + 1))).collect();
    files.push(("t9".into(), "extends = \"geek\"\n".into()));
    let ls: BTreeMap<String, Layer> = files.iter().map(|(n, t)| (n.clone(), ok(n, t))).collect();
    let p = resolve("t0", &ls).unwrap_err();
    assert!(p.message.contains("more than 8 levels"), "{p}");
    // Seven files and the built-in theme: allowed.
    assert!(resolve("t3", &ls).is_ok());

    assert!(resolve("ghost", &BTreeMap::new()).unwrap_err().message.contains("no theme named ghost"));
}

// --- Contrast --------------------------------------------------------------

#[test]
fn contrast_follows_wcag() {
    assert!((contrast("#000000", "#FFFFFF") - 21.0).abs() < 1e-9);
    assert!((contrast("#777777", "#777777") - 1.0).abs() < 1e-9);
    // The grey of the mock-ups for clean: 5.4:1 on white.
    assert!((contrast("#676C65", "#FFFFFF") - 5.4).abs() < 0.1);
}

#[test]
fn a_pale_text_gives_a_warning_per_appearance() {
    let ls = layers(&[("pale", "extends = \"clean\"\n[colors]\ntext = { light = \"#BBBBBB\", dark = \"#E6E7E3\" }\n")]);
    let r = resolve("pale", &ls).unwrap();
    let w = contrast_warnings("pale", &r.values);
    assert!(w.iter().all(|p| p.key.as_deref() == Some("colors.text")), "{w:?}");
    assert!(w.iter().all(|p| p.message.contains("light appearance")), "{w:?}");
    assert_eq!(w.len(), 2, "on background and on list: {w:?}");
}

// --- The view --------------------------------------------------------------

#[test]
fn the_view_carries_every_key_of_the_catalog() {
    let r = resolve("regular", &BTreeMap::new()).unwrap();
    let v = serde_json::to_value(view(&r, Path::new("/t"), None, &[])).unwrap();
    for key in KEYS {
        let found = if key.table == "colors" { &v["colors"][key.name] } else { &v[key.table][key.name] };
        assert!(!found.is_null(), "{} missing from the view", key.dotted());
    }
    assert_eq!(v["colors"]["unread"], serde_json::json!({"system": "accent"}));
    assert_eq!(v["colors"]["marked"], serde_json::json!({"light": "#F3E3EE", "dark": "#3A2E45"}));
    // No other key in the tables.
    for table in catalog::TABLES {
        let n = v[*table].as_object().unwrap().len();
        assert_eq!(n, KEYS.iter().filter(|k| k.table == *table).count(), "{table}");
    }
}

#[test]
fn the_builtins_json_reads_back() {
    let json = builtins_json();
    let themes: BTreeMap<String, nm_api::ThemeView> = serde_json::from_str(&json).unwrap();
    assert_eq!(themes.keys().cloned().collect::<Vec<_>>(), vec!["clean", "geek", "regular"]);
    assert_eq!(themes["clean"].motion.sidebar_ms, 120);
}

// --- Writing a file --------------------------------------------------------

#[test]
fn a_template_changes_nothing() {
    for name in BUILTINS {
        let text = template(name).unwrap();
        let l = ok(name, &text);
        assert!(l.values.is_empty() && l.warnings.is_empty(), "{name}: {l:?}");
    }
}

#[test]
fn uncommenting_any_line_of_a_template_gives_its_value() {
    for name in BUILTINS {
        let text = template(name).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        let mut tried = 0;
        for (i, line) in lines.iter().enumerate() {
            let Some(rest) = line.strip_prefix("# ") else { continue };
            // Only the lines of values: `key = value`.
            let Some((k, _)) = rest.split_once(" = ") else { continue };
            if k.contains(' ') {
                continue;
            }
            let mut copy = lines.clone();
            copy[i] = rest;
            let l = parse(name, &copy.join("\n")).unwrap_or_else(|p| panic!("{name}, line {}: {p}", i + 1));
            assert_eq!(l.values.len(), 1, "{name}: {rest}");
            let (dotted, value) = l.values.iter().next().unwrap();
            assert_eq!(&builtin(name).unwrap()[dotted], value, "{name}: {rest}");
            tried += 1;
        }
        assert_eq!(tried, KEYS.len(), "{name}");
    }
}

#[test]
fn a_new_theme_extends_its_parent_and_changes_nothing() {
    let values = resolve("clean", &BTreeMap::new()).unwrap().values;
    let text = new_theme_file("mine", "clean", &values);
    let l = ok("mine", &text);
    assert_eq!(l.extends.as_ref().map(|(p, _)| p.as_str()), Some("clean"));
    assert!(l.values.is_empty());
    assert_eq!(resolve("mine", &BTreeMap::from([("mine".to_string(), l)])).unwrap().values, values);
}

// --- The directory ---------------------------------------------------------

fn write(dir: &Path, name: &str, text: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(format!("{name}.toml")), text).unwrap();
}

fn changed(events: &[nm_api::Event]) -> bool {
    events.iter().any(|e| matches!(e, nm_api::Event::ThemeChanged {}))
}

fn errors(events: &[nm_api::Event]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| match e {
            nm_api::Event::ConfigError { file, .. } => Some(file.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_set_follows_the_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("themes");
    write(&dir, "sepia", "extends = \"clean\"\n[layout]\nzebra = true\n");
    let mut set = ThemeSet::new(dir.clone(), "sepia");
    // The first reading is no change.
    let events = set.reload(true);
    assert!(events.is_empty(), "{events:?}");
    assert_eq!(set.applied().chain, vec!["sepia", "clean"]);

    // Saved with the same text: nothing.
    write(&dir, "sepia", "extends = \"clean\"\n[layout]\nzebra = true\n");
    assert!(set.reload(false).is_empty());

    // A change: ThemeChanged.
    write(&dir, "sepia", "extends = \"clean\"\n[layout]\nzebra = false\n");
    assert!(changed(&set.reload(false)));
    assert_eq!(set.applied().values["layout.zebra"], Value::Bool(false));

    // Refused: the previous version stays, the app is told why.
    write(&dir, "sepia", "extends = \"clean\"\n[layout]\nzebra = 7\n");
    let events = set.reload(false);
    assert_eq!(errors(&events), vec!["themes/sepia.toml"]);
    assert!(changed(&events), "the error is part of the view");
    assert_eq!(set.applied().values["layout.zebra"], Value::Bool(false));
    assert_eq!(set.view().error.unwrap().line, Some(3));
    // The same refusal again (another event of the same save): silent.
    std::fs::write(dir.join("sepia.toml"), "extends = \"clean\"\n[layout]\nzebra = 7\n\n").unwrap();
    assert!(errors(&set.reload(false)).is_empty());

    // Valid again.
    write(&dir, "sepia", "extends = \"clean\"\n[layout]\nzebra = true\n");
    assert!(changed(&set.reload(false)));
    assert!(set.view().error.is_none());
    assert_eq!(set.applied().values["layout.zebra"], Value::Bool(true));
}

#[test]
fn changing_the_theme_chosen() {
    let tmp = tempfile::tempdir().unwrap();
    let mut set = ThemeSet::new(tmp.path().join("themes"), "geek");
    set.reload(true);
    assert!(set.set_selected("geek").is_empty());
    assert!(changed(&set.set_selected("regular")));
    assert_eq!(set.applied().name, "regular");
    assert_eq!(set.view().window.chrome, "toolbar");
}

#[test]
fn a_theme_that_does_not_exist_gives_geek_with_a_warning() {
    let tmp = tempfile::tempdir().unwrap();
    let mut set = ThemeSet::new(tmp.path().join("themes"), "nowhere");
    set.reload(true);
    assert_eq!(set.applied().name, "geek");
    let w = set.warnings().iter().find(|w| w.key.as_deref() == Some("ui.theme")).expect("a warning");
    assert_eq!(w.file, "config.toml");
    assert!(w.message.contains("no theme named nowhere"), "{w}");
}

#[test]
fn a_theme_refused_at_start_gives_geek_and_says_why() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("themes");
    write(&dir, "sepia", "extends = \"clean\"\n[layout]\nzebra = 7\n");
    let mut set = ThemeSet::new(dir, "sepia");
    let events = set.reload(true);
    assert_eq!(errors(&events), vec!["themes/sepia.toml"]);
    assert_eq!(set.applied().name, "geek");
    assert_eq!(set.error().unwrap().file, "themes/sepia.toml");
    let entry = set.list().themes.into_iter().find(|t| t.name == "sepia").unwrap();
    assert!(entry.error.is_some());
}

#[test]
fn a_builtin_file_refused_at_start_leaves_the_builtin_theme() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("themes");
    write(&dir, "clean", "[layout]\nzebra = 7\n");
    let mut set = ThemeSet::new(dir, "clean");
    set.reload(true);
    assert_eq!(set.applied().name, "clean");
    assert_eq!(&set.applied().values, builtin("clean").unwrap());
    assert_eq!(set.error().unwrap().file, "themes/clean.toml");
}

#[test]
fn a_parent_refused_is_used_in_its_last_valid_version() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("themes");
    write(&dir, "base", "extends = \"clean\"\n[layout]\nrow_height = 40\n");
    write(&dir, "child", "extends = \"base\"\n");
    let mut set = ThemeSet::new(dir.clone(), "child");
    set.reload(true);
    write(&dir, "base", "extends = \"clean\"\n[layout]\nrow_height = \"tall\"\n");
    let events = set.reload(false);
    assert_eq!(errors(&events), vec!["themes/base.toml"]);
    assert_eq!(set.applied().values["layout.row_height"], Value::Int(40));
    assert_eq!(set.error().unwrap().file, "themes/base.toml");
}

#[test]
fn a_parent_deleted_breaks_the_chain() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("themes");
    write(&dir, "base", "extends = \"clean\"\n");
    write(&dir, "child", "extends = \"base\"\n[layout]\nzebra = true\n");
    let mut set = ThemeSet::new(dir.clone(), "child");
    set.reload(true);
    std::fs::remove_file(dir.join("base.toml")).unwrap();
    let events = set.reload(false);
    assert_eq!(errors(&events), vec!["themes/child.toml"]);
    // The theme in effect stays as it was.
    assert_eq!(set.applied().name, "child");
    assert_eq!(set.applied().values["layout.zebra"], Value::Bool(true));
    let entry = set.list().themes.into_iter().find(|t| t.name == "child").unwrap();
    assert!(entry.error.unwrap().message.contains("no theme named base"));
}

#[test]
fn deleting_the_file_of_a_builtin_theme_brings_it_back() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("themes");
    write(&dir, "clean", "[layout]\nzebra = true\n");
    let mut set = ThemeSet::new(dir.clone(), "clean");
    set.reload(true);
    assert_eq!(set.list().themes[1].origin, "builtin_modified");
    std::fs::remove_file(dir.join("clean.toml")).unwrap();
    assert!(changed(&set.reload(false)));
    assert_eq!(&set.applied().values, builtin("clean").unwrap());
    assert_eq!(set.list().themes[1].origin, "builtin");
}

#[test]
fn the_list_gives_the_builtin_themes_first() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("themes");
    write(&dir, "zen", "extends = \"clean\"\n");
    write(&dir, "amber", "[layout]\nzebra = true\n");
    write(&dir, "Bad Name", "");
    std::fs::write(dir.join("notes.txt"), "not a theme").unwrap();
    let mut set = ThemeSet::new(dir, "geek");
    set.reload(true);
    let list = set.list();
    let names: Vec<_> = list.themes.iter().map(|t| (t.name.as_str(), t.origin.as_str(), t.parent.as_deref())).collect();
    assert_eq!(
        names,
        vec![
            ("geek", "builtin", None),
            ("clean", "builtin", None),
            ("regular", "builtin", None),
            ("amber", "user", Some("geek")),
            ("zen", "user", Some("clean")),
        ]
    );
    // The file whose name cannot name a theme is a warning.
    assert!(set.warnings().iter().any(|w| w.file == "themes/Bad Name.toml"), "{:?}", set.warnings());
}

#[test]
fn a_file_too_big_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("themes");
    write(&dir, "huge", &format!("extends = \"clean\"\n# {}\n", "x".repeat(70 * 1024)));
    let mut set = ThemeSet::new(dir, "huge");
    let events = set.reload(true);
    assert_eq!(errors(&events), vec!["themes/huge.toml"]);
    assert!(set.error().unwrap().message.contains("bigger than 64 KB"));
}

#[test]
fn a_missing_directory_has_the_builtin_themes() {
    let tmp = tempfile::tempdir().unwrap();
    let mut set = ThemeSet::new(tmp.path().join("nothing-here"), "clean");
    assert!(set.reload(true).is_empty());
    assert_eq!(set.applied().name, "clean");
    assert_eq!(set.list().themes.len(), 3);
}

/// `apps/NiceMailer/Sources/NiceMailer/Resources/themes-builtin.json` must
/// be [`builtins_json`]: the app reads it until the core answers and in its
/// tests (`themes.md`, section 7). Regenerate it with
/// `nicemailer theme show --builtins --json > apps/NiceMailer/Sources/NiceMailer/Resources/themes-builtin.json`.
#[test]
fn the_json_of_the_app_is_up_to_date() {
    let app = include_str!("../../../../apps/NiceMailer/Sources/NiceMailer/Resources/themes-builtin.json");
    assert!(
        app == builtins_json(),
        "themes-builtin.json is stale: run `nicemailer theme show --builtins --json > \
         apps/NiceMailer/Sources/NiceMailer/Resources/themes-builtin.json`"
    );
}

#[test]
fn the_file_created_leaves_a_builtin_theme_unmodified() {
    // The core creates every built-in file with its keys commented out:
    // the theme is listed as built in, not modified, until a line is
    // uncommented.
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("themes");
    write(&dir, "clean", &template("clean").unwrap());
    let mut set = ThemeSet::new(dir.clone(), "clean");
    set.reload(true);
    assert_eq!(set.list().themes[1].origin, "builtin");
    write(&dir, "clean", &template("clean").unwrap().replace("# zebra = false", "zebra = true"));
    set.reload(false);
    assert_eq!(set.list().themes[1].origin, "builtin_modified");
}
