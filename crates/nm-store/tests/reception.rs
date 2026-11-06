//! The reception of a mail (`reception.rs`): verdict, screening of the
//! Inbox, return of a conversation from Todo or Waiting, effects of a mail
//! found in Sent (`authentification-expediteur.md`, section 8;
//! `dossiers.md`). Without a server: arrivals are given to the store as the
//! synchronization would.

use std::collections::{BTreeMap, BTreeSet};

use nm_core::{AuthPolicy, FolderRole, ScreeningTarget, Source};
use nm_store::ingest::parse_headers;
use nm_store::insert::Inserter;
use nm_store::reception::Reception;
use nm_store::screening::MoveOptions;
use nm_store::sync::{Arrival, ListedFolder, ServerFlags};
use nm_store::{ChangeSet, Store};

const SERVER: &str = "mx.ours.example";
const ME: &str = "me@hey.example";

struct Env {
    s: Store,
    acc: i64,
    ins: Inserter,
    own: Vec<String>,
    next_uid: u32,
    /// A read-only account: the screening runs dry.
    dry_run: bool,
}

fn env() -> Env {
    let mut s = Store::open_in_memory().unwrap();
    s.conn().execute("INSERT INTO account (config_key, kind, created_by, modified_by) VALUES ('perso', 'imap', 'core', 'core')", []).unwrap();
    let acc = s.conn().last_insert_rowid();
    let roles = ["inbox", "quarantine", "newsletter", "paperwork", "banned", "todo", "waiting", "archive", "sent"];
    let listed: Vec<ListedFolder> = roles
        .iter()
        .enumerate()
        .map(|(i, r)| ListedFolder { path: if *r == "inbox" { "INBOX".into() } else { r.to_string() }, delimiter: Some("/".into()), proposed_role: Some(r), uidvalidity: Some(i as u32 + 1) })
        .collect();
    s.sync_folder_list(acc, &listed).unwrap();
    s.set_server_accounts(BTreeSet::from([acc])).unwrap();
    Env { s, acc, ins: Inserter::new(Source::Imap, None), own: vec![ME.to_string()], next_uid: 10, dry_run: false }
}

fn folder(s: &Store, role: &str) -> i64 {
    s.conn().query_row("SELECT id FROM folder WHERE role = ?1", [role], |r| r.get(0)).unwrap()
}

fn count(s: &Store, sql: &str) -> i64 {
    s.conn().query_row(sql, [], |r| r.get(0)).unwrap()
}

/// A header block from `from`, with an `Authentication-Results` of the
/// server giving `auth` (`dmarc=pass`…), and more header lines.
fn mail(id: &str, from: &str, auth: Option<&str>, extra: &[&str]) -> String {
    let mut h = String::new();
    if let Some(a) = auth {
        h.push_str(&format!("Authentication-Results: {SERVER}; {a}\r\n"));
    }
    h.push_str(&format!("Message-ID: <{id}>\r\nFrom: {from}\r\nTo: {ME}\r\nSubject: Mail {id}\r\n"));
    for e in extra {
        h.push_str(e);
        h.push_str("\r\n");
    }
    h.push_str("\r\n");
    h
}

impl Env {
    /// The mail arrives in the folder of `role`, as the synchronization
    /// gives it after the folder's first synchronization; returns its id
    /// and the role of the folder it ends in.
    fn arrive(&mut self, role: &str, header: &str) -> (i64, String, ChangeSet) {
        self.arrive_with(role, header, Some(SERVER), Some(1))
    }

    fn arrive_with(&mut self, role: &str, header: &str, authserv: Option<&str>, screen_from: Option<u32>) -> (i64, String, ChangeSet) {
        let uid = self.next_uid;
        self.next_uid += 1;
        let a = Arrival {
            uid,
            flags: ServerFlags::default(),
            date_received: 1_700_000_000 + uid as i64,
            size: Some(header.len() as u64),
            modseq: None,
            headers: parse_headers(header.as_bytes()),
        };
        let rc = Reception { role, announce: true, keywords_local: false, authserv_id: authserv, own: &self.own, screen_from_uid: screen_from, dry_run: self.dry_run };
        let f = folder(&self.s, role);
        let (_, changes) = self.s.insert_arrivals(&mut self.ins, self.acc, f, &[a], &mut BTreeMap::new(), &rc).unwrap();
        let id: i64 = self.s.conn().query_row("SELECT max(id) FROM message", [], |r| r.get(0)).unwrap();
        (id, self.role_of(id), changes)
    }

    fn role_of(&self, id: i64) -> String {
        self.s.conn().query_row("SELECT f.role FROM message m JOIN folder f ON f.id = m.folder_id WHERE m.id = ?1", [id], |r| r.get(0)).unwrap()
    }

    fn int(&self, sql: &str, id: i64) -> i64 {
        self.s.conn().query_row(sql, [id], |r| r.get(0)).unwrap()
    }

    fn decide(&mut self, addr: &str, t: ScreeningTarget) {
        self.s.set_address_screening(addr, Some(t), Source::Cli).unwrap();
    }

    fn screen_rule(&self, id: i64) -> String {
        self.s
            .conn()
            .query_row("SELECT json_extract(detail, '$.rule') FROM audit_log WHERE action = 'screen' AND entity_id = ?1", [id], |r| r.get(0))
            .unwrap()
    }
}

#[test]
fn an_unknown_sender_goes_to_quarantine_through_the_spool() {
    let mut e = env();
    let (id, role, changes) = e.arrive("inbox", &mail("u1@x", "stranger@far.example", Some("dmarc=pass"), &[]));
    assert_eq!(role, "quarantine");
    assert_eq!(e.screen_rule(id), "unknown_sender");
    assert_eq!(e.int("SELECT auth_result = 'pass' AND auth_source = 'server' FROM message WHERE id = ?1", id), 1);
    assert_eq!(count(&e.s, "SELECT count(*) FROM pending_action WHERE kind = 'move'"), 1, "the server's copy moves too");
    assert!(changes.arrived.contains(&(id, folder(&e.s, "quarantine"))), "announced where it was filed");
}

#[test]
fn the_examples_of_the_specification() {
    let mut e = env();
    // A forger of support@banque.fr (DMARC reject): fail.
    e.decide("support@banque.fr", ScreeningTarget::Inbox);
    let (id, role, _) = e.arrive("inbox", &mail("f1@x", "support@banque.fr", Some("dmarc=fail"), &[]));
    assert_eq!(role, "quarantine");
    assert_eq!(e.int("SELECT auth_suspect FROM message WHERE id = ?1", id), 1, "warned of probable spoofing");
    // A domain without DKIM nor DMARC, never seen authenticated: let in,
    // unverified (the uncovered case).
    e.decide("contact@pme.fr", ScreeningTarget::Inbox);
    let (id, role, _) = e.arrive("inbox", &mail("p1@x", "contact@pme.fr", Some("dmarc=none"), &[]));
    assert_eq!((role.as_str(), e.int("SELECT auth_suspect FROM message WHERE id = ?1", id)), ("inbox", 0));
    // A colleague seen authenticated, then from a CRM without signature.
    e.decide("anne@entreprise.fr", ScreeningTarget::Inbox);
    assert_eq!(e.arrive("inbox", &mail("c1@x", "anne@entreprise.fr", Some("dmarc=pass"), &[])).1, "inbox");
    assert_eq!(count(&e.s, "SELECT auth_seen FROM address WHERE addr = 'anne@entreprise.fr'"), 1);
    assert_eq!(e.arrive("inbox", &mail("c2@x", "anne@entreprise.fr", Some("dmarc=none"), &[])).1, "quarantine");
    assert_eq!(e.arrive("inbox", &mail("c3@x", "anne@entreprise.fr", Some("dmarc=fail"), &[])).1, "quarantine");
    assert_eq!(count(&e.s, "SELECT auth_seen FROM address WHERE addr = 'anne@entreprise.fr'"), 1, "never reset");
    // The user lets this address in without authentication.
    e.s.conn().execute("UPDATE address SET auth_policy = ?1 WHERE addr = 'anne@entreprise.fr'", [AuthPolicy::NotRequired.as_str()]).unwrap();
    assert_eq!(e.arrive("inbox", &mail("c4@x", "anne@entreprise.fr", Some("dmarc=fail"), &[])).1, "inbox");
    // An account whose server gives no trusted header: unknown, the rules
    // apply as without authentication.
    e.decide("jo@pme.fr", ScreeningTarget::Inbox);
    let (id, role, _) = e.arrive_with("inbox", &mail("n1@x", "jo@pme.fr", Some("dmarc=fail"), &[]), None, Some(1));
    assert_eq!((role.as_str(), e.int("SELECT auth_result = 'unknown' FROM message WHERE id = ?1", id)), ("inbox", 1));
}

#[test]
fn rule_targets_and_restrictive_rules() {
    let mut e = env();
    e.decide("news@letter.example", ScreeningTarget::Newsletter);
    assert_eq!(e.arrive("inbox", &mail("n@x", "news@letter.example", Some("dmarc=pass"), &[])).1, "newsletter");
    e.decide("spam@bad.example", ScreeningTarget::Banned);
    // Banned whatever the verdict, even forged.
    assert_eq!(e.arrive("inbox", &mail("b@x", "spam@bad.example", Some("dmarc=fail"), &[])).1, "banned");
    // A domain rule, for a subdomain too, authenticated only.
    e.s.set_domain_screening("exemple.fr", Some(ScreeningTarget::Paperwork), Source::Cli).unwrap();
    assert_eq!(e.arrive("inbox", &mail("d1@x", "factures@mail.exemple.fr", Some("dmarc=pass"), &[])).1, "paperwork");
    let (id, role, _) = e.arrive("inbox", &mail("d2@x", "factures@mail.exemple.fr", Some("dmarc=none"), &[]));
    assert_eq!((role.as_str(), e.int("SELECT auth_suspect FROM message WHERE id = ?1", id)), ("quarantine", 1));
    // A banned domain applies to its addresses without rule.
    e.s.set_domain_screening("evil.example", Some(ScreeningTarget::Banned), Source::Cli).unwrap();
    assert_eq!(e.arrive("inbox", &mail("e@x", "x@evil.example", None, &[])).1, "banned");
}

#[test]
fn no_from_or_several_is_unknown_and_quarantined() {
    let mut e = env();
    let header = format!("Authentication-Results: {SERVER}; dmarc=pass\r\nMessage-ID: <nf@x>\r\nSubject: none\r\n\r\n");
    let (id, role, _) = e.arrive("inbox", &header);
    assert_eq!((role.as_str(), e.int("SELECT auth_result = 'unknown' FROM message WHERE id = ?1", id)), ("quarantine", 1));
    e.decide("a@x.example", ScreeningTarget::Inbox);
    let (id, role, _) = e.arrive("inbox", &mail("two@x", "a@x.example, b@y.example", Some("dmarc=pass"), &[]));
    assert_eq!((role.as_str(), e.screen_rule(id)), ("quarantine", "no_sender".to_string()));
}

#[test]
fn a_reply_in_a_known_conversation() {
    let mut e = env();
    e.decide("alice@a.example", ScreeningTarget::Inbox);
    let (first, _, _) = e.arrive("inbox", &mail("t1@x", "alice@a.example", Some("dmarc=pass"), &[]));
    // A colleague without decision, in copy of the exchange: let in, no
    // decision taken on them.
    let reply = mail("t2@x", "bob@b.example", Some("dmarc=none"), &["In-Reply-To: <t1@x>", "References: <t1@x>"]);
    let (id, role, _) = e.arrive("inbox", &reply);
    assert_eq!((role.as_str(), e.screen_rule(id)), ("inbox", "conversation".to_string()));
    assert_eq!(count(&e.s, "SELECT count(*) FROM address WHERE addr = 'bob@b.example' AND screening_target IS NOT NULL"), 0);
    // Forged: quarantined, suspect.
    let forged = mail("t3@x", "carol@c.example", Some("dmarc=fail"), &["In-Reply-To: <t1@x>"]);
    let (id, role, _) = e.arrive("inbox", &forged);
    assert_eq!((role.as_str(), e.int("SELECT auth_suspect FROM message WHERE id = ?1", id)), ("quarantine", 1));
    // A banned sender replying in it: banned.
    e.decide("dan@d.example", ScreeningTarget::Banned);
    assert_eq!(e.arrive("inbox", &mail("t4@x", "dan@d.example", None, &["In-Reply-To: <t1@x>"])).1, "banned");
    // A "reply" to a newsletter does not count.
    e.decide("news@n.example", ScreeningTarget::Newsletter);
    e.arrive("inbox", &mail("nl@x", "news@n.example", Some("dmarc=pass"), &[]));
    assert_eq!(e.arrive("inbox", &mail("t5@x", "eve@e.example", None, &["In-Reply-To: <nl@x>"])).1, "quarantine");
    let _ = first;
}

#[test]
fn a_mail_below_the_screening_boundary_is_not_screened() {
    let mut e = env();
    // Mails there before NiceMailer (UID below `uid_screening_start`).
    let (_, role, _) = e.arrive_with("inbox", &mail("old@x", "stranger@far.example", None, &[]), Some(SERVER), Some(1000));
    assert_eq!(role, "inbox");
    // Screening suspended (account being created).
    assert_eq!(e.arrive_with("inbox", &mail("o2@x", "stranger@far.example", None, &[]), Some(SERVER), None).1, "inbox");
    assert_eq!(count(&e.s, "SELECT count(*) FROM audit_log WHERE action = 'screen'"), 0);
}

#[test]
fn a_new_mail_brings_its_conversation_back_from_todo_or_waiting() {
    let mut e = env();
    e.decide("alice@a.example", ScreeningTarget::Inbox);
    let (first, _, _) = e.arrive("inbox", &mail("w1@x", "alice@a.example", Some("dmarc=pass"), &[]));
    let thread: i64 = e.int("SELECT thread_id FROM message WHERE id = ?1", first);
    e.s.move_thread(thread, FolderRole::Waiting, None, MoveOptions::default(), Source::Cli).unwrap();
    assert_eq!(e.role_of(first), "waiting");
    // The answer comes: the conversation is back, marked.
    let (answer, role, _) = e.arrive("inbox", &mail("w2@x", "alice@a.example", Some("dmarc=pass"), &["In-Reply-To: <w1@x>"]));
    assert_eq!(role, "inbox");
    assert_eq!(e.role_of(first), "inbox");
    let mark = |e: &Env, id: i64| e.int("SELECT count(*) FROM message_keyword WHERE message_id = ?1 AND keyword = '$NmWasWaiting'", id);
    assert_eq!((mark(&e, first), mark(&e, answer)), (1, 0), "the old mail marked, not the new one");
    assert_eq!(count(&e.s, "SELECT count(*) FROM pending_action WHERE kind = 'set_flags' AND payload LIKE '%NmWasWaiting%'"), 1, "on the server too");
    // The next command on the conversation removes the mark.
    e.s.move_thread(thread, FolderRole::Archive, None, MoveOptions::default(), Source::Cli).unwrap();
    assert_eq!(mark(&e, first), 0);
}

#[test]
fn what_does_not_bring_a_conversation_back() {
    let mut e = env();
    e.decide("alice@a.example", ScreeningTarget::Inbox);
    let (first, _, _) = e.arrive("inbox", &mail("k1@x", "alice@a.example", Some("dmarc=pass"), &[]));
    let thread: i64 = e.int("SELECT thread_id FROM message WHERE id = ?1", first);
    e.s.move_thread(thread, FolderRole::Todo, None, MoveOptions::default(), Source::Cli).unwrap();
    // A mail screened elsewhere (a forgery in Quarantine).
    e.arrive("inbox", &mail("k2@x", "alice@a.example", Some("dmarc=fail"), &["In-Reply-To: <k1@x>"]));
    assert_eq!(e.role_of(first), "todo");
    // The user's own mail (a copy to themselves).
    let (own, role, _) = e.arrive("inbox", &mail("k3@x", ME, None, &["In-Reply-To: <k1@x>"]));
    assert_eq!((role.as_str(), e.screen_rule(own)), ("inbox", "self".to_string()));
    assert_eq!(e.role_of(first), "todo");
}

#[test]
fn a_mail_found_in_sent_accepts_its_recipients_and_files_its_conversation() {
    let mut e = env();
    // Bob wrote, unknown: in Quarantine.
    let (bobs, _, _) = e.arrive("inbox", &mail("q1@x", "bob@b.example", Some("dmarc=pass"), &[]));
    assert_eq!(e.role_of(bobs), "quarantine");
    e.decide("carol@c.example", ScreeningTarget::Newsletter);
    // Sent from the phone to Bob, Carol and oneself.
    let sent = format!("Message-ID: <s1@x>\r\nFrom: {ME}\r\nTo: bob@b.example, carol@c.example\r\nCc: {ME}\r\nSubject: hi\r\n\r\n");
    e.arrive("sent", &sent);
    assert_eq!(count(&e.s, "SELECT screening_target = 'inbox' FROM address WHERE addr = 'bob@b.example'"), 1);
    let carol: String = e.s.conn().query_row("SELECT screening_target FROM address WHERE addr = 'carol@c.example'", [], |r| r.get(0)).unwrap();
    assert_eq!(carol, "newsletter", "an existing decision is kept");
    assert_eq!(count(&e.s, &format!("SELECT count(*) FROM address WHERE addr = '{ME}' AND screening_target IS NOT NULL")), 0, "not oneself");
    assert_eq!(count(&e.s, "SELECT count(*) FROM audit_log WHERE action = 'screening' AND json_extract(detail, '$.reason') = 'sent_to'"), 1);
    assert_eq!(e.role_of(bobs), "inbox", "Bob's mail leaves the Quarantine");

    // A reply sent elsewhere files its conversation as preselected: from
    // the Inbox, back from Waiting, to Waiting.
    e.decide("alice@a.example", ScreeningTarget::Inbox);
    let (a1, _, _) = e.arrive("inbox", &mail("r1@x", "alice@a.example", Some("dmarc=pass"), &[]));
    let thread: i64 = e.int("SELECT thread_id FROM message WHERE id = ?1", a1);
    e.s.move_thread(thread, FolderRole::Waiting, None, MoveOptions::default(), Source::Cli).unwrap();
    e.arrive("inbox", &mail("r2@x", "alice@a.example", Some("dmarc=pass"), &["In-Reply-To: <r1@x>"]));
    assert_eq!(e.role_of(a1), "inbox");
    let reply = format!("Message-ID: <r3@x>\r\nFrom: {ME}\r\nTo: alice@a.example\r\nIn-Reply-To: <r2@x>\r\nSubject: Re\r\n\r\n");
    e.arrive("sent", &reply);
    assert_eq!(e.role_of(a1), "waiting");
    assert_eq!(count(&e.s, "SELECT count(*) FROM message_keyword WHERE keyword LIKE '$NmWas%'"), 0, "marks removed");
    // A plain reply from the Inbox archives.
    let (b1, _, _) = e.arrive("inbox", &mail("z1@x", "alice@a.example", Some("dmarc=pass"), &[]));
    e.arrive("sent", &format!("Message-ID: <z2@x>\r\nFrom: {ME}\r\nTo: alice@a.example\r\nIn-Reply-To: <z1@x>\r\n\r\n"));
    assert_eq!(e.role_of(b1), "archive");
}

#[test]
fn a_marked_mail_that_left_the_inbox_elsewhere_loses_its_mark() {
    let mut e = env();
    e.decide("alice@a.example", ScreeningTarget::Inbox);
    let (first, _, _) = e.arrive("inbox", &mail("m1@x", "alice@a.example", Some("dmarc=pass"), &[]));
    let thread: i64 = e.int("SELECT thread_id FROM message WHERE id = ?1", first);
    e.s.move_thread(thread, FolderRole::Todo, None, MoveOptions::default(), Source::Cli).unwrap();
    e.arrive("inbox", &mail("m2@x", "alice@a.example", Some("dmarc=pass"), &["In-Reply-To: <m1@x>"]));
    // Pretend every action was sent: the old mail is back in the Inbox at
    // UID 77, marked.
    e.s.conn().execute("UPDATE pending_action SET state = 'done'", []).unwrap();
    e.s.conn().execute("UPDATE message SET uid = 77 WHERE id = ?1", [first]).unwrap();
    // The phone moves it to Archive: it vanished from the Inbox (UID 77),
    // appears in Archive with its keyword.
    let header = mail("m1@x", "alice@a.example", Some("dmarc=pass"), &[]);
    let size: i64 = e.int("SELECT size FROM message WHERE id = ?1", first);
    let a = Arrival {
        uid: 5,
        flags: ServerFlags::parse(&["$NmWasTodo"]),
        date_received: 0,
        size: Some(size as u64),
        modseq: None,
        headers: parse_headers(header.as_bytes()),
    };
    let mut deletions = BTreeMap::from([(folder(&e.s, "inbox"), BTreeSet::from([77]))]);
    let own = e.own.clone();
    let rc = Reception { role: "archive", announce: true, keywords_local: false, authserv_id: Some(SERVER), own: &own, screen_from_uid: None, dry_run: false };
    let archive = folder(&e.s, "archive");
    let (got, _) = e.s.insert_arrivals(&mut e.ins, e.acc, archive, &[a], &mut deletions, &rc).unwrap();
    assert_eq!(got.moved, 1);
    assert_eq!(e.role_of(first), "archive");
    assert_eq!(e.int("SELECT count(*) FROM message_keyword WHERE message_id = ?1", first), 0);
    assert_eq!(count(&e.s, "SELECT count(*) FROM pending_action WHERE state = 'queued' AND payload LIKE '%\"set\":false%'"), 1);
}

#[test]
fn a_read_only_account_is_screened_dry_and_its_mail_cannot_be_changed() {
    let mut e = env();
    e.s.set_server_accounts(BTreeSet::new()).unwrap();
    e.s.set_read_only_accounts(BTreeSet::from([e.acc])).unwrap();
    e.dry_run = true;
    // Where it would go is said; it stays.
    let (id, role, _) = e.arrive("inbox", &mail("ro1@x", "stranger@far.example", Some("dmarc=fail"), &[]));
    assert_eq!(role, "inbox");
    let (folder, dry): (String, bool) = e
        .s
        .conn()
        .query_row("SELECT json_extract(detail, '$.folder'), json_extract(detail, '$.dry_run') FROM audit_log WHERE action = 'screen' AND entity_id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    assert_eq!((folder.as_str(), dry), ("quarantine", true));
    assert_eq!(e.int("SELECT auth_suspect FROM message WHERE id = ?1", id), 0, "nothing marked either");
    assert_eq!(count(&e.s, "SELECT count(*) FROM pending_action"), 0);
    // No command changes its mail, and a decision files none of it.
    let thread = e.int("SELECT thread_id FROM message WHERE id = ?1", id);
    let refused = |r: Result<(), nm_store::StoreError>| assert!(matches!(r, Err(nm_store::StoreError::ReadOnly(_))), "{r:?}");
    refused(e.s.set_thread_seen(thread, true, Source::Cli).map(|_| ()));
    refused(e.s.move_thread(thread, FolderRole::Archive, None, MoveOptions::default(), Source::Cli).map(|_| ()));
    refused(e.s.tag_thread(thread, "Projet", false, Source::Cli).map(|_| ()));
    e.decide("stranger@far.example", ScreeningTarget::Newsletter);
    assert_eq!(e.role_of(id), "inbox");
    // A mail found in Sent changes nothing.
    e.arrive("sent", &format!("Message-ID: <rs@x>\r\nFrom: {ME}\r\nTo: zed@z.example\r\n\r\n"));
    assert_eq!(count(&e.s, "SELECT count(*) FROM address WHERE addr = 'zed@z.example' AND screening_target IS NOT NULL"), 0);
}
