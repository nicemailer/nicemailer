//! Shared harness of the end-to-end tests: starts the real
//! `nicemailer-core` binary on a temporary data directory and talks to it
//! like the CLI does.
//!
//! Rust note: a file in `tests/common/` is not a test program of its own;
//! each test file includes it with `mod common;`. Not every test uses every
//! helper, hence `allow(dead_code)`.
#![allow(dead_code)]

pub mod fixtures;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// The core process, killed when the test ends (even on failure).
pub struct Core(Child);

impl Core {
    /// The process id of the core, to measure it.
    pub fn pid(&self) -> u32 {
        self.0.id()
    }

    /// Waits for the core to stop by itself (a start that fails), at most
    /// `secs` seconds; its exit status.
    pub fn exit_status(&mut self, secs: u64) -> std::process::ExitStatus {
        let deadline = Instant::now() + Duration::from_secs(secs);
        loop {
            if let Some(status) = self.0.try_wait().unwrap() {
                return status;
            }
            assert!(Instant::now() < deadline, "the core did not stop within {secs} s");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for Core {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub struct Client {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
    next_id: u64,
    /// Events received while waiting for a response, kept for `event`: an
    /// event caused by a request often arrives before its response.
    pending: std::collections::VecDeque<Value>,
}

impl Client {
    pub fn connect(dir: &Path) -> Client {
        Client::connect_as(dir, "cli")
    }

    /// Connects declaring the client `client`, and returns the error of
    /// its `hello` if the core refused it (a maintenance).
    pub fn try_connect_as(dir: &Path, client: &str) -> Result<Client, Value> {
        let stream = Self::socket(dir);
        let reader = BufReader::new(stream.try_clone().unwrap());
        let mut c = Client { stream, reader, next_id: 1, pending: Default::default() };
        c.call("hello", json!({"protocol_version": 1, "client": client}))?;
        Ok(c)
    }

    /// Connects as `client` once the core serves clients: a maintenance
    /// (a migration, an update) is waited for, at most 30 seconds.
    pub fn connect_until_served(dir: &Path, client: &str) -> Client {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            match Client::try_connect_as(dir, client) {
                Ok(c) => return c,
                Err(e) if e["code"] == 1003 && Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
                Err(e) => panic!("hello refused: {e}"),
            }
        }
    }

    /// The socket of the core of `dir`, waited for (the core needs a
    /// moment to create it).
    fn socket(dir: &Path) -> UnixStream {
        let deadline = Instant::now() + Duration::from_secs(10);
        let stream = loop {
            match UnixStream::connect(dir.join("core.sock")) {
                Ok(s) => break s,
                Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
                Err(e) => panic!("core not reachable: {e}"),
            }
        };
        stream.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
        stream
    }

    /// The next line, or None when the core closed the connection.
    pub fn try_line(&mut self) -> Option<Value> {
        let mut l = String::new();
        match self.reader.read_line(&mut l) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(serde_json::from_str(&l).unwrap()),
        }
    }

    /// Connects declaring the client `client` (`app`, `cli`, `mcp`…).
    pub fn connect_as(dir: &Path, client: &str) -> Client {
        // The core needs a moment to create its socket.
        let deadline = Instant::now() + Duration::from_secs(10);
        let stream = loop {
            match UnixStream::connect(dir.join("core.sock")) {
                Ok(s) => break s,
                Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
                Err(e) => panic!("core not reachable: {e}"),
            }
        };
        stream.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
        let reader = BufReader::new(stream.try_clone().unwrap());
        let mut c = Client { stream, reader, next_id: 1, pending: Default::default() };
        c.call("hello", json!({"protocol_version": 1, "client": client})).unwrap();
        c
    }

    pub fn line(&mut self) -> Value {
        let mut l = String::new();
        self.reader.read_line(&mut l).unwrap();
        serde_json::from_str(&l).unwrap()
    }

    /// Sends a request; returns its result or its error object. Event lines
    /// received meanwhile are kept for `event`.
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, Value> {
        let id = self.next_id;
        self.next_id += 1;
        writeln!(self.stream, "{}", json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})).unwrap();
        loop {
            let v = self.line();
            if v["id"].as_u64() == Some(id) {
                return match v.get("error") {
                    Some(e) => Err(e.clone()),
                    None => Ok(v["result"].clone()),
                };
            }
            if v.get("id").is_none() {
                self.pending.push_back(v);
            }
        }
    }
}

/// Starts the core on `dir`; it is killed when the returned value drops.
///
/// The core gets nothing of the machine: its settings files are
/// `dir/config.toml` and `dir/accounts.toml`, created from the templates
/// at its start unless the test writes them first, and its keychain a
/// throwaway file `dir/test.keychain-db` (created unlocked; `security
/// create-keychain` does not add it to the user's search list), never the
/// login keychain.
pub fn start(dir: &Path) -> Core {
    start_with(dir, &[])
}

/// The same, with these environment variables given to the core
/// (`NICEMAILER_FAKE_NETWORK`…).
pub fn start_with(dir: &Path, env: &[(&str, &str)]) -> Core {
    start_logged(dir, env, None)
}

/// The same, with the core's log (its standard error, `core.log` in the
/// app) written to `log`, for a test that reads what the core said.
pub fn start_logged(dir: &Path, env: &[(&str, &str)], log: Option<&Path>) -> Core {
    let keychain = dir.join("test.keychain-db");
    if !keychain.exists() {
        for args in [["create-keychain", "-p", "test"], ["unlock-keychain", "-p", "test"]] {
            let ok = Command::new("/usr/bin/security").args(args).arg(&keychain).status().unwrap().success();
            assert!(ok, "security {args:?}");
        }
        // A new keychain locks itself after 5 minutes without use; a long
        // test (the volume test) would then lose its password. No timeout.
        let ok = Command::new("/usr/bin/security").arg("set-keychain-settings").arg(&keychain).status().unwrap().success();
        assert!(ok, "security set-keychain-settings");
    }
    Core(
        Command::new(env!("CARGO_BIN_EXE_nicemailer-core"))
            .arg("--data-dir")
            .arg(dir)
            .arg("--config")
            .arg(dir.join("config.toml"))
            .arg("--keychain")
            .arg(&keychain)
            .envs(env.iter().copied())
            .stderr(match log {
                Some(path) => std::process::Stdio::from(std::fs::File::create(path).unwrap()),
                None => std::process::Stdio::inherit(),
            })
            .spawn()
            .unwrap(),
    )
}

impl Client {
    /// Sends a request without waiting for its response; returns its id.
    /// With `response`, to send several at once and see the order in which
    /// the core answers them.
    pub fn send(&mut self, method: &str, params: Value) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        writeln!(self.stream, "{}", json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})).unwrap();
        id
    }

    /// The next response, whatever its id: `(id, result or error)`. Events
    /// received meanwhile are kept for `event`.
    pub fn response(&mut self) -> (u64, Result<Value, Value>) {
        loop {
            let v = self.line();
            match v["id"].as_u64() {
                Some(id) => {
                    return (id, match v.get("error") {
                        Some(e) => Err(e.clone()),
                        None => Ok(v["result"].clone()),
                    });
                }
                None => self.pending.push_back(v),
            }
        }
    }

    /// The next event: one kept while waiting for a response first, else
    /// the next line that is an event. For a test that reads several kinds
    /// of events in order (the fragments of an AI run, then its end).
    pub fn next_event(&mut self) -> Value {
        if let Some(v) = self.pending.pop_front() {
            return v;
        }
        loop {
            let v = self.line();
            if v.get("id").is_none() {
                return v;
            }
        }
    }

    /// Forgets the events received so far, so that `event` waits for one
    /// caused by what the test does next.
    pub fn clear_events(&mut self) {
        self.pending.clear();
    }

    /// Waits for the event `method`, dropping the others; panics after
    /// `secs` seconds. Returns its params.
    pub fn event(&mut self, method: &str, secs: u64) -> Value {
        // Rust note: `position` finds the index of the first match; `remove`
        // takes it out of the queue, so an event is returned only once.
        if let Some(i) = self.pending.iter().position(|v| v["method"] == method) {
            let v = self.pending.remove(i).expect("index found above");
            self.pending.drain(..i);
            return v["params"].clone();
        }
        self.pending.clear();
        let deadline = Instant::now() + Duration::from_secs(secs);
        loop {
            assert!(Instant::now() < deadline, "no {method} event within {secs} s");
            let v = self.line();
            if v.get("id").is_none() && v["method"] == method {
                return v["params"].clone();
            }
        }
    }
}
