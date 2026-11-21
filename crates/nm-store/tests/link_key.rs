//! The link key of a mail (`link_key.rs`; `modele de donnees.md`,
//! `link_key`): the same on two databases that number the mail
//! differently, whether the mail came by IMAP or from an mbox export, and
//! the row a link opens when several rows share it.

use nm_core::Source;
use nm_store::ChangeSet;
use nm_store::ingest::{parse, parse_headers};
use nm_store::insert::{Body, Flags, Inserter, NewMessage};
use nm_store::raw::RawStore;
use nm_store::{Store, StoreError, rusqlite};

/// A database of its own, with one account and a folder per role asked.
struct Env {
    store: Store,
    raw: RawStore,
    account: i64,
    _dir: tempfile::TempDir,
}

impl Env {
    fn new() -> Env {
        let dir = tempfile::tempdir().unwrap();
        let raw = RawStore::open(dir.path()).unwrap();
        let store = Store::open_in_memory().unwrap();
        store
            .conn()
            .execute("INSERT INTO account (config_key, kind, created_by, modified_by) VALUES ('perso', 'imap', 'core', 'core')", [])
            .unwrap();
        let account = store.conn().last_insert_rowid();
        Env { store, raw, account, _dir: dir }
    }

    /// The folder of `role`, created the first time.
    fn folder(&self, role: &str) -> i64 {
        let c = self.store.conn();
        c.execute(
            "INSERT OR IGNORE INTO folder (account_id, path, delimiter, role, created_by, modified_by)
             VALUES (?1, ?2, '/', ?2, 'core', 'core')",
            rusqlite::params![self.account, role],
        )
        .unwrap();
        c.query_row("SELECT id FROM folder WHERE role = ?1", [role], |r| r.get(0)).unwrap()
    }

    /// Inserts `raw` in the folder of `role`: from its headers alone, as the
    /// synchronization does first, or whole, as the mbox import does.
    fn insert(&mut self, role: &str, uid: u32, received: i64, raw: &str, whole: bool) -> i64 {
        let folder = self.folder(role);
        let mut ins = Inserter::new(Source::Imap, None);
        let mut changes = ChangeSet::default();
        let written = whole.then(|| {
            let mut w = self.raw.writer(true).unwrap();
            w.write(raw.as_bytes()).unwrap();
            w.finish(Some(raw.len() as u64)).unwrap()
        });
        let parsed = written.as_ref().map(|f| parse(f.plain.as_ref().unwrap()));
        let header = &raw[..raw.find("\n\n").or(raw.find("\r\n\r\n")).map_or(raw.len(), |i| i + 2)];
        let h = parsed.clone().unwrap_or_else(|| parse_headers(header.as_bytes()));
        let m = NewMessage {
            account_id: self.account,
            folder_id: folder,
            uid: Some(uid),
            date_received: received,
            size: Some(raw.len() as u64),
            modseq: None,
            flags: Flags::default(),
            auth: (nm_core::AuthResult::Unknown, None),
            headers: &h,
        };
        let tx = self.store.transaction().unwrap();
        let body = written.as_ref().zip(parsed.as_ref()).map(|(f, p)| Body { written: &f.written, parsed: p });
        let id = ins.insert(&tx, &m, body, &mut changes).unwrap();
        ins.commit(tx, &mut changes).unwrap();
        id
    }

    /// The second step for a message inserted from its headers: its whole
    /// raw file.
    fn complete(&mut self, id: i64, raw: &str) {
        let mut ins = Inserter::new(Source::Imap, None);
        let mut w = self.raw.writer(true).unwrap();
        w.write(raw.as_bytes()).unwrap();
        let f = w.finish(Some(raw.len() as u64)).unwrap();
        let parsed = parse(&f.plain.unwrap());
        let mut changes = ChangeSet::default();
        let tx = self.store.transaction().unwrap();
        ins.complete(&tx, id, Body { written: &f.written, parsed: &parsed }, &mut changes).unwrap();
        ins.commit(tx, &mut changes).unwrap();
    }

    fn key(&self, id: i64) -> Option<i64> {
        self.store.conn().query_row("SELECT link_key FROM message WHERE id = ?1", [id], |r| r.get(0)).unwrap()
    }
}

/// The mail as an IMAP server gives it: CRLF line endings.
const BY_IMAP: &str = "Message-ID: <devis-42@a.example>\r\nFrom: alice@a.example\r\nTo: me@hey.example\r\n\
Subject: Devis toiture\r\n\r\nBonjour.\r\n";

/// The same mail from an mbox export (Gmail's Takeout): a header added by
/// the export, LF line endings. Other bytes, so another raw hash.
const BY_MBOX: &str = "X-Gmail-Labels: Inbox,Important\nMessage-ID: <devis-42@a.example>\nFrom: alice@a.example\n\
To: me@hey.example\nSubject: Devis toiture\n\nBonjour.\n";

const OTHER: &str = "Message-ID: <autre@b.example>\r\nFrom: bob@b.example\r\nSubject: Autre\r\n\r\nSalut.\r\n";

const NO_ID: &str = "From: robot@c.example\r\nSubject: Sans identifiant\r\n\r\nBip.\r\n";

#[test]
fn two_databases_that_number_a_mail_differently_give_it_the_same_link() {
    // Mac A synchronizes by IMAP, headers first; Mac B imported an mbox
    // after another mail, so the same mail has another id there.
    let mut a = Env::new();
    let on_a = a.insert("inbox", 1, 1_700_000_000, BY_IMAP, false);
    let mut b = Env::new();
    b.insert("archive", 1, 1_600_000_000, OTHER, true);
    let on_b = b.insert("archive", 2, 1_700_000_000, BY_MBOX, true);
    assert_ne!(on_a, on_b, "the ids differ");

    let link = a.store.message_link(on_a).unwrap();
    assert_eq!(link, b.store.message_link(on_b).unwrap(), "the links are the same");
    assert_eq!(link.len(), "nicemailer://m/".len() + 12, "{link}");
    // Each Mac opens its own row from the link.
    let key = nm_api::deep_link::parse_key(link.trim_start_matches("nicemailer://m/")).unwrap();
    assert_eq!(a.store.locate_link(key).unwrap().message_id, on_a);
    assert_eq!(b.store.locate_link(key).unwrap().message_id, on_b);
    // The body arriving on A does not change the key: the link already
    // pasted stays good.
    a.complete(on_a, BY_IMAP);
    assert_eq!(a.store.message_link(on_a).unwrap(), link);
    // The thread view gives the same link to the app.
    let thread: i64 = a.store.conn().query_row("SELECT thread_id FROM message WHERE id = ?1", [on_a], |r| r.get(0)).unwrap();
    let (_, messages) = a.store.get_thread(thread, &[]).unwrap();
    assert_eq!(messages[0].link, link);
}

#[test]
fn a_mail_without_message_id_takes_the_key_of_its_raw_file_once_downloaded() {
    let mut e = Env::new();
    let id = e.insert("inbox", 1, 1_700_000_000, NO_ID, false);
    // Headers only: nothing that depends only on the mail yet, so the link
    // is the id of this database.
    assert_eq!(e.key(id), None);
    assert_eq!(e.store.message_link(id).unwrap(), format!("nicemailer://m/{id}"));

    e.complete(id, NO_ID);
    let hash: Vec<u8> = e.store.conn().query_row("SELECT raw_hash FROM message WHERE id = ?1", [id], |r| r.get(0)).unwrap();
    // The key is the start of the raw file's name.
    let hex: String = hash[..6].iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(e.store.message_link(id).unwrap(), format!("nicemailer://m/{hex}"));
    // Evicting the raw file later (`gc.rs`) keeps the key.
    e.store.conn().execute("UPDATE message SET raw_hash = NULL, body_state = 0 WHERE id = ?1", [id]).unwrap();
    assert_eq!(e.store.message_link(id).unwrap(), format!("nicemailer://m/{hex}"));

    // Inserted whole, it has its key at once.
    let mut other = Env::new();
    let whole = other.insert("inbox", 1, 1_700_000_000, NO_ID, true);
    assert_eq!(other.store.message_link(whole).unwrap(), format!("nicemailer://m/{hex}"));
}

#[test]
fn the_link_opens_the_copy_out_of_the_bins_and_of_sent_then_the_latest() {
    let mut e = Env::new();
    // The same mail (one Message-ID) in four folders: its copy in Sent is
    // the latest, a copy in Deleted later still.
    let archive = e.insert("archive", 1, 1_700_000_000, BY_IMAP, false);
    let inbox = e.insert("inbox", 1, 1_700_000_100, BY_IMAP, false);
    let sent = e.insert("sent", 1, 1_700_000_200, BY_IMAP, false);
    let deleted = e.insert("deleted", 1, 1_700_000_300, BY_IMAP, false);
    let key = e.key(inbox).unwrap();
    for id in [archive, sent, deleted] {
        assert_eq!(e.key(id), Some(key));
    }
    // The latest received out of the bins and of Sent: the Inbox.
    assert_eq!(e.store.locate_link(key).unwrap().message_id, inbox);
    e.store.conn().execute("DELETE FROM message WHERE id IN (?1, ?2)", [inbox, archive]).unwrap();
    // Then Sent before the bins.
    assert_eq!(e.store.locate_link(key).unwrap().message_id, sent);
    e.store.conn().execute("DELETE FROM message WHERE id = ?1", [sent]).unwrap();
    // A copy in the bins is still better than nothing.
    let place = e.store.locate_link(key).unwrap();
    assert_eq!((place.message_id, place.folder_id), (deleted, e.folder("deleted")));
}

#[test]
fn a_key_no_mail_has_is_not_found() {
    let mut e = Env::new();
    e.insert("inbox", 1, 1_700_000_000, BY_IMAP, false);
    assert!(matches!(e.store.locate_link(0x123456789abc), Err(StoreError::NotFound(_))));
    assert!(matches!(e.store.message_link(999), Err(StoreError::NotFound(_))));
}
