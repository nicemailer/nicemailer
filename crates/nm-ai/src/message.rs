//! The user message sent to the model: the inputs of a prompt, each in its
//! tagged section (`specifications/ia.md`, section 5, "Entrées").
//!
//! Only the segments of the user's text are here so far; the other inputs
//! come with the core's code that computes them.

use crate::provider::AiError;

/// The segments of `draft.own_text`, numbered from 1, as the `replace`
/// output expects them back ([`crate::answer::segments`]).
///
/// A text that already holds a segment tag is refused rather than sent:
/// the answer could then not be read without guessing which text is whose.
pub fn segments(texts: &[&str]) -> Result<String, AiError> {
    if texts.iter().all(|t| t.trim().is_empty()) {
        return Err(AiError::BadInput(
            "there is no text of yours to send: write some first".into(),
        ));
    }
    let mut out = String::new();
    for (i, t) in texts.iter().enumerate() {
        if t.contains("<segment") || t.contains("</segment>") {
            return Err(AiError::BadInput(
                "the text contains a <segment> tag, which the AI uses to give the text back; remove it first".into(),
            ));
        }
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&format!("<segment n=\"{}\">{}</segment>", i + 1, t));
    }
    Ok(out)
}

/// The segments of a formatted draft (`draft.own_text` of a `replace`
/// prompt): the user's blocks as HTML, numbered as [`segments`] numbers
/// them, after a note that tells the model to answer in the same HTML. Its
/// formatting (headings, lists, bold, links) then stays on the words it
/// was on; the core cleans the answer before it goes into the draft
/// (`nm_store::mail_html::ai_block`), whatever the model wrote.
pub fn html_segments(texts: &[&str]) -> Result<String, AiError> {
    let segments = segments(texts)?;
    Ok(format!(
        "{}\n\n{segments}",
        section(
            "format",
            "Each segment is HTML, as a mail editor writes it: paragraphs <p> (an empty <p></p> is a blank \
             line), headings <h1> to <h3>, lists <ul> and <ol> of <li><p>…</p></li>, <strong>, <em>, <u>, \
             links <a href>, <span style> and <br>. Give each segment back as HTML of the same kinds: keep \
             each tag around the words it formats, one <p> per paragraph, and no other tag. Write the text \
             as HTML text: &amp; for &, &lt; for <."
        )
    ))
}

/// The user's text for an output that does not replace it (subjects, …):
/// the segments one after the other, in a `<draft>` section.
pub fn draft_text(texts: &[&str]) -> Result<String, AiError> {
    if texts.iter().all(|t| t.trim().is_empty()) {
        return Err(AiError::BadInput(
            "there is no text of yours to send: write some first".into(),
        ));
    }
    Ok(section("draft", &texts.join("\n\n")))
}

/// `draft.own_text` of a prompt of a passage (`on = "selection"`): the
/// passage the user selected, alone, in a `<passage>` section.
pub fn passage(text: &str) -> Result<String, AiError> {
    if text.trim().is_empty() {
        return Err(AiError::BadInput("the selection is empty".into()));
    }
    Ok(section("passage", text))
}

/// Most characters of `draft.context`, the passage included.
pub const MAX_CONTEXT_CHARS: usize = 12_000;

/// `draft.context`: the draft's whole body, the passage selected marked
/// `<selected>…</selected>` where it stands, so that the model fits its
/// answer to what comes before and after: the user's sentences, and the
/// mail replied to, quoted below. A long draft keeps the passage whole and
/// as much text on each side as the limit allows, each cut said by `[…]`.
///
/// `selection` is a byte range of `body` on character boundaries (the
/// core checked it).
pub fn draft_context(body: &str, selection: std::ops::Range<usize>) -> String {
    let before = &body[..selection.start];
    let chosen = &body[selection.clone()];
    let after = &body[selection.end..];
    let room = MAX_CONTEXT_CHARS.saturating_sub(chosen.chars().count()) / 2;
    // Rust note: `chars().rev().take(n)` walks back from the end by
    // characters, never cutting one of several bytes in two.
    let before_kept: usize = before.chars().rev().take(room).map(char::len_utf8).sum();
    let after_kept: usize = after.chars().take(room).map(char::len_utf8).sum();
    let mut text = String::new();
    if before_kept < before.len() {
        text.push_str("[…]");
    }
    text.push_str(&before[before.len() - before_kept..]);
    text.push_str("<selected>");
    text.push_str(chosen);
    text.push_str("</selected>");
    text.push_str(&after[..after_kept]);
    if after_kept < after.len() {
        text.push_str("[…]");
    }
    section("draft", &text)
}

/// The headers of a draft: From, To, Cc, Subject, one per line, the empty
/// ones left out.
pub fn draft_headers(from: &str, to: &[String], cc: &[String], subject: &str) -> String {
    let mut lines = vec![format!("From: {from}")];
    if !to.is_empty() {
        lines.push(format!("To: {}", to.join(", ")));
    }
    if !cc.is_empty() {
        lines.push(format!("Cc: {}", cc.join(", ")));
    }
    if !subject.trim().is_empty() {
        lines.push(format!("Subject: {}", subject.trim()));
    }
    section("headers", &lines.join("\n"))
}

/// A tagged section: `<tag>\n…\n</tag>`. The guard of every system prompt
/// names these sections as data (`crate::prompt::GUARD`).
pub fn section(tag: &str, text: &str) -> String {
    format!("<{tag}>\n{}\n</{tag}>", text.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::answer;

    #[test]
    fn the_mail_replied_to_and_the_user_are_sections() {
        let m = Mail {
            from: "Alice <alice@a.example>".into(),
            date: "Tue, 23 Jun 2020 19:20:56 +0200".into(),
            subject: "Devis".into(),
            text: "Tu viens jeudi ?".into(),
            ..Default::default()
        };
        assert_eq!(
            replied(Some(&m)),
            "<replied>\nFrom: Alice <alice@a.example>\nDate: Tue, 23 Jun 2020 19:20:56 +0200\nSubject: Devis\n\nTu viens jeudi ?\n</replied>"
        );
        assert_eq!(replied(None), "<replied>\n(none: this is a new mail, not a reply)\n</replied>");
        let long = Mail { text: "x".repeat(MAX_REPLIED_CHARS + 5), ..m };
        assert!(replied(Some(&long)).ends_with("[…]\n</replied>"));
        assert_eq!(draft_identity("Me <me@x.fr>", false), "<user>\nThe user writes from: Me <me@x.fr>\n</user>");
        assert!(draft_identity("Me <me@x.fr>", true).contains("without signing"));
    }

    #[test]
    fn an_attachment_comes_with_its_name_and_is_cut() {
        assert_eq!(attachment_text("devis.pdf", "pdf", "Total : 12 €"), "<attachment>\nFile: devis.pdf (pdf)\n\nTotal : 12 €\n</attachment>");
        let long = attachment_text("a.txt", "text", &"é".repeat(MAX_ATTACHMENT_CHARS + 1));
        assert!(long.ends_with("[…]\n</attachment>"), "the model is told it is only the start");
    }

    #[test]
    fn a_passage_comes_with_its_draft_marked() {
        let body = "Bonjour Paul,\n\nJe viens jeudi.\n\nÀ bientôt";
        let start = body.find("viens").unwrap();
        let ctx = draft_context(body, start..start + "viens".len());
        assert_eq!(
            ctx,
            "<draft>\nBonjour Paul,\n\nJe <selected>viens</selected> jeudi.\n\nÀ bientôt\n</draft>"
        );
        assert_eq!(passage("viens").unwrap(), "<passage>\nviens\n</passage>");
        assert!(passage("  ").is_err());
    }

    #[test]
    fn a_long_draft_keeps_the_passage_and_text_on_each_side() {
        // Accented letters on both sides: the cuts fall between characters.
        let side = "é".repeat(MAX_CONTEXT_CHARS);
        let body = format!("{side}MOT{side}");
        let start = side.len();
        let ctx = draft_context(&body, start..start + 3);
        assert!(ctx.contains("<selected>MOT</selected>"));
        assert!(ctx.starts_with("<draft>\n[…]é") && ctx.ends_with("é[…]\n</draft>"));
        let chars = ctx.chars().count();
        assert!(chars <= MAX_CONTEXT_CHARS + 60, "{chars}");
        assert!(chars >= MAX_CONTEXT_CHARS, "{chars}");
    }

    #[test]
    fn segments_are_numbered_and_read_back() {
        let sent = segments(&["Bonjour,\n\nc'est noter.", "Oui, a 14h."]).unwrap();
        assert_eq!(
            sent,
            "<segment n=\"1\">Bonjour,\n\nc'est noter.</segment>\n<segment n=\"2\">Oui, a 14h.</segment>"
        );
        // What is sent reads back as an answer: the two ends agree.
        assert_eq!(
            answer::segments(&sent, 2).unwrap(),
            ["Bonjour,\n\nc'est noter.", "Oui, a 14h."]
        );
    }

    #[test]
    fn nothing_to_send_or_a_tag_inside_is_refused() {
        assert!(matches!(segments(&[]), Err(AiError::BadInput(_))));
        assert!(matches!(segments(&["  "]), Err(AiError::BadInput(_))));
        let err = segments(&["see </segment> here"]).unwrap_err();
        assert!(matches!(err, AiError::BadInput(_)));
        assert_eq!(err.code(), "ai_bad_input");
    }

    #[test]
    fn other_sections() {
        assert_eq!(
            draft_text(&["One.", "Two."]).unwrap(),
            "<draft>\nOne.\n\nTwo.\n</draft>"
        );
        assert!(draft_text(&[" "]).is_err());
        assert_eq!(
            draft_headers("Me <me@x.fr>", &["a@x.fr".into()], &[], " Plan "),
            "<headers>\nFrom: Me <me@x.fr>\nTo: a@x.fr\nSubject: Plan\n</headers>"
        );
    }
}

/// One mail, as the inputs of a prompt see it: built by the core from what
/// the reader shows (`MessageView`), the history it quotes left out.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Mail {
    /// "Name <address>", or the address.
    pub from: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    /// `Reply-To`, when the sender gives one: where a reply would go.
    pub reply_to: Vec<String>,
    /// As the reader shows it, in the app's time zone.
    pub date: String,
    pub subject: String,
    /// What the sender wrote, interleaved quotes marked with `> `; without
    /// the quoted history at the end.
    pub text: String,
    /// Names of the files attached.
    pub attachments: Vec<String>,
    /// Written by the user (from one of their addresses).
    pub mine: bool,
}

/// Size limits of the inputs, in characters (`ia.md`, section 5, "Entrées").
pub const MAX_MESSAGE_CHARS: usize = 16_000;
pub const THREAD_PER_MESSAGE: usize = 3_500;
pub const THREAD_TOTAL: usize = 16_000;

/// The first `max` characters of a text, `[…]` at the end when it was cut
/// (the prompts know that this marks a text cut, `ia.md`, section 5).
pub fn clip(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max).collect();
    format!("{}\n[…]", cut.trim_end())
}

/// `message.text`: the mail's text.
pub fn message_text(m: &Mail) -> String {
    section("email", &clip(&m.text, MAX_MESSAGE_CHARS))
}

/// `message.headers`: From, To, Cc, Date, Subject, the files attached.
pub fn message_headers(m: &Mail) -> String {
    let mut lines = vec![format!("From: {}", m.from)];
    if !m.reply_to.is_empty() {
        lines.push(format!("Reply-To: {}", m.reply_to.join(", ")));
    }
    if !m.to.is_empty() {
        lines.push(format!("To: {}", m.to.join(", ")));
    }
    if !m.cc.is_empty() {
        lines.push(format!("Cc: {}", m.cc.join(", ")));
    }
    lines.push(format!("Date: {}", m.date));
    lines.push(format!("Subject: {}", m.subject));
    lines.push(format!(
        "Attachments: {}",
        if m.attachments.is_empty() {
            "(none)".to_string()
        } else {
            m.attachments.join(", ")
        }
    ));
    section("headers", &lines.join("\n"))
}

/// `thread.text`: the mails of a conversation, oldest first, each cut at
/// [`THREAD_PER_MESSAGE`]; beyond [`THREAD_TOTAL`] in all, the oldest are
/// left out (the newest are the ones a summary or a reply needs), and the
/// section says how many.
pub fn thread_text(mails: &[Mail]) -> String {
    let blocks: Vec<String> = mails
        .iter()
        .map(|m| {
            let who = if m.mine {
                format!("{} (the user)", m.from)
            } else {
                m.from.clone()
            };
            format!(
                "--- From {who}, {}:\n{}",
                m.date,
                clip(&m.text, THREAD_PER_MESSAGE)
            )
        })
        .collect();
    // The newest first, while they fit.
    let mut kept: Vec<&String> = Vec::new();
    let mut size = 0;
    for b in blocks.iter().rev() {
        let n = b.chars().count();
        if size + n > THREAD_TOTAL && !kept.is_empty() {
            break;
        }
        size += n;
        kept.push(b);
    }
    kept.reverse();
    let left_out = blocks.len() - kept.len();
    let mut text = String::new();
    if left_out > 0 {
        text.push_str(&format!("[{left_out} older message(s) left out]\n\n"));
    }
    text.push_str(
        &kept
            .iter()
            .map(|b| b.as_str())
            .collect::<Vec<_>>()
            .join("\n\n"),
    );
    section("conversation", &text)
}

/// `message.auth`: the authentication verdict, and what the core found in
/// the headers and links (`crate::answer::Finding`).
pub fn auth(verdict: &str, findings: &[crate::answer::Finding]) -> String {
    let mut lines = vec![format!(
        "Authentication of the sender (SPF, DKIM, DMARC): {verdict}"
    )];
    if findings.is_empty() {
        lines.push("Found by the app: nothing unusual in the headers and links.".into());
    } else {
        lines.push("Found by the app:".into());
        lines.extend(findings.iter().map(|f| format!("- {}", f.text)));
    }
    section("authentication", &lines.join("\n"))
}

/// `sender.history`: what the mailbox knows of the sender.
pub fn sender_history(lines: &[String]) -> String {
    section("sender", &lines.join("\n"))
}

/// `user.identity`: who the user is.
pub fn identity(addresses: &[String]) -> String {
    section(
        "user",
        &format!("The user writes from: {}", addresses.join(", ")),
    )
}

/// `user.identity` of a draft: the user as the draft's From names them
/// ("Achille Martin <achille@…>"), which a mail written for them is signed
/// with, and whether the draft already ends with their signature, which
/// then signs it instead (`nm_core::segments::has_signature`).
pub fn draft_identity(from: &str, signed: bool) -> String {
    let mut text = format!("The user writes from: {from}");
    if signed {
        text.push_str(
            "\nThe draft already ends with the user's signature, kept below your text: end with the closing \
             words, without signing.",
        );
    }
    section("user", &text)
}

/// Most characters of `draft.replied` (`ia.md`, section 5, "Entrées").
pub const MAX_REPLIED_CHARS: usize = 8_000;

/// `draft.replied`: the mail a draft answers, its headers then its text;
/// for a new mail, a section that says there is none, so that the prompt's
/// language rule (`Prompt::system`) knows to follow the notes.
pub fn replied(mail: Option<&Mail>) -> String {
    let Some(m) = mail else {
        return section("replied", "(none: this is a new mail, not a reply)");
    };
    let text = format!(
        "From: {}\nDate: {}\nSubject: {}\n\n{}",
        m.from,
        m.date,
        m.subject,
        clip(&m.text, MAX_REPLIED_CHARS)
    );
    section("replied", &text)
}

/// Most characters of `attachment.text` (`ia.md`, section 5, "Entrées").
pub const MAX_ATTACHMENT_CHARS: usize = 20_000;

/// `attachment.text`: the file's name and kind, then its text, cut at
/// [`MAX_ATTACHMENT_CHARS`] (`[…]` then says it is only the start, which
/// the summary prompt mentions).
pub fn attachment_text(name: &str, kind: &str, text: &str) -> String {
    section("attachment", &format!("File: {name} ({kind})\n\n{}", clip(text, MAX_ATTACHMENT_CHARS)))
}

/// One conversation of a batch (`selection.items`): its id, which the
/// answer names, and what the model needs to sort it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Item {
    pub thread_id: i64,
    /// Its latest mail received.
    pub mail: Mail,
    /// The authentication verdict of that mail, and what the core found.
    pub auth: String,
    pub findings: Vec<crate::answer::Finding>,
    /// What the mailbox knows of its sender.
    pub sender: Vec<String>,
}

/// Start of the text of each conversation of a batch, in characters.
pub const ITEM_TEXT_CHARS: usize = 1_500;

/// `selection.items`: the conversations of a batch, each with its id.
pub fn selection_items(items: &[Item]) -> String {
    let blocks: Vec<String> = items
        .iter()
        .map(|i| {
            let mut lines = vec![
                format!("thread: {}", i.thread_id),
                format!("From: {}", i.mail.from),
                format!("Subject: {}", i.mail.subject),
                format!("Date: {}", i.mail.date),
                format!("Authentication: {}", i.auth),
            ];
            lines.extend(
                i.findings
                    .iter()
                    .map(|f| format!("Found by the app: {}", f.text)),
            );
            lines.extend(i.sender.iter().map(|l| format!("Sender: {l}")));
            if !i.mail.attachments.is_empty() {
                lines.push(format!("Attachments: {}", i.mail.attachments.join(", ")));
            }
            lines.push(format!("Text:\n{}", clip(&i.mail.text, ITEM_TEXT_CHARS)));
            format!("<item>\n{}\n</item>", lines.join("\n"))
        })
        .collect();
    section("conversations", &blocks.join("\n"))
}

/// `targets`: the folders a conversation may go to from the folder shown,
/// by the name the answer must use.
pub fn targets(roles: &[&str]) -> String {
    section(
        "targets",
        &format!("Answer with one of: {}", roles.join(", ")),
    )
}

/// The text of a streamed `replace` answer as the user should see it while
/// it comes: without the `<segment n="…">` tags, two segments apart by an
/// empty line (`specifications/ia.md`, section 11, "Panneau IA").
///
/// A tag may be cut between two fragments (`<seg`, then `ment n="1">`): a
/// `<` that may start one is held back until the next fragment tells.
/// Anything else (a `<` of the text itself) is let through as soon as it
/// cannot be a tag.
#[derive(Debug, Default)]
pub struct SegmentFilter {
    /// Text received and not shown yet: a possible tag being written.
    held: String,
    /// Whether a segment has been opened: the next opening tag separates.
    opened: bool,
    /// Whether the text is inside a segment. Outside, it is only the
    /// whitespace between segments (`answer::segments` refuses anything
    /// else), and is not shown.
    inside: bool,
}

impl SegmentFilter {
    pub fn new() -> Self {
        SegmentFilter::default()
    }

    /// Takes a fragment, gives what can be shown of it now.
    pub fn push(&mut self, fragment: &str) -> String {
        self.held.push_str(fragment);
        let mut out = String::new();
        loop {
            let Some(lt) = self.held.find('<') else {
                if self.inside {
                    out.push_str(&self.held);
                }
                self.held.clear();
                break;
            };
            if self.inside {
                out.push_str(&self.held[..lt]);
            }
            let rest = &self.held[lt..];
            match tag_at(rest) {
                Tag::Open(len) => {
                    if self.opened {
                        out.push_str("\n\n");
                    }
                    self.opened = true;
                    self.inside = true;
                    self.held.drain(..lt + len);
                }
                Tag::Close(len) => {
                    self.inside = false;
                    self.held.drain(..lt + len);
                }
                // Maybe a tag, not complete yet: wait for more.
                Tag::Partial => {
                    self.held.drain(..lt);
                    break;
                }
                Tag::Not => {
                    if self.inside {
                        out.push('<');
                    }
                    self.held.drain(..lt + 1);
                }
            }
        }
        out
    }

    /// The end of the answer: what was held back is text after all, if it
    /// was inside a segment.
    pub fn finish(&mut self) -> String {
        let held = std::mem::take(&mut self.held);
        if self.inside { held } else { String::new() }
    }
}

enum Tag {
    /// An opening tag of this many bytes.
    Open(usize),
    /// A closing tag.
    Close(usize),
    /// The start of a tag, not complete yet.
    Partial,
    Not,
}

/// What `s`, which starts with `<`, starts with.
fn tag_at(s: &str) -> Tag {
    const OPEN: &str = "<segment n=\"";
    const CLOSE: &str = "</segment>";
    if s.starts_with(CLOSE) {
        return Tag::Close(CLOSE.len());
    }
    if CLOSE.starts_with(s) || OPEN.starts_with(s) {
        return Tag::Partial;
    }
    if let Some(after) = s.strip_prefix(OPEN) {
        let digits = after.bytes().take_while(u8::is_ascii_digit).count();
        let rest = &after[digits..];
        if rest.is_empty() {
            return Tag::Partial;
        }
        if digits > 0 && rest.starts_with("\">") {
            return Tag::Open(OPEN.len() + digits + 2);
        }
        if digits > 0 && rest == "\"" {
            return Tag::Partial;
        }
    }
    Tag::Not
}

#[cfg(test)]
mod mail_tests {
    use super::*;

    fn mail(from: &str, text: &str) -> Mail {
        Mail {
            from: from.into(),
            date: "Nov 14".into(),
            subject: "Plan".into(),
            text: text.into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_long_text_is_cut_and_says_so() {
        assert_eq!(clip("  short  ", 10), "short");
        let c = clip(&"é".repeat(30), 10);
        assert_eq!(c, format!("{}\n[…]", "é".repeat(10)));
    }

    #[test]
    fn headers_list_what_is_known() {
        let mut m = mail("Alice <a@x.fr>", "Hi");
        m.to = vec!["me@x.fr".into()];
        m.attachments = vec!["devis.pdf".into()];
        assert_eq!(
            message_headers(&m),
            "<headers>\nFrom: Alice <a@x.fr>\nTo: me@x.fr\nDate: Nov 14\nSubject: Plan\nAttachments: devis.pdf\n</headers>"
        );
    }

    #[test]
    fn a_conversation_keeps_its_newest_mails() {
        let mut mails: Vec<Mail> = (0..10)
            .map(|i| mail(&format!("P{i}"), &format!("{i}{}", "x".repeat(3_000))))
            .collect();
        mails[9].mine = true;
        let t = thread_text(&mails);
        assert!(t.contains("--- From P9 (the user)"));
        assert!(t.contains("--- From P5,"));
        assert!(
            !t.contains("--- From P4,"),
            "beyond the total, the oldest go"
        );
        assert!(t.contains("[5 older message(s) left out]"));
        // In order, oldest first.
        assert!(t.find("P5,").unwrap() < t.find("P9 ").unwrap());
        // One mail longer than everything is still given, cut.
        let one = thread_text(&[mail("A", &"y".repeat(50_000))]);
        assert!(one.contains("[…]") && !one.contains("left out"));
    }

    #[test]
    fn a_batch_names_each_conversation() {
        let mut m = mail("Shop <news@shop.example>", &"x".repeat(2_000));
        m.attachments = vec!["promo.pdf".into()];
        let item = Item {
            thread_id: 42,
            mail: m,
            auth: "pass".into(),
            findings: vec![crate::answer::Finding {
                text: "a link lies".into(),
                strong: true,
            }],
            sender: vec!["Mails received from it: 3".into()],
        };
        let s = selection_items(&[
            item.clone(),
            Item {
                thread_id: 43,
                ..item
            },
        ]);
        assert!(
            s.starts_with("<conversations>\n<item>\nthread: 42\nFrom: Shop <news@shop.example>"),
            "{s}"
        );
        assert!(s.contains("Found by the app: a link lies\nSender: Mails received from it: 3\nAttachments: promo.pdf\nText:\n"));
        assert!(s.contains("thread: 43"));
        assert_eq!(s.matches("[…]").count(), 2, "each text cut at its limit");
        assert_eq!(
            targets(&["inbox", "junk"]),
            "<targets>\nAnswer with one of: inbox, junk\n</targets>"
        );
    }

    #[test]
    fn auth_says_what_was_found() {
        let f = crate::answer::Finding {
            text: "a link shows a.fr but opens b.ru".into(),
            strong: true,
        };
        let a = auth("fail", &[f]);
        assert!(a.contains("DMARC): fail") && a.contains("- a link shows a.fr but opens b.ru"));
        assert!(auth("pass", &[]).contains("nothing unusual"));
    }
}

#[cfg(test)]
mod filter_tests {
    use super::*;

    /// The text shown, the answer given in fragments of `size` bytes.
    fn shown(answer: &str, size: usize) -> String {
        let mut f = SegmentFilter::new();
        let mut out = String::new();
        let mut rest = answer;
        while !rest.is_empty() {
            let mut cut = size.min(rest.len());
            while !rest.is_char_boundary(cut) {
                cut += 1;
            }
            out.push_str(&f.push(&rest[..cut]));
            rest = &rest[cut..];
        }
        out + &f.finish()
    }

    #[test]
    fn tags_are_hidden_however_the_answer_is_cut() {
        let answer = "<segment n=\"1\">Oui, à jeudi.</segment>\n<segment n=\"2\">Je prends les clés.</segment>";
        for size in [1, 2, 3, 5, 7, 11, 1000] {
            assert_eq!(
                shown(answer, size),
                "Oui, à jeudi.\n\nJe prends les clés.",
                "fragments of {size}"
            );
        }
    }

    #[test]
    fn a_less_than_sign_of_the_text_is_kept() {
        let answer = "<segment n=\"1\">2 < 3 and <b> stays, <seg too</segment>";
        for size in [1, 4, 100] {
            assert_eq!(
                shown(answer, size),
                "2 < 3 and <b> stays, <seg too",
                "fragments of {size}"
            );
        }
        // An answer that ends on what looked like the start of a tag.
        assert_eq!(shown("<segment n=\"1\">x <segm", 3), "x <segm");
        // Text outside the segments is not the user's: not shown.
        assert_eq!(
            shown("Here you go:\n<segment n=\"1\">x</segment>\nDone.", 4),
            "x"
        );
    }
}
