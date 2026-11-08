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

use nm_core::{AuthPolicy, FolderCategory, FolderRole, ScreeningTarget};
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
    pub const HELLO: &str = "hello";
    pub const PING: &str = "ping";
    pub const SUBSCRIBE: &str = "subscribe";
    pub const LIST_FOLDERS: &str = "list_folders";
    pub const LIST_THREADS: &str = "list_threads";
    pub const GET_THREAD: &str = "get_thread";
    pub const SEARCH: &str = "search";
    pub const MOVE_THREAD: &str = "move_thread";
    pub const SET_THREAD_SEEN: &str = "set_thread_seen";
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
    pub const GET_ATTACHMENT: &str = "get_attachment";
    pub const UNSUBSCRIBE: &str = "unsubscribe";
    /// Audit log, newest first (`tracabilite.md`).
    pub const LIST_HISTORY: &str = "list_history";
    /// Tags of an account, or of every account.
    pub const LIST_TAGS: &str = "list_tags";
    /// Adds (or removes) a tag on every message of a thread.
    pub const TAG_THREAD: &str = "tag_thread";
    /// Contact groups (`groupes.md`).
    pub const LIST_GROUPS: &str = "list_groups";
    /// Adds (or removes) an address, or the sender of a thread, to a group.
    pub const SET_GROUP_MEMBER: &str = "set_group_member";
    /// Known addresses, with their decision and groups.
    pub const LIST_CONTACTS: &str = "list_contacts";
    /// Takes, changes or removes the screening decision on an address, and
    /// files the mails already received from it (`dossiers.md`).
    pub const SET_ADDRESS_SCREENING: &str = "set_address_screening";
    /// The same for a domain.
    pub const SET_DOMAIN_SCREENING: &str = "set_domain_screening";
    /// Undoes a move or a decision from the token it returned.
    pub const UNDO: &str = "undo";
    /// Every account, IMAP and archive, with its state (`AccountView`).
    pub const LIST_ACCOUNTS: &str = "list_accounts";
    /// State of `config.toml` and `accounts.toml`: paths, error refusing
    /// them, warnings (`ConfigStatus`).
    pub const CONFIG_STATUS: &str = "config_status";
    /// Changes the display order of the accounts (account picker, sidebar).
    pub const SET_ACCOUNT_ORDER: &str = "set_account_order";
    /// Saves the IMAP password of an account in the Keychain, for the
    /// server the file names now (`configuration.md`, section 6).
    pub const SET_PASSWORD: &str = "set_password";
    /// Where a message is (its thread and folder), from its id: opens a
    /// deep link `nicemailer://m/<id>` (`docs/deep-links.md`).
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
    /// Creates an IMAP account: folders, roles, initial import, password,
    /// `accounts.toml`.
    pub const ADD_ACCOUNT: &str = "add_account";
    /// Changes the workflow roles of the folders of an account.
    pub const SET_FOLDER_ROLES: &str = "set_folder_roles";
    /// Renames the key of an account, keeping its data.
    pub const RENAME_ACCOUNT: &str = "rename_account";
    /// Deletes an account, its data and its password.
    pub const REMOVE_ACCOUNT: &str = "remove_account";
    /// Runs the garbage collector of the raw store now.
    pub const RAW_GC: &str = "raw_gc";
    /// Starts a reply to all to a message, or gives back the reply already
    /// started (`reponse.md`, section 4): recipients, subject and
    /// quotation prepared by the core.
    pub const CREATE_DRAFT: &str = "create_draft";
    /// The latest version of a draft, marked open.
    pub const OPEN_DRAFT: &str = "open_draft";
    /// Saves the body typed, at each pause (`reponse.md`, section 5).
    pub const UPDATE_DRAFT: &str = "update_draft";
    /// The draft window closed.
    pub const CLOSE_DRAFT: &str = "close_draft";
    /// Abandons a draft; returns it, so that the client can restore it.
    pub const DISCARD_DRAFT: &str = "discard_draft";
    /// Queues the draft for sending, after the undo delay (`reponse.md`,
    /// section 7).
    pub const SEND_DRAFT: &str = "send_draft";
    /// Cancels a send during the undo delay: the draft is editable again.
    pub const CANCEL_SEND: &str = "cancel_send";
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
    /// The core is busy with maintenance (migration, rebuild). Not used yet.
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
    /// the message holds its answer.
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
    pub code: i64,
    /// Human-readable description, for logs and debugging.
    pub message: String,
    /// Optional structured details.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl RpcError {
    /// Builds an error without `data`.
    ///
    /// Rust note: `impl Into<String>` accepts anything convertible into a
    /// `String`, so callers can pass either a literal `"text"` or a
    /// `String` built with `format!`. `Self` is shorthand for `RpcError`.
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        RpcError { code, message: message.into(), data: None }
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
    /// The counters or the content of a folder changed.
    FolderChanged { folder_id: i64 },
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
    /// An account cannot connect or authenticate; `error` says why. The
    /// state stays available in `list_accounts` until it is resolved,
    /// which is announced by `AccountChanged`.
    AccountError { account_id: i64, error: String },
    /// `config.toml` and `accounts.toml` were applied with these changed
    /// keys (dotted, sorted; those of the accounts start with `accounts.`).
    /// Empty `keys` after a `ConfigError`: the files are valid again,
    /// without any change to the settings applied.
    ConfigChanged { keys: Vec<String> },
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
    /// of the same message (`modele de donnees.md`, section 6). `reason` is
    /// the server's answer.
    ActionReverted { message_id: i64, kind: String, reason: String },
    /// A draft was sent: the server accepted it (`reponse.md`, section 7).
    /// `message_id` is the mail in Sent; `moved_to` the folder its
    /// conversation was filed in (absent: it stayed); `undo` cancels that
    /// filing and the decisions taken on the recipients, not the sending.
    DraftSent { draft_id: i64, message_id: i64, thread_id: Option<i64>, moved_to: Option<FolderRole>, undo: Option<String> },
    /// A draft could not be sent, for good: it is editable again, and
    /// `reply_to` is the mail it answers (the next `r` on it reopens it).
    SendFailed { draft_id: i64, reply_to: Option<i64>, reason: String },
    /// A draft was sent, but the server refused some recipients (each with
    /// its answer); the others received it.
    SendPartial { draft_id: i64, refused: Vec<String> },
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
/// for one.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EverythingScope {
    #[serde(default)]
    pub account_id: Option<i64>,
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
    /// Latest message received at or after this Unix time.
    #[serde(default)]
    pub after: Option<i64>,
    /// Latest message received before this Unix time.
    #[serde(default)]
    pub before: Option<i64>,
    /// Tagged with this tag: a message of the thread carries it. A name,
    /// not an id: the tags of the same name in every account are one (each
    /// account has its own "Clients"), compared without case.
    #[serde(default)]
    pub tag: Option<String>,
    /// Of the members of this group (`groupes.md`): a message of the thread
    /// in the listed folders is from one of them or to them (To, Cc).
    #[serde(default)]
    pub group_id: Option<i64>,
    /// Of this contact (an `address` id): a message of the thread in the
    /// listed folders is from it or to it (To, Cc).
    #[serde(default)]
    pub address_id: Option<i64>,
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

/// Params of `locate_message`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocateMessageParams {
    pub message_id: i64,
}

/// Result of `locate_message`: where to show a message. A message is in
/// one folder; its thread may span several.
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
    pub thread_id: i64,
    pub seen: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetThreadFlaggedParams {
    pub thread_id: i64,
    pub flagged: bool,
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
    /// Number of threads in the folder.
    pub thread_count: i64,
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
    /// Names of the senders of the thread's messages in this folder, from
    /// oldest to newest, without duplicates.
    pub participants: Vec<String>,
    /// Beginning of the latest message's text.
    pub snippet: Option<String>,
    /// Unix timestamp (seconds) of the most recent message received in this
    /// folder. The list is sorted on it, newest first.
    pub date_last_received: i64,
    /// Counters restricted to the listed folder.
    pub message_count: i64,
    pub unread_count: i64,
    pub is_flagged: bool,
    pub has_attachments: bool,
    /// Set when the thread came back to the Inbox from Todo or Waiting.
    pub returned_from: Option<FolderRole>,
    /// Folder preselected if the user replies (see `FolderRole::reply_target`).
    pub reply_target: Option<FolderRole>,
    /// Folder this row describes: the listed folder, or for a list across
    /// folders (Everything) the folder of the latest message. Moves
    /// pass it as `from_folder_id`.
    pub folder_id: i64,
    /// Unix timestamp of the latest change to a message of the thread in
    /// this folder (sort `modified`).
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
    /// `"images"` (HTML with remote images), or `None` for the client's
    /// default (`set_body_format`).
    pub from_body_format: Option<String>,
    /// The text body cut into segments, for a simplified display: what the
    /// message brings is visible, the history it repeats is `hidden`
    /// (`docs/thread-display.md`). Empty when the body is not downloaded.
    pub segments: Vec<SegmentView>,
    /// The message brings nothing new: it only quotes earlier ones.
    pub only_quotes: bool,
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

/// Params of `create_draft`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateDraftParams {
    /// The message replied to; its account sends the reply.
    pub reply_to: i64,
    /// The line introducing the quotation, written by the client in its
    /// language ("On Nov 7, 2026, at 10:12, Alice <a@x> wrote:").
    #[serde(default)]
    pub quote_header: String,
    /// The body to start with instead of the quotation: restores a draft
    /// abandoned by mistake (`z` after `discard_draft`).
    #[serde(default)]
    pub body: Option<String>,
}

/// Params of `open_draft`, `close_draft`, `discard_draft`, `cancel_send`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftIdParams {
    pub id: i64,
}

/// Params of `update_draft`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateDraftParams {
    pub id: i64,
    /// The revision the client edited: refused if it is not the current
    /// one (`REVISION_CONFLICT`).
    pub revision: i64,
    pub body: String,
}

/// Params of `send_draft`: the last body typed is saved with it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendDraftParams {
    pub id: i64,
    pub revision: i64,
    pub body: String,
    /// Where to file the conversation after sending; absent: the core's
    /// preselection (the only choice of the app in v1).
    #[serde(default)]
    pub after_send: Option<FolderRole>,
}

/// A draft, as the reply screen shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DraftView {
    pub id: i64,
    pub account_id: i64,
    pub from: AddressView,
    pub to: Vec<AddressView>,
    pub cc: Vec<AddressView>,
    pub subject: String,
    pub body: String,
    pub revision: i64,
    /// `editing` or `sending`.
    pub state: String,
    /// The message replied to.
    pub reply_to: Option<i64>,
    /// Its conversation.
    pub thread_id: Option<i64>,
    /// Where the conversation goes once the mail is sent, as preselected
    /// now (`reponse.md`, section 8); absent when it stays.
    pub after_send_target: Option<FolderRole>,
    /// When a draft `sending` leaves, Unix seconds.
    pub send_at: Option<i64>,
}

/// Result of `create_draft`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateDraftResult {
    pub draft: DraftView,
    /// The draft already existed (a reply to this message was started).
    pub reused: bool,
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
    pub thread_id: i64,
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
    #[serde(default)]
    pub thread_id: Option<i64>,
    /// Remove the address from the group instead of adding it.
    #[serde(default)]
    pub remove: bool,
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
    /// Why the account cannot connect, if known. `None` does not mean
    /// connected, only that nothing prevents it.
    pub error: Option<String>,
    /// Display order: `list_accounts` and `list_folders` give the accounts
    /// by increasing position (then by id). Changed by `set_account_order`.
    pub position: i64,
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

/// A problem of `config.toml` or `accounts.toml`: why it was refused, or a
/// warning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigProblem {
    /// The file: `config.toml` or `accounts.toml`.
    #[serde(default)]
    pub file: String,
    /// Line, from 1.
    pub line: Option<u32>,
    /// Dotted key, e.g. `accounts.perso.imap.port`.
    pub key: Option<String>,
    pub message: String,
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
    pub as_of: Position,
}

/// The settings of `[ui]` of `config.toml` the app applies itself
/// (`docs/geek-ui.md`): sent with `config_status`, read again after
/// `ConfigChanged`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiConfig {
    /// `hidden` (the default) or `shown`.
    pub title_bar: String,
}

/// A plain list with the event position of the read.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Items<T> {
    pub items: Vec<T>,
    pub as_of: Position,
}

pub mod paths {
    // `PathBuf` is an owned file system path (like `String` for text).
    use std::path::PathBuf;

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
        let chosen = std::fs::read_to_string(default.join(CHOICE_FILE)).ok();
        resolve(std::env::var_os("NICEMAILER_DATA_DIR").map(PathBuf::from), chosen.as_deref(), default)
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
    }

    /// SQLite database file name, inside the data directory.
    pub const DATABASE: &str = "nicemailer.db";
    /// Unix socket file name, inside the data directory.
    pub const SOCKET: &str = "core.sock";
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

/// Params of `test_account`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestAccountParams {
    pub email: String,
    pub imap: ImapParams,
    pub password: String,
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
}

/// Params of `count_archive`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CountArchiveParams {
    pub email: String,
    pub imap: ImapParams,
    pub password: String,
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
    pub password: String,
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
}
