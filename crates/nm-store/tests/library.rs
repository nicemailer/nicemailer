//! Integration tests of `library.rs`: sort by modification, display
//! preference, history, export, tags, groups, contacts and thread lists
//! across folders.

use std::sync::Mutex;

use nm_api::{ExportFormat, ThreadSort};
use nm_core::{FolderRole, Source};
use nm_store::import::{ImportOptions, import_mbox};
use nm_store::raw::RawStore;
use nm_store::seed::{SeedOptions, seed};
use nm_store::{Store, StoreError};

fn seeded() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    seed(&mut store, &SeedOptions { threads: 300, seed: 7, now: 1_790_000_000 }).unwrap();
    store
}

fn folder_id(store: &Store, role: FolderRole) -> i64 {
    store.list_folders().unwrap().into_iter().find(|f| f.role == Some(role)).unwrap().id
}

/// A thread of the Inbox and its folder.
fn inbox_thread(store: &Store) -> (i64, i64) {
    let inbox = folder_id(store, FolderRole::Inbox);
    let (items, _) = store.list_threads(inbox, None, 1, ThreadSort::Received).unwrap();
    (items[0].id, inbox)
}

#[test]
fn modified_sort_puts_the_last_handled_thread_first() {
    let mut store = seeded();
    let archive = folder_id(&store, FolderRole::Archive);
    // The oldest thread of Archive, by reception.
    let (all, _) = store.list_threads(archive, None, 200, ThreadSort::Received).unwrap();
    let oldest = all.last().unwrap().id;
    // Flagging it modifies it now (unixepoch(), later than every seed date).
    store.set_thread_flagged(oldest, true, Source::App).unwrap();
    let (by_modified, _) = store.list_threads(archive, None, 5, ThreadSort::Modified).unwrap();
    assert_eq!(by_modified[0].id, oldest);
    let (by_received, _) = store.list_threads(archive, None, 5, ThreadSort::Received).unwrap();
    assert_ne!(by_received[0].id, oldest);
}

#[test]
fn modified_sort_paginates_every_thread_once() {
    let store = seeded();
    let archive = folder_id(&store, FolderRole::Archive);
    let mut seen = std::collections::HashSet::new();
    let mut cursor: Option<String> = None;
    loop {
        let (items, next) = store.list_threads(archive, cursor.as_deref(), 13, ThreadSort::Modified).unwrap();
        for t in &items {
            assert!(seen.insert(t.id), "thread {} listed twice", t.id);
        }
        match next {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    let expected = store.list_folders().unwrap().into_iter().find(|f| f.id == archive).unwrap().thread_count;
    assert_eq!(seen.len() as i64, expected);
}

#[test]
fn body_format_is_remembered_per_sender() {
    let mut store = seeded();
    let (thread, _) = inbox_thread(&store);
    let (_, messages) = store.get_thread(thread).unwrap();
    let sender = messages.iter().find_map(|m| m.from_addr.clone()).unwrap();
    assert!(messages.iter().all(|m| m.from_body_format.is_none()));
    store.set_body_format(&sender, Some("html")).unwrap();
    let (_, messages) = store.get_thread(thread).unwrap();
    let m = messages.iter().find(|m| m.from_addr.as_deref() == Some(sender.as_str())).unwrap();
    assert_eq!(m.from_body_format.as_deref(), Some("html"));
    store.set_body_format(&sender, None).unwrap();
    assert!(matches!(store.set_body_format(&sender, Some("pdf")), Err(StoreError::InvalidParams(_))));
    assert!(matches!(store.set_body_format("nobody@x.example", Some("text")), Err(StoreError::NotFound(_))));
}

#[test]
fn history_lists_actions_newest_first_with_a_summary() {
    let mut store = seeded();
    let (thread, inbox) = inbox_thread(&store);
    let subject = store.get_thread(thread).unwrap().0;
    store.move_thread(thread, FolderRole::Todo, Some(inbox), Source::App).unwrap();
    let (items, _) = store.list_history(None, 10).unwrap();
    let first = &items[0];
    assert_eq!((first.entity.as_str(), first.action.as_str(), first.source.as_str()), ("message", "move", "app"));
    assert!(first.summary.ends_with("→ todo"), "{}", first.summary);
    // The summary names the message (its subject may add "Re: ").
    assert!(items.iter().any(|e| e.summary.contains(&subject)));
    // Pagination: every line once.
    let total: i64 = store.conn().query_row("SELECT count(*) FROM audit_log", [], |r| r.get(0)).unwrap();
    let mut n = 0;
    let mut cursor: Option<String> = None;
    loop {
        let (items, next) = store.list_history(cursor.as_deref(), 2).unwrap();
        n += items.len() as i64;
        match next {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    assert_eq!(n, total);
}

#[test]
fn tags_are_created_listed_shown_and_removed() {
    let mut store = seeded();
    let (thread, inbox) = inbox_thread(&store);
    let changes = store.tag_thread(thread, "Équipe Paris", false, Source::App).unwrap();
    assert!(changes.changed_messages() > 0);
    // Same tag again, other case: nothing changes, no second tag.
    assert_eq!(store.tag_thread(thread, "équipe paris", false, Source::App).unwrap().changed_messages(), 0);
    let tags = store.list_tags(None).unwrap();
    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].name, "Équipe Paris");
    assert_eq!(tags[0].message_count as usize, changes.changed_messages());
    let keyword: String = store.conn().query_row("SELECT keyword FROM tag", [], |r| r.get(0)).unwrap();
    assert_eq!(keyword, "Equipe_Paris");
    let (items, _) = store.list_threads(inbox, None, 200, ThreadSort::Received).unwrap();
    assert_eq!(items.iter().find(|t| t.id == thread).unwrap().tags, vec!["Équipe Paris"]);
    // Threads of the tag.
    let by_tag = store.list_threads_by(Some(tags[0].id), None, None, 50).unwrap();
    assert_eq!(by_tag.iter().map(|t| t.id).collect::<Vec<_>>(), vec![thread]);
    // Removal.
    store.tag_thread(thread, "Équipe Paris", true, Source::App).unwrap();
    assert_eq!(store.list_tags(None).unwrap()[0].message_count, 0);
    // Two names mapping to the same keyword get distinct keywords.
    store.tag_thread(thread, "Café", false, Source::App).unwrap();
    store.tag_thread(thread, "Cafe", false, Source::App).unwrap();
    let kws: Vec<String> = store
        .conn()
        .prepare("SELECT keyword FROM tag WHERE name IN ('Café', 'Cafe') ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(kws, vec!["Cafe", "Cafe_2"]);
}

#[test]
fn group_membership_follows_the_rules_of_groupes_md() {
    let mut store = seeded();
    let (thread, _) = inbox_thread(&store);
    // The sender of the thread (not the user, whose mails are in Sent).
    let (gid, addr, changed) = store.set_group_member("VIP", None, Some(thread), false, Source::App).unwrap();
    assert!(changed && addr.contains('@'));
    let (_, msgs) = store.get_thread(thread).unwrap();
    assert!(msgs.iter().any(|m| m.from_addr.as_deref() == Some(addr.as_str()) && m.folder_role != Some(FolderRole::Sent)));
    // Adding twice changes nothing; "vip" is the same group.
    assert!(!store.set_group_member("vip", Some(&addr), None, false, Source::App).unwrap().2);
    assert_eq!(store.list_groups().unwrap()[0].member_count, 1);
    // Several groups for one address.
    store.set_group_member("Équipe", Some(&addr), None, false, Source::App).unwrap();
    let (contacts, _) = store.list_contacts(Some(&addr), None, 10).unwrap();
    assert_eq!(contacts[0].groups, vec!["VIP", "Équipe"]);
    // Threads of the group include this thread.
    assert!(store.list_threads_by(None, Some(gid), None, 200).unwrap().iter().any(|t| t.id == thread));
    // Removal is a tombstone; adding back clears it.
    assert!(store.set_group_member("VIP", Some(&addr), None, true, Source::App).unwrap().2);
    let removed: Option<i64> = store
        .conn()
        .query_row("SELECT date_removed FROM contact_group_member WHERE group_id = ?1", [gid], |r| r.get(0))
        .unwrap();
    assert!(removed.is_some());
    store.set_group_member("VIP", Some(&addr), None, false, Source::App).unwrap();
    let rows: i64 = store
        .conn()
        .query_row("SELECT count(*) FROM contact_group_member WHERE group_id = ?1 AND date_removed IS NULL", [gid], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 1);
    // A deleted group recreated under its name reuses the row and starts empty.
    store.conn().execute("UPDATE contact_group SET date_deleted = 1 WHERE id = ?1", [gid]).unwrap();
    let (gid2, _, _) = store.set_group_member("VIP", Some("new@x.example"), None, false, Source::App).unwrap();
    assert_eq!(gid2, gid);
    let vip = store.list_groups().unwrap().into_iter().find(|g| g.id == gid).unwrap();
    assert_eq!(vip.member_count, 1);
    // Audit trail.
    let n: i64 = store
        .conn()
        .query_row("SELECT count(*) FROM audit_log WHERE entity = 'contact_group' AND action = 'add_member'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 4);
}

#[test]
fn contacts_are_filtered_and_paginated() {
    let store = seeded();
    let total: i64 = store.conn().query_row("SELECT count(*) FROM address", [], |r| r.get(0)).unwrap();
    let mut n = 0;
    let mut cursor: Option<String> = None;
    let mut last = String::new();
    loop {
        let (items, next) = store.list_contacts(None, cursor.as_deref(), 25).unwrap();
        for c in &items {
            assert!(c.addr > last);
            last = c.addr.clone();
        }
        n += items.len() as i64;
        match next {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    assert_eq!(n, total);
    // Filter on a part of the name or address; `%` is literal.
    let (items, _) = store.list_contacts(Some("gazette"), None, 50).unwrap();
    assert!(!items.is_empty() && items.iter().all(|c| c.addr.contains("gazette")));
    assert!(items[0].message_count > 0);
    assert!(store.list_contacts(Some("%"), None, 50).unwrap().0.is_empty());
}

#[test]
fn threads_by_contact_include_mail_to_and_from_the_address() {
    let store = seeded();
    let (thread, _) = inbox_thread(&store);
    let (_, msgs) = store.get_thread(thread).unwrap();
    let addr = msgs.iter().find_map(|m| m.from_addr.clone()).unwrap();
    let id: i64 = store.conn().query_row("SELECT id FROM address WHERE addr = ?1", [&addr], |r| r.get(0)).unwrap();
    let list = store.list_threads_by(None, None, Some(id), 500).unwrap();
    assert!(list.iter().any(|t| t.id == thread));
    // Each thread once.
    let ids: std::collections::HashSet<i64> = list.iter().map(|t| t.id).collect();
    assert_eq!(ids.len(), list.len());
    assert!(matches!(store.list_threads_by(None, None, None, 10), Err(StoreError::InvalidParams(_))));
}

#[test]
fn exports_a_message_as_eml_txt_and_md_without_overwriting() {
    let dir = tempfile::tempdir().unwrap();
    let raw = RawStore::open(dir.path()).unwrap();
    let store = Mutex::new(Store::open_in_memory().unwrap());
    let mbox = dir.path().join("a.mbox");
    let mail = "Subject: Devis: v2\nFrom: Alice <alice@a.example>\nTo: me@x.example\nMessage-ID: <1@a>\n\nBonjour\n";
    std::fs::write(&mbox, format!("From a Tue Jun 23 19:20:56 2020\n{mail}")).unwrap();
    let opts = ImportOptions {
        path: mbox,
        account: "arch".into(),
        address: "me@x.example".into(),
        mark_correspondents: false,
        source: Source::Cli,
        batch_size: 10,
    };
    import_mbox(&store, &raw, &opts, &mut |_, _, _| {}).unwrap();
    let store = store.into_inner().unwrap();
    let id: i64 = store.conn().query_row("SELECT id FROM message", [], |r| r.get(0)).unwrap();
    let out = dir.path().join("out");
    std::fs::create_dir(&out).unwrap();

    let eml = store.export_message(&raw, id, ExportFormat::Eml, &out).unwrap();
    assert_eq!(eml.file_name().unwrap(), "2020-06-23 Devis v2.eml");
    assert_eq!(std::fs::read_to_string(&eml).unwrap(), mail);
    let again = store.export_message(&raw, id, ExportFormat::Eml, &out).unwrap();
    assert_eq!(again.file_name().unwrap(), "2020-06-23 Devis v2 (2).eml");

    let txt = std::fs::read_to_string(store.export_message(&raw, id, ExportFormat::Txt, &out).unwrap()).unwrap();
    assert!(txt.starts_with("From: Alice <alice@a.example>\nTo: me@x.example\nDate: 2020-06-23 19:20 UTC\nSubject: Devis: v2\n\nBonjour"));
    let md = std::fs::read_to_string(store.export_message(&raw, id, ExportFormat::Md, &out).unwrap()).unwrap();
    assert!(md.starts_with("# Devis: v2\n\n- **From:** Alice <alice@a.example>"));

    assert!(matches!(store.export_message(&raw, 999, ExportFormat::Txt, &out), Err(StoreError::NotFound(_))));
    assert!(matches!(store.export_message(&raw, id, ExportFormat::Txt, &out.join("absent")), Err(StoreError::InvalidParams(_))));
}
