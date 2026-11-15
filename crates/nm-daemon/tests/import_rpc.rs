//! End-to-end test of `import_mbox` through the socket.

mod common;

use common::{Client, start};
use serde_json::{Value, json};

const MBOX: &str = "From a@x Tue Jun 23 19:20:56 2020
From: Alice <alice@a.example>
To: me@hey.example
Subject: Devis
Message-ID: <m1@a.example>

Bonjour

From me@hey.example Wed Jun 24 09:00:00 2020
From: me@hey.example
To: alice@a.example
Subject: Re: Devis
Message-ID: <m2@hey.example>
In-Reply-To: <m1@a.example>

Merci
";

#[test]
fn import_mbox_runs_in_the_background_and_reports_events() {
    let dir = tempfile::tempdir().unwrap();
    let mbox = dir.path().join("test.mbox");
    std::fs::write(&mbox, MBOX).unwrap();
    let _core = start(dir.path());
    let mut c = Client::connect(dir.path());
    c.call("subscribe", json!({})).unwrap();

    // A relative path is refused: the core does not share the CLI's
    // working directory.
    let err = c.call("import_mbox", json!({"path": "test.mbox", "account": "hey", "address": "me@hey.example"})).unwrap_err();
    assert_eq!(err["code"], -32602);

    let started = c
        .call("import_mbox", json!({"path": mbox.to_str().unwrap(), "account": "hey", "address": "me@hey.example"}))
        .unwrap();
    let job = started["job_id"].as_u64().unwrap();
    assert_eq!(started["running"], true);

    // Follow the events until the end of this import.
    let mut saw_progress = false;
    let mut saw_folder = false;
    let finished = loop {
        let v = c.line();
        match v["method"].as_str() {
            Some("ImportProgress") => saw_progress = true,
            Some("FolderChanged") => {
                saw_folder = true;
                // An import names no conversation (no `ThreadChanged` per
                // mail): its folders are never announced, and a client
                // reloads their whole list.
                assert!(v["params"].get("threads_announced").is_none(), "{v}");
            }
            Some("ImportFinished") if v["params"]["job_id"].as_u64() == Some(job) => break v["params"].clone(),
            _ => {}
        }
    };
    assert!(saw_progress && saw_folder);
    assert_eq!(finished["running"], false);
    assert!(finished.get("error").is_none());
    assert_eq!((finished["imported"].as_u64(), finished["sent"].as_u64()), (Some(2), Some(1)));

    // The status is kept after the end, and the folders are visible.
    let status = c.call("import_status", Value::Null).unwrap();
    assert_eq!(status["job_id"].as_u64(), Some(job));
    let folders = c.call("list_folders", json!({})).unwrap();
    let archive = folders["items"].as_array().unwrap().iter().find(|f| f["role"] == "archive").unwrap();
    assert_eq!(archive["account_kind"], "archive");
    assert_eq!(archive["total_count"], 1);

    // The virtual folder Everything: the conversation once, shown in its
    // folder (Archive, not Sent where its latest mail, the reply, is) with
    // the counters of the whole thread, for every account or for this one
    // (`dossiers.md`, "Dossier d'une conversation").
    let account = archive["account_id"].clone();
    for scope in [json!({}), json!({"account_id": account})] {
        let page = c.call("list_threads", json!({"everything": scope, "limit": 10})).unwrap();
        let items = page["items"].as_array().unwrap();
        assert_eq!(items.len(), 1, "{scope}");
        assert_eq!(items[0]["folder_id"], archive["id"]);
        assert_eq!(items[0]["message_count"], 2);
    }
    // All Mail (`all_mail`): the same here, nothing being in Deleted, Junk
    // or Banned; once the archived mail is deleted, the conversation stays,
    // by the reply in Sent (its fallback folder), and Deleted is never
    // where it is shown.
    let sent = folders["items"].as_array().unwrap().iter().find(|f| f["role"] == "sent").unwrap()["id"].clone();
    for scope in [json!({"all_mail": true}), json!({"account_id": account, "all_mail": true})] {
        let page = c.call("list_threads", json!({"everything": scope})).unwrap();
        assert_eq!(page["items"].as_array().unwrap().len(), 1, "{scope}");
        assert_eq!(page["items"][0]["folder_id"], archive["id"]);
    }
    let thread = c.call("list_threads", json!({"folder_id": archive["id"]})).unwrap()["items"][0]["id"].clone();
    c.call("move_thread", json!({"thread_id": thread, "target": "deleted", "from_folder_id": archive["id"]})).unwrap();
    let page = c.call("list_threads", json!({"everything": {"all_mail": true}})).unwrap();
    assert_eq!(page["items"].as_array().unwrap().len(), 1);
    assert_eq!(page["items"][0]["folder_id"], sent);
    // From Alice: her mail is in Deleted now, out of All Mail only.
    let page = c.call("list_threads", json!({"everything": {"all_mail": true}, "filter": {"from": ["alice"]}})).unwrap();
    assert!(page["items"].as_array().unwrap().is_empty(), "{page}");
    let page = c.call("list_threads", json!({"everything": {}, "filter": {"from": ["alice"]}})).unwrap();
    assert_eq!(page["items"].as_array().unwrap().len(), 1);
    // The threads of a contact are a criterion of the list (the former
    // `list_threads_by` is gone).
    let contacts = c.call("list_contacts", json!({"query": "alice"})).unwrap();
    let alice = contacts["items"][0]["id"].clone();
    let page = c.call("list_threads", json!({"everything": {}, "filter": {"address_id": alice}})).unwrap();
    assert_eq!(page["items"].as_array().unwrap().len(), 1);
    let page = c.call("list_threads", json!({"everything": {}, "filter": {"tag": "absent"}})).unwrap();
    assert!(page["items"].as_array().unwrap().is_empty());
    let err = c.call("list_threads_by", json!({"address_id": alice})).unwrap_err();
    assert_eq!(err["code"], -32601);
    // Exactly one of folder_id, folder_ids and everything.
    let err = c.call("list_threads", json!({"everything": {}, "folder_id": archive["id"]})).unwrap_err();
    assert_eq!(err["code"], -32602);
    let err = c.call("list_threads", json!({})).unwrap_err();
    assert_eq!(err["code"], -32602);
    let err = c.call("list_threads", json!({"everything": {"account": 1}})).unwrap_err();
    assert_eq!(err["code"], -32602);

    // Same account name with another address: refused before starting.
    let err = c
        .call("import_mbox", json!({"path": mbox.to_str().unwrap(), "account": "hey", "address": "other@x.example"}))
        .unwrap_err();
    assert_eq!(err["code"], -32602);
}
