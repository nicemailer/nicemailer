//! A provider that answers from a table, for the tests of the layers above
//! (the engine, the contract, the app through `FakeCore`): no process, no
//! network, no account.
//!
//! Every system prompt starts with `TASK=<id>` (`specifications/ia.md`,
//! section 4), and the fake answers by that id. A task it does not know
//! fails, so a test cannot pass by accident on an empty answer.

use std::collections::HashMap;

use crate::provider::{AiError, AiOutcome, AiRequest, Usage};

/// What the fake does for one task.
#[derive(Debug, Clone, PartialEq)]
pub enum FakeAnswer {
    /// Answer this text; for a request with a schema, it must be the JSON.
    Text(String),
    /// Fail with this error.
    Error(AiError),
}

/// The fake provider.
#[derive(Debug, Clone, Default)]
pub struct Fake {
    answers: HashMap<String, FakeAnswer>,
}

impl Fake {
    pub fn new() -> Self {
        Fake::default()
    }

    /// Adds an answer for a task, builder style: `Fake::new().answer("proofread", …)`.
    ///
    /// Rust note: taking `mut self` by value and returning `Self` lets calls
    /// chain without a temporary variable; `impl Into<String>` accepts a
    /// `&str` as well as a `String`.
    pub fn answer(mut self, task: &str, text: impl Into<String>) -> Self {
        self.answers
            .insert(task.to_string(), FakeAnswer::Text(text.into()));
        self
    }

    pub fn fail(mut self, task: &str, error: AiError) -> Self {
        self.answers
            .insert(task.to_string(), FakeAnswer::Error(error));
        self
    }

    /// Runs a request like a real provider: the text in fragments (one per
    /// word, to exercise streaming), then the outcome.
    pub async fn run(
        &self,
        req: &AiRequest,
        mut on_delta: impl FnMut(&str),
    ) -> Result<AiOutcome, AiError> {
        let task = task_of(&req.system).ok_or_else(|| {
            AiError::Failed("fake provider: the system prompt has no TASK= line".into())
        })?;
        let text = match self.answers.get(task) {
            // A test of sorting cannot know the ids of the conversations a
            // batch holds: `@each-thread <target>` suggests that target for
            // each `thread: N` of the message.
            Some(FakeAnswer::Text(t)) if t.starts_with("@each-thread ") => {
                each_thread(&req.message, &t["@each-thread ".len()..])
            }
            Some(FakeAnswer::Text(t)) => t.clone(),
            Some(FakeAnswer::Error(e)) => return Err(e.clone()),
            None => {
                return Err(AiError::Failed(format!(
                    "fake provider: no answer for task {task}"
                )));
            }
        };
        let structured = match &req.schema {
            Some(_) => Some(
                serde_json::from_str(&text)
                    .map_err(|e| AiError::InvalidOutput(format!("fake answer is not JSON: {e}")))?,
            ),
            None => {
                for piece in text.split_inclusive(' ') {
                    on_delta(piece);
                }
                None
            }
        };
        Ok(AiOutcome {
            usage: Usage {
                input_tokens: (req.system.len() + req.message.len()) as u64 / 4,
                output_tokens: text.len() as u64 / 4,
            },
            text,
            structured,
            cost_usd: Some(0.0),
            billed: false,
            duration_ms: Some(0),
        })
    }
}

/// The answer of `@each-thread <target>`.
fn each_thread(message: &str, target: &str) -> String {
    let items: Vec<serde_json::Value> = message
        .lines()
        .filter_map(|l| l.strip_prefix("thread: ")?.trim().parse::<i64>().ok())
        .map(|t| serde_json::json!({"thread": t, "target": target.trim(), "reason": format!("Fake reason {t}.")}))
        .collect();
    serde_json::json!({ "items": items }).to_string()
}

/// The id after `TASK=` on the first line of a system prompt.
pub fn task_of(system: &str) -> Option<&str> {
    let first = system.lines().next()?.trim();
    let id = first.strip_prefix("TASK=")?;
    // The id may be followed by parameters: `TASK=rewrite style=formal`.
    id.split_whitespace().next()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(system: &str, schema: bool) -> AiRequest {
        AiRequest {
            system: system.into(),
            message: "<draft>teh text</draft>".into(),
            model: "claude-haiku-5-5".into(),
            schema: schema.then(|| serde_json::json!({"type": "object"})),
            max_output_tokens: None,
        }
    }

    #[test]
    fn task_ids_are_read_from_the_first_line() {
        assert_eq!(task_of("TASK=proofread\nYou are…"), Some("proofread"));
        assert_eq!(task_of("TASK=rewrite style=formal\n…"), Some("rewrite"));
        assert_eq!(task_of("You are…\nTASK=proofread"), None);
    }

    #[tokio::test]
    async fn text_answers_stream_word_by_word() {
        let f = Fake::new().answer("proofread", "The text.");
        let mut got = Vec::new();
        let o = f
            .run(&req("TASK=proofread", false), |d| got.push(d.to_string()))
            .await
            .unwrap();
        assert_eq!(got, ["The ", "text."]);
        assert_eq!(o.text, "The text.");
        assert_eq!(o.structured, None);
    }

    #[tokio::test]
    async fn json_answers_are_parsed_and_not_streamed() {
        let f = Fake::new().answer("subjects", r#"{"subjects":["A"]}"#);
        let mut deltas = 0;
        let o = f
            .run(&req("TASK=subjects", true), |_| deltas += 1)
            .await
            .unwrap();
        assert_eq!(deltas, 0);
        assert_eq!(o.structured.unwrap()["subjects"][0], "A");
    }

    #[tokio::test]
    async fn each_thread_answers_for_the_conversations_sent() {
        let f = Fake::new().answer("triage", "@each-thread junk");
        let mut r = req("TASK=triage", true);
        r.message =
            "<conversations>\n<item>\nthread: 4\n</item>\n<item>\nthread: 9\n</item>".into();
        let o = f.run(&r, |_| {}).await.unwrap();
        let items = o.structured.unwrap()["items"].clone();
        assert_eq!(items[0]["thread"], 4);
        assert_eq!(items[1]["target"], "junk");
    }

    #[tokio::test]
    async fn unknown_tasks_and_set_errors_fail() {
        let f = Fake::new().fail("proofread", AiError::NotLoggedIn);
        assert_eq!(
            f.run(&req("TASK=proofread", false), |_| {}).await,
            Err(AiError::NotLoggedIn)
        );
        assert!(matches!(
            f.run(&req("TASK=other", false), |_| {}).await,
            Err(AiError::Failed(_))
        ));
    }
}
