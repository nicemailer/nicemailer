//! Sending a reply end to end (`specifications/reponse.md`): the real core,
//! a test Dovecot for IMAP, Mailpit for SMTP (`dev/dovecot/compose.yaml`),
//! and a second IMAP connection playing the phone, which delivers the mail
//! answered and checks the server afterwards.
//!
//! `#[ignore]`: needs the test servers (`make imap-up`, then
//! `make imap-test`). Every test runs on a new random user and only looks
//! at the mails of its own `Message-ID` in Mailpit, which the tests share.

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::{Client, start};
use nm_proto::imap::{MailboxName, SearchKey, Security, ServerSpec, Session, StoreOp, Timeouts, UidSet, Uids};
use serde_json::{Value, json};

const FULL: u16 = 31993;
const MINIMAL: u16 = 32993;
/// Mailpit with STARTTLS, any password; refuses recipients outside
/// `@example.com` and `@ok.example`.
const SMTP: u16 = 31587;
const SMTP_API: u16 = 38025;
/// Mailpit with TLS and a real password.
const SMTPS: u16 = 31465;
const SMTPS_API: u16 = 38026;

fn ca() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../dev/dovecot/tls/ca.crt")
}

/// The test account, for `accounts.toml`.
fn accounts(port: u16, user: &str, smtp: &str) -> String {
    format!(
        "[accounts.dovecot]\nemail = \"{user}@example.com\"\ndisplay_name = \"Test User\"\n\
         [accounts.dovecot.imap]\nhost = \"127.0.0.1\"\nport = {port}\nsecurity = \"tls\"\nusername = \"{user}\"\nca_file = \"{ca}\"\n\
         [accounts.dovecot.smtp]\nhost = \"127.0.0.1\"\nca_file = \"{ca}\"\n{smtp}",
        ca = ca().display()
    )
}

fn starttls() -> String {
    format!("port = {SMTP}\nsecurity = \"starttls\"\n")
}

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

    /// Alice writes to `user`, Carol and `cc` in copy.
    fn deliver(&mut self, user: &str, id: &str, cc: &str) {
        let mail = format!(
            "From: Alice <alice@example.com>\r\nTo: Test User <{user}@example.com>\r\nCc: carol@ok.example{cc}\r\n\
             Subject: Devis {id}\r\nMessage-ID: <{id}@test.nicemailer>\r\nDate: Sat, 07 Nov 2026 10:00:00 +0000\r\n\r\n\
             Bonjour,\r\nvoici le devis.\r\n"
        );
        self.rt.block_on(self.s.append(&MailboxName::inbox(), &[], None, mail.into_bytes())).unwrap();
    }

    /// Deletes the mail `uid` of `folder`, as the phone would.
    fn delete(&mut self, folder: &str, uid: u32) {
        self.rt.block_on(self.s.select(&MailboxName::new(folder), None)).unwrap();
        let set = Uids::Set(UidSet::from_uids([uid]));
        self.rt.block_on(self.s.uid_store(&set, StoreOp::Add, &["\\Deleted"], true, None)).unwrap();
        self.rt.block_on(self.s.uid_expunge(&set)).unwrap();
    }

    /// The UIDs of the mails of `folder` with this `Message-ID`.
    fn find(&mut self, folder: &str, msgid: &str) -> Vec<u32> {
        self.rt.block_on(self.s.examine(&MailboxName::new(folder), None)).unwrap();
        self.rt.block_on(self.s.uid_search(&SearchKey::Header("Message-ID".into(), format!("<{msgid}>")))).unwrap()
    }
}

fn new_user() -> String {
    format!("r{:016x}", rand::random::<u64>())
}

fn db(dir: &Path) -> nm_store::rusqlite::Connection {
    nm_store::rusqlite::Connection::open_with_flags(dir.join("nicemailer.db"), nm_store::rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap()
}

fn account(c: &mut Client) -> Value {
    c.call("sync_status", json!({"account": "dovecot"})).unwrap()["accounts"][0].clone()
}

/// Waits for a pass after `passes`, every body and every action done.
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

fn sync(c: &mut Client) -> u64 {
    let before = account(c)["passes"].as_u64().unwrap_or(0);
    c.call("sync_now", json!({"account": "dovecot"})).unwrap();
    settled(c, before)
}

/// The id of the mail `id` in the database, once synchronized.
fn message(dir: &Path, id: &str) -> i64 {
    db(dir).query_row("SELECT id FROM message WHERE message_id_hdr = ?1", [format!("{id}@test.nicemailer")], |r| r.get(0)).unwrap()
}

/// The mails Mailpit received with this `Message-ID`: raw sources.
fn received(api: u16, msgid: &str) -> Vec<String> {
    let list: Value = ureq::get(&format!("http://127.0.0.1:{api}/api/v1/messages?limit=1000"))
        .call()
        .unwrap()
        .body_mut()
        .read_json()
        .unwrap();
    list["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["MessageID"] == msgid)
        .map(|m| {
            ureq::get(&format!("http://127.0.0.1:{api}/api/v1/message/{}/raw", m["ID"].as_str().unwrap()))
                .call()
                .unwrap()
                .body_mut()
                .read_to_string()
                .unwrap()
        })
        .collect()
}

fn wait_until(what: &str, secs: u64, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while !f() {
        assert!(Instant::now() < deadline, "not within {secs} s: {what}");
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// The core with the account's IMAP password saved, synchronized once,
/// subscribed to the events.
fn core(dir: &Path, port: u16, user: &str, smtp: &str, delay: u64) -> (common::Core, Client) {
    std::fs::write(dir.join("config.toml"), format!("[compose]\nundo_send_seconds = {delay}\n")).unwrap();
    std::fs::write(dir.join("accounts.toml"), accounts(port, user, smtp)).unwrap();
    let core = start(dir);
    let mut c = Client::connect(dir);
    c.call("set_password", json!({"account": "dovecot", "password": "nicemailer"})).unwrap();
    settled(&mut c, 0);
    c.call("subscribe", json!({})).unwrap();
    (core, c)
}

/// The whole path of a reply: prepared by the core, sent through SMTP,
/// copied to Sent with its UID, the conversation filed in Archive on the
/// server, Carol accepted, and nothing twice after the next pass.
fn a_reply_goes_out_and_files_its_conversation(port: u16) {
    let dir = tempfile::tempdir().unwrap();
    let user = new_user();
    let mut phone = Phone::open(port, &user);
    phone.create("Archive");
    phone.create("Sent");
    phone.deliver(&user, "q1", "");
    let (_core, mut c) = core(dir.path(), port, &user, &starttls(), 1);
    let m = message(dir.path(), "q1");

    let d = c.call("create_draft", json!({"reply_to": m, "quote_header": "On Nov 7, 2026, at 10:00, Alice <alice@example.com> wrote:"})).unwrap();
    assert_eq!(d["reused"], false);
    let draft = &d["draft"];
    assert_eq!(draft["to"][0]["addr"], "alice@example.com");
    assert_eq!(draft["cc"][0]["addr"], "carol@ok.example");
    assert_eq!(draft["subject"], "Re: Devis q1");
    assert_eq!(draft["after_send_target"], "archive");
    let body = format!("Merci, c'est noté.{}", draft["body"].as_str().unwrap());
    let r = c.call("send_draft", json!({"id": draft["id"], "revision": draft["revision"], "body": body})).unwrap();
    assert!(r["send_at"].as_i64().is_some());

    let sent = c.event("DraftSent", 60);
    assert_eq!(sent["draft_id"], draft["id"]);
    assert_eq!(sent["moved_to"], "archive");
    assert!(sent["undo"].as_str().is_some());
    let hdr: String = db(dir.path()).query_row("SELECT message_id_hdr FROM message WHERE id = ?1", [sent["message_id"].as_i64().unwrap()], |r| r.get(0)).unwrap();

    // What Mailpit received: one mail, with the headers of a reply.
    let got = received(SMTP_API, &hdr);
    assert_eq!(got.len(), 1, "{got:?}");
    let raw = &got[0];
    assert!(raw.contains("In-Reply-To: <q1@test.nicemailer>"), "{raw}");
    assert!(raw.contains("Subject: Re: Devis q1"), "{raw}");
    assert!(raw.contains(&format!("From: Test User <{user}@example.com>")), "{raw}");
    assert!(raw.contains("Merci, c'est not=C3=A9."), "{raw}");
    assert!(raw.contains("> voici le devis."), "{raw}");

    // On the server: the copy in Sent, the conversation in Archive.
    wait_until("the copy in Sent", 30, || phone.find("Sent", &hdr).len() == 1);
    wait_until("the conversation in Archive", 30, || phone.find("Archive", "q1@test.nicemailer").len() == 1);
    assert!(phone.find("INBOX", "q1@test.nicemailer").is_empty());

    // The next pass attaches nothing twice.
    sync(&mut c);
    let rows: i64 = db(dir.path()).query_row("SELECT count(*) FROM message WHERE message_id_hdr = ?1 AND uid IS NOT NULL", [&hdr], |r| r.get(0)).unwrap();
    assert_eq!(rows, 1);
    let v = c.call("sync_verify", json!({"account": "dovecot"})).unwrap();
    assert_eq!(v["ok"], true, "{v:#}");
    // Writing to Carol accepted her; Alice was the sender.
    let carol: Option<String> = db(dir.path()).query_row("SELECT screening_target FROM address WHERE addr = 'carol@ok.example'", [], |r| r.get(0)).unwrap();
    assert_eq!(carol.as_deref(), Some("inbox"));
    assert_eq!(db(dir.path()).query_row("SELECT count(*) FROM draft", [], |r| r.get::<_, i64>(0)).unwrap(), 0);

    // `z`: the conversation goes back to the Inbox, on the server too.
    c.call("undo", json!({"token": sent["undo"]})).unwrap();
    wait_until("the conversation back in the Inbox", 30, || phone.find("INBOX", "q1@test.nicemailer").len() == 1);
}

#[test]
#[ignore]
fn a_reply_goes_out_and_files_its_conversation_full() {
    a_reply_goes_out_and_files_its_conversation(FULL);
}

#[test]
#[ignore]
fn a_reply_goes_out_and_files_its_conversation_minimal() {
    a_reply_goes_out_and_files_its_conversation(MINIMAL);
}

/// A reply to a mail of an archive account (an mbox import, no server)
/// leaves from the IMAP account that can send (`reponse.md`, section 4
/// ter): its address in From, through its SMTP server, copied to its Sent
/// folder on the server; the archive account keeps nothing of it but the
/// mail answered.
#[test]
#[ignore]
fn a_mail_of_an_archive_account_is_answered_from_the_imap_account() {
    let dir = tempfile::tempdir().unwrap();
    let user = new_user();
    let mut phone = Phone::open(FULL, &user);
    phone.create("Sent");
    let (_core, mut c) = core(dir.path(), FULL, &user, &starttls(), 1);
    let mbox = dir.path().join("old.mbox");
    std::fs::write(
        &mbox,
        "From a@x Tue Jun 23 19:20:56 2020\nFrom: Alice <alice@example.com>\nTo: old@hey.example\n\
         Subject: Ancien devis\nMessage-ID: <arch1@test.nicemailer>\n\nBonjour.\n",
    )
    .unwrap();
    c.call("import_mbox", json!({"path": mbox.to_str().unwrap(), "account": "hey", "address": "old@hey.example"})).unwrap();
    c.event("ImportFinished", 30);
    let m = message(dir.path(), "arch1");
    let d = c.call("create_draft", json!({"reply_to": m, "quote_header": "x"})).unwrap()["draft"].clone();
    assert_eq!(d["from"]["addr"], format!("{user}@example.com"));
    assert_ne!(d["send_account_id"], d["account_id"]);
    c.call("send_draft", json!({"id": d["id"], "revision": d["revision"], "body": "Toujours valable ?"})).unwrap();
    let sent = c.event("DraftSent", 60);
    let (hdr, account): (String, i64) = db(dir.path())
        .query_row("SELECT message_id_hdr, account_id FROM message WHERE id = ?1", [sent["message_id"].as_i64().unwrap()], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(account, d["send_account_id"].as_i64().unwrap());
    let got = received(SMTP_API, &hdr);
    assert_eq!(got.len(), 1, "{got:?}");
    assert!(got[0].contains(&format!("From: Test User <{user}@example.com>")), "{}", got[0]);
    assert!(got[0].contains("In-Reply-To: <arch1@test.nicemailer>"), "{}", got[0]);
    wait_until("the copy in Sent", 30, || phone.find("Sent", &hdr).len() == 1);
}

#[test]
#[ignore]
fn a_send_cancelled_in_time_never_leaves() {
    let dir = tempfile::tempdir().unwrap();
    let user = new_user();
    let mut phone = Phone::open(FULL, &user);
    phone.create("Sent");
    phone.deliver(&user, "c1", "");
    let (_core, mut c) = core(dir.path(), FULL, &user, &starttls(), 4);
    let m = message(dir.path(), "c1");
    let d = c.call("create_draft", json!({"reply_to": m, "quote_header": "x"})).unwrap()["draft"].clone();
    let hdr: String = db(dir.path()).query_row("SELECT message_id_hdr FROM draft WHERE id = ?1", [d["id"].as_i64().unwrap()], |r| r.get(0)).unwrap();
    c.call("send_draft", json!({"id": d["id"], "revision": d["revision"], "body": "Oups"})).unwrap();
    let back = c.call("cancel_send", json!({"id": d["id"]})).unwrap();
    assert_eq!(back["state"], "editing");
    assert_eq!(back["body"], "Oups");
    std::thread::sleep(Duration::from_secs(7));
    assert!(received(SMTP_API, &hdr).is_empty());
    assert!(phone.find("Sent", &hdr).is_empty());
    // Still there, editable, reopened by the next `r`.
    let again = c.call("create_draft", json!({"reply_to": m, "quote_header": "x"})).unwrap();
    assert_eq!(again["reused"], true);
    assert_eq!(again["draft"]["body"], "Oups");
}

#[test]
#[ignore]
fn a_refused_recipient_does_not_stop_the_others() {
    let dir = tempfile::tempdir().unwrap();
    let user = new_user();
    let mut phone = Phone::open(FULL, &user);
    phone.create("Sent");
    phone.deliver(&user, "p1", ", bad@refused.invalid.example");
    let (_core, mut c) = core(dir.path(), FULL, &user, &starttls(), 0);
    let m = message(dir.path(), "p1");
    let d = c.call("create_draft", json!({"reply_to": m, "quote_header": "x"})).unwrap()["draft"].clone();
    c.call("send_draft", json!({"id": d["id"], "revision": d["revision"], "body": "Bonjour"})).unwrap();
    // `DraftSent` first, then what was refused.
    let sent = c.event("DraftSent", 60);
    let partial = c.event("SendPartial", 10);
    assert_eq!(partial["draft_id"], d["id"]);
    let refused = partial["refused"].as_array().unwrap();
    assert_eq!(refused.len(), 1, "{partial}");
    assert!(refused[0].as_str().unwrap().starts_with("bad@refused.invalid.example (550"), "{partial}");
    let hdr: String = db(dir.path()).query_row("SELECT message_id_hdr FROM message WHERE id = ?1", [sent["message_id"].as_i64().unwrap()], |r| r.get(0)).unwrap();
    assert_eq!(received(SMTP_API, &hdr).len(), 1);
}

#[test]
#[ignore]
fn a_refused_login_gives_the_draft_back_until_the_smtp_password_is_saved() {
    let dir = tempfile::tempdir().unwrap();
    let user = new_user();
    let mut phone = Phone::open(FULL, &user);
    phone.create("Sent");
    phone.deliver(&user, "a1", "");
    // The SMTP server wants its own password: the IMAP one is refused.
    let smtps = format!("port = {SMTPS}\nsecurity = \"tls\"\nusername = \"nicemailer\"\n");
    let (_core, mut c) = core(dir.path(), FULL, &user, &smtps, 0);
    let m = message(dir.path(), "a1");
    let d = c.call("create_draft", json!({"reply_to": m, "quote_header": "x"})).unwrap()["draft"].clone();
    c.call("send_draft", json!({"id": d["id"], "revision": d["revision"], "body": "Bonjour"})).unwrap();
    let failed = c.event("SendFailed", 60);
    assert_eq!(failed["draft_id"], d["id"]);
    assert_eq!(failed["reply_to"], m);
    assert!(failed["message"].as_str().unwrap().contains("refused the login"), "{failed}");
    assert_eq!(failed["why"]["reason"], "smtp_refused", "{failed}");
    assert!(failed["why"]["server_text"].as_str().unwrap().contains("refused the login"), "{failed}");
    // Editable again, with its text.
    let again = c.call("create_draft", json!({"reply_to": m, "quote_header": "x"})).unwrap()["draft"].clone();
    assert_eq!((again["state"].as_str(), again["body"].as_str()), (Some("editing"), Some("Bonjour")));
    // The SMTP password saved apart: sent.
    c.call("set_password", json!({"account": "dovecot", "password": "smtp-secret", "smtp": true})).unwrap();
    c.call("send_draft", json!({"id": again["id"], "revision": again["revision"], "body": "Bonjour"})).unwrap();
    let sent = c.event("DraftSent", 60);
    let hdr: String = db(dir.path()).query_row("SELECT message_id_hdr FROM message WHERE id = ?1", [sent["message_id"].as_i64().unwrap()], |r| r.get(0)).unwrap();
    assert_eq!(received(SMTPS_API, &hdr).len(), 1);
}

#[test]
#[ignore]
fn an_agent_cannot_send() {
    let dir = tempfile::tempdir().unwrap();
    let user = new_user();
    let mut phone = Phone::open(FULL, &user);
    phone.deliver(&user, "g1", "");
    let (_core, _c) = core(dir.path(), FULL, &user, &starttls(), 0);
    let mut agent = Client::connect_as(dir.path(), "mcp");
    let m = message(dir.path(), "g1");
    // An agent writes drafts (`docs/mcp.md`, phase 2 of `mcp.md`): the user
    // reviews them and sends them from NiceMailer. Until 16 November 2026
    // it could not even create one, which this test checked.
    let d = agent.call("create_draft", json!({"reply_to": m, "quote_header": "x"})).unwrap()["draft"].clone();
    // But it cannot send: refused with its reason, the draft left as it is.
    let e = agent.call("send_draft", json!({"id": d["id"], "revision": d["revision"], "body": "Bonjour"})).unwrap_err();
    assert_eq!((e["code"].as_i64(), e["data"]["reason"].as_str()), (Some(1007), Some("agent_cannot_send")), "{e}");
    let state: String = db(dir.path()).query_row("SELECT state FROM draft WHERE id = ?1", [d["id"].as_i64().unwrap()], |r| r.get(0)).unwrap();
    assert_eq!(state, "editing", "not queued for sending");
    // And nothing reached the SMTP server.
    let hdr: String = db(dir.path()).query_row("SELECT message_id_hdr FROM draft WHERE id = ?1", [d["id"].as_i64().unwrap()], |r| r.get(0)).unwrap();
    std::thread::sleep(std::time::Duration::from_secs(2));
    assert!(received(SMTP_API, &hdr).is_empty());
}

/// Escape files the draft in Drafts, on the server too; a second version
/// replaces the first (one copy); a copy deleted on the phone takes the
/// draft; a draft sent leaves Drafts (`brouillons.md`, sections 4 to 6).
fn drafts_follow_the_server(port: u16) {
    let dir = tempfile::tempdir().unwrap();
    let user = new_user();
    let mut phone = Phone::open(port, &user);
    phone.create("Sent");
    phone.create("Drafts");
    phone.deliver(&user, "d1", "");
    phone.deliver(&user, "d2", "");
    let (_core, mut c) = core(dir.path(), port, &user, &starttls(), 1);
    let hdr_of = |dir: &Path, id: i64| -> String {
        db(dir).query_row("SELECT message_id_hdr FROM draft WHERE id = ?1", [id], |r| r.get(0)).unwrap()
    };

    // Escape: in Drafts, here and on the server.
    let m = message(dir.path(), "d1");
    let d = c.call("create_draft", json!({"reply_to": m, "quote_header": "x"})).unwrap()["draft"].clone();
    let id = d["id"].as_i64().unwrap();
    let r = c.call("update_draft", json!({"id": id, "revision": d["revision"], "body": "Première version"})).unwrap();
    c.call("close_draft", json!({"id": id})).unwrap();
    let hdr = hdr_of(dir.path(), id);
    wait_until("the copy in Drafts", 30, || phone.find("Drafts", &hdr).len() == 1);
    let first = phone.find("Drafts", &hdr)[0];
    wait_until("the UID recorded", 30, || {
        db(dir.path()).query_row("SELECT m.uid FROM draft d JOIN message m ON m.id = d.message_id WHERE d.id = ?1", [id], |r| r.get::<_, Option<u32>>(0)).unwrap() == Some(first)
    });
    // A second version: one copy, the new one.
    c.call("update_draft", json!({"id": id, "revision": r["revision"], "body": "Deuxième version"})).unwrap();
    c.call("close_draft", json!({"id": id})).unwrap();
    wait_until("one new copy", 30, || {
        let found = phone.find("Drafts", &hdr);
        found.len() == 1 && found[0] != first
    });
    sync(&mut c);
    let v = c.call("sync_verify", json!({"account": "dovecot"})).unwrap();
    assert_eq!(v["ok"], true, "{v:#}");
    let rows: i64 = db(dir.path()).query_row(
        "SELECT count(*) FROM message m JOIN folder f ON f.id = m.folder_id WHERE f.role = 'drafts'", [], |r| r.get(0)).unwrap();
    assert_eq!(rows, 1);
    // Deleted on the phone, not changed here since: the draft goes.
    let copy = phone.find("Drafts", &hdr)[0];
    phone.delete("Drafts", copy);
    sync(&mut c);
    wait_until("the draft gone", 30, || {
        db(dir.path()).query_row("SELECT count(*) FROM draft", [], |r| r.get::<_, i64>(0)).unwrap() == 0
    });

    // A draft in Drafts, then sent: it leaves Drafts on the server.
    let m2 = message(dir.path(), "d2");
    let d2 = c.call("create_draft", json!({"reply_to": m2, "quote_header": "x"})).unwrap()["draft"].clone();
    let id2 = d2["id"].as_i64().unwrap();
    c.call("close_draft", json!({"id": id2})).unwrap();
    let hdr2 = hdr_of(dir.path(), id2);
    wait_until("the copy in Drafts", 30, || phone.find("Drafts", &hdr2).len() == 1);
    // `r` on the row of Drafts reopens the same draft.
    let row: i64 = db(dir.path()).query_row("SELECT message_id FROM draft WHERE id = ?1", [id2], |r| r.get(0)).unwrap();
    let again = c.call("create_draft", json!({"reply_to": row, "quote_header": "x"})).unwrap();
    assert_eq!((again["reused"].as_bool(), again["draft"]["id"].as_i64()), (Some(true), Some(id2)));
    c.call("send_draft", json!({"id": id2, "revision": again["draft"]["revision"], "body": "Envoyé"})).unwrap();
    c.event("DraftSent", 60);
    wait_until("the copy gone from Drafts", 30, || phone.find("Drafts", &hdr2).is_empty());
    wait_until("the mail in Sent", 30, || phone.find("Sent", &hdr2).len() == 1);
}

#[test]
#[ignore]
fn drafts_follow_the_server_full() {
    drafts_follow_the_server(FULL);
}

#[test]
#[ignore]
fn drafts_follow_the_server_minimal() {
    drafts_follow_the_server(MINIMAL);
}
