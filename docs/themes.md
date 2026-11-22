# Themes: `themes/`

The look of the app comes from a theme: its colors, fonts, proportions and animations, and a few variants of the window (a status bar or a toolbar, the blocks it shows…). Three themes are built in, **geek**, **clean** and **regular**; each one is a file you can modify, and you can write your own. A theme only changes the presentation: the keys, the menus and every feature are the same in all of them. The specification is `specifications/themes.md`.

State (November 2026): the core reads, checks and serves the themes, and the app applies every key of them.

With `chrome = "toolbar"` (regular), the status bar becomes a macOS toolbar, always at the top: the folder is its title (a click opens the menu `;`), with the message of the moment or the synchronization under it, as in Mail; the other blocks are buttons, which open the same menus and pickers under them. The keys are those of every theme.

## Choosing a theme

In `config.toml`:

```toml
[ui]
theme = "clean"
```

`geek` is the default. The name is that of a built-in theme or of a file of `themes/`. The **Appearance** tab of the Preferences (Settings…, ⌘,) lists the themes and writes the one you click in `config.toml`, keeping the rest of the file; its button opens the folder of the themes. A name that names no theme is not an error of `config.toml` (the accounts stay on): geek is applied, with a warning. The change applies at once, without restarting.

`ui.title_bar` is replaced by `title_bar` in the `[window]` table of the theme: `shown`, `hidden`, or `merged`, the red, yellow and green buttons over the left end of the status bar, which leaves them room (with the status bar at the bottom or hidden, the title bar is shown). Written in `config.toml`, `ui.title_bar` is still applied, with a warning.

## The files

The themes are in the folder `themes/`, next to `config.toml` (`~/.config/nicemailer/themes/`, or next to the file `NICEMAILER_CONFIG` names). `nicemailer config path` prints it.

At its first start, the core creates `geek.toml`, `clean.toml` and `regular.toml` there, with every key commented out at its value, after a line that says what it does. They change nothing until you remove the `# ` in front of a line. A key left commented keeps the value NiceMailer ships, even when a later version changes it. An existing file is never touched; a file you delete stays deleted, and the built-in theme stays.

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

Without `extends`, the parent is geek, with a warning. A theme can extend a theme of yours that extends a built-in one, up to 8 levels. A parent is used as it is, your changes included: a theme that extends clean follows what you changed in `clean.toml`.

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
| `[window]` | `chrome` (`status_bar` or `toolbar`), `status_bar_position` (`top`, `bottom`, `hidden`), `title_bar` (`shown`, `hidden`, `merged`: the red, yellow and green buttons in the bar), `key_hints`, `block_labels` (the blocks of actions say their name), `status_bar_height`, `block_style` (`plain`, or `pill`: each block in a pill, the account and the folder in bold, the block open highlighted), `bar_line` (a line between the bar and the mails), `blocks`, `default_columns` (1 to 3) |
| `[fonts]` | `interface`, `reading`, `reading_scale`, `reading_line_spacing` (points between two lines of the body, whatever the text size), `title_scale` |
| `[layout]` | `row_height`, `row_radius`, `row_inset`, `zebra`, `menu_row_height` (0: the height of the text), `popup_radius`, `popup_shadow`, `popup_padding` (the margin inside menus, pickers and panels), `menu_keys` (`left`, `right`), `message_cards`, `avatars`, `reader_max_width` (0: the whole width), `scrollers` (`none`, `thin`, `standard`), `separators` (`lines`, `space`) |
| `[motion]` | `sidebar_ms`, `popup_ms` (0 when System Settings reduce motion) |
| `[colors]` | `background`, `sidebar`, `list`, `bar`, `text`, `muted`, `accent`, `accent_text`, `selection`, `selection_text`, `marked`, `marked_bar`, `separator`, `popup`, `unread`, `flag`, `error`, `virtual_folder` |

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

- **The core** (`crates/nm-engine/src/theme/`): `catalog.rs` lists every key, its type, its bounds and its role; the built-in themes are `geek.toml`, `clean.toml` and `regular.toml`, embedded with `include_str!`, each giving every key a value (a test checks it). `mod.rs` reads a file (`parse`, with `toml_edit` for the line of each problem), resolves the inheritance (`resolve`), checks the contrast, builds the view (`view`), writes the commented files (`template`, `new_theme_file`), and follows the folder (`ThemeSet`, which keeps the last valid version of each file). `config_write::set_theme` writes `ui.theme`, keeping the rest of `config.toml`.
- **The daemon** reads the folder at start, watches it (`watch::watch_dir`), follows `ui.theme` after each change of `config.toml`, and answers `theme_get`, `theme_list` and `config_set_theme` ([contract.md](contract.md)). It emits `ThemeChanged` when the theme in effect changes, and `ConfigError` with `file` `themes/<name>.toml` for a refused file.
- **The app** (`ThemeStyle.swift`) embeds the three built-in themes resolved (`Resources/themes-builtin.json`, from `nicemailer theme show --builtins --json`; a Rust test checks it is up to date), its theme until the core answers. `ThemeStore.shared` holds the theme in effect, read with `theme_get` at the start of a session and at each `ThemeChanged`. The colors of `Palette` are dynamic colors that read the theme's when AppKit draws, so the views kept their names; a new theme rebuilds the window (`ThemeStore.revision`, as a new text size does). `Fonts.interface` gives the theme's interface font, and `TextMeasure` measures text in it: the list (`RowLayout`), the key column of the menus, the help (`Help.columnCount`) and the attachment tiles are sized with it, in characters for a font of fixed pitch, as before, and on the measured texts for a proportional one. What a view draws differently from one theme to another is asked of `Look` (`ThemeLook.swift`): the rows (`rowFill`: inset, radius, alternate rows), the panels (`popupChrome`: radius, shadow), the scroll bars, the separators, the side of the keys in a menu, the title of a menu (in a theme of rounded panels, a small grey heading over a line, as a section of a macOS menu) and the room inside its highlighted row, the animations of the folder column and of the menus (none in geek, nor when System Settings reduce motion), the width of the reader and the size of its subject. The HTML reader and the editor take the theme's reading font (`Fonts.css`). The Appearance tab is `Preferences/ThemeChooser.swift`. Tests: `ThemeTests.swift` (with `ThemeChooserTests`), `TextMeasureTests.swift`, `themes.uiscript`, `themes-refused.uiscript`, `themes-clean.uiscript`, `preferences-appearance.uiscript`.
- **Tests**: `crates/nm-engine/src/theme/tests.rs` (reading, inheritance, contrast, view, templates, the folder), `crates/nm-engine/tests/watch.rs` (the folder watched), `crates/nm-daemon/tests/theme_rpc.rs` (end to end), `config_write` (writing `ui.theme`).
