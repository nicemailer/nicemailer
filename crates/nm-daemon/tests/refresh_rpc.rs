//! The reload of the conversations that changed, rather than of the whole
//! list, through the socket (`evenements.md`, `FolderChanged` with
//! `threads_announced`; `list_threads` with `filter.thread_ids`).

mod common;

use common::{Client, start};
use serde_json::{Value, json};

/// Three conversations from Alice, Bob and Carol, the first two unread
/// once imported (an mbox import marks its mails read: set below).
const MBOX: &str = "From a@x Tue Jun 23 19:20:56 2020
From: Alice <alice@a.example>
To: me@hey.example
Subject: Devis
Message-ID: <m1@a.example>

Bonjour

From b@x Wed Jun 24 09:00:00 2020
From: Bob <bob@b.example>
To: me@hey.example
Subject: Facture
Message-ID: <m2@b.example>

Voici

From c@x Thu Jun 25 09:00:00 2020
From: Carol <carol@c.example>
To: me@hey.example
Subject: Rendez-vous
Message-ID: <m3@c.example>

Demain ?
";

/// Imports the mbox and waits for its end; returns the client.
fn imported(dir: &std::path::Path) -> Client {
    let mbox = dir.join("test.mbox");
    std::fs::write(&mbox, MBOX).unwrap();
    let mut c = Client::connect(dir);
    c.call("subscribe", json!({})).unwrap();
    c.call("import_mbox", json!({"path": mbox.to_str().unwrap(), "account": "hey", "address": "me@hey.example"})).unwrap();
    c.event("ImportFinished", 30);
    c
}

/// The ids of a list's rows, newest first.
fn ids(page: &Value) -> Vec<i64> {
    page["items"].as_array().unwrap().iter().map(|t| t["id"].as_i64().unwrap()).collect()
}

/// Marking a conversation read announces its folders: every conversation
/// that changed has its `ThreadChanged` before the `FolderChanged` that
/// says so.
#[test]
fn a_flag_change_announces_its_conversations() {
    let dir = tempfile::tempdir().unwrap();
    let _core = start(dir.path());
    let mut c = imported(dir.path());
    let all = c.call("list_threads", json!({"everything": {}, "limit": 10})).unwrap();
    let rows = ids(&all);
    assert_eq!(rows.len(), 3);
    c.clear_events();
    c.call("set_thread_seen", json!({"thread_id": rows[0], "seen": false})).unwrap();
    // The events of that change, in order, until its last FolderChanged.
    let mut announced_threads = Vec::new();
    let mut folders = Vec::new();
    while folders.is_empty() || announced_threads.is_empty() {
        let v = c.next_event();
        match v["method"].as_str() {
            Some("ThreadChanged") => announced_threads.push(v["params"]["thread_id"].as_i64().unwrap()),
            Some("FolderChanged") => {
                assert!(!announced_threads.is_empty(), "a FolderChanged before its ThreadChanged");
                folders.push(v["params"]["threads_announced"].clone());
            }
            _ => {}
        }
    }
    assert_eq!(announced_threads, vec![rows[0]]);
    assert!(folders.iter().all(|a| *a == json!(true)), "{folders:?}");
}

/// `filter.thread_ids` gives those of the list's conversations, in its
/// order, with its other criteria: a conversation that no longer belongs
/// to the list is left out.
#[test]
fn given_conversations_are_listed_with_the_lists_criteria() {
    let dir = tempfile::tempdir().unwrap();
    let _core = start(dir.path());
    let mut c = imported(dir.path());
    let rows = ids(&c.call("list_threads", json!({"everything": {}, "limit": 10})).unwrap());
    // Newest first: Carol, Bob, Alice. Bob unread.
    c.call("set_thread_seen", json!({"thread_id": rows[1], "seen": false})).unwrap();
    let asked = json!([rows[2], rows[1], rows[0]]);
    // No other criterion: the three, in the order of the list.
    let page = c.call("list_threads", json!({"everything": {}, "limit": 3, "filter": {"thread_ids": asked}})).unwrap();
    assert_eq!(ids(&page), rows);
    assert!(page["next_cursor"].is_null());
    // Unread: Bob only; the two others left the list "unread".
    let page = c
        .call("list_threads", json!({"everything": {}, "limit": 3, "filter": {"thread_ids": asked, "seen": false}}))
        .unwrap();
    assert_eq!(ids(&page), vec![rows[1]]);
    // A conversation unknown or gone: simply not returned.
    let page = c.call("list_threads", json!({"everything": {}, "limit": 3, "filter": {"thread_ids": [999_999]}})).unwrap();
    assert!(ids(&page).is_empty());
}
