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
//! (`spawn_blocking`, see `with_store`).
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
    CommandResult, DEFAULT_LIMIT, Event, ExportMessageParams, ExportResult, FolderList, GetThreadParams,
    GroupMemberResult, HelloParams, HelloResult, ImportMboxParams, ImportStatus, Items, ListContactsParams,
    ListHistoryParams, ListTagsParams, ListThreadsByParams, ListThreadsParams, MAX_LIMIT, MoveThreadParams,
    PROTOCOL_VERSION, Page, Request, Response, RpcError, SearchParams, SearchResult, SetBodyFormatParams,
    SetGroupMemberParams, SetThreadFlaggedParams, SetThreadSeenParams, SubscribeParams, SubscribeResult,
    TagThreadParams, ThreadView, error_code, method,
};
use nm_core::Source;
use nm_store::import::{BATCH_SIZE, ImportOptions, ImportProgress, import_mbox};
use nm_store::raw::RawStore;
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
}

/// State shared by all connections.
///
/// Rust note: a `Mutex` guarantees that only one thread at a time accesses
/// the value inside. `lock()` returns a guard; the lock is released when the
/// guard goes out of scope. The store and the event log have separate locks
/// so that reading the event position never waits for a slow query.
///
/// Lock order, to avoid deadlocks: `store`, then `import`, then `events`.
/// A thread holding one of them never waits for one that comes earlier in
/// this list.
struct Core {
    store: Mutex<Store>,
    events: Mutex<EventLog>,
    /// Immutable raw message files (`stockage-brut.md`).
    raw: RawStore,
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
    let store = Store::open(&db_path).with_context(|| format!("ouverture de {}", db_path.display()))?;
    // `Arc` so that every connection task can hold a pointer to the shared
    // state. Replay buffer of 10 000 events; a client may lag 1 000 events
    // behind before being told to resync.
    let raw = RawStore::open(&data_dir).with_context(|| format!("stockage brut dans {}", data_dir.display()))?;
    let core = Arc::new(Core {
        store: Mutex::new(store),
        events: Mutex::new(EventLog::new(10_000, 1_000)),
        raw,
        import: Mutex::new(ImportState::default()),
    });

    let socket_path = data_dir.join(nm_api::paths::SOCKET);
    // A socket file left behind by a crash would make `bind` fail. Thanks to
    // the lock above, we know no other core is using it.
    let _ = fs::remove_file(&socket_path);
    let listener = UnixListener::bind(&socket_path)?;
    // 0o600: only the current user can connect.
    fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))?;
    eprintln!("nicemailer-core : base {}, socket {}", db_path.display(), socket_path.display());

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
    let (out, mut rx) = mpsc::channel::<Arc<String>>(1_000);
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
            let items = with_store(core, |s| s.list_folders()).await?;
            to_value(FolderList { items, as_of })
        }
        method::LIST_THREADS => {
            let p: ListThreadsParams = params(req.params)?;
            let as_of = as_of();
            let (items, next_cursor) =
                with_store(core, move |s| s.list_threads(p.folder_id, p.cursor.as_deref(), limit(p.limit), p.sort)).await?;
            to_value(Page { items, next_cursor, as_of })
        }
        method::GET_THREAD => {
            let p: GetThreadParams = params(req.params)?;
            let as_of = as_of();
            let (subject, messages) = with_store(core, move |s| s.get_thread(p.thread_id)).await?;
            to_value(ThreadView { id: p.thread_id, subject, messages, as_of })
        }
        method::SEARCH => {
            let p: SearchParams = params(req.params)?;
            let as_of = as_of();
            let items = with_store(core, move |s| s.search(&p.query, limit(p.limit))).await?;
            to_value(SearchResult { items, as_of })
        }
        method::MOVE_THREAD => {
            let p: MoveThreadParams = params(req.params)?;
            command(core, move |s| s.move_thread(p.thread_id, p.target, p.from_folder_id, source)).await
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
            let path = with_store(core, move |s| s.export_message(&raw, p.message_id, p.format, &dir)).await?;
            to_value(ExportResult { path: path.display().to_string() })
        }
        method::LIST_HISTORY => {
            let p: ListHistoryParams = params(req.params)?;
            let as_of = as_of();
            let (items, next_cursor) = with_store(core, move |s| s.list_history(p.cursor.as_deref(), limit(p.limit))).await?;
            to_value(Page { items, next_cursor, as_of })
        }
        method::LIST_TAGS => {
            let p: ListTagsParams = params(req.params)?;
            let as_of = as_of();
            let items = with_store(core, move |s| s.list_tags(p.account_id)).await?;
            to_value(Items { items, as_of })
        }
        method::TAG_THREAD => {
            let p: TagThreadParams = params(req.params)?;
            command(core, move |s| s.tag_thread(p.thread_id, &p.name, p.remove, source)).await
        }
        method::LIST_GROUPS => {
            let as_of = as_of();
            let items = with_store(core, |s| s.list_groups()).await?;
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
            let (items, next_cursor) = with_store(core, move |s| {
                s.list_contacts(p.query.as_deref(), p.cursor.as_deref(), limit(p.limit))
            })
            .await?;
            to_value(Page { items, next_cursor, as_of })
        }
        method::LIST_THREADS_BY => {
            let p: ListThreadsByParams = params(req.params)?;
            let as_of = as_of();
            let items =
                with_store(core, move |s| s.list_threads_by(p.tag_id, p.group_id, p.address_id, limit(p.limit))).await?;
            to_value(Items { items, as_of })
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
