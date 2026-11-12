//! Integration tests of the mbox import (`import-initial.md`, section 9).
//!
//! Each test writes a small mbox into a temporary directory, imports it into
//! an in-memory store and checks the database through public queries and a
//! few direct SQL reads.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use nm_core::{FolderRole, Source};
use nm_store::screening::MoveOptions;
use nm_store::import::{ImportOptions, ImportReport, MAX_DELIVERY_DELAY_SECS, import_mbox, reception_date};
use nm_store::raw::RawStore;
use nm_store::{ChangeSet, Store, StoreError};

const ME: &str = "me@hey.example";

/// One message of a test mbox. `headers` are full header lines.
struct Mail {
    from_line_date: &'static str,
    headers: Vec<String>,
    body: String,
}

fn mail(date: &'static str, headers: &[&str], body: &str) -> Mail {
    Mail { from_line_date: date, headers: headers.iter().map(|h| h.to_string()).collect(), body: body.to_string() }
}

/// Builds an mboxrd file from `mails`, escaping `From ` lines in bodies.
fn mbox(mails: &[Mail]) -> Vec<u8> {
    let mut out = String::new();
    for m in mails {
        out.push_str(&format!("From sender@x {}\n", m.from_line_date));
        for h in &m.headers {
            out.push_str(h);
            out.push('\n');
        }
        out.push('\n');
        for line in m.body.lines() {
            let gt = line.chars().take_while(|c| *c == '>').count();
            if line[gt..].starts_with("From ") {
                out.push('>');
            }
            out.push_str(line);
            out.push('\n');
        }
        out.push('\n');
    }
    out.into_bytes()
}

/// A test environment: temporary data directory, store and raw storage.
struct Env {
    dir: tempfile::TempDir,
    store: Mutex<Store>,
    raw: RawStore,
}

impl Env {
    fn new() -> Env {
        let dir = tempfile::tempdir().unwrap();
        let raw = RawStore::open(dir.path()).unwrap();
        Env { store: Mutex::new(Store::open_in_memory().unwrap()), raw, dir }
    }

    fn write_mbox(&self, name: &str, mails: &[Mail]) -> PathBuf {
        let path = self.dir.path().join(name);
        std::fs::write(&path, mbox(mails)).unwrap();
        path
    }

    fn options(&self, path: &Path) -> ImportOptions {
        ImportOptions {
            path: path.to_path_buf(),
            account: "hey".into(),
            address: ME.into(),
            mark_correspondents: true,
            source: Source::Cli,
            // Small batches so that tests cross batch boundaries.
            batch_size: 2,
            lock_budget: nm_store::import::LOCK_BUDGET,
        }
    }

    fn import(&self, opts: &ImportOptions) -> Result<(ImportReport, Vec<ChangeSet>), StoreError> {
        let mut batches = vec![];
        let report = import_mbox(&self.store, &self.raw, opts, &mut |_, changes, _| batches.push(changes.clone()))?;
        Ok((report, batches))
    }

    fn count(&self, sql: &str) -> i64 {
        self.store.lock().unwrap().conn().query_row(sql, [], |r| r.get(0)).unwrap()
    }

    fn text(&self, sql: &str) -> Option<String> {
        self.store.lock().unwrap().conn().query_row(sql, [], |r| r.get(0)).unwrap()
    }
}

/// A small mailbox: a conversation (with the reply placed *before* the
/// message it answers, as can happen in an export), an HTML newsletter, and
/// a sent message with an attachment.
fn sample() -> Vec<Mail> {
    vec![
        mail(
            "Wed Jun 24 09:00:00 2020",
            &[
                "From: Me <ME@hey.example>",
                "To: Alice <alice@a.example>, Bob <bob@b.example>",
                "Cc: Me <me@hey.example>",
                "Subject: Re: Devis",
                "Message-ID: <m2@hey.example>",
                "In-Reply-To: <m1@a.example>",
                "References: <m1@a.example>",
                "Date: Wed, 24 Jun 2020 11:00:00 +0200",
            ],
            "OK pour moi.\n\nFrom my phone",
        ),
        mail(
            "Tue Jun 23 19:20:56 2020",
            &[
                "From: \"Alice Martin\" <alice@a.example>",
                "To: me@hey.example",
                "Subject: Devis",
                "Message-ID: <m1@a.example>",
                "Date: Tue, 23 Jun 2020 21:20:56 +0200",
            ],
            "Bonjour,\nvoici le devis pour la rénovation.",
        ),
        mail(
            "Thu Jun 25 07:00:00 2020",
            &[
                "From: Lettre <news@letter.example>",
                "To: me@hey.example",
                "Subject: La lettre",
                "Message-ID: <n1@letter.example>",
                "Content-Type: text/html; charset=utf-8",
            ],
            "<p>Nouveaut\u{e9}s <img src=\"https://img.example/p.gif\"><img src=\"https://track.example/o.gif\" width=\"1\" height=\"1\"></p><script>x()</script>",
        ),
        mail(
            "Fri Jun 26 08:00:00 2020",
            &[
                "From: me@hey.example",
                "To: carol@c.example",
                "Subject: Facture",
                "Message-ID: <m3@hey.example>",
                "Content-Type: multipart/mixed; boundary=\"B\"",
            ],
            "--B\nContent-Type: text/plain\n\nCi-joint.\n--B\nContent-Type: application/pdf\nContent-Disposition: attachment; filename=\"facture.pdf\"\nContent-Transfer-Encoding: base64\n\nJVBERi0xLjQK\n--B--",
        ),
    ]
}

#[test]
fn creates_an_archive_account_with_every_workflow_folder() {
    let env = Env::new();
    let path = env.write_mbox("a.mbox", &sample());
    let (report, batches) = env.import(&env.options(&path)).unwrap();
    assert_eq!(report.progress.imported, 4);
    assert_eq!(env.count("SELECT count(*) FROM account WHERE kind = 'archive' AND address = 'me@hey.example'"), 1);
    let folders = env.store.lock().unwrap().list_folders().unwrap();
    assert_eq!(folders.len(), FolderRole::ALL.len());
    assert!(folders.iter().all(|f| f.account_kind == "archive" && f.account_name == "hey"));
    // 4 messages, batches of 2: two batches, then the final step.
    assert_eq!(batches.len(), 3);
    assert_eq!(report.progress.bytes_read, report.progress.total_bytes);
}

/// A batch that writes slowly is cut at `lock_budget`, so that the store
/// lock is released for the synchronization and the app (`import.rs`,
/// `LOCK_BUDGET`). With a budget of zero, every message is its own batch,
/// and the result is the same as in large batches: same order, same
/// threads.
#[test]
fn a_slow_batch_is_cut_at_the_time_limit_and_the_rest_follows() {
    let env = Env::new();
    let path = env.write_mbox("a.mbox", &sample());
    let opts = ImportOptions { batch_size: 500, lock_budget: std::time::Duration::ZERO, ..env.options(&path) };
    let (report, batches) = env.import(&opts).unwrap();
    assert_eq!(report.progress.imported, 4);
    // One batch per message, at least one message each, then the final step.
    assert_eq!(batches.len(), 5);
    // Written in the order of the file.
    let order: Vec<String> = {
        let store = env.store.lock().unwrap();
        let mut stmt = store.conn().prepare("SELECT message_id_hdr FROM message ORDER BY id").unwrap();
        stmt.query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect()
    };
    assert_eq!(order, ["m2@hey.example", "m1@a.example", "n1@letter.example", "m3@hey.example"]);
    assert_eq!(env.count("SELECT count(*) FROM thread"), 3);
    assert_eq!(env.count("SELECT count(DISTINCT thread_id) FROM message WHERE message_id_hdr IN ('m1@a.example', 'm2@hey.example')"), 1);

    // With the production limit, a small file is one batch.
    let env = Env::new();
    let path = env.write_mbox("a.mbox", &sample());
    let opts = ImportOptions { batch_size: 500, ..env.options(&path) };
    assert_eq!(env.import(&opts).unwrap().1.len(), 2);
}

#[test]
fn files_sent_messages_in_sent_and_the_rest_in_archive_all_read() {
    let env = Env::new();
    let path = env.write_mbox("a.mbox", &sample());
    let (report, _) = env.import(&env.options(&path)).unwrap();
    assert_eq!((report.sent, report.archived), (2, 2));
    // The From address is compared case-insensitively ("ME@hey.example").
    let sent = env.count(
        "SELECT count(*) FROM message m JOIN folder f ON f.id = m.folder_id WHERE f.role = 'sent' AND m.message_id_hdr IN ('m2@hey.example', 'm3@hey.example')",
    );
    assert_eq!(sent, 2);
    assert_eq!(env.count("SELECT count(*) FROM message m JOIN folder f ON f.id = m.folder_id WHERE f.role = 'archive'"), 2);
    assert_eq!(env.count("SELECT count(*) FROM message WHERE is_seen = 0"), 0);
    // No server: no UID, unknown authentication, created by the CLI.
    assert_eq!(env.count("SELECT count(*) FROM message WHERE uid IS NOT NULL OR auth_result <> 'unknown' OR created_by <> 'cli'"), 0);
    // Folder counters match the messages.
    let folders = env.store.lock().unwrap().list_folders().unwrap();
    let archive = folders.iter().find(|f| f.role == Some(FolderRole::Archive)).unwrap();
    assert_eq!((archive.total_count, archive.unread_count), (2, 0));
}

#[test]
fn threads_a_reply_imported_before_its_parent() {
    let env = Env::new();
    let path = env.write_mbox("a.mbox", &sample());
    env.import(&env.options(&path)).unwrap();
    let threads = env.count(
        "SELECT count(DISTINCT thread_id) FROM message WHERE message_id_hdr IN ('m1@a.example', 'm2@hey.example')",
    );
    assert_eq!(threads, 1);
    // Four messages, three conversations; the subject is the oldest one's.
    assert_eq!(env.count("SELECT count(*) FROM thread"), 3);
    assert_eq!(
        env.text("SELECT t.subject FROM thread t JOIN message m ON m.thread_id = t.id WHERE m.message_id_hdr = 'm2@hey.example'"),
        Some("Devis".into())
    );
    // The conversation is in Archive, flagged as having a mail in Sent.
    assert_eq!(
        env.text("SELECT t.folder FROM thread t JOIN message m ON m.thread_id = t.id WHERE m.message_id_hdr = 'm1@a.example'"),
        Some("a".into())
    );
    assert_eq!(
        env.count("SELECT t.has_sent FROM thread t JOIN message m ON m.thread_id = t.id WHERE m.message_id_hdr = 'm1@a.example'"),
        1
    );
    // The received message answered by the user is marked as answered.
    assert_eq!(env.count("SELECT is_answered FROM message WHERE message_id_hdr = 'm1@a.example'"), 1);
}

#[test]
fn merges_two_threads_linked_by_a_later_message() {
    // a and b are unrelated until c references both.
    let mails = vec![
        mail("Mon Jun  1 10:00:00 2020", &["From: x@x.example", "Subject: a", "Message-ID: <a@x>"], "a"),
        mail("Mon Jun  1 11:00:00 2020", &["From: y@y.example", "Subject: b", "Message-ID: <b@y>"], "b"),
        mail("Mon Jun  1 12:00:00 2020", &["From: z@z.example", "Subject: c", "Message-ID: <c@z>", "References: <a@x> <b@y>"], "c"),
    ];
    let env = Env::new();
    let path = env.write_mbox("m.mbox", &mails);
    env.import(&env.options(&path)).unwrap();
    assert_eq!(env.count("SELECT count(*) FROM thread"), 1);
    assert_eq!(env.count("SELECT message_count FROM thread"), 3);
    assert_eq!(env.text("SELECT folder FROM thread"), Some("a".into()));
}

#[test]
fn reimporting_the_same_file_adds_nothing() {
    let env = Env::new();
    let path = env.write_mbox("a.mbox", &sample());
    env.import(&env.options(&path)).unwrap();
    let (again, _) = env.import(&env.options(&path)).unwrap();
    assert_eq!((again.progress.imported, again.progress.skipped), (0, 4));
    assert_eq!(env.count("SELECT count(*) FROM message"), 4);
    assert_eq!(env.count("SELECT count(*) FROM raw_blob"), 4);
    // One audit line per run, plus one for the account creation.
    assert_eq!(env.count("SELECT count(*) FROM audit_log WHERE entity = 'account' AND action = 'import_mbox'"), 2);
    assert_eq!(env.count("SELECT count(*) FROM audit_log WHERE entity = 'account' AND action = 'create'"), 1);
}

#[test]
fn keeps_every_raw_message_byte_for_byte() {
    let env = Env::new();
    let mails = sample();
    let path = env.write_mbox("a.mbox", &mails);
    env.import(&env.options(&path)).unwrap();
    let hashes: Vec<Vec<u8>> = {
        let store = env.store.lock().unwrap();
        let mut stmt = store.conn().prepare("SELECT raw_hash FROM message WHERE message_id_hdr = 'm2@hey.example'").unwrap();
        stmt.query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect()
    };
    let hash: [u8; 32] = hashes[0].clone().try_into().unwrap();
    let bytes = env.raw.read(&hash).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    // Unescaped ("From my phone" was written ">From my phone" in the mbox)
    // and without the separator line.
    assert!(text.starts_with("From: Me <ME@hey.example>\n"));
    assert!(text.ends_with("OK pour moi.\n\nFrom my phone\n"));
    assert_eq!(env.count("SELECT count(*) FROM message WHERE body_state = 1 AND raw_hash IS NOT NULL"), 4);
}

#[test]
fn stores_text_sanitized_html_and_attachments() {
    let env = Env::new();
    let path = env.write_mbox("a.mbox", &sample());
    env.import(&env.options(&path)).unwrap();
    let html = env
        .text("SELECT b.html_sanitized FROM message_body b JOIN message m ON m.id = b.message_id WHERE m.message_id_hdr = 'n1@letter.example'")
        .unwrap();
    // The pixel is gone; the image keeps its URL in the app's scheme, which
    // no web view loads by itself (`remote_images.rs`).
    assert!(!html.contains("script") && !html.contains("track.example"), "{html}");
    assert!(html.contains("src=\"nm-remote://image?u=https%3A%2F%2Fimg.example%2Fp.gif\""), "{html}");
    assert_eq!(
        env.count("SELECT b.has_remote_content FROM message_body b JOIN message m ON m.id = b.message_id WHERE m.message_id_hdr = 'n1@letter.example'"),
        1
    );
    assert_eq!(
        env.count("SELECT b.tracking_pixels FROM message_body b JOIN message m ON m.id = b.message_id WHERE m.message_id_hdr = 'n1@letter.example'"),
        1
    );
    let text = env
        .text("SELECT b.text_plain FROM message_body b JOIN message m ON m.id = b.message_id WHERE m.message_id_hdr = 'n1@letter.example'")
        .unwrap();
    assert!(text.contains("Nouveautés"));
    assert_eq!(env.count("SELECT count(*) FROM attachment WHERE filename = 'facture.pdf' AND part_id = '2'"), 1);
    assert_eq!(env.count("SELECT has_attachments FROM message WHERE message_id_hdr = 'm3@hey.example'"), 1);
}

#[test]
fn imported_messages_are_searchable_by_body_and_recipient() {
    let env = Env::new();
    let path = env.write_mbox("a.mbox", &sample());
    env.import(&env.options(&path)).unwrap();
    let store = env.store.lock().unwrap();
    // Accent-insensitive search on the body.
    assert_eq!(store.search("renovation", 10).unwrap().len(), 1);
    // Search on a recipient of a sent message.
    assert_eq!(store.search("carol", 10).unwrap().len(), 1);
}

#[test]
fn new_addresses_are_marked_imported_and_existing_ones_kept() {
    let env = Env::new();
    // Bob is already known (normal reception: imported_from NULL).
    env.store
        .lock()
        .unwrap()
        .conn()
        .execute("INSERT INTO address (addr, created_by, modified_by) VALUES ('bob@b.example', 'core', 'core')", [])
        .unwrap();
    let path = env.write_mbox("a.mbox", &sample());
    env.import(&env.options(&path)).unwrap();
    assert_eq!(env.text("SELECT imported_from FROM address WHERE addr = 'bob@b.example'"), None);
    assert_eq!(env.text("SELECT imported_from FROM address WHERE addr = 'alice@a.example'"), Some("mbox".into()));
    // The name comes from the first message of the file that mentions the
    // address ("To: Alice <…>"), not from a later one ("Alice Martin").
    assert_eq!(env.text("SELECT name FROM address WHERE addr = 'alice@a.example'"), Some("Alice".into()));
    // Addresses are stored in lowercase, once.
    assert_eq!(env.count("SELECT count(*) FROM address WHERE addr = 'me@hey.example'"), 1);
}

#[test]
fn recipients_of_sent_messages_become_inbox_correspondents() {
    let env = Env::new();
    // Bob already has a decision: it must be kept.
    env.store
        .lock()
        .unwrap()
        .conn()
        .execute(
            "INSERT INTO address (addr, screening_target, date_screening_modified, created_by, modified_by)
             VALUES ('bob@b.example', 'newsletter', 1, 'app', 'app')",
            [],
        )
        .unwrap();
    let path = env.write_mbox("a.mbox", &sample());
    let (report, _) = env.import(&env.options(&path)).unwrap();
    // Alice and Carol; not Bob (decision kept), not the user's own address,
    // not the newsletter (the user never wrote to it).
    assert_eq!(report.correspondents, 2);
    assert_eq!(env.text("SELECT screening_target FROM address WHERE addr = 'alice@a.example'"), Some("inbox".into()));
    assert_eq!(env.text("SELECT screening_target FROM address WHERE addr = 'carol@c.example'"), Some("inbox".into()));
    assert_eq!(env.text("SELECT screening_target FROM address WHERE addr = 'bob@b.example'"), Some("newsletter".into()));
    assert_eq!(env.text("SELECT screening_target FROM address WHERE addr = 'me@hey.example'"), None);
    assert_eq!(env.text("SELECT screening_target FROM address WHERE addr = 'news@letter.example'"), None);
    // One audit line per decision, by the core, with the reason.
    assert_eq!(
        env.count("SELECT count(*) FROM audit_log WHERE entity = 'address' AND action = 'screening' AND created_by = 'core' AND json_extract(detail, '$.reason') = 'sent_correspondent'"),
        2
    );
}

#[test]
fn correspondents_can_be_left_without_decision() {
    let env = Env::new();
    let path = env.write_mbox("a.mbox", &sample());
    let mut opts = env.options(&path);
    opts.mark_correspondents = false;
    let (report, _) = env.import(&opts).unwrap();
    assert_eq!(report.correspondents, 0);
    assert_eq!(env.count("SELECT count(*) FROM address WHERE screening_target IS NOT NULL"), 0);
}

#[test]
fn refuses_an_imap_account_name_or_another_address() {
    let env = Env::new();
    env.store
        .lock()
        .unwrap()
        .conn()
        .execute("INSERT INTO account (config_key, created_by, modified_by) VALUES ('perso', 'app', 'app')", [])
        .unwrap();
    let path = env.write_mbox("a.mbox", &sample());
    let mut opts = env.options(&path);
    opts.account = "perso".into();
    assert!(matches!(env.import(&opts), Err(StoreError::InvalidParams(_))));

    let opts = env.options(&path);
    env.import(&opts).unwrap();
    let mut other = env.options(&path);
    other.address = "someone@else.example".into();
    assert!(matches!(env.import(&other), Err(StoreError::InvalidParams(_))));

    let mut bad = env.options(&path);
    bad.account = "a b".into();
    assert!(matches!(env.import(&bad), Err(StoreError::InvalidParams(_))));
}

#[test]
fn rejects_a_file_that_is_not_an_mbox_without_importing_anything() {
    let env = Env::new();
    let path = env.dir.path().join("not.mbox");
    std::fs::write(&path, "Subject: hello\n\nbody\n").unwrap();
    assert!(env.import(&env.options(&path)).is_err());
    assert_eq!(env.count("SELECT count(*) FROM message"), 0);
}

#[test]
fn missing_file_is_an_error() {
    let env = Env::new();
    let opts = env.options(&env.dir.path().join("absent.mbox"));
    assert!(matches!(env.import(&opts), Err(StoreError::InvalidParams(_))));
}

#[test]
fn imported_threads_can_be_moved_locally() {
    let env = Env::new();
    let path = env.write_mbox("a.mbox", &sample());
    env.import(&env.options(&path)).unwrap();
    let thread_id = env.count("SELECT thread_id FROM message WHERE message_id_hdr = 'n1@letter.example'");
    let changes = env.store.lock().unwrap().move_thread(thread_id, FolderRole::Newsletter, None, MoveOptions::default(), Source::App).unwrap();
    assert_eq!(changes.changes.changed_messages(), 1);
    assert_eq!(
        env.text("SELECT f.role FROM message m JOIN folder f ON f.id = m.folder_id WHERE m.message_id_hdr = 'n1@letter.example'"),
        Some("newsletter".into())
    );
}

// --- Reception date (`import-initial.md`, section 9, "Autres attributs") ---

/// `date_received` of the message with this `Message-ID`.
fn date_received(env: &Env, message_id: &str) -> i64 {
    env.count(&format!("SELECT date_received FROM message WHERE message_id_hdr = '{message_id}'"))
}

#[test]
fn the_from_line_date_is_the_reception_date() {
    let env = Env::new();
    let path = env.write_mbox("a.mbox", &sample());
    env.import(&env.options(&path)).unwrap();
    // From_ line "Tue Jun 23 19:20:56 2020", Date header two hours later in
    // +0200, i.e. the same instant: the From_ line wins either way.
    assert_eq!(date_received(&env, "m1@a.example"), 1_592_940_056);
    // No Date header: the From_ line date ("Thu Jun 25 07:00:00 2020").
    assert_eq!(date_received(&env, "n1@letter.example"), 1_593_068_400);
}

#[test]
fn a_from_line_written_at_export_time_falls_back_to_the_date_header() {
    // What Python's `mailbox` module writes when the exporting script does
    // not give it a date: the time of the export, years after the message.
    let env = Env::new();
    let path = env.write_mbox(
        "export.mbox",
        &[
            mail(
                "Wed Nov  4 10:04:44 2026",
                &["From: a@a.example", "To: me@hey.example", "Subject: Vieux", "Message-ID: <old@a.example>",
                  "Date: Tue, 27 Oct 2015 09:49:25 +0100"],
                "Bonjour",
            ),
            // A From_ line in the future: never a delivery date, even without
            // a Date header to compare with. No date at all: 0.
            mail(
                "Thu Jan  1 00:00:00 2099",
                &["From: b@b.example", "To: me@hey.example", "Subject: Futur", "Message-ID: <future@b.example>"],
                "Bonjour",
            ),
        ],
    );
    env.import(&env.options(&path)).unwrap();
    assert_eq!(date_received(&env, "old@a.example"), 1_445_935_765);
    assert_eq!(date_received(&env, "future@b.example"), 0);
    // The thread's date follows the message date.
    assert_eq!(
        env.count(
            "SELECT t.date_last_received FROM thread t JOIN message m ON m.thread_id = t.id
              WHERE m.message_id_hdr = 'old@a.example'"
        ),
        1_445_935_765
    );
}

#[test]
fn reception_date_rules() {
    let now = 2_000_000_000;
    let sent = 1_000_000_000;
    // Normal delivery a few minutes after sending: the From_ line.
    assert_eq!(reception_date(Some(sent + 300), Some(sent), now), sent + 300);
    // Delivered at the very limit of the delay: still the From_ line.
    assert_eq!(reception_date(Some(sent + MAX_DELIVERY_DELAY_SECS), Some(sent), now), sent + MAX_DELIVERY_DELAY_SECS);
    // One second later: not a delivery, the Date header.
    assert_eq!(reception_date(Some(sent + MAX_DELIVERY_DELAY_SECS + 1), Some(sent), now), sent);
    // Sender's clock ahead (From_ before Date): the From_ line.
    assert_eq!(reception_date(Some(sent - 3600), Some(sent), now), sent - 3600);
    // From_ line after the import started: the Date header, else 0.
    assert_eq!(reception_date(Some(now + 1), Some(sent), now), sent);
    assert_eq!(reception_date(Some(now + 1), None, now), 0);
    // From_ line at the import's start, no Date header: kept.
    assert_eq!(reception_date(Some(now), None, now), now);
    // No From_ line date.
    assert_eq!(reception_date(None, Some(sent), now), sent);
    assert_eq!(reception_date(None, None, now), 0);
}

// --- Names of addresses (`schema.sql`, table `address`) ---

#[test]
fn an_address_without_a_name_gets_the_first_one_a_mail_gives() {
    let env = Env::new();
    // An address known before any mail with a name: a screening decision
    // taken on it creates it without a name.
    env.store.lock().unwrap().set_address_screening("carol@c.example", Some(nm_core::ScreeningTarget::Inbox), Source::App).unwrap();
    let path = env.write_mbox(
        "names.mbox",
        &[
            // Dave first appears without a name, then with one, then with
            // another: the first name seen is kept.
            mail("Mon Jun  1 10:00:00 2020", &["From: dave@d.example", "To: me@hey.example", "Subject: a", "Message-ID: <a@d>"], "x"),
            mail("Mon Jun  1 11:00:00 2020", &["From: Dave Durand <dave@d.example>", "To: me@hey.example", "Subject: b", "Message-ID: <b@d>"], "x"),
            mail("Mon Jun  1 12:00:00 2020", &["From: D. Durand <dave@d.example>", "To: me@hey.example", "Subject: c", "Message-ID: <c@d>"], "x"),
            // Carol's name arrives as a recipient.
            mail("Mon Jun  1 13:00:00 2020", &["From: me@hey.example", "To: Carol Petit <carol@c.example>", "Subject: d", "Message-ID: <d@me>"], "x"),
            // An encoded word glued inside the name is decoded.
            mail("Mon Jun  1 14:00:00 2020", &["From: Achille Ber=?ISO-8859-1?B?6A==?=s <achille@b.example>", "To: me@hey.example", "Subject: e", "Message-ID: <e@b>"], "x"),
        ],
    );
    env.import(&env.options(&path)).unwrap();
    let name = |addr: &str| env.text(&format!("SELECT name FROM address WHERE addr = '{addr}'"));
    assert_eq!(name("dave@d.example").as_deref(), Some("Dave Durand"));
    assert_eq!(name("carol@c.example").as_deref(), Some("Carol Petit"));
    assert_eq!(name("achille@b.example").as_deref(), Some("Achille Berès"));
    assert_eq!(env.text("SELECT from_name FROM message WHERE message_id_hdr = 'e@b'").as_deref(), Some("Achille Berès"));
    assert_eq!(env.text("SELECT modified_by FROM address WHERE addr = 'carol@c.example'").as_deref(), Some("core"));
    // The From filter finds Dave by the name of his address.
    let filter = nm_api::ThreadFilter { from: vec!["durand".into()], ..Default::default() };
    let store = env.store.lock().unwrap();
    let (items, _) = store.list_everything(None, None, 10, nm_api::ThreadSort::Received, &filter).unwrap();
    assert_eq!(items.len(), 3);
}

#[test]
fn a_name_already_set_is_never_replaced_by_a_mail() {
    let env = Env::new();
    let path = env.write_mbox("a.mbox", &sample());
    env.import(&env.options(&path)).unwrap();
    // The user renames Alice; a new mail with another name changes nothing.
    env.store.lock().unwrap().conn().execute("UPDATE address SET name = 'Alice (cliente)' WHERE addr = 'alice@a.example'", []).unwrap();
    let path = env.write_mbox(
        "b.mbox",
        &[mail("Mon Jun  1 10:00:00 2020", &["From: Alice M. <alice@a.example>", "To: me@hey.example", "Subject: z", "Message-ID: <z@a>"], "x")],
    );
    env.import(&env.options(&path)).unwrap();
    assert_eq!(env.text("SELECT name FROM address WHERE addr = 'alice@a.example'").as_deref(), Some("Alice (cliente)"));
}
