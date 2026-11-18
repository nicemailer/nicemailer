# Changelog

What changes for the users of NiceMailer, release by release. `make release` publishes the section of the version it builds as the notes of the GitHub release (`docs/app-bundle.md`, "Release notes").

How to write it:

- Each change that users can notice adds its entry under `## Unreleased`, in the same commit as the change. A change users cannot notice (refactoring, tests, documentation for developers) adds none.
- One line per entry, written for users: what they can now do or what no longer goes wrong, not how the code changed.
- Groups, in this order, only those that have entries: `### Added`, `### Changed`, `### Fixed`, `### Removed`.
- At a release, the commit that sets the new `version` in `Cargo.toml` renames `## Unreleased` to `## <version> — <YYYY-MM-DD>` and starts a new, empty `## Unreleased` above it.

## Unreleased

### Added

- `nicemailer core stop` and `nicemailer core start` stop and start the background core, and return once it is done.
- Every key of the app can be changed in `~/.config/nicemailer/keymap.toml`, created with all the default keys commented out: keys moved or removed, keys of your own for any folder or action, menus of your own. The help, the status bar and the menus show the keys in effect; `nicemailer keymap check` validates the file (docs/keymap.md).
- NiceMailer is available in German, Spanish, Italian, Portuguese (Brazil), Dutch, Swedish, Danish, Norwegian, Finnish, Polish, Russian, Ukrainian, Japanese, Simplified and Traditional Chinese and Korean, following the language of macOS or the one chosen for the app. These translations have not yet been reviewed by native speakers.
- NiceMailer updates itself: it checks for a new version once a day, or at once with NiceMailer > Check for Updates…, and installs it with its background core.
- While NiceMailer is updated or upgrades its mailbox, the app, the `nicemailer` command and AI agents say so and wait, instead of reporting that NiceMailer is not running. `nicemailer --wait` waits for the end; without it, the command exits with status 75.
- The mailbox is copied before each upgrade of its data format, and put back as it was if the upgrade fails: nothing is lost, and the previous version of NiceMailer can still open it.
- Before an account is added, the Preferences show what it will change on the server: the folders NiceMailer will create, each role's folder changeable to one the server already has.
- An account can be added "only to look": NiceMailer shows its mail and changes nothing on the server. "Organize…" on its page (or `nicemailer account activate`) later creates the folders, archives the old mail and starts screening, without downloading the mail again.
- The open mail shows its tags and the groups of its sender after the subject.
- `t` and `g` offer to remove a tag or a group, for one mail or a selection: each action has its own row, and with several mails each row says how many of them have it.
- Tag… in the attribute selector (`f`): the list narrowed to the mails with one of the chosen tags, or without them.
- "without tag" in Tag… and "without group" in Group…: the mails with no tag, or of nobody in a group.
- Rename (`r`) or delete (`d`) a tag or a group from the Tags (`;T`) and Groups (`;g`) screens. Deleting a tag takes it off all its mails, on the server too.
- Preferences (Settings…, ⌘,) with an Accounts tab: add a mail account from its address, password and name, NiceMailer finding its servers and asking for more only when needed; the accounts listed, and their order changed.
- When the SMTP server refuses the mailbox password, `nicemailer account add` keeps it and says to save its own password, instead of creating the account without a server to send from.
- The Tags screen shows one row per tag name, its mails counted in every account.
- Contact groups as a filter: Group… in the attribute selector (`f`), and the groups listed under the folders in three columns; a click filters the list, and the active groups are highlighted like the current folder.

### Changed

- The attribute selector (`f`) lists its rows in pairs, a thin line between them: Unread / Read, Flagged / Not flagged, From… / To…, Subject… / Text…, the two dates, Group… / Tag…, Human / Robot, attachments, your reply, conversations. "Answered" is now "With my reply".

### Fixed

- Sizes and large numbers are written in the app's language and region ("251 ko", "1 234"), and the counts of the Preferences take their singular ("1 mail").
- A date typed in the attribute selector is read in the order of the Mac's region (month first in the United States), and the lists show dates in that same order.
- The quoted history at the end of a reply is folded in many more languages: German as Gmail and Apple Mail write it, Portuguese, Dutch, Swedish, Danish, Norwegian, Polish, Russian, Chinese and Japanese, and the Outlook headers and forwards of those languages.
- Dates follow the language of the interface and the Mac's region settings, in the app and in the line introducing a quotation of a reply: an English interface no longer shows French month names, and the 12- or 24-hour clock chosen in System Settings is respected.
- Counts read correctly in English and French: "1 mail", "1 attachment saved", and in French "0 mail" (no more "1 mails" or "(and 1 others)").
- The classic theme and the connection errors are translated: they were shown in French whatever the language.
- Errors reported by NiceMailer (a tag name taken, a password refused, a mail not sent, a change the server refused, an AI that failed, an account that cannot connect…) are written in the app's language; the words of a mail server or of the AI are quoted as they came.
- The "answered" attribute finds the conversations you replied to from NiceMailer or from a mail client that does not mark the mail as answered: a reply of yours in Sent is enough.
- Replying from NiceMailer marks the mail as answered, also on the server, so other mail clients show it.

- The attribute selector (`f`) shows all its rows at once, without scrolling.
- The tag picker (`t`) lists the tags of every account, not only those of the mail's account, which left it empty for a mail of another account.
- `g` adds the person the conversation is with, no longer yourself when your reply was filed with it; your own addresses can no longer be in a group, and those already there are removed.

- The app no longer stays without its core after a new version, signed differently, replaces the installed one.

### Removed

- NiceMailer > Stop the Core: the background core is now stopped with `nicemailer core stop`.
- File > New Window and window tabs: a second window only mirrored the first one and reconnected to the core. Windows with their own folder and selection will come back later.

## 0.1.0 — 2026-11-16

First release, for testers. Apple Silicon Macs only, macOS 14 or later. No automatic updates: to update, download the new version and replace the app in Applications.

### Added

- IMAP accounts: servers found from the address, initial import of the mailbox, synchronization in both directions, new mail pushed by IDLE.
- A core that keeps running in the background when the app is closed: mail keeps arriving and being sorted, pending sends leave.
- New mail screened at reception: people and robots told apart, sender authentication checked.
- A folder workflow driven by the keyboard, with a decision on each sender and undo.
- Simplified display of conversations: quotes, signatures and copies hidden.
- Replies with a text or formatted editor, sent through SMTP with an undo delay.
- AI assistance through your own `claude` command-line tool.
- Import of a whole mailbox from an mbox file.
- Links to a mail (`nicemailer://m/<id>`) to paste in notes, for example in Obsidian.
- The `nicemailer` command-line client, inside the app (`Contents/Helpers/nicemailer`).
- English and French.
