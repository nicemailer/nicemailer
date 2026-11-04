# The folder workflow

How moving a conversation records a decision on its sender, files the sender's other mails, and how it is all undone. The rules are those of `specifications/dossiers.md` ("Déplacer un mail", "Décision sur un expéditeur") and `specifications/authentification-expediteur.md` (sections 4 and 5). They live in the core (`nm-core` for the pure rules, `crates/nm-store/src/screening.rs` and `commands.rs` for the database), so the app, the CLI and future clients behave the same.

## A move can decide on the sender

| Move | Effect |
| --- | --- |
| To Newsletter, Paperwork or Banned, from any folder | decision on the sender, and filing of its other mails |
| To Inbox, from Quarantine or Banned | decision `inbox` on the sender, and filing |
| To Inbox from another folder, to Todo, Waiting, Archive or Deleted | this conversation only |
| To Junk | this conversation only, even from Quarantine: spam changes address every time |

**Which sender.** The author of the conversation's latest received mail, leaving out your own mails (Sent, Drafts, or sent from the account's address). A conversation with several senders is decided on that one only; the others keep their decision. A conversation with only your own mails decides nothing.

**Where it is recorded.** On the address (`address.screening_target`), with the date and the source of the change, and a `screening` line in the history naming the mail that triggered it. Removing a decision keeps the date, so an import never gives back a decision you removed.

## Filing the sender's other mails

Taking or changing a decision files the mails already received from that sender, in every account (archive accounts included):

| Decision | Mails filed | From |
| --- | --- | --- |
| Inbox | to the Inbox | Quarantine only: archived mail has been dealt with |
| Newsletter, Paperwork, Banned | to the decision's folder | Quarantine, Inbox, Archive, and the two other folders among Newsletter, Paperwork and Banned |

Never moved: Todo, Waiting, Sent, Drafts, Deleted and Junk (your own commands), and the flagged mails of the Inbox. Removing a decision moves nothing.

**Authentication.** A favorable decision (Inbox, Newsletter, Paperwork) only files the mails its rule accepts: with the address's policy `auto`, a `pass` mail, or an unverified one (`none`, `unknown`) as long as no `pass` mail was ever seen from the address; `required` wants `pass`; `not_required` takes everything. A forged mail stays in Quarantine. Banned files whatever the verdict. Mails of archive accounts (mbox imports) are filed without condition.

**A sender suspected of spoofing** (a mail of the conversation from that sender has `auth_suspect = 1`):

- moving it to Banned concerns that mail only: banning the real address because of a forger would be a mistake;
- a favorable move is refused with the error `1005` (`CONFIRMATION_REQUIRED`, `data: {addr, verdict}`) and changes nothing. Sent again with `confirm_suspect: true`, it records the decision; with `auth_policy: "not_required"` too, the address accepts unauthenticated mail from now on (a legitimate sender using a badly configured tool).

## Domain decisions

`set_domain_screening` records a decision on a domain (`domain.screening_target`). It applies to the addresses of the domain, and of its subdomains (`news.shop.fr` belongs to `shop.fr`), that have no decision of their own; an address's own decision always wins. A favorable domain decision only files `pass` mails.

## Undo

Every command that moves or decides returns an `undo` token. `undo {token}` puts each mail back in the folder it left, and each decision and policy back to its previous value. A change made since by someone else wins: a mail moved again, or a decision changed again, is left alone, so undoing twice is harmless.

The token is the range of the command's lines in the history (`audit_log`): a command holds the write lock from its first write to its commit, so its lines are consecutive. Undoing replays them backwards and writes its own lines, marked `undo_of`.

## Contract

| Method | Parameters | Result |
| --- | --- | --- |
| `move_thread` | `thread_id`, `target`, `from_folder_id?`, `confirm_suspect?`, `auth_policy?` | `WorkflowResult` |
| `set_address_screening` | `addr`, `target?` (absent: remove) | `WorkflowResult` |
| `set_domain_screening` | `domain`, `target?` | `WorkflowResult` |
| `undo` | `token` | `WorkflowResult` |

`WorkflowResult`: `changed` (messages modified), `moved` (mails of the conversation, or put back by `undo`), `filed` (other mails of the sender), `decision` (`{kind: address|domain, name, before, after}`, absent when nothing was decided) and `undo` (absent when nothing changed). A decision received from another Mac (`config_sync`) files nothing: that Mac did. `list_contacts` gives each address its own decision, its domain's (`domain_target`) and its `auth_policy`.

## CLI

```
nicemailer address news@shop.fr newsletter   # or inbox, paperwork, banned, none
nicemailer domain shop.fr banned
nicemailer undo 120:131
```

Each prints the decision, the number of mails moved and filed, and the `undo` command.

## In the app

See `geek-ui.md`: the message after a move, `z`, the confirmation of a suspect sender, and `m` / `M` on the contacts screen.

## Tests

`nm-core` tests the rule tables (decisions by move, folders filed, authentication policies). `crates/nm-store/tests/screening.rs` builds small mailboxes by hand and covers every row of the tables, the sender chosen in a conversation, the flagged Inbox mails, the authentication rules and archive accounts, the suspect sender, removal, `config_sync`, domain decisions and subdomains, and undo (decision, policy, a mail moved since, twice, bad tokens). `crates/nm-daemon/tests/workflow_rpc.rs` goes through the socket: results, the `1005` error and its data.
