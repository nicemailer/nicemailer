# Updates: Sparkle, the core in maintenance, and the database

NiceMailer updates itself with [Sparkle](https://sparkle-project.org): the app checks the feed of the releases once a day (after asking the user, at the second launch), offers a newer version, downloads it, checks its signature, and replaces itself. The menu **NiceMailer > Check for Updates…** checks at once.

What makes an update of NiceMailer more than an app replaced is its core: it runs on its own (a launch agent, [app-bundle.md](app-bundle.md)), from the bundle being replaced, serves other clients (the CLI, an AI agent), and the new version may have to upgrade the database. The rules are in `specifications/mise-a-jour.md`; this page says how they are built.

## The core in maintenance

While an update is installed or the database is upgraded, **the core serves no client**. It keeps its socket open, but answers every request with the error `MAINTENANCE` (1003), whose `data` says why (`nm_api::Maintenance`, `crates/nm-daemon/src/update.rs`), then closes the connection:

| `reason` | When | Ends |
| --- | --- | --- |
| `update` | the app called `prepare_update`, just before Sparkle installs | by itself: the core leaves once its program is replaced, or at `cancel_update`, or after `core.update_timeout_secs` (900 s) |
| `migration` | the core upgrades the database at its start; `from`, `to`, and `at`, the version reached so far | by itself, when the migration ends |
| `newer_schema` | the database was written by a newer NiceMailer (`from` is its version, `to` this core's); never opened, never written | when the core is stopped (another version installed, another mailbox chosen) |
| `migration_failed` | the migration failed; the database was put back as it was (below); `message` is the error | when the core is stopped; its next start tries again |

Why answer rather than refuse the connection: a client that finds no core starts one (`launchctl kickstart`), possibly from a bundle half replaced, and gives up after a few seconds with "unreachable". Told "maintenance", it waits, and starts nothing.

- **New connections** get the error at their `hello`. Connections open when the maintenance begins get it too (a line without an id), after the answers of the requests they already sent, and are closed.
- **The app is the exception during an update**: it drives it, and must be able to cancel it, from its connection or a new one (an app relaunched after a crash). Its `hello` is answered and its connection stays open, but every request but `prepare_update` and `cancel_update` gets the error, `subscribe` included, which is how a new connection of the app learns of it. During a migration nothing is open, so there is no exception.
- **During a migration**, the socket is opened before the database: the core answers while it migrates. A core whose database needs no migration opens its socket after the database, as before.

## An update, step by step

1. Sparkle finds a newer version in the appcast and the user accepts it ("Install and Relaunch"), or it was downloaded in the background and installs when the app quits.
2. **The app prepares the core** (`AppModel.prepareUpdate`, from `Updater.swift`): Sparkle asks the app whether to postpone its relaunch (`shouldPostponeRelaunchForUpdate`), or, for an install at quit, the app delays its own termination (`applicationShouldTerminate`). It calls `prepare_update` and waits for its answer, 10 s at most (5 s at quit). The core enters maintenance, stops its IMAP workers (a batch in progress ends; the actions of the spool stay queued, [spool.md](spool.md)), waits for the write in progress, and **stays alive**: stopped, it would be started again from the old bundle by the next client.
3. **Sparkle replaces the bundle.** The core watches its own program (every 0.5 s while an update is prepared); once it is a new file, it closes cleanly and leaves with code **75**, which launchd restarts (`KeepAlive` restarts any exit but 0): the new core starts from the new bundle.
4. **The app, relaunched, checks the version of the core** (`hello` gives it): a core of another version than the app (one that did not leave by itself: an update installed by hand, an app copied over the old one) is restarted once (`launchctl kickstart -k`, `AppModel.coreNeedsRestart`).
5. **The new core upgrades the database** if needed (below), in maintenance, then serves its clients. Their subscriptions get `ResyncRequired`: the epoch of the events changed ([`evenements.md`](../specifications/evenements.md)).

An update **abandoned**: if Sparkle fails after the preparation (`didAbortWithError`), the app calls `cancel_update`, and the core starts its workers again. If the app died in between, the core leaves the maintenance by itself after `core.update_timeout_secs`; and an app launched again finds the maintenance of an update it did not prepare (the core's version is its own), and cancels it at once rather than waiting.

## The database upgraded: backup and failure

The core upgrades the database at its start, one version per transaction ([migrations.md](migrations.md)). A version that fails is rolled back, but the versions before it are committed: a database taken from 9 to 11 that fails at 12 would stay at 11, a version no released NiceMailer ever wrote, and the previous NiceMailer refuses it. So the migration is wrapped (`nm_store::upgrade::migrate`):

1. The backups of earlier migrations are removed.
2. Both files of the database (`nicemailer.db`, `nicemailer_body.db`) and their write-ahead logs are copied next to them: `nicemailer.db.bak-v9`, `nicemailer.db.bak-v9-wal`, `nicemailer_body.db.bak-v9`. `std::fs::copy` clones on APFS: instant, and no space taken until the files differ. The copy cannot be made: nothing is migrated (it could not be undone).
3. The migrations run.
4. If they fail, the copies are renamed back into place: the database is exactly as the previous version left it. The core stays in maintenance (`migration_failed`); the app shows the error, a **Show the Core Log** button, and **Try Again**, which restarts the core. It never tries again by itself.
5. If they succeed, the backup stays until the next migration. As the database changes, the clone shares fewer blocks with it: at most, it takes the size of the database.

`nicemailer mailbox upgrade` (the app's "upgrade it and use it", [app-bundle.md](app-bundle.md)) does the same.

The raw files are not copied: their format does not depend on the schema.

## What each client does

| Client | `MAINTENANCE` |
| --- | --- |
| App | `update`, `migration`: "NiceMailer is being updated…" or "The core is upgrading this mailbox to the data format 13 (from 9, now at 11), for 1 min 15 s…", with a spinner, asked again every second without starting a core (`AppModel.waitOutMaintenance`). `newer_schema`, `migration_failed`: the reason and what to do, on the screen without a core |
| CLI | A message and the exit status **75** (`EX_TEMPFAIL`); other errors exit with 1. With `--wait`, an update or a migration is waited for, trying every second; `newer_schema` and `migration_failed` fail at once |
| MCP server | Waits, trying every second for 30 seconds, and says so through the progress notifications of the tool call ([mcp.md](mcp.md)); `newer_schema` and `migration_failed` are given to the agent at once |

## Building a release with updates

### Keys

Sparkle accepts an update only if it is signed with the EdDSA key whose public half the installed app carries (`SUPublicEDKey`). The key pair is made **once**, by whoever publishes the releases:

```sh
make build    # downloads Sparkle and its tools
apps/NiceMailer/.build/artifacts/sparkle/Sparkle/bin/generate_keys
```

`generate_keys` keeps the private key in the login keychain and prints the public key. Write the public key, alone, to `apps/NiceMailer/sparkle-public-key` and commit it: `make bundle` puts it in Info.plist. Export the private key and keep it outside the repository (`generate_keys -x <file>`): **a lost key means no installed NiceMailer can ever be updated again**, and the CI will need it.

Without `sparkle-public-key`, a development build has no updates (the menu item is disabled), and `make release` refuses to build.

### The feed

`SUFeedURL` is `https://github.com/nicemailer/nicemailer/releases/latest/download/appcast.xml` (`SPARKLE_FEED_URL` at `make bundle` changes it): the appcast of the latest GitHub release, which only needs to describe that release. GitHub's `latest` skips pre-releases: a release meant to reach the installed apps is published as a normal release.

`make release` (`scripts/make-release.sh`), after the notarization:

- checks, before building, that the private key in the keychain is the one of `sparkle-public-key`;
- signs the disk image with it (Sparkle's `sign_update`);
- writes `appcast.xml` next to the image (`scripts/make-appcast.py`): one item, with `sparkle:version` the build number (`CFBundleVersion`, the count of commits, which only grows), the version, macOS 14 at least, the size and signature of the image, and the release notes of `CHANGELOG.md` as HTML (lists, `###` titles, and inside a line bold, `code` and https links; everything else escaped);
- prints the command that publishes both: `gh release create v<version> <dmg> <appcast.xml> …`.

### Inside the bundle

`Contents/Frameworks/Sparkle.framework` (Sparkle 2.10.1, an exact version in `apps/NiceMailer/Package.swift`), re-signed by our identity from the inside out: its XPC services, `Autoupdate`, `Updater.app`, then the framework, all with the hardened runtime (`scripts/make-bundle.sh`; `scripts/check-signature.sh` checks each against the team of the app). The app finds it through the rpath `@executable_path/../Frameworks`.

Info.plist: `SUFeedURL`, `SUPublicEDKey`, `SUEnableAutomaticChecks`.

Sparkle never starts outside a bundle (`swift run`), without a feed and a key, or in a UI scenario (`Updater.isConfigured`).

## Trying an update by hand

1. Build and install a version (`make bundle`), and note its build number (`CFBundleVersion`, in `/Applications/NiceMailer.app/Contents/Info.plist`).
2. Make a release of a later commit (`make release`), and serve its `appcast.xml` and image from a local web server (`python3 -m http.server`), the appcast's enclosure URL pointing there.
3. Build the installed version with `SPARKLE_FEED_URL=http://localhost:8000/appcast.xml make bundle`, then **Check for Updates…**.
4. Watch the core in `core.log` of the mailbox: `mise à jour : préparation…`, then `… remplacé, le noyau s'arrête…`, and the new core's start; the app's side in the system log: `/usr/bin/log show --last 10m --predicate 'subsystem == "eu.nicemailer.app" AND category == "update"'`.

## Tests

- `crates/nm-daemon/tests/update_rpc.rs`, the real core: no client served between `prepare_update` and `cancel_update` (an open connection told and closed, a new one refused at `hello`, the app let in but refused everything else); only the app may prepare an update; the timeout; the program replaced, the core leaving with 75, and the new core's `ResyncRequired`; a client told of a migration, with its versions, and a SIGTERM cancelling it; a migration failing halfway, the database put back and the core in maintenance; a newer database never written.
- `crates/nm-store/src/upgrade.rs` and `tests/migrations.rs`: the backup put back exactly (files added since removed), only our backups removed, a migration failing at version 13 put back at version 1 with its data, the backup kept after a success.
- `crates/nm-daemon/tests/mcp_proxy.rs`: the MCP server waits through an update, with progress. `crates/nm-cli/tests/maintenance.rs`: the CLI's message and status 75 for each reason, `--wait`.
- `scripts/tests/test_make_appcast.py`: the appcast as Sparkle reads it, escaped notes, their inline Markdown converted, and no Markdown marker left in any section of `CHANGELOG.md`.
- Swift: `MaintenanceTests` (the error read as the core sends it, each text, the wait through an update then a migration, a maintenance only the user can end, the restart of a core of another version, when Sparkle starts); the scenario `mailbox.uiscript` (the screens of an update and of a migration, then served; a failed migration and a newer mailbox; no updates in a scenario).
- Not automated: Sparkle's own download and replacement of a signed bundle, and the restart by launchd. Tried by hand as above before a release (`specifications/plan-release.md`, section 5).
