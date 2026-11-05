# Testing

| Command | What runs | Needs |
| --- | --- | --- |
| `make dev-cert` | Once: the certificate that signs the core of `make core`, so it keeps reading the passwords it saved in the Keychain across rebuilds | |
| `make test` | Rust tests (`cargo test`), Python script tests and Swift unit tests (`swift test`) | nothing running |
| `make swift-test` | Swift unit tests only | |
| `make ui-test` | scripted UI tests of the app | builds the core; starts its own cores on temporary data |
| `make imap-up`, `make imap-down` | start and stop the test IMAP servers (Dovecot, see below) | OrbStack or Docker |
| `make bench` | times every thread list (folders, Everything, search, each filter) through the socket on 100,000 generated conversations, or on `BENCH_DATA_DIR` | starts its own core |

## Rust

Unit tests live next to the code (`#[cfg(test)]`), integration tests in `crates/*/tests/`. `crates/nm-daemon/tests/import_rpc.rs` starts the real core binary and talks to it over its socket. The harness (`tests/common/mod.rs`) gives the core `--config <temporary dir>/config.toml`, and `scripts/ui-test.sh` and `scripts/bench.sh` set a settings file of their own too: a test must never read the settings of the machine, whose accounts the core would start to synchronize.

Nor its Keychain: the harness creates a throwaway keychain file in the test's directory (`security create-keychain`, which does not add it to the user's search list) and gives it to the core with `--keychain`; `crates/nm-engine/tests/keychain.rs` does the same. No test ever reads or writes the login keychain, and the core never shows a Keychain dialog (see [configuration.md](configuration.md), "Passwords").

Speed is tested by the plans of the queries rather than by timing them, which would be slow and flaky. `plan_tests` in `crates/nm-store/src/queries.rs` runs `EXPLAIN QUERY PLAN` on every list query (each filter, sort, scope and page) on an empty database, that is without planner statistics, and fails if one reads a whole folder, scans `message` or `thread_folder`, or runs a subquery for each row. `crates/nm-store/tests/everything.rs` checks that Everything, read from whole threads, lists exactly what merging every folder lists, page after page, on a generated mailbox. `crates/nm-store/tests/migrations.rs` checks that a version 1 database (`tests/fixtures/schema_v1.sql`) migrated by the core has exactly the schema of a fresh one, and repaired data.

## Test IMAP servers

`dev/dovecot/compose.yaml` describes two Dovecot 2.4 servers, started by `make imap-up` (OrbStack or Docker) and stopped by `make imap-down`:

| Server | Ports (127.0.0.1) | Announces |
| --- | --- | --- |
| `full` | 31143 (STARTTLS), 31993 (TLS) | every extension of Dovecot: CONDSTORE, QRESYNC, MOVE, ESEARCH, SPECIAL-USE… |
| `minimal` | 32143 (STARTTLS), 32993 (TLS) | `IMAP4rev1 UIDPLUS IDLE LITERAL+` only: a plain server without the sync extensions, to test the fallbacks |

The core only uses what a server announces, so the minimal server exercises the fallbacks (no MOVE, no CONDSTORE, no SPECIAL-USE) that many real servers need. Any user name logs in with the password `nicemailer`, and a new user starts with an empty mailbox: a test picks a random user and needs no setup or cleanup. Mail is kept in memory and lost when the servers stop.

The servers present a certificate for `localhost` and `127.0.0.1`, signed by a test authority that `dev/dovecot/make-tls.sh` creates on the first `make imap-up` (`dev/dovecot/tls/`, not committed). Clients trust `dev/dovecot/tls/ca.crt` and check the certificate as they would with a real server.

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

Tests use their own `UserDefaults` domain and export directory: they never touch the user's settings or `~/Downloads`.

## Scripted UI tests

`apps/NiceMailer/UITests`: scenarios played by the app itself, with simulated keys and clicks, on fresh data. See `apps/NiceMailer/UITests/README.md` for the script language, the state fields and the list of scenarios. The app stays in the background and off screen, so the tests can run while someone uses the machine.

## Rule

Every UI feature comes with both a unit test and a UI scenario, written in the same change (see CLAUDE.md, "Testing the app").
