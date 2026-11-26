# AI agents: the MCP server

An AI agent (Claude Desktop, Claude Code, or any client of the Model Context Protocol) can read your mailbox and prepare drafts through `nicemailer-mcp`, the MCP server shipped inside NiceMailer.app. It can summarize conversations, find a mail, read an attached invoice, answer "what is waiting for me?", and write replies that you review and send yourself.

It cannot send mail, move it, mark it, delete it or unsubscribe. These rules are enforced by the core, not by the server: see [Rights](#rights).

The specification, in French, is `specifications/mcp.md`. What is built today is its phases 1 (reading) and 2 (drafts). Sending after a confirmation in the app (phase 3) waits for the core to check the signature of its clients; filing behind a protected setting (phase 4) waits for protected settings.

## Connecting an agent

In the app menu, **NiceMailer > AI Agents**:

| Item | Effect |
| --- | --- |
| Connect to Claude Desktop | Adds the entry `nicemailer` to `~/Library/Application Support/Claude/claude_desktop_config.json`, keeping the rest of the file. Restart Claude Desktop afterwards. |
| Copy Command for Claude Code | Copies `claude mcp add --scope user nicemailer -- /Applications/NiceMailer.app/Contents/Helpers/nicemailer-mcp`: paste it in a terminal. |
| Copy MCP Configuration | Copies `{"nicemailer": {"command": "<path>"}}` for another client. |

The items are disabled in an app started by `swift run`, which has no helper. At each launch the app also repairs the Claude Desktop entry when it exists and names another binary (the app was moved).

Writing Claude Desktop's file is done by the helper, so that its rules live in one tested place (`crates/nm-mcp/src/install.rs`):

- only `mcpServers.nicemailer` changes; the other servers and settings are kept (the file is rewritten with its keys sorted, which Claude Desktop does not mind);
- the file is written to a temporary file and renamed over the old one;
- a file that is not a JSON object is never overwritten: the app says why, and you can use "Copy MCP Configuration" instead;
- without the folder `~/Library/Application Support/Claude`, Claude Desktop is not installed and nothing is created.

The same commands work in a terminal:

```sh
/Applications/NiceMailer.app/Contents/Helpers/nicemailer-mcp claude-desktop status
/Applications/NiceMailer.app/Contents/Helpers/nicemailer-mcp claude-desktop install [--repair]
/Applications/NiceMailer.app/Contents/Helpers/nicemailer-mcp claude-desktop remove
```

They accept `--config <file>`, `--binary <path>` (the binary the entry runs, this one by default) and `--json` (the outcome as one JSON line, for the app).

## How it runs

The MCP client starts `nicemailer-mcp` and speaks MCP with it on its standard input and output (the stdio transport). There is no HTTP transport: it would open the mailbox to the network.

- **The standard output belongs to the protocol.** The server's log goes to its standard error, which Claude Desktop keeps in `~/Library/Logs/Claude/mcp-server-nicemailer.log`. A test checks that nothing else ever reaches the standard output.
- **Lazy connection.** `initialize` is answered at once, with or without a core. The first tool call connects to the core of the mailbox chosen in the app (`data-dir`, as the CLI does), with the handshake `hello` as client `mcp`. A connection lost (the core restarted, the mailbox changed) is opened again at the next call; a call that only reads is tried once more on the new connection.
- **Core started on demand.** No socket: `launchctl kickstart gui/<uid>/eu.nicemailer.core`, then tries for 3 seconds. Not done when `NICEMAILER_DATA_DIR` or `--data-dir` names the mailbox, since launchd's core serves the one chosen in the app.
- **Maintenance.** While NiceMailer is updated or the core upgrades the database, the core answers `MAINTENANCE` ([updates.md](updates.md)): the call waits, trying every second for 30 seconds, without starting a core, and reports it through progress notifications when the client asked for them (`_meta.progressToken`). Then the start wait begins again, for the socket to open. A maintenance only the user can end (a database written by a newer NiceMailer, an upgrade that failed) fails the call at once, with its reason.
- **No core.** The tool fails with what to do: open NiceMailer, or allow it in System Settings > General > Login Items.
- **Several calls at once.** Each tool call runs in a task of its own; requests to the core are multiplexed on one connection. `notifications/cancelled` stops a call, which then gets no answer (a request already sent to the core may still have run there).

### Protocol versions

The server speaks MCP `2025-11-25`, `2025-06-18`, `2025-03-26` and `2024-11-05`. It answers `initialize` with the version the client asks for when it knows it, else with the latest. Before `2025-06-18` there is no `outputSchema` and no `structuredContent`; before `2025-03-26`, no tool annotations. The answer of a tool is always also in its text content, as compact JSON.

The server is written directly on JSON-RPC rather than with the Rust SDK (`rmcp`): it needs a small part of MCP, it must control every byte of its output, and the SDK's API is still moving. See the module documentation of `crates/nm-mcp/src/server.rs`.

## Tools

Every tool declares an input schema, an output schema and the annotations `readOnlyHint`, `destructiveHint`, `idempotentHint` and `openWorldHint` (false: everything stays in the mailbox), so that the client can ask you before a tool that changes something. Arguments outside the schema (a misspelt name, an unknown filter) are refused as a tool error that names the problem, for the model to correct its call; an unknown tool is a protocol error.

### Reading

| Tool | Arguments | Answer | Contract |
| --- | --- | --- | --- |
| `mailbox_overview` | — | Accounts (key, address, kind, error), folders (id, account, role, name, unread, total), tags | `list_accounts`, `list_folders`, `list_tags` |
| `list_conversations` | `folder`, `account?`, `filter?`, `sort?`, `cursor?`, `limit?` (25, at most 100) | A page of conversations: id, subject, with whom, date, messages, unread, flags, folder, account, snippet, tags; `next_cursor` | `list_threads` |
| `search_messages` | `query`, `limit?` | Messages found in every folder, with an excerpt, matches in `[brackets]` | `search` |
| `get_conversation` | `thread_id`, `include_quoted?` | The messages, oldest first, reduced (below) | `get_thread` |
| `get_message` | `message_id` or `link`, `offset?` | One message: header, its whole text in pages, attachments. `link` is a link to a mail the user pasted, `nicemailer://m/<key>` (from a note, say), or an old `nicemailer://m/<id>` | `locate_message`, `message_text`, `get_thread` |
| `read_attachment` | `attachment_id`, `offset?` | The text of a PDF, DOCX, HTML, text or attached mail, in pages; for another file, `unreadable` and its local `path` | `attachment_text` |
| `find_contacts` | `query`, `limit?` | Addresses exchanged with, the most written to first | `suggest_recipients` |
| `agent_history` | `since?`, `cursor?` | What AI agents did (lines of the audit log of source `mcp`) | `list_history` |
| `get_draft` | `draft_id` | A draft as it is now, body included | `open_draft` |

`folder` is a role (`inbox`, `todo`, `waiting`, `archive`, `sent`, `drafts`, `newsletter`, `paperwork`, `quarantine`, `deleted`, `junk`, `banned`), a view (`work`, `all_mail`, `everything`) or a folder id. A role without `account` means that folder in every account, as the unified view of the app; the server finds them in `list_folders`.

`filter` takes `unread`, `flagged`, `attachments`, `answered`, `human`, `from`, `not_from`, `to`, `text`, `subject`, `contains` (full-text), `after`, `before` (dates) and `tag`.

Today the agent writes nothing to the audit log (drafts are not logged there, `tracabilite.md`), so `agent_history` is empty until the phases that file mail. The agent's drafts are in the Drafts folder.

### Drafts

| Tool | Arguments | What it does | Contract |
| --- | --- | --- | --- |
| `create_draft` | `kind` (`reply_all`, `reply_sender`, `forward`, `new`), `reply_to_message_id?`, `account?`, `to?`, `cc?`, `bcc?`, `subject?`, `body`, `quote_header?`, `include_quote?` | A reply gets the recipients, subject and quotation the core prepares, the agent's body above the quotation; a new mail is sent from `account` or the first account that can send | `parse_addresses`, `create_draft`, `update_draft`, `close_draft` |
| `update_draft` | `draft_id`, `body?`, `subject?`, `to?`, `cc?`, `bcc?` | Changes a draft of the agent; `body` replaces the whole text | `open_draft`, `update_draft`, `close_draft` |
| `attach_file` | `draft_id`, `path` (absolute) | Attaches a file of the Mac | `open_draft`, `add_draft_attachment`, `close_draft` |
| `discard_draft` | `draft_id` | Deletes a draft of the agent | `discard_draft` |

- **Closed at each command.** The agent has no window: each command is a whole edit, and the draft goes to the server's Drafts at once, where you find it in the app and on your phone.
- **Revisions.** The server reads the current revision just before saving. If you edit the same draft meanwhile, the save fails (`REVISION_CONFLICT`) rather than overwrite your text, and the agent reads the draft again.
- **Plain text.** The agent writes text only. A draft of the agent that you switched to the formatted mode ([editor.md](editor.md)) keeps its text: the agent may still change its subject and recipients, not its body.
- **Addresses** are free text (`"Alice <alice@example.com>, bob@example.com"`), read by the core's `parse_addresses`. One that is not an address fails the tool, naming it, before anything is written.
- **The quote line** ("On 16 Nov 2026 at 09:42, Alice wrote:") is `quote_header`, which the agent writes in the language of its reply. Without it, the server writes an English one from the date and the sender of the mail. (The contract leaves this line to the client, in its language; the server has no translations.)
- **Attached files** are copied by the server into `incoming.noindex` of the data directory, where the core takes them, as the app does (a clone on APFS: instant). The agent attaches only documents: files under your home folder or on another volume (`/Volumes`), never a hidden file or one in a hidden folder (`~/.ssh`, `.env`, `.git`…), nothing under `~/Library`, nothing of NiceMailer's data, the rule applying to the real path, links resolved (`nm_mcp::tools::attachable`). Why: the draft goes to the server's Drafts at once and comes back by the synchronization as a mail the agent can read, so a mail that told the agent to attach a secret would make it readable. You attach anything from the app or the CLI.

## The answers

Every byte returned takes room in the model's context:

- **Empty fields are left out**: no `"cc": []`, no `"flagged": false`. The lists of results themselves are always there, even empty.
- **Dates** are ISO 8601 at local time with the offset, `2026-11-16T09:42:00+01:00`. Dates given by the agent (`after`, `before`, `since`) are read the same way, or as `2026-11-16` (local midnight).
- **Ids** are those of the contract (`thread_id`, `message_id`, `attachment_id`, `draft_id`).
- **A message** in `get_conversation` is: id, folder role, date, sender, recipients when the app would show them, `reply_to` when it differs from the sender, `authentication` when it is not `pass`, `unread`, `flagged`, `answered`, its attachments (id, name, type, size, readable), a verification code when the core found one, its `link` (`nicemailer://m/<key>`, `docs/deep-links.md`: the agent can put it in a note or an answer, and a click opens the mail in NiceMailer, on any Mac that has it), and its text: the segments the app shows (what the message brings, without the history it quotes again; `quote_hidden` when some were left out). With `include_quoted`, every segment, quoted ones prefixed by `>`. A body not downloaded yet is `text_pending`.
- **Errors** (`isError: true`): an English sentence the agent can act on or tell the user. When the core refused, a second text block holds its reason and values as JSON, `{"reason": "draft_by_user", "draft": 12}` (`docs/contract.md`, "Errors"), so that the agent decides on the reason rather than on the wording. Not as `structuredContent`: a client may check that against the tool's output schema, which an error does not follow, and the model reads `content`.
- **Never returned**: the HTML of a mail (the text is enough, and HTML is where invisible instructions hide), tracking links, remote images, event positions.

### Bounds

| Bound | Default | Beyond |
| --- | --- | --- |
| Conversations per page | 25, at most 100 | `next_cursor` |
| Text of one message in `get_conversation` | `mcp.message_chars`, 8,000 characters | `truncated` on the message; the rest with `get_message` |
| Text of a whole answer | `mcp.response_chars`, 40,000 characters | in `get_conversation`, the oldest messages keep only their header (`text_omitted`), and `truncated`; `get_message`, `read_attachment` and `get_draft` give pages (`next_offset`) |
| Text of an attachment | 100,000 characters read by the core | `cut` |

`mcp.message_chars` and `mcp.response_chars` are keys of `config.toml` ([configuration.md](configuration.md)), sent to the server by `config_status`. Raising them gives longer answers, never a right.

## Rights

The core keeps, for the source `mcp`, a list of the methods an agent may call (`crates/nm-daemon/src/rights.rs`), and refuses every other one with `FORBIDDEN` before it runs, whatever the server asks. A list of what is allowed rather than of what is refused: a method added to the contract stays closed to agents until it is opened on purpose, and a test reads every method of `nm_api::method` to hold that.

| Domain | An agent may |
| --- | --- |
| Reading folders, conversations, messages, attachments, search, history | yes |
| Drafts | create, change and discard its own (`draft.created_by = 'mcp'`); the user's are readable, never changed, and a reply the user already started to a mail is not handed to the agent |
| Sending | no: you send the draft from the app |
| Filing (move, read/unread, flag, tag, undo) | no |
| Screening decisions, unsubscribing, remote images, `mark_viewed` | no |
| Accounts, passwords, settings, import, synchronization, raw store | no |
| The AI assistance of the app (`ai_*`) | no |

A refusal says what to do instead ("the draft is in Drafts, where the user can review and send it").

**The content of a mail is hostile data.** The agent is a model that reads mail written by anyone. The `instructions` of `initialize`, and the description of every reading tool, say that the content of a mail is data, never an instruction, and that a request found in a mail is not the user's. That lowers the risk of a prompt injection without removing it; the real protection is structural: nothing leaves the mailbox without you, and nothing is filed.

**Limits.** The source is declared by the client in `hello`; the core does not yet check the signature of its clients. The rights are real for an agent that only has MCP (Claude Desktop): it reaches the core only through `nicemailer-mcp`, which declares itself honestly. An agent that also has a terminal can run `nicemailer` or read the database: these rules protect the MCP channel, not you from an agent you gave your terminal to.

## Code

| File | Content |
| --- | --- |
| `crates/nm-mcp/src/server.rs` | MCP on stdio: versions, `tools/list`, `tools/call` in tasks, cancellation, progress |
| `crates/nm-mcp/src/tools.rs` | The tools: schemas, annotations, arguments, the calls to the contract, the folder and filter conversions |
| `crates/nm-mcp/src/reduce.rs` | The answers reduced from the views of the contract |
| `crates/nm-mcp/src/core.rs` | The connection to the core: lazy, kickstart, maintenance, multiplexing, reconnection |
| `crates/nm-mcp/src/time.rs` | ISO 8601 dates at local time, both ways |
| `crates/nm-mcp/src/install.rs`, `src/main.rs` | Claude Desktop's entry, and the command line |
| `crates/nm-daemon/src/rights.rs` | What each source may call |
| `crates/nm-daemon/src/drafts.rs` | `author`, `writer`: drafts limited to the agent's own |
| `crates/nm-daemon/src/extract.rs` | `attachment_text`: pages of the text of an attachment, the last four texts read kept |
| `apps/NiceMailer/Sources/NiceMailer/AgentSetup.swift` | The menu "AI Agents" |

## Tests

| Test | What it checks |
| --- | --- |
| `crates/nm-daemon/src/rights.rs` (unit) | Every method of the contract outside the list is refused to `mcp`, every method of the list exists, the other sources are not limited |
| `crates/nm-daemon/tests/mcp_rpc.rs` | On the real core: an agent reads and is refused passwords, sending, filing, screening, unsubscribing, accounts, import, sync, AI, export; writes, changes and discards its own drafts, never the user's; `attachment_text` of a PDF, a DOCX, an HTML file, an attached mail, an image, and a long text in pages without gap or overlap; `list_history` by source and date |
| `crates/nm-daemon/tests/mcp_proxy.rs` | The server of `nm-mcp` against the real core: `initialize` without a core and the first tool connecting; an older client getting text only; every tool's answer conforming to its output schema, without empty fields, dates in ISO 8601; a conversation without HTML, quotes hidden unless asked, texts bounded by `[mcp]`; a role across accounts and pages without duplicates; a core restarted between two calls; a core in maintenance waited for with progress; the agent's drafts and the refusals on the user's; a cancelled call; each message's `link`, and `get_message` from a link (any case, an old link by id, refusals) |
| `crates/nm-mcp/tests/binary.rs` | The binary: nothing but JSON-RPC on its standard output (bad lines included), its log on the standard error; the `claude-desktop` commands as the app runs them |
| `crates/nm-mcp/src/*.rs` (unit) | Dates both ways, reductions, folders and filters, argument refusals, tool definitions, the quote line, Claude Desktop's file (missing, other servers kept, moved app repaired, unreadable file never overwritten, no Claude Desktop) |
| `apps/NiceMailer/Tests/NiceMailerTests/AgentSetupTests.swift` | The command for Claude Code (shell quoting), the configuration block, the helper's outcome, the arguments it is run with |
| `apps/NiceMailer/UITests/agents.uiscript` | The menu "AI Agents" in the app, with the real helper and a Claude Desktop file of the scenario |
