//! The prompts in use: the built-in ones, compiled into the binary, merged
//! with the user's files (`specifications/ia.md`, section 4, "Deux
//! sources").
//!
//! # Reading a file
//!
//! As for `config.toml` (`nm-engine/src/config.rs`, "Why a hand-written
//! reader"), a file is parsed by `toml_edit`, which keeps the position of
//! every key, and walked by hand: every problem names its file and line,
//! unknown keys are warnings, and the rules of the specification (an input
//! that has no meaning in the prompt's scope, `unmarked = "folder"` without
//! `folders`) are errors with a line too.
//!
//! # Merging
//!
//! A user file whose `id` is a built-in prompt's replaces it; another `id`
//! adds a prompt; `enabled = false` hides the prompt of that `id`. A file
//! with a problem is refused alone: the other prompts stay usable, unlike
//! `config.toml`, refused whole, because a prompt has no effect on the
//! others. Then the keys are checked screen by screen ([`Screen`]): two
//! prompts shown in the same menu cannot share a letter unless their
//! `folders` do not overlap. The later one is refused, a user's prompt
//! before a built-in one.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::path::{Path, PathBuf};

use nm_core::FolderRole;
use sha2::{Digest, Sha256};
use toml_edit::{Item, TableLike};

use crate::prompt::{
    Input, Labels, Lang, Output, Prompt, Scope, Screen, Source, Then, Unmarked, Variant,
};

/// The built-in prompts: file name and text, compiled into the binary. Their
/// order is the order of the menus within a scope (`Catalog::menu`): the
/// most used first, since the first row is the one Return chooses.
///
/// Rust note: `include_str!` reads the file at compile time and embeds its
/// text as a `&'static str`; a missing file is a compile error, not a
/// runtime one.
pub const BUILTIN: &[(&str, &str)] = &[
    ("proofread.toml", include_str!("../prompts/proofread.toml")),
    ("rewrite.toml", include_str!("../prompts/rewrite.toml")),
    (
        "translate-draft.toml",
        include_str!("../prompts/translate-draft.toml"),
    ),
    (
        "write-from-notes.toml",
        include_str!("../prompts/write-from-notes.toml"),
    ),
    (
        "subject-ideas.toml",
        include_str!("../prompts/subject-ideas.toml"),
    ),
    (
        "summarize-thread.toml",
        include_str!("../prompts/summarize-thread.toml"),
    ),
    (
        "quick-replies.toml",
        include_str!("../prompts/quick-replies.toml"),
    ),
    (
        "translate-message.toml",
        include_str!("../prompts/translate-message.toml"),
    ),
    (
        "security-check.toml",
        include_str!("../prompts/security-check.toml"),
    ),
    ("dates.toml", include_str!("../prompts/dates.toml")),
    (
        "summarize-attachment.toml",
        include_str!("../prompts/summarize-attachment.toml"),
    ),
    (
        "triage-quarantine.toml",
        include_str!("../prompts/triage-quarantine.toml"),
    ),
    (
        "suggest-folder.toml",
        include_str!("../prompts/suggest-folder.toml"),
    ),
];

/// A problem in a prompt file. The texts are in English, like the CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// The file's path, or `builtin:<name>` for a built-in prompt.
    pub file: String,
    /// Line of the file, from 1.
    pub line: Option<u32>,
    /// The key concerned (`input`, `variants.key`).
    pub key: Option<String>,
    pub message: String,
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.file)?;
        if let Some(l) = self.line {
            write!(f, ", line {l}")?;
        }
        if let Some(k) = &self.key {
            write!(f, ", {k}")?;
        }
        write!(f, ": {}", self.message)
    }
}

/// A user's prompt file, read from disk.
#[derive(Debug, Clone)]
pub struct UserFile {
    pub path: PathBuf,
    pub text: String,
}

/// The outcome of a load: the prompts in use and what was wrong.
#[derive(Debug, Clone, Default)]
pub struct Loaded {
    pub catalog: Catalog,
    /// Files, or prompts, refused.
    pub problems: Vec<Problem>,
    /// Unknown keys and other harmless oddities.
    pub warnings: Vec<Problem>,
}

/// The prompts in use, in a stable order: built-in ones (or their
/// replacement) first, in [`BUILTIN`] order, then the user's added prompts,
/// by file name.
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    prompts: Vec<Prompt>,
}

impl Catalog {
    pub fn prompts(&self) -> &[Prompt] {
        &self.prompts
    }

    pub fn get(&self, id: &str) -> Option<&Prompt> {
        self.prompts.iter().find(|p| p.id == id)
    }

    /// The prompts of a screen's AI menu, in a folder of this role (`None`
    /// for the compose screen), the most used first: the first row is
    /// highlighted when the menu opens, so that ⌘I then Return proofreads
    /// a reply. By the screen's order of scopes (`Screen::scopes`: the
    /// Quarantine sorts before it summarizes), then in the catalog's order: the
    /// built-in prompts as `BUILTIN` ranks them, from the most useful, then
    /// the user's.
    pub fn menu(&self, screen: Screen, folder: Option<FolderRole>) -> Vec<&Prompt> {
        let scopes = screen.scopes();
        let mut v: Vec<(usize, usize, &Prompt)> = self
            .prompts
            .iter()
            .enumerate()
            .filter(|(_, p)| p.offered_in(folder) && screen.offers(p))
            .filter_map(|(i, p)| Some((scopes.iter().position(|s| *s == p.scope)?, i, p)))
            .collect();
        v.sort_by_key(|(scope, i, _)| (*scope, *i));
        v.into_iter().map(|(_, _, p)| p).collect()
    }
}

/// The built-in prompts and the user's prompts folder. A missing folder
/// means no user prompt; the folder is never created (`ia.md`, section 4).
pub fn load_dir(dir: &Path) -> Loaded {
    let mut files = Vec::new();
    let mut problems = Vec::new();
    match std::fs::read_dir(dir) {
        Ok(entries) => {
            let mut paths: Vec<PathBuf> = entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| {
                    p.extension().is_some_and(|x| x == "toml")
                        && !p
                            .file_name()
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| n.starts_with('.'))
                })
                .collect();
            // Rust note: `read_dir` gives no order; sorting makes the merge,
            // and so which of two conflicting files wins, repeatable.
            paths.sort();
            for path in paths {
                match std::fs::read(&path).map(String::from_utf8) {
                    Ok(Ok(text)) => files.push(UserFile { path, text }),
                    Ok(Err(_)) => problems.push(file_problem(&path, "the file is not valid UTF-8")),
                    Err(e) => problems.push(file_problem(&path, &format!("cannot read: {e}"))),
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => problems.push(file_problem(
            dir,
            &format!("cannot read the prompts folder: {e}"),
        )),
    }
    let mut loaded = load(&files);
    problems.append(&mut loaded.problems);
    loaded.problems = problems;
    loaded
}

fn file_problem(path: &Path, message: &str) -> Problem {
    Problem {
        file: path.display().to_string(),
        line: None,
        key: None,
        message: message.into(),
    }
}

/// What a file declares.
///
/// Rust note: `Box<Prompt>` puts the prompt on the heap, so that the enum
/// stays small: an enum is as large as its largest variant, and the other
/// variant is only an id.
enum Declared {
    Prompt(Box<Prompt>),
    /// `enabled = false`: hides the prompt of that id.
    Disabled {
        id: String,
        line: Option<u32>,
    },
}

/// Merges the built-in prompts and the user's files (see the module's
/// documentation).
pub fn load(user: &[UserFile]) -> Loaded {
    let mut problems = Vec::new();
    let mut warnings = Vec::new();

    let mut builtin: Vec<Prompt> = Vec::new();
    for (name, text) in BUILTIN {
        match parse(
            &format!("builtin:{name}"),
            text,
            Source::Builtin,
            &mut warnings,
        ) {
            Ok(Declared::Prompt(p)) => builtin.push(*p),
            Ok(Declared::Disabled { .. }) => {}
            Err(e) => problems.push(e),
        }
    }

    // The user's files, by id: a second file with the same id is refused.
    let mut added: Vec<Prompt> = Vec::new();
    let mut replacing: BTreeMap<String, Prompt> = BTreeMap::new();
    let mut disabled: Vec<(String, String, Option<u32>)> = Vec::new();
    let mut seen: BTreeMap<String, String> = BTreeMap::new();
    for f in user {
        let file = f.path.display().to_string();
        let declared = match parse(&file, &f.text, Source::User(f.path.clone()), &mut warnings) {
            Ok(d) => d,
            Err(e) => {
                problems.push(e);
                continue;
            }
        };
        let id = match &declared {
            Declared::Prompt(p) => p.id.clone(),
            Declared::Disabled { id, .. } => id.clone(),
        };
        if let Some(other) = seen.get(&id) {
            problems.push(Problem {
                file,
                line: None,
                key: Some("id".into()),
                message: format!("the prompt {id} is already defined in {other}"),
            });
            continue;
        }
        seen.insert(id.clone(), file.clone());
        match declared {
            Declared::Disabled { id, line } => disabled.push((id, file, line)),
            Declared::Prompt(p) if builtin.iter().any(|b| b.id == p.id) => {
                replacing.insert(p.id.clone(), *p);
            }
            Declared::Prompt(p) => added.push(*p),
        }
    }

    // Built-in prompts, replaced where the user says so, then the added ones.
    let mut candidates: Vec<Prompt> = builtin
        .into_iter()
        .map(|b| replacing.remove(&b.id).unwrap_or(b))
        .chain(added)
        .collect();
    for (id, file, line) in disabled {
        let before = candidates.len();
        candidates.retain(|p| p.id != id);
        if candidates.len() == before {
            warnings.push(Problem {
                file,
                line,
                key: Some("id".into()),
                message: format!("no prompt {id} to disable"),
            });
        }
    }

    // Keys, screen by screen. A user's prompt yields to a built-in one: the
    // built-in prompts are checked first, so a conflict is always reported
    // on the user's file, which the user can change. The check runs in that
    // order; the catalog keeps the order of `candidates`.
    //
    // Rust note: `sort_by_key` is stable: indices with the same key (two
    // built-in prompts) keep their relative order.
    let mut order: Vec<usize> = (0..candidates.len()).collect();
    order.sort_by_key(|&i| matches!(candidates[i].source, Source::User(_)));
    let mut accepted = vec![false; candidates.len()];
    for &i in &order {
        let p = &candidates[i];
        match order.iter().copied().filter(|&j| accepted[j]).find(|&j| clash(&candidates[j], p)) {
            Some(j) => problems.push(Problem {
                file: source_name(p),
                line: None,
                key: Some("key".into()),
                message: format!(
                    "the key {} is already the prompt {}'s in the same menu; choose another key, or folders \
                     that do not overlap",
                    p.key, candidates[j].id
                ),
            }),
            None => accepted[i] = true,
        }
    }
    let prompts = candidates
        .into_iter()
        .zip(accepted)
        .filter_map(|(p, ok)| ok.then_some(p))
        .collect();
    Loaded {
        catalog: Catalog { prompts },
        problems,
        warnings,
    }
}

/// Whether two prompts would share a letter in one menu.
fn clash(a: &Prompt, b: &Prompt) -> bool {
    let same_screen = Screen::ALL.iter().any(|s| s.offers(a) && s.offers(b));
    let overlap = match (&a.folders, &b.folders) {
        (Some(x), Some(y)) => x.iter().any(|f| y.contains(f)),
        // No `folders`: offered everywhere, so it meets any other.
        _ => true,
    };
    a.key == b.key && same_screen && overlap
}

fn source_name(p: &Prompt) -> String {
    match &p.source {
        Source::Builtin => format!("builtin:{}.toml", p.id),
        Source::User(path) => path.display().to_string(),
    }
}

// ---------------------------------------------------------------------------
// Reading one file
// ---------------------------------------------------------------------------

type R<T> = Result<T, Problem>;

/// Walks the tree of one file, keeping the keys read to report the others.
struct Reader<'t> {
    file: &'t str,
    text: &'t str,
    read: Vec<&'static str>,
}

impl<'t> Reader<'t> {
    fn line(&self, span: Option<Range<usize>>) -> Option<u32> {
        span.map(|s| {
            let end = s.start.min(self.text.len());
            self.text.as_bytes()[..end]
                .iter()
                .filter(|&&b| b == b'\n')
                .count() as u32
                + 1
        })
    }

    fn problem(&self, item: Option<&Item>, key: &str, message: impl Into<String>) -> Problem {
        Problem {
            file: self.file.into(),
            line: self.line(item.and_then(|i| i.span())),
            key: Some(key.into()),
            message: message.into(),
        }
    }

    fn get<'a>(&mut self, root: &'a dyn TableLike, key: &'static str) -> Option<&'a Item> {
        self.read.push(key);
        root.get(key).filter(|i| !i.is_none())
    }

    fn required<'a>(&mut self, root: &'a dyn TableLike, key: &'static str) -> R<&'a Item> {
        self.get(root, key).ok_or_else(|| Problem {
            file: self.file.into(),
            line: None,
            key: Some(key.into()),
            message: "required key missing".into(),
        })
    }

    fn str<'a>(&self, item: &'a Item, key: &str) -> R<&'a str> {
        item.as_value()
            .and_then(|v| v.as_str())
            .ok_or_else(|| self.problem(Some(item), key, "expected a string"))
    }

    /// A string from a closed list, through its `parse` function.
    fn choice<T>(
        &self,
        item: &Item,
        key: &str,
        parse: impl Fn(&str) -> Option<T>,
        allowed: &str,
    ) -> R<T> {
        let s = self.str(item, key)?;
        parse(s).ok_or_else(|| {
            self.problem(
                Some(item),
                key,
                format!("unknown value {s:?}; expected {allowed}"),
            )
        })
    }

    /// The strings of an array.
    fn strings<'a>(&self, item: &'a Item, key: &str) -> R<Vec<&'a str>> {
        let arr = item
            .as_value()
            .and_then(|v| v.as_array())
            .ok_or_else(|| self.problem(Some(item), key, "expected an array of strings"))?;
        arr.iter()
            .map(|v| {
                v.as_str()
                    .ok_or_else(|| self.problem(Some(item), key, "expected an array of strings"))
            })
            .collect()
    }

    /// A label: a string (English), or a table of language code to text.
    fn labels(&self, item: &Item, key: &str) -> R<Labels> {
        if let Some(s) = item.as_value().and_then(|v| v.as_str()) {
            let s = s.trim();
            if s.is_empty() {
                return Err(self.problem(Some(item), key, "the label is empty"));
            }
            return Ok(Labels([("en".to_string(), s.to_string())].into()));
        }
        let table = item.as_table_like().ok_or_else(|| {
            self.problem(
                Some(item),
                key,
                "expected a string or a table such as { en = \"…\", fr = \"…\" }",
            )
        })?;
        let mut map = BTreeMap::new();
        for (lang, v) in table.iter() {
            let ok_code = lang.len() == 2 && lang.bytes().all(|b| b.is_ascii_lowercase());
            let text = v
                .as_value()
                .and_then(|v| v.as_str())
                .map(str::trim)
                .unwrap_or("");
            if !ok_code || text.is_empty() {
                return Err(self.problem(
                    Some(v),
                    &format!("{key}.{lang}"),
                    "expected a two-letter language code and a text",
                ));
            }
            map.insert(lang.to_string(), text.to_string());
        }
        if map.is_empty() {
            return Err(self.problem(Some(item), key, "the label is empty"));
        }
        Ok(Labels(map))
    }

    /// A menu key: one lower-case letter or digit.
    fn key_char(&self, item: &Item, key: &str) -> R<char> {
        let s = self.str(item, key)?;
        let mut chars = s.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) if c.is_ascii_lowercase() || c.is_ascii_digit() => Ok(c),
            _ => Err(self.problem(Some(item), key, "expected one lower-case letter or digit")),
        }
    }
}

/// Reads one prompt file.
fn parse(file: &str, text: &str, source: Source, warnings: &mut Vec<Problem>) -> R<Declared> {
    let doc = toml_edit::Document::parse(text).map_err(|e| Problem {
        file: file.into(),
        line: e.span().map(|s| {
            text.as_bytes()[..s.start.min(text.len())]
                .iter()
                .filter(|&&b| b == b'\n')
                .count() as u32
                + 1
        }),
        key: None,
        message: e
            .message()
            .lines()
            .next()
            .unwrap_or("invalid TOML")
            .to_string(),
    })?;
    let root: &dyn TableLike = doc.as_table();
    let mut r = Reader {
        file,
        text,
        read: Vec::new(),
    };

    let version = r.required(root, "version")?;
    if version.as_value().and_then(|v| v.as_integer()) != Some(1) {
        return Err(r.problem(
            Some(version),
            "version",
            "this version of NiceMailer reads version = 1",
        ));
    }

    let id_item = r.required(root, "id")?;
    let id = r.str(id_item, "id")?.to_string();
    let id_ok = (1..=40).contains(&id.len())
        && id.starts_with(|c: char| c.is_ascii_lowercase())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if !id_ok {
        return Err(r.problem(
            Some(id_item),
            "id",
            "expected 1 to 40 characters among a-z, 0-9 and -, starting with a letter",
        ));
    }

    if let Some(item) = r.get(root, "enabled") {
        let enabled = item
            .as_value()
            .and_then(|v| v.as_bool())
            .ok_or_else(|| r.problem(Some(item), "enabled", "expected true or false"))?;
        if !enabled {
            // The rest of a disabled file is not checked: it may be a prompt
            // put aside while being written.
            return Ok(Declared::Disabled {
                id,
                line: r.line(item.span()),
            });
        }
    }

    let scope_item = r.required(root, "scope")?;
    let scope = r.choice(
        scope_item,
        "scope",
        Scope::parse,
        "draft, message, thread, selection or attachment",
    )?;

    let key_item = r.required(root, "key")?;
    let key = r.key_char(key_item, "key")?;

    let label_item = r.required(root, "label")?;
    let label = r.labels(label_item, "label")?;

    let input_item = r.required(root, "input")?;
    let mut inputs = Vec::new();
    for name in r.strings(input_item, "input")? {
        let input = Input::parse(name).ok_or_else(|| {
            r.problem(Some(input_item), "input", format!("unknown input {name:?}"))
        })?;
        if !input.scopes().contains(&scope) {
            return Err(r.problem(
                Some(input_item),
                "input",
                format!("{name} has no meaning in the scope {}", scope.as_str()),
            ));
        }
        if inputs.contains(&input) {
            return Err(r.problem(Some(input_item), "input", format!("{name} is given twice")));
        }
        inputs.push(input);
    }
    if inputs.is_empty() {
        return Err(r.problem(Some(input_item), "input", "at least one input is needed"));
    }

    let output_item = r.required(root, "output")?;
    let output = r.choice(
        output_item,
        "output",
        Output::parse,
        "replace, insert, reply, panel, choices, subjects, summary, verdict or targets",
    )?;
    if !output.scopes().contains(&scope) {
        return Err(r.problem(
            Some(output_item),
            "output",
            format!(
                "{} cannot be applied in the scope {}",
                output.as_str(),
                scope.as_str()
            ),
        ));
    }
    if output == Output::Replace && !inputs.contains(&Input::DraftOwnText) {
        return Err(r.problem(
            Some(input_item),
            "input",
            "replace replaces draft.own_text, which must be an input",
        ));
    }
    if output == Output::Targets
        && !(inputs.contains(&Input::SelectionItems) && inputs.contains(&Input::Targets))
    {
        return Err(r.problem(
            Some(input_item),
            "input",
            "targets needs the inputs selection.items and targets",
        ));
    }

    let then = match r.get(root, "then") {
        None if output == Output::Choices => {
            return Err(r.problem(
                Some(output_item),
                "then",
                "choices needs then = \"insert\" or \"reply\"",
            ));
        }
        None => None,
        Some(item) if output != Output::Choices => {
            return Err(r.problem(Some(item), "then", "then is only for output = \"choices\""));
        }
        Some(item) => {
            let t = r.choice(
                item,
                "then",
                |s| match s {
                    "insert" => Some(Then::Insert),
                    "reply" => Some(Then::Reply),
                    _ => None,
                },
                "insert or reply",
            )?;
            let fits = match t {
                Then::Insert => scope == Scope::Draft,
                Then::Reply => scope != Scope::Draft,
            };
            if !fits {
                return Err(r.problem(
                    Some(item),
                    "then",
                    format!(
                        "then = {:?} cannot be applied in the scope {}",
                        r.str(item, "then")?,
                        scope.as_str()
                    ),
                ));
            }
            Some(t)
        }
    };

    let model = match r.get(root, "model") {
        None => None,
        Some(item) => {
            let m = r.str(item, "model")?.trim();
            // An empty model means "the default", as in the spec's example.
            (!m.is_empty()).then(|| m.to_string())
        }
    };

    let max_output_tokens = match r.get(root, "max_output_tokens") {
        None => None,
        Some(item) => match item.as_value().and_then(|v| v.as_integer()) {
            Some(n @ 1..=64_000) => Some(n as u32),
            _ => {
                return Err(r.problem(
                    Some(item),
                    "max_output_tokens",
                    "must be between 1 and 64000",
                ));
            }
        },
    };

    let variants = match r.get(root, "variants") {
        None => Vec::new(),
        Some(item) => variants(&r, item)?,
    };

    let text_item = r.required(root, "system")?;
    let text = r.str(text_item, "system")?.trim().to_string();
    if text.is_empty() {
        return Err(r.problem(Some(text_item), "system", "the prompt's text is empty"));
    }
    match (variants.is_empty(), text.contains("{{variant}}")) {
        (false, false) => {
            return Err(r.problem(
                Some(text_item),
                "system",
                "a prompt with variants must say where they go with {{variant}}",
            ));
        }
        (true, true) => {
            return Err(r.problem(
                Some(text_item),
                "system",
                "{{variant}} is used but there is no variant",
            ));
        }
        _ => {}
    }

    let lang = match r.get(root, "lang") {
        // Writing follows the text's language, the rest the app's.
        None => {
            if scope == Scope::Draft {
                Lang::Content
            } else {
                Lang::App
            }
        }
        Some(item) => {
            let l = r.choice(
                item,
                "lang",
                |s| match s {
                    "content" => Some(Lang::Content),
                    "app" => Some(Lang::App),
                    "variant" => Some(Lang::Variant),
                    _ => None,
                },
                "content, app or variant",
            )?;
            if l == Lang::Variant && variants.is_empty() {
                return Err(r.problem(
                    Some(item),
                    "lang",
                    "lang = \"variant\" needs variants that name the language",
                ));
            }
            l
        }
    };

    let folders = match r.get(root, "folders") {
        None => None,
        Some(item) => {
            let mut list = Vec::new();
            for name in r.strings(item, "folders")? {
                let role = FolderRole::parse(name).ok_or_else(|| {
                    r.problem(
                        Some(item),
                        "folders",
                        format!("unknown folder role {name:?}"),
                    )
                })?;
                if !list.contains(&role) {
                    list.push(role);
                }
            }
            if list.is_empty() {
                return Err(r.problem(
                    Some(item),
                    "folders",
                    "at least one folder role is needed; leave the key out for every folder",
                ));
            }
            Some(list)
        }
    };

    let unmarked = match r.get(root, "unmarked") {
        None => Unmarked::Cursor,
        Some(item) => {
            if scope != Scope::Selection {
                return Err(r.problem(
                    Some(item),
                    "unmarked",
                    "unmarked is only for scope = \"selection\"",
                ));
            }
            let u = r.choice(
                item,
                "unmarked",
                |s| match s {
                    "cursor" => Some(Unmarked::Cursor),
                    "folder" => Some(Unmarked::Folder),
                    _ => None,
                },
                "cursor or folder",
            )?;
            // So that a prompt cannot send the whole Archive by mistake
            // (`ia.md`, section 4).
            if u == Unmarked::Folder && folders.is_none() {
                return Err(r.problem(
                    Some(item),
                    "unmarked",
                    "unmarked = \"folder\" needs folders, the roles where it applies",
                ));
            }
            u
        }
    };

    // Unknown keys: warnings, so that a file written for a newer NiceMailer
    // stays usable.
    for (k, item) in root.iter() {
        if !r.read.contains(&k) {
            let line = root
                .get_key_value(k)
                .and_then(|(key, _)| key.span())
                .or_else(|| item.span());
            warnings.push(Problem {
                file: file.into(),
                line: r.line(line),
                key: Some(k.into()),
                message: "unknown key, ignored".into(),
            });
        }
    }

    let mut p = Prompt {
        id,
        scope,
        key,
        label,
        inputs,
        output,
        then,
        lang,
        model,
        max_output_tokens,
        variants,
        folders,
        unmarked,
        text,
        source,
        hash: [0; 32],
    };
    p.hash = hash(&p);
    Ok(Declared::Prompt(Box::new(p)))
}

/// The `[[variants]]` tables.
fn variants(r: &Reader, item: &Item) -> R<Vec<Variant>> {
    // Rust note: `[[variants]]` gives an *array of tables*, while
    // `variants = [{ … }, { … }]` gives an array of inline tables; both are
    // accepted, through the common `TableLike` view.
    let tables: Vec<(&dyn TableLike, Option<Range<usize>>)> =
        if let Some(aot) = item.as_array_of_tables() {
            aot.iter()
                .map(|t| (t as &dyn TableLike, t.span()))
                .collect()
        } else if let Some(arr) = item.as_value().and_then(|v| v.as_array()) {
            let mut v = Vec::new();
            for x in arr.iter() {
                let t = x.as_inline_table().ok_or_else(|| {
                    r.problem(
                        Some(item),
                        "variants",
                        "expected tables with key, label and instruction",
                    )
                })?;
                v.push((t as &dyn TableLike, x.span()));
            }
            v
        } else {
            return Err(r.problem(Some(item), "variants", "expected [[variants]] tables"));
        };
    let mut out: Vec<Variant> = Vec::new();
    for (t, span) in tables {
        let at = |key: &str, message: &str| Problem {
            file: r.file.into(),
            line: r.line(span.clone()),
            key: Some(format!("variants.{key}")),
            message: message.into(),
        };
        let key_item = t
            .get("key")
            .ok_or_else(|| at("key", "required key missing"))?;
        let key = r.key_char(key_item, "variants.key")?;
        if out.iter().any(|v| v.key == key) {
            return Err(at(
                "key",
                &format!("the key {key} is given to two variants"),
            ));
        }
        let label = r.labels(
            t.get("label")
                .ok_or_else(|| at("label", "required key missing"))?,
            "variants.label",
        )?;
        let instruction = t
            .get("instruction")
            .and_then(|i| i.as_value())
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| at("instruction", "expected a non-empty string"))?
            .to_string();
        for (k, _) in t.iter() {
            if !["key", "label", "instruction"].contains(&k) {
                return Err(at(k, "unknown key in a variant"));
            }
        }
        out.push(Variant {
            key,
            label,
            instruction,
        });
    }
    if out.is_empty() {
        return Err(r.problem(
            Some(item),
            "variants",
            "leave the key out rather than giving no variant",
        ));
    }
    Ok(out)
}

/// The SHA-256 of what changes the prompt's behaviour, in a fixed form: a
/// comment, a key moved or a label translated leave it unchanged, a word of
/// the text does not (`ia.md`, section 4, "hash").
fn hash(p: &Prompt) -> [u8; 32] {
    let mut h = Sha256::new();
    let mut field = |name: &str, value: &str| {
        // Length-prefixed, so that no two different prompts give the same
        // bytes by moving text from one field to the next.
        h.update(format!("{name}:{}:", value.len()).as_bytes());
        h.update(value.as_bytes());
    };
    field("id", &p.id);
    field("scope", p.scope.as_str());
    field("key", &p.key.to_string());
    field(
        "input",
        &p.inputs
            .iter()
            .map(|i| i.as_str())
            .collect::<Vec<_>>()
            .join(","),
    );
    field("output", p.output.as_str());
    field("then", &format!("{:?}", p.then));
    field("lang", &format!("{:?}", p.lang));
    field("model", p.model.as_deref().unwrap_or(""));
    field("max_output_tokens", &format!("{:?}", p.max_output_tokens));
    let folders: BTreeSet<&str> = p.folders.iter().flatten().map(|f| f.as_str()).collect();
    field(
        "folders",
        &format!("{:?}{:?}", p.folders.is_some(), folders),
    );
    field("unmarked", &format!("{:?}", p.unmarked));
    for v in &p.variants {
        field("variant", &format!("{}={}", v.key, v.instruction));
    }
    field("system", &p.text);
    h.finalize().into()
}
