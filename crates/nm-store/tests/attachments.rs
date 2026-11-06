//! Integration tests of `attachments.rs`: the attachments listed under each
//! message by `get_thread`, and their extraction from the raw message by
//! `get_attachment` (`stockage-brut.md`, section 6).

use std::os::unix::fs::PermissionsExt;
use std::sync::Mutex;

use nm_core::Source;
use nm_store::attachments::AttachmentFiles;
use nm_store::import::{ImportOptions, import_mbox};
use nm_store::raw::RawStore;
use nm_store::seed::{SeedOptions, seed};
use nm_store::{Store, StoreError};

/// Four mails: a PDF and a PNG attached; an HTML mail with its logo
/// embedded (`cid:`); a plain text mail with a photo marked inline (Apple
/// Mail); a file name trying to escape its directory.
const MBOX: &str = "From x Mon Nov  2 10:00:00 2026
From: Alice <alice@x.org>
To: me@x.org
Subject: Devis
Message-ID: <1@x>
MIME-Version: 1.0
Content-Type: multipart/mixed; boundary=\"A\"

--A
Content-Type: text/plain

Le devis et la photo.
--A
Content-Type: application/pdf; name=\"devis.pdf\"
Content-Disposition: attachment; filename=\"devis.pdf\"
Content-Transfer-Encoding: base64

JVBERi0xLjQK
--A
Content-Type: image/png; name=\"photo.png\"
Content-Disposition: attachment; filename=\"photo.png\"
Content-Transfer-Encoding: base64

iVBORw0KGgo=
--A--

From x Mon Nov  2 11:00:00 2026
From: Lettre <news@x.org>
To: me@x.org
Subject: Lettre
Message-ID: <2@x>
MIME-Version: 1.0
Content-Type: multipart/related; boundary=\"B\"

--B
Content-Type: text/html

<p>Bonjour <img src=\"cid:logo@x\"></p>
--B
Content-Type: image/png; name=\"logo.png\"
Content-Disposition: inline; filename=\"logo.png\"
Content-ID: <logo@x>
Content-Transfer-Encoding: base64

iVBORw0KGgo=
--B--

From x Mon Nov  2 12:00:00 2026
From: Bob <bob@x.org>
To: me@x.org
Subject: Photo
Message-ID: <3@x>
MIME-Version: 1.0
Content-Type: multipart/mixed; boundary=\"C\"

--C
Content-Type: text/plain

Regarde.
--C
Content-Type: image/jpeg; name=\"IMG_1.jpg\"
Content-Disposition: inline; filename=\"IMG_1.jpg\"
Content-ID: <img1@x>
Content-Transfer-Encoding: base64

/9j/4AAQ
--C--

From x Mon Nov  2 13:00:00 2026
From: Eve <eve@x.org>
To: me@x.org
Subject: Piege
Message-ID: <4@x>
MIME-Version: 1.0
Content-Type: multipart/mixed; boundary=\"D\"

--D
Content-Type: text/plain

Ouvre.
--D
Content-Type: application/x-sh; name=\"../../.zshrc\"
Content-Disposition: attachment; filename=\"../../.zshrc\"

echo pwned
--D--
";

struct Fixture {
    dir: tempfile::TempDir,
    raw: RawStore,
    files: AttachmentFiles,
    store: Store,
}

fn imported() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let raw = RawStore::open(dir.path()).unwrap();
    let files = AttachmentFiles::open(dir.path()).unwrap();
    let path = dir.path().join("a.mbox");
    std::fs::write(&path, MBOX).unwrap();
    let store = Mutex::new(Store::open_in_memory().unwrap());
    let opts = ImportOptions {
        path,
        account: "t".into(),
        address: "me@x.org".into(),
        mark_correspondents: false,
        source: Source::Cli,
        batch_size: 10,
    };
    import_mbox(&store, &raw, &opts, &mut |_, _, _| {}).unwrap();
    Fixture { dir, raw, files, store: store.into_inner().unwrap() }
}

/// The attachments `get_thread` lists under the mail `subject`.
fn listed(store: &Store, subject: &str) -> Vec<nm_api::AttachmentView> {
    let thread: i64 = store
        .conn()
        .query_row("SELECT thread_id FROM message WHERE subject = ?1", [subject], |r| r.get(0))
        .unwrap();
    store.get_thread(thread).unwrap().1.remove(0).attachments
}

#[test]
fn each_message_lists_its_files_but_not_the_images_of_its_html() {
    let f = imported();
    let devis = listed(&f.store, "Devis");
    let names: Vec<_> = devis.iter().map(|a| (a.filename.as_deref().unwrap(), a.mime_type.as_str(), a.size)).collect();
    assert_eq!(names, [("devis.pdf", "application/pdf", 9), ("photo.png", "image/png", 8)]);
    // The logo is shown by the HTML: not a file to download.
    assert!(listed(&f.store, "Lettre").is_empty());
    // A photo marked inline in a mail without HTML would be invisible.
    let photo = listed(&f.store, "Photo");
    assert_eq!(photo.len(), 1);
    assert_eq!(photo[0].filename.as_deref(), Some("IMG_1.jpg"));
}

#[test]
fn the_images_of_the_html_get_a_source_the_reader_can_load() {
    let f = imported();
    let thread: i64 = f.store.conn().query_row("SELECT thread_id FROM message WHERE subject = 'Lettre'", [], |r| r.get(0)).unwrap();
    let m = f.store.get_thread(thread).unwrap().1.remove(0);
    let logo: i64 = f.store.conn().query_row("SELECT id FROM attachment WHERE content_id = 'logo@x'", [], |r| r.get(0)).unwrap();
    let html = m.body_html.unwrap();
    // `cid:` is replaced by the app's scheme, with the attachment's id,
    // which `get_attachment` extracts like any other file.
    assert!(html.contains(&format!(r#"<img src="nm-cid://image/{logo}">"#)), "{html}");
    assert!(!html.contains("cid:logo"), "{html}");
    let file = f.store.get_attachment(&f.raw, &f.files, logo).unwrap();
    assert_eq!(file.mime_type, "image/png");
    // The stored HTML keeps `cid:`: the source is made on each read.
    let stored: String = f.store.conn().query_row(
        "SELECT html_sanitized FROM message_body WHERE message_id = ?1", [m.id], |r| r.get(0)).unwrap();
    assert!(stored.contains("cid:logo@x"), "{stored}");
}

#[test]
fn a_file_is_extracted_once_read_only_and_quarantined() {
    let f = imported();
    let pdf = &listed(&f.store, "Devis")[0];
    let file = f.store.get_attachment(&f.raw, &f.files, pdf.id).unwrap();
    assert_eq!(file.filename, "devis.pdf");
    assert_eq!(file.mime_type, "application/pdf");
    assert_eq!(std::fs::read(&file.path).unwrap(), b"%PDF-1.4\n");
    assert!(file.path.starts_with(f.dir.path().join("attachments.noindex")));
    assert_eq!(std::fs::metadata(&file.path).unwrap().permissions().mode() & 0o777, 0o400);
    assert!(quarantined(&file.path), "no quarantine attribute on {}", file.path.display());
    // Recorded, then reused.
    let recorded: Option<String> = f
        .store
        .conn()
        .query_row("SELECT disk_path FROM attachment WHERE id = ?1", [pdf.id], |r| r.get(0))
        .unwrap();
    assert_eq!(recorded.as_deref(), Some(file.path.to_str().unwrap()));
    assert_eq!(f.store.get_attachment(&f.raw, &f.files, pdf.id).unwrap(), file);
}

#[test]
fn a_purged_file_is_extracted_again() {
    let f = imported();
    let png = &listed(&f.store, "Devis")[1];
    let first = f.store.get_attachment(&f.raw, &f.files, png.id).unwrap();
    // Purging the extracted files is allowed at any time.
    std::fs::remove_dir_all(f.dir.path().join("attachments.noindex")).unwrap();
    let files = AttachmentFiles::open(f.dir.path()).unwrap();
    let again = f.store.get_attachment(&f.raw, &files, png.id).unwrap();
    assert_eq!(again.path, first.path);
    assert_eq!(std::fs::read(&again.path).unwrap(), b"\x89PNG\r\n\x1a\n");
}

#[test]
fn a_name_chosen_by_the_sender_cannot_escape_the_directory() {
    let f = imported();
    let trap = &listed(&f.store, "Piege")[0];
    let file = f.store.get_attachment(&f.raw, &f.files, trap.id).unwrap();
    assert_eq!(file.filename, "zshrc");
    assert!(file.path.starts_with(f.dir.path().join("attachments.noindex")));
    assert_eq!(std::fs::read_to_string(&file.path).unwrap().trim_end(), "echo pwned");
}

#[test]
fn unknown_attachments_and_mails_without_raw_are_errors() {
    let f = imported();
    assert!(matches!(f.store.get_attachment(&f.raw, &f.files, 9_999), Err(StoreError::NotFound(_))));
    // Generated mail has attachments listed but no raw file to read them from.
    let mut seeded = Store::open_in_memory().unwrap();
    seed(&mut seeded, &SeedOptions { threads: 50, seed: 3, now: 1_790_000_000 }).unwrap();
    let id: i64 = seeded.conn().query_row("SELECT id FROM attachment LIMIT 1", [], |r| r.get(0)).unwrap();
    assert!(matches!(seeded.get_attachment(&f.raw, &f.files, id), Err(StoreError::NotFound(_))));
}

/// Whether macOS's quarantine attribute is set on `path`.
#[cfg(target_os = "macos")]
fn quarantined(path: &std::path::Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let cpath = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    // Rust note: `unsafe` for a C call; a null buffer asks for the size only.
    let size = unsafe { libc::getxattr(cpath.as_ptr(), c"com.apple.quarantine".as_ptr(), std::ptr::null_mut(), 0, 0, 0) };
    size > 0
}

#[cfg(not(target_os = "macos"))]
fn quarantined(_path: &std::path::Path) -> bool {
    true
}
