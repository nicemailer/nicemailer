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

The HTML view keeps the **layout** the sender chose, and shows it in the **theme's colours**:

- Kept by the core's sanitizer (`ingest.rs`, `LAYOUT_ATTRIBUTES`, parser version 9): table widths and heights, `border`, `cellpadding`, `cellspacing`, the alignment of cells (`valign`, `nowrap`), of paragraphs and headings (`align`), the spacing of images (`hspace`, `vspace`), `<font face size>`, `<tfoot>` and the text direction (`dir`). None of these takes a URL.
- Left out: the colours (`bgcolor`, `color`), which the reader does not apply so that every mail stays readable in the dark theme; `background` (an image URL); `style` attributes and `<style>` blocks (not yet: they need a CSS sanitizer).
- Fonts: the theme font (iA Writer Mono) is the page's default; a font the sender names in `<font face>` is used for its text when the Mac has it. Web fonts are never loaded.
- A table wider than the reader shrinks to it (`max-width: 100%`).
- **Embedded images** (`cid:`, a logo or a signature carried by the mail) are shown, in the html and images displays alike: nothing is requested from the network. `get_thread` gives each one a source in the app's scheme, `nm-cid://image/<attachment id>` (`attachments.rs`, `resolve_inline_images`; the stored HTML keeps `cid:`), and the app's handler serves the part the core extracts (`get_attachment`; `InlineImages.swift`): images only, SVG excepted, 10 MB at most, and only the ids of the page. A mail that writes an `nm-cid:` source itself loses it in the sanitizer, so it cannot show another mail's attachment.
- The page carries a **Content Security Policy** (`ContentPolicy`): nothing loads but the app's schemes (`nm-remote:`, `nm-cid:`) and `data:`; no script, frame, form or `<base>`. It is a third barrier after the sanitizer and WebKit's block list (`RemoteBlocker`), and stops a remote load on its own (`RemoteBlockerTests`).

Bodies stored by an older parser are rewritten by the background reindex: the HTML ones read their raw message again, the others are only marked.

## Limits

- Messages are not deleted, unlike Outlook's cleanup: redundant content is only hidden.
- An attribution written in another language, or a client with an unusual header block, leaves the history visible (as text or quote).
- The HTML view hides quotes only for the markers above, and has no per-part "···" lines: `T` shows everything.

## Tests

`crates/nm-store/src/clean.rs` (unit tests of every rule), `crates/nm-store/tests/thread_display.rs` (a conversation imported then read with `get_thread`), `apps/NiceMailer/Tests/NiceMailerTests/ThreadDisplayTests.swift` (blocks of the reader, toggles, `S`), and the UI scenario `apps/NiceMailer/UITests/thread.uiscript`.

HTML of the sender: `ingest.rs` (attributes kept, colours and URLs not, a mail's own `nm-cid:` removed), `tests/reindex.rs` (version 9 reads only the HTML bodies again), `tests/attachments.rs` (the `nm-cid:` source of an embedded image), `InlineImagesTests` (ids, URLs refused, parts shown, the handler in a web view through the policy), `RemoteBlockerTests` (the policy alone stops a request), and `images.uiscript` (table width, sender's font, embedded logo drawn, policy present).
