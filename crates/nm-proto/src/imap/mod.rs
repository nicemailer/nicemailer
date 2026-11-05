//! The IMAP client (`plan-imap.md`, phase 2): a connection, the commands
//! the synchronization needs, and the responses sorted into plain types.
//!
//! - [`Session`] (`session.rs`): connect, log in, the commands, IDLE;
//! - `types.rs`: what the commands return, with no type of `imap-codec`;
//! - `encode.rs`: writing commands, and quoting what comes from outside;
//! - `convert.rs`: from the types of `imap-codec` to ours;
//! - `transport.rs`: TCP, TLS, STARTTLS and certificates;
//! - `utf7.rs`: modified UTF-7, the encoding of mailbox names.
//!
//! This module knows nothing of the database, the accounts or the
//! Keychain: it talks to one server for whoever calls it. `nm-engine`
//! decides what to ask and what to do with the answers.
//!
//! # A typical session
//!
//! ```no_run
//! # async fn demo() -> nm_proto::imap::Result<()> {
//! use nm_proto::imap::*;
//! let spec = ServerSpec { host: "mail.example.com".into(), port: 993, security: Security::Tls, ca_file: None };
//! let mut s = Session::connect(&spec, Timeouts::default()).await?;
//! s.login("alice", "secret").await?;
//! s.enable_sync().await?;
//! let inbox = s.select(&MailboxName::inbox(), None).await?;
//! let items = FetchItems { flags: true, size: true, header: true, ..FetchItems::default() };
//! for m in s.uid_fetch(&Uids::From(1), &items).await? {
//!     println!("{:?} {:?}", m.uid, m.size);
//! }
//! s.logout().await;
//! # let _ = inbox; Ok(()) }
//! ```

mod convert;
mod encode;
mod error;
mod session;
mod transport;
mod types;
pub mod utf7;

pub use error::{ImapError, Result};
pub use session::{BODY_CHUNK, IDLE_MAX, MAX_RESPONSE, Notice, Session, WHOLE_BODY_MAX};
pub use transport::Stream;
pub use types::*;
