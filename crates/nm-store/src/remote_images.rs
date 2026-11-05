//! Remote images of an HTML body: tracking pixels and the URLs a client may
//! load (`ui.md`, "Images distantes").
//!
//! The sanitization (`ingest::sanitize_html`) never leaves a URL a web view
//! would load by itself. For each `<img>` whose source is remote
//! (`http:`, `https:`, or `//host/…`):
//! - a **tracking pixel** loses its source: it is never requested, in any
//!   display mode, and only counted (`message_body.tracking_pixels`);
//! - any other image gets a source in the app's own scheme,
//!   `nm-remote://image?u=<percent-encoded URL>`. A web view does not know
//!   this scheme, so nothing is loaded unless the client handles it, and the
//!   client only does so when the user asked for the images.
//!
//! A pixel must be recognised *before* any request: once the request is
//! sent, the sender knows the mail was opened. The rules below only read
//! the tag itself, never the network. On a real mailbox of 46,539 mails
//! (29,244 with remote images, 224,097 images), a script applying close
//! rules found 26,868 pixels in 16,956 mails.
//!
//! Limits, said plainly in `docs/geek-ui.md`: a pixel hidden by a parent
//! (`<div style="display:none">`) or declared with no size at all is not
//! recognised, and a mail whose every image carries a token proper to the
//! recipient reveals the opening whatever is done here.

use std::cell::RefCell;
use std::collections::HashSet;

use html5ever::tendril::StrTendril;
use html5ever::tokenizer::{BufferQueue, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts};

/// Prefix of the source given to a remote image that a client may load.
/// The original URL follows, percent-encoded (`decode_source` reverses it).
pub const SOURCE_PREFIX: &str = "nm-remote://image?u=";

/// Whether an image source is remote: loaded from the network.
/// Protocol-relative sources (`//host/x`) count: a web view resolves them
/// against `http:` or `https:`.
pub fn is_remote(src: &str) -> bool {
    let v = src.trim_start().to_ascii_lowercase();
    v.starts_with("http:") || v.starts_with("https:") || v.starts_with("//")
}

/// The source a client may load for the remote image `src`: the URL made
/// absolute (`//host` becomes `https://host`), percent-encoded after
/// `SOURCE_PREFIX`. `http:` is kept as written: the client decides whether
/// to upgrade it (the app only ever sends HTTPS requests).
pub fn rewrite_source(src: &str) -> String {
    let url = src.trim();
    let url = if url.starts_with("//") { format!("https:{url}") } else { url.to_string() };
    let mut out = String::with_capacity(SOURCE_PREFIX.len() + url.len() * 3 / 2);
    out.push_str(SOURCE_PREFIX);
    for b in url.bytes() {
        // RFC 3986 unreserved characters stay as they are; every other byte,
        // `&`, `=`, `%`, `#` and non-ASCII included, becomes `%XX`, so the
        // URL survives as one query value.
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The original URL of a source written by `rewrite_source`; `None` for any
/// other source or a malformed encoding. Used by tests here; the app has
/// its own decoder (`RemoteImages.swift`), tested on the same examples.
pub fn decode_source(src: &str) -> Option<String> {
    let encoded = src.strip_prefix(SOURCE_PREFIX)?;
    let bytes = encoded.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = encoded.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The remote tracking pixels of a whole HTML document: the set of their
/// `src` values, exactly as the HTML5 tokenizer reads them (entities
/// decoded).
///
/// It is a pre-pass over the original HTML because the sanitizer cannot
/// decide alone: ammonia's attribute filter sees one attribute at a time
/// (the `src` without the `width` next to it), and `style` is already
/// removed when the filter runs. The tokenizer is html5ever's, the one
/// ammonia parses with, so a value read here is the value the filter gets.
pub fn find_pixels(html: &str) -> HashSet<String> {
    // Most bodies have no image at all: no tokenization for them. The test
    // is on bytes, ASCII case ignored (`<IMG`), with no allocation.
    if !html.as_bytes().windows(4).any(|w| w.eq_ignore_ascii_case(b"<img")) {
        return HashSet::new();
    }
    /// Receives the tokens; only `<img>` start tags matter.
    ///
    /// Rust note: html5ever's `TokenSink::process_token` takes `&self`,
    /// not `&mut self`, so the sink's state lives in a `RefCell`, which
    /// checks borrowing rules at run time instead of compile time.
    #[derive(Default)]
    struct Sink {
        found: RefCell<HashSet<String>>,
    }
    impl TokenSink for Sink {
        type Handle = ();
        fn process_token(&self, token: Token, _line: u64) -> TokenSinkResult<()> {
            if let Token::TagToken(tag) = token
                && tag.kind == TagKind::StartTag
                && &*tag.name == "img"
            {
                let get = |name: &str| tag.attrs.iter().find(|a| &*a.name.local == name).map(|a| &*a.value);
                if let Some(src) = get("src")
                    && is_remote(src)
                    && is_pixel(get("width"), get("height"), get("style"))
                {
                    self.found.borrow_mut().insert(src.to_string());
                }
            }
            TokenSinkResult::Continue
        }
    }

    let input = BufferQueue::default();
    input.push_back(StrTendril::from_slice(html));
    let tokenizer = Tokenizer::new(Sink::default(), TokenizerOpts::default());
    let _ = tokenizer.feed(&input);
    tokenizer.end();
    tokenizer.sink.found.into_inner()
}

/// Whether an `<img>` is a tracking pixel, from its attributes: a width or
/// height of 2 pixels or less (attribute or style), or a style that hides
/// it (`display: none`, `visibility: hidden`, `opacity: 0`).
///
/// 2 rather than 1: senders also use 2×2 and 0×0 images, and an image that
/// small has nothing to show. A size in another unit (`width="1%"`,
/// `1em`) is not read as a pixel: it may be a real image.
pub fn is_pixel(width: Option<&str>, height: Option<&str>, style: Option<&str>) -> bool {
    if width.is_some_and(tiny) || height.is_some_and(tiny) {
        return true;
    }
    let Some(style) = style else { return false };
    style.split(';').any(|declaration| {
        let Some((name, value)) = declaration.split_once(':') else { return false };
        let name = name.trim().to_ascii_lowercase();
        let value = value.to_ascii_lowercase().replace("!important", "");
        let value = value.trim();
        match name.as_str() {
            "display" => value == "none",
            "visibility" => value == "hidden",
            "opacity" => value.trim_end_matches('%').parse::<f64>().is_ok_and(|o| o == 0.0),
            "width" | "height" | "max-width" | "max-height" => tiny(value),
            _ => false,
        }
    })
}

/// A size of 2 pixels or less: `0`, `1`, `2`, optionally followed by `px`
/// (`"1"`, `"1px"`, `" 0 "`).
fn tiny(value: &str) -> bool {
    let v = value.trim().to_ascii_lowercase();
    let digits = v.trim_end_matches("px").trim_end();
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) && digits.parse::<u32>().is_ok_and(|n| n <= 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_sources() {
        assert!(is_remote("https://x.org/a.png"));
        assert!(is_remote("  HTTP://x.org/a.png"));
        assert!(is_remote("//cdn.x.org/a.png"));
        assert!(!is_remote("cid:logo@x"));
        assert!(!is_remote("data:image/png;base64,AAAA"));
        assert!(!is_remote("logo.png"));
    }

    #[test]
    fn sizes_of_a_pixel() {
        for v in ["0", "1", "2", "1px", " 1 ", "2PX"] {
            assert!(tiny(v), "{v}");
        }
        for v in ["3", "600", "1%", "1em", "", "px", "1.5", "-1"] {
            assert!(!tiny(v), "{v}");
        }
    }

    #[test]
    fn pixels_by_attributes_and_style() {
        assert!(is_pixel(Some("1"), Some("1"), None));
        assert!(is_pixel(Some("1"), None, None));
        assert!(is_pixel(None, Some("0"), None));
        assert!(is_pixel(None, None, Some("display:none")));
        assert!(is_pixel(None, None, Some("border:0; DISPLAY : none !important")));
        assert!(is_pixel(None, None, Some("visibility:hidden")));
        assert!(is_pixel(None, None, Some("opacity:0")));
        assert!(is_pixel(None, None, Some("opacity: 0.0")));
        assert!(is_pixel(None, None, Some("width:1px;height:1px")));
        assert!(is_pixel(None, None, Some("max-height:0")));
        // Real images.
        assert!(!is_pixel(Some("600"), Some("200"), None));
        assert!(!is_pixel(None, None, None));
        assert!(!is_pixel(Some("100%"), None, Some("display:block; width:600px; opacity:0.9")));
    }

    #[test]
    fn rewritten_sources_round_trip() {
        for url in [
            "https://x.org/a.png",
            "https://x.org/o?id=1&u=me%40x.org#f",
            "http://x.org/caf\u{e9} \"q\".png",
        ] {
            let src = rewrite_source(url);
            assert!(src.starts_with(SOURCE_PREFIX));
            assert!(!src[SOURCE_PREFIX.len()..].contains(['&', '=', '"', '#', '/', ' ']), "{src}");
            assert_eq!(decode_source(&src).as_deref(), Some(url));
        }
        // Protocol-relative: made HTTPS.
        assert_eq!(decode_source(&rewrite_source(" //cdn.x.org/a.png")).as_deref(), Some("https://cdn.x.org/a.png"));
        assert_eq!(rewrite_source("https://x.org/a b"), "nm-remote://image?u=https%3A%2F%2Fx.org%2Fa%20b");
        assert_eq!(decode_source("https://x.org"), None);
        assert_eq!(decode_source("nm-remote://image?u=%4"), None);
        assert_eq!(decode_source("nm-remote://image?u=%ZZ"), None);
    }

    #[test]
    fn pixels_of_a_document() {
        let html = r#"<p><img src="https://x.org/banner.png" width="600">
            <img src="https://t.x.org/open?id=1&amp;u=2" width="1" height="1">
            <IMG SRC="//t.x.org/o.gif" style="display: none">
            <img src="https://t.x.org/open?id=1&amp;u=2" height="1">
            <img src="cid:logo" width="1"></p>"#;
        let set = find_pixels(html);
        // Entities decoded, as ammonia's filter will see the value.
        assert!(set.contains("https://t.x.org/open?id=1&u=2"));
        assert!(set.contains("//t.x.org/o.gif"));
        // Neither the banner nor the `cid:` image.
        assert_eq!(set.len(), 2);
        assert!(find_pixels("<p>no image</p>").is_empty());
        assert!(find_pixels("<img src=").is_empty());
    }
}
