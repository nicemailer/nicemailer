//! Tests of `keymap.toml` (`raccourcis.md`, section 11).

use super::catalog::{ACTIONS, Context};
use super::*;

fn chord(s: &str) -> Chord {
    Chord::parse(s).unwrap()
}

fn ok(text: &str) -> Parsed {
    parse(text).unwrap_or_else(|p| panic!("refused: {p}"))
}

/// The problem refusing `text`, which must say `says`.
fn refused(text: &str, says: &str) -> Problem {
    let p = parse(text).expect_err("accepted");
    assert!(p.to_string().contains(says), "{p}");
    assert_eq!(p.file, KEYMAP_FILE);
    p
}

// --- The defaults ----------------------------------------------------------

#[test]
fn an_empty_file_gives_the_defaults() {
    let p = ok("");
    assert_eq!(&p.keymap, defaults());
    assert!(p.warnings.is_empty());
    assert_eq!(&ok("version = 1\n").keymap, defaults());
}

#[test]
fn every_action_has_a_default_key_in_a_context_it_belongs_to() {
    let d = defaults();
    for a in ACTIONS {
        let in_context = d.contexts.iter().any(|(c, list)| {
            list.iter().any(|b| b.action.as_deref() == Some(a.id) && c.within(a.scope))
        });
        let in_menu = d.menus.iter().any(|m| m.entries.iter().any(|(_, x)| x == a.id));
        assert!(in_context || in_menu, "{} has no default key", a.id);
    }
}

#[test]
fn the_defaults_are_those_of_the_app_until_now() {
    // A few keys checked one by one: those the help, the UI scripts and
    // the habits of the users rely on.
    let d = defaults();
    let at = |c: Context, k: &str| d.resolve(c, &chord(k)).map(str::to_string);
    assert_eq!(at(Context::Mail, "d").as_deref(), Some("mail.archive"));
    assert_eq!(at(Context::Quarantine, "d").as_deref(), Some("move.deleted"));
    assert_eq!(at(Context::Quarantine, "j").as_deref(), Some("list.down"));
    assert_eq!(at(Context::Mail, "J").as_deref(), Some("list.mark_down"));
    assert_eq!(at(Context::Mail, "shift-j").as_deref(), Some("list.mark_down"));
    assert_eq!(at(Context::Mail, "ctrl-C").as_deref(), Some("sender.mails"));
    assert_eq!(at(Context::Mail, ";").as_deref(), Some("menu.goto"));
    assert_eq!(at(Context::Contacts, "?").as_deref(), Some("help.show"));
    assert_eq!(at(Context::Contacts, "j").as_deref(), Some("screen.down"));
    assert_eq!(at(Context::Compose, "ctrl-cmd-f").as_deref(), Some("window.full_screen"));
    assert_eq!(at(Context::Compose, "j"), None);
    assert_eq!(at(Context::Picker, "ctrl-a").as_deref(), Some("line.start"));
    let save = d.menu("save").unwrap();
    assert_eq!(save.entries.first().map(|(c, a)| (c.canonical(), a.as_str())), Some(("c".into(), "save.clipboard")));
    assert_eq!(d.menus.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), BUILTIN_MENUS);
    assert!(d.menus.iter().all(|m| m.builtin));
}

#[test]
fn the_template_gives_the_defaults_and_any_line_can_be_uncommented() {
    let t = template();
    assert_eq!(&ok(&t).keymap, defaults());
    let lines: Vec<&str> = t.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        // A commented key: `# key = "action"   # role`.
        if let Some(rest) = line.strip_prefix("# ")
            && rest.contains(" = \"")
        {
            let mut copy = lines.clone();
            copy[i] = rest;
            let text = copy.join("\n");
            let p = parse(&text).unwrap_or_else(|e| panic!("line {} uncommented: {e}", i + 1));
            assert_eq!(&p.keymap, defaults(), "line {}", i + 1);
        }
    }
}

#[test]
fn the_defaults_file_is_the_defaults() {
    assert_eq!(&ok(&defaults_file()).keymap, defaults());
}

/// `apps/NiceMailer/Sources/NiceMailer/Resources/keymap-defaults.json`
/// must be [`defaults_json`]: the app reads it for its tests and until the
/// core answers. Regenerate it with
/// `nicemailer keymap show --defaults --json > apps/NiceMailer/Sources/NiceMailer/Resources/keymap-defaults.json`.
#[test]
fn the_json_of_the_app_is_up_to_date() {
    let app = include_str!("../../../../apps/NiceMailer/Sources/NiceMailer/Resources/keymap-defaults.json");
    assert!(
        app == defaults_json(),
        "keymap-defaults.json is stale: run `nicemailer keymap show --defaults --json > \
         apps/NiceMailer/Sources/NiceMailer/Resources/keymap-defaults.json`"
    );
}

/// The tables of the specification (`raccourcis.md`, section 6.1) list the
/// same actions as the catalogue.
#[test]
fn the_specification_lists_every_action() {
    let spec = include_str!("../../../../specifications/raccourcis.md");
    for a in ACTIONS {
        assert!(spec.contains(&format!("`{}`", a.id)), "{} is not in raccourcis.md", a.id);
    }
}

// --- Merging ----------------------------------------------------------------

#[test]
fn a_key_replaces_the_default_in_place_or_is_added() {
    let p = ok("[mail]\nd = \"mail.delete\"\ne = \"mail.archive\"\n");
    let k = &p.keymap;
    assert_eq!(k.resolve(Context::Mail, &chord("d")), Some("mail.delete"));
    assert_eq!(k.resolve(Context::Mail, &chord("e")), Some("mail.archive"));
    // In place: `d` keeps its position among the keys of `[mail]`.
    let mail = &k.contexts[&Context::Mail];
    let pos = |key: &str| mail.iter().position(|b| b.chord == chord(key)).unwrap();
    let default_pos = defaults().contexts[&Context::Mail].iter().position(|b| b.chord == chord("d")).unwrap();
    assert_eq!(pos("d"), default_pos);
    assert_eq!(pos("e"), mail.len() - 1);
}

#[test]
fn none_removes_a_key_and_hides_the_parent() {
    let k = ok("[mail]\nd = \"none\"\n").keymap;
    assert_eq!(k.resolve(Context::Mail, &chord("d")), None);
    // Quarantine has its own `d`.
    assert_eq!(k.resolve(Context::Quarantine, &chord("d")), Some("move.deleted"));
    // `none` in a child hides the parent's key there only.
    let k = ok("[contacts]\n\"?\" = \"none\"\n").keymap;
    assert_eq!(k.resolve(Context::Contacts, &chord("?")), None);
    assert_eq!(k.resolve(Context::Tags, &chord("?")), Some("help.show"));
    assert_eq!(k.resolve(Context::Mail, &chord("?")), Some("help.show"));
}

#[test]
fn a_child_context_wins_over_its_parent() {
    let k = ok("[normal]\nx = \"undo.last\"\n[mail]\nx = \"mail.archive\"\n").keymap;
    assert_eq!(k.resolve(Context::Mail, &chord("x")), Some("mail.archive"));
    assert_eq!(k.resolve(Context::Contacts, &chord("x")), Some("undo.last"));
}

#[test]
fn two_spellings_of_one_key() {
    let k = ok("[mail]\n\"shift-d\" = \"mail.toggle_flag\"\n").keymap;
    assert_eq!(k.resolve(Context::Mail, &chord("D")), Some("mail.toggle_flag"));
    let p = refused("[mail]\nJ = \"list.down\"\n\"shift-j\" = \"list.up\"\n", "same key");
    assert_eq!(p.line, Some(3));
}

#[test]
fn menus_change_add_and_remove_rows() {
    let k = ok("[menu.save]\nc = \"none\"\nx = \"save.pdf\"\nt = \"save.eml\"\n").keymap;
    let save = k.menu("save").unwrap();
    let rows: Vec<(String, &str)> = save.entries.iter().map(|(c, a)| (c.canonical(), a.as_str())).collect();
    assert!(!rows.iter().any(|(c, _)| c == "c"));
    assert!(rows.contains(&("t".into(), "save.eml")));
    // `x` replaces `p`, the default key of save.pdf, and `e` is no longer
    // save.eml's: each action keeps a single row.
    assert!(rows.contains(&("x".into(), "save.pdf")));
    assert!(!rows.iter().any(|(c, _)| c == "p" || c == "e"));
    assert_eq!(rows.iter().filter(|(_, a)| *a == "save.eml").count(), 1);
}

/// The rows of `menu` in `k`: (key, action), in their order.
fn rows<'a>(k: &'a Keymap, menu: &str) -> Vec<(String, &'a str)> {
    k.menu(menu).unwrap().entries.iter().map(|(c, a)| (c.canonical(), a.as_str())).collect()
}

/// A key given to an action in a menu replaces its default key there, at
/// its place: the menu shows one row per action, with the file's key.
#[test]
fn a_new_key_in_a_menu_replaces_the_default_key_of_its_action() {
    let k = ok("[menu.move]\nf = \"move.todo\"\n").keymap;
    let move_rows = rows(&k, "move");
    let defaults_rows = rows(defaults(), "move");
    assert_eq!(move_rows.len(), defaults_rows.len());
    let at = defaults_rows.iter().position(|(c, _)| c == "t").unwrap();
    assert_eq!(move_rows[at], ("f".into(), "move.todo"));
    assert!(!move_rows.iter().any(|(c, _)| c == "t"));

    // Two keys written for one action in the file: both kept, together.
    let k = ok("[menu.move]\nf = \"move.todo\"\nF = \"move.todo\"\n").keymap;
    let move_rows = rows(&k, "move");
    assert_eq!(&move_rows[at..at + 2], &[("f".into(), "move.todo"), ("F".into(), "move.todo")]);
}

/// The menu `;` of a real file: keys swapped between folders, one taken
/// from a screen. Every action keeps at most one row, and the keys the file
/// does not mention stay.
#[test]
fn keys_swapped_in_a_menu() {
    let text = "[menu.goto]\nf = \"goto.todo\"\nr = \"goto.waiting\"\nl = \"goto.newsletter\"\n\
                e = \"goto.sent\"\nb = \"goto.drafts\"\nc = \"goto.deleted\"\ns = \"goto.junk\"\n";
    let k = ok(text).keymap;
    let goto = rows(&k, "goto");
    let key = |action: &str| goto.iter().filter(|(_, a)| *a == action).map(|(c, _)| c.as_str()).collect::<Vec<_>>();
    assert_eq!(key("goto.todo"), ["f"]);
    assert_eq!(key("goto.waiting"), ["r"]);
    assert_eq!(key("goto.newsletter"), ["l"]);
    assert_eq!(key("goto.sent"), ["e"]);
    assert_eq!(key("goto.drafts"), ["b"]);
    assert_eq!(key("goto.deleted"), ["c"]);
    assert_eq!(key("goto.junk"), ["s"]);
    // Their keys taken: Banned (`b`), Everything (`e`), Contacts (`c`).
    assert!(key("goto.banned").is_empty() && key("goto.everything").is_empty() && key("goto.contacts").is_empty());
    // Untouched.
    assert_eq!(key("goto.inbox"), ["i"]);
    assert_eq!(key("goto.history"), ["h"]);
    // One key, one row; and the order of the defaults kept.
    let keys: BTreeSet<&str> = goto.iter().map(|(c, _)| c.as_str()).collect();
    assert_eq!(keys.len(), goto.len());
    assert_eq!(goto[0].1, "goto.inbox");
    assert_eq!(goto[1], ("f".into(), "goto.todo"));
}

/// A row keeps the place of its action, not of its key: an action whose
/// default key the file gave to another keeps its place with its new key.
/// The menu `;` of a real file, where Contacts went after History.
#[test]
fn a_row_keeps_the_place_of_its_action() {
    let text = "[menu.goto]\nf = \"goto.todo\"\nr = \"goto.waiting\"\nt = \"goto.work\"\nl = \"goto.newsletter\"\n\
                e = \"goto.sent\"\nb = \"goto.drafts\"\nc = \"goto.deleted\"\ns = \"goto.junk\"\nv = \"goto.banned\"\n\
                E = \"goto.everything\"\nC = \"goto.contacts\"\nG = \"goto.groups\"\n";
    let k = ok(text).keymap;
    let actions: Vec<&str> = rows(&k, "goto").iter().map(|(_, a)| *a).collect();
    let defaults: Vec<&str> = rows(defaults(), "goto").iter().map(|(_, a)| *a).collect();
    assert_eq!(actions, defaults, "the order of the defaults, every action kept");
    let goto = rows(&k, "goto");
    let at = |action: &str| goto.iter().find(|(_, a)| *a == action).map(|(c, _)| c.as_str()).unwrap();
    assert_eq!((at("goto.todo"), at("goto.work"), at("goto.contacts"), at("goto.tags")), ("f", "t", "C", "T"));
    // The screens together, History last.
    assert_eq!(&actions[actions.len() - 4..], ["goto.contacts", "goto.tags", "goto.groups", "goto.history"]);
}

/// In a context, an action may have several keys: the default key stays,
/// but the file's comes first, as the app shows the first one.
#[test]
fn a_new_key_in_a_context_comes_before_the_default_key_of_its_action() {
    let k = ok("[quarantine]\nl = \"move.newsletter\"\n").keymap;
    let keys: Vec<String> = k.contexts[&Context::Quarantine]
        .iter()
        .filter(|b| b.action.as_deref() == Some("move.newsletter"))
        .map(|b| b.chord.canonical())
        .collect();
    assert_eq!(keys, ["l", "n"]);
    assert_eq!(k.resolve(Context::Quarantine, &chord("n")), Some("move.newsletter"));
    // A key that is new for an action the context did not have: at the end.
    let k = ok("[quarantine]\nX = \"move.todo\"\n").keymap;
    assert_eq!(k.contexts[&Context::Quarantine].last().unwrap().chord.canonical(), "X");
}

#[test]
fn a_menu_of_the_user() {
    let text = "[mail]\nspace = \"menu.mine\"\n\n[menu.mine]\na = \"save.attachments\"\np = \"save.pdf_open\"\n";
    let k = ok(text).keymap;
    assert_eq!(k.resolve(Context::Mail, &chord("space")), Some("menu.mine"));
    let m = k.menu("mine").unwrap();
    assert!(!m.builtin);
    assert_eq!(m.entries.len(), 2);
    assert_eq!(k.menus.last().unwrap().name, "mine");
    // Opened from any screen.
    ok("[normal]\nQ = \"menu.mine\"\n[menu.mine]\na = \"goto.inbox\"\n");
}

#[test]
fn a_menu_nobody_opens() {
    refused("[menu.mine]\na = \"save.pdf\"\n", "no key opens this menu");
    // A built-in one: a warning, the status bar still opens it.
    let p = ok("[mail]\nm = \"mail.archive\"\n");
    assert!(p.warnings.iter().any(|w| w.to_string().contains("menu.move")), "{:?}", p.warnings);
    // Given another key, no warning.
    assert!(ok("[mail]\nm = \"mail.archive\"\nM = \"menu.move\"\n").warnings.is_empty());
}

#[test]
fn a_menu_left_empty_is_refused() {
    let rows: String = defaults().menu("view").unwrap().entries.iter().map(|(c, _)| format!("\"{c}\" = \"none\"\n")).collect();
    let p = refused(&format!("\n[menu.view]\n{rows}"), "no row left");
    assert_eq!(p.line, Some(2));
}

// --- Refusals ---------------------------------------------------------------

#[test]
fn refusals_with_their_line() {
    let cases: &[(&str, &str, u32)] = &[
        ("[mail]\nd = \n", "", 2),
        ("version = 2\n", "expected 1", 1),
        ("\n[nowhere]\nd = \"mail.archive\"\n", "unknown context", 2),
        ("[mail]\n\"shift-/\" = \"help.show\"\n", "Shift is part of the character", 2),
        ("[mail]\nesc = \"mail.archive\"\n", "cannot be changed", 2),
        ("[everywhere]\n\"cmd-q\" = \"none\"\n", "cannot be changed", 2),
        ("[compose]\nx = \"compose.send\"\n", "types a character", 2),
        ("[everywhere]\nx = \"text.bigger\"\n", "types a character", 2),
        ("[picker]\n\"alt-n\" = \"picker.next\"\n", "types a character", 2),
        ("[compose]\nspace = \"compose.send\"\n", "types a character", 2),
        ("[compose]\nenter = \"compose.send\"\n", "edits the text", 2),
        ("[everywhere]\ntab = \"text.bigger\"\n", "edits the text", 2),
        ("[line_edit]\nleft = \"line.start\"\n", "edits the text", 2),
        ("[picker]\n\"shift-backspace\" = \"picker.next\"\n", "edits the text", 2),
        ("[contacts]\nx = \"compose.send\"\n", "belongs to [compose]", 2),
        ("[normal]\nx = \"mail.archive\"\n", "belongs to [mail]", 2),
        ("[mail]\nd = 1\n", "name of an action", 2),
        ("[mail]\n\n\nd = { a = 1 }\n", "name of an action", 4),
        ("[menu.save]\nx = \"menu.view\"\n", "menus do not nest", 2),
        ("[menu.save]\nx = \"picker.next\"\n", "cannot be a row", 2),
        ("[menu.Bad]\nx = \"save.pdf\"\n", "menu name", 1),
        ("mail = 1\n", "table of keys", 1),
        ("menu = 1\n", "expected tables", 1),
    ];
    for (text, says, line) in cases {
        let p = parse(text).expect_err(text);
        assert!(p.to_string().contains(says), "{text:?}: {p}");
        assert_eq!(p.line, Some(*line), "{text:?}: {p}");
    }
}

#[test]
fn keys_allowed_where_text_is_typed() {
    ok("[compose]\n\"cmd-shift-s\" = \"compose.send\"\n\"ctrl-s\" = \"compose.send\"\nf5 = \"compose.send\"\n");
    ok("[line_edit]\n\"ctrl-b\" = \"line.word_left\"\n\"alt-left\" = \"none\"\n");
    // Tab goes from field to field in a reply; Home and the arrows up and
    // down are free in a line.
    ok("[compose]\n\"shift-tab\" = \"compose.next_field\"\n[line_edit]\nhome = \"line.end\"\n[picker]\ndown = \"picker.previous\"\n");
    // Characters are fine in the panels that take no text.
    ok("[ai_panel]\no = \"ai.accept\"\n[choice]\nl = \"choice.choose\"\n");
}

#[test]
fn an_action_of_a_parent_in_a_child() {
    // `goto.*` belong to `normal`: allowed on the mail screen and in
    // Quarantine; `move.*` to `mail`, allowed in Quarantine.
    let k = ok("[mail]\nI = \"goto.inbox\"\n[quarantine]\nt = \"move.todo\"\n").keymap;
    assert_eq!(k.resolve(Context::Quarantine, &chord("I")), Some("goto.inbox"));
    assert_eq!(k.resolve(Context::Quarantine, &chord("t")), Some("move.todo"));
}

#[test]
fn unknown_actions_are_warnings() {
    let p = ok("[mail]\nx = \"mail.teleport\"\ny = \"menu.nothing\"\n");
    assert_eq!(p.warnings.len(), 2);
    assert_eq!(p.warnings[0].line, Some(2));
    assert!(p.warnings[0].message.contains("unknown action"));
    assert!(p.warnings[1].message.contains("no menu of that name"));
    assert_eq!(p.keymap.resolve(Context::Mail, &chord("x")), None);
}

#[test]
fn a_syntax_error_has_its_line() {
    let p = refused("[mail]\nd = \"mail.archive\n", "");
    assert_eq!(p.line, Some(2));
}

// --- The file followed by the core -------------------------------------------

/// Whether `events` is exactly one `KeymapChanged`.
fn changed(events: Vec<nm_api::Event>) -> bool {
    matches!(&events[..], [nm_api::Event::KeymapChanged {}])
}

#[test]
fn reload_applies_refuses_and_recovers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(KEYMAP_FILE);
    let mut f = KeymapFile::new(path.clone());

    // Missing at start: the defaults, nothing announced.
    assert!(f.reload(true).is_empty());
    assert_eq!(f.keymap(), defaults());

    std::fs::write(&path, "[mail]\ne = \"mail.archive\"\n").unwrap();
    assert!(changed(f.reload(false)));
    assert_eq!(f.keymap().resolve(Context::Mail, &chord("e")), Some("mail.archive"));
    // Read again, unchanged: nothing.
    assert!(f.reload(false).is_empty());

    // Invalid: refused, the previous keys stay, announced once.
    std::fs::write(&path, "[mail]\nesc = \"mail.archive\"\n").unwrap();
    let events = f.reload(false);
    assert!(matches!(&events[..], [nm_api::Event::ConfigError { file, line: Some(2), .. }] if file == KEYMAP_FILE), "{events:?}");
    assert_eq!(f.keymap().resolve(Context::Mail, &chord("e")), Some("mail.archive"));
    assert!(f.error().is_some());

    // Valid again with the same keys as before the error: announced, so the
    // app clears its message.
    std::fs::write(&path, "[mail]\ne = \"mail.archive\"\n").unwrap();
    assert!(changed(f.reload(false)));
    assert!(f.error().is_none());

    // Deleted: the defaults.
    std::fs::remove_file(&path).unwrap();
    assert!(changed(f.reload(false)));
    assert_eq!(f.keymap(), defaults());
}

#[test]
fn invalid_at_start_gives_the_defaults_and_the_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(KEYMAP_FILE);
    std::fs::write(&path, "[nowhere]\n").unwrap();
    let mut f = KeymapFile::new(path);
    let events = f.reload(true);
    assert_eq!(events.len(), 1);
    assert_eq!(f.keymap(), defaults());
    assert_eq!(f.error().map(|p| p.line), Some(Some(1)));
}

#[test]
fn the_view_spells_keys_canonically() {
    let k = ok("[mail]\n\"shift-cmd-x\" = \"mail.archive\"\nq = \"none\"\n").keymap;
    let (contexts, menus) = view(&k);
    let mail = &contexts["mail"];
    assert!(mail.iter().any(|b| b.key == "cmd-shift-x" && b.action == "mail.archive"));
    assert!(mail.iter().any(|b| b.key == "q" && b.action == "none"));
    assert!(mail.iter().any(|b| b.key == "J" && b.action == "list.mark_down"));
    assert!(contexts["mail"].iter().any(|b| b.key == "ctrl-shift-c"));
    assert_eq!(menus.len(), 4);
}
