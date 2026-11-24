//! What the server is asked when an account is created
//! (`import-initial.md`, sections 2 to 4; `plan-imap.md`, phase 7): the
//! test connection with the counts of the import screen, the exact count of
//! the archiving, the recipients of Sent, the folders to create.
//!
//! Nothing here writes the database (the daemon does it, with `nm-store`,
//! `setup.rs`). On the server, only [`create_folders`] writes; everything
//! else opens folders read-only (`EXAMINE`).

use std::collections::{BTreeMap, BTreeSet};

use nm_api::{ArchiveCount, ImapParams, RolePlan, TestAccountReport};
use nm_core::FolderRole;
use nm_proto::imap::{FetchItems, ImapError, MailboxName, SearchKey, Security, ServerSpec, Session, Timeouts, UidSet, Uids};
use nm_store::ingest::parse_headers;

use crate::config::ImportSettings;
use crate::probe::{self, probe};

/// Headers of Sent fetched per command.
const SENT_BATCH: usize = 500;

/// The server of `ImapParams`, for `nm-proto`.
pub fn spec_of(p: &ImapParams) -> Result<ServerSpec, String> {
    let security = match p.security.as_str() {
        "tls" => Security::Tls,
        "starttls" => Security::Starttls,
        "none" => Security::None,
        other => return Err(format!("security must be tls, starttls or none, not {other:?}")),
    };
    Ok(ServerSpec { host: p.host.clone(), port: p.port, security, ca_file: p.ca_file.as_ref().map(Into::into) })
}

/// The user name: the one given, else the address.
pub fn username_of(p: &ImapParams, email: &str) -> String {
    p.username.clone().filter(|u| !u.is_empty()).unwrap_or_else(|| email.to_string())
}

/// A logged-in session.
pub async fn open(spec: &ServerSpec, username: &str, cred: probe::Cred<'_>) -> Result<Session, ImapError> {
    let mut s = Session::connect(spec, Timeouts::default()).await?;
    cred.login(&mut s, username).await?;
    Ok(s)
}

/// The test connection of the creation screen: the probe (capabilities,
/// folders, proposed roles), then the counts of the import screen.
pub async fn test_account(
    spec: &ServerSpec,
    username: &str,
    cred: probe::Cred<'_>,
    own: &[String],
    import: &ImportSettings,
    today: i64,
) -> Result<TestAccountReport, ImapError> {
    let report = probe(spec, username, cred, Timeouts::default()).await?;
    let mut s = open(spec, username, cred).await?;
    // Only reading here: a test of a production mailbox writes nothing.
    s.forbid_writes();
    let st = s.status(&MailboxName::inbox()).await?;
    let uidnext = st.uid_next.unwrap_or(1);
    let sent = report.folders.iter().find(|f| f.proposed_role.as_deref() == Some("sent")).map(|f| f.name.clone());
    let correspondents = match sent {
        Some(path) => sent_recipients(&mut s, &path, own).await?.len() as u64,
        None => 0,
    };
    let messages = st.messages.unwrap_or(0) as u64;
    let unseen = st.unseen.unwrap_or(0) as u64;
    let read_before = day(today + 86_400);
    let unread_before = day(today - import.unseen_default_age_days as i64 * 86_400);
    let offer = messages > import.threshold_total || unseen > import.threshold_unseen;
    let count = if offer { count_archive(&mut s, uidnext, Some(&read_before), Some(&unread_before)).await? } else { ArchiveCount { read: 0, unread: 0 } };
    s.logout().await;
    let roles = role_plan(&report);
    Ok(TestAccountReport {
        roles,
        probe: report,
        inbox_messages: messages,
        inbox_unseen: unseen,
        inbox_uidnext: uidnext,
        correspondents,
        offer_archive: offer,
        read_before,
        unread_before,
        archive_read: count.read,
        archive_unread: count.unread,
    })
}

/// The folder of each workflow role (`dossiers.md`, "A la création du
/// compte IMAP dans niceMailer"): the user's choice, else the folder
/// proposed for it (special-use, then names), else a new folder under the
/// role's English name, in the personal namespace (`prefix`, `INBOX.` on
/// some servers). The Inbox is always `INBOX`.
///
/// `proposed`: each folder of the server, with the role proposed for it
/// (`folders::propose_roles`). The same rule for `test_account`, which
/// shows the plan, and `add_account`, which applies it: what the screen
/// says is what is done.
pub fn plan_roles(chosen: &BTreeMap<FolderRole, String>, proposed: &[(&str, Option<&str>)], prefix: &str) -> BTreeMap<FolderRole, String> {
    let mut out = BTreeMap::new();
    for role in FolderRole::ALL {
        let path = if role == FolderRole::Inbox {
            "INBOX".to_string()
        } else if let Some(p) = chosen.get(&role) {
            p.clone()
        } else if let Some((name, _)) = proposed.iter().find(|(_, r)| *r == Some(role.as_str())) {
            name.to_string()
        } else {
            format!("{prefix}{}", role.display_name())
        };
        out.insert(role, path);
    }
    out
}

/// The plan of `test_account`: [`plan_roles`] without choice, each folder
/// marked with whether the server has it.
fn role_plan(report: &nm_api::ProbeReport) -> Vec<RolePlan> {
    let proposed: Vec<(&str, Option<&str>)> = report.folders.iter().map(|f| (f.name.as_str(), f.proposed_role.as_deref())).collect();
    let prefix = report.namespace.as_ref().map(|n| n.prefix.as_str()).unwrap_or("");
    // Listed in the order of `FolderRole::ALL` (the sidebar's), each role
    // read back from the map by `plan[&role]`.
    let plan = plan_roles(&BTreeMap::new(), &proposed, prefix);
    FolderRole::ALL
        .into_iter()
        .map(|role| {
            let path = plan[&role].clone();
            let exists = report.folders.iter().any(|f| f.name == path);
            RolePlan { role, path, exists }
        })
        .collect()
}

/// The recipients (To, Cc, Bcc) of every mail of Sent, lowercase, without
/// the account's own addresses: the people the user wrote to
/// (`import-initial.md`, section 4).
pub async fn sent_recipients(s: &mut Session, sent: &str, own: &[String]) -> Result<Vec<String>, ImapError> {
    let sel = s.examine(&MailboxName::new(sent), None).await?;
    if sel.exists == 0 {
        return Ok(Vec::new());
    }
    let uids = s.uid_search(&SearchKey::All).await?;
    let mut out: BTreeSet<String> = BTreeSet::new();
    let items = FetchItems { header: true, ..FetchItems::default() };
    for chunk in uids.chunks(SENT_BATCH) {
        for m in s.uid_fetch(&Uids::Set(UidSet::from_uids(chunk.iter().copied())), &items).await? {
            let h = parse_headers(m.header.as_deref().unwrap_or(b""));
            for a in h.to.iter().chain(&h.cc).chain(&h.bcc) {
                if !own.contains(&a.addr) {
                    out.insert(a.addr.clone());
                }
            }
        }
    }
    Ok(out.into_iter().collect())
}

/// The search of the archiving: the Inbox below `boundary`, not flagged,
/// read (or unread) before `before` (`YYYY-MM-DD`, internal date).
pub fn archive_key(boundary: u32, seen: bool, before: &str) -> Option<SearchKey> {
    let (year, month, day) = parse_day(before)?;
    let below = SearchKey::Uid(if boundary > 1 { UidSet::from_ranges([(1, boundary - 1)]) } else { UidSet::default() });
    let state = if seen { SearchKey::Seen } else { SearchKey::Unseen };
    Some(SearchKey::And(
        Box::new(below),
        Box::new(SearchKey::And(
            Box::new(SearchKey::Not(Box::new(SearchKey::Flagged))),
            Box::new(SearchKey::And(Box::new(state), Box::new(SearchKey::Before { year, month, day }))),
        )),
    ))
}

/// The UIDs the archiving would move, read and unread, the Inbox opened
/// read-only first.
pub async fn archive_uids(s: &mut Session, boundary: u32, read_before: Option<&str>, unread_before: Option<&str>) -> Result<(Vec<u32>, Vec<u32>), ImapError> {
    s.examine(&MailboxName::inbox(), None).await?;
    let mut read = Vec::new();
    let mut unread = Vec::new();
    if let Some(key) = read_before.and_then(|d| archive_key(boundary, true, d)) {
        read = s.uid_search(&key).await?;
    }
    if let Some(key) = unread_before.and_then(|d| archive_key(boundary, false, d)) {
        unread = s.uid_search(&key).await?;
    }
    Ok((read, unread))
}

/// The exact count of the import screen (`import-initial.md`, section 3).
pub async fn count_archive(s: &mut Session, boundary: u32, read_before: Option<&str>, unread_before: Option<&str>) -> Result<ArchiveCount, ImapError> {
    let (read, unread) = archive_uids(s, boundary, read_before, unread_before).await?;
    Ok(ArchiveCount { read: read.len() as u64, unread: unread.len() as u64 })
}

/// Creates the folders of `paths` the server does not have yet; returns
/// those created.
pub async fn create_folders(s: &mut Session, paths: &[String]) -> Result<Vec<String>, ImapError> {
    let existing: BTreeSet<String> = s.list().await?.into_iter().map(|e| e.name.name).collect();
    let mut created = Vec::new();
    for p in paths {
        if !existing.contains(p) && !p.eq_ignore_ascii_case("INBOX") {
            s.create(&MailboxName::new(p)).await?;
            created.push(p.clone());
        }
    }
    Ok(created)
}

/// `YYYY-MM-DD` of a Unix time, in UTC.
pub fn day(unix: i64) -> String {
    let (y, m, d) = civil(unix.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

/// `YYYY-MM-DD` read, checked.
pub fn parse_day(s: &str) -> Option<(i32, u32, u32)> {
    let mut it = s.trim().splitn(3, '-');
    let y: i32 = it.next()?.parse().ok()?;
    let m: u32 = it.next()?.parse().ok()?;
    let d: u32 = it.next()?.parse().ok()?;
    ((1970..=9999).contains(&y) && (1..=12).contains(&m) && (1..=31).contains(&d)).then_some((y, m, d))
}

/// The civil date of a day number since 1970-01-01 (Howard Hinnant's
/// algorithm).
fn civil(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_and_their_search() {
        assert_eq!(day(0), "1970-01-01");
        assert_eq!(day(1_793_952_000), "2026-11-06");
        assert_eq!(day(951_782_400), "2000-02-29");
        assert_eq!(parse_day("2026-10-07"), Some((2026, 10, 7)));
        assert_eq!(parse_day("07/10/2026"), None);
        assert_eq!(parse_day("2026-13-01"), None);
        assert!(archive_key(100, true, "2026-10-07").is_some());
        assert!(archive_key(100, true, "yesterday").is_none());
    }
}
