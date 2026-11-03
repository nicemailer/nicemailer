# Core features behind the screens

JSON-RPC methods added for the geek theme (`docs/geek-ui.md`). They can be called by any client, for instance with `nicemailer call METHOD 'PARAMS'`. All lists are newest first unless stated otherwise; paginated lists return `next_cursor`, to pass back as `cursor`.

## Sorting a folder by modification

`list_threads` takes `folder_id`, or `folder_ids` to list several folders together (the unified view "all" of the app lists the folders of one role in every account). Each folder is read with its own index, one page at most, and the pages are merged, so a page costs the same whatever the size of the folders. It also takes `"sort": "received"` (default) or `"sort": "modified"`, and `filter`, attributes combined with AND: `seen`, `flagged`, `attachments`, `answered`, `conversation` (true / false), `from` and `to` (lists of parts of names or addresses, OR-ed), `contains` (full-text words), `after` and `before` (Unix times, on the latest message received). With several folders, a thread present in more than one of them is listed once, in the folder of its latest message among them, on every page. The conditions are added to the query of each folder: correlated lookups on the thread's messages through their indexes, and one full-text query for `contains`. With `modified`, threads are ordered by the latest change the user made to one of their messages in the folder (move, flag, read, tag): "what did I just do". Each thread row also carries `folder_id`, `date_last_modified` and its `tags`.

Backed by the column `thread_folder.date_last_modified` and its index `idx_thread_folder_modified`, kept up to date with the other thread aggregates.

```sh
nicemailer call list_threads '{"folder_id": 4, "sort": "modified", "limit": 10}'
```

## Text or HTML, per sender

`set_body_format {addr, format}` remembers how the mails of an address are displayed: `"text"`, `"html"`, or `null` for the theme's default. `get_thread` returns, for each message, `body_html` (sanitized), `has_remote_content` and `from_body_format`, the choice remembered for its sender.

The preference is stored in `address.body_format`. It is local to the Mac: not synced, not journaled.

## Saving a message

`export_message {message_id, format, dir?}` writes one message to a file and returns its `path`:

| Format | Content |
| --- | --- |
| `eml` | the raw message, byte for byte, read from the raw storage and checked against its hash |
| `txt` | From, To, Cc, Date, Subject, then the text body |
| `md` | the same in Markdown, with the subject as title |

The file goes to `dir` (absolute), by default `~/Downloads`, named `<date> <subject>.<ext>`. An existing file is never overwritten: ` (2)`, ` (3)`… is added.

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

`list_contacts {query?, cursor?, limit?}`: known addresses sorted by address, with their name, screening decision, groups and number of messages received from them. `query` filters on a part of the address or the name, case-insensitively.

## Threads across folders

`list_threads_by {tag_id | group_id | address_id, limit?}` lists the latest threads (200 at most by default) of a tag, of a group's members, or of a contact, in every folder. For a group or a contact, a thread qualifies when one of its messages is from the address or addressed to it (To, Cc). Each thread is listed with the folder of its latest message (`folder_id`), from which moves start.

## Tests

`crates/nm-store/tests/library.rs` covers each of these features: pagination of both sorts, the per-sender display choice, history pagination and summaries, tag creation, idempotence, Unicode case and keyword collisions, group rules (sender of a thread, tombstones, revived group), contact filter and pagination, threads by contact, and export in the three formats without overwriting.
