//! Watching `config.toml` to apply it without a restart
//! (`configuration.md`, section 4).
//!
//! The directory of the file is watched, not the file itself: editors save
//! by writing a new file and renaming it over the old one, and a watch on
//! the old file would stop at the first save. The directory also sees the
//! file appear when it did not exist.
//!
//! A dotfiles repository often makes `~/.config/nicemailer/config.toml` a
//! symbolic link to a file of the repository. Editing that file changes
//! nothing in `~/.config/nicemailer`, so the directory of the link's target
//! is watched too.
//!
//! One save produces several file system events (write, rename, attribute
//! change). They are grouped: the callback runs once the directory has been
//! quiet for [`QUIET`]. Reading a half-written file is harmless anyway: it
//! is refused as invalid, and the next event reads it again, complete.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use notify::{RecursiveMode, Watcher as _};

/// How long the directory must stay quiet before the file is read.
pub const QUIET: Duration = Duration::from_millis(150);

/// A running watch. Dropping it stops the watch and its thread.
pub struct Watcher {
    _inner: notify::RecommendedWatcher,
}

/// Starts watching `path`; `on_change` runs on a thread of its own after
/// each group of changes. Creates the directory of the file if it is
/// missing, so that the file can be created later and still be seen (the
/// core never creates the file itself).
pub fn watch(path: &Path, on_change: impl Fn() + Send + 'static) -> Result<Watcher, String> {
    let mut dirs: BTreeSet<PathBuf> = BTreeSet::new();
    let mut names: BTreeSet<OsString> = BTreeSet::new();
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new(".")).to_path_buf();
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    dirs.insert(dir);
    if let Some(n) = path.file_name() {
        names.insert(n.to_os_string());
    }
    // Rust note: `if let … && let …` chains two pattern matches; the block
    // runs only if both succeed.
    if let Ok(target) = std::fs::canonicalize(path)
        && target != path
        && let (Some(d), Some(n)) = (target.parent(), target.file_name())
    {
        dirs.insert(d.to_path_buf());
        names.insert(n.to_os_string());
    }

    // The watcher calls its handler on a thread of `notify`; the handler
    // only forwards to a channel, and a thread of ours does the grouping.
    let (tx, rx) = mpsc::channel::<()>();
    let handler = move |res: notify::Result<notify::Event>| {
        let relevant = match res {
            // An event about the file, under any of its names.
            Ok(event) => event.paths.is_empty() || event.paths.iter().any(|p| p.file_name().is_some_and(|n| names.contains(n))),
            // An error of the watch (events lost by the system): read the
            // file again to be sure.
            Err(_) => true,
        };
        if relevant {
            let _ = tx.send(());
        }
    };
    let mut inner = notify::recommended_watcher(handler).map_err(|e| e.to_string())?;
    for d in &dirs {
        inner.watch(d, RecursiveMode::NonRecursive).map_err(|e| format!("cannot watch {}: {e}", d.display()))?;
    }

    std::thread::Builder::new()
        .name("nicemailer-config-watch".into())
        .spawn(move || {
            // `recv` fails once the watcher, which owns the sending side
            // through its handler, is dropped: the thread then ends.
            while rx.recv().is_ok() {
                // Wait until no event arrived for `QUIET`.
                loop {
                    match rx.recv_timeout(QUIET) {
                        Ok(()) => continue,
                        Err(mpsc::RecvTimeoutError::Timeout) => break,
                        Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    }
                }
                on_change();
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(Watcher { _inner: inner })
}
