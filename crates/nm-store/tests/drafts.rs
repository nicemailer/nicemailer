//! Replies and their sending, on the database side (`drafts.rs`,
//! `specifications/reponse.md`): the draft prepared from the mail
//! answered, its revisions, the send queued with its undo delay, and what
//! the acceptance by the SMTP server does in one transaction (the mail in
//! Sent, the conversation filed, the recipients accepted, the draft gone).
//! Without a server: the engine's sender is played by the test.

use std::collections::{BTreeMap, BTreeSet};

use nm_core::compose::Mailbox;
use nm_core::{AfterSend, FolderRole, ScreeningTarget, Source};
use nm_store::drafts::{Identities, SendPhase, Sender, Sending};
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
    let roles = ["inbox", "quarantine", "todo", "waiting", "archive", "sent", "newsletter", "drafts", "deleted"];
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
    s.set_outgoing(nm_store::outgoing::OutgoingStore::open(dir.path()).unwrap());
    let ids = Identities { list: vec![Mailbox::new(ME, Some("Me")), Mailbox::new("contact@hey.example", Some("Contact"))] };
    Env { s, raw, _dir: dir, acc, ins: Inserter::new(Source::Imap, None), ids, next_uid: 1 }
}

/// The folder of `role` of the first account (the tests' `perso`).
fn folder(s: &Store, role: &str) -> i64 {
    s.conn().query_row("SELECT id FROM folder WHERE role = ?1 ORDER BY id LIMIT 1", [role], |r| r.get(0)).unwrap()
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

    /// The account of the mail sends.
    fn sending(&self) -> Sending {
        Sending::own_account(self.acc, self.ids.clone())
    }

    fn reply(&mut self, to: i64) -> nm_api::DraftView {
        self.s.create_reply_draft(to, false, &self.sending(), "On Nov 7, Alice wrote:", None, Source::App).unwrap().0
    }

    /// The draft sent and accepted by the server at once.
    fn send_now(&mut self, draft: &nm_api::DraftView, after: Option<AfterSend>) -> nm_store::ChangeSet {
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
    let (again, reused) = e.s.create_reply_draft(m, false, &e.sending(), "ignored", None, Source::App).unwrap();
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
    assert!(matches!(e.s.create_reply_draft(id, false, &e.sending(), "", None, Source::App), Err(StoreError::BodyPending)));
    // A message that does not exist.
    assert!(matches!(e.s.create_reply_draft(12345, false, &e.sending(), "", None, Source::App), Err(StoreError::NotFound(_))));
    // An archive account.
    e.s.conn().execute("UPDATE account SET kind = 'archive' WHERE id = ?1", [e.acc]).unwrap();
    let m = e.deliver("inbox", &alice("m9@a.example"));
    assert!(matches!(e.s.create_reply_draft(m, false, &e.sending(), "", None, Source::App), Err(StoreError::CannotSend(_))));
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
    assert!(matches!(e.s.discard_draft(d.id, Source::App), Err(StoreError::DraftSending)));
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
    let c = e.send_now(&d, Some(AfterSend::Todo));
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

/// `⇧⌘↩` (`reponse.md`, section 8): the filing chosen is saved with the
/// draft, survives a send cancelled with `z`, and wins over the
/// preselection when the server accepts the mail.
#[test]
fn the_filing_chosen_is_kept_and_wins() {
    let mut e = env();
    let m = e.deliver("inbox", &alice("m@a.example"));
    let d = e.reply(m);
    assert_eq!((d.after_send_target, d.after_send), (Some(FolderRole::Archive), None));
    let (_, _) = e.s.send_draft(d.id, d.revision, "Merci", Some(AfterSend::Waiting), 5, "NiceMailer/test", Source::App).unwrap();
    // `z`: the draft comes back with its choice, the preselection unchanged.
    let (v, _) = e.s.cancel_send(d.id, Source::App).unwrap();
    assert_eq!((v.after_send_target, v.after_send), (Some(FolderRole::Archive), Some(AfterSend::Waiting)));
    // `⌘↩` without a choice: the one saved.
    let c = e.send_now(&v, None);
    assert_eq!(e.role_of(m), "waiting");
    assert_eq!(c.sent[0].moved_to, Some(FolderRole::Waiting));
    // A choice made later replaces the earlier one.
    let n = e.deliver("inbox", &alice("n@a.example"));
    let d = e.reply(n);
    e.s.send_draft(d.id, d.revision, "Merci", Some(AfterSend::Todo), 5, "NiceMailer/test", Source::App).unwrap();
    let (v, _) = e.s.cancel_send(d.id, Source::App).unwrap();
    e.send_now(&v, Some(AfterSend::Archive));
    assert_eq!(e.role_of(n), "archive");
}

/// The trial option `stay`: nothing moves, and the conversation keeps its
/// return mark, as if the reply had not been sent.
#[test]
fn stay_leaves_the_conversation_as_it_was() {
    let mut e = env();
    let m = e.deliver("inbox", &alice("m@a.example"));
    e.s.conn().execute("INSERT INTO message_keyword (message_id, keyword) VALUES (?1, '$NmWasTodo')", [m]).unwrap();
    let d = e.reply(m);
    assert_eq!(d.after_send_target, Some(FolderRole::Todo));
    let c = e.send_now(&d, Some(AfterSend::Stay));
    assert_eq!(e.role_of(m), "inbox");
    assert_eq!(c.sent[0].moved_to, None);
    assert_eq!(e.int(&format!("SELECT count(*) FROM message_keyword WHERE message_id = {m}")), 1);
    assert_eq!(e.int("SELECT count(*) FROM audit_log WHERE action = 'move'"), 0);
    // The mail is sent all the same, and its recipients accepted.
    assert_eq!(e.int("SELECT count(*) FROM message m JOIN folder f ON f.id = m.folder_id WHERE f.role = 'sent'"), 1);
    assert_eq!(e.decision("alice@a.example").as_deref(), Some("inbox"));
}

/// The choice never moves a conversation the preselection leaves alone:
/// one in Archive or in a filing folder stays where it is.
#[test]
fn a_choice_does_not_move_a_conversation_outside_the_work_folders() {
    let mut e = env();
    let n = e.deliver("newsletter", &alice("n@a.example"));
    let d = e.reply(n);
    let c = e.send_now(&d, Some(AfterSend::Todo));
    assert_eq!(e.role_of(n), "newsletter");
    assert_eq!(c.sent[0].moved_to, None);
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
    // A refused send saves nothing, not even the filing chosen. (A target
    // outside the four choices cannot be asked: `AfterSend` has none.)
    assert!(e.s.send_draft(d.id, d.revision, "x", Some(AfterSend::Todo), 0, "t", Source::App).is_err());
    assert_eq!(e.s.get_draft(d.id).unwrap().after_send, None);
}

#[test]
fn a_discarded_draft_can_be_restored_with_its_text() {
    let mut e = env();
    let m = e.deliver("inbox", &alice("m1@a.example"));
    let d = e.reply(m);
    e.s.update_draft(d.id, d.revision, "Mon texte", Source::App).unwrap();
    let (gone, _) = e.s.discard_draft(d.id, Source::App).unwrap();
    assert_eq!(gone.body, "Mon texte");
    assert_eq!(e.int("SELECT count(*) FROM draft"), 0);
    assert!(e.s.find_reply_draft(m).unwrap().is_none());
    let (back, reused) = e.s.create_reply_draft(m, false, &e.sending(), "", Some(&gone.body), Source::App).unwrap();
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
    assert!(matches!(e.s.create_reply_draft(m, false, &e.sending(), "", None, Source::App), Err(StoreError::CannotSend(_))));
    e.s.set_fake_sending(true);
    let d = e.reply(m);
    e.send_now(&d, None);
    assert_eq!(e.int("SELECT count(*) FROM message m JOIN folder f ON f.id = m.folder_id WHERE f.role = 'sent'"), 1);
    assert_eq!(e.role_of(m), "archive");
}

// ---------------------------------------------------------------------------
// The copies in Drafts (`draft_copies.rs`)
// ---------------------------------------------------------------------------

impl Env {
    /// Escape: the draft closed, as the daemon does it.
    fn close(&mut self, id: i64) -> nm_store::ChangeSet {
        let Some(mime) = self.s.draft_to_close(id, "NiceMailer/test").unwrap() else { return Default::default() };
        let written = self.raw.write(&mime).unwrap();
        self.s.close_draft(id, &written, &mime, Source::App).unwrap()
    }

    fn drafts_rows(&self) -> i64 {
        self.int("SELECT count(*) FROM message m JOIN folder f ON f.id = m.folder_id WHERE f.role = 'drafts'")
    }

    fn queued(&self, kind: &str) -> i64 {
        self.s.conn().query_row("SELECT count(*) FROM pending_action WHERE kind = ?1 AND state = 'queued'", [kind], |r| r.get(0)).unwrap()
    }
}

#[test]
fn a_closed_draft_is_a_mail_of_drafts_queued_for_the_server() {
    let mut e = env();
    let m = e.deliver("inbox", &alice("m1@a.example"));
    let d = e.reply(m);
    e.s.update_draft(d.id, d.revision, "Merci\n\n> Bonjour", Source::App).unwrap();
    let changes = e.close(d.id);
    assert!(changes.spooled.contains(&e.acc));
    // A complete row of Drafts, read, flagged draft, in the conversation.
    assert_eq!(e.drafts_rows(), 1);
    let row: i64 = e.int("SELECT message_id FROM draft");
    assert_eq!(e.int(&format!("SELECT is_draft + is_seen + body_state FROM message WHERE id = {row}")), 3);
    let thread: i64 = e.int(&format!("SELECT thread_id FROM message WHERE id = {m}"));
    assert_eq!(e.int(&format!("SELECT thread_id FROM message WHERE id = {row}")), thread);
    let text: String = e.s.conn().query_row("SELECT text_plain FROM message_body WHERE message_id = ?1", [row], |r| r.get(0)).unwrap();
    assert!(text.starts_with("Merci"), "{text}");
    assert_eq!(e.queued("append_draft"), 1);
    // Closed again without a change since: nothing more.
    let work = e.s.draft_work_claim(e.acc).unwrap();
    assert_eq!(work.len(), 1);
    let nm_store::draft_copies::DraftWork::Append { action_id, revision, raw_hash, previous_uid, .. } = work[0].clone() else { panic!() };
    assert_eq!(previous_uid, None);
    // Its MIME carries the revision, for the copy on the server.
    let mime = String::from_utf8(e.raw.read(&raw_hash).unwrap()).unwrap();
    assert!(mime.contains(&format!("X-NiceMailer-Draft-Revision: {revision}")), "{mime}");
    e.s.draft_appended(action_id, Some(5)).unwrap();
    assert_eq!(e.int(&format!("SELECT uid FROM message WHERE id = {row}")), 5);
    assert!(e.s.draft_to_close(d.id, "t").unwrap().is_none());
    assert!(e.s.drafts_to_close().unwrap().is_empty());
    // Changed and closed again: a new row, the previous UID carried.
    let v = e.s.get_draft(d.id).unwrap();
    e.s.update_draft(d.id, v.revision, "Merci beaucoup", Source::App).unwrap();
    assert_eq!(e.s.drafts_to_close().unwrap(), vec![d.id]);
    e.close(d.id);
    assert_eq!(e.drafts_rows(), 1);
    let work = e.s.draft_work_claim(e.acc).unwrap();
    assert!(matches!(work[..], [nm_store::draft_copies::DraftWork::Append { previous_uid: Some(5), .. }]), "{work:?}");
}

#[test]
fn closing_twice_before_the_copy_leaves_one_copy_to_make() {
    let mut e = env();
    let m = e.deliver("inbox", &alice("m1@a.example"));
    let d = e.reply(m);
    e.close(d.id);
    let v = e.s.get_draft(d.id).unwrap();
    e.s.update_draft(d.id, v.revision, "Autre", Source::App).unwrap();
    e.close(d.id);
    assert_eq!(e.queued("append_draft"), 1);
    assert_eq!(e.drafts_rows(), 1);
    // `r` on the row of Drafts reopens the same draft.
    let row: i64 = e.int("SELECT message_id FROM draft");
    let (again, reused) = e.s.create_reply_draft(row, false, &e.sending(), "", None, Source::App).unwrap();
    assert!(reused);
    assert_eq!(again.id, d.id);
}

#[test]
fn sent_or_abandoned_the_draft_leaves_drafts_and_the_server() {
    let mut e = env();
    // Abandoned after its copy reached the server: removed there too.
    let m = e.deliver("inbox", &alice("m1@a.example"));
    let d = e.reply(m);
    e.close(d.id);
    let a = e.s.draft_work_claim(e.acc).unwrap()[0].action_id();
    e.s.draft_appended(a, Some(5)).unwrap();
    let (_, changes) = e.s.discard_draft(d.id, Source::App).unwrap();
    assert!(changes.spooled.contains(&e.acc));
    assert_eq!(e.drafts_rows(), 0);
    assert_eq!(e.queued("delete"), 1);
    let work = e.s.draft_work_claim(e.acc).unwrap();
    assert!(matches!(&work[..], [nm_store::draft_copies::DraftWork::Remove { .. }]));
    e.s.draft_work_done(work[0].action_id(), None).unwrap();
    // Abandoned before any copy: nothing to remove on the server, the
    // copy not made is cancelled.
    let m2 = e.deliver("inbox", &alice("m2@a.example"));
    let d2 = e.reply(m2);
    e.close(d2.id);
    e.s.discard_draft(d2.id, Source::App).unwrap();
    assert_eq!(e.queued("append_draft") + e.queued("delete"), 0);
    // Sent: its row goes, its copy too.
    let m3 = e.deliver("inbox", &alice("m3@a.example"));
    let d3 = e.reply(m3);
    e.close(d3.id);
    let a3 = e.s.draft_work_claim(e.acc).unwrap()[0].action_id();
    e.s.draft_appended(a3, Some(6)).unwrap();
    let v = e.s.get_draft(d3.id).unwrap();
    e.send_now(&v, None);
    assert_eq!(e.drafts_rows(), 0);
    assert_eq!(e.queued("delete"), 1);
}

#[test]
fn a_copy_deleted_elsewhere_takes_the_draft_unless_changed_here() {
    let mut e = env();
    let drafts = folder(&e.s, "drafts");
    let state = FolderState { uidvalidity: 8, uidnext: Some(20), highestmodseq: None };
    // Not changed here since its copy: deleted on the phone, deleted here.
    let m = e.deliver("inbox", &alice("m1@a.example"));
    let d = e.reply(m);
    e.close(d.id);
    let a = e.s.draft_work_claim(e.acc).unwrap()[0].action_id();
    e.s.draft_appended(a, Some(5)).unwrap();
    e.s.finish_folder(drafts, &[], &BTreeSet::from([5]), state.clone(), false).unwrap();
    assert_eq!(e.int("SELECT count(*) FROM draft"), 0);
    assert_eq!(e.int("SELECT count(*) FROM audit_log WHERE action = 'draft_removed'"), 1);
    // Changed here since: the text is kept, and goes back at its closing.
    let m2 = e.deliver("inbox", &alice("m2@a.example"));
    let d2 = e.reply(m2);
    e.close(d2.id);
    let a2 = e.s.draft_work_claim(e.acc).unwrap()[0].action_id();
    e.s.draft_appended(a2, Some(6)).unwrap();
    let v = e.s.get_draft(d2.id).unwrap();
    e.s.update_draft(d2.id, v.revision, "Nouveau texte", Source::App).unwrap();
    e.s.finish_folder(drafts, &[], &BTreeSet::from([6]), state, false).unwrap();
    assert_eq!(e.s.get_draft(d2.id).unwrap().body, "Nouveau texte");
    assert_eq!(e.s.drafts_to_close().unwrap(), vec![d2.id]);
    // Sent elsewhere (found in Sent by the engine): the draft goes.
    e.close(d2.id);
    let a3 = e.s.draft_work_claim(e.acc).unwrap()[0].action_id();
    e.s.draft_sent_elsewhere(a3, d2.id).unwrap();
    assert_eq!(e.int("SELECT count(*) FROM draft"), 0);
    assert_eq!(e.drafts_rows(), 0);
}

#[test]
fn an_account_without_server_keeps_its_drafts_here_only() {
    let mut e = env();
    e.s.set_server_accounts(BTreeSet::new()).unwrap();
    let m = e.deliver("inbox", &alice("m1@a.example"));
    let d = e.reply(m);
    e.close(d.id);
    assert_eq!(e.drafts_rows(), 1);
    assert_eq!(e.int("SELECT count(*) FROM pending_action"), 0);
    assert!(e.s.draft_to_close(d.id, "t").unwrap().is_none());
}

#[test]
fn a_failed_copy_is_tried_again_then_given_up() {
    let mut e = env();
    let m = e.deliver("inbox", &alice("m1@a.example"));
    let d = e.reply(m);
    e.close(d.id);
    let a = e.s.draft_work_claim(e.acc).unwrap()[0].action_id();
    e.s.draft_work_retry(a, "connection lost").unwrap();
    assert_eq!(e.queued("append_draft"), 1);
    e.s.conn().execute("UPDATE pending_action SET attempts = 7", []).unwrap();
    e.s.draft_work_retry(a, "connection lost").unwrap();
    assert_eq!(e.int("SELECT count(*) FROM pending_action WHERE state = 'reverted'"), 1);
    // The draft and its row stay here.
    assert_eq!(e.drafts_rows(), 1);
}

fn mb(addr: &str, name: Option<&str>) -> Mailbox {
    Mailbox::new(addr, name)
}

/// The recipients and the subject are edited (`reponse.md`, section 6):
/// one revision for the whole change, none for a change that changes
/// nothing, each address once across the three lists.
#[test]
fn recipients_and_subject_are_edited() {
    use nm_store::drafts::DraftEdit;
    let mut e = env();
    let m = e.deliver("inbox", &alice("m@a.example"));
    let d = e.reply(m);
    let edit = DraftEdit {
        subject: Some("Devis signé\r\n".into()),
        to: Some(vec![mb("Alice@A.example", Some("Alice")), mb("dan@d.example", None)]),
        cc: Some(vec![mb("alice@a.example", None)]),
        bcc: Some(vec![mb("boss@b.example", Some("Boss"))]),
        ..DraftEdit::default()
    };
    let rev = e.s.edit_draft(d.id, d.revision, &edit, Source::App).unwrap();
    assert_eq!(rev, d.revision + 1);
    let v = e.s.get_draft(d.id).unwrap();
    assert_eq!(v.subject, "Devis signé");
    assert_eq!(v.to.iter().map(|a| a.addr.as_str()).collect::<Vec<_>>(), ["alice@a.example", "dan@d.example"]);
    assert!(v.cc.is_empty());
    assert_eq!(v.bcc.iter().map(|a| a.addr.as_str()).collect::<Vec<_>>(), ["boss@b.example"]);
    // The same again: nothing changes, not even the revision.
    assert_eq!(e.s.edit_draft(d.id, rev, &edit, Source::App).unwrap(), rev);
    // Only Cc given: To and Bcc stay.
    let rev = e.s.edit_draft(d.id, rev, &DraftEdit { cc: Some(vec![mb("eve@e.example", None)]), ..Default::default() }, Source::App).unwrap();
    let v = e.s.get_draft(d.id).unwrap();
    assert_eq!((v.to.len(), v.cc.len(), v.bcc.len()), (2, 1, 1));
    // An address that is not one: refused, nothing saved.
    let bad = DraftEdit { to: Some(vec![mb("nope", None)]), body: Some("changed".into()), ..Default::default() };
    assert!(matches!(e.s.edit_draft(d.id, rev, &bad, Source::App), Err(StoreError::InvalidParams(_))));
    assert_eq!(e.s.get_draft(d.id).unwrap().revision, rev);
    // A stale revision: refused.
    assert!(matches!(e.s.edit_draft(d.id, rev - 1, &DraftEdit::default(), Source::App), Err(StoreError::RevisionConflict { .. })));
}

/// The hidden recipients get the mail but never appear in it; the copy the
/// user keeps (raw store, Sent row) lists them, and they get their decision
/// like the others (`reponse.md`, sections 7 and 10).
#[test]
fn bcc_is_in_the_envelope_and_the_copy_only() {
    use nm_store::drafts::DraftEdit;
    let mut e = env();
    let m = e.deliver("inbox", &alice("m@a.example"));
    let d = e.reply(m);
    let edit = DraftEdit { bcc: Some(vec![mb("boss@b.example", Some("Boss"))]), ..DraftEdit::default() };
    let rev = e.s.edit_draft(d.id, d.revision, &edit, Source::App).unwrap();
    e.s.send_draft(d.id, rev, "Merci", None, 0, "NiceMailer/test", Source::App).unwrap();
    let jobs = e.s.send_claim(e.acc).unwrap();
    let j = &jobs[0];
    assert!(j.rcpt.contains(&"boss@b.example".to_string()));
    let sent = String::from_utf8(j.mime.clone()).unwrap();
    assert!(!sent.contains("boss@b.example"), "{sent}");
    let copy = String::from_utf8(j.copy.clone()).unwrap();
    assert!(copy.contains("Bcc: Boss <boss@b.example>\r\n"), "{copy}");
    let written = e.raw.write(&j.copy).unwrap();
    e.s.send_accepted(j.action_id, &written, &[], &e.ids.own()).unwrap();
    assert_eq!(
        e.int("SELECT count(*) FROM message_address ma JOIN address a ON a.id = ma.address_id WHERE ma.role = 'bcc' AND a.addr = 'boss@b.example'"),
        1
    );
    assert_eq!(e.decision("boss@b.example").as_deref(), Some("inbox"));
}

/// Without hidden recipients, the copy is the mail sent: the payload does
/// not carry it twice.
#[test]
fn without_bcc_the_copy_is_the_mail() {
    let mut e = env();
    let m = e.deliver("inbox", &alice("m@a.example"));
    let d = e.reply(m);
    e.s.send_draft(d.id, d.revision, "Merci", None, 0, "NiceMailer/test", Source::App).unwrap();
    assert_eq!(e.int("SELECT count(*) FROM pending_action WHERE json_extract(payload, '$.mime_copy') IS NOT NULL"), 0);
    let jobs = e.s.send_claim(e.acc).unwrap();
    assert_eq!(jobs[0].copy, jobs[0].mime);
}

/// Suggestions (`reponse.md`, section 6): by the start of the address, of
/// its domain, of the name or a word of it; those written to most first;
/// never a banned sender nor one's own address.
#[test]
fn recipients_are_suggested_by_use() {
    let mut e = env();
    // Alice wrote once, Dora three times; I wrote twice to Dan Martin.
    e.deliver("inbox", &alice("m@a.example"));
    for i in 0..3 {
        e.deliver("inbox", &format!("Message-ID: <r{i}@d.example>\r\nFrom: Dora <dora@d.example>\r\nTo: {ME}\r\nSubject: x\r\n\r\nx\r\n"));
    }
    for i in 0..2 {
        e.deliver("sent", &format!("Message-ID: <s{i}@hey.example>\r\nFrom: {ME}\r\nTo: Dan Martin <dan@d.example>\r\nSubject: x\r\n\r\nx\r\n"));
    }
    e.deliver("inbox", "Message-ID: <z@z.example>\r\nFrom: Dark <dark@z.example>\r\nTo: me@hey.example\r\nSubject: x\r\n\r\nx\r\n");
    e.s.conn().execute("UPDATE address SET screening_target = 'banned' WHERE addr = 'dark@z.example'", []).unwrap();
    let names = |q: &str| -> Vec<String> {
        e.s.suggest_recipients(q, 8, &[ME.to_string()]).unwrap().into_iter().map(|a| a.addr).collect()
    };
    // `d`: Dan (written to) before Dora (received from); Dark, banned, never.
    assert_eq!(names("d"), ["dan@d.example", "dora@d.example"]);
    // A word of the name, the domain, case.
    assert_eq!(names("mart"), ["dan@d.example"]);
    assert!(names("A.EXAMPLE").contains(&"alice@a.example".to_string()));
    assert!(names("Ali").contains(&"alice@a.example".to_string()));
    // One's own address, nothing typed, LIKE wildcards taken literally.
    assert!(!names("me").contains(&ME.to_string()));
    assert!(names("  ").is_empty());
    assert!(names("%").is_empty());
}


impl Env {
    /// A file put in `incoming.noindex` by "the app": its staged name.
    fn stage(&self, name: &str, bytes: &[u8]) -> String {
        std::fs::write(self._dir.path().join("incoming.noindex").join(name), bytes).unwrap();
        name.to_string()
    }

    fn outgoing(&self) -> nm_store::outgoing::OutgoingStore {
        nm_store::outgoing::OutgoingStore::open(self._dir.path()).unwrap()
    }
}

/// Attaching (`brouillons.md`, section 7): the file the app staged is
/// taken by the core, kept by its content, listed in the draft; each
/// change is a revision.
#[test]
fn files_are_attached_and_removed() {
    let mut e = env();
    let m = e.deliver("inbox", &alice("m@a.example"));
    let d = e.reply(m);
    let staged = e.stage("f1", b"%PDF-1.4 devis");
    let (rev, a, warning) = e.s.add_draft_attachment(d.id, d.revision, &staged, "/Users/me/Devis mars.pdf", Some("application/pdf"), Source::App).unwrap();
    assert_eq!(rev, d.revision + 1);
    assert_eq!((a.filename.as_str(), a.mime_type.as_str(), a.size), ("Devis mars.pdf", "application/pdf", 14));
    assert!(warning.is_none());
    assert!(!e._dir.path().join("incoming.noindex/f1").exists());
    // The same content again, under another name: one file kept.
    let staged = e.stage("f2", b"%PDF-1.4 devis");
    let (rev, b, _) = e.s.add_draft_attachment(d.id, rev, &staged, "copie.pdf", Some("nonsense"), Source::App).unwrap();
    assert_eq!(b.mime_type, "application/octet-stream");
    assert_eq!(e.outgoing().kept().len(), 1);
    let v = e.s.get_draft(d.id).unwrap();
    assert_eq!(v.attachments.iter().map(|a| a.filename.as_str()).collect::<Vec<_>>(), ["Devis mars.pdf", "copie.pdf"]);
    // Refused: a stale revision, a name that is not a staged file.
    let staged = e.stage("f3", b"x");
    assert!(matches!(e.s.add_draft_attachment(d.id, rev - 1, &staged, "x", None, Source::App), Err(StoreError::RevisionConflict { .. })));
    assert!(matches!(e.s.add_draft_attachment(d.id, rev, "../nicemailer.db", "x", None, Source::App), Err(StoreError::InvalidParams(_))));
    let rev = e.s.remove_draft_attachment(d.id, rev, b.id, Source::App).unwrap();
    assert_eq!(e.s.get_draft(d.id).unwrap().attachments.len(), 1);
    assert!(matches!(e.s.remove_draft_attachment(d.id, rev, b.id, Source::App), Err(StoreError::NotFound(_))));
}

/// A mail with attachments: `multipart/mixed`, the files read back with
/// their names (RFC 2231 for the accented one), the message kept as files
/// rather than in the payload; once sent and copied, the garbage collector
/// removes what nobody needs any more.
#[test]
fn a_mail_with_attachments_is_sent_then_its_files_collected() {
    let mut e = env();
    let m = e.deliver("inbox", &alice("m@a.example"));
    let d = e.reply(m);
    let data: Vec<u8> = (0..=255u8).cycle().take(70_000).collect();
    let staged = e.stage("f1", &data);
    let (rev, _, _) = e.s.add_draft_attachment(d.id, d.revision, &staged, "Résumé été.bin", Some("application/octet-stream"), Source::App).unwrap();
    // A copy for Drafts carries the file.
    let draft_copy = e.s.draft_to_close(d.id, "t").unwrap().unwrap();
    assert_eq!(parse(&draft_copy).attachments.len(), 1);
    e.s.send_draft(d.id, rev, "Ci-joint.", None, 0, "NiceMailer/test", Source::App).unwrap();
    assert_eq!(e.int("SELECT count(*) FROM pending_action WHERE kind = 'send' AND json_extract(payload, '$.mime') IS NULL AND json_extract(payload, '$.mime_blob') IS NOT NULL"), 1);
    let jobs = e.s.send_claim(e.acc).unwrap();
    let j = &jobs[0];
    let parsed = parse(&j.mime);
    assert_eq!(parsed.attachments.len(), 1);
    assert_eq!(parsed.attachments[0].filename.as_deref(), Some("Résumé été.bin"));
    assert_eq!(parsed.attachments[0].size, 70_000);
    assert!(parsed.text.unwrap_or_default().contains("Ci-joint."));
    // While the mail waits, its files stay.
    let store = std::sync::Mutex::new(std::mem::replace(&mut e.s, Store::open_in_memory().unwrap()));
    assert_eq!(nm_store::gc::collect_outgoing(&store, &e.outgoing(), std::time::Duration::from_secs(3600)).unwrap(), 0);
    // The file attached, and the message (sent and kept are the same
    // bytes without Bcc: one file).
    assert_eq!(e.outgoing().kept().len(), 2);
    e.s = store.into_inner().unwrap();
    let written = e.raw.write(&j.copy).unwrap();
    e.s.send_accepted(j.action_id, &written, &[], &e.ids.own()).unwrap();
    assert_eq!(e.int("SELECT has_attachments FROM message m JOIN folder f ON f.id = m.folder_id WHERE f.role = 'sent'"), 1);
    e.s.send_copied(j.action_id, Some(77)).unwrap();
    // Sent: the draft, its attachment and the action's files are gone.
    let store = std::sync::Mutex::new(std::mem::replace(&mut e.s, Store::open_in_memory().unwrap()));
    assert_eq!(nm_store::gc::collect_outgoing(&store, &e.outgoing(), std::time::Duration::from_secs(3600)).unwrap(), 2);
    assert!(e.outgoing().kept().is_empty());
    e.s = store.into_inner().unwrap();
    assert_eq!(e.int("SELECT count(*) FROM outgoing_blob"), 0);
}

/// A file of `outgoing.noindex` without a row (a crash between the file
/// and its row) goes after the grace delay; a file still attached never.
#[test]
fn the_collector_sweeps_orphans_after_the_grace_delay() {
    let mut e = env();
    let m = e.deliver("inbox", &alice("m@a.example"));
    let d = e.reply(m);
    let staged = e.stage("f1", b"attached");
    e.s.add_draft_attachment(d.id, d.revision, &staged, "a.txt", None, Source::App).unwrap();
    e.outgoing().write(b"orphan").unwrap();
    let store = std::sync::Mutex::new(std::mem::replace(&mut e.s, Store::open_in_memory().unwrap()));
    assert_eq!(nm_store::gc::collect_outgoing(&store, &e.outgoing(), std::time::Duration::from_secs(3600)).unwrap(), 0);
    assert_eq!(nm_store::gc::collect_outgoing(&store, &e.outgoing(), std::time::Duration::ZERO).unwrap(), 1);
    assert_eq!(e.outgoing().kept().len(), 1);
}

/// Beyond 25 MB once encoded, a warning; beyond 100 MB of files, refused,
/// and the file taken is not kept.
#[test]
fn big_attachments_are_warned_then_refused() {
    let mut e = env();
    let m = e.deliver("inbox", &alice("m@a.example"));
    let d = e.reply(m);
    let big = |e: &Env, name: &str, bytes: u64| -> String {
        let f = std::fs::File::create(e._dir.path().join("incoming.noindex").join(name)).unwrap();
        // A sparse file: no disk used, read as zeros.
        f.set_len(bytes).unwrap();
        name.to_string()
    };
    let staged = big(&e, "big1", 20 << 20);
    let (rev, _, warning) = e.s.add_draft_attachment(d.id, d.revision, &staged, "a.bin", None, Source::App).unwrap();
    assert!(warning.unwrap().contains("25 MB"));
    let staged = big(&e, "big2", 81 << 20);
    assert!(matches!(e.s.add_draft_attachment(d.id, rev, &staged, "b.bin", None, Source::App), Err(StoreError::InvalidParams(_))));
    assert_eq!(e.outgoing().kept().len(), 1);
    assert_eq!(e.s.get_draft(d.id).unwrap().revision, rev);
}

/// `R` (`reponse.md`, section 4): to the sender alone, no Cc; a reply
/// already started to the mail is given back whatever its kind.
#[test]
fn a_reply_to_the_sender_only() {
    let mut e = env();
    let m = e.deliver("inbox", &alice("m@a.example"));
    let (d, reused) = e.s.create_reply_draft(m, true, &e.sending(), "x", None, Source::App).unwrap();
    assert!(!reused);
    assert_eq!(d.kind, "reply");
    assert_eq!(d.to.iter().map(|a| a.addr.as_str()).collect::<Vec<_>>(), ["alice@a.example"]);
    assert!(d.cc.is_empty());
    let (again, reused) = e.s.create_reply_draft(m, false, &e.sending(), "x", None, Source::App).unwrap();
    assert!(reused);
    assert_eq!(again.id, d.id);
}

/// A mail with a file, as it arrives.
fn with_file(id: &str) -> String {
    format!(
        "Message-ID: <{id}>\r\nFrom: Alice <alice@a.example>\r\nTo: {ME}\r\nSubject: Devis\r\nMIME-Version: 1.0\r\n\
         Content-Type: multipart/mixed; boundary=\"b\"\r\n\r\n--b\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nLe devis.\r\n\
         --b\r\nContent-Type: application/pdf; name=\"devis.pdf\"\r\nContent-Disposition: attachment; filename=\"devis.pdf\"\r\n\
         Content-Transfer-Encoding: base64\r\n\r\nJVBERi0xLjQ=\r\n--b--\r\n"
    )
}

/// `f`: no recipient, `Fwd:`, the client's block then the text as it is,
/// the mail's files taken along; sent, it files nothing and has no
/// `In-Reply-To`.
#[test]
fn a_forward_carries_the_mail_and_its_files() {
    let mut e = env();
    let m = e.deliver("inbox", &with_file("f@a.example"));
    let raw = e.raw.clone();
    let (d, reused, missing) =
        e.s.create_forward_draft(m, &e.sending(), "---------- Forwarded message ---------\nFrom: Alice", None, &raw, Source::App).unwrap();
    assert!(!reused && missing.is_empty());
    assert_eq!((d.kind.as_str(), d.forward_of, d.reply_to, d.thread_id), ("forward", Some(m), None, None));
    assert!(d.to.is_empty() && d.cc.is_empty());
    assert_eq!(d.subject, "Fwd: Devis");
    assert!(d.body.starts_with("\n\n---------- Forwarded message ---------\nFrom: Alice\n\nLe devis."), "{:?}", d.body);
    assert_eq!(d.attachments.len(), 1);
    assert_eq!((d.attachments[0].filename.as_str(), d.attachments[0].size), ("devis.pdf", 8));
    assert_eq!(d.after_send_target, None);
    // `f` again: the same forward.
    let (again, reused, _) = e.s.create_forward_draft(m, &e.sending(), "x", None, &raw, Source::App).unwrap();
    assert!(reused && again.id == d.id);
    use nm_store::drafts::DraftEdit;
    let rev = e.s.edit_draft(d.id, d.revision, &DraftEdit { to: Some(vec![mb("dan@d.example", None)]), ..Default::default() }, Source::App).unwrap();
    // A filing asked for anyway (another client): a forward files
    // nothing, neither the conversation forwarded nor its own copy.
    e.s.send_draft(d.id, rev, &d.body, Some(AfterSend::Waiting), 0, "t", Source::App).unwrap();
    let jobs = e.s.send_claim(e.acc).unwrap();
    let parsed = parse(&jobs[0].mime);
    assert!(parsed.in_reply_to.is_none());
    assert_eq!(parsed.attachments[0].filename.as_deref(), Some("devis.pdf"));
    let written = e.raw.write(&jobs[0].copy).unwrap();
    let c = e.s.send_accepted(jobs[0].action_id, &written, &[], &e.ids.own()).unwrap();
    assert_eq!((c.sent[0].moved_to, c.sent[0].filing), (None, None));
    let sent = c.sent[0].message_id;
    e.s.send_copied(jobs[0].action_id, Some(915)).unwrap();
    assert_eq!(e.role_of(sent), "sent");
    assert_eq!(e.role_of(m), "inbox");
}

/// A forward of a mail whose raw file is not here: the text goes, the
/// files it could not take are named.
#[test]
fn a_forward_without_raw_file_says_which_files_stay_behind() {
    let mut e = env();
    let m = e.deliver("inbox", &with_file("g@a.example"));
    e.s.conn().execute("UPDATE message SET raw_hash = NULL WHERE id = ?1", [m]).unwrap();
    let raw = e.raw.clone();
    let (d, _, missing) = e.s.create_forward_draft(m, &e.sending(), "x", None, &raw, Source::App).unwrap();
    assert_eq!(missing, ["devis.pdf"]);
    assert!(d.attachments.is_empty());
    assert!(d.body.contains("Le devis."));
}

/// `c`: from the main address, empty; left empty, it is dropped rather
/// than kept in Drafts.
#[test]
fn a_new_mail_starts_empty_and_is_dropped_when_left_empty() {
    let mut e = env();
    let d = e.s.create_new_draft(e.acc, &e.ids, None, Source::App).unwrap();
    assert_eq!((d.kind.as_str(), d.from.addr.as_str(), d.subject.as_str(), d.body.as_str()), ("new", ME, "", ""));
    assert!(d.to.is_empty() && d.reply_to.is_none() && d.forward_of.is_none());
    assert!(e.s.drop_if_blank(d.id).unwrap());
    assert!(e.s.get_draft(d.id).is_err());
    // Written in: kept.
    let d = e.s.create_new_draft(e.acc, &e.ids, None, Source::App).unwrap();
    e.s.update_draft(d.id, d.revision, "  \n", Source::App).unwrap();
    assert!(e.s.drop_if_blank(d.id).unwrap());
    let d = e.s.create_new_draft(e.acc, &e.ids, Some("Bonjour"), Source::App).unwrap();
    assert!(!e.s.drop_if_blank(d.id).unwrap());
    // A reply is never dropped, even emptied.
    let m = e.deliver("inbox", &alice("m@a.example"));
    let r = e.reply(m);
    e.s.update_draft(r.id, r.revision, "", Source::App).unwrap();
    assert!(!e.s.drop_if_blank(r.id).unwrap());
}

/// A new mail filed with ⇧⌘↩ (`reponse.md`, section 8, "Nouveau mail"):
/// it answers nothing, so the mail written is what is filed. The move
/// waits for the copy in Sent, whose UID the server must know to follow
/// it: `send_accepted` only announces it (`filing`), `send_copied` does
/// it, and the spool carries the move with the UID of the copy.
#[test]
fn a_new_mail_is_filed_once_its_copy_in_sent_is_made() {
    let mut e = env();
    let d = e.s.create_new_draft(e.acc, &e.ids, None, Source::App).unwrap();
    use nm_store::drafts::DraftEdit;
    let rev = e.s.edit_draft(d.id, d.revision, &DraftEdit { to: Some(vec![mb("dan@d.example", None)]), ..Default::default() }, Source::App).unwrap();
    // Nothing to preselect: the conversation does not exist yet.
    assert_eq!(e.s.get_draft(d.id).unwrap().after_send_target, None);
    e.s.send_draft(d.id, rev, "Bonjour Dan.", Some(AfterSend::Waiting), 0, "NiceMailer/test", Source::App).unwrap();
    // The choice is kept in the draft, like a reply's.
    assert_eq!(e.s.get_draft(d.id).unwrap().after_send, Some(AfterSend::Waiting));
    let jobs = e.s.send_claim(e.acc).unwrap();
    let written = e.raw.write(&jobs[0].copy).unwrap();
    let c = e.s.send_accepted(jobs[0].action_id, &written, &[], &e.ids.own()).unwrap();
    let sent = c.sent[0].message_id;
    // Announced, not done: the copy in Sent does not exist on the server
    // yet. `moved_to`, which `undo` covers, stays empty.
    assert_eq!((c.sent[0].moved_to, c.sent[0].filing), (None, Some(FolderRole::Waiting)));
    assert_eq!(e.role_of(sent), "sent");
    // The copy made, with its UID: the mail leaves Sent for Waiting, and
    // the server is told, from the UID the copy had.
    let c = e.s.send_copied(jobs[0].action_id, Some(910)).unwrap();
    assert_eq!(e.role_of(sent), "waiting");
    assert!(c.messages.iter().any(|(id, _)| *id == sent));
    let (kind, payload): (String, String) = e
        .s
        .conn()
        .query_row("SELECT kind, payload FROM pending_action WHERE message_id = ?1 AND kind = 'move'", [sent], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(kind, "move");
    let moved: serde_json::Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(moved["from_uid"], 910);
    assert_eq!(moved["to_folder_id"], folder(&e.s, "waiting"));
    // The conversation of the mail written is in Waiting, and nothing is
    // left in Sent.
    assert_eq!(e.int("SELECT count(*) FROM message m JOIN folder f ON f.id = m.folder_id WHERE f.role = 'sent'"), 0);
}

/// ⌘↩ without a choice on a new mail, and a forward: nothing is filed
/// (`reponse.md`, section 8). The mail stays in Sent.
#[test]
fn a_new_mail_without_a_choice_stays_in_sent() {
    let mut e = env();
    let d = e.s.create_new_draft(e.acc, &e.ids, None, Source::App).unwrap();
    use nm_store::drafts::DraftEdit;
    let rev = e.s.edit_draft(d.id, d.revision, &DraftEdit { to: Some(vec![mb("dan@d.example", None)]), ..Default::default() }, Source::App).unwrap();
    e.s.send_draft(d.id, rev, "Bonjour.", None, 0, "NiceMailer/test", Source::App).unwrap();
    let jobs = e.s.send_claim(e.acc).unwrap();
    let written = e.raw.write(&jobs[0].copy).unwrap();
    let c = e.s.send_accepted(jobs[0].action_id, &written, &[], &e.ids.own()).unwrap();
    assert_eq!((c.sent[0].moved_to, c.sent[0].filing), (None, None));
    let sent = c.sent[0].message_id;
    e.s.send_copied(jobs[0].action_id, Some(911)).unwrap();
    assert_eq!(e.role_of(sent), "sent");
    assert_eq!(e.int(&format!("SELECT count(*) FROM pending_action WHERE message_id = {sent} AND kind = 'move'")), 0);
    // The trial option `stay` on a new mail: the same, nothing moves.
    let d = e.s.create_new_draft(e.acc, &e.ids, None, Source::App).unwrap();
    let rev = e.s.edit_draft(d.id, d.revision, &DraftEdit { to: Some(vec![mb("eve@e.example", None)]), ..Default::default() }, Source::App).unwrap();
    e.s.send_draft(d.id, rev, "Bonjour.", Some(AfterSend::Stay), 0, "NiceMailer/test", Source::App).unwrap();
    let jobs = e.s.send_claim(e.acc).unwrap();
    let written = e.raw.write(&jobs[0].copy).unwrap();
    let c = e.s.send_accepted(jobs[0].action_id, &written, &[], &e.ids.own()).unwrap();
    assert_eq!((c.sent[0].moved_to, c.sent[0].filing), (None, None));
    let second = c.sent[0].message_id;
    e.s.send_copied(jobs[0].action_id, Some(912)).unwrap();
    assert_eq!(e.role_of(second), "sent");
}

/// A second account, `key`, with the folders of the workflow, synchronized
/// with a server: the account that sends for one that cannot.
fn second_account(e: &mut Env, key: &str) -> i64 {
    e.s.conn().execute("INSERT INTO account (config_key, kind, created_by, modified_by) VALUES (?1, 'imap', 'core', 'core')", [key]).unwrap();
    let acc = e.s.conn().last_insert_rowid();
    let listed: Vec<ListedFolder> = ["inbox", "archive", "sent", "drafts"]
        .iter()
        .enumerate()
        .map(|(i, r)| ListedFolder {
            path: if *r == "inbox" { "INBOX".into() } else { r.to_string() },
            delimiter: Some("/".into()),
            proposed_role: Some(r),
            uidvalidity: Some(100 + i as u32),
        })
        .collect();
    e.s.sync_folder_list(acc, &listed).unwrap();
    e.s.set_server_accounts(BTreeSet::from([e.acc, acc])).unwrap();
    acc
}

/// A mail of an account that cannot send (`reponse.md`, section 4 ter):
/// the reply is sent by another account, by preference one the mail was
/// sent to, from that address; the draft stays in the mail's account and
/// conversation; the mail leaves from the account that sends and is kept
/// in its Sent folder; the conversation answered is filed in its own
/// account.
#[test]
fn a_mail_of_an_account_that_cannot_send_is_answered_from_another() {
    let mut e = env();
    let work = second_account(&mut e, "work");
    let sales = second_account(&mut e, "sales");
    let m = e.deliver("inbox", &alice("m1@a.example"));
    // `perso` cannot send; `work` was not written to, `sales` was (Carol,
    // in Cc, is one of its addresses).
    let sending = Sending {
        own: e.ids.own(),
        senders: vec![
            Sender { account_id: work, identities: Identities { list: vec![Mailbox::new("me@work.example", Some("Me at work"))] } },
            Sender {
                account_id: sales,
                identities: Identities { list: vec![Mailbox::new("team@sales.example", None), Mailbox::new("carol@c.example", Some("Carol"))] },
            },
        ],
    };
    let (d, _) = e.s.create_reply_draft(m, false, &sending, "x", None, Source::App).unwrap();
    assert_eq!((d.account_id, d.send_account_id), (e.acc, sales));
    assert_eq!((d.from.addr.as_str(), d.from.name.as_deref()), ("carol@c.example", Some("Carol")));
    // Neither the mail's account nor the sender's addresses are answered.
    assert_eq!(d.to.iter().map(|a| a.addr.as_str()).collect::<Vec<_>>(), ["alice@a.example"]);
    assert_eq!(d.cc.iter().map(|a| a.addr.as_str()).collect::<Vec<_>>(), ["bob@b.example"]);
    // Filed in the mail's account, as any reply.
    assert_eq!(d.after_send_target, Some(FolderRole::Archive));
    // Sent to none of them: the first account, from its main address.
    let other = e.deliver("inbox", "Message-ID: <o@a>\r\nFrom: a@a.example\r\nTo: list@l.example\r\nSubject: x\r\n\r\nHi\r\n");
    let (o, _) = e.s.create_reply_draft(other, false, &sending, "x", None, Source::App).unwrap();
    assert_eq!((o.send_account_id, o.from.addr.as_str()), (work, "me@work.example"));
    // No account at all: refused.
    let none = Sending { own: e.ids.own(), senders: vec![] };
    let third = e.deliver("inbox", &alice("m3@a.example"));
    assert!(matches!(e.s.create_reply_draft(third, false, &none, "x", None, Source::App), Err(StoreError::CannotSend(_))));

    // Sending: queued on the account that sends, which alone claims it.
    e.s.send_draft(d.id, d.revision, "Merci.", None, 0, "NiceMailer/test", Source::App).unwrap();
    assert_eq!(e.int("SELECT account_id FROM pending_action WHERE kind = 'send'"), sales);
    assert!(e.s.send_claim(e.acc).unwrap().is_empty());
    let jobs = e.s.send_claim(sales).unwrap();
    assert_eq!((jobs.len(), jobs[0].account_id, jobs[0].from.as_str()), (1, sales, "carol@c.example"));
    let written = e.raw.write(&jobs[0].copy).unwrap();
    let c = e.s.send_accepted(jobs[0].action_id, &written, &[], &["carol@c.example".to_string()]).unwrap();
    // In the Sent folder of the account that sent; the conversation
    // answered filed in Archive of its own account.
    let sent = c.sent[0].message_id;
    let (acc, role): (i64, String) = e
        .s
        .conn()
        .query_row("SELECT m.account_id, f.role FROM message m JOIN folder f ON f.id = m.folder_id WHERE m.id = ?1", [sent], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!((acc, role.as_str()), (sales, "sent"));
    assert_eq!(c.sent[0].moved_to, Some(FolderRole::Archive));
    assert_eq!(e.role_of(m), "archive");
    let archive: i64 = e.s.conn().query_row("SELECT folder_id FROM message WHERE id = ?1", [m], |r| r.get(0)).unwrap();
    assert_eq!(e.int(&format!("SELECT account_id FROM folder WHERE id = {archive}")), e.acc);
    assert_eq!(e.int(&format!("SELECT count(*) FROM draft WHERE id = {}", d.id)), 0);
}

/// The sender changed in any draft (`update_draft` with `from`): one
/// revision, none when it is the same; back to the draft's own account,
/// `send_account_id` is NULL again. A send cancelled wakes the account
/// that sends.
#[test]
fn the_sender_of_a_draft_can_be_changed() {
    use nm_store::drafts::DraftEdit;
    let mut e = env();
    let work = second_account(&mut e, "work");
    let m = e.deliver("inbox", &alice("m1@a.example"));
    let d = e.reply(m);
    assert_eq!((d.account_id, d.send_account_id), (e.acc, e.acc));
    let to_work = DraftEdit { from: Some((work, Mailbox::new("me@work.example", Some("Me at work")))), ..Default::default() };
    let rev = e.s.edit_draft(d.id, d.revision, &to_work, Source::App).unwrap();
    assert_eq!(rev, d.revision + 1);
    let v = e.s.get_draft(d.id).unwrap();
    assert_eq!((v.send_account_id, v.from.addr.as_str(), v.from.name.as_deref()), (work, "me@work.example", Some("Me at work")));
    // The same again: nothing changes.
    assert_eq!(e.s.edit_draft(d.id, rev, &to_work, Source::App).unwrap(), rev);
    // The mail sent carries the new From, queued on `work`.
    e.s.send_draft(d.id, rev, "Merci.", None, 30, "t", Source::App).unwrap();
    assert_eq!(e.int("SELECT account_id FROM pending_action WHERE kind = 'send'"), work);
    let (v, changes) = e.s.cancel_send(d.id, Source::App).unwrap();
    assert_eq!(changes.sending, BTreeSet::from([work]));
    // Back to its own account.
    let back = DraftEdit { from: Some((e.acc, Mailbox::new(ME, Some("Me")))), ..Default::default() };
    let rev2 = e.s.edit_draft(d.id, v.revision, &back, Source::App).unwrap();
    assert_eq!(rev2, v.revision + 1);
    assert_eq!(e.int(&format!("SELECT count(*) FROM draft WHERE id = {} AND send_account_id IS NULL AND from_addr = '{ME}'", d.id)), 1);
}

// ---------------------------------------------------------------------------
// The draft proposed before it is written (`reponse.md`, section 5): `r`,
// `R`, `f` and `c` open the screen from a proposal computed on a reading
// connection, so that they never wait for the store's write lock.
// ---------------------------------------------------------------------------

/// Runs `f` with the connection made read-only (`PRAGMA query_only`): any
/// write in it fails, which proves `prepare_*` writes nothing.
fn read_only<T>(e: &Env, f: impl FnOnce(&Store) -> T) -> T {
    e.s.conn().execute_batch("PRAGMA query_only = ON").unwrap();
    let out = f(&e.s);
    e.s.conn().execute_batch("PRAGMA query_only = OFF").unwrap();
    out
}

/// The view of a draft written, with the id and the revision of a
/// proposal: what was proposed must be what is written.
fn as_proposed(mut d: nm_api::DraftView) -> nm_api::DraftView {
    d.id = 0;
    d.revision = 0;
    d
}

#[test]
fn a_reply_is_proposed_without_writing_then_written_as_proposed() {
    let mut e = env();
    let m = e.deliver("inbox", &alice("p1@a.example"));
    let sending = e.sending();
    let (p, reused) = read_only(&e, |s| s.prepare_reply_draft(m, false, &sending, "On Nov 7, Alice wrote:", None, false)).unwrap();
    assert!(!reused);
    assert_eq!((p.id, p.revision, p.state.as_str(), p.kind.as_str()), (0, 0, "editing", "reply"));
    assert_eq!(e.int("SELECT count(*) FROM draft"), 0);
    // Written as it was proposed, the recipients, the subject, the
    // quotation, the filing preselected and the thread included.
    let d = e.reply(m);
    assert_eq!(as_proposed(d.clone()), p);
    // Proposed again: the draft written, reused.
    let (again, reused) = read_only(&e, |s| s.prepare_reply_draft(m, false, &sending, "x", None, false)).unwrap();
    assert!(reused);
    assert_eq!(again, d);
    // `R` proposes the sender alone.
    let m2 = e.deliver("inbox", &alice("p2@a.example"));
    let (p, _) = read_only(&e, |s| s.prepare_reply_draft(m2, true, &sending, "x", None, false)).unwrap();
    assert_eq!(p.to.iter().map(|a| a.addr.as_str()).collect::<Vec<_>>(), ["alice@a.example"]);
    assert!(p.cc.is_empty());
}

/// The text typed in a proposal goes with its first save (`body`): the
/// row written has it, not the quotation.
#[test]
fn the_first_save_writes_the_text_typed() {
    let mut e = env();
    let m = e.deliver("inbox", &alice("p3@a.example"));
    let (d, reused) = e.s.create_reply_draft(m, false, &e.sending(), "x", Some("Merci !\n\n> cité"), Source::App).unwrap();
    assert!(!reused);
    assert_eq!(d.body, "Merci !\n\n> cité");
    assert_eq!(d.subject, "Re: Devis");
}

#[test]
fn a_proposal_is_refused_as_the_draft_would_be() {
    let mut e = env();
    let sending = e.sending();
    assert!(matches!(
        read_only(&e, |s| s.prepare_reply_draft(12345, false, &sending, "", None, false)),
        Err(StoreError::NotFound(_))
    ));
    e.s.conn().execute("UPDATE account SET kind = 'archive' WHERE id = ?1", [e.acc]).unwrap();
    let m = e.deliver("inbox", &alice("p4@a.example"));
    assert!(matches!(read_only(&e, |s| s.prepare_reply_draft(m, false, &sending, "", None, false)), Err(StoreError::CannotSend(_))));
    // The fake network lets an archive account answer, as the writing
    // store does with `set_fake_sending`.
    assert!(read_only(&e, |s| s.prepare_reply_draft(m, false, &sending, "", None, true)).is_ok());
}

#[test]
fn a_forward_is_proposed_with_the_files_it_will_take() {
    let mut e = env();
    let m = e.deliver("inbox", &with_file("pf@a.example"));
    let raw = e.raw.clone();
    let sending = e.sending();
    let (p, reused, missing) = read_only(&e, |s| s.prepare_forward_draft(m, &sending, "Fwd block", None, &raw, false)).unwrap();
    assert!(!reused && missing.is_empty());
    assert_eq!((p.id, p.kind.as_str(), p.subject.as_str()), (0, "forward", "Fwd: Devis"));
    // Listed, not copied: a negative id, nothing in `outgoing.noindex`.
    assert_eq!(p.attachments.len(), 1);
    assert_eq!((p.attachments[0].id, p.attachments[0].filename.as_str()), (-1, "devis.pdf"));
    assert_eq!(e.int("SELECT count(*) FROM outgoing_blob") + e.int("SELECT count(*) FROM draft"), 0);
    // Written: the same text and the same file, copied now.
    let (d, _, _) = e.s.create_forward_draft(m, &e.sending(), "Fwd block", None, &raw, Source::App).unwrap();
    let mut written = as_proposed(d.clone());
    assert_eq!(written.attachments.iter().map(|a| (&a.filename, &a.mime_type)).collect::<Vec<_>>(),
               p.attachments.iter().map(|a| (&a.filename, &a.mime_type)).collect::<Vec<_>>());
    // The size as recorded at reception: the file's own, decoded.
    assert_eq!(p.attachments[0].size, d.attachments[0].size);
    written.attachments = p.attachments.clone();
    assert_eq!(written, p);
    let (again, reused, _) = read_only(&e, |s| s.prepare_forward_draft(m, &sending, "x", None, &raw, false)).unwrap();
    assert!(reused);
    assert_eq!(again, d);
}

#[test]
fn a_forward_proposed_without_raw_file_says_which_files_stay_behind() {
    let mut e = env();
    let m = e.deliver("inbox", &with_file("pg@a.example"));
    e.s.conn().execute("UPDATE message SET raw_hash = NULL WHERE id = ?1", [m]).unwrap();
    let raw = e.raw.clone();
    let sending = e.sending();
    let (p, _, missing) = read_only(&e, |s| s.prepare_forward_draft(m, &sending, "x", None, &raw, false)).unwrap();
    assert_eq!(missing, ["devis.pdf"]);
    assert!(p.attachments.is_empty());
}

#[test]
fn a_new_mail_is_proposed_without_writing() {
    let mut e = env();
    let ids = e.ids.clone();
    let p = read_only(&e, |s| s.prepare_new_draft(e.acc, &ids, None)).unwrap();
    assert_eq!((p.id, p.kind.as_str(), p.from.addr.as_str(), p.body.as_str()), (0, "new", ME, ""));
    assert_eq!(e.int("SELECT count(*) FROM draft"), 0);
    let d = e.s.create_new_draft(e.acc, &e.ids, None, Source::App).unwrap();
    assert_eq!(as_proposed(d), p);
    let none = Identities { list: vec![] };
    assert!(matches!(read_only(&e, |s| s.prepare_new_draft(e.acc, &none, None)), Err(StoreError::CannotSend(_))));
}

// ---------------------------------------------------------------------------
// `del` in Drafts (`dossiers.md`, "Sent et Drafts"): a draft goes to
// Deleted, alone; the conversation it answers stays where it is.
// ---------------------------------------------------------------------------

/// `del` from Drafts on a conversation: `move_thread` to Deleted, given
/// from the folder Drafts.
fn delete_from_drafts(e: &mut Env, thread: i64) -> Result<nm_store::screening::Outcome, StoreError> {
    let drafts = folder(&e.s, "drafts");
    e.s.move_thread(thread, FolderRole::Deleted, Some(drafts), nm_store::screening::MoveOptions::default(), Source::App)
}

fn thread_of(e: &Env, m: i64) -> i64 {
    e.int(&format!("SELECT thread_id FROM message WHERE id = {m}"))
}

/// A draft written by another client (Spark, the phone), in a conversation
/// otherwise archived: as the one of the production mailbox that `del`
/// could not remove.
#[test]
fn a_draft_of_another_client_goes_to_deleted_alone() {
    let mut e = env();
    let a = e.deliver("archive", &alice("pd1@a.example"));
    let foreign = e.deliver(
        "drafts",
        &format!("Message-ID: <x@Spark>\r\nIn-Reply-To: <pd1@a.example>\r\nReferences: <root-pd1@a.example> <pd1@a.example>\r\nFrom: {ME}\r\nTo: alice@a.example\r\nSubject: Re: Devis\r\n\r\nPas fini\r\n"),
    );
    let thread = thread_of(&e, a);
    assert_eq!(thread_of(&e, foreign), thread);
    // Archive is not a target from Drafts: nothing moves.
    let drafts = folder(&e.s, "drafts");
    let refused = e.s.move_thread(thread, FolderRole::Archive, Some(drafts), Default::default(), Source::App);
    assert!(matches!(refused, Err(StoreError::InvalidMove(_))), "{refused:?}");
    let out = delete_from_drafts(&mut e, thread).unwrap();
    assert_eq!(out.moved, 1);
    assert_eq!(e.role_of(foreign), "deleted");
    assert_eq!(e.role_of(a), "archive");
    // On the server too, through the spool.
    assert_eq!(e.queued("move"), 1);
    // `z`: back in Drafts.
    e.s.undo(&out.undo.unwrap(), Source::App).unwrap();
    assert_eq!(e.role_of(foreign), "drafts");
}

/// A draft of NiceMailer, closed and copied: its row of Drafts goes to
/// Deleted with its text, and its row of `draft` goes, else the next
/// closing would bring it back.
#[test]
fn a_draft_of_nicemailer_goes_to_deleted_and_is_forgotten() {
    let mut e = env();
    let m = e.deliver("archive", &alice("pd2@a.example"));
    let d = e.reply(m);
    e.s.update_draft(d.id, d.revision, "Merci", Source::App).unwrap();
    e.close(d.id);
    let row: i64 = e.int("SELECT message_id FROM draft");
    let thread = thread_of(&e, m);
    // Its copy to the server still to make: refused, nothing changes.
    assert!(matches!(delete_from_drafts(&mut e, thread), Err(StoreError::InvalidMove(_))));
    assert_eq!(e.role_of(row), "drafts");
    assert_eq!(e.int("SELECT count(*) FROM draft"), 1);
    // Copied: deleted.
    let work = e.s.draft_work_claim(e.acc).unwrap();
    let nm_store::draft_copies::DraftWork::Append { action_id, .. } = work[0].clone() else { panic!() };
    e.s.draft_appended(action_id, Some(7)).unwrap();
    delete_from_drafts(&mut e, thread).unwrap();
    assert_eq!(e.role_of(row), "deleted");
    assert_eq!(e.role_of(m), "archive");
    assert_eq!(e.int("SELECT count(*) FROM draft"), 0);
    assert!(e.s.drafts_to_close().unwrap().is_empty());
    let text: String = e.s.conn().query_row("SELECT text_plain FROM message_body WHERE message_id = ?1", [row], |r| r.get(0)).unwrap();
    assert!(text.starts_with("Merci"), "{text}");
}

/// A draft being sent stays: `z` cancels the sending first.
#[test]
fn a_draft_being_sent_is_not_deleted() {
    let mut e = env();
    let m = e.deliver("archive", &alice("pd3@a.example"));
    let d = e.reply(m);
    e.close(d.id);
    let work = e.s.draft_work_claim(e.acc).unwrap();
    let nm_store::draft_copies::DraftWork::Append { action_id, .. } = work[0].clone() else { panic!() };
    e.s.draft_appended(action_id, Some(8)).unwrap();
    let v = e.s.get_draft(d.id).unwrap();
    e.s.send_draft(d.id, v.revision, "Merci", None, 30, "t", Source::App).unwrap();
    let row: i64 = e.int("SELECT message_id FROM draft");
    let thread = thread_of(&e, m);
    assert!(matches!(delete_from_drafts(&mut e, thread), Err(StoreError::DraftSending)));
    assert_eq!(e.role_of(row), "drafts");
}
