//! Secrets of the accounts: where the password comes from, and the rule
//! that binds it to its server (`configuration.md`, section 6, "Secrets
//! liés au serveur").
//!
//! In the shipped application, secrets live in the Keychain, entered only
//! in the app or the CLI. The Keychain arrives with `add_account` (phase 7
//! of `plan-imap.md`); until then the only provider is [`ConfigPasswords`],
//! which reads the `password` key of the file, for development.
//!
//! # The binding to the server
//!
//! Each secret is stored with the host and the user name it was entered
//! for. If the file later names another host or user, the core refuses to
//! authenticate ([`Refusal::ServerChanged`]) until the user confirms the new
//! server by typing the password again. Without this rule, anyone able to
//! edit the file, an agent tricked by a mail included, could point an
//! account at their own server and receive the password at the next login.

use crate::config::{AccountConfig, SecretString};

/// A secret, with the server it was entered for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credential {
    pub host: String,
    pub username: String,
    pub secret: SecretString,
}

/// Where the secrets of the accounts are kept.
///
/// Rust note: a *trait* is an interface: a set of methods a type promises
/// to provide. The engine works with `dyn Secrets` ("any type implementing
/// `Secrets`"), so the Keychain can replace [`ConfigPasswords`] later
/// without any change to the code that uses it, and tests can provide a
/// fake. `Send + Sync` allows sharing it between threads.
pub trait Secrets: Send + Sync {
    /// The IMAP secret of the account `key`, if one is stored.
    fn imap(&self, key: &str, account: &AccountConfig) -> Result<Option<Credential>, String>;
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
    ServerChanged { stored_host: String, stored_username: String },
    /// The store of secrets failed (Keychain locked…).
    Unavailable(String),
}

impl Refusal {
    /// The message shown to the user (`AccountError`, `list_accounts`).
    pub fn message(&self, account: &AccountConfig) -> String {
        match self {
            Refusal::Missing => "no password stored for this account".into(),
            Refusal::OauthUnsupported => "auth = \"oauth2\" is not supported yet".into(),
            Refusal::ServerChanged { stored_host, stored_username } => format!(
                "the password was entered for {stored_username} on {stored_host}, the file now says {} on {}: \
                 enter the password again to confirm the new server",
                account.imap.username, account.imap.host
            ),
            Refusal::Unavailable(e) => format!("secrets unavailable: {e}"),
        }
    }
}

/// The IMAP password of an account, if the core may send it to the server
/// named in the file. Every authentication goes through this function.
pub fn imap_password(secrets: &dyn Secrets, key: &str, account: &AccountConfig) -> Result<SecretString, Refusal> {
    if account.auth == crate::config::AuthMethod::Oauth2 {
        return Err(Refusal::OauthUnsupported);
    }
    let credential = secrets.imap(key, account).map_err(Refusal::Unavailable)?.ok_or(Refusal::Missing)?;
    // Host names are compared without case (the file reader lowercases
    // them); user names exactly, since some servers distinguish them.
    if !credential.host.eq_ignore_ascii_case(&account.imap.host) || credential.username != account.imap.username {
        return Err(Refusal::ServerChanged { stored_host: credential.host, stored_username: credential.username });
    }
    Ok(credential.secret)
}

/// Development provider: the `password` key of `[accounts.<key>.imap]`.
///
/// The password is in the same file as the host, so the binding to the
/// server is always satisfied: this provider is a stand-in for the
/// Keychain, not a safe place. The core uses it only when built with the
/// `dev-passwords` feature (`nm-daemon`); a build without it ignores the
/// key and says so in the account's error.
pub struct ConfigPasswords;

impl Secrets for ConfigPasswords {
    fn imap(&self, _key: &str, account: &AccountConfig) -> Result<Option<Credential>, String> {
        Ok(account.imap.password.clone().map(|secret| Credential {
            host: account.imap.host.clone(),
            username: account.imap.username.clone(),
            secret,
        }))
    }
}

/// Provider of builds without `dev-passwords`, until the Keychain: there is
/// no secret anywhere yet.
pub struct NoSecrets;

impl Secrets for NoSecrets {
    fn imap(&self, _key: &str, _account: &AccountConfig) -> Result<Option<Credential>, String> {
        Ok(None)
    }
}
