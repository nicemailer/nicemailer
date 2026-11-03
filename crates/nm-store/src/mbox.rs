//! Streaming reader of mbox files (`import-initial.md`, section 9).
//!
//! An mbox is a plain concatenation of messages. Each one starts with a
//! "From_ line" (`From sender Tue Jun 23 19:20:56 2020`) that is not part of
//! the message itself, and messages are separated by an empty line. A body
//! line that starts with `From ` is escaped as `>From ` by the exporter, so
//! it cannot be mistaken for a separator.
//!
//! Variants and how this reader copes with them:
//! - mboxrd (the common one today): every `>*From ` line in the body got one
//!   more `>`. Reading removes exactly one `>` from such lines, which
//!   restores the original bytes.
//! - mboxo (older): only `From ` was escaped, so `>From ` in the original
//!   cannot be told apart from an escaped `From `. Unescaping is then
//!   ambiguous for those rare lines; the mboxrd rule is applied anyway.
//! - Unescaped `From ` in a body: a line only starts a new message if it
//!   follows an empty line *and* ends with a valid date, as From_ lines do.
//!   This avoids splitting a message on a sentence such as "From what I
//!   understood…".
//!
//! The reader never loads the file into memory: it reads one line at a time
//! and keeps only the current message. Memory use is bounded by the largest
//! message, whatever the file size (the test file is 9.7 GB).

use std::io::{self, BufRead};

/// One message read from the mbox.
#[derive(Debug)]
pub struct MboxMessage {
    /// The message bytes (RFC 5322), unescaped, without the From_ line and
    /// without the empty separator line that follows it.
    pub bytes: Vec<u8>,
    /// Date of the From_ line, in Unix seconds (UTC): when the exporting
    /// server delivered the message. `None` if the line has no usable date
    /// (only possible for the first message, see [`MboxReader::next`]).
    pub delivered_at: Option<i64>,
    /// Offset of the From_ line in the file, for error messages.
    pub offset: u64,
}

/// Iterator over the messages of an mbox.
///
/// Rust note: `<R: BufRead>` makes the reader generic over its source: a
/// file in production (`BufReader<File>`), an in-memory byte slice in tests.
/// The compiler generates one specialized copy per source type, so the
/// abstraction costs nothing at runtime.
pub struct MboxReader<R: BufRead> {
    input: R,
    /// The From_ line that starts the next message, already read while
    /// looking for the end of the previous one.
    pending_from: Option<(Vec<u8>, u64)>,
    /// Bytes consumed so far, for progress reporting.
    position: u64,
    started: bool,
}

impl<R: BufRead> MboxReader<R> {
    pub fn new(input: R) -> MboxReader<R> {
        MboxReader { input, pending_from: None, position: 0, started: false }
    }

    /// Number of bytes read from the input so far.
    pub fn position(&self) -> u64 {
        self.position
    }

    /// Reads one line, newline included, into `buf`. Returns `false` at the
    /// end of the input.
    fn read_line(&mut self, buf: &mut Vec<u8>) -> io::Result<bool> {
        buf.clear();
        let n = self.input.read_until(b'\n', buf)?;
        self.position += n as u64;
        Ok(n > 0)
    }

    /// Returns the next message, `Ok(None)` at the end of the file.
    ///
    /// The first line of the file must be a From_ line, otherwise the file
    /// is not an mbox and an `InvalidData` error is returned. The date of
    /// that first line is not required: some exporters write a bare
    /// `From ` with no date, and there is no ambiguity at the start of the
    /// file.
    pub fn next_message(&mut self) -> io::Result<Option<MboxMessage>> {
        let mut line = Vec::new();
        let (from_line, offset) = match self.pending_from.take() {
            Some(p) => p,
            None if !self.started => {
                self.started = true;
                let offset = self.position;
                if !self.read_line(&mut line)? {
                    return Ok(None); // empty file: no message
                }
                if !line.starts_with(b"From ") {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "le fichier ne commence pas par une ligne « From » : ce n'est pas un mbox"));
                }
                (std::mem::take(&mut line), offset)
            }
            // Started and nothing pending: the previous call reached the end.
            None => return Ok(None),
        };

        let mut bytes = Vec::new();
        // Whether the previous line was empty: a From_ line only counts
        // after an empty line.
        let mut previous_empty = false;
        loop {
            let offset_line = self.position;
            if !self.read_line(&mut line)? {
                break;
            }
            if previous_empty && line.starts_with(b"From ") && from_line_date(&line).is_some() {
                // `line` starts the next message. The empty line before it is
                // the separator, not part of this message: remove it.
                self.pending_from = Some((std::mem::take(&mut line), offset_line));
                strip_last_newline(&mut bytes);
                break;
            }
            previous_empty = line == b"\n" || line == b"\r\n";
            // mboxrd unescaping: `>From `, `>>From `… lose one `>`.
            if line.first() == Some(&b'>') {
                let gt = line.iter().take_while(|b| **b == b'>').count();
                if line[gt..].starts_with(b"From ") {
                    bytes.extend_from_slice(&line[1..]);
                    continue;
                }
            }
            bytes.extend_from_slice(&line);
        }
        // At the end of the file, a trailing empty line is also a separator.
        if self.pending_from.is_none() && (bytes.ends_with(b"\n\n") || bytes.ends_with(b"\r\n\r\n")) {
            strip_last_newline(&mut bytes);
        }
        Ok(Some(MboxMessage { bytes, delivered_at: from_line_date(&from_line), offset }))
    }
}

/// Removes the last line break (`\n` or `\r\n`) of `bytes`, if any.
fn strip_last_newline(bytes: &mut Vec<u8>) {
    if bytes.ends_with(b"\r\n") {
        bytes.truncate(bytes.len() - 2);
    } else if bytes.ends_with(b"\n") {
        bytes.truncate(bytes.len() - 1);
    }
}

const MONTHS: [&[u8]; 12] = [b"Jan", b"Feb", b"Mar", b"Apr", b"May", b"Jun", b"Jul", b"Aug", b"Sep", b"Oct", b"Nov", b"Dec"];

/// Parses the date at the end of a From_ line, in the asctime format
/// `Tue Jun 23 19:20:56 2020` (UTC), optionally with a zone between the time
/// and the year (`Tue Jun 23 19:20:56 +0000 2020`). Returns Unix seconds.
///
/// The sender part may contain spaces (`From "John Doe"@x.org …`), so the
/// date is searched from the end of the line rather than by position.
pub fn from_line_date(line: &[u8]) -> Option<i64> {
    let text = std::str::from_utf8(line).ok()?;
    let tokens: Vec<&str> = text.split_whitespace().collect();
    // Find, from the end, a token that is a month name followed by a day and
    // a time; the year is the first 4-digit token after the time.
    for i in (1..tokens.len().saturating_sub(2)).rev() {
        let Some(month) = MONTHS.iter().position(|m| m.eq_ignore_ascii_case(tokens[i].as_bytes())) else {
            continue;
        };
        let day: u32 = tokens[i + 1].parse().ok()?;
        let mut hms = tokens[i + 2].split(':').map(|p| p.parse::<u32>());
        let (h, m) = (hms.next()?.ok()?, hms.next()?.ok()?);
        // Seconds are optional in some exports.
        let s = match hms.next() {
            Some(v) => v.ok()?,
            None => 0,
        };
        let year: i32 = tokens[i + 3..].iter().find(|t| t.len() == 4 && t.bytes().all(|b| b.is_ascii_digit()))?.parse().ok()?;
        if !(1..=31).contains(&day) || h > 23 || m > 59 || s > 60 {
            return None;
        }
        return Some(days_from_civil(year, month as u32 + 1, day) * 86_400 + (h * 3600 + m * 60 + s) as i64);
    }
    None
}

/// Number of days since 1970-01-01 of a calendar date (proleptic Gregorian
/// calendar). Algorithm by Howard Hinnant, exact for every year.
fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y } as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads every message of an in-memory mbox.
    fn all(input: &[u8]) -> Vec<MboxMessage> {
        let mut r = MboxReader::new(input);
        let mut out = vec![];
        while let Some(m) = r.next_message().unwrap() {
            out.push(m);
        }
        out
    }

    #[test]
    fn splits_messages_and_strips_separators() {
        let input = b"From a@x Tue Jun 23 19:20:56 2020\nSubject: one\n\nbody one\n\nFrom b@x Wed Jun 24 08:00:00 2020\nSubject: two\n\nbody two\n";
        let msgs = all(input);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].bytes, b"Subject: one\n\nbody one\n");
        assert_eq!(msgs[1].bytes, b"Subject: two\n\nbody two\n");
        assert_eq!(msgs[0].delivered_at, Some(1_592_940_056));
        assert_eq!(msgs[0].offset, 0);
        assert_eq!(msgs[1].offset, 58);
    }

    #[test]
    fn unescapes_from_lines_mboxrd() {
        let input = b"From a@x Tue Jun 23 19:20:56 2020\nSubject: s\n\n>From here\n>>From there\n> From not escaped\n";
        let msgs = all(input);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].bytes, b"Subject: s\n\nFrom here\n>From there\n> From not escaped\n");
    }

    #[test]
    fn body_from_without_date_does_not_split() {
        // "From what…" follows an empty line but has no date: same message.
        let input = b"From a@x Tue Jun 23 19:20:56 2020\nSubject: s\n\nhello\n\nFrom what I understood, yes.\n";
        let msgs = all(input);
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].bytes.ends_with(b"\n\nFrom what I understood, yes.\n"));
    }

    #[test]
    fn from_line_with_date_but_no_empty_line_does_not_split() {
        let input = b"From a@x Tue Jun 23 19:20:56 2020\nSubject: s\n\nline\nFrom b@x Wed Jun 24 08:00:00 2020\n";
        assert_eq!(all(input).len(), 1);
    }

    #[test]
    fn keeps_crlf_line_endings() {
        let input = b"From a@x Tue Jun 23 19:20:56 2020\r\nSubject: s\r\n\r\nbody\r\n\r\nFrom b@x Wed Jun 24 08:00:00 2020\r\nSubject: t\r\n\r\nx\r\n";
        let msgs = all(input);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].bytes, b"Subject: s\r\n\r\nbody\r\n");
        assert_eq!(msgs[1].bytes, b"Subject: t\r\n\r\nx\r\n");
    }

    #[test]
    fn last_message_without_trailing_newline() {
        let msgs = all(b"From a@x Tue Jun 23 19:20:56 2020\nSubject: s\n\nno newline");
        assert_eq!(msgs[0].bytes, b"Subject: s\n\nno newline");
    }

    #[test]
    fn empty_file_has_no_message() {
        assert!(all(b"").is_empty());
    }

    #[test]
    fn rejects_a_file_that_is_not_an_mbox() {
        let mut r = MboxReader::new(&b"Subject: not an mbox\n"[..]);
        assert_eq!(r.next_message().unwrap_err().kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn position_reaches_the_file_size() {
        let input = b"From a@x Tue Jun 23 19:20:56 2020\nA: b\n\nc\n";
        let mut r = MboxReader::new(&input[..]);
        while r.next_message().unwrap().is_some() {}
        assert_eq!(r.position(), input.len() as u64);
    }

    #[test]
    fn parses_from_line_dates() {
        // Quoted sender with a space, as in the HEY export.
        assert_eq!(from_line_date(b"From \"a b\"@x.org Tue Jun 23 19:20:56 2020\n"), Some(1_592_940_056));
        // Zone between time and year; single-digit day with double space.
        assert_eq!(from_line_date(b"From a@x Mon Mar  1 00:00:00 +0000 2021"), Some(1_614_556_800));
        // Leap day, and the epoch itself.
        assert_eq!(from_line_date(b"From a Thu Feb 29 12:00:00 2024"), Some(1_709_208_000));
        assert_eq!(from_line_date(b"From a Thu Jan  1 00:00:00 1970"), Some(0));
        // Not a date.
        assert_eq!(from_line_date(b"From what I understood, yes."), None);
        assert_eq!(from_line_date(b"From a Jun 99 10:00:00 2020"), None);
    }
}
