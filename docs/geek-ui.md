# The geek theme

The geek theme is a keyboard-first interface designed like a code editor: no rounded zones, no scroll bars, no animations, a compact status bar, and menus in the style of [Helix](https://helix-editor.com). The specification (in French) is `specifications/ui.md`.

Switch themes with the **Theme** menu (Classic, Design, Geek). Design does not exist yet and shows the classic interface. The choice is remembered.

## Layout

- **Mail list** on the left (about a third of the window; drag the separator to resize), **reading pane** on the right.
- `v1` shows the list only, `v2` the list and the mail, `v3` adds the folders on the left, in a single block titled with the current view: **all** or the account's name.

## Accounts and the unified view "all"

With several accounts, the app opens on **all**, the unified view: each folder (Inbox, Archive, Todo…) gathers the folders of the same role in every account, with their unread counts added up, and lists their mails together, newest first. Choose another view with `@` (or a click on the account in the status bar or on the title of the folder column): **all** comes first, then each account. Changing view keeps the folder role (Archive to Archive), or falls back to the Inbox.

The `;` menu and the folder column only show the folders of the current view: the account is chosen with `@`. At the foot of the list, the virtual folder **Everything** (`;e`) lists every folder of the view together (all accounts in "all"), to search everywhere with the attributes; a conversation present in several folders (Inbox and Sent) appears once, in the folder of its latest mail. Since the list mixes every folder, each row shows its folder in a muted column before the message count, and the reader shows it at the right end of the subject line. In "all", the account follows the folder ("Archive · perso"): always in the reader, and in the list only when the subject keeps at least 30 characters (a wide list, one column), otherwise the folder alone. Moves and other actions apply to each mail in its own account.
- The **status bar** shows the mode (`NOR`, or the prefix waiting for its second key, such as `m`), then the account (`@` icon) and the folder (folder icon), each followed by the key that opens it: `@` for the account picker, `;` for the menu of folders and screens. A click on either opens the same; the rows of a menu can be clicked too. While a tag, group or contact is listed, it shows that instead. Messages (result of an action, errors) appear on the right and disappear at the next key. A move names the mail and the folder: “Devis toiture” moved to Archive (the subject is cut at 50 characters); several mails give their number instead: 3 threads moved to Archive. `vs` moves it to the top, hides it, or brings it back to the bottom.
- When a list opens, its first mail is selected, so the reading pane always shows something.

Layout choices (columns, status bar position, list width) are remembered.

## Text size

As in Zed, **⌘+** (or **⌘=**, which needs no Shift on AZERTY keyboards) makes the whole interface bigger, **⌘−** smaller, and **⌘0** brings it back to the default size, one point at a time between 9 and 24 points (default 12). The same commands are in the View menu. Everything follows a single base size: fonts, row heights, column widths, menus, pickers and the HTML view. The size is remembered.

## Attributes (filters)

The magnifier after the folder in the status bar (key **F**, or a click) opens the attribute selector. Attributes narrow the current list (folder of an account, or of "all"), combined with AND; a thread qualifies when its mails in the folder match.

| Attribute | Values |
| --- | --- |
| read state | unread / read |
| flag | flagged / not flagged |
| attachments | with / without |
| answered | answered (you replied) / not answered |
| conversation | conversation (several mails) / single mail |
| From… | part of a name or address, contacts suggested as you type; several values OR-ed |
| To… | same, on To and Cc |
| Contains… | words of the full text (subject, sender, recipients, body), as a prefix, accents ignored |
| Date… | today, last 7 days, last 30 days, this year, older than a year, or "after" / "before" a typed date (2026-10-01, 1/10/2026) |

Choosing an active value again removes it; choosing the opposite value replaces it. An attribute with "…" expands the panel at once to choose its value. Active attributes are chips in the status bar: a click on a chip removes it; Escape clears them all (after the marks); they stay when changing folder or account.

## Search (`/`)

`/` is the quick way to search everywhere: it opens the same panel as `F` → Contains… (titled "Search everywhere"); Return goes to **Everything** of the current view (every folder, every account in "all") with the words as the Contains… attribute. The result is the ordinary list: one row per conversation, with its folder, the `contains:` chip, and `F` to narrow it further (unread, From…). Escape clears the attributes and goes back to the folder `/` was pressed from; going elsewhere first (`;`, `@`, a click on a folder) forgets that folder, and Escape then only clears the attributes. A second `/` replaces the words and keeps the folder to go back to.

## Reading a mail

The whole theme (lists, status bar, menus, mails, including the HTML view) uses **iA Writer Mono V**, a font bundled with the app (SIL Open Font License). List columns have fixed widths counted in characters of this monospaced font, computed from the list width, so they stay aligned when the window is resized. Mails are shown as plain text, simplified: each message shows what it brings, the history it repeats is hidden, older read messages are folded (see `thread-display.md`). The `text | html` switch at the top of the mail, or `vh`, shows the sanitized HTML version instead (scripts and remote images are removed by the core). The choice is remembered **for the sender**: the next mails from the same address open the same way.

### Attachments

At the foot of each unfolded mail, its attachments are tiles: the icon of the file type (an SF Symbol in the theme's colours: image, PDF, text, spreadsheet, slides, archive, audio, video, code, calendar, contact, mail, other file), the name cut in the middle to keep its extension, and the size. A row holds 3 to 5 tiles, depending on the width of the reading pane. A folded mail shows a paperclip and the number of its attachments.

- A click saves the file to `~/Downloads`, never overwriting a file (`devis (2).pdf`); the status bar says where it went.
- ⌘-click saves it the same way, then opens the saved copy with its default app, in front.
- `l` lists the links of the mail, then one row per attached file, with its icon; Return saves the file, ⌘-Return saves it and opens the saved copy at once, in front.
- `sa` saves every attachment of the marked mails (or of the one under the cursor), `sg` all but the graphics.

Images the HTML of the mail shows (`cid:`) are part of the body, not tiles. An image marked inline that no HTML shows (Apple Mail puts photos in a plain text mail this way) is a tile. The tiles follow the body in the HTML view too.

Files are extracted by the core from the raw message on first use (`docs/core-library.md`, "Attachments"). Generated mail (`make seed`) has no raw message: its tiles show, but saving one reports that the content is not available.

### Unsubscribing

When the latest received mail of the conversation says how to unsubscribe from its list, a button appears on the title line, left of `text | html` (never in Junk, Quarantine or Banned: unsubscribing from a spam confirms the address to the spammer). Its look says where the link comes from and what a click does:

| Button | Link | A click (or ⌘U) |
| --- | --- | --- |
| `unsubscribe`, accent colour | `List-Unsubscribe` header, one click (RFC 8058) | the core unsubscribes at once; the status bar confirms, and the button gives way to "unsubscribed <date>" |
| `unsubscribe` with ↗, accent colour | header, a page of the list | opens the page in the browser |
| `unsubscribe` with an envelope, accent colour | header, an address only | opens the unsubscribe mail in Mail (until NiceMailer sends mail) |
| `unsubscribe` with a link symbol, muted | a link found in the text of the mail | opens it in the browser |

A one-click attempt the server refuses opens the page instead, and says why. The tooltip of the button tells the same. Only a one-click unsubscribe is recorded ("unsubscribed <date>"): after a page or a mail, the app cannot know whether you finished. The sender's decision and its mails are not changed. The links are found by the core when the mail arrives (`docs/core-library.md`, "Unsubscribing from a list").

### When a mail becomes read

- A mail you select (`j` `k`, a click, the next mail after a move) is marked read once shown for one second. Browsing quickly with `j` / `k` marks nothing.
- The mail selected for you, the first one when a list opens or the one that replaces a mail that left the list, is not marked read by itself. It is marked read when you move from it to another mail of the list after at least one second. Moving it (`d`, `m`) or changing folder does not mark it.
- In a filtered list (unread, flagged…), the selected mail that stops matching (read after one second, `u`, `*`) stays under the cursor, shown with its new state, until you leave it; it then leaves the list. As in Gmail's unread view: without this, a list of unread mails would empty itself, one mail per second.

## Keys

Actions apply to the **marked** mails if any (`J` / `K`), otherwise to the mail under the cursor.

| Key | Action |
| --- | --- |
| `j` `k` (or arrows) | next / previous mail |
| `ctrl-d` `ctrl-u` | half a page down / up in the list, as in Zed: the cursor moves by half the rows that fit in the list, and stops on the first or last mail |
| `J` `K`, shift-↓ shift-↑ | mark the mail and move (multiple selection) |
| `F` | attribute selector (filters on the current list) |
| `@` | account picker: **all** (unified view) or one account, keeping the folder role (Archive to Archive), or its Inbox |
| `l` | links of the mail, then its attached files; Return opens a link in the browser, or saves a file to Downloads; ⌘-Return saves the file and opens it |
| `d` | done: move to Archive |
| `m` + letter | move to a folder (`mt` Todo, `md` Deleted…); only allowed targets are offered |
| Delete | move to Deleted |
| `t` | tag picker: choose a tag, or type a new name; the same tag again removes it |
| `g` | group picker: adds the sender to a group (new name: the group is created) |
| `T` | show everything in the thread (folded mails, quotes, signatures) or go back to the simplified view |
| `S` | skip to the text after the next quoted block |
| `u` | read / unread |
| ⌘U | unsubscribe from the list of the mail (see "Unsubscribing") |
| `*` | flag |
| `/` | search everywhere: Contains… in Everything (see [Search](#search-)); Escape goes back |
| `;` + key | go to a folder or a screen (see below) |
| `s` + key | save to `~/Downloads`, for every marked conversation (or the one under the cursor): its latest mail with `se` .eml, `st` text, `sm` Markdown; its attachments with `sa` all of them, `sg` all but the graphics (images: logos, signatures, photos). For attachments every message counts, and a file attached again to a reply is saved once. No file is overwritten |
| `v` + key | display: `v1` `v2` `v3` columns, `vc` sort by reception, `vm` sort by modification, `vh` text / HTML, `vs` status bar |
| Space, shift-Space | scroll the mail a page down / up, as text or as HTML |
| `ctrl-f` `ctrl-b` | same, a page down / up in the mail |
| `?` | list of keys |
| Escape | cancel a prefix, close a panel, unmark, then go back from a tag, group or contact list to the folder |
| `r` `R` `f` | reply, reply all, forward: not available yet (writing mail needs SMTP) |

After a prefix (`;`, `m`, `s`, `v`), a menu lists the possible second keys, as in Helix. Each prefix has a block in the status bar: the folder for `;` (folders, Everything, screens), and an icon with its key for `m` (move, arrow), `s` (save, tray) and `v` (display, eye). The blocks form three groups set apart by a wide gap: what the list shows (`@`, `;`, `F` and the chips), the display preference (`v`), and actions on the current mail (`m`, `s`). The menu drops down from its block: under it when the bar is at the top, above it when the bar is at the bottom; a click on the block opens it, and its rows can be clicked. The arrows highlight a row (down starts on the first one, up on the last, both turn around) and Return goes to it; a key still acts at once, and Return with nothing highlighted closes the menu. With the status bar hidden, menus appear in the bottom-right corner.

### Mouse

The keyboard comes first, but every list can also be used with the mouse:

| Where | Click | Other |
| --- | --- | --- |
| Mail list | select the mail, unmark the others | ⌘-click marks or unmarks a mail (the current one joins the selection); ⇧-click marks every mail from the current one to the clicked one |
| Folders (3 columns) | go to the folder | |
| Account or folder in the status bar | open the account or folder picker | |
| Pickers and menus | choose the row | |
| Contacts, tags, groups | select the row | double-click lists its mails |

A click acts at once: double-clicks are detected without delaying single clicks.

### Folders and screens (`;`)

The menu lists the folders in blocks separated by a blank line, the folder column too: to deal with (Inbox, Todo, Waiting, Quarantine), filed (Archive, Newsletter, Paperwork), the user's own folders, written (Sent, Drafts), thrown away (Deleted, Junk, Banned); then Everything, the screens Contacts, Tags, Groups, and History on its own. The unread count of each folder is aligned at the right end of its row.

| Key | Goes to |
| --- | --- |
| `;i` `;t` `;w` `;q` | Inbox, Todo, Waiting, Quarantine of the current view |
| `;a` `;n` `;p` | Archive, Newsletter, Paperwork |
| `;s` `;r` | Sent, Drafts |
| `;d` `;x` `;b` | Deleted, Junk, Banned |
| `;e` | Everything: every folder of the view |
| `;c` | Contacts: every known address, with its decision, groups and number of mails; `/` filters; Return lists the contact's mails |
| `;T` | Tags; Return lists the mails of the tag (uppercase: `;t` is Todo) |
| `;g` | Groups; Return lists the mails of the group's members |
| `;h` | History: the audit log, newest first |

On a screen, `j` `k` move and Escape goes back to the mails.

### Pickers

Pickers (`@`, `l`, `t`, `g`, `F`) are filterable lists: type to filter (every word typed must appear, in any order, ignoring case and accents), `ctrl-n` / `ctrl-p` or the arrows to move, Return to choose, Escape to close. A picker opens on its first row and turns around: up (or `ctrl-p`) from the first row selects the last one, so the end of the list is one key away, and down (or `ctrl-n`) from the last row comes back to the first.

## Not available yet

- The "edition" context: replying, forwarding, writing a new mail.
- Saving attachments (`sa`, `sg`) and saving as PDF (`sp`).
- A dark/light override independent of macOS (the palette follows the system appearance).

## Implementation notes

- Keys are read by an AppKit event monitor (`GeekController.handle`) before any view, so a key is never lost to a focused field: picker text is edited by the controller itself.
- All user-facing strings go through `L(...)` and the String Catalog `Resources/Localizable.xcstrings` (English and French). The language is chosen explicitly from the user's preferences, because an app launched with `swift run` has no main bundle declaring its languages.
- Rows of lazy lists receive plain values (cursor, marks) as parameters: reading the observable model only inside a `ScrollView` closure did not always refresh them.
- Clicks use tap gestures filtered by modifier (`View.onClick`): SwiftUI runs tap actions after the click was processed, when `NSApp.currentEvent` no longer describes it.
- Tested by `Tests/NiceMailerTests` (unit) and `UITests/*.uiscript` (scripted UI); see `docs/testing.md`.
