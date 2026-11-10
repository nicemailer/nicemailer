//! Integration tests of `reindex` (`stockage-brut.md`, section 8): the
//! bodies of an older parser are read again from their raw file and
//! rewritten; the others are left alone.
//!
//! The store is filled by a real import, then some rows are put back as an
//! older parser (version 5) left them.

use std::sync::Mutex;

use nm_core::Source;
use nm_store::import::{ImportOptions, import_mbox};
use nm_store::ingest::PARSER_VERSION;
use nm_store::raw::RawStore;
use nm_store::reindex::{ReindexReport, pending, reindex};
use nm_store::Store;

/// Three mails: a newsletter whose text part holds `&nbsp;` (and lacks the
/// paragraph "Vendredi"), a plain mail to Bob, and a plain mail whose raw
/// file will be lost.
const MBOX: &str = "From a Tue Jun 23 19:20:56 2020
From: Lettre <news@letter.example>
To: me@x.example
Subject: Une heure
Message-ID: <n1@letter.example>
Content-Type: multipart/alternative; boundary=\"B\"

--B
Content-Type: text/plain; charset=utf-8

Un titre.Le texte&nbsp;(suite)
--B
Content-Type: text/html; charset=utf-8

<p>Un titre.</p><p>Vendredi</p><p>Le texte&nbsp;(suite)</p>
--B--

From a Wed Jun 24 09:00:00 2020
From: me@x.example
To: Bob Martin <bob@b.example>
Subject: Bonjour
Message-ID: <p1@x.example>

Salut Bob

From a Thu Jun 25 09:00:00 2020
From: Carol <carol@c.example>
To: me@x.example
Subject: Perdu
Message-ID: <l1@c.example>

Le brut disparu.
";

struct Env {
    _dir: tempfile::TempDir,
    store: Mutex<Store>,
    raw: RawStore,
}

fn env() -> Env {
    env_with(MBOX)
}

/// A store filled by the import of `mbox`.
fn env_with(mbox: &str) -> Env {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.mbox");
    std::fs::write(&path, mbox).unwrap();
    let raw = RawStore::open(dir.path()).unwrap();
    let store = Mutex::new(Store::open_in_memory().unwrap());
    let opts = ImportOptions {
        path,
        account: "arch".into(),
        address: "me@x.example".into(),
        mark_correspondents: false,
        source: Source::Cli,
        batch_size: 10,
        lock_budget: nm_store::import::LOCK_BUDGET,
    };
    import_mbox(&store, &raw, &opts, &mut |_, _, _| {}).unwrap();
    Env { _dir: dir, store, raw }
}

impl Env {
    fn exec(&self, sql: &str) {
        self.store.lock().unwrap().conn().execute_batch(sql).unwrap();
    }
    fn text(&self, sql: &str) -> Option<String> {
        self.store.lock().unwrap().conn().query_row(sql, [], |r| r.get(0)).unwrap()
    }
    fn int(&self, sql: &str) -> i64 {
        self.store.lock().unwrap().conn().query_row(sql, [], |r| r.get(0)).unwrap()
    }
    fn run(&self) -> ReindexReport {
        let mut batches = vec![];
        let r = reindex(&self.store, &self.raw, 1, &mut |done, total| batches.push((done, total))).unwrap();
        // One batch per message (batch size 1), the last one complete.
        if let Some(last) = batches.last() {
            assert_eq!(last.0, last.1);
        }
        r
    }
}

/// Puts the stored data back as version 5 left it: the newsletter's text
/// part as written (entities included), the other bodies at version 5, the
/// body of the lost mail missing, its raw file deleted, and Bob without a
/// name.
fn as_version_5(e: &Env) {
    let old = "Un titre.Le texte&nbsp;(suite)";
    e.exec(&format!(
        "UPDATE message_body SET parser_version = 5;
         UPDATE message_body SET text_plain = '{old}'
          WHERE message_id = (SELECT id FROM message WHERE message_id_hdr = 'n1@letter.example');
         UPDATE message SET snippet = '{old}' WHERE message_id_hdr = 'n1@letter.example';
         UPDATE message_fts SET body = '{old}' WHERE rowid = (SELECT id FROM message WHERE message_id_hdr = 'n1@letter.example');
         DELETE FROM message_body WHERE message_id = (SELECT id FROM message WHERE message_id_hdr = 'l1@c.example');
         UPDATE address SET name = NULL WHERE addr = 'bob@b.example';"
    ));
    let hash: Vec<u8> = e
        .store
        .lock()
        .unwrap()
        .conn()
        .query_row("SELECT raw_hash FROM message WHERE message_id_hdr = 'l1@c.example'", [], |r| r.get(0))
        .unwrap();
    std::fs::remove_file(e.raw.path(&hash.try_into().unwrap())).unwrap();
}

#[test]
fn a_fresh_import_needs_no_reindex() {
    let e = env();
    assert_eq!(pending(&e.store.lock().unwrap()).unwrap(), 0);
    let r = e.run();
    assert_eq!(r, ReindexReport { seconds: r.seconds, ..Default::default() });
}

#[test]
fn older_bodies_are_read_again_and_the_others_only_marked() {
    let e = env();
    as_version_5(&e);
    assert_eq!(pending(&e.store.lock().unwrap()).unwrap(), 3);
    let r = e.run();
    // Bob's mail has no entity: marked, not reparsed. The newsletter is
    // reparsed; the lost mail (no body, no raw) is counted as missing.
    assert_eq!((r.unchanged, r.reparsed, r.missing_raw, r.parse_errors), (1, 1, 1, 0));
    let text = e.text("SELECT b.text_plain FROM message_body b JOIN message m ON m.id = b.message_id
                        WHERE m.message_id_hdr = 'n1@letter.example'").unwrap();
    assert!(!text.contains("&nbsp;") && text.contains("Vendredi"), "{text}");
    let snippet = e.text("SELECT snippet FROM message WHERE message_id_hdr = 'n1@letter.example'").unwrap();
    assert!(!snippet.contains("&nbsp;"), "{snippet}");
    // The search index finds the paragraph the text part lacked.
    assert_eq!(e.int("SELECT count(*) FROM message_fts WHERE message_fts MATCH 'vendredi'"), 1);
    // Every body with a raw file is now current; the lost one stays to do.
    assert_eq!(e.int(&format!("SELECT count(*) FROM message_body WHERE parser_version = {PARSER_VERSION}")), 2);
    assert_eq!(pending(&e.store.lock().unwrap()).unwrap(), 1);
    // A second run reads only what is still missing.
    let again = e.run();
    assert_eq!((again.reparsed, again.missing_raw), (0, 1));
}

#[test]
fn a_missing_body_is_created_and_recipients_give_their_names() {
    let e = env();
    as_version_5(&e);
    // Bob's mail is reparsed too when its text holds an entity.
    e.exec(
        "UPDATE message_body SET text_plain = 'Tom &amp; Jerry'
          WHERE message_id = (SELECT id FROM message WHERE message_id_hdr = 'p1@x.example');
         DELETE FROM message_body WHERE message_id = (SELECT id FROM message WHERE message_id_hdr = 'n1@letter.example');",
    );
    let r = e.run();
    assert_eq!(r.reparsed, 2);
    assert!(e.text("SELECT b.text_plain FROM message_body b JOIN message m ON m.id = b.message_id
                     WHERE m.message_id_hdr = 'n1@letter.example'").unwrap().contains("Vendredi"));
    // The plain mail is back as written (no entity in its raw text part).
    assert_eq!(
        e.text("SELECT b.text_plain FROM message_body b JOIN message m ON m.id = b.message_id
                 WHERE m.message_id_hdr = 'p1@x.example'").unwrap().trim_end(),
        "Salut Bob"
    );
    assert_eq!(e.text("SELECT name FROM address WHERE addr = 'bob@b.example'").as_deref(), Some("Bob Martin"));
    assert_eq!(r.names_filled, 1);
}

#[test]
fn a_name_set_by_the_user_is_kept() {
    let e = env();
    as_version_5(&e);
    e.exec(
        "UPDATE address SET name = 'Bobby' WHERE addr = 'bob@b.example';
         UPDATE message_body SET text_plain = 'Tom &amp; Jerry'
          WHERE message_id = (SELECT id FROM message WHERE message_id_hdr = 'p1@x.example');",
    );
    let r = e.run();
    assert_eq!(r.names_filled, 0);
    assert_eq!(e.text("SELECT name FROM address WHERE addr = 'bob@b.example'").as_deref(), Some("Bobby"));
}

#[test]
fn a_body_read_again_but_unchanged_is_not_rewritten() {
    // Bob's mail with "&x;" in its text is reread (the choice is wide), but
    // the parser gives the same text: its body and its row of the search
    // index are left as they are (a marker put in the index stays).
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.mbox");
    std::fs::write(&path, "From a Wed Jun 24 09:00:00 2020\nFrom: me@x.example\nTo: bob@b.example\nSubject: s\nMessage-ID: <q@x>\n\nAT&T; R&D\n").unwrap();
    let raw = RawStore::open(dir.path()).unwrap();
    let store = Mutex::new(Store::open_in_memory().unwrap());
    let opts = ImportOptions {
        path,
        account: "arch".into(),
        address: "me@x.example".into(),
        mark_correspondents: false,
        source: Source::Cli,
        batch_size: 10,
        lock_budget: nm_store::import::LOCK_BUDGET,
    };
    import_mbox(&store, &raw, &opts, &mut |_, _, _| {}).unwrap();
    store.lock().unwrap().conn().execute_batch("UPDATE message_body SET parser_version = 5; UPDATE message_fts SET body = 'marqueur';").unwrap();
    let r = reindex(&store, &raw, 10, &mut |_, _| {}).unwrap();
    assert_eq!(r.reparsed, 1);
    let guard = store.lock().unwrap();
    let body: String = guard.conn().query_row("SELECT body FROM message_fts", [], |r| r.get(0)).unwrap();
    assert_eq!(body, "marqueur");
    let version: i64 = guard.conn().query_row("SELECT parser_version FROM message_body", [], |r| r.get(0)).unwrap();
    assert_eq!(version, PARSER_VERSION);
}

#[test]
fn entities_encoded_twice_are_repaired_from_what_is_stored() {
    let e = env();
    // A version 6 body as a sender's double encoding left it: "&nbsp;" in
    // the text, "&amp;nbsp;" in the HTML. Its raw file is gone: the repair
    // does not need it.
    e.exec(
        "UPDATE message_body SET parser_version = 6;
         UPDATE message_body SET text_plain = 'Zoom :&nbsp;https://zoom.us/j/1',
                                 html_sanitized = '<p>Zoom :&amp;nbsp;<a href=\"https://z.us/?a=1&amp;b=2\">x</a></p>'
          WHERE message_id = (SELECT id FROM message WHERE message_id_hdr = 'p1@x.example');
         UPDATE message_fts SET body = 'Zoom :&nbsp;https://zoom.us/j/1'
          WHERE rowid = (SELECT id FROM message WHERE message_id_hdr = 'p1@x.example');",
    );
    let hashes: Vec<Vec<u8>> = e.store.lock().unwrap().conn().prepare("SELECT raw_hash FROM message").unwrap()
        .query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect();
    for h in hashes {
        let _ = std::fs::remove_file(e.raw.path(&h.try_into().unwrap()));
    }
    let r = e.run();
    // Repaired from what is stored; but version 9 (layout attributes)
    // needs the raw file of each HTML body, gone here: those stay at
    // version 8, counted as missing.
    assert_eq!((r.repaired, r.reparsed, r.missing_raw), (1, 0, 2));
    let q = |col: &str| e.text(&format!(
        "SELECT {col} FROM message_body WHERE message_id = (SELECT id FROM message WHERE message_id_hdr = 'p1@x.example')"));
    assert_eq!(q("text_plain").as_deref(), Some("Zoom : https://zoom.us/j/1"));
    let html = q("html_sanitized").unwrap();
    assert!(html.contains("Zoom :&nbsp;<a") && html.contains("a=1&amp;b=2"), "{html}");
    assert_eq!(e.text("SELECT snippet FROM message WHERE message_id_hdr = 'p1@x.example'").as_deref(), Some("Zoom : https://zoom.us/j/1"));
    assert_eq!(e.int("SELECT count(*) FROM message_fts WHERE body = 'Zoom : https://zoom.us/j/1'"), 1);
    // The text body is current; the two HTML bodies wait for their raw.
    assert_eq!(e.int(&format!("SELECT count(*) FROM message_body WHERE parser_version = {PARSER_VERSION}")), 1);
    assert_eq!(e.int("SELECT count(*) FROM message_body WHERE parser_version = 8"), 2);
    assert_eq!(pending(&e.store.lock().unwrap()).unwrap(), 2);
}

/// Two HTML mails: a newsletter with a remote banner and a tracking pixel,
/// and a mail without any remote image.
const MBOX_IMAGES: &str = "From a Tue Jun 23 19:20:56 2020
From: Lettre <news@letter.example>
To: me@x.example
Subject: Images
Message-ID: <i1@letter.example>
Content-Type: text/html; charset=utf-8

<p>Hello</p><img src=\"https://img.letter.example/banner.png\" width=\"600\"><img src=\"https://t.letter.example/open?u=1\" width=\"1\" height=\"1\">

From a Wed Jun 24 09:00:00 2020
From: Bob <bob@b.example>
To: me@x.example
Subject: Sans image
Message-ID: <i2@b.example>
Content-Type: text/html; charset=utf-8

<p>Rien de distant</p>
";

#[test]
fn version_8_reads_again_only_the_bodies_with_remote_images() {
    let e = env_with(MBOX_IMAGES);
    // As version 7 stored them: the remote image without its source, no
    // count of pixels.
    e.exec(
        "UPDATE message_body SET parser_version = 7, tracking_pixels = 0;
         UPDATE message_body SET html_sanitized = '<p>Hello</p><img width=\"600\"><img width=\"1\" height=\"1\">'
          WHERE message_id = (SELECT id FROM message WHERE message_id_hdr = 'i1@letter.example');",
    );
    assert_eq!(pending(&e.store.lock().unwrap()).unwrap(), 2);
    let r = e.run();
    // Version 8 alone would only mark the mail without remote image; but
    // both are HTML, which version 9 reads again.
    assert_eq!((r.unchanged, r.reparsed, r.missing_raw), (0, 2, 0));
    let q = |col: &str| e.text(&format!(
        "SELECT {col} FROM message_body WHERE message_id = (SELECT id FROM message WHERE message_id_hdr = 'i1@letter.example')"));
    let html = q("html_sanitized").unwrap();
    assert!(html.contains("src=\"nm-remote://image?u=https%3A%2F%2Fimg.letter.example%2Fbanner.png\""), "{html}");
    assert!(!html.contains("t.letter.example"), "{html}");
    assert_eq!(e.int("SELECT tracking_pixels FROM message_body WHERE message_id = (SELECT id FROM message WHERE message_id_hdr = 'i1@letter.example')"), 1);
    assert_eq!(e.int("SELECT has_remote_content FROM message_body WHERE message_id = (SELECT id FROM message WHERE message_id_hdr = 'i1@letter.example')"), 1);
    assert_eq!(e.int(&format!("SELECT count(*) FROM message_body WHERE parser_version = {PARSER_VERSION}")), 2);
    assert_eq!(pending(&e.store.lock().unwrap()).unwrap(), 0);
}

/// A text mail and an HTML newsletter laid out with a table.
const MBOX_LAYOUT: &str = "From a Tue Jun 23 19:20:56 2020
From: Lettre <news@letter.example>
To: me@x.example
Subject: Mise en page
Message-ID: <l1@letter.example>
Content-Type: text/html; charset=utf-8

<table width=\"600\" cellpadding=\"12\" align=\"center\"><tr><td valign=\"top\"><font face=\"Georgia\">Bonjour</font></td></tr></table>

From a Wed Jun 24 09:00:00 2020
From: Bob <bob@b.example>
To: me@x.example
Subject: Texte
Message-ID: <l2@b.example>
Content-Type: text/plain; charset=utf-8

Rien que du texte
";

#[test]
fn version_9_reads_again_only_the_html_bodies() {
    let e = env_with(MBOX_LAYOUT);
    // As version 8 stored them: the table without its layout attributes.
    e.exec(
        "UPDATE message_body SET parser_version = 8;
         UPDATE message_body SET html_sanitized = '<table align=\"center\"><tr><td>Bonjour</td></tr></table>'
          WHERE message_id = (SELECT id FROM message WHERE message_id_hdr = 'l1@letter.example');",
    );
    assert_eq!(pending(&e.store.lock().unwrap()).unwrap(), 2);
    let r = e.run();
    // The text mail is only marked.
    assert_eq!((r.unchanged, r.reparsed, r.missing_raw), (1, 1, 0));
    let html = e
        .text("SELECT html_sanitized FROM message_body WHERE message_id = (SELECT id FROM message WHERE message_id_hdr = 'l1@letter.example')")
        .unwrap();
    for kept in [r#"width="600""#, r#"cellpadding="12""#, r#"valign="top""#, r#"<font face="Georgia">"#] {
        assert!(html.contains(kept), "{kept} missing in {html}");
    }
    assert_eq!(e.int(&format!("SELECT count(*) FROM message_body WHERE parser_version = {PARSER_VERSION}")), 2);
    assert_eq!(pending(&e.store.lock().unwrap()).unwrap(), 0);
}
