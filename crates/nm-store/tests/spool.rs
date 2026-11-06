//! The spool, database side (`spool.rs`; `modele de donnees.md`, section
//! 6, scenario by scenario): actions written with the local change, merged,
//! claimed one per message in order, done, retried, reverted with the
//! changes after them, resumed after a crash, purged; and what the
//! synchronization does with them.

use std::collections::{BTreeMap, BTreeSet};

use nm_core::{FolderRole, Source};
use nm_store::ingest::parse_headers;
use nm_store::insert::Inserter;
use nm_store::screening::MoveOptions;
use nm_store::spool::{ActionKind, MAX_ATTEMPTS};
use nm_store::sync::{Arrival, FlagUpdate, FolderState, ListedFolder, ServerFlags};
use nm_store::Store;
use nm_store::reception::Reception;

struct Env {
    s: Store,
    acc: i64,
    inbox: i64,
    archive: i64,
    todo: i64,
    thread: i64,
    message: i64,
}

fn count(s: &Store, sql: &str) -> i64 {
    s.conn().query_row(sql, [], |r| r.get(0)).unwrap()
}

fn arrival(uid: u32, msgid: &str) -> Arrival {
    let header = format!("Message-ID: <{msgid}>\r\nFrom: alice@a.example\r\nSubject: Devis\r\n\r\n");
    Arrival { uid, flags: ServerFlags::default(), date_received: 1_700_000_000, size: Some(100), modseq: None, headers: parse_headers(header.as_bytes()) }
}

/// A synchronized account (in the server accounts) with one mail in the
/// Inbox, UID 7.
fn env() -> Env {
    let mut s = Store::open_in_memory().unwrap();
    s.conn().execute("INSERT INTO account (config_key, kind, created_by, modified_by) VALUES ('perso', 'imap', 'core', 'core')", []).unwrap();
    let acc = s.conn().last_insert_rowid();
    let listed = |p: &str, r: &'static str, uv| ListedFolder { path: p.into(), delimiter: Some("/".into()), proposed_role: Some(r), uidvalidity: Some(uv) };
    s.sync_folder_list(acc, &[listed("INBOX", "inbox", 1), listed("Archive", "archive", 2), listed("Todo", "todo", 3)]).unwrap();
    let id = |s: &Store, p: &str| -> i64 { s.conn().query_row("SELECT id FROM folder WHERE path = ?1", [p], |r| r.get(0)).unwrap() };
    let (inbox, archive, todo) = (id(&s, "INBOX"), id(&s, "Archive"), id(&s, "Todo"));
    let mut ins = Inserter::new(Source::Imap, None);
    s.insert_arrivals(&mut ins, acc, inbox, &[arrival(7, "m1@x")], &mut BTreeMap::new(), &Reception::plain(false)).unwrap();
    s.set_server_accounts(BTreeSet::from([acc])).unwrap();
    let (thread, message): (i64, i64) = s.conn().query_row("SELECT thread_id, id FROM message", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    Env { s, acc, inbox, archive, todo, thread, message }
}

fn actions(s: &Store) -> Vec<(String, String, String)> {
    s.conn()
        .prepare("SELECT kind, state, payload FROM pending_action ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(|r| r.unwrap())
        .collect()
}

fn place(s: &Store, id: i64) -> (i64, Option<u32>) {
    s.conn().query_row("SELECT folder_id, uid FROM message WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?))).unwrap()
}

fn mv(e: &mut Env, to: FolderRole) {
    let out = e.s.move_thread(e.thread, to, None, MoveOptions::default(), Source::Cli).unwrap();
    assert!(out.changes.spooled.contains(&e.acc), "the worker is to be woken");
}

#[test]
fn a_move_is_local_at_once_with_its_action() {
    let mut e = env();
    mv(&mut e, FolderRole::Archive);
    assert_eq!(place(&e.s, e.message), (e.archive, None), "in Archive, UID unknown until the server says");
    let a = actions(&e.s);
    assert_eq!(a.len(), 1);
    assert_eq!((a[0].0.as_str(), a[0].1.as_str()), ("move", "queued"));
    let p: serde_json::Value = serde_json::from_str(&a[0].2).unwrap();
    assert_eq!((p["from_folder_id"].as_i64(), p["from_uid"].as_u64(), p["to_folder_id"].as_i64()), (Some(e.inbox), Some(7), Some(e.archive)));
}

#[test]
fn an_account_not_synchronized_gets_no_action() {
    let mut e = env();
    // A fake account of `make seed`, an archive account: changed locally,
    // nothing to send.
    e.s.set_server_accounts(BTreeSet::new()).unwrap();
    let out = e.s.move_thread(e.thread, FolderRole::Archive, None, MoveOptions::default(), Source::Cli).unwrap();
    assert!(out.changes.spooled.is_empty());
    e.s.set_thread_seen(e.thread, true, Source::Cli).unwrap();
    assert!(actions(&e.s).is_empty());
}

#[test]
fn successive_moves_are_merged_and_a_return_cancels() {
    let mut e = env();
    mv(&mut e, FolderRole::Todo);
    mv(&mut e, FolderRole::Archive);
    let a = actions(&e.s);
    assert_eq!(a.len(), 1, "Inbox → Todo → Archive is one move");
    let p: serde_json::Value = serde_json::from_str(&a[0].2).unwrap();
    assert_eq!((p["from_folder_id"].as_i64(), p["to_folder_id"].as_i64()), (Some(e.inbox), Some(e.archive)));
    // Back to the Inbox before anything was sent: nothing to send, and the
    // message has its UID again.
    mv(&mut e, FolderRole::Inbox);
    assert_eq!(actions(&e.s)[0].1, "cancelled");
    assert_eq!(place(&e.s, e.message), (e.inbox, Some(7)));
}

#[test]
fn a_flag_set_then_cleared_before_being_sent_sends_nothing() {
    let mut e = env();
    e.s.set_thread_seen(e.thread, true, Source::Cli).unwrap();
    let a = actions(&e.s);
    assert_eq!((a[0].0.as_str(), a[0].1.as_str()), ("set_flags", "queued"));
    assert!(a[0].2.contains("\\\\Seen"), "{}", a[0].2);
    e.s.set_thread_seen(e.thread, false, Source::Cli).unwrap();
    assert_eq!(actions(&e.s)[0].1, "cancelled");
    assert_eq!(actions(&e.s).len(), 1);
    // A tag is a keyword.
    e.s.tag_thread(e.thread, "Projet", false, Source::Cli).unwrap();
    let last = actions(&e.s).pop().unwrap();
    assert!(last.2.contains("\"set\":true") && last.2.contains("Projet"), "{}", last.2);
}

#[test]
fn actions_of_a_message_run_one_at_a_time_in_order() {
    let mut e = env();
    mv(&mut e, FolderRole::Archive);
    e.s.set_thread_flagged(e.thread, true, Source::Cli).unwrap();
    let claimed = e.s.spool_claim(e.acc, 100).unwrap();
    assert_eq!(claimed.len(), 1, "the flag waits for the move");
    assert!(matches!(claimed[0].kind, ActionKind::Move { from_uid: Some(7), .. }));
    assert!(e.s.spool_claim(e.acc, 100).unwrap().is_empty(), "the move is running, the flag still waits");
    // Done, with the UID the server gave in Archive: the flag can go.
    e.s.spool_done(claimed[0].id, Some(3)).unwrap();
    assert_eq!(place(&e.s, e.message), (e.archive, Some(3)));
    let next = e.s.spool_claim(e.acc, 100).unwrap();
    assert!(matches!(&next[0].kind, ActionKind::SetFlag { uid: 3, flag, set: true, .. } if flag == "\\Flagged"), "{next:?}");
}

#[test]
fn a_flag_on_a_message_without_uid_waits() {
    let mut e = env();
    mv(&mut e, FolderRole::Archive);
    let m = e.s.spool_claim(e.acc, 10).unwrap();
    e.s.set_thread_seen(e.thread, true, Source::Cli).unwrap();
    // Done without UIDPLUS: no new UID, the flag cannot be sent yet.
    e.s.spool_done(m[0].id, None).unwrap();
    assert!(e.s.spool_claim(e.acc, 10).unwrap().is_empty());
    // The pass attaches the UID: now it can.
    let mut ins = Inserter::new(Source::Imap, None);
    let (got, _) = e.s.insert_arrivals(&mut ins, e.acc, e.archive, &[arrival(5, "m1@x")], &mut BTreeMap::new(), &Reception::plain(true)).unwrap();
    assert_eq!((got.attached, got.inserted), (1, 0));
    assert_eq!(place(&e.s, e.message), (e.archive, Some(5)));
    assert_eq!(e.s.spool_claim(e.acc, 10).unwrap().len(), 1);
}

#[test]
fn a_move_done_after_the_message_moved_on_gives_its_uid_to_the_next_move() {
    let mut e = env();
    mv(&mut e, FolderRole::Todo);
    let first = e.s.spool_claim(e.acc, 10).unwrap();
    // Moved again while the first move runs: not merged, queued after it.
    mv(&mut e, FolderRole::Archive);
    assert_eq!(actions(&e.s).len(), 2);
    e.s.spool_done(first[0].id, Some(9)).unwrap();
    assert_eq!(place(&e.s, e.message), (e.archive, None), "not the UID of Todo");
    let next = e.s.spool_claim(e.acc, 10).unwrap();
    assert!(matches!(&next[0].kind, ActionKind::Move { from_folder, from_uid: Some(9), .. } if *from_folder == e.todo), "{next:?}");
}

#[test]
fn temporary_failures_wait_longer_each_time_then_give_up() {
    let mut e = env();
    mv(&mut e, FolderRole::Archive);
    let id = e.s.spool_claim(e.acc, 10).unwrap()[0].id;
    e.s.spool_retry(id, "connection lost").unwrap();
    let (state, attempts, wait): (String, i64, i64) = e
        .s
        .conn()
        .query_row("SELECT state, attempts, date_run_after - unixepoch() FROM pending_action", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap();
    assert_eq!((state.as_str(), attempts), ("queued", 1));
    assert!((29..=30).contains(&wait), "{wait}");
    assert!(e.s.spool_claim(e.acc, 10).unwrap().is_empty(), "not before its time");
    for _ in 1..MAX_ATTEMPTS - 1 {
        e.s.spool_retry(id, "connection lost").unwrap();
    }
    // 30 s doubled six times: 32 min. With 8 attempts the cap of an hour
    // is never reached; the attempts span about an hour in all.
    let wait: i64 = e.s.conn().query_row("SELECT date_run_after - unixepoch() FROM pending_action", [], |r| r.get(0)).unwrap();
    assert!((1919..=1920).contains(&wait), "{wait}");
    let changes = e.s.spool_retry(id, "connection lost").unwrap();
    assert_eq!(changes.reverted.len(), 1, "the 8th failure is definitive");
    assert_eq!(place(&e.s, e.message), (e.inbox, Some(7)));
}

#[test]
fn a_refusal_undoes_the_change_and_the_ones_after_it() {
    let mut e = env();
    mv(&mut e, FolderRole::Archive);
    e.s.set_thread_flagged(e.thread, true, Source::Cli).unwrap();
    let id = e.s.spool_claim(e.acc, 10).unwrap()[0].id;
    let changes = e.s.spool_revert(id, "NO [TRYCREATE] no such mailbox").unwrap();
    assert_eq!(place(&e.s, e.message), (e.inbox, Some(7)), "back in the Inbox with its UID");
    assert_eq!(count(&e.s, "SELECT is_flagged FROM message"), 0, "the later flag undone too");
    let states: Vec<String> = actions(&e.s).into_iter().map(|a| a.1).collect();
    assert_eq!(states, ["reverted", "cancelled"]);
    assert_eq!(changes.reverted, [(e.message, "move".to_string(), "NO [TRYCREATE] no such mailbox".to_string())]);
    assert!(changes.folders.contains(&e.inbox) && changes.folders.contains(&e.archive));
    assert_eq!(count(&e.s, "SELECT total_count FROM folder WHERE path = 'INBOX'"), 1);
    assert_eq!(count(&e.s, "SELECT count(*) FROM audit_log WHERE action = 'revert'"), 1);
}

#[test]
fn running_actions_are_queued_again_at_start_and_ended_ones_purged() {
    let mut e = env();
    mv(&mut e, FolderRole::Archive);
    e.s.spool_claim(e.acc, 10).unwrap();
    assert_eq!(e.s.spool_requeue_running().unwrap(), 1);
    assert_eq!(actions(&e.s)[0].1, "queued");
    let id = e.s.spool_claim(e.acc, 10).unwrap()[0].id;
    e.s.spool_done(id, Some(2)).unwrap();
    assert_eq!(e.s.spool_purge().unwrap(), 0, "kept a week");
    e.s.conn().execute("UPDATE pending_action SET date_modified = unixepoch() - 8 * 86400", []).unwrap();
    assert_eq!(e.s.spool_purge().unwrap(), 1);
}

#[test]
fn the_synchronization_does_not_undo_what_waits_in_the_spool() {
    let mut e = env();
    mv(&mut e, FolderRole::Archive);
    // The server still shows UID 7 in the Inbox: ignored, not new mail.
    assert_eq!(e.s.pending_source_uids(e.inbox).unwrap(), BTreeSet::from([7]));
    // A flag set locally, not sent: the server's old value does not win.
    e.s.set_thread_seen(e.thread, true, Source::Cli).unwrap();
    let mut ins = Inserter::new(Source::Imap, None);
    // The move is done (no COPYUID): the pass attaches the UID in Archive.
    let id = e.s.spool_claim(e.acc, 10).unwrap()[0].id;
    e.s.spool_done(id, None).unwrap();
    e.s.insert_arrivals(&mut ins, e.acc, e.archive, &[arrival(4, "m1@x")], &mut BTreeMap::new(), &Reception::plain(true)).unwrap();
    let unseen = [FlagUpdate { uid: 4, flags: ServerFlags::default(), modseq: None }];
    e.s.finish_folder(e.archive, &unseen, &BTreeSet::new(), FolderState { uidvalidity: 2, uidnext: Some(5), highestmodseq: None }, false).unwrap();
    assert_eq!(count(&e.s, "SELECT is_seen FROM message"), 1, "the local value stays until sent");
    assert_eq!(count(&e.s, "SELECT count(*) FROM message"), 1, "attached, not inserted twice");
}

#[test]
fn a_row_left_without_uid_and_without_action_is_dropped() {
    let mut e = env();
    mv(&mut e, FolderRole::Archive);
    let id = e.s.spool_claim(e.acc, 10).unwrap()[0].id;
    // The move found the message gone (moved elsewhere by the phone), and
    // it is not in Archive either: done, without UID.
    e.s.spool_done(id, None).unwrap();
    e.s.finish_folder(e.archive, &[], &BTreeSet::new(), FolderState { uidvalidity: 2, uidnext: Some(1), highestmodseq: None }, false).unwrap();
    assert_eq!(count(&e.s, "SELECT count(*) FROM message"), 0, "it shows again where it really is");
    // But a row whose move is still to be sent stays.
    let mut e = env();
    mv(&mut e, FolderRole::Archive);
    e.s.finish_folder(e.archive, &[], &BTreeSet::new(), FolderState { uidvalidity: 2, uidnext: Some(1), highestmodseq: None }, false).unwrap();
    assert_eq!(count(&e.s, "SELECT count(*) FROM message"), 1);
}
