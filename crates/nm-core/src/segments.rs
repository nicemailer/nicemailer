//! The text the user wrote in a draft, cut into segments, and put back
//! after the AI has rewritten it (`specifications/ia.md`, section 5, "Le
//! texte écrit par l'utilisateur").
//!
//! A reply's body is the user's text above a quotation; a forward's is the
//! user's text above the mail forwarded; either may end with a signature,
//! and the user may write between quoted lines. Proofreading must touch the
//! user's words only: [`own_segments`] finds them, [`splice`] puts the new
//! words in their place, leaving every other byte of the body as it was.
//!
//! # What is not the user's text
//!
//! The two headers the app writes (`reponse.md`, "Corps et citation" and
//! section 4 bis) are in the app's language and not stored, so they are
//! recognised by their form, in any language:
//!
//! - **quoted lines**: they start with `>`;
//! - **the line introducing a quotation**: it ends with `:` and the next
//!   line is quoted ("On Nov 7, 2026, at 10:12, Alice wrote:");
//! - **a forwarded mail**: from a line of dashes around a text
//!   ("---------- Forwarded message ---------") to the end of the body;
//! - **a signature**: from a line `-- ` (or `--`, an editor may have dropped
//!   the space) to the next quoted line or the end.
//!
//! In doubt, a line is left out rather than taken in: a line left out by
//! mistake is only not proofread, while a line taken in by mistake would be
//! rewritten.

use std::ops::Range;

/// What a line of the body is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Own,
    Quoted,
    Intro,
    Signature,
    Forwarded,
}

/// The byte ranges of the user's segments in `body`, in order. Each range
/// starts at its first non-blank line and ends with its last non-blank
/// character: blank lines and the whitespace around a segment stay outside
/// it, so a rewritten segment cannot change the layout around it.
pub fn own_segments(body: &str) -> Vec<Range<usize>> {
    // Rust note: `split_inclusive` keeps the `\n` at the end of each piece,
    // so the pieces' lengths add up to the body's and give byte offsets.
    let lines: Vec<(usize, &str)> = {
        let mut at = 0;
        body.split_inclusive('\n')
            .map(|l| {
                let start = at;
                at += l.len();
                (start, l)
            })
            .collect()
    };
    let kinds = classify(&lines);

    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if kinds[i] != Kind::Own {
            i += 1;
            continue;
        }
        // A run of own lines, from i to j (excluded).
        let mut j = i;
        while j < lines.len() && kinds[j] == Kind::Own {
            j += 1;
        }
        let start = lines[i].0;
        let end = lines[j - 1].0 + lines[j - 1].1.len();
        let text = &body[start..end];
        let lead = text.len() - text.trim_start().len();
        let trail = text.len() - text.trim_end().len();
        if lead < text.len() {
            out.push(start + lead..end - trail);
        }
        i = j;
    }
    out
}

/// The kind of each line.
fn classify(lines: &[(usize, &str)]) -> Vec<Kind> {
    let content = |l: &str| l.trim_end_matches(['\n', '\r']).to_string();
    let quoted = |l: &str| l.starts_with('>');
    let mut kinds = vec![Kind::Own; lines.len()];
    let mut in_signature = false;
    for (i, (_, line)) in lines.iter().enumerate() {
        let c = content(line);
        if is_forward_separator(&c) {
            for k in &mut kinds[i..] {
                *k = Kind::Forwarded;
            }
            break;
        }
        if quoted(&c) {
            // A quotation ends a signature: what follows may be the user's
            // answer to the next quoted part.
            in_signature = false;
            kinds[i] = Kind::Quoted;
            continue;
        }
        if c == "-- " || c == "--" {
            in_signature = true;
        }
        if in_signature {
            kinds[i] = Kind::Signature;
            continue;
        }
        let next_quoted = lines.get(i + 1).is_some_and(|(_, n)| quoted(&content(n)));
        if next_quoted && c.trim_end().ends_with(':') {
            kinds[i] = Kind::Intro;
        }
    }
    kinds
}

/// A line of at least five dashes, some text, and at least five dashes:
/// the header of a forwarded mail, in any language.
fn is_forward_separator(line: &str) -> bool {
    let t = line.trim();
    let inner = t.trim_matches('-');
    t.starts_with("-----") && t.ends_with("-----") && inner.chars().any(char::is_alphabetic)
}

/// The body with each segment replaced by its new text. The whitespace that
/// surrounded each original segment is kept, and the new texts are trimmed:
/// the layout is the body's, the words the AI's.
///
/// `segments` must come from [`own_segments`] (or be one selection), in
/// order and without overlap; `new` must have one text per segment.
pub fn splice(body: &str, segments: &[Range<usize>], new: &[String]) -> Result<String, String> {
    if segments.len() != new.len() {
        return Err(format!(
            "{} segments, {} new texts",
            segments.len(),
            new.len()
        ));
    }
    let mut out = String::with_capacity(body.len() + 64);
    let mut at = 0;
    for (r, text) in segments.iter().zip(new) {
        if r.start < at
            || r.end < r.start
            || r.end > body.len()
            || !body.is_char_boundary(r.start)
            || !body.is_char_boundary(r.end)
        {
            return Err(format!("segment {r:?} does not fit the body"));
        }
        out.push_str(&body[at..r.start]);
        // The original segment's own leading and trailing whitespace (a
        // selection may have some; a segment of `own_segments` has none).
        let old = &body[r.clone()];
        let lead = &old[..old.len() - old.trim_start().len()];
        let trail = &old[old.trim_end().len()..];
        out.push_str(lead);
        out.push_str(text.trim());
        out.push_str(trail);
        at = r.end;
    }
    out.push_str(&body[at..]);
    Ok(out)
}

#[cfg(test)]
// One range in an array is what `splice` takes for one selection, not a
// mistaken `0..n` meant as a list of numbers, which this lint guards against.
#[allow(clippy::single_range_in_vec_init)]
mod tests {
    use super::*;

    /// The segments' texts.
    fn texts(body: &str) -> Vec<&str> {
        own_segments(body).into_iter().map(|r| &body[r]).collect()
    }

    #[test]
    fn a_reply_above_its_quotation() {
        let body = "Bonjour Alice,\n\nC'est noté.\n\nLe 7 nov. 2026 à 10:12, Alice <a@x.fr> a écrit :\n> Tu viens ?\n>\n> A.\n";
        assert_eq!(texts(body), ["Bonjour Alice,\n\nC'est noté."]);
    }

    #[test]
    fn the_initial_body_of_a_reply_has_no_segment() {
        // `initial_body`: two empty lines, then the header and the quotation.
        assert_eq!(
            texts("\n\nOn Nov 7, 2026, at 10:12, Alice wrote:\n> Hi\n"),
            Vec::<&str>::new()
        );
    }

    #[test]
    fn answers_between_quoted_lines_are_separate_segments() {
        let body = "On Nov 7, Alice wrote:\n> Thursday?\nYes, at 2pm.\n> And the keys?\nI'll bring them.\n";
        assert_eq!(texts(body), ["Yes, at 2pm.", "I'll bring them."]);
    }

    #[test]
    fn a_signature_is_left_out_until_the_next_quotation() {
        let body = "See you.\n\n-- \nAchille\n+33 6 12 34 56 78\n\nOn Nov 7, Alice wrote:\n> Hi\n";
        assert_eq!(texts(body), ["See you."]);
        // An editor that dropped the space.
        assert_eq!(texts("See you.\n--\nAchille\n"), ["See you."]);
        // Text after a quotation that follows a signature is the user's again.
        assert_eq!(texts("A\n-- \nSig\n> q\nB\n"), ["A", "B"]);
    }

    #[test]
    fn a_forwarded_mail_is_left_out_in_any_language() {
        for sep in [
            "---------- Forwarded message ---------",
            "---------- Message transféré ---------",
        ] {
            let body = format!(
                "Pour info.\n\n{sep}\nFrom: Alice\nSubject: Plan\n\nThe plan is…\n> not a quote of ours\nMore text.\n"
            );
            assert_eq!(texts(&body), ["Pour info."], "{sep}");
        }
        // Dashes alone are not a forward: a line the user drew.
        assert_eq!(
            texts("Top\n----------\nBottom\n"),
            ["Top\n----------\nBottom"]
        );
    }

    #[test]
    fn an_intro_line_needs_a_quotation_right_after() {
        // A line ending with ':' followed by the user's own text stays theirs.
        assert_eq!(
            texts("Here is the plan:\n- one\n- two\n"),
            ["Here is the plan:\n- one\n- two"]
        );
    }

    #[test]
    fn a_new_mail_is_one_segment() {
        assert_eq!(
            texts("Hello,\n\nI have a question.\n\nThanks"),
            ["Hello,\n\nI have a question.\n\nThanks"]
        );
        assert_eq!(texts(""), Vec::<&str>::new());
        assert_eq!(texts("  \n\n "), Vec::<&str>::new());
    }

    #[test]
    fn windows_line_ends_are_understood() {
        let body = "Yes.\r\nOn Nov 7, Alice wrote:\r\n> Hi\r\n";
        assert_eq!(texts(body), ["Yes."]);
    }

    #[test]
    fn splice_changes_only_the_segments() {
        let body = "Bonjour Alice,\n\nc'est noter.\n\nLe 7 nov., Alice a écrit :\n> Tu viens ?\nOui, a 14h.\n\n-- \nAchille\n";
        let segs = own_segments(body);
        assert_eq!(segs.len(), 2);
        let new = vec![
            "  Bonjour Alice,\n\nC'est noté.\n".to_string(),
            "Oui, à 14h.".to_string(),
        ];
        let out = splice(body, &segs, &new).unwrap();
        assert_eq!(
            out,
            "Bonjour Alice,\n\nC'est noté.\n\nLe 7 nov., Alice a écrit :\n> Tu viens ?\nOui, à 14h.\n\n-- \nAchille\n"
        );
    }

    #[test]
    fn splice_with_unchanged_texts_gives_the_body_back_byte_for_byte() {
        let bodies = [
            "Bonjour,\n\nOK.\n\nOn Nov 7, Alice wrote:\n> Hi\n>\n>> older\n",
            "A\n> q\n  B  \n\n> q2\n\nC\n-- \nSig\n",
            "Pour info.\n\n---------- Forwarded message ---------\nFrom: x\n\nBody\n",
            "été, ça va ? 🌞\n> déjà\n",
        ];
        for body in bodies {
            let segs = own_segments(body);
            let same: Vec<String> = segs.iter().map(|r| body[r.clone()].to_string()).collect();
            assert_eq!(splice(body, &segs, &same).unwrap(), body, "{body:?}");
        }
    }

    #[test]
    fn a_selection_keeps_its_own_whitespace() {
        let body = "One two three.";
        // The user selected " two " with its spaces.
        let out = splice(body, &[3..8], &["deux".to_string()]).unwrap();
        assert_eq!(out, "One deux three.");
    }

    /// Bodies built at random from every kind of line: whatever the mix,
    /// unchanged segments give the body back, and no segment holds a quoted
    /// line, a header, a signature or a forwarded mail.
    #[test]
    fn random_bodies_keep_their_invariants() {
        const PIECES: &[&str] = &[
            "Bonjour,",
            "",
            "  ",
            "Merci, à jeudi 14h.",
            "> quoted",
            ">> older",
            ">",
            "On Nov 7, Alice wrote:",
            "Le 7 nov. 2026 à 10:12, Alice <a@x.fr> a écrit :",
            "-- ",
            "--",
            "Achille",
            "---------- Forwarded message ---------",
            "Here is the list:",
            "- item 🌞",
            "été\r",
        ];
        // A small linear congruential generator: random enough to mix the
        // pieces, and the same sequence at every run, so a failure repeats.
        let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
        let mut next = move |n: usize| {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((seed >> 33) as usize) % n
        };
        for _ in 0..3_000 {
            let count = next(12);
            let mut body = String::new();
            for _ in 0..count {
                body.push_str(PIECES[next(PIECES.len())]);
                body.push('\n');
            }
            if next(2) == 0 {
                body.pop(); // a body that does not end with a line break
            }
            let segs = own_segments(&body);
            let same: Vec<String> = segs.iter().map(|r| body[r.clone()].to_string()).collect();
            assert_eq!(splice(&body, &segs, &same).unwrap(), body, "{body:?}");
            let forwarded_at = body.find("---------- Forwarded").unwrap_or(body.len());
            for r in &segs {
                let s = &body[r.clone()];
                assert!(!s.is_empty() && s.trim() == s, "{body:?}: {s:?}");
                assert!(
                    r.end <= forwarded_at,
                    "{body:?}: {s:?} is in the forwarded mail"
                );
                for line in s.lines() {
                    assert!(!line.starts_with('>'), "{body:?}: quoted line in {s:?}");
                    assert!(
                        line != "-- " && line != "--",
                        "{body:?}: signature in {s:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn splice_refuses_what_does_not_fit() {
        assert!(splice("abc", &[0..1], &[]).is_err());
        assert!(splice("abc", &[0..9], &["x".into()]).is_err());
        assert!(splice("abc", &[2..3, 0..1], &["x".into(), "y".into()]).is_err());
        // Inside a multi-byte character.
        assert!(splice("é", &[0..1], &["x".into()]).is_err());
    }
}
