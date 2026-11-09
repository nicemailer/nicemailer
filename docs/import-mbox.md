# Importing an mbox file

NiceMailer can import a whole mailbox exported as an mbox file (HEY, Gmail Takeout, Thunderbird, Apple Mail…) into an **archive account**. For now the import is only available from the command line. An archive account has no server: `r`, `R` and `f` on its mails answer from an account of `accounts.toml` that can send ([sending.md](sending.md), "Choosing the sender").

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

- **Archive account.** The account has no IMAP or SMTP server, now or later, and does not appear in `accounts.toml`. It gets every workflow folder, so imported mail can be filed like any other (Todo, Paperwork, Deleted…). Moves only change the local database.
- **Filing.** A message whose `From` is the account address (case-insensitive) goes to Sent; every other message goes to Archive. All are marked as read. Nothing goes through screening or to Quarantine.
- **Dates.** The reception date is the date of the mbox `From ` separator line (when the exporting server received the message), or the `Date` header if that line has no date. Some export tools write the time of the export in that line instead (Python's `mailbox` module does when it is not given a date), which would make every message look received on the export day. So the `From ` line date is ignored, and the `Date` header used, when it cannot be a delivery date: it is after the start of the import, or more than 7 days after the `Date` header (SMTP servers stop retrying after about 5 days). A message with neither date gets 1970 and sorts last.
- **Answered.** A received message that one of your sent messages replies to is marked as answered.
- **Conversations.** Messages are grouped using `Message-ID`, `In-Reply-To` and `References`, whatever their order in the file; two replies to a message missing from the file are grouped too, since they cite the same message. Messages without references are not grouped by subject. The rules are those of every message, IMAP included: [ingestion.md](ingestion.md), "Threads".
- **Raw messages.** Each message is kept byte for byte (after removing the mbox `>From ` escaping), compressed, under `raw.noindex/` in the data directory, and named after its SHA-256. Text, sanitized HTML (scripts removed, tracking pixels removed, other remote images kept in a form only the app loads, on request) and attachment metadata are stored in the database. A message without a text part gets its text from its HTML, converted by the core (`html_text.rs`): headings, paragraphs, list items and table rows stay on separate lines, entities are decoded, scripts and styles dropped, and so are the elements the mail hides; its hidden preview text becomes the snippet (`docs/core-library.md`).
- **Search.** Subject, sender, recipients and text body are indexed.
- **Addresses.** Addresses created by the import are marked `imported_from = 'mbox'`. The display name comes from the first message that gives one: an address known without a name (a bare recipient, a decision taken before any mail) gets the name of the first mail that carries it, and a name already set is never replaced. Names and subjects with RFC 2047 encoded words glued inside a word, as old mail clients wrote them (`Achille Ber=?ISO-8859-1?B?6A==?=s`), are decoded (`encoded_words.rs`).
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

## Importing all of Apple Mail

Two scripts move a whole Apple Mail library into NiceMailer, including IMAP accounts whose server no longer exists (Apple Mail's own export fails on them).

1. `scripts/export-apple-mail.py` reads `~/Library/Mail/V10` directly (never writing to it, never contacting a server) and writes `mail-export/` at the project root:
   - one mbox per IMAP account, all its folders merged, named `<Account> (<address>).mbox`; each message gets an `X-Mail-Folder:` header per Apple Mail folder it was in. Copies of a message with the same `Message-ID`, `Date` and `Subject` are written once (Gmail's "All Mail" holds a copy of everything; `Message-ID` alone is not unique in practice);
   - accounts deleted from Apple Mail whose files are still on disk: `Deleted account <id> (<address>).mbox`, the address being the most frequent recipient;
   - one mbox per local folder ("On My Mac"), the tree reproduced as directories under `mail-export/Sur mon Mac/`;
   - attachments Apple Mail stored outside the message (`.partial.emlx`) are put back; attachments it never downloaded are replaced by a text note with the header `X-Missing-Attachment: true`.
   - the `From ` separator line of each message carries the date the server received it, as recorded by Apple Mail (`date-received` in the `.emlx` file), else its `Date` header, else the time Apple Mail wrote the file. Exports made before this fix (November 2026) carry the export time there instead; the import falls back on the `Date` header for them (see Dates above), but re-exporting gives the exact reception dates.
2. `scripts/import-mail-export.py` imports every mbox of `mail-export/` with `nicemailer import-mbox`. First it plans, asking only what it cannot tell:
   - IMAP account files: the address comes from the file name and the account name from the Apple Mail account (`Achille Sh` → `achille-sh`). It asks when the name is taken by another NiceMailer account (for example an IMAP account `perso`) and for deleted accounts;
   - `Messages récupérés (<Account>).mbox` goes with `<Account>`;
   - a file name that contains an address (`HEY-emails-me@hey.com.mbox`): the address is the suffix of the name found most often in the headers; it asks to confirm it and the name;
   - other local folders mix several of your addresses: it counts which of them sent the messages and asks once per top-level local folder (`Sur mon Mac/Pro`…), or file by file if you type `f`. Type `-` to skip.

   The answers are saved to `mail-export/import-plan.json` and reused next time (`--replan` asks again). After a recap and a single confirmation, the files are imported one after the other without further questions (the core runs one import at a time); a failed file does not stop the others. The CLI output goes to `mail-export/import.log`. Running the script again resumes an interrupted run, since messages already imported are skipped.

```sh
make core                                   # in one terminal
scripts/export-apple-mail.py                # ~5 min for 230,000 messages
scripts/import-mail-export.py --dry-run     # plan only
scripts/import-mail-export.py               # plan, confirm, import
```

The export script is tested by `scripts/tests/test_export_apple_mail.py` (part of `make test`).

Options of the import script: `--export-dir`, `--data-dir` (as for the CLI), `--cli PATH` (default: builds `target/release/nicemailer`), `--yes` (no final confirmation).

Because each folder of an account is not kept as a NiceMailer folder (the import files by sender, see above), a message exported from two Apple Mail stores with different bytes (for example once in an IMAP account and once in a local copy) is imported twice; both copies end up in the same conversation.

## Limitations

- No import from the app yet.
- mbox flags (`Status:`, `X-Status:`, `X-Gmail-Labels`) are ignored: everything is read and filed by sender.
- One address per archive account (no aliases).
