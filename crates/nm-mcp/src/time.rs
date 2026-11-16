//! Dates for the agent (`specifications/mcp.md`, section 7, "Forme"): ISO
//! 8601 at local time with the offset, `2026-11-16T09:42:00+01:00`, rather
//! than Unix seconds. A model reads such a date without computing, and
//! makes fewer mistakes with it.
//!
//! The core speaks Unix seconds. This module converts both ways, with the
//! calendar computed here (Howard Hinnant's `days_from_civil` and
//! `civil_from_days`) and only the local offset asked to the system
//! (`localtime_r`): no date crate for two conversions.

/// The offset of local time from UTC at the Unix time `t`, in seconds
/// (east positive: Paris in winter is 3600).
pub fn local_offset(t: i64) -> i32 {
    // Rust note: `libc::tm` is a plain C struct; `zeroed` fills it with
    // zeros, a valid value for it. `localtime_r` is the thread-safe
    // version of `localtime`: it writes into our struct, not a global one.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let time = t as libc::time_t;
    let ok = unsafe { !libc::localtime_r(&time, &mut tm).is_null() };
    if ok { tm.tm_gmtoff as i32 } else { 0 }
}

/// `t` as an ISO 8601 date at local time, with its offset.
pub fn iso(t: i64) -> String {
    iso_at(t, local_offset(t))
}

/// `t` as an ISO 8601 date at the offset `offset` (seconds east of UTC). A
/// pure function, tested without depending on the machine's time zone.
pub fn iso_at(t: i64, offset: i32) -> String {
    let local = t + i64::from(offset);
    let (days, secs) = (local.div_euclid(86_400), local.rem_euclid(86_400));
    let (y, m, d) = civil_from_days(days);
    let sign = if offset < 0 { '-' } else { '+' };
    let off = offset.unsigned_abs();
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}{sign}{:02}:{:02}",
        secs / 3600,
        secs % 3600 / 60,
        secs % 60,
        off / 3600,
        off % 3600 / 60
    )
}

/// A date given by the agent, as Unix seconds: `2026-11-16` (local
/// midnight), `2026-11-16T09:42` or `…T09:42:00`, at local time, or with an
/// offset (`+01:00`, `-05:00`, `Z`). Fractions of a second are ignored.
pub fn parse(s: &str) -> Result<i64, String> {
    parse_with(s, local_offset)
}

/// [`parse`], with the local offset given by `offset_at`, for the tests.
pub fn parse_with(s: &str, offset_at: impl Fn(i64) -> i32) -> Result<i64, String> {
    let bad = || format!("{s:?} is not a date: expected 2026-11-16 or 2026-11-16T09:42:00+01:00");
    let s = s.trim();
    let (date, rest) = match s.split_once(['T', ' ']) {
        Some((d, r)) => (d, Some(r)),
        None => (s, None),
    };
    let mut parts = date.split('-');
    let num = |p: Option<&str>, len: usize| -> Result<i64, String> {
        match p {
            Some(p) if p.len() == len && p.bytes().all(|b| b.is_ascii_digit()) => Ok(p.parse().map_err(|_| bad())?),
            _ => Err(bad()),
        }
    };
    let (y, m, d) = (num(parts.next(), 4)?, num(parts.next(), 2)?, num(parts.next(), 2)?);
    if parts.next().is_some() || !(1..=12).contains(&m) || d < 1 || d > days_in_month(y, m) {
        return Err(bad());
    }
    let (mut secs, mut offset): (i64, Option<i64>) = (0, None);
    if let Some(rest) = rest {
        // The offset: `Z`, or a sign after the time.
        let (clock, off) = if let Some(c) = rest.strip_suffix(['Z', 'z']) {
            (c, Some(0))
        } else if let Some(i) = rest.rfind(['+', '-']) {
            let (c, o) = rest.split_at(i);
            let sign = if o.starts_with('-') { -1 } else { 1 };
            let o = &o[1..];
            let (oh, om) = o.split_once(':').unwrap_or((o, "00"));
            let (oh, om) = (num(Some(oh), 2)?, num(Some(om), 2)?);
            (c, Some(sign * (oh * 3600 + om * 60)))
        } else {
            (rest, None)
        };
        offset = off;
        let clock = clock.split('.').next().unwrap_or(clock);
        let mut c = clock.split(':');
        let h = num(c.next(), 2)?;
        let min = num(c.next(), 2)?;
        let sec = match c.next() {
            Some(x) => num(Some(x), 2)?,
            None => 0,
        };
        if c.next().is_some() || h > 23 || min > 59 || sec > 60 {
            return Err(bad());
        }
        secs = h * 3600 + min * 60 + sec;
    }
    let naive = days_from_civil(y, m, d) * 86_400 + secs;
    Ok(match offset {
        Some(o) => naive - o,
        // Local time: the offset of the moment itself, found in two steps
        // (the offset at the naive time, then at the time it gives), which
        // is right but for the hour that a change of time skips or repeats.
        None => {
            let first = naive - i64::from(offset_at(naive));
            naive - i64::from(offset_at(first))
        }
    })
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        2 if is_leap(y) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days since 1970-01-01 of a date of the proleptic Gregorian calendar.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The date of a number of days since 1970-01-01.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_written_with_their_offset() {
        // 2026-11-16 08:42:00 UTC.
        let t = 1_794_818_520;
        assert_eq!(iso_at(t, 3600), "2026-11-16T09:42:00+01:00");
        assert_eq!(iso_at(t, 0), "2026-11-16T08:42:00+00:00");
        assert_eq!(iso_at(t, -5 * 3600 - 1800), "2026-11-16T03:12:00-05:30");
        assert_eq!(iso_at(0, 0), "1970-01-01T00:00:00+00:00");
        // A leap day, and the day before the epoch.
        assert_eq!(iso_at(951_782_400, 0), "2000-02-29T00:00:00+00:00");
        assert_eq!(iso_at(-1, 0), "1969-12-31T23:59:59+00:00");
    }

    #[test]
    fn dates_given_by_the_agent_are_read_back() {
        let paris = |_| 3600;
        assert_eq!(parse_with("2026-11-16T09:42:00+01:00", paris), Ok(1_794_818_520));
        assert_eq!(parse_with("2026-11-16T08:42:00Z", paris), Ok(1_794_818_520));
        assert_eq!(parse_with("2026-11-16T03:12:00.250-05:30", paris), Ok(1_794_818_520));
        // Local time: the offset of the machine.
        assert_eq!(parse_with("2026-11-16T09:42", paris), Ok(1_794_818_520));
        assert_eq!(parse_with("2026-11-16 09:42:00", paris), Ok(1_794_818_520));
        assert_eq!(parse_with("2026-11-16", paris), Ok(1_794_818_520 - 9 * 3600 - 42 * 60));
        // Every day of a few years, both ways.
        for day in -800..20_000 {
            let t = day * 86_400 + 12 * 3600;
            assert_eq!(parse_with(&iso_at(t, 0), paris), Ok(t), "{}", iso_at(t, 0));
        }
        for bad in ["", "16/11/2026", "2026-13-01", "2026-02-30", "2026-11-16T25:00", "2026-11-16Tnoon", "2026-1-16"] {
            assert!(parse_with(bad, paris).is_err(), "{bad}");
        }
        assert_eq!(parse_with("2024-02-29", |_| 0), Ok(1_709_164_800));
        assert!(parse_with("2025-02-29", |_| 0).is_err());
    }

    #[test]
    fn the_local_offset_is_the_systems() {
        // Whatever the zone of the machine, the date written is the same
        // instant as the one read back.
        let t = 1_794_818_520;
        assert_eq!(parse(&iso(t)), Ok(t));
    }
}
