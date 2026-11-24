//! A secret (a password, an OAuth2 token) that never shows in logs or debug
//! output. It lives here, in `nm-core`, so that every crate that handles a
//! secret on the wire (`nm-proto`'s SASL modules, `nm-engine`'s Keychain)
//! takes it as a [`SecretString`] (`specifications/oauth2.md`, section 7).

use std::fmt;

use serde::Serialize;
use sha2::{Digest, Sha256};

/// A secret that never shows in logs or debug output.
///
/// Rust note: this is a *newtype*, a struct with a single field. It costs
/// nothing at run time, but it is a different type from `String`, so the
/// compiler refuses to print it by mistake: its `Debug` implementation
/// below prints a placeholder, and it has no `Display` at all. Reading the
/// secret takes an explicit call to [`SecretString::expose`].
#[derive(Clone, PartialEq, Eq)]
pub struct SecretString(String);

impl SecretString {
    pub fn new(s: impl Into<String>) -> SecretString {
        SecretString(s.into())
    }

    /// The secret itself, for the authentication only.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretString(***)")
    }
}

/// Serialized as a short fingerprint, never as the secret: enough to see
/// that the secret changed, useless to an attacker (a SHA-256 truncated
/// to 64 bits of a secret still has to be guessed).
impl Serialize for SecretString {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let digest = Sha256::digest(self.0.as_bytes());
        let hex: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
        s.serialize_str(&format!("sha256:{hex}"))
    }
}
