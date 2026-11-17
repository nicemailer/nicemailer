//! The connection of `nicemailer-mcp` to the core (`specifications/mcp.md`,
//! section 3, "Connexion au noyau").
//!
//! - **Lazy.** Nothing is opened before the first tool call: the MCP
//!   client gets its `initialize` answered at once, whether the core runs
//!   or not. A connection lost (the core restarted, the mailbox changed in
//!   the app) is opened again at the next call.
//! - **Started on demand**, as every client does (`architecture.md`,
//!   section 6, "Démarrage à la demande par les clients"): no socket, then
//!   `launchctl kickstart` of the core's launch agent, and tries for 3
//!   seconds. Only for the mailbox chosen in the app: with
//!   `NICEMAILER_DATA_DIR` or `--data-dir`, the core of launchd would serve
//!   another directory, so it is not started (and a test never starts the
//!   user's core).
//! - **Maintenance.** While NiceMailer is updated or the core upgrades the
//!   database, the core answers the error `MAINTENANCE` and closes the
//!   connection (`mise-a-jour.md`, section 2): the proxy waits, trying
//!   every second for 30 seconds, without starting a core, and says so
//!   through the progress notifications of the tool call. A maintenance
//!   that only the user can end (a database written by a newer NiceMailer,
//!   a migration that failed) is not waited for: the agent gets its reason.
//! - **Several calls at once.** An MCP client may run tools in parallel:
//!   requests are written as they come, each with its id, and a task reads
//!   the responses and hands each to the call waiting for it.
//!
//! Rust note: `tokio::sync::Mutex` can be held across an `.await` (the
//! connection is opened under it, so two calls do not open two); the
//! `std::sync::Mutex` of the pending calls is only held for a lookup, never
//! across an `.await`, and is the cheaper of the two.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nm_api::{PROTOCOL_VERSION, RpcError, error_code, method, paths};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::OwnedWriteHalf;
use tokio::sync::oneshot;

/// What went wrong talking to the core.
#[derive(Debug, Clone)]
pub enum CoreError {
    /// The core answered with an error.
    Rpc(RpcError),
    /// No core: the message says what to do.
    Unreachable(String),
    /// The core is still in maintenance after the wait (an update, an
    /// upgrade of the mailbox).
    Maintenance,
    /// The connection closed before the answer: the core stopped or
    /// restarted. The request may or may not have run.
    Lost,
}

/// Something to tell the user while a call waits (a progress notification
/// of MCP, when the client asked for them).
pub type Progress = dyn Fn(&str) + Send + Sync;

/// How long to wait; shorter in the tests.
#[derive(Debug, Clone, Copy)]
pub struct Waits {
    /// For a core being started (`kickstart`).
    pub start: Duration,
    /// For a core in maintenance.
    pub maintenance: Duration,
    /// Between two tries during maintenance.
    pub maintenance_step: Duration,
}

impl Default for Waits {
    fn default() -> Self {
        Waits { start: Duration::from_secs(3), maintenance: Duration::from_secs(30), maintenance_step: Duration::from_secs(1) }
    }
}

/// The link to the core, shared by every tool call.
pub struct CoreLink {
    /// The data directory given on the command line; `None`: the one
    /// chosen in the app, read again at each connection
    /// (`nm_api::paths::data_dir`).
    dir: Option<PathBuf>,
    waits: Waits,
    conn: tokio::sync::Mutex<Option<Arc<Conn>>>,
}

/// The calls waiting for their answer, by the id of their request: each
/// holds the sending half of a one-shot channel the reading task fills.
type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, RpcError>>>>>;

/// One open connection.
struct Conn {
    dir: PathBuf,
    write: tokio::sync::Mutex<OwnedWriteHalf>,
    pending: Pending,
    next_id: AtomicU64,
    closed: Arc<AtomicBool>,
}

impl CoreLink {
    pub fn new(dir: Option<PathBuf>, waits: Waits) -> CoreLink {
        CoreLink { dir, waits, conn: tokio::sync::Mutex::new(None) }
    }

    /// The data directory of the connection, opening it if needed: where
    /// the proxy stages the files the agent attaches (`incoming.noindex`).
    pub async fn data_dir(&self, progress: &Progress) -> Result<PathBuf, CoreError> {
        Ok(self.connection(progress).await?.dir.clone())
    }

    /// Calls `method` of the contract. A core in maintenance is waited
    /// for, as at the connection.
    pub async fn call(&self, method: &str, params: Value, progress: &Progress) -> Result<Value, CoreError> {
        let started = Instant::now();
        loop {
            let conn = self.connection(progress).await?;
            match conn.call(method, params.clone()).await {
                Err(CoreError::Rpc(e)) if e.code == error_code::MAINTENANCE => {
                    // The core closes the connection after this error.
                    self.forget(&conn).await;
                    if let Some(reason) = lasting(&e) {
                        return Err(CoreError::Unreachable(reason));
                    }
                    if started.elapsed() >= self.waits.maintenance {
                        return Err(CoreError::Maintenance);
                    }
                    progress("NiceMailer is being updated or is upgrading its mailbox; waiting");
                    tokio::time::sleep(self.waits.maintenance_step).await;
                }
                Err(CoreError::Lost) => {
                    self.forget(&conn).await;
                    return Err(CoreError::Lost);
                }
                other => return other,
            }
        }
    }

    /// [`CoreLink::call`] for a method that only reads: tried once more on
    /// a new connection when the first one was lost (the core restarted
    /// between two calls of the agent), since reading twice changes
    /// nothing.
    pub async fn read(&self, method: &str, params: Value, progress: &Progress) -> Result<Value, CoreError> {
        match self.call(method, params.clone(), progress).await {
            Err(CoreError::Lost) => self.call(method, params, progress).await,
            other => other,
        }
    }

    /// Drops `conn` if it is still the current one.
    async fn forget(&self, conn: &Arc<Conn>) {
        let mut current = self.conn.lock().await;
        if current.as_ref().is_some_and(|c| Arc::ptr_eq(c, conn)) {
            *current = None;
        }
    }

    /// The open connection, or a new one.
    async fn connection(&self, progress: &Progress) -> Result<Arc<Conn>, CoreError> {
        let mut current = self.conn.lock().await;
        if let Some(c) = current.as_ref()
            && !c.closed.load(Ordering::Acquire)
        {
            return Ok(c.clone());
        }
        *current = None;
        let conn = Arc::new(self.open(progress).await?);
        *current = Some(conn.clone());
        Ok(conn)
    }

    /// Opens a connection to the core of the data directory and makes the
    /// handshake (module documentation).
    async fn open(&self, progress: &Progress) -> Result<Conn, CoreError> {
        let (dir, chosen_in_app) = match &self.dir {
            Some(d) => (d.clone(), false),
            None => (paths::data_dir(), std::env::var_os("NICEMAILER_DATA_DIR").is_none()),
        };
        let socket = dir.join(paths::SOCKET);
        let first_try = Instant::now();
        // The start wait counts from the first try, or from the end of a
        // maintenance: after an upgrade, the core still opens its socket.
        let mut started = first_try;
        let mut in_maintenance = false;
        let mut kicked = false;
        let mut delay = Duration::from_millis(50);
        loop {
            match UnixStream::connect(&socket).await {
                Ok(stream) => {
                    let conn = Conn::start(dir.clone(), stream);
                    match conn.call(method::HELLO, json!({"protocol_version": PROTOCOL_VERSION, "client": "mcp"})).await {
                        Ok(_) => return Ok(conn),
                        Err(CoreError::Rpc(e)) if e.code == error_code::MAINTENANCE => {
                            if let Some(reason) = lasting(&e) {
                                return Err(CoreError::Unreachable(reason));
                            }
                        }
                        Err(CoreError::Rpc(e)) if e.code == error_code::PROTOCOL_VERSION => {
                            return Err(CoreError::Unreachable(format!(
                                "this nicemailer-mcp and the running NiceMailer core do not speak the same protocol ({}): update NiceMailer",
                                e.message
                            )));
                        }
                        Err(CoreError::Rpc(e)) => return Err(CoreError::Rpc(e)),
                        // Closed during the handshake: a core stopping.
                        Err(_) => {}
                    }
                }
                Err(_) => {
                    if std::mem::take(&mut in_maintenance) {
                        started = Instant::now();
                    }
                    if chosen_in_app && !kicked {
                        kicked = true;
                        progress("starting NiceMailer's core");
                        kickstart().await;
                    }
                    if started.elapsed() >= self.waits.start {
                        return Err(CoreError::Unreachable(unreachable_message(&dir, chosen_in_app)));
                    }
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(Duration::from_millis(500));
                    continue;
                }
            }
            // NiceMailer is updated, or the core upgrades the mailbox:
            // waited for, longer.
            in_maintenance = true;
            if first_try.elapsed() >= self.waits.maintenance {
                return Err(CoreError::Maintenance);
            }
            progress("NiceMailer is being updated or is upgrading its mailbox; waiting");
            tokio::time::sleep(self.waits.maintenance_step).await;
        }
    }
}

/// What to do when no core answers, said to the agent, which tells the
/// user.
fn unreachable_message(dir: &Path, chosen_in_app: bool) -> String {
    if chosen_in_app {
        format!(
            "NiceMailer is not running (no core at {}). Ask the user to open NiceMailer; if it is open, NiceMailer must be allowed in System Settings > General > Login Items",
            dir.display()
        )
    } else {
        format!("no NiceMailer core runs on {} (set by NICEMAILER_DATA_DIR or --data-dir)", dir.display())
    }
}

/// The reason of a maintenance that does not end by itself
/// (`nm_api::Maintenance::is_transient`): waiting for it is useless, the
/// user must act. None for an update or a migration, which end.
fn lasting(e: &RpcError) -> Option<String> {
    let m: nm_api::Maintenance = serde_json::from_value(e.data.clone()?).ok()?;
    (!m.is_transient()).then(|| format!("NiceMailer cannot open its mailbox: {}. Ask the user to open NiceMailer", e.message))
}

/// `launchctl kickstart gui/<uid>/eu.nicemailer.core`: starts the core of
/// the launch agent that NiceMailer.app registered. A failure (no agent:
/// the app never opened, or turned off in System Settings) shows as the
/// socket staying closed.
async fn kickstart() {
    let target = format!("gui/{}/eu.nicemailer.core", unsafe { libc::getuid() });
    let status = tokio::process::Command::new("/bin/launchctl")
        .args(["kickstart", &target])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await;
    eprintln!("nicemailer-mcp: launchctl kickstart {target}: {status:?}");
}

impl Conn {
    /// Wraps a connected socket, with the task reading its responses.
    fn start(dir: PathBuf, stream: UnixStream) -> Conn {
        let (read, write) = stream.into_split();
        let pending: Pending = Arc::default();
        let closed = Arc::new(AtomicBool::new(false));
        let (p, c) = (pending.clone(), closed.clone());
        tokio::spawn(async move {
            let mut lines = BufReader::new(read).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                // Lines without an id are events: not subscribed, so none
                // should come; ignored if one does.
                let Some(id) = v.get("id").and_then(Value::as_u64) else { continue };
                let Some(tx) = p.lock().unwrap().remove(&id) else { continue };
                let answer = match serde_json::from_value::<nm_api::Response>(v) {
                    Ok(r) => match r.error {
                        Some(e) => Err(e),
                        None => Ok(r.result.unwrap_or(Value::Null)),
                    },
                    Err(e) => Err(RpcError::internal(format!("bad response from the core: {e}"))),
                };
                let _ = tx.send(answer);
            }
            // The core closed the connection: every call still waiting
            // fails (its sender dropped), and the next one reconnects.
            c.store(true, Ordering::Release);
            p.lock().unwrap().clear();
        });
        Conn { dir, write: tokio::sync::Mutex::new(write), pending, next_id: AtomicU64::new(1), closed }
    }

    async fn call(&self, method: &str, params: Value) -> Result<Value, CoreError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        if self.closed.load(Ordering::Acquire) {
            self.pending.lock().unwrap().remove(&id);
            return Err(CoreError::Lost);
        }
        let line = format!("{}\n", json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        if self.write.lock().await.write_all(line.as_bytes()).await.is_err() {
            self.pending.lock().unwrap().remove(&id);
            self.closed.store(true, Ordering::Release);
            return Err(CoreError::Lost);
        }
        match rx.await {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => Err(CoreError::Rpc(e)),
            Err(_) => Err(CoreError::Lost),
        }
    }
}
