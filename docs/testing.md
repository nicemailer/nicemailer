# Testing

| Command | What runs | Needs |
| --- | --- | --- |
| `make dev-cert` | Fallback only: a self-signed certificate for `make core` when there is no Apple Development one; its passwords survive no rebuild (`configuration.md`, "Passwords") | |
| `make test` | the editor built and its tests (`make editor-test`), Rust tests (`cargo test`), Python script tests and Swift unit tests (`swift test`) | Node, nothing running |
| `make editor-test` | the editor of a mail's text: type check and Vitest ([editor.md](editor.md)) | Node |
| `make swift-test` | Swift unit tests only | |
| `make ui-test` | scripted UI tests of the app | builds the core; starts its own cores on temporary data |
| `make imap-up`, `make imap-down` | start and stop the test IMAP servers (Dovecot, see below) | OrbStack or Docker |
| `make imap-test` | the tests that talk to the test IMAP servers: the IMAP client on both servers, `probe_account` end to end, a large body streamed from the server into the raw store ([ingestion.md](ingestion.md)), the synchronization end to end against a second connection playing the phone, with network cuts ([sync.md](sync.md)), the spool: changes reaching the server, refusals, two cores, a crash after a MOVE ([spool.md](spool.md)), the screening of new mail ([screening.md](screening.md)), the sending of replies through the test SMTP servers ([sending.md](sending.md)), and the creation of an account with its initial import ([accounts.md](accounts.md)) | `make imap-up` |
| `make volume-test` | the volume test: a generated mailbox of `NM_VOLUME` mails (100,000) synchronized from nothing by the real core; durations, memory, latency of the lists during the sync ([sync.md](sync.md), "Volume") | `make imap-up` |
| `make bench` | times every thread list (folders, Everything, search, each filter) through the socket on 100,000 generated conversations, or on `BENCH_DATA_DIR` | starts its own core |

## Rust

Unit tests live next to the code (`#[cfg(test)]`), integration tests in `crates/*/tests/`. `crates/nm-daemon/tests/import_rpc.rs` starts the real core binary and talks to it over its socket. The harness (`tests/common/mod.rs`) gives the core `--config <temporary dir>/config.toml` (so its `accounts.toml` is in that directory too; the core creates both at its start), and `scripts/ui-test.sh` and `scripts/bench.sh` set settings files of their own, in a directory of their own, too: a test must never read the settings of the machine, whose accounts the core would start to synchronize.

Nor its Keychain: the harness creates a throwaway keychain file in the test's directory (`security create-keychain`, which does not add it to the user's search list) and gives it to the core with `--keychain`; `crates/nm-engine/tests/keychain.rs` does the same. No test ever reads or writes the login keychain, and the core never shows a Keychain dialog (see [configuration.md](configuration.md), "Passwords").

The ingestion path ([ingestion.md](ingestion.md)) has its own tests: the order of arrival of messages (every one of 120 orders gives the same threads), a batch rolled back, a full disk simulated by `RawStore::with_disk_space`, and a 150 MB message whose heap use is measured by a counting allocator (`crates/nm-store/tests/big_message.rs`). Hashing, MIME parsing and compression are compiled optimized even in debug builds (`[profile.dev.package.*]` in `Cargo.toml`), so that these tests take about a second instead of half a minute.

Speed is tested by the plans of the queries rather than by timing them, which would be slow and flaky. `plan_tests` in `crates/nm-store/src/queries.rs` runs `EXPLAIN QUERY PLAN` on every list query (each filter, sort, scope and page) on an empty database, that is without planner statistics, and fails if one reads a whole folder, scans `message`, runs a subquery for each row, or, without criterion, does anything but walk its own index of `thread`. `crates/nm-store/tests/conversation_folder.rs` checks the folder of a conversation (`dossiers.md`, "Dossier d'une conversation"): a reply listed in the folder and in Sent, a new mail bringing the conversation back to the Inbox, rejected and technical mails, the unified view of several accounts, a folder that changes role, Conversation date… against Mail date…, the folder counts. `crates/nm-store/tests/bodies.rs` checks the second file, `nicemailer_body.db` ([backup.md](backup.md)): bodies and search there and from a reader, a message deleted taking its body and search row along, a body file deleted then rebuilt from the raw files, rows left by a crash between the two commits cleaned at opening, a body file left by a deleted database emptied. `refresh_all_threads_matches_refresh_thread` (`crates/nm-store/src/lib.rs`) checks that the set-based SQL of the migration gives exactly what `refresh_thread` gives. `crates/nm-store/tests/statistics.rs` checks that statistics computed before an account existed do not make the planner take it for empty (`ANALYZE` samples are removed at opening and after an import). `crates/nm-store/tests/everything.rs` checks that Everything and All Mail list exactly what their folders listed together list, page after page, on a generated mailbox. `crates/nm-store/tests/migrations.rs` checks that a version 1 database (`tests/fixtures/schema_v1.sql`) migrated by the core has exactly the schema of a fresh one, and repaired data.

## Test IMAP servers

`dev/dovecot/compose.yaml` describes two Dovecot 2.4 servers, started by `make imap-up` (OrbStack or Docker) and stopped by `make imap-down`:

| Server | Ports (127.0.0.1) | Announces |
| --- | --- | --- |
| `full` | 31143 (STARTTLS), 31993 (TLS) | every extension of Dovecot: CONDSTORE, QRESYNC, MOVE, ESEARCH, SPECIAL-USE… |
| `minimal` | 32143 (STARTTLS), 32993 (TLS) | `IMAP4rev1 UIDPLUS IDLE LITERAL+` only: a plain server without the sync extensions, to test the fallbacks |

Two Mailpit servers play SMTP for the sending tests: `smtp` (STARTTLS on 31587, any password, recipients outside `@example.com` and `@ok.example` refused with 550) and `smtps` (TLS on 31465, user `nicemailer`, password `smtp-secret`). What they received is read on their API (38025 and 38026). They use the same test certificate.

The core only uses what a server announces, so the minimal server exercises the fallbacks (no MOVE, no CONDSTORE, no SPECIAL-USE) that many real servers need. Any user name logs in with the password `nicemailer`, and a new user starts with an empty mailbox: a test picks a random user and needs no setup or cleanup. Mail is kept in memory and lost when the servers stop.

The servers present a certificate for `localhost` and `127.0.0.1`, signed by a test authority that `dev/dovecot/make-tls.sh` creates on the first `make imap-up` (`dev/dovecot/tls/`, not committed). Clients trust `dev/dovecot/tls/ca.crt` and check the certificate as they would with a real server: the core checks it with macOS's own rules, which refuse a server certificate valid for more than 825 days or without the "server authentication" usage, so the script makes one that follows them, and makes it again when an older one does not or is about to expire. `make imap-up` recreates the servers each time, so they pick up a new certificate (and lose their mail).

### IMAP client tests

The IMAP client of `nm-proto` ([imap.md](imap.md)) is tested at two levels:

- `crates/nm-proto/tests/imap_script.rs`, run by `make test`: the client against a scripted server over an in-memory pipe. Each test is the exact conversation, what the client must send and what the server answers, with the tags the client uses (`N1`, `N2`…). They cover what a real server rarely does on demand: a connection cut in the middle of a literal, a response that cannot be read, a response over the size limit, silence until the time limit, a response to an unknown tag, `BYE`, a literal refused before it is sent, plus the details of the commands (quoting, literals with and without `LITERAL+`, QRESYNC at `SELECT`, `COPYUID`, `MODIFIED`, bodies in pieces, IDLE keep-alives);
- `crates/nm-proto/tests/dovecot.rs`, run by `make imap-test`: every command against both Dovecot servers, so each fallback is exercised by the minimal one (moves without `MOVE`, no CONDSTORE, no LIST-STATUS). One test puts a TCP relay between the client and the server, which cuts the connection in the middle of a message body: the session must close with a fatal error, give nothing of the incomplete body, and a new session must fetch it whole.

To look at a server by hand:

```sh
openssl s_client -connect 127.0.0.1:32993 -CAfile dev/dovecot/tls/ca.crt -quiet
a LOGIN alice nicemailer
b LIST "" "*"
```

## Swift unit tests

`apps/NiceMailer/Tests/NiceMailerTests`, with Swift Testing:

- `SelectionTests`: the selection rules of the mail list (click, ⌘-click, ⇧-click, `J` / `K`), pure functions of `Selection`;
- `GeekControllerTests`: every key and menu of the geek theme, against `FakeCore`, a core that answers from Swift dictionaries and records the requests (`CoreCalling` is the protocol the app talks to; the real implementation is the socket client);
- `FormatTests`: list dates, fixed column widths, the bundled font, and translations (every `L("…")` of the sources is in the catalog, and every catalog string has a French translation).
- `LocalizationTests`: the rules of [localization.md](localization.md): placeholders and plural forms of the catalog, no dead key, no literal text in a view, dates in several locales.

Tests use their own `UserDefaults` domain and export directory: they never touch the user's settings or `~/Downloads`.

## Scripted UI tests

`apps/NiceMailer/UITests`: scenarios played by the app itself, with simulated keys and clicks, on fresh data. See `apps/NiceMailer/UITests/README.md` for the script language, the state fields and the list of scenarios. The app stays in the background and off screen, so the tests can run while someone uses the machine.

## Rule

Every UI feature comes with both a unit test and a UI scenario, written in the same change (see CLAUDE.md, "Testing the app").
