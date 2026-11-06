# NiceMailer documentation

User and developer documentation, in English. It is written together with the code: a change that adds or modifies a feature updates these pages in the same change. The specifications, in French, are in `specifications/`.

| Page | Content |
| --- | --- |
| [import-mbox.md](import-mbox.md) | Importing a whole mailbox from an mbox file into an archive account |
| [sync.md](sync.md) | The synchronization of the IMAP accounts, server to database: passes, moves made elsewhere, IDLE, failures, `nicemailer sync status | now | verify` |
| [screening.md](screening.md) | New mail sorted at reception: authentication verdict (`Authentication-Results`, Public Suffix List), routing, conversations back from Todo/Waiting, mail found in Sent |
| [spool.md](spool.md) | The changes made in NiceMailer sent to the server: actions, merging, grouping, retries, refusals undone, crash recovery, two Macs on one mailbox |
| [ingestion.md](ingestion.md) | How a message enters the database, from an mbox or IMAP: headers then body, threads, streaming writes of large messages, failures |
| [geek-ui.md](geek-ui.md) | The geek theme of the app: layout, keys, menus and pickers |
| [thread-display.md](thread-display.md) | Simplified display of a thread: hidden quotes, signatures and copies, folded messages, light headers, `T` and `S` |
| [testing.md](testing.md) | How the core and the app are tested: Rust tests, Swift unit tests, scripted UI tests |
| [folder-workflow.md](folder-workflow.md) | Moving mail: the decision on the sender, filing its other mails, authentication, domain decisions, undo |
| [migrations.md](migrations.md) | Upgrading a database: schema migrations at opening, reinterpretation of stored mail after a parser change, and how to add either |
| [configuration.md](configuration.md) | Settings and accounts in `config.toml`: where it is, what is checked, how changes apply while the core runs, passwords in development, the read pool |
| [imap.md](imap.md) | The IMAP client of the core: connection, certificates, login, commands and their fallbacks, large messages, failures, IDLE, `nicemailer imap probe` |
| [deep-links.md](deep-links.md) | The URL of each mail, `nicemailer://m/<id>`: copying it from `l` (Markdown for Obsidian), opening it, `make bundle` |
| [core-library.md](core-library.md) | Core methods behind the screens: sort by modification, text/HTML per sender, export, history, tags, groups, contacts |

Features of the first working version that predate this folder (fake-mail seed, folders and moves, search, events) are described in the main `README.md` and still need their own pages.
