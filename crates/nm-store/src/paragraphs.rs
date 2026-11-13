//! Paragraphs of a text body made whole again, for the text the app copies
//! to the clipboard (`ui.md`, menu `s`, "c to clipboard"). Pasted into a
//! document, a note or a ticket, a mail's text must give one line per
//! paragraph, not the line breaks its sender's mail app put every 72
//! characters.
//!
//! Where the extra line breaks come from:
//! - **format=flowed** (RFC 3676, Apple Mail, Thunderbird): a long line is
//!   cut after a space, and the space stays at the end of the piece; a
//!   reader that knows the format joins the pieces. The stored text keeps
//!   the pieces (mail-parser does not decode the format).
//! - **hard wrapping** (mutt, mailing lists, older clients): lines are cut
//!   at a fixed width, usually 72 to 78 characters, with nothing to say so.
//!   The width is guessed from the text itself: in a wrapped text, many
//!   lines end close to the same width, and a line was cut there because
//!   the next word did not fit.
//! - **empty lines piled up** by HTML conversions (`<br><br>`, paragraphs
//!   holding only `&nbsp;` or invisible characters).
//!
//! What is never joined: lines of different quote depths (`>`), the lines
//! of a signature (after `-- `), list items, indented lines (code, poems,
//! addresses), lines with tabs (tables), short lines (a greeting, a
//! closing, a name), and the lines of a block where every line ends a
//! sentence (one sentence per line, as people write a list without
//! bullets: a wrapping tool would not end every line on a full stop). A line that does not look cut is left alone: being
//! wrong there glues two paragraphs together, which is worse than leaving
//! a line break the user can remove.
//!
//! The function is pure and works line by line, in one pass to measure the
//! widths and one pass to join: linear in the size of the text.

use std::collections::{BTreeMap, BTreeSet};

/// Longest line (in characters, quote prefix excluded) taken into account
/// to guess the wrapping width: wrapping tools cut at 80 at most. Longer
/// lines (a URL, an unwrapped paragraph) say the text is not wrapped.
const MAX_WRAP: usize = 80;

/// Narrowest width taken for a wrapped text. Below, the "widest" lines are
/// short lines (a greeting, a list), not lines cut by a tool.
const MIN_WRAP: usize = 60;

/// How far below the width a line still counts as "full" when deciding if
/// a text is wrapped: the last word of a cut line is rarely longer.
const NEAR_WRAP: usize = 12;

/// A line ending with a space (a format=flowed soft break) is joined to the
/// next one only from this length: a short line with a stray space at its
/// end ("Cordialement, ") is a line of its own.
const FLOWED_MIN: usize = 40;

/// Characters that show nothing, which newsletters put in otherwise empty
/// paragraphs. A line holding only these (and whitespace, no-break spaces
/// included) is an empty line. They are not removed from the text: the
/// zero-width joiner, for one, holds emoji sequences together.
const INVISIBLE: &[char] = &['\u{200b}', '\u{200c}', '\u{200d}', '\u{2060}', '\u{feff}', '\u{34f}', '\u{ad}'];

/// One line of the text, split into its quote prefix and its content.
///
/// Rust note: `'a` is a lifetime: the `&'a str` fields borrow slices of
/// the text given to `unwrap`, without copying them. The compiler checks
/// that no `Line` outlives that text.
struct Line<'a> {
    /// The whole line, as written.
    line: &'a str,
    /// The quote marks as written (`"> "`, `">> "`, `"> > "`), kept as is
    /// in the output.
    prefix: &'a str,
    /// Number of `>`: lines of different depths are different texts.
    depth: usize,
    content: &'a str,
}

impl<'a> Line<'a> {
    fn new(line: &'a str) -> Self {
        let bytes = line.as_bytes();
        let (mut depth, mut end) = (0, 0);
        loop {
            // "> > text": a single space may separate two quote marks.
            let mut j = end;
            if depth > 0 && bytes.get(j) == Some(&b' ') {
                j += 1;
            }
            if bytes.get(j) == Some(&b'>') {
                depth += 1;
                end = j + 1;
            } else {
                break;
            }
        }
        // The space after the last mark belongs to the prefix.
        if depth > 0 && bytes.get(end) == Some(&b' ') {
            end += 1;
        }
        // Rust note: slicing a `&str` at a byte index is only allowed on a
        // character boundary; `>` and space are one byte each, so `end` is
        // always one.
        Line { line, prefix: &line[..end], depth, content: &line[end..] }
    }

    /// The line as written, prefix included, without trailing spaces.
    fn whole(&self) -> &'a str {
        self.line.trim_end()
    }

    /// Ends with the end of a sentence (or of a clause: `:`, `;`), a
    /// closing quote or parenthesis allowed after it.
    fn ends_sentence(&self) -> bool {
        let s = self.content.trim_end().trim_end_matches(['"', '\'', '»', '”', ')', ' ', '\u{a0}']);
        s.ends_with(['.', '!', '?', ':', ';', '…'])
    }

    fn is_blank(&self) -> bool {
        self.content.chars().all(|c| c.is_whitespace() || INVISIBLE.contains(&c))
    }

    /// `-- ` (or `--`, when a tool has trimmed the space): what follows is
    /// a signature.
    fn is_signature_mark(&self) -> bool {
        self.content.trim_end() == "--"
    }

    /// Indented with a tab or two spaces: code, a poem, an address block.
    /// One space is not enough: format=flowed adds one before a line that
    /// starts with a space, `>` or "From ".
    fn is_indented(&self) -> bool {
        self.content.starts_with('\t') || self.content.starts_with("  ")
    }

    /// Starts a list item: a bullet, or a number followed by `.` or `)`.
    fn is_list_item(&self) -> bool {
        let s = self.content.trim_start();
        if ["- ", "* ", "+ ", "• ", "– ", "— ", "· "].iter().any(|b| s.starts_with(b)) {
            return true;
        }
        let digits = s.bytes().take_while(u8::is_ascii_digit).count();
        (1..=3).contains(&digits) && matches!(s.as_bytes().get(digits), Some(b'.' | b')'))
            && matches!(s.as_bytes().get(digits + 1), Some(b' ') | None)
    }
}

/// What the lines of one quote depth say about its wrapping.
#[derive(Default)]
struct Width {
    /// The widest line of at most `MAX_WRAP` characters, in characters…
    chars: usize,
    /// …and in bytes: some tools count bytes, so a line with accents is
    /// cut sooner than its characters say.
    bytes: usize,
    /// Lines longer than `MAX_WRAP`.
    long: usize,
    /// Lines within `NEAR_WRAP` of `chars`.
    near: usize,
}

impl Width {
    /// Wrapped: the widest lines are wide enough for a tool's width, at
    /// least two lines end near it, and lines too long for any tool are
    /// fewer (a URL or two in a wrapped text, not a text of long lines).
    fn wrapped(&self) -> bool {
        self.chars >= MIN_WRAP && self.near >= 2 && self.long < self.near
    }
}

fn char_len(s: &str) -> usize {
    s.chars().count()
}

/// How the text is wrapped: per quote depth, and as a whole.
struct Widths {
    /// Each depth on its own, contents measured without their prefix: the
    /// replier's text and the quoted one may come from different tools.
    depth: BTreeMap<usize, Width>,
    /// All the lines together, measured with their prefix: for a depth
    /// with too few lines to say (two quoted lines), the width of the
    /// whole text is the best guess. The quoting app rewraps quotes at its
    /// own width prefix included, hence the whole line.
    whole: Width,
}

/// Measures the wrapping of the text: two passes, one for the widest
/// lines, one for the lines near them.
fn widths(lines: &[Line]) -> Widths {
    let mut depth: BTreeMap<usize, Width> = BTreeMap::new();
    let mut whole = Width::default();
    let measure = |w: &mut Width, s: &str| {
        let n = char_len(s);
        if n > MAX_WRAP {
            w.long += 1;
        } else {
            w.chars = w.chars.max(n);
            w.bytes = w.bytes.max(s.len());
        }
    };
    let count_near = |w: &mut Width, s: &str| {
        let n = char_len(s);
        if n <= MAX_WRAP && n + NEAR_WRAP >= w.chars {
            w.near += 1;
        }
    };
    let text_lines = || lines.iter().filter(|l| !l.is_blank());
    for l in text_lines() {
        measure(depth.entry(l.depth).or_default(), l.content.trim_end());
        measure(&mut whole, l.whole());
    }
    for l in text_lines() {
        count_near(depth.get_mut(&l.depth).expect("measured above"), l.content.trim_end());
        count_near(&mut whole, l.whole());
    }
    Widths { depth, whole }
}

/// For each line, whether it belongs to a block (lines one after the
/// other, at the same depth, no empty line between) of at least two lines
/// that all end a sentence. Such a block is kept line for line; only a
/// format=flowed soft break could join its lines, and a flowed piece ends
/// with a space, not a full stop.
fn sentence_blocks(lines: &[Line]) -> Vec<bool> {
    let mut out = vec![false; lines.len()];
    let mut start = 0;
    while start < lines.len() {
        // `end`: one past the last line of the block starting at `start`.
        let mut end = start + 1;
        if !lines[start].is_blank() {
            while end < lines.len() && !lines[end].is_blank() && lines[end].depth == lines[start].depth {
                end += 1;
            }
            let block = &lines[start..end];
            if block.len() >= 2 && block.iter().all(Line::ends_sentence) {
                out[start..end].fill(true);
            }
        }
        start = end;
    }
    out
}

/// Whether the break between `a` and the next line `b` is a soft one,
/// left by a tool rather than by the writer.
fn soft_break(a: &Line, b: &Line, widths: &Widths) -> bool {
    if a.depth != b.depth || a.is_blank() || b.is_blank() {
        return false;
    }
    if a.is_signature_mark() || b.is_signature_mark() || a.is_indented() || b.is_indented() || b.is_list_item() {
        return false;
    }
    if a.content.contains('\t') || b.content.contains('\t') {
        return false;
    }
    // format=flowed: the trailing space says it, on a line long enough not
    // to be a stray space.
    if a.content.ends_with(' ') && char_len(a.content.trim_end()) >= FLOWED_MIN {
        return true;
    }
    // The depth's own width; failing that, the width of the whole text,
    // unless the depth has lines too long for any tool (it is not wrapped).
    let own = widths.depth.get(&a.depth);
    let (w, text) = match own {
        Some(w) if w.wrapped() => (w, a.content.trim_end()),
        Some(w) if w.long == 0 && widths.whole.wrapped() => (&widths.whole, a.whole()),
        _ => return false,
    };
    let n = char_len(text);
    if n > w.chars {
        // Longer than the width: a word too long to cut (a URL) that the
        // tool put on a line of its own, then went on on the next line.
        return !text.trim_start().contains(' ');
    }
    // A short line was ended by its writer, whatever comes next.
    if n * 2 < w.chars {
        return false;
    }
    let Some(word) = b.content.split_whitespace().next() else { return false };
    // Cut because the next word did not fit, in characters or in bytes.
    n + 1 + char_len(word) > w.chars || text.len() + 1 + word.len() > w.bytes
}

/// The text with its paragraphs joined back, each on one line, and its
/// empty lines reduced to one between paragraphs, none at either end.
/// Line endings become `\n`; trailing spaces go.
pub fn unwrap(text: &str) -> String {
    // A text from a raw part may still have CRLF, or lone CR.
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<Line> = text.split('\n').map(Line::new).collect();
    let widths = widths(&lines);
    let sentences = sentence_blocks(&lines);

    // The output lines: an empty line, with its quote depth, or a line of
    // text. Built first, written out at the end, so that runs of empty
    // lines can be reduced.
    enum Out {
        Blank { prefix: String, depth: usize },
        Text(String),
    }
    let mut out: Vec<Out> = Vec::new();
    // Depths that have reached their signature: its lines stay as written.
    let mut signed: BTreeSet<usize> = BTreeSet::new();
    let mut current: Option<String> = None;

    for (i, line) in lines.iter().enumerate() {
        if line.is_signature_mark() {
            signed.insert(line.depth);
        }
        let joined = i > 0
            && current.is_some()
            && !signed.contains(&line.depth)
            && !sentences[i]
            && soft_break(&lines[i - 1], line, &widths);
        if joined {
            // Rust note: `as_mut()` borrows the `String` inside the
            // `Option` mutably, without taking it out.
            let s = current.as_mut().expect("checked above");
            s.truncate(s.trim_end().len());
            s.push(' ');
            s.push_str(line.content.trim());
            continue;
        }
        if let Some(s) = current.take() {
            out.push(Out::Text(s));
        }
        if line.is_blank() {
            let prefix = line.prefix.trim_end().to_string();
            match out.last_mut() {
                // A run of empty lines is one; between a quote and the
                // reply, the shallower one stays.
                Some(Out::Blank { prefix: p, depth: d }) => {
                    if line.depth < *d {
                        (*p, *d) = (prefix, line.depth);
                    }
                }
                // None at the start.
                None => {}
                Some(Out::Text(_)) => out.push(Out::Blank { prefix, depth: line.depth }),
            }
        } else {
            let mut s = String::with_capacity(line.prefix.len() + line.content.len());
            s.push_str(line.prefix);
            s.push_str(line.content.trim_end());
            current = Some(s);
        }
    }
    if let Some(s) = current.take() {
        out.push(Out::Text(s));
    }
    // None at the end.
    while matches!(out.last(), Some(Out::Blank { .. })) {
        out.pop();
    }

    let mut result = String::with_capacity(text.len());
    for (i, o) in out.iter().enumerate() {
        if i > 0 {
            result.push('\n');
        }
        match o {
            Out::Blank { prefix, .. } => result.push_str(prefix),
            Out::Text(s) => result.push_str(s),
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::unwrap;

    /// A French text wrapped at 72 characters, as mutt or a mailing list
    /// would send it.
    const WRAPPED: &str = "\
Bonjour Jean,

Suite à notre échange de mardi, je vous envoie le devis pour la réfection
de la toiture du garage. Les travaux pourraient commencer la semaine du
17 novembre si la météo le permet, et dureraient environ trois jours.

Merci de me confirmer votre accord avant vendredi, afin que je puisse
réserver l'échafaudage.

Cordialement,
Pierre Martin";

    #[test]
    fn joins_the_lines_of_a_wrapped_paragraph_and_keeps_short_lines() {
        assert_eq!(
            unwrap(WRAPPED),
            "Bonjour Jean,\n\n\
             Suite à notre échange de mardi, je vous envoie le devis pour la réfection de la toiture du garage. \
             Les travaux pourraient commencer la semaine du 17 novembre si la météo le permet, et dureraient \
             environ trois jours.\n\n\
             Merci de me confirmer votre accord avant vendredi, afin que je puisse réserver l'échafaudage.\n\n\
             Cordialement,\nPierre Martin"
        );
    }

    #[test]
    fn joins_format_flowed_pieces() {
        let flowed = "Voici le compte rendu de la réunion de ce matin, avec les \r\n\
                      décisions prises et les points restant à trancher.\r\n\r\nBonne journée,\r\nAnne";
        assert_eq!(
            unwrap(flowed),
            "Voici le compte rendu de la réunion de ce matin, avec les décisions prises et les points \
             restant à trancher.\n\nBonne journée,\nAnne"
        );
    }

    #[test]
    fn a_short_line_with_a_stray_space_stays_a_line() {
        assert_eq!(unwrap("Cordialement, \nPierre Martin"), "Cordialement,\nPierre Martin");
    }

    #[test]
    fn leaves_a_text_that_is_not_wrapped_alone() {
        // One paragraph per line, some longer than any wrapping width, and
        // lines one after the other: nothing is cut, nothing is joined.
        let text = "Bonjour,\n\
            Je vous confirme le rendez-vous de jeudi à 14 h dans nos locaux de la rue de la République, au troisième étage.\n\
            Merci de vous présenter à l'accueil avec une pièce d'identité, le badge vous sera remis sur place.\n\
            Le parking visiteurs se trouve juste derrière.\n\
            À jeudi,\nClaire";
        assert_eq!(unwrap(text), text);
    }

    #[test]
    fn joins_quoted_lines_within_their_depth_and_keeps_the_prefix() {
        let text = "\
D'accord pour vendredi, je préviens l'équipe et je réserve la salle du
deuxième étage pour toute la matinée.

Le 12 nov. 2026 à 10:04, Pierre Martin <pierre@exemple.fr> a écrit :
> Pourriez-vous me dire si vendredi matin vous conviendrait pour la
> présentation du projet ? Il faudrait prévoir une heure et demie, avec
> le temps des questions.
>
>> Nous avions parlé d'une présentation avant la fin du mois, mais je
>> n'ai pas encore de date ferme de mon côté.";
        assert_eq!(
            unwrap(text),
            "D'accord pour vendredi, je préviens l'équipe et je réserve la salle du deuxième étage pour toute \
             la matinée.\n\n\
             Le 12 nov. 2026 à 10:04, Pierre Martin <pierre@exemple.fr> a écrit :\n\
             > Pourriez-vous me dire si vendredi matin vous conviendrait pour la présentation du projet ? \
             Il faudrait prévoir une heure et demie, avec le temps des questions.\n\
             >\n\
             >> Nous avions parlé d'une présentation avant la fin du mois, mais je n'ai pas encore de date \
             ferme de mon côté."
        );
    }

    #[test]
    fn keeps_lists_indented_lines_tables_and_the_signature() {
        let text = "\
Pour la réunion de lundi, voici les points que je propose d'aborder dans
l'ordre, en commençant par ceux qui demandent une décision rapide :
- le budget du deuxième semestre et la répartition entre les deux équipes
- le calendrier des recrutements prévus avant la fin de l'année civile
1. valider le budget
2) fixer les dates
    fn main() { println!(\"code indenté que personne ne veut voir recollé\"); }
    // une deuxième ligne de code indentée elle aussi, à garder telle quelle
Poste\tBudget\tÉcart constaté sur le trimestre précédent et prévisions
Total\t12 000\t+3 %
--
Pierre Martin, responsable des travaux et de la coordination des équipes
12 rue des Lilas, 69003 Lyon — téléphone 04 72 00 00 00, du lundi au jeudi";
        let out = unwrap(text);
        assert!(out.starts_with(
            "Pour la réunion de lundi, voici les points que je propose d'aborder dans l'ordre, en commençant \
             par ceux qui demandent une décision rapide :\n- le budget"
        ));
        // Everything after the first paragraph stays line for line (but
        // the signature mark loses its trailing space).
        let rest: Vec<&str> = text.lines().skip(2).map(str::trim_end).collect();
        assert_eq!(out.lines().skip(1).collect::<Vec<_>>(), rest);
    }

    #[test]
    fn collapses_empty_lines_and_trims_both_ends() {
        let text = "\n\u{a0}\nBonjour,\n\n\n \u{200b}\n\t\nÀ demain.\n\n\n";
        assert_eq!(unwrap(text), "Bonjour,\n\nÀ demain.");
    }

    #[test]
    fn an_empty_line_between_a_quote_and_the_reply_is_the_shallower_one() {
        assert_eq!(unwrap("> Une question ?\n>\n\nUne réponse."), "> Une question ?\n\nUne réponse.");
        assert_eq!(unwrap(""), "");
        assert_eq!(unwrap("\n\n"), "");
    }

    #[test]
    fn a_line_cut_by_a_tool_counting_bytes_is_joined() {
        // Cut at 72 bytes: the accents make the first line 66 characters
        // long, too short for a 72-character width, but 72 bytes.
        let text = "\
Le comité a décidé de reporter l'étude des créations à l'été prochain,
après le vote du budget par le conseil d'administration de janvier 2027.
Les élus réunis en séance plénière ont été informés de cette décision très
rapidement par un courrier du président daté du 3 novembre dernier.";
        assert_eq!(unwrap(text).lines().count(), 1);
    }

    #[test]
    fn a_long_url_on_its_own_line_is_part_of_the_paragraph() {
        let text = "\
Vous trouverez le document complet à l'adresse suivante, valable un mois
https://partage.exemple.fr/documents/2026/11/compte-rendu-reunion-de-chantier-numero-12.pdf
et les plans sur le même site, dans le dossier du chantier, à partir de
lundi prochain au plus tard.";
        assert_eq!(unwrap(text).lines().count(), 1);
    }

    #[test]
    fn a_block_of_full_lines_that_all_end_a_sentence_stays_line_for_line() {
        // Each line as long as a wrapped one, but each one a sentence: one
        // per line, as written.
        let text = "\
Ligne 1 du long mail, pour tester le défilement du panneau de lecture.
Ligne 2 du long mail, pour tester le défilement du panneau de lecture.
Ligne 3 du long mail, pour tester le défilement du panneau de lecture.";
        assert_eq!(unwrap(text), text);
        // One line not ending a sentence: the block is prose, wrapped.
        let prose = format!("{text}\nEt une dernière ligne, coupée au milieu de la phrase comme le ferait\nun outil.");
        assert_eq!(unwrap(&prose).lines().count(), 1);
    }

    #[test]
    fn a_line_ended_before_the_width_is_kept_apart() {
        // The second line ends at 43 characters although "Nous" would have
        // fit in the 72-character width: its writer ended it.
        let text = "\
Je vous remercie pour votre retour sur le devis de la toiture du garage,
que nous avons étudié avec attention lundi.
Nous acceptons votre proposition sans réserve et vous laissons fixer la
date du début des travaux.";
        assert_eq!(
            unwrap(text),
            "Je vous remercie pour votre retour sur le devis de la toiture du garage, que nous avons étudié \
             avec attention lundi.\n\
             Nous acceptons votre proposition sans réserve et vous laissons fixer la date du début des travaux."
        );
    }
}
