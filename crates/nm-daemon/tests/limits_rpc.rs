//! The resource limits of the socket (`specifications/architecture.md`,
//! section 9, "Menaces couvertes et non couvertes"): one line is read up
//! to a maximum, the connections are capped, the handshake must come
//! quickly, and the requests of a connection are rate-limited. A client
//! that abuses the socket gets refused or slowed, it cannot make the core's
//! memory or its threads grow without bound.

mod common;

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
