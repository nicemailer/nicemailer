//! The link key of a message: what `nicemailer://m/<key>` names
//! (`modele de donnees.md`, `link_key`; `docs/deep-links.md`).
//!
//! `message.id` is short but belongs to one database: another Mac, or the
//! same mailbox imported again, numbers the same mail differently, so a
//! link pasted in a note would open nothing there, or the wrong mail. The
//! key depends only on the mail:
//!
//! - its `Message-ID`, as stored (`message_id_hdr`: no angle brackets, no
//!   spaces, case kept). The same whatever brought the mail (IMAP, or an
//!   mbox export whose bytes differ: added headers, line endings), on every
//!   account that received it, and known from the headers alone;
//! - for the rare mail without one (about 1 % of a real mailbox), the hash
//!   of its raw file (`raw_hash`): stable only between Macs that got the
//!   mail from the same source, and unknown until its body is downloaded.
//!
//! The key is the first 48 bits of that SHA-256, written in a link as 12
//! lowercase hexadecimal characters. Fixed length on purpose, not Git's
//! "shortest unique prefix": a prefix unique in one database when copied
//! says nothing of another database, or of the mail still to come. With N
//! mails, a link is ambiguous with a probability of about N / 2⁴⁸: 10⁻⁹
//! for 300,000 mails. Stored as an `INTEGER` (below 2⁴⁸, so always
//! positive in an `i64`): a compact index, and an equality to find it.

use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};

use crate::Result;

/// The key of a message, from what the database holds: its `Message-ID`
/// if it has one, else the hash of its raw file, else none (headers only,
/// without `Message-ID`: its link stays `nicemailer://m/<id>`).
pub fn key(message_id_hdr: Option<&str>, raw_hash: Option<&[u8]>) -> Option<i64> {
    // Rust note: `filter` keeps the `Some` only if the closure says so, so
    // an empty `Message-ID` counts as none.
    match message_id_hdr.filter(|m| !m.is_empty()) {
        Some(m) => Some(first_48_bits(&Sha256::digest(m.as_bytes()))),
        // The raw hash already is a SHA-256: its first bits are the key,
        // so a key from the raw file is also the start of the file's name
        // in `raw.noindex/` (`raw.rs`), handy to find it by hand.
        None => raw_hash.filter(|h| h.len() >= 6).map(first_48_bits),
    }
}

/// The first 6 bytes of a hash, read as a big-endian number: the order in
/// which they are written in hexadecimal, so the key's 12 characters are
/// the hash's first 12.
fn first_48_bits(hash: &[u8]) -> i64 {
    hash[..6].iter().fold(0i64, |acc, b| (acc << 8) | i64::from(*b))
}

/// Gives a key to every message that can have one and has none yet: the
/// mail stored before version 15 (`migrations.rs`), and the rows of the
/// demonstration mailbox (`seed.rs`). Reads the ids first, then updates in
/// the same transaction as the caller. Each update rewrites its row: on a
/// real mailbox of 275,000 mails, about 10 s, once (measured on a copy;
/// the hashing itself is negligible).
pub(crate) fn fill_missing(conn: &Connection) -> Result<()> {
    let rows: Vec<(i64, Option<String>, Option<Vec<u8>>)> = conn
        .prepare(
            "SELECT id, message_id_hdr, raw_hash FROM message
              WHERE link_key IS NULL AND (coalesce(message_id_hdr, '') <> '' OR raw_hash IS NOT NULL)",
        )?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<std::result::Result<_, _>>()?;
    let mut update = conn.prepare("UPDATE message SET link_key = ?2 WHERE id = ?1")?;
    for (id, msgid, raw) in rows {
        if let Some(k) = key(msgid.as_deref(), raw.as_deref()) {
            update.execute(params![id, k])?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference values: another Mac, or a later version, must compute the
    /// very same keys, or the links already pasted in notes break. The
    /// expected value is the start of `printf %s 'abc@x.org' | shasum -a 256`.
    #[test]
    fn the_key_is_the_start_of_the_sha256_of_the_message_id() {
        assert_eq!(nm_api::deep_link::key_text(key(Some("abc@x.org"), None).unwrap()), REFERENCE_ABC);
    }

    const REFERENCE_ABC: &str = "07bcf99aef4d";

    #[test]
    fn the_message_id_wins_over_the_raw_file() {
        let raw = [0xabu8, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89];
        assert_eq!(key(Some("abc@x.org"), Some(&raw)), key(Some("abc@x.org"), None));
        assert_eq!(key(None, Some(&raw)), Some(0xabcdef012345));
        // An empty Message-ID counts as none.
        assert_eq!(key(Some(""), Some(&raw)), Some(0xabcdef012345));
        assert_eq!(key(None, None), None);
        assert_eq!(key(Some(""), None), None);
    }

    #[test]
    fn the_case_of_the_message_id_counts() {
        // RFC 5322: the left part of a Message-ID is case sensitive; two
        // ids that differ by case are two mails.
        assert_ne!(key(Some("ABC@x.org"), None), key(Some("abc@x.org"), None));
    }

    #[test]
    fn a_key_fits_in_48_bits() {
        let raw = [0xffu8; 32];
        assert_eq!(key(None, Some(&raw)), Some((1 << 48) - 1));
    }
}
