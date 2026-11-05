# Deep links to a mail

Each mail has its own URL, as short as possible:

```
nicemailer://m/48213
```

`m` stands for "message", and `48213` is the message's id in the database. Clicked anywhere on the Mac (a note, a task, a calendar event), the link opens NiceMailer on that mail: in its folder, selected and shown, with any open panel closed; in one column, the mail fills the window, as after Return.

## Copying the link of a mail

The first row of the links panel (`l`) is **Link to this mail**, with its URL on the right:

- Return copies a Markdown link, `[subject](nicemailer://m/48213)`, ready to paste in Obsidian or any Markdown note: the note shows the subject, a click opens the mail. Brackets in the subject are escaped (`[\[Spam\] Offre](…)`).
- ⌘-Return copies the bare URL.

The status bar says what was copied. Typing `obsidian`, `markdown` or `deeplink` in the panel also finds the row.

A conversation holds several mails; the link points to its **latest received** message, so that it opens the conversation in its folder (Inbox, Archive…) rather than in Sent.

## Use with Obsidian

1. Read the mail in NiceMailer, press `l`, then Return.
2. Paste in the note (⌘V): `[Devis toiture](nicemailer://m/48213)`.
3. Later, a click on the link in the note (⌘-click in edit mode) opens the mail in NiceMailer.

Obsidian opens links of other apps through macOS, which needs NiceMailer to be installed as an app (below).

## Installing the app for links

macOS gives a URL only to an app bundle that declares its scheme. `make app` (`swift run`) starts a bare executable, which macOS cannot reach, so the links need:

```sh
make bundle
```

It builds the app in release mode, wraps it in `NiceMailer.app` (its `Info.plist` declares the `nicemailer` scheme, and why the app reads Contacts: `docs/geek-ui.md`, "Senders in the address book"), signs it ad hoc (enough for this Mac), installs it in `~/Applications` and registers it with Launch Services. Run it again after pulling new code.

`make open` does the same, then launches the app: once built, it quits the installed app if it runs (a SIGTERM, waiting 5 s at most; it keeps nothing unsaved, the core holds the data), installs the new one and opens it. An app started by `make app` is left alone. With the core in another terminal, the two commands to run the app with links are:

```sh
NICEMAILER_DATA_DIR="$HOME/Library/Application Support/NiceMailer-archive" make core
NICEMAILER_DATA_DIR="$HOME/Library/Application Support/NiceMailer-archive" make open
```

`make app` stays the quickest way to run a debug build, its output in the terminal, but a link clicked meanwhile launches the installed app as well, in a second window on the same core.

An app opened by a link or by the Finder gets no variable from a shell. To use another data directory, give it when building: `NICEMAILER_DATA_DIR=/other/directory make bundle` writes it into the app (`LSEnvironment`). The core must be running, as for `make app`.

If the app is not running, the link launches it, and the mail opens once the app is connected to the core. If it is running, the link goes to its window (no second window opens).

## How long a link lives

The id is the core's `message.id`: short and exact, but tied to the database. A link keeps working while the mail is in the database, whatever folder it is moved to. It breaks if the mail is deleted, or if the database is rebuilt (a re-import gives new ids): the app then says "This mail is no longer in NiceMailer" and stays where it was. A URL that is not a mail link (`nicemailer://x/1`, letters in the id) is refused without asking the core.

## Behind the link

- The app reads the id (`DeepLink.messageId`, `apps/NiceMailer/Sources/NiceMailer/DeepLink.swift`): scheme and `m` in any case, a final slash allowed, digits only.
- It asks the core where the message is: `locate_message {message_id}` answers `{message_id, thread_id, folder_id, account_id}`, or the error `1001` (not found). `thread_id` is `null` for a message not linked to a conversation yet.
- It shows the thread as a search result does (`AppModel.show`): in the unified view, the folder's role for all accounts; otherwise the folder; the thread stays selected even if it is not in the pages of the list loaded so far.

```sh
nicemailer call locate_message '{"message_id": 48213}'
```

## Tests

- `crates/nm-daemon/tests/deeplink_rpc.rs`: `locate_message` through the socket, a missing message, a malformed request.
- `DeepLinkTests`: the URL of a message, reading it back, URLs refused, the Markdown link with brackets, line breaks and an empty subject.
- `GeekControllerTests`: the first row of `l` copies the Markdown link (Return) and the URL (⌘-Return) to a private pasteboard; a link opens the mail in its folder, closes the panels and, in one column, shows the mail; a missing mail and a foreign URL are reported.
- `deeplinks.uiscript`: in the app, copy the link of the newsletter, go to another folder with the help open, open the link (`openurl clipboard`): the newsletter is back, in Archive; the same in one column; a missing mail and a foreign URL.
- The app of `NiceMailer.app` can play the scenarios: `NICEMAILER_UI_BIN=~/Applications/NiceMailer.app/Contents/MacOS/NiceMailer scripts/ui-test.sh deeplinks.uiscript` (it finds its fonts and translations in `Contents/Resources`, `resources_bundle`).

Launch Services itself (a click in Obsidian reaching the app) is not covered by the automated tests: it would open the app on the user's screen.
