# NiceMailer.app: the app, its core, and the mailbox

`make bundle` builds `NiceMailer.app` and installs it in `/Applications`. The app carries everything: its core, the CLI, the MCP server for AI agents, and the launch agent that runs the core. Open it from the Finder like any app; no `make core` is needed.

```
NiceMailer.app/Contents/
├── Resources/AppIcon.icns                        the icon (design/icon/README.md)
├── Resources/LICENSE                             the licence, AGPL-3.0
├── MacOS/NiceMailer                              the app
├── Helpers/nicemailer-core                       the core
├── Helpers/nicemailer                            the CLI
├── Helpers/nicemailer-mcp                        the MCP server for AI agents (mcp.md)
├── Frameworks/Sparkle.framework                  the updates (updates.md)
└── Library/LaunchAgents/eu.nicemailer.core.plist the agent of the core
```

| Command | Effect |
| --- | --- |
| `make bundle` | builds and installs `/Applications/NiceMailer.app`; restarts its core if it runs |
| `make open` | the same, then quits the running app and opens the new one |
| `nicemailer core stop` | stops the core of the agent, and waits until it has left (also: `make core-stop`, without the wait) |
| `nicemailer core start` | starts it again, and waits until it answers (also: `make core-start`, without the wait; opening the app) |

`NICEMAILER_DATA_DIR=<dir> make bundle` chooses that mailbox (as "Change Mailbox…" does); `NICEMAILER_FAKE_NETWORK=1 make bundle` makes its core pretend to send ([sending.md](sending.md), "Pretending to send").

## The core runs on its own

At each launch the app registers the agent `eu.nicemailer.core` if it is not (`SMAppService`, `CoreAgent.swift`). macOS shows "Background item added" the first time. launchd then:

- starts the core at once, and at each session opening;
- starts it again after a crash;
- keeps it running when the app quits: mail is received and sorted, and pending sends leave.

The agent has the `ProcessType` `Standard`. With `Background`, macOS limits the CPU and the disk of the core, and the app, which waits for the core at each action, becomes slow when the Mac is busy.

If the app does not find the core, it asks launchd to start it (`launchctl kickstart`, again every 2 s, and if launchd no longer knows the agent, after registering it again; if launchd refuses to start the core, "spawn failed" in `launchctl print`, the agent is registered again once, see "Signature") and waits up to 8 seconds, trying the socket at least every 0.1 s (a longer wait between two tries was time lost once the core was ready). Coming back to the app without a core (reopened before it had quite quit, for one) connects again at once, and so starts a core stopped by `nicemailer core stop`. What the app does with the agent is in the system log: `/usr/bin/log show --last 1h --predicate 'subsystem == "eu.nicemailer.app"'` (in zsh, `log` alone is a builtin of the shell). If NiceMailer is turned off in System Settings > General > Login Items, the app says so, with a button that opens that panel.

The core of the agent writes its messages in `core.log` of its mailbox (renamed `core.log.1` beyond 10 MB): `tail -f "<mailbox>/core.log"`.

### Stopping it

- `nicemailer core stop` (`launchctl kill SIGTERM gui/$UID/eu.nicemailer.core`): the core finishes its work and leaves with code 0, which launchd does not restart (`KeepAlive` restarts only a failure). The command returns once the core has left, at most 30 s later. The core starts again with `nicemailer core start`, when the app is launched or comes back to the front, and at the next session.
- `nicemailer core start` asks launchd for the core and returns once it answers on its socket, at most 15 s later.
- Both succeed when there is nothing to do. Both act only on the core of the agent, which serves the mailbox chosen in the app: with `--data-dir` or `NICEMAILER_DATA_DIR` naming another mailbox they refuse, and `core stop` also refuses a core started by hand, to be stopped where it runs (Ctrl-C). `core start` needs the agent, which only the app can register: open NiceMailer once.
- To keep it from starting at all: turn NiceMailer off in System Settings > General > Login Items.

### With `make core`

A core already serving a mailbox keeps it: a second core on the same directory leaves at once, with code 0 ("un autre noyau utilise déjà…"). To run the core of the source tree in a terminal, stop the one of the agent first (`nicemailer core stop`), then `make core`; `nicemailer core start` gives the agent its core back.

## Choosing the mailbox

A mailbox is a data directory of NiceMailer (`nicemailer.db`, the raw store, the socket). Like Photos' libraries, the app can use any of them:

- menu **NiceMailer > Change Mailbox…**, or hold **⌥** while the app opens: a panel chooses the folder (an empty one starts a new mailbox);
- the app records the choice, restarts the core on it and reloads everything. The window's title names the mailbox when it is not the default one.

Which directory, for the app, the core and the CLI alike (`nm_api::paths::data_dir`, `DataDirectory.swift`):

1. `NICEMAILER_DATA_DIR`, when set: it always wins (development, tests; the app then refuses to change it);
2. the mailbox chosen: one line, an absolute path, in `~/Library/Application Support/NiceMailer/data-dir` (the core started by launchd gets no variable, so the choice is a file every program reads). The app writes it `0600`. The core, the CLI and the app follow it only when no one but the current user can have written it: a regular file, never a symbolic link, owned by the user, writable by her alone, in a directory writable by her alone (`nm_api::paths::choice`, `DataDirectory.readChoice`). Otherwise all three serve or look for the default mailbox, and the core says why in its log. This guards against another account of the Mac, not against a program of the same user;
3. `~/Library/Application Support/NiceMailer`.

So `nicemailer` and `make core` without the variable follow the mailbox chosen in the app.

### The folder is checked before the switch

Choosing a folder the core cannot open used to restart the core on it, and leave the app without any core. The app now asks first, through the CLI it carries (`Contents/Helpers/nicemailer --data-dir <folder> mailbox check --json`, [migrations.md](migrations.md), "Choosing a mailbox"), which reads the database without changing it:

| The database | The app |
| --- | --- |
| None yet (an empty folder), or current | Switches to it |
| An earlier data format, which the migrations upgrade | Asks: **upgrade it and use it**, or keep the current mailbox. In the geek theme the question is in the status bar (`ASK`; `y` upgrades, Escape cancels, or a click on the answer); in the classic one, a dialog |
| An early development database (it lacks what the migrations assume), one written by a newer NiceMailer, or a file that is not a database | Refuses it, says why, and keeps the current mailbox |

The upgrade is made by the CLI (`nicemailer --data-dir <folder> mailbox upgrade`) **before** the switch, while the current mailbox stays in use: the app waits only a few seconds for a core to answer, and a large mailbox takes longer to migrate. It cannot be undone: an older NiceMailer refuses the result, which the question says (copy the folder first to keep the old version able to open it). If it fails (a core already holds that folder, say), the current mailbox stays and the error is shown.

An app without its CLI (`swift run`) uses the folder unchecked, as before; `NICEMAILER_CLI=<path>` names one (the UI scenarios do).

## How long the launch takes

Until the first list is shown, the list and the status bar say what is happening instead of "No mail": "Connecting to the core…", then "Starting the core…" when no core answered and launchd is starting it, then "Loading the mail…" once connected (`AppModel.StartupPhase`).

Once connected, the app asks for the folders and the first list at once; the settings of `[ui]` and the state of the AI are asked at the same time, not before (`AppModel.startSession`).

Both sides time their part, so that a slow launch can be measured on the Mac where it happens:

- the core, in `core.log`, once its socket is bound: `startup: socket ready after 1840 ms (store 1210, readers 4, settings 35, recovery 580, socket 11)` (the database opened, the read connections, the settings and accounts, the actions and drafts taken up again, the socket), after the steps of the opening of the database: `startup: store opened after 949 ms (inspect 6, open 0, version 2, body file 1, schema 0, reconcile 931, statistics 6)`. The reconciliation of the two files, the longest step, is skipped after a clean stop of the core ([backup.md](backup.md), "Atomic commits");
- the app, in the system log (category `startup`), once its first list is shown: `first list after 2310 ms (60 rows; socket after 1840 ms, core started by the app)`, from the app's launch. Read it with `/usr/bin/log show --last 10m --predicate 'subsystem == "eu.nicemailer.app" AND category == "startup"'`.

## When the core cannot start

A core that stops on an error writes why to `core-error.json` in its folder (`nm_api::StartupError`: `kind` `outdated` or `other`, the message, and what is missing), and removes it once a core starts there. When no core answers, the app shows that reason, in the user's language, instead of "unreachable", with a **Change Mailbox…** button beside "Try Again": the way out when the folder chosen is one the core refuses.

The core still exits with a failure there, so launchd starts it again every 10 seconds (`KeepAlive` restarts a failed exit); each attempt fails the same way and rewrites the file. Harmless, if useless, until another mailbox is chosen.

A database the core can read but must not serve does not stop it: one written by a newer NiceMailer, or one whose upgrade failed. The core stays, in maintenance, and tells every client why ([updates.md](updates.md)); the app shows it on the same screen, with **Show the Core Log** after a failed upgrade, whose **Try Again** restarts the core.

## While the core upgrades the mailbox

A core newer than the mailbox it opens upgrades its database first ([migrations.md](migrations.md)). Most upgrades take a fraction of a second, but some take minutes on a large mailbox (version 10 copies the bodies to their own file: 4 minutes for 400,000 mails). So ([updates.md](updates.md), "The core in maintenance"):

- **The core says so.** It opens its socket before migrating, and answers every client with the error `MAINTENANCE`, reason `migration`, the versions and the one reached so far. Its log says it too: `mise à jour de la base : version 9 vers 13`, then `version 10 atteinte`…
- **The app waits.** It shows "The core is upgrading this mailbox to the data format 13 (from 9, now at 11), for 1 min 15 s…" with a spinner, instead of "unreachable", asks again every second without starting a core, and connects as soon as the core serves (`AppModel.waitOutMaintenance`).
- **The database is copied first** and put back if the upgrade fails ([updates.md](updates.md), "The database upgraded").
- **A stop cancels it cleanly.** The opening runs on a thread of its own while the core watches for SIGTERM and Ctrl-C. A stop during the upgrade leaves with code 0, which launchd does not restart; the upgrade of a version is one transaction, rolled back, and done again from the start at the next launch. Before, a SIGTERM killed the core in the middle of the upgrade (its handler was not installed yet): launchd took it for a failure, started it again, and the upgrade started over every time, so the core seemed to refuse to stop.

## Updates

Sparkle updates the app, its core and its helpers together, from the GitHub releases: **NiceMailer > Check for Updates…**, or once a day in the background. The core is put in maintenance while the bundle is replaced, then restarted from the new one. Everything is in [updates.md](updates.md).

## Signature

`make bundle` signs the app and its helpers from the inside out (the core, the CLI, the MCP server, then the app), all with the **hardened runtime**, which notarization requires. A development build is signed the same way, so that it meets the same restrictions every day rather than on the day of a release.

| Program | Identifier | Entitlements |
| --- | --- | --- |
| `Contents/MacOS/NiceMailer` | `eu.nicemailer.NiceMailer` | `apps/NiceMailer/NiceMailer.entitlements`: the address book only |
| `Contents/Helpers/nicemailer-core` | `eu.nicemailer.core` (`scripts/sign-core.sh`) | none |
| `Contents/Helpers/nicemailer` | `eu.nicemailer.cli` | none |
| `Contents/Helpers/nicemailer-mcp` | `eu.nicemailer.mcp` | none |

Under the hardened runtime, a protected resource must be declared: without `com.apple.security.personal-information.addressbook`, macOS refuses the Contacts without asking (`docs/geek-ui.md`, "Senders in the address book"). The editor's WebKit runs in WebKit's own processes and needs no entitlement from the app.

**Client verification.** At each connection to its socket, the core reads the peer's audit token (`getsockopt(LOCAL_PEERTOKEN)`, preferred to the PID, which the system may reassign) and validates its code signature (`SecCodeCopyGuestWithAttributes`, then `SecCodeCheckValidity`, on tokio's blocking pool so that the loop accepting connections never waits) against:

```
anchor apple generic and certificate leaf[subject.OU] = "<team id>"
  and (identifier "eu.nicemailer.NiceMailer" or identifier "eu.nicemailer.cli" or identifier "eu.nicemailer.tui"
       or identifier "eu.nicemailer.mcp" or identifier "eu.nicemailer.web")
  [and certificate 1[field.1.2.840.113635.100.6.2.6] and certificate leaf[field.1.2.840.113635.100.6.1.13]]
  and !(entitlement["com.apple.security.get-task-allow"] exists)
```

plus the hardened runtime flag of the client's code directory, which a requirement cannot express (`crates/nm-signing`; `specifications/architecture.md`, section 9). The identity, and so the rights, come from that signature, never from what the client declares in `hello`. A connection whose signature does not meet the requirement is closed and logged, at most one log line a second. The other direction too: the CLI, the MCP server and the app verify that the program serving the socket is the core (`identifier "eu.nicemailer.core"`, same rules) before they send anything, so that no other program listening there receives a password or a draft (`CoreSignature.swift`).

The team id is never embedded at build time: each binary reads it from its own signature at run time (`SecCodeCopySelf`), and demands the same team of its peer. A Developer ID build (a release) also demands the two Developer ID markers in brackets above, so it refuses development builds of the same team; an Apple Development build (`make bundle`) demands its team only. A release build without a team of its own (unsigned, ad hoc) refuses every peer (fail-closed) and says so at its start. The check costs a few milliseconds in a release build. Development builds relax it *at compile time*, never through an environment variable: a debug build, or a release built with the feature `nm-signing/dev-unverified` (`make core`, `make ui-test`, `make bench`, `make volume-test`, and an ad-hoc bundle when no identity is found), checks nothing, and `hello` declares the source. `make bundle` and `make release` build the core, the CLI and the MCP server in release, without that feature, so a development bundle is protected like a release. The app checks nothing in a debug build, nor when it has no team itself (`swift run`, an ad-hoc bundle). Not covered: an older version of one of our binaries, signed by the same team, meets the requirement.

**Test variables.** A release app ignores the variables its UI tests drive it with (`NICEMAILER_UI_SCRIPT`, `NICEMAILER_UI_OUT`, `NICEMAILER_CLI`, `NICEMAILER_MCP`, `NICEMAILER_EXPORT_DIR`, `DevEnvironment.swift`): otherwise any program could launch the signed app with a script and play keys and clicks with the app's rights. `make ui-test` plays a debug build; `NICEMAILER_UI_BIN` must name a debug build too.

**Development** (`make bundle`, `make open`, `make core`): the Apple Development identity of the same team as the Developer ID Application identity, no secure timestamp (offline, faster). `scripts/signing-identity.sh` chooses it, for both scripts. The team matters for the Keychain, which ties the passwords to it: with one team for both, a release installed over a development build, or the reverse, keeps its passwords; with two teams, they must be entered again. When the app registers the agent, launchd also records a launch constraint made from the core's signature: identifier, team, and validation category (Apple Development or Developer ID). Any switch between a development build and a release breaks it, even within one team: launchd refuses to start the new core (`job state = spawn failed`, exit 78, in `launchctl print gui/$UID/eu.nicemailer.core`) while `launchctl kickstart` succeeds. The app repairs it: after 2 s without a core it reads `launchctl print`, and on a refused spawn registers the agent again, once per try (`CoreAgent.repairIfSpawnRefused`). By hand: `launchctl bootout gui/$UID/eu.nicemailer.core`, then launch the app. Without an Apple Development identity of that team, the first Apple Development identity signs, with a warning; without a Developer ID, the first one, silently. The Keychain sees the core of the app, the one of `make core` and the one of a release as the same program (`scripts/sign-core.sh` gives all of them the same designated requirement), and the passwords saved by one are readable by the other ([configuration.md](configuration.md), "Passwords"). Without that identity the bundle is signed ad hoc: it runs, but its core cannot read the passwords saved by another build, and `make bundle` warns.

**Release** (`make release`): the **Developer ID Application** identity, the one Gatekeeper accepts outside the App Store, and a secure timestamp from Apple on each signature. `$SIGN_IDENTITY` (a SHA-1 hash or a name) overrides the identity chosen, in development as in a release.

`scripts/check-signature.sh [--release] <NiceMailer.app>` checks a bundle, and `make bundle` and `make release` run it: valid seal, hardened runtime, the identifiers above, one team for the three programs (the Keychain partition is `teamid:<team>`), a designated requirement of the core that names only its identifier and its team (so that development builds and releases read each other's passwords, [configuration.md](configuration.md), "Passwords"), arm64 code, the address book entitlement; with `--release`, a Developer ID Application, a secure timestamp and no `get-task-allow`. A release that fails it is not built; a development build that fails it installs with a warning.

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
| `CFBundleVersion` | number of commits (`git rev-list --count HEAD`) | macOS and Sparkle compare it to know the newer build: it must only grow |
| `NMGitDescribe` | `git describe --tags --always --dirty` | Traces a build to its commit |

A release bumps `version` in `Cargo.toml` in a commit of its own, then is tagged `v<version>`.

## Releases

`make release` (`scripts/make-release.sh`) writes `dist/<version>-<time>/NiceMailer-<version>.dmg` and `notes.md`, a new directory each time:

0. Before building, it refuses a version whose tag `v<version>` already exists (the version of `Cargo.toml` was not raised) and a changelog that does not give its notes ("Release notes" below).
1. `scripts/make-bundle.sh --release`: refuses uncommitted changes, `NICEMAILER_FAKE_NETWORK` and a missing `NICEMAILER_MICROSOFT_CLIENT_ID`; signs with the Developer ID; runs `check-signature.sh --release`; installs nothing and leaves the running app and its core alone.

`NICEMAILER_MICROSOFT_CLIENT_ID` is the Entra application id ("client ID") of NiceMailer, from the "Overview" page of its registration in the Entra admin center. The build embeds it in the core (`crates/nm-engine/build.rs` checks its form and fails the build on a malformed value), so the distributed app signs Microsoft 365 and Outlook.com accounts in at the first install, with no configuration by the user. The id is public — a public client sends it in every authorization URL — and no client secret is compiled in: the flow is protected by PKCE and the loopback redirect, not by a shared secret (`specifications/oauth2.md`, section 4; `docs/accounts.md`, "OAuth2 sign-in"). Keep the variable in the environment of the release machine (a CI secret for the release workflow), not in the repository.
2. A disk image with the app and a link to `/Applications`, signed.
3. Notarization (`notarytool submit --wait`, a few minutes); on a refusal, Apple's log, which names each problem, is printed.
4. The ticket stapled to the image (`stapler`), so that the first launch is accepted offline.
5. Gatekeeper's verdict (`spctl`) on the image and on the app in it.

It publishes nothing: it ends by printing the SHA-256 and the commands that tag and create the GitHub release with `--notes-file <dist/…/notes.md>`. `make release-check` skips step 0 and stops after step 2, to check the signature without Apple, between two releases too. Before publishing, the image is tried on another Mac or user account (`specifications/plan-release.md`, section 5).

### Homebrew cask

The same disk image is installable with Homebrew from the tap [`nicemailer/homebrew-tap`](https://github.com/nicemailer/homebrew-tap): `brew install --cask nicemailer/tap/nicemailer` (one command, no `brew tap` needed). The cask installs the app from the release DMG, links the CLI of `Contents/Helpers` into Homebrew's `bin`, and on `brew uninstall` quits the app and unregisters the `eu.nicemailer.core` agent (`uninstall launchctl:`); `--zap` also removes the preferences and the mailbox. Its `version` and `sha256` are refreshed by the tap's own workflow (`.github/workflows/update-cask.yml`) within an hour of each release; run that workflow by hand from the tap's Actions tab to refresh at once. The app itself keeps updating through Sparkle, not Homebrew.

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

`crates/nm-api` (`paths::resolve`), `crates/nm-daemon/tests/agent_rpc.rs` (a second core leaves with code 0; `core.log` written), `crates/nm-daemon/tests/startup_rpc.rs` (`core-error.json` written for an early development database, removed at the next start), `crates/nm-daemon/tests/update_rpc.rs` (the maintenance during an upgrade, a SIGTERM then leaving with code 0, the database left at its version; a newer database; [updates.md](updates.md)), `MaintenanceTests` (the wait), `DataDirectoryTests` (the same rule in the app; no agent outside a bundle), `MailboxCheckTests` (what each answer of the CLI makes the app do, the question and its keys, a failed upgrade, the reason shown without a core), and the scenario `mailbox.uiscript` (the screens of an update and an upgrade; refusals, a new folder, the question answered by Escape then by a click, the upgraded mailbox used without a question). The registration with launchd and the stop were checked by hand on this Mac: `launchctl print gui/$UID/eu.nicemailer.core` shows the core running, then `not running`, `last exit code = 0` after `make core-stop`. `crates/nm-cli/src/core_agent.rs` tests `nicemailer core` against a launchd played by the test, on the output of a real `launchctl print`; `crates/nm-cli/tests/core_command.rs` tests the refusal of another mailbox.
