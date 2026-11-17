# Core features behind the screens

JSON-RPC methods added for the geek theme (`docs/geek-ui.md`). They can be called by any client, for instance with `nicemailer call METHOD 'PARAMS'`. All lists are newest first unless stated otherwise; paginated lists return `next_cursor`, to pass back as `cursor`.

## Sorting a folder by modification

`list_folders` gives every folder shown, with its counters of mails and unread mails (columns of `folder`, kept up to date by every write: read, not counted). It gives no count of conversations: no interface shows one, and counting them read the list index of every conversation of the mailbox, 0.6 to 1.4 s just after the core started on a real mailbox, at every refresh of the sidebar (`Store::folder_thread_count` keeps that count for the tests of the lists).

`list_threads` takes `folder_id`, or `folder_ids` to list several folders together (the unified view "all" of the app lists the folders of one role in every account), or `everything` for the virtual folder Everything: `{}` for every account, `{"account_id": 3}` for one (exactly one of the three); with `"all_mail": true` in it, the virtual folder All Mail, every conversation but those in Deleted, Junk and Banned; with `"work": true`, the virtual folder Work, the conversations of Inbox, Todo and Waiting together (`folder IN ('i','t','w')`, its own partial indexes, named in the query with `INDEXED BY`: without statistics SQLite preferred to read three ranges of the role index and sort them). **Every list reads the table `thread` alone** (schema version 9): a conversation has one folder, the column `thread.folder` (a one-letter code of its role, `FolderRole::code`), and a flag per technical folder (`has_sent`, `has_draft`, `has_quarantine`, `has_deleted`); the rule is in `specifications/dossiers.md`, "Dossier d'une conversation", and in `nm_core::thread_folder`. A real folder lists `folder = '<code>'`, Sent, Drafts, Quarantine and Deleted list their flag, All Mail `folder NOT IN ('d','x','b')`, Everything every thread. Each is one walk of its own index (every account, or one account), newest first, stopping after a page: the unified view of a role is **one** query whatever the number of accounts. Only folders of different roles listed together (Inbox and Sent, from the CLI) are one query per role, merged on the sort key, each thread once. A row shows, in every list, the counters and the date of the whole conversation, without the mails of Deleted, Junk and Banned (all its mails when it has only such mails); the list of Sent is therefore sorted by the latest mail of the conversation, not by the latest sending. Its `folder_id` is the listed folder of the thread's account, or in Everything and All Mail the thread's own folder. A conversation belongs to one account: the same mail received on two accounts is two conversations. Measured on a real mailbox of 400,000 mails and 296,000 conversations (`make bench`): 6 to 8 ms a page without criterion, whatever the list, 70 ms with a frequent word. It also takes `"sort": "received"` (default) or `"sort": "modified"`, and `filter`, attributes combined with AND: `seen`, `flagged`, `attachments`, `answered`, `conversation` (true / false), `from` and `to` (lists of parts of names or addresses, OR-ed; a value starting with `@` is a domain, matched exactly with its subdomains: `@acme.fr` matches `x@mail.acme.fr`, not `x@notacme.fr`), `not_from` and `not_to` (same values, a thread is left out as soon as one of its messages matches one), `contains` (full-text words anywhere, addresses included: the `/` search), `text` (words of the subject and body only, FTS5 column filter `{subject body}`), `subject` (words of the subject only), `after` and `before` (Unix times, **Conversation date**: the date of the thread, that of its latest mail as shown, a bound of the walk of the index, free), `mail_after` and `mail_before` (**Mail date**: a mail of the thread in the listed folders received in the interval; a thread from 2016 to 2026 is "before 2020". A set of thread ids computed once through `idx_message_received`, or `idx_message_list (folder_id, date_received)` in a list of folders: its cost follows the mails of the interval, 8 ms for a month, 220 ms for "before 2020" on the mailbox above, hence a criterion of its own). In `contains`, `text` and `subject`, every word is a prefix, quoted so that FTS syntax typed by the user is literal, and a word written `-word` leaves out the threads holding it. A word holding punctuation other than a hyphen or an apostrophe (`paris.fr`, `x@y.org`, `a/b`) is also checked as written, with `LIKE` on the text of the candidates the index found, in the columns of the criterion (`queries.rs`, `is_literal`): FTS5 splits words on punctuation, so `"paris.fr"*` alone is "paris" followed by a word starting with "fr", which every postal address "75016 PARIS / France" matched (14,542 mails of a real mailbox; 417 hold "paris.fr"). That check reads the text of the candidates: 0.4 s a page (2 s cold) on 400,000 mails, for such words only. `search` applies the same rule. An unknown key in `filter` is refused (`INVALID_PARAMS`) rather than ignored: an app newer than the running core gets an error instead of the whole list. `from` and `to` match the address or the name of the contact (`address.name`), not the display name of each mail: a mail from a shared address such as `notifications@github.com` showing a person's name is not from that person. Address, attachment and Mail date criteria look at the thread's messages in every listed folder (every folder in Everything), not only the one the thread is shown in. Every criterion is a set of thread ids computed once through an index (`address` then `idx_message_from` or `idx_message_address_address`; the partial indexes of flagged, answered and attachment messages; one full-text query for the wanted words of a text criterion and one for its excluded words), never a lookup run for each row: a page costs about the same whether the criterion is frequent or rare. `seen` is the exception, because unread mail is common in some mailboxes and absent from others: the list tests `thread.unread_count` while walking its index, which stops early when unread mail is common but walks the whole mailbox when there is none (64 ms on 296,000 conversations with 0 unread). So each query first adds up the sidebar counters of the listed folders (`folder.unread_count`, of every folder of the account in Everything): up to 2,000 unread mails (`UNREAD_SET_MAX`), it reads the set of the threads with an unread mail from the partial index `idx_message_unread`, then those threads by primary key (`NOT INDEXED`, whatever the planner statistics), and sorts them; above, it walks as before. Measured on that mailbox, the set costs 0.1 ms with 100 unread mails and 74 ms with 50,000, the walk the opposite (`queries.rs`, `UnreadPlan`). The count is the listed folders', not the mailbox's: 100 unread mails all in the Inbox leave the archives without any. Both plans give the same rows. With `modified`, threads are ordered by the latest change the user made to one of their messages, in any folder (move, flag, read, tag): "what did I just do". Each thread row also carries `folder_id`, `date_last_modified` and its `tags`. Finally, `thread_ids` (a list of ids) keeps only those conversations, among the list's, with its scope and its other criteria, in its order: a conversation that no longer belongs to the list is not returned. It is how a client reloads the conversations that changed instead of the whole list (`FolderChanged` with `threads_announced`, `specifications/evenements.md`, "Rechargement par conversation"); the threads are read by primary key, then sorted, whatever the list (a plan test checks it).

`FolderChanged` carries `threads_announced: true` when every conversation that changed in that folder has its `ThreadChanged` in the same batch: a client may then reload those conversations alone. The store says so per folder (`ChangeSet::announced`, set by `announce_threads`) only where it checked it: a change of flags asked by a client (`set_thread_seen`, `set_thread_flagged`) and flags changed on the server (`finish_folder`, when no mail vanished). Merging change sets keeps a folder announced only if every part that touched it announced it; anything else (an import, new mail, a move) leaves it out, and the client reloads the list, which is always right.

**Who a row names.** Besides `participants` (the authors of the conversation, oldest to newest, each once), a row carries `correspondent`, the person the conversation is with, and `last_is_mine`, true when the user wrote the latest mail (added in November 2026). `correspondent` is the latest author who is not the user, by name, else by address; when the user wrote every mail, the first recipient of the latest one (To, then Cc, then Bcc, in header order: one lookup by primary key of `message_address`, for such threads only). A mail is the user's by the rule of `get_thread`: its sender is an address of the mailbox, or it is in Sent or Drafts. The addresses come from `accounts.toml`, which the daemon passes to every list as it does to `get_thread`; the store adds the archive accounts' own, read once per page. Drafts do not count, unless the conversation has nothing else: a reply being written has not been sent. The mails counted are those of the counters (without Deleted, Junk and Banned, unless the conversation has only such mails). It costs no extra query for most rows: the mails are those `participants` already reads (`queries.rs`, `correspondent`).

Backed by the column `thread.date_last_modified` and the "modified" variant of the index of each list (`idx_thread_role_modified`, `idx_thread_sent_modified`…, `idx_thread_modified` for Everything), kept up to date with the other thread aggregates.

```sh
nicemailer call list_threads '{"folder_id": 4, "sort": "modified", "limit": 10}'
nicemailer call list_threads '{"everything": {}, "filter": {"contains": "devis"}}'
```

## Text, HTML or images, per sender

`set_body_format {addr, format}` remembers how the mails of an address are displayed: `"text"`, `"html"`, `"images"` (the HTML with its remote images loaded), or `null` for the theme's default. `get_thread` returns, for each message, `body_html` (sanitized), `has_remote_content`, `tracking_pixels` and `from_body_format`, the choice remembered for its sender.

The preference is stored in `address.body_format`; `"images"` is `body_format = 'html'` with `address.remote_images = 1` (a new column rather than a new value: SQLite can only change a `CHECK` constraint by rebuilding the table). It is local to the Mac: not synced, not journaled.

### Remote images in `body_html`

The sanitized HTML never holds a source a web view would load by itself (`crates/nm-store/src/remote_images.rs`):

- a remote **tracking pixel** (an `<img>` with an `http:`, `https:` or `//` source, 2 pixels or less in `width`, `height` or its style, or hidden by its style) loses its source; `tracking_pixels` counts them;
- any other remote image gets the source `nm-remote://image?u=<percent-encoded URL>` (`//host` made `https://host`; every byte but `A-Z a-z 0-9 - . _ ~` encoded). `has_remote_content` is true when there is at least one.

The pixels are found by a pass of html5ever's tokenizer over the original HTML before ammonia runs: ammonia's attribute filter sees one attribute at a time, and `style` is already gone when it runs. A body without `<img` skips that pass. The client decides when to load the images and loads them itself; the core never requests them.

## Saving a message

`export_message {message_id, format, dir?}` writes one message to a file and returns its `path`:

| Format | Content |
| --- | --- |
| `eml` | the raw message, byte for byte, read from the raw storage and checked against its hash |
| `txt` | From, To, Cc, Date, Subject, then the text body |
| `md` | the same in Markdown, with the subject as title |

The file goes to `dir` (absolute), by default `~/Downloads`, named `<date> <subject>.<ext>`. An existing file is never overwritten: ` (2)`, ` (3)`… is added.

`message_record {message_id}` returns everything the app prints in the PDF of `sp` (`docs/geek-ui.md`, "Saving a mail as PDF"), read from the raw message when the core has it (`nm-store/src/record.rs`):

- `from` (every author), `sender`, `reply_to`, `to`, `cc`, `bcc`: each `{addr, name}`, the name as written in this mail (the `address` table keeps only the last name seen for an address), in header order;
- `date_header` (the `Date` header as written, with its time zone), `date_sent`, `date_received`;
- `message_id`, `in_reply_to`, `references`, `account`, `folder` (its path), `auth_result`;
- `body`: the text body with its paragraphs whole, as `message_text`;
- `attachments`: `{part_id, filename, mime_type, size, inline, sha256}`, the hash and size of the decoded content, so that the hash is that of the file `get_attachment` gives;
- `headers`: the trace headers as written, unfolded on one line and cut beyond 4,000 characters: `Return-Path`, `Received` (newest hop first), `Received-SPF`, `Authentication-Results`, `ARC-Authentication-Results`, `DKIM-Signature`, `ARC-Message-Signature`, `ARC-Seal`, `X-Mailer`, `User-Agent`;
- `raw_size` and `raw_sha256`, the SHA-256 of the raw message, that is of the `.eml` file `export_message` writes.

Without a raw file (a seeded mail, or one whose body has not been downloaded), the fields come from the database, `raw_sha256` and the attachment hashes are `null`, and `headers` is empty. A damaged raw file answers `1011` (`RAW_DAMAGED`) and is downloaded again, as for `export_message`.

`message_text {message_id}` returns, as `text`, what the app puts on the clipboard (`sc`): the headers of the `txt` export, an empty line, then the text body with its paragraphs made whole again (`nm-store/src/paragraphs.rs`), and no line break at the end. The stored body keeps the line breaks of the sender's app; `paragraphs::unwrap` removes the ones a tool put there:

- a format=flowed soft break: a line of at least 40 characters ending with a space is joined to the next;
- hard wrapping: per quote depth, the width is the widest line of at most 80 characters; the text counts as wrapped when that width is at least 60, at least two lines end within 12 characters of it, and lines longer than 80 are fewer than those. A line of at least half the width is then joined to the next when the next line's first word would not have fit after it, in characters or in bytes (some tools count bytes). A depth with too few lines to tell (two quoted lines) uses the width of the whole text, prefix included. A line longer than the width made of a single word (a URL) is joined too;
- never joined: different quote depths, the signature after `-- `, list items (`-`, `*`, `•`, `1.`, `2)`…), lines indented by a tab or two spaces, lines with a tab, and blocks of two lines or more that all end a sentence (`.`, `!`, `?`, `:`, `;`, `…`);
- CRLF becomes LF, trailing spaces go, runs of empty lines (whitespace, no-break spaces and invisible characters only) become one, none at either end. Between a quote and the reply, the shallower empty line stays.

The pass is linear in the size of the text. Its unit tests are in `paragraphs.rs`; `crates/nm-store/tests/library.rs` checks a format=flowed mail through import.

## Attachments

`get_thread` lists the attachments of each message in `attachments`: `id`, `filename` (as given by the sender, possibly null), `mime_type`, `size` in bytes, in MIME order. An image the HTML body shows (`cid:`) belongs to the body and is not listed; an image marked inline but shown by no HTML (Apple Mail puts photos in a plain text mail this way) is listed, otherwise nobody would see it. The list comes from the database, filled when the message arrives: opening a thread never reads the raw message.

`get_attachment {attachment_id}` returns `{path, filename, mime_type, size}`. The first call extracts the file from the raw message: it reads it while checking its hash, finds the MIME part by its IMAP number, decodes it (base64, quoted-printable) and writes it to `<data_dir>/attachments.noindex/<raw hash>/<part>/<name>`, read-only (0400), with macOS's quarantine attribute so that Gatekeeper checks it before it runs. The name is made safe (no path, no leading dot, no control character, at most 200 bytes, extension kept; "attachment-<part>.<ext>" without one). The path is recorded in `attachment.disk_path` and reused; a purged file is extracted again. The file belongs to the core: clients copy or open it and ask again before each use. Mail without a raw file (generated by `seed`) lists its attachments but cannot give them: `get_attachment` answers "introuvable".

```sh
nicemailer call get_attachment '{"attachment_id": 12}'
```

## Links of a message

`get_thread` gives with each message its `links`, for the `l` panel (`crates/nm-store/src/links.rs`): a list of `{url, text}`, in order of appearance, each once. The app shows them in groups: links with a text, phone numbers, the attached files, links without a text (`docs/geek-ui.md`).

1. The `<a href>` of the sanitized HTML, with the visible text of the link (`"Voir ma facture"`), whitespace collapsed, cut at 200 characters; `text` is `null` for a link around an image.
2. Then the addresses written in the text part (`https://…`, `http://…`, `mailto:…`) that the HTML does not already have, with `text` `null`. Brackets, parentheses and a final punctuation mark are not part of the address.

3. Then the phone numbers written in the text part (`crates/nm-store/src/phones.rs`), as `tel:` links whose `text` is the number as written: `{"url": "tel:0612345678", "text": "06 12 34 56 78"}`. A `tel:` link of the HTML is kept too, with its text. The URL holds the digits only, after a `+` for an international number. A number is listed once, known by its last nine digits, whether written `06 12 34 56 78` or `+33 6 12 34 56 78`.

**Once per destination.** A newsletter gives each place of a link its own tracking address (`https://r.tracker.fr/lnk/…/13/…`, `…/14/…`) to the same page. When the address carries the page, base64 in a segment of its path (`aHR0cHM6Ly8…` is "https://") or percent-encoded in a parameter (`?url=https%3A%2F%2F…`), the core reads it into `target` (`links.rs`, `target`; a decoded address must have a plausible host, otherwise it is ignored). Links are then the same when their `target`, or their URL when there is none, is the same: one link, named by its first text, its other texts in `other_texts` (each once, in order; only texts of HTML links count, not a number or an address written in the text part). The link still opens through its own URL: the target is a reading of it, shown and used to compare.

**Touching anchors are one link.** A sender may cut a phrase into several links to the same page, `<a>ressent </a><i><a>"les angoisses"</a></i><a> du pays</a>`: anchors separated only by tags or spaces, to the same destination, are joined into one text, `ressent "les angoisses" du pays`, with the spaces as they read. Touching anchors to different places (icons of networks side by side) stay apart. Invisible characters (zero-width space, joiners, soft hyphen) count as spaces: a link whose text is only a U+200B has no text.

Only `http`, `https`, `mailto` and `tel` links are kept (no `javascript:`, `cid:`, `#anchor` or relative path), 2,048 bytes long at most, 500 per message at most.

A mail is full of digits that are not phone numbers (dates, amounts, order numbers, SIRET, IBAN), so the rules are strict: a false number is noise, a missed one only costs a copy. A number is taken when it is

- international: `+` (or `00` glued to the country code) then 8 to 15 digits, the first not 0, in any grouping (`+33 (0)6 12 34 56 78`, `+1 415-555-0100`, `0044 20 7946 0958`);
- national, `0` then a non-zero digit, grouped as in France, Belgium and Switzerland: 10 digits as 2-2-2-2-2, 3-3-2-2 or 4-2-2-2, 9 digits as 2-3-2-2 or 3-2-2-2, with one kind of separator (space, dot or dash);
- 9 or 10 digits without separators only right after a word announcing a phone ("Tél", "phone", "mobile", "portable", "GSM", "standard"…).

A run of digits is read whole (ten digits inside an IBAN are not a number), digits glued to a letter or inside an address (`https://…`, `x@y`) are left out, and so is a fax: announced before ("Fax :", "Télécopie", "F :", "par fax au") or right after ("(Fax)"). National numbers of other countries written without their `+` prefix (`(415) 555-0100`) are not recognized.

Measured on 30,000 mails of a real archive: about 13 µs per mail for all the links and numbers of a mail under 200 KB, 66 ms for the worst (a 10 MB log); the 120 numbers drawn at random from the 15,000 found were all phone numbers. The scanner reads bytes, never goes back further than an address's length, and drops runs of fewer than 8 digits first, so its time stays linear on lines of megabytes.

The HTML is read because the text the core makes from an HTML part (an HTML-only mail, or a botched text part replaced by the HTML, parser version 7) keeps the words of a link and drops its address. The links are computed when the thread is read, not stored: it is one linear scan of bodies already loaded, and a change of the rule needs no reindexing.

`crates/nm-store/tests/links.rs` covers a botched text part replaced by its HTML, an HTML-only mail and a text mail with a phone number; `links.rs` itself the scanner (entities, tags inside a link, `>` inside an attribute, links left open, schemes, bounds, a number given by a `tel:` link and the text once, touching anchors joined or kept apart, one link behind several tracking addresses with its other texts, invisible texts, targets read or refused); `phones.rs` the formats, the numbers that are not phones, addresses, faxes and a huge line.

## Verification codes

`get_thread` gives with each message its `verification_code`: the one-time code it gives when it is a code mail ("Votre code de vérification : 610731"), or `null` (`crates/nm-store/src/codes.rs`). The app offers it first in the `l` panel, where Return copies it (`docs/geek-ui.md`).

**What a code looks like.** A word of its own: 4 to 8 digits (`3280`, `610731`, `78355770`, not a year 1900–2099); 6 to 8 capitals and digits with at least one of each (`4T39TT`, `1RLUBS`, `VSM6WQZ7`), but not a word and a number of two or more digits (`BOOMER30`, `NEWS1120`, `BF2024`: promotion codes); 6 to 8 capitals without a vowel (`PGXFGG`: generators leave vowels out so that a code never spells a word); two groups of three with a dash or a space (`904-426`, `65A-TQC`, `831 884`), given without the separator, as the field that takes it expects. A masked number (`XXXX0405`), a word inside an address (`…/2fa?code=G49DTC&…`) and a number with dots, colons, slashes or another group of digits next to it (dates, times, IP addresses, `2641 801 9509`) are never codes.

**Which mails.** Only a mail that speaks of a code to type: the word "code" with, within 40 characters, a word that says what for (verification, confirmation, security, authentication, login, connexion, validation, one-time use, temporary, OTP, 2FA, "à 6 chiffres", "digits", identity, access, recovery, reset, "expire", "minutes", "valable" before a duration), or "OTP", "passcode", "one-time password" alone. These words start a word ("digitale" is not "digits"). A "code" followed or preceded by what makes it another kind of code does not count: promo, postal, gift, coupon, cadeau, exclusif, offert, réservation, client, erreur, error, exit, status, the law ("code du travail", "code de la sécurité sociale"). A reply or a forward (`RE:`, `TR:`, `Fwd:`) is not a code mail: a conversation between people that speaks of codes holds references and numbers far more often than a code to type.

**Where the code is**, in this order:

1. the subject, when it gives it (`Votre code ChatGPT : 797899`, `[Gandi] 772432 - …`);
2. a line of its own in the body (one word, or two groups of three), within 300 characters of a word that announces it ("code", "OTP", "passcode", "mot de passe"), counted once runs of spaces are collapsed (an HTML layout can put thousands between the sentence and the code). When the subject does not say the mail is a code mail, only the qualified words of the body announce: a newsletter that mentions a "code de sécurité" once does not lend its other "code" words to the numbers of its footer;
3. the first number after an announcing word, on its line or the next (`enter this verification code: 651440`, `est le suivant :\n427242 Il est valable`), then the last one before it (`036764 is your OTP code`); nothing if that number is not a code. Only there may digits be glued to the text (`compte IKEA.677412Veiller`, a layout that lost its line breaks), 6 at least, and not after a letter (`ju0583116` is an order number).

The text part is read first (20,000 characters at most, after collapsing whitespace); the HTML's text when the text part is empty, or when it gives no code (some stop short of it). In a thread, the app shows the code of the latest message that has one.

**Measured** on a real mailbox of 400,000 messages: 615 codes found in 37 s (about 90 µs per message, HTML conversions included), all 26 sample code mails right; among those found, three known false ones (a promotion `25NANO`, a number in a conversation, a scam). The rules were tuned by reading, each time, the codes lost and gained over the whole mailbox: no rule names a sender. Not found: codes in no mail text (an image), a 4-digit code that reads as a year, a forward of a code mail, a permanent PIN announced with no qualifying word ("Votre code d'accès Flying Blue est : 0358").

`codes.rs` covers each way of showing a code (from the real mails, with their codes and names changed), each kind of number that is not one, promotion codes, replies, the HTML fallback and a huge body.

## Locating a message

`locate_message {message_id}` answers where a message is, `{message_id, thread_id, folder_id, account_id}` (`thread_id` null for a message not linked to a thread yet), or the error `1001` when it is not in the database. The app uses it to open a deep link `nicemailer://m/<id>` (`docs/deep-links.md`).

## Unsubscribing from a list

When a message arrives, the core looks for a way to unsubscribe from the list it comes from (`crates/nm-store/src/unsubscribe.rs`) and stores it in `message_unsubscribe`:

1. the `List-Unsubscribe` header (RFC 2369): the first web URL and the first `mailto:` in angle brackets, whitespace of a folded header removed. With `List-Unsubscribe-Post: List-Unsubscribe=One-Click` (RFC 8058) and an https URL, the unsubscribe takes one click;
2. otherwise a link of the HTML body (read by the same scanner as the links above) whose text (or address) announces an unsubscription, in English, French, German, Spanish, Italian or Dutch ("unsubscribe", "se désabonner", "désinscription", "abmelden", "darse de baja", "disiscriviti", "uitschrijven", "opt out"…);
3. otherwise, in a plain text body, a URL on such a line or on the next one.

`get_thread` shows it in `unsubscribe`: `method` (`one_click`, `browser` or `mail`), `source` (`header`, reliable, or `body`, a page of the sender to open), `url`, `mailto`, and `sender_unsubscribed_at`.

`unsubscribe {message_id}` acts on it. With one click, the core sends the POST itself (crate `nm-proto`: https only, 15 s at most, no cookie, no credential, user agent "NiceMailer"), then records `address.unsubscribed_at` and an `unsubscribe` line in the audit log; the sender's screening decision and its mails are left alone. It answers `{method, done, open, error}`: `done` when it unsubscribed, otherwise `open` is the page or the `mailto:` for the client to open, with `error` when a one-click attempt failed and fell back to the page. It refuses a message in Junk, Quarantine or Banned: unsubscribing from a spam tells the spammer the address is read.

The environment variable `NICEMAILER_FAKE_NETWORK`, set by the UI tests, makes the core take a one-click request as accepted without sending it.

```sh
nicemailer call unsubscribe '{"message_id": 42}'
```

## History

`list_history {cursor?, limit?}` returns the audit log (`tracabilite.md`), newest first. Each entry has `date`, `source` (who: `app`, `cli`, `core`…), `entity`, `entity_id`, `action`, the raw JSON `detail`, and a readable `summary` built by the core, such as `Devis → todo` or `alice@example.com → inbox`.

## Tags

- `list_tags {account_id?}`: tags not deleted, with the number of messages carrying each.
- `tag_thread {thread_id, name, remove?}`: adds (or removes) the tag on every message of the thread. The tag is created in the thread's account if needed; a deleted tag of the same name is revived.

A tag is an IMAP keyword. Its keyword is derived from the name: accents removed, other characters replaced by `_`, never starting with `$` (so never the core's reserved `$Nm` prefix). "Équipe Paris" becomes `Equipe_Paris`; if two names give the same keyword in an account, `_2`, `_3`… is added. Names are compared without case, Unicode included.

Each changed message gets `date_modified`, the client as `modified_by`, and a `set_tags` line in the audit log; the usual `MessageChanged`, `ThreadChanged` and `FolderChanged` events are emitted.

## Contact groups

- `list_groups`: groups not deleted, in display order, with their member count, `date_modified` (latest change of the group or of one of its memberships, removals included) and `mail_count` (mails received from the active members: the sum of their `address.sender_count`, a counter kept by ingestion, so the cost follows the number of members, never the number of mails). A client with room for a few groups ranks them with these two (the app's folder column, `docs/geek-ui.md`).
- `set_group_member {group, addr | thread_id, remove?}`: adds (or removes) an address, or the person a thread is with, to the named group, created if needed. Returns `group_id`, `addr` and `changed` (adding an existing member changes nothing). The person of a thread is the one the sender column names: its latest author who is not the user, else, for a thread the user wrote alone, the first recipient of its latest mail who is not the user (To before Cc); drafts are left out. It used to be the author of the latest message outside Sent and Drafts, which was the user whenever their reply was filed with the conversation (Archive, Waiting…). **None of the user's addresses can join a group** (those of `accounts.toml`, accounts and identities, and those of the archive accounts): the group's filter would then match nearly every mail. Adding one fails with an invalid-parameters error; removing one still works. Each reading of `accounts.toml` (at start, then at each change) removes the ones already in a group, as ordinary removals (`Store::remove_own_from_groups`, `groupes.md`, "Adresses de l'utilisateur").

Rules (`specifications/groupes.md`): an address can be in several groups; removing a member keeps its row with `date_removed` (for the future sync between Macs); a deleted group recreated under its name starts empty. Every change is journaled (`create`, `add_member`, `remove_member`).

## Contacts

`list_contacts {query?, cursor?, limit?}`: known addresses sorted by address, with their name, screening decision, the decision of their domain (`domain_target`), their authentication policy, groups and number of messages received from them. Decisions are taken with `set_address_screening` and `set_domain_screening` (`folder-workflow.md`). `query` filters on a part of the address or the name, case-insensitively.

## Threads across folders

The threads of a tag, of a group's members or of a contact are criteria of `filter` in `list_threads`, used with `everything`: `tag` (a name: the tags of that name in every account, compared without case, since each account has its own "Clients"), `group_id` and `address_id`. `group_ids` (several groups, OR-ed: the members of any of them) and `not_group_ids` (a thread is left out as soon as one of its mails in the listed folders is from or to a member of one of them, the rule of `not_from`) are attributes like the others, usable in any list, a folder included: the Group… attribute of the app. `group_id` and `group_ids` combine by "and": the list of a group narrowed to the members of another. `tags` (several tag names, OR-ed, each read like `tag`) and `not_tags` (a thread is left out as soon as one of its mails carries one of them) are the Tag… attribute, in any list; `tag` and `tags` combine by "and", and a name no tag has matches nothing when wanted and leaves nothing out when excluded. `no_tag` keeps the threads none of whose mails carries an active tag, and `no_group` those none of whose mails in the listed folders is from or to a member of an active group (the rule of `not_group_ids`, for every group): "without tag" and "without group" of the app. Both read every tag (a few rows) or every active membership (through the partial index of the active ones), then the mails through their index. For a group or a contact, a thread qualifies when one of its messages is from the address or addressed to it (To, Cc). The list is then an ordinary one: newest first on the date shown, paginated by cursor, sorted by reception or by modification, and combined with the other attributes ("unread mails of the group"). Each thread is listed with the folder of its latest message (`folder_id`), from which moves start. On 275,000 mails: a tag of 5,000 threads 18 ms, a group of 6 colleagues 35 ms, a group of 67 addresses covering 140,000 mails 150 ms. They replace `list_threads_by`, which returned at most 200 threads, ignored the attributes and the sort, and ordered a thread by its latest mail from or to the member rather than by the date it showed.

```sh
nicemailer call list_threads '{"everything": {}, "filter": {"tag": "Clients", "seen": false}}'
nicemailer call list_threads '{"everything": {}, "filter": {"group_id": 1}, "sort": "modified"}'
```

## Text of a mail, and reinterpretation

The text body of a mail is its text part, except when that part was botched by the sender's own conversion from HTML: HTML entities left as written (`&nbsp;`, `&#39;`), tags or remains of attributes (`</p>`, `<br />`, `" target="_blank" style=…`). Such a part also tends to glue paragraphs together and to miss some. When the mail has an HTML part, the text is then the core's conversion of it (`html_text.rs`); without one, the entities are decoded and the tags kept (they may be meant). Every HTML5 named entity is decoded. Entities a sender encoded twice or three times (`Zoom :&amp;nbsp;https://…`, which every mail client shows as `&nbsp;`) are removed too: encoded once in the HTML view (a plain `&amp;` of a link is kept), decoded in the text.

The text made from HTML leaves out what the mail hides, as a mail app does not show it: an element whose `style` attribute hides it (`display: none`, `visibility: hidden`, `opacity: 0`, a height of 0 with `overflow: hidden`; `css::hides`). A class of the mail's `<style>` that shows it again with `!important` keeps it (the "hidden from Outlook" pattern, `css::shown_classes`). Classes hidden by the `<style>` alone are not followed: which rule wins depends on CSS specificity, and a wrong guess would drop the body of the mail (as in an Apple newsletter, where the class hiding the content at the top level is shown by a more specific rule). A hidden element never closed (broken HTML) hides nothing: the text is then made as before. The first hidden element before any visible text is the **preview text** a newsletter writes for the list of mails ("C'est délirant."): it is the message's snippet, as mail apps show it, and stays out of the body (`html_text::convert`). On 3000 recent mails of a real mailbox, 1057 had such a preview; 4 lost more than 15 visible words, all parts their sender hides on purpose (template blocks marked `condition_show="false"`, a block for plain-text readers).

The core records with each body the version of the parser that produced it (`message_body.parser_version`, now 11). At start, if bodies of an older version remain, it reinterprets them in the background (`reindex.rs`, `stockage-brut.md` section 8): it reads their raw file again and rewrites the body, the snippet, the body of the search index row and the unsubscribe information, and gives a name to the addresses that have none. A change that can be applied to what is stored needs no raw file: the entities encoded twice are removed from the stored text and HTML directly. Nothing to do: nothing happens. The app keeps being answered meanwhile; a thread already open shows its new body when opened again. The core's log reports the progress and the end:

```
reindex : 274939 mails analysés par une version antérieure, en tâche de fond
reindex terminé : 31599 mails réanalysés, 350 corrigés sans relecture, 242990 inchangés, … en 116.8 s
```

## Tests

`crates/nm-store/tests/library.rs` covers each of these features: pagination of both sorts, the per-sender display choice, history pagination and summaries, tag creation, idempotence, Unicode case and keyword collisions, group rules (the person of a thread, tombstones, revived group; the user's addresses refused and removed are in `crates/nm-store/tests/correspondent.rs`), contact filter and pagination, threads by contact, and export in the three formats without overwriting. `crates/nm-store/tests/record.rs` covers `message_record`: names, every recipient, `Sender` and several authors, the `Date` header as written, the trace headers unfolded, the hashes of the raw message and of a decoded attachment, the fallback without raw file and a damaged one (`crates/nm-daemon/tests/record_rpc.rs`, the same through the socket). `crates/nm-store/tests/attachments.rs` covers the attachments: listed per message, images of the HTML left out, inline photos of a text mail kept, extraction (content, read-only, quarantine, reuse, re-extraction after a purge), names that try to leave the directory, and the errors; `attachments.rs` itself covers the IMAP part numbers and the decoding. `crates/nm-store/tests/unsubscribe.rs` covers what reception stores for each kind of list mail, the view, the record of a one-click unsubscribe and the refusals; `unsubscribe.rs` the header and body formats; `crates/nm-proto` the POST, against a local HTTP server (no request leaves the machine).

## Grouped commands

`set_thread_seen`, `set_thread_flagged` and `tag_thread` take `thread_id`, or `thread_ids` for several conversations (exactly one of the two, else `INVALID_PARAMS`); `move_threads` moves several (`docs/folder-workflow.md`). Several conversations are one transaction, one commit and one batch of events (`ChangeSet::merge`), where one request per conversation made the app's rows change one after the other (`specifications/ui.md`, "Actions groupées"). A conversation gone meanwhile (merged, deleted by the synchronization) is skipped; one of a read-only account refuses the whole command before any write. With `thread_id` alone, an unknown conversation is still an error. Tests: `crates/nm-store/tests/screening.rs` (several conversations moved, refused, held for a confirmation and undone at once; read, flagged and tagged together), `crates/nm-daemon/tests/workflow_rpc.rs` (the same through the socket, and the parameters refused).
