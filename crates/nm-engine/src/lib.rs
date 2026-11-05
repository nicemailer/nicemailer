//! Mail engine of the core: everything that is neither storage (`nm-store`)
//! nor the socket of the clients (`nm-daemon`).
//!
//! It depends on no client and no interface (`architecture.md`, principle
//! 1): the daemon drives it and turns what it reports into events. Phase 1
//! of `specifications/plan-imap.md` brings its foundations:
//!
//! - `config.rs`: reading and validating `config.toml`,
//! - `watch.rs`: watching the file to apply it without a restart,
//! - `secrets.rs`: where passwords come from, bound to their server,
//! - [`Engine`] (this file): the settings applied, and the accounts they
//!   declare, kept in line with the database.
//!
//! The IMAP synchronization will live here too.

pub mod config;
pub mod secrets;
pub mod watch;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use nm_api::{AccountView, ConfigProblem, Event};
use nm_store::accounts::AccountRow;
use nm_store::{Store, StoreError};

use config::{Config, Problem};
use secrets::{Refusal, Secrets};

/// The settings applied, and the state of the accounts they declare.
///
/// There is one `Engine` in the core, behind a mutex. Its methods are
/// synchronous and short (reading a small file, a few rows): they run on a
/// blocking thread of the daemon, with the store's lock held, so the events
/// they return are emitted in the order of the writes.
pub struct Engine {
    path: PathBuf,
    secrets: Box<dyn Secrets>,
    /// Settings applied: the last valid file, or the defaults.
    config: Config,
    /// Text of the last file read, valid or not (`None`: no file). Used to
    /// ignore events that did not change the file.
    last_text: Option<Option<String>>,
    /// Why the current file is refused; `None` when it is the one applied.
    error: Option<Problem>,
    /// Warnings about the file applied.
    warnings: Vec<Problem>,
    /// Id of the account of each key of the file.
    ids: BTreeMap<String, i64>,
    /// Why an account cannot connect, by account id.
    account_errors: BTreeMap<i64, String>,
}

impl Engine {
    /// An engine with the default settings and no account, until
    /// [`Engine::reload`] reads the file at `path`.
    pub fn new(path: PathBuf, secrets: Box<dyn Secrets>) -> Engine {
        Engine {
            path,
            secrets,
            config: Config::default(),
            last_text: None,
            error: None,
            warnings: Vec::new(),
            ids: BTreeMap::new(),
            account_errors: BTreeMap::new(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Settings applied.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Why the current file is refused, if it is.
    pub fn error(&self) -> Option<&Problem> {
        self.error.as_ref()
    }

    pub fn warnings(&self) -> &[Problem] {
        &self.warnings
    }

    /// Reads the file again and applies it if it is valid; returns the
    /// events to emit, in order. `startup` is true for the first read: it
    /// is not recorded as a change (there is nothing to compare it with)
    /// and announces nothing but a refusal.
    ///
    /// An invalid file is refused whole and the settings applied stay
    /// (`configuration.md`, section 4). Reading the same text again does
    /// nothing, so the several events of one save cost one parse.
    pub fn reload(&mut self, store: &mut Store, startup: bool) -> Result<Vec<Event>, StoreError> {
        let text = match config::read(&self.path) {
            Ok(t) => t,
            Err(p) => {
                // Forget the last text: once the file is readable again,
                // even unchanged, it must be parsed to clear the error.
                self.last_text = None;
                return Ok(self.refuse(p));
            }
        };
        if self.last_text.as_ref() == Some(&text) {
            return Ok(Vec::new());
        }
        self.last_text = Some(text.clone());
        let parsed = match config::parse(text.as_deref().unwrap_or("")) {
            Ok(p) => p,
            Err(p) => return Ok(self.refuse(p)),
        };

        let had_error = self.error.take().is_some();
        let keys = config::changed_keys(&self.config, &parsed.config);
        self.config = parsed.config;
        self.warnings = parsed.warnings;
        let mut events = Vec::new();
        if !startup {
            if !keys.is_empty() {
                store.record_config_reload(&keys)?;
            }
            if !keys.is_empty() || had_error {
                events.push(Event::ConfigChanged { keys: keys.clone() });
            }
        }
        events.extend(self.sync_accounts(store, &keys)?);
        Ok(events)
    }

    /// Keeps the previous settings and reports `problem`, once: the same
    /// problem found again (another event on the same broken file) is not
    /// announced twice.
    fn refuse(&mut self, problem: Problem) -> Vec<Event> {
        if self.error.as_ref() == Some(&problem) {
            return Vec::new();
        }
        self.error = Some(problem.clone());
        vec![Event::ConfigError { line: problem.line, key: problem.key, message: problem.message }]
    }

    /// Brings the accounts of the database in line with the file, and works
    /// out which accounts changed or cannot connect.
    fn sync_accounts(&mut self, store: &mut Store, keys: &[String]) -> Result<Vec<Event>, StoreError> {
        let names: Vec<&str> = self.config.accounts.keys().map(String::as_str).collect();
        let sync = store.sync_config_accounts(&names)?;
        self.warnings.extend(sync.clashes.iter().map(|k| Problem {
            line: None,
            key: Some(format!("accounts.{k}")),
            message: "this name is already used by an archive account: this account is ignored until one of \
                      the two is renamed"
                .into(),
        }));

        // Accounts whose own settings changed.
        let mut changed: BTreeSet<i64> = BTreeSet::new();
        for (key, id) in &sync.ids {
            let prefix = format!("accounts.{key}.");
            if keys.iter().any(|k| k.starts_with(&prefix)) {
                changed.insert(*id);
            }
        }
        changed.extend(sync.created.iter().chain(&sync.enabled).chain(&sync.disabled));

        // Errors, account by account.
        let mut errors = BTreeMap::new();
        for (key, id) in &sync.ids {
            let account = &self.config.accounts[key];
            if let Err(refusal) = secrets::imap_password(self.secrets.as_ref(), key, account) {
                let mut message = refusal.message(account);
                if refusal == Refusal::Missing && account.imap.password.is_some() {
                    message = "this build of the core does not read passwords from config.toml \
                               (development builds only: feature dev-passwords)"
                        .into();
                }
                errors.insert(*id, message);
            }
        }
        let mut events = Vec::new();
        let mut errored = Vec::new();
        for (id, message) in &errors {
            if self.account_errors.get(id) != Some(message) {
                errored.push(Event::AccountError { account_id: *id, error: message.clone() });
            }
        }
        // An error that went away is a change of the account's state.
        for id in self.account_errors.keys() {
            if !errors.contains_key(id) && sync.ids.values().any(|v| v == id) {
                changed.insert(*id);
            }
        }
        events.extend(changed.iter().map(|id| Event::AccountChanged { account_id: *id }));
        events.extend(errored);
        self.ids = sync.ids;
        self.account_errors = errors;
        Ok(events)
    }

    /// The `list_accounts` view of the rows of the database: their address
    /// and name come from the file, their error from the engine.
    pub fn account_views(&self, rows: Vec<AccountRow>) -> Vec<AccountView> {
        rows.into_iter()
            .map(|row| {
                let account = (row.kind == "imap" && !row.disabled)
                    .then(|| self.config.accounts.get(&row.config_key))
                    .flatten();
                AccountView {
                    id: row.id,
                    email: account.map(|a| a.email.clone()).or(row.address),
                    display_name: account.and_then(|a| a.display_name.clone()),
                    enabled: !row.disabled,
                    error: self.account_errors.get(&row.id).cloned(),
                    position: row.position,
                    key: row.config_key,
                    kind: row.kind,
                }
            })
            .collect()
    }
}

/// A [`Problem`] in the shape of the contract.
pub fn problem_view(p: &Problem) -> ConfigProblem {
    ConfigProblem { line: p.line, key: p.key.clone(), message: p.message.clone() }
}
