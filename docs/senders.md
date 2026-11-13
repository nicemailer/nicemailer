# People and robots

NiceMailer tells the mails written by people (`alice.dupont@gmail.com`) from those sent by machines (`info@amazon.fr`, a shop's newsletter signed with a salesperson's name). The verdict is on the **address**, drawn from clues read in the **headers** of its mails; it is used by the **filter human / robot** of the list and shown in the **Contacts** screen, where the user can force it. Nothing moves: no folder is created, no mail filed.

Specification (French, the reference): `specifications/expediteurs.md`. It holds the measures on a real mailbox the rules were built from.

| | Code |
| --- | --- |
| Clues, natures, verdict rules (pure) | `crates/nm-store/src/sender_kind.rs` |
| Counts, verdicts in the database, forced verdict, catch-up | `crates/nm-store/src/senders.rs` |
| Clues read at ingestion | `crates/nm-store/src/ingest.rs` (`ParsedMessage::sender_signals`) |
| Recording at insertion | `crates/nm-store/src/insert.rs` |
| Filter | `crates/nm-store/src/queries.rs` (`filter_clause`, `human`) |
| Catch-up thread | `crates/nm-daemon/src/main.rs` (`start_sender_backfill`) |
| App | `Attributes.swift` (`ThreadFilter.human`), `GeekController.swift` (`o` on Contacts), `AppModel+Workflow.swift` (`setSenderKind`) |

## Clues of a mail

Read once, from the headers, when the mail arrives (before its body is downloaded). Each clue is a bit of `message.sender_signals`, stored with the **nature** of the mail:

| Nature | When |
| --- | --- |
| automatic reply | `Auto-Submitted: auto-replied`, `X-Autoreply`; or `Auto-Submitted: auto-generated` alone (Exchange's answers to calendar invitations) |
| bulk | `List-Unsubscribe` or `Precedence: bulk` outside a discussion list, `Feedback-ID`, a `noreply` local part in any spelling, an `X-Mailer` of a sending platform (Mailchimp, Sendinblue, Adobe Campaign…) |
| discussion list | `List-Post` (not `NO`), `X-BeenThere`, a Google Groups `List-Id` |
| human | an `X-Mailer` / `User-Agent` of a mail client or a browser, a Gmail `Message-ID`, a reply (`In-Reply-To`, `References`) |
| bulk (platform relay) | a header only sending platforms add (`X-SES-`, `X-Mandrill-`…), without any sign of a person |
| none | nothing of the above |

The order matters, and each choice comes from a trap seen in the measures: a Google Group carries `List-Unsubscribe` on every mail (the discussion list is recognized first); security gateways add headers that look like platforms' (`X-TM-`, `X-MC-Unique`, `X-EMS-`), so the platform prefixes are precise (`X-MC-User`); people relayed by Mandrill keep their mail client's clues; PHPMailer is used by webmails of real people and counts for nothing.

## Verdict on an address

Each received mail is counted on its sender's address by nature (`address.sender_count`, `sender_bulk`, `sender_list`, `sender_human`, `sender_client`), automatic replies left out. The verdict (`address.sender_kind`) is computed again from these counts at each mail, never by reading the mails again. The first rule that concludes decides:

1. the verdict forced by the user (`address.sender_kind_user`);
2. one of the user's own addresses: person;
3. no mail counted: person if the user wrote to it, else robot;
4. mostly discussion-list mail: person;
5. the user wrote to it (`address.written_to`, a recipient of a mail of Sent): person if at least 10% of its mails show a person or less than half are bulk, else robot;
6. mostly bulk: robot;
7. a service local part (`info`, `contact`, `support`, `hello`…): robot;
8. mostly written with a mail client or Gmail: person;
9. no bulk mail, and a public domain (`data/public-domains/`) or mostly replies: person;
10. else robot.

## Verdict of a mail and of a conversation

`message.sender_kind` is `person` for the user's own mails (Sent, Drafts), `robot` for a mail that is automatic itself (bulk, platform, automatic reply) whatever its address, else the verdict of its address. A shop that writes to the user and sends its newsletter from the same address is a person, and its newsletters are robots.

`thread.has_person` is 1 when a mail of the conversation, in any folder, is `person`. It is kept by `refresh_thread` like the other aggregates of a thread, and by `senders.rs` where mails change kind without it.

When the verdict of an address changes (its first mails, a mail written to it, the user's choice), all its mails and their conversations are rewritten in the same transaction, through `idx_message_from`.

## Filter

The attribute selector (`f`) has **human** / **robot**, a yes / no attribute sent to the core as `ThreadFilter.human`. `human` keeps the conversations with a mail of a person, `robot` the others: the two split the list. The criterion is a test of `thread.has_person` while the list walks its index: on 400,000 mails, three pages of Archive take 2 ms, as without filter. (A set of threads built at each request, as for attachments, took 30 to 160 ms; hence the column.)

## Contacts

The Contacts screen shows the origin of each address, "human" or "robot", followed by "*" when the user chose it. `o` asks: `h` human, `r` robot, `-` automatic (the computation takes over again). The core method is `set_sender_kind` (`address_id`, `kind`: `person`, `robot` or absent); the choice is journaled (`audit_log`, action `sender_kind`) and undone by `undo` like a screening decision. It is not yet synchronized between Macs.

## Catch-up of stored mail

The mails stored before schema version 12 have no clues (`sender_signals IS NULL`, in the partial index `idx_message_sender_pending`). At start, the core launches a thread that reads them (`senders::backfill`):

- by batches of 500 mails, in the order of their ids;
- the start of each raw file only (`RawStore::read_header`: decompressed up to the empty line that ends the headers), read in parallel without the store lock;
- each batch in one transaction that does what the insertion does (`senders::record`): clues, count, verdict, conversation. A crash loses one batch at most, read again at the next start; the counts stay exact, since a mail is counted in the transaction that marks it read. A mail inserted meanwhile already has its clues and is skipped.

A mail without a readable raw file gets no clue. On 400,000 mails: 22 s, at most 120 ms per batch. Log:

```
personnes et robots : 399864 mails à classer, en tâche de fond
personnes et robots : 399864 mails classés, 0 sans brut lisible, en 22.5 s
```

Until it is done, the mails not read yet have no verdict: the filter counts them with the robots.

The fake mailbox (`nicemailer seed`) has no headers to read: its mails get the clues their kind of sender would give (contacts write with a mail client, newsletters and invoices are bulk), then `senders::classify_all` computes counts, verdicts and kinds in a few statements.

## Limits

- Spam written by people (offers sent from Hotmail or Gmail) is classified as a person; the screening keeps new ones in Quarantine.
- A newsletter sent from a person's Outlook, without any list header, is a person: nothing in its headers tells otherwise.

## Tests

- `crates/nm-store/src/sender_kind.rs` (unit): every clue and trap, the ten rules in order.
- `crates/nm-store/tests/senders.rs`: verdicts at insertion, a mail written to an address, a mixed address, a discussion list, automatic replies, the filter (and the user's reply in Sent), forcing and undo, contacts, and the catch-up, which must give exactly what the insertion gives.
- `crates/nm-store/src/raw.rs`: `read_header`. `crates/nm-store/src/seed.rs`: the fake mailbox is classified. `crates/nm-daemon/tests/workflow_rpc.rs`: `set_sender_kind` through the socket.
- App: `AttributesTests`, `GeekControllerTests` (the selector), `WorkflowTests` (`o` on Contacts), and the scenario `UITests/senders.uiscript`.
