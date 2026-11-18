# The core contract

Every client (the Swift app, the `nicemailer` CLI, the MCP server `nicemailer-mcp`, later the TUI and the web) talks to the core through one contract: JSON-RPC 2.0 requests on the core's Unix socket, plus the events it pushes to subscribed clients (`specifications/architecture.md`, "Règles du contrat"). The method names are defined in `crates/nm-api/src/lib.rs` (module `method`), together with their parameter and result types.

This page lists every method, what it does, and the CLI command that uses it, if there is one. In the CLI column, "—" means no dedicated command.

## Keeping this page right

Tests in `crates/nm-cli/src/contract_doc.rs` (run by `make test`) fail as soon as this page and the code disagree:

- the methods in the tables are exactly those of `nm_api::method`, each listed once;
- every command shown here (inline code or a code-block line starting with `nicemailer`) exists, with its subcommands and options, in the clap definition of the CLI;
- the CLI column is right both ways: a method the CLI calls (through a `method::` constant, never a string) is not marked "—", and a method given a command is really called by the CLI;
- every constant of `nm_api::method` has a `///` comment.
- the table of the section "Errors" lists exactly the reasons of `nm_api::reason`, with the values of their sentences, and every reason is given by the code of some crate.

The tests cannot check that a description is still true. A change to what a method or a command does updates its row here in the same change.

When adding a method, declare it in `nm_api::method` with its comment and add its row to the right table, then fill the CLI column if a command calls it. A new CLI source file that calls the core must be added to `CLI_SOURCES` in `contract_doc.rs`.

## What each client may call

The core checks the source a client declared in `hello` before running a method (`crates/nm-daemon/src/rights.rs`). The app, the CLI and the TUI may call every method (sending is reserved to them), except the update's (`prepare_update`, `cancel_update`), reserved to the app. An AI agent, the source `mcp` (`nicemailer-mcp`, `mcp.md`), gets only an allowlist: the reading methods its tools use, `config_status`, and the draft methods, on its own drafts only (`created_by = 'mcp'`); every other method is refused with `FORBIDDEN` (1007), and a method added to the contract stays refused to agents until it is added to that list. The source is declared, not yet checked against the client's signature (`specifications/architecture.md`, section 9).

## Calling any method from the CLI

`nicemailer call` sends any method to the core and prints the JSON result, so every method below can be called from a terminal, even the ones without a dedicated command:

```sh
nicemailer call list_threads '{"folder_id": 1}'
nicemailer call get_thread '{"thread_id": 42}'
nicemailer call ping
```

The CLI makes the `hello` call by itself when it connects. All commands accept `--data-dir <dir>` to target another mailbox folder.

A dedicated command prints readable output, asks for confirmation when needed and often chains several calls (`account add` makes six). `call` only prints raw JSON.

## Session and events

| Method | What it does | CLI |
| --- | --- | --- |
| `hello` | First call of every client: checks the protocol version, gives the core version, names the client (recorded as `modified_by`). In maintenance, refused with `MAINTENANCE` (1003), whose `data` says why ([updates.md](updates.md)), except for the app during an update. | Made automatically on connection; `--wait` waits for the end of an update or a migration. |
| `ping` | Answers `"pong"`: checks the core is alive. | — (`nicemailer call ping`) |
| `subscribe` | Subscribes the connection to the core's events. With `since`, replays the events the client missed, if the core still has them. | `nicemailer watch` prints the events as they happen. |

## Reading mail

| Method | What it does | CLI |
| --- | --- | --- |
| `list_folders` | Every folder of every account, with its role and counts. | `nicemailer folders` prints account, folder, total and unread. |
| `list_threads` | A page of threads (`ThreadSummary`) of a folder, of several folders (unified view) or of "Everything", with filters, sort and an opaque cursor. | — |
| `get_thread` | A whole thread, ready to display (`ThreadView`, `MessageView`), with its tags, the person it is with and that person's groups. | — |
| `search` | Full-text search from plain text typed by the user. | — |
| `locate_message` | The thread and folder of a message, from its id: what opens a `nicemailer://m/<id>` link (`deep-links.md`). | — |
| `message_text` | The text of one message, one line per paragraph, for the clipboard. | — |
| `message_record` | Everything about one message, read from its raw file, for printing it as evidence. | — |
| `export_message` | Writes one message to a file (`.eml`, `.txt`, `.md`). | — |
| `get_attachment` | Extracts an attachment from the raw message on first use and returns its read-only path. | — |
| `attachment_text` | The text of an attachment (text, HTML, PDF, DOCX, mail attached), one page of characters at a time (`offset`, `limit`, `next_offset`); a file whose text cannot be read gives `unreadable` and its path instead of an error. Used by `read_attachment` of `nicemailer-mcp` (`mcp.md`). | — |
| `set_body_format` | Remembers whether the mails of an address are shown as text or HTML. | — |

## Acting on threads

| Method | What it does | CLI |
| --- | --- | --- |
| `move_thread` | Moves a thread to the folder of a role (archive, todo…), and may take the screening decision on its sender. Returns an undo token. | — |
| `move_threads` | The same for several threads in one request. | — |
| `set_thread_seen` | Marks one or several threads as read or unread. | — |
| `set_thread_flagged` | Flags or unflags one or several threads. | — |
| `tag_thread` | Adds or removes a tag on every message of one or several threads. Creates the tag if needed. | — |
| `list_tags` | The tags of an account, or of every account, with their message counts. | — |
| `rename_tag` | Renames a tag in every account that has it. Only the displayed name changes, the IMAP keyword stays. Refuses a name another tag of the account has. | — |
| `delete_tag` | Deletes a tag in every account that has it, except read-only ones: removes it from every message (server included, through the spool), then keeps the tag as a tombstone. | — |
| `unsubscribe` | Unsubscribes from a mailing list: in one click when the core can, otherwise returns the page or `mailto:` to open. | — |
| `list_history` | The audit log, newest first, paginated (`specifications/tracabilite.md`); `since` (Unix time) and `sources` (`["mcp"]`: what AI agents did) filter it. | — |
| `undo` | Undoes a move or a decision, from the token it returned. | `nicemailer undo <token>` |

## Screening, senders, groups, contacts

| Method | What it does | CLI |
| --- | --- | --- |
| `set_address_screening` | Takes, changes or removes the screening decision on an address, and files the mails already received from it. | `nicemailer address <addr> <inbox\|newsletter\|paperwork\|banned\|none>`. Prints the undo token. |
| `set_domain_screening` | The same for a domain and its subdomains, for the addresses that have no decision of their own. | `nicemailer domain <domain> <target>` |
| `set_sender_kind` | Forces an address to be a person or a robot, or hands it back to the computed verdict (`senders.md`). | — |
| `list_groups` | The contact groups, with their member count, latest change and mails received from their members. | — |
| `rename_group` | Renames a contact group; its members stay. Refuses the name of another group. | — |
| `delete_group` | Deletes a contact group (a tombstone); no mail changes. Returns how many members it had. | — |
| `set_group_member` | Adds or removes an address, or the person a thread is with, in a group. Creates the group if needed. Refuses the user's own addresses. | — |
| `thread_labels` | For up to 1,000 threads: their tags, the person each is with and that person's groups, so that a client can offer to remove them from a selection. | — |
| `list_contacts` | The known addresses, with their decision and groups. | — |

## Import

| Method | What it does | CLI |
| --- | --- | --- |
| `import_mbox` | Starts importing an mbox file into an archive account, in the background. Progress comes through `ImportProgress` and `ImportFinished` events. | `nicemailer import-mbox <file> --account <name> --address <addr> [--no-correspondents]` (`import-mbox.md`) |
| `import_status` | The state of the running import, or of the last one. | `nicemailer import-status` |

## Accounts and settings

| Method | What it does | CLI |
| --- | --- | --- |
| `list_accounts` | Every account (IMAP and archive) with its state, in display order, and whether it is only looked at (`read_only`). | `nicemailer accounts` |
| `config_status` | The state of `config.toml` and `accounts.toml`: paths, the error that makes the core refuse them, warnings, and the settings the clients apply themselves (`ui`, `mcp`). | — (`nicemailer config check` checks the files itself, without the core.) |
| `keymap_get` | The keys of the app: the defaults with `keymap.toml` applied, by context and by menu, each key in its canonical spelling; the error that makes the core refuse the file (the previous keys then stay), and warnings. `KeymapChanged` says when to read it again (`keymap.md`). | — (`nicemailer keymap check` and `show` read the file themselves, without the core.) |
| `discover_account` | Finds the servers of an address before the account exists: provider, IMAP server, SMTP servers to try. | Step 1 of `nicemailer account add`. |
| `test_account` | Tries the IMAP server of an account not created yet: capabilities, folders with proposed roles, the folder each role will get and whether it exists (`roles`), counts for the import. A failure says in `data.reason` whether the password (`imap_auth_refused`) or the server (`imap_unreachable`) is at fault. | Step 2 of `account add`. |
| `count_archive` | Exact count of the mails the initial archiving would move. | Step 3 of `account add` (`--archive-read-before`, `--archive-unread-before`, `--no-archive`). |
| `test_smtp` | Tries SMTP servers in order, until one accepts the login; names the one that refused the password (`refused`) when none did. With `smtp_password`, tries that password instead of the IMAP one, on any server. | Step 4 of `account add` (skipped with `--no-smtp`). |
| `add_account` | Creates an IMAP account: folders, roles, initial import, password (and the SMTP server's own, `smtp_password`), `accounts.toml`. | Last step of `nicemailer account add [key] --email <addr> [--host …] [--role ROLE=FOLDER] [--read-only] [--yes]` (`accounts.md`) |
| `set_folder_roles` | Changes the workflow role of folders of an account. | `nicemailer account roles <key> todo=Todo archive=Archives [--create-missing]` |
| `prepare_activation` | What organizing an account only looked at (`read_only`) would do: the report of `test_account`, with the password of the Keychain; nothing written. | First step of `nicemailer account activate <key>` (`accounts.md`, "Looking first") |
| `activate_account` | Organizes an account only looked at, as if created then: folders, roles, the screening boundary set again, correspondents, archiving, then `read_only` removed from `accounts.toml`. | Last step of `nicemailer account activate <key> [--role ROLE=FOLDER] [--no-archive] [--no-correspondents] [--yes]` |
| `rename_account` | Renames the key of an account, keeping its data. | `nicemailer account rename <from> <to>` |
| `remove_account` | Deletes an account, its data in NiceMailer and its password. The mail on the server is not touched. | `nicemailer account remove <key> [--yes]` |
| `set_account_order` | Changes the display order of the accounts. | `nicemailer account order <key>…`: the keys named come first. |
| `set_password` | Saves the IMAP (or SMTP) password of an account in the Keychain. | `nicemailer account password <key> [--smtp]`: asked without echo, or read from stdin. |
| `probe_account` | Connects to the IMAP server of an existing account and reports what it offers, without changing anything. | `nicemailer imap probe <key> [--json]` (`imap.md`) |

## Synchronization and raw store

| Method | What it does | CLI |
| --- | --- | --- |
| `sync_status` | The synchronization state of each IMAP account: connection, last pass, folders, bodies left to download. | `nicemailer sync status [key] [--json]` (`sync.md`) |
| `sync_now` | Synchronizes an account, or every account, now. | `nicemailer sync now [key]` |
| `sync_verify` | Compares server and database folder by folder (UIDs, flags, raw files), and can repair the raw files. | `nicemailer sync verify <key> [--skip-hash] [--repair] [--json]`: exit status 1 if they differ. |
| `raw_gc` | Runs the garbage collector of the raw store now (the core runs it every hour anyway). | `nicemailer raw gc` |

## Drafts and sending

| Method | What it does | CLI |
| --- | --- | --- |
| `prepare_draft` | What a reply or forward key opens, without writing anything: the draft already started, or the one the core proposes. | — |
| `create_draft` | Writes a draft (reply to all by default) with recipients, subject and quotation prepared by the core, or returns the one already started. | — |
| `open_draft` | The latest version of a draft, marked open. | Used by `nicemailer ai run`. |
| `update_draft` | Saves what was typed: the body, and the recipients or subject when they changed. | — |
| `add_draft_attachment` | Attaches a file the app put in `incoming.noindex`. | — |
| `remove_draft_attachment` | Removes an attachment from a draft. | — |
| `parse_addresses` | Reads the addresses typed or pasted in a recipient line: the valid ones and the pieces that are not addresses. | — |
| `suggest_recipients` | The addresses to suggest for the text typed in a recipient line, the most written-to first. | — |
| `list_senders` | The addresses a mail can be sent from (accounts with `[smtp]`, not read-only). | — |
| `compose_formats` | The fixed lists of the formatted mode (fonts, sizes, colours) and the mode a new draft starts in (`editor.md`). | — |
| `html_to_text` | The plain text of a formatted draft's HTML, and whether it had any formatting. | — |
| `close_draft` | Tells the core the draft window closed. | Used by `nicemailer ai run`. |
| `discard_draft` | Abandons a draft and returns it, so that the client can restore it. | — |
| `send_draft` | Queues a draft for sending, after the undo delay (`sending.md`). | — |
| `cancel_send` | Cancels a send during the undo delay: the draft is editable again. | — |

## AI assistance

| Method | What it does | CLI |
| --- | --- | --- |
| `ai_status` | Whether the AI is on, the provider and model, whether a run can start, the problems of the prompt files. | `nicemailer ai status [--check]`: `--check` also asks the `claude` binary its version and subscription (`ai.md`). |
| `ai_list_prompts` | The prompts of one screen's AI menu, in a folder. | `nicemailer ai prompts list [--screen compose] [--folder <role>] [--lang en]` |
| `ai_run` | Starts a run. Answers at once with its id. The text arrives through `AiDelta`, `AiDone` and `AiFailed` events. | `nicemailer ai run <prompt> --draft <id> [--variant <key>] [--lang en] [--apply] [--choice <n>]` streams the proposal as it comes. |
| `ai_get_run` | The state and the proposal of a run. | Used by `ai run` once the run is done. |
| `ai_apply` | Applies the proposal of a finished run to the draft. | `ai run … --apply` |
| `ai_dismiss` | Turns down the proposal of a run, or stops it if it is still running. | `ai run` without `--apply`, or when the run fails. |
| `ai_cancel` | Stops a run. | — |
| `ai_suggestions` | The suggested targets waiting for the user, for the threads shown. | — |
| `ai_dismiss_suggestion` | Sets one suggested target aside. | — |

## Updates of NiceMailer

Reserved to the app, which Sparkle updates ([updates.md](updates.md)); the CLI, the TUI, the web and agents get `FORBIDDEN`.

| Method | What it does | CLI |
| --- | --- | --- |
| `prepare_update` | Called just before the update is installed: the core enters maintenance (every other client gets `MAINTENANCE`, 1003, and is disconnected), stops its IMAP workers, waits for the write in progress, and stays alive until its program is replaced, `cancel_update`, or `core.update_timeout_secs`. Answers the timeout. | — |
| `cancel_update` | The update was abandoned: the core leaves the maintenance and starts its workers again. Answers whether an update was being prepared. | — |

## CLI commands outside the contract

These commands work on files directly and do not go through the core:

- `nicemailer seed`: fills the database with fake mail. The core must be stopped.
- `nicemailer mailbox check | upgrade | backup`: the database of a mailbox folder (`migrations.md`, `backup.md`).
- `nicemailer config check | defaults | path`: the settings files (`configuration.md`).
- `nicemailer keymap check | defaults | show`: the keys of the app, `keymap.toml` (`keymap.md`).
- `nicemailer ai prompts export | check`: the prompt files (`ai.md`).

## Coverage

Of the 75 methods, 32 are used by a dedicated CLI command. The reading of mail, the actions on threads and the drafts have no dedicated command yet, only `nicemailer call`. The CLI is meant to be the test bench of the contract (`specifications/architecture.md`, step 4), so these are the gaps to fill first.

## Errors

A failed request answers a JSON-RPC error: a `code` (the family, `nm_api::error_code`), a `message`, and `data` with a `reason` and the values of its sentence (`specifications/localisation.md`, section 3.1):

```json
{"code": -32602, "message": "the tag Urgent exists already", "data": {"reason": "tag_exists", "tag": "Urgent"}}
```

- `reason` says why, in a stable word: a client branches on it and writes the sentence in the user's language (the app: `CoreReason.swift`). The core does not know the user's language.
- `message` is the English sentence of the reason, its `{value}` filled in. The CLI prints it, the logs keep it, and AI agents read it.
- A value whose key ends with `_text` was written by someone else (the mail server, the system, the AI provider): it is cited as it is, never translated. `detail` is a technical precision in English, for the generic reasons (`invalid_params`, `not_found`, `internal`…), which are mistakes of a client's program rather than of the user.
- Some codes add to `data` values outside the sentence: the AI's `code` (`ai.md`), the current `revision` of a draft. The `data` of `MAINTENANCE` (1003) is a whole `Maintenance`, whose `reason` is one of the four at the end of the table.

Events and results that say why something failed carry the same object as `why`, beside their English text: `AccountError` and the accounts of `list_accounts` (`error`, `why`), `ActionReverted`, `SendFailed` and `AiFailed` (`message`, `why`). `SendPartial` gives each refused address with the server's answer, cited. Some texts stay technical details in English, cited by the app inside its own sentence: the problems of the settings files (`ConfigProblem`), the error of a core that could not start (`StartupError`), a lookup of MX servers (`mx_error`), a one-click unsubscribe that failed, the CLI's error when it checks a mailbox, and the error of a past AI run read back with `ai_get_run`.

The app, the core, the CLI and the MCP server are shipped and updated together, so no client meets a reason it does not know: the list is not versioned. Tests keep it whole: the reasons below are exactly those of `nm_api::reason`, with the values of their sentences (`crates/nm-cli/src/contract_doc.rs`); each one is used by the code; the app writes each one, or says it is generic (`LocalizationTests`).

| Reason | Values | English message |
| --- | --- | --- |
| `parse_error` | `detail` | not JSON: {detail} |
| `invalid_request` | `detail` | invalid request: {detail} |
| `method_not_found` | `method` | unknown method: {method} |
| `invalid_params` | `detail` | invalid parameters: {detail} |
| `internal` | `detail` | internal error: {detail} |
| `not_found` | `detail` | not found: {detail} |
| `hello_expected` | — | the handshake hello is expected first |
| `protocol_version` | `version`, `expected` | protocol {version} is not supported (expected {expected}) |
| `reserved_to_people` | — | only the app, the TUI and the CLI can do this |
| `reserved_to_app` | `method` | {method} is reserved to NiceMailer.app, which installs the updates |
| `agent_cannot_send` | — | AI agents cannot send mail yet: the draft is in Drafts, where the user can review and send it from NiceMailer |
| `agent_cannot_file` | — | AI agents cannot file mail yet (move, mark, flag, tag): ask the user to do it in NiceMailer |
| `agent_cannot_unsubscribe` | — | unsubscribing sends a request to an address the sender chose, which confirms the mail was read: only the user can do it, in NiceMailer |
| `agent_cannot_set_password` | — | passwords are given in NiceMailer only |
| `agent_not_allowed` | `method` | {method} is not available to AI agents |
| `draft_by_user` | `draft` | draft {draft} was written by the user: AI agents may read it, not change it; start a new draft instead |
| `reply_started_by_user` | `draft` | the user has already started a reply to this mail (draft {draft}): AI agents may read it, not change it |
| `account_unknown` | `account` | no account {account} in accounts.toml |
| `account_not_in_database` | `account` | account {account} is not in the database |
| `account_not_synchronized` | `account` | account {account} is not synchronized |
| `no_imap_account` | — | no IMAP account to synchronize |
| `account_exists` | `account` | an account {account} exists already |
| `account_key_taken` | `account`, `kind` | {account} is an {kind} account: choose another key |
| `account_set_up` | `account` | the account {account} is not read-only: it is set up already |
| `account_read_only` | — | this account is read-only (read_only = true in accounts.toml): nothing is changed |
| `account_read_only_send` | — | this account is read-only (read_only = true in accounts.toml): it cannot send |
| `password_empty` | — | empty password |
| `smtp_password_empty` | — | empty SMTP password |
| `smtp_password_without_server` | — | an SMTP password without an SMTP server |
| `no_smtp_server` | `account` | account {account} has no [smtp] server in accounts.toml: it cannot send |
| `date_invalid` | `date` | date {date}: YYYY-MM-DD expected |
| `read_only_archives_nothing` | — | a read-only account archives nothing on its server |
| `server_folder_missing` | `folder` | the server has no folder {folder}: ask to create it |
| `keychain` | `system_text` | the Keychain refused: {system_text} |
| `password_missing` | `account` | no password saved for this account: nicemailer account password {account} |
| `oauth2_unsupported` | — | auth = "oauth2" is not supported yet |
| `password_server_changed` | `saved_user`, `saved_host`, `user`, `host`, `account` | the password was saved for {saved_user} on {saved_host}, the file now says {user} on {host}: save the password again to confirm the new server (nicemailer account password {account}) |
| `smtp_password_server_changed` | `saved_user`, `saved_host`, `user`, `host`, `account` | the SMTP password was saved for {saved_user} on {saved_host}, the file now says {user} on {host}: save it again to confirm the new server (nicemailer account password {account} --smtp) |
| `smtp_password_needed` | `smtp_host`, `host`, `account` | the SMTP server {smtp_host} is not of the same provider as the IMAP server {host}: NiceMailer does not send it the IMAP password. Save its own: nicemailer account password {account} --smtp |
| `imap_auth_refused` | `server_text` | the server refused the login: {server_text} |
| `imap_unreachable` | `server_text` | the server could not be reached: {server_text} |
| `imap_failed` | `server_text` | the server failed: {server_text} |
| `mail_not_in_account` | — | this mail is not in an account of accounts.toml (an archive account?): it cannot be answered from here |
| `no_account_can_send` | — | no account of accounts.toml can send: add an [smtp] server to one |
| `archive_cannot_send` | — | this mail is in an archive account, which cannot send |
| `no_sender_address` | — | the account has no address to send from |
| `not_own_address` | `address` | {address} is not an address of this account |
| `draft_no_recipient` | — | this draft has no recipient |
| `draft_cannot_be_answered` | — | a draft cannot be answered |
| `draft_cannot_be_forwarded` | — | a draft cannot be forwarded |
| `formatted_text_too_big` | — | the formatted text is too big |
| `revision_conflict` | `revision` | this draft was changed elsewhere (revision {revision}): reload it |
| `draft_sending` | — | this draft is being sent: cancel the sending first |
| `send_started` | — | too late to cancel: the mail is leaving, or has left |
| `draft_copying` | — | this draft is being copied to the server: try again in a moment |
| `body_pending` | — | the body of this mail is not downloaded yet: it is asked for, try again in a moment |
| `raw_damaged` | `hash` | the raw file of this mail is damaged ({hash}): it is set aside and downloaded again from the server if possible |
| `server_refused_change` | `server_text` | the server refused the change: {server_text} |
| `attempts_failed` | `attempts`, `server_text` | {attempts} attempts failed, the last: {server_text} |
| `smtp_refused` | `server_text` | the mail server refused the mail: {server_text} |
| `move_refused` | `target`, `from` | {target} is not a target from {from} |
| `confirmation_required` | `addr`, `verdict` | confirmation required: probable spoofing of {addr} (verdict {verdict}) |
| `tag_name_invalid` | — | a tag name has 1 to 64 characters |
| `tag_exists` | `tag` | the tag {tag} exists already |
| `group_name_invalid` | — | a group name has 1 to 64 characters |
| `group_exists` | `group` | the group {group} exists already |
| `address_invalid` | `address` | not an address: {address} |
| `domain_invalid` | `domain` | not a domain: {domain} |
| `import_running` | — | an import is already running |
| `path_not_absolute` | `path` | an absolute path is expected: {path} |
| `not_a_file` | `path` | {path} is not a file |
| `folder_missing` | `path` | no folder {path} |
| `file_unreadable` | `path`, `system_text` | {path}: {system_text} |
| `database_not_empty` | — | the database already holds mail |
| `ai_disabled` | — | the AI assistance is off: set ai.enabled = true in config.toml |
| `ai_provider_missing` | — | the claude binary was not found; install Claude Code or set ai.claude_path |
| `ai_not_logged_in` | — | Claude Code is not signed in; run `claude` in a terminal to sign in |
| `ai_provider_unsafe` | `tools`, `mcp_servers` | the claude binary announced tools ({tools}) and MCP servers ({mcp_servers}); nothing was sent to it |
| `ai_provider_unsupported` | `provider` | the {provider} provider is not available yet: set ai.provider = "claude-code" in config.toml |
| `ai_rate_limited` | — | the usage limit of the Claude subscription is reached |
| `ai_timeout` | — | the AI did not answer in time |
| `ai_invalid_output` | `detail` | the AI's answer could not be read: {detail} |
| `ai_failed` | `provider_text` | the AI failed: {provider_text} |
| `ai_io` | `system_text` | cannot run the claude binary: {system_text} |
| `ai_prompt_unavailable` | `prompt` | the prompt {prompt} is not available in this version of NiceMailer yet |
| `ai_prompt_not_here` | `prompt` | the prompt {prompt} is not offered in this folder |
| `ai_selection_needed` | — | select the text to work on first |
| `ai_selection_unwanted` | — | this prompt works on the whole text: send no selection |
| `ai_selection_empty` | — | the selection is empty |
| `ai_text_empty` | — | there is no text of yours to send: write some first |
| `ai_segment_tag` | — | the text contains a <segment> tag, which the AI uses to give the text back; remove it first |
| `ai_mail_downloading` | — | this mail is still downloading: try again in a moment |
| `ai_folder_without_role` | — | this folder has no role: nothing to sort it into |
| `ai_folder_empty` | — | this folder is empty: nothing to sort |
| `ai_mark_to_sort` | — | mark the conversations to sort, or put the cursor on one |
| `ai_draft_unsaved` | — | the draft changed since: save it and run again |
| `ai_cancelled` | — | cancelled |
| `ai_draft_changed` | — | the draft changed since the AI read it: run it again |
| `file_unsupported` | `file` | {file}: this kind of file has no text the AI can read |
| `file_too_large` | `file`, `size_mb` | {file}: the file is too large to read ({size_mb} MB, 50 MB at most) |
| `file_encrypted` | `file` | {file}: the file is protected by a password |
| `file_without_text` | `file` | {file}: no text in this file: a scan or an image? |
| `file_damaged` | `file`, `detail` | {file}: the file cannot be read: {detail} |
| `update` | — | NiceMailer is being updated |
| `migration` | `from`, `to` | the database is being upgraded (version {from} to {to}) |
| `newer_schema` | `from`, `to` | the database was written by a newer NiceMailer (version {from}, this core reads {to}) |
| `migration_failed` | `message` | the upgrade of the database failed: {message} |
