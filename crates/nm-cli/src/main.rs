//! NiceMailer CLI: a client of the core's socket, plus development tools.
//!
//! Unlike the core, the CLI is *synchronous*: it sends one request, waits
//! for the answer, prints it and exits. Plain blocking I/O from the standard
//! library is enough, no async runtime needed.
//!
//! The `///` comments on the `Cli` and `Command` items below are turned into
//! `--help` text by clap. Like everything the CLI prints, they are in English
//! (see CLAUDE.md, "Languages").

use std::fs::{self, File};
// `BufRead` and `Write` are traits: importing them makes `read_line` and
// `writeln!` available on the reader and the stream.
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

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
#[command(name = "nicemailer", version, about = "NiceMailer command-line client")]
struct Cli {
    /// Data directory (database, socket).
    #[arg(long, global = true, env = "NICEMAILER_DATA_DIR")]
    data_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

/// Subcommands: each enum variant becomes a subcommand (`nicemailer seed`,
/// `nicemailer call`...), and its fields become its options.
#[derive(Subcommand)]
enum Command {
    /// Fill the database with fake mail (development). The core must be stopped.
    Seed {
        /// Number of conversations (about two mails each).
        #[arg(long, default_value_t = 2500)]
        threads: usize,
        /// Generator seed: the same seed produces the same database.
        #[arg(long, default_value_t = 1)]
        seed: u64,
        /// Delete the existing database before filling it.
        #[arg(long)]
        reset: bool,
    },
    /// Import a whole mailbox from an mbox file into an archive account.
    ///
    /// The core does the work and must be running. Messages sent from
    /// --address go to Sent, all others to Archive, all marked as read.
    /// Running the same import again only adds the messages that are missing.
    ImportMbox {
        /// The mbox file.
        file: PathBuf,
        /// Name of the archive account, created if needed (letters, digits, - _ .).
        #[arg(long)]
        account: String,
        /// Address of the archived mailbox.
        #[arg(long)]
        address: String,
        /// Do not give the "inbox" screening decision to the people you wrote to.
        #[arg(long)]
        no_correspondents: bool,
    },
    /// Show the running mbox import, or the last one.
    ImportStatus,
    /// Send a JSON-RPC request to the core and print the result.
    Call {
        method: String,
        /// JSON parameters, e.g. '{"folder_id": 1}'.
        params: Option<String>,
    },
    /// List the folders.
    Folders,
    /// Print the core's events as they happen.
    Watch,
    /// Screening decision on an address: where its next mails go. Taking or
    /// changing one also files the mails already received from it.
    Address {
        /// The address, e.g. news@example.com.
        addr: String,
        /// inbox, newsletter, paperwork, banned, or none to remove the decision.
        target: String,
    },
    /// Screening decision on a domain and its subdomains, for the addresses
    /// that have no decision of their own.
    Domain {
        /// The domain, e.g. example.com.
        domain: String,
        /// inbox, newsletter, paperwork, banned, or none to remove the decision.
        target: String,
    },
    /// Undo a move or a decision, from the token it printed.
    Undo { token: String },
    /// The settings file, config.toml.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// List the accounts and their state, as the core sees them, in their
    /// display order.
    Accounts,
    /// Work on one account.
    Account {
        #[command(subcommand)]
        action: AccountAction,
    },
    /// Talk to an account's IMAP server.
    Imap {
        #[command(subcommand)]
        action: ImapAction,
    },
}

/// `nicemailer imap …`: through the core, which alone reads the password.
#[derive(Subcommand)]
enum ImapAction {
    /// Connect to the account's server and show what it offers: its
    /// capabilities before and after login, its folders, and the workflow
    /// role NiceMailer would propose for each. Changes nothing, on the
    /// server or in the database.
    Probe {
        /// Account key, as in [accounts.<key>].
        key: String,
        /// Print the report as JSON.
        #[arg(long)]
        json: bool,
    },
}

/// `nicemailer account …`.
#[derive(Subcommand)]
enum AccountAction {
    /// Save the IMAP password of an account in the Keychain, for the server
    /// config.toml names now. Asked without echo; read from the standard
    /// input when it is not a terminal (`printf %s "$PW" | nicemailer …`).
    Password {
        /// Account key, as in [accounts.<key>].
        key: String,
    },
    /// Change the display order (account picker, sidebar): the accounts
    /// named come first, in this order, the others after them.
    Order {
        /// Account keys, e.g. `perso pro`.
        #[arg(required = true)]
        keys: Vec<String>,
    },
}

/// The report of `nicemailer imap probe`, for a terminal.
fn print_probe(r: &nm_api::ProbeReport) {
    println!(
        "Server {} ({}): connected in {} ms, logged in in {} ms, folders listed in {} ms",
        r.server, r.security, r.connect_ms, r.login_ms, r.list_ms
    );
    println!("Before login: {}", r.capabilities_before_login.join(" "));
    println!("After login:  {}", r.capabilities.join(" "));
    println!("Enabled:      {}", if r.enabled.is_empty() { "none".to_string() } else { r.enabled.join(" ") });
    if let Some(ns) = &r.namespace {
        println!("Namespace:    prefix {:?}, separator {}", ns.prefix, ns.delimiter.as_deref().unwrap_or("none"));
    }
    println!();
    let width = r.folders.iter().map(|f| f.name.chars().count()).max().unwrap_or(0).clamp(6, 40);
    println!("{:<width$}  {:<24}  {:>8}  {:>7}  attributes", "folder", "proposed role", "messages", "unread");
    for f in &r.folders {
        let role = match (&f.proposed_role, &f.role_from) {
            (Some(role), Some(from)) if from != "inbox" => format!("{role} ({from})"),
            (Some(role), _) => role.clone(),
            _ if !f.selectable => "(holds no mail)".to_string(),
            _ => "-".to_string(),
        };
        let n = |v: Option<u32>| v.map(|v| v.to_string()).unwrap_or_else(|| "-".into());
        println!("{:<width$}  {:<24}  {:>8}  {:>7}  {}", f.name, role, n(f.messages), n(f.unseen), f.attributes.join(" "));
    }
}

/// `nicemailer config …`: works on the file directly, without the core.
#[derive(Subcommand)]
enum ConfigAction {
    /// Check the file without applying it: errors (with their line) and
    /// warnings. Exits with status 1 if the core would refuse it.
    Check {
        /// File to check; by default the one the core reads.
        file: Option<PathBuf>,
    },
    /// Print a complete file: every key at its default value, with what it
    /// does in a comment.
    Defaults,
    /// Print the path of the file: $NICEMAILER_CONFIG, or
    /// ~/.config/nicemailer/config.toml.
    Path,
}

/// `none` removes the decision; anything else must be a screening target.
fn decision_target(target: &str) -> anyhow::Result<Value> {
    match target {
        "none" => Ok(Value::Null),
        t if nm_core::ScreeningTarget::parse(t).is_some() => Ok(json!(t)),
        t => bail!("unknown target {t}: inbox, newsletter, paperwork, banned or none"),
    }
}

/// Prints the result of a workflow command (`WorkflowResult`).
fn print_workflow(result: &Value) {
    if let Some(d) = result["decision"].as_object() {
        let name = |v: &Value| v.as_str().unwrap_or("none").to_string();
        println!("{} {}: {} -> {}", d["kind"].as_str().unwrap_or(""), name(&d["name"]), name(&d["before"]), name(&d["after"]));
    }
    println!("{} moved, {} other mails filed", result["moved"], result["filed"]);
    if let Some(t) = result["undo"].as_str() {
        println!("undo: nicemailer undo {t}");
    }
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let data_dir = cli.data_dir.unwrap_or_else(paths::data_dir);
    // Matching on an enum variant with fields extracts them: `seed: s`
    // renames the `seed` field to `s`, to avoid shadowing the `seed` function.
    match cli.command {
        Command::Seed { threads, seed: s, reset } => run_seed(&data_dir, threads, s, reset),
        Command::ImportMbox { file, account, address, no_correspondents } => {
            run_import(&data_dir, &file, &account, &address, !no_correspondents)
        }
        Command::ImportStatus => {
            let mut client = Client::connect(&data_dir)?;
            let status = client.call("import_status", Value::Null)?;
            if status.is_null() {
                println!("No import since the core started.");
            } else {
                print_status(&status);
            }
            Ok(())
        }
        Command::Call { method, params } => {
            let params: Value = match params {
                Some(p) => serde_json::from_str(&p).context("invalid JSON parameters")?,
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
                    "{:>3}  {:<8} {}  {:<12} {:>6} mails {:>6} unread {:>6} threads",
                    f["id"],
                    f["account_name"].as_str().unwrap_or(""),
                    f["letter"].as_str().unwrap_or(" "),
                    f["name"].as_str().unwrap_or(""),
                    f["total_count"],
                    f["unread_count"],
                    f["thread_count"],
                );
            }
            Ok(())
        }
        Command::Address { addr, target } => {
            let mut client = Client::connect(&data_dir)?;
            let result = client.call("set_address_screening", json!({ "addr": addr, "target": decision_target(&target)? }))?;
            print_workflow(&result);
            Ok(())
        }
        Command::Domain { domain, target } => {
            let mut client = Client::connect(&data_dir)?;
            let result =
                client.call("set_domain_screening", json!({ "domain": domain, "target": decision_target(&target)? }))?;
            print_workflow(&result);
            Ok(())
        }
        Command::Undo { token } => {
            let mut client = Client::connect(&data_dir)?;
            let result = client.call("undo", json!({ "token": token }))?;
            print_workflow(&result);
            Ok(())
        }
        Command::Config { action } => run_config(action),
        Command::Account { action: AccountAction::Password { key } } => {
            // Connect first: no point asking for a password the core
            // cannot receive.
            let mut client = Client::connect(&data_dir)?;
            let password = read_password(&format!("IMAP password of {key}: "))?;
            client.call("set_password", json!({ "account": key, "password": password }))?;
            let result = client.call("list_accounts", Value::Null)?;
            let account = result["items"].as_array().into_iter().flatten().find(|a| a["key"] == key.as_str());
            match account.and_then(|a| a["error"].as_str()) {
                Some(e) => println!("Saved, but the account still has an error: {e}"),
                None => println!("Saved in the Keychain."),
            }
            Ok(())
        }
        Command::Account { action: AccountAction::Order { keys } } => {
            let mut client = Client::connect(&data_dir)?;
            let items = client.call("list_accounts", Value::Null)?["items"].as_array().cloned().unwrap_or_default();
            let mut ids = Vec::new();
            for k in &keys {
                match items.iter().find(|a| a["key"].as_str() == Some(k.as_str())) {
                    Some(a) => ids.push(a["id"].clone()),
                    None => bail!("no account {k} (see `nicemailer accounts`)"),
                }
            }
            client.call("set_account_order", json!({ "account_ids": ids }))?;
            for (i, k) in keys.iter().enumerate() {
                println!("{}. {k}", i + 1);
            }
            Ok(())
        }
        Command::Imap { action: ImapAction::Probe { key, json } } => {
            let mut client = Client::connect(&data_dir)?;
            let report = client.call("probe_account", json!({ "account": key }))?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                let r: nm_api::ProbeReport = serde_json::from_value(report)?;
                print_probe(&r);
            }
            Ok(())
        }
        Command::Accounts => {
            let mut client = Client::connect(&data_dir)?;
            let result = client.call("list_accounts", Value::Null)?;
            let items = result["items"].as_array().cloned().unwrap_or_default();
            if items.is_empty() {
                println!("No account. Declare one in {} (see `nicemailer config defaults`).", nm_engine::config::default_path().display());
            }
            for a in items {
                let state = match (a["enabled"].as_bool(), a["error"].as_str()) {
                    (Some(false), _) => "disabled (no longer in config.toml, mail kept)".to_string(),
                    (_, Some(e)) => format!("error: {e}"),
                    _ => "ok".to_string(),
                };
                println!(
                    "{:>3}  {:<12} {:<8} {:<30} {state}",
                    a["id"],
                    a["key"].as_str().unwrap_or(""),
                    a["kind"].as_str().unwrap_or(""),
                    a["email"].as_str().unwrap_or("-"),
                );
            }
            Ok(())
        }
        Command::Watch => {
            let mut client = Client::connect(&data_dir)?;
            let pos = client.call("subscribe", json!({}))?;
            eprintln!("subscribed from {pos}");
            // Print every line pushed by the core, until it closes the
            // connection (then `read_line` returns an error and `?` exits).
            loop {
                let line = client.read_line()?;
                println!("{line}");
            }
        }
    }
}

/// Reads a password: from the terminal without echo, or a line of the
/// standard input when it is not a terminal (a script, a pipe).
///
/// Turning echo off is done by hand with the C library (`termios`): the
/// terminal's settings are read, the `ECHO` flag removed, and the old
/// settings put back after the line is read, even when reading fails.
fn read_password(prompt: &str) -> anyhow::Result<String> {
    use std::io::IsTerminal;
    let stdin = std::io::stdin();
    let mut line = String::new();
    if !stdin.is_terminal() {
        stdin.lock().read_line(&mut line)?;
    } else {
        eprint!("{prompt}");
        std::io::stderr().flush()?;
        // Rust note: `unsafe` marks calls the compiler cannot check, here
        // functions of the C library. `mem::zeroed` makes a blank
        // `termios` struct for `tcgetattr` to fill.
        let fd = libc::STDIN_FILENO;
        let mut old: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(fd, &mut old) } != 0 {
            bail!("cannot read the terminal settings");
        }
        let mut quiet = old;
        quiet.c_lflag &= !libc::ECHO;
        unsafe { libc::tcsetattr(fd, libc::TCSANOW, &quiet) };
        let read = stdin.lock().read_line(&mut line);
        unsafe { libc::tcsetattr(fd, libc::TCSANOW, &old) };
        // The Enter key was not echoed either.
        eprintln!();
        read?;
    }
    // Only the line ending is removed: spaces may be part of a password.
    let password = line.trim_end_matches(['\n', '\r']).to_string();
    if password.is_empty() {
        bail!("empty password");
    }
    Ok(password)
}

/// `nicemailer config check | defaults | path`.
fn run_config(action: ConfigAction) -> anyhow::Result<()> {
    use nm_engine::config;
    match action {
        ConfigAction::Path => println!("{}", config::default_path().display()),
        ConfigAction::Defaults => print!("{}", config::defaults_file()),
        ConfigAction::Check { file } => {
            let path = file.unwrap_or_else(config::default_path);
            let text = match config::read(&path) {
                Ok(Some(t)) => t,
                Ok(None) => {
                    println!("{}: no file; the core starts with the defaults and no account", path.display());
                    return Ok(());
                }
                Err(p) => bail!("{}: {p}", path.display()),
            };
            match config::parse(&text) {
                Ok(parsed) => {
                    for w in &parsed.warnings {
                        println!("{}: warning: {w}", path.display());
                    }
                    let n = parsed.config.accounts.len();
                    println!("{}: valid, {n} account{}", path.display(), if n == 1 { "" } else { "s" });
                }
                // `std::process::exit` rather than an error: the message is
                // the whole output, without anyhow's "Error:" prefix, and
                // the status says "refused" to scripts.
                Err(p) => {
                    println!("{}: refused: {p}", path.display());
                    std::process::exit(1);
                }
            }
        }
    }
    Ok(())
}

/// `nicemailer seed`: fills the database with fake mail, directly through
/// the store (not through the core).
fn run_seed(data_dir: &Path, threads: usize, s: u64, reset: bool) -> anyhow::Result<()> {
    fs::create_dir_all(data_dir)?;
    // The core is the only owner of the database: take its lock, so seeding
    // is impossible while the core runs (it would not see the new data, and
    // deleting the file under it would break it).
    let lock = File::create(data_dir.join("lock"))?;
    if lock.try_lock().is_err() {
        bail!("the core is running and holds the database. Stop it (Ctrl-C in its terminal, or `pkill nicemailer-core`), then run seed again");
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
        .context("use --reset to replace the existing database")?;
    println!(
        "{}: {} threads, {} mails, {} addresses in {:.1} s",
        db.display(),
        report.threads,
        report.messages,
        report.addresses,
        started.elapsed().as_secs_f64()
    );
    // `lock` is dropped here, at the end of the function, which releases it.
    Ok(())
}

/// `nicemailer import-mbox`: asks the core to import the file, then follows
/// the import's events and prints its progress until it ends.
///
/// The import runs in the core: interrupting this command (Ctrl-C) does not
/// stop it. `nicemailer import-status` shows where it is.
fn run_import(data_dir: &Path, file: &Path, account: &str, address: &str, correspondents: bool) -> anyhow::Result<()> {
    // The core has its own working directory, so it needs an absolute path.
    let path = fs::canonicalize(file).with_context(|| format!("{}", file.display()))?;
    let mut client = Client::connect(data_dir)?;
    // Subscribe *before* starting, so no event of this import can be missed.
    client.call("subscribe", json!({}))?;
    let started = client.call(
        "import_mbox",
        json!({
            "path": path.to_string_lossy(),
            "account": account,
            "address": address,
            "mark_correspondents": correspondents,
        }),
    )?;
    let job_id = started["job_id"].as_u64().unwrap_or_default();
    eprintln!("Importing {} into the archive account \"{account}\" (Ctrl-C stops following, not the import)", path.display());
    let clock = Instant::now();
    loop {
        let line = client.read_line()?;
        let v: Value = serde_json::from_str(&line)?;
        let params = &v["params"];
        match v["method"].as_str() {
            Some("ImportProgress") if params["job_id"].as_u64() == Some(job_id) => {
                let read = params["bytes_read"].as_u64().unwrap_or(0) as f64;
                let total = params["total_bytes"].as_u64().unwrap_or(0).max(1) as f64;
                let secs = clock.elapsed().as_secs_f64().max(0.001);
                // `\r` returns to the start of the line, so the progress line
                // is rewritten in place instead of scrolling.
                eprint!(
                    "\r{:5.1} %  {:.2} / {:.2} GB  {} imported  {} already present  {:.0} MB/s   ",
                    100.0 * read / total,
                    read / 1e9,
                    total / 1e9,
                    params["imported"],
                    params["skipped"],
                    read / 1e6 / secs
                );
            }
            Some("ImportFinished") if params["job_id"].as_u64() == Some(job_id) => {
                eprintln!();
                print_status(params);
                if let Some(e) = params["error"].as_str() {
                    bail!("import failed: {e}");
                }
                return Ok(());
            }
            // Missed events (this client fell behind): ask for the state.
            Some("ResyncRequired") => {
                let status = client.call("import_status", Value::Null)?;
                if status["job_id"].as_u64() == Some(job_id) && status["running"] == false {
                    print_status(&status);
                    return Ok(());
                }
            }
            _ => {}
        }
    }
}

/// Prints an import status (`ImportStatus` in nm-api).
fn print_status(s: &Value) {
    let state = if s["running"] == true {
        "running"
    } else if s["error"].is_string() {
        "failed"
    } else {
        "finished"
    };
    println!("Import {} ({state}): {}", s["job_id"], s["path"].as_str().unwrap_or(""));
    println!(
        "  {} imported ({} to Sent, {} to Archive), {} already present, {} not parsed",
        s["imported"], s["sent"], s["archived"], s["skipped"], s["parse_errors"]
    );
    if s["running"] == false && s["error"].is_null() {
        println!("  {} correspondents now go straight to the Inbox; {:.1} s", s["correspondents"], s["seconds"].as_f64().unwrap_or(0.0));
    }
    if let Some(e) = s["error"].as_str() {
        println!("  error: {e}");
    }
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
            .with_context(|| format!("cannot connect to {}: is the core running? (make core)", path.display()))?;
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
            bail!("connection closed by the core");
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
                bail!("error {}: {}", e.code, e.message);
            }
            return Ok(r.result.unwrap_or(Value::Null));
        }
    }
}
