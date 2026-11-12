//! The insertion path shared by the mbox import and the IMAP
//! synchronization (`insert.rs`, `plan-imap.md`, phase 3): headers first,
//! body later, threads whatever the order of arrival, and a batch that
//! fails leaves nothing behind.

use nm_core::Source;
use nm_store::ChangeSet;
use nm_store::ingest::{parse, parse_headers};
use nm_store::insert::{Body, Completed, Flags, Inserter, NewMessage};
use nm_store::raw::RawStore;
use nm_store::{Store, rusqlite};

struct Env {
    store: Store,
    raw: RawStore,
    account: i64,
    inbox: i64,
    archive: i64,
    _dir: tempfile::TempDir,
}

impl Env {
    fn new() -> Env {
        let dir = tempfile::tempdir().unwrap();
        let raw = RawStore::open(dir.path()).unwrap();
        let store = Store::open_in_memory().unwrap();
        let c = store.conn();
        c.execute("INSERT INTO account (config_key, kind, created_by, modified_by) VALUES ('perso', 'imap', 'core', 'core')", [])
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
        Env { store, raw, account, inbox, archive, _dir: dir }
    }

    fn count(&self, sql: &str) -> i64 {
        self.store.conn().query_row(sql, [], |r| r.get(0)).unwrap()
    }

    fn int(&self, sql: &str, id: i64) -> Option<i64> {
        self.store.conn().query_row(sql, [id], |r| r.get(0)).unwrap()
    }

    /// Inserts messages from their headers alone, in one transaction, as
    /// the synchronization will; returns their ids.
    fn headers(&mut self, ins: &mut Inserter, folder: i64, mails: &[(u32, &str)]) -> (Vec<i64>, ChangeSet) {
        let mut changes = ChangeSet::default();
        let tx = self.store.transaction().unwrap();
        let mut ids = Vec::new();
        for (uid, raw) in mails {
            let header = &raw[..raw.find("\r\n\r\n").map_or(raw.len(), |i| i + 4)];
            let h = parse_headers(header.as_bytes());
            let m = NewMessage {
                account_id: self.account,
                folder_id: folder,
                uid: Some(*uid),
                date_received: 1_700_000_000 + *uid as i64,
                size: Some(raw.len() as u64),
                modseq: Some(10 + *uid as u64),
                flags: Flags { seen: *uid % 2 == 0, flagged: *uid == 3, ..Flags::default() },
                auth: (nm_core::AuthResult::Unknown, None),
                headers: &h,
            };
            ids.push(ins.insert(&tx, &m, None, &mut changes).unwrap());
        }
        ins.commit(tx, &mut changes).unwrap();
        (ids, changes)
    }

    /// The second step for `id`: the whole message stored, then completed.
    fn body(&mut self, ins: &mut Inserter, id: i64, raw: &str) -> (Completed, ChangeSet) {
        let mut w = self.raw.writer(true).unwrap();
        // In pieces, as IMAP gives large messages.
        for piece in raw.as_bytes().chunks(5) {
            w.write(piece).unwrap();
        }
        let f = w.finish(Some(raw.len() as u64)).unwrap();
        let parsed = parse(&f.plain.unwrap());
        let mut changes = ChangeSet::default();
        let tx = self.store.transaction().unwrap();
        let done = ins.complete(&tx, id, Body { written: &f.written, parsed: &parsed }, &mut changes).unwrap();
        ins.commit(tx, &mut changes).unwrap();
        (done, changes)
    }

    fn search(&self, words: &str) -> i64 {
        self.store.conn().query_row("SELECT count(*) FROM message_fts WHERE message_fts MATCH ?1", [words], |r| r.get(0)).unwrap()
    }
}

const ORIGINAL: &str = "Message-ID: <m1@a.example>\r\nFrom: \"Alice Martin\" <Alice@a.example>\r\nTo: Me <me@hey.example>\r\n\
Subject: Devis toiture\r\nDate: Tue, 23 Jun 2020 21:20:56 +0200\r\nList-Unsubscribe: <https://a.example/u>\r\n\
Content-Type: multipart/mixed; boundary=B\r\n\r\n--B\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n\
Bonjour, voici le devis pour la rénovation.\r\n--B\r\nContent-Type: application/pdf\r\n\
Content-Disposition: attachment; filename=devis.pdf\r\nContent-Transfer-Encoding: base64\r\n\r\nJVBERi0xLjQK\r\n--B--\r\n";

const REPLY: &str = "Message-ID: <m2@hey.example>\r\nFrom: me@hey.example\r\nTo: alice@a.example\r\nSubject: Re: Devis toiture\r\n\
In-Reply-To: <m1@a.example>\r\nReferences: <m1@a.example>\r\n\r\nOK pour moi.\r\n";

const LATER: &str = "Message-ID: <m3@a.example>\r\nFrom: alice@a.example\r\nTo: me@hey.example\r\nSubject: Re: Re: Devis toiture\r\n\
References: <m1@a.example> <m2@hey.example>\r\n\r\nParfait.\r\n";

#[test]
fn headers_first_then_the_body() {
    let mut env = Env::new();
    let mut ins = Inserter::new(Source::Imap, None);
    let (ids, changes) = env.headers(&mut ins, env.inbox, &[(3, ORIGINAL)]);
    let id = ids[0];
    assert!(changes.folders.contains(&env.inbox));
    assert!(changes.messages.is_empty(), "a new message is announced by its folder, not as a change");

    // The list can show it, the search finds its headers, not its body yet.
    assert_eq!(env.int("SELECT body_state FROM message WHERE id = ?1", id), Some(0));
    assert_eq!(env.int("SELECT raw_hash IS NULL FROM message WHERE id = ?1", id), Some(1));
    assert_eq!(env.int("SELECT uid FROM message WHERE id = ?1", id), Some(3));
    assert_eq!(env.int("SELECT modseq FROM message WHERE id = ?1", id), Some(13));
    assert_eq!(env.int("SELECT is_flagged * 10 + is_seen FROM message WHERE id = ?1", id), Some(10));
    assert_eq!(env.int("SELECT size FROM message WHERE id = ?1", id), Some(ORIGINAL.len() as i64));
    assert_eq!(env.count("SELECT total_count * 10 + unread_count FROM folder WHERE role = 'inbox'"), 11);
    assert_eq!(env.search("toiture"), 1);
    assert_eq!(env.search("alice"), 1);
    assert_eq!(env.search("rénovation"), 0);
    assert_eq!(env.count("SELECT count(*) FROM message_body"), 0);
    assert_eq!(env.count("SELECT count(*) FROM address WHERE imported_from IS NULL"), 2, "Alice and me");

    let (done, changes) = env.body(&mut ins, id, ORIGINAL);
    assert_eq!(done, Completed::Done);
    assert_eq!(changes.messages.len(), 1, "a client showing it wants its body");
    assert_eq!(env.int("SELECT body_state FROM message WHERE id = ?1", id), Some(1));
    assert_eq!(env.int("SELECT has_attachments FROM message WHERE id = ?1", id), Some(1));
    assert_eq!(env.count("SELECT count(*) FROM raw_blob"), 1);
    assert_eq!(env.count("SELECT count(*) FROM attachment WHERE filename = 'devis.pdf' AND size = 9"), 1);
    assert_eq!(env.count("SELECT count(*) FROM message_unsubscribe"), 1);
    assert_eq!(env.search("rénovation"), 1, "the body is in the index now");
    assert_eq!(env.search("toiture"), 1, "and the headers still are");
    let snippet: String = env.store.conn().query_row("SELECT snippet FROM message WHERE id = ?1", [id], |r| r.get(0)).unwrap();
    assert!(snippet.starts_with("Bonjour, voici le devis"), "{snippet}");

    // Downloaded twice, deleted meanwhile: nothing happens.
    assert_eq!(env.body(&mut ins, id, ORIGINAL).0, Completed::Already);
    assert_eq!(env.body(&mut ins, id + 100, ORIGINAL).0, Completed::Missing);
    assert_eq!(env.count("SELECT count(*) FROM message_body"), 1);
}

const BOB: &str = "Message-ID: <x1@b.example>\r\nFrom: bob@b.example\r\nTo: alice@a.example\r\nSubject: Re: Devis toiture\r\n\
In-Reply-To: <m1@a.example>\r\n\r\nEt moi ?\r\n";

const UNRELATED: &str = "Message-ID: <z@c.example>\r\nFrom: carol@c.example\r\nSubject: Devis toiture\r\n\r\nAutre chose.\r\n";

/// The threads of `mails` inserted in this order, one transaction each, as
/// groups of positions in `mails`: `[[0, 2], [1]]`.
fn threads_of(mails: &[&str], order: &[usize]) -> Vec<Vec<usize>> {
    let mut env = Env::new();
    let mut ins = Inserter::new(Source::Imap, None);
    let mut ids = vec![0; mails.len()];
    for &i in order {
        let (got, _) = env.headers(&mut ins, env.inbox, &[(i as u32 + 1, mails[i])]);
        ids[i] = got[0];
    }
    let mut groups: std::collections::BTreeMap<i64, Vec<usize>> = Default::default();
    for (i, id) in ids.iter().enumerate() {
        groups.entry(env.int("SELECT thread_id FROM message WHERE id = ?1", *id).unwrap()).or_default().push(i);
    }
    // Every thread has the right counters, and no thread is left empty.
    assert_eq!(env.count("SELECT count(*) FROM thread"), groups.len() as i64);
    assert_eq!(env.count("SELECT sum(message_count) FROM thread"), mails.len() as i64);
    let mut out: Vec<Vec<usize>> = groups.into_values().collect();
    out.sort();
    out
}

/// Every order of `items[k..]` (Heap's swaps), appended to `out`.
fn permutations(items: &mut Vec<usize>, k: usize, out: &mut Vec<Vec<usize>>) {
    if k == items.len() {
        out.push(items.clone());
        return;
    }
    for i in k..items.len() {
        items.swap(k, i);
        permutations(items, k + 1, out);
        items.swap(k, i);
    }
}

#[test]
fn threads_whatever_the_order_of_arrival() {
    // The original, two answers to it (one from Bob, who does not cite the
    // other answer), a third message citing both, and a mail with the same
    // subject but no link: same threads in every one of the 120 orders.
    let mails = [ORIGINAL, REPLY, BOB, LATER, UNRELATED];
    let expected = vec![vec![0, 1, 2, 3], vec![4]];
    let mut orders = Vec::new();
    permutations(&mut (0..mails.len()).collect(), 0, &mut orders);
    assert_eq!(orders.len(), 120);
    for order in &orders {
        assert_eq!(threads_of(&mails, order), expected, "order {order:?}");
    }
}

#[test]
fn two_answers_to_a_message_never_seen_are_one_conversation() {
    // The original is not in the database (deleted, or in a folder
    // NiceMailer does not synchronize): its answers still go together,
    // since both cite it.
    assert_eq!(threads_of(&[REPLY, BOB], &[0, 1]), vec![vec![0, 1]]);
    assert_eq!(threads_of(&[REPLY, BOB], &[1, 0]), vec![vec![0, 1]]);
}

#[test]
fn a_merge_keeps_the_oldest_thread_and_refreshes_every_folder() {
    let mut env = Env::new();
    let mut ins = Inserter::new(Source::Imap, None);
    let (a, _) = env.headers(&mut ins, env.archive, &[(1, REPLY)]);
    let (b, _) = env.headers(&mut ins, env.inbox, &[(2, UNRELATED)]);
    let old = env.int("SELECT thread_id FROM message WHERE id = ?1", a[0]).unwrap();
    // A message citing both REPLY and UNRELATED merges their threads.
    let both = "Message-ID: <w@a.example>\r\nFrom: alice@a.example\r\nSubject: Fusion\r\nReferences: <m2@hey.example> <z@c.example>\r\n\r\n.\r\n";
    let (c, changes) = env.headers(&mut ins, env.inbox, &[(3, both)]);
    for id in [a[0], b[0], c[0]] {
        assert_eq!(env.int("SELECT thread_id FROM message WHERE id = ?1", id), Some(old));
    }
    assert_eq!(env.count("SELECT count(*) FROM thread"), 1);
    assert!(changes.folders.contains(&env.inbox) && changes.folders.contains(&env.archive));
    // Subject of the first message received; the counters of the whole
    // thread, and its folder, that of its latest mail (the Inbox).
    let subject: String = env.store.conn().query_row("SELECT subject FROM thread", [], |r| r.get(0)).unwrap();
    assert_eq!(subject, "Re: Devis toiture");
    assert_eq!(env.count("SELECT message_count FROM thread"), 3);
    let folder: String = env.store.conn().query_row("SELECT folder FROM thread", [], |r| r.get(0)).unwrap();
    assert_eq!(folder, "i");
}

#[test]
fn a_batch_rolled_back_leaves_nothing_and_its_addresses_are_forgotten() {
    let mut env = Env::new();
    let mut ins = Inserter::new(Source::Imap, None);
    {
        let tx = env.store.transaction().unwrap();
        let h = parse_headers(ORIGINAL.as_bytes());
        let m = NewMessage {
            account_id: env.account,
            folder_id: env.inbox,
            uid: Some(1),
            date_received: 0,
            size: None,
            modseq: None,
            flags: Flags::default(),
            auth: (nm_core::AuthResult::Unknown, None),
            headers: &h,
        };
        ins.insert(&tx, &m, None, &mut ChangeSet::default()).unwrap();
        // Something fails later in the batch: `tx` is dropped, so rolled
        // back, and the inserter is told.
        drop(tx);
        ins.abandon();
    }
    assert_eq!(env.count("SELECT count(*) FROM message") + env.count("SELECT count(*) FROM address"), 0);
    assert_eq!(env.count("SELECT count(*) FROM thread"), 0);
    // The next batch creates the addresses again, rather than using ids
    // of rows that were rolled back (a foreign key error, or worse, the id
    // of another address).
    let (ids, _) = env.headers(&mut ins, env.inbox, &[(1, ORIGINAL)]);
    let from: String = env
        .store
        .conn()
        .query_row("SELECT a.addr FROM message m JOIN address a ON a.id = m.from_address_id WHERE m.id = ?1", [ids[0]], |r| r.get(0))
        .unwrap();
    assert_eq!(from, "alice@a.example");
    assert_eq!(env.count("SELECT count(*) FROM address"), 2);
}

#[test]
fn a_full_disk_while_the_body_arrives_changes_nothing() {
    let mut env = Env::new();
    let mut ins = Inserter::new(Source::Imap, None);
    let (ids, _) = env.headers(&mut ins, env.inbox, &[(1, ORIGINAL)]);
    let full = env.raw.clone().with_disk_space(10);
    let mut w = full.writer(true).unwrap();
    // The error comes at a write or at the end, depending on buffering;
    // either way the caller stops there and never reaches `complete`.
    let failed = w.write(ORIGINAL.as_bytes()).is_err() || w.finish(Some(ORIGINAL.len() as u64)).is_err();
    assert!(failed);
    assert_eq!(env.int("SELECT body_state FROM message WHERE id = ?1", ids[0]), Some(0), "still waiting for its body");
    assert_eq!(env.count("SELECT count(*) FROM raw_blob"), 0);
    let tmp = env._dir.path().join(nm_store::raw::RAW_DIR).join(".tmp");
    assert_eq!(std::fs::read_dir(tmp).unwrap().count(), 0, "no partial file");
    // The next attempt, with room, completes it.
    assert_eq!(env.body(&mut ins, ids[0], ORIGINAL).0, Completed::Done);
}
