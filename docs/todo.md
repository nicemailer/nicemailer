# To do

Work identified but left for later, one section per ticket. A ticket says what is missing, why it matters and where to start. Remove it in the change that does it.

## Workflow consistency check (`nicemailer db verify`)

**What.** A core function, exposed as `nicemailer db verify`, that walks the database and reports every place where the workflow's invariants do not hold, without changing anything:

- a mail outside the folder its sender's decision asks for, in a folder the decision files from (`ScreeningTarget::filing_sources`), when its verdict is accepted: a filing that was missed;
- a conversation spread over several work or filing folders (Inbox, Todo, Waiting, Archive, Newsletter, Paperwork) without a new mail explaining it (`dossiers.md`, "Une conversation, pas un mail");
- a mail back in Quarantine after it left it (an earlier `move` line in `audit_log` from Quarantine);
- a `$NmWasTodo` / `$NmWasWaiting` keyword on a mail outside the Inbox;
- denormalized counters (`folder.total_count`, `unread_count`, `thread`, `thread_folder`) different from what `message` gives.

**Why.** Many code paths move mail (moves, filings, undo, and later the IMAP sync, returns from Todo/Waiting, replies). A bug in one of them leaves the mailbox inconsistent without any error. `specifications/analyse-critique.md` asks for the same command, run by the tests after each scenario.

**Where to start.** A `verify.rs` module in `nm-store` returning a list of findings (kind, entity, ids); the `nm-store` integration tests call it after each test of `screening.rs` and `store.rs`.

## Repair (`nicemailer db verify --fix`)

**What.** Correct what `db verify` finds when the right state is certain: recompute the counters, remove stray return keywords, run the missed filings (with source `core` and their `audit_log` lines). Report without fixing what needs a decision of the user (a conversation spread over folders).

**Why.** A user should not need to recreate a database because a counter drifted.

**Depends on** the ticket above.

## Stricter validation in the commands

Done in the folder workflow change: a suspect sender is never banned and needs a confirmation for a favorable decision; Quarantine is never a target. Still to check in the commands, once the features exist:

- a decision on an address of the account itself (its identities from `config.toml`, not only `account.address`): refuse rather than ignore;
- a move from a folder without a role (the user's own folders): today allowed to any command target, the spec says nothing;
- a decision whose domain has the opposite decision (address `inbox`, domain `banned`): allowed, the address wins, but the app could say so.

## Undo: what it does not restore yet

- The return marks (`$NmWasTodo`, `$NmWasWaiting`) a move removes are not journaled, so undo cannot put them back.
- The undo stack of the app lives for the session; the tokens stay valid in the core, but a restarted app forgets them.
- Undo covers moves, decisions and authentication policies; not read/unread, flags or tags.

## Several senders in a conversation

The user chose to decide on the sender of the latest received mail (`dossiers.md`, "Décision sur l'expéditeur"). If a colleague in copy is ever banned this way, revisit: the earlier spec asked the user which senders the decision concerns.

## Negation and narrower text search in the attribute selector

**What.** The rest of the attribute selector specified in `specifications/ui.md` ("Sélecteur d'attributs", "Négation"):

- Contains… gives way to **Text…** (subject and body, not the addresses) and **Subject…** (subject only); `/` keeps its own criterion over every column (chip "anywhere: …");
- excluded values for From…, To…, Text… and Subject…: a "− not acme" row under the "+ acme" row of the value panel, or a value typed with a leading `-`; chips "not from: …", "not to: …"; positive values OR-ed, excluded values AND-ed; a thread is excluded as soon as one of its mails in the folder matches;
- a value starting with `@` is a domain: exact, with its subdomains (`@acme.fr` matches `x@mail.acme.fr`, not `x@notacme.fr`);
- `-word` inside Text… and Subject… excludes a word.

**Why.** The owner manages domains at registrars: they need the mails that talk about acme.fr without those sent to or by acme.fr (Text… `acme.fr`, not from `@acme.fr`, not to `@acme.fr`). Today Contains… also searches the addresses, so it brings back exactly the mails to leave out, and nothing can be excluded.

**Where to start.** `ThreadFilter` in `crates/nm-api/src/lib.rs` (new fields `not_from`, `not_to`, `text`, `subject`, `anywhere`), the thread query of `nm-store` (`NOT EXISTS` on the addresses; FTS5 column filters `{subject body}: …` and `subject: …` on `message_fts`, `NOT` for the excluded words, every typed word quoted so that FTS syntax in the text is not interpreted), then `ThreadFilter` and the value panel of the app (`Attributes.swift`, `GeekController.valueEntries`). Tests: store tests on each criterion and their combinations, `AttributesTests`, `GeekControllerTests`, and a `filters.uiscript` scenario with the registrar example.
