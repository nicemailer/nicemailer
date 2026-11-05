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
apps/
  NiceMailer/  SwiftUI app (Swift Package)
```

## Running

You need Rust and Xcode: the Command Line Tools alone do not ship the SwiftUI macros.

```sh
make seed     # creates ~/Library/Application Support/NiceMailer/nicemailer.db (≈ 4,500 mails)
make core     # starts the core (leave it running)
make app      # in another terminal
```

`NICEMAILER_DATA_DIR=/other/directory` changes the data directory for the core, the CLI and the app. The socket path must stay under 104 characters.

## Importing a real mailbox

With the core running, an mbox export (HEY, Gmail, Thunderbird…) can be imported into an archive account: sent mails go to Sent, the rest to Archive. See `docs/import-mbox.md`.

```sh
nicemailer import-mbox ~/Downloads/mailbox.mbox --account hey --address me@hey.com
```

## App themes and shortcuts

The app has a **Theme** menu. The default **geek** theme is driven by the keyboard, like an editor: `j`/`k` to move, `d` to archive, `m` + a folder letter to move, `;` + a letter to go to a folder or a screen, `?` for the list of keys. See `docs/geek-ui.md`.

In the **classic** theme (the first interface):

| Key | Action |
| --- | --- |
| `j` / `k` | next / previous conversation |
| a folder's letter (`I T W A N P D X B`…) | move the conversation, then go to the next one |
| `u` | read / unread |
| `f` | flag |
| `⌘R` | reload |

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
nicemailer watch          # live event stream
```

The protocol is JSON-RPC 2.0, one JSON object per line. A client starts with `hello {"protocol_version": 1}`, then calls `subscribe` if it wants events.
