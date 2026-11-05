//! Modified UTF-7, the encoding of mailbox names on the wire (RFC 3501,
//! section 5.1.3).
//!
//! IMAP4rev1 predates UTF-8 everywhere: a mailbox name is sent in 7-bit
//! ASCII, and characters outside printable ASCII are written as base64 of
//! their UTF-16 code units, between `&` and `-`. `Envoyés` becomes
//! `Envoy&AOk-s`. The rules:
//!
//! - printable ASCII (0x20 to 0x7e) stands for itself, except `&`, which is
//!   written `&-`;
//! - anything else is a run of UTF-16BE code units, base64-encoded with the
//!   alphabet of RFC 2045 except that `,` replaces `/`, without `=` padding,
//!   between `&` and `-`.
//!
//! The server keeps the name in this form: NiceMailer sends back the bytes
//! the server listed, never a name it encoded again
//! ([`super::MailboxName`]), and only encodes names it creates itself.
//!
//! `UTF8=ACCEPT` (RFC 6855), which lets names travel as UTF-8, is not
//! enabled: Infomaniak does not announce it, and one encoding everywhere is
//! one less source of mismatch.

/// The base64 alphabet of modified UTF-7: `,` instead of `/`.
const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+,";

/// Encodes a mailbox name for the wire.
pub fn encode(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    // UTF-16 code units of the current run of non-ASCII characters.
    let mut run: Vec<u16> = Vec::new();
    for c in name.chars() {
        if (' '..='~').contains(&c) {
            flush(&mut run, &mut out);
            if c == '&' {
                out.push_str("&-");
            } else {
                out.push(c);
            }
        } else {
            // Rust note: `encode_utf16` writes the one or two UTF-16 code
            // units of a character (two for those beyond U+FFFF, such as
            // emoji: a "surrogate pair") into the buffer, and returns them.
            let mut buf = [0u16; 2];
            run.extend_from_slice(c.encode_utf16(&mut buf));
        }
    }
    flush(&mut run, &mut out);
    out
}

/// Writes a run of UTF-16 code units as `&<base64>-`, and empties it.
fn flush(run: &mut Vec<u16>, out: &mut String) {
    if run.is_empty() {
        return;
    }
    let bytes: Vec<u8> = run.iter().flat_map(|u| u.to_be_bytes()).collect();
    out.push('&');
    // Each group of 3 bytes gives 4 characters of 6 bits; a last group of
    // 1 or 2 bytes gives 2 or 3 characters, without padding.
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let chars = chunk.len() + 1;
        for i in 0..chars {
            out.push(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    out.push('-');
    run.clear();
}

/// Decodes a mailbox name from the wire. `None` if the bytes are not valid
/// modified UTF-7: the caller then shows them as they are
/// ([`super::MailboxName::name`]), since a server that breaks the rule must
/// not make the folder disappear.
pub fn decode(wire: &[u8]) -> Option<String> {
    let mut out = String::with_capacity(wire.len());
    let mut i = 0;
    while i < wire.len() {
        let b = wire[i];
        if b != b'&' {
            if !(0x20..=0x7e).contains(&b) {
                return None;
            }
            out.push(b as char);
            i += 1;
            continue;
        }
        let end = i + 1 + wire[i + 1..].iter().position(|&c| c == b'-')?;
        let encoded = &wire[i + 1..end];
        if encoded.is_empty() {
            out.push('&');
        } else {
            out.push_str(&decode_run(encoded)?);
        }
        i = end + 1;
    }
    Some(out)
}

/// Decodes the base64 between `&` and `-` into characters.
fn decode_run(encoded: &[u8]) -> Option<String> {
    let mut bits: u32 = 0;
    let mut nbits = 0;
    let mut bytes = Vec::with_capacity(encoded.len() * 3 / 4);
    for &c in encoded {
        let v = ALPHABET.iter().position(|&a| a == c)? as u32;
        bits = (bits << 6) | v;
        nbits += 6;
        if nbits >= 8 {
            nbits -= 8;
            bytes.push((bits >> nbits) as u8);
            bits &= (1 << nbits) - 1;
        }
    }
    // The leftover bits are padding, and must be zero (RFC 3501 forbids
    // other encodings of the same name).
    // Fewer than 6 of them: a whole character left over is one too many.
    if bits != 0 || nbits >= 6 || bytes.is_empty() || bytes.len() % 2 != 0 {
        return None;
    }
    let units: Vec<u16> = bytes.chunks(2).map(|p| u16::from_be_bytes([p[0], p[1]])).collect();
    let s = String::from_utf16(&units).ok()?;
    // A run that only holds printable ASCII should have been written as is.
    if s.chars().any(|c| (' '..='~').contains(&c)) {
        return None;
    }
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn examples_of_the_rfc_and_common_names() {
        // RFC 3501, section 5.1.3.
        assert_eq!(encode("~peter/mail/台北/日本語"), "~peter/mail/&U,BTFw-/&ZeVnLIqe-");
        assert_eq!(encode("Envoyés"), "Envoy&AOk-s");
        assert_eq!(encode("Éléments supprimés"), "&AMk-l&AOk-ments supprim&AOk-s");
        assert_eq!(encode("A&B"), "A&-B");
        assert_eq!(encode("INBOX"), "INBOX");
        assert_eq!(decode(b"~peter/mail/&U,BTFw-/&ZeVnLIqe-").as_deref(), Some("~peter/mail/台北/日本語"));
        assert_eq!(decode(b"Envoy&AOk-s").as_deref(), Some("Envoyés"));
        assert_eq!(decode(b"A&-B").as_deref(), Some("A&B"));
    }

    #[test]
    fn round_trip_of_any_text() {
        for name in ["", "&", "&&", "é", "😀", "a😀b", "Courrier indésirable", "x\u{7f}y", "tab\there", "日本語&English"] {
            assert_eq!(decode(encode(name).as_bytes()).as_deref(), Some(name), "{name:?}");
        }
    }

    #[test]
    fn invalid_names_are_refused() {
        assert_eq!(decode(b"Envoy&AOk"), None, "no closing -");
        assert_eq!(decode(b"a&A-"), None, "odd number of bytes");
        assert_eq!(decode(b"a&AGE-"), None, "printable ASCII written in base64");
        assert_eq!(decode(b"a&!!-"), None, "outside the alphabet");
        assert_eq!(decode("Envoyés".as_bytes()), None, "raw 8-bit");
        assert_eq!(decode(b"&2D0-"), None, "lone surrogate");
    }
}
