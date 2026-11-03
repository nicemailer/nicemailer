//! End-to-end test of `import_mbox` through the socket: starts the real
//! `nicemailer-core` binary on a temporary data directory and talks to it
//! like the CLI does.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// The core process, killed when the test ends (even on failure).
struct Core(Child);

impl Drop for Core {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Client {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
    next_id: u64,
}

impl Client {
    fn connect(dir: &Path) -> Client {
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
        let mut c = Client { stream, reader, next_id: 1 };
        c.call("hello", json!({"protocol_version": 1, "client": "cli"})).unwrap();
        c
    }

    fn line(&mut self) -> Value {
        let mut l = String::new();
        self.reader.read_line(&mut l).unwrap();
        serde_json::from_str(&l).unwrap()
    }

    /// Sends a request; returns its result or its error object. Event lines
    /// received meanwhile are dropped.
    fn call(&mut self, method: &str, params: Value) -> Result<Value, Value> {
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
        }
    }
}

const MBOX: &str = "From a@x Tue Jun 23 19:20:56 2020
From: Alice <alice@a.example>
To: me@hey.example
Subject: Devis
Message-ID: <m1@a.example>

Bonjour

From me@hey.example Wed Jun 24 09:00:00 2020
From: me@hey.example
To: alice@a.example
Subject: Re: Devis
Message-ID: <m2@hey.example>
In-Reply-To: <m1@a.example>

Merci
";

#[test]
fn import_mbox_runs_in_the_background_and_reports_events() {
    let dir = tempfile::tempdir().unwrap();
    let mbox = dir.path().join("test.mbox");
    std::fs::write(&mbox, MBOX).unwrap();
    let _core = Core(
        Command::new(env!("CARGO_BIN_EXE_nicemailer-core"))
            .arg("--data-dir")
            .arg(dir.path())
            .spawn()
            .unwrap(),
    );
    let mut c = Client::connect(dir.path());
    c.call("subscribe", json!({})).unwrap();

    // A relative path is refused: the core does not share the CLI's
    // working directory.
    let err = c.call("import_mbox", json!({"path": "test.mbox", "account": "hey", "address": "me@hey.example"})).unwrap_err();
    assert_eq!(err["code"], -32602);

    let started = c
        .call("import_mbox", json!({"path": mbox.to_str().unwrap(), "account": "hey", "address": "me@hey.example"}))
        .unwrap();
    let job = started["job_id"].as_u64().unwrap();
    assert_eq!(started["running"], true);

    // Follow the events until the end of this import.
    let mut saw_progress = false;
    let mut saw_folder = false;
    let finished = loop {
        let v = c.line();
        match v["method"].as_str() {
            Some("ImportProgress") => saw_progress = true,
            Some("FolderChanged") => saw_folder = true,
            Some("ImportFinished") if v["params"]["job_id"].as_u64() == Some(job) => break v["params"].clone(),
            _ => {}
        }
    };
    assert!(saw_progress && saw_folder);
    assert_eq!(finished["running"], false);
    assert!(finished.get("error").is_none());
    assert_eq!((finished["imported"].as_u64(), finished["sent"].as_u64()), (Some(2), Some(1)));

    // The status is kept after the end, and the folders are visible.
    let status = c.call("import_status", Value::Null).unwrap();
    assert_eq!(status["job_id"].as_u64(), Some(job));
    let folders = c.call("list_folders", json!({})).unwrap();
    let archive = folders["items"].as_array().unwrap().iter().find(|f| f["role"] == "archive").unwrap();
    assert_eq!(archive["account_kind"], "archive");
    assert_eq!(archive["total_count"], 1);

    // Same account name with another address: refused before starting.
    let err = c
        .call("import_mbox", json!({"path": mbox.to_str().unwrap(), "account": "hey", "address": "other@x.example"}))
        .unwrap_err();
    assert_eq!(err["code"], -32602);
}
