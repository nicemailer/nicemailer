//! A 150 MB message, received in pieces as IMAP gives it, stored, parsed
//! and inserted with bounded memory (`plan-imap.md`, phase 3;
//! `stockage-brut.md`, section 11, "Messages volumineux").
//!
//! The test counts every allocation of its process: a test file is its own
//! program, so the counting allocator below applies to this file only.

use std::alloc::{GlobalAlloc, Layout, System};
use std::io::Read;
use std::sync::atomic::{AtomicUsize, Ordering};

use nm_core::Source;
use nm_store::ingest::parse;
use nm_store::insert::{Body, Flags, Inserter, NewMessage};
use nm_store::raw::{PlainBytes, RawStore};
use nm_store::{ChangeSet, Store};

/// The system allocator, counting the bytes in use and their peak.
///
/// Rust note: `#[global_allocator]` replaces the allocator of the whole
/// program. `GlobalAlloc` is an `unsafe trait`: the implementation
/// promises to return valid memory, here by delegating to `System`.
struct Counting;

static IN_USE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            let now = IN_USE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(now, Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        unsafe { System.dealloc(p, layout) };
        IN_USE.fetch_sub(layout.size(), Ordering::Relaxed);
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Size of the attachment, before base64 (at least).
const ATTACHMENT: usize = 110 << 20;

/// The message, produced one piece at a time and never whole: headers, a
/// text part, then a PDF of 110 MB in base64 (about 150 MB of lines).
///
/// The attachment repeats one block of 1 MB (about 18,000 lines of
/// base64): generating 110 MB of distinct bytes, twice (to write, then to
/// compare), cost more than the rest of the test in a debug build.
struct BigMessage {
    state: u8,
    produced: usize,
    /// The bytes one block encodes, and the block in base64.
    block_bytes: usize,
    block: Vec<u8>,
}

impl BigMessage {
    fn new() -> BigMessage {
        let mut seed = 1u32;
        let mut block = Vec::with_capacity(1 << 20);
        let mut block_bytes = 0;
        while block.len() < (1 << 20) - 80 {
            // 76 base64 characters per line encode 57 bytes; random-looking
            // ones, so that the compression has real work to do.
            let mut line = [0u8; 57];
            for b in line.iter_mut() {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                *b = (seed >> 16) as u8;
            }
            base64_line(&line, &mut block);
            block_bytes += 57;
        }
        BigMessage { state: 0, produced: 0, block_bytes, block }
    }

    /// The next piece, about 1 MB, or `None` at the end.
    fn next_piece(&mut self) -> Option<Vec<u8>> {
        match self.state {
            0 => {
                self.state = 1;
                Some(
                    b"Message-ID: <big@a.example>\r\nFrom: Alice <alice@a.example>\r\nTo: me@hey.example\r\n\
Subject: Les plans\r\nDate: Tue, 23 Jun 2020 21:20:56 +0200\r\nContent-Type: multipart/mixed; boundary=B\r\n\r\n\
--B\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nVoici les plans du chantier.\r\n--B\r\n\
Content-Type: application/pdf; name=plans.pdf\r\nContent-Disposition: attachment; filename=plans.pdf\r\n\
Content-Transfer-Encoding: base64\r\n\r\n"
                        .to_vec(),
                )
            }
            1 => {
                self.produced += self.block_bytes;
                if self.produced >= ATTACHMENT {
                    self.state = 2;
                }
                Some(self.block.clone())
            }
            2 => {
                self.state = 3;
                Some(b"--B--\r\n".to_vec())
            }
            _ => None,
        }
    }

    /// Bytes of the attachment, once decoded.
    fn attachment_size(&self) -> usize {
        ATTACHMENT.div_ceil(self.block_bytes) * self.block_bytes
    }
}

fn base64_line(bytes: &[u8], out: &mut Vec<u8>) {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(A[(n >> 18) as usize & 63]);
        out.push(A[(n >> 12) as usize & 63]);
        out.push(if c.len() > 1 { A[(n >> 6) as usize & 63] } else { b'=' });
        out.push(if c.len() > 2 { A[n as usize & 63] } else { b'=' });
    }
    out.extend_from_slice(b"\r\n");
}

#[test]
fn a_150_mb_message_in_pieces_with_bounded_memory() {
    let dir = tempfile::tempdir().unwrap();
    let raw = RawStore::open(dir.path()).unwrap();
    let mut store = Store::open_in_memory().unwrap();
    store
        .conn()
        .execute_batch(
            "INSERT INTO account (config_key, kind, created_by, modified_by) VALUES ('perso', 'imap', 'core', 'core');
             INSERT INTO folder (account_id, path, delimiter, role, created_by, modified_by)
             VALUES (1, 'INBOX', '/', 'inbox', 'core', 'core');",
        )
        .unwrap();
    let before = IN_USE.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);

    let mut w = raw.writer(true).unwrap();
    let mut msg = BigMessage::new();
    let attachment = msg.attachment_size();
    while let Some(piece) = msg.next_piece() {
        w.write(&piece).unwrap();
    }
    let size = w.size();
    assert!(size > 150_000_000, "{size}");
    let f = w.finish(Some(size)).unwrap();
    let plain = f.plain.unwrap();
    assert!(matches!(plain, PlainBytes::Mapped(_)), "a mapped file, not memory");
    let parsed = parse(&plain);
    assert_eq!(parsed.subject, "Les plans");
    assert_eq!(parsed.text.as_deref().map(str::trim), Some("Voici les plans du chantier."));
    assert_eq!(parsed.attachments.len(), 1);
    assert_eq!(parsed.attachments[0].size, attachment as i64, "measured without decoding");

    let mut ins = Inserter::new(Source::Imap, None);
    let mut changes = ChangeSet::default();
    let tx = store.transaction().unwrap();
    let m = NewMessage {
        account_id: 1,
        folder_id: 1,
        uid: Some(7),
        date_received: 0,
        size: Some(size),
        modseq: None,
        flags: Flags::default(),
        auth: (nm_core::AuthResult::Unknown, None),
        headers: &parsed,
    };
    ins.insert(&tx, &m, Some(Body { written: &f.written, parsed: &parsed }), &mut changes).unwrap();
    ins.commit(tx, &mut changes).unwrap();
    drop(plain);

    let peak = PEAK.load(Ordering::Relaxed) - before;
    println!("peak heap growth: {} MB for a {} MB message", peak >> 20, size >> 20);
    // The pieces (1 MB), zstd's window and buffers, the parser's tables:
    // a few tens of MB at most, whatever the size of the message.
    assert!(peak < 48 << 20, "peak heap growth {} MB", peak >> 20);

    // And the stored file is the message, byte for byte: compared piece by
    // piece with a new copy of the generator, without holding either.
    let file = std::fs::File::open(raw.path(&f.written.hash)).unwrap();
    let mut decoder = zstd::stream::read::Decoder::new(file).unwrap();
    let mut again = BigMessage::new();
    let mut buf = Vec::new();
    while let Some(piece) = again.next_piece() {
        buf.resize(piece.len(), 0);
        decoder.read_exact(&mut buf).unwrap();
        assert!(buf == piece);
    }
    assert_eq!(decoder.read(&mut [0u8; 1]).unwrap(), 0, "nothing more");
    let stored: i64 = store.conn().query_row("SELECT size FROM message", [], |r| r.get(0)).unwrap();
    assert_eq!(stored as u64, size);
}
