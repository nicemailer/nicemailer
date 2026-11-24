//! The engine applies `config.toml` and `accounts.toml`: reloads, refusals,
//! accounts enabled and disabled, secrets bound to their server
//! (`configuration.md`, section 12).

use std::path::PathBuf;
use std::sync::Arc;

use nm_api::Event;
use nm_engine::Engine;
use nm_engine::config::SecretString;
use nm_engine::secrets::{self, Binding, Credential, MemorySecrets, Refusal, Secrets};
use nm_store::Store;

/// The files of the test, and a store in memory.
struct Setup {
    _dir: tempfile::TempDir,
    /// `config.toml`; `accounts.toml` is next to it.
    path: PathBuf,
    store: Store,
}

impl Setup {
    fn new() -> Setup {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        Setup { _dir: dir, path, store: Store::open_in_memory().unwrap() }
    }

    /// Writes `config.toml`.
    fn write(&self, text: &str) {
        std::fs::write(&self.path, text).unwrap();
    }

    /// Writes `accounts.toml`.
    fn accounts(&self, text: &str) {
        std::fs::write(nm_engine::config::accounts_path(&self.path), text).unwrap();
    }

    fn engine(&self, secrets: Arc<dyn Secrets>) -> Engine {
        Engine::new(self.path.clone(), secrets)
    }
}

/// An account `key` on `host`.
fn account(key: &str, host: &str) -> String {
    format!("[accounts.{key}]\nemail = \"{key}@example.com\"\n[accounts.{key}.imap]\nhost = \"{host}\"\n")
}

/// Secrets with a password saved for `key` on `host` (user `key@example.com`).
fn stored(key: &str, host: &str) -> Arc<MemorySecrets> {
    let secrets = MemorySecrets::default();
    let binding = Binding { host: host.into(), username: format!("{key}@example.com") };
    secrets.set_imap(key, &Credential { binding, secret: SecretString::new("pw") }).unwrap();
    Arc::new(secrets)
}

/// No password saved.
fn none() -> Arc<MemorySecrets> {
    Arc::new(MemorySecrets::default())
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
    let mut engine = s.engine(stored("perso", "h"));
    let events = engine.reload(&mut s.store, true).unwrap();
    assert!(events.is_empty(), "{events:?}");
    assert_eq!(engine.config(), &nm_engine::config::Config::default());
    assert!(engine.error().is_none());
    assert!(s.store.list_account_rows().unwrap().is_empty());
}

#[test]
fn an_invalid_file_at_startup_keeps_the_defaults_and_says_where() {
    let mut s = Setup::new();
    s.accounts(&account("perso", "h"));
    s.write("version = 1\n[sync]\npoll_interval_secs = 1\n");
    let mut engine = s.engine(stored("perso", "h"));
    let events = engine.reload(&mut s.store, true).unwrap();
    match &events[..] {
        [Event::ConfigError { file, line, key, .. }] => {
            assert_eq!(file, "config.toml");
            assert_eq!(*line, Some(3));
            assert_eq!(key.as_deref(), Some("sync.poll_interval_secs"));
        }
        other => panic!("{other:?}"),
    }
    // Nothing is applied, not even the valid accounts.toml.
    assert!(engine.config().accounts.is_empty());
    assert!(s.store.list_account_rows().unwrap().is_empty());
}

#[test]
fn an_invalid_file_later_keeps_the_previous_settings() {
    let mut s = Setup::new();
    s.write("[sync]\npoll_interval_secs = 60\n");
    s.accounts(&account("perso", "h"));
    let mut engine = s.engine(stored("perso", "h"));
    engine.reload(&mut s.store, true).unwrap();
    assert_eq!(engine.config().sync.poll_interval_secs, 60);

    // A valid config.toml, an invalid accounts.toml: both refused.
    s.write("[sync]\npoll_interval_secs = 90\n");
    s.accounts("[accounts.perso]\nemail = \"oops\"\n");
    let events = engine.reload(&mut s.store, false).unwrap();
    assert!(matches!(&events[..], [Event::ConfigError { file, line: Some(2), .. }] if file == "accounts.toml"), "{events:?}");
    assert_eq!(engine.config().sync.poll_interval_secs, 60, "previous settings kept");
    assert!(engine.config().accounts.contains_key("perso"));
    assert!(s.store.list_account_rows().unwrap().iter().all(|r| !r.disabled), "the account is not disabled");
    assert!(engine.error().is_some());
    // Further events on the same broken file announce nothing more.
    assert!(engine.reload(&mut s.store, false).unwrap().is_empty());

    // Fixed: applied, and announced.
    s.accounts(&account("perso", "h"));
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
    s.accounts(&good);
    let mut engine = s.engine(stored("perso", "h"));
    engine.reload(&mut s.store, true).unwrap();
    s.accounts("not toml at all = = =");
    assert_eq!(names(&engine.reload(&mut s.store, false).unwrap()), vec!["ConfigError"]);
    s.accounts(&good);
    match &engine.reload(&mut s.store, false).unwrap()[..] {
        [Event::ConfigChanged { keys }] => assert!(keys.is_empty()),
        other => panic!("{other:?}"),
    }
    assert!(engine.error().is_none());
}

#[test]
fn the_same_text_twice_does_nothing() {
    let mut s = Setup::new();
    s.accounts(&account("perso", "h"));
    let mut engine = s.engine(stored("perso", "h"));
    engine.reload(&mut s.store, true).unwrap();
    s.accounts(&format!("{}\n# a comment\n", account("perso", "h")));
    s.write("# a comment in config.toml too\n");
    // A new text, but the same settings: no change to announce.
    assert!(engine.reload(&mut s.store, false).unwrap().is_empty());
    assert!(engine.reload(&mut s.store, false).unwrap().is_empty());
}

#[test]
fn an_unknown_key_is_a_warning_and_the_rest_applies() {
    let mut s = Setup::new();
    s.accounts(&account("perso", "h"));
    s.write("[sync]\npoll_interval_secs = 45\nwhatever = 1\n");
    let mut engine = s.engine(stored("perso", "h"));
    engine.reload(&mut s.store, true).unwrap();
    assert!(engine.error().is_none());
    assert_eq!(engine.config().sync.poll_interval_secs, 45);
    assert!(engine.warnings().iter().any(|w| w.key.as_deref() == Some("sync.whatever")));
}

#[test]
fn a_change_is_recorded_and_announced() {
    let mut s = Setup::new();
    s.accounts(&account("perso", "h"));
    let mut engine = s.engine(stored("perso", "h"));
    let events = engine.reload(&mut s.store, true).unwrap();
    // At startup: the account is created, nothing else is announced.
    assert_eq!(names(&events), vec!["AccountChanged"]);
    assert_eq!(audit(&s.store), vec![("account".into(), "create".into())]);

    s.write("[sync]\npoll_interval_secs = 600\n");
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
    s.accounts(&format!("{}{}", account("perso", "h"), account("pro", "h2")));
    let mut engine = s.engine(stored("perso", "h"));
    engine.reload(&mut s.store, true).unwrap();
    let rows = s.store.list_account_rows().unwrap();
    let perso = rows.iter().find(|r| r.config_key == "perso").unwrap().id;
    // Some data of the account: a folder.
    s.store
        .conn()
        .execute("INSERT INTO folder (account_id, path, role, created_by, modified_by) VALUES (?1, 'INBOX', 'inbox', 'core', 'core')", [perso])
        .unwrap();

    s.accounts(&account("pro", "h2"));
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

    s.accounts(&format!("{}{}", account("perso", "h"), account("pro", "h2")));
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
    s.accounts(&account("old", "h"));
    let mut engine = s.engine(stored("perso", "h"));
    engine.reload(&mut s.store, true).unwrap();
    let rows = s.store.list_account_rows().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, "archive");
    assert!(!rows[0].disabled);
    assert!(engine.warnings().iter().any(|w| w.key.as_deref() == Some("accounts.old") && w.message.contains("archive")));
    let views = engine.account_views(rows);
    assert_eq!(views[0].email.as_deref(), Some("old@example.com"));

    s.accounts("");
    engine.reload(&mut s.store, false).unwrap();
    assert!(!s.store.list_account_rows().unwrap()[0].disabled, "an archive account stays enabled");
}

#[test]
fn a_new_imap_host_gets_no_password_before_confirmation() {
    let mut s = Setup::new();
    s.accounts(&account("perso", "imap.example.com"));
    let mut engine = s.engine(stored("perso", "imap.example.com"));
    let events = engine.reload(&mut s.store, true).unwrap();
    assert_eq!(names(&events), vec!["AccountChanged"], "no error: the secret matches");

    // Someone points the account at another server.
    s.accounts(&account("perso", "imap.attacker.example"));
    let events = engine.reload(&mut s.store, false).unwrap();
    assert_eq!(names(&events), vec!["ConfigChanged", "AccountChanged", "AccountError"]);
    let Event::AccountError { error, .. } = &events[2] else { unreachable!() };
    assert!(error.contains("imap.example.com") && error.contains("imap.attacker.example"), "{error}");
    let view = engine.account_views(s.store.list_account_rows().unwrap());
    assert!(view[0].error.as_deref().unwrap().contains("save the password again"));

    // And the function every login goes through refuses to give it.
    let config = engine.config().accounts["perso"].clone();
    let refusal = secrets::imap_password(&*stored("perso", "imap.example.com"), "perso", &config).unwrap_err();
    assert!(matches!(refusal, Refusal::ServerChanged { ref stored } if stored.host == "imap.example.com"));

    // Back to the right server: the error goes away, announced as a change.
    s.accounts(&account("perso", "imap.example.com"));
    let events = engine.reload(&mut s.store, false).unwrap();
    assert_eq!(names(&events), vec!["ConfigChanged", "AccountChanged"]);
    assert!(engine.account_views(s.store.list_account_rows().unwrap())[0].error.is_none());
}

#[test]
fn a_changed_username_is_a_changed_server_too() {
    let config = nm_engine::config::parse("", &format!("{}username = \"someone-else\"\n", account("perso", "h")))
        .unwrap()
        .config
        .accounts["perso"]
        .clone();
    let refusal = secrets::imap_password(&*stored("perso", "h"), "perso", &config).unwrap_err();
    assert!(matches!(refusal, Refusal::ServerChanged { .. }));
}

#[test]
fn a_missing_password_is_an_error_until_it_is_saved() {
    let mut s = Setup::new();
    s.accounts(&account("perso", "imap.example.com"));
    let secrets = none();
    let mut engine = s.engine(secrets.clone());
    let events = engine.reload(&mut s.store, true).unwrap();
    assert_eq!(names(&events), vec!["AccountChanged", "AccountError"]);
    let Event::AccountError { error, .. } = &events[1] else { unreachable!() };
    assert!(error.contains("nicemailer account password perso"), "{error}");
    // Nothing changed: rechecking announces nothing.
    assert!(engine.recheck_accounts().is_empty());

    // Saved, for the server of the file: the error goes away.
    let config = engine.config().accounts["perso"].clone();
    secrets.set_imap("perso", &Credential { binding: Binding::of(&config), secret: SecretString::new("pw") }).unwrap();
    assert_eq!(names(&engine.recheck_accounts()), vec!["AccountChanged"]);
    assert!(engine.account_views(s.store.list_account_rows().unwrap())[0].error.is_none());
    assert_eq!(secrets::imap_password(secrets.as_ref(), "perso", &config).unwrap().expose(), "pw");
}

#[test]
fn saving_again_confirms_a_new_server() {
    let mut s = Setup::new();
    s.accounts(&account("perso", "imap.example.com"));
    let secrets = stored("perso", "imap.example.com");
    let mut engine = s.engine(secrets.clone());
    engine.reload(&mut s.store, true).unwrap();
    s.accounts(&account("perso", "imap.new.example"));
    let events = engine.reload(&mut s.store, false).unwrap();
    assert!(names(&events).contains(&"AccountError".to_string()));
    // The user saves the password again: it is now bound to the new server.
    let config = engine.config().accounts["perso"].clone();
    secrets.set_imap("perso", &Credential { binding: Binding::of(&config), secret: SecretString::new("pw2") }).unwrap();
    assert_eq!(names(&engine.recheck_accounts()), vec!["AccountChanged"]);
    assert_eq!(secrets::imap_password(secrets.as_ref(), "perso", &config).unwrap().expose(), "pw2");
}

#[test]
fn the_login_checks_the_binding_again() {
    // `check` passed when the file was read; the file changed since, and
    // the secret was replaced by one for another server: the login, which
    // reads the item itself, refuses.
    let config = nm_engine::config::parse("", &account("perso", "imap.example.com")).unwrap().config.accounts["perso"].clone();
    let secrets = stored("perso", "imap.example.com");
    assert!(secrets::check(secrets.as_ref(), "perso", &config).is_ok());
    let other = stored("perso", "imap.attacker.example");
    assert!(matches!(secrets::imap_password(other.as_ref(), "perso", &config), Err(Refusal::ServerChanged { .. })));
}

#[test]
fn an_oauth2_account_on_an_unknown_server_is_in_error_without_any_token() {
    // `imap.gmail.com` is not in the compiled OAuth2 table: no token is
    // ever requested or sent to it (`specifications/oauth2.md`, section 7).
    let mut s = Setup::new();
    s.accounts("[accounts.g]\nemail = \"g@example.com\"\nauth = \"oauth2\"\n[accounts.g.imap]\nhost = \"imap.gmail.com\"\n");
    let mut engine = s.engine(stored("perso", "h"));
    engine.reload(&mut s.store, true).unwrap();
    let views = engine.account_views(s.store.list_account_rows().unwrap());
    assert!(views[0].error.as_deref().unwrap().contains("not a known OAuth2 server"), "{:?}", views[0].error);
}

#[test]
fn an_oauth2_account_on_a_known_server_needs_a_sign_in() {
    let mut s = Setup::new();
    s.accounts(
        "[accounts.o]\nemail = \"o@outlook.com\"\nauth = \"oauth2\"\n[accounts.o.imap]\nhost = \"outlook.office365.com\"\n",
    );
    let mut engine = s.engine(stored("perso", "h"));
    engine.reload(&mut s.store, true).unwrap();
    let views = engine.account_views(s.store.list_account_rows().unwrap());
    assert!(views[0].error.as_deref().unwrap().contains("signed in again"), "{:?}", views[0].error);
    // The clients offer "Sign in again", not a password, from `auth`.
    assert_eq!(views[0].auth.as_deref(), Some("oauth2"));
}

#[test]
fn a_password_account_says_so_and_an_archive_has_no_sign_in() {
    let mut s = Setup::new();
    s.accounts(&account("perso", "imap.example.com"));
    let mut engine = s.engine(stored("perso", "imap.example.com"));
    engine.reload(&mut s.store, true).unwrap();
    let views = engine.account_views(s.store.list_account_rows().unwrap());
    assert_eq!(views[0].auth.as_deref(), Some("password"));
    // Gone from the file: nothing to sign in to.
    s.accounts("");
    engine.reload(&mut s.store, true).unwrap();
    let views = engine.account_views(s.store.list_account_rows().unwrap());
    assert!(!views[0].enabled && views[0].auth.is_none(), "{views:?}");
}

/// A refresh token refused at run time: the Keychain item is there, so
/// only the worker knows; what it reports is what `list_accounts` gives
/// (the app offers "Sign in again" from it), until the accounts are
/// checked again after a new sign-in (`oauth2.md`, section 6).
#[test]
fn a_worker_error_shows_in_the_account_until_it_is_checked_again() {
    let mut s = Setup::new();
    s.accounts(&account("perso", "imap.example.com"));
    let mut engine = s.engine(stored("perso", "imap.example.com"));
    engine.reload(&mut s.store, true).unwrap();
    let id = engine.account_id("perso").unwrap();
    let why = nm_api::reason::OAUTH_REAUTH_NEEDED.with("account", "perso");
    assert!(engine.set_worker_error(id, Some(why.clone())));
    // The same verdict again is no change.
    assert!(!engine.set_worker_error(id, Some(why)));
    let view = &engine.account_views(s.store.list_account_rows().unwrap())[0];
    assert_eq!(view.why.as_ref().unwrap()["reason"], "oauth_reauth_needed", "{view:?}");
    assert!(view.error.as_deref().unwrap().contains("account login perso"), "{view:?}");

    // A new credential saved: the worker's verdict is about the old one.
    assert_eq!(names(&engine.recheck_accounts()), vec!["AccountChanged"]);
    assert!(engine.account_views(s.store.list_account_rows().unwrap())[0].why.is_none());
    assert!(engine.recheck_accounts().is_empty());
    assert!(!engine.set_worker_error(id, None));
}

#[test]
fn an_unreadable_file_is_an_error_until_it_is_readable() {
    use std::os::unix::fs::PermissionsExt;
    let mut s = Setup::new();
    s.write("");
    s.accounts(&account("perso", "h"));
    let mut engine = s.engine(stored("perso", "h"));
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

#[test]
fn an_edit_of_either_file_is_applied() {
    let mut s = Setup::new();
    s.write("[sync]\npoll_interval_secs = 60\n");
    s.accounts(&account("perso", "h"));
    let mut engine = s.engine(stored("perso", "h"));
    engine.reload(&mut s.store, true).unwrap();
    assert_eq!(engine.config().accounts["perso"].sync.poll_interval_secs, 60, "the account inherits config.toml");
    // accounts.toml alone: an account's override.
    s.accounts(&format!("{}[accounts.perso.sync]\npoll_interval_secs = 120\n", account("perso", "h")));
    match &engine.reload(&mut s.store, false).unwrap()[..] {
        [Event::ConfigChanged { keys }, Event::AccountChanged { .. }] => assert_eq!(keys, &["accounts.perso.sync.poll_interval_secs"]),
        other => panic!("{other:?}"),
    }
    // config.toml alone: the override stays.
    s.write("[sync]\npoll_interval_secs = 90\n");
    match &engine.reload(&mut s.store, false).unwrap()[..] {
        [Event::ConfigChanged { keys }] => assert_eq!(keys, &["sync.poll_interval_secs"]),
        other => panic!("{other:?}"),
    }
    assert_eq!(engine.config().accounts["perso"].sync.poll_interval_secs, 120);
}

#[test]
fn accounts_left_in_config_toml_are_refused_not_ignored() {
    let mut s = Setup::new();
    s.write(&account("perso", "h"));
    let mut engine = s.engine(stored("perso", "h"));
    let events = engine.reload(&mut s.store, true).unwrap();
    assert!(
        matches!(&events[..], [Event::ConfigError { file, message, .. }] if file == "config.toml" && message.contains("accounts.toml")),
        "{events:?}"
    );
}
