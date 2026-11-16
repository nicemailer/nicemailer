//! The backup of the database around a migration (`mise-a-jour.md`,
//! section 4, "Migration de la base").
//!
//! Each step of a migration is one transaction (`migrations.rs`): a step
//! that fails is rolled back, and the database stays at the previous
//! version, consistent. That is not enough after an update: a database
//! migrated from version 9 to 11 that fails at 12 is at a version no
//! released NiceMailer ever wrote, and the user who goes back to the
//! previous NiceMailer finds it refused. So before migrating, the core
//! copies the database; if the migration fails, the copy is put back, and
//! the database is exactly as the previous version left it.
//!
//! What is copied: both files of the database (`nicemailer.db` and the
//! bodies, `nicemailer_body.db`, since version 10 a migration may change
//! either) with their write-ahead logs, which may hold the last commits
//! when the previous core did not close cleanly. Not the raw files: their
//! format does not depend on the schema.
//!
//! How: `std::fs::copy`, which on APFS (every Mac since 2017) clones the
//! file: instant, and no space taken until the two copies differ. A copy
//! of 11 GB of bodies through SQLite's backup API, as the specification
//! first said, would have taken minutes and needed 11 GB free before every
//! migration. On another file system it is a real copy: slower, still
//! correct. Copying the files is safe because nothing has them open: the
//! core holds the lock of its data directory, and copies them before it
//! opens the database.
//!
//! Names: `nicemailer.db.bak-v9`, and `nicemailer.db.bak-v9-wal` for its
//! log, so that the copy opens as a database on its own (SQLite looks for
//! the log of `X` at `X-wal`).
//!
//! The copy is kept after a successful migration, in case the new version
//! turns out to be wrong, and removed at the next migration
//! ([`remove_backups`]). As the database changes, the clone shares fewer
//! blocks with it and takes more space: at most the size of the database.

use std::io;
use std::path::{Path, PathBuf};

/// The suffix of a backup: `.bak-v` then the schema version it holds.
const SUFFIX: &str = ".bak-v";

/// The files of the database at `db`, in the order they are copied: the
/// main file, its log, the body file, its log.
fn database_files(db: &Path) -> Vec<PathBuf> {
    let body = crate::body_path(db);
    vec![db.to_path_buf(), with_suffix(db, "-wal"), body.clone(), with_suffix(&body, "-wal")]
}

/// `path` with `suffix` added to its file name.
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// The backup of `file` holding schema `version`: `nicemailer.db.bak-v9`,
/// `nicemailer.db.bak-v9-wal` for a log.
fn backup_name(file: &Path, version: i64) -> PathBuf {
    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    // The log keeps `-wal` at the end, after the version.
    let name = match name.strip_suffix("-wal") {
        Some(base) => format!("{base}{SUFFIX}{version}-wal"),
        None => format!("{name}{SUFFIX}{version}"),
    };
    file.with_file_name(name)
}

/// A backup made by [`backup`]: each file of the database that existed,
/// with its copy.
#[derive(Debug)]
pub struct Backup {
    /// The schema version it holds.
    pub version: i64,
    /// `(file, copy)`, in the order of [`database_files`].
    files: Vec<(PathBuf, PathBuf)>,
    /// The files of the database, also those that did not exist: a log
    /// that the migration created must go when the backup is put back.
    all: Vec<PathBuf>,
}

impl Backup {
    /// The copies, for the log and the tests.
    pub fn copies(&self) -> impl Iterator<Item = &Path> {
        self.files.iter().map(|(_, copy)| copy.as_path())
    }

    /// Puts the database back as it was when the backup was made. Nothing
    /// may have the database open. Each file of the database is removed,
    /// with its index of shared memory (`-shm`, rebuilt by SQLite from the
    /// log), then the copies are renamed into place: a rename on the same
    /// disk, instant whatever the size. The copies are gone afterwards:
    /// the database is them.
    pub fn restore(&self) -> io::Result<()> {
        for file in &self.all {
            remove_if_present(file)?;
            if !file.to_string_lossy().ends_with("-wal") {
                remove_if_present(&with_suffix(file, "-shm"))?;
            }
        }
        for (file, copy) in &self.files {
            std::fs::rename(copy, file)?;
        }
        Ok(())
    }
}

/// Removes `path`, unless it is already missing.
fn remove_if_present(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Copies the database at `db`, which holds schema `version`, next to it
/// (module documentation). Nothing may have it open. A copy that fails
/// halfway is removed, and the error returned: the migration must then not
/// start, since it could not be undone.
pub fn backup(db: &Path, version: i64) -> io::Result<Backup> {
    let all = database_files(db);
    let mut files = Vec::new();
    for file in &all {
        if !file.exists() {
            continue;
        }
        let copy = backup_name(file, version);
        // A copy left by an earlier attempt at the same version: replaced
        // (`copy` would overwrite it, but a clone needs the name free).
        remove_if_present(&copy)?;
        if let Err(e) = std::fs::copy(file, &copy) {
            for (_, done) in &files {
                let _ = std::fs::remove_file(done);
            }
            let _ = std::fs::remove_file(&copy);
            return Err(io::Error::new(e.kind(), format!("copie de {} : {e}", file.display())));
        }
        files.push((file.clone(), copy));
    }
    Ok(Backup { version, files, all })
}

/// Removes the backups of earlier migrations next to `db` (any version):
/// done before a new migration, which makes its own (`mise-a-jour.md`,
/// section 4: kept until the next update). Returns how many files went.
pub fn remove_backups(db: &Path) -> io::Result<usize> {
    let Some(dir) = db.parent() else { return Ok(0) };
    // The names of the database's files, whose backups start with them.
    let prefixes: Vec<String> = database_files(db)
        .iter()
        .filter(|f| !f.to_string_lossy().ends_with("-wal"))
        .filter_map(|f| f.file_name().map(|n| format!("{}{SUFFIX}", n.to_string_lossy())))
        .collect();
    let mut removed = 0;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(rest) = prefixes.iter().find_map(|p| name.strip_prefix(p.as_str())) else { continue };
        // `9` or `9-wal`: a version number, nothing else (a file the user
        // named `nicemailer.db.bak-v9-mine` is not ours).
        let version = rest.strip_suffix("-wal").unwrap_or(rest);
        if !version.is_empty() && version.bytes().all(|b| b.is_ascii_digit()) {
            std::fs::remove_file(entry.path())?;
            removed += 1;
        }
    }
    Ok(removed)
}

/// Why [`migrate`] did not give an upgraded store.
#[derive(Debug)]
pub enum MigrationError {
    /// The backup could not be made: nothing was migrated (the migration
    /// is not started when it could not be undone).
    Backup(io::Error),
    /// The migration failed; `restored` says whether the backup was put
    /// back (an error there leaves the database as the failure left it, at
    /// the last version committed, and the backup in place).
    Migration { error: crate::StoreError, restored: io::Result<()> },
}

impl std::fmt::Display for MigrationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MigrationError::Backup(e) => write!(f, "sauvegarde avant la mise à jour impossible : {e}"),
            MigrationError::Migration { error, restored: Ok(()) } => {
                write!(f, "{error} ; la base a été remise dans son état d'avant la mise à jour")
            }
            MigrationError::Migration { error, restored: Err(e) } => {
                write!(f, "{error} ; la sauvegarde n'a pas pu être remise ({e})")
            }
        }
    }
}

impl std::error::Error for MigrationError {}

/// Migrates the database at `db`, which holds schema `from`, with a backup
/// (module documentation): the backups of earlier migrations are removed,
/// a new one is made, then the store is opened, which migrates it
/// (`on_version`: each version reached, [`crate::Store::open_migrating`]).
/// If the opening fails, the backup is put back. Nothing may have the
/// database open.
pub fn migrate(
    db: &Path,
    from: i64,
    on_version: &dyn Fn(i64),
) -> Result<(crate::Store, crate::OpeningSteps), MigrationError> {
    remove_backups(db).map_err(MigrationError::Backup)?;
    let backup = backup(db, from).map_err(MigrationError::Backup)?;
    // Rust note: the store that failed to open is dropped inside
    // `open_migrating`, with its connection, before `restore` runs: no
    // file of the database is open any more.
    crate::Store::open_migrating(db, on_version)
        .map_err(|error| MigrationError::Migration { error, restored: backup.restore() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backups_are_named_after_the_file_and_the_version() {
        let db = Path::new("/d/nicemailer.db");
        assert_eq!(backup_name(db, 9), Path::new("/d/nicemailer.db.bak-v9"));
        assert_eq!(backup_name(Path::new("/d/nicemailer.db-wal"), 9), Path::new("/d/nicemailer.db.bak-v9-wal"));
        assert_eq!(backup_name(Path::new("/d/nicemailer_body.db"), 12), Path::new("/d/nicemailer_body.db.bak-v12"));
    }

    #[test]
    fn a_backup_puts_back_every_file_and_removes_those_added_since() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("nicemailer.db");
        let body = crate::body_path(&db);
        std::fs::write(&db, "main v9").unwrap();
        std::fs::write(with_suffix(&db, "-wal"), "log v9").unwrap();
        std::fs::write(&body, "bodies v9").unwrap();
        let b = backup(&db, 9).unwrap();
        assert_eq!(b.copies().count(), 3, "the body file has no log: nothing to copy for it");
        // The migration changes the files, adds a log and an index.
        std::fs::write(&db, "main v12").unwrap();
        std::fs::write(&body, "bodies v12").unwrap();
        std::fs::write(with_suffix(&body, "-wal"), "log v12").unwrap();
        std::fs::write(with_suffix(&db, "-shm"), "index").unwrap();
        b.restore().unwrap();
        assert_eq!(std::fs::read_to_string(&db).unwrap(), "main v9");
        assert_eq!(std::fs::read_to_string(with_suffix(&db, "-wal")).unwrap(), "log v9");
        assert_eq!(std::fs::read_to_string(&body).unwrap(), "bodies v9");
        assert!(!with_suffix(&body, "-wal").exists());
        assert!(!with_suffix(&db, "-shm").exists());
        assert!(!dir.path().join("nicemailer.db.bak-v9").exists(), "the copies became the database");
    }

    #[test]
    fn only_our_backups_are_removed() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("nicemailer.db");
        for name in [
            "nicemailer.db",
            "nicemailer.db.bak-v9",
            "nicemailer.db.bak-v9-wal",
            "nicemailer_body.db.bak-v9",
            "nicemailer.db.bak-v9-mine",
            "nicemailer.db.bak-v",
            "other.db.bak-v9",
        ] {
            std::fs::write(dir.path().join(name), "x").unwrap();
        }
        assert_eq!(remove_backups(&db).unwrap(), 3);
        let mut left: Vec<String> =
            std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, ["nicemailer.db", "nicemailer.db.bak-v", "nicemailer.db.bak-v9-mine", "other.db.bak-v9"]);
    }
}
