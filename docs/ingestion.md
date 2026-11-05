# How a message enters the database

Every message goes through the same path, whether it comes from an mbox file ([import-mbox.md](import-mbox.md)) or from an IMAP server (the synchronization, phase 4 of `specifications/plan-imap.md`). Two modules make that path:

- `crates/nm-store/src/raw.rs`: the raw store, which keeps every message byte for byte, compressed, under its SHA-256 (`specifications/stockage-brut.md`);
- `crates/nm-store/src/insert.rs`: the rows of the database, for a message and its thread.

## Two steps

IMAP gives the headers of a new mail long before its body: a new account may have 200,000 bodies to download. A mail must appear in the list, be threaded and screened as soon as its headers are known, so the insertion is in two steps.

| Step | Call | What it writes |
| --- | --- | --- |
| Headers | `Inserter::insert(tx, message, None, changes)`, headers parsed by `ingest::parse_headers` | the `message` row (`body_state = 0`, no `raw_hash`, no snippet), addresses, references, thread, the search entry (subject, sender, recipients), folder and thread counters |
| Body | `Inserter::complete(tx, id, body, changes)`, once the message is in the raw store and parsed by `ingest::parse` | `raw_blob`, `message_body`, attachments, how to unsubscribe, snippet, `has_attachments`, the body in the search entry, `body_state = 1` |

The mbox import has the whole message at once and calls `insert` with the body: both steps in one, in one `INSERT` of the `message` row.

`complete` returns what it did: `Done`; `Already`, when the body was downloaded twice; `Missing`, when the message was deleted while its body was downloading. In the last two cases nothing changes. The raw file of a `Missing` message stays on disk, harmless, until the garbage collector removes it (phase 8).

`parse_headers` gives exactly the header fields `parse` gives for the whole message: same code, same bytes. The fields that come from the body stay empty, including `unsubscribe`, which `complete` reads with the body, since a link in the body may be the only one.

### Events

A new message adds its folder to the `ChangeSet` (counters changed), not the message itself: an import of 50,000 mails sends one event per folder and batch rather than 50,000 `MessageChanged`. The synchronization will announce a new mail with its own event (`MessageArrived`, phase 4). `complete` does add the message (`MessageChanged`): a client showing it wants its body now.

## Transactions

The caller opens the transaction (`Store::transaction`, which takes the write lock at once), inserts any number of messages, then hands it to `Inserter::commit`. The commit recomputes what the batch changed once for the whole batch (the subject and the per-folder rows of each thread touched, the counters of each folder), then commits. A batch is one transaction: a reader never sees half of it.

The `Inserter` keeps the ids of the addresses it has seen: most mails involve the same few hundred people, so it saves most address lookups. The ids created in a transaction are only kept once it is committed. If the transaction is rolled back (dropped without `commit`), the caller calls `Inserter::abandon`, and the next batch creates the addresses again instead of using the ids of rows that no longer exist.

## Threads

A message joins the thread of every message of the same account that:

- it references (`In-Reply-To`, `References`);
- references it;
- references one of the messages it references;
- has the same `Message-ID` (another copy).

When a message links several threads, they are merged into the oldest one. The third rule is the "phantom parent" of the JWZ algorithm: two replies to a message the database does not have (deleted long ago, or in a folder NiceMailer does not synchronize) are one conversation, which they say themselves by citing the same message. Without it they would stay apart until the parent arrives, if ever.

Every link is between two messages present, whichever came first: the threads depend on which messages are there, never on the order in which they arrived. A test inserts the same five messages in each of their 120 orders and checks the threads are the same.

Messages without any reference are not grouped by subject, which would merge unrelated mails such as monthly invoices.

## Streaming writes

A message received from an IMAP server comes in pieces of 1 MB above 4 MB (`nm-proto`, [imap.md](imap.md)), and may weigh 150 MB. `RawStore::writer` returns a `RawWriter`, which takes the pieces one by one:

1. each piece is hashed (SHA-256) and compressed (zstd, level 9) as it arrives, and the compressed bytes go to a temporary file in `raw.noindex/.tmp/`;
2. `finish(Some(size))` checks the number of bytes against the size the server announced (`RFC822.SIZE`), and refuses a message that is shorter or longer (`finish(None)` checks nothing). The specification asks for this check (`stockage-brut.md`, section 5, step 2), but some servers count `RFC822.SIZE` differently from the bytes they send: refusing such a message would leave it without body for good. What the synchronization passes, and what it does on a difference, is decided in phase 4;
3. the file gets a plain `fsync`, then its final name, the hash (`rename`, atomic), and its directory an `fsync`. If a file with that hash already exists, the new copy is deleted instead (the same bytes in two folders are stored once).

A writer dropped without `finish` (a network cut, a full disk, the core stopping) removes its temporary files: a message partially received is never stored, and nothing is left behind. The `raw_blob` and `message` rows are written by `complete` only after `finish` succeeded, in the transaction of the batch, after `RawStore::barrier` (one `F_FULLFSYNC` per batch, see [import-mbox.md](import-mbox.md), "Performance"): a row never exists without its file.

`RawStore::write(bytes)`, for a message already in memory (the mbox import), is the same writer fed in one piece. It hashes the bytes first, so a message already stored costs no compression at all.

### The plain copy, and memory

The message must still be parsed, and the parser wants all its bytes at once. `writer(true)` therefore also keeps a plain copy, returned by `finish` as `PlainBytes`:

- up to 16 MB (`PLAIN_IN_MEMORY`), in memory;
- above, in a second temporary file, *mapped* into memory at the end (`mmap`) and deleted at once. A mapped file is not heap memory: macOS reads its pages from the file on demand and drops them when memory runs short. Deleting the name right away is safe, since the mapping keeps the file alive, and nothing can be left behind even after a crash.

The parser, `mail-parser` 1.0, decodes nothing until asked: it only scans the bytes to find the parts, and measures attachments without decoding their base64. Parsing the mapped copy of a 150 MB message is therefore cheap too.

Measured by `crates/nm-store/tests/big_message.rs`, which counts every allocation of its process: a message of 150 MB (a 110 MB attachment), written in pieces of 1 MB, parsed and inserted, raises the heap by 26 MB at most. Most of it is the plain copy kept in memory until it passes 16 MB; the rest is zstd's buffers and the parser's tables, whatever the size of the message.

A text body itself is not limited: a 150 MB `text/plain` mail would have 150 MB of text stored in the database and its search index. Such mails are not expected; a limit would need a new parser version (`migrations.md`).

## Failures

| Failure | Result |
| --- | --- |
| Network cut in the middle of a body | The session closes (`nm-proto`), the writer is dropped: no file, the message stays `body_state = 0`, and the next attempt downloads it again |
| Size other than announced | `finish` fails with `InvalidData` ("message of N bytes, M announced"): nothing stored |
| Full disk | The write fails with `StorageFull`: no file left, no row |
| Error in the middle of a batch | The transaction is rolled back: none of the batch's rows remain; the raw files already written stay, without rows, until the orphan sweep removes them (phase 8) |
| Crash between the rename and the commit | Same: a file without a row, harmless (`stockage-brut.md`, section 5) |
| The MIME parser cannot read the message | The raw file is kept and linked, without `message_body`; `reindex` creates the body once a newer parser can read it |

## Tests

| Test | What it checks |
| --- | --- |
| `crates/nm-store/src/raw.rs` (unit) | pieces give the same file as one write; a size other than announced, a writer dropped unfinished, a full disk: nothing stored, nothing left in `.tmp/`; above 16 MB the plain copy is a mapped file |
| `crates/nm-store/tests/insert.rs` | headers then body (list, counters, search before and after the body, `Already`, `Missing`); threads in each of 120 orders of arrival; two replies to an absent message; a merge; a batch rolled back leaves nothing and its addresses are forgotten; a full disk while the body arrives changes nothing |
| `crates/nm-store/tests/big_message.rs` | 150 MB in pieces, bounded heap, stored byte for byte |
| `crates/nm-store/tests/import.rs` | the mbox import, through the same path |
| `crates/nm-engine/tests/ingest_imap.rs` (`make imap-test`) | against Dovecot: the headers of a 22 MB mail, then its body streamed from the server into the raw store; a cut at 8 MB stores nothing, and the next connection completes the message |

The simulated full disk is `RawStore::with_disk_space(bytes)`, for tests only: every write beyond fails with the error of a full disk.

In debug builds, which the tests use, `sha2`, `mail-parser` and `zstd-sys` are compiled optimized (`[profile.dev.package.*]` in `Cargo.toml`): the 150 MB test took 24 s without it, under a second with it. NiceMailer's own code stays unoptimized in debug, so it still compiles fast.
