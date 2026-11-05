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
password = "…"                    # development builds only, see below

[accounts.perso.smtp]             # read and checked; sending is not implemented yet
host = "smtp.example.com"         # ports: 465 tls, 587 starttls, 25 none

[[accounts.perso.identities]]
email = "contact@example.com"
name = "My Name"

[accounts.perso.sync]             # overrides [sync] for this account
poll_interval_secs = 60
```

- **`security = "none"`** is refused unless the host is `localhost`, `127.0.0.1` or `::1`: elsewhere it would send the password in clear text.
- **`ca_file`** adds an authority to the system's, for a self-hosted server or the test servers (`dev/dovecot/tls/ca.crt`). There is no way to turn certificate checks off.
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

Disabled accounts are still listed, and their mail is still shown; only their synchronization stops (when it exists).

### `list_accounts` and `nicemailer accounts`

```sh
nicemailer accounts
  1  perso        imap     me@example.com                 ok
  2  old          archive  old@example.com                ok
  3  pro          imap     -                              disabled (no longer in config.toml, mail kept)
```

`list_accounts` returns, for each account: `id`, `key`, `kind` (`imap` or `archive`), `email`, `display_name`, `enabled`, and `error`, the reason it cannot connect if one is known. `error: null` does not mean connected: there is no connection yet in this phase.

## Passwords

The specification puts secrets in the Keychain only, entered in the app or the CLI. The Keychain arrives with `add_account` (phase 7). Until then, for development, a password can be written in the file, `password` in `[accounts.<key>.imap]`, on two conditions:

- **only a core built with the `dev-passwords` feature reads it.** `make core` builds it so; `cargo build` and the shipped core do not. A core without the feature ignores the key and says so in the account's error ("this build of the core does not read passwords from config.toml");
- **every reading of the file warns about it** (`config check`, `config_status`, the core's log).

Keep such a file out of Git: put the test accounts in a file of their own and point `NICEMAILER_CONFIG` at it, rather than in `~/.config/nicemailer/config.toml`, which the specification invites to version.

The password never leaves the core: its debug output shows `SecretString(***)`, and the list of changed keys (`ConfigChanged`, `audit_log`) only says that `accounts.<key>.imap.password` changed.

### A password belongs to its server

Every login goes through one function, `nm_engine::secrets::imap_password`. A secret is stored with the host and the user name it was entered for; if the file now names another host or user, the function refuses and the account gets the error "the password was entered for … on …, the file now says … on …: enter the password again to confirm the new server". Without this rule, anyone able to edit the file, an agent tricked by a mail included, could point the account at their own server and receive the password at the next login.

With the development passwords, the secret is in the same file as the host, so the check always passes: it starts protecting with the Keychain. It is tested with a fake store of secrets (`crates/nm-engine/tests/engine.rs`).

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

Also part of this phase: the queries of the contract (`list_threads`, `get_thread`, `search`, `list_folders`, `list_history`, `list_tags`, `list_groups`, `list_contacts`, `list_accounts`, `export_message`, `unsubscribe`'s lookup) run on a pool of four read-only connections (`nm-store`, `pool.rs`), and only commands take the single writing connection. In WAL mode a reader does not wait for a writer: a synchronization writing a batch of mail will not make the app wait for its list. Each query runs in one read transaction, so all its statements see the same state of the database. Reader connections are `query_only`: a query that writes by mistake fails in the tests instead of racing the writer.

## Code

| Where | What |
| --- | --- |
| `crates/nm-engine/src/config.rs` | The settings, the reader (lines of errors, unknown keys, overrides), `changed_keys`, `defaults_file` |
| `crates/nm-engine/src/watch.rs` | The watch of the file |
| `crates/nm-engine/src/secrets.rs` | `Secrets`, `imap_password` and the binding to the server, `ConfigPasswords`, `NoSecrets` |
| `crates/nm-engine/src/lib.rs` | `Engine`: reload, refusals, accounts and their errors, events |
| `crates/nm-store/src/accounts.rs` | `sync_config_accounts`, `list_account_rows`, `record_config_reload` |
| `crates/nm-store/src/pool.rs` | `ReadPool` |
| `crates/nm-daemon/src/main.rs` | Reading at start, `reload_config`, `list_accounts`, `config_status`, `with_reader` |

## Tests

- `crates/nm-engine/tests/config.rs`: the example of the specification, defaults per security, lines of syntax, type and range errors, duplicate keys, unknown keys (general, account, server), required keys, account keys, `security = "none"`, newer format, overrides, identities in both forms, `~/` paths, passwords hidden, `changed_keys`, and the defaults file parsing back to the defaults with every key present.
- `crates/nm-engine/tests/engine.rs`: no file; invalid file at start then later (previous settings kept, one `ConfigError`, cleared when fixed, even back to the same text); unreadable file; same text twice; unknown key; reload recorded in `audit_log`; account removed then restored with its data and id; archive account clash; a new IMAP host or user refused before confirmation; a build without `dev-passwords`; OAuth2.
- `crates/nm-engine/tests/watch.rs`: an edit, a save by rename (twice), a file created in a missing directory, the target of a symbolic link, other files ignored.
- `crates/nm-store/tests/pool.rs`: a read during a write held 2 s returns at once and sees the committed state; one snapshot per read; readers cannot write; a panic gives the connection back; six threads on three connections; wrong schema version.
- `crates/nm-daemon/tests/config_rpc.rs`, end to end on the real binary: no file; accounts listed, then an invalid file, its fix, an account removed and restored, seen through events, `config_status`, `list_accounts` and `list_history`; an invalid file at start. Run it with `--features dev-passwords` too, for the other case of the passwords.
