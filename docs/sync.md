# Synchronization of the IMAP accounts

The core keeps the database in line with the server of every IMAP account of `accounts.toml`: new mail, flags, deletions, moves made on the phone or the webmail. This is phase 4 of `specifications/plan-imap.md`: **server to database**. The other direction, the moves, flags and tags made in NiceMailer, goes through the spool ([spool.md](spool.md), phase 5).

The code is in `crates/nm-engine/src/sync/` (the connections and the passes) and `crates/nm-store/src/sync.rs` (what is written in the database, testable without a server).

## Read-only towards the server

The synchronization itself cannot change anything on the server, even by mistake: every folder is opened with `EXAMINE` (the read-only `SELECT`), and every fetch is a `PEEK`, which never marks a mail read. Only the spool writes ([spool.md](spool.md)), before each pass on the same connection, and the pass does not undo what waits in it (moves not sent, flags not sent).

(In phase 4 alone, before the spool, the commands that changed mail were refused on these accounts; that refusal is gone.)

## Read-only accounts

`read_only = true` in `[accounts.<key>]` (or `nicemailer account add … --read-only`) is for a mailbox in production that NiceMailer must look at without touching: nothing is ever changed on its server.

- **The session refuses.** Each connection of the account is told to write nothing (`Session::forbid_writes`): `SELECT`, `STORE`, `COPY`, `MOVE`, `EXPUNGE`, `APPEND` and `CREATE` fail before they are sent (`ImapError::ReadOnly`), whatever the rest of NiceMailer asks. Only `EXAMINE` and the `PEEK` fetches go out.
- **No spool.** The account is not among the accounts whose changes go to the spool, and the spool is not even run for it.
- **Commands refused.** Moving a thread, marking it read or flagged, tagging it: error `1010` (`READ_ONLY`), since the next synchronization would undo a local change. A decision on a sender is recorded, but files none of this account's mail.
- **Screening dry.** A new mail of the Inbox is screened as usual, and its `screen` row in `audit_log` says where it would go, with `"dry_run": true`; it stays in the Inbox, unmarked. A mail found in Sent has no effect. `audit_log` thus shows what NiceMailer would have done.
- **Creation.** `add_account` with `read_only` creates no folder (the roles without a folder stay unassigned) and refuses any archiving.
- **Made writable.** `activate_account` (or `nicemailer account activate`) runs the creation's steps then, and removes `read_only` from the file: the worker restarts writable ([accounts.md](accounts.md), "Looking first"). Editing the file by hand instead skips those steps: no folder is created, no old mail archived.

Tested against Dovecot by taking a snapshot of the server (folders, UIDs, flags) before NiceMailer works on the account, and comparing after (`crates/nm-daemon/tests/setup_rpc.rs`, `a_read_only_account_changes_nothing_on_its_server`), and by a scripted server that would fail on any writing command (`crates/nm-proto/tests/imap_script.rs`).

## Connections

Each account has two connections, each on a thread of its own:

| Connection | Job |
| --- | --- |
| Work | The passes over the folders; between two passes, IDLE on the Inbox |
| Bodies | Downloads the bodies, in the background ([ingestion.md](ingestion.md)) |

`sync.max_connections = 1` (in `[sync]`, or `[accounts.<key>.sync]`) puts both jobs on the work connection: bodies are then downloaded 30 s at a time between two passes. Use it for a server that allows few connections per user.

The work connection also takes the bodies over, the same way, whenever the body connection has no session: before its first login, while it waits for the budget of logins (after a few sleeps of the Mac, up to an hour), or after a failure until its next attempt succeeds. As long as the work connection is up, a new mail gets its body within seconds of its headers, and a mail opened gets its body first. The two connections never download at the same time (one holds the turn, and the other reads its queue again after it), so no body is downloaded twice. Until 20 November 2026 the body connection kept the bodies to itself while it waited: new mail came in with its headers alone and showed "(body not downloaded)" for as long as the wait, even when opened.

The plan had a third connection for IDLE alone. The work connection leaves IDLE within milliseconds when it has something to do, so a third one would only use one more of the few connections servers allow (Dovecot: 10 per user and address by default).

## A pass

1. **The folders.** `LIST`, with `LIST-STATUS` when the server has it, else one `STATUS` per folder to synchronize. The `folder` rows follow the server:
   - a new folder gets a row, **without role**: NiceMailer manages only the folders of its workflow (`dossiers.md`);
   - except the first listing of an account, which has no role at all: each folder then gets the role NiceMailer proposes for it (INBOX, the special-use attribute, then the usual names; the rules of `nicemailer imap probe`). This applies to an account declared by hand in `accounts.toml`; an account created by `nicemailer account add` (`add_account`) has its roles confirmed by the user ([accounts.md](accounts.md)). Recorded in `audit_log` (`role`, reason `proposed`);
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
4. **Arrivals.** The headers, flags, date and size of the new UIDs (`BODY.PEEK[HEADER]`), 100 per command and per transaction ([ingestion.md](ingestion.md), first step). `uidnext` advances with each batch. In the same transaction, each new mail gets its authentication verdict and, in the Inbox, is screened ([screening.md](screening.md)); the actions the screening queues are sent right after the pass.
5. **End of each folder.** Flags, deletions and the new `UIDVALIDITY`, `UIDNEXT`, `HIGHESTMODSEQ`, in one transaction.

`UIDVALIDITY` changed: the UIDs of the folder name other messages, so its messages are dropped and the folder is fetched again from the start (`modele de donnees.md`, section 5).

### Crash safety

`uidnext` advances in the transaction of each batch of arrivals: after a crash, the next pass fetches from the first UID not written, never twice (`UNIQUE (folder_id, uid)` refuses a duplicate anyway). `HIGHESTMODSEQ` only advances at the end of the folder, with the flags and deletions: after a crash before it, the server reports the same changes again, and applying them twice changes nothing.

### Moves made elsewhere

A mail moved by the phone from Inbox to Archive vanishes from the Inbox and appears in Archive. Deleted then inserted again, it would lose its id, tags, history and consultation. The pass therefore finds every deletion of every folder (step 3) before writing any arrival (step 4): a new mail is the move of an existing row when

- it has a `Message-ID`, and exactly one row of the account has it, in another folder;
- of the same size (RFC822.SIZE, or the bytes stored once the body came);
- whose UID vanished in this pass.

The row then moves (`folder_id`, `uid`, flags, `modified_by = 'imap'`) and keeps everything else. Like every change seen on the server, it is not journaled in `audit_log` (`tracabilite.md`, "Ce qui n'est pas journalisé"). Anything else is a new row: a copy (the original stayed), or a move that cannot be told from one.

After IDLE reports a change, the pass waits 1.5 s. A client that moves a mail without `MOVE` copies it, marks it `\Deleted`, then expunges it, in three commands; a pass started at the first would see a new mail and the original still there. The delay also gathers a burst of new mail into one pass.

### New mail

`MessageArrived` (`message_id`, `folder_id`) for each new row, after the folder's `FolderChanged`. Not during the first synchronization of a folder (`folder.date_last_sync` still empty), whose every mail would be "new", even when it was interrupted and resumed.

## Rhythm

After a pass, the work connection waits with IDLE on the Inbox, which ends:

- when the server reports something: a pass (the folders whose counters changed);
- at `sync.poll_interval_secs` (300 s) since the last full pass: a full pass;
- on `nicemailer sync now` (`sync_now`): a full pass;
- when the Mac wakes from sleep: the connection is tested, and kept if it answers ("Sleep of the Mac" below);
- every 25 minutes at most, to renew IDLE (RFC 2177).

A server without IDLE is polled every minute. The first pass of each connection is full.

## Pace of the connections

A server sees every login, and one that sees too many locks the account. Orange did on 8 November 2026: its IMAP server closes a connection left in IDLE after 45 to 60 seconds (RFC 2177 lets a client wait 29 minutes), without a TLS goodbye (`peer closed connection without sending TLS close_notify`). Each cut was followed by a new connection 5 s later, because the pass made on it succeeded and reset the delay: about a login a minute on the work connection, as many on the body connection, for twenty minutes, and the account was locked, webmail and phone included. Three rules now make that impossible (`crates/nm-engine/src/sync/pace.rs`):

| Rule | What it does |
| --- | --- |
| A budget of logins | At most 10 per account over any hour, both connections together, attempts that fail included. Past it, the next connection waits until the oldest login of the hour leaves it, whatever happens (`sync_now` does not shorten it). The core logs `N logins in the last hour, the most allowed: next one in … s`. The hour is real time, the sleeps of the Mac included ("Sleep of the Mac") |
| A delay a short connection does not reset | After a failure, the delay doubles from 5 s up to 30 min, and goes back to 5 s only after a connection that lived 10 minutes. A pass that succeeds is not enough |
| IDLE adapted to the server | A connection cut while in IDLE teaches how long the server keeps one: IDLE is then ended and started again (`DONE`, `IDLE`) at half that time, and the account keeps one connection only (the body connection closes; bodies are fetched on the work connection, 30 s at a time). If the server cuts it even so, or cut within 10 s, the account is **polled**: a connection per pass, at most every 10 minutes (`sync.poll_interval_secs` if longer), closed after its pass |

Such a cut is not an error: the mail goes on arriving, so it is not announced (`AccountError`) and the state is `waiting`, without error, until the next connection. The core logs it: `the server closed the connection after 47 s in IDLE (…); IDLE renewed every 23 s from now on`, or `the account is polled from now on, one connection per 10 min at most`. `nicemailer sync status` shows the mode next to the account (`IDLE renewed every 23 s`, `polled every 10 min`; `hold` in `sync_status`: `renew N`, `poll N`).

What is learnt lasts as long as the worker: a restart of the core, or a change of the account in `accounts.toml`, learns it again, at the cost of one or two cuts. Whether Orange counts a renewed IDLE as activity is not known yet: if it does not, the account ends up polled.

## Sleep of the Mac

A Mac left alone sleeps and wakes every few minutes, often for a few seconds only (idle sleep, then a "dark wake" for the network, then sleep again: `pmset -g log`). On 21 November 2026 this left three accounts without connection for over an hour while the phone showed the new mail:

- each wake dropped both connections and opened two new ones: five wakes spent the budget of 10 logins an hour;
- the budget and the waits were counted on the clock of `Instant` and of tokio's timers, which macOS stops during sleep: "an hour" lasted an hour of awake time, several real hours, and the next attempt that `sync status` announced (on the wall clock) passed without any attempt.

Now (`crates/nm-engine/src/sync/clock.rs`, `account.rs`):

| Rule | What it does |
| --- | --- |
| Real time | The budget of logins, the delays before a new attempt and the time since the last full pass are counted on the *continuous* clock (`CLOCK_MONOTONIC` on macOS), which goes on during sleep and cannot be set. A wait sleeps 15 s at most at a time on tokio's timer, then reads that clock again: it ends at the time announced, or at the wake if that time passed during the sleep |
| Seeing a sleep | Every 15 s, the gap between the continuous clock and the awake clock (`CLOCK_UPTIME_RAW`, which stops during sleep): a gap of more than 5 s is a sleep. Neither clock moves when the wall clock is set or corrected |
| Testing before dropping | At a wake, IDLE is ended (`DONE`) or a `NOOP` sent; a connection that answers within 10 s is kept, and a full pass follows on it, without a login. One that does not is dropped. Neither is announced as an error, nor taken for a server that cuts IDLE ("Pace of the connections") |
| Once awake | No login before the Mac has been awake 15 s (`kern.waketime`), so that a dark wake of a few seconds spends none. Logged as `the Mac has just woken, login in … s`; `sync status` shows `waiting` meanwhile |
| One login, not two | Only the work connection logs in again. A body connection that died with the sleep waits until the work connection leaves bodies after one of its slices of 30 s; until then the work connection fetches them, so a few new mails need no second login |

The core logs each wake: `the Mac slept, the connection is still alive`, or `… did not survive: new connection` (and `(bodies) … the work connection fetches the bodies`). A wake therefore costs no login when the connections survive it, one otherwise, and none during a dark wake.

## Bodies

On the body connection (or the work connection, "Connections" above): first the bodies the user is waiting for (a thread opened by `get_thread` whose mails have no body yet), then the Inbox, the most recent first, then the other folders. Each body is streamed into the raw store and completed in the database in batches ([ingestion.md](ingestion.md)).

A body whose size differs from the `RFC822.SIZE` the server announced is refused (`stockage-brut.md`, section 5, step 2), and not tried again until the core restarts: retrying would download it again and again. `sync status` counts them (`bodies_refused`). A server that counts sizes differently from what it sends would leave such messages without body: none was seen on Dovecot; to be watched on other servers.

## Failures

| Failure | What happens |
| --- | --- |
| Connection lost, server unreachable, time limit | New attempt after 5 s, doubled at each failure up to 30 min, with a little randomness; back to 5 s only after a connection that lived 10 minutes, and never more than 10 logins an hour ("Pace of the connections"). The body connection waits while the work connection does |
| Connection cut by the server while in IDLE | Not an error: learnt, IDLE renewed sooner or the account polled ("Pace of the connections") |
| Login refused | `AccountError`, state `auth_failed`, and no new attempt until the password is saved again (`nicemailer account password`) or the file changes: a client that keeps sending a wrong password may get the account locked by the server |
| No password, or one saved for another server | State `stopped`; the error is the account's (`list_accounts`), and the worker starts once the password is saved |
| A folder cannot be opened | The pass goes on with the others; the problem is printed on the core's standard error |
| Cut in the middle of a body | Nothing is stored for it ([ingestion.md](ingestion.md)); the next connection reads the queue again from the database and downloads it |

`AccountError {account_id, error, why}` is emitted once per error (not at every attempt), `AccountChanged` when it goes away; `why` is the reason (`imap_auth_refused`, `imap_failed` with the server's words, or a refusal of the password, `docs/contract.md`, "Errors"), as is `why` in `list_accounts`.

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

### Progress

`SyncProgress` (`account_id`, `folder_id`, `stage` `headers` or `bodies`, `done`, `total`) is a **transient** event: it has no `seq`, takes no place in the sequence and is never replayed (`evenements.md`); a client must not take it for a gap in the sequence (`AppModel.order`). At most four a second:

- **Headers**, per folder: said at `done` 0 as soon as the folder's new mails are counted, before the first batch of 100 (a slow server may take seconds over it), then every 250 ms at most, and at `done` = `total` when the folder is done.
- **Bodies**, per account (`folder_id` null): every 500 ms at most while bodies come in, and once more with `done` = `total` when none is left. `total` is `done` plus the bodies still queued, and `done` starts again from 0 each time the queue is read again: what is left (`total − done`) is the number that means something.

The app shows it at the right of its status bar, so that the first synchronization of a large account (tens of thousands of mails, minutes) does not look like a frozen app: "Importing Inbox: 12,300 of 44,829", then "Downloading mails: 41,429 left", the account first when there are several ("perso · …"). The headers come first, as a mail is listed only once its headers are in. A stage ends with its last event, or after 30 s without one (a pass cut by the network sends no end). The line stays beside the messages of the bar, and is never cut (`AppModel+Sync.swift`, `GeekStatusBar`).

## Tests

| Test | What it checks |
| --- | --- |
| `crates/nm-store/tests/sync.rs` | folder listing (roles of the first listing only, a rename found by `UIDVALIDITY`, a role folder gone kept), flags and keywords, a move made elsewhere kept, a copy or an uncertain move inserted, flags and deletions in one transaction applied twice harmlessly, `uidnext` per batch, a UID already known not inserted twice, purge, nothing of it journaled (`tracabilite.md`), the order of bodies |
| `crates/nm-daemon/tests/sync_rpc.rs` (`make imap-test`) | the real core against Dovecot, a second connection playing the phone. On both servers: first synchronization, new mail seen by IDLE and announced, flags, a move (with `MOVE`, and copy-delete-expunge on the minimal server) keeping the message's id, a deletion, changes made in NiceMailer reaching the server ([spool.md](spool.md)); `sync_verify` clean after each step. Also: one connection doing everything; a folder replaced (new `UIDVALIDITY`); a refused login waiting for a new password; network cuts at several moments (login, listing, bodies) without loss, duplicate or partial file |
| `crates/nm-engine/src/sync/pace.rs` (unit tests) | the budget of logins (ten an hour, then the wait; a login a minute for an hour impossible; an hour asleep counts); no login within 15 s of a wake; what a cut in IDLE teaches (renewal at half, then polling; polling at once for a cut within 10 s) |
| `crates/nm-engine/src/sync/clock.rs` (unit tests) | the continuous clock never behind the awake clock; a gap of a few seconds is a sleep, a drift is not; a wait lasts its time on the continuous clock |
| `crates/nm-daemon/tests/pace_rpc.rs` (`make imap-test`) | the real core against Dovecot through a proxy that cuts connections as Orange did, without a TLS goodbye, counting them: cut after 12 s of silence, IDLE renewed every 6 s, no new connection for 30 s, a new mail still seen by IDLE and its body fetched on the same connection, no error shown; cut after 20 s whatever happens, polling chosen after the second cut, no connection before the next poll; every connection after the first cut at once (the body connection never gets a session), the bodies still fetched, a new mail's within seconds. And the sleeps of the Mac, simulated (`NICEMAILER_TEST_SLEEP_FILE`, `clock.rs`): connections that survive a sleep kept, without a login, IDLE working on them; connections frozen by the proxy during the sleep replaced by a single login, made only once the Mac has been awake 15 s, without error, the new mail and its body through the work connection |
| `apps/NiceMailer/Tests/NiceMailerTests/EventOrderTests.swift` | a transient event neither reloads the app nor moves its position in the sequence |

## Volume

`make volume-test` (after `make imap-up`; `crates/nm-daemon/tests/volume_rpc.rs`): a mailbox of `NM_VOLUME` generated mails (100,000 by default: 60 % Inbox, 30 % Archive, 10 % Sent, bodies of 1 to 6 kB, one mail in three a reply) on the complete test Dovecot, synchronized from nothing by the real core, in release mode. Filling the mailbox takes the test Dovecot about 20 minutes; `NM_VOLUME_USER=<user>` synchronizes again a mailbox filled by an earlier run (its name is printed).

Measured on 2026-11-06 (MacBook, Dovecot in Docker on the same machine):

| Measure | 100,000 mails |
| --- | --- |
| Headers of every mail (the lists are complete) | 20 s |
| Everything, bodies included | 728 s (about 140 bodies a second) |
| Core memory (RSS), at most | 289 MB |
| List of the Inbox (60 threads) during the sync | 2 ms average, 4 ms p99, 5 ms at worst (2,430 samples) |
| Database (with the WAL) / raw store | 807 MB / 390 MB |
| `sync verify --skip-hash` | 0.7 s |

The lists stay instant while the core writes, thanks to the read pool on WAL. The bodies are the long part; on a real server, latency rather than the core sets their pace. The database is twice the raw store: it holds the full-text index and the extracted text of each body, on top of the headers.
