//! `locate_message` through the socket: where a message is, for a deep
//! link `nicemailer://m/<id>` (`docs/deep-links.md`).

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
