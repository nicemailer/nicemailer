# NiceMailer.app: the app, its core, and the mailbox

`make bundle` builds `NiceMailer.app` and installs it in `/Applications`. The app carries everything: its core, the CLI, and the launch agent that runs the core. Open it from the Finder like any app; no `make core` is needed.

```
NiceMailer.app/Contents/
├── Resources/AppIcon.icns                        the icon (design/icon/README.md)
├── MacOS/NiceMailer                              the app
├── Helpers/nicemailer-core                       the core
├── Helpers/nicemailer                            the CLI
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

If the app does not find the core, it asks launchd to start it (`launchctl kickstart`, again every 2 s, and if launchd no longer knows the agent, after registering it again) and waits up to 8 seconds. Coming back to the app without a core (reopened before it had quite quit, for one) connects again at once, unless the core was stopped from the menu in this session. What the app does with the agent is in the system log: `/usr/bin/log show --last 1h --predicate 'subsystem == "eu.nicemailer.app"'` (in zsh, `log` alone is a builtin of the shell). If NiceMailer is turned off in System Settings > General > Login Items, the app says so, with a button that opens that panel.

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

## Signature

The app and its helpers are signed with the Apple Development identity of the Mac, as `make core` signs its core (`scripts/sign-core.sh`, identifier `org.nicemailer.core`): the Keychain sees both cores as the same program, and the passwords saved by one are readable by the other ([configuration.md](configuration.md), "Passwords"). Without that identity the bundle is signed ad hoc: it runs, but its core cannot read the passwords saved by another build.

## Not done yet (`architecture.md`, section 6)

- The option "Keep sorting my mail when NiceMailer is closed" (`core.background_sorting`, `core.idle_exit_secs`): the core always keeps running.
- Moving the app to `/Applications` when opened elsewhere (App Translocation), installing the CLI links, the menu "Uninstall NiceMailer…", the TUI and MCP helpers.
- The CLI does not start the core on demand.

## Tests

`crates/nm-api` (`paths::resolve`), `crates/nm-daemon/tests/agent_rpc.rs` (a second core leaves with code 0; `core.log` written), `DataDirectoryTests` (the same rule in the app; no agent outside a bundle). The registration with launchd and the stop were checked by hand on this Mac: `launchctl print gui/$UID/eu.nicemailer.core` shows the core running, then `not running`, `last exit code = 0` after `make core-stop`.
