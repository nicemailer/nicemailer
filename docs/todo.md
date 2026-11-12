# To do

Work identified but left for later, one section per ticket. A ticket says what is missing, why it matters and where to start. Remove it in the change that does it.

## Workflow consistency check (`nicemailer db verify`)

**What.** A core function, exposed as `nicemailer db verify`, that walks the database and reports every place where the workflow's invariants do not hold, without changing anything:

- a mail outside the folder its sender's decision asks for, in a folder the decision files from (`ScreeningTarget::filing_sources`), when its verdict is accepted: a filing that was missed;
- a conversation spread over several work or filing folders (Inbox, Todo, Waiting, Archive, Newsletter, Paperwork) without a new mail explaining it (`dossiers.md`, "Une conversation, pas un mail");
- a mail back in Quarantine after it left it (an earlier `move` line in `audit_log` from Quarantine);
- a `$NmWasTodo` / `$NmWasWaiting` keyword on a mail outside the Inbox;
- denormalized counters (`folder.total_count`, `unread_count`, `thread`) different from what `message` gives.

**Why.** Many code paths move mail (moves, filings, undo, and later the IMAP sync, returns from Todo/Waiting, replies). A bug in one of them leaves the mailbox inconsistent without any error. `specifications/analyse-critique.md` asks for the same command, run by the tests after each scenario.

**Where to start.** A `verify.rs` module in `nm-store` returning a list of findings (kind, entity, ids); the `nm-store` integration tests call it after each test of `screening.rs` and `store.rs`.

## Repair (`nicemailer db verify --fix`)

**What.** Correct what `db verify` finds when the right state is certain: recompute the counters, remove stray return keywords, run the missed filings (with source `core` and their `audit_log` lines). Report without fixing what needs a decision of the user (a conversation spread over folders).

**Why.** A user should not need to recreate a database because a counter drifted.

**Depends on** the ticket above.

## Stricter validation in the commands

Done in the folder workflow change: a suspect sender is never banned and needs a confirmation for a favorable decision; Quarantine is never a target. Still to check in the commands, once the features exist:

- a decision on an address of the account itself (its identities from `accounts.toml`, not only `account.address`): refuse rather than ignore;
- a move from a folder without a role (the user's own folders): today allowed to any command target, the spec says nothing;
- a decision whose domain has the opposite decision (address `inbox`, domain `banned`): allowed, the address wins, but the app could say so.

## Undo: what it does not restore yet

- The return marks (`$NmWasTodo`, `$NmWasWaiting`) a move removes are not journaled, so undo cannot put them back.
- The undo stack of the app lives for the session; the tokens stay valid in the core, but a restarted app forgets them.
- Undo covers moves, decisions and authentication policies; not read/unread, flags or tags.

## Settings: what phase 1 left out

- **The app shows nothing of `config.toml` and `accounts.toml` yet**: no banner for `ConfigError` pointing at the file and line (`configuration.md`, section 4), no list of accounts. The core gives everything needed: `config_status`, `list_accounts`, the events `ConfigError`, `ConfigChanged`, `AccountChanged`, `AccountError`.
- **`set_config`, `nicemailer account rename` and `account remove`** (`configuration.md`, sections 5 and 9): writing the file with `toml_edit`, keeping comments; renaming a key in the file and the database together. The plan puts the writing with `add_account` (phase 7).
- **Settings read at start only** (`events.*`) are accepted while the core runs and apply at the next start; the others are validated but not used yet (table in `docs/configuration.md`).
- **The Keychain item of an account** is not deleted or renamed yet: it should be by `account remove` and `account rename`.
- **Edits made while the core was stopped** are not in `audit_log`: the first reading has nothing to compare with. Storing the keys applied last would allow it.

## Several senders in a conversation

The user chose to decide on the sender of the latest received mail (`dossiers.md`, "Décision sur l'expéditeur"). If a colleague in copy is ever banned this way, revisit: the earlier spec asked the user which senders the decision concerns.

## Replying: what v1 leaves out

`specifications/reponse.md` (section 2) delivers the reply to all, plain text, sent through SMTP with an undo delay. Left for later, each its own change:

- **Drafts changed on another device** (`brouillons.md`, section 6): the adoption of a version changed on the phone (today its copy disappearing is seen as a deletion, or written over by a version changed here), conflicts kept as two drafts with a warning, a draft open while changed elsewhere. Drafts are checked at each pass of Drafts, not at the opening of a draft.
- **Resuming a draft written on the phone**: shown in Drafts, read-only; resuming it means reading its MIME into a `draft` row, with editable recipients.
- **Signatures.**
- **`z` after ⌘⌫ restores the text only**: the app keeps the abandoned body and recreates the draft from the mail answered, so edited recipients, the subject and the attachments are lost. Restoring them means keeping the whole `DraftView` (and the files, which the garbage collector may have taken) or abandoning drafts lazily.
- **Streaming the mail to SMTP and IMAP**: a mail with files is built in memory (100 MB of files at most); writing it to the server from its file would remove that bound.
- **Attachments of a draft written on the phone**: `brouillons.md`, section 7, "Brouillon créé ailleurs".
- **Suggestions from the Mac's Contacts**: `suggest_recipients` knows the addresses of the mailbox only. The app's `AddressBook` can only look cards up by address; a search by name (`CNContact.predicateForContacts(matchingName:)`) would add the people never written to.
- **A reply sent from another account in its conversation**: a reply to a mail of an archive account (or of an account without `[smtp]`) sent from another account is kept in the Sent folder of the account that sent it, so in a conversation of that account; the conversation answered, in the other account, does not show it. Showing it there means a local copy in the archive account (for an IMAP account, a copy in its Sent on the server, or the view of all accounts joining conversations across accounts) (`reponse.md`, section 4 ter).
- **The trial option "leave it where it is"** of `⇧⌘↩` (`reponse.md`, section 8): to keep, drop, or write into `dossiers.md` once tried.
- **Never twice, fully**: a crash between the SMTP server's acceptance and `send_accepted` sends again where the provider does not copy to Sent (`brouillons.md`, "Double envoi"). A test of the restart after `data_sent` against the test servers is missing too.
- **`nicemailer reply`**: a CLI bench of the contract, for agents later.
