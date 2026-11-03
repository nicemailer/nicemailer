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

use nm_core::{FolderCategory, FolderRole};
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
    /// Threads of a tag, a group or a contact, across folders.
    pub const LIST_THREADS_BY: &str = "list_threads_by";
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
    pub folder_id: i64,
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
    /// folders (`list_threads_by`) the folder of the latest message. Moves
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Remote images were removed from `body_html`.
    pub has_remote_content: bool,
    /// Display preferred for the sender's mails: `"text"`, `"html"`, or
    /// `None` for the client's default (`set_body_format`).
    pub from_body_format: Option<String>,
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
    /// `"text"`, `"html"`, or `null` to go back to the default.
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
    /// Names of the groups the address belongs to.
    pub groups: Vec<String>,
    /// Number of messages received from the address.
    pub message_count: i64,
}

/// Parameters of `list_threads_by`: exactly one of the three ids.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListThreadsByParams {
    #[serde(default)]
    pub tag_id: Option<i64>,
    #[serde(default)]
    pub group_id: Option<i64>,
    #[serde(default)]
    pub address_id: Option<i64>,
    #[serde(default)]
    pub limit: Option<u32>,
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

    /// Data directory: `~/Library/Application Support/NiceMailer`, or the
    /// value of `NICEMAILER_DATA_DIR` when set (useful in development to
    /// keep a separate test database).
    pub fn data_dir() -> PathBuf {
        // `if let Some(x) = ...` runs the block only when the Option holds
        // a value, and binds it to `x`. `var_os` returns `None` when the
        // variable is not set.
        if let Some(dir) = std::env::var_os("NICEMAILER_DATA_DIR") {
            return PathBuf::from(dir);
        }
        // `map` transforms the value inside the Option if there is one;
        // `unwrap_or_else` provides a fallback when there is none.
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
        home.join("Library/Application Support/NiceMailer")
    }

    /// SQLite database file name, inside the data directory.
    pub const DATABASE: &str = "nicemailer.db";
    /// Unix socket file name, inside the data directory.
    pub const SOCKET: &str = "core.sock";
}
