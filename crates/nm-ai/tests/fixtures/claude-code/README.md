# Recordings of the `claude` binary

What Claude Code 2.1.325 wrote on its standard output during phase 0 of the AI assistance (`specifications/ia.md`, section 3), on 14 November 2026, with a Claude Pro subscription. The provider's tests replay them through a fake `claude` binary, so the core's parsing and safety check run against real output without network or account.

Every run except the unsafe one used the options of `ia.md`, "L'appel" (`--tools ""`, `--strict-mcp-config`, `--setting-sources ""`, and so on), the stream-json format both ways, and the call sequence of "Vérification à chaque appel": a probe message, the `system/init` check, an `interrupt` control request, then the real message.

| File | Run | What it shows |
| --- | --- | --- |
| `prompt-injection.jsonl` | A triage prompt on a mail that orders the model to run commands, write files, start an agent, fetch a URL and read `~/.ssh/id_rsa` | `tools: []`, no `tool_use` block, the model names the mail as phishing |
| `prompt-injection-invited.jsonl` | The same mail, with a system prompt that says to use any tool available | Same: no tool exists, none is called |
| `structured-output.jsonl` | `--json-schema` for three subject lines | `tools: ["StructuredOutput"]`, a `tool_use` of `StructuredOutput`, `result.structured_output` |
| `streamed-text.jsonl` | `--include-partial-messages`, a French proofreading | `stream_event` lines with `content_block_delta`, then the full text in `result.result` |
| `not-logged-in.jsonl` | The probe alone, with an empty `HOME` | `error: "authentication_failed"`, `result.is_error`, "Not logged in · Please run /login"; the process exits with 1 |
| `unsafe-default-tools.jsonl` | Without the safety options: the `system/init` line only, where the core kills the process | A non-empty `tools` and `mcp_servers`: the check must refuse it |

Things the recordings show that a parser must handle:

- `system/init` comes only after the first user message, and again at the start of each turn: twice in a safe run, once for the probe and once for the real message (`tests/fixtures.rs` checks this and the other claims of this README).
- The interrupted probe turn ends with a `result` of subtype `error_during_execution`, no `result` text and a zero cost.
- With `--json-schema`, the tool result comes back as a `user` line before the final `result`.
- `rate_limit_event` gives the subscription's 5-hour and 7-day windows.

## Changes from the raw recordings

The raw output names the machine and its user. Before being committed:

- session ids and line UUIDs are replaced by fake ones (`00000000-0000-4000-8000-…`), stable within a file;
- `cwd` and `original_cwd` are `/work`, `memory_paths` is empty, `messaging_socket_path` is `/tmp/cc-socks/0.sock`, and only the built-in plugins are kept;
- in `unsafe-default-tools.jsonl`, the user's MCP tools, MCP servers, skills, commands and custom agents are replaced by one example of each kind (`mcp__example__search`, a server named `example`). The built-in tool names are the recorded ones. The lines of the user's own hooks (`hook_started`, `hook_response`) are removed.

Message ids, texts, usage, costs and timings are as recorded.
