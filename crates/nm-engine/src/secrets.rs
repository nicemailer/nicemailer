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
}

/// Why the core will not authenticate an account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// No secret stored for this account.
    Missing,
    /// `auth = "oauth2"`, not supported yet.
    OauthUnsupported,
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
    /// The message shown to the user (`AccountError`, `list_accounts`).
    pub fn message(&self, key: &str, account: &AccountConfig) -> String {
        match self {
            Refusal::Missing => format!("no password saved for this account: nicemailer account password {key}"),
            Refusal::OauthUnsupported => "auth = \"oauth2\" is not supported yet".into(),
            Refusal::ServerChanged { stored } => format!(
                "the password was saved for {} on {}, the file now says {} on {}: \
                 save the password again to confirm the new server (nicemailer account password {key})",
                stored.username, stored.host, account.imap.username, account.imap.host
            ),
            Refusal::Unavailable(e) => e.clone(),
            Refusal::NoSmtp => "this account has no [smtp] server in accounts.toml: it cannot send".into(),
            Refusal::SmtpServerChanged { stored } => format!(
                "the SMTP password was saved for {} on {}, the file now says {}: save it again to confirm the new \
                 server (nicemailer account password {key} --smtp)",
                stored.username,
                stored.host,
                account.smtp.as_ref().map(|s| format!("{} on {}", s.username, s.host)).unwrap_or_default()
            ),
            Refusal::SmtpPasswordNeeded => format!(
                "the SMTP server {} is not of the same provider as the IMAP server {}: NiceMailer does not send it the \
                 IMAP password. Save its own: nicemailer account password {key} --smtp",
                account.smtp.as_ref().map(|s| s.host.as_str()).unwrap_or(""),
                account.imap.host
            ),
        }
    }
}

/// Whether the core could log in to the account: a secret is stored, for
/// the server the file names. Reads no secret, asks nothing: used at each
/// reading of the file to show the state of the accounts.
pub fn check(secrets: &dyn Secrets, key: &str, account: &AccountConfig) -> Result<(), Refusal> {
    if account.auth == AuthMethod::Oauth2 {
        return Err(Refusal::OauthUnsupported);
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
        return Err(Refusal::OauthUnsupported);
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
        return Err(Refusal::OauthUnsupported);
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

/// Secrets in memory, for tests: the same behavior as the Keychain, without
/// touching it.
#[derive(Default)]
pub struct MemorySecrets(Mutex<BTreeMap<String, Credential>>, Mutex<BTreeMap<String, Credential>>);

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
}
