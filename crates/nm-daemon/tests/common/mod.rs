//! Shared harness of the end-to-end tests: starts the real
//! `nicemailer-core` binary on a temporary data directory and talks to it
//! like the CLI does.
//!
//! Rust note: a file in `tests/common/` is not a test program of its own;
//! each test file includes it with `mod common;`. Not every test uses every
//! helper, hence `allow(dead_code)`.
#![allow(dead_code)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// The core process, killed when the test ends (even on failure).
pub struct Core(Child);

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
        c.call("hello", json!({"protocol_version": 1, "client": "cli"})).unwrap();
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
/// Its settings file is `dir/config.toml`, missing unless the test writes
/// it: a test must never read the settings (and accounts) of the machine.
pub fn start(dir: &Path) -> Core {
    Core(
        Command::new(env!("CARGO_BIN_EXE_nicemailer-core"))
            .arg("--data-dir")
            .arg(dir)
            .arg("--config")
            .arg(dir.join("config.toml"))
            .spawn()
            .unwrap(),
    )
}

impl Client {
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
