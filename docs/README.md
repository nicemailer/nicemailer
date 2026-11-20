# NiceMailer documentation

User and developer documentation, in English. It is written together with the code: a change that adds or modifies a feature updates these pages in the same change. The specifications, in French, are in `specifications/`.

| Page | Content |
| --- | --- |
| [contract.md](contract.md) | Every method of the core contract (JSON-RPC on the socket), what it does, and the CLI command that uses it |
| [import-mbox.md](import-mbox.md) | Importing a whole mailbox from an mbox file into an archive account |
| [accounts.md](accounts.md) | Creating an IMAP account (its servers found from the address, through the database of mail providers and the MX of its domain; the SMTP server tried) with its initial import (archiving on the server, correspondents), folder roles, renaming and removing an account, how `accounts.toml` is written |
| [sync.md](sync.md) | The synchronization of the IMAP accounts, server to database: passes, moves made elsewhere, IDLE, failures, `nicemailer sync status | now | verify` |
| [senders.md](senders.md) | People and robots: the clues read in the headers of each mail, the verdict on an address, the filter human / robot, forcing an address from Contacts, the catch-up of stored mail |
| [screening.md](screening.md) | New mail sorted at reception: authentication verdict (`Authentication-Results`, Public Suffix List), routing, conversations back from Todo/Waiting, mail found in Sent |
| [spool.md](spool.md) | The changes made in NiceMailer sent to the server: actions, merging, grouping, retries, refusals undone, crash recovery, two Macs on one mailbox |
| [ingestion.md](ingestion.md) | How a message enters the database, from an mbox or IMAP: headers then body, threads, streaming writes of large messages, failures |
| [geek-ui.md](geek-ui.md) | The geek theme of the app: layout, keys, menus and pickers |
| [thread-display.md](thread-display.md) | Simplified display of a thread: hidden quotes, signatures and copies, folded messages, light headers, `T` and `S` |
| [localization.md](localization.md) | The app in the user's language: the two locales (the app's language, the Mac's region and clock), dates by template, one key per sentence, plural forms, how to add a string, the tests |
| [testing.md](testing.md) | How the core and the app are tested: Rust tests, Swift unit tests, scripted UI tests |
| [folder-workflow.md](folder-workflow.md) | Moving mail: the decision on the sender, filing its other mails, authentication, domain decisions, undo |
| [backup.md](backup.md) | The two database files (`nicemailer.db`, and `nicemailer_body.db` for the bodies and the search index), what to back up, `nicemailer mailbox backup`, rebuilding the body file |
| [updates.md](updates.md) | Updates by Sparkle: the core in maintenance while the app is replaced or the database upgraded, `prepare_update`, the backup around a migration, what each client does, the keys and the appcast of a release |
| [migrations.md](migrations.md) | Upgrading a database: schema migrations at opening, reinterpretation of stored mail after a parser change, and how to add either |
| [keymap.md](keymap.md) | The keys of the app redefined in `keymap.toml`: writing a key, contexts, actions, menus of your own, errors, `nicemailer keymap check | defaults | show` |
| [configuration.md](configuration.md) | Settings in `config.toml` and accounts in `accounts.toml`: where they are, how they are created at the first start, what is checked, how changes apply while the core runs, passwords in development, the read pool |
| [imap.md](imap.md) | The IMAP client of the core: connection, certificates, login, commands and their fallbacks, large messages, failures, IDLE, `nicemailer imap probe` |
| [deep-links.md](deep-links.md) | The URL of each mail, `nicemailer://m/<id>`: copying it from `l` (Markdown for Obsidian), opening it, `make bundle` |
| [app-bundle.md](app-bundle.md) | `NiceMailer.app` in `/Applications`: its core run by a launch agent, stopping it, choosing the mailbox (⌥ at launch, Change Mailbox…), `make bundle` |
| [editor.md](editor.md) | The editor of a mail's text: a standard Mac text field (Tiptap in a web view), text only or formatted, its keys, pasting, what the core sends, the protocol, building it |
| [sending.md](sending.md) | Replying to a mail: the reply screen, what the core prepares, sending through SMTP with an undo delay, the copy in Sent, the filing, failures, the SMTP password |
| [core-library.md](core-library.md) | Core methods behind the screens: sort by modification, text/HTML per sender, export, history, tags, groups, contacts |
| [mcp.md](mcp.md) | AI agents (Claude Desktop, Claude Code, any MCP client) reading the mailbox and preparing drafts through `nicemailer-mcp`: connecting them from the app, the tools, what an agent may and may not do (enforced by the core), the answers, the tests |
| [ai.md](ai.md) | AI assistance: the providers (the user's `claude` binary, a fake for tests), the safety check that proves at every call that the binary has no tool, the built-in prompts and how to write your own, the tests |

Features of the first working version that predate this folder (fake-mail seed, folders and moves, search, events) still need their own pages; the main `README.md` only gives the commands that run them.
