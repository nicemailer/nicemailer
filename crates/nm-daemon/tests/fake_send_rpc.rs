//! Pretended sending (`NICEMAILER_FAKE_NETWORK`, `src/drafts.rs`): on a
//! base of archive accounts imported from mbox, without any server in
//! `accounts.toml`, a reply can be written and "sent": recorded in Sent as
//! after a real SMTP server, without any network. Without the variable, an
//! archive account cannot answer.

mod common;

use std::path::Path;

use common::{Client, start, start_with};
use serde_json::json;

const MBOX: &str = "From a@x Tue Jun 23 19:20:56 2020
From: Alice <alice@a.example>
To: me@hey.example
Cc: bob@b.example
Subject: Devis
Message-ID: <m1@a.example>

Bonjour, voici le devis.
";

/// Imports the mbox into the archive account `hey` and waits for the end.
fn import(dir: &Path, c: &mut Client) {
    let mbox = dir.join("test.mbox");
    std::fs::write(&mbox, MBOX).unwrap();
    c.call("subscribe", json!({})).unwrap();
    c.call("import_mbox", json!({"path": mbox.to_str().unwrap(), "account": "hey", "address": "me@hey.example"})).unwrap();
    c.event("ImportFinished", 30);
}

fn message(dir: &Path) -> i64 {
    let db = nm_store::rusqlite::Connection::open_with_flags(dir.join("nicemailer.db"), nm_store::rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    db.query_row("SELECT id FROM message WHERE message_id_hdr = 'm1@a.example'", [], |r| r.get(0)).unwrap()
}

#[test]
fn an_archive_account_cannot_answer_normally() {
    let dir = tempfile::tempdir().unwrap();
    let _core = start(dir.path());
    let mut c = Client::connect_as(dir.path(), "app");
    import(dir.path(), &mut c);
    let e = c.call("create_draft", json!({"reply_to": message(dir.path()), "quote_header": "x"})).unwrap_err();
    assert_eq!(e["code"], 1015, "{e}");
}

#[test]
fn with_the_fake_network_an_archive_account_pretends_to_send() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.toml"), "[compose]\nundo_send_seconds = 1\n").unwrap();
    let _core = start_with(dir.path(), &[("NICEMAILER_FAKE_NETWORK", "1")]);
    let mut c = Client::connect_as(dir.path(), "app");
    import(dir.path(), &mut c);
    let m = message(dir.path());
    let d = c.call("create_draft", json!({"reply_to": m, "quote_header": "On Jun 23, Alice wrote:"})).unwrap()["draft"].clone();
    // From the archive account's own address, to all.
    assert_eq!(d["from"]["addr"], "me@hey.example");
    assert_eq!(d["to"][0]["addr"], "alice@a.example");
    assert_eq!(d["cc"][0]["addr"], "bob@b.example");
    c.call("send_draft", json!({"id": d["id"], "revision": d["revision"], "body": "Merci !"})).unwrap();
    let sent = c.event("DraftSent", 15);
    assert_eq!(sent["draft_id"], d["id"]);
    // Recorded in Sent, in the conversation of the mail answered.
    let db = nm_store::rusqlite::Connection::open_with_flags(dir.path().join("nicemailer.db"), nm_store::rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let (role, same_thread): (String, bool) = db
        .query_row(
            "SELECT f.role, m.thread_id = (SELECT thread_id FROM message WHERE id = ?2)
               FROM message m JOIN folder f ON f.id = m.folder_id WHERE m.id = ?1",
            [sent["message_id"].as_i64().unwrap(), m],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(role, "sent");
    assert!(same_thread);
    assert_eq!(db.query_row("SELECT count(*) FROM draft", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
}

/// `send_draft` takes the filing chosen with `⇧⌘↩` (`reponse.md`, section
/// 8): one of the four choices, saved with the draft and given back by
/// `cancel_send`; anything else is refused before the draft is touched.
#[test]
fn the_filing_chosen_goes_through_the_socket() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.toml"), "[compose]\nundo_send_seconds = 30\n").unwrap();
    let _core = start_with(dir.path(), &[("NICEMAILER_FAKE_NETWORK", "1")]);
    let mut c = Client::connect_as(dir.path(), "app");
    import(dir.path(), &mut c);
    let d = c.call("create_draft", json!({"reply_to": message(dir.path()), "quote_header": "x"})).unwrap()["draft"].clone();
    assert!(d["after_send"].is_null());
    let e = c.call("send_draft", json!({"id": d["id"], "revision": d["revision"], "body": "Merci", "after_send": "inbox"})).unwrap_err();
    assert_eq!(e["code"], -32602, "{e}");
    c.call("send_draft", json!({"id": d["id"], "revision": d["revision"], "body": "Merci", "after_send": "stay"})).unwrap();
    let v = c.call("cancel_send", json!({"id": d["id"]})).unwrap();
    assert_eq!(v["after_send"], "stay");
    assert_eq!(v["state"], "editing");
}

/// The recipient lines through the socket (`reponse.md`, section 6): a
/// text read into addresses, suggestions, and the edited headers saved.
#[test]
fn recipients_are_parsed_suggested_and_saved() {
    let dir = tempfile::tempdir().unwrap();
    let _core = start_with(dir.path(), &[("NICEMAILER_FAKE_NETWORK", "1")]);
    let mut c = Client::connect_as(dir.path(), "app");
    import(dir.path(), &mut c);
    let r = c.call("parse_addresses", json!({"text": "Bob <BOB@b.example>, nope; carol@c.example"})).unwrap();
    assert_eq!(r["addresses"], json!([{"addr": "bob@b.example", "name": "Bob"}, {"addr": "carol@c.example", "name": null}]));
    assert_eq!(r["invalid"], json!(["nope"]));
    // Alice wrote: she is suggested; the archive account's own address never.
    let s = c.call("suggest_recipients", json!({"query": "al"})).unwrap();
    assert_eq!(s["items"][0]["addr"], "alice@a.example");
    let s = c.call("suggest_recipients", json!({"query": "me@"})).unwrap();
    assert_eq!(s["items"], json!([]));
    let d = c.call("create_draft", json!({"reply_to": message(dir.path()), "quote_header": "x"})).unwrap()["draft"].clone();
    let r = c
        .call(
            "update_draft",
            json!({"id": d["id"], "revision": d["revision"], "subject": "Autre objet",
                   "bcc": [{"addr": "boss@b.example", "name": "Boss"}]}),
        )
        .unwrap();
    assert_eq!(r["revision"], d["revision"].as_i64().unwrap() + 1);
    let v = c.call("open_draft", json!({"id": d["id"]})).unwrap();
    assert_eq!(v["subject"], "Autre objet");
    assert_eq!(v["bcc"][0]["addr"], "boss@b.example");
    // The body was left out: kept.
    assert_eq!(v["body"], d["body"]);
    let e = c.call("update_draft", json!({"id": d["id"], "revision": r["revision"], "to": [{"addr": "nope", "name": null}]})).unwrap_err();
    assert_eq!(e["code"], -32602, "{e}");
}

/// Attachments through the socket (`brouillons.md`, section 7): the app
/// stages a file in `incoming.noindex`, the core takes it; the mail sent
/// carries it.
#[test]
fn a_staged_file_is_attached_and_sent() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.toml"), "[compose]\nundo_send_seconds = 1\n").unwrap();
    let _core = start_with(dir.path(), &[("NICEMAILER_FAKE_NETWORK", "1")]);
    let mut c = Client::connect_as(dir.path(), "app");
    import(dir.path(), &mut c);
    let d = c.call("create_draft", json!({"reply_to": message(dir.path()), "quote_header": "x"})).unwrap()["draft"].clone();
    std::fs::write(dir.path().join("incoming.noindex").join("A1B2"), b"%PDF devis").unwrap();
    let r = c
        .call("add_draft_attachment", json!({"id": d["id"], "revision": d["revision"], "staged": "A1B2", "filename": "devis.pdf", "mime_type": "application/pdf"}))
        .unwrap();
    assert_eq!(r["attachment"]["filename"], "devis.pdf");
    assert_eq!(r["attachment"]["size"], 10);
    assert!(r["warning"].is_null());
    // A path instead of a staged name: refused.
    let e = c
        .call("add_draft_attachment", json!({"id": d["id"], "revision": r["revision"], "staged": "../nicemailer.db", "filename": "x"}))
        .unwrap_err();
    assert_eq!(e["code"], -32602, "{e}");
    let v = c.call("open_draft", json!({"id": d["id"]})).unwrap();
    assert_eq!(v["attachments"][0]["mime_type"], "application/pdf");
    c.call("send_draft", json!({"id": d["id"], "revision": r["revision"], "body": "Ci-joint"})).unwrap();
    let sent = c.event("DraftSent", 15);
    let db = nm_store::rusqlite::Connection::open_with_flags(dir.path().join("nicemailer.db"), nm_store::rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let has: i64 = db.query_row("SELECT has_attachments FROM message WHERE id = ?1", [sent["message_id"].as_i64().unwrap()], |r| r.get(0)).unwrap();
    assert_eq!(has, 1);
}

/// `R`, `f` and `c` through the socket: the kind of a draft, a new mail
/// from the default account, and a new mail left empty dropped at closing.
#[test]
fn the_kinds_of_draft_go_through_the_socket() {
    let dir = tempfile::tempdir().unwrap();
    let _core = start_with(dir.path(), &[("NICEMAILER_FAKE_NETWORK", "1")]);
    let mut c = Client::connect_as(dir.path(), "app");
    import(dir.path(), &mut c);
    let m = message(dir.path());
    let r = c.call("create_draft", json!({"kind": "reply_sender", "reply_to": m, "quote_header": "x"})).unwrap()["draft"].clone();
    assert_eq!(r["to"].as_array().unwrap().len(), 1);
    assert_eq!(r["cc"], json!([]));
    let f = c.call("create_draft", json!({"kind": "forward", "reply_to": m, "quote_header": "--- Forwarded ---"})).unwrap();
    assert_eq!(f["draft"]["kind"], "forward");
    assert_eq!(f["draft"]["subject"], "Fwd: Devis");
    let e = c.call("create_draft", json!({"kind": "forward"})).unwrap_err();
    assert_eq!(e["code"], -32602, "{e}");
    let n = c.call("create_draft", json!({"kind": "new"})).unwrap()["draft"].clone();
    assert_eq!((n["kind"].as_str(), n["from"]["addr"].as_str()), (Some("new"), Some("me@hey.example")));
    c.call("close_draft", json!({"id": n["id"]})).unwrap();
    let e = c.call("open_draft", json!({"id": n["id"]})).unwrap_err();
    assert_eq!(e["code"], 1001, "{e}");
}
