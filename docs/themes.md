# Themes: `themes/`

The look of the app comes from a theme: its colors, fonts, proportions and animations, and a few variants of the window (a status bar or a toolbar, the blocks it shows…). Four themes are built in, **geek**, **clean**, **regular** and **minimal**; each one is a file you can modify, and you can write your own. A theme only changes the presentation: the keys, the menus and every feature are the same in all of them. The specification is `specifications/themes.md`.

State (November 2026): the core reads, checks and serves the themes, and the app applies every key of them.

**minimal** (November 2026) is clean without anything that moves or waits, in the font of geek: iA Writer Mono V for the interface and the mails, a smaller status bar (`bar_size = "small"`, 30 pt high) that reminds the key of each block, a column of text up to 760 pt wide, no animation of the folder column or the menus, and mails shown as text until you choose HTML for a sender. Everything else (colors, the list on three lines, rounded rows and menus, the buttons in the bar) is clean's.

With `bar_type = "toolbar"` (regular), the status bar becomes a macOS toolbar, always at the top: the folder is its title (a click opens the menu `;`), with the message of the moment or the synchronization under it, as in Mail; the other blocks are buttons, which open the same menus and pickers under them. The keys are those of every theme.

## Choosing a theme

In `config.toml`:

```toml
[ui]
theme = "clean"
```

`clean` is the default ("The default theme" below). The name is that of a built-in theme or of a file of `themes/`. From the keyboard, `v` `t` (Show, theme…) in the main window opens a picker of the themes the core lists (`theme_list`), on the one in effect; Return writes the one chosen in `config.toml` (`config_set_theme`, keeping the rest of the file), and a theme whose file is refused says why instead of being chosen. A name that names no theme is not an error of `config.toml` (the accounts stay on): clean is applied, with a warning. The change applies at once, without restarting.

`ui.title_bar` is replaced by `title_bar` in the `[window]` table of the theme: `shown`, `hidden`, or `merged`, the red, yellow and green buttons over the left end of the status bar, which leaves them room (with the status bar at the bottom or hidden, the title bar is shown). Written in `config.toml`, `ui.title_bar` is still applied, with a warning.

### The default theme

A new install opens on **clean**: `config.toml`, created at the first start, leaves `ui.theme` commented out, and clean is the default (`DEFAULT_THEME`, `crates/nm-engine/src/theme/catalog.rs`). The default is also the parent of a theme without `extends`, and the theme applied in place of one that names nothing or is refused.

An install from before the themes (NiceMailer 0.2 and earlier) keeps **geek**, the only look it had: its user never chose a theme, there was none to choose, and an update should not change the app under them. At the first start of the new version, before it creates `themes/`, the core writes `theme = "geek"` in the `[ui]` table of their `config.toml`, keeping the rest of the file, and says so in its log (`config_write::keep_geek_for_earlier_install`). Such an install is recognized by a `config.toml` without the folder `themes/` next to it: the core creates that folder at every start since the themes came, so the pair is seen at one start only. The line is not written when:

- `ui.theme` is already written, even to geek: it is a choice;
- the core refuses `config.toml`: the file is the user's to fix first;
- `config.toml` is a symbolic link that points nowhere. A link that points to a file has that file written, and stays a link.

A user who wants clean afterwards chooses it with `v` `t`, or removes the line: the core never writes it again.

The app draws geek until the core answers `theme_get`, a moment at launch: its unit tests share one theme store and were written for geek.

## The files

The themes are in the folder `themes/`, next to `config.toml` (`~/.config/nicemailer/themes/`, or next to the file `NICEMAILER_CONFIG` names). `nicemailer config path` prints it.

At its first start, the core creates `geek.toml`, `clean.toml`, `regular.toml` and `minimal.toml` there (`minimal.toml` at the first start after it came, in November 2026), with every key commented out at its value, after a line that says what it does. They change nothing until you remove the `# ` in front of a line. A key left commented keeps the value NiceMailer ships, even when a later version changes it. An existing file is never touched; a file you delete stays deleted, and the built-in theme stays.

- **A file named like a built-in theme modifies it.** `regular.toml` is laid over the built-in regular; it cannot extend another theme.
- **Any other file is a theme of yours** and says what it extends. It holds only what differs:

```toml
version = 1
extends = "clean"

[window]
key_hints = true
blocks = ["account", "folder", "attributes", "->", "move", "save", "help"]

[colors]
background = { light = "#F6F0E4", dark = "#221E18" }
text = { light = "#3B3025", dark = "#E8DFD0" }
```

Without `extends`, the parent is the default theme, clean, with a warning. A theme can extend a theme of yours that extends a built-in one, up to 8 levels. A parent is used as it is, your changes included: a theme that extends clean follows what you changed in `clean.toml`.

A theme holds data only: no code, no image, no font, no address to load. A theme someone sends you can be put in the folder without risk.

## Writing a value

- **A color**: `"#RRGGBB"` or `"#RRGGBBAA"` for both appearances of macOS; `{ light = "#FFFFFF", dark = "#1E1E1E" }` for one each; or a color of macOS that follows the appearance and the accent color of System Settings by itself: `"system.accent"`, `label`, `secondary_label`, `tertiary_label`, `separator`, `window_background`, `control_background`, `under_page_background`, `text_background`, `selected_content_background`, `unemphasized_selected_content_background`, `alternating_content_background`, `find_highlight`, `red`, `orange`, `yellow`, `green`, `blue`, `purple`, `gray`.
- **A font**: `"system"` (SF Pro), `"system-mono"` (SF Mono), `"system-serif"` (New York), `"system-rounded"`, or the name of a family installed on the Mac (`"iA Writer Mono V"` comes with the app). A family that is not installed is replaced by the system font, and the status bar says so when the theme is applied. Any font works: in a font of fixed pitch the columns of the list are counted in characters, in a proportional one each column is as wide as the texts it holds (the widest date the list can show, three digits for the count).
- **A length** is in points for a 12 pt text: it grows with `ui.text_size`, which stays in `config.toml`. The text size depends on your eyes, not on the theme.
- **A duration** is in milliseconds.

## The keys

`nicemailer theme defaults geek` prints them all with their role. In short:

| Table | Keys |
| --- | --- |
| `[window]` | `bar_type` (`status_bar` or `toolbar`), `status_bar_position` (`top`, `bottom`, `hidden`), `title_bar` (`shown`, `hidden`, `merged`: the red, yellow and green buttons in the bar), `key_hints`, `block_labels` (the blocks of actions say their name), `bar_height` (of the status bar or the toolbar), `bar_size` (`small`, `medium`, `large`: the icons and texts of the bar, × 0.85, × 1 or × 1.2 of `ui.status_bar_text_size`; its height follows), `block_style` (`plain`, or `pill`: each block in a pill, the account and the folder in bold, the block open highlighted), `bar_line` (a line between the bar and the mails), `blocks`, `default_columns` (1 to 3), `list_layout` (`columns`: one line per mail; `stacked`: three lines, sender and date, subject, the beginning of the mail; until the user chooses with `v` `l`) |
| `[fonts]` | `interface`, `reading`, `reading_scale`, `reading_line_spacing` (points between two lines of the body, whatever the text size), `title_scale` |
| `[layout]` | `row_height`, `row_radius` (also rounds the buttons of the reader's title line, text | html | images and unsubscribe, in proportion to their height), `row_inset`, `zebra`, `menu_row_height` (0: the height of the text), `popup_radius`, `popup_shadow`, `popup_padding` (the margin inside menus, pickers and panels), `menu_keys` (`left`, `right`), `message_cards`, `avatars`, `attachments` (`tiles`: a grid of tiles with a symbol in the theme's colors, geek; `rows`: a row per file with the icon macOS gives its type, as the Finder does, and its whole name, clean and regular), `reader_max_width` (the widest column of the reader, centered, in text and in HTML alike; the title line is in the same column: the subject starts where the text starts, the controls end where it ends; an HTML mail laid out wider scrolls sideways in its frame; 0: the whole width, less 12 pt on each side), `reader_title_padding` (room above and below the subject of the reader, never on its sides, so that the title stays lined up with the text; half of it sets apart the two lines of a title too long for one; 0 in geek and regular, whose line under the title sets it apart), `scrollers` (`none`, `thin`, `standard`), `separators` (`lines`, `space`) |
| `[motion]` | `sidebar_ms`, `popup_ms` (0 when System Settings reduce motion) |
| `[colors]` | `background`, `sidebar`, `list`, `bar`, `text`, `muted`, `accent`, `accent_text`, `selection`, `selection_text`, `marked`, `marked_bar`, `separator`, `popup`, `unread`, `flag`, `error`, `virtual_folder` |
| `[reader]` | `body_format` (`text`, `html`): how a received mail is shown when nothing is remembered for its sender; text in geek and minimal, HTML in clean and regular. A choice remembered for the sender (`vh`, a click on `text | html | images`) wins in every theme, and a mail without HTML is text. Only the reading: the mode a new mail starts in is `compose.format` of `config.toml` ([editor.md](editor.md)), so that a theme never changes what recipients get. `html_min_font_size` (0, or 6 to 24; 11 in the four built-in themes): the smallest text of a mail in HTML, in points for a 12 pt text; smaller text, however the mail writes it, is drawn at this size (WebKit's minimum font size), and it grows with the text size as the whole page; 0 turns it off. `title_actions`: the controls of the reader's title line, in order — `folder_name`, `show_all`, `pixels`, `unsubscribed`, then the actions `reply_all`, `reply_sender`, `forward`, `unsubscribe`, `flag`, `text_or_html`. One left out is hidden, its key still works; an unknown or repeated name refuses the file; one without an object for the mail shown (no unsubscribe offer, no HTML part) is hidden too, and in Quarantine the reply and forward buttons are hidden. |

`blocks` lists the blocks of the status bar or the toolbar, in the order they are drawn: `account`, `folder`, `attributes`, `view`, `move`, `save`, `ai`, `help`, with `"|"` for a wide gap and `"->"` to send what follows to the right. A block left out is hidden; its key still works. A gap is drawn only between two blocks shown (`m` and `s` are not in Quarantine, the menus of the mail are not on the contacts screen…), and without `"->"` the messages go to the right end. In the toolbar, the folder is always its title, on the left. The messages of the app, the progress of the synchronization and its questions are always shown.

## Errors and warnings

A theme file is read again as soon as it is saved. Like `keymap.toml`, an invalid file is refused whole, and alone: the previous version of that theme stays applied, the other files too. Refused:

- a TOML syntax error, an unknown table, a `version` other than 1;
- a value of the wrong type, outside its range, or not among its choices;
- a color badly written, an unknown color of the system, a pair without `light` or `dark`;
- an unknown block, or one listed twice;
- an `extends` toward a theme that does not exist, a circular inheritance, more than 8 levels;
- a file bigger than 64 KB.

Warnings, which do not refuse the file: an unknown key (a theme written for a newer NiceMailer stays readable), a missing `extends`, and a text color below 4.5:1 of contrast on its background (WCAG AA), in either appearance. The colors of the system are not checked: macOS answers for them.

A file whose name cannot name a theme (`My Theme.toml`) is ignored, with a warning.

## The command line

The commands read the files themselves, without the core:

```sh
nicemailer theme list                      # the themes, * before the one chosen
nicemailer theme check sepia               # or a path: nicemailer theme check ./sepia.toml
nicemailer theme show clean                # every key with its value, inheritance applied
nicemailer theme show clean --json         # as the app receives it
nicemailer theme defaults regular          # the complete file of a built-in theme
nicemailer theme new sepia --extends clean # creates themes/sepia.toml
```

`check` exits with status 1 when the core would refuse the file.

## For developers

- **The core** (`crates/nm-engine/src/theme/`): `catalog.rs` lists every key, its type, its bounds and its role; the built-in themes are `geek.toml`, `clean.toml`, `regular.toml` and `minimal.toml`, embedded with `include_str!`, each giving every key a value (a test checks it). `mod.rs` reads a file (`parse`, with `toml_edit` for the line of each problem), resolves the inheritance (`resolve`), checks the contrast, builds the view (`view`), writes the commented files (`template`, `new_theme_file`), and follows the folder (`ThemeSet`, which keeps the last valid version of each file). `config_write::set_theme` writes `ui.theme`, keeping the rest of `config.toml` (`set_compose_format` writes `compose.format` the same way). The app reads `reader.body_format` in `AppModel.bodyFormat` and `reader.title_actions` in `ReaderTitleAction` (Interface/ReaderTitleActions.swift).
- **The daemon** reads the folder at start, watches it (`watch::watch_dir`), follows `ui.theme` after each change of `config.toml`, and answers `theme_get`, `theme_list` and `config_set_theme` ([contract.md](contract.md)). It emits `ThemeChanged` when the theme in effect changes, and `ConfigError` with `file` `themes/<name>.toml` for a refused file.
- **The app** (`ThemeStyle.swift`) embeds the four built-in themes resolved (`Resources/themes-builtin.json`, from `nicemailer theme show --builtins --json`; a Rust test checks it is up to date), its theme until the core answers. `ThemeStore.shared` holds the theme in effect, read with `theme_get` at the start of a session and at each `ThemeChanged`. The colors of `Palette` are dynamic colors that read the theme's when AppKit draws, so the views kept their names; a new theme rebuilds the window (`ThemeStore.revision`, as a new text size does). `Fonts.interface` gives the theme's interface font, and `TextMeasure` measures text in it: the list (`RowLayout`), the key column of the menus, the help (`Help.columnCount`) and the attachment tiles are sized with it, in characters for a font of fixed pitch, as before, and on the measured texts for a proportional one. What a view draws differently from one theme to another is asked of `Look` (`ThemeLook.swift`): the rows (`rowFill`: inset, radius, alternate rows), the panels (`popupChrome`: radius, shadow), the scroll bars, the separators, the side of the keys in a menu (on the right, the unread count of a folder sits in a small pill tinted with the unread color, set apart from the key, in the menu `;` and in the folder column; with a rounded highlight, the text of the folder column keeps room inside it, `sidebarTextInset`), the title of a menu (in a theme of rounded panels, a small grey heading over a line, as a section of a macOS menu) and the room inside its highlighted row, the animations of the folder column and of the menus (none in geek, nor when System Settings reduce motion), the color of what is drawn on a selection of the accent (`Look.ink`: in regular, every text and icon of the selected mail, the current folder or group, the highlighted row of a menu, a picker or a screen (Contacts, History, Tags, Groups), the current word of text | html | images, and in a reply the addresses, the files, the suggestion highlighted, the Bcc button, the text selected and the choices of the AI turn white, as in a macOS list), the width of the reader, the size of its subject and the room around it (`readerTitleInsets`). The HTML reader and the editor take the theme's reading font (`Fonts.css`). The picker `v` `t` (`PickerKind.themes`) is `ThemeChooser` (`Preferences/ThemeChooser.swift`). Tests: `ThemeTests.swift` (with `ThemeChooserTests`), `TextMeasureTests.swift`, `themes.uiscript`, `themes-refused.uiscript`, `themes-clean.uiscript`, `themes-minimal.uiscript`.
- **Tests**: `crates/nm-engine/src/theme/tests.rs` (reading, inheritance, contrast, view, templates, the folder), `crates/nm-engine/tests/watch.rs` (the folder watched), `crates/nm-daemon/tests/theme_rpc.rs` (end to end), `config_write` (writing `ui.theme`).
