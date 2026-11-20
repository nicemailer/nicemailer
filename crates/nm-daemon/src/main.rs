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
//! The core creates `config.toml` and `accounts.toml` at start when they
//! are missing, reads them and watches them (`nm-engine`): a valid change
//! is applied at once, an invalid file is refused whole and reported
//! (`configuration.md`, sections 3 and 4).
//!
//! Per connection there are:
//! - a reading loop (`serve`) that handles one request at a time,
//! - a writer task that sends the lines queued in an `mpsc` channel,
//! - after `subscribe`, a `forward` task that relays events into the same
//!   channel.
//!
//! Funnelling responses and events through one channel guarantees that two
//! lines are never interleaved on the socket.

mod ai;
mod drafts;
mod events;
mod extract;
mod rights;
mod setup;
mod sync;
mod update;

use std::fs::{self, File};
// Unix-specific extension trait that adds `from_mode` (chmod-style
// permissions) to `fs::Permissions`.
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

// `anyhow` provides a catch-all error type for applications: any error can
// be converted into `anyhow::Error` with `?`, and `.context(...)` adds an
// explanation.
use anyhow::Context;
use clap::Parser;
use nm_api::reason;
use nm_api::{
    AttachmentFileView, CommandResult, DEFAULT_LIMIT, Event, ExportMessageParams, ExportResult, FolderList, MessageRecordParams, MessageTextParams, MessageTextResult,
    GetAttachmentParams, GetThreadParams, LocateMessageParams, UnsubscribeParams, UnsubscribeResult,
    DeleteGroupResult, DeleteNameParams, GroupMemberResult, HelloParams, HelloResult, ImportMboxParams, ImportStatus, Items, ListContactsParams, RenameParams, RenameResult,
    ListHistoryGroupsParams, ListHistoryParams, ListTagsParams, ListThreadsParams, MAX_LIMIT, MoveThreadParams,
    PROTOCOL_VERSION, Page, ProbeAccountParams, ProbeReport, Request, Response, RpcError, SearchParams, SearchResult, SetBodyFormatParams,
    SetGroupMemberParams, SetThreadFlaggedParams, ThreadLabelsParams, SetThreadSeenParams, SubscribeParams, SubscribeResult,
    SetAddressScreeningParams, SetDomainScreeningParams, SetSenderKindParams, TagThreadParams, ThreadView, UndoParams, error_code,
    method, ConfigStatus, SetAccountOrderParams, SetPasswordParams,
};
use nm_core::Source;
use nm_engine::Engine;
use nm_store::pool::ReadPool;
use nm_store::import::{BATCH_SIZE, ImportOptions, ImportProgress, LOCK_BUDGET, import_mbox};
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
use std::time::{Duration, Instant};

use tokio::sync::{OwnedSemaphorePermit, Semaphore, broadcast, mpsc};

use events::{EventLog, Replay};

/// Sends the standard error of the core (every message it prints) to
/// `path`, appended; a log larger than 10 MB is renamed `core.log.1` first,
/// so that it never grows without bound.
fn log_to_file(path: &Path) {
    if fs::metadata(path).is_ok_and(|m| m.len() > 10 << 20) {
        let _ = fs::rename(path, path.with_extension("log.1"));
    }
    let Ok(file) = fs::OpenOptions::new().create(true).append(true).open(path) else { return };
    // Rust note: `dup2` is a C function of the system: it makes file
    // descriptor 2 (standard error) point to the log file. `unsafe`
    // because the compiler cannot check C code; both descriptors are valid.
    use std::os::fd::AsRawFd;
    unsafe {
        libc::dup2(file.as_raw_fd(), 2);
    }
    eprintln!(
        "--- nicemailer-core {} started at {} ---",
        env!("CARGO_PKG_VERSION"),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
    );
}

/// Command-line arguments. `clap` generates the parser (and `--help`) from
/// this struct: each field becomes an option, and the `///` comment
/// becomes its help text, which is why it stays in French (user-facing).
#[derive(Parser)]
#[command(name = "nicemailer-core", version, about = "Noyau NiceMailer")]
struct Args {
    /// Dossier de données (base, socket, verrou).
    #[arg(long, env = "NICEMAILER_DATA_DIR")]
    data_dir: Option<PathBuf>,
    /// Fichier de réglages (par défaut ~/.config/nicemailer/config.toml) ;
    /// les comptes sont dans accounts.toml, à côté.
    #[arg(long, env = "NICEMAILER_CONFIG")]
    config: Option<PathBuf>,
    /// Trousseau des mots de passe : un fichier de trousseau déverrouillé,
    /// au lieu du trousseau de session (tests, trousseau de développement).
    #[arg(long, env = "NICEMAILER_KEYCHAIN")]
    keychain: Option<PathBuf>,
    /// Internal: reads the text of this attachment, writes it as one JSON
    /// line and exits (`extract.rs`, the reader of PDF and DOCX files run
    /// in a process of its own).
    #[arg(long, hide = true, requires = "extract_kind")]
    extract_text: Option<PathBuf>,
    #[arg(long, hide = true)]
    extract_kind: Option<String>,
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
    /// The writing store. Taken through [`Core::lock_store`], which times
    /// the wait and the hold.
    store: Mutex<Store>,
    /// Read-only connections for the queries of the clients.
    readers: ReadPool,
    /// Read-only connections for the synchronization workers
    /// (`CoreHost::read`). A pool of their own: the reads of the workers
    /// (the bodies to download, the folders to compare) are long during an
    /// import, and must never take every connection the app's lists need.
    sync_readers: ReadPool,
    /// `NICEMAILER_TEST_WRITE_DELAY_MS`: each write holds the store's lock
    /// that much longer. Only for the tests of the core, which need a slow
    /// write to check that reads do not wait for it (`tests/parallel_rpc.rs`).
    test_write_delay: Option<Duration>,
    /// Settings applied and state of the accounts (`nm-engine`).
    engine: Mutex<Engine>,
    /// The keys of the app, `keymap.toml` (`raccourcis.md`). Apart from the
    /// engine: the file is read and refused on its own.
    keymap: Mutex<nm_engine::keymap::KeymapFile>,
    events: Mutex<EventLog>,
    /// `events.client_queue`: lines a client may have waiting.
    client_queue: usize,
    /// Immutable raw message files (`stockage-brut.md`).
    raw: RawStore,
    /// Attachments extracted from them on demand (`attachments.rs`).
    attachments: AttachmentFiles,
    /// Files of the mails to send: attachments of drafts, mails with
    /// attachments waiting to leave (`nm_store::outgoing`).
    outgoing: nm_store::outgoing::OutgoingStore,
    /// The running mbox import, or the last one.
    import: Mutex<ImportState>,
    /// The synchronization of the IMAP accounts (`sync.rs`). Set once the
    /// core is shared, since the workers hold a pointer to it.
    syncer: std::sync::OnceLock<nm_engine::sync::Syncer>,
    /// The AI assistance: prompts, runs, proposals (`ai.rs`).
    ai: ai::AiState,
    /// The maintenance under way, if any (`update.rs`): every request is
    /// refused meanwhile, and the connections close.
    maintenance: update::State,
    /// Raised by each `prepare_update` and `cancel_update`: a timer or a
    /// watch of an earlier update sees it changed and stops (`update.rs`).
    update_generation: std::sync::atomic::AtomicU64,
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
    // A child reading an attachment (`extract.rs`): nothing else, not even
    // the data directory.
    if let (Some(path), Some(kind)) = (&args.extract_text, &args.extract_kind) {
        extract::child_main(path, kind);
        return Ok(());
    }
    // `--data-dir` (or the env variable) if given, otherwise the default.
    let data_dir = args.data_dir.clone().unwrap_or_else(nm_api::paths::data_dir);
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
        // Another core serves this directory: nothing to do, and not a
        // failure. Code 0, so that launchd does not start this one again
        // and again (`KeepAlive` restarts only a failed exit,
        // `docs/app-bundle.md`).
        eprintln!("un autre noyau utilise déjà {} : rien à faire", data_dir.display());
        return Ok(());
    }
    // Started by the agent of NiceMailer.app, the core has no terminal:
    // its messages go to `core.log` in its data directory.
    if std::env::var_os("NICEMAILER_LOG_IN_DATA_DIR").is_some() {
        log_to_file(&data_dir.join("core.log"));
    }
    // Whatever stops the core from here on is also written where the app
    // finds it (`startup_error`): a core started by launchd has no
    // terminal, and the app, which can only see that no core answers,
    // would otherwise say nothing more useful than "unreachable".
    let result = run(args, &data_dir).await;
    if let Err(e) = &result {
        write_startup_error(&data_dir, e);
    }
    drop(lock);
    result
}

/// The core itself, once it holds its directory: settings, database,
/// socket, then the clients until it is asked to stop.
async fn run(args: Args, data_dir: &Path) -> anyhow::Result<()> {
    let data_dir = data_dir.to_path_buf();
    let db_path = data_dir.join(nm_api::paths::DATABASE);
    // How long each step before the socket takes: until the socket is
    // bound, the app can only wait ("starting the core…"), so this is the
    // part of the launch the user sees. Written once, when it is ready.
    let mut clock = StartupClock::new();
    let socket_path = data_dir.join(nm_api::paths::SOCKET);
    // The maintenance of the core (`update.rs`). A migration happens in
    // it: the socket is then opened before the database, to answer
    // `MAINTENANCE` meanwhile (`open_store`), and handed over afterwards.
    let maintenance = update::state();
    let (mut store, early_listener) = open_store(&db_path, &socket_path, &maintenance).await?;
    clock.mark("store");
    // Opened after the writing store, which creates or migrates the
    // database. Six connections for the clients: since the requests of a
    // connection run in parallel (`serve`), the app may have a list, a
    // thread, a search and a reply being prepared at once, and the CLI,
    // the TUI or an agent theirs. Three for the synchronization workers,
    // apart (see `Core::sync_readers`). A connection costs its page cache
    // (32 MB at most, `Store::open_reader`), filled only when used.
    let readers = ReadPool::open(&db_path, 6).with_context(|| format!("lecture de {}", db_path.display()))?;
    let sync_readers = ReadPool::open(&db_path, 3).with_context(|| format!("lecture de {}", db_path.display()))?;
    clock.mark("readers");

    // Settings, before the event log, whose sizes they give. An invalid
    // file does not stop the core: it starts with the defaults and no
    // account, and reports the error (`configuration.md`, section 4).
    // The core never shows a Keychain dialog (`nm-engine`, `keychain.rs`):
    // an access that would need one fails, and the account says why.
    nm_engine::keychain::never_prompt().map_err(|e| anyhow::anyhow!("trousseau : {e}"))?;
    let secrets: Arc<dyn nm_engine::secrets::Secrets> = match &args.keychain {
        Some(path) => Arc::new(nm_engine::keychain::Keychain::file(path).map_err(|e| anyhow::anyhow!(e))?),
        None => Arc::new(nm_engine::keychain::Keychain::login()),
    };
    let config_path = args.config.unwrap_or_else(nm_engine::config::default_path);
    // The files are created when missing, before they are read: a user
    // who changes nothing still finds every setting listed, commented out,
    // with its default (`configuration.md`, section 3). Failing to create
    // them (a read-only folder) is not fatal: missing files mean the
    // defaults and no account.
    match nm_engine::config_write::create_missing(&config_path) {
        Ok(created) => {
            for path in created {
                eprintln!("config : {} créé", path.display());
            }
        }
        Err(e) => eprintln!("config : création impossible ({e}) ; valeurs par défaut"),
    }
    let accounts_path = nm_engine::config::accounts_path(&config_path);
    let mut engine = Engine::new(config_path.clone(), secrets);
    let startup_events = engine.reload(&mut store, true)?;
    log_config(&engine);
    clock.mark("settings");
    let settings = engine.config().events.clone();
    let ai_settings = engine.config().ai.clone();

    let raw = RawStore::open(&data_dir).with_context(|| format!("stockage brut dans {}", data_dir.display()))?;
    let attachments = AttachmentFiles::open(&data_dir)
        .with_context(|| format!("pièces jointes dans {}", data_dir.display()))?;
    let outgoing = nm_store::outgoing::OutgoingStore::open(&data_dir)
        .with_context(|| format!("fichiers à envoyer dans {}", data_dir.display()))?;
    store.set_outgoing(outgoing.clone());
    // The keys: a file refused at start is reported like config.toml's
    // (`ConfigError`), with the defaults in effect.
    let keymap_path = nm_engine::keymap::path(&config_path);
    let mut keymap = nm_engine::keymap::KeymapFile::new(keymap_path.clone());
    let keymap_events = keymap.reload(true);
    log_keymap(&keymap);
    let mut events = EventLog::new(settings.replay_buffer as usize, settings.client_queue as usize);
    for e in startup_events.into_iter().chain(keymap_events) {
        events.push(e);
    }
    // `Arc` so that every connection task can hold a pointer to the shared
    // state.
    let core = Arc::new(Core {
        store: Mutex::new(store),
        readers,
        sync_readers,
        test_write_delay: std::env::var("NICEMAILER_TEST_WRITE_DELAY_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .map(Duration::from_millis),
        engine: Mutex::new(engine),
        keymap: Mutex::new(keymap),
        events: Mutex::new(events),
        client_queue: settings.client_queue as usize,
        raw,
        attachments,
        outgoing,
        import: Mutex::new(ImportState::default()),
        syncer: std::sync::OnceLock::new(),
        ai: ai::AiState::new(&config_path, &data_dir, &ai_settings),
        maintenance,
        update_generation: std::sync::atomic::AtomicU64::new(0),
    });
    // The workers of the IMAP accounts of the file (`plan-imap.md`,
    // phase 4). They only start working once the socket is up: their
    // first writes wait for the store's lock like any command.
    {
        let secrets = core.engine.lock().unwrap().secrets();
        let host: Arc<dyn nm_engine::sync::SyncHost> = Arc::new(sync::CoreHost(core.clone()));
        let _ = core.syncer.set(nm_engine::sync::Syncer::new(host, secrets));
        let mut store = core.lock_store("startup");
        // Actions that were being sent when the core stopped: sent again
        // (each can be replayed, `nm-store`, `spool.rs`).
        let requeued = store.spool_requeue_running()?;
        if requeued > 0 {
            eprintln!("spool : {requeued} action(s) interrompue(s) remise(s) en file");
        }
        // Pretended sending (`drafts.rs`): archive accounts may answer.
        store.set_fake_sending(drafts::fake_network());
        // A draft left frozen without its sending action is editable again.
        let repaired = store.repair_drafts()?;
        if repaired > 0 {
            eprintln!("brouillons : {repaired} brouillon(s) en cours d'envoi sans action, remis en rédaction");
        }
        sync::apply(&core, &mut store);
        // Drafts changed since their last copy (the app quit, or the core
        // stopped, with a reply open): closed now, so that they reach the
        // Drafts folder (`brouillons.md`, section 4, "Plantage"). After
        // `apply`, which tells which accounts have a server.
        for id in store.drafts_to_close()? {
            if let Err(e) = drafts::close_now(&core, &mut store, id, Source::Core) {
                eprintln!("brouillons : {id} non copié dans Drafts : {e}");
            }
        }
    }
    clock.mark("recovery");
    // Kept alive until the end of `main`: dropping it stops the watch. A
    // watch that cannot start is not fatal: the file is then read at the
    // next start only.
    let _watch = {
        let core = core.clone();
        // One watch for the three files: a change to any reads them again,
        // and each reload skips a file whose text did not change.
        match nm_engine::watch::watch(&[&config_path, &accounts_path, &keymap_path], move || {
            reload_config(&core);
            reload_keymap(&core);
        }) {
            Ok(w) => Some(w),
            Err(e) => {
                eprintln!("config : surveillance impossible ({e}) ; les fichiers seront relus au prochain démarrage");
                None
            }
        }
    };

    // Bound already when the database was migrated (`open_store`).
    let listener = match early_listener {
        Some(l) => l,
        None => bind(&socket_path)?,
    };
    // Started: a reason left by a previous failed start no longer applies.
    let _ = fs::remove_file(data_dir.join(nm_api::paths::STARTUP_ERROR));
    eprintln!("nicemailer-core : base {}, socket {}", db_path.display(), socket_path.display());
    clock.mark("socket");
    eprintln!("{}", clock.line());
    start_reindex(&core)?;
    start_sender_backfill(&core)?;
    sync::start_gc(&core);
    ai::start_upkeep(&core);

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
    // A batch in progress ends; the workers stop at their next check.
    if let Some(syncer) = core.syncer.get() {
        syncer.stop_all(std::time::Duration::from_secs(2));
    }
    // Both files emptied of their logs and marked, under the store lock so
    // that nothing writes in between: the next start skips the
    // reconciliation of the two files, most of its opening
    // (`nm_store::clean_close`). A write after this changes the files, and
    // the next start reconciles: never wrong, only slower.
    {
        let store = core.lock_store("close");
        match store.close_cleanly() {
            Ok(r) => eprintln!(
                "clean close: {} (checkpoint main {} ms, body {} ms)",
                if r.marked {
                    "the next start skips the reconciliation of the two files"
                } else {
                    "not marked, a reader was still busy; the next start reconciles"
                },
                r.main.as_millis(),
                r.body.as_millis()
            ),
            Err(e) => eprintln!("clean close: not marked ({e}); the next start reconciles"),
        }
    }
    cleanup(&socket_path);
    Ok(())
}

/// Creates the socket at `path`, readable by the current user only.
fn bind(path: &Path) -> anyhow::Result<UnixListener> {
    // A socket file left behind by a crash would make `bind` fail. Thanks to
    // the lock of the data directory, we know no other core is using it.
    let _ = fs::remove_file(path);
    let listener = UnixListener::bind(path)?;
    // 0o600: only the current user can connect.
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

/// Opens the writing store. A database of an earlier version is migrated
/// first, in maintenance (`mise-a-jour.md`, section 4; `update.rs`):
///
/// - the socket is opened at once, and every client gets the error
///   `MAINTENANCE` (reason `migration`, with the version reached so far)
///   rather than no answer, so that it waits instead of starting a core;
///   the socket is returned, to serve once the database is open;
/// - the database is copied first, and put back if the migration fails
///   (`nm_store::upgrade`); the core then stays in maintenance
///   (`migration_failed`) until it is stopped;
/// - a database written by a newer NiceMailer is never opened: the core
///   stays in maintenance (`newer_schema`) until it is stopped.
///
/// A migration can take minutes on a large mailbox (version 10 copies the
/// bodies: 4 minutes for 400,000 mails). It runs on a thread of its own
/// while this task answers the clients and watches for SIGTERM and Ctrl-C:
/// a stop cancels it and leaves with code 0, which launchd does not
/// restart. The migration of a version is one transaction: cancelled, it
/// is rolled back, and the migration starts again at the next start.
async fn open_store(
    db_path: &Path,
    socket_path: &Path,
    maintenance: &update::State,
) -> anyhow::Result<(Store, Option<UnixListener>)> {
    use nm_api::MaintenanceReason;
    use nm_store::inspect::DatabaseState;
    let inspecting = Instant::now();
    let state = nm_store::inspect::inspect(db_path);
    let inspected = inspecting.elapsed();
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let (from, to) = match state {
        DatabaseState::Upgradable { from, to } => (from, to),
        DatabaseState::TooNew { found, expected } => {
            // Never opened, never written: an older core does not know
            // what the newer columns mean.
            eprintln!(
                "base de version {found}, plus récente que celle de ce noyau ({expected}) : aucune écriture, mode maintenance jusqu'à l'arrêt"
            );
            let mut m = update::begin(MaintenanceReason::NewerSchema);
            (m.from, m.to) = (Some(found), Some(expected));
            maintenance.send_replace(Some(m));
            serve_maintenance_until_stopped(bind(socket_path)?, maintenance, &mut sigterm).await
        }
        // A new database, a current one, or one the opening refuses with
        // its own error (an early development database).
        _ => {
            let (store, steps) = Store::open_timed(db_path).with_context(|| format!("ouverture de {}", db_path.display()))?;
            log_opening(inspected, steps);
            return Ok((store, None));
        }
    };
    let listener = bind(socket_path)?;
    let mut m = update::begin(MaintenanceReason::Migration);
    (m.from, m.to, m.at) = (Some(from), Some(to), Some(from));
    maintenance.send_replace(Some(m));
    eprintln!("mise à jour de la base : version {from} vers {to}, peut prendre plusieurs minutes");
    let started = Instant::now();
    let path = db_path.to_path_buf();
    let progress = maintenance.clone();
    // Rust note: `spawn_blocking` runs a synchronous function on a thread
    // pool made for blocking work, and gives a future of its result; the
    // `Store` it returns can cross threads (`Send`).
    let mut opening = tokio::task::spawn_blocking(move || {
        nm_store::upgrade::migrate(&path, from, &|v| {
            eprintln!("mise à jour de la base : version {v} atteinte");
            progress.send_modify(|m| {
                if let Some(m) = m {
                    m.at = Some(v);
                }
            });
        })
    });
    let result = loop {
        tokio::select! {
            // Rust note: `&mut opening` polls the future without moving it,
            // so the loop can wait on it again after a client.
            r = &mut opening => break r.map_err(|e| anyhow::anyhow!("ouverture interrompue : {e}"))?,
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                tokio::spawn(update::serve_refusing(stream, maintenance.subscribe()));
            }
            _ = sigterm.recv() => stopped_while_opening(),
            _ = tokio::signal::ctrl_c() => stopped_while_opening(),
        }
    };
    match result {
        Ok((store, steps)) => {
            eprintln!("mise à jour de la base : version {from} vers {to} faite en {:.0} s", started.elapsed().as_secs_f64());
            log_opening(inspected, steps);
            maintenance.send_replace(None);
            Ok((store, Some(listener)))
        }
        Err(e) => {
            // The app shows the error and offers the log; it does not try
            // again by itself (`mise-a-jour.md`, section 4).
            eprintln!("mise à jour de la base : échec, {e} ; mode maintenance jusqu'à l'arrêt");
            let mut m = update::begin(MaintenanceReason::MigrationFailed);
            (m.from, m.to, m.message) = (Some(from), Some(to), Some(e.to_string()));
            maintenance.send_replace(Some(m));
            serve_maintenance_until_stopped(listener, maintenance, &mut sigterm).await
        }
    }
}

/// Each step of the opening, after the check of the file (`inspect`): the
/// largest part of a slow start, on a real mailbox.
fn log_opening(inspected: Duration, steps: nm_store::OpeningSteps) {
    let mut all = vec![("inspect", inspected)];
    all.extend(steps.steps);
    let total = all.iter().map(|(_, d)| *d).sum();
    eprintln!("{}", startup_line("store opened", total, &all));
}

/// A maintenance that only a stop ends (`newer_schema`,
/// `migration_failed`): every client gets its error, until SIGTERM or
/// Ctrl-C, then the core leaves with code 0. Not a failure: launchd would
/// start the core again every 10 seconds, for the same answer.
async fn serve_maintenance_until_stopped(
    listener: UnixListener,
    maintenance: &update::State,
    sigterm: &mut tokio::signal::unix::Signal,
) -> ! {
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                if let Ok((stream, _)) = accepted {
                    tokio::spawn(update::serve_refusing(stream, maintenance.subscribe()));
                }
            }
            _ = sigterm.recv() => break,
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    if let Some(path) = listener.local_addr().ok().and_then(|a| a.as_pathname().map(Path::to_path_buf)) {
        cleanup(&path);
    }
    std::process::exit(0)
}

/// A stop while the database opens: the opening thread is abandoned with
/// the process, and SQLite rolls its transaction back at the next opening.
/// Code 0, so that launchd does not start the core again at once.
fn stopped_while_opening() -> ! {
    eprintln!("arrêt demandé pendant la mise à jour de la base : annulée, elle reprendra au prochain démarrage");
    std::process::exit(0)
}

/// Writes why the core could not run to `core-error.json` of its data
/// directory (`nm_api::StartupError`), for the app's screen without a core.
/// Atomic (a temporary file, then `rename`), so the app never reads half a
/// file. Failing to write it is only logged: the reason is in `core.log`
/// anyway.
fn write_startup_error(data_dir: &Path, error: &anyhow::Error) {
    let report = startup_error(error);
    let path = data_dir.join(nm_api::paths::STARTUP_ERROR);
    let tmp = data_dir.join(format!(".{}.tmp", nm_api::paths::STARTUP_ERROR));
    let text = serde_json::to_string(&report).expect("plain struct");
    if let Err(e) = fs::write(&tmp, text).and_then(|()| fs::rename(&tmp, &path)) {
        eprintln!("{} : écriture impossible ({e})", path.display());
    }
}

/// The report of `error`: its kind, found in the chain of causes (`anyhow`
/// keeps them; the store's error is the innermost one), and its message.
fn startup_error(error: &anyhow::Error) -> nm_api::StartupError {
    use nm_store::StoreError;
    // Rust note: `chain()` walks the error and its causes; `downcast_ref`
    // asks whether one of them is a `StoreError`, and borrows it if so.
    let store = error.chain().find_map(|c| c.downcast_ref::<StoreError>());
    let (kind, missing, found, expected) = match store {
        Some(StoreError::OutdatedSchema(what)) => ("outdated", Some(what.to_string()), None, None),
        Some(StoreError::SchemaTooNew { found, expected }) => ("too_new", None, Some(*found), Some(*expected)),
        _ => ("other", None, None, None),
    };
    nm_api::StartupError {
        kind: kind.into(),
        // `{:#}` prints the whole chain on one line: "ouverture de …: base …".
        message: format!("{error:#}"),
        missing,
        found,
        expected,
        at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
    }
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

/// A wait for, or a hold of, the store's lock longer than this is written
/// to `core.log` (`Core::lock_store`).
const SLOW_LOCK: Duration = Duration::from_millis(250);

impl Core {
    /// The store's write lock, timed: a wait or a hold longer than
    /// [`SLOW_LOCK`] is written to `core.log` with who took it (`request`,
    /// `sync`, `draft`…). One writer at a time is the design (SQLite has a
    /// single writer anyway); these lines say who keeps the others waiting,
    /// and for how long.
    ///
    /// `unwrap` on `lock()`: it fails only if another thread panicked while
    /// holding the lock ("poisoned" mutex), a bug we do not try to recover
    /// from.
    pub(crate) fn lock_store(&self, who: &'static str) -> StoreLock<'_> {
        let asked = Instant::now();
        let guard = self.store.lock().unwrap();
        let waited = asked.elapsed();
        if waited >= SLOW_LOCK {
            eprintln!("{} store lock: {who} waited {} ms", local_time(), waited.as_millis());
        }
        if let Some(d) = self.test_write_delay {
            std::thread::sleep(d);
        }
        StoreLock { guard, who, since: Instant::now() }
    }
}

/// The store, locked for writing; logs on release if it was held long.
///
/// Rust note: `Deref` and `DerefMut` make `StoreLock` usable where a
/// `&Store` or a `&mut Store` is expected (`&mut lock` becomes
/// `&mut Store` by "deref coercion"), as the `MutexGuard` it wraps is.
/// `Drop` runs when the value goes out of scope: that is when the lock is
/// released, and when the hold is measured.
pub(crate) struct StoreLock<'a> {
    guard: std::sync::MutexGuard<'a, Store>,
    who: &'static str,
    since: Instant,
}

impl std::ops::Deref for StoreLock<'_> {
    type Target = Store;
    fn deref(&self) -> &Store {
        &self.guard
    }
}

impl std::ops::DerefMut for StoreLock<'_> {
    fn deref_mut(&mut self) -> &mut Store {
        &mut self.guard
    }
}

impl Drop for StoreLock<'_> {
    fn drop(&mut self) {
        let held = self.since.elapsed();
        if held >= SLOW_LOCK {
            eprintln!("{} store lock: {} held {} ms", local_time(), self.who, held.as_millis());
        }
    }
}

/// Requests of one connection running or waiting at once. Beyond, the core
/// stops reading the connection until one ends: a client that floods the
/// socket gets slower, it cannot make the core's memory grow.
const IN_FLIGHT: usize = 64;

/// Whether a method only reads, and so may run alongside the other
/// requests of its connection.
///
/// Why: the requests of a connection used to run one after the other. The
/// app has a single connection; during the import of an account, each
/// batch written sends events, the app reloads its lists, and a request
/// the user waits for (`r`) queued behind those reloads, for seconds. A
/// read now never waits for another request of its connection.
///
/// Everything else keeps the order in which the client sent it, among
/// themselves (`move_thread` then `undo`, the saves of a draft): those run
/// one after the other, in a task of the connection. A method missing from
/// this list is ordered: the safe side, at worst slower.
fn runs_in_parallel(method: &str) -> bool {
    matches!(
        method,
        method::LIST_FOLDERS
            | method::LIST_THREADS
            | method::GET_THREAD
            | method::LOCATE_MESSAGE
            | method::SEARCH
            | method::IMPORT_STATUS
            | method::LIST_HISTORY
            | method::LIST_HISTORY_GROUPS
            | method::LIST_TAGS
            | method::LIST_GROUPS
            | method::THREAD_LABELS
            | method::LIST_CONTACTS
            | method::LIST_ACCOUNTS
            | method::PREPARE_DRAFT
            | method::OPEN_DRAFT
            | method::PARSE_ADDRESSES
            | method::COMPOSE_FORMATS
            | method::HTML_TO_TEXT
            | method::SUGGEST_RECIPIENTS
            | method::LIST_SENDERS
            | method::SYNC_STATUS
            | method::CONFIG_STATUS
            | method::KEYMAP_GET
    )
}

/// Handles one client connection until it closes.
///
/// Three kinds of requests (`architecture.md`, "Protocole"):
/// - those of the connection itself (`ping`, `hello`, `subscribe`) are
///   answered here, in the order read: they change the session;
/// - reads ([`runs_in_parallel`]) each run in a task of their own;
/// - the others run one after the other, in the order read, in the task
///   `ordered`.
///
/// So responses may come back in another order than the requests: the
/// client matches them by `id`, as JSON-RPC allows. A client that needs a
/// read to see a write waits for the write's response before sending it.
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

    // Rust note: a `Semaphore` hands out a fixed number of permits;
    // `acquire_owned` waits while none is left. Each request holds one
    // until its response is queued (the permit is dropped with the task's
    // variables), which bounds the requests in flight.
    let in_flight = Arc::new(Semaphore::new(IN_FLIGHT));
    // The ordered requests, with the source of the session when read and
    // their permit.
    let (ordered_tx, mut ordered_rx) = mpsc::channel::<(Request, Source, OwnedSemaphorePermit)>(IN_FLIGHT);
    let ordered = {
        let (core, out) = (core.clone(), out.clone());
        tokio::spawn(async move {
            while let Some((req, source, _permit)) = ordered_rx.recv().await {
                if respond(&out, answer(&core, source, req).await).await.is_err() {
                    break;
                }
            }
        })
    };

    // The source defaults to CLI until `hello` says otherwise.
    let mut session = Session { hello: false, source: Source::Cli, out: out.clone(), subscribed: false };
    // `BufReader` buffers the reads; `lines()` splits the stream on newlines.
    let mut lines = BufReader::new(read).lines();
    // A maintenance that begins closes the connection (`update.rs`).
    let mut maintenance = core.maintenance.subscribe();
    let mut closed_for = None;
    loop {
        // `next_line` returns `Ok(None)` when the client closes the
        // connection.
        let line = tokio::select! {
            line = lines.next_line() => match line? {
                Some(line) => line,
                None => break,
            },
            // Rust note: `wait_for` returns at once if the value already
            // matches, else when it changes to one that does. The `if`
            // after it is a precondition: the branch is left out of the
            // `select!` for the app, whose connection stays open during an
            // update (`update.rs`): it must be able to cancel it. And
            // before `hello`, which says who the client is: a `hello`
            // refused by the maintenance closes the connection below.
            m = maintenance.wait_for(Option::is_some), if session.hello && session.source != Source::App => {
                closed_for = m.ok().and_then(|m| m.as_ref().map(nm_api::Maintenance::error));
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let req = match serde_json::from_str::<Request>(&line) {
            Ok(req) => req,
            // Invalid JSON: the id is unknown, so the JSON-RPC spec says to
            // answer with `id: null`.
            Err(e) => {
                let r = Response::err(Value::Null, RpcError::new(error_code::PARSE_ERROR, reason::PARSE_ERROR.with("detail", e.to_string())));
                if respond(&out, Some(r)).await.is_err() {
                    break;
                }
                continue;
            }
        };
        if let Some(result) = session_call(&core, &mut session, &req) {
            // A client let in by no maintenance: told so, then closed.
            let refused = req.method == method::HELLO && matches!(&result, Err(e) if e.code == error_code::MAINTENANCE);
            if respond(&out, req.id.map(|id| reply(id, result))).await.is_err() || refused {
                break;
            }
            continue;
        }
        let timer = Instant::now();
        let permit = in_flight.clone().acquire_owned().await.expect("the semaphore is never closed");
        // Waiting for a permit means the client has IN_FLIGHT requests
        // pending: worth knowing when a response is late.
        let waited = timer.elapsed();
        if waited >= SLOW_WAIT {
            eprintln!("{} connection: {} ms before a request could start ({IN_FLIGHT} in flight)", local_time(), waited.as_millis());
        }
        let source = session.source;
        if runs_in_parallel(&req.method) {
            let (core, out) = (core.clone(), out.clone());
            tokio::spawn(async move {
                let _permit = permit;
                let _ = respond(&out, answer(&core, source, req).await).await;
            });
        } else if ordered_tx.send((req, source, permit)).await.is_err() {
            break;
        }
    }
    // The ordered requests already read are answered, then the queue is
    // closed: drop every sender (the session holds a clone) so that the
    // writer task sees its end, and wait for it to flush. The reads still
    // running hold a sender each: the writer waits for them too.
    drop(ordered_tx);
    let _ = ordered.await;
    // Closed by a maintenance: the client is told why, after the answers
    // of what it asked before (`prepare_update` itself, for the app). No
    // id: it answers no request in particular (JSON-RPC, "id: null").
    if let Some(error) = closed_for {
        let _ = respond(&out, Some(Response::err(Value::Null, error))).await;
    }
    drop(session);
    drop(out);
    let _ = writer.await;
    Ok(())
}

/// Queues a response for the writer task; an error means the connection
/// closed. `None` (a notification) sends nothing.
async fn respond(out: &mpsc::Sender<Arc<String>>, response: Option<Response>) -> anyhow::Result<()> {
    if let Some(r) = response {
        out.send(Arc::new(serde_json::to_string(&r)?)).await?;
    }
    Ok(())
}

fn reply(id: Value, result: Result<Value, RpcError>) -> Response {
    match result {
        Ok(v) => Response::ok(id, v),
        Err(e) => Response::err(id, e),
    }
}

/// A request that is slower than this is written to `core.log`, with its
/// method and its duration: the numbers to look at when the app feels
/// slow, rather than guesses.
const SLOW_REQUEST: Duration = Duration::from_millis(500);
/// The same for a wait (a permit, the store's lock).
const SLOW_WAIT: Duration = Duration::from_millis(100);

tokio::task_local! {
    /// What the request running in this task waited for the database: a
    /// connection of the read pool, the store's write lock, and a thread of
    /// the blocking pool to run on. `with_reader` and `with_store` add to it.
    ///
    /// Rust note: a `task_local!` is a variable of the current tokio task,
    /// as a thread-local is of a thread; `scope` gives it a value for the
    /// duration of a future. A `Cell` lets that value change through a
    /// shared reference (`get`, `set`), for a type that is `Copy`.
    static DB_WAIT: std::cell::Cell<Duration>;
}

/// Adds `d` to the database wait of the current request, if any (a call
/// outside `answer`, at startup, has none).
fn add_db_wait(d: Duration) {
    let _ = DB_WAIT.try_with(|w| w.set(w.get() + d));
}

/// Runs a request that is not one of the session's, and builds its
/// response (`None` for a notification, which has no `id`).
async fn answer(core: &Arc<Core>, source: Source, req: Request) -> Option<Response> {
    let id = req.id.clone();
    let method = req.method.clone();
    let timer = Instant::now();
    let (result, waited) = DB_WAIT
        .scope(std::cell::Cell::new(Duration::ZERO), async {
            let result = call(core, source, req).await;
            (result, DB_WAIT.with(|w| w.get()))
        })
        .await;
    let took = timer.elapsed();
    if took >= SLOW_REQUEST {
        eprintln!("{}", slow_request_line(&local_time(), &method, took, waited));
    }
    id.map(|id| reply(id, result))
}

/// The line of a slow request in `core.log`: when, what, and how its time
/// splits between waiting for the database and running. A request that
/// mostly waited was stuck behind others (every connection of the read
/// pool lent, the write lock held): its own query is not the one to
/// speed up. A pure function, tested alone.
fn slow_request_line(at: &str, method: &str, took: Duration, waited: Duration) -> String {
    let waited = waited.min(took);
    format!(
        "{at} slow request: {method} took {} ms ({} ms waiting for the database, {} ms running)",
        took.as_millis(),
        waited.as_millis(),
        (took - waited).as_millis()
    )
}

/// The local time of the Mac, `HH:MM:SS`, to date a line of `core.log`.
///
/// Rust note: `unsafe` because `localtime_r` is a C function the compiler
/// cannot check; `localtime_r`, unlike `localtime`, is safe to call from
/// several threads.
fn local_time() -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let time = now as libc::time_t;
    // SAFETY: `tm` is a plain C struct, valid when zeroed; `localtime_r`
    // only reads `time` and writes `tm`.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&time, &mut tm).is_null() } {
        return "--:--:--".into();
    }
    format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec)
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
    serde_json::from_value(v).map_err(|e| RpcError::invalid(e))
}

/// Maps a store error to a JSON-RPC error: a stable code, and a reason
/// (`nm_api::reason`) that a client writes in the user's language.
pub(crate) fn store_error(e: StoreError) -> RpcError {
    match e {
        StoreError::NotFound(what) => RpcError::not_found(what),
        StoreError::InvalidParams(detail) => RpcError::invalid(detail),
        StoreError::Invalid(why) => RpcError::new(error_code::INVALID_PARAMS, why),
        StoreError::InvalidMove(why) => RpcError::new(error_code::INVALID_MOVE, why),
        StoreError::CannotSend(why) => RpcError::new(error_code::CANNOT_SEND, why),
        StoreError::ReadOnly => RpcError::new(error_code::READ_ONLY, reason::ACCOUNT_READ_ONLY),
        StoreError::RawDamaged(hash) => RpcError::new(error_code::RAW_DAMAGED, reason::RAW_DAMAGED.with("hash", hash)),
        StoreError::BodyPending => RpcError::new(error_code::BODY_PENDING, reason::BODY_PENDING),
        StoreError::DraftSending => RpcError::new(error_code::DRAFT_SENDING, reason::DRAFT_SENDING),
        StoreError::SendStarted => RpcError::new(error_code::SEND_ALREADY_STARTED, reason::SEND_STARTED),
        StoreError::RevisionConflict { current } => {
            RpcError::new(error_code::REVISION_CONFLICT, reason::REVISION_CONFLICT.with("revision", current))
        }
        // The client needs the address and the verdict to ask the user.
        StoreError::ConfirmationRequired { addr, verdict } => {
            RpcError::new(error_code::CONFIRMATION_REQUIRED, reason::CONFIRMATION_REQUIRED.with("addr", addr).with("verdict", verdict))
        }
        other => RpcError::internal(other),
    }
}

/// Serializes a result into a JSON `Value`.
fn to_value<T: serde::Serialize>(v: T) -> Result<Value, RpcError> {
    serde_json::to_value(v).map_err(|e| RpcError::internal(e))
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
    let asked = Instant::now();
    let (result, waited) = tokio::task::spawn_blocking(move || {
        let mut store = core.lock_store("request");
        // From the call to the lock: a blocking thread, then the lock.
        let waited = asked.elapsed();
        (f(&mut store), waited)
    })
    .await
    // Two layers of errors: the first `map_err` handles a failure of the
    // blocking task itself (a panic), the second converts a store error.
    .map_err(|e| RpcError::internal(e))?;
    add_db_wait(waited);
    result.map_err(store_error)
}

/// Runs the query `f` on a connection of the read pool, on a blocking
/// thread. Unlike `with_store`, it never waits for a write in progress.
pub(crate) async fn with_reader<T: Send + 'static>(
    core: &Arc<Core>,
    f: impl FnOnce(&Store) -> Result<T, StoreError> + Send + 'static,
) -> Result<T, RpcError> {
    let core = core.clone();
    let asked = Instant::now();
    let (result, waited) = tokio::task::spawn_blocking(move || {
        // A blocking thread, then a free connection of the pool.
        let queued = asked.elapsed();
        let (result, waited) = core.readers.read_timed(f);
        (result, queued + waited)
    })
    .await
    .map_err(|e| RpcError::internal(e))?;
    add_db_wait(waited);
    result.map_err(store_error)
}

/// `set_password`: saves the IMAP password of an account in the Keychain,
/// bound to the server the file names now. Saving it again after the file
/// changed server is how the user confirms the new server.
async fn set_password(core: &Arc<Core>, p: SetPasswordParams, source: Source) -> Result<CommandResult, RpcError> {
    // `configuration.md`, section 6: the secret is entered in the app or
    // the CLI only. An agent (MCP) or the web server must not be able to
    // bind a password of its choosing to a server of its choosing. The
    // source is declared by the client until its signature is checked.
    if !matches!(source, Source::App | Source::Cli | Source::Tui) {
        return Err(RpcError::new(error_code::FORBIDDEN, reason::RESERVED_TO_PEOPLE));
    }
    if p.password.is_empty() {
        return Err(RpcError::new(error_code::INVALID_PARAMS, reason::PASSWORD_EMPTY));
    }
    let (secrets, binding, account_id) = {
        let engine = core.engine.lock().unwrap();
        let Some(account) = engine.config().accounts.get(&p.account) else {
            return Err(RpcError::new(error_code::NOT_FOUND, reason::ACCOUNT_UNKNOWN.with("account", p.account.as_str())));
        };
        let Some(id) = engine.account_id(&p.account) else {
            return Err(RpcError::new(error_code::NOT_FOUND, reason::ACCOUNT_NOT_IN_DATABASE.with("account", p.account.as_str())));
        };
        // The SMTP password saved apart is bound to the SMTP server
        // (`reponse.md`, section 7).
        let binding = if p.smtp {
            nm_engine::secrets::smtp_binding(account).ok_or_else(|| {
                RpcError::new(error_code::INVALID_PARAMS, reason::NO_SMTP_SERVER.with("account", p.account.as_str()))
            })?
        } else {
            nm_engine::secrets::Binding::of(account)
        };
        (engine.secrets(), binding, id)
    };
    // The Keychain call runs on a blocking thread, without any lock held.
    let credential = nm_engine::secrets::Credential {
        binding: binding.clone(),
        secret: nm_engine::config::SecretString::new(p.password),
    };
    let key = p.account.clone();
    let smtp = p.smtp;
    tokio::task::spawn_blocking(move || if smtp { secrets.set_smtp(&key, &credential) } else { secrets.set_imap(&key, &credential) })
        .await
        .map_err(|e| RpcError::internal(e))?
        .map_err(|e| RpcError::new(error_code::SECRETS, reason::KEYCHAIN.with("system_text", e)))?;
    let core2 = core.clone();
    with_store(core, move |s| {
        s.record_password_saved(account_id, &binding.host, &binding.username, source)?;
        // Lock order: store, engine, events.
        let events = core2.engine.lock().unwrap().recheck_accounts();
        let mut log = core2.events.lock().unwrap();
        for e in events {
            log.push(e);
        }
        drop(log);
        // A worker stopped by a refused login or a missing password tries
        // again with the new password.
        if let Some(syncer) = core2.syncer.get() {
            syncer.kick();
        }
        Ok(())
    })
    .await?;
    Ok(CommandResult { changed: 0 })
}

/// `probe_account`: connects to the account's IMAP server and reports what
/// it offers (`nm-engine`, `probe.rs`). Changes nothing, so any client may
/// call it. No lock is held while the Keychain and the server answer.
async fn probe_account(core: &Arc<Core>, p: ProbeAccountParams) -> Result<ProbeReport, RpcError> {
    let (secrets, account) = {
        let engine = core.engine.lock().unwrap();
        let Some(account) = engine.config().accounts.get(&p.account) else {
            return Err(RpcError::new(error_code::NOT_FOUND, reason::ACCOUNT_UNKNOWN.with("account", p.account.as_str())));
        };
        (engine.secrets(), account.clone())
    };
    // The password is read on a blocking thread (the Keychain may take a
    // moment), after the check that it was saved for this very server.
    let key = p.account.clone();
    let acc = account.clone();
    let password = tokio::task::spawn_blocking(move || nm_engine::secrets::imap_password(secrets.as_ref(), &key, &acc))
        .await
        .map_err(|e| RpcError::internal(e))?
        .map_err(|refusal| {
            let code = match refusal {
                nm_engine::secrets::Refusal::Unavailable(_) => error_code::SECRETS,
                _ => error_code::ACCOUNT_NOT_READY,
            };
            RpcError::new(code, refusal.why(&p.account, &account))
        })?;
    let spec = nm_engine::probe::imap_spec(&account);
    nm_engine::probe::probe(&spec, &account.imap.username, password.expose(), nm_proto::imap::Timeouts::default())
        .await
        .map_err(|e| RpcError::new(error_code::SERVER, reason::IMAP_FAILED.with("server_text", e.to_string())))
}

/// Reads `config.toml` and `accounts.toml` again after a change seen by the
/// watch (or written by an account command), and emits
/// what it changed. Runs on the thread of the watch: it may block on the
/// store while a command or an import batch finishes.
fn reload_config(core: &Arc<Core>) {
    // Lock order: store, engine, events. The events are emitted with the
    // store's lock held, like those of the commands, so their order is the
    // order of the writes.
    let mut store = core.lock_store("config");
    let mut engine = core.engine.lock().unwrap();
    let changed = match engine.reload(&mut store, false) {
        Ok(events) => {
            let changed = !events.is_empty();
            if changed {
                log_config(&engine);
            }
            let mut log = core.events.lock().unwrap();
            for e in events {
                log.push(e);
            }
            changed
        }
        Err(e) => {
            eprintln!("config : {e}");
            false
        }
    };
    drop(engine);
    // The workers follow the accounts of the file (`sync.rs`); a file
    // changed may also fix what stopped one (a user name). Not while an
    // update is prepared, which stopped them: they start again when it is
    // cancelled (`update.rs`, `leave`), with the file as it is then.
    if changed && core.maintenance.borrow().is_none() {
        sync::apply(core, &mut store);
        if let Some(syncer) = core.syncer.get() {
            syncer.kick();
        }
    }
}

/// Reads `keymap.toml` again after a change seen by the watch, and emits
/// `KeymapChanged` or `ConfigError`. It needs neither the store nor the
/// engine: a keymap only matters to the app.
fn reload_keymap(core: &Arc<Core>) {
    let mut keymap = core.keymap.lock().unwrap();
    let events = keymap.reload(false);
    if !events.is_empty() {
        log_keymap(&keymap);
    }
    let mut log = core.events.lock().unwrap();
    for e in events {
        log.push(e);
    }
}

/// Prints the state of `keymap.toml` on the core's standard error.
fn log_keymap(keymap: &nm_engine::keymap::KeymapFile) {
    match keymap.error() {
        Some(p) => eprintln!("keymap : refusée, touches précédentes conservées : {p}"),
        None => eprintln!("keymap : {}", keymap.path().display()),
    }
    for w in keymap.warnings() {
        eprintln!("keymap : avertissement : {w}");
    }
}

/// Prints the state of the settings on the core's standard error.
fn log_config(engine: &Engine) {
    let (path, accounts) = (engine.path().display(), engine.accounts_path());
    match engine.error() {
        Some(p) => eprintln!("config : refusée, réglages précédents conservés : {p}"),
        None => eprintln!("config : {path}, {} : {} compte(s)", accounts.display(), engine.config().accounts.len()),
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
        // Changes to send to a server: its worker leaves IDLE for them.
        sync::wake(&core2, &changes);
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
        sync::wake(&core2, &out.changes);
        Ok(out.view())
    })
    .await?;
    to_value(result)
}

/// The threads of `thread_id` / `thread_ids` (`nm_api::thread_list`), or
/// `INVALID_PARAMS` when both or neither are given.
fn threads_param(one: Option<i64>, several: Option<Vec<i64>>) -> Result<Vec<i64>, RpcError> {
    nm_api::thread_list(one, several)
        .ok_or_else(|| RpcError::invalid("one of thread_id, thread_ids expected"))
}

/// Page size requested by the client, or the default, clamped to the
/// allowed range.
fn limit(l: Option<u32>) -> u32 {
    l.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
}

/// The requests of the connection itself, answered at once, in the order
/// read: `ping`, `hello` (which sets the session's source), `subscribe`,
/// and the refusal of everything before `hello`. `None`: another method,
/// for [`call`].
fn session_call(core: &Arc<Core>, session: &mut Session, req: &Request) -> Option<Result<Value, RpcError>> {
    if req.jsonrpc != "2.0" {
        return Some(Err(RpcError::new(error_code::INVALID_REQUEST, reason::INVALID_REQUEST.with("detail", "jsonrpc must be \"2.0\""))));
    }
    // First match: methods available before the handshake (`ping`, `hello`),
    // and refusal of everything else until `hello` succeeds.
    Some(match req.method.as_str() {
        method::PING => Ok(Value::String("pong".into())),
        method::HELLO => (|| {
            let p: HelloParams = params(req.params.clone())?;
            if p.protocol_version != PROTOCOL_VERSION {
                return Err(RpcError::new(
                    error_code::PROTOCOL_VERSION,
                    reason::PROTOCOL_VERSION.with("version", p.protocol_version).with("expected", PROTOCOL_VERSION),
                ));
            }
            // Provisional: the identity will come from the client's code
            // signature, never from what the client declares.
            let source = match p.client.as_deref() {
                Some("app") => Source::App,
                Some("tui") => Source::Tui,
                Some("mcp") => Source::Mcp,
                Some("web") => Source::Web,
                _ => Source::Cli,
            };
            // In maintenance, only the app is let in, to prepare or cancel
            // the update (`update.rs`); every other request of it is
            // refused by `call`. The others get the maintenance's error,
            // and the connection closes.
            if source != Source::App
                && let Some(e) = update::refusal(&core.maintenance)
            {
                return Err(e);
            }
            session.source = source;
            session.hello = true;
            to_value(HelloResult {
                protocol_version: PROTOCOL_VERSION,
                // `env!` reads a variable at compile time; Cargo sets
                // CARGO_PKG_VERSION to the crate's version.
                core_version: env!("CARGO_PKG_VERSION").into(),
            })
        })(),
        _ if !session.hello => Err(RpcError::new(error_code::PROTOCOL_VERSION, reason::HELLO_EXPECTED)),
        method::SUBSCRIBE => (|| {
            // Nothing to follow during a maintenance: the app learns of it
            // here, after its `hello`.
            if let Some(e) = update::refusal(&core.maintenance) {
                return Err(e);
            }
            let p: SubscribeParams = params(req.params.clone())?;
            if session.subscribed {
                return Err(RpcError::new(error_code::INVALID_REQUEST, reason::INVALID_REQUEST.with("detail", "already subscribed")));
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
        })(),
        _ => return None,
    })
}

/// Routes a request to the code of its method. Runs in a task of its own
/// or in the ordered task of the connection (see [`serve`]): it gets the
/// session's source, never the session.
async fn call(core: &Arc<Core>, source: Source, req: Request) -> Result<Value, RpcError> {
    // What this source may ask, before anything runs (`rights.rs`).
    rights::check(source, &req.method)?;
    // In maintenance, nothing runs (`update.rs`): a request read just
    // before it began, and still queued, is refused like a new client.
    // Only the update's own methods go on: `prepare_update` again, and
    // `cancel_update` to leave it.
    if !matches!(req.method.as_str(), method::PREPARE_UPDATE | method::CANCEL_UPDATE)
        && let Some(e) = update::refusal(&core.maintenance)
    {
        return Err(e);
    }
    // Event position, read *before* running a query and returned as `as_of`.
    // Any event with a higher seq may not be reflected in the data, so the
    // client knows it must apply it.
    let as_of = || core.events.lock().unwrap().position();
    match req.method.as_str() {
        method::PREPARE_UPDATE => to_value(update::prepare(core).await?),
        method::CANCEL_UPDATE => to_value(update::cancel(core).await?),
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
                AllMail(Option<i64>),
                Work(Option<i64>),
            }
            let scope = match (p.folder_id, p.folder_ids, p.everything) {
                (Some(f), None, None) => Scope::Folders(vec![f]),
                (None, Some(fs), None) if !fs.is_empty() => Scope::Folders(fs),
                (None, None, Some(e)) if e.all_mail && e.work => {
                    return Err(RpcError::invalid("all_mail or work, not both"));
                }
                (None, None, Some(e)) if e.all_mail => Scope::AllMail(e.account_id),
                (None, None, Some(e)) if e.work => Scope::Work(e.account_id),
                (None, None, Some(e)) => Scope::Everything(e.account_id),
                _ => {
                    return Err(RpcError::invalid("one of folder_id, folder_ids, everything expected"));
                }
            };
            // The addresses of the mailbox, as for `get_thread`: a row names
            // the person a conversation is with, not the user who answered
            // last (`ThreadSummary::correspondent`).
            let own = drafts::declared_addresses(core);
            let (items, next_cursor) = with_reader(core, move |s| match scope {
                Scope::Folders(folders) => {
                    s.list_threads_in(&folders, p.cursor.as_deref(), limit(p.limit), p.sort, &p.filter, &own)
                }
                Scope::Everything(account) => {
                    s.list_everything(account, p.cursor.as_deref(), limit(p.limit), p.sort, &p.filter, &own)
                }
                Scope::AllMail(account) => {
                    s.list_all_mail(account, p.cursor.as_deref(), limit(p.limit), p.sort, &p.filter, &own)
                }
                Scope::Work(account) => {
                    s.list_work(account, p.cursor.as_deref(), limit(p.limit), p.sort, &p.filter, &own)
                }
            })
            .await?;
            to_value(Page { items, next_cursor, as_of })
        }
        method::GET_THREAD => {
            let p: GetThreadParams = params(req.params)?;
            let as_of = as_of();
            let thread_id = p.thread_id;
            // The addresses of the mailbox, which only `accounts.toml`
            // holds: the store shows the recipients of the mails they
            // sent (`present_thread`, `docs/thread-display.md`).
            let own = drafts::declared_addresses(core);
            let ((subject, messages), labels, missing) = with_reader(core, move |s| {
                Ok((s.get_thread(thread_id, &own)?, s.thread_labels(thread_id, &own)?, s.thread_bodies_missing(thread_id)?))
            })
            .await?;
            // Bodies not downloaded yet: the user is waiting for them.
            sync::prioritize(core, missing);
            to_value(ThreadView {
                id: p.thread_id,
                subject,
                messages,
                tags: labels.tags,
                correspondent: labels.correspondent,
                correspondent_groups: labels.groups,
                as_of,
            })
        }
        method::LOCATE_MESSAGE => {
            let p: LocateMessageParams = params(req.params)?;
            to_value(with_reader(core, move |s| s.locate_message(p.message_id)).await?)
        }
        method::SEARCH => {
            let p: SearchParams = params(req.params)?;
            let as_of = as_of();
            let items = with_reader(core, move |s| s.search(&p.query, limit(p.limit))).await?;
            to_value(SearchResult { items, as_of })
        }
        method::MOVE_THREADS => {
            let p: nm_api::MoveThreadsParams = params(req.params)?;
            if p.threads.is_empty() {
                return Err(RpcError::invalid("threads is empty"));
            }
            let opts = MoveOptions { confirm_suspect: p.confirm_suspect, auth_policy: p.auth_policy };
            let threads: Vec<(i64, Option<i64>)> = p.threads.iter().map(|t| (t.thread_id, t.from_folder_id)).collect();
            let core2 = core.clone();
            let result = with_store(core, move |store| {
                let batch = store.move_threads(&threads, p.target, opts, source)?;
                // The AI's suggestions for the conversations moved are
                // decided by the move (`ia.md`, section 9): accepted when
                // it went to the target suggested, other otherwise.
                let moved: Vec<i64> = batch
                    .outcome
                    .changes
                    .threads
                    .iter()
                    .map(|t| t.0)
                    .filter(|t| threads.iter().any(|r| r.0 == *t))
                    .collect();
                store.ai_suggestions_resolve(&moved, p.target.as_str())?;
                // One batch of events for the whole move, emitted under the
                // store lock like any command (`command`).
                core2.events.lock().unwrap().emit(&batch.outcome.changes);
                sync::wake(&core2, &batch.outcome.changes);
                Ok(batch.view())
            })
            .await?;
            to_value(result)
        }
        method::MOVE_THREAD => {
            let p: MoveThreadParams = params(req.params)?;
            let opts = MoveOptions { confirm_suspect: p.confirm_suspect, auth_policy: p.auth_policy };
            workflow(core, move |s| {
                let out = s.move_thread(p.thread_id, p.target, p.from_folder_id, opts, source)?;
                if out.moved > 0 {
                    s.ai_suggestions_resolve(&[p.thread_id], p.target.as_str())?;
                }
                Ok(out)
            })
            .await
        }
        method::SET_ADDRESS_SCREENING => {
            let p: SetAddressScreeningParams = params(req.params)?;
            workflow(core, move |s| s.set_address_screening(&p.addr, p.target, source)).await
        }
        method::SET_DOMAIN_SCREENING => {
            let p: SetDomainScreeningParams = params(req.params)?;
            workflow(core, move |s| s.set_domain_screening(&p.domain, p.target, source)).await
        }
        method::SET_SENDER_KIND => {
            let p: SetSenderKindParams = params(req.params)?;
            workflow(core, move |s| s.set_sender_kind(p.address_id, p.kind, source)).await
        }
        method::UNDO => {
            let p: UndoParams = params(req.params)?;
            workflow(core, move |s| s.undo(&p.token, source)).await
        }
        method::SET_THREAD_SEEN => {
            let p: SetThreadSeenParams = params(req.params)?;
            match (p.thread_id, p.thread_ids) {
                // One thread: an unknown one is an error, as it always was.
                (Some(id), None) => command(core, move |s| s.set_thread_seen(id, p.seen, source)).await,
                (one, several) => {
                    let ids = threads_param(one, several)?;
                    command(core, move |s| s.set_threads_seen(&ids, p.seen, source)).await
                }
            }
        }
        method::SET_THREAD_FLAGGED => {
            let p: SetThreadFlaggedParams = params(req.params)?;
            match (p.thread_id, p.thread_ids) {
                (Some(id), None) => command(core, move |s| s.set_thread_flagged(id, p.flagged, source)).await,
                (one, several) => {
                    let ids = threads_param(one, several)?;
                    command(core, move |s| s.set_threads_flagged(&ids, p.flagged, source)).await
                }
            }
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
                return Err(RpcError::new(error_code::INVALID_PARAMS, reason::PATH_NOT_ABSOLUTE.with("path", dir.display().to_string())));
            }
            let raw = core.raw.clone();
            let path = sync::repaired(core, with_reader(core, move |s| s.export_message(&raw, p.message_id, p.format, &dir)).await).await?;
            to_value(ExportResult { path: path.display().to_string() })
        }
        method::MESSAGE_TEXT => {
            // `ui.md`, menu `s`: the client puts the text on the clipboard.
            let p: MessageTextParams = params(req.params)?;
            let text = with_reader(core, move |s| s.message_text(p.message_id)).await?;
            to_value(MessageTextResult { text })
        }
        method::MESSAGE_RECORD => {
            // `ui.md`, menu `s`, "p as pdf": the client lays the record out.
            // `repaired`: a damaged raw file is set aside and downloaded
            // again, as for an export.
            let p: MessageRecordParams = params(req.params)?;
            let raw = core.raw.clone();
            let record = sync::repaired(core, with_reader(core, move |s| s.message_record(&raw, p.message_id)).await).await?;
            to_value(record)
        }
        method::UNSUBSCRIBE => {
            // `ui.md`, "Désabonnement". With one click (RFC 8058), the core
            // sends the request itself and records it; otherwise the client
            // opens the page or the address. A failed one-click attempt
            // falls back to opening the page.
            let p: UnsubscribeParams = params(req.params)?;
            let id = p.message_id;
            // Read before the query: the lock of the engine is not held
            // while the database is read.
            let in_banned = core.engine.lock().unwrap().config().unsubscribe.in_banned;
            let target = with_reader(core, move |s| s.unsubscribe_target(id, in_banned)).await?;
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
                .map_err(|e| RpcError::internal(e))?;
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
            let f = sync::repaired(core, with_store(core, move |s| s.get_attachment(&raw, &files, p.attachment_id)).await).await?;
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
            let filter = nm_store::library::HistoryFilter { since: p.since, sources: p.sources };
            let (items, next_cursor) =
                with_reader(core, move |s| s.list_history_filtered(p.cursor.as_deref(), limit(p.limit), &filter)).await?;
            to_value(Page { items, next_cursor, as_of })
        }
        method::LIST_HISTORY_GROUPS => {
            let p: ListHistoryGroupsParams = params(req.params)?;
            let as_of = as_of();
            let query = nm_store::library::HistoryGroupQuery {
                include_read: p.include_read,
                range: p.first_id.zip(p.last_id),
                sources: p.sources,
            };
            let (items, next_cursor) =
                with_reader(core, move |s| s.list_history_groups(p.cursor.as_deref(), limit(p.limit), &query)).await?;
            to_value(Page { items, next_cursor, as_of })
        }
        method::ATTACHMENT_TEXT => to_value(extract::attachment_text(core, params(req.params)?).await?),
        method::LIST_TAGS => {
            let p: ListTagsParams = params(req.params)?;
            let as_of = as_of();
            let items = with_reader(core, move |s| s.list_tags(p.account_id)).await?;
            to_value(Items { items, as_of })
        }
        method::TAG_THREAD => {
            let p: TagThreadParams = params(req.params)?;
            match (p.thread_id, p.thread_ids) {
                (Some(id), None) => command(core, move |s| s.tag_thread(id, &p.name, p.remove, source)).await,
                (one, several) => {
                    let ids = threads_param(one, several)?;
                    command(core, move |s| s.tag_threads(&ids, &p.name, p.remove, source)).await
                }
            }
        }
        method::LIST_GROUPS => {
            let as_of = as_of();
            let items = with_reader(core, |s| s.list_groups()).await?;
            to_value(Items { items, as_of })
        }
        method::SET_GROUP_MEMBER => {
            let p: SetGroupMemberParams = params(req.params)?;
            // The addresses of `accounts.toml`: none can join a group
            // (`groupes.md`, "Adresses de l'utilisateur").
            let own = drafts::declared_addresses(core);
            let (group_id, addr, changed) = with_store(core, move |s| {
                s.set_group_member(&p.group, p.addr.as_deref(), p.thread_id, p.remove, &own, source)
            })
            .await?;
            to_value(GroupMemberResult { group_id, addr, changed })
        }
        method::RENAME_TAG => {
            let p: RenameParams = params(req.params)?;
            let n = with_store(core, move |s| s.rename_tag(&p.name, &p.new_name, source)).await?;
            to_value(RenameResult { changed: n > 0 })
        }
        // A command: the removals reach the server through the spool, and
        // the threads changed are announced, as for `tag_thread`.
        method::DELETE_TAG => {
            let p: DeleteNameParams = params(req.params)?;
            command(core, move |s| s.delete_tag(&p.name, source)).await
        }
        method::RENAME_GROUP => {
            let p: RenameParams = params(req.params)?;
            let changed = with_store(core, move |s| s.rename_group(&p.name, &p.new_name, source)).await?;
            to_value(RenameResult { changed })
        }
        method::DELETE_GROUP => {
            let p: DeleteNameParams = params(req.params)?;
            let (group_id, members) = with_store(core, move |s| s.delete_group(&p.name, source)).await?;
            to_value(DeleteGroupResult { group_id, members })
        }
        method::THREAD_LABELS => {
            let p: ThreadLabelsParams = params(req.params)?;
            if p.thread_ids.len() > nm_api::MAX_LABEL_THREADS {
                return Err(RpcError::invalid(format!("thread_ids: {} at most", nm_api::MAX_LABEL_THREADS)));
            }
            // As for `set_group_member`: the person of each thread is never
            // one of the user's addresses.
            let own = drafts::declared_addresses(core);
            let as_of = as_of();
            let items = with_reader(core, move |s| {
                p.thread_ids.iter().map(|&t| s.thread_labels(t, &own)).collect::<Result<Vec<_>, _>>()
            })
            .await?;
            to_value(Items { items, as_of })
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
        method::SET_PASSWORD => {
            let p: SetPasswordParams = params(req.params)?;
            to_value(set_password(core, p, source).await?)
        }
        method::TEST_ACCOUNT => {
            let p: nm_api::TestAccountParams = params(req.params)?;
            to_value(setup::test_account(core, p, source).await?)
        }
        method::DISCOVER_ACCOUNT => {
            let p: nm_api::DiscoverAccountParams = params(req.params)?;
            to_value(setup::discover_account(core, p, source).await?)
        }
        method::TEST_SMTP => {
            let p: nm_api::TestSmtpParams = params(req.params)?;
            to_value(setup::test_smtp(p, source).await?)
        }
        method::COUNT_ARCHIVE => {
            let p: nm_api::CountArchiveParams = params(req.params)?;
            to_value(setup::count_archive(p, source).await?)
        }
        method::ADD_ACCOUNT => {
            let p: nm_api::AddAccountParams = params(req.params)?;
            to_value(setup::add_account(core, p, source).await?)
        }
        method::SET_FOLDER_ROLES => {
            let p: nm_api::SetFolderRolesParams = params(req.params)?;
            to_value(setup::set_folder_roles(core, p, source).await?)
        }
        method::PREPARE_ACTIVATION => {
            let p: nm_api::PrepareActivationParams = params(req.params)?;
            to_value(setup::prepare_activation(core, p, source).await?)
        }
        method::ACTIVATE_ACCOUNT => {
            let p: nm_api::ActivateAccountParams = params(req.params)?;
            to_value(setup::activate_account(core, p, source).await?)
        }
        method::RENAME_ACCOUNT => {
            let p: nm_api::RenameAccountParams = params(req.params)?;
            setup::rename_account(core, p, source).await?;
            to_value(CommandResult { changed: 0 })
        }
        method::REMOVE_ACCOUNT => {
            let p: nm_api::RemoveAccountParams = params(req.params)?;
            setup::remove_account(core, p, source).await?;
            to_value(CommandResult { changed: 0 })
        }
        method::PREPARE_DRAFT => to_value(drafts::prepare(core, params(req.params)?, source).await?),
        method::CREATE_DRAFT => to_value(drafts::create(core, params(req.params)?, source).await?),
        method::OPEN_DRAFT => to_value(drafts::open(core, params(req.params)?).await?),
        method::UPDATE_DRAFT => to_value(drafts::update(core, params(req.params)?, source).await?),
        method::ADD_DRAFT_ATTACHMENT => to_value(drafts::add_attachment(core, params(req.params)?, source).await?),
        method::REMOVE_DRAFT_ATTACHMENT => to_value(drafts::remove_attachment(core, params(req.params)?, source).await?),
        method::PARSE_ADDRESSES => to_value(drafts::parse_addresses(params(req.params)?)),
        method::COMPOSE_FORMATS => to_value(drafts::compose_formats(core)),
        method::HTML_TO_TEXT => to_value(drafts::html_to_text(params(req.params)?)?),
        method::SUGGEST_RECIPIENTS => to_value(drafts::suggest(core, params(req.params)?).await?),
        method::LIST_SENDERS => to_value(drafts::list_senders(core).await?),
        method::CLOSE_DRAFT => {
            drafts::close(core, params(req.params)?, source).await?;
            to_value(CommandResult { changed: 0 })
        }
        method::DISCARD_DRAFT => to_value(drafts::discard(core, params(req.params)?, source).await?),
        method::SEND_DRAFT => to_value(drafts::send(core, params(req.params)?, source).await?),
        method::CANCEL_SEND => to_value(drafts::cancel(core, params(req.params)?, source).await?),
        method::AI_STATUS => to_value(ai::status(core, params(req.params)?).await?),
        method::AI_LIST_PROMPTS => to_value(ai::list_prompts(core, params(req.params)?)?),
        method::AI_RUN => to_value(ai::run(core, params(req.params)?, source).await?),
        method::AI_CANCEL => to_value(ai::cancel(core, params(req.params)?).await?),
        method::AI_GET_RUN => to_value(ai::get_run(core, params(req.params)?).await?),
        method::AI_APPLY => to_value(ai::apply(core, params(req.params)?, source).await?),
        method::AI_DISMISS => to_value(ai::dismiss(core, params(req.params)?).await?),
        method::AI_SUGGESTIONS => to_value(ai::suggestions(core, params(req.params)?).await?),
        method::AI_DISMISS_SUGGESTION => to_value(ai::dismiss_suggestion(core, params(req.params)?).await?),
        method::RAW_GC => {
            let core2 = core.clone();
            let r = tokio::task::spawn_blocking(move || sync::run_gc(&core2))
                .await
                .map_err(|e| RpcError::internal(e))?
                .map_err(store_error)?;
            to_value(nm_api::RawGcResult { blobs: r.blobs, temporary: r.temporary, orphans: r.orphans, attachment_dirs: r.attachment_dirs, outgoing: r.outgoing })
        }
        method::SYNC_STATUS => {
            let p: nm_api::SyncAccountParams = params(req.params)?;
            to_value(sync::status(core, p).await?)
        }
        method::SYNC_NOW => {
            let p: nm_api::SyncAccountParams = params(req.params)?;
            sync::now(core, p)?;
            to_value(CommandResult { changed: 0 })
        }
        method::SYNC_VERIFY => {
            let p: nm_api::SyncVerifyParams = params(req.params)?;
            to_value(sync::verify(core, p).await?)
        }
        method::PROBE_ACCOUNT => {
            let p: ProbeAccountParams = params(req.params)?;
            to_value(probe_account(core, p).await?)
        }
        method::SET_ACCOUNT_ORDER => {
            let p: SetAccountOrderParams = params(req.params)?;
            command(core, move |s| s.set_account_order(&p.account_ids, source)).await
        }
        method::KEYMAP_GET => {
            let keymap = core.keymap.lock().unwrap();
            let (contexts, menus) = nm_engine::keymap::view(keymap.keymap());
            to_value(nm_api::KeymapView {
                path: keymap.path().display().to_string(),
                exists: keymap.path().exists(),
                contexts,
                menus,
                error: keymap.error().map(nm_engine::problem_view),
                warnings: keymap.warnings().iter().map(nm_engine::problem_view).collect(),
            })
        }
        method::CONFIG_STATUS => {
            let as_of = as_of();
            let engine = core.engine.lock().unwrap();
            to_value(ConfigStatus {
                path: engine.path().display().to_string(),
                exists: engine.path().exists(),
                accounts_path: engine.accounts_path().display().to_string(),
                accounts_exists: engine.accounts_path().exists(),
                error: engine.error().map(nm_engine::problem_view),
                warnings: engine.warnings().iter().map(nm_engine::problem_view).collect(),
                ui: nm_api::UiConfig {
                    title_bar: engine.config().ui.title_bar.as_str().into(),
                    auto_confirm_calls: engine.config().ui.auto_confirm_calls,
                    text_size: Some(engine.config().ui.text_size),
                    status_bar_text_size: Some(engine.config().ui.status_bar_text_size),
                },
                mcp: nm_api::McpConfig {
                    message_chars: engine.config().mcp.message_chars,
                    response_chars: engine.config().mcp.response_chars,
                },
                unsubscribe: nm_api::UnsubscribeConfig { in_banned: engine.config().unsubscribe.in_banned },
                folder_names: engine.config().folder_names.to_map(),
                as_of,
            })
        }
        other => Err(RpcError::new(error_code::METHOD_NOT_FOUND, reason::METHOD_NOT_FOUND.with("method", other))),
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
    let invalid = |why: nm_api::Why| RpcError::new(error_code::INVALID_PARAMS, why);
    let path = PathBuf::from(&p.path);
    if !path.is_absolute() {
        return Err(invalid(reason::PATH_NOT_ABSOLUTE.with("path", p.path.as_str())));
    }
    let meta = fs::metadata(&path)
        .map_err(|e| invalid(reason::FILE_UNREADABLE.with("path", p.path.as_str()).with("system_text", e.to_string())))?;
    if !meta.is_file() {
        return Err(invalid(reason::NOT_A_FILE.with("path", p.path.as_str())));
    }

    // Reserve the import slot before doing anything slow, so two requests
    // cannot both start an import.
    let job_id = {
        let mut st = core.import.lock().unwrap();
        if st.status.as_ref().is_some_and(|s| s.running) {
            return Err(RpcError::new(error_code::BUSY, reason::IMPORT_RUNNING));
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
        lock_budget: LOCK_BUDGET,
    };
    let core = core.clone();
    std::thread::Builder::new()
        .name("nicemailer-import".into())
        .spawn(move || run_import(core, job_id, opts))
        .map_err(|e| RpcError::internal(e))?;
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
/// The steps of the start up to the socket, each with its duration
/// (`run`): one line in the log, such as `startup: socket ready after
/// 1840 ms (store 1210, readers 4, settings 35, recovery 580, socket 11)`.
struct StartupClock {
    started: Instant,
    last: Instant,
    steps: Vec<(&'static str, Duration)>,
}

impl StartupClock {
    fn new() -> StartupClock {
        let now = Instant::now();
        StartupClock { started: now, last: now, steps: Vec::new() }
    }

    /// Ends the step `name`, which began at the previous mark.
    fn mark(&mut self, name: &'static str) {
        let now = Instant::now();
        self.steps.push((name, now - self.last));
        self.last = now;
    }

    fn line(&self) -> String {
        startup_line("socket ready", self.last - self.started, &self.steps)
    }
}

/// A log line of the start (`StartupClock`, `open_store`): what is
/// ready, after how long, and each step. A pure function, tested.
fn startup_line(what: &str, total: Duration, steps: &[(&str, Duration)]) -> String {
    let steps: Vec<String> = steps.iter().map(|(name, d)| format!("{name} {}", d.as_millis())).collect();
    format!("startup: {what} after {} ms ({})", total.as_millis(), steps.join(", "))
}

fn start_reindex(core: &Arc<Core>) -> anyhow::Result<()> {
    let core = core.clone();
    // Everything happens on the reindex thread, the check included: the
    // main thread must reach the loop that accepts connections at once
    // (a slow check here once left the app unanswered for 30 s).
    std::thread::Builder::new().name("nicemailer-reindex".into()).spawn(move || {
        // Counted on a read connection, not under the store lock: the
        // count reads indexes of both files, cold at the start (up to 2.5 s
        // on a large mailbox), and held the lock meanwhile, so the app's
        // first requests waited for it.
        let pending = match core.readers.read(nm_store::reindex::pending) {
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

/// Starts the catch-up of people and robots, if mails stored before
/// version 12 have not had their clues read (`expediteurs.md`, section 10):
/// a plain thread, like the reindex, that takes the store lock one batch at
/// a time. Until it is done, those mails have no verdict, and the filter
/// "human" leaves them out.
///
/// No event: a list filtered on human / robot is the only one that changes,
/// and the app reloads it when it is filtered again.
fn start_sender_backfill(core: &Arc<Core>) -> anyhow::Result<()> {
    let core = core.clone();
    std::thread::Builder::new().name("nicemailer-senders".into()).spawn(move || {
        // On a read connection too (see `start_reindex`).
        let pending = match core.readers.read(nm_store::senders::pending) {
            Ok(0) => return,
            Ok(n) => n,
            Err(e) => return eprintln!("personnes et robots : {e}"),
        };
        eprintln!("personnes et robots : {pending} mails à classer, en tâche de fond");
        let mut last_logged = std::time::Instant::now();
        let mut progress = |done: u64, total: u64| {
            if last_logged.elapsed().as_secs() >= 10 {
                eprintln!("personnes et robots : {done} / {total}");
                last_logged = std::time::Instant::now();
            }
        };
        match nm_store::senders::backfill(&core.store, &core.raw, nm_store::senders::BATCH_SIZE, &mut progress) {
            Ok(r) => eprintln!(
                "personnes et robots : {} mails classés, {} sans brut lisible, en {:.1} s",
                r.read, r.missing_raw, r.seconds
            ),
            Err(e) => eprintln!("personnes et robots : échec : {e}"),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The line of the start: the total, then each step in its order.
    #[test]
    fn the_startup_line_gives_each_step() {
        let ms = Duration::from_millis;
        assert_eq!(
            startup_line("socket ready", ms(1840), &[("store", ms(1210)), ("readers", ms(4)), ("socket", ms(11))]),
            "startup: socket ready after 1840 ms (store 1210, readers 4, socket 11)"
        );
    }

    /// The line of a slow request: dated, its total, its wait for the
    /// database and the rest; a wait measured longer than the total (clocks
    /// read at different points) is capped.
    #[test]
    fn a_slow_request_line_splits_waiting_and_running() {
        let ms = Duration::from_millis;
        assert_eq!(
            slow_request_line("09:12:03", "get_thread", ms(9630), ms(9500)),
            "09:12:03 slow request: get_thread took 9630 ms (9500 ms waiting for the database, 130 ms running)"
        );
        assert_eq!(
            slow_request_line("09:12:03", "list_threads", ms(600), ms(700)),
            "09:12:03 slow request: list_threads took 600 ms (600 ms waiting for the database, 0 ms running)"
        );
    }

    #[test]
    fn the_local_time_is_hours_minutes_seconds() {
        let t = local_time();
        assert_eq!(t.len(), 8, "{t}");
        assert_eq!((&t[2..3], &t[5..6]), (":", ":"), "{t}");
    }
}
