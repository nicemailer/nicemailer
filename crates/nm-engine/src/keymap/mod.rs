//! `keymap.toml`: the keys of the app, redefined by the user
//! (`specifications/raccourcis.md`).
//!
//! The file holds only what differs from the defaults, which are in
//! `defaults.toml` next to this source (embedded at compile time). Each
//! table is a *context* (`[mail]`, `[compose]`…) or a *menu*
//! (`[menu.save]`), each line a key and the action it runs:
//!
//! ```toml
//! [mail]
//! e = "mail.archive"
//! d = "none"
//! ```
//!
//! [`parse`] reads a file, checks it, and merges it into the defaults: a key
//! of the file replaces the same key of the defaults, `"none"` removes it,
//! a new key is added; in a menu, a new key for an action replaces the
//! default key of that action ([`merge`]). The result, a [`Keymap`], is what the app receives
//! (`keymap_get`), already resolved: the app never sees the file.
//!
//! Like `config.toml` (`config.rs`), the file is read with `toml_edit`
//! rather than `serde`, for the line of every problem, and an invalid file
//! is refused whole: the previous keymap stays (`raccourcis.md`, section 3).
//! Unlike `config.toml`, it is refused alone: a mistake in a key must not
//! turn off the accounts.
//!
//! Speed: the file is small (a few hundred lines at most) and read only
//! when it changes; the app turns the result into dictionaries once, so a
//! key press is a lookup, not a search.

pub mod catalog;
pub mod notation;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use toml_edit::TableLike;

use crate::config::{self, Problem};
use catalog::{BUILTIN_MENUS, Context};
pub use notation::Chord;

/// Name of the file, always next to `config.toml`.
pub const KEYMAP_FILE: &str = "keymap.toml";

/// Version of the file format: the `version` key.
pub const FORMAT_VERSION: i64 = 1;

/// The file of defaults (`raccourcis.md`, section 6.1). Rust note:
/// `include_str!` embeds the file in the binary when it is compiled; there
/// is nothing to install or find at run time.
const DEFAULTS: &str = include_str!("defaults.toml");

/// Path of `keymap.toml`: next to `config_path`, whatever its name, so that
/// `NICEMAILER_CONFIG` moves the three files together.
pub fn path(config_path: &Path) -> PathBuf {
    config_path.with_file_name(KEYMAP_FILE)
}

// ---------------------------------------------------------------------------
// The keymap
// ---------------------------------------------------------------------------

/// One key of a context and what it does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub chord: Chord,
    /// The action; `None` for `"none"`, which removes the key here and
    /// hides the action its parents give it (`raccourcis.md`, section 5.1).
    pub action: Option<String>,
}

/// A menu: the second keys after the key that opens it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Menu {
    /// `save` for `[menu.save]`.
    pub name: String,
    /// Provided by the app (`catalog::BUILTIN_MENUS`), or made by the user.
    pub builtin: bool,
    /// In the order shown: the defaults first, then the file's new rows.
    pub entries: Vec<(Chord, String)>,
}

/// The keys in effect: the defaults with the file applied.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Keymap {
    /// The bindings of each context, in file order. Rust note: a
    /// `BTreeMap` iterates in key order (here the order of `Context`), so
    /// the JSON sent to the app is the same from one run to the next.
    pub contexts: BTreeMap<Context, Vec<Binding>>,
    /// The menus, the built-in ones first.
    pub menus: Vec<Menu>,
}

impl Keymap {
    /// The action of `chord` in `context` alone (not its parents):
    /// `Some(None)` for a `"none"`, `None` when the context does not have
    /// the key.
    pub fn get(&self, context: Context, chord: &Chord) -> Option<Option<&str>> {
        self.contexts.get(&context)?.iter().find(|b| b.chord == *chord).map(|b| b.action.as_deref())
    }

    /// What `chord` does in `context`, looking into the parents as the app
    /// does: the action, or `None` when no context has it or a `"none"`
    /// hides it.
    pub fn resolve(&self, context: Context, chord: &Chord) -> Option<&str> {
        let mut c = Some(context);
        while let Some(ctx) = c {
            if let Some(found) = self.get(ctx, chord) {
                return found;
            }
            c = ctx.parent();
        }
        None
    }

    pub fn menu(&self, name: &str) -> Option<&Menu> {
        self.menus.iter().find(|m| m.name == name)
    }

    /// Whether some key of some context opens the menu `name`.
    fn opens(&self, name: &str) -> bool {
        let id = format!("menu.{name}");
        self.contexts.values().flatten().any(|b| b.action.as_deref() == Some(id.as_str()))
    }
}

/// A valid file: the keymap it gives, and the warnings about it.
#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    pub keymap: Keymap,
    pub warnings: Vec<Problem>,
}

/// The keymap of an empty file: the defaults alone, read once.
///
/// Rust note: `OnceLock` runs its closure the first time `get_or_init` is
/// called, from whichever thread, and hands out the same value after.
pub fn defaults() -> &'static Keymap {
    static DEFAULT: OnceLock<Keymap> = OnceLock::new();
    DEFAULT.get_or_init(|| {
        let layer = read_layer(DEFAULTS).expect("defaults.toml is invalid");
        assert!(layer.warnings.is_empty(), "defaults.toml has warnings: {:?}", layer.warnings);
        let mut keymap = Keymap::default();
        merge(&mut keymap, layer, true);
        keymap
    })
}

/// Reads the text of `keymap.toml` (`""` for a missing file) and applies it
/// to the defaults; the first problem refuses the whole file.
pub fn parse(text: &str) -> Result<Parsed, Problem> {
    let mut layer = read_layer(text)?;
    let warnings = std::mem::take(&mut layer.warnings);
    let menu_lines = layer.menu_lines.clone();
    let mut keymap = defaults().clone();
    merge(&mut keymap, layer, false);
    let mut parsed = Parsed { keymap, warnings };
    check_menus(&mut parsed, &menu_lines)?;
    Ok(parsed)
}

// ---------------------------------------------------------------------------
// Reading a file
// ---------------------------------------------------------------------------

/// What one file says, before it is merged into the defaults.
struct Layer {
    contexts: Vec<(Context, Vec<Binding>)>,
    menus: Vec<(String, Vec<Binding>)>,
    /// Line of each `[menu.<name>]` header, for the problems found once
    /// the file is merged (a menu left empty).
    menu_lines: BTreeMap<String, Option<u32>>,
    warnings: Vec<Problem>,
}

fn problem(line: Option<u32>, key: Option<String>, message: impl Into<String>) -> Problem {
    Problem { file: KEYMAP_FILE, line, key, message: message.into() }
}

/// Reads and checks one file, without the defaults.
fn read_layer(text: &str) -> Result<Layer, Problem> {
    let doc = toml_edit::Document::parse(text).map_err(|e| {
        problem(
            e.span().map(|s| config::line_of(text, s.start)),
            None,
            e.message().lines().next().unwrap_or("invalid TOML").to_string(),
        )
    })?;
    let line = |span: Option<std::ops::Range<usize>>| span.map(|s| config::line_of(text, s.start));
    let root = doc.as_table();
    let mut layer = Layer { contexts: Vec::new(), menus: Vec::new(), menu_lines: BTreeMap::new(), warnings: Vec::new() };

    // The menus first: a key of a context may open a menu of the user
    // defined further down the file.
    let mut user_menus: BTreeSet<String> = BTreeSet::new();
    if let Some(item) = root.get("menu") {
        let table = item
            .as_table_like()
            .ok_or_else(|| problem(line(item.span()), Some("menu".into()), "expected tables [menu.<name>]"))?;
        for (name, item) in table.iter() {
            let key_line = line(table.get_key_value(name).and_then(|(k, _)| k.span())).or(line(item.span()));
            if !catalog::valid_menu_name(name) {
                return Err(problem(key_line, Some(format!("menu.{name}")), "a menu name is made of a-z, 0-9 and _, 32 at most"));
            }
            if !BUILTIN_MENUS.contains(&name) {
                user_menus.insert(name.to_string());
            }
            layer.menu_lines.insert(name.to_string(), key_line);
        }
    }

    for (name, item) in root.iter() {
        let key_line = line(root.get_key_value(name).and_then(|(k, _)| k.span())).or(line(item.span()));
        match name {
            "version" => match item.as_integer() {
                Some(FORMAT_VERSION) => {}
                _ => return Err(problem(key_line, Some("version".into()), format!("expected {FORMAT_VERSION}"))),
            },
            "menu" => {
                // Checked above to be a table of tables.
                let table = item.as_table_like().expect("checked above");
                for (menu, item) in table.iter() {
                    let path = format!("menu.{menu}");
                    let rows = item
                        .as_table_like()
                        .ok_or_else(|| problem(layer.menu_lines[menu], Some(path.clone()), "expected a table of keys"))?;
                    let bindings = read_table(rows, &path, Where::Menu, &user_menus, text, &mut layer.warnings)?;
                    layer.menus.push((menu.to_string(), bindings));
                }
            }
            _ => {
                let context = Context::from_name(name).ok_or_else(|| {
                    let names: Vec<&str> = Context::ALL.iter().map(|c| c.name()).collect();
                    problem(key_line, Some(name.into()), format!("unknown context; the contexts are {}, and menu.<name>", names.join(", ")))
                })?;
                let rows = item.as_table_like().ok_or_else(|| problem(key_line, Some(name.into()), "expected a table of keys"))?;
                let bindings = read_table(rows, name, Where::Context(context), &user_menus, text, &mut layer.warnings)?;
                layer.contexts.push((context, bindings));
            }
        }
    }
    Ok(layer)
}

/// The kind of table being read.
#[derive(Clone, Copy)]
enum Where {
    Context(Context),
    Menu,
}

/// Reads the lines of one table: `key = "action"`.
fn read_table(
    table: &dyn TableLike,
    path: &str,
    place: Where,
    user_menus: &BTreeSet<String>,
    text: &str,
    warnings: &mut Vec<Problem>,
) -> Result<Vec<Binding>, Problem> {
    let mut out: Vec<Binding> = Vec::new();
    // The key as written, by chord: `J` and `shift-j` are two TOML keys but
    // one key of the keyboard.
    let mut written: BTreeMap<Chord, String> = BTreeMap::new();
    for (key, item) in table.iter() {
        let line = table
            .get_key_value(key)
            .and_then(|(k, _)| k.span())
            .or(item.span())
            .map(|s| config::line_of(text, s.start));
        let dotted = format!("{path}.{key}");
        let err = |message: String| problem(line, Some(dotted.clone()), message);

        let chord = Chord::parse(key).map_err(err)?;
        if let Some(first) = written.insert(chord, key.to_string()) {
            return Err(err(format!("{key:?} is the same key as {first:?}, already in this table")));
        }
        if chord.is_reserved() {
            return Err(err(format!("{chord} cannot be changed: Escape always goes back, and ⌘Q always quits")));
        }
        if let Where::Context(c) = place {
            if c.is_typing() && chord.types_character() {
                return Err(err(format!(
                    "{chord} types a character, and text is typed in [{}]: add ctrl- or cmd-, or use a named key",
                    c.name()
                )));
            }
            // The keys a text needs for itself (`raccourcis.md`, section
            // 5.3): in a reply, those of its editor (but Tab, which goes
            // from field to field outside the text); in a line, ← → ⌫ ⌦.
            // `everywhere` applies in a reply: it takes neither.
            let editor = match c {
                Context::Everywhere => chord.edits_text(),
                Context::Compose => chord.edits_text() && chord.key != notation::Key::Named(notation::Named::Tab),
                Context::LineEdit | Context::Picker | Context::ContactFilter => chord.edits_line(),
                _ => false,
            };
            if editor {
                return Err(err(format!("{chord} edits the text in [{}]: it cannot be changed there", c.name())));
            }
        }

        let Some(id) = item.as_str() else {
            return Err(err("expected the name of an action, in quotes, or \"none\"".into()));
        };
        if id == "none" {
            out.push(Binding { chord, action: None });
            continue;
        }
        let scope = match (catalog::action(id), catalog::menu_name(id)) {
            (Some(a), _) => Some((a.scope, a.in_menu(), a.opens_menu())),
            (None, Some(menu)) if user_menus.contains(menu) => Some((catalog::USER_MENU_SCOPE, false, true)),
            _ => None,
        };
        let Some((scope, in_menu, opens_menu)) = scope else {
            // A newer NiceMailer may know it: the file stays readable, the
            // line is skipped (`raccourcis.md`, section 3).
            let what = if catalog::menu_name(id).is_some() { "no menu of that name" } else { "unknown action" };
            warnings.push(problem(line, Some(dotted.clone()), format!("{what} {id:?}: the line is ignored")));
            continue;
        };
        match place {
            Where::Context(c) if !c.within(scope) => {
                return Err(err(format!("{id} cannot be bound in [{}]: it belongs to [{}]", c.name(), scope.name())));
            }
            Where::Menu if opens_menu => return Err(err(format!("{id} opens a menu, and menus do not nest"))),
            Where::Menu if !in_menu => return Err(err(format!("{id} cannot be a row of a menu"))),
            _ => {}
        }
        out.push(Binding { chord, action: Some(id.to_string()) });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Merging and the checks of the result
// ---------------------------------------------------------------------------

/// Applies `layer` to `keymap` (`raccourcis.md`, sections 5 and 7).
///
/// In a context, a key already there gets the new action, in place; a new
/// key is added. A context may give one action several keys (`j` and
/// `down`), so the default key of an action stays when the file gives it
/// another: the file's key is put before it, as the app shows the first
/// key of an action (the help, the status bar, the targets of Quarantine).
///
/// In a menu, a row is an action, shown once with its key: a key the file
/// gives an action **replaces** the default key of that action in that
/// menu, at its place (`f = "move.todo"` in `[menu.move]` takes the row of
/// `t`, and `t` no longer moves to Todo). A key already in the menu gets the
/// new action in place, `"none"` removes the row, and a row for an action
/// the menu did not have is added at the end. `defaults`: the layer is the
/// file of defaults (its menus are the built-in ones).
fn merge(keymap: &mut Keymap, layer: Layer, defaults: bool) {
    for (context, bindings) in layer.contexts {
        let list = keymap.contexts.entry(context).or_default();
        for b in bindings {
            if let Some(x) = list.iter_mut().find(|x| x.chord == b.chord) {
                x.action = b.action;
                continue;
            }
            // The user's key before the first key of the same action, if
            // it has one; the defaults keep their own order (`j`, then
            // `down`).
            let at = if defaults {
                None
            } else {
                b.action.as_ref().and_then(|a| list.iter().position(|x| x.action.as_ref() == Some(a)))
            };
            match at {
                Some(i) => list.insert(i, b),
                None => list.push(b),
            }
        }
    }
    for (name, bindings) in layer.menus {
        let index = match keymap.menus.iter().position(|m| m.name == name) {
            Some(i) => i,
            None => {
                keymap.menus.push(Menu { builtin: defaults || BUILTIN_MENUS.contains(&name.as_str()), name, entries: Vec::new() });
                keymap.menus.len() - 1
            }
        };
        let menu = &mut keymap.menus[index];
        menu.entries = merge_menu(std::mem::take(&mut menu.entries), &bindings);
    }
}

/// The rows of a menu once the file's `bindings` are applied to `rows`
/// (see [`merge`]). The menu is rebuilt in one pass over its rows rather
/// than changed in place: a row of the file can both take the key of one
/// row and replace the key of another (`b = "goto.drafts"` takes the key of
/// Banned and replaces `r` of Drafts), and the rebuild keeps the order of
/// the rows simple to follow. A menu has a few dozen rows at most.
fn merge_menu(rows: Vec<(Chord, String)>, bindings: &[Binding]) -> Vec<(Chord, String)> {
    // The file's rows, by key and by action.
    let by_chord: BTreeMap<Chord, Option<&String>> = bindings.iter().map(|b| (b.chord, b.action.as_ref())).collect();
    let actions: BTreeSet<&String> = bindings.iter().filter_map(|b| b.action.as_ref()).collect();
    // The file's rows already placed. Rust note: `bindings` is borrowed
    // (`&[Binding]`), so the rows are cloned into the new menu.
    let mut placed: BTreeSet<Chord> = BTreeSet::new();
    let mut out: Vec<(Chord, String)> = Vec::new();
    let place = |b: &Binding, out: &mut Vec<(Chord, String)>, placed: &mut BTreeSet<Chord>| {
        if let Some(a) = &b.action
            && placed.insert(b.chord)
        {
            out.push((b.chord, a.clone()));
        }
    };
    for (chord, action) in rows {
        if let Some(new) = by_chord.get(&chord) {
            // The key is the file's: its action (all the file's keys for
            // it, together), or nothing for a `"none"`.
            if let Some(a) = new {
                for b in bindings.iter().filter(|b| b.action.as_ref() == Some(*a)) {
                    place(b, &mut out, &mut placed);
                }
            }
        } else if actions.contains(&action) {
            // The file gave this action other keys: they take this row.
            for b in bindings.iter().filter(|b| b.action.as_ref() == Some(&action)) {
                place(b, &mut out, &mut placed);
            }
        } else {
            out.push((chord, action));
        }
    }
    // The actions the menu did not have, in the order of the file.
    for b in bindings {
        place(b, &mut out, &mut placed);
    }
    out
}

/// The checks that need the whole keymap (`raccourcis.md`, section 7): a
/// menu left without a row, or a menu of the user that no key opens, is an
/// error; a built-in menu that no key opens any more is a warning (a click
/// on its block of the status bar still opens it).
fn check_menus(parsed: &mut Parsed, menu_lines: &BTreeMap<String, Option<u32>>) -> Result<(), Problem> {
    for menu in &parsed.keymap.menus {
        let line = menu_lines.get(&menu.name).copied().flatten();
        let key = Some(format!("menu.{}", menu.name));
        if menu.entries.is_empty() {
            return Err(problem(line, key, "this menu has no row left"));
        }
        if !parsed.keymap.opens(&menu.name) {
            if menu.builtin {
                parsed.warnings.push(problem(
                    line,
                    key,
                    format!("no key opens menu.{} any more; its block of the status bar still does", menu.name),
                ));
            } else {
                return Err(problem(line, key, format!("no key opens this menu: bind \"menu.{}\" to a key", menu.name)));
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The texts of the file
// ---------------------------------------------------------------------------

/// The output of `nicemailer keymap defaults`: every default key, active.
pub fn defaults_file() -> String {
    format!(
        "# NiceMailer keys: every key at its default (docs/keymap.md).\n\
         # A real keymap.toml needs only the keys that differ.\n{DEFAULTS}"
    )
}

/// The `keymap.toml` the core creates when there is none: the defaults,
/// every key commented out, the table headers kept, so that removing the
/// `# ` of a line is all it takes to change a key (as `config_template`).
pub fn template() -> String {
    let body: String = DEFAULTS
        .lines()
        .map(|line| {
            if line.is_empty() || line.starts_with('#') || line.starts_with('[') || line.starts_with("version ") {
                line.to_string()
            } else {
                format!("# {line}")
            }
        })
        .map(|l| l + "\n")
        .collect();
    format!(
        "# NiceMailer keys (docs/keymap.md).\n\
         #\n\
         # Each line binds a key to an action, in a table per context ([mail],\n\
         # [compose]…) or per menu ([menu.save]). Every default key is listed\n\
         # below, commented out. To change one, remove the \"# \" in front of it\n\
         # and edit the action, or write \"none\" to remove the key. New keys and\n\
         # new menus can be added to any table. `nicemailer keymap check`\n\
         # validates the file; NiceMailer applies it as soon as it is saved.\n{body}"
    )
}

// ---------------------------------------------------------------------------
// What the app receives
// ---------------------------------------------------------------------------

/// The keymap as the contract sends it (`keymap_get`).
pub fn view(keymap: &Keymap) -> (BTreeMap<String, Vec<nm_api::KeyBinding>>, Vec<nm_api::KeymapMenu>) {
    let binding = |chord: &Chord, action: Option<&str>| nm_api::KeyBinding {
        key: chord.canonical(),
        action: action.unwrap_or("none").to_string(),
    };
    let contexts = keymap
        .contexts
        .iter()
        .map(|(c, list)| (c.name().to_string(), list.iter().map(|b| binding(&b.chord, b.action.as_deref())).collect()))
        .collect();
    let menus = keymap
        .menus
        .iter()
        .map(|m| nm_api::KeymapMenu {
            name: m.name.clone(),
            builtin: m.builtin,
            entries: m.entries.iter().map(|(c, a)| binding(c, Some(a))).collect(),
        })
        .collect();
    (contexts, menus)
}

/// The JSON the app embeds (`Resources/keymap-defaults.json`): the keymap
/// of the defaults and every action of the catalogue. The app uses it in
/// its tests (`FakeCore`) and until the core answers; a test of the app
/// checks that it handles every action of the list. A test below checks
/// that the file of the app is this text.
pub fn defaults_json() -> String {
    let (contexts, menus) = view(defaults());
    let actions: Vec<&str> = catalog::ACTIONS.iter().map(|a| a.id).collect();
    let value = serde_json::json!({ "actions": actions, "contexts": contexts, "menus": menus });
    serde_json::to_string_pretty(&value).expect("JSON") + "\n"
}

// ---------------------------------------------------------------------------
// The file, followed while the core runs
// ---------------------------------------------------------------------------

/// The keymap in effect in the core, and what was last read.
pub struct KeymapFile {
    path: PathBuf,
    keymap: Keymap,
    warnings: Vec<Problem>,
    /// Why the file on disk is refused; the keymap in effect is then the
    /// previous one (or the defaults, at start).
    error: Option<Problem>,
    /// The text last read (`None`: missing), to skip a reload that changes
    /// nothing (one save gives several file system events).
    last_text: Option<Option<String>>,
}

impl KeymapFile {
    pub fn new(path: PathBuf) -> KeymapFile {
        KeymapFile { path, keymap: defaults().clone(), warnings: Vec::new(), error: None, last_text: None }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn keymap(&self) -> &Keymap {
        &self.keymap
    }

    pub fn warnings(&self) -> &[Problem] {
        &self.warnings
    }

    pub fn error(&self) -> Option<&Problem> {
        self.error.as_ref()
    }

    /// Reads the file again. Returns the events to emit: `KeymapChanged`
    /// when a new keymap is applied (or the file is valid again after an
    /// error), `ConfigError` when it is refused, nothing when it did not
    /// change. At start (`startup`), a valid file announces nothing: the
    /// app asks for the keymap when it connects.
    pub fn reload(&mut self, startup: bool) -> Vec<nm_api::Event> {
        let text = match config::read(&self.path, KEYMAP_FILE) {
            Ok(t) => t,
            Err(p) => {
                self.last_text = None;
                return self.refuse(p);
            }
        };
        if self.last_text.as_ref() == Some(&text) {
            return Vec::new();
        }
        self.last_text = Some(text.clone());
        let parsed = match parse(text.as_deref().unwrap_or("")) {
            Ok(p) => p,
            Err(p) => return self.refuse(p),
        };
        let had_error = self.error.take().is_some();
        let changed = parsed.keymap != self.keymap || parsed.warnings != self.warnings;
        self.keymap = parsed.keymap;
        self.warnings = parsed.warnings;
        if !startup && (changed || had_error) { vec![nm_api::Event::KeymapChanged {}] } else { Vec::new() }
    }

    /// Keeps the keymap in effect and reports `problem`, once.
    fn refuse(&mut self, problem: Problem) -> Vec<nm_api::Event> {
        if self.error.as_ref() == Some(&problem) {
            return Vec::new();
        }
        self.error = Some(problem.clone());
        vec![nm_api::Event::ConfigError { file: problem.file.to_string(), line: problem.line, key: problem.key, message: problem.message }]
    }
}

#[cfg(test)]
mod tests;
