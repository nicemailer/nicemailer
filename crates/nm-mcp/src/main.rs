//! `nicemailer-mcp`: started by an MCP client (Claude Desktop, Claude
//! Code), it speaks MCP on its standard input and output (`docs/mcp.md`).
//! Its log goes to the standard error, which Claude Desktop keeps in
//! `~/Library/Logs/Claude/mcp-server-nicemailer.log`: the standard output
//! belongs to the protocol, and nothing else is ever written there.
//!
//! `nicemailer-mcp claude-desktop install|status|remove` writes the entry
//! of NiceMailer in Claude Desktop's configuration; the app calls it
//! (`nm_mcp::install`).

use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// MCP server of NiceMailer: lets an AI agent read the mailbox and prepare drafts.
#[derive(Parser)]
#[command(name = "nicemailer-mcp", version)]
struct Args {
    /// Data directory of the mailbox; by default the one chosen in NiceMailer.
    #[arg(long, env = "NICEMAILER_DATA_DIR")]
    data_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Manages the entry of NiceMailer in Claude Desktop's configuration.
    ClaudeDesktop {
        #[command(subcommand)]
        action: DesktopAction,
        /// Configuration file of Claude Desktop (default: ~/Library/Application Support/Claude/claude_desktop_config.json).
        #[arg(long, global = true)]
        config: Option<PathBuf>,
        /// The nicemailer-mcp binary the entry runs (default: this one).
        #[arg(long, global = true)]
        binary: Option<PathBuf>,
        /// Prints the outcome as JSON, for the app.
        #[arg(long, global = true)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum DesktopAction {
    /// Adds the entry, or points it to this binary.
    Install {
        /// Only updates an entry that exists (the app was moved).
        #[arg(long)]
        repair: bool,
    },
    /// Says whether the entry exists and runs this binary.
    Status,
    /// Removes the entry.
    Remove,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    match args.command {
        None => serve(args.data_dir),
        Some(Command::ClaudeDesktop { action, config, binary, json }) => {
            let config = config.unwrap_or_else(nm_mcp::install::default_config);
            let binary = match binary {
                Some(b) => b,
                None => std::env::current_exe()?.canonicalize()?,
            };
            desktop(action, &config, &binary, json)
        }
    }
}

/// Serves MCP on stdio until the client closes it.
fn serve(data_dir: Option<PathBuf>) -> anyhow::Result<()> {
    // Rust note: a runtime built by hand rather than `#[tokio::main]`, so
    // that the commands of Claude Desktop's configuration do not start one.
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()?;
    runtime.block_on(async {
        let link = nm_mcp::core::CoreLink::new(data_dir, nm_mcp::core::Waits::default());
        let tools = nm_mcp::tools::Tools::new(link);
        let input = tokio::io::BufReader::new(tokio::io::stdin());
        nm_mcp::server::serve(input, tokio::io::stdout(), tools).await
    })
}

fn desktop(action: DesktopAction, config: &std::path::Path, binary: &std::path::Path, json: bool) -> anyhow::Result<()> {
    use nm_mcp::install::{self, Outcome, Status};
    let path = config.display().to_string();
    let (outcome, message): (&str, String) = match action {
        DesktopAction::Install { repair } => match install::install(config, binary, repair) {
            Ok(Outcome::Added) => ("added", format!("NiceMailer added to {path}: restart Claude Desktop")),
            Ok(Outcome::Updated) => ("updated", format!("NiceMailer's entry in {path} now runs {}: restart Claude Desktop", binary.display())),
            Ok(Outcome::Unchanged) => ("unchanged", format!("NiceMailer is already set up in {path}")),
            Ok(Outcome::NotInstalled) => ("not_installed", "NiceMailer is not set up in Claude Desktop: nothing to repair".into()),
            Err(e) => ("error", e),
        },
        DesktopAction::Status => match install::status(config, binary) {
            Status::Absent => ("absent", format!("NiceMailer is not set up in {path}")),
            Status::Configured => ("configured", format!("NiceMailer is set up in {path}")),
            Status::Elsewhere(c) => ("elsewhere", format!("NiceMailer's entry in {path} runs another binary: {c}")),
            Status::Unreadable(e) => ("error", e),
        },
        DesktopAction::Remove => match install::remove(config) {
            Ok(true) => ("removed", format!("NiceMailer removed from {path}: restart Claude Desktop")),
            Ok(false) => ("absent", format!("NiceMailer was not set up in {path}")),
            Err(e) => ("error", e),
        },
    };
    if json {
        println!("{}", serde_json::json!({"outcome": outcome, "message": message, "config": path, "entry": install::entry(binary)}));
    } else if outcome == "error" {
        eprintln!("error: {message}");
    } else {
        println!("{message}");
    }
    if outcome == "error" {
        std::process::exit(1);
    }
    Ok(())
}
