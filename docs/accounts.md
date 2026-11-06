# Creating, renaming and removing an IMAP account

How an IMAP account is created with its initial import, its folder roles changed, its key renamed, or the account deleted. This is phase 7 of `specifications/plan-imap.md`; the rules are in `specifications/import-initial.md`, `specifications/dossiers.md` ("A la création du compte") and `specifications/configuration.md` (sections 5 and 6). The core does it all (`crates/nm-daemon/src/setup.rs`, `crates/nm-engine/src/setup.rs` and `config_write.rs`, `crates/nm-store/src/setup.rs`); the CLI asks and shows.

These commands carry a password or delete mail: only the app, the TUI and the CLI may call them, never an agent (error `1007`).

## `nicemailer account add`

```sh
nicemailer account add perso --email me@example.fr --host imap.example.fr
# --port 993 --security tls --username me --ca-file … --display-name "Me"
# --role sent="Éléments envoyés"      the folder of a role (repeat it)
# --archive-read-before 2026-11-07    --archive-unread-before 2026-10-07
# --no-archive    --no-correspondents    --yes (no confirmation)
# --read-only     never change anything on this server (docs/sync.md, "Read-only accounts")
```

The password is asked without echo, or read from the standard input (`printf %s "$PW" | nicemailer account add … --yes`). Then:

1. **Test connection** (`test_account`): the server's capabilities and folders, the role proposed for each (special-use attribute, then the usual names in English and French: the rules of `nicemailer imap probe`), the Inbox's counts and `UIDNEXT`, and the number of people written to (distinct recipients of Sent, the account's own address excluded). Nothing is written anywhere.
2. **The import screen**, in text: what would be archived, what stays, how many correspondents go straight to the Inbox. The archiving is offered when the Inbox holds more than `import.threshold_total` (100) mails or `import.threshold_unseen` (50) unread ones; by default, every read mail and the unread ones older than `import.unseen_default_age_days` (30) days. Flagged mails always stay. The counts are exact (`count_archive`: the same `UID SEARCH` the archiving will run).
3. **Confirmation** (unless `--yes`), then `add_account`.

`add_account`, in this order, so that the worker started by the file finds everything ready:

| Step | What |
| --- | --- |
| Folders | Each workflow role gets a folder: the one given (`--role`), else the proposal, else a new folder under the role's English name (`Todo`, `Quarantine`…, in the personal namespace), created on the server |
| Account | Its row, the roles (`audit_log`: `role`, with the client's source), and the Inbox's screening boundary (`uid_screening_start`): the `UIDNEXT` read by the test connection, so that a mail arriving while the user reads the screen is screened (`import-initial.md`, section 5) |
| Correspondents | The recipients of the mails of Sent without decision, and who never had one, get the decision `inbox` (`created_by = 'core'`, reason `sent_correspondent`), unless `--no-correspondents`; unknown ones are created with `imported_from = 'imap_initial'` |
| Archiving | One spool action `initial_archive` on the account, with the criteria, if dates were given |
| Password | In the Keychain, bound to this server (host and user) |
| File | `[accounts.<key>]` added to `config.toml` (below), then applied at once: the worker starts |

### The archiving on the server

The worker sends the `initial_archive` action before anything else, and the Inbox is not synchronized until it ends: each old mail is fetched once, straight into Archive, instead of 18,000 headers in the Inbox then 18,000 moves (`import-initial.md`, section 6). It searches the Inbox below the boundary, not flagged, read before one date or unread before the other (`BEFORE` is the internal date), then moves by batches of `import.batch_size` (1,000) with `UID MOVE`.

- **Replayable**: interrupted (the core stopped, the connection cut), it searches again and finds only what is left. Tested by killing the core after the first batch (`NICEMAILER_TEST_CRASH_DURING_ARCHIVE`, tests only).
- **Temporary failure**: tried again by the spool's rules ([spool.md](spool.md)).
- **Definitive failure** (no Archive folder, say): the action is `reverted` with the server's answer, and nothing local is to undo. `nicemailer sync status` shows "initial archiving: reverted (…)".
- **Journal**: one `initial_archive` row on the account, with the criteria and the number of mails moved; not one per mail.

The mails already in the mailbox are synchronized without screening (below the boundary); the addresses met then are marked `imported_from = 'imap_initial'`. After the import, every new mail is screened ([screening.md](screening.md)): a sender of the history without decision goes to Quarantine with their next mail.

## Trying NiceMailer on a mailbox in production

To look at a real mailbox without any risk to it, in a data directory and a settings file of their own, apart from the everyday ones:

```sh
# Terminal 1: a core of its own.
export NICEMAILER_DATA_DIR=~/nm-trial NICEMAILER_CONFIG=~/nm-trial/config.toml
make core

# Terminal 2, same variables: the account, read-only.
nicemailer account add trial --email you@example.fr --host imap.example.fr --read-only
nicemailer sync status trial          # until "idle" and no body left
nicemailer sync verify trial          # every folder, every raw file checked
```

With `--read-only`, the test connection and every connection after it refuse any writing command before it leaves (`docs/sync.md`, "Read-only accounts"): no folder created, no archiving, no flag, no move. New mail is screened dry: where each would have gone is in `audit_log`:

```sh
sqlite3 ~/nm-trial/nicemailer.db "SELECT json_extract(detail, '$.folder'), json_extract(detail, '$.rule'), count(*)
  FROM audit_log WHERE action = 'screen' GROUP BY 1, 2"
```

`nicemailer sync status` also shows the `authserv-id` detected on the server's `Authentication-Results` (open question 3 of `plan-imap.md`). Removing the trial afterwards: `nicemailer account remove trial` (the server is not touched), then delete `~/nm-trial`.

## Writing `config.toml`

The file belongs to the user (`configuration.md`, section 5), so the core writes it only at a client's request, and:

- edits it in place with `toml_edit`: comments, order and formatting stay, only the account's tables change. Comments at the end of the file stay above the new account;
- reads it again right before the change, so that an edit made by hand meanwhile is kept;
- refuses a result the core would refuse (`config::parse`);
- writes it atomically: a temporary file next to it, then `rename`;
- writes no default value: `username` only when it is not the address.

## `nicemailer account roles`

```sh
nicemailer account roles perso todo=Todo archive=Archives --create-missing
```

`set_folder_roles`: gives roles to folders of an existing account (one folder per role: the previous one loses it), creating on the server the folders named that it lacks if `--create-missing`; then a pass at once. Each change has a `role` row in `audit_log`.

## `nicemailer account rename`

```sh
nicemailer account rename perso home
```

Renames the key and keeps everything: the account row (same id, so mail, history, decisions), the Keychain item (`imap:perso` becomes `imap:home`), and the tables in the file, where they were. The database first, then the Keychain, then the file: the file read next finds the account under its new key. Renaming the key by hand in the file instead creates a new account and disables the old one (`configuration.md`, section 6).

## `nicemailer account remove`

```sh
nicemailer account remove perso        # asks to type the key again; --yes not to
```

Stops the account's worker and waits for it, removes its tables from the file, deletes its data (folders, mails, threads, actions; the raw files stay until the garbage collector of phase 8) with a `remove` row in `audit_log`, which outlives it, and deletes its Keychain item. The mail on the server is not touched.

## Accounts declared by hand

An account written in `config.toml` by hand (or by an agent: its password can still only be entered in the app or the CLI) skips the creation steps. Its first synchronization then gives each folder the proposed role and sets the Inbox's boundary to its `UIDNEXT`, without archiving or correspondents ([sync.md](sync.md)).

## Tests

| Test | What it checks |
| --- | --- |
| `crates/nm-engine/src/config_write.rs` (unit) | adding keeps comments and order, comments at the end stay above, into an empty file, no default written, keys checked; renaming keeps the tables in place; removing takes only the account; atomic write, a hand edit kept, an invalid result writes nothing, no temporary file left |
| `crates/nm-engine/src/setup.rs` (unit) | dates and the archiving's search |
| `crates/nm-store/tests/setup.rs` | the account row, roles moved, correspondents (decided, removed, own address), the archiving's life (claimed, retried, done with its journal row, failed), the Inbox waiting for it, addresses of a first synchronization marked imported, rename and remove |
| `crates/nm-engine/tests/keychain.rs` | a password removed |
| `crates/nm-daemon/tests/setup_rpc.rs` (`make imap-up`, `make imap-test`) | against Dovecot: a small Inbox left as it is, the people written to accepted, a mail arriving during the screen screened, the file written with its comment, an imported sender's next mail in Quarantine; old mail archived on the server exactly as counted, flagged mail left; an archiving interrupted after a batch resumed without loss or duplicate; an account renamed with its data and password, then removed, the server untouched; an agent refused |
