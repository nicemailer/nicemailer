//! The `claude-code` provider: the user's own `claude` binary, signed in with
//! their Claude subscription (`specifications/ia.md`, section 3).
//!
//! NiceMailer stores no connection: Claude Code keeps its own credentials.
//! The core only starts the binary, with options that leave it no tool, and
//! checks at every call that the process says so before sending it anything
//! of the user's (see [`crate::stream`] for the reading of its output).
//!
//! # One call
//!
//! ```text
//! spawn (own process group, empty work dir, safe options)
//!   ─▶ write the probe            ─▶ read until `system/init`, check it
//!   ─▶ write `interrupt`          ─▶ read until the probe's `result`
//!   ─▶ write the real message, close stdin
//!   ─▶ read deltas until `result` ─▶ kill the group, reap the process
//! ```
//!
//! # Cancellation
//!
//! Rust note: an `async fn` does nothing until it is polled, and a caller
//! cancels it by dropping its future (`tokio::time::timeout` does exactly
//! that when time is up, and so does `tokio::select!` for the branch that
//! loses). The process must not outlive the call, so it lives inside a guard
//! whose `Drop` kills its whole process group: whatever the path out of
//! [`ClaudeCode::run`] (answer, error, timeout, cancel), nothing is left
//! running.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use crate::provider::{AiError, AiOutcome, AiRequest};
use crate::stream::{ClaudeStream, Step};

/// The first message sent, before the process has proved it has no tool. It
/// carries nothing of the user's, and its wording stays in the conversation
/// without changing the answer (an earlier "Reply with OK." made the model
/// end its answer with "OK": `ia.md`, section 3).
pub const PROBE: &str = "The content to process follows in the next message.";

/// Request id of the interrupt sent to end the probe turn.
const INTERRUPT_ID: &str = "nm-interrupt";

/// Where Claude Code is looked for when `ai.claude_path` is empty, relative
/// to the home directory or absolute. The core runs under launchd, without
/// the user's shell `PATH`, so the search is explicit (`ia.md`, section 3).
const SEARCH: &[&str] = &[
    "~/.local/bin/claude",
    "~/.claude/local/claude",
    "/opt/homebrew/bin/claude",
    "/usr/local/bin/claude",
];

/// The `claude` binary of `ai.claude_path`, or the first one found.
///
/// Rust note: `Option<&Path>` borrows the configured path without taking
/// ownership of it: the caller keeps its configuration, and this function
/// only reads it for the duration of the call.
pub fn find_binary(configured: Option<&Path>, home: &Path) -> Option<PathBuf> {
    if let Some(p) = configured {
        return is_executable(p).then(|| p.to_path_buf());
    }
    SEARCH
        .iter()
        .map(|s| match s.strip_prefix("~/") {
            Some(rest) => home.join(rest),
            None => PathBuf::from(s),
        })
        .find(|p| is_executable(p))
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    // `metadata` follows symbolic links: `~/.local/bin/claude` is a link to
    // the installed version.
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// The command-line options of a call (`ia.md`, section 3, table "L'appel").
/// Each one closes a way for the process to act or to read the user's
/// configuration; the check of [`crate::stream::check_init`] then proves
/// the result at every call.
pub fn args(req: &AiRequest) -> Vec<String> {
    let mut a: Vec<String> = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        // No built-in tool: no Bash, files, web or agents.
        "--tools",
        "",
        // No MCP server, neither the user's nor Claude Code's.
        "--strict-mcp-config",
        "--mcp-config",
        r#"{"mcpServers":{}}"#,
        // No user, project or local settings: no hooks, permissions or
        // CLAUDE.md.
        "--setting-sources",
        "",
        "--disable-slash-commands",
        // Nothing kept: no transcript, no memory.
        "--no-session-persistence",
        "--settings",
        r#"{"autoMemoryEnabled":false}"#,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    a.extend(["--model".into(), req.model.clone()]);
    a.extend(["--system-prompt".into(), req.system.clone()]);
    match &req.schema {
        Some(schema) => a.extend(["--json-schema".into(), schema.to_string()]),
        // Fragments as they come; a partial JSON answer would mean nothing
        // to the user, so structured answers are not streamed.
        None => a.push("--include-partial-messages".into()),
    }
    a
}

/// The provider. Cheap to clone; one value serves every call.
#[derive(Debug, Clone)]
pub struct ClaudeCode {
    /// The `claude` binary ([`find_binary`]).
    pub binary: PathBuf,
    /// An empty directory of the core's, always the same: Claude Code files
    /// its data by working directory, so a fixed one creates only one entry.
    pub workdir: PathBuf,
    /// `ai.timeout_secs`.
    pub timeout: Duration,
}

/// What [`ClaudeCode::status`] learns without calling the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeStatus {
    pub version: String,
    /// "Claude Pro", "Claude Max"…; `None` when the binary gives no account.
    pub subscription: Option<String>,
}

impl ClaudeCode {
    /// Runs one request. `on_delta` receives the fragments of a text answer
    /// as they come; the outcome holds the whole answer.
    ///
    /// Rust note: `impl FnMut(&str)` accepts any closure that may change
    /// what it captured (here, typically, send an event); it is a generic
    /// parameter, so the call costs nothing more than a direct function call.
    pub async fn run(
        &self,
        req: &AiRequest,
        mut on_delta: impl FnMut(&str),
    ) -> Result<AiOutcome, AiError> {
        match tokio::time::timeout(self.timeout, self.call(req, &mut on_delta)).await {
            Ok(r) => r,
            // The future of `call` is dropped here, and with it the guard
            // that kills the process group.
            Err(_) => Err(AiError::Timeout),
        }
    }

    async fn call(
        &self,
        req: &AiRequest,
        on_delta: &mut impl FnMut(&str),
    ) -> Result<AiOutcome, AiError> {
        let mut proc = Proc::spawn(&self.binary, &self.workdir, &args(req))?;
        let mut stream = ClaudeStream::new(req.schema.is_some());
        proc.write(&user_line(PROBE)).await?;
        loop {
            let line = proc.next_line().await?;
            match stream.feed(&line)? {
                Step::Continue => {}
                Step::SendInterrupt => proc.write(&interrupt_line()).await?,
                Step::SendMessage => {
                    // Only now does anything of the user's reach the process.
                    proc.write(&user_line(&req.message)).await?;
                    proc.close_stdin();
                }
                Step::Delta(t) => on_delta(&t),
                Step::Done(outcome) => {
                    proc.finish().await;
                    return Ok(outcome);
                }
            }
        }
    }

    /// The version of Claude Code and the account's subscription, from the
    /// `initialize` control request alone: no message, so no call to the
    /// model (`ia.md`, section 3, "État du fournisseur"). It does not list
    /// the tools; the check at every call does.
    pub async fn status(&self) -> Result<ClaudeStatus, AiError> {
        let ask = async {
            let req = AiRequest {
                system: String::new(),
                message: String::new(),
                model: String::new(),
                schema: None,
                max_output_tokens: None,
            };
            // The same safe options as a call, minus the model and prompt,
            // which an empty value would only confuse.
            let a: Vec<String> = args(&req)
                .into_iter()
                .take_while(|a| a != "--model")
                .collect();
            let mut proc = Proc::spawn(&self.binary, &self.workdir, &a)?;
            let ask = json!({"type": "control_request", "request_id": "nm-status",
                             "request": {"subtype": "initialize"}});
            proc.write(&ask.to_string()).await?;
            loop {
                let line = proc.next_line().await?;
                let Ok(d) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if d["type"] == "control_response" {
                    let r = &d["response"]["response"];
                    let status = ClaudeStatus {
                        version: r["claude_code_version"].as_str().unwrap_or("").to_string(),
                        subscription: r["account"]["subscriptionType"]
                            .as_str()
                            .map(str::to_string),
                    };
                    proc.finish().await;
                    return Ok(status);
                }
            }
        };
        let limit = self.timeout.min(Duration::from_secs(15));
        tokio::time::timeout(limit, ask)
            .await
            .unwrap_or(Err(AiError::Timeout))
    }
}

/// A user message in the stream-json input format.
fn user_line(content: &str) -> String {
    json!({"type": "user", "message": {"role": "user", "content": content}}).to_string()
}

fn interrupt_line() -> String {
    json!({"type": "control_request", "request_id": INTERRUPT_ID, "request": {"subtype": "interrupt"}}).to_string()
}

/// The running process, with its pipes, killed with its group when dropped.
struct Proc {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: Lines<BufReader<ChildStdout>>,
    /// The process group id, which is the child's pid: `process_group(0)`
    /// makes the child the leader of a new group.
    group: Option<i32>,
}

impl Proc {
    fn spawn(binary: &Path, workdir: &Path, args: &[String]) -> Result<Proc, AiError> {
        std::fs::create_dir_all(workdir)
            .map_err(|e| AiError::Io(format!("{}: {e}", workdir.display())))?;
        let mut child = Command::new(binary)
            .args(args)
            .current_dir(workdir)
            // Its own process group, so that killing the group reaches
            // everything it may start.
            .process_group(0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Claude Code reports its failures on stdout; stderr would only
            // fill a pipe nobody reads.
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied => {
                    AiError::ProviderMissing
                }
                _ => AiError::Io(e.to_string()),
            })?;
        let group = child.id().map(|id| id as i32);
        // Rust note: `take()` moves the pipe out of the `Option` the child
        // keeps it in, leaving `None`: from here on, `Proc` owns it.
        let stdin = child.stdin.take();
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AiError::Io("no stdout".into()))?;
        Ok(Proc {
            child,
            stdin,
            stdout: BufReader::new(stdout).lines(),
            group,
        })
    }

    async fn write(&mut self, line: &str) -> Result<(), AiError> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| AiError::Io("stdin closed".into()))?;
        stdin.write_all(line.as_bytes()).await.map_err(io)?;
        stdin.write_all(b"\n").await.map_err(io)?;
        stdin.flush().await.map_err(io)
    }

    /// Closing stdin tells the process no more message will come.
    fn close_stdin(&mut self) {
        self.stdin = None;
    }

    /// The next line of standard output; the end of it before an answer is
    /// an error, since a call always ends with a `result` line.
    async fn next_line(&mut self) -> Result<String, AiError> {
        match self.stdout.next_line().await.map_err(io)? {
            Some(l) => Ok(l),
            None => Err(AiError::InvalidOutput(
                "the claude process ended without a result".into(),
            )),
        }
    }

    /// After the answer: kill the group, then reap the process.
    ///
    /// The order matters. The group id is the child's pid, reserved as long
    /// as the child is not reaped (even dead, as a zombie). Killing after
    /// reaping could reach a new, unrelated process that got the same pid.
    async fn finish(mut self) {
        self.kill_group();
        let _ = self.child.wait().await;
    }

    fn kill_group(&mut self) {
        if let Some(g) = self.group.take() {
            // SIGKILL rather than SIGTERM: the answer is read, or the call
            // is abandoned, and the process keeps nothing worth saving
            // (no session is persisted).
            //
            // Rust note: calling a C function is `unsafe` because the
            // compiler cannot check it; `killpg` only takes two integers
            // and touches no memory of ours.
            unsafe {
                libc::killpg(g, libc::SIGKILL);
            }
        }
    }
}

impl Drop for Proc {
    /// Any other way out (an error, a timeout, a cancelled call) kills the
    /// group too. The child is not reaped here (that needs `.await`, which
    /// `Drop` cannot do); tokio reaps dropped children in the background.
    fn drop(&mut self) {
        self.kill_group();
    }
}

fn io(e: std::io::Error) -> AiError {
    AiError::Io(e.to_string())
}
