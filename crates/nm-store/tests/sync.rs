//! The database side of the IMAP synchronization (`sync.rs`), without a
//! server: folder listing and roles, arrivals and moves made elsewhere,
//! flags, deletions, purge after a UIDVALIDITY change, and the refusal of
//! commands on IMAP accounts until phase 5.

use std::collections::{BTreeMap, BTreeSet};

use nm_core::Source;
use nm_store::ingest::parse_headers;
use nm_store::insert::Inserter;
use nm_store::sync::{Arrival, FlagUpdate, FolderState, ListedFolder, ServerFlags};
use nm_store::Store;
use nm_store::reception::Reception;

fn store() -> (Store, i64) {
    let store = Store::open_in_memory().unwrap();
    store
        .conn()
        .execute("INSERT INTO account (config_key, kind, created_by, modified_by) VALUES ('perso', 'imap', 'core', 'core')", [])
        .unwrap();
    let id = store.conn().last_insert_rowid();
    (store, id)
}

fn listed(path: &str, role: Option<&'static str>, uidvalidity: u32) -> ListedFolder {
    ListedFolder { path: path.into(), delimiter: Some("/".into()), proposed_role: role, uidvalidity: Some(uidvalidity) }
}

fn standard() -> Vec<ListedFolder> {
    vec![
        listed("INBOX", Some("inbox"), 1),
        listed("Archive", Some("archive"), 2),
        listed("Sent", Some("sent"), 3),
        listed("Clients", None, 4),
    ]
}

fn count(s: &Store, sql: &str) -> i64 {
    s.conn().query_row(sql, [], |r| r.get(0)).unwrap()
}

fn arrival(uid: u32, msgid: &str, size: u64, flags: &[&str]) -> Arrival {
    let header = format!("Message-ID: <{msgid}>\r\nFrom: alice@a.example\r\nSubject: Devis {msgid}\r\n\r\n");
    Arrival {
        uid,
        flags: ServerFlags::parse(flags),
        date_received: 1_700_000_000 + uid as i64,
        size: Some(size),
        modseq: Some(uid as u64),
        headers: parse_headers(header.as_bytes()),
    }
}

fn folder_id(s: &Store, path: &str) -> i64 {
    s.conn().query_row("SELECT id FROM folder WHERE path = ?1", [path], |r| r.get(0)).unwrap()
}

#[test]
fn the_first_listing_gives_the_proposed_roles_and_later_ones_do_not() {
    let (mut s, acc) = store();
    let (folders, change, _) = s.sync_folder_list(acc, &standard()).unwrap();
    assert_eq!(folders.iter().map(|f| f.role.as_str()).collect::<Vec<_>>(), ["inbox", "sent", "archive"], "Inbox first");
    assert_eq!(change.created.len(), 4);
    assert_eq!(change.roles_given.len(), 3);
    assert_eq!(count(&s, "SELECT count(*) FROM folder WHERE role IS NULL"), 1, "Clients: no role, not synchronized");
    assert_eq!(count(&s, "SELECT count(*) FROM audit_log WHERE action = 'role'"), 3);

    // A folder created later on the server, even with a known name, gets
    // no role: the user chose the roles once (phase 7 will ask).
    let mut more = standard();
    more.push(listed("Junk", Some("junk"), 5));
    let (folders, change, _) = s.sync_folder_list(acc, &more).unwrap();
    assert_eq!(folders.len(), 3);
    assert_eq!(change.created, ["Junk"]);
    assert!(change.roles_given.is_empty());
}

#[test]
fn a_folder_renamed_on_the_server_keeps_its_row_and_its_mail() {
    let (mut s, acc) = store();
    s.sync_folder_list(acc, &standard()).unwrap();
    let archive = folder_id(&s, "Archive");
    s.conn().execute("UPDATE folder SET uidvalidity = 2 WHERE id = ?1", [archive]).unwrap();
    let mut ins = Inserter::new(Source::Imap, None);
    s.insert_arrivals(&mut ins, acc, archive, &[arrival(1, "a@x", 100, &[])], &mut BTreeMap::new(), &Reception::plain(false)).unwrap();

    let mut renamed = standard();
    renamed[1] = listed("Archives 2026", Some("archive"), 2);
    let (folders, change, _) = s.sync_folder_list(acc, &renamed).unwrap();
    assert_eq!(change.renamed, [("Archive".to_string(), "Archives 2026".to_string())]);
    assert_eq!(folder_id(&s, "Archives 2026"), archive);
    assert!(folders.iter().any(|f| f.id == archive && f.role == "archive"));
    assert_eq!(count(&s, "SELECT count(*) FROM message"), 1);
}

#[test]
fn a_role_folder_gone_is_kept_and_a_folder_without_role_is_dropped() {
    let (mut s, acc) = store();
    s.sync_folder_list(acc, &standard()).unwrap();
    // Sent and Clients disappear, nothing has their UIDVALIDITY.
    let fewer = vec![listed("INBOX", Some("inbox"), 1), listed("Archive", Some("archive"), 2)];
    let (folders, change, _) = s.sync_folder_list(acc, &fewer).unwrap();
    assert_eq!(change.missing, ["Sent"]);
    assert_eq!(folders.len(), 2, "Sent is not synchronized while it is missing");
    assert_eq!(count(&s, "SELECT count(*) FROM folder WHERE path = 'Sent'"), 1);
    assert_eq!(count(&s, "SELECT count(*) FROM folder WHERE path = 'Clients'"), 0);
}

#[test]
fn flags_and_keywords_are_read_without_case() {
    let f = ServerFlags::parse(&["\\Seen", "\\FLAGGED", "$Forwarded", "$forwarded", "\\Recent", "\\Something", "Projet"]);
    assert!(f.flags.seen && f.flags.flagged && !f.flags.answered);
    assert_eq!(f.keywords, ["$Forwarded", "Projet"]);
}

/// Inbox and Archive, synchronized once, with one message in the Inbox.
fn with_one_mail() -> (Store, i64, i64, i64, Inserter) {
    let (mut s, acc) = store();
    s.sync_folder_list(acc, &standard()).unwrap();
    let (inbox, archive) = (folder_id(&s, "INBOX"), folder_id(&s, "Archive"));
    let mut ins = Inserter::new(Source::Imap, None);
    let (got, _) = s.insert_arrivals(&mut ins, acc, inbox, &[arrival(7, "m1@x", 1000, &["$Projet"])], &mut BTreeMap::new(), &Reception::plain(false)).unwrap();
    assert_eq!(got.inserted, 1);
    (s, acc, inbox, archive, ins)
}

#[test]
fn a_move_made_elsewhere_keeps_the_message() {
    let (mut s, acc, inbox, archive, mut ins) = with_one_mail();
    let id: i64 = s.conn().query_row("SELECT id FROM message", [], |r| r.get(0)).unwrap();
    // The phone moved it: UID 7 vanished from the Inbox, UID 3 appeared in
    // Archive, in the same pass.
    let mut deletions = BTreeMap::from([(inbox, BTreeSet::from([7]))]);
    let (got, changes) = s.insert_arrivals(&mut ins, acc, archive, &[arrival(3, "m1@x", 1000, &["\\Seen"])], &mut deletions, &Reception::plain(true)).unwrap();
    assert_eq!((got.moved, got.inserted), (1, 0));
    assert!(deletions[&inbox].is_empty(), "no longer a deletion");
    assert!(changes.arrived.is_empty(), "a move is not new mail");
    let (fid, uid, seen): (i64, u32, bool) =
        s.conn().query_row("SELECT folder_id, uid, is_seen FROM message WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
    assert_eq!((fid, uid, seen), (archive, 3, true), "same id, new place and flags");
    assert_eq!(count(&s, "SELECT count(*) FROM message_keyword"), 0, "the keywords are the server's");
    // Changes seen on the server are not journaled (`tracabilite.md`).
    assert_eq!(count(&s, "SELECT count(*) FROM audit_log WHERE action = 'move'"), 0);
    assert_eq!(count(&s, "SELECT count(*) FROM message WHERE modified_by = 'imap'"), 1);
    // The end of the Inbox deletes nothing more.
    s.finish_folder(inbox, &[], &deletions[&inbox], FolderState { uidvalidity: 1, uidnext: Some(8), highestmodseq: None }, false).unwrap();
    assert_eq!(count(&s, "SELECT count(*) FROM message"), 1);
    assert_eq!(count(&s, "SELECT total_count FROM folder WHERE path = 'Archive'"), 1);
    assert_eq!(count(&s, "SELECT total_count FROM folder WHERE path = 'INBOX'"), 0);
}

#[test]
fn a_copy_or_an_uncertain_move_is_a_new_message() {
    // The original stayed (no deletion): a copy.
    let (mut s, acc, inbox, archive, mut ins) = with_one_mail();
    let (got, changes) = s.insert_arrivals(&mut ins, acc, archive, &[arrival(3, "m1@x", 1000, &[])], &mut BTreeMap::new(), &Reception::plain(true)).unwrap();
    assert_eq!((got.moved, got.inserted), (0, 1));
    assert_eq!(changes.arrived.len(), 1);
    let threads = count(&s, "SELECT count(DISTINCT thread_id) FROM message");
    assert_eq!(threads, 1, "two copies of a mail are one conversation");

    // Another size: not the same message, whatever its Message-ID says.
    let (mut s, acc, inbox2, archive, mut ins) = with_one_mail();
    let mut deletions = BTreeMap::from([(inbox2, BTreeSet::from([7]))]);
    let (got, _) = s.insert_arrivals(&mut ins, acc, archive, &[arrival(3, "m1@x", 999, &[])], &mut deletions, &Reception::plain(true)).unwrap();
    assert_eq!((got.moved, got.inserted), (0, 1));
    assert_eq!(deletions[&inbox2].len(), 1, "still to delete");
    let _ = inbox;
}

#[test]
fn flags_deletions_and_state_in_one_transaction() {
    let (mut s, acc, inbox, _, mut ins) = with_one_mail();
    s.insert_arrivals(&mut ins, acc, inbox, &[arrival(8, "m2@x", 10, &[])], &mut BTreeMap::new(), &Reception::plain(true)).unwrap();
    assert_eq!(count(&s, "SELECT unread_count FROM folder WHERE path = 'INBOX'"), 2);
    let flags = [FlagUpdate { uid: 8, flags: ServerFlags::parse(&["\\Seen", "\\Flagged", "Urgent"]), modseq: Some(40) }];
    let changes = s
        .finish_folder(inbox, &flags, &BTreeSet::from([7, 99]), FolderState { uidvalidity: 1, uidnext: Some(9), highestmodseq: Some(40) }, false)
        .unwrap();
    assert_eq!(changes.messages.len(), 2, "one changed, one deleted");
    assert_eq!(count(&s, "SELECT count(*) FROM message"), 1);
    assert_eq!(count(&s, "SELECT is_seen + is_flagged FROM message"), 2);
    assert_eq!(count(&s, "SELECT count(*) FROM message WHERE modified_by = 'imap' AND modseq = 40"), 1);
    assert_eq!(count(&s, "SELECT count(*) FROM message_keyword WHERE keyword = 'Urgent'"), 1);
    assert_eq!(count(&s, "SELECT unread_count FROM folder WHERE path = 'INBOX'"), 0);
    assert_eq!(count(&s, "SELECT highestmodseq FROM folder WHERE path = 'INBOX'"), 40);
    assert_eq!(count(&s, "SELECT uidnext FROM folder WHERE path = 'INBOX'"), 9);
    assert_eq!(count(&s, "SELECT count(*) FROM thread"), 1, "the thread of the deleted mail is gone");
    // Applied twice (a crash before the next pass): nothing more changes.
    let again = s
        .finish_folder(inbox, &flags, &BTreeSet::from([7]), FolderState { uidvalidity: 1, uidnext: Some(9), highestmodseq: Some(40) }, false)
        .unwrap();
    assert!(again.messages.is_empty());
}

#[test]
fn arrivals_advance_uidnext_and_announce_only_after_the_first_sync() {
    let (mut s, acc) = store();
    s.sync_folder_list(acc, &standard()).unwrap();
    let inbox = folder_id(&s, "INBOX");
    let mut ins = Inserter::new(Source::Imap, None);
    let (_, first) = s.insert_arrivals(&mut ins, acc, inbox, &[arrival(1, "a@x", 1, &[]), arrival(5, "b@x", 1, &[])], &mut BTreeMap::new(), &Reception::plain(false)).unwrap();
    assert!(first.arrived.is_empty());
    assert_eq!(count(&s, "SELECT uidnext FROM folder WHERE id = (SELECT id FROM folder WHERE path = 'INBOX')"), 6);
    let (_, later) = s.insert_arrivals(&mut ins, acc, inbox, &[arrival(6, "c@x", 1, &[])], &mut BTreeMap::new(), &Reception::plain(true)).unwrap();
    assert_eq!(later.arrived.len(), 1);
    // The same UID again (a pass after a crash, or a message the spool
    // moved here with its UID): nothing new, never two rows.
    let (again, changes) = s.insert_arrivals(&mut ins, acc, inbox, &[arrival(6, "c@x", 1, &[])], &mut BTreeMap::new(), &Reception::plain(true)).unwrap();
    assert_eq!(again.inserted, 0);
    assert!(changes.arrived.is_empty());
    assert_eq!(count(&s, "SELECT count(*) FROM message"), 3);
    let known = s.known_messages(inbox).unwrap();
    assert_eq!(known.iter().map(|k| k.uid).collect::<Vec<_>>(), [1, 5, 6]);
}

#[test]
fn a_new_uidvalidity_purges_the_folder() {
    let (mut s, _, inbox, _, _) = with_one_mail();
    // A screening boundary set (account created): a new UIDVALIDITY during
    // the import makes everything there count as imported, the pass sets
    // the boundary again to the new UIDNEXT (`import-initial.md`, section 5).
    s.set_screening_start(inbox, 5).unwrap();
    let (n, changes) = s.purge_folder(inbox, 77).unwrap();
    assert_eq!(count(&s, "SELECT uid_screening_start IS NULL FROM folder WHERE path = 'INBOX'"), 1);
    assert_eq!(n, 1);
    assert!(changes.folders.contains(&inbox));
    assert_eq!(count(&s, "SELECT count(*) FROM message") + count(&s, "SELECT count(*) FROM thread"), 0);
    assert_eq!(count(&s, "SELECT uidvalidity FROM folder WHERE path = 'INBOX'"), 77);
    assert_eq!(count(&s, "SELECT uidnext IS NULL FROM folder WHERE path = 'INBOX'"), 1);
    assert_eq!(count(&s, "SELECT count(*) FROM audit_log WHERE created_by = 'imap' AND action <> 'role'"), 0, "not journaled");
}

#[test]
fn bodies_are_fetched_inbox_first_then_newest_first() {
    let (mut s, acc, inbox, archive, mut ins) = with_one_mail();
    s.insert_arrivals(&mut ins, acc, archive, &[arrival(1, "old@x", 1, &[]), arrival(2, "new@x", 1, &[])], &mut BTreeMap::new(), &Reception::plain(false))
        .unwrap();
    let order: Vec<(i64, u32)> = s.bodies_missing(acc).unwrap().iter().map(|b| (b.folder_id, b.uid)).collect();
    assert_eq!(order, [(inbox, 7), (archive, 2), (archive, 1)]);
}

