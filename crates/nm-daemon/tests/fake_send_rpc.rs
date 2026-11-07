//! Pretended sending (`NICEMAILER_FAKE_NETWORK`, `src/drafts.rs`): on a
//! base of archive accounts imported from mbox, without any server in
//! `config.toml`, a reply can be written and "sent": recorded in Sent as
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
