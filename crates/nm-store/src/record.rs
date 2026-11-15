//! The record of one message (`message_record`): everything a client needs
//! to print it as a piece of evidence (`ui.md`, menu `s`, "p as pdf").
//!
//! A mail produced in a dispute must show who wrote it to whom, when, and
//! that the copy is faithful. So the record is read from the raw message,
//! the bytes the server delivered, rather than from the database:
//!
//! - the names are those written in this very mail (the `address` table
//!   keeps one name per address, the last one seen);
//! - every recipient is there, Bcc and Reply-To included, in header order;
//! - the `Date` header is given as written, with the sender's time zone,
//!   beside the reception date of the server;
//! - the technical headers that trace the mail (`Received`, `Return-Path`,
//!   `Authentication-Results`, `DKIM-Signature`) are kept as written;
//! - the SHA-256 of the raw message and of each attachment let anyone check
//!   that the `.eml` (`se`) and the files are those the PDF describes.
//!
//! A message without a raw file (made by `seed`, or whose body has not been
//! downloaded yet) falls back on the database: the record says so
//! (`raw_sha256` is `None`), and the client prints it.

use mail_parser::MessageParser;
use nm_api::{AddressView, MessageRecordView, RecordAttachmentView, RecordHeaderView};
use rusqlite::OptionalExtension;

use crate::ingest::ParsedAddress;
use crate::raw::{Hash, RawStore};
use crate::{Result, Store, StoreError};

/// Header fields of the technical annex, in this order, each kept as many
/// times as it appears. They trace the path of the mail (`Received`, newest
/// hop first, as servers add them on top), the address bounces go to
/// (`Return-Path`), the checks of the receiving server
/// (`Authentication-Results`, `Received-SPF`) and the signature of the
/// sending domain (`DKIM-Signature`, `ARC-*`).
const TRACE_HEADERS: &[&str] = &[
    "Return-Path",
    "Received",
    "Received-SPF",
    "Authentication-Results",
    "ARC-Authentication-Results",
    "DKIM-Signature",
    "ARC-Message-Signature",
    "ARC-Seal",
    "X-Mailer",
    "User-Agent",
];

/// A header value longer than this is cut, with "…": a DKIM signature or
/// an ARC seal is a few hundred characters, but a broken mail may hold a
/// header of megabytes, which would make a PDF of thousands of pages.
const MAX_HEADER_CHARS: usize = 4000;

impl Store {
    /// The record of `message_id`, read from its raw file when there is one
    /// (see the module comment).
    pub fn message_record(&self, raw: &RawStore, message_id: i64) -> Result<MessageRecordView> {
        type Row = (String, Option<String>, Option<String>, Option<i64>, i64, Option<Vec<u8>>, Option<String>, Option<i64>);
        let (subject, from_name, from_addr, date_sent, date_received, hash, text, size): Row = self
            .conn
            .query_row(
                "SELECT m.subject, m.from_name, m.from_addr, m.date_sent, m.date_received, m.raw_hash,
                        b.text_plain, m.size
                   FROM message m LEFT JOIN message_body b ON b.message_id = m.id WHERE m.id = ?1",
                [message_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?)),
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound(format!("message {message_id}")))?;
        let (account, folder, auth_result, message_id_hdr, in_reply_to): (String, String, String, Option<String>, Option<String>) =
            self.conn.query_row(
                "SELECT a.config_key, f.path, m.auth_result, m.message_id_hdr, m.in_reply_to
                   FROM message m JOIN folder f ON f.id = m.folder_id JOIN account a ON a.id = m.account_id
                  WHERE m.id = ?1",
                [message_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )?;
        // The body as the reader shows it, its paragraphs whole: the soft
        // line breaks a mail app puts every 72 characters would make ragged
        // lines in a proportional font. Only those breaks go; the words,
        // quotes and signature stay (`paragraphs.rs`).
        let body = text.map(|t| crate::paragraphs::unwrap(&t));

        let mut record = MessageRecordView {
            message_id: message_id_hdr,
            subject,
            from: from_addr.map(|addr| vec![AddressView { addr, name: from_name }]).unwrap_or_default(),
            sender: None,
            reply_to: self.recipients(message_id, "reply_to")?,
            to: self.recipients(message_id, "to")?,
            cc: self.recipients(message_id, "cc")?,
            bcc: self.recipients(message_id, "bcc")?,
            date_header: None,
            date_sent,
            date_received,
            in_reply_to,
            references: Vec::new(),
            account,
            folder,
            auth_result,
            body,
            attachments: self.record_attachments(message_id)?,
            headers: Vec::new(),
            raw_size: size,
            raw_sha256: None,
        };

        let Some(hash) = hash else { return Ok(record) };
        let hash: Hash = hash.try_into().map_err(|_| StoreError::Io("hash de brut invalide".into()))?;
        let bytes = raw.read(&hash).map_err(|e| crate::raw_error(&hash, e))?;
        fill_from_raw(&mut record, &bytes);
        record.raw_size = Some(bytes.len() as i64);
        // `raw.read` checked the bytes against `hash`: it is their SHA-256.
        record.raw_sha256 = Some(crate::raw::hex(&hash));
        Ok(record)
    }

    /// Recipients of one role from the database, in header order: the
    /// fallback of a message without a raw file.
    fn recipients(&self, message_id: i64, role: &str) -> Result<Vec<AddressView>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT a.addr, a.name FROM message_address ma JOIN address a ON a.id = ma.address_id
              WHERE ma.message_id = ?1 AND ma.role = ?2 ORDER BY ma.position",
        )?;
        let rows = stmt.query_map(rusqlite::params![message_id, role], |r| Ok(AddressView { addr: r.get(0)?, name: r.get(1)? }))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// The attachments of the database, without their hash (computed from
    /// the raw file, `fill_from_raw`). Inline images of the HTML are listed
    /// too: they are part of what the sender sent.
    fn record_attachments(&self, message_id: i64) -> Result<Vec<RecordAttachmentView>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT part_id, filename, mime_type, size, is_inline FROM attachment WHERE message_id = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map([message_id], |r| {
            Ok(RecordAttachmentView {
                part_id: r.get(0)?,
                filename: r.get(1)?,
                mime_type: r.get(2)?,
                size: r.get::<_, Option<i64>>(3)?.unwrap_or(0),
                inline: r.get(4)?,
                sha256: None,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }
}

/// Replaces what the database gave by what the raw message says, and adds
/// what only it holds. A message `mail-parser` cannot read at all keeps the
/// database's fields.
fn fill_from_raw(record: &mut MessageRecordView, bytes: &[u8]) {
    let parsed = crate::ingest::parse(bytes);
    if !parsed.parsed {
        return;
    }
    let view = |a: ParsedAddress| AddressView { addr: a.addr, name: a.name };
    let views = |list: Vec<ParsedAddress>| list.into_iter().map(view).collect::<Vec<_>>();
    // Only replaced when the header gave something: a broken `From` should
    // not erase what the database kept.
    if let Some(from) = parsed.from {
        record.from = vec![view(from)];
    }
    record.to = views(parsed.to);
    record.cc = views(parsed.cc);
    record.bcc = views(parsed.bcc);
    record.reply_to = views(parsed.reply_to);
    record.references = parsed.references;

    let Some(message) = MessageParser::default().parse(bytes) else { return };
    // Rust note: `root()` borrows `message`, which borrows `bytes`; the
    // values copied out below (`into_owned`, `to_string`) outlive both.
    let m = message.root();
    // Every mailbox of `From` (rare, but a mail may have several authors):
    // `ingest::parse` keeps only the first, the one rules apply to.
    let all_from: Vec<AddressView> = m
        .from()
        .iter()
        .flat_map(|l| l.mailboxes())
        .filter_map(|mb| {
            let addr = mb.address()?.trim().to_lowercase();
            let name = mb.name().map(|n| n.trim().trim_matches('"').trim().to_string()).filter(|n| !n.is_empty() && *n != addr);
            Some(AddressView { addr, name })
        })
        .collect();
    if all_from.len() > 1 {
        record.from = all_from;
    }
    record.sender = m
        .sender()
        .and_then(|l| l.first())
        .and_then(|mb| Some(AddressView { addr: mb.address()?.trim().to_lowercase(), name: mb.name().map(|n| n.trim().to_string()) }));
    let raw_value = |name: &str| -> Option<String> {
        m.headers().iter().find(|h| h.name().as_str().eq_ignore_ascii_case(name)).map(|h| unfold(&String::from_utf8_lossy(h.raw_value())))
    };
    record.date_header = raw_value("Date").filter(|d| !d.is_empty());
    for name in TRACE_HEADERS {
        for h in m.headers().iter().filter(|h| h.name().as_str().eq_ignore_ascii_case(name)) {
            record.headers.push(RecordHeaderView { name: (*name).to_string(), value: unfold(&String::from_utf8_lossy(h.raw_value())) });
        }
    }
    // Each attachment hashed as the file `get_attachment` gives: its
    // transfer-decoded content (base64 undone). The person receiving the
    // PDF hashes the file they were sent (`shasum -a 256 devis.pdf`) and
    // finds the same value.
    for a in &mut record.attachments {
        if let Some(content) = crate::attachments::part_content(bytes, &a.part_id) {
            a.sha256 = Some(crate::raw::hex(&crate::raw::hash(&content)));
            a.size = content.len() as i64;
        }
    }
}

/// A header value on one line: the folding of RFC 5322 (a line break
/// followed by spaces) becomes one space, and the value is cut beyond
/// `MAX_HEADER_CHARS`.
fn unfold(v: &str) -> String {
    let one_line = v.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= MAX_HEADER_CHARS {
        return one_line;
    }
    let mut cut: String = one_line.chars().take(MAX_HEADER_CHARS).collect();
    cut.push('…');
    cut
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unfold_joins_folded_lines_and_bounds_the_length() {
        assert_eq!(unfold(" from a.example\r\n\tby b.example;\r\n Mon, 1 Jun 2020 "), "from a.example by b.example; Mon, 1 Jun 2020");
        let long = "x".repeat(MAX_HEADER_CHARS + 10);
        let cut = unfold(&long);
        assert_eq!(cut.chars().count(), MAX_HEADER_CHARS + 1);
        assert!(cut.ends_with('…'));
    }
}
