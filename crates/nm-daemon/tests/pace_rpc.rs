//! The pace of the connections against a server that cuts idle ones
//! (`docs/sync.md`, "Pace of the connections"), end to end: the real core,
//! the test Dovecot, and between them a proxy that closes a connection the
//! way Orange did on 8 November 2026, without a TLS close_notify, after a
//! silence of the client or after a fixed time. The proxy counts the
//! connections: what the server would see as logins.
//!
//! The sleeps of the Mac too (`docs/sync.md`, "Sleep of the Mac"): the core
//! is told it slept through `NICEMAILER_TEST_SLEEP_FILE`
//! (`nm-engine/src/sync/clock.rs`), and the proxy can freeze the
//! connections open at that moment, as a sleep does to a connection the
//! server or the network dropped meanwhile: no byte goes through, and
//! nothing closes it.
//!
//! Needs the test servers: `make imap-up`, then `make imap-test`.

mod common;

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{Client, start_with};
use nm_proto::imap::{MailboxName, Security, ServerSpec, Session, Timeouts};
use serde_json::{Value, json};

/// Dovecot with TLS: the proxy passes the bytes through, TLS included.
const DOVECOT_TLS: u16 = 31993;

fn ca() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../dev/dovecot/tls/ca.crt")
}

/// When the proxy cuts a connection.
#[derive(Clone, Copy)]
enum Cut {
    /// After this long without a byte from the client: a server that sees
    /// a renewed IDLE as activity.
    ClientSilence(Duration),
    /// After this long, whatever happens: renewing does not help.
    Lifetime(Duration),
    /// The first connection lives; every other one is cut at once. The
    /// body connection then never gets a session, as when it waits for
    /// the budget of logins after a few sleeps of the Mac (20 November
    /// 2026: new mail stayed "(body not downloaded)" for 40 minutes).
    AllButFirst,
    /// Never cut ([`CuttingProxy::freeze`] only).
    Never,
}

/// A TCP proxy to Dovecot that cuts connections as `cut` says.
struct CuttingProxy {
    port: u16,
    connections: Arc<AtomicUsize>,
    /// Incremented by [`CuttingProxy::freeze`]: the connections opened
    /// before pass no byte any more.
    epoch: Arc<AtomicUsize>,
}

impl CuttingProxy {
    fn start(cut: Cut) -> CuttingProxy {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let connections = Arc::new(AtomicUsize::new(0));
        let count = connections.clone();
        let epoch = Arc::new(AtomicUsize::new(0));
        let frozen = epoch.clone();
        std::thread::spawn(move || {
            for client in listener.incoming() {
                let Ok(client) = client else { return };
                let index = count.fetch_add(1, Ordering::SeqCst);
                // At once, not at the next check of the cutting thread:
                // within its 100 ms a local login succeeds and fetches.
                if matches!(cut, Cut::AllButFirst) && index > 0 {
                    let _ = client.shutdown(Shutdown::Both);
                    continue;
                }
                let Ok(server) = TcpStream::connect(("127.0.0.1", DOVECOT_TLS)) else { continue };
                let last_from_client = Arc::new(Mutex::new(Instant::now()));
                let born = Instant::now();
                let alive = Alive { epoch: frozen.clone(), born: frozen.load(Ordering::SeqCst) };
                // Client to server, noting each byte's time.
                {
                    let (mut from, mut to) = (client.try_clone().unwrap(), server.try_clone().unwrap());
                    let last = last_from_client.clone();
                    let alive = alive.clone();
                    std::thread::spawn(move || pump(&mut from, &mut to, Some(&last), &alive));
                }
                {
                    let (mut from, mut to) = (server.try_clone().unwrap(), client.try_clone().unwrap());
                    std::thread::spawn(move || pump(&mut from, &mut to, None, &alive));
                }
                // The cut: both sockets shut down, no TLS goodbye.
                std::thread::spawn(move || {
                    loop {
                        std::thread::sleep(Duration::from_millis(100));
                        let due = match cut {
                            Cut::ClientSilence(d) => last_from_client.lock().unwrap().elapsed() >= d,
                            Cut::Lifetime(d) => born.elapsed() >= d,
                            Cut::AllButFirst | Cut::Never => false,
                        };
                        // A socket already closed by either side: nothing
                        // to cut.
                        if client.peer_addr().is_err() || due {
                            let _ = client.shutdown(Shutdown::Both);
                            let _ = server.shutdown(Shutdown::Both);
                            return;
                        }
                    }
                });
            }
        });
        CuttingProxy { port, connections, epoch }
    }

    fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }

    /// The connections open now pass nothing any more, in either
    /// direction, and stay open: dead, as far as the core can tell, only
    /// by a time limit. The next connections work.
    fn freeze(&self) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
    }
}

/// Whether a connection still passes bytes: it was opened in the current
/// epoch of the proxy.
#[derive(Clone)]
struct Alive {
    epoch: Arc<AtomicUsize>,
    born: usize,
}

impl Alive {
    fn frozen(&self) -> bool {
        self.epoch.load(Ordering::SeqCst) != self.born
    }
}

fn pump(from: &mut TcpStream, to: &mut TcpStream, last: Option<&Mutex<Instant>>, alive: &Alive) {
    let mut buf = [0u8; 16 * 1024];
    loop {
        match from.read(&mut buf) {
            Ok(0) | Err(_) => break,
            // Frozen: what is read is lost, and nothing is closed.
            Ok(_) if alive.frozen() => {}
            Ok(n) => {
                if let Some(last) = last {
                    *last.lock().unwrap() = Instant::now();
                }
                if to.write_all(&buf[..n]).is_err() {
                    break;
                }
            }
        }
    }
    let _ = to.shutdown(Shutdown::Both);
    let _ = from.shutdown(Shutdown::Both);
}

fn new_user() -> String {
    format!("p{:016x}", rand::random::<u64>())
}

/// The core with one account behind the proxy, its password saved.
fn core(dir: &Path, port: u16, user: &str) -> (common::Core, Client) {
    core_with(dir, port, user, &[])
}

fn core_with(dir: &Path, port: u16, user: &str, env: &[(&str, &str)]) -> (common::Core, Client) {
    let accounts = format!(
        "[accounts.cut]\nemail = \"{user}@example.com\"\n[accounts.cut.imap]\nhost = \"127.0.0.1\"\nport = {port}\n\
         security = \"tls\"\nusername = \"{user}\"\nca_file = \"{}\"\n",
        ca().display()
    );
    std::fs::write(dir.join("accounts.toml"), accounts).unwrap();
    let core = start_with(dir, env);
    let mut c = Client::connect(dir);
    c.call("set_password", json!({"account": "cut", "password": "nicemailer"})).unwrap();
    (core, c)
}

fn account(c: &mut Client) -> Value {
    c.call("sync_status", json!({"account": "cut"})).unwrap()["accounts"][0].clone()
}

/// Waits until `cond` holds on the account's status, at most `secs`.
fn until(c: &mut Client, secs: u64, what: &str, cond: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        let a = account(c);
        if cond(&a) {
            return a;
        }
        assert!(Instant::now() < deadline, "{what}: {a:#}");
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Delivers a mail straight to Dovecot (not through the proxy), as
/// another client would.
fn deliver(user: &str, id: &str) {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let spec = ServerSpec { host: "127.0.0.1".into(), port: DOVECOT_TLS, security: Security::Tls, ca_file: Some(ca()) };
        let mut s = Session::connect(&spec, Timeouts::default()).await.unwrap();
        s.login(user, "nicemailer").await.unwrap();
        let mail = format!(
            "From: Alice <alice@example.com>\r\nTo: {user}@example.com\r\nSubject: Mail {id}\r\nMessage-ID: <{id}@test.nicemailer>\r\n\
             Date: Thu, 05 Nov 2026 10:00:00 +0000\r\n\r\nBody of {id}.\r\n"
        );
        s.append(&MailboxName::inbox(), &[], None, mail.into_bytes()).await.unwrap();
        s.logout().await;
    });
}

fn has_mail(dir: &Path, id: &str) -> bool {
    let db = nm_store::rusqlite::Connection::open_with_flags(dir.join("nicemailer.db"), nm_store::rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .unwrap();
    db.query_row("SELECT count(*) FROM message WHERE message_id_hdr = ?1", [format!("{id}@test.nicemailer")], |r| r.get::<_, i64>(0))
        .unwrap()
        > 0
}

#[test]
#[ignore = "needs the test servers: make imap-up, then make imap-test"]
fn a_server_that_cuts_idle_gets_idle_renewed_and_one_connection() {
    let dir = tempfile::tempdir().unwrap();
    let user = new_user();
    deliver(&user, "first");
    let proxy = CuttingProxy::start(Cut::ClientSilence(Duration::from_secs(12)));
    let (_core, mut c) = core(dir.path(), proxy.port, &user);

    // The first IDLE is cut after 12 s: learnt, IDLE renewed every 6 s, and
    // nothing announced as an error.
    let a = until(&mut c, 60, "IDLE renewal learnt", |a| a["hold"] == "renew 6" && a["state"] == "idle");
    assert!(a["error"].is_null(), "{a:#}");
    // Then the connection holds: no new connection for 30 s (renewed five
    // times), where a login every 17 s was made before.
    let before = proxy.connections();
    assert!(before <= 3, "the work connection twice and the body connection once: {before}");
    std::thread::sleep(Duration::from_secs(30));
    assert_eq!(proxy.connections(), before, "no new connection while IDLE is renewed");
    let a = account(&mut c);
    assert_eq!(a["state"], "idle", "{a:#}");
    assert!(a["error"].is_null(), "{a:#}");

    // And IDLE still works: a new mail arrives within seconds, its body
    // fetched on the same connection.
    deliver(&user, "second");
    let deadline = Instant::now() + Duration::from_secs(20);
    while !has_mail(dir.path(), "second") {
        assert!(Instant::now() < deadline, "the new mail did not arrive");
        std::thread::sleep(Duration::from_millis(200));
    }
    until(&mut c, 30, "body fetched", |a| a["bodies_missing"] == 0 && a["state"] == "idle");
    assert_eq!(proxy.connections(), before, "the body was fetched on the work connection");
}

#[test]
#[ignore = "needs the test servers: make imap-up, then make imap-test"]
fn a_server_that_cuts_even_renewed_idle_gets_polled() {
    let dir = tempfile::tempdir().unwrap();
    let user = new_user();
    deliver(&user, "first");
    let proxy = CuttingProxy::start(Cut::Lifetime(Duration::from_secs(20)));
    let (_core, mut c) = core(dir.path(), proxy.port, &user);

    // Cut once (IDLE renewal learnt), cut again although renewed: polled,
    // a connection every 10 minutes at most.
    let a = until(&mut c, 120, "polling chosen", |a| a["hold"] == "poll 600" && a["state"] == "idle" && a["passes"].as_u64() >= Some(3));
    assert!(a["error"].is_null(), "{a:#}");
    let before = proxy.connections();
    assert!(before <= 4, "work three times, bodies once: {before}");
    std::thread::sleep(Duration::from_secs(30));
    assert_eq!(proxy.connections(), before, "no connection before the next poll");
    assert_eq!(account(&mut c)["state"], "idle");
}

#[test]
#[ignore = "needs the test servers: make imap-up, then make imap-test"]
fn bodies_arrive_while_the_body_connection_has_no_session() {
    let dir = tempfile::tempdir().unwrap();
    let user = new_user();
    deliver(&user, "first");
    let proxy = CuttingProxy::start(Cut::AllButFirst);
    let (_core, mut c) = core(dir.path(), proxy.port, &user);

    // The body connection is cut at every attempt: the work connection
    // fetches the bodies after its pass, the one already there included.
    let a = until(&mut c, 60, "first body fetched", |a| a["passes"].as_u64() >= Some(1) && a["bodies_missing"] == 0);
    assert!(a["error"].is_null(), "{a:#}");
    assert!(proxy.connections() >= 2, "the body connection tried: {}", proxy.connections());

    // A new mail, seen by IDLE on the work connection: its body follows
    // within seconds, not when the body connection gets a session.
    until(&mut c, 30, "back in IDLE", |a| a["state"] == "idle");
    deliver(&user, "second");
    let deadline = Instant::now() + Duration::from_secs(20);
    while !has_mail(dir.path(), "second") {
        assert!(Instant::now() < deadline, "the new mail did not arrive");
        std::thread::sleep(Duration::from_millis(200));
    }
    until(&mut c, 30, "second body fetched", |a| a["bodies_missing"] == 0);
}

/// The core told it slept (`NICEMAILER_TEST_SLEEP_FILE`), settled: in IDLE
/// after its first pass, the body connection logged in, every body there.
fn settled_core(dir: &Path, proxy: &CuttingProxy, user: &str) -> (common::Core, Client, PathBuf) {
    let sleep = dir.join("sleep");
    let (core, mut c) = core_with(dir, proxy.port, user, &[("NICEMAILER_TEST_SLEEP_FILE", sleep.to_str().unwrap())]);
    until(&mut c, 60, "first pass", |a| a["passes"].as_u64() >= Some(1) && a["bodies_missing"] == 0 && a["state"] == "idle");
    let deadline = Instant::now() + Duration::from_secs(30);
    while proxy.connections() < 2 {
        assert!(Instant::now() < deadline, "the body connection did not log in");
        std::thread::sleep(Duration::from_millis(200));
    }
    // Its first `NOOP` or IDLE settled.
    std::thread::sleep(Duration::from_secs(2));
    (core, c, sleep)
}

/// The Mac "slept" `secs` and "woke" at the file's time.
fn sleep_for(file: &Path, secs: u64) {
    std::fs::write(file, secs.to_string()).unwrap();
}

#[test]
#[ignore = "needs the test servers: make imap-up, then make imap-test"]
fn a_connection_that_survives_a_sleep_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let user = new_user();
    deliver(&user, "first");
    let proxy = CuttingProxy::start(Cut::Never);
    let (_core, mut c, sleep) = settled_core(dir.path(), &proxy, &user);
    let before = proxy.connections();
    let passes = account(&mut c)["passes"].as_u64().unwrap();

    // Ten minutes asleep: the connections are tested (IDLE ended, `NOOP`),
    // answer, and are kept. A full pass follows on the work connection.
    // Until 21 November 2026 both were dropped, two logins per wake.
    sleep_for(&sleep, 600);
    let a = until(&mut c, 45, "a pass after the wake", |a| a["passes"].as_u64() > Some(passes) && a["state"] == "idle");
    assert!(a["error"].is_null(), "{a:#}");
    assert_eq!(proxy.connections(), before, "no new login");

    // IDLE still works on it.
    deliver(&user, "second");
    let deadline = Instant::now() + Duration::from_secs(20);
    while !has_mail(dir.path(), "second") {
        assert!(Instant::now() < deadline, "the new mail did not arrive");
        std::thread::sleep(Duration::from_millis(200));
    }
    until(&mut c, 30, "body fetched", |a| a["bodies_missing"] == 0);
    assert_eq!(proxy.connections(), before, "still no new login");
}

#[test]
#[ignore = "needs the test servers: make imap-up, then make imap-test"]
fn after_a_sleep_that_kills_the_connections_one_login_once_awake() {
    let dir = tempfile::tempdir().unwrap();
    let user = new_user();
    deliver(&user, "first");
    let proxy = CuttingProxy::start(Cut::Never);
    let (_core, mut c, sleep) = settled_core(dir.path(), &proxy, &user);
    let before = proxy.connections();
    let passes = account(&mut c)["passes"].as_u64().unwrap();

    // Both connections die with the sleep, without a word: they do not
    // answer the test (`PROBE`, 10 s). The Mac "woke" 20 s in the future:
    // the core sees a wake of 0 s ago until then, so that the wait for
    // `WAKE_SETTLE` (15 s) shows apart from the time the test takes.
    proxy.freeze();
    sleep_for(&sleep, 600);
    let file = std::fs::OpenOptions::new().write(true).open(&sleep).unwrap();
    let woke = std::time::SystemTime::now() + Duration::from_secs(20);
    file.set_modified(woke).unwrap();
    let written = Instant::now();

    let deadline = Instant::now() + Duration::from_secs(90);
    while proxy.connections() == before {
        assert!(Instant::now() < deadline, "no new connection after the wake");
        std::thread::sleep(Duration::from_millis(100));
    }
    let after = written.elapsed();
    assert!(after >= Duration::from_secs(20 + 15), "logged in {after:?} after the wake was written, before the Mac was awake 15 s");
    let a = until(&mut c, 45, "a pass on the new connection", |a| a["passes"].as_u64() > Some(passes) && a["state"] == "idle");
    assert!(a["error"].is_null(), "a sleep is not an error: {a:#}");

    // The work connection alone logged in again: a new mail and its body
    // come through it, and the body connection waits for a backlog.
    deliver(&user, "second");
    let deadline = Instant::now() + Duration::from_secs(20);
    while !has_mail(dir.path(), "second") {
        assert!(Instant::now() < deadline, "the new mail did not arrive");
        std::thread::sleep(Duration::from_millis(200));
    }
    until(&mut c, 30, "body fetched", |a| a["bodies_missing"] == 0);
    std::thread::sleep(Duration::from_secs(5));
    assert_eq!(proxy.connections(), before + 1, "one login for the wake, not one per connection");
}
