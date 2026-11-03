//! Interpretation of a raw message: everything the database stores about a
//! message (headers, addresses, bodies, attachments) is extracted here, once,
//! when the message arrives (`stockage-brut.md`, section 1). Displaying a
//! message never reads the raw file again.
//!
//! This module is pure: bytes in, a `ParsedMessage` out. No database, no
//! file. That keeps it easy to test and safe to run on several threads at
//! once (the import parses messages in parallel).
//!
//! Parsing relies on the `mail-parser` crate (MIME, encodings, charsets) and
//! HTML sanitization on `ammonia`.

use std::borrow::Cow;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use mail_parser::{MessageParser, MessagePart, PartKind};

/// Version of the parser and of the sanitization, stored in
/// `message_body.parser_version`. Bump it when the extraction or the
/// sanitization changes: `reindex` will reinterpret older rows
/// (`stockage-brut.md`, section 8).
///
/// 2: the sanitized HTML keeps the markers of quoted history (see
/// `sanitize_html`).
/// 3: HTML-only messages are converted to text by `html_text.rs`, which
/// keeps blocks apart.
pub const PARSER_VERSION: i64 = 3;

/// Length of `message.snippet`, in characters.
const SNIPPET_CHARS: usize = 200;

/// Maximum number of `References` kept per message. Real threads rarely go
/// beyond a few dozen; a pathological header with thousands of ids would
/// only slow down threading.
const MAX_REFERENCES: usize = 100;

/// An address found in a header, already normalized: lowercase address,
/// trimmed display name (`None` if empty).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedAddress {
    pub addr: String,
    pub name: Option<String>,
}

/// Metadata of an attachment. The content stays in the raw file.
#[derive(Debug, Clone)]
pub struct ParsedAttachment {
    /// IMAP part number ("1", "2.1"…), the key used later to extract the
    /// attachment from the raw message (`stockage-brut.md`, section 6).
    pub part_id: String,
    pub filename: Option<String>,
    pub mime_type: String,
    /// Decoded size in bytes.
    pub size: i64,
    pub content_id: Option<String>,
    pub is_inline: bool,
}

/// Everything extracted from a message.
#[derive(Debug, Clone, Default)]
pub struct ParsedMessage {
    /// `Message-ID`, without angle brackets.
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    /// `References`, oldest first, then `In-Reply-To` if it was missing from
    /// them. Used for threading.
    pub references: Vec<String>,
    pub subject: String,
    pub from: Option<ParsedAddress>,
    pub to: Vec<ParsedAddress>,
    pub cc: Vec<ParsedAddress>,
    pub bcc: Vec<ParsedAddress>,
    pub reply_to: Vec<ParsedAddress>,
    /// `Date` header, Unix seconds.
    pub date_sent: Option<i64>,
    /// Plain text body: the text part, or the HTML part converted to text.
    pub text: Option<String>,
    /// Sanitized HTML body, if the message has an HTML part.
    pub html: Option<String>,
    /// Whether the HTML referenced remote images (removed by sanitization).
    pub has_remote_content: bool,
    pub snippet: Option<String>,
    pub attachments: Vec<ParsedAttachment>,
    /// `false` when `mail-parser` could not make sense of the bytes at all.
    /// The raw file is kept anyway, and `reindex` can retry later
    /// (`stockage-brut.md`, section 5, "Échec de l'analyse MIME").
    pub parsed: bool,
}

impl ParsedMessage {
    /// Whether the message has at least one attachment that is not an
    /// inline image of the body.
    pub fn has_attachments(&self) -> bool {
        self.attachments.iter().any(|a| !a.is_inline)
    }
}

/// Parses a raw RFC 5322 message.
pub fn parse(raw: &[u8]) -> ParsedMessage {
    let Some(message) = MessageParser::default().parse(raw) else {
        return ParsedMessage::default();
    };
    // Rust note: `root()` returns a lightweight view (`MessageRef`) that
    // borrows `message`. Strings returned by its accessors borrow the raw
    // bytes too ("zero-copy"); `.to_string()` / `.into_owned()` copy them
    // into owned `String`s that can outlive `raw`.
    let m = message.root();
    let message_id = m.message_id().map(clean_id);
    let in_reply_to = m.in_reply_to().and_then(|l| l.first().map(clean_id));
    let mut references: Vec<String> = m
        .references()
        .map(|l| l.iter().map(clean_id).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();
    if let Some(irt) = &in_reply_to
        && !references.contains(irt)
    {
        references.push(irt.clone());
    }
    // Keep the most recent ids: they are the closest ancestors.
    if references.len() > MAX_REFERENCES {
        references.drain(..references.len() - MAX_REFERENCES);
    }

    // The text part, or the HTML part converted to text by the core's own
    // conversion, which keeps blocks apart (`html_text.rs`).
    let text = m.text_body().next().and_then(|part| match part.kind() {
        PartKind::Html => part.text().map(|h| crate::html_text::html_to_text(&h)),
        _ => m.body_text(0).map(Cow::into_owned),
    });
    // Only a real HTML part is kept: mail-parser also offers a text part
    // converted to HTML, which would only duplicate `text`.
    let (html, has_remote_content) = match m.html_body().next() {
        Some(part) if matches!(part.kind(), PartKind::Html) => match part.text() {
            Some(h) => {
                let (clean, remote) = sanitize_html(&h);
                (Some(clean), remote)
            }
            None => (None, false),
        },
        _ => (None, false),
    };
    let snippet = text.as_deref().map(snippet).filter(|s| !s.is_empty());

    let mut attachments = Vec::new();
    collect_attachments(m.root_part(), "", true, &mut attachments);

    ParsedMessage {
        message_id: message_id.filter(|s| !s.is_empty()),
        in_reply_to: in_reply_to.filter(|s| !s.is_empty()),
        references,
        subject: m.subject().map(|s| s.trim().to_string()).unwrap_or_default(),
        from: m.from().and_then(|l| l.first()).and_then(|mb| address(mb.address(), mb.name())),
        to: addresses(m.to()),
        cc: addresses(m.cc()),
        bcc: addresses(m.bcc()),
        reply_to: addresses(m.reply_to()),
        // A date that is not valid (year 0, day 31 of February…) is ignored
        // rather than stored as a nonsensical timestamp.
        date_sent: m.date().filter(|d| d.is_valid()).map(|d| d.to_timestamp()),
        text,
        html,
        has_remote_content,
        snippet,
        attachments,
        parsed: true,
    }
}

/// Removes the angle brackets and spaces around a message id.
fn clean_id(s: &str) -> String {
    s.trim().trim_start_matches('<').trim_end_matches('>').trim().to_string()
}

/// Normalizes one address. Addresses without `@` (undisclosed recipients,
/// broken headers) are dropped: they cannot identify a correspondent.
fn address(addr: Option<&str>, name: Option<&str>) -> Option<ParsedAddress> {
    let addr = addr?.trim().to_lowercase();
    if !addr.contains('@') || addr.len() > 320 {
        return None;
    }
    let name = name.map(|n| n.trim().trim_matches('"').trim().to_string()).filter(|n| !n.is_empty() && *n != addr);
    Some(ParsedAddress { addr, name })
}

/// All the mailboxes of an address header, groups flattened, duplicates
/// removed (keeping the first occurrence).
fn addresses(list: Option<mail_parser::AddressList<'_>>) -> Vec<ParsedAddress> {
    let mut out: Vec<ParsedAddress> = Vec::new();
    for mb in list.iter().flat_map(|l| l.mailboxes()) {
        if let Some(a) = address(mb.address(), mb.name())
            && !out.iter().any(|o| o.addr == a.addr)
        {
            out.push(a);
        }
    }
    out
}

/// First characters of the text, whitespace collapsed, for the list
/// preview (`message.snippet`).
pub fn snippet(text: &str) -> String {
    let mut out = String::new();
    let mut count = 0;
    for word in text.split_whitespace() {
        if count > 0 {
            out.push(' ');
            count += 1;
        }
        for c in word.chars() {
            if count >= SNIPPET_CHARS {
                return out;
            }
            out.push(c);
            count += 1;
        }
        if count >= SNIPPET_CHARS {
            break;
        }
    }
    out
}

/// Walks the MIME tree and records every attachment with its IMAP part
/// number.
///
/// IMAP numbering (RFC 3501, section 6.4.5): the children of a multipart
/// are numbered 1, 2, 3…; a part inside the second child is "2.1", and so
/// on. A message that is not multipart has a single part, "1". A nested
/// message (`message/rfc822`) is recorded as one attachment and not walked
/// into: the user opens it as a whole.
///
/// `is_root` distinguishes the top-level part, which has no number of its
/// own when it is a multipart.
fn collect_attachments(part: MessagePart<'_>, prefix: &str, is_root: bool, out: &mut Vec<ParsedAttachment>) {
    if part.is_multipart() {
        for (i, child) in part.children().enumerate() {
            let id = if prefix.is_empty() { format!("{}", i + 1) } else { format!("{prefix}.{}", i + 1) };
            collect_attachments(child, &id, false, out);
        }
        return;
    }
    let part_id = if is_root { "1".to_string() } else { prefix.to_string() };
    // A part is an attachment if mail-parser lists it as one (by
    // disposition, type and position in the tree) and it is not a body.
    if !part.is_attachment() || part.in_text_body() || part.in_html_body() {
        return;
    }
    let mime_type = part
        .content_type()
        .map(|ct| match ct.subtype() {
            Some(sub) => format!("{}/{}", ct.ctype(), sub),
            None => ct.ctype().to_string(),
        })
        .unwrap_or_else(|| "application/octet-stream".to_string())
        .to_lowercase();
    let content_id = part.content_id().map(clean_id).filter(|s| !s.is_empty());
    // An inline part referenced by the HTML (`cid:`) is part of the body
    // rather than a file the user downloads.
    let is_inline = part.is_inline() && content_id.is_some();
    out.push(ParsedAttachment {
        part_id,
        filename: part.attachment_name().map(|n| n.trim().to_string()).filter(|n| !n.is_empty()),
        mime_type,
        size: part.decoded_len() as i64,
        content_id,
        is_inline,
    });
}

/// Sanitizes an HTML body for display: removes scripts, event handlers,
/// forms, styles and anything ammonia does not allow, and drops remote
/// images (tracking pixels, `modele de donnees.md`). Returns the clean HTML
/// and whether remote content was removed.
///
/// Images embedded in the message (`cid:` URLs) are kept, and so are the
/// markers mail apps put on quoted history and signatures, so that clients
/// can fold them (`docs/thread-display.md`): Gmail's `gmail_quote`,
/// `gmail_attr` and `gmail_signature` classes, Thunderbird's
/// `moz-cite-prefix` and `moz-signature`, Apple Mail's
/// `<blockquote type="cite">`, Outlook's `divRplyFwdMsg` and `appendonsend`
/// ids. Ids are prefixed with `nm-` so a mail cannot clash with the app's
/// own ids.
pub fn sanitize_html(html: &str) -> (String, bool) {
    // The filter closure must be `'static`, so the flag is shared through an
    // `Arc` (a reference-counted pointer) rather than borrowed.
    //
    // Rust note: `AtomicBool` can be modified through a shared reference,
    // safely from several threads; plain `bool` could not be changed from
    // inside a `Fn` closure.
    let remote = Arc::new(AtomicBool::new(false));
    let flag = remote.clone();
    let clean = ammonia::Builder::default()
        .add_url_schemes(["cid"])
        .add_allowed_classes("div", ["gmail_quote", "gmail_attr", "gmail_signature", "moz-cite-prefix", "moz-signature"])
        .add_allowed_classes("blockquote", ["gmail_quote"])
        .add_tag_attributes("blockquote", ["type"])
        .add_tag_attributes("div", ["id"])
        .id_prefix(Some("nm-"))
        .attribute_filter(move |element, attribute, value| {
            if element == "img" && attribute == "src" {
                let v = value.trim_start().to_ascii_lowercase();
                if v.starts_with("http:") || v.starts_with("https:") || v.starts_with("//") {
                    flag.store(true, Ordering::Relaxed);
                    return None;
                }
            }
            Some(value.into())
        })
        .clean(html)
        .to_string();
    (clean, remote.load(Ordering::Relaxed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_headers_and_normalizes_addresses() {
        let raw = b"Message-ID: <abc@x.org>\r\nIn-Reply-To: <p2@x.org>\r\nReferences: <p1@x.org> <p2@x.org>\r\n\
From: \"Alice Martin\" <Alice@X.org>\r\nTo: bob@y.org, Undisclosed recipients:;\r\nCc: =?utf-8?Q?Ren=C3=A9?= <rene@z.org>, bob@y.org\r\n\
Subject:  Bonjour =?utf-8?Q?=C3=A0_tous?= \r\nDate: Tue, 23 Jun 2020 21:20:56 +0200\r\n\r\nHello\r\n";
        let p = parse(raw);
        assert!(p.parsed);
        assert_eq!(p.message_id.as_deref(), Some("abc@x.org"));
        assert_eq!(p.in_reply_to.as_deref(), Some("p2@x.org"));
        assert_eq!(p.references, vec!["p1@x.org", "p2@x.org"]);
        assert_eq!(p.from, Some(ParsedAddress { addr: "alice@x.org".into(), name: Some("Alice Martin".into()) }));
        assert_eq!(p.to.len(), 1);
        assert_eq!(p.cc.iter().map(|a| a.addr.as_str()).collect::<Vec<_>>(), ["rene@z.org", "bob@y.org"]);
        assert_eq!(p.cc[0].name.as_deref(), Some("René"));
        assert_eq!(p.subject, "Bonjour à tous");
        assert_eq!(p.date_sent, Some(1_592_940_056));
        assert_eq!(p.text.as_deref().map(str::trim), Some("Hello"));
        assert!(p.html.is_none());
    }

    #[test]
    fn in_reply_to_is_added_to_references() {
        let p = parse(b"In-Reply-To: <p@x>\r\nSubject: s\r\n\r\nb");
        assert_eq!(p.references, vec!["p@x"]);
    }

    #[test]
    fn html_only_message_gets_text_and_sanitized_html() {
        let raw = b"Content-Type: text/html; charset=utf-8\r\nSubject: s\r\n\r\n\
<html><body><p>Caf\xc3\xa9 <b>ok</b></p><script>alert(1)</script><img src=\"https://t.example/p.gif\"><img src=\"cid:logo\"><a href=\"https://x.org\" onclick=\"x()\">lien</a></body></html>";
        let p = parse(raw);
        let html = p.html.unwrap();
        assert!(!html.contains("script") && !html.contains("alert"));
        assert!(!html.contains("t.example"));
        assert!(html.contains("cid:logo"));
        assert!(!html.contains("onclick"));
        assert!(p.has_remote_content);
        let text = p.text.unwrap();
        assert!(text.contains("Café"));
        assert!(p.snippet.unwrap().starts_with("Café ok"));
    }

    #[test]
    fn attachments_get_imap_part_numbers() {
        let raw = b"Subject: s\r\nContent-Type: multipart/mixed; boundary=\"b1\"\r\n\r\n\
--b1\r\nContent-Type: multipart/alternative; boundary=\"b2\"\r\n\r\n\
--b2\r\nContent-Type: text/plain\r\n\r\ntext\r\n--b2\r\nContent-Type: text/html\r\n\r\n<p>html</p>\r\n--b2--\r\n\
--b1\r\nContent-Type: application/pdf; name=\"facture.pdf\"\r\nContent-Disposition: attachment; filename=\"facture.pdf\"\r\nContent-Transfer-Encoding: base64\r\n\r\nJVBERi0xLjQK\r\n\
--b1\r\nContent-Type: image/png\r\nContent-Disposition: inline\r\nContent-ID: <logo@x>\r\nContent-Transfer-Encoding: base64\r\n\r\niVBORw0K\r\n--b1--\r\n";
        let p = parse(raw);
        assert_eq!(p.text.as_deref().map(str::trim), Some("text"));
        assert!(p.html.as_deref().unwrap().contains("html"));
        let pdf = p.attachments.iter().find(|a| a.mime_type == "application/pdf").expect("pdf");
        assert_eq!(pdf.part_id, "2");
        assert_eq!(pdf.filename.as_deref(), Some("facture.pdf"));
        assert_eq!(pdf.size, 9);
        assert!(!pdf.is_inline);
        if let Some(png) = p.attachments.iter().find(|a| a.mime_type == "image/png") {
            assert_eq!(png.part_id, "3");
            assert!(png.is_inline);
            assert_eq!(png.content_id.as_deref(), Some("logo@x"));
        }
        assert!(p.has_attachments());
    }

    #[test]
    fn single_part_attachment_is_part_1() {
        let raw = b"Subject: s\r\nContent-Type: application/pdf\r\nContent-Disposition: attachment; filename=a.pdf\r\n\r\nxx";
        let p = parse(raw);
        assert_eq!(p.attachments.len(), 1);
        assert_eq!(p.attachments[0].part_id, "1");
    }

    #[test]
    fn snippet_collapses_whitespace_and_truncates() {
        assert_eq!(snippet("  a\n\n b\tc  "), "a b c");
        let long = "é".repeat(500);
        assert_eq!(snippet(&long).chars().count(), SNIPPET_CHARS);
    }

    #[test]
    fn garbage_still_produces_a_message() {
        // mail-parser is lenient: arbitrary bytes are parsed as a body.
        let p = parse(b"\x00\x01\x02 not a mail");
        assert!(p.subject.is_empty());
    }

    #[test]
    fn quote_markers_survive_sanitization() {
        let html = r#"<div>OK</div><div class="gmail_quote evil"><div class="gmail_attr">On Mon, A wrote:</div><blockquote class="gmail_quote" type="cite" onclick="x()">old</blockquote></div><div id="divRplyFwdMsg">From: A</div>"#;
        let (clean, _) = sanitize_html(html);
        assert!(clean.contains(r#"class="gmail_quote""#), "{clean}");
        assert!(clean.contains(r#"class="gmail_attr""#));
        assert!(clean.contains(r#"type="cite""#));
        assert!(clean.contains(r#"id="nm-divRplyFwdMsg""#));
        assert!(!clean.contains("evil") && !clean.contains("onclick"));
    }
}
