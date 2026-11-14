//! Checking what the model answered (`specifications/ia.md`, section 6, "Ce
//! qu'un prompt reçoit").
//!
//! [`read`] takes the expected output, the provider's outcome and what was
//! sent (how many segments, which conversations, which targets) and gives a
//! typed [`Answer`], or `ai_invalid_output` with the reason. The reading is
//! strict on purpose: a tolerant one (accepting `"subject_lines"` for
//! `"subjects"`, a target nobody offered) would hide the drift of a prompt,
//! and an answer the core applies must be exactly what it asked for. The raw
//! answer is kept in the history anyway (`ai_run.output`), to understand a
//! refusal.
//!
//! Everything here is pure: no provider, no mail.

use nm_core::FolderRole;
use serde_json::{Map, Value};

use crate::ics::{Event, When};
use crate::prompt::Output;
use crate::provider::{AiError, AiOutcome};

/// Longest text accepted in a list answer (a subject, a quick reply), in
/// characters: far above what the prompts ask, low enough to refuse a model
/// that wrote an essay where a line was expected.
const MAX_ITEM_CHARS: usize = 2_000;
/// Longest reason of a verdict or a suggestion.
const MAX_REASON_CHARS: usize = 300;

/// A checked answer, one variant per [`Output`].
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    /// `replace`: the new text of each segment, in order, trimmed. The core
    /// puts back the whitespace that surrounded each original segment.
    Segments(Vec<String>),
    /// `insert`, `reply`, `panel`.
    Text(String),
    /// `choices`.
    Choices(Vec<String>),
    /// `subjects`.
    Subjects(Vec<String>),
    /// `summary`.
    Summary {
        summary: String,
        actions: Vec<ActionItem>,
    },
    /// `verdict`, before the floor of [`apply_floor`].
    Verdict(Verdict),
    /// `targets`: at most one suggestion per conversation sent.
    Targets(Vec<Suggestion>),
    /// `events`: the appointments of a mail, in the order of the answer;
    /// none when the mail names no date.
    Events(Vec<Event>),
}

/// One action item of a summary. `owner` and `due` may be empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionItem {
    pub task: String,
    pub owner: String,
    pub due: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Risk {
    Low,
    Medium,
    High,
}

impl Risk {
    pub fn as_str(self) -> &'static str {
        match self {
            Risk::Low => "low",
            Risk::Medium => "medium",
            Risk::High => "high",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub risk: Risk,
    pub reasons: Vec<String>,
    /// Set by [`apply_floor`] when the core raised the model's verdict.
    pub raised: bool,
}

/// A suggested target for one conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub thread: i64,
    pub target: FolderRole,
    pub reason: String,
}

/// What was sent, to check the answer against.
#[derive(Debug, Clone, Copy, Default)]
pub struct Sent<'a> {
    /// `replace`: the number of segments sent.
    pub segments: usize,
    /// `targets`: the conversations sent.
    pub threads: &'a [i64],
    /// `targets`: the targets offered.
    pub targets: &'a [FolderRole],
}

/// Checks an answer (see the module's documentation).
pub fn read(output: Output, outcome: &AiOutcome, sent: &Sent) -> Result<Answer, AiError> {
    if output.is_structured() {
        // A provider with structured output gives the JSON as a value; one
        // without gives it as text, possibly fenced.
        let value = match &outcome.structured {
            Some(v) => v.clone(),
            None => serde_json::from_str(&unfence(&outcome.text))
                .map_err(|e| invalid(format!("the answer is not JSON: {e}")))?,
        };
        let obj = value
            .as_object()
            .ok_or_else(|| invalid("the answer is not a JSON object"))?;
        return match output {
            Output::Choices => Ok(Answer::Choices(texts(obj, "choices")?)),
            Output::Subjects => Ok(Answer::Subjects(texts(obj, "subjects")?)),
            Output::Summary => summary(obj),
            Output::Verdict => verdict(obj),
            Output::Targets => targets(obj, sent),
            Output::Events => events(obj),
            _ => unreachable!("is_structured covers exactly these outputs"),
        };
    }
    let text = unfence(&outcome.text);
    match output {
        Output::Replace => Ok(Answer::Segments(segments(&text, sent.segments)?)),
        _ => {
            if text.is_empty() {
                return Err(invalid("the answer is empty"));
            }
            Ok(Answer::Text(text))
        }
    }
}

fn invalid(m: impl Into<String>) -> AiError {
    AiError::InvalidOutput(m.into())
}

/// The text without surrounding whitespace and, if the whole answer is one
/// code fence (```` ```json … ``` ````), without the fence: models add one
/// even when told not to.
pub fn unfence(s: &str) -> String {
    let t = s.trim();
    if let Some(rest) = t.strip_prefix("```")
        && let Some(inner) = rest.strip_suffix("```")
    {
        // The rest of the opening line is the fence's language (`json`).
        return match inner.split_once('\n') {
            Some((_lang, body)) => body.trim().to_string(),
            None => inner.trim().to_string(),
        };
    }
    t.to_string()
}

/// The `<segment n="1">…</segment>` elements of a `replace` answer:
/// exactly `expected` of them, numbered 1 to `expected` in order, and
/// nothing but whitespace around them.
///
/// One exception, the model correcting itself: it gives every segment, then
/// writes "Wait — I must correct that…" and gives them all again (seen with
/// a proofreading that first changed the writer's words). The answer may
/// then hold several complete sets of segments, with text between them;
/// the last set is the model's final word and is the one read, the text
/// between the sets is dropped. Everything else stays refused: text before
/// the first set or after the last, a set incomplete or out of order. The
/// result is a proposal shown as differences anyway (`ia.md`, section 6).
pub fn segments(text: &str, expected: usize) -> Result<Vec<String>, AiError> {
    let mut rest = text;
    loop {
        let (set, after) = segment_set(rest, expected)?;
        let tail = after.trim_start();
        if tail.is_empty() {
            return Ok(set);
        }
        // Text after a complete set: only a comment before another set.
        match tail.find("<segment") {
            Some(at) if tail[at..].starts_with("<segment n=\"1\">") => rest = &tail[at..],
            _ => {
                return Err(invalid(format!(
                    "text outside the segments near {:?}",
                    clip(tail, 40)
                )));
            }
        }
    }
}

/// One complete set of segments at the start of `text` (leading whitespace
/// aside), and the text after it.
fn segment_set(text: &str, expected: usize) -> Result<(Vec<String>, &str), AiError> {
    let mut out = Vec::new();
    let mut rest = text;
    while out.len() < expected {
        let before = rest.trim_start();
        if before.is_empty() {
            break;
        }
        let n = out.len() + 1;
        let open = format!("<segment n=\"{n}\">");
        let Some(after_open) = before.strip_prefix(open.as_str()) else {
            let what = if before.starts_with("<segment") {
                "a segment out of order or badly written"
            } else {
                "text outside the segments"
            };
            return Err(invalid(format!("{what} near {:?}", clip(before, 40))));
        };
        let Some(end) = after_open.find("</segment>") else {
            return Err(invalid(format!("segment {n} is not closed")));
        };
        let body = &after_open[..end];
        // A nested tag would make the reading ambiguous: the core does not
        // guess which text belongs to which segment.
        if body.contains("<segment") {
            return Err(invalid(format!("segment {n} contains another segment")));
        }
        out.push(body.trim().to_string());
        rest = &after_open[end + "</segment>".len()..];
    }
    if out.len() != expected {
        return Err(invalid(format!(
            "{} segments in the answer, {expected} sent",
            out.len()
        )));
    }
    // One segment too many, right after the set, is a count error, not a
    // second set: a second set starts again at 1.
    let tail = rest.trim_start();
    if tail.starts_with("<segment") && !tail.starts_with("<segment n=\"1\">") {
        let total = expected + tail.matches("<segment n=").count();
        return Err(invalid(format!(
            "{total} segments in the answer, {expected} sent"
        )));
    }
    Ok((out, rest))
}

/// Fails on any key of `obj` outside `allowed`: the schemas forbid them
/// (`additionalProperties: false`), and a provider that ignores the schema
/// must not get through.
fn only_keys(obj: &Map<String, Value>, allowed: &[&str], what: &str) -> Result<(), AiError> {
    match obj.keys().find(|k| !allowed.contains(&k.as_str())) {
        Some(k) => Err(invalid(format!("unexpected key {k:?} in {what}"))),
        None => Ok(()),
    }
}

/// A string field, trimmed; `required_text` refuses an empty one.
fn string(
    obj: &Map<String, Value>,
    key: &str,
    max: usize,
    required_text: bool,
) -> Result<String, AiError> {
    let s = obj
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(format!("{key} is missing or not a string")))?
        .trim();
    if required_text && s.is_empty() {
        return Err(invalid(format!("{key} is empty")));
    }
    if s.chars().count() > max {
        return Err(invalid(format!("{key} is longer than {max} characters")));
    }
    Ok(s.to_string())
}

/// `{"<key>": ["…", …]}`: 1 to 5 distinct, non-empty texts.
fn texts(obj: &Map<String, Value>, key: &str) -> Result<Vec<String>, AiError> {
    only_keys(obj, &[key], "the answer")?;
    let list = obj
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| invalid(format!("{key} is missing or not a list")))?;
    if !(1..=5).contains(&list.len()) {
        return Err(invalid(format!(
            "{key} has {} items, 1 to 5 expected",
            list.len()
        )));
    }
    let mut out: Vec<String> = Vec::new();
    for v in list {
        let s = v
            .as_str()
            .ok_or_else(|| invalid(format!("an item of {key} is not a string")))?
            .trim();
        if s.is_empty() {
            return Err(invalid(format!("an item of {key} is empty")));
        }
        if s.chars().count() > MAX_ITEM_CHARS {
            return Err(invalid(format!(
                "an item of {key} is longer than {MAX_ITEM_CHARS} characters"
            )));
        }
        // Rust note: `to_lowercase` handles every alphabet; comparing that
        // way finds "Rendez-vous" and "rendez-vous" to be the same item.
        if out.iter().any(|o| o.to_lowercase() == s.to_lowercase()) {
            return Err(invalid(format!("{key} gives the same item twice")));
        }
        out.push(s.to_string());
    }
    Ok(out)
}

fn summary(obj: &Map<String, Value>) -> Result<Answer, AiError> {
    only_keys(obj, &["summary", "actions"], "the answer")?;
    let summary = string(obj, "summary", 4_000, true)?;
    let list = obj
        .get("actions")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("actions is missing or not a list"))?;
    if list.len() > 8 {
        return Err(invalid(format!(
            "{} action items, at most 8 expected",
            list.len()
        )));
    }
    let mut actions = Vec::new();
    for a in list {
        let a = a
            .as_object()
            .ok_or_else(|| invalid("an action item is not an object"))?;
        only_keys(a, &["task", "owner", "due"], "an action item")?;
        actions.push(ActionItem {
            task: string(a, "task", MAX_ITEM_CHARS, true)?,
            owner: string(a, "owner", 200, false)?,
            due: string(a, "due", 100, false)?,
        });
    }
    Ok(Answer::Summary { summary, actions })
}

fn verdict(obj: &Map<String, Value>) -> Result<Answer, AiError> {
    only_keys(obj, &["risk", "reasons"], "the answer")?;
    let risk = match obj.get("risk").and_then(Value::as_str) {
        Some("low") => Risk::Low,
        Some("medium") => Risk::Medium,
        Some("high") => Risk::High,
        other => {
            return Err(invalid(format!(
                "risk is {other:?}, expected low, medium or high"
            )));
        }
    };
    let list = obj
        .get("reasons")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("reasons is missing or not a list"))?;
    if !(1..=5).contains(&list.len()) {
        return Err(invalid(format!("{} reasons, 1 to 5 expected", list.len())));
    }
    let mut reasons = Vec::new();
    for r in list {
        let s = r
            .as_str()
            .ok_or_else(|| invalid("a reason is not a string"))?
            .trim();
        if s.is_empty() || s.chars().count() > MAX_REASON_CHARS {
            return Err(invalid(format!(
                "a reason is empty or longer than {MAX_REASON_CHARS} characters"
            )));
        }
        reasons.push(s.to_string());
    }
    Ok(Answer::Verdict(Verdict {
        risk,
        reasons,
        raised: false,
    }))
}

fn targets(obj: &Map<String, Value>, sent: &Sent) -> Result<Answer, AiError> {
    only_keys(obj, &["items"], "the answer")?;
    let list = obj
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("items is missing or not a list"))?;
    let mut out: Vec<Suggestion> = Vec::new();
    for item in list {
        let item = item
            .as_object()
            .ok_or_else(|| invalid("an item is not an object"))?;
        only_keys(item, &["thread", "target", "reason"], "an item")?;
        let thread = item
            .get("thread")
            .and_then(Value::as_i64)
            .ok_or_else(|| invalid("thread is missing or not an integer"))?;
        // A conversation that was not sent: the model made it up, or mixed
        // up two batches. Applying it would move a mail nobody looked at.
        if !sent.threads.contains(&thread) {
            return Err(invalid(format!("conversation {thread} was not sent")));
        }
        if out.iter().any(|s| s.thread == thread) {
            return Err(invalid(format!(
                "conversation {thread} has two suggestions"
            )));
        }
        let name = item.get("target").and_then(Value::as_str).unwrap_or("");
        let target = FolderRole::parse(name)
            .filter(|t| sent.targets.contains(t))
            .ok_or_else(|| invalid(format!("target {name:?} was not offered")))?;
        let reason = string(item, "reason", MAX_REASON_CHARS, true)?;
        out.push(Suggestion {
            thread,
            target,
            reason,
        });
    }
    // A conversation the model left out simply gets no suggestion: refusing
    // a whole batch of twenty for one gap would cost more than it protects.
    Ok(Answer::Targets(out))
}

/// `{"events": [{"title", "start", "end", "location", "details"}]}`: at
/// most 10 appointments, each with a title and a start that is a real day
/// (`ics::When::parse`); an end, when given, of the kind of its start and
/// not before it. A date the model wrote another way ("jeudi 14h", "18/11")
/// is refused rather than guessed: the file would land on the wrong day.
fn events(obj: &Map<String, Value>) -> Result<Answer, AiError> {
    only_keys(obj, &["events"], "the answer")?;
    let list = obj
        .get("events")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("events is missing or not a list"))?;
    if list.len() > 10 {
        return Err(invalid(format!("{} events, at most 10 expected", list.len())));
    }
    let mut out = Vec::new();
    for e in list {
        let e = e.as_object().ok_or_else(|| invalid("an event is not an object"))?;
        only_keys(e, &["title", "start", "end", "location", "details"], "an event")?;
        let title = string(e, "title", 200, true)?;
        let when = |key: &str| -> Result<Option<When>, AiError> {
            let s = string(e, key, 40, key == "start")?;
            if s.is_empty() {
                return Ok(None);
            }
            When::parse(&s)
                .map(Some)
                .ok_or_else(|| invalid(format!("{key} {s:?} of {title:?} is not YYYY-MM-DD or YYYY-MM-DDTHH:MM")))
        };
        let start = when("start")?.expect("start is required, so not empty");
        let end = when("end")?;
        if let Some(end) = end {
            match end.is_after(start) {
                None => return Err(invalid(format!("the end of {title:?} is not of the form of its start"))),
                // A whole day may end the day it starts (written twice);
                // a time must end after it starts.
                Some(false) if end != start || matches!(start, When::At(..)) => {
                    return Err(invalid(format!("{title:?} ends before it starts")));
                }
                _ => {}
            }
        }
        out.push(Event {
            title,
            start,
            end,
            location: string(e, "location", 300, false)?,
            details: string(e, "details", 1_000, false)?,
        });
    }
    Ok(Answer::Events(out))
}

/// One fact the core found by itself in a mail, for the floor of the
/// security verdict (`ia.md`, section 6, "Plancher du verdict de sécurité").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// Shown to the user, in the app's language.
    pub text: String,
    /// A strong sign: failed authentication, a Reply-To of another
    /// registrable domain, a link that shows a site and opens another, a
    /// link to an IP address.
    pub strong: bool,
}

/// Raises a `low` verdict to `medium` when the core found a strong sign: the
/// model may have missed the evidence, and the user must see it. The first
/// strong finding goes first among the reasons, which stay at most 5.
pub fn apply_floor(mut v: Verdict, findings: &[Finding]) -> Verdict {
    if v.risk != Risk::Low {
        return v;
    }
    let Some(f) = findings.iter().find(|f| f.strong) else {
        return v;
    };
    v.risk = Risk::Medium;
    v.raised = true;
    let mut reasons = vec![f.text.clone()];
    reasons.extend(v.reasons.into_iter().filter(|r| r != &f.text));
    reasons.truncate(5);
    v.reasons = reasons;
    v
}

fn clip(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}
