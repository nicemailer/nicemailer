//! Integration tests of the store.
//!
//! Rust note: files in a crate's `tests/` directory are compiled as separate
//! programs that can only use the crate's *public* API, exactly like
//! another crate would. Unit tests that need private items live next to
//! the code instead, in `#[cfg(test)] mod tests` blocks.
//!
//! Each test builds its own in-memory database filled by the seed, so tests
//! are independent and can run in parallel.

use std::collections::HashSet;

use nm_core::{FolderRole, Source};
use nm_api::ThreadSort;
use nm_store::Store;
use nm_store::seed::{SeedOptions, seed};

/// A fresh in-memory store with 600 fake threads. The seed and the date are
/// fixed so that every run sees the same data.
fn seeded() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    seed(&mut store, &SeedOptions { threads: 600, seed: 42, now: 1_790_000_000 }).unwrap();
    store
}

/// Id of the folder with the given role.
fn folder_id(store: &Store, role: FolderRole) -> i64 {
    store.list_folders().unwrap().into_iter().find(|f| f.role == Some(role)).unwrap().id
}

/// Walking every page of a folder must return each thread exactly once,
/// in strictly decreasing (date, id) order. A small page size (17) forces
/// many pages and exercises the cursor.
#[test]
fn pagination_covers_every_thread_once_in_order() {
    let store = seeded();
    let archive = folder_id(&store, FolderRole::Archive);
    let expected = store.list_folders().unwrap().into_iter().find(|f| f.id == archive).unwrap().thread_count;
    let mut seen = HashSet::new();
    let mut cursor: Option<String> = None;
    let mut last = (i64::MAX, i64::MAX);
    loop {
        // `as_deref` turns `Option<String>` into `Option<&str>`.
        let (items, next) = store.list_threads(archive, cursor.as_deref(), 17, ThreadSort::Received).unwrap();
        for t in &items {
            assert!((t.date_last_received, t.id) < last, "order must be decreasing");
            last = (t.date_last_received, t.id);
            // `insert` returns false if the value was already in the set.
            assert!(seen.insert(t.id), "duplicate thread");
        }
        match next {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    assert_eq!(seen.len() as i64, expected);
}

/// Moving a thread from Inbox to Todo removes it from one list, adds it to
/// the other, and shifts the folder counters by its message count.
#[test]
fn move_thread_updates_lists_and_counters() {
    let mut store = seeded();
    let inbox = folder_id(&store, FolderRole::Inbox);
    let todo = folder_id(&store, FolderRole::Todo);
    let (items, _) = store.list_threads(inbox, None, 1, ThreadSort::Received).unwrap();
    let thread = &items[0];

    let before = store.list_folders().unwrap();
    let changes = store.move_thread(thread.id, FolderRole::Todo, Some(inbox), Source::App).unwrap();
    assert!(changes.changed_messages() > 0);

    let (inbox_items, _) = store.list_threads(inbox, None, 500, ThreadSort::Received).unwrap();
    assert!(!inbox_items.iter().any(|t| t.id == thread.id));
    let (todo_items, _) = store.list_threads(todo, None, 500, ThreadSort::Received).unwrap();
    assert!(todo_items.iter().any(|t| t.id == thread.id));

    let after = store.list_folders().unwrap();
    // Small helper closure: total message count of folder `id` in `list`.
    let count = |list: &[nm_api::FolderView], id| list.iter().find(|f| f.id == id).unwrap().total_count;
    assert_eq!(count(&before, inbox) - thread.message_count, count(&after, inbox));
    assert_eq!(count(&before, todo) + thread.message_count, count(&after, todo));

    // Sent never follows the thread: every message is now either in Todo
    // or still in Sent.
    let (_, messages) = store.get_thread(thread.id).unwrap();
    assert!(messages.iter().all(|m| matches!(m.folder_role, Some(FolderRole::Todo | FolderRole::Sent))));
}

/// Quarantine and Sent can never be chosen as targets, with or without a
/// source folder.
#[test]
fn quarantine_is_never_a_target() {
    let mut store = seeded();
    let inbox = folder_id(&store, FolderRole::Inbox);
    let (items, _) = store.list_threads(inbox, None, 1, ThreadSort::Received).unwrap();
    assert!(store.move_thread(items[0].id, FolderRole::Quarantine, Some(inbox), Source::App).is_err());
    assert!(store.move_thread(items[0].id, FolderRole::Sent, None, Source::App).is_err());
}

/// Deleted is outside `CONVERSATION_SCOPE`, so restoring only works because
/// the command is given from Deleted (`from_folder_id`).
#[test]
fn restore_from_deleted_uses_command_folder() {
    let mut store = seeded();
    let deleted = folder_id(&store, FolderRole::Deleted);
    let (items, _) = store.list_threads(deleted, None, 1, ThreadSort::Received).unwrap();
    store.move_thread(items[0].id, FolderRole::Inbox, Some(deleted), Source::App).unwrap();
    let (_, messages) = store.get_thread(items[0].id).unwrap();
    assert!(!messages.iter().any(|m| m.folder_role == Some(FolderRole::Deleted)));
}

/// Read/unread and flag commands are reflected in the thread list.
#[test]
fn seen_and_flag_commands() {
    let mut store = seeded();
    let inbox = folder_id(&store, FolderRole::Inbox);
    let (items, _) = store.list_threads(inbox, None, 200, ThreadSort::Received).unwrap();
    // `expect` is like `unwrap` with a message shown if it fails.
    let unread = items.iter().find(|t| t.unread_count > 0).expect("an unread thread");
    store.set_thread_seen(unread.id, true, Source::App).unwrap();
    let (items, _) = store.list_threads(inbox, None, 200, ThreadSort::Received).unwrap();
    assert_eq!(items.iter().find(|t| t.id == unread.id).unwrap().unread_count, 0);

    store.set_thread_flagged(unread.id, true, Source::App).unwrap();
    let (items, _) = store.list_threads(inbox, None, 200, ThreadSort::Received).unwrap();
    assert!(items.iter().find(|t| t.id == unread.id).unwrap().is_flagged);
    store.set_thread_flagged(unread.id, false, Source::App).unwrap();
    let (items, _) = store.list_threads(inbox, None, 200, ThreadSort::Received).unwrap();
    assert!(!items.iter().find(|t| t.id == unread.id).unwrap().is_flagged);
}

/// A thread that came back from Todo/Waiting shows the mark, preselects the
/// folder it came from on reply, and loses the mark after any command.
#[test]
fn returned_mark_is_shown_then_removed_by_a_command() {
    let mut store = seeded();
    let inbox = folder_id(&store, FolderRole::Inbox);
    let (items, _) = store.list_threads(inbox, None, 500, ThreadSort::Received).unwrap();
    let returned = items.iter().find(|t| t.returned_from.is_some()).expect("a returned thread");
    assert_eq!(returned.reply_target, returned.returned_from);
    // Archive it, then bring it back: the mark must be gone.
    store.move_thread(returned.id, FolderRole::Archive, Some(inbox), Source::App).unwrap();
    store.move_thread(returned.id, FolderRole::Inbox, None, Source::App).unwrap();
    let (items, _) = store.list_threads(inbox, None, 500, ThreadSort::Received).unwrap();
    assert_eq!(items.iter().find(|t| t.id == returned.id).unwrap().returned_from, None);
}

/// Search matches prefixes, ignores accents, and does not choke on FTS5
/// special characters (a lone quote, an apostrophe).
#[test]
fn search_ignores_accents_and_matches_prefixes() {
    let store = seeded();
    assert!(!store.search("factur", 10).unwrap().is_empty());
    assert!(!store.search("electricite", 10).unwrap().is_empty());
    assert!(store.search("\"", 10).unwrap().is_empty());
    assert!(!store.search("l'organisation", 10).unwrap().is_empty());
}
