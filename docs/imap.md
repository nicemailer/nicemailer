# The IMAP client

The core talks to mail servers through its own IMAP client, in `crates/nm-proto/src/imap/`. It connects, logs in, and runs the commands the synchronization needs; it knows nothing of the database, the accounts or the Keychain. `nm-engine` decides what to ask and what to do with the answers. This page describes the client and `nicemailer imap probe`; the synchronization built on it is [sync.md](sync.md). The plan of the IMAP back end, phase by phase, is `specifications/plan-imap.md`.

## Layers

| File | Role |
| --- | --- |
| `session.rs` | `Session`: one connection, the commands, IDLE, the sorting of responses |
| `types.rs` | What the commands return: plain owned types (`Fetched`, `Selected`, `ListEntry`, `UidSet`…) |
| `encode.rs` | Writing commands: atoms, quoted strings and literals |
| `convert.rs` | From the types of `imap-codec` to ours |
| `transport.rs` | TCP, TLS, STARTTLS, certificates |
| `utf7.rs` | Modified UTF-7, the encoding of mailbox names |

Responses are parsed by [`imap-codec`](https://crates.io/crates/imap-codec), pinned to `2.0.0-alpha.10`, the version tried in phase 0. It is an alpha, so only `convert.rs` and `session.rs` see its types: if a later version changes them, those two files change and the rest of the core does not notice. Commands are written by `encode.rs` rather than by `imap-codec`, whose types cannot express some that NiceMailer sends (`LIST … RETURN (SPECIAL-USE)`, `SELECT … (QRESYNC …)`). Writing them is the easy half of IMAP; what must be right is the quoting of values that come from outside (user names, mailbox names, search values), and each of them goes through one function that picks an atom, a quoted string or a literal, so that no value can end the line early and inject a second command.

## Connecting

`imap.security` of the account in `accounts.toml` ([configuration.md](configuration.md)) chooses how:

| `security` | Connection |
| --- | --- |
| `tls` | TLS from the first byte (port 993) |
| `starttls` | Clear text, then `STARTTLS` before anything else (port 143). If the server refuses, the connection fails: there is never a fallback to clear text |
| `none` | Clear text, allowed only to `localhost`, `127.0.0.1` or `::1`. The client checks it again, on top of the check of `accounts.toml` |

**Certificates are checked by macOS** (crate `rustls-platform-verifier`, which asks the Security framework): the same trust as Mail and Safari, including authorities the user added and the system's revocation checks. `imap.ca_file` adds one authority (a PEM file) on top of those, for a self-hosted server or the test servers. There is no way to turn the check off. macOS also applies its own rules to server certificates, even from an authority the user trusts: valid 825 days at most, with the "server authentication" usage. A certificate that breaks them fails with "not standards compliant".

**STARTTLS.** The capabilities announced before the upgrade are forgotten, and read again over TLS: anyone on the network could have changed them. Bytes the server sent right after its `OK` to `STARTTLS`, before the TLS handshake, are refused: they would otherwise be read as if protected (the "STARTTLS injection" of CVE-2011-0411).

**Time limits** (`Timeouts`): 30 seconds to open the connection, TLS included; 90 seconds of silence while a response is expected, and per piece of 64 KiB written. The limit applies to each read and each write, not to a whole command: a large download that keeps flowing never times out, a server that stops answering does.

## Logging in

- `AUTHENTICATE PLAIN` when the server announces `AUTH=PLAIN`, in one line with `SASL-IR`, else after the server's `+`. Any password, UTF-8 included, travels base64-encoded without quoting.
- `LOGIN` otherwise, with the user name and the password quoted or sent as literals as needed.
- A refusal is `ImapError::AuthFailed` with the server's text. The password is only ever written to the connection: never into an error, a log, or a `Debug` output.
- **The capabilities after login are the ones that count.** Many servers announce their extensions only once logged in (Infomaniak: `IMAP4rev1 UIDPLUS IDLE LITERAL+ QUOTA AUTH=PLAIN AUTH=LOGIN` before, `CONDSTORE QRESYNC MOVE SPECIAL-USE…` after). The client takes them from the `[CAPABILITY …]` code of the `OK`, or sends `CAPABILITY`. The earlier ones remain available (`capabilities_before_login`) but only served to choose how to log in.

Then:
- `ID` tells the server which client this is, when it announces `ID`;
- `enable_sync` turns on `QRESYNC` (which implies `CONDSTORE`), or `CONDSTORE` alone, when announced. On a server with `CONDSTORE` but without `ENABLE`, the first `SELECT` turns it on with its `(CONDSTORE)` parameter (RFC 7162, 3.1).

`UTF8=ACCEPT` is not turned on: Infomaniak does not announce it, and keeping one encoding of mailbox names everywhere removes a source of mismatch.

## Commands

| Method | IMAP | Without the extension |
| --- | --- | --- |
| `capability`, `noop`, `logout` | `CAPABILITY`, `NOOP`, `LOGOUT` | |
| `namespace` | `NAMESPACE` | `Unsupported` |
| `list` | `LIST "" "*"`, with `RETURN (SPECIAL-USE)` when `SPECIAL-USE` and `LIST-EXTENDED` are announced | a plain `LIST` (Dovecot gives the special uses in it anyway) |
| `list_with_status` | `LIST … RETURN (STATUS (…))`: every folder and its counters in one answer | `Unsupported`: one `STATUS` per folder |
| `status` | `STATUS (MESSAGES UIDNEXT UIDVALIDITY UNSEEN [HIGHESTMODSEQ])` | `HIGHESTMODSEQ` asked only with `CONDSTORE` |
| `create` | `CREATE` | |
| `select`, `examine` | `SELECT`, `EXAMINE`, with `(QRESYNC (uidvalidity modseq known-uids))` when QRESYNC is on and the caller knows the mailbox | `(CONDSTORE)`, or nothing |
| `uid_fetch`, `uid_fetch_vanished` | `UID FETCH` of UID, flags, internal date, size, modseq, `BODY.PEEK[HEADER]`, `BODY.PEEK[]` whole or a piece; `(CHANGEDSINCE m VANISHED)` | modseq and `CHANGEDSINCE` need `CONDSTORE`, `VANISHED` needs `QRESYNC` |
| `fetch_body` | the body of one message, given to a callback as it arrives (see below) | |
| `uid_search` | `UID SEARCH` of `ALL`, `UID`, `SINCE`, `HEADER`, `UNSEEN`, `DELETED`, and their combination; `CHARSET UTF-8` when a value needs it | |
| `uid_store` | `UID STORE ±FLAGS[.SILENT]`, with `(UNCHANGEDSINCE m)` | `UNCHANGEDSINCE` needs `CONDSTORE` |
| `uid_copy` | `UID COPY`, with the new UIDs from `COPYUID` | without `UIDPLUS`, no new UIDs |
| `uid_move` | `UID MOVE` | `UID COPY`, `\Deleted`, then `UID EXPUNGE` of those UIDs only (`UIDPLUS`). Without `UIDPLUS` either, the messages stay flagged `\Deleted` (`MoveResult::expunged` is false): a plain `EXPUNGE` would also expunge messages someone else flagged |
| `uid_expunge` | `UID EXPUNGE` | `Unsupported` |
| `append` | `APPEND` with flags and internal date, the new UID from `APPENDUID` | without `UIDPLUS`, no UID |
| `idle` | `IDLE` … `DONE` | `Unsupported` |

Literals go at once as `{n+}` when the server announces `LITERAL+` (or `LITERAL-` up to 4096 bytes), else after the server's `+`. If the server refuses a literal instead (an `APPEND` too large), the message is not sent and the session goes on.

A set of UIDs is written as merged ranges (`1:5,7,9:12`). A set too long for one command line (4,000 characters, well under what servers accept) is split into several commands, whose results are joined.

## What the server reports on its own

A server can report at any time that mail arrived (`EXISTS`), was expunged (`EXPUNGE`, `VANISHED`) or changed (`FETCH`), and that something must be shown to the user (`[ALERT]`). These responses are **never dropped**: they go to a queue of `Notice`s, each with the mailbox selected when it arrived, which `take_unsolicited` empties. A command takes from its responses what answers it, and puts the rest in the queue: during a `UID FETCH`, a `FETCH` with a UID is a result (the server includes the UID in every `FETCH` caused by a UID command; a flag change from another client is the current state too), a `FETCH` without UID is a notice.

## Large messages

A response is kept in memory until decoded, so responses are bounded:

| Limit | Value | Why |
| --- | --- | --- |
| Largest response read | 16 MiB (`MAX_RESPONSE`) | anything larger is refused without being kept: memory stays bounded whatever the server sends |
| Body fetched whole | up to 4 MiB (`WHOLE_BODY_MAX`) | most messages: one command |
| Pieces of a larger body | 1 MiB (`BODY_CHUNK`), `BODY.PEEK[]<offset.length>` | a 150 MB message never takes more than about 1 MiB |

`fetch_body` uses the size from `RFC822.SIZE` to choose, but stops on the first short piece rather than trusting the size, which some servers count differently. It gives each piece to a callback, the streaming writer of the raw store (`RawWriter::write`, [ingestion.md](ingestion.md)), and returns `None` if the message disappeared meanwhile. The writer can check the total against `RFC822.SIZE` at the end (`RawWriter::finish`); the synchronization does not, since Exchange Online counts it differently from the bytes it sends, and stores a body whose size differs ([sync.md](sync.md), "Bodies").

## Failures

`ImapError::is_fatal` sorts them:

- **Fatal**, the session closes and every later call fails at once with `Closed`: network error, time limit, TLS, `BYE`, and anything that means client and server no longer agree on where they are in the conversation (a response to an unknown tag, an unexpected `+`). Going on could read the bytes of a literal as a response. The caller connects again.
- **Not fatal**, the session goes on: `NO`, `BAD`, `AuthFailed`, a missing extension (`Unsupported`), a call out of state (`State`: a fetch without a selected mailbox, an invalid flag name; nothing is sent), and `Malformed`.

`Malformed` is a response that could not be decoded, or larger than 16 MiB. The framing of `imap-codec` found where it ends, so the next response is read correctly; the command it belongs to finishes, then fails with `Malformed`, since its result may lack something. One case is repaired rather than refused: RFC 3501 forbids NUL bytes in a literal and `imap-codec` refuses them, but a stored message can hold one, and a server that sends it as is would make that message unreadable for good. The client then reads it as 0x80, which is what Dovecot sends in its place. A second one: a `UIDVALIDITY 0`, forbidden by RFC 3501 (a non-zero number). Dovecot gives it for a folder it is still creating, when several sessions of a new user open at once (reproduced: 1 session in 15). The client reads the response without it: in `STATUS` (and `LIST-STATUS`), the folder has no UIDVALIDITY and the synchronization opens it to learn the real one; in the answer to `EXAMINE`, the pass skips the folder until the next pass. Only `* STATUS` and `* OK [UIDVALIDITY 0]` are touched, never a literal, so a mail's bytes are never changed.

## Mailbox names

On the wire, names are in modified UTF-7 (RFC 3501, 5.1.3): `Envoyés` is `Envoy&AOk-s`. `MailboxName` keeps both forms: `raw`, the bytes the server sent, which are what the client sends back (never a name decoded and encoded again), and `name`, decoded for display. A name that is not valid modified UTF-7 is shown as sent rather than hidden. `INBOX` is the one name whose case does not matter.

## IDLE

`idle(max, stop)` sends `IDLE` and waits until the server reports something (`Changes`), `max` passes (`Timeout`; at most 25 minutes, since RFC 2177 lets servers end IDLE after 30), or the future `stop` completes (`Stopped`). It then sends `DONE` and keeps what came in between. Keep-alive lines such as Dovecot's `* OK Still here` do not end it. There is no time limit on the server's silence during IDLE, since silence is its purpose.

## `nicemailer imap probe <key>`

Connects to the server of an account through the core (method `probe_account`), which alone can read the password, and shows what the server offers:

```text
$ nicemailer imap probe dovecot
Server 127.0.0.1:31993 (tls): connected in 49 ms, logged in in 35 ms, folders listed in 4 ms
Before login: AUTH=PLAIN ENABLE ID IDLE IMAP4REV1 IMAP4REV2 LITERAL+ LOGIN-REFERRALS SASL-IR
After login:  BINARY … CONDSTORE … LIST-STATUS … MOVE … QRESYNC … SPECIAL-USE … UIDPLUS …
Enabled:      CONDSTORE QRESYNC
Namespace:    prefix "", separator /

folder  proposed role             messages   unread  attributes
INBOX   inbox                            0        0
Drafts  drafts (special-use)             0        0  \Drafts
Junk    junk (special-use)               0        0  \Junk
Sent    sent (special-use)               0        0  \Sent
Trash   deleted (special-use)            0        0  \Trash
```

`--json` prints the report as the core sends it. The probe changes nothing: it selects no mailbox (so nothing can be marked read), writes nothing in the database, and `ENABLE` lasts for its session only. The counters come from `LIST-STATUS` when the server has it, else from one `STATUS` per folder, up to 300 folders.

**Proposed roles** (`nm-engine`, `folders.rs`; `dossiers.md`, "A la création du compte"): what the account creation panel will propose in phase 7, for the user to confirm.

1. `INBOX` is the Inbox.
2. A special-use attribute gives the role: `\Sent`, `\Drafts`, `\Trash` (Deleted), `\Junk`, `\Archive`. A Drafts folder the phone already uses is taken over as is. `\All` and `\Flagged` are views of other folders, never a role.
3. Failing that, the name, in English or French (`Sent Items`, `Éléments envoyés`, `Corbeille`, `Courrier indésirable`…), or the name NiceMailer gives its own folders (`Todo`, `Waiting`, `Newsletter`, `Paperwork`, `Quarantine`, `Banned`). Only a folder at the top of the personal namespace counts (`INBOX/Sent` too, a common layout): `Clients/Sent` is a folder of the user.

Each role goes to one folder at most: the attribute wins over the name, then the first name in the list, then the first folder.

**Errors** (codes of `nm-api`):

| Code | When |
| --- | --- |
| `1001` NOT_FOUND | no such account in `accounts.toml` |
| `1008` ACCOUNT_NOT_READY | no password saved, the server changed since it was saved, or `auth = "oauth2"`: the message says what to do |
| `1006` SECRETS | the Keychain refused the password |
| `1009` SERVER | the server could not be reached, refused the login, or failed: the message holds its answer |

The password is read on a blocking thread, after checking that it was saved for the server the file names now (`secrets::imap_password`); no lock of the core is held while the Keychain and the server answer.

## Tests

See [testing.md](testing.md), "IMAP client tests": a scripted server for what real servers rarely do on demand (cuts, unreadable responses, silence), both Dovecot servers for every command and fallback, and a relay that cuts the connection in the middle of a body.

## Not yet

- Commands are sent one at a time; pipelining several `FETCH` would cut the round trips of a first synchronization (phase 8, if measurements ask for it).
- `COMPRESS=DEFLATE`, `UTF8=ACCEPT`, `BINARY`, `ESEARCH`: not used.
- OAuth2 (`XOAUTH2`, `OAUTHBEARER`): `plan-imap.md`, section 6.
