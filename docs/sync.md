# Synchronization of the IMAP accounts

The core keeps the database in line with the server of every IMAP account of `config.toml`: new mail, flags, deletions, moves made on the phone or the webmail. This is phase 4 of `specifications/plan-imap.md`: **server to database only**. Nothing is sent to the server yet; the spool, which will send moves, flags and tags made in NiceMailer, is phase 5.

The code is in `crates/nm-engine/src/sync/` (the connections and the passes) and `crates/nm-store/src/sync.rs` (what is written in the database, testable without a server).

## Read-only towards the server

Until phase 5, the synchronization cannot change anything on the server, even by mistake: every folder is opened with `EXAMINE` (the read-only `SELECT`), and every fetch is a `PEEK`, which never marks a mail read.

The commands that change mail are refused on the accounts the core synchronizes, with the error `1010` (`READ_ONLY`): moving a thread, marking it read or flagged, tags. A change made locally would be undone by the next synchronization. A decision on a sender (`set_address_screening`, `set_domain_screening`, or a move from an archive account) is still recorded, but it files no mail of these accounts. The fake accounts of `make seed` and the archive accounts of mbox imports are not concerned: the core gives the store the list of the accounts of `config.toml` (`Store::set_server_accounts`).

## Connections

Each account has two connections, each on a thread of its own:

| Connection | Job |
| --- | --- |
| Work | The passes over the folders; between two passes, IDLE on the Inbox |
| Bodies | Downloads the bodies, in the background ([ingestion.md](ingestion.md)) |

`sync.max_connections = 1` (in `[sync]`, or `[accounts.<key>.sync]`) puts both jobs on the work connection: bodies are then downloaded 30 s at a time between two passes. Use it for a server that allows few connections per user.

The plan had a third connection for IDLE alone. The work connection leaves IDLE within milliseconds when it has something to do, so a third one would only use one more of the few connections servers allow (Dovecot: 10 per user and address by default).

## A pass

1. **The folders.** `LIST`, with `LIST-STATUS` when the server has it, else one `STATUS` per folder to synchronize. The `folder` rows follow the server:
   - a new folder gets a row, **without role**: NiceMailer manages only the folders of its workflow (`dossiers.md`);
   - except the first listing of an account, which has no role at all: each folder then gets the role NiceMailer proposes for it (INBOX, the special-use attribute, then the usual names; the rules of `nicemailer imap probe`). This is provisional: the account creation panel (phase 7) will ask the user to confirm. Recorded in `audit_log` (`role`, reason `proposed`);
   - a folder with a role that the server no longer lists, while a new folder has its `UIDVALIDITY`, was renamed on the server: the row follows, with its mail and its role;
   - a folder with a role that is no longer listed, and not renamed, is kept with its mail, and not synchronized (`sync status` says "no longer on the server"). A folder deleted by mistake, or a server that lists badly for a moment, must not delete thousands of mails;
   - a folder without role no longer listed is deleted (it holds no mail).
2. **Which folders changed.** `UIDVALIDITY`, `UIDNEXT`, the message count and `HIGHESTMODSEQ` of `STATUS`, against the database: a quiet folder is not even opened. The folder the session has selected (the Inbox, for IDLE) is always opened: Dovecot answers the `STATUS` of the selected mailbox with what the session already knows, not with its current state (measured: the flags changed by the phone were missed).
3. **Discovery** of each folder that changed, with what the server offers:

   | Server | Expunged mail | Flags changed |
   | --- | --- | --- |
   | QRESYNC | `VANISHED` of `EXAMINE … (QRESYNC (uidvalidity modseq))` | the `FETCH` of the same `EXAMINE` |
   | CONDSTORE | `UID SEARCH ALL` compared with the database | `UID FETCH 1:* (FLAGS) (CHANGEDSINCE m)` |
   | neither | `UID SEARCH ALL` compared with the database | `UID FETCH 1:* (FLAGS)` compared with the database: the Inbox at every pass, the other folders at each `poll_interval_secs` |

   New UIDs: `UID SEARCH UID <uidnext>:*` with QRESYNC; the UIDs of the server not in the database otherwise (which also repairs a gap).
4. **Arrivals.** The headers, flags, date and size of the new UIDs (`BODY.PEEK[HEADER]`), 100 per command and per transaction ([ingestion.md](ingestion.md), first step). `uidnext` advances with each batch.
5. **End of each folder.** Flags, deletions and the new `UIDVALIDITY`, `UIDNEXT`, `HIGHESTMODSEQ`, in one transaction.

`UIDVALIDITY` changed: the UIDs of the folder name other messages, so its messages are dropped (`audit_log`, `purge`) and the folder is fetched again from the start (`modele de donnees.md`, section 5).

### Crash safety

`uidnext` advances in the transaction of each batch of arrivals: after a crash, the next pass fetches from the first UID not written, never twice (`UNIQUE (folder_id, uid)` refuses a duplicate anyway). `HIGHESTMODSEQ` only advances at the end of the folder, with the flags and deletions: after a crash before it, the server reports the same changes again, and applying them twice changes nothing.

### Moves made elsewhere

A mail moved by the phone from Inbox to Archive vanishes from the Inbox and appears in Archive. Deleted then inserted again, it would lose its id, tags, history and consultation. The pass therefore finds every deletion of every folder (step 3) before writing any arrival (step 4): a new mail is the move of an existing row when

- it has a `Message-ID`, and exactly one row of the account has it, in another folder;
- of the same size (RFC822.SIZE, or the bytes stored once the body came);
- whose UID vanished in this pass.

The row then moves (`folder_id`, `uid`, flags; `audit_log`, `move`, reason `elsewhere`) and keeps everything else. Anything else is a new row: a copy (the original stayed), or a move that cannot be told from one.

After IDLE reports a change, the pass waits 1.5 s. A client that moves a mail without `MOVE` copies it, marks it `\Deleted`, then expunges it, in three commands; a pass started at the first would see a new mail and the original still there. The delay also gathers a burst of new mail into one pass.

### New mail

`MessageArrived` (`message_id`, `folder_id`) for each new row, after the folder's `FolderChanged`. Not during the first synchronization of a folder (`folder.date_last_sync` still empty), whose every mail would be "new", even when it was interrupted and resumed.

## Rhythm

After a pass, the work connection waits with IDLE on the Inbox, which ends:

- when the server reports something: a pass (the folders whose counters changed);
- at `sync.poll_interval_secs` (300 s) since the last full pass: a full pass;
- on `nicemailer sync now` (`sync_now`): a full pass;
- when the Mac wakes from sleep: the connection is surely dead, it is dropped without waiting for its time limit (90 s), and a new one opened at once. A sleep shows as a jump of the wall clock, which goes on during sleep, against the monotonic clock, which macOS stops;
- every 25 minutes at most, to renew IDLE (RFC 2177).

A server without IDLE is polled every minute. The first pass of each connection is full.

## Bodies

On the body connection: first the bodies the user is waiting for (a thread opened by `get_thread` whose mails have no body yet), then the Inbox, the most recent first, then the other folders. Each body is streamed into the raw store and completed in the database in batches ([ingestion.md](ingestion.md)).

A body whose size differs from the `RFC822.SIZE` the server announced is refused (`stockage-brut.md`, section 5, step 2), and not tried again until the core restarts: retrying would download it again and again. `sync status` counts them (`bodies_refused`). A server that counts sizes differently from what it sends would leave such messages without body: none was seen on Dovecot; to be watched on other servers.

## Failures

| Failure | What happens |
| --- | --- |
| Connection lost, server unreachable, time limit | New attempt after 5 s, doubled at each failure up to 5 min, with a little randomness; reset by a pass that succeeds. The body connection waits while the work connection does |
| Login refused | `AccountError`, state `auth_failed`, and no new attempt until the password is saved again (`nicemailer account password`) or the file changes: a client that keeps sending a wrong password may get the account locked by the server |
| No password, or one saved for another server | State `stopped`; the error is the account's (`list_accounts`), and the worker starts once the password is saved |
| A folder cannot be opened | The pass goes on with the others; the problem is printed on the core's standard error |
| Cut in the middle of a body | Nothing is stored for it ([ingestion.md](ingestion.md)); the next connection reads the queue again from the database and downloads it |

`AccountError` is emitted once per error (not at every attempt), `AccountChanged` when it goes away.

## `nicemailer sync`

```sh
nicemailer sync status [<key>] [--json]     # connection, last pass, folders, bodies left
nicemailer sync now [<key>]                 # a full pass now
nicemailer sync verify <key> [--skip-hash] [--json]
```

`sync verify` is the criterion of "perfect retrieval" (`plan-imap.md`, section 4): on a connection of its own, read-only, it compares each folder of the server with the database: same UIDs, same flags and keywords, and for each complete message a raw file whose hash is checked again (`--skip-hash`: only that the file exists). It exits with status 1 on any difference. Run while the synchronization works, it may see a mail the server has and the database not yet.

```
INBOX                      3 on the server,       3 here,       3 raw files checked,      0 bodies to come: ok
Archive                    1 on the server,       1 here,       1 raw files checked,      0 bodies to come: ok
Same on both sides (0.1 s)
```

`sync_status` gives, per account: `state` (`connecting`, `syncing`, `idle`, `waiting`, `auth_failed`, `stopped`), `error`, `mode` (`qresync`, `condstore`, `none`), `last_pass`, `passes` (count since the core started), `next_retry`, `bodies_missing`, `bodies_refused`, and per folder its counters, its last synchronization and whether the server still lists it.

`SyncProgress` (`account_id`, `folder_id`, `stage` `headers` or `bodies`, `done`, `total`) is a **transient** event: it has no `seq`, takes no place in the sequence and is never replayed (`evenements.md`). The app ignores it for now; it must not take it for a gap in the sequence (`AppModel.order`).

## Tests

| Test | What it checks |
| --- | --- |
| `crates/nm-store/tests/sync.rs` | folder listing (roles of the first listing only, a rename found by `UIDVALIDITY`, a role folder gone kept), flags and keywords, a move made elsewhere kept, a copy or an uncertain move inserted, flags and deletions in one transaction applied twice harmlessly, `uidnext` per batch, a duplicate refused, purge, the order of bodies, the commands refused |
| `crates/nm-daemon/tests/sync_rpc.rs` (`make imap-test`) | the real core against Dovecot, a second connection playing the phone. On both servers: first synchronization, new mail seen by IDLE and announced, flags, a move (with `MOVE`, and copy-delete-expunge on the minimal server) keeping the message's id, a deletion, the commands refused; `sync_verify` clean after each step. Also: one connection doing everything; a folder replaced (new `UIDVALIDITY`); a refused login waiting for a new password; network cuts at several moments (login, listing, bodies) without loss, duplicate or partial file |
| `apps/NiceMailer/Tests/NiceMailerTests/EventOrderTests.swift` | a transient event neither reloads the app nor moves its position in the sequence |
