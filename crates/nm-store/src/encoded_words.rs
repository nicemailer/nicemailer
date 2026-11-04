//! Decoding of the RFC 2047 "encoded words" that `mail-parser` leaves as is.
//!
//! A header that is not plain ASCII carries its text as encoded words:
//! `=?ISO-8859-1?B?6A==?=` is "è" (charset, then `B` for Base64 or `Q` for a
//! quoted-printable variant, then the bytes). RFC 2047 only allows an
//! encoded word as a whole word, separated from the rest by spaces, and
//! that is all `mail-parser` decodes. Old mail clients did not care and
//! glued them inside words: `Achille Ber=?ISO-8859-1?B?6A==?=s` for
//! "Achille Berès". Others put spaces inside a `Q` word. Left as is, such a
//! name shows garbage in the list and is not found when searching for
//! "Berès".
//!
//! `decode_leftovers` runs after `mail-parser` on the display names and
//! the subject (`ingest.rs`), and once on the rows stored before it existed
//! (schema migration 2, `migrations.rs`). It decodes every encoded word it
//! finds, wherever it is. A text without `=?` is returned untouched without
//! copying (`Cow::Borrowed`), so the common case costs one substring search.
//!
//! Rust note: `Cow<'_, str>` ("clone on write") is either a borrowed `&str`
//! or an owned `String`. Returning it lets the function avoid allocating
//! when there is nothing to change.

use std::borrow::Cow;

/// Longest encoded text accepted between `?B?`/`?Q?` and `?=`. RFC 2047
/// limits a whole encoded word to 75 characters; broken clients exceed it,
/// but a "word" of several kilobytes is not an encoded word.
const MAX_ENCODED_LEN: usize = 1000;

/// The text with every encoded word decoded. Whitespace between two encoded
/// words is dropped, as RFC 2047 says (it only separates them); whitespace
/// next to plain text is kept. An encoded word that cannot be decoded
/// (unknown encoding, broken Base64) is left as it was.
pub fn decode_leftovers(text: &str) -> Cow<'_, str> {
    if !text.contains("=?") {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    // Whitespace seen after the last decoded word, not yet written: it is
    // dropped if another encoded word follows, written otherwise.
    let mut pending_space = String::new();
    let mut after_word = false;
    while let Some(start) = rest.find("=?") {
        let (before, candidate) = rest.split_at(start);
        match parse_word(candidate) {
            Some((decoded, len)) => {
                if after_word && before.chars().all(char::is_whitespace) {
                    // Only whitespace between two encoded words: dropped.
                } else {
                    out.push_str(&pending_space);
                    out.push_str(before);
                }
                pending_space.clear();
                out.push_str(&decoded);
                rest = &candidate[len..];
                // Keep the whitespace that follows aside until we know what
                // comes after it.
                let trimmed = rest.trim_start();
                pending_space.push_str(&rest[..rest.len() - trimmed.len()]);
                rest = trimmed;
                after_word = true;
            }
            None => {
                // Not an encoded word: copy up to and including "=?" and go on.
                out.push_str(&pending_space);
                pending_space.clear();
                out.push_str(before);
                out.push_str("=?");
                rest = &candidate[2..];
                after_word = false;
            }
        }
    }
    out.push_str(&pending_space);
    out.push_str(rest);
    Cow::Owned(out)
}

/// Decodes the encoded word at the start of `s` (which starts with `=?`).
/// Returns the text and the number of bytes the word takes in `s`.
fn parse_word(s: &str) -> Option<(String, usize)> {
    let body = &s[2..];
    let q1 = body.find('?')?;
    let charset = &body[..q1];
    // A charset label: letters, digits and a few signs. RFC 2231 may add a
    // language after `*` (`UTF-8*fr`), which does not change the decoding.
    if charset.is_empty() || !charset.chars().all(|c| c.is_ascii_alphanumeric() || "-_.:*".contains(c)) {
        return None;
    }
    let charset = charset.split('*').next().unwrap_or(charset);
    let after_charset = &body[q1 + 1..];
    let mut chars = after_charset.chars();
    let encoding = chars.next()?.to_ascii_uppercase();
    if chars.next()? != '?' {
        return None;
    }
    let encoded_and_rest = &after_charset[2..];
    let end = encoded_and_rest.find("?=")?;
    if end > MAX_ENCODED_LEN {
        return None;
    }
    let encoded = &encoded_and_rest[..end];
    let bytes = match encoding {
        'B' => base64(encoded)?,
        'Q' => q_decode(encoded)?,
        _ => return None,
    };
    let text = mail_parser::decoders::charsets::decode(charset.as_bytes(), &bytes).into_owned();
    // `=?` + charset + `?` + encoding + `?` + text + `?=`.
    let len = 2 + q1 + 1 + 2 + end + 2;
    Some((text, len))
}

/// Base64 (RFC 4648), lenient: whitespace is ignored and padding is
/// optional. Any other character makes it fail.
fn base64(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut acc: u32 = 0;
    let mut bits = 0;
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            c if c.is_ascii_whitespace() => continue,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// The "Q" encoding: `_` is a space, `=XX` a byte in hexadecimal, any other
/// character itself. A `=` not followed by two hex digits makes it fail.
fn q_decode(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'_' => out.push(b' '),
            b'=' => {
                let hex = s.get(i + 1..i + 3)?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 2;
            }
            c => out.push(c),
        }
        i += 1;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::decode_leftovers;

    #[test]
    fn plain_text_is_borrowed_untouched() {
        assert!(matches!(decode_leftovers("Achille Berès"), std::borrow::Cow::Borrowed(_)));
    }

    #[test]
    fn decodes_words_glued_inside_words() {
        // Real names found in an Apple Mail export.
        assert_eq!(decode_leftovers("Achille Ber=?ISO-8859-1?B?6A==?=s"), "Achille Berès");
        assert_eq!(decode_leftovers("Ang=?ISO-8859-1?B?6Q==?=lique Ber=?ISO-8859-1?B?6A==?=s"), "Angélique Berès");
        assert_eq!(decode_leftovers("Fr=?ISO-8859-1?B?6WTp?=ric Morel"), "Frédéric Morel");
        // The encoded word holds the space: "é " then "Mainguet".
        assert_eq!(decode_leftovers("Herv=?ISO-8859-1?B?6SA=?=Mainguet"), "Hervé Mainguet");
        assert_eq!(decode_leftovers("Jean-=?ISO-8859-1?B?yQ==?=ric Garnier"), "Jean-Éric Garnier");
        assert_eq!(decode_leftovers("L'=?UTF-8?Q?=C3=A9quipe=20Plaxo?="), "L'équipe Plaxo");
    }

    #[test]
    fn whitespace_between_two_encoded_words_is_dropped() {
        assert_eq!(decode_leftovers("=?UTF-8?Q?Bonjour_?= =?UTF-8?Q?=C3=A0_tous?="), "Bonjour à tous");
        // Next to plain text, whitespace stays.
        assert_eq!(decode_leftovers("Re: =?UTF-8?Q?caf=C3=A9?= demain"), "Re: café demain");
    }

    #[test]
    fn tolerates_spaces_inside_a_q_word_and_lowercase_labels() {
        assert_eq!(
            decode_leftovers("Fwd: =?iso-8859-15?Q?Haut D=E9bit d'Orange?="),
            "Fwd: Haut Débit d'Orange"
        );
        assert_eq!(decode_leftovers("=?utf-8?b?w6k=?="), "é");
        // Missing Base64 padding.
        assert_eq!(decode_leftovers("=?UTF-8?B?w6k?="), "é");
    }

    #[test]
    fn leaves_what_is_not_a_valid_encoded_word() {
        // Broken hexadecimal in a Q word.
        assert_eq!(decode_leftovers("a =?UTF-8?Q?x=D=A?= b"), "a =?UTF-8?Q?x=D=A?= b");
        // Not an encoded word at all.
        assert_eq!(decode_leftovers("1 + 1 =? 2"), "1 + 1 =? 2");
        assert_eq!(decode_leftovers("=?"), "=?");
        // Unknown encoding letter.
        assert_eq!(decode_leftovers("=?UTF-8?X?abc?="), "=?UTF-8?X?abc?=");
        // One valid word next to a broken one: the valid one is decoded.
        assert_eq!(decode_leftovers("=?UTF-8?Q?a=ZZ?= =?UTF-8?Q?=C3=A9?="), "=?UTF-8?Q?a=ZZ?= é");
    }
}
