//! `locate_message` through the socket: where a message is, for a deep
//! link `nicemailer://m/<key>`, or an old one `nicemailer://m/<id>`
//! (`docs/deep-links.md`).

mod common;

use common::{Client, start};
use nm_store::Store;
use nm_store::seed::{SeedOptions, seed};
use serde_json::json;

#[test]
fn a_message_is_located_by_its_id_and_a_missing_one_is_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let (message, thread, folder, account) = {
        let mut store = Store::open(&dir.path().join("nicemailer.db")).unwrap();
        seed(&mut store, &SeedOptions { threads: 5, seed: 1, now: 1_790_000_000 }).unwrap();
        store
            .conn()
            .query_row("SELECT id, thread_id, folder_id, account_id FROM message ORDER BY id DESC LIMIT 1", [], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?))
            })
            .unwrap()
    };
    let _core = start(dir.path());
    let mut c = Client::connect(dir.path());

    let r = c.call("locate_message", json!({"message_id": message})).unwrap();
    assert_eq!(r, json!({"message_id": message, "thread_id": thread, "folder_id": folder, "account_id": account}));

    // A link kept in a note after the mail was deleted, or after the
    // database was rebuilt: not found, with the core's error code.
    let err = c.call("locate_message", json!({"message_id": 999_999_999})).unwrap_err();
    assert_eq!(err["code"], 1001);
    // A malformed request.
    assert_eq!(c.call("locate_message", json!({"message_id": "x"})).unwrap_err()["code"], -32602);
}

#[test]
fn a_message_is_located_by_its_link_key_in_any_case() {
    let dir = tempfile::tempdir().unwrap();
    let (message, thread, key) = {
        let mut store = Store::open(&dir.path().join("nicemailer.db")).unwrap();
        seed(&mut store, &SeedOptions { threads: 5, seed: 1, now: 1_790_000_000 }).unwrap();
        store
            .conn()
            .query_row("SELECT id, thread_id, link_key FROM message ORDER BY id DESC LIMIT 1", [], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))
            })
            .unwrap()
    };
    let _core = start(dir.path());
    let mut c = Client::connect(dir.path());
    let text = nm_api::deep_link::key_text(key);

    let r = c.call("locate_message", json!({"link_key": text})).unwrap();
    assert_eq!((r["message_id"].as_i64(), r["thread_id"].as_i64()), (Some(message), Some(thread)));
    // A note app may change the case of the URL.
    let r = c.call("locate_message", json!({"link_key": text.to_uppercase()})).unwrap();
    assert_eq!(r["message_id"].as_i64(), Some(message));
    // The thread gives the app the same link to copy.
    let t = c.call("get_thread", json!({"thread_id": thread})).unwrap();
    let last = t["messages"].as_array().unwrap().iter().find(|m| m["id"] == message).unwrap();
    assert_eq!(last["link"], format!("nicemailer://m/{text}"));

    // A key no mail of this database has: not found.
    assert_eq!(c.call("locate_message", json!({"link_key": "000000000000"})).unwrap_err()["code"], 1001);
    // Malformed: not 12 hexadecimal characters, both or neither field.
    for bad in [
        json!({"link_key": "07bcf99aef4"}),
        json!({"link_key": "07bcf99aef4g"}),
        json!({"link_key": text, "message_id": message}),
        json!({}),
    ] {
        assert_eq!(c.call("locate_message", bad.clone()).unwrap_err()["code"], -32602, "{bad}");
    }
}
