# AI assistance

NiceMailer can ask a language model to help with mail: proofread a draft, summarize a conversation, suggest where a mail of the Quarantine should go. The rules are in `specifications/ia.md` (in French). This page describes what is built so far and how it is tested.

**Status (November 2026):** the core can call Claude through the user's own `claude` binary (crate `nm-ai`). Nothing in the app, the CLI or the contract uses it yet; prompts, inputs and the API-key provider come next.

## Providers

`nm_ai::Provider` is the provider the configuration chose (`ai.provider`). Each one takes an `AiRequest` (system prompt, message, model, optional JSON Schema) and returns an `AiOutcome` (text, structured answer, tokens, cost) or an `AiError`, whose `code()` is the error code of the contract (`ai_not_logged_in`, `ai_provider_unsafe`…). Text answers come in fragments, given to a callback as they arrive. To cancel a call, drop its future.

| Provider | What it is |
| --- | --- |
| `claude-code` (`nm_ai::claude_code`) | The user's `claude` binary (Claude Code), signed in with their Claude subscription. NiceMailer stores no credential |
| `fake` (`nm_ai::fake`) | Answers from a table, keyed by the `TASK=<id>` first line of the system prompt. For the tests of the layers above |

### The `claude` binary

The binary is `ai.claude_path`, or the first executable found in `~/.local/bin/claude`, `~/.claude/local/claude`, `/opt/homebrew/bin/claude`, `/usr/local/bin/claude`. The core runs under launchd, without the shell's `PATH`, so the search is explicit (`claude_code::find_binary`).

Each call is one process, started in its own process group, in an empty work directory, with options that leave it no tool, no MCP server, no settings, no hooks and no memory (`claude_code::args`; the list and the reason for each option are in `ia.md`, section 3, "L'appel"). The content of a mail is never a command-line argument, since arguments are visible to every process of the Mac.

**The safety check.** Claude Code is an agent: with its default options it can run commands, edit files and browse the web. A mail of the Quarantine read by such an agent could order it to. So the core proves, at every call, that the process has no tool before giving it anything of the user's:

1. It sends a probe message that holds nothing of the user's.
2. It reads the `system/init` line the process writes in answer, and checks that `tools` is empty (or exactly `["StructuredOutput"]`, the internal tool that `--json-schema` adds) and that `mcp_servers` is empty (`stream::check_init`).
3. If the check fails, the process is killed and the call fails with `ai_provider_unsafe`, which names what was announced. The mail was never written to the process.
4. If it passes, the core interrupts the probe turn before any call to the model (it costs nothing), then sends the real message.

The process only announces itself after a first message, hence the probe. The check runs at every call because an update of Claude Code could change what its options mean.

**Reading the output.** `stream::ClaudeStream` is a state machine fed one JSON line at a time, with no I/O. It ignores fragments of the probe's turn, recognizes the errors (not signed in, usage limit, missing structured output, a result before the announcement, a line that is not JSON) and builds the outcome. The cost Claude Code states is the API-price equivalent of the call, not a bill: the outcome says `billed: false`.

**Ending a call.** Whatever the way out (answer, error, timeout, cancel), the process group gets `SIGKILL`, grandchildren included. The group is killed before the process is reaped: the group id is the process's pid, which could name another process once reaped.

`ClaudeCode::status` gives the version of Claude Code and the subscription ("Claude Pro") from the `initialize` control request alone, without calling the model.

## Tests

| Test | What it covers |
| --- | --- |
| `cargo test -p nm-ai` (unit, `src/`) | The safety check (every way an announcement can fail), the reading of the stream, the fake provider |
| `tests/fixtures.rs` | The recordings of the real binary (`tests/fixtures/claude-code/`, described in their README) show what they claim, and name no user or machine |
| `tests/claude_code.rs` | The provider against a fake `claude`, a shell script that replays the recordings turn by turn and keeps what it read: the order probe, interrupt, message; a process announcing tools gets the probe only; not signed in is found before the content is sent; the safety options are passed; a timeout and a cancel kill the process group, grandchildren included; a missing binary |
| `tests/real_claude.rs` | Ignored by default. The real binary, with the user's subscription: status, a prompt injection that must run nothing, a structured answer. Run it after an update of Claude Code: `cargo test -p nm-ai --test real_claude -- --ignored` |
