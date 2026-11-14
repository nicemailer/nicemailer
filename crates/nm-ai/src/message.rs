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
}
