//! Integration tests of unsubscribing (`ui.md`, "Désabonnement"): what the
//! reception finds and stores, what `get_thread` shows, the refusal from a
//! spam folder, and the record of a one-click unsubscribe.

use std::sync::Mutex;

use nm_core::{FolderRole, Source};
use nm_store::import::{ImportOptions, import_mbox};
use nm_store::raw::RawStore;
use nm_store::seed::{SeedOptions, seed};
use nm_store::{Store, StoreError};

/// One-click header; mailto-only header; a link of the HTML body; nothing.
const MBOX: &str = "From x Mon Nov  2 10:00:00 2026
From: Lettre <news@x.org>
To: me@x.org
Subject: Un clic
Message-ID: <1@x>
List-Unsubscribe: <mailto:leave@x.org?subject=unsubscribe>,
 <https://x.org/u?id=1>
List-Unsubscribe-Post: List-Unsubscribe=One-Click

Bonjour.

From x Mon Nov  2 11:00:00 2026
From: Liste <list@y.org>
To: me@x.org
Subject: Par mail
Message-ID: <2@x>
List-Unsubscribe: <mailto:leave@y.org>

Bonjour.

From x Mon Nov  2 12:00:00 2026
From: Boutique <shop@z.org>
To: me@x.org
Subject: Promo
Message-ID: <3@x>
Content-Type: text/html; charset=utf-8

<p>Promo</p><p><a href=\"https://z.org/optout?u=9\">Se d&eacute;sinscrire</a></p>

From x Mon Nov  2 13:00:00 2026
From: Alice <alice@w.org>
To: me@x.org
Subject: Bonjour
Message-ID: <4@x>

Rien a voir.
";

fn imported() -> Store {
    let dir = tempfile::tempdir().unwrap();
    let raw = RawStore::open(dir.path()).unwrap();
    let path = dir.path().join("a.mbox");
    std::fs::write(&path, MBOX).unwrap();
    let store = Mutex::new(Store::open_in_memory().unwrap());
    let opts = ImportOptions {
        path,
        account: "t".into(),
        address: "me@x.org".into(),
        mark_correspondents: false,
        source: Source::Cli,
        batch_size: 10,
        lock_budget: nm_store::import::LOCK_BUDGET,
    };
    import_mbox(&store, &raw, &opts, &mut |_, _, _| {}).unwrap();
    store.into_inner().unwrap()
}

/// The message `subject` and how `get_thread` shows its unsubscribe.
fn shown(store: &Store, subject: &str) -> (i64, Option<nm_api::UnsubscribeView>) {
    let thread: i64 =
        store.conn().query_row("SELECT thread_id FROM message WHERE subject = ?1", [subject], |r| r.get(0)).unwrap();
    let m = store.get_thread(thread).unwrap().1.remove(0);
    (m.id, m.unsubscribe)
}

#[test]
fn reception_finds_the_header_then_the_body() {
    let store = imported();
    let one = shown(&store, "Un clic").1.unwrap();
    assert_eq!((one.method.as_str(), one.source.as_str()), ("one_click", "header"));
    assert_eq!(one.url.as_deref(), Some("https://x.org/u?id=1"));
    assert_eq!(one.mailto.as_deref(), Some("mailto:leave@x.org?subject=unsubscribe"));
    assert_eq!(one.sender_unsubscribed_at, None);

    let mail = shown(&store, "Par mail").1.unwrap();
    assert_eq!((mail.method.as_str(), mail.source.as_str(), mail.url), ("mail", "header", None));

    let body = shown(&store, "Promo").1.unwrap();
    assert_eq!((body.method.as_str(), body.source.as_str()), ("browser", "body"));
    assert_eq!(body.url.as_deref(), Some("https://z.org/optout?u=9"));

    assert!(shown(&store, "Bonjour").1.is_none());
}

#[test]
fn a_one_click_unsubscribe_is_recorded_on_the_sender_and_in_the_log() {
    let mut store = imported();
    let (id, _) = shown(&store, "Un clic");
    let target = store.unsubscribe_target(id).unwrap();
    assert!(target.one_click && target.source == "header");
    let changes = store.mark_unsubscribed(id, Source::App).unwrap();
    assert_eq!(changes.threads.len(), 1);
    let after = shown(&store, "Un clic").1.unwrap();
    assert!(after.sender_unsubscribed_at.is_some());
    // The sender's decision is left alone.
    let decision: Option<String> = store
        .conn()
        .query_row("SELECT screening_target FROM address WHERE addr = 'news@x.org'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(decision, None);
    let (action, detail): (String, String) = store
        .conn()
        .query_row("SELECT action, detail FROM audit_log WHERE action = 'unsubscribe'", [], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    assert_eq!(action, "unsubscribe");
    assert!(detail.contains("https://x.org/u?id=1") && detail.contains("one_click"));
}

#[test]
fn no_unsubscribe_from_a_spam_folder_or_without_a_link() {
    let mut store = Store::open_in_memory().unwrap();
    seed(&mut store, &SeedOptions { threads: 200, seed: 5, now: 1_790_000_000 }).unwrap();
    let junk: i64 = store
        .conn()
        .query_row(
            "SELECT m.id FROM message m JOIN folder f ON f.id = m.folder_id WHERE f.role = ?1 LIMIT 1",
            [FolderRole::Junk.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    store
        .conn()
        .execute("INSERT INTO message_unsubscribe (message_id, url, source) VALUES (?1, 'https://spam.example/u', 'body')", [junk])
        .unwrap();
    assert!(matches!(store.unsubscribe_target(junk), Err(StoreError::InvalidParams(_))));
    // A message without any unsubscribe link.
    let other: i64 = store
        .conn()
        .query_row("SELECT id FROM message WHERE id NOT IN (SELECT message_id FROM message_unsubscribe) LIMIT 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(matches!(store.unsubscribe_target(other), Err(StoreError::NotFound(_))));
}
