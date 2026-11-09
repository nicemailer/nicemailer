//! Files of mails to send (`specifications/brouillons.md`, section 7): the
//! attachments of drafts, and the MIME messages of the mails with
//! attachments waiting to leave.
//!
//! ```text
//! <data_dir>/
//!   incoming.noindex/      ← files the app puts there to attach them
//!     3F2C…                ← one per file, named by the app (a UUID)
//!   outgoing.noindex/      ← what the core keeps
//!     ab/
//!       abcdef…0123        ← SHA-256 of the content, no compression
//! ```
//!
//! # Why the app copies the file
//!
//! The core runs as a launch agent, without access to the user's folders:
//! macOS protects Desktop, Documents and Downloads, and only the app, to
//! which the user gave the file (open panel, drag and drop), may read it.
//! The core reads nothing outside its data directory. So the app copies the
//! file into `incoming.noindex` (on APFS a clone: instant, no space taken
//! until one of the two changes), then asks the core to take it
//! ([`OutgoingStore::take`]): hashed while read once, then moved (a
//! rename, on the same volume) into `outgoing.noindex`. From then on the
//! draft depends no more on the user's file, which may be moved, changed
//! or deleted.
//!
//! # Addressed by content
//!
//! A file is named by its hash: the same file attached to two drafts is
//! kept once. Files are read-only (`0400`), directories private (`0700`).
//! No compression: attachments (PDF, images, archives) are mostly
//! compressed already. The table `outgoing_blob` lists them; the garbage
//! collector ([`crate::gc::collect_outgoing`]) deletes those no draft and
//! no waiting mail refer to.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use sha2::{Digest, Sha256};

use crate::raw::{Hash, hex};

/// Directory of the files kept for sending, under the data directory.
pub const OUTGOING_DIR: &str = "outgoing.noindex";
/// Directory where the app puts the files to attach.
pub const INCOMING_DIR: &str = "incoming.noindex";

/// Handle on the two directories. Cheap to clone: two paths.
#[derive(Debug, Clone)]
pub struct OutgoingStore {
    root: PathBuf,
    incoming: PathBuf,
}

impl OutgoingStore {
    /// Opens (and creates if needed) the two directories under `data_dir`,
    /// with mode 0700.
    pub fn open(data_dir: &Path) -> io::Result<OutgoingStore> {
        let root = data_dir.join(OUTGOING_DIR);
        let incoming = data_dir.join(INCOMING_DIR);
        for dir in [&root, &incoming] {
            fs::create_dir_all(dir)?;
            fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
        }
        Ok(OutgoingStore { root, incoming })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn incoming(&self) -> &Path {
        &self.incoming
    }

    /// The file holding `hash`.
    pub fn path(&self, hash: &Hash) -> PathBuf {
        let h = hex(hash);
        self.root.join(&h[..2]).join(h)
    }

    /// Takes the file `staged` the app put in `incoming.noindex`: returns
    /// its hash and size once it is in `outgoing.noindex`.
    ///
    /// `staged` must be a plain name (letters, digits, `-`): never a path,
    /// so that a client cannot make the core read or move any other file.
    /// A symbolic link is refused for the same reason. The file is read
    /// once, in pieces, to hash it; a file of the same content already
    /// kept makes this copy useless, and it is deleted.
    pub fn take(&self, staged: &str) -> io::Result<(Hash, u64)> {
        let valid = !staged.is_empty() && staged.len() <= 64 && staged.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        if !valid {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("not a staged file name: {staged:?}")));
        }
        let src = self.incoming.join(staged);
        let meta = fs::symlink_metadata(&src)?;
        if !meta.file_type().is_file() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "the staged file is not a plain file"));
        }
        let (hash, size) = hash_file(&src)?;
        let dest = self.path(&hash);
        if dest.exists() {
            fs::remove_file(&src)?;
        } else {
            self.make_parent(&dest)?;
            fs::rename(&src, &dest)?;
            fs::set_permissions(&dest, fs::Permissions::from_mode(0o400))?;
        }
        // Now, for the garbage collector's grace delay: a copy keeps the
        // date of the user's file, which may be old.
        touch(&dest);
        Ok((hash, size))
    }

    /// Keeps `bytes` (a MIME message waiting to leave): its hash.
    pub fn write(&self, bytes: &[u8]) -> io::Result<Hash> {
        let hash: Hash = Sha256::digest(bytes).into();
        let dest = self.path(&hash);
        if dest.exists() {
            touch(&dest);
            return Ok(hash);
        }
        self.make_parent(&dest)?;
        // Written aside, then renamed: the file is complete or absent.
        let tmp = self.incoming.join(format!("w-{:016x}{:016x}", rand::random::<u64>(), rand::random::<u64>()));
        let result = (|| {
            let mut f = OpenOptions::new().write(true).create_new(true).mode(0o400).open(&tmp)?;
            f.write_all(bytes)?;
            f.sync_all()?;
            fs::rename(&tmp, &dest)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        result.map(|()| hash)
    }

    pub fn read(&self, hash: &Hash) -> io::Result<Vec<u8>> {
        fs::read(self.path(hash))
    }

    /// Deletes the file of `hash`, if it is there.
    pub fn remove(&self, hash: &Hash) {
        let _ = fs::remove_file(self.path(hash));
    }

    /// The files of `incoming.noindex` older than `age`: left by an app
    /// that stopped between its copy and the core taking the file.
    pub fn stale_incoming(&self, age: Duration) -> Vec<PathBuf> {
        stale(&self.incoming, age)
    }

    /// Every file kept, with its hash: for the garbage collector's sweep.
    pub fn kept(&self) -> Vec<(Hash, PathBuf)> {
        let mut out = Vec::new();
        let Ok(dirs) = fs::read_dir(&self.root) else { return out };
        for d in dirs.flatten() {
            let Ok(files) = fs::read_dir(d.path()) else { continue };
            for f in files.flatten() {
                let name = f.file_name();
                if let Some(hash) = name.to_str().and_then(parse_hex) {
                    out.push((hash, f.path()));
                }
            }
        }
        out
    }

    fn make_parent(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent()
            && !dir.exists()
        {
            fs::create_dir_all(dir)?;
            fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
        }
        Ok(())
    }
}

/// SHA-256 and size of a file, read in pieces of 1 MB.
fn hash_file(path: &Path) -> io::Result<(Hash, u64)> {
    let mut f = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut size = 0u64;
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        size += n as u64;
    }
    Ok((hasher.finalize().into(), size))
}

/// Sets the modification date of a file to now (read-only files too: the
/// date is not the content).
fn touch(path: &Path) {
    if let Ok(f) = File::open(path) {
        let _ = f.set_modified(SystemTime::now());
    }
}

fn stale(dir: &Path, age: Duration) -> Vec<PathBuf> {
    let now = SystemTime::now();
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    entries
        .flatten()
        .filter(|e| e.metadata().and_then(|m| m.modified()).is_ok_and(|t| now.duration_since(t).unwrap_or_default() >= age))
        .map(|e| e.path())
        .collect()
}

/// A hash from its 64 hexadecimal digits.
pub fn parse_hex(s: &str) -> Option<Hash> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_staged_file_is_taken_once_by_its_content() {
        let dir = tempfile::tempdir().unwrap();
        let o = OutgoingStore::open(dir.path()).unwrap();
        fs::write(o.incoming().join("a1"), b"hello").unwrap();
        fs::write(o.incoming().join("b2"), b"hello").unwrap();
        let (h1, size) = o.take("a1").unwrap();
        assert_eq!(size, 5);
        let (h2, _) = o.take("b2").unwrap();
        assert_eq!(h1, h2);
        assert_eq!(o.read(&h1).unwrap(), b"hello");
        // Both staged files are gone; one file is kept, read-only.
        assert_eq!(fs::read_dir(o.incoming()).unwrap().count(), 0);
        assert_eq!(o.kept().len(), 1);
        assert_eq!(fs::metadata(o.path(&h1)).unwrap().permissions().mode() & 0o777, 0o400);
    }

    #[test]
    fn only_a_plain_staged_name_is_taken() {
        let dir = tempfile::tempdir().unwrap();
        let o = OutgoingStore::open(dir.path()).unwrap();
        fs::write(dir.path().join("secret"), b"x").unwrap();
        for bad in ["../secret", "/etc/passwd", ".", "", "a/b", "a.txt"] {
            assert!(o.take(bad).is_err(), "{bad}");
        }
        std::os::unix::fs::symlink(dir.path().join("secret"), o.incoming().join("link")).unwrap();
        assert!(o.take("link").is_err());
        assert!(o.take("missing").is_err());
    }

    #[test]
    fn a_message_is_written_once() {
        let dir = tempfile::tempdir().unwrap();
        let o = OutgoingStore::open(dir.path()).unwrap();
        let h = o.write(b"MIME").unwrap();
        assert_eq!(o.write(b"MIME").unwrap(), h);
        assert_eq!(o.read(&h).unwrap(), b"MIME");
        assert_eq!(parse_hex(&hex(&h)), Some(h));
        assert_eq!(parse_hex("zz"), None);
        o.remove(&h);
        assert!(o.read(&h).is_err());
    }
}
