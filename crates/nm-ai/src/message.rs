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
