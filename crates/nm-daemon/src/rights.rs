//! What each source may ask the core (`specifications/mcp.md`, section 4,
//! "Droits"; `architecture.md`, section 9).
//!
//! For now only the source `mcp`, an AI agent speaking through
//! `nicemailer-mcp`, is limited here: it gets the methods of an
//! **allowlist**, and everything else is refused with `FORBIDDEN` before
//! the method runs. A list of what is allowed rather than of what is
//! refused: a method added to the contract is closed to the agent until
//! someone opens it here, on purpose. The test
//! `every_contract_method_outside_the_list_is_refused` reads every method
//! of `nm_api::method` to hold that.
//!
//! The rights are the core's, not the proxy's: `nicemailer-mcp` only shows
//! the tools these methods allow, but a modified proxy, or another program
//! declaring itself `mcp` in `hello`, gets nothing more.
//!
//! The source comes from the client's code signature, verified at the
//! connect (`signing.rs`; `architecture.md`, section 9): only a debug
//! build, which verifies nothing, takes it from what the client declares
//! in `hello`. That is why sending by an agent (phase 3 of `mcp.md`) is
//! for release builds only.
//!
//! The update of NiceMailer (`prepare_update`, `cancel_update`) is reserved
//! to the app, which Sparkle updates (`mise-a-jour.md`, section 3).
//!
//! Finer rules live with the methods they limit: an agent changes only its
//! own drafts (`drafts.rs`, `writer`), never saves a password
//! (`set_password`).

use nm_api::reason;
use nm_api::{RpcError, error_code, method};
use nm_core::Source;

/// The methods an AI agent may call (`mcp.md`, section 10, phases 1 and
/// 2): reading the mailbox, and writing its own drafts. `ping`, `hello`
/// and `subscribe` are the connection's own and never reach this check.
///
/// Deliberately not here, with the reason the agent is given
/// ([`refusal`]): sending (phase 3, after the signature check), filing
/// (phase 4, behind a protected setting), screening decisions without a
/// mail, unsubscribing and remote images (requests to addresses the sender
/// chose), accounts, passwords, settings, import, synchronization, the raw
/// store, the AI assistance (the agent is a model itself), and
/// `prepare_draft` (the screen of the app; the agent writes at once).
pub(crate) const MCP_ALLOWED: &[&str] = &[
    // Reading (phase 1).
    method::LIST_ACCOUNTS,
    method::LIST_FOLDERS,
    method::LIST_TAGS,
    method::LIST_THREADS,
    method::GET_THREAD,
    method::SEARCH,
    method::LOCATE_MESSAGE,
    method::MESSAGE_TEXT,
    method::GET_ATTACHMENT,
    method::ATTACHMENT_TEXT,
    method::SUGGEST_RECIPIENTS,
    method::LIST_HISTORY,
    // The bounds of the answers, `[mcp]` of config.toml.
    method::CONFIG_STATUS,
    // Drafts (phase 2): its own only (`drafts.rs`).
    method::PARSE_ADDRESSES,
    method::LIST_SENDERS,
    method::CREATE_DRAFT,
    method::OPEN_DRAFT,
    method::UPDATE_DRAFT,
    method::ADD_DRAFT_ATTACHMENT,
    method::REMOVE_DRAFT_ATTACHMENT,
    method::CLOSE_DRAFT,
    method::DISCARD_DRAFT,
];

/// Whether `source` may call `method`; the refusal otherwise.
pub(crate) fn check(source: Source, method: &str) -> Result<(), RpcError> {
    match source {
        Source::Mcp if !MCP_ALLOWED.contains(&method) => Err(RpcError::new(error_code::FORBIDDEN, refusal(method))),
        // The update of NiceMailer is the app's: it is the one Sparkle
        // replaces (`mise-a-jour.md`, section 3). Another client that put
        // the core in maintenance would only lock everyone out.
        Source::Cli | Source::Tui | Source::Web if UPDATE_METHODS.contains(&method) => {
            Err(RpcError::new(error_code::FORBIDDEN, reason::RESERVED_TO_APP.with("method", method)))
        }
        _ => Ok(()),
    }
}

/// The methods of the update (`update.rs`), reserved to the app.
const UPDATE_METHODS: &[&str] = &[method::PREPARE_UPDATE, method::CANCEL_UPDATE];

/// Why an agent is refused, its English sentence written for the agent to
/// act on or to tell the user: what it may do instead, where a right is
/// given.
fn refusal(m: &str) -> nm_api::Why {
    match m {
        method::SEND_DRAFT | method::CANCEL_SEND => reason::AGENT_CANNOT_SEND.into(),
        method::MOVE_THREAD
        | method::MOVE_THREADS
        | method::SET_THREAD_SEEN
        | method::SET_THREAD_FLAGGED
        | method::TAG_THREAD
        | method::UNDO => reason::AGENT_CANNOT_FILE.into(),
        method::UNSUBSCRIBE => reason::AGENT_CANNOT_UNSUBSCRIBE.into(),
        method::SET_PASSWORD => reason::AGENT_CANNOT_SET_PASSWORD.into(),
        _ => reason::AGENT_NOT_ALLOWED.with("method", m),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The methods of `nm_api::method`, read from its source as the test of
    /// `docs/contract.md` does (`nm-cli/src/contract_doc.rs`): a list kept
    /// in `nm-api` would be one more thing to keep up to date, and the
    /// point is to catch a method nobody thought of.
    fn contract_methods() -> Vec<String> {
        const API: &str = include_str!("../../nm-api/src/lib.rs");
        let start = API.find("pub mod method {").expect("nm-api has no `pub mod method`");
        let module = &API[start..];
        let end = module.find("\n}").expect("`pub mod method` is not closed");
        module[..end]
            .lines()
            .filter_map(|l| l.trim().strip_prefix("pub const "))
            .map(|rest| rest.split('"').nth(1).expect("a method constant without a string value").to_string())
            .collect()
    }

    /// The connection's own methods, answered before any check.
    const SESSION: [&str; 3] = [method::PING, method::HELLO, method::SUBSCRIBE];

    #[test]
    fn every_contract_method_outside_the_list_is_refused() {
        let methods = contract_methods();
        assert!(methods.len() > 60, "the methods of nm-api were not read: {}", methods.len());
        for m in &methods {
            if SESSION.contains(&m.as_str()) {
                continue;
            }
            let allowed = MCP_ALLOWED.contains(&m.as_str());
            match check(Source::Mcp, m) {
                Ok(()) => assert!(allowed, "{m} is open to mcp without being in the list"),
                Err(e) => {
                    assert!(!allowed, "{m} is in the list but refused");
                    assert_eq!(e.code, error_code::FORBIDDEN, "{m}");
                }
            }
            // The app may call everything; the CLI and the TUI everything
            // but the update.
            assert!(check(Source::App, m).is_ok(), "{m} refused to the app");
            for source in [Source::Cli, Source::Tui] {
                assert_eq!(check(source, m).is_ok(), !UPDATE_METHODS.contains(&m.as_str()), "{m} for {source:?}");
            }
        }
        // Every method of the list exists: a typo would open nothing and
        // close a tool.
        for m in MCP_ALLOWED {
            assert!(methods.iter().any(|c| c == m), "{m} is not a method of the contract");
        }
    }

    #[test]
    fn what_an_agent_must_never_do_is_refused() {
        // `mcp.md`, section 12: passwords, sending, filing, unsubscribing,
        // accounts, settings, the AI assistance.
        for m in [
            method::SET_PASSWORD,
            method::SEND_DRAFT,
            method::CANCEL_SEND,
            method::MOVE_THREAD,
            method::MOVE_THREADS,
            method::SET_ADDRESS_SCREENING,
            method::SET_DOMAIN_SCREENING,
            method::SET_SENDER_KIND,
            method::UNSUBSCRIBE,
            method::ADD_ACCOUNT,
            method::REMOVE_ACCOUNT,
            method::IMPORT_MBOX,
            method::SYNC_NOW,
            method::RAW_GC,
            method::AI_RUN,
            method::EXPORT_MESSAGE,
            method::PREPARE_DRAFT,
        ] {
            assert_eq!(check(Source::Mcp, m).unwrap_err().code, error_code::FORBIDDEN, "{m}");
        }
        // A method that does not exist is refused too, not routed.
        assert!(check(Source::Mcp, "drop_database").is_err());
    }

    #[test]
    fn only_the_app_prepares_or_cancels_an_update() {
        for m in UPDATE_METHODS {
            assert!(check(Source::App, m).is_ok());
            for source in [Source::Cli, Source::Tui, Source::Web, Source::Mcp] {
                assert_eq!(check(source, m).unwrap_err().code, error_code::FORBIDDEN, "{m} for {source:?}");
            }
        }
    }

    #[test]
    fn a_refusal_says_what_to_do_instead() {
        assert!(refusal(method::SEND_DRAFT).message().contains("Drafts"));
        assert!(refusal(method::MOVE_THREADS).message().contains("ask the user"));
        assert_eq!(refusal(method::ADD_ACCOUNT).message(), "add_account is not available to AI agents");
    }
}
