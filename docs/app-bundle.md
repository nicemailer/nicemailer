# NiceMailer.app: the app, its core, and the mailbox

`make bundle` builds `NiceMailer.app` and installs it in `/Applications`. The app carries everything: its core, the CLI, the MCP server for AI agents, and the launch agent that runs the core. Open it from the Finder like any app; no `make core` is needed.

```
NiceMailer.app/Contents/
├── Resources/AppIcon.icns                        the icon (design/icon/README.md)
├── MacOS/NiceMailer                              the app
├── Helpers/nicemailer-core                       the core
├── Helpers/nicemailer                            the CLI
├── Helpers/nicemailer-mcp                        the MCP server for AI agents (mcp.md)
└── Library/LaunchAgents/eu.nicemailer.core.plist the agent of the core
```

| Command | Effect |
| --- | --- |
| `make bundle` | builds and installs `/Applications/NiceMailer.app`; restarts its core if it runs |
| `make open` | the same, then quits the running app and opens the new one |
| `make core-stop` | stops the core of the agent (also: menu NiceMailer > Stop the Core) |
| `make core-start` | starts it again (also: opening the app) |

`NICEMAILER_DATA_DIR=<dir> make bundle` chooses that mailbox (as "Change Mailbox…" does); `NICEMAILER_FAKE_NETWORK=1 make bundle` makes its core pretend to send ([sending.md](sending.md), "Pretending to send").

## The core runs on its own

At each launch the app registers the agent `eu.nicemailer.core` if it is not (`SMAppService`, `CoreAgent.swift`). macOS shows "Background item added" the first time. launchd then:

- starts the core at once, and at each session opening;
- starts it again after a crash;
- keeps it running when the app quits: mail is received and sorted, and pending sends leave.

If the app does not find the core, it asks launchd to start it (`launchctl kickstart`, again every 2 s, and if launchd no longer knows the agent, after registering it again) and waits up to 8 seconds, trying the socket at least every 0.1 s (a longer wait between two tries was time lost once the core was ready). Coming back to the app without a core (reopened before it had quite quit, for one) connects again at once, unless the core was stopped from the menu in this session. What the app does with the agent is in the system log: `/usr/bin/log show --last 1h --predicate 'subsystem == "eu.nicemailer.app"'` (in zsh, `log` alone is a builtin of the shell). If NiceMailer is turned off in System Settings > General > Login Items, the app says so, with a button that opens that panel.

The core of the agent writes its messages in `core.log` of its mailbox (renamed `core.log.1` beyond 10 MB): `tail -f "<mailbox>/core.log"`.

### Stopping it

- Menu **NiceMailer > Stop the Core**, or `make core-stop` (`launchctl kill SIGTERM gui/$UID/eu.nicemailer.core`): the core finishes its work and leaves with code 0, which launchd does not restart (`KeepAlive` restarts only a failure). It starts again at the next launch of the app, at the next session, or with `make core-start`.
- To keep it from starting at all: turn NiceMailer off in System Settings > General > Login Items.

### With `make core`

A core already serving a mailbox keeps it: a second core on the same directory leaves at once, with code 0 ("un autre noyau utilise déjà…"). To run the core of the source tree in a terminal, stop the one of the agent first (`make core-stop`), then `make core`; `make core-start` gives the agent its core back.

## Choosing the mailbox

A mailbox is a data directory of NiceMailer (`nicemailer.db`, the raw store, the socket). Like Photos' libraries, the app can use any of them:

- menu **NiceMailer > Change Mailbox…**, or hold **⌥** while the app opens: a panel chooses the folder (an empty one starts a new mailbox);
- the app records the choice, restarts the core on it and reloads everything. The window's title names the mailbox when it is not the default one.

Which directory, for the app, the core and the CLI alike (`nm_api::paths::data_dir`, `DataDirectory.swift`):

1. `NICEMAILER_DATA_DIR`, when set: it always wins (development, tests; the app then refuses to change it);
2. the mailbox chosen: one line, an absolute path, in `~/Library/Application Support/NiceMailer/data-dir` (the core started by launchd gets no variable, so the choice is a file every program reads);
3. `~/Library/Application Support/NiceMailer`.

So `nicemailer` and `make core` without the variable follow the mailbox chosen in the app.

### The folder is checked before the switch

Choosing a folder the core cannot open used to restart the core on it, and leave the app without any core. The app now asks first, through the CLI it carries (`Contents/Helpers/nicemailer mailbox check --json <folder>`, [migrations.md](migrations.md), "Choosing a mailbox"), which reads the database without changing it:

| The database | The app |
| --- | --- |
| None yet (an empty folder), or current | Switches to it |
| An earlier data format, which the migrations upgrade | Asks: **upgrade it and use it**, or keep the current mailbox. In the geek theme the question is in the status bar (`ASK`; `y` upgrades, Escape cancels, or a click on the answer); in the classic one, a dialog |
| An early development database (it lacks what the migrations assume), one written by a newer NiceMailer, or a file that is not a database | Refuses it, says why, and keeps the current mailbox |

The upgrade is made by the CLI (`nicemailer mailbox upgrade <folder>`) **before** the switch, while the current mailbox stays in use: the app waits only a few seconds for a core to answer, and a large mailbox takes longer to migrate. It cannot be undone: an older NiceMailer refuses the result, which the question says (copy the folder first to keep the old version able to open it). If it fails (a core already holds that folder, say), the current mailbox stays and the error is shown.

An app without its CLI (`swift run`) uses the folder unchecked, as before; `NICEMAILER_CLI=<path>` names one (the UI scenarios do).

## How long the launch takes

Until the first list is shown, the list and the status bar say what is happening instead of "No mail": "Connecting to the core…", then "Starting the core…" when no core answered and launchd is starting it, then "Loading the mail…" once connected (`AppModel.StartupPhase`).

Once connected, the app asks for the folders and the first list at once; the settings of `[ui]` and the state of the AI are asked at the same time, not before (`AppModel.startSession`).

Both sides time their part, so that a slow launch can be measured on the Mac where it happens:

- the core, in `core.log`, once its socket is bound: `startup: socket ready after 1840 ms (store 1210, readers 4, settings 35, recovery 580, socket 11)` (the database opened, the read connections, the settings and accounts, the actions and drafts taken up again, the socket), after the steps of the opening of the database: `startup: store opened after 949 ms (inspect 6, open 0, version 2, body file 1, schema 0, reconcile 931, statistics 6)`. The reconciliation of the two files, the longest step, is skipped after a clean stop of the core ([backup.md](backup.md), "Atomic commits");
- the app, in the system log (category `startup`), once its first list is shown: `first list after 2310 ms (60 rows; socket after 1840 ms, core started by the app)`, from the app's launch. Read it with `/usr/bin/log show --last 10m --predicate 'subsystem == "eu.nicemailer.app" AND category == "startup"'`.

## When the core cannot start

A core that stops on an error writes why to `core-error.json` in its folder (`nm_api::StartupError`: `kind` `outdated`, `too_new` or `other`, the message, and what is missing or the versions), and removes it once a core starts there. When no core answers, the app shows that reason, in the user's language, instead of "unreachable", with a **Change Mailbox…** button beside "Try Again": the way out when the folder chosen is one the core refuses.

The core still exits with a failure there, so launchd starts it again every 10 seconds (`KeepAlive` restarts a failed exit); each attempt fails the same way and rewrites the file. Harmless, if useless, until another mailbox is chosen.

## While the core upgrades the mailbox

A core newer than the mailbox it opens upgrades its database first (`docs/migrations.md`), and its socket only opens afterwards. Most upgrades take a fraction of a second, but some take minutes on a large mailbox (version 10 copies the bodies to their own file: 4 minutes for 400,000 mails). So:

- **The core says so.** Before migrating, it writes `core-upgrading.json` in its folder (`nm_api::UpgradeStatus`: the versions, its pid, when it started), and removes it once the database is open. Its log says it too: `mise à jour de la base : version 9 vers 10`.
- **The app waits.** When no core answers and that file names a process that runs, the app shows "The core is upgrading this mailbox to the data format 10 (from 9), for 1 min 15 s…" with a spinner, instead of "unreachable", and checks again every second; it connects as soon as the file is gone (`AppModel.waitForUpgrade`). A file left by a core that was killed names a process that is gone: it is ignored.
- **A stop cancels it cleanly.** The opening runs on a thread of its own while the core watches for SIGTERM and Ctrl-C. A stop during the upgrade leaves with code 0, which launchd does not restart; the upgrade of a version is one transaction, rolled back, and done again from the start at the next launch. Before, a SIGTERM killed the core in the middle of the upgrade (its handler was not installed yet): launchd took it for a failure, started it again, and the upgrade started over every time, so the core seemed to refuse to stop.

## Signature

`make bundle` signs the app and its helpers from the inside out (the core, the CLI, the MCP server, then the app), all with the **hardened runtime**, which notarization requires. A development build is signed the same way, so that it meets the same restrictions every day rather than on the day of a release.

| Program | Identifier | Entitlements |
| --- | --- | --- |
| `Contents/MacOS/NiceMailer` | `eu.nicemailer.NiceMailer` | `apps/NiceMailer/NiceMailer.entitlements`: the address book only |
| `Contents/Helpers/nicemailer-core` | `org.nicemailer.core` (`scripts/sign-core.sh`) | none |
| `Contents/Helpers/nicemailer` | `eu.nicemailer.cli` | none |
| `Contents/Helpers/nicemailer-mcp` | `eu.nicemailer.mcp` | none |

Under the hardened runtime, a protected resource must be declared: without `com.apple.security.personal-information.addressbook`, macOS refuses the Contacts without asking (`docs/geek-ui.md`, "Senders in the address book"). The editor's WebKit runs in WebKit's own processes and needs no entitlement from the app.

**Development** (`make bundle`, `make core`): the Apple Development identity of the Mac, no secure timestamp (offline, faster). The Keychain sees the core of the app and the one of `make core` as the same program, and the passwords saved by one are readable by the other ([configuration.md](configuration.md), "Passwords"). Without that identity the bundle is signed ad hoc: it runs, but its core cannot read the passwords saved by another build, and `make bundle` warns.

**Release** (`make release`): the **Developer ID Application** identity, the one Gatekeeper accepts outside the App Store, and a secure timestamp from Apple on each signature. `$SIGN_IDENTITY` (a SHA-1 hash or a name) chooses among several identities.

`scripts/check-signature.sh [--release] <NiceMailer.app>` checks a bundle, and `make bundle` and `make release` run it: valid seal, hardened runtime, the identifiers above, one team for the three programs (the Keychain partition is `teamid:<team>`), arm64 code, the address book entitlement; with `--release`, a Developer ID Application, a secure timestamp and no `get-task-allow`. A release that fails it is not built; a development build that fails it installs with a warning.

The seal of the app records every resource by its hash, but a helper by its signature requirement (identifier and team): a helper re-signed by the same identity still passes the seal, which is why each helper is checked on its own.

### Creating the Developer ID certificate

Once, by the Account Holder of the paid Apple Developer team:

1. Xcode → Settings → Accounts → the paid team (not "Personal Team") → **Manage Certificates…** → **+** → **Developer ID Application**.
2. `security find-identity -v -p codesigning` must list `"Developer ID Application: … (<team>)"`. If not valid, install the intermediate "Developer ID - G2" from <https://www.apple.com/certificateauthority/> into the login keychain.
3. Export the certificate with its private key (Keychain Access → Export, `.p12` with a password) and keep it outside the repository: a lost key cannot sign again, and the CI will need it.

The notarization credentials, once: an app-specific password (<https://account.apple.com>, Sign-In and Security), then `xcrun notarytool store-credentials nicemailer --apple-id <Apple ID> --team-id <team>`, which keeps it in the keychain under the profile `nicemailer`.

## Versions

| Key of `Info.plist` | Value | Why |
| --- | --- | --- |
| `CFBundleShortVersionString` | `version` of `[workspace.package]` in `Cargo.toml` | One version for the app, the core and `nicemailer --version` |
| `CFBundleVersion` | number of commits (`git rev-list --count HEAD`) | macOS, and later Sparkle, compare it to know the newer build: it must only grow |
| `NMGitDescribe` | `git describe --tags --always --dirty` | Traces a build to its commit |

A release bumps `version` in `Cargo.toml` in a commit of its own, then is tagged `v<version>`.

## Releases

`make release` (`scripts/make-release.sh`) writes `dist/<version>-<time>/NiceMailer-<version>.dmg` and `notes.md`, a new directory each time:

0. Before building, it refuses a version whose tag `v<version>` already exists (the version of `Cargo.toml` was not raised) and a changelog that does not give its notes ("Release notes" below).
1. `scripts/make-bundle.sh --release`: refuses uncommitted changes and `NICEMAILER_FAKE_NETWORK`; signs with the Developer ID; runs `check-signature.sh --release`; installs nothing and leaves the running app and its core alone.
2. A disk image with the app and a link to `/Applications`, signed.
3. Notarization (`notarytool submit --wait`, a few minutes); on a refusal, Apple's log, which names each problem, is printed.
4. The ticket stapled to the image (`stapler`), so that the first launch is accepted offline.
5. Gatekeeper's verdict (`spctl`) on the image and on the app in it.

It publishes nothing: it ends by printing the SHA-256 and the commands that tag and create the GitHub release with `--notes-file <dist/…/notes.md>`. `make release-check` skips step 0 and stops after step 2, to check the signature without Apple, between two releases too. Before publishing, the image is tried on another Mac or user account (`specifications/plan-release.md`, section 5).

## Release notes

The notes of each release come from `CHANGELOG.md` at the root of the repository, written by hand for users (its header gives the rules):

- a change users can notice adds its entry under `## Unreleased` in the same commit;
- the commit that raises `version` in `Cargo.toml` renames that section to `## <version> — <YYYY-MM-DD>` and starts a new, empty `## Unreleased` above it.

`scripts/release-notes.py <version>` prints the body of the section of a version, up to the next `## ` heading, and refuses (exit code 1, with what to fix) a version without a section or with an empty one, and entries left under `## Unreleased`, which would be changes shipped without their notes. `make release` runs it before the build. Commit messages are not used: they are written for developers.

Tests: `scripts/tests/test_release_notes.py` (`make test`), which also checks that `CHANGELOG.md` has a section for the version of `Cargo.toml`.

Known limit: arm64 only (`specifications/plan-release.md`, section 2).

## Not done yet (`architecture.md`, section 6)

- The option "Keep sorting my mail when NiceMailer is closed" (`core.background_sorting`, `core.idle_exit_secs`): the core always keeps running.
- Moving the app to `/Applications` when opened elsewhere (App Translocation), installing the CLI links, the menu "Uninstall NiceMailer…", the TUI and MCP helpers.
- The CLI does not start the core on demand.

## Tests

`crates/nm-api` (`paths::resolve`), `crates/nm-daemon/tests/agent_rpc.rs` (a second core leaves with code 0; `core.log` written), `crates/nm-daemon/tests/startup_rpc.rs` (`core-error.json` written for an early development database and for a newer one, removed at the next start; `core-upgrading.json` written during an upgrade, and a SIGTERM then leaving with code 0, the database left at its version), `UpgradeWaitTests` (the file read as the core writes it, a dead pid ignored, the wait), `DataDirectoryTests` (the same rule in the app; no agent outside a bundle), `MailboxCheckTests` (what each answer of the CLI makes the app do, the question and its keys, a failed upgrade, the reason shown without a core), and the scenario `mailbox.uiscript` (the upgrading screen; refusals, a new folder, the question answered by Escape then by a click, the upgraded mailbox used without a question). The registration with launchd and the stop were checked by hand on this Mac: `launchctl print gui/$UID/eu.nicemailer.core` shows the core running, then `not running`, `last exit code = 0` after `make core-stop`.
