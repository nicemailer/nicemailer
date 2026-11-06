//! The database side of creating an IMAP account and of its initial import
//! (`setup.rs`; `import-initial.md`, section 11): roles, correspondents,
//! the server-side archiving and the Inbox waiting for it, renaming and
//! removing an account.

use std::collections::BTreeMap;

use nm_core::{FolderRole, Source};
use nm_store::ingest::parse_headers;
use nm_store::insert::Inserter;
use nm_store::reception::Reception;
use nm_store::setup::{ArchiveCriteria, RoleAssignment};
use nm_store::sync::{Arrival, ServerFlags};
use nm_store::Store;

fn count(s: &Store, sql: &str) -> i64 {
    s.conn().query_row(sql, [], |r| r.get(0)).unwrap()
}

fn assign(path: &str, role: FolderRole) -> RoleAssignment {
    RoleAssignment { path: path.into(), delimiter: Some("/".into()), role: Some(role) }
}

#[test]
fn an_account_gets_its_rows_and_roles_once() {
    let mut s = Store::open_in_memory().unwrap();
    let id = s.prepare_imap_account("perso", Source::Cli).unwrap();
    assert_eq!(s.prepare_imap_account("perso", Source::Cli).unwrap(), id, "tried again: the same row");
    s.set_folder_roles(id, &[assign("INBOX", FolderRole::Inbox), assign("Envoyés", FolderRole::Sent), assign("Archive", FolderRole::Archive)], Source::Cli)
        .unwrap();
    assert_eq!(count(&s, "SELECT count(*) FROM folder WHERE role IS NOT NULL"), 3);
    assert_eq!(count(&s, "SELECT count(*) FROM audit_log WHERE action = 'role' AND created_by = 'cli'"), 3);
    // The role moves: one folder per role.
    s.set_folder_roles(id, &[assign("Sent", FolderRole::Sent)], Source::Cli).unwrap();
    assert_eq!(count(&s, "SELECT count(*) FROM folder WHERE role = 'sent'"), 1);
    assert_eq!(count(&s, "SELECT count(*) FROM folder WHERE path = 'Envoyés' AND role IS NULL"), 1);
    assert_eq!(count(&s, "SELECT count(*) FROM audit_log WHERE action = 'create' AND entity = 'account'"), 1);
}

#[test]
fn correspondents_get_inbox_unless_decided_or_own() {
    let mut s = Store::open_in_memory().unwrap();
    s.set_address_screening("news@n.example", Some(nm_core::ScreeningTarget::Newsletter), Source::Cli).unwrap();
    // A decision removed: a tombstone, a choice too.
    s.set_address_screening("gone@g.example", Some(nm_core::ScreeningTarget::Inbox), Source::Cli).unwrap();
    s.set_address_screening("gone@g.example", None, Source::Cli).unwrap();
    let addrs: Vec<String> = ["Bob@B.example", "news@n.example", "gone@g.example", "me@hey.example"].iter().map(|a| a.to_string()).collect();
    let n = s.add_correspondents(&addrs, &["me@hey.example".to_string()]).unwrap();
    assert_eq!(n, 1, "Bob alone");
    assert_eq!(count(&s, "SELECT count(*) FROM address WHERE addr = 'bob@b.example' AND screening_target = 'inbox' AND imported_from = 'imap_initial'"), 1);
    assert_eq!(count(&s, "SELECT count(*) FROM address WHERE addr = 'news@n.example' AND screening_target = 'newsletter' AND imported_from IS NULL"), 1, "kept, as it was");
    assert_eq!(count(&s, "SELECT count(*) FROM address WHERE addr = 'gone@g.example' AND screening_target IS NULL"), 1);
    assert_eq!(count(&s, "SELECT count(*) FROM address WHERE addr = 'me@hey.example'"), 0);
    assert_eq!(count(&s, "SELECT count(*) FROM audit_log WHERE json_extract(detail, '$.reason') = 'sent_correspondent' AND created_by = 'core'"), 1);
}

fn criteria() -> ArchiveCriteria {
    ArchiveCriteria {
        from_path: "INBOX".into(),
        to_path: "Archive".into(),
        boundary: 120,
        read_before: Some("2026-11-07".into()),
        unread_before: Some("2026-10-07".into()),
        batch_size: 1000,
    }
}

#[test]
fn the_inbox_waits_for_the_archiving() {
    let mut s = Store::open_in_memory().unwrap();
    let id = s.prepare_imap_account("perso", Source::Cli).unwrap();
    s.queue_initial_archive(id, &criteria(), Source::Cli).unwrap();
    assert!(s.pending_initial_archive(id).unwrap());
    let claimed = s.spool_claim_account(id).unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].criteria, criteria());
    assert!(s.pending_initial_archive(id).unwrap(), "running is still pending");
    // A temporary failure: later, the same criteria.
    s.initial_archive_retry(claimed[0].id, "connection lost").unwrap();
    assert!(s.spool_claim_account(id).unwrap().is_empty(), "not before its time");
    s.conn().execute("UPDATE pending_action SET date_run_after = 0", []).unwrap();
    let again = s.spool_claim_account(id).unwrap();
    s.initial_archive_done(again[0].id, 17).unwrap();
    assert!(!s.pending_initial_archive(id).unwrap());
    assert_eq!(s.initial_archive_state(id).unwrap(), Some(("done".to_string(), None)));
    // One row for the whole archiving, with its criteria and its count.
    assert_eq!(count(&s, "SELECT json_extract(detail, '$.moved') FROM audit_log WHERE action = 'initial_archive'"), 17);
    assert_eq!(count(&s, "SELECT json_extract(detail, '$.boundary') FROM audit_log WHERE action = 'initial_archive'"), 120);
    // A definitive failure leaves nothing to undo.
    s.queue_initial_archive(id, &criteria(), Source::Cli).unwrap();
    let a = s.spool_claim_account(id).unwrap();
    s.initial_archive_failed(a[0].id, "NO no such mailbox").unwrap();
    assert_eq!(s.initial_archive_state(id).unwrap(), Some(("reverted".to_string(), Some("NO no such mailbox".to_string()))));
}

#[test]
fn addresses_met_by_the_first_synchronization_are_imported() {
    let mut s = Store::open_in_memory().unwrap();
    let id = s.prepare_imap_account("perso", Source::Cli).unwrap();
    s.set_folder_roles(id, &[assign("INBOX", FolderRole::Inbox)], Source::Cli).unwrap();
    s.set_address_screening("known@k.example", Some(nm_core::ScreeningTarget::Inbox), Source::Cli).unwrap();
    let inbox: i64 = s.conn().query_row("SELECT id FROM folder WHERE role = 'inbox'", [], |r| r.get(0)).unwrap();
    let mut ins = Inserter::new(Source::Imap, Some("imap_initial"));
    let a = |uid: u32, from: &str| Arrival {
        uid,
        flags: ServerFlags::default(),
        date_received: 0,
        size: Some(10),
        modseq: None,
        headers: parse_headers(format!("Message-ID: <{uid}@x>\r\nFrom: {from}\r\n\r\n").as_bytes()),
    };
    s.insert_arrivals(&mut ins, id, inbox, &[a(1, "new@n.example"), a(2, "known@k.example")], &mut BTreeMap::new(), &Reception::plain(false))
        .unwrap();
    assert_eq!(count(&s, "SELECT count(*) FROM address WHERE addr = 'new@n.example' AND imported_from = 'imap_initial'"), 1);
    assert_eq!(count(&s, "SELECT count(*) FROM address WHERE addr = 'known@k.example' AND imported_from IS NULL"), 1, "kept");
}

#[test]
fn renaming_keeps_the_data_and_removing_deletes_it() {
    let mut s = Store::open_in_memory().unwrap();
    let id = s.prepare_imap_account("perso", Source::Cli).unwrap();
    s.set_folder_roles(id, &[assign("INBOX", FolderRole::Inbox)], Source::Cli).unwrap();
    s.prepare_imap_account("pro", Source::Cli).unwrap();
    assert!(s.rename_account("perso", "pro", Source::Cli).is_err(), "taken");
    assert_eq!(s.rename_account("perso", "home", Source::Cli).unwrap(), id);
    assert_eq!(count(&s, "SELECT count(*) FROM folder WHERE account_id = (SELECT id FROM account WHERE config_key = 'home')"), 1);
    let changes = s.remove_account("home", Source::Cli).unwrap();
    assert!(changes.accounts.contains(&id));
    assert_eq!(count(&s, "SELECT count(*) FROM account WHERE config_key = 'home'") + count(&s, "SELECT count(*) FROM folder WHERE account_id = 1"), 0);
    assert_eq!(count(&s, "SELECT count(*) FROM audit_log WHERE action IN ('rename', 'remove')"), 2, "the journal outlives the account");
}
