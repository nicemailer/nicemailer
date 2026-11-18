//! What the core lets an AI agent do, on the real core (`mcp.md`,
//! sections 4, 8 and 12): the client `mcp` reads, writes its own drafts,
//! and is refused everything else; `attachment_text` reads every kind of
//! file the core knows, one page at a time; `list_history` filters by
//! source and date.

mod common;

use common::fixtures::{self, attachments, draft_sources, message};
use common::{Client, start_with};
use serde_json::{Value, json};

const FORBIDDEN: i64 = 1007;

/// A core with the fake network (an archive account can then write
/// drafts, as in the UI tests), the test mails imported by the app.
fn prepared(dir: &std::path::Path) -> common::Core {
    let core = start_with(dir, &[("NICEMAILER_FAKE_NETWORK", "1")]);
    let mut app = Client::connect_as(dir, "app");
    fixtures::import(dir, &mut app, "hey");
    core
}

fn code(e: &Value) -> i64 {
    e["code"].as_i64().unwrap()
}

#[test]
fn an_agent_reads_and_is_refused_everything_else() {
    let dir = tempfile::tempdir().unwrap();
    let _core = prepared(dir.path());
    let (_, thread) = message(dir.path(), "hey", "m1@a.example");
    let mut agent = Client::connect_as(dir.path(), "mcp");
    // Reading.
    assert!(agent.call("list_folders", json!({})).is_ok());
    assert!(agent.call("get_thread", json!({"thread_id": thread})).is_ok());
    assert!(agent.call("search", json!({"query": "devis"})).is_ok());
    assert!(agent.call("list_history", json!({})).is_ok());
    // Everything that leaves the mailbox, files mail, or touches the
    // accounts and the settings.
    let refused = [
        ("set_password", json!({"account": "hey", "password": "x"})),
        ("send_draft", json!({"id": 1, "revision": 1, "body": "x"})),
        ("move_thread", json!({"thread_id": thread, "target": "deleted"})),
        ("move_threads", json!({"threads": [{"thread_id": thread}], "target": "banned"})),
        ("set_thread_seen", json!({"thread_id": thread, "seen": true})),
        ("set_thread_flagged", json!({"thread_id": thread, "flagged": true})),
        ("tag_thread", json!({"thread_id": thread, "name": "x"})),
        ("set_address_screening", json!({"addr": "alice@a.example", "target": "inbox"})),
        ("set_domain_screening", json!({"domain": "a.example", "target": "banned"})),
        ("unsubscribe", json!({"message_id": 1})),
        ("import_mbox", json!({"path": "/tmp/x.mbox", "account": "x", "address": "x@x.example"})),
        ("remove_account", json!({"account": "hey"})),
        ("sync_now", json!({})),
        ("raw_gc", json!({})),
        ("ai_run", json!({"prompt": "summarize-thread", "thread_id": thread, "lang": "en"})),
        ("export_message", json!({"message_id": 1, "format": "eml", "dir": "/tmp"})),
        ("prepare_draft", json!({"reply_to": 1})),
    ];
    for (m, params) in refused {
        let e = agent.call(m, params).unwrap_err();
        assert_eq!(code(&e), FORBIDDEN, "{m}: {e}");
    }
    // A refusal says what to do instead.
    let e = agent.call("send_draft", json!({"id": 1, "revision": 1, "body": "x"})).unwrap_err();
    assert!(e["message"].as_str().unwrap().contains("Drafts"), "{e}");
    // Nothing of it ran: the thread is not flagged, still in Archive.
    let t = agent.call("get_thread", json!({"thread_id": thread})).unwrap();
    assert_eq!((t["messages"][0]["is_flagged"].clone(), t["messages"][0]["folder_role"].clone()), (json!(false), json!("archive")));
    // The app, on the same core, still can.
    let mut app = Client::connect_as(dir.path(), "app");
    assert!(app.call("set_thread_flagged", json!({"thread_id": thread, "flagged": true})).is_ok());
}

#[test]
fn an_agent_writes_its_own_drafts_and_only_reads_the_users() {
    let dir = tempfile::tempdir().unwrap();
    let _core = prepared(dir.path());
    let (first, _) = message(dir.path(), "hey", "m1@a.example");
    let (second, _) = message(dir.path(), "hey", "m2@a.example");
    let mut app = Client::connect_as(dir.path(), "app");
    let mut agent = Client::connect_as(dir.path(), "mcp");

    // The user's reply to the first mail.
    let user = app.call("create_draft", json!({"reply_to": first, "quote_header": "Alice a écrit :"})).unwrap()["draft"].clone();
    let (id, revision) = (user["id"].clone(), user["revision"].clone());
    // Readable by the agent...
    let read = agent.call("open_draft", json!({"id": id})).unwrap();
    assert_eq!(read["id"], id);
    // ...never changed by it.
    for (m, params) in [
        ("update_draft", json!({"id": id, "revision": revision, "body": "pwned"})),
        ("add_draft_attachment", json!({"id": id, "revision": revision, "staged": "x", "filename": "x"})),
        ("remove_draft_attachment", json!({"id": id, "revision": revision, "attachment_id": 1})),
        ("close_draft", json!({"id": id})),
        ("discard_draft", json!({"id": id})),
    ] {
        let e = agent.call(m, params).unwrap_err();
        assert_eq!(code(&e), FORBIDDEN, "{m}: {e}");
        assert!(e["message"].as_str().unwrap().contains("written by the user"), "{e}");
    }
    // Asking for a reply to the same mail gives back the user's: refused.
    let e = agent.call("create_draft", json!({"reply_to": first, "quote_header": "x"})).unwrap_err();
    assert_eq!(code(&e), FORBIDDEN, "{e}");
    assert!(e["message"].as_str().unwrap().contains("already started a reply"), "{e}");
    assert_eq!(app.call("open_draft", json!({"id": id})).unwrap()["body"], user["body"]);

    // Its own reply, to the second mail: written, changed, closed.
    let mine = agent.call("create_draft", json!({"reply_to": second, "quote_header": "Alice wrote:"})).unwrap()["draft"].clone();
    let mine_id = mine["id"].as_i64().unwrap();
    assert_eq!(draft_sources(dir.path(), mine_id), ("mcp".into(), "mcp".into()));
    let body = format!("Oui, jeudi.\n\n{}", mine["body"].as_str().unwrap());
    let r = agent.call("update_draft", json!({"id": mine_id, "revision": mine["revision"], "body": body})).unwrap();
    assert!(agent.call("close_draft", json!({"id": mine_id})).is_ok());
    // Asked again, the same draft comes back, and it is the agent's.
    let again = agent.call("create_draft", json!({"reply_to": second, "quote_header": "x"})).unwrap();
    assert_eq!((again["draft"]["id"].as_i64(), again["reused"].clone()), (Some(mine_id), json!(true)));
    // A stale revision fails rather than overwrite.
    let e = agent.call("update_draft", json!({"id": mine_id, "revision": mine["revision"], "body": "old"})).unwrap_err();
    assert_eq!(code(&e), 1012, "{e}");
    // The user may change what the agent wrote (it is their mailbox).
    let rev = r["revision"].clone();
    assert!(app.call("update_draft", json!({"id": mine_id, "revision": rev, "body": "Oui, jeudi 10 h."})).is_ok());
    assert_eq!(draft_sources(dir.path(), mine_id), ("mcp".into(), "app".into()));
    // Sending stays the user's.
    let d = agent.call("open_draft", json!({"id": mine_id})).unwrap();
    let e = agent.call("send_draft", json!({"id": mine_id, "revision": d["revision"], "body": d["body"]})).unwrap_err();
    assert_eq!(code(&e), FORBIDDEN, "{e}");

    // A new mail of its own, then discarded.
    let new = agent.call("create_draft", json!({"kind": "new", "body": "Bonjour"})).unwrap()["draft"].clone();
    assert_eq!(draft_sources(dir.path(), new["id"].as_i64().unwrap()).0, "mcp");
    assert!(agent.call("discard_draft", json!({"id": new["id"]})).is_ok());
    assert_eq!(code(&agent.call("open_draft", json!({"id": new["id"]})).unwrap_err()), 1001);
}

#[test]
fn the_text_of_every_kind_of_attachment_is_read_in_pages() {
    let dir = tempfile::tempdir().unwrap();
    let _core = prepared(dir.path());
    let (_, thread) = message(dir.path(), "hey", "f1@b.example");
    let mut agent = Client::connect_as(dir.path(), "mcp");
    let files = attachments(&mut agent, thread);
    let read = |c: &mut Client, name: &str, offset: u64, limit: u64| {
        c.call("attachment_text", json!({"attachment_id": files[name]["id"], "offset": offset, "limit": limit})).unwrap()
    };
    let pdf = read(&mut agent, "facture.pdf", 0, 1_000);
    assert_eq!((pdf["kind"].clone(), pdf["text"].clone()), (json!("pdf"), json!("Facture 2026-114 : 1 200 EUR")));
    assert!(pdf["path"].as_str().unwrap().contains("attachments.noindex"), "{pdf}");
    assert!(pdf.get("next_offset").is_none() && pdf.get("cut").is_none(), "{pdf}");
    let docx = read(&mut agent, "devis.docx", 0, 1_000);
    assert_eq!((docx["kind"].clone(), docx["text"].clone()), (json!("docx"), json!("Devis 42 : peinture du salon")));
    // HTML: the text, not the markup, nor the script.
    let html = read(&mut agent, "page.html", 0, 1_000);
    assert_eq!(html["kind"], "html");
    let text = html["text"].as_str().unwrap();
    assert!(text.contains("Bonjour Alice") && !text.contains('<') && !text.contains("alert"), "{text}");
    let eml = read(&mut agent, "planning.eml", 0, 1_000);
    assert_eq!(eml["kind"], "mail");
    assert!(eml["text"].as_str().unwrap().contains("Réunion lundi à 10 h."), "{eml}");
    // An image: no text, said, with the path to open it.
    let png = read(&mut agent, "logo.png", 0, 1_000);
    assert!(png.get("text").is_none() && png.get("kind").is_none(), "{png}");
    assert!(png["unreadable"].as_str().unwrap().contains("no text"), "{png}");
    assert!(png["path"].as_str().unwrap().ends_with("logo.png"), "{png}");
    // A long text, in pages of 7,000 characters: every character once.
    let long = fixtures::long_text();
    let (mut joined, mut offset, mut pages) = (String::new(), 0, 0);
    loop {
        let p = read(&mut agent, "long.txt", offset, 7_000);
        assert_eq!(p["offset"], offset);
        joined.push_str(p["text"].as_str().unwrap());
        pages += 1;
        match p["next_offset"].as_u64() {
            Some(n) => offset = n,
            None => {
                assert_eq!(p["total_chars"].as_u64(), Some(long.trim_end().chars().count() as u64));
                break;
            }
        }
    }
    assert_eq!(joined, long.trim_end());
    assert_eq!(pages, 6);
    // An unknown attachment.
    assert_eq!(code(&agent.call("attachment_text", json!({"attachment_id": 999_999})).unwrap_err()), 1001);
}

#[test]
fn the_history_is_filtered_by_source_and_date() {
    let dir = tempfile::tempdir().unwrap();
    let _core = prepared(dir.path());
    let (_, thread) = message(dir.path(), "hey", "m1@a.example");
    let mut app = Client::connect_as(dir.path(), "app");
    app.call("move_thread", json!({"thread_id": thread, "target": "todo"})).unwrap();
    let all = app.call("list_history", json!({})).unwrap();
    assert!(all["items"].as_array().unwrap().iter().any(|e| e["source"] == "app" && e["action"] == "move"), "{all}");
    // The agent did nothing that is logged.
    let agent = app.call("list_history", json!({"sources": ["mcp"]})).unwrap();
    assert_eq!(agent["items"], json!([]));
    let mine = app.call("list_history", json!({"sources": ["app"]})).unwrap();
    assert!(mine["items"].as_array().unwrap().iter().all(|e| e["source"] == "app"));
    // Since: in the future, nothing.
    let later = app.call("list_history", json!({"since": 4_000_000_000_i64})).unwrap();
    assert_eq!(later["items"], json!([]));
    assert_eq!(code(&app.call("list_history", json!({"sources": ["robot"]})).unwrap_err()), -32602);
}

/// The History screen reads the log in groups, with the words it needs; an
/// AI agent keeps its own method, and is refused this one.
#[test]
fn the_history_screen_reads_the_log_in_groups() {
    let dir = tempfile::tempdir().unwrap();
    let _core = prepared(dir.path());
    let (_, thread) = message(dir.path(), "hey", "m1@a.example");
    let mut app = Client::connect_as(dir.path(), "app");
    app.call("move_thread", json!({"thread_id": thread, "target": "todo"})).unwrap();
    let page = app.call("list_history_groups", json!({"limit": 10})).unwrap();
    let group = page["items"].as_array().unwrap().iter().find(|g| g["kind"] == "move").cloned().expect("the move");
    // The conversation holds two mails: one action, one group.
    assert_eq!((group["source"].as_str(), group["value"].as_str(), group["count"].as_i64()), (Some("app"), Some("todo"), Some(2)));
    // Its detail: each mail on its own.
    let detail = app
        .call("list_history_groups", json!({"first_id": group["first_id"], "last_id": group["last_id"]}))
        .unwrap();
    assert_eq!(detail["items"].as_array().unwrap().len(), 2);
    let mut agent = Client::connect_as(dir.path(), "mcp");
    assert_eq!(code(&agent.call("list_history_groups", json!({})).unwrap_err()), 1007);
}
