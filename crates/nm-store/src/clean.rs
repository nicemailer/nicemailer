//! Simplified display of a thread: what each message brings, without the
//! history it repeats (`ui.md`, "Affichage d'une conversation").
//!
//! Two steps:
//! 1. [`split`] cuts the text of one message into typed segments: new text,
//!    interleaved quotes, the final quoted history, signature, repeated
//!    mentions ("Sent from my iPhone", confidentiality notices), forwarded
//!    content. Pure, per message.
//! 2. [`clean_thread`] looks at the whole thread: a segment that copies an
//!    earlier message is hidden, a signature repeated by a sender is hidden,
//!    and a message that brings nothing new is flagged.
//!
//! Nothing is lost: hidden segments are returned with `hidden = true`, and
//! the app shows them on demand. What other mail clients do, and why these
//! rules, is in `docs/thread-display.md`.
//!
//! Computed by the core when a thread is opened (`get_thread`), so every
//! client (app, CLI, future TUI) gets the same simplified view without
//! reimplementing it. Not stored: splitting a message takes microseconds,
//! and the rules can change without a reindex.

/// Kind of a segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentKind {
    /// What the sender wrote.
    Text,
    /// Quoted lines interleaved with the sender's answers (point-by-point
    /// reply): kept, as they give the context of the answers.
    Quote,
    /// The quoted history at the end of the message, with its attribution
    /// line ("On … wrote:") or its Outlook header block.
    QuoteTail,
    /// Signature: after the `-- ` delimiter, or the closing lines a sender
    /// repeats in every message.
    Signature,
    /// Repeated mentions: "Sent from my iPhone", confidentiality notices.
    Boilerplate,
    /// A forwarded message: new content for the thread, never hidden.
    Forward,
    /// New-looking text that copies an earlier message of the thread.
    Copy,
}

impl SegmentKind {
    /// Name used in the contract (`SegmentView.kind`).
    pub fn as_str(self) -> &'static str {
        match self {
            SegmentKind::Text => "text",
            SegmentKind::Quote => "quote",
            SegmentKind::QuoteTail => "quote_tail",
            SegmentKind::Signature => "signature",
            SegmentKind::Boilerplate => "boilerplate",
            SegmentKind::Forward => "forward",
            SegmentKind::Copy => "copy",
        }
    }
}

/// A piece of a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub kind: SegmentKind,
    /// The text, quote markers (`>`) removed for quotes.
    pub text: String,
    /// Quote depth (number of `>`), 0 for other kinds.
    pub depth: u8,
}

/// Number of `>` at the start of a line (spaces between them allowed), and
/// the rest of the line.
fn quote_depth(line: &str) -> (u8, &str) {
    let mut depth = 0u8;
    let mut rest = line;
    loop {
        let t = rest.trim_start_matches(' ');
        if let Some(r) = t.strip_prefix('>') {
            depth = depth.saturating_add(1);
            rest = r;
        } else {
            break;
        }
    }
    if depth == 0 { (0, line) } else { (depth, rest.strip_prefix(' ').unwrap_or(rest)) }
}

fn lower(s: &str) -> String {
    s.trim().to_lowercase()
}

/// How a mail client words the line that introduces a quote, in one
/// language: the line starts with `start` (empty: anywhere), holds `word`,
/// and ends with `:` after it. Gathered from Apple Mail, Gmail, Outlook and
/// Thunderbird (`specifications/localisation.md`, section 3.3), lower case.
struct Wording {
    start: &'static str,
    word: &'static str,
}

/// The wordings known, by language.
const WORDINGS: &[Wording] = &[
    // English: "On Tue, 3 Nov 2026 at 10:52, Alice <a@x.org> wrote:".
    Wording { start: "on ", word: "wrote" },
    // French: "Le 3 nov. 2026 à 10:52, Alice <a@x.org> a écrit :".
    Wording { start: "le ", word: "a écrit" },
    // German: "Am 03.11.2026 um 10:52 schrieb Alice <a@x.org>:" (Apple
    // Mail, Gmail), "Am 03.11.26 um 10:52 schrieb Alice:" (Thunderbird).
    Wording { start: "am ", word: "schrieb" },
    // Spanish: "El mar, 3 nov 2026 a las 10:52, Alice (<a@x.org>) escribió:".
    Wording { start: "el ", word: "escribió" },
    // Italian: "Il giorno mar 3 nov 2026 alle ore 10:52 Alice <a@x.org> ha scritto:".
    Wording { start: "il ", word: "ha scritto" },
    // Portuguese: "Em ter., 3 de nov. de 2026 às 10:52, Alice <a@x.org> escreveu:".
    Wording { start: "em ", word: "escreveu" },
    // Dutch: "Op di 3 nov 2026 om 10:52 schreef Alice <a@x.org>:" (Gmail),
    // "Op 3 nov. 2026 om 10:52 heeft Alice <a@x.org> het volgende geschreven:" (Apple Mail).
    Wording { start: "op ", word: "schreef" },
    Wording { start: "op ", word: "het volgende geschreven" },
    // Swedish and Danish: "Den tis 3 nov. 2026 kl 10:52 skrev Alice <a@x.org>:".
    Wording { start: "den ", word: "skrev" },
    // Norwegian: "tir. 3. nov. 2026 kl. 10:52 skrev Alice <a@x.org>:".
    Wording { start: "", word: " skrev " },
    // Polish: "wt., 3 lis 2026 o 10:52 Alice <a@x.org> napisał(a):" (Gmail),
    // "W dniu 3.11.2026 o 10:52, Alice pisze:" (Thunderbird).
    Wording { start: "", word: "napisał(a)" },
    Wording { start: "w dniu ", word: "pisze" },
    // Russian (Thunderbird): "03.11.2026 10:52, Alice пишет:". Ukrainian:
    // "3 лист. 2026 р. о 10:52 Alice <a@x.org> пише:" ("пишет" holds it too).
    Wording { start: "", word: "пише" },
    // Finnish: "ti 3.11.2026 klo 10.52 Alice <a@x.org> kirjoitti:".
    Wording { start: "", word: "kirjoitti" },
    // Japanese (Apple Mail): "2026/11/03 10:52、Alice <a@x.org>のメール:".
    Wording { start: "", word: "のメール" },
    // Korean (Gmail): "2026년 11월 3일 (화) 오전 10:52, Alice <a@x.org>님이 작성:".
    Wording { start: "", word: "님이 작성" },
    // Chinese, simplified and traditional: "Alice <a@x.org> 于2026年11月3日周二 10:52写道：".
    Wording { start: "", word: "写道" },
    Wording { start: "", word: "寫道" },
];

/// Whether `l` (lower case, trimmed) ends with a colon, full-width
/// included (Chinese and Japanese clients write `：`).
fn ends_with_colon(l: &str) -> bool {
    l.ends_with(':') || l.ends_with('：')
}

/// One line that introduces a quote: a known wording, or, for the clients
/// that put no word at all (Gmail in Japanese, "2026年11月3日(火) 10:52 Alice
/// <a@x.org>:", or in Russian), a line ending with a colon that holds an
/// address and a figure. Only ever asked of the line right above the
/// quoted history (`split`), so a sentence of the user's ending with a
/// colon, which rarely holds an address, is not mistaken for one.
fn attribution_line(line: &str) -> bool {
    let l = lower(line);
    if !ends_with_colon(&l) {
        return false;
    }
    let body = l.trim_end_matches([':', '：', ' ']);
    let worded = WORDINGS.iter().any(|w| l.starts_with(w.start) && (body.ends_with(w.word.trim()) || body.contains(w.word)));
    let bare = l.contains('@') && l.chars().any(|c| c.is_ascii_digit()) && (l.contains('>') || l.contains(')'));
    worded || bare
}

/// Attribution line(s) introducing a quote (`WORDINGS`). Some clients wrap
/// it over two lines. Returns the number of lines it takes (0 if `line` is
/// not one).
fn attribution(line: &str, next: Option<&str>) -> usize {
    if attribution_line(line) {
        return 1;
    }
    if let Some(n) = next {
        if n.trim().is_empty() || ends_with_colon(&lower(line)) {
            return 0;
        }
        let joined = format!("{} {}", line.trim(), n.trim());
        if attribution_line(&joined) {
            return 2;
        }
    }
    0
}

/// Separator lines that start a quoted history in Outlook and similar
/// clients.
fn history_separator(line: &str) -> bool {
    let l = lower(line);
    let core = l.trim_matches(|c: char| c == '-' || c == ' ' || c == '_');
    (l.starts_with("---") || l.starts_with("___"))
        && (core.is_empty()
            || core == "original message"
            || core == "message d'origine"
            || core == "message original"
            || core == "ursprüngliche nachricht"
            || core == "originalnachricht"
            || core == "mensaje original"
            || core == "messaggio originale"
            || core == "mensagem original"
            || core == "oorspronkelijk bericht"
            || core == "ursprungligt meddelande")
}

/// Lines that start a forwarded message.
fn forward_marker(line: &str) -> bool {
    let l = lower(line);
    let core = l.trim_matches(|c: char| c == '-' || c == ' ');
    [
        "forwarded message",
        "message transféré",
        "weitergeleitete nachricht",
        "mensaje reenviado",
        "messaggio inoltrato",
        "mensagem encaminhada",
        "mensagem reencaminhada",
        "doorgestuurd bericht",
        "vidarebefordrat meddelande",
        "videresendt meddelelse",
        "videresendt melding",
        "välitetty viesti",
        "przekazana wiadomość",
        "wiadomość przekazana dalej",
        "пересылаемое сообщение",
        "переслане повідомлення",
        "転送メッセージ",
        "전달된 메시지",
        "转发的邮件",
        "轉寄的郵件",
    ]
    .contains(&core)
        || [
            "begin forwarded message",
            "début du message réexpédié",
            "début du message transféré",
            "anfang der weitergeleiteten nachricht",
            "comienzo del mensaje reenviado",
            "inizio messaggio inoltrato",
            "início da mensagem encaminhada",
            "início da mensagem reencaminhada",
            "begin doorgestuurd bericht",
        ]
        .iter()
        .any(|p| l.starts_with(p))
}

/// Whether an Outlook-style header block ("From: … Sent: … To: …
/// Subject: …") starts at `i`: the reply history follows it, unquoted.
fn header_block(lines: &[&str], i: usize) -> bool {
    let starts = |l: &str, keys: &[&str]| {
        let l = lower(l).trim_start_matches('*').to_string();
        keys.iter().any(|k| l.starts_with(k))
    };
    // English, French, German, Spanish and Portuguese (`de:`), Italian
    // (`da:`), Dutch, Swedish.
    if !starts(lines[i], &["from:", "de :", "de:", "von:", "expéditeur :", "da:", "van:", "från:"]) {
        return false;
    }
    let window = &lines[i + 1..lines.len().min(i + 6)];
    let date = window.iter().any(|l| {
        starts(l, &[
            "sent:", "envoyé :", "envoyé:", "date:", "date :", "gesendet:", "datum:", "enviado:", "enviada:", "fecha:",
            "data:", "inviato:", "verzonden:", "skickat:",
        ])
    });
    let to = window.iter().any(|l| {
        starts(l, &[
            "to:", "à :", "à:", "a :", "a:", "subject:", "objet :", "objet:", "an:", "betreff:", "para:", "asunto:",
            "assunto:", "oggetto:", "aan:", "onderwerp:", "till:", "ämne:",
        ])
    });
    date && to
}

/// Mentions added by mail apps under a message.
fn mention(line: &str) -> bool {
    let l = lower(line);
    [
        "envoyé de mon ",
        "envoyé depuis mon ",
        "sent from my ",
        "sent from mail for",
        "sent from outlook",
        "sent from yahoo mail",
        "sent with proton mail",
        "get outlook for",
        "obtenir outlook pour",
        "téléchargez outlook pour",
        "envoyé à partir de outlook",
        "envoyé à partir de mon",
        "von meinem iphone gesendet",
        "gesendet von meinem ",
        "enviado desde mi ",
        "enviado do meu ",
        "inviato da ",
        "verzonden vanaf mijn ",
        "skickat från min ",
        "sendt fra min ",
        "wysłane z mojego ",
    ]
    .iter()
    .any(|p| l.starts_with(p))
}

/// Confidentiality notices, appended by mail servers of companies.
fn disclaimer(paragraph: &str) -> bool {
    let l = paragraph.to_lowercase();
    (l.contains("confidential") || l.contains("confidentiel"))
        && (l.contains("intended") || l.contains("recipient") || l.contains("destinataire") || l.contains("destiné"))
}

/// Cuts the text of a message into segments, in order. See the module
/// documentation.
pub fn split(text: &str) -> Vec<Segment> {
    let text = text.replace("\r\n", "\n");
    let lines: Vec<&str> = text.split('\n').collect();
    let n = lines.len();
    let first_forward = lines.iter().position(|l| forward_marker(l));

    // 1. Start of the final quoted history.
    let mut tail = n;
    // Outlook: a header block (or a separator line) followed by the
    // history, unquoted. Ignored inside a forwarded message, whose header
    // looks the same.
    for i in 0..n {
        if first_forward.is_some_and(|f| f <= i) {
            break;
        }
        if history_separator(lines[i]) && (i + 1 < n && (header_block(&lines, i + 1) || lower(lines[i]).contains("message"))) {
            tail = i;
            break;
        }
        if header_block(&lines, i) {
            tail = i;
            break;
        }
    }
    // Quoted lines at the end, after the last line of new text, with the
    // attribution line just before them.
    if tail == n {
        let mut end = n;
        while end > 0 && lines[end - 1].trim().is_empty() {
            end -= 1;
        }
        let mut start = end;
        while start > 0 && (quote_depth(lines[start - 1]).0 > 0 || lines[start - 1].trim().is_empty()) {
            start -= 1;
        }
        let has_quote = (start..end).any(|i| quote_depth(lines[i]).0 > 0);
        if has_quote && first_forward.is_none_or(|f| f > start) {
            // Skip blank lines back to a possible attribution.
            let mut a = start;
            while a > 0 && lines[a - 1].trim().is_empty() {
                a -= 1;
            }
            if a >= 2 && attribution(lines[a - 2], Some(lines[a - 1])) == 2 {
                start = a - 2;
            } else if a >= 1 && attribution(lines[a - 1], None) == 1 {
                start = a - 1;
            }
            tail = start;
        } else if let Some(i) = (0..n).find(|&i| {
            // An attribution followed only by quotes and blank lines is also
            // a tail, even if new text is under it (top and bottom posting
            // mixed are rare; an attribution in the middle is handled above).
            attribution(lines[i], lines.get(i + 1).copied()) > 0
                && lines[i + 1..].iter().all(|l| l.trim().is_empty() || quote_depth(l).0 > 0)
                && lines[i + 1..].iter().any(|l| quote_depth(l).0 > 0)
        }) {
            tail = i;
        }
    }

    // 2. Signature, mentions and notices, before the history.
    let body = &lines[..tail];
    let mut end = body.len();
    let mut trailing: Vec<Segment> = Vec::new();
    // Signature delimiter "-- " (RFC 3676), the last one.
    if let Some(s) = body.iter().rposition(|l| l.trim_end() == "--")
        && first_forward.is_none_or(|f| f > s)
    {
        trailing.push(Segment { kind: SegmentKind::Signature, text: join(&body[s..end]), depth: 0 });
        end = s;
    }
    // Mentions on their own line and confidentiality paragraphs at the end.
    loop {
        let mut e = end;
        while e > 0 && body[e - 1].trim().is_empty() {
            e -= 1;
        }
        if e == 0 {
            break;
        }
        if mention(body[e - 1]) {
            trailing.insert(0, Segment { kind: SegmentKind::Boilerplate, text: body[e - 1].trim().to_string(), depth: 0 });
            end = e - 1;
            continue;
        }
        // The last paragraph.
        let mut p = e;
        while p > 0 && !body[p - 1].trim().is_empty() {
            p -= 1;
        }
        let paragraph = join(&body[p..e]);
        if p > 0 && disclaimer(&paragraph) {
            trailing.insert(0, Segment { kind: SegmentKind::Boilerplate, text: paragraph, depth: 0 });
            end = p;
            continue;
        }
        break;
    }

    // 3. The rest: runs of text and of quotes, and the forwarded part.
    let mut segments: Vec<Segment> = Vec::new();
    let forward_at = first_forward.filter(|&f| f < end);
    let main_end = forward_at.unwrap_or(end);
    let mut i = 0;
    while i < main_end {
        let (d, _) = quote_depth(body[i]);
        let mut j = i;
        let mut text_lines: Vec<&str> = Vec::new();
        if d > 0 {
            // Consecutive quoted lines (blank lines between them included).
            let mut max_depth = d;
            while j < main_end {
                let (dj, rest) = quote_depth(body[j]);
                if dj > 0 {
                    max_depth = max_depth.max(dj);
                    text_lines.push(rest);
                } else if body[j].trim().is_empty() && j + 1 < main_end && quote_depth(body[j + 1]).0 > 0 {
                    text_lines.push("");
                } else {
                    break;
                }
                j += 1;
            }
            push(&mut segments, SegmentKind::Quote, &text_lines, max_depth);
        } else {
            while j < main_end && quote_depth(body[j]).0 == 0 {
                text_lines.push(body[j]);
                j += 1;
            }
            push(&mut segments, SegmentKind::Text, &text_lines, 0);
        }
        i = j;
    }
    if let Some(f) = forward_at {
        push(&mut segments, SegmentKind::Forward, &body[f..end], 0);
    }
    segments.extend(trailing.into_iter().filter(|s| !s.text.trim().is_empty()));
    if tail < n {
        let tail_lines: Vec<&str> = lines[tail..].iter().map(|l| quote_depth(l).1).collect();
        push(&mut segments, SegmentKind::QuoteTail, &tail_lines, 1);
    }
    segments
}

fn join(lines: &[&str]) -> String {
    lines.join("\n").trim_matches('\n').to_string()
}

/// Adds a segment, tidied: blank lines at both ends removed, runs of blank
/// lines reduced to one. Empty segments are dropped.
fn push(segments: &mut Vec<Segment>, kind: SegmentKind, lines: &[&str], depth: u8) {
    let mut out: Vec<&str> = Vec::new();
    for l in lines {
        let l = l.trim_end();
        if l.is_empty() && out.last().is_none_or(|p| p.is_empty()) {
            continue;
        }
        out.push(l);
    }
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    if out.is_empty() {
        return;
    }
    segments.push(Segment { kind, text: out.join("\n"), depth });
}

/// Text reduced to lowercase letters and digits, to compare contents
/// whatever their wrapping, quote markers and punctuation.
pub fn normalize(text: &str) -> String {
    text.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// A segment as displayed: whether it is hidden by default, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanSegment {
    pub segment: Segment,
    pub hidden: bool,
    /// For a hidden quote tail: whether its content is in an earlier
    /// message of the thread (if not, the thread lacks that history).
    pub in_thread: bool,
}

/// The cleaned messages of a thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanMessage {
    pub segments: Vec<CleanSegment>,
    /// No visible new content: the message only quotes earlier ones.
    pub only_quotes: bool,
    /// Nothing written and nothing quoted: an empty body, blank lines, a
    /// signature alone, files without a word. Never true with
    /// `only_quotes`: the two say different things about the message.
    pub no_text: bool,
}

/// Minimum normalized length for a text to count as a copy of an earlier
/// message: short sentences ("Thanks!", "OK pour moi") repeat naturally.
const MIN_COPY: usize = 40;

/// Cleans a thread. `messages` are (sender address, text) in chronological
/// order; the result is in the same order.
pub fn clean_thread(messages: &[(Option<&str>, &str)]) -> Vec<CleanMessage> {
    let mut seen = String::new();
    // New content of each earlier message, normalized.
    let mut earlier: Vec<String> = Vec::new();
    let mut out = Vec::with_capacity(messages.len());
    let split_all: Vec<Vec<Segment>> = messages.iter().map(|(_, t)| split(t)).collect();
    for (i, segments) in split_all.iter().enumerate() {
        let mut clean: Vec<CleanSegment> = Vec::new();
        for s in segments {
            let norm = normalize(&s.text);
            let (kind, hidden, in_thread) = match s.kind {
                SegmentKind::QuoteTail => {
                    // In the thread if the beginning of an earlier message is
                    // inside it. (The tail starts with an attribution or an
                    // Outlook header that no earlier message contains, and
                    // senders often cut the oldest history.)
                    let found = earlier.iter().any(|m| {
                        let head: String = m.chars().take(150).collect();
                        head.len() >= 10 && norm.contains(&head)
                    });
                    (SegmentKind::QuoteTail, true, found)
                }
                SegmentKind::Signature | SegmentKind::Boilerplate => (s.kind, true, false),
                SegmentKind::Text if norm.len() >= MIN_COPY && seen.contains(&norm) => (SegmentKind::Copy, true, true),
                k => (k, false, false),
            };
            clean.push(CleanSegment { segment: Segment { kind, ..s.clone() }, hidden, in_thread });
        }
        split_repeated_signature(messages, &split_all, i, &mut clean);
        // "Only quotes earlier messages": nothing written shows, and what
        // the message holds is a quote or a copy of earlier ones. A message
        // with nothing written and nothing quoted (an empty body, a
        // signature alone, files without a word) quotes nothing: the app
        // says it has no text instead, from `no_text`
        // (`docs/thread-display.md`).
        let writes = clean.iter().any(|c| {
            !c.hidden && matches!(c.segment.kind, SegmentKind::Text | SegmentKind::Forward) && !c.segment.text.trim().is_empty()
        });
        let quotes = clean
            .iter()
            .any(|c| matches!(c.segment.kind, SegmentKind::QuoteTail | SegmentKind::Copy | SegmentKind::Quote));
        let only_quotes = !writes && quotes;
        let no_text = !writes && !quotes;
        // What later messages may copy or quote: the content of this one.
        // `earlier` keeps only what the sender wrote (text, forward): a
        // message that starts with an interleaved quote must still be
        // recognized when a later message quotes its answer.
        let mut written = String::new();
        for c in &clean {
            let norm = normalize(&c.segment.text);
            match c.segment.kind {
                SegmentKind::Text | SegmentKind::Forward => {
                    written.push_str(&norm);
                    seen.push_str(&norm);
                }
                SegmentKind::Copy | SegmentKind::Quote => seen.push_str(&norm),
                _ => {}
            }
        }
        earlier.push(written);
        out.push(CleanMessage { segments: clean, only_quotes, no_text });
    }
    out
}

/// Closing lines a sender repeats in their messages, without the `-- `
/// delimiter (name, title, phone…): the lines at the end of the last text
/// segment that are identical in another message of the same sender become
/// a hidden signature. At least two lines, and some text must remain.
fn split_repeated_signature(
    messages: &[(Option<&str>, &str)],
    split_all: &[Vec<Segment>],
    i: usize,
    clean: &mut Vec<CleanSegment>,
) {
    let Some(sender) = messages[i].0 else { return };
    let Some(pos) = clean.iter().rposition(|c| c.segment.kind == SegmentKind::Text && !c.hidden) else { return };
    let mine: Vec<String> = clean[pos].segment.text.lines().map(|l| l.trim().to_string()).collect();
    let mut best = 0;
    for (j, (other_sender, _)) in messages.iter().enumerate() {
        if j == i || *other_sender != Some(sender) {
            continue;
        }
        let Some(other) = split_all[j].iter().rev().find(|s| s.kind == SegmentKind::Text) else { continue };
        let theirs: Vec<String> = other.text.lines().map(|l| l.trim().to_string()).collect();
        let common = mine.iter().rev().zip(theirs.iter().rev()).take_while(|(a, b)| a == b).count();
        best = best.max(common);
    }
    if best < 2 || best >= mine.len() {
        return;
    }
    let lines: Vec<&str> = clean[pos].segment.text.lines().collect();
    let keep = lines[..lines.len() - best].join("\n").trim_end().to_string();
    let sig = lines[lines.len() - best..].join("\n");
    clean[pos].segment.text = keep;
    clean.insert(
        pos + 1,
        CleanSegment { segment: Segment { kind: SegmentKind::Signature, text: sig, depth: 0 }, hidden: true, in_thread: false },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<SegmentKind> {
        split(text).into_iter().map(|s| s.kind).collect()
    }

    #[test]
    fn plain_text_is_one_segment_tidied() {
        let s = split("\n\nBonjour,\n\n\n\nMerci.\n\n");
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].text, "Bonjour,\n\nMerci.");
    }

    #[test]
    fn final_quote_with_french_attribution_is_a_tail() {
        let s = split("OK pour moi.\n\nLe 3 nov. 2026 à 10:52, Alice <a@x.org> a écrit :\n> Tu es dispo jeudi ?\n> Alice\n");
        assert_eq!(s.iter().map(|s| s.kind).collect::<Vec<_>>(), vec![SegmentKind::Text, SegmentKind::QuoteTail]);
        assert_eq!(s[0].text, "OK pour moi.");
        assert!(s[1].text.starts_with("Le 3 nov."));
        assert!(s[1].text.contains("Tu es dispo jeudi ?"));
        assert!(s[1].text.lines().all(|l| !l.starts_with('>')));
    }

    /// The line that introduces a quote, as the common clients write it in
    /// each language (`WORDINGS`): the quote under it is the history.
    #[test]
    fn attributions_in_many_languages_start_the_history() {
        for line in [
            "On Tue, 3 Nov 2026 at 10:52, Alice <a@x.org> wrote:",
            "Le 3 nov. 2026 à 10:52, Alice <a@x.org> a écrit :",
            "Am 03.11.2026 um 10:52 schrieb Alice <a@x.org>:",
            "Am Di., 3. Nov. 2026 um 10:52 Uhr schrieb Alice Martin <a@x.org>:",
            "Am 03.11.26 um 10:52 schrieb Alice:",
            "El mar, 3 nov 2026 a las 10:52, Alice (<a@x.org>) escribió:",
            "Il giorno mar 3 nov 2026 alle ore 10:52 Alice <a@x.org> ha scritto:",
            "Em ter., 3 de nov. de 2026 às 10:52, Alice <a@x.org> escreveu:",
            "Op di 3 nov 2026 om 10:52 schreef Alice <a@x.org>:",
            "Op 3 nov. 2026 om 10:52 heeft Alice <a@x.org> het volgende geschreven:",
            "Den tis 3 nov. 2026 kl 10:52 skrev Alice <a@x.org>:",
            "tir. 3. nov. 2026 kl. 10:52 skrev Alice <a@x.org>:",
            "wt., 3 lis 2026 o 10:52 Alice <a@x.org> napisał(a):",
            "W dniu 3.11.2026 o 10:52, Alice pisze:",
            "03.11.2026 10:52, Alice пишет:",
            "3 лист. 2026 р. о 10:52 Alice <a@x.org> пише:",
            "ti 3.11.2026 klo 10.52 Alice <a@x.org> kirjoitti:",
            "2026/11/03 10:52、Alice <a@x.org>のメール:",
            "2026년 11월 3일 (화) 오전 10:52, Alice <a@x.org>님이 작성:",
            "Alice <a@x.org> 於 2026年11月3日 週二 10:52 寫道：",
            "вт, 3 нояб. 2026 г. в 10:52, Alice <a@x.org>:",
            "2026年11月3日(火) 10:52 Alice <a@x.org>:",
            "Alice <a@x.org> 于2026年11月3日周二 10:52写道：",
        ] {
            let s = split(&format!("Merci.\n\n{line}\n> Bonjour\n> Alice\n"));
            assert_eq!(s.iter().map(|s| s.kind).collect::<Vec<_>>(), vec![SegmentKind::Text, SegmentKind::QuoteTail], "{line}");
            assert_eq!(s[0].text, "Merci.", "{line}");
        }
    }

    /// A sentence of the user's that ends with a colon, above a quote,
    /// stays the user's text: in doubt, a line is shown.
    #[test]
    fn a_sentence_ending_with_a_colon_is_not_an_attribution() {
        for line in ["Voici ce que tu disais :", "On the plan, two points:", "Am Ende bleibt:", "Le devis de 2026 :"] {
            let s = split(&format!("{line}\n> Bonjour\n> Alice\n"));
            assert_eq!(s[0].kind, SegmentKind::Text, "{line}");
            assert_eq!(s[0].text, line, "{line}");
        }
    }

    /// Outlook's header block and the forwarded messages, beyond English
    /// and French.
    #[test]
    fn header_blocks_and_forwards_in_other_languages() {
        let de = "Danke.\n\nVon: Alice <a@x.org>\nGesendet: Dienstag, 3. November 2026 10:52\nAn: Bob\nBetreff: Angebot\n\nalt";
        assert_eq!(kinds(de), vec![SegmentKind::Text, SegmentKind::QuoteTail]);
        let it = "Grazie.\n\nDa: Alice <a@x.org>\nInviato: martedì 3 novembre 2026 10:52\nA: Bob\nOggetto: Offerta\n\nvecchio";
        assert_eq!(kinds(it), vec![SegmentKind::Text, SegmentKind::QuoteTail]);
        let nl = "Bedankt.\n\nVan: Alice <a@x.org>\nVerzonden: dinsdag 3 november 2026 10:52\nAan: Bob\nOnderwerp: Offerte\n\noud";
        assert_eq!(kinds(nl), vec![SegmentKind::Text, SegmentKind::QuoteTail]);
        let es = "Gracias.\n\n-----Mensaje original-----\nDe: Alice\nEnviado: martes\nPara: Bob\nAsunto: Oferta\n\nviejo";
        assert_eq!(kinds(es), vec![SegmentKind::Text, SegmentKind::QuoteTail]);
        for marker in [
            "---------- Weitergeleitete Nachricht ---------",
            "---------- Mensaje reenviado ---------",
            "---------- Messaggio inoltrato ---------",
            "Anfang der weitergeleiteten Nachricht:",
            "Inizio messaggio inoltrato:",
            "Begin doorgestuurd bericht:",
        ] {
            assert!(forward_marker(marker), "{marker}");
        }
        for m in ["Von meinem iPhone gesendet", "Enviado desde mi iPhone", "Inviato da iPhone", "Verzonden vanaf mijn iPhone"] {
            assert!(mention(m), "{m}");
        }
    }

    /// The app writes the line above a quote and the forward separator in
    /// the user's language, from its String Catalog. Every language the
    /// catalog holds must write them in a form `split` recognizes, or the
    /// app's own replies would show the whole history (`localisation.md`,
    /// section 3.3). Reading the catalog makes a new language checked as
    /// soon as it is added.
    #[test]
    fn the_apps_own_quote_lines_are_recognized_in_every_language() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../apps/NiceMailer/Sources/NiceMailer/Resources/Localizable.xcstrings");
        let catalog: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let translations = |key: &str| -> Vec<(String, String)> {
            let entry = &catalog["strings"][key];
            let mut all = vec![("en".to_string(), key.to_string())];
            for (lang, l) in entry["localizations"].as_object().unwrap() {
                all.push((lang.clone(), l["stringUnit"]["value"].as_str().unwrap().to_string()));
            }
            all
        };
        let header = translations("On %@, at %@, %@ wrote:");
        assert!(header.len() > 2, "the catalog has languages");
        for (lang, format) in &header {
            // `quoteHeader` passes the date, the time and the author: name
            // and address, the address alone, or the name alone.
            for who in ["Alice Martin <a@x.org>", "a@x.org", "Alice Martin"] {
                let line = format
                    .replacen("%@", "3 Nov 2026", 1)
                    .replacen("%@", "10:52", 1)
                    .replacen("%@", who, 1)
                    .replace("%1$@", "3 Nov 2026")
                    .replace("%2$@", "10:52")
                    .replace("%3$@", who);
                let s = split(&format!("Merci.\n\n{line}\n> Bonjour\n> Alice\n"));
                assert_eq!(s.iter().map(|s| s.kind).collect::<Vec<_>>(), vec![SegmentKind::Text, SegmentKind::QuoteTail], "{lang}: {line}");
            }
        }
        for (lang, marker) in translations("---------- Forwarded message ---------") {
            assert!(forward_marker(&marker), "{lang}: {marker}");
        }
    }

    #[test]
    fn english_attribution_wrapped_over_two_lines() {
        let s = split("Sure.\n\nOn Tue, Nov 3, 2026 at 10:52 AM Alice Martin\n<alice@x.org> wrote:\n\n> Lunch?\n");
        assert_eq!(s.iter().map(|s| s.kind).collect::<Vec<_>>(), vec![SegmentKind::Text, SegmentKind::QuoteTail]);
        assert_eq!(s[0].text, "Sure.");
    }

    #[test]
    fn interleaved_quotes_are_kept_as_quotes() {
        let s = split("> Jeudi ?\nOui.\n> Quelle heure ?\n>> (après 14 h)\n10 h.\n");
        assert_eq!(
            s.iter().map(|s| s.kind).collect::<Vec<_>>(),
            vec![SegmentKind::Quote, SegmentKind::Text, SegmentKind::Quote, SegmentKind::Text]
        );
        assert_eq!(s[2].depth, 2);
        assert_eq!(s[2].text, "Quelle heure ?\n(après 14 h)");
    }

    #[test]
    fn outlook_header_block_starts_the_history() {
        let text = "Bien reçu.\n\nCordialement\n\n________________________________\nDe : Alice Martin <a@x.org>\nEnvoyé : mardi 3 novembre 2026 10:52\nÀ : Bob\nObjet : Devis\n\nBonjour Bob,\nvoici le devis.";
        let k = kinds(text);
        assert_eq!(k, vec![SegmentKind::Text, SegmentKind::QuoteTail]);
        assert!(split(text)[1].text.contains("voici le devis"));
        // Without separator line.
        let k2 = kinds("Merci.\n\nFrom: Alice\nSent: Tuesday\nTo: Bob\nSubject: Devis\n\nold text");
        assert_eq!(k2, vec![SegmentKind::Text, SegmentKind::QuoteTail]);
        // "-----Original Message-----".
        let k3 = kinds("Merci.\n\n-----Original Message-----\nFrom: Alice\nSent: Tuesday\nTo: Bob\n\nold");
        assert_eq!(k3, vec![SegmentKind::Text, SegmentKind::QuoteTail]);
    }

    #[test]
    fn signature_delimiter_and_mentions() {
        let s = split("Merci !\n\n-- \nAlice Martin\n06 12 34 56 78\n");
        assert_eq!(s.iter().map(|s| s.kind).collect::<Vec<_>>(), vec![SegmentKind::Text, SegmentKind::Signature]);
        assert!(s[1].text.contains("06 12"));
        let m = split("Je regarde.\n\nEnvoyé de mon iPhone\n");
        assert_eq!(m.iter().map(|s| s.kind).collect::<Vec<_>>(), vec![SegmentKind::Text, SegmentKind::Boilerplate]);
        let m2 = split("Yes\n\nSent from my iPad\n\nOn Mon, Bob wrote:\n> Hi");
        assert_eq!(
            m2.iter().map(|s| s.kind).collect::<Vec<_>>(),
            vec![SegmentKind::Text, SegmentKind::Boilerplate, SegmentKind::QuoteTail]
        );
    }

    #[test]
    fn confidentiality_notice_is_boilerplate() {
        let s = split("Voici le contrat.\n\nCe message et ses pièces jointes sont confidentiels et destinés exclusivement à leur destinataire.");
        assert_eq!(s.iter().map(|s| s.kind).collect::<Vec<_>>(), vec![SegmentKind::Text, SegmentKind::Boilerplate]);
    }

    #[test]
    fn forwarded_messages_are_never_a_tail() {
        let text = "Pour info.\n\n---------- Forwarded message ---------\nDe : Alice <a@x.org>\nDate: mar. 3 nov. 2026\nObjet : Devis\nÀ : Bob\n\n> cité dans l'original\nContenu transféré.";
        let k = kinds(text);
        assert_eq!(k, vec![SegmentKind::Text, SegmentKind::Forward]);
        assert!(split(text)[1].text.contains("Contenu transféré."));
    }

    #[test]
    fn crlf_and_only_quotes() {
        let s = split("Le lun. 2 nov. 2026, Bob a écrit :\r\n> Salut\r\n");
        assert_eq!(s.iter().map(|s| s.kind).collect::<Vec<_>>(), vec![SegmentKind::QuoteTail]);
    }

    #[test]
    fn quote_markers_with_spaces_count_as_depth() {
        assert_eq!(quote_depth("> > x"), (2, "x"));
        assert_eq!(quote_depth(">>x"), (2, "x"));
        assert_eq!(quote_depth("x > y"), (0, "x > y"));
    }

    #[test]
    fn normalize_ignores_wrapping_markers_and_case() {
        assert_eq!(normalize("> Hello,\n> World!"), normalize("hello world"));
    }

    // Thread level.

    #[test]
    fn a_tail_found_in_the_thread_is_marked_so() {
        let first = "Tu es dispo jeudi pour la réunion de chantier ?\nAlice";
        let second = "Oui.\n\nLe 3 nov. 2026, Alice a écrit :\n> Tu es dispo jeudi pour la réunion de chantier ?\n> Alice";
        let c = clean_thread(&[(Some("a@x"), first), (Some("b@x"), second)]);
        let tail = c[1].segments.iter().find(|s| s.segment.kind == SegmentKind::QuoteTail).unwrap();
        assert!(tail.hidden && tail.in_thread);
        assert!(!c[1].only_quotes);
        // A tail with history absent from the thread is hidden too, but says so.
        let c2 = clean_thread(&[(Some("b@x"), second)]);
        let t2 = c2[0].segments.iter().find(|s| s.segment.kind == SegmentKind::QuoteTail).unwrap();
        assert!(t2.hidden && !t2.in_thread);
    }

    #[test]
    fn a_message_without_text_quotes_nothing() {
        let first = "Tu es dispo jeudi pour la réunion de chantier ?\nAlice";
        // An empty body, blank lines, a signature alone: nothing quoted.
        for body in ["", "\n\n  \n", "-- \nBob Martin\n06 12 34 56 78"] {
            let c = clean_thread(&[(Some("a@x"), first), (Some("b@x"), body)]);
            assert!(!c[1].only_quotes, "{body:?}");
            assert!(c[1].no_text, "{body:?}");
        }
        // The first message wrote something.
        let c = clean_thread(&[(Some("a@x"), first), (Some("b@x"), "")]);
        assert!(!c[0].no_text);
        // Only the quote of the earlier message: it only quotes.
        let quote = "Le 3 nov. 2026, Alice a écrit :\n> Tu es dispo jeudi pour la réunion de chantier ?\n> Alice";
        let c = clean_thread(&[(Some("a@x"), first), (Some("b@x"), quote)]);
        assert!(c[1].only_quotes);
        assert!(!c[1].no_text);
    }

    #[test]
    fn unquoted_copies_of_an_earlier_message_are_hidden() {
        let first = "Voici le compte rendu complet de la réunion de jeudi dernier avec toutes les décisions.";
        let second = format!("Je transmets.\n\n{first}");
        let c = clean_thread(&[(Some("a@x"), first), (Some("b@x"), second.as_str())]);
        // "Je transmets." and the copy are one text segment here: the copy
        // is detected when it stands alone.
        let third = first.to_string();
        let c3 = clean_thread(&[(Some("a@x"), first), (Some("b@x"), third.as_str())]);
        assert_eq!(c3[1].segments[0].segment.kind, SegmentKind::Copy);
        assert!(c3[1].segments[0].hidden);
        assert!(c3[1].only_quotes);
        assert!(!c[1].only_quotes);
    }

    #[test]
    fn a_message_that_only_quotes_is_flagged() {
        let c = clean_thread(&[
            (Some("a@x"), "Question ?"),
            (Some("b@x"), "Le 3 nov., Alice a écrit :\n> Question ?"),
        ]);
        assert!(c[1].only_quotes);
        assert!(!c[0].only_quotes);
    }

    #[test]
    fn a_repeated_closing_is_a_signature() {
        let a = "Bonjour,\nvoici le devis.\n\nCordialement,\nAlice Martin\nAtelier Dupuis";
        let b = "Bonjour,\nle devis est signé.\n\nCordialement,\nAlice Martin\nAtelier Dupuis";
        let c = clean_thread(&[(Some("a@x"), a), (Some("b@x"), "Merci"), (Some("a@x"), b)]);
        let kinds: Vec<_> = c[2].segments.iter().map(|s| s.segment.kind).collect();
        assert_eq!(kinds, vec![SegmentKind::Text, SegmentKind::Signature]);
        assert_eq!(c[2].segments[0].segment.text, "Bonjour,\nle devis est signé.");
        assert!(c[2].segments[1].hidden);
        // The other sender's single message is untouched.
        assert_eq!(c[1].segments.len(), 1);
    }

    #[test]
    fn short_repeated_answers_are_not_copies() {
        let c = clean_thread(&[(Some("a@x"), "OK pour moi."), (Some("b@x"), "OK pour moi.")]);
        assert_eq!(c[1].segments[0].segment.kind, SegmentKind::Text);
        assert!(!c[1].only_quotes);
    }

    #[test]
    fn a_quote_of_an_answer_after_an_interleaved_quote_is_in_the_thread() {
        let first = "> Jeudi ?\nParfait, à jeudi. J'apporte les plans imprimés.";
        let second = "Le 30 oct., Alice a écrit :\n> Parfait, à jeudi. J'apporte les plans imprimés.";
        let c = clean_thread(&[(Some("a@x"), first), (Some("b@x"), second)]);
        let tail = &c[1].segments[0];
        assert_eq!(tail.segment.kind, SegmentKind::QuoteTail);
        assert!(tail.in_thread);
    }
}
