# The editor of a mail's text

How the text of a reply, a forward or a new mail is edited, and what the core makes of it. The rules are those of `specifications/edition.md`.

## Two ways of working

The app has two interfaces. **Navigating** (most of the time) is made of keys without modifier, NiceMailer's own. **Writing a mail** follows the Mac: the text is a standard text component, with the Mac's keys and integrations, and the app injects only its few commands, all with a modifier.

The component is **Tiptap** (an editor built on ProseMirror), in a web view, `apps/editor`. It is structured by a schema: nothing enters the text that is not in it, whether typed, pasted, dropped, or written back by another app (Writing Tools, a grammar checker). It does not depend on macOS: the protocol it speaks (`apps/editor/PROTOCOL.md`) is the same for any host, the future web client included.

The recipients and the subject stay the app's own fields (`LineEdit`, [sending.md](sending.md)).

## Two modes

| | Text only (default) | Formatted |
| --- | --- | --- |
| Schema | paragraphs, one per line | below |
| Formatting bar | none | always shown |
| `draft.body_format` | `text` | `html` |
| `draft.body` | the text | Tiptap's HTML |
| Mail sent | `text/plain` | text and HTML, or text only without any formatting |

`⇧⌘T` switches, as "Make Plain Text" in Mail. To formatted: each line becomes a paragraph, and the lines starting with `>` become a quotation (`>>` one inside it), so the quoted mail of a reply becomes a quotation block. To text only: the core makes the text of the HTML (`html_to_text`, the same text as the text part of the mail sent); when the draft has formatting that would be lost, a first `⇧⌘T` says so ("The formatting will be lost · ⇧⌘T again to convert") and a second one within five seconds converts.

`compose.format = "html"` in `config.toml` makes a new mail, a reply and a forward start formatted. The core still proposes the draft as text (`prepare_draft`); the editor shows it converted, and it stays a text draft until something changes, so that a proposal left as it was is written nowhere.

## The formatted mode

| Element | Keys | Typing shortcut |
| --- | --- | --- |
| Bold, italic, underline | ⌘B, ⌘I, ⌘U | `**x**`, `*x*` |
| Headings 1 to 3 | ⌥⌘1 to ⌥⌘3 | `# `, `## `, `### ` at the start of a line |
| Bulleted, numbered list | ⇧⌘8, ⇧⌘7; ⇥ ⇧⇥ indent and outdent an item; Return on an empty item ends the list | `- `, `1. ` |
| Quotation | ⇧⌘B | `> ` |
| Link | ⌘K: the address, and the text when nothing is selected; `http`, `https` and `mailto` only (`exemple.fr` becomes `https://exemple.fr`, an email address `mailto:`) | an address typed or pasted becomes a link |
| Font, size, colour | the bar's menus | |
| Line break inside a paragraph | ⇧↩ | |

The typing shortcuts format as one types; no Markdown is ever stored, and NiceMailer reads none.

**Fonts, sizes, colours**, given to the editor with each draft (the `load` message):

- fonts: Default (no font: the reader's own), then every font family of the Mac (`MacFonts`), under "Recently used" (the five chosen last, remembered in `editor.recentFonts`) and "All fonts". Each is written with a fallback, so that a reader without it reads the same kind of font: those found on every system with the stack of the core (`nm_core::formats::FONTS`: `Georgia, 'Times New Roman', serif`), the others with their generic family, read from the font's traits (`'Avenir Next', sans-serif`, `'Menlo', monospace`). The Mac's families are read once, in the background, when the editor is created (about half a second for 350 fonts). The core keeps any font whose name is safe in CSS (letters, digits, spaces, `- . _ &`; `formats::font_stack`): it cannot know the fonts of the Mac;
- sizes: Normal (no size), then Apple Mail's sizes, 9 to 288 px (`nm_core::formats::SIZES`);
- colours: Default (the reader's), and eight hues of middle lightness, each with a contrast of at least 3:1 on a white page and on a dark one (a test checks it): grey, red, orange, green, teal, blue, purple, pink.

A value outside the lists (a paste, a draft written elsewhere) is dropped, its text kept: in the editor, a font that is not on the Mac; in the core, a font name that is not safe. Not in the schema: tables, images, code, strike, rules, alignment, background colour.

## Keys in the text

The app sees each key before the editor (`GeekController.handleComposeKey`, then `handleBodyKey`) and keeps only these; every other key is the editor's, that is the Mac's.

| Key | Effect |
| --- | --- |
| ⌘↩, ⇧⌘↩ | send, send then file ([sending.md](sending.md)) |
| ⌥⌘F, ⌥⌘B, ⇧⌘A | sending address, Bcc line, attach files |
| ⌃I | the AI menu; ⌘I is the italic |
| ⌘⎋ | abandon the draft; ⌘⌫ is the Mac's "delete to the start of the line". Not ⌥⎋, macOS's "Speak selection" |
| Escape | back to the mail, the draft kept; the editor's while its link dialog or a menu of its bar is open |
| ⇧⌘T | text only ↔ formatted |
| ⌥⇧⌘V | paste the text alone, at the style of the cursor ("Paste and Match Style") |
| ⌘V | paste; files copied in the Finder are attached instead |
| ⌘+ ⌘− ⌘0 | the size of the app's text, never WebKit's zoom |

⇥ and ⇧⇥ are the editor's: in a list they indent and outdent; elsewhere ⇧⇥ goes back to the field before the text (the editor's `leave`), and ⇥ types a tab in text only.

## Pasting

⌘V pastes with the formatting in the formatted mode: the HTML of the pasteboard is read through the schema (Tiptap), so a page from Safari keeps its headings, lists, bold and links, and loses its styles, tables and images. A text written in Markdown elsewhere is pasted as HTML: Obsidian can copy a note as HTML (to check: it must put real HTML on the pasteboard, not its source as text). In text only, ⌘V pastes the pasteboard's text, even when it also holds HTML (a copy from Obsidian, Safari or Notes): read through the text schema, the HTML would give a line per paragraph and lose the empty lines between them; only a pasteboard with HTML and no text goes through the schema. A plain text pasted keeps every line, the empty ones included. A copy (⌘C, ⌘X) gives the text as written: a line per paragraph, heading or list item, the empty ones included, and not Tiptap's empty line between blocks; otherwise a corrector that copies the selection and pastes its fix back (Kerlig) would double every line break.

The cases of `apps/editor/test/fixtures` (Obsidian, Safari, Pages, Gmail, Notes) are checked by the editor's tests. Word is not handled specially yet: its HTML goes through the same schema.

## What leaves

The core decides what leaves, whatever the editor produced (`nm-store`, `mail_html.rs`):

1. **Closed list** (`mail_html::clean`, ammonia): `p br h1 h2 h3 ul ol li blockquote strong em u a span`; `href` on `a`, in `http`, `https` or `mailto`; `start` on `ol`, a number; `style` on `span`, only `color`, `font-family`, `font-size`, to a value of the lists. `<script>` and `<style>` go with their content, any other tag leaves its text.
2. **Styles on the tags**: Gmail and Outlook often ignore a `<style>` block, so each tag gets a `style` from a fixed table (no margin on paragraphs, as in the editor; heading sizes; the grey bar of a quotation). No colour is imposed but the user's.
3. **Document**: `<!DOCTYPE html>`, `<meta charset="utf-8">`, no remote resource; an empty paragraph gets a `<br>` so the blank line stays.
4. **Text part** (`mail_html::to_text`): a paragraph per line, a heading on its line, `- ` or `1. ` before each list item (nested ones indented under the text), `> ` before each quoted line (`>> ` nested), a link whose text is not its address written `text <address>`. It is not the reader's conversion (`html_text.rs`), which is tuned for any received HTML: changing it would change how every received mail is shown and indexed.
5. **MIME** (`compose::build_message_with_html`): a formatted draft with some formatting (a heading, a list, bold, italic, underline, a styled span, a link whose text is not its address) leaves as `multipart/alternative`, the text then the HTML (`quoted-printable`); with files, inside the `multipart/mixed`. Without formatting (paragraphs, quotations and plain links only), it leaves as text only, as a text draft.

The copy in Drafts is built the same way. A formatted body larger than 2 MB is refused.

## The AI

The AI reads and rewrites text: a formatted draft is not offered to it yet (the app says so, and the core refuses `ai_run` on one). In text only, a change of the AI the core applied goes to the editor as one range (`replaceRange`), one step of the editor's undo: ⌘Z gives the words back.

## In the app

- **One web view**, `EditorBridge.shared` (`Geek/EditorWebView.swift`), created and loaded once the first list is shown (`AppModel.ensureEditor`: WebKit starting during the launch slowed the first list) and kept hidden, so that `r` opens the editor without waiting for WebKit; a reply opened before is the one that creates it. Keys typed for the text while its page still loads are kept by the app and typed into it once it is ready (`EditorBridge.keepKeyUntilReady`): none is lost. Each opening sends `load`. If WebKit's process for the page stops, the page is loaded again; the draft is at its last save.
- **Isolated**: the page is served from the app's resources under its own scheme `nm-editor:` (`Resources/Editor`, built by `make editor`), with a Content Security Policy (`default-src 'none'`), the reader's block list of remote loads, no navigation (a click on a link opens nothing), no persistent storage. The app's font is given as data URLs.
- **Saving**: the editor reports the content after each pause of typing (`changed`, 0.5 s), and the app saves it then (`update_draft` with `body_format`). ⌘↩, Escape and ⌘⎋ first ask for the content as it is (`flush`, answered at once): the very last key typed is never lost. Without an answer within a second, the mail is not sent ("Not sent: the text could not be read · try again").
- **Files dropped on the text** are attached (`EditorWebView.performDragOperation`), never put in it.
- **Focus**: the editor gives itself the focus at once (`view.focus()`), not through Tiptap's `focus` command, which waits for the next frame: WebKit draws no frame for a page it does not show, and a reply opened in a hidden window would never get the keys.
- **The bar** is always shown in the formatted mode, and never in text only: ⇧⌘T is the switch. Its texts come from the app's String Catalog, in `load`: the editor has no translation of its own.

## Building

`make editor` installs the editor's dependencies from the lock file (`npm ci`) and bundles it with esbuild into `apps/NiceMailer/Sources/NiceMailer/Resources/Editor` (`editor.js`, `editor.css`, `index.html`, and `THIRD_PARTY_LICENSES.txt`, the licences of the packages bundled). The bundle is never committed: every target that builds the app (`build`, `app`, `test`, `ui-test`, `bundle`) depends on `make editor`, which needs Node (`apps/editor/.nvmrc`). Every dependency is pinned exactly, and all are MIT; no paid extension of Tiptap is used.

## Tests

- **Editor** (`make editor-test`: `tsc` and Vitest with jsdom, in `make test`): the schema and the values outside the lists, the HTML it gives back unchanged (a draft reopened), the pastes of the fixtures, plain text pasted line by line, the conversions of text only, the offsets of the text, every message of the protocol (`changed` once at the pause, `flush` at once, `setMode`, `replaceSelection` and `replaceRange` in one step of undo, `state`), the keys (⇥ ⇧⇥ in and out of a list, ⌘B ⌘I ⌘U, the typing shortcuts, ⌘K and its refusals), the bar (shown in the formatted mode only, the font menu and its recent fonts, the sizes).
- **Core**: `nm_core::formats` (the lists, the spellings recognized, any safe font name kept and the others refused, the contrast of the colours), `compose` (the alternative, inside the mixed part, the boundary), `mail_html` (the closed list, the links, the styles, the document, the text, the formatting detected), `tests/drafts.rs` (a draft switched to HTML, an unknown format and a huge HTML refused, a formatted draft sent as text and HTML, without formatting as text only, with files, an empty formatted mail dropped).
- **App** (`MacFontsTests`: the stacks of the Mac's fonts, the names refused, the generic family, the recent fonts; `ComposeTests`, `AITests`, against `FakeCore` and `FakeEditor`): the keys left to the editor and those the app takes, Escape while the editor's dialog is open, ⌥⇧⌘V, a click in the text, ⌘↩ and Escape taking the last keys, no sending without an answer, ⇧⌘T both ways and its confirmation, the AI refused on a formatted draft, the minimal range of an AI change.
- **UI scenarios** (`UITests/editor.uiscript`): real keys typed in the web view. The window of UI script mode says it is the key window (`ScriptWindow`) without the app being activated, since WebKit gives its page the keys only in the key window; the app stays in the background, off screen. ⌘V reads the Mac's own pasteboard, which a scenario must never touch: the pastes are checked by the editor's tests, and ⌥⇧⌘V (which reads the app's private pasteboard) by the scenarios.
