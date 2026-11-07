//! Replies and their sending, on the database side (`drafts.rs`,
//! `specifications/reponse.md`): the draft prepared from the mail
//! answered, its revisions, the send queued with its undo delay, and what
//! the acceptance by the SMTP server does in one transaction (the mail in
//! Sent, the conversation filed, the recipients accepted, the draft gone).
//! Without a server: the engine's sender is played by the test.

use std::collections::{BTreeMap, BTreeSet};

use nm_core::compose::Mailbox;
use nm_core::{FolderRole, ScreeningTarget, Source};
use nm_store::drafts::{Identities, SendPhase};
use nm_store::ingest::parse;
use nm_store::insert::{Body, Flags, Inserter, NewMessage};
use nm_store::raw::RawStore;
use nm_store::reception::Reception;
use nm_store::sync::{Arrival, FolderState, ListedFolder, ServerFlags};
use nm_store::{Store, StoreError};

const ME: &str = "me@hey.example";

struct Env {
    s: Store,
    raw: RawStore,
    _dir: tempfile::TempDir,
    acc: i64,
    ins: Inserter,
    ids: Identities,
    next_uid: u32,
}

fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let raw = RawStore::open(dir.path()).unwrap();
    let mut s = Store::open_in_memory().unwrap();
    s.conn().execute("INSERT INTO account (config_key, kind, created_by, modified_by) VALUES ('perso', 'imap', 'core', 'core')", []).unwrap();
    let acc = s.conn().last_insert_rowid();
    let roles = ["inbox", "quarantine", "todo", "waiting", "archive", "sent", "newsletter"];
    let listed: Vec<ListedFolder> = roles
        .iter()
        .enumerate()
        .map(|(i, r)| ListedFolder {
            path: if *r == "inbox" { "INBOX".into() } else { r.to_string() },
            delimiter: Some("/".into()),
            proposed_role: Some(r),
            uidvalidity: Some(i as u32 + 1),
        })
        .collect();
    s.sync_folder_list(acc, &listed).unwrap();
    s.set_server_accounts(BTreeSet::from([acc])).unwrap();
    let ids = Identities { list: vec![Mailbox::new(ME, Some("Me")), Mailbox::new("contact@hey.example", Some("Contact"))] };
    Env { s, raw, _dir: dir, acc, ins: Inserter::new(Source::Imap, None), ids, next_uid: 1 }
}

fn folder(s: &Store, role: &str) -> i64 {
    s.conn().query_row("SELECT id FROM folder WHERE role = ?1", [role], |r| r.get(0)).unwrap()
}

impl Env {
    /// A complete mail (headers and body) in the folder of `role`.
    fn deliver(&mut self, role: &str, raw: &str) -> i64 {
        let f = folder(&self.s, role);
        let written = self.raw.write(raw.as_bytes()).unwrap();
        let parsed = parse(raw.as_bytes());
        let uid = self.next_uid;
        self.next_uid += 1;
        let m = NewMessage {
            account_id: self.acc,
            folder_id: f,
            uid: Some(uid),
            date_received: 1_700_000_000 + uid as i64,
            size: Some(written.size),
            modseq: None,
            flags: Flags::default(),
            auth: (nm_core::AuthResult::Pass, Some("server")),
            headers: &parsed,
        };
        let mut changes = Default::default();
        let tx = self.s.transaction().unwrap();
        let id = self.ins.insert(&tx, &m, Some(Body { written: &written, parsed: &parsed }), &mut changes).unwrap();
        self.ins.commit(tx, &mut changes).unwrap();
        id
    }

    fn role_of(&self, id: i64) -> String {
        self.s.conn().query_row("SELECT f.role FROM message m JOIN folder f ON f.id = m.folder_id WHERE m.id = ?1", [id], |r| r.get(0)).unwrap()
    }

    fn int(&self, sql: &str) -> i64 {
        self.s.conn().query_row(sql, [], |r| r.get(0)).unwrap()
    }

    fn decision(&self, addr: &str) -> Option<String> {
        self.s.conn().query_row("SELECT screening_target FROM address WHERE addr = ?1", [addr], |r| r.get(0)).unwrap()
    }

    fn reply(&mut self, to: i64) -> nm_api::DraftView {
        self.s.create_reply_draft(to, &self.ids, "On Nov 7, Alice wrote:", None, Source::App).unwrap().0
    }

    /// The draft sent and accepted by the server at once.
    fn send_now(&mut self, draft: &nm_api::DraftView, after: Option<FolderRole>) -> nm_store::ChangeSet {
        let (_, _) = self.s.send_draft(draft.id, draft.revision, &format!("Merci.{}", draft.body), after, 0, "NiceMailer/test", Source::App).unwrap();
        let jobs = self.s.send_claim(self.acc).unwrap();
        assert_eq!(jobs.len(), 1);
        let written = self.raw.write(&jobs[0].mime).unwrap();
        self.s.send_accepted(jobs[0].action_id, &written, &[], &self.ids.own()).unwrap()
    }
}

/// Alice writes to me, with Bob in To and Carol in Cc, in a conversation
/// of its own (`root-<id>`).
fn alice(id: &str) -> String {
    format!(
        "Message-ID: <{id}>\r\nReferences: <root-{id}>\r\nIn-Reply-To: <root-{id}>\r\nFrom: Alice <alice@a.example>\r\n\
         To: Me <{ME}>, Bob <bob@b.example>\r\nCc: carol@c.example\r\nSubject: Devis\r\nDate: Sat, 07 Nov 2026 10:12:00 +0100\r\n\r\n\
         Bonjour,\r\n\r\n> une ancienne citation\r\nVoici le devis.\r\n"
    )
}

#[test]
fn a_reply_to_all_is_prepared_by_the_core() {
    let mut e = env();
    let m = e.deliver("inbox", &alice("m1@a.example"));
    let d = e.reply(m);
    assert_eq!(d.from.addr, ME);
    assert_eq!(d.from.name.as_deref(), Some("Me"));
    assert_eq!(d.to.iter().map(|a| a.addr.as_str()).collect::<Vec<_>>(), ["alice@a.example"]);
    assert_eq!(d.cc.iter().map(|a| a.addr.as_str()).collect::<Vec<_>>(), ["bob@b.example", "carol@c.example"]);
    assert_eq!(d.subject, "Re: Devis");
    assert_eq!(d.body, "\n\nOn Nov 7, Alice wrote:\n> Bonjour,\n>\n>> une ancienne citation\n> Voici le devis.\n");
    assert_eq!(d.state, "editing");
    assert_eq!(d.reply_to, Some(m));
    assert_eq!(d.after_send_target, Some(FolderRole::Archive));
    assert_eq!(d.send_at, None);
    // `r` again: the same draft, not a second one.
    let (again, reused) = e.s.create_reply_draft(m, &e.ids, "ignored", None, Source::App).unwrap();
    assert!(reused);
    assert_eq!(again.id, d.id);
    assert_eq!(e.int("SELECT count(*) FROM draft"), 1);
    assert_eq!(e.s.find_reply_draft(m).unwrap().map(|d| d.id), Some(d.id));
    // Never sent to the server in v1 (`reponse.md`, section 5).
    assert_eq!(e.int("SELECT count(*) FROM draft WHERE server_revision IS NULL AND message_id IS NULL"), 1);
    assert_eq!(e.int("SELECT count(*) FROM pending_action"), 0);
}

#[test]
fn the_identity_the_mail_was_sent_to_answers() {
    let mut e = env();
    let m = e.deliver("inbox", "Message-ID: <m@a>\r\nFrom: a@a.example\r\nTo: contact@hey.example\r\nSubject: x\r\n\r\nHi\r\n");
    let d = e.reply(m);
    assert_eq!(d.from.addr, "contact@hey.example");
    assert_eq!(d.from.name.as_deref(), Some("Contact"));
}

#[test]
fn what_cannot_be_answered() {
    let mut e = env();
    // A body not downloaded yet.
    let headers = "Message-ID: <h@a>\r\nFrom: a@a.example\r\nTo: me@hey.example\r\nSubject: x\r\n\r\n";
    let parsed = nm_store::ingest::parse_headers(headers.as_bytes());
    let f = folder(&e.s, "inbox");
    let m = NewMessage {
        account_id: e.acc,
        folder_id: f,
        uid: Some(99),
        date_received: 1,
        size: Some(10),
        modseq: None,
        flags: Flags::default(),
        auth: (nm_core::AuthResult::Pass, None),
        headers: &parsed,
    };
    let mut changes = Default::default();
    let tx = e.s.transaction().unwrap();
    let id = e.ins.insert(&tx, &m, None, &mut changes).unwrap();
    e.ins.commit(tx, &mut changes).unwrap();
    assert!(matches!(e.s.create_reply_draft(id, &e.ids, "", None, Source::App), Err(StoreError::BodyPending)));
    // A message that does not exist.
    assert!(matches!(e.s.create_reply_draft(12345, &e.ids, "", None, Source::App), Err(StoreError::NotFound(_))));
    // An archive account.
    e.s.conn().execute("UPDATE account SET kind = 'archive' WHERE id = ?1", [e.acc]).unwrap();
    let m = e.deliver("inbox", &alice("m9@a.example"));
    assert!(matches!(e.s.create_reply_draft(m, &e.ids, "", None, Source::App), Err(StoreError::CannotSend(_))));
}

#[test]
fn revisions_protect_the_text() {
    let mut e = env();
    let m = e.deliver("inbox", &alice("m1@a.example"));
    let d = e.reply(m);
    let r2 = e.s.update_draft(d.id, d.revision, "Merci", Source::App).unwrap();
    assert_eq!(r2, d.revision + 1);
    // The same body again: nothing written, same revision.
    assert_eq!(e.s.update_draft(d.id, r2, "Merci", Source::App).unwrap(), r2);
    // An old revision: refused, the text of the other window is kept.
    assert!(matches!(e.s.update_draft(d.id, d.revision, "Autre", Source::Cli), Err(StoreError::RevisionConflict { current }) if current == r2));
    assert_eq!(e.s.get_draft(d.id).unwrap().body, "Merci");
}

#[test]
fn sending_waits_for_the_undo_delay_and_can_be_cancelled() {
    let mut e = env();
    let m = e.deliver("inbox", &alice("m1@a.example"));
    let d = e.reply(m);
    let (send_at, changes) = e.s.send_draft(d.id, d.revision, "Merci", None, 5, "NiceMailer/test", Source::App).unwrap();
    assert!(changes.sending.contains(&e.acc));
    let v = e.s.get_draft(d.id).unwrap();
    assert_eq!(v.state, "sending");
    assert_eq!(v.body, "Merci");
    assert_eq!(v.send_at, Some(send_at));
    // Frozen while it is being sent.
    assert!(matches!(e.s.update_draft(d.id, v.revision, "x", Source::App), Err(StoreError::DraftSending)));
    assert!(matches!(e.s.discard_draft(d.id), Err(StoreError::DraftSending)));
    // Not due yet: the sender claims nothing.
    assert!(e.s.send_claim(e.acc).unwrap().is_empty());
    assert_eq!(e.s.send_next_due(e.acc).unwrap(), Some(send_at));
    // `z`: back to editing, the text kept.
    let (v, changes) = e.s.cancel_send(d.id, Source::App).unwrap();
    assert!(changes.sending.contains(&e.acc));
    assert_eq!(v.state, "editing");
    assert_eq!(v.body, "Merci");
    assert_eq!(e.int("SELECT count(*) FROM pending_action WHERE state = 'cancelled'"), 1);
    assert_eq!(e.s.send_next_due(e.acc).unwrap(), None);
    // Sent again, and claimed once due.
    e.s.send_draft(d.id, v.revision, "Merci !", None, 0, "NiceMailer/test", Source::App).unwrap();
    let jobs = e.s.send_claim(e.acc).unwrap();
    assert_eq!(jobs.len(), 1);
    let j = &jobs[0];
    assert_eq!(j.phase, SendPhase::Smtp);
    assert_eq!(j.from, ME);
    assert_eq!(j.rcpt, ["alice@a.example", "bob@b.example", "carol@c.example"]);
    let mime = String::from_utf8(j.mime.clone()).unwrap();
    assert!(mime.contains("In-Reply-To: <m1@a.example>\r\n"), "{mime}");
    assert!(mime.contains("References: <root-m1@a.example>\r\n <m1@a.example>\r\n"), "{mime}");
    assert!(mime.contains(&format!("Message-ID: <{}>", j.message_id_hdr)));
    assert!(mime.ends_with("Merci !\r\n"));
    // Claimed: too late to cancel.
    assert!(matches!(e.s.cancel_send(d.id, Source::App), Err(StoreError::SendStarted)));
}

#[test]
fn once_the_server_may_have_it_cancelling_is_refused() {
    let mut e = env();
    let m = e.deliver("inbox", &alice("m1@a.example"));
    let d = e.reply(m);
    e.s.send_draft(d.id, d.revision, "x", None, 0, "t", Source::App).unwrap();
    let j = e.s.send_claim(e.acc).unwrap().remove(0);
    e.s.send_data_started(j.action_id).unwrap();
    // The connection broke after the end of the message: queued again,
    // but the server may have it.
    e.s.send_retry(j.action_id, "connection lost").unwrap();
    assert!(matches!(e.s.cancel_send(d.id, Source::App), Err(StoreError::SendStarted)));
    e.s.conn().execute("UPDATE pending_action SET date_run_after = 0", []).unwrap();
    let j = e.s.send_claim(e.acc).unwrap().remove(0);
    assert!(j.data_sent);
    assert_eq!(j.attempts, 1);
}

#[test]
fn accepted_the_mail_goes_to_sent_and_the_conversation_is_filed() {
    let mut e = env();
    // Bob is already banned; Carol unknown; Alice accepted.
    e.s.set_address_screening("bob@b.example", Some(ScreeningTarget::Banned), Source::Cli).unwrap();
    // A mail of Carol waiting in Quarantine.
    let q = e.deliver("quarantine", "Message-ID: <q@c>\r\nFrom: carol@c.example\r\nTo: me@hey.example\r\nSubject: hello\r\n\r\nhi\r\n");
    let m = e.deliver("inbox", &alice("m1@a.example"));
    let d = e.reply(m);
    let changes = e.send_now(&d, None);
    // The mail is in Sent, complete, without UID until the copy is made.
    let sent: i64 = e.int("SELECT m.id FROM message m JOIN folder f ON f.id = m.folder_id WHERE f.role = 'sent'");
    assert_eq!(e.int(&format!("SELECT body_state FROM message WHERE id = {sent}")), 1);
    assert_eq!(e.int(&format!("SELECT uid IS NULL FROM message WHERE id = {sent}")), 1);
    assert_eq!(e.int(&format!("SELECT is_seen FROM message WHERE id = {sent}")), 1);
    // In the conversation of the mail answered.
    let thread: i64 = e.int(&format!("SELECT thread_id FROM message WHERE id = {m}"));
    assert_eq!(e.int(&format!("SELECT thread_id FROM message WHERE id = {sent}")), thread);
    // Filed as preselected: Inbox → Archive.
    assert_eq!(e.role_of(m), "archive");
    // Carol had no decision: Inbox, and her mail leaves Quarantine; Bob
    // stays banned; my own addresses get nothing.
    assert_eq!(e.decision("carol@c.example").as_deref(), Some("inbox"));
    assert_eq!(e.role_of(q), "inbox");
    assert_eq!(e.decision("bob@b.example").as_deref(), Some("banned"));
    assert_eq!(e.decision(ME), None);
    // The draft is gone; the action goes on with the copy in Sent.
    assert_eq!(e.int("SELECT count(*) FROM draft"), 0);
    assert_eq!(e.int("SELECT count(*) FROM pending_action WHERE kind = 'send' AND state = 'running' AND json_extract(payload, '$.phase') = 'append'"), 1);
    assert_eq!(e.int("SELECT count(*) FROM audit_log WHERE action = 'send'"), 1);
    // `DraftSent`, with the undo token of the filing and the decisions.
    assert_eq!(changes.sent.len(), 1);
    let s = &changes.sent[0];
    assert_eq!((s.draft_id, s.message_id, s.thread_id, s.moved_to), (d.id, sent, Some(thread), Some(FolderRole::Archive)));
    let undo = s.undo.clone().unwrap();
    // Replayed after a crash: nothing twice.
    let j = e.int("SELECT id FROM pending_action WHERE kind = 'send'");
    let written = e.raw.write(b"x").unwrap();
    let again = e.s.send_accepted(j, &written, &[], &e.ids.own()).unwrap();
    assert!(again.sent.is_empty());
    assert_eq!(e.int("SELECT count(*) FROM message m JOIN folder f ON f.id = m.folder_id WHERE f.role = 'sent'"), 1);
    // `z`: the filing and the decision are undone, the mail stays sent.
    e.s.undo(&undo, Source::App).unwrap();
    assert_eq!(e.role_of(m), "inbox");
    assert_eq!(e.decision("carol@c.example"), None);
    assert_eq!(e.role_of(q), "quarantine");
    assert_eq!(e.role_of(sent), "sent");
}

#[test]
fn the_copy_in_sent_gives_the_uid_and_the_synchronization_attaches_without_doubling() {
    let mut e = env();
    let m = e.deliver("inbox", &alice("m1@a.example"));
    let d = e.reply(m);
    e.send_now(&d, None);
    let j = e.int("SELECT id FROM pending_action WHERE kind = 'send'");
    let sent: i64 = e.int("SELECT m.id FROM message m JOIN folder f ON f.id = m.folder_id WHERE f.role = 'sent'");
    let changes = e.s.send_copied(j, Some(42)).unwrap();
    assert!(changes.messages.iter().any(|(id, _)| *id == sent));
    assert_eq!(e.int(&format!("SELECT uid FROM message WHERE id = {sent}")), 42);
    assert_eq!(e.int("SELECT count(*) FROM pending_action WHERE kind = 'send' AND state = 'done'"), 1);

    // A provider that copies itself: no UID; the pass of Sent sees the
    // copy and attaches it to the row, no second row, no second filing.
    let m2 = e.deliver("inbox", &alice("m2@a.example"));
    let d2 = e.reply(m2);
    e.send_now(&d2, None);
    let j2 = e.int("SELECT max(id) FROM pending_action WHERE kind = 'send'");
    e.s.send_copied(j2, None).unwrap();
    let row: i64 = e.int("SELECT max(m.id) FROM message m JOIN folder f ON f.id = m.folder_id WHERE f.role = 'sent'");
    let hdr: String = e.s.conn().query_row("SELECT message_id_hdr FROM message WHERE id = ?1", [row], |r| r.get(0)).unwrap();
    let copy = format!("Message-ID: <{hdr}>\r\nFrom: {ME}\r\nTo: alice@a.example\r\nSubject: Re: Devis\r\n\r\n");
    let a = Arrival { uid: 77, flags: ServerFlags::default(), date_received: 1, size: None, modseq: None, headers: nm_store::ingest::parse_headers(copy.as_bytes()) };
    let own = e.ids.own();
    let rc = Reception { role: "sent", announce: true, keywords_local: false, authserv_id: None, own: &own, screen_from_uid: None, dry_run: false };
    let sent_folder = folder(&e.s, "sent");
    e.s.insert_arrivals(&mut e.ins, e.acc, sent_folder, &[a], &mut BTreeMap::new(), &rc).unwrap();
    assert_eq!(e.int(&format!("SELECT uid FROM message WHERE id = {row}")), 77);
    assert_eq!(e.int("SELECT count(*) FROM message m JOIN folder f ON f.id = m.folder_id WHERE f.role = 'sent'"), 2);
}

#[test]
fn a_fresh_sent_row_without_uid_survives_the_end_of_a_pass() {
    let mut e = env();
    let m = e.deliver("inbox", &alice("m1@a.example"));
    let d = e.reply(m);
    e.send_now(&d, None);
    let j = e.int("SELECT id FROM pending_action WHERE kind = 'send'");
    e.s.send_copied(j, None).unwrap();
    let sent_folder = folder(&e.s, "sent");
    e.s.finish_folder(sent_folder, &[], &BTreeSet::new(), FolderState { uidvalidity: 6, uidnext: Some(10), highestmodseq: None }, false).unwrap();
    assert_eq!(e.int("SELECT count(*) FROM message m JOIN folder f ON f.id = m.folder_id WHERE f.role = 'sent'"), 1);
    // An hour later, still without copy on the server: an orphan, removed
    // like the others (the server's copy, if it comes, is a mail of Sent).
    e.s.conn().execute("UPDATE message SET date_created = date_created - 7200 WHERE folder_id = ?1", [sent_folder]).unwrap();
    e.s.finish_folder(sent_folder, &[], &BTreeSet::new(), FolderState { uidvalidity: 6, uidnext: Some(10), highestmodseq: None }, false).unwrap();
    assert_eq!(e.int("SELECT count(*) FROM message m JOIN folder f ON f.id = m.folder_id WHERE f.role = 'sent'"), 0);
}

#[test]
fn the_filing_follows_the_folder_and_its_marks() {
    let mut e = env();
    // Waiting: a reminder, nothing moves, nothing to undo.
    let w = e.deliver("waiting", &alice("w@a.example"));
    let d = e.reply(w);
    assert_eq!(d.after_send_target, Some(FolderRole::Waiting));
    let c = e.send_now(&d, None);
    assert_eq!(e.role_of(w), "waiting");
    assert_eq!(c.sent[0].moved_to, None);
    // Back in the Inbox from Todo: to Todo, and the mark goes.
    let t = e.deliver("inbox", &alice("t@a.example"));
    e.s.conn().execute("INSERT INTO message_keyword (message_id, keyword) VALUES (?1, '$NmWasTodo')", [t]).unwrap();
    let d = e.reply(t);
    assert_eq!(d.after_send_target, Some(FolderRole::Todo));
    e.send_now(&d, None);
    assert_eq!(e.role_of(t), "todo");
    assert_eq!(e.int(&format!("SELECT count(*) FROM message_keyword WHERE message_id = {t}")), 0);
    // Newsletter: no choice, it stays.
    let n = e.deliver("newsletter", &alice("n@a.example"));
    let d = e.reply(n);
    assert_eq!(d.after_send_target, None);
    e.send_now(&d, None);
    assert_eq!(e.role_of(n), "newsletter");
    // An explicit choice wins over the preselection.
    let i = e.deliver("inbox", &alice("i@a.example"));
    let d = e.reply(i);
    let c = e.send_now(&d, Some(FolderRole::Todo));
    assert_eq!(e.role_of(i), "todo");
    assert_eq!(c.sent[0].moved_to, Some(FolderRole::Todo));
    // The conversation moved while the reply was written: recomputed at
    // sending. Todo → Archive.
    let k = e.deliver("inbox", &alice("k@a.example"));
    let d = e.reply(k);
    let thread: i64 = e.int(&format!("SELECT thread_id FROM message WHERE id = {k}"));
    e.s.move_thread(thread, FolderRole::Todo, None, Default::default(), Source::App).unwrap();
    e.send_now(&d, None);
    assert_eq!(e.role_of(k), "archive");
}

#[test]
fn a_definitive_failure_gives_the_draft_back() {
    let mut e = env();
    let m = e.deliver("inbox", &alice("m1@a.example"));
    let d = e.reply(m);
    e.s.send_draft(d.id, d.revision, "Merci", None, 0, "t", Source::App).unwrap();
    let j = e.s.send_claim(e.acc).unwrap().remove(0);
    // Temporary: later, with the spool's delays.
    e.s.send_retry(j.action_id, "connection refused").unwrap();
    assert_eq!(e.int("SELECT date_run_after - unixepoch() BETWEEN 25 AND 35 FROM pending_action"), 1);
    assert_eq!(e.s.get_draft(d.id).unwrap().state, "sending");
    // Definitive: editable again, `SendFailed`, nothing filed.
    e.s.conn().execute("UPDATE pending_action SET state = 'running'", []).unwrap();
    let c = e.s.send_failed(j.action_id, "550 no such user").unwrap();
    assert_eq!(c.send_failed, vec![(d.id, Some(m), "550 no such user".to_string())]);
    let v = e.s.get_draft(d.id).unwrap();
    assert_eq!((v.state.as_str(), v.body.as_str()), ("editing", "Merci"));
    assert_eq!(e.role_of(m), "inbox");
    // The 8th temporary failure is definitive.
    e.s.send_draft(d.id, v.revision, "Merci", None, 0, "t", Source::App).unwrap();
    let j = e.s.send_claim(e.acc).unwrap().remove(0);
    e.s.conn().execute("UPDATE pending_action SET attempts = 7 WHERE id = ?1", [j.action_id]).unwrap();
    let c = e.s.send_retry(j.action_id, "timeout").unwrap();
    assert_eq!(c.send_failed.len(), 1);
    assert_eq!(e.s.get_draft(d.id).unwrap().state, "editing");
}

#[test]
fn a_failed_copy_does_not_undo_the_sending() {
    let mut e = env();
    let m = e.deliver("inbox", &alice("m1@a.example"));
    let d = e.reply(m);
    e.send_now(&d, None);
    let j = e.int("SELECT id FROM pending_action WHERE kind = 'send'");
    e.s.conn().execute("UPDATE pending_action SET attempts = 7 WHERE id = ?1", [j]).unwrap();
    let c = e.s.send_retry(j, "IMAP unreachable").unwrap();
    assert!(c.send_failed.is_empty());
    assert_eq!(e.int("SELECT count(*) FROM pending_action WHERE state = 'done' AND last_error LIKE 'sent, but not copied%'"), 1);
    assert_eq!(e.int("SELECT count(*) FROM message m JOIN folder f ON f.id = m.folder_id WHERE f.role = 'sent'"), 1);
}

#[test]
fn partial_refusals_are_reported() {
    let mut e = env();
    let m = e.deliver("inbox", &alice("m1@a.example"));
    let d = e.reply(m);
    e.s.send_draft(d.id, d.revision, "x", None, 0, "t", Source::App).unwrap();
    let j = e.s.send_claim(e.acc).unwrap().remove(0);
    let written = e.raw.write(&j.mime).unwrap();
    let c = e.s.send_accepted(j.action_id, &written, &["carol@c.example (550 unknown)".into()], &e.ids.own()).unwrap();
    assert_eq!(c.send_partial, vec![(d.id, vec!["carol@c.example (550 unknown)".to_string()])]);
}

#[test]
fn a_draft_without_recipient_is_not_sent() {
    let mut e = env();
    let m = e.deliver("inbox", &alice("m1@a.example"));
    let d = e.reply(m);
    e.s.conn().execute("UPDATE draft SET recipients = '[]'", []).unwrap();
    assert!(matches!(e.s.send_draft(d.id, d.revision, "x", None, 0, "t", Source::App), Err(StoreError::CannotSend(_))));
    assert_eq!(e.s.get_draft(d.id).unwrap().state, "editing");
    assert!(matches!(
        e.s.send_draft(d.id, d.revision, "x", Some(FolderRole::Inbox), 0, "t", Source::App),
        Err(StoreError::InvalidParams(_))
    ));
}

#[test]
fn a_discarded_draft_can_be_restored_with_its_text() {
    let mut e = env();
    let m = e.deliver("inbox", &alice("m1@a.example"));
    let d = e.reply(m);
    e.s.update_draft(d.id, d.revision, "Mon texte", Source::App).unwrap();
    let gone = e.s.discard_draft(d.id).unwrap();
    assert_eq!(gone.body, "Mon texte");
    assert_eq!(e.int("SELECT count(*) FROM draft"), 0);
    assert!(e.s.find_reply_draft(m).unwrap().is_none());
    let (back, reused) = e.s.create_reply_draft(m, &e.ids, "", Some(&gone.body), Source::App).unwrap();
    assert!(!reused);
    assert_eq!(back.body, "Mon texte");
}

#[test]
fn a_frozen_draft_without_action_is_repaired_at_start() {
    let mut e = env();
    let m = e.deliver("inbox", &alice("m1@a.example"));
    let d = e.reply(m);
    e.s.send_draft(d.id, d.revision, "x", None, 60, "t", Source::App).unwrap();
    assert_eq!(e.s.repair_drafts().unwrap(), 0);
    e.s.conn().execute("DELETE FROM pending_action", []).unwrap();
    assert_eq!(e.s.repair_drafts().unwrap(), 1);
    assert_eq!(e.s.get_draft(d.id).unwrap().state, "editing");
}

#[test]
fn the_time_zone_offset_is_sane() {
    let off = nm_store::drafts::local_offset(1_794_042_720);
    assert!((-14 * 3600..=14 * 3600).contains(&off));
}

#[test]
fn pretended_sending_lets_an_archive_account_answer() {
    let mut e = env();
    e.s.conn().execute("UPDATE account SET kind = 'archive' WHERE id = ?1", [e.acc]).unwrap();
    let m = e.deliver("inbox", &alice("m1@a.example"));
    assert!(matches!(e.s.create_reply_draft(m, &e.ids, "", None, Source::App), Err(StoreError::CannotSend(_))));
    e.s.set_fake_sending(true);
    let d = e.reply(m);
    e.send_now(&d, None);
    assert_eq!(e.int("SELECT count(*) FROM message m JOIN folder f ON f.id = m.folder_id WHERE f.role = 'sent'"), 1);
    assert_eq!(e.role_of(m), "archive");
}
