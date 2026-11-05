//! The links of a message, for the `l` panel (`ui.md`: "l link (panneau
//! des liens du mail ; entrée ouvre le lien dans le navigateur)").
//!
//! Read from the sanitized HTML first, then from the text part:
//!
//! - In the HTML, a link is `<a href="…">text</a>`: its address is in an
//!   attribute, never in the text. The text the core makes from the HTML
//!   (`html_text.rs`) keeps "text" and drops the address, as a reader
//!   would. Looking for addresses in that text found nothing, so the HTML
//!   is read here. The text of the link comes with it: in a newsletter the
//!   address is often an unreadable tracking redirect, and "See my invoice"
//!   is what the user is looking for.
//! - In a text part, an address is written out (`https://…`, `mailto:…`).
//!   Those not already found in the HTML are added, without text.
//! - Phone numbers, as `tel:` links: those of the HTML (`<a href="tel:…">`)
//!   and those written in the text (`phones.rs`), the number as written
//!   being their text.
//!
//! Computed when a thread is read (`queries.rs`), not stored: it is one
//! linear scan of a body already loaded, and nothing has to be reindexed
//! when the rule changes.
//!
//! The scanner is small rather than a full HTML parser: the HTML is the
//! core's sanitized copy (`ingest.rs`, ammonia), where links are plain
//! `<a href="…">` with quoted attributes and entities encoded the standard
//! way. `unsubscribe.rs` reads the same anchors.

/// Longest URL kept: anything longer is not a link a human would follow.
pub const MAX_URL: usize = 2048;

/// Longest link text kept, in characters: a link wrapping a whole
/// paragraph is cut, the picker shows one line anyway.
const MAX_TEXT: usize = 200;

/// Most links kept for one message: enough for any real newsletter, and a
/// bound on what a hostile mail can make the core send.
const MAX_LINKS: usize = 500;

/// A link of a message (`nm_api::LinkView`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    /// `http://`, `https://`, `mailto:` or `tel:` URL, entities decoded. A
    /// `tel:` URL holds digits only, after a `+` for an international
    /// number (`tel:+33612345678`, `tel:0612345678`).
    pub url: String,
    /// Visible text of the link, whitespace collapsed; for a number found
    /// in the text, the number as written. `None` for an address written in
    /// the text part, or a link with no text (an image).
    pub text: Option<String>,
    /// Where a tracking link really goes, when its address says so
    /// (`target`): `https://www.youtube.com/live/…` behind
    /// `https://r.tracker.fr/lnk/…/aHR0cHM6Ly93d3cu…`.
    pub target: Option<String>,
    /// The other texts of the same link elsewhere in the message, each
    /// once, in order: a newsletter links the same page from a title, a
    /// picture and a button.
    pub other_texts: Vec<String>,
}

/// An `<a href>` of an HTML document: its address and its visible text,
/// both with entities decoded and whitespace collapsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anchor {
    pub href: String,
    pub text: String,
    /// Nothing visible separates it from the anchor before: only tags and
    /// spaces. A sender cutting one link into several (`<a>ressent </a><i>
    /// <a>"les angoisses"</a></i><a> du pays</a>`) gives touching anchors.
    pub touches_previous: bool,
    /// A space separates its text from the anchor before (at the end of
    /// that one, between them, or at the start of this one), to join the
    /// texts as they read.
    pub space_before: bool,
}

/// A link of the HTML as the reader sees it: touching anchors to the same
/// place are one.
struct Occurrence {
    url: String,
    key: String,
    target: Option<String>,
    text: String,
}

/// The links of a message, in order of appearance, each once: those of the
/// HTML (with their text), then the addresses of the text part not already
/// found, then its phone numbers.
///
/// "Once" means once per destination (`dedup_key`): the tracking links of a
/// newsletter differ for each place they are in, and lead to the same page.
/// The first text found names the link, the others are kept in
/// `other_texts`.
pub fn find(html: Option<&str>, text: Option<&str>) -> Vec<Link> {
    // 1. The anchors of the HTML, those that touch and go to the same
    //    place joined into one text.
    let mut occurrences: Vec<Occurrence> = Vec::new();
    for a in html.map(anchors).unwrap_or_default() {
        let href = a.href.trim();
        let url = if href.get(..4).is_some_and(|p| p.eq_ignore_ascii_case("tel:")) {
            crate::phones::tel_url(&href[4..])
        } else {
            is_followable(href).then(|| href.to_string())
        };
        let Some(url) = url else { continue };
        let target = target(&url);
        let key = dedup_key(target.as_deref().unwrap_or(&url));
        if a.touches_previous
            && let Some(last) = occurrences.last_mut()
            && last.key == key
        {
            if !a.text.is_empty() {
                if !last.text.is_empty() && a.space_before {
                    last.text.push(' ');
                }
                last.text.push_str(&a.text);
            }
            continue;
        }
        occurrences.push(Occurrence { url, key, target, text: a.text });
    }

    // 2. One link per destination.
    let mut out: Vec<Link> = Vec::new();
    // Rust note: a `HashMap` from a key to the place of its link in `out`.
    let mut index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    // `label`: the text is that of a link of the HTML, which counts among
    // the texts of the link; a number found in the text part only names a
    // link that has no text yet.
    let mut add = |url: String, key: String, target: Option<String>, text: Option<String>, label: bool, out: &mut Vec<Link>| {
        let text = text.filter(|t| !t.is_empty()).map(|t| t.chars().take(MAX_TEXT).collect::<String>());
        match index.get(&key) {
            Some(&i) => {
                let link = &mut out[i];
                match (&link.text, text) {
                    (None, Some(t)) => link.text = Some(t),
                    (Some(first), Some(t)) if label && *first != t && !link.other_texts.contains(&t) => {
                        link.other_texts.push(t)
                    }
                    _ => {}
                }
            }
            None if out.len() < MAX_LINKS => {
                index.insert(key, out.len());
                out.push(Link { url, text, target, other_texts: Vec::new() });
            }
            None => {}
        }
    };
    for o in occurrences {
        add(o.url, o.key, o.target, Some(o.text), true, &mut out);
    }
    for url in text.map(text_urls).unwrap_or_default() {
        let target = target(&url);
        let key = dedup_key(target.as_deref().unwrap_or(&url));
        add(url, key, target, None, false, &mut out);
    }
    for phone in text.map(crate::phones::find).unwrap_or_default() {
        let key = dedup_key(&phone.url);
        add(phone.url, key, None, Some(phone.written), false, &mut out);
    }
    out
}

/// What makes two links the same: the destination (the URL, or the target
/// of a tracking link), except for a phone number, known by its last nine
/// digits. The same number is often written twice,
/// `<a href="tel:+33612345678">06 12 34 56 78</a>` in the HTML and
/// "06 12 34 56 78" in the text: one row, not two.
fn dedup_key(url: &str) -> String {
    match url.strip_prefix("tel:") {
        Some(number) => {
            let digits: Vec<char> = number.chars().filter(char::is_ascii_digit).collect();
            format!("tel:{}", digits[digits.len().saturating_sub(9)..].iter().collect::<String>())
        }
        None => url.to_string(),
    }
}

/// Where a tracking link really goes, when its address carries it: the
/// first part of it (a segment of the path, or the value of a parameter)
/// that is, once decoded, a web address of its own. Senders write it
/// percent-encoded (`?url=https%3A%2F%2F…`) or in base64 (`/aHR0cHM6Ly8…`,
/// "https://" in base64). `None` for an ordinary link, or one whose
/// destination is hidden (an id the tracker looks up).
///
/// Used to know two links lead to the same page, and to show where a link
/// goes. Opening it still goes through the link as written: the decoded
/// address is a guess.
pub fn target(url: &str) -> Option<String> {
    let lower = url.get(..8)?.to_ascii_lowercase();
    if !lower.starts_with("https://") && !lower.starts_with("http://") {
        return None;
    }
    let after_scheme = url.find("://")? + 3;
    // The path and the query, without the host.
    let rest = &url[after_scheme..];
    let rest = &rest[rest.find('/').unwrap_or(rest.len())..];
    let rest = rest.split('#').next().unwrap_or("");
    for part in rest.split(['/', '?', '&', ';']) {
        // `name=value`: the value.
        let value = part.split_once('=').map_or(part, |(_, v)| v);
        if value.len() < 12 {
            continue;
        }
        let candidate = if value.get(..4).is_some_and(|p| p.eq_ignore_ascii_case("http")) || value.contains('%') {
            percent_decode(value)
        } else {
            base64_text(value)
        };
        if let Some(found) = candidate.map(|c| c.trim().to_string())
            && found.get(..8).is_some_and(|p| {
                let p = p.to_ascii_lowercase();
                p.starts_with("https://") || p.starts_with("http://")
            })
            && is_followable(&found)
            && has_plausible_host(&found)
            && !found.chars().any(char::is_control)
            && found != url
        {
            return Some(found);
        }
    }
    None
}

/// The host of a decoded address looks like one: letters, digits, dots
/// and dashes, at least one dot, not at either end (`http://../../x` is a
/// relative path a sender botched, not an address).
fn has_plausible_host(url: &str) -> bool {
    let after = &url[url.find("://").map_or(0, |i| i + 3)..];
    let host = after.split(['/', '?', '#', ':']).next().unwrap_or("");
    host.contains('.')
        && !host.starts_with('.')
        && !host.ends_with('.')
        && !host.contains("..")
        && host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

/// `%XX` sequences decoded (and `+` as a space, as in a form), if the
/// result is UTF-8.
fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            // `%` and two hexadecimal digits; a `%` without them stays.
            b'%' if i + 2 < b.len() && b[i + 1].is_ascii_hexdigit() && b[i + 2].is_ascii_hexdigit() => {
                let hex = std::str::from_utf8(&b[i + 1..i + 3]).ok()?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

/// A base64 text (standard or URL alphabet, padding optional) decoded, if
/// it is UTF-8; `None` as soon as a character is not base64.
fn base64_text(s: &str) -> Option<String> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
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
    String::from_utf8(out).ok()
}

/// Every `<a … href="…">…</a>` of an HTML document, in order. An anchor
/// without `href` is skipped; one left open at the end of the document
/// keeps its address, with no text.
pub fn anchors(html: &str) -> Vec<Anchor> {
    // ASCII lowercase keeps every byte at its place, so positions found in
    // `lower` are valid in `html`.
    let lower = html.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut from = 0;
    // End of the previous anchor with an address (after its `</a>`), and
    // whether its text ended with a space.
    let mut previous: Option<(usize, bool)> = None;
    while let Some(i) = lower[from..].find("<a").map(|i| i + from) {
        from = i + 2;
        // `<a` followed by a space or `>`, not `<abbr>`.
        if !lower[from..].starts_with(|c: char| c.is_ascii_whitespace() || c == '>') {
            continue;
        }
        let Some(tag_end) = tag_end(&lower, from) else { break };
        let href = attribute(&html[from..tag_end], "href");
        let close = lower[tag_end..].find("</a").map(|j| j + tag_end);
        let inner = close.map_or("", |c| &html[tag_end + 1..c]);
        let close_end = close.map(|c| lower[c..].find('>').map_or(lower.len(), |k| c + k + 1));
        if let Some(href) = href {
            let shown = visible(inner);
            let (touches_previous, space_before) = match previous {
                Some((end, ended_with_space)) if end <= i => {
                    let gap = visible(&html[end..i]);
                    let touches = gap.chars().all(|c| c.is_whitespace() || is_invisible(c));
                    let space = ended_with_space
                        || gap.chars().any(char::is_whitespace)
                        || shown.starts_with(char::is_whitespace);
                    (touches, space)
                }
                _ => (false, false),
            };
            out.push(Anchor {
                href: crate::html_text::decode_entities(&href),
                text: collapse(&crate::html_text::decode_entities(&strip_tags(inner))),
                touches_previous,
                space_before,
            });
            previous = close_end.map(|end| (end, shown.ends_with(char::is_whitespace)));
        }
        match close {
            Some(c) => from = c,
            None => break,
        }
    }
    out
}

/// The text a reader sees in an HTML fragment: tags removed (without the
/// space `strip_tags` puts for them, so that the spaces at the ends are
/// those of the text), entities decoded.
fn visible(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    crate::html_text::decode_entities(&out)
}

/// The addresses written in a plain text: `http://…`, `https://…` and
/// `mailto:…`, up to a space or a character that closes them (`>` of
/// `<https://…>`, `)` of "(https://…)"), without a final punctuation mark.
pub fn text_urls(text: &str) -> Vec<String> {
    let lower = text.to_ascii_lowercase();
    let mut out = Vec::new();
    // Next position of each prefix, searched again only once passed: a
    // prefix absent from the text is searched once, not at every address
    // (which made a text of many `https://` and no `mailto:` quadratic).
    const PREFIXES: [&str; 3] = ["https://", "http://", "mailto:"];
    let mut next: [Option<usize>; 3] = PREFIXES.map(|p| lower.find(p));
    let mut from = 0;
    while let Some(start) = next.iter().flatten().copied().min() {
        // A scheme inside a word ("xhttp://") is not a link.
        let glued = text[..start].chars().next_back().is_some_and(|c| c.is_alphanumeric());
        let url: String = text[start..]
            .chars()
            .take_while(|c| !c.is_whitespace() && !matches!(c, '<' | '>' | '"' | ')' | ']' | '}' | '|'))
            .collect();
        from = from.max(start + url.len());
        let url = url.trim_end_matches(['.', ',', ';', ':', '!', '?', '\'']);
        if !glued && is_followable(url) {
            out.push(url.to_string());
        }
        for (n, p) in PREFIXES.iter().enumerate() {
            if next[n].is_some_and(|at| at < from) {
                next[n] = lower[from..].find(p).map(|at| at + from);
            }
        }
    }
    out
}

/// A link the user can follow: web or mail, with something after the
/// scheme, not absurdly long. `javascript:`, `cid:`, anchors (`#top`) and
/// relative paths are left out.
fn is_followable(url: &str) -> bool {
    let l = url.get(..8).unwrap_or(url).to_ascii_lowercase();
    let rest = if l.starts_with("https://") {
        &url[8..]
    } else if l.starts_with("http://") || l.starts_with("mailto:") {
        &url[7..]
    } else {
        return false;
    };
    !rest.is_empty() && url.len() <= MAX_URL
}

/// Position of the `>` closing a tag that starts at `from`, skipping `>`
/// inside quoted attribute values.
pub(crate) fn tag_end(s: &str, from: usize) -> Option<usize> {
    let mut quote: Option<u8> = None;
    for (i, &b) in s.as_bytes()[from..].iter().enumerate() {
        match (quote, b) {
            (None, b'"' | b'\'') => quote = Some(b),
            (Some(q), _) if b == q => quote = None,
            (None, b'>') => return Some(from + i),
            _ => {}
        }
    }
    None
}

/// Value of the attribute `name` in the inside of a tag (`a href="…"`),
/// as written (entities not decoded).
pub(crate) fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(name).map(|i| i + from) {
        from = i + name.len();
        // A whole attribute name: preceded by whitespace, followed by `=`.
        let before_ok = i == 0 || lower.as_bytes()[i - 1].is_ascii_whitespace();
        let after = lower[from..].trim_start();
        if !before_ok || !after.starts_with('=') {
            continue;
        }
        let value_start = tag.len() - after.len() + 1;
        let value = tag[value_start..].trim_start();
        return Some(match value.chars().next() {
            Some(q @ ('"' | '\'')) => value[1..].split(q).next().unwrap_or("").to_string(),
            _ => value.split(|c: char| c.is_whitespace() || c == '>').next().unwrap_or("").to_string(),
        });
    }
    None
}

/// Text of an HTML fragment without its tags (each tag counts as a space).
pub(crate) fn strip_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

/// Single spaces between words, none at the ends (`&nbsp;` included: it
/// is whitespace for Rust's `split_whitespace`). The invisible characters
/// senders put in links (zero-width space, joiners, word joiner, BOM) are
/// spaces too: a link whose text is only a U+200B has no text.
fn collapse(s: &str) -> String {
    s.split(|c: char| c.is_whitespace() || is_invisible(c)).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
}

/// Characters that take no room on screen.
fn is_invisible(c: char) -> bool {
    matches!(c, '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{2060}' | '\u{FEFF}' | '\u{00AD}')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(url: &str, text: Option<&str>) -> Link {
        Link { url: url.into(), text: text.map(Into::into), target: None, other_texts: vec![] }
    }

    #[test]
    fn html_links_come_with_their_text() {
        let html = r#"<p>Votre facture est <a href="https://x.org/f?id=1&amp;k=2" target="_blank">ici</a>.
            <a href="https://t.example/c?u=9"><b>Voir</b>&nbsp;mon
            compte</a></p>"#;
        assert_eq!(
            find(Some(html), None),
            vec![link("https://x.org/f?id=1&k=2", Some("ici")), link("https://t.example/c?u=9", Some("Voir mon compte"))]
        );
    }

    #[test]
    fn a_link_without_text_keeps_its_address_only() {
        let html = r#"<a href="https://x.org/"><img src="cid:logo" alt="Logo"></a>"#;
        assert_eq!(find(Some(html), None), vec![link("https://x.org/", None)]);
    }

    #[test]
    fn only_web_and_mail_links_are_kept() {
        let html = r##"<a href="#top">Haut</a><a href="javascript:x()">X</a><a href="cid:a">A</a>
            <a href="/relatif">R</a><a href="https://">vide</a><a name="ancre">sans href</a>
            <a href="mailto:contact@x.org">Écrire</a><a href="HTTP://X.ORG/A">Maj</a>"##;
        assert_eq!(
            find(Some(html), None),
            vec![link("mailto:contact@x.org", Some("Écrire")), link("HTTP://X.ORG/A", Some("Maj"))]
        );
    }

    #[test]
    fn the_text_adds_the_addresses_the_html_does_not_have() {
        let html = r#"<a href="https://x.org/a">Lire</a>"#;
        let text = "Lire ( https://x.org/a )\nAutre : https://x.org/b.\nmailto:moi@x.org";
        assert_eq!(
            find(Some(html), Some(text)),
            vec![link("https://x.org/a", Some("Lire")), link("https://x.org/b", None), link("mailto:moi@x.org", None)]
        );
    }

    #[test]
    fn a_text_part_alone_gives_its_addresses_once_each() {
        let text = "Voir <https://x.org/a>, (http://x.org/b) et https://x.org/a!\nxhttps://non.example";
        assert_eq!(find(None, Some(text)), vec![link("https://x.org/a", None), link("http://x.org/b", None)]);
    }

    #[test]
    fn the_text_the_core_makes_from_html_has_no_address_left() {
        // The reason this module reads the HTML (`html_text.rs`).
        let html = r#"<p>Le rapport est <a href="https://x.org/r">ici</a>.</p>"#;
        let text = crate::html_text::html_to_text(html);
        assert!(text_urls(&text).is_empty());
        assert_eq!(find(Some(html), Some(&text)), vec![link("https://x.org/r", Some("ici"))]);
    }

    #[test]
    fn phone_numbers_come_from_tel_links_and_from_the_text_once_each() {
        // The same number as a link of the HTML and written in the text:
        // one row, with the text of the link.
        let html = r#"<p><a href="tel:+33 6 12 34 56 78">Nous appeler</a></p>"#;
        let text = "Nous appeler : 06 12 34 56 78\nStandard : 01 23 45 67 89";
        assert_eq!(
            find(Some(html), Some(text)),
            vec![
                link("tel:+33612345678", Some("Nous appeler")),
                link("tel:0123456789", Some("01 23 45 67 89")),
            ]
        );
    }

    #[test]
    fn touching_anchors_to_the_same_place_are_one_link_with_their_texts_joined() {
        // A sender cut one link into three (seen in a real newsletter): the
        // phrase reads as one link.
        let html = r#"<p>lui, <a href="https://x.org/v">ressent </a><i><a href="https://x.org/v">"les angoisses"</a></i><a href="https://x.org/v"> du pays</a>.</p>"#;
        assert_eq!(find(Some(html), None), vec![link("https://x.org/v", Some(r#"ressent "les angoisses" du pays"#))]);
        // No space between them: the words stay glued, as they read.
        let html = r#"<a href="https://x.org/v">Lire</a><b><a href="https://x.org/v">la suite</a></b>"#;
        assert_eq!(find(Some(html), None)[0].text.as_deref(), Some("Lirela suite"));
        // A space between the tags counts.
        let html = r#"<a href="https://x.org/v">Lire</a> <a href="https://x.org/v">la suite</a>"#;
        assert_eq!(find(Some(html), None)[0].text.as_deref(), Some("Lire la suite"));
    }

    #[test]
    fn anchors_apart_or_to_other_places_stay_apart() {
        // Text between them: two places in the mail, one link, two texts.
        let html = r#"<a href="https://x.org/v">Voir</a> ou <a href="https://x.org/v">ici</a>"#;
        let links = find(Some(html), None);
        assert_eq!(links.len(), 1);
        assert_eq!((links[0].text.as_deref(), links[0].other_texts.clone()), (Some("Voir"), vec!["ici".to_string()]));
        // Touching, to two places (icons of networks side by side).
        let html = r#"<a href="https://facebook.com/x">Facebook</a><a href="https://twitter.com/x">Twitter</a>"#;
        assert_eq!(find(Some(html), None).len(), 2);
    }

    #[test]
    fn the_same_destination_behind_several_tracking_links_is_one_link() {
        // "https://www.youtube.com/watch?v=abc" in base64, after two
        // different tracking paths.
        let b64 = "aHR0cHM6Ly93d3cueW91dHViZS5jb20vd2F0Y2g_dj1hYmM";
        let html = format!(
            r#"<a href="https://r.t.fr/lnk/AAA/13/sig1/{b64}">ressent</a><i><a href="https://r.t.fr/lnk/AAA/14/sig2/{b64}">"les angoisses"</a></i>
            <p>plus loin, <a href="https://r.t.fr/lnk/AAA/20/sig3/{b64}">la même vidéo</a> et <a href="https://r.t.fr/lnk/AAA/21/sig4/{b64}">ressent</a></p>"#
        );
        let links = find(Some(&html), None);
        assert_eq!(links.len(), 1);
        let l = &links[0];
        assert_eq!(l.text.as_deref(), Some(r#"ressent"les angoisses""#));
        assert_eq!(l.target.as_deref(), Some("https://www.youtube.com/watch?v=abc"));
        assert_eq!(l.url, format!("https://r.t.fr/lnk/AAA/13/sig1/{b64}"));
        // Each other text once; the first one is not repeated.
        assert_eq!(l.other_texts, vec!["la même vidéo".to_string(), "ressent".to_string()]);
    }

    #[test]
    fn invisible_texts_are_no_texts() {
        let html = "<a href=\"https://x.org/\">Abonnez-vous</a> <p>…</p><a href=\"https://x.org/\">\u{200B}</a>";
        let links = find(Some(html), None);
        assert_eq!((links[0].text.as_deref(), links[0].other_texts.len()), (Some("Abonnez-vous"), 0));
    }

    #[test]
    fn the_target_of_a_tracking_link_is_read_from_its_address() {
        // Base64 (URL alphabet, no padding) in the path.
        assert_eq!(
            target("https://r.t.fr/lnk/AcsAAE/15/LFq/aHR0cHM6Ly93d3cueW91dHViZS5jb20vbGl2ZS82c0FKMDk3UmF1ND90PTEyNzk").as_deref(),
            Some("https://www.youtube.com/live/6sAJ097Rau4?t=1279")
        );
        // Percent-encoded in a parameter.
        assert_eq!(
            target("http://www.x.com/redir.asp?Origin=27&Dest=http%3A%2F%2Fshop.example.com%2Fa%3Fb%3D1").as_deref(),
            Some("http://shop.example.com/a?b=1")
        );
        // Ordinary links, hidden destinations, botched decodings: none.
        for url in [
            "https://www.example.com/article/2026/10/15/titre-long-de-l-article",
            "https://r.t.fr/c/ABCDEFGHIJKLMNOP",
            "https://proxy.example.com/v4?u=http%3A%2F%2F..%2F..%2Fdocuments%2Fx",
            "https://twitter.com/intent/tweet?text=Une+le%C3%A7on+https%3A%2F%2Fttso.paris",
            "mailto:a@x.org",
        ] {
            assert_eq!(target(url), None, "{url}");
        }
    }

    #[test]
    fn tricky_markup_does_not_derail_the_scanner() {
        // `>` inside an attribute, `<abbr>`, an anchor left open at the end.
        let html = r#"<abbr>x</abbr><a title="a > b" href='https://x.org/1'>Un</a><a href="https://x.org/2">Deux"#;
        assert_eq!(
            find(Some(html), None),
            vec![link("https://x.org/1", Some("Un")), link("https://x.org/2", None)]
        );
        assert!(find(Some("<a href"), None).is_empty());
        assert!(find(Some(""), Some("")).is_empty());
    }

    #[test]
    fn long_texts_too_long_urls_and_too_many_links_are_bounded() {
        let long_text = "mot ".repeat(100);
        let html = format!(r#"<a href="https://x.org/">{long_text}</a>"#);
        assert_eq!(find(Some(&html), None)[0].text.as_ref().unwrap().chars().count(), MAX_TEXT);
        let huge = format!("https://x.org/{}", "a".repeat(MAX_URL));
        assert!(find(None, Some(&huge)).is_empty());
        let many: String = (0..MAX_LINKS + 10).map(|i| format!("https://x.org/{i} ")).collect();
        assert_eq!(find(None, Some(&many)).len(), MAX_LINKS);
    }
}
