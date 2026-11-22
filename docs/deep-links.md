# Deep links to a mail

Each mail has its own URL, the same on every Mac that has the mail:

```
nicemailer://m/3f9a0c2b7e41
```

`m` stands for "message", and `3f9a0c2b7e41` is the mail's **link key**: 12 hexadecimal characters taken from its `Message-ID`, so they depend only on the mail, not on the database that stores it (below, "How long a link lives"). Clicked anywhere on the Mac (a note, a task, a calendar event), the link opens NiceMailer on that mail: in its folder, selected and shown, with any open panel closed; in one column, the mail fills the window, as after Return.

## Copying the link of a mail

The first row of the links panel (`l`) is **Link to this mail**, with its URL on the right:

- Return copies a Markdown link, `[subject](nicemailer://m/3f9a0c2b7e41)`, ready to paste in Obsidian or any Markdown note: the note shows the subject, a click opens the mail. Brackets in the subject are escaped (`[\[Spam\] Offre](…)`).
- ⌘-Return copies the bare URL.

The status bar says what was copied. Typing `obsidian`, `markdown` or `deeplink` in the panel also finds the row.

A conversation holds several mails; the link names the **message the reader is on**: move up and down the conversation with ctrl-k / ctrl-j (or click a header), then `l`, and the link is that message's, as `r` and the exports take it. Without a choice, it is the latest received message, so that it opens the conversation in its folder (Inbox, Archive…) rather than in Sent.

Opening the link shows that very message: its conversation is selected, the message becomes the current one (ctrl-k / ctrl-j go on from it), unfolded even if the reader folds it by default, and the reader scrolls to it. A link to one of your own messages (in Sent or Drafts) shows its conversation where it is filed, the folder of its latest received message, not Sent; Sent only when the conversation has nothing else. Return on a mail of the History screen shows its message the same way, in the message's own folder.

## Use with Obsidian

1. Read the mail in NiceMailer, press `l`, then Return.
2. Paste in the note (⌘V): `[Devis toiture](nicemailer://m/3f9a0c2b7e41)`.
3. Later, a click on the link in the note (⌘-click in edit mode) opens the mail in NiceMailer.

Obsidian opens links of other apps through macOS, which needs NiceMailer to be installed as an app (below).

## For AI agents

The MCP server (`docs/mcp.md`) gives each message its `link`, so that an agent can point back at a mail in what it writes. `get_message` also takes a `link` instead of `message_id`: paste a link from a note into the conversation, and the agent reads that mail.

## In calendar events

The calendar files of the AI's "Dates and appointments" (`docs/ai.md`) carry the link of their mail, as the event's URL and the last line of its notes: from Calendar, a click goes back to the mail that set the appointment.

The contact cards of the AI's "Contact card" carry it too, in their note: from Contacts, the card leads back to the mail it was made from.

## Installing the app for links

macOS gives a URL only to an app bundle that declares its scheme. `make app` (`swift run`) starts a bare executable, which macOS cannot reach, so the links need:

```sh
make bundle
```

It builds the app in release mode, wraps it in `NiceMailer.app` (its `Info.plist` declares the `nicemailer` scheme, and why the app reads Contacts: `docs/geek-ui.md`, "Senders in the address book"), signs it, installs it in `/Applications` with its core ([app-bundle.md](app-bundle.md)) and registers it with Launch Services. Run it again after pulling new code.

`make open` does the same, then launches the app: once built, it quits the installed app if it runs (a SIGTERM, waiting 5 s at most; it keeps nothing unsaved, the core holds the data), installs the new one and opens it. An app started by `make app` is left alone. With the core in another terminal, the two commands to run the app with links are:

```sh
NICEMAILER_DATA_DIR="$HOME/Library/Application Support/NiceMailer-archive" make core
NICEMAILER_DATA_DIR="$HOME/Library/Application Support/NiceMailer-archive" make open
```

`make app` stays the quickest way to run a debug build, its output in the terminal, but a link clicked meanwhile launches the installed app as well, in a second window on the same core.

An app opened by a link or by the Finder gets no variable from a shell. To use another data directory, give it when building: `NICEMAILER_DATA_DIR=/other/directory make bundle` writes it into the app (`LSEnvironment`). The core must be running, as for `make app`.

If the app is not running, the link launches it, and the mail opens once the app is connected to the core. If it is running, the link goes to its window (no second window opens).

## How long a link lives

The link names the mail, not a row of the database. Its key is the first 48 bits of the SHA-256 of the mail's `Message-ID` (`nm_store::link_key`; `specifications/modele de donnees.md`, `link_key`), written as 12 lowercase hexadecimal characters. So the link:

- opens the mail on **another Mac** that has it, whatever number its database gives the mail, whether it came there by IMAP or from an mbox export (whose bytes differ);
- survives moving the mail to another folder, and a **rebuilt or re-imported** database;
- keeps the same value from the headers on: a mail whose body is not downloaded yet already has it, and evicting its raw file later does not change it.

The same mail often has several rows: received on two accounts, its copy in Sent, copies in several folders. The link opens, in this order, a row in a conversation, out of Deleted, Junk, Banned and Quarantine, out of Sent and Drafts, and the latest received.

Limits:

- A mail **without `Message-ID`** (about 1 % of a real mailbox, mostly old or automated mail) takes the first 48 bits of the hash of its raw file instead. That key is the same on two Macs only if both got the mail from the same source (both by IMAP from the same server, say). Until its body is downloaded it has no key at all, and its link is its id: `nicemailer://m/48213`, valid on this database only.
- A sender that reuses a `Message-ID` for different mails (rare, a misconfigured server) makes them share a link, which opens one of them.
- 48 bits make an accidental clash between two different mails negligible: about one chance in a billion per link for 300,000 mails. The length is fixed on purpose: a prefix shortened until it is unique in one database (as Git does with commit hashes) may not be unique in another one.

**Old links** carry the message's id, `nicemailer://m/48213` (1 to 11 digits): they still open the mail on the database that made them, and break if the mail is deleted or the database rebuilt. The app then says "This mail is no longer in NiceMailer". A key no mail of this database has says "This mail is not in NiceMailer on this Mac". In both cases the app stays where it was. A URL that is not a mail link (`nicemailer://x/1`, a key of 11 characters, letters in an id) is refused without asking the core.

## Behind the link

- The core writes the link of each message of a thread (`MessageView.link`, `nm_api::deep_link::url`); the app copies it as it is (`DeepLink.url(of:)`), and falls back on the id only if the core gave none.
- The app reads the link (`DeepLink.target`, `apps/NiceMailer/Sources/NiceMailer/DeepLink.swift`): scheme, `m` and key in any case, a final slash allowed; exactly 12 hexadecimal characters for a key, 1 to 11 digits for an old id.
- It asks the core where the message is: `locate_message {link_key}` (or `{message_id}` for an old link) answers `{message_id, thread_id, folder_id, account_id}`, or the error `1001` (not found). `thread_id` is `null` for a message not linked to a conversation yet. The column `message.link_key` is indexed (`idx_message_link`): the lookup is an equality.
- It shows the thread as a search result does (`AppModel.show`): in the unified view, the folder's role for all accounts; otherwise the folder; the thread stays selected even if it is not in the pages of the list loaded so far.
- Then the message: the reader state of that thread makes it current and unfolded (`ThreadDisplay.focus`), and the reader scrolls to it once laid out (`InterfaceController.showDeepLinkedMail`, `pendingMessageScroll`, as for ctrl-k / ctrl-j). The link copied is the one of `ThreadDisplay.linkMessage`: the current message, else the latest received.

```sh
nicemailer call locate_message '{"link_key": "3f9a0c2b7e41"}'
nicemailer call locate_message '{"message_id": 48213}'
```

## Tests

- `crates/nm-store/src/link_key.rs`: the key of a Message-ID against a reference value (`shasum -a 256`), the raw file's key without one.
- `crates/nm-store/tests/link_key.rs`: the link of a reply in Sent shows its conversation where it is filed; two databases that number a mail differently (IMAP headers first on one, an mbox export with other bytes on the other) give it the same link; a mail without Message-ID gets its key once downloaded and keeps it; the copy a link opens among several.
- `crates/nm-store/tests/migrations.rs`: version 15 computes the keys of the mail already stored.
- `crates/nm-daemon/tests/deeplink_rpc.rs`: `locate_message` through the socket by id and by key (any case), the link of `get_thread`, a missing message or key, malformed requests.
- `ThreadDisplayTests`: the message linked (the current one, else the latest received, not one of Sent), and a link opened on a message unfolding it.
- `DeepLinkTests`: the link the core wrote and the fallback on the id, keys and old ids read back, URLs refused, the Markdown link with brackets, line breaks and an empty subject.
- `InterfaceControllerTests`: the first row of `l` copies the Markdown link (Return) and the URL (⌘-Return) to a private pasteboard; a link opens the mail in its folder, closes the panels and, in one column, shows the mail; the link copied is the key the core gives; a link by key asks the core by key; up the conversation with ctrl-k, the link is that message's; opening the link of an older message makes it current and unfolded; a missing mail, a key this Mac does not have and a foreign URL are reported.
- `deeplinks.uiscript`: in the app, copy the link of the newsletter, go to another folder with the help open, open the link (`openurl clipboard`): the newsletter is back, in Archive; the same in one column; up "Chantier" to my reply, its link copied, the reader moved away and another folder shown: the link brings back that reply, current and unfolded, in Archive where the conversation is filed; a missing mail (old link), an unknown key and a foreign URL.
- The app of `NiceMailer.app` can play the scenarios: `NICEMAILER_UI_BIN=/Applications/NiceMailer.app/Contents/MacOS/NiceMailer scripts/ui-test.sh deeplinks.uiscript` (it finds its fonts and translations in `Contents/Resources`, `resources_bundle`).

Launch Services itself (a click in Obsidian reaching the app) is not covered by the automated tests: it would open the app on the user's screen.
