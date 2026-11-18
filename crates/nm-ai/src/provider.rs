//! What every AI provider takes and gives back (`specifications/ia.md`,
//! section 3): a request in, text fragments while it runs, then an outcome
//! or a typed error.
//!
//! The providers themselves are in [`crate::claude_code`] (the user's local
//! `claude` binary) and [`crate::fake`] (canned answers, for the tests of
//! the layers above). The API-key provider will join them.

use serde_json::Value;

/// One call to a model: everything the provider needs, already built by the
/// core. The prompt files and the inputs of a mail are turned into these
/// two strings before a provider sees them, so a provider knows nothing of
/// mails or prompts (`ia.md`, section 4: "Le message envoyé au modèle n'est
/// pas écrit dans le prompt").
#[derive(Debug, Clone, PartialEq)]
pub struct AiRequest {
    /// The system prompt, guard included.
    pub system: String,
    /// The user message: the tagged sections of the inputs.
    pub message: String,
    /// A model id (`claude-sonnet-5-5`), accepted by every provider.
    pub model: String,
    /// The JSON Schema of a structured answer; `None` for a text answer.
    pub schema: Option<Value>,
    /// Upper bound of the answer's length, when the prompt gives one.
    pub max_output_tokens: Option<u32>,
}

/// Tokens counted by the provider for one call.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    /// Every input token, cached or not: what the call cost in input.
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// A finished call.
#[derive(Debug, Clone, PartialEq)]
pub struct AiOutcome {
    /// The answer's text. For a structured answer, its JSON as text.
    pub text: String,
    /// The structured answer, when the request had a schema.
    pub structured: Option<Value>,
    pub usage: Usage,
    /// Cost in US dollars as the provider states it (`ia.md`, section 9).
    pub cost_usd: Option<f64>,
    /// `true` when the cost is billed (an API key); `false` when it is the
    /// API-price equivalent of a call made under a subscription.
    pub billed: bool,
    /// Duration of the call, as the provider measured it.
    pub duration_ms: Option<u64>,
}

/// Why a call failed. Each variant is one error code of the contract
/// (`ia.md`, section 3, table of the errors); the texts reach the user and
/// say what to do.
///
/// Rust note: `thiserror::Error` derives the standard `Error` trait and,
/// from each `#[error(...)]` attribute, the `Display` text of the variant;
/// `{0}` and `{tools:?}` refer to the variant's fields.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum AiError {
    /// No `claude` binary where we looked.
    #[error("the claude binary was not found; install Claude Code or set ai.claude_path")]
    ProviderMissing,
    /// Claude Code is installed but not signed in.
    #[error("Claude Code is not signed in; run `claude` in a terminal to sign in")]
    NotLoggedIn,
    /// The process announced a tool or an MCP server: killed before it got
    /// any content (`ia.md`, section 3, "Vérification à chaque appel").
    #[error(
        "the claude binary announced tools {tools:?} and MCP servers {mcp_servers:?}; nothing was sent to it"
    )]
    ProviderUnsafe {
        tools: Vec<String>,
        mcp_servers: Vec<String>,
    },
    /// The subscription's usage limit is reached; `resets_at` is a Unix
    /// time when the provider gives one.
    #[error("the usage limit of the Claude subscription is reached")]
    RateLimited { resets_at: Option<i64> },
    /// `ai.timeout_secs` passed.
    #[error("the AI did not answer in time")]
    Timeout,
    /// The provider's output could not be read: not JSON, no result, no
    /// structured output where a schema asked for one.
    #[error("the AI's answer could not be read: {0}")]
    InvalidOutput(String),
    /// Any other failure the provider reports, with its own text.
    #[error("the AI failed: {0}")]
    Failed(String),
    /// The process could not be started or talked to.
    #[error("cannot run the claude binary: {0}")]
    Io(String),
    /// Nothing was sent: the input is empty, or cannot be sent as it is.
    /// Its reason says what to do.
    #[error("{0}")]
    BadInput(nm_api::Why),
}

impl AiError {
    /// The stable error code of the contract (`ia.md`, sections 3 and 10).
    pub fn code(&self) -> &'static str {
        match self {
            AiError::ProviderMissing => "ai_provider_missing",
            AiError::NotLoggedIn => "ai_not_logged_in",
            AiError::ProviderUnsafe { .. } => "ai_provider_unsafe",
            AiError::RateLimited { .. } => "ai_rate_limited",
            AiError::Timeout => "ai_timeout",
            AiError::InvalidOutput(_) => "ai_invalid_output",
            AiError::Failed(_) => "ai_failed",
            AiError::Io(_) => "ai_io",
            AiError::BadInput(_) => "ai_bad_input",
        }
    }

    /// Why, as the contract says it (`nm_api::reason`): the reason of the
    /// code, or for a bad input its own.
    pub fn why(&self) -> nm_api::Why {
        use nm_api::reason as r;
        match self {
            AiError::ProviderMissing => r::AI_PROVIDER_MISSING.into(),
            AiError::NotLoggedIn => r::AI_NOT_LOGGED_IN.into(),
            AiError::ProviderUnsafe { tools, mcp_servers } => {
                r::AI_PROVIDER_UNSAFE.with("tools", tools.clone()).with("mcp_servers", mcp_servers.clone())
            }
            AiError::RateLimited { resets_at } => match resets_at {
                Some(t) => r::AI_RATE_LIMITED.with("resets_at", *t),
                None => r::AI_RATE_LIMITED.into(),
            },
            AiError::Timeout => r::AI_TIMEOUT.into(),
            AiError::InvalidOutput(d) => r::AI_INVALID_OUTPUT.with("detail", d.as_str()),
            AiError::Failed(t) => r::AI_FAILED.with("provider_text", t.as_str()),
            AiError::Io(t) => r::AI_IO.with("system_text", t.as_str()),
            AiError::BadInput(why) => why.clone(),
        }
    }
}
