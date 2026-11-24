# Creating, renaming and removing an IMAP account

How an IMAP account is created with its initial import (from the CLI, or from the app's Preferences: [In the app](#in-the-app)), or only looked at first and organized later ([Looking first](#looking-first)), its folder roles changed, its key renamed, or the account deleted. This is phase 7 of `specifications/plan-imap.md`; the rules are in `specifications/import-initial.md`, `specifications/dossiers.md` ("A la création du compte") and `specifications/configuration.md` (sections 5 and 6). The core does it all (`crates/nm-daemon/src/setup.rs`, `crates/nm-engine/src/setup.rs` and `config_write.rs`, `crates/nm-store/src/setup.rs`); the CLI asks and shows.

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

1. **Test connection** (`test_account`): the server's capabilities and folders, the role proposed for each (special-use attribute, then the usual names in English and French: the rules of `nicemailer imap probe`), the plan of the roles (`roles`: the folder each role will get, and whether the server has it; the command prints it, `(created)` after a folder to create), the Inbox's counts and `UIDNEXT`, and the number of people written to (distinct recipients of Sent, the account's own address excluded). Nothing is written anywhere.
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
3. **IMAP**: the server given with `--host`, else the provider's first server NiceMailer can log in to: with a password, or with OAuth2 when the server is in the core's compiled table and an application id is in effect (below). A provider that only takes OAuth2 (Outlook.com, Microsoft 365) without an application id is reported as not usable yet.
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
| `auth_refused` | a server of the IMAP server's organization refused the IMAP password (an app password needed for SMTP, say) | not at once: the next server is tried. If none works, this one is kept (`refused` in the report) and the command says to save its password with `nicemailer account password <key> --smtp` |
| `no_login` | the server answered but offers no login: a relay, not a server to send from | no |
| `failed` | no connection, TLS refused, a login method NiceMailer does not speak (OAuth2 only): the reason is printed | no |

`test_smtp` takes an optional `smtp_password`: the password the user gave for the SMTP server itself. It is then tried on every server, whatever its organization (it is meant for SMTP, so sending it there is what the user asked), and the outcomes are `logged_in` or `auth_refused`. The app uses it to check the password before the account is created (below); `add_account` saves it in the Keychain with the account (`smtp_password`, item `smtp:<key>`, bound to the SMTP server), as `nicemailer account password <key> --smtp` would.

Nothing is sent: the connection ends with `QUIT`. When no server works and none refused the password, the account is created without `[smtp]` and the command says it cannot send until one is added; meanwhile its mails are answered from another account that can ([sending.md](sending.md), "Choosing the sender").

### The archiving on the server

The worker sends the `initial_archive` action before anything else, and the Inbox is not synchronized until it ends: each old mail is fetched once, straight into Archive, instead of 18,000 headers in the Inbox then 18,000 moves (`import-initial.md`, section 6). It searches the Inbox below the boundary, not flagged, read before one date or unread before the other (`BEFORE` is the internal date), then moves by batches of `import.batch_size` (1,000) with `UID MOVE`.

- **Replayable**: interrupted (the core stopped, the connection cut), it searches again and finds only what is left. Tested by killing the core after the first batch (`NICEMAILER_TEST_CRASH_DURING_ARCHIVE`, tests only).
- **Temporary failure**: tried again by the spool's rules ([spool.md](spool.md)).
- **Definitive failure** (no Archive folder, say): the action is `reverted` with the server's answer, and nothing local is to undo. `nicemailer sync status` shows "initial archiving: reverted (…)".
- **Journal**: one `initial_archive` row on the account, with the criteria and the number of mails moved; not one per mail.

The mails already in the mailbox are synchronized without screening (below the boundary); the addresses met then are marked `imported_from = 'imap_initial'`. After the import, every new mail is screened ([screening.md](screening.md)): a sender of the history without decision goes to Quarantine with their next mail.

## OAuth2 sign-in (Microsoft 365 and Outlook.com)

Microsoft cuts the password authentication of its IMAP and SMTP servers: OAuth2 is the only way in. `discover_account` recognizes those accounts — an address of `outlook.com`, `hotmail.com`, `live.com`, `msn.com` and their national variants, or a custom domain whose MX is under `mail.protection.outlook.com` (Microsoft 365) — and answers `auth: "oauth2"`, `oauth_provider: "microsoft"`, with the servers of the core's compiled table (`outlook.office365.com`, `smtp.office365.com`, `smtp-mail.outlook.com`). The full flow is in `specifications/oauth2.md`.

**From the CLI**, no password is asked when the discovery says `oauth2`: the command calls `oauth_begin`, prints the authorization URL, opens it with `open`, and waits for the `OAuthFinished` event, printing « Waiting for sign-in in your browser… ». With `--no-browser` (SSH: the redirection arrives on the loopback of the Mac that runs the core, open the URL in a browser of that Mac), the URL is printed without being opened. Ctrl-C sends `oauth_cancel`. The finished flow's id then goes as `oauth: {flow}` in `test_account`, `count_archive`, `test_smtp` and `add_account`, in place of the password, and `add_account` saves its tokens in the Keychain.

**To sign an existing account in again** (`oauth_reauth_needed`, after a password change or a revocation, say):

```sh
nicemailer account login <key> [--no-browser]
```

The same browser flow for the account; on a password account the core points to `nicemailer account password`. Once the tokens are stored, the core starts the pace of the account's logins again, as after a new password (a pause learnt from the old token ends), and only then sends `OAuthFinished` with `ok: true`; if the Keychain refuses the tokens, `OAuthFinished` comes with `ok: false` and the `keychain` reason.

An account gets `oauth_reauth_needed` when it has no token yet, and also when Microsoft refuses its refresh token while it syncs (`invalid_grant`, `interaction_required`: a password change, a revocation, 90 days without use). The worker then stops trying, announces the reason (`AccountError`) and `list_accounts` gives it until the account is signed in again; another explicit refusal of the provider (admin consent needed, conditional access) is given the same way with its own reason. A token endpoint that does not answer (network, `5xx`) is never a reason to sign in again: the account, or the mail being sent, is tried again later. A token refused by the IMAP server just after a renewal means IMAP is disabled for the mailbox (`imap_disabled_for_mailbox`), never a lock for too many logins.

The access token of a flow goes only to the servers of the core's compiled table, in TLS: `test_account`, `count_archive` and `add_account` with `oauth: {flow}` refuse any other IMAP server (`oauth2_unknown_server`) before connecting, and `test_smtp` tries the token only on the SMTP servers of the same provider. The SMTP servers proposed for Microsoft are on port 587 with STARTTLS, the port Microsoft documents for XOAUTH2.

**From the app**, the servers are looked for as soon as the address field loses the focus (Tab, or a click elsewhere), not only at « Add »: for such an address, the password field of the add-account form is replaced at once by a « Sign in with Microsoft » button, before any password is typed, with the sentence that the sign-in happens in the browser and NiceMailer never sees the password (one typed meanwhile is not sent). The button opens the URL, shows « En attente de la connexion dans le navigateur… » with Cancel and Reopen the page, and at `OAuthFinished` the form says « Connecté avec Microsoft. » (Cancel and Reopen gone: the flow's tokens wait in the core for the adding) and the adding resumes where it stopped. Two sign-ins for two new addresses at once (the app and the CLI) do not cancel each other; a new one for the same address cancels the previous one. The page of an OAuth2 account (`auth: "oauth2"` in `list_accounts`) has a « Connexion Microsoft » section with a « Se reconnecter » button in place of the password section, whatever its error: signing in again mends a refused refresh token as well as a Keychain item the core cannot read.

**The application id** is compiled into the binary at build time (`NICEMAILER_MICROSOFT_CLIENT_ID`, required by `make release`; the build script refuses a malformed value), so the distributed app needs no configuration: the first install signs in with no setting. The id is public, not a secret — a public client sends it in every authorization URL — and no client secret is compiled in. In a development build without the variable, the compiled id is a zero GUID and `oauth_begin` refuses cleanly; `oauth.microsoft_client_id` in `config.toml` then replaces it (the value must be a non-zero GUID: an invalid one refuses the file like any out-of-range value, [configuration.md](configuration.md), "The settings").

**In the Keychain**, the refresh token is stored in place of the IMAP password, in the same `imap:<key>` item (its data is a JSON record with the binding, provider, client id, tenant, user id and address), bound to the server and user name of the file as a password is. The access token never touches the disk: it lives in the core's memory and is renewed before it expires. Removing the account deletes the item; Microsoft offers a public client no way to revoke a refresh token, so the authorization stays visible and revocable on the « Applications » page of the Microsoft account (or « Mes applications » for a work account), and the confirmation of `nicemailer account remove` says so.

## Looking first

An account can be added **only to look at it** (`read_only`, [sync.md](sync.md), "Read-only accounts"): its mail is downloaded and shown, and nothing changes on its server, no folder created, no mail moved, flagged, marked read or sent; new mail is screened on paper only (`audit_log` says where each would have gone). The people written to are still accepted, a decision of the database the dry screening follows. It is the gentle way in: a new user sees their mail in NiceMailer first, and organizes the account when they like what they see.

Organizing it later runs the steps the creation skipped, as if the account were created then (`activate_account`, `crates/nm-daemon/src/setup.rs`):

```sh
nicemailer account activate perso
# --role todo=Projets    --no-archive    --no-correspondents    --yes
```

1. **What it would do** (`prepare_activation`): the report of `test_account`, the password taken from the Keychain, nothing written. The command prints the plan of the folders and the import screen (the default dates; changing them is for a creation for now).
2. **`activate_account`**, in this order: the folders created on the server; the roles; the Inbox's boundary set **again** to the `UIDNEXT` just read, so that the mail that came while the account was only looked at is not screened either, it is mail already there (`import-initial.md`, section 1); the correspondents; the archiving queued; then `read_only` removed from `[accounts.<key>]` (only that key: the rest of the table, and its comments, stay), which restarts the worker writable.

The archiving moves the old mail on the server, as for a creation. What changes is that this mail is already in the database: the next pass sees it leave the Inbox and arrive in Archive in the same pass, and keeps each row (a move, [sync.md](sync.md), "Moves made elsewhere"), so nothing is downloaded again but the headers of Archive's new UIDs, one fetch per batch. That is the cost of looking first: one `UID MOVE` per batch and a pass over the moved mail, against a creation that archives before the first synchronization.

A failure before the file is written leaves the account read-only: the folders created stay, and are found when it is tried again. A writable account is refused (`-32602`), and so is an agent (`1007`).

## In the app

The app menu's **Settings…** (⌘,, "Réglages…" in French) opens the Preferences (`apps/NiceMailer/Sources/NiceMailer/Preferences/`). Their **Accounts** tab lists every account in display order (the order of the account picker `@` and of the folder column), with what is wrong with it (disabled, cannot connect, archive of an mbox import) or that it is only looked at. An account selected shows its address, name, key and state; changing its key or servers, or removing it, is still done with the CLI.

**Order.** A drag in the list, or the ↑ ↓ buttons under it, moves the account selected. The list shows the new order at once and sends all of it with `set_account_order`; if the core refuses, the list is read again from the core.

**Adding an account.** The **+** button (and the tab itself, when there is no account yet) shows a form of three fields:

- the address;
- the password of the mailbox;
- the name shown in "From", proposed from the accounts already set: the first, in display order, that has one (a name typed is never replaced).

"Add Account" then runs the steps of `nicemailer account add`, without asking anything else while the steps succeed. Each step's result is kept: pressing the button again after filling a field asked for goes on from where it stopped, and changing a field forgets only what depended on it (a new address starts again from scratch, a new password tries the IMAP server again). Other fields are shown, below the three, only when a step shows they are needed, each with why:

| What happened | What is asked |
| --- | --- |
| `discover_account` found no IMAP server: the domain and its MX are not in the database, the DNS lookup failed, the domain has no MX, or the provider takes only OAuth2 | the IMAP server (host, port, SSL/TLS or STARTTLS, user name), prefilled with `imap.<domain>` on port 993; the servers are then looked for again with it, for its SMTP guesses |
| `test_account` could not connect (`data.reason` = `imap_unreachable`) | the IMAP server, prefilled with the one tried |
| `test_account` refused the password (`data.reason` = `imap_auth_refused`) | nothing new: the error points at the password |
| the SMTP server refused the IMAP password (`refused`), or belongs to another organization (`reachable`) | the password of the SMTP server, then tried on that server alone with `smtp_password`; refused again, it is asked again. Or **Add the account without sending for now**, its password to save later |
| no SMTP server to try, or none answered | the SMTP server, prefilled with the first tried (or `smtp.<domain>` on port 465), or **Add the account without sending for now** |
| the servers work (always, once) | **On the server**: what the account will change there, before anything is written. A sentence names the folders to create; below, the folder of each workflow role, from `roles` of `test_account`, each one a menu of the server's folders (and of the new folder proposed, when the server has none for the role), as `dossiers.md` asks ("A la création du compte IMAP dans niceMailer"). A folder chosen for two roles is refused before anything is done. Or **Only look for now: change nothing on the server**: the account is added `read_only`, without folder nor archiving ([Looking first](#looking-first)) |
| the Inbox is above the thresholds (`offer_archive`), and the account is organized now | the import screen ([import-initial.md](../specifications/import-initial.md), section 3), between the sentence and the folders: the two archiving boxes and the correspondents' box, checked, with the exact counts of `test_account` for the default dates (read and unread mails do not overlap, so unchecking one subtracts its count without counting again). The dates are the defaults; changing them is for the CLI for now |

A section that appears is scrolled into view, and the cursor goes to the field asked for. The key is the one `discover_account` proposes, never asked. Under the threshold the correspondents are accepted and nothing is archived, as the CLI does. "Add Account" on "On the server" adds the account as shown; a new password (a new test) shows it again. Once added, the form empties (the name of "From" stays for the next account), says what happens next (the mail being downloaded, the old mail being archived, or nothing changed on the server), and the list shows the account.

**App passwords.** Gmail, iCloud and Fastmail refuse the password of their website to a mail app: they ask for an app password, created in the security settings of the account. As soon as the address is typed, the form says so under the password, with a link to the step-by-step guide of the website (`https://nicemailer.org/docs/app-passwords/`, in French when the app is: `AppPasswordProvider`, `AppPasswords.swift`). The provider is known by the domain of the address (`gmail.com`, `googlemail.com`, `icloud.com`, `me.com`, `mac.com`, `fastmail.com`, `fastmail.fm`), or, once `discover_account` answered, by the provider it found (`googlemail.com`, `me.com`, `messagingengine` in the providers' database), which also catches a domain of one's own hosted at Google Workspace or Fastmail. When such a server refuses the password (`imap_auth_refused`), the error says that first, then the server's words.

**A new password.** The page of an IMAP account has a **Password** section: the password typed there is saved by `set_password`, as `nicemailer account password <key>` does (`AccountPassword`). It is for a password changed on the provider's side, or a new app password (changing the password of a Google account revokes its app passwords). The core saves it without trying it: the account's worker connects with it at once, and a refusal comes back as the account's error, shown at the top of the page. The SMTP password saved apart (`--smtp`) is still given with the CLI.

**Organizing an account only looked at.** Its page says what looking means and has **Organize…**: the same "On the server" (without the choice to look only), with the import screen when the Inbox is above the thresholds, read by `prepare_activation`; **Organize** applies it (`activate_account`), **Cancel** goes back. Once done, the page says what happens next and the list no longer says "Only looked at".

Only the app, the TUI and the CLI may call these methods (error `1007`): an agent cannot add an account.

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
- writes no default value: `username` only when it is not the address. An OAuth2 account (added from the app or the CLI after a browser sign-in) is written with `auth = "oauth2"` and the servers of the provider's table; declared by hand, such an account waits for its first sign-in (`nicemailer account login <key>`), like a password account without a password ([configuration.md](configuration.md), "Passwords").

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

Stops the account's worker and waits for it, removes its tables from the file, deletes its data (folders, mails, threads, actions; the raw files stay until the garbage collector of phase 8) with a `remove` row in `audit_log`, which outlives it, and deletes its Keychain item. The mail on the server is not touched. For an OAuth2 account, the confirmation also says that the authorization stays revocable on the « Applications » page of the Microsoft account (a public client cannot revoke a refresh token itself).

## Accounts declared by hand

An account written in `accounts.toml` by hand (or by an agent: its password can still only be entered in the app or the CLI) skips the creation steps. Its first synchronization then gives each folder the proposed role and sets the Inbox's boundary to its `UIDNEXT`, without archiving or correspondents ([sync.md](sync.md)).

## Tests

| Test | What it checks |
| --- | --- |
| `crates/nm-engine/src/providers.rs` (unit) | the key: the domain's name for its first account, the whole address for a second one or when the name is another account's key (a disabled account too); the key made from the address; a listed domain needs no MX; our providers win over Thunderbird's; a domain hosted by Google found by its MX, one hosted by Infomaniak by the `mx` of our file; never above the organizational domain (`co.uk`); a failed DNS lookup reported; an OAuth2-only provider refused; the SMTP guesses (`imap` → `smtp`, 465 then 587, each once); the provider's SMTP tried first only for its own IMAP server; placeholders; the checks of our files; the built-in database reads, and knows Gmail and Infomaniak |
| `crates/nm-proto/src/dns.rs` (unit) | MX sorted by preference, the null MX dropped; with the network (`--ignored`): Gmail's MX, a domain that does not exist |
| `crates/nm-cli/src/main.rs` (unit) | the rule of the key, each step of the discovery and each SMTP outcome put in words |
| `scripts/tests/test_update_autoconfig.py` (`make test`) | the conversion: servers kept, POP3-only providers and clear text dropped and counted, placeholders, a missing SMTP user name, invalid values, a domain claimed twice, entities and files outside `ispdb/` never read; the committed file well formed |
| `crates/nm-engine/src/discover.rs` (unit) | `test_smtp` against a scripted SMTP server on the loopback: the IMAP password tried on the same organization, refused (`auth_refused`, `refused`) and the next server tried, a later server taking it; the SMTP password given tried on another organization, accepted then refused; a server offering only OAuth2 failing without a password tried; nothing listening |
| `crates/nm-engine/src/config_write.rs` (unit) | the `[smtp]` table written without default; adding keeps comments and order, comments at the end stay above, into an empty file, no default written, keys checked; renaming keeps the tables in place; removing takes only the account; `read_only` cleared from one account only, a writable one unchanged; atomic write, a hand edit kept, an invalid result writes nothing, no temporary file left |
| `crates/nm-engine/src/setup.rs` (unit) | dates and the archiving's search |
| `crates/nm-store/tests/setup.rs` | the account row, roles moved, correspondents (decided, removed, own address), the archiving's life (claimed, retried, done with its journal row, failed), the Inbox waiting for it, addresses of a first synchronization marked imported, rename and remove |
| `crates/nm-engine/tests/keychain.rs` | a password removed |
| `crates/nm-daemon/tests/setup_rpc.rs` (`make imap-up`, `make imap-test`) | against Mailpit: SMTP servers tried in order (no answer, then the password accepted, the third not tried), a wrong password, the password not sent to another organization, the guesses for a given host, the key of a first then of a second account of a domain, `[smtp]` written by `add_account`, an agent refused. Against Dovecot: a small Inbox left as it is, the people written to accepted, a mail arriving during the screen screened, the file written with its comment, an imported sender's next mail in Quarantine; old mail archived on the server exactly as counted, flagged mail left; an archiving interrupted after a batch resumed without loss or duplicate; an account renamed with its data and password, then removed, the server untouched; an agent refused; the plan of the roles (found, to create), an account only looked at, a mail arriving meanwhile screened dry, `prepare_activation` changing nothing on the server, then `activate_account`: folders created, old mail archived as counted with its rows kept, the mail of the meantime left unscreened, `read_only` gone from the file, the next mail screened for real, a writable account and an agent refused. Against Mailpit too: a wrong SMTP password `auth_refused` and `refused`, the SMTP password given apart accepted, a wrong IMAP password with `data.reason` = `imap_auth_refused`. Without the servers: an unreachable IMAP server with `data.reason` = `imap_unreachable`, an SMTP password without SMTP server or empty refused, an OAuth2 flow refused (`oauth2_unknown_server`) on an IMAP server outside the table or without TLS by `test_account`, `count_archive` and `add_account` |
| `apps/NiceMailer/Tests/NiceMailerTests/AccountSetupTests.swift` (`make swift-test`) | the app's flow against a fake core: three fields enough when the servers are found (what `add_account` receives); the name of "From" proposed and never replacing a typed one; an address or password missing said without a call; the IMAP server asked when the MX does not tell it, for an OAuth2-only provider, after a connection failure, a wrong port refused; a refused password pointing at it and only the IMAP test run again; the SMTP password asked when the server refuses the IMAP one or is of another organization, refused again, then saved; an SMTP server asked when none answers, or the account added without one; "On the server" always shown before `add_account`, the roles sent; the plan's folders found and to create, the options of a role, a folder for two roles refused before anything; only looking sending `read_only` without roles nor archiving, even for a big Inbox; a new password showing the plan again; the import screen with the plan, its boxes changing what is archived; a new address starting again; a refused creation keeping the form and the plan; an account only looked at organized (`prepare_activation`, then `activate_account` with the plan's choices), a refusal keeping the plan, a server out of reach said; the order of the list after a drag, the ↑ ↓ moves sending the whole order, a refused move reading the core's order again |
| `apps/NiceMailer/UITests/preferences.uiscript` (`make ui-test`) | the Preferences played with real keys and clicks, the creation methods answered without any server (`UIScriptSetupCore`): the accounts in order, moved up and down; a domain unknown asking for its IMAP server, then added; a wrong password; an SMTP server refusing the mailbox password, a wrong SMTP password, the right one; "On the server" with the folders to create, before anything is added; the import screen with the unread mail kept; no SMTP server and the account added without sending; an account added only to look, then organized from its page; the keys back to the main window once closed |
