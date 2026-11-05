//! Phone numbers written in the text of a mail, offered by the `l` panel
//! as `tel:` links (`links.rs`): a click calls them with the Mac's phone
//! app.
//!
//! A mail is full of digits that are not phone numbers: dates, amounts,
//! order and invoice numbers, SIRET, IBAN, tracking numbers. A false
//! number in the panel is noise, a missed one only costs a copy: the rules
//! are strict on purpose. A number is taken when it is one of:
//!
//! - **international**: `+` (or `00`) then 8 to 15 digits, the first not 0
//!   (E.164), in any grouping: `+33 6 12 34 56 78`, `+33 (0)6 12 34 56 78`,
//!   `+1 415-555-0100`, `0044 20 7946 0958`;
//! - **national**, `0` then 1 to 9, grouped the way France, Belgium and
//!   Switzerland write them: 10 digits as `06 12 34 56 78` (2-2-2-2-2),
//!   `079 123 45 67` (3-3-2-2) or `0475 12 34 56` (4-2-2-2); 9 digits (a
//!   Belgian landline) as `02 123 45 67` (2-3-2-2) or `071 12 34 56`
//!   (3-2-2-2); with spaces, dots or dashes;
//! - **national without separators** (`0612345678`), only right after a
//!   word that announces a phone ("Tél", "phone", "mobile"…): alone, ten
//!   digits are as likely an order number.
//!
//! A run of digits is read whole: ten digits inside a longer number (an
//! IBAN, a SIRET) are never taken. Digits inside an address (`https://…`,
//! `x@y`) are left out, and so is a number announced as a fax.
//!
//! Not recognized: national numbers of other countries written without
//! their `+` prefix (`(415) 555-0100`), which cannot be told from other
//! numbers without knowing the country.

/// A phone number found in a text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Phone {
    /// `tel:` URL: `tel:+` and the digits of an international number, or
    /// `tel:` and the 9 or 10 digits of a national one, dialled as written.
    pub url: String,
    /// The number as written, spaces normalized ("06 12 34 56 78").
    pub written: String,
}

/// Most numbers kept for one text.
const MAX_PHONES: usize = 50;

/// Groupings of a national number, in digits per group.
const NATIONAL_GROUPS: &[&[usize]] = &[&[2, 2, 2, 2, 2], &[3, 3, 2, 2], &[4, 2, 2, 2], &[2, 3, 2, 2], &[3, 2, 2, 2]];

/// Words that announce a fax number (folded), before it ("Fax :",
/// "Télécopie", "F :" in a signature) or right after it ("(Fax)").
const FAX_WORDS: &[&str] = &["fax", "telecopie", "telecopieur", "fx", "f"];

/// Words that announce a phone number (folded: lowercase, no accent).
const PHONE_WORDS: &[&str] =
    &["tel", "telephone", "phone", "mobile", "portable", "mob", "gsm", "cell", "standard", "ligne", "appelez"];

/// The phone numbers of a text, in order, each once.
///
/// Read byte by byte: every character of a number (digits, `+`, spaces,
/// dots, dashes, parentheses) is ASCII but the no-break space, so the text
/// is never decoded as a whole, and a run of fewer than 8 digits (a date,
/// an amount, a time) is dropped before any other check. Mail texts can
/// weigh megabytes; this runs at each opening of a thread.
pub fn find(text: &str) -> Vec<Phone> {
    let b = text.as_bytes();
    let mut out: Vec<Phone> = Vec::new();
    let mut i = 0;
    while out.len() < MAX_PHONES {
        // Straight to the next digit or `+`.
        let Some(at) = b[i..].iter().position(|c| c.is_ascii_digit() || *c == b'+') else { break };
        i += at;
        if b[i] == b'+' && !b.get(i + 1).is_some_and(u8::is_ascii_digit) {
            i += 1;
            continue;
        }
        // The whole run: digits, and between them at most two separators
        // in a row ("+33 (0)6": space and parenthesis).
        let mut j = i + 1;
        let mut digits = usize::from(b[i].is_ascii_digit());
        loop {
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
                digits += 1;
            }
            let (mut k, mut seps) = (j, 0);
            while seps < 2 && let Some(width) = separator_width(b, k) {
                k += width;
                seps += 1;
            }
            if seps > 0 && k < b.len() && b[k].is_ascii_digit() {
                j = k;
            } else {
                break;
            }
        }
        // `i` is on an ASCII byte and `j` right after one: both are
        // character boundaries, so slicing the `&str` there is valid.
        if digits >= 8
            && bounded(text, i, j)
            && !in_address(text, i, j)
            && !announced_as_fax_after(&text[j..])
            && let Some(url) = read(&text[i..j], &text[..i])
            && !out.iter().any(|p| p.url == url)
        {
            out.push(Phone { url, written: text[i..j].replace('\u{a0}', " ") });
        }
        i = j;
    }
    out
}

/// The `tel:` URL of a number found in a `tel:` link of the HTML: its
/// digits, after a `+` when it has one. `None` below 6 digits.
pub fn tel_url(number: &str) -> Option<String> {
    // An `href` may encode its spaces and its `+` (`tel:+33%206%2012…`):
    // the digits of "%20" are not digits of the number.
    let number = number.replace("%20", " ").replace("%2B", "+").replace("%2b", "+");
    let number = number.trim();
    let digits: String = number.chars().filter(char::is_ascii_digit).collect();
    if digits.len() < 6 || digits.len() > 15 {
        return None;
    }
    Some(if number.starts_with('+') { format!("tel:+{digits}") } else { format!("tel:{digits}") })
}

fn is_separator(c: char) -> bool {
    matches!(c, ' ' | '\u{a0}' | '.' | '-' | '(' | ')')
}

/// Width in bytes of the separator at `k` (a no-break space is two bytes
/// in UTF-8, `C2 A0`), or `None`.
fn separator_width(b: &[u8], k: usize) -> Option<usize> {
    match b.get(k)? {
        b' ' | b'.' | b'-' | b'(' | b')' => Some(1),
        0xC2 if b.get(k + 1) == Some(&0xA0) => Some(2),
        _ => None,
    }
}

/// The run `text[i..j]` is a whole number, not a piece of a word or of a
/// longer one: no letter or digit glued to it, no "1.234" or "12,50"
/// around it.
fn bounded(text: &str, i: usize, j: usize) -> bool {
    let mut back = text[..i].chars().rev();
    let (before, before2) = (back.next(), back.next());
    let mut ahead = text[j..].chars();
    let (after, after2) = (ahead.next(), ahead.next());
    let glued = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || matches!(c, '/' | '=' | '&' | '#' | '_' | '%' | '@' | '+'));
    let decimal = |sep: Option<char>, digit: Option<char>| {
        sep.is_some_and(|c| matches!(c, '.' | ',')) && digit.is_some_and(|c| c.is_ascii_digit())
    };
    !glued(before) && !glued(after) && !decimal(before, before2) && !decimal(after, after2)
        && before != Some('-') && after != Some('-')
}

/// The number is followed by what it is a fax of: "01 56 33 33 01 (Fax)".
/// Only right after it, in parentheses or not, so that "Tél : … / Fax : …"
/// keeps the phone before the slash.
fn announced_as_fax_after(after: &str) -> bool {
    let line = after.split('\n').next().unwrap_or("");
    let next: String = line.trim_start_matches([' ', '\u{a0}', '(']).chars().take(10).collect();
    let next = fold(&next);
    FAX_WORDS.iter().any(|w| next.starts_with(w) && !next[w.len()..].starts_with(char::is_alphanumeric))
}

/// The number is inside an address: the word around it (between spaces)
/// holds `://`, `www.` or `@`.
fn in_address(text: &str, start: usize, end: usize) -> bool {
    // Looked for within an address's length on each side only: on a line
    // of megabytes without a space (minified HTML, encoded data), going
    // back to the previous space for each number would be quadratic.
    let lo = floor_boundary(text, start.saturating_sub(crate::links::MAX_URL));
    let hi = ceil_boundary(text, end + crate::links::MAX_URL);
    // After the space found, which may take several bytes (a no-break
    // space takes two): `p + 1` would land inside it.
    let from = text[lo..start]
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_whitespace())
        .map_or(lo, |(p, c)| lo + p + c.len_utf8());
    let to = text[end..hi].find(char::is_whitespace).map_or(hi, |p| p + end);
    let word = &text[from..to];
    word.contains("://") || word.contains("www.") || word.contains('@')
}

/// The nearest character boundary at or before `i`.
fn floor_boundary(text: &str, mut i: usize) -> usize {
    while !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// The nearest character boundary at or after `i`, at most the end.
fn ceil_boundary(text: &str, i: usize) -> usize {
    let mut i = i.min(text.len());
    while !text.is_char_boundary(i) {
        i += 1;
    }
    i
}

/// The `tel:` URL of a run of digits, if it is a phone number. `before`
/// is the text before it, read for the word announcing it.
fn read(run: &str, before: &str) -> Option<String> {
    let announced = last_words(before);
    // "Fax : …", "F : …", or "par fax au …": a full word among the last
    // two, a letter only as the last one ("Chef 06…" is not "f").
    let fax = |w: &String| FAX_WORDS.contains(&w.as_str());
    if announced.last().is_some_and(fax) || announced.iter().any(|w| w.len() > 2 && fax(w)) {
        return None;
    }
    // `00` is the international prefix only when the country code follows
    // it at once (`0044 20…`); "00 12 34 56 78" is not a number.
    let international =
        run.starts_with('+') || (run.starts_with("00") && run[2..].starts_with(|c: char| ('1'..='9').contains(&c)));
    if international {
        // "+33 (0)6…": the 0 in parentheses is dialled only from inside
        // the country.
        let run = run.replace("(0)", "");
        let digits: String = run.chars().filter(char::is_ascii_digit).collect();
        let digits = if run.starts_with('+') { digits.as_str() } else { &digits[2..] };
        return ((8..=15).contains(&digits.len()) && !digits.starts_with('0')).then(|| format!("tel:+{digits}"));
    }
    let groups: Vec<usize> =
        run.split(is_separator).filter(|g| !g.is_empty()).map(str::len).collect();
    let digits: String = run.chars().filter(char::is_ascii_digit).collect();
    let national = (9..=10).contains(&digits.len()) && digits.starts_with('0') && !digits[1..].starts_with('0');
    if !national || run.contains(['(', ')']) {
        return None;
    }
    // Separators all of one kind: "06 12.34-56 78" is no way to write it.
    let mut kinds = run.chars().filter(|c| is_separator(*c)).map(|c| if c == '\u{a0}' { ' ' } else { c });
    let first = kinds.next();
    if kinds.any(|c| Some(c) != first) {
        return None;
    }
    let grouped = NATIONAL_GROUPS.contains(&groups.as_slice());
    let alone_but_announced = groups.len() == 1 && announced.iter().any(|w| PHONE_WORDS.contains(&w.as_str()));
    (grouped || alone_but_announced).then(|| format!("tel:{digits}"))
}

/// The last two words of the line before the number, folded ("Tél. :"
/// gives "tel").
fn last_words(before: &str) -> Vec<String> {
    // The last 60 bytes are enough for two words, and keep this cheap on
    // a huge line.
    let before = &before[floor_boundary(before, before.len().saturating_sub(60))..];
    let line = before.rsplit('\n').next().unwrap_or("");
    let words: Vec<String> = line.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(fold).collect();
    words[words.len().saturating_sub(2)..].to_vec()
}

/// Lowercase without the accents of the words compared here.
fn fold(s: &str) -> String {
    s.to_lowercase().replace(['é', 'è', 'ê'], "e")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn urls(text: &str) -> Vec<String> {
        find(text).into_iter().map(|p| p.url).collect()
    }

    #[test]
    fn french_belgian_and_swiss_groupings_are_found() {
        assert_eq!(urls("Appelez-moi au 06 12 34 56 78."), ["tel:0612345678"]);
        assert_eq!(urls("Bureau : 01.23.45.67.89"), ["tel:0123456789"]);
        assert_eq!(urls("ligne 04-78-12-34-56"), ["tel:0478123456"]);
        assert_eq!(urls("079 123 45 67"), ["tel:0791234567"]);
        assert_eq!(urls("0475 12 34 56"), ["tel:0475123456"]);
        // Belgian landlines: 9 digits.
        assert_eq!(urls("02 123 45 67"), ["tel:021234567"]);
        assert_eq!(urls("071 12 34 56"), ["tel:071123456"]);
    }

    #[test]
    fn international_numbers_in_any_grouping() {
        assert_eq!(urls("+33 6 12 34 56 78"), ["tel:+33612345678"]);
        assert_eq!(urls("+33 (0)6 12 34 56 78"), ["tel:+33612345678"]);
        assert_eq!(urls("Call +1 415-555-0100 today"), ["tel:+14155550100"]);
        assert_eq!(urls("0044 20 7946 0958"), ["tel:+442079460958"]);
        let p = &find("Tél.\u{a0}: +33\u{a0}1\u{a0}23\u{a0}45\u{a0}67\u{a0}89")[0];
        assert_eq!((p.url.as_str(), p.written.as_str()), ("tel:+33123456789", "+33 1 23 45 67 89"));
    }

    #[test]
    fn ten_digits_alone_need_a_word_announcing_a_phone() {
        assert_eq!(urls("Tél : 0612345678"), ["tel:0612345678"]);
        assert_eq!(urls("Mobile 0612345678"), ["tel:0612345678"]);
        assert!(urls("Commande 0612345678").is_empty());
        assert!(urls("0612345678").is_empty());
    }

    #[test]
    fn other_numbers_are_not_phones() {
        for text in [
            "Le 05.11.2026 à 14:30",
            "Montant : 1 234 567,89 €",
            "SIRET 123 456 789 00012",
            "IBAN FR76 3000 6000 0112 3456 7890 189",
            "Colis 6A12345678901",
            "Réf. 06 12 34 56 78 90",
            "Facture n° 2026-000123",
            "Version 10.12.3.4.5",
            "06 12.34-56 78",
            "06123 45678",
            "00 12 34 56 78",
        ] {
            assert!(urls(text).is_empty(), "{text}");
        }
    }

    #[test]
    fn digits_inside_an_address_or_a_fax_are_left_out() {
        assert!(urls("https://x.org/c?id=06 12 34 56 78").is_empty());
        assert!(urls("https://x.org/06.12.34.56.78").is_empty());
        assert!(urls("0612345678@sms.example").is_empty());
        assert!(urls("Fax : 01 23 45 67 89").is_empty());
        // A no-break space before the number (two bytes in UTF-8).
        assert_eq!(urls("Tél\u{a0}:\u{a0}01 23 45 67 88"), ["tel:0123456788"]);
        assert!(urls("voir\u{a0}https://x.org/06.12.34.56.78").is_empty());
        // Far from any space: still read, without going back to the start.
        let long = format!("{}é 06 12 34 56 78", "x".repeat(10_000));
        assert_eq!(urls(&long), ["tel:0612345678"]);
        assert_eq!(urls("Tél : 01 23 45 67 88 / Fax : 01 23 45 67 89"), ["tel:0123456788"]);
        // Seen in real signatures.
        assert!(urls("Télécopie 01 56 33 33 01").is_empty());
        assert!(urls("F : 04 42 97 66 92").is_empty());
        assert!(urls("01 56 33 33 01 (Fax)").is_empty());
        assert!(urls("à nous adresser par fax au 01 41 66 18 63").is_empty());
        assert!(urls("1 page(s) fax from +33 1 42 93 21 17").is_empty());
        assert_eq!(urls("T : 04 42 97 98 92\nF : 04 42 97 66 92"), ["tel:0442979892"]);
        assert_eq!(urls("01 56 33 33 02 (direct GM)"), ["tel:0156333302"]);
        // "f" is a fax only as a word of its own.
        assert_eq!(urls("Chef 06 12 34 56 78"), ["tel:0612345678"]);
    }

    #[test]
    fn a_huge_line_of_digits_takes_linear_time() {
        // Megabytes without a space or a line break, full of numbers.
        let text = "0612345678x".repeat(200_000);
        let t = std::time::Instant::now();
        assert!(find(&text).is_empty());
        assert!(t.elapsed() < std::time::Duration::from_secs(2), "{:?}", t.elapsed());
    }

    #[test]
    fn each_number_once_in_order() {
        assert_eq!(
            urls("06 12 34 56 78\n+33 1 23 45 67 89\nencore 06 12 34 56 78"),
            ["tel:0612345678", "tel:+33123456789"]
        );
    }

    #[test]
    fn tel_links_of_the_html_keep_their_digits() {
        assert_eq!(tel_url("+33 6 12 34 56 78").as_deref(), Some("tel:+33612345678"));
        assert_eq!(tel_url("01-23-45-67-89").as_deref(), Some("tel:0123456789"));
        assert_eq!(tel_url("123"), None);
        assert_eq!(tel_url("%2B33%206%2012%2034%2056%2078").as_deref(), Some("tel:+33612345678"));
    }
}
