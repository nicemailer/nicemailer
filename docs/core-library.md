# Core features behind the screens

JSON-RPC methods added for the geek theme (`docs/geek-ui.md`). They can be called by any client, for instance with `nicemailer call METHOD 'PARAMS'`. All lists are newest first unless stated otherwise; paginated lists return `next_cursor`, to pass back as `cursor`.

## Sorting a folder by modification

`list_threads` takes `folder_id`, or `folder_ids` to list several folders together (the unified view "all" of the app lists the folders of one role in every account), or `everything` for the virtual folder Everything: `{}` for every account, `{"account_id": 3}` for one (exactly one of the three). With `folder_ids`, each folder is read with its own index, one page at most, and the pages are merged, so a page costs the same whatever the size of the folders. Everything reads whole threads from the table `thread`, one walk of an index whatever the number of folders, and shows each thread in the folder of its latest message with that folder's counters; `seen` then looks at the whole thread. It also takes `"sort": "received"` (default) or `"sort": "modified"`, and `filter`, attributes combined with AND: `seen`, `flagged`, `attachments`, `answered`, `conversation` (true / false), `from` and `to` (lists of parts of names or addresses, OR-ed; a value starting with `@` is a domain, matched exactly with its subdomains: `@acme.fr` matches `x@mail.acme.fr`, not `x@notacme.fr`), `not_from` and `not_to` (same values, a thread is left out as soon as one of its messages matches one), `contains` (full-text words anywhere, addresses included: the `/` search), `text` (words of the subject and body only, FTS5 column filter `{subject body}`), `subject` (words of the subject only), `after` and `before` (Unix times, on the latest message received). In `contains`, `text` and `subject`, every word is a prefix, quoted so that FTS syntax typed by the user is literal, and a word written `-word` leaves out the threads holding it. An unknown key in `filter` is refused (`INVALID_PARAMS`) rather than ignored: an app newer than the running core gets an error instead of the whole list. `from` and `to` match the address or the name of the contact (`address.name`), not the display name of each mail: a mail from a shared address such as `notifications@github.com` showing a person's name is not from that person. Address and attachment criteria look at the thread's messages in every listed folder, not only the one the thread is shown in. With several folders of the same account, a thread present in more than one of them is listed once, in the folder of its latest message among them, on every page. Every criterion is a set of thread ids computed once through an index (`address` then `idx_message_from` or `idx_message_address_address`; the partial indexes of flagged, answered and attachment messages; one full-text query for the wanted words of a text criterion and one for its excluded words), never a lookup run for each row: a page costs about the same whether the criterion is frequent or rare. Measured on 275,000 mails: every list under 60 ms (`make bench`). With `modified`, threads are ordered by the latest change the user made to one of their messages in the folder (move, flag, read, tag): "what did I just do". Each thread row also carries `folder_id`, `date_last_modified` and its `tags`.

Backed by the column `thread_folder.date_last_modified` and its index `idx_thread_folder_modified` (`thread.date_last_modified` and `idx_thread_modified` for Everything), kept up to date with the other thread aggregates.

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

- `list_groups`: groups not deleted, in display order, with their member count.
- `set_group_member {group, addr | thread_id, remove?}`: adds (or removes) an address, or the sender of a thread (author of its latest message outside Sent and Drafts), to the named group, created if needed. Returns `group_id`, `addr` and `changed` (adding an existing member changes nothing).

Rules (`specifications/groupes.md`): an address can be in several groups; removing a member keeps its row with `date_removed` (for the future sync between Macs); a deleted group recreated under its name starts empty. Every change is journaled (`create`, `add_member`, `remove_member`).

## Contacts

`list_contacts {query?, cursor?, limit?}`: known addresses sorted by address, with their name, screening decision, the decision of their domain (`domain_target`), their authentication policy, groups and number of messages received from them. Decisions are taken with `set_address_screening` and `set_domain_screening` (`folder-workflow.md`). `query` filters on a part of the address or the name, case-insensitively.

## Threads across folders

The threads of a tag, of a group's members or of a contact are criteria of `filter` in `list_threads`, used with `everything`: `tag` (a name: the tags of that name in every account, compared without case, since each account has its own "Clients"), `group_id` and `address_id`. For a group or a contact, a thread qualifies when one of its messages is from the address or addressed to it (To, Cc). The list is then an ordinary one: newest first on the date shown, paginated by cursor, sorted by reception or by modification, and combined with the other attributes ("unread mails of the group"). Each thread is listed with the folder of its latest message (`folder_id`), from which moves start. On 275,000 mails: a tag of 5,000 threads 18 ms, a group of 6 colleagues 35 ms, a group of 67 addresses covering 140,000 mails 150 ms. They replace `list_threads_by`, which returned at most 200 threads, ignored the attributes and the sort, and ordered a thread by its latest mail from or to the member rather than by the date it showed.

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

`crates/nm-store/tests/library.rs` covers each of these features: pagination of both sorts, the per-sender display choice, history pagination and summaries, tag creation, idempotence, Unicode case and keyword collisions, group rules (sender of a thread, tombstones, revived group), contact filter and pagination, threads by contact, and export in the three formats without overwriting. `crates/nm-store/tests/attachments.rs` covers the attachments: listed per message, images of the HTML left out, inline photos of a text mail kept, extraction (content, read-only, quarantine, reuse, re-extraction after a purge), names that try to leave the directory, and the errors; `attachments.rs` itself covers the IMAP part numbers and the decoding. `crates/nm-store/tests/unsubscribe.rs` covers what reception stores for each kind of list mail, the view, the record of a one-click unsubscribe and the refusals; `unsubscribe.rs` the header and body formats; `crates/nm-proto` the POST, against a local HTTP server (no request leaves the machine).
