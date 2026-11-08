//! What a database file is, without changing it: new, current, to be
//! upgraded, or unusable (`docs/migrations.md`, "Choosing a mailbox").
//!
//! The app asks before it switches to another mailbox ("Change Mailbox…",
//! through `nicemailer mailbox check`): a database of an earlier schema is
//! upgraded only once the user agreed, and one the core would refuse is
//! refused at once, the current mailbox kept. Without this, choosing the
//! wrong folder restarted the core on a database it refuses, and the app
//! was left with no core at all.
//!
//! The answer comes from the same rules as `Store::open` (`lib.rs`,
//! `init`): the version in `PRAGMA user_version`, then `check_schema` for a
//! version that is not the latest. Only reads are made: the file is opened
//! without the right to create it, and the connection is `query_only`.

use std::path::Path;

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

use crate::{SCHEMA_VERSION, StoreError, check_schema};

/// What a database file is.
///
/// Rust note: `#[serde(tag = "state")]` writes each variant as a JSON
/// object whose field `state` names it, its fields beside it:
/// `{"state": "upgradable", "from": 2, "to": 4}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum DatabaseState {
    /// No database yet (no file, or an empty one): a new mailbox, created
    /// at the core's start.
    New,
    /// At the schema of this code: opened as it is.
    Current,
    /// An earlier schema that the migrations bring up to date. Upgrading
    /// cannot be undone: an older NiceMailer refuses the result.
    Upgradable { from: i64, to: i64 },
    /// An early development database the migrations cannot handle (a
    /// table or column they assume is missing): it cannot be upgraded.
    Outdated { missing: String },
    /// Written by a newer NiceMailer: refused, so that this one does not
    /// damage what it does not know.
    TooNew { found: i64, expected: i64 },
    /// Not a database this code can read (another kind of file, damaged,
    /// no permission).
    Unreadable { message: String },
}

/// Inspects the database at `path`. Never fails: a problem is a state.
pub fn inspect(path: &Path) -> DatabaseState {
    match std::fs::metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return DatabaseState::New,
        Err(e) => return DatabaseState::Unreadable { message: e.to_string() },
        Ok(m) if m.is_dir() => return DatabaseState::Unreadable { message: "a folder, not a database file".into() },
        // SQLite treats an empty file as an empty database: the core fills it.
        Ok(m) if m.len() == 0 => return DatabaseState::New,
        Ok(_) => {}
    }
    match read(path) {
        Ok(state) => state,
        Err(e) => DatabaseState::Unreadable { message: e.to_string() },
    }
}

fn read(path: &Path) -> Result<DatabaseState, rusqlite::Error> {
    // Read-write without `CREATE`, then `query_only`: the same choice as the
    // readers of the core (`Store::open_reader`), which a read-only file
    // handle would not allow on a database in WAL mode whose `-shm` file is
    // missing. Nothing is written either way.
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    conn.execute_batch("PRAGMA query_only = ON; PRAGMA busy_timeout = 2000;")?;
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    Ok(classify(&conn, version)?)
}

fn classify(conn: &Connection, version: i64) -> Result<DatabaseState, rusqlite::Error> {
    if version == 0 {
        // Version 0 is "never initialized": empty, or not ours. A file with
        // tables but no version is not something `Store::open` can take
        // (it would try to create the schema over them).
        let tables: i64 = conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get(0))?;
        return Ok(if tables == 0 {
            DatabaseState::New
        } else {
            DatabaseState::Outdated { missing: "schema version (not a NiceMailer database?)".into() }
        });
    }
    if version > SCHEMA_VERSION {
        return Ok(DatabaseState::TooNew { found: version, expected: SCHEMA_VERSION });
    }
    match check_schema(conn, version) {
        Ok(()) if version == SCHEMA_VERSION => Ok(DatabaseState::Current),
        Ok(()) => Ok(DatabaseState::Upgradable { from: version, to: SCHEMA_VERSION }),
        Err(StoreError::OutdatedSchema(what)) => Ok(DatabaseState::Outdated { missing: what.to_string() }),
        Err(StoreError::Sqlite(e)) => Err(e),
        Err(other) => Ok(DatabaseState::Unreadable { message: other.to_string() }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Store;

    const V1: &str = include_str!("../tests/fixtures/schema_v1.sql");

    fn db(dir: &Path, sql: &str) -> std::path::PathBuf {
        let path = dir.join("nicemailer.db");
        Connection::open(&path).unwrap().execute_batch(sql).unwrap();
        path
    }

    #[test]
    fn every_state_is_told_apart() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("none.db");
        assert_eq!(inspect(&missing), DatabaseState::New);
        std::fs::write(dir.path().join("empty.db"), "").unwrap();
        assert_eq!(inspect(&dir.path().join("empty.db")), DatabaseState::New);

        let current = dir.path().join("current.db");
        drop(Store::open(&current).unwrap());
        assert_eq!(inspect(&current), DatabaseState::Current);

        let old = tempfile::tempdir().unwrap();
        let path = db(old.path(), V1);
        assert_eq!(inspect(&path), DatabaseState::Upgradable { from: 1, to: SCHEMA_VERSION });

        let early = tempfile::tempdir().unwrap();
        let path = db(early.path(), "CREATE TABLE account (id INTEGER PRIMARY KEY); PRAGMA user_version = 1;");
        assert!(matches!(inspect(&path), DatabaseState::Outdated { missing } if missing == "account.kind"));

        let foreign = tempfile::tempdir().unwrap();
        let path = db(foreign.path(), "CREATE TABLE notes (t TEXT);");
        assert!(matches!(inspect(&path), DatabaseState::Outdated { .. }));

        let newer = tempfile::tempdir().unwrap();
        let path = db(newer.path(), "PRAGMA user_version = 99;");
        assert_eq!(inspect(&path), DatabaseState::TooNew { found: 99, expected: SCHEMA_VERSION });

        std::fs::write(dir.path().join("text.db"), "not a database, only some text that is long enough").unwrap();
        assert!(matches!(inspect(&dir.path().join("text.db")), DatabaseState::Unreadable { .. }));
        assert!(matches!(inspect(dir.path()), DatabaseState::Unreadable { .. }));
    }

    #[test]
    fn inspecting_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = db(dir.path(), V1);
        let before = std::fs::read(&path).unwrap();
        assert!(matches!(inspect(&path), DatabaseState::Upgradable { .. }));
        assert_eq!(std::fs::read(&path).unwrap(), before, "the file is the same, byte for byte");
        // Still at version 1: nothing was migrated.
        let v: i64 = Connection::open(&path).unwrap().query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(v, 1);
    }

    #[test]
    fn the_upgrade_the_inspection_announces_is_the_one_open_does() {
        let dir = tempfile::tempdir().unwrap();
        let path = db(dir.path(), V1);
        assert!(matches!(inspect(&path), DatabaseState::Upgradable { from: 1, .. }));
        drop(Store::open(&path).unwrap());
        assert_eq!(inspect(&path), DatabaseState::Current);
    }

    #[test]
    fn the_json_names_the_state() {
        let j = serde_json::to_value(DatabaseState::Upgradable { from: 2, to: 4 }).unwrap();
        assert_eq!(j, serde_json::json!({"state": "upgradable", "from": 2, "to": 4}));
        let j = serde_json::to_value(DatabaseState::TooNew { found: 9, expected: 4 }).unwrap();
        assert_eq!(j, serde_json::json!({"state": "too_new", "found": 9, "expected": 4}));
        assert_eq!(serde_json::to_value(DatabaseState::New).unwrap(), serde_json::json!({"state": "new"}));
    }
}
