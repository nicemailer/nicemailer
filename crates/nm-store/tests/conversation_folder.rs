//! The folder of a conversation (`dossiers.md`, "Dossier d'une
//! conversation"; `schema.sql`, version 9): every list reads `thread`, whose
//! `folder` column and technical flags say where a conversation is listed.
//!
//! The mailbox is written directly in SQL (two accounts with every folder,
//! a few threads), then `refresh_thread_aggregates` computes the thread
//! columns as every write path of the core does; commands go through the
//! store's API.

use nm_api::{ThreadFilter, ThreadSort, ThreadSummary};
use nm_core::{FolderRole, Source};
use nm_store::{Store, rusqlite};

/// Two accounts, each with a folder of every role, and an unmanaged one.
struct Env {
    store: Store,
    accounts: [i64; 2],
}

impl Env {
    fn new() -> Env {
        let store = Store::open_in_memory().unwrap();
        let c = store.conn();
        let mut accounts = [0; 2];
        for (i, key) in ["perso", "pro"].iter().enumerate() {
            c.execute(
                "INSERT INTO account (config_key, kind, created_by, modified_by) VALUES (?1, 'archive', 'cli', 'cli')",
                [key],
            )
            .unwrap();
            accounts[i] = c.last_insert_rowid();
            for role in FolderRole::ALL {
                c.execute(
                    "INSERT INTO folder (account_id, path, role, created_by, modified_by) VALUES (?1, ?2, ?2, 'cli', 'cli')",
                    rusqlite::params![accounts[i], role.as_str()],
                )
                .unwrap();
            }
            c.execute(
                "INSERT INTO folder (account_id, path, created_by, modified_by) VALUES (?1, 'Old', 'cli', 'cli')",
                [accounts[i]],
            )
            .unwrap();
        }
        Env { store, accounts }
    }

    /// The folder of `role` (or the folder without a role, `None`) of the
    /// account `a` (0 or 1).
    fn folder(&self, a: usize, role: Option<FolderRole>) -> i64 {
        let c = self.store.conn();
        match role {
            Some(r) => c.query_row("SELECT id FROM folder WHERE account_id = ?1 AND role = ?2", rusqlite::params![self.accounts[a], r.as_str()], |r| r.get(0)),
            None => c.query_row("SELECT id FROM folder WHERE account_id = ?1 AND role IS NULL", [self.accounts[a]], |r| r.get(0)),
        }
        .unwrap()
    }

    /// A new thread of account `a`.
    fn thread(&self, a: usize, subject: &str) -> i64 {
        let c = self.store.conn();
        c.execute("INSERT INTO thread (account_id, subject) VALUES (?1, ?2)", rusqlite::params![self.accounts[a], subject]).unwrap();
        c.last_insert_rowid()
    }

    /// A mail of `thread` in the folder of `role` of account `a`, received
    /// at `date`, then the thread recomputed.
    fn mail(&mut self, a: usize, thread: i64, role: Option<FolderRole>, date: i64, seen: bool) -> i64 {
        let folder = self.folder(a, role);
        let c = self.store.conn();
        c.execute(
            "INSERT INTO message (account_id, folder_id, thread_id, subject, date_received, is_seen, date_modified, created_by, modified_by)
             VALUES (?1, ?2, ?3, 'x', ?4, ?5, ?4, 'cli', 'cli')",
            rusqlite::params![self.accounts[a], folder, thread, date, seen],
        )
        .unwrap();
        let id = c.last_insert_rowid();
        self.store.refresh_thread_aggregates(thread).unwrap();
        id
    }

    /// Every page of the folders `folders` listed together.
    fn list(&self, folders: &[i64], filter: &ThreadFilter) -> Vec<ThreadSummary> {
        pages(|c| self.store.list_threads_in(folders, c, 2, ThreadSort::Received, filter).unwrap())
    }

    /// The ids listed in the folder of `role`, of both accounts.
    fn ids(&self, role: FolderRole) -> Vec<i64> {
        self.list(&[self.folder(0, Some(role)), self.folder(1, Some(role))], &ThreadFilter::default())
            .iter()
            .map(|t| t.id)
            .collect()
    }

    fn code(&self, thread: i64) -> Option<String> {
        self.store.conn().query_row("SELECT folder FROM thread WHERE id = ?1", [thread], |r| r.get(0)).unwrap()
    }
}

/// Every page of a list, through the cursor.
fn pages(mut next: impl FnMut(Option<&str>) -> (Vec<ThreadSummary>, Option<String>)) -> Vec<ThreadSummary> {
    let mut out = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let (items, c) = next(cursor.as_deref());
        out.extend(items);
        match c {
            Some(c) => cursor = Some(c),
            None => return out,
        }
    }
}

#[test]
fn a_reply_lists_the_conversation_in_its_folder_and_in_sent() {
    let mut env = Env::new();
    let t = env.thread(0, "Devis");
    env.mail(0, t, Some(FolderRole::Inbox), 100, false);
    env.mail(0, t, Some(FolderRole::Sent), 200, true);
    // Its folder is the Inbox, not Sent where its latest mail is.
    assert_eq!(env.code(t).as_deref(), Some("i"));
    assert_eq!(env.ids(FolderRole::Inbox), [t]);
    assert_eq!(env.ids(FolderRole::Sent), [t]);
    // The same row everywhere: the whole thread, dated by its latest mail.
    let inbox = &env.list(&[env.folder(0, Some(FolderRole::Inbox))], &ThreadFilter::default())[0];
    let sent = &env.list(&[env.folder(0, Some(FolderRole::Sent))], &ThreadFilter::default())[0];
    assert_eq!((inbox.message_count, inbox.unread_count, inbox.date_last_received), (2, 1, 200));
    assert_eq!((sent.message_count, sent.date_last_received), (2, 200));
    // Each row is in the listed folder: moves from Sent have no target.
    assert_eq!(inbox.folder_id, env.folder(0, Some(FolderRole::Inbox)));
    assert_eq!(sent.folder_id, env.folder(0, Some(FolderRole::Sent)));
    // Everything and All Mail show it once, in its folder.
    let everything = pages(|c| env.store.list_everything(None, c, 10, ThreadSort::Received, &ThreadFilter::default()).unwrap());
    assert_eq!(everything.iter().map(|t| (t.id, t.folder_id)).collect::<Vec<_>>(), [(t, env.folder(0, Some(FolderRole::Inbox)))]);
    assert_eq!(everything[0].reply_target, Some(FolderRole::Archive));
    // Listed together (as the CLI may ask), Inbox and Sent show it once.
    let both = env.list(&[env.folder(0, Some(FolderRole::Inbox)), env.folder(0, Some(FolderRole::Sent))], &ThreadFilter::default());
    assert_eq!(both.iter().map(|t| (t.id, t.folder_id)).collect::<Vec<_>>(), [(t, env.folder(0, Some(FolderRole::Inbox)))]);
}

#[test]
fn a_new_mail_brings_the_whole_conversation_back_to_the_inbox() {
    let mut env = Env::new();
    let t = env.thread(0, "Toiture");
    env.mail(0, t, Some(FolderRole::Archive), 100, true);
    env.mail(0, t, Some(FolderRole::Archive), 110, true);
    assert_eq!(env.ids(FolderRole::Archive), [t]);
    env.mail(0, t, Some(FolderRole::Inbox), 200, false);
    assert_eq!(env.ids(FolderRole::Inbox), [t]);
    assert!(env.ids(FolderRole::Archive).is_empty(), "it left Archive");
    // A command files the whole conversation again.
    env.store.move_thread(t, FolderRole::Archive, Some(env.folder(0, Some(FolderRole::Inbox))), Default::default(), Source::Cli).unwrap();
    assert_eq!(env.ids(FolderRole::Archive), [t]);
    assert!(env.ids(FolderRole::Inbox).is_empty());
}

#[test]
fn technical_and_rejected_mails_do_not_take_a_conversation_away() {
    let mut env = Env::new();
    // A mail of an unknown sender in an archived conversation.
    let quarantined = env.thread(0, "q");
    env.mail(0, quarantined, Some(FolderRole::Archive), 100, true);
    env.mail(0, quarantined, Some(FolderRole::Quarantine), 200, false);
    assert_eq!(env.ids(FolderRole::Archive), [quarantined]);
    assert_eq!(env.ids(FolderRole::Quarantine), [quarantined]);
    // A banned sender's mail in a conversation of the Inbox.
    let banned = env.thread(0, "b");
    env.mail(0, banned, Some(FolderRole::Inbox), 300, false);
    env.mail(0, banned, Some(FolderRole::Banned), 400, false);
    assert_eq!(env.code(banned).as_deref(), Some("i"));
    assert!(!env.ids(FolderRole::Banned).contains(&banned));
    // Its counters leave the banned mail out.
    let row = env.list(&[env.folder(0, Some(FolderRole::Inbox))], &ThreadFilter::default())[0].clone();
    assert_eq!((row.id, row.message_count, row.unread_count, row.date_last_received), (banned, 1, 1, 300));
    // Only technical folders: the fixed order, Quarantine before Deleted.
    let fallback = env.thread(0, "f");
    env.mail(0, fallback, Some(FolderRole::Quarantine), 500, false);
    env.mail(0, fallback, Some(FolderRole::Deleted), 600, true);
    assert_eq!(env.code(fallback).as_deref(), Some("q"));
    let all_mail = pages(|c| env.store.list_all_mail(None, c, 10, ThreadSort::Received, &ThreadFilter::default()).unwrap());
    assert!(all_mail.iter().any(|t| t.id == fallback), "its quarantined mail keeps it in All Mail");
    // Wholly deleted: out of All Mail, its counters those of every mail.
    let gone = env.thread(0, "g");
    env.mail(0, gone, Some(FolderRole::Deleted), 700, false);
    let all_mail = pages(|c| env.store.list_all_mail(None, c, 10, ThreadSort::Received, &ThreadFilter::default()).unwrap());
    assert!(!all_mail.iter().any(|t| t.id == gone));
    let deleted = env.list(&[env.folder(0, Some(FolderRole::Deleted))], &ThreadFilter::default());
    let row = deleted.iter().find(|t| t.id == gone).unwrap();
    assert_eq!((row.message_count, row.date_last_received), (1, 700));
    // Only a folder without a role: in Everything only, in that folder.
    let old = env.thread(0, "o");
    env.mail(0, old, None, 800, true);
    assert_eq!(env.code(old), None);
    let everything = pages(|c| env.store.list_everything(None, c, 10, ThreadSort::Received, &ThreadFilter::default()).unwrap());
    assert_eq!(everything.iter().find(|t| t.id == old).map(|t| t.folder_id), Some(env.folder(0, None)));
    assert_eq!(env.list(&[env.folder(0, None)], &ThreadFilter::default()).iter().map(|t| t.id).collect::<Vec<_>>(), [old]);
}

#[test]
fn the_unified_view_is_one_list_of_every_account() {
    let mut env = Env::new();
    // Interleaved dates across the two accounts; the same mail received on
    // both makes two conversations, one per account.
    let mut expected = Vec::new();
    for i in 0..7 {
        let t = env.thread(i % 2, "x");
        env.mail(i % 2, t, Some(FolderRole::Inbox), 100 + i as i64, false);
        expected.push(t);
    }
    expected.reverse();
    let inboxes = [env.folder(0, Some(FolderRole::Inbox)), env.folder(1, Some(FolderRole::Inbox))];
    let all = env.list(&inboxes, &ThreadFilter::default());
    assert_eq!(all.iter().map(|t| t.id).collect::<Vec<_>>(), expected);
    // Each row is in the Inbox of its own account.
    for t in &all {
        let account: i64 = env.store.conn().query_row("SELECT account_id FROM thread WHERE id = ?1", [t.id], |r| r.get(0)).unwrap();
        let a = env.accounts.iter().position(|x| *x == account).unwrap();
        assert_eq!(t.folder_id, inboxes[a]);
    }
    // One account: its threads only, in the same order.
    let one = env.list(&inboxes[..1], &ThreadFilter::default());
    assert_eq!(one.iter().map(|t| t.id).collect::<Vec<_>>(), expected.iter().copied().filter(|t| all.iter().any(|x| x.id == *t && x.folder_id == inboxes[0])).collect::<Vec<_>>());
    // A third account with an Inbox, not listed: some accounts, not all.
    let c = env.store.conn();
    c.execute("INSERT INTO account (config_key, kind, created_by, modified_by) VALUES ('off', 'archive', 'cli', 'cli')", []).unwrap();
    let off = c.last_insert_rowid();
    c.execute("INSERT INTO folder (account_id, path, role, created_by, modified_by) VALUES (?1, 'INBOX', 'inbox', 'cli', 'cli')", [off]).unwrap();
    c.execute("INSERT INTO thread (account_id, subject, folder, date_last_received, message_count) VALUES (?1, 'z', 'i', 999, 1)", [off]).unwrap();
    assert_eq!(env.list(&inboxes, &ThreadFilter::default()).iter().map(|t| t.id).collect::<Vec<_>>(), expected);
}

#[test]
fn a_folder_that_changes_role_moves_its_conversations() {
    let mut env = Env::new();
    let t = env.thread(0, "x");
    env.mail(0, t, None, 100, true);
    assert_eq!(env.code(t), None);
    // The folder without a role becomes the Archive (the old one loses it).
    let path: String = env.store.conn().query_row("SELECT path FROM folder WHERE id = ?1", [env.folder(0, None)], |r| r.get(0)).unwrap();
    let assignment = nm_store::setup::RoleAssignment { path, delimiter: None, role: Some(FolderRole::Archive) };
    env.store.set_folder_roles(env.accounts[0], &[assignment], Source::App).unwrap();
    assert_eq!(env.code(t).as_deref(), Some("a"));
    assert_eq!(env.ids(FolderRole::Archive), [t]);
}

#[test]
fn conversation_date_and_mail_date_are_two_criteria() {
    let mut env = Env::new();
    // A conversation from 2016 to 2026, and one of 2018 only.
    let long = env.thread(0, "long");
    env.mail(0, long, Some(FolderRole::Archive), 1_460_000_000, true); // 2016
    env.mail(0, long, Some(FolderRole::Archive), 1_780_000_000, true); // 2026
    let short = env.thread(0, "short");
    env.mail(0, short, Some(FolderRole::Archive), 1_520_000_000, true); // 2018
    let y2020 = 1_577_836_800;
    let archive = [env.folder(0, Some(FolderRole::Archive))];
    let ids = |env: &Env, f: ThreadFilter| env.list(&archive, &f).iter().map(|t| t.id).collect::<Vec<_>>();
    // Conversation date: the date of the latest mail.
    assert_eq!(ids(&env, ThreadFilter { before: Some(y2020), ..Default::default() }), [short]);
    assert_eq!(ids(&env, ThreadFilter { after: Some(y2020), ..Default::default() }), [long]);
    // Mail date: any mail in the interval; still sorted by the thread.
    assert_eq!(ids(&env, ThreadFilter { mail_before: Some(y2020), ..Default::default() }), [long, short]);
    assert_eq!(ids(&env, ThreadFilter { mail_after: Some(1_500_000_000), mail_before: Some(y2020), ..Default::default() }), [short]);
    // The two combine.
    assert_eq!(ids(&env, ThreadFilter { after: Some(y2020), mail_before: Some(y2020), ..Default::default() }), [long]);
    // Mail date looks at the mails of the listed folders: an old mail of
    // the long thread moved to Deleted no longer counts in Archive, but
    // does in Everything.
    env.store
        .conn()
        .execute(
            "UPDATE message SET folder_id = ?1 WHERE thread_id = ?2 AND date_received = 1460000000",
            [env.folder(0, Some(FolderRole::Deleted)), long],
        )
        .unwrap();
    env.store.refresh_thread_aggregates(long).unwrap();
    assert_eq!(ids(&env, ThreadFilter { mail_before: Some(y2020), ..Default::default() }), [short]);
    let f = ThreadFilter { mail_before: Some(y2020), ..Default::default() };
    let everything = pages(|c| env.store.list_everything(None, c, 10, ThreadSort::Received, &f).unwrap());
    assert_eq!(everything.iter().map(|t| t.id).collect::<Vec<_>>(), [long, short]);
}

#[test]
fn folder_counts_follow_the_folder_and_the_flags() {
    let mut env = Env::new();
    let t = env.thread(0, "x");
    env.mail(0, t, Some(FolderRole::Inbox), 100, false);
    env.mail(0, t, Some(FolderRole::Sent), 200, true);
    let u = env.thread(0, "y");
    env.mail(0, u, Some(FolderRole::Sent), 300, true);
    let count = |role: FolderRole| {
        env.store.list_folders().unwrap().iter().find(|f| f.account_id == env.accounts[0] && f.role == Some(role)).unwrap().thread_count
    };
    assert_eq!(count(FolderRole::Inbox), 1);
    assert_eq!(count(FolderRole::Sent), 2);
    assert_eq!(count(FolderRole::Archive), 0);
}
