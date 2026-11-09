# Creating, renaming and removing an IMAP account

How an IMAP account is created with its initial import, its folder roles changed, its key renamed, or the account deleted. This is phase 7 of `specifications/plan-imap.md`; the rules are in `specifications/import-initial.md`, `specifications/dossiers.md` ("A la création du compte") and `specifications/configuration.md` (sections 5 and 6). The core does it all (`crates/nm-daemon/src/setup.rs`, `crates/nm-engine/src/setup.rs` and `config_write.rs`, `crates/nm-store/src/setup.rs`); the CLI asks and shows.

These commands carry a password or delete mail: only the app, the TUI and the CLI may call them, never an agent (error `1007`).

## `nicemailer account add`

```sh
nicemailer account add --email pierre.dupont@toto-mail.com
# key: toto-mail (below); servers found from the address (below)
nicemailer account add perso --email me@example.fr --host imap.example.fr
# with --host: --port 993 --security tls --username me --ca-file …
# --display-name "Me"    --no-smtp (do not look for an SMTP server)
# --role sent="Éléments envoyés"      the folder of a role (repeat it)
# --archive-read-before 2026-11-07    --archive-unread-before 2026-10-07
# --no-archive    --no-correspondents    --yes (no confirmation)
# --read-only     never change anything on this server (docs/sync.md, "Read-only accounts")
```

The **key** is optional (next section). The command prints the one it proposes and why, and stops before asking anything if an account has that key already.

`--host` is optional too: without it the servers are found from the address (next section). Every step is printed, before the password is asked.

The password is asked without echo, or read from the standard input (`printf %s "$PW" | nicemailer account add … --yes`). Then:

1. **Test connection** (`test_account`): the server's capabilities and folders, the role proposed for each (special-use attribute, then the usual names in English and French: the rules of `nicemailer imap probe`), the Inbox's counts and `UIDNEXT`, and the number of people written to (distinct recipients of Sent, the account's own address excluded). Nothing is written anywhere.
2. **The import screen**, in text: what would be archived, what stays, how many correspondents go straight to the Inbox. The archiving is offered when the Inbox holds more than `import.threshold_total` (100) mails or `import.threshold_unseen` (50) unread ones; by default, every read mail and the unread ones older than `import.unseen_default_age_days` (30) days. Flagged mails always stay. The counts are exact (`count_archive`: the same `UID SEARCH` the archiving will run).
3. **SMTP** (`test_smtp`): the SMTP servers found are tried in order, and the first that works is kept (below). Not for a read-only account, nor with `--no-smtp`.
4. **Confirmation** (unless `--yes`), then `add_account`, with the SMTP server kept written to `[accounts.<key>.smtp]`.

`add_account`, in this order, so that the worker started by the file finds everything ready:

| Step | What |
| --- | --- |
| Folders | Each workflow role gets a folder: the one given (`--role`), else the proposal, else a new folder under the role's English name (`Todo`, `Quarantine`…, in the personal namespace), created on the server |
| Account | Its row, the roles (`audit_log`: `role`, with the client's source), and the Inbox's screening boundary (`uid_screening_start`): the `UIDNEXT` read by the test connection, so that a mail arriving while the user reads the screen is screened (`import-initial.md`, section 5) |
| Correspondents | The recipients of the mails of Sent without decision, and who never had one, get the decision `inbox` (`created_by = 'core'`, reason `sent_correspondent`), unless `--no-correspondents`; unknown ones are created with `imported_from = 'imap_initial'` |
| Archiving | One spool action `initial_archive` on the account, with the criteria, if dates were given |
| Password | In the Keychain, bound to this server (host and user) |
| SMTP password | The IMAP one by default, when the SMTP server is of the same provider; else saved apart with `nicemailer account password <key> --smtp` ([sending.md](sending.md)), which the command says to do |
| File | `[accounts.<key>]` added to `accounts.toml` (below), then applied at once: the worker starts |

### The key

Without a key, `discover_account` proposes one, from the accounts the core has (those of the database, disabled and archive accounts included, and those of `accounts.toml`):

- **the first account of a domain is named after the domain**: its name without the public suffix nor the subdomains (the label before the suffix, by the public suffix list): `pierre.dupont@toto-mail.com` gives `toto-mail`, `me@mail.example.co.uk` gives `example`;
- **otherwise, the whole address**: lower case, every character other than an ASCII letter or digit replaced by `_`, several in a row by one, at most 64 characters: `pierre_dupont_toto_mail_com`. This is when an account of the same domain exists (whatever its key), or when the domain's name is the key of another account (`toto-mail`, taken by an account of `toto-mail.fr`).

"The same domain" is the domain of the address itself: `toto-mail.com` and `mail.toto-mail.com` are two. The command says which rule applied:

```text
  Account key: toto-mail (the name of the domain: no account of toto-mail.com yet; give a key after `add` to choose another)
  Account key: pierre_dupont_toto_mail_com (the whole address: toto-mail.com has an account already, toto-mail; …)
```

A key given after `add` is used as it is.

### Finding the servers

`discover_account` (no password, nothing written, no mail server contacted) looks in the **database of mail providers**, built into the core ([data/providers/README.md](../data/providers/README.md)):

- `data/providers/thunderbird.json`: [Thunderbird's ISP database](https://github.com/thunderbird/autoconfig) (MPL 2.0), converted by `make autoconfig` (`scripts/update-autoconfig.py`): IMAP and SMTP servers only, encrypted only (what was dropped, and why, is printed); about 120 providers, 800 domains. Run it now and then, and commit the result;
- `data/providers/local/*.toml`: our own providers, by hand, such as Infomaniak, which Thunderbird does not list. They win over Thunderbird's for the domains they name.

In this order:

1. **The domain of the address** (`gmail.com`, `ik.me`): if a provider lists it, its servers are used.
2. Otherwise, **the MX servers of the domain** (DNS, the system's resolvers, 3 s per resolver): for each MX by preference, its name then each parent down to its organizational domain is looked up among the providers' domains, and the `mx` domains of our files. `anthropic.com`, whose MX is `aspmx.l.google.com`, is found under `google.com`, which Thunderbird's Google entry lists for this purpose; a domain whose MX is `mta-gw.infomaniak.ch` is found under `infomaniak.ch`, in our Infomaniak file. A failed lookup is reported, not hidden.
3. **IMAP**: the server given with `--host`, else the provider's first server that takes a password. A provider that only takes OAuth2 (Outlook.com) is reported as not supported yet.
4. **SMTP servers to try**, in order:
   - the provider's, when its IMAP server is the one used (with `--host` naming another server, they may not be the right ones: not tried);
   - for a server given with `--host` (or a provider without SMTP server), guesses: the host with `imap` replaced by `smtp` in its first label holding it (`imap.example.fr` gives `smtp.example.fr`), then the host itself; each on port 465 (TLS) then 587 (STARTTLS). Port 25 is not tried: it is for servers relaying between them.

The output of the command, for an address of a domain hosted by Google:

```text
Finding the servers of contact@anthropic.com
  Account key: anthropic (the name of the domain: no account of anthropic.com yet; give a key after `add` to choose another)
  anthropic.com is not in the database of mail providers.
  MX of anthropic.com: aspmx.l.google.com, alt1.aspmx.l.google.com, …
  aspmx.l.google.com is under google.com, listed by Google Mail (Thunderbird's database): the domain is hosted there.
  IMAP: imap.gmail.com:993 (tls), from Google Mail.
  SMTP servers to try, in order:
    smtp.gmail.com:465 (tls), from Google Mail
```

Then, once the IMAP server accepted the password, `test_smtp` tries the SMTP servers one by one, 8 s at most to connect, and stops at the first that works. Each attempt is printed:

| Outcome | When | Kept |
| --- | --- | --- |
| `logged_in` | a server of the IMAP server's organization (same host, or same organizational domain: `imap.gmail.com` and `smtp.gmail.com`) accepted the IMAP password | yes; it sends with the IMAP password |
| `reachable` | a server of **another** organization answered and offers a login. The IMAP password is not sent there: the sender would not use it there either ([sending.md](sending.md), "Password") | yes; the command says to save its password with `nicemailer account password <key> --smtp` |
| `no_login` | the server answered but offers no login: a relay, not a server to send from | no |
| `failed` | no connection, TLS refused, login refused: the reason is printed | no |

Nothing is sent: the connection ends with `QUIT`. When no server works, the account is created without `[smtp]` and the command says it cannot send until one is added; meanwhile its mails are answered from another account that can ([sending.md](sending.md), "Choosing the sender").

### The archiving on the server

The worker sends the `initial_archive` action before anything else, and the Inbox is not synchronized until it ends: each old mail is fetched once, straight into Archive, instead of 18,000 headers in the Inbox then 18,000 moves (`import-initial.md`, section 6). It searches the Inbox below the boundary, not flagged, read before one date or unread before the other (`BEFORE` is the internal date), then moves by batches of `import.batch_size` (1,000) with `UID MOVE`.

- **Replayable**: interrupted (the core stopped, the connection cut), it searches again and finds only what is left. Tested by killing the core after the first batch (`NICEMAILER_TEST_CRASH_DURING_ARCHIVE`, tests only).
- **Temporary failure**: tried again by the spool's rules ([spool.md](spool.md)).
- **Definitive failure** (no Archive folder, say): the action is `reverted` with the server's answer, and nothing local is to undo. `nicemailer sync status` shows "initial archiving: reverted (…)".
- **Journal**: one `initial_archive` row on the account, with the criteria and the number of mails moved; not one per mail.

The mails already in the mailbox are synchronized without screening (below the boundary); the addresses met then are marked `imported_from = 'imap_initial'`. After the import, every new mail is screened ([screening.md](screening.md)): a sender of the history without decision goes to Quarantine with their next mail.

## Trying NiceMailer on a mailbox in production

To look at a real mailbox without any risk to it, in a data directory and settings files of their own, apart from the everyday ones (the core creates `~/nm-trial/config.toml` and `~/nm-trial/accounts.toml` at its start):

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

## Writing `accounts.toml`

The account commands write `accounts.toml`, never `config.toml` ([configuration.md](configuration.md)). The file belongs to the user (`configuration.md`, section 5), so the core changes it only at a client's request (it creates it, at its start, only when it is missing), and:

- edits it in place with `toml_edit`: comments, order and formatting stay, only the account's tables change. Comments at the end of the file stay above the new account: an account added to the created file goes after its commented example;
- reads it again right before the change, so that an edit made by hand meanwhile is kept;
- refuses a result the core would refuse (`config::parse_accounts`);
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

An account written in `accounts.toml` by hand (or by an agent: its password can still only be entered in the app or the CLI) skips the creation steps. Its first synchronization then gives each folder the proposed role and sets the Inbox's boundary to its `UIDNEXT`, without archiving or correspondents ([sync.md](sync.md)).

## Tests

| Test | What it checks |
| --- | --- |
| `crates/nm-engine/src/providers.rs` (unit) | the key: the domain's name for its first account, the whole address for a second one or when the name is another account's key (a disabled account too); the key made from the address; a listed domain needs no MX; our providers win over Thunderbird's; a domain hosted by Google found by its MX, one hosted by Infomaniak by the `mx` of our file; never above the organizational domain (`co.uk`); a failed DNS lookup reported; an OAuth2-only provider refused; the SMTP guesses (`imap` → `smtp`, 465 then 587, each once); the provider's SMTP tried first only for its own IMAP server; placeholders; the checks of our files; the built-in database reads, and knows Gmail and Infomaniak |
| `crates/nm-proto/src/dns.rs` (unit) | MX sorted by preference, the null MX dropped; with the network (`--ignored`): Gmail's MX, a domain that does not exist |
| `crates/nm-cli/src/main.rs` (unit) | the rule of the key, each step of the discovery and each SMTP outcome put in words |
| `scripts/tests/test_update_autoconfig.py` (`make test`) | the conversion: servers kept, POP3-only providers and clear text dropped and counted, placeholders, a missing SMTP user name, invalid values, a domain claimed twice, entities and files outside `ispdb/` never read; the committed file well formed |
| `crates/nm-engine/src/config_write.rs` (unit) | the `[smtp]` table written without default; adding keeps comments and order, comments at the end stay above, into an empty file, no default written, keys checked; renaming keeps the tables in place; removing takes only the account; atomic write, a hand edit kept, an invalid result writes nothing, no temporary file left |
| `crates/nm-engine/src/setup.rs` (unit) | dates and the archiving's search |
| `crates/nm-store/tests/setup.rs` | the account row, roles moved, correspondents (decided, removed, own address), the archiving's life (claimed, retried, done with its journal row, failed), the Inbox waiting for it, addresses of a first synchronization marked imported, rename and remove |
| `crates/nm-engine/tests/keychain.rs` | a password removed |
| `crates/nm-daemon/tests/setup_rpc.rs` (`make imap-up`, `make imap-test`) | against Mailpit: SMTP servers tried in order (no answer, then the password accepted, the third not tried), a wrong password, the password not sent to another organization, the guesses for a given host, the key of a first then of a second account of a domain, `[smtp]` written by `add_account`, an agent refused. Against Dovecot: a small Inbox left as it is, the people written to accepted, a mail arriving during the screen screened, the file written with its comment, an imported sender's next mail in Quarantine; old mail archived on the server exactly as counted, flagged mail left; an archiving interrupted after a batch resumed without loss or duplicate; an account renamed with its data and password, then removed, the server untouched; an agent refused |
