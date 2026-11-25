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
//! - when the Mac wakes from sleep (`clock.rs`).
//!
//! A server without IDLE is polled every minute instead.
//!
//! # Sleep of the Mac
//!
//! A Mac left alone sleeps and wakes every few minutes, often for a few
//! seconds only. Each wake once cost two logins (both connections dropped
//! and opened again), and five wakes spent the budget of the hour
//! (21 November 2026). Now, at a wake:
//!
//! 1. the connection is **tested** before anything else: IDLE is ended
//!    (`DONE`), or a `NOOP` sent, with [`PROBE`] for the server to answer.
//!    A connection that answers is kept, and a full pass follows on it:
//!    no login at all;
//! 2. a connection that does not answer is dropped, and the new login
//!    waits for the Mac to have been awake `pace::WAKE_SETTLE` (15 s), so
//!    that a dark wake of a few seconds spends none;
//! 3. only the work connection logs in again. The body connection, if it
//!    died too, waits until the work connection leaves bodies after one of
//!    its slices ([`Control::backlog`](super::Control)): a few new mails
//!    need no second login.
//!
//! # Pace (`pace.rs`)
//!
//! - No limit of logins until the server shows a block (refused "for
//!   now", a password it accepted refused, turned away before the login
//!   three times in a row): the account then pauses, and logs in at most
//!   half as often as it did in the last hour, both connections together.
//!   What is learnt is saved in `account.login_*`, and read back when the
//!   worker starts. A body connection that waits (or failed, or has not
//!   logged in yet) leaves the bodies to the work connection, in slices of
//!   `BODY_SLICE` after each pass: new mail never waits for its body as
//!   long as the work connection is up. The two never fetch at the same
//!   time (`Shared::body_turn`), so no body is downloaded twice.
//! - A server that cuts a connection left in IDLE: IDLE is renewed at half
//!   the time it allowed, and the account keeps one connection only (the
//!   bodies are fetched on it). If the server cuts it even so, the account
//!   is polled: a connection per pass, at most every 10 minutes, closed
//!   after it. Such a cut is not announced as an error: the mail goes on
//!   arriving.
//!
//! # Failures
//!
//! - Connection lost, server unreachable, time limit: new attempt after a
//!   delay doubled at each failure, 5 s to 30 min, reset only by a
//!   connection that lived `pace::STABLE` (10 min). A pass that succeeds
//!   is not enough: a server that cut every connection a minute after its
//!   pass was otherwise logged into once a minute.
//! - Login refused: no new attempt until the password is saved again or the
//!   file changes (`Syncer::kick`, `apply`): a client that retries a wrong
//!   password gets the account locked by the server. `AccountError` is
//!   emitted.
//! - No password, or a password saved for another server: the same, without
//!   trying to connect.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use nm_api::{AccountSync, Event};
use nm_core::Source;
use nm_proto::imap::{IDLE_MAX, IdleEnd, ImapError, MailboxName, Namespace, Session, Timeouts};
use nm_store::insert::Inserter;

use super::bodies::Bodies;
use super::clock;
use super::pace::{self, IdleHold, Learnt, LoginPace, Signal};
use super::pass::{self, Ctx, Mode, PassError};
use super::spool;
use super::{AccountSpec, Control, SyncHost, Worker, read, unix_now, write};
use crate::config::AuthMethod;
use crate::probe::imap_spec;
use crate::secrets::{self, Refusal, Secrets};

/// First delay after a failure; doubled at each one, up to [`BACKOFF_MAX`].
const BACKOFF_MIN: Duration = Duration::from_secs(5);
const BACKOFF_MAX: Duration = Duration::from_secs(30 * 60);
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
/// After a wake, how long a connection has to answer (`DONE` of IDLE, or
/// `NOOP`) to be kept. A connection the server closed during the sleep
/// usually fails at once (the server's TCP reset); one lost with the
/// network (another Wi-Fi) never answers, and costs this wait.
const PROBE: Duration = Duration::from_secs(10);

/// What the state of the workers is shared through.
struct Shared {
    spec: AccountSpec,
    host: Arc<dyn SyncHost>,
    secrets: Arc<dyn Secrets>,
    /// The OAuth2 state: access tokens and renewals (`oauth2.rs`).
    oauth: Arc<crate::oauth2::Oauth>,
    control: Arc<Control>,
    status: Arc<Mutex<AccountSync>>,
    /// The work thread has waited for the previous worker of the account:
    /// the body thread may start.
    ready: Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>,
    /// The logins of the last hour, both connections together, and what
    /// the server's blocks taught (`pace.rs`).
    pace: Mutex<LoginPace>,
    /// Held while bodies are fetched, by either connection: the work
    /// connection takes over while the body connection has no session
    /// (`Control::work_fetches_bodies`), and that one may log in again in
    /// the middle of a slice. Each reads its queue again from the database
    /// once it holds the turn, so neither downloads a body the other has
    /// just completed.
    ///
    /// Rust note: `tokio::sync::Mutex`, not `std::sync::Mutex`, since it
    /// is held across `.await` (a whole download); the two connections run
    /// on runtimes of their own, which this mutex does not mind.
    body_turn: tokio::sync::Mutex<()>,
}

/// Starts the worker of `spec`, once the threads of `wait` (the previous
/// worker of the same account) have ended.
pub(super) fn start(
    spec: AccountSpec,
    host: Arc<dyn SyncHost>,
    secrets: Arc<dyn Secrets>,
    oauth: Arc<crate::oauth2::Oauth>,
    wait: Vec<JoinHandle<()>>,
) -> Worker {
    let control = Arc::new(Control::default());
    // What the server taught before: a pause it imposed still runs after a
    // restart of the core, and its limit is not learnt again at the cost
    // of another block.
    let row = read(host.as_ref(), |st| st.account_login_pace(spec.account_id)).unwrap_or_default();
    let pace = LoginPace::restore(row.limit, row.blocked_at, row.pause_until, unix_now(), clock::now());
    let status = Arc::new(Mutex::new(AccountSync {
        account_id: spec.account_id,
        key: spec.key.clone(),
        read_only: spec.config.read_only,
        state: "connecting".into(),
        login_limit: pace.limit().map(|l| l as u32),
        ..AccountSync::default()
    }));
    let shared = Arc::new(Shared {
        spec: spec.clone(),
        host,
        secrets,
        oauth,
        control: control.clone(),
        status: status.clone(),
        ready: Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new())),
        pace: Mutex::new(pace),
        body_turn: tokio::sync::Mutex::new(()),
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
    // The sender, for an account that can send (`send.rs`): never on a
    // read-only account, whose server must not change.
    if spec.config.smtp.is_some() && !spec.config.read_only {
        let sender = shared.clone();
        threads.push(
            std::thread::Builder::new()
                .name(format!("nm-send-{}", spec.key))
                .spawn(move || {
                    // Not before the previous worker of the account has
                    // ended, like the body thread: two senders of one
                    // account would claim the same queue.
                    {
                        let (lock, cv) = &*sender.ready;
                        let mut ready = lock.lock().unwrap();
                        while !*ready {
                            ready = cv.wait(ready).unwrap();
                        }
                    }
                    let sx = super::send::Sender {
                        spec: &sender.spec,
                        host: sender.host.as_ref(),
                        secrets: sender.secrets.as_ref(),
                        oauth: sender.oauth.as_ref(),
                        control: &sender.control,
                    };
                    runtime().block_on(super::send::run(sx));
                })
                .expect("thread"),
        );
    }
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
    /// Do not try again until kicked (password, file). `why`: the reason
    /// to announce and to give in `list_accounts` when only a person can
    /// unblock the account (an OAuth2 refresh token refused); `None` when
    /// the engine already reports it, or for a refused login, which the
    /// worker announces from `state`.
    Blocked { state: &'static str, error: String, why: Option<nm_api::Why> },
    Stopped,
}

/// Which connection of the worker asks for a session.
#[derive(Clone, Copy, PartialEq)]
enum Conn {
    Work,
    Bodies,
}

/// Opens a session: reads the password (checked against the server the
/// file names), connects, logs in, turns CONDSTORE and QRESYNC on.
async fn connect(w: &Shared, conn: Conn) -> Result<(Session, Option<Namespace>), NoSession> {
    if w.control.stopped() {
        return Err(NoSession::Stopped);
    }
    let cfg = &w.spec.config;
    // The pace of the logins (`pace.rs`): checked before anything reaches
    // the server, and not shortened by `sync_now`.
    //
    // Each connection waits on its own signal. The body connection once
    // waited on the work connection's: `notify_one` wakes a single waiter,
    // so it could take a stop or a `sync_now` meant for the work
    // connection. It does not touch the account's state either: the work
    // connection may be up and syncing meanwhile, and the bodies go on
    // through it.
    let wake = if conn == Conn::Work { &w.control.work } else { &w.control.bodies };
    let who = if conn == Conn::Work { "" } else { " (bodies)" };
    let (mut told_pace, mut told_wake) = (false, false);
    // Both rules are checked again after each wait: a wait for the pace
    // may end at a wake (its clock counts the sleep), and a wait after a
    // wake may be cut by another sleep.
    loop {
        // A new password: the pause after a refusal of the old one ends.
        if w.control.password_saved.swap(false, Ordering::SeqCst) {
            w.pace.lock().unwrap().clear_pause();
        }
        // A Mac that has just woken may sleep again within seconds (a dark
        // wake): no login before it has been awake `WAKE_SETTLE`.
        let settle = pace::after_wake(clock::since_wake());
        let paced = w.pace.lock().unwrap().wait(clock::now());
        let d = settle.max(paced);
        if d.is_zero() {
            break;
        }
        // Once per wait: a body connection is woken by every new mail.
        if !paced.is_zero() && !told_pace {
            told_pace = true;
            let limit = w.pace.lock().unwrap().limit().map_or(String::new(), |l| format!(", at most {l} an hour"));
            eprintln!("sync {}{who}: logins held back by the server's limit{limit}: next one in {} s", w.spec.key, paced.as_secs());
        } else if paced.is_zero() && !told_wake {
            told_wake = true;
            eprintln!("sync {}{who}: the Mac has just woken, login in {} s", w.spec.key, settle.as_secs().max(1));
        }
        if conn == Conn::Work {
            let mut st = w.status.lock().unwrap();
            st.state = "waiting".into();
            st.next_retry = Some(unix_now() + d.as_secs() as i64);
        }
        tokio::select! {
            _ = clock::sleep(d) => {}
            _ = wake.notified() => {}
        }
        if w.control.stopped() {
            return Err(NoSession::Stopped);
        }
    }
    // The credential of the login: the password, or an OAuth2 access token
    // renewed on demand (`specifications/oauth2.md`, section 6).
    let (password, oauth2) = match cfg.auth {
        AuthMethod::Password => match secrets::imap_password(w.secrets.as_ref(), &w.spec.key, cfg) {
            Ok(p) => (p, false),
            Err(r @ Refusal::Unavailable(_)) => return Err(NoSession::Retry(r.message(&w.spec.key, cfg))),
            Err(r) => return Err(NoSession::Blocked { state: "stopped", error: r.message(&w.spec.key, cfg), why: None }),
        },
        AuthMethod::Oauth2 => match w.oauth.access_token(&w.spec.key, cfg, w.secrets.as_ref()).await {
            Ok(t) => (t, true),
            Err(e) if e.is_temporary() => return Err(NoSession::Retry(e.why().message())),
            // A refresh token refused (`invalid_grant`, `interaction_required`)
            // or another explicit refusal of the provider (admin consent,
            // conditional access): retrying cannot help, and the Keychain
            // item exists, so the engine's static check does not see it.
            // Its reason goes to the clients, which offer "Sign in again"
            // for `oauth_reauth_needed`.
            Err(e) => {
                let why = e.why();
                return Err(NoSession::Blocked { state: "stopped", error: why.message(), why: Some(why) });
            }
        },
    };
    // Counted before connecting: an attempt that fails is seen by the
    // server all the same.
    w.pace.lock().unwrap().record(clock::now());
    let mut s = Session::connect(&imap_spec(cfg), Timeouts::default()).await.map_err(|e| refused(w, conn, e, false))?;
    // A read-only account: the session itself refuses any writing command,
    // whatever the rest of NiceMailer asks.
    if cfg.read_only {
        s.forbid_writes();
    }
    let login = if oauth2 { s.login_oauth2(&cfg.imap.username, &password).await } else { s.login(&cfg.imap.username, password.expose()).await };
    let mut fresh = false;
    let login = match login {
        Ok(()) => Ok(()),
        Err(e) => {
            // A token the server refuses though the core believed it valid
            // (revoked, conditional access): renew once, retry once.
            if oauth2 && matches!(e, ImapError::AuthFailed(_)) {
                match w.oauth.access_token_forced(&w.spec.key, cfg, w.secrets.as_ref()).await {
                    Ok(t) => {
                        fresh = true;
                        s.login_oauth2(&cfg.imap.username, &t).await
                    }
                    // The renewal itself failed. Temporary (network, 5xx):
                    // try again later, the account is not in error. Else
                    // the refresh token is refused: a sign-in is needed,
                    // said as such rather than as a refused login.
                    Err(t) if t.is_temporary() => return Err(NoSession::Retry(t.why().message())),
                    Err(t) => {
                        let why = t.why();
                        return Err(NoSession::Blocked { state: "stopped", error: why.message(), why: Some(why) });
                    }
                }
            } else {
                Err(e)
            }
        }
    };
    if let Err(e) = login {
        // IMAP disabled for the mailbox is concluded only after the refusal
        // of a token that was just renewed (`specifications/oauth2.md`,
        // section 11): Exchange does not distinguish it from a refused token.
        let disabled = fresh && matches!(e, ImapError::AuthFailed(_));
        return Err(refused(w, conn, e, disabled));
    }
    w.pace.lock().unwrap().logged_in();
    // This password works: refused later, it is more likely a lock than a
    // wrong password (`pace::classify`). Not an error if it cannot be
    // written: the refusal would then be taken for a wrong password, as
    // before.
    let id = w.spec.account_id;
    let _ = write(w.host.as_ref(), |st| st.set_account_login_ok(id, Some(unix_now())).map(|()| ((), Default::default())));
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

/// A connection or a login that failed: what the worker does next, and
/// what the server taught, if it showed a block (`pace.rs`).
fn refused(w: &Shared, conn: Conn, e: ImapError, disabled: bool) -> NoSession {
    let id = w.spec.account_id;
    // Read at the refusal, not at the start: a password saved meanwhile
    // has not worked yet (`login_password_changed`).
    // Not for an OAuth2 account: a token refused after a renewal is
    // revoked, or IMAP is disabled for the mailbox (`oauth2.md`, section
    // 11), never a lock for too many logins, which a password that worked
    // before suggests.
    let worked = matches!(e, ImapError::AuthFailed(_))
        && w.spec.config.auth != AuthMethod::Oauth2
        && read(w.host.as_ref(), |st| st.account_login_pace(id)).is_ok_and(|row| row.ok_at.is_some());
    let signal = pace::classify(&e, worked);
    let learnt = signal.and_then(|signal| w.pace.lock().unwrap().blocked(clock::now(), signal).map(|l| (signal, l)));
    let Some((signal, Learnt { limit, pause })) = learnt else {
        return match e {
            // A wrong password: never tried again until a new one is saved,
            // or the server could lock the account.
            ImapError::AuthFailed(_) if signal != Some(Signal::Locked) => NoSession::Blocked { state: if disabled { "auth_disabled" } else { "auth_failed" }, error: e.to_string(), why: None },
            _ => NoSession::Retry(e.to_string()),
        };
    };
    let now = unix_now();
    let _ = write(w.host.as_ref(), |st| {
        st.set_account_login_block(id, Some(limit as i64), Some(now), Some(now + pause.as_secs() as i64)).map(|()| ((), Default::default()))
    });
    w.status.lock().unwrap().login_limit = Some(limit as u32);
    let why = match signal {
        Signal::Locked => "it accepted this password before: most likely a lock for too many logins",
        Signal::Said => "the server limits logins",
        Signal::TurnedAway => "the server turned the connection away three times in a row",
    };
    let error = format!("{e} ({why}; at most {limit} logins an hour from now on, next attempt in {} min)", pause.as_secs().div_ceil(60));
    let who = if conn == Conn::Work { "" } else { " (bodies)" };
    eprintln!("sync {}{who}: {error}", w.spec.key);
    NoSession::Retry(error)
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

/// Waits `d` (a sleep of the Mac counts, `clock.rs`), or less if the
/// worker is woken (stop, now, kick).
async fn pause(control: &Control, d: Duration) {
    tokio::select! {
        _ = clock::sleep(d) => {}
        _ = control.work.notified() => {}
    }
}

/// Waits until kicked or stopped.
async fn blocked(control: &Control) {
    while !control.stopped() && !control.take_kick() {
        control.work.notified().await;
    }
}

/// The account's error, announced once: `AccountError` when it appears or
/// changes, `AccountChanged` when it goes away.
struct Announced(Option<String>);

impl Announced {
    /// A failure of the connection or of the server: its text cited
    /// (`imap_failed`, `docs/contract.md`, "Errors").
    fn error(&mut self, w: &Shared, error: &str) {
        self.announce(w, nm_api::reason::IMAP_FAILED.with("server_text", error));
    }

    /// The server refused the login.
    fn refused(&mut self, w: &Shared, error: &str) {
        self.announce(w, nm_api::reason::IMAP_AUTH_REFUSED.with("server_text", error));
    }

    /// The server refused a token that was just renewed: IMAP is disabled
    /// for this mailbox (`specifications/oauth2.md`, section 11).
    fn disabled(&mut self, w: &Shared) {
        self.announce(w, nm_api::Why::from(nm_api::reason::IMAP_DISABLED_FOR_MAILBOX));
    }

    /// Only a person can unblock the account (an OAuth2 refresh token
    /// refused): announced, and recorded for `list_accounts`.
    fn blocked(&mut self, w: &Shared, why: nm_api::Why) {
        w.host.account_blocked(w.spec.account_id, Some(why.clone()));
        self.announce(w, why);
    }

    fn announce(&mut self, w: &Shared, why: nm_api::Why) {
        let error = why.message();
        if self.0.as_deref() != Some(error.as_str()) {
            self.0 = Some(error.clone());
            w.host.event(Event::AccountError { account_id: w.spec.account_id, error, why: why.data() });
        }
    }

    fn clear(&mut self, w: &Shared) {
        if self.0.take().is_some() {
            w.host.account_blocked(w.spec.account_id, None);
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
    /// The Mac slept; `alive`: the connection answered within [`PROBE`]
    /// and is kept, else it is dropped.
    Slept { alive: bool },
}

async fn work_loop(w: &Shared) {
    let configured_single = w.spec.config.sync.max_connections < 2;
    // How the connection is kept between passes, learnt from the server's
    // cuts (`pace.rs`). For the life of the worker: a restart of the core
    // learns it again, at the cost of one or two cuts.
    let mut hold = IdleHold::Unknown;
    let poll = Duration::from_secs(w.spec.config.sync.poll_interval_secs);
    let mut inserter = Inserter::new(Source::Imap, None);
    let mut bodies = Bodies::new();
    let mut backoff = Backoff(BACKOFF_MIN);
    let mut announced = Announced(None);
    // On the continuous clock (`clock::now`): after a sleep of an hour, a
    // full pass is due.
    let mut last_full: Option<Duration> = None;
    // Whether each folder keeps keywords, learnt by the spool's SELECTs.
    let mut keywords: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
    // The user's own addresses: their mail is not screened, and they get
    // no decision when they are among the recipients of a sent mail.
    let cfg = &w.spec.config;
    let own: Vec<String> = std::iter::once(cfg.email.to_lowercase()).chain(cfg.identities.iter().map(|i| i.email.to_lowercase())).collect();
    let mut detect_tried: Option<Instant> = None;
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
        let (mut s, namespace) = match connect(w, Conn::Work).await {
            Ok(s) => s,
            Err(NoSession::Stopped) => return,
            Err(NoSession::Blocked { state, error, why }) => {
                // A missing password, or one saved for another server, is
                // already announced by the engine (`list_accounts`,
                // `AccountError`): only a refused login, or a refused OAuth2
                // refresh token (`why`), is news.
                match (why, state) {
                    (Some(why), _) => announced.blocked(w, why),
                    (None, "auth_failed") => announced.refused(w, &error),
                    // A token just renewed was refused too: IMAP is disabled
                    // for this mailbox, not a wrong sign-in.
                    (None, "auth_disabled") => announced.disabled(w),
                    _ => {}
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
        // Since when this connection works: a failure after `STABLE` starts
        // the delays again from their minimum, one sooner doubles them.
        let connected_at = clock::now();
        bodies.new_session();
        // The first pass of a connection is full: what happened while
        // disconnected may be anywhere.
        let mut full = true;
        loop {
            set(&w.status, "syncing", None);
            // What NiceMailer changed first, then what the server changed:
            // the pass then sees the moves done, and attaches their UIDs.
            w.control.spool.store(false, Ordering::SeqCst);
            let spool_result = if cfg.read_only { Ok(Default::default()) } else { spool::run(&mut s, w.host.as_ref(), w.spec.account_id, &mut keywords).await };
            match spool_result {
                Ok(r) => {
                    if r.done + r.reverted + r.retried > 0 {
                        eprintln!("sync {}: spool: {} done, {} undone, {} to retry", w.spec.key, r.done, r.reverted, r.retried);
                    }
                }
                Err(PassError::Stopped) => return,
                Err(e) => {
                    let error = e.to_string();
                    announced.error(w, &error);
                    if clock::now().saturating_sub(connected_at) >= pace::STABLE {
                        backoff.reset();
                        w.pace.lock().unwrap().stable();
                    }
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
                own: &own,
                authserv_config: cfg.authserv_id.as_deref(),
                detect_tried: &mut detect_tried,
                read_only: cfg.read_only,
            };
            match pass::run(&mut s, &mut ctx, full).await {
                Ok(report) => {
                    // Not `backoff.reset()`: a pass that succeeds on a
                    // connection the server cuts a minute later must not
                    // bring the next login back to 5 s (`pace.rs`).
                    announced.clear(w);
                    if full {
                        last_full = Some(clock::now());
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
                            "sync {}: {} new ({} screened out of the Inbox), {} moved elsewhere, {} deleted",
                            w.spec.key, report.inserted, report.screened, report.moved, report.deleted
                        );
                    }
                    // The body queue has new messages, maybe before its rest.
                    w.control.reload.store(true, Ordering::SeqCst);
                    w.control.bodies.notify_one();
                    // The pass itself may have queued actions (the screening
                    // of new mail, a conversation brought back): sent now,
                    // not at the next pass.
                    let spool_result = if cfg.read_only { Ok(Default::default()) } else { spool::run(&mut s, w.host.as_ref(), w.spec.account_id, &mut keywords).await };
                    match spool_result {
                        Ok(r) if r.done + r.reverted + r.retried > 0 => {
                            eprintln!("sync {}: spool: {} done, {} undone, {} to retry", w.spec.key, r.done, r.reverted, r.retried);
                        }
                        Ok(_) => {}
                        Err(PassError::Stopped) => return,
                        Err(e) => {
                            let error = e.to_string();
                            announced.error(w, &error);
                            if clock::now().saturating_sub(connected_at) >= pace::STABLE {
                                backoff.reset();
                                w.pace.lock().unwrap().stable();
                            }
                            retry_later(w, &mut backoff, error).await;
                            continue 'connect;
                        }
                    }
                }
                Err(PassError::Stopped) => return,
                Err(e) => {
                    let error = e.to_string();
                    announced.error(w, &error);
                    if clock::now().saturating_sub(connected_at) >= pace::STABLE {
                        backoff.reset();
                        w.pace.lock().unwrap().stable();
                    }
                    retry_later(w, &mut backoff, error).await;
                    continue 'connect;
                }
            }
            // One connection only (configured, or learnt from the server),
            // or a body connection without session: the bodies are this
            // connection's, a slice after each pass.
            if w.control.work_fetches_bodies() {
                set(&w.status, "syncing", None);
                let deadline = Some(Instant::now() + BODY_SLICE);
                let result = {
                    // Rust note: the guard of the lock lives until the end
                    // of this block, so the turn is given back before the
                    // pass or the wait that follows.
                    let _turn = w.body_turn.lock().await;
                    // Shared with a body connection, the queue may be out
                    // of date: read it again (one indexed query).
                    if !configured_single {
                        bodies.reread();
                    }
                    let r = bodies.run(&mut s, w.host.as_ref(), w.spec.account_id, &w.control, deadline).await;
                    // And the body connection reads its own again.
                    w.control.reload.store(true, Ordering::SeqCst);
                    r
                };
                match result {
                    Ok(more) => {
                        w.status.lock().unwrap().bodies_size_differs = w.control.size_differs.load(Ordering::SeqCst);
                        // Bodies left after a whole slice: worth a login of
                        // the body connection, if it waits for that.
                        w.control.backlog.store(more, Ordering::SeqCst);
                        if more {
                            w.control.bodies.notify_one();
                            // Bodies left: a quick pass first, for new mail.
                            full = due(last_full, poll) || w.control.take_now();
                            continue;
                        }
                    }
                    Err(PassError::Stopped) => return,
                    Err(e) => {
                        let error = e.to_string();
                        announced.error(w, &error);
                        if clock::now().saturating_sub(connected_at) >= pace::STABLE {
                            backoff.reset();
                            w.pace.lock().unwrap().stable();
                        }
                        retry_later(w, &mut backoff, error).await;
                        continue 'connect;
                    }
                }
            }
            if hold == IdleHold::Poll {
                // No connection kept with this server: closed now, a new one
                // for the next pass, within the pace of the logins.
                s.logout().await;
                set(&w.status, "idle", None);
                let d = poll.max(pace::POLL_FLOOR).saturating_sub(since(last_full));
                pause(&w.control, d).await;
                if w.control.stopped() {
                    return;
                }
                continue 'connect;
            }
            set(&w.status, "idle", None);
            let until_full = last_full.map_or(Duration::ZERO, |_| poll.saturating_sub(since(last_full)));
            let woken = match wait(&mut s, &w.control, until_full, hold.renew_every()).await {
                Ok(woken) => woken,
                Err(WaitError { error, idled }) => {
                    // Cut by the server while in IDLE, after a pass that
                    // worked: its limit on idle connections, not a failure
                    // to announce. Learnt, and the connection is not
                    // counted as a good one (`connected_at`), so the delay
                    // still grows if it happens again.
                    if let Some(idled) = idled.filter(|_| error.is_fatal() && !matches!(error, ImapError::Timeout(_))) {
                        hold = hold.after_cut(idled);
                        w.control.one_connection.store(true, Ordering::SeqCst);
                        w.status.lock().unwrap().hold = Some(match hold {
                            IdleHold::Renew(d) => format!("renew {}", d.as_secs()),
                            _ => format!("poll {}", poll.max(pace::POLL_FLOOR).as_secs()),
                        });
                        w.control.bodies.notify_one();
                        let next = match hold {
                            IdleHold::Renew(d) => format!("IDLE renewed every {} s from now on", d.as_secs()),
                            _ => format!(
                                "the account is polled from now on, one connection per {} min at most",
                                poll.max(pace::POLL_FLOOR).as_secs() / 60
                            ),
                        };
                        eprintln!("sync {}: the server closed the connection after {} s in IDLE ({error}); {next}", w.spec.key, idled.as_secs());
                        retry_later_quietly(w, &mut backoff).await;
                        continue 'connect;
                    }
                    let error = error.to_string();
                    announced.error(w, &error);
                    if clock::now().saturating_sub(connected_at) >= pace::STABLE {
                        backoff.reset();
                        w.pace.lock().unwrap().stable();
                    }
                    retry_later(w, &mut backoff, error).await;
                    continue 'connect;
                }
            };
            if w.control.stopped() {
                s.logout().await;
                return;
            }
            full = match woken {
                Woken::Slept { alive: true } => {
                    // Kept: what came during the sleep may be in any
                    // folder, and IDLE only watched the Inbox.
                    eprintln!("sync {}: the Mac slept, the connection is still alive", w.spec.key);
                    true
                }
                Woken::Slept { alive: false } => {
                    // Not announced: the network is back in a moment. The
                    // login waits for the Mac to be awake (`connect`).
                    eprintln!("sync {}: the Mac slept, the connection did not survive: new connection", w.spec.key);
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
fn due(last_full: Option<Duration>, poll: Duration) -> bool {
    last_full.is_none_or(|_| since(last_full) >= poll)
}

/// Time since a moment of the continuous clock (zero for none).
fn since(t: Option<Duration>) -> Duration {
    t.map_or(Duration::ZERO, |t| clock::now().saturating_sub(t))
}

/// Records the error and waits before the next attempt.
async fn retry_later(w: &Shared, backoff: &mut Backoff, error: String) {
    // Not before the pause a block imposed: the attempt would only wait
    // for it in `connect`, after a log saying otherwise.
    let d = backoff.next().max(w.pace.lock().unwrap().wait(clock::now()));
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

/// Waits `backoff`'s next delay after a cut the worker understands (the
/// server's limit on IDLE): the state says "waiting", without an error.
async fn retry_later_quietly(w: &Shared, backoff: &mut Backoff) {
    let d = backoff.next();
    {
        let mut st = w.status.lock().unwrap();
        st.state = "waiting".into();
        st.next_retry = Some(unix_now() + d.as_secs() as i64);
    }
    pause(&w.control, d).await;
    w.control.take_kick();
}

/// A wait that failed: the error, and how long the current IDLE command
/// had run when it came (`None`: not in IDLE).
struct WaitError {
    error: ImapError,
    idled: Option<Duration>,
}

impl From<ImapError> for WaitError {
    fn from(error: ImapError) -> WaitError {
        WaitError { error, idled: None }
    }
}

/// The wait between two passes: IDLE on the Inbox, or a plain timer for a
/// server without IDLE. `renew`: the longest IDLE command this server
/// keeps (`IdleHold::Renew`); IDLE is ended and started again at that
/// interval until the wait is over.
async fn wait(s: &mut Session, control: &Control, until_full: Duration, renew: Option<Duration>) -> Result<Woken, WaitError> {
    if !s.capabilities().has("IDLE") {
        let d = until_full.min(NO_IDLE_POLL).max(Duration::from_secs(1));
        // Rust note: the branch's handler runs once its future is done,
        // and the futures no longer borrow anything then: `s` is free for
        // the `NOOP`.
        return Ok(tokio::select! {
            _ = tokio::time::sleep(d) => Woken::Timeout,
            _ = control.work.notified() => Woken::Signal,
            _ = clock::slept() => Woken::Slept { alive: answers(s).await },
        });
    }
    // IDLE watches the selected mailbox: the Inbox, read-only.
    s.examine(&MailboxName::inbox(), None).await?;
    let max = until_full.clamp(Duration::from_secs(1), IDLE_MAX);
    let deadline = Instant::now() + max;
    // The watch for a sleep lives across the renewals of IDLE: started
    // again at each, it would miss a sleep that ends just before one.
    //
    // Rust note: `pin!` fixes the future in place, so that it can be
    // polled again and again by reference (`as_mut`) without moving.
    let mut watch = std::pin::pin!(clock::slept());
    let woke = Woke::default();
    let ended = tokio::select! {
        r = idle_until(s, control, deadline, renew, watch.as_mut(), &woke) => r?,
        // A sleep noticed: IDLE ends with `DONE`, and the server's answer
        // shows the connection alive. One that does not answer within
        // `PROBE` is dead (its own time limit would be 90 s).
        _ = async { woke.notify.notified().await; tokio::time::sleep(PROBE).await } => Ended::Slept { alive: false },
    };
    s.take_unsolicited();
    Ok(match ended {
        Ended::Slept { alive } => Woken::Slept { alive },
        Ended::Idle(IdleEnd::Changes) => Woken::Changes,
        Ended::Idle(IdleEnd::Timeout) => Woken::Timeout,
        Ended::Idle(IdleEnd::Stopped) => Woken::Signal,
    })
}

/// Whether the connection answers a `NOOP` within [`PROBE`].
///
/// Rust note: `timeout` gives `Err` when the time ran out, and `Ok` with
/// the result of the command otherwise: two `Result`s, one in the other.
async fn answers(s: &mut Session) -> bool {
    matches!(tokio::time::timeout(PROBE, s.noop()).await, Ok(Ok(())))
}

/// A sleep noticed while in IDLE, by the watch of [`idle_until`].
///
/// Rust note: `Cell` lets a shared reference (`&Woke`) change a plain
/// value; it is not thread-safe, which a single-threaded runtime does not
/// need.
#[derive(Default)]
struct Woke {
    seen: std::cell::Cell<bool>,
    /// Starts the time limit of the answer, in [`wait`].
    notify: tokio::sync::Notify,
}

/// How [`idle_until`] ended.
enum Ended {
    Idle(IdleEnd),
    /// The Mac slept; `alive`: the server answered the `DONE`.
    Slept { alive: bool },
}

/// IDLE until `deadline`, each command at most `renew` long: one ended by
/// its time limit before the deadline is started again at once (`DONE`,
/// then `IDLE`), which the server sees as activity. A sleep of the Mac,
/// seen by `watch`, ends IDLE like a signal: `DONE` is sent, and its
/// answer tells whether the connection survived.
async fn idle_until(
    s: &mut Session,
    control: &Control,
    deadline: Instant,
    renew: Option<Duration>,
    mut watch: std::pin::Pin<&mut impl Future<Output = ()>>,
    woke: &Woke,
) -> Result<Ended, WaitError> {
    loop {
        let left = deadline.saturating_duration_since(Instant::now()).max(Duration::from_secs(1));
        let slice = renew.map_or(left, |r| r.min(left));
        let started = Instant::now();
        let stop = async {
            tokio::select! {
                _ = control.work.notified() => {}
                _ = watch.as_mut() => {
                    woke.seen.set(true);
                    woke.notify.notify_one();
                }
            }
        };
        let r = s.idle(slice, stop).await;
        if woke.seen.get() {
            // Not a cut of the server in IDLE, to learn from (`pace.rs`):
            // the connection died with the sleep, or answered.
            return Ok(Ended::Slept { alive: r.is_ok() });
        }
        match r {
            Ok(IdleEnd::Timeout) if Instant::now() + Duration::from_secs(1) < deadline => continue,
            Ok(end) => return Ok(Ended::Idle(end)),
            Err(error) => return Err(WaitError { error, idled: Some(started.elapsed()) }),
        }
    }
}

/// The body connection has no session any more: the work connection
/// fetches the bodies until it has one again. Woken if it waits in IDLE,
/// so that the bodies left, and those of the mail that comes meanwhile,
/// do not wait for the next change on the Inbox. Not woken in any other
/// state: it does a pass first anyway, which hands the bodies over; and a
/// signal there would cut short its delay before a new login.
fn body_session_lost(w: &Shared) {
    if w.control.body_session.swap(false, Ordering::SeqCst) && w.status.lock().unwrap().state == "idle" {
        w.control.work.notify_one();
    }
}

/// The body connection (`sync.max_connections = 2`).
async fn body_loop(w: &Shared) {
    let mut bodies = Bodies::new();
    let mut backoff = Backoff(BACKOFF_MIN);
    // The connection died with a sleep of the Mac: no new login until the
    // work connection leaves bodies (`Control::backlog`).
    let mut lazy = false;
    'connect: loop {
        body_session_lost(w);
        // The server cuts idle connections: the work connection fetches
        // the bodies from now on, this one stays closed (`pace.rs`).
        if w.control.one_connection.load(Ordering::SeqCst) {
            parked_bodies(w).await;
            return;
        }
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
        if lazy {
            until_backlog(w).await;
            if w.control.stopped() {
                return;
            }
            if w.control.one_connection.load(Ordering::SeqCst) {
                continue;
            }
            lazy = false;
        }
        let mut s = match connect(w, Conn::Bodies).await {
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
        // From now on the bodies are this connection's: the work
        // connection goes back to IDLE after its current slice.
        w.control.body_session.store(true, Ordering::SeqCst);
        loop {
            let result = {
                // The turn, while the work connection may still be in a
                // slice it began before this session. The queue is read
                // again after it (`control.reload`, set by that slice).
                let _turn = w.body_turn.lock().await;
                bodies.run(&mut s, w.host.as_ref(), w.spec.account_id, &w.control, None).await
            };
            match result {
                Ok(_) => {
                    backoff.reset();
                    w.control.backlog.store(false, Ordering::SeqCst);
                    w.status.lock().unwrap().bodies_size_differs = w.control.size_differs.load(Ordering::SeqCst);
                    // None left: wait for new messages, a body asked for,
                    // the time to check the connection is alive, or a
                    // sleep of the Mac, after which it is checked too.
                    let (alive, slept) = tokio::select! {
                        _ = w.control.bodies.notified() => (true, false),
                        _ = tokio::time::sleep(BODY_KEEPALIVE) => (s.noop().await.is_ok(), false),
                        _ = clock::slept() => (answers(&mut s).await, true),
                    };
                    if slept && alive {
                        eprintln!("sync {} (bodies): the Mac slept, the connection is still alive", w.spec.key);
                    }
                    if w.control.stopped() {
                        s.logout().await;
                        return;
                    }
                    if w.control.one_connection.load(Ordering::SeqCst) {
                        s.logout().await;
                        continue 'connect;
                    }
                    if !alive {
                        if slept {
                            eprintln!("sync {} (bodies): the Mac slept, the connection did not survive: the work connection fetches the bodies", w.spec.key);
                        }
                        lazy = slept;
                        continue 'connect;
                    }
                }
                Err(PassError::Stopped) => return,
                Err(e) => {
                    eprintln!("sync {} (bodies): {e}", w.spec.key);
                    // Before the delay, not after it: the work connection
                    // fetches the bodies meanwhile.
                    body_session_lost(w);
                    pause_bodies(&w.control, backoff.next()).await;
                    continue 'connect;
                }
            }
        }
    }
}

/// The body thread with nothing to do until the worker stops: the work
/// connection fetches the bodies (`Control::one_connection`).
async fn parked_bodies(w: &Shared) {
    while !w.control.stopped() {
        tokio::select! {
            _ = w.control.bodies.notified() => {}
            _ = tokio::time::sleep(Duration::from_secs(5)) => {}
        }
    }
}

/// The body connection, dead with a sleep of the Mac, waits for the work
/// connection to leave bodies after one of its slices (or for the worker
/// to stop). Until then the work connection fetches them: a few new mails
/// are not worth a second login, which a Mac that sleeps every few
/// minutes would make at each wake.
async fn until_backlog(w: &Shared) {
    while !w.control.stopped() && !w.control.backlog.load(Ordering::SeqCst) && !w.control.one_connection.load(Ordering::SeqCst) {
        tokio::select! {
            _ = w.control.bodies.notified() => {}
            _ = tokio::time::sleep(Duration::from_secs(5)) => {}
        }
    }
}

async fn pause_bodies(control: &Control, d: Duration) {
    tokio::select! {
        _ = clock::sleep(d) => {}
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
