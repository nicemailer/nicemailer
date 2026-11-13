//! Pool of read-only connections to the database.
//!
//! The core has a single *writing* `Store`, behind a mutex: SQLite allows one
//! writer at a time anyway, and a single writer keeps the order of the
//! events equal to the order of the commits. Until phase 1 of the IMAP plan
//! (`specifications/plan-imap.md`) every query went through it too, so a long
//! write (a batch of 500 mails of a synchronization, a migration of the
//! data) made the app wait for its list of threads.
//!
//! In WAL mode (`schema.sql`, `journal_mode = WAL`), readers do not block
//! the writer and the writer does not block readers: each reader sees the
//! database as it was when its read transaction began. So the queries of the
//! contract (`list_threads`, `get_thread`, `search`…) run on connections of
//! this pool, and only commands take the writing store.
//!
//! # Consistency
//!
//! Each call of [`ReadPool::read`] runs inside one read transaction
//! (`BEGIN` … `COMMIT`). Without it, every statement of a query would see
//! the latest commit, and a query made of two statements (a page of threads,
//! then their tags) could mix two states of the database. Inside the
//! transaction, every statement sees the same snapshot.
//!
//! The `as_of` position returned with a view stays correct: the core reads
//! it *before* the query, and every event up to that position was emitted
//! after its transaction committed (`nm-daemon`, `command`). The snapshot,
//! taken later, therefore contains at least everything up to `as_of`. It may
//! contain a later change too; the client then applies its event again,
//! which is harmless.

use std::path::Path;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::{Result, Store};

/// A fixed set of read-only stores, lent one at a time.
///
/// Rust note: `Condvar` (condition variable) lets a thread sleep until
/// another one signals a change. Here a reader that finds the pool empty
/// waits on `returned` until a store comes back, instead of spinning.
pub struct ReadPool {
    idle: Mutex<Vec<Store>>,
    returned: Condvar,
}

impl ReadPool {
    /// Opens `size` read-only connections to the database at `path`. The
    /// database must already exist at the current schema version: the
    /// writing store, opened first, creates or migrates it.
    pub fn open(path: &Path, size: usize) -> Result<ReadPool> {
        let idle = (0..size.max(1)).map(|_| Store::open_reader(path)).collect::<Result<Vec<_>>>()?;
        Ok(ReadPool { idle: Mutex::new(idle), returned: Condvar::new() })
    }

    /// Runs `f` on one of the stores, inside a read transaction, and gives
    /// the store back. Blocks while every store is lent.
    pub fn read<T>(&self, f: impl FnOnce(&Store) -> Result<T>) -> Result<T> {
        self.read_timed(f).0
    }

    /// [`ReadPool::read`], and how long it waited for a free store. A
    /// request that is slow because every connection was lent (the app
    /// sent many at once) is told apart in `core.log` from one whose query
    /// is slow: the cures differ.
    pub fn read_timed<T>(&self, f: impl FnOnce(&Store) -> Result<T>) -> (Result<T>, Duration) {
        let asked = Instant::now();
        let store = {
            let mut idle = self.idle.lock().unwrap();
            // Rust note: `wait` releases the lock while sleeping and takes it
            // again when woken. The loop guards against spurious wakeups.
            loop {
                if let Some(s) = idle.pop() {
                    break s;
                }
                idle = self.returned.wait(idle).unwrap();
            }
        };
        let waited = asked.elapsed();
        (self.run(store, f), waited)
    }

    /// Runs `f` on `store` in a read transaction, then gives it back.
    fn run<T>(&self, store: Store, f: impl FnOnce(&Store) -> Result<T>) -> Result<T> {
        let lent = Lent { pool: self, store: Some(store) };
        let conn = lent.store().conn();
        conn.execute_batch("BEGIN")?;
        let result = f(lent.store());
        // A read transaction has nothing to undo: `COMMIT` and `ROLLBACK`
        // both just end it. `ROLLBACK` is used after an error because the
        // error may have ended the transaction already, and `COMMIT` would
        // then fail with a second, misleading error.
        if result.is_ok() {
            conn.execute_batch("COMMIT")?;
        } else {
            let _ = conn.execute_batch("ROLLBACK");
        }
        result
    }
}

/// A store borrowed from the pool, returned when dropped.
struct Lent<'a> {
    pool: &'a ReadPool,
    /// Always `Some` until `drop` takes it.
    store: Option<Store>,
}

impl Lent<'_> {
    fn store(&self) -> &Store {
        self.store.as_ref().expect("store present until drop")
    }
}

/// Rust note: implementing `Drop` runs code when a value goes out of scope,
/// whatever the path: normal return, `?`, or a panic unwinding the stack.
impl Drop for Lent<'_> {
    fn drop(&mut self) {
        if let Some(store) = self.store.take() {
            // A panic inside `f` may have left a transaction open: end it,
            // so the next user of this store does not inherit an old
            // snapshot. `is_autocommit` is true outside a transaction.
            if !store.conn().is_autocommit() {
                let _ = store.conn().execute_batch("ROLLBACK");
            }
            // `unwrap_or_else(|e| e.into_inner())`: if another thread
            // panicked while holding this lock, take it anyway. The `Vec` of
            // idle stores is still valid, and refusing it would lose the
            // store.
            self.pool.idle.lock().unwrap_or_else(|e| e.into_inner()).push(store);
            self.pool.returned.notify_one();
        }
    }
}
