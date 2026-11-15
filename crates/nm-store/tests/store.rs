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
use nm_store::screening::MoveOptions;
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
    let expected = store.folder_thread_count(archive).unwrap();
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
    let changes = store.move_thread(thread.id, FolderRole::Todo, Some(inbox), MoveOptions::default(), Source::App).unwrap();
    assert!(changes.changes.changed_messages() > 0);

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
    let (_, messages) = store.get_thread(thread.id, &[]).unwrap();
    assert!(messages.iter().all(|m| matches!(m.folder_role, Some(FolderRole::Todo | FolderRole::Sent))));
}

/// Quarantine and Sent can never be chosen as targets, with or without a
/// source folder.
#[test]
fn quarantine_is_never_a_target() {
    let mut store = seeded();
    let inbox = folder_id(&store, FolderRole::Inbox);
    let (items, _) = store.list_threads(inbox, None, 1, ThreadSort::Received).unwrap();
    assert!(store.move_thread(items[0].id, FolderRole::Quarantine, Some(inbox), MoveOptions::default(), Source::App).is_err());
    assert!(store.move_thread(items[0].id, FolderRole::Sent, None, MoveOptions::default(), Source::App).is_err());
}

/// Deleted is outside `CONVERSATION_SCOPE`, so restoring only works because
/// the command is given from Deleted (`from_folder_id`).
#[test]
fn restore_from_deleted_uses_command_folder() {
    let mut store = seeded();
    let deleted = folder_id(&store, FolderRole::Deleted);
    let (items, _) = store.list_threads(deleted, None, 1, ThreadSort::Received).unwrap();
    store.move_thread(items[0].id, FolderRole::Inbox, Some(deleted), MoveOptions::default(), Source::App).unwrap();
    let (_, messages) = store.get_thread(items[0].id, &[]).unwrap();
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
    store.move_thread(returned.id, FolderRole::Archive, Some(inbox), MoveOptions::default(), Source::App).unwrap();
    store.move_thread(returned.id, FolderRole::Inbox, None, MoveOptions::default(), Source::App).unwrap();
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

/// From Sent, a conversation is filed like anywhere else, but its copies
/// in Sent stay there (`dossiers.md`, "Cibles proposées"): it leaves the
/// Inbox for Todo and is still listed in Sent.
#[test]
fn move_from_sent_files_the_conversation_and_keeps_the_sent_copies() {
    let mut store = seeded();
    let sent = folder_id(&store, FolderRole::Sent);
    let todo = folder_id(&store, FolderRole::Todo);
    let (items, _) = store.list_threads(sent, None, 500, ThreadSort::Received).unwrap();
    // A conversation with mails in Sent and in the Inbox: an answered mail.
    let thread = items
        .iter()
        .find(|t| {
            let (_, m) = store.get_thread(t.id, &[]).unwrap();
            m.iter().any(|m| m.folder_role == Some(FolderRole::Inbox))
        })
        .expect("an answered conversation of the Inbox");
    let (_, before) = store.get_thread(thread.id, &[]).unwrap();
    let in_sent = before.iter().filter(|m| m.folder_role == Some(FolderRole::Sent)).count();
    assert!(in_sent > 0);

    let out = store.move_thread(thread.id, FolderRole::Todo, Some(sent), MoveOptions::default(), Source::App).unwrap();
    assert!(out.moved > 0);

    let (_, after) = store.get_thread(thread.id, &[]).unwrap();
    assert_eq!(after.iter().filter(|m| m.folder_role == Some(FolderRole::Sent)).count(), in_sent);
    assert!(!after.iter().any(|m| m.folder_role == Some(FolderRole::Inbox)));
    let (todo_items, _) = store.list_threads(todo, None, 500, ThreadSort::Received).unwrap();
    assert!(todo_items.iter().any(|t| t.id == thread.id));
    let (sent_items, _) = store.list_threads(sent, None, 500, ThreadSort::Received).unwrap();
    assert!(sent_items.iter().any(|t| t.id == thread.id));
}

/// A conversation with nothing but mails in Sent (a new mail, no answer
/// yet) would not move at all: its sent mails go to the target instead,
/// and it leaves the list of Sent. Undo brings them back to Sent.
#[test]
fn move_from_sent_takes_a_lone_sent_mail_along() {
    let mut store = seeded();
    let sent = folder_id(&store, FolderRole::Sent);
    let waiting = folder_id(&store, FolderRole::Waiting);
    let (items, _) = store.list_threads(sent, None, 500, ThreadSort::Received).unwrap();
    let thread = items
        .iter()
        .find(|t| {
            let (_, m) = store.get_thread(t.id, &[]).unwrap();
            m.iter().all(|m| m.folder_role == Some(FolderRole::Sent))
        })
        .expect("a conversation only in Sent");
    let (_, before) = store.get_thread(thread.id, &[]).unwrap();

    let out = store.move_thread(thread.id, FolderRole::Waiting, Some(sent), MoveOptions::default(), Source::App).unwrap();
    assert_eq!(out.moved, before.len());

    let (_, after) = store.get_thread(thread.id, &[]).unwrap();
    assert!(after.iter().all(|m| m.folder_role == Some(FolderRole::Waiting)));
    let (waiting_items, _) = store.list_threads(waiting, None, 500, ThreadSort::Received).unwrap();
    assert!(waiting_items.iter().any(|t| t.id == thread.id));
    let (sent_items, _) = store.list_threads(sent, None, 500, ThreadSort::Received).unwrap();
    assert!(!sent_items.iter().any(|t| t.id == thread.id));

    store.undo(out.undo.as_deref().expect("an undo token"), Source::App).unwrap();
    let (_, undone) = store.get_thread(thread.id, &[]).unwrap();
    assert!(undone.iter().all(|m| m.folder_role == Some(FolderRole::Sent)));
}

/// Sent is still never a target: only sending fills it.
#[test]
fn sent_is_never_a_target_from_sent_either() {
    let mut store = seeded();
    let sent = folder_id(&store, FolderRole::Sent);
    let (items, _) = store.list_threads(sent, None, 1, ThreadSort::Received).unwrap();
    assert!(store.move_thread(items[0].id, FolderRole::Sent, Some(sent), MoveOptions::default(), Source::App).is_err());
}
