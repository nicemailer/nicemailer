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

mod ai;
#[cfg(test)]
mod contract_doc;

use anyhow::{Context, bail};
use clap::{Parser, Subcommand};
use nm_api::{PROTOCOL_VERSION, Response, method, paths};
use nm_store::Store;
use nm_store::seed::{SeedOptions, seed};
// `json!` builds a JSON value with a JSON-like syntax.
use serde_json::{Value, json};

/// Top-level arguments. `global = true` makes `--data-dir` accepted after
/// any subcommand too.
#[derive(Parser)]
#[command(
    name = "nicemailer",
    version,
    about = "NiceMailer command-line client",
    after_help = "Exit status: 0 on success, 75 when the core is in maintenance (NiceMailer being updated, its database upgraded or unusable), 1 on any other error."
)]
struct Cli {
    /// Data directory (database, socket).
    #[arg(long, global = true, env = "NICEMAILER_DATA_DIR")]
    data_dir: Option<PathBuf>,
    /// While NiceMailer is being updated or upgrades its database, wait for
    /// it to finish instead of failing.
    #[arg(long, global = true)]
    wait: bool,
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
    /// The settings files, config.toml and accounts.toml.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// A mailbox folder (a data directory): what its database is, and
    /// upgrading it to this version. Works on the folder directly, without
    /// the core.
    Mailbox {
        #[command(subcommand)]
        action: MailboxAction,
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
    /// The synchronization of the IMAP accounts.
    Sync {
        #[command(subcommand)]
        action: SyncAction,
    },
    /// The store of raw messages.
    Raw {
        #[command(subcommand)]
        action: RawAction,
    },
    /// The AI assistance: its state, the prompts, running one on a draft.
    Ai {
        #[command(subcommand)]
        action: ai::AiAction,
    },
}

/// `nicemailer raw …`.
#[derive(Subcommand)]
enum RawAction {
    /// Delete now the raw files no mail references any more, the temporary
    /// files left by a crash, and the orphan files (the core does it every
    /// hour anyway).
    Gc,
}

/// `nicemailer sync …`.
#[derive(Subcommand)]
enum SyncAction {
    /// Show the state of the synchronization of each IMAP account, or of
    /// one: connection, last pass, folders, bodies left to download.
    Status {
        /// Account key, as in [accounts.<key>]; every account if omitted.
        key: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Synchronize now: every folder checked, without waiting for the next
    /// interval.
    Now {
        /// Account key; every account if omitted.
        key: Option<String>,
    },
    /// Compare the server and the database, folder by folder: UIDs, flags
    /// and the raw file of every complete message. Changes nothing. Exits
    /// with status 1 if they differ.
    Verify {
        /// Account key.
        key: String,
        /// Only check that each raw file exists, without reading it.
        #[arg(long)]
        skip_hash: bool,
        /// Set aside the raw files found corrupt or missing, and download
        /// their mails again from the server.
        #[arg(long)]
        repair: bool,
        #[arg(long)]
        json: bool,
    },
}

/// A date of the core (Unix seconds), for a terminal: "12:04:31" today,
/// "2026-11-04 12:04" before.
fn when(t: Option<i64>) -> String {
    let Some(t) = t else { return "never".into() };
    let ago = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0) - t;
    match ago {
        i64::MIN..=-1 => format!("in {} s", -ago),
        0..=59 => format!("{ago} s ago"),
        60..=3599 => format!("{} min ago", ago / 60),
        3600..=86_399 => format!("{} h ago", ago / 3600),
        _ => format!("{} days ago", ago / 86_400),
    }
}

/// The report of `nicemailer sync status`, for a terminal.
fn print_sync_status(st: &nm_api::SyncStatus) {
    if st.accounts.is_empty() {
        println!("No IMAP account is synchronized (accounts.toml has none).");
    }
    for a in &st.accounts {
        // How the connection is kept, when the server forced a change:
        // "IDLE renewed every 23 s", "polled every 10 min".
        let hold = match a.hold.as_deref().and_then(|h| h.split_once(' ')) {
            Some(("renew", n)) => format!(", IDLE renewed every {n} s"),
            Some(("poll", n)) => format!(", polled every {} min", n.parse::<u64>().unwrap_or(0) / 60),
            _ => String::new(),
        };
        print!("{} ({}{}{hold}): {}", a.key, a.mode.as_deref().unwrap_or("not connected yet"), if a.read_only { ", read-only" } else { "" }, a.state);
        if let Some(t) = a.next_retry {
            print!(", next attempt {}", when(Some(t)));
        }
        println!(", last pass {}", when(a.last_pass));
        if let Some(e) = &a.error {
            println!("  error: {e}");
        }
        println!("  changes to send to the server: {}", a.actions_pending);
        if let Some(state) = &a.initial_archive {
            match &a.initial_archive_error {
                Some(e) => println!("  initial archiving: {state} ({e})"),
                None => println!("  initial archiving: {state}"),
            }
        }
        match &a.authserv_id {
            Some(id) => println!("  trusted Authentication-Results: {id}"),
            None => println!("  trusted Authentication-Results: none found yet (every verdict is unknown)"),
        }
        println!("  bodies left to download: {}{}", a.bodies_missing, if a.bodies_refused > 0 {
            format!(", {} refused (size other than announced)", a.bodies_refused)
        } else {
            String::new()
        });
        let width = a.folders.iter().map(|f| f.path.chars().count()).max().unwrap_or(0).clamp(6, 40);
        println!("  {:<width$}  {:<11}  {:>8}  {:>9}  last sync", "folder", "role", "messages", "no body");
        for f in &a.folders {
            let last = if f.missing_on_server { "no longer on the server".to_string() } else { when(f.last_sync) };
            println!("  {:<width$}  {:<11}  {:>8}  {:>9}  {last}", f.path, f.role, f.messages, f.bodies_missing);
        }
    }
}

/// The report of `nicemailer sync verify`, for a terminal.
fn print_verify(r: &nm_api::SyncVerifyReport) {
    for f in &r.folders {
        let mut problems = Vec::new();
        let list = |v: &[u32], n: u64| {
            let shown: Vec<String> = v.iter().map(u32::to_string).collect();
            format!("{n} ({}{})", shown.join(" "), if n as usize > v.len() { " …" } else { "" })
        };
        if let Some(uv) = f.uidvalidity_differs {
            problems.push(format!("UIDVALIDITY {uv} on the server"));
        }
        if f.missing_locally_count > 0 {
            problems.push(format!("not in the database: {}", list(&f.missing_locally, f.missing_locally_count)));
        }
        if f.extra_locally_count > 0 {
            problems.push(format!("no longer on the server: {}", list(&f.extra_locally, f.extra_locally_count)));
        }
        if f.flags_differ_count > 0 {
            problems.push(format!("other flags: {}", list(&f.flags_differ, f.flags_differ_count)));
        }
        for p in &f.raw_problems {
            problems.push(format!("raw file of UID {p}"));
        }
        let state = if problems.is_empty() { "ok".to_string() } else { problems.join("; ") };
        println!(
            "{:<20} {:>7} on the server, {:>7} here, {:>7} raw files checked, {:>6} bodies to come: {state}",
            f.path, f.server_messages, f.local_messages, f.bodies_checked, f.bodies_missing
        );
    }
    println!("{} ({:.1} s)", if r.ok { "Same on both sides" } else { "Differences found" }, r.seconds);
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
    /// accounts.toml names now. Asked without echo; read from the standard
    /// input when it is not a terminal (`printf %s "$PW" | nicemailer …`).
    Password {
        /// Account key, as in [accounts.<key>].
        key: String,
        /// Save the password of the SMTP server instead, when it differs
        /// from the IMAP one (by default, sending uses the IMAP password).
        #[arg(long)]
        smtp: bool,
    },
    /// Create an IMAP account: find its servers (unless --host is given),
    /// try them, show the folders and the roles proposed, then create the
    /// missing folders, archive the old mail of the Inbox on the server if
    /// asked, accept the people you wrote to, save the password and add the
    /// account to accounts.toml, with its SMTP server when one worked.
    /// Every step is printed. The password is asked without echo, or read
    /// from the standard input.
    Add {
        /// Key of the account, as in [accounts.<key>]. Default: the name of
        /// the domain for its first account (pierre.dupont@toto-mail.com:
        /// toto-mail), else the whole address (pierre_dupont_toto_mail_com).
        key: Option<String>,
        #[arg(long)]
        email: String,
        /// The IMAP server. Default: found from the address, in the
        /// database of mail providers (Thunderbird's and ours), else from
        /// the MX servers of its domain.
        #[arg(long)]
        host: Option<String>,
        /// With --host; default 993.
        #[arg(long, requires = "host")]
        port: Option<u16>,
        /// With --host: tls (default), starttls, or none (loopback only).
        #[arg(long, requires = "host")]
        security: Option<String>,
        /// With --host. Default: the email address.
        #[arg(long, requires = "host")]
        username: Option<String>,
        /// With --host: an authority to trust besides the system's
        /// (self-hosted server).
        #[arg(long, requires = "host")]
        ca_file: Option<PathBuf>,
        /// Do not look for an SMTP server: the account cannot send until
        /// [smtp] is added to accounts.toml.
        #[arg(long)]
        no_smtp: bool,
        #[arg(long)]
        display_name: Option<String>,
        /// The folder of a role, e.g. `--role sent="Sent Items"`; repeat it.
        /// Without: the proposal, else a new folder.
        #[arg(long = "role", value_name = "ROLE=FOLDER")]
        roles: Vec<String>,
        /// Archive the read mails of the Inbox received before this day
        /// (YYYY-MM-DD). Default when the Inbox is large: all of them.
        #[arg(long)]
        archive_read_before: Option<String>,
        /// Archive the unread mails received before this day. Default when
        /// the Inbox is large: those older than 30 days.
        #[arg(long)]
        archive_unread_before: Option<String>,
        /// Archive nothing: the Inbox stays as it is.
        #[arg(long)]
        no_archive: bool,
        /// Do not accept the people you wrote to.
        #[arg(long)]
        no_correspondents: bool,
        /// Never change anything on this server: no folder created, no
        /// archiving, new mail screened dry (where it would go is recorded,
        /// nothing moves), changes in NiceMailer refused. For a mailbox in
        /// production you want to look at only.
        #[arg(long)]
        read_only: bool,
        /// Do not ask for confirmation.
        #[arg(long)]
        yes: bool,
    },
    /// Change the workflow role of folders, e.g. `todo=Todo archive=Archives`.
    Roles {
        key: String,
        #[arg(required = true, value_name = "ROLE=FOLDER")]
        roles: Vec<String>,
        /// Create on the server the folders named that do not exist.
        #[arg(long)]
        create_missing: bool,
    },
    /// Rename the key of an account in accounts.toml, keeping its mail, its
    /// history and its password.
    Rename { from: String, to: String },
    /// Delete an account: its mail in NiceMailer, its password, its table
    /// of accounts.toml. The mail on the server is not touched.
    Remove {
        key: String,
        /// Do not ask to type the key again.
        #[arg(long)]
        yes: bool,
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
/// `host:port (security)`.
fn server_label(s: &nm_api::ImapParams) -> String {
    format!("{}:{} ({})", s.host, s.port, s.security)
}

/// Where a server comes from, in words.
fn origin_words(c: &nm_api::ServerCandidate, d: &nm_api::DiscoveryReport) -> String {
    let provider = d.by_domain.as_ref().or(d.by_mx.as_ref()).map(|p| p.name.as_str()).unwrap_or("the provider");
    match c.origin.as_str() {
        "given" => "given with --host".into(),
        "provider" => format!("from {provider}"),
        "smtp_name" => "the IMAP host with \"imap\" replaced by \"smtp\"".into(),
        "same_host" => "the IMAP host itself".into(),
        other => other.into(),
    }
}

/// Which database a provider comes from, in words.
fn source_words(p: &nm_api::ProviderMatch) -> &'static str {
    if p.source == "local" { "our own list, data/providers/local" } else { "Thunderbird's database" }
}

/// The steps of `discover_account`, one line each: what was looked up,
/// what was found, and what will be used.
fn discovery_lines(d: &nm_api::DiscoveryReport) -> Vec<String> {
    let mut out = Vec::new();
    match &d.by_domain {
        Some(p) => out.push(format!("{} is listed by {} ({}).", d.domain, p.name, source_words(p))),
        None => out.push(format!("{} is not in the database of mail providers.", d.domain)),
    }
    if d.mx_looked_up {
        match (&d.mx_error, d.mx.is_empty()) {
            (Some(e), _) => out.push(format!("MX of {}: lookup failed ({e}).", d.domain)),
            (None, true) => out.push(format!("MX of {}: none.", d.domain)),
            (None, false) => out.push(format!("MX of {}: {}.", d.domain, d.mx.join(", "))),
        }
        match &d.by_mx {
            Some(p) => out.push(format!(
                "{} is under {}, listed by {} ({}): the domain is hosted there.",
                p.mx_host.as_deref().unwrap_or("?"),
                p.matched,
                p.name,
                source_words(p)
            )),
            None if !d.mx.is_empty() => out.push("No MX server is under a domain of the database.".into()),
            None => {}
        }
    }
    if let Some(p) = d.by_domain.as_ref().or(d.by_mx.as_ref())
        && p.imap_unsupported
    {
        out.push(format!("{} only accepts OAuth2 logins, which NiceMailer does not support yet.", p.name));
    }
    match &d.imap {
        Some(c) => out.push(format!("IMAP: {}, {}.", server_label(&c.server), origin_words(c, d))),
        None => out.push("IMAP: no server found.".into()),
    }
    if !d.smtp.is_empty() {
        out.push("SMTP servers to try, in order:".into());
        for c in &d.smtp {
            out.push(format!("  {}, {}", server_label(&c.server), origin_words(c, d)));
        }
    }
    out
}

/// The key proposed by `discover_account`, and the rule that made it.
fn key_line(d: &nm_api::DiscoveryReport) -> String {
    let why = match (d.key_source.as_str(), &d.domain_account) {
        ("domain", _) => format!("the name of the domain: no account of {} yet", d.domain),
        (_, Some(other)) => format!("the whole address: {} has an account already, {other}", d.domain),
        _ => format!("the whole address: the key {} is another account's", d.domain_key),
    };
    format!("Account key: {} ({why}; give a key after `add` to choose another)", d.key)
}

/// One SMTP attempt of `test_smtp`, in words.
fn smtp_attempt_line(a: &nm_api::SmtpAttempt, key: &str) -> String {
    let what = match a.outcome.as_str() {
        "logged_in" => "logged in with the IMAP password.".to_string(),
        "reachable" => format!(
            "answers. The IMAP password was not tried: this server is not of the IMAP server's organization \
             (save its own password after the creation: nicemailer account password {key} --smtp)."
        ),
        "no_login" => "answers but offers no login: a relay, not a server to send from.".into(),
        _ => format!("failed: {}.", a.detail.as_deref().unwrap_or("?")),
    };
    format!("{}: {what}", server_label(&a.candidate.server))
}

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

/// `nicemailer mailbox …` (`docs/migrations.md`, "Choosing a mailbox").
#[derive(Subcommand)]
enum MailboxAction {
    /// Say what the database of a mailbox folder is: new, current, of an
    /// earlier version (upgradable), or one this version cannot open.
    /// Changes nothing.
    Check {
        /// The mailbox folder (the one holding nicemailer.db).
        dir: PathBuf,
        /// Print JSON (what the app reads).
        #[arg(long)]
        json: bool,
    },
    /// Upgrade the database of a mailbox folder to this version. Cannot be
    /// undone: older versions of NiceMailer will refuse it. No core may be
    /// using the folder.
    Upgrade {
        dir: PathBuf,
    },
    /// Copy the database of a mailbox folder to DEST, consistent even while
    /// the core runs (`docs/backup.md`). Only nicemailer.db is copied: the
    /// bodies and the search index (nicemailer_body.db) are rebuilt from
    /// the raw files, which a file-level copy (rsync) of raw.noindex keeps.
    Backup {
        dir: PathBuf,
        /// The file to write; must not exist.
        dest: PathBuf,
    },
}

/// `nicemailer config …`: works on the file directly, without the core.
#[derive(Subcommand)]
enum ConfigAction {
    /// Check config.toml and the accounts.toml next to it without applying
    /// them: errors (with their file and line) and warnings. Exits with
    /// status 1 if the core would refuse them.
    Check {
        /// config.toml to check (with the accounts.toml next to it); by
        /// default the one the core reads.
        file: Option<PathBuf>,
    },
    /// Print a complete config.toml: every key at its default value, with
    /// what it does in a comment.
    Defaults {
        /// Print the model of accounts.toml instead: an example account,
        /// commented out.
        #[arg(long)]
        accounts: bool,
    },
    /// Print the paths of the files: config.toml ($NICEMAILER_CONFIG, or
    /// ~/.config/nicemailer/config.toml), then accounts.toml, next to it.
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

/// The exit status of a command refused because the core is in
/// maintenance (`mise-a-jour.md`, section 5): 75, `EX_TEMPFAIL` of
/// `sysexits.h`, "try again later", so that a script can tell it from a
/// failure.
const MAINTENANCE_EXIT: u8 = 75;

/// `--wait`, read by [`Client::connect`]. A process-wide switch rather
/// than one more parameter for each of the commands that connect.
///
/// Rust note: an `AtomicBool` can be read and written from anywhere
/// without a lock (and `static` items must be thread-safe).
static WAIT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The core refused the request because it is in maintenance: why, and
/// what to do, for the user.
#[derive(Debug)]
struct InMaintenance(nm_api::Maintenance);

impl std::fmt::Display for InMaintenance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use nm_api::MaintenanceReason as R;
        let m = &self.0;
        let n = |v: Option<i64>| v.unwrap_or(0);
        match m.reason {
            R::Update => write!(f, "NiceMailer is being updated; try again in a moment, or add --wait"),
            R::Migration => write!(
                f,
                "NiceMailer is upgrading its database (data format {} to {}, now at {}); try again in a moment, or add --wait",
                n(m.from),
                n(m.to),
                n(m.at)
            ),
            R::NewerSchema => write!(
                f,
                "this mailbox was written by a newer NiceMailer (data format {}, this one reads {}): install the newer NiceMailer, or choose another mailbox in the app",
                n(m.from),
                n(m.to)
            ),
            R::MigrationFailed => write!(
                f,
                "the upgrade of the database failed, and the database was put back as it was: {}. Details in core.log of the mailbox; the next start of the core tries again",
                m.message.as_deref().unwrap_or("unknown error")
            ),
        }
    }
}

impl std::error::Error for InMaintenance {}

/// The maintenance of a line from the core, if it is a refusal for one:
/// the answer to a request, or the line a connection gets when a
/// maintenance closes it (no id).
fn maintenance_of(error: &nm_api::RpcError) -> Option<InMaintenance> {
    if error.code != nm_api::error_code::MAINTENANCE {
        return None;
    }
    serde_json::from_value(error.data.clone()?).ok().map(InMaintenance)
}

/// Runs the command; the exit status says how it went (`--help`): 75 for
/// a core in maintenance, which a script may want to try again later.
fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            // `{:?}` on an `anyhow::Error` prints it with its causes, as
            // `fn main() -> anyhow::Result<()>` did.
            eprintln!("Error: {e:?}");
            if e.chain().any(|c| c.is::<InMaintenance>()) {
                std::process::ExitCode::from(MAINTENANCE_EXIT)
            } else {
                std::process::ExitCode::FAILURE
            }
        }
    }
}

fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    WAIT.store(cli.wait, std::sync::atomic::Ordering::Relaxed);
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
            let status = client.call(method::IMPORT_STATUS, Value::Null)?;
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
            let result = client.call(method::LIST_FOLDERS, Value::Null)?;
            // `as_array()` returns `Option<&Vec<Value>>`; `into_iter().flatten()`
            // iterates over the elements, or over nothing if it is `None`.
            // `{:>3}` right-aligns on 3 characters, `{:<12}` left-aligns on 12.
            for f in result["items"].as_array().into_iter().flatten() {
                println!(
                    "{:>3}  {:<8} {}  {:<12} {:>6} mails {:>6} unread",
                    f["id"],
                    f["account_name"].as_str().unwrap_or(""),
                    f["letter"].as_str().unwrap_or(" "),
                    f["name"].as_str().unwrap_or(""),
                    f["total_count"],
                    f["unread_count"],
                );
            }
            Ok(())
        }
        Command::Address { addr, target } => {
            let mut client = Client::connect(&data_dir)?;
            let result = client.call(method::SET_ADDRESS_SCREENING, json!({ "addr": addr, "target": decision_target(&target)? }))?;
            print_workflow(&result);
            Ok(())
        }
        Command::Domain { domain, target } => {
            let mut client = Client::connect(&data_dir)?;
            let result =
                client.call(method::SET_DOMAIN_SCREENING, json!({ "domain": domain, "target": decision_target(&target)? }))?;
            print_workflow(&result);
            Ok(())
        }
        Command::Undo { token } => {
            let mut client = Client::connect(&data_dir)?;
            let result = client.call(method::UNDO, json!({ "token": token }))?;
            print_workflow(&result);
            Ok(())
        }
        Command::Config { action } => run_config(action),
        Command::Ai { action } => ai::run(&data_dir, action),
        Command::Mailbox { action } => run_mailbox(action),
        Command::Account { action: AccountAction::Password { key, smtp } } => {
            // Connect first: no point asking for a password the core
            // cannot receive.
            let mut client = Client::connect(&data_dir)?;
            let what = if smtp { "SMTP" } else { "IMAP" };
            let password = read_password(&format!("{what} password of {key}: "))?;
            client.call(method::SET_PASSWORD, json!({ "account": key, "password": password, "smtp": smtp }))?;
            let result = client.call(method::LIST_ACCOUNTS, Value::Null)?;
            let account = result["items"].as_array().into_iter().flatten().find(|a| a["key"] == key.as_str());
            match account.and_then(|a| a["error"].as_str()) {
                Some(e) => println!("Saved, but the account still has an error: {e}"),
                None => println!("Saved in the Keychain."),
            }
            Ok(())
        }
        Command::Account {
            action:
                AccountAction::Add {
                    key,
                    email,
                    host,
                    port,
                    security,
                    username,
                    ca_file,
                    display_name,
                    roles,
                    archive_read_before,
                    archive_unread_before,
                    no_archive,
                    no_correspondents,
                    read_only,
                    no_smtp,
                    yes,
                },
        } => {
            let mut client = Client::connect(&data_dir)?;
            let roles = parse_roles(&roles)?;
            let given = host.map(|host| nm_api::ImapParams {
                host,
                port: port.unwrap_or(993),
                security: security.unwrap_or_else(|| "tls".into()),
                username,
                ca_file: ca_file.map(|p| std::path::absolute(&p).unwrap_or(p).display().to_string()),
            });

            // Where the servers come from, before any password is asked.
            let d: nm_api::DiscoveryReport =
                serde_json::from_value(client.call(method::DISCOVER_ACCOUNT, json!({ "email": email, "imap": given }))?)?;
            println!("Finding the servers of {email}");
            let key = match key {
                Some(k) => k,
                None => {
                    println!("  {}", key_line(&d));
                    d.key.clone()
                }
            };
            let accounts = client.call(method::LIST_ACCOUNTS, Value::Null)?["items"].as_array().cloned().unwrap_or_default();
            if accounts.iter().any(|a| a["key"].as_str() == Some(key.as_str())) {
                bail!("an account {key} exists already: give another key after `add`");
            }
            for line in discovery_lines(&d) {
                println!("  {line}");
            }
            let Some(imap_candidate) = d.imap.clone() else {
                if let Some(p) = d.by_domain.as_ref().or(d.by_mx.as_ref()).filter(|p| p.imap_unsupported) {
                    bail!("{} needs OAuth2, which NiceMailer does not support yet: the account cannot be added", p.name);
                }
                bail!("no IMAP server found for {email}: give it with --host (and --port, --security if needed)");
            };
            let imap = imap_candidate.server.clone();
            let host = imap.host.clone();
            println!();

            let password = read_password(&format!("IMAP password of {email}: "))?;
            eprintln!("Trying {host}…");
            let r: nm_api::TestAccountReport =
                serde_json::from_value(client.call(method::TEST_ACCOUNT, json!({ "email": email, "imap": imap, "password": password }))?)?;
            print_probe(&r.probe);
            println!();
            println!("Inbox: {} mails, {} unread.", r.inbox_messages, r.inbox_unseen);
            let (read_before, unread_before) = if no_archive || read_only {
                (None, None)
            } else if archive_read_before.is_some() || archive_unread_before.is_some() {
                (archive_read_before, archive_unread_before)
            } else if r.offer_archive {
                (Some(r.read_before.clone()), Some(r.unread_before.clone()))
            } else {
                (None, None)
            };
            if read_before.is_some() || unread_before.is_some() {
                let n: nm_api::ArchiveCount = serde_json::from_value(client.call(
                    method::COUNT_ARCHIVE,
                    json!({ "email": email, "imap": imap, "password": password, "uidnext": r.inbox_uidnext,
                            "read_before": read_before, "unread_before": unread_before }),
                )?)?;
                println!(
                    "To archive on the server: {} read mails before {}, {} unread mails before {} (flagged mails stay).",
                    n.read,
                    read_before.as_deref().unwrap_or("-"),
                    n.unread,
                    unread_before.as_deref().unwrap_or("-")
                );
                println!("{} mails will stay in the Inbox.", r.inbox_messages.saturating_sub(n.read + n.unread));
            } else {
                println!("Nothing archived: the Inbox stays as it is.");
            }
            if no_correspondents {
                println!("The people you wrote to ({}) are not accepted: their next mail goes to Quarantine.", r.correspondents);
            } else {
                println!("The people you wrote to ({}) will go straight to the Inbox.", r.correspondents);
            }
            if read_only {
                println!("Read-only: no folder is created and nothing changes on the server; new mail is screened dry.");
            } else {
                println!("The other senders go through the Quarantine with their next mail.");
            }

            // The SMTP server: tried now, so that the summary says exactly
            // what accounts.toml will hold.
            println!();
            let smtp = if read_only {
                println!("SMTP: not looked for, a read-only account does not send.");
                None
            } else if no_smtp {
                println!("SMTP: not looked for (--no-smtp); the account cannot send until [smtp] is added to accounts.toml.");
                None
            } else if d.smtp.is_empty() {
                println!("SMTP: no server to try; the account cannot send until [smtp] is added to accounts.toml.");
                None
            } else {
                println!("Trying the SMTP servers, in order (the first that works is kept):");
                eprintln!("  …");
                let t: nm_api::SmtpTestReport = serde_json::from_value(client.call(
                    method::TEST_SMTP,
                    json!({ "email": email, "imap": imap, "password": password, "candidates": d.smtp }),
                )?)?;
                for (i, a) in t.attempts.iter().enumerate() {
                    println!("  {}{}", smtp_attempt_line(a, &key), if t.chosen == Some(i) { " Kept." } else { "" });
                }
                let left = d.smtp.len() - t.attempts.len();
                if t.chosen.is_some() && left > 0 {
                    println!("  ({left} other candidate{} not tried.)", if left > 1 { "s" } else { "" });
                }
                match t.chosen.map(|i| &t.attempts[i]) {
                    Some(a) => {
                        println!(
                            "SMTP: {} will be written to [accounts.{key}.smtp]{}.",
                            server_label(&a.candidate.server),
                            if a.outcome == "logged_in" { ", sending with the IMAP password" } else { "" }
                        );
                        // Second: whether it needs its own password.
                        Some((a.candidate.server.clone(), a.outcome == "reachable"))
                    }
                    None => {
                        println!("SMTP: no server worked; the account is created without [smtp] and cannot send until one is added to accounts.toml.");
                        None
                    }
                }
            };
            if !yes && !confirm("Create the account? [y/N] ")? {
                bail!("nothing created");
            }
            let res: nm_api::AddAccountResult = serde_json::from_value(client.call(
                method::ADD_ACCOUNT,
                json!({ "key": key, "email": email, "display_name": display_name, "imap": imap, "password": password,
                        "roles": roles, "uidnext": r.inbox_uidnext, "archive_read_before": read_before,
                        "archive_unread_before": unread_before, "correspondents": !no_correspondents, "read_only": read_only,
                        "smtp": smtp.as_ref().map(|s| &s.0) }),
            )?)?;
            if !res.created.is_empty() {
                println!("Folders created on the server: {}", res.created.join(", "));
            }
            println!("Account {key} created; {} correspondents accepted.", res.correspondents);
            if smtp.as_ref().is_some_and(|s| s.1) {
                println!("Save the password of its SMTP server with: nicemailer account password {key} --smtp");
            }
            println!("Follow the import with: nicemailer sync status {key}");
            Ok(())
        }
        Command::Account { action: AccountAction::Roles { key, roles, create_missing } } => {
            let mut client = Client::connect(&data_dir)?;
            let roles = parse_roles(&roles)?;
            let created = client.call(method::SET_FOLDER_ROLES, json!({ "account": key, "roles": roles, "create_missing": create_missing }))?;
            for c in created.as_array().into_iter().flatten() {
                println!("Created on the server: {}", c.as_str().unwrap_or_default());
            }
            println!("Roles set.");
            Ok(())
        }
        Command::Account { action: AccountAction::Rename { from, to } } => {
            let mut client = Client::connect(&data_dir)?;
            client.call(method::RENAME_ACCOUNT, json!({ "from": from, "to": to }))?;
            println!("Account {from} is now {to}.");
            Ok(())
        }
        Command::Account { action: AccountAction::Remove { key, yes } } => {
            let mut client = Client::connect(&data_dir)?;
            if !yes {
                eprint!("This deletes the mail of {key} in NiceMailer (not on the server) and its password. Type {key} to confirm: ");
                std::io::stderr().flush()?;
                let mut line = String::new();
                std::io::stdin().read_line(&mut line)?;
                if line.trim() != key {
                    bail!("nothing deleted");
                }
            }
            client.call(method::REMOVE_ACCOUNT, json!({ "account": key }))?;
            println!("Account {key} deleted.");
            Ok(())
        }
        Command::Account { action: AccountAction::Order { keys } } => {
            let mut client = Client::connect(&data_dir)?;
            let items = client.call(method::LIST_ACCOUNTS, Value::Null)?["items"].as_array().cloned().unwrap_or_default();
            let mut ids = Vec::new();
            for k in &keys {
                match items.iter().find(|a| a["key"].as_str() == Some(k.as_str())) {
                    Some(a) => ids.push(a["id"].clone()),
                    None => bail!("no account {k} (see `nicemailer accounts`)"),
                }
            }
            client.call(method::SET_ACCOUNT_ORDER, json!({ "account_ids": ids }))?;
            for (i, k) in keys.iter().enumerate() {
                println!("{}. {k}", i + 1);
            }
            Ok(())
        }
        Command::Raw { action: RawAction::Gc } => {
            let mut client = Client::connect(&data_dir)?;
            let r: nm_api::RawGcResult = serde_json::from_value(client.call(method::RAW_GC, Value::Null)?)?;
            println!(
                "{} raw files no mail referenced, {} temporary files, {} orphans, {} attachment folders deleted.",
                r.blobs, r.temporary, r.orphans, r.attachment_dirs
            );
            Ok(())
        }
        Command::Sync { action } => {
            let mut client = Client::connect(&data_dir)?;
            match action {
                SyncAction::Status { key, json } => {
                    let r = client.call(method::SYNC_STATUS, json!({ "account": key }))?;
                    if json {
                        println!("{}", serde_json::to_string_pretty(&r)?);
                    } else {
                        print_sync_status(&serde_json::from_value(r)?);
                    }
                }
                SyncAction::Now { key } => {
                    client.call(method::SYNC_NOW, json!({ "account": key }))?;
                    println!("Synchronization asked; follow it with: nicemailer sync status");
                }
                SyncAction::Verify { key, skip_hash, repair, json } => {
                    let r = client.call(method::SYNC_VERIFY, json!({ "account": key, "skip_hash": skip_hash, "repair": repair }))?;
                    let report: nm_api::SyncVerifyReport = serde_json::from_value(r.clone())?;
                    if json {
                        println!("{}", serde_json::to_string_pretty(&r)?);
                    } else {
                        print_verify(&report);
                    }
                    if report.repaired > 0 {
                        println!("{} mails set to download again (damaged raw files set aside).", report.repaired);
                    }
                    if !report.ok {
                        std::process::exit(1);
                    }
                }
            }
            Ok(())
        }
        Command::Imap { action: ImapAction::Probe { key, json } } => {
            let mut client = Client::connect(&data_dir)?;
            let report = client.call(method::PROBE_ACCOUNT, json!({ "account": key }))?;
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
            let result = client.call(method::LIST_ACCOUNTS, Value::Null)?;
            let items = result["items"].as_array().cloned().unwrap_or_default();
            if items.is_empty() {
                println!(
                    "No account. Add one with `nicemailer account add`, or declare one in {}.",
                    nm_engine::config::accounts_path(&nm_engine::config::default_path()).display()
                );
            }
            for a in items {
                let state = match (a["enabled"].as_bool(), a["error"].as_str()) {
                    (Some(false), _) => "disabled (no longer in accounts.toml, mail kept)".to_string(),
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
            let pos = client.call(method::SUBSCRIBE, json!({}))?;
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
/// `role=folder` pairs, the role a workflow role (`sent`, `todo`…).
fn parse_roles(pairs: &[String]) -> anyhow::Result<serde_json::Map<String, Value>> {
    let mut out = serde_json::Map::new();
    for p in pairs {
        let Some((role, folder)) = p.split_once('=') else { bail!("{p:?}: ROLE=FOLDER expected") };
        let role = role.trim().to_lowercase();
        if nm_core::FolderRole::parse(&role).is_none() {
            bail!("{role:?} is not a workflow role (inbox, todo, waiting, archive, sent, drafts, newsletter, paperwork, quarantine, deleted, junk, banned)");
        }
        out.insert(role, Value::String(folder.trim().to_string()));
    }
    Ok(out)
}

/// Asks a yes or no question on the terminal; `false` without one.
fn confirm(question: &str) -> anyhow::Result<bool> {
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() {
        bail!("no terminal to confirm: add --yes");
    }
    eprint!("{question}");
    std::io::stderr().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(matches!(line.trim().to_lowercase().as_str(), "y" | "yes"))
}

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
        ConfigAction::Path => {
            let path = config::default_path();
            println!("{}", path.display());
            println!("{}", config::accounts_path(&path).display());
        }
        ConfigAction::Defaults { accounts: false } => print!("{}", config::defaults_file()),
        ConfigAction::Defaults { accounts: true } => print!("{}", config::accounts_template()),
        ConfigAction::Check { file } => {
            let path = file.unwrap_or_else(config::default_path);
            let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
            // Each problem names its file; the directory says where both are.
            let (settings, accounts) = match config::read_both(&path) {
                Ok(texts) => texts,
                Err(p) => bail!("{}: {p}", dir.display()),
            };
            for (text, name) in [(&settings, path.display().to_string()), (&accounts, config::accounts_path(&path).display().to_string())] {
                if text.is_none() {
                    println!("{name}: no file (the core creates it at its start)");
                }
            }
            match config::parse(settings.as_deref().unwrap_or(""), accounts.as_deref().unwrap_or("")) {
                Ok(parsed) => {
                    for w in &parsed.warnings {
                        println!("{}: warning: {w}", dir.display());
                    }
                    let n = parsed.config.accounts.len();
                    println!("{}: valid, {n} account{}", dir.display(), if n == 1 { "" } else { "s" });
                }
                // `std::process::exit` rather than an error: the message is
                // the whole output, without anyhow's "Error:" prefix, and
                // the status says "refused" to scripts.
                Err(p) => {
                    println!("{}: refused: {p}", dir.display());
                    std::process::exit(1);
                }
            }
        }
    }
    Ok(())
}

/// What `mailbox check --json` prints: the folder, its database's state,
/// and whether a core holds the folder.
fn mailbox_report(dir: &Path) -> Value {
    let state = nm_store::inspect::inspect(&dir.join(paths::DATABASE));
    let mut report = serde_json::to_value(&state).expect("plain enum");
    report["path"] = json!(dir.display().to_string());
    report["in_use"] = json!(in_use(dir));
    report
}

/// Whether a core holds the single-instance lock of `dir`. The lock file is
/// not created if missing: checking changes nothing.
fn in_use(dir: &Path) -> bool {
    match fs::OpenOptions::new().write(true).open(dir.join("lock")) {
        // `try_lock` fails if another process holds it; ours is released
        // when `f` is dropped, at the end of this function.
        Ok(f) => f.try_lock().is_err(),
        Err(_) => false,
    }
}

/// `nicemailer mailbox check | upgrade`.
fn run_mailbox(action: MailboxAction) -> anyhow::Result<()> {
    use nm_store::inspect::DatabaseState as S;
    match action {
        MailboxAction::Check { dir, json } => {
            let report = mailbox_report(&dir);
            if json {
                println!("{report}");
                return Ok(());
            }
            let state = nm_store::inspect::inspect(&dir.join(paths::DATABASE));
            let what = match state {
                S::New => "no database yet: a new mailbox".to_string(),
                S::Current => format!("current (schema version {})", nm_store::SCHEMA_VERSION),
                S::Upgradable { from, to } => {
                    format!("schema version {from}, upgradable to {to}: nicemailer mailbox upgrade {}", dir.display())
                }
                S::Outdated { missing } => format!("an early development database ({missing} missing): it cannot be upgraded"),
                S::TooNew { found, expected } => {
                    format!("schema version {found}, written by a newer NiceMailer (this one reads {expected})")
                }
                S::Unreadable { message } => format!("unreadable: {message}"),
            };
            let busy = if report["in_use"] == true { " (in use by a core)" } else { "" };
            println!("{}: {what}{busy}", dir.display());
            Ok(())
        }
        MailboxAction::Upgrade { dir } => {
            let db = dir.join(paths::DATABASE);
            let from = match nm_store::inspect::inspect(&db) {
                S::Upgradable { from, .. } => from,
                S::Current => {
                    println!("{}: already current, nothing to do", dir.display());
                    return Ok(());
                }
                S::New => bail!("{}: no database to upgrade", dir.display()),
                S::Outdated { missing } => {
                    bail!("{}: an early development database ({missing} missing): it cannot be upgraded", dir.display())
                }
                S::TooNew { found, expected } => {
                    bail!("{}: schema version {found} is newer than this NiceMailer's ({expected})", dir.display())
                }
                S::Unreadable { message } => bail!("{}: {message}", dir.display()),
            };
            // The core is the only owner of a database it uses: take its lock
            // for the time of the upgrade, as `seed` does.
            let lock = File::create(dir.join("lock"))?;
            if lock.try_lock().is_err() {
                bail!("a core is using {}: choose this mailbox in the app, or stop that core first", dir.display());
            }
            let start = Instant::now();
            // As the core does at its start (`mise-a-jour.md`, section 4):
            // the database is copied first (`nicemailer.db.bak-v<from>`),
            // migrated one version per transaction, and put back as it was
            // if a version fails (`nm_store::upgrade`).
            let (store, _) = nm_store::upgrade::migrate(&db, from, &|v| eprintln!("  version {v} reached"))
                .with_context(|| format!("upgrading {}", db.display()))?;
            drop(store);
            println!(
                "{}: upgraded to schema version {} in {:.1} s",
                dir.display(),
                nm_store::SCHEMA_VERSION,
                start.elapsed().as_secs_f64()
            );
            Ok(())
        }
        MailboxAction::Backup { dir, dest } => {
            let db = dir.join(paths::DATABASE);
            if !db.exists() {
                bail!("{}: no database in this folder", dir.display());
            }
            if dest.exists() {
                bail!("{}: already exists, give a new file", dest.display());
            }
            let start = Instant::now();
            // `VACUUM INTO` writes a consistent copy from one read
            // transaction: the core can go on writing meanwhile (WAL), and
            // the copy is compact. Copying the file itself would miss the
            // writes still in `nicemailer.db-wal`.
            let conn = nm_store::rusqlite::Connection::open_with_flags(&db, nm_store::rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)?;
            conn.execute_batch("PRAGMA busy_timeout = 5000;")?;
            conn.execute("VACUUM main INTO ?1", [dest.to_string_lossy().as_ref()])?;
            let size = fs::metadata(&dest)?.len();
            println!(
                "{} -> {} ({:.1} MB) in {:.1} s. Also copy {} (rsync); nicemailer_body.db is rebuilt from it.",
                db.display(),
                dest.display(),
                size as f64 / 1e6,
                start.elapsed().as_secs_f64(),
                dir.join("raw.noindex").display()
            );
            Ok(())
        }
    }
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
        // The body file goes too (`nicemailer_body.db`): it holds the
        // bodies of the mails of the old database.
        for file in [db.clone(), nm_store::body_path(&db)] {
            for suffix in ["", "-wal", "-shm"] {
                let p = PathBuf::from(format!("{}{suffix}", file.display()));
                if p.exists() {
                    fs::remove_file(&p)?;
                }
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
    client.call(method::SUBSCRIBE, json!({}))?;
    let started = client.call(
        method::IMPORT_MBOX,
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
                let status = client.call(method::IMPORT_STATUS, Value::Null)?;
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
    /// Connects to the core and performs the `hello` handshake. A core in
    /// maintenance refuses it ([`InMaintenance`]); with `--wait`, an update
    /// or a migration is waited for, trying again every second
    /// (`mise-a-jour.md`, section 5), and a maintenance that only the user
    /// can end fails at once.
    fn connect(data_dir: &std::path::Path) -> anyhow::Result<Client> {
        let mut told = false;
        loop {
            match Self::connect_once(data_dir) {
                Err(e) if WAIT.load(std::sync::atomic::Ordering::Relaxed) => {
                    let Some(m) = e.downcast_ref::<InMaintenance>().filter(|m| m.0.is_transient()) else { return Err(e) };
                    if !told {
                        told = true;
                        eprintln!("Waiting: {}…", m.to_string().split(';').next().unwrap_or_default());
                    }
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
                other => return other,
            }
        }
    }

    fn connect_once(data_dir: &std::path::Path) -> anyhow::Result<Client> {
        let path = data_dir.join(paths::SOCKET);
        let stream = UnixStream::connect(&path)
            .with_context(|| format!("cannot connect to {}: is the core running? (make core)", path.display()))?;
        let reader = BufReader::new(stream.try_clone()?);
        let mut client = Client { stream, reader, next_id: 1 };
        client.call(method::HELLO, json!({"protocol_version": PROTOCOL_VERSION, "client": "cli"}))?;
        Ok(client)
    }

    /// The error of a response, turned into an error of the command: a
    /// maintenance as [`InMaintenance`], anything else with its code.
    fn failure(e: nm_api::RpcError) -> anyhow::Error {
        match maintenance_of(&e) {
            Some(m) => m.into(),
            None => anyhow::anyhow!("error {}: {}", e.code, e.message),
        }
    }

    /// A line without an id that says the core closed the connection for a
    /// maintenance (`update.rs` of the core): that error, else None.
    fn closing(v: &Value) -> Option<anyhow::Error> {
        if !v.get("id").is_some_and(Value::is_null) {
            return None;
        }
        let e: nm_api::RpcError = serde_json::from_value(v.get("error")?.clone()).ok()?;
        maintenance_of(&e).map(Into::into)
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

    /// Like [`Client::call`], but keeps the events received while waiting
    /// for the response instead of dropping them: an event caused by the
    /// request (the first fragment of an AI run) may come before it.
    fn call_keeping_events(&mut self, method: &str, params: Value) -> anyhow::Result<(Value, Vec<Value>)> {
        let id = self.next_id;
        self.next_id += 1;
        writeln!(self.stream, "{}", json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))?;
        let mut events = Vec::new();
        loop {
            let v: Value = serde_json::from_str(&self.read_line()?)?;
            if let Some(e) = Self::closing(&v) {
                return Err(e);
            }
            match v.get("id").and_then(Value::as_u64) {
                Some(i) if i == id => {
                    let r: Response = serde_json::from_value(v)?;
                    if let Some(e) = r.error {
                        return Err(Self::failure(e));
                    }
                    return Ok((r.result.unwrap_or(Value::Null), events));
                }
                Some(_) => {}
                None => events.push(v),
            }
        }
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
            if let Some(e) = Self::closing(&v) {
                return Err(e);
            }
            // Skip lines that are not the response to this request: these are
            // event notifications, which can arrive at any time after
            // `subscribe`.
            if v.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            let r: Response = serde_json::from_value(v)?;
            if let Some(e) = r.error {
                return Err(Self::failure(e));
            }
            return Ok(r.result.unwrap_or(Value::Null));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(host: &str, port: u16, security: &str) -> nm_api::ImapParams {
        nm_api::ImapParams { host: host.into(), port, security: security.into(), username: None, ca_file: None }
    }

    fn candidate(host: &str, port: u16, security: &str, origin: &str) -> nm_api::ServerCandidate {
        nm_api::ServerCandidate { server: server(host, port, security), origin: origin.into() }
    }

    fn google(mx_host: Option<&str>) -> nm_api::ProviderMatch {
        nm_api::ProviderMatch {
            id: "googlemail.com".into(),
            name: "Google Mail".into(),
            source: "thunderbird".into(),
            matched: if mx_host.is_some() { "google.com".into() } else { "gmail.com".into() },
            mx_host: mx_host.map(Into::into),
            imap_unsupported: false,
        }
    }

    fn report() -> nm_api::DiscoveryReport {
        nm_api::DiscoveryReport {
            key: "my-company".into(),
            key_source: "domain".into(),
            domain_key: "my-company".into(),
            domain_account: None,
            domain: "my-company.fr".into(),
            by_domain: None,
            mx: vec!["aspmx.l.google.com".into()],
            mx_looked_up: true,
            mx_error: None,
            by_mx: Some(google(Some("aspmx.l.google.com"))),
            imap: Some(candidate("imap.gmail.com", 993, "tls", "provider")),
            smtp: vec![candidate("smtp.gmail.com", 465, "tls", "provider")],
        }
    }

    /// Every step is said: the domain not listed, the MX, the provider
    /// found through it and from which database, the servers and where
    /// they come from.
    #[test]
    fn the_discovery_says_each_step() {
        assert_eq!(
            discovery_lines(&report()),
            [
                "my-company.fr is not in the database of mail providers.",
                "MX of my-company.fr: aspmx.l.google.com.",
                "aspmx.l.google.com is under google.com, listed by Google Mail (Thunderbird's database): the domain is hosted there.",
                "IMAP: imap.gmail.com:993 (tls), from Google Mail.",
                "SMTP servers to try, in order:",
                "  smtp.gmail.com:465 (tls), from Google Mail",
            ]
        );
    }

    #[test]
    fn a_listed_domain_skips_the_mx_and_a_failed_lookup_is_said() {
        let mut d = report();
        d.by_domain = Some(google(None));
        d.by_mx = None;
        d.mx = vec![];
        d.mx_looked_up = false;
        let lines = discovery_lines(&d);
        assert_eq!(lines[0], "my-company.fr is listed by Google Mail (Thunderbird's database).");
        assert!(lines.iter().all(|l| !l.starts_with("MX")), "{lines:?}");

        let mut d = report();
        d.by_mx = None;
        d.mx = vec![];
        d.mx_error = Some("the domain my-company.fr does not exist".into());
        d.imap = Some(candidate("imap.my-company.fr", 993, "tls", "given"));
        d.smtp = vec![candidate("smtp.my-company.fr", 465, "tls", "smtp_name"), candidate("imap.my-company.fr", 587, "starttls", "same_host")];
        let lines = discovery_lines(&d);
        assert!(lines.contains(&"MX of my-company.fr: lookup failed (the domain my-company.fr does not exist).".to_string()));
        assert!(lines.contains(&"IMAP: imap.my-company.fr:993 (tls), given with --host.".to_string()));
        assert!(lines.contains(&"  smtp.my-company.fr:465 (tls), the IMAP host with \"imap\" replaced by \"smtp\"".to_string()));
        assert!(lines.contains(&"  imap.my-company.fr:587 (starttls), the IMAP host itself".to_string()));
    }

    #[test]
    fn the_key_says_its_rule() {
        let mut d = report();
        assert_eq!(
            key_line(&d),
            "Account key: my-company (the name of the domain: no account of my-company.fr yet; give a key after `add` to choose another)"
        );
        d.key = "me_my_company_fr".into();
        d.key_source = "address".into();
        d.domain_account = Some("work".into());
        assert!(key_line(&d).contains("(the whole address: my-company.fr has an account already, work;"));
        d.domain_account = None;
        assert!(key_line(&d).contains("(the whole address: the key my-company is another account's;"));
    }

    #[test]
    fn each_smtp_outcome_is_explained() {
        let a = |outcome: &str, detail: Option<&str>| nm_api::SmtpAttempt {
            candidate: candidate("smtp.x.fr", 465, "tls", "provider"),
            outcome: outcome.into(),
            detail: detail.map(Into::into),
        };
        assert_eq!(smtp_attempt_line(&a("logged_in", None), "k"), "smtp.x.fr:465 (tls): logged in with the IMAP password.");
        assert!(smtp_attempt_line(&a("reachable", None), "k").contains("nicemailer account password k --smtp"));
        assert!(smtp_attempt_line(&a("no_login", None), "k").contains("offers no login"));
        assert_eq!(smtp_attempt_line(&a("failed", Some("cannot connect to smtp.x.fr")), "k"), "smtp.x.fr:465 (tls): failed: cannot connect to smtp.x.fr.");
    }
}
