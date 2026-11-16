# Settings and accounts: `config.toml` and `accounts.toml`

The settings of NiceMailer are in two TOML text files that the core reads at start and watches while it runs:

- **`config.toml`**: every setting of the application (intervals, sizes, display);
- **`accounts.toml`**: the accounts, their servers and identities.

The specification is `specifications/configuration.md`; this page describes what is implemented (phase 1 of `specifications/plan-imap.md`).

Two files rather than one: the general settings can be shared between Macs, or published in a dotfiles repository, without the addresses and servers of the accounts, which are personal and differ from one Mac to the next. Neither file holds a password.

## Where the files are

`~/.config/nicemailer/config.toml`, or the file named by `NICEMAILER_CONFIG` (the core also takes `--config <file>`). `accounts.toml` is always **next to it**, in the same directory, whatever the name of the first one: one variable moves both, so a test core can never read the real accounts with its own settings.

```sh
nicemailer config path                # the two files the core reads
nicemailer config defaults            # a complete config.toml, every key at its default, with comments
nicemailer config defaults --accounts # the model of accounts.toml, with an example account
nicemailer config check               # validate both files without applying them (status 1 if refused)
nicemailer config check other/config.toml   # ... and other/accounts.toml
```

### Created at the first start

When a file is missing at the start of the core, the core creates it:

- `config.toml` lists **every setting, commented out, with its default value and its role** (the same text as `nicemailer config defaults`, each `key = value` behind a `# `). A user who changes nothing still sees what can be changed. To change a setting, remove the `# ` in front of it: the table headers (`[sync]`, `[ui]`…) are not commented, so the line is already in the right table.
- `accounts.toml` holds no account, only a commented example of one.

Commented out rather than written: a value written in the file is the user's choice and stays when a later NiceMailer changes its default, whereas a commented line changes nothing. The created files therefore give exactly the defaults.

The rules of the creation (`config_write::create_missing`):

- **An existing file is never touched**, even empty or invalid. A symbolic link counts as existing even when its target is missing (a dotfiles repository not cloned yet).
- **Complete or not at all.** The file is written under a temporary name, then given its name with `hard_link`, which fails if the name is taken: a file created meanwhile is kept, and no reader sees half a file.
- **Only at the start.** A file deleted while the core runs stays deleted until the next start: a missing `config.toml` means every default, a missing `accounts.toml` means no account.
- A file that cannot be created (a read-only directory) is not fatal: the core logs it and runs with the defaults.

A key absent from a file takes its default value.

## Accounts

An account is a table `[accounts.<key>]` of `accounts.toml`. The key (letters, digits, `-`, `_`, `.`; 64 at most) names it, and is what the database links its folders and mail to. The tables keep their `accounts.` prefix in the file of accounts: keys stay the same everywhere (errors, `ConfigChanged`, `audit_log`), and a table can be copied from an older single file as it is.

An `[accounts…]` table in `config.toml` is an **error**, not a warning: the file is refused, and the message says to move the table to `accounts.toml`. Ignoring it would start the core without the user's accounts, silently.

```toml
version = 1

[accounts.perso]
email = "me@example.com"          # required
display_name = "My Name"
auth = "password"                 # or "oauth2": accepted, not supported yet

[accounts.perso.imap]             # required
host = "imap.example.com"         # required
port = 993                        # default: 993 for tls, 143 otherwise
security = "tls"                  # tls (default), starttls, or none (localhost only)
username = "me"                   # default: the email
ca_file = "~/certs/my-ca.pem"     # extra certificate authority for this server

[accounts.perso.smtp]             # the server mail is sent through (sending.md)
host = "smtp.example.com"         # ports: 465 tls, 587 starttls, 25 none

[[accounts.perso.identities]]
email = "contact@example.com"
name = "My Name"

[accounts.perso.sync]             # overrides [sync] for this account
poll_interval_secs = 60
```

- **`email` and `identities`.** `email` is the main address: required, the login name unless `username` is set, and the default sender. `[[accounts.<key>.identities]]` (double brackets: an item of a list, repeated once per address) are the other addresses the mailbox sends from, aliases or other domains. They are never used to log in; with the main address they are the only senders a draft accepts, a reply goes from the one the mail was sent to, and they count as the user's own addresses (writing from one does not make it a correspondent). NiceMailer does not check that the SMTP server allows them: an address it refuses fails at sending.
- **`config_sync`** (default `false`) will share the decisions of the mailbox (screening, folder roles, tags, groups) with the other Macs or people using it, through a folder `nicemailer-sync` on its server (`synchronisation-configuration.md`). Off by default because anyone who can open the mailbox can read that folder. Not implemented yet: the key is validated and has no effect.
- **`security = "none"`** is refused unless the host is `localhost`, `127.0.0.1` or `::1`: elsewhere it would send the password in clear text.
- **`ca_file`** adds an authority to the system's, for a self-hosted server or the test servers (`dev/dovecot/tls/ca.crt`). There is no way to turn certificate checks off. Certificates are checked by macOS, with its own rules (at most 825 days, "server authentication" usage): see [imap.md](imap.md), "Connecting".
- **Per-account overrides.** `[accounts.<key>.sync]` and `[accounts.<key>.import]` start from the general `[sync]` and `[import]` of `config.toml` and replace the keys they set. The other tables are global. A `[sync]` at the root of `accounts.toml` is an unknown key (a warning): it has no effect.

### What happens to the database

The core keeps the `account` table in line with the keys of `accounts.toml` (`nm-store`, `accounts.rs`), each time it reads it:

| In the file | In the database |
| --- | --- |
| A new key | A new account (`audit_log`: `account` / `create`) |
| A key that disappears | The account is **disabled**: `account.disabled_at` is set, nothing is deleted (`disable`) |
| A disabled key that comes back | The same account, same id, enabled again, with all its mail (`enable`) |
| A key equal to the name of an archive account | Ignored, with a warning: rename one of the two |

Disabling instead of deleting is the rule of the specification: going back to an older version of the file in Git, or taking the file of another Mac, must not erase gigabytes of cached mail. Renaming a key by hand therefore creates a new account and disables the old one; `nicemailer account rename` does it while keeping the data, and `nicemailer account add` writes a new account into `accounts.toml` without touching the rest ([accounts.md](accounts.md)). Account commands never write `config.toml`.

Archive accounts, created by an mbox import ([import-mbox.md](import-mbox.md)), are never in the file and never disabled.

Disabled accounts are still listed, and their mail is still shown (the user's choice); only their synchronization stops (when it exists).

### Order of the accounts

`account.position` orders the accounts: `list_accounts` lists them by position (then id), and so does `list_folders`, whose order the app's account picker (`@`) and sidebar follow. A new account, from the file or from an mbox import, goes after the others; accounts created by the same reading of the file come in the order of their keys. A disabled account keeps its place.

The order is a state of the interface, not an intention about the servers, so it is in the database, not in `accounts.toml`. It is changed by `set_account_order {account_ids: [...]}`: the accounts given come first, in this order, the others after them in their current order, and positions become 1, 2, 3… An unknown or repeated id is refused. Each account that moved gets an `audit_log` line (`reorder`, before and after), an `AccountChanged`, and a `FolderChanged` for each of its folders, which makes the app reload its folder list.

```sh
nicemailer account order pro perso    # pro first, then perso, then the others
```

### `list_accounts` and `nicemailer accounts`

```sh
nicemailer accounts
  1  perso        imap     me@example.com                 ok
  2  old          archive  old@example.com                ok
  3  pro          imap     -                              disabled (no longer in accounts.toml, mail kept)
```

`list_accounts` returns, for each account in display order: `id`, `key`, `kind` (`imap` or `archive`), `email`, `display_name`, `enabled`, `position`, and `error`, the reason it cannot connect if one is known. `error: null` does not mean connected: there is no connection yet in this phase.

## Passwords

Passwords are in the macOS Keychain, never in a file (`configuration.md`, section 2). A `password` key in `accounts.toml` is an error: the file is refused, and the message says how to save the password instead. It is refused rather than ignored, so that a password does not stay, in clear text, in a file the specification invites to keep in Git.

```sh
nicemailer account password perso                 # asked without echo
printf %s "$PW" | nicemailer account password perso   # from a script
```

The CLI sends the password to the core (`set_password {account, password}`, over the socket only the user can open), which saves it; the app will do the same. The core alone reads and writes the Keychain. Only the app and the CLI may call `set_password`: an agent (`mcp`) or the web server gets `FORBIDDEN` (1007), so that none can bind a password of its choosing to a server of its choosing. Each save is in `audit_log` (`account` / `set_password`, with the host and the user name, never the password).

The password never leaves the core otherwise: `SecretString` prints `SecretString(***)`, and `SetPasswordParams` prints `***` too.

### The Keychain item

One generic password of the login keychain per account:

| Attribute | Value |
| --- | --- |
| service | `NiceMailer` |
| account | `imap:<key>` |
| label | `NiceMailer: <key> (IMAP)`, the name Keychain Access shows |
| comment | `{"host": "…", "username": "…"}`, the server it was saved for |
| data | the password |

### A password belongs to its server

The server a password was saved for is the one the file named when it was saved. If the file now names another host or user, the core refuses to send the password, and the account gets the error "the password was saved for … on …, the file now says … on …: save the password again to confirm the new server". Saving the password again is the confirmation. Without this rule, anyone able to edit the file, an agent tricked by a mail included, could point the account at their own server and receive the password at the next login.

It is checked twice:

- **at each reading of the file** (`secrets::check`), from the attributes of the item only. The Keychain gives attributes to anyone, without any dialog, even when it is locked, so the core checks them with its locks held, and an account without a password or pointing at another server shows its error at once;
- **at each login** (`secrets::imap_password`, from phase 2), on the item read with its password: the file may have changed since the last check.

### No dialog, ever

An item of the Keychain has an access list: the application that saved it reads it without a dialog, any other one needs the user's permission. The core runs in the background, so it turns the Keychain's dialogs off at start (`keychain::never_prompt`): a dialog would appear out of nowhere and block the core. An access that would need one fails instead, and the account's error says so ("the Keychain refused the password of this account: it is locked, or the password was saved by another build of the core").

### Development: an Apple Development certificate, once

macOS recognizes an application by its code signature. A Rust binary is signed by the linker with a signature of its own content, so every rebuild of the core would be a new application, refused by the items the previous build saved. `make core` therefore signs the core (`scripts/sign-core.sh`, identifier `org.nicemailer.core`).

The certificate must carry an Apple **Team ID**. Besides its access list, an item of the login keychain has a *partition*, which macOS sets from the signature of the program that created it:

| Signature of the core | Partition of the items it saves | The next build |
| --- | --- | --- |
| Apple Development (Team ID `ABCDE12345`) | `teamid:ABCDE12345` | reads them |
| self-signed (`NiceMailer Dev`), ad hoc, unsigned | `cdhash:<hash of this very binary>` | refused |

An Apple Development certificate is free with an Apple ID, once:

1. Xcode → Settings → Accounts → **+** → Apple ID; a "Personal Team" appears.
2. Select it → **Manage Certificates…** → **+** → **Apple Development**.
3. `security find-identity -v -p codesigning` must list it as valid. If it says `0 valid identities found`, the intermediate certificate of Apple that issued it is missing: install "Worldwide Developer Relations - G3" from <https://www.apple.com/certificateauthority/> (double-click, login keychain).

At the first signature macOS may ask whether `codesign` may use the key: answer **Always Allow**. Renewing the certificate, every year, keeps the Team ID and the name the access list checks: the saved passwords should stay readable (not yet observed: the first renewal will tell).

`sign-core.sh` uses `$SIGN_IDENTITY` when set, else the first valid Apple Development identity, else the self-signed `NiceMailer Dev` (`make dev-cert`) with a warning: a core signed with it reads its passwords until the next rebuild only.

A keychain made by `security create-keychain`, as the tests use, gets no partition: there, two builds signed with the same self-signed certificate do share their items. This is why the first design, which relied on `NiceMailer Dev` alone, passed its tests and failed on the login keychain.

A core run without signing (`cargo run`) works, but cannot read the passwords saved by `make core`; its accounts say so. A password saved by a build with another signature cannot be read or replaced by the signed core: delete the item `NiceMailer: <key> (IMAP)` in Keychain Access, then save the password again.

The released core is signed by the Developer ID Application identity (`make release`, [app-bundle.md](app-bundle.md), "Signature"), whose partition `teamid:<team>` plays the same role. When the Apple Development and Developer ID certificates belong to the same team, the passwords saved by development builds stay readable by the released core; otherwise they must be saved again once.

Limit, the one the specification accepts (`configuration.md`, section 7): a program running as the user can use the development certificate to sign itself as the core. The protection is aimed at agents and edits of the file, not at malware on the Mac.

### Test servers

The test Dovecot servers accept any user name with the password `nicemailer` (`docs/testing.md`):

```toml
[accounts.dovecot]
email = "alice@example.com"
[accounts.dovecot.imap]
host = "127.0.0.1"
port = 31993
username = "alice"
ca_file = "~/path/to/niceMailer/dev/dovecot/tls/ca.crt"
```

```sh
printf %s nicemailer | nicemailer account password dovecot
nicemailer imap probe dovecot    # what the server offers (docs/imap.md)
```

## Changes while the core runs

The core watches the directory of the files (FSEvents, crate `notify`), not the files themselves, because editors save by renaming a new file over the old one. If a file is a symbolic link (a dotfiles repository), the directory of its target is watched too. The events of one save are grouped (150 ms of quiet), and texts equal to the last ones read are ignored.

The two files are read **together** at each change of either: an account's overrides start from the `[sync]` of `config.toml`, and an invalid file is refused **with the other one**: applying `config.toml` while refusing `accounts.toml` would produce a state nobody wrote.

On each change:

| The file | The core | Events |
| --- | --- | --- |
| Valid, settings changed | Applies it whole; `audit_log`: `config` / `reload` with the changed keys | `ConfigChanged {keys}`, then `AccountChanged` for each account created, disabled, enabled or whose settings changed, then `AccountError` for each new error |
| Valid, same settings (a comment changed) | Nothing | None |
| Invalid (syntax, type, value out of range, missing key, unreadable) | Refuses both files **whole**: the previous settings stay | `ConfigError {file, line, key, message}`, once per problem |
| Valid again after a refusal | Applies it | `ConfigChanged` (empty `keys` if the settings are those already applied) |

`file` is `config.toml` or `accounts.toml`; `line` starts at 1, in that file; `key` is dotted (`accounts.perso.imap.port`); `line` and `key` are `null` when they do not apply (an unreadable file). Every problem, warnings included, names its file the same way, and prints as `accounts.toml, line 5: accounts.perso.imap.port: must be between 1 and 65535`.

At start, the files are created if missing, then read before the core accepts clients. An invalid file does not stop it: it runs with the defaults and no account, and the refusal is in `config_status`. The first reading is not recorded in `audit_log`: there is nothing to compare it with, so edits made while the core was stopped are not traced (the user's Git history has them).

### `config_status`

```json
{"path": "/Users/me/.config/nicemailer/config.toml", "exists": true,
 "accounts_path": "/Users/me/.config/nicemailer/accounts.toml", "accounts_exists": true,
 "error": {"file": "config.toml", "line": 12, "key": "sync.poll_interval_secs", "message": "must be between 30 and 86400"},
 "warnings": [{"file": "config.toml", "line": 4, "key": "core.turbo", "message": "unknown key, ignored"}],
 "ui": {"title_bar": "hidden", "auto_confirm_calls": false},
 "mcp": {"message_chars": 8000, "response_chars": 40000},
 "as_of": {"epoch": "e_…", "seq": 7}}
```

What `ConfigError` announces is also available here, as the rule of `evenements.md` requires: a client that connects after the event still sees the refusal.

### Unknown keys

A key the core does not know is a warning, not an error, so a file written by a newer NiceMailer stays readable by an older one. The rest of the file applies. A newer format (`version = 2`, in either file: each has its own `version`) is refused.

## The settings

They are all in `config.toml`; `nicemailer config defaults` prints them all. Ranges are checked; a value outside is an error on its line.

| Key | Default | Range | Used today |
| --- | --- | --- | --- |
| `core.background_sorting` | true | | no |
| `core.idle_exit_secs` | 60 | 1 to 86400 | no |
| `core.update_timeout_secs` | 900 | 60 to 86400 | no |
| `sync.poll_interval_secs` | 300 | 30 to 86400 | yes, when a worker starts ([sync.md](sync.md)) |
| `sync.max_connections` | 2 | 1 to 2 | yes, when a worker starts: connections per account ([sync.md](sync.md)) |
| `config_sync.push_delay_secs` | 5 | 0 to 3600 | no |
| `events.replay_buffer` | 10000 | 100 to 1000000 | at start |
| `events.client_queue` | 1000 | 10 to 100000 | at start |
| `import.threshold_total` | 100 | 0 to 1000000 | no (phase 7) |
| `import.threshold_unseen` | 50 | 0 to 1000000 | no (phase 7) |
| `import.unseen_default_age_days` | 30 | 0 to 36500 | no (phase 7) |
| `import.batch_size` | 1000 | 1 to 10000 | no (phase 7) |
| `raw.dir` | `<data_dir>/raw.noindex` | absolute or `~/` | no (phase 3) |
| `raw.zstd_level` | 9 | 1 to 19 | no (phase 3) |
| `raw.fanout` | 1 | 0 to 2 | no (phase 3) |
| `raw.gc_grace_secs` | 86400 | 0 to 30 days | no (phase 8) |
| `raw.verify_on_read` | true | | no (phase 3) |
| `ui.view_threshold_ms` | 2000 | 0 to 60000 | no |
| `ui.title_bar` | `hidden` | `hidden`, `shown` | yes: the app applies it at once (`geek-ui.md`, "Window") |
| `ui.auto_confirm_calls` | false | | yes: the app applies it at once; it presses the confirmation macOS shows before a call (`geek-ui.md`, "Calling a number") |
| `audit.retention_days` | 365 | 1 to 36500 | no |
| `compose.undo_send_seconds` | 5 | 0 to 30 | yes: the delay before a mail leaves ([sending.md](sending.md)) |
| `ai.enabled` | false | | not yet: allows mail to be sent to an AI provider, for every account ([ai.md](ai.md)) |
| `ai.provider` | `claude-code` | `claude-code`, `anthropic-api` | not yet ([ai.md](ai.md), "Providers") |
| `ai.model` | `claude-sonnet-5-5` | a model id, no space | not yet; a prompt may name its own |
| `ai.claude_path` | searched | absolute or `~/` | not yet: the `claude` binary ([ai.md](ai.md), "The `claude` binary") |
| `ai.timeout_secs` | 120 | 5 to 600 | not yet |
| `ai.max_parallel` | 2 | 1 to 8 | not yet |
| `ai.batch_size` | 20 | 1 to 100 | not yet: conversations per call when several are processed |
| `ai.max_folder_items` | 500 | 1 to 5000 | not yet: most conversations sent when a prompt takes the whole folder |
| `ai.history_days` | 30 | 1 to 3650 | not yet: days before the texts of the run history are purged |
| `mcp.message_chars` | 8000 | 1000 to 100000 | yes: characters of one message when an AI agent reads a conversation ([mcp.md](mcp.md)); read by `nicemailer-mcp` at each call |
| `mcp.response_chars` | 40000 | 4000 to 200000 | yes: characters of the text of one answer to an AI agent ([mcp.md](mcp.md)) |

The `[mcp]` keys only bound the length of the answers to an AI agent: what an agent may do is never a key of this file, which an agent able to write files could change ([mcp.md](mcp.md), "Rights").

"At start": the value is read when the core starts; a change is accepted and announced, and applies at the next start. Every key is validated today, so a file valid now stays valid when the key starts being used.

## Reads and writes of the database

Also part of this phase: the queries of the contract (`list_threads`, `get_thread`, `search`, `list_folders`, `list_history`, `list_tags`, `list_groups`, `list_contacts`, `list_accounts`, `locate_message`, `export_message`, `unsubscribe`'s lookup) run on a pool of six read-only connections (`nm-store`, `pool.rs`), and only commands take the single writing connection. The synchronization workers read through a pool of three of their own, so that their long reads during an import never take the connections the app's lists need. `prepare_draft`, which opens the reply screen, is one of these queries. In WAL mode a reader does not wait for a writer: a synchronization writing a batch of mail will not make the app wait for its list. Each query runs in one read transaction, so all its statements see the same state of the database.

The requests of one connection no longer run one after the other (`serve`, `crates/nm-daemon/src/main.rs`): the reads (`runs_in_parallel`) each run in a task of their own; the other requests keep the order in which the client sent them, in one task per connection; `hello` and `subscribe` are answered as they are read. At most 64 requests of a connection are in flight; beyond, the core stops reading it until one ends. Responses may therefore come back out of order, matched by `id`. The app has a single connection: before, its list reloads queued in front of what the user waited for.

What is slow is written to `core.log`, with the local time: a request over 500 ms, with how much of it was spent waiting for the database (a free connection of the read pool, the write lock, a thread to run on) and how much running (`09:12:03 slow request: get_thread took 9630 ms (9500 ms waiting for the database, 130 ms running)`: that request was stuck behind others, its own query is fast), a wait for the write lock or a hold of it over 250 ms, with who held it (`09:12:03 store lock: sync held 900 ms`, `store lock: request waited 1400 ms`). These lines are the numbers to look at when the app feels slow during a synchronization. `NICEMAILER_TEST_WRITE_DELAY_MS`, read by the core's tests only, makes every write hold the lock that much longer. Reader connections are `query_only`: a query that writes by mistake fails in the tests instead of racing the writer.

For a closer look, `NICEMAILER_SQL_LOG` in the core's environment logs every SQL statement of every connection (the writer and the read pools), with its parameters bound (`nm-store`, `sql_log.rs`): `1` (or `stderr`) writes to standard error, any other value is a file path, appended to. The log carries subjects, addresses and search words, and slows every query: a debugging tool, never on by default.

## Code

| Where | What |
| --- | --- |
| `crates/nm-engine/src/config.rs` | The settings, the reader of both files (files and lines of errors, unknown keys, overrides), `changed_keys`, `defaults_file`, `config_template`, `accounts_template` |
| `crates/nm-engine/src/config_write.rs` | `create_missing` (the files created at start), and the account commands' writes to `accounts.toml` |
| `crates/nm-engine/src/watch.rs` | The watch of the files |
| `crates/nm-engine/src/secrets.rs` | `Secrets`, `check` and `imap_password` (the binding to the server), `MemorySecrets` for tests |
| `crates/nm-engine/src/keychain.rs` | The Keychain: items, `never_prompt` |
| `scripts/sign-core.sh`, `scripts/dev-cert.sh` | The signature of `make core` (Apple Development, else the self-signed fallback) |
| `scripts/check-signature.sh`, `scripts/make-release.sh` | The checks of a signed bundle, and the signed and notarized release ([app-bundle.md](app-bundle.md), "Releases") |
| `crates/nm-engine/src/lib.rs` | `Engine`: reload, refusals, accounts and their errors, events |
| `crates/nm-store/src/accounts.rs` | `sync_config_accounts`, `list_account_rows`, `set_account_order`, `record_config_reload` |
| `crates/nm-store/src/pool.rs` | `ReadPool` |
| `crates/nm-daemon/src/main.rs` | Creating and reading at start, `reload_config`, `list_accounts`, `config_status`, `with_reader` |

## Tests

- `crates/nm-engine/tests/config.rs`: the example of the specification, a password in the file refused, accounts in `config.toml` refused, every problem naming its file, a valid `config.toml` refused with an invalid `accounts.toml`, each file's `version`, `accounts.toml` next to `config.toml`, the created `config.toml` giving the defaults with every key commented out (and valid with any line uncommented), the created `accounts.toml` with no account and a valid example, defaults per security, lines of syntax, type and range errors, duplicate keys, unknown keys (general, account, server), required keys, account keys, `security = "none"`, newer format, overrides, identities in both forms, `~/` paths, passwords hidden, `changed_keys`, and the defaults file parsing back to the defaults with every key present.
- `crates/nm-engine/src/config_write.rs` (unit tests): missing files created from the templates in a missing directory, nothing created twice, an existing file (empty, invalid) never touched, only the missing one of the two created, a dangling symbolic link left alone, an account added after the comments of the created `accounts.toml`.
- `crates/nm-engine/tests/engine.rs`: no file; invalid file at start then later (previous settings kept, one `ConfigError`, cleared when fixed, even back to the same text); unreadable file; same text twice; unknown key; reload recorded in `audit_log`; account removed then restored with its data and id; archive account clash; a missing password, then saved; a new IMAP host or user refused until the password is saved again; the login checks the binding again; OAuth2; an edit of either file applied with the right keys; accounts left in `config.toml` refused.
- `crates/nm-daemon/tests/config_rpc.rs`: end to end, the files created at start (equal to the templates, `config_status` reporting both), an existing file kept as it is, problems with their file, reloads of either file.
- `crates/nm-engine/tests/keychain.rs`, on a throwaway keychain file (never the login keychain, never a dialog): a password saved and read back with its server (UTF-8, spaces); saving again replaces it; the item as Keychain Access shows it; an item saved by another program gives its server but not its password; a locked keychain refuses without a dialog and still gives the server.
- `crates/nm-engine/tests/watch.rs`: an edit, a save by rename (twice), a file created in a missing directory, the target of a symbolic link, other files ignored, both files watched.
- `crates/nm-store/tests/accounts.rs`: a new account goes last, a reorder moves the accounts and the folder list (the picker's order), audit lines and announced changes, the same order twice changes nothing, unknown and repeated ids refused, a disabled account keeps its place.
- `crates/nm-daemon/tests/parallel_rpc.rs`, on the real binary with a slow write: a read sent after a write on the same connection answered first; the writes of a connection answered and applied in their order; `hello`, the refusal before it, and a notification without answer, in the order read.
- `crates/nm-store/tests/pool.rs`: a read during a write held 2 s returns at once and sees the committed state; one snapshot per read; readers cannot write; a panic gives the connection back; six threads on three connections; wrong schema version.
- `crates/nm-daemon/tests/config_rpc.rs`, end to end on the real binary: no file; accounts listed, then an invalid file, its fix, an account removed and restored, an archive account placed last then moved first with `set_account_order`, seen through events, `config_status`, `list_accounts`, `list_folders` and `list_history`; an invalid file at start; `set_password` refused to an agent and for an unknown account, then saved in the core's keychain, the account's error cleared, refused again when the file changes server, traced without the password.
