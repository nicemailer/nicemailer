//! Public contract between the core and its clients (`architecture.md`,
//! section 5).
//!
//! Transport: JSON-RPC 2.0 over a Unix socket, one JSON object per line
//! (newline-delimited JSON, often called NDJSON). A client sends a
//! *request* `{"jsonrpc":"2.0","id":1,"method":"list_folders","params":{}}`
//! and receives a *response* carrying the same `id`. The core can also push
//! *notifications* (requests without an `id`) to announce changes.
//!
//! Responses are display-ready *views*: they do not mirror the SQLite schema,
//! so the schema can evolve without breaking clients.
//!
//! This crate only contains data types and constants. Both the daemon (which
//! serializes them) and the Rust CLI (which deserializes them) depend on it,
//! which guarantees they agree on the format. The Swift app has its own
//! mirror of these types in `Models.swift`.

use std::collections::BTreeMap;

use nm_core::{AfterSend, AuthPolicy, FolderCategory, FolderRole, ScreeningTarget, SenderKind};
use serde::{Deserialize, Serialize};
// `Value` is serde_json's type for "any JSON value" (object, array, string...).
// It is used where the shape is not known in advance, e.g. request params
// before we know which method they belong to.
use serde_json::Value;

/// Protocol version announced in the `hello` handshake. Bump it on any
/// incompatible change so that old clients get a clear error.
pub const PROTOCOL_VERSION: u32 = 1;

/// Default and maximum page size for lists. The maximum protects the core
/// from a client asking for 100 000 rows at once.
pub const DEFAULT_LIMIT: u32 = 50;
pub const MAX_LIMIT: u32 = 200;

/// Method names, gathered in one place to avoid typos.
///
/// Rust note: `pub mod` declares a nested module (a namespace). Constants
/// are then used as `method::HELLO`.
pub mod method {
    /// First call of every client: checks the protocol version, names the client.
    pub const HELLO: &str = "hello";
    /// Answers `"pong"`: checks the core is alive.
    pub const PING: &str = "ping";
    /// Subscribes the connection to the core's events, replaying the missed ones.
    pub const SUBSCRIBE: &str = "subscribe";
    /// Every folder of every account, with its role and counts.
    pub const LIST_FOLDERS: &str = "list_folders";
    /// A page of threads of a folder, of several folders, or of "Everything".
    pub const LIST_THREADS: &str = "list_threads";
    /// A whole thread, ready to display.
    pub const GET_THREAD: &str = "get_thread";
    /// Full-text search from the text typed by the user.
    pub const SEARCH: &str = "search";
    /// Moves a thread to the folder of a role; returns an undo token.
    pub const MOVE_THREAD: &str = "move_thread";
    /// Several threads moved in one request (`MoveThreadsParams`).
    pub const MOVE_THREADS: &str = "move_threads";
    /// Marks one or several threads as read or unread.
    pub const SET_THREAD_SEEN: &str = "set_thread_seen";
    /// Flags or unflags one or several threads.
    pub const SET_THREAD_FLAGGED: &str = "set_thread_flagged";
    /// Starts the import of an mbox file into an archive account; the work
    /// continues in the background and reports through `ImportProgress` and
    /// `ImportFinished` events.
    pub const IMPORT_MBOX: &str = "import_mbox";
    /// State of the running import, or of the last one.
    pub const IMPORT_STATUS: &str = "import_status";
    /// Remembers whether the mails of an address are shown as text or HTML.
    pub const SET_BODY_FORMAT: &str = "set_body_format";
    /// Writes one message to a file (.eml, .txt, .md).
    pub const EXPORT_MESSAGE: &str = "export_message";
    /// The text of one message, each paragraph on one line, for the
    /// clipboard.
    pub const MESSAGE_TEXT: &str = "message_text";
    /// Everything about one message read from its raw file, for printing
    /// it as evidence (the PDF of `sp`).
    pub const MESSAGE_RECORD: &str = "message_record";
    /// The file of an attachment, extracted from the raw message on first use.
    pub const GET_ATTACHMENT: &str = "get_attachment";
    /// The text of an attachment (text, HTML, PDF, DOCX, mail attached),
    /// one page of characters at a time (`mcp.md`, section 8).
    pub const ATTACHMENT_TEXT: &str = "attachment_text";
    /// Unsubscribes from a list: in one click, or gives the page to open.
    pub const UNSUBSCRIBE: &str = "unsubscribe";
    /// Audit log, newest first (`tracabilite.md`).
    pub const LIST_HISTORY: &str = "list_history";
    /// The audit log as the History screen shows it: the lines of one
    /// action on several items make one group (`tracabilite.md`, section 6).
    pub const LIST_HISTORY_GROUPS: &str = "list_history_groups";
    /// Tags of an account, or of every account.
    pub const LIST_TAGS: &str = "list_tags";
    /// Adds (or removes) a tag on every message of a thread.
    pub const TAG_THREAD: &str = "tag_thread";
    /// Contact groups (`groupes.md`).
    pub const LIST_GROUPS: &str = "list_groups";
    /// Adds (or removes) an address, or the person a thread is with, to a
    /// group. The user's own addresses are refused (`groupes.md`).
    pub const SET_GROUP_MEMBER: &str = "set_group_member";
    /// Tags of some threads, the person each is with and that person's
    /// groups: what `t` and `g` can remove from a selection (`ui.md`).
    pub const THREAD_LABELS: &str = "thread_labels";
    /// Renames a tag, in every account that has it; the IMAP keyword
    /// stays (`ui.md`, "Écrans Tags et Groupes").
    pub const RENAME_TAG: &str = "rename_tag";
    /// Deletes a tag: removed from every message, server included, then
    /// kept as a tombstone.
    pub const DELETE_TAG: &str = "delete_tag";
    /// Renames a contact group (`groupes.md`, section 6).
    pub const RENAME_GROUP: &str = "rename_group";
    /// Deletes a contact group; its mails do not change.
    pub const DELETE_GROUP: &str = "delete_group";
    /// Known addresses, with their decision and groups.
    pub const LIST_CONTACTS: &str = "list_contacts";
    /// Takes, changes or removes the screening decision on an address, and
    /// files the mails already received from it (`dossiers.md`).
    pub const SET_ADDRESS_SCREENING: &str = "set_address_screening";
    /// The same for a domain.
    pub const SET_DOMAIN_SCREENING: &str = "set_domain_screening";
    /// Forces an address to be a person or a robot, or gives it back to the
    /// computation (`expediteurs.md`, section 9).
    pub const SET_SENDER_KIND: &str = "set_sender_kind";
    /// Undoes a move or a decision from the token it returned.
    pub const UNDO: &str = "undo";
    /// Every account, IMAP and archive, with its state (`AccountView`).
    pub const LIST_ACCOUNTS: &str = "list_accounts";
    /// State of `config.toml` and `accounts.toml`: paths, error refusing
    /// them, warnings (`ConfigStatus`).
    pub const CONFIG_STATUS: &str = "config_status";
    /// The keys of the app: the defaults with `keymap.toml` applied, by
    /// context and by menu, and why the file is refused (`KeymapView`,
    /// `raccourcis.md`).
    pub const KEYMAP_GET: &str = "keymap_get";
    /// The theme in effect, resolved: `ui.theme` of `config.toml`, its
    /// file of `themes/` and those it extends, every key with its value
    /// (`ThemeView`, `themes.md`).
    pub const THEME_GET: &str = "theme_get";
    /// The themes that can be chosen: the built-in ones and the files of
    /// `themes/`, with their origin, their parent and why a file is refused
    /// (`ThemeListView`).
    pub const THEME_LIST: &str = "theme_list";
    /// Writes `ui.theme` in `config.toml`, the rest of the file kept as it
    /// is (`ConfigSetThemeParams`).
    pub const CONFIG_SET_THEME: &str = "config_set_theme";
    /// Writes `compose.format` in `config.toml`, the rest of the file kept
    /// as it is: the mode ⇧⌘T converted a draft to becomes the mode of the
    /// next mails (`ConfigSetComposeFormatParams`, `edition.md`, section 3).
    pub const CONFIG_SET_COMPOSE_FORMAT: &str = "config_set_compose_format";
    /// Changes the display order of the accounts (account picker, sidebar).
    pub const SET_ACCOUNT_ORDER: &str = "set_account_order";
    /// Saves the IMAP password of an account in the Keychain, for the
    /// server the file names now (`configuration.md`, section 6).
    pub const SET_PASSWORD: &str = "set_password";
    /// Starts an OAuth2 sign-in flow: the core listens on the loopback and
    /// returns the authorization URL, which the client opens in the
    /// browser; the flow ends with the `OAuthFinished` event
    /// (`specifications/oauth2.md`, section 8).
    pub const OAUTH_BEGIN: &str = "oauth_begin";
    /// Abandons an OAuth2 flow: closes the listener and forgets the flow.
    pub const OAUTH_CANCEL: &str = "oauth_cancel";
    /// Where a message is (its thread and folder), from its id or its link
    /// key: opens a deep link `nicemailer://m/<key>` (`docs/deep-links.md`).
    pub const LOCATE_MESSAGE: &str = "locate_message";
    /// Connects to the IMAP server of an account and reports what it
    /// offers, without changing anything (`ProbeAccountParams`,
    /// `ProbeReport`).
    pub const PROBE_ACCOUNT: &str = "probe_account";
    /// State of the synchronization of each IMAP account (`plan-imap.md`,
    /// phase 4).
    pub const SYNC_STATUS: &str = "sync_status";
    /// Synchronizes an account, or every account, now.
    pub const SYNC_NOW: &str = "sync_now";
    /// Compares, folder by folder, the server and the database: UIDs,
    /// flags, raw files of complete messages.
    pub const SYNC_VERIFY: &str = "sync_verify";
    /// Tries the server of an account not created yet: capabilities,
    /// folders with the proposed roles, counts for the import screen
    /// (`import-initial.md`, sections 2 and 3).
    pub const TEST_ACCOUNT: &str = "test_account";
    /// Exact count of the mails the initial archiving would move.
    pub const COUNT_ARCHIVE: &str = "count_archive";
    /// Finds the servers of an address before the account is created: the
    /// provider (by its domain, else by its MX), the IMAP server, the SMTP
    /// servers to try (`docs/accounts.md`, "Finding the servers").
    pub const DISCOVER_ACCOUNT: &str = "discover_account";
    /// Tries SMTP servers in order, until one accepts the login.
    pub const TEST_SMTP: &str = "test_smtp";
    /// Creates an IMAP account: folders, roles, initial import, password,
    /// `accounts.toml`.
    pub const ADD_ACCOUNT: &str = "add_account";
    /// Changes the workflow roles of the folders of an account.
    pub const SET_FOLDER_ROLES: &str = "set_folder_roles";
    /// What `activate_account` would do to a read-only account: the
    /// report of `test_account`, with the password of the Keychain.
    pub const PREPARE_ACTIVATION: &str = "prepare_activation";
    /// Makes a read-only account writable, with the steps of a creation:
    /// folders, roles, boundary, correspondents, archiving
    /// (`docs/accounts.md`, "Looking first").
    pub const ACTIVATE_ACCOUNT: &str = "activate_account";
    /// Renames the key of an account, keeping its data.
    pub const RENAME_ACCOUNT: &str = "rename_account";
    /// Deletes an account, its data and its password.
    pub const REMOVE_ACCOUNT: &str = "remove_account";
    /// Runs the garbage collector of the raw store now.
    pub const RAW_GC: &str = "raw_gc";
    /// What `r`, `R`, `f` or `c` opens, without writing anything
    /// (`reponse.md`, section 5): the draft already started to the mail,
    /// or the one the core proposes (`DraftView.id` 0), which
    /// `create_draft` writes at its first save. Never waits for a write.
    pub const PREPARE_DRAFT: &str = "prepare_draft";
    /// Starts a reply to all to a message, or gives back the reply already
    /// started (`reponse.md`, section 4): recipients, subject and
    /// quotation prepared by the core. Written: the app calls it at the
    /// first save of a draft `prepare_draft` proposed, with the text typed.
    pub const CREATE_DRAFT: &str = "create_draft";
    /// The latest version of a draft, marked open.
    pub const OPEN_DRAFT: &str = "open_draft";
    /// Saves what was typed, at each pause (`reponse.md`, section 5): the
    /// body, and the recipients or the subject when they changed.
    pub const UPDATE_DRAFT: &str = "update_draft";
    /// Attaches a file the app put in `incoming.noindex` (`brouillons.md`,
    /// section 7).
    pub const ADD_DRAFT_ATTACHMENT: &str = "add_draft_attachment";
    /// Removes an attachment from a draft.
    pub const REMOVE_DRAFT_ATTACHMENT: &str = "remove_draft_attachment";
    /// Reads the addresses typed or pasted in a recipient line (`reponse.md`,
    /// section 6): the ones understood, and the pieces that are not one.
    pub const PARSE_ADDRESSES: &str = "parse_addresses";
    /// The closed lists of the formatted mode (fonts, sizes, colours) and
    /// the mode a draft starts in (`edition.md`, sections 3 and 5).
    pub const COMPOSE_FORMATS: &str = "compose_formats";
    /// The plain text of the HTML of a formatted draft, and whether it has
    /// any formatting: what the editor shows when a draft goes back to
    /// text only (`edition.md`, section 3).
    pub const HTML_TO_TEXT: &str = "html_to_text";
    /// The addresses to suggest for the text typed in a recipient line,
    /// those the user writes to most first.
    pub const SUGGEST_RECIPIENTS: &str = "suggest_recipients";
    /// The addresses a mail can be sent from: those of every account that
    /// can send (`[smtp]`, not read-only), for the "From" of any draft
    /// (`reponse.md`, section 4 ter).
    pub const LIST_SENDERS: &str = "list_senders";
    /// The draft window closed.
    pub const CLOSE_DRAFT: &str = "close_draft";
    /// Abandons a draft; returns it, so that the client can restore it.
    pub const DISCARD_DRAFT: &str = "discard_draft";
    /// Queues the draft for sending, after the undo delay (`reponse.md`,
    /// section 7).
    pub const SEND_DRAFT: &str = "send_draft";
    /// Cancels a send during the undo delay: the draft is editable again.
    pub const CANCEL_SEND: &str = "cancel_send";
    /// The AI assistance (`ia.md`, section 10): on or off, the provider and
    /// whether it is ready, the problems of the prompt files.
    pub const AI_STATUS: &str = "ai_status";
    /// The prompts of one screen's AI menu, in a folder.
    pub const AI_LIST_PROMPTS: &str = "ai_list_prompts";
    /// Starts a run; answers at once with its id, the rest comes by events
    /// (`AiDelta`, `AiDone`, `AiFailed`).
    pub const AI_RUN: &str = "ai_run";
    /// Stops a run.
    pub const AI_CANCEL: &str = "ai_cancel";
    /// The state and the proposal of a run.
    pub const AI_GET_RUN: &str = "ai_get_run";
    /// Applies the proposal of a finished run.
    pub const AI_APPLY: &str = "ai_apply";
    /// Turns down the proposal of a run, or stops it if it still runs.
    pub const AI_DISMISS: &str = "ai_dismiss";
    /// The suggested targets waiting for the user, for conversations shown.
    pub const AI_SUGGESTIONS: &str = "ai_suggestions";
    /// Sets one suggested target aside.
    pub const AI_DISMISS_SUGGESTION: &str = "ai_dismiss_suggestion";
    /// The app is about to install an update of NiceMailer
    /// (`mise-a-jour.md`, section 3): the core enters maintenance
    /// (`Maintenance`, reason `update`), stops its IMAP workers, and serves
    /// no client until it is replaced, `cancel_update` is called, or
    /// `core.update_timeout_secs` passes. Reserved to the app.
    pub const PREPARE_UPDATE: &str = "prepare_update";
    /// The update was abandoned: the core leaves the maintenance of
    /// `prepare_update` and starts its workers again. Reserved to the app.
    pub const CANCEL_UPDATE: &str = "cancel_update";
}

/// Stable error codes. Negative codes are the ones defined by the JSON-RPC
/// 2.0 standard; positive codes are specific to NiceMailer. Clients should
/// branch on the code, never on the message text.
pub mod error_code {
    /// The line received is not valid JSON.
    pub const PARSE_ERROR: i64 = -32700;
    /// Valid JSON, but not a valid JSON-RPC request.
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    /// The params do not match what the method expects.
    pub const INVALID_PARAMS: i64 = -32602;
    /// Unexpected failure inside the core (e.g. a SQLite error).
    pub const INTERNAL_ERROR: i64 = -32603;
    /// The client did not call `hello`, or its version is not supported.
    pub const PROTOCOL_VERSION: i64 = 1000;
    /// The requested folder, thread or message does not exist.
    pub const NOT_FOUND: i64 = 1001;
    /// Move refused by the workflow rules (e.g. moving to Quarantine).
    pub const INVALID_MOVE: i64 = 1002;
    /// The core is in maintenance (`mise-a-jour.md`, section 2): an update
    /// being installed, a migration of the database, or a database it
    /// cannot open. It serves no request; `data` is a [`crate::Maintenance`]
    /// that says why. The connection is closed
    /// after this error: try again later, without starting a core.
    pub const MAINTENANCE: i64 = 1003;
    /// An import is already running: only one at a time.
    pub const BUSY: i64 = 1004;
    /// The move would take a favorable decision on a sender whose mail is
    /// suspected of spoofing: send it again with `confirm_suspect` once the
    /// user has confirmed. `data` holds `addr` and `verdict`.
    pub const CONFIRMATION_REQUIRED: i64 = 1005;
    /// The store of secrets (the Keychain) refused to save or give a
    /// password; the message says why.
    pub const SECRETS: i64 = 1006;
    /// The client may not do this: saving a password, for one, is
    /// reserved to the app and the CLI.
    pub const FORBIDDEN: i64 = 1007;
    /// The account cannot log in yet: no password saved, the server changed
    /// since it was saved, OAuth2. The message says what to do.
    pub const ACCOUNT_NOT_READY: i64 = 1008;
    /// The mail server could not be reached, refused the login, or failed;
    /// the message holds its answer. From the account creation methods
    /// (`test_account`, `count_archive`, `add_account`), `data.reason` says
    /// which: `auth` (the user name or the password refused), `connect`
    /// (no connection, TLS refused, no answer in time), `server` (anything
    /// else), so that a client points at the field to correct.
    pub const SERVER: i64 = 1009;
    /// A command that changes the mail of a read-only account
    /// (`read_only = true` in `accounts.toml`).
    pub const READ_ONLY: i64 = 1010;
    /// The raw file of the mail is corrupt or missing: set aside, and the
    /// mail downloaded again from its server; try again later.
    pub const RAW_DAMAGED: i64 = 1011;
    /// `update_draft` or `send_draft` with a revision that is not the
    /// draft's current one: another window or client changed it.
    pub const REVISION_CONFLICT: i64 = 1012;
    /// The draft is being sent and cannot be changed (`cancel_send` first).
    pub const DRAFT_SENDING: i64 = 1013;
    /// `cancel_send` too late: the mail is leaving, or has left.
    pub const SEND_ALREADY_STARTED: i64 = 1014;
    /// The account cannot send: no `[smtp]` in `accounts.toml`, read-only, an
    /// archive account, or a draft without recipient. The message says which.
    pub const CANNOT_SEND: i64 = 1015;
    /// The body of the mail answered is not downloaded yet: it is asked
    /// for first, try again in a moment.
    pub const BODY_PENDING: i64 = 1016;
    /// The AI assistance is off (`ai.enabled`): nothing was sent.
    pub const AI_DISABLED: i64 = 1017;
    /// A run could not start or failed; `data.code` is the AI error code
    /// (`ai_not_logged_in`, `ai_bad_input`, `ai_provider_unsafe`…,
    /// `ia.md`, sections 3 and 10).
    pub const AI: i64 = 1018;
    /// The connection sends requests faster than the core accepts: the
    /// request is refused until the client slows down
    /// (`architecture.md`, section 9, "limite de débit").
    pub const RATE_LIMITED: i64 = 1019;
}

/// Why a request failed, as a stable word and the values of its sentence
/// (`localisation.md`, section 3.1; `docs/contract.md`, "Reasons").
///
/// The core does not know the user's language: it says *why* with a
/// [`Reason`] and its values, in `data` (`{"reason": "tag_exists", "tag":
/// "Urgent"}`), and each client writes the sentence. The CLI, the logs and
/// the AI agents read `message`, the English sentence built here from the
/// reason's template; the app looks the reason up in its String Catalog.
///
/// A value whose key ends with `_text` is written by someone else (a mail
/// server, the system, the AI provider): a client cites it as it is, never
/// translates it. `detail` is a technical precision in English, for the
/// generic reasons (`invalid_params`, `not_found`, `internal`): the app
/// shows a generic sentence and keeps the detail for the logs.
///
/// Rust note: `macro_rules!` writes code from a pattern. Here each line
/// `NAME = "name": "template";` becomes a constant, and all of them a
/// list, `ALL`, so that the tests can check that each reason is used,
/// documented and translated, without a second list to keep in step.
pub mod reason {
    /// One reason: its stable name and its English sentence, whose `{key}`
    /// are filled with the values of the same key.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Reason {
        pub name: &'static str,
        pub english: &'static str,
    }

    impl Reason {
        /// This reason with one value: `reason::TAG_EXISTS.with("tag", name)`.
        pub fn with(self, key: &str, value: impl Into<serde_json::Value>) -> crate::Why {
            crate::Why::from(self).with(key, value)
        }

        /// The keys of the values its sentence takes, in order of appearance.
        pub fn keys(&self) -> Vec<&'static str> {
            let mut out = Vec::new();
            let mut rest = self.english;
            while let Some(start) = rest.find('{') {
                let Some(len) = rest[start..].find('}') else { break };
                let key = &rest[start + 1..start + len];
                if !out.contains(&key) {
                    out.push(key);
                }
                rest = &rest[start + len + 1..];
            }
            out
        }
    }

    /// The reason named `name`, if the contract has one.
    pub fn named(name: &str) -> Option<Reason> {
        ALL.iter().copied().find(|r| r.name == name)
    }

    macro_rules! reasons {
        ($( $(#[$doc:meta])* $id:ident = $name:literal : $english:literal; )*) => {
            $( $(#[$doc])* pub const $id: Reason = Reason { name: $name, english: $english }; )*
            /// Every reason of the contract.
            pub const ALL: &[Reason] = &[ $( $id ),* ];
        };
    }

    reasons! {
        // --- Generic: errors of a client's program, not of the user ---

        /// The line received is not JSON.
        PARSE_ERROR = "parse_error": "not JSON: {detail}";
        /// Valid JSON, but not a request the protocol knows.
        INVALID_REQUEST = "invalid_request": "invalid request: {detail}";
        /// No method of that name.
        METHOD_NOT_FOUND = "method_not_found": "unknown method: {method}";
        /// The params do not match what the method expects.
        INVALID_PARAMS = "invalid_params": "invalid parameters: {detail}";
        /// Unexpected failure inside the core (a SQLite error, a panic).
        INTERNAL = "internal": "internal error: {detail}";
        /// What was asked for does not exist (any more): a stale id.
        NOT_FOUND = "not_found": "not found: {detail}";
        /// The client did not call `hello` first.
        HELLO_EXPECTED = "hello_expected": "the handshake hello is expected first";
        /// The client speaks another version of the protocol.
        PROTOCOL_VERSION = "protocol_version": "protocol {version} is not supported (expected {expected})";

        // --- Who may do what ---

        /// Reserved to the app, the TUI and the CLI: saving a password,
        /// creating an account, sending mail.
        RESERVED_TO_PEOPLE = "reserved_to_people": "only the app, the TUI and the CLI can do this";
        /// Reserved to NiceMailer.app, which installs the updates.
        RESERVED_TO_APP = "reserved_to_app": "{method} is reserved to NiceMailer.app, which installs the updates";
        /// An AI agent may not send mail yet.
        AGENT_CANNOT_SEND = "agent_cannot_send": "AI agents cannot send mail yet: the draft is in Drafts, where the user can review and send it from NiceMailer";
        /// An AI agent may not file mail yet.
        AGENT_CANNOT_FILE = "agent_cannot_file": "AI agents cannot file mail yet (move, mark, flag, tag): ask the user to do it in NiceMailer";
        /// An AI agent may not unsubscribe: it confirms the mail was read.
        AGENT_CANNOT_UNSUBSCRIBE = "agent_cannot_unsubscribe": "unsubscribing sends a request to an address the sender chose, which confirms the mail was read: only the user can do it, in NiceMailer";
        /// An AI agent may not give a password.
        AGENT_CANNOT_SET_PASSWORD = "agent_cannot_set_password": "passwords are given in NiceMailer only";
        /// Any other method an AI agent may not call.
        AGENT_NOT_ALLOWED = "agent_not_allowed": "{method} is not available to AI agents";
        /// An AI agent may not change a draft the user wrote.
        DRAFT_BY_USER = "draft_by_user": "draft {draft} was written by the user: AI agents may read it, not change it; start a new draft instead";
        /// An AI agent may not change the reply the user started.
        REPLY_STARTED_BY_USER = "reply_started_by_user": "the user has already started a reply to this mail (draft {draft}): AI agents may read it, not change it";

        // --- Accounts ---

        /// No account of that key in `accounts.toml`.
        ACCOUNT_UNKNOWN = "account_unknown": "no account {account} in accounts.toml";
        /// In `accounts.toml`, not yet in the database.
        ACCOUNT_NOT_IN_DATABASE = "account_not_in_database": "account {account} is not in the database";
        /// The account is not synchronized (an archive account, a failure).
        ACCOUNT_NOT_SYNCHRONIZED = "account_not_synchronized": "account {account} is not synchronized";
        /// No IMAP account at all.
        NO_IMAP_ACCOUNT = "no_imap_account": "no IMAP account to synchronize";
        /// The key is taken by another account.
        ACCOUNT_EXISTS = "account_exists": "an account {account} exists already";
        /// The key is that of an account of another kind (an archive).
        ACCOUNT_KEY_TAKEN = "account_key_taken": "{account} is an {kind} account: choose another key";
        /// "Organize…" on an account that is not only looked at.
        ACCOUNT_SET_UP = "account_set_up": "the account {account} is not read-only: it is set up already";
        /// A change to the mail of a read-only account.
        ACCOUNT_READ_ONLY = "account_read_only": "this account is read-only (read_only = true in accounts.toml): nothing is changed";
        /// Sending from a read-only account.
        ACCOUNT_READ_ONLY_SEND = "account_read_only_send": "this account is read-only (read_only = true in accounts.toml): it cannot send";

        // --- Themes ---

        /// `config_set_theme` with a name no theme has (`themes.md`).
        THEME_UNKNOWN = "theme_unknown": "no theme {theme}: neither built in nor a file of the folder themes";
        /// `config_set_theme` could not write `config.toml`: the file is
        /// refused as it is, or cannot be written.
        CONFIG_NOT_WRITTEN = "config_not_written": "config.toml was not changed: {detail}";
        /// `config_set_compose_format` with a mode the editor does not have.
        COMPOSE_FORMAT_UNKNOWN = "compose_format_unknown": "no mode {format}: text or html";
        /// An empty password given.
        PASSWORD_EMPTY = "password_empty": "empty password";
        /// An empty SMTP password given.
        SMTP_PASSWORD_EMPTY = "smtp_password_empty": "empty SMTP password";
        /// An SMTP password for an account without SMTP server.
        SMTP_PASSWORD_WITHOUT_SERVER = "smtp_password_without_server": "an SMTP password without an SMTP server";
        /// The account has no `[smtp]`.
        NO_SMTP_SERVER = "no_smtp_server": "account {account} has no [smtp] server in accounts.toml: it cannot send";
        /// A day that is not `YYYY-MM-DD`.
        DATE_INVALID = "date_invalid": "date {date}: YYYY-MM-DD expected";
        /// Archiving asked for an account only looked at.
        READ_ONLY_ARCHIVES_NOTHING = "read_only_archives_nothing": "a read-only account archives nothing on its server";
        /// A folder chosen for a role that the server does not have.
        SERVER_FOLDER_MISSING = "server_folder_missing": "the server has no folder {folder}: ask to create it";

        // --- Passwords (the Keychain) ---

        /// The Keychain refused to save or to give a password.
        KEYCHAIN = "keychain": "the Keychain refused: {system_text}";
        /// No password saved for the account.
        PASSWORD_MISSING = "password_missing": "no password saved for this account: nicemailer account password {account}";
        /// `auth = "oauth2"`, but the IMAP server is not a known OAuth2
        /// server: no token is ever requested or sent to it
        /// (`specifications/oauth2.md`, section 7).
        OAUTH2_UNKNOWN_SERVER = "oauth2_unknown_server": "the IMAP server {host} is not a known OAuth2 server: no token will be sent to it";
        /// `oauth_begin` on an address whose provider does not accept OAuth2.
        OAUTH2_UNSUPPORTED_PROVIDER = "oauth2_unsupported_provider": "the provider of {email} does not accept OAuth2 sign-in";
        /// The user refused the authorization at the provider.
        OAUTH_DENIED = "oauth_denied": "the authorization was refused";
        /// The flow expired: 10 minutes without redirection.
        OAUTH_TIMEOUT = "oauth_timeout": "the authorization took more than 10 minutes";
        /// The client cancelled the flow (`oauth_cancel`).
        OAUTH_CANCELLED = "oauth_cancelled": "the authorization was cancelled";
        /// The redirection's `state` did not match the flow's.
        OAUTH_STATE_MISMATCH = "oauth_state_mismatch": "the authorization response did not match the request";
        /// The account authorized is not the one asked for.
        OAUTH_WRONG_ACCOUNT = "oauth_wrong_account": "the account authorized is {authorized}, not {email}";
        /// The refresh token was refused: the account must be signed in
        /// again (`nicemailer account login`).
        OAUTH_REAUTH_NEEDED = "oauth_reauth_needed": "the account must be signed in again: nicemailer account login {account}";
        /// The organization must approve the app first (administrator
        /// consent, `AADSTS65001`/`AADSTS90094`).
        OAUTH_ADMIN_CONSENT_NEEDED = "oauth_admin_consent_needed": "the organization must approve NiceMailer before it can sign in ({system_text})";
        /// The organization's conditional access blocked the sign-in
        /// (`AADSTS53003`).
        OAUTH_BLOCKED_BY_POLICY = "oauth_blocked_by_policy": "the organization's conditional access blocked the sign-in ({system_text})";
        /// The provider rejected the application id (`invalid_client`,
        /// `unauthorized_client`).
        OAUTH_CLIENT_REJECTED = "oauth_client_rejected": "the provider rejected the application id ({system_text})";
        /// SMTP AUTH is disabled for the mailbox: the account can read but
        /// not send (`SmtpClientAuthenticationDisabled`).
        SMTP_AUTH_DISABLED = "smtp_auth_disabled": "SMTP AUTH is disabled for this mailbox: the account can read but not send";
        /// IMAP is disabled for the mailbox, or it has no Exchange licence.
        IMAP_DISABLED_FOR_MAILBOX = "imap_disabled_for_mailbox": "IMAP is disabled for this mailbox";
        /// The password was saved for another server than the file's.
        PASSWORD_SERVER_CHANGED = "password_server_changed": "the password was saved for {saved_user} on {saved_host}, the file now says {user} on {host}: save the password again to confirm the new server (nicemailer account password {account})";
        /// The SMTP password was saved for another server than the file's.
        SMTP_PASSWORD_SERVER_CHANGED = "smtp_password_server_changed": "the SMTP password was saved for {saved_user} on {saved_host}, the file now says {user} on {host}: save it again to confirm the new server (nicemailer account password {account} --smtp)";
        /// The SMTP server is of another provider: it needs its own password.
        SMTP_PASSWORD_NEEDED = "smtp_password_needed": "the SMTP server {smtp_host} is not of the same provider as the IMAP server {host}: NiceMailer does not send it the IMAP password. Save its own: nicemailer account password {account} --smtp";

        // --- Mail servers ---

        /// The IMAP server refused the user name or the password.
        IMAP_AUTH_REFUSED = "imap_auth_refused": "the server refused the login: {server_text}";
        /// No connection to the IMAP server: refused, TLS, no answer.
        IMAP_UNREACHABLE = "imap_unreachable": "the server could not be reached: {server_text}";
        /// Any other failure of the IMAP server.
        IMAP_FAILED = "imap_failed": "the server failed: {server_text}";

        // --- Replying and sending ---

        /// The mail answered is not in an account of `accounts.toml`.
        MAIL_NOT_IN_ACCOUNT = "mail_not_in_account": "this mail is not in an account of accounts.toml (an archive account?): it cannot be answered from here";
        /// The mail's account cannot send, and no other account can.
        NO_ACCOUNT_CAN_SEND = "no_account_can_send": "no account of accounts.toml can send: add an [smtp] server to one";
        /// The mail is in an archive account.
        ARCHIVE_CANNOT_SEND = "archive_cannot_send": "this mail is in an archive account, which cannot send";
        /// The account has no address to send from.
        NO_SENDER_ADDRESS = "no_sender_address": "the account has no address to send from";
        /// The sender chosen is not an address of the account.
        NOT_OWN_ADDRESS = "not_own_address": "{address} is not an address of this account";
        /// A draft without recipient.
        DRAFT_NO_RECIPIENT = "draft_no_recipient": "this draft has no recipient";
        /// A draft cannot be answered.
        DRAFT_CANNOT_BE_ANSWERED = "draft_cannot_be_answered": "a draft cannot be answered";
        /// A draft cannot be forwarded.
        DRAFT_CANNOT_BE_FORWARDED = "draft_cannot_be_forwarded": "a draft cannot be forwarded";
        /// The formatted text of a draft is over the limit.
        FORMATTED_TEXT_TOO_BIG = "formatted_text_too_big": "the formatted text is too big";
        /// Another window or client changed the draft.
        REVISION_CONFLICT = "revision_conflict": "this draft was changed elsewhere (revision {revision}): reload it";
        /// The draft is being sent.
        DRAFT_SENDING = "draft_sending": "this draft is being sent: cancel the sending first";
        /// `cancel_send` too late.
        SEND_STARTED = "send_started": "too late to cancel: the mail is leaving, or has left";
        /// The draft is being copied to the server.
        DRAFT_COPYING = "draft_copying": "this draft is being copied to the server: try again in a moment";
        /// The body of the mail is not downloaded yet.
        BODY_PENDING = "body_pending": "the body of this mail is not downloaded yet: it is asked for, try again in a moment";
        /// The raw file of the mail is damaged.
        RAW_DAMAGED = "raw_damaged": "the raw file of this mail is damaged ({hash}): it is set aside and downloaded again from the server if possible";

        // --- Changes sent to the server, undone (`ActionReverted`, `SendFailed`) ---

        /// The server refused a change made in NiceMailer, for good.
        SERVER_REFUSED_CHANGE = "server_refused_change": "the server refused the change: {server_text}";
        /// A change or a mail could not reach the server after its attempts.
        ATTEMPTS_FAILED = "attempts_failed": "{attempts} attempts failed, the last: {server_text}";
        /// The mail server refused the mail, for good.
        SMTP_REFUSED = "smtp_refused": "the mail server refused the mail: {server_text}";

        // --- Moving mail ---

        /// The target is not offered from this folder.
        MOVE_REFUSED = "move_refused": "{target} is not a target from {from}";
        /// A favorable decision on a sender suspected of spoofing.
        CONFIRMATION_REQUIRED = "confirmation_required": "confirmation required: probable spoofing of {addr} (verdict {verdict})";

        // --- Tags, groups, addresses ---

        /// A tag name empty or over 64 characters.
        TAG_NAME_INVALID = "tag_name_invalid": "a tag name has 1 to 64 characters";
        /// Another tag has that name.
        TAG_EXISTS = "tag_exists": "the tag {tag} exists already";
        /// A group name empty or over 64 characters.
        GROUP_NAME_INVALID = "group_name_invalid": "a group name has 1 to 64 characters";
        /// Another group has that name.
        GROUP_EXISTS = "group_exists": "the group {group} exists already";
        /// Not a mail address.
        ADDRESS_INVALID = "address_invalid": "not an address: {address}";
        /// Not a domain.
        DOMAIN_INVALID = "domain_invalid": "not a domain: {domain}";

        // --- Files and imports ---

        /// An import is running: one at a time.
        IMPORT_RUNNING = "import_running": "an import is already running";
        /// A path that is not absolute.
        PATH_NOT_ABSOLUTE = "path_not_absolute": "an absolute path is expected: {path}";
        /// A path that is not a file.
        NOT_A_FILE = "not_a_file": "{path} is not a file";
        /// A folder that does not exist.
        FOLDER_MISSING = "folder_missing": "no folder {path}";
        /// A file the core cannot read.
        FILE_UNREADABLE = "file_unreadable": "{path}: {system_text}";
        /// The database already holds mail.
        DATABASE_NOT_EMPTY = "database_not_empty": "the database already holds mail";

        // --- The AI (`ia.md`); `data.code` keeps the AI error code ---

        /// The AI assistance is off.
        AI_DISABLED = "ai_disabled": "the AI assistance is off: set ai.enabled = true in config.toml";
        /// No `claude` binary.
        AI_PROVIDER_MISSING = "ai_provider_missing": "the claude binary was not found; install Claude Code or set ai.claude_path";
        /// Claude Code is not signed in.
        AI_NOT_LOGGED_IN = "ai_not_logged_in": "Claude Code is not signed in; run `claude` in a terminal to sign in";
        /// The provider announced tools: nothing was sent.
        AI_PROVIDER_UNSAFE = "ai_provider_unsafe": "the claude binary announced tools ({tools}) and MCP servers ({mcp_servers}); nothing was sent to it";
        /// This provider is not supported yet.
        AI_PROVIDER_UNSUPPORTED = "ai_provider_unsupported": "the {provider} provider is not available yet: set ai.provider = \"claude-code\" in config.toml";
        /// The usage limit of the subscription is reached.
        AI_RATE_LIMITED = "ai_rate_limited": "the usage limit of the Claude subscription is reached";
        /// No answer in time.
        AI_TIMEOUT = "ai_timeout": "the AI did not answer in time";
        /// The answer could not be read.
        AI_INVALID_OUTPUT = "ai_invalid_output": "the AI's answer could not be read: {detail}";
        /// The provider failed, with its own text.
        AI_FAILED = "ai_failed": "the AI failed: {provider_text}";
        /// The process could not be started or talked to.
        AI_IO = "ai_io": "cannot run the claude binary: {system_text}";
        /// The prompt is not available in this version.
        AI_PROMPT_UNAVAILABLE = "ai_prompt_unavailable": "the prompt {prompt} is not available in this version of NiceMailer yet";
        /// The prompt is not offered in this folder.
        AI_PROMPT_NOT_HERE = "ai_prompt_not_here": "the prompt {prompt} is not offered in this folder";
        /// A prompt of a reply (`lang = "replied"`: translate it into the
        /// language of the mail received) on a draft that replies to no
        /// mail, or to one whose text is not there.
        AI_REPLY_NEEDED = "ai_reply_needed": "this prompt works on a reply: the draft replies to no mail whose text is here";
        /// A prompt on a selection, without a selection.
        AI_SELECTION_NEEDED = "ai_selection_needed": "select the text to work on first";
        /// A prompt on the whole text, with a selection.
        AI_SELECTION_UNWANTED = "ai_selection_unwanted": "this prompt works on the whole text: send no selection";
        /// The selection is empty.
        AI_SELECTION_EMPTY = "ai_selection_empty": "the selection is empty";
        /// The user's text to send is empty.
        AI_TEXT_EMPTY = "ai_text_empty": "there is no text of yours to send: write some first";
        /// The user's text holds the tag the AI gives the text back in.
        AI_SEGMENT_TAG = "ai_segment_tag": "the text contains a <segment> tag, which the AI uses to give the text back; remove it first";
        /// The mail is still downloading.
        AI_MAIL_DOWNLOADING = "ai_mail_downloading": "this mail is still downloading: try again in a moment";
        /// Sorting a folder that has no role.
        AI_FOLDER_WITHOUT_ROLE = "ai_folder_without_role": "this folder has no role: nothing to sort it into";
        /// Sorting an empty folder.
        AI_FOLDER_EMPTY = "ai_folder_empty": "this folder is empty: nothing to sort";
        /// Sorting a selection, without any.
        AI_MARK_TO_SORT = "ai_mark_to_sort": "mark the conversations to sort, or put the cursor on one";
        /// The draft sent is not the saved one.
        AI_DRAFT_UNSAVED = "ai_draft_unsaved": "the draft changed since: save it and run again";
        /// The run was cancelled (`AiFailed`, code `ai_cancelled`).
        AI_CANCELLED = "ai_cancelled": "cancelled";
        /// The draft changed since the AI read it.
        AI_DRAFT_CHANGED = "ai_draft_changed": "the draft changed since the AI read it: run it again";
        /// An attachment of a kind with no text to read.
        FILE_UNSUPPORTED = "file_unsupported": "{file}: this kind of file has no text the AI can read";
        /// An attachment over the size read.
        FILE_TOO_LARGE = "file_too_large": "{file}: the file is too large to read ({size_mb} MB, 50 MB at most)";
        /// An attachment protected by a password.
        FILE_ENCRYPTED = "file_encrypted": "{file}: the file is protected by a password";
        /// An attachment without text.
        FILE_WITHOUT_TEXT = "file_without_text": "{file}: no text in this file: a scan or an image?";
        /// An attachment that cannot be read.
        FILE_DAMAGED = "file_damaged": "{file}: the file cannot be read: {detail}";

        // --- Maintenance (`mise-a-jour.md`); `data` is a `Maintenance` ---

        /// An update is being installed.
        UPDATE = "update": "NiceMailer is being updated";
        /// The database is being upgraded.
        MIGRATION = "migration": "the database is being upgraded (version {from} to {to})";
        /// The database was written by a newer NiceMailer.
        NEWER_SCHEMA = "newer_schema": "the database was written by a newer NiceMailer (version {from}, this core reads {to})";
        /// The upgrade of the database failed.
        MIGRATION_FAILED = "migration_failed": "the upgrade of the database failed: {message}";

        // --- Transport limits (`architecture.md`, section 9) ---

        /// The connection sends requests faster than the core accepts: it
        /// is refused until it slows down.
        RATE_LIMITED = "rate_limited": "too many requests: slow down";
    }
}

/// A reason with the values of its sentence: what an error says, before it
/// has a code (`RpcError::new`). Other crates build one (the Keychain's
/// refusals, the AI's), so that the reason travels with the error up to
/// the contract.
#[derive(Debug, Clone, PartialEq)]
pub struct Why {
    pub reason: reason::Reason,
    pub values: serde_json::Map<String, Value>,
}

impl From<reason::Reason> for Why {
    fn from(reason: reason::Reason) -> Self {
        Why { reason, values: serde_json::Map::new() }
    }
}

impl Why {
    /// The same, with one more value.
    pub fn with(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.values.insert(key.to_string(), value.into());
        self
    }

    /// The English sentence: the template, each `{key}` replaced by its
    /// value (a list joined by ", "). A key without value stays as it is,
    /// which the tests of each reason catch.
    pub fn message(&self) -> String {
        let mut out = self.reason.english.to_string();
        for (key, value) in &self.values {
            let text = match value {
                Value::String(s) => s.clone(),
                Value::Array(items) => {
                    items.iter().map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())).collect::<Vec<_>>().join(", ")
                }
                other => other.to_string(),
            };
            out = out.replace(&format!("{{{key}}}"), &text);
        }
        out
    }

    /// The `data` of the error: the reason and its values.
    pub fn data(&self) -> Value {
        let mut data = self.values.clone();
        data.insert("reason".into(), Value::String(self.reason.name.into()));
        Value::Object(data)
    }
}

impl std::fmt::Display for Why {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

// ---------------------------------------------------------------------------
// JSON-RPC envelopes
// ---------------------------------------------------------------------------

/// A request sent by a client.
///
/// Rust note: a `struct` groups named fields. Serde turns it into a JSON
/// object whose keys are the field names.
/// - `#[serde(default)]` means "if the key is missing, use the type's default
///   value" (`None` for an `Option`, `Value::Null` for a `Value`).
/// - `skip_serializing_if = "Option::is_none"` omits the key entirely from
///   the JSON output when the value is `None`, instead of writing `null`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    /// Always `"2.0"`.
    pub jsonrpc: String,
    /// Chosen by the client and echoed in the response. A request without
    /// an `id` is a notification and gets no response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

/// The `error` member of a failed response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    /// The family of the error (`error_code`).
    pub code: i64,
    /// The English sentence of its reason, for the CLI, the logs and the
    /// AI agents; a client in another language writes its own from `data`.
    pub message: String,
    /// The reason and its values (`reason`, `Why::data`), and for some
    /// codes more (`Maintenance`, the AI's `code`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl RpcError {
    /// An error of the family `code`, for the reason `why`: its English
    /// sentence as `message`, the reason and its values as `data`.
    ///
    /// Rust note: `impl Into<Why>` accepts a `Why` or a bare `Reason`
    /// (`From<Reason> for Why`), so that a reason without values needs no
    /// conversion at the call.
    pub fn new(code: i64, why: impl Into<Why>) -> Self {
        let why = why.into();
        RpcError { code, message: why.message(), data: Some(why.data()) }
    }

    /// `INVALID_PARAMS`, for a mistake of the client's program: `detail`
    /// says which, in English.
    pub fn invalid(detail: impl std::fmt::Display) -> Self {
        RpcError::new(error_code::INVALID_PARAMS, reason::INVALID_PARAMS.with("detail", detail.to_string()))
    }

    /// `INTERNAL_ERROR`, for a failure inside the core.
    pub fn internal(detail: impl std::fmt::Display) -> Self {
        RpcError::new(error_code::INTERNAL_ERROR, reason::INTERNAL.with("detail", detail.to_string()))
    }

    /// `NOT_FOUND`, for an id that names nothing (any more).
    pub fn not_found(detail: impl std::fmt::Display) -> Self {
        RpcError::new(error_code::NOT_FOUND, reason::NOT_FOUND.with("detail", detail.to_string()))
    }

    /// The name of its reason (`data.reason`).
    pub fn reason(&self) -> Option<&str> {
        self.data.as_ref()?.get("reason")?.as_str()
    }

    /// Adds a value to `data`, outside the reason's sentence (the AI's
    /// `code`, the revision of a draft).
    pub fn with_data(mut self, key: &str, value: impl Into<Value>) -> Self {
        if let Some(Value::Object(data)) = self.data.as_mut() {
            data.insert(key.to_string(), value.into());
        } else {
            let mut data = serde_json::Map::new();
            data.insert(key.to_string(), value.into());
            self.data = Some(Value::Object(data));
        }
        self
    }
}

/// A response sent by the core. Exactly one of `result` and `error` is set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl Response {
    /// Successful response.
    pub fn ok(id: Value, result: Value) -> Self {
        Response { jsonrpc: "2.0".into(), id, result: Some(result), error: None }
    }

    /// Failed response.
    pub fn err(id: Value, error: RpcError) -> Self {
        Response { jsonrpc: "2.0".into(), id, result: None, error: Some(error) }
    }
}

/// JSON-RPC notification: an event pushed by the core, without `id`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub jsonrpc: String,
    pub method: String,
    pub params: Value,
}

// ---------------------------------------------------------------------------
// Events (`evenements.md`)
// ---------------------------------------------------------------------------

/// Position in the event stream.
///
/// `epoch` is a random identifier chosen each time the core starts; `seq` is
/// a counter incremented for each event. A client that remembers the last
/// position it saw can detect that it missed events (gap in `seq`) or that
/// the core restarted (different `epoch`), and then reload everything.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    pub epoch: String,
    pub seq: u64,
}

/// Sequenced events. Events only say *what* changed, not the new content:
/// the client reloads what it is displaying. This keeps events small and
/// avoids sending data the client does not need.
///
/// `epoch` and `seq` are added to the params when the event is sent (see
/// `nm-daemon/src/events.rs`).
///
/// Rust note: in Rust an enum variant can carry data, like a small struct.
/// `#[serde(tag = "method", content = "params")]` serializes
/// `Event::FolderChanged { folder_id: 3 }` as
/// `{"method": "FolderChanged", "params": {"folder_id": 3}}`, which is
/// exactly the shape of a JSON-RPC notification.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params")]
pub enum Event {
    /// The counters or the content of a folder changed. With
    /// `threads_announced`, every conversation that changed in it has its
    /// own `ThreadChanged` in the same batch, before this event: a client
    /// may reload those conversations alone rather than the whole list
    /// (`evenements.md`). Without it (the default, left out of the JSON),
    /// the client reloads the list.
    FolderChanged {
        folder_id: i64,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        threads_announced: bool,
    },
    /// A thread changed; `folder_ids` lists the folders it was in before
    /// and after the change, so a client knows which lists to refresh.
    ThreadChanged { thread_id: i64, folder_ids: Vec<i64> },
    /// A single message changed (flags, folder...).
    MessageChanged { message_id: i64, thread_id: Option<i64> },
    /// The client missed events and must reload everything it displays.
    ResyncRequired {},
    /// An mbox import progressed (one event per batch of messages). The
    /// folders it fills also get `FolderChanged` events.
    ImportProgress(ImportStatus),
    /// An mbox import ended, successfully (`error` is `None`) or not.
    ImportFinished(ImportStatus),
    /// An account was created, enabled again, disabled, or its settings or
    /// state changed: reload it with `list_accounts`.
    AccountChanged { account_id: i64 },
    /// An account cannot connect or authenticate; `why` says why
    /// (`docs/contract.md`, "Errors"), `error` in English. The state stays
    /// available in `list_accounts` until it is resolved, which is
    /// announced by `AccountChanged`.
    AccountError { account_id: i64, error: String, why: Value },
    /// An OAuth2 sign-in flow ended: `ok` when the tokens were obtained.
    /// `why` says why when it failed (`specifications/oauth2.md`,
    /// section 8). The client does not poll while the user is in the
    /// browser.
    OAuthFinished {
        flow: String,
        ok: bool,
        #[serde(default)]
        account: Option<String>,
        email: String,
        #[serde(default)]
        why: Option<Value>,
    },
    /// `config.toml` and `accounts.toml` were applied with these changed
    /// keys (dotted, sorted; those of the accounts start with `accounts.`).
    /// Empty `keys` after a `ConfigError`: the files are valid again,
    /// without any change to the settings applied.
    ConfigChanged { keys: Vec<String> },
    /// A new `keymap.toml` was applied, or the file is valid again after a
    /// `ConfigError` about it: read it again with `keymap_get`. A refused
    /// file gives `ConfigError` with `file` `keymap.toml`, and the previous
    /// keys stay (`raccourcis.md`, section 3).
    KeymapChanged {},
    /// The theme in effect changed: another `ui.theme`, a file of its
    /// chain of inheritance changed, refused or valid again. Read it again
    /// with `theme_get`. A refused theme file also gives `ConfigError` with
    /// `file` `themes/<name>.toml` (`themes.md`, section 3).
    ThemeChanged {},
    /// `config.toml` or `accounts.toml` is invalid and both were refused
    /// whole: the previous settings stay applied (`configuration.md`,
    /// section 4). `file` names the file at fault (`config.toml` or
    /// `accounts.toml`); `line` (from 1) and `key` locate the problem when
    /// known.
    ConfigError {
        #[serde(default)]
        file: String,
        line: Option<u32>,
        key: Option<String>,
        message: String,
    },
    /// New mail, found by the synchronization in `folder_id`; what the app
    /// turns into a notification. Not sent for the first synchronization
    /// of a folder, whose every mail would be "new".
    MessageArrived { message_id: i64, folder_id: i64 },
    /// Progress of a synchronization: `done` of `total` (messages whose
    /// headers or bodies are fetched). Transient: without sequence number,
    /// never replayed (`evenements.md`), at most a few per second.
    SyncProgress { account_id: i64, folder_id: Option<i64>, stage: String, done: u64, total: u64 },
    /// The server refused for good a change made in NiceMailer (`kind`:
    /// `move`, `set_flags`): it was undone locally, with the later changes
    /// of the same message (`modele de donnees.md`, section 6). `why` says
    /// why, the server's answer in `server_text`; `message` in English.
    ActionReverted { message_id: i64, kind: String, message: String, why: Value },
    /// A draft was sent: the server accepted it (`reponse.md`, section 7).
    /// `message_id` is the mail in Sent; `moved_to` the folder its
    /// conversation was filed in (absent: it stayed); `undo` cancels that
    /// filing and the decisions taken on the recipients, not the sending.
    ///
    /// `filing` is the folder a **new mail** goes to, chosen with ⇧⌘↩
    /// (section 8, "Nouveau mail"): decided, but applied once the copy in
    /// Sent is made, and not covered by `undo`. A client may say so at
    /// once ("Sent · filed in Waiting"); the move itself arrives as the
    /// usual `MessagesChanged` and `FolderChanged`.
    DraftSent {
        draft_id: i64,
        message_id: i64,
        thread_id: Option<i64>,
        moved_to: Option<FolderRole>,
        filing: Option<FolderRole>,
        undo: Option<String>,
    },
    /// A draft could not be sent, for good: it is editable again, and
    /// `reply_to` is the mail it answers (the next `r` on it reopens it).
    /// `why` says why, `message` in English.
    SendFailed { draft_id: i64, reply_to: Option<i64>, message: String, why: Value },
    /// A draft was sent, but the server refused some recipients (each with
    /// its answer); the others received it.
    SendPartial { draft_id: i64, refused: Vec<String> },
    /// A fragment of the text of a run, to show as it comes. Transient:
    /// without sequence number, never replayed; `ai_get_run` gives the
    /// whole answer once it is done.
    AiDelta { run_id: i64, text: String },
    /// A run ended with an answer, checked: `ai_get_run` gives its proposal.
    AiDone { run_id: i64 },
    /// A run failed or was cancelled (`code` `ai_cancelled`); nothing to
    /// apply. `code` is the AI error code, `why` its reason (`ai_cancelled`
    /// for a cancel), `message` the English sentence.
    AiFailed { run_id: i64, code: String, message: String, why: Value },
    /// A run on several conversations went through `done` of `total`.
    /// Transient: `ai_get_run` says where a finished one ended.
    AiProgress { run_id: i64, done: u64, total: u64 },
    /// A batch of a run gave targets for these conversations: reload their
    /// suggestions (`ai_suggestions`).
    AiSuggestions { run_id: i64, thread_ids: Vec<i64> },
}

// ---------------------------------------------------------------------------
// Parameters and results of each method
// ---------------------------------------------------------------------------

/// Params of `hello`, the first call any client must make.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelloParams {
    pub protocol_version: u32,
    /// Client name. Informative only for now: the core uses it to fill the
    /// `modified_by` column, but trusts it blindly. The real identity will
    /// later come from checking the client's code signature.
    #[serde(default)]
    pub client: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelloResult {
    pub protocol_version: u32,
    pub core_version: String,
}

/// Params of `subscribe`. `since` is the last position the client saw; when
/// given, the core replays the events the client missed (if it still has
/// them in memory).
///
/// `Default` is derived so that missing params mean "no `since`".
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SubscribeParams {
    #[serde(default)]
    pub since: Option<Position>,
}

/// Result of `subscribe`: the current position, and whether missed events
/// were replayed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscribeResult {
    pub epoch: String,
    pub seq: u64,
    pub replayed: bool,
}

/// Params of `list_threads`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListThreadsParams {
    /// The folder to list…
    #[serde(default)]
    pub folder_id: Option<i64>,
    /// …or several folders listed together (the unified view lists the
    /// folders of the same role in every account)…
    #[serde(default)]
    pub folder_ids: Option<Vec<i64>>,
    /// …or the virtual folder "Everything": every folder of one account, or
    /// of every account. Exactly one of the three. Faster than listing every
    /// folder in `folder_ids`: the core reads whole threads instead of
    /// merging folders.
    #[serde(default)]
    pub everything: Option<EverythingScope>,
    /// Opaque cursor returned as `next_cursor` by the previous page. Absent
    /// for the first page. Clients must not build or parse it. A cursor is
    /// only valid with the `sort` it was produced with.
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    /// Sort key, newest first. Default: reception date.
    #[serde(default)]
    pub sort: ThreadSort,
    /// Attributes narrowing the list (`ui.md`, attribute selector).
    #[serde(default)]
    pub filter: ThreadFilter,
}

/// Scope of the virtual folder "Everything" in `list_threads`:
/// `{"everything": {}}` for every account, `{"everything": {"account_id": 3}}`
/// for one. With `"all_mail": true`, the virtual folder "All Mail": the
/// same without Deleted, Junk and Banned (`FolderRole::OUT_OF_ALL_MAIL`).
/// With `"work": true`, the virtual folder "Work": the conversations of
/// Inbox, Todo and Waiting together (`FolderRole::WORK`). At most one of
/// the two.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EverythingScope {
    #[serde(default)]
    pub account_id: Option<i64>,
    #[serde(default)]
    pub all_mail: bool,
    #[serde(default)]
    pub work: bool,
}

/// Attributes that narrow a thread list, combined with AND. A thread
/// qualifies when its messages in the listed folder match (flag and
/// "answered": any message of the thread, as the list shows them).
/// Empty by default: no filtering.
///
/// Unknown fields are refused rather than ignored: an app newer than the
/// running core would send a criterion this core does not know (`subject`
/// to a core built before it), and ignoring it would return the whole list
/// as if it were the answer. Refused, the request fails and the app shows
/// the error. An older app still works with a newer core: every field is
/// optional.
///
/// Rust note: `deny_unknown_fields` makes serde fail on a JSON key that
/// matches no field, where by default it skips it silently.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThreadFilter {
    /// Read (true: no unread message) or unread (false: at least one).
    #[serde(default)]
    pub seen: Option<bool>,
    /// Flagged or not.
    #[serde(default)]
    pub flagged: Option<bool>,
    /// With or without attachments.
    #[serde(default)]
    pub attachments: Option<bool>,
    /// The user replied (a message is marked answered) or not.
    #[serde(default)]
    pub answered: Option<bool>,
    /// A conversation (several messages) or a single mail.
    #[serde(default)]
    pub conversation: Option<bool>,
    /// From one of these, OR-ed. A value is part of the address or of the
    /// name, or, when it starts with `@`, a domain: `@acme.fr` matches
    /// `jean@acme.fr` and `x@mail.acme.fr`, not `x@notacme.fr` (`ui.md`,
    /// "Négation").
    #[serde(default)]
    pub from: Vec<String>,
    /// Not from any of these (same values as `from`): a thread is left out
    /// as soon as one of its messages in the folder is from one of them.
    #[serde(default)]
    pub not_from: Vec<String>,
    /// To or Cc one of these (same values as `from`), OR-ed.
    #[serde(default)]
    pub to: Vec<String>,
    /// To or Cc none of these: a thread is left out as soon as one of its
    /// messages in the folder is to one of them.
    #[serde(default)]
    pub not_to: Vec<String>,
    /// Words anywhere: subject, sender, recipients or body. The widest
    /// search, the one of `/` (full text, every word as a prefix, like
    /// `search`). In this field and the two below, a word written `-word`
    /// leaves out the threads that contain it.
    #[serde(default)]
    pub contains: Option<String>,
    /// Words of the subject or the body, never the addresses: searching a
    /// name in the text does not bring back every mail of that person.
    #[serde(default)]
    pub text: Option<String>,
    /// Words of the subject only.
    #[serde(default)]
    pub subject: Option<String>,
    /// Conversation date (`ui.md`, "Deux critères de date"): the date of
    /// the thread, that of its latest mail (outside Deleted, Junk and
    /// Banned, as shown in the list), at or after this Unix time. Costs
    /// nothing: the list is sorted on that date.
    #[serde(default)]
    pub after: Option<i64>,
    /// Conversation date: the latest mail before this Unix time.
    #[serde(default)]
    pub before: Option<i64>,
    /// Mail date: a mail of the thread in the listed folders was received
    /// at or after this Unix time. A thread from 2016 to 2026 matches
    /// "before 2020" by its mails of 2016. Heavier than `after` (a set of
    /// mails computed per request), hence a criterion of its own.
    #[serde(default)]
    pub mail_after: Option<i64>,
    /// Mail date: a mail of the thread in the listed folders was received
    /// before this Unix time.
    #[serde(default)]
    pub mail_before: Option<i64>,
    /// Tagged with this tag: a message of the thread carries it. A name,
    /// not an id: the tags of the same name in every account are one (each
    /// account has its own "Clients"), compared without case.
    #[serde(default)]
    pub tag: Option<String>,
    /// Tagged with one of these tags, OR-ed: the Tag… attribute of the
    /// selector (`ui.md`, "Sélecteur d'attributs"). Each name is read like
    /// `tag` (every account, without case); `tag` (a tag's own list) and
    /// these combine by "and". A name no tag has matches nothing. Empty:
    /// no criterion.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Tagged with none of these: a thread is left out as soon as one of
    /// its messages carries one of them (the rule of `not_from`, `ui.md`,
    /// "Négation"). A name no tag has leaves nothing out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub not_tags: Vec<String>,
    /// Without any tag: no message of the thread carries one ("without
    /// tag" of Tag…). `false`, the default, is no criterion.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_tag: bool,
    /// Of the members of this group (`groupes.md`): a message of the thread
    /// in the listed folders is from one of them or to them (To, Cc).
    #[serde(default)]
    pub group_id: Option<i64>,
    /// Of the members of one of these groups, OR-ed: the Group… attribute
    /// of the selector (`ui.md`, "Sélecteur d'attributs"), and the groups
    /// clicked in the folder column. Each group is read like `group_id`;
    /// `group_id` (a group's own list) and these combine by "and": the
    /// list of VIP narrowed to the members of Family. Empty: no criterion.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub group_ids: Vec<i64>,
    /// Of the members of none of these groups: a thread is left out as soon
    /// as one of its messages in the listed folders is from a member or to
    /// one (To, Cc), the rule of `not_from` (`ui.md`, "Négation").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub not_group_ids: Vec<i64>,
    /// Of nobody in a group: no message of the thread in the listed folders
    /// is from a member of any group or to one ("without group" of
    /// Group…), the rule of `not_group_ids` for every group. `false`, the
    /// default, is no criterion.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_group: bool,
    /// Of this contact (an `address` id): a message of the thread in the
    /// listed folders is from it or to it (To, Cc).
    #[serde(default)]
    pub address_id: Option<i64>,
    /// Human (true: a mail of the thread, in any folder, was written by a
    /// person, the user's own replies included) or robot (false: none
    /// was), `expediteurs.md`, section 8. The two values split the list,
    /// with no overlap and no gap.
    #[serde(default)]
    pub human: Option<bool>,
    /// Only these conversations, among those the list holds: a client
    /// that knows which conversations changed (`FolderChanged` with
    /// `threads_announced`) reloads them alone, with the scope and the
    /// other criteria of its list, rather than the whole list. A
    /// conversation that no longer belongs to the list is not returned.
    /// Empty: no such criterion.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub thread_ids: Vec<i64>,
}

impl ThreadFilter {
    pub fn is_empty(&self) -> bool {
        *self == ThreadFilter::default()
    }
}

/// Order of a thread list (`ui.md`, display menu `vc` / `vm`).
///
/// Rust note: `#[serde(rename_all = "lowercase")]` maps the variants to
/// the JSON strings `"received"` and `"modified"`; `#[default]` marks the
/// value used by `Default::default()`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThreadSort {
    /// Latest message received in the folder.
    #[default]
    Received,
    /// Latest change by the user (move, flag, read…) to a message of the
    /// thread in the folder: "what did I just do".
    Modified,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetThreadParams {
    pub thread_id: i64,
}

/// Params of `thread_labels`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadLabelsParams {
    /// The threads, at most `MAX_LABEL_THREADS`.
    pub thread_ids: Vec<i64>,
}

/// Most threads one `thread_labels` reads: a selection made with ⌘A on a
/// long list stays answered in a few milliseconds (three indexed reads per
/// thread), and a client never waits on an unbounded request.
pub const MAX_LABEL_THREADS: usize = 1000;

/// The tags of a thread, the person it is with and that person's groups
/// (`thread_labels`, and the same fields in `ThreadView`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadLabels {
    pub thread_id: i64,
    /// Names of the thread's tags, sorted.
    pub tags: Vec<String>,
    /// Address of the person the thread is with (the one `set_group_member`
    /// takes for `thread_id`), lowercase; `None` when it names nobody but
    /// the user.
    pub correspondent: Option<String>,
    /// Groups that person is in, in display order.
    pub groups: Vec<String>,
}

/// Params of `locate_message`: exactly one of the two. `message_id` names
/// a row of this database (an old link `nicemailer://m/48213`, the History
/// screen, the MCP tools); `link_key` names a mail wherever it is stored
/// (`nicemailer://m/3f9a0c2b7e41`, [`deep_link`]).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LocateMessageParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<i64>,
    /// The 12 hexadecimal characters of the link, in any case.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_key: Option<String>,
}

/// Deep links to a mail, `nicemailer://m/<key>` (`docs/deep-links.md`;
/// `modele de donnees.md`, `link_key`). The core writes them (the AI's
/// calendar files and contact cards, [`MessageView::link`]); the app reads
/// them back. The key itself is computed by the store (`nm_store::link_key`).
pub mod deep_link {
    /// Number of hexadecimal characters of a key: 48 bits.
    pub const KEY_LEN: usize = 12;
    /// Longest message id an old link may carry. Ids stay far below 10¹¹,
    /// so a link of 12 characters is always a key, even one made of digits
    /// only.
    pub const MAX_ID_LEN: usize = 11;

    /// A key as the link writes it: 12 lowercase hexadecimal characters,
    /// zeros in front kept (a fixed length is what makes it a key).
    pub fn key_text(key: i64) -> String {
        format!("{key:012x}")
    }

    /// The key of 12 hexadecimal characters, in any case, or `None` for
    /// anything else (another length, a sign, another letter).
    pub fn parse_key(text: &str) -> Option<i64> {
        if text.len() != KEY_LEN || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        i64::from_str_radix(text, 16).ok()
    }

    /// What a link names: the key of a mail, or a message id of one
    /// database (an old link).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Target {
        Key(i64),
        Id(i64),
    }

    impl Target {
        /// The params of `locate_message` that find it.
        pub fn locate_params(self) -> super::LocateMessageParams {
            match self {
                Target::Key(k) => super::LocateMessageParams { link_key: Some(key_text(k)), message_id: None },
                Target::Id(id) => super::LocateMessageParams { message_id: Some(id), link_key: None },
            }
        }
    }

    /// What the link `text` names, as the app reads it (`DeepLink.target`
    /// in Swift): `nicemailer://m/` with the scheme and `m` in any case, a
    /// final slash allowed, then exactly 12 hexadecimal characters for a
    /// key, or 1 to 11 digits for an old id. Spaces around are ignored (a
    /// link pasted by hand); anything else is `None`.
    pub fn parse_url(text: &str) -> Option<Target> {
        let text = text.trim();
        let prefix = "nicemailer://m/";
        // `get` rather than slicing: a text shorter than the prefix, or cut
        // inside a multi-byte character, gives `None` instead of a panic.
        let head = text.get(..prefix.len())?;
        if !head.eq_ignore_ascii_case(prefix) {
            return None;
        }
        let rest = &text[prefix.len()..];
        let rest = rest.strip_suffix('/').unwrap_or(rest);
        if let Some(k) = parse_key(rest) {
            return Some(Target::Key(k));
        }
        if rest.is_empty() || rest.len() > MAX_ID_LEN || !rest.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        rest.parse().ok().filter(|id| *id > 0).map(Target::Id)
    }

    /// The link of a message: by its key when it has one, else by its id,
    /// which only this database knows (a mail without `Message-ID` whose
    /// body is not downloaded yet).
    pub fn url(message_id: i64, key: Option<i64>) -> String {
        match key {
            Some(k) => format!("nicemailer://m/{}", key_text(k)),
            None => format!("nicemailer://m/{message_id}"),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_key_is_written_on_12_characters_and_read_back() {
            assert_eq!(key_text(0x07bcf99aef4d), "07bcf99aef4d");
            assert_eq!(key_text(1), "000000000001");
            assert_eq!(parse_key("07BCF99AEF4D"), Some(0x07bcf99aef4d));
            assert_eq!(parse_key("123456789012"), Some(0x123456789012));
            for bad in ["", "07bcf99aef4", "07bcf99aef4d0", "07bcf99aef4g", "+7bcf99aef4d", "07bcf99aef4é"] {
                assert_eq!(parse_key(bad), None, "{bad}");
            }
        }

        #[test]
        fn a_link_is_read_back_as_the_app_reads_it() {
            assert_eq!(parse_url("nicemailer://m/3f9a0c2b7e41"), Some(Target::Key(0x3f9a0c2b7e41)));
            assert_eq!(parse_url(" NiceMailer://M/3F9A0C2B7E41/ "), Some(Target::Key(0x3f9a0c2b7e41)));
            assert_eq!(parse_url("nicemailer://m/123456789012"), Some(Target::Key(0x123456789012)));
            assert_eq!(parse_url("nicemailer://m/48213"), Some(Target::Id(48213)));
            for bad in [
                "", "nicemailer://m/", "nicemailer://x/48213", "https://m/48213", "nicemailer://m/0", "nicemailer://m/-3",
                "nicemailer://m/3f9a0c2b7e4", "nicemailer://m/3f9a0c2b7e4g", "nicemailer://m/1/2", "3f9a0c2b7e41",
                "nicemailer://m/é", "nicemailé",
            ] {
                assert_eq!(parse_url(bad), None, "{bad}");
            }
            assert_eq!(Target::Key(0x3f9a0c2b7e41).locate_params().link_key.as_deref(), Some("3f9a0c2b7e41"));
            assert_eq!(Target::Id(7).locate_params().message_id, Some(7));
        }

        #[test]
        fn the_url_takes_the_key_or_else_the_id() {
            assert_eq!(url(48213, Some(0x3f9a0c2b7e41)), "nicemailer://m/3f9a0c2b7e41");
            assert_eq!(url(48213, None), "nicemailer://m/48213");
        }
    }
}

/// Result of `locate_message`: where to show a message. A message is in
/// one folder; its thread may span several. By `link_key`, `folder_id` is
/// the folder to show the conversation in: for one of the user's own
/// messages (Sent, Drafts), the folder of its latest received message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessageLocation {
    pub message_id: i64,
    /// `None` for a message not linked to a thread yet (`message.thread_id`
    /// may be NULL while threading is pending).
    pub thread_id: Option<i64>,
    pub folder_id: i64,
    pub account_id: i64,
}

/// Params of `search`. `query` is plain text typed by the user; the core
/// turns it into a full-text query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchParams {
    pub query: String,
    #[serde(default)]
    pub limit: Option<u32>,
}

/// Params of `move_thread`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoveThreadParams {
    pub thread_id: i64,
    /// Target folder, by role (e.g. `"archive"`).
    pub target: FolderRole,
    /// Folder displayed when the command is given. The thread's messages in
    /// that folder are moved even when the folder is outside the ordinary
    /// scope of a command (restoring from Deleted, Junk or Banned, leaving
    /// Quarantine). Without it, only messages in `CONVERSATION_SCOPE` move.
    #[serde(default)]
    pub from_folder_id: Option<i64>,
    /// The user confirmed a favorable decision on a sender whose mail is
    /// suspected of spoofing (error `CONFIRMATION_REQUIRED`).
    #[serde(default)]
    pub confirm_suspect: bool,
    /// Authentication policy chosen with that confirmation, set on the
    /// address (`auto` or `not_required` in the app).
    #[serde(default)]
    pub auth_policy: Option<AuthPolicy>,
}

/// Params of `move_threads`: several threads moved to `target` in one
/// request, one transaction, one undo token (`ui.md`, "Actions groupées").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoveThreadsParams {
    /// Each thread with the folder it is shown in (`from_folder_id` of
    /// `move_thread`).
    pub threads: Vec<ThreadInFolder>,
    pub target: FolderRole,
    /// As in `move_thread`: the user confirmed the suspected senders.
    #[serde(default)]
    pub confirm_suspect: bool,
    #[serde(default)]
    pub auth_policy: Option<AuthPolicy>,
}

/// A thread and the folder it is shown in.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadInFolder {
    pub thread_id: i64,
    #[serde(default)]
    pub from_folder_id: Option<i64>,
}

/// Result of `move_threads`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoveThreadsResult {
    /// Messages modified, filed mails included.
    pub changed: usize,
    /// Mails of the threads moved.
    pub moved: usize,
    /// Other mails filed by the decisions.
    pub filed: usize,
    /// The decisions taken, one per sender decided on.
    pub decisions: Vec<DecisionView>,
    /// Pass it to `undo` to cancel the whole batch; absent when nothing
    /// changed.
    pub undo: Option<String>,
    /// The threads moved.
    pub moved_threads: Vec<i64>,
    /// Threads left: the target is not allowed from their folder.
    pub refused: Vec<i64>,
    /// Threads left: their sender is suspected of spoofing and the move
    /// would accept it. Ask the user, then move them again with
    /// `confirm_suspect`.
    pub suspects: Vec<SuspectView>,
}

/// A thread `move_threads` left for a confirmation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuspectView {
    pub thread_id: i64,
    pub addr: String,
    pub verdict: String,
}

/// A screening decision taken, changed or removed by a command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionView {
    /// `address` or `domain`.
    pub kind: String,
    /// The address or the domain.
    pub name: String,
    pub before: Option<ScreeningTarget>,
    pub after: Option<ScreeningTarget>,
}

/// Result of `move_thread`, `set_address_screening`,
/// `set_domain_screening` and `undo`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowResult {
    /// Messages modified, filed mails included. `0`: nothing changed.
    pub changed: usize,
    /// Mails of the thread moved (for `undo`: mails put back).
    pub moved: usize,
    /// Other mails of the sender filed by the decision.
    pub filed: usize,
    /// The decision recorded, if the command took one. For a move that
    /// decides nothing (to Todo, Junk…), absent.
    pub decision: Option<DecisionView>,
    /// Pass it to `undo` to cancel the command; absent when nothing changed.
    pub undo: Option<String>,
}

/// Parameters of `set_address_screening`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetAddressScreeningParams {
    pub addr: String,
    /// `None` removes the decision.
    #[serde(default)]
    pub target: Option<ScreeningTarget>,
}

/// Parameters of `set_sender_kind`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetSenderKindParams {
    /// The `address` id (`ContactView.id`).
    pub address_id: i64,
    /// `None` gives the address back to the computation.
    #[serde(default)]
    pub kind: Option<SenderKind>,
}

/// Parameters of `set_domain_screening`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetDomainScreeningParams {
    /// E.g. `example.com`; its subdomains follow the rule.
    pub domain: String,
    #[serde(default)]
    pub target: Option<ScreeningTarget>,
}

/// Parameters of `undo`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UndoParams {
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetThreadSeenParams {
    /// One thread…
    #[serde(default)]
    pub thread_id: Option<i64>,
    /// …or several, in one transaction and one batch of events (the marked
    /// threads of the app, `ui.md`, "Actions groupées"). Exactly one of the
    /// two. Threads gone meanwhile are skipped.
    #[serde(default)]
    pub thread_ids: Option<Vec<i64>>,
    pub seen: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetThreadFlaggedParams {
    /// One thread, or several (`thread_ids`), as for `set_thread_seen`.
    #[serde(default)]
    pub thread_id: Option<i64>,
    #[serde(default)]
    pub thread_ids: Option<Vec<i64>>,
    pub flagged: bool,
}

/// The threads of a command: `thread_id` or `thread_ids`, exactly one.
/// `None` when both or neither are given.
pub fn thread_list(one: Option<i64>, several: Option<Vec<i64>>) -> Option<Vec<i64>> {
    match (one, several) {
        (Some(id), None) => Some(vec![id]),
        (None, Some(ids)) if !ids.is_empty() => Some(ids),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Presentation views
// ---------------------------------------------------------------------------

/// A folder, as displayed in the sidebar.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderView {
    pub id: i64,
    /// `None` for a user-created folder without a workflow role (not
    /// possible yet, but allowed by the schema).
    pub role: Option<FolderRole>,
    pub name: String,
    /// Keyboard shortcut that moves a thread to this folder.
    pub letter: Option<String>,
    pub category: Option<FolderCategory>,
    /// Number of messages in the folder.
    pub total_count: i64,
    pub unread_count: i64,
    /// Targets offered by the move command from this folder. Computed by
    /// the core so that clients never reimplement workflow rules.
    pub move_targets: Vec<FolderRole>,
    /// Account of the folder: a client lists one group of folders per
    /// account.
    pub account_id: i64,
    /// Account name (`account.config_key`).
    pub account_name: String,
    /// `"imap"`, or `"archive"` for an account created by an mbox import,
    /// which never has a server.
    pub account_kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderList {
    pub items: Vec<FolderView>,
    /// Event position at the time of the read; events after it are newer
    /// than this data.
    pub as_of: Position,
}

/// A thread, as displayed in a folder's list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadSummary {
    pub id: i64,
    pub subject: String,
    /// Names of the senders of the thread's messages, from oldest to
    /// newest, without duplicates. Like the counters, without the mails of
    /// Deleted, Junk and Banned unless the thread has only such mails.
    pub participants: Vec<String>,
    /// The person the conversation is with, for the sender column of a
    /// list (`ui.md`, "Liste"): the author of its latest mail that the
    /// user did not write; when the user wrote every mail, the first
    /// recipient of the latest one. A name if the header gives one, else
    /// the address. `None` when the mails name nobody (no `From`, no
    /// recipient). Counted on the same mails as `participants`; drafts
    /// only count in a conversation of drafts alone.
    #[serde(default)]
    pub correspondent: Option<String>,
    /// Who wrote the mail that names the correspondent: a person or a
    /// robot (`message.sender_kind`, `expediteurs.md`, section 6). `None`
    /// when no mail names one, when the user wrote every mail, or when the
    /// mail has no verdict yet (catch-up not done). The app shows it as an
    /// icon before the name in Quarantine, where telling a person from a
    /// machine helps the decision (`ui.md`, "Quarantine").
    #[serde(default)]
    pub correspondent_kind: Option<SenderKind>,
    /// The user wrote the latest mail: the conversation waits on the
    /// correspondent. The list marks it before the name ("→ Alice").
    #[serde(default)]
    pub last_is_mine: bool,
    /// Beginning of the latest message's text.
    pub snippet: Option<String>,
    /// Unix timestamp (seconds) of the thread's latest mail, sent or
    /// received, outside Deleted, Junk and Banned (all its mails when it
    /// has only such mails). The list is sorted on it, newest first.
    pub date_last_received: i64,
    /// Counters of the whole thread, on the same mails as
    /// `date_last_received` (`dossiers.md`, "Dossier d'une conversation"):
    /// the same in every list the thread is in.
    pub message_count: i64,
    pub unread_count: i64,
    pub is_flagged: bool,
    pub has_attachments: bool,
    /// Set when the thread came back to the Inbox from Todo or Waiting.
    pub returned_from: Option<FolderRole>,
    /// Folder preselected if the user replies (see `FolderRole::reply_target`).
    pub reply_target: Option<FolderRole>,
    /// Folder this row describes: the listed folder (of the thread's
    /// account), or in Everything and All Mail the folder of the thread
    /// (`dossiers.md`, "Dossier d'une conversation"). Moves pass it as
    /// `from_folder_id`.
    pub folder_id: i64,
    /// Unix timestamp of the latest change to a message of the thread, in
    /// any folder (sort `modified`).
    pub date_last_modified: i64,
    /// Names of the tags on the thread's messages, sorted.
    pub tags: Vec<String>,
}

/// One page of a paginated list.
///
/// Rust note: `<T>` makes the struct *generic*: `Page<ThreadSummary>` is a
/// page of threads, and the same definition could hold any other item type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    /// Cursor to pass to get the next page; `None` on the last page.
    pub next_cursor: Option<String>,
    pub as_of: Position,
}

/// An email address with its optional display name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddressView {
    pub addr: String,
    pub name: Option<String>,
}

/// A message, as displayed when reading a thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageView {
    pub id: i64,
    pub folder_id: i64,
    pub folder_role: Option<FolderRole>,
    pub subject: String,
    pub from_name: Option<String>,
    pub from_addr: Option<String>,
    pub to: Vec<AddressView>,
    pub cc: Vec<AddressView>,
    /// `Reply-To`: where a reply goes, when the sender says another
    /// address than its From. Another domain than the From's is a strong
    /// sign of phishing for the security check (`ia.md`, section 6).
    #[serde(default)]
    pub reply_to: Vec<AddressView>,
    /// `Date:` header, as claimed by the sender (may be missing or wrong).
    pub date_sent: Option<i64>,
    /// When the server received the message; trustworthy, used for sorting.
    pub date_received: i64,
    /// Plain text body; `None` if the body has not been downloaded yet.
    pub body_text: Option<String>,
    pub is_seen: bool,
    pub is_flagged: bool,
    pub is_answered: bool,
    pub has_attachments: bool,
    /// Sender authentication result (SPF/DKIM/DMARC): `"pass"`, `"fail"`, ...
    pub auth_result: String,
    /// Sanitized HTML body, when the message has an HTML part.
    pub body_html: Option<String>,
    /// `body_html` holds remote images a client may load on request: their
    /// sources are `nm-remote://image?u=<percent-encoded URL>`, a scheme no
    /// web view loads by itself. Tracking pixels are not among them: they
    /// are removed (`tracking_pixels`).
    pub has_remote_content: bool,
    /// Display preferred for the sender's mails: `"text"`, `"html"`,
    /// `"images"` (HTML with remote images), or `None`: the default of the
    /// theme, `reader.body_format` (`themes.md`, section 6.6). Set by
    /// `set_body_format`, always to an explicit value.
    pub from_body_format: Option<String>,
    /// The text body cut into segments, for a simplified display: what the
    /// message brings is visible, the history it repeats is `hidden`
    /// (`docs/thread-display.md`). Empty when the body is not downloaded.
    pub segments: Vec<SegmentView>,
    /// The message brings nothing new: it only quotes earlier ones.
    pub only_quotes: bool,
    /// The body is downloaded and holds nothing written and nothing
    /// quoted (an empty body, blank lines, a signature alone): clients say
    /// "no text" rather than "only quotes" (`docs/thread-display.md`).
    /// False while the body is not downloaded: nothing is known of it.
    /// Rust note: `#[serde(default)]` reads a missing field as `false`, so
    /// a message written by an older core still parses.
    #[serde(default)]
    pub no_text: bool,
    /// Shown unfolded by default: the latest message and the unread ones,
    /// as in Gmail; the others are folded on one line.
    pub expanded: bool,
    /// Whether to show the recipients: only when they differ from the
    /// previous message's (the first message: when there are several).
    pub show_recipients: bool,
    /// Files attached to the message, in MIME order. Images embedded in the
    /// HTML body (`cid:`) are left out, unless no HTML shows them.
    #[serde(default)]
    pub attachments: Vec<AttachmentView>,
    /// How to unsubscribe from the list the message comes from, when it
    /// says (`ui.md`, "Désabonnement").
    #[serde(default)]
    pub unsubscribe: Option<UnsubscribeView>,
    /// Remote tracking pixels removed from `body_html`: never requested,
    /// whatever the display.
    #[serde(default)]
    pub tracking_pixels: i64,
    /// Links of the message, for the `l` panel: the `<a href>` of the HTML
    /// with their text, then the addresses written in the text part, then
    /// its phone numbers as `tel:` links, each once (`links.rs` and
    /// `phones.rs` in nm-store). Only `http`, `https`, `mailto` and `tel`.
    #[serde(default)]
    pub links: Vec<LinkView>,
    /// The verification code the message gives, when it is a code mail
    /// ("Votre code de vérification : 610731"): 4 to 8 digits, or 6 to 8
    /// capitals and digits (`codes.rs` in nm-store). Offered first by the
    /// `l` panel, where Return copies it.
    #[serde(default)]
    pub verification_code: Option<String>,
    /// The deep link of the message, to paste in a note:
    /// `nicemailer://m/<key>`, the same on every Mac that has the mail
    /// ([`deep_link::url`], `docs/deep-links.md`).
    #[serde(default)]
    pub link: String,
}

/// A link of a message (`MessageView.links`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinkView {
    /// `tel:` URLs hold digits only, after a `+` for an international
    /// number: `tel:+33612345678`, `tel:0612345678`.
    pub url: String,
    /// Visible text of the link in the HTML ("See my invoice"), or a phone
    /// number as written in the text; `None` for an address written in the
    /// text, or a link around an image.
    pub text: Option<String>,
    /// Where a tracking link really goes, when its address carries it
    /// (decoded from base64 or percent-encoding); `None` otherwise.
    #[serde(default)]
    pub target: Option<String>,
    /// The other texts of the same link (same destination) in the message,
    /// each once, in order.
    #[serde(default)]
    pub other_texts: Vec<String>,
}

/// How to unsubscribe from a mailing list (`MessageView.unsubscribe`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnsubscribeView {
    /// `one_click`: the core unsubscribes by itself (RFC 8058); `browser`:
    /// a web page to open; `mail`: an address to write to.
    pub method: String,
    /// `header` (List-Unsubscribe, reliable) or `body` (a link of the
    /// text, opened in the browser).
    pub source: String,
    pub url: Option<String>,
    pub mailto: Option<String>,
    /// When the user unsubscribed from this sender with one click.
    pub sender_unsubscribed_at: Option<i64>,
}

/// An attachment of a message (`MessageView.attachments`). The content is
/// fetched on demand with `get_attachment`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttachmentView {
    pub id: i64,
    /// Name given by the sender; `None` when it gave none.
    pub filename: Option<String>,
    /// Lowercase MIME type, `application/octet-stream` when unknown.
    pub mime_type: String,
    /// Decoded size in bytes.
    pub size: i64,
    /// Whether the core can read its text, for the AI to summarize it
    /// (`ia.md`, section 5, `attachment.text`): text, HTML, PDF, DOCX, a
    /// mail attached. Not whether it holds any: a scanned PDF has none,
    /// which reading it says.
    #[serde(default)]
    pub readable: bool,
}

/// A piece of a message's text (`MessageView.segments`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SegmentView {
    /// `text` (written by the sender), `quote` (interleaved quote, shown
    /// dimmed), `quote_tail` (quoted history at the end), `signature`,
    /// `boilerplate` ("Sent from my iPhone", confidentiality notice),
    /// `forward` (forwarded message), `copy` (copies an earlier message).
    pub kind: String,
    /// The text, without quote markers.
    pub text: String,
    /// Quote depth, 0 outside quotes.
    pub depth: u8,
    /// Hidden by default; the client shows it on demand.
    pub hidden: bool,
    /// For a hidden `quote_tail`: its content is in an earlier message of
    /// the thread. If not, the history it holds is missing from the thread.
    pub in_thread: bool,
}

/// A whole thread, as displayed in the reading pane.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadView {
    pub id: i64,
    pub subject: String,
    /// Every message of the thread, across all folders, sorted by date.
    pub messages: Vec<MessageView>,
    /// Names of the thread's tags, sorted: shown in the header (`ui.md`,
    /// "Tags et groupes dans l'en-tête").
    #[serde(default)]
    pub tags: Vec<String>,
    /// The person the thread is with (`ThreadLabels::correspondent`).
    #[serde(default)]
    pub correspondent: Option<String>,
    /// Groups of that person, in display order, shown in the header.
    #[serde(default)]
    pub correspondent_groups: Vec<String>,
    pub as_of: Position,
}

/// One search result. Results are messages, not threads.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub message_id: i64,
    pub thread_id: Option<i64>,
    pub folder_id: i64,
    pub folder_role: Option<FolderRole>,
    pub subject: String,
    pub from_name: Option<String>,
    pub from_addr: Option<String>,
    pub date_received: i64,
    /// Excerpt of the matching text, with matched terms wrapped in `[` and
    /// `]` so the client can highlight them.
    pub highlight: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub items: Vec<SearchHit>,
    pub as_of: Position,
}

/// Result of a write command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandResult {
    /// Number of messages modified. `0` means the command was a no-op.
    pub changed: usize,
}

// ---------------------------------------------------------------------------
// Drafts and sending (`reponse.md`)
// ---------------------------------------------------------------------------

/// What a new draft is (`reponse.md`): `r`, `R`, `f` or `c`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DraftKind {
    /// `r`: to the sender and every recipient.
    #[default]
    ReplyAll,
    /// `R`: to the sender only.
    ReplySender,
    /// `f`: the mail and its files, to recipients chosen by the user.
    Forward,
    /// `c`: a new mail, from scratch.
    New,
}

/// Params of `prepare_draft` and `create_draft`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateDraftParams {
    #[serde(default)]
    pub kind: DraftKind,
    /// The message replied to or forwarded; its account sends, if it can,
    /// else an account that can (by preference one the mail was sent to).
    /// Absent for a new mail.
    #[serde(default)]
    pub reply_to: Option<i64>,
    /// A new mail: the account that sends it; absent, or an account that
    /// cannot send, the first account that can.
    #[serde(default)]
    pub account: Option<i64>,
    /// The line introducing the quotation, written by the client in its
    /// language ("On Nov 7, 2026, at 10:12, Alice <a@x> wrote:"); for a
    /// forward, the block introducing the mail forwarded.
    #[serde(default)]
    pub quote_header: String,
    /// The body to start with instead of the quotation: restores a draft
    /// abandoned by mistake (`z` after `discard_draft`).
    #[serde(default)]
    pub body: Option<String>,
    /// The format of `body` when it is given: `text` (the default) or
    /// `html`, the editor's formatted text (`edition.md`, section 3).
    #[serde(default)]
    pub body_format: Option<String>,
}

/// Params of `open_draft`, `close_draft`, `discard_draft`, `cancel_send`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftIdParams {
    pub id: i64,
}

/// Params of `update_draft`. Each field left out stays as it is: the
/// client sends the body at each pause, the recipients and the subject only
/// when they changed.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateDraftParams {
    pub id: i64,
    /// The revision the client edited: refused if it is not the current
    /// one (`REVISION_CONFLICT`).
    pub revision: i64,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub subject: Option<String>,
    /// The recipients, each list whole. An address that is not one is
    /// refused (`INVALID_PARAMS`): the client sends what `parse_addresses`
    /// accepted.
    #[serde(default)]
    pub to: Option<Vec<AddressView>>,
    #[serde(default)]
    pub cc: Option<Vec<AddressView>>,
    #[serde(default)]
    pub bcc: Option<Vec<AddressView>>,
    /// The address to send from, one of `list_senders`: its account sends
    /// the mail (its SMTP server, its Sent folder). Refused (`CANNOT_SEND`)
    /// for an account that cannot send, `INVALID_PARAMS` for an address
    /// that is not one of the account's.
    #[serde(default)]
    pub from: Option<DraftSender>,
    /// `text` (text only) or `html` (formatted: `body` is the editor's
    /// HTML), `edition.md`, section 3. Sent with the body when the user
    /// switches mode (`⇧⌘T`).
    #[serde(default)]
    pub body_format: Option<String>,
}

/// The address a draft is sent from, and the account it belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftSender {
    pub account_id: i64,
    pub addr: String,
}

/// An address a mail can be sent from, in `list_senders`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SenderView {
    pub account_id: i64,
    /// Key of the account in `accounts.toml` (or name of the account).
    pub account: String,
    pub addr: String,
    pub name: Option<String>,
}

/// Result of `list_senders`: the accounts in display order, each with its
/// main address first, then its identities.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListSendersResult {
    pub items: Vec<SenderView>,
}

/// A file attached to a draft.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DraftAttachmentView {
    pub id: i64,
    pub filename: String,
    pub mime_type: String,
    /// Bytes of the file (base64 makes it a third bigger in the mail).
    pub size: i64,
}

/// Params of `add_draft_attachment`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddDraftAttachmentParams {
    pub id: i64,
    pub revision: i64,
    /// The name of the file in `incoming.noindex` (letters, digits, `-`).
    pub staged: String,
    /// The name shown and sent.
    pub filename: String,
    /// `type/subtype`; absent or unknown: `application/octet-stream`.
    #[serde(default)]
    pub mime_type: Option<String>,
}

/// Result of `add_draft_attachment`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddDraftAttachmentResult {
    pub revision: i64,
    pub attachment: DraftAttachmentView,
    /// The mail is now bigger than most providers accept: said, not refused.
    pub warning: Option<String>,
}

/// Params of `remove_draft_attachment`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoveDraftAttachmentParams {
    pub id: i64,
    pub revision: i64,
    pub attachment_id: i64,
}

/// A font with known fallbacks (`nm_core::formats::FONTS`): its name, and
/// the CSS stack sent. The editor offers every font of the machine; these
/// give the common ones their fallbacks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FontView {
    pub name: String,
    pub stack: String,
}

/// A size of the formatted mode: its key, the number shown (`18`), and
/// the CSS value sent (`18px`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SizeView {
    pub key: String,
    pub css: String,
}

/// A colour of the palette: a key the client translates, its `#rrggbb`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColorView {
    pub key: String,
    pub hex: String,
}

/// Result of `compose_formats` (`nm_core::formats`): the editor offers
/// exactly what the core lets through when the mail leaves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComposeFormatsResult {
    pub fonts: Vec<FontView>,
    pub sizes: Vec<SizeView>,
    pub colors: Vec<ColorView>,
    /// `text` or `html`: the mode a new draft starts in (`compose.format`).
    pub default_format: String,
}

/// Params of `html_to_text`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HtmlToTextParams {
    pub html: String,
}

/// Result of `html_to_text`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HtmlToTextResult {
    pub text: String,
    /// The HTML has formatting that text only loses.
    pub formatted: bool,
}

/// Params of `parse_addresses`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParseAddressesParams {
    pub text: String,
}

/// Result of `parse_addresses`: the addresses understood, in order, each
/// once; the pieces that are not an address, as typed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParseAddressesResult {
    pub addresses: Vec<AddressView>,
    pub invalid: Vec<String>,
}

/// Params of `suggest_recipients`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuggestRecipientsParams {
    /// What is typed: the start of an address, of a name, of a word of a
    /// name, or of a domain.
    pub query: String,
    #[serde(default = "default_suggestions")]
    pub limit: u32,
}

fn default_suggestions() -> u32 {
    8
}

/// Result of `suggest_recipients`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuggestRecipientsResult {
    pub items: Vec<AddressView>,
}

/// Params of `send_draft`: the last body typed is saved with it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendDraftParams {
    pub id: i64,
    pub revision: i64,
    pub body: String,
    /// The format of `body`, `text` or `html`; absent: the draft's.
    #[serde(default)]
    pub body_format: Option<String>,
    /// Where to file the conversation after sending (`⇧⌘↩`), saved with
    /// the draft. Absent: the choice saved earlier, if any (a send
    /// cancelled with `z`), else the core's preselection.
    #[serde(default)]
    pub after_send: Option<AfterSend>,
}

/// A draft, as the reply screen shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DraftView {
    /// 0 for a draft `prepare_draft` proposes, which no row holds yet: the
    /// client writes it with `create_draft` before any call that takes an
    /// id. A real draft's id is never 0 (SQLite row ids start at 1).
    pub id: i64,
    /// The account of the draft: of the mail answered or forwarded, whose
    /// conversation it joins and whose Drafts folder keeps it.
    pub account_id: i64,
    /// The account that sends it (its SMTP server, its Sent folder):
    /// `account_id`, unless the draft is sent from another account, as a
    /// reply to a mail of an account that cannot send (`reponse.md`,
    /// section 4 ter).
    #[serde(default)]
    pub send_account_id: i64,
    pub from: AddressView,
    pub to: Vec<AddressView>,
    pub cc: Vec<AddressView>,
    /// Never in the mail sent; kept in the copies of the user (Sent, Drafts).
    #[serde(default)]
    pub bcc: Vec<AddressView>,
    pub subject: String,
    pub body: String,
    /// `text`: `body` is the text; `html`: it is the HTML of the editor's
    /// formatted mode (`edition.md`, section 3).
    #[serde(default = "text_format")]
    pub body_format: String,
    /// In a forward proposed by `prepare_draft`, the files of the mail it
    /// will take, each with a negative id: they are copied, and get their
    /// real id, when `create_draft` writes the forward.
    #[serde(default)]
    pub attachments: Vec<DraftAttachmentView>,
    /// 0 for a draft proposed (see `id`).
    pub revision: i64,
    /// `editing` or `sending`.
    pub state: String,
    /// `reply`, `forward` or `new`.
    pub kind: String,
    /// The message replied to.
    pub reply_to: Option<i64>,
    /// The message forwarded.
    #[serde(default)]
    pub forward_of: Option<i64>,
    /// Its conversation.
    pub thread_id: Option<i64>,
    /// Where the conversation goes once the mail is sent, as preselected
    /// now (`reponse.md`, section 8); absent when it stays, and then no
    /// choice is offered.
    pub after_send_target: Option<FolderRole>,
    /// The filing the user chose (`⇧⌘↩`), which wins over the
    /// preselection; absent: none chosen.
    #[serde(default)]
    pub after_send: Option<AfterSend>,
    /// When a draft `sending` leaves, Unix seconds.
    pub send_at: Option<i64>,
}

fn text_format() -> String {
    "text".into()
}

/// Result of `prepare_draft` and `create_draft`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateDraftResult {
    pub draft: DraftView,
    /// The draft already existed (a reply to this message was started).
    pub reused: bool,
    /// A forward whose files could not all be taken (a mail whose raw file
    /// is not here): which. The text is forwarded all the same.
    #[serde(default)]
    pub warning: Option<String>,
}

/// Result of `update_draft`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateDraftResult {
    pub revision: i64,
}

/// Result of `send_draft`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendDraftResult {
    /// When the mail leaves, Unix seconds; until then `cancel_send` stops it.
    pub send_at: i64,
}

/// File locations shared by the core and its clients (`stockage-brut.md`).
/// Parameters of `import_mbox`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportMboxParams {
    /// Absolute path of the mbox file, readable by the core.
    pub path: String,
    /// Name of the archive account, created if it does not exist.
    pub account: String,
    /// Address of the archived mailbox: its messages go to Sent.
    pub address: String,
    /// Give the screening decision `inbox` to the recipients of sent
    /// messages that have none (`import-initial.md`, section 4). Default:
    /// true, like the checked box of the import screen.
    #[serde(default = "default_true")]
    pub mark_correspondents: bool,
}

fn default_true() -> bool {
    true
}

/// State of an mbox import: result of `import_mbox` and `import_status`,
/// and payload of the import events.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ImportStatus {
    /// Identifier of the import, unique while the core runs.
    pub job_id: u64,
    pub path: String,
    pub account_id: i64,
    pub running: bool,
    pub bytes_read: u64,
    pub total_bytes: u64,
    /// Messages added.
    pub imported: u64,
    /// Messages already present in the account, not added again.
    pub skipped: u64,
    /// Messages kept but that the MIME parser could not interpret.
    pub parse_errors: u64,
    /// Final figures, set by `ImportFinished`.
    #[serde(default)]
    pub sent: u64,
    #[serde(default)]
    pub archived: u64,
    #[serde(default)]
    pub correspondents: u64,
    #[serde(default)]
    pub seconds: f64,
    /// Why the import failed, if it did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Parameters of `set_body_format`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetBodyFormatParams {
    /// Address whose mails are concerned (usually the sender's).
    pub addr: String,
    /// `"text"`, `"html"`, `"images"` (HTML with its remote images), or
    /// `null` to go back to the default.
    #[serde(default)]
    pub format: Option<String>,
}

/// File format of `export_message`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    /// The raw message, byte for byte (RFC 5322), openable by any mail app.
    Eml,
    /// Headers and text body.
    Txt,
    /// Headers and text body, in Markdown.
    Md,
}

/// Parameters of `export_message`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportMessageParams {
    pub message_id: i64,
    pub format: ExportFormat,
    /// Absolute directory to write into; default `~/Downloads`.
    #[serde(default)]
    pub dir: Option<String>,
}

/// Parameters of `message_text`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageTextParams {
    pub message_id: i64,
}

/// Result of `message_text`: the headers of the text export (From, To, Cc,
/// Date, Subject), an empty line, then the text body with each paragraph
/// on one line and one empty line between paragraphs, the line breaks of
/// the sender's mail app removed (format=flowed, hard wrapping). Made to
/// be pasted into a document; no line break at the end.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageTextResult {
    pub text: String,
}

/// Parameters of `message_record`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageRecordParams {
    pub message_id: i64,
}

/// Result of `message_record`: one message as a piece of evidence
/// (`ui.md`, menu `s`, "p as pdf"). Read from the raw message when the core
/// has it (`raw_sha256` set): the names as written in this mail, every
/// recipient, the `Date` header as written, the trace headers. Otherwise
/// from the database, which the client says on the page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageRecordView {
    /// `Message-ID` header, without angle brackets.
    pub message_id: Option<String>,
    pub subject: String,
    /// Usually one mailbox; several when the mail names several authors.
    pub from: Vec<AddressView>,
    /// `Sender` header: who sent the mail on behalf of `from`, if given.
    pub sender: Option<AddressView>,
    pub reply_to: Vec<AddressView>,
    pub to: Vec<AddressView>,
    pub cc: Vec<AddressView>,
    /// Only known for mails the user sent (a received mail does not carry
    /// its Bcc).
    pub bcc: Vec<AddressView>,
    /// `Date` header as written, with the sender's time zone
    /// ("Tue, 23 Jun 2020 21:20:56 +0200").
    pub date_header: Option<String>,
    /// `Date` header, Unix seconds.
    pub date_sent: Option<i64>,
    /// When the server received the mail (IMAP INTERNALDATE), Unix seconds.
    pub date_received: i64,
    pub in_reply_to: Option<String>,
    /// `References`, oldest first.
    pub references: Vec<String>,
    /// Account (`account.config_key`) and folder path holding the mail.
    pub account: String,
    pub folder: String,
    /// Verdict of the sender's authentication: `pass`, `fail`, `none`,
    /// `unknown` (`authentification-expediteur.md`).
    pub auth_result: String,
    /// Text body, paragraphs whole (`message_text`); `None` while the body
    /// is not downloaded.
    pub body: Option<String>,
    pub attachments: Vec<RecordAttachmentView>,
    /// Trace headers as written, one line each: Return-Path, Received
    /// (newest hop first), Authentication-Results, DKIM-Signature…
    pub headers: Vec<RecordHeaderView>,
    /// Size of the raw message in bytes.
    pub raw_size: Option<i64>,
    /// SHA-256 of the raw message (the `.eml` of `se`), lowercase hex;
    /// `None` when the core has no raw file for it.
    pub raw_sha256: Option<String>,
}

/// An attachment of `MessageRecordView`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordAttachmentView {
    /// IMAP part number ("2", "1.2").
    pub part_id: String,
    pub filename: Option<String>,
    pub mime_type: String,
    /// Decoded size in bytes.
    pub size: i64,
    /// An image shown inside the HTML body (`cid:`).
    pub inline: bool,
    /// SHA-256 of the decoded content, lowercase hex: the hash of the file
    /// `get_attachment` gives. `None` without a raw file.
    pub sha256: Option<String>,
}

/// A header field of `MessageRecordView.headers`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordHeaderView {
    pub name: String,
    /// Unfolded onto one line, cut beyond 4,000 characters.
    pub value: String,
}

/// Parameters of `unsubscribe`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnsubscribeParams {
    pub message_id: i64,
}

/// Result of `unsubscribe`. With `done`, the core unsubscribed by itself
/// (one click) and recorded it; otherwise the client opens `open` (a web
/// page or a `mailto:`), and `error` says why a one-click attempt failed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnsubscribeResult {
    pub method: String,
    pub done: bool,
    pub open: Option<String>,
    pub error: Option<String>,
}

/// Parameters of `get_attachment`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetAttachmentParams {
    pub attachment_id: i64,
}

/// Result of `get_attachment`: the file, extracted from the raw message on
/// first use (`architecture.md`, section 5). It belongs to the core
/// (read-only, quarantined): clients copy or open it, never change it, and
/// ask again before each use since it may be purged and re-extracted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentFileView {
    /// Absolute path under `<data_dir>/attachments.noindex/`.
    pub path: String,
    /// Safe file name, to copy the file under.
    pub filename: String,
    pub mime_type: String,
    pub size: i64,
}

/// Parameters of `attachment_text`: one page of the text of an attachment,
/// counted in characters (Unicode scalar values, what Rust's `chars`
/// counts), so that a page never cuts a character in two.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentTextParams {
    pub attachment_id: i64,
    /// First character of the page; 0 when absent.
    #[serde(default)]
    pub offset: Option<u64>,
    /// Characters of the page: [`ATTACHMENT_TEXT_DEFAULT_CHARS`] when
    /// absent, [`ATTACHMENT_TEXT_MAX_CHARS`] at most.
    #[serde(default)]
    pub limit: Option<u64>,
}

/// Default page of `attachment_text`, in characters.
pub const ATTACHMENT_TEXT_DEFAULT_CHARS: u64 = 40_000;
/// Largest page of `attachment_text`: the whole text the core reads of a
/// file (`nm_store::attachment_text::MAX_TEXT_CHARS`).
pub const ATTACHMENT_TEXT_MAX_CHARS: u64 = 100_000;

/// Result of `attachment_text`. A file whose text cannot be read (an
/// image, a scan, a file protected by a password) is not an error: `text`
/// is absent and `unreadable` says why, so that a client that can open
/// files still gets the `path`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentTextResult {
    /// The file, as `get_attachment` gives it (`AttachmentFileView`).
    pub path: String,
    pub filename: String,
    pub mime_type: String,
    pub size: i64,
    /// How it was read: `text`, `html`, `pdf`, `docx` or `mail`; absent
    /// for a kind of file whose text is not read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// The page asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Why there is no text, in English.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unreadable: Option<String>,
    /// First character of the page.
    pub offset: u64,
    /// First character of the next page; absent on the last one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_offset: Option<u64>,
    /// Characters of the whole text read.
    pub total_chars: u64,
    /// The text reached the most the core reads of a file
    /// ([`ATTACHMENT_TEXT_MAX_CHARS`]): the end of the file is not in it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cut: bool,
}

/// Result of `export_message`: where the file was written. An existing
/// file is never overwritten: " (2)", " (3)"… is added to the name.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportResult {
    pub path: String,
}

/// Parameters of `list_history`: cursor pagination, newest first.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListHistoryParams {
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    /// Only the lines written since this Unix time, included
    /// (`tracabilite.md`, section 6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<i64>,
    /// Only the lines of these sources (`app`, `mcp`…); every source when
    /// empty. What an AI agent did is `["mcp"]` (`mcp.md`, `agent_history`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<String>,
}

/// One `audit_log` line.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: i64,
    /// Unix timestamp.
    pub date: i64,
    /// Who did it: `app`, `cli`, `core`…
    pub source: String,
    /// `account`, `folder`, `address`, `contact_group`, `message`…
    pub entity: String,
    pub entity_id: i64,
    /// `move`, `set_flags`, `screening`, `import_mbox`…
    pub action: String,
    /// Readable one-line description built by the core (subject of the
    /// message, address…), so clients do not decode `detail`.
    pub summary: String,
    /// The raw JSON `detail` of the line.
    pub detail: Value,
}

/// Parameters of `list_history_groups`: cursor pagination, newest first.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListHistoryGroupsParams {
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    /// Also give the mails marked as read, most of the log and rarely what
    /// is looked for.
    #[serde(default)]
    pub include_read: bool,
    /// With `last_id`: the lines of one group, each on its own (the
    /// `first_id` and `last_id` of that group). `include_read` then has no
    /// effect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_id: Option<i64>,
    /// Only the lines of these sources (`app`, `core`, `cli`, `mcp`…): who
    /// did it, the filter of the History screen. Every source when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<String>,
}

/// Consecutive `audit_log` lines of one action: one source doing the same
/// thing to several items at once, or a single line (`count` 1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryGroup {
    /// The oldest and the newest line of the group.
    pub first_id: i64,
    pub last_id: i64,
    /// How many lines it holds.
    pub count: i64,
    /// Unix timestamp of the newest line.
    pub date: i64,
    /// Who did it: `app`, `cli`, `core`…
    pub source: String,
    /// `message`, `address`, `account`… and the id of the newest line's.
    pub entity: String,
    pub entity_id: i64,
    /// What was done, as a stable code for the client to word: the `action`
    /// of the log, or for the actions that mean several things `read`,
    /// `unread`, `flagged`, `unflagged`, `tag_added`, `tag_removed`,
    /// `decision`, `decision_removed`.
    pub kind: String,
    /// What it was done with: a folder (its role, else its path), a tag, an
    /// address, a screening target, a number of mails imported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// The subject of the mail, the address, the name of the newest line's
    /// item. `None`: the item no longer exists.
    pub title: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListTagsParams {
    /// Only the tags of this account; all accounts when absent.
    #[serde(default)]
    pub account_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagView {
    pub id: i64,
    pub account_id: i64,
    pub name: String,
    pub color: Option<String>,
    /// Number of messages carrying the tag.
    pub message_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagThreadParams {
    /// One thread, or several (`thread_ids`), as for `set_thread_seen`.
    #[serde(default)]
    pub thread_id: Option<i64>,
    #[serde(default)]
    pub thread_ids: Option<Vec<i64>>,
    /// Tag name; the tag is created in the thread's account if needed.
    pub name: String,
    /// Remove the tag instead of adding it.
    #[serde(default)]
    pub remove: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupView {
    pub id: i64,
    pub name: String,
    pub color: Option<String>,
    pub member_count: i64,
    /// Latest change of the group (Unix time): its own row (name, color,
    /// position) or one of its memberships (added, removed). With
    /// `mail_count`, how a client picks the groups worth showing when it
    /// has room for a few only (`groupes.md`, section 6).
    pub date_modified: i64,
    /// Mails received from the members, all accounts (the sum of their
    /// `address.sender_count`, automatic replies left out). A counter kept
    /// by ingestion: reading it costs one row per member, never a count of
    /// the mails.
    pub mail_count: i64,
}

/// Parameters of `set_group_member`. Give either `addr` or `thread_id`
/// (then the member is the thread's sender: the author of its latest
/// message that is not in Sent or Drafts).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetGroupMemberParams {
    /// Group name; the group is created if needed.
    pub group: String,
    #[serde(default)]
    pub addr: Option<String>,
    /// The thread whose correspondent is added: its latest author who is
    /// not the user, else the first recipient of the user's latest mail.
    #[serde(default)]
    pub thread_id: Option<i64>,
    /// Remove the address from the group instead of adding it.
    #[serde(default)]
    pub remove: bool,
}

/// Parameters of `rename_tag` and `rename_group`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenameParams {
    /// The current name, compared without case.
    pub name: String,
    /// The new name: 64 characters at most, not the name of another tag
    /// (of the same account) or group.
    pub new_name: String,
}

/// Result of `rename_tag` and `rename_group`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenameResult {
    /// Whether a name changed (`false` when the new name is the old one).
    pub changed: bool,
}

/// Parameters of `delete_tag` and `delete_group`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteNameParams {
    /// The name, compared without case.
    pub name: String,
}

/// Result of `delete_group`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteGroupResult {
    pub group_id: i64,
    /// Members the group had when it was deleted.
    pub members: i64,
}

/// Result of `set_group_member`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupMemberResult {
    pub group_id: i64,
    pub addr: String,
    /// Whether something changed (adding an existing member does not).
    pub changed: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListContactsParams {
    /// Filter: part of the address or of the name, case-insensitive.
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContactView {
    pub id: i64,
    pub addr: String,
    pub name: Option<String>,
    /// Screening decision, `None` without decision.
    pub screening_target: Option<String>,
    /// Decision of the address's domain (or of a parent domain), which
    /// applies when the address has none.
    #[serde(default)]
    pub domain_target: Option<String>,
    /// `auto`, `required` or `not_required`.
    #[serde(default)]
    pub auth_policy: String,
    /// Names of the groups the address belongs to.
    pub groups: Vec<String>,
    /// Number of messages received from the address.
    pub message_count: i64,
    /// Who writes its mails (`expediteurs.md`): the user's choice, else the
    /// computed verdict; `None` before any mail was counted.
    #[serde(default)]
    pub sender_kind: Option<SenderKind>,
    /// `sender_kind` was forced by the user.
    #[serde(default)]
    pub sender_kind_forced: bool,
}

/// An account, in `list_accounts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountView {
    pub id: i64,
    /// Key in `accounts.toml` (`[accounts.<key>]`), or name of the archive
    /// account.
    pub key: String,
    /// `imap` or `archive`.
    pub kind: String,
    /// Main address. `None` for a disabled IMAP account: its address was in
    /// the file, which no longer has it.
    pub email: Option<String>,
    pub display_name: Option<String>,
    /// `false` when the key of an IMAP account left `accounts.toml`: it is no
    /// longer synchronized, its mail is kept (`configuration.md`, section 6).
    pub enabled: bool,
    /// Why the account cannot connect, if known, in English. `None` does
    /// not mean connected, only that nothing prevents it.
    pub error: Option<String>,
    /// The same as a reason and its values (`docs/contract.md`, "Errors"),
    /// for a client to write it in the user's language.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<Value>,
    /// Display order: `list_accounts` and `list_folders` give the accounts
    /// by increasing position (then by id). Changed by `set_account_order`.
    pub position: i64,
    /// `read_only = true` in `accounts.toml`: nothing is changed on its
    /// server, until `activate_account`.
    #[serde(default)]
    pub read_only: bool,
    /// How the core signs in to an IMAP account of the file: `password`, or
    /// `oauth2` (a browser sign-in, `specifications/oauth2.md`). A client
    /// offers a new password for the first and "Sign in again" for the
    /// second. `None` for an archive, or an account no longer in the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<String>,
}

/// Params of `set_password`. The password is never logged nor stored
/// outside the Keychain. Answered with `CommandResult` (`changed`: 0);
/// the account's state is announced by `AccountChanged` or `AccountError`.
#[derive(Clone, Serialize, Deserialize)]
pub struct SetPasswordParams {
    /// Key of the account in `accounts.toml`.
    pub account: String,
    pub password: String,
    /// The password of the SMTP server, when it differs from the IMAP one
    /// (`reponse.md`, section 7); absent: the IMAP password.
    #[serde(default)]
    pub smtp: bool,
}

/// Rust note: `Debug` is written by hand rather than derived, so that
/// printing the params (in a log, a panic message) never shows the
/// password.
impl std::fmt::Debug for SetPasswordParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SetPasswordParams").field("account", &self.account).field("password", &"***").field("smtp", &self.smtp).finish()
    }
}

/// Params of `probe_account`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeAccountParams {
    /// Key of the account in `accounts.toml`.
    pub account: String,
}

/// What `probe_account` found on the server. Nothing is written: neither
/// the database nor the server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeReport {
    /// `host:port`, and how the connection is protected (`tls`,
    /// `starttls`, `none`).
    pub server: String,
    pub security: String,
    /// Capabilities announced before login, then after: only the second
    /// list counts for the synchronization.
    pub capabilities_before_login: Vec<String>,
    pub capabilities: Vec<String>,
    /// Extensions turned on by `ENABLE` (`CONDSTORE`, `QRESYNC`).
    pub enabled: Vec<String>,
    /// Prefix and separator of the personal namespace, if the server
    /// announces NAMESPACE.
    pub namespace: Option<ProbeNamespace>,
    pub folders: Vec<ProbeFolder>,
    /// Milliseconds to connect (TLS included), to log in, to list.
    pub connect_ms: u64,
    pub login_ms: u64,
    pub list_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeNamespace {
    pub prefix: String,
    pub delimiter: Option<String>,
}

/// One folder of the server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeFolder {
    /// Full name, decoded (`Envoyés/2026`).
    pub name: String,
    pub delimiter: Option<String>,
    /// Attributes as the server sent them (`\Sent`, `\HasNoChildren`).
    pub attributes: Vec<String>,
    /// `false` for a node of the hierarchy that holds no mail.
    pub selectable: bool,
    /// Workflow role NiceMailer would propose (`dossiers.md`, "A la
    /// création du compte"): `inbox`, `sent`, `drafts`, `deleted`, `junk`,
    /// `archive`… `None` when none fits.
    pub proposed_role: Option<String>,
    /// Why: `inbox`, `special-use` (the server's attribute) or `name`.
    pub role_from: Option<String>,
    /// From STATUS, when the server answered it.
    pub messages: Option<u32>,
    pub unseen: Option<u32>,
}

/// Params of `set_account_order`: these accounts first, in this order;
/// the others after them, in their current order. Answered with
/// `CommandResult` (`changed`: 0), and announced by `AccountChanged` and
/// `FolderChanged` for each account that moved and its folders.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetAccountOrderParams {
    pub account_ids: Vec<i64>,
}

/// A problem of `config.toml`, `accounts.toml` or `keymap.toml`: why it was
/// refused, or a warning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigProblem {
    /// The file: `config.toml`, `accounts.toml` or `keymap.toml`.
    #[serde(default)]
    pub file: String,
    /// Line, from 1.
    pub line: Option<u32>,
    /// Dotted key, e.g. `accounts.perso.imap.port`.
    pub key: Option<String>,
    pub message: String,
}

/// Result of `keymap_get`: the keys in effect (`raccourcis.md`, section 9).
///
/// Already resolved: the defaults with the file applied. Each key is in its
/// canonical spelling (`ctrl-shift-c`, `J`), which the app builds the same
/// way from a key press, so finding a key is comparing two strings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeymapView {
    /// Path of the `keymap.toml` read by the core, next to `config.toml`.
    pub path: String,
    /// Whether it exists; missing, every key is a default.
    pub exists: bool,
    /// The keys of each context (`mail`, `compose`…), in the order of the
    /// file. A key is looked for in the context, then in its parents; an
    /// action `none` hides the key of a parent.
    pub contexts: BTreeMap<String, Vec<KeyBinding>>,
    /// The menus, the built-in ones first, their rows in display order.
    pub menus: Vec<KeymapMenu>,
    /// Why the file on disk is refused; the keys of `contexts` are then the
    /// previous ones (or the defaults, at start).
    pub error: Option<ConfigProblem>,
    /// Lines ignored (unknown actions), and menus no key opens.
    pub warnings: Vec<ConfigProblem>,
}

/// Result of `theme_get`: the theme in effect, resolved (`themes.md`,
/// section 8). Every key of the catalog has its value: the app never reads
/// a file nor walks the inheritance itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThemeView {
    /// The theme applied: `ui.theme`, or `geek` when it names no theme.
    pub name: String,
    /// The chain of inheritance, from the theme to the built-in one it
    /// rests on: `["sepia", "clean"]`.
    pub chain: Vec<String>,
    /// Path of the directory of the themes, next to `config.toml`.
    pub path: String,
    pub window: ThemeWindow,
    pub fonts: ThemeFonts,
    pub layout: ThemeLayout,
    pub motion: ThemeMotion,
    /// Every color of the catalog, by its name (`background`,
    /// `accent_text`…).
    pub colors: BTreeMap<String, ThemeColor>,
    pub reader: ThemeReader,
    /// Why a file of the chain is refused: the theme is then made of its
    /// last valid version (`themes.md`, section 3).
    pub error: Option<ConfigProblem>,
    /// Unknown keys, a missing `extends`, a `ui.theme` naming no theme,
    /// and the colors below the contrast of WCAG AA (section 6.5).
    pub warnings: Vec<ConfigProblem>,
}

/// `[window]` of a theme (`themes.md`, section 6.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeWindow {
    /// `status_bar` or `toolbar`.
    pub bar_type: String,
    /// `top`, `bottom` or `hidden`.
    pub status_bar_position: String,
    /// `shown`, `hidden` or `merged`.
    pub title_bar: String,
    pub key_hints: bool,
    /// The blocks of actions say their name beside their icon.
    pub block_labels: bool,
    /// Height of the status bar or toolbar, in points for a 12 pt text of the bar.
    pub bar_height: u32,
    /// `small`, `medium` or `large`: the icons and texts of the bar.
    pub bar_size: String,
    /// `plain` or `pill`.
    pub block_style: String,
    /// A line between the status bar and the mails.
    pub bar_line: bool,
    /// The blocks shown, in order, with the separators `|` and `->`.
    pub blocks: Vec<String>,
    pub default_columns: u8,
    /// How the list shows each mail until the user chooses with `v` `l`:
    /// `columns` (one line per mail) or `stacked` (three lines: sender and
    /// date, subject, snippet).
    pub list_layout: String,
}

/// `[fonts]` of a theme (section 6.2). A font is `system`, `system-mono`,
/// `system-serif`, `system-rounded` or the name of a family.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThemeFonts {
    pub interface: String,
    pub reading: String,
    pub reading_scale: f64,
    /// Points added between two lines of the body, whatever the text size.
    pub reading_line_spacing: u32,
    pub title_scale: f64,
}

/// `[layout]` of a theme (section 6.3): lengths in points for a 12 pt text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeLayout {
    pub row_height: u32,
    pub row_radius: u32,
    pub row_inset: u32,
    pub zebra: bool,
    /// Height of a row of the menus and pickers; 0: the height of its text.
    pub menu_row_height: u32,
    pub popup_radius: u32,
    pub popup_shadow: bool,
    /// Margin inside menus, pickers and panels, in points.
    pub popup_padding: u32,
    /// `left` or `right`.
    pub menu_keys: String,
    pub message_cards: bool,
    pub avatars: bool,
    /// `tiles` (a grid of tiles with a symbol, geek) or `rows` (a row per
    /// file with the icon of macOS and the whole name).
    pub attachments: String,
    /// 0: the whole width.
    pub reader_max_width: u32,
    /// Room above and below the subject of the reader, in points, also
    /// added to the 12 pt on its sides; 0: none (geek).
    pub reader_title_padding: u32,
    /// `none`, `thin` or `standard`.
    pub scrollers: String,
    /// `lines` or `space`.
    pub separators: String,
}

/// `[reader]` of a theme (section 6.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeReader {
    /// `text` or `html`: how a received mail is shown when nothing is
    /// remembered for its sender (`address.body_format` NULL). A choice
    /// remembered for the sender wins.
    pub body_format: String,
    /// The smallest text of a mail in HTML, in points for a 12 pt text,
    /// grown with the text size as the whole page is; 0: no minimum.
    pub html_min_font_size: u32,
}

/// `[motion]` of a theme (section 6.4), in milliseconds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeMotion {
    pub sidebar_ms: u32,
    pub popup_ms: u32,
}

/// A color of a theme (section 5.1): a pair `#RRGGBB` or `#RRGGBBAA`, in
/// capitals, for the light and the dark appearance (the same twice when
/// the file gives one), or a color of macOS the app translates.
///
/// Rust note: `untagged` lets serde tell the two forms apart by their
/// fields, `{"light": …, "dark": …}` or `{"system": …}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ThemeColor {
    Pair { light: String, dark: String },
    System { system: String },
}

/// Result of `theme_list` (`themes.md`, section 8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeListView {
    /// Path of the directory of the themes.
    pub path: String,
    /// The theme chosen in `config.toml` (`ui.theme`).
    pub selected: String,
    /// The built-in themes first, then the others by name.
    pub themes: Vec<ThemeEntry>,
}

/// One theme of `ThemeListView`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeEntry {
    pub name: String,
    /// `builtin`, `builtin_modified` (a file of the same name modifies it)
    /// or `user`.
    pub origin: String,
    /// What it extends; `None` for a built-in theme.
    pub parent: Option<String>,
    /// Why its file, or one it extends, is refused; `None` when it can be
    /// used.
    pub error: Option<ConfigProblem>,
}

/// Parameters of `config_set_theme`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigSetThemeParams {
    /// The name of a theme `theme_list` gives.
    pub name: String,
}

/// Parameters of `config_set_compose_format`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigSetComposeFormatParams {
    /// `text` or `html`.
    pub format: String,
}

/// One key and its action, in `KeymapView`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyBinding {
    /// Canonical spelling: `ctrl-cmd-alt-shift-` then the key.
    pub key: String,
    /// The action (`mail.archive`), or `none`.
    pub action: String,
}

/// A menu of `KeymapView`: `[menu.<name>]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeymapMenu {
    pub name: String,
    /// Provided by the app (`move`, `goto`, `save`, `view`), or made by the
    /// user, who opens it with the action `menu.<name>`.
    pub builtin: bool,
    pub entries: Vec<KeyBinding>,
}

/// Result of `config_status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigStatus {
    /// Path of the `config.toml` read by the core.
    pub path: String,
    /// Whether it exists. A missing file is valid: every setting at its
    /// default. The core creates it at its start when it is missing, so
    /// it is missing only if it was deleted since, or could not be created.
    pub exists: bool,
    /// Path of the `accounts.toml` read by the core, next to `path`.
    #[serde(default)]
    pub accounts_path: String,
    /// Whether it exists; missing, it means no account.
    #[serde(default)]
    pub accounts_exists: bool,
    /// Why the current files are refused; the previous settings (or the
    /// defaults, at start) stay applied. `None` when they are applied.
    pub error: Option<ConfigProblem>,
    /// Warnings about the files applied: unknown keys, and keys clashing
    /// with an archive account.
    pub warnings: Vec<ConfigProblem>,
    /// The settings of `[ui]` the app applies itself.
    #[serde(default)]
    pub ui: UiConfig,
    /// The settings of `[mcp]` the MCP server applies itself.
    #[serde(default)]
    pub mcp: McpConfig,
    /// `[unsubscribe]`: the unsubscribe of the mails of Banned. Off when
    /// absent, for a client reading an older core.
    #[serde(default)]
    pub unsubscribe: UnsubscribeConfig,
    /// `[folder_names]`: the name the app shows for each folder of the
    /// workflow, by role id (`inbox`, `waiting`…), and for the virtual
    /// folders of the app (`work`, `all_mail`, `everything`): always these
    /// 15, English unless the file names it (`configuration.md`, "Noms des
    /// dossiers"). For display only: `FolderView.name`, the CLI and the MCP
    /// agent keep the English names. Empty from an older core: the client
    /// then shows its own English names.
    #[serde(default)]
    pub folder_names: BTreeMap<String, String>,
    pub as_of: Position,
}

/// The settings of `[mcp]` of `config.toml` (`mcp.md`, section 11): the
/// bounds of the answers of `nicemailer-mcp`, which reads them with
/// `config_status`. Raising them gives the agent longer answers, never a
/// right.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpConfig {
    /// Characters of one message in `get_conversation`.
    pub message_chars: u64,
    /// Characters of the text of a whole answer.
    pub response_chars: u64,
}

impl Default for McpConfig {
    fn default() -> Self {
        McpConfig { message_chars: 8_000, response_chars: 40_000 }
    }
}

/// The settings of `[ui]` of `config.toml` the app applies itself
/// (`docs/geek-ui.md`): sent with `config_status`, read again after
/// `ConfigChanged`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiConfig {
    /// `ui.title_bar`, `shown` or `hidden`, only when the file writes it:
    /// replaced by `window.title_bar` of the theme (`themes.md`, section 9),
    /// it still wins while it is written. `None` (absent): the theme's.
    #[serde(default)]
    pub title_bar: Option<String>,
    /// `ui.theme`: the name of the theme chosen (`geek` by default).
    #[serde(default)]
    pub theme: String,
    /// `ui.auto_confirm_calls`: the app confirms its calls itself
    /// (`docs/geek-ui.md`, "Calling a number"). `false` when absent, for a
    /// client reading an older core.
    #[serde(default)]
    pub auto_confirm_calls: bool,
    /// `ui.text_size`: the text size of the app, in points, except the
    /// status bar (`docs/geek-ui.md`, "Text size"). `None` when absent, for
    /// a client reading an older core: it keeps its own default.
    #[serde(default)]
    pub text_size: Option<u64>,
    /// `ui.status_bar_text_size`: the text size of the status bar.
    #[serde(default)]
    pub status_bar_text_size: Option<u64>,
}

/// `[unsubscribe]` of `config.toml`, sent with `config_status`: the app
/// shows the unsubscribe button of the mails of Banned only when
/// `in_banned` is on (`ui.md`, "Désabonnement"), as the core accepts it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnsubscribeConfig {
    pub in_banned: bool,
}

/// Why the core is in maintenance (`mise-a-jour.md`, section 2).
///
/// Rust note: `rename_all = "snake_case"` writes each variant in JSON as
/// `update`, `migration`, `newer_schema` and `migration_failed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceReason {
    /// An update of NiceMailer is being installed (`prepare_update`).
    Update,
    /// The database is being upgraded to the schema of this core.
    Migration,
    /// The database was written by a newer NiceMailer: this core does not
    /// touch it, and stays in maintenance until it is replaced.
    NewerSchema,
    /// The migration failed: the database was put back as it was before,
    /// and the core stays in maintenance until it is restarted.
    MigrationFailed,
}

/// The `data` of the error `MAINTENANCE`: why the core serves no client,
/// and for a migration, how far it is. The core answers it to every
/// handshake while in maintenance, and sends it to the connections open
/// when the maintenance begins, before closing them (`mise-a-jour.md`,
/// section 2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Maintenance {
    pub reason: MaintenanceReason,
    /// `migration`, `migration_failed`: the schema version of the database
    /// before the migration. `newer_schema`: the version of the database.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<i64>,
    /// `migration`, `migration_failed`, `newer_schema`: the schema version
    /// of this core.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<i64>,
    /// `migration`: the version the database has reached so far (`from`
    /// at first, one more after each step), the progress shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<i64>,
    /// When the maintenance began, in seconds since 1970.
    pub since: u64,
    /// `migration_failed`: the error, as the core logged it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl Maintenance {
    /// The error answered to a client: code `MAINTENANCE`, this value as
    /// its `data`, and a message for logs.
    pub fn error(&self) -> RpcError {
        // The message comes from the reason's template, as for any error;
        // `data` stays this whole value, whose `reason` field is the
        // reason's name.
        let why = Why::from(match self.reason {
            MaintenanceReason::Update => reason::UPDATE,
            MaintenanceReason::Migration => reason::MIGRATION,
            MaintenanceReason::NewerSchema => reason::NEWER_SCHEMA,
            MaintenanceReason::MigrationFailed => reason::MIGRATION_FAILED,
        })
        .with("from", self.from.unwrap_or(0))
        .with("to", self.to.unwrap_or(0))
        .with("message", self.message.clone().unwrap_or_else(|| "unknown error".into()));
        RpcError {
            code: error_code::MAINTENANCE,
            message: why.message(),
            data: Some(serde_json::to_value(self).expect("plain struct")),
        }
    }

    /// Whether the maintenance ends by itself (an update, a migration), so
    /// that a client waits; `newer_schema` and `migration_failed` need the
    /// user (another version of NiceMailer, a restart).
    pub fn is_transient(&self) -> bool {
        matches!(self.reason, MaintenanceReason::Update | MaintenanceReason::Migration)
    }
}

/// The answer of `prepare_update`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrepareUpdateResult {
    /// After this many seconds without being replaced or `cancel_update`,
    /// the core leaves the maintenance by itself
    /// (`core.update_timeout_secs`).
    pub timeout_secs: u64,
}

/// The answer of `cancel_update`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelUpdateResult {
    /// Whether an update was being prepared (false: nothing to cancel).
    pub cancelled: bool,
}

/// Why the core could not run on a data directory: written by the core to
/// `paths::STARTUP_ERROR` when it stops on an error, read by the app when
/// no core answers (`docs/app-bundle.md`, "When the core cannot start").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartupError {
    /// `outdated` (an early development database, which cannot be
    /// upgraded), `too_new` (written by a newer NiceMailer), or `other`.
    pub kind: String,
    /// The error and its causes, as the core logged it (in French, the
    /// language of the core's messages): what the app shows for `other`.
    pub message: String,
    /// `outdated`: what the database lacks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing: Option<String>,
    /// `too_new`: the schema version of the database, and this core's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub found: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<i64>,
    /// When, in seconds since 1970.
    pub at: u64,
}

/// A plain list with the event position of the read.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Items<T> {
    pub items: Vec<T>,
    pub as_of: Position,
}

pub mod paths {
    // `PathBuf` is an owned file system path (like `String` for text).
    use std::path::{Path, PathBuf};

    /// The default data directory: `~/Library/Application Support/NiceMailer`.
    pub fn default_data_dir() -> PathBuf {
        // `map` transforms the value inside the Option if there is one;
        // `unwrap_or_else` provides a fallback when there is none.
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
        home.join("Library/Application Support/NiceMailer")
    }

    /// File of the default data directory naming the data directory chosen
    /// in the app ("Change Mailbox…", like Photos' libraries,
    /// `docs/app-bundle.md`): one line, an absolute path. Absent: the
    /// default directory itself.
    pub const CHOICE_FILE: &str = "data-dir";

    /// Data directory of the core and its clients, in this order:
    /// 1. `NICEMAILER_DATA_DIR`, when set (development, tests): it always
    ///    wins, so that a test never uses the mailbox chosen in the app;
    /// 2. the directory chosen in the app, named by [`CHOICE_FILE`] in the
    ///    default directory: the core started by launchd has no variable,
    ///    and the CLI follows the same choice;
    /// 3. the default directory.
    pub fn data_dir() -> PathBuf {
        let default = default_data_dir();
        let chosen = match choice(&default) {
            Choice::Chosen(c) => Some(c),
            Choice::Absent | Choice::Ignored(_) => None,
        };
        resolve(std::env::var_os("NICEMAILER_DATA_DIR").map(PathBuf::from), chosen.as_deref(), default)
    }

    /// What [`CHOICE_FILE`] in the default directory says.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum Choice {
        /// No file: the default directory.
        Absent,
        /// A file that is not trusted, and why: the default directory, and
        /// the core says so in its log.
        Ignored(String),
        /// The text of the file, trusted.
        Chosen(String),
    }

    /// The choice of [`CHOICE_FILE`] in `default`, trusted only when no one
    /// but the current user can have written it (`architecture.md`,
    /// section 9): the core started by launchd follows this file, and one
    /// written by another account would point it at a mailbox of that
    /// account's choosing. So the file must be a regular file — never a
    /// symbolic link, which is not followed — owned by the current user,
    /// writable by her alone, and so must its directory. That others may
    /// read it is harmless: it names a path, not a secret (and older
    /// versions of the app wrote it `0644`).
    ///
    /// The file is opened first (`O_NOFOLLOW`), then checked on the open
    /// file (`fstat`): checking the path and then reading it would leave a
    /// moment when the file could be swapped in between.
    ///
    /// The protection is against other accounts of the Mac: a program of
    /// the same user can write a file that passes, as it can do anything
    /// the user can (`architecture.md`, section 9, "Menaces couvertes et
    /// non couvertes").
    pub fn choice(default: &Path) -> Choice {
        use std::io::Read;
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        // The directory first: whoever can write in it can replace the file.
        match std::fs::symlink_metadata(default) {
            Ok(dir) if !dir.is_dir() => return Choice::Ignored(format!("{} is not a directory", default.display())),
            Ok(dir) => {
                if let Some(why) = untrusted(dir.uid(), dir.mode()) {
                    return Choice::Ignored(format!("{}: {why}", default.display()));
                }
            }
            Err(_) => return Choice::Absent,
        }
        let path = default.join(CHOICE_FILE);
        // Rust note: `custom_flags` passes flags of `open(2)` that the
        // portable API does not name; `O_NOFOLLOW` makes the open fail on a
        // symbolic link instead of following it.
        let mut file = match std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW).open(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Choice::Absent,
            Err(e) => return Choice::Ignored(format!("{}: {e}", path.display())),
        };
        // `metadata` on an open file is `fstat`: the file just opened.
        let Ok(meta) = file.metadata() else { return Choice::Ignored(format!("{}: unreadable", path.display())) };
        if !meta.is_file() {
            return Choice::Ignored(format!("{} is not a regular file", path.display()));
        }
        if let Some(why) = untrusted(meta.uid(), meta.mode()) {
            return Choice::Ignored(format!("{}: {why}", path.display()));
        }
        // One line, an absolute path: a few hundred bytes. More is not a
        // choice the app wrote.
        let mut text = String::new();
        match (&mut file).take(MAX_CHOICE_BYTES + 1).read_to_string(&mut text) {
            Ok(n) if n as u64 <= MAX_CHOICE_BYTES => Choice::Chosen(text),
            Ok(_) => Choice::Ignored(format!("{}: longer than {MAX_CHOICE_BYTES} bytes", path.display())),
            Err(e) => Choice::Ignored(format!("{}: {e}", path.display())),
        }
    }

    /// The longest choice file read.
    const MAX_CHOICE_BYTES: u64 = 4096;

    /// Why a file or directory of owner `uid` and permissions `mode` could
    /// have been written by someone else than the current user: owned by
    /// another, or writable by the group or the others (`mode & 0o022`).
    /// `None` when only the current user can write it.
    pub fn untrusted(uid: u32, mode: u32) -> Option<&'static str> {
        // `geteuid` is a C call, hence `unsafe`; it has no precondition and
        // cannot fail.
        if uid != unsafe { libc::geteuid() } {
            return Some("owned by another user");
        }
        if mode & 0o022 != 0 {
            return Some("writable by the group or the others");
        }
        None
    }

    /// The rule of [`data_dir`], as a pure function: the variable, else the
    /// choice (an absolute path, spaces around it ignored), else the
    /// default. A choice that is empty or relative is ignored.
    pub fn resolve(env: Option<PathBuf>, choice: Option<&str>, default: PathBuf) -> PathBuf {
        // `if let Some(x) = ...` runs the block only when the Option holds
        // a value, and binds it to `x`.
        if let Some(dir) = env.filter(|d| !d.as_os_str().is_empty()) {
            return dir;
        }
        match choice.map(str::trim) {
            Some(c) if c.starts_with('/') => PathBuf::from(c),
            _ => default,
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn the_variable_then_the_choice_then_the_default() {
            let d = PathBuf::from("/home/Library/Application Support/NiceMailer");
            let env = Some(PathBuf::from("/tmp/test"));
            assert_eq!(resolve(env.clone(), Some("/Users/me/Archive"), d.clone()), PathBuf::from("/tmp/test"));
            assert_eq!(resolve(None, Some("  /Users/me/Archive\n"), d.clone()), PathBuf::from("/Users/me/Archive"));
            assert_eq!(resolve(None, None, d.clone()), d);
            // A broken choice is ignored rather than followed.
            assert_eq!(resolve(None, Some(""), d.clone()), d);
            assert_eq!(resolve(None, Some("relative/dir"), d.clone()), d);
            assert_eq!(resolve(Some(PathBuf::new()), None, d.clone()), d);
        }

        /// The choice file is followed only when no one but the current
        /// user can have written it: `0644` (the app's own, readable) is
        /// fine, writable by the group or the others is not.
        #[test]
        fn the_choice_file_must_be_writable_by_the_user_alone() {
            use std::os::unix::fs::PermissionsExt;
            let dir = tempfile::tempdir().unwrap();
            std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
            let file = dir.path().join(CHOICE_FILE);
            // Absent: nothing ignored, nothing read.
            assert_eq!(choice(dir.path()), Choice::Absent);
            std::fs::write(&file, "/tmp/elsewhere\n").unwrap();
            for mode in [0o600, 0o644, 0o400] {
                std::fs::set_permissions(&file, std::fs::Permissions::from_mode(mode)).unwrap();
                assert_eq!(choice(dir.path()), Choice::Chosen("/tmp/elsewhere\n".into()), "{mode:o}");
            }
            for mode in [0o620, 0o602, 0o666] {
                std::fs::set_permissions(&file, std::fs::Permissions::from_mode(mode)).unwrap();
                assert!(matches!(choice(dir.path()), Choice::Ignored(_)), "{mode:o}");
            }
        }

        /// A directory others can write in: whoever can replace the file.
        #[test]
        fn a_choice_in_a_directory_others_can_write_is_ignored() {
            use std::os::unix::fs::PermissionsExt;
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join(CHOICE_FILE), "/tmp/elsewhere").unwrap();
            std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o777)).unwrap();
            assert!(matches!(choice(dir.path()), Choice::Ignored(why) if why.contains("writable")));
            std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
            assert_eq!(choice(dir.path()), Choice::Chosen("/tmp/elsewhere".into()));
        }

        /// A symbolic link is never followed, even towards a good file; a
        /// directory, or a file too long, is not a choice.
        #[test]
        fn a_link_or_an_odd_file_is_not_a_choice() {
            use std::os::unix::fs::PermissionsExt;
            let dir = tempfile::tempdir().unwrap();
            std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
            let target = dir.path().join("target");
            std::fs::write(&target, "/tmp/elsewhere").unwrap();
            std::os::unix::fs::symlink(&target, dir.path().join(CHOICE_FILE)).unwrap();
            assert!(matches!(choice(dir.path()), Choice::Ignored(_)));
            std::fs::remove_file(dir.path().join(CHOICE_FILE)).unwrap();
            std::fs::create_dir(dir.path().join(CHOICE_FILE)).unwrap();
            assert!(matches!(choice(dir.path()), Choice::Ignored(_)));
            std::fs::remove_dir(dir.path().join(CHOICE_FILE)).unwrap();
            std::fs::write(dir.path().join(CHOICE_FILE), "/".repeat(5000)).unwrap();
            assert!(matches!(choice(dir.path()), Choice::Ignored(why) if why.contains("longer")));
        }

        #[test]
        fn only_the_user_writing_alone_is_trusted() {
            let me = unsafe { libc::geteuid() };
            assert_eq!(untrusted(me, 0o100644), None);
            assert_eq!(untrusted(me, 0o40700), None);
            assert!(untrusted(me, 0o100664).is_some());
            assert!(untrusted(me, 0o100646).is_some());
            assert!(untrusted(me + 1, 0o100600).is_some());
        }
    }

    /// SQLite database file name, inside the data directory.
    pub const DATABASE: &str = "nicemailer.db";
    /// Unix socket file name, inside the data directory.
    pub const SOCKET: &str = "core.sock";
    /// Why the core could not start or stopped on an error
    /// (`StartupError`, JSON), inside the data directory. Removed once a
    /// core runs there.
    pub const STARTUP_ERROR: &str = "core-error.json";
}

// ---------------------------------------------------------------------------
// Synchronization (`plan-imap.md`, phase 4)
// ---------------------------------------------------------------------------

/// Params of `sync_status` and `sync_now`: one account (its key in
/// `accounts.toml`), or every account.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncAccountParams {
    #[serde(default)]
    pub account: Option<String>,
}

/// Params of `sync_verify`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncVerifyParams {
    pub account: String,
    /// Skip reading every raw file again (fast, but checks only that the
    /// file exists).
    #[serde(default)]
    pub skip_hash: bool,
    /// Set aside the raw files found corrupt or missing, and download
    /// their mails again from the server.
    #[serde(default)]
    pub repair: bool,
}

/// Result of `sync_status`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncStatus {
    pub accounts: Vec<AccountSync>,
}

/// The synchronization of one IMAP account.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountSync {
    pub account_id: i64,
    pub key: String,
    /// `read_only = true`: nothing is changed on the server.
    pub read_only: bool,
    /// `connecting`, `syncing`, `idle` (waiting for new mail on the
    /// Inbox), `waiting` (until the next attempt after an error),
    /// `auth_failed` (stopped until the password or the file changes),
    /// `stopped` (no password, or disabled).
    pub state: String,
    pub error: Option<String>,
    /// How the server is followed: `qresync`, `condstore` or `none` (full
    /// comparisons), once connected.
    pub mode: Option<String>,
    /// How the connection is kept between passes, once the server cut one
    /// left idle (`docs/sync.md`, "Pace of the connections"): `renew N`
    /// (IDLE renewed every N seconds, one connection only) or `poll N`
    /// (a connection per pass, every N seconds at most). `None`: IDLE as
    /// usual.
    #[serde(default)]
    pub hold: Option<String>,
    /// Logins per hour at most, learnt from a block of the server
    /// (`docs/sync.md`, "Pace of the connections"); `None`: no limit, the
    /// server never blocked a login (or not for 30 days).
    #[serde(default)]
    pub login_limit: Option<u32>,
    /// End of the last complete pass, Unix seconds.
    pub last_pass: Option<i64>,
    /// Passes completed since the core started: a client (or a test) that
    /// asked `sync_now` knows the pass it waits for is done when this
    /// grows.
    pub passes: u64,
    /// Next attempt after an error, Unix seconds.
    pub next_retry: Option<i64>,
    /// Messages still without body.
    pub bodies_missing: u64,
    /// Changes made in NiceMailer not sent to the server yet (spool).
    pub actions_pending: u64,
    /// The server-side archiving of the initial import, if there was one:
    /// `queued`, `running`, `done`, or `reverted` (failed for good), with
    /// its error (`import-initial.md`, section 6).
    pub initial_archive: Option<String>,
    pub initial_archive_error: Option<String>,
    /// The `authserv-id` whose `Authentication-Results` are trusted: the
    /// file's, else the one detected; `None`: every verdict is `unknown`
    /// (`authentification-expediteur.md`).
    pub authserv_id: Option<String>,
    /// Bodies refused this run: their size differs from what the server
    /// announced (`stockage-brut.md`, section 5, step 2). Tried again at
    /// the next start of the core.
    pub bodies_refused: u64,
    pub folders: Vec<FolderSync>,
}

/// One folder of an account, as synchronized.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderSync {
    pub folder_id: i64,
    pub path: String,
    pub role: String,
    pub messages: u64,
    pub bodies_missing: u64,
    /// End of its last synchronization, Unix seconds.
    pub last_sync: Option<i64>,
    /// The server no longer lists it: kept with its mail, not synchronized.
    pub missing_on_server: bool,
}

/// Result of `sync_verify`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SyncVerifyReport {
    pub account: String,
    /// No difference anywhere.
    pub ok: bool,
    pub folders: Vec<FolderVerify>,
    pub seconds: f64,
    /// With `repair`: mails whose damaged raw file was set aside, to be
    /// downloaded again.
    pub repaired: u64,
}

/// The comparison of one folder. UID lists are cut at 20 entries; the
/// counts are complete.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderVerify {
    pub path: String,
    pub role: String,
    pub server_messages: u64,
    pub local_messages: u64,
    /// UIDs on the server, not in the database.
    pub missing_locally: Vec<u32>,
    pub missing_locally_count: u64,
    /// UIDs in the database, no longer on the server.
    pub extra_locally: Vec<u32>,
    pub extra_locally_count: u64,
    /// UIDs whose flags or keywords differ.
    pub flags_differ: Vec<u32>,
    pub flags_differ_count: u64,
    /// UIDVALIDITY of the server, if it differs from the database's.
    pub uidvalidity_differs: Option<u32>,
    /// Complete messages whose raw file was checked.
    pub bodies_checked: u64,
    /// Messages still without body (not a difference: still downloading).
    pub bodies_missing: u64,
    /// Raw files missing or corrupt, as `uid: problem`.
    pub raw_problems: Vec<String>,
}

// ---------------------------------------------------------------------------
// Creating an account (`plan-imap.md`, phase 7)
// ---------------------------------------------------------------------------

/// The server of an account, as `accounts.toml` will say it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImapParams {
    pub host: String,
    #[serde(default = "default_imap_port")]
    pub port: u16,
    /// `tls` (default), `starttls`, or `none` (loopback only).
    #[serde(default = "default_security")]
    pub security: String,
    /// Default: the email address.
    #[serde(default)]
    pub username: Option<String>,
    /// An authority to trust besides the system's (self-hosted server).
    #[serde(default)]
    pub ca_file: Option<String>,
}

fn default_imap_port() -> u16 {
    993
}

fn default_security() -> String {
    "tls".into()
}

/// A finished OAuth2 flow, given to `test_account`, `test_smtp` and
/// `add_account` in place of a password (`specifications/oauth2.md`,
/// section 8): the tokens stay in the core, attached to the flow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OauthFlow {
    pub flow: String,
}

/// Params of `oauth_begin`: the address of the account to create, or the
/// key of an existing account to sign in again. Exactly one of the two.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OauthBeginParams {
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub account: Option<String>,
}

/// What `oauth_begin` started: the flow's id, the authorization URL to
/// open in the default browser, and the provider (`microsoft`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OauthBeginResult {
    pub flow: String,
    pub url: String,
    pub provider: String,
}

/// Params of `oauth_cancel`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OauthCancelParams {
    pub flow: String,
}

/// Params of `test_account`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestAccountParams {
    pub email: String,
    pub imap: ImapParams,
    /// Ignored when `oauth` is given: the tokens of the flow are used.
    pub password: String,
    /// The finished OAuth2 flow to use instead of `password`.
    #[serde(default)]
    pub oauth: Option<OauthFlow>,
    /// Other addresses of the account (identities), excluded from the
    /// correspondents.
    #[serde(default)]
    pub identities: Vec<String>,
}

/// What `test_account` found: the probe of the server, and the counts of
/// the import screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestAccountReport {
    pub probe: ProbeReport,
    pub inbox_messages: u64,
    pub inbox_unseen: u64,
    /// The boundary of the screening: mails at or above it are screened.
    /// Given back to `add_account`, so that a mail arriving while the user
    /// fills the import screen is screened (`import-initial.md`, section 5).
    pub inbox_uidnext: u32,
    /// Distinct recipients of the mails of Sent, the account's own
    /// addresses excluded.
    pub correspondents: u64,
    /// The Inbox is above `import.threshold_total` or
    /// `import.threshold_unseen`: the archiving is offered.
    pub offer_archive: bool,
    /// Defaults of the screen, `YYYY-MM-DD`: today for read mail, 30 days
    /// ago (`import.unseen_default_age_days`) for unread mail.
    pub read_before: String,
    pub unread_before: String,
    /// What the defaults would archive.
    pub archive_read: u64,
    pub archive_unread: u64,
    /// The folder each workflow role would get, in sidebar order, Inbox
    /// first: what `add_account` does without `roles` (`dossiers.md`, "A
    /// la création du compte IMAP dans niceMailer").
    #[serde(default)]
    pub roles: Vec<RolePlan>,
}

/// The folder of a workflow role, as planned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RolePlan {
    pub role: FolderRole,
    pub path: String,
    /// `false`: the server has no such folder; it would be created.
    pub exists: bool,
}

/// Params of `prepare_activation`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrepareActivationParams {
    /// Key of a read-only account.
    pub account: String,
}

/// Params of `activate_account`: the choices of the import screen, as for
/// `add_account`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivateAccountParams {
    /// Key of a read-only account.
    pub account: String,
    /// As in `add_account`: roles left out get the proposal, else a new
    /// folder.
    #[serde(default)]
    pub roles: BTreeMap<FolderRole, String>,
    /// The boundary read by `prepare_activation`; omitted: read again now.
    /// The mails below it, those of the time the account was only looked
    /// at included, are never screened (`import-initial.md`, section 5).
    #[serde(default)]
    pub uidnext: Option<u32>,
    #[serde(default)]
    pub archive_read_before: Option<String>,
    #[serde(default)]
    pub archive_unread_before: Option<String>,
    #[serde(default = "yes")]
    pub correspondents: bool,
}

/// What `activate_account` did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivateAccountResult {
    /// Folders created on the server.
    pub created: Vec<String>,
    pub roles: BTreeMap<FolderRole, String>,
    pub correspondents: u64,
    pub archive_queued: bool,
}

/// Params of `count_archive`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CountArchiveParams {
    pub email: String,
    pub imap: ImapParams,
    /// Ignored when `oauth` is given: the tokens of the flow are used.
    pub password: String,
    /// The finished OAuth2 flow to use instead of `password`.
    #[serde(default)]
    pub oauth: Option<OauthFlow>,
    pub uidnext: u32,
    #[serde(default)]
    pub read_before: Option<String>,
    #[serde(default)]
    pub unread_before: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveCount {
    pub read: u64,
    pub unread: u64,
}

/// Params of `add_account`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddAccountParams {
    /// The key of `[accounts.<key>]`.
    pub key: String,
    pub email: String,
    #[serde(default)]
    pub display_name: Option<String>,
    pub imap: ImapParams,
    /// Ignored when `oauth` is given: the tokens of the flow are saved in
    /// the Keychain by `add_account`.
    pub password: String,
    /// The finished OAuth2 flow to use instead of `password`.
    #[serde(default)]
    pub oauth: Option<OauthFlow>,
    /// The folder of each workflow role: a folder of the server, or a new
    /// one to create. Roles left out get the proposal (special-use, then
    /// names); a proposed role without folder gets one created under its
    /// English name.
    #[serde(default)]
    pub roles: BTreeMap<FolderRole, String>,
    /// The boundary read by `test_account`; omitted: read again now.
    #[serde(default)]
    pub uidnext: Option<u32>,
    /// Archive read mails before this day, unread mails before that one
    /// (`YYYY-MM-DD`); none: no archiving.
    #[serde(default)]
    pub archive_read_before: Option<String>,
    #[serde(default)]
    pub archive_unread_before: Option<String>,
    /// The people written to get the decision `inbox` (default yes).
    #[serde(default = "yes")]
    pub correspondents: bool,
    /// Never change anything on the server (`read_only = true`): no folder
    /// created, no archiving, screening dry.
    #[serde(default)]
    pub read_only: bool,
    /// The SMTP server, written to `[accounts.<key>.smtp]`; none: the
    /// account cannot send until one is added.
    #[serde(default)]
    pub smtp: Option<ImapParams>,
    /// The password of that SMTP server, when it is not the IMAP one:
    /// saved in the Keychain (`smtp:<key>`, bound to the SMTP server), as
    /// `set_password` with `smtp` would. Refused without `smtp`.
    #[serde(default)]
    pub smtp_password: Option<String>,
}

fn yes() -> bool {
    true
}

/// What `add_account` did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddAccountResult {
    pub account_id: i64,
    /// Folders created on the server.
    pub created: Vec<String>,
    /// Role of each folder, as set.
    pub roles: BTreeMap<FolderRole, String>,
    pub correspondents: u64,
    /// The server-side archiving is queued; `sync status` follows it.
    pub archive_queued: bool,
}

/// Params of `discover_account`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoverAccountParams {
    pub email: String,
    /// The IMAP server the user gave (`--host`); none: it is looked for.
    #[serde(default)]
    pub imap: Option<ImapParams>,
}

/// What `discover_account` found, and how: every step is reported, so
/// that the client can say exactly what was done (nothing is guessed
/// silently). Nothing is written, no server is contacted but DNS.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryReport {
    /// The account key proposed (`docs/accounts.md`, "The key"): the name
    /// of the domain for its first account (`pierre.dupont@toto-mail.com`:
    /// `toto-mail`), else the whole address
    /// (`pierre_dupont_toto_mail_com`).
    pub key: String,
    /// How `key` was made: `domain` or `address`.
    pub key_source: String,
    /// The name of the domain, the key of a first account of it.
    pub domain_key: String,
    /// The key of an account of the same domain, if one exists: why the
    /// whole address is used. `key_source` is `address` without it when
    /// `domain_key` is the key of an account of another domain.
    pub domain_account: Option<String>,
    /// The domain of the address, lower case.
    pub domain: String,
    /// The provider listing the domain itself, if any.
    pub by_domain: Option<ProviderMatch>,
    /// The MX servers of the domain, by preference; looked up only when the
    /// domain is not listed. Empty when there is none, or when not looked up.
    pub mx: Vec<String>,
    /// Whether the MX were looked up.
    pub mx_looked_up: bool,
    /// The DNS failure, if the lookup failed.
    pub mx_error: Option<String>,
    /// The provider recognized from an MX server, if any.
    pub by_mx: Option<ProviderMatch>,
    /// The IMAP server to use: the one given, else the provider's. None:
    /// nothing found, the user must give it.
    pub imap: Option<ServerCandidate>,
    /// The SMTP servers to try, in order (`test_smtp`).
    pub smtp: Vec<ServerCandidate>,
    /// How the account authenticates: `password` (default) or `oauth2`
    /// (`specifications/oauth2.md`).
    #[serde(default = "default_auth_password")]
    pub auth: String,
    /// The OAuth2 provider of the account, when `auth` is `oauth2`
    /// (`microsoft`).
    #[serde(default)]
    pub oauth_provider: Option<String>,
}

fn default_auth_password() -> String {
    "password".to_string()
}

/// A provider of the database (`data/providers/`), and why it matched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderMatch {
    /// Its file name (`infomaniak`, `googlemail.com`).
    pub id: String,
    pub name: String,
    /// `local` (data/providers/local/, ours) or `thunderbird`.
    pub source: String,
    /// The domain found in its list: the address's domain, or the domain
    /// of the MX server.
    pub matched: String,
    /// The MX server that led to it, when found by MX.
    pub mx_host: Option<String>,
    /// Its IMAP servers need a login NiceMailer does not support (OAuth2
    /// only): it cannot be used.
    pub imap_unsupported: bool,
}

/// A server to use or to try.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerCandidate {
    pub server: ImapParams,
    /// Where it comes from: `given` (the user), `provider` (the database),
    /// `smtp_name` (the IMAP host with `imap` replaced by `smtp`),
    /// `same_host` (the IMAP host itself). The last two are guesses, made
    /// for a server the user gave, or for a provider that lists no SMTP
    /// server.
    pub origin: String,
}

/// Params of `test_smtp`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestSmtpParams {
    pub email: String,
    /// The IMAP server of the account: its password is tried only on an
    /// SMTP server of the same host or organization.
    pub imap: ImapParams,
    /// Ignored when `oauth` is given: the flow's access token is used.
    pub password: String,
    /// The finished OAuth2 flow to use instead of `password`.
    #[serde(default)]
    pub oauth: Option<OauthFlow>,
    /// The password of the SMTP server itself, when the user gave one (the
    /// server refused the IMAP password, or belongs to another
    /// organization): tried on every candidate, whatever its organization.
    #[serde(default)]
    pub smtp_password: Option<String>,
    /// Tried in this order; the first that works is chosen.
    pub candidates: Vec<ServerCandidate>,
}

/// What `test_smtp` did: each attempt, and the server chosen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmtpTestReport {
    pub attempts: Vec<SmtpAttempt>,
    /// Index in `attempts` of the server chosen, if one worked.
    pub chosen: Option<usize>,
    /// When none worked: index of the first server that refused the
    /// password (`auth_refused`). It is the right server, short of its own
    /// password: a client asks for it and tries again with `smtp_password`.
    #[serde(default)]
    pub refused: Option<usize>,
}

/// One SMTP server tried.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmtpAttempt {
    pub candidate: ServerCandidate,
    /// `logged_in`: the password was accepted; `auth_refused`: the server
    /// refused it (it needs a password of its own); `reachable`: the server
    /// answered, the password was not tried (another organization than the
    /// IMAP server: it will need its own password); `no_login`: the server
    /// offers no login (a relay, not a submission server); `failed`.
    pub outcome: String,
    /// The server's answer or the network error, for `failed`.
    pub detail: Option<String>,
}

/// Params of `set_folder_roles`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetFolderRolesParams {
    pub account: String,
    pub roles: BTreeMap<FolderRole, String>,
    /// Create on the server the folders named that do not exist.
    #[serde(default)]
    pub create_missing: bool,
}

/// Params of `rename_account`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenameAccountParams {
    pub from: String,
    pub to: String,
}

/// Params of `remove_account`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoveAccountParams {
    pub account: String,
}

/// Result of `raw_gc`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawGcResult {
    /// Raw files no mail referenced, deleted.
    pub blobs: u64,
    /// Temporary files older than a day.
    pub temporary: u64,
    /// Files without row older than `raw.gc_grace_secs`.
    pub orphans: u64,
    pub attachment_dirs: u64,
    /// Files of `outgoing.noindex` no draft and no mail waiting to leave
    /// referred to, deleted.
    #[serde(default)]
    pub outgoing: u64,
}

// ---------------------------------------------------------------------------
// AI assistance (`specifications/ia.md`, section 10)
// ---------------------------------------------------------------------------

/// Params of `ai_status`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AiStatusParams {
    /// Asks the provider itself (about 0.3 s for the `claude` binary, no
    /// call to the model): its version and the subscription.
    #[serde(default)]
    pub check: bool,
}

/// Result of `ai_status`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AiStatusResult {
    /// `ai.enabled`.
    pub enabled: bool,
    /// `claude-code` or `anthropic-api`.
    pub provider: String,
    pub model: String,
    /// Whether a run can start now; `reason` says why not.
    pub ready: bool,
    pub reason: Option<String>,
    /// With `check`: the provider's version and the account's subscription
    /// ("Claude Pro").
    pub version: Option<String>,
    pub subscription: Option<String>,
    /// The prompt files refused, and their warnings, as "file, line N,
    /// key: message".
    pub prompt_problems: Vec<String>,
    pub prompt_warnings: Vec<String>,
}

/// Params of `ai_list_prompts`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiListPromptsParams {
    /// `compose` (the reply written), `mail` (one mail: shown, marked
    /// alone, or under the cursor), `selection` (several mails marked) or
    /// `attachment` (`ia.md`, section 11, "Contexte").
    pub screen: String,
    /// The role of the folder shown; absent for the compose screen.
    #[serde(default)]
    pub folder: Option<FolderRole>,
    /// The app's language, for the labels (`en`, `fr`).
    #[serde(default)]
    pub lang: Option<String>,
    /// For the compose screen: whether the draft replies to a mail. `false`
    /// leaves out the prompts of a reply (translate it into the language
    /// of the mail received, `lang = "replied"`); absent, every prompt is
    /// listed.
    #[serde(default)]
    pub reply: Option<bool>,
}

/// A prompt of an AI menu.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiPromptView {
    pub id: String,
    /// Its letter in the menu.
    pub key: String,
    pub label: String,
    pub scope: String,
    pub output: String,
    /// A submenu; empty when the prompt has none.
    pub variants: Vec<AiVariantView>,
    /// `builtin` or `user`.
    pub source: String,
    /// Whether this version of the core can run it from this screen yet.
    pub available: bool,
    /// For a prompt of several conversations, what it takes when none is
    /// marked: `cursor` (the client sends the one under its cursor) or
    /// `folder` (the core takes the whole folder).
    pub unmarked: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiVariantView {
    pub key: String,
    pub label: String,
}

/// A part of a draft's body chosen by the user, in UTF-16 code units (the
/// unit of the text views of the app), end excluded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiSelection {
    pub start: u32,
    pub end: u32,
}

/// Params of `ai_run`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiRunParams {
    pub prompt: String,
    /// The key of a variant, for a prompt that has some.
    #[serde(default)]
    pub variant: Option<String>,
    /// The draft, for the prompts of the compose screen, at this revision:
    /// the text must be saved first, and the run is refused if it changed.
    #[serde(default)]
    pub draft_id: Option<i64>,
    #[serde(default)]
    pub revision: Option<i64>,
    /// Only this part of the draft's body.
    #[serde(default)]
    pub selection: Option<AiSelection>,
    /// The conversation, for the prompts of the mail and selection screens;
    /// with `message_id`, the mail of it a `message` prompt works on (its
    /// latest received when absent).
    #[serde(default)]
    pub thread_id: Option<i64>,
    #[serde(default)]
    pub message_id: Option<i64>,
    /// For the prompts of several conversations: the folders shown (one
    /// per account in the view of all accounts), whose role gives the
    /// targets, and the conversations marked. None marked: what the
    /// prompt's `unmarked` says, the conversation under the cursor (given
    /// here by the client) or the whole folder.
    #[serde(default)]
    pub folder_ids: Option<Vec<i64>>,
    #[serde(default)]
    pub thread_ids: Option<Vec<i64>>,
    /// The attachment, for the prompts of an attachment (its text is read
    /// by the core: `AttachmentView::readable`).
    #[serde(default)]
    pub attachment_id: Option<i64>,
    /// The app's language (`fr`), for the answers that follow it.
    pub lang: String,
}

/// Result of `ai_run`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiRunResult {
    pub run_id: i64,
    /// Whether `AiDelta` events will come (text answers); a JSON answer
    /// only comes whole.
    pub streamed: bool,
}

/// Params of `ai_cancel`, `ai_get_run` and `ai_dismiss`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiRunIdParams {
    pub run_id: i64,
}

/// A piece of a word diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiDiffPiece {
    /// `same`, `removed` or `added`.
    pub kind: String,
    pub text: String,
}

/// One segment of a draft rewritten.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiSegmentChange {
    /// Where the segment was in the body, in UTF-16 code units.
    pub start: u32,
    pub end: u32,
    pub old: String,
    pub new: String,
    pub diff: Vec<AiDiffPiece>,
}

/// What a finished run proposes.
///
/// Rust note: `#[serde(tag = "kind")]` writes the variant's name in a
/// `kind` field next to its own fields: `{"kind": "replace", "segments":
/// […], "body": "…"}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AiProposal {
    /// The draft's segments rewritten; `body` is the whole body as it would
    /// be, `unchanged` when the AI changed nothing.
    Replace { segments: Vec<AiSegmentChange>, body: String, unchanged: bool },
    /// Subject lines to choose from.
    Subjects { items: Vec<String> },
    /// A text to read (a translation, the dates of a mail): nothing to
    /// apply.
    Text { text: String },
    /// A summary and the action items of a conversation.
    Summary { summary: String, actions: Vec<AiActionView> },
    /// The phishing risk of a mail; `raised` when the core raised the
    /// model's `low` because of what it found itself (`ia.md`, section 6).
    Verdict { risk: String, reasons: Vec<String>, raised: bool },
    /// Texts to choose from; `then` says what the one chosen does
    /// (`reply`: starts a reply with it, `insert`: goes into the draft).
    Choices { items: Vec<String>, then: String },
    /// A target suggested for each conversation sorted; the batches whose
    /// answer could not be read gave none.
    Targets { items: Vec<AiSuggestionView>, failed_batches: u64 },
    /// The appointments of a mail (`dates`); `ai_apply {choice}` gives the
    /// calendar file of one, and the proposal stays, to save another.
    Events { items: Vec<AiEventView> },
    /// The sender's contact card (`contact`); `ai_apply` gives its vCard
    /// file and ends the run.
    Contact { card: AiContactView },
}

/// The contact card of the sender of a mail (`ia.md`, section 8, "Fiche
/// contact"). Every text may be empty; the lists may be empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiContactView {
    /// The name as the sender signs.
    pub name: String,
    pub given: String,
    pub family: String,
    pub organization: String,
    /// The job title.
    pub title: String,
    /// The From address first.
    pub emails: Vec<String>,
    pub phones: Vec<AiPhoneView>,
    pub address: AiAddressView,
    /// Full `http(s)://` links.
    pub urls: Vec<String>,
    /// What the model gave that is not an address, a number or a link, as
    /// written: left out of the card, shown so the user knows.
    #[serde(default)]
    pub dropped: Vec<String>,
}

/// A phone number of a contact card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiPhoneView {
    /// `mobile`, `work`, `home`, `fax` or `other`.
    pub kind: String,
    /// As the mail writes it.
    pub number: String,
}

/// The postal address of a contact card; every part may be empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiAddressView {
    /// May hold several lines.
    pub street: String,
    pub locality: String,
    pub postcode: String,
    pub region: String,
    pub country: String,
}

/// An appointment of a mail (`ia.md`, section 8, "Dates et rendez-vous").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiEventView {
    pub title: String,
    /// `YYYY-MM-DD` (a whole day) or `YYYY-MM-DDTHH:MM` (floating: the time
    /// as the mail gives it, read in the user's time zone).
    pub start: String,
    /// Of the form of `start`; empty when the mail gives no end. A whole
    /// day's end is the last day included.
    pub end: String,
    /// Empty when the mail gives none.
    pub location: String,
    /// How the date was read, what to bring; may be empty.
    pub details: String,
}

/// A file to save (`ai_apply` on an appointment or a contact card): the
/// client writes it where the user's files go (the core has no access to
/// Downloads), under `filename` or a free variant of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiFile {
    /// `2026-11-20 Visite de l'appartement.ics`, `Alice Martin.vcf`.
    pub filename: String,
    /// The iCalendar (RFC 5545) or vCard (RFC 2426) text, CRLF line ends.
    pub content: String,
}

/// A suggested target for a conversation (`ia.md`, section 11).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiSuggestionView {
    pub run_id: i64,
    pub thread_id: i64,
    pub target: FolderRole,
    pub reason: String,
}

/// Params of `ai_suggestions`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiSuggestionsParams {
    pub thread_ids: Vec<i64>,
}

/// Params of `ai_dismiss_suggestion`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiDismissSuggestionParams {
    pub run_id: i64,
    pub thread_id: i64,
}

/// An action item of a summary; `owner` and `due` may be empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiActionView {
    pub task: String,
    pub owner: String,
    pub due: String,
}

/// Result of `ai_get_run`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiRunView {
    pub run_id: i64,
    pub prompt: String,
    pub variant: Option<String>,
    /// `running`, `done`, `failed` or `cancelled`.
    pub state: String,
    /// `accepted`, `rejected`, `partial`, or absent.
    pub decision: Option<String>,
    /// Present while the proposal can be applied.
    pub proposal: Option<AiProposal>,
    /// The error of a failed run: AI code and message.
    pub error_code: Option<String>,
    pub error: Option<String>,
}

/// Params of `ai_apply`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiApplyParams {
    pub run_id: i64,
    /// The draft's revision the client shows: refused if it moved since the
    /// run started, so that a text typed meanwhile is never overwritten.
    /// Absent for a proposal that is not applied to a draft.
    #[serde(default)]
    pub revision: Option<i64>,
    /// For a list (`subjects`, `choices`, `events`): the index of the one
    /// chosen, from 0.
    #[serde(default)]
    pub choice: Option<usize>,
}

/// Result of `ai_apply`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiApplyResult {
    /// The draft as saved, for a proposal applied to a draft.
    pub draft: Option<DraftView>,
    /// The text chosen, for a choice the client applies itself (a quick
    /// reply: the client starts the reply and puts the text in it).
    pub text: Option<String>,
    /// The calendar file of the appointment chosen, for the client to save.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calendar: Option<AiFile>,
    /// The vCard file of the sender's card, for the client to save.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contact: Option<AiFile>,
}

#[cfg(test)]
mod reason_tests {
    use super::*;
    use serde_json::json;

    /// The message is the template with its values; `data` the reason
    /// and the same values, so that a client writes its own sentence.
    #[test]
    fn an_error_carries_its_reason_its_values_and_the_english_sentence() {
        let e = RpcError::new(error_code::INVALID_PARAMS, reason::TAG_EXISTS.with("tag", "Urgent"));
        assert_eq!(e.message, "the tag Urgent exists already");
        assert_eq!(e.data, Some(json!({"reason": "tag_exists", "tag": "Urgent"})));
        assert_eq!(e.reason(), Some("tag_exists"));
        // Numbers in digits, lists joined.
        let w = reason::AI_PROVIDER_UNSAFE.with("tools", vec!["Bash", "Read"]).with("mcp_servers", Vec::<String>::new());
        assert_eq!(w.message(), "the claude binary announced tools (Bash, Read) and MCP servers (); nothing was sent to it");
        assert_eq!(reason::REVISION_CONFLICT.with("revision", 4).message(), "this draft was changed elsewhere (revision 4): reload it");
        // A bare reason needs no conversion.
        assert_eq!(RpcError::new(error_code::BUSY, reason::IMPORT_RUNNING).message, "an import is already running");
    }

    /// Values outside the sentence join `data` without changing it.
    #[test]
    fn data_outside_the_sentence_is_added_beside_the_reason() {
        let e = RpcError::new(error_code::AI, reason::AI_TIMEOUT).with_data("code", "ai_timeout");
        assert_eq!(e.data, Some(json!({"reason": "ai_timeout", "code": "ai_timeout"})));
        assert_eq!(e.message, "the AI did not answer in time");
    }

    /// The keys of a template, in order, each once.
    #[test]
    fn the_keys_of_a_sentence_are_read_from_its_template() {
        assert_eq!(reason::PASSWORD_SERVER_CHANGED.keys(), vec!["saved_user", "saved_host", "user", "host", "account"]);
        assert!(reason::IMPORT_RUNNING.keys().is_empty());
        assert_eq!(reason::named("tag_exists"), Some(reason::TAG_EXISTS));
        assert_eq!(reason::named("nope"), None);
    }

    /// Every template filled with all its keys leaves no brace: a reason
    /// cannot be given a sentence its values do not complete.
    #[test]
    fn every_template_is_completed_by_its_values() {
        for r in reason::ALL {
            let mut why = Why::from(*r);
            for k in r.keys() {
                why = why.with(k, "x");
            }
            let m = why.message();
            assert!(!m.contains('{') && !m.contains('}'), "{}: {m}", r.name);
        }
    }

    /// A maintenance keeps its whole value as `data`, its message from its
    /// reason.
    #[test]
    fn a_maintenance_says_its_reason() {
        let m = Maintenance {
            reason: MaintenanceReason::Migration,
            from: Some(9),
            to: Some(13),
            at: None,
            since: 0,
            message: None,
        };
        let e = m.error();
        assert_eq!(e.message, "the database is being upgraded (version 9 to 13)");
        assert_eq!(e.reason(), Some("migration"));
        assert_eq!(e.code, error_code::MAINTENANCE);
    }
}
