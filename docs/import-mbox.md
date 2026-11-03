# Importing an mbox file

NiceMailer can import a whole mailbox exported as an mbox file (HEY, Gmail Takeout, Thunderbird, Apple Mail…) into an **archive account**. For now the import is only available from the command line.

The specification (in French) is `specifications/import-initial.md`, section 9.

## Usage

The core does the import, so it must be running:

```sh
make core                       # in one terminal
nicemailer import-mbox ~/Downloads/HEY-emails-me@hey.com.mbox \
    --account hey --address me@hey.com
```

| Option | Meaning |
| --- | --- |
| `FILE` | The mbox file. Relative paths are resolved by the CLI before being sent to the core. |
| `--account NAME` | Name of the archive account: letters, digits, `-`, `_`, `.`; at most 64 characters. Created if it does not exist. |
| `--address ADDR` | Address of the exported mailbox. Messages sent from it go to Sent. |
| `--no-correspondents` | Do not give the `inbox` screening decision to the people you wrote to (see below). |

The command prints the progress, then a summary:

```
Importing /Users/me/HEY-emails-me@hey.com.mbox into the archive account "hey" (Ctrl-C stops following, not the import)
 83.0 %  8.05 / 9.70 GB  39500 imported  0 already present  270 MB/s
Import 1 (finished): /Users/me/HEY-emails-me@hey.com.mbox
  46539 imported (4587 to Sent, 41952 to Archive), 0 already present, 0 not parsed
  772 correspondents now go straight to the Inbox; 35.9 s
```

Pressing Ctrl-C only stops the CLI from following the import: the import itself goes on in the core. `nicemailer import-status` shows where it is.

## What the import does

- **Archive account.** The account has no IMAP or SMTP server, now or later, and does not appear in `config.toml`. It gets every workflow folder, so imported mail can be filed like any other (Todo, Paperwork, Deleted…). Moves only change the local database.
- **Filing.** A message whose `From` is the account address (case-insensitive) goes to Sent; every other message goes to Archive. All are marked as read. Nothing goes through screening or to Quarantine.
- **Dates.** The reception date is the date of the mbox `From ` separator line (when the exporting server received the message), or the `Date` header if that line has no date.
- **Answered.** A received message that one of your sent messages replies to is marked as answered.
- **Conversations.** Messages are grouped using `Message-ID`, `In-Reply-To` and `References`, whatever their order in the file. Messages without references are not grouped by subject.
- **Raw messages.** Each message is kept byte for byte (after removing the mbox `>From ` escaping), compressed, under `raw.noindex/` in the data directory, and named after its SHA-256. Text, sanitized HTML (scripts and remote images removed) and attachment metadata are stored in the database. A message without a text part gets its text from its HTML, converted by the core (`html_text.rs`): headings, paragraphs, list items and table rows stay on separate lines, entities are decoded, scripts and styles dropped.
- **Search.** Subject, sender, recipients and text body are indexed.
- **Addresses.** Addresses created by the import are marked `imported_from = 'mbox'`. The display name comes from the first message of the file that mentions the address.
- **Correspondents.** The recipients of your sent messages are people you wrote to. Those without a screening decision get the decision `inbox`: their next mail, on any account, goes straight to the Inbox. Your own address is excluded, and an existing decision (or a decision you removed) is never changed. Disable with `--no-correspondents`.
- **Authentication.** Imported messages have the verdict `unknown`: an export does not tell which `Authentication-Results` headers can be trusted.

## Running it again

Importing the same file again adds nothing: a message whose exact bytes are already in the account is skipped and counted as "already present". This also means an interrupted import (core stopped, machine asleep) is resumed by running the same command again.

Two copies of a message that differ by a single byte (for example the same mail exported twice with different headers) are two messages; they end up in the same conversation.

Reusing an account name is allowed only for an archive account with the same address. A name already used by an IMAP account, or an archive account with another address, is refused.

## Performance

Measured on a 10-core MacBook with a 9.7 GB HEY export (46,539 messages):

| Step | Time |
| --- | --- |
| Full import | 36 s (about 270 MB/s) |
| Same import again (everything skipped) | 6 s |
| Raw storage on disk | 5.7 GB |
| Database | 0.9 GB |

How it is fast:

- the file is read as a stream; memory use depends on the largest message, not on the file size;
- hashing, compression and MIME parsing run on every core;
- a single writer inserts messages in transactions of 500, so the app keeps being answered between batches;
- on macOS, raw files get a plain `fsync` each and one `F_FULLFSYNC` per batch (a full flush per file capped the import at 22 MB/s);
- the search index row of each message is written once, by the code, instead of by triggers (twice as fast).

## Protocol

For clients other than the CLI:

- `import_mbox {path, account, address, mark_correspondents?}` starts the import and returns its initial status (`job_id`, `running: true`…). `path` must be absolute and readable by the core. Errors: `-32602` (invalid parameters, account refused), `1004` (`BUSY`: an import is already running).
- `import_status` returns the status of the running import, or of the last one, or `null`.
- Events: `ImportProgress` after each batch and `ImportFinished` at the end, both carrying the status fields (`job_id`, `bytes_read`, `total_bytes`, `imported`, `skipped`, `parse_errors`, and at the end `sent`, `archived`, `correspondents`, `seconds`, `error`). Each batch also emits `FolderChanged` for the folders it fills.

## Limitations

- No import from the app yet.
- mbox flags (`Status:`, `X-Status:`, `X-Gmail-Labels`) are ignored: everything is read and filed by sender.
- One address per archive account (no aliases).
- Filing imported mail when you later take a screening decision on a sender is specified but not implemented yet: screening decisions themselves are not implemented.
- Attachments are recorded but cannot be opened yet (extraction from the raw file is not implemented).
