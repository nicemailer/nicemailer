//! Network adapters of NiceMailer (`architecture.md`, crate `nm-proto`).
//!
//! For now a single one: the one-click unsubscribe of RFC 8058. IMAP, SMTP
//! and OAuth2 will join it. This crate knows nothing of the database: it
//! talks to servers, and `nm-daemon` decides what to do with the answer.

pub mod unsubscribe;
