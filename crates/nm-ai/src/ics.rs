//! Calendar files of the appointments a mail mentions (`specifications/ia.md`,
//! section 8, "Dates et rendez-vous"): the dates the model gives, checked,
//! and the iCalendar text (RFC 5545) of one event, for the user's calendar.
//!
//! Everything here is pure: no clock (the caller passes the time of the
//! stamp), no file (the app writes it, since the core has no access to the
//! user's Downloads, `nm_store::outgoing`). No date library either: the
//! few computations needed (validity of a day, a day after, an hour after)
//! are a handful of lines, tested below.
//!
//! # Floating times
//!
//! A time is written without a time zone (RFC 5545, section 3.3.5, "floating
//! time"): the calendar reads 14:00 as 14:00 in the zone the user is in.
//! That is what a mail means almost always ("see you Thursday at 2pm"),
//! and the model cannot tell a zone the mail does not name. A meeting set
//! in another zone is the exception; the event's details say how the model
//! read the time, and the user can fix it in the calendar.

use std::fmt::Write as _;

/// A day of the Gregorian calendar, checked (`Date::parse`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Date {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

/// When an event starts or ends: a whole day, or a time of a day.
///
/// Rust note: deriving `PartialOrd`/`Ord` on an enum compares the variant
/// first, then the fields in order; that is why [`When::is_after`] compares
/// two values of the same kind only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum When {
    /// `2026-11-18`: an all-day event.
    Day(Date),
    /// `2026-11-18T14:00`: hour and minute, floating (module documentation).
    At(Date, u32, u32),
}

impl Date {
    /// `YYYY-MM-DD`, a day that exists (no 31 April, 29 February only in a
    /// leap year).
    pub fn parse(s: &str) -> Option<Date> {
        let b = s.as_bytes();
        if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
            return None;
        }
        let year: i32 = digits(&s[0..4])?;
        let month: u32 = digits(&s[5..7])?;
        let day: u32 = digits(&s[8..10])?;
        let d = Date { year, month, day };
        ((1..=12).contains(&month) && (1..=days_in_month(year, month)).contains(&day)).then_some(d)
    }

    /// The day after.
    pub fn next(self) -> Date {
        if self.day < days_in_month(self.year, self.month) {
            Date { day: self.day + 1, ..self }
        } else if self.month < 12 {
            Date { month: self.month + 1, day: 1, ..self }
        } else {
            Date { year: self.year + 1, month: 1, day: 1 }
        }
    }

    /// `YYYYMMDD`, the form of an iCalendar date.
    fn compact(self) -> String {
        format!("{:04}{:02}{:02}", self.year, self.month, self.day)
    }

    /// `YYYY-MM-DD`.
    pub fn iso(self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

impl When {
    /// `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM` (24-hour clock). Seconds and time
    /// zones are refused: the prompt does not ask for them, and a model
    /// that adds them did not follow it.
    pub fn parse(s: &str) -> Option<When> {
        match s.split_once('T') {
            None => Date::parse(s).map(When::Day),
            Some((day, time)) => {
                let b = time.as_bytes();
                if b.len() != 5 || b[2] != b':' {
                    return None;
                }
                let (h, m): (u32, u32) = (digits(&time[0..2])?, digits(&time[3..5])?);
                (h < 24 && m < 60).then_some(When::At(Date::parse(day)?, h, m))
            }
        }
    }

    /// The form the model and the clients use: `YYYY-MM-DD[THH:MM]`.
    pub fn iso(self) -> String {
        match self {
            When::Day(d) => d.iso(),
            When::At(d, h, m) => format!("{}T{h:02}:{m:02}", d.iso()),
        }
    }

    pub fn date(self) -> Date {
        match self {
            When::Day(d) | When::At(d, _, _) => d,
        }
    }

    /// Whether `self` comes after `other`, both of the same kind; `None`
    /// when one is a day and the other a time (an end must be of the kind
    /// of its start).
    pub fn is_after(self, other: When) -> Option<bool> {
        match (self, other) {
            (When::Day(a), When::Day(b)) => Some(a > b),
            (When::At(a, ah, am), When::At(b, bh, bm)) => Some((a, ah, am) > (b, bh, bm)),
            _ => None,
        }
    }
}

/// An appointment, as the model gave it and the core checked it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// What happens, in a few words.
    pub title: String,
    pub start: When,
    /// Of the kind of `start`, not before it. A whole-day end is the last
    /// day included (a three-day fair: start the 18th, end the 20th).
    pub end: Option<When>,
    /// Empty when the mail gives none.
    pub location: String,
    /// How the model read the date ("Thursday: the 20th, the day after the
    /// mail"), who is there: empty when there is nothing to add.
    pub details: String,
}

/// How long a timed event without an end lasts in the calendar: an hour,
/// what Calendar proposes for a new event. A zero-length event (RFC 5545's
/// reading of a start alone) shows as a line nobody can click.
const DEFAULT_MINUTES: u32 = 60;

/// The iCalendar text of one event: a `VCALENDAR` with one `VEVENT`, lines
/// ended by CRLF and folded at 75 bytes, as RFC 5545 requires.
///
/// - `uid` identifies the event: the calendar updates an event it already
///   has instead of adding it twice, so the same run and index give the
///   same uid (`nicemailer-ai-<run>-<n>@nicemailer`).
/// - `stamp` is when the file is made, in Unix seconds (`DTSTAMP`, UTC).
/// - `mail` is the link of the mail the event comes from
///   (`nicemailer://m/<key>`): the event's `URL`, which Calendar shows as a
///   link, and the last line of its description, for the calendars that
///   do not show `URL` (Google Calendar).
pub fn calendar(event: &Event, uid: &str, stamp: i64, mail: Option<&str>) -> String {
    let mut lines: Vec<String> = vec![
        "BEGIN:VCALENDAR".into(),
        "VERSION:2.0".into(),
        "PRODID:-//NiceMailer//AI dates//EN".into(),
        "CALSCALE:GREGORIAN".into(),
        "METHOD:PUBLISH".into(),
        "BEGIN:VEVENT".into(),
        format!("UID:{}", escape(uid)),
        format!("DTSTAMP:{}", utc_stamp(stamp)),
    ];
    match (event.start, event.end) {
        (When::Day(start), end) => {
            // The end of a whole-day event is exclusive in iCalendar: the
            // day after the last one.
            let last = match end {
                Some(When::Day(d)) => d,
                _ => start,
            };
            lines.push(format!("DTSTART;VALUE=DATE:{}", start.compact()));
            lines.push(format!("DTEND;VALUE=DATE:{}", last.next().compact()));
        }
        (When::At(d, h, m), end) => {
            let end = match end {
                Some(e @ When::At(..)) => e,
                _ => add_minutes(d, h, m, DEFAULT_MINUTES),
            };
            lines.push(format!("DTSTART:{}", local(event.start)));
            lines.push(format!("DTEND:{}", local(end)));
        }
    }
    lines.push(format!("SUMMARY:{}", escape(&event.title)));
    if !event.location.is_empty() {
        lines.push(format!("LOCATION:{}", escape(&event.location)));
    }
    let description = match mail {
        Some(link) if event.details.is_empty() => link.to_string(),
        Some(link) => format!("{}\n\n{link}", event.details),
        None => event.details.clone(),
    };
    if !description.is_empty() {
        lines.push(format!("DESCRIPTION:{}", escape(&description)));
    }
    if let Some(link) = mail {
        // A URI value: not escaped as a text (RFC 5545, section 3.3.13).
        lines.push(format!("URL:{link}"));
    }
    lines.push("END:VEVENT".into());
    lines.push("END:VCALENDAR".into());
    let mut out = String::new();
    for l in &lines {
        out.push_str(&fold(l));
        out.push_str("\r\n");
    }
    out
}

/// The file name of an event: `2026-11-18 Visite de l'appartement.ics`,
/// without the characters a file name cannot hold, the title cut at 60
/// characters. The app adds " (2)" when the file exists.
pub fn file_name(event: &Event) -> String {
    let title = file_stem(&event.title);
    let date = event.start.date().iso();
    if title.is_empty() { format!("{date}.ics") } else { format!("{date} {title}.ics") }
}

/// A text made fit for a file name: the characters a file name cannot
/// hold (or that Finder shows oddly) turned into spaces, runs of spaces
/// made one, cut at 60 characters, no leading dot (the file would be
/// hidden). Shared with the contact cards (`vcard::file_name`).
pub(crate) fn file_stem(text: &str) -> String {
    let clean: String = text
        .chars()
        .map(|c| if c.is_control() || "/\\:*?\"<>|".contains(c) { ' ' } else { c })
        .collect();
    let clean: String = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    let clean: String = clean.chars().take(60).collect();
    clean.trim_start_matches('.').trim().to_string()
}

/// The text of a property value: backslash, semicolon, comma and line
/// break escaped (RFC 5545, section 3.3.11). A carriage return is dropped:
/// a CRLF in the text is one line break.
pub(crate) fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            c => out.push(c),
        }
    }
    out
}

/// A content line folded at 75 bytes (RFC 5545, section 3.1): the rest
/// goes on the next line after a CRLF and one space. A cut never splits a
/// UTF-8 character, which would make the file unreadable.
pub(crate) fn fold(line: &str) -> String {
    let mut out = String::with_capacity(line.len() + line.len() / 70 * 3);
    let mut width = 0;
    for c in line.chars() {
        let n = c.len_utf8();
        // 75 bytes on the first line; the continuation lines start with a
        // space, which counts.
        if width + n > 75 {
            out.push_str("\r\n ");
            width = 1;
        }
        out.push(c);
        width += n;
    }
    out
}

/// `YYYYMMDDTHHMMSS`, floating (no zone).
fn local(w: When) -> String {
    match w {
        When::At(d, h, m) => format!("{}T{h:02}{m:02}00", d.compact()),
        When::Day(d) => d.compact(),
    }
}

/// `minutes` after a time, the day changing past midnight.
fn add_minutes(d: Date, h: u32, m: u32, minutes: u32) -> When {
    let total = h * 60 + m + minutes;
    let mut day = d;
    for _ in 0..total / (24 * 60) {
        day = day.next();
    }
    let rest = total % (24 * 60);
    When::At(day, rest / 60, rest % 60)
}

/// Unix seconds as `YYYYMMDDTHHMMSSZ`.
pub(crate) fn utc_stamp(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rest = secs.rem_euclid(86_400);
    let (y, mo, d) = civil_from_days(days);
    let mut s = String::new();
    // Rust note: `write!` into a `String` needs `std::fmt::Write` in scope
    // (imported `as _`: only its methods are wanted, not its name).
    let _ = write!(s, "{y:04}{mo:02}{d:02}T{:02}{:02}{:02}Z", rest / 3600, rest % 3600 / 60, rest % 60);
    s
}

/// The Gregorian day of a count of days since 1970-01-01 (Howard Hinnant's
/// `civil_from_days`, valid for every day a mail may name).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

fn is_leap(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(y) => 29,
        2 => 28,
        _ => 31,
    }
}

/// A number written with ASCII digits only: `"+1"` or `" 1"` are refused,
/// which `str::parse` alone would let through for some types.
fn digits<T: std::str::FromStr>(s: &str) -> Option<T> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(s: &str) -> Date {
        Date::parse(s).unwrap()
    }

    fn event(start: &str, end: Option<&str>) -> Event {
        Event {
            title: "Visite".into(),
            start: When::parse(start).unwrap(),
            end: end.map(|e| When::parse(e).unwrap()),
            location: String::new(),
            details: String::new(),
        }
    }

    #[test]
    fn only_real_days_and_times_parse() {
        assert_eq!(When::parse("2026-11-18"), Some(When::Day(day("2026-11-18"))));
        assert_eq!(When::parse("2026-11-18T09:05"), Some(When::At(day("2026-11-18"), 9, 5)));
        assert!(Date::parse("2028-02-29").is_some(), "a leap year");
        for bad in [
            "2026-02-29", "2100-02-29", "2026-04-31", "2026-13-01", "2026-00-10", "2026-1-18", "18/11/2026",
            "2026-11-18T24:00", "2026-11-18T14:60", "2026-11-18T14:00:00", "2026-11-18T14:00Z", "2026-11-18 14:00",
            "+026-11-18", "", "jeudi",
        ] {
            assert_eq!(When::parse(bad), None, "{bad}");
        }
        assert!(Date::parse("2000-02-29").is_some(), "every 400 years");
    }

    #[test]
    fn the_next_day_crosses_months_and_years() {
        assert_eq!(day("2026-11-18").next(), day("2026-11-19"));
        assert_eq!(day("2026-11-30").next(), day("2026-12-01"));
        assert_eq!(day("2026-12-31").next(), day("2027-01-01"));
        assert_eq!(day("2028-02-28").next(), day("2028-02-29"));
        assert_eq!(day("2026-02-28").next(), day("2026-03-01"));
    }

    #[test]
    fn an_end_compares_with_a_start_of_its_kind() {
        let (a, b) = (When::parse("2026-11-18T14:00").unwrap(), When::parse("2026-11-18T15:30").unwrap());
        assert_eq!(b.is_after(a), Some(true));
        assert_eq!(a.is_after(b), Some(false));
        assert_eq!(a.is_after(a), Some(false));
        assert_eq!(When::parse("2026-11-19").unwrap().is_after(When::parse("2026-11-18").unwrap()), Some(true));
        assert_eq!(a.is_after(When::parse("2026-11-18").unwrap()), None);
    }

    #[test]
    fn a_timed_event_is_floating_and_lasts_an_hour_without_an_end() {
        let ics = calendar(&event("2026-11-18T14:00", None), "nicemailer-ai-7-1@nicemailer", 1_763_000_000, None);
        assert!(ics.contains("\r\nDTSTART:20261118T140000\r\n"), "{ics}");
        assert!(ics.contains("\r\nDTEND:20261118T150000\r\n"), "{ics}");
        // No zone: neither TZID nor Z on the times of the event.
        assert!(!ics.contains("TZID") && !ics.contains("T140000Z"), "{ics}");
        assert!(ics.starts_with("BEGIN:VCALENDAR\r\nVERSION:2.0\r\n"), "{ics}");
        assert!(ics.ends_with("END:VEVENT\r\nEND:VCALENDAR\r\n"), "{ics}");
        assert!(ics.contains("\r\nUID:nicemailer-ai-7-1@nicemailer\r\n"), "{ics}");
        // 1_763_000_000 is 2025-11-13 02:13:20 UTC.
        assert!(ics.contains("\r\nDTSTAMP:20251113T021320Z\r\n"), "{ics}");
        // Every line ends with CRLF, none with a bare LF.
        assert_eq!(ics.matches('\n').count(), ics.matches("\r\n").count());
    }

    #[test]
    fn an_hour_late_in_the_evening_ends_the_next_day() {
        let ics = calendar(&event("2026-12-31T23:30", None), "u", 0, None);
        assert!(ics.contains("DTEND:20270101T003000"), "{ics}");
        let ics = calendar(&event("2026-11-18T14:00", Some("2026-11-18T16:15")), "u", 0, None);
        assert!(ics.contains("DTEND:20261118T161500"), "{ics}");
    }

    #[test]
    fn a_whole_day_ends_the_day_after_its_last() {
        let ics = calendar(&event("2026-11-18", None), "u", 0, None);
        assert!(ics.contains("\r\nDTSTART;VALUE=DATE:20261118\r\nDTEND;VALUE=DATE:20261119\r\n"), "{ics}");
        let ics = calendar(&event("2026-11-28", Some("2026-11-30")), "u", 0, None);
        assert!(ics.contains("DTEND;VALUE=DATE:20261201"), "{ics}");
        assert!(ics.contains("DTSTAMP:19700101T000000Z"), "{ics}");
    }

    #[test]
    fn texts_are_escaped_and_long_lines_folded_whole_characters() {
        let mut e = event("2026-11-18T14:00", None);
        e.title = "Réunion; budget, \\ v2".into();
        e.location = "12 rue de la Paix, Paris".into();
        e.details = "Jeudi : le 20.\r\nApporter les clés.".into();
        let ics = calendar(&e, "u", 0, None);
        assert!(ics.contains("SUMMARY:Réunion\\; budget\\, \\\\ v2\r\n"), "{ics}");
        assert!(ics.contains("LOCATION:12 rue de la Paix\\, Paris\r\n"), "{ics}");
        assert!(ics.contains("DESCRIPTION:Jeudi : le 20.\\nApporter les clés.\r\n"), "{ics}");
        // Empty fields are left out.
        let ics = calendar(&event("2026-11-18", None), "u", 0, None);
        assert!(!ics.contains("LOCATION") && !ics.contains("DESCRIPTION"), "{ics}");
        // A long title of two-byte letters: no line over 75 bytes, every
        // continuation starts with a space, and unfolding gives it back.
        e.title = "é".repeat(100);
        let ics = calendar(&e, "u", 0, None);
        for line in ics.split("\r\n") {
            assert!(line.len() <= 75, "{} bytes: {line}", line.len());
        }
        assert!(ics.replace("\r\n ", "").contains(&format!("SUMMARY:{}\r\n", "é".repeat(100))), "{ics}");
    }

    #[test]
    fn the_mail_is_linked_from_the_event() {
        let mut e = event("2026-11-18T14:00", None);
        let ics = calendar(&e, "u", 0, Some("nicemailer://m/399914"));
        assert!(ics.contains("\r\nDESCRIPTION:nicemailer://m/399914\r\nURL:nicemailer://m/399914\r\n"), "{ics}");
        e.details = "Jeudi : le 20.".into();
        let ics = calendar(&e, "u", 0, Some("nicemailer://m/399914"));
        assert!(ics.contains("DESCRIPTION:Jeudi : le 20.\\n\\nnicemailer://m/399914\r\n"), "{ics}");
        assert!(ics.contains("\r\nURL:nicemailer://m/399914\r\n"), "{ics}");
    }

    #[test]
    fn a_file_name_is_dated_and_safe() {
        let mut e = event("2026-11-18T14:00", None);
        e.title = "Visite : appart/cave".into();
        assert_eq!(file_name(&e), "2026-11-18 Visite appart cave.ics");
        e.title = "..\n".into();
        assert_eq!(file_name(&e), "2026-11-18.ics");
        e.title = "a".repeat(100);
        assert_eq!(file_name(&e), format!("2026-11-18 {}.ics", "a".repeat(60)));
    }
}
