//! The clocks of the synchronization, and the sleeps of the Mac
//! (`docs/sync.md`, "Sleep of the Mac").
//!
//! macOS has two monotonic clocks, and they differ by the time the Mac
//! slept:
//!
//! - [`now`], the *continuous* clock (`CLOCK_MONOTONIC` on macOS): it goes
//!   on while the Mac sleeps, like the wall clock, but no one can set it.
//!   The rules that speak of real time use it: the pace of the logins
//!   ("N an hour" is an hour of the server's, which does not sleep), the
//!   delays before a new attempt, the time since the last full pass;
//! - [`awake`], the *awake* clock (`CLOCK_UPTIME_RAW`): it stops while the
//!   Mac sleeps. It is the clock of `std::time::Instant` and of tokio's
//!   timers on macOS.
//!
//! Until 21 November 2026 everything used `Instant`. A Mac that sleeps
//! every few minutes when left alone (idle sleep, then a "dark wake" for
//! the network, then sleep again) thus stretched "an hour" over several
//! real hours: after five wakes the budget of logins was spent, and the
//! wait for it did not end when `sync status` said it would. Three
//! accounts stayed without connection for over an hour while the phone
//! showed the new mail.
//!
//! The gap between the two clocks shows a sleep ([`slept_between`]), and
//! [`since_wake`] says how long ago the Mac last woke, from the kernel
//! (`kern.waketime`).
//!
//! # Simulated sleep, for the tests
//!
//! A sleep cannot be caused in a test. With
//! `NICEMAILER_TEST_SLEEP_FILE=<path>`, the core reads a number of
//! seconds in that file: the continuous clock is that much ahead (the Mac
//! "slept" that long), and the Mac "woke" when the file was last written
//! (`crates/nm-daemon/tests/pace_rpc.rs`). Without the variable, nothing
//! is read.

use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

/// The longest a wait sleeps on tokio's timer before it looks at the
/// continuous clock again. Tokio's timer stops while the Mac sleeps: a
/// wait of 30 minutes started before a sleep of an hour would otherwise
/// end 30 minutes after the wake, not at it.
const SLICE: Duration = Duration::from_secs(15);

/// A gap between the two clocks longer than this is a sleep. Short: a
/// sleep costs nothing to notice, since the connection is tested before
/// it is replaced (`account.rs`). The gap is measured over 15 s, so the
/// small drift of the two clocks never reaches it.
const SLEEP_MIN: Duration = Duration::from_secs(5);

/// The continuous clock: time since boot, sleeps included.
pub(crate) fn now() -> Duration {
    os::continuous() + simulated().map_or(Duration::ZERO, |s| s.slept)
}

/// The awake clock: time since boot, sleeps excluded.
pub(crate) fn awake() -> Duration {
    os::awake()
}

/// Whether the Mac slept between two readings: the continuous clock went
/// ahead of the awake clock by more than [`SLEEP_MIN`].
///
/// Rust note: `saturating_sub` stops at zero instead of panicking when
/// the second duration is the larger (the two clocks drift a little).
pub(crate) fn slept_between(continuous: Duration, awake: Duration) -> bool {
    continuous.saturating_sub(awake) > SLEEP_MIN
}

/// Waits `d` of the continuous clock: a sleep of the Mac counts.
pub(crate) async fn sleep(d: Duration) {
    let deadline = now() + d;
    loop {
        let left = deadline.saturating_sub(now());
        if left.is_zero() {
            return;
        }
        tokio::time::sleep(left.min(SLICE)).await;
    }
}

/// Returns when the Mac seems to have slept, looking every 15 s.
pub(crate) async fn slept() {
    loop {
        let (c, a) = (now(), awake());
        tokio::time::sleep(SLICE).await;
        if slept_between(now().saturating_sub(c), awake().saturating_sub(a)) {
            return;
        }
    }
}

/// How long ago the Mac last woke, `None` if it never slept since boot.
pub(crate) fn since_wake() -> Option<Duration> {
    let real = os::wake_time().map(|t| SystemTime::now().duration_since(t).unwrap_or_default());
    let fake = simulated().and_then(|s| s.woke).map(|t| SystemTime::now().duration_since(t).unwrap_or_default());
    // The most recent of the two wakes.
    match (real, fake) {
        (Some(r), Some(f)) => Some(r.min(f)),
        (r, f) => r.or(f),
    }
}

/// A sleep simulated by a test (`NICEMAILER_TEST_SLEEP_FILE`).
struct Simulated {
    slept: Duration,
    woke: Option<SystemTime>,
}

fn simulated() -> Option<Simulated> {
    // Rust note: `OnceLock` computes a value once, on first use, safely
    // from any thread: the variable is read once per process.
    static PATH: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
    let path = PATH.get_or_init(|| std::env::var_os("NICEMAILER_TEST_SLEEP_FILE").map(Into::into)).as_ref()?;
    let text = std::fs::read_to_string(path).ok()?;
    let secs: u64 = text.trim().parse().ok()?;
    let woke = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    Some(Simulated { slept: Duration::from_secs(secs), woke })
}

#[cfg(target_vendor = "apple")]
mod os {
    use std::time::{Duration, SystemTime};

    fn clock(id: libc::clockid_t) -> Duration {
        let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
        // SAFETY: `ts` is a valid, writable timespec; these clocks exist
        // on every macOS NiceMailer runs on (10.12 and later).
        unsafe { libc::clock_gettime(id, &mut ts) };
        Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32)
    }

    pub(super) fn continuous() -> Duration {
        clock(libc::CLOCK_MONOTONIC)
    }

    pub(super) fn awake() -> Duration {
        clock(libc::CLOCK_UPTIME_RAW)
    }

    /// The wall time of the last wake (`sysctl kern.waketime`), dark
    /// wakes included; `None` before the first sleep.
    pub(super) fn wake_time() -> Option<SystemTime> {
        let mut tv = libc::timeval { tv_sec: 0, tv_usec: 0 };
        let mut len = std::mem::size_of::<libc::timeval>();
        // SAFETY: the name is NUL-terminated, `tv` and `len` are valid and
        // `len` gives the size of `tv`; no new value is written.
        let r = unsafe {
            libc::sysctlbyname(c"kern.waketime".as_ptr(), (&raw mut tv).cast(), &mut len, std::ptr::null_mut(), 0)
        };
        if r != 0 || tv.tv_sec <= 0 {
            return None;
        }
        Some(SystemTime::UNIX_EPOCH + Duration::new(tv.tv_sec as u64, tv.tv_usec as u32 * 1000))
    }
}

/// Elsewhere (the core is built for macOS; this keeps `cargo check`
/// working on another system): no sleep is seen.
#[cfg(not(target_vendor = "apple"))]
mod os {
    use std::sync::OnceLock;
    use std::time::{Duration, Instant, SystemTime};

    fn since_start() -> Duration {
        static START: OnceLock<Instant> = OnceLock::new();
        START.get_or_init(Instant::now).elapsed()
    }

    pub(super) fn continuous() -> Duration {
        since_start()
    }

    pub(super) fn awake() -> Duration {
        since_start()
    }

    pub(super) fn wake_time() -> Option<SystemTime> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_continuous_clock_is_never_behind_the_awake_clock() {
        // On macOS the continuous clock counts the sleeps since boot on
        // top: never less than the awake clock.
        let (a, c) = (awake(), now());
        assert!(c + Duration::from_millis(1) >= a, "continuous {c:?}, awake {a:?}");
    }

    #[test]
    fn a_gap_of_a_few_seconds_is_a_sleep_a_drift_is_not() {
        let fifteen = Duration::from_secs(15);
        assert!(!slept_between(fifteen, fifteen));
        assert!(!slept_between(fifteen + Duration::from_millis(30), fifteen), "a drift");
        assert!(!slept_between(fifteen, fifteen + Duration::from_millis(30)), "a drift the other way");
        // The dark wakes of 21 November 2026: sleeps of 9 s to 7 min.
        assert!(slept_between(fifteen + Duration::from_secs(9), fifteen));
        assert!(slept_between(Duration::from_secs(403 + 15), fifteen));
    }

    #[test]
    fn the_last_wake_is_in_the_past() {
        if let Some(d) = since_wake() {
            // Not more than the time since boot.
            assert!(d <= now() + Duration::from_secs(1), "{d:?}");
        }
    }

    #[tokio::test]
    async fn a_wait_lasts_its_time_on_the_continuous_clock() {
        let start = now();
        sleep(Duration::from_millis(120)).await;
        assert!(now() - start >= Duration::from_millis(120));
    }
}
