//! End-to-end test of the folder workflow through the socket: the result of
//! `move_thread` (decision, filing, undo token), the confirmation a suspect
//! sender requires, `set_address_screening`, `set_domain_screening` and
//! `undo` (`dossiers.md`, "Décision sur un expéditeur").

mod common;

use common::{Client, start};
use nm_store::Store;
use nm_store::rusqlite::params;
use nm_store::seed::{SeedOptions, seed};
use serde_json::json;

/// A database with the seed's empty account and three mails in
/// Quarantine: two from jean@pme.fr (one forged), one from news@shop.fr.
/// Returns the ids of the Quarantine folder and of the three threads.
fn prepare(dir: &std::path::Path) -> (i64, [i64; 3]) {
    let mut store = Store::open(&dir.join("nicemailer.db")).unwrap();
    seed(&mut store, &SeedOptions { threads: 0, seed: 1, now: 1_790_000_000 }).unwrap();
    let c = store.conn();
    let quarantine: i64 = c.query_row("SELECT id FROM folder WHERE role = 'quarantine'", [], |r| r.get(0)).unwrap();
    let mut threads = [0; 3];
    for (i, (from, verdict, suspect)) in
        [("jean@pme.fr", "pass", false), ("jean@pme.fr", "fail", true), ("news@shop.fr", "pass", false)].iter().enumerate()
    {
        c.execute("INSERT INTO address (addr, created_by, modified_by) VALUES (?1, 'imap', 'imap') ON CONFLICT DO NOTHING", [from])
            .unwrap();
        c.execute("INSERT INTO thread (account_id, subject) VALUES (1, 'sujet')", []).unwrap();
        threads[i] = c.last_insert_rowid();
        c.execute(
            "INSERT INTO message (account_id, folder_id, thread_id, subject, from_address_id, from_addr, date_received,
                                  auth_result, auth_suspect, created_by, modified_by)
             VALUES (1, ?1, ?2, 'sujet', (SELECT id FROM address WHERE addr = ?3), ?3, ?4, ?5, ?6, 'imap', 'imap')",
            params![quarantine, threads[i], from, 1_700_000_000 + i as i64, verdict, suspect],
        )
        .unwrap();
    }
    (quarantine, threads)
}

#[test]
fn workflow_commands_report_decisions_filing_and_undo() {
    let dir = tempfile::tempdir().unwrap();
    let (quarantine, [first, forged, _news]) = prepare(dir.path());
    let _core = start(dir.path());
    let mut c = Client::connect(dir.path());

    // The forged mail's sender needs a confirmation, with what to show.
    let err = c
        .call("move_thread", json!({"thread_id": forged, "target": "inbox", "from_folder_id": quarantine}))
        .unwrap_err();
    assert_eq!(err["code"], 1005);
    assert_eq!(err["data"], json!({"addr": "jean@pme.fr", "verdict": "fail"}));

    // Accepting the sender from the genuine mail: the forged one stays.
    let r = c.call("move_thread", json!({"thread_id": first, "target": "inbox", "from_folder_id": quarantine})).unwrap();
    assert_eq!((r["moved"].as_u64(), r["filed"].as_u64()), (Some(1), Some(0)));
    assert_eq!(r["decision"], json!({"kind": "address", "name": "jean@pme.fr", "before": null, "after": "inbox"}));
    let token = r["undo"].as_str().unwrap().to_string();

    // Confirmed, with a policy: accepted.
    let r = c
        .call(
            "move_thread",
            json!({"thread_id": forged, "target": "inbox", "from_folder_id": quarantine,
                   "confirm_suspect": true, "auth_policy": "not_required"}),
        )
        .unwrap();
    assert_eq!(r["moved"], 1);

    // A domain decision, then undoing the first move.
    let r = c.call("set_domain_screening", json!({"domain": "shop.fr", "target": "newsletter"})).unwrap();
    assert_eq!((r["filed"].as_u64(), r["decision"]["kind"].as_str()), (Some(1), Some("domain")));
    let r = c.call("undo", json!({"token": token})).unwrap();
    assert_eq!(r["moved"], 1);
    let contacts = c.call("list_contacts", json!({"query": "pme.fr"})).unwrap();
    assert_eq!(contacts["items"][0]["screening_target"], json!(null));
    assert_eq!(contacts["items"][0]["auth_policy"], "not_required");

    // Removing a decision; bad parameters.
    let r = c.call("set_address_screening", json!({"addr": "news@shop.fr", "target": "banned"})).unwrap();
    assert_eq!(r["decision"]["after"], "banned");
    let r = c.call("set_address_screening", json!({"addr": "news@shop.fr"})).unwrap();
    assert_eq!(r["decision"]["after"], json!(null));
    assert_eq!(c.call("undo", json!({"token": "x"})).unwrap_err()["code"], -32602);
    assert_eq!(c.call("set_address_screening", json!({"addr": "a@b.fr", "target": "todo"})).unwrap_err()["code"], -32602);
}
