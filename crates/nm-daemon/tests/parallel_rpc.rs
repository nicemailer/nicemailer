//! The requests of one connection (`serve`, `src/main.rs`): reads run
//! alongside the other requests, the others keep the order in which the
//! client sent them. And `prepare_draft`, which opens the reply screen,
//! never waits for a write.
//!
//! A write is made slow on purpose: `NICEMAILER_TEST_WRITE_DELAY_MS` holds
//! the store's lock that much longer at each write, as a batch of an
//! import does.

mod common;

use std::time::{Duration, Instant};

use common::{Client, start_logged, start_with};
use serde_json::json;

const DELAY_MS: u64 = 1500;

/// A read sent after a slow write, on the same connection, is answered
/// first, without waiting for it. Before, the requests of a connection
/// ran one after the other, and the app's `r` waited behind its reloads.
#[test]
fn a_read_does_not_wait_for_a_write_of_its_connection() {
    let dir = tempfile::tempdir().unwrap();
    let delay = DELAY_MS.to_string();
    let _core = start_with(dir.path(), &[("NICEMAILER_TEST_WRITE_DELAY_MS", delay.as_str())]);
    let mut c = Client::connect_as(dir.path(), "app");
    let start = Instant::now();
    // A decision on a sender: a workflow command, written under the lock.
    let write = c.send("set_address_screening", json!({"addr": "a@x.example", "target": "inbox"}));
    let read = c.send("list_folders", json!({}));
    let (first, result) = c.response();
    assert_eq!(first, read, "the read must not wait for the write");
    assert!(result.is_ok(), "{result:?}");
    assert!(start.elapsed() < Duration::from_millis(DELAY_MS), "read answered after {:?}", start.elapsed());
    let (second, result) = c.response();
    assert_eq!(second, write);
    assert!(result.is_ok(), "{result:?}");
}

/// The writes of a connection run in the order sent: the second never
/// overtakes the first, even when both wait for the lock.
#[test]
fn the_writes_of_a_connection_keep_their_order() {
    let dir = tempfile::tempdir().unwrap();
    let _core = start_with(dir.path(), &[("NICEMAILER_TEST_WRITE_DELAY_MS", "200")]);
    let mut c = Client::connect_as(dir.path(), "app");
    let ids: Vec<u64> = ["newsletter", "paperwork", "banned"]
        .iter()
        .map(|t| c.send("set_address_screening", json!({"addr": "a@x.example", "target": t})))
        .collect();
    let answered: Vec<u64> = (0..ids.len()).map(|_| c.response().0).collect();
    assert_eq!(answered, ids);
    // The last one sent is the one kept.
    let db = nm_store::rusqlite::Connection::open_with_flags(
        dir.path().join("nicemailer.db"),
        nm_store::rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let target: String = db.query_row("SELECT screening_target FROM address WHERE addr = 'a@x.example'", [], |r| r.get(0)).unwrap();
    assert_eq!(target, "banned");
}

/// The session's own requests are answered in the order read, before
/// what follows: everything is refused before `hello`, `hello` sets the
/// session before the next request runs, and a notification (no `id`) is
/// run but gets no answer.
#[test]
fn the_session_requests_stay_in_order() {
    use std::io::{BufRead, BufReader, Write};
    let dir = tempfile::tempdir().unwrap();
    let _core = start_with(dir.path(), &[]);
    // `Client::connect` would send `hello`: a bare connection.
    let _ = Client::connect(dir.path());
    let mut stream = std::os::unix::net::UnixStream::connect(dir.path().join("core.sock")).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut lines = BufReader::new(stream.try_clone().unwrap());
    let mut next = || {
        let mut l = String::new();
        lines.read_line(&mut l).unwrap();
        serde_json::from_str::<serde_json::Value>(&l).unwrap()
    };
    // Sent at once: refused, hello, a notification, a read.
    for line in [
        json!({"jsonrpc": "2.0", "id": 1, "method": "list_folders"}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "hello", "params": {"protocol_version": 1, "client": "app"}}),
        json!({"jsonrpc": "2.0", "method": "set_address_screening", "params": {"addr": "n@x.example", "target": "inbox"}}),
        json!({"jsonrpc": "2.0", "id": 3, "method": "list_folders"}),
    ] {
        writeln!(stream, "{line}").unwrap();
    }
    let r = next();
    assert_eq!((r["id"].as_u64(), r["error"]["code"].as_i64()), (Some(1), Some(1000)), "{r}");
    let r = next();
    assert_eq!(r["id"], 2);
    assert_eq!(r["result"]["protocol_version"], 1);
    let r = next();
    assert_eq!(r["id"], 3, "the notification is not answered: {r}");
    assert!(r["result"]["items"].is_array(), "{r}");
}

/// A slow request says in `core.log` when it ran and how its time splits:
/// the second of two writes waits for the first (the lock is held
/// `DELAY_MS`), so most of its time is waiting for the database, not its
/// own work.
#[test]
fn a_slow_request_says_how_long_it_waited_for_the_database() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("core.log");
    let delay = DELAY_MS.to_string();
    let core = start_logged(dir.path(), &[("NICEMAILER_TEST_WRITE_DELAY_MS", delay.as_str())], Some(&log));
    // Two connections: the writes of one connection run one after the
    // other before reaching the lock; of two, they meet at the lock.
    let mut a = Client::connect_as(dir.path(), "app");
    let mut b = Client::connect_as(dir.path(), "app");
    a.send("set_address_screening", json!({"addr": "a@x.example", "target": "inbox"}));
    std::thread::sleep(Duration::from_millis(100));
    b.send("set_address_screening", json!({"addr": "b@x.example", "target": "inbox"}));
    assert!(a.response().1.is_ok());
    assert!(b.response().1.is_ok());
    drop(core);
    let text = std::fs::read_to_string(&log).unwrap();
    // "12:03:41 slow request: set_address_screening took 2955 ms (2953 ms
    // waiting for the database, 1 ms running)": the second one waited for
    // the first. (The test delay is spent inside `lock_store`, so it counts
    // as waiting too.)
    let waited: Vec<u128> = text
        .lines()
        .filter(|l| l.contains("slow request: set_address_screening"))
        .map(|l| {
            assert!(l.as_bytes()[2] == b':' && l.as_bytes()[5] == b':', "dated: {l}");
            let n = l.split(" ms waiting").next().unwrap().rsplit('(').next().unwrap();
            n.parse().unwrap()
        })
        .collect();
    assert_eq!(waited.len(), 2, "{text}");
    assert!(waited.iter().any(|w| *w >= u128::from(DELAY_MS) - 300), "{text}");
}
