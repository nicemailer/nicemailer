//! `probe_account`: what an account's IMAP server offers, without changing
//! anything (`plan-imap.md`, phase 2, `nicemailer imap probe`).
//!
//! The probe logs in, lists the capabilities before and after login, turns
//! on CONDSTORE and QRESYNC as the synchronization will (ENABLE lasts for
//! the session only), reads the namespace, lists the folders with their
//! counters and the role NiceMailer would propose for each, and logs out.
//! It selects no mailbox: nothing can be marked read, nothing is written.

use std::time::Instant;

use nm_api::{ProbeFolder, ProbeNamespace, ProbeReport};
use nm_proto::imap::{ImapError, ListEntry, MailboxStatus, Security, ServerSpec, Session, Timeouts};

use crate::config::{self, AccountConfig};
use crate::folders::propose_roles;

/// Above this many folders, the counters of each are not asked one by one
/// (without LIST-STATUS, one STATUS per folder): the probe stays quick.
const MAX_STATUS: usize = 300;

/// The server of an account, for `nm-proto`.
pub fn imap_spec(account: &AccountConfig) -> ServerSpec {
    ServerSpec {
        host: account.imap.host.clone(),
        port: account.imap.port,
        security: match account.imap.security {
            config::Security::Tls => Security::Tls,
            config::Security::Starttls => Security::Starttls,
            config::Security::None => Security::None,
        },
        ca_file: account.imap.ca_file.clone(),
    }
}

/// Runs the probe. `password` was read from the Keychain by the caller,
/// after the binding to the server was checked (`secrets::imap_password`).
pub async fn probe(spec: &ServerSpec, username: &str, password: &str, timeouts: Timeouts) -> Result<ProbeReport, ImapError> {
    let started = Instant::now();
    let mut s = Session::connect(spec, timeouts).await?;
    let connect_ms = started.elapsed().as_millis() as u64;
    let before: Vec<String> = s.capabilities().names().map(str::to_string).collect();

    let started = Instant::now();
    s.login(username, password).await?;
    let login_ms = started.elapsed().as_millis() as u64;
    // Failures of the optional steps do not fail the probe.
    let _ = s.id().await;
    let enabled = s.enable_sync().await.unwrap_or_default();
    let namespace = match s.namespace().await {
        Ok(ns) => ns.personal.into_iter().next(),
        Err(e) if e.is_fatal() => return Err(e),
        Err(_) => None,
    };

    let started = Instant::now();
    let listed: Vec<(ListEntry, Option<MailboxStatus>)> = if s.capabilities().has("LIST-STATUS") {
        s.list_with_status().await?
    } else {
        let entries = s.list().await?;
        let mut out = Vec::with_capacity(entries.len());
        let ask = entries.len() <= MAX_STATUS;
        for e in entries {
            let status = if ask && e.selectable() {
                match s.status(&e.name).await {
                    Ok(st) => Some(st),
                    Err(err) if err.is_fatal() => return Err(err),
                    Err(_) => None,
                }
            } else {
                None
            };
            out.push((e, status));
        }
        out
    };
    let list_ms = started.elapsed().as_millis() as u64;
    let capabilities: Vec<String> = s.capabilities().names().map(str::to_string).collect();
    s.logout().await;

    let entries: Vec<ListEntry> = listed.iter().map(|(e, _)| e.clone()).collect();
    let roles = propose_roles(&entries, namespace.as_ref());
    let mut folders: Vec<ProbeFolder> = listed
        .into_iter()
        .zip(roles)
        .map(|((e, st), role)| ProbeFolder {
            selectable: e.selectable(),
            name: e.name.name,
            delimiter: e.delimiter.map(String::from),
            attributes: e.attributes,
            proposed_role: role.map(|(r, _)| r.to_string()),
            role_from: role.map(|(_, from)| from.as_str().to_string()),
            messages: st.as_ref().and_then(|s| s.messages),
            unseen: st.as_ref().and_then(|s| s.unseen),
        })
        .collect();
    // INBOX first, then by name: the order servers list in varies.
    folders.sort_by(|a, b| (a.proposed_role.as_deref() != Some("inbox"), &a.name).cmp(&(b.proposed_role.as_deref() != Some("inbox"), &b.name)));

    Ok(ProbeReport {
        server: format!("{}:{}", spec.host, spec.port),
        security: match spec.security {
            Security::Tls => "tls",
            Security::Starttls => "starttls",
            Security::None => "none",
        }
        .into(),
        capabilities_before_login: before,
        capabilities,
        enabled,
        namespace: namespace.map(|n| ProbeNamespace { prefix: n.prefix, delimiter: n.delimiter.map(String::from) }),
        folders,
        connect_ms,
        login_ms,
        list_ms,
    })
}
