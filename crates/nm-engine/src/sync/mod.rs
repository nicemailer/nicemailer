//! The synchronization of the IMAP accounts, server to database
//! (`plan-imap.md`, phase 4; `modele de donnees.md`, section 5).
//!
//! # Who does what
//!
//! - [`Syncer`] (this file) starts one *worker* per IMAP account of
//!   `config.toml`, restarts it when the account's settings change, stops
//!   it when the account goes away.
//! - A worker has two connections to the server, each on a thread of its
//!   own (`account.rs`):
//!   - the **work connection** synchronizes the folders in passes
//!     (`pass.rs`), then waits on the Inbox with IDLE: new mail is seen at
//!     once, the other folders are checked every `sync.poll_interval_secs`;
//!   - the **body connection** downloads the bodies (`bodies.rs`), Inbox
//!     first, newest first, so that 200,000 bodies to download never delay
//!     the arrival of a mail. A body the user opens goes first.
//!
//!   - the **sender** (`send.rs`), for an account with `[smtp]`, sends
//!     the mails queued by `send_draft` and copies them to Sent, with
//!     connections of its own (`reponse.md`).
//!
//!   With `sync.max_connections = 1`, both jobs share the work connection.
//!   The plan had a third connection for IDLE alone: the work connection
//!   leaves IDLE in a few milliseconds when it has something to do, so a
//!   third would only cost one of the few connections servers allow.
//! - What the workers find goes to the database through `nm-store`
//!   (`sync.rs`), whose functions talk to no server; the daemon gives the
//!   workers the store and emits the events ([`SyncHost`]).
//!
//! # Read-only towards the server
//!
//! Until the spool (phase 5), NiceMailer sends no change to the server:
//! every folder is opened with `EXAMINE`, the read-only `SELECT`, and every
//! fetch is a `PEEK`. The synchronization cannot mark a mail read, move or
//! delete anything, even by mistake. The commands that change mail are
//! refused on these accounts (`nm-store`, `refuse_imap_thread`).
//!
//! # Threads
//!
//! Each connection runs on an OS thread with its own single-threaded tokio
//! runtime: the network is asynchronous (IDLE waits for the server and a
//! stop signal at once), and the database calls are plain blocking calls,
//! short (a batch of 100 headers), without blocking the daemon's runtime.

mod account;
mod bodies;
mod pass;
mod send;
mod spool;
pub mod verify;

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use nm_api::{AccountSync, Event};
use nm_store::raw::RawStore;
use nm_store::{ChangeSet, Store, StoreError};
use tokio::sync::Notify;

use crate::config::{AccountConfig, SyncSettings};
use crate::secrets::Secrets;

/// What the workers need from the daemon: the store, and the events.
///
/// Rust note: a *trait object* (`Arc<dyn SyncHost>`) hides the daemon's own
/// types from the engine: `nm-engine` does not depend on `nm-daemon`, the
/// daemon implements this trait for its shared state.
pub trait SyncHost: Send + Sync + 'static {
    /// Runs `f` on the writing store, and emits the events of the
    /// `ChangeSet` it returns while the store's lock is still held, so that
    /// their order is the order of the writes.
    fn write(&self, f: &mut dyn FnMut(&mut Store) -> Result<ChangeSet, StoreError>) -> Result<(), StoreError>;
    /// Runs `f` on a reading connection.
    fn read(&self, f: &mut dyn FnMut(&Store) -> Result<(), StoreError>) -> Result<(), StoreError>;
    /// A sequenced event that does not come from a write (`AccountError`,
    /// `AccountChanged`).
    fn event(&self, event: Event);
    /// A transient event (`SyncProgress`): not numbered, not replayed.
    fn transient(&self, event: Event);
    fn raw(&self) -> &RawStore;
}

/// `host.write` returning a value: `f` gives the value and the changes.
///
/// Rust note: the closure given to `write` is `FnMut`, called once here;
/// the value it produces is moved out through an `Option` the closure
/// borrows.
pub(crate) fn write<T>(
    host: &dyn SyncHost,
    f: impl FnOnce(&mut Store) -> Result<(T, ChangeSet), StoreError>,
) -> Result<T, StoreError> {
    let mut f = Some(f);
    let mut out = None;
    host.write(&mut |s| {
        let f = f.take().expect("called once");
        let (v, changes) = f(s)?;
        out = Some(v);
        Ok(changes)
    })?;
    Ok(out.expect("write ran"))
}

/// `host.read` returning a value.
pub(crate) fn read<T>(host: &dyn SyncHost, f: impl FnOnce(&Store) -> Result<T, StoreError>) -> Result<T, StoreError> {
    let mut f = Some(f);
    let mut out = None;
    host.read(&mut |s| {
        out = Some((f.take().expect("called once"))(s)?);
        Ok(())
    })?;
    Ok(out.expect("read ran"))
}

/// One IMAP account to synchronize.
#[derive(Debug, Clone, PartialEq)]
pub struct AccountSpec {
    /// Its key in `config.toml`.
    pub key: String,
    pub account_id: i64,
    pub config: AccountConfig,
}

impl AccountSpec {
    fn settings(&self) -> &SyncSettings {
        &self.config.sync
    }
}

/// Signals between the daemon and the threads of one worker.
#[derive(Default)]
pub(crate) struct Control {
    /// The worker must end (account removed, changed, core stopping).
    stop: AtomicBool,
    /// A pass is asked for now (`sync_now`).
    now: AtomicBool,
    /// Something changed that may unblock a worker waiting without retry
    /// (a password saved after an authentication failure).
    kick: AtomicBool,
    /// Wakes the work connection: stop, now, kick.
    work: Notify,
    /// Wakes the body connection: stop, new messages, a body asked for.
    bodies: Notify,
    /// Wakes the sender: stop, a mail queued or cancelled (`send.rs`).
    send: Notify,
    /// The body queue must be read again: a pass brought new messages.
    reload: AtomicBool,
    /// A command queued actions in the spool: send them now.
    spool: AtomicBool,
    /// Bodies the user is waiting for (a message opened), first served.
    priority: Mutex<VecDeque<i64>>,
    /// Bodies refused this run (size other than announced): not tried
    /// again until the core restarts.
    refused: Mutex<HashSet<i64>>,
}

impl Control {
    pub(crate) fn stopped(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        // `notify_one` keeps a permit when nobody waits yet: the signal is
        // never lost between a check and the wait that follows it.
        self.work.notify_one();
        self.bodies.notify_one();
        self.send.notify_one();
    }

    /// Takes the "now" request, if any.
    pub(crate) fn take_now(&self) -> bool {
        self.now.swap(false, Ordering::SeqCst)
    }

    pub(crate) fn take_kick(&self) -> bool {
        self.kick.swap(false, Ordering::SeqCst)
    }
}

/// A running worker.
struct Worker {
    spec: AccountSpec,
    control: Arc<Control>,
    status: Arc<Mutex<AccountSync>>,
    threads: Vec<JoinHandle<()>>,
}

/// The workers of every IMAP account.
pub struct Syncer {
    host: Arc<dyn SyncHost>,
    secrets: Arc<dyn Secrets>,
    workers: Mutex<BTreeMap<String, Worker>>,
}

impl Syncer {
    pub fn new(host: Arc<dyn SyncHost>, secrets: Arc<dyn Secrets>) -> Syncer {
        Syncer { host, secrets, workers: Mutex::new(BTreeMap::new()) }
    }

    /// Brings the workers in line with the accounts of the file: starts
    /// those of new accounts, restarts those whose settings changed (a new
    /// server, a new user), stops those of accounts gone. Quick: it does
    /// not wait for a stopped worker to end; the new worker of a changed
    /// account waits for it instead, so that two workers never write the
    /// same account at once.
    pub fn apply(&self, accounts: Vec<AccountSpec>) {
        let mut workers = self.workers.lock().unwrap();
        let keys: Vec<String> = workers.keys().cloned().collect();
        let mut previous: BTreeMap<String, Vec<JoinHandle<()>>> = BTreeMap::new();
        for key in keys {
            let keep = accounts.iter().any(|a| a.key == key && workers[&key].spec == *a);
            if !keep {
                let w = workers.remove(&key).expect("listed");
                w.control.stop();
                previous.insert(key, w.threads);
            }
        }
        for spec in accounts {
            if workers.contains_key(&spec.key) {
                continue;
            }
            let wait = previous.remove(&spec.key).unwrap_or_default();
            let w = account::start(spec.clone(), self.host.clone(), self.secrets.clone(), wait);
            workers.insert(spec.key.clone(), w);
        }
    }

    /// A password was saved, or the file changed: workers stopped by an
    /// authentication failure or a missing password try again.
    pub fn kick(&self) {
        for w in self.workers.lock().unwrap().values() {
            w.control.kick.store(true, Ordering::SeqCst);
            w.control.work.notify_one();
            w.control.bodies.notify_one();
        }
    }

    /// `sync_now`: a pass now, for one account or every one. `false`: no
    /// such account is synchronized.
    pub fn sync_now(&self, key: Option<&str>) -> bool {
        let workers = self.workers.lock().unwrap();
        let mut found = false;
        for (k, w) in workers.iter() {
            if key.is_none_or(|key| key == k) {
                w.control.now.store(true, Ordering::SeqCst);
                w.control.kick.store(true, Ordering::SeqCst);
                w.control.work.notify_one();
                found = true;
            }
        }
        found
    }

    /// Commands queued actions for these accounts: their workers send them
    /// now, leaving IDLE (`spool.rs`).
    pub fn wake(&self, account_ids: &std::collections::BTreeSet<i64>) {
        if account_ids.is_empty() {
            return;
        }
        for w in self.workers.lock().unwrap().values() {
            if account_ids.contains(&w.spec.account_id) {
                w.control.spool.store(true, Ordering::SeqCst);
                w.control.work.notify_one();
            }
        }
    }

    /// Mails were queued for sending, or a send cancelled, on these
    /// accounts: their senders look at their queue again (`send.rs`).
    pub fn wake_send(&self, account_ids: &std::collections::BTreeSet<i64>) {
        if account_ids.is_empty() {
            return;
        }
        for w in self.workers.lock().unwrap().values() {
            if account_ids.contains(&w.spec.account_id) {
                w.control.send.notify_one();
            }
        }
    }

    /// Bodies were reset to be downloaded again (a raw file damaged): the
    /// body connections of these accounts read their queue again.
    pub fn refetch(&self, account_ids: &std::collections::BTreeSet<i64>) {
        for w in self.workers.lock().unwrap().values() {
            if account_ids.contains(&w.spec.account_id) {
                w.control.reload.store(true, Ordering::SeqCst);
                w.control.bodies.notify_one();
                if w.spec.settings().max_connections < 2 {
                    w.control.work.notify_one();
                }
            }
        }
    }

    /// The bodies of `message_ids` of `account_id` are wanted now (the user
    /// opened them): first in line on the body connection.
    pub fn prioritize(&self, account_id: i64, message_ids: &[i64]) {
        if message_ids.is_empty() {
            return;
        }
        for w in self.workers.lock().unwrap().values() {
            if w.spec.account_id == account_id {
                let mut q = w.control.priority.lock().unwrap();
                for id in message_ids {
                    if !q.contains(id) {
                        q.push_back(*id);
                    }
                }
                drop(q);
                w.control.bodies.notify_one();
                // With a single connection, the work connection fetches
                // bodies: wake it from IDLE.
                if w.spec.settings().max_connections < 2 {
                    w.control.work.notify_one();
                }
            }
        }
    }

    /// The state of each worker, by key; the daemon adds the counters of
    /// the database (`sync_status`).
    pub fn status(&self) -> Vec<AccountSync> {
        self.workers.lock().unwrap().values().map(|w| w.status.lock().unwrap().clone()).collect()
    }

    /// The account of a key, if synchronized.
    pub fn spec(&self, key: &str) -> Option<AccountSpec> {
        self.workers.lock().unwrap().get(key).map(|w| w.spec.clone())
    }

    /// Stops the worker of `key` and waits for its threads, at most `wait`:
    /// before its account's data is deleted, nothing may still write it.
    pub fn stop_and_join(&self, key: &str, wait: std::time::Duration) {
        let Some(w) = self.workers.lock().unwrap().remove(key) else { return };
        w.control.stop();
        let deadline = std::time::Instant::now() + wait;
        for t in w.threads {
            while !t.is_finished() && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            if t.is_finished() {
                let _ = t.join();
            }
        }
    }

    /// Stops every worker and waits for them, at most `wait` (core
    /// stopping). A transaction in progress ends; the rest stops at the
    /// next point where a worker checks its signal.
    pub fn stop_all(&self, wait: std::time::Duration) {
        let workers: Vec<Worker> = std::mem::take(&mut *self.workers.lock().unwrap()).into_values().collect();
        for w in &workers {
            w.control.stop();
        }
        let deadline = std::time::Instant::now() + wait;
        for w in workers {
            for t in w.threads {
                while !t.is_finished() && std::time::Instant::now() < deadline {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                if t.is_finished() {
                    let _ = t.join();
                }
            }
        }
    }
}

/// Current time, Unix seconds.
pub(crate) fn unix_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}
