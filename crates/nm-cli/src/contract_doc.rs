//! Keeps `docs/contract.md` true to the code. The page lists every method
//! of the contract and the CLI command that uses it; written by hand, it
//! would drift the first time a method or an option is added. These tests
//! fail `make test` as soon as the page and the code disagree:
//!
//! 1. the methods of the page's tables are exactly those of `nm_api::method`;
//! 2. every `nicemailer …` command the page shows exists, with its options,
//!    in the clap definition of the CLI;
//! 3. the CLI column is right both ways: a method the CLI calls is not
//!    marked "—", and a method given a command is really called by the CLI;
//! 4. every method has a `///` comment in `nm-api`, so that a new method
//!    is described where it is declared;
//! 5. the reasons of errors of the section "Errors" are exactly those of
//!    `nm_api::reason`, with the values of their sentences, and each one
//!    is used by the code of some crate.
//!
//! What they cannot check is the meaning of the descriptions: a method
//! that changes behaviour under the same name needs a human to update the
//! page (rule in `CLAUDE.md`, "Documentation").
//!
//! Rust note: `include_str!` reads a file at compile time and embeds it as
//! a `&'static str`, with a path relative to this source file. The test
//! therefore reads the page and the sources as they were when it was
//! built, with no file access at run time, and Cargo rebuilds it when one
//! of them changes.

use std::collections::{BTreeMap, BTreeSet};

use clap::CommandFactory;

use crate::Cli;

const DOC: &str = include_str!("../../../docs/contract.md");
const API: &str = include_str!("../../nm-api/src/lib.rs");
/// The sources of the CLI that call the core. A new file that calls it
/// must be added here, or its methods will look unused.
const CLI_SOURCES: [(&str, &str); 2] = [("main.rs", include_str!("main.rs")), ("ai.rs", include_str!("ai.rs"))];

/// One method of the contract, read from `pub mod method` in `nm-api`.
struct ContractMethod {
    /// The Rust constant, e.g. `LIST_FOLDERS`.
    constant: String,
    /// The JSON-RPC name, e.g. `list_folders`.
    name: String,
    /// Whether a `///` comment is right above the constant.
    documented: bool,
}

/// The methods of the contract, in declaration order. The module is read
/// as text rather than through a list kept in `nm-api`: such a list would
/// itself be one more thing to keep up to date.
fn contract_methods() -> Vec<ContractMethod> {
    let start = API.find("pub mod method {").expect("nm-api has no `pub mod method`");
    let module = &API[start..];
    // The module ends at the first closing brace at the start of a line.
    let end = module.find("\n}").expect("`pub mod method` is not closed");
    let mut methods = Vec::new();
    let mut documented = false;
    for line in module[..end].lines().skip(1) {
        let line = line.trim();
        if line.starts_with("///") {
            documented = true;
            continue;
        }
        if let Some(rest) = line.strip_prefix("pub const ") {
            let (constant, value) = rest.split_once(':').expect("a method constant without a type");
            let name = value.split('"').nth(1).expect("a method constant without a string value");
            methods.push(ContractMethod { constant: constant.trim().to_string(), name: name.to_string(), documented });
        }
        documented = false;
    }
    methods
}

/// Splits a Markdown table row into its cells. A `\|` is a pipe inside a
/// cell (in a command such as `<inbox\|banned>`), not a cell border.
fn split_row(line: &str) -> Vec<String> {
    let mut cells = vec![String::new()];
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                chars.next();
                cells.last_mut().unwrap().push('|');
            }
            '|' => cells.push(String::new()),
            _ => cells.last_mut().unwrap().push(c),
        }
    }
    // A row starts and ends with a border: drop the empty outer cells.
    cells.into_iter().map(|c| c.trim().to_string()).filter(|c| !c.is_empty()).collect()
}

/// The page before the section "Errors", whose table lists reasons, not
/// methods.
fn methods_part() -> &'static str {
    DOC.split("\n## Errors\n").next().unwrap_or(DOC)
}

/// The rows of the method tables: method name and CLI cell, in page order.
/// A method row is a row whose first cell is a name in backquotes; the
/// header rows ("| Method |") are not.
fn doc_rows() -> Vec<(String, String)> {
    methods_part()
        .lines()
        .filter(|line| line.starts_with("| `"))
        .map(|line| {
            let cells = split_row(line);
            assert_eq!(cells.len(), 3, "a method row must have three cells (method, what it does, CLI): {line}");
            (cells[0].trim_matches('`').to_string(), cells[2].clone())
        })
        .collect()
}

/// The constants of `nm_api::method` the CLI uses (`method::LIST_FOLDERS`).
fn constants_used_by_cli() -> BTreeSet<String> {
    let mut used = BTreeSet::new();
    for (_, source) in CLI_SOURCES {
        for (i, _) in source.match_indices("method::") {
            let name: String =
                source[i + "method::".len()..].chars().take_while(|c| c.is_ascii_uppercase() || *c == '_').collect();
            if !name.is_empty() {
                used.insert(name);
            }
        }
    }
    used
}

/// The `nicemailer …` commands the page shows: inline code that starts
/// with `nicemailer `, and the lines of code blocks that do.
fn doc_commands() -> Vec<String> {
    let mut commands = Vec::new();
    let mut in_block = false;
    for line in DOC.lines() {
        if line.starts_with("```") {
            in_block = !in_block;
            continue;
        }
        if in_block {
            if line.starts_with("nicemailer ") {
                commands.push(line.to_string());
            }
            continue;
        }
        // Between backquotes: the odd pieces of the split.
        for span in line.split('`').skip(1).step_by(2) {
            if span.starts_with("nicemailer ") {
                commands.push(span.replace("\\|", "|"));
            }
        }
    }
    commands
}

/// Checks one command of the page against the clap definition: each
/// subcommand exists, the command ends on one that runs (not on a group
/// such as `account`), and each `--option` exists on it or is global.
/// `a | b` names sibling subcommands (`mailbox check | upgrade`).
/// Placeholders (`<key>`), values and JSON are not checked.
fn check_command(root: &clap::Command, text: &str) -> Result<(), String> {
    let mut words = text.split_whitespace();
    words.next(); // "nicemailer"
    let mut path: Vec<&clap::Command> = vec![root];
    // Still reading subcommand names: stops at the first word that is not
    // one (a placeholder, a value, an option).
    let mut naming = true;
    let leaf = |path: &[&clap::Command]| {
        let cmd = path.last().unwrap();
        if cmd.has_subcommands() {
            Err(format!("`{text}`: `{}` needs a subcommand", cmd.get_name()))
        } else {
            Ok(())
        }
    };
    for word in words {
        if word == "|" {
            leaf(&path)?;
            if path.len() < 2 {
                return Err(format!("`{text}`: `|` before any subcommand"));
            }
            path.pop();
            naming = true;
            continue;
        }
        let current = *path.last().unwrap();
        if let Some(flag) = word.trim_start_matches('[').strip_prefix("--") {
            naming = false;
            let flag = flag.split(['=', ']', '…']).next().unwrap_or_default();
            let known = current.get_arguments().chain(root.get_arguments()).any(|a| a.get_long() == Some(flag));
            if !known {
                return Err(format!("`{text}`: `{}` has no option --{flag}", current.get_name()));
            }
            continue;
        }
        let looks_like_name = word.chars().all(|c| c.is_ascii_lowercase() || c == '-');
        if naming && current.has_subcommands() && looks_like_name {
            match current.find_subcommand(word) {
                Some(sub) => path.push(sub),
                None => return Err(format!("`{text}`: `{}` has no subcommand `{word}`", current.get_name())),
            }
            continue;
        }
        naming = false;
    }
    leaf(&path)
}

#[test]
fn every_method_of_the_contract_is_on_the_page_once() {
    let code: Vec<String> = contract_methods().into_iter().map(|m| m.name).collect();
    assert!(code.len() > 50, "only {} methods read from nm-api: is `pub mod method` still parsed right?", code.len());
    let page: Vec<String> = doc_rows().into_iter().map(|(name, _)| name).collect();

    let mut seen = BTreeSet::new();
    let twice: Vec<&String> = page.iter().filter(|n| !seen.insert(*n)).collect();
    let code_set: BTreeSet<&String> = code.iter().collect();
    let page_set: BTreeSet<&String> = page.iter().collect();
    let missing: Vec<&&String> = code_set.difference(&page_set).collect();
    let stale: Vec<&&String> = page_set.difference(&code_set).collect();
    assert!(
        missing.is_empty() && stale.is_empty() && twice.is_empty(),
        "docs/contract.md is out of date.\n  missing from the page: {missing:?}\n  \
         on the page but not in nm_api::method: {stale:?}\n  listed twice: {twice:?}"
    );
}

#[test]
fn every_method_is_described_in_nm_api() {
    let undocumented: Vec<String> =
        contract_methods().into_iter().filter(|m| !m.documented).map(|m| m.constant).collect();
    assert!(undocumented.is_empty(), "these constants of nm_api::method have no `///` comment: {undocumented:?}");
}

#[test]
fn the_cli_column_matches_the_methods_the_cli_calls() {
    let by_constant: BTreeMap<String, String> = contract_methods().into_iter().map(|m| (m.constant, m.name)).collect();
    let used: BTreeSet<String> = constants_used_by_cli()
        .into_iter()
        .map(|c| by_constant.get(&c).cloned().unwrap_or_else(|| panic!("the CLI uses method::{c}, unknown to nm-api")))
        .collect();
    assert!(used.len() > 20, "only {} methods found in the CLI sources: is the scan still right?", used.len());

    let mut errors = Vec::new();
    for (name, cli) in doc_rows() {
        // "—" (possibly followed by a remark) means "no dedicated command".
        let marked = !cli.starts_with('—');
        match (marked, used.contains(&name)) {
            (false, true) => errors.push(format!("`{name}` is called by the CLI but its CLI cell is \"—\"")),
            (true, false) => errors.push(format!("`{name}` has a CLI cell but the CLI never calls method::…")),
            _ => {}
        }
    }
    assert!(errors.is_empty(), "docs/contract.md, CLI column:\n  {}", errors.join("\n  "));
}

/// The scan of `the_cli_column_matches_the_methods_the_cli_calls` only sees
/// `method::` constants: a method called by its name in a string would be
/// invisible to it (and is a typo waiting to happen).
#[test]
fn the_cli_calls_methods_through_constants() {
    let mut errors = Vec::new();
    for (file, source) in CLI_SOURCES {
        for call in [".call(", ".call_keeping_events("] {
            for (i, _) in source.match_indices(call) {
                if source[i + call.len()..].trim_start().starts_with('"') {
                    let line = source[..i].lines().count();
                    errors.push(format!("{file}:{line}: a method called by a string literal, use method::…"));
                }
            }
        }
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

#[test]
fn every_command_on_the_page_exists() {
    let root = Cli::command();
    let commands = doc_commands();
    assert!(commands.len() > 20, "only {} commands found on the page: is it still parsed right?", commands.len());
    let errors: Vec<String> = commands.iter().filter_map(|c| check_command(&root, c).err()).collect();
    assert!(errors.is_empty(), "docs/contract.md shows commands the CLI does not have:\n  {}", errors.join("\n  "));
}

/// The checker itself: it must refuse what does not exist, or the test
/// above would pass on anything.
#[test]
fn the_command_checker_refuses_what_does_not_exist() {
    let root = Cli::command();
    for good in [
        "nicemailer sync verify <key> [--skip-hash] [--repair] [--json]",
        "nicemailer mailbox check | upgrade | backup",
        "nicemailer call ping",
        "nicemailer account roles <key> todo=Todo [--create-missing]",
        "nicemailer folders --data-dir <dir>",
    ] {
        assert_eq!(check_command(&root, good), Ok(()), "{good}");
    }
    for bad in [
        "nicemailer sync nope",
        "nicemailer sync now --bogus",
        "nicemailer account",
        "nicemailer mailbox check | nope",
        "nicemailer ai run <prompt> [--no-such-flag]",
    ] {
        assert!(check_command(&root, bad).is_err(), "{bad} should be refused");
    }
}

#[test]
fn split_row_keeps_escaped_pipes_in_their_cell() {
    assert_eq!(split_row("| `a` | b | `x <c\\|d>` |"), vec!["`a`", "b", "`x <c|d>`"]);
}

/// The rows of the table of reasons: the reason and the keys of its
/// values, in page order.
fn doc_reasons() -> Vec<(String, Vec<String>)> {
    let errors = DOC.split("\n## Errors\n").nth(1).expect("docs/contract.md has no section \"Errors\"");
    errors
        .lines()
        .filter(|line| line.starts_with("| `"))
        .map(|line| {
            let cells = split_row(line);
            assert_eq!(cells.len(), 3, "a reason row must have three cells (reason, values, message): {line}");
            let values = if cells[1] == "—" {
                Vec::new()
            } else {
                cells[1].split(", ").map(|v| v.trim_matches('`').to_string()).collect()
            };
            (cells[0].trim_matches('`').to_string(), values)
        })
        .collect()
}

/// The page lists every reason of `nm_api::reason`, in its order, with the
/// values its sentence takes; and nothing else.
#[test]
fn the_page_lists_every_reason_with_its_values() {
    let page = doc_reasons();
    let code: Vec<(String, Vec<String>)> = nm_api::reason::ALL
        .iter()
        .map(|r| (r.name.to_string(), r.keys().iter().map(|k| k.to_string()).collect()))
        .collect();
    assert_eq!(page, code, "the table of docs/contract.md, section \"Errors\", differs from nm_api::reason");
}

/// Two reasons never share a name: a client could not tell them apart.
#[test]
fn every_reason_has_a_name_of_its_own() {
    let mut seen = BTreeSet::new();
    for r in nm_api::reason::ALL {
        assert!(seen.insert(r.name), "two reasons are named {}", r.name);
        assert!(r.name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'), "{}", r.name);
    }
}

/// Every reason is given by the code somewhere (`reason::NAME` or, in a
/// module that names the module `r`, `r::NAME`): a reason left behind by
/// a change is removed with it. The constants are read from the source of
/// `nm-api`, as the methods are.
#[test]
fn every_reason_is_used_by_the_code() {
    let start = API.find("reasons! {\n").expect("nm-api has no `reasons!` list");
    let constants: Vec<&str> = API[start..]
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            let (ident, rest) = l.split_once(" = \"")?;
            (ident.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') && rest.contains("\": \""))
                .then_some(ident)
        })
        .collect();
    assert_eq!(constants.len(), nm_api::reason::ALL.len(), "the constants of `reasons!` were not all read");
    // Every Rust source of the crates, tests aside.
    let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut sources = String::new();
    let mut dirs = vec![crates.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            if path.is_dir() {
                if name != "tests" && name != "target" {
                    dirs.push(path);
                }
            } else if name.ends_with(".rs") {
                sources.push_str(&std::fs::read_to_string(&path).unwrap());
            }
        }
    }
    for ident in constants {
        let used = sources.match_indices(&format!("::{ident}")).any(|(i, m)| {
            !sources[i + m.len()..].starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
        });
        assert!(used, "the reason {ident} is never given: remove it, or give it where it applies");
    }
}
