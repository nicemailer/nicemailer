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
}

/// An `<a href>` of an HTML document: its address and its visible text,
/// both with entities decoded and whitespace collapsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anchor {
    pub href: String,
    pub text: String,
}

/// The links of a message, in order of appearance, each once: those of the
/// HTML (with their text), then the addresses of the text part not already
/// found, then its phone numbers.
pub fn find(html: Option<&str>, text: Option<&str>) -> Vec<Link> {
    let mut out: Vec<Link> = Vec::new();
    // Rust note: a `HashSet<String>` keeps each key once; `insert` returns
    // `false` when it was already there, which is the duplicate test.
    let mut seen = std::collections::HashSet::new();
    let mut push = |url: String, text: Option<String>, out: &mut Vec<Link>| {
        if out.len() < MAX_LINKS && seen.insert(dedup_key(&url)) {
            out.push(Link { url, text });
        }
    };
    for a in html.map(anchors).unwrap_or_default() {
        let href = a.href.trim();
        let url = if href.get(..4).is_some_and(|p| p.eq_ignore_ascii_case("tel:")) {
            crate::phones::tel_url(&href[4..])
        } else {
            is_followable(href).then(|| href.to_string())
        };
        if let Some(url) = url {
            let text = (!a.text.is_empty()).then(|| a.text.chars().take(MAX_TEXT).collect());
            push(url, text, &mut out);
        }
    }
    for url in text.map(text_urls).unwrap_or_default() {
        push(url, None, &mut out);
    }
    for phone in text.map(crate::phones::find).unwrap_or_default() {
        push(phone.url, Some(phone.written), &mut out);
    }
    out
}

/// What makes two links the same: the URL, except for a phone number,
/// known by its last nine digits. The same number is often written twice,
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

/// Every `<a … href="…">…</a>` of an HTML document, in order. An anchor
/// without `href` is skipped; one left open at the end of the document
/// keeps its address, with no text.
pub fn anchors(html: &str) -> Vec<Anchor> {
    // ASCII lowercase keeps every byte at its place, so positions found in
    // `lower` are valid in `html`.
    let lower = html.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut from = 0;
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
        if let Some(href) = href {
            out.push(Anchor {
                href: crate::html_text::decode_entities(&href),
                text: collapse(&crate::html_text::decode_entities(&strip_tags(inner))),
            });
        }
        match close {
            Some(c) => from = c,
            None => break,
        }
    }
    out
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
/// is whitespace for Rust's `split_whitespace`).
fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(url: &str, text: Option<&str>) -> Link {
        Link { url: url.into(), text: text.map(Into::into) }
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
