# Database files and backups

The data directory of a mailbox (`~/Library/Application Support/NiceMailer` by default) holds two SQLite databases since schema version 10 (`specifications/modele de donnees.md`, "Deux fichiers"):

| File | Content | On a mailbox of 400,000 mails |
| --- | --- | --- |
| `nicemailer.db` | Everything that is not derived: accounts, folders, messages (headers, flags, folder), conversations, addresses and decisions, tags, groups, drafts, spool, history | 516 MB |
| `nicemailer_body.db` | The bodies of the mails (`message_body`: text and sanitized HTML) and the full-text search index (`message_fts`) | 11.7 GB |
| `raw.noindex/` | The raw messages, immutable files named by their SHA-256 (`specifications/stockage-brut.md`) | 23 GB |

Everything in `nicemailer_body.db` is an interpretation of the raw files: it can always be made again from `raw.noindex/` and `nicemailer.db`. So a backup needs only the other two.

## What to back up

- **`raw.noindex/`**: with any file-level copy, for instance `rsync -a`. Its files never change once written, and a new mail adds a file, so every copy after the first one only sends the new files.
- **`nicemailer.db`**: with `nicemailer mailbox backup`, which writes a consistent, compact copy even while the core runs:

  ```sh
  nicemailer mailbox backup ~/Library/Application\ Support/NiceMailer /Volumes/Backup/nicemailer-2026-11-12.db
  ```

  It uses SQLite's `VACUUM INTO`, a copy made from one read transaction: the core goes on writing meanwhile, and the copy includes the writes still in `nicemailer.db-wal`, which copying the file itself would miss. 2 s for 516 MB. The destination must not exist.
- **`nicemailer_body.db`** need not be backed up. It can be left out of Time Machine:

  ```sh
  tmutil addexclusion ~/Library/Application\ Support/NiceMailer/nicemailer_body.db
  ```

The settings (`config.toml`, `accounts.toml`) and the passwords (Keychain) are elsewhere (`docs/configuration.md`).

## Restoring, and rebuilding the body file

Put `nicemailer.db` and `raw.noindex/` back in the data directory, without any `nicemailer_body.db`, then start the core. At opening it:

1. creates an empty `nicemailer_body.db`;
2. gives every mail its row of the search index from what `nicemailer.db` knows (subject, sender, recipients): searching by these works at once;
3. starts the reinterpretation in the background (`docs/migrations.md`, "Reinterpretation"), which reads every raw file again and writes its body and the body of its search row. The core logs its progress (`reindex : 12000 / 400000`); the app works meanwhile, and a mail opened before its turn shows its headers until its body is back.

The same procedure rebuilds a damaged body file: stop the core, delete `nicemailer_body.db` (and its `-wal` and `-shm` files), start the core.

## How the two files are kept in agreement

The core attaches the body file to each of its connections (`ATTACH DATABASE … AS body`): queries name `message_body` and `message_fts` as before, and SQLite finds them in the second file. Two things do not cross files in SQLite, and the core makes up for them (`crates/nm-store/src/lib.rs`):

- **Triggers and foreign keys.** A message deleted must take its body and its search row along, and a subject changed must reach the search index. Stored triggers and `ON DELETE CASCADE` only work within one file, so these triggers are created `TEMP` on each connection that writes (`TEMP_TRIGGERS`).
- **Atomic commits.** In WAL mode, a transaction writing to both files commits each file on its own: a crash in between can leave a body without its message, or a message without its search row. At the opening, `reconcile_bodies` deletes the rows of the body file whose message is gone and makes the missing search rows again (0.23 s on the mailbox above with the files in memory, 0.9 s with the body file cold, most of the core's start: it reads indexes and the small table of sizes of the search index, never the text). A missing body is the reinterpretation's to rebuild. It is skipped after a clean close, which left the files in step (`clean_close.rs`): a core stopped by SIGTERM (launchd, `nicemailer core stop`, `make core-stop`) empties both write-ahead logs into their files (waiting at most 200 ms for a reader still in a read: else no mark, rather than a stop slow enough for launchd to kill it after 20 s; `core.log` says how long each checkpoint took) and writes `clean-close.json`, the inode, size and time of change of each file. The next opening skips the reconciliation if both files still match it exactly and neither log holds anything. Anything else reconciles: no mark (a crash, a kill, the CLI), a file written, deleted or replaced since (a backup restored), a log not empty. The mark is removed at every opening, before anything is written, so a crash later never leaves one that matches.

A new `nicemailer.db` (deleted, or `nicemailer seed --reset`, which deletes both files) empties a `nicemailer_body.db` it finds: message ids start again at 1, and the old bodies would answer for new mails.

## Tests

`crates/nm-store/tests/bodies.rs`: the tables in the second file and found by the queries and the readers, a message deleted taking its body along, a body file deleted then rebuilt from the raw files, rows left by a crash cleaned at opening, a body file left by a deleted database emptied. `crates/nm-store/tests/migrations.rs`: a database migrated to version 10 searches through its copied index, which passes FTS5's `integrity-check`.
