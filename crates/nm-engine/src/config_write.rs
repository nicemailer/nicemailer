//! Writing the settings files (`configuration.md`, sections 3 and 5):
//! creating them when they are missing, adding, renaming and removing an
//! account in `accounts.toml` for a command, and setting one key of
//! `config.toml` (`ui.theme`, `compose.format`) for the app.
//!
//! The files belong to the user, who may keep them in Git, with comments.
//! So:
//! - it is edited in place by `toml_edit`, which keeps comments, order and
//!   formatting: only the account's tables change;
//! - it is read again right before the change, so that an edit made by
//!   hand meanwhile is kept, and only the account's tables are touched;
//! - the result must be a valid file (`config::parse_accounts`), else
//!   nothing is written;
//! - it is written atomically: a temporary file next to it, then `rename`.
//!   A crash leaves the old file or the new one, never half of one.
//!
//! The core writes a file on its own only to create it, at its start, when
//! it is missing ([`create_missing`]); it never changes an existing file
//! but at a client's request.

use std::io::Write;
use std::path::{Path, PathBuf};

use toml_edit::{DocumentMut, Item, Table, value};

use crate::config::{self, Security};

/// The server of a new account, as the file says it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewAccount {
    pub key: String,
    pub email: String,
    pub display_name: Option<String>,
    pub host: String,
    pub port: u16,
    pub security: Security,
    pub username: String,
    pub ca_file: Option<String>,
    /// `read_only = true`: nothing is ever changed on the server.
    pub read_only: bool,
    /// `auth = "oauth2"` (`specifications/oauth2.md`): the account signs in
    /// with OAuth2 instead of a password. None: `password` (not written).
    pub auth: Option<String>,
    /// `[accounts.<key>.smtp]`; none: the account cannot send.
    pub smtp: Option<NewServer>,
}

/// The SMTP server of a new account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewServer {
    pub host: String,
    pub port: u16,
    pub security: Security,
    pub username: String,
    pub ca_file: Option<String>,
}

/// The name of the file the account commands write, for their messages.
const FILE: &str = config::ACCOUNTS_FILE;

fn doc(text: &str) -> Result<DocumentMut, String> {
    text.parse::<DocumentMut>().map_err(|e| format!("{FILE} cannot be read: {e}"))
}

/// The `[accounts]` table, created (implicit: no `[accounts]` header of its
/// own) if the file has none.
fn accounts(doc: &mut DocumentMut) -> Result<&mut Table, String> {
    if !doc.contains_key("accounts") {
        let mut t = Table::new();
        t.set_implicit(true);
        doc.insert("accounts", Item::Table(t));
    }
    doc["accounts"].as_table_mut().ok_or_else(|| format!("accounts is not a table in {FILE}"))
}

/// An account key: what the file and the Keychain item name can hold
/// without quoting surprises.
pub fn check_key(key: &str) -> Result<(), String> {
    if key.is_empty() || key.len() > 64 || !key.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')) {
        return Err(format!("invalid account key {key:?}: letters, digits, - and _, 64 characters at most"));
    }
    Ok(())
}

/// The text of `accounts.toml` with a new account added at its end.
pub fn add_account(text: &str, a: &NewAccount) -> Result<String, String> {
    check_key(&a.key)?;
    let mut doc = doc(text)?;
    if accounts(&mut doc)?.contains_key(&a.key) {
        return Err(format!("{FILE} already has an account {}", a.key));
    }
    let mut t = Table::new();
    t["email"] = value(&a.email);
    if let Some(n) = &a.display_name {
        t["display_name"] = value(n);
    }
    if a.read_only {
        t["read_only"] = value(true);
    }
    if let Some(auth) = &a.auth {
        t["auth"] = value(auth.as_str());
    }
    let imap = NewServer {
        host: a.host.clone(),
        port: a.port,
        security: a.security,
        username: a.username.clone(),
        ca_file: a.ca_file.clone(),
    };
    t.insert("imap", Item::Table(server_table(&imap, &a.email)));
    if let Some(smtp) = &a.smtp {
        t.insert("smtp", Item::Table(server_table(smtp, &a.email)));
    }
    // What follows the last table of the file (comments, or the whole file
    // when it holds only comments) is the document's "trailing" text in
    // `toml_edit`, written after every table, the new one included: it
    // would move below the new account. It goes before it instead.
    let trailing = doc.trailing().as_str().unwrap_or("").to_string();
    if !trailing.trim().is_empty() {
        let sep = if trailing.ends_with("\n\n") { "" } else { "\n" };
        t.decor_mut().set_prefix(format!("{trailing}{sep}"));
        doc.set_trailing("");
    }
    accounts(&mut doc)?.insert(&a.key, Item::Table(t));
    finish(doc)
}

/// `[accounts.<key>.imap]` or `.smtp`: no default value written.
fn server_table(s: &NewServer, email: &str) -> Table {
    let mut t = Table::new();
    t["host"] = value(&s.host);
    t["port"] = value(s.port as i64);
    t["security"] = value(match s.security {
        Security::Tls => "tls",
        Security::Starttls => "starttls",
        Security::None => "none",
    });
    // The user name only when it is not the address (the default).
    if s.username != email {
        t["username"] = value(&s.username);
    }
    if let Some(ca) = &s.ca_file {
        t["ca_file"] = value(ca);
    }
    t
}

/// The file with the account `from` renamed `to`, its tables where they
/// were.
pub fn rename_account(text: &str, from: &str, to: &str) -> Result<String, String> {
    check_key(to)?;
    let mut doc = doc(text)?;
    let accounts = accounts(&mut doc)?;
    if accounts.contains_key(to) {
        return Err(format!("{FILE} already has an account {to}"));
    }
    let item = accounts.remove(from).ok_or_else(|| format!("{FILE} has no account {from}"))?;
    // The tables keep their `position`: they are written back where they
    // were, under the new name.
    accounts.insert(to, item);
    finish(doc)
}

/// The file without the account `key`.
pub fn remove_account(text: &str, key: &str) -> Result<String, String> {
    let mut doc = doc(text)?;
    let accounts = accounts(&mut doc)?;
    accounts.remove(key).ok_or_else(|| format!("{FILE} has no account {key}"))?;
    finish(doc)
}

/// The file with the account `key` writable again: its `read_only` key
/// removed (the default is `false`), the rest of its table as it was. The
/// key is removed rather than set to `false`, so that the file reads as if
/// the account had been created writable; a comment on that line goes with
/// it.
pub fn clear_read_only(text: &str, key: &str) -> Result<String, String> {
    let mut doc = doc(text)?;
    let accounts = accounts(&mut doc)?;
    let account = accounts
        .get_mut(key)
        .and_then(Item::as_table_like_mut)
        .ok_or_else(|| format!("{FILE} has no account {key}"))?;
    account.remove("read_only");
    finish(doc)
}

/// `config.toml` with `ui.theme` set to `name`, the rest of the file kept
/// as it is, comments and order included (`themes.md`, section 8): the
/// Preferences and `v` `t` choose a theme with it. That the theme exists
/// is the caller's to check.
pub fn set_theme(text: &str, name: &str) -> Result<String, String> {
    if !crate::theme::catalog::valid_name(name) {
        return Err(format!("{name:?} cannot name a theme"));
    }
    set_config_value(text, "ui", "theme", name)
}

/// `config.toml` with `compose.format` set to `format` (`text` or `html`),
/// the rest of the file kept as it is: ⇧⌘T makes the mode it converts a
/// draft to the mode of the next mails (`edition.md`, section 3).
pub fn set_compose_format(text: &str, format: &str) -> Result<String, String> {
    if !matches!(format, "text" | "html") {
        return Err(format!("{format:?} is not a mode of the editor: text or html"));
    }
    set_config_value(text, "compose", "format", format)
}

/// `config.toml` with `[table] key = value`. These are the only changes
/// the core makes to that file, each at a client's request and limited to
/// its key. A table is added when the file has none.
///
/// The file must be valid before and after: a file the core refuses is not
/// touched, so that the user's own edit in progress is not mixed with this
/// one.
fn set_config_value(text: &str, table: &str, key: &str, new_value: &str) -> Result<String, String> {
    const FILE: &str = config::CONFIG_FILE;
    if let Err(p) = config::parse(text, "") {
        return Err(format!("{FILE} is refused, fix it first: {p}"));
    }
    let mut doc = text.parse::<DocumentMut>().map_err(|e| format!("{FILE} cannot be read: {e}"))?;
    if !doc.contains_key(table) {
        doc.insert(table, Item::Table(Table::new()));
    }
    let t = doc[table].as_table_like_mut().ok_or_else(|| format!("{FILE}: {table} is not a table"))?;
    // The comment after the old value (`theme = "geek"   # for now`) is
    // the user's: the new value takes its decoration.
    let mut new = toml_edit::Value::from(new_value);
    if let Some(old) = t.get(key).and_then(Item::as_value) {
        *new.decor_mut() = old.decor().clone();
    }
    t.insert(key, Item::Value(new));
    let out = doc.to_string();
    config::parse(&out, "").map_err(|p| format!("the change would make {FILE} invalid: {p}"))?;
    Ok(out)
}

/// The new text, refused if the core would refuse it.
fn finish(doc: DocumentMut) -> Result<String, String> {
    let text = doc.to_string();
    config::parse_accounts(&text).map_err(|p| format!("the file would be invalid: {p}"))?;
    Ok(text)
}

/// Applies `change` to the file at `path` as it is now (missing: empty),
/// and writes the result atomically. Returns the new text.
pub fn update(path: &Path, change: impl FnOnce(&str) -> Result<String, String>) -> Result<String, String> {
    let current = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let text = change(&current)?;
    write_atomic(path, &text)?;
    Ok(text)
}

/// Writes `text` to `path` through a temporary file in the same directory
/// and a `rename`, keeping the file's permissions.
fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let tmp = temp_path(dir, path);
    let result = (|| -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        if let Ok(meta) = std::fs::metadata(path) {
            f.set_permissions(meta.permissions())?;
        }
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("{}: {e}", path.display()));
    }
    Ok(())
}

/// A temporary file next to `path`, hidden, with a random suffix: two
/// writers never share one.
fn temp_path(dir: &Path, path: &Path) -> PathBuf {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    dir.join(format!(".{name}.{:016x}", rand::random::<u64>()))
}

/// Creates `config.toml` at `config_path`, and `accounts.toml` and
/// `keymap.toml` next to it, from their templates (`config::config_template`,
/// `config::accounts_template`, `keymap::template`), when they are missing. Returns the files
/// created. Called once, at the start of the core
/// (`configuration.md`, section 3): even a user who changes nothing finds
/// every setting listed, with its default and its role.
///
/// - **An existing file is never touched**, even empty or invalid: it is
///   the user's. A symbolic link counts as existing even if its target is
///   missing (a dotfiles repository not cloned yet): creating a file in
///   its place would hide the link's intent.
/// - **The file appears complete or not at all.** It is written to a
///   temporary file, then given its name by `hard_link`, which, unlike
///   `rename`, fails if the name is taken: a file created meanwhile by the
///   user (or another core) is kept, and a reader never sees half a file.
/// - Only at the start: a file the user deletes while the core runs stays
///   deleted (meaning "the defaults", or "no account") until the next one.
pub fn create_missing(config_path: &Path) -> Result<Vec<PathBuf>, String> {
    let accounts_path = config::accounts_path(config_path);
    let mut created = Vec::new();
    let keymap_path = crate::keymap::path(config_path);
    for (path, template) in [
        (config_path.to_path_buf(), config::config_template()),
        (accounts_path, config::accounts_template()),
        (keymap_path, crate::keymap::template()),
    ] {
        if create_if_missing(&path, &template)? {
            created.push(path);
        }
    }
    // The directory of the themes and the files of the four built-in
    // ones, every key commented out (`themes.md`, section 3): they change
    // nothing, and show what can be changed. A file deleted later stays
    // deleted until the next start, and the built-in theme stays.
    let themes = crate::theme::dir(config_path);
    for name in crate::theme::catalog::BUILTINS {
        let path = themes.join(format!("{name}.toml"));
        let template = crate::theme::template(name).expect("a built-in theme has a template");
        if create_if_missing(&path, &template)? {
            created.push(path);
        }
    }
    Ok(created)
}

/// Creates `path` with `text` unless something is already there; returns
/// whether it did.
fn create_if_missing(path: &Path, text: &str) -> Result<bool, String> {
    // `symlink_metadata` looks at the link itself, not at its target.
    match std::fs::symlink_metadata(path) {
        Ok(_) => return Ok(false),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("{}: {e}", path.display())),
    }
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let tmp = temp_path(dir, path);
    let result = (|| -> std::io::Result<bool> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        match std::fs::hard_link(&tmp, path) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
            Err(e) => Err(e),
        }
    })();
    // The temporary name goes in every case; on success the file stays
    // under its real name.
    let _ = std::fs::remove_file(&tmp);
    result.map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "# My accounts, in Git.\nversion = 1\n\n\
# The personal account.\n[accounts.perso]\nemail = \"me@example.fr\"\n[accounts.perso.imap]\nhost = \"imap.example.fr\"\n\
port = 993\nsecurity = \"tls\"\n\n# Work.\n[accounts.pro]\nemail = \"me@work.example\"\n[accounts.pro.imap]\nhost = \"imap.work.example\"\n\
port = 993\nsecurity = \"tls\"\n";

    fn new(key: &str) -> NewAccount {
        NewAccount {
            key: key.into(),
            email: "bob@example.org".into(),
            display_name: Some("Bob".into()),
            host: "imap.example.org".into(),
            port: 993,
            security: Security::Tls,
            username: "bob".into(),
            ca_file: None,
            read_only: false,
            auth: None,
            smtp: None,
        }
    }

    #[test]
    fn the_smtp_server_gets_its_table() {
        let mut a = new("bob");
        a.smtp = Some(NewServer {
            host: "smtp.example.org".into(),
            port: 587,
            security: Security::Starttls,
            username: "bob@example.org".into(),
            ca_file: None,
        });
        let out = add_account("", &a).unwrap();
        assert!(
            out.contains("[accounts.bob.smtp]\nhost = \"smtp.example.org\"\nport = 587\nsecurity = \"starttls\"\n"),
            "the address as user name is the default, not written: {out}"
        );
        let parsed = config::parse_accounts(&out).unwrap();
        let smtp = parsed["bob"].smtp.as_ref().unwrap();
        assert_eq!((smtp.host.as_str(), smtp.port, smtp.username.as_str()), ("smtp.example.org", 587, "bob@example.org"));
    }

    #[test]
    fn adding_keeps_comments_and_order_and_writes_no_default() {
        let out = add_account(TEXT, &new("bob")).unwrap();
        assert!(out.starts_with(TEXT.trim_end()), "the existing text is untouched:\n{out}");
        assert!(out.contains("[accounts.bob]\nemail = \"bob@example.org\"\ndisplay_name = \"Bob\""), "{out}");
        assert!(out.contains("[accounts.bob.imap]\nhost = \"imap.example.org\"\nport = 993\nsecurity = \"tls\"\nusername = \"bob\""), "{out}");
        assert_eq!(config::parse_accounts(&out).unwrap().len(), 3);
        // A file of comments only keeps them on top.
        let out = add_account("# My settings.\n", &new("bob")).unwrap();
        assert!(out.starts_with("# My settings.\n\n[accounts.bob]"), "{out}");
        let out = add_account(&format!("{TEXT}\n# The end.\n"), &new("bob")).unwrap();
        assert!(out.find("# The end.").unwrap() < out.find("[accounts.bob]").unwrap(), "{out}");
        // Into an empty file, too.
        let out = add_account("", &new("bob")).unwrap();
        assert!(!out.contains("[accounts]\n"), "no empty [accounts] header: {out}");
        assert!(add_account(&out, &new("bob")).is_err(), "a key used already");
        assert!(add_account("", &new("a b")).is_err(), "a key with a space");
    }

    #[test]
    fn renaming_keeps_the_tables_where_they_were() {
        let out = rename_account(TEXT, "perso", "home").unwrap();
        assert!(out.contains("# The personal account.\n[accounts.home]\nemail = \"me@example.fr\"\n[accounts.home.imap]"), "{out}");
        assert!(out.find("[accounts.home]").unwrap() < out.find("[accounts.pro]").unwrap());
        assert!(out.contains("# My accounts, in Git.\nversion = 1\n"));
        assert!(rename_account(TEXT, "perso", "pro").is_err());
        assert!(rename_account(TEXT, "nobody", "x").is_err());
    }

    #[test]
    fn removing_takes_only_the_account() {
        let out = remove_account(TEXT, "pro").unwrap();
        assert!(!out.contains("accounts.pro"));
        assert!(out.contains("[accounts.perso.imap]") && out.contains("# My accounts, in Git."));
        assert_eq!(config::parse_accounts(&out).unwrap().len(), 1);
    }

    #[test]
    fn clearing_read_only_takes_only_that_key() {
        let mut a = new("bob");
        a.read_only = true;
        let text = add_account(TEXT, &a).unwrap();
        assert!(config::parse_accounts(&text).unwrap()["bob"].read_only);
        let out = clear_read_only(&text, "bob").unwrap();
        assert!(!out.contains("read_only"), "{out}");
        assert!(!config::parse_accounts(&out).unwrap()["bob"].read_only);
        // Exactly the file of an account created writable.
        assert_eq!(out, add_account(TEXT, &new("bob")).unwrap());
        // Already writable: the file unchanged.
        assert_eq!(clear_read_only(TEXT, "perso").unwrap(), TEXT);
        assert!(clear_read_only(TEXT, "nobody").is_err());
    }

    #[test]
    fn the_file_is_written_atomically_and_read_again_first() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("accounts.toml");
        std::fs::write(&path, TEXT).unwrap();
        // An edit by hand after the core last read the file is kept.
        std::fs::write(&path, format!("{TEXT}\n# added by hand\n")).unwrap();
        update(&path, |t| add_account(t, &new("bob"))).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# added by hand") && text.contains("[accounts.bob]"));
        // A change that would make the file invalid writes nothing.
        let before = std::fs::read_to_string(&path).unwrap();
        assert!(update(&path, |_| Err("refused".into())).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
        // No temporary file is left.
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn setting_the_theme_keeps_the_rest_of_the_file() {
        // The file the core creates: every key commented out.
        let template = config::config_template();
        let out = set_theme(&template, "clean").unwrap();
        assert_eq!(config::parse(&out, "").unwrap().config.ui.theme, "clean");
        // Only one line more: the comments are all kept.
        assert_eq!(out.lines().count(), template.lines().count() + 1, "{out}");
        assert!(out.contains("[ui]\ntheme = \"clean\"\n"), "{out}");
        // Again: the value is replaced, not written twice.
        let again = set_theme(&out, "regular").unwrap();
        assert_eq!(again.lines().count(), out.lines().count());
        assert_eq!(config::parse(&again, "").unwrap().config.ui.theme, "regular");

        // A hand-written file: its comments, order and other keys stay.
        let mine = "# mine\n[ui]\ntext_size = 14 # my eyes\ntheme = \"geek\"   # for now\n\n[sync]\npoll_interval_secs = 120\n";
        let out = set_theme(mine, "clean").unwrap();
        assert_eq!(out, mine.replace("theme = \"geek\"", "theme = \"clean\""));

        // No [ui] table: one is added.
        let out = set_theme("[sync]\npoll_interval_secs = 120\n", "clean").unwrap();
        assert_eq!(config::parse(&out, "").unwrap().config.ui.theme, "clean");
        assert_eq!(config::parse(&out, "").unwrap().config.sync.poll_interval_secs, 120);
        assert_eq!(set_theme("", "clean").unwrap(), "[ui]\ntheme = \"clean\"\n");
    }

    #[test]
    fn setting_the_compose_format_keeps_the_rest_of_the_file() {
        // The file the core creates: every key commented out. The value
        // goes under `[compose]`, the comments are all kept.
        let template = config::config_template();
        let out = set_compose_format(&template, "html").unwrap();
        assert_eq!(config::parse(&out, "").unwrap().config.compose.format, "html");
        assert_eq!(out.lines().count(), template.lines().count() + 1, "{out}");
        assert!(out.contains("[compose]\nformat = \"html\"\n"), "{out}");
        // Again: the value is replaced, not written twice.
        let back = set_compose_format(&out, "text").unwrap();
        assert_eq!(back.lines().count(), out.lines().count());
        assert_eq!(config::parse(&back, "").unwrap().config.compose.format, "text");

        // A hand-written file: its comments and other keys stay.
        let mine = "[compose]\nundo_send_seconds = 5\nformat = \"text\" # mine\n\n[ui]\ntheme = \"clean\"\n";
        let out = set_compose_format(mine, "html").unwrap();
        assert_eq!(out, mine.replace("format = \"text\"", "format = \"html\""));

        // No [compose] table: one is added, the rest kept.
        let out = set_compose_format("[ui]\ntheme = \"clean\"\n", "html").unwrap();
        let c = config::parse(&out, "").unwrap().config;
        assert_eq!((c.compose.format, c.ui.theme.as_str()), ("html", "clean"));
    }

    #[test]
    fn a_refused_compose_format_is_not_written() {
        assert!(set_compose_format("", "rtf").unwrap_err().contains("text or html"));
        assert!(set_compose_format("[sync]\npoll_interval_secs = 1\n", "html").unwrap_err().contains("fix it first"));
        assert!(set_compose_format("compose = 3\n", "html").is_err());
    }

    #[test]
    fn a_refused_file_or_name_is_not_written() {
        assert!(set_theme("[sync]\npoll_interval_secs = 1\n", "clean").unwrap_err().contains("fix it first"));
        assert!(set_theme("", "../evil").unwrap_err().contains("cannot name a theme"));
        assert!(set_theme("ui = 3\n", "clean").is_err());
    }

    #[test]
    fn missing_files_are_created_from_the_templates() {
        let dir = tempfile::tempdir().unwrap();
        // A directory that does not exist yet, like ~/.config/nicemailer
        // on a new Mac.
        let path = dir.path().join("nicemailer/config.toml");
        let created = create_missing(&path).unwrap();
        let themes = dir.path().join("nicemailer/themes");
        assert_eq!(
            created,
            vec![
                path.clone(),
                dir.path().join("nicemailer/accounts.toml"),
                dir.path().join("nicemailer/keymap.toml"),
                themes.join("geek.toml"),
                themes.join("clean.toml"),
                themes.join("regular.toml"),
                themes.join("minimal.toml"),
            ]
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), config::config_template());
        assert_eq!(std::fs::read_to_string(&created[1]).unwrap(), config::accounts_template());
        assert_eq!(std::fs::read_to_string(&created[2]).unwrap(), crate::keymap::template());
        assert_eq!(std::fs::read_to_string(&created[4]).unwrap(), crate::theme::template("clean").unwrap());
        // No temporary file is left: three files and the directory of the
        // themes, which holds the four themes.
        assert_eq!(std::fs::read_dir(path.parent().unwrap()).unwrap().count(), 4);
        assert_eq!(std::fs::read_dir(&themes).unwrap().count(), 4);
        // The created files give the defaults and no account.
        let parsed = config::parse(&config::config_template(), &config::accounts_template()).unwrap();
        assert_eq!(parsed.config, config::Config::default());
        // Once there, nothing is created again.
        assert!(create_missing(&path).unwrap().is_empty());
    }

    #[test]
    fn an_account_added_to_the_created_file_goes_after_its_comments() {
        // The `accounts.toml` the core creates ends with the commented
        // example: the new account comes after it, not inside it.
        let template = config::accounts_template();
        let out = add_account(&template, &new("bob")).unwrap();
        assert!(out.starts_with(template.trim_end()), "the template is kept whole:\n{out}");
        assert!(out.find("# poll_interval_secs = 600").unwrap() < out.find("[accounts.bob]").unwrap(), "{out}");
        assert!(!out.contains("[accounts]\n"), "no empty [accounts] header: {out}");
        assert_eq!(config::parse_accounts(&out).unwrap().len(), 1);
    }

    #[test]
    fn an_existing_file_is_never_touched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // Empty, or invalid: still the user's.
        std::fs::write(&path, "").unwrap();
        std::fs::write(dir.path().join("accounts.toml"), "not toml [").unwrap();
        std::fs::write(dir.path().join("keymap.toml"), "").unwrap();
        std::fs::create_dir(dir.path().join("themes")).unwrap();
        for name in ["geek", "clean", "regular", "minimal"] {
            std::fs::write(dir.path().join(format!("themes/{name}.toml")), "").unwrap();
        }
        assert!(create_missing(&path).unwrap().is_empty());
        assert_eq!(std::fs::read_to_string(dir.path().join("themes/clean.toml")).unwrap(), "");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "");
        assert_eq!(std::fs::read_to_string(dir.path().join("accounts.toml")).unwrap(), "not toml [");
        // Only the missing one of the two is created.
        std::fs::remove_file(dir.path().join("accounts.toml")).unwrap();
        assert_eq!(create_missing(&path).unwrap(), vec![dir.path().join("accounts.toml")]);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "");
    }

    #[test]
    fn a_dangling_link_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // A dotfiles repository not cloned yet: the link points nowhere.
        std::os::unix::fs::symlink(dir.path().join("dotfiles/config.toml"), &path).unwrap();
        let created = create_missing(&path).unwrap();
        assert_eq!(created[..2], [dir.path().join("accounts.toml"), dir.path().join("keymap.toml")]);
        assert_eq!(created.len(), 6, "and the four themes: {created:?}");
        assert!(std::fs::symlink_metadata(&path).unwrap().file_type().is_symlink());
        assert!(!dir.path().join("dotfiles").exists());
    }
}
