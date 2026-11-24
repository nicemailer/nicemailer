//! Finding the servers of an address, and trying its SMTP servers, before
//! the account is created (`docs/accounts.md`, "Finding the servers"). The
//! decisions are in `providers.rs` (pure, tested without network); this
//! file does the network part: the MX lookup, and the SMTP connections.

use std::time::Duration;

use nm_api::{DiscoveryReport, ImapParams, ServerCandidate, SmtpAttempt, SmtpTestReport};
use nm_core::secret::SecretString;
use nm_proto::imap::Timeouts;
use nm_proto::smtp;

use crate::providers::{self, Existing, Providers};
use crate::setup;

/// The report of `discover_account`: the database, and the MX of the
/// domain when the domain itself is not listed. `existing`: the accounts
/// the core has, for the key proposed. `client_id_available`: whether an
/// OAuth2 application id is in effect, for the OAuth2-only servers
/// (`specifications/oauth2.md`, section 7).
pub async fn discover(email: &str, existing: &[Existing], given: Option<&ImapParams>, client_id_available: bool) -> Result<DiscoveryReport, String> {
    let db = Providers::get()?;
    let (_, domain) = providers::split_address(email)?;
    let mx = if providers::needs_mx(db, email) {
        Some(nm_proto::dns::mx(&domain).await.map(|v| v.into_iter().map(|m| m.host).collect()).map_err(|e| e.to_string()))
    } else {
        None
    };
    providers::plan(db, email, existing, given, mx, client_id_available)
}

/// Time limits of an attempt: shorter than a sending's, since several
/// servers may be tried in a row while the user waits; a server that does
/// not answer in 8 seconds is not the one.
const TIMEOUTS: Timeouts = Timeouts { connect: Duration::from_secs(8), io: Duration::from_secs(15) };

/// Tries the candidates in order and stops at the first that works:
///
/// - on a server of the IMAP server's organization (`same_organization`),
///   the IMAP password is tried: `logged_in` when accepted, `auth_refused`
///   when the server refuses it (an app password needed for SMTP, say);
/// - elsewhere the password is not sent (the sender would not use it
///   there either, `docs/sending.md`): `reachable` when the server answers
///   and offers a login; the account will need its SMTP password;
/// - with `smtp_password`, the password the user gave for the SMTP server
///   itself, that one is tried on every server, whatever its organization:
///   it is meant for SMTP, so sending it there is what the user asked;
/// - a server that offers no login at all (`AUTH` absent) is a relay, not
///   the provider's submission server: `no_login`, and the next is tried.
///
/// `auth_refused` does not stop the search (a later candidate may take the
/// password), but when nothing works, `refused` names the first server
/// that refused it: the right server, short of its own password. The app
/// then asks for that password (`docs/accounts.md`, "In the app").
///
/// Nothing is sent: the connection ends with `QUIT` after the login.
pub async fn test_smtp(
    email: &str,
    imap: &ImapParams,
    password: &str,
    smtp_password: Option<&str>,
    // An OAuth2 access token, from a finished flow: used instead of the
    // password, and sent only to a server of the provider's table
    // (`specifications/oauth2.md`, section 7).
    token: Option<&SecretString>,
    candidates: &[ServerCandidate],
) -> SmtpTestReport {
    let mut attempts = Vec::new();
    let mut chosen = None;
    for c in candidates {
        let (outcome, detail) = attempt(email, imap, password, smtp_password, token, &c.server).await;
        let works = outcome == "logged_in" || outcome == "reachable";
        attempts.push(SmtpAttempt { candidate: c.clone(), outcome: outcome.into(), detail });
        if works {
            chosen = Some(attempts.len() - 1);
            break;
        }
    }
    let refused = if chosen.is_none() { attempts.iter().position(|a| a.outcome == "auth_refused") } else { None };
    SmtpTestReport { attempts, chosen, refused }
}

async fn attempt(
    email: &str,
    imap: &ImapParams,
    password: &str,
    smtp_password: Option<&str>,
    token: Option<&SecretString>,
    server: &ImapParams,
) -> (&'static str, Option<String>) {
    let spec = match setup::spec_of(server) {
        Ok(s) => s,
        Err(e) => return ("failed", Some(e)),
    };
    let mut client = match smtp::Client::connect(&spec, TIMEOUTS).await {
        Ok(c) => c,
        Err(e) => return ("failed", Some(e.to_string())),
    };
    let auth = &client.extensions().auth;
    if auth.is_empty() {
        client.quit().await;
        return ("no_login", None);
    }
    let user = setup::username_of(server, email);
    // OAuth2: the token goes only to a server of the account's provider, in
    // the compiled table, that offers XOAUTH2 (`oauth2.md`, section 7).
    if let Some(t) = token {
        if !auth.iter().any(|m| m == "XOAUTH2") {
            let offered = auth.join(" ");
            client.quit().await;
            return ("failed", Some(format!("the server does not offer XOAUTH2 (it offers {offered})")));
        }
        if !crate::oauth2::smtp_host_allowed(&imap.host, &server.host) {
            client.quit().await;
            return ("failed", Some("the OAuth2 token is not sent to this server: it is not in the provider's table".into()));
        }
        let r = client.login_oauth2(&user, t).await;
        client.quit().await;
        return match r {
            Ok(()) => ("logged_in", None),
            Err(e @ smtp::SmtpError::Auth(_)) => ("auth_refused", Some(e.to_string())),
            Err(e) => ("failed", Some(e.to_string())),
        };
    }
    // A server that offers only methods NiceMailer does not speak (OAuth2
    // alone): no password would ever pass, so asking the user for one
    // would lead nowhere. Said as a failure, with the methods offered.
    if !auth.iter().any(|m| m == "PLAIN" || m == "LOGIN") {
        let offered = auth.join(" ");
        client.quit().await;
        return ("failed", Some(format!("no login method NiceMailer knows (the server offers {offered})")));
    }
    let password = match smtp_password {
        Some(p) => p,
        None if providers::same_organization(&imap.host, &server.host) => password,
        None => {
            client.quit().await;
            return ("reachable", None);
        }
    };
    let r = client.login(&user, password).await;
    client.quit().await;
    match r {
        Ok(()) => ("logged_in", None),
        // Rust note: `Err(e @ smtp::SmtpError::Auth(_))` matches the
        // variant and binds the whole error to `e` at once.
        Err(e @ smtp::SmtpError::Auth(_)) => ("auth_refused", Some(e.to_string())),
        Err(e) => ("failed", Some(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    //! `test_smtp` against a small scripted SMTP server on the loopback,
    //! without TLS (allowed there: `transport::is_loopback`), so that each
    //! outcome is checked without the test servers of `make imap-up`.

    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpListener;

    /// The `AUTH PLAIN` token of a password, as a client sends it.
    fn plain(user: &str, password: &str) -> String {
        // A tiny base64 encoder: the crate does not depend on one, and
        // the test must not reuse the code it checks.
        const ABC: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let bytes = format!("\0{user}\0{password}").into_bytes();
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
            let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
            for i in 0..4 {
                if i <= chunk.len() {
                    out.push(ABC[(n >> (18 - 6 * i) & 63) as usize] as char);
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    /// A server that offers `auth` (empty: no `AUTH` line) and accepts
    /// only `password`, for every connection; returns its port, and the
    /// passwords it was sent (`AUTH PLAIN` tokens), in order.
    async fn server(auth: &'static str, password: &'static str) -> (u16, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else { return };
                let seen = seen2.clone();
                tokio::spawn(async move {
                    let (r, mut w) = stream.into_split();
                    let mut lines = BufReader::new(r).lines();
                    w.write_all(b"220 test ESMTP\r\n").await.unwrap();
                    while let Ok(Some(line)) = lines.next_line().await {
                        let reply = if line.starts_with("EHLO") {
                            if auth.is_empty() { "250 test".to_string() } else { format!("250-test\r\n250 AUTH {auth}") }
                        } else if let Some(token) = line.strip_prefix("AUTH PLAIN ") {
                            seen.lock().unwrap().push(token.to_string());
                            if token == plain("me@example.com", password) { "235 ok".into() } else { "535 5.7.8 bad credentials".into() }
                        } else if line == "QUIT" {
                            let _ = w.write_all(b"221 bye\r\n").await;
                            return;
                        } else {
                            "500 what".into()
                        };
                        w.write_all(format!("{reply}\r\n").as_bytes()).await.unwrap();
                    }
                });
            }
        });
        (port, seen)
    }

    fn at(host: &str, port: u16) -> ImapParams {
        ImapParams { host: host.into(), port, security: "none".into(), username: None, ca_file: None }
    }

    fn candidate(port: u16) -> ServerCandidate {
        ServerCandidate { server: at("127.0.0.1", port), origin: "provider".into() }
    }

    fn outcomes(r: &SmtpTestReport) -> Vec<&str> {
        r.attempts.iter().map(|a| a.outcome.as_str()).collect()
    }

    #[tokio::test]
    async fn the_imap_password_is_tried_on_a_server_of_the_same_organization() {
        let (port, seen) = server("PLAIN LOGIN", "imap-pw").await;
        let r = test_smtp("me@example.com", &at("127.0.0.1", 993), "imap-pw", None, None, &[candidate(port)]).await;
        assert_eq!((outcomes(&r), r.chosen, r.refused), (vec!["logged_in"], Some(0), None));
        assert_eq!(seen.lock().unwrap().len(), 1);
    }

    /// The server that refuses the IMAP password is the right one, short
    /// of its own password: the others are still tried, and when none
    /// works, `refused` names it.
    #[tokio::test]
    async fn a_refused_password_is_reported_and_the_next_server_tried() {
        let (refusing, _) = server("PLAIN", "smtp-pw").await;
        let (relay, _) = server("", "").await;
        let r = test_smtp("me@example.com", &at("127.0.0.1", 993), "imap-pw", None, None, &[candidate(refusing), candidate(relay)]).await;
        assert_eq!((outcomes(&r), r.chosen, r.refused), (vec!["auth_refused", "no_login"], None, Some(0)));
        assert!(r.attempts[0].detail.as_deref().unwrap().contains("535"), "{r:?}");

        // A later server that takes the password wins: nothing refused.
        let (taking, _) = server("PLAIN", "imap-pw").await;
        let r = test_smtp("me@example.com", &at("127.0.0.1", 993), "imap-pw", None, None, &[candidate(refusing), candidate(taking)]).await;
        assert_eq!((outcomes(&r), r.chosen, r.refused), (vec!["auth_refused", "logged_in"], Some(1), None));
    }

    /// The SMTP password the user gave is the one tried, on any server.
    #[tokio::test]
    async fn the_smtp_password_given_is_tried_whatever_the_organization() {
        let (port, seen) = server("PLAIN", "smtp-pw").await;
        let other = at("imap.other.example", 993);
        // Without it, the IMAP password never leaves for another
        // organization.
        let r = test_smtp("me@example.com", &other, "imap-pw", None, None, &[candidate(port)]).await;
        assert_eq!((outcomes(&r), r.chosen), (vec!["reachable"], Some(0)));
        assert!(seen.lock().unwrap().is_empty());
        // With it: tried, accepted.
        let r = test_smtp("me@example.com", &other, "imap-pw", Some("smtp-pw"), None, &[candidate(port)]).await;
        assert_eq!((outcomes(&r), r.chosen), (vec!["logged_in"], Some(0)));
        assert_eq!(*seen.lock().unwrap(), [plain("me@example.com", "smtp-pw")]);
        // A wrong one: refused again.
        let r = test_smtp("me@example.com", &other, "imap-pw", Some("nope"), None, &[candidate(port)]).await;
        assert_eq!((outcomes(&r), r.chosen, r.refused), (vec!["auth_refused"], None, Some(0)));
    }

    /// A server offering only a method NiceMailer does not speak: no
    /// password would pass, so it is a failure, not a password to ask for.
    #[tokio::test]
    async fn a_server_without_a_known_login_method_fails() {
        let (port, seen) = server("XOAUTH2", "").await;
        let r = test_smtp("me@example.com", &at("127.0.0.1", 993), "imap-pw", None, None, &[candidate(port)]).await;
        assert_eq!((outcomes(&r), r.chosen, r.refused), (vec!["failed"], None, None));
        assert!(r.attempts[0].detail.as_deref().unwrap().contains("XOAUTH2"), "{r:?}");
        assert!(seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn nothing_listening_fails() {
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let r = test_smtp("me@example.com", &at("127.0.0.1", 993), "imap-pw", None, None, &[candidate(port)]).await;
        assert_eq!((outcomes(&r), r.chosen, r.refused), (vec!["failed"], None, None));
    }
}
