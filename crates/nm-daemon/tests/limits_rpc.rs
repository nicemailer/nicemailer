//! The resource limits of the socket (`specifications/architecture.md`,
//! section 9, "Menaces couvertes et non couvertes"): one line is read up
//! to a maximum, the connections are capped, the handshake must come
//! quickly, and the requests of a connection are rate-limited. A client
//! that abuses the socket gets refused or slowed, it cannot make the core's
//! memory or its threads grow without bound.

mod common;

use std::io::{Read, Write};
use std::time::Duration;

use common::{Client, start_with};
use serde_json::json;

/// A line longer than the limit is refused with an error, then the
/// connection closes: the core does not read the whole line.
#[test]
fn a_line_longer_than_the_limit_is_refused_then_closed() {
    let dir = tempfile::tempdir().unwrap();
    let _core = start_with(dir.path(), &[("NICEMAILER_TEST_MAX_LINE_BYTES", "1024")]);
    let mut c = Client::connect(dir.path());
    c.send_line(&" ".repeat(4096));
    let answer = c.try_line().expect("an answer before the close");
    assert_eq!(answer["error"]["code"], json!(-32600), "{answer}");
    assert!(answer["error"]["message"].as_str().unwrap().contains("1024"), "{answer}");
    assert!(c.try_line().is_none(), "the connection closes after the refusal");
}

/// A line of exactly the limit (plus its newline) is read normally.
#[test]
fn a_line_of_exactly_the_limit_is_read() {
    let dir = tempfile::tempdir().unwrap();
    let _core = start_with(dir.path(), &[("NICEMAILER_TEST_MAX_LINE_BYTES", "1024")]);
    let mut c = Client::connect(dir.path());
    // 1024 bytes of JSON plus the newline: a `ping` whose params are padded.
    let padding = 1024 - r#"{"jsonrpc":"2.0","id":99,"method":"ping","params":{"pad":""}}"#.len();
    let line = format!(r#"{{"jsonrpc":"2.0","id":99,"method":"ping","params":{{"pad":"{}"}}}}"#, " ".repeat(padding));
    assert_eq!(line.len(), 1024);
    c.send_line(&line);
    let answer = c.try_line().expect("an answer");
    assert_eq!(answer["result"], json!("pong"), "{answer}");
}

/// Beyond the cap, a new connection is closed at once: the client sees a
/// closed socket, not a hang.
#[test]
fn connections_beyond_the_cap_are_closed_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let _core = start_with(dir.path(), &[("NICEMAILER_TEST_MAX_CONNECTIONS", "4")]);
    // Four connections, each with its `hello`, stay open.
    let mut held: Vec<Client> = (0..4).map(|_| Client::connect(dir.path())).collect();
    // The fifth is closed by the core before any answer.
    let mut extra = Client::socket(dir.path());
    extra.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    extra.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n").unwrap();
    let mut buf = [0u8; 16];
    let n = extra.read(&mut buf).unwrap();
    assert_eq!(n, 0, "the core closed the connection without answering");
    // A slot freed, a new connection is served again (the core notices the
    // close of the dropped one first).
    held.pop();
    std::thread::sleep(Duration::from_millis(500));
    let mut c = Client::connect(dir.path());
    assert_eq!(c.call("ping", json!({})).unwrap(), json!("pong"));
}

/// A connection that does not send its `hello` within the timeout is
/// closed, so it cannot hold a task and a file descriptor for nothing.
#[test]
fn a_connection_without_hello_is_closed_after_the_timeout() {
    let dir = tempfile::tempdir().unwrap();
    let _core = start_with(dir.path(), &[("NICEMAILER_TEST_HANDSHAKE_TIMEOUT_MS", "300")]);
    let mut s = Client::socket(dir.path());
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut buf = [0u8; 16];
    let n = s.read(&mut buf).unwrap();
    assert_eq!(n, 0, "the core closed the connection without a hello");
}

/// A `hello` sent within the timeout keeps the connection open: the normal
/// case, covered by every other test, made explicit here with a short one.
#[test]
fn a_hello_within_the_timeout_keeps_the_connection_open() {
    let dir = tempfile::tempdir().unwrap();
    let _core = start_with(dir.path(), &[("NICEMAILER_TEST_HANDSHAKE_TIMEOUT_MS", "2000")]);
    let mut c = Client::connect(dir.path());
    assert_eq!(c.call("ping", json!({})).unwrap(), json!("pong"));
}
