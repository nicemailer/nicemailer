//! NiceMailer core: JSON-RPC 2.0 server over a Unix socket, one JSON object
//! per line.
//!
//! The core is the only process that touches the database. The app, the CLI
//! and future clients connect to its socket and send requests; the core
//! answers and pushes events when something changes.
//!
//! Development v1: started by hand, without a LaunchAgent and without
//! checking the clients' code signature (`architecture.md`, sections 6
//! and 9).
//!
//! # Concurrency model
//!
//! The core uses tokio, an *asynchronous* runtime. Instead of one OS thread
//! per client, each connection is a lightweight *task*; while a task waits
//! (for a line from its socket, say), tokio runs other tasks on the same
//! threads. `async fn` declares such a function, and `.await` marks the
//! points where it may pause.
//!
//! SQLite calls, however, are blocking: they would stall a tokio thread.
//! They therefore run on tokio's pool of blocking threads
//! (`spawn_blocking`, see `with_store` and `with_reader`). Commands take the
//! single writing store; queries take a connection of the read pool, so a
//! long write never makes a list wait (`nm-store`, `pool.rs`).
//!
//! # Settings
//!
//! The core reads `config.toml` at start and watches it (`nm-engine`): a
//! valid change is applied at once, an invalid file is refused whole and
//! reported (`configuration.md`, section 4).
//!
//! Per connection there are:
//! - a reading loop (`serve`) that handles one request at a time,
//! - a writer task that sends the lines queued in an `mpsc` channel,
//! - after `subscribe`, a `forward` task that relays events into the same
//!   channel.
//!
//! Funnelling responses and events through one channel guarantees that two
//! lines are never interleaved on the socket.

mod events;

use std::fs::{self, File};
// Unix-specific extension trait that adds `from_mode` (chmod-style
// permissions) to `fs::Permissions`.
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

// `anyhow` provides a catch-all error type for applications: any error can
// be converted into `anyhow::Error` with `?`, and `.context(...)` adds an
// explanation. `bail!(...)` returns early with an error message.
use anyhow::{Context, bail};
use clap::Parser;
use nm_api::{
    AttachmentFileView, CommandResult, DEFAULT_LIMIT, Event, ExportMessageParams, ExportResult, FolderList,
    GetAttachmentParams, GetThreadParams, UnsubscribeParams, UnsubscribeResult,
    GroupMemberResult, HelloParams, HelloResult, ImportMboxParams, ImportStatus, Items, ListContactsParams,
    ListHistoryParams, ListTagsParams, ListThreadsParams, MAX_LIMIT, MoveThreadParams,
    PROTOCOL_VERSION, Page, Request, Response, RpcError, SearchParams, SearchResult, SetBodyFormatParams,
    SetGroupMemberParams, SetThreadFlaggedParams, SetThreadSeenParams, SubscribeParams, SubscribeResult,
    SetAddressScreeningParams, SetDomainScreeningParams, TagThreadParams, ThreadView, UndoParams, error_code,
    method, ConfigStatus, SetAccountOrderParams,
};
use nm_core::Source;
use nm_engine::Engine;
use nm_store::pool::ReadPool;
use nm_store::import::{BATCH_SIZE, ImportOptions, ImportProgress, import_mbox};
use nm_store::attachments::AttachmentFiles;
use nm_store::raw::RawStore;
use nm_store::screening::{MoveOptions, Outcome};
use nm_store::{ChangeSet, Store, StoreError};
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
// Two kinds of channels: `mpsc` (multi-producer, single-consumer) is a queue
// with one reader; `broadcast` delivers each message to every receiver.
use tokio::sync::{broadcast, mpsc};

use events::{EventLog, Replay};

/// Command-line arguments. `clap` generates the parser (and `--help`) from
/// this struct: each field becomes an option, and the `///` comment
/// becomes its help text, which is why it stays in French (user-facing).
#[derive(Parser)]
#[command(name = "nicemailer-core", version, about = "Noyau NiceMailer")]
struct Args {
    /// Dossier de données (base, socket, verrou).
    #[arg(long, env = "NICEMAILER_DATA_DIR")]
    data_dir: Option<PathBuf>,
    /// Fichier de réglages (par défaut ~/.config/nicemailer/config.toml).
    #[arg(long, env = "NICEMAILER_CONFIG")]
    config: Option<PathBuf>,
}

/// State shared by all connections.
///
/// Rust note: a `Mutex` guarantees that only one thread at a time accesses
/// the value inside. `lock()` returns a guard; the lock is released when the
/// guard goes out of scope. The store and the event log have separate locks
/// so that reading the event position never waits for a slow query.
///
/// Lock order, to avoid deadlocks: `store`, then `engine`, then `import`,
/// then `events`. A thread holding one of them never waits for one that
/// comes earlier in this list. The read pool has its own internal lock,
/// held only to lend or take back a connection, never while waiting for
/// another lock.
struct Core {
    store: Mutex<Store>,
    /// Read-only connections for the queries.
    readers: ReadPool,
    /// Settings applied and state of the accounts (`nm-engine`).
    engine: Mutex<Engine>,
    events: Mutex<EventLog>,
    /// `events.client_queue`: lines a client may have waiting.
    client_queue: usize,
    /// Immutable raw message files (`stockage-brut.md`).
    raw: RawStore,
    /// Attachments extracted from them on demand (`attachments.rs`).
    attachments: AttachmentFiles,
    /// The running mbox import, or the last one.
    import: Mutex<ImportState>,
}

/// Imports run one at a time, on a dedicated thread (see `start_import`).
#[derive(Default)]
struct ImportState {
    /// Id of the last import started.
    last_job: u64,
    /// Running import, or the last one, finished.
    status: Option<ImportStatus>,
}

/// Entry point. `#[tokio::main]` wraps `main` so that it runs inside the
/// tokio runtime, which allows `main` itself to be `async`.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    // `--data-dir` (or the env variable) if given, otherwise the default.
    let data_dir = args.data_dir.unwrap_or_else(nm_api::paths::data_dir);
    fs::create_dir_all(&data_dir).with_context(|| format!("création de {}", data_dir.display()))?;
    // 0o700: only the current user can enter the directory (the `0o` prefix
    // is octal, as with chmod).
    fs::set_permissions(&data_dir, fs::Permissions::from_mode(0o700))?;

    // Single-instance lock (`stockage-brut.md`): two cores writing to the
    // same database would corrupt its derived data. `try_lock` takes an
    // exclusive OS lock on the file without waiting. The OS releases it when
    // the file is closed, even if the process crashes, so a stale lock can
    // never block a restart. `lock` is kept alive until the end of `main`.
    let lock = File::create(data_dir.join("lock"))?;
    if lock.try_lock().is_err() {
        bail!("un autre noyau utilise déjà {}", data_dir.display());
    }

    let db_path = data_dir.join(nm_api::paths::DATABASE);
    let mut store = Store::open(&db_path).with_context(|| format!("ouverture de {}", db_path.display()))?;
    // Opened after the writing store, which creates or migrates the
    // database. Four connections: the app rarely has more queries running
    // at once (a list, a thread, a search), and each is short.
    let readers = ReadPool::open(&db_path, 4).with_context(|| format!("lecture de {}", db_path.display()))?;

    // Settings, before the event log, whose sizes they give. An invalid
    // file does not stop the core: it starts with the defaults and no
    // account, and reports the error (`configuration.md`, section 4).
    let config_path = args.config.unwrap_or_else(nm_engine::config::default_path);
    let mut engine = Engine::new(config_path.clone(), secrets());
    let startup_events = engine.reload(&mut store, true)?;
    log_config(&engine);
    let settings = engine.config().events.clone();

    let raw = RawStore::open(&data_dir).with_context(|| format!("stockage brut dans {}", data_dir.display()))?;
    let attachments = AttachmentFiles::open(&data_dir)
        .with_context(|| format!("pièces jointes dans {}", data_dir.display()))?;
    let mut events = EventLog::new(settings.replay_buffer as usize, settings.client_queue as usize);
    for e in startup_events {
        events.push(e);
    }
    // `Arc` so that every connection task can hold a pointer to the shared
    // state.
    let core = Arc::new(Core {
        store: Mutex::new(store),
        readers,
        engine: Mutex::new(engine),
        events: Mutex::new(events),
        client_queue: settings.client_queue as usize,
        raw,
        attachments,
        import: Mutex::new(ImportState::default()),
    });
    // Kept alive until the end of `main`: dropping it stops the watch. A
    // watch that cannot start is not fatal: the file is then read at the
    // next start only.
    let _watch = {
        let core = core.clone();
        match nm_engine::watch::watch(&config_path, move || reload_config(&core)) {
            Ok(w) => Some(w),
            Err(e) => {
                eprintln!("config : surveillance impossible ({e}) ; le fichier sera relu au prochain démarrage");
                None
            }
        }
    };

    let socket_path = data_dir.join(nm_api::paths::SOCKET);
    // A socket file left behind by a crash would make `bind` fail. Thanks to
    // the lock above, we know no other core is using it.
    let _ = fs::remove_file(&socket_path);
    let listener = UnixListener::bind(&socket_path)?;
    // 0o600: only the current user can connect.
    fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))?;
    eprintln!("nicemailer-core : base {}, socket {}", db_path.display(), socket_path.display());
    start_reindex(&core)?;

    // Accept connections until Ctrl-C (SIGINT) or SIGTERM (sent by `kill`
    // or by launchd).
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    loop {
        // `select!` waits on several futures at once and runs the branch of
        // the first one that completes.
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let core = core.clone();
                // `tokio::spawn` starts an independent task; `async move`
                // moves `core` and `stream` into it, so the task owns them.
                tokio::spawn(async move {
                    if let Err(e) = serve(core, stream).await {
                        eprintln!("connexion fermée : {e}");
                    }
                });
            }
            _ = tokio::signal::ctrl_c() => break,
            _ = sigterm.recv() => break,
        }
    }
    cleanup(&socket_path);
    drop(lock);
    Ok(())
}

/// Removes the socket file on shutdown, so clients get "no such file"
/// rather than "connection refused".
fn cleanup(socket: &Path) {
    let _ = fs::remove_file(socket);
}

/// State of one connection.
struct Session {
    /// Whether the client completed the `hello` handshake.
    hello: bool,
    /// Who the client claims to be; recorded in `modified_by` and the audit log.
    source: Source,
    /// Queue of lines to write to this client's socket.
    out: mpsc::Sender<Arc<String>>,
    /// Whether the client called `subscribe` (allowed only once).
    subscribed: bool,
}

/// Handles one client connection until it closes.
async fn serve(core: Arc<Core>, stream: UnixStream) -> anyhow::Result<()> {
    // Split the socket into a reading half and a writing half, so that they
    // can be used by different tasks.
    let (read, mut write) = stream.into_split();
    // Bounded queue per client (`events.client_queue`): a client that stops
    // reading cannot make the core's memory grow without limit.
    let (out, mut rx) = mpsc::channel::<Arc<String>>(core.client_queue);
    // Writer task: takes lines from the queue and writes them, each followed
    // by a newline. `recv` returns `None` once every sender is dropped,
    // which ends the loop.
    let writer = tokio::spawn(async move {
        while let Some(line) = rx.recv().await {
            if write.write_all(line.as_bytes()).await.is_err() || write.write_all(b"\n").await.is_err() {
                break;
            }
        }
    });

    // The source defaults to CLI until `hello` says otherwise.
    let mut session = Session { hello: false, source: Source::Cli, out: out.clone(), subscribed: false };
    // `BufReader` buffers the reads; `lines()` splits the stream on newlines.
    let mut lines = BufReader::new(read).lines();
    // `next_line` returns `Ok(None)` when the client closes the connection.
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Request>(&line) {
            // Invalid JSON: the id is unknown, so the JSON-RPC spec says to
            // answer with `id: null`.
            Err(e) => Some(Response::err(Value::Null, RpcError::new(error_code::PARSE_ERROR, e.to_string()))),
            Ok(req) => {
                let id = req.id.clone();
                let result = dispatch(&core, &mut session, req).await;
                // No id: the request is a client notification and gets no
                // response. `id.map(...)` builds a response only if there is
                // an id.
                id.map(|id| match result {
                    Ok(v) => Response::ok(id, v),
                    Err(e) => Response::err(id, e),
                })
            }
        };
        // Rust note: this `if let ... && ...` is a *let chain* (Rust 2024):
        // the body runs only if there is a response AND sending it failed,
        // which means the writer task stopped (socket closed).
        if let Some(r) = response
            && out.send(Arc::new(serde_json::to_string(&r)?)).await.is_err()
        {
            break;
        }
    }
    // Drop every sender of the queue (the session holds a clone) so that the
    // writer task sees the end of the queue, then wait for it to flush.
    drop(session);
    drop(out);
    let _ = writer.await;
    Ok(())
}

/// Deserializes the params of a request into the expected type `T`.
/// Missing params (`null`) are treated as an empty object, so that methods
/// whose params are all optional can be called without params.
///
/// Rust note: `<T: DeserializeOwned>` is a *generic* function with a
/// *trait bound*: it works for any type `T` that serde can build from JSON.
/// The caller picks `T` through the type annotation, e.g.
/// `let p: HelloParams = params(req.params)?;`.
fn params<T: DeserializeOwned>(v: Value) -> Result<T, RpcError> {
    let v = if v.is_null() { Value::Object(Default::default()) } else { v };
    serde_json::from_value(v).map_err(|e| RpcError::new(error_code::INVALID_PARAMS, e.to_string()))
}

/// Maps a store error to a JSON-RPC error with a stable code.
fn store_error(e: StoreError) -> RpcError {
    let code = match e {
        StoreError::NotFound(_) => error_code::NOT_FOUND,
        StoreError::InvalidMove(_) => error_code::INVALID_MOVE,
        StoreError::InvalidParams(_) => error_code::INVALID_PARAMS,
        // The client needs the address and the verdict to ask the user.
        StoreError::ConfirmationRequired { ref addr, ref verdict } => {
            let mut err = RpcError::new(error_code::CONFIRMATION_REQUIRED, e.to_string());
            err.data = Some(serde_json::json!({ "addr": addr, "verdict": verdict }));
            return err;
        }
        _ => error_code::INTERNAL_ERROR,
    };
    RpcError::new(code, e.to_string())
}

/// Serializes a result into a JSON `Value`.
fn to_value<T: serde::Serialize>(v: T) -> Result<Value, RpcError> {
    serde_json::to_value(v).map_err(|e| RpcError::new(error_code::INTERNAL_ERROR, e.to_string()))
}

/// Runs `f` on the store, on a blocking thread, and waits for the result
/// without blocking the async runtime.
///
/// Rust note on the signature:
/// - `impl FnOnce(&mut Store) -> ...` accepts any closure that can be called
///   once with the store.
/// - `Send` means the closure can be moved to another thread, and
///   `'static` that it does not borrow anything from the caller's stack
///   (the caller may be paused while the closure runs elsewhere). That is
///   why callers write `move |s| ...`: the closure takes ownership of what
///   it uses.
async fn with_store<T: Send + 'static>(
    core: &Arc<Core>,
    f: impl FnOnce(&mut Store) -> Result<T, StoreError> + Send + 'static,
) -> Result<T, RpcError> {
    let core = core.clone();
    tokio::task::spawn_blocking(move || {
        // `unwrap` on `lock()`: it fails only if another thread panicked
        // while holding the lock ("poisoned" mutex), a bug we do not try to
        // recover from.
        let mut store = core.store.lock().unwrap();
        f(&mut store)
    })
    .await
    // Two layers of errors: the first `map_err` handles a failure of the
    // blocking task itself (a panic), the second converts a store error.
    .map_err(|e| RpcError::new(error_code::INTERNAL_ERROR, e.to_string()))?
    .map_err(store_error)
}

/// Runs the query `f` on a connection of the read pool, on a blocking
/// thread. Unlike `with_store`, it never waits for a write in progress.
async fn with_reader<T: Send + 'static>(
    core: &Arc<Core>,
    f: impl FnOnce(&Store) -> Result<T, StoreError> + Send + 'static,
) -> Result<T, RpcError> {
    let core = core.clone();
    tokio::task::spawn_blocking(move || core.readers.read(f))
        .await
        .map_err(|e| RpcError::new(error_code::INTERNAL_ERROR, e.to_string()))?
        .map_err(store_error)
}

/// Where the passwords of the accounts come from. The `password` key of
/// `config.toml` only in a build with the `dev-passwords` feature; the
/// Keychain arrives with `add_account` (`plan-imap.md`, phase 7).
fn secrets() -> Box<dyn nm_engine::secrets::Secrets> {
    if cfg!(feature = "dev-passwords") {
        Box::new(nm_engine::secrets::ConfigPasswords)
    } else {
        Box::new(nm_engine::secrets::NoSecrets)
    }
}

/// Reads `config.toml` again after a change seen by the watch, and emits
/// what it changed. Runs on the thread of the watch: it may block on the
/// store while a command or an import batch finishes.
fn reload_config(core: &Arc<Core>) {
    // Lock order: store, engine, events. The events are emitted with the
    // store's lock held, like those of the commands, so their order is the
    // order of the writes.
    let mut store = core.store.lock().unwrap();
    let mut engine = core.engine.lock().unwrap();
    match engine.reload(&mut store, false) {
        Ok(events) => {
            if !events.is_empty() {
                log_config(&engine);
            }
            let mut log = core.events.lock().unwrap();
            for e in events {
                log.push(e);
            }
        }
        Err(e) => eprintln!("config : {e}"),
    }
}

/// Prints the state of the settings on the core's standard error.
fn log_config(engine: &Engine) {
    let path = engine.path().display();
    match engine.error() {
        Some(p) => eprintln!("config : {path} refusé, réglages précédents conservés : {p}"),
        None => eprintln!("config : {path}, {} compte(s)", engine.config().accounts.len()),
    }
    for w in engine.warnings() {
        eprintln!("config : avertissement : {w}");
    }
}

/// Runs a write command and emits its events once the transaction is
/// committed. Returns the number of messages changed.
async fn command(
    core: &Arc<Core>,
    f: impl FnOnce(&mut Store) -> Result<ChangeSet, StoreError> + Send + 'static,
) -> Result<Value, RpcError> {
    // A second pointer to the core, moved into the closure below.
    let core2 = core.clone();
    let changed = with_store(core, move |store| {
        let changes = f(store)?;
        // Emitted while still holding the store lock: the order of sequence
        // numbers then matches the order of transactions. If the lock were
        // released first, two concurrent commands could emit their events in
        // the opposite order of their commits.
        core2.events.lock().unwrap().emit(&changes);
        Ok(changes.changed_messages())
    })
    .await?;
    to_value(CommandResult { changed })
}

/// Runs a workflow command (move, decision, undo) like `command`, and
/// returns what it did: mails filed, decision taken, undo token.
async fn workflow(
    core: &Arc<Core>,
    f: impl FnOnce(&mut Store) -> Result<Outcome, StoreError> + Send + 'static,
) -> Result<Value, RpcError> {
    let core2 = core.clone();
    let result = with_store(core, move |store| {
        let out = f(store)?;
        core2.events.lock().unwrap().emit(&out.changes);
        Ok(out.view())
    })
    .await?;
    to_value(result)
}

/// Page size requested by the client, or the default, clamped to the
/// allowed range.
fn limit(l: Option<u32>) -> u32 {
    l.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
}

/// Routes a request to the code of its method.
async fn dispatch(core: &Arc<Core>, session: &mut Session, req: Request) -> Result<Value, RpcError> {
    if req.jsonrpc != "2.0" {
        return Err(RpcError::new(error_code::INVALID_REQUEST, "jsonrpc doit valoir \"2.0\""));
    }
    // First match: methods available before the handshake (`ping`, `hello`),
    // and refusal of everything else until `hello` succeeds.
    match req.method.as_str() {
        method::PING => return Ok(Value::String("pong".into())),
        method::HELLO => {
            let p: HelloParams = params(req.params)?;
            if p.protocol_version != PROTOCOL_VERSION {
                return Err(RpcError::new(
                    error_code::PROTOCOL_VERSION,
                    format!("protocole {} non pris en charge (attendu : {PROTOCOL_VERSION})", p.protocol_version),
                ));
            }
            // Provisional: the identity will come from the client's code
            // signature, never from what the client declares.
            session.source = match p.client.as_deref() {
                Some("app") => Source::App,
                Some("tui") => Source::Tui,
                _ => Source::Cli,
            };
            session.hello = true;
            return to_value(HelloResult {
                protocol_version: PROTOCOL_VERSION,
                // `env!` reads a variable at compile time; Cargo sets
                // CARGO_PKG_VERSION to the crate's version.
                core_version: env!("CARGO_PKG_VERSION").into(),
            });
        }
        _ if !session.hello => {
            return Err(RpcError::new(error_code::PROTOCOL_VERSION, "poignée de main `hello` attendue"));
        }
        _ => {}
    }

    // Event position, read *before* running a query and returned as `as_of`.
    // Any event with a higher seq may not be reflected in the data, so the
    // client knows it must apply it.
    let as_of = || core.events.lock().unwrap().position();
    let source = session.source;
    match req.method.as_str() {
        method::SUBSCRIBE => {
            let p: SubscribeParams = params(req.params)?;
            if session.subscribed {
                return Err(RpcError::new(error_code::INVALID_REQUEST, "déjà abonné"));
            }
            // Everything is computed under the same lock: the replay, the
            // receiver and the position are consistent with one another. The
            // inner block releases the lock before going on.
            let (replay, rx, pos, resync) = {
                let log = core.events.lock().unwrap();
                let (replay, rx) = log.subscribe(p.since.as_ref());
                (replay, rx, log.position(), log.resync_line())
            };
            session.subscribed = true;
            let replayed = matches!(replay, Replay::Events(_));
            let out = session.out.clone();
            tokio::spawn(forward(core.clone(), rx, out, replay, resync));
            to_value(SubscribeResult { epoch: pos.epoch, seq: pos.seq, replayed })
        }
        method::LIST_FOLDERS => {
            let as_of = as_of();
            let items = with_reader(core, |s| s.list_folders()).await?;
            to_value(FolderList { items, as_of })
        }
        method::LIST_THREADS => {
            let p: ListThreadsParams = params(req.params)?;
            let as_of = as_of();
            // Rust note: an enum local to this arm says which of the three
            // exclusive parameters was given, so the closure below gets a
            // single value to match on.
            enum Scope {
                Folders(Vec<i64>),
                Everything(Option<i64>),
            }
            let scope = match (p.folder_id, p.folder_ids, p.everything) {
                (Some(f), None, None) => Scope::Folders(vec![f]),
                (None, Some(fs), None) if !fs.is_empty() => Scope::Folders(fs),
                (None, None, Some(e)) => Scope::Everything(e.account_id),
                _ => {
                    return Err(RpcError::new(
                        error_code::INVALID_PARAMS,
                        "un seul de folder_id, folder_ids, everything attendu",
                    ));
                }
            };
            let (items, next_cursor) = with_reader(core, move |s| match scope {
                Scope::Folders(folders) => {
                    s.list_threads_in(&folders, p.cursor.as_deref(), limit(p.limit), p.sort, &p.filter)
                }
                Scope::Everything(account) => {
                    s.list_everything(account, p.cursor.as_deref(), limit(p.limit), p.sort, &p.filter)
                }
            })
            .await?;
            to_value(Page { items, next_cursor, as_of })
        }
        method::GET_THREAD => {
            let p: GetThreadParams = params(req.params)?;
            let as_of = as_of();
            let (subject, messages) = with_reader(core, move |s| s.get_thread(p.thread_id)).await?;
            to_value(ThreadView { id: p.thread_id, subject, messages, as_of })
        }
        method::SEARCH => {
            let p: SearchParams = params(req.params)?;
            let as_of = as_of();
            let items = with_reader(core, move |s| s.search(&p.query, limit(p.limit))).await?;
            to_value(SearchResult { items, as_of })
        }
        method::MOVE_THREAD => {
            let p: MoveThreadParams = params(req.params)?;
            let opts = MoveOptions { confirm_suspect: p.confirm_suspect, auth_policy: p.auth_policy };
            workflow(core, move |s| s.move_thread(p.thread_id, p.target, p.from_folder_id, opts, source)).await
        }
        method::SET_ADDRESS_SCREENING => {
            let p: SetAddressScreeningParams = params(req.params)?;
            workflow(core, move |s| s.set_address_screening(&p.addr, p.target, source)).await
        }
        method::SET_DOMAIN_SCREENING => {
            let p: SetDomainScreeningParams = params(req.params)?;
            workflow(core, move |s| s.set_domain_screening(&p.domain, p.target, source)).await
        }
        method::UNDO => {
            let p: UndoParams = params(req.params)?;
            workflow(core, move |s| s.undo(&p.token, source)).await
        }
        method::SET_THREAD_SEEN => {
            let p: SetThreadSeenParams = params(req.params)?;
            command(core, move |s| s.set_thread_seen(p.thread_id, p.seen, source)).await
        }
        method::SET_THREAD_FLAGGED => {
            let p: SetThreadFlaggedParams = params(req.params)?;
            command(core, move |s| s.set_thread_flagged(p.thread_id, p.flagged, source)).await
        }
        method::IMPORT_MBOX => {
            let p: ImportMboxParams = params(req.params)?;
            to_value(start_import(core, p, source).await?)
        }
        method::IMPORT_STATUS => to_value(core.import.lock().unwrap().status.clone()),
        method::SET_BODY_FORMAT => {
            let p: SetBodyFormatParams = params(req.params)?;
            with_store(core, move |s| s.set_body_format(&p.addr, p.format.as_deref())).await?;
            to_value(CommandResult { changed: 1 })
        }
        method::EXPORT_MESSAGE => {
            let p: ExportMessageParams = params(req.params)?;
            let dir = match p.dir {
                Some(d) => PathBuf::from(d),
                None => std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join("Downloads"),
            };
            if !dir.is_absolute() {
                return Err(RpcError::new(error_code::INVALID_PARAMS, "dossier absolu attendu"));
            }
            let raw = core.raw.clone();
            let path = with_reader(core, move |s| s.export_message(&raw, p.message_id, p.format, &dir)).await?;
            to_value(ExportResult { path: path.display().to_string() })
        }
        method::UNSUBSCRIBE => {
            // `ui.md`, "Désabonnement". With one click (RFC 8058), the core
            // sends the request itself and records it; otherwise the client
            // opens the page or the address. A failed one-click attempt
            // falls back to opening the page.
            let p: UnsubscribeParams = params(req.params)?;
            let id = p.message_id;
            let target = with_reader(core, move |s| s.unsubscribe_target(id)).await?;
            let browser = |error: Option<String>| UnsubscribeResult {
                method: "browser".into(),
                done: false,
                open: target.url.clone(),
                error,
            };
            if target.one_click
                && let Some(url) = target.url.clone()
            {
                // The request is blocking and may take seconds: off the
                // runtime, so other clients are not kept waiting.
                // `NICEMAILER_FAKE_NETWORK` (UI tests): no request leaves the
                // machine, the server is taken to have accepted.
                let fake = std::env::var_os("NICEMAILER_FAKE_NETWORK").is_some();
                let sent = tokio::task::spawn_blocking(move || {
                    if fake { Ok(()) } else { nm_proto::unsubscribe::one_click(&url, true) }
                })
                .await
                .map_err(|e| RpcError::new(error_code::INTERNAL_ERROR, e.to_string()))?;
                return match sent {
                    Ok(()) => {
                        let core2 = core.clone();
                        with_store(core, move |s| {
                            let changes = s.mark_unsubscribed(id, source)?;
                            core2.events.lock().unwrap().emit(&changes);
                            Ok(())
                        })
                        .await?;
                        to_value(UnsubscribeResult { method: "one_click".into(), done: true, open: None, error: None })
                    }
                    Err(e) => to_value(browser(Some(e.to_string()))),
                };
            }
            if target.url.is_some() {
                return to_value(browser(None));
            }
            to_value(UnsubscribeResult { method: "mail".into(), done: false, open: target.mailto.clone(), error: None })
        }
        method::GET_ATTACHMENT => {
            // The file is extracted from the raw message on first use; the
            // client receives its path, never its bytes.
            let p: GetAttachmentParams = params(req.params)?;
            let (raw, files) = (core.raw.clone(), core.attachments.clone());
            let f = with_store(core, move |s| s.get_attachment(&raw, &files, p.attachment_id)).await?;
            to_value(AttachmentFileView {
                path: f.path.display().to_string(),
                filename: f.filename,
                mime_type: f.mime_type,
                size: f.size,
            })
        }
        method::LIST_HISTORY => {
            let p: ListHistoryParams = params(req.params)?;
            let as_of = as_of();
            let (items, next_cursor) = with_reader(core, move |s| s.list_history(p.cursor.as_deref(), limit(p.limit))).await?;
            to_value(Page { items, next_cursor, as_of })
        }
        method::LIST_TAGS => {
            let p: ListTagsParams = params(req.params)?;
            let as_of = as_of();
            let items = with_reader(core, move |s| s.list_tags(p.account_id)).await?;
            to_value(Items { items, as_of })
        }
        method::TAG_THREAD => {
            let p: TagThreadParams = params(req.params)?;
            command(core, move |s| s.tag_thread(p.thread_id, &p.name, p.remove, source)).await
        }
        method::LIST_GROUPS => {
            let as_of = as_of();
            let items = with_reader(core, |s| s.list_groups()).await?;
            to_value(Items { items, as_of })
        }
        method::SET_GROUP_MEMBER => {
            let p: SetGroupMemberParams = params(req.params)?;
            let (group_id, addr, changed) =
                with_store(core, move |s| s.set_group_member(&p.group, p.addr.as_deref(), p.thread_id, p.remove, source))
                    .await?;
            to_value(GroupMemberResult { group_id, addr, changed })
        }
        method::LIST_CONTACTS => {
            let p: ListContactsParams = params(req.params)?;
            let as_of = as_of();
            let (items, next_cursor) = with_reader(core, move |s| {
                s.list_contacts(p.query.as_deref(), p.cursor.as_deref(), limit(p.limit))
            })
            .await?;
            to_value(Page { items, next_cursor, as_of })
        }
        method::LIST_ACCOUNTS => {
            let as_of = as_of();
            let rows = with_reader(core, |s| s.list_account_rows()).await?;
            // The rows are read first, then the engine's state: the engine
            // lock is never held while waiting for the database.
            let items = core.engine.lock().unwrap().account_views(rows);
            to_value(Items { items, as_of })
        }
        method::SET_ACCOUNT_ORDER => {
            let p: SetAccountOrderParams = params(req.params)?;
            command(core, move |s| s.set_account_order(&p.account_ids, source)).await
        }
        method::CONFIG_STATUS => {
            let as_of = as_of();
            let engine = core.engine.lock().unwrap();
            to_value(ConfigStatus {
                path: engine.path().display().to_string(),
                exists: engine.path().exists(),
                error: engine.error().map(nm_engine::problem_view),
                warnings: engine.warnings().iter().map(nm_engine::problem_view).collect(),
                as_of,
            })
        }
        other => Err(RpcError::new(error_code::METHOD_NOT_FOUND, format!("méthode inconnue : {other}"))),
    }
}

/// Relays events to a subscribed client: first the replay, then the live
/// events.
///
/// The core never waits for a client. A client that reads too slowly makes
/// its broadcast receiver fall behind ("lag"); it then receives a single
/// `ResyncRequired` instead of the lost events, and reloads what it shows.
async fn forward(
    core: Arc<Core>,
    mut rx: broadcast::Receiver<(u64, Arc<String>)>,
    out: mpsc::Sender<Arc<String>>,
    replay: Replay,
    resync: Arc<String>,
) {
    let initial = match replay {
        Replay::Resync => vec![resync],
        Replay::Events(lines) => lines,
        Replay::None => vec![],
    };
    for l in initial {
        // `send` fails when the connection is closed: stop relaying.
        if out.send(l).await.is_err() {
            return;
        }
    }
    loop {
        let line = match rx.recv().await {
            Ok((_, line)) => line,
            Err(broadcast::error::RecvError::Lagged(_)) => {
                // While we hold the log lock, no event can be emitted. After
                // draining the receiver, the current position is therefore
                // exactly the one the client will be at, and the resync line
                // carries it. The guard `log` is dropped at the end of this
                // block, before the `.await` below: holding a std mutex across
                // an await point could block other tasks.
                let log = core.events.lock().unwrap();
                while rx.try_recv().is_ok() {}
                log.resync_line()
            }
            // The sender was dropped: the core is shutting down.
            Err(broadcast::error::RecvError::Closed) => return,
        };
        if out.send(line).await.is_err() {
            return;
        }
    }
}

/// `import_mbox`: checks the request, creates the archive account, then
/// starts the import on a dedicated thread and answers at once with the
/// initial status. Progress and the end are reported by events.
///
/// Why a plain thread rather than `spawn_blocking`: an import of a large
/// mailbox runs for minutes, and tokio's blocking pool is meant for short
/// tasks. The import thread takes the store lock one batch at a time, so
/// requests from the app keep being answered meanwhile.
async fn start_import(core: &Arc<Core>, p: ImportMboxParams, source: Source) -> Result<ImportStatus, RpcError> {
    let invalid = |m: String| RpcError::new(error_code::INVALID_PARAMS, m);
    let path = PathBuf::from(&p.path);
    if !path.is_absolute() {
        return Err(invalid(format!("chemin absolu attendu : {}", p.path)));
    }
    let meta = fs::metadata(&path).map_err(|e| invalid(format!("{} : {e}", p.path)))?;
    if !meta.is_file() {
        return Err(invalid(format!("{} n'est pas un fichier", p.path)));
    }

    // Reserve the import slot before doing anything slow, so two requests
    // cannot both start an import.
    let job_id = {
        let mut st = core.import.lock().unwrap();
        if st.status.as_ref().is_some_and(|s| s.running) {
            return Err(RpcError::new(error_code::BUSY, "un import est déjà en cours"));
        }
        st.last_job += 1;
        st.status = Some(ImportStatus {
            job_id: st.last_job,
            path: p.path.clone(),
            running: true,
            total_bytes: meta.len(),
            ..Default::default()
        });
        st.last_job
    };
    // Created here rather than in the thread, so that a refused name or
    // address is reported as the error of this request.
    let (name, address) = (p.account.clone(), p.address.clone());
    let account = with_store(core, move |s| s.prepare_archive_account(&name, &address, source)).await;
    let account = match account {
        Ok(a) => a,
        Err(e) => {
            if let Some(s) = core.import.lock().unwrap().status.as_mut() {
                s.running = false;
                s.error = Some(e.message.clone());
            }
            return Err(e);
        }
    };
    let status = {
        let mut st = core.import.lock().unwrap();
        let s = st.status.as_mut().expect("status set above");
        s.account_id = account.id;
        s.clone()
    };

    let opts = ImportOptions {
        path,
        account: p.account,
        address: p.address,
        mark_correspondents: p.mark_correspondents,
        source,
        batch_size: BATCH_SIZE,
    };
    let core = core.clone();
    std::thread::Builder::new()
        .name("nicemailer-import".into())
        .spawn(move || run_import(core, job_id, opts))
        .map_err(|e| RpcError::new(error_code::INTERNAL_ERROR, e.to_string()))?;
    Ok(status)
}

/// Starts the reinterpretation of the bodies of an older parser, if any
/// (`stockage-brut.md`, section 8): after an improvement of the parser, the
/// stored mails get it in the background, without any command. A plain
/// thread, as for the import: it runs for minutes on a large mailbox, and
/// takes the store lock one batch at a time, so the app is answered
/// meanwhile.
///
/// No event is sent: a body only shows when a thread is opened, and an
/// event per batch would make the app reload its list hundreds of times.
/// A thread already open keeps its old body until opened again.
fn start_reindex(core: &Arc<Core>) -> anyhow::Result<()> {
    let core = core.clone();
    // Everything happens on the reindex thread, the check included: the
    // main thread must reach the loop that accepts connections at once
    // (a slow check here once left the app unanswered for 30 s).
    std::thread::Builder::new().name("nicemailer-reindex".into()).spawn(move || {
        let pending = match nm_store::reindex::pending(&core.store.lock().unwrap()) {
            Ok(0) => return,
            Ok(n) => n,
            Err(e) => return eprintln!("reindex : {e}"),
        };
        eprintln!("reindex : {pending} mails analysés par une version antérieure, en tâche de fond");
        let mut last_logged = std::time::Instant::now();
        let mut progress = |done: u64, total: u64| {
            if last_logged.elapsed().as_secs() >= 10 {
                eprintln!("reindex : {done} / {total}");
                last_logged = std::time::Instant::now();
            }
        };
        match nm_store::reindex::reindex(&core.store, &core.raw, nm_store::reindex::BATCH_SIZE, &mut progress) {
            Ok(r) => eprintln!(
                "reindex terminé : {} mails réanalysés, {} corrigés sans relecture, {} inchangés, {} bruts introuvables, {} illisibles, {} noms d'adresses complétés, en {:.1} s",
                r.reparsed, r.repaired, r.unchanged, r.missing_raw, r.parse_errors, r.names_filled, r.seconds
            ),
            Err(e) => eprintln!("reindex échoué : {e}"),
        }
    })?;
    Ok(())
}

/// Body of the import thread.
fn run_import(core: Arc<Core>, job_id: u64, opts: ImportOptions) {
    // Called after each batch, with the store lock held: the events are
    // therefore ordered exactly like the writes.
    let mut on_batch = |_: &Store, changes: &ChangeSet, p: &ImportProgress| {
        let status = {
            let mut st = core.import.lock().unwrap();
            let s = st.status.as_mut().expect("import status");
            s.bytes_read = p.bytes_read;
            s.total_bytes = p.total_bytes;
            s.imported = p.imported;
            s.skipped = p.skipped;
            s.parse_errors = p.parse_errors;
            s.clone()
        };
        let mut events = core.events.lock().unwrap();
        events.emit(changes);
        events.push(Event::ImportProgress(status));
    };
    let result = import_mbox(&core.store, &core.raw, &opts, &mut on_batch);

    let status = {
        let mut st = core.import.lock().unwrap();
        let s = st.status.as_mut().expect("import status");
        s.running = false;
        match &result {
            Ok(r) => {
                s.bytes_read = r.progress.bytes_read;
                s.imported = r.progress.imported;
                s.skipped = r.progress.skipped;
                s.parse_errors = r.progress.parse_errors;
                s.sent = r.sent;
                s.archived = r.archived;
                s.correspondents = r.correspondents;
                s.seconds = r.seconds;
            }
            Err(e) => s.error = Some(e.to_string()),
        }
        s.clone()
    };
    match &result {
        Ok(r) => eprintln!(
            "import {job_id} terminé : {} mails importés, {} déjà présents, en {:.1} s",
            r.progress.imported, r.progress.skipped, r.seconds
        ),
        Err(e) => eprintln!("import {job_id} échoué : {e}"),
    }
    core.events.lock().unwrap().push(Event::ImportFinished(status));
}
