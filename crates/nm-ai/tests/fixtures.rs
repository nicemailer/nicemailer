//! Checks that the phase 0 recordings (`tests/fixtures/claude-code/`) show
//! what their README says. The provider's tests will replay them; a
//! recording that does not show what it claims would make those tests prove
//! nothing.
//!
//! Rust note: every file in `tests/` is an *integration test*, compiled as a
//! separate crate that only sees the public API of `nm-ai`. Here it does not
//! even use it: it reads JSON lines with `serde_json`.

use serde_json::Value;
use std::path::PathBuf;

/// Parses one recording into its JSON lines.
///
/// Rust note: `env!("CARGO_MANIFEST_DIR")` is the crate's directory, fixed
/// at compile time, so the test finds its files whatever the current
/// directory is when `cargo test` runs.
fn lines(name: &str) -> Vec<Value> {
    let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "tests/fixtures/claude-code", name]
        .iter()
        .collect();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    text.lines()
        .enumerate()
        .map(|(i, l)| {
            serde_json::from_str(l).unwrap_or_else(|e| panic!("{name}, line {}: {e}", i + 1))
        })
        .collect()
}

/// The `system/init` lines of a recording.
fn inits(v: &[Value]) -> Vec<&Value> {
    v.iter()
        .filter(|d| d["type"] == "system" && d["subtype"] == "init")
        .collect()
}

/// The tool names an init line announces.
fn tools(init: &Value) -> Vec<&str> {
    init["tools"]
        .as_array()
        .expect("init has a tools list")
        .iter()
        .map(|t| t.as_str().unwrap())
        .collect()
}

/// Names of the tools the model called in `assistant` lines.
fn tool_uses(v: &[Value]) -> Vec<&str> {
    v.iter()
        .filter(|d| d["type"] == "assistant")
        .flat_map(|d| d["message"]["content"].as_array().into_iter().flatten())
        .filter(|b| b["type"] == "tool_use")
        .map(|b| b["name"].as_str().unwrap())
        .collect()
}

/// The `result` lines: the probe's, then the real message's.
fn results(v: &[Value]) -> Vec<&Value> {
    v.iter().filter(|d| d["type"] == "result").collect()
}

/// A safe run: a probe turn ended by the interrupt, then the real turn.
/// `allowed` is the tool list the safety check accepts for this run.
fn assert_safe_run(name: &str, allowed: &[&str]) -> Vec<Value> {
    let v = lines(name);
    let i = inits(&v);
    // One init per turn: the probe's, then the real message's.
    assert_eq!(i.len(), 2, "{name}: one init per turn");
    for init in &i {
        assert_eq!(tools(init), allowed, "{name}: announced tools");
        assert_eq!(init["mcp_servers"].as_array().unwrap().len(), 0, "{name}: MCP servers");
    }
    let r = results(&v);
    assert_eq!(r.len(), 2, "{name}: two turns");
    // The interrupted probe costs nothing and has no answer.
    assert_eq!(r[0]["subtype"], "error_during_execution", "{name}: probe interrupted");
    assert_eq!(r[0]["total_cost_usd"].as_f64(), Some(0.0), "{name}: probe is free");
    assert_eq!(r[1]["subtype"], "success", "{name}: real turn");
    assert_eq!(r[1]["is_error"], false, "{name}: real turn");
    v
}

#[test]
fn injection_finds_no_tool_to_call() {
    for name in ["prompt-injection.jsonl", "prompt-injection-invited.jsonl"] {
        let v = assert_safe_run(name, &[]);
        assert!(tool_uses(&v).is_empty(), "{name}: no tool called");
    }
}

#[test]
fn structured_output_adds_only_its_own_tool() {
    let v = assert_safe_run("structured-output.jsonl", &["StructuredOutput"]);
    assert_eq!(tool_uses(&v), ["StructuredOutput"]);
    let out = &results(&v)[1]["structured_output"];
    assert_eq!(out["subjects"].as_array().map(Vec::len), Some(3));
}

#[test]
fn streamed_text_comes_in_deltas() {
    let v = assert_safe_run("streamed-text.jsonl", &[]);
    let deltas = v
        .iter()
        .filter(|d| d["type"] == "stream_event" && d["event"]["type"] == "content_block_delta")
        .count();
    assert!(deltas > 1, "several deltas, got {deltas}");
    assert!(results(&v)[1]["result"].as_str().unwrap().contains("notaire"));
}

#[test]
fn not_logged_in_is_recognizable() {
    let v = lines("not-logged-in.jsonl");
    let assistant = v.iter().find(|d| d["type"] == "assistant").expect("an assistant line");
    assert_eq!(assistant["error"], "authentication_failed");
    let r = results(&v);
    assert_eq!(r.len(), 1);
    assert_eq!(r[0]["is_error"], true);
    assert!(r[0]["result"].as_str().unwrap().contains("Not logged in"));
}

#[test]
fn default_options_announce_tools() {
    let v = lines("unsafe-default-tools.jsonl");
    // Only the init line: the core kills the process there.
    assert_eq!(v.len(), 1);
    let init = inits(&v)[0];
    let t = tools(init);
    assert!(t.contains(&"Bash") && t.contains(&"Write"), "dangerous tools announced");
    assert!(!init["mcp_servers"].as_array().unwrap().is_empty());
}

#[test]
fn recordings_name_no_user_or_machine() {
    for name in [
        "prompt-injection.jsonl",
        "prompt-injection-invited.jsonl",
        "structured-output.jsonl",
        "streamed-text.jsonl",
        "not-logged-in.jsonl",
        "unsafe-default-tools.jsonl",
    ] {
        let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "tests/fixtures/claude-code", name]
            .iter()
            .collect();
        let text = std::fs::read_to_string(path).unwrap();
        for needle in ["/Users/", "/private/tmp/"] {
            assert!(!text.contains(needle), "{name} contains {needle:?}");
        }
        // An "@" may only be the test mail's sender or a built-in plugin's
        // source; anything else could be a real email address.
        //
        // Rust note: `match_indices` yields `(byte offset, matched text)`;
        // slicing `&text[i + 1..]` is safe because "@" is one byte long, so
        // `i + 1` is always on a character boundary.
        for (i, _) in text.match_indices('@') {
            let after = &text[i + 1..];
            assert!(
                after.starts_with("evil.example") || after.starts_with("builtin"),
                "{name}: unexpected address near {:?}",
                &after[..after.len().min(20)]
            );
        }
    }
}
