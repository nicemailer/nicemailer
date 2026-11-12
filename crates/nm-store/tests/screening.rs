//! Integration tests of the folder workflow: the decision a move takes on
//! the sender, the filing of the sender's other mails, authentication,
//! domain decisions and undo (`dossiers.md`, "Déplacement et décision sur
//! l'expéditeur", "Décision sur un expéditeur : rangement des autres
//! mails"; `authentification-expediteur.md`, sections 4 and 5).
//!
//! Each test builds a small mailbox by hand, so that every mail's folder,
//! verdict and flag is known: the seed's random mail would hide the cases.

use nm_core::{AuthPolicy, FolderRole, ScreeningTarget, Source};
use nm_store::screening::{MoveOptions, RuleKind};
use nm_store::seed::{SeedOptions, seed};
use nm_store::rusqlite::{OptionalExtension, params};
use nm_store::{Store, StoreError};

/// The account's own address (`seed.rs`).
const ME: &str = "alex@moreau-conseil.fr";

struct Box {
    store: Store,
    account: i64,
    next_date: i64,
}

/// An empty mailbox: the seed's account and folders, without mail.
fn mailbox() -> Box {
    let mut store = Store::open_in_memory().unwrap();
    seed(&mut store, &SeedOptions { threads: 0, seed: 1, now: 1_790_000_000 }).unwrap();
    let account = store.conn().query_row("SELECT id FROM account", [], |r| r.get(0)).unwrap();
    Box { store, account, next_date: 1_700_000_000 }
}

/// How a test mail is received.
#[derive(Clone, Copy)]
struct M {
    verdict: &'static str,
    suspect: bool,
    flagged: bool,
}

const OK: M = M { verdict: "pass", suspect: false, flagged: false };

impl Box {
    fn folder(&self, account: i64, role: FolderRole) -> i64 {
        self.store
            .conn()
            .query_row("SELECT id FROM folder WHERE account_id = ?1 AND role = ?2", params![account, role.as_str()], |r| {
                r.get(0)
            })
            .unwrap()
    }

    /// Adds a mail from `from` in `role`, to `thread` or to a new thread.
    /// Returns (message id, thread id). Mails are dated in insertion order.
    fn mail_in(&mut self, account: i64, role: FolderRole, from: &str, thread: Option<i64>, m: M) -> (i64, i64) {
        let folder = self.folder(account, role);
        let c = self.store.conn();
        c.execute(
            "INSERT INTO address (addr, created_by, modified_by) VALUES (?1, 'imap', 'imap') ON CONFLICT (addr) DO NOTHING",
            [from],
        )
        .unwrap();
        let address: i64 = c.query_row("SELECT id FROM address WHERE addr = ?1", [from], |r| r.get(0)).unwrap();
        let thread = thread.unwrap_or_else(|| {
            c.execute("INSERT INTO thread (account_id, subject) VALUES (?1, 'sujet')", [account]).unwrap();
            c.last_insert_rowid()
        });
        self.next_date += 60;
        c.execute(
            "INSERT INTO message (account_id, folder_id, thread_id, subject, from_address_id, from_addr, date_received,
                                  is_flagged, auth_result, auth_suspect, created_by, modified_by)
             VALUES (?1, ?2, ?3, 'sujet', ?4, ?5, ?6, ?7, ?8, ?9, 'imap', 'imap')",
            params![account, folder, thread, address, from, self.next_date, m.flagged, m.verdict, m.suspect],
        )
        .unwrap();
        (c.last_insert_rowid(), thread)
    }

    fn mail(&mut self, role: FolderRole, from: &str, m: M) -> (i64, i64) {
        self.mail_in(self.account, role, from, None, m)
    }

    fn role_of(&self, message: i64) -> FolderRole {
        let r: String = self
            .store
            .conn()
            .query_row(
                "SELECT f.role FROM message m JOIN folder f ON f.id = m.folder_id WHERE m.id = ?1",
                [message],
                |r| r.get(0),
            )
            .unwrap();
        FolderRole::parse(&r).unwrap()
    }

    fn decision(&self, addr: &str) -> Option<String> {
        self.store
            .conn()
            .query_row("SELECT screening_target FROM address WHERE addr = ?1", [addr], |r| r.get(0))
            .optional()
            .unwrap()
            .flatten()
    }

    fn policy(&self, addr: &str) -> String {
        self.store.conn().query_row("SELECT auth_policy FROM address WHERE addr = ?1", [addr], |r| r.get(0)).unwrap()
    }

    fn mv(&mut self, thread: i64, from: FolderRole, to: FolderRole) -> nm_store::screening::Outcome {
        let from = self.folder(self.account, from);
        self.store.move_thread(thread, to, Some(from), MoveOptions::default(), Source::App).unwrap()
    }
}

#[test]
fn leaving_quarantine_for_inbox_accepts_the_sender_and_empties_quarantine_only() {
    let mut b = mailbox();
    let (first, thread) = b.mail(FolderRole::Quarantine, "jean@pme.fr", OK);
    let (second, _) = b.mail(FolderRole::Quarantine, "jean@pme.fr", OK);
    let (archived, _) = b.mail(FolderRole::Archive, "jean@pme.fr", OK);
    let (other, _) = b.mail(FolderRole::Quarantine, "paul@pme.fr", OK);

    let out = b.mv(thread, FolderRole::Quarantine, FolderRole::Inbox);
    assert_eq!((out.moved, out.filed), (1, 1));
    let d = out.decision.unwrap();
    assert_eq!((d.kind, d.name.as_str(), d.before, d.after), (RuleKind::Address, "jean@pme.fr", None, Some(ScreeningTarget::Inbox)));
    assert_eq!(b.decision("jean@pme.fr").as_deref(), Some("inbox"));
    assert_eq!(b.role_of(first), FolderRole::Inbox);
    assert_eq!(b.role_of(second), FolderRole::Inbox);
    // Archived mail has been dealt with: it stays.
    assert_eq!(b.role_of(archived), FolderRole::Archive);
    assert_eq!(b.role_of(other), FolderRole::Quarantine);
    // The decision names the mail that triggered it, and the filing the address.
    let trigger: i64 = b
        .store
        .conn()
        .query_row("SELECT detail ->> '$.message_id' FROM audit_log WHERE action = 'screening'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(trigger, first);
    let filed_by: String = b
        .store
        .conn()
        .query_row("SELECT detail ->> '$.address' FROM audit_log WHERE entity_id = ?1 AND action = 'move'", [second], |r| r.get(0))
        .unwrap();
    assert_eq!(filed_by, "jean@pme.fr");
}

#[test]
fn a_newsletter_decision_files_from_every_source_folder_but_not_from_commands() {
    let mut b = mailbox();
    let (_, thread) = b.mail(FolderRole::Inbox, "news@shop.fr", OK);
    let mut at = std::collections::HashMap::new();
    for role in [
        FolderRole::Quarantine,
        FolderRole::Inbox,
        FolderRole::Archive,
        FolderRole::Paperwork,
        FolderRole::Banned,
        FolderRole::Todo,
        FolderRole::Waiting,
        FolderRole::Deleted,
        FolderRole::Junk,
    ] {
        at.insert(role, b.mail(role, "news@shop.fr", OK).0);
    }
    let (flagged_inbox, _) = b.mail(FolderRole::Inbox, "news@shop.fr", M { flagged: true, ..OK });
    let (flagged_archive, _) = b.mail(FolderRole::Archive, "news@shop.fr", M { flagged: true, ..OK });

    let out = b.mv(thread, FolderRole::Inbox, FolderRole::Newsletter);
    assert_eq!(b.decision("news@shop.fr").as_deref(), Some("newsletter"));
    for role in [FolderRole::Quarantine, FolderRole::Inbox, FolderRole::Archive, FolderRole::Paperwork, FolderRole::Banned] {
        assert_eq!(b.role_of(at[&role]), FolderRole::Newsletter, "from {role:?}");
    }
    for role in [FolderRole::Todo, FolderRole::Waiting, FolderRole::Deleted, FolderRole::Junk] {
        assert_eq!(b.role_of(at[&role]), role, "{role:?} must not move");
    }
    // A flag protects a mail of the Inbox only.
    assert_eq!(b.role_of(flagged_inbox), FolderRole::Inbox);
    assert_eq!(b.role_of(flagged_archive), FolderRole::Newsletter);
    assert_eq!(out.filed, 6);
    // Folder counters follow.
    let total: i64 = b
        .store
        .conn()
        .query_row("SELECT total_count FROM folder WHERE id = ?1", [b.folder(b.account, FolderRole::Newsletter)], |r| r.get(0))
        .unwrap();
    assert_eq!(total, 7);
}

#[test]
fn moves_that_do_not_decide_concern_the_thread_only() {
    let mut b = mailbox();
    for (from, to) in [
        (FolderRole::Quarantine, FolderRole::Junk),
        (FolderRole::Quarantine, FolderRole::Deleted),
        (FolderRole::Inbox, FolderRole::Todo),
        (FolderRole::Inbox, FolderRole::Archive),
        (FolderRole::Archive, FolderRole::Inbox),
        (FolderRole::Deleted, FolderRole::Inbox),
    ] {
        let addr = format!("x-{}-{}@spam.biz", from.as_str(), to.as_str());
        let (_, thread) = b.mail(from, &addr, OK);
        let (other, _) = b.mail(from, &addr, OK);
        let out = b.mv(thread, from, to);
        assert!(out.decision.is_none(), "{from:?} -> {to:?}");
        assert_eq!((out.moved, out.filed), (1, 0));
        assert_eq!(b.decision(&addr), None);
        assert_eq!(b.role_of(other), from);
    }
}

#[test]
fn the_decision_is_on_the_latest_sender_never_on_the_user() {
    let mut b = mailbox();
    let (_, thread) = b.mail(FolderRole::Inbox, "boss@client.fr", OK);
    b.mail_in(b.account, FolderRole::Inbox, "colleague@client.fr", Some(thread), OK);
    // The user's own reply, then a copy of it in the Inbox (Cc to self).
    b.mail_in(b.account, FolderRole::Sent, ME, Some(thread), OK);
    b.mail_in(b.account, FolderRole::Inbox, ME, Some(thread), OK);
    let out = b.mv(thread, FolderRole::Inbox, FolderRole::Paperwork);
    assert_eq!(out.decision.unwrap().name, "colleague@client.fr");
    assert_eq!(b.decision("boss@client.fr"), None);
    assert_eq!(b.decision(ME), None);

    // A thread with only the user's mails decides nothing.
    let (_, mine) = b.mail(FolderRole::Inbox, ME, OK);
    assert!(b.mv(mine, FolderRole::Inbox, FolderRole::Newsletter).decision.is_none());
}

#[test]
fn a_suspect_sender_needs_a_confirmation_and_is_never_banned() {
    let mut b = mailbox();
    let forged = M { verdict: "fail", suspect: true, flagged: false };
    let (mail, thread) = b.mail(FolderRole::Quarantine, "support@banque.fr", forged);
    let (other, _) = b.mail(FolderRole::Quarantine, "support@banque.fr", forged);
    let quarantine = b.folder(b.account, FolderRole::Quarantine);

    let err = b.store.move_thread(thread, FolderRole::Inbox, Some(quarantine), MoveOptions::default(), Source::App);
    match err {
        Err(StoreError::ConfirmationRequired { addr, verdict }) => assert_eq!((addr.as_str(), verdict.as_str()), ("support@banque.fr", "fail")),
        other => panic!("{other:?}"),
    }
    // Refused before any write.
    assert_eq!(b.role_of(mail), FolderRole::Quarantine);
    assert_eq!(b.decision("support@banque.fr"), None);
    let audit: i64 = b.store.conn().query_row("SELECT count(*) FROM audit_log", [], |r| r.get(0)).unwrap();
    assert_eq!(audit, 0);

    // Banning on a suspect mail concerns that mail only.
    let out = b.mv(thread, FolderRole::Quarantine, FolderRole::Banned);
    assert!(out.decision.is_none());
    assert_eq!(b.role_of(mail), FolderRole::Banned);
    assert_eq!(b.role_of(other), FolderRole::Quarantine);

    // Confirmed, with the policy for a badly configured tool: the decision
    // is taken and the other failing mail now passes the policy.
    let banned = b.folder(b.account, FolderRole::Banned);
    let opts = MoveOptions { confirm_suspect: true, auth_policy: Some(AuthPolicy::NotRequired) };
    let out = b.store.move_thread(thread, FolderRole::Inbox, Some(banned), opts, Source::App).unwrap();
    assert_eq!(out.decision.unwrap().after, Some(ScreeningTarget::Inbox));
    assert_eq!(b.policy("support@banque.fr"), "not_required");
    assert_eq!(b.role_of(other), FolderRole::Inbox);
}

#[test]
fn a_favorable_decision_files_only_mail_its_policy_accepts() {
    let mut b = mailbox();
    let (_, thread) = b.mail(FolderRole::Quarantine, "anne@asso.org", OK);
    let (unverified, _) = b.mail(FolderRole::Quarantine, "anne@asso.org", M { verdict: "none", ..OK });
    let (failed, _) = b.mail(FolderRole::Quarantine, "anne@asso.org", M { verdict: "fail", ..OK });
    // A pass mail was seen: an unverified one is now suspicious.
    b.store.conn().execute("UPDATE address SET auth_seen = 1 WHERE addr = 'anne@asso.org'", []).unwrap();
    // Mail of an archive account is filed whatever its verdict.
    let archive = b.store.prepare_archive_account("old", "me@old.fr", Source::Cli).unwrap().id;
    let (old, _) = b.mail_in(archive, FolderRole::Quarantine, "anne@asso.org", None, M { verdict: "fail", ..OK });

    let out = b.mv(thread, FolderRole::Quarantine, FolderRole::Inbox);
    assert_eq!(out.filed, 1);
    assert_eq!(b.role_of(unverified), FolderRole::Quarantine);
    assert_eq!(b.role_of(failed), FolderRole::Quarantine);
    // Every account is filed, each in its own folder.
    assert_eq!(b.role_of(old), FolderRole::Inbox);
    let account: i64 = b.store.conn().query_row("SELECT f.account_id FROM message m JOIN folder f ON f.id = m.folder_id WHERE m.id = ?1", [old], |r| r.get(0)).unwrap();
    assert_eq!(account, archive);

    // Banned applies whatever the verdict.
    let (_, spam) = b.mail(FolderRole::Inbox, "pub@promo.biz", OK);
    let (forged, _) = b.mail(FolderRole::Quarantine, "pub@promo.biz", M { verdict: "fail", ..OK });
    b.mv(spam, FolderRole::Inbox, FolderRole::Banned);
    assert_eq!(b.role_of(forged), FolderRole::Banned);
}

#[test]
fn undo_restores_the_decision_and_every_mail() {
    let mut b = mailbox();
    let (mail, thread) = b.mail(FolderRole::Inbox, "news@shop.fr", OK);
    let (archived, _) = b.mail(FolderRole::Archive, "news@shop.fr", OK);
    let (moved_since, _) = b.mail(FolderRole::Quarantine, "news@shop.fr", OK);
    b.store.set_address_screening("news@shop.fr", Some(ScreeningTarget::Paperwork), Source::Cli).unwrap();
    assert_eq!(b.role_of(archived), FolderRole::Paperwork);
    let paperwork = b.role_of(mail);
    assert_eq!(paperwork, FolderRole::Paperwork);

    let out = b.mv(thread, FolderRole::Paperwork, FolderRole::Newsletter);
    let token = out.undo.clone().unwrap();
    assert_eq!(out.decision.unwrap().before, Some(ScreeningTarget::Paperwork));
    // Someone moves one of the filed mails in the meantime: that change wins.
    let moved_since_thread: i64 = b.store.conn().query_row("SELECT thread_id FROM message WHERE id = ?1", [moved_since], |r| r.get(0)).unwrap();
    b.mv(moved_since_thread, FolderRole::Newsletter, FolderRole::Todo);

    let undone = b.store.undo(&token, Source::App).unwrap();
    assert_eq!(b.decision("news@shop.fr").as_deref(), Some("paperwork"));
    assert_eq!(b.role_of(mail), FolderRole::Paperwork);
    assert_eq!(b.role_of(archived), FolderRole::Paperwork);
    assert_eq!(b.role_of(moved_since), FolderRole::Todo);
    assert_eq!(undone.moved, 2);
    assert!(undone.undo.is_some());
    // The undoing is itself in the history.
    let undo_rows: i64 = b
        .store
        .conn()
        .query_row("SELECT count(*) FROM audit_log WHERE detail ->> '$.undo_of' IS NOT NULL", [], |r| r.get(0))
        .unwrap();
    assert_eq!(undo_rows, 3);

    // Undoing again finds nothing in the state the command left.
    let again = b.store.undo(&token, Source::App).unwrap();
    assert_eq!((again.moved, again.undo), (0, None));
    // Malformed tokens are refused.
    for bad in ["", "3", "a:b", "5:2", "0:4"] {
        assert!(matches!(b.store.undo(bad, Source::App), Err(StoreError::InvalidParams(_))), "{bad}");
    }
    assert!(matches!(b.store.undo("900000:900001", Source::App), Err(StoreError::NotFound(_))));
}

#[test]
fn undo_restores_the_policy_chosen_with_a_confirmation() {
    let mut b = mailbox();
    let (_, thread) = b.mail(FolderRole::Quarantine, "crm@pme.fr", M { verdict: "fail", suspect: true, flagged: false });
    let quarantine = b.folder(b.account, FolderRole::Quarantine);
    let opts = MoveOptions { confirm_suspect: true, auth_policy: Some(AuthPolicy::NotRequired) };
    let out = b.store.move_thread(thread, FolderRole::Newsletter, Some(quarantine), opts, Source::App).unwrap();
    b.store.undo(&out.undo.unwrap(), Source::App).unwrap();
    assert_eq!(b.policy("crm@pme.fr"), "auto");
    assert_eq!(b.decision("crm@pme.fr"), None);
}

#[test]
fn setting_an_address_decision_creates_records_and_removes() {
    let mut b = mailbox();
    // Unknown address: created, nothing to file.
    let out = b.store.set_address_screening("New@Example.org", Some(ScreeningTarget::Banned), Source::Cli).unwrap();
    assert_eq!(out.decision.unwrap().name, "new@example.org");
    assert_eq!(b.decision("new@example.org").as_deref(), Some("banned"));

    // Removing moves nothing and keeps a tombstone.
    let (mail, _) = b.mail(FolderRole::Quarantine, "jean@pme.fr", OK);
    b.store.set_address_screening("jean@pme.fr", Some(ScreeningTarget::Inbox), Source::Cli).unwrap();
    assert_eq!(b.role_of(mail), FolderRole::Inbox);
    let out = b.store.set_address_screening("jean@pme.fr", None, Source::Cli).unwrap();
    assert_eq!((out.filed, out.decision.unwrap().before), (0, Some(ScreeningTarget::Inbox)));
    assert_eq!(b.role_of(mail), FolderRole::Inbox);
    let date: Option<i64> = b.store.conn().query_row("SELECT date_screening_modified FROM address WHERE addr = 'jean@pme.fr'", [], |r| r.get(0)).unwrap();
    assert!(date.is_some());

    // The same decision twice writes nothing.
    let out = b.store.set_address_screening("new@example.org", Some(ScreeningTarget::Banned), Source::Cli).unwrap();
    assert!(out.undo.is_none());

    // A decision received from another Mac files nothing: that Mac did.
    let (q, _) = b.mail(FolderRole::Quarantine, "synced@pme.fr", OK);
    b.store.set_address_screening("synced@pme.fr", Some(ScreeningTarget::Inbox), Source::ConfigSync).unwrap();
    assert_eq!(b.role_of(q), FolderRole::Quarantine);

    assert!(matches!(b.store.set_address_screening("nobody", None, Source::Cli), Err(StoreError::InvalidParams(_))));
}

#[test]
fn a_domain_decision_files_the_addresses_without_their_own() {
    let mut b = mailbox();
    let (plain, _) = b.mail(FolderRole::Quarantine, "a@shop.fr", OK);
    let (sub, _) = b.mail(FolderRole::Quarantine, "b@news.shop.fr", OK);
    let (unverified, _) = b.mail(FolderRole::Quarantine, "c@shop.fr", M { verdict: "none", ..OK });
    let (own, _) = b.mail(FolderRole::Quarantine, "d@shop.fr", OK);
    let (lookalike, _) = b.mail(FolderRole::Quarantine, "e@myshop.fr", OK);
    b.store.set_address_screening("d@shop.fr", Some(ScreeningTarget::Banned), Source::Cli).unwrap();

    let out = b.store.set_domain_screening("@Shop.fr", Some(ScreeningTarget::Newsletter), Source::Cli).unwrap();
    assert_eq!(out.decision.as_ref().unwrap().kind, RuleKind::Domain);
    assert_eq!(out.filed, 2);
    assert_eq!(b.role_of(plain), FolderRole::Newsletter);
    assert_eq!(b.role_of(sub), FolderRole::Newsletter);
    // A favorable domain rule wants `pass`.
    assert_eq!(b.role_of(unverified), FolderRole::Quarantine);
    // The address rule wins.
    assert_eq!(b.role_of(own), FolderRole::Banned);
    assert_eq!(b.role_of(lookalike), FolderRole::Quarantine);

    // The contacts show the domain's decision next to the address's.
    let (contacts, _) = b.store.list_contacts(Some("news.shop.fr"), None, 10).unwrap();
    assert_eq!(contacts[0].screening_target, None);
    assert_eq!(contacts[0].domain_target.as_deref(), Some("newsletter"));
    assert_eq!(contacts[0].auth_policy, "auto");

    // History names the domain.
    let (history, _) = b.store.list_history(None, 50).unwrap();
    assert!(history.iter().any(|h| h.entity == "domain" && h.summary == "shop.fr → newsletter"), "{history:?}");

    assert!(b.store.set_domain_screening("a@b.fr", None, Source::Cli).is_err());
    assert!(b.store.set_domain_screening("localhost", None, Source::Cli).is_err());
}

// ---------------------------------------------------------------------------
// Grouped commands (`ui.md`, "Actions groupées"): several threads in one
// transaction, one batch of events, one undo token.
// ---------------------------------------------------------------------------

#[test]
fn several_threads_are_moved_in_one_transaction_and_undone_at_once() {
    let mut b = mailbox();
    let inbox = b.folder(b.account, FolderRole::Inbox);
    let quarantine = b.folder(b.account, FolderRole::Quarantine);
    let (m1, t1) = b.mail(FolderRole::Inbox, "a@one.fr", OK);
    let (m2, t2) = b.mail(FolderRole::Inbox, "b@two.fr", OK);
    let (m3, t3) = b.mail(FolderRole::Inbox, "news@shop.fr", OK);
    // A forged sender in Quarantine, and a sent mail (no target from Sent).
    let forged = M { verdict: "fail", suspect: true, flagged: false };
    let (m4, t4) = b.mail(FolderRole::Quarantine, "support@banque.fr", forged);
    let (_, t5) = b.mail(FolderRole::Sent, ME, OK);
    let sent = b.folder(b.account, FolderRole::Sent);
    let audit_before: i64 = b.store.conn().query_row("SELECT count(*) FROM audit_log", [], |r| r.get(0)).unwrap();

    // To Newsletter: every thread decides on its sender.
    let batch = b
        .store
        .move_threads(
            &[(t1, Some(inbox)), (t2, Some(inbox)), (t3, Some(inbox)), (t4, Some(quarantine)), (t5, Some(sent)), (999_999, None)],
            FolderRole::Newsletter,
            MoveOptions::default(),
            Source::App,
        )
        .unwrap();
    // Moved together; the forged sender left for a confirmation, the sent
    // mail refused, the unknown thread skipped.
    assert_eq!(batch.moved_threads, vec![t1, t2, t3]);
    assert_eq!(batch.refused, vec![t5]);
    assert_eq!(batch.suspects.iter().map(|s| (s.thread_id, s.addr.as_str())).collect::<Vec<_>>(), [(t4, "support@banque.fr")]);
    assert_eq!(batch.decisions.len(), 3);
    for m in [m1, m2, m3] {
        assert_eq!(b.role_of(m), FolderRole::Newsletter);
    }
    assert_eq!(b.role_of(m4), FolderRole::Quarantine);
    // One batch of changes, naming the three threads.
    let threads: Vec<i64> = batch.outcome.changes.threads.iter().map(|(t, _)| *t).collect();
    assert!([t1, t2, t3].iter().all(|t| threads.contains(t)));
    assert!(audit_before < b.store.conn().query_row::<i64, _, _>("SELECT count(*) FROM audit_log", [], |r| r.get(0)).unwrap());

    // One token undoes the whole batch, decisions included.
    b.store.undo(&batch.outcome.undo.unwrap(), Source::App).unwrap();
    for m in [m1, m2, m3] {
        assert_eq!(b.role_of(m), FolderRole::Inbox);
    }
    for addr in ["a@one.fr", "b@two.fr", "news@shop.fr"] {
        assert_eq!(b.decision(addr), None, "{addr}");
    }

    // The suspect thread, confirmed, moves alone.
    let confirmed = MoveOptions { confirm_suspect: true, ..MoveOptions::default() };
    let again = b.store.move_threads(&[(t4, Some(quarantine))], FolderRole::Inbox, confirmed, Source::App).unwrap();
    assert_eq!(again.moved_threads, vec![t4]);
    assert_eq!(b.role_of(m4), FolderRole::Inbox);
}

#[test]
fn several_threads_are_marked_read_flagged_and_tagged_in_one_transaction() {
    let mut b = mailbox();
    let threads: Vec<i64> = (0..5).map(|i| b.mail(FolderRole::Inbox, &format!("p{i}@x.fr"), OK).1).collect();
    for t in &threads {
        b.store.refresh_thread_aggregates(*t).unwrap();
    }
    let mut ids = threads.clone();
    ids.push(999_999); // gone meanwhile: skipped

    let changes = b.store.set_threads_seen(&ids, true, Source::App).unwrap();
    assert_eq!(changes.changed_messages(), 5);
    assert_eq!(changes.threads.len(), 5);
    let unread: i64 = b.store.conn().query_row("SELECT sum(unread_count) FROM thread", [], |r| r.get(0)).unwrap();
    assert_eq!(unread, 0);
    // Again: nothing to change, nothing reported.
    assert_eq!(b.store.set_threads_seen(&ids, true, Source::App).unwrap().changed_messages(), 0);

    let changes = b.store.set_threads_flagged(&ids, true, Source::App).unwrap();
    assert_eq!(changes.changed_messages(), 5);
    let flagged: i64 = b.store.conn().query_row("SELECT count(*) FROM message WHERE is_flagged = 1", [], |r| r.get(0)).unwrap();
    assert_eq!(flagged, 5);

    let changes = b.store.tag_threads(&ids, "Clients", false, Source::App).unwrap();
    assert_eq!(changes.changed_messages(), 5);
    let tagged: i64 = b.store.conn().query_row("SELECT count(*) FROM message_keyword", [], |r| r.get(0)).unwrap();
    assert_eq!(tagged, 5);
    // One tag created for all of them.
    let tags: i64 = b.store.conn().query_row("SELECT count(*) FROM tag WHERE name = 'Clients'", [], |r| r.get(0)).unwrap();
    assert_eq!(tags, 1);
}
