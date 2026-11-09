# Replying and sending

How a reply is written in the app and sent by the core. The rules are those of `specifications/reponse.md` (v1), which narrows `brouillons.md` and applies `dossiers.md` ("Répondre à une conversation", "Écrire à quelqu'un vaut décision Inbox").

## In the app

| Key | Where | Effect |
| --- | --- | --- |
| `r` | a mail | reply to all to the mail under the reader's cursor (`ctrl-j` / `ctrl-k` move it); if a reply to that mail was started, it is resumed |
| `R`, `f` | a mail | reply to the sender only, forward: not available yet |
| ⌘↩ | the reply | send, after the undo delay (5 s); the status bar says "Sending in 5 s · z to cancel" |
| ⇧⌘↩ | the reply | "Send, then file the conversation in": `a` Archive, `w` Waiting, `t` Todo, `s` leave it where it is (trial); the key (or a click) sends, Escape goes back to the text |
| Escape | the reply | back to the mail; the reply goes to Drafts (here and on the server), and `r` on the mail, or `r` / Return on the draft in Drafts, reopens it |
| ⌘⌫ | the reply | abandon the reply; `z` gives it back with its text |
| `z` | after ⌘↩ | during the delay: the mail does not leave, and the reply opens again; once sent: undoes the filing of the conversation (not the sending) |

The reply takes the reader's place (the whole window with one column), with the mode `EDT` in the status bar. From the top: the sending address, To and Cc (one line each, cut with "+3" when long), the subject, "After sending: Archive" when the conversation will be filed, then the text. The cursor starts at the top, above the line introducing the quotation ("On Nov 7, 2026, at 10:12, Alice <alice@x.fr> wrote:", in the app's language) and the quoted mail.

While the reply is open every key is text: letters, Return, Delete (⌥ a word), ⌦, the arrows (⌥ by word, ⌘ to the ends of the line, ⌘↑ ⌘↓ to the ends of the text, ⌥↑ ⌥↓ to the ends of the line), Home, End, `ctrl-a`, `ctrl-e`, ⌘V, and a click in the text puts the cursor there.

As in any macOS text view, **Shift with any of these moves selects** (⇧←, ⇧⌥→ a word, ⇧⌘← to the start of the line, ⇧↓, ⇧⌘↓ to the end of the text…), as does a Shift-click; ⌘A selects everything. The selection is highlighted and the caret hidden. Typing or pasting replaces it, Delete removes it, ⌘C copies it, ⌘X cuts it; ← and → without Shift end it at its start or its end. Selecting by dragging the mouse is not there yet.

**Other apps** that work on the selected text of the front app (Kerlig, a grammar checker, the Services menu) see the reply as any macOS text: its text view is the window's focused element, its selection is the reply's, and what the app writes back (through the accessibility API, a Service, or ⌘V) replaces the selection as if typed, and is saved. The view is not editable by AppKit (the keys stay the controller's): it hands what it receives to `GeekController.replaceComposeSelection`. ↑ and ↓ follow the lines as drawn, so they move inside a long wrapped paragraph. The recipients and the subject are not editable yet.

The text is saved by the core after each half-second without typing: a crash loses at most that much.

Then the status bar says what happened: "Sent · filed in Archive · z: undo the filing", "Sent, but the server refused: bad@x.fr (550 …)", or "Not sent: … · r on the mail to reopen the reply" (the draft is editable again, with its text).

## Choosing the filing

`⌘↩` files the conversation as the line "After sending: …" says: the core's preselection (`dossiers.md`, "Répondre à une conversation"). `⇧⌘↩` asks first, in a small menu with the preselection marked:

- `a` Archive, `w` Waiting, `t` Todo;
- `s` leave it where it is, a **trial** option shown only in this menu: the mail is sent and its recipients get their decision, but the conversation is not touched at all: nothing moves, and its return mark ("↩ Todo") stays.

The choice is sent as `after_send` and saved in the draft (`draft.after_send`): `z` during the undo delay gives the reply back with it ("After sending: Waiting", or "stays where it is"), and the next `⌘↩` applies it. It wins over the preselection, which is worked out again when the server accepts the mail; but nothing moves a conversation that is no longer in the Inbox, Todo or Waiting by then. Where the core offers no filing (a conversation in Archive, Newsletter…), `⇧⌘↩` says so and sends nothing.

## What the core prepares

`create_draft` builds the reply from the mail answered (`nm_core::compose`, pure functions):

- **From**: the first of the account's addresses (main, then identities) among the mail's To and Cc, else the main address.
- **To / Cc** (reply to all): To is `Reply-To` or the sender, Cc the mail's To and Cc; a reply to one's own mail takes its recipients back. The account's addresses, duplicates and invalid addresses are removed; an empty To takes the first Cc; nobody left: the sender.
- **Subject**: `Re: ` added unless a reply prefix is there (`Re:`, `AW:`, `Sv:`, `Réf. :`, `Re[2]:`…).
- **In-Reply-To / References**: the mail's `Message-ID`, after its references (20 at most, the first kept).
- **Text**: two empty lines, the line of the client, the mail's text quoted with `> ` (`>>` for a line already quoted), 1,000 lines at most.

## Drafts

While the reply is open, the draft lives in the `draft` table only: each pause of typing writes there. When it is closed (Escape, or the core starting again after a stop or a crash with a draft changed since its last copy), `close_draft` (`nm-store`, `draft_copies.rs`):

1. writes its MIME message, with `X-NiceMailer-Draft-Revision`, in the raw store;
2. replaces its row of the Drafts folder by a new one, complete, read, flagged `\Draft`, in the conversation of the mail it answers: the app lists it in Drafts at once;
3. on an account with a server, queues an action `append_draft`. The work connection (`nm-engine`, `sync/drafts.rs`) writes the version (`APPEND`, `\Draft \Seen`), records its UID, then removes every other copy of the same `Message-ID` from Drafts: the previous version, or a copy left by a crash. Replayed, the action leaves exactly one copy. Closed twice before the copy is made, the draft is copied once.

Sent or abandoned, the draft leaves Drafts: its row goes, and an action `delete` removes its copies from the server.

Changes made elsewhere, the simple cases: a copy deleted (or sent) on the phone takes the draft, if it was not changed here since its copy; changed here, its text is kept, and its next closing looks for the mail in Sent first (sent elsewhere: the draft goes), else writes it back as a new copy. The adoption of a version changed on the phone, and the conflicts kept as two drafts, come later; so does resuming a draft written on the phone (it is shown, read-only).

An account without server (archive, `seed`) keeps its drafts in Drafts locally only.

## Sending

```text
send_draft ─▶ action `send` (phase smtp), due after the undo delay ─▶ SMTP ─▶ accepted ─▶ phase append ─▶ copy in Sent
     │                    ▲
     └── cancel_send ─────┘ (while queued)
```

`send_draft` freezes the draft (`sending`), builds the MIME message (`text/plain; format=flowed`, lines cut at 72 characters for sending only, `quoted-printable` when not ASCII, headers encoded in RFC 2047) and queues an action `send` in `pending_action`, due after `compose.undo_send_seconds`.

Each account with `[smtp]` has a **sender**, a thread of its worker with its own connections (`nm-engine`, `sync/send.rs`). It sleeps until the next mail is due, or until a mail is queued or cancelled, then:

1. connects to the SMTP server (TLS or STARTTLS, certificates checked by macOS, `ca_file` honoured), logs in (`AUTH PLAIN`, else `LOGIN`, only over TLS), `MAIL FROM`, `RCPT TO` for every recipient, `DATA`;
2. once accepted, in one transaction (`Store::send_accepted`): the mail becomes a row of Sent (its raw file is what was sent), its conversation is filed as preselected, its recipients without decision get `inbox` (and their mails leave Quarantine), the draft is deleted, a `send` line goes to the history, and `DraftSent` carries the undo token of the filing;
3. copies the mail to the Sent folder of the server (`APPEND`, `\Seen`) and records its UID (`APPENDUID`). Gmail (capability `X-GM-EXT-1` or `smtp.gmail.com`) and Fastmail copy sent mail themselves: there the sender only looks for the copy's UID. `save_sent = "always" | "never"` overrides the detection. Without a UID the next pass of Sent attaches the row by its `Message-ID` (a row NiceMailer sent less than an hour ago is never taken for an orphan).

| What happened | What follows |
| --- | --- |
| Network, time limit, `4xx` | tried again later: 30 s, doubled, 8 attempts |
| Login refused, `5xx`, larger than `SIZE`, no recipient accepted, no password | the draft is editable again, `SendFailed` |
| Some recipients refused | sent to the others, `SendPartial` |
| The copy in Sent fails | tried again; the mail is sent whatever happens |

**Never twice.** Just before the final `.` of `DATA`, the action records `data_sent`: from then on the server may have the mail even without an answer. An attempt after that point (connection lost, core stopped) first looks for the `Message-ID` in the Sent folder of the server. `cancel_send` is refused then (`SEND_ALREADY_STARTED`). A crash between the server's acceptance and the record can still send twice where the provider does not copy to Sent (`brouillons.md`, "Double envoi").

At start, the core queues the interrupted actions again and makes editable any draft left `sending` without an action.

## Password

The sender uses the SMTP password of the account if one was saved (`nicemailer account password <key> --smtp`, item `smtp:<key>` of the Keychain, bound to the SMTP server), else the IMAP password, **only** when the SMTP server is the IMAP server or of the same organizational domain (`smtp.gmail.com` and `imap.gmail.com`). Otherwise the sending is refused with a message saying to save the SMTP password: an `accounts.toml` changed to point `[smtp]` elsewhere must not receive the IMAP password.

## Who may send

The app, the TUI and the CLI. An agent (MCP) or the web server get `FORBIDDEN`: a mail an agent asks for will need a confirmation in the app, which does not exist yet.

## Contract

| Method | Parameters | Result |
| --- | --- | --- |
| `create_draft` | `reply_to`, `quote_header`, `body?` (restores an abandoned text) | `{draft, reused}` |
| `open_draft`, `close_draft` | `id` | `DraftView`, nothing |
| `update_draft` | `id`, `revision`, `body` | `{revision}`; `REVISION_CONFLICT` (1012), `DRAFT_SENDING` (1013) |
| `discard_draft` | `id` | the `DraftView` abandoned |
| `send_draft` | `id`, `revision`, `body`, `after_send?` (`archive`, `waiting`, `todo`, `stay`; saved in the draft; absent: the one saved, else the preselection) | `{send_at}`; `CANNOT_SEND` (1015) |
| `cancel_send` | `id` | `DraftView`, editable; `SEND_ALREADY_STARTED` (1014) |

`create_draft` returns `BODY_PENDING` (1016) when the mail's body is not downloaded yet; the core asks for it first. Events: `DraftSent {draft_id, message_id, thread_id, moved_to, undo}`, `SendFailed {draft_id, reply_to, reason}`, `SendPartial {draft_id, refused}`.

## Settings

| Key | Default | |
| --- | --- | --- |
| `compose.undo_send_seconds` | 5 | 0 to 30; 0 sends at once |
| `accounts.<key>.save_sent` | `auto` | `always`, `never` |
| `accounts.<key>.smtp` | none | without it the account cannot send |

## Tests

- `nm-core` (`compose.rs`): recipients, identity, subject, references, quotation, `format=flowed` read back as typed, MIME headers and encodings, header injection, dates, providers.
- `nm-proto` (`smtp.rs`): a scripted server: PLAIN and LOGIN, refused login, no password without TLS, recipients refused in part or all, `SIZE`, `5xx` / `4xx` to the message, hostile addresses, broken servers, dot-stuffing.
- `nm-store` (`tests/drafts.rs`): the draft prepared, reused, revisions, the filing chosen (kept after `cancel_send`, winning over the preselection, `stay` touching nothing), the undo delay and `cancel_send`, `data_sent`, the acceptance in one transaction (Sent row, filing, decisions, undo), replay, the copy's UID and the attachment by the synchronization, a fresh Sent row kept by a pass, failures, partial refusals, restore, repair.
- `nm-daemon` (`tests/send_rpc.rs`, `make imap-test`): the real core against Dovecot and Mailpit: a reply sent, copied to Sent, filed on the server, nothing twice after a pass, undo, on both servers; a send cancelled in time; a refused recipient; a refused login then the SMTP password saved; an agent refused.
- `nm-daemon` (`tests/fake_send_rpc.rs`): an archive account refused, then pretending to send with `NICEMAILER_FAKE_NETWORK`; `after_send` through the socket (`stay` saved, `inbox` refused).
- App: `TextBufferTests`, `ComposeTests` (pure rules, and the keys through `GeekController` against `FakeCore`), and `UITests/compose.uiscript`.

## Pretending to send

With `NICEMAILER_FAKE_NETWORK=1` in the core's environment, an account that `accounts.toml` does not declare (the fake mail of `seed`, or an archive account imported from mbox) can reply, from its own address, and its mails "leave" without any network: once the undo delay is over, the core records them as accepted (the mail in Sent, the filing, the decisions on the recipients), exactly as after a real SMTP server. The UI tests use it, and it lets you try replying on a real mailbox imported from mbox:

```sh
NICEMAILER_FAKE_NETWORK=1 NICEMAILER_DATA_DIR="$HOME/Library/Application Support/NiceMailer-archive" make core
```

The accounts declared in `accounts.toml` are not affected: one with `[smtp]` sends for real. The changes are real in the database: the reply in Sent, and the decision `inbox` given to recipients without one (`z` undoes the filing and the decisions).
