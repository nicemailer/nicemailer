//! `nicemailer sync verify <account>`: the criterion of "perfect
//! retrieval" (`plan-imap.md`, section 4). Folder by folder, the server and
//! the database must have the same UIDs and the same flags, and every
//! complete message a raw file whose hash is checked.
//!
//! The verification opens a connection of its own, read-only like the
//! synchronization (`EXAMINE`), and reads `UID FETCH 1:* (FLAGS)` of each
//! folder: about 40 bytes a message. It changes nothing. Run while the
//! synchronization works, it may see a mail the server has and the
//! database not yet: the tests run it once the synchronization is idle.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use nm_api::{FolderVerify, SyncVerifyReport};
use nm_proto::imap::{FetchItems, MailboxName, Session, Timeouts, Uids};
use nm_store::sync::ServerFlags;

use super::{AccountSpec, SyncHost, read};
use crate::probe::imap_spec;
use crate::secrets::{self, Secrets};

/// Differences listed per kind, at most.
const LIST_MAX: usize = 20;

/// Runs the verification on a thread of the caller (blocking): it builds
/// its own runtime, like the workers.
pub fn verify_blocking(
    spec: &AccountSpec,
    secrets: Arc<dyn Secrets>,
    host: Arc<dyn SyncHost>,
    skip_hash: bool,
) -> Result<SyncVerifyReport, String> {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    rt.block_on(verify(spec, secrets.as_ref(), host.as_ref(), skip_hash))
}

async fn verify(spec: &AccountSpec, secrets: &dyn Secrets, host: &dyn SyncHost, skip_hash: bool) -> Result<SyncVerifyReport, String> {
    let started = Instant::now();
    let cfg = &spec.config;
    let password = secrets::imap_password(secrets, &spec.key, cfg).map_err(|r| r.message(&spec.key, cfg))?;
    let mut s = Session::connect(&imap_spec(cfg), Timeouts::default()).await.map_err(|e| e.to_string())?;
    s.login(&cfg.imap.username, password.expose()).await.map_err(|e| e.to_string())?;
    let account_id = spec.account_id;
    let folders = read(host, |st| st.account_sync_folders(account_id)).map_err(|e| e.to_string())?;
    let listed: Vec<String> = s.list().await.map_err(|e| e.to_string())?.into_iter().map(|e| e.name.name).collect();

    let mut report = SyncVerifyReport { account: spec.key.clone(), ok: true, ..Default::default() };
    for (f, _, _) in folders {
        if !listed.contains(&f.path) {
            // Not a difference of the synchronization: reported by
            // `sync status` (`missing_on_server`).
            continue;
        }
        let mut v = FolderVerify { path: f.path.clone(), role: f.role.clone(), ..Default::default() };
        let sel = s.examine(&MailboxName::new(&f.path), None).await.map_err(|e| format!("{}: {e}", f.path))?;
        if sel.uid_validity != f.uidvalidity {
            v.uidvalidity_differs = sel.uid_validity;
        }
        let server: BTreeMap<u32, ServerFlags> = if sel.exists == 0 {
            BTreeMap::new()
        } else {
            let items = FetchItems { flags: true, ..FetchItems::default() };
            s.uid_fetch(&Uids::From(1), &items)
                .await
                .map_err(|e| format!("{}: {e}", f.path))?
                .into_iter()
                .filter_map(|m| Some((m.uid?, ServerFlags::parse(m.flags.as_deref().unwrap_or(&[])))))
                .collect()
        };
        let folder_id = f.id;
        let local: BTreeMap<u32, ServerFlags> =
            read(host, |st| st.known_messages(folder_id)).map_err(|e| e.to_string())?.into_iter().map(|k| (k.uid, k.flags)).collect();
        v.server_messages = server.len() as u64;
        v.local_messages = local.len() as u64;
        for (uid, flags) in &server {
            match local.get(uid) {
                None => push(&mut v.missing_locally, &mut v.missing_locally_count, *uid),
                Some(l) if l != flags => push(&mut v.flags_differ, &mut v.flags_differ_count, *uid),
                Some(_) => {}
            }
        }
        for uid in local.keys() {
            if !server.contains_key(uid) {
                push(&mut v.extra_locally, &mut v.extra_locally_count, *uid);
            }
        }
        let (bodies, missing) = read(host, |st| st.folder_bodies(folder_id)).map_err(|e| e.to_string())?;
        v.bodies_missing = missing;
        for (uid, hash) in bodies {
            let path = host.raw().path(&hash);
            let problem = if skip_hash {
                (!path.exists()).then(|| "raw file missing".to_string())
            } else {
                match host.raw().verify(&hash) {
                    Ok(true) => None,
                    Ok(false) => Some("raw file missing".to_string()),
                    Err(e) => Some(e.to_string()),
                }
            };
            v.bodies_checked += 1;
            if let Some(p) = problem
                && v.raw_problems.len() < LIST_MAX
            {
                v.raw_problems.push(format!("{uid}: {p}"));
            }
        }
        let clean = v.missing_locally_count + v.extra_locally_count + v.flags_differ_count == 0
            && v.uidvalidity_differs.is_none()
            && v.raw_problems.is_empty();
        report.ok &= clean;
        report.folders.push(v);
    }
    s.logout().await;
    report.seconds = started.elapsed().as_secs_f64();
    Ok(report)
}

fn push(list: &mut Vec<u32>, count: &mut u64, uid: u32) {
    *count += 1;
    if list.len() < LIST_MAX {
        list.push(uid);
    }
}
