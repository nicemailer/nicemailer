//! The binary `nicemailer-mcp` as an MCP client starts it (`mcp.md`,
//! section 12): nothing but the protocol on its standard output, whatever
//! it is sent, its log on the standard error; and its commands for Claude
//! Desktop's configuration, as the app runs them.

use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::{Value, json};

const BIN: &str = env!("CARGO_BIN_EXE_nicemailer-mcp");

#[test]
fn nothing_but_the_protocol_is_written_on_the_standard_output() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = Command::new(BIN)
        .arg("--data-dir")
        .arg(dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let requests = [
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}}).to_string(),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string(),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}).to_string(),
        // No core on this directory: a tool error, and a line in the log.
        json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "search_messages", "arguments": {"query": "x"}}}).to_string(),
        "this is not JSON".to_string(),
        "[1, 2]".to_string(),
        json!({"jsonrpc": "2.0", "id": "s", "method": "prompts/list"}).to_string(),
        json!({"jsonrpc": "2.0", "id": 4, "method": "ping"}).to_string(),
    ];
    let mut stdin = child.stdin.take().unwrap();
    for r in requests {
        writeln!(stdin, "{r}").unwrap();
    }
    // The output read as it comes, in a thread: the tool call waits up to
    // 3 s for a core, and closing the input before its answer would drop
    // it (the client is gone).
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let reader = std::thread::spawn(move || {
        use std::io::BufRead;
        for line in std::io::BufReader::new(stdout).lines() {
            tx.send(line.unwrap()).unwrap();
        }
    });
    let mut raw = Vec::new();
    while !raw.iter().any(|l: &String| l.starts_with("{") && serde_json::from_str::<Value>(l).is_ok_and(|v| v["id"] == 3)) {
        raw.push(rx.recv_timeout(std::time::Duration::from_secs(10)).expect("no answer to the tool call within 10 s"));
    }
    drop(stdin);
    reader.join().unwrap();
    raw.extend(rx.try_iter());
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "{:?}", out.status);
    let stdout = raw.join("\n");
    let lines: Vec<Value> = raw.iter().map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("not JSON: {l:?}: {e}"))).collect();
    for l in &lines {
        assert_eq!(l["jsonrpc"], "2.0", "{l}");
        assert!(l.get("result").is_some() != l.get("error").is_some(), "{l}");
    }
    let by_id = |id: Value| lines.iter().find(|l| l["id"] == id).unwrap_or_else(|| panic!("no answer to {id}: {stdout}"));
    assert_eq!(by_id(json!(1))["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(by_id(json!(2))["result"]["tools"].as_array().unwrap().len(), 13);
    assert_eq!(by_id(json!(3))["result"]["isError"], true);
    assert_eq!(by_id(json!("s"))["error"]["code"], -32601);
    assert_eq!(by_id(json!(4))["result"], json!({}));
    // The two bad lines: answered with id null.
    assert_eq!(lines.iter().filter(|l| l["id"].is_null()).count(), 2, "{stdout}");
    assert_eq!(lines.len(), 7);
    // The log is on the standard error.
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("initialize by t 1"), "{stderr}");
}

#[test]
fn claude_desktop_gets_its_entry_as_the_app_asks() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("claude_desktop_config.json");
    std::fs::write(&config, r#"{"mcpServers": {"other": {"command": "x"}}}"#).unwrap();
    let run = |args: &[&str]| -> (bool, Value) {
        let out = Command::new(BIN)
            .args(["claude-desktop"])
            .args(args)
            .args(["--config", config.to_str().unwrap(), "--binary", "/Applications/NiceMailer.app/Contents/Helpers/nicemailer-mcp", "--json"])
            .output()
            .unwrap();
        (out.status.success(), serde_json::from_slice(&out.stdout).unwrap())
    };
    assert_eq!(run(&["status"]).1["outcome"], "absent");
    assert_eq!(run(&["install", "--repair"]).1["outcome"], "not_installed");
    let (ok, v) = run(&["install"]);
    assert!(ok);
    assert_eq!(v["outcome"], "added");
    assert_eq!(v["entry"], json!({"command": "/Applications/NiceMailer.app/Contents/Helpers/nicemailer-mcp"}));
    assert_eq!(run(&["status"]).1["outcome"], "configured");
    assert_eq!(run(&["install"]).1["outcome"], "unchanged");
    let file: Value = serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    assert_eq!(file["mcpServers"]["other"], json!({"command": "x"}));
    assert_eq!(run(&["remove"]).1["outcome"], "removed");
    // A broken file: an error, the file left as it was.
    std::fs::write(&config, "{ broken").unwrap();
    let (ok, v) = run(&["install"]);
    assert!(!ok);
    assert_eq!(v["outcome"], "error");
    assert_eq!(std::fs::read_to_string(&config).unwrap(), "{ broken");
}
