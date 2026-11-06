//! Errors of the IMAP client.
//!
//! They split in two kinds ([`ImapError::is_fatal`]):
//! - **the connection is lost or out of step** (network, time limit, TLS,
//!   a response that makes no sense, `BYE`): the session is closed and
//!   every later call fails at once. The caller connects again;
//! - **the command failed but the connection is fine** (`NO`, `BAD`, a
//!   missing extension, a response that could not be read but whose end
//!   was found): the caller can go on with the same session.
//!
//! No message ever holds the password: the server's texts are kept, but
//! the client never writes the password into an error.

/// An IMAP failure. The texts reach the user (account errors, the CLI), in
/// English like the CLI.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ImapError {
    #[error("cannot connect to {0}")]
    Connect(String),
    #[error("TLS: {0}")]
    Tls(String),
    #[error("connection lost: {0}")]
    Io(String),
    #[error("the server did not answer in time ({0})")]
    Timeout(&'static str),
    #[error("the server closed the connection")]
    Closed,
    #[error("the server closed the connection: {0}")]
    Bye(String),
    /// The server sent something that breaks the protocol (an unknown tag,
    /// an unexpected `+`, a greeting that cannot be read).
    #[error("protocol error: {0}")]
    Protocol(String),
    /// The server refused the user name or the password.
    #[error("the server refused the login: {0}")]
    AuthFailed(String),
    /// `NO`: the command failed (no such mailbox, quota…).
    #[error("{command} failed: {text}")]
    No { command: &'static str, text: String, code: Option<String> },
    /// `BAD`: the server did not understand the command.
    #[error("{command} refused by the server: {text}")]
    Bad { command: &'static str, text: String },
    /// A response could not be read, or was larger than the limit; the
    /// command ended, its result is incomplete.
    #[error("{command}: unreadable response from the server ({detail})")]
    Malformed { command: &'static str, detail: String },
    /// The server lacks what the command needs.
    #[error("the server does not support {0}")]
    Unsupported(&'static str),
    /// A command that could change the mailbox, on a session told to
    /// write nothing (`Session::forbid_writes`). Never sent.
    #[error("{0} refused: the account is read-only")]
    ReadOnly(&'static str),
    /// A call that makes no sense in the current state (a FETCH without a
    /// selected mailbox): a bug of the caller.
    #[error("{0}")]
    State(&'static str),
}

impl ImapError {
    /// Whether the session can no longer be used.
    pub fn is_fatal(&self) -> bool {
        matches!(
            self,
            ImapError::Connect(_)
                | ImapError::Tls(_)
                | ImapError::Io(_)
                | ImapError::Timeout(_)
                | ImapError::Closed
                | ImapError::Bye(_)
                | ImapError::Protocol(_)
        )
    }
}

pub type Result<T> = std::result::Result<T, ImapError>;
