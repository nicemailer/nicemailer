# Upgrading a database: schema migrations and reinterpretation

A NiceMailer database lives for years and is upgraded by every new version of the core. Two independent mechanisms do it:

| | Schema migrations | Reinterpretation (`reindex`) |
| --- | --- | --- |
| What changes | tables, columns, indexes, and data that must be fixed at once | what the parser extracted from the raw mails: bodies, snippets, search index… |
| Version number | `PRAGMA user_version` of the database file, compared with `SCHEMA_VERSION` (`crates/nm-store/src/lib.rs`) | `message_body.parser_version` of each body, compared with `PARSER_VERSION` (`crates/nm-store/src/ingest.rs`) |
| Code | `crates/nm-store/src/migrations.rs` | `crates/nm-store/src/reindex.rs` |
| When | when the database is opened, before anything else | in the background, after the core has started |
| Blocks the app | yes, until it is done | no: the app keeps being answered |
| Unit of work | one version at a time, one transaction each | batches of mails, short transactions |

Rule of thumb: a migration does what the code cannot work without (a new column, an index the queries rely on); everything that can wait, and anything that needs the raw files, goes to the reinterpretation.

## Schema versions

`specifications/schema.sql` always creates the **latest** schema, and ends with `PRAGMA user_version = N`. A database created by an earlier version is brought up to it by the migrations, one version at a time.

| Version | Content |
| --- | --- |
| 1 | First schema. Edited in place during development, before any release (see "Incomplete version 1" below). Frozen in `crates/nm-store/tests/fixtures/schema_v1.sql`. |
| 2 | Indexes for large mailboxes: `idx_message_thread` becomes `(thread_id, folder_id, date_received, id)`; partial indexes for flagged, answered and attachment messages; `thread.date_last_modified` and the four thread indexes of Everything. Data: names and subjects with RFC 2047 encoded words glued inside a word decoded (`encoded_words.rs`), in `message`, `thread`, `address` and the search index; empty address names filled from the oldest mail the address sent with a name. About 30 s on 275,000 mails. |
| 3 | Remote images: `address.remote_images` (the sender's mails are shown with their remote images) and `message_body.tracking_pixels`. Two columns added at the end of their tables, instant. The bodies get their `nm-remote:` sources and pixel counts from the reinterpretation (`PARSER_VERSION` 8), in the background: bodies without remote image are only marked, the others (about two in three on a real mailbox) are read again from their raw file. |
| 4 | Accounts: `account.disabled_at`, the date the key of an IMAP account left `accounts.toml` (`config.toml` before the accounts had a file of their own) (`NULL` while it is there), and `account.position`, the display order of the accounts, set to the id of each existing account so the order stays the creation order. Two columns added at the end of a small table, instant. |

The reasons for the indexes of version 2, with measurements, are in `specifications/modele de donnees.md`, section 11.

## What happens when a database is opened

`Store::open` (`lib.rs`, `init`) runs these steps, in this order, every time:

1. **Connection settings.** `foreign_keys`, `synchronous = NORMAL`, `temp_store`, `busy_timeout`, `cache_size`: they are not stored in the file, so they are set on every connection.
2. **Read `PRAGMA user_version`.**
3. **Decide:**
   - `0`: an empty file. `schema.sql` is run as is, and the database is at the latest version.
   - Greater than `SCHEMA_VERSION`: the database was written by a newer core. It is refused with `SchemaTooNew` ("version du schéma N plus récente que celle du code"), and nothing is touched: an older core does not know what the newer columns mean, and writing to them could corrupt the data.
   - Otherwise, an earlier version (or the current one): `check_schema` checks it is complete (below), then `migrations::migrate` upgrades it. At the current version there is nothing to migrate.
4. **Planner statistics.** `PRAGMA optimize = 0x10002` analyzes the tables whose size changed much since the last analysis, with a bounded effort. The queries are written to be fast without statistics (the query plan tests check it); this is a safety net.

`migrate` loops from the database's version to `SCHEMA_VERSION`. Each step:

```text
BEGIN IMMEDIATE                     -- the write lock, at once
  to_N(tx)                          -- DDL and data of version N
  PRAGMA user_version = N
COMMIT
```

So each version is applied entirely or not at all. A crash, a full disk, or an error in the middle of a step rolls it back: the database stays at the previous version, consistent, and the next start migrates it again. Two steps are never mixed in one transaction, so a database migrated from version 1 to version 3 stops at 2 if step 3 fails.

### Who opens the database

- **The core** (`nicemailer-core`), at start. It holds the single-instance lock of the data directory (`lock` file), so no other core writes meanwhile.
- **The CLI**, for `nicemailer seed` only, which works on the database directly and asks for the core to be stopped. It migrates the same way.
- The app never opens the database: it talks to the core over its socket. The scripts (`scripts/import-mail-export.py`) only read it, read-only.

### Incomplete version 1: `OutdatedSchema`

Before the first release, version 1 was edited in place: development databases created early may lack tables or columns added since. Such a database cannot be migrated (the migrations assume a complete version 1), so `check_schema` probes it first: each check prepares a query on a table or column (`SELECT kind, address FROM account`, `SELECT one_click FROM message_unsubscribe`…), which fails without running anything if it is missing, and the database is refused with `OutdatedSchema` ("base créée par une version de développement antérieure (… absent) : recréez-la"). The old trigger `message_fts_ai`, from a time when a trigger filled the search index, is refused too: with it, every new mail would be indexed twice. From version 2 on, `check_schema` also requires what each version added (`thread.date_last_modified` for 2).

### Choosing a mailbox: `nicemailer mailbox check` and `upgrade`

The core migrates its own database at its start, without asking: after an update of NiceMailer, the mailbox in use simply goes on. Another folder chosen in the app ("Change Mailbox…") is different: the user may not want an old mailbox upgraded, since an older NiceMailer will then refuse it. So the app asks first ([app-bundle.md](app-bundle.md), "The folder is checked before the switch"), through two commands of the CLI that work on a folder without the core:

```sh
nicemailer mailbox check <folder>          # what its database is; changes nothing
nicemailer mailbox check <folder> --json   # what the app reads
nicemailer mailbox upgrade <folder>        # migrates it to this version
```

`check` (`nm_store::inspect`) applies the rules of `Store::open` without writing: the file is opened without the right to create it, the connection is `query_only`, and the answer is one of `new` (no file, an empty one, or version 0 without tables), `current`, `upgradable {from, to}`, `outdated {missing}` (what `check_schema` finds missing: an incomplete version 1, below), `too_new {found, expected}`, or `unreadable {message}`. The JSON adds `path` and `in_use` (a core holds the folder's lock).

`upgrade` refuses anything but `upgradable`, takes the folder's lock as a core would (it fails if a core uses the folder), and opens the database with `Store::open`, which migrates it one version per transaction, as at a core's start.

### Limits

- **A migration is one-way.** Once migrated, the database is refused by the previous core (`SchemaTooNew`). Before the first start of a new core on a mailbox that matters, copy the database; on APFS the copy is instant and takes no space until the files differ:

  ```sh
  cp -c "$HOME/Library/Application Support/NiceMailer/nicemailer.db" ~/nicemailer-backup.db
  ```

  (Stop the core first, or copy `nicemailer.db-wal` too: recent writes may still be there.)
- **The app waits during a migration.** `Store::open` runs before the core creates its socket, so the app cannot connect until the migration is over (about 30 s for version 2 on 275,000 mails), and the core prints nothing meanwhile. Keep migrations short: what can wait belongs to the reinterpretation.
- **One big transaction.** A step that rewrites many rows grows the WAL file (`nicemailer.db-wal`) by about as much; it is folded back into the database at the next checkpoints.

## Adding a schema version

Say version 3:

1. **`specifications/schema.sql`**: change the schema to the new state (tables, columns, indexes, with their French comments, `(version 3)` where it helps the reader), and set `PRAGMA user_version = 3`. A column added to an existing table goes **at the end** of it: `ALTER TABLE ADD COLUMN` can only add there, and the test below compares the columns in order.
2. **`lib.rs`**: `SCHEMA_VERSION = 3`, with a line in its comment saying what version 3 brings.
3. **`migrations.rs`**: a function `to_3(conn)` with the same changes as statements (`CREATE INDEX`, `ALTER TABLE … ADD COLUMN`, data updates), and the arm `2 => to_3(&tx)?` in `migrate`. Its comment says what it does and why.
   - SQLite cannot change or drop a column in place. That needs the table to be rebuilt (create the new table, copy, drop, rename, recreate indexes and triggers, with `foreign_keys` off), the procedure of the SQLite documentation, "Making Other Kinds Of Table Schema Changes".
   - Heavy data work (reading every body, every raw file) does not belong here: see the reinterpretation.
   - Never change `to_2` or an earlier step once released: databases in the wild went through it.
4. **`check_schema`**: add a check for what version 3 adds, under `if version >= 3`.
5. **Tests** (`crates/nm-store/tests/migrations.rs`):
   - `migrating_version_1_gives_the_latest_schema` must pass unchanged: it migrates a version 1 database (`fixtures/schema_v1.sql`, never edited) through every step, and compares it with a fresh database, table by table (columns from `PRAGMA table_xinfo`) and index by index (definitions, whitespace collapsed). It catches a step that forgets an index, or a `schema.sql` that drifts from the migrations.
   - Add a fixture of the last released version, taken from git (`git show <tag>:specifications/schema.sql > crates/nm-store/tests/fixtures/schema_v2.sql`), and a test that migrates it with data and checks the data of `to_3`, as `migration_repairs_names_and_subjects_and_fills_thread_dates` does for version 2.
6. **Documentation**: the table of versions above, and `specifications/modele de donnees.md` for the reasons of the change.
7. **Try it on a real mailbox** (below) and note how long it takes in the table of versions.

## Reinterpretation: parser versions and `reindex`

`message_body` is an interpretation of the raw message (`specifications/stockage-brut.md`, section 8), and `message_body.parser_version` says which parser produced it. The history of `PARSER_VERSION` is in its comment in `ingest.rs`; the versions that changed what is stored:

| Version | Change |
| --- | --- |

### When it runs

At start, the core launches a thread (`start_reindex`, `crates/nm-daemon/src/main.rs`) that counts the bodies older than `PARSER_VERSION`, plus the messages that have no body (their first parse failed). If there are none, it stops at once. The count and the work list are read from the index `idx_message_body_parser`, never from the bodies themselves: a join with `message_body` reads the whole text of every mail, which took 30 s on a large mailbox.

The core's log shows it:

```text
reindex : 274939 mails analysés par une version antérieure, en tâche de fond
reindex : 3800 / 31599
reindex terminé : 31599 mails réanalysés, 350 corrigés sans relecture, 242990 inchangés, 0 bruts introuvables, 0 illisibles, 201 noms d'adresses complétés, en 116.8 s
```

### What it does

`reindex::reindex` works in three passes.

1. **Bodies that cannot change are only marked.** A parser version can say which bodies it may change; the others are set to the new version without being read again (`skip_unchanged`). Version 6 only changes a text holding `&…;` or HTML markup: the other version 5 bodies are marked as version 6 by one `UPDATE` with `NOT LIKE` conditions, wider than the parser's own test, so that no body that may change is skipped. About nine bodies in ten.
2. **Changes that can be computed from what is stored need no raw file** (`repair_stored`). Version 7 decodes the stored text and encodes the stored HTML once more; only the bodies that actually change are written, with their snippet and their row of the search index.
3. **The rest is parsed again from the raw files.** Reading, decompressing and parsing run in parallel on every processor core, without the store lock. The writes rewrite the body (created if it was missing), the snippet, the body column of the search index and `message_unsubscribe`, and give a name to the addresses that have none (as at ingestion: a name already set is never replaced). A body that comes out the same is not rewritten.

Not rewritten: the interpreted headers (threads, recipients, references), the attachments (their ids and extracted files are in use), the authentication verdict, folders and flags. The raw files are never modified. A missing or corrupt raw file is counted (`bruts introuvables`) and the body left as it was; the next start tries again.

### Keeping the app answered

The reinterpretation shares the database with the app's requests through the store lock. The constants of `reindex.rs`, and the measurements behind them on 275,000 mails:

| Constant | Value | Why |
| --- | --- | --- |
| `RANGE` | 500 message ids | The first two passes read the bodies by ranges of ids, the lock released between two ranges. In one statement, the app waited 49 s for its first list. |
| pause | 2 ms after each range or write | A Rust `Mutex` is not fair: a thread that releases it and asks again at once usually gets it back. Without the pause, the app's requests waited for the end of the loop (37 s). |
| `BATCH_SIZE` | 50 mails | Parsed together, written together. With 200, a list waited up to 0.4 s. |
| `WRITE_BYTES` | 2 MB of text | A transaction is committed when this much text has been written. Rewriting a 10 MB body and its search index row takes about 1 s; fifty of them in one transaction held the lock 40 s. |

Result: during a full reinterpretation (2 minutes), the app's lists answered in 96 ms on average, 2.6 s at worst (a 10 MB mail being written). No event is sent: a body only shows when its thread is opened, and an event per batch would make the app reload its list hundreds of times. A thread already open shows its new body when opened again.

### Raising the parser version

1. Change the parser (`ingest.rs`, `html_text.rs`…), raise `PARSER_VERSION`, and add a line to its comment.
2. Decide which stored bodies the change can affect:
   - **If you can tell from the stored text or HTML which bodies may change**, add a rule like `skip_unchanged` (a wide SQL condition: when in doubt, the body is read again).
   - **If the change can be computed from what is stored**, a function like `repair_stored` avoids reading the raw files.
   - **Otherwise every older body is parsed again**: the default, always correct, just slower (an estimate from the 31,600 mails reparsed in about 100 s: some 15 minutes for 275,000 mails, in the background).

   `repair_stored` asserts (in debug builds, so in the tests) that `PARSER_VERSION` is 7: raising the version makes the tests fail there until the passes are reviewed for the new version.
3. Tests in `crates/nm-store/tests/reindex.rs`: put rows back as the previous version left them (an `UPDATE message_body SET parser_version = …` plus the old text), run `reindex`, check what changed and what did not. Existing tests cover a missing body, a missing raw file, a name set by the user, an unchanged body not rewritten, a repair without raw files.
4. Try it on a real mailbox (below), watching the count of each pass and the app's latency.
5. Update this page and `docs/core-library.md` if what users see changes.

## Trying an upgrade on a real mailbox

Never on the mailbox itself first. On APFS, a copy of the database is instant (`cp -c` clones the file):

```sh
D="$HOME/Library/Application Support/NiceMailer-archive"   # the data directory
T=/tmp/nmtry; mkdir -p $T
cp -c "$D/nicemailer.db" $T/
[ -s "$D/nicemailer.db-wal" ] && cp -c "$D/nicemailer.db-wal" $T/
ln -s "$D/raw.noindex" $T/raw.noindex     # the raw files, read only
target/release/nicemailer-core --data-dir $T
```

- Keep the copy's path short: the core's socket lives in the data directory, and a Unix socket path is limited to about 100 characters.
- The raw files are shared through the link: the reinterpretation only reads them.
- While it runs, time the app's lists from another terminal (`nicemailer --data-dir $T call list_threads '{"everything": {}, "limit": 60}'`), and check the result with `sqlite3 $T/nicemailer.db`.
- `make bench` with `BENCH_DATA_DIR=$T` times every list afterwards (`docs/testing.md`).

Delete the copy afterwards (`rm -rf $T`).
