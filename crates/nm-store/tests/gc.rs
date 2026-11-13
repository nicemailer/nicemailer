//! Housekeeping of the raw store (`gc.rs`; `stockage-brut.md`, sections 6,
//! 7 and 11): the garbage collector, the race with an ingestion of the same
//! hash, the sweep of orphans, and a raw file corrupt or missing.

use std::sync::Mutex;
use std::time::Duration;

use nm_core::Source;
use nm_store::gc::collect_garbage;
use nm_store::ingest::parse;
use nm_store::insert::{Body, Flags, Inserter, NewMessage};
use nm_store::raw::{RawStore, hex};
use nm_store::{ChangeSet, Store};

struct Env {
    store: Mutex<Store>,
    raw: RawStore,
    dir: tempfile::TempDir,
    folder: i64,
}

fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let raw = RawStore::open(dir.path()).unwrap();
    let store = Store::open_in_memory().unwrap();
    store
        .conn()
        .execute_batch(
            "INSERT INTO account (config_key, kind, created_by, modified_by) VALUES ('perso', 'imap', 'core', 'core');
             INSERT INTO folder (account_id, path, delimiter, role, created_by, modified_by) VALUES (1, 'INBOX', '/', 'inbox', 'core', 'core');",
        )
        .unwrap();
    Env { store: Mutex::new(store), raw, dir, folder: 1 }
}

fn mail(id: &str) -> String {
    format!("Message-ID: <{id}@x>\r\nFrom: a@a.example\r\nSubject: {id}\r\nContent-Type: multipart/mixed; boundary=B\r\n\r\n--B\r\n\
Content-Type: text/plain\r\n\r\nHello {id}\r\n--B\r\nContent-Type: application/pdf\r\nContent-Disposition: attachment; filename=a.pdf\r\n\
Content-Transfer-Encoding: base64\r\n\r\nJVBERi0xLjQK\r\n--B--\r\n")
}

impl Env {
    /// Stores and inserts a complete mail, UID `uid` (`None`: an archive
    /// account's, without server).
    fn add(&self, id: &str, uid: Option<u32>) -> (i64, nm_store::raw::Hash) {
        let raw = mail(id);
        let (w, pin) = self.raw.write_pinned(raw.as_bytes()).unwrap();
        let parsed = parse(raw.as_bytes());
        let mut st = self.store.lock().unwrap();
        let mut ins = Inserter::new(Source::Imap, None);
        let mut changes = ChangeSet::default();
        let tx = st.transaction().unwrap();
        let m = NewMessage {
            account_id: 1,
            folder_id: self.folder,
            uid,
            date_received: 0,
            size: Some(w.size),
            modseq: None,
            flags: Flags::default(),
            auth: (nm_core::AuthResult::Unknown, None),
            headers: &parsed,
        };
        let id = ins.insert(&tx, &m, Some(Body { written: &w, parsed: &parsed }), &mut changes).unwrap();
        ins.commit(tx, &mut changes).unwrap();
        drop(pin);
        (id, w.hash)
    }

    fn count(&self, sql: &str) -> i64 {
        self.store.lock().unwrap().conn().query_row(sql, [], |r| r.get(0)).unwrap()
    }

    fn gc(&self) -> nm_store::gc::GcReport {
        collect_garbage(&self.store, &self.raw, Some(&self.dir.path().join("attachments.noindex")), Duration::from_secs(0)).unwrap()
    }
}

#[test]
fn files_nothing_references_go_and_the_others_stay() {
    let e = env();
    let (gone, gone_hash) = e.add("a", Some(1));
    let (_, kept_hash) = e.add("b", Some(2));
    // An extracted attachment of the mail about to go.
    let att = e.dir.path().join("attachments.noindex").join(hex(&gone_hash)).join("2");
    std::fs::create_dir_all(&att).unwrap();
    // The server deleted mail `a`.
    e.store.lock().unwrap().conn().execute("DELETE FROM message WHERE id = ?1", [gone]).unwrap();
    let r = e.gc();
    assert_eq!((r.blobs, r.attachment_dirs), (1, 1));
    assert!(!e.raw.path(&gone_hash).exists() && !att.parent().unwrap().exists());
    assert!(e.raw.path(&kept_hash).exists());
    assert_eq!(e.count("SELECT count(*) FROM raw_blob"), 1);
    assert_eq!(e.gc(), Default::default(), "nothing more");
}

#[test]
fn a_hash_an_ingestion_holds_is_never_collected() {
    // `stockage-brut.md`, section 11: "GC concurrent avec une ingestion du
    // même hash". An ingestion found the file already there (pinned), the
    // only row referencing it is deleted meanwhile: the collector leaves the
    // file, so the row the ingestion commits next has its file.
    let e = env();
    let (id, hash) = e.add("a", Some(1));
    let (w, pin) = e.raw.write_pinned(mail("a").as_bytes()).unwrap();
    assert!(w.existed && w.hash == hash);
    e.store.lock().unwrap().conn().execute("DELETE FROM message WHERE id = ?1", [id]).unwrap();
    assert_eq!(e.gc().blobs, 0);
    assert!(e.raw.path(&hash).exists());
    drop(pin);
    assert_eq!(e.gc().blobs, 1, "collected once released");
}

#[test]
fn leftovers_of_a_crash_are_swept() {
    let e = env();
    let (_, kept) = e.add("kept", Some(1));
    // A file renamed into place whose row was never committed, and a
    // temporary file: what a crash leaves.
    let orphan = e.raw.write(b"Subject: orphan\r\n\r\nx\r\n").unwrap();
    let tmp = e.dir.path().join("raw.noindex/.tmp/leftover");
    std::fs::write(&tmp, b"x").unwrap();
    // Young, they stay (the grace delay; a write may be in progress).
    let young = collect_garbage(&e.store, &e.raw, None, Duration::from_secs(3600)).unwrap();
    assert_eq!((young.orphans, young.temporary), (0, 0));
    // Old enough, they go; the file with a row stays.
    let old = std::time::SystemTime::now() - Duration::from_secs(2 * 86_400);
    for p in [e.raw.path(&orphan.hash), tmp.clone()] {
        let f = std::fs::File::options().write(true).open(&p).or_else(|_| std::fs::File::open(&p)).unwrap();
        f.set_modified(old).unwrap();
    }
    let r = e.gc();
    assert_eq!((r.orphans, r.temporary), (1, 1));
    assert!(!e.raw.path(&orphan.hash).exists() && !tmp.exists());
    assert!(e.raw.path(&kept).exists());
}

#[test]
fn a_damaged_raw_file_is_set_aside_and_its_mail_fetched_again() {
    let e = env();
    let (id, hash) = e.add("a", Some(7));
    // Corrupt one byte of the compressed file.
    let p = e.raw.path(&hash);
    let mut bytes = std::fs::read(&p).unwrap();
    let n = bytes.len() / 2;
    bytes[n] ^= 0xff;
    std::fs::set_permissions(&p, std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
    std::fs::write(&p, &bytes).unwrap();
    // Reading an attachment finds it.
    let files = nm_store::attachments::AttachmentFiles::open(e.dir.path()).unwrap();
    let att: i64 = e.count("SELECT id FROM attachment");
    let err = e.store.lock().unwrap().get_attachment(&e.raw, &files, att).unwrap_err();
    assert!(matches!(err, nm_store::StoreError::RawDamaged(ref h) if *h == hex(&hash)), "{err:?}");
    let r = e.store.lock().unwrap().raw_damaged(&e.raw, &hash).unwrap();
    assert_eq!((r.refetch, r.lost, r.set_aside), (1, 0, true));
    assert!(r.accounts.contains(&1));
    assert!(!p.exists());
    assert_eq!(std::fs::read_dir(e.dir.path().join("raw.noindex/.corrupt")).unwrap().count(), 1, "kept for inspection");
    // Back to "headers only": the body connection downloads it again.
    assert_eq!(e.count(&format!("SELECT body_state FROM message WHERE id = {id}")), 0);
    assert_eq!(e.count("SELECT count(*) FROM message_body") + e.count("SELECT count(*) FROM attachment") + e.count("SELECT count(*) FROM raw_blob"), 0);
    let missing = e.store.lock().unwrap().bodies_missing(1).unwrap();
    assert_eq!(missing.len(), 1);
    // Downloaded again, it completes like the first time.
    let raw = mail("a");
    let f = {
        let mut w = e.raw.writer(true).unwrap();
        w.write(raw.as_bytes()).unwrap();
        w.finish(None).unwrap()
    };
    let parsed = parse(raw.as_bytes());
    let mut st = e.store.lock().unwrap();
    let mut ins = Inserter::new(Source::Imap, None);
    let mut changes = ChangeSet::default();
    let tx = st.transaction().unwrap();
    ins.complete(&tx, id, Body { written: &f.written, parsed: &parsed }, &mut changes).unwrap();
    ins.commit(tx, &mut changes).unwrap();
    drop(st);
    assert_eq!(e.count("SELECT count(*) FROM attachment"), 1);
}

#[test]
fn a_damaged_mail_without_server_cannot_be_fetched_again() {
    let e = env();
    let (_, hash) = e.add("arch", None);
    std::fs::remove_file(e.raw.path(&hash)).unwrap();
    let r = e.store.lock().unwrap().raw_damaged(&e.raw, &hash).unwrap();
    assert_eq!((r.refetch, r.lost, r.set_aside), (0, 1, false));
    assert_eq!(e.count("SELECT body_state FROM message"), 1, "kept as it was, with its error");
}

/// The sweep reads `raw_blob` one range of hashes at a time (`SWEEP_CHUNK`
/// files): over several chunks, with orphans spread among files that have
/// their row, only the old orphans go.
#[test]
fn the_sweep_finds_the_orphans_among_many_files() {
    let e = env();
    let kept: Vec<_> = (0..1200).map(|i| e.add(&format!("m{i}"), Some(i + 1)).1).collect();
    // Orphans: their hashes fall anywhere among the others'.
    let orphans: Vec<_> = (0..30).map(|i| e.raw.write(format!("Subject: orphan {i}\r\n\r\nx\r\n").as_bytes()).unwrap().hash).collect();
    let old = std::time::SystemTime::now() - Duration::from_secs(2 * 86_400);
    for h in &orphans[..20] {
        let p = e.raw.path(h);
        let f = std::fs::File::options().write(true).open(&p).or_else(|_| std::fs::File::open(&p)).unwrap();
        f.set_modified(old).unwrap();
    }
    let r = collect_garbage(&e.store, &e.raw, None, Duration::from_secs(3600)).unwrap();
    assert_eq!((r.blobs, r.orphans), (0, 20));
    assert!(orphans[..20].iter().all(|h| !e.raw.path(h).exists()));
    // Young: a write may be in progress.
    assert!(orphans[20..].iter().all(|h| e.raw.path(h).exists()));
    assert!(kept.iter().all(|h| e.raw.path(h).exists()));
}
