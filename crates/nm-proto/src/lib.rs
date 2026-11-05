//! Network adapters of NiceMailer (`architecture.md`, crate `nm-proto`).
//!
//! - `imap`: the IMAP client;
//! - `unsubscribe`: the one-click unsubscribe of RFC 8058.
//!
//! SMTP and OAuth2 will join them. This crate knows nothing of the
//! database: it talks to servers, and `nm-engine` and `nm-daemon` decide
//! what to do with the answers.

pub mod imap;
pub mod unsubscribe;
