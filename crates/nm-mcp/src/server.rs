//! The MCP side of `nicemailer-mcp`: JSON-RPC 2.0 on the standard input
//! and output, one message per line (the stdio transport of MCP,
//! `specifications/mcp.md`, section 3).
//!
//! # Why not the Rust SDK (`rmcp`)
//!
//! The specification left the choice to the coding. What this server needs
//! is a small part of MCP: `initialize` with the negotiation of the
//! version, `tools/list`, `tools/call` (several at once), `ping`,
//! cancellation and progress notifications. Written here, that is a few
//! hundred lines whose every byte on the standard output is ours, which the
//! protocol requires (the standard output belongs to it, the log goes to
//! the standard error); the SDK brings a tree of dependencies and macros
//! for schemas we write by hand anyway, and its API is still moving (3.x
//! in November 2026, after 0.x versions). Should we need more of MCP
//! (resources, sampling), the question can be asked again.
//!
//! # Versions
//!
//! The client proposes a version in `initialize`; the server answers with
//! the same if it knows it, else with its latest, and the client decides.
//! What a version lacks is left out rather than refused: no `outputSchema`
//! nor `structuredContent` before 2025-06-18, no annotations before
//! 2025-03-26. The answer of a tool is always also in its text content (the
//! JSON, compact), for the clients that read only that.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;

use crate::tools::{self, Tools};

/// The versions of MCP this server speaks, newest first.
pub const VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// The JSON-RPC errors of the protocol.
const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// The version agreed with the client, and what it allows.
#[derive(Debug, Clone, Copy)]
struct Agreed {
    structured: bool,
    annotations: bool,
}

impl Agreed {
    fn of(version: &str) -> Agreed {
        // The versions are dates: their text order is their time order.
        Agreed { structured: version >= "2025-06-18", annotations: version >= "2025-03-26" }
    }
}

/// The state shared by the tasks of the server.
struct Shared {
    tools: Tools,
    /// Lines to write on the standard output, in order, by one task.
    out: mpsc::UnboundedSender<String>,
    agreed: Mutex<Agreed>,
    /// The tool calls running, by the id of their request, to cancel them.
    running: Mutex<HashMap<String, tokio::task::AbortHandle>>,
}

impl Shared {
    fn send(&self, v: Value) {
        // The writer stops only when the output closed: nothing to do then.
        let _ = self.out.send(v.to_string());
    }

    fn reply(&self, id: &Value, result: Value) {
        self.send(json!({"jsonrpc": "2.0", "id": id, "result": result}));
    }

    fn error(&self, id: &Value, code: i64, message: &str) {
        self.send(json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}));
    }
}

/// Serves MCP on `input` and `output` until the input ends (the client
/// closed it, or quit). The tool calls still running are then dropped:
/// nobody would read their answers.
pub async fn serve<I, O>(input: I, output: O, tools: Tools) -> anyhow::Result<()>
where
    I: AsyncBufRead + Unpin,
    O: AsyncWrite + Unpin + Send + 'static,
{
    let (out, mut rx) = mpsc::unbounded_channel::<String>();
    let writer = tokio::spawn(async move {
        let mut output = output;
        while let Some(line) = rx.recv().await {
            if output.write_all(line.as_bytes()).await.is_err() || output.write_all(b"\n").await.is_err() {
                break;
            }
            let _ = output.flush().await;
        }
    });
    let shared = Arc::new(Shared {
        tools,
        out,
        agreed: Mutex::new(Agreed::of(VERSIONS[0])),
        running: Mutex::new(HashMap::new()),
    });
    let mut lines = input.lines();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                shared.error(&Value::Null, PARSE_ERROR, &format!("not JSON: {e}"));
                continue;
            }
        };
        handle(&shared, msg);
    }
    // Dropping the shared state closes the queue once the tasks are gone:
    // abort them, then let the writer flush what was queued.
    for (_, task) in shared.running.lock().unwrap().drain() {
        task.abort();
    }
    drop(shared);
    let _ = writer.await;
    Ok(())
}

/// One message from the client.
fn handle(shared: &Arc<Shared>, msg: Value) {
    let Some(obj) = msg.as_object() else {
        // A batch (an array), allowed by MCP 2025-03-26 only and used by no
        // client known: refused rather than half supported.
        shared.error(&Value::Null, INVALID_REQUEST, "one JSON-RPC message per line is expected");
        return;
    };
    let Some(method) = obj.get("method").and_then(Value::as_str) else {
        // A response to a request of ours: the server sends none.
        return;
    };
    let params = obj.get("params").cloned().unwrap_or(Value::Null);
    let Some(id) = obj.get("id").cloned() else {
        notification(shared, method, &params);
        return;
    };
    match method {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or(VERSIONS[0]);
            let version = VERSIONS.iter().find(|v| **v == asked).copied().unwrap_or(VERSIONS[0]);
            *shared.agreed.lock().unwrap() = Agreed::of(version);
            // The client's name is declared, never checked: logged for the
            // diagnosis, never used to give a right (`mcp.md`, section 4).
            let client = params.pointer("/clientInfo/name").and_then(Value::as_str).unwrap_or("unknown");
            let client_version = params.pointer("/clientInfo/version").and_then(Value::as_str).unwrap_or("");
            eprintln!("nicemailer-mcp: initialize by {client} {client_version}, MCP {version} (asked {asked})");
            shared.reply(
                &id,
                json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": "nicemailer", "title": "NiceMailer", "version": env!("CARGO_PKG_VERSION")},
                    "instructions": tools::instructions(),
                }),
            );
        }
        "ping" => shared.reply(&id, json!({})),
        "tools/list" => {
            let a = *shared.agreed.lock().unwrap();
            let list: Vec<Value> = tools::definitions().iter().map(|d| d.to_json(a.structured, a.annotations)).collect();
            shared.reply(&id, json!({"tools": list}));
        }
        "tools/call" => call(shared, id, params),
        // No resources, prompts or completions: not announced in the
        // capabilities, so a client should not ask.
        other => shared.error(&id, METHOD_NOT_FOUND, &format!("method not found: {other}")),
    }
}

fn notification(shared: &Arc<Shared>, method: &str, params: &Value) {
    // The client no longer waits for this call: stopped, and no answer is
    // sent (the MCP rule for a cancelled request). A call already sent to
    // the core may still have run there. `notifications/initialized` and
    // the others need nothing.
    if method == "notifications/cancelled"
        && let Some(id) = params.get("requestId")
        && let Some(task) = shared.running.lock().unwrap().remove(&key(id))
    {
        task.abort();
    }
}

/// The key of a request id in [`Shared::running`]: ids are strings or
/// numbers, compared as their JSON.
fn key(id: &Value) -> String {
    id.to_string()
}

/// `tools/call`: runs the tool in a task of its own, so that the server
/// keeps reading (a `ping`, a cancellation, another call) meanwhile.
fn call(shared: &Arc<Shared>, id: Value, params: Value) {
    let Some(name) = params.get("name").and_then(Value::as_str).map(String::from) else {
        shared.error(&id, INVALID_PARAMS, "tools/call needs the name of a tool");
        return;
    };
    if !tools::definitions().iter().any(|d| d.name == name) {
        shared.error(&id, INVALID_PARAMS, &format!("unknown tool: {name}"));
        return;
    }
    let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);
    let token = params.pointer("/_meta/progressToken").cloned();
    let s = shared.clone();
    let task_id = id.clone();
    // The task registers itself only once spawned; the lock below is taken
    // before it can finish, since it is held while spawning.
    let mut running = shared.running.lock().unwrap();
    let handle = tokio::spawn(async move {
        // Progress notifications, when the client gave a token for them:
        // what the call waits for (the core starting, an upgrade).
        let progress_out = s.out.clone();
        let step = std::sync::atomic::AtomicU64::new(0);
        let progress = move |message: &str| {
            if let Some(t) = &token {
                let n = step.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                let v = json!({"jsonrpc": "2.0", "method": "notifications/progress",
                               "params": {"progressToken": t, "progress": n, "message": message}});
                let _ = progress_out.send(v.to_string());
            }
        };
        let result = s.tools.call(&name, arguments, &progress).await.expect("the tool exists: checked above");
        let structured = s.agreed.lock().unwrap().structured;
        let answer = match result {
            Ok(v) => {
                let mut r = json!({"content": [{"type": "text", "text": v.to_string()}], "isError": false});
                if structured {
                    r["structuredContent"] = v;
                }
                r
            }
            Err(e) => {
                eprintln!("nicemailer-mcp: {name}: {}", e.0);
                json!({"content": [{"type": "text", "text": e.0}], "isError": true})
            }
        };
        // Answered only if not cancelled meanwhile.
        if s.running.lock().unwrap().remove(&key(&task_id)).is_some() {
            s.reply(&task_id, answer);
        }
    });
    running.insert(key(&id), handle.abort_handle());
}
