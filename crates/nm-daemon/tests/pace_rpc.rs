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
//! And the blocks of a server that limits logins (`pace.rs`, `LoginPace`):
//! a proxy in front of the Dovecot without TLS answers the login of a
//! chosen connection itself, as a server that limits logins would, or
//! spoils its password, as a server that locked the account would.
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
    /// the pace of the logins after a block of the server (20 November
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

/// What [`LoginProxy`] does to the login of the connections it picks.
#[derive(Clone, Copy)]
enum Answer {
    /// Answers itself, as Gmail or Infomaniak would: refused for now.
    Unavailable,
    /// Spoils the password: Dovecot refuses it, as a server that locked
    /// the account refuses a password it accepted before.
    WrongPassword,
}

/// A proxy that speaks clear text to the core and TLS to the test Dovecot,
/// so that it can read the login: it answers the login of the connections
/// whose index (from 0, in the order they come) is in `picked`, as
/// `answer` says; the others pass through. (Dovecot's own clear-text port
/// refuses logins, `LOGINDISABLED`: the connections come from outside its
/// container.)
struct LoginProxy {
    port: u16,
    connections: Arc<AtomicUsize>,
}

impl LoginProxy {
    fn start(answer: Answer, picked: std::ops::Range<usize>) -> LoginProxy {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let connections = Arc::new(AtomicUsize::new(0));
        let count = connections.clone();
        std::thread::spawn(move || {
            for client in listener.incoming() {
                let Ok(client) = client else { return };
                let index = count.fetch_add(1, Ordering::SeqCst);
                let answer = picked.contains(&index).then_some(answer);
                std::thread::spawn(move || {
                    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
                    rt.block_on(relay(client, answer));
                });
            }
        });
        LoginProxy { port, connections }
    }

    fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }
}

/// One connection of [`LoginProxy`]: Dovecot's bytes to the core as they
/// come; the core's line by line until the login, which `answer` (if any)
/// replaces, then as they come.
async fn relay(client: TcpStream, answer: Option<Answer>) {
    use rustls_pki_types::pem::PemObject;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    let mut roots = tokio_rustls::rustls::RootCertStore::empty();
    for cert in rustls_pki_types::CertificateDer::pem_file_iter(ca()).unwrap() {
        roots.add(cert.unwrap()).unwrap();
    }
    let provider = Arc::new(tokio_rustls::rustls::crypto::ring::default_provider());
    let config = tokio_rustls::rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let tcp = tokio::net::TcpStream::connect(("127.0.0.1", DOVECOT_TLS)).await.unwrap();
    let name = rustls_pki_types::ServerName::try_from("127.0.0.1").unwrap();
    let tls = tokio_rustls::TlsConnector::from(Arc::new(config)).connect(name, tcp).await.unwrap();
    let (mut from_server, mut to_server) = tokio::io::split(tls);
    client.set_nonblocking(true).unwrap();
    let client = tokio::net::TcpStream::from_std(client).unwrap();
    let (from_client, to_client) = client.into_split();
    // Written by both directions: Dovecot's bytes, and the proxy's own
    // answer to a login.
    let to_client = tokio::sync::Mutex::new(to_client);
    let mut reader = tokio::io::BufReader::new(from_client);
    let down = async {
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            match tokio::io::AsyncReadExt::read(&mut from_server, &mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if to_client.lock().await.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
            }
        }
        let _ = to_client.lock().await.shutdown().await;
    };
    let up = async {
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                let _ = to_server.shutdown().await;
                return;
            }
            let mut words = line.split_whitespace();
            let (tag, command) = (words.next().unwrap_or("").to_string(), words.next().unwrap_or("").to_ascii_uppercase());
            let login = command == "AUTHENTICATE" || command == "LOGIN";
            match (login, answer) {
                (true, Some(Answer::Unavailable)) => {
                    // Not sent to Dovecot: the proxy is the server here.
                    let reply = format!("{tag} NO [UNAVAILABLE] Too many logins, try again later\r\n");
                    let _ = to_client.lock().await.write_all(reply.as_bytes()).await;
                }
                (true, Some(Answer::WrongPassword)) => {
                    // AUTHENTICATE PLAIN with SASL-IR: "\0user\0password"
                    // in base64, the last word. Replaced by a wrong one.
                    let plain = base64_decode(line.split_whitespace().last().unwrap());
                    let user = String::from_utf8(plain).unwrap().split('\0').nth(1).unwrap().to_string();
                    let spoilt = base64_encode(format!("\0{user}\0wrong").as_bytes());
                    to_server.write_all(format!("{tag} AUTHENTICATE PLAIN {spoilt}\r\n").as_bytes()).await.unwrap();
                }
                _ => to_server.write_all(line.as_bytes()).await.unwrap(),
            }
            if login {
                break;
            }
        }
        let _ = tokio::io::copy(&mut reader, &mut to_server).await;
    };
    tokio::join!(down, up);
}

const B64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn base64_decode(text: &str) -> Vec<u8> {
    let (mut bits, mut n, mut out) = (0u32, 0, Vec::new());
    for c in text.bytes().filter(|c| *c != b'=') {
        bits = bits << 6 | B64.iter().position(|b| *b == c).unwrap() as u32;
        n += 6;
        if n >= 8 {
            n -= 8;
            out.push((bits >> n) as u8);
        }
    }
    out
}

#[test]
fn base64_both_ways() {
    for s in ["", "a", "ab", "abc", "\0user\0nicemailer"] {
        assert_eq!(base64_decode(&base64_encode(s.as_bytes())), s.as_bytes());
    }
    assert_eq!(base64_encode(b"\0alice\0secret"), "AGFsaWNlAHNlY3JldA==");
}

/// The core with one account behind the login proxy, without TLS.
fn plain_core(dir: &Path, port: u16, user: &str) -> (common::Core, Client) {
    let accounts = format!(
        "[accounts.cut]\nemail = \"{user}@example.com\"\n[accounts.cut.imap]\nhost = \"127.0.0.1\"\nport = {port}\n\
         security = \"none\"\nusername = \"{user}\"\n"
    );
    std::fs::write(dir.join("accounts.toml"), accounts).unwrap();
    let core = start_with(dir, &[]);
    let mut c = Client::connect(dir);
    c.call("set_password", json!({"account": "cut", "password": "nicemailer"})).unwrap();
    (core, c)
}

/// `account.login_limit`, `login_pause_until` and `login_ok_at`, from the
/// database.
fn saved_pace(dir: &Path) -> (Option<i64>, Option<i64>, Option<i64>) {
    let db = nm_store::rusqlite::Connection::open_with_flags(dir.join("nicemailer.db"), nm_store::rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .unwrap();
    db.query_row("SELECT login_limit, login_pause_until, login_ok_at FROM account WHERE config_key = 'cut'", [], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?))
    })
    .unwrap()
}

fn unix_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64
}

#[test]
#[ignore = "needs the test servers: make imap-up, then make imap-test"]
fn a_server_that_says_it_limits_logins_pauses_the_account_and_teaches_its_limit() {
    let dir = tempfile::tempdir().unwrap();
    let user = new_user();
    deliver(&user, "first");
    // The work connection logs in; the body connection, the second, is
    // refused "for now".
    let proxy = LoginProxy::start(Answer::Unavailable, 1..2);
    let (core, mut c) = plain_core(dir.path(), proxy.port, &user);

    // Learnt: two logins in the hour, so the floor of three an hour, and a
    // pause of 15 min. Not an error of the account: the work connection
    // works, and fetches the bodies.
    let a = until(&mut c, 60, "the limit learnt", |a| a["login_limit"] == 3 && a["bodies_missing"] == 0 && a["state"] == "idle");
    assert!(a["error"].is_null(), "{a:#}");
    let (limit, pause_until, ok_at) = saved_pace(dir.path());
    assert_eq!(limit, Some(3));
    assert!(ok_at.is_some(), "the work connection's login was recorded");
    let pause = pause_until.unwrap() - unix_now();
    assert!((800..=900).contains(&pause), "a pause of 15 min: {pause} s left");
    // The body connection does not try again during the pause.
    std::thread::sleep(Duration::from_secs(15));
    assert_eq!(proxy.connections(), 2, "no new login during the pause");

    // A restart of the core does not undo it: the limit is shown at once,
    // and the work connection waits for the end of the pause.
    drop(c);
    drop(core);
    let core = start_with(dir.path(), &[]);
    let mut c = Client::connect(dir.path());
    let a = until(&mut c, 20, "the pause kept", |a| a["state"] == "waiting");
    assert_eq!(a["login_limit"], 3, "{a:#}");
    assert!(a["next_retry"].as_i64().unwrap() > unix_now() + 600, "{a:#}");
    std::thread::sleep(Duration::from_secs(5));
    assert_eq!(proxy.connections(), 2, "no login after the restart");

    // A new password ends the pause (it may have been the reason); the
    // limit stays, it is the server's.
    c.call("set_password", json!({"account": "cut", "password": "nicemailer"})).unwrap();
    let a = until(&mut c, 30, "connected again", |a| a["state"] == "idle" && a["passes"].as_u64() >= Some(1));
    assert_eq!(a["login_limit"], 3, "{a:#}");
    assert!(proxy.connections() >= 3);
    drop(core);
}

#[test]
#[ignore = "needs the test servers: make imap-up, then make imap-test"]
fn a_password_refused_after_it_worked_is_a_lock_not_a_wrong_password() {
    let dir = tempfile::tempdir().unwrap();
    let user = new_user();
    deliver(&user, "first");
    // The work connection logs in with the password; the body connection's
    // is refused by Dovecot, as by a server that locked the account.
    let proxy = LoginProxy::start(Answer::WrongPassword, 1..2);
    let (_core, mut c) = plain_core(dir.path(), proxy.port, &user);

    // Taken for a lock: a pause of 30 min and the floor of three an hour,
    // not `auth_failed` with no attempt until a new password.
    let a = until(&mut c, 60, "the lock learnt", |a| a["login_limit"] == 3 && a["bodies_missing"] == 0);
    assert_eq!(a["state"], "idle", "{a:#}");
    let (_, pause_until, _) = saved_pace(dir.path());
    let pause = pause_until.unwrap() - unix_now();
    assert!((1700..=1800).contains(&pause), "a pause of 30 min: {pause} s left");
    std::thread::sleep(Duration::from_secs(10));
    assert_eq!(proxy.connections(), 2, "no new login during the pause");
}

#[test]
#[ignore = "needs the test servers: make imap-up, then make imap-test"]
fn a_password_wrong_from_the_start_is_not_tried_again_and_teaches_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let user = new_user();
    let proxy = LoginProxy::start(Answer::WrongPassword, 0..usize::MAX);
    let (_core, mut c) = plain_core(dir.path(), proxy.port, &user);

    // Never worked: a wrong password, as before. No limit, no pause, and
    // no attempt until a new password is saved.
    let a = until(&mut c, 30, "auth_failed", |a| a["state"] == "auth_failed");
    assert!(a["login_limit"].is_null(), "{a:#}");
    assert_eq!(saved_pace(dir.path()), (None, None, None));
    std::thread::sleep(Duration::from_secs(10));
    assert_eq!(proxy.connections(), 1, "tried once");
}
