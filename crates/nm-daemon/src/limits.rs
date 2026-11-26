//! The resource limits of the socket (`specifications/architecture.md`,
//! section 9, "Journal et limites"): how long a line may be, how many
//! connections are served, how fast a client may send. A client that abuses
//! the socket — a bug, an agent in a loop, a process that floods it — is
//! refused or slowed; it cannot make the core's memory, its file
//! descriptors or its threads grow without bound, nor lock the app out.
//!
//! Every limit is per **verified source** where it can be (`signing.rs`),
//! not per connection: opening more connections gives a client nothing
//! more. The app is never rate-limited — it is the user at the keyboard,
//! and the only client the spec leaves free — and has connections of its
//! own that no other client can take.
//!
//! Each value can be lowered by a test through a `NICEMAILER_TEST_*`
//! variable ([`Limits::from_env`]); they only make a limit smaller or a
//! wait shorter in practice, and the tests rely on them to stay fast.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nm_api::method;
use nm_core::Source;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// The longest line read from a client of a person (the app, the CLI, the
/// TUI), in bytes: a request carries text — a draft's body is the biggest
/// of them — never a file, an attachment travels by its path
/// (`architecture.md`, section 6). 16 MiB leaves room for a very long body
/// and bounds what one line can make the core allocate.
pub(crate) const MAX_LINE: usize = 16 << 20;

/// The longest line of an agent or of the web server (MCP, web): their
/// drafts are written by a model, 4 MiB is far beyond any of them.
pub(crate) const AGENT_LINE: usize = 4 << 20;

/// The longest line before `hello`: the handshake is a few dozen bytes. A
/// connection that has not said who it is cannot make the core allocate
/// 16 MiB.
pub(crate) const PRE_HELLO_LINE: usize = 64 << 10;

/// How long a connection may take, from its accept, to be verified and to
/// send its `hello`: a connection that never does holds a task and a file
/// descriptor for nothing.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Connections whose signature is being checked at once. Beyond, a new
/// connection is closed at once: the checks run on the blocking pool, and
/// a flood of connections must not fill it.
const PENDING_CONNECTIONS: usize = 16;

/// Connections served at once, per source. The app has its own, which no
/// other client can take; a CLI run in a loop by a script, or an agent
/// opening connections, stops at its own cap (`architecture.md`,
/// section 9). Their sum, with [`PENDING_CONNECTIONS`], bounds the file
/// descriptors the socket can use.
fn source_cap(source: Source) -> usize {
    match source {
        Source::App => 8,
        Source::Cli => 32,
        Source::Tui | Source::Web | Source::Mcp => 8,
        // Not clients of the socket.
        Source::Core | Source::ConfigSync | Source::Imap => 0,
    }
}

/// Connections served at once whose signature was not verified: only in a
/// relaxed build (debug, `dev-unverified`), where `hello` declares the
/// source.
const UNVERIFIED_CONNECTIONS: usize = 64;

/// The requests of one source (other than the app) may burst up to this
/// many at once, then [`RATE_PER_SEC`] per second on average; beyond, the
/// request is refused with `RATE_LIMITED`.
const RATE_BURST: f64 = 100.0;
const RATE_PER_SEC: f64 = 20.0;

/// The deletions, moves and sends of one source other than the app
/// (`architecture.md`, section 9: "Les suppressions et les demandes d'envoi
/// des clients autres que l'app sont soumises à une limite de débit"): a
/// smaller budget, on top of the general one. A batch moves many threads
/// in one request, so a script is not slowed by it.
const DESTRUCTIVE_BURST: f64 = 30.0;
const DESTRUCTIVE_PER_SEC: f64 = 2.0;

/// What a connection other than the app's may send outside the methods of
/// the contract: `ping`, `hello`, `subscribe`, anything before `hello`,
/// and invalid JSON. They change nothing, but each costs a parse and an
/// answer: a flood of them is refused too.
const SESSION_BURST: f64 = 50.0;
const SESSION_PER_SEC: f64 = 10.0;

/// The methods counted against [`DESTRUCTIVE_BURST`]: what deletes, moves
/// out of sight or sends.
pub(crate) fn is_destructive(m: &str) -> bool {
    matches!(
        m,
        method::MOVE_THREAD
            | method::MOVE_THREADS
            | method::DELETE_TAG
            | method::DELETE_GROUP
            | method::REMOVE_ACCOUNT
            | method::DISCARD_DRAFT
            | method::SEND_DRAFT
            | method::RAW_GC
    )
}

/// The limits of this core: the constants above, or the smaller values a
/// test asks for.
#[derive(Debug, Clone)]
pub(crate) struct Limits {
    pub max_line: usize,
    pub handshake_timeout: Duration,
    pub rate: (f64, f64),
    pub destructive: (f64, f64),
    pub session: (f64, f64),
    pub unverified_connections: usize,
}

impl Limits {
    /// The constants, with the overrides of the tests:
    /// `NICEMAILER_TEST_MAX_LINE_BYTES`, `NICEMAILER_TEST_HANDSHAKE_TIMEOUT_MS`,
    /// `NICEMAILER_TEST_RATE_BURST` / `_PER_SEC`,
    /// `NICEMAILER_TEST_DESTRUCTIVE_BURST` / `_PER_SEC`,
    /// `NICEMAILER_TEST_SESSION_BURST` / `_PER_SEC`,
    /// `NICEMAILER_TEST_MAX_CONNECTIONS` (the unverified connections).
    pub fn from_env() -> Limits {
        // Rust note: a generic function; `T: FromStr` says it works for any
        // type that can be parsed from text (numbers here).
        fn var<T: std::str::FromStr>(name: &str) -> Option<T> {
            std::env::var(name).ok().and_then(|v| v.parse().ok())
        }
        Limits {
            max_line: var("NICEMAILER_TEST_MAX_LINE_BYTES").unwrap_or(MAX_LINE),
            handshake_timeout: var("NICEMAILER_TEST_HANDSHAKE_TIMEOUT_MS").map(Duration::from_millis).unwrap_or(HANDSHAKE_TIMEOUT),
            rate: (var("NICEMAILER_TEST_RATE_BURST").unwrap_or(RATE_BURST), var("NICEMAILER_TEST_RATE_PER_SEC").unwrap_or(RATE_PER_SEC)),
            destructive: (
                var("NICEMAILER_TEST_DESTRUCTIVE_BURST").unwrap_or(DESTRUCTIVE_BURST),
                var("NICEMAILER_TEST_DESTRUCTIVE_PER_SEC").unwrap_or(DESTRUCTIVE_PER_SEC),
            ),
            session: (
                var("NICEMAILER_TEST_SESSION_BURST").unwrap_or(SESSION_BURST),
                var("NICEMAILER_TEST_SESSION_PER_SEC").unwrap_or(SESSION_PER_SEC),
            ),
            unverified_connections: var("NICEMAILER_TEST_MAX_CONNECTIONS").unwrap_or(UNVERIFIED_CONNECTIONS),
        }
    }

    /// The longest line the next read of a connection accepts: small before
    /// `hello`, then by source. Never more than `max_line`, which a test
    /// may lower.
    pub fn line_limit(&self, hello: bool, source: Source) -> usize {
        let by_stage = if !hello {
            PRE_HELLO_LINE
        } else {
            match source {
                Source::Mcp | Source::Web => AGENT_LINE,
                _ => MAX_LINE,
            }
        };
        by_stage.min(self.max_line)
    }
}

/// The places of connection: one held while a signature is checked, then
/// one of the client's source for the whole life of the connection.
pub(crate) struct Slots {
    pending: Arc<Semaphore>,
    unverified: Arc<Semaphore>,
    /// By [`index`] of the source.
    by_source: [Arc<Semaphore>; SOURCES],
}

impl Slots {
    pub fn new(limits: &Limits) -> Slots {
        let sem = |n| Arc::new(Semaphore::new(n));
        Slots {
            pending: sem(PENDING_CONNECTIONS),
            unverified: sem(limits.unverified_connections),
            by_source: CLIENT_SOURCES.map(|s| sem(source_cap(s))),
        }
    }

    /// A place to check a signature, or `None` when too many are checked.
    pub fn pending(&self) -> Option<OwnedSemaphorePermit> {
        self.pending.clone().try_acquire_owned().ok()
    }

    /// The place of a connection admitted as `source` (`None`: unverified),
    /// or `None` when that source has all its places taken.
    pub fn admit(&self, source: Option<Source>) -> Option<OwnedSemaphorePermit> {
        let sem = match source {
            Some(s) => &self.by_source[index(s)?],
            None => &self.unverified,
        };
        sem.clone().try_acquire_owned().ok()
    }
}

/// The connections a core in maintenance serves at once: each is only told
/// the maintenance (`update.rs`), their number is bounded all the same.
pub(crate) fn refusing_slots() -> Arc<Semaphore> {
    Arc::new(Semaphore::new(PENDING_CONNECTIONS + UNVERIFIED_CONNECTIONS))
}

/// The sources that are clients of the socket.
const CLIENT_SOURCES: [Source; SOURCES] = [Source::App, Source::Cli, Source::Tui, Source::Web, Source::Mcp];
const SOURCES: usize = 5;

/// The place of `source` in [`CLIENT_SOURCES`].
fn index(source: Source) -> Option<usize> {
    CLIENT_SOURCES.iter().position(|s| *s == source)
}

/// A token bucket: `burst` requests may go at once, then `per_sec` per
/// second on average. A pure function of the instants, tested alone.
#[derive(Debug, Clone)]
pub(crate) struct RateLimit {
    burst: f64,
    per_sec: f64,
    tokens: f64,
    last: Instant,
}

impl RateLimit {
    pub fn new((burst, per_sec): (f64, f64)) -> RateLimit {
        RateLimit { burst, per_sec, tokens: burst, last: Instant::now() }
    }

    /// Whether a request may go at `now`, taking one token if so.
    pub fn allow(&mut self, now: Instant) -> bool {
        let elapsed = now.saturating_duration_since(self.last);
        self.tokens = (self.tokens + elapsed.as_secs_f64() * self.per_sec).min(self.burst);
        self.last = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// The rate limits of each source, shared by all its connections.
pub(crate) struct SourceRates {
    general: Mutex<Vec<RateLimit>>,
    destructive: Mutex<Vec<RateLimit>>,
}

impl SourceRates {
    pub fn new(limits: &Limits) -> SourceRates {
        SourceRates {
            general: Mutex::new(vec![RateLimit::new(limits.rate); SOURCES]),
            destructive: Mutex::new(vec![RateLimit::new(limits.destructive); SOURCES]),
        }
    }

    /// Whether `source` may call `m` at `now`. The app always may.
    pub fn allow(&self, source: Source, m: &str, now: Instant) -> bool {
        if source == Source::App {
            return true;
        }
        let Some(i) = index(source) else { return false };
        if !self.general.lock().unwrap()[i].allow(now) {
            return false;
        }
        !is_destructive(m) || self.destructive.lock().unwrap()[i].allow(now)
    }
}

/// A log line at most once a second, with the number of the lines it
/// stood for since: a flood of refused connections must not fill the log
/// (`launchd` keeps it on disk).
pub(crate) struct LogThrottle {
    state: Mutex<(Option<Instant>, u64)>,
}

impl LogThrottle {
    pub const fn new() -> LogThrottle {
        LogThrottle { state: Mutex::new((None, 0)) }
    }

    /// Writes `line()` to the log, unless a line was written less than a
    /// second ago (it is then counted, and said with the next one).
    pub fn log(&self, line: impl FnOnce() -> String) {
        if let Some(text) = self.next(Instant::now(), line) {
            eprintln!("{text}");
        }
    }

    /// The line to write at `now`, if any (pure, for the test).
    fn next(&self, now: Instant, line: impl FnOnce() -> String) -> Option<String> {
        let mut state = self.state.lock().unwrap();
        let (last, skipped) = &mut *state;
        if last.is_some_and(|l| now.saturating_duration_since(l) < Duration::from_secs(1)) {
            *skipped += 1;
            return None;
        }
        let text = if *skipped > 0 { format!("{} ({} autres depuis la dernière ligne)", line(), skipped) } else { line() };
        *last = Some(now);
        *skipped = 0;
        Some(text)
    }
}

/// What to do after a failed `accept`: log it (throttled), and wait a
/// little when the process is out of file descriptors, which would
/// otherwise make the loop spin. Never a reason to stop the core.
pub(crate) async fn accept_failed(e: &std::io::Error) {
    static ERRORS: LogThrottle = LogThrottle::new();
    ERRORS.log(|| format!("accept a échoué : {e}"));
    if matches!(e.raw_os_error(), Some(libc::EMFILE) | Some(libc::ENFILE)) {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> Limits {
        Limits {
            max_line: MAX_LINE,
            handshake_timeout: HANDSHAKE_TIMEOUT,
            rate: (RATE_BURST, RATE_PER_SEC),
            destructive: (DESTRUCTIVE_BURST, DESTRUCTIVE_PER_SEC),
            session: (SESSION_BURST, SESSION_PER_SEC),
            unverified_connections: UNVERIFIED_CONNECTIONS,
        }
    }

    /// The token bucket lets the burst through, refuses beyond it, and
    /// refills with time, never holding more than the burst.
    #[test]
    fn rate_limit_lets_the_burst_through_then_refills() {
        let start = Instant::now();
        let mut r = RateLimit::new((3.0, 10.0));
        assert!(r.allow(start) && r.allow(start) && r.allow(start), "the burst goes at once");
        assert!(!r.allow(start), "the burst is spent");
        assert!(!r.allow(start + Duration::from_millis(50)), "half a token is not a token");
        assert!(r.allow(start + Duration::from_millis(100)), "one token refilled in 100 ms at 10/s");
        assert!(!r.allow(start + Duration::from_millis(100)));
        let later = start + Duration::from_secs(3600);
        assert!(r.allow(later) && r.allow(later) && r.allow(later));
        assert!(!r.allow(later), "capped at the burst");
    }

    /// The app is never limited; another source is, across all its
    /// connections, and its deletions have a budget of their own.
    #[test]
    fn rates_are_per_source_and_spare_the_app() {
        let mut l = limits();
        l.rate = (5.0, 0.0);
        l.destructive = (2.0, 0.0);
        let rates = SourceRates::new(&l);
        let now = Instant::now();
        for _ in 0..1000 {
            assert!(rates.allow(Source::App, method::MOVE_THREAD, now));
        }
        // Two deletions, then the third is refused although the general
        // budget has three left.
        assert!(rates.allow(Source::Mcp, method::DELETE_TAG, now));
        assert!(rates.allow(Source::Mcp, method::DELETE_TAG, now));
        assert!(!rates.allow(Source::Mcp, method::DELETE_TAG, now));
        // Reads go on until the general budget is spent (5 in all).
        assert!(rates.allow(Source::Mcp, method::LIST_FOLDERS, now));
        assert!(rates.allow(Source::Mcp, method::LIST_FOLDERS, now));
        assert!(!rates.allow(Source::Mcp, method::LIST_FOLDERS, now));
        // Another source has its own budget.
        assert!(rates.allow(Source::Cli, method::LIST_FOLDERS, now));
        // A source that is not a client of the socket is refused.
        assert!(!rates.allow(Source::Imap, method::LIST_FOLDERS, now));
    }

    #[test]
    fn deletions_moves_and_sends_are_destructive() {
        for m in [method::MOVE_THREAD, method::MOVE_THREADS, method::DELETE_TAG, method::DELETE_GROUP, method::REMOVE_ACCOUNT, method::DISCARD_DRAFT, method::SEND_DRAFT, method::RAW_GC] {
            assert!(is_destructive(m), "{m}");
        }
        for m in [method::LIST_FOLDERS, method::GET_THREAD, method::CREATE_DRAFT, method::HELLO] {
            assert!(!is_destructive(m), "{m}");
        }
    }

    /// Small before `hello`, smaller for an agent, never beyond the limit
    /// a test sets.
    #[test]
    fn the_line_limit_depends_on_the_stage_and_the_source() {
        let mut l = limits();
        assert_eq!(l.line_limit(false, Source::App), PRE_HELLO_LINE);
        assert_eq!(l.line_limit(true, Source::App), MAX_LINE);
        assert_eq!(l.line_limit(true, Source::Cli), MAX_LINE);
        assert_eq!(l.line_limit(true, Source::Mcp), AGENT_LINE);
        assert_eq!(l.line_limit(true, Source::Web), AGENT_LINE);
        l.max_line = 1024;
        assert_eq!(l.line_limit(false, Source::App), 1024);
        assert_eq!(l.line_limit(true, Source::App), 1024);
    }

    /// Each source has its own places: an agent that takes all of its own
    /// leaves the app's untouched.
    #[test]
    fn each_source_has_its_own_places() {
        let slots = Slots::new(&limits());
        let held: Vec<_> = (0..source_cap(Source::Mcp)).map(|_| slots.admit(Some(Source::Mcp)).expect("a place")).collect();
        assert!(slots.admit(Some(Source::Mcp)).is_none(), "the agent's places are all taken");
        assert!(slots.admit(Some(Source::App)).is_some(), "the app still has its own");
        drop(held);
        assert!(slots.admit(Some(Source::Mcp)).is_some(), "freed with the connections");
        assert!(slots.admit(Some(Source::Imap)).is_none(), "not a client of the socket");
        let pending: Vec<_> = (0..PENDING_CONNECTIONS).map(|_| slots.pending().unwrap()).collect();
        assert!(slots.pending().is_none());
        drop(pending);
        // The sum of the places bounds the file descriptors.
        let total: usize = CLIENT_SOURCES.iter().map(|s| source_cap(*s)).sum::<usize>() + PENDING_CONNECTIONS;
        assert!(total <= 128, "{total}");
    }

    #[test]
    fn the_log_says_a_flood_once_a_second() {
        let t = LogThrottle::new();
        let start = Instant::now();
        assert_eq!(t.next(start, || "refusée".into()).as_deref(), Some("refusée"));
        for i in 1..=5 {
            assert_eq!(t.next(start + Duration::from_millis(i * 100), || "refusée".into()), None);
        }
        assert_eq!(t.next(start + Duration::from_secs(1), || "refusée".into()).as_deref(), Some("refusée (5 autres depuis la dernière ligne)"));
    }
}
