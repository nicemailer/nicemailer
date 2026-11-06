//! Housekeeping of the raw store (`stockage-brut.md`, sections 6 and 7;
//! `plan-imap.md`, phase 8): the garbage collector, the sweep of orphans,
//! and what is done with a raw file found corrupt or missing.
//!
//! # Garbage collector
//!
//! A raw file is immutable, not eternal: once no message references it
//! (the server deleted the mail, an account was removed), it goes, with
//! the attachments extracted from it.
//!
//! 1. With the store's lock and the pins' lock held: the `raw_blob` rows no
//!    message references, and whose hash no ingestion has pinned, are
//!    deleted, and committed;
//! 2. still under the pins' lock: their files, and their extracted
//!    attachments.
//!
//! The pins (`raw.rs`, `Pin`) close the race of the specification: an
//! ingestion that found the file "already there" pins it until its row is
//! committed, so the collector never deletes a file a row is about to
//! reference. The other order is harmless: deleted first, the file is
//! written again by the ingestion, which then finds it missing.
//!
//! # Sweep of orphans
//!
//! What a crash may leave: temporary files (`.tmp/`) older than a day, and
//! final files without a `raw_blob` row older than `raw.gc_grace_secs`
//! (a day), not pinned.
//!
//! # A raw file corrupt or missing
//!
//! Found when it is read (an attachment, an export, `sync verify
//! --repair`): the file is set aside in `.corrupt/`, and every message of a
//! synchronized folder that had it goes back to "headers only"
//! (`body_state = 0`, its body and attachments rows deleted): the body
//! connection downloads it again from the server. A message without server
//! (archive account) cannot be repaired: it keeps its row, and the error.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use rusqlite::params;

use crate::raw::{Hash, RawStore, hex};
use crate::{Result, Store};

/// What a collection did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GcReport {
    /// Raw files deleted with their row: no message referenced them.
    pub blobs: u64,
    /// Temporary files older than a day.
    pub temporary: u64,
    /// Final files without row, older than the grace delay.
    pub orphans: u64,
    /// Directories of extracted attachments removed.
    pub attachment_dirs: u64,
}

/// Temporary files are kept this long: a write in progress is never this
/// old.
const TMP_GRACE: Duration = Duration::from_secs(24 * 3600);

/// Rows checked per query during the sweep.
const SWEEP_CHUNK: usize = 500;

/// Collects the raw files nothing references, then sweeps the orphans.
/// `attachments`: the root of the extracted attachments
/// (`attachments.noindex`), whose `<hash>/` directories go with their raw
/// file.
pub fn collect_garbage(store: &Mutex<Store>, raw: &RawStore, attachments: Option<&Path>, grace: Duration) -> Result<GcReport> {
    let mut report = GcReport::default();
    // 1 and 2.
    {
        let mut st = store.lock().unwrap();
        let deleted: Result<Vec<Hash>> = raw.with_pins_locked(|pinned| {
            let tx = st.transaction()?;
            let candidates: Vec<Vec<u8>> = tx
                .prepare("SELECT hash FROM raw_blob b WHERE NOT EXISTS (SELECT 1 FROM message m WHERE m.raw_hash = b.hash)")?
                .query_map([], |r| r.get(0))?
                .collect::<std::result::Result<_, _>>()?;
            let mut gone = Vec::new();
            for h in candidates {
                let Ok(hash) = <Hash>::try_from(h.as_slice()) else { continue };
                if pinned(&hash) {
                    continue;
                }
                tx.execute("DELETE FROM raw_blob WHERE hash = ?1", [&h])?;
                gone.push(hash);
            }
            tx.commit()?;
            for hash in &gone {
                let _ = std::fs::remove_file(raw.path(hash));
            }
            Ok(gone)
        });
        let deleted = deleted?;
        report.blobs = deleted.len() as u64;
        if let Some(root) = attachments {
            for hash in &deleted {
                let dir = root.join(hex(hash));
                if dir.exists() && std::fs::remove_dir_all(&dir).is_ok() {
                    report.attachment_dirs += 1;
                }
            }
        }
    }
    // The sweep: the directory walked without any lock, each candidate
    // checked under the locks.
    let now = SystemTime::now();
    let old = |p: &Path, age: Duration| {
        std::fs::metadata(p).and_then(|m| m.modified()).is_ok_and(|t| now.duration_since(t).unwrap_or_default() >= age)
    };
    if let Ok(entries) = std::fs::read_dir(raw.root().join(".tmp")) {
        for e in entries.flatten() {
            if old(&e.path(), TMP_GRACE) && std::fs::remove_file(e.path()).is_ok() {
                report.temporary += 1;
            }
        }
    }
    let mut files: Vec<(Hash, std::path::PathBuf)> = Vec::new();
    for dir in std::fs::read_dir(raw.root()).into_iter().flatten().flatten() {
        let name = dir.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || !dir.path().is_dir() {
            continue;
        }
        for f in std::fs::read_dir(dir.path()).into_iter().flatten().flatten() {
            let fname = f.file_name();
            let Some(h) = fname.to_string_lossy().strip_suffix(".eml.zst").and_then(parse_hex) else { continue };
            if old(&f.path(), grace) {
                files.push((h, f.path()));
            }
        }
    }
    for chunk in files.chunks(SWEEP_CHUNK) {
        let st = store.lock().unwrap();
        raw.with_pins_locked(|pinned| -> Result<()> {
            for (hash, path) in chunk {
                if pinned(hash) {
                    continue;
                }
                let known: bool = st
                    .conn()
                    .prepare_cached("SELECT EXISTS (SELECT 1 FROM raw_blob WHERE hash = ?1)")?
                    .query_row([&hash[..]], |r| r.get(0))?;
                if !known && std::fs::remove_file(path).is_ok() {
                    report.orphans += 1;
                }
            }
            Ok(())
        })?;
    }
    Ok(report)
}

fn parse_hex(s: &str) -> Option<Hash> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

/// What [`Store::raw_damaged`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Repair {
    /// Messages back to "headers only", their body to download again.
    pub refetch: u64,
    /// Messages without server (archive account): nothing to download from.
    pub lost: u64,
    /// The file was set aside in `.corrupt/` (it existed).
    pub set_aside: bool,
    /// The accounts whose body connection must look again.
    pub accounts: BTreeSet<i64>,
}

impl Store {
    /// A raw file was found corrupt or missing: set aside, and its messages
    /// of synchronized folders downloaded again (module documentation).
    pub fn raw_damaged(&mut self, raw: &RawStore, hash: &Hash) -> Result<Repair> {
        let mut out = Repair { set_aside: raw.set_aside(hash).map_err(|e| crate::StoreError::Io(e.to_string()))?, ..Repair::default() };
        let tx = self.transaction()?;
        let rows: Vec<(i64, i64, Option<u32>)> = tx
            .prepare("SELECT id, account_id, uid FROM message WHERE raw_hash = ?1")?
            .query_map([&hash[..]], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<std::result::Result<_, _>>()?;
        for (id, account, uid) in rows {
            if uid.is_none() {
                out.lost += 1;
                continue;
            }
            tx.execute("DELETE FROM message_body WHERE message_id = ?1", [id])?;
            tx.execute("DELETE FROM attachment WHERE message_id = ?1", [id])?;
            tx.execute("DELETE FROM message_unsubscribe WHERE message_id = ?1", [id])?;
            tx.execute("UPDATE message SET raw_hash = NULL, body_state = 0 WHERE id = ?1", [id])?;
            out.refetch += 1;
            out.accounts.insert(account);
        }
        tx.execute(
            "DELETE FROM raw_blob WHERE hash = ?1 AND NOT EXISTS (SELECT 1 FROM message WHERE raw_hash = ?1)",
            params![&hash[..]],
        )?;
        tx.commit()?;
        Ok(out)
    }
}
