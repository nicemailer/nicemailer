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
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

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
/// 4: images shown inline in a mail without HTML (Apple Mail) are recorded
/// as attachments, and "inline" means "referenced by the HTML" (`cid:`).
/// 5: how to unsubscribe from a mailing list (`unsubscribe.rs`).
/// 6: a text part holding HTML entities (`&nbsp;`) or HTML markup
/// (`</p>`, `<br />`, `target="_blank"`) is replaced by the conversion of
/// the HTML part; without HTML part, its entities are decoded (see
/// `parse`).
/// 7: entities encoded twice by the sender (`&amp;nbsp;`) are encoded once
/// in the HTML and decoded in the text (`html_text::undouble_entities`).
/// 8: remote images keep their URL, in the app's `nm-remote:` scheme, so a
/// client can load them on request; tracking pixels are removed and counted
/// (`remote_images.rs`).
pub const PARSER_VERSION: i64 = 8;

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
    /// Whether the sanitized HTML holds remote images a client may load on
    /// request (`nm-remote:` sources, `remote_images.rs`). Tracking pixels
    /// do not count: they are removed.
    pub has_remote_content: bool,
    /// Remote tracking pixels removed from the HTML.
    pub tracking_pixels: i64,
    pub snippet: Option<String>,
    pub attachments: Vec<ParsedAttachment>,
    /// How to unsubscribe from the list the message comes from, if it says.
    pub unsubscribe: Option<crate::unsubscribe::Unsubscribe>,
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

    // The real HTML part, if any (mail-parser can also offer a text part
    // converted to HTML, which is not one), with the entities its sender
    // encoded twice ("Zoom :&amp;nbsp;https://…", shown "&nbsp;" by every
    // client) encoded once, for the HTML view and for the text made from it.
    let html_part = m
        .html_body()
        .next()
        .filter(|p| matches!(p.kind(), PartKind::Html))
        .and_then(|p| p.text())
        .map(|h| crate::html_text::undouble_entities(&h).into_owned());
    // The text part, or the HTML part converted to text by the core's own
    // conversion, which keeps blocks apart (`html_text.rs`).
    //
    // Some senders generate their text part from their HTML and botch it:
    // entities left as written ("l'État de droit&nbsp;(elle a…"), tags and
    // the remains of attributes ("<em>Espace Client</em>", "\" target=
    // \"_blank\" style=…"). Such a part is also poor otherwise: paragraphs
    // glued together ("complet.Si vous"), whole paragraphs missing. When the
    // text part shows these marks (`html_text::has_entity`,
    // `has_html_markup`) and the mail has an HTML part, the text is the
    // core's conversion of the HTML instead. A false alarm (a text quoting
    // HTML) costs nothing: the HTML part says the same thing. Without an
    // HTML part, only the entities are decoded: tags in a plain text may be
    // meant.
    let text = m.text_body().next().and_then(|part| match part.kind() {
        PartKind::Html => part.text().map(|h| crate::html_text::html_to_text(&h)),
        _ => m.body_text(0).map(|t| {
            let entities = crate::html_text::has_entity(&t);
            match &html_part {
                Some(h) if entities || crate::html_text::has_html_markup(&t) => crate::html_text::html_to_text(h),
                // Decoded until none is left: entities encoded twice too.
                None if entities => crate::html_text::decode_all_entities(&t).into_owned(),
                _ => t.into_owned(),
            }
        }),
    })
    // Entities still in the text after one decoding were encoded several
    // times by the sender ("&amp;amp;#039;"): decoded until none is left,
    // as the repair of stored bodies does (`reindex.rs`, `repair_stored`).
    .map(|t| match crate::html_text::decode_all_entities(&t) {
        Cow::Owned(decoded) => decoded,
        Cow::Borrowed(_) => t,
    });
    // Only a real HTML part is kept: mail-parser also offers a text part
    // converted to HTML, which would only duplicate `text`.
    let (html, has_remote_content, tracking_pixels) = match &html_part {
        Some(h) => {
            let s = sanitize_html(h);
            (Some(s.html), s.remote_images, s.tracking_pixels)
        }
        None => (None, false, 0),
    };
    let snippet = text.as_deref().map(snippet).filter(|s| !s.is_empty());

    let mut attachments = Vec::new();
    collect_attachments(m.root_part(), "", true, html.as_deref(), &mut attachments);

    // The raw values of the list headers: `unsubscribe.rs` reads the URLs
    // in them itself (mail-parser's address reading would drop `mailto:`
    // parameters such as `?subject=`).
    let header = |name: &str| m.headers().get(name).map(|h| String::from_utf8_lossy(h.raw_value()).into_owned());
    let unsubscribe = crate::unsubscribe::find(
        header("List-Unsubscribe").as_deref(),
        header("List-Unsubscribe-Post").as_deref(),
        html.as_deref(),
        text.as_deref(),
    );

    ParsedMessage {
        message_id: message_id.filter(|s| !s.is_empty()),
        in_reply_to: in_reply_to.filter(|s| !s.is_empty()),
        references,
        // `decode_leftovers`: encoded words `mail-parser` did not decode
        // because they are glued inside a word (`encoded_words.rs`).
        subject: m.subject().map(|s| crate::encoded_words::decode_leftovers(s.trim()).into_owned()).unwrap_or_default(),
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
        tracking_pixels,
        snippet,
        attachments,
        unsubscribe,
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
    // Encoded words glued inside the name (`Ber=?ISO-8859-1?B?6A==?=s`) are
    // decoded here: `mail-parser` only decodes whole words.
    let name = name
        .map(|n| crate::encoded_words::decode_leftovers(n.trim().trim_matches('"').trim()).trim().to_string())
        .filter(|n| !n.is_empty() && *n != addr);
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
fn collect_attachments(
    part: MessagePart<'_>,
    prefix: &str,
    is_root: bool,
    html: Option<&str>,
    out: &mut Vec<ParsedAttachment>,
) {
    if part.is_multipart() {
        for (i, child) in part.children().enumerate() {
            let id = if prefix.is_empty() { format!("{}", i + 1) } else { format!("{prefix}.{}", i + 1) };
            collect_attachments(child, &id, false, html, out);
        }
        return;
    }
    let part_id = if is_root { "1".to_string() } else { prefix.to_string() };
    // A part is an attachment if mail-parser lists it as one (by
    // disposition, type and position in the tree), or if it sits among the
    // body parts without being text: an image shown inline in a mail
    // without HTML (Apple Mail), which mail-parser counts as body.
    let in_body = part.in_text_body() || part.in_html_body();
    let is_text = matches!(part.kind(), PartKind::Text | PartKind::Html);
    if in_body && is_text || !in_body && !part.is_attachment() {
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
    // A part the HTML shows (`cid:`) is part of the body rather than a file
    // the user downloads. Marked inline but shown by no HTML, it stays a
    // file: otherwise nobody would see it.
    let is_inline = match (&content_id, html) {
        (Some(cid), Some(h)) => h.contains(&format!("cid:{cid}")),
        _ => false,
    };
    out.push(ParsedAttachment {
        part_id,
        filename: part.attachment_name().map(|n| n.trim().to_string()).filter(|n| !n.is_empty()),
        mime_type,
        size: part.decoded_len() as i64,
        content_id,
        is_inline,
    });
}

/// An HTML body made safe to display (`sanitize_html`).
#[derive(Debug, Clone, PartialEq)]
pub struct SanitizedHtml {
    pub html: String,
    /// Remote images left, with an `nm-remote:` source.
    pub remote_images: bool,
    /// Remote tracking pixels removed.
    pub tracking_pixels: i64,
}

/// Sanitizes an HTML body for display: removes scripts, event handlers,
/// forms, styles and anything ammonia does not allow.
///
/// No source a web view would load by itself is left: a remote tracking
/// pixel loses its source, any other remote image gets one in the app's
/// `nm-remote:` scheme, loaded only when the user asks for the images
/// (`remote_images.rs`, `ui.md`, "Images distantes").
///
/// Images embedded in the message (`cid:` URLs) are kept, and so are the
/// markers mail apps put on quoted history and signatures, so that clients
/// can fold them (`docs/thread-display.md`): Gmail's `gmail_quote`,
/// `gmail_attr` and `gmail_signature` classes, Thunderbird's
/// `moz-cite-prefix` and `moz-signature`, Apple Mail's
/// `<blockquote type="cite">`, Outlook's `divRplyFwdMsg` and `appendonsend`
/// ids. Ids are prefixed with `nm-` so a mail cannot clash with the app's
/// own ids.
pub fn sanitize_html(html: &str) -> SanitizedHtml {
    // The pixels are found first, on the original HTML: the filter below
    // sees one attribute at a time, and no longer sees `style`.
    let pixels = crate::remote_images::find_pixels(html);
    // The filter closure must be `'static`, so the flag is shared through an
    // `Arc` (a reference-counted pointer) rather than borrowed.
    //
    // Rust note: `AtomicBool` can be modified through a shared reference,
    // safely from several threads; plain `bool` could not be changed from
    // inside a `Fn` closure. The same goes for the set of pixels, moved into
    // the closure and only read there.
    let remote = Arc::new(AtomicBool::new(false));
    let flag = remote.clone();
    // Counted here, where they are removed: an image with the same source as
    // a pixel (rare) loses it too, and counts.
    let removed = Arc::new(AtomicI64::new(0));
    let removed_in_filter = removed.clone();
    let clean = ammonia::Builder::default()
        .add_url_schemes(["cid"])
        .add_allowed_classes("div", ["gmail_quote", "gmail_attr", "gmail_signature", "moz-cite-prefix", "moz-signature"])
        .add_allowed_classes("blockquote", ["gmail_quote"])
        .add_tag_attributes("blockquote", ["type"])
        .add_tag_attributes("div", ["id"])
        .id_prefix(Some("nm-"))
        .attribute_filter(move |element, attribute, value| {
            if element == "img" && attribute == "src" && crate::remote_images::is_remote(value) {
                if pixels.contains(value) {
                    removed_in_filter.fetch_add(1, Ordering::Relaxed);
                    return None;
                }
                flag.store(true, Ordering::Relaxed);
                return Some(crate::remote_images::rewrite_source(value).into());
            }
            Some(value.into())
        })
        .clean(html)
        .to_string();
    SanitizedHtml {
        html: clean,
        remote_images: remote.load(Ordering::Relaxed),
        tracking_pixels: removed.load(Ordering::Relaxed),
    }
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
    fn decodes_encoded_words_glued_inside_names_and_subject() {
        let raw = b"From: Achille Ber=?ISO-8859-1?B?6A==?=s <A@x.org>\r\n\
To: Herv=?ISO-8859-1?B?6SA=?=Mainguet <h@x.org>\r\n\
Subject: Re: 2=?ISO-8859-1?B?6A==?=me rappel\r\n\r\nbody";
        let p = parse(raw);
        assert_eq!(p.from.unwrap().name.as_deref(), Some("Achille Berès"));
        assert_eq!(p.to[0].name.as_deref(), Some("Hervé Mainguet"));
        assert_eq!(p.subject, "Re: 2ème rappel");
    }

    /// A newsletter whose text part was generated from its HTML without
    /// decoding the entities, and with a paragraph missing.
    const BROKEN_TEXT_PART: &[u8] = b"Subject: s\r\nContent-Type: multipart/alternative; boundary=\"B\"\r\n\r\n\
--B\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nUn titre.Le texte&nbsp;(suite)&nbsp;: fin\r\n\
--B\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>Un titre.</p><p>Vendredi 9 octobre</p><p>Le texte&nbsp;(suite)&nbsp;: fin</p>\r\n\
--B--\r\n";

    #[test]
    fn a_text_part_with_html_entities_gives_way_to_the_html_part() {
        let p = parse(BROKEN_TEXT_PART);
        let text = p.text.unwrap();
        assert!(!text.contains("&nbsp;"), "{text}");
        // The HTML's paragraphs, including the one the text part lacked.
        assert!(text.contains("Un titre.\n\nVendredi 9 octobre\n\nLe texte (suite) : fin"), "{text}");
        assert!(p.html.is_some());
    }

    #[test]
    fn a_text_part_with_html_markup_gives_way_to_the_html_part() {
        // The text part of a Swiss Life notice: tags and attribute remains,
        // no entity.
        let raw = b"Subject: s\r\nContent-Type: multipart/alternative; boundary=\"B\"\r\n\r\n\
--B\r\nContent-Type: text/plain; charset=utf-8\r\n\r\ndans votre <em>Espace Client</em>\r\n(https://x.fr)\" target=\"_blank\r\n\
--B\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>Cher Monsieur,</p><p>dans votre <em>Espace Client</em></p>\r\n\
--B--\r\n";
        let text = parse(raw).text.unwrap();
        assert!(text.starts_with("Cher Monsieur,\n\ndans votre Espace Client"), "{text}");
        assert!(!text.contains('<') && !text.contains("target"), "{text}");
    }

    #[test]
    fn markup_in_a_text_only_mail_is_kept() {
        // No HTML part to fall back on: tags may be meant.
        let p = parse(b"Subject: s\r\nContent-Type: text/plain\r\n\r\nUse <br /> between lines.\r\n");
        assert_eq!(p.text.as_deref().map(str::trim_end), Some("Use <br /> between lines."));
    }

    #[test]
    fn entities_encoded_twice_are_removed_from_the_text_and_the_html() {
        // The HTML of a Zoom invitation forwarded by Outlook.
        let raw = b"Subject: s\r\nContent-Type: multipart/alternative; boundary=\"B\"\r\n\r\n\
--B\r\nContent-Type: text/plain\r\n\r\nZoom :&nbsp;https://zoom.us/j/1\r\n\
--B\r\nContent-Type: text/html\r\n\r\n<p>Zoom :&amp;nbsp;<a href=\"https://zoom.us/j/1?a=1&amp;b=2\">lien</a></p>\r\n--B--\r\n";
        let p = parse(raw);
        let text = p.text.unwrap();
        assert!(text.starts_with("Zoom : lien"), "{text}");
        let html = p.html.unwrap();
        assert!(!html.contains("&amp;nbsp;") && html.contains("a=1&amp;b=2"), "{html}");
        // A text-only mail, entities encoded twice.
        let p = parse(b"Subject: s\r\nContent-Type: text/plain\r\n\r\nl&amp;#39;offre&amp;nbsp;!\r\n");
        assert_eq!(p.text.as_deref().map(str::trim_end), Some("l'offre !"));
    }

    #[test]
    fn entities_of_a_text_only_mail_are_decoded() {
        let p = parse(b"Subject: s\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nL&#39;offre &eacute;t&eacute;&nbsp;: 5&nbsp;&euro;\r\n");
        assert_eq!(p.text.as_deref().map(str::trim_end), Some("L'offre été : 5 €"));
    }

    #[test]
    fn a_plain_text_part_without_entities_is_kept_as_written() {
        // An ampersand that is not an entity, next to an HTML part.
        let raw = b"Subject: s\r\nContent-Type: multipart/alternative; boundary=\"B\"\r\n\r\n\
--B\r\nContent-Type: text/plain\r\n\r\nTom & Jerry; R&D\r\n\
--B\r\nContent-Type: text/html\r\n\r\n<p>Autre chose</p>\r\n--B--\r\n";
        assert_eq!(parse(raw).text.as_deref().map(str::trim_end), Some("Tom & Jerry; R&D"));
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
        // The remote image keeps its URL, in a scheme no web view loads.
        assert!(html.contains(r#"src="nm-remote://image?u=https%3A%2F%2Ft.example%2Fp.gif""#), "{html}");
        assert!(!html.contains("src=\"https:"));
        assert!(html.contains("cid:logo"));
        assert!(!html.contains("onclick"));
        assert!(p.has_remote_content);
        assert_eq!(p.tracking_pixels, 0);
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
        // Marked inline, but the HTML does not show it: a file to list.
        let png = p.attachments.iter().find(|a| a.mime_type == "image/png").expect("png");
        assert_eq!(png.part_id, "3");
        assert!(!png.is_inline);
        assert_eq!(png.content_id.as_deref(), Some("logo@x"));
        assert!(p.has_attachments());
        // Shown by the HTML (`cid:`): part of the body.
        let with_img = String::from_utf8_lossy(raw).replace("<p>html</p>", "<p><img src=\"cid:logo@x\"></p>");
        let shown = parse(with_img.as_bytes());
        let png = shown.attachments.iter().find(|a| a.mime_type == "image/png").expect("png");
        assert!(png.is_inline);
    }

    #[test]
    fn an_image_shown_inline_in_a_plain_text_mail_is_an_attachment() {
        // Apple Mail: a photo in a mail without HTML, marked inline.
        let raw = b"Subject: s\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"C\"\r\n\r\n\
--C\r\nContent-Type: text/plain\r\n\r\nRegarde.\r\n\
--C\r\nContent-Type: image/jpeg; name=\"IMG_1.jpg\"\r\nContent-Disposition: inline; filename=\"IMG_1.jpg\"\r\nContent-ID: <img1@x>\r\n\
Content-Transfer-Encoding: base64\r\n\r\n/9j/4AAQ\r\n--C--\r\n";
        let p = parse(raw);
        assert_eq!(p.attachments.len(), 1);
        assert_eq!(p.attachments[0].filename.as_deref(), Some("IMG_1.jpg"));
        assert_eq!(p.attachments[0].part_id, "2");
        assert!(!p.attachments[0].is_inline);
        assert!(p.has_attachments());
        assert_eq!(p.text.as_deref().map(str::trim), Some("Regarde."));
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
        let clean = sanitize_html(html).html;
        assert!(clean.contains(r#"class="gmail_quote""#), "{clean}");
        assert!(clean.contains(r#"class="gmail_attr""#));
        assert!(clean.contains(r#"type="cite""#));
        assert!(clean.contains(r#"id="nm-divRplyFwdMsg""#));
        assert!(!clean.contains("evil") && !clean.contains("onclick"));
    }

    #[test]
    fn tracking_pixels_are_removed_and_counted() {
        let html = r#"<p>Hi</p><img src="https://x.org/banner.png" width="600" alt="Banner">
            <img src="https://t.x.org/open?id=1&amp;u=2" width="1" height="1">
            <img src="//t.x.org/o.gif" style="display:none"><img src="cid:logo">"#;
        let s = sanitize_html(html);
        assert_eq!(s.tracking_pixels, 2);
        assert!(s.remote_images);
        // No source a web view would load: neither the pixels, nor the
        // banner, whose source is in the app's scheme.
        assert!(!s.html.contains("t.x.org"), "{}", s.html);
        assert!(!s.html.contains("src=\"https:") && !s.html.contains("src=\"//"), "{}", s.html);
        assert!(s.html.contains(r#"src="nm-remote://image?u=https%3A%2F%2Fx.org%2Fbanner.png""#), "{}", s.html);
        assert!(s.html.contains(r#"width="600""#) && s.html.contains("cid:logo"));
        // Only pixels: nothing left to load.
        let s = sanitize_html(r#"<p>Hi</p><img src="https://t.x.org/p.gif" width="1" height="1">"#);
        assert_eq!((s.remote_images, s.tracking_pixels), (false, 1));
        // Nothing remote.
        let s = sanitize_html("<p>Hi</p>");
        assert_eq!((s.remote_images, s.tracking_pixels), (false, 0));
    }

    #[test]
    fn a_parsed_message_counts_its_pixels() {
        let raw = b"Content-Type: text/html\r\nSubject: s\r\n\r\n<p>x</p><img src=\"https://t.x.org/p\" height=\"1\"><img src=\"https://x.org/a.png\">";
        let p = parse(raw);
        assert_eq!(p.tracking_pixels, 1);
        assert!(p.has_remote_content);
    }
}
