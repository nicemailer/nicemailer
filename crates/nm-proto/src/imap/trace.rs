//! IMAP conversation log, for debugging only (`docs/imap.md`, "Debug
//! log"). With `NICEMAILER_IMAP_LOG` in the environment, every line a
//! session sends and every response it receives is written, with the local
//! time and the connection it belongs to: `1` (or `stderr`) writes to
//! standard error, any other value is a file path, appended to. The
//! variable unset, nothing is formatted and a session costs exactly what it
//! costs without this module.
//!
//! What it shows is what the synchronization cannot tell by itself: what
//! the server said while in IDLE, and when (on 25 November 2026 a new mail
//! of an Outlook account was not seen by IDLE, and nothing said whether
//! Exchange had reported it).
//!
//! What it hides: the credentials. The lines of `LOGIN` and `AUTHENTICATE`
//! (a password, an OAuth2 access token in base64) are replaced by their
//! tag and command name ([`Session`](super::Session) mutes them). What it
//! cuts: a response longer than [`MAX_SHOWN`] bytes keeps its beginning and
//! its length, so that a body does not fill the log. It still carries
//! subjects, addresses and the beginning of mails: never on by default.

use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

/// The beginning of a response that is kept: enough for any status line,
/// `FETCH` of flags or `EXISTS`, not for a body.
pub(crate) const MAX_SHOWN: usize = 300;

/// The sink shared by every session, decided once from the environment.
/// `None`: `NICEMAILER_IMAP_LOG` unset, no log.
///
/// Rust note: a `static` with `OnceLock` is initialized by the first
/// caller, once, whatever the threads: every session of every account
/// writes to the same file, a line at a time under the mutex.
static SINK: OnceLock<Option<Mutex<Box<dyn Write + Send>>>> = OnceLock::new();

/// Numbers the connections, so that the lines of two sessions to the same
/// server can be told apart.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn sink() -> &'static Option<Mutex<Box<dyn Write + Send>>> {
    SINK.get_or_init(|| {
        let w: Box<dyn Write + Send> = match std::env::var("NICEMAILER_IMAP_LOG") {
            Err(_) => return None,
            Ok(v) if v.is_empty() || v == "1" || v == "stderr" => Box::new(std::io::stderr()),
            Ok(path) => match std::fs::File::options().create(true).append(true).open(&path) {
                Ok(f) => Box::new(f),
                Err(e) => {
                    eprintln!("NICEMAILER_IMAP_LOG: cannot open {path} ({e}), logging to stderr");
                    Box::new(std::io::stderr())
                }
            },
        };
        Some(Mutex::new(w))
    })
}

/// Whether the log is on.
pub(crate) fn enabled() -> bool {
    sink().is_some()
}

/// A new connection number; 0 when the log is off (none is needed).
pub(crate) fn next_id() -> u64 {
    if enabled() { NEXT_ID.fetch_add(1, Ordering::Relaxed) } else { 0 }
}

/// Which way a line goes.
#[derive(Clone, Copy)]
pub(crate) enum Way {
    /// From the client: `C:`.
    Sent,
    /// From the server: `S:`.
    Received,
}

/// Writes one line of the log: `14:42:31.207 #3 outlook/work S: * 5 EXISTS`.
/// `bytes` is shown as text, cut to [`MAX_SHOWN`], its line ends escaped.
pub(crate) fn line(id: u64, label: &str, way: Way, bytes: &[u8]) {
    let Some(sink) = sink() else { return };
    let text = shown(bytes);
    let way = match way {
        Way::Sent => "C:",
        Way::Received => "S:",
    };
    if let Ok(mut w) = sink.lock() {
        let _ = writeln!(w, "{} #{id} {label} {way} {text}", local_time());
    }
}

/// `bytes` as one line of text: CR and LF escaped (a response with a
/// literal holds several lines), cut after [`MAX_SHOWN`] bytes with the
/// whole length said.
pub(crate) fn shown(bytes: &[u8]) -> String {
    let trimmed = bytes.strip_suffix(b"\r\n").unwrap_or(bytes);
    let cut = &trimmed[..trimmed.len().min(MAX_SHOWN)];
    let mut text = String::from_utf8_lossy(cut).replace('\r', "\\r").replace('\n', "\\n");
    if trimmed.len() > MAX_SHOWN {
        text.push_str(&format!("… ({} bytes)", bytes.len()));
    }
    text
}

/// The local time of the Mac with milliseconds, `HH:MM:SS.mmm`: the delay
/// between a mail sent and the server's `EXISTS` is a matter of seconds.
///
/// Rust note: `unsafe` because `localtime_r` is a C function the compiler
/// cannot check; `localtime_r`, unlike `localtime`, is safe to call from
/// several threads.
fn local_time() -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let time = now.as_secs() as libc::time_t;
    // SAFETY: `tm` is a plain C struct, valid when zeroed; `localtime_r`
    // only reads `time` and writes `tm`.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&time, &mut tm).is_null() } {
        return "--:--:--.---".into();
    }
    format!("{:02}:{:02}:{:02}.{:03}", tm.tm_hour, tm.tm_min, tm.tm_sec, now.subsec_millis())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_escaped_and_a_long_response_cut() {
        assert_eq!(shown(b"* 5 EXISTS\r\n"), "* 5 EXISTS");
        assert_eq!(shown(b"* 1 FETCH (BODY[] {5}\r\nHello)\r\n"), "* 1 FETCH (BODY[] {5}\\r\\nHello)");
        let long = vec![b'x'; 1000];
        let s = shown(&long);
        assert!(s.starts_with(&"x".repeat(MAX_SHOWN)));
        assert!(s.ends_with("… (1000 bytes)"), "{s}");
    }
}
