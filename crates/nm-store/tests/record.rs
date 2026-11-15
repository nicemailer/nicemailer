//! Integration tests of `record.rs`: the record of a message printed as
//! evidence (`message_record`, the PDF of `sp`).

use std::sync::Mutex;

use nm_core::Source;
use nm_store::import::{ImportOptions, import_mbox};
use nm_store::raw::{self, RawStore};
use nm_store::{Store, StoreError};

/// A store holding the single mail `mail`, imported from an mbox into an
/// archive account whose address is `me@x.example`, and its message id.
fn with_mail(mail: &str) -> (tempfile::TempDir, RawStore, Store, i64) {
    let dir = tempfile::tempdir().unwrap();
    let raw = RawStore::open(dir.path()).unwrap();
    let store = Mutex::new(Store::open_in_memory().unwrap());
    let mbox = dir.path().join("a.mbox");
    std::fs::write(&mbox, format!("From a Tue Jun 23 19:20:56 2020\n{mail}")).unwrap();
    let opts = ImportOptions {
        path: mbox,
        account: "arch".into(),
        address: "me@x.example".into(),
        mark_correspondents: false,
        source: Source::Cli,
        batch_size: 10,
        lock_budget: nm_store::import::LOCK_BUDGET,
    };
    import_mbox(&store, &raw, &opts, &mut |_, _, _| {}).unwrap();
    let store = store.into_inner().unwrap();
    let id: i64 = store.conn().query_row("SELECT id FROM message", [], |r| r.get(0)).unwrap();
    (dir, raw, store, id)
}

const MAIL: &str = "Return-Path: <bounce@a.example>\n\
Received: from mx.a.example by mx.x.example; Tue, 23 Jun 2020 19:20:57 +0000\n\
Received: from laptop by mx.a.example;\n\tTue, 23 Jun 2020 19:20:56 +0000\n\
Authentication-Results: mx.x.example; dkim=pass header.d=a.example; spf=pass\n\
DKIM-Signature: v=1; a=rsa-sha256; d=a.example; s=sel;\n b=abc\n\
Subject: =?UTF-8?Q?Mise_en_demeure_=E2=80=93_loyer?=\n\
From: \"Alice Martin\" <Alice@A.example>\n\
Reply-To: Cabinet <avocat@c.example>\n\
To: Moi <me@x.example>, \"Bob Durand\" <bob@b.example>\n\
Cc: carol@c.example\n\
Date: Tue, 23 Jun 2020 21:20:56 +0200\n\
Message-ID: <1@a.example>\n\
In-Reply-To: <0@x.example>\n\
References: <00@x.example> <0@x.example>\n\
MIME-Version: 1.0\n\
Content-Type: multipart/mixed; boundary=\"b\"\n\
\n\
--b\n\
Content-Type: text/plain; charset=utf-8\n\
\n\
Madame, Monsieur,\n\
\n\
Je vous mets en demeure.\n\
--b\n\
Content-Type: application/pdf; name=\"bail.pdf\"\n\
Content-Disposition: attachment; filename=\"bail.pdf\"\n\
Content-Transfer-Encoding: base64\n\
\n\
JVBERi0xLjQK\n\
--b--\n";

#[test]
fn the_record_comes_from_the_raw_message() {
    let (_dir, raw, store, id) = with_mail(MAIL);
    let r = store.message_record(&raw, id).unwrap();

    assert_eq!(r.subject, "Mise en demeure – loyer");
    assert_eq!(r.message_id.as_deref(), Some("1@a.example"));
    assert_eq!(r.in_reply_to.as_deref(), Some("0@x.example"));
    assert_eq!(r.references, ["00@x.example", "0@x.example"]);
    // Names as written in this mail, addresses in lowercase.
    assert_eq!(r.from.len(), 1);
    assert_eq!(r.from[0].addr, "alice@a.example");
    assert_eq!(r.from[0].name.as_deref(), Some("Alice Martin"));
    let to: Vec<(&str, Option<&str>)> = r.to.iter().map(|a| (a.addr.as_str(), a.name.as_deref())).collect();
    assert_eq!(to, [("me@x.example", Some("Moi")), ("bob@b.example", Some("Bob Durand"))]);
    assert_eq!(r.cc[0].addr, "carol@c.example");
    assert_eq!(r.cc[0].name, None);
    assert_eq!(r.reply_to[0].addr, "avocat@c.example");
    assert!(r.bcc.is_empty());

    // The Date header as written, with its time zone, and both dates.
    assert_eq!(r.date_header.as_deref(), Some("Tue, 23 Jun 2020 21:20:56 +0200"));
    assert_eq!(r.date_sent, Some(1_592_940_056));
    assert_eq!(r.date_received, 1_592_940_056);
    assert_eq!(r.account, "arch");
    assert_eq!(r.body.as_deref(), Some("Madame, Monsieur,\n\nJe vous mets en demeure."));

    // The trace headers, in the order of `TRACE_HEADERS`, each unfolded.
    let names: Vec<&str> = r.headers.iter().map(|h| h.name.as_str()).collect();
    assert_eq!(names, ["Return-Path", "Received", "Received", "Authentication-Results", "DKIM-Signature"]);
    assert_eq!(r.headers[2].value, "from laptop by mx.a.example; Tue, 23 Jun 2020 19:20:56 +0000");
    assert_eq!(r.headers[4].value, "v=1; a=rsa-sha256; d=a.example; s=sel; b=abc");

    // The hash of the raw message is that of the bytes stored (the .eml).
    let mail_bytes = MAIL.as_bytes();
    assert_eq!(r.raw_sha256.as_deref(), Some(raw::hex(&raw::hash(mail_bytes)).as_str()));
    assert_eq!(r.raw_size, Some(mail_bytes.len() as i64));

    // The attachment hashed decoded: the file the recipient has.
    assert_eq!(r.attachments.len(), 1);
    let a = &r.attachments[0];
    assert_eq!(a.filename.as_deref(), Some("bail.pdf"));
    assert_eq!(a.mime_type, "application/pdf");
    assert_eq!(a.size, 9);
    assert_eq!(a.sha256.as_deref(), Some(raw::hex(&raw::hash(b"%PDF-1.4\n")).as_str()));
}

#[test]
fn several_authors_and_a_sender_are_all_kept() {
    let mail = "From: Alice <alice@a.example>, Bob <bob@b.example>\nSender: Secretariat <sec@a.example>\n\
To: me@x.example\nBcc: Archive <arch@x.example>\nSubject: s\nMessage-ID: <2@a>\n\nBody\n";
    let (_dir, raw, store, id) = with_mail(mail);
    let r = store.message_record(&raw, id).unwrap();
    let from: Vec<&str> = r.from.iter().map(|a| a.addr.as_str()).collect();
    assert_eq!(from, ["alice@a.example", "bob@b.example"]);
    let sender = r.sender.unwrap();
    assert_eq!((sender.addr.as_str(), sender.name.as_deref()), ("sec@a.example", Some("Secretariat")));
    assert_eq!(r.bcc[0].addr, "arch@x.example");
    // No Date header: none is invented.
    assert_eq!(r.date_header, None);
    assert_eq!(r.date_sent, None);
}

#[test]
fn without_a_raw_file_the_record_comes_from_the_database() {
    let (_dir, raw, store, id) = with_mail(MAIL);
    store.conn().execute("UPDATE message SET raw_hash = NULL WHERE id = ?1", [id]).unwrap();
    let r = store.message_record(&raw, id).unwrap();
    assert_eq!(r.raw_sha256, None);
    assert_eq!(r.subject, "Mise en demeure – loyer");
    assert_eq!(r.from[0].addr, "alice@a.example");
    assert_eq!(r.to.len(), 2);
    assert!(r.headers.is_empty());
    assert_eq!(r.attachments[0].sha256, None);
}

#[test]
fn a_damaged_raw_file_is_reported_and_an_unknown_message_refused() {
    let (_dir, raw, store, id) = with_mail(MAIL);
    assert!(matches!(store.message_record(&raw, 999), Err(StoreError::NotFound(_))));
    let hash: Vec<u8> = store.conn().query_row("SELECT raw_hash FROM message WHERE id = ?1", [id], |r| r.get(0)).unwrap();
    let path = raw.path(&hash.try_into().unwrap());
    std::fs::remove_file(path).unwrap();
    assert!(matches!(store.message_record(&raw, id), Err(StoreError::RawDamaged(_))));
}
