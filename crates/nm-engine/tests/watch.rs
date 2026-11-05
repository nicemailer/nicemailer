//! Watching `config.toml` (FSEvents): an edit, a file created later, the
//! target of a symbolic link.

use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use nm_engine::watch::{Watcher, watch};

/// Starts a watch on `path` that reports each call on a channel.
fn watched(path: &Path) -> (Watcher, mpsc::Receiver<()>) {
    let (tx, rx) = mpsc::channel();
    let w = watch(path, move || {
        let _ = tx.send(());
    })
    .unwrap();
    // FSEvents starts delivering a short while after the watch is set up;
    // an event earlier than that may be lost.
    std::thread::sleep(Duration::from_millis(300));
    (w, rx)
}

/// FSEvents delivers with a latency of its own; 10 s is far above it.
fn changed(rx: &mpsc::Receiver<()>) -> bool {
    rx.recv_timeout(Duration::from_secs(10)).is_ok()
}

#[test]
fn an_edit_is_seen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "version = 1\n").unwrap();
    let (_w, rx) = watched(&path);
    std::fs::write(&path, "version = 1\n[sync]\npoll_interval_secs = 60\n").unwrap();
    assert!(changed(&rx));
}

#[test]
fn a_save_by_rename_is_seen_and_the_watch_goes_on() {
    // How editors save: write a temporary file, rename it over the old one.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "version = 1\n").unwrap();
    let (_w, rx) = watched(&path);
    for i in 0..2 {
        let tmp = dir.path().join(".config.toml.swp");
        std::fs::write(&tmp, format!("version = 1\n# save {i}\n")).unwrap();
        std::fs::rename(&tmp, &path).unwrap();
        assert!(changed(&rx), "save {i}");
        // Let the group of events of this save end.
        std::thread::sleep(Duration::from_millis(400));
        while rx.try_recv().is_ok() {}
    }
}

#[test]
fn a_file_created_later_in_a_missing_directory_is_seen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nicemailer/config.toml");
    let (_w, rx) = watched(&path);
    assert!(path.parent().unwrap().is_dir(), "the directory is created");
    std::fs::write(&path, "version = 1\n").unwrap();
    assert!(changed(&rx));
}

#[test]
fn an_edit_of_the_target_of_a_link_is_seen() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("dotfiles");
    std::fs::create_dir(&repo).unwrap();
    let target = repo.join("nicemailer.toml");
    std::fs::write(&target, "version = 1\n").unwrap();
    let link_dir = dir.path().join("config");
    std::fs::create_dir(&link_dir).unwrap();
    let link = link_dir.join("config.toml");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let (_w, rx) = watched(&link);
    std::fs::write(&target, "version = 1\n# edited in the repository\n").unwrap();
    assert!(changed(&rx));
}

#[test]
fn other_files_of_the_directory_are_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "version = 1\n").unwrap();
    let (_w, rx) = watched(&path);
    std::fs::write(dir.path().join("notes.txt"), "hello").unwrap();
    assert!(rx.recv_timeout(Duration::from_secs(2)).is_err());
}
