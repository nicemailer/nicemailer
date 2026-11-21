//! The pace of the connections of an account: how often it may log in, and
//! how it keeps a connection open between passes (`docs/sync.md`, "Pace of
//! the connections").
//!
//! A server sees every login. One that sees too many locks the account:
//! Orange did, on 8 November 2026, after NiceMailer logged in about once a
//! minute for twenty minutes, and the user could no longer read their mail
//! anywhere, webmail and phone included. The cause was ordinary: the
//! server closes a connection that stays in IDLE for about 50 seconds
//! (RFC 2177 lets a client wait 29 minutes), and every cut was followed by
//! a new connection 5 s later, since the pass made on it succeeded and
//! reset the delay. Three rules make that impossible:
//!
//! - **A budget of logins** ([`LoginBudget`]): at most
//!   [`LOGINS_PER_HOUR`] per account over any hour, both connections
//!   together, whatever happens. Past it, the next one waits.
//! - **A delay that a short connection does not reset**: the worker resets
//!   its delay only once a connection has lived [`STABLE`]; a connection
//!   that dies soon after a pass that worked counts as a failure.
//! - **IDLE adapted to the server** ([`IdleHold`]): a connection cut while
//!   in IDLE teaches how long the server keeps one; IDLE is then renewed
//!   at half that time. If the server cuts it even so, the account is
//!   polled instead: one connection per [`POLL_FLOOR`] at most, closed
//!   after its pass.
//!
//! The sleeps of the Mac (21 November 2026) added a fourth: **no login
//! within [`WAKE_SETTLE`] of a wake** ([`after_wake`]). A Mac left alone
//! sleeps and wakes every few minutes, often for a few seconds only (a
//! "dark wake", for the network): a login then is spent for a connection
//! that dies with the next sleep.
//!
//! Everything here is pure (no network, no clock of its own: the callers
//! give `now`, read on the continuous clock of `clock.rs`, which counts
//! the sleeps of the Mac), so the rules are tested on their own.

use std::collections::VecDeque;
use std::time::Duration;

/// Logins allowed per account over any hour, both connections together.
/// A working account needs two per start, and none per wake of the Mac
/// as long as its connections survive the sleep (`account.rs`), one
/// otherwise; an account polled ([`IdleHold::Poll`]) six per hour.
pub(crate) const LOGINS_PER_HOUR: usize = 10;
const HOUR: Duration = Duration::from_secs(3600);
/// A connection that lived this long was a good one: the delay between
/// two attempts goes back to its minimum.
pub(crate) const STABLE: Duration = Duration::from_secs(10 * 60);
/// IDLE is never renewed more often than this: a server that cuts sooner
/// is polled.
pub(crate) const RENEW_FLOOR: Duration = Duration::from_secs(5);
/// A polled account connects at most this often, whatever
/// `sync.poll_interval_secs` says.
pub(crate) const POLL_FLOOR: Duration = Duration::from_secs(10 * 60);
/// No login before the Mac has been awake this long. The dark wakes seen
/// on 21 November 2026 lasted 5 to 6 s: 15 s lets them pass, and delays
/// the mail of a real wake by 15 s at most, only when its connection did
/// not survive the sleep.
pub(crate) const WAKE_SETTLE: Duration = Duration::from_secs(15);

/// The logins of the last hour, as moments of the continuous clock
/// (`clock::now`): an hour while the Mac sleeps is an hour too, for the
/// server.
#[derive(Debug, Default)]
pub(crate) struct LoginBudget {
    logins: VecDeque<Duration>,
}

impl LoginBudget {
    /// How long to wait before the next login: zero while the budget
    /// allows one, else until the oldest login of the hour leaves it.
    pub(crate) fn wait(&self, now: Duration) -> Duration {
        let recent = self.logins.iter().filter(|t| now.saturating_sub(**t) < HOUR).count();
        if recent < LOGINS_PER_HOUR {
            return Duration::ZERO;
        }
        // The oldest of the hour: once it is an hour old, one is allowed.
        let oldest = self.logins.iter().copied().filter(|t| now.saturating_sub(*t) < HOUR).min().unwrap_or(now);
        (oldest + HOUR).saturating_sub(now)
    }

    /// Records a login (an attempt counts: the server sees it).
    pub(crate) fn record(&mut self, now: Duration) {
        self.logins.push_back(now);
        while self.logins.front().is_some_and(|t| now.saturating_sub(*t) >= HOUR) {
            self.logins.pop_front();
        }
    }
}

/// How long a login waits for the Mac to have been awake
/// [`WAKE_SETTLE`], given the time since its last wake (`None`: it never
/// slept since boot).
pub(crate) fn after_wake(since_wake: Option<Duration>) -> Duration {
    since_wake.map_or(Duration::ZERO, |d| WAKE_SETTLE.saturating_sub(d))
}

/// How the account waits between two passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IdleHold {
    /// IDLE, renewed only at the client's own limit (`IDLE_MAX`): the
    /// server has not cut one yet.
    Unknown,
    /// IDLE, ended and started again at this interval: the server cuts a
    /// connection left in IDLE longer.
    Renew(Duration),
    /// No connection kept: one per pass, closed after it.
    Poll,
}

impl IdleHold {
    /// The connection was cut after `idled` in IDLE: the next way to wait.
    pub(crate) fn after_cut(self, idled: Duration) -> IdleHold {
        match self {
            // Half the time the server allowed, so that a server whose
            // limit varies (Orange cut after 47 s, then 56 s) is renewed in
            // time.
            IdleHold::Unknown if idled / 2 >= RENEW_FLOOR => IdleHold::Renew(idled / 2),
            // Cut sooner than the floor allows to renew, or cut although
            // renewed: keeping a connection does not work with this server.
            _ => IdleHold::Poll,
        }
    }

    /// The longest IDLE command before it is renewed, if shorter than the
    /// client's own limit.
    pub(crate) fn renew_every(self) -> Option<Duration> {
        match self {
            IdleHold::Renew(d) => Some(d),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_budget_allows_ten_logins_an_hour_then_waits() {
        let t0 = Duration::from_secs(1000);
        let mut b = LoginBudget::default();
        for i in 0..LOGINS_PER_HOUR {
            let now = t0 + Duration::from_secs(i as u64 * 60);
            assert_eq!(b.wait(now), Duration::ZERO, "login {i}");
            b.record(now);
        }
        // The eleventh, at 10 min: waits until the first is an hour old.
        let now = t0 + Duration::from_secs(600);
        assert_eq!(b.wait(now), Duration::from_secs(3000));
        // Then one is allowed again, and only one.
        let later = t0 + HOUR;
        assert_eq!(b.wait(later), Duration::ZERO);
        b.record(later);
        assert_eq!(b.wait(later), Duration::from_secs(60), "the second login of the hour leaves a minute later");
    }

    #[test]
    fn a_login_per_minute_is_impossible() {
        // What locked the Orange account: a login every minute for an
        // hour. With the budget, ten at most.
        let t0 = Duration::from_secs(1000);
        let mut b = LoginBudget::default();
        let mut now = t0;
        let mut logins = 0;
        while now < t0 + HOUR {
            now += b.wait(now).max(Duration::from_secs(60));
            if now < t0 + HOUR {
                b.record(now);
                logins += 1;
            }
        }
        assert!(logins <= LOGINS_PER_HOUR, "{logins}");
    }

    #[test]
    fn an_hour_asleep_counts_in_the_budget() {
        // 21 November 2026: ten logins in twenty minutes of wakes, then
        // the Mac sleeps an hour and ten minutes. The moments are read on
        // the continuous clock, which went on: the budget is free again
        // at the wake. (On the awake clock, which stopped, the logins were
        // still "twenty minutes old", and the wait went on for forty more
        // minutes of awake time.)
        let t0 = Duration::from_secs(1000);
        let mut b = LoginBudget::default();
        for i in 0..LOGINS_PER_HOUR {
            b.record(t0 + Duration::from_secs(i as u64 * 120));
        }
        assert!(b.wait(t0 + Duration::from_secs(20 * 60)) > Duration::ZERO, "spent");
        let woke = t0 + Duration::from_secs(20 * 60 + 70 * 60);
        assert_eq!(b.wait(woke), Duration::ZERO);
    }

    #[test]
    fn no_login_within_fifteen_seconds_of_a_wake() {
        assert_eq!(after_wake(None), Duration::ZERO, "never slept");
        assert_eq!(after_wake(Some(Duration::ZERO)), WAKE_SETTLE);
        // A dark wake of 6 s ends before the login is made.
        assert_eq!(after_wake(Some(Duration::from_secs(6))), Duration::from_secs(9));
        assert_eq!(after_wake(Some(WAKE_SETTLE)), Duration::ZERO);
        assert_eq!(after_wake(Some(Duration::from_secs(3600))), Duration::ZERO);
    }

    #[test]
    fn a_cut_in_idle_teaches_the_renewal_then_polling() {
        // Orange: cut after 47 s, renewed every 23.5 s.
        let hold = IdleHold::Unknown.after_cut(Duration::from_secs(47));
        assert_eq!(hold, IdleHold::Renew(Duration::from_millis(23_500)));
        assert_eq!(hold.renew_every(), Some(Duration::from_millis(23_500)));
        // Cut again although renewed: renewing does not help, poll.
        assert_eq!(hold.after_cut(Duration::from_secs(50)), IdleHold::Poll);
        assert_eq!(IdleHold::Poll.after_cut(Duration::from_secs(600)), IdleHold::Poll);
        // A server that cuts within seconds: poll at once.
        assert_eq!(IdleHold::Unknown.after_cut(Duration::from_secs(8)), IdleHold::Poll);
        assert_eq!(IdleHold::Unknown.renew_every(), None);
        assert_eq!(IdleHold::Poll.renew_every(), None);
    }
}
