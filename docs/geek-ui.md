# The geek theme

The geek theme is a keyboard-first interface designed like a code editor: no rounded zones, no scroll bars, no animations, a compact status bar, and menus in the style of [Helix](https://helix-editor.com). The specification (in French) is `specifications/ui.md`.

Switch themes with the **Theme** menu (Classic, Design, Geek). Design does not exist yet and shows the classic interface. The choice is remembered.

## Layout

- **Mail list** on the left (about a third of the window; drag the separator to resize), **reading pane** on the right. Each row: the unread dot, the date, the sender, a **paperclip** when the conversation has attachments, the subject and its tags, (in Everything) the folder, the number of messages, the **flag**. The paperclip has no number and comes before the subject: next to the message count at the end of the row, "📎 3" would read as three attachments. The flag is an orange icon at the end of the row, a colour of its own, apart from the unread dot's (`GeekThreadRow`, `RowLayout`: fixed icon columns, so the rows line up). ⌘A marks every mail of the list as shown (the rows loaded so far), for a grouped action; the status bar says how many, Escape unmarks. Only on the mail screen, without a panel open.
- `v1` shows the list only: Return, or a double-click on a row, reads the mail in the whole window and marks it read at once, `j` / `k` go on to the next / previous mail, and Escape goes back to the list. `v2` shows the list and the mail, `v3` adds the folders on the left, in a single block titled with the current view: **all** or the account's name.

## Window

The title bar of the window (its title and the red, orange and green buttons) is hidden by default: the content goes up to the top edge, and the status bar, at the top by default, takes its place. The empty parts of the status bar move the window when dragged and zoom it when double-clicked. Everything else of a window stays: resize by the edges, ⌘W, ⌘M, ⌘Q, full screen with ⌃⌘F.

`ui.title_bar = "shown"` in `config.toml` brings the title bar back; the app applies a change of the file at once (`config_status` gives it, `ConfigChanged` says to read it again). With the status bar hidden (`vs`) or at the bottom and the title bar hidden, the window is moved by the bottom bar, or not at all when no bar is shown: show the title bar, or the status bar, to move it.

## Accounts and the unified view "all"

With several accounts, the app opens on **all**, the unified view: each folder (Inbox, Archive, Todo…) gathers the folders of the same role in every account, with their unread counts added up, and lists their mails together, newest first. Choose another view with `@` (or a click on the account in the status bar or on the title of the folder column): **all** comes first, then each account. Each row shows its unread count at the right end, as in the `;` menu. Changing view keeps the folder role (Archive to Archive), or falls back to the Inbox.

The `;` menu and the folder column only show the folders of the current view: the account is chosen with `@`. The `;` menu and the folder column show four blocks, each ended by a virtual folder that lists folders of the view together (all accounts in "all"): Inbox, Todo, Waiting, then **Work** (`;o`, those three); Archive, Newsletter, Paperwork, Quarantine; Sent, Drafts, then **All Mail** (`;m`, every folder but Deleted, Junk and Banned, Quarantine included, what most searches want); Deleted, Junk, Banned, then **Everything** (`;e`, every folder). The virtual folders are drawn apart from the real ones: a stack icon before the name, in the accent colour (`VirtualFolderLabel`). A conversation present in several folders (Inbox and Sent) appears once, in the folder of its latest mail; in All Mail, a conversation whose latest mail was deleted keeps its place, that of its latest mail left there. Since the list mixes every folder, each row shows its folder in a muted column before the message count, and the reader shows it at the right end of the subject line. In "all", the account follows the folder ("Archive · perso"): always in the reader, and in the list only when the subject keeps at least 30 characters (a wide list, one column), otherwise the folder alone. Moves and other actions apply to each mail in its own account.
- The **status bar** shows the mode: the app's icon in the normal mode, otherwise the mode written (`EDT` while writing a reply, the prefix waiting for its second key such as `m`, `PCK` in a picker, `ASK` for a question), then the account (`@` icon) and the folder (folder icon), each followed by the key that opens it: `@` for the account picker, `;` for the menu of folders and screens. A click on either opens the same; the rows of a menu can be clicked too. While a tag, group or contact is listed, it shows that instead. Messages (result of an action, errors) appear on the right and disappear at the next key. A move names the mail and the folder: “Devis toiture” moved to Archive (the subject is cut at 50 characters); several mails give their number instead: 3 threads moved to Archive. At its right end, a `?` block (a question mark in a circle) opens the list of keys, as the `?` key does; a second click closes it. `vs` moves it to the top, hides it, or brings it back to the bottom.
- When a list opens, its first mail is selected, so the reading pane always shows something.

Layout choices (columns, status bar position, list width) are remembered.

## Text size

As in Zed, **⌘+** (or **⌘=**, which needs no Shift on AZERTY keyboards) makes the whole interface bigger, **⌘−** smaller, and **⌘0** brings it back to the default size, one point at a time between 9 and 24 points (default 12). The same commands are in the View menu. Everything follows a single base size: fonts, row heights, column widths, menus, pickers and the HTML view. The size is remembered.

## Attributes (filters)

The magnifier after the folder in the status bar (key **F**, or a click) opens the attribute selector. Attributes narrow the current list (folder of an account, or of "all", or the mails of a tag, a group or a contact, whose status bar then shows the magnifier and the chips after "tag: …"), combined with AND; a thread qualifies when its mails in the listed folders match (in Everything, every folder: a thread shown in Sent because your reply is its latest mail is still seen with the mail it answers).

| Attribute | Values (English / French rows) | Typed to reach it (English / French) |
| --- | --- | --- |
| read state | unread / read — non lus / lus | `un` `r` — `no` (or `nl`) `l` |
| flag | flagged / not flagged — drapeau / sans drapeau | `f` `nf` — `dr` `sd` |
| attachments | with attachments / without attachments — pièce jointe / sans pièce jointe | `wi` `witho` — `pj` (or `pi`) `spj` |
| answered (you replied) | answered / not answered — répondus / sans ma réponse | `an` `na` — `r` `sm` |
| conversation | conversation (several mails) / single mail — conversations / mails seuls | `co` `si` — `co` `m` |
| From… — De… | part of a name or address, or a domain written `@acme.fr`; contacts suggested as you type; several values OR-ed; `-value` excludes. The name is the contact's, not the display name of each mail | `fr`, `sender` — `de`, `exp` |
| To… — À… | same, on To and Cc | `to`, `cc` — `a`, `cc`, `dest` |
| Text… — Texte… | words of the subject and body, never the addresses; each word a prefix, accents ignored; `-word` excludes | `te` — `te` |
| Subject… — Sujet… | words of the subject only; `-word` excludes | `su`, `title` — `su`, `objet` |
| Conversation date… — Date conversation… | the date of the conversation, that of its latest mail (as shown in the list): today, last 7 days, last 30 days, this year, older than a year, or "after" / "before" a typed date, or newer / older than a typed duration (see "Typed dates" below); the two bounds add up to an interval. The frequent one: it costs nothing | `cd` — `da`, `dc`, `quand` |
| Mail date… — Date mail… | the same choices, on each mail: a conversation from 2016 to 2026 is "before 2020" by its mails of 2016. Heavier for the core (it reads the mails of the interval), hence a criterion of its own; its chips say "mail: …". Both can be combined | `ma` — `dm`, `message` |

Choosing an active value again removes it; choosing the opposite value replaces it. An attribute with "…" expands the panel at once to choose its value. Active attributes are chips in the status bar: a click on a chip removes it; Escape clears them all (after the marks); they stay when changing folder or account.

### Reaching a row from the keyboard

The selector opens on its first row and Return chooses the highlighted row, so what matters is which row comes first once you type. Rows are ranked (`PickerMatch.swift`), best first; rows of the same rank keep the list order:

1. the label starts with the typed text: `read` puts "read" first, before "unread" which only contains it; `lu` puts "lus" before "non lus";
2. each typed word starts a word of the label, in any order: `lus` finds "non lus", `jointe sans` finds "sans pièce jointe";
3. the typed letters are the initials of the label's words (one word of two letters or more): `nf` → "not flagged", `sd` → "sans drapeau", `spj` → "sans pièce jointe", `sm` → "sans ma réponse";
4. each typed word appears anywhere in the label (the former filter, so nothing it found is lost): `lagg` → "flagged".

Case and accents are ignored (`a` finds "À…"). Attributes with a value also have hidden search words, localized (`AttributeField.keywords`): `cc` or `destinataire` finds To… / À…, `expéditeur` or `sender` finds From… / De…. The French labels were chosen so that two letters always reach a row: "De…", "Date conversation…" and "drapeau" differ at the second letter (`da` reaches Date conversation…, listed before Date mail…, which `dm` reaches), and "À…" replaces "Destinataire…", which started like "De…" (and matches the "à :" chip and the "À :" header of a mail).

### Typed dates

The input line of every value panel says what can be typed while nothing is typed (in Date…, which opens on its presets, it was otherwise invisible). The Date… panel takes dates and durations:

- `2026-10-01`, `1/10/2026`, `1/10/26`, `1.10.2026` (day first, as in French);
- **six digits**, read as `ddmmyy` and as `yymmdd`. Each reading that is a real day gives two rows, "after" and "before" that day. When both readings are real days, the panel shows four rows, and each says on its right how the digits were read ("dd mm yy", "yy mm dd"): `010120` is 1 January 2020 or 20 January 2001. One real day gives two rows (`311299`: 31 December 1999; `290223`: 23 February 2029, since 2023 has no 29 February). Both readings on the same day count once (`111111`). Two-digit years below 70 are 20yy, from 70 19yy.

- **a duration**: a number and a unit, by its first letter in English or in French, or its whole word: `d`/`j` day, `w`/`s` week, `m` month, `y`/`a` year (`10j`, `3 s`, `8w`, `6m`, `2 years`). Both languages are understood whatever the app's language, since the letters do not collide. No number means 1 (`m` is one month). The panel offers "newer than 10 days" (French "depuis 10 jours") and "older than 10 days" ("il y a plus de 10 jours"), with on the right the date it gives. Every unit counts back from now by the calendar: `1d` is this time yesterday, `1m` this day last month, so `7d` and `1w` agree.

"after" a day includes that day; "before" it does not.

**Two bounds, an interval.** Each row sets one bound, the lower one ("after", "newer than", the presets but the last) or the upper one ("before", "older than", "older than a year"), and keeps the other: "after 2020-01-01" then "before 2020-07-01" is the first half of 2020, shown as two chips, each removable alone. Choosing a bound again replaces it. A row that would leave no day between the two bounds says "nothing in between" on its right; it can still be chosen (the list is then empty), since the next choice usually moves the other bound.

### Values: domains and exclusions

In the From… and To… panels, a value starting with `@` is a **domain**: `@acme.fr` matches `jean@acme.fr` and its subdomains (`x@mail.acme.fr`), not `x@notacme.fr`; the row says "domain" on its right. Without `@`, a value is any part of a name or address (`acme.fr` also finds `notacme.fr`).

Choosing Text… or Subject… again opens its panel on the current words, selected, as in a macOS field reached with Tab: typing (or ⌘V) replaces them, Return alone keeps them, → (or ←) ends the selection to edit them, for instance to add `-word` after the fact, and Delete then Return removes the criterion ("remove subject: …"). A second Subject… search therefore replaces the first one instead of narrowing it unseen. Text… never looks at the addresses: searching a name in the text does not bring back every mail of that person; From… and To… do that.

### Negation

- **Yes / no attributes**: each has its opposite as a row of its own ("sans drapeau", "sans pièce jointe", "non lus", "sans ma réponse", "mails seuls"), reached by its initials (`sd`, `spj`, `nl`, `sm`). Choosing the opposite of an active value replaces it.
- **From…, To…, Text…, Subject…**: the panel shows the typed value ("+", Return) and, just under it, the same value excluded ("− not …", Down then Return). Typing the value with a leading `-` (`-@acme.fr`) leaves only the exclusion, Return at once; the contacts suggested are then offered as exclusions too.
- In Text… and Subject…, words can be mixed: `renouvellement -facture` wants one word and excludes the other. An exclusion added later joins the words already there.
- **Combination**: the wanted values of an attribute are OR-ed, its excluded values AND-ed ("from alice or bob, but from nobody at acme.fr"); a thread is left out as soon as one of its mails in the listed folders matches an excluded value. A value is wanted or excluded, not both: the last choice wins.
- **Chips**: `not from: @acme.fr`, `not to: @acme.fr`; Text… and Subject… keep the dash of their excluded words (`text: renouvellement -facture`).

Example, at a registrar: the mails that talk about the domain acme.fr, without those sent to or by acme.fr. `f` `te` Return `acme.fr` Return; `f` `de` Return `-@acme.fr` Return; `f` `a` Return `-@acme.fr` Return (French keys; in English `fr` and `to` instead of `de` and `a`). Three chips: `text: acme.fr`, `not from: @acme.fr`, `not to: @acme.fr`.

## Search (`/`)

`/` is the widest search, the joker: its words are looked for anywhere, subject, sender, recipients and body (where Text… leaves the addresses out), and `-word` excludes a word there too. It opens a panel titled "Search everywhere"; Return goes to **All Mail** of the current view (every folder but Deleted, Junk and Banned, every account in "all") with these words; typed from Everything, it stays in Everything. The result is the ordinary list: one row per conversation, with its folder, the `anywhere:` chip, and `f` to narrow it further (unread, From…). Escape clears the attributes and goes back to the folder `/` was pressed from; going elsewhere first (`;`, `@`, a click on a folder) forgets that folder, and Escape then only clears the attributes. A second `/` replaces the words and keeps the folder to go back to.

## Reading a mail

The whole theme (lists, status bar, menus, mails, including the HTML view, where a font the sender names wins in its own text) uses **iA Writer Mono V**, a font bundled with the app (SIL Open Font License). List columns have fixed widths counted in characters of this monospaced font, computed from the list width, so they stay aligned when the window is resized. Mails are shown as plain text, simplified: each message shows what it brings, the history it repeats is hidden, older read messages are folded (see `thread-display.md`). The `text | html | images` switch at the top of the mail, or `vh`, shows the sanitized HTML version instead (scripts are removed by the core, and remote images are not loaded; the sender's CSS, layout and embedded images are kept, each mail in a frame of its own, a mail made for a white page adapted to the dark theme by Dark Reader: see `thread-display.md`, "HTML of the sender"); `images`, or `vi`, also loads its remote images (see "Remote images" below). Each word of the switch is clicked on its own. The **title line** holds the subject, then the controls of the mail (unsubscribe, blocked pixels, the switch, the folder in "Everything"). It stays on one line while the whole subject fits beside them; otherwise the subject takes its own line and the controls go on a second one, aligned on the right. The choice is measured, not set by a threshold: the length of the subject, the text size (⌘+ ⌘−), the controls the mail has and the width of the reader all count, and it is made again when one changes (`ReaderTitleBar`). On two lines, a subject longer than the whole reader is cut at the end, its tooltip shows it whole. The choice is remembered **for the sender**: the next mails from the same address open the same way.

### Senders in the address book

The links panel (`l`) looks up the **senders of the conversation** in the Mac's address book (Contacts): the authors of its received messages, latest first; not the user's own messages (Sent, Drafts), nor the people in copy. For each card found: one row per phone number, named after the person and the card's label (`Alice Martin — mobile`, the number as the card writes it on the right), then **Card of Alice Martin**, which Return opens in Contacts. Return on a number calls it, as for the numbers of the mail. A number that is also in the mail (a signature) is listed once, under the name; a number shared by two cards (a switchboard), once under the first.

- **Access**: macOS asks the first time `l` is pressed, never at launch. Refused, the panel shows the mail's links alone and the status bar says once per session where to allow it (System Settings, Privacy & Security, Contacts). Once allowed, the cards are read with the thread, so they are there when `l` opens.
- **Installed app only**: macOS asks only an app whose `Info.plist` says why (`NSContactsUsageDescription`, written by `make bundle`, in English and French). `make app` runs a bare executable: it does not read Contacts and says so once.
- **Nothing is copied**: the cards are read on demand and kept for the session; nothing goes into NiceMailer's database or to the core. The lookup searches the cards' addresses (Contacts' own index), whatever their case, away from the main thread (`AddressBook.swift`).
- **Tests**: `AddressBookTests` (rows of a card, `tel:` URLs, senders, the question asked from `l` only, a refusal said once) and `addressbook.uiscript` use a book of their own (`FixtureAddressBook`): the user's cards are never read, and macOS's real question is not covered by automated tests.

### Attachments

At the foot of each unfolded mail, its attachments are tiles: the icon of the file type (an SF Symbol in the theme's colours: image, PDF, text, spreadsheet, slides, archive, audio, video, code, calendar, contact, mail, other file), the name cut in the middle to keep its extension, and the size. A row holds 3 to 5 tiles, depending on the width of the reading pane. A folded mail shows a paperclip and the number of its attachments.

- A click saves the file to `~/Downloads`, never overwriting a file (`devis (2).pdf`); the status bar says where it went.
- ⌘-click saves it the same way, then opens the saved copy with its default app, in front.
- `l` lists the links of the mail, then one row per attached file, with its icon; Return saves the file, ⌘-Return saves it and opens the saved copy at once, in front. The first row is **Link to this mail**: Return copies a Markdown link to the mail, `[subject](nicemailer://m/<id>)`, ⌘-Return its bare URL (`docs/deep-links.md`). Next come the **senders' cards** from the Mac's address book (below). Then the rows come in groups: links with a text first ("Voir ma facture"); then the **phone numbers** of the mail, with a phone icon, which Return calls with the Mac's phone app (macOS opens `tel:` links with it); then the attached files; last, the links without a text, mostly an image leading to a social network or a shop, shown by their site (`www.facebook.com`). Each row is in two halves of the same width: on the left what the link is (its text, the site, the number), on the right, muted, where it goes in full (the whole address, the address of a `mailto:`, the number behind "Nous appeler") or the size of a file, so that a long text never hides the address. The panel is as wide as the window allows (a margin of 40 points on each side, 1,100 points at most at the default text size), so that long link texts and addresses can be read; what still does not fit is cut in the middle. Typing filters on the text and on the whole address, even the part not shown. A link the mail gives several times (a title, a picture, a button, or several tracking addresses to the same page) is one row, named by its first text and followed by how many other texts lead there: `ressent "les angoisses" du pays (and 2 others)`; typing one of these other texts finds it. For a tracking link, the right half shows the page it leads to when its address says so, rather than the tracker; it still opens through the tracking address. A phone number written in national form (`06 12 34 56 78`) is dialled as written; the core's rules for finding numbers are in `docs/core-library.md`, "Links of a message".
- `sa` saves every attachment of the marked mails (or of the one under the cursor), `sg` all but the graphics.

Images the HTML of the mail shows (`cid:`) are part of the body, not tiles. An image marked inline that no HTML shows (Apple Mail puts photos in a plain text mail this way) is a tile. The tiles follow the body in the HTML view too.

Files are extracted by the core from the raw message on first use (`docs/core-library.md`, "Attachments"). Generated mail (`make seed`) has no raw message: its tiles show, but saving one reports that the content is not available.

### Remote images

Most HTML mails show images stored on the sender's servers. Loading one tells the sender that the mail was opened, when, and from which IP address; many mails also hold a **tracking pixel**, an invisible image whose only use is that. NiceMailer never loads them unless asked.

- **Tracking pixels are removed by the core**, when the mail arrives: a remote image of 2 pixels or less (`width`, `height` or its style), or hidden by its style (`display: none`, `visibility: hidden`, `opacity: 0`), loses its source and is never requested, whatever the display. Their number is shown on the title line by an eye struck out and the number (its tooltip and VoiceOver say "pixels blocked: N"). On a mailbox of 46,539 mails, 63% of the mails had remote images, and a script applying close rules found 26,868 pixels in 58% of those.
- **Other remote images keep their address** in a form the web view cannot load by itself (`nm-remote:`). The `images` word of the switch appears only when the conversation has some.
- **`images`** (a click, or `vi`) shows the HTML with those images. The app downloads each one once, over HTTPS only (an `http:` address is upgraded, never sent in clear), without cookies, `Referer` or a revealing User-Agent, never towards this Mac or the local network, at most 10 MB and only `image/…` content. Images are kept in `~/Library/Caches/NiceMailer/RemoteImages` (file names are hashes: no address appears), so reopening a mail sends no new request. Only **unfolded** messages load their images.
- **Remembered for the sender, applied to authenticated mail only.** Choosing `images` remembers it for the sender, but the next mails load their images by themselves only when they pass authentication (`pass`, see `authentification-expediteur.md`): anybody can write in the name of a trusted sender, and loading their images would confirm that the address is read. For any other mail (imported mail, whose verdict is unknown, included), the thread opens as HTML; choosing `images` loads its images **for this conversation only**, and the status bar says why.
- **Never in Junk, Quarantine or Banned**, even when asked.
- **A second barrier inside WebKit.** The HTML view also carries a content rule list that blocks every `http`, `https`, `ws`, `wss` and `ftp` resource of the page (images, style sheets, fonts, scripts, media, fetches), should a remote source ever pass the core's sanitization. If WebKit refuses the list, the page is still shown (the core's sanitization being the first barrier) and the refusal is written once to the system log (`log stream --predicate 'subsystem == "NiceMailer"'`). `RemoteBlockerTests` checks that the list compiles and that a page with raw `http://` sources sends no request to a local server.
- `vh` goes from HTML or images to text; `vi` toggles between HTML and images.

What this does not hide: the IP address and the time of the first opening, for each image loaded (only a VPN changes that); a mail whose every image carries a token proper to the recipient (about a quarter of the mails with remote images) reveals the opening as soon as its images load; a pixel hidden by a parent element, or declared without any size, is not recognised. A message unfolded by a click inside the HTML page (rather than from the keyboard) shows its images only once the conversation is reopened. Images inside the mail itself (`cid:`) are not shown yet.

### Unsubscribing

When the latest received mail of the conversation says how to unsubscribe from its list, a button appears on the title line, left of `text | html | images` (never in Junk, Quarantine or Banned: unsubscribing from a spam confirms the address to the spammer). Its look says where the link comes from and what a click does:

| Button | Link | A click (or ⌘U) |
| --- | --- | --- |
| `unsubscribe`, accent colour | `List-Unsubscribe` header, one click (RFC 8058) | the core unsubscribes at once; the status bar confirms, and the button gives way to "unsubscribed <date>" |
| `unsubscribe` with ↗, accent colour | header, a page of the list | opens the page in the browser |
| `unsubscribe` with an envelope, accent colour | header, an address only | opens the unsubscribe mail in Mail (until NiceMailer sends mail) |
| `unsubscribe` with a link symbol, muted | a link found in the text of the mail | opens it in the browser |

A one-click attempt the server refuses opens the page instead, and says why. The tooltip of the button tells the same. Only a one-click unsubscribe is recorded ("unsubscribed <date>"): after a page or a mail, the app cannot know whether you finished. The sender's decision and its mails are not changed. The links are found by the core when the mail arrives (`docs/core-library.md`, "Unsubscribing from a list").

### When a mail becomes read

- A mail you select (`j` `k`, a click, the next mail after a move) is marked read once shown for one second. Browsing quickly with `j` / `k` marks nothing. With one column the list alone shows no content, so staying on a row marks nothing; opening the mail (Return, double-click) marks it read at once, and inside the open mail `j` / `k` follow the one-second rule.
- The mail selected for you, the first one when a list opens or the one that replaces a mail that left the list, is not marked read by itself. It is marked read when you move from it to another mail of the list after at least one second. Moving it (`d`, `m`) or changing folder does not mark it.
- In a filtered list (unread, flagged…), the selected mail that stops matching (read after one second, `u`, `U`, `*`) stays under the cursor, shown with its new state, until you leave it; it then leaves the list. As in Gmail's unread view: without this, a list of unread mails would empty itself, one mail per second.
- A mail of **Quarantine** is never marked read by viewing it, by any of the rules above, wherever it is listed (Everything too): you are judging an unknown sender, not reading its mail, and the unread dot keeps saying which mails still wait for a decision. See "Quarantine" below.

## Keys

Actions apply to the **marked** mails if any (`J` / `K`), otherwise to the mail under the cursor.

| Key | Action |
| --- | --- |
| `j` `k` (or arrows) | next / previous mail |
| `ctrl-d` `ctrl-u` | half a page down / up in the list, as in Zed: the cursor moves by half the rows that fit in the list, and stops on the first or last mail |
| `J` `K`, shift-↓ shift-↑ | mark the mail and move (multiple selection) |
| `f` | attribute selector (filters on the current list) |
| `@` | account picker: **all** (unified view) or one account, keeping the folder role (Archive to Archive), or its Inbox |
| `l` | links of the mail, the senders' numbers and cards (Contacts), then its attached files; Return opens a link in the browser, or saves a file to Downloads; ⌘-Return saves the file and opens it |
| `d` | done: move to Archive |
| `m` + letter | move to a folder (`mt` Todo, `md` Deleted…); only allowed targets are offered. In Quarantine, the letter alone (`i n p x b d`), see "Quarantine" |
| Delete | move to Deleted |
| `t` | tag picker: choose a tag, or type a new name; the same tag again removes it |
| `g` | group picker: adds the sender to a group (new name: the group is created) |
| `T` | show everything in the thread (folded mails, quotes, signatures) or go back to the simplified view |
| `S` | skip to the text after the next quoted block |
| `ctrl-k` / `ctrl-j` | previous / next message of the conversation: the current one folds, the other unfolds and is scrolled to (`thread-display.md`) |
| `u` | read / unread: all the targets become read if one of them is unread, else all become unread |
| `U` | mark read, whatever the current state (a read mail stays read) |
| `z` | undo the last move or decision, then the one before (see "Moves and decisions") |
| ⌘U | unsubscribe from the list of the mail (see "Unsubscribing") |
| ⌃⌘F | full screen, and back; works everywhere, even while writing a reply or in a menu |
| `*` | flag |
| `/` | search everywhere: words anywhere, addresses included, in All Mail (see [Search](#search-)); Escape goes back |
| `;` + key | go to a folder or a screen (see below) |
| `s` + key | save to `~/Downloads`, for every marked conversation (or the one under the cursor): its latest mail with `se` .eml, `st` text, `sm` Markdown, and `sT` / `sM` to save then open the file in front, in the app macOS gives its type (TextEdit, Obsidian…; also ⌘-Return on the highlighted row; the row says "T: and open" on the right, muted; beyond 5 mails the files are saved but not opened, and the status bar says so); its attachments with `sa` all of them, `sg` all but the graphics (images: logos, signatures, photos). For attachments every message counts, and a file attached again to a reply is saved once. No file is overwritten |
| `v` + key | display: `v1` `v2` `v3` columns, `vc` sort by reception, `vm` sort by modification, `vh` text / HTML, `vi` remote images, `vs` status bar |
| Return | with one column (`v1`), read the mail in the whole window and mark it read; Escape goes back to the list |
| Space, shift-Space | with one column, on the list: Space reads the mail, as Return; with the mail on screen, scroll it a page down / up, as text or as HTML |
| `ctrl-f` `ctrl-b` | same, a page down / up in the mail |
| `?` | list of keys, grouped by use (Navigate, Read, Select, Act on mails, Write, Find and go, Window), one key per line, in a centered panel of up to three columns: fewer when the window is narrow. Commands not available yet are grayed out. Any key or a click outside closes it |
| Escape | cancel a prefix, close a panel, leave the full-window mail (`v1`), unmark, then go back from a tag, group or contact list to the folder |
| `r` | reply to all to the mail under the reader's cursor: the reply takes the reader's place, mode `EDT`, every key is text; ⌘↩ sends (`z` cancels during 5 s), ⌥⌘F chooses the address (and the account) it leaves from, Escape keeps the draft, ⌘⌫ abandons it; a mail of an account that cannot send is answered from one that can (see [sending.md](sending.md)) |
| `R` `F` `c` | reply to the sender only, forward, write a new mail ([sending.md](sending.md)) |

After a prefix (`;`, `m`, `s`, `v`), a menu lists the possible second keys, as in Helix. Each prefix has a block in the status bar: the folder for `;` (folders, Everything, screens), and an icon with its key for `m` (move, arrow), `s` (save, tray) and `v` (display, eye). The blocks form three groups set apart by a wide gap: what the list shows (`@`, `;`, `f` and the chips), the display preference (`v`), and actions on the current mail (`m`, `s`). The menu drops down from its block: under it when the bar is at the top, above it when the bar is at the bottom; a click on the block opens it, and its rows can be clicked. The arrows highlight a row (down starts on the first one, up on the last, both turn around) and Return goes to it; a key still acts at once, and Return with nothing highlighted closes the menu. With the status bar hidden, menus appear in the bottom-right corner.

### Mouse

The keyboard comes first, but every list can also be used with the mouse:

| Where | Click | Other |
| --- | --- | --- |
| Mail list | select the mail, unmark the others | ⌘-click marks or unmarks a mail (the current one joins the selection); ⇧-click marks every mail from the current one to the clicked one; with one column (`v1`), a double-click reads the mail in the whole window |
| Folders (3 columns) | go to the folder | |
| Account or folder in the status bar | open the account or folder picker | |
| Pickers and menus | choose the row | a click outside closes them |
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
| `;o` | Work: Inbox, Todo and Waiting of the view together |
| `;m` | All Mail: every folder of the view but Deleted, Junk and Banned |
| `;e` | Everything: every folder of the view |
| `;c` | Contacts: every known address, with its decision (or its domain's, after "@"), groups and number of mails; `/` filters; `m` chooses the decision on the address, `M` on its domain (`i` Inbox, `n` Newsletter, `p` Paperwork, `b` Banned, `-` none); Return lists the contact's mails |
| `;T` | Tags; Return lists the mails of the tag, in every account (uppercase: `;t` is Todo) |
| `;g` | Groups; Return lists the mails of the group's members |
| `;h` | History: the audit log, newest first |

On a screen, `j` `k` move and Escape goes back to the mails.

### Pickers

Pickers (`@`, `l`, `t`, `g`, `f`) are filterable lists: type to filter, ignoring case and accents; the best matches come first (the label starts with the typed text, then each typed word starts a word of the label, then initials such as `nf` for "not flagged", then each word anywhere in the label; see "Reaching a row from the keyboard" above), `ctrl-n` / `ctrl-p` or the arrows to move, Return to choose, Escape to close. A detail at the right end of a row is either a count to notice (unread mails, in the unread colour; a warning such as "nothing in between") or an information (where a link goes, how a date was read, "domain"), muted. A picker opens on its first row and turns around: up (or `ctrl-p`) from the first row selects the last one, so the end of the list is one key away, and down (or `ctrl-n`) from the last row comes back to the first.

The typed text is edited as in any macOS text field (`LineEdit.swift`): ← and → move the cursor, ⌥← ⌥→ by word, ⌘← ⌘→ (or Home, End, `ctrl-a`, `ctrl-e`) to either end; Delete removes the character left of the cursor, forward delete (fn-Delete) the one right of it, ⌥-Delete a word, ⌘-Delete everything before the cursor. **⌘V** pastes the text copied in any app at the cursor, as one line: line breaks become spaces and the spaces around it go (copying a word often takes the space after it). The contacts filter (`/` on the contacts screen) is edited the same way.

A click outside an open picker, prefix menu or the help closes it, like Escape. The click does nothing else: the mail under it is not selected. A click on a block of the status bar opens that block's menu or picker directly. A question (a suspect move, a decision) is not closed by a click: it waits for its answer.

## Moves and decisions

Moving a mail to Newsletter, Paperwork or Banned, or out of Quarantine to the Inbox, also decides where its sender's next mails go and files the mails already received from it (`folder-workflow.md`). The message says so: "“Promo” moved to Newsletter · next mails of news@shop.fr too · 12 other mails filed · z: undo".

`z` undoes the last move or decision: the mails go back where they were and the previous decision returns. Each `z` goes one step further back (50 steps, for the session); a move of several marked mails is one step. A mail moved again since stays where it is. The cursor goes back to the conversation brought back (the first of the list, for several): the move had taken it to the next one, where a second Delete after `z` would have hit another conversation (`Workflow.cursorAfterUndo`).

When the sender's mail is suspected of spoofing, the core refuses the move and the app asks, in a small menu (mode `ASK`) with the mail under the cursor: `y` moves it anyway, `t` moves it and accepts this sender's mail without authentication from now on (a sender whose tool is badly configured), Escape leaves it where it is. Every other key waits for the answer. Moving such a mail to Banned concerns that mail only and asks nothing.

The same small menu chooses a decision on the contacts screen (`m`, `M`); the current decision is marked "•". Keys, the arrows with Return, or a click choose.

### Quarantine

In Quarantine the only thing to do with a mail is to decide where it goes, so the move targets are single keys:

- **The status bar shows the targets** at its right end, just before `?`, in place of the `m` and `s` blocks: the move arrow, then each target with its key, `Inbox i  Newsletter n  Paperwork p  Junk x  Banned b  Deleted d` (the core's `move_targets` for the folder of the mail under the cursor). On the right they stay in the same place whatever the account, the folder and the chips on the left; a message (the result of a move) comes before them and is cut first when the bar is short. A click on one moves as its key does.
- **One key moves**: `i` does what `mi` does elsewhere, for the marked mails or the one under the cursor. The cursor goes to the next mail, as after any move, and `z` undoes. `d` moves to **Deleted** here (Archive is not a target from Quarantine), while it archives everywhere else.
- **`m`** opens no menu: the status bar says which keys to press.
- **Refused**: `u` `U` (read state), `t` (tag), `g` (group), `r` `R` (reply: writing to the sender would accept it), `F` (forward: sort the mail first) and `s` (save: its block leaves the status bar; the mail is to be sorted, not kept). The status bar says "In Quarantine, a mail can only be moved or flagged".
- **Still available**: `*` (flag), the navigation and marks, `v` (display), `l` (links), `z`, Delete, and the keys of every screen.
- **Never read by viewing**: see "When a mail becomes read". The core itself does not refuse `set_thread_seen`: the CLI can still mark a mail of Quarantine read when asked to.

This holds while a Quarantine folder is listed (of an account, or of "all"). In Everything, or a tag, group or contact list, the rows come from several folders: the `m` and `s` blocks and the usual keys stay (`GeekController.directTargets`).

## Not available yet

- In the "edition" context: signatures, choosing the sending address ([sending.md](sending.md)).
- Saving as PDF (`sp`).
- A dark/light override independent of macOS (the palette follows the system appearance).

## Implementation notes

- Keys are read by an AppKit event monitor (`GeekController.handle`) before any view, so a key is never lost to a focused field: picker text is edited by the controller itself, with its cursor in a `LineEdit` value (pure functions, tested on their own), and ⌘V is read there too. AppKit gives the arrows and other function keys characters of the private use area (U+F700–U+F8FF): `LineEdit.isTypable` rejects them, otherwise the left arrow would type an invisible character.
- All user-facing strings go through `L(...)` and the String Catalog `Resources/Localizable.xcstrings` (English and French). The language is chosen explicitly from the user's preferences, because an app launched with `swift run` has no main bundle declaring its languages.
- Rows of lazy lists receive plain values (cursor, marks) as parameters: reading the observable model only inside a `ScrollView` closure did not always refresh them.
- Clicks use tap gestures filtered by modifier (`View.onClick`): SwiftUI runs tap actions after the click was processed, when `NSApp.currentEvent` no longer describes it.
- The core's events invalidate folders and threads; the reloads they cause are paced (`AppModel.scheduleRefresh`, `evenements.md`, section 7): the first event makes a reload due, the next ones add to it; it starts after 40 ms, at least 500 ms after the previous one started (`refreshDelay`, a pure function), and never while another runs. During an import the events never pause: the lists are still reloaded, twice a second at most, and never pile up in the core in front of what the user waits for (`RefreshPacingTests`).
- A status bar message computed after a wait (the reply screen opening, which waits for `list_senders`) is shown only if nothing newer was said meanwhile (`AppModel.sayOpening`, `flashCount`): Escape pressed in between keeps its own message.
- Tested by `Tests/NiceMailerTests` (unit) and `UITests/*.uiscript` (scripted UI); see `docs/testing.md`.
- A mail is listed and can be opened as soon as its headers are in; the reader then says "(body not downloaded)", and opening it puts its body first in the download queue. The body's arrival is announced by a `MessageChanged` alone (no `ThreadChanged`), so the app reads it too: for the open thread, the reader is reloaded and shows the body without the user leaving the mail; for a listed thread, its list is reloaded (the row's snippet comes with the body); for any other thread, nothing is reloaded, so the bodies of a whole account coming in cost nothing beyond the mails on screen (`AppModel.messageDirties`, `BodyArrivalTests`).
