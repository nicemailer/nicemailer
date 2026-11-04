# Testing

| Command | What runs | Needs |
| --- | --- | --- |
| `make test` | Rust tests (`cargo test`), Python script tests and Swift unit tests (`swift test`) | nothing running |
| `make swift-test` | Swift unit tests only | |
| `make ui-test` | scripted UI tests of the app | builds the core; starts its own cores on temporary data |
| `make bench` | times every thread list (folders, Everything, search, each filter) through the socket on 100,000 generated conversations, or on `BENCH_DATA_DIR` | starts its own core |

## Rust

Unit tests live next to the code (`#[cfg(test)]`), integration tests in `crates/*/tests/`. `crates/nm-daemon/tests/import_rpc.rs` starts the real core binary and talks to it over its socket.

Speed is tested by the plans of the queries rather than by timing them, which would be slow and flaky. `plan_tests` in `crates/nm-store/src/queries.rs` runs `EXPLAIN QUERY PLAN` on every list query (each filter, sort, scope and page) on an empty database, that is without planner statistics, and fails if one reads a whole folder, scans `message` or `thread_folder`, or runs a subquery for each row. `crates/nm-store/tests/everything.rs` checks that Everything, read from whole threads, lists exactly what merging every folder lists, page after page, on a generated mailbox. `crates/nm-store/tests/migrations.rs` checks that a version 1 database (`tests/fixtures/schema_v1.sql`) migrated by the core has exactly the schema of a fresh one, and repaired data.

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
