//! A word-level diff between a text sent and the text the AI gave back, for
//! the preview of a `replace` (`specifications/ia.md`, section 11, "Panneau
//! IA": removed words struck out, added words underlined).
//!
//! Computed in the core, so that every client shows the same differences
//! (`architecture.md`, principle 3: no logic in Swift).

/// One piece of the diff. The pieces of `Same` and `Removed` rebuild the
/// old text, those of `Same` and `Added` the new one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece {
    Same(String),
    Removed(String),
    Added(String),
}

/// Above this many word pairs, the texts are not compared word by word
/// (the table grows as the product of their lengths): the whole old text is
/// shown removed and the new one added. 4 million pairs is two texts of
/// 2,000 words, far above a mail, and about 16 MB of table.
const MAX_PAIRS: usize = 4_000_000;

/// Words with the whitespace that follows them, so that the pieces join
/// back into the text exactly.
fn tokens(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut in_space = false;
    for (i, c) in s.char_indices() {
        if c.is_whitespace() {
            in_space = true;
        } else if in_space {
            out.push(&s[start..i]);
            start = i;
            in_space = false;
        }
    }
    if start < s.len() {
        out.push(&s[start..]);
    }
    out
}

/// The diff of `old` and `new`, word by word. Two words are the same when
/// they differ only in the whitespace after them; the new text's spacing is
/// kept.
pub fn words(old: &str, new: &str) -> Vec<Piece> {
    let a = tokens(old);
    let b = tokens(new);
    let (n, m) = (a.len(), b.len());
    let mut out = Vec::new();
    if n.saturating_mul(m) > MAX_PAIRS {
        if !old.is_empty() {
            out.push(Piece::Removed(old.to_string()));
        }
        if !new.is_empty() {
            out.push(Piece::Added(new.to_string()));
        }
        return out;
    }
    let ka: Vec<&str> = a.iter().map(|t| t.trim_end()).collect();
    let kb: Vec<&str> = b.iter().map(|t| t.trim_end()).collect();
    // lcs[i][j]: length of the longest common subsequence of a[i..], b[j..].
    // One flat vector rather than a vector of vectors: one allocation.
    let w = m + 1;
    let mut lcs = vec![0u32; (n + 1) * w];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i * w + j] = if ka[i] == kb[j] {
                lcs[(i + 1) * w + j + 1] + 1
            } else {
                lcs[(i + 1) * w + j].max(lcs[i * w + j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut push = |p: Piece| match (out.last_mut(), p) {
        // Neighbouring pieces of the same kind are merged.
        (Some(Piece::Same(s)), Piece::Same(t))
        | (Some(Piece::Removed(s)), Piece::Removed(t))
        | (Some(Piece::Added(s)), Piece::Added(t)) => s.push_str(&t),
        (_, p) => out.push(p),
    };
    // A common word takes the new text's spacing. When that is none (the
    // last word of the new text) while the old word had some, words removed
    // right after it would touch it ("a~~b~~"): the old spacing goes in
    // front of them instead.
    let mut owed = "";
    while i < n || j < m {
        if i < n && j < m && ka[i] == kb[j] {
            push(Piece::Same(b[j].to_string()));
            owed = if b[j].len() == kb[j].len() {
                &a[i][ka[i].len()..]
            } else {
                ""
            };
            i += 1;
            j += 1;
        } else if i < n && (j == m || lcs[(i + 1) * w + j] >= lcs[i * w + j + 1]) {
            // Removed before added, at equal cost: "~~je~~ Je" reads better
            // than "Je ~~je~~".
            push(Piece::Removed(format!("{owed}{}", a[i])));
            owed = "";
            i += 1;
        } else {
            push(Piece::Added(b[j].to_string()));
            owed = "";
            j += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rebuild(d: &[Piece], old: bool) -> String {
        d.iter()
            .filter_map(|p| match p {
                Piece::Same(s) => Some(s.as_str()),
                Piece::Removed(s) if old => Some(s.as_str()),
                Piece::Added(s) if !old => Some(s.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn changed_words_are_marked() {
        let d = words(
            "je te confirme le rendez vous a 14h",
            "Je te confirme le rendez-vous à 14h",
        );
        assert_eq!(
            d,
            [
                Piece::Removed("je ".into()),
                Piece::Added("Je ".into()),
                Piece::Same("te confirme le ".into()),
                Piece::Removed("rendez vous a ".into()),
                Piece::Added("rendez-vous à ".into()),
                Piece::Same("14h".into()),
            ]
        );
    }

    #[test]
    fn the_pieces_rebuild_both_texts() {
        // In the order removed then added, whatever the texts.
        for (old, new) in [
            ("teh meeting is on monday", "The meeting is on Monday."),
            ("", "Hello"),
            ("Hello", ""),
            ("été\n\nça va", "Été,\n\nça va ?"),
            ("a b c", "c b a"),
        ] {
            let d = words(old, new);
            assert_eq!(rebuild(&d, false), new, "{old:?} → {new:?}");
            // The old text, up to the spacing of words kept (the new one's).
            assert_eq!(
                rebuild(&d, true).split_whitespace().collect::<Vec<_>>(),
                old.split_whitespace().collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn spacing_alone_is_not_a_change() {
        assert_eq!(words("a  b", "a b"), [Piece::Same("a b".into())]);
        assert_eq!(words("same", "same"), [Piece::Same("same".into())]);
    }

    #[test]
    fn huge_texts_are_replaced_whole() {
        let old = "w ".repeat(2_100);
        let new = "x ".repeat(2_100);
        assert_eq!(
            words(&old, &new),
            [Piece::Removed(old.clone()), Piece::Added(new.clone())]
        );
    }
}
