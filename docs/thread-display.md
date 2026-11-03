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

## Limits

- Messages are not deleted, unlike Outlook's cleanup: redundant content is only hidden.
- An attribution written in another language, or a client with an unusual header block, leaves the history visible (as text or quote).
- The HTML view hides quotes only for the markers above, and has no per-part "···" lines: `T` shows everything.

## Tests

`crates/nm-store/src/clean.rs` (unit tests of every rule), `crates/nm-store/tests/thread_display.rs` (a conversation imported then read with `get_thread`), `apps/NiceMailer/Tests/NiceMailerTests/ThreadDisplayTests.swift` (blocks of the reader, toggles, `S`), and the UI scenario `apps/NiceMailer/UITests/thread.uiscript`.
