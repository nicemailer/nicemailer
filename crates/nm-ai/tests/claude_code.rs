//! The `claude-code` provider against a fake `claude` binary that replays
//! the phase 0 recordings (`tests/fixtures/claude-code/`).
//!
//! The fake is a shell script that behaves like the real binary turn by
//! turn: it writes the next piece of a recording only after reading one line
//! on its standard input, and it keeps everything it read. A test can thus
//! prove what reached the process, for instance that a process announcing a
//! tool got the probe and nothing else.

use std::path::{Path, PathBuf};
use std::time::Duration;

use nm_ai::claude_code::{self, ClaudeCode, ClaudeStatus, PROBE};
use nm_ai::{AiError, AiRequest};
use serde_json::{Value, json};

/// A fake binary in a temporary directory.
struct FakeClaude {
    // Rust note: the `TempDir` value owns the directory and deletes it when
    // dropped, at the end of the test; keeping it in the struct keeps the
    // directory alive as long as the fake is used.
    dir: tempfile::TempDir,
}

impl FakeClaude {
    /// A fake that replays `chunks`: the first after the first line read on
    /// stdin, the second after the second line, and so on. `tail` is shell
    /// code run after the last chunk.
    fn new(chunks: &[String], tail: &str) -> FakeClaude {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        for (i, c) in chunks.iter().enumerate() {
            std::fs::write(d.join(format!("chunk{}", i + 1)), c).unwrap();
        }
        let script = format!(
            r#"#!/bin/sh
D='{d}'
echo $$ > "$D/pid"
printf '%s\n' "$@" > "$D/args"
n=1
while [ -f "$D/chunk$n" ]; do
  IFS= read -r line || exit 0
  printf '%s\n' "$line" >> "$D/stdin"
  cat "$D/chunk$n"
  n=$((n+1))
done
{tail}
"#,
            d = d.display()
        );
        let bin = d.join("claude");
        std::fs::write(&bin, script).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        FakeClaude { dir }
    }

    /// A fake replaying a recording, cut where the real binary waits for
    /// input: after the first `system/init` (it waits for the interrupt),
    /// after the first `result` (it waits for the message), then the rest.
    fn replay(fixture: &str) -> FakeClaude {
        FakeClaude::new(&turns(fixture), "")
    }

    fn provider(&self) -> ClaudeCode {
        ClaudeCode {
            binary: self.dir.path().join("claude"),
            workdir: self.dir.path().join("work"),
            timeout: Duration::from_secs(10),
        }
    }

    /// The lines the fake read on its stdin, parsed.
    fn received(&self) -> Vec<Value> {
        std::fs::read_to_string(self.dir.path().join("stdin"))
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    /// The arguments it was started with, one per line.
    fn args(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir.path().join("args"))
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect()
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }
}

fn fixture(name: &str) -> String {
    let p: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "tests/fixtures/claude-code",
        name,
    ]
    .iter()
    .collect();
    std::fs::read_to_string(p).unwrap()
}

/// Cuts a recording into the pieces the real binary writes between two
/// reads of its input.
fn turns(name: &str) -> Vec<String> {
    let mut chunks = vec![String::new()];
    let mut cuts = 0;
    for line in fixture(name).lines() {
        let d: Value = serde_json::from_str(line).unwrap();
        chunks.last_mut().unwrap().push_str(line);
        chunks.last_mut().unwrap().push('\n');
        let is_init = d["type"] == "system" && d["subtype"] == "init";
        let is_result = d["type"] == "result";
        if (cuts == 0 && is_init) || (cuts == 1 && is_result) {
            cuts += 1;
            chunks.push(String::new());
        }
    }
    chunks.retain(|c| !c.is_empty());
    chunks
}

fn request(schema: Option<Value>) -> AiRequest {
    AiRequest {
        system: "TASK=test\nA system prompt.".into(),
        message: "<email>The content.</email>".into(),
        model: "claude-haiku-5-5".into(),
        schema,
        max_output_tokens: None,
    }
}

fn subjects_schema() -> Value {
    json!({"type": "object", "required": ["subjects"],
           "properties": {"subjects": {"type": "array", "items": {"type": "string"}}}})
}

/// The `content` of a user line the fake received.
fn content(v: &Value) -> &str {
    v["message"]["content"].as_str().unwrap_or("")
}

#[tokio::test]
async fn a_safe_call_sends_the_probe_then_the_interrupt_then_the_message() {
    let fake = FakeClaude::replay("prompt-injection.jsonl");
    let out = fake.provider().run(&request(None), |_| {}).await.unwrap();
    assert!(out.text.contains("Junk"), "{}", out.text);
    assert!(!out.billed, "a subscription call is not billed");
    assert!(out.cost_usd.unwrap() > 0.0);
    assert!(out.usage.input_tokens > 0 && out.usage.output_tokens > 0);

    let got = fake.received();
    assert_eq!(got.len(), 3, "{got:?}");
    assert_eq!(content(&got[0]), PROBE);
    assert_eq!(got[1]["request"]["subtype"], "interrupt");
    assert_eq!(content(&got[2]), "<email>The content.</email>");
}

#[tokio::test]
async fn a_process_announcing_tools_never_gets_the_content() {
    let fake = FakeClaude::replay("unsafe-default-tools.jsonl");
    let err = fake
        .provider()
        .run(&request(None), |_| {})
        .await
        .unwrap_err();
    let AiError::ProviderUnsafe { tools, mcp_servers } = &err else {
        panic!("{err:?}")
    };
    assert!(tools.iter().any(|t| t == "Bash"));
    assert_eq!(mcp_servers, &["example"]);
    assert_eq!(err.code(), "ai_provider_unsafe");
    // The probe alone reached it.
    let got = fake.received();
    assert_eq!(got.len(), 1);
    assert_eq!(content(&got[0]), PROBE);
}

#[tokio::test]
async fn structured_output_is_returned_and_not_streamed() {
    let fake = FakeClaude::replay("structured-output.jsonl");
    let mut deltas = 0;
    let out = fake
        .provider()
        .run(&request(Some(subjects_schema())), |_| deltas += 1)
        .await
        .unwrap();
    assert_eq!(
        out.structured.unwrap()["subjects"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(deltas, 0);
    let args = fake.args();
    assert!(args.contains(&"--json-schema".to_string()));
    assert!(!args.contains(&"--include-partial-messages".to_string()));
}

#[tokio::test]
async fn text_comes_in_fragments_that_make_the_whole_answer() {
    let fake = FakeClaude::replay("streamed-text.jsonl");
    let mut got = String::new();
    let out = fake
        .provider()
        .run(&request(None), |d| got.push_str(d))
        .await
        .unwrap();
    assert!(!got.is_empty());
    assert_eq!(got, out.text);
    assert!(
        fake.args()
            .contains(&"--include-partial-messages".to_string())
    );
}

#[tokio::test]
async fn not_logged_in_is_recognized_before_the_content_is_sent() {
    let fake = FakeClaude::replay("not-logged-in.jsonl");
    let err = fake
        .provider()
        .run(&request(None), |_| {})
        .await
        .unwrap_err();
    assert_eq!(err, AiError::NotLoggedIn);
    // Probe and interrupt; the failed probe turn stops the call before the message.
    let got = fake.received();
    assert_eq!(got.len(), 2, "{got:?}");
    assert!(
        got.iter()
            .all(|l| content(l) != "<email>The content.</email>")
    );
}

#[tokio::test]
async fn the_safety_options_are_passed() {
    let fake = FakeClaude::replay("prompt-injection.jsonl");
    fake.provider().run(&request(None), |_| {}).await.unwrap();
    let a = fake.args();
    // `printf '%s\n' "$@"` writes an empty argument as an empty line.
    let after = |flag: &str| a.iter().position(|x| x == flag).map(|i| a[i + 1].clone());
    assert_eq!(after("--tools").as_deref(), Some(""));
    assert_eq!(after("--setting-sources").as_deref(), Some(""));
    assert_eq!(
        after("--mcp-config").as_deref(),
        Some(r#"{"mcpServers":{}}"#)
    );
    for flag in [
        "--strict-mcp-config",
        "--disable-slash-commands",
        "--no-session-persistence",
    ] {
        assert!(a.contains(&flag.to_string()), "{flag}");
    }
    assert_eq!(after("--model").as_deref(), Some("claude-haiku-5-5"));
    assert!(after("--system-prompt").unwrap().starts_with("TASK=test"));
    // The content is never an argument: arguments are visible to every
    // process of the Mac (`ps`).
    assert!(!a.iter().any(|x| x.contains("The content.")));
}

#[tokio::test]
async fn a_timeout_kills_the_process_group() {
    // The fake starts a grandchild and then hangs: the timeout must kill
    // both, the grandchild through the process group.
    let tail = r#"sleep 60 & echo $! > "$D/grandchild"; wait"#;
    let fake = FakeClaude::new(&[String::new()], tail);
    let mut p = fake.provider();
    // Long enough for the fake to start its grandchild even on a loaded
    // machine (500 ms was not, once): the grandchild must exist when the
    // timeout kills the group, or the test would prove nothing.
    p.timeout = Duration::from_secs(3);
    // The first chunk is empty, so the fake writes nothing after the probe.
    let err = p.run(&request(None), |_| {}).await.unwrap_err();
    assert_eq!(err, AiError::Timeout);
    let pid = read_pid(&fake.path().join("grandchild"))
        .expect("the grandchild started before the timeout");
    assert!(
        gone_within(pid, Duration::from_secs(3)),
        "grandchild {pid} still running"
    );
}

#[tokio::test]
async fn a_cancelled_call_kills_the_process_group() {
    let tail = r#"sleep 60 & echo $! > "$D/grandchild"; wait"#;
    let fake = FakeClaude::new(&[String::new()], tail);
    let p = fake.provider();
    let req = request(None);
    let pid_file = fake.path().join("grandchild");
    // Dropping the future is how a caller cancels (`ia.md`, `ai_cancel`):
    // here, `select!` drops the call as soon as the other branch ends, which
    // is when the grandchild is running. A fixed delay instead made the test
    // fail on a loaded machine, where the fake had not started it yet.
    let pid = tokio::select! {
        _ = p.run(&req, |_| {}) => panic!("the call cannot finish"),
        pid = async {
            loop {
                if let Some(pid) = read_pid(&pid_file) {
                    break pid;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        } => pid,
    };
    assert!(
        gone_within(pid, Duration::from_secs(3)),
        "grandchild {pid} still running"
    );
}

#[tokio::test]
async fn a_process_that_ends_without_a_result_is_an_error() {
    // Announces itself safely, then exits after the interrupt.
    let init = r#"{"type":"system","subtype":"init","tools":[],"mcp_servers":[]}"#;
    let fake = FakeClaude::new(&[format!("{init}\n")], "IFS= read -r line; exit 0");
    let err = fake
        .provider()
        .run(&request(None), |_| {})
        .await
        .unwrap_err();
    assert!(matches!(err, AiError::InvalidOutput(_)), "{err:?}");
}

#[tokio::test]
async fn a_missing_binary_is_provider_missing() {
    let dir = tempfile::tempdir().unwrap();
    let p = ClaudeCode {
        binary: PathBuf::from("/nonexistent/claude"),
        workdir: dir.path().join("work"),
        timeout: Duration::from_secs(5),
    };
    assert_eq!(
        p.run(&request(None), |_| {}).await.unwrap_err(),
        AiError::ProviderMissing
    );
}

#[tokio::test]
async fn status_reads_the_version_and_subscription_without_a_message() {
    // Hand-written in the shape of the recorded `initialize` answer, whose
    // real copy names the user's account and is not kept.
    let resp = json!({"type": "control_response", "response": {"subtype": "success", "request_id": "nm-status",
        "response": {"claude_code_version": "2.1.325",
                     "account": {"subscriptionType": "Claude Pro", "apiProvider": "firstParty"}}}});
    let fake = FakeClaude::new(&[format!("{resp}\n")], "");
    let s = fake.provider().status().await.unwrap();
    assert_eq!(
        s,
        ClaudeStatus {
            version: "2.1.325".into(),
            subscription: Some("Claude Pro".into())
        }
    );
    let got = fake.received();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0]["request"]["subtype"], "initialize");
    assert!(!fake.args().contains(&"--model".to_string()));
}

#[test]
fn the_binary_is_found_in_the_usual_places() {
    use std::os::unix::fs::PermissionsExt;
    let home = tempfile::tempdir().unwrap();
    assert_eq!(
        claude_code::find_binary(None, home.path()).filter(|p| p.starts_with(home.path())),
        None
    );
    let bin = home.path().join(".local/bin/claude");
    std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
    std::fs::write(&bin, "#!/bin/sh\n").unwrap();
    // Not executable yet: not taken.
    assert_eq!(
        claude_code::find_binary(None, home.path()).filter(|p| p.starts_with(home.path())),
        None
    );
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        claude_code::find_binary(None, home.path()),
        Some(bin.clone())
    );
    // A configured path wins, and is not replaced by a search when wrong.
    assert_eq!(
        claude_code::find_binary(Some(&bin), Path::new("/nonexistent")),
        Some(bin)
    );
    assert_eq!(
        claude_code::find_binary(Some(Path::new("/nonexistent/claude")), home.path()),
        None
    );
}

/// Whether a process is gone within `limit`. `kill(pid, 0)` sends nothing
/// and only says whether the process exists.
fn gone_within(pid: i32, limit: Duration) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < limit {
        // Rust note: `unsafe` because `kill` is a C function; signal 0 only
        // checks that the process exists.
        if unsafe { libc::kill(pid, 0) } != 0 {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

/// The pid a fake wrote to a file, once it is there and complete.
fn read_pid(path: &Path) -> Option<i32> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
}
