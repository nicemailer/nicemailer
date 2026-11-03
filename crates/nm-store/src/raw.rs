//! Raw message storage (`stockage-brut.md`): every complete message is kept
//! as an immutable, zstd-compressed file named after the SHA-256 of its
//! bytes.
//!
//! Layout (section 3 of the spec):
//!
//! ```text
//! <data_dir>/raw.noindex/        ".noindex": Spotlight skips it
//!   .tmp/                        files being written
//!   ab/                          first two hex characters of the hash
//!     abcdef…0123.eml.zst        full hash, 64 lowercase hex characters
//! ```
//!
//! Why content addressing: the file name depends only on the bytes, so the
//! same message seen twice (two folders, or an import run twice) is stored
//! once, and a file can be verified at any time by hashing it again.
//!
//! Why write to `.tmp/` then `rename`: on the same volume, `rename` is
//! atomic. A crash leaves either no final file or a complete one, never a
//! truncated file under a valid name. Leftovers in `.tmp/` are harmless and
//! will be swept by the orphan scan (section 7, not implemented yet).
//!
//! This module only deals with files. The matching `raw_blob` row is written
//! by the caller, inside the same SQLite transaction as the message
//! (section 5, step 5).
//!
//! ## Durability on macOS
//!
//! `fsync` on macOS pushes the data to the drive but does not flush the
//! drive's own cache: after a power loss, data still in that cache is lost.
//! Only `fcntl(F_FULLFSYNC)` flushes it, and Rust's `File::sync_all` uses
//! it on Apple platforms. It costs about 10 ms and drives serialize it, so
//! calling it for every file capped an import at 22 MB/s.
//!
//! So each file gets a plain `fsync` (data handed to the drive), and the
//! caller calls [`RawStore::barrier`] once per batch, before committing the
//! batch's rows: a single `F_FULLFSYNC` flushes the whole drive cache, every
//! file of the batch included. The guarantee is unchanged (a `raw_blob` row
//! never exists without its file on stable storage) for one flush per 500
//! messages instead of two per message.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

// Rust note: `sha2::Digest` is a trait; importing it brings the `digest`
// method into scope on `Sha256`.
use sha2::{Digest, Sha256};

/// Name of the raw directory inside the data directory.
pub const RAW_DIR: &str = "raw.noindex";

/// zstd compression level. Measured on 300 MB of a real mailbox (one
/// thread): level 3 compresses at about 640 MB/s down to 60 %, level 9 at
/// about 120 MB/s down to 55 %, level 15 at 18 MB/s for almost no further
/// gain. Raw files are written once and rarely read, so level 9 is a good
/// trade-off: an import on 10 cores still compresses faster than a disk
/// reads (spec section 5, "Niveau de compression").
pub const ZSTD_LEVEL: i32 = 9;

/// A SHA-256 hash: 32 bytes.
///
/// Rust note: `[u8; 32]` is a fixed-size array stored inline (no heap
/// allocation). It is `Copy`, so passing it around copies the 32 bytes,
/// which is cheap.
pub type Hash = [u8; 32];

/// Computes the SHA-256 of `bytes`.
pub fn hash(bytes: &[u8]) -> Hash {
    Sha256::digest(bytes).into()
}

/// Lowercase hexadecimal form of a hash, used as the file name.
pub fn hex(hash: &Hash) -> String {
    // `write!` into a `String` cannot fail, hence `let _ =`.
    use std::fmt::Write as _;
    let mut s = String::with_capacity(64);
    for b in hash {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Result of [`RawStore::write`].
#[derive(Debug, Clone, Copy)]
pub struct Written {
    pub hash: Hash,
    /// Size of the original bytes.
    pub size: u64,
    /// Size of the compressed file on disk.
    pub stored_size: u64,
    /// `true` if the file already existed (deduplication): nothing was
    /// written this time.
    pub existed: bool,
}

/// Handle on the raw directory. Cheap to clone: it is only a path.
#[derive(Debug, Clone)]
pub struct RawStore {
    root: PathBuf,
}

impl RawStore {
    /// Opens (and creates if needed) the raw directory under `data_dir`.
    /// Directories are created with mode 0700: only the user can list them.
    pub fn open(data_dir: &Path) -> io::Result<RawStore> {
        let root = data_dir.join(RAW_DIR);
        for dir in [root.clone(), root.join(".tmp"), root.join(".corrupt")] {
            fs::create_dir_all(&dir)?;
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
        }
        Ok(RawStore { root })
    }

    /// Final path of the file holding `hash`.
    pub fn path(&self, hash: &Hash) -> PathBuf {
        let h = hex(hash);
        // `&h[..2]` is a string slice of the first two characters; hex
        // digits are ASCII, so byte indices are character indices.
        self.root.join(&h[..2]).join(format!("{h}.eml.zst"))
    }

    /// Stores `bytes` and returns their hash. If a file with the same hash
    /// already exists, nothing is written (`existed = true`).
    ///
    /// Steps (spec section 5): compress into `.tmp/<random>`, `fsync` the
    /// file, `rename` it to its final name, `fsync` the parent directory so
    /// the rename itself survives a power loss.
    ///
    /// The whole message is in memory here: an mbox import needs the full
    /// bytes anyway to parse the message. The IMAP sync will need a
    /// streaming variant (spec section 5, step 1).
    pub fn write(&self, bytes: &[u8]) -> io::Result<Written> {
        let hash = hash(bytes);
        let size = bytes.len() as u64;
        let path = self.path(&hash);
        if let Ok(meta) = fs::metadata(&path) {
            return Ok(Written { hash, size, stored_size: meta.len(), existed: true });
        }
        let compressed = zstd::bulk::compress(bytes, ZSTD_LEVEL)?;
        let tmp = self.root.join(".tmp").join(format!("{}-{:016x}", hex(&hash), rand::random::<u64>()));
        // The `{ }` block closes the file before the rename.
        {
            // `mode(0o400)`: read-only for the owner, since the content never
            // changes. The mode applies at creation; we can still write
            // through this already-open handle.
            let mut f = OpenOptions::new().write(true).create_new(true).mode(0o400).open(&tmp)?;
            f.write_all(&compressed)?;
            fsync(&f)?;
        }
        let parent = path.parent().expect("path has a parent");
        if let Err(e) = fs::create_dir_all(parent).and_then(|_| fs::rename(&tmp, &path)) {
            // Do not leave the temporary file behind on failure (disk full…).
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
        // The new directory entry, so the rename itself is not lost.
        fsync(&File::open(parent)?)?;
        Ok(Written { hash, size, stored_size: compressed.len() as u64, existed: false })
    }

    /// Makes every file written so far durable, drive cache included (see
    /// "Durability on macOS" above). Call it before committing the rows that
    /// reference the files.
    pub fn barrier(&self) -> io::Result<()> {
        // On Apple platforms `sync_all` is `F_FULLFSYNC`; elsewhere it is a
        // plain `fsync`, which already flushes the drive cache on Linux.
        File::open(&self.root)?.sync_all()
    }

    /// Reads and decompresses the message `hash`, checking that its content
    /// still matches the hash. A mismatch returns an `InvalidData` error
    /// (`RawCorrupt` in the spec, section 6).
    pub fn read(&self, hash: &Hash) -> io::Result<Vec<u8>> {
        let mut compressed = Vec::new();
        File::open(self.path(hash))?.read_to_end(&mut compressed)?;
        let bytes = zstd::stream::decode_all(&compressed[..])?;
        if &self::hash(&bytes) != hash {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("brut corrompu : {}", hex(hash))));
        }
        Ok(bytes)
    }
}

/// Plain `fsync(2)`: hands the file's data to the drive without the costly
/// drive-cache flush of `F_FULLFSYNC` (see the module documentation).
///
/// Rust note: `unsafe` marks a call the compiler cannot check, here a C
/// function from the `libc` crate. It is safe as long as the file
/// descriptor is valid, which holds while `f` is borrowed.
fn fsync(f: &File) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    if unsafe { libc::fsync(f.as_raw_fd()) } == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_then_read_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let raw = RawStore::open(dir.path()).unwrap();
        let w = raw.write(b"Subject: hi\r\n\r\nbody\r\n").unwrap();
        assert!(!w.existed);
        assert_eq!(w.size, 21);
        assert_eq!(raw.read(&w.hash).unwrap(), b"Subject: hi\r\n\r\nbody\r\n");
        // Final file is read-only, under its two-character fan-out directory.
        let path = raw.path(&w.hash);
        assert!(path.parent().unwrap().ends_with(&hex(&w.hash)[..2]));
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o400);
        // Nothing is left in .tmp.
        assert_eq!(fs::read_dir(dir.path().join(RAW_DIR).join(".tmp")).unwrap().count(), 0);
    }

    #[test]
    fn same_bytes_are_stored_once() {
        let dir = tempfile::tempdir().unwrap();
        let raw = RawStore::open(dir.path()).unwrap();
        let a = raw.write(b"same").unwrap();
        let b = raw.write(b"same").unwrap();
        assert!(!a.existed && b.existed);
        assert_eq!(a.hash, b.hash);
        assert_eq!(a.stored_size, b.stored_size);
    }

    #[test]
    fn corruption_is_detected() {
        let dir = tempfile::tempdir().unwrap();
        let raw = RawStore::open(dir.path()).unwrap();
        let w = raw.write(b"original").unwrap();
        // Replace the file with the compressed form of other bytes.
        let path = raw.path(&w.hash);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        fs::write(&path, zstd::bulk::compress(b"tampered", 3).unwrap()).unwrap();
        let err = raw.read(&w.hash).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn hex_is_lowercase_and_64_chars() {
        let h = hash(b"");
        assert_eq!(hex(&h), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    }
}
