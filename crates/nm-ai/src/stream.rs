//! Reading what the `claude` binary writes, one JSON line at a time
//! (`specifications/ia.md`, section 3, "Vérification à chaque appel" and
//! "Résultat, coût et erreurs").
//!
//! This is a pure state machine: [`ClaudeStream::feed`] takes one line and
//! says what the caller must do next. It starts no process and does no I/O,
//! so the tests run it straight on the recordings of phase 0
//! (`tests/fixtures/claude-code/`), and the process driver
//! ([`crate::claude_code`]) only moves bytes.
//!
//! The call goes through three phases:
//!
//! ```text
//! AwaitInit ──init checked──▶ AwaitProbeEnd ──probe's result──▶ Answer ──result──▶ done
//!     │                            │                               │
//!     └─ init with a tool: ProviderUnsafe (the mail was never sent)
//! ```

use serde_json::Value;

use crate::provider::{AiError, AiOutcome, Usage};

/// The only tool accepted, and only when the request has a schema: the
/// internal tool by which Claude Code returns a structured answer. It acts on
/// nothing outside the conversation.
pub const STRUCTURED_OUTPUT_TOOL: &str = "StructuredOutput";

/// Where the call is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// The probe is sent; waiting for the process to announce itself.
    AwaitInit,
    /// The announcement was safe and the probe is being interrupted.
    AwaitProbeEnd,
    /// The real message is sent; reading the answer.
    Answer,
}

/// What the caller must do after a line.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// Nothing; read the next line.
    Continue,
    /// The process proved it has no tool: interrupt the probe turn.
    SendInterrupt,
    /// The probe turn is over: send the real message.
    SendMessage,
    /// A fragment of the answer's text, to show as it comes.
    Delta(String),
    /// The answer is complete.
    Done(AiOutcome),
}

/// The state of one call. Create one per process.
#[derive(Debug)]
pub struct ClaudeStream {
    phase: Phase,
    /// Whether `--json-schema` was passed, which adds one allowed tool.
    with_schema: bool,
    /// An assistant line said `"error": "authentication_failed"`.
    auth_failed: bool,
    /// The last `rate_limit_event` said the limit is reached; the inner
    /// value is when it resets, if given.
    ///
    /// Rust note: `Option<Option<i64>>` keeps two facts apart: the outer
    /// `None` means "no limit reached", `Some(None)` "reached, reset time
    /// unknown".
    rate_limited: Option<Option<i64>>,
}

impl ClaudeStream {
    pub fn new(with_schema: bool) -> Self {
        ClaudeStream {
            phase: Phase::AwaitInit,
            with_schema,
            auth_failed: false,
            rate_limited: None,
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// Reads one line of the process's standard output.
    ///
    /// Rust note: the return type `Result<Step, AiError>` makes failure part
    /// of the signature: the caller cannot get a `Step` without handling
    /// the error case, usually by passing it on with `?`.
    pub fn feed(&mut self, line: &str) -> Result<Step, AiError> {
        let line = line.trim();
        if line.is_empty() {
            return Ok(Step::Continue);
        }
        let d: Value = serde_json::from_str(line)
            .map_err(|e| AiError::InvalidOutput(format!("not a JSON line: {e}")))?;
        let kind = d["type"].as_str().unwrap_or("");
        let subtype = d["subtype"].as_str().unwrap_or("");
        match (kind, subtype) {
            ("system", "init") => self.on_init(&d),
            ("assistant", _) => {
                // A failure of the account comes as an assistant line with an
                // `error` field, before the turn's result.
                if d["error"] == "authentication_failed" {
                    self.auth_failed = true;
                }
                Ok(Step::Continue)
            }
            ("rate_limit_event", _) => {
                let info = &d["rate_limit_info"];
                // Any state other than "allowed" counts as reached: a new
                // state name must not let a call pass as if all was well.
                self.rate_limited =
                    (info["status"] != "allowed").then(|| info["resetsAt"].as_i64());
                Ok(Step::Continue)
            }
            ("stream_event", _) if self.phase == Phase::Answer => {
                let ev = &d["event"];
                // Rust note: `if let … && …` (a "let chain", Rust 2024)
                // tests a pattern and conditions in one `if`.
                if ev["type"] == "content_block_delta"
                    && ev["delta"]["type"] == "text_delta"
                    && let Some(t) = ev["delta"]["text"].as_str()
                {
                    return Ok(Step::Delta(t.to_string()));
                }
                Ok(Step::Continue)
            }
            ("result", _) => self.on_result(&d),
            _ => Ok(Step::Continue),
        }
    }

    fn on_init(&mut self, d: &Value) -> Result<Step, AiError> {
        // Every turn announces itself again: the second announcement is
        // checked too, as a second line of defence.
        check_init(d, self.with_schema)?;
        match self.phase {
            Phase::AwaitInit => {
                self.phase = Phase::AwaitProbeEnd;
                Ok(Step::SendInterrupt)
            }
            _ => Ok(Step::Continue),
        }
    }

    fn on_result(&mut self, d: &Value) -> Result<Step, AiError> {
        if self.auth_failed {
            return Err(AiError::NotLoggedIn);
        }
        let is_error = d["is_error"].as_bool().unwrap_or(true);
        let text = d["result"].as_str().unwrap_or("").to_string();
        match self.phase {
            // A result before any announcement: the process cannot be trusted
            // to have no tool, and nothing was checked.
            Phase::AwaitInit => Err(AiError::InvalidOutput(
                "a result came before the init announcement".into(),
            )),
            Phase::AwaitProbeEnd => {
                // The interrupted probe ends with `error_during_execution`; a
                // probe that ran to its end is harmless too. Any other error
                // is the account's or the process's.
                if is_error && d["subtype"] != "error_during_execution" {
                    return Err(self.failure(text));
                }
                self.phase = Phase::Answer;
                Ok(Step::SendMessage)
            }
            Phase::Answer => {
                if is_error {
                    return Err(self.failure(text));
                }
                let structured = d.get("structured_output").cloned();
                if self.with_schema && structured.is_none() {
                    // The doc: "If a run ends without valid structured output,
                    // for example because Claude declined the request, the
                    // response has no structured_output field."
                    return Err(AiError::InvalidOutput(format!(
                        "no structured output; the model said: {}",
                        clip(&text, 200)
                    )));
                }
                Ok(Step::Done(outcome(d, text, structured)))
            }
        }
    }

    /// The error of a failed turn: the usage limit if the last rate-limit
    /// event said so, else the provider's own text.
    fn failure(&self, text: String) -> AiError {
        match self.rate_limited {
            Some(resets_at) => AiError::RateLimited { resets_at },
            None => AiError::Failed(if text.is_empty() {
                "no message".into()
            } else {
                clip(&text, 300)
            }),
        }
    }
}

/// The safety check of an announcement (`ia.md`, section 3): the tool list
/// is exactly empty, or exactly `["StructuredOutput"]` with a schema, and
/// there is no MCP server. A missing or malformed list fails: only a process
/// that proves it has no tool gets the content.
pub fn check_init(init: &Value, with_schema: bool) -> Result<(), AiError> {
    let (Some(tools), Some(servers)) = (init["tools"].as_array(), init["mcp_servers"].as_array())
    else {
        return Err(AiError::ProviderUnsafe {
            tools: vec!["(no tool list announced)".into()],
            mcp_servers: vec![],
        });
    };
    // A non-string entry is kept, as its JSON text, so that it fails the
    // comparison below instead of being skipped.
    let mut tools: Vec<String> = tools
        .iter()
        .map(|t| {
            t.as_str()
                .map(str::to_string)
                .unwrap_or_else(|| t.to_string())
        })
        .collect();
    tools.sort();
    let servers: Vec<String> = servers
        .iter()
        .map(|s| {
            s["name"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| s.to_string())
        })
        .collect();
    let allowed: &[&str] = if with_schema {
        &[STRUCTURED_OUTPUT_TOOL]
    } else {
        &[]
    };
    if tools != allowed || !servers.is_empty() {
        return Err(AiError::ProviderUnsafe {
            tools,
            mcp_servers: servers,
        });
    }
    Ok(())
}

/// The outcome of a successful `result` line.
fn outcome(d: &Value, text: String, structured: Option<Value>) -> AiOutcome {
    let u = &d["usage"];
    let n = |k: &str| u[k].as_u64().unwrap_or(0);
    AiOutcome {
        text,
        structured,
        usage: Usage {
            // Cached input is still input: the cost of a call is the sum.
            input_tokens: n("input_tokens")
                + n("cache_creation_input_tokens")
                + n("cache_read_input_tokens"),
            output_tokens: n("output_tokens"),
        },
        cost_usd: d["total_cost_usd"].as_f64(),
        // Under a subscription, the stated cost is an API-price equivalent,
        // not a bill (`ia.md`, section 3, "Coût").
        billed: false,
        duration_ms: d["duration_ms"].as_u64(),
    }
}

/// The first `max` characters of a text, for error messages.
fn clip(s: &str, max: usize) -> String {
    // Rust note: strings are UTF-8, and slicing at a byte offset inside a
    // character panics; `chars()` walks whole characters instead.
    s.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn init(tools: Value, mcp: Value) -> Value {
        json!({"type": "system", "subtype": "init", "tools": tools, "mcp_servers": mcp})
    }

    #[test]
    fn empty_lists_pass_without_schema() {
        assert_eq!(check_init(&init(json!([]), json!([])), false), Ok(()));
    }

    #[test]
    fn structured_output_passes_only_with_a_schema() {
        let i = init(json!(["StructuredOutput"]), json!([]));
        assert_eq!(check_init(&i, true), Ok(()));
        assert!(matches!(
            check_init(&i, false),
            Err(AiError::ProviderUnsafe { .. })
        ));
    }

    #[test]
    fn any_other_tool_fails() {
        for tools in [
            json!(["Bash"]),
            json!(["StructuredOutput", "Read"]),
            json!([42]),
        ] {
            assert!(
                matches!(
                    check_init(&init(tools.clone(), json!([])), true),
                    Err(AiError::ProviderUnsafe { .. })
                ),
                "{tools}"
            );
        }
    }

    #[test]
    fn an_mcp_server_fails_and_is_named() {
        let i = init(json!([]), json!([{"name": "gmail", "status": "connected"}]));
        assert_eq!(
            check_init(&i, false),
            Err(AiError::ProviderUnsafe {
                tools: vec![],
                mcp_servers: vec!["gmail".into()]
            })
        );
    }

    #[test]
    fn missing_lists_fail() {
        for i in [
            json!({"type": "system", "subtype": "init"}),
            init(json!("none"), json!([])),
        ] {
            assert!(matches!(
                check_init(&i, false),
                Err(AiError::ProviderUnsafe { .. })
            ));
        }
    }

    #[test]
    fn a_result_before_init_is_refused() {
        let mut s = ClaudeStream::new(false);
        let r = s.feed(r#"{"type":"result","subtype":"success","is_error":false,"result":"hi"}"#);
        assert!(matches!(r, Err(AiError::InvalidOutput(_))));
    }

    #[test]
    fn a_line_that_is_not_json_is_refused() {
        let mut s = ClaudeStream::new(false);
        assert!(matches!(
            s.feed("Segmentation fault"),
            Err(AiError::InvalidOutput(_))
        ));
    }

    #[test]
    fn deltas_before_the_answer_are_ignored() {
        // A fragment of the probe's turn must never reach the user.
        let mut s = ClaudeStream::new(false);
        let delta = r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"x"}}}"#;
        assert_eq!(s.feed(delta), Ok(Step::Continue));
    }

    #[test]
    fn a_reached_limit_turns_a_failure_into_rate_limited() {
        let mut s = ClaudeStream::new(false);
        s.feed(r#"{"type":"system","subtype":"init","tools":[],"mcp_servers":[]}"#)
            .unwrap();
        s.feed(r#"{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","resetsAt":1794657600}}"#)
            .unwrap();
        let r = s.feed(r#"{"type":"result","subtype":"success","is_error":true,"result":"limit"}"#);
        assert_eq!(
            r,
            Err(AiError::RateLimited {
                resets_at: Some(1794657600)
            })
        );
    }

    #[test]
    fn a_missing_structured_output_is_invalid() {
        let mut s = ClaudeStream::new(true);
        for l in [
            r#"{"type":"system","subtype":"init","tools":["StructuredOutput"],"mcp_servers":[]}"#,
            r#"{"type":"result","subtype":"error_during_execution","is_error":true}"#,
        ] {
            s.feed(l).unwrap();
        }
        let r =
            s.feed(r#"{"type":"result","subtype":"success","is_error":false,"result":"I won't."}"#);
        assert!(matches!(r, Err(AiError::InvalidOutput(m)) if m.contains("I won't.")));
    }
}
