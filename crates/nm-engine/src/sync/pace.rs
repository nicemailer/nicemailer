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
//! reset the delay. These rules make that impossible:
//!
//! - **A delay that a short connection does not reset**: the worker resets
//!   its delay only once a connection has lived [`STABLE`]; a connection
//!   that dies soon after a pass that worked counts as a failure.
//! - **IDLE adapted to the server** ([`IdleHold`]): a connection cut while
//!   in IDLE teaches how long the server keeps one; IDLE is then renewed
//!   at half that time. If the server cuts it even so, the account is
//!   polled instead: one connection per [`POLL_FLOOR`] at most, closed
//!   after its pass.
//! - **No login within [`WAKE_SETTLE`] of a wake of the Mac**
//!   ([`after_wake`], 21 November 2026). A Mac left alone sleeps and wakes
//!   every few minutes, often for a few seconds only (a "dark wake", for
//!   the network): a login then is spent for a connection that dies with
//!   the next sleep.
//! - **A pace learnt from the server's blocks** ([`LoginPace`]). There is
//!   no limit of logins until the server shows one: IMAP has no way to
//!   announce how many logins a server accepts, so a fixed limit is either
//!   too low for a generous server or too high for a strict one. At the
//!   first block ([`classify`]: refused "for now", the password it
//!   accepted refused, turned away before the login three times in a
//!   row), the account pauses, and from then on logs in at most half as
//!   often as it did in the last hour. What is learnt is kept in the
//!   database (`account.login_*`): a restart of the core does not undo a
//!   pause, nor teach the limit again at the cost of another block.
//!
//! Until 21 November 2026 the pace was a fixed budget of ten logins an
//! hour, for every server.
//!
//! The limit is learnt only downwards: a block shows a limit was crossed,
//! nothing shows how much room is left below it. It is forgotten
//! [`FORGET_AFTER`] after the last block.
//!
//! The limit protects only once the server has spoken. Orange locked the
//! account without a word first: against such a server, the three other
//! rules are what keeps the logins few.
//!
//! Everything here is pure (no network, no clock of its own: the callers
//! give `now`, read on the continuous clock of `clock.rs`, which counts
//! the sleeps of the Mac), so the rules are tested on their own.

use std::collections::VecDeque;
use std::time::Duration;

use nm_proto::imap::ImapError;

const HOUR: Duration = Duration::from_secs(3600);
/// A learnt limit is never below this many logins an hour: a working
/// account needs two per start (both connections), one more for a
/// connection lost.
pub(crate) const LIMIT_FLOOR: usize = 3;
/// The pause after a first block; doubled at each block that follows
/// without a connection that lived [`STABLE`] in between, up to
/// [`PAUSE_MAX`].
const PAUSE_MIN: Duration = Duration::from_secs(15 * 60);
/// After a password the server accepted is refused: a lock, whose usual
/// length is half an hour or more. Trying sooner may extend it.
const PAUSE_LOCKED: Duration = Duration::from_secs(30 * 60);
const PAUSE_MAX: Duration = Duration::from_secs(4 * 3600);
/// A server that turns the client away before the login (a `BYE`, a
/// connection closed) this many times in a row is taken for one that
/// blocks: once or twice may be the network.
const TURNED_AWAY_COUNT: u32 = 3;
/// A learnt limit is forgotten this long after the last block, in
/// seconds: a server may have changed its rules, and a limit kept for
/// ever would slow a generous server down for good.
pub(crate) const FORGET_AFTER: i64 = 30 * 86_400;
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

/// What the answer to a login attempt says of the server's patience
/// ([`classify`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Signal {
    /// The server says it: a code of RFC 5530 (`[UNAVAILABLE]`, `[LIMIT]`,
    /// `[INUSE]`), or words that mean it ("too many connections", "try
    /// again later"), in a refusal or a `BYE`.
    Said,
    /// The password the server accepted before is refused: most likely a
    /// lock for too many logins. A password changed on the server looks
    /// the same; it is then tried again after the pause, rarely.
    Locked,
    /// A `BYE` or a connection closed before the login, without a word:
    /// counts as a block only from the [`TURNED_AWAY_COUNT`]th in a row.
    TurnedAway,
}

/// The words of the servers that limit logins, lower case: Gmail
/// ("Too many simultaneous connections", "Account exceeded command or
/// bandwidth limits", "Try again later"), Dovecot ("Maximum number of
/// connections from user+IP exceeded"), and the usual ways to say it.
const BLOCK_WORDS: &[&str] = &[
    "too many",
    "maximum number of connections",
    "max connections",
    "rate limit",
    "exceeded",
    "try again later",
    "try later",
    "temporarily",
    "throttl",
    "too fast",
    "too often",
];

fn says_blocked(text: &str) -> bool {
    let text = text.to_lowercase();
    BLOCK_WORDS.iter().any(|w| text.contains(w))
}

/// What a failed connection or login says of the server's patience.
/// `password_worked`: this password logged in before, and was not changed
/// since. Failures of the network (no route, DNS, a time limit, TLS) say
/// nothing: they are not the server's.
pub(crate) fn classify(error: &ImapError, password_worked: bool) -> Option<Signal> {
    match error {
        ImapError::Unavailable { .. } => Some(Signal::Said),
        ImapError::AuthFailed(text) | ImapError::Bye(text) if says_blocked(text) => Some(Signal::Said),
        ImapError::AuthFailed(_) if password_worked => Some(Signal::Locked),
        ImapError::Bye(_) | ImapError::Closed | ImapError::Io(_) => Some(Signal::TurnedAway),
        _ => None,
    }
}

/// A block learnt: what to save and to log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Learnt {
    /// Logins per hour at most, from now on.
    pub(crate) limit: usize,
    /// No login before this long.
    pub(crate) pause: Duration,
}

/// The pace of an account's logins: those of the last hour, as moments of
/// the continuous clock (`clock::now`: an hour while the Mac sleeps is an
/// hour too, for the server), and what its server's blocks taught.
#[derive(Debug, Default)]
pub(crate) struct LoginPace {
    logins: VecDeque<Duration>,
    /// Logins per hour at most; `None` until a block.
    limit: Option<usize>,
    /// No login before this moment.
    pause_until: Option<Duration>,
    /// Blocks since the last connection that lived [`STABLE`]: each one
    /// doubles the pause.
    blocks: u32,
    /// Times in a row the server turned the client away before the login.
    turned_away: u32,
}

impl LoginPace {
    /// The pace saved in the database (Unix seconds), at `unix_now`, the
    /// continuous clock reading `now`. A limit whose last block is older
    /// than [`FORGET_AFTER`] is forgotten.
    pub(crate) fn restore(limit: Option<i64>, blocked_at: Option<i64>, pause_until: Option<i64>, unix_now: i64, now: Duration) -> LoginPace {
        let fresh = blocked_at.is_some_and(|t| unix_now - t < FORGET_AFTER);
        let limit = limit.filter(|_| fresh).map(|l| (l.max(0) as usize).max(LIMIT_FLOOR));
        let pause_until = pause_until.filter(|t| *t > unix_now).map(|t| now + Duration::from_secs((t - unix_now) as u64));
        LoginPace { limit, pause_until, ..LoginPace::default() }
    }

    /// The limit learnt, if any.
    pub(crate) fn limit(&self) -> Option<usize> {
        self.limit
    }

    /// How long to wait before the next login: the pause after a block,
    /// and with a limit, until the oldest login of the hour leaves it.
    pub(crate) fn wait(&self, now: Duration) -> Duration {
        let pause = self.pause_until.map_or(Duration::ZERO, |t| t.saturating_sub(now));
        let Some(limit) = self.limit else { return pause };
        let recent = self.recent(now);
        if recent < limit {
            return pause;
        }
        // The oldest of the hour: once it is an hour old, one is allowed.
        let oldest = self.logins.iter().copied().filter(|t| now.saturating_sub(*t) < HOUR).min().unwrap_or(now);
        pause.max((oldest + HOUR).saturating_sub(now))
    }

    fn recent(&self, now: Duration) -> usize {
        self.logins.iter().filter(|t| now.saturating_sub(**t) < HOUR).count()
    }

    /// Records a login (an attempt counts: the server sees it).
    pub(crate) fn record(&mut self, now: Duration) {
        self.logins.push_back(now);
        while self.logins.front().is_some_and(|t| now.saturating_sub(*t) >= HOUR) {
            self.logins.pop_front();
        }
    }

    /// The login worked: the server no longer turns the client away.
    pub(crate) fn logged_in(&mut self) {
        self.turned_away = 0;
    }

    /// A new password was saved: the pause, taken after a refusal of the
    /// old one, ends. The limit stays: it is the server's.
    pub(crate) fn clear_pause(&mut self) {
        self.pause_until = None;
        self.blocks = 0;
    }

    /// A connection lived [`STABLE`]: the next block pauses from the start
    /// again.
    pub(crate) fn stable(&mut self) {
        self.blocks = 0;
    }

    /// The server showed `signal` at `now`, after the attempt recorded:
    /// the limit and the pause learnt, or `None` when it is not a block
    /// yet (the first times the server turns the client away).
    pub(crate) fn blocked(&mut self, now: Duration, signal: Signal) -> Option<Learnt> {
        // Already paused: the other connection of the account, refused in
        // the same moment, saw the same block. Learnt once.
        if self.pause_until.is_some_and(|t| t > now) {
            return None;
        }
        if signal == Signal::TurnedAway {
            self.turned_away += 1;
            if self.turned_away < TURNED_AWAY_COUNT {
                return None;
            }
        }
        self.turned_away = 0;
        // Half the pace of the last hour, which crossed the server's limit;
        // half the limit already learnt, if the server blocks within it.
        let recent = self.recent(now).max(1);
        let base = self.limit.map_or(recent, |l| l.min(recent));
        let limit = (base / 2).max(LIMIT_FLOOR);
        let first = if signal == Signal::Locked { PAUSE_LOCKED } else { PAUSE_MIN };
        let pause = first.saturating_mul(1 << self.blocks.min(8)).min(PAUSE_MAX);
        self.blocks += 1;
        self.limit = Some(limit);
        self.pause_until = Some(now + pause);
        Some(Learnt { limit, pause })
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

    const T0: Duration = Duration::from_secs(1000);
    const MIN: Duration = Duration::from_secs(60);

    #[test]
    fn without_a_block_there_is_no_limit() {
        // A login a minute for an hour: what locked Orange, allowed by
        // this rule alone. The delay a short connection does not reset
        // keeps it from happening (`account.rs`).
        let mut p = LoginPace::default();
        for i in 0..60 {
            let now = T0 + MIN * i;
            assert_eq!(p.wait(now), Duration::ZERO, "login {i}");
            p.record(now);
        }
        assert_eq!(p.limit(), None);
    }

    #[test]
    fn a_block_pauses_then_halves_the_pace() {
        let mut p = LoginPace::default();
        for i in 0..10 {
            p.record(T0 + MIN * i);
        }
        let now = T0 + MIN * 9;
        let learnt = p.blocked(now, Signal::Said).unwrap();
        assert_eq!(learnt, Learnt { limit: 5, pause: PAUSE_MIN });
        // The pause is 15 min, but the new limit holds longer: ten logins
        // in the hour, five allowed, until the oldest leaves it (51 min).
        assert_eq!(p.wait(now), T0 + HOUR - now);
        // Fewer logins before the block: the pause alone.
        let mut q = LoginPace::default();
        q.record(T0);
        q.blocked(T0, Signal::Said).unwrap();
        assert_eq!(q.wait(T0), PAUSE_MIN);
        assert_eq!(q.wait(T0 + PAUSE_MIN), Duration::ZERO);
        // After the pause: the logins of the hour still count, five at
        // most an hour from now on.
        let later = now + PAUSE_MIN;
        assert!(p.wait(later) > Duration::ZERO, "ten logins in the hour, five allowed");
        let hour_on = T0 + HOUR + MIN * 10;
        assert_eq!(p.wait(hour_on), Duration::ZERO);
        for i in 0..5 {
            p.record(hour_on + MIN * i);
        }
        assert!(p.wait(hour_on + MIN * 5) > Duration::ZERO, "the sixth of the hour waits");
    }

    #[test]
    fn blocks_that_follow_double_the_pause_and_halve_the_limit_down_to_the_floor() {
        let mut p = LoginPace::default();
        let mut now = T0;
        for i in 0..8 {
            p.record(now + MIN * i);
        }
        now += MIN * 8;
        assert_eq!(p.blocked(now, Signal::Said).unwrap(), Learnt { limit: 4, pause: PAUSE_MIN });
        now += PAUSE_MIN;
        p.record(now);
        assert_eq!(p.blocked(now, Signal::Said).unwrap(), Learnt { limit: LIMIT_FLOOR, pause: PAUSE_MIN * 2 });
        now += PAUSE_MIN * 2;
        p.record(now);
        assert_eq!(p.blocked(now, Signal::Said).unwrap().pause, PAUSE_MIN * 4);
        for _ in 0..10 {
            now += PAUSE_MAX;
            p.record(now);
            p.blocked(now, Signal::Said);
        }
        now += PAUSE_MAX;
        assert_eq!(p.blocked(now, Signal::Said).unwrap(), Learnt { limit: LIMIT_FLOOR, pause: PAUSE_MAX });
        // Refused again within the pause (the other connection, at the
        // same moment): learnt once.
        assert_eq!(p.blocked(now, Signal::Said), None);
        // A connection that lived: the next pause starts from the start.
        now += PAUSE_MAX;
        p.stable();
        assert_eq!(p.blocked(now, Signal::Said).unwrap().pause, PAUSE_MIN);
        // A new password saved: no pause any more, the limit stays.
        p.clear_pause();
        assert_eq!(p.wait(now), Duration::ZERO);
        assert_eq!(p.limit(), Some(LIMIT_FLOOR));
    }

    #[test]
    fn a_lock_pauses_longer() {
        let mut p = LoginPace::default();
        p.record(T0);
        assert_eq!(p.blocked(T0, Signal::Locked).unwrap(), Learnt { limit: LIMIT_FLOOR, pause: PAUSE_LOCKED });
    }

    #[test]
    fn turned_away_counts_from_the_third_time_in_a_row() {
        let mut p = LoginPace::default();
        p.record(T0);
        assert_eq!(p.blocked(T0, Signal::TurnedAway), None);
        assert_eq!(p.blocked(T0, Signal::TurnedAway), None);
        // A login that works in between: the count starts again.
        p.logged_in();
        assert_eq!(p.blocked(T0, Signal::TurnedAway), None);
        assert_eq!(p.blocked(T0, Signal::TurnedAway), None);
        assert!(p.blocked(T0, Signal::TurnedAway).is_some());
        assert_eq!(p.wait(T0), PAUSE_MIN);
    }

    #[test]
    fn the_signals_of_the_servers() {
        let unavailable = ImapError::Unavailable { code: "UNAVAILABLE".into(), text: "Temporary System Problem. Try again later".into() };
        assert_eq!(classify(&unavailable, false), Some(Signal::Said));
        // Gmail, Dovecot: said in words, with or without a working password.
        let gmail = ImapError::AuthFailed("[ALERT] Too many simultaneous connections. (Failure)".into());
        assert_eq!(classify(&gmail, false), Some(Signal::Said));
        let dovecot = ImapError::Bye("Maximum number of connections from user+IP exceeded (mail_max_userip_connections=10)".into());
        assert_eq!(classify(&dovecot, true), Some(Signal::Said));
        // A refused password: a lock if it worked, else a wrong password,
        // which the worker never tries again (`auth_failed`).
        let refused = ImapError::AuthFailed("[AUTHENTICATIONFAILED] Authentication failed.".into());
        assert_eq!(classify(&refused, true), Some(Signal::Locked));
        assert_eq!(classify(&refused, false), None);
        // Turned away before the login.
        assert_eq!(classify(&ImapError::Bye("Server shutting down".into()), false), Some(Signal::TurnedAway));
        assert_eq!(classify(&ImapError::Closed, false), Some(Signal::TurnedAway));
        assert_eq!(classify(&ImapError::Io("connection reset by peer".into()), false), Some(Signal::TurnedAway));
        // The network says nothing of the server.
        assert_eq!(classify(&ImapError::Connect("no route to host".into()), true), None);
        assert_eq!(classify(&ImapError::Timeout("greeting"), true), None);
        assert_eq!(classify(&ImapError::Tls("invalid certificate".into()), true), None);
    }

    #[test]
    fn what_was_learnt_survives_a_restart_and_is_forgotten_after_thirty_days() {
        let unix = 1_800_000_000;
        // Blocked an hour ago, paused for two more: both kept.
        let p = LoginPace::restore(Some(4), Some(unix - 3600), Some(unix + 7200), unix, T0);
        assert_eq!(p.limit(), Some(4));
        assert_eq!(p.wait(T0), Duration::from_secs(7200));
        // A pause over: no wait.
        let p = LoginPace::restore(Some(4), Some(unix - 3600), Some(unix - 1), unix, T0);
        assert_eq!(p.wait(T0), Duration::ZERO);
        // Thirty days without a block: no limit any more.
        let p = LoginPace::restore(Some(4), Some(unix - FORGET_AFTER), None, unix, T0);
        assert_eq!(p.limit(), None);
        // Never below the floor, whatever the file says.
        assert_eq!(LoginPace::restore(Some(0), Some(unix), None, unix, T0).limit(), Some(LIMIT_FLOOR));
        assert_eq!(LoginPace::restore(None, None, None, unix, T0).limit(), None);
    }

    #[test]
    fn an_hour_asleep_counts_in_the_limit() {
        // 21 November 2026: the logins are moments of the continuous
        // clock, which goes on while the Mac sleeps. Ten logins in twenty
        // minutes against a limit of five, then an hour and ten minutes
        // asleep: free again at the wake. (On the awake clock, which
        // stopped, they were still "twenty minutes old".)
        let mut p = LoginPace::restore(Some(5), Some(1), None, 2, T0);
        for i in 0..10 {
            p.record(T0 + MIN * (2 * i));
        }
        assert!(p.wait(T0 + MIN * 20) > Duration::ZERO, "spent");
        assert_eq!(p.wait(T0 + MIN * (20 + 70)), Duration::ZERO);
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
