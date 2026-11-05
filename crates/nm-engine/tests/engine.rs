//! The engine applies `config.toml`: reloads, refusals, accounts enabled and
//! disabled, secrets bound to their server (`configuration.md`, section 12).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use nm_api::Event;
use nm_engine::Engine;
use nm_engine::config::{AccountConfig, SecretString};
use nm_engine::secrets::{self, ConfigPasswords, Credential, NoSecrets, Refusal, Secrets};
use nm_store::Store;

/// A file of the test, and a store in memory.
struct Setup {
    _dir: tempfile::TempDir,
    path: PathBuf,
    store: Store,
}

impl Setup {
    fn new() -> Setup {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        Setup { _dir: dir, path, store: Store::open_in_memory().unwrap() }
    }

    fn write(&self, text: &str) {
        std::fs::write(&self.path, text).unwrap();
    }

    fn engine(&self, secrets: Box<dyn Secrets>) -> Engine {
        Engine::new(self.path.clone(), secrets)
    }
}

/// An account `key` on `host`, with a password in the file.
fn account(key: &str, host: &str) -> String {
    format!("[accounts.{key}]\nemail = \"{key}@example.com\"\n[accounts.{key}.imap]\nhost = \"{host}\"\npassword = \"pw\"\n")
}

fn names(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .map(|e| serde_json::to_value(e).unwrap()["method"].as_str().unwrap().to_string())
        .collect()
}

fn audit(store: &Store) -> Vec<(String, String)> {
    let mut stmt = store.conn().prepare("SELECT entity, action FROM audit_log ORDER BY id").unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect()
}

#[test]
fn no_file_means_no_account_and_the_defaults() {
    let mut s = Setup::new();
    let mut engine = s.engine(Box::new(ConfigPasswords));
    let events = engine.reload(&mut s.store, true).unwrap();
    assert!(events.is_empty(), "{events:?}");
    assert_eq!(engine.config(), &nm_engine::config::Config::default());
    assert!(engine.error().is_none());
    assert!(s.store.list_account_rows().unwrap().is_empty());
}

#[test]
fn an_invalid_file_at_startup_keeps_the_defaults_and_says_where() {
    let mut s = Setup::new();
    s.write(&format!("{}[sync]\npoll_interval_secs = 1\n", account("perso", "h")));
    let mut engine = s.engine(Box::new(ConfigPasswords));
    let events = engine.reload(&mut s.store, true).unwrap();
    match &events[..] {
        [Event::ConfigError { line, key, .. }] => {
            assert_eq!(*line, Some(7));
            assert_eq!(key.as_deref(), Some("sync.poll_interval_secs"));
        }
        other => panic!("{other:?}"),
    }
    // Nothing of the file is applied, not even its valid account.
    assert!(engine.config().accounts.is_empty());
    assert!(s.store.list_account_rows().unwrap().is_empty());
}

#[test]
fn an_invalid_file_later_keeps_the_previous_settings() {
    let mut s = Setup::new();
    s.write(&format!("[sync]\npoll_interval_secs = 60\n{}", account("perso", "h")));
    let mut engine = s.engine(Box::new(ConfigPasswords));
    engine.reload(&mut s.store, true).unwrap();
    assert_eq!(engine.config().sync.poll_interval_secs, 60);

    s.write("[sync]\npoll_interval_secs = 90\n[accounts.perso]\nemail = \"oops\"\n");
    let events = engine.reload(&mut s.store, false).unwrap();
    assert_eq!(names(&events), vec!["ConfigError"]);
    assert_eq!(engine.config().sync.poll_interval_secs, 60, "previous settings kept");
    assert!(engine.config().accounts.contains_key("perso"));
    assert!(s.store.list_account_rows().unwrap().iter().all(|r| !r.disabled), "the account is not disabled");
    assert!(engine.error().is_some());
    // Further events on the same broken file announce nothing more.
    assert!(engine.reload(&mut s.store, false).unwrap().is_empty());

    // Fixed: applied, and announced.
    s.write(&format!("[sync]\npoll_interval_secs = 90\n{}", account("perso", "h")));
    let events = engine.reload(&mut s.store, false).unwrap();
    match &events[..] {
        // The account's effective interval changed with the general one.
        [Event::ConfigChanged { keys }, Event::AccountChanged { .. }] => {
            assert_eq!(keys, &["accounts.perso.sync.poll_interval_secs", "sync.poll_interval_secs"]);
        }
        other => panic!("{other:?}"),
    }
    assert!(engine.error().is_none());
    assert_eq!(engine.config().sync.poll_interval_secs, 90);
}

#[test]
fn fixing_a_file_back_to_the_applied_one_clears_the_error() {
    let mut s = Setup::new();
    let good = account("perso", "h");
    s.write(&good);
    let mut engine = s.engine(Box::new(ConfigPasswords));
    engine.reload(&mut s.store, true).unwrap();
    s.write("not toml at all = = =");
    assert_eq!(names(&engine.reload(&mut s.store, false).unwrap()), vec!["ConfigError"]);
    s.write(&good);
    match &engine.reload(&mut s.store, false).unwrap()[..] {
        [Event::ConfigChanged { keys }] => assert!(keys.is_empty()),
        other => panic!("{other:?}"),
    }
    assert!(engine.error().is_none());
}

#[test]
fn the_same_text_twice_does_nothing() {
    let mut s = Setup::new();
    s.write(&account("perso", "h"));
    let mut engine = s.engine(Box::new(ConfigPasswords));
    engine.reload(&mut s.store, true).unwrap();
    s.write(&format!("{}\n# a comment\n", account("perso", "h")));
    // A new text, but the same settings: no change to announce.
    assert!(engine.reload(&mut s.store, false).unwrap().is_empty());
    assert!(engine.reload(&mut s.store, false).unwrap().is_empty());
}

#[test]
fn an_unknown_key_is_a_warning_and_the_rest_applies() {
    let mut s = Setup::new();
    s.write(&format!("{}[sync]\npoll_interval_secs = 45\nwhatever = 1\n", account("perso", "h")));
    let mut engine = s.engine(Box::new(ConfigPasswords));
    engine.reload(&mut s.store, true).unwrap();
    assert!(engine.error().is_none());
    assert_eq!(engine.config().sync.poll_interval_secs, 45);
    assert!(engine.warnings().iter().any(|w| w.key.as_deref() == Some("sync.whatever")));
}

#[test]
fn a_change_is_recorded_and_announced() {
    let mut s = Setup::new();
    s.write(&account("perso", "h"));
    let mut engine = s.engine(Box::new(ConfigPasswords));
    let events = engine.reload(&mut s.store, true).unwrap();
    // At startup: the account is created, nothing else is announced.
    assert_eq!(names(&events), vec!["AccountChanged"]);
    assert_eq!(audit(&s.store), vec![("account".into(), "create".into())]);

    s.write(&format!("{}[sync]\npoll_interval_secs = 600\n", account("perso", "h")));
    let events = engine.reload(&mut s.store, false).unwrap();
    // The account's effective setting changed too, so it is announced.
    assert_eq!(names(&events), vec!["ConfigChanged", "AccountChanged"]);
    let detail: String = s
        .store
        .conn()
        .query_row("SELECT detail FROM audit_log WHERE entity = 'config' AND action = 'reload'", [], |r| r.get(0))
        .unwrap();
    let detail: serde_json::Value = serde_json::from_str(&detail).unwrap();
    assert_eq!(detail["keys"], serde_json::json!(["accounts.perso.sync.poll_interval_secs", "sync.poll_interval_secs"]));
}

#[test]
fn an_account_removed_then_restored_keeps_its_data() {
    let mut s = Setup::new();
    s.write(&format!("{}{}", account("perso", "h"), account("pro", "h2")));
    let mut engine = s.engine(Box::new(ConfigPasswords));
    engine.reload(&mut s.store, true).unwrap();
    let rows = s.store.list_account_rows().unwrap();
    let perso = rows.iter().find(|r| r.config_key == "perso").unwrap().id;
    // Some data of the account: a folder.
    s.store
        .conn()
        .execute("INSERT INTO folder (account_id, path, role, created_by, modified_by) VALUES (?1, 'INBOX', 'inbox', 'core', 'core')", [perso])
        .unwrap();

    s.write(&account("pro", "h2"));
    let events = engine.reload(&mut s.store, false).unwrap();
    assert!(matches!(events[..], [Event::ConfigChanged { .. }, Event::AccountChanged { account_id }] if account_id == perso), "{events:?}");
    let row = s.store.list_account_rows().unwrap().into_iter().find(|r| r.id == perso).unwrap();
    assert!(row.disabled);
    let folders: i64 = s.store.conn().query_row("SELECT count(*) FROM folder WHERE account_id = ?1", [perso], |r| r.get(0)).unwrap();
    assert_eq!(folders, 1, "data kept");
    let view = engine.account_views(s.store.list_account_rows().unwrap());
    let v = view.iter().find(|v| v.id == perso).unwrap();
    assert!(!v.enabled);
    assert_eq!(v.email, None);

    s.write(&format!("{}{}", account("perso", "h"), account("pro", "h2")));
    engine.reload(&mut s.store, false).unwrap();
    let rows = s.store.list_account_rows().unwrap();
    let row = rows.iter().find(|r| r.config_key == "perso").unwrap();
    assert_eq!(row.id, perso, "same account, same id");
    assert!(!row.disabled);
    let folders: i64 = s.store.conn().query_row("SELECT count(*) FROM folder WHERE account_id = ?1", [perso], |r| r.get(0)).unwrap();
    assert_eq!(folders, 1);
    let actions: Vec<String> = audit(&s.store).into_iter().filter(|(e, _)| e == "account").map(|(_, a)| a).collect();
    assert_eq!(actions, vec!["create", "create", "disable", "enable"]);
}

#[test]
fn an_archive_account_is_never_disabled_and_its_name_cannot_be_taken() {
    let mut s = Setup::new();
    s.store.prepare_archive_account("old", "old@example.com", nm_core::Source::Cli).unwrap();
    s.write(&account("old", "h"));
    let mut engine = s.engine(Box::new(ConfigPasswords));
    engine.reload(&mut s.store, true).unwrap();
    let rows = s.store.list_account_rows().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, "archive");
    assert!(!rows[0].disabled);
    assert!(engine.warnings().iter().any(|w| w.key.as_deref() == Some("accounts.old") && w.message.contains("archive")));
    let views = engine.account_views(rows);
    assert_eq!(views[0].email.as_deref(), Some("old@example.com"));

    s.write("");
    engine.reload(&mut s.store, false).unwrap();
    assert!(!s.store.list_account_rows().unwrap()[0].disabled, "an archive account stays enabled");
}

/// Secrets entered for given servers, as the Keychain will hold them.
struct Stored(Mutex<BTreeMap<String, Credential>>);

impl Secrets for Stored {
    fn imap(&self, key: &str, _: &AccountConfig) -> Result<Option<Credential>, String> {
        Ok(self.0.lock().unwrap().get(key).cloned())
    }
}

fn stored(key: &str, host: &str) -> Box<Stored> {
    let c = Credential { host: host.into(), username: format!("{key}@example.com"), secret: SecretString::new("pw") };
    Box::new(Stored(Mutex::new(BTreeMap::from([(key.to_string(), c)]))))
}

#[test]
fn a_new_imap_host_gets_no_password_before_confirmation() {
    let mut s = Setup::new();
    s.write(&account("perso", "imap.example.com"));
    let mut engine = s.engine(stored("perso", "imap.example.com"));
    let events = engine.reload(&mut s.store, true).unwrap();
    assert_eq!(names(&events), vec!["AccountChanged"], "no error: the secret matches");

    // Someone points the account at another server.
    s.write(&account("perso", "imap.attacker.example"));
    let events = engine.reload(&mut s.store, false).unwrap();
    assert_eq!(names(&events), vec!["ConfigChanged", "AccountChanged", "AccountError"]);
    let Event::AccountError { error, .. } = &events[2] else { unreachable!() };
    assert!(error.contains("imap.example.com") && error.contains("imap.attacker.example"), "{error}");
    let view = engine.account_views(s.store.list_account_rows().unwrap());
    assert!(view[0].error.as_deref().unwrap().contains("enter the password again"));

    // And the function every login goes through refuses to give it.
    let config = engine.config().accounts["perso"].clone();
    let refusal = secrets::imap_password(&*stored("perso", "imap.example.com"), "perso", &config).unwrap_err();
    assert!(matches!(refusal, Refusal::ServerChanged { ref stored_host, .. } if stored_host == "imap.example.com"));

    // Back to the right server: the error goes away, announced as a change.
    s.write(&account("perso", "imap.example.com"));
    let events = engine.reload(&mut s.store, false).unwrap();
    assert_eq!(names(&events), vec!["ConfigChanged", "AccountChanged"]);
    assert!(engine.account_views(s.store.list_account_rows().unwrap())[0].error.is_none());
}

#[test]
fn a_changed_username_is_a_changed_server_too() {
    let config = nm_engine::config::parse(&format!("{}username = \"someone-else\"\n", account("perso", "h")))
        .unwrap()
        .config
        .accounts["perso"]
        .clone();
    let refusal = secrets::imap_password(&*stored("perso", "h"), "perso", &config).unwrap_err();
    assert!(matches!(refusal, Refusal::ServerChanged { .. }));
}

#[test]
fn a_build_without_dev_passwords_says_why_it_has_none() {
    let mut s = Setup::new();
    s.write(&account("perso", "h"));
    let mut engine = s.engine(Box::new(NoSecrets));
    let events = engine.reload(&mut s.store, true).unwrap();
    assert_eq!(names(&events), vec!["AccountChanged", "AccountError"]);
    let Event::AccountError { error, .. } = &events[1] else { unreachable!() };
    assert!(error.contains("dev-passwords"), "{error}");

    // Without a password in the file: the plain "missing" message.
    s.write("[accounts.perso]\nemail = \"p@example.com\"\n[accounts.perso.imap]\nhost = \"h\"\n");
    let events = engine.reload(&mut s.store, false).unwrap();
    let error = events.iter().find_map(|e| if let Event::AccountError { error, .. } = e { Some(error.clone()) } else { None });
    assert_eq!(error.as_deref(), Some("no password stored for this account"));
}

#[test]
fn oauth2_is_reported_as_unsupported() {
    let mut s = Setup::new();
    s.write("[accounts.g]\nemail = \"g@example.com\"\nauth = \"oauth2\"\n[accounts.g.imap]\nhost = \"imap.gmail.com\"\n");
    let mut engine = s.engine(Box::new(ConfigPasswords));
    engine.reload(&mut s.store, true).unwrap();
    let views = engine.account_views(s.store.list_account_rows().unwrap());
    assert!(views[0].error.as_deref().unwrap().contains("oauth2"));
}

#[test]
fn an_unreadable_file_is_an_error_until_it_is_readable() {
    use std::os::unix::fs::PermissionsExt;
    let mut s = Setup::new();
    s.write(&account("perso", "h"));
    let mut engine = s.engine(Box::new(ConfigPasswords));
    engine.reload(&mut s.store, true).unwrap();
    std::fs::set_permissions(&s.path, std::fs::Permissions::from_mode(0o000)).unwrap();
    let events = engine.reload(&mut s.store, false).unwrap();
    std::fs::set_permissions(&s.path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(matches!(&events[..], [Event::ConfigError { line: None, .. }]), "{events:?}");
    assert!(engine.config().accounts.contains_key("perso"), "previous settings kept");
    // Readable again, unchanged: the error is cleared.
    assert_eq!(names(&engine.reload(&mut s.store, false).unwrap()), vec!["ConfigChanged"]);
    assert!(engine.error().is_none());
}
