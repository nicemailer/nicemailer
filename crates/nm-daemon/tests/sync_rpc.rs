//! The synchronization end to end (`plan-imap.md`, phase 4): the real core,
//! a test Dovecot, and a second connection playing the phone, which adds,
//! flags, deletes and moves mail. After each step, `sync_verify` must find
//! the server and the database the same.
//!
//! `#[ignore]`: needs the test servers (`make imap-up`, then
//! `make imap-test`). Every test runs on a new random user, so an empty
//! mailbox, and most on both servers: the complete one (QRESYNC, MOVE…)
//! and the minimal one (none of the synchronization extensions, the
//! fallbacks).

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::{Client, start};
use nm_proto::imap::{MailboxName, Security, ServerSpec, Session, StoreOp, Timeouts, UidSet, Uids};
use serde_json::{Value, json};

const FULL: u16 = 31993;
const MINIMAL: u16 = 32993;

fn ca() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../dev/dovecot/tls/ca.crt")
}

fn config(port: u16, user: &str, extra: &str) -> String {
    format!(
        "[accounts.dovecot]\nemail = \"{user}@example.com\"\n[accounts.dovecot.imap]\nhost = \"127.0.0.1\"\nport = {port}\n\
         security = \"tls\"\nusername = \"{user}\"\nca_file = \"{}\"\n{extra}",
        ca().display()
    )
}

/// The phone: a session of its own on the same mailbox, with its own
/// little runtime (the test itself is synchronous, like the CLI).
struct Phone {
    rt: tokio::runtime::Runtime,
    s: Session,
}

impl Phone {
    fn open(port: u16, user: &str) -> Phone {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let s = rt.block_on(async {
            let spec = ServerSpec { host: "127.0.0.1".into(), port, security: Security::Tls, ca_file: Some(ca()) };
            let mut s = Session::connect(&spec, Timeouts::default()).await.unwrap();
            s.login(user, "nicemailer").await.unwrap();
            s
        });
        Phone { rt, s }
    }

    fn create(&mut self, name: &str) {
        let _ = self.rt.block_on(self.s.create(&MailboxName::new(name)));
    }

    /// Delivers a mail to `folder`; returns its UID.
    fn deliver(&mut self, folder: &str, id: &str, flags: &[&str]) -> u32 {
        let mail = format!(
            "From: Alice <alice@example.com>\r\nTo: bob@example.com\r\nSubject: Mail {id}\r\nMessage-ID: <{id}@test.nicemailer>\r\n\
             Date: Thu, 05 Nov 2026 10:00:00 +0000\r\n\r\nBody of {id}, with the word chantier.\r\n"
        );
        self.rt.block_on(self.s.append(&MailboxName::new(folder), flags, None, mail.into_bytes())).unwrap().unwrap().uid
    }

    fn select(&mut self, folder: &str) {
        self.rt.block_on(self.s.select(&MailboxName::new(folder), None)).unwrap();
    }

    fn store(&mut self, uid: u32, op: StoreOp, flags: &[&str]) {
        self.rt.block_on(self.s.uid_store(&Uids::Set(UidSet::from_uids([uid])), op, flags, true, None)).unwrap();
    }

    fn expunge(&mut self, uid: u32) {
        self.store(uid, StoreOp::Add, &["\\Deleted"]);
        self.rt.block_on(self.s.uid_expunge(&Uids::Set(UidSet::from_uids([uid])))).unwrap();
    }

    /// The flags of `uid` in `folder`.
    fn flags(&mut self, folder: &str, uid: u32) -> Vec<String> {
        self.select(folder);
        let items = nm_proto::imap::FetchItems { flags: true, ..Default::default() };
        let got = self.rt.block_on(self.s.uid_fetch(&Uids::Set(UidSet::from_uids([uid])), &items)).unwrap();
        got.into_iter().next().and_then(|f| f.flags).unwrap_or_default()
    }

    fn uids(&mut self, folder: &str) -> Vec<u32> {
        self.select(folder);
        self.rt.block_on(self.s.uid_search(&nm_proto::imap::SearchKey::All)).unwrap()
    }

    /// Whether `folder` holds the mail `id` (by Message-ID).
    fn has(&mut self, folder: &str, id: &str) -> bool {
        self.select(folder);
        let key = nm_proto::imap::SearchKey::Header("Message-ID".into(), format!("<{id}@test.nicemailer>"));
        !self.rt.block_on(self.s.uid_search(&key)).unwrap().is_empty()
    }

    /// Moves `uid` of the selected folder to `to`: `MOVE` where the server
    /// has it, else copy, `\Deleted`, expunge (the minimal server).
    fn move_to(&mut self, uid: u32, to: &str) {
        self.rt.block_on(self.s.uid_move(&Uids::Set(UidSet::from_uids([uid])), &MailboxName::new(to))).unwrap();
    }
}

/// The core on a fresh data directory, with the account's password saved.
fn core(dir: &Path, port: u16, user: &str, extra: &str) -> (common::Core, Client) {
    std::fs::write(dir.join("accounts.toml"), config(port, user, extra)).unwrap();
    let core = start(dir);
    let mut c = Client::connect(dir);
    c.call("set_password", json!({"account": "dovecot", "password": "nicemailer"})).unwrap();
    (core, c)
}

fn account(c: &mut Client) -> Value {
    c.call("sync_status", json!({"account": "dovecot"})).unwrap()["accounts"][0].clone()
}

/// Waits for a pass after `passes` (the count before the change), and for
/// every body to be downloaded; returns the new count.
fn settled(c: &mut Client, passes: u64) -> u64 {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let a = account(c);
        let n = a["passes"].as_u64().unwrap_or(0);
        if n > passes && a["state"] == "idle" && a["bodies_missing"] == 0 && a["actions_pending"] == 0 {
            return n;
        }
        assert!(Instant::now() < deadline, "not settled: {a}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// `sync_now`, then waits for its pass.
fn sync(c: &mut Client) -> u64 {
    let before = account(c)["passes"].as_u64().unwrap_or(0);
    c.call("sync_now", json!({"account": "dovecot"})).unwrap();
    settled(c, before)
}

fn verify_ok(c: &mut Client) -> Value {
    let r = c.call("sync_verify", json!({"account": "dovecot"})).unwrap();
    assert_eq!(r["ok"], true, "{r:#}");
    r
}

fn folder<'a>(r: &'a Value, path: &str) -> &'a Value {
    r["folders"].as_array().unwrap().iter().find(|f| f["path"] == path).unwrap_or_else(|| panic!("no {path} in {r:#}"))
}

/// The database of the core, read directly.
fn db(dir: &Path) -> nm_store::rusqlite::Connection {
    nm_store::rusqlite::Connection::open_with_flags(dir.join("nicemailer.db"), nm_store::rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap()
}

fn message_id_of(dir: &Path, msgid: &str) -> (i64, String, u32) {
    db(dir)
        .query_row(
            "SELECT m.id, f.path, m.uid FROM message m JOIN folder f ON f.id = m.folder_id WHERE m.message_id_hdr = ?1",
            [format!("{msgid}@test.nicemailer")],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap()
}

fn new_user() -> String {
    format!("s{:016x}", rand::random::<u64>())
}

/// Every step of a phone's life, on one server.
fn follows_the_phone(port: u16) {
    let dir = tempfile::tempdir().unwrap();
    let user = new_user();
    let mut phone = Phone::open(port, &user);
    phone.create("Archive");
    phone.create("Sent");
    let a = phone.deliver("INBOX", "a", &[]);
    let _b = phone.deliver("INBOX", "b", &["\\Flagged"]);
    phone.deliver("Sent", "c", &["\\Seen"]);
    phone.deliver("Archive", "d", &["\\Seen", "$Projet"]);

    // First synchronization: every folder, every body.
    let (_core, mut c) = core(dir.path(), port, &user, "");
    let mut passes = settled(&mut c, 0);
    let r = verify_ok(&mut c);
    assert_eq!(folder(&r, "INBOX")["server_messages"], 2);
    assert_eq!(folder(&r, "INBOX")["bodies_checked"], 2);
    assert_eq!(folder(&r, "Archive")["local_messages"], 1);
    let st = account(&mut c);
    let roles: Vec<&str> = st["folders"].as_array().unwrap().iter().map(|f| f["role"].as_str().unwrap()).collect();
    assert!(roles.starts_with(&["inbox"]) && roles.contains(&"archive") && roles.contains(&"sent"), "{roles:?}");
    // The search finds the bodies.
    let hits = c.call("search", json!({"query": "chantier"})).unwrap();
    assert_eq!(hits["items"].as_array().unwrap().len(), 4, "{hits}");

    // New mail: seen by IDLE, announced.
    c.call("subscribe", json!({})).unwrap();
    c.clear_events();
    phone.deliver("INBOX", "e", &[]);
    let arrived = c.event("MessageArrived", 30);
    assert!(arrived["message_id"].as_i64().is_some());
    settled(&mut c, passes);
    verify_ok(&mut c);

    // Flags changed on the phone.
    phone.select("INBOX");
    phone.store(a, StoreOp::Add, &["\\Seen", "Urgent"]);
    passes = sync(&mut c);
    let r = verify_ok(&mut c);
    assert_eq!(folder(&r, "INBOX")["flags_differ_count"], 0);
    let seen: i64 = db(dir.path())
        .query_row("SELECT is_seen FROM message WHERE message_id_hdr = 'a@test.nicemailer'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(seen, 1);

    // A mail moved by the phone keeps its identity: same id, new folder.
    let (id_before, _, _) = message_id_of(dir.path(), "a");
    phone.move_to(a, "Archive");
    passes = settled(&mut c, passes);
    let (id_after, path, _) = message_id_of(dir.path(), "a");
    assert_eq!((id_after, path.as_str()), (id_before, "Archive"), "moved, not deleted and created again");
    verify_ok(&mut c);

    // A mail deleted by the phone.
    let e = message_id_of(dir.path(), "e").2;
    phone.select("INBOX");
    phone.expunge(e);
    let _ = sync(&mut c);
    let r = verify_ok(&mut c);
    assert_eq!(folder(&r, "INBOX")["local_messages"], 1, "b alone");
    let _ = passes;

    // What NiceMailer changes reaches the server (phase 5): read, flagged,
    // tagged, moved; the phone sees it, and the message keeps its id.
    let (b_id, _, _) = message_id_of(dir.path(), "b");
    let thread: i64 = db(dir.path())
        .query_row("SELECT thread_id FROM message WHERE message_id_hdr = 'b@test.nicemailer'", [], |r| r.get(0))
        .unwrap();
    let before = account(&mut c)["passes"].as_u64().unwrap();
    c.call("set_thread_seen", json!({"thread_id": thread, "seen": true})).unwrap();
    c.call("tag_thread", json!({"thread_id": thread, "name": "Chantier"})).unwrap();
    c.call("move_thread", json!({"thread_id": thread, "target": "archive"})).unwrap();
    settled(&mut c, before);
    let (id, path, uid) = message_id_of(dir.path(), "b");
    assert_eq!((id, path.as_str()), (b_id, "Archive"));
    let flags = phone.flags("Archive", uid);
    assert!(flags.iter().any(|f| f == "\\Seen") && flags.iter().any(|f| f == "\\Flagged"), "{flags:?}");
    assert!(flags.iter().any(|f| f.eq_ignore_ascii_case("chantier")), "the tag is a keyword: {flags:?}");
    assert!(phone.uids("INBOX").is_empty() || !phone.has("INBOX", "b"), "gone from the Inbox");
    verify_ok(&mut c);
}

#[test]
#[ignore = "needs the test servers: make imap-up, then make imap-test"]
fn follows_the_phone_on_the_complete_server() {
    follows_the_phone(FULL);
}

#[test]
#[ignore = "needs the test servers: make imap-up, then make imap-test"]
fn follows_the_phone_on_the_minimal_server() {
    follows_the_phone(MINIMAL);
}

#[test]
#[ignore = "needs the test servers: make imap-up, then make imap-test"]
fn one_connection_does_everything() {
    let dir = tempfile::tempdir().unwrap();
    let user = new_user();
    let mut phone = Phone::open(FULL, &user);
    for i in 0..30 {
        phone.deliver("INBOX", &format!("m{i}"), &[]);
    }
    let (_core, mut c) = core(dir.path(), FULL, &user, "[accounts.dovecot.sync]\nmax_connections = 1\n");
    settled(&mut c, 0);
    let r = verify_ok(&mut c);
    assert_eq!(folder(&r, "INBOX")["bodies_checked"], 30);
}

#[test]
#[ignore = "needs the test servers: make imap-up, then make imap-test"]
fn a_new_uidvalidity_fetches_the_folder_again() {
    let dir = tempfile::tempdir().unwrap();
    let user = new_user();
    let mut phone = Phone::open(FULL, &user);
    phone.create("Archive");
    phone.deliver("Archive", "old", &[]);
    let (_core, mut c) = core(dir.path(), FULL, &user, "");
    settled(&mut c, 0);
    // The server's Archive is replaced by another folder of the same name:
    // a new UIDVALIDITY, other UIDs.
    phone.rt.block_on(phone.s.create(&MailboxName::new("Archive2"))).unwrap();
    raw_command(&user, "RENAME Archive Gone");
    raw_command(&user, "RENAME Archive2 Archive");
    phone.deliver("Archive", "new", &[]);
    sync(&mut c);
    let r = verify_ok(&mut c);
    assert_eq!(folder(&r, "Archive")["local_messages"], 1);
    let n: i64 = db(dir.path()).query_row("SELECT count(*) FROM message WHERE message_id_hdr = 'old@test.nicemailer'", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 0, "the old folder's mail is gone with it");
}

/// RENAME is not in the client (NiceMailer renames no folder on the server
/// yet): the test sends it through openssl's TLS client, as `user`.
fn raw_command(user: &str, command: &str) {
    use std::io::Write;
    let mut child = std::process::Command::new("/usr/bin/openssl")
        .args(["s_client", "-connect", "127.0.0.1:31993", "-quiet", "-CAfile"])
        .arg(ca())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    write!(stdin, "a LOGIN {user} nicemailer\r\nb {command}\r\nc LOGOUT\r\n").unwrap();
    drop(stdin);
    let out = child.wait_with_output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("b OK"), "{command}: {text}");
}

/// A TCP relay to the complete server that cuts its first connections,
/// the `n`-th after `cuts[n]` bytes from the server, and relays the next
/// ones whole: network cuts at chosen moments.
fn cutting_proxy(cuts: Vec<usize>) -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            listener.set_nonblocking(true).unwrap();
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            let mut n = 0;
            loop {
                let (client, _) = listener.accept().await.unwrap();
                let cut = cuts.get(n).copied().unwrap_or(usize::MAX);
                n += 1;
                tokio::spawn(async move {
                    let server = tokio::net::TcpStream::connect(("127.0.0.1", FULL)).await.unwrap();
                    let (mut cr, mut cw) = client.into_split();
                    let (mut sr, mut sw) = server.into_split();
                    let up = tokio::spawn(async move {
                        let _ = tokio::io::copy(&mut cr, &mut sw).await;
                    });
                    let mut relayed = 0usize;
                    let mut buf = vec![0; 16 * 1024];
                    while relayed < cut {
                        let k = match sr.read(&mut buf).await {
                            Ok(0) | Err(_) => break,
                            Ok(k) => k.min(cut - relayed),
                        };
                        if cw.write_all(&buf[..k]).await.is_err() {
                            break;
                        }
                        relayed += k;
                    }
                    up.abort();
                });
            }
        });
    });
    port
}

#[test]
#[ignore = "needs the test servers: make imap-up, then make imap-test"]
fn cuts_at_every_step_lose_nothing_and_duplicate_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let user = new_user();
    let mut phone = Phone::open(FULL, &user);
    phone.create("Archive");
    // 150 mails of about 30 kB: the headers take a few round trips, the
    // bodies 4.5 MB.
    let filler = "x".repeat(30_000);
    for i in 0..150 {
        let mail = format!(
            "From: alice@example.com\r\nSubject: Cut {i}\r\nMessage-ID: <cut{i}@test.nicemailer>\r\n\r\n{filler}\r\n"
        );
        let folder = if i % 3 == 0 { "Archive" } else { "INBOX" };
        phone.rt.block_on(phone.s.append(&MailboxName::new(folder), &[], None, mail.into_bytes())).unwrap();
    }
    // Cuts during the login, the listing, the headers, the bodies (both
    // connections draw from the same list, in the order they connect).
    let port = cutting_proxy(vec![3_000, 20_000, 60_000, 400_000, 1_500_000, 3_000_000]);
    std::fs::write(dir.path().join("accounts.toml"), config(port, &user, "")).unwrap();
    let _core = start(dir.path());
    let mut c = Client::connect(dir.path());
    c.call("set_password", json!({"account": "dovecot", "password": "nicemailer"})).unwrap();
    settled(&mut c, 0);
    // The proxy relays whole now: verify through it as well.
    let r = verify_ok(&mut c);
    assert_eq!(folder(&r, "INBOX")["local_messages"], 100);
    assert_eq!(folder(&r, "Archive")["local_messages"], 50);
    let d = db(dir.path());
    let dup: i64 = d
        .query_row("SELECT count(*) FROM (SELECT message_id_hdr FROM message GROUP BY message_id_hdr HAVING count(*) > 1)", [], |r| r.get(0))
        .unwrap();
    assert_eq!(dup, 0, "no message twice");
    let tmp = std::fs::read_dir(dir.path().join("raw.noindex/.tmp")).unwrap().count();
    assert_eq!(tmp, 0, "no partial file left");
}

#[test]
#[ignore = "needs the test servers: make imap-up, then make imap-test"]
fn a_refused_login_waits_for_a_new_password() {
    let dir = tempfile::tempdir().unwrap();
    let user = new_user();
    Phone::open(FULL, &user).deliver("INBOX", "one", &[]);
    std::fs::write(dir.path().join("accounts.toml"), config(FULL, &user, "")).unwrap();
    let _core = start(dir.path());
    let mut c = Client::connect(dir.path());
    c.call("subscribe", json!({})).unwrap();
    c.call("set_password", json!({"account": "dovecot", "password": "wrong"})).unwrap();
    let e = c.event("AccountError", 30);
    assert!(e["error"].as_str().unwrap().contains("refused the login"), "{e}");
    assert_eq!(e["why"]["reason"], "imap_auth_refused", "{e}");
    // No new attempt on its own: a server may lock an account that keeps
    // sending a wrong password.
    std::thread::sleep(Duration::from_secs(8));
    let a = account(&mut c);
    assert_eq!(a["state"], "auth_failed", "{a}");
    assert_eq!(a["passes"], 0);
    // The right password: it starts again at once.
    c.call("set_password", json!({"account": "dovecot", "password": "nicemailer"})).unwrap();
    settled(&mut c, 0);
    verify_ok(&mut c);
}

/// The core started with `env` set (the crash point of the spool).
fn start_with(dir: &Path, env: &[(&str, &str)]) -> std::process::Child {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_nicemailer-core"));
    cmd.arg("--data-dir").arg(dir).arg("--config").arg(dir.join("config.toml")).arg("--keychain").arg(dir.join("test.keychain-db"));
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.spawn().unwrap()
}

#[test]
#[ignore = "needs the test servers: make imap-up, then make imap-test"]
fn a_move_the_server_refuses_is_undone_and_announced() {
    let dir = tempfile::tempdir().unwrap();
    let user = new_user();
    let mut phone = Phone::open(FULL, &user);
    phone.create("Archive");
    phone.deliver("INBOX", "r", &[]);
    let (_core, mut c) = core(dir.path(), FULL, &user, "");
    settled(&mut c, 0);
    c.call("subscribe", json!({})).unwrap();
    // The phone takes the Archive away; the core does not know yet.
    raw_command(&user, "RENAME Archive Elsewhere");
    let (id, _, _) = message_id_of(dir.path(), "r");
    let thread: i64 = db(dir.path()).query_row("SELECT thread_id FROM message WHERE id = ?1", [id], |r| r.get(0)).unwrap();
    c.clear_events();
    c.call("move_thread", json!({"thread_id": thread, "target": "archive"})).unwrap();
    let e = c.event("ActionReverted", 30);
    assert_eq!(e["message_id"], id);
    assert_eq!(e["kind"], "move");
    assert!(e["message"].as_str().unwrap().contains("doesn't exist"), "{e}");
    assert_eq!(e["why"]["reason"], "server_refused_change", "{e}");
    assert!(e["why"]["server_text"].as_str().unwrap().contains("doesn't exist"), "the server's answer, cited: {e}");
    let (_, path, uid) = message_id_of(dir.path(), "r");
    assert_eq!(path, "INBOX", "back where it was");
    assert!(uid > 0);
    sync(&mut c);
    verify_ok(&mut c);
}

#[test]
#[ignore = "needs the test servers: make imap-up, then make imap-test"]
fn two_cores_moving_the_same_mail_agree() {
    let user = new_user();
    let mut phone = Phone::open(FULL, &user);
    phone.create("Archive");
    phone.create("Todo");
    phone.deliver("INBOX", "both", &[]);
    let (a_dir, b_dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (_a, mut a) = core(a_dir.path(), FULL, &user, "");
    let (_b, mut b) = core(b_dir.path(), FULL, &user, "");
    settled(&mut a, 0);
    settled(&mut b, 0);
    let thread = |dir: &Path| -> i64 { db(dir).query_row("SELECT thread_id FROM message", [], |r| r.get(0)).unwrap() };
    // Two Macs on one mailbox file the same mail differently, at once.
    let (ta, tb) = (thread(a_dir.path()), thread(b_dir.path()));
    let pa = account(&mut a)["passes"].as_u64().unwrap();
    let pb = account(&mut b)["passes"].as_u64().unwrap();
    a.call("move_thread", json!({"thread_id": ta, "target": "archive"})).unwrap();
    b.call("move_thread", json!({"thread_id": tb, "target": "todo"})).unwrap();
    settled(&mut a, pa);
    settled(&mut b, pb);
    sync(&mut a);
    sync(&mut b);
    // Neither is an error for the user, the mail is not lost, and both
    // databases agree with the server. Two moves at the very same moment
    // may leave two copies on the server (each MOVE saw the mail before the
    // other expunged it): no client can prevent it, and NiceMailer then
    // shows both, as the server has them.
    let places: Vec<&str> = ["INBOX", "Archive", "Todo"].into_iter().filter(|f| phone.has(f, "both")).collect();
    assert!(!places.is_empty() && !places.contains(&"INBOX"), "{places:?}");
    for (dir, c) in [(a_dir.path(), &mut a), (b_dir.path(), &mut b)] {
        verify_ok(c);
        let n: i64 = db(dir).query_row("SELECT count(*) FROM message", [], |r| r.get(0)).unwrap();
        assert_eq!(n as usize, places.len());
        let reverted: i64 = db(dir).query_row("SELECT count(*) FROM pending_action WHERE state = 'reverted'", [], |r| r.get(0)).unwrap();
        assert_eq!(reverted, 0, "a mail moved by the other Mac is not a failure");
    }
}

#[test]
#[ignore = "needs the test servers: make imap-up, then make imap-test"]
fn a_crash_between_the_move_and_done_is_replayed() {
    let dir = tempfile::tempdir().unwrap();
    let user = new_user();
    let mut phone = Phone::open(FULL, &user);
    phone.create("Archive");
    phone.deliver("INBOX", "crash", &[]);
    let (core1, mut c) = core(dir.path(), FULL, &user, "");
    settled(&mut c, 0);
    let (id, _, _) = message_id_of(dir.path(), "crash");
    let thread: i64 = db(dir.path()).query_row("SELECT thread_id FROM message WHERE id = ?1", [id], |r| r.get(0)).unwrap();
    drop(c);
    drop(core1);
    // A core that dies right after the server moved the mail, before
    // marking the action done.
    let mut dying = start_with(dir.path(), &[("NICEMAILER_TEST_CRASH_AFTER_MOVE", "1")]);
    let mut c = Client::connect(dir.path());
    settled(&mut c, 0);
    let _ = c.call("move_thread", json!({"thread_id": thread, "target": "archive"}));
    let status = dying.wait().unwrap();
    assert!(!status.success(), "the core died at the crash point");
    assert!(phone.has("Archive", "crash"), "the server did move it");
    let state: String = db(dir.path()).query_row("SELECT state FROM pending_action", [], |r| r.get(0)).unwrap();
    assert_eq!(state, "running", "not marked done");
    // Started again: the action is queued again, finds the UID gone from
    // the Inbox, the mail in Archive, and ends done.
    let _core = start(dir.path());
    let mut c = Client::connect(dir.path());
    settled(&mut c, 0);
    let (id2, path, uid) = message_id_of(dir.path(), "crash");
    assert_eq!((id2, path.as_str()), (id, "Archive"));
    assert!(uid > 0);
    let state: String = db(dir.path()).query_row("SELECT state FROM pending_action", [], |r| r.get(0)).unwrap();
    assert_eq!(state, "done");
    verify_ok(&mut c);
}

impl Phone {
    /// Delivers a raw mail (headers and body as given) to `folder`.
    fn deliver_raw(&mut self, folder: &str, raw: &str) -> u32 {
        self.rt.block_on(self.s.append(&MailboxName::new(folder), &[], None, raw.as_bytes().to_vec())).unwrap().unwrap().uid
    }
}

/// A mail as the receiving server leaves it: its `Authentication-Results`
/// on top (the test Dovecot adds none, so the test writes the one a
/// receiving server `mx.test.nicemailer` would have).
fn received(id: &str, from: &str, auth: &str, extra: &str) -> String {
    format!(
        "Authentication-Results: mx.test.nicemailer; {auth}\r\nFrom: {from}\r\nTo: me@example.com\r\nSubject: Mail {id}\r\n\
         Message-ID: <{id}@test.nicemailer>\r\nDate: Thu, 05 Nov 2026 10:00:00 +0000\r\n{extra}\r\nBody of {id}.\r\n"
    )
}

#[test]
#[ignore = "needs the test servers: make imap-up, then make imap-test"]
fn new_mail_is_screened_on_arrival() {
    let dir = tempfile::tempdir().unwrap();
    let user = new_user();
    let mut phone = Phone::open(FULL, &user);
    for f in ["Archive", "Quarantine", "Newsletter", "Todo", "Sent"] {
        phone.create(f);
    }
    // Mails there before NiceMailer: never screened. They also teach the
    // core the id of the receiving server.
    for i in 0..5 {
        phone.deliver_raw("INBOX", &received(&format!("old{i}"), "stranger@far.example", "dmarc=pass", ""));
    }
    let (_core, mut c) = core(dir.path(), FULL, &user, "");
    let mut passes = settled(&mut c, 0);
    assert_eq!(account(&mut c)["authserv_id"], "mx.test.nicemailer", "detected on the Inbox");
    assert_eq!(phone.uids("INBOX").len(), 5, "the mails already there stay");

    // An unknown sender: Quarantine, on the server too.
    phone.deliver_raw("INBOX", &received("new1", "stranger@far.example", "dmarc=pass", ""));
    passes = settled(&mut c, passes);
    assert!(phone.has("Quarantine", "new1") && !phone.has("INBOX", "new1"));
    assert_eq!(message_id_of(dir.path(), "new1").1, "Quarantine");

    // An accepted sender, then a forgery of them with a fake `dmarc=pass`
    // written under the server's `fail`.
    c.call("set_address_screening", json!({"addr": "alice@a.example", "target": "inbox"})).unwrap();
    phone.deliver_raw("INBOX", &received("real", "alice@a.example", "dmarc=pass", ""));
    let forged = format!(
        "Authentication-Results: mx.test.nicemailer; dmarc=fail header.from=a.example\r\n{}",
        received("forged", "alice@a.example", "dmarc=pass", "")
    );
    phone.deliver_raw("INBOX", &forged);
    passes = settled(&mut c, passes);
    assert!(phone.has("INBOX", "real"));
    assert!(phone.has("Quarantine", "forged"), "the server's verdict wins");
    let suspect: i64 = db(dir.path())
        .query_row("SELECT auth_suspect FROM message WHERE message_id_hdr = 'forged@test.nicemailer'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(suspect, 1);

    // Alice's conversation waits in Todo; her answer brings it back,
    // marked on the server.
    let thread: i64 = db(dir.path()).query_row("SELECT thread_id FROM message WHERE message_id_hdr = 'real@test.nicemailer'", [], |r| r.get(0)).unwrap();
    c.call("move_thread", json!({"thread_id": thread, "target": "todo"})).unwrap();
    passes = settled(&mut c, passes);
    assert!(phone.has("Todo", "real"));
    phone.deliver_raw("INBOX", &received("answer", "alice@a.example", "dmarc=pass", "In-Reply-To: <real@test.nicemailer>\r\n"));
    passes = settled(&mut c, passes);
    let (_, path, uid) = message_id_of(dir.path(), "real");
    assert_eq!(path, "INBOX");
    let flags = phone.flags("INBOX", uid);
    assert!(flags.iter().any(|f| f.eq_ignore_ascii_case("$NmWasTodo")), "{flags:?}");

    // Sent from the phone to the stranger: accepted, out of Quarantine.
    phone.deliver_raw(
        "Sent",
        "From: me@example.com\r\nTo: stranger@far.example\r\nSubject: hello\r\nMessage-ID: <sent1@test.nicemailer>\r\n\r\nHi.\r\n",
    );
    c.call("sync_now", json!({"account": "dovecot"})).unwrap();
    settled(&mut c, passes);
    // Its filing went through the spool: one more round to be sure.
    sync(&mut c);
    assert!(phone.has("INBOX", "new1"), "the stranger's mail left the Quarantine");
    verify_ok(&mut c);
}

#[test]
#[ignore = "needs the test servers: make imap-up, then make imap-test"]
fn a_damaged_raw_file_is_fetched_again_and_a_deleted_mail_collected() {
    let dir = tempfile::tempdir().unwrap();
    let user = new_user();
    let mut phone = Phone::open(FULL, &user);
    phone.deliver("INBOX", "a", &[]);
    let b = phone.deliver("INBOX", "b", &[]);
    let (_core, mut c) = core(dir.path(), FULL, &user, "");
    let passes = settled(&mut c, 0);
    // The raw file of `a` is damaged on disk.
    let hash: Vec<u8> = db(dir.path())
        .query_row(
            "SELECT raw_hash FROM message WHERE message_id_hdr = 'a@test.nicemailer'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let hex: String = hash.iter().map(|x| format!("{x:02x}")).collect();
    let path = dir
        .path()
        .join("raw.noindex")
        .join(&hex[..2])
        .join(format!("{hex}.eml.zst"));
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(&path, b"not zstd at all").unwrap();
    let r = c
        .call("sync_verify", json!({"account": "dovecot", "repair": true}))
        .unwrap();
    assert_eq!(r["ok"], false, "the damage is reported");
    assert_eq!(r["repaired"], 1);
    // The body connection downloads it again (no pass needed).
    let _ = passes;
    let deadline = Instant::now() + Duration::from_secs(30);
    while db(dir.path())
        .query_row(
            "SELECT body_state FROM message WHERE message_id_hdr = 'a@test.nicemailer'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
        == 0
    {
        assert!(Instant::now() < deadline, "not downloaded again");
        std::thread::sleep(Duration::from_millis(100));
    }
    verify_ok(&mut c);
    assert!(
        std::fs::read_dir(dir.path().join("raw.noindex/.corrupt"))
            .unwrap()
            .count()
            == 1
    );

    // The phone deletes `b`: once synchronized, its raw file has no mail.
    phone.select("INBOX");
    phone.expunge(b);
    sync(&mut c);
    let r = c.call("raw_gc", json!({})).unwrap();
    assert_eq!(r["blobs"], 1, "{r}");
    verify_ok(&mut c);
}

/// Actions of the spool queued before an update run after it, once each
/// (`mise-a-jour.md`, section 7): five moves, then at once the app prepares
/// the update (the workers stop, what was not sent stays queued), the core
/// is stopped as `launchctl kickstart -k` does, and the new core sends the
/// rest. The server and the database then agree, each mail in Archive once.
#[test]
#[ignore = "needs the test servers: make imap-up, then make imap-test"]
fn actions_queued_before_an_update_run_after_it_once() {
    let dir = tempfile::tempdir().unwrap();
    let user = new_user();
    let mut phone = Phone::open(FULL, &user);
    phone.create("Archive");
    let ids = ["u1", "u2", "u3", "u4", "u5"];
    for id in ids {
        phone.deliver("INBOX", id, &[]);
    }
    let (core, mut c) = core(dir.path(), FULL, &user, "");
    settled(&mut c, 0);
    let mut app = Client::connect_as(dir.path(), "app");
    for id in ids {
        let (m, _, _) = message_id_of(dir.path(), id);
        let thread: i64 = db(dir.path()).query_row("SELECT thread_id FROM message WHERE id = ?1", [m], |r| r.get(0)).unwrap();
        c.call("move_thread", json!({"thread_id": thread, "target": "archive"})).unwrap();
    }
    app.call("prepare_update", json!({})).unwrap();
    // Nothing is served now: the CLI's connection was closed.
    assert!(Client::try_connect_as(dir.path(), "cli").is_err());
    let mut core = core;
    unsafe { libc::kill(core.pid() as i32, libc::SIGTERM) };
    assert!(core.exit_status(20).success());

    let _core = start(dir.path());
    let mut c = Client::connect_until_served(dir.path(), "cli");
    settled(&mut c, 0);
    verify_ok(&mut c);
    for id in ids {
        assert_eq!(message_id_of(dir.path(), id).1, "Archive", "{id}");
    }
    let pending: i64 = db(dir.path())
        .query_row("SELECT count(*) FROM pending_action WHERE state IN ('queued', 'running')", [], |r| r.get(0))
        .unwrap();
    assert_eq!(pending, 0);
    // No move sent twice: one action done per mail.
    let done: i64 = db(dir.path())
        .query_row("SELECT count(*) FROM pending_action WHERE kind = 'move' AND state = 'done'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(done, ids.len() as i64);
    let archived: i64 = db(dir.path())
        .query_row("SELECT count(*) FROM message m JOIN folder f ON f.id = m.folder_id WHERE f.path = 'Archive'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(archived, ids.len() as i64, "each mail once");
}
