# Changelog

What changes for the users of NiceMailer, release by release. `make release` publishes the section of the version it builds as the notes of the GitHub release (`docs/app-bundle.md`, "Release notes").

How to write it:

- Each change that users can notice adds its entry under `## Unreleased`, in the same commit as the change. A change users cannot notice (refactoring, tests, documentation for developers) adds none.
- One line per entry, written for users: what they can now do or what no longer goes wrong, not how the code changed.
- Groups, in this order, only those that have entries: `### Added`, `### Changed`, `### Fixed`, `### Removed`.
- At a release, the commit that sets the new `version` in `Cargo.toml` renames `## Unreleased` to `## <version> — <YYYY-MM-DD>` and starts a new, empty `## Unreleased` above it.

## Unreleased

### Added

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

- The "answered" attribute finds the conversations you replied to from NiceMailer or from a mail client that does not mark the mail as answered: a reply of yours in Sent is enough.
- Replying from NiceMailer marks the mail as answered, also on the server, so other mail clients show it.

- The attribute selector (`f`) shows all its rows at once, without scrolling.
- The tag picker (`t`) lists the tags of every account, not only those of the mail's account, which left it empty for a mail of another account.
- `g` adds the person the conversation is with, no longer yourself when your reply was filed with it; your own addresses can no longer be in a group, and those already there are removed.

- The app no longer stays without its core after a new version, signed differently, replaces the installed one.

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
