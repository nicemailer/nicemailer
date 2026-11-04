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
            Some("FolderChanged") => saw_folder = true,
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

    // Same account name with another address: refused before starting.
    let err = c
        .call("import_mbox", json!({"path": mbox.to_str().unwrap(), "account": "hey", "address": "other@x.example"}))
        .unwrap_err();
    assert_eq!(err["code"], -32602);
}
