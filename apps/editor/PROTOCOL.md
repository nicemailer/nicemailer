# The editor's protocol

The messages between the editor of a mail's text and its host (`specifications/edition.md`, section 8). This file is the reference for any host: the macOS app (`EditorBridge`, a `WKWebView`) and a web client (an `<iframe>`). The types are in `src/protocol.ts`.

## Transport

One channel of JSON objects, each with a `type`.

| Host | Editor → host | Host → editor |
| --- | --- | --- |
| WebKit (the app) | `window.webkit.messageHandlers.editor.postMessage(message)` | `window.nicemailerEditor.receive(message)`, through `evaluateJavaScript` |
| Web page | `window.parent.postMessage(message, '*')` | `iframe.contentWindow.postMessage(message, origin)` |

The page is `index.html` with `editor.js` and `editor.css` (`node build.mjs --out <dir>`), and its element `#editor`.

## Version

`ready` gives the version (`v`, 1 now); `load` carries the host's. Each refuses the other's if they differ: the editor ignores a `load` of another version, the app logs an editor of another one and never sends it anything.

## Host → editor

| `type` | Fields | Effect |
| --- | --- | --- |
| `load` | `v`, `mode` (`text` or `html`), `content`, `theme`, `strings`, `formats`, `recentFonts`, `focus`, `fromText?`, `cursor?` | Opens a draft. `content` is the text (text only) or the HTML (formatted). `fromText`: in formatted mode, `content` is a text to convert (lines to paragraphs, `>` lines to quotations). `cursor`: in text only, the cursor's place in UTF-16 units (0 by default). No `changed` follows a load. |
| `theme` | `theme` | New colours or text size. |
| `setMode` | `mode`, `content` | Switches mode. `content` is a text in both directions: to formatted, the editor converts it; to text only, it is the host's text of the HTML (the core's `html_to_text`). A `changed` follows at once. |
| `replaceSelection` | `text?` or `html?` | Puts it in place of the selection, in one step of undo. A text is a paragraph per line; an HTML (formatted mode) is read through the schema, as a paste. |
| `replaceRange` | `start`, `end`, `text` | Text only: replaces the range (UTF-16 units of the text), in one step of undo, the cursor after the new words. |
| `flush` | `id` | Asks for the content now; answered by `flushed` with the same `id`, at once. |
| `focus` | | Gives the text the focus. |
| `blur` | | The keys went to another field. |

`theme`: `dark`, `font`, `size` (pixels), and the colours `text`, `background`, `muted`, `accent`, `selection`, `border`, `panel`, as CSS values.

`formats`: `fonts` (`{name, stack}`: every font of the host's machine, the menu's), `sizes` (`{key, css}`: the number shown, the CSS value), `colors` (`{key, hex}`); the sizes and colours are those of `compose_formats`. `recentFonts`: the names of the fonts chosen last, most recent first, shown at the top of the font menu.

`strings`: the texts of the interface, by key (`toolbar.bold`, `font.recent`, `color.blue`, `link.url`…), in the host's language.

## Editor → host

| `type` | Fields | When |
| --- | --- | --- |
| `ready` | `v` | The page is loaded. Messages sent before are lost: the host queues them. |
| `changed` | `content`, `mode`, `revision` | At the first pause of 0.5 s after a change, once; and at once after `setMode`. `revision` counts the changes since the last `load`. |
| `flushed` | `id`, `content`, `mode`, `revision` | The answer to `flush`. Revision 0: nothing changed since the load. |
| `state` | `modal`, `focused`, `textRange`, `empty`, `selection`, `marks` (`bold`, `italic`, `underline`, `link`), `block` (`paragraph`, `h1`…`h3`), `bulletList`, `orderedList`, `blockquote`, `color`, `font`, `size` | What is under the cursor, at most once per turn of the event loop. `modal`: a dialog of the editor (the link) or its bar has the keys, Escape is its. `textRange`: in text only, the selection in UTF-16 units of the text. `selection`: the text selected, 20,000 characters at most. |
| `leave` | `direction` (`previous`) | ⇧⇥ outside a list: the host gives the keys to the field before the text. |
| `fontUsed` | `name` | A font was chosen in the bar's menu: the host puts it first in `recentFonts`. |

## Rules for a host

- Send `load` only when a draft opens, `replaceSelection` and `replaceRange` only on an action of the user: never the content back at each refresh, or the cursor and the undo history are lost.
- Before sending or closing a draft, `flush` and use its answer: `changed` comes only after the pause.
- Keep the editor isolated: no network (a Content Security Policy and a block list), no navigation, nothing persistent.
- Keep the content the editor gives as the draft's body; the core cleans what leaves (`nm-store`, `mail_html.rs`).
