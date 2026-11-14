//! AI assistance for NiceMailer (`specifications/ia.md`).
//!
//! What is here so far, phase 1 of the roadmap (`ia.md`, section 14):
//!
//! - [`provider`]: the request, outcome and errors shared by every provider;
//! - [`claude_code`]: the user's local `claude` binary, with the safety
//!   check of every call, and [`stream`], the reading of its output;
//! - [`fake`]: canned answers, for the tests of the layers above;
//! - [`Provider`]: the one the configuration chose;
//! - [`prompt`]: what a prompt is, and the system prompt built from it;
//! - [`catalog`]: the built-in prompts (`prompts/*.toml`) merged with the
//!   user's files, checked.
//!
//! The API-key provider and the building of the messages from a mail come
//! next.

pub mod catalog;
pub mod claude_code;
pub mod fake;
pub mod prompt;
pub mod provider;
pub mod stream;

pub use provider::{AiError, AiOutcome, AiRequest, Usage};

/// The provider in use, chosen by `ai.provider`.
///
/// Rust note: an `enum` with one variant per provider, rather than a trait
/// object (`Box<dyn AiProvider>`). The set of providers is closed and known
/// here, a `match` dispatches with no allocation, and the providers can keep
/// plain `async fn` methods, which a trait object cannot call.
#[derive(Debug, Clone)]
pub enum Provider {
    ClaudeCode(claude_code::ClaudeCode),
    Fake(fake::Fake),
}

impl Provider {
    /// The `ai.provider` name, recorded with each run (`ai_run.provider`).
    pub fn name(&self) -> &'static str {
        match self {
            Provider::ClaudeCode(_) => "claude-code",
            Provider::Fake(_) => "fake",
        }
    }

    /// Runs one request; `on_delta` gets the text as it comes. Dropping the
    /// returned future cancels the call.
    pub async fn run(
        &self,
        req: &AiRequest,
        on_delta: impl FnMut(&str),
    ) -> Result<AiOutcome, AiError> {
        match self {
            Provider::ClaudeCode(p) => p.run(req, on_delta).await,
            Provider::Fake(p) => p.run(req, on_delta).await,
        }
    }
}
