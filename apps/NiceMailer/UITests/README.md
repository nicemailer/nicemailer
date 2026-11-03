# Scripted UI tests

Each `*.uiscript` file is a scenario played by the app itself in **UI script mode**: it sends simulated keys and mouse clicks to its own window and checks the resulting state. Run them all with `make ui-test` (or `scripts/ui-test.sh scenario.uiscript` for one).

For each scenario, `scripts/ui-test.sh`:

1. creates a fresh data directory with fake mail (`nicemailer seed --threads 300 --seed 1`, account `perso`) and imports `fixtures/sample.mbox` (account `fixture`, address `me@example.com`: an HTML newsletter, a thread with a link, a long mail, a four-message thread with quotes, signatures and a mobile mention);
2. starts a core on it;
3. runs the app with `NICEMAILER_UI_SCRIPT=<scenario>`, `NICEMAILER_EXPORT_DIR=<temporary directory>` and `-AppleLanguages (en)` (English messages whatever the machine's language);
4. prints `PASS` or `FAIL` with the failed lines, then removes the data directory.

Results go to `apps/NiceMailer/.build/ui-test/<scenario>/`: `result.txt` (every check), the snapshots asked by `snap`, and `fail-line-N.png` for each failed check.

## The app stays out of your way

In UI script mode the app is never activated (no Dock icon, no keyboard focus), its window is borderless, transparent and off screen, it uses its own emptied preferences domain, and it exports into the temporary directory. You can keep working while the tests run: your typing never reaches it.

## Script language

One command per line; `#` starts a comment.

| Command | Effect |
| --- | --- |
| `key K` | one key: a character (`j`, `J`, `;`, `*`…), `esc`, `ret`, `del`, `up`, `down`, `space`, `shift-space`, `ctrl-X` or `cmd-X` |
| `type TEXT` | each character of TEXT as a key |
| `click TARGET [cmd] [shift]` | a click on a row, with modifiers |
| `dblclick TARGET` | a double-click |
| `resize WxH` | window size (default 1400×900) |
| `snap NAME` | PNG of the window (a web view, drawn in another process, appears empty: check it with the `html_*` fields) |
| `wait MS` | pause |
| `expect FIELD OP VALUE` | check a state field; `OP` is `=`, `!=`, `~` (contains), `!~` (does not contain), `>` or `<`; waits up to 5 s for the condition |

Targets: `thread:N` (N-th row of the mail list, from 0), `search:N`, `picker:N`, `screen:N` (row of the contacts, history, tags or groups screen), `folder:NAME` (folder column of the current view, 3-column layout), `reader-link:N` (N-th header or "···" line of the text reader), `status:account`, `status:folder`, `status:attributes`, `status:m`, `status:s` and `status:v` (the blocks of the status bar), `menu:N` (N-th row of the open prefix menu), `chip:ID` (an attribute chip: `seen`, `from:0`, `contains`, `date`…).

State fields (see `Runner.state()` in `Sources/NiceMailer/UIScript.swift`): `screen`, `overlay`, `mode`, `location`, `flash`, `error`, `folder`, `account`, `thread_count`, `selected` (index), `selected_subject`, `selected_unread`, `selected_flagged`, `selected_tags`, `marked` (indices, comma-separated), `marked_count`, `search_text`, `search_count`, `search_index`, `picker_query`, `picker_index`, `picker_count`, `screen_index`, `contacts_count`, `history_count`, `tags_count`, `groups_count`, `columns`, `status_position`, `sort`, `body_format`, `reader_subject`, `reader_offset`, `list_rows` (rows of the mail list that fit, for `ctrl-d` / `ctrl-u`), `list_filter`, `export_files`, `html_text`, `html_font_loaded`, `html_font_family`, `font_size`, `row_height` (height of the selected row, as laid out), `reader_text` (text shown by the text reader), `reader_links` (its header and "···" links), `reader_folded` (folded messages), `reader_folder` (folder shown next to the subject in "Everything", empty elsewhere), `show_all` (`T`), `filters` (labels of the attribute chips, comma-separated), `menu_x` and `menu_y` (position of the first row of the open menu, window coordinates).

## Scenarios

| File | Covers |
| --- | --- |
| `filters.uiscript` | attribute selector: toggles, From… panel with suggestions, Date… preset, Contains…, chips, kept across folders, Escape; folder (and account in "all") of each mail in Everything |
| `navigation.uiscript` | `j` `k` arrows, `ctrl-d` `ctrl-u` half pages, `J` `K` marks, Escape, click, ⌘-click, ⇧-click |
| `actions.uiscript` | `m` menu and moves, `d`, Delete, grouped actions, flag, unknown keys, reply not available, help |
| `screens.uiscript` | `;` folders, contacts (keys, filter, click, double-click, mails of a contact), history, tags, groups |
| `pickers.uiscript` | `;` menu (keys, and mouse from the status bar), link, tag and group pickers (keys and mouse), search (keys, results turning around at both ends, `ctrl-u`, click, double-click) |
| `statusbar.uiscript` | account and folder blocks of the status bar, unified view "all", account picker keeping the folder role, turning around at both ends, single-block folder column |
| `textsize.uiscript` | ⌘+ ⌘= ⌘− ⌘0, bounds, row height, keys and clicks after the rebuild |
| `thread.uiscript` | simplified thread: folded messages, hidden quotes and signatures, `T`, clicks on headers and "···" lines |
| `view.uiscript` | 1 / 2 / 3 columns, folder column clicks, status bar, sort, text / HTML with the bundled font, scrolling (Space, shift-Space, `ctrl-f` `ctrl-b`), saving |

## Writing a new scenario

Make clickable rows targetable with `.uiTarget("kind-\(id)")` (no cost outside UI script mode), expose any new state the scenario needs in `Runner.state()`, then add a `.uiscript` file and a line to the table above.
