//! `nicemailer-mcp`: the MCP server through which an AI agent (Claude
//! Desktop, Claude Code, any MCP client) reads the mailbox and prepares
//! drafts (`specifications/mcp.md`; user documentation: `docs/mcp.md`).
//!
//! A thin adapter: it turns the tools of MCP into calls of the core's
//! contract (`nm-api`) and holds no rule of the mailbox. The core knows the
//! client as the source `mcp` and decides what it may do (`nm-daemon`,
//! `rights.rs`).
//!
//! - `server`: MCP on the standard input and output;
//! - `tools`: the tools, their schemas, the calls they make;
//! - `reduce`: the answers reduced from the views of the contract;
//! - `core`: the connection to the core (lazy, started on demand,
//!   maintenance);
//! - `time`: ISO 8601 dates at local time;
//! - `install`: the entry of NiceMailer in Claude Desktop's configuration.
//!
//! The binary is `src/main.rs`; the end-to-end tests, against the real
//! core, are in `nm-daemon/tests/mcp_proxy.rs`.

pub mod core;
pub mod install;
pub mod reduce;
pub mod server;
pub mod time;
pub mod tools;
