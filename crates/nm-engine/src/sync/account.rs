//! The worker of one account: its connections, passes, waits and failures
//! (`plan-imap.md`, phase 4, "Rythme" and "Pannes").
//!
//! # The work connection
//!
//! ```text
//! connect ─▶ pass ─▶ IDLE on the Inbox ─▶ pass ─▶ IDLE … ─▶ (error) ─▶ wait ─▶ connect
//! ```
//!
//! A pass is followed by IDLE on the Inbox, which ends:
//! - when the server reports something (new mail, a flag changed): a pass
//!   at once, of the folders whose counters changed;
//! - at `sync.poll_interval_secs` since the last full pass: a full pass
//!   (every folder checked; every flag compared on a server without
//!   CONDSTORE);
//! - on `sync_now`, on a stop, or (one connection only) on a body the user
//!   is waiting for;
//! - when the Mac wakes from sleep: the connection is surely dead, it is
//!   dropped without waiting for its time limit, and a new one is opened at
//!   once. A sleep shows as a jump of the wall clock, which goes on during
//!   sleep, against the monotonic clock, which macOS stops.
//!
//! A server without IDLE is polled every minute instead.
//!
//! # Failures
//!
//! - Connection lost, server unreachable, time limit: new attempt after a
//!   delay doubled at each failure, 5 s to 5 min, reset by a pass that
//!   succeeds.
//! - Login refused: no new attempt until the password is saved again or the
//!   file changes (`Syncer::kick`, `apply`): a client that retries a wrong
//!   password gets the account locked by the server. `AccountError` is
//!   emitted.
//! - No password, or a password saved for another server: the same, without
//!   trying to connect.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use nm_api::{AccountSync, Event};
use nm_core::Source;
use nm_proto::imap::{IDLE_MAX, IdleEnd, ImapError, MailboxName, Namespace, Session, Timeouts};
use nm_store::insert::Inserter;

use super::bodies::Bodies;
use super::pass::{self, Ctx, Mode, PassError};
use super::spool;
use super::{AccountSpec, Control, SyncHost, Worker, unix_now};
use crate::probe::imap_spec;
use crate::secrets::{self, Refusal, Secrets};

/// First delay after a failure; doubled at each one, up to [`BACKOFF_MAX`].
const BACKOFF_MIN: Duration = Duration::from_secs(5);
const BACKOFF_MAX: Duration = Duration::from_secs(300);
/// Polling of a server without IDLE.
const NO_IDLE_POLL: Duration = Duration::from_secs(60);
/// After IDLE reports a change, the pass waits this long: another client
/// that moves a mail without `MOVE` copies it, marks it `\Deleted`, then
/// expunges it, in three commands. A pass started at the first would see
/// a new mail in the target and the original still there, and could not
/// tell the move from a copy (`nm-store`, `sync.rs`, "moves made
/// elsewhere"). The commands of a client follow each other in
/// milliseconds; the delay also gathers a burst of new mail into one pass.
const SETTLE: Duration = Duration::from_millis(1500);
/// One connection only: bodies are downloaded for this long between two
/// passes, so that new mail is still seen within a minute.
const BODY_SLICE: Duration = Duration::from_secs(30);
/// The body connection, when idle, checks it is still alive this often
/// (servers log out after 30 minutes of silence, RFC 3501, 5.4).
const BODY_KEEPALIVE: Duration = Duration::from_secs(20 * 60);

/// What the state of the workers is shared through.
struct Shared {
    spec: AccountSpec,
    host: Arc<dyn SyncHost>,
    secrets: Arc<dyn Secrets>,
    control: Arc<Control>,
    status: Arc<Mutex<AccountSync>>,
    /// The work thread has waited for the previous worker of the account:
    /// the body thread may start.
    ready: Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>,
}

/// Starts the worker of `spec`, once the threads of `wait` (the previous
/// worker of the same account) have ended.
pub(super) fn start(spec: AccountSpec, host: Arc<dyn SyncHost>, secrets: Arc<dyn Secrets>, wait: Vec<JoinHandle<()>>) -> Worker {
    let control = Arc::new(Control::default());
    let status = Arc::new(Mutex::new(AccountSync {
        account_id: spec.account_id,
        key: spec.key.clone(),
        state: "connecting".into(),
        ..AccountSync::default()
    }));
    let shared = Arc::new(Shared {
        spec: spec.clone(),
        host,
        secrets,
        control: control.clone(),
        status: status.clone(),
        ready: Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new())),
    });
    let mut threads = Vec::new();
    let work = shared.clone();
    threads.push(
        std::thread::Builder::new()
            .name(format!("nm-sync-{}", spec.key))
            .spawn(move || {
                for t in wait {
                    let _ = t.join();
                }
                {
                    let (lock, cv) = &*work.ready;
                    *lock.lock().unwrap() = true;
                    cv.notify_all();
                }
                runtime().block_on(work_loop(&work));
                set(&work.status, "stopped", None);
            })
            .expect("thread"),
    );
    if spec.config.sync.max_connections >= 2 {
        let bodies = shared.clone();
        threads.push(
            std::thread::Builder::new()
                .name(format!("nm-bodies-{}", spec.key))
                .spawn(move || {
                    {
                        let (lock, cv) = &*bodies.ready;
                        let mut ready = lock.lock().unwrap();
                        while !*ready {
                            ready = cv.wait(ready).unwrap();
                        }
                    }
                    runtime().block_on(body_loop(&bodies));
                })
                .expect("thread"),
        );
    }
    Worker { spec, control, status, threads }
}

/// A single-threaded tokio runtime for one connection.
///
/// Rust note: `enable_all` turns on the timers and the network, which a
/// runtime built by hand does not have by default.
fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio runtime")
}

fn set(status: &Mutex<AccountSync>, state: &str, error: Option<String>) {
    let mut s = status.lock().unwrap();
    s.state = state.into();
    if state != "waiting" {
        s.next_retry = None;
    }
    s.error = error;
}

/// Why no session could be opened.
enum NoSession {
    /// Try again after a delay.
    Retry(String),
    /// Do not try again until kicked (password, file).
    Blocked { state: &'static str, error: String },
    Stopped,
}

/// Opens a session: reads the password (checked against the server the
/// file names), connects, logs in, turns CONDSTORE and QRESYNC on.
async fn connect(w: &Shared) -> Result<(Session, Option<Namespace>), NoSession> {
    if w.control.stopped() {
        return Err(NoSession::Stopped);
    }
    let cfg = &w.spec.config;
    let password = match secrets::imap_password(w.secrets.as_ref(), &w.spec.key, cfg) {
        Ok(p) => p,
        Err(r @ Refusal::Unavailable(_)) => return Err(NoSession::Retry(r.message(&w.spec.key, cfg))),
        Err(r) => return Err(NoSession::Blocked { state: "stopped", error: r.message(&w.spec.key, cfg) }),
    };
    let mut s = Session::connect(&imap_spec(cfg), Timeouts::default()).await.map_err(|e| NoSession::Retry(e.to_string()))?;
    match s.login(&cfg.imap.username, password.expose()).await {
        Ok(()) => {}
        Err(e @ ImapError::AuthFailed(_)) => return Err(NoSession::Blocked { state: "auth_failed", error: e.to_string() }),
        Err(e) => return Err(NoSession::Retry(e.to_string())),
    }
    // Optional: a server without them is followed by full comparisons.
    let _ = s.id().await;
    if let Err(e) = s.enable_sync().await
        && e.is_fatal()
    {
        return Err(NoSession::Retry(e.to_string()));
    }
    let namespace = match s.namespace().await {
        Ok(ns) => ns.personal.into_iter().next(),
        Err(e) if e.is_fatal() => return Err(NoSession::Retry(e.to_string())),
        Err(_) => None,
    };
    Ok((s, namespace))
}

/// The delay before the next attempt.
struct Backoff(Duration);

impl Backoff {
    fn next(&mut self) -> Duration {
        let d = self.0;
        self.0 = (self.0 * 2).min(BACKOFF_MAX);
        // Up to a fifth more, at random: several accounts on the same
        // server do not retry all at the same second.
        d + Duration::from_millis(rand::random::<u64>() % (d.as_millis() as u64 / 5 + 1))
    }

    fn reset(&mut self) {
        self.0 = BACKOFF_MIN;
    }
}

/// Waits `d`, or less if the worker is woken (stop, now, kick).
async fn pause(control: &Control, d: Duration) {
    tokio::select! {
        _ = tokio::time::sleep(d) => {}
        _ = control.work.notified() => {}
    }
}

/// Waits until kicked or stopped.
async fn blocked(control: &Control) {
    while !control.stopped() && !control.take_kick() {
        control.work.notified().await;
    }
}

/// Returns when the Mac seems to have slept: the wall clock jumped ahead of
/// the monotonic clock by more than 30 s.
async fn slept() {
    loop {
        let (wall, mono) = (SystemTime::now(), Instant::now());
        tokio::time::sleep(Duration::from_secs(15)).await;
        let walked = wall.elapsed().unwrap_or_default();
        if walked > mono.elapsed() + Duration::from_secs(30) {
            return;
        }
    }
}

/// The account's error, announced once: `AccountError` when it appears or
/// changes, `AccountChanged` when it goes away.
struct Announced(Option<String>);

impl Announced {
    fn error(&mut self, w: &Shared, error: &str) {
        if self.0.as_deref() != Some(error) {
            self.0 = Some(error.to_string());
            w.host.event(Event::AccountError { account_id: w.spec.account_id, error: error.to_string() });
        }
    }

    fn clear(&mut self, w: &Shared) {
        if self.0.take().is_some() {
            w.host.event(Event::AccountChanged { account_id: w.spec.account_id });
        }
    }
}

/// How a wait ended.
enum Woken {
    /// The server reported something on the Inbox.
    Changes,
    /// The time asked for passed.
    Timeout,
    /// `sync_now`, a kick, a stop, or a body asked for.
    Signal,
    /// The Mac slept: the connection is dropped.
    Slept,
}

async fn work_loop(w: &Shared) {
    let single = w.spec.config.sync.max_connections < 2;
    let poll = Duration::from_secs(w.spec.config.sync.poll_interval_secs);
    let mut inserter = Inserter::new(Source::Imap, None);
    let mut bodies = Bodies::new();
    let mut backoff = Backoff(BACKOFF_MIN);
    let mut announced = Announced(None);
    let mut last_full: Option<Instant> = None;
    let mut refused = 0u64;
    // Whether each folder keeps keywords, learnt by the spool's SELECTs.
    let mut keywords: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
    'connect: loop {
        if w.control.stopped() {
            return;
        }
        // Rust note: the guard of `lock()` lives until the end of the
        // statement that creates it; taken inside `set`'s arguments, it
        // would still be held when `set` locks again, and the thread would
        // wait for itself forever. Hence the `let` first.
        let error = w.status.lock().unwrap().error.clone();
        set(&w.status, "connecting", error);
        let (mut s, namespace) = match connect(w).await {
            Ok(s) => s,
            Err(NoSession::Stopped) => return,
            Err(NoSession::Blocked { state, error }) => {
                // A missing password, or one saved for another server, is
                // already announced by the engine (`list_accounts`,
                // `AccountError`): only a refused login is news.
                if state == "auth_failed" {
                    announced.error(w, &error);
                }
                set(&w.status, state, Some(error));
                blocked(&w.control).await;
                continue;
            }
            Err(NoSession::Retry(error)) => {
                announced.error(w, &error);
                retry_later(w, &mut backoff, error).await;
                continue;
            }
        };
        let mode = Mode::of(&s);
        w.status.lock().unwrap().mode = Some(mode.as_str().into());
        bodies.new_session();
        // The first pass of a connection is full: what happened while
        // disconnected may be anywhere.
        let mut full = true;
        loop {
            set(&w.status, "syncing", None);
            // What NiceMailer changed first, then what the server changed:
            // the pass then sees the moves done, and attaches their UIDs.
            w.control.spool.store(false, Ordering::SeqCst);
            match spool::run(&mut s, w.host.as_ref(), w.spec.account_id, &mut keywords).await {
                Ok(r) => {
                    if r.done + r.reverted + r.retried > 0 {
                        eprintln!("sync {}: spool: {} done, {} undone, {} to retry", w.spec.key, r.done, r.reverted, r.retried);
                    }
                }
                Err(PassError::Stopped) => return,
                Err(e) => {
                    let error = e.to_string();
                    announced.error(w, &error);
                    retry_later(w, &mut backoff, error).await;
                    continue 'connect;
                }
            }
            let mut ctx = Ctx {
                host: w.host.as_ref(),
                account_id: w.spec.account_id,
                control: &w.control,
                inserter: &mut inserter,
                namespace: namespace.clone(),
                keywords: &keywords,
            };
            match pass::run(&mut s, &mut ctx, full).await {
                Ok(report) => {
                    backoff.reset();
                    announced.clear(w);
                    if full {
                        last_full = Some(Instant::now());
                    }
                    {
                        let mut st = w.status.lock().unwrap();
                        st.last_pass = Some(unix_now());
                        st.passes += 1;
                        st.error = None;
                    }
                    for p in &report.problems {
                        eprintln!("sync {}: {p}", w.spec.key);
                    }
                    // The folders gone from the server, for `sync_status`
                    // (the daemon adds the counters of the database).
                    w.status.lock().unwrap().folders = report
                        .missing
                        .iter()
                        .map(|p| nm_api::FolderSync { path: p.clone(), missing_on_server: true, ..Default::default() })
                        .collect();
                    if !report.missing.is_empty() {
                        eprintln!("sync {}: folders no longer on the server, kept: {}", w.spec.key, report.missing.join(", "));
                    }
                    if report.inserted + report.moved + report.deleted > 0 {
                        eprintln!(
                            "sync {}: {} new, {} moved elsewhere, {} deleted",
                            w.spec.key, report.inserted, report.moved, report.deleted
                        );
                    }
                    // The body queue has new messages, maybe before its rest.
                    w.control.reload.store(true, Ordering::SeqCst);
                    w.control.bodies.notify_one();
                }
                Err(PassError::Stopped) => return,
                Err(e) => {
                    let error = e.to_string();
                    announced.error(w, &error);
                    retry_later(w, &mut backoff, error).await;
                    continue 'connect;
                }
            }
            if single {
                set(&w.status, "syncing", None);
                let deadline = Some(Instant::now() + BODY_SLICE);
                match bodies.run(&mut s, w.host.as_ref(), w.spec.account_id, &w.control, &mut refused, deadline).await {
                    Ok(more) => {
                        w.status.lock().unwrap().bodies_refused = refused;
                        if more {
                            // Bodies left: a quick pass first, for new mail.
                            full = due(last_full, poll) || w.control.take_now();
                            continue;
                        }
                    }
                    Err(PassError::Stopped) => return,
                    Err(e) => {
                        let error = e.to_string();
                        announced.error(w, &error);
                        retry_later(w, &mut backoff, error).await;
                        continue 'connect;
                    }
                }
            }
            set(&w.status, "idle", None);
            let until_full = last_full.map_or(Duration::ZERO, |t| poll.saturating_sub(t.elapsed()));
            let woken = match wait(&mut s, &w.control, until_full).await {
                Ok(woken) => woken,
                Err(e) => {
                    let error = e.to_string();
                    announced.error(w, &error);
                    retry_later(w, &mut backoff, error).await;
                    continue 'connect;
                }
            };
            if w.control.stopped() {
                s.logout().await;
                return;
            }
            full = match woken {
                Woken::Slept => {
                    eprintln!("sync {}: the Mac slept, new connection", w.spec.key);
                    continue 'connect;
                }
                Woken::Changes => {
                    pause(&w.control, SETTLE).await;
                    w.control.take_now()
                }
                Woken::Timeout => due(last_full, poll) || w.control.take_now(),
                Woken::Signal => {
                    w.control.take_kick();
                    // Woken for the spool alone: a quick pass after it.
                    w.control.take_now() || (due(last_full, poll) && !w.control.spool.load(Ordering::SeqCst))
                }
            };
        }
    }
}

/// Whether a full pass is due.
fn due(last_full: Option<Instant>, poll: Duration) -> bool {
    last_full.is_none_or(|t| t.elapsed() >= poll)
}

/// Records the error and waits before the next attempt.
async fn retry_later(w: &Shared, backoff: &mut Backoff, error: String) {
    let d = backoff.next();
    eprintln!("sync {}: {error}; new attempt in {} s", w.spec.key, d.as_secs());
    {
        let mut st = w.status.lock().unwrap();
        st.state = "waiting".into();
        st.error = Some(error);
        st.next_retry = Some(unix_now() + d.as_secs() as i64);
    }
    pause(&w.control, d).await;
    // A kick or `sync_now` during the wait: try at once, but the next
    // failure waits longer, as if the wait had run out.
    w.control.take_kick();
}

/// The wait between two passes: IDLE on the Inbox, or a plain timer for a
/// server without IDLE.
async fn wait(s: &mut Session, control: &Control, until_full: Duration) -> Result<Woken, ImapError> {
    if !s.capabilities().has("IDLE") {
        let d = until_full.min(NO_IDLE_POLL).max(Duration::from_secs(1));
        return Ok(tokio::select! {
            _ = tokio::time::sleep(d) => Woken::Timeout,
            _ = control.work.notified() => Woken::Signal,
            _ = slept() => Woken::Slept,
        });
    }
    // IDLE watches the selected mailbox: the Inbox, read-only.
    s.examine(&MailboxName::inbox(), None).await?;
    let max = until_full.clamp(Duration::from_secs(1), IDLE_MAX);
    let end = tokio::select! {
        r = s.idle(max, control.work.notified()) => Some(r?),
        _ = slept() => None,
    };
    s.take_unsolicited();
    Ok(match end {
        None => Woken::Slept,
        Some(IdleEnd::Changes) => Woken::Changes,
        Some(IdleEnd::Timeout) => Woken::Timeout,
        Some(IdleEnd::Stopped) => Woken::Signal,
    })
}

/// The body connection (`sync.max_connections = 2`).
async fn body_loop(w: &Shared) {
    let mut bodies = Bodies::new();
    let mut backoff = Backoff(BACKOFF_MIN);
    let mut refused = 0u64;
    'connect: loop {
        // Not before the work connection has logged in and made its first
        // pass: there is no body to fetch before, and a wrong password is
        // tried once, not twice.
        let state = w.status.lock().unwrap().state.clone();
        if matches!(state.as_str(), "auth_failed" | "stopped" | "connecting" | "waiting") {
            blocked_bodies(w).await;
            if w.control.stopped() {
                return;
            }
        }
        let mut s = match connect(w).await {
            Ok((s, _)) => s,
            Err(NoSession::Stopped) => return,
            // The work connection reports the account's errors; this one
            // only waits.
            Err(NoSession::Blocked { .. }) => {
                blocked_bodies(w).await;
                continue;
            }
            Err(NoSession::Retry(_)) => {
                pause_bodies(&w.control, backoff.next()).await;
                continue;
            }
        };
        bodies.new_session();
        loop {
            match bodies.run(&mut s, w.host.as_ref(), w.spec.account_id, &w.control, &mut refused, None).await {
                Ok(_) => {
                    backoff.reset();
                    w.status.lock().unwrap().bodies_refused = refused;
                    // None left: wait for new messages, a body asked for,
                    // or the time to check the connection is alive.
                    let alive = tokio::select! {
                        _ = w.control.bodies.notified() => true,
                        _ = tokio::time::sleep(BODY_KEEPALIVE) => s.noop().await.is_ok(),
                        _ = slept() => false,
                    };
                    if w.control.stopped() {
                        s.logout().await;
                        return;
                    }
                    if !alive {
                        continue 'connect;
                    }
                }
                Err(PassError::Stopped) => return,
                Err(e) => {
                    eprintln!("sync {} (bodies): {e}", w.spec.key);
                    pause_bodies(&w.control, backoff.next()).await;
                    continue 'connect;
                }
            }
        }
    }
}

async fn pause_bodies(control: &Control, d: Duration) {
    tokio::select! {
        _ = tokio::time::sleep(d) => {}
        _ = control.bodies.notified() => {}
    }
}

/// The body connection waits while the work connection is blocked (login
/// refused, no password) or waiting to try again (server unreachable): it
/// would only fail too, and a server that sees a wrong password again and
/// again may lock the account. The work connection alone tries again.
async fn blocked_bodies(w: &Shared) {
    loop {
        tokio::select! {
            _ = w.control.bodies.notified() => {}
            _ = tokio::time::sleep(Duration::from_secs(5)) => {}
        }
        let state = w.status.lock().unwrap().state.clone();
        if w.control.stopped() || !matches!(state.as_str(), "auth_failed" | "stopped" | "connecting" | "waiting") {
            return;
        }
    }
}
