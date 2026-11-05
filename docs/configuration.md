# Settings and accounts: `config.toml`

The settings of NiceMailer and the accounts are in a TOML text file that the core reads at start and watches while it runs. The specification is `specifications/configuration.md`; this page describes what is implemented (phase 1 of `specifications/plan-imap.md`).

## Where the file is

`~/.config/nicemailer/config.toml`, or the file named by `NICEMAILER_CONFIG`. The core also takes `--config <file>`.

```sh
nicemailer config path        # the file the core reads
nicemailer config defaults    # a complete file, every key at its default, with comments
nicemailer config check       # validate the file without applying it (status 1 if refused)
nicemailer config check other.toml
```

A missing file is an empty file: the core starts with every default and no account. The file needs only the keys that differ from the defaults.

## Accounts

An account is a table `[accounts.<key>]`. The key (letters, digits, `-`, `_`, `.`; 64 at most) names it, and is what the database links its folders and mail to.

```toml
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

[accounts.perso.smtp]             # read and checked; sending is not implemented yet
host = "smtp.example.com"         # ports: 465 tls, 587 starttls, 25 none

[[accounts.perso.identities]]
email = "contact@example.com"
name = "My Name"

[accounts.perso.sync]             # overrides [sync] for this account
poll_interval_secs = 60
```

- **`security = "none"`** is refused unless the host is `localhost`, `127.0.0.1` or `::1`: elsewhere it would send the password in clear text.
- **`ca_file`** adds an authority to the system's, for a self-hosted server or the test servers (`dev/dovecot/tls/ca.crt`). There is no way to turn certificate checks off. Certificates are checked by macOS, with its own rules (at most 825 days, "server authentication" usage): see [imap.md](imap.md), "Connecting".
- **Per-account overrides.** `[accounts.<key>.sync]` and `[accounts.<key>.import]` start from the general `[sync]` and `[import]` and replace the keys they set. The other tables are global.

### What happens to the database

The core keeps the `account` table in line with the keys of the file (`nm-store`, `accounts.rs`), each time it reads it:

| In the file | In the database |
| --- | --- |
| A new key | A new account (`audit_log`: `account` / `create`) |
| A key that disappears | The account is **disabled**: `account.disabled_at` is set, nothing is deleted (`disable`) |
| A disabled key that comes back | The same account, same id, enabled again, with all its mail (`enable`) |
| A key equal to the name of an archive account | Ignored, with a warning: rename one of the two |

Disabling instead of deleting is the rule of the specification: going back to an older version of the file in Git, or taking the file of another Mac, must not erase gigabytes of cached mail. Renaming a key by hand therefore creates a new account and disables the old one; `nicemailer account rename` will do it while keeping the data (not implemented yet).

Archive accounts, created by an mbox import ([import-mbox.md](import-mbox.md)), are never in the file and never disabled.

Disabled accounts are still listed, and their mail is still shown (the user's choice); only their synchronization stops (when it exists).

### Order of the accounts

`account.position` orders the accounts: `list_accounts` lists them by position (then id), and so does `list_folders`, whose order the app's account picker (`@`) and sidebar follow. A new account, from the file or from an mbox import, goes after the others; accounts created by the same reading of the file come in the order of their keys. A disabled account keeps its place.

The order is a state of the interface, not an intention about the servers, so it is in the database, not in `config.toml`. It is changed by `set_account_order {account_ids: [...]}`: the accounts given come first, in this order, the others after them in their current order, and positions become 1, 2, 3… An unknown or repeated id is refused. Each account that moved gets an `audit_log` line (`reorder`, before and after), an `AccountChanged`, and a `FolderChanged` for each of its folders, which makes the app reload its folder list.

```sh
nicemailer account order pro perso    # pro first, then perso, then the others
```

### `list_accounts` and `nicemailer accounts`

```sh
nicemailer accounts
  1  perso        imap     me@example.com                 ok
  2  old          archive  old@example.com                ok
  3  pro          imap     -                              disabled (no longer in config.toml, mail kept)
```

`list_accounts` returns, for each account in display order: `id`, `key`, `kind` (`imap` or `archive`), `email`, `display_name`, `enabled`, `position`, and `error`, the reason it cannot connect if one is known. `error: null` does not mean connected: there is no connection yet in this phase.

## Passwords

Passwords are in the macOS Keychain, never in the file (`configuration.md`, section 2). A `password` key in `config.toml` is an error: the file is refused, and the message says how to save the password instead. It is refused rather than ignored, so that a password does not stay, in clear text, in a file the specification invites to keep in Git.

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

### Development: `make dev-cert`, once

macOS recognizes an application by its code signature. A Rust binary is signed by the linker with a signature of its own content, so every rebuild of the core would be a new application, refused by the items the previous build saved. Hence, in development:

```sh
make dev-cert    # once: creates the certificate "NiceMailer Dev" in the login keychain
make core        # builds, signs with it (identifier org.nicemailer.core), runs
```

Signed with the same certificate and identifier, every build is the same application for the Keychain. Measured on a throwaway keychain (`scripts/dev-cert.sh`, then two different builds signed by `scripts/sign-core.sh`): the second build reads the password saved by the first; an unsigned build, or one signed with another identifier, is refused. The certificate is self-signed and not marked as trusted, which would need an administrator password: `codesign` uses it anyway, and the Keychain only compares a signature with the one that saved the item.

A core run without signing (`cargo run`) works, but cannot read the passwords saved by `make core`; its accounts say so. A password saved before `make dev-cert` (by an unsigned build) cannot be read or replaced by the signed core: delete the item `NiceMailer: <key> (IMAP)` in Keychain Access, then save the password again.

The shipped core will be signed by the developer's Apple identity, which plays the same role.

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

The core watches the directory of the file (FSEvents, crate `notify`), not the file itself, because editors save by renaming a new file over the old one. If the file is a symbolic link (a dotfiles repository), the directory of its target is watched too. The events of one save are grouped (150 ms of quiet), and a text equal to the last one read is ignored.

On each change:

| The file | The core | Events |
| --- | --- | --- |
| Valid, settings changed | Applies it whole; `audit_log`: `config` / `reload` with the changed keys | `ConfigChanged {keys}`, then `AccountChanged` for each account created, disabled, enabled or whose settings changed, then `AccountError` for each new error |
| Valid, same settings (a comment changed) | Nothing | None |
| Invalid (syntax, type, value out of range, missing key, unreadable) | Refuses it **whole**: the previous settings stay | `ConfigError {line, key, message}`, once per problem |
| Valid again after a refusal | Applies it | `ConfigChanged` (empty `keys` if the settings are those already applied) |

`line` starts at 1; `key` is dotted (`accounts.perso.imap.port`); both are `null` when they do not apply (an unreadable file).

At start, the file is read before the core accepts clients. An invalid file does not stop it: it runs with the defaults and no account, and the refusal is in `config_status`. The first reading is not recorded in `audit_log`: there is nothing to compare it with, so edits made while the core was stopped are not traced (the user's Git history has them).

### `config_status`

```json
{"path": "/Users/me/.config/nicemailer/config.toml", "exists": true,
 "error": {"line": 12, "key": "sync.poll_interval_secs", "message": "must be between 30 and 86400"},
 "warnings": [{"line": 4, "key": "core.turbo", "message": "unknown key, ignored"}],
 "as_of": {"epoch": "e_…", "seq": 7}}
```

What `ConfigError` announces is also available here, as the rule of `evenements.md` requires: a client that connects after the event still sees the refusal.

### Unknown keys

A key the core does not know is a warning, not an error, so a file written by a newer NiceMailer stays readable by an older one. The rest of the file applies. A newer format (`version = 2`) is refused.

## The settings

`nicemailer config defaults` prints them all. Ranges are checked; a value outside is an error on its line.

| Key | Default | Range | Used today |
| --- | --- | --- | --- |
| `core.background_sorting` | true | | no |
| `core.idle_exit_secs` | 60 | 1 to 86400 | no |
| `core.update_timeout_secs` | 900 | 60 to 86400 | no |
| `sync.poll_interval_secs` | 300 | 30 to 86400 | no (phase 4) |
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
| `audit.retention_days` | 365 | 1 to 36500 | no |

"At start": the value is read when the core starts; a change is accepted and announced, and applies at the next start. Every key is validated today, so a file valid now stays valid when the key starts being used.

## Reads and writes of the database

Also part of this phase: the queries of the contract (`list_threads`, `get_thread`, `search`, `list_folders`, `list_history`, `list_tags`, `list_groups`, `list_contacts`, `list_accounts`, `locate_message`, `export_message`, `unsubscribe`'s lookup) run on a pool of four read-only connections (`nm-store`, `pool.rs`), and only commands take the single writing connection. In WAL mode a reader does not wait for a writer: a synchronization writing a batch of mail will not make the app wait for its list. Each query runs in one read transaction, so all its statements see the same state of the database. Reader connections are `query_only`: a query that writes by mistake fails in the tests instead of racing the writer.

## Code

| Where | What |
| --- | --- |
| `crates/nm-engine/src/config.rs` | The settings, the reader (lines of errors, unknown keys, overrides), `changed_keys`, `defaults_file` |
| `crates/nm-engine/src/watch.rs` | The watch of the file |
| `crates/nm-engine/src/secrets.rs` | `Secrets`, `check` and `imap_password` (the binding to the server), `MemorySecrets` for tests |
| `crates/nm-engine/src/keychain.rs` | The Keychain: items, `never_prompt` |
| `scripts/dev-cert.sh`, `scripts/sign-core.sh` | The development certificate, and the signature of `make core` |
| `crates/nm-engine/src/lib.rs` | `Engine`: reload, refusals, accounts and their errors, events |
| `crates/nm-store/src/accounts.rs` | `sync_config_accounts`, `list_account_rows`, `set_account_order`, `record_config_reload` |
| `crates/nm-store/src/pool.rs` | `ReadPool` |
| `crates/nm-daemon/src/main.rs` | Reading at start, `reload_config`, `list_accounts`, `config_status`, `with_reader` |

## Tests

- `crates/nm-engine/tests/config.rs`: the example of the specification, a password in the file refused, defaults per security, lines of syntax, type and range errors, duplicate keys, unknown keys (general, account, server), required keys, account keys, `security = "none"`, newer format, overrides, identities in both forms, `~/` paths, passwords hidden, `changed_keys`, and the defaults file parsing back to the defaults with every key present.
- `crates/nm-engine/tests/engine.rs`: no file; invalid file at start then later (previous settings kept, one `ConfigError`, cleared when fixed, even back to the same text); unreadable file; same text twice; unknown key; reload recorded in `audit_log`; account removed then restored with its data and id; archive account clash; a missing password, then saved; a new IMAP host or user refused until the password is saved again; the login checks the binding again; OAuth2.
- `crates/nm-engine/tests/keychain.rs`, on a throwaway keychain file (never the login keychain, never a dialog): a password saved and read back with its server (UTF-8, spaces); saving again replaces it; the item as Keychain Access shows it; an item saved by another program gives its server but not its password; a locked keychain refuses without a dialog and still gives the server.
- `crates/nm-engine/tests/watch.rs`: an edit, a save by rename (twice), a file created in a missing directory, the target of a symbolic link, other files ignored.
- `crates/nm-store/tests/accounts.rs`: a new account goes last, a reorder moves the accounts and the folder list (the picker's order), audit lines and announced changes, the same order twice changes nothing, unknown and repeated ids refused, a disabled account keeps its place.
- `crates/nm-store/tests/pool.rs`: a read during a write held 2 s returns at once and sees the committed state; one snapshot per read; readers cannot write; a panic gives the connection back; six threads on three connections; wrong schema version.
- `crates/nm-daemon/tests/config_rpc.rs`, end to end on the real binary: no file; accounts listed, then an invalid file, its fix, an account removed and restored, an archive account placed last then moved first with `set_account_order`, seen through events, `config_status`, `list_accounts`, `list_folders` and `list_history`; an invalid file at start; `set_password` refused to an agent and for an unknown account, then saved in the core's keychain, the account's error cleared, refused again when the file changes server, traced without the password.
