//! AI assistance for NiceMailer (`specifications/ia.md`).
//!
//! This crate will hold the `AiProvider` trait and its implementations
//! (`claude-code`, the user's local `claude` binary; `anthropic-api`, an API
//! key), the loading and validation of prompt files, the building of the
//! message sent to the model and the validation of its answer.
//!
//! For now it only carries the recordings made in phase 0 (`tests/fixtures/
//! claude-code/`, described in the README there), which the provider's tests
//! will replay through a fake `claude` binary.
