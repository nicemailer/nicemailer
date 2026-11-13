# Replying and sending

How a reply is written in the app and sent by the core. The rules are those of `specifications/reponse.md` (v1), which narrows `brouillons.md` and applies `dossiers.md` ("Répondre à une conversation", "Écrire à quelqu'un vaut décision Inbox").

## In the app

| Key | Where | Effect |
| --- | --- | --- |
| `r` | a mail | reply to all to the mail under the reader's cursor (`ctrl-j` / `ctrl-k` move it); if a reply to that mail was started, it is resumed |
| `R` | a mail | reply to the sender only (`Reply-To` or the sender, never a Cc); a reply already started to the mail is resumed |
| `F` | a mail | forward it with its files: no recipient (the keys go to To), `Fwd:`, the block "---------- Forwarded message ---------" in the app's language, then the mail's text as it is; nothing is filed |
| `c` | anywhere in the list | a new mail from the account of the view; left empty, Escape drops it |
| ⌘↩ | the reply | send, after the undo delay (5 s); the status bar says "Sending in 5 s · z to cancel" |
| ⇧⌘↩ | the reply | "Send, then file the conversation in": `a` Archive, `w` Waiting, `t` Todo, `s` leave it where it is (trial); the key (or a click) sends, Escape goes back to the text. On a new mail, "Send, then file this mail in": the mail written is what is filed, and `s` leaves it in Sent |
| Escape | the reply | back to the mail; the reply goes to Drafts (here and on the server), and `r` on the mail, or `r` / Return on the draft in Drafts, reopens it |
| ⌘⌫ | the reply | abandon the reply; `z` gives it back with its text |
| ⇥ ⇧⇥ | the reply | next / previous field: To, Cc, Bcc (when shown), Subject, the text; in the text ⇥ is typed, ⇧⇥ goes up to the subject |
| ⌥⌘B | the reply | show the Bcc line and go to it |
| ⌥⌘F | the reply | "Send from": choose the address, and so the account, the mail leaves from (a click on the line "From" too); below |
| ⇧⌘A | the reply | attach files (the open panel); a drop on the reply, or ⌘V of files copied in the Finder, attaches too |
| `z` | after ⌘↩ | during the delay: the mail does not leave, and the reply opens again; once sent: undoes the filing of the conversation (not the sending) |

The reply takes the reader's place (the whole window with one column), with the mode `EDT` in the status bar. From the top: the sending address with the key of the account that sends ("Me <me@x.fr> · perso"), To, Cc and Bcc (Bcc only when it has addresses or ⌥⌘B opened it), one tag per address, wrapping when long, the subject, "After sending: Archive" when the conversation will be filed, then the text. The cursor starts at the top, above the line introducing the quotation ("On Nov 7, 2026, at 10:12, Alice <alice@x.fr> wrote:", in the app's language) and the quoted mail.

While the reply is open every key is text: letters, Return, Delete (⌥ a word), ⌦, the arrows (⌥ by word, ⌘ to the ends of the line, ⌘↑ ⌘↓ to the ends of the text, ⌥↑ ⌥↓ to the ends of the line), Home, End, `ctrl-a`, `ctrl-e`, ⌘V, and a click in the text puts the cursor there.

As in any macOS text view, **Shift with any of these moves selects** (⇧←, ⇧⌥→ a word, ⇧⌘← to the start of the line, ⇧↓, ⇧⌘↓ to the end of the text…), as does a Shift-click; ⌘A selects everything. The selection is highlighted and the caret hidden. Typing or pasting replaces it, Delete removes it, ⌘C copies it, ⌘X cuts it; ← and → without Shift end it at its start or its end. Selecting by dragging the mouse is not there yet.

**Other apps** that work on the selected text of the front app (Kerlig, a grammar checker, the Services menu) see the reply as any macOS text: its text view is the window's focused element, its selection is the reply's, and what the app writes back (through the accessibility API, a Service, or ⌘V) replaces the selection as if typed, and is saved. The view is not editable by AppKit (the keys stay the controller's): it hands what it receives to `GeekController.replaceComposeSelection`. ↑ and ↓ follow the lines as drawn, so they move inside a long wrapped paragraph. ## Recipients and subject

The keys go to one field at a time; the label of that field is in the accent colour, and a click on a line gives it the keys. The reply opens with the keys in the text.

In a recipient line, the text typed has the editing keys of a text field. Return, `,` or `;` make an address of it, and so do Tab, a click elsewhere, ⌘↩ and Escape. The core reads it (`parse_addresses`): `alice@x.fr`, `Alice <alice@x.fr>`, `"Dupont, J." <j@x.fr>`, `Alice Martin alice@x.fr`, `mailto:alice@x.fr`, several separated by commas, semicolons or line breaks; ⌘V reads a pasted list at once. What is not an address stays in its line, which takes the keys back, and the status bar says "Not an address: …"; ⌘↩ then sends nothing. An address already in To, Cc or Bcc is not added twice.

**Removing or correcting an address.** ⌫ in an empty line takes its last address back into the line, **selected**, as a macOS field reached with Tab: a second ⌫ wipes it, what is typed replaces it, and ← or → turns it into an ordinary text whose letters can be corrected. It used to come back with the caret at its end, so removing it meant deleting every character. A **click on an address** takes that one back the same way, wherever it sits in the three lines, so the ones after it no longer have to be taken back to reach it; what was being typed in the line becomes an address first, as a click elsewhere does, and nothing typed is lost.

From the first letter typed, up to 8 suggestions show under the line (`suggest_recipients`): addresses whose address, domain, name or a word of the name starts with the text; never a banned sender, an address of an account, or one already a recipient. ↓ ↑ choose, Return or a click takes one, Escape closes the list (a second Escape leaves the reply). The Mac's Contacts are not searched yet.

**In which order** (changed in November 2026, which ranked on the number of mails alone): by how recently the user wrote to the address, then by how often. The addresses written to in the last week come first, then those of the last month, the last quarter, the last year, those written to longer ago, and last those never written to; inside each of those groups, the address written to most often, then the one most heard from. Whom one writes to changes over the years: the address of this morning's correspondent is wanted before that of a heavy exchange long over. Groups rather than a score that fades by the day keep the order predictable — a list answered with ↓ and Return has to be — and hold it until a mail crosses the edge of a group.

The query scans `address` for at most 2,000 candidates, then counts and dates their mails through `idx_message_address_address` and `idx_message_from`. On a generated mailbox of 285,000 mails, one letter takes 61 ms (55 ms when only counting). The mails received are counted but not dated, on purpose: counting reads the index alone, while reading the date of every mail received from every candidate doubled that time, and what is wanted is when the user **wrote**.

The subject is a line of text; Return goes to the text. What changed is saved at the pause of typing, with the text: `update_draft` gets only the parts that changed (the body, a whole recipient list, the subject), in one revision; the core checks each address again (`compose::clean_recipients`: valid, once across the three lists, 500 at most) and refuses the save otherwise.

## Attachments

`⇧⌘A` opens the panel to choose files; files dropped on the reply, or copied in the Finder and pasted with ⌘V in the text, are attached too. A line "Files" lists them under the subject ("devis.pdf (240 KB)"); it is a field like the others (⇧Tab from the text reaches it, a click on a file too): ← → select a file, ⌫ removes it, Return goes back to the text. A folder is refused.

**Who reads the file.** The core runs as a launch agent and reads nothing outside its data directory: macOS protects Desktop, Documents and Downloads, and only the app was given the file. So the app copies it into `<data_dir>/incoming.noindex/` under a UUID (an APFS clone: instant, no space used until one of the two changes), off the main thread, then calls `add_draft_attachment` with that name, the file's name and its MIME type (from its extension). The core (`nm_store::outgoing`) accepts only a plain name (letters, digits, `-`), never a path nor a symbolic link, hashes the file while reading it once, and moves it into `outgoing.noindex/ab/<sha256>` (read-only, kept once whatever the number of drafts). From then on the draft no longer depends on the user's file. A refused file is deleted from `incoming.noindex` by the app.

**Limits.** Past 25 MB once encoded (base64 makes a file a third bigger), the status bar warns that many servers refuse such a mail; past 100 MB of files on a draft, the core refuses (the mail is built in memory).

**The mail.** With files, `build_message` writes a `multipart/mixed` message: the text, then each file in base64 (lines of 76 characters), its name in quotes when it is plain ASCII, else in RFC 2231 (`filename*0*=utf-8''caf%C3%A9…`, in pieces so that no line is long) and as an RFC 2047 word in `name` for older readers. The boundary comes from the `Message-ID`: the same draft gives the same bytes. The `send` action keeps such a message in `outgoing.noindex` and names it in its payload (`mime_blob`, `copy_blob`), so that reading the payloads of `pending_action` stays light.

**Drafts.** Adding or removing a file is a revision: the copy in Drafts carries the files, and is written again only when the draft changed since the last one. The previous copy leaves the raw store with its row of Drafts.

**Clean-up.** `raw_gc` also runs `gc::collect_outgoing`, under the database's lock: rows of `outgoing_blob` that no draft attaches and no queued or running `send` action is made of go, with their files; files without a row older than `raw.gc_grace_secs`; files left in `incoming.noindex` for a day.

**Bcc.** The hidden recipients are in the SMTP envelope, never in the mail given to the server. The copy the user keeps has a `Bcc` header, to know who got it: the raw file, the row and the copy in Sent, the copy in Drafts. The core builds the MIME twice (same `Message-ID`), and the `send` action carries the second one (`mime_copy`) only when there are hidden recipients.

The text is saved by the core after each half-second without typing: a crash loses at most that much.

Then the status bar says what happened: "Sent · filed in Archive · z: undo the filing", "Sent, but the server refused: bad@x.fr (550 …)", or "Not sent: … · r on the mail to reopen the reply" (the draft is editable again, with its text).

## Choosing the filing

`⌘↩` files the conversation as the line "After sending: …" says: the core's preselection (`dossiers.md`, "Répondre à une conversation"). `⇧⌘↩` asks first, in a small menu shown **in the headers of the mail**, in place of that very line (as "Send from" takes the place of the line "From"), with the preselection marked:

- `a` Archive, `w` Waiting, `t` Todo;
- `s` leave it where it is, a **trial** option shown only in this menu: the mail is sent and its recipients get their decision, but the conversation is not touched at all: nothing moves, and its return mark ("↩ Todo") stays.

The choice is sent as `after_send` and saved in the draft (`draft.after_send`): `z` during the undo delay gives the reply back with it ("After sending: Waiting", or "stays where it is"), and the next `⌘↩` applies it. It wins over the preselection, which is worked out again when the server accepts the mail; but nothing moves a conversation that is no longer in the Inbox, Todo or Waiting by then. Where the core offers no filing (a conversation in Archive, Newsletter…, or a forward, which files nothing), `⇧⌘↩` says so and sends nothing.

### A new mail files itself

A new mail (`c`) answers no conversation, so there is nothing to file at the moment it is sent — but the mail written is often one to follow: a request whose answer is awaited belongs in Waiting, one to come back to in Todo. So `⇧⌘↩` offers the same menu, titled "Send, then file this mail in", with `s` "leave it in Sent" as the preselection: `⌘↩` alone leaves the mail in Sent, as before.

What moves is **the mail written**: its copy leaves Sent for the folder chosen, here and on the server, exactly as a move given from Sent on a conversation with nothing else does (`folder-workflow.md`). It happens **once the copy in Sent is made** (the end of the `append` phase, in the same transaction as the copy's UID): before that the copy does not exist on the server and its UID is unknown, so the server could not follow the move. The status bar says "Sent · filed in Waiting" as soon as the mail is accepted — the event `DraftSent` carries the folder in `filing` — and the list shows the move a moment later.

`z` after sending undoes the decisions taken on the recipients, **not** that filing: it happens after the undo token of the sending. The status bar does not offer it, and the mail is moved from its list like any other. A send whose copy in Sent never succeeds (a server unreachable to the last attempt, an account without a Sent folder) files nothing.

## Choosing the sender

Any mail can be sent from any account that can send: an account of `accounts.toml` with `[smtp]`, not read-only (with `NICEMAILER_FAKE_NETWORK`, the accounts the file does not declare too). `⌥⌘F`, or a click on the line "From", opens "Send from": every address of those accounts, each account's main address then its identities, in the order of the accounts, keyed `1` to `9` then by letter, the current one marked. A key (or a click) chooses; Escape goes back to the text. The panel takes **the place of the line "From"**, at the top of the mail, rather than the bottom-right corner the other questions use: as for "Send, then file…", the answer is read where the line it changes is. The status bar says "Sending from Sales <sales@work.fr> · work". With a single address there is nothing to choose, and the status bar says which it is.

The choice is saved with the draft at the next pause, like the other headers (`update_draft` with `from: {account_id, addr}`): the core checks that the account can send and that the address is one of its own, and takes the name `accounts.toml` gives it. Escape then `r` gives the reply back from the address chosen.

**A mail of an account that cannot send.** `r`, `R` and `F` on a mail of an archive account (an mbox import), of an IMAP account without `[smtp]`, or of a read-only one, no longer refuse: the core starts the reply from another account (`compose::choose_sender`), the first, in the order of the accounts, with an address among the To and Cc of the mail (that address), else the first account that can send, from its main address. The status bar says "This account cannot send: from me@work.fr · work · ⌥⌘F to change". Only when no account at all can send is the reply refused, with the reason. `c` in the view of such an account writes from the first account that can send.

**Where everything goes.** The draft stays in the account of the mail answered: its conversation, its row of Drafts and, for an IMAP account, its copy in the Drafts of that server. The account that sends is recorded in `draft.send_account_id` (schema version 7; NULL when it is the draft's own) and shown as `send_account_id` in `DraftView`. At sending, the `send` action is queued on the account that sends: its sender takes it, its SMTP server sends it, the mail becomes a row of its Sent folder and is copied to its Sent folder on the server. The conversation answered is filed in its own account, by the usual rules (an archive account's mail is in Archive: it stays there). The recipients are cleaned of the addresses of both accounts. The `Message-ID` of the draft, made from the address it started from, stays the same when the sender changes: its copies in Drafts are found by it.

## Forward and new mail

A forward (`create_draft` with `kind: forward`) is linked to the mail by `draft.forward_of_message_id` (schema version 6): `F` again on the mail reopens it. The core takes the mail's files (not its inline images) from its raw file into `outgoing.noindex`, as if the user had attached them; those it cannot take (a mail whose raw file is not here) are named in `warning`, and the text is forwarded all the same. No `In-Reply-To`: a forward is not a reply and files nothing.

A new mail (`kind: new`) is sent from the main address of the account the app names (the account of the view, or of the selected mail in the view of all accounts) if it can send, else from the first account that can (above). `close_draft` deletes a new mail left blank (no recipient, subject, text nor file) instead of copying it to Drafts.

Escape with a text that is not an address in a recipient line keeps the draft open, to be corrected. Quarantine refuses `R` and `F`, as it refuses `r`.

## What the core prepares

`create_draft` builds the reply from the mail answered (`nm_core::compose`, pure functions):

- **From**: the first of the account's addresses (main, then identities) among the mail's To and Cc, else the main address.
- **To / Cc** (reply to all): To is `Reply-To` or the sender, Cc the mail's To and Cc; a reply to one's own mail takes its recipients back. The account's addresses, duplicates and invalid addresses are removed; an empty To takes the first Cc; nobody left: the sender.
- **Subject**: `Re: ` added unless a reply prefix is there (`Re:`, `AW:`, `Sv:`, `Réf. :`, `Re[2]:`…).
- **In-Reply-To / References**: the mail's `Message-ID`, after its references (20 at most, the first kept).
- **Text**: two empty lines, the line of the client, the mail's text quoted with `> ` (`>>` for a line already quoted), 1,000 lines at most.

## Drafts

`r`, `R`, `F` and `c` open the screen on a **proposal** (`prepare_draft`): the core computes the draft on a reading connection and writes nothing, so the screen opens at once even while a synchronization or an import keeps the database writing (each of their batches holds the single write lock; during the initial import of two accounts, `r` used to wait ten seconds). The proposal has `id` 0 and `revision` 0; in a forward, the mail's files are listed with negative ids and copied only when the draft is written. The app writes the draft (`create_draft`, with the text typed as `body`) when there is something to keep: the first pause of typing, a file attached or removed, the sending (`AppModel.ensureWritten`, one writing at a time, so a new mail is never written twice). Left as proposed (Escape, ⌘⌫), it leaves nothing, and the status bar says "Nothing written: no draft kept". If another client wrote the reply to the same mail meanwhile, `create_draft` gives it back, and the text typed here replaces it at the next save.

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
2. once accepted, in one transaction (`Store::send_accepted`): the mail becomes a row of Sent (its raw file is what was sent), its conversation is filed as preselected, its recipients without decision get `inbox` (and their mails leave Quarantine), the draft is deleted, a `send` line goes to the history, and `DraftSent` carries the undo token of the filing (a new mail's own filing is only announced here, in `filing`, and carried by the action as `file_after_copy`);
3. copies the mail to the Sent folder of the server (`APPEND`, `\Seen`) and records its UID (`APPENDUID`), then applies a new mail's filing (`Store::send_copied`): the copy, now on the server with its UID, moves to the folder chosen and the spool tells the server. Gmail (capability `X-GM-EXT-1` or `smtp.gmail.com`) and Fastmail copy sent mail themselves: there the sender only looks for the copy's UID. `save_sent = "always" | "never"` overrides the detection. Without a UID the next pass of Sent attaches the row by its `Message-ID` (a row NiceMailer sent less than an hour ago is never taken for an orphan).

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
| `prepare_draft` | the same as `create_draft` | `{draft, reused, warning}`, writing nothing and never waiting for a write: the draft already started (`reused`), else the proposal (`id` 0, `revision` 0; a forward's files with negative ids) |
| `create_draft` | `kind?` (`reply_all`, `reply_sender`, `forward`, `new`), `reply_to?`, `account?` (a new mail), `quote_header`, `body?` (the text typed in a proposal, or an abandoned text restored) | `{draft, reused, warning}` |
| `open_draft`, `close_draft` | `id` | `DraftView`, nothing |
| `update_draft` | `id`, `revision`, and what changed: `body?`, `subject?`, `to?`, `cc?`, `bcc?` (whole lists of `{addr, name}`), `from?` (`{account_id, addr}`, one of `list_senders`) | `{revision}`; `REVISION_CONFLICT` (1012), `DRAFT_SENDING` (1013), `INVALID_PARAMS` for an address that is not one (or not the account's), `CANNOT_SEND` (1015) for an account that cannot send |
| `add_draft_attachment` | `id`, `revision`, `staged`, `filename`, `mime_type?` | `{revision, attachment, warning}`; `INVALID_PARAMS` (not a staged file, beyond 100 MB) |
| `remove_draft_attachment` | `id`, `revision`, `attachment_id` | `{revision}` |
| `parse_addresses` | `text` | `{addresses, invalid}` |
| `suggest_recipients` | `query`, `limit?` (8, at most 50) | `{items}` of `{addr, name}` |
| `list_senders` | none | `{items}` of `{account_id, account, addr, name}`: every address of every account that can send, in the order of the accounts |
| `discard_draft` | `id` | the `DraftView` abandoned |
| `send_draft` | `id`, `revision`, `body`, `after_send?` (`archive`, `waiting`, `todo`, `stay`; saved in the draft; absent: the one saved, else the preselection) | `{send_at}`; `CANNOT_SEND` (1015), also when the account that sends no longer can (`accounts.toml` changed) |
| `cancel_send` | `id` | `DraftView`, editable; `SEND_ALREADY_STARTED` (1014) |

`prepare_draft` and `create_draft` return `BODY_PENDING` (1016) when the mail's body is not downloaded yet; the core asks for it first. Events: `DraftSent {draft_id, message_id, thread_id, moved_to, filing, undo}` (`filing`: a new mail's own filing, applied once the copy in Sent is made, and not covered by `undo`), `SendFailed {draft_id, reply_to, reason}`, `SendPartial {draft_id, refused}`.

## Settings

| Key | Default | |
| --- | --- | --- |
| `compose.undo_send_seconds` | 5 | 0 to 30; 0 sends at once |
| `accounts.<key>.save_sent` | `auto` | `always`, `never` |
| `accounts.<key>.smtp` | none | without it the account cannot send; its mails are answered from another account |

## Tests

- `nm-core` (`compose.rs`): the account that sends for one that cannot (the one the mail was sent to, else the first), reply to the sender, forward subject and body, attachments in `multipart/mixed` (names in RFC 2231, short lines, the same bytes twice, a text holding the boundary), file names and types cleaned, the address lists typed or pasted, cleaning the edited recipients and subject, `Bcc` only in the copy, recipients, identity, subject, references, quotation, `format=flowed` read back as typed, MIME headers and encodings, header injection, dates, providers.
- `nm-proto` (`smtp.rs`): a scripted server: PLAIN and LOGIN, refused login, no password without TLS, recipients refused in part or all, `SIZE`, `5xx` / `4xx` to the message, hostile addresses, broken servers, dot-stuffing.
- `nm-store` (`tests/drafts.rs`): a reply, a forward (its files listed, not copied) and a new mail proposed on a connection made `query_only` (proof that nothing is written), refused as the draft would be, then written exactly as proposed; the first save writing the text typed; the draft prepared, reused, revisions, the filing chosen (kept after `cancel_send`, winning over the preselection, `stay` touching nothing), recipients and subject edited (one revision, refused addresses), Bcc in the envelope and the copy only (and its decision), suggestions by use, files attached and removed (refusals, sizes), a mail with files sent and its files collected, orphans swept after the grace delay, `R`, a forward with its files (and without raw file), a new mail dropped when blank, a mail of an account that cannot send answered from another (the account the mail was sent to preferred, queued and claimed on the account that sends, kept in its Sent, the conversation filed in its own account), the sender changed (one revision, back to the draft's own account); `outgoing.rs`: a staged file taken once by its content, only a plain name taken, the undo delay and `cancel_send`, `data_sent`, the acceptance in one transaction (Sent row, filing, decisions, undo), replay, the copy's UID and the attachment by the synchronization, a fresh Sent row kept by a pass, failures, partial refusals, restore, repair.
- `nm-daemon` (`tests/send_rpc.rs`, `make imap-test`): the real core against Dovecot and Mailpit: a reply sent, copied to Sent, filed on the server, nothing twice after a pass, undo, on both servers; a mail of an archive account answered from the IMAP account (its From, its SMTP server, its Sent on the server); a send cancelled in time; a refused recipient; a refused login then the SMTP password saved; an agent refused.
- `nm-daemon` (`tests/fake_send_rpc.rs`): `prepare_draft` answered while a write holds the lock (`NICEMAILER_TEST_WRITE_DELAY_MS`), nothing written, then `create_draft` with the text typed and `prepare_draft` reusing it, refused to an agent; an archive account refused, then pretending to send with `NICEMAILER_FAKE_NETWORK`; `after_send` through the socket (`stay` saved, `inbox` refused); `parse_addresses`, `suggest_recipients`, headers saved by `update_draft`; a staged file attached and sent; the kinds of draft (`R`, `F`, `c`, a blank new mail dropped at closing); an archive mail answered from an account of `accounts.toml` (`list_senders`, the sender changed, an archive account and a foreign address refused, the `send` queued on the account that sends, a new mail asked from the archive account).
- App: `TextBufferTests`, `ComposeTests` (pure rules, among them `createParams` and `Composer.written`, and the keys through `GeekController` against `FakeCore`: `r` writes nothing, the first pause writes, Escape on a proposal leaves nothing, ⌘⌫ before the pause discards nothing, a proposal sent is written first, written once when two need it, a refusal leaves the text open), `UITests/proposal.uiscript`, `UITests/compose.uiscript`, `UITests/recipients.uiscript`, `UITests/attachments.uiscript`, `UITests/writing.uiscript` and `UITests/sender.uiscript`.

## Pretending to send

With `NICEMAILER_FAKE_NETWORK=1` in the core's environment, an account that `accounts.toml` does not declare (the fake mail of `seed`, or an archive account imported from mbox) can reply, from its own address, and its mails "leave" without any network: once the undo delay is over, the core records them as accepted (the mail in Sent, the filing, the decisions on the recipients), exactly as after a real SMTP server. The UI tests use it, and it lets you try replying on a real mailbox imported from mbox:

```sh
NICEMAILER_FAKE_NETWORK=1 NICEMAILER_DATA_DIR="$HOME/Library/Application Support/NiceMailer-archive" make core
```

The accounts declared in `accounts.toml` are not affected: one with `[smtp]` sends for real, and is offered in "Send from" with the others. The changes are real in the database: the reply in Sent, and the decision `inbox` given to recipients without one (`z` undoes the filing and the decisions).
