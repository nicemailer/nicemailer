//! People and robots (`specifications/expediteurs.md`): the verdict on an
//! address drawn from the clues of its mails, its copy in every mail and in
//! the threads, the verdict forced by the user and its undo, the list
//! filter, the contacts, and the catch-up of the mails stored before
//! version 12, which must give exactly what the insertion gives.

use std::sync::Mutex;

use nm_api::{ThreadFilter, ThreadSort};
use nm_core::{SenderKind, Source};
use nm_store::ChangeSet;
use nm_store::ingest::parse;
use nm_store::insert::{Body, Flags, Inserter, NewMessage};
use nm_store::raw::RawStore;
use nm_store::{Store, rusqlite};

struct Env {
    store: Store,
    raw: RawStore,
    account: i64,
    inbox: i64,
    archive: i64,
    sent: i64,
    next_uid: u32,
    _dir: tempfile::TempDir,
}

impl Env {
    fn new() -> Env {
        let dir = tempfile::tempdir().unwrap();
        let raw = RawStore::open(dir.path()).unwrap();
        let store = Store::open_in_memory().unwrap();
        let c = store.conn();
        c.execute(
            "INSERT INTO account (config_key, kind, address, created_by, modified_by)
             VALUES ('perso', 'imap', 'me@home.fr', 'core', 'core')",
            [],
        )
        .unwrap();
        let account = c.last_insert_rowid();
        let folder = |path: &str, role: &str| {
            c.execute(
                "INSERT INTO folder (account_id, path, delimiter, role, created_by, modified_by) VALUES (?1, ?2, '/', ?3, 'core', 'core')",
                rusqlite::params![account, path, role],
            )
            .unwrap();
            c.last_insert_rowid()
        };
        let inbox = folder("INBOX", "inbox");
        let archive = folder("Archive", "archive");
        let sent = folder("Sent", "sent");
        Env { store, raw, account, inbox, archive, sent, next_uid: 1, _dir: dir }
    }

    /// Inserts whole messages in `folder`, in one transaction; returns
    /// their ids. Each one is stored in the raw store, so that the
    /// catch-up can read it again.
    fn insert(&mut self, folder: i64, mails: &[String]) -> Vec<i64> {
        let mut ins = Inserter::new(Source::Imap, None);
        let mut changes = ChangeSet::default();
        let mut ids = Vec::new();
        let tx = self.store.transaction().unwrap();
        for raw in mails {
            let written = self.raw.write(raw.as_bytes()).unwrap();
            let parsed = parse(raw.as_bytes());
            let uid = self.next_uid;
            self.next_uid += 1;
            let m = NewMessage {
                account_id: self.account,
                folder_id: folder,
                uid: Some(uid),
                date_received: 1_700_000_000 + i64::from(uid),
                size: Some(raw.len() as u64),
                modseq: None,
                flags: Flags::default(),
                auth: (nm_core::AuthResult::Unknown, None),
                headers: &parsed,
            };
            ids.push(ins.insert(&tx, &m, Some(Body { written: &written, parsed: &parsed }), &mut changes).unwrap());
        }
        ins.commit(tx, &mut changes).unwrap();
        ids
    }

    fn one(&mut self, folder: i64, mail: String) -> i64 {
        self.insert(folder, &[mail])[0]
    }

    fn address(&self, addr: &str) -> i64 {
        self.store.conn().query_row("SELECT id FROM address WHERE addr = ?1", [addr], |r| r.get(0)).unwrap()
    }

    fn address_kind(&self, addr: &str) -> Option<String> {
        self.store
            .conn()
            .query_row("SELECT coalesce(sender_kind_user, sender_kind) FROM address WHERE addr = ?1", [addr], |r| r.get(0))
            .unwrap()
    }

    fn kind(&self, message: i64) -> Option<String> {
        self.store.conn().query_row("SELECT sender_kind FROM message WHERE id = ?1", [message], |r| r.get(0)).unwrap()
    }

    fn thread_has_person(&self, message: i64) -> bool {
        self.store
            .conn()
            .query_row(
                "SELECT t.has_person FROM thread t JOIN message m ON m.thread_id = t.id WHERE m.id = ?1",
                [message],
                |r| r.get(0),
            )
            .unwrap()
    }

    /// Subjects of the threads of `folder` under `filter`.
    fn subjects(&self, folder: i64, filter: &ThreadFilter) -> Vec<String> {
        let (rows, _) = self.store.list_threads_in(&[folder], None, 50, ThreadSort::default(), filter, &[]).unwrap();
        let mut s: Vec<String> = rows.into_iter().map(|t| t.subject).collect();
        s.sort();
        s
    }
}

/// A mail with `extra` header lines (each ending with CRLF).
fn mail(from: &str, to: &str, subject: &str, extra: &str) -> String {
    let id = format!("{}-{}@test", subject.replace(' ', "-"), from.replace('@', "."));
    format!("From: {from}\r\nTo: {to}\r\nSubject: {subject}\r\nMessage-ID: <{id}>\r\n{extra}\r\nbody\r\n")
}

const NEWSLETTER: &str = "List-Unsubscribe: <https://shop.example/u>\r\nX-Mailer: Sendinblue\r\n";
const APPLE_MAIL: &str = "X-Mailer: Apple Mail (2.3445.104.11)\r\n";

#[test]
fn a_newsletter_is_a_robot_and_a_person_writing_with_a_mail_client_is_a_person() {
    let mut e = Env::new();
    let news = e.one(e.inbox, mail("hello@shop.example", "me@home.fr", "Soldes", NEWSLETTER));
    let alice = e.one(e.inbox, mail("alice@acme.fr", "me@home.fr", "Dîner", APPLE_MAIL));
    assert_eq!(e.address_kind("hello@shop.example").as_deref(), Some("robot"));
    assert_eq!(e.kind(news).as_deref(), Some("robot"));
    assert!(!e.thread_has_person(news));
    assert_eq!(e.address_kind("alice@acme.fr").as_deref(), Some("person"));
    assert_eq!(e.kind(alice).as_deref(), Some("person"));
    assert!(e.thread_has_person(alice));
    // The counts behind the verdicts.
    let counts: (i64, i64, i64) = e
        .store
        .conn()
        .query_row("SELECT sender_count, sender_bulk, sender_client FROM address WHERE addr = 'alice@acme.fr'", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .unwrap();
    assert_eq!(counts, (1, 0, 1));
}

#[test]
fn writing_to_an_address_makes_it_a_person_and_rewrites_its_mails() {
    let mut e = Env::new();
    // No clue at all, never written to: a machine (rule 10).
    let first = e.one(e.archive, mail("j.martin@acme.fr", "me@home.fr", "Devis", ""));
    assert_eq!(e.address_kind("j.martin@acme.fr").as_deref(), Some("robot"));
    assert!(!e.thread_has_person(first));
    // The user writes to them: a person (rule 5), and their mail with it.
    let reply = e.one(e.sent, mail("me@home.fr", "j.martin@acme.fr", "Re Devis", ""));
    let written: bool = e.store.conn().query_row("SELECT written_to FROM address WHERE addr = 'j.martin@acme.fr'", [], |r| r.get(0)).unwrap();
    assert!(written);
    assert_eq!(e.address_kind("j.martin@acme.fr").as_deref(), Some("person"));
    assert_eq!(e.kind(first).as_deref(), Some("person"));
    assert!(e.thread_has_person(first));
    // The user's own mail: a person, never counted on their address.
    assert_eq!(e.kind(reply).as_deref(), Some("person"));
    let counted: i64 = e.store.conn().query_row("SELECT sender_count FROM address WHERE addr = 'me@home.fr'", [], |r| r.get(0)).unwrap();
    assert_eq!(counted, 0);
}

#[test]
fn a_mixed_address_is_a_person_but_its_newsletter_is_a_robot() {
    let mut e = Env::new();
    e.one(e.sent, mail("me@home.fr", "galerie@art.example", "Visite", ""));
    let personal = e.one(e.archive, mail("galerie@art.example", "me@home.fr", "Votre visite", APPLE_MAIL));
    let letter = e.one(e.archive, mail("galerie@art.example", "me@home.fr", "Lettre de novembre", NEWSLETTER));
    let letter2 = e.one(e.archive, mail("galerie@art.example", "me@home.fr", "Lettre de décembre", NEWSLETTER));
    assert_eq!(e.address_kind("galerie@art.example").as_deref(), Some("person"));
    assert_eq!(e.kind(personal).as_deref(), Some("person"));
    assert_eq!(e.kind(letter).as_deref(), Some("robot"));
    assert_eq!(e.kind(letter2).as_deref(), Some("robot"));
    let human = ThreadFilter { human: Some(true), ..Default::default() };
    assert_eq!(e.subjects(e.archive, &human), ["Votre visite"]);
}

#[test]
fn a_discussion_list_is_people_despite_its_unsubscribe_link() {
    let mut e = Env::new();
    let group = "List-Id: <pommef.googlegroups.com>\r\nList-Unsubscribe: <mailto:u@googlegroups.com>\r\nPrecedence: list\r\n";
    let m = e.one(e.archive, mail("bob@gmail.com", "pommef@googlegroups.com", "Question", group));
    assert_eq!(e.address_kind("bob@gmail.com").as_deref(), Some("person"));
    assert_eq!(e.kind(m).as_deref(), Some("person"));
}

#[test]
fn an_automatic_reply_of_a_person_is_a_robot_mail_and_does_not_count() {
    let mut e = Env::new();
    e.one(e.archive, mail("alice@acme.fr", "me@home.fr", "Dîner", APPLE_MAIL));
    let away = e.one(e.archive, mail("alice@acme.fr", "me@home.fr", "Absente", "Auto-Submitted: auto-replied\r\n"));
    assert_eq!(e.address_kind("alice@acme.fr").as_deref(), Some("person"));
    assert_eq!(e.kind(away).as_deref(), Some("robot"));
    let counted: i64 = e.store.conn().query_row("SELECT sender_count FROM address WHERE addr = 'alice@acme.fr'", [], |r| r.get(0)).unwrap();
    assert_eq!(counted, 1);
}

#[test]
fn the_filter_splits_the_list_and_sees_the_users_reply_in_sent() {
    let mut e = Env::new();
    e.one(e.archive, mail("alice@acme.fr", "me@home.fr", "Dîner", APPLE_MAIL));
    e.one(e.archive, mail("hello@shop.example", "me@home.fr", "Soldes", NEWSLETTER));
    // A notification answered by the user: its thread holds a person's
    // mail (the reply, in Sent), so it is a conversation between people.
    let notif = mail("noreply@forum.example", "me@home.fr", "Nouveau message", "");
    e.one(e.archive, notif);
    let reply = "From: me@home.fr\r\nTo: noreply@forum.example\r\nSubject: Re: Nouveau message\r\nMessage-ID: <r@home.fr>\r\nIn-Reply-To: <Nouveau-message-noreply.forum.example@test>\r\n\r\nok\r\n";
    e.one(e.sent, reply.to_string());
    let all = e.subjects(e.archive, &ThreadFilter::default());
    let human = e.subjects(e.archive, &ThreadFilter { human: Some(true), ..Default::default() });
    let robot = e.subjects(e.archive, &ThreadFilter { human: Some(false), ..Default::default() });
    assert_eq!(human, ["Dîner", "Nouveau message"]);
    assert_eq!(robot, ["Soldes"]);
    assert_eq!(human.len() + robot.len(), all.len());
}

#[test]
fn forcing_a_verdict_rewrites_the_mails_and_undo_gives_it_back() {
    let mut e = Env::new();
    let news = e.one(e.inbox, mail("hello@shop.example", "me@home.fr", "Soldes", NEWSLETTER));
    let other = e.one(e.inbox, mail("hello@shop.example", "me@home.fr", "Bonjour", ""));
    let id = e.address("hello@shop.example");
    assert_eq!(e.kind(other).as_deref(), Some("robot"));

    let out = e.store.set_sender_kind(id, Some(SenderKind::Person), Source::App).unwrap();
    let token = out.undo.expect("an undo token");
    assert_eq!(e.address_kind("hello@shop.example").as_deref(), Some("person"));
    // Every mail already received follows the user's decision, the
    // newsletter itself too (`expediteurs.md`, section 9).
    assert_eq!(e.kind(other).as_deref(), Some("person"));
    assert!(e.thread_has_person(other));
    assert_eq!(e.kind(news).as_deref(), Some("person"));
    assert!(e.thread_has_person(news));
    // Journaled.
    let audit: String = e
        .store
        .conn()
        .query_row("SELECT detail FROM audit_log WHERE action = 'sender_kind' AND entity_id = ?1", [id], |r| r.get(0))
        .unwrap();
    assert!(audit.contains("\"after\":\"person\""), "{audit}");
    // The same choice again changes nothing.
    assert!(e.store.set_sender_kind(id, Some(SenderKind::Person), Source::App).unwrap().undo.is_none());

    // Undo: back to the computation.
    e.store.undo(&token, Source::App).unwrap();
    let forced: Option<String> = e.store.conn().query_row("SELECT sender_kind_user FROM address WHERE id = ?1", [id], |r| r.get(0)).unwrap();
    assert_eq!(forced, None);
    assert_eq!(e.kind(other).as_deref(), Some("robot"));
    assert!(!e.thread_has_person(other));
    assert_eq!(e.kind(news).as_deref(), Some("robot"));
    assert!(!e.thread_has_person(news));

    // A forced verdict survives new mail: the computation never overrides it.
    e.store.set_sender_kind(id, Some(SenderKind::Person), Source::App).unwrap();
    let third = e.one(e.inbox, mail("hello@shop.example", "me@home.fr", "Encore", ""));
    assert_eq!(e.kind(third).as_deref(), Some("person"));
    // A newsletter received after the choice follows it too.
    let letter = e.one(e.inbox, mail("hello@shop.example", "me@home.fr", "Soldes d'hiver", NEWSLETTER));
    assert_eq!(e.kind(letter).as_deref(), Some("person"));
    // Given back to the computation.
    e.store.set_sender_kind(id, None, Source::App).unwrap();
    assert_eq!(e.kind(third).as_deref(), Some("robot"));
    assert_eq!(e.kind(letter).as_deref(), Some("robot"));

    // An unknown address is an error.
    assert!(e.store.set_sender_kind(99_999, Some(SenderKind::Robot), Source::App).is_err());
}

/// The effective verdict of a mixed address does not change when the user
/// forces it to what it already is, but its mails do: the user's choice is
/// on the sender, and its newsletters become a person's mails. Forcing
/// robot turns its personal mails into robots; the user's own reply stays
/// a person.
#[test]
fn forcing_the_computed_verdict_rewrites_the_automatic_mails_too() {
    let mut e = Env::new();
    let reply = e.one(e.sent, mail("me@home.fr", "galerie@art.example", "Visite", ""));
    let personal = e.one(e.archive, mail("galerie@art.example", "me@home.fr", "Votre visite", APPLE_MAIL));
    let letter = e.one(e.archive, mail("galerie@art.example", "me@home.fr", "Lettre de novembre", NEWSLETTER));
    let away = e.one(e.archive, mail("galerie@art.example", "me@home.fr", "Absente", "Auto-Submitted: auto-replied\r\n"));
    let id = e.address("galerie@art.example");
    assert_eq!(e.address_kind("galerie@art.example").as_deref(), Some("person"));
    assert_eq!(e.kind(letter).as_deref(), Some("robot"));

    let out = e.store.set_sender_kind(id, Some(SenderKind::Person), Source::App).unwrap();
    assert!(out.undo.is_some(), "a choice that rewrites mails can be undone");
    assert_eq!(e.kind(personal).as_deref(), Some("person"));
    assert_eq!(e.kind(letter).as_deref(), Some("person"));
    assert_eq!(e.kind(away).as_deref(), Some("person"));
    let human = ThreadFilter { human: Some(true), ..Default::default() };
    assert_eq!(e.subjects(e.archive, &human).len(), 3);

    e.store.set_sender_kind(id, Some(SenderKind::Robot), Source::App).unwrap();
    assert_eq!(e.kind(personal).as_deref(), Some("robot"));
    assert_eq!(e.kind(letter).as_deref(), Some("robot"));
    assert_eq!(e.kind(away).as_deref(), Some("robot"));
    assert_eq!(e.kind(reply).as_deref(), Some("person"));

    // Back to the computation: the mixed address again.
    e.store.set_sender_kind(id, None, Source::App).unwrap();
    assert_eq!(e.kind(personal).as_deref(), Some("person"));
    assert_eq!(e.kind(letter).as_deref(), Some("robot"));
    assert_eq!(e.kind(away).as_deref(), Some("robot"));
}

#[test]
fn contacts_show_the_verdict_and_whether_it_is_forced() {
    let mut e = Env::new();
    e.one(e.inbox, mail("hello@shop.example", "me@home.fr", "Soldes", NEWSLETTER));
    e.one(e.inbox, mail("alice@acme.fr", "me@home.fr", "Dîner", APPLE_MAIL));
    e.store.set_sender_kind(e.address("alice@acme.fr"), Some(SenderKind::Robot), Source::App).unwrap();
    let (contacts, _) = e.store.list_contacts(None, None, 50).unwrap();
    let get = |a: &str| contacts.iter().find(|c| c.addr == a).unwrap();
    assert_eq!(get("hello@shop.example").sender_kind, Some(SenderKind::Robot));
    assert!(!get("hello@shop.example").sender_kind_forced);
    assert_eq!(get("alice@acme.fr").sender_kind, Some(SenderKind::Robot));
    assert!(get("alice@acme.fr").sender_kind_forced);
}

/// The catch-up of a mailbox migrated to version 12 gives exactly what the
/// insertion gives: same clues, same counts, same verdicts, same threads.
#[test]
fn the_catch_up_gives_what_the_insertion_gives() {
    let mut e = Env::new();
    e.one(e.sent, mail("me@home.fr", "galerie@art.example", "Visite", ""));
    e.insert(
        e.archive,
        &[
            mail("galerie@art.example", "me@home.fr", "Votre visite", APPLE_MAIL),
            mail("galerie@art.example", "me@home.fr", "Lettre", NEWSLETTER),
            mail("hello@shop.example", "me@home.fr", "Soldes", NEWSLETTER),
            mail("alice@acme.fr", "me@home.fr", "Dîner", APPLE_MAIL),
            mail("noreply@bank.example", "me@home.fr", "Relevé", ""),
            mail("x.y@acme.fr", "me@home.fr", "Sans indice", ""),
        ],
    );
    let snapshot = |s: &Store| -> Vec<String> {
        let mut out: Vec<String> = s
            .conn()
            .prepare(
                "SELECT 'm' || id || ' ' || coalesce(sender_signals, '-') || ' ' || coalesce(sender_kind, '-') FROM message
                 UNION ALL SELECT 'a' || addr || ' ' || coalesce(sender_kind, '-') || ' ' || sender_count || sender_bulk
                        || sender_list || sender_human || sender_client || written_to FROM address
                 UNION ALL SELECT 't' || id || ' ' || has_person FROM thread",
            )
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        out.sort();
        out
    };
    let live = snapshot(&e.store);
    // What the migration leaves: no clue, no count, no verdict (but
    // `written_to`, which the migration fills).
    e.store
        .conn()
        .execute_batch(
            "UPDATE message SET sender_signals = NULL, sender_kind = NULL;
             UPDATE address SET sender_kind = NULL, sender_count = 0, sender_bulk = 0, sender_list = 0,
                    sender_human = 0, sender_client = 0;
             UPDATE thread SET has_person = 0;",
        )
        .unwrap();
    assert_eq!(nm_store::senders::pending(&e.store).unwrap(), 7);
    let Env { store, raw, .. } = e;
    let store = Mutex::new(store);
    let mut batches = 0;
    // Batches of 2: the catch-up goes on from one batch to the next.
    let report = nm_store::senders::backfill(&store, &raw, 2, &mut |_, _| batches += 1).unwrap();
    assert_eq!(report.read, 7);
    assert_eq!(report.missing_raw, 0);
    assert_eq!(batches, 4);
    let store = store.into_inner().unwrap();
    assert_eq!(snapshot(&store), live);
    assert_eq!(nm_store::senders::pending(&store).unwrap(), 0);
    // Nothing left: a second run does nothing.
    let store = Mutex::new(store);
    assert_eq!(nm_store::senders::backfill(&store, &raw, 2, &mut |_, _| {}).unwrap().read, 0);
}

#[test]
fn a_mail_without_raw_file_is_caught_up_without_clue() {
    let mut e = Env::new();
    let m = e.one(e.archive, mail("x.y@acme.fr", "me@home.fr", "Sans brut", APPLE_MAIL));
    e.store
        .conn()
        .execute_batch(
            "UPDATE message SET sender_signals = NULL, sender_kind = NULL;
             UPDATE address SET sender_kind = NULL, sender_count = 0, sender_client = 0, sender_human = 0;",
        )
        .unwrap();
    let hash: Vec<u8> = e.store.conn().query_row("SELECT raw_hash FROM message WHERE id = ?1", [m], |r| r.get(0)).unwrap();
    let path = e.raw.path(&hash.try_into().unwrap());
    std::fs::remove_file(path).unwrap();
    let Env { store, raw, .. } = e;
    let store = Mutex::new(store);
    let report = nm_store::senders::backfill(&store, &raw, 10, &mut |_, _| {}).unwrap();
    assert_eq!((report.read, report.missing_raw), (0, 1));
    let store = store.into_inner().unwrap();
    let kind: Option<String> = store.conn().query_row("SELECT sender_kind FROM message WHERE id = ?1", [m], |r| r.get(0)).unwrap();
    assert_eq!(kind.as_deref(), Some("robot"));
}
