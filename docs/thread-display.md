# Simplified thread display

When a thread is opened, the core returns each message cut into typed **segments**, and the app shows only what each message brings. Everything hidden stays one key or one click away. The work is done by the core (`crates/nm-store/src/clean.rs`, called by `get_thread`), so every client gets the same view.

## What other mail clients do

| Client | Practice |
| --- | --- |
| Gmail | Trims the quoted history at the end of a message (and repeated signatures) behind a "···" button; folds read messages of a conversation to one line, keeping the latest and the unread ones open. |
| Outlook | "Clean Up Conversation" removes messages whose whole content is quoted in a later one. |
| Apple Mail | Quote levels shown with colored bars; on iOS the final quote is folded behind "See More". |
| HEY | Clips quoted history so each reply shows what it adds. |
| Thunderbird, Fastmail | Collapse or color quoted text on demand. |
| mutt / neomutt | `T` toggles quoted text, `S` skips the next quoted block. |

Shared rules, kept here: only the **final** quote is trimmed (interleaved quotes give the context of point-by-point answers); content that appears nowhere else is never removed; trimming is visible and reversible.

## Segments

| Kind | What | Default |
| --- | --- | --- |
| `text` | What the sender wrote | shown |
| `quote` | Quoted lines interleaved with answers | shown dimmed, one bar per quote level |
| `quote_tail` | The quoted history at the end, with its attribution ("On … wrote:", "Le … a écrit :", German, Spanish, Italian forms, possibly over two lines) or its Outlook header block ("From: … Sent: … To: …", "De : … Envoyé : … À : …", after `________` or `-----Original Message-----`) | hidden: "··· N quoted lines", or "··· N quoted lines, not in this conversation" when its content is in no earlier message |
| `signature` | After the `-- ` delimiter, or closing lines a sender repeats in several messages (at least two lines) | hidden: "··· signature" |
| `boilerplate` | "Sent from my iPhone", "Envoyé de mon iPhone", "Get Outlook for…", confidentiality notices | hidden |
| `forward` | A forwarded message ("Forwarded message", "Message transféré", "Début du message réexpédié") | shown, never treated as history |
| `copy` | Text (40 characters or more, compared ignoring case, punctuation, wrapping and quote markers) found in an earlier message | hidden: "··· copy of an earlier message" |

A message whose visible content is empty is flagged `only_quotes`: folded, it shows "only quotes earlier messages" instead of a snippet.

## Folding and headers

As in Gmail, the latest message and the unread ones are open (`expanded`); the others are one line: sender, date, beginning of what they bring. A message that only quotes stays folded unless it is the latest.

Headers are light: sender and date ("14:05", "yesterday 14:05", "3 Nov 14:05", "3 Nov 2021"). Recipients are shown only when they differ from the previous message's (`show_recipients`; for the first message, when there are several).

## In the app (geek theme)

| Action | Effect |
| --- | --- |
| `T` | Shows everything in this thread (all messages open, every hidden segment shown), or goes back to the simplified view. Per thread, for the session. |
| `S` | Scrolls to the text after the next quoted block. |
| Click on a header | Folds or unfolds that message. |
| Click on a "···" line | Shows that part only. |

In the HTML view, messages are `<details>` elements (they fold without JavaScript). The core's sanitizer keeps the markers mail apps put on quoted history and signatures (Gmail's `gmail_quote` and `gmail_signature`, Thunderbird's `moz-cite-prefix` and `moz-signature`, Apple Mail's `blockquote type="cite"`, Outlook's `divRplyFwdMsg`, prefixed `nm-`), and the app hides them unless `T` shows everything. Messages imported before this change keep their old sanitized HTML (parser version 1) until a reindex.

## HTML of the sender

The HTML view shows the mail as its sender laid it out, CSS included, each message in a frame of its own; nothing it holds is loaded from the network in the html display.

**What the core keeps** (`ingest.rs`, `sanitize_html`; parser version 10):

- The sender's **CSS**, sanitized by `css.rs` with a real CSS tokenizer (`cssparser`, never regular expressions): every `style` attribute, and the `<style>` blocks, gathered in one block at the start of the body. Those inside comments (Outlook's `<!--[if mso]>`) are left out.
  - Every `url()` is rewritten: a remote image to `nm-remote://image?u=…`, a remote font to `nm-remote://font?u=…` (neither is loaded in the html display), a `data:` image or font kept; any other URL drops its declaration. A URL under `:hover`, `:focus`, `:active`… or in a `@media` on the screen's size is dropped too: loading it would tell the sender what the reader does, or which device it uses. `prefers-color-scheme` is allowed.
  - `@import`, `@charset`, `@namespace`, `@keyframes` and the other at-rules go; `@media`, `@supports` and `@font-face` stay.
  - `expression()`, `image-set()`, `behavior`, `-moz-binding`, `position: fixed` and `sticky` go.
  - Selectors are adapted to the frame: `:root` and `html` become `:host`, `body` becomes `.nm-body`, `#x` becomes `#nm-x` (ids are prefixed in the HTML).
  - The result never holds `<`: a block cannot close its `<style>` element. A sheet larger than 256 kB, or a `style` attribute larger than 16 kB, is dropped.
- **Classes and ids** (the CSS selects on them), and the layout and colour attributes of older mails (`LAYOUT_ATTRIBUTES`): table widths and heights, `border`, `cellpadding`, `cellspacing`, `bgcolor`, the alignment of cells and paragraphs, `<font face size color>`, `<tfoot>`, `dir`. `background` (an image URL) is left out.
- The document's `<title>` is removed with its text (it used to show at the top of the mail).

**How the app shows it** (`MailFrame.swift`):

- Each HTML message is in its own **shadow root**, written in the page (`<template shadowrootmode="open">`, no script needed). The CSS of a mail applies to it alone: not to the other messages of the thread, nor to the app's lines (headers, attachment tiles), and the app's styles do not enter it, except the inherited text font and colour. `contain: layout paint` makes the frame the box of any positioned element of the mail, and clips what overflows it.
- **Colours**: a mail that brings its own dark mode (`prefers-color-scheme` rules in its CSS) is shown with its colours, the dark ones in a dark theme, as its author designed them. Any other mail is, for now, shown on a white sheet in the dark theme, as written.
- Fonts: a font the mail names is used when the Mac has it (most newsletters fall back on `Helvetica`, `Arial`, `sans-serif`); the theme font otherwise. Web fonts are not loaded.
- **Embedded images** (`cid:`, a logo or a signature carried by the mail) are shown, in the html and images displays alike: nothing is requested from the network. `get_thread` gives each one a source in the app's scheme, `nm-cid://image/<attachment id>` (`attachments.rs`, `resolve_inline_images`; the stored HTML keeps `cid:`), and the app's handler serves the part the core extracts (`get_attachment`; `InlineImages.swift`): images only, SVG excepted, 10 MB at most, and only the ids of the page. A mail that writes an `nm-cid:` source itself loses it in the sanitizer, so it cannot show another mail's attachment.
- The page carries a **Content Security Policy** (`ContentPolicy`): nothing loads but the app's schemes (`nm-remote:`, `nm-cid:`) and `data:`; no script, frame, form or `<base>`. It is a third barrier after the sanitizer and WebKit's block list (`RemoteBlocker`), and stops a remote load on its own (`RemoteBlockerTests`). The remote fonts of a mail are refused by it in every display, for now.

Bodies stored by an older parser are rewritten by the background reindex: the HTML ones read their raw message again, the others are only marked.

## Limits

- Messages are not deleted, unlike Outlook's cleanup: redundant content is only hidden.
- An attribution written in another language, or a client with an unusual header block, leaves the history visible (as text or quote).
- The HTML view hides quotes only for the markers above, and has no per-part "···" lines: `T` shows everything.

## Tests

`crates/nm-store/src/clean.rs` (unit tests of every rule), `crates/nm-store/tests/thread_display.rs` (a conversation imported then read with `get_thread`), `apps/NiceMailer/Tests/NiceMailerTests/ThreadDisplayTests.swift` (blocks of the reader, toggles, `S`), and the UI scenario `apps/NiceMailer/UITests/thread.uiscript`.

HTML of the sender: `css.rs` (declarations, URLs rewritten or refused, escaped `url(`, at-rules, selectors adapted, nothing closing `<style>`, broken CSS, the `<style>` blocks outside comments), `ingest.rs` (attributes, classes and CSS kept, the title removed, a mail's own `nm-cid:` removed), `tests/reindex.rs` (only the HTML bodies are read again), `tests/attachments.rs` (the `nm-cid:` source of an embedded image), `InlineImagesTests` (ids, URLs refused, parts shown, the handler in a web view through the policy), `RemoteBlockerTests` (the policy alone stops a request), `MailFrameTests` (own dark mode, the frame, and in a web view: the CSS of a mail reaches neither another message nor the page), and `images.uiscript` (table width, sender's font, embedded logo drawn, policy present, the column of the CSS, the hidden preview text, the mail's own dark mode in a dark and a light theme).
