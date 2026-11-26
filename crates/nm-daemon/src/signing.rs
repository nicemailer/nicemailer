//! Who a client of the socket is, from its code signature
//! (`specifications/architecture.md`, section 9, "Vérification de la
//! signature"). The checks themselves are in `nm-signing`, shared with the
//! clients, which verify the core in turn; this module maps a verified
//! identifier to a [`Source`], whose rights `rights.rs` gives.
//!
//! **The identity of a client is its signature, never what it declares in
//! `hello`**: each of our binaries has its own identifier. Only a relaxed
//! build (debug, or the `dev-unverified` feature, both decided at compile
//! time) lets an unverified client in, and `hello` then declares its source.

use nm_core::Source;
use nm_signing::{AuditToken, Verdict};
use tokio::net::UnixStream;

/// What the verification of a connection found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Admission {
    /// The signature meets the requirement; the identifier says which of
    /// our binaries the client is, and that alone gives its rights.
    Verified(Source),
    /// Relaxed build, no acceptable signature (an unsigned development
    /// binary): `hello` declares the source.
    DevUnverified,
    /// Refused: the connection is closed, and the reason logged
    /// (`architecture.md`, section 9).
    Refused(String),
}

/// The source of a signed client, from its identifier (`architecture.md`,
/// section 9, "Droits par client").
pub(crate) fn source_of_identifier(identifier: &str) -> Option<Source> {
    Some(match identifier {
        nm_signing::APP_IDENTIFIER => Source::App,
        nm_signing::CLI_IDENTIFIER => Source::Cli,
        nm_signing::TUI_IDENTIFIER => Source::Tui,
        nm_signing::MCP_IDENTIFIER => Source::Mcp,
        nm_signing::WEB_IDENTIFIER => Source::Web,
        _ => return None,
    })
}

/// Verifies the client at the other end of `stream`. The audit token is
/// read at once (cheap); the signature check may read the client's binary
/// from disk and take a while, so it runs on tokio's blocking pool and
/// never holds up the loop that accepts the connections.
pub(crate) async fn admit(stream: &UnixStream) -> Admission {
    // Rust note: `AsRawFd` gives the file descriptor (a small integer) of
    // the socket, which the C call takes; the stream keeps owning it.
    use std::os::unix::io::AsRawFd;
    let token = match nm_signing::peer_audit_token(stream.as_raw_fd()) {
        Ok(t) => t,
        Err(e) => return refused_or_dev(format!("no audit token: {e}")),
    };
    // Rust note: `spawn_blocking` runs a synchronous function on a thread
    // pool made for blocking work; `move` hands it the token (a plain
    // array, copied).
    match tokio::task::spawn_blocking(move || verdict(&token)).await {
        Ok(a) => a,
        Err(e) => refused_or_dev(format!("the check failed: {e}")),
    }
}

/// The admission of the client of `token` (blocking).
fn verdict(token: &AuditToken) -> Admission {
    match nm_signing::verify(token, nm_signing::CLIENT_IDENTIFIERS) {
        Verdict::Verified(identifier) => match source_of_identifier(&identifier) {
            Some(source) => Admission::Verified(source),
            None => refused_or_dev(format!("unknown identifier {identifier}")),
        },
        Verdict::Unverified => Admission::DevUnverified,
        Verdict::Refused(why) => Admission::Refused(why),
    }
}

/// A failure that is fine in a relaxed build only.
fn refused_or_dev(why: String) -> Admission {
    if nm_signing::RELAXED { Admission::DevUnverified } else { Admission::Refused(why) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_identifier_of_a_signed_client_gives_its_source() {
        assert_eq!(source_of_identifier("eu.nicemailer.NiceMailer"), Some(Source::App));
        assert_eq!(source_of_identifier("eu.nicemailer.cli"), Some(Source::Cli));
        assert_eq!(source_of_identifier("eu.nicemailer.tui"), Some(Source::Tui));
        assert_eq!(source_of_identifier("eu.nicemailer.mcp"), Some(Source::Mcp));
        assert_eq!(source_of_identifier("eu.nicemailer.web"), Some(Source::Web));
        // The core is not a client.
        assert_eq!(source_of_identifier("eu.nicemailer.core"), None);
        assert_eq!(source_of_identifier("com.evil.trojan"), None);
        // Every identifier the core accepts maps to a source.
        for i in nm_signing::CLIENT_IDENTIFIERS {
            assert!(source_of_identifier(i).is_some(), "{i}");
        }
    }

    /// The test binary is not signed with one of our identifiers: a relaxed
    /// build admits it unverified, a strict one refuses it.
    #[tokio::test]
    async fn an_unsigned_client_is_refused_unless_relaxed() {
        let (a, _b) = UnixStream::pair().unwrap();
        let relaxed = match admit(&a).await {
            Admission::Verified(s) => panic!("the test binary passed as {s:?}"),
            Admission::DevUnverified => true,
            Admission::Refused(_) => false,
        };
        assert_eq!(relaxed, nm_signing::RELAXED);
    }
}
