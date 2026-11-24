//! Secrets of the accounts: where the password comes from, and the rule
//! that binds it to its server (`configuration.md`, section 6, "Secrets
//! liés au serveur").
//!
//! Secrets live in the macOS Keychain ([`crate::keychain`]), entered only in
//! the app or the CLI, and sent to the core, which alone reads and writes
//! them. There is no secret in `config.toml` or `accounts.toml`.
//!
//! # The binding to the server
//!
//! Each secret is stored with the host and the user name it was entered
//! for. If the file later names another host or user, the core refuses to
//! authenticate ([`Refusal::ServerChanged`]) until the user confirms the new
//! server by entering the password again. Without this rule, anyone able to
//! edit the file, an agent tricked by a mail included, could point an
//! account at their own server and receive the password at the next login.
//!
//! # Two reads, one of which never asks anything
//!
//! The Keychain may refuse to give a secret, or want the user's permission,
//! but it gives the *attributes* of an item (here the host and the user
//! name) to anyone. So the trait has two reads:
//! - [`Secrets::imap_binding`]: the attributes only. It never blocks on the
//!   user, so the core calls it while holding its locks, at each reading of
//!   the file, to tell the user at once that an account lacks a password or
//!   points at another server;
//! - [`Secrets::imap`]: the secret itself, only at login time, without any
//!   lock held.

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::config::{AccountConfig, AuthMethod, SecretString};

/// The server a secret was entered for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub host: String,
    pub username: String,
}

impl Binding {
    /// The server of an account's IMAP settings.
    pub fn of(account: &AccountConfig) -> Binding {
        Binding { host: account.imap.host.clone(), username: account.imap.username.clone() }
    }

    /// Whether it is the server the file names now. Host names are
    /// compared without case (the file reader lowercases them); user names
    /// exactly, since some servers distinguish them.
    pub fn matches(&self, account: &AccountConfig) -> bool {
        self.host.eq_ignore_ascii_case(&account.imap.host) && self.username == account.imap.username
    }
}

/// A secret, with the server it was entered for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credential {
    pub binding: Binding,
    pub secret: SecretString,
}

/// The OAuth2 entry of an account, stored in the Keychain in place of the
/// IMAP password, under the same derived name (`imap:<key>`): the refresh
/// token, the provider and the application id that got it, the tenant and
/// user ids, and the authorized address (`specifications/oauth2.md`,
/// section 6). The access token is never stored: it lives in memory only
/// (`crate::oauth2`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OauthCredential {
    pub binding: Binding,
    pub refresh_token: SecretString,
    pub provider: String,
    pub client_id: String,
    pub tenant: Option<String>,
    pub user_id: Option<String>,
    pub email: String,
}

/// The JSON form stored in the item's data. The binding stays in the
/// item's comment, as for a password.
#[derive(Serialize, Deserialize)]
pub struct StoredOauth {
    pub provider: String,
    pub client_id: String,
    pub refresh_token: String,
    pub tenant: Option<String>,
    pub user_id: Option<String>,
    pub email: String,
}

impl OauthCredential {
    pub fn to_stored(&self) -> StoredOauth {
        StoredOauth {
            provider: self.provider.clone(),
            client_id: self.client_id.clone(),
            refresh_token: self.refresh_token.expose().to_string(),
            tenant: self.tenant.clone(),
            user_id: self.user_id.clone(),
            email: self.email.clone(),
        }
    }

    pub fn from_stored(binding: Binding, s: StoredOauth) -> OauthCredential {
        OauthCredential {
            binding,
            refresh_token: SecretString::new(s.refresh_token),
            provider: s.provider,
            client_id: s.client_id,
            tenant: s.tenant,
            user_id: s.user_id,
            email: s.email,
        }
    }
}

/// Where the secrets of the accounts are kept.
///
/// Rust note: a *trait* is an interface: a set of methods a type promises
/// to provide. The engine works with `dyn Secrets` ("any type implementing
/// `Secrets`"): the Keychain in the core, [`MemorySecrets`] in tests.
/// `Send + Sync` allows sharing it between threads.
pub trait Secrets: Send + Sync {
    /// The server the IMAP secret of account `key` was entered for, if one
    /// is stored. Never asks the user anything.
    fn imap_binding(&self, key: &str) -> Result<Option<Binding>, String>;
    /// The IMAP secret of account `key`, with its server. The Keychain may
    /// refuse it: call it only to log in, never with a lock held.
    fn imap(&self, key: &str) -> Result<Option<Credential>, String>;
    /// Stores (or replaces) the IMAP secret of account `key`.
    fn set_imap(&self, key: &str, credential: &Credential) -> Result<(), String>;
    /// Deletes the IMAP secret of account `key` (`account remove`,
    /// `account rename`); nothing stored is not an error.
    fn remove_imap(&self, key: &str) -> Result<(), String>;
    /// The SMTP secret of account `key`, saved apart when the SMTP server
    /// wants another password than the IMAP one (`reponse.md`, section 7).
    /// Same rule as [`Secrets::imap`]: only to log in, never with a lock held.
    fn smtp(&self, key: &str) -> Result<Option<Credential>, String>;
    fn set_smtp(&self, key: &str, credential: &Credential) -> Result<(), String>;
    fn remove_smtp(&self, key: &str) -> Result<(), String>;
    /// The OAuth2 entry of account `key`, stored in the `imap:<key>` item
    /// in place of the password (`specifications/oauth2.md`, section 6).
    /// Same rule as [`Secrets::imap`]: only to log in, never with a lock held.
    fn oauth(&self, key: &str) -> Result<Option<OauthCredential>, String>;
    /// Stores (or replaces) the OAuth2 entry of account `key`. Removed by
    /// [`Secrets::remove_imap`]: it is the same item.
    fn set_oauth(&self, key: &str, credential: &OauthCredential) -> Result<(), String>;
}

/// Why the core will not authenticate an account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// No secret stored for this account.
    Missing,
    /// `auth = "oauth2"`, but the IMAP server is not a known OAuth2 server:
    /// no token is ever requested or sent to it (`oauth2.md`, section 7).
    OauthUnknownServer { host: String },
    /// `auth = "oauth2"`, but no OAuth2 entry is stored: sign in again.
    OauthMissing,
    /// The file names another server than the one the secret was entered
    /// for: the user must confirm it.
    ServerChanged { stored: Binding },
    /// The store of secrets failed (Keychain locked, access refused…).
    Unavailable(String),
    /// The account has no `[smtp]`: it cannot send.
    NoSmtp,
    /// The SMTP password saved apart was entered for another server than
    /// the one the file names now.
    SmtpServerChanged { stored: Binding },
    /// No SMTP password saved apart, and the SMTP server is not of the
    /// IMAP server's provider: the IMAP password is not sent there.
    SmtpPasswordNeeded,
}

impl Refusal {
    /// The English sentence (`AccountError`, `list_accounts`, the logs):
    /// that of its reason.
    pub fn message(&self, key: &str, account: &AccountConfig) -> String {
        self.why(key, account).message()
    }

    /// Why, as the contract says it (`nm_api::reason`): a client writes
    /// the sentence in the user's language from the reason and its values.
    pub fn why(&self, key: &str, account: &AccountConfig) -> nm_api::Why {
        use nm_api::reason as r;
        let smtp = account.smtp.as_ref();
        match self {
            Refusal::Missing => r::PASSWORD_MISSING.with("account", key),
            Refusal::OauthUnknownServer { host } => r::OAUTH2_UNKNOWN_SERVER.with("host", host.as_str()),
            Refusal::OauthMissing => r::OAUTH_REAUTH_NEEDED.with("account", key),
            Refusal::ServerChanged { stored } => r::PASSWORD_SERVER_CHANGED
                .with("account", key)
                .with("saved_user", stored.username.as_str())
                .with("saved_host", stored.host.as_str())
                .with("user", account.imap.username.as_str())
                .with("host", account.imap.host.as_str()),
            Refusal::Unavailable(e) => r::KEYCHAIN.with("system_text", e.as_str()),
            Refusal::NoSmtp => r::NO_SMTP_SERVER.with("account", key),
            Refusal::SmtpServerChanged { stored } => r::SMTP_PASSWORD_SERVER_CHANGED
                .with("account", key)
                .with("saved_user", stored.username.as_str())
                .with("saved_host", stored.host.as_str())
                .with("user", smtp.map(|s| s.username.as_str()).unwrap_or(""))
                .with("host", smtp.map(|s| s.host.as_str()).unwrap_or("")),
            Refusal::SmtpPasswordNeeded => r::SMTP_PASSWORD_NEEDED
                .with("account", key)
                .with("smtp_host", smtp.map(|s| s.host.as_str()).unwrap_or(""))
                .with("host", account.imap.host.as_str()),
        }
    }
}

/// Whether the core could log in to the account: a secret is stored, for
/// the server the file names. Reads no secret, asks nothing: used at each
/// reading of the file to show the state of the accounts.
pub fn check(secrets: &dyn Secrets, key: &str, account: &AccountConfig) -> Result<(), Refusal> {
    if account.auth == AuthMethod::Oauth2 {
        if crate::oauth2::provider_for_imap(&account.imap.host).is_none() {
            return Err(Refusal::OauthUnknownServer { host: account.imap.host.clone() });
        }
        let stored = secrets.imap_binding(key).map_err(Refusal::Unavailable)?.ok_or(Refusal::OauthMissing)?;
        if !stored.matches(account) {
            return Err(Refusal::ServerChanged { stored });
        }
        return Ok(());
    }
    let stored = secrets.imap_binding(key).map_err(Refusal::Unavailable)?.ok_or(Refusal::Missing)?;
    if !stored.matches(account) {
        return Err(Refusal::ServerChanged { stored });
    }
    Ok(())
}

/// The IMAP password of an account, if the core may send it to the server
/// named in the file. Every login goes through this function. The binding
/// is checked again on the item read, not on an earlier [`check`]: the
/// file may have changed in between.
pub fn imap_password(secrets: &dyn Secrets, key: &str, account: &AccountConfig) -> Result<SecretString, Refusal> {
    if account.auth == AuthMethod::Oauth2 {
        // The token path is used instead (`crate::oauth2`); this guard
        // keeps the JSON entry from ever being sent as a password.
        return Err(Refusal::OauthMissing);
    }
    let credential = secrets.imap(key).map_err(Refusal::Unavailable)?.ok_or(Refusal::Missing)?;
    if !credential.binding.matches(account) {
        return Err(Refusal::ServerChanged { stored: credential.binding });
    }
    Ok(credential.secret)
}

/// The SMTP password of an account (`reponse.md`, section 7, "SMTP"): the
/// one saved apart for the SMTP server (`smtp:<key>`), if any, checked
/// against the server the file names now; else the IMAP password, but only
/// when the SMTP server is of the same provider as the IMAP server (the
/// same host, or the same organizational domain: `smtp.gmail.com` and
/// `imap.gmail.com`). Without that rule, a file changed to point `[smtp]`
/// elsewhere would send the IMAP password to that server.
///
/// Rust note: `fn(&str) -> String` is a plain function pointer: the
/// caller passes the rule that computes an organizational domain (the
/// public suffix list lives in `nm-store`), which keeps this file free of it.
pub fn smtp_password(
    secrets: &dyn Secrets,
    key: &str,
    account: &AccountConfig,
    organizational_domain: fn(&str) -> String,
) -> Result<SecretString, Refusal> {
    if account.auth == AuthMethod::Oauth2 {
        return Err(Refusal::OauthMissing);
    }
    let smtp = account.smtp.as_ref().ok_or(Refusal::NoSmtp)?;
    if let Some(c) = secrets.smtp(key).map_err(Refusal::Unavailable)? {
        if !(c.binding.host.eq_ignore_ascii_case(&smtp.host) && c.binding.username == smtp.username) {
            return Err(Refusal::SmtpServerChanged { stored: c.binding });
        }
        return Ok(c.secret);
    }
    let same = smtp.host.eq_ignore_ascii_case(&account.imap.host)
        || organizational_domain(&smtp.host.to_lowercase()) == organizational_domain(&account.imap.host.to_lowercase());
    if !same {
        return Err(Refusal::SmtpPasswordNeeded);
    }
    imap_password(secrets, key, account)
}

/// The binding of the SMTP password of an account: its SMTP server.
pub fn smtp_binding(account: &AccountConfig) -> Option<Binding> {
    account.smtp.as_ref().map(|s| Binding { host: s.host.clone(), username: s.username.clone() })
}

/// The OAuth2 entry of an account, if the core may renew with it. The
/// server is checked against the compiled table first: a token is never
/// requested for, or sent to, a host outside it (`oauth2.md`, section 7).
/// The binding is checked again on the item read: the file may have
/// changed in between.
pub fn oauth_credential(secrets: &dyn Secrets, key: &str, account: &AccountConfig) -> Result<Option<OauthCredential>, Refusal> {
    if crate::oauth2::provider_for_imap(&account.imap.host).is_none() {
        return Err(Refusal::OauthUnknownServer { host: account.imap.host.clone() });
    }
    let Some(c) = secrets.oauth(key).map_err(Refusal::Unavailable)? else { return Ok(None) };
    if !c.binding.matches(account) {
        return Err(Refusal::ServerChanged { stored: c.binding });
    }
    Ok(Some(c))
}

/// Secrets in memory, for tests: the same behavior as the Keychain, without
/// touching it.
#[derive(Default)]
pub struct MemorySecrets(Mutex<BTreeMap<String, Credential>>, Mutex<BTreeMap<String, Credential>>, Mutex<BTreeMap<String, OauthCredential>>);

impl Secrets for MemorySecrets {
    fn imap_binding(&self, key: &str) -> Result<Option<Binding>, String> {
        Ok(self.0.lock().unwrap().get(key).map(|c| c.binding.clone()))
    }

    fn imap(&self, key: &str) -> Result<Option<Credential>, String> {
        Ok(self.0.lock().unwrap().get(key).cloned())
    }

    fn set_imap(&self, key: &str, credential: &Credential) -> Result<(), String> {
        self.0.lock().unwrap().insert(key.to_string(), credential.clone());
        Ok(())
    }

    fn remove_imap(&self, key: &str) -> Result<(), String> {
        self.0.lock().unwrap().remove(key);
        Ok(())
    }

    fn smtp(&self, key: &str) -> Result<Option<Credential>, String> {
        Ok(self.1.lock().unwrap().get(key).cloned())
    }

    fn set_smtp(&self, key: &str, credential: &Credential) -> Result<(), String> {
        self.1.lock().unwrap().insert(key.to_string(), credential.clone());
        Ok(())
    }

    fn remove_smtp(&self, key: &str) -> Result<(), String> {
        self.1.lock().unwrap().remove(key);
        Ok(())
    }

    fn oauth(&self, key: &str) -> Result<Option<OauthCredential>, String> {
        Ok(self.2.lock().unwrap().get(key).cloned())
    }

    fn set_oauth(&self, key: &str, credential: &OauthCredential) -> Result<(), String> {
        self.2.lock().unwrap().insert(key.to_string(), credential.clone());
        Ok(())
    }
}
