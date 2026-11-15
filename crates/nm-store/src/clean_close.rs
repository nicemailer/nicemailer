//! The mark of a clean close: what lets the opening skip the reconciliation
//! of the two files (`reconcile_bodies`), its longest step on a real
//! mailbox (0.9 s with the body file cold, most of the core's start).
//!
//! The main file and the body file are committed apart (`schema_body.sql`):
//! only a stop in the middle of a write leaves them out of step, and the
//! reconciliation at the opening repairs that. After a clean close, there
//! is nothing to repair. So the core, as it stops, empties both write-ahead
//! logs into their files (a checkpoint) and writes `clean-close.json` next
//! to the database: for each file, its inode, size and time of last change.
//! At the next opening, if both files still match the mark exactly, and
//! neither log holds anything, nothing touched them since: the
//! reconciliation is skipped.
//!
//! Anything else reconciles, as before: no mark (a crash, a kill, a first
//! start, the CLI), a file changed or replaced (a backup restored, the body
//! file deleted), a log not empty. The mark is removed at every opening,
//! before anything is written, so a crash later never leaves a mark that
//! still matches.

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The name of the mark, next to the database.
pub const FILE: &str = "clean-close.json";

/// What identifies a file as it was: replaced, it gets another inode;
/// written, another size or time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileState {
    pub inode: u64,
    pub size: u64,
    /// Time of the last change, in nanoseconds since 1970.
    pub modified_ns: i128,
}

/// The mark: both files as the clean close left them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mark {
    pub main: FileState,
    pub body: FileState,
}

fn mark_path(db: &Path) -> PathBuf {
    db.with_file_name(FILE)
}

/// The state of `path`, or None when it is missing or unreadable.
pub fn file_state(path: &Path) -> Option<FileState> {
    let m = std::fs::metadata(path).ok()?;
    let modified_ns = i128::from(m.mtime()) * 1_000_000_000 + i128::from(m.mtime_nsec());
    Some(FileState { inode: m.ino(), size: m.size(), modified_ns })
}

/// Whether the write-ahead log of `path` holds anything: a log that is
/// missing or empty (truncated by the checkpoint of the close) does not.
fn log_has_content(path: &Path) -> bool {
    let mut wal = path.as_os_str().to_owned();
    wal.push("-wal");
    std::fs::metadata(PathBuf::from(wal)).map(|m| m.len() > 0).unwrap_or(false)
}

/// The state of both files, if both exist and neither log holds anything.
pub fn current(db: &Path, body: &Path) -> Option<Mark> {
    if log_has_content(db) || log_has_content(body) {
        return None;
    }
    Some(Mark { main: file_state(db)?, body: file_state(body)? })
}

/// At the opening: whether the files are as the last clean close left them.
/// The mark is removed in any case, before the caller writes anything.
/// Rust note: `.ok()` turns a `Result` into an `Option`, dropping the
/// error; `?` on an `Option` returns `None` early.
pub fn take(db: &Path, body: &Path) -> bool {
    let path = mark_path(db);
    let saved: Option<Mark> = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok());
    let _ = std::fs::remove_file(&path);
    match saved {
        Some(saved) => current(db, body).is_some_and(|now| now == saved),
        None => false,
    }
}

/// At a clean close, once the logs are emptied: writes the mark. Not
/// fatal when it cannot: the next opening reconciles.
pub fn write(db: &Path, body: &Path) -> bool {
    let Some(mark) = current(db, body) else { return false };
    let Ok(json) = serde_json::to_vec(&mark) else { return false };
    std::fs::write(mark_path(db), json).is_ok()
}
