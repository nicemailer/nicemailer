# The spool: changes sent to the server

What the user does in NiceMailer on an IMAP account (move a thread, mark it read, flag it, tag it, file a sender's mail after a decision, undo) is shown at once and sent to the server afterwards. This is phase 5 of `specifications/plan-imap.md`; the rules are in `specifications/modele de donnees.md`, section 6. The database side is `crates/nm-store/src/spool.rs`, the sending `crates/nm-engine/src/sync/spool.rs`.

## Principle

A command changes the `message` rows at once, and in the same transaction writes an **action** in `pending_action`, with what it takes to undo the change:

| `kind` | `payload` | `undo` |
| --- | --- | --- |
| `move` | `{from_folder_id, from_uid, to_folder_id}` | `{folder_id, uid}` |
| `set_flags` | `{flag, set, folder_id, uid}`: `\Seen`, `\Flagged`, or the keyword of a tag; where the message was when the flag changed | `{set}` |

A moved message has no UID until the server gives the new one: the UID of an IMAP message is only valid in its folder.

Every command goes through the same three places of `nm-store`, so none can forget the spool: `move_message` (moves, filings after a decision, undo), the end of the flag commands (`set_thread_seen`, `set_thread_flagged`), `tag_thread`.

Only the accounts the core synchronizes get actions: the IMAP accounts of `config.toml`, which the core gives the store (`Store::set_server_accounts`, kept in a temporary table of the writing connection so that the SQL of the commands can tell). The fake accounts of `make seed` and the archive accounts of mbox imports change locally only.

### Merging

- A move asked while the previous move of the same message is still queued is merged into it: Inbox → Todo, then Todo → Archive, is one move Inbox → Archive, with the `undo` of the first. Back to where it started, the action is cancelled, and the message gets its UID back.
- A flag set then cleared (or the reverse) before being sent cancels the first action: the server never hears of it.
- A move asked while the previous one is being sent is not merged (its source is on its way): it is queued after it, and gets its source UID when the first is done.

## Sending

The work connection of the account ([sync.md](sync.md)) sends the actions before each pass, and at once when a command queued some: the command wakes the worker, which leaves IDLE. Each round claims the actions that can run, marks them `running`, and groups them:

- the actions of one message run one at a time, in order (a flag waits for the move before it);
- moves with the same source and target: one `UID MOVE` for all, or, on a server without `MOVE`, `UID COPY`, `\Deleted` and `UID EXPUNGE` (UIDPLUS). `COPYUID` gives the new UID of each message;
- flags with the same folder, flag and value: one `UID STORE … .SILENT`.

Folders are opened with `SELECT` here: the spool is the only part of NiceMailer that writes on the server. The synchronization still uses `EXAMINE`.

| What happened | How the action ends |
| --- | --- |
| The server did it | `done`; a moved message gets its new UID from `COPYUID` |
| The source UID is gone: moved or deleted elsewhere (the phone, another Mac), or moved by this very action before a crash | `done`: not a failure. The message is looked for in the target by `Message-ID`, and gets its UID there if found |
| The server refused (`NO`: target folder missing, quota…) | `reverted` (below). The UIDs are checked once more first: one that went meanwhile is "gone", not refused |
| Connection lost, time limit | queued again, after 30 s, doubled at each failure (30 s, 1, 2, 4, 8, 16, 32 min); the 8th failure is definitive |
| A keyword in a folder that keeps none (`PERMANENTFLAGS` without `\*`) | `done` without sending: the tag stays local, and the synchronization does not erase it |

With 8 attempts and a first delay of 30 s, the longest wait is 32 minutes and the attempts span about an hour: the cap of an hour, also validated, is never reached.

### Reverted

A definitive failure undoes the change of the action and of every later action on the same message, latest first: the message goes back to its folder with its old UID, its flags as they were. The action is `reverted`, the later ones `cancelled`, an `audit_log` row `revert` gives the reason, and the event `ActionReverted {message_id, kind, reason}` tells the clients. The app says it in its status bar ("The server refused the move · undone: Mailbox doesn't exist: Archive").

### After a crash

At start, the core queues the `running` actions again. Each can be replayed: a move whose source UID is gone was done (the server moved the mail, the core stopped before marking it), so the rule "source UID gone" applies, and the message is found in its target. Tested by stopping the core right after the server's `MOVE` (`NICEMAILER_TEST_CRASH_AFTER_MOVE`, tests only).

### Purge

Actions ended (`done`, `reverted`, `cancelled`) are deleted after 7 days, at the start of a round.

## The synchronization and the spool

The synchronization must not undo what waits in the spool:

- **source folder**: the server still shows the UID of a move not sent yet; the pass ignores it, rather than insert the message again;
- **target folder**: a UID already known there (the move gave it by `COPYUID`) is not new; an unknown UID is first attached to a row of the folder without UID that has the same `Message-ID` (moved without `COPYUID`), rather than inserted twice;
- **flags**: a flag changed in NiceMailer and not sent yet keeps its local value, whatever the server says;
- **leftovers**: a row still without UID and without action pending, at the end of its folder's pass, is deleted: its move found the message gone and not in the target. It shows again through the folder it really is in.

## Two Macs, one mailbox

Two cores that move the same mail differently end the same, with no error for either: the second finds the UID gone and its move is done. When both move at the very same moment, the server may keep two copies (each `MOVE` saw the mail before the other expunged it): no client can prevent it, and both cores then show the two copies, as the server has them.

## Known limits

- A server with neither `MOVE` nor `UIDPLUS` cannot expunge just the moved messages: they are copied and flagged `\Deleted`, and stay in the source until another client expunges them. The synchronization then shows them there too, flagged deleted.
- The commands report success when the local change is made: a refusal comes later, as `ActionReverted`.

## `sync status`

`nicemailer sync status` shows, per account, the changes not sent yet ("changes to send to the server"; `actions_pending` in `sync_status`).

## Tests

| Test | What it checks |
| --- | --- |
| `crates/nm-store/tests/spool.rs` | an action with its local change; none for an account not synchronized; moves merged and cancelled by a return; a flag set then cleared sends nothing; tags as keywords; one action per message at a time, in order; a flag waiting for a UID; a move done after the message moved on gives its UID to the next move; retries and their delays, then the definitive failure; a refusal undoing the later changes too; actions requeued at start, purged after 7 days; the synchronization keeping local flags, attaching a moved message, dropping a row left without UID |
| `crates/nm-daemon/tests/sync_rpc.rs` (`make imap-test`) | against Dovecot: read, tag and move reaching the server on both servers (with and without `MOVE`), the message keeping its id; a move to a folder gone undone and announced; two cores moving the same mail agreeing with the server, without error; a crash right after the server's `MOVE`, replayed at the next start |
| `apps/NiceMailer/Tests/NiceMailerTests/EventOrderTests.swift` | the status bar message of `ActionReverted` |

The app's handling of `ActionReverted` has no scripted UI scenario: the UI scenarios run on the fake accounts of `make seed`, without a server, where no change can be refused.
