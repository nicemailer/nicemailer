//! Themes of the app: `themes/<name>.toml`, next to `config.toml`
//! (`specifications/themes.md`).
//!
//! A theme sets the presentation of the app (colors, fonts, proportions,
//! animations) and chooses among the variants the app can draw (status
//! bar or toolbar, blocks shown…); the keys it can set are those of
//! [`catalog::KEYS`]. Three themes are built in, embedded in the binary
//! (`geek.toml`, `clean.toml`, `regular.toml` next to this source); a file
//! of the same name modifies one of them, any other file is a theme of the
//! user that extends another one:
//!
//! ```toml
//! version = 1
//! extends = "clean"
//!
//! [colors]
//! background = { light = "#F6F0E4", dark = "#221E18" }
//! ```
//!
//! [`parse`] reads one file into a [`Layer`], what it says; [`resolve`]
//! walks the chain of inheritance down to a built-in theme and lays the
//! layers over it, which gives a value to every key. The app receives that
//! result ([`view`], `theme_get`): it never reads a file nor walks the
//! inheritance itself.
//!
//! [`ThemeSet`] follows the directory: it reads the files again when they
//! change, keeps the last valid version of each one (an invalid file is
//! refused whole, like `keymap.toml`, and its previous version stays), and
//! says which events to emit.
//!
//! A theme is data only (`themes.md`, section 2): nothing here reads a path
//! or an address a file gives, so a theme received from someone can be put
//! in the directory without risk.
//!
//! Speed: a theme is a few dozen lines, read only when a file of the
//! directory changes; the app turns the result into its own values once.

pub mod catalog;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use toml_edit::{Item, TableLike};

use crate::config::{self, Problem};
use catalog::{BUILTINS, DEFAULT_THEME, KEYS, Key, Kind};

/// Name of the directory of the themes, always next to `config.toml`.
pub const THEMES_DIR: &str = "themes";

/// Version of the file format: the `version` key.
pub const FORMAT_VERSION: i64 = 1;

/// A bigger file is refused: a theme is a few kilobytes, and the bound
/// keeps a file that is not one from being read whole (`themes.md`,
/// section 3).
pub const MAX_FILE_BYTES: u64 = 64 * 1024;

/// The longest chain of inheritance, built-in theme included.
pub const MAX_DEPTH: usize = 8;

/// The built-in themes. Rust note: `include_str!` embeds each file in the
/// binary when it is compiled; nothing is installed or looked for at run
/// time.
const BUILTIN_TEXTS: &[(&str, &str)] = &[
    ("geek", include_str!("geek.toml")),
    ("clean", include_str!("clean.toml")),
    ("regular", include_str!("regular.toml")),
];

/// Path of the directory of the themes: next to `config_path`, whatever its
/// name, so that `NICEMAILER_CONFIG` moves it with the other files.
pub fn dir(config_path: &Path) -> PathBuf {
    config_path.with_file_name(THEMES_DIR)
}

/// How problems name the file of `name`: `themes/clean.toml`.
pub fn file_label(name: &str) -> String {
    format!("{THEMES_DIR}/{name}.toml")
}

/// Whether `name` is a built-in theme.
pub fn is_builtin(name: &str) -> bool {
    BUILTINS.contains(&name)
}

// ---------------------------------------------------------------------------
// Values
// ---------------------------------------------------------------------------

/// A color of a theme (`themes.md`, section 5.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Color {
    /// `#RRGGBB` or `#RRGGBBAA`, in capitals, for the light and the dark
    /// appearance; a single color in the file gives the same twice.
    Pair { light: String, dark: String },
    /// A color of macOS, by its name in [`catalog::SYSTEM_COLORS`].
    System(String),
}

/// The value of a key, of the type its [`Kind`] says.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// A choice or a font.
    Text(String),
    Bool(bool),
    Int(i64),
    Float(f64),
    Color(Color),
    Blocks(Vec<String>),
}

/// The values of a theme, by dotted key (`colors.background`).
pub type Values = BTreeMap<String, Value>;

/// What one file says, before it is laid over its parent.
#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    /// `extends`, with its line.
    pub extends: Option<(String, Option<u32>)>,
    pub values: Values,
    /// Unknown keys, ignored.
    pub warnings: Vec<Problem>,
}

/// A theme resolved: every key of the catalog has its value.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    pub name: String,
    /// From the theme to the built-in one it rests on.
    pub chain: Vec<String>,
    pub values: Values,
    /// Those of the files of the chain, and a missing `extends`.
    pub warnings: Vec<Problem>,
}

fn problem(name: &str, line: Option<u32>, key: Option<String>, message: impl Into<String>) -> Problem {
    Problem { file: file_label(name), line, key, message: message.into() }
}

// ---------------------------------------------------------------------------
// Reading a file
// ---------------------------------------------------------------------------

/// Reads the text of the file of the theme `name` (`""` for an empty file);
/// the first problem refuses the whole file. Each value is checked against
/// the catalog; nothing is resolved here.
pub fn parse(name: &str, text: &str) -> Result<Layer, Problem> {
    let doc = toml_edit::Document::parse(text).map_err(|e| {
        problem(
            name,
            e.span().map(|s| config::line_of(text, s.start)),
            None,
            e.message().lines().next().unwrap_or("invalid TOML").to_string(),
        )
    })?;
    let line = |span: Option<std::ops::Range<usize>>| span.map(|s| config::line_of(text, s.start));
    let root = doc.as_table();
    let mut layer = Layer { extends: None, values: BTreeMap::new(), warnings: Vec::new() };

    for (top, item) in root.iter() {
        let key_line = line(root.get_key_value(top).and_then(|(k, _)| k.span())).or(line(item.span()));
        match top {
            "version" => {
                if item.as_integer() != Some(FORMAT_VERSION) {
                    return Err(problem(name, key_line, Some("version".into()), format!("expected {FORMAT_VERSION}")));
                }
            }
            "extends" => {
                let parent = item
                    .as_str()
                    .ok_or_else(|| problem(name, key_line, Some("extends".into()), "expected the name of a theme"))?;
                // A file named like a built-in theme modifies it: its
                // parent is that theme, and naming another one would make
                // `clean` something else than clean (section 4).
                if is_builtin(name) && parent != name {
                    return Err(problem(
                        name,
                        key_line,
                        Some("extends".into()),
                        format!("this file modifies the built-in theme {name}: it cannot extend another theme"),
                    ));
                }
                if !catalog::valid_name(parent) {
                    return Err(problem(name, key_line, Some("extends".into()), "a theme is named by lowercase letters, digits and hyphens"));
                }
                if parent == name && !is_builtin(name) {
                    return Err(problem(name, key_line, Some("extends".into()), "a theme cannot extend itself"));
                }
                layer.extends = Some((parent.to_string(), key_line));
            }
            table_name if catalog::TABLES.contains(&table_name) => {
                let table = item
                    .as_table_like()
                    .ok_or_else(|| problem(name, key_line, Some(table_name.into()), "expected a table"))?;
                read_table(name, table_name, table, text, &mut layer)?;
            }
            other if item.is_table_like() => {
                return Err(problem(
                    name,
                    key_line,
                    Some(other.into()),
                    format!("unknown table; the tables are {}", catalog::TABLES.join(", ")),
                ));
            }
            other => layer.warnings.push(problem(name, key_line, Some(other.into()), "unknown key, ignored")),
        }
    }
    Ok(layer)
}

/// Reads the keys of one table of a file into `layer`.
fn read_table(name: &str, table_name: &str, table: &dyn TableLike, text: &str, layer: &mut Layer) -> Result<(), Problem> {
    for (k, item) in table.iter() {
        let span = table.get_key_value(k).and_then(|(key, _)| key.span()).or_else(|| item.span());
        let key_line = span.map(|s| config::line_of(text, s.start));
        let dotted = format!("{table_name}.{k}");
        let Some(key) = catalog::key(table_name, k) else {
            layer.warnings.push(problem(name, key_line, Some(dotted), "unknown key, ignored"));
            continue;
        };
        let value = read_value(key, item).map_err(|message| problem(name, key_line, Some(dotted.clone()), message))?;
        layer.values.insert(dotted, value);
    }
    Ok(())
}

/// The value of `item` for `key`, or why it is refused.
fn read_value(key: &Key, item: &Item) -> Result<Value, String> {
    match key.kind {
        Kind::Choice(choices) => {
            let v = item.as_str().ok_or_else(|| format!("expected one of: {}", choices.join(", ")))?;
            if choices.contains(&v) { Ok(Value::Text(v.into())) } else { Err(format!("must be one of: {}", choices.join(", "))) }
        }
        Kind::Bool => item.as_bool().map(Value::Bool).ok_or_else(|| "expected true or false".into()),
        Kind::Int(min, max) => {
            let v = item.as_integer().ok_or_else(|| format!("expected an integer from {min} to {max}"))?;
            if (min..=max).contains(&v) { Ok(Value::Int(v)) } else { Err(format!("must be from {min} to {max}")) }
        }
        Kind::IntOrZero(min, max) => {
            let v = item.as_integer().ok_or_else(|| format!("expected 0, or an integer from {min} to {max}"))?;
            if v == 0 || (min..=max).contains(&v) { Ok(Value::Int(v)) } else { Err(format!("must be 0, or from {min} to {max}")) }
        }
        Kind::Float(min, max) => {
            // An integer is accepted: `reading_scale = 1` means 1.0.
            let v = item
                .as_float()
                .or_else(|| item.as_integer().map(|i| i as f64))
                .ok_or_else(|| format!("expected a number from {min} to {max}"))?;
            if v.is_finite() && v >= min && v <= max { Ok(Value::Float(v)) } else { Err(format!("must be from {min} to {max}")) }
        }
        Kind::Color => read_color(item).map(Value::Color),
        Kind::Font => {
            let v = item.as_str().ok_or("expected the name of a font, or system, system-mono, system-serif, system-rounded")?;
            let v = v.trim();
            if v.is_empty() || v.chars().count() > 100 || v.chars().any(char::is_control) {
                return Err("expected the name of a font (100 characters at most)".into());
            }
            Ok(Value::Text(v.into()))
        }
        Kind::Blocks => read_blocks(item).map(Value::Blocks),
    }
}

/// A color: `"#RRGGBB"`, `"#RRGGBBAA"`, `"system.<name>"`, or a table
/// `{ light = …, dark = … }` of two hexadecimal colors (section 5.1).
fn read_color(item: &Item) -> Result<Color, String> {
    const EXPECTED: &str = "expected \"#RRGGBB\", \"#RRGGBBAA\", \"system.<name>\" or { light = \"#…\", dark = \"#…\" }";
    if let Some(s) = item.as_str() {
        if let Some(system) = s.strip_prefix("system.") {
            return if catalog::SYSTEM_COLORS.contains(&system) {
                Ok(Color::System(system.into()))
            } else {
                Err(format!("unknown color of the system; they are {}", catalog::SYSTEM_COLORS.join(", ")))
            };
        }
        let hex = hex_color(s).ok_or(EXPECTED)?;
        return Ok(Color::Pair { light: hex.clone(), dark: hex });
    }
    let table = item.as_table_like().ok_or(EXPECTED)?;
    for (k, _) in table.iter() {
        if k != "light" && k != "dark" {
            return Err(format!("unknown key {k}: a pair of colors has light and dark"));
        }
    }
    let side = |which: &str| -> Result<String, String> {
        let v = table.get(which).ok_or_else(|| format!("a pair of colors needs both light and dark; {which} is missing"))?;
        v.as_str().and_then(hex_color).ok_or_else(|| format!("{which}: expected \"#RRGGBB\" or \"#RRGGBBAA\""))
    };
    Ok(Color::Pair { light: side("light")?, dark: side("dark")? })
}

/// `#rrggbb` or `#rrggbbaa`, in capitals; `None` for anything else.
fn hex_color(s: &str) -> Option<String> {
    let digits = s.strip_prefix('#')?;
    if (digits.len() == 6 || digits.len() == 8) && digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        Some(format!("#{}", digits.to_ascii_uppercase()))
    } else {
        None
    }
}

/// The list of blocks (section 6.1): known names, each at most once, the
/// separators `|` (several allowed) and `->` (once).
fn read_blocks(item: &Item) -> Result<Vec<String>, String> {
    let array = item.as_array().ok_or("expected a list of blocks: [\"account\", \"folder\", …]")?;
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for v in array.iter() {
        let b = v.as_str().ok_or("expected a list of names of blocks")?;
        let known = catalog::BLOCKS.contains(&b) || b == catalog::BLOCK_GAP || b == catalog::BLOCK_SPRING;
        if !known {
            return Err(format!(
                "unknown block {b:?}; the blocks are {}, and the separators \"{}\" and \"{}\"",
                catalog::BLOCKS.join(", "),
                catalog::BLOCK_GAP,
                catalog::BLOCK_SPRING
            ));
        }
        if b != catalog::BLOCK_GAP && !seen.insert(b) {
            return Err(format!("the block {b:?} is listed twice"));
        }
        out.push(b.to_string());
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// The built-in themes
// ---------------------------------------------------------------------------

/// The values of the built-in theme `name`, read once. Each one gives a
/// value to every key of the catalog: it is the base the inheritance lays
/// the files over (checked by a test, and here at the first use).
///
/// Rust note: `OnceLock` runs its closure the first time `get_or_init` is
/// called, from whichever thread, and hands out the same value after.
pub fn builtin(name: &str) -> Option<&'static Values> {
    static BUILT: OnceLock<BTreeMap<&'static str, Values>> = OnceLock::new();
    BUILT
        .get_or_init(|| {
            BUILTIN_TEXTS
                .iter()
                .map(|(name, text)| {
                    let layer = parse(name, text).unwrap_or_else(|p| panic!("built-in theme {name} is invalid: {p}"));
                    assert!(layer.warnings.is_empty(), "built-in theme {name} has warnings: {:?}", layer.warnings);
                    for key in KEYS {
                        assert!(layer.values.contains_key(&key.dotted()), "built-in theme {name} has no {}", key.dotted());
                    }
                    (*name, layer.values)
                })
                .collect()
        })
        .get(name)
}

// ---------------------------------------------------------------------------
// Inheritance
// ---------------------------------------------------------------------------

/// Resolves the theme `name` with the `layers` of the files (the last
/// valid version of each one): walks its chain down to a built-in theme,
/// then lays each layer over its parent, key by key (section 4).
pub fn resolve(name: &str, layers: &BTreeMap<String, Layer>) -> Result<Resolved, Problem> {
    let mut chain: Vec<String> = Vec::new();
    // The layers to lay, from the theme to its root.
    let mut stack: Vec<&Layer> = Vec::new();
    let mut warnings = Vec::new();
    let mut current = name.to_string();
    // The file whose `extends` led to `current`, with its line: where a
    // problem of the chain is reported.
    let mut from: Option<(String, Option<u32>)> = None;
    let base = loop {
        if chain.contains(&current) {
            let (file, line) = from.clone().unwrap_or((name.to_string(), None));
            return Err(problem(&file, line, Some("extends".into()), format!("circular inheritance: {} -> {current}", chain.join(" -> "))));
        }
        if chain.len() == MAX_DEPTH {
            let (file, line) = from.clone().unwrap_or((name.to_string(), None));
            return Err(problem(&file, line, Some("extends".into()), format!("more than {MAX_DEPTH} levels of inheritance")));
        }
        chain.push(current.clone());
        if let Some(values) = builtin(&current) {
            if let Some(layer) = layers.get(&current) {
                stack.push(layer);
            }
            break values;
        }
        let Some(layer) = layers.get(&current) else {
            return Err(match &from {
                Some((file, line)) => problem(file, *line, Some("extends".into()), format!("no theme named {current}")),
                None => problem(name, None, None, format!("no theme named {current}")),
            });
        };
        stack.push(layer);
        let parent = match &layer.extends {
            Some((parent, line)) => {
                from = Some((current.clone(), *line));
                parent.clone()
            }
            None => {
                warnings.push(problem(&current, None, Some("extends".into()), format!("no extends: the parent is {DEFAULT_THEME}")));
                from = Some((current.clone(), None));
                DEFAULT_THEME.to_string()
            }
        };
        current = parent;
    };
    let mut values = base.clone();
    for layer in stack.iter().rev() {
        for (k, v) in &layer.values {
            values.insert(k.clone(), v.clone());
        }
    }
    for layer in &stack {
        warnings.extend(layer.warnings.iter().cloned());
    }
    Ok(Resolved { name: name.to_string(), chain, values, warnings })
}

// ---------------------------------------------------------------------------
// Contrast
// ---------------------------------------------------------------------------

/// Relative luminance of `#RRGGBB…` (WCAG 2), alpha ignored.
fn luminance(hex: &str) -> f64 {
    let channel = |i: usize| {
        let v = u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0) as f64 / 255.0;
        if v <= 0.039_28 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * channel(1) + 0.7152 * channel(3) + 0.0722 * channel(5)
}

/// Contrast ratio of two colors (WCAG 2), from 1 to 21.
pub fn contrast(a: &str, b: &str) -> f64 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// A warning for each text color below [`catalog::MIN_CONTRAST`] on its
/// background, in each appearance (section 6.5). A color of the system is
/// not checked (macOS answers for it), nor a color with transparency, whose
/// contrast depends on what is behind it.
pub fn contrast_warnings(name: &str, values: &Values) -> Vec<Problem> {
    let mut out = Vec::new();
    let pair = |k: &str| match values.get(&format!("colors.{k}")) {
        Some(Value::Color(Color::Pair { light, dark })) if light.len() == 7 && dark.len() == 7 => Some((light.clone(), dark.clone())),
        _ => None,
    };
    for (fg, bg) in catalog::CONTRAST_PAIRS {
        let (Some(f), Some(b)) = (pair(fg), pair(bg)) else { continue };
        for (appearance, f, b) in [("light", &f.0, &b.0), ("dark", &f.1, &b.1)] {
            let ratio = contrast(f, b);
            if ratio < catalog::MIN_CONTRAST {
                out.push(problem(
                    name,
                    None,
                    Some(format!("colors.{fg}")),
                    format!(
                        "contrast {:.1}:1 on colors.{bg} in the {appearance} appearance, below {}:1 (WCAG AA)",
                        (ratio * 10.0).floor() / 10.0,
                        catalog::MIN_CONTRAST
                    ),
                ));
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// The view sent to the app
// ---------------------------------------------------------------------------

fn text(values: &Values, key: &str) -> String {
    match values.get(key) {
        Some(Value::Text(s)) => s.clone(),
        other => panic!("theme key {key}: expected a text, found {other:?}"),
    }
}

fn boolean(values: &Values, key: &str) -> bool {
    match values.get(key) {
        Some(Value::Bool(b)) => *b,
        other => panic!("theme key {key}: expected a boolean, found {other:?}"),
    }
}

fn int(values: &Values, key: &str) -> i64 {
    match values.get(key) {
        Some(Value::Int(i)) => *i,
        other => panic!("theme key {key}: expected an integer, found {other:?}"),
    }
}

fn float(values: &Values, key: &str) -> f64 {
    match values.get(key) {
        Some(Value::Float(f)) => *f,
        other => panic!("theme key {key}: expected a number, found {other:?}"),
    }
}

/// The view of a resolved theme (`theme_get`). The keys are those of the
/// catalog, all present in a resolved theme: a missing one is a bug of the
/// core, hence the panics of the accessors above.
pub fn view(resolved: &Resolved, dir: &Path, error: Option<&Problem>, warnings: &[Problem]) -> nm_api::ThemeView {
    let v = &resolved.values;
    let blocks = match v.get("window.blocks") {
        Some(Value::Blocks(b)) => b.clone(),
        other => panic!("theme key window.blocks: expected blocks, found {other:?}"),
    };
    let colors = KEYS
        .iter()
        .filter(|k| k.table == "colors")
        .map(|k| {
            let c = match v.get(&k.dotted()) {
                Some(Value::Color(Color::Pair { light, dark })) => nm_api::ThemeColor::Pair { light: light.clone(), dark: dark.clone() },
                Some(Value::Color(Color::System(s))) => nm_api::ThemeColor::System { system: s.clone() },
                other => panic!("theme key {}: expected a color, found {other:?}", k.dotted()),
            };
            (k.name.to_string(), c)
        })
        .collect();
    nm_api::ThemeView {
        name: resolved.name.clone(),
        chain: resolved.chain.clone(),
        path: dir.display().to_string(),
        window: nm_api::ThemeWindow {
            bar_type: text(v, "window.bar_type"),
            status_bar_position: text(v, "window.status_bar_position"),
            title_bar: text(v, "window.title_bar"),
            key_hints: boolean(v, "window.key_hints"),
            block_labels: boolean(v, "window.block_labels"),
            bar_height: int(v, "window.bar_height") as u32,
            bar_size: text(v, "window.bar_size"),
            block_style: text(v, "window.block_style"),
            bar_line: boolean(v, "window.bar_line"),
            blocks,
            default_columns: int(v, "window.default_columns") as u8,
        },
        fonts: nm_api::ThemeFonts {
            interface: text(v, "fonts.interface"),
            reading: text(v, "fonts.reading"),
            reading_scale: float(v, "fonts.reading_scale"),
            reading_line_spacing: int(v, "fonts.reading_line_spacing") as u32,
            title_scale: float(v, "fonts.title_scale"),
        },
        layout: nm_api::ThemeLayout {
            row_height: int(v, "layout.row_height") as u32,
            row_radius: int(v, "layout.row_radius") as u32,
            row_inset: int(v, "layout.row_inset") as u32,
            zebra: boolean(v, "layout.zebra"),
            menu_row_height: int(v, "layout.menu_row_height") as u32,
            popup_radius: int(v, "layout.popup_radius") as u32,
            popup_shadow: boolean(v, "layout.popup_shadow"),
            popup_padding: int(v, "layout.popup_padding") as u32,
            menu_keys: text(v, "layout.menu_keys"),
            message_cards: boolean(v, "layout.message_cards"),
            avatars: boolean(v, "layout.avatars"),
            reader_max_width: int(v, "layout.reader_max_width") as u32,
            reader_title_padding: int(v, "layout.reader_title_padding") as u32,
            scrollers: text(v, "layout.scrollers"),
            separators: text(v, "layout.separators"),
        },
        motion: nm_api::ThemeMotion {
            sidebar_ms: int(v, "motion.sidebar_ms") as u32,
            popup_ms: int(v, "motion.popup_ms") as u32,
        },
        colors,
        reader: nm_api::ThemeReader { body_format: text(v, "reader.body_format") },
        error: error.map(crate::problem_view),
        warnings: warnings.iter().map(crate::problem_view).collect(),
    }
}

/// The three built-in themes resolved, as the app embeds them (a JSON
/// object by name): its fallback until the core answers, and the themes of
/// `FakeCore` in the tests (`themes.md`, section 7).
pub fn builtins_json() -> String {
    let dir = Path::new("~/.config/nicemailer/themes");
    let themes: BTreeMap<&str, nm_api::ThemeView> = BUILTINS
        .iter()
        .map(|name| {
            let resolved = resolve(name, &BTreeMap::new()).expect("a built-in theme resolves");
            let warnings = contrast_warnings(name, &resolved.values);
            (*name, view(&resolved, dir, None, &warnings))
        })
        .collect();
    serde_json::to_string_pretty(&themes).expect("JSON") + "\n"
}

// ---------------------------------------------------------------------------
// Writing a theme file
// ---------------------------------------------------------------------------

/// `value` as TOML.
fn toml_value(value: &Value) -> String {
    let quoted = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
    match value {
        Value::Text(s) => quoted(s),
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        // `1.0` rather than `1`: the file then says it is a number with
        // decimals, as the comment of the key does.
        Value::Float(f) if f.fract() == 0.0 => format!("{f:.1}"),
        Value::Float(f) => f.to_string(),
        Value::Color(Color::System(s)) => quoted(&format!("system.{s}")),
        Value::Color(Color::Pair { light, dark }) if light == dark => quoted(light),
        Value::Color(Color::Pair { light, dark }) => format!("{{ light = {}, dark = {} }}", quoted(light), quoted(dark)),
        Value::Blocks(b) => format!("[{}]", b.iter().map(|s| quoted(s)).collect::<Vec<_>>().join(", ")),
    }
}

/// A theme file with every key of `values` commented out, each after the
/// line that says its role: removing the `# ` in front of a value changes
/// it (`themes.md`, section 3). `extends`, when given, is written
/// uncommented: it is what makes the file a theme of its own.
fn commented_file(header: &str, extends: Option<&str>, values: &Values) -> String {
    let mut out = String::new();
    for line in header.lines() {
        out.push_str(format!("# {line}").trim_end());
        out.push('\n');
    }
    out.push_str(&format!("version = {FORMAT_VERSION}\n"));
    if let Some(parent) = extends {
        out.push_str(&format!("extends = \"{parent}\"\n"));
    }
    for table in catalog::TABLES {
        out.push_str(&format!("\n[{table}]\n"));
        for key in KEYS.iter().filter(|k| k.table == *table) {
            out.push_str(&format!("# {}\n", key.role));
            out.push_str(&format!("# {} = {}\n", key.name, toml_value(&values[&key.dotted()])));
        }
    }
    out
}

/// The file the core creates for the built-in theme `name`, and what
/// `nicemailer theme defaults <name>` prints: every key commented out at
/// its value. Such a file changes nothing until a line is uncommented.
pub fn template(name: &str) -> Option<String> {
    let values = builtin(name)?;
    let header = format!(
        "NiceMailer, the built-in theme \"{name}\" (docs/themes.md).\n\
         \n\
         Every key of the theme is listed below, commented out, at its value.\n\
         To change one, remove the \"# \" in front of it and edit the value;\n\
         NiceMailer applies the file as soon as it is saved. A key left\n\
         commented keeps the value NiceMailer ships, even when a later\n\
         version changes it. `nicemailer theme check {name}` validates the file.\n\
         \n\
         A color is \"#RRGGBB\", \"#RRGGBBAA\", {{ light = \"#…\", dark = \"#…\" }}\n\
         for the two appearances of macOS, or a color of the system,\n\
         \"system.accent\". Lengths are in points for a 12 pt text: they\n\
         grow with ui.text_size of config.toml."
    );
    Some(commented_file(&header, None, values))
}

/// The file of a new theme `name` that extends `parent`, whose resolved
/// values are `values` (`nicemailer theme new`): `extends`, then every
/// key commented out at the parent's value.
pub fn new_theme_file(name: &str, parent: &str, values: &Values) -> String {
    let header = format!(
        "NiceMailer, the theme \"{name}\" (docs/themes.md), which extends \"{parent}\".\n\
         \n\
         Every key is listed below, commented out, at the value of {parent}:\n\
         a key left commented follows {parent}, even when it changes. Remove\n\
         the \"# \" in front of a key to give it a value of this theme. Choose\n\
         the theme with `theme = \"{name}\"` in the [ui] table of config.toml."
    );
    commented_file(&header, Some(parent), values)
}

// ---------------------------------------------------------------------------
// The directory
// ---------------------------------------------------------------------------

/// What the core knows of one file of the directory.
#[derive(Debug, Clone, Default)]
struct FileState {
    /// The text last read, to skip a reload that changes nothing (one save
    /// gives several file system events).
    text: Option<String>,
    /// The last valid version; kept while the file on disk is refused.
    layer: Option<Layer>,
    /// Why the file on disk is refused.
    error: Option<Problem>,
    /// Why the theme cannot be resolved (a parent missing, a cycle), with
    /// the last valid versions of the files.
    chain_error: Option<Problem>,
}

/// The theme in effect, as the app sees it.
#[derive(Debug, Clone, PartialEq)]
struct Applied {
    resolved: Resolved,
    error: Option<Problem>,
    warnings: Vec<Problem>,
}

/// The directory of the themes, followed while the core runs
/// (`themes.md`, section 3).
pub struct ThemeSet {
    dir: PathBuf,
    files: BTreeMap<String, FileState>,
    /// Files whose name cannot name a theme: ignored, with a warning.
    ignored: Vec<Problem>,
    /// `ui.theme` of `config.toml`.
    selected: String,
    applied: Applied,
}

impl ThemeSet {
    /// The set of `dir`, before any reading: the default theme, built in.
    pub fn new(dir: PathBuf, selected: &str) -> ThemeSet {
        let resolved = resolve(DEFAULT_THEME, &BTreeMap::new()).expect("the default theme resolves");
        ThemeSet {
            dir,
            files: BTreeMap::new(),
            ignored: Vec::new(),
            selected: selected.to_string(),
            applied: Applied { resolved, error: None, warnings: Vec::new() },
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn selected(&self) -> &str {
        &self.selected
    }

    /// The theme in effect, as `theme_get` gives it.
    pub fn view(&self) -> nm_api::ThemeView {
        view(&self.applied.resolved, &self.dir, self.applied.error.as_ref(), &self.applied.warnings)
    }

    /// The theme in effect: name, chain and values.
    pub fn applied(&self) -> &Resolved {
        &self.applied.resolved
    }

    /// Why a file of the chain of the theme in effect is refused.
    pub fn error(&self) -> Option<&Problem> {
        self.applied.error.as_ref()
    }

    pub fn warnings(&self) -> &[Problem] {
        &self.applied.warnings
    }

    /// The last valid version of each file.
    fn layers(&self) -> BTreeMap<String, Layer> {
        self.files.iter().filter_map(|(n, f)| f.layer.clone().map(|l| (n.clone(), l))).collect()
    }

    /// Resolves the theme `name` with the last valid version of each file:
    /// what `nicemailer theme show` prints.
    pub fn resolve(&self, name: &str) -> Result<Resolved, Problem> {
        resolve(name, &self.layers())
    }

    /// Checks `text` as the file of the theme `name`, with the other files
    /// of the directory as they are (`nicemailer theme check`): the theme
    /// it gives and every warning, contrast included, or the problem that
    /// refuses it.
    pub fn check(&self, name: &str, text: &str) -> Result<(Resolved, Vec<Problem>), Problem> {
        let mut layers = self.layers();
        layers.insert(name.to_string(), parse(name, text)?);
        let resolved = resolve(name, &layers)?;
        let mut warnings = resolved.warnings.clone();
        warnings.extend(contrast_warnings(name, &resolved.values));
        Ok((resolved, warnings))
    }

    /// Whether a theme named `name` exists: built in, or a file of the
    /// directory (valid or not).
    pub fn exists(&self, name: &str) -> bool {
        is_builtin(name) || self.files.contains_key(name)
    }

    /// The themes that can be chosen (`theme_list`).
    pub fn list(&self) -> nm_api::ThemeListView {
        let mut themes = Vec::new();
        for name in BUILTINS {
            let file = self.files.get(*name);
            // Modified only when its file changes something: the file the
            // core creates, every key commented out, leaves it as it is.
            let modified = file.is_some_and(|f| f.error.is_some() || f.layer.as_ref().is_some_and(|l| !l.values.is_empty()));
            themes.push(nm_api::ThemeEntry {
                name: name.to_string(),
                origin: if modified { "builtin_modified" } else { "builtin" }.into(),
                parent: None,
                error: file.and_then(|f| f.error.as_ref().or(f.chain_error.as_ref())).map(crate::problem_view),
            });
        }
        for (name, file) in self.files.iter().filter(|(n, _)| !is_builtin(n)) {
            let parent = file
                .layer
                .as_ref()
                .map(|l| l.extends.as_ref().map(|(p, _)| p.clone()).unwrap_or_else(|| DEFAULT_THEME.to_string()));
            let error = file
                .error
                .as_ref()
                .or(file.chain_error.as_ref())
                .cloned()
                .or_else(|| file.layer.is_none().then(|| problem(name, None, None, "the file is refused")));
            themes.push(nm_api::ThemeEntry {
                name: name.clone(),
                origin: "user".into(),
                parent,
                error: error.as_ref().map(crate::problem_view),
            });
        }
        nm_api::ThemeListView { path: self.dir.display().to_string(), selected: self.selected.clone(), themes }
    }

    /// Reads the directory again. Returns the events to emit:
    /// `ConfigError` for each file newly refused or whose chain broke, and
    /// `ThemeChanged` when the theme in effect changed. At start
    /// (`startup`), the first reading is no change: only the refusals are
    /// returned.
    pub fn reload(&mut self, startup: bool) -> Vec<nm_api::Event> {
        let mut events = Vec::new();
        let mut seen = BTreeSet::new();
        self.ignored.clear();
        for (name, path) in self.list_files() {
            let label = file_label(&name);
            if !catalog::valid_name(&name) {
                self.ignored.push(Problem {
                    file: format!("{THEMES_DIR}/{name}.toml"),
                    line: None,
                    key: None,
                    message: "ignored: a theme file is named by lowercase letters, digits and hyphens".into(),
                });
                continue;
            }
            seen.insert(name.clone());
            let state = self.files.entry(name.clone()).or_default();
            let text = match read_file(&path, &label) {
                Ok(t) => t,
                Err(p) => {
                    state.text = None;
                    if state.error.as_ref() != Some(&p) {
                        events.push(error_event(&p));
                    }
                    state.error = Some(p);
                    continue;
                }
            };
            if state.text.as_ref() == Some(&text) {
                continue;
            }
            state.text = Some(text.clone());
            match parse(&name, &text) {
                Ok(layer) => {
                    state.layer = Some(layer);
                    state.error = None;
                }
                Err(p) => {
                    if state.error.as_ref() != Some(&p) {
                        events.push(error_event(&p));
                    }
                    state.error = Some(p);
                }
            }
        }
        // A file deleted: its theme is gone, or back to the built-in one.
        self.files.retain(|name, _| seen.contains(name));

        // The chains, with the last valid versions: a parent deleted or a
        // cycle makes a theme unusable, which `theme_list` says.
        let layers = self.layers();
        for (name, state) in self.files.iter_mut() {
            let chain_error = if state.layer.is_some() { resolve(name, &layers).err() } else { None };
            if let Some(p) = &chain_error
                && state.chain_error.as_ref() != Some(p)
            {
                events.push(error_event(p));
            }
            state.chain_error = chain_error;
        }

        if self.apply() && !startup {
            events.push(nm_api::Event::ThemeChanged {});
        }
        events
    }

    /// Changes the theme chosen (`ui.theme` of a new `config.toml`).
    /// Returns `ThemeChanged` when the theme in effect changed.
    pub fn set_selected(&mut self, name: &str) -> Vec<nm_api::Event> {
        if name == self.selected {
            return Vec::new();
        }
        self.selected = name.to_string();
        if self.apply() { vec![nm_api::Event::ThemeChanged {}] } else { Vec::new() }
    }

    /// Works out the theme in effect from `selected` and the files; returns
    /// whether it changed.
    fn apply(&mut self) -> bool {
        let layers = self.layers();
        let name = self.selected.clone();
        let mut warnings = Vec::new();
        let (resolved, error) = match resolve(&name, &layers) {
            Ok(r) => {
                // The first refusal of a file of the chain: the theme is
                // made of its last valid version, and the app says why.
                let error = r.chain.iter().find_map(|n| self.files.get(n).and_then(|f| f.error.clone()));
                (r, error)
            }
            Err(p) => {
                // The theme cannot be resolved. While it was the one in
                // effect, it stays as it was; else the default theme
                // stands in, with a warning.
                let known = self.files.get(&name).map(|f| f.error.clone().unwrap_or_else(|| p.clone()));
                if self.applied.resolved.name == name {
                    (self.applied.resolved.clone(), Some(known.unwrap_or(p)))
                } else {
                    let message = match &known {
                        Some(_) => format!("the theme {name} cannot be used: {DEFAULT_THEME} is applied"),
                        None => format!("no theme named {name}: {DEFAULT_THEME} is applied"),
                    };
                    warnings.push(Problem { file: config::CONFIG_FILE.into(), line: None, key: Some("ui.theme".into()), message });
                    let fallback = resolve(DEFAULT_THEME, &layers)
                        .unwrap_or_else(|_| resolve(DEFAULT_THEME, &BTreeMap::new()).expect("the default theme resolves"));
                    (fallback, known)
                }
            }
        };
        warnings.extend(resolved.warnings.iter().cloned());
        warnings.extend(contrast_warnings(&resolved.name, &resolved.values));
        warnings.extend(self.ignored.iter().cloned());
        let applied = Applied { resolved, error, warnings };
        let changed = applied != self.applied;
        self.applied = applied;
        changed
    }

    /// The `.toml` files of the directory, by the name of their theme. A
    /// missing directory has none: the built-in themes alone.
    fn list_files(&self) -> Vec<(String, PathBuf)> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return Vec::new() };
        let mut out: Vec<(String, PathBuf)> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "toml"))
            .filter_map(|p| Some((p.file_stem()?.to_str()?.to_string(), p)))
            .collect();
        out.sort();
        out
    }
}

/// Reads a theme file, at most [`MAX_FILE_BYTES`].
fn read_file(path: &Path, label: &str) -> Result<String, Problem> {
    let refuse = |message: String| Problem { file: label.to_string(), line: None, key: None, message };
    let meta = std::fs::metadata(path).map_err(|e| refuse(format!("cannot read {}: {e}", path.display())))?;
    if meta.len() > MAX_FILE_BYTES {
        return Err(refuse(format!("the file is bigger than {} KB: a theme holds a few", MAX_FILE_BYTES / 1024)));
    }
    let bytes = std::fs::read(path).map_err(|e| refuse(format!("cannot read {}: {e}", path.display())))?;
    String::from_utf8(bytes).map_err(|_| refuse("the file is not valid UTF-8".into()))
}

fn error_event(p: &Problem) -> nm_api::Event {
    nm_api::Event::ConfigError { file: p.file.clone(), line: p.line, key: p.key.clone(), message: p.message.clone() }
}
