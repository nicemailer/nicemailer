//! Creating an account and its initial import, end to end
//! (`import-initial.md`, section 11; `plan-imap.md`, phase 7): the real
//! core, a test Dovecot, and a second connection playing the phone.
//!
//! `#[ignore]`: needs the test servers (`make imap-up`, then
//! `make imap-test`). Each test runs on a new random user.

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::{Client, start};
use nm_proto::imap::{MailboxName, SearchKey, Security, ServerSpec, Session, Timeouts};
use serde_json::{Value, json};

const PORT: u16 = 31993;

fn ca() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../dev/dovecot/tls/ca.crt")
}

fn new_user() -> String {
    format!("i{:016x}", rand::random::<u64>())
}

struct Phone {
    rt: tokio::runtime::Runtime,
    s: Session,
}

impl Phone {
    fn open(user: &str) -> Phone {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let s = rt.block_on(async {
            let spec = ServerSpec { host: "127.0.0.1".into(), port: PORT, security: Security::Tls, ca_file: Some(ca()) };
            let mut s = Session::connect(&spec, Timeouts::default()).await.unwrap();
            s.login(user, "nicemailer").await.unwrap();
            s
        });
        Phone { rt, s }
    }

    fn create(&mut self, name: &str) {
        let _ = self.rt.block_on(self.s.create(&MailboxName::new(name)));
    }

    /// A mail from `from`, received `days_ago` days ago, with `flags`.
    fn deliver(&mut self, folder: &str, id: &str, from: &str, to: &str, flags: &[&str], days_ago: i64) {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
        let mail = format!(
            "From: {from}\r\nTo: {to}\r\nSubject: Mail {id}\r\nMessage-ID: <{id}@test.nicemailer>\r\n\
             Date: Thu, 05 Nov 2026 10:00:00 +0000\r\n\r\nBody of {id}.\r\n"
        );
        self.rt.block_on(self.s.append(&MailboxName::new(folder), flags, Some(now - days_ago * 86_400), mail.into_bytes())).unwrap();
    }

    fn has(&mut self, folder: &str, id: &str) -> bool {
        self.rt.block_on(self.s.examine(&MailboxName::new(folder), None)).unwrap();
        let key = SearchKey::Header("Message-ID".into(), format!("<{id}@test.nicemailer>"));
        !self.rt.block_on(self.s.uid_search(&key)).unwrap().is_empty()
    }

    fn count(&mut self, folder: &str) -> usize {
        self.rt.block_on(self.s.examine(&MailboxName::new(folder), None)).unwrap();
        self.rt.block_on(self.s.uid_search(&SearchKey::All)).unwrap().len()
    }
}

fn imap(user: &str) -> Value {
    json!({ "host": "127.0.0.1", "port": PORT, "security": "tls", "username": user, "ca_file": ca().display().to_string() })
}

fn email(user: &str) -> String {
    format!("{user}@example.com")
}

fn status(c: &mut Client, key: &str) -> Value {
    c.call("sync_status", json!({"account": key})).unwrap()["accounts"][0].clone()
}

/// Waits until the account's worker made a pass and is idle, with every
/// body and action done.
fn settled(c: &mut Client, key: &str) {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        let a = c.call("sync_status", json!({"account": key})).ok().map(|v| v["accounts"][0].clone());
        if let Some(a) = &a
            && a["passes"].as_u64().unwrap_or(0) > 0
            && a["state"] == "idle"
            && a["bodies_missing"] == 0
            && a["actions_pending"] == 0
        {
            return;
        }
        assert!(Instant::now() < deadline, "not settled: {a:?}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn db(dir: &Path) -> nm_store::rusqlite::Connection {
    nm_store::rusqlite::Connection::open_with_flags(dir.join("nicemailer.db"), nm_store::rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap()
}

fn int(dir: &Path, sql: &str) -> i64 {
    db(dir).query_row(sql, [], |r| r.get(0)).unwrap()
}

fn verify_ok(c: &mut Client, key: &str) {
    let r = c.call("sync_verify", json!({"account": key})).unwrap();
    assert_eq!(r["ok"], true, "{r:#}");
}

/// A core with `config` as its settings.
fn core(dir: &Path, config: &str) -> (common::Core, Client) {
    std::fs::write(dir.join("config.toml"), config).unwrap();
    let core = start(dir);
    let c = Client::connect(dir);
    (core, c)
}

#[test]
#[ignore = "needs the test servers: make imap-up, then make imap-test"]
fn a_small_inbox_stays_and_the_people_written_to_are_accepted() {
    let dir = tempfile::tempdir().unwrap();
    let user = new_user();
    let me = email(&user);
    let mut phone = Phone::open(&user);
    phone.deliver("INBOX", "a", "stranger@far.example", &me, &[], 3);
    phone.deliver("INBOX", "b", "carol@c.example", &me, &["\\Seen"], 2);
    phone.deliver("Sent", "s", &me, &format!("bob@b.example, {me}"), &["\\Seen"], 1);
    // An accounts.toml of the user's, with a comment.
    std::fs::write(dir.path().join("accounts.toml"), "# My accounts.\n").unwrap();
    let (_core, mut c) = core(dir.path(), "# My settings.\n");

    let r = c.call("test_account", json!({"email": me, "imap": imap(&user), "password": "nicemailer"})).unwrap();
    assert_eq!(r["inbox_messages"], 2);
    assert_eq!(r["offer_archive"], false, "under the thresholds");
    assert_eq!(r["correspondents"], 1, "Bob; not oneself");
    let roles: Vec<&str> = r["probe"]["folders"].as_array().unwrap().iter().filter_map(|f| f["proposed_role"].as_str()).collect();
    assert!(roles.contains(&"sent"));

    // A mail arrives while the user reads the import screen: it is new, and
    // screened.
    phone.deliver("INBOX", "during", "someone@new.example", &me, &[], 0);
    let res = c
        .call(
            "add_account",
            json!({"key": "dovecot", "email": me, "imap": imap(&user), "password": "nicemailer", "uidnext": r["inbox_uidnext"]}),
        )
        .unwrap();
    let created: Vec<&str> = res["created"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    assert!(created.contains(&"Quarantine") && created.contains(&"Todo"), "{created:?}");
    assert_eq!(res["correspondents"], 1);
    settled(&mut c, "dovecot");

    // accounts.toml has the account, its comment kept; config.toml is
    // not touched.
    let file = std::fs::read_to_string(dir.path().join("accounts.toml")).unwrap();
    assert!(file.starts_with("# My accounts.") && file.contains("[accounts.dovecot]"), "{file}");
    assert_eq!(std::fs::read_to_string(dir.path().join("config.toml")).unwrap(), "# My settings.\n");
    // The old mails stay, unscreened; the new one was screened.
    assert!(phone.has("INBOX", "a") && phone.has("INBOX", "b"));
    assert!(phone.has("Quarantine", "during"));
    assert_eq!(int(dir.path(), "SELECT count(*) FROM audit_log WHERE action = 'screen'"), 1);
    // Bob was accepted; the senders found are marked imported.
    assert_eq!(int(dir.path(), "SELECT count(*) FROM address WHERE addr = 'bob@b.example' AND screening_target = 'inbox'"), 1);
    assert_eq!(int(dir.path(), "SELECT count(*) FROM address WHERE addr = 'stranger@far.example' AND imported_from = 'imap_initial'"), 1);
    verify_ok(&mut c, "dovecot");

    // After the import: the next mail of an imported sender without
    // decision goes to Quarantine, Bob's to the Inbox.
    phone.deliver("INBOX", "again", "stranger@far.example", &me, &[], 0);
    phone.deliver("INBOX", "bob2", "bob@b.example", &me, &[], 0);
    c.call("sync_now", json!({"account": "dovecot"})).unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while !(phone.has("Quarantine", "again") && phone.has("INBOX", "bob2")) {
        assert!(Instant::now() < deadline, "not screened");
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// A mailbox above the thresholds: read mail of 60 days and of today,
/// unread mail of 60 days and of 2 days, a flagged mail of 60 days.
fn big_inbox(phone: &mut Phone, me: &str) {
    phone.create("Archive");
    phone.deliver("INBOX", "read_old", "a@a.example", me, &["\\Seen"], 60);
    phone.deliver("INBOX", "read_new", "a@a.example", me, &["\\Seen"], 0);
    phone.deliver("INBOX", "unread_old", "a@a.example", me, &[], 60);
    phone.deliver("INBOX", "unread_new", "a@a.example", me, &[], 2);
    phone.deliver("INBOX", "flagged_old", "a@a.example", me, &["\\Seen", "\\Flagged"], 60);
}

const SMALL_THRESHOLDS: &str = "[import]\nthreshold_total = 2\nbatch_size = 1\n";

fn add_with_defaults(c: &mut Client, user: &str) -> Value {
    add_with_defaults_maybe(c, user).unwrap()
}

/// The same, the answer of `add_account` returned as it came: a core that
/// dies at once may not send it.
fn add_with_defaults_maybe(c: &mut Client, user: &str) -> Result<Value, Value> {
    let me = email(user);
    let r = c.call("test_account", json!({"email": me, "imap": imap(user), "password": "nicemailer"})).unwrap();
    assert_eq!(r["offer_archive"], true);
    // Defaults: every read mail, the unread ones older than 30 days; never
    // the flagged ones.
    assert_eq!((r["archive_read"].as_u64(), r["archive_unread"].as_u64()), (Some(2), Some(1)), "{r:#}");
    let n = c
        .call(
            "count_archive",
            json!({"email": me, "imap": imap(user), "password": "nicemailer", "uidnext": r["inbox_uidnext"],
                   "read_before": r["read_before"], "unread_before": r["unread_before"]}),
        )
        .unwrap();
    assert_eq!((n["read"].as_u64(), n["unread"].as_u64()), (Some(2), Some(1)));
    c.call(
        "add_account",
        json!({"key": "dovecot", "email": me, "imap": imap(user), "password": "nicemailer", "uidnext": r["inbox_uidnext"],
               "archive_read_before": r["read_before"], "archive_unread_before": r["unread_before"]}),
    )
}

fn archived_as_counted(phone: &mut Phone, dir: &Path) {
    for id in ["read_old", "read_new", "unread_old"] {
        assert!(phone.has("Archive", id), "{id} archived");
    }
    for id in ["unread_new", "flagged_old"] {
        assert!(phone.has("INBOX", id), "{id} stays");
    }
    assert_eq!((phone.count("INBOX"), phone.count("Archive")), (2, 3), "nothing lost, nothing twice");
    assert_eq!(int(dir, "SELECT json_extract(detail, '$.moved') FROM audit_log WHERE action = 'initial_archive'"), 3, "as counted");
    assert_eq!(int(dir, "SELECT count(*) FROM message m JOIN folder f ON f.id = m.folder_id WHERE f.role = 'archive'"), 3);
    assert_eq!(int(dir, "SELECT count(*) FROM audit_log WHERE action = 'screen'"), 0, "none screened");
}

#[test]
#[ignore = "needs the test servers: make imap-up, then make imap-test"]
fn old_mail_is_archived_on_the_server_as_counted() {
    let dir = tempfile::tempdir().unwrap();
    let user = new_user();
    let mut phone = Phone::open(&user);
    big_inbox(&mut phone, &email(&user));
    let (_core, mut c) = core(dir.path(), SMALL_THRESHOLDS);
    add_with_defaults(&mut c, &user);
    settled(&mut c, "dovecot");
    assert_eq!(status(&mut c, "dovecot")["initial_archive"], "done");
    archived_as_counted(&mut phone, dir.path());
    verify_ok(&mut c, "dovecot");
}

#[test]
#[ignore = "needs the test servers: make imap-up, then make imap-test"]
fn an_archiving_interrupted_resumes_without_loss_or_duplicate() {
    let dir = tempfile::tempdir().unwrap();
    let user = new_user();
    let mut phone = Phone::open(&user);
    big_inbox(&mut phone, &email(&user));
    std::fs::write(dir.path().join("config.toml"), SMALL_THRESHOLDS).unwrap();
    // A first core that dies after the first batch of one mail.
    let mut dying = {
        let keychain = dir.path().join("test.keychain-db");
        for args in [["create-keychain", "-p", "test"], ["unlock-keychain", "-p", "test"]] {
            assert!(std::process::Command::new("/usr/bin/security").args(args).arg(&keychain).status().unwrap().success());
        }
        std::process::Command::new(env!("CARGO_BIN_EXE_nicemailer-core"))
            .arg("--data-dir")
            .arg(dir.path())
            .arg("--config")
            .arg(dir.path().join("config.toml"))
            .arg("--keychain")
            .arg(&keychain)
            .env("NICEMAILER_TEST_CRASH_DURING_ARCHIVE", "1")
            .spawn()
            .unwrap()
    };
    let mut c = Client::connect(dir.path());
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| add_with_defaults_maybe(&mut c, &user)));
    assert!(!dying.wait().unwrap().success(), "died during the archiving");
    assert_eq!(phone.count("Archive"), 1, "one batch done");
    // Started again: the search no longer finds what moved, the rest moves.
    let _core = start(dir.path());
    let mut c = Client::connect(dir.path());
    settled(&mut c, "dovecot");
    archived_as_counted_after_crash(&mut phone, dir.path());
    verify_ok(&mut c, "dovecot");
}

/// After a crash, the count of the audit is what the second run moved.
fn archived_as_counted_after_crash(phone: &mut Phone, dir: &Path) {
    assert_eq!((phone.count("INBOX"), phone.count("Archive")), (2, 3));
    assert_eq!(int(dir, "SELECT json_extract(detail, '$.moved') FROM audit_log WHERE action = 'initial_archive'"), 2);
    assert_eq!(int(dir, "SELECT count(*) FROM message"), 5, "each mail once");
}

#[test]
#[ignore = "needs the test servers: make imap-up, then make imap-test"]
fn an_account_is_renamed_with_its_data_and_removed_with_them() {
    let dir = tempfile::tempdir().unwrap();
    let user = new_user();
    let mut phone = Phone::open(&user);
    phone.deliver("INBOX", "a", "a@a.example", &email(&user), &[], 1);
    let (_core, mut c) = core(dir.path(), "");
    c.call("add_account", json!({"key": "dovecot", "email": email(&user), "imap": imap(&user), "password": "nicemailer"})).unwrap();
    settled(&mut c, "dovecot");
    let id = int(dir.path(), "SELECT id FROM account WHERE config_key = 'dovecot'");

    c.call("rename_account", json!({"from": "dovecot", "to": "mail"})).unwrap();
    let file = std::fs::read_to_string(dir.path().join("accounts.toml")).unwrap();
    assert!(file.contains("[accounts.mail]") && !file.contains("accounts.dovecot"), "{file}");
    assert_eq!(int(dir.path(), "SELECT id FROM account WHERE config_key = 'mail'"), id, "same account");
    let item = |key: &str| {
        std::process::Command::new("/usr/bin/security")
            .args(["find-generic-password", "-s", "NiceMailer", "-a", &format!("imap:{key}")])
            .arg(dir.path().join("test.keychain-db"))
            .status()
            .unwrap()
            .success()
    };
    assert!(item("mail") && !item("dovecot"), "the password followed");
    settled(&mut c, "mail");
    verify_ok(&mut c, "mail");

    c.call("remove_account", json!({"account": "mail"})).unwrap();
    let file = std::fs::read_to_string(dir.path().join("accounts.toml")).unwrap();
    assert!(!file.contains("accounts.mail"), "{file}");
    assert_eq!(int(dir.path(), "SELECT count(*) FROM account") + int(dir.path(), "SELECT count(*) FROM message"), 0);
    assert!(!item("mail"));
    assert!(phone.has("INBOX", "a"), "the server is not touched");
    // An agent may do none of it.
    let mut agent = Client::connect_as(dir.path(), "mcp");
    let err = agent.call("add_account", json!({"key": "x", "email": "x@y.example", "imap": imap(&user), "password": "p"})).unwrap_err();
    assert_eq!(err["code"], 1007);
}

/// Each folder of a server, and the UIDs and flags of its mails.
type Snapshot = Vec<(String, Vec<(u32, Vec<String>)>)>;

impl Phone {
    /// Everything the server shows.
    fn snapshot(&mut self) -> Snapshot {
        let mut folders: Vec<String> = self
            .rt
            .block_on(self.s.list())
            .unwrap()
            .into_iter()
            .map(|e| e.name.name)
            .collect();
        folders.sort();
        folders
            .into_iter()
            .map(|f| {
                let sel = self
                    .rt
                    .block_on(self.s.examine(&MailboxName::new(&f), None))
                    .unwrap();
                let mut mails = Vec::new();
                if sel.exists > 0 {
                    let items = nm_proto::imap::FetchItems {
                        flags: true,
                        ..Default::default()
                    };
                    for m in self
                        .rt
                        .block_on(self.s.uid_fetch(&nm_proto::imap::Uids::From(1), &items))
                        .unwrap()
                    {
                        let mut fl = m.flags.unwrap_or_default();
                        fl.sort();
                        mails.push((m.uid.unwrap(), fl));
                    }
                }
                mails.sort();
                (f, mails)
            })
            .collect()
    }
}

#[test]
#[ignore = "needs the test servers: make imap-up, then make imap-test"]
fn a_read_only_account_changes_nothing_on_its_server() {
    let dir = tempfile::tempdir().unwrap();
    let user = new_user();
    let me = email(&user);
    let mut phone = Phone::open(&user);
    phone.deliver("INBOX", "kept", "stranger@far.example", &me, &[], 3);
    phone.deliver("Sent", "s", &me, "bob@b.example", &["\\Seen"], 1);
    let (_core, mut c) = core(dir.path(), "");
    // Archiving is refused on a read-only account.
    let err = c
        .call("add_account", json!({"key": "prod", "email": me, "imap": imap(&user), "password": "nicemailer", "read_only": true,
                                     "archive_read_before": "2026-11-01"}))
        .unwrap_err();
    assert_eq!(err["code"], -32602, "{err}");
    let before = phone.snapshot();
    let res = c.call("add_account", json!({"key": "prod", "email": me, "imap": imap(&user), "password": "nicemailer", "read_only": true})).unwrap();
    assert_eq!(res["created"], json!([]), "no folder created");
    settled(&mut c, "prod");
    assert_eq!(status(&mut c, "prod")["read_only"], true);
    let file = std::fs::read_to_string(dir.path().join("accounts.toml")).unwrap();
    assert!(file.contains("read_only = true"), "{file}");

    // New mail: screened dry, left where it is.
    phone.deliver("INBOX", "new", "someone@new.example", &me, &[], 0);
    c.call("sync_now", json!({"account": "prod"})).unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while int(
        dir.path(),
        "SELECT count(*) FROM audit_log WHERE action = 'screen'",
    ) == 0
    {
        assert!(Instant::now() < deadline, "not screened");
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(
        int(
            dir.path(),
            "SELECT json_extract(detail, '$.dry_run') FROM audit_log WHERE action = 'screen'"
        ),
        1
    );
    // Commands are refused.
    let thread = int(
        dir.path(),
        "SELECT thread_id FROM message WHERE message_id_hdr = 'kept@test.nicemailer'",
    );
    for (method, params) in [
        (
            "set_thread_seen",
            json!({"thread_id": thread, "seen": true}),
        ),
        (
            "move_thread",
            json!({"thread_id": thread, "target": "deleted"}),
        ),
        ("tag_thread", json!({"thread_id": thread, "name": "x"})),
    ] {
        assert_eq!(
            c.call(method, params).unwrap_err()["code"],
            1010,
            "{method}"
        );
    }
    settled(&mut c, "prod");
    verify_ok(&mut c, "prod");
    // The server is as it was, plus the mail the phone delivered.
    let mut after = phone.snapshot();
    for (f, mails) in after.iter_mut() {
        if f == "INBOX" {
            mails.pop();
        }
    }
    assert_eq!(after, before, "nothing changed on the server");
}

/// Mailpit with TLS from the first byte and a real password
/// (`dev/dovecot/compose.yaml`, `smtps`).
const SMTPS: u16 = 31465;

fn smtps(username: &str) -> Value {
    json!({ "host": "127.0.0.1", "port": SMTPS, "security": "tls", "username": username, "ca_file": ca().display().to_string() })
}

/// `docs/accounts.md`, "Finding the servers": the SMTP servers tried in
/// order, the first that works kept, the IMAP password sent only to a
/// server of the IMAP server's organization; then `add_account` writes
/// the server kept.
#[test]
#[ignore = "needs the test servers: make imap-up, then make imap-test"]
fn smtp_servers_are_tried_in_order_and_the_one_that_works_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let user = new_user();
    let me = email(&user);
    let (_core, mut c) = core(dir.path(), "");
    let nothing = json!({ "host": "127.0.0.1", "port": 1, "security": "tls" });
    let candidates = json!([
        { "server": nothing, "origin": "same_host" },
        { "server": smtps("nicemailer"), "origin": "same_host" },
        { "server": smtps("nicemailer"), "origin": "same_host" },
    ]);

    // Nothing listens on port 1; the next one accepts the password; the
    // third is not tried.
    let r = c.call("test_smtp", json!({"email": me, "imap": imap(&user), "password": "smtp-secret", "candidates": candidates})).unwrap();
    let outcomes: Vec<&str> = r["attempts"].as_array().unwrap().iter().map(|a| a["outcome"].as_str().unwrap()).collect();
    assert_eq!(outcomes, ["failed", "logged_in"], "{r:#}");
    assert_eq!(r["chosen"], 1);

    // A wrong password: refused, nothing chosen.
    let r = c
        .call("test_smtp", json!({"email": me, "imap": imap(&user), "password": "wrong", "candidates": [{"server": smtps("nicemailer"), "origin": "given"}]}))
        .unwrap();
    assert_eq!(r["attempts"][0]["outcome"], "failed");
    assert!(r["attempts"][0]["detail"].as_str().unwrap().contains("refused the login"), "{r:#}");
    assert_eq!(r["chosen"], Value::Null);

    // An IMAP server of another organization: the password is not tried,
    // the server is kept as reachable.
    let other = json!({ "host": "imap.other.example", "port": 993, "security": "tls" });
    let r = c
        .call("test_smtp", json!({"email": me, "imap": other, "password": "wrong", "candidates": [{"server": smtps("nicemailer"), "origin": "given"}]}))
        .unwrap();
    assert_eq!((r["attempts"][0]["outcome"].as_str(), r["chosen"].as_u64()), (Some("reachable"), Some(0)), "{r:#}");

    // The server the user gave, with its SMTP guesses.
    let r = c.call("discover_account", json!({"email": me, "imap": imap(&user)})).unwrap();
    assert_eq!(r["imap"]["origin"], "given");
    let smtp: Vec<String> = r["smtp"].as_array().unwrap().iter().map(|s| format!("{}:{}", s["server"]["host"].as_str().unwrap(), s["server"]["port"])).collect();
    assert_eq!(smtp, ["127.0.0.1:465", "127.0.0.1:587"]);
    // The first account of example.com: named after the domain.
    assert_eq!((r["key"].as_str(), r["key_source"].as_str()), (Some("example"), Some("domain")), "{r:#}");

    // The account created with the server kept: [smtp] in the file.
    c.call(
        "add_account",
        json!({"key": "dovecot", "email": me, "imap": imap(&user), "password": "nicemailer", "smtp": smtps("nicemailer")}),
    )
    .unwrap();
    // A second address of example.com: the whole address, because of it.
    let r = c.call("discover_account", json!({"email": "other@example.com", "imap": imap(&user)})).unwrap();
    assert_eq!((r["key"].as_str(), r["domain_account"].as_str()), (Some("other_example_com"), Some("dovecot")), "{r:#}");
    let file = std::fs::read_to_string(dir.path().join("accounts.toml")).unwrap();
    assert!(file.contains(&format!("[accounts.dovecot.smtp]\nhost = \"127.0.0.1\"\nport = {SMTPS}\nsecurity = \"tls\"\nusername = \"nicemailer\"\n")), "{file}");

    // An agent may not.
    let mut agent = Client::connect_as(dir.path(), "mcp");
    for (method, params) in [
        ("discover_account", json!({"email": me})),
        ("test_smtp", json!({"email": me, "imap": imap(&user), "password": "x", "candidates": []})),
    ] {
        assert_eq!(agent.call(method, params).unwrap_err()["code"], 1007, "{method}");
    }
}
