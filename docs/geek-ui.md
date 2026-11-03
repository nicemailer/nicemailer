# The geek theme

The geek theme is a keyboard-first interface designed like a code editor: no rounded zones, no scroll bars, no animations, a compact status bar, and menus in the style of [Helix](https://helix-editor.com). The specification (in French) is `specifications/ui.md`.

Switch themes with the **Theme** menu (Classic, Design, Geek). Design does not exist yet and shows the classic interface. The choice is remembered.

## Layout

- **Mail list** on the left (about a third of the window; drag the separator to resize), **reading pane** on the right.
- `v1` shows the list only, `v2` the list and the mail, `v3` adds the folders on the left, in a single block titled with the current view: **all** or the account's name.

## Accounts and the unified view "all"

With several accounts, the app opens on **all**, the unified view: each folder (Inbox, Archive, Todo…) gathers the folders of the same role in every account, with their unread counts added up, and lists their mails together, newest first. Choose another view with `@` (or a click on the account in the status bar or on the title of the folder column): **all** comes first, then each account. Changing view keeps the folder role (Archive to Archive), or falls back to the Inbox.

The folder picker (`h`), the `;` menu and the folder column only show the folders of the current view: the account is chosen with `@`. Moves and other actions apply to each mail in its own account.
- The **status bar** shows the mode (`NOR`, or the prefix waiting for its second key, such as `m`), then the account (`@` icon) and the folder (folder icon), each followed by the key that opens its picker (`@` for accounts, `h` for folders). A click on either opens the same picker. While a tag, group, contact or search is listed, it shows that instead. Messages (result of an action, errors) appear on the right and disappear at the next key. `vs` moves it to the top, hides it, or brings it back to the bottom.
- When a list opens, its first mail is selected, so the reading pane always shows something.

Layout choices (columns, status bar position, list width) are remembered.

## Text size

As in Zed, **⌘+** (or **⌘=**, which needs no Shift on AZERTY keyboards) makes the whole interface bigger, **⌘−** smaller, and **⌘0** brings it back to the default size, one point at a time between 9 and 24 points (default 12). The same commands are in the View menu. Everything follows a single base size: fonts, row heights, column widths, menus, pickers and the HTML view. The size is remembered.

## Reading a mail

The whole theme (lists, status bar, menus, mails, including the HTML view) uses **iA Writer Mono V**, a font bundled with the app (SIL Open Font License). List columns have fixed widths counted in characters of this monospaced font, computed from the list width, so they stay aligned when the window is resized. Mails are shown as plain text, simplified: each message shows what it brings, the history it repeats is hidden, older read messages are folded (see `thread-display.md`). The `text | html` switch at the top of the mail, or `vh`, shows the sanitized HTML version instead (scripts and remote images are removed by the core). The choice is remembered **for the sender**: the next mails from the same address open the same way.

## Keys

Actions apply to the **marked** mails if any (`J` / `K`), otherwise to the mail under the cursor.

| Key | Action |
| --- | --- |
| `j` `k` (or arrows) | next / previous mail |
| `J` `K` | mark the mail and move (multiple selection) |
| `h` | folder picker |
| `@` | account picker: **all** (unified view) or one account, keeping the folder role (Archive to Archive), or its Inbox |
| `l` | links of the mail; Return opens one in the browser |
| `d` | done: move to Archive |
| `m` + letter | move to a folder (`mt` Todo, `md` Deleted…); only allowed targets are offered |
| Delete | move to Deleted |
| `t` | tag picker: choose a tag, or type a new name; the same tag again removes it |
| `g` | group picker: adds the sender to a group (new name: the group is created) |
| `T` | show everything in the thread (folded mails, quotes, signatures) or go back to the simplified view |
| `S` | skip to the text after the next quoted block |
| `u` | read / unread |
| `*` | flag |
| `/` | search: type, Return keeps the results, `j` `k` move in them, Return opens a mail, Escape clears |
| `;` + key | go to a folder or a screen (see below) |
| `s` + key | save the latest mail of the conversation to `~/Downloads`: `se` .eml, `st` text, `sm` Markdown |
| `v` + key | display: `v1` `v2` `v3` columns, `vc` sort by reception, `vm` sort by modification, `vh` text / HTML, `vs` status bar |
| Space, `ctrl-d` `ctrl-u` `ctrl-f` `ctrl-b` | scroll the mail |
| `?` | list of keys |
| Escape | cancel a prefix, close a panel, unmark, then go back from a tag, group or contact list to the folder |
| `r` `R` `f` | reply, reply all, forward: not available yet (writing mail needs SMTP) |

After a prefix (`m`, `;`, `s`, `v`), a menu in the bottom-right corner lists the possible second keys, as in Helix.

### Mouse

The keyboard comes first, but every list can also be used with the mouse:

| Where | Click | Other |
| --- | --- | --- |
| Mail list | select the mail, unmark the others | ⌘-click marks or unmarks a mail (the current one joins the selection); ⇧-click marks every mail from the current one to the clicked one |
| Search results | select the result | double-click opens it in its folder |
| Folders (3 columns) | go to the folder | |
| Account or folder in the status bar | open the account or folder picker | |
| Pickers | choose the row | |
| Contacts, tags, groups | select the row | double-click lists its mails |

A click acts at once: double-clicks are detected without delaying single clicks.

### Folders and screens (`;`)

| Key | Goes to |
| --- | --- |
| `;i` `;t` `;w` `;a` `;s` `;r` `;n` `;p` `;q` `;d` `;x` `;b` | Inbox, Todo, Waiting, Archive, Sent, Drafts, Newsletter, Paperwork, Quarantine, Deleted, Junk, Banned of the current account |
| `;c` | Contacts: every known address, with its decision, groups and number of mails; `/` filters; Return lists the contact's mails |
| `;h` | History: the audit log, newest first |
| `;T` | Tags; Return lists the mails of the tag (uppercase: `;t` is Todo) |
| `;g` | Groups; Return lists the mails of the group's members |

On a screen, `j` `k` move and Escape goes back to the mails.

### Pickers

Pickers (`h`, `l`, `t`, `g`) are filterable lists: type to filter (every word typed must appear, in any order, ignoring case and accents), `ctrl-n` / `ctrl-p` or the arrows to move, Return to choose, Escape to close.

## Not available yet

- The "edition" context: replying, forwarding, writing a new mail.
- Saving attachments (`sa`, `sg`) and saving as PDF (`sp`).
- A dark/light override independent of macOS (the palette follows the system appearance).

## Implementation notes

- Keys are read by an AppKit event monitor (`GeekController.handle`) before any view, so a key is never lost to a focused field: picker and search text is edited by the controller itself.
- All user-facing strings go through `L(...)` and the String Catalog `Resources/Localizable.xcstrings` (English and French). The language is chosen explicitly from the user's preferences, because an app launched with `swift run` has no main bundle declaring its languages.
- Rows of lazy lists receive plain values (cursor, marks) as parameters: reading the observable model only inside a `ScrollView` closure did not always refresh them.
- Clicks use tap gestures filtered by modifier (`View.onClick`): SwiftUI runs tap actions after the click was processed, when `NSApp.currentEvent` no longer describes it.
- Tested by `Tests/NiceMailerTests` (unit) and `UITests/*.uiscript` (scripted UI); see `docs/testing.md`.
