# NiceMailer

Mail client for macOS: a Rust core owns the database and the logic, and a SwiftUI app is only one of its clients. The specifications are in `specifications/` (in French).

## Status: first working version

This version is for testing user-experience hypotheses on a database of fake mails, or on a real mailbox imported from an mbox file. It has no IMAP, no SMTP, no LaunchAgent and no code signing.

Documentation of the features is in `docs/`.

```
crates/
  nm-core/     pure domain: folder roles, move targets, preselection after a reply
  nm-store/    SQLite: schema (loaded from specifications/schema.sql), paginated queries, commands,
               raw message files, mbox import, seed
  nm-api/      JSON-RPC contract: methods, views, events, error codes
  nm-daemon/   nicemailer-core binary: JSON-RPC server on a Unix socket
  nm-cli/      nicemailer binary: socket client, seed and import-mbox commands
  nm-mcp/      nicemailer-mcp binary: MCP server for AI agents (docs/mcp.md)
apps/
  NiceMailer/  SwiftUI app (Swift Package)
```

## Running

You need Rust and Xcode: the Command Line Tools alone do not ship the SwiftUI macros. You also need Node (version in `apps/editor/.nvmrc`): the editor of a mail's text is a TypeScript component, bundled into the app by `make editor`, on which every target that builds the app depends ([docs/editor.md](docs/editor.md)).

```sh
make seed     # creates ~/Library/Application Support/NiceMailer/nicemailer.db (≈ 4,500 mails)
make core     # starts the core (leave it running)
make app      # in another terminal
```

`make bundle` installs `NiceMailer.app` in `~/Applications` (`make open` also quits the old one and launches it), needed for the links to a mail (`nicemailer://m/<id>`, to paste in Obsidian): see `docs/deep-links.md`.

`NICEMAILER_DATA_DIR=/other/directory` changes the data directory for the core, the CLI and the app; without it, they use the mailbox chosen in the app (`docs/app-bundle.md`). The socket path must stay under 104 characters.

## Importing a real mailbox

With the core running, an mbox export (HEY, Gmail, Thunderbird…) can be imported into an archive account: sent mails go to Sent, the rest to Archive. See `docs/import-mbox.md`.

```sh
nicemailer import-mbox ~/Downloads/mailbox.mbox --account hey --address me@hey.com
```

## App shortcuts

The app is driven by the keyboard, like an editor: `j`/`k` to move, `d` to archive, `m` + a folder letter to move, `;` + a letter to go to a folder or a screen, `?` for the list of keys. See `docs/geek-ui.md`.

A conversation displayed for one second is marked as read.

## Querying the core

```sh
nicemailer folders
nicemailer call list_threads '{"folder_id": 1, "limit": 5}'
nicemailer call get_thread '{"thread_id": 42}'
nicemailer call search '{"query": "facture"}'
nicemailer call move_thread '{"thread_id": 42, "target": "todo", "from_folder_id": 1}'
nicemailer import-status  # running or last mbox import
nicemailer accounts       # accounts of config.toml and archive accounts
nicemailer config check   # validate config.toml (docs/configuration.md)
nicemailer account password perso   # save an IMAP password in the Keychain
nicemailer imap probe perso     # what the server offers: capabilities, folders, roles (docs/imap.md)
nicemailer account add perso --email me@example.fr --host imap.example.fr   # create an account (docs/accounts.md)
nicemailer sync status          # the synchronization of each IMAP account (docs/sync.md)
nicemailer sync verify perso    # server and database compared, folder by folder; --repair downloads damaged raw files again
nicemailer raw gc         # delete raw files no mail references (the core does it hourly; docs/ingestion.md)
nicemailer watch          # live event stream
```

The protocol is JSON-RPC 2.0, one JSON object per line. A client starts with `hello {"protocol_version": 1}`, then calls `subscribe` if it wants events.

## Third-party code bundled with the app

- **iA Writer Mono V**, the font of the geek theme (SIL Open Font License), in `apps/NiceMailer/Sources/NiceMailer/Resources/Fonts`.
- **Dark Reader** 4.9.134 (MIT License), which adapts mails made for a white page to the dark theme, in `apps/NiceMailer/Sources/NiceMailer/Resources/DarkReader` with its licence (`docs/thread-display.md`, "HTML of the sender").

## License

Copyright (C) 2026 the NiceMailer autor.

NiceMailer is free software: you can redistribute it and/or modify it under the terms of the GNU Affero General Public License, version 3, as published by the Free Software Foundation. It is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See [LICENSE](LICENSE) for the full text.

The third-party code listed above keeps its own licence.
