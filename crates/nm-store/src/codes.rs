//! Verification codes: the one-time code a site or an app mails to confirm
//! a login or an address ("Votre code de vérification : 610731"), offered
//! first in the `l` panel, where Return copies it (`ui.md`, "Codes de
//! vérification").
//!
//! The rules were drawn from real code mails (banks, administrations,
//! shops, clouds, in French and English), then checked on a mailbox of
//! 400,000 messages: about 570 codes found, and the false ones traced back
//! to a rule each, none to a sender. The code is a word of its own:
//!
//! - 4 to 8 digits (`3280`, `610731`, `78355770`);
//! - 6 to 8 capital letters and digits, with at least one of each
//!   (`4T39TT`, `VSM6WQZ7`), but not a word and a number (`BOOMER30`,
//!   `NEWS1120`: promotion codes);
//! - 6 to 8 capitals without a vowel (`PGXFGG`);
//! - two groups of three, with a dash or a space (`904-426`, `65A-TQC`,
//!   `831 884`), copied without it.
//!
//! It is looked for only in a mail that speaks of a code to type: the word
//! "code" with, near it, a word of verification, confirmation, security,
//! login, one-time use… ("code de vérification", "authentication code",
//! "code à usage unique", "code à 6 chiffres"), or "OTP" / "passcode". A
//! "code promo", "code postal" or "gift code" does not count. Then, in this
//! order:
//!
//! 1. **the subject**, when it gives it (`Votre code ChatGPT : 797899`,
//!    `[Gandi] 772432 - Votre code à usage unique`): half of the mails do,
//!    and the subject holds nothing else that looks like a code;
//! 2. **a line of its own** in the body, near the word that announces it
//!    (most mails: the code is shown big, alone on its line); "near" is
//!    counted once runs of spaces are collapsed, since an HTML layout can
//!    put thousands of them between the sentence and the code;
//! 3. **right after the announcing word**, on the same line
//!    (`enter this verification code: 651440`, `… application : *G49DTC*`),
//!    then right before it (`036764 is your OTP code`). Only there may the
//!    code be glued to the text around it (`compte IKEA.677412Veiller`, a
//!    layout whose blocks lost their line breaks), and then it must have 6
//!    digits at least.
//!
//! A mail whose text part is empty (HTML only) is read from its HTML, and
//! so is one whose text part gives no code: some stop short of it.
//!
//! A reply or a forward ("RE:", "Fwd:") is not a code mail: a conversation
//! between people that speaks of codes holds references and numbers far
//! more often than a code to type.
//!
//! What is never a code: a word inside an address (the same code sits in
//! `…/2fa?code=G49DTC&…`), a number with dots, colons, dashes or slashes in
//! it (dates, times, IP addresses, phone numbers), a year (1900 to 2099:
//! a 4-digit code among them is missed, a footer's "© 2026" would be taken
//! far more often), and words of other lengths (an IBAN, a VAT number). As
//! for phone numbers (`phones.rs`), a false code in the panel is worse than
//! a missed one, which only costs a copy by hand.

/// Most characters read in a body: the code comes at the top of the mail,
/// and bodies can weigh megabytes. Counted after collapsing whitespace.
const MAX_CHARS: usize = 20_000;

/// Words that announce a code, and must be qualified (`QUALIFIERS`) to
/// make the mail a code mail.
const CODE_WORDS: &[&str] = &["code", "codes"];

/// Words that announce a code on their own.
const SELF_QUALIFIED: &[&str] = &["otp", "passcode", "one-time password", "mot de passe a usage unique"];

/// Words that announce a code shown nearby, once the mail is known to be a
/// code mail: the code words, and "password" (Levi's "saisir le mot de
/// passe ci-dessous").
const ANCHORS: &[&str] = &["code", "codes", "otp", "passcode", "mot de passe", "password"];

/// Beginnings of words that make a "code" a verification code, when one of
/// them is within `QUALIFIER_WINDOW` characters of it (folded: lowercase,
/// no accent). Prefixes: "verif" covers vérification, verification,
/// verify, vérifier. They start a word: "digit" alone would be found in
/// "communication digitale", hence "digits" and "digit code".
const QUALIFIERS: &[&str] = &[
    "verif", "confirm", "securit", "security", "authenti", "connexion", "connecter", "login", "log in", "sign in",
    "sign-in", "signin", "validation", "usage unique", "one-time", "one time", "temporaire", "temporary", "otp",
    "2fa", "chiffres", "digits", "digit code", "identite", "identity", "acceder", "access",
    // Getting an account back ("code de récupération", Facebook).
    "recuper", "reinitialis", "reset", "recovery",
    // Not "autorisation" / "approved": that is also the authorization
    // code of a card payment, on every receipt. Firefox's "code
    // d'autorisation" is found by its "Il expire dans 5 minutes".
    // Its short life, which nearly every code mail states: "Il expire
    // dans 5 minutes", "expires in 10 minutes"; "valable" too, but only
    // before a duration (`DURATION_WORDS`).
    "expir", "minute",
];

/// Words that qualify a "code" when a number follows them: "valable
/// 15min" (OVH), "valid for 10 minutes", not "Offre valable jusqu'au…",
/// which follows promotion codes.
const DURATION_WORDS: &[&str] = &["valable", "valide", "valid for"];

/// Characters around a "code" where a qualifier is looked for.
const QUALIFIER_WINDOW: usize = 40;

/// A "code" followed by one of these is another kind of code (folded).
const NOT_AFTER: &[&str] = &[
    "promo", "postal", "de reduction", "reduction", "de parrainage", "parrainage", "cadeau", "avantage", "remise",
    "client", "article", "produit", "naf", "ape", "source", "barre", "qr", "couleur",
    // A booking reference, an error, a bank's identifiers, the law
    // ("Code de la sécurité sociale" would be qualified by its "sécurité").
    // Not "code confidentiel": OVH sends its login codes so.
    "de reservation", "reservation", "dossier", "d'erreur", "erreur", "d'etablissement", "banque", "guichet", "html",
    "de la route", "du travail", "civil", "penal", "general", "de commerce", "de conduite", "de la consommation",
    "de la securite sociale", "des assurances", "monetaire", "de la sante", "de l'environnement", "electoral",
    "des impots", "de deontologie",
    // A shop's offer ("Votre code exclusif 60666 expire lundi").
    "exclusif", "offert", "offre", "de bienvenue", "bienvenue", "fidelite", "privilege", "vip", "surprise", "d'achat",
];

/// A "code" preceded by one of these is another kind of code (folded).
const NOT_BEFORE: &[&str] = &[
    "promo", "discount", "gift", "coupon", "referral", "zip", "postal", "source", "qr", "voucher", "bar",
    // A program's or a server's codes (a LogWatch report).
    "exit", "error", "status", "return", "response", "http", "country", "area",
];

/// Subject prefixes of a reply or a forward (folded). A code is sent by a
/// machine, alone: a conversation between people that speaks of codes
/// ("RE: Création code d'accès") holds references, amounts and addresses
/// that look like one, far more often than a code worth copying.
const REPLY_PREFIXES: &[&str] = &["re", "tr", "fw", "fwd", "aw", "wg", "sv", "rv", "ref"];

/// How far from an anchor a code alone on its line may be (rule 2).
const LINE_DISTANCE: usize = 300;
/// How far after an anchor a code on its line may be (rule 3).
const AFTER_WINDOW: usize = 120;
/// How far before an anchor (rule 3).
const BEFORE_WINDOW: usize = 40;

/// The verification code of a message, if it is a code mail and the code
/// is found: from its subject, then its text, else the text of its HTML.
pub fn find(subject: &str, text: Option<&str>, html: Option<&str>) -> Option<String> {
    let subject = Text::new(subject);
    if subject.is_reply() {
        return None;
    }
    // The HTML's text, made only when needed: converting a page costs more
    // than everything else here.
    let html_text = || html.map(|h| Text::new(&crate::html_text::html_to_text(h)));
    // The text part, unless it has no word at all (an HTML-only mail whose
    // text part is a couple of line breaks): then the HTML's text.
    let (body, from_html) = match text {
        Some(t) if t.chars().any(char::is_alphanumeric) => (Text::new(t), false),
        _ => (html_text().unwrap_or_else(|| Text::new("")), true),
    };
    let subject_says = subject.is_code_mail();
    if !subject_says && !body.is_code_mail() {
        return None;
    }
    // 1. The subject, when it announces the code itself.
    if !subject.anchors().is_empty() {
        let words: Vec<&[char]> = Text::words_of(&subject.orig).collect();
        if let Some(code) = (0..words.len()).find_map(|i| code_at(&words, i)) {
            return Some(code);
        }
    }
    // 2 and 3 in the body. A text part can stop short of the code that its
    // HTML shows (an ameli mail: "veuillez trouver ci-dessous votre code de
    // confirmation", and nothing below): then the HTML is read too.
    in_body(&body, subject_says).or_else(|| if from_html { None } else { html_text().and_then(|b| in_body(&b, subject_says)) })
}

/// Rules 2 and 3 in a body. With `all_anchors` (the subject says the mail
/// is about a code), any word that may announce a code is an anchor ("mot
/// de passe ci-dessous"); otherwise only those that say so themselves. A
/// newsletter that speaks of a "code de sécurité" once in its footer must
/// not lend its other "code" words to the numbers around them.
fn in_body(body: &Text, all_anchors: bool) -> Option<String> {
    let mut anchors = if all_anchors { body.anchors() } else { body.qualified_anchors() };
    // Without an announcing word in the body (the subject said it all and
    // the body shows the code alone), its top stands for one.
    if anchors.is_empty() {
        anchors.push((0, 0));
    }
    // 2. A line of its own, near an anchor: one word, or two groups of
    // three ("831 884").
    let mut start = 0;
    for line in body.orig[..].split(|c| *c == '\n') {
        let end = start + line.len();
        let near = anchors.iter().any(|&(a, a_end)| a.saturating_sub(LINE_DISTANCE) <= end && start <= a_end + LINE_DISTANCE);
        if near {
            let words: Vec<&[char]> = Text::words_of(line).collect();
            if (words.len() == 1 || words.len() == 2 && words.iter().all(|w| w.len() == 3))
                && let Some(code) = code_at(&words, 0)
            {
                return Some(code);
            }
        }
        start = end + 1;
    }
    // 3. Right after an anchor, then right before it, on its line.
    for &(_, a_end) in &anchors {
        let line_end = body.orig[a_end..].iter().position(|c| *c == '\n').map_or(body.orig.len(), |p| a_end + p);
        // The line after: "est le suivant :\n427242 Il est valable".
        let next_end = body.orig.get(line_end + 1..).and_then(|r| r.iter().position(|c| *c == '\n')).map_or(body.orig.len(), |p| line_end + 1 + p);
        let hi = next_end.min(a_end + AFTER_WINDOW);
        let words: Vec<&[char]> = Text::words_of(&body.orig[a_end..hi.max(a_end)]).collect();
        if let Some(i) = words.iter().position(|w| is_number(w))
            && let Some(code) = code_at(&words, i).or_else(|| glued_digits(words[i]))
        {
            return Some(code);
        }
    }
    for &(a, _) in &anchors {
        let line_start = body.orig[..a].iter().rposition(|c| *c == '\n').map_or(0, |p| p + 1);
        let words: Vec<&[char]> = Text::words_of(&body.orig[line_start.max(a.saturating_sub(BEFORE_WINDOW))..a]).collect();
        if let Some(i) = words.iter().rposition(|w| is_number(w))
            && let Some(code) = code_at(&words, i)
        {
            return Some(code);
        }
    }
    None
}

/// A text read once: its characters with runs of whitespace collapsed (a
/// newline if the run held one, else a space), as written (`orig`) and
/// folded (`folded`: lowercase, accents removed), one for one, so that a
/// position found in one is the same in the other.
struct Text {
    orig: Vec<char>,
    folded: Vec<char>,
}

impl Text {
    fn new(s: &str) -> Text {
        let mut orig = Vec::new();
        let mut folded = Vec::new();
        // `Option<char>`: the whitespace of the run being read, if any.
        let mut space: Option<char> = None;
        for c in s.chars() {
            if orig.len() >= MAX_CHARS {
                break;
            }
            if c.is_whitespace() {
                space = Some(if c == '\n' || space == Some('\n') { '\n' } else { ' ' });
                continue;
            }
            if let Some(sp) = space.take()
                && !orig.is_empty()
            {
                orig.push(sp);
                folded.push(sp);
            }
            orig.push(c);
            folded.push(fold(c));
        }
        Text { orig, folded }
    }

    /// Whether the text speaks of a code to type: a qualified code word,
    /// or a word that is one by itself.
    fn is_code_mail(&self) -> bool {
        !self.qualified_anchors().is_empty()
    }

    /// Whether the text, a subject, starts as a reply or a forward does
    /// ("RE: ", "Fwd : ", "TR:").
    fn is_reply(&self) -> bool {
        let word: String = self.folded.iter().take_while(|c| c.is_alphabetic()).collect();
        let rest = self.folded[word.chars().count()..].iter().find(|c| **c != ' ');
        rest == Some(&':') && REPLY_PREFIXES.contains(&word.as_str())
    }

    /// The anchors that make the mail a code mail: the code words
    /// qualified (`qualified`), and the words that are one by themselves.
    fn qualified_anchors(&self) -> Vec<(usize, usize)> {
        let mut all: Vec<(usize, usize)> = SELF_QUALIFIED.iter().flat_map(|w| self.occurrences(w)).collect();
        all.extend(CODE_WORDS.iter().flat_map(|w| self.occurrences(w)).filter(|&(s, e)| self.qualified(s, e)));
        all.sort_unstable();
        all.dedup();
        all
    }

    /// A code word at `start..end` that is another kind of code: "code
    /// promo", "code postal", "error code"…
    fn excluded(&self, start: usize, end: usize) -> bool {
        let after: String = self.folded[end..(end + 24).min(self.folded.len())].iter().collect();
        let after = after.trim_start();
        let before: String = self.folded[start.saturating_sub(16)..start].iter().collect();
        let before = before.trim_end();
        NOT_AFTER.iter().any(|w| after.starts_with(w)) || NOT_BEFORE.iter().any(|w| before.ends_with(w))
    }

    /// A code word at `start..end` that is a verification code's: no
    /// "promo" or "postal" around it, a qualifier near it.
    fn qualified(&self, start: usize, end: usize) -> bool {
        if self.excluded(start, end) {
            return false;
        }
        let window: String =
            self.folded[start.saturating_sub(QUALIFIER_WINDOW)..(end + QUALIFIER_WINDOW).min(self.folded.len())].iter().collect();
        // Rust note: `match_indices` gives each place a word is found (a
        // byte offset); the character before it must not be a letter or a
        // digit, so that the word starts there.
        let starts_word = |i: usize| window[..i].chars().next_back().is_none_or(|c| !c.is_alphanumeric());
        QUALIFIERS.iter().any(|q| window.match_indices(q).any(|(i, _)| starts_word(i)))
            || DURATION_WORDS.iter().any(|q| {
                window.match_indices(q).any(|(i, m)| {
                    starts_word(i) && window[i + m.len()..].trim_start().chars().next().is_some_and(|c| c.is_ascii_digit())
                })
            })
    }

    /// Positions of the anchors (`ANCHORS`) of the text, in order.
    fn anchors(&self) -> Vec<(usize, usize)> {
        let mut all: Vec<(usize, usize)> =
            ANCHORS.iter().flat_map(|w| self.occurrences(w)).filter(|&(s, e)| !self.excluded(s, e)).collect();
        all.sort_unstable();
        all.dedup();
        all
    }

    /// Where `word` (folded, possibly several words) stands as whole words.
    fn occurrences(&self, word: &str) -> Vec<(usize, usize)> {
        let w: Vec<char> = word.chars().collect();
        let f = &self.folded;
        if w.is_empty() || f.len() < w.len() {
            return vec![];
        }
        (0..=f.len() - w.len())
            .filter(|&i| {
                f[i..i + w.len()] == w[..]
                    && (i == 0 || !f[i - 1].is_alphanumeric())
                    && f.get(i + w.len()).is_none_or(|c| !c.is_alphanumeric())
                    // Not inside an address: `…&code_validation=…` in a
                    // tracking link says nothing of the text around it.
                    && !is_address(word_around(&self.orig, i, i + w.len()))
            })
            .map(|i| (i, i + w.len()))
            .collect()
    }

    /// The words of `chars` (runs between spaces and newlines), trimmed of
    /// the punctuation around them. A word that is part of an address
    /// (`https://…`, `x@y`) is left out.
    fn words_of(chars: &[char]) -> impl Iterator<Item = &[char]> {
        let mut i = 0;
        // Rust note: `std::iter::from_fn` makes an iterator of a closure,
        // called for each item until it returns `None`. `move` gives the
        // closure its own `i`, kept from one call to the next; the words it
        // returns borrow `chars`, which the signature's elided lifetime
        // ties to the iterator.
        std::iter::from_fn(move || {
            loop {
                while i < chars.len() && (chars[i] == ' ' || chars[i] == '\n') {
                    i += 1;
                }
                if i >= chars.len() {
                    return None;
                }
                let start = i;
                while i < chars.len() && chars[i] != ' ' && chars[i] != '\n' {
                    i += 1;
                }
                let word = &chars[start..i];
                if !is_address(word) {
                    return Some(trim(word));
                }
            }
        })
    }
}

/// A word that holds a number (two digits or more), or capitals without a
/// vowel: what may be a code. Rule 3 takes the first one after the
/// announcing word (the last one before it), and nothing if that one is
/// not a code: in "Codes de Paris vérifiés… MN6121/96-UBO comptés 1030",
/// the text speaks of other codes than one to type. A single digit ("le
/// code à 6 chiffres") is not a number of that kind.
fn is_number(word: &[char]) -> bool {
    word.iter().filter(|c| c.is_ascii_digit()).count() >= 2 || is_letter_code(word)
}

/// The code `words[i]` starts, if it is one: a code word (`as_code`), or
/// two groups of three ("831 884", Beeper), copied as one. Digits next to
/// other digits ("access code: 2641 801 9509", "Meeting ID: 948 4026
/// 0686") are a group of a longer number.
fn code_at(words: &[&[char]], i: usize) -> Option<String> {
    let digits = |w: &&[char]| !w.is_empty() && w.iter().all(char::is_ascii_digit);
    let w = words[i];
    let before = i.checked_sub(1).and_then(|j| words.get(j)).is_some_and(digits);
    if digits(&w) && w.len() == 3 && words.get(i + 1).is_some_and(|n| digits(n) && n.len() == 3) {
        let after = words.get(i + 2).is_some_and(digits);
        return if before || after { None } else { as_code(&[w, words[i + 1]].concat()) };
    }
    if digits(&w) && (before || words.get(i + 1).is_some_and(digits)) {
        return None;
    }
    as_code(w)
}

/// Capitals without a vowel, 6 to 8 of them ("PGXFGG"): a code of letters
/// only. Generators leave the vowels out so that a code never spells a
/// word, and a word in capitals ("DOCTOLIB", "BONJOUR") nearly always has
/// one.
fn is_letter_code(word: &[char]) -> bool {
    (6..=8).contains(&word.len())
        && word.iter().all(char::is_ascii_uppercase)
        && !word.iter().any(|c| matches!(c, 'A' | 'E' | 'I' | 'O' | 'U' | 'Y'))
}

/// `word` as a code, if it is one: 4 to 8 digits that are not a year, or
/// 6 to 8 capitals and digits with at least one of each.
fn as_code(word: &[char]) -> Option<String> {
    // Two groups of three with a dash ("904-426", WhatsApp; "65A-TQC",
    // Texts): copied without it, as the field that takes it expects.
    if word.len() == 7 && word[3] == '-' {
        return as_code(&[&word[..3], &word[4..]].concat());
    }
    if is_letter_code(word) {
        return Some(word.iter().collect());
    }
    let digits = word.iter().filter(|c| c.is_ascii_digit()).count();
    let capitals = word.iter().filter(|c| c.is_ascii_uppercase()).count();
    // A masked number ("XXXX0405", a card or an account in a bank's
    // letter) is not a code.
    if word.windows(2).any(|p| p == ['X', 'X']) {
        return None;
    }
    // A promotion code is a word and a two- or three-digit number
    // ("BOOMER30", "POINTE01", "AIRD19"); a verification code mixes them at
    // random ("4T39TT", "1RLUBS", "VSM6WQZ7", and "FTNQG7": a single digit
    // at the end is chance, not a number).
    // So are "NEWS1120" and a word with a year ("BF2024"). Not a short
    // number then letters ("15MERCIS" is a promotion, but "97PTRG",
    // "8NBGIT" and "1RLUBS" are codes: random codes often start so).
    let letters = word.iter().take_while(|c| c.is_ascii_uppercase()).count();
    let tail = &word[letters..];
    let promotion = tail.iter().all(char::is_ascii_digit)
        && ((letters >= 3 && tail.len() >= 2) || (letters >= 2 && is_year(tail)));
    if promotion && digits < word.len() {
        return None;
    }
    let ok = if digits == word.len() {
        (4..=8).contains(&word.len()) && !(word.len() == 4 && is_year(word))
    } else {
        (6..=8).contains(&word.len()) && digits > 0 && capitals > 0 && digits + capitals == word.len()
    };
    ok.then(|| word.iter().collect())
}

/// 1900 to 2099.
fn is_year(word: &[char]) -> bool {
    matches!(word, ['1', '9', _, _] | ['2', '0', _, _])
}

/// A run of 6 to 8 digits glued to other characters ("IKEA.677412Veiller"),
/// the first in `chars`: rule 3 only, right after an announcing word.
fn glued_digits(chars: &[char]) -> Option<String> {
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && chars[i].is_ascii_digit() {
            i += 1;
        }
        // Not part of a number with separators (a date, an amount): the
        // characters around are letters, or the punctuation that ends a
        // sentence.
        let around = [start.checked_sub(1).map(|p| chars[p]), chars.get(i).copied()];
        let joined = around.iter().flatten().any(|c| matches!(c, '/' | ':' | '-' | ',' | '+' | '€' | '$' | '%'))
            || chars.get(i).is_some_and(|c| *c == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit));
        // After a letter, the digits end an identifier ("ju0583116", an
        // order number), not a sentence.
        let after_letter = start > 0 && chars[start - 1].is_alphanumeric();
        let word = &chars[start..i];
        if (6..=8).contains(&word.len()) && !joined && !after_letter && !is_address(word_around(chars, start, i)) {
            return Some(word.iter().collect());
        }
    }
    None
}

/// The whole word (between spaces) that holds `start..end`.
fn word_around(chars: &[char], start: usize, end: usize) -> &[char] {
    let lo = chars[..start].iter().rposition(|c| *c == ' ' || *c == '\n').map_or(0, |p| p + 1);
    let hi = chars[end..].iter().position(|c| *c == ' ' || *c == '\n').map_or(chars.len(), |p| end + p);
    &chars[lo..hi]
}

/// A word that is (part of) an address: a URL, a mail address, a query.
fn is_address(word: &[char]) -> bool {
    let s: String = word.iter().collect::<String>().to_lowercase();
    s.contains("://") || s.contains('@') || s.starts_with("www.") || s.contains('=') || s.contains('?') || s.contains('&')
}

/// `word` without the punctuation around it: `*G49DTC*`, `(123456)`,
/// `651440.`, `« 345847 »`.
fn trim(word: &[char]) -> &[char] {
    let p = |c: &char| !c.is_alphanumeric();
    let start = word.iter().position(|c| !p(c)).unwrap_or(word.len());
    let end = word.iter().rposition(|c| !p(c)).map_or(start, |e| e + 1);
    &word[start..end]
}

/// A character lowercased, without its accent, for the word lists. One
/// character for one, so positions stay those of the text as written.
fn fold(c: char) -> char {
    // `to_lowercase` may give several characters (for a few letters of
    // other alphabets): the first one is enough to compare French and
    // English words.
    let c = c.to_lowercase().next().unwrap_or(c);
    match c {
        'à' | 'â' | 'ä' | 'á' => 'a',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'î' | 'ï' | 'í' => 'i',
        'ô' | 'ö' | 'ó' => 'o',
        'ù' | 'û' | 'ü' | 'ú' => 'u',
        'ç' => 'c',
        '’' => '\'',
        // Hyphens and dashes as written by mail apps: "one‑time".
        '‐' | '‑' | '–' => '-',
        c => c,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(subject: &str, body: &str) -> Option<String> {
        find(subject, Some(body), None)
    }

    // The mails below are shortened from real code mails, with their
    // codes, names and addresses changed: each is one way of showing a
    // code.

    #[test]
    fn the_subject_gives_the_code() {
        assert_eq!(text("Votre code ChatGPT : 797899", "Saisissez le code de vérification temporaire :\n\n797899").as_deref(), Some("797899"));
        assert_eq!(text("263117 est le code de confirmation de votre nouvelle adresse e-mail", "").as_deref(), Some("263117"));
        assert_eq!(text("[Gandi] 772432 - Votre code à usage unique", "").as_deref(), Some("772432"));
        assert_eq!(text("Code de vérification: 3280", "").as_deref(), Some("3280"));
        assert_eq!(text("Booking.com - Voici votre code de vérification : 4T39TT", "").as_deref(), Some("4T39TT"));
        assert_eq!(text("VSM6WQZ7 est votre code de vérification SUPER PDP", "").as_deref(), Some("VSM6WQZ7"));
        assert_eq!(text("Your Hetzner verification code is 597676", "").as_deref(), Some("597676"));
        assert_eq!(text("036764 is your verification code", "").as_deref(), Some("036764"));
        assert_eq!(text("115896 : code pour accéder à votre Compte La Poste", "Voici votre code de vérification").as_deref(), Some("115896"));
    }

    #[test]
    fn a_code_alone_on_its_line() {
        let google = "Bonjour Marie,\n\nNous avons reçu une demande d'accès à votre compte Google\nexemple@gmail.com. Votre code de\nvalidation Google est :\n\n\n230605\n\n\nSi vous n'avez pas demandé ce code…\n© Google Inc., 1600 Amphitheatre Parkway, CA 94043";
        assert_eq!(text("Code de validation Google", google).as_deref(), Some("230605"));
        let doctolib = "Doctolib\n\nCode de vérification\n\nPour valider votre nouvelle adresse e-mail, veuillez entrer le\ncode ci-dessous sur Doctolib.\n\n78355770\n\nCe code à usage unique n'est valable que pendant 10 minutes.";
        assert_eq!(text("Code de vérification Doctolib", doctolib).as_deref(), Some("78355770"));
        let digitalocean = "Date: 2026-11-10 14:15:32 UTC\nAccount: moi@example.com\nIP Address: 90.90.96.79\n\nEnter this 6 digit code on the sign in page to confirm your identity:\n\n884673\n";
        assert_eq!(text("DigitalOcean Verification Code", digitalocean).as_deref(), Some("884673"));
        let booking = "   Voici votre code unique, que vous pouvez utiliser pour vous connecter\n   en toute sécurité à votre compte.\n   4T39TT\n\n   Ce code ne peut être utilisé qu'une seule fois.";
        assert_eq!(text("Booking.com - Voici votre code de vérification", booking).as_deref(), Some("4T39TT"));
    }

    #[test]
    fn the_code_may_come_before_the_sentence_that_announces_it() {
        let xpeng = "3280\n\nIl s'agit de votre code de vérification pour le changement d'e-mail, valable 15 minutes.";
        assert_eq!(text("Votre compte", xpeng).as_deref(), Some("3280"));
        // "mot de passe ci-dessous" announces it; the subject says it is
        // a one-time code.
        let levi = "Veuillez saisir le mot de passe ci-dessous pour vous connecter à votre compte.\n\n141599\n\nCe code expire dans 30 minutes.";
        assert_eq!(text("Votre code à usage unique pour votre compte est prêt!", levi).as_deref(), Some("141599"));
    }

    #[test]
    fn thousands_of_spaces_of_an_html_layout_do_not_part_the_code_from_its_sentence() {
        let pathe = format!(
            "Bonjour,\n{}Pour confirmer la connexion à votre Compte, saisissez le code ci-dessous :\n{}119094\n{}Ce code est personnel.",
            " \n".repeat(400),
            "        \n".repeat(500),
            "    \n".repeat(300)
        );
        assert_eq!(text("Votre code d'authentification", &pathe).as_deref(), Some("119094"));
    }

    #[test]
    fn a_code_on_the_line_of_its_announcement() {
        let bitwarden = "To finish logging in, enter this verification code: 651440\nIf this was not you…\nIP: 90.47.66.34";
        assert_eq!(text("Your Bitwarden Verification Code", bitwarden).as_deref(), Some("651440"));
        let impots = "Bonjour,\n\nVotre code de sécurité est le suivant :\n427242 Il est valable 30 minutes.\nContactez le 0 809 401 401.";
        assert_eq!(text("impots.gouv.fr - Code de sécurité", impots).as_deref(), Some("427242"));
        // The same code is in a link first: never taken from an address.
        let cooltra = "Vérifier la tentative de connexion [https://link.example.com/2fa?code=G49DTC&utm=x]\n\nVous pouvez également entrer manuellement le code dans votre application : *G49DTC*";
        assert_eq!(text("Votre code de vérification", cooltra).as_deref(), Some("G49DTC"));
        let orbstack = "036764 is your OTP code for OrbStack.\n\nDo not share this with anyone. It was requested at 18 June 2026, 13:39 UTC.";
        assert_eq!(text("Sign in", orbstack).as_deref(), Some("036764"));
    }

    #[test]
    fn a_code_called_confidential() {
        let ovh = "Si vous êtes à l'origine de cette connexion, entrez le code suivant afin de confirmer l'origine de la connexion.\n\
                   Ne le donnez à personne.\nCode confidentiel: 30973368  (valable 15min)\nOVH SAS, au capital de 10 069 020 euros";
        assert_eq!(text("Confirmez votre demande de connexion", ovh).as_deref(), Some("30973368"));
    }

    #[test]
    fn a_code_glued_to_the_text_of_a_layout_without_line_breaks() {
        let ikea = "HejVoici le code à 6 chiffres à saisir pour vérifier votre email et ainsi sécuriser votre compte IKEA.677412Veiller à ne pas communiquer ce code.© Inter IKEA Systems B.V. 1999-2025";
        assert_eq!(text("Code à usage unique pour accéder à votre compte IKEA.", ikea).as_deref(), Some("677412"));
    }

    #[test]
    fn an_html_only_mail_is_read_from_its_html() {
        let html = "<p>Bonjour,</p><p>Veuillez utiliser le code suivant pour vous connecter à votre compte : <b>772432</b></p><p>Il est valable 15 minutes.</p>";
        assert_eq!(find("Connexion", Some("\n\n"), Some(html)).as_deref(), Some("772432"));
        assert_eq!(find("Connexion", None, Some(html)).as_deref(), Some("772432"));
    }

    #[test]
    fn a_mail_that_speaks_of_no_code_to_type_has_none() {
        // A promo code, a postal code, a newsletter, an invoice.
        assert_eq!(text("Votre code promo : BIENVENUE20", "Utilisez le code promo AB12CD34 avant le 31.\n\n123456\n"), None);
        assert_eq!(text("Livraison", "Code postal :\n75009\nConfirmation de votre commande"), None);
        assert_eq!(text("Gift code inside", "Your gift code, valid until confirmation:\nXK29PQ7"), None);
        assert_eq!(text("Facture 2025-118", "Commande n°\n4512398\nMerci pour votre achat."), None);
        assert_eq!(text("Lettre du lundi", "Le code source du projet est en ligne.\n\n2026\n"), None);
    }

    #[test]
    fn numbers_that_are_not_codes_are_left_out() {
        // A code mail whose body holds dates, times, a phone number, a
        // postal code, a year, but no code.
        let body = "Nous avons reçu une demande de connexion avec un code de vérification.\nDate : 2026-11-10 14:15:32\nIP 90.47.66.34\nTel.: +49 9831 505-0\n\n2026\n\nHRB 6089\nDE812871812";
        assert_eq!(text("Code de vérification", body), None);
        // Capital words without a digit, or too short.
        assert_eq!(text("Code de vérification", "Votre code de vérification :\n\nDOCTOLIB\n\nAB12\n"), None);
    }

    #[test]
    fn a_reply_or_a_forward_is_a_conversation_not_a_code_mail() {
        let body = "Bonjour,\nvoici le code d'accès pour votre connexion :\n482913\n";
        assert_eq!(text("Code d'accès", body).as_deref(), Some("482913"));
        for subject in ["RE: Code d'accès", "TR : Code d'accès", "Fwd: Code d'accès", "fw:Code d'accès"] {
            assert_eq!(text(subject, body), None, "{subject}");
        }
        // A word that only starts like a prefix is not one.
        assert_eq!(text("Rendez-vous : code de vérification", "\n482913\n").as_deref(), Some("482913"));
    }

    #[test]
    fn other_kinds_of_codes_are_left_out() {
        // The law, an error, a bank card's PIN in a fraud warning, a
        // booking reference.
        assert_eq!(text("Lettre sociale", "Le code du travail change, connexion à votre espace :\n482913\n"), None);
        assert_eq!(text("LogWatch for server", "sshd: login failures\nexit code 255\n\n48291\n"), None);
        assert_eq!(text("Mentions", "Entreprise régie par le Code de la sécurité sociale\n482913\n"), None);
        // A bank's fraud warning speaks of codes; its footer's numbers are
        // not one.
        let warning = format!(
            "Ne communiquez jamais votre code confidentiel ni le code d'authentification reçu par SMS.\n{}\n\
             SA au capital de 37 902 080 € – Code d'établissement bancaire n°11 238\nN° ORIAS :\n08040222",
            "Votre conseiller ne vous demandera jamais ces informations. ".repeat(8)
        );
        assert_eq!(text("Fraude au faux conseiller, appel à la vigilance !", &warning), None);
        assert_eq!(text("Votre voyage", "Votre code de réservation, à confirmer :\nK7Q2ZT\n"), None);
    }

    #[test]
    fn a_newsletter_lends_no_code_word_to_the_numbers_of_its_footer() {
        // One qualified "code" in a paragraph; another "code", further,
        // near a postal code: the postal code is not taken.
        let body = format!(
            "Activez la double authentification avec un code de sécurité.\n{}\nUtilisez le code AIRD19 en boutique.\n12 rue de la Paix\n75008\nParis",
            "Notre actualité du mois. ".repeat(40)
        );
        assert_eq!(text("Votre lettre d'information", &body), None);
    }

    #[test]
    fn promotion_codes_grouped_numbers_and_tracking_links_are_left_out() {
        // "digitale" is not "digits".
        assert_eq!(text("Formation", "L'agence de communication digitale vous offre 37 € avec le code BOOMER30 jusqu'au 30 avril."), None);
        assert_eq!(text("Code de vérification", "Votre code de vérification :\nAIRD19\n"), None);
        for promo in ["NEWS1120", "BF2024"] {
            assert_eq!(text("Code de vérification", &format!("Votre code, valable 10 minutes :\n{promo}\n")), None, "{promo}");
        }
        assert_eq!(text("Vite !", "Votre code exclusif 60666 expire lundi minuit"), None);
        assert_eq!(text("Portes ouvertes", "Offre valable uniquement sur présentation du code 2829. Offre valable jusqu'au 30."), None);
        // "valable" before a duration does qualify a code.
        assert_eq!(text("Connexion", "Code confidentiel: 30973368  (valable 15min)").as_deref(), Some("30973368"));
        // A card payment's authorization code is not one to type.
        assert_eq!(text("Confirmation de votre commande", "Montant : 42,00 EUR\nCode d'autorisation : 709214\nMerci de votre achat."), None);
        // A meeting number cut in groups.
        assert_eq!(text("Webex", "Meeting number (access code): 2641 801 9509\nMeeting password: kBvnHz"), None);
        // A qualified "code" in a tracking link, a postal code after it.
        assert_eq!(text("Vos infos", "http://t.example.com/r.aspx?cle=1&code_validation=32583140\nOrange, 1 avenue Nelson Mandela\n94745 Arcueil cedex"), None);
        // Other codes than one to type: the first number after the word is
        // not a code, nothing is taken.
        assert_eq!(text("Inventaire", "Ci-joint les codes vérifiés : MN6121/96-UBO comptés 1030 ex"), None);
    }

    #[test]
    fn a_code_to_authorize_a_login_and_a_code_said_to_expire() {
        let firefox = "Adresse IP : 90.0.0.1\nmercredi, 16 août 2023\n19:13:13 (CEST)\nSi oui, voici le code d’autorisation :\n110406\nIl expire dans 5 minutes.";
        assert_eq!(text("Accepter la connexion à Firefox", firefox).as_deref(), Some("110406"));
        assert_eq!(text("Bienvenue", "Your code is below, it expires in 10 minutes:\n\nFTNQG7\n").as_deref(), Some("FTNQG7"));
        // Random codes that happen to start with letters or digits.
        assert_eq!(text("Code de vérification", "\n1RLUBS\n").as_deref(), Some("1RLUBS"));
        assert_eq!(text("Code de vérification", "\nAB5832\n").as_deref(), Some("AB5832"));
        assert_eq!(text("Code de vérification", "\n97PTRG\n").as_deref(), Some("97PTRG"));
    }

    #[test]
    fn a_code_in_two_groups_is_copied_as_one() {
        assert_eq!(text("Code de vérification WhatsApp", "Saisissez ce code :\n904-426\nCe code expirera dans 4 heures.").as_deref(), Some("904426"));
        assert_eq!(text("65A-TQC is your Texts confirmation code", "").as_deref(), Some("65ATQC"));
        assert_eq!(text("Confirm your email", "Your 6 digit code is below\n831 884\nIf you didn't request this…").as_deref(), Some("831884"));
        assert_eq!(text("Beeper confirmation code: 831 884", "").as_deref(), Some("831884"));
        assert_eq!(text("112-458 - votre code de vérification", "").as_deref(), Some("112458"));
        // A date, a longer number cut in groups.
        assert_eq!(text("Code de vérification", "Votre code de vérification\n2026-11-10\n"), None);
        assert_eq!(text("Code de vérification", "Votre code de vérification\n948 402 068\n"), None);
    }

    #[test]
    fn a_code_of_capitals_without_a_vowel() {
        assert_eq!(text("Code de Vérification", "Veuillez saisir le code ci-dessous :\nCode de Vérification : PGXFGG\nIl est valide 8 minutes.").as_deref(), Some("PGXFGG"));
        // A word in capitals has a vowel.
        assert_eq!(text("Code de vérification", "Votre code de vérification :\nBONJOUR\n"), None);
    }

    #[test]
    fn a_code_to_get_an_account_back() {
        let facebook = "Nous avons reçu une demande de réinitialisation de votre mot de passe.Entrez le code de réinitialisation du mot de passe suivant :06150540Vous pouvez aussi changer directement votre mot de passe.";
        assert_eq!(text("Votre compte", facebook).as_deref(), Some("06150540"));
    }

    #[test]
    fn a_text_part_that_stops_short_of_the_code_leaves_it_to_the_html() {
        let text_part = "Veuillez trouver ci-dessous votre code de confirmation à saisir pour finaliser votre demande.";
        let html = "<p>Veuillez trouver ci-dessous votre code de confirmation à saisir pour finaliser votre demande.</p><p><b>482913</b></p>";
        assert_eq!(find("Votre compte - Code de confirmation", Some(text_part), Some(html)).as_deref(), Some("482913"));
    }

    #[test]
    fn codes_to_collect_a_parcel_or_join_a_meeting_are_codes_too() {
        assert_eq!(text("Votre commande est prête", "Vous pouvez venir la récupérer en présentant le code 1680 et votre pièce d'identité.").as_deref(), Some("1680"));
        assert_eq!(text("Invitation", "Meeting ID: 948 4026 0686\nPasscode: 219108").as_deref(), Some("219108"));
    }

    #[test]
    fn masked_numbers_and_identifiers_are_not_codes() {
        assert_eq!(text("Code de vérification", "Votre code de vérification :\nXXXX0405\n"), None);
        assert_eq!(text("Code de vérification", "Votre code de vérification pour la commande ju0583116 est envoyé."), None);
    }

    #[test]
    fn the_announcing_words_are_found_in_any_case_and_accent() {
        assert_eq!(text("CODE DE VÉRIFICATION", "\n482913\n").as_deref(), Some("482913"));
        assert_eq!(text("Your one‑time passcode", "\n482913\n").as_deref(), Some("482913"));
    }

    #[test]
    fn a_huge_body_is_read_only_at_its_top() {
        let body = format!("Votre code de vérification\n\n{}\n\n482913", "lorem ipsum ".repeat(100_000));
        let start = std::time::Instant::now();
        assert_eq!(text("Connexion", &body), None);
        assert!(start.elapsed() < std::time::Duration::from_millis(200), "{:?}", start.elapsed());
    }
}
