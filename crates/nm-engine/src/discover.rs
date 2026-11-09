//! Finding the servers of an address, and trying its SMTP servers, before
//! the account is created (`docs/accounts.md`, "Finding the servers"). The
//! decisions are in `providers.rs` (pure, tested without network); this
//! file does the network part: the MX lookup, and the SMTP connections.

use std::time::Duration;

use nm_api::{DiscoveryReport, ImapParams, ServerCandidate, SmtpAttempt, SmtpTestReport};
use nm_proto::imap::Timeouts;
use nm_proto::smtp;

use crate::providers::{self, Existing, Providers};
use crate::setup;

/// The report of `discover_account`: the database, and the MX of the
/// domain when the domain itself is not listed. `existing`: the accounts
/// the core has, for the key proposed.
pub async fn discover(email: &str, existing: &[Existing], given: Option<&ImapParams>) -> Result<DiscoveryReport, String> {
    let db = Providers::get()?;
    let (_, domain) = providers::split_address(email)?;
    let mx = if providers::needs_mx(db, email) {
        Some(nm_proto::dns::mx(&domain).await.map(|v| v.into_iter().map(|m| m.host).collect()).map_err(|e| e.to_string()))
    } else {
        None
    };
    providers::plan(db, email, existing, given, mx)
}

/// Time limits of an attempt: shorter than a sending's, since several
/// servers may be tried in a row while the user waits; a server that does
/// not answer in 8 seconds is not the one.
const TIMEOUTS: Timeouts = Timeouts { connect: Duration::from_secs(8), io: Duration::from_secs(15) };

/// Tries the candidates in order and stops at the first that works:
///
/// - on a server of the IMAP server's organization (`same_organization`),
///   the IMAP password is tried: `logged_in` when accepted;
/// - elsewhere the password is not sent (the sender would not use it
///   there either, `docs/sending.md`): `reachable` when the server answers
///   and offers a login; the account will need its SMTP password;
/// - a server that offers no login at all (`AUTH` absent) is a relay, not
///   the provider's submission server: `no_login`, and the next is tried.
///
/// Nothing is sent: the connection ends with `QUIT` after the login.
pub async fn test_smtp(email: &str, imap: &ImapParams, password: &str, candidates: &[ServerCandidate]) -> SmtpTestReport {
    let mut attempts = Vec::new();
    let mut chosen = None;
    for c in candidates {
        let (outcome, detail) = attempt(email, imap, password, &c.server).await;
        let works = outcome == "logged_in" || outcome == "reachable";
        attempts.push(SmtpAttempt { candidate: c.clone(), outcome: outcome.into(), detail });
        if works {
            chosen = Some(attempts.len() - 1);
            break;
        }
    }
    SmtpTestReport { attempts, chosen }
}

async fn attempt(email: &str, imap: &ImapParams, password: &str, server: &ImapParams) -> (&'static str, Option<String>) {
    let spec = match setup::spec_of(server) {
        Ok(s) => s,
        Err(e) => return ("failed", Some(e)),
    };
    let mut client = match smtp::Client::connect(&spec, TIMEOUTS).await {
        Ok(c) => c,
        Err(e) => return ("failed", Some(e.to_string())),
    };
    if client.extensions().auth.is_empty() {
        client.quit().await;
        return ("no_login", None);
    }
    if !providers::same_organization(&imap.host, &server.host) {
        client.quit().await;
        return ("reachable", None);
    }
    let user = setup::username_of(server, email);
    let r = client.login(&user, password).await;
    client.quit().await;
    match r {
        Ok(()) => ("logged_in", None),
        Err(e) => ("failed", Some(e.to_string())),
    }
}
