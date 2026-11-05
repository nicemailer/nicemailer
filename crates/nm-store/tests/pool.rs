//! The read pool: queries do not wait for a long write, see a consistent
//! snapshot, and cannot write (`pool.rs`).

use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

use nm_store::Store;
use nm_store::pool::ReadPool;
use nm_store::seed::{SeedOptions, seed};

/// A seeded database in a file (a pool cannot share an in-memory one).
fn seeded() -> (tempfile::TempDir, std::path::PathBuf, Store) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nicemailer.db");
    let mut store = Store::open(&path).unwrap();
    seed(&mut store, &SeedOptions { threads: 100, seed: 1, now: 1_790_000_000 }).unwrap();
    (dir, path, store)
}

#[test]
fn a_read_does_not_wait_for_a_long_write() {
    let (_dir, path, store) = seeded();
    let pool = Arc::new(ReadPool::open(&path, 2).unwrap());
    let before = pool.read(|s| s.list_folders()).unwrap();

    // The writer takes the write lock and holds it for 2 s, with an
    // uncommitted change.
    let started = Arc::new(Barrier::new(2));
    let writer = {
        let started = started.clone();
        std::thread::spawn(move || {
            let conn = store.conn();
            conn.execute_batch("BEGIN IMMEDIATE; UPDATE folder SET total_count = total_count + 1000;").unwrap();
            started.wait();
            std::thread::sleep(Duration::from_secs(2));
            conn.execute_batch("COMMIT").unwrap();
        })
    };
    started.wait();
    let t = Instant::now();
    let during = pool.read(|s| s.list_folders()).unwrap();
    assert!(t.elapsed() < Duration::from_millis(500), "the read waited {:?}", t.elapsed());
    // It sees the last committed state, not the write in progress.
    assert_eq!(during.iter().map(|f| f.total_count).collect::<Vec<_>>(), before.iter().map(|f| f.total_count).collect::<Vec<_>>());
    writer.join().unwrap();
    let after = pool.read(|s| s.list_folders()).unwrap();
    assert_eq!(after[0].total_count, before[0].total_count + 1000, "the commit is seen by the next read");
}

#[test]
fn a_read_sees_one_snapshot() {
    let (_dir, path, store) = seeded();
    let pool = ReadPool::open(&path, 1).unwrap();
    let count = |s: &Store| -> i64 { s.conn().query_row("SELECT count(*) FROM folder", [], |r| r.get(0)).unwrap() };
    let (first, second) = pool
        .read(|s| {
            let first = count(s);
            // A write committed in the middle of the read...
            store
                .conn()
                .execute("INSERT INTO folder (account_id, path, created_by, modified_by) SELECT id, 'New', 'core', 'core' FROM account LIMIT 1", [])
                .unwrap();
            // ...is not seen by the rest of it.
            Ok((first, count(s)))
        })
        .unwrap();
    assert_eq!(first, second);
    assert_eq!(pool.read(|s| Ok(count(s))).unwrap(), first + 1);
}

#[test]
fn a_reader_cannot_write() {
    let (_dir, path, _store) = seeded();
    let pool = ReadPool::open(&path, 1).unwrap();
    let err = pool.read(|s| Ok(s.conn().execute("DELETE FROM folder", [])?)).unwrap_err();
    assert!(err.to_string().contains("readonly") || err.to_string().contains("read"), "{err}");
    // And the pool still works after the error.
    assert!(!pool.read(|s| s.list_folders()).unwrap().is_empty());
}

#[test]
fn a_panic_in_a_read_gives_the_connection_back() {
    let (_dir, path, _store) = seeded();
    let pool = Arc::new(ReadPool::open(&path, 1).unwrap());
    let p = pool.clone();
    let r = std::thread::spawn(move || p.read(|_| -> nm_store::Result<()> { panic!("bug in a query") })).join();
    assert!(r.is_err());
    // With a pool of one, this would wait forever if the store was lost.
    assert!(!pool.read(|s| s.list_folders()).unwrap().is_empty());
}

#[test]
fn readers_share_the_load() {
    let (_dir, path, _store) = seeded();
    let pool = Arc::new(ReadPool::open(&path, 3).unwrap());
    // Six threads, three connections: every read completes.
    let handles: Vec<_> = (0..6)
        .map(|_| {
            let p = pool.clone();
            std::thread::spawn(move || {
                for _ in 0..20 {
                    p.read(|s| s.list_folders()).unwrap();
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
}

#[test]
fn a_reader_refuses_a_database_at_another_version() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("x.db");
    nm_store::rusqlite::Connection::open(&path).unwrap().execute_batch("PRAGMA user_version = 1;").unwrap();
    assert!(matches!(ReadPool::open(&path, 1), Err(nm_store::StoreError::ReaderVersion { found: 1, .. })));
}
