# Keys: `keymap.toml`

Every key of the geek theme can be changed in `keymap.toml`, a text file next to `config.toml` (`~/.config/nicemailer/keymap.toml`, or the folder of `NICEMAILER_CONFIG`). There is no screen to edit it: it is meant for users who are at ease with a TOML file, and to be kept in a dotfiles repository. NiceMailer never syncs it between Macs. The specification (in French) is `specifications/raccourcis.md`.

The keys named in the other pages of this documentation, and in `ui.md`, are the defaults.

## The file

The core creates the file at its first start, with every default key **commented out**, and never touches it again. To change a key, remove the `# ` in front of its line and change the action; to remove a default key, write `"none"`. A line needs to be there only when it differs from the defaults: a key that is not in the file keeps its default.

```toml
version = 1

[mail]
e = "mail.archive"        # archive with e, as in Gmail
d = "none"                # d no longer archives
"#" = "mail.delete"
I = "goto.inbox"          # straight to the Inbox, without `;`
space = "menu.mine"       # a menu of your own

[menu.mine]
a = "save.attachments"
p = "save.pdf_open"

[compose]
"cmd-shift-s" = "compose.file_and_send"
```

The core reads the file again as soon as it is saved: the app applies it at once, without a restart.

```sh
nicemailer keymap defaults          # every default key, with what it does
nicemailer keymap check             # validate keymap.toml without applying it (status 1 if refused)
nicemailer keymap check other.toml
nicemailer keymap show              # the keys in effect, by context and by menu
nicemailer keymap show --json
nicemailer config path              # config.toml, accounts.toml, keymap.toml
```

## Writing a key

- **Without ctrl, alt nor cmd, a key is the character it types**: `j`, `J` (Shift on `j`), `?`, `*`, `@`, `;`, `é`. `?` is the key that types `?`, wherever your keyboard layout puts it. A key that is not a letter or a digit is written in quotes: `"?" = "help.show"`.
- **Named keys**: `space`, `enter`, `tab`, `esc`, `backspace` (⌫), `del` (⌦), `up`, `down`, `left`, `right`, `home`, `end`, `pageup`, `pagedown`, `f1` to `f12`.
- **Modifiers**: `ctrl-`, `alt-` (⌥), `shift-`, `cmd-`, in any order: `cmd-shift-a` and `shift-cmd-a` are the same key. With one of them, the key is the letter printed on it (⌥n types `~` on an AZERTY keyboard, but is still written `alt-n`).
- **Shift**: for a letter, `J` and `shift-j` are the same key, and so are `ctrl-C` and `ctrl-shift-c`. For a key that is not a letter, Shift is part of the character: write `?`, not `shift-/` (which is refused: `/` and `?` are not on the same key on every layout). For a named key, Shift is written: `shift-space`, `shift-down`.
- **Reserved**: `esc` and `cmd-q` can be neither bound nor removed. Escape always goes back; ⌘Q always quits.
- There are no free sequences of keys (`g g`): a sequence is a menu (below).

`nicemailer keymap show` prints each key in its one spelling: modifiers in the order `ctrl-cmd-alt-shift-`, a capital for Shift on a letter alone. The app shows keys with the symbols of the Mac: `ctrl-shift-c` is `⌃⇧C`.

## Contexts

Each table is a context: where the key works. A key is looked for in the most precise context active, then in its parents:

| Context | Active | Then |
| --- | --- | --- |
| `everywhere` | always, even while a reply is written or a panel is open | — |
| `normal` | on every screen, no panel open, no reply being written | `everywhere` |
| `mail` | the mail screen (list and reader) | `normal` |
| `quarantine` | a Quarantine folder is listed | `mail` |
| `screen` | the Contacts, History, Tags and Groups screens | `normal` |
| `contacts`, `tags`, `groups` | that screen | `screen` |
| `menu_navigation` | a menu is open | `everywhere` |
| `picker` | a picker is open (`@ f / l t g`, a rename) | `line_edit` |
| `contact_filter` | the filter line of Contacts is open | `line_edit` |
| `line_edit` | a line of text has the keys | `everywhere` |
| `choice` | a question waits for its answer | `everywhere` |
| `ai_panel` | the AI panel of a mail or a reply is open | `everywhere` |
| `compose` | a reply is being written | `everywhere` |

While a reply is written, its subject and recipient lines look in `line_edit`, then `compose`, then `everywhere`: ⌘V pastes on one line there, and ⌘↩ still sends.

A `"none"` in a context also hides the key of its parents there: `"?" = "none"` in `[contacts]` makes `?` do nothing on the Contacts screen only.

**Where text is typed** (`everywhere`, `compose`, `line_edit`, `picker`, `contact_filter`), a key that types a character (a letter, a symbol, `space`, `alt-` with a character) is refused: that character could no longer be typed. In `everywhere` and `compose`, the keys the text editor needs are refused too (`enter`, `backspace`, `del`, the arrows, `home`, `end`, `pageup`, `pagedown`, and `tab` in `everywhere`); in a line, ←, →, ⌫ and ⌦.

**Not in the keymap**: the keys of the editor of a reply's text (`editor.md`), the plain ← → ⌫ ⌦ of a line, the behaviour of a reply's fields (Return, `,` and `;` ending an address, ⌫ taking the last address back, the line of files), the letters answering a question, the digits choosing a row in the AI panel, Escape, ⌘Q, and the mouse.

**Shift selects**: in a line, Shift added to a key that moves (`line.start`, `line.end`, `line.word_left`, `line.word_right`) selects instead, without a line of its own: if `ctrl-b` moves a word left, `ctrl-shift-b` selects a word.

## Actions

Each action can be bound in its context and in the contexts under it (`move.inbox`, of `mail`, also in `quarantine`); elsewhere it is refused. `nicemailer keymap defaults` lists them all with their default keys; the main ones:

| Context | Actions |
| --- | --- |
| `everywhere` | `window.full_screen`, `text.bigger`, `text.smaller`, `text.reset`, `app.reload` |
| `normal` | `menu.goto`, `help.show`, `undo.last`, `goto.<folder>` (`inbox`, `todo`, `waiting`, `work`, `archive`, `newsletter`, `paperwork`, `sent`, `drafts`, `all_mail`, `quarantine`, `deleted`, `junk`, `banned`, `everything`), `goto.contacts`, `goto.tags`, `goto.groups`, `goto.history` |
| `mail` | `list.down`, `list.up`, `list.mark_down`, `list.mark_up`, `list.mark_all`, `list.half_page_down`, `list.half_page_up`, `message.next`, `message.previous`, `mail.open`, `reader.open_or_page_down`, `reader.page_down`, `reader.page_up`, `reader.toggle_all`, `reader.skip_quoted`, `sender.card_or_copy`, `sender.mails`, `mail.archive`, `mail.delete`, `mail.toggle_seen`, `mail.mark_seen`, `mail.toggle_flag`, `mail.unsubscribe`, `picker.links`, `picker.tag`, `picker.group`, `picker.search`, `picker.attributes`, `picker.accounts`, `compose.reply_all`, `compose.reply_sender`, `compose.forward`, `compose.new`, `ai.menu`, `menu.move`, `menu.save`, `menu.view`, `move.<folder>`, `save.*`, `view.*` |
| `screen` | `screen.down`, `screen.up`, `screen.open`, `label.rename`, `label.delete` (Tags and Groups) |
| `contacts` | `contacts.filter`, `contacts.decide_address`, `contacts.decide_domain`, `contacts.sender_kind` |
| `menu_navigation` | `menu.next`, `menu.previous`, `menu.choose`, `menu.choose_alt` |
| `picker` | `picker.next`, `picker.previous`, `picker.choose`, `picker.choose_alt` |
| `contact_filter` | `contact_filter.close` |
| `line_edit` | `line.start`, `line.end`, `line.word_left`, `line.word_right`, `line.delete_word`, `line.delete_to_start`, `line.select_all`, `line.copy`, `line.cut`, `line.paste` |
| `choice` | `choice.next`, `choice.previous`, `choice.choose` |
| `ai_panel` | `ai.accept`, `ai.refuse`, `ai.copy`, `ai.confirm`, `ai.open`, `ai.next`, `ai.previous` |
| `compose` | `compose.send`, `compose.file_and_send`, `compose.abandon`, `ai.compose_menu`, `compose.toggle_plain`, `compose.attach`, `compose.from`, `compose.bcc`, `compose.next_field`, `compose.previous_field`, `compose.paste`, `compose.paste_plain` |

In Quarantine, the actions that would change a mail other than by moving or flagging it (`mail.toggle_seen`, `mail.mark_seen`, `picker.tag`, `picker.group`, the replies, the forward, `menu.save`) say why they do nothing, whatever their key.

`move.<folder>` and `goto.<folder>` can be bound to a key of their own, outside their menu: a move is made only when the core allows it from the folder of the mail (the menu `m` shows only those), and a folder the current view does not have says so.

## Menus

A table `[menu.<name>]` is a menu: its keys are the second keys, after the key that opens it. The action `menu.<name>` opens it. The app has four:

- `menu.move` (`m`): the move targets, only those allowed from the mail's folder;
- `menu.goto` (`;`): the folders of the view, in their blocks, then the screens; the folder column shows the key each folder has here;
- `menu.save` (`s`): `save.text` and `save.text_open` (and the other `save.*` with their `_open` variant) make one row: the variant's key on its right ("T: and open"), and ⌘-Return on the row runs it;
- `menu.view` (`v`).

A row is changed or added like a key of a context; `"none"` removes it. A menu left without a row is refused.

**Menus of your own**: a table `[menu.<name>]` with another name (`a-z`, `0-9`, `_`) creates a menu, which a key opens with `menu.<name>` from any screen. It shows its rows with the label of each action, has no block in the status bar, and the help lists it under "Your menus". A menu of your own that no key opens is refused. A menu cannot hold another menu.

Taking the key of a built-in menu for something else (`m = "mail.archive"`) without giving `menu.move` another key is only a warning: its block of the status bar still opens it.

## Errors

The file is checked whole. A file with a mistake is **refused whole**, and the keys in effect stay (the defaults, at start). The status bar says it in red, with the line, until the file is fixed: "keymap.toml refused, previous keys kept: keymap.toml, line 7: mail.esc: …". `nicemailer keymap check` says the same. A refused `keymap.toml` never affects `config.toml` or `accounts.toml`, nor the reverse.

Refused: a TOML syntax error; a key badly written; two spellings of one key in a table (`J` and `shift-j`); an unknown context or a bad menu name; an action in a context it does not belong to; a reserved key; a key that types a character, or one the editor needs, where text is typed; a menu left empty, or of your own and opened by no key.

A warning only (the line is ignored): an **unknown action**, so that a file written for a newer NiceMailer stays readable by an older one.

## What shows the keys

Everything that shows a key takes it from the keymap in effect: the help (`?`; a line whose actions have no key is left out), the status bar blocks and the targets of Quarantine, the rows of the menus and their "T: and open", the folder column, the items of the View menu of the menu bar (Reload, text size), the hints of the pickers, the screens and the AI panel, the line of keys above a reply, and the messages that name a key ("Text only · ⇧⌘T for formatted text"). A message whose action has no key names the action instead.

## For developers

- **The core** reads the file (`crates/nm-engine/src/keymap/`): `notation.rs` (spelling a key), `catalog.rs` (contexts and actions), `mod.rs` (reading, merging into the defaults, checks, `KeymapFile` followed by the watch of `config.toml`). The defaults are `defaults.toml`, embedded in the core; the template of the created file is made from it. The contract gives the result: `keymap_get` and the event `KeymapChanged` (`contract.md`); a refused file gives `ConfigError` with `file` `keymap.toml`.
- **The app** (`apps/NiceMailer/Sources/NiceMailer/Geek/Keymap.swift`) spells each `NSEvent` as the core spells the keys (`KeyChord`), and looks it up in the contexts of what has the keys (`GeekController.keyChain`); `GeekController.perform` runs the action. It embeds the defaults as `Resources/keymap-defaults.json`, written by the core: after a change to the catalogue or the defaults, run `nicemailer keymap show --defaults --json > apps/NiceMailer/Sources/NiceMailer/Resources/keymap-defaults.json`. A test of the core fails while the file is stale, and a test of the app checks that the app knows exactly the actions of the core.
- **Adding an action**: a line in `catalog.rs` (with the highest context it belongs to), its default key in `defaults.toml` (every action has one, so the help shows it), a row in `raccourcis.md`, section 6.1 (a test checks it), the case in `KeyAction` with its label, its code in `GeekController.perform` (or in the handler of its panel), the JSON regenerated.
- **Tests**: `crates/nm-engine/src/keymap/tests.rs`, `crates/nm-daemon/tests/keymap_rpc.rs`, `KeymapTests.swift`, and the UI scenarios `keymap.uiscript` and `keymaperror.uiscript` (a scenario's `fixtures/<scenario>.keymap.toml` is its core's `keymap.toml`).
