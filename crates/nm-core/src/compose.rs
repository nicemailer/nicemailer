//! Writing a reply: the pure rules of `specifications/reponse.md`.
//!
//! Everything here is a function of its arguments, with no database, no
//! clock and no network, so that every rule of the specification is a unit
//! test away:
//!
//! - who the reply goes to ([`reply_all`]) and from which address
//!   ([`sender_identity`]), section 4, "Destinataires" and "Adresse
//!   d'expédition";
//! - the addresses typed or pasted in a recipient line
//!   ([`parse_address_list`]), the recipients and the subject as edited
//!   ([`clean_recipients`], [`clean_subject`]), section 6;
//! - its subject ([`reply_subject`]) and its conversation headers
//!   ([`references`]), section 4, "Objet" and "En-têtes de conversation";
//! - the quotation of the mail it answers ([`quote`], [`initial_body`]),
//!   section 4, "Corps et citation";
//! - the MIME message sent ([`build_message`]): `text/plain` in
//!   `format=flowed` (RFC 3676), headers encoded (RFC 2047), section 7,
//!   "Format du mail";
//! - whether the provider copies sent mail to Sent itself
//!   ([`provider_copies_sent`]), section 9.
//!
//! The store gathers the inputs (the mail answered, the account's
//! identities), the engine sends the bytes; neither repeats a rule.

use std::fmt::Write as _;

/// An address with its optional display name.
///
/// Rust note: `#[derive(Default)]` gives `Mailbox::default()`, an empty
/// address; the tests use it with `..` to fill in only some fields.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Mailbox {
    pub addr: String,
    pub name: Option<String>,
}

impl Mailbox {
    pub fn new(addr: &str, name: Option<&str>) -> Mailbox {
        Mailbox { addr: addr.to_string(), name: name.map(str::to_string) }
    }
}

/// Delay before a mail queued by `send_draft` leaves, during which `z`
/// cancels it (`reponse.md`, section 7): the default of
/// `compose.undo_send_seconds`, and its upper bound.
pub const UNDO_SEND_DEFAULT_SECS: u64 = 5;
pub const UNDO_SEND_MAX_SECS: u64 = 30;
/// Lines of the quoted mail kept in the reply: beyond, the quotation ends
/// with `> […]` (`reponse.md`, section 4).
pub const QUOTE_MAX_LINES: usize = 1000;
/// Identifiers kept in `References`, the first (the root) always among them.
pub const REFERENCES_MAX: usize = 20;

/// Whether `addr` is a plain address that can go in an SMTP envelope and a
/// header without escaping: one `@`, something on both sides, no space, no
/// control character, none of the characters that delimit addresses. An
/// address that fails is left out of the reply rather than sent broken; it
/// is also what keeps a hostile header from injecting a command into the
/// SMTP conversation (`RCPT TO:<…>` cannot be closed early).
pub fn valid_addr(addr: &str) -> bool {
    let Some((local, domain)) = addr.rsplit_once('@') else { return false };
    !local.is_empty()
        && !domain.is_empty()
        && addr.len() <= 254
        && (domain.contains('.') || domain.eq_ignore_ascii_case("localhost"))
        && !addr.chars().any(|c| c.is_whitespace() || c.is_control() || "<>()[],;:\\\"".contains(c))
        && !domain.contains('@')
}

/// Most recipients a draft may have, To, Cc and Bcc together: far more
/// than a person writes to, few enough that a paste of a whole text by
/// mistake does not make a mail to thousands.
pub const RECIPIENTS_MAX: usize = 500;

/// The addresses typed or pasted in a recipient line (`reponse.md`,
/// section 6), in order, each once (without case): `(addresses, invalid)`,
/// where `invalid` keeps the pieces that are not an address, as typed, for
/// the user to correct.
///
/// Understood: `alice@x.fr`, `Alice <alice@x.fr>`, `"Dupont, J." <j@x.fr>`,
/// `Alice Martin alice@x.fr` (a name then the address, as some lists are
/// pasted), `<alice@x.fr>`, `mailto:alice@x.fr`, separated by commas,
/// semicolons or line breaks. A comma inside quotes or angle brackets does
/// not separate.
pub fn parse_address_list(text: &str) -> (Vec<Mailbox>, Vec<String>) {
    let mut out: Vec<Mailbox> = Vec::new();
    let mut invalid = Vec::new();
    for piece in split_addresses(text) {
        let piece = piece.trim();
        if piece.is_empty() {
            continue;
        }
        match parse_mailbox(piece) {
            Some(m) => {
                if !out.iter().any(|o| o.addr.eq_ignore_ascii_case(&m.addr)) {
                    out.push(m);
                }
            }
            None => invalid.push(piece.to_string()),
        }
    }
    (out, invalid)
}

/// Splits a list of addresses on `,`, `;` and line breaks outside quotes
/// and angle brackets.
fn split_addresses(text: &str) -> Vec<&str> {
    let mut pieces = Vec::new();
    let (mut start, mut quoted, mut angle, mut escaped) = (0, false, false, false);
    // Rust note: `char_indices` gives each character with its byte offset,
    // so the pieces are slices of `text`, never copies.
    for (i, c) in text.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            '<' if !quoted => angle = true,
            '>' if !quoted => angle = false,
            ',' | ';' | '\n' | '\r' if !quoted && !angle => {
                pieces.push(&text[start..i]);
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    pieces.push(&text[start..]);
    pieces
}

/// One address with its optional name, or `None` if it is not one.
fn parse_mailbox(piece: &str) -> Option<Mailbox> {
    let bare = |s: &str| -> Option<String> {
        let s = s.trim();
        let s = s.strip_prefix("mailto:").or_else(|| s.strip_prefix("MAILTO:")).unwrap_or(s);
        valid_addr(s).then(|| s.to_lowercase())
    };
    let unquote = |name: &str| -> Option<String> {
        let name = name.trim();
        let name = match name.strip_prefix('"').and_then(|n| n.strip_suffix('"')) {
            Some(inner) => inner.replace("\\\"", "\"").replace("\\\\", "\\"),
            None => name.to_string(),
        };
        let name = name.trim().to_string();
        (!name.is_empty()).then_some(name)
    };
    if let (Some(open), true) = (piece.rfind('<'), piece.ends_with('>')) {
        let addr = bare(&piece[open + 1..piece.len() - 1])?;
        return Some(Mailbox { addr, name: unquote(&piece[..open]) });
    }
    if let Some(addr) = bare(piece) {
        return Some(Mailbox { addr, name: None });
    }
    // "Alice Martin alice@x.fr": a name, then the address.
    let (name, last) = piece.rsplit_once(char::is_whitespace)?;
    let addr = bare(last)?;
    Some(Mailbox { addr, name: unquote(name) })
}

/// The recipients of a draft as the user edited them: each address once
/// across the three lists (To first, then Cc, then Bcc, without case), the
/// names trimmed. `Err` names the first address that is not one: the
/// client sends only what [`parse_address_list`] accepted, so this guards
/// the database, not the user.
pub fn clean_recipients(lists: [&[Mailbox]; 3]) -> Result<[Vec<Mailbox>; 3], String> {
    let mut seen: Vec<String> = Vec::new();
    let mut out: [Vec<Mailbox>; 3] = Default::default();
    for (i, list) in lists.into_iter().enumerate() {
        for m in list {
            let addr = m.addr.trim().to_lowercase();
            if !valid_addr(&addr) {
                return Err(m.addr.clone());
            }
            if seen.contains(&addr) {
                continue;
            }
            seen.push(addr.clone());
            let name = m.name.as_deref().map(str::trim).filter(|n| !n.is_empty()).map(clean);
            out[i].push(Mailbox { addr, name });
        }
    }
    if seen.len() > RECIPIENTS_MAX {
        return Err(format!("more than {RECIPIENTS_MAX} recipients"));
    }
    Ok(out)
}

/// A subject as typed: one line, without control characters, at most
/// 998 characters (the longest line of a mail).
pub fn clean_subject(subject: &str) -> String {
    clean(subject).chars().filter(|c| !c.is_control()).take(998).collect::<String>().trim().to_string()
}

/// What the reply rules read of the mail answered.
///
/// Rust note: the fields are *borrowed* slices (`&'a [Mailbox]`): the
/// struct only points at lists owned by the caller, nothing is copied. The
/// lifetime `'a` says the struct cannot outlive those lists.
#[derive(Debug, Clone, Copy)]
pub struct Original<'a> {
    pub from: Option<&'a Mailbox>,
    pub reply_to: &'a [Mailbox],
    pub to: &'a [Mailbox],
    pub cc: &'a [Mailbox],
}

/// The recipients of a reply to all, `(to, cc)` (`reponse.md`, section 4,
/// "Destinataires"). `own`: the addresses of the account (main address and
/// identities), lowercase.
///
/// 1. To: `Reply-To` when present, else the sender;
/// 2. Cc: the recipients of the mail (To then Cc);
/// 3. a reply to one's own mail takes the mail's recipients back: To its
///    To, Cc its Cc (a reminder, not a reply to oneself);
/// 4. the account's addresses, duplicates (without case) and invalid
///    addresses are removed; an address of To is not repeated in Cc;
/// 5. To empty after that: the first of Cc moves to To; nobody left: the
///    sender, even if it is an address of the account.
pub fn reply_all(original: &Original, own: &[String]) -> (Vec<Mailbox>, Vec<Mailbox>) {
    let is_own = |m: &Mailbox| own.iter().any(|o| o.eq_ignore_ascii_case(&m.addr));
    let from_own = original.from.is_some_and(is_own);
    let (to, cc): (Vec<&Mailbox>, Vec<&Mailbox>) = if from_own {
        (original.to.iter().collect(), original.cc.iter().collect())
    } else {
        let to = if original.reply_to.is_empty() { original.from.into_iter().collect() } else { original.reply_to.iter().collect() };
        (to, original.to.iter().chain(original.cc).collect())
    };
    let mut seen: Vec<String> = Vec::new();
    // Rust note: a closure that changes a variable it captures (`seen`)
    // is `FnMut`, hence `let mut keep`.
    let mut keep = |list: Vec<&Mailbox>| -> Vec<Mailbox> {
        let mut out = Vec::new();
        for m in list {
            let addr = m.addr.trim().to_lowercase();
            if !valid_addr(&addr) || is_own(m) || seen.contains(&addr) {
                continue;
            }
            seen.push(addr.clone());
            out.push(Mailbox { addr, name: m.name.clone().filter(|n| !n.trim().is_empty()) });
        }
        out
    };
    let mut to = keep(to);
    let mut cc = keep(cc);
    if to.is_empty() && !cc.is_empty() {
        to.push(cc.remove(0));
    }
    if to.is_empty()
        && let Some(f) = original.from.filter(|f| valid_addr(f.addr.trim()))
    {
        to.push(Mailbox { addr: f.addr.trim().to_lowercase(), name: f.name.clone() });
    }
    (to, cc)
}

/// The address the reply is sent from (`reponse.md`, section 4, "Adresse
/// d'expédition"): the first identity of the account (main address first)
/// among the recipients of the mail answered, else the main address.
/// `identities` is never empty: it starts with the account's address.
pub fn sender_identity(identities: &[Mailbox], original: &Original) -> Mailbox {
    let addressed = |i: &&Mailbox| original.to.iter().chain(original.cc).any(|r| r.addr.trim().eq_ignore_ascii_case(&i.addr));
    identities.iter().find(addressed).or(identities.first()).cloned().unwrap_or_default()
}

/// The subject of the reply: `Re: ` and the subject, unless it already
/// starts with a reply prefix (`reponse.md`, section 4, "Objet").
pub fn reply_subject(subject: &str) -> String {
    let s = subject.trim();
    if has_reply_prefix(s) { s.to_string() } else { format!("Re: {s}") }
}

/// Whether `s` starts with a reply prefix in one of the usual languages:
/// a word (`Re`, `AW`, `Sv`, `Réf.`…), an optional counter (`[2]`, `(2)`),
/// spaces, then `:`.
fn has_reply_prefix(s: &str) -> bool {
    // Reply prefixes of the clients met in practice: English and Latin
    // `Re`, German `AW`/`Antw`, Nordic `Sv`/`Vs`, French `Réf`, Polish
    // `Odp`, Turkish `Ynt`.
    const WORDS: &[&str] = &["re", "aw", "antw", "sv", "vs", "réf", "ref", "odp", "ynt"];
    let word: String = s.chars().take_while(|c| c.is_alphabetic()).collect();
    if !WORDS.contains(&word.to_lowercase().as_str()) {
        return false;
    }
    let mut rest = &s[word.len()..];
    rest = rest.strip_prefix('.').unwrap_or(rest);
    // An optional counter: `Re[2]:`, `Re(2):`.
    for (open, close) in [('[', ']'), ('(', ')')] {
        if let Some(r) = rest.strip_prefix(open)
            && let Some(end) = r.find(close)
            && r[..end].chars().all(|c| c.is_ascii_digit())
        {
            rest = &r[end + 1..];
        }
    }
    rest.trim_start().starts_with(':')
}

/// The `References` of the reply (`reponse.md`, section 4): those of the
/// mail answered, or its `In-Reply-To` when it has none, then its own
/// `Message-ID`. Without duplicates, and at most [`REFERENCES_MAX`]: the
/// first (the root of the conversation) and the latest ones.
pub fn references(parent_refs: &[String], parent_in_reply_to: Option<&str>, parent_id: Option<&str>) -> Vec<String> {
    let base: Vec<&str> = if parent_refs.is_empty() {
        parent_in_reply_to.into_iter().collect()
    } else {
        parent_refs.iter().map(String::as_str).collect()
    };
    let mut out: Vec<String> = Vec::new();
    for id in base.into_iter().chain(parent_id) {
        let id = id.trim().trim_start_matches('<').trim_end_matches('>');
        if !id.is_empty() && valid_msgid(id) && !out.iter().any(|o| o == id) {
            out.push(id.to_string());
        }
    }
    if out.len() > REFERENCES_MAX {
        let tail = out.split_off(out.len() - (REFERENCES_MAX - 1));
        out.truncate(1);
        out.extend(tail);
    }
    out
}

/// A `Message-ID` that can be written between `<` and `>` as is: no space,
/// no control character, no angle bracket.
pub fn valid_msgid(id: &str) -> bool {
    !id.is_empty() && id.len() <= 250 && !id.chars().any(|c| c.is_whitespace() || c.is_control() || c == '<' || c == '>' || !c.is_ascii())
}

/// The text of the mail answered, quoted (`reponse.md`, section 4):
/// `> ` before each line, `>` alone before a line already quoted (`>> `)
/// and for an empty line; spaces at the end of lines removed; beyond
/// `max_lines`, the rest is replaced by `> […]`.
pub fn quote(text: &str, max_lines: usize) -> String {
    let text = text.replace("\r\n", "\n");
    let lines: Vec<&str> = text.trim_end().lines().collect();
    let mut out = String::with_capacity(text.len() + lines.len() * 2);
    for (i, line) in lines.iter().enumerate() {
        if i == max_lines {
            out.push_str("> […]\n");
            break;
        }
        let line = line.trim_end();
        if line.is_empty() {
            out.push('>');
        } else if line.starts_with('>') {
            out.push('>');
            out.push_str(line);
        } else {
            out.push_str("> ");
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

/// The body a reply starts with: an empty line where the cursor goes, an
/// empty line, the line introducing the quotation (written by the client,
/// in its language), then the quotation. The reply is written above it.
pub fn initial_body(quote_header: &str, quoted: &str) -> String {
    let header = quote_header.replace(['\r', '\n'], " ");
    format!("\n\n{}\n{}", header.trim(), quoted)
}

// ---------------------------------------------------------------------------
// format=flowed (RFC 3676)
// ---------------------------------------------------------------------------

/// Width the text is cut at, quotation marks included. RFC 3676 asks for
/// lines of at most 78 characters, and recommends 66 to 72.
pub const FLOWED_WIDTH: usize = 72;

/// The text in `format=flowed`, lines ended by CRLF (`reponse.md`,
/// section 7).
///
/// A line longer than [`FLOWED_WIDTH`] is cut after a space, and the space
/// stays at the end of the piece: a reader that knows `format=flowed` joins
/// the pieces back, and the text reads as typed; one that does not still
/// shows readable lines. Each piece of a quoted line repeats its `>`. The
/// text typed is never changed:
///
/// - spaces at the end of a line are removed: they would mean "this line
///   continues", and join it with the next;
/// - a line starting with a space or with `From ` gets one space more
///   ("space-stuffing"), which a reader removes: without it, the space
///   would be lost, and `From ` changed to `>From ` by some servers;
/// - the signature separator `-- ` keeps its space, as the RFC says.
pub fn flowed(text: &str) -> String {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = String::with_capacity(text.len() + text.len() / 32 + 2);
    for raw in text.split('\n') {
        if raw == "-- " {
            out.push_str("-- \r\n");
            continue;
        }
        let line = raw.trim_end_matches(' ');
        let depth = line.chars().take_while(|c| *c == '>').count();
        // Rust note: slicing a `str` is by bytes; `>` is one byte, so
        // `depth` characters are `depth` bytes here.
        let rest = &line[depth..];
        let (prefix, content): (String, &str) = if depth > 0 {
            let content = rest.strip_prefix(' ').unwrap_or(rest);
            // One space after the marks: it is "stuffing", removed by the
            // reader, and makes the quotation readable everywhere.
            (format!("{}{}", ">".repeat(depth), if content.is_empty() { "" } else { " " }), content)
        } else {
            (String::new(), rest)
        };
        let room = FLOWED_WIDTH.saturating_sub(prefix.chars().count()).max(20);
        for piece in cut(content, room) {
            out.push_str(&prefix);
            // A piece starting with `>` would read as a quotation.
            if depth == 0 && (piece.starts_with(' ') || piece.starts_with("From ") || piece.starts_with('>')) {
                out.push(' ');
            }
            out.push_str(piece);
            out.push_str("\r\n");
        }
    }
    // `split('\n')` gives a last empty piece after a final newline: one
    // CRLF too many.
    if text.ends_with('\n') && out.ends_with("\r\n\r\n") {
        out.truncate(out.len() - 2);
    }
    out
}

/// Cuts `s` into pieces of at most `room` characters where it can, after
/// a run of spaces, which stays at the end of the piece. A word longer than
/// `room` is not cut: the line is then longer, which is allowed.
fn cut(s: &str, room: usize) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut rest = s;
    while rest.chars().count() > room {
        // Byte index of the end of each run of spaces, with its position
        // in characters.
        let mut best: Option<usize> = None;
        let mut first_after: Option<usize> = None;
        let mut chars = rest.char_indices().peekable();
        let mut count = 0;
        while let Some((i, c)) = chars.next() {
            count += 1;
            if c == ' ' && chars.peek().is_some_and(|(_, n)| *n != ' ') {
                let end = i + 1;
                // A piece that is nothing but spaces would be a line
                // starting with a space: not a place to cut.
                if rest[..end].trim().is_empty() {
                    continue;
                }
                if count <= room {
                    best = Some(end);
                } else {
                    first_after = Some(end);
                    break;
                }
            }
        }
        let Some(end) = best.or(first_after) else { break };
        pieces.push(&rest[..end]);
        rest = &rest[end..];
    }
    pieces.push(rest);
    pieces
}

// ---------------------------------------------------------------------------
// The MIME message
// ---------------------------------------------------------------------------

/// The headers of a mail to send.
#[derive(Debug, Clone)]
pub struct Outgoing<'a> {
    pub from: &'a Mailbox,
    pub to: &'a [Mailbox],
    pub cc: &'a [Mailbox],
    /// Written as a `Bcc` header only in the copies kept by the user (Sent,
    /// Drafts, the raw store), for the record; the mail given to the SMTP
    /// server is built with none, so that no recipient learns of them
    /// (`reponse.md`, section 7).
    pub bcc: &'a [Mailbox],
    pub subject: &'a str,
    /// Unix seconds, and the offset of the Mac's time zone, in seconds
    /// east of UTC: the `Date` is written in local time.
    pub date: i64,
    pub tz_offset_secs: i32,
    /// Without angle brackets.
    pub message_id: &'a str,
    pub in_reply_to: Option<&'a str>,
    pub references: &'a [String],
    pub user_agent: &'a str,
    /// The revision of a draft copied to the Drafts folder of the server
    /// (`brouillons.md`, section 5): written in `X-NiceMailer-Draft-Revision`,
    /// never in the mail sent.
    pub draft_revision: Option<i64>,
}

/// The MIME message of a text mail (`reponse.md`, section 7): RFC 5322
/// headers, a `text/plain; charset=utf-8; format=flowed` body in `7bit`
/// when it is ASCII with short lines, in `quoted-printable` otherwise.
/// Lines end with CRLF. `Bcc` is written when `h.bcc` is not empty: the
/// caller gives an empty list for the mail sent.
pub fn build_message(h: &Outgoing, body: &str) -> Vec<u8> {
    let mut out = String::with_capacity(body.len() + 1024);
    header(&mut out, "From", &mailbox(h.from));
    if !h.to.is_empty() {
        address_list(&mut out, "To", h.to);
    }
    if !h.cc.is_empty() {
        address_list(&mut out, "Cc", h.cc);
    }
    if !h.bcc.is_empty() {
        address_list(&mut out, "Bcc", h.bcc);
    }
    header(&mut out, "Subject", &unstructured(h.subject));
    header(&mut out, "Date", &rfc5322_date(h.date, h.tz_offset_secs));
    header(&mut out, "Message-ID", &format!("<{}>", h.message_id));
    if let Some(parent) = h.in_reply_to.filter(|p| valid_msgid(p)) {
        header(&mut out, "In-Reply-To", &format!("<{parent}>"));
    }
    let refs: Vec<String> = h.references.iter().filter(|r| valid_msgid(r)).map(|r| format!("<{r}>")).collect();
    if !refs.is_empty() {
        // One identifier per line: they are long, and a folded header
        // keeps every line short.
        header(&mut out, "References", &refs.join("\r\n "));
    }
    header(&mut out, "MIME-Version", "1.0");
    header(&mut out, "User-Agent", &clean(h.user_agent));
    if let Some(rev) = h.draft_revision {
        header(&mut out, "X-NiceMailer-Draft-Revision", &rev.to_string());
    }
    let text = flowed(body);
    let plain = text.is_ascii() && text.split("\r\n").all(|l| l.len() <= 998);
    header(&mut out, "Content-Type", "text/plain; charset=utf-8; format=flowed");
    header(&mut out, "Content-Transfer-Encoding", if plain { "7bit" } else { "quoted-printable" });
    out.push_str("\r\n");
    if plain {
        out.push_str(&text);
    } else {
        quoted_printable(&mut out, &text);
    }
    if !out.ends_with("\r\n") {
        out.push_str("\r\n");
    }
    out.into_bytes()
}

/// A header line. The value is already encoded and folded.
fn header(out: &mut String, name: &str, value: &str) {
    out.push_str(name);
    out.push_str(": ");
    out.push_str(value);
    out.push_str("\r\n");
}

/// A value from outside without line breaks: a CR or LF in a subject or a
/// name would end the header and start another (header injection).
fn clean(s: &str) -> String {
    s.chars().map(|c| if c == '\r' || c == '\n' || c == '\t' { ' ' } else { c }).collect()
}

/// An unstructured value (the subject): as is when it is ASCII, else in
/// RFC 2047 encoded words; folded at spaces to keep lines short.
fn unstructured(s: &str) -> String {
    let s = clean(s);
    if s.is_ascii() {
        fold_words(&s, "Subject: ".len())
    } else {
        encoded_words(&s)
    }
}

/// Folds an ASCII text at spaces so that lines stay under 78 characters
/// where possible; `first` is the length of the header name before it.
fn fold_words(s: &str, first: usize) -> String {
    let mut out = String::new();
    let mut line = first;
    for (i, word) in s.split(' ').enumerate() {
        if i > 0 {
            if line + 1 + word.len() > 78 && line > 0 {
                out.push_str("\r\n ");
                line = 1;
            } else {
                out.push(' ');
                line += 1;
            }
        }
        out.push_str(word);
        line += word.len();
    }
    out
}

/// RFC 2047 encoded words, `=?UTF-8?B?…?=`, of at most 75 characters each,
/// cut between characters, separated by a folding space.
fn encoded_words(s: &str) -> String {
    let mut words = Vec::new();
    let mut chunk = String::new();
    for c in s.chars() {
        // 36 bytes give 48 characters of base64: with `=?UTF-8?B?` and
        // `?=`, 60, under the limit of 75 for a word, and a first line
        // under 78 even after `Subject: `.
        if chunk.len() + c.len_utf8() > 36 {
            words.push(format!("=?UTF-8?B?{}?=", base64(chunk.as_bytes())));
            chunk.clear();
        }
        chunk.push(c);
    }
    if !chunk.is_empty() || words.is_empty() {
        words.push(format!("=?UTF-8?B?{}?=", base64(chunk.as_bytes())));
    }
    words.join("\r\n ")
}

/// A display name and its address: `Name <addr>`, the name quoted when it
/// has special characters, encoded when it is not ASCII.
fn mailbox(m: &Mailbox) -> String {
    let addr = clean(&m.addr);
    match m.name.as_deref().map(clean).map(|n| n.trim().to_string()).filter(|n| !n.is_empty()) {
        None => addr,
        Some(n) if !n.is_ascii() => format!("{} <{addr}>", encoded_words(&n)),
        Some(n) if n.chars().all(|c| c.is_ascii_alphanumeric() || " !#$%&'*+-/=?^_`{|}~".contains(c)) => format!("{n} <{addr}>"),
        Some(n) => format!("\"{}\" <{addr}>", n.replace('\\', "\\\\").replace('"', "\\\"")),
    }
}

/// `To` or `Cc`: the addresses separated by commas, a new line before an
/// address that would make the line longer than 78 characters.
fn address_list(out: &mut String, name: &str, list: &[Mailbox]) {
    let mut value = String::new();
    let mut line = name.len() + 2;
    for (i, m) in list.iter().enumerate() {
        let item = mailbox(m);
        if i > 0 {
            value.push(',');
            line += 1;
            if line + 1 + item.len() > 78 {
                value.push_str("\r\n ");
                line = 1;
            } else {
                value.push(' ');
                line += 1;
            }
        }
        // An encoded name has its own folds: the line length restarts
        // after its last one.
        line = match item.rfind("\r\n") {
            Some(p) => item.len() - p - 2,
            None => line + item.len(),
        };
        value.push_str(&item);
    }
    header(out, name, &value);
}

/// Quoted-printable (RFC 2045, 6.7) of text whose lines end with CRLF:
/// line ends stay, spaces before them are encoded (`=20`, they matter in
/// `format=flowed`), lines longer than 76 characters get soft breaks.
fn quoted_printable(out: &mut String, text: &str) {
    let lines: Vec<&str> = text.split("\r\n").collect();
    let last = lines.len().saturating_sub(1);
    for (n, line) in lines.iter().enumerate() {
        if n == last && line.is_empty() {
            break;
        }
        let bytes = line.as_bytes();
        let mut len = 0;
        for (i, &b) in bytes.iter().enumerate() {
            let at_end = i + 1 == bytes.len();
            let literal = (b == b' ' || b == b'\t') && !at_end || (33..=126).contains(&b) && b != b'=';
            let piece_len = if literal { 1 } else { 3 };
            // A soft break keeps the line at 76 characters, `=` included.
            if len + piece_len > 75 {
                out.push_str("=\r\n");
                len = 0;
            }
            if literal {
                out.push(b as char);
            } else {
                let _ = write!(out, "={b:02X}");
            }
            len += piece_len;
        }
        out.push_str("\r\n");
    }
}

/// Base64 (RFC 4648), with padding.
pub fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The `Date` of a mail (RFC 5322, 3.3), in local time:
/// `Sat, 07 Nov 2026 10:12:00 +0100`.
pub fn rfc5322_date(unix: i64, tz_offset_secs: i32) -> String {
    let local = unix + i64::from(tz_offset_secs);
    let days = local.div_euclid(86_400);
    let secs = local.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    // 1970-01-01 was a Thursday.
    const WEEKDAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let sign = if tz_offset_secs < 0 { '-' } else { '+' };
    let off = tz_offset_secs.unsigned_abs() / 60;
    format!(
        "{}, {:02} {} {} {:02}:{:02}:{:02} {}{:02}{:02}",
        WEEKDAYS[days.rem_euclid(7) as usize],
        d,
        MONTHS[(m - 1) as usize],
        y,
        secs / 3600,
        secs / 60 % 60,
        secs % 60,
        sign,
        off / 60,
        off % 60
    )
}

/// Year, month (1 to 12), day of a count of days since 1970-01-01, in the
/// proleptic Gregorian calendar (Howard Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// The `Message-ID` of a new mail, without angle brackets: a random part
/// given by the caller (the store, which has the random generator) and the
/// domain of the address it is sent from, so that it is unique and says
/// nothing of the Mac.
pub fn new_message_id(random: u128, from_addr: &str) -> String {
    let domain = from_addr.rsplit_once('@').map(|(_, d)| d.to_lowercase()).filter(|d| valid_msgid(d)).unwrap_or_else(|| "nicemailer.invalid".into());
    format!("{random:032x}@{domain}")
}

/// `save_sent` of an account (`reponse.md`, section 9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SaveSent {
    /// Copy to Sent unless the provider does.
    #[default]
    Auto,
    Always,
    Never,
}

/// Whether NiceMailer must copy a sent mail to Sent itself
/// (`reponse.md`, section 9): not when the provider does it (Gmail, which
/// says so by its IMAP capability `X-GM-EXT-1`, and Fastmail), which would
/// give two copies.
pub fn must_copy_to_sent(save: SaveSent, smtp_host: &str, gmail_capability: bool) -> bool {
    match save {
        SaveSent::Always => true,
        SaveSent::Never => false,
        SaveSent::Auto => !provider_copies_sent(smtp_host, gmail_capability),
    }
}

/// Whether the provider of this SMTP server copies the mail it sends to
/// the Sent folder itself.
pub fn provider_copies_sent(smtp_host: &str, gmail_capability: bool) -> bool {
    let h = smtp_host.trim_end_matches('.').to_lowercase();
    let under = |d: &str| h == d || h.ends_with(&format!(".{d}"));
    gmail_capability || h == "smtp.gmail.com" || h == "smtp.googlemail.com" || under("fastmail.com") || under("messagingengine.com")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mb(addr: &str) -> Mailbox {
        Mailbox::new(addr, None)
    }

    fn addrs(list: &[Mailbox]) -> Vec<&str> {
        list.iter().map(|m| m.addr.as_str()).collect()
    }

    /// Joins the pieces of a `format=flowed` text, as a reader does: what
    /// the receiver reads must be what the user typed.
    fn unflow(text: &str) -> String {
        let mut out = String::new();
        let mut joining = false;
        for line in text.split("\r\n") {
            let depth = line.chars().take_while(|c| *c == '>').count();
            let mut content = &line[depth..];
            content = content.strip_prefix(' ').unwrap_or(content);
            if !joining && depth > 0 {
                out.push_str(&">".repeat(depth));
                if !content.is_empty() {
                    out.push(' ');
                }
            }
            out.push_str(content);
            joining = content.ends_with(' ') && line != "-- ";
            if !joining {
                out.push('\n');
            }
        }
        out.trim_end_matches('\n').to_string()
    }

    #[test]
    fn reply_all_goes_to_the_sender_and_copies_the_others() {
        let from = Mailbox::new("alice@a.fr", Some("Alice"));
        let to = [mb("me@x.fr"), mb("bob@b.fr")];
        let cc = [mb("carol@c.fr")];
        let o = Original { from: Some(&from), reply_to: &[], to: &to, cc: &cc };
        let (t, c) = reply_all(&o, &["me@x.fr".into()]);
        assert_eq!(addrs(&t), ["alice@a.fr"]);
        assert_eq!(t[0].name.as_deref(), Some("Alice"));
        assert_eq!(addrs(&c), ["bob@b.fr", "carol@c.fr"]);
    }

    #[test]
    fn reply_to_replaces_the_sender() {
        let from = mb("noreply@a.fr");
        let rt = [mb("list@a.fr")];
        let to = [mb("me@x.fr")];
        let o = Original { from: Some(&from), reply_to: &rt, to: &to, cc: &[] };
        let (t, c) = reply_all(&o, &["me@x.fr".into()]);
        assert_eq!(addrs(&t), ["list@a.fr"]);
        assert!(c.is_empty());
    }

    #[test]
    fn own_addresses_duplicates_and_case_are_removed() {
        let from = mb("Alice@A.fr");
        let to = [mb("ME@x.fr"), mb("alice@a.fr"), mb("other@me.fr"), mb("bob@b.fr")];
        let cc = [mb("BOB@b.fr"), mb("not an address"), mb("x@")];
        let o = Original { from: Some(&from), reply_to: &[], to: &to, cc: &cc };
        let (t, c) = reply_all(&o, &["me@x.fr".into(), "other@me.fr".into()]);
        assert_eq!(addrs(&t), ["alice@a.fr"]);
        assert_eq!(addrs(&c), ["bob@b.fr"]);
    }

    #[test]
    fn a_reply_to_ones_own_mail_takes_its_recipients_back() {
        let from = mb("me@x.fr");
        let to = [mb("bob@b.fr")];
        let cc = [mb("carol@c.fr"), mb("me@x.fr")];
        let o = Original { from: Some(&from), reply_to: &[], to: &to, cc: &cc };
        let (t, c) = reply_all(&o, &["me@x.fr".into()]);
        assert_eq!(addrs(&t), ["bob@b.fr"]);
        assert_eq!(addrs(&c), ["carol@c.fr"]);
    }

    #[test]
    fn to_empty_takes_the_first_copy_and_nobody_left_is_the_sender() {
        // Sent to oneself, with one person in copy.
        let from = mb("me@x.fr");
        let to = [mb("me@x.fr")];
        let cc = [mb("carol@c.fr"), mb("dan@d.fr")];
        let o = Original { from: Some(&from), reply_to: &[], to: &to, cc: &cc };
        let (t, c) = reply_all(&o, &["me@x.fr".into()]);
        assert_eq!(addrs(&t), ["carol@c.fr"]);
        assert_eq!(addrs(&c), ["dan@d.fr"]);
        // Sent to oneself only: the reply goes to oneself.
        let o = Original { from: Some(&from), reply_to: &[], to: &to, cc: &[] };
        let (t, c) = reply_all(&o, &["me@x.fr".into()]);
        assert_eq!(addrs(&t), ["me@x.fr"]);
        assert!(c.is_empty());
        // No sender at all, nobody: an empty reply, refused at sending.
        let o = Original { from: None, reply_to: &[], to: &[], cc: &[] };
        assert_eq!(reply_all(&o, &[]), (vec![], vec![]));
    }

    #[test]
    fn the_identity_is_the_one_the_mail_was_sent_to() {
        let ids = [Mailbox::new("me@x.fr", Some("Me")), Mailbox::new("contact@x.fr", Some("Contact"))];
        let from = mb("a@a.fr");
        let to = [mb("Contact@X.fr")];
        let o = Original { from: Some(&from), reply_to: &[], to: &to, cc: &[] };
        assert_eq!(sender_identity(&ids, &o).addr, "contact@x.fr");
        let to = [mb("list@l.fr")];
        let o = Original { from: Some(&from), reply_to: &[], to: &to, cc: &[] };
        assert_eq!(sender_identity(&ids, &o).addr, "me@x.fr");
        assert_eq!(sender_identity(&[], &o), Mailbox::default());
    }

    #[test]
    fn reply_prefixes_are_never_stacked() {
        assert_eq!(reply_subject("Devis"), "Re: Devis");
        assert_eq!(reply_subject("  Devis  "), "Re: Devis");
        assert_eq!(reply_subject(""), "Re: ");
        for s in ["Re: Devis", "RE: Devis", "re:Devis", "Re[2]: Devis", "Re(3): Devis", "AW: Angebot", "Sv: Hej", "Réf. : Dossier", "Antw: x", "Re : Devis"] {
            assert_eq!(reply_subject(s), s, "{s}");
        }
        // A word that only starts like a prefix is not one.
        assert_eq!(reply_subject("Relevé: novembre"), "Re: Relevé: novembre");
        assert_eq!(reply_subject("Réunion"), "Re: Réunion");
        // A forward is answered like any mail.
        assert_eq!(reply_subject("TR: Devis"), "Re: TR: Devis");
        assert_eq!(reply_subject("Fwd: Devis"), "Re: Fwd: Devis");
    }

    #[test]
    fn references_follow_the_parent_and_stay_bounded() {
        let refs: Vec<String> = vec!["a@x".into(), "b@x".into()];
        assert_eq!(references(&refs, Some("b@x"), Some("c@x")), ["a@x", "b@x", "c@x"]);
        assert_eq!(references(&[], Some("<b@x>"), Some("c@x")), ["b@x", "c@x"]);
        assert_eq!(references(&[], None, Some("c@x")), ["c@x"]);
        assert!(references(&[], None, None).is_empty());
        // Duplicates and invalid identifiers are dropped.
        let refs: Vec<String> = vec!["a@x".into(), "a@x".into(), "bad id".into()];
        assert_eq!(references(&refs, None, Some("a@x")), ["a@x"]);
        // 30 identifiers: the root and the 19 latest.
        let refs: Vec<String> = (0..30).map(|i| format!("m{i}@x")).collect();
        let out = references(&refs, None, Some("p@x"));
        assert_eq!(out.len(), REFERENCES_MAX);
        assert_eq!(out[0], "m0@x");
        assert_eq!(out[1], "m12@x");
        assert_eq!(out.last().unwrap(), "p@x");
    }

    #[test]
    fn quoting_marks_every_line_once() {
        let q = quote("Bonjour,  \r\n\r\n> Avant\nFin\n\n\n", 1000);
        assert_eq!(q, "> Bonjour,\n>\n>> Avant\n> Fin\n");
        assert_eq!(quote("", 1000), "");
        let long: String = (0..10).map(|i| format!("l{i}\n")).collect();
        assert_eq!(quote(&long, 3), "> l0\n> l1\n> l2\n> […]\n");
        assert_eq!(quote(&long, 10).lines().count(), 10);
    }

    #[test]
    fn the_initial_body_leaves_room_above_the_quotation() {
        assert_eq!(initial_body("On Nov 7, Alice wrote:", "> Hi\n"), "\n\nOn Nov 7, Alice wrote:\n> Hi\n");
        // A header from the client cannot add lines.
        assert_eq!(initial_body("a\nb", ""), "\n\na b\n");
    }

    #[test]
    fn flowed_cuts_long_lines_after_spaces_and_reads_back_the_same() {
        let text = "Bonjour Alice,\n\nVoici une très longue ligne qui dépasse largement la largeur de soixante-douze caractères, et qu'il faut couper.\n\n> Une citation elle aussi beaucoup trop longue pour tenir sur une seule ligne de soixante-douze caractères.\n";
        let f = flowed(text);
        for line in f.split("\r\n") {
            assert!(line.chars().count() <= 78, "{line:?}");
        }
        // Every piece of the quotation keeps its mark.
        assert!(f.lines().filter(|l| l.starts_with("> ")).count() >= 2, "{f}");
        assert!(f.ends_with("\r\n") && !f.ends_with("\r\n\r\n"));
        assert_eq!(unflow(&f), text.trim_end());
    }

    #[test]
    fn flowed_keeps_what_was_typed() {
        // Trailing spaces would join the line with the next: removed.
        assert_eq!(flowed("a  \nb"), "a\r\nb\r\n");
        // A leading space and "From " are stuffed.
        assert_eq!(flowed(" indent\nFrom here"), "  indent\r\n From here\r\n");
        // The signature separator keeps its space.
        assert_eq!(flowed("-- \nMe"), "-- \r\nMe\r\n");
        // Nested quotations, an empty quoted line.
        assert_eq!(flowed(">> deep\n>\n> x"), ">> deep\r\n>\r\n> x\r\n");
        // A word longer than the line is not cut.
        let url = format!("see https://example.com/{}", "a".repeat(100));
        let f = flowed(&url);
        assert_eq!(f, format!("see \r\nhttps://example.com/{}\r\n", "a".repeat(100)));
        assert_eq!(unflow(&f), url);
        // Several spaces between words stay with the piece before the cut.
        let spaced = format!("{} {}   {}", "x".repeat(40), "y".repeat(28), "z".repeat(30));
        assert_eq!(unflow(&flowed(&spaced)), spaced);
    }

    #[test]
    fn the_message_has_the_headers_of_a_reply() {
        let from = Mailbox::new("me@x.fr", Some("Achille Moi"));
        let to = [Mailbox::new("alice@a.fr", Some("Alice Martin"))];
        let cc = [Mailbox::new("bob@b.fr", Some("Dupont, Bob")), mb("carol@c.fr")];
        let refs = vec!["root@a".to_string(), "p@a".to_string()];
        let h = Outgoing {
            from: &from,
            to: &to,
            cc: &cc,
            bcc: &[],
            subject: "Re: Devis",
            date: 1_793_960_000,
            tz_offset_secs: 3600,
            message_id: "abc@x.fr",
            in_reply_to: Some("p@a"),
            references: &refs,
            user_agent: "NiceMailer/0.1.0",
            draft_revision: None,
        };
        let m = String::from_utf8(build_message(&h, "Merci !\n\n> Hi")).unwrap();
        assert!(m.contains("From: Achille Moi <me@x.fr>\r\n"), "{m}");
        assert!(m.contains("To: Alice Martin <alice@a.fr>\r\n"));
        assert!(m.contains("Cc: \"Dupont, Bob\" <bob@b.fr>, carol@c.fr\r\n"));
        assert!(m.contains("Subject: Re: Devis\r\n"));
        assert!(m.contains("Message-ID: <abc@x.fr>\r\n"));
        assert!(m.contains("In-Reply-To: <p@a>\r\n"));
        assert!(m.contains("References: <root@a>\r\n <p@a>\r\n"));
        assert!(m.contains("Content-Type: text/plain; charset=utf-8; format=flowed\r\n"));
        assert!(m.contains("Content-Transfer-Encoding: 7bit\r\n\r\nMerci !\r\n\r\n> Hi\r\n"));
        assert!(!m.to_lowercase().contains("bcc"));
        assert!(!m.contains("X-NiceMailer-Draft-Revision"));
        let draft = String::from_utf8(build_message(&Outgoing { draft_revision: Some(7), ..h.clone() }, "x")).unwrap();
        assert!(draft.contains("X-NiceMailer-Draft-Revision: 7\r\n"), "{draft}");
        assert!(m.split("\r\n").all(|l| l.len() <= 998));
        assert!(!m.replace("\r\n", "").contains('\n'), "bare LF");
    }

    #[test]
    fn non_ascii_goes_encoded() {
        let from = Mailbox::new("me@x.fr", Some("Hélène"));
        let to = [mb("a@a.fr")];
        let h = Outgoing {
            from: &from,
            to: &to,
            cc: &[],
            bcc: &[],
            subject: "Re: Réunion de lundi, ordre du jour détaillé et très long à n'en plus finir",
            date: 0,
            tz_offset_secs: 0,
            message_id: "m@x.fr",
            in_reply_to: None,
            references: &[],
            user_agent: "NiceMailer",
            draft_revision: None,
        };
        let m = String::from_utf8(build_message(&h, "Très bien, à lundi.")).unwrap();
        assert!(m.is_ascii());
        assert!(m.contains("From: =?UTF-8?B?SMOpbMOobmU=?= <me@x.fr>\r\n"), "{m}");
        assert!(m.contains("Subject: =?UTF-8?B?"));
        assert!(m.split("\r\n").all(|l| l.len() <= 78), "{m}");
        assert!(m.contains("Content-Transfer-Encoding: quoted-printable\r\n\r\nTr=C3=A8s bien, =C3=A0 lundi.\r\n"), "{m}");
        assert!(!m.contains("In-Reply-To") && !m.contains("References"));
    }

    #[test]
    fn header_injection_is_impossible() {
        let from = Mailbox::new("me@x.fr", Some("Evil\r\nBcc: victim@v.fr"));
        let to = [mb("a@a.fr")];
        let h = Outgoing {
            from: &from,
            to: &to,
            cc: &[],
            bcc: &[],
            subject: "Hi\r\nBcc: victim@v.fr",
            date: 0,
            tz_offset_secs: 0,
            message_id: "m@x.fr",
            in_reply_to: Some("bad\r\nid"),
            references: &[],
            user_agent: "NiceMailer",
            draft_revision: None,
        };
        let m = String::from_utf8(build_message(&h, "x")).unwrap();
        assert!(!m.contains("\r\nBcc:"), "{m}");
        assert!(!m.contains("In-Reply-To"));
    }

    #[test]
    fn quoted_printable_keeps_trailing_spaces_and_short_lines() {
        let mut out = String::new();
        let line = format!("{} \r\n", "é".repeat(40));
        quoted_printable(&mut out, &line);
        for l in out.split("\r\n") {
            assert!(l.len() <= 76, "{l}");
        }
        assert!(out.ends_with("=20\r\n"), "{out}");
        let mut out = String::new();
        quoted_printable(&mut out, "a=b\r\n");
        assert_eq!(out, "a=3Db\r\n");
    }

    #[test]
    fn base64_matches_the_rfc() {
        for (i, o) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("foobar", "Zm9vYmFy")] {
            assert_eq!(base64(i.as_bytes()), o);
        }
    }

    #[test]
    fn dates_are_in_local_time() {
        assert_eq!(rfc5322_date(0, 0), "Thu, 01 Jan 1970 00:00:00 +0000");
        // 2026-11-07 10:12:00 in Paris (UTC+1).
        assert_eq!(rfc5322_date(1_794_042_720, 3600), "Sat, 07 Nov 2026 10:12:00 +0100");
        assert_eq!(rfc5322_date(1_794_042_720, -5 * 3600 - 1800), "Sat, 07 Nov 2026 03:42:00 -0530");
        assert_eq!(rfc5322_date(951_782_400, 0), "Tue, 29 Feb 2000 00:00:00 +0000");
    }

    #[test]
    fn message_ids_use_the_senders_domain() {
        assert_eq!(new_message_id(255, "Me@Example.FR"), "000000000000000000000000000000ff@example.fr");
        assert_eq!(new_message_id(1, "broken"), "00000000000000000000000000000001@nicemailer.invalid");
    }

    #[test]
    fn providers_that_copy_to_sent() {
        assert!(provider_copies_sent("smtp.gmail.com", false));
        assert!(provider_copies_sent("SMTP.GoogleMail.com.", false));
        assert!(provider_copies_sent("smtp.fastmail.com", false));
        assert!(provider_copies_sent("mail.messagingengine.com", false));
        assert!(provider_copies_sent("smtp.example.com", true));
        assert!(!provider_copies_sent("mail.infomaniak.com", false));
        assert!(!provider_copies_sent("notfastmail.com", false));
        assert!(must_copy_to_sent(SaveSent::Auto, "mail.infomaniak.com", false));
        assert!(!must_copy_to_sent(SaveSent::Auto, "smtp.gmail.com", false));
        assert!(must_copy_to_sent(SaveSent::Always, "smtp.gmail.com", true));
        assert!(!must_copy_to_sent(SaveSent::Never, "mail.infomaniak.com", false));
    }

    #[test]
    fn addresses_that_cannot_be_sent_to() {
        assert!(valid_addr("a@b.fr"));
        assert!(valid_addr("a.b+c@sub.b.fr"));
        assert!(valid_addr("me@localhost"));
        for bad in ["", "a", "a@", "@b.fr", "a b@c.fr", "a@b", "<a@b.fr>", "a@b.fr>\r\nRCPT TO:<x@y.fr", "a,b@c.fr"] {
            assert!(!valid_addr(bad), "{bad:?}");
        }
    }

    #[test]
    fn a_typed_or_pasted_list_of_addresses_is_read() {
        let (ok, bad) = parse_address_list(
            "alice@x.fr, Bob <BOB@y.fr>; \"Dupont, J.\" <j@z.fr>\nCarol Martin carol@c.fr, <d@d.fr>, mailto:e@e.fr",
        );
        let got: Vec<(&str, Option<&str>)> = ok.iter().map(|m| (m.addr.as_str(), m.name.as_deref())).collect();
        assert_eq!(
            got,
            [
                ("alice@x.fr", None),
                ("bob@y.fr", Some("Bob")),
                ("j@z.fr", Some("Dupont, J.")),
                ("carol@c.fr", Some("Carol Martin")),
                ("d@d.fr", None),
                ("e@e.fr", None),
            ]
        );
        assert!(bad.is_empty());
    }

    #[test]
    fn what_is_not_an_address_is_given_back_as_typed() {
        let (ok, bad) = parse_address_list("alice@x.fr, alice, bob@, ALICE@x.fr, , Evil <a@b.fr>\r\nBcc: c@d.fr");
        // The line break separates: what would have been an injected header
        // is only one more recipient in the line, seen by the user, named
        // "Bcc:" by the rule "a name, then the address".
        assert_eq!(ok.iter().map(|m| m.addr.as_str()).collect::<Vec<_>>(), ["alice@x.fr", "a@b.fr", "c@d.fr"]);
        assert_eq!(ok[2].name.as_deref(), Some("Bcc:"));
        assert_eq!(bad, ["alice", "bob@"]);
        assert_eq!(parse_address_list("  ").0, []);
        // A quoted name with an escaped quote.
        let (ok, _) = parse_address_list(r#""Al \"Bo\" C" <a@b.fr>"#);
        assert_eq!(ok[0].name.as_deref(), Some(r#"Al "Bo" C"#));
    }

    #[test]
    fn recipients_are_cleaned_across_the_three_lists() {
        let to = [Mailbox::new("A@x.fr", Some("  Alice ")), Mailbox::new("b@x.fr", Some(""))];
        let cc = [Mailbox::new("a@x.fr", None), Mailbox::new("c@x.fr", Some("C\r\nBcc: z@z.fr"))];
        let bcc = [Mailbox::new("c@x.fr", None), Mailbox::new("d@x.fr", None)];
        let [to, cc, bcc] = clean_recipients([&to, &cc, &bcc]).unwrap();
        assert_eq!(to, [Mailbox::new("a@x.fr", Some("Alice")), Mailbox::new("b@x.fr", None)]);
        assert_eq!(cc, [Mailbox::new("c@x.fr", Some("C  Bcc: z@z.fr"))]);
        assert_eq!(bcc, [Mailbox::new("d@x.fr", None)]);
        assert_eq!(clean_recipients([&[Mailbox::new("nope", None)], &[], &[]]), Err("nope".to_string()));
        let many: Vec<Mailbox> = (0..=RECIPIENTS_MAX).map(|i| Mailbox::new(&format!("p{i}@x.fr"), None)).collect();
        assert!(clean_recipients([&many, &[], &[]]).is_err());
    }

    #[test]
    fn a_subject_is_one_clean_line() {
        assert_eq!(clean_subject("  Devis\r\nBcc: x@y.fr\u{7} "), "Devis  Bcc: x@y.fr");
        assert_eq!(clean_subject(&"é".repeat(2000)).chars().count(), 998);
    }

    #[test]
    fn bcc_is_written_only_when_given() {
        let from = Mailbox::new("me@x.fr", None);
        let to = [Mailbox::new("a@x.fr", None)];
        let bcc = [Mailbox::new("secret@x.fr", Some("Secret"))];
        let h = Outgoing {
            from: &from,
            to: &to,
            cc: &[],
            bcc: &bcc,
            subject: "S",
            date: 0,
            tz_offset_secs: 0,
            message_id: "id@x.fr",
            in_reply_to: None,
            references: &[],
            user_agent: "t",
            draft_revision: None,
        };
        let kept = String::from_utf8(build_message(&h, "x")).unwrap();
        assert!(kept.contains("\r\nBcc: Secret <secret@x.fr>\r\n"), "{kept}");
        let sent = String::from_utf8(build_message(&Outgoing { bcc: &[], ..h }, "x")).unwrap();
        assert!(!sent.to_lowercase().contains("bcc"), "{sent}");
        assert!(!sent.contains("secret"));
    }
}
