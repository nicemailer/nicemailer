//! Writing `config.toml` for a command (`configuration.md`, section 5):
//! adding, renaming and removing an account.
//!
//! The file belongs to the user, who may keep it in Git, with comments.
//! So:
//! - it is edited in place by `toml_edit`, which keeps comments, order and
//!   formatting: only the account's tables change;
//! - it is read again right before the change, so that an edit made by
//!   hand meanwhile is kept, and only the account's tables are touched;
//! - the result must be a valid file (`config::parse`), else nothing is
//!   written;
//! - it is written atomically: a temporary file next to it, then `rename`.
//!   A crash leaves the old file or the new one, never half of one.
//!
//! The core never writes the file on its own, only at a client's request.

use std::io::Write;
use std::path::Path;

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
}

fn doc(text: &str) -> Result<DocumentMut, String> {
    text.parse::<DocumentMut>().map_err(|e| format!("config.toml cannot be read: {e}"))
}

/// The `[accounts]` table, created (implicit: no `[accounts]` header of its
/// own) if the file has none.
fn accounts(doc: &mut DocumentMut) -> Result<&mut Table, String> {
    if !doc.contains_key("accounts") {
        let mut t = Table::new();
        t.set_implicit(true);
        doc.insert("accounts", Item::Table(t));
    }
    doc["accounts"].as_table_mut().ok_or_else(|| "accounts is not a table in config.toml".to_string())
}

/// An account key: what the file and the Keychain item name can hold
/// without quoting surprises.
pub fn check_key(key: &str) -> Result<(), String> {
    if key.is_empty() || key.len() > 64 || !key.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')) {
        return Err(format!("invalid account key {key:?}: letters, digits, - and _, 64 characters at most"));
    }
    Ok(())
}

/// The file with a new account added at its end.
pub fn add_account(text: &str, a: &NewAccount) -> Result<String, String> {
    check_key(&a.key)?;
    let mut doc = doc(text)?;
    if accounts(&mut doc)?.contains_key(&a.key) {
        return Err(format!("config.toml already has an account {}", a.key));
    }
    let mut t = Table::new();
    t["email"] = value(&a.email);
    if let Some(n) = &a.display_name {
        t["display_name"] = value(n);
    }
    let mut imap = Table::new();
    imap["host"] = value(&a.host);
    imap["port"] = value(a.port as i64);
    imap["security"] = value(match a.security {
        Security::Tls => "tls",
        Security::Starttls => "starttls",
        Security::None => "none",
    });
    // The user name only when it is not the address (the default).
    if a.username != a.email {
        imap["username"] = value(&a.username);
    }
    if let Some(ca) = &a.ca_file {
        imap["ca_file"] = value(ca);
    }
    t.insert("imap", Item::Table(imap));
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

/// The file with the account `from` renamed `to`, its tables where they
/// were.
pub fn rename_account(text: &str, from: &str, to: &str) -> Result<String, String> {
    check_key(to)?;
    let mut doc = doc(text)?;
    let accounts = accounts(&mut doc)?;
    if accounts.contains_key(to) {
        return Err(format!("config.toml already has an account {to}"));
    }
    let item = accounts.remove(from).ok_or_else(|| format!("config.toml has no account {from}"))?;
    // The tables keep their `position`: they are written back where they
    // were, under the new name.
    accounts.insert(to, item);
    finish(doc)
}

/// The file without the account `key`.
pub fn remove_account(text: &str, key: &str) -> Result<String, String> {
    let mut doc = doc(text)?;
    let accounts = accounts(&mut doc)?;
    accounts.remove(key).ok_or_else(|| format!("config.toml has no account {key}"))?;
    finish(doc)
}

/// The new text, refused if the core would refuse it.
fn finish(doc: DocumentMut) -> Result<String, String> {
    let text = doc.to_string();
    config::parse(&text).map_err(|p| format!("the file would be invalid: {p}"))?;
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
    let tmp = dir.join(format!(".config.toml.{:016x}", rand::random::<u64>()));
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

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "# My settings, in Git.\nversion = 1\n\n[sync]\npoll_interval_secs = 120 # faster\n\n\
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
        }
    }

    #[test]
    fn adding_keeps_comments_and_order_and_writes_no_default() {
        let out = add_account(FILE, &new("bob")).unwrap();
        assert!(out.starts_with(FILE.trim_end()), "the existing text is untouched:\n{out}");
        assert!(out.contains("[accounts.bob]\nemail = \"bob@example.org\"\ndisplay_name = \"Bob\""), "{out}");
        assert!(out.contains("[accounts.bob.imap]\nhost = \"imap.example.org\"\nport = 993\nsecurity = \"tls\"\nusername = \"bob\""), "{out}");
        let parsed = config::parse(&out).unwrap();
        assert_eq!(parsed.config.accounts.len(), 3);
        // A file of comments only keeps them on top.
        let out = add_account("# My settings.\n", &new("bob")).unwrap();
        assert!(out.starts_with("# My settings.\n\n[accounts.bob]"), "{out}");
        let out = add_account(&format!("{FILE}\n# The end.\n"), &new("bob")).unwrap();
        assert!(out.find("# The end.").unwrap() < out.find("[accounts.bob]").unwrap(), "{out}");
        // Into an empty file, too.
        let out = add_account("", &new("bob")).unwrap();
        assert!(!out.contains("[accounts]\n"), "no empty [accounts] header: {out}");
        assert!(add_account(&out, &new("bob")).is_err(), "a key used already");
        assert!(add_account("", &new("a b")).is_err(), "a key with a space");
    }

    #[test]
    fn renaming_keeps_the_tables_where_they_were() {
        let out = rename_account(FILE, "perso", "home").unwrap();
        assert!(out.contains("# The personal account.\n[accounts.home]\nemail = \"me@example.fr\"\n[accounts.home.imap]"), "{out}");
        assert!(out.find("[accounts.home]").unwrap() < out.find("[accounts.pro]").unwrap());
        assert!(out.contains("poll_interval_secs = 120 # faster"));
        assert!(rename_account(FILE, "perso", "pro").is_err());
        assert!(rename_account(FILE, "nobody", "x").is_err());
    }

    #[test]
    fn removing_takes_only_the_account() {
        let out = remove_account(FILE, "pro").unwrap();
        assert!(!out.contains("accounts.pro"));
        assert!(out.contains("[accounts.perso.imap]") && out.contains("# My settings, in Git."));
        assert_eq!(config::parse(&out).unwrap().config.accounts.len(), 1);
    }

    #[test]
    fn the_file_is_written_atomically_and_read_again_first() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, FILE).unwrap();
        // An edit by hand after the core last read the file is kept.
        std::fs::write(&path, format!("{FILE}\n# added by hand\n")).unwrap();
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
}
