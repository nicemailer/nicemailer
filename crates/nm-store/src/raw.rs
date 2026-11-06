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
//!
//! ## Streaming writes
//!
//! A message received from an IMAP server comes in pieces (1 MB each above
//! 4 MB, `nm-proto::imap::BODY_CHUNK`), and may weigh 150 MB. [`RawWriter`]
//! takes the pieces one by one: each is hashed and compressed as it
//! arrives, and the compressed bytes go straight to the temporary file. The
//! final name, the hash, is only known at the end, hence the temporary
//! file (`stockage-brut.md`, section 5, steps 1 to 4).
//!
//! The message must still be parsed, and the parser needs all its bytes at
//! once. [`RawWriter`] can therefore keep a plain copy ([`Finished::plain`]): in
//! memory up to [`PLAIN_IN_MEMORY`], else in a second temporary file that
//! is *mapped* into memory at the end (`mmap`). A mapped file is not heap
//! memory: its pages are read from the file on demand and dropped by macOS
//! when memory runs short, so a 150 MB message costs no 150 MB of RAM. The
//! parser (`mail-parser` 1.0) decodes nothing until asked: it only scans
//! the bytes to find the parts, so parsing the mapped copy stays cheap too.
//!
//! [`RawStore::write`], for bytes already in memory (mbox import), is the
//! same writer fed in one piece.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

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

/// Up to this size, the plain copy kept for parsing ([`Finished::plain`]) stays in
/// memory; above, it goes to a temporary file mapped at the end. 16 MB:
/// nearly every mail fits (the IMAP client fetches up to 4 MB in one
/// piece), and a few workers holding that much each stay well within
/// memory.
pub const PLAIN_IN_MEMORY: usize = 16 << 20;

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

/// Handle on the raw directory. Cheap to clone: a path, and the optional
/// simulated disk of the tests.
#[derive(Debug, Clone)]
pub struct RawStore {
    root: PathBuf,
    /// Tests only ([`RawStore::with_disk_space`]): bytes that may still be
    /// written before the "disk" is full. Shared by every writer of the
    /// store, as a real disk is.
    ///
    /// Rust note: `Arc` (atomically reference-counted pointer) lets several
    /// clones of the store, on several threads, share one counter;
    /// `AtomicU64` lets them change it without a lock.
    space: Option<Arc<AtomicU64>>,
    /// Hashes written or found already stored, whose rows are not
    /// committed yet: the garbage collector leaves them alone
    /// (`stockage-brut.md`, section 7, "Concurrence entre ingestion et
    /// GC"). Shared by every clone of the store.
    pins: Arc<Mutex<HashMap<Hash, usize>>>,
}

/// A hash protected from the garbage collector while it lives: from the
/// moment the file is known to be there (written, or found already) until
/// the row that references it is committed, when the caller drops it.
#[derive(Debug)]
pub struct Pin {
    pins: Arc<Mutex<HashMap<Hash, usize>>>,
    hash: Hash,
}

impl Drop for Pin {
    fn drop(&mut self) {
        let mut pins = self.pins.lock().unwrap();
        if let Some(n) = pins.get_mut(&self.hash) {
            *n -= 1;
            if *n == 0 {
                pins.remove(&self.hash);
            }
        }
    }
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
        Ok(RawStore { root, space: None, pins: Arc::default() })
    }

    /// Tests only: simulates a disk with `bytes` of free space left. Every
    /// write beyond fails with `StorageFull`, the error of a full disk
    /// (`ENOSPC`), so the tests can check that nothing is left behind.
    #[doc(hidden)]
    pub fn with_disk_space(mut self, bytes: u64) -> RawStore {
        self.space = Some(Arc::new(AtomicU64::new(bytes)));
        self
    }

    /// Starts a streaming write ([`RawWriter`]). `keep_plain`: also keep
    /// the plain bytes, for parsing ([`Finished::plain`]).
    pub fn writer(&self, keep_plain: bool) -> io::Result<RawWriter> {
        let tmp = self.tmp_path("");
        let file = self.create(&tmp, 0o400)?;
        // `BufWriter`: zstd hands its output in small pieces; 256 KB of
        // buffer turns them into few large writes.
        let encoder = match zstd::stream::write::Encoder::new(BufWriter::with_capacity(256 << 10, file), ZSTD_LEVEL) {
            Ok(e) => e,
            Err(e) => {
                let _ = fs::remove_file(&tmp);
                return Err(e);
            }
        };
        Ok(RawWriter {
            store: self.clone(),
            tmp,
            encoder: Some(encoder),
            hasher: Sha256::new(),
            size: 0,
            plain: if keep_plain { Plain::Memory(Vec::new()) } else { Plain::None },
        })
    }

    /// A new, unique path in `.tmp/`.
    fn tmp_path(&self, suffix: &str) -> PathBuf {
        self.root.join(".tmp").join(format!("{:016x}{:016x}{suffix}", rand::random::<u64>(), rand::random::<u64>()))
    }

    /// Creates a new file (never an existing one) with `mode`, behind the
    /// simulated disk of the tests if any.
    fn create(&self, path: &Path, mode: u32) -> io::Result<Disk> {
        // `mode` applies at creation: a read-only mode (0o400) does not stop
        // writing through this already-open handle.
        let file = OpenOptions::new().write(true).create_new(true).mode(mode).open(path)?;
        Ok(Disk { file, space: self.space.clone() })
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
    /// The bytes are hashed first: a message already stored (an import run
    /// twice) then costs no compression at all.
    pub fn write(&self, bytes: &[u8]) -> io::Result<Written> {
        Ok(self.write_pinned(bytes)?.0)
    }

    /// [`RawStore::write`], with the hash pinned until the caller has
    /// committed the rows that reference it.
    pub fn write_pinned(&self, bytes: &[u8]) -> io::Result<(Written, Pin)> {
        let hash = hash(bytes);
        {
            let mut pins = self.pins.lock().unwrap();
            if let Ok(meta) = fs::metadata(self.path(&hash)) {
                *pins.entry(hash).or_default() += 1;
                drop(pins);
                return Ok((Written { hash, size: bytes.len() as u64, stored_size: meta.len(), existed: true }, self.pin_taken(hash)));
            }
        }
        let mut w = self.writer(false)?;
        w.write(bytes)?;
        let f = w.finish(Some(bytes.len() as u64))?;
        Ok((f.written, f.pin))
    }

    /// The `Pin` of a hash already counted in `pins`.
    fn pin_taken(&self, hash: Hash) -> Pin {
        Pin { pins: self.pins.clone(), hash }
    }

    /// Runs `f` with the pins locked, given a test of whether a hash is
    /// pinned: no writer can find a file "already there" meanwhile. The
    /// garbage collector deletes files inside it.
    pub fn with_pins_locked<R>(&self, f: impl FnOnce(&dyn Fn(&Hash) -> bool) -> R) -> R {
        let pins = self.pins.lock().unwrap();
        f(&|h: &Hash| pins.contains_key(h))
    }

    /// Root of the raw store.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Moves the file of `hash` to `.corrupt/`, out of the way and kept for
    /// inspection (`stockage-brut.md`, section 6). `false`: no such file.
    pub fn set_aside(&self, hash: &Hash) -> io::Result<bool> {
        let from = self.path(hash);
        if !from.exists() {
            return Ok(false);
        }
        let to = self.root.join(".corrupt").join(format!("{}-{:08x}.eml.zst", hex(hash), rand::random::<u32>()));
        fs::rename(from, to)?;
        Ok(true)
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

/// A message being written in pieces ([`RawStore::writer`]): hashed and
/// compressed as the pieces arrive, written to `.tmp/`, given its final
/// name by [`RawWriter::finish`].
///
/// Dropped without `finish` (an error, a connection cut in the middle of
/// the message), it removes its temporary files: a message partially
/// received is never stored (`stockage-brut.md`, section 5).
pub struct RawWriter {
    store: RawStore,
    /// The compressed temporary file.
    tmp: PathBuf,
    /// `Option` so that `finish` can take it out (`Option::take`): the
    /// encoder must be consumed to write its last bytes.
    encoder: Option<zstd::stream::write::Encoder<'static, BufWriter<Disk>>>,
    hasher: Sha256,
    size: u64,
    plain: Plain,
}

/// The plain copy of a message being written, kept for parsing.
enum Plain {
    /// Not asked for.
    None,
    Memory(Vec<u8>),
    /// Above [`PLAIN_IN_MEMORY`]: in a temporary file.
    Spilled { file: BufWriter<Disk>, path: PathBuf },
}

/// What [`RawWriter::finish`] returns.
pub struct Finished {
    pub written: Written,
    /// The plain bytes, when asked for ([`RawStore::writer`]).
    pub plain: Option<PlainBytes>,
    /// Keeps the file from the garbage collector until the caller has
    /// committed its row: drop it after the commit.
    pub pin: Pin,
}

/// The plain bytes of a message: in memory, or a mapped file. Either way a
/// `&[u8]` (`Deref`), for the parser.
pub enum PlainBytes {
    Memory(Vec<u8>),
    Mapped(Mapped),
}

impl std::ops::Deref for PlainBytes {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            PlainBytes::Memory(v) => v,
            PlainBytes::Mapped(m) => m.as_slice(),
        }
    }
}

impl RawWriter {
    /// Adds the next piece of the message.
    pub fn write(&mut self, chunk: &[u8]) -> io::Result<()> {
        self.hasher.update(chunk);
        self.size += chunk.len() as u64;
        self.encoder.as_mut().expect("not finished").write_all(chunk)?;
        match &mut self.plain {
            Plain::None => {}
            Plain::Spilled { file, .. } => file.write_all(chunk)?,
            Plain::Memory(v) if v.len() + chunk.len() <= PLAIN_IN_MEMORY => v.extend_from_slice(chunk),
            Plain::Memory(v) => {
                // Too large for memory: what was kept goes to a file, and
                // the rest will follow it there.
                let path = self.store.tmp_path(".plain");
                // `Plain` is replaced before anything can fail, so that
                // `Drop` knows about the file and removes it.
                let kept = std::mem::take(v);
                let file = BufWriter::with_capacity(256 << 10, self.store.create(&path, 0o600)?);
                self.plain = Plain::Spilled { file, path };
                if let Plain::Spilled { file, .. } = &mut self.plain {
                    file.write_all(&kept)?;
                    file.write_all(chunk)?;
                }
            }
        }
        Ok(())
    }

    /// Bytes received so far.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// Ends the message: checks its size against `expected` (the size the
    /// server announced, if known), makes the file durable and gives it its
    /// final name, or removes it if that hash is already stored.
    ///
    /// On error, nothing is left in `.tmp/` and nothing was stored.
    pub fn finish(mut self, expected: Option<u64>) -> io::Result<Finished> {
        if let Some(n) = expected
            && n != self.size
        {
            // `self` is dropped on return: `Drop` removes the files.
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("message of {} bytes, {n} announced", self.size),
            ));
        }
        let hash: Hash = std::mem::take(&mut self.hasher).finalize().into();
        let size = self.size;
        // `finish` writes zstd's last bytes; `into_inner` flushes the
        // buffer and gives the file back (an error here is a write error).
        let disk = self.encoder.take().expect("not finished").finish()?.into_inner().map_err(|e| e.into_error())?;
        fsync(&disk.file)?;
        let stored_size = disk.file.metadata()?.len();
        drop(disk);

        let path = self.store.path(&hash);
        // Under the pins' lock: the garbage collector, which deletes files
        // under it too, cannot remove this file between the check and the
        // pin.
        let parent = path.parent().expect("path has a parent");
        let existed = {
            let mut pins = self.store.pins.lock().unwrap();
            let existed = path.exists();
            if existed {
                // Same bytes already stored: the copy just written is useless.
                fs::remove_file(&self.tmp)?;
            } else {
                fs::create_dir_all(parent)?;
                fs::rename(&self.tmp, &path)?;
            }
            *pins.entry(hash).or_default() += 1;
            existed
        };
        let pin = self.store.pin_taken(hash);
        if !existed {
            // The new directory entry, so the rename itself is not lost;
            // outside the lock, which other writers wait for.
            fsync(&File::open(parent)?)?;
        }
        // From here, `Drop` has no temporary file to remove: mark it so
        // (an empty path is never removed).
        self.tmp = PathBuf::new();
        let stored_size = if existed { fs::metadata(&path)?.len() } else { stored_size };

        let plain = match std::mem::replace(&mut self.plain, Plain::None) {
            Plain::None => None,
            Plain::Memory(v) => Some(PlainBytes::Memory(v)),
            Plain::Spilled { file, path } => {
                let result = file.into_inner().map_err(|e| e.into_error()).and_then(|_| Mapped::open(&path, size));
                // Removed at once: a mapped file stays readable after its
                // name is gone, and nothing can be left behind.
                let _ = fs::remove_file(&path);
                Some(PlainBytes::Mapped(result?))
            }
        };
        Ok(Finished { written: Written { hash, size, stored_size, existed }, plain, pin })
    }
}

impl Drop for RawWriter {
    fn drop(&mut self) {
        // Rust note: `Drop::drop` runs when the value goes out of scope,
        // whatever the path: an early `return`, a `?`, a panic. That makes
        // it the one place to clean up after a write that did not finish.
        if !self.tmp.as_os_str().is_empty() {
            let _ = fs::remove_file(&self.tmp);
        }
        if let Plain::Spilled { path, .. } = &self.plain {
            let _ = fs::remove_file(path);
        }
    }
}

/// A file being written by the raw store, behind the simulated disk of the
/// tests when there is one ([`RawStore::with_disk_space`]).
#[derive(Debug)]
struct Disk {
    file: File,
    space: Option<Arc<AtomicU64>>,
}

impl Write for Disk {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if let Some(space) = &self.space {
            // Take `buf.len()` from the free space, or fail without writing.
            let len = buf.len() as u64;
            if space.try_update(Ordering::SeqCst, Ordering::SeqCst, |left| left.checked_sub(len)).is_err() {
                return Err(io::Error::new(io::ErrorKind::StorageFull, "No space left on device (simulated)"));
            }
        }
        self.file.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

/// A file mapped read-only into memory (`mmap`).
///
/// Rust note: the standard library has no `mmap`; `libc` gives the C call.
/// The `unsafe` blocks below are sound because the mapping is private to
/// this value, read-only, unmapped exactly once (in `Drop`), and the slice
/// it gives cannot outlive it (its lifetime is tied to `&self`). The file
/// is a temporary file of the store that nobody else writes.
pub struct Mapped {
    ptr: *mut libc::c_void,
    len: usize,
}

// Rust note: a raw pointer is neither `Send` nor `Sync` by default, since
// the compiler cannot know what it points to. A read-only mapping can be
// read from any thread, so these promises hold.
unsafe impl Send for Mapped {}
unsafe impl Sync for Mapped {}

impl Mapped {
    fn open(path: &Path, len: u64) -> io::Result<Mapped> {
        use std::os::fd::AsRawFd;
        let file = File::open(path)?;
        if file.metadata()?.len() != len {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "plain copy of the wrong size"));
        }
        let len = usize::try_from(len).map_err(|_| io::Error::other("message too large to map"))?;
        if len == 0 {
            // mmap refuses an empty length.
            return Ok(Mapped { ptr: std::ptr::null_mut(), len: 0 });
        }
        let ptr = unsafe { libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ, libc::MAP_PRIVATE, file.as_raw_fd(), 0) };
        if ptr == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        // The mapping keeps the file alive; the descriptor can be closed.
        Ok(Mapped { ptr, len })
    }

    pub fn as_slice(&self) -> &[u8] {
        if self.len == 0 {
            return &[];
        }
        unsafe { std::slice::from_raw_parts(self.ptr as *const u8, self.len) }
    }
}

impl Drop for Mapped {
    fn drop(&mut self) {
        if self.len > 0 {
            unsafe { libc::munmap(self.ptr, self.len) };
        }
    }
}

impl RawStore {
    /// Checks that the file of `hash` exists and still holds the bytes of
    /// that hash, reading it as a stream: a 150 MB message is checked with
    /// a few MB of memory (`nicemailer sync verify`). `Ok(false)`: the
    /// file is missing; an `InvalidData` error: it is corrupt.
    pub fn verify(&self, hash: &Hash) -> io::Result<bool> {
        let file = match File::open(self.path(hash)) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e),
        };
        let mut decoder = zstd::stream::read::Decoder::new(file)?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 256 << 10];
        loop {
            let n = decoder.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        let got: Hash = hasher.finalize().into();
        if &got != hash {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("brut corrompu : {}", hex(hash))));
        }
        Ok(true)
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

    fn tmp_count(dir: &Path) -> usize {
        fs::read_dir(dir.join(RAW_DIR).join(".tmp")).unwrap().count()
    }

    #[test]
    fn pieces_give_the_same_file_as_one_write() {
        let dir = tempfile::tempdir().unwrap();
        let raw = RawStore::open(dir.path()).unwrap();
        let bytes: Vec<u8> = (0..300_000u32).flat_map(|i| i.to_le_bytes()).collect();
        let mut w = raw.writer(true).unwrap();
        for piece in bytes.chunks(7_777) {
            w.write(piece).unwrap();
        }
        assert_eq!(w.size(), bytes.len() as u64);
        let f = w.finish(Some(bytes.len() as u64)).unwrap();
        assert!(!f.written.existed);
        assert_eq!(f.written.hash, hash(&bytes));
        assert_eq!(&*f.plain.unwrap(), &bytes[..], "the plain copy is the message");
        assert_eq!(raw.read(&f.written.hash).unwrap(), bytes);
        // Written again in one piece: already there.
        let again = raw.write(&bytes).unwrap();
        assert!(again.existed);
        assert_eq!(again.stored_size, f.written.stored_size);
        assert_eq!(tmp_count(dir.path()), 0);
    }

    #[test]
    fn a_stored_message_written_again_in_pieces_leaves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let raw = RawStore::open(dir.path()).unwrap();
        let first = raw.write(b"Subject: x\r\n\r\nsame\r\n").unwrap();
        let mut w = raw.writer(false).unwrap();
        w.write(b"Subject: x\r\n\r\n").unwrap();
        w.write(b"same\r\n").unwrap();
        let f = w.finish(None).unwrap();
        assert!(f.written.existed && f.plain.is_none());
        assert_eq!(f.written.hash, first.hash);
        assert_eq!(f.written.stored_size, first.stored_size);
        assert_eq!(tmp_count(dir.path()), 0);
    }

    #[test]
    fn a_size_other_than_announced_stores_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let raw = RawStore::open(dir.path()).unwrap();
        let mut w = raw.writer(true).unwrap();
        w.write(b"cut in the mid").unwrap();
        let err = w.finish(Some(1000)).err().unwrap();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("14 bytes, 1000 announced"), "{err}");
        assert!(!raw.path(&hash(b"cut in the mid")).exists());
        assert_eq!(tmp_count(dir.path()), 0);
    }

    #[test]
    fn a_writer_dropped_unfinished_leaves_nothing() {
        // The connection is cut in the middle of a message: the writer is
        // dropped with half the bytes, the plain copy already on disk.
        let dir = tempfile::tempdir().unwrap();
        let raw = RawStore::open(dir.path()).unwrap();
        let mut w = raw.writer(true).unwrap();
        w.write(&vec![b'a'; PLAIN_IN_MEMORY + 10]).unwrap();
        assert_eq!(tmp_count(dir.path()), 2, "compressed file and plain copy");
        drop(w);
        assert_eq!(tmp_count(dir.path()), 0);
    }

    #[test]
    fn a_large_plain_copy_is_a_mapped_file() {
        let dir = tempfile::tempdir().unwrap();
        let raw = RawStore::open(dir.path()).unwrap();
        let bytes: Vec<u8> = (0..PLAIN_IN_MEMORY + 3_000_000).map(|i| (i % 251) as u8).collect();
        let mut w = raw.writer(true).unwrap();
        for piece in bytes.chunks(1 << 20) {
            w.write(piece).unwrap();
        }
        let f = w.finish(Some(bytes.len() as u64)).unwrap();
        let plain = f.plain.unwrap();
        assert!(matches!(plain, PlainBytes::Mapped(_)));
        assert!(*plain == bytes[..]);
        // Its file is already gone: nothing to clean up, even after a crash.
        assert_eq!(tmp_count(dir.path()), 0);
        assert_eq!(raw.read(&f.written.hash).unwrap(), bytes);
    }

    #[test]
    fn a_full_disk_stores_nothing_and_leaves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let raw = RawStore::open(dir.path()).unwrap().with_disk_space(100_000);
        // Random bytes do not compress: the compressed file is as large.
        let bytes: Vec<u8> = (0..2_000_000).map(|_| rand::random::<u8>()).collect();
        let mut w = raw.writer(true).unwrap();
        let mut failed = None;
        for piece in bytes.chunks(64 << 10) {
            if let Err(e) = w.write(piece) {
                failed = Some(e);
                break;
            }
        }
        // The caller gives up on the error: the writer is dropped.
        let err = match failed {
            Some(e) => {
                drop(w);
                e
            }
            None => w.finish(Some(bytes.len() as u64)).err().expect("the disk is full"),
        };
        assert_eq!(err.kind(), io::ErrorKind::StorageFull, "{err}");
        assert!(!raw.path(&hash(&bytes)).exists());
        assert_eq!(tmp_count(dir.path()), 0);
        // Same for a write in one piece.
        assert_eq!(raw.write(&bytes).unwrap_err().kind(), io::ErrorKind::StorageFull);
        assert_eq!(tmp_count(dir.path()), 0);
    }

    #[test]
    fn verify_reads_as_a_stream() {
        let dir = tempfile::tempdir().unwrap();
        let raw = RawStore::open(dir.path()).unwrap();
        let w = raw.write(b"original").unwrap();
        assert!(raw.verify(&w.hash).unwrap());
        assert!(!raw.verify(&hash(b"never stored")).unwrap());
        let path = raw.path(&w.hash);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        fs::write(&path, zstd::bulk::compress(b"tampered", 3).unwrap()).unwrap();
        assert_eq!(raw.verify(&w.hash).unwrap_err().kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn hex_is_lowercase_and_64_chars() {
        let h = hash(b"");
        assert_eq!(hex(&h), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    }
}
