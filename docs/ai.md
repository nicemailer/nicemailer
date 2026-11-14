# AI assistance

NiceMailer can ask a language model to help with mail: proofread a draft, summarize a conversation, suggest where a mail of the Quarantine should go. The rules are in `specifications/ia.md` (in French). This page describes what is built so far and how it is tested.

**Status (November 2026):** the core runs the prompts of the compose screen (proofread, rewrite, translate, subjects) and those of a mail or a conversation (summary, translation, quick replies, security check, dates) through the contract, with the user's own `claude` binary. The CLI (`nicemailer ai`) and the app (⌘I in a reply) run the first ones. The core also sorts several conversations in batches (the Quarantine); the attachments and the API-key provider come next.

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

## Prompts

A prompt is a TOML file. The built-in ones are in `crates/nm-ai/prompts/`, compiled into the binary; the user's are in `~/.config/nicemailer/prompts/` (next to `config.toml`), a folder NiceMailer reads but never creates. `nm_ai::catalog::load_dir` merges them into a `Catalog`, and `Catalog::menu(screen, folder)` gives the prompts of one AI menu, sorted by key.

| Built-in prompt | Screen, key | Does |
| --- | --- | --- |
| `proofread` | compose, `c` | Fixes spelling and grammar only, keeping the writer's typography (`14h` stays `14h`) |
| `rewrite` | compose, `r` + `f` `a` `c` `l` `s` | More formal, friendlier, shorter, longer, clearer |
| `translate-draft` | compose, `t` + `e` `f` `s` `d` `i` | Into English, French, Spanish, German, Italian |
| `write-from-notes` | compose, `w` | Turns notes into a mail, with `[placeholders]` for what is unknown |
| `subject-ideas` | compose, `o` | Three subject lines |
| `summarize-thread` | reading and list, `r` | Summary and action items (owner, due date) |
| `quick-replies` | reading and list, `p` | Three short replies, one of which starts a reply |
| `translate-message` | reading, `t` + language | The mail translated, in the panel |
| `security-check` | reading, `s` | Phishing risk and reasons |
| `dates` | reading, `d` | Dates and appointments the mail mentions |
| `summarize-attachment` | attachment, `r` | What the file is and what matters in it |
| `triage-quarantine` | list in Quarantine, `s` | A target per conversation; with nothing marked, the whole folder |
| `suggest-folder` | list elsewhere, `s` | A target per marked conversation, or the one under the cursor |

### Writing your own

Copy a built-in prompt to start from (`nicemailer ai prompts export <id>`, coming with the CLI), or write a file:

```toml
version = 1
id = "polite-no"
scope = "thread"
key = "n"
label = { en = "Polite refusal", fr = "Refus poli" }
input = ["thread.text", "user.identity"]
output = "reply"
lang = "content"
system = """
Write a short, warm reply that declines what the newest message asks, without giving reasons
the conversation does not contain.
"""
```

- The file's `id` decides what it does: the id of a built-in prompt replaces it, another id adds a prompt, and `enabled = false` hides the prompt of that id (the rest of such a file is not checked).
- `system` describes the task only. NiceMailer builds the system prompt around it: a `TASK=<id>` line, your text, the format the answer must follow (taken from `output`), the language rule (`lang`) and a guard against instructions hidden in mail.
- The full list of keys, scopes, inputs and outputs, and the rules checked, are in `specifications/ia.md`, sections 4 to 6.
- A file with a problem is refused alone, with its line; the other prompts stay. An unknown key is only a warning.
- A key must be free in the menu where the prompt appears. The reading screen shows the `message` and `thread` prompts together, and the list shows `selection` and `thread`. A conflict with a built-in prompt is reported on your file.

## The user's text in a draft

Proofreading must touch the user's words only. `nm_core::segments::own_segments(body)` finds them in a draft's body, as byte ranges, and `segments::splice(body, ranges, new_texts)` puts the rewritten texts in their place, every other byte unchanged. What is not the user's text is recognised by its form, since the app writes the reply and forward headers in its own language and the core does not store them:

- quoted lines (`>`), and the line just above a quotation that ends with `:` ("On Nov 7, 2026, at 10:12, Alice wrote:");
- a forwarded mail: everything from a line of dashes around a text ("---------- Forwarded message ---------");
- a signature: from `-- ` (or `--`) to the next quoted line or the end.

The rest, cut where quoted lines interrupt it, makes the segments; a reply written between quoted lines has several. A segment has no blank line or whitespace around it, and `splice` keeps the whitespace that surrounded the original, so the model never decides the layout. In doubt, a line is left out: then it is only not proofread. With a selection, the selection alone is one segment.

`nm_ai::message::segments(texts)` writes them for the model as `<segment n="1">…</segment>`, the form `answer::segments` reads back. A text that already contains a segment tag is not sent (`ai_bad_input`).

## Inputs of a mail

The prompts of the reading and list screens take `thread_id` (and `message_id` for a mail of it; absent, the latest mail received, the latest of all when every one is the user's). The core builds their inputs from the conversation as the reader shows it (`get_thread`), in `crates/nm-daemon/src/ai.rs`, rendered by `nm_ai::message`:

| Input | What the model gets |
| --- | --- |
| `message.text` | What the sender wrote: interleaved quotes marked `> `, the quoted history at the end and the copies of earlier mails left out (`docs/thread-display.md`); 16,000 characters at most, `[…]` when cut |
| `message.headers` | From, To, Cc, the date (at the mail's own offset of this Mac's time zone: a mail of June is in summer time), Subject, the files attached |
| `thread.text` | Every mail, oldest first, each as above and cut at 3,500 characters, the user's marked "(the user)"; beyond 16,000 in all, the oldest are left out and the section says how many |
| `message.auth` | The authentication verdict, and what the core found: a failed authentication, a link whose text names another site than it opens (organizational domains compared, `nm_store::auth`), a link to a bare IP address |
| `sender.history` | The decision on the address and on its domain, the mails received from it, whether the user wrote to it, person or robot |
| `user.identity` | The user's addresses: those of `accounts.toml`, and of the accounts in the database (an archive account) |

What the core found raises a `low` security verdict to `medium` (`answer::apply_floor`), its first strong finding first among the reasons. The Reply-To of the specification is not read yet: the reader does not give it.

Their proposals: `text` (a translation, the dates), `summary` (with its action items), `verdict` (`risk`, `reasons`, `raised`), `choices` (`then: reply`). A text, a summary or a verdict is only read: `ai_dismiss` closes it without a decision, and `ai_apply` has nothing to do with it. A quick reply is applied by the client: `ai_apply {run_id, choice}` records it and gives its text back, and the client starts the reply with it.

## Sorting several conversations

The prompts of several conversations (`triage-quarantine`, `suggest-folder`) take `folder_ids`, the folders shown (one per account in the view of all accounts), and `thread_ids`, the conversations marked. None marked, the prompt's `unmarked` decides (`ai_list_prompts` gives it): `cursor`, the client sends the one under its cursor; `folder`, the core takes the whole folder, newest first, at most `ai.max_folder_items`. The targets are those of the folder's role (`FolderRole::move_targets`), and the answer's schema allows only them.

The core sends the conversations in batches of `ai.batch_size`, one after the other, building each batch when its turn comes: the first suggestions show after a few seconds, even for a Quarantine of 500. Each conversation is given by its id, with its latest mail received (the latest of all when every one is the user's): sender, subject, date, authentication and what the core found, what the mailbox knows of the sender, files, the start of its text (1,500 characters). A conversation whose mail is not downloaded yet is left out.

As each batch ends, its suggestions are kept (`ai_suggestion`) and announced (`AiSuggestions {run_id, thread_ids}`), then the progress (`AiProgress {run_id, done, total}`, transient). A batch whose answer cannot be read (a conversation not sent, a target not offered) gives no suggestion, and the next batches go on: one bad answer does not sink twenty conversations. A failure of the provider itself (not signed in, the limit, a timeout) stops the run, since the next batches would fail the same; so does a run where no batch could be read. The proposal (`targets`) lists the suggestions and the batches that failed; the run's input names the conversations sent, its output keeps every batch's raw answer.

`ai_suggestions {thread_ids}` gives the suggestion waiting for each conversation shown (the latest, when one was sorted twice). A move decides it: `accepted` when it went to the target suggested, `other` otherwise (`move_threads`, `move_thread`); `ai_dismiss_suggestion` sets one aside.

## Answers

`nm_ai::answer::read(output, outcome, sent)` checks what the model answered against what was asked and sent, and gives a typed `Answer` or `ai_invalid_output` with the reason. It is strict: a key the schema does not have, a target that was not offered, a conversation that was not sent, segments in another number or order are all refused, rather than guessed at. A tolerant reading would hide the drift of a prompt, and the core applies these answers.

| Output | Accepted |
| --- | --- |
| `replace` | Exactly the segments sent, `<segment n="1">` to `n="N"` in order, only whitespace around; each one trimmed, the core restoring the original whitespace |
| `insert`, `reply`, `panel` | A non-empty text |
| `choices`, `subjects` | 1 to 5 distinct texts (case ignored), 2,000 characters at most |
| `summary` | A summary and at most 8 action items, each with a task and possibly empty owner and due date |
| `verdict` | `low`, `medium` or `high`, and 1 to 5 reasons of 300 characters at most |
| `targets` | At most one suggestion per conversation sent, each to a target offered, with a reason. A conversation the model left out gets no suggestion; the rest of the batch stands |

A whole answer in a code fence (```` ```json ````) is unfenced first. A structured answer given as text, by a provider without structured output, is read as JSON.

`answer::apply_floor(verdict, findings)` raises a `low` security verdict to `medium` when the core found a strong sign in the mail itself (a failed authentication, a link that shows one site and opens another…), and puts that sign first among the reasons. The findings come from the core, with the inputs of the mail.

## History of runs

Every run is a row of `ai_run` (`crates/nm-store/src/ai_runs.rs`; schema version 13, [migrations.md](migrations.md)): the prompt, its source and hash, the provider and model, the scope and target, the language, the texts sent and received, the state, the error, the user's decision, the tokens, the cost and whether it is billed, the duration.

| Call | Does |
| --- | --- |
| `ai_run_start` | A new row, `running` |
| `ai_run_done` | `running` to `done`, with the answer, tokens, cost and duration |
| `ai_run_end` | `running` to `failed` or `cancelled`, with the error and the raw answer when there was one (an answer refused as invalid is kept, to understand why) |
| `ai_run_decide` | `accepted`, `rejected` or `partial`, once, on a `done` run |
| `ai_runs_abandoned` | At start: the runs a stop of the core left `running` become `failed` |
| `ai_runs_purge(before)` | The texts of the runs created before that date are forgotten (`ai.history_days`); the rest of the row stays |
| `ai_suggestions_add` | The suggestions of a batch, in one transaction; a suggestion goes with its conversation |

Each change checks the state it starts from: a run ends once and is decided once, so a second answer for the same run is refused instead of written over the first. Nothing calls these yet; the core will, with the contract.

## The contract

| Method | Does |
| --- | --- |
| `ai_status {check?}` | On or off, the provider and model, whether a run can start (`ready`, else `reason`), the problems of the prompt files. With `check`, asks the `claude` binary its version and the subscription (0.3 s, no call to the model) |
| `ai_list_prompts {screen, folder?, lang?}` | The prompts of a menu (`compose`, `reading`, `list`, `attachment`), labels in `lang`, with `available`: whether this version can run them yet |
| `ai_run {prompt, variant?, draft_id, revision, selection?, lang}` | Starts a run and answers `{run_id, streamed}` at once |
| `ai_get_run {run_id}` | State, decision, error, and the proposal while it can be applied |
| `ai_apply {run_id, revision, choice?}` | Applies the proposal to the draft, if the draft is still at the revision the run read; answers the draft saved |
| `ai_dismiss {run_id}` | Turns the proposal down, or cancels a run still going |
| `ai_cancel {run_id}` | Stops a run; its process group is killed |

Events: `AiDelta {run_id, text}`, transient, the fragments of a text answer; `AiDone {run_id}`; `AiFailed {run_id, code, message}`, `code` `ai_cancelled` for a cancel. Errors: `AI_DISABLED` (1017) when `ai.enabled` is off, `AI` (1018) with the AI error code in `data.code`, `REVISION_CONFLICT` (1012) when the draft changed.

A run is checked before anything is written or sent: the AI on, the prompt known and wired, its variant, the draft's revision, some text of the user's. Then its row is written (`running`), the run waits for one of the `ai.max_parallel` places, the text comes as `AiDelta`, the answer is checked (`answer::read`) and kept as a proposal, in memory, for `ai_apply`. `ai_cancel` and the end of a run take it out of the running ones under one lock, so only one of them decides how it ended.

The user's prompts folder is read again when its files change (name, size, date), at the next use: no watch thread.

For tests, `NICEMAILER_FAKE_AI=<file.json>` makes the core answer from a table `{"<task>": "<answer>"}` instead of calling a provider.

## In the app

In a reply, **⌘I** opens the AI menu at the foot of the headers, just above the text it works on: the prompts of the compose screen, each with its letter, `…` after one that opens its variants (⌘I `r` `c`: rewrite shorter), "not yet" after one this version cannot run. If the AI is off, the status bar says how to turn it on (`ai.enabled`).

Out of a reply, **`a`** opens the AI menu of the mail shown (two or three columns, or a mail read in the whole window): `r` summary, `p` quick replies, `t` translation (then the language), `s` security check, `d` dates. A prompt of one mail works on the mail under the reader's cursor (`ctrl-j`, `ctrl-k`), a prompt of the conversation on all of it. On the list alone (one column), `a` opens the menu of the list, whose prompts (sorting the Quarantine) are not available yet. The two menus cannot be one: their prompts share letters (`s`).

The panel of a mail is in the bottom-right corner, as the choices: a translation, a summary with its action items, a verdict (its risk in a colour, "Raised by NiceMailer" when the core raised it), or the quick replies, numbered: a number starts the reply to the mail with that text above the quotation, apart from what a reply already started holds by an empty line.

A prompt chosen, the text is saved first (the core reads the draft as saved, at its revision), and the **AI panel** opens between the headers and the text:

| While | Shows | Keys |
| --- | --- | --- |
| the run goes on | the text as it comes, without the segment tags | Escape stops it |
| a replacement is proposed | the differences, removed words struck out in the error colour, added ones underlined in the accent colour | Return or `y` accepts, Escape or `n` refuses |
| subjects are proposed | the numbered list | the number chooses one; Escape or `n` refuses |
| a mail's text, summary or verdict is shown | it | Escape or Return closes it |
| quick replies are proposed | the numbered list | the number starts the reply with it; Escape or `n` refuses |
| it failed | why, in the error colour | Escape or Return closes it |

The panel takes every key while it is open: the text cannot change under a proposal, and a letter pressed only says which keys answer. Accepted, the text becomes the core's (`ai_apply`), the cursor going to the end of the words rewritten if it was in them; **⌘Z** then gives the text back, once, and only while it is still the one the AI gave.

The rules are pure functions (`AI.swift`: the key answers, the hint, the cursor, the diff as text); the model calls the core (`AppModel+AI.swift`); the panel is `AIPanel` (`ComposerView.swift`). An AI event that comes before the answer of `ai_run` is kept and replayed once the run's id is known; the end of a run is taken even after a gap of events, and after missed events the app asks the core for the state of the run it waits for.

## From the terminal

```sh
nicemailer ai status [--check]          # on or off, provider, model, ready; --check: claude's version and subscription
nicemailer ai prompts list [--screen compose|reading|list|attachment] [--folder quarantine] [--lang fr]
nicemailer ai prompts export [<id>…] [--dir …]   # copies of built-in prompts to start from; never overwrites
nicemailer ai prompts check [<dir>]     # your prompt files checked without the core; status 1 if one is refused
nicemailer ai run <prompt> --draft <id> [--variant k] [--lang fr] [--apply] [--choice N]
```

`ai run` opens the draft as the app does, starts the prompt, shows the text as it comes, then the proposal: a replacement as a word diff in the `wdiff` style, `[-removed-]{+added+}`, or the numbered subjects. Nothing changes without `--apply` (and `--choice N` for a subject); without it, the proposal is dismissed. Ctrl-C stops following, not the run.

While a text answer comes, the tags of the segments are hidden (`nm_ai::message::SegmentFilter`, in the core, for every client): a tag may be cut between two fragments, so a `<` that may start one is held back until the next fragment tells, and the text outside the segments is not shown.

Tried on 14 November 2026 with Claude Code 2.1.325, Claude Pro, `claude-haiku-5-5`: a reply proofread and applied in 1.75 s, the start of the process included; the signature and the quotation untouched, `14h` kept.

## Tests

| Test | What it covers |
| --- | --- |
| `cargo test -p nm-ai` (unit, `src/`) | The safety check (every way an announcement can fail), the reading of the stream, the fake provider |
| `tests/fixtures.rs` | The recordings of the real binary (`tests/fixtures/claude-code/`, described in their README) show what they claim, and name no user or machine |
| `tests/claude_code.rs` | The provider against a fake `claude`, a shell script that replays the recordings turn by turn and keeps what it read: the order probe, interrupt, message; a process announcing tools gets the probe only; not signed in is found before the content is sent; the safety options are passed; a timeout and a cancel kill the process group, grandchildren included; a missing binary |
| `tests/catalog.rs` | Every built-in prompt loads without problem or warning, has English and French labels, and builds a system prompt for each variant; the menus of the spec; replacing, adding and disabling; a file refused alone; each rule refused with its line; keys shared across a screen; the hash, unchanged by comments, labels and key order, changed by the text, inputs or model |
| `nm-ai` (unit, `diff.rs`) | The word diff: order removed-then-added, both texts rebuilt, spacing alone not a change, no glued words, huge texts replaced whole |
| `nm-core` (unit, `segments.rs`) | Replies, answers between quoted lines, signatures, forwards in English and French, Windows line ends; unchanged segments give the body back byte for byte, also on 3,000 random bodies made of every kind of line, none of whose segments holds a quoted line, a signature or a forwarded mail |
| `nm-ai` (unit, `message.rs`) | The segments sent and read back; the tags hidden from the streamed text however the answer is cut, from 1 byte to whole; a `<` of the text kept; text outside the segments not shown |
| `nm-cli` (unit, `ai.rs`) | The word diff as text, the export that keeps existing files and writes files that load as the built-in prompts, the status and the menu as text |
| `tests/answer.rs` | Each output, accepted and refused, case by case; the recorded answers of phase 0 read; the floor of the verdict |
| `nm-store`, `tests/ai_runs.rs` | A run from start to decision; ending and deciding once; failed and cancelled runs keep their raw answer; runs left running fail at start; the purge keeps everything but the texts and leaves running runs alone; suggestions follow their run and their conversation, and a batch is written whole or not at all |
| `nm-daemon`, `tests/ai_rpc.rs` | The real core: a conversation summarized (the mails in order, the quote of a reply left out, the user marked, the app's language), a mail translated (the latest received, the variant's language), a quick reply chosen and given back, a low verdict raised by a link that lies, a mail prompt without its conversation; a reply proofread from the menu to the saved draft, quotation untouched; nothing runs while the AI is off; what cannot work refused before any row is written; an answer with the wrong segments fails and is kept; a draft changed during the run is not overwritten; a subject chosen; a run cancelled and its `claude` process killed; a `claude` that announces tools never gets the mail |
| `nm-daemon`, `tests/ai_rpc.rs`, sorting | On a mailbox of the generator: the whole Quarantine sorted in one batch (the events in order, the suggestions by conversation, a move to the target suggested accepting it, one elsewhere not, one set aside); batches of two, the bad ones not sinking the good one, the progress to the end; a run whose every batch fails; a prompt outside its folder, nothing to sort |
| `nm-daemon` (unit, `ai.rs`) | The hosts of URLs and link texts ("Cliquez ici" names no site), the findings: authentication, a link that lies, a bare IP, the same organization under another name not flagged |
| Swift, `AITests.swift` | The keys of the panel in each phase (a mail's proposals only read, its quick replies chosen), a result as text, a quick reply put above the quotation, the hint, the diff as text, the cursor after a replacement, the proposals decoded, the errors said; through the keys against `FakeCore`: ⌘I and the menu (a prompt not available yet, the variants), a proofreading saved, run, streamed, accepted and undone, Escape stopping a run and refusing a proposal, a failure, a subject chosen, an end that comes before the run's id or after a gap, the AI off; `a` with a mail shown (the reading menu, a summary that takes the keys until closed, the mail under the cursor for a translation, a quick reply starting the reply), `a` on the list alone |
| `UITests/ai.uiscript` | The app with a real core answering from `fixtures/ai.ai.json`: the menu (not typed), a prompt not available yet, the differences, a letter not typed, applied with the quotation untouched and the cursor after the words, ⌘Z, a variant refused, a subject chosen; `a` with a mail shown: a summary (the keys not reaching the list meanwhile), a quick reply starting the reply apart from the words already typed; `a` on the list alone |
| `tests/real_claude.rs` | Ignored by default. The real binary, with the user's subscription: status, a prompt injection that must run nothing, a structured answer. Run it after an update of Claude Code: `cargo test -p nm-ai --test real_claude -- --ignored` |
