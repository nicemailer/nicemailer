//! Finding how to unsubscribe from a mailing list (`ui.md`, "Désabonnement").
//!
//! Done once, when the message arrives (`ingest.rs`), and stored in
//! `message_unsubscribe`. Two sources, in this order:
//!
//! 1. The `List-Unsubscribe` header (RFC 2369): `<https://…>` and/or
//!    `<mailto:…>`, written for machines, hence reliable. With
//!    `List-Unsubscribe-Post: List-Unsubscribe=One-Click` (RFC 8058) and an
//!    https URL, the core can unsubscribe by itself with one POST request.
//! 2. Otherwise a link of the body whose text announces an unsubscription
//!    ("unsubscribe", "se désabonner", "abmelden"…): only good to open in a
//!    browser, since nothing says what the page does.
//!
//! This module is pure: strings in, an `Unsubscribe` out.

/// Where the unsubscribe link was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Header,
    Body,
}

impl Source {
    /// Value stored in `message_unsubscribe.source`.
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Header => "header",
            Source::Body => "body",
        }
    }
}

/// How to unsubscribe from the list a message comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unsubscribe {
    /// https (or http) URL; `None` when the header only gives an address.
    pub url: Option<String>,
    /// `mailto:` URL, from the header only.
    pub mailto: Option<String>,
    /// RFC 8058: one POST to `url` unsubscribes.
    pub one_click: bool,
    pub source: Source,
}

/// Longest URL kept (shared with the `l` panel).
use crate::links::MAX_URL;

/// Words announcing an unsubscription, folded (lowercase, no accents), in
/// the languages of the mail the user is likely to receive.
const KEYWORDS: &[&str] = &[
    "unsubscribe",
    "opt out",
    "opt-out",
    "desabonner",
    "desabonnement",
    "desabonnez",
    "desinscri", // désinscrire, désinscription, désinscrivez
    "abmelden",
    "abbestellen",
    "darse de baja",
    "cancelar suscripcion",
    "disiscriviti",
    "annulla l'iscrizione",
    "uitschrijven",
    "afmelden",
];

/// The unsubscribe of a message: from the header if it has a usable one,
/// else from the HTML body, else from the text body.
pub fn find(header: Option<&str>, post: Option<&str>, html: Option<&str>, text: Option<&str>) -> Option<Unsubscribe> {
    if let Some(u) = header.and_then(|h| from_header(h, post)) {
        return Some(u);
    }
    let url = html.and_then(from_html).or_else(|| text.and_then(from_text))?;
    Some(Unsubscribe { url: Some(url), mailto: None, one_click: false, source: Source::Body })
}

/// Reads `List-Unsubscribe` (and `List-Unsubscribe-Post`). The header is a
/// list of URLs in angle brackets, possibly folded over several lines:
/// `<mailto:l@x.org?subject=unsub>, <https://x.org/u?id=1>`. The first web
/// URL and the first `mailto:` are kept; other schemes are ignored.
pub fn from_header(value: &str, post: Option<&str>) -> Option<Unsubscribe> {
    let mut items: Vec<String> = Vec::new();
    let mut rest = value;
    // Rust note: `while let` loops as long as the pattern matches; here,
    // as long as an opening bracket is left.
    while let Some(start) = rest.find('<') {
        let Some(len) = rest[start + 1..].find('>') else { break };
        // A folded header can break a URL across lines: drop all whitespace.
        items.push(rest[start + 1..start + 1 + len].split_whitespace().collect());
        rest = &rest[start + 1 + len + 1..];
    }
    if items.is_empty() {
        // Some senders forget the brackets.
        items = value.split(',').map(|s| s.split_whitespace().collect()).collect();
    }
    let url = items.iter().find(|u| is_web(u)).filter(|u| u.len() <= MAX_URL).cloned();
    let mailto = items
        .iter()
        .find(|u| u.len() > 7 && u[..7].eq_ignore_ascii_case("mailto:") && u.contains('@'))
        .filter(|u| u.len() <= MAX_URL)
        .cloned();
    if url.is_none() && mailto.is_none() {
        return None;
    }
    let one_click = post.is_some_and(|p| p.to_ascii_lowercase().contains("list-unsubscribe=one-click"))
        && url.as_deref().is_some_and(|u| u[..8].eq_ignore_ascii_case("https://"));
    Some(Unsubscribe { url, mailto, one_click, source: Source::Header })
}

/// The first link `<a href="http…">` whose text (or address) announces an
/// unsubscription. The HTML is the core's sanitized copy, read by the
/// anchor scanner of `links.rs`.
pub fn from_html(html: &str) -> Option<String> {
    crate::links::anchors(html).into_iter().find_map(|a| {
        let href = a.href;
        if !is_web(&href) || href.len() > MAX_URL {
            return None;
        }
        let folded_href = fold(&href);
        (has_keyword(&fold(&a.text)) || ["unsubscribe", "desabonn", "optout", "opt-out"].iter().any(|k| folded_href.contains(k)))
            .then_some(href)
    })
}

/// In a plain text body, a URL on a line that announces an unsubscription
/// ("Se désabonner : https://…"), or on the line right after it.
pub fn from_text(text: &str) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        if !has_keyword(&fold(line)) {
            continue;
        }
        if let Some(url) = first_url(line).or_else(|| lines.get(i + 1).and_then(|l| first_url(l))) {
            return Some(url);
        }
    }
    None
}

fn is_web(u: &str) -> bool {
    let l = u.get(..8).unwrap_or(u).to_ascii_lowercase();
    l.starts_with("https://") || l.starts_with("http://")
}

fn has_keyword(folded: &str) -> bool {
    KEYWORDS.iter().any(|k| folded.contains(k))
}

/// The first http(s) URL of a line, up to a space or a closing character.
fn first_url(line: &str) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    let start = [lower.find("https://"), lower.find("http://")].into_iter().flatten().min()?;
    let url: String = line[start..]
        .chars()
        .take_while(|c| !c.is_whitespace() && !matches!(c, '<' | '>' | '"' | ')' | ']'))
        .collect();
    let url = url.trim_end_matches(['.', ',', ';', ':']).to_string();
    (url.len() > 8 && url.len() <= MAX_URL).then_some(url)
}

/// Lowercase without the accents of the Latin languages, and with single
/// spaces, to compare with `KEYWORDS`.
fn fold(s: &str) -> String {
    let lower: String = s
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'à' | 'â' | 'ä' | 'á' | 'ã' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'î' | 'ï' | 'í' | 'ì' => 'i',
            'ô' | 'ö' | 'ó' | 'ò' | 'õ' => 'o',
            'ù' | 'û' | 'ü' | 'ú' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            '\u{2019}' => '\'',
            c => c,
        })
        .collect();
    lower.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_header_gives_a_web_url_an_address_and_one_click() {
        let u = from_header(
            "<mailto:list@x.org?subject=unsubscribe>,\r\n <https://x.org/u?id=1&k=2>",
            Some("List-Unsubscribe=One-Click"),
        )
        .unwrap();
        assert_eq!(u.url.as_deref(), Some("https://x.org/u?id=1&k=2"));
        assert_eq!(u.mailto.as_deref(), Some("mailto:list@x.org?subject=unsubscribe"));
        assert!(u.one_click);
        assert_eq!(u.source, Source::Header);
    }

    #[test]
    fn one_click_needs_the_post_header_and_https() {
        assert!(!from_header("<https://x.org/u>", None).unwrap().one_click);
        assert!(!from_header("<http://x.org/u>", Some("List-Unsubscribe=One-Click")).unwrap().one_click);
        // An address only: no web URL, no one click.
        let m = from_header("<mailto:l@x.org>", Some("List-Unsubscribe=One-Click")).unwrap();
        assert_eq!((m.url, m.one_click), (None, false));
    }

    #[test]
    fn folded_urls_missing_brackets_and_junk_are_handled() {
        // A URL cut by the folding of a long header.
        let u = from_header("<https://x.org/very/long/\r\n path?id=1>", None).unwrap();
        assert_eq!(u.url.as_deref(), Some("https://x.org/very/long/path?id=1"));
        // No brackets at all.
        assert_eq!(from_header("https://x.org/u", None).unwrap().url.as_deref(), Some("https://x.org/u"));
        // Nothing usable: other schemes, an address without @.
        assert!(from_header("<ftp://x.org/u>, <mailto:nobody>", None).is_none());
        assert!(from_header("", None).is_none());
    }

    #[test]
    fn a_link_of_the_html_body_is_found_by_its_text_in_several_languages() {
        let html = r#"<p>Lettre</p><a href="https://x.org/news">Lire</a> <a class="f" href="https://x.org/u?a=1&amp;b=2">Se <b>d&eacute;sabonner</b></a>"#;
        assert_eq!(from_html(html).as_deref(), Some("https://x.org/u?a=1&b=2"));
        for text in ["Unsubscribe", "Désinscription", "Hier abmelden", "Darse de baja", "Uitschrijven", "Opt out"] {
            let html = format!("<a href='https://x.org/x'>{text}</a>");
            assert_eq!(from_html(&html).as_deref(), Some("https://x.org/x"), "{text}");
        }
        // Or by the link itself.
        assert_eq!(
            from_html(r#"<a href="https://x.org/unsubscribe?id=9">click here</a>"#).as_deref(),
            Some("https://x.org/unsubscribe?id=9")
        );
    }

    #[test]
    fn other_links_and_other_schemes_are_not_unsubscribe_links() {
        assert!(from_html(r#"<a href="https://x.org/a">Lire la suite</a><abbr>unsubscribe</abbr>"#).is_none());
        assert!(from_html(r#"<a href="javascript:x()">Unsubscribe</a>"#).is_none());
        assert!(from_html(r#"<a href="mailto:l@x.org">Unsubscribe</a>"#).is_none());
        // A `>` inside an attribute does not end the tag.
        assert_eq!(
            from_html(r#"<a title="a > b" href="https://x.org/u">Se désabonner</a>"#).as_deref(),
            Some("https://x.org/u")
        );
        assert!(from_html("<a href=\"https://x.org/u\">Se désabonner").is_none());
    }

    #[test]
    fn a_plain_text_body_gives_the_url_on_or_after_the_line() {
        assert_eq!(
            from_text("Bonne lecture !\n\nSe désabonner : https://x.org/desabonnement.").as_deref(),
            Some("https://x.org/desabonnement")
        );
        assert_eq!(from_text("To unsubscribe, visit:\n<https://x.org/u>").as_deref(), Some("https://x.org/u"));
        assert!(from_text("Unsubscribe by replying STOP.").is_none());
        assert!(from_text("Voir https://x.org/news").is_none());
    }

    #[test]
    fn the_header_wins_over_the_body() {
        let u = find(Some("<https://x.org/h>"), None, Some(r#"<a href="https://x.org/b">Unsubscribe</a>"#), None).unwrap();
        assert_eq!((u.url.as_deref(), u.source), (Some("https://x.org/h"), Source::Header));
        let b = find(None, None, Some(r#"<a href="https://x.org/b">Unsubscribe</a>"#), None).unwrap();
        assert_eq!((b.url.as_deref(), b.source, b.one_click), (Some("https://x.org/b"), Source::Body, false));
        let t = find(Some("<ftp://x>"), None, None, Some("Unsubscribe: https://x.org/t")).unwrap();
        assert_eq!(t.url.as_deref(), Some("https://x.org/t"));
        assert!(find(None, None, Some("<p>Bonjour</p>"), Some("Bonjour")).is_none());
    }
}
