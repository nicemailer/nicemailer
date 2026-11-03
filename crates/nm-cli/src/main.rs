//! NiceMailer CLI: a client of the core's socket, plus development tools.
//!
//! Unlike the core, the CLI is *synchronous*: it sends one request, waits
//! for the answer, prints it and exits. Plain blocking I/O from the standard
//! library is enough, no async runtime needed.
//!
//! The `///` comments on the `Cli` and `Command` items below are turned into
//! `--help` text by clap, which is why they stay in French (user-facing).

use std::fs::{self, File};
// `BufRead` and `Write` are traits: importing them makes `read_line` and
// `writeln!` available on the reader and the stream.
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, bail};
use clap::{Parser, Subcommand};
use nm_api::{PROTOCOL_VERSION, Response, paths};
use nm_store::Store;
use nm_store::seed::{SeedOptions, seed};
// `json!` builds a JSON value with a JSON-like syntax.
use serde_json::{Value, json};

/// Top-level arguments. `global = true` makes `--data-dir` accepted after
/// any subcommand too.
#[derive(Parser)]
#[command(name = "nicemailer", version, about = "Client en ligne de commande de NiceMailer")]
struct Cli {
    /// Dossier de données (base, socket).
    #[arg(long, global = true, env = "NICEMAILER_DATA_DIR")]
    data_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

/// Subcommands: each enum variant becomes a subcommand (`nicemailer seed`,
/// `nicemailer call`...), and its fields become its options.
#[derive(Subcommand)]
enum Command {
    /// Remplit la base de faux mails (développement). Le noyau doit être arrêté.
    Seed {
        /// Nombre de conversations (environ deux mails chacune).
        #[arg(long, default_value_t = 2500)]
        threads: usize,
        /// Graine du générateur : la même graine produit la même base.
        #[arg(long, default_value_t = 1)]
        seed: u64,
        /// Supprime la base existante avant de la remplir.
        #[arg(long)]
        reset: bool,
    },
    /// Envoie une requête JSON-RPC au noyau et affiche le résultat.
    Call {
        method: String,
        /// Paramètres JSON, par exemple '{"folder_id": 1}'.
        params: Option<String>,
    },
    /// Liste les dossiers.
    Folders,
    /// Affiche les événements du noyau au fil de l'eau.
    Watch,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let data_dir = cli.data_dir.unwrap_or_else(paths::data_dir);
    // Matching on an enum variant with fields extracts them: `seed: s`
    // renames the `seed` field to `s`, to avoid shadowing the `seed` function.
    match cli.command {
        Command::Seed { threads, seed: s, reset } => run_seed(&data_dir, threads, s, reset),
        Command::Call { method, params } => {
            let params: Value = match params {
                Some(p) => serde_json::from_str(&p).context("paramètres JSON invalides")?,
                None => Value::Null,
            };
            let mut client = Client::connect(&data_dir)?;
            let result = client.call(&method, params)?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            Ok(())
        }
        Command::Folders => {
            let mut client = Client::connect(&data_dir)?;
            let result = client.call("list_folders", Value::Null)?;
            // `as_array()` returns `Option<&Vec<Value>>`; `into_iter().flatten()`
            // iterates over the elements, or over nothing if it is `None`.
            // `{:>3}` right-aligns on 3 characters, `{:<12}` left-aligns on 12.
            for f in result["items"].as_array().into_iter().flatten() {
                println!(
                    "{:>3}  {}  {:<12} {:>6} mails {:>6} non lus {:>6} conversations",
                    f["id"],
                    f["letter"].as_str().unwrap_or(" "),
                    f["name"].as_str().unwrap_or(""),
                    f["total_count"],
                    f["unread_count"],
                    f["thread_count"],
                );
            }
            Ok(())
        }
        Command::Watch => {
            let mut client = Client::connect(&data_dir)?;
            let pos = client.call("subscribe", json!({}))?;
            eprintln!("abonné à partir de {pos}");
            // Print every line pushed by the core, until it closes the
            // connection (then `read_line` returns an error and `?` exits).
            loop {
                let line = client.read_line()?;
                println!("{line}");
            }
        }
    }
}

/// `nicemailer seed`: fills the database with fake mail, directly through
/// the store (not through the core).
fn run_seed(data_dir: &PathBuf, threads: usize, s: u64, reset: bool) -> anyhow::Result<()> {
    fs::create_dir_all(data_dir)?;
    // The core is the only owner of the database: take its lock, so seeding
    // is impossible while the core runs (it would not see the new data, and
    // deleting the file under it would break it).
    let lock = File::create(data_dir.join("lock"))?;
    if lock.try_lock().is_err() {
        bail!("le noyau tourne et tient la base. Arrêtez-le (Ctrl-C dans son terminal, ou `pkill nicemailer-core`) puis relancez seed");
    }
    let db = data_dir.join(paths::DATABASE);
    if reset {
        // In WAL mode SQLite uses two companion files next to the database:
        // `-wal` (pending writes) and `-shm` (shared index). All three must
        // go, or the new database could pick up stale pages.
        for suffix in ["", "-wal", "-shm"] {
            let p = PathBuf::from(format!("{}{suffix}", db.display()));
            if p.exists() {
                fs::remove_file(&p)?;
            }
        }
    }
    let mut store = Store::open(&db)?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;
    let started = std::time::Instant::now();
    let report = seed(&mut store, &SeedOptions { threads, seed: s, now })
        .context("utilisez --reset pour remplacer la base existante")?;
    println!(
        "{} : {} conversations, {} mails, {} adresses en {:.1} s",
        db.display(),
        report.threads,
        report.messages,
        report.addresses,
        started.elapsed().as_secs_f64()
    );
    // `lock` is dropped here, at the end of the function, which releases it.
    Ok(())
}

/// Minimal synchronous JSON-RPC client.
struct Client {
    /// Writing side of the socket.
    stream: UnixStream,
    /// Reading side, buffered to read line by line. It wraps a clone of the
    /// same socket (`try_clone` duplicates the file descriptor).
    reader: BufReader<UnixStream>,
    /// Id of the next request.
    next_id: u64,
}

impl Client {
    /// Connects to the core and performs the `hello` handshake.
    fn connect(data_dir: &std::path::Path) -> anyhow::Result<Client> {
        let path = data_dir.join(paths::SOCKET);
        let stream = UnixStream::connect(&path)
            .with_context(|| format!("connexion à {} : le noyau est-il lancé ?", path.display()))?;
        let reader = BufReader::new(stream.try_clone()?);
        let mut client = Client { stream, reader, next_id: 1 };
        client.call("hello", json!({"protocol_version": PROTOCOL_VERSION, "client": "cli"}))?;
        Ok(client)
    }

    /// Reads one line from the core, without the trailing newline.
    fn read_line(&mut self) -> anyhow::Result<String> {
        let mut line = String::new();
        // `read_line` returns the number of bytes read; 0 means end of stream.
        if self.reader.read_line(&mut line)? == 0 {
            bail!("connexion fermée par le noyau");
        }
        Ok(line.trim_end().to_string())
    }

    /// Sends a request and waits for its response, returning its `result`
    /// or turning its `error` into an error.
    fn call(&mut self, method: &str, params: Value) -> anyhow::Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let req = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        // `writeln!` writes the request followed by a newline: one request
        // per line, as the protocol requires.
        writeln!(self.stream, "{req}")?;
        loop {
            let line = self.read_line()?;
            let v: Value = serde_json::from_str(&line)?;
            // Skip lines that are not the response to this request: these are
            // event notifications, which can arrive at any time after
            // `subscribe`.
            if v.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            let r: Response = serde_json::from_value(v)?;
            if let Some(e) = r.error {
                bail!("erreur {} : {}", e.code, e.message);
            }
            return Ok(r.result.unwrap_or(Value::Null));
        }
    }
}
