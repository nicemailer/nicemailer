//! The common ingestion fed by a real IMAP server (`plan-imap.md`, phase
//! 3): the headers of a message first, then its body streamed from the
//! server into the raw store, as the synchronization of phase 4 will do.
//!
//! `#[ignore]`: needs the test servers (`make imap-up`, then
//! `make imap-test`). Each test logs in as a new random user, whose
//! mailbox is empty.

use std::path::PathBuf;

use nm_core::Source;
use nm_proto::imap::*;
use nm_store::ingest::{parse, parse_headers};
use nm_store::insert::{Body, Completed, Flags, Inserter, NewMessage};
use nm_store::raw::{PlainBytes, RAW_DIR, RawStore};
use nm_store::{ChangeSet, Store};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

fn spec(port: u16) -> ServerSpec {
    let ca = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../dev/dovecot/tls/ca.crt");
    ServerSpec { host: "127.0.0.1".into(), port, security: Security::Tls, ca_file: Some(ca) }
}

async fn open(port: u16, user: &str) -> Session {
    let mut s = Session::connect(&spec(port), Timeouts::default()).await.unwrap();
    s.login(user, "nicemailer").await.unwrap();
    s
}

/// A mail of about 22 MB: a text part and a 16 MB attachment in base64.
/// Above the 16 MB the raw store keeps in memory for parsing, so its plain
/// copy is a mapped file.
fn big_mail() -> Vec<u8> {
    let mut m = b"Message-ID: <big@test.nicemailer>\r\nFrom: Alice <alice@example.com>\r\nTo: bob@example.com\r\n\
Subject: Les plans\r\nDate: Thu, 05 Nov 2026 10:00:00 +0000\r\nContent-Type: multipart/mixed; boundary=B\r\n\r\n\
--B\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nVoici les plans du chantier.\r\n--B\r\n\
Content-Type: application/pdf\r\nContent-Disposition: attachment; filename=plans.pdf\r\nContent-Transfer-Encoding: base64\r\n\r\n"
        .to_vec();
    let line: Vec<u8> = (0..76).map(|i| b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"[(i * 7) % 64]).collect();
    // 76 characters encode 57 bytes: 300,000 lines are 17.1 MB decoded.
    for _ in 0..300_000 {
        m.extend_from_slice(&line);
        m.extend_from_slice(b"\r\n");
    }
    m.extend_from_slice(b"--B--\r\n");
    m
}

struct Env {
    store: Store,
    raw: RawStore,
    dir: tempfile::TempDir,
}

impl Env {
    fn new() -> Env {
        let dir = tempfile::tempdir().unwrap();
        let raw = RawStore::open(dir.path()).unwrap();
        let store = Store::open_in_memory().unwrap();
        store
            .conn()
            .execute_batch(
                "INSERT INTO account (config_key, kind, created_by, modified_by) VALUES ('dovecot', 'imap', 'core', 'core');
                 INSERT INTO folder (account_id, path, delimiter, role, created_by, modified_by)
                 VALUES (1, 'INBOX', '/', 'inbox', 'core', 'core');",
            )
            .unwrap();
        Env { store, raw, dir }
    }

    fn tmp_files(&self) -> usize {
        std::fs::read_dir(self.dir.path().join(RAW_DIR).join(".tmp")).unwrap().count()
    }

    fn body_state(&self, id: i64) -> i64 {
        self.store.conn().query_row("SELECT body_state FROM message WHERE id = ?1", [id], |r| r.get(0)).unwrap()
    }
}

/// Step 1: the headers, flags, date and size of `uid`, inserted.
async fn insert_headers(s: &mut Session, env: &mut Env, ins: &mut Inserter, uid: u32) -> (i64, u32) {
    let items = FetchItems { flags: true, internal_date: true, size: true, header: true, ..FetchItems::default() };
    let f = s.uid_fetch(&Uids::Set(UidSet::from_uids([uid])), &items).await.unwrap().remove(0);
    let headers = parse_headers(f.header.as_deref().unwrap());
    let flags = f.flags.unwrap_or_default();
    let m = NewMessage {
        account_id: 1,
        folder_id: 1,
        uid: Some(uid),
        date_received: f.internal_date.unwrap(),
        size: f.size.map(u64::from),
        modseq: f.modseq,
        flags: Flags { seen: flags.iter().any(|f| f == "\\Seen"), flagged: flags.iter().any(|f| f == "\\Flagged"), ..Flags::default() },
        headers: &headers,
    };
    let mut changes = ChangeSet::default();
    let tx = env.store.transaction().unwrap();
    let id = ins.insert(&tx, &m, None, &mut changes).unwrap();
    ins.commit(tx, &mut changes).unwrap();
    (id, f.size.unwrap())
}

#[tokio::test]
#[ignore = "needs the test servers: make imap-up, then make imap-test"]
async fn headers_then_a_large_body_streamed_from_the_server() {
    let user = format!("t{:016x}", rand::random::<u64>());
    let big = big_mail();
    let mut s = open(31993, &user).await;
    let uid = s.append(&MailboxName::inbox(), &["\\Flagged"], None, big.clone()).await.unwrap().unwrap().uid;
    s.select(&MailboxName::inbox(), None).await.unwrap();

    let mut env = Env::new();
    let mut ins = Inserter::new(Source::Imap, None);
    let (id, size) = insert_headers(&mut s, &mut env, &mut ins, uid).await;
    assert_eq!(env.body_state(id), 0);
    let subject: String = env.store.conn().query_row("SELECT subject FROM message WHERE id = ?1", [id], |r| r.get(0)).unwrap();
    assert_eq!(subject, "Les plans");

    // Step 2: the body, piece by piece into the raw store.
    let mut w = env.raw.writer(true).unwrap();
    let got = s.fetch_body(uid, size, &mut |piece: &[u8]| w.write(piece)).await.unwrap().unwrap();
    assert_eq!(got, Some(big.len() as u64));
    let f = w.finish(Some(size as u64)).unwrap();
    let plain = f.plain.unwrap();
    assert!(matches!(plain, PlainBytes::Mapped(_)));
    let parsed = parse(&plain);
    let mut changes = ChangeSet::default();
    let tx = env.store.transaction().unwrap();
    assert_eq!(ins.complete(&tx, id, Body { written: &f.written, parsed: &parsed }, &mut changes).unwrap(), Completed::Done);
    ins.commit(tx, &mut changes).unwrap();

    assert_eq!(env.body_state(id), 1);
    assert!(env.raw.read(&f.written.hash).unwrap() == big, "stored byte for byte");
    let (flagged, attachment): (i64, i64) = env
        .store
        .conn()
        .query_row("SELECT m.is_flagged, a.size FROM message m JOIN attachment a ON a.message_id = m.id WHERE m.id = ?1", [id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!((flagged, attachment), (1, 300_000 * 57));
    assert_eq!(env.tmp_files(), 0);
    s.logout().await;
}

/// A TCP relay to the full server that cuts both sides after `cut_after`
/// bytes from the server.
async fn cutting_proxy(cut_after: usize) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (client, _) = listener.accept().await.unwrap();
        let server = TcpStream::connect("127.0.0.1:31993").await.unwrap();
        let (mut cr, mut cw) = client.into_split();
        let (mut sr, mut sw) = server.into_split();
        let up = tokio::spawn(async move {
            let _ = tokio::io::copy(&mut cr, &mut sw).await;
        });
        let mut relayed = 0;
        let mut buf = vec![0; 16 * 1024];
        while relayed < cut_after {
            let n = match sr.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => n.min(cut_after - relayed),
            };
            if cw.write_all(&buf[..n]).await.is_err() {
                break;
            }
            relayed += n;
        }
        up.abort();
    });
    port
}

#[tokio::test]
#[ignore = "needs the test servers: make imap-up, then make imap-test"]
async fn a_cut_in_the_middle_of_a_body_stores_nothing_and_the_next_try_does() {
    let user = format!("t{:016x}", rand::random::<u64>());
    let big = big_mail();
    let mut s = open(31993, &user).await;
    let uid = s.append(&MailboxName::inbox(), &[], None, big.clone()).await.unwrap().unwrap().uid;
    s.select(&MailboxName::inbox(), None).await.unwrap();
    let mut env = Env::new();
    let mut ins = Inserter::new(Source::Imap, None);
    let (id, size) = insert_headers(&mut s, &mut env, &mut ins, uid).await;
    s.logout().await;

    // Cut after 8 MB: several pieces of 1 MB have reached the writer.
    let mut s = open(cutting_proxy(8 << 20).await, &user).await;
    s.select(&MailboxName::inbox(), None).await.unwrap();
    let mut w = env.raw.writer(true).unwrap();
    let err = s.fetch_body(uid, size, &mut |piece: &[u8]| w.write(piece)).await.unwrap_err();
    assert!(err.is_fatal(), "{err:?}");
    assert!(w.size() > 0, "some pieces arrived before the cut");
    assert!(w.size() < size as u64);
    // The synchronization gives up on this body: the writer is dropped.
    drop(w);
    assert_eq!(env.tmp_files(), 0, "no partial file");
    assert_eq!(env.body_state(id), 0, "still waiting for its body");

    // The next connection completes it.
    let mut s = open(31993, &user).await;
    s.select(&MailboxName::inbox(), None).await.unwrap();
    let mut w = env.raw.writer(true).unwrap();
    s.fetch_body(uid, size, &mut |piece: &[u8]| w.write(piece)).await.unwrap().unwrap();
    let f = w.finish(Some(size as u64)).unwrap();
    let parsed = parse(&f.plain.unwrap());
    let mut changes = ChangeSet::default();
    let tx = env.store.transaction().unwrap();
    assert_eq!(ins.complete(&tx, id, Body { written: &f.written, parsed: &parsed }, &mut changes).unwrap(), Completed::Done);
    ins.commit(tx, &mut changes).unwrap();
    assert!(env.raw.read(&f.written.hash).unwrap() == big);
    s.logout().await;
}
