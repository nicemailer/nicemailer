//! `message_record` through the socket: the record of a message the app
//! prints as a PDF (`sp`, `docs/core-library.md`).

mod common;

use common::{Client, start};
use nm_store::Store;
use nm_store::seed::{SeedOptions, seed};
use serde_json::json;

#[test]
fn the_record_of_a_seeded_message_comes_from_the_database() {
    let dir = tempfile::tempdir().unwrap();
    let (message, subject) = {
        let mut store = Store::open(&dir.path().join("nicemailer.db")).unwrap();
        seed(&mut store, &SeedOptions { threads: 5, seed: 1, now: 1_790_000_000 }).unwrap();
        store
            .conn()
            .query_row("SELECT id, subject FROM message ORDER BY id DESC LIMIT 1", [], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
            .unwrap()
    };
    let _core = start(dir.path());
    let mut c = Client::connect(dir.path());

    let r = c.call("message_record", json!({"message_id": message})).unwrap();
    assert_eq!(r["subject"], json!(subject));
    // Seeded mails have no raw file: no hash is claimed.
    assert_eq!(r["raw_sha256"], json!(null));
    assert!(r["from"].as_array().is_some_and(|f| !f.is_empty()));
    assert!(r["date_received"].as_i64().is_some());

    assert_eq!(c.call("message_record", json!({"message_id": 999_999_999})).unwrap_err()["code"], 1001);
    assert_eq!(c.call("message_record", json!({})).unwrap_err()["code"], -32602);
}
