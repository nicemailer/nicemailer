//! The maintenance mode of the core, and the update of NiceMailer
//! (`specifications/mise-a-jour.md`; `docs/updates.md`).
//!
//! **While an update is installed or the database is migrated, the core
//! serves no client.** It keeps listening on its socket, but answers every
//! request with the error `MAINTENANCE`, whose `data` says why
//! (`nm_api::Maintenance`), then closes the connection. A client that
//! simply found no socket would start a core (`launchctl kickstart`),
//! possibly from a bundle half replaced; told "maintenance", it waits.
//!
//! One exception, during an update: the app. It drives the update, and
//! must be able to cancel it, from the connection that prepared it or
//! from a new one (an app relaunched after a crash). So its connections
//! stay open and its `hello` is answered; every other request of it gets
//! the maintenance's error (`subscribe` included, which is how a new
//! connection of the app learns of it), except `prepare_update` and
//! `cancel_update`. A migration has no such exception: the database is not
//! open, there is nothing to serve, and nothing to cancel.
//!
//! Four reasons:
//! - `migration`: the database is being upgraded, at start (`main.rs`,
//!   `open_store`), with its progress;
//! - `newer_schema`: the database was written by a newer NiceMailer; the
//!   core never touches it, and stays in maintenance until it is replaced;
//! - `migration_failed`: the migration failed and its backup was put back
//!   (`nm_store::upgrade`); the core stays in maintenance until restarted;
//! - `update`: the app is about to install an update ([`prepare`]).
//!
//! The update, step by step (`mise-a-jour.md`, section 3):
//! 1. Sparkle found an update, and the user accepted it (or it installs
//!    itself at quit).
//! 2. The app calls `prepare_update` ([`prepare`]): the core enters
//!    maintenance, stops its IMAP workers (the actions of the spool stay
//!    queued), waits for the write in progress, and **stays alive**: a core
//!    that stopped would be started again from the old bundle by the next
//!    client.
//! 3. Sparkle replaces the bundle. The core notices its own program was
//!    replaced ([`watch_program`]) and leaves with [`RESTART_CODE`]:
//!    launchd starts the new one (`KeepAlive` restarts an exit that is not
//!    0). The app, relaunched, also asks launchd to restart a core whose
//!    version is not its own (`launchctl kickstart -k`).
//! 4. The new core migrates the database if needed (in maintenance,
//!    `migration`), then serves its clients, whose subscriptions get
//!    `ResyncRequired` since the epoch of the events changed.
//!
//! An update abandoned (a failed download, the user said no, the app
//! crashed): `cancel_update` ([`cancel`]), or after
//! `core.update_timeout_secs` the core leaves the maintenance by itself
//! ([`prepare`]'s timer).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use nm_api::{CancelUpdateResult, Maintenance, MaintenanceReason, PrepareUpdateResult, Request, Response, RpcError};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::watch;

use crate::Core;

/// The exit code of a core whose program was replaced during an update:
/// not 0, so that launchd starts it again, now from the new bundle
/// (`KeepAlive` = `SuccessfulExit: false`). 75 is `EX_TEMPFAIL` of
/// `sysexits.h`, "try again later".
pub const RESTART_CODE: i32 = 75;

/// How long a client in maintenance has to send its first line. A client
/// that connects and says nothing does not keep a task forever.
const FIRST_LINE: Duration = Duration::from_secs(5);

/// The state of the maintenance, shared by the connections.
///
/// Rust note: a tokio `watch` channel holds one value; every receiver can
/// read the latest one and wait for it to change. The connections wait on
/// it to close themselves when a maintenance begins (`main.rs`, `serve`).
pub type State = watch::Sender<Option<Maintenance>>;

/// A new state, outside any maintenance.
pub fn state() -> State {
    watch::channel(None).0
}

/// Seconds since 1970, for `Maintenance::since`.
pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// A maintenance of `reason` beginning now.
pub fn begin(reason: MaintenanceReason) -> Maintenance {
    Maintenance { reason, from: None, to: None, at: None, since: now(), message: None }
}

/// The error of the maintenance under way, if any: what every request
/// gets meanwhile (`main.rs`, `call`).
pub fn refusal(state: &State) -> Option<RpcError> {
    state.borrow().as_ref().map(Maintenance::error)
}

/// A connection accepted while the database is not open (a migration,
/// `newer_schema`, `migration_failed`; `main.rs`, `open_store`): its first
/// request is answered with the error `MAINTENANCE` (with its id, so that the client
/// matches it with its `hello`), then the connection is closed. The
/// maintenance read is the one at the moment of the answer, so a migration
/// shows its latest progress.
pub async fn serve_refusing(stream: UnixStream, state: watch::Receiver<Option<Maintenance>>) {
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    let id = match tokio::time::timeout(FIRST_LINE, lines.next_line()).await {
        Ok(Ok(Some(line))) => serde_json::from_str::<Request>(&line).ok().and_then(|r| r.id).unwrap_or(Value::Null),
        // Nothing sent in time, or the client left: the error alone.
        _ => Value::Null,
    };
    let Some(error) = state.borrow().as_ref().map(Maintenance::error) else { return };
    let line = serde_json::to_string(&Response::err(id, error)).expect("plain struct");
    let _ = write.write_all(line.as_bytes()).await;
    let _ = write.write_all(b"\n").await;
    let _ = write.shutdown().await;
}

/// `prepare_update` (module documentation, step 2). Called again while an
/// update is prepared, it starts its timer again: the app may call it
/// before each attempt.
///
/// Refused during another maintenance (a migration, a database the core
/// cannot open): `call` answers those with their own error before this
/// runs.
pub async fn prepare(core: &Arc<Core>) -> Result<PrepareUpdateResult, RpcError> {
    let timeout_secs = core.engine.lock().unwrap().config().core.update_timeout_secs;
    // Rust note: `send_replace` sets the value and wakes every receiver;
    // the connections open now close themselves (`serve`), after the
    // answers of the requests they already read, this one among them.
    core.maintenance.send_replace(Some(begin(MaintenanceReason::Update)));
    let generation = core.update_generation.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    eprintln!("mise à jour : préparation, le noyau ne sert plus aucun client (au plus {timeout_secs} s)");
    // The workers stop: a batch in progress ends, the actions of the spool
    // stay queued, and the IMAP connections close. Then the store's lock is
    // taken once: a write that was running has ended. Both block, on a
    // thread of the blocking pool.
    let c = core.clone();
    tokio::task::spawn_blocking(move || {
        if let Some(syncer) = c.syncer.get() {
            syncer.stop_all(Duration::from_secs(5));
        }
        drop(c.lock_store("update"));
    })
    .await
    .map_err(|e| RpcError::new(nm_api::error_code::INTERNAL_ERROR, e.to_string()))?;
    // An update that never comes: the maintenance ends by itself.
    let c = core.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(timeout_secs)).await;
        if c.update_generation.load(std::sync::atomic::Ordering::SeqCst) == generation && leave(&c).await {
            eprintln!("mise à jour : aucune nouvelle version après {timeout_secs} s, le noyau sert de nouveau ses clients");
        }
    });
    // The program replaced: the update is installed.
    if let Some(program) = program() {
        tokio::spawn(watch_program(core.clone(), program, generation));
    }
    Ok(PrepareUpdateResult { timeout_secs })
}

/// `cancel_update`: the update was abandoned; the core serves its clients
/// again and starts its workers.
pub async fn cancel(core: &Arc<Core>) -> Result<CancelUpdateResult, RpcError> {
    // A timer or a watch still running belongs to an update no longer
    // under way: a new generation makes them stop.
    core.update_generation.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let cancelled = leave(core).await;
    if cancelled {
        eprintln!("mise à jour : annulée, le noyau sert de nouveau ses clients");
    }
    Ok(CancelUpdateResult { cancelled })
}

/// Leaves the maintenance of an update, and starts the workers again.
/// Returns false, changing nothing, if no update was being prepared.
async fn leave(core: &Arc<Core>) -> bool {
    // Rust note: `send_if_modified` changes the value under the channel's
    // lock and wakes the receivers only when the closure returns true:
    // the test and the change are one step, never split by another caller.
    let left = core.maintenance.send_if_modified(|m| {
        let preparing = matches!(m, Some(Maintenance { reason: MaintenanceReason::Update, .. }));
        if preparing {
            *m = None;
        }
        preparing
    });
    if left {
        let c = core.clone();
        let _ = tokio::task::spawn_blocking(move || {
            let mut store = c.lock_store("update");
            crate::sync::apply(&c, &mut store);
            if let Some(syncer) = c.syncer.get() {
                syncer.kick();
            }
        })
        .await;
    }
    left
}

/// The program whose replacement means the update is installed: the core's
/// own executable, or for the tests the file named by
/// `NICEMAILER_TEST_UPDATE_PROGRAM` (replacing the binary under test would
/// break the other tests).
fn program() -> Option<PathBuf> {
    match std::env::var_os("NICEMAILER_TEST_UPDATE_PROGRAM") {
        Some(p) => Some(PathBuf::from(p)),
        None => std::env::current_exe().ok(),
    }
}

/// The identity of a file: replaced, it has another inode (Sparkle moves
/// the new bundle into place, it does not write into the old files).
fn identity(path: &std::path::Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::metadata(path).ok()?;
    Some((m.dev(), m.ino()))
}

/// Watches the core's program while the update `generation` is prepared.
/// Once it is replaced, the core closes cleanly and leaves with
/// [`RESTART_CODE`], for launchd to start the new program (module
/// documentation, step 3). A program missing for a moment (between the
/// removal of the old bundle and the move of the new one) is waited for.
async fn watch_program(core: Arc<Core>, program: PathBuf, generation: u64) {
    let Some(before) = identity(&program) else { return };
    loop {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if core.update_generation.load(std::sync::atomic::Ordering::SeqCst) != generation {
            return;
        }
        match identity(&program) {
            Some(now) if now != before => break,
            _ => continue,
        }
    }
    eprintln!("mise à jour : {} remplacé, le noyau s'arrête pour que launchd lance la nouvelle version", program.display());
    let c = core.clone();
    let _ = tokio::task::spawn_blocking(move || {
        let store = c.lock_store("update");
        if let Err(e) = store.close_cleanly() {
            eprintln!("clean close: not marked ({e}); the next start reconciles");
        }
        // Rust note: `exit` ends the process at once, without running the
        // destructors: the store's lock is held until then, so nothing is
        // written after the clean close.
        std::process::exit(RESTART_CODE);
    })
    .await;
}
