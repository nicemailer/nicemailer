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

/// The requests of one source beyond its rate are refused with
/// RATE_LIMITED (1019); `hello` counts against the session's budget, not
/// this one.
#[test]
fn requests_beyond_the_rate_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let _core = start_with(
        dir.path(),
        &[("NICEMAILER_TEST_RATE_BURST", "3"), ("NICEMAILER_TEST_RATE_PER_SEC", "0")],
    );
    // `hello` is one of the session's own: it goes through whatever the rate.
    let mut c = Client::connect(dir.path());
    let ids: Vec<u64> = (0..6).map(|_| c.send("list_folders", json!({}))).collect();
    let mut ok = 0;
    let mut limited = 0;
    for _ in 0..6 {
        let (id, res) = c.response();
        assert!(ids.contains(&id), "answer of an unknown id {id}");
        match res {
            Ok(_) => ok += 1,
            Err(e) => {
                assert_eq!(e["code"], json!(1019), "{e}");
                assert_eq!(e["data"]["reason"], json!("rate_limited"), "{e}");
                limited += 1;
            }
        }
    }
    assert_eq!(ok, 3, "the burst went through");
    assert_eq!(limited, 3, "the rest was refused");
}

/// The code of the error of `r`, or 0 for a success.
fn code(r: &Result<serde_json::Value, serde_json::Value>) -> i64 {
    match r {
        Ok(_) => 0,
        Err(e) => e["code"].as_i64().unwrap_or(-1),
    }
}

/// The rate is the source's, not the connection's: a second connection of
/// the same client has nothing left; another source has its own budget,
/// and the app is never limited (`limits.rs`).
#[test]
fn the_rate_is_shared_by_the_connections_of_a_source_and_spares_the_app() {
    let dir = tempfile::tempdir().unwrap();
    let _core = start_with(dir.path(), &[("NICEMAILER_TEST_RATE_BURST", "2"), ("NICEMAILER_TEST_RATE_PER_SEC", "0")]);
    let mut first = Client::connect_as(dir.path(), "cli");
    let mut second = Client::connect_as(dir.path(), "cli");
    assert_eq!(code(&first.call("list_folders", json!({}))), 0);
    assert_eq!(code(&second.call("list_folders", json!({}))), 0);
    assert_eq!(code(&second.call("list_folders", json!({}))), 1019, "the second connection shares the budget");
    assert_eq!(code(&first.call("list_folders", json!({}))), 1019);
    let mut agent = Client::connect_as(dir.path(), "mcp");
    assert_eq!(code(&agent.call("list_folders", json!({}))), 0, "another source, another budget");
    let mut app = Client::connect_as(dir.path(), "app");
    for _ in 0..20 {
        assert_eq!(code(&app.call("list_folders", json!({}))), 0, "the app is never limited");
    }
}

/// Deletions, moves and sends have a smaller budget of their own, on top
/// of the general one (`architecture.md`, section 9).
#[test]
fn deletions_have_a_budget_of_their_own() {
    let dir = tempfile::tempdir().unwrap();
    let _core = start_with(dir.path(), &[("NICEMAILER_TEST_DESTRUCTIVE_BURST", "2"), ("NICEMAILER_TEST_DESTRUCTIVE_PER_SEC", "0")]);
    let mut c = Client::connect(dir.path());
    // The tag does not exist: the handler refuses it, but the request was
    // counted all the same.
    for _ in 0..2 {
        assert_ne!(code(&c.call("delete_tag", json!({"name": "nope"}))), 1019);
    }
    assert_eq!(code(&c.call("delete_tag", json!({"name": "nope"}))), 1019);
    // Reads go on.
    assert_eq!(code(&c.call("list_folders", json!({}))), 0);
    // The app deletes freely.
    let mut app = Client::connect_as(dir.path(), "app");
    assert_ne!(code(&app.call("delete_tag", json!({"name": "nope"}))), 1019);
}

/// `ping`, `hello` and the like have a budget per connection: a flood of
/// them is refused too.
#[test]
fn a_flood_of_pings_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let _core = start_with(dir.path(), &[("NICEMAILER_TEST_SESSION_BURST", "3"), ("NICEMAILER_TEST_SESSION_PER_SEC", "0")]);
    // `hello` takes one of the three.
    let mut c = Client::connect(dir.path());
    assert_eq!(code(&c.call("ping", json!({}))), 0);
    assert_eq!(code(&c.call("ping", json!({}))), 0);
    assert_eq!(code(&c.call("ping", json!({}))), 1019);
    // The app has no such budget.
    let mut app = Client::connect_as(dir.path(), "app");
    for _ in 0..10 {
        assert_eq!(code(&app.call("ping", json!({}))), 0);
    }
}

/// Invalid JSON beyond the session's budget closes the connection: a
/// correct client never sends it.
#[test]
fn a_flood_of_invalid_json_closes_the_connection() {
    let dir = tempfile::tempdir().unwrap();
    let _core = start_with(dir.path(), &[("NICEMAILER_TEST_SESSION_BURST", "3"), ("NICEMAILER_TEST_SESSION_PER_SEC", "0")]);
    let mut c = Client::connect(dir.path());
    for _ in 0..3 {
        c.send_line("{not json");
    }
    let mut answers = 0;
    while let Some(line) = c.try_line() {
        assert_eq!(line["error"]["code"], json!(-32700), "{line}");
        answers += 1;
    }
    assert!((2..=3).contains(&answers), "{answers} answers before the close");
}

/// Before `hello`, a line is small (64 KiB), whatever the limit after it.
#[test]
fn a_long_line_before_hello_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let _core = start_with(dir.path(), &[]);
    let s = Client::socket(dir.path());
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    // Written from a thread: the core stops reading at the limit, and a
    // write of the whole line would wait for it forever.
    let mut w = s.try_clone().unwrap();
    let writer = std::thread::spawn(move || {
        let _ = w.write_all(&vec![b' '; 100 << 10]);
        let _ = w.write_all(b"\n");
    });
    let mut reader = std::io::BufReader::new(s);
    let mut line = String::new();
    std::io::BufRead::read_line(&mut reader, &mut line).unwrap();
    let answer: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(answer["error"]["code"], json!(-32600), "{answer}");
    assert!(answer["error"]["message"].as_str().unwrap().contains(&(64 << 10).to_string()), "{answer}");
    let _ = writer.join();
}
