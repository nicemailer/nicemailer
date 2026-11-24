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
| `quote_tail` | The quoted history at the end, with its attribution ("On … wrote:", "Le … a écrit :", and the forms of Apple Mail, Gmail, Outlook and Thunderbird in German, Spanish, Italian, Portuguese, Dutch, Swedish, Danish, Norwegian, Polish, Russian and Chinese, possibly over two lines; without any word, as Gmail writes it in Japanese or Russian, a line ending with a colon that holds an address and a figure) or its Outlook header block ("From: … Sent: … To: …", "De : … Envoyé : … À : …", "Von: … Gesendet: … An: …", the Italian, Spanish, Portuguese, Dutch and Swedish ones, after `________` or `-----Original Message-----` in those languages) | hidden: "··· N quoted lines", or "··· N quoted lines, not in this conversation" when its content is in no earlier message |
| `signature` | After the `-- ` delimiter, or closing lines a sender repeats in several messages (at least two lines) | hidden: "··· signature" |
| `boilerplate` | "Sent from my iPhone", "Envoyé de mon iPhone", "Get Outlook for…", confidentiality notices | hidden |
| `forward` | A forwarded message ("Forwarded message", "Message transféré", "Début du message réexpédié") | shown, never treated as history |
| `copy` | Text (40 characters or more, compared ignoring case, punctuation, wrapping and quote markers) found in an earlier message | hidden: "··· copy of an earlier message" |

A message whose visible content is empty is flagged `only_quotes`: folded, it shows "only quotes earlier messages" instead of a snippet.

## Folding and headers

As in Gmail, the latest message and the unread ones are open (`expanded`); the others are one line: sender, date, beginning of what they bring. A message that only quotes stays folded unless it is the latest.

Headers are light: sender and date ("14:05", "yesterday 14:05", "3 Nov 14:05", "3 Nov 2021"). Recipients are shown only when they differ from the previous message's (`show_recipients`; for the first message of a conversation, when there are several) — **always for a mail alone in its conversation**, which has nothing to be compared with (a mail sent to the user only shows "To: me@…"), and **always for a mail the user wrote**, whose sender says nothing and whose recipients are the whole point (added in November 2026). A mail is the user's own when its sender is one of the mailbox's addresses, or when it sits in Sent or Drafts. The addresses come from `accounts.toml`, which the database does not hold: the daemon passes them to `get_thread`, which adds the archive accounts' own (`account.address`). So a mail sent and then filed in Waiting or Todo (`sending.md`) still shows who it went to.

## In the app (geek theme)

| Action | Effect |
| --- | --- |
| `T` | Shows everything in this thread (all messages open, every hidden segment shown), or goes back to the simplified view. Per thread, for the session. |
| `S` | Scrolls to the text after the next quoted block. |
| `ctrl-k` / `ctrl-j` | Up to the previous message of the conversation / down to the next, one open at a time: the current message folds, the other unfolds and the reader scrolls to its header. It starts from the last unfolded message, or from the last header clicked; it stops at either end, and the status bar says so. With `T`, nothing folds: it only goes from header to header. A single mail has nowhere to go. |
| Click on a header | Folds or unfolds that message. |
| Click on a "···" line | Shows that part only. |

**One line per message** (`ThreadDisplay.blocks`, `senderAlert`): "name · date · address". Unfolded, the sender's address in full ("▾ Alice Martin · 30 oct. 14:00 · alice@atelier-dupuis.fr"), the recipients, when worth showing (To, Cc, by name), on the next line. Folded, the domain only, then the snippet ("▸ Alice Martin · 30 oct. 11:00 · @atelier-dupuis.fr  Peux-tu…"). Without a name, the address is the name and is not repeated. The arrow (▸ folded, ▾ unfolded) only in a conversation, where a click on the line folds or unfolds the message; a single mail has no arrow and no folding: it is always unfolded. The address is in the alert colour when the mail failed its authentication, or when the name shows another address or domain than the real one (`"service@paypal.com" <x7@promo-mail.ru>`, "PayPal.com"); a domain of the name that the real domain ends with does not count ("Le Monde.fr" from `news.lemonde.fr`). A click on the address copies it; ⌘-click filters the list on this sender (a "from:" chip). The same in the HTML view, recipients included: both readers take the line from `ThreadDisplay.recipients`.

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

- **Text size**: the page is written for a 12 pt text, and the web view zooms all of it by the text size over 12 (`HTMLReader`, `pageZoom`: × 4/3 at the default 16 pt; ⌘+ ⌘− change it), the sizes the mail writes in `pt` or `px` as the rest. The app's own lengths are given before the zoom; the column of the page (12 pt of margin, `layout.reader_max_width`) is divided by it, so that once zoomed it is the column of the title above. No text is drawn smaller than the theme's `reader.html_min_font_size` (11 pt for a 12 pt text), grown with the zoom; WebKit compares its minimum with the zoomed size, so the app gives it the minimum times the zoom, rounded to a whole point (`Look.htmlMinFontSize`). Tests: `html-text-size.uiscript`, `ReaderColumnTests`, `HTMLMinFontSizeTests`.

- Each HTML message is in its own **shadow root**, written in the page (`<template shadowrootmode="open">`, no script needed). The CSS of a mail applies to it alone: not to the other messages of the thread, nor to the app's lines (headers, attachment tiles), and the app's styles do not enter it, except the inherited text font and colour. `contain: layout paint` makes the frame the box of any positioned element of the mail, and clips what overflows it.
- **Colours**: a mail that brings its own dark mode (`prefers-color-scheme` rules in its CSS) is shown with its colours, the dark ones in a dark theme, as its author designed them. Any other mail is made for a white page: in the dark theme, **Dark Reader** adapts it (below); in the light theme it keeps its colours.
- **Dark Reader** (`DarkMode.swift`; `Resources/DarkReader`, the npm package `darkreader` 4.9.134 under the MIT License, unchanged) rewrites each colour of the page for a dark background, keeping its hue: a white background becomes the theme's (`#1B1D23`), dark text becomes light (`#C3C6C1`, the soft white of the theme), an orange button stays orange. It is only added to a page holding a mail without a dark mode of its own.
  - It runs in a content world of its own (`WKContentWorld`), isolated from the page, whose JavaScript stays off.
  - No network: the function it is given to download images and style sheets refuses every request, and its attempt to add a script to the page itself is refused by the Content Security Policy.
  - It follows the app's appearance (`DarkReader.auto`), and works at once (`immediateModify`), without waiting for the page to be visible: no white flash, and the UI scenarios run off screen. `requestAnimationFrame` is replaced, in its world only, by a 16 ms timer, since WebKit stops animation frames in a hidden window.
  - The app's own lines keep their colours (`fixes`).
  - Measured on real newsletters: applied within about 100 ms of the page's load. Its source (360 kB) is read once per launch.
  - In a thread that mixes a mail with its own dark mode and one without, Dark Reader also goes over the first: its dark colours stay dark.
  - Without Dark Reader (its file missing from the app), a mail made for a white page is shown on a white sheet, as written.
- Fonts: a font the mail names is used when the Mac has it (most newsletters fall back on `Helvetica`, `Arial`, `sans-serif`); the theme font otherwise. Web fonts are not loaded.
- **Embedded images** (`cid:`, a logo or a signature carried by the mail) are shown, in the html and images displays alike: nothing is requested from the network. `get_thread` gives each one a source in the app's scheme, `nm-cid://image/<attachment id>` (`attachments.rs`, `resolve_inline_images`; the stored HTML keeps `cid:`), and the app's handler serves the part the core extracts (`get_attachment`; `InlineImages.swift`): images only, SVG excepted, 10 MB at most, and only the ids of the page. A mail that writes an `nm-cid:` source itself loses it in the sanitizer, so it cannot show another mail's attachment.
- The page carries a **Content Security Policy** (`ContentPolicy`): nothing loads but the app's schemes (`nm-remote:`, `nm-cid:`) and `data:`; no script, frame, form or `<base>`. It is a third barrier after the sanitizer and WebKit's block list (`RemoteBlocker`), and stops a remote load on its own (`RemoteBlockerTests`). The remote fonts of a mail are refused by it in every display, for now.

Bodies stored by an older parser are rewritten by the background reindex: the HTML ones read their raw message again, the others are only marked.

## Limits

- Messages are not deleted, unlike Outlook's cleanup: redundant content is only hidden.
- An attribution in a language or a form not in `WORDINGS` (`crates/nm-store/src/clean.rs`), without an address, or a client with an unusual header block, leaves the history visible (as text or quote). A sentence of the user's ending with a colon above a quote is not taken for an attribution unless it holds an address.
- The HTML view hides quotes only for the markers above, and has no per-part "···" lines: `T` shows everything.

## Tests

`crates/nm-store/src/clean.rs` (unit tests of every rule), `crates/nm-store/tests/thread_display.rs` (a conversation imported then read with `get_thread`), `apps/NiceMailer/Tests/NiceMailerTests/ThreadDisplayTests.swift` (blocks of the reader, toggles, `S`, the sender's line, domain and alert), `InterfaceControllerTests` (a click on the address copies it, ⌘-click filters), and the UI scenario `apps/NiceMailer/UITests/thread.uiscript` (the domain, the address, its click and ⌘-click).

HTML of the sender: `css.rs` (declarations, URLs rewritten or refused, escaped `url(`, at-rules, selectors adapted, nothing closing `<style>`, broken CSS, the `<style>` blocks outside comments), `ingest.rs` (attributes, classes and CSS kept, the title removed, a mail's own `nm-cid:` removed), `tests/reindex.rs` (only the HTML bodies are read again), `tests/attachments.rs` (the `nm-cid:` source of an embedded image), `InlineImagesTests` (ids, URLs refused, parts shown, the handler in a web view through the policy), `RemoteBlockerTests` (the policy alone stops a request), `MailFrameTests` (own dark mode, the frame, and in a web view: the CSS of a mail reaches neither another message nor the page), `DarkModeTests` (which pages need Dark Reader; in a web view, a white mail takes the theme's colours in the dark theme, its button its hue, nothing is fetched, and the light theme leaves it as is), `darkmode.uiscript` (the same in the app, and a mail with its own dark mode without Dark Reader), and `images.uiscript` (table width, sender's font, embedded logo drawn, policy present, the column of the CSS, the hidden preview text, the mail's own dark mode in a dark and a light theme).
