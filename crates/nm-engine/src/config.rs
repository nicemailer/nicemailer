//! `config.toml` and `accounts.toml`: reading and validating the settings
//! files (`specifications/configuration.md`).
//!
//! The files hold the user's intentions: intervals and display choices in
//! `config.toml`, servers and identities in `accounts.toml`, next to it.
//! Two files rather than one (`configuration.md`, section 3): the general
//! settings can be shared between Macs, or published in a dotfiles
//! repository, without the addresses and servers of the accounts, which
//! are personal and change from one Mac to the next.
//!
//! A key absent from a file takes its default value, and a missing file is
//! an empty file. The core creates both files at its start when they are
//! missing (`config_write::create_missing`), from [`config_template`] and
//! [`accounts_template`]: every key is listed there, commented out, so the
//! user sees what can be changed without any value being pinned.
//!
//! # Why a hand-written reader
//!
//! The usual way to read TOML in Rust is `serde`: derive `Deserialize` on
//! the structs below and let the library fill them. It is not used here
//! because the specification asks for three things serde does not give:
//!
//! - **the line of every error**, including the errors of *our* rules (a
//!   port out of range, a missing address), so the app can point at it;
//! - **unknown keys as warnings**, not errors, so a file written by a newer
//!   NiceMailer stays readable by an older one;
//! - **per-account overrides** of general settings (`[accounts.perso.sync]`),
//!   which start from the general values.
//!
//! So the file is parsed by `toml_edit` into a tree that keeps the position
//! of every key and value, and the functions below walk it: each reads the
//! keys it knows, checks them, and reports the keys left over.
//!
//! # All or nothing
//!
//! [`parse`] reads the two files together and returns either a complete
//! [`Config`] or the first problem. An invalid file is refused whole, and
//! with it the other one: applying part of the settings would produce a
//! state nobody wrote (`configuration.md`, section 4). The two are read
//! together anyway, because an account's `[accounts.<key>.sync]` starts
//! from the `[sync]` of `config.toml`.

use std::collections::BTreeMap;
use std::fmt;
use std::ops::{Range, RangeInclusive};
use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};
use toml_edit::{Item, TableLike};

/// Version of the file format read by this code: the `version` key, the
/// same for both files.
pub const FORMAT_VERSION: i64 = 1;

/// Name of the file of general settings.
pub const CONFIG_FILE: &str = "config.toml";

/// Name of the file of accounts, always next to [`CONFIG_FILE`].
pub const ACCOUNTS_FILE: &str = "accounts.toml";

// ---------------------------------------------------------------------------
// The settings
// ---------------------------------------------------------------------------

/// Every setting of the file, with the defaults filled in.
///
/// `Serialize` is used only to list the keys that changed between two
/// versions of the file ([`changed_keys`]); the file itself is never
/// written from this struct (writing arrives with `set_config`, through
/// `toml_edit`, to keep the user's comments).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Config {
    pub version: i64,
    pub core: CoreSettings,
    pub sync: SyncSettings,
    pub config_sync: ConfigSyncSettings,
    pub events: EventSettings,
    pub import: ImportSettings,
    pub raw: RawSettings,
    pub ui: UiSettings,
    pub audit: AuditSettings,
    pub compose: ComposeSettings,
    /// IMAP accounts, by key (`[accounts.<key>]` of `accounts.toml`).
    ///
    /// Rust note: a `BTreeMap` is a map sorted by key, so accounts are
    /// always listed in the same order, whatever their order in the file.
    pub accounts: BTreeMap<String, AccountConfig>,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            version: FORMAT_VERSION,
            core: CoreSettings::default(),
            sync: SyncSettings::default(),
            config_sync: ConfigSyncSettings::default(),
            events: EventSettings::default(),
            import: ImportSettings::default(),
            raw: RawSettings::default(),
            ui: UiSettings::default(),
            audit: AuditSettings::default(),
            compose: ComposeSettings::default(),
            accounts: BTreeMap::new(),
        }
    }
}

/// `[core]` (`architecture.md`, section 6; `mise-a-jour.md`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CoreSettings {
    pub background_sorting: bool,
    pub idle_exit_secs: u64,
    pub update_timeout_secs: u64,
}

impl Default for CoreSettings {
    fn default() -> Self {
        CoreSettings { background_sorting: true, idle_exit_secs: 60, update_timeout_secs: 900 }
    }
}

/// `[sync]`, which an account may override (`[accounts.<key>.sync]`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SyncSettings {
    /// Interval between two checks of the folders other than the Inbox,
    /// which is followed live with IDLE (`dossiers.md`).
    pub poll_interval_secs: u64,
    /// IMAP connections per account (`plan-imap.md`, phase 4): 2, one for
    /// the folders and IDLE, one for downloading bodies; 1, both on the
    /// same connection, for a server that allows few.
    pub max_connections: u64,
}

impl Default for SyncSettings {
    fn default() -> Self {
        SyncSettings { poll_interval_secs: 300, max_connections: 2 }
    }
}

/// `[config_sync]` (`synchronisation-configuration.md`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConfigSyncSettings {
    pub push_delay_secs: u64,
}

impl Default for ConfigSyncSettings {
    fn default() -> Self {
        ConfigSyncSettings { push_delay_secs: 5 }
    }
}

/// `[events]` (`evenements.md`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EventSettings {
    pub replay_buffer: u64,
    pub client_queue: u64,
}

impl Default for EventSettings {
    fn default() -> Self {
        EventSettings { replay_buffer: 10_000, client_queue: 1_000 }
    }
}

/// `[import]` (`import-initial.md`), which an account may override.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ImportSettings {
    pub threshold_total: u64,
    pub threshold_unseen: u64,
    pub unseen_default_age_days: u64,
    pub batch_size: u64,
}

impl Default for ImportSettings {
    fn default() -> Self {
        ImportSettings { threshold_total: 100, threshold_unseen: 50, unseen_default_age_days: 30, batch_size: 1_000 }
    }
}

/// `[raw]` (`stockage-brut.md`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RawSettings {
    /// Root of the raw store; `None` means `<data_dir>/raw.noindex`.
    pub dir: Option<PathBuf>,
    /// 9: measured on 300 MB of a real mailbox (`nm-store`, `raw.rs`).
    pub zstd_level: u64,
    pub fanout: u64,
    pub gc_grace_secs: u64,
    pub verify_on_read: bool,
}

impl Default for RawSettings {
    fn default() -> Self {
        RawSettings { dir: None, zstd_level: 9, fanout: 1, gc_grace_secs: 86_400, verify_on_read: true }
    }
}

/// `[ui]` (`tracabilite.md`; `docs/geek-ui.md`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UiSettings {
    pub view_threshold_ms: u64,
    /// The title bar of the window (its title and the red, orange and
    /// green buttons): hidden by default, the content up to the top edge.
    pub title_bar: TitleBar,
}

impl Default for UiSettings {
    fn default() -> Self {
        UiSettings { view_threshold_ms: 2_000, title_bar: TitleBar::Hidden }
    }
}

/// `ui.title_bar`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TitleBar {
    Hidden,
    Shown,
}

impl TitleBar {
    pub fn as_str(self) -> &'static str {
        match self {
            TitleBar::Hidden => "hidden",
            TitleBar::Shown => "shown",
        }
    }
}

/// `[audit]` (`tracabilite.md`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AuditSettings {
    pub retention_days: u64,
}

impl Default for AuditSettings {
    fn default() -> Self {
        AuditSettings { retention_days: 365 }
    }
}

/// `[compose]` (`reponse.md`, section 12).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ComposeSettings {
    /// Seconds a mail waits after `⌘↩` before it leaves, during which `z`
    /// cancels it; 0: at once, no cancelling.
    pub undo_send_seconds: u64,
}

impl Default for ComposeSettings {
    fn default() -> Self {
        ComposeSettings { undo_send_seconds: nm_core::compose::UNDO_SEND_DEFAULT_SECS }
    }
}

/// `save_sent` of an account (`reponse.md`, section 9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SaveSent {
    /// Copy to Sent, unless the provider does it itself (Gmail, Fastmail).
    Auto,
    Always,
    Never,
}

impl SaveSent {
    pub fn rule(self) -> nm_core::compose::SaveSent {
        match self {
            SaveSent::Auto => nm_core::compose::SaveSent::Auto,
            SaveSent::Always => nm_core::compose::SaveSent::Always,
            SaveSent::Never => nm_core::compose::SaveSent::Never,
        }
    }
}

/// One `[accounts.<key>]` table.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AccountConfig {
    /// Main address of the account.
    pub email: String,
    pub display_name: Option<String>,
    pub auth: AuthMethod,
    /// Whether this account carries the synchronized configuration
    /// (`synchronisation-configuration.md`; not implemented yet).
    pub config_sync: bool,
    /// Never change anything on this account's server: the mail is
    /// synchronized and screened "dry" (where each mail would go is
    /// journaled, nothing moves), and the commands that change mail are
    /// refused (`docs/sync.md`, "Read-only accounts").
    pub read_only: bool,
    /// Trusted `authserv-id` of `Authentication-Results`, replacing the
    /// detected one (`configuration.md`, section 6).
    pub authserv_id: Option<String>,
    pub imap: ServerConfig,
    /// The server mail is sent through; without it, the account cannot
    /// send (`reponse.md`, section 3).
    pub smtp: Option<ServerConfig>,
    /// Whether NiceMailer copies a sent mail to Sent (`reponse.md`,
    /// section 9).
    pub save_sent: SaveSent,
    /// Other sending addresses of the account.
    pub identities: Vec<Identity>,
    /// General `[sync]` settings, overridden by `[accounts.<key>.sync]`.
    pub sync: SyncSettings,
    /// General `[import]` settings, overridden by `[accounts.<key>.import]`.
    pub import: ImportSettings,
}

/// `auth`: how the core proves who it is to the servers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMethod {
    Password,
    /// Accepted in the file; not supported yet (`plan-imap.md`, section 6).
    Oauth2,
}

/// `security` of a server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Security {
    /// TLS from the first byte (IMAP 993, SMTP 465). The default.
    Tls,
    /// Plain connection upgraded to TLS by `STARTTLS` (IMAP 143, SMTP 587).
    Starttls,
    /// No encryption at all. Accepted only for a server on this Mac
    /// (`localhost`): elsewhere it would send the password in clear text.
    None,
}

/// `[accounts.<key>.imap]` or `[accounts.<key>.smtp]`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub security: Security,
    /// Login name; the account's address by default.
    pub username: String,
    /// Extra certificate authority (PEM file) trusted for this server, on
    /// top of the system's: for a self-hosted server, and for the test
    /// servers (`dev/dovecot`). Certificate checks are never disabled.
    pub ca_file: Option<PathBuf>,
}

/// One entry of `[[accounts.<key>.identities]]`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Identity {
    pub email: String,
    pub name: Option<String>,
}

/// A secret (a password) that never shows in logs or debug output.
///
/// Rust note: this is a *newtype*, a struct with a single field. It costs
/// nothing at run time, but it is a different type from `String`, so the
/// compiler refuses to print it by mistake: its `Debug` implementation
/// below prints a placeholder, and it has no `Display` at all. Reading the
/// secret takes an explicit call to [`SecretString::expose`].
#[derive(Clone, PartialEq, Eq)]
pub struct SecretString(String);

impl SecretString {
    pub fn new(s: impl Into<String>) -> SecretString {
        SecretString(s.into())
    }

    /// The secret itself, for the authentication only.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretString(***)")
    }
}

/// Serialized as a short fingerprint, never as the secret: enough to see
/// that the password changed ([`changed_keys`]), useless to an attacker
/// (a SHA-256 truncated to 64 bits of a password still has to be guessed).
impl Serialize for SecretString {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let digest = Sha256::digest(self.0.as_bytes());
        let hex: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
        s.serialize_str(&format!("sha256:{hex}"))
    }
}

// ---------------------------------------------------------------------------
// Problems
// ---------------------------------------------------------------------------

/// An error or a warning about the file, with where it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Problem {
    /// The file concerned: [`CONFIG_FILE`] or [`ACCOUNTS_FILE`]. Needed
    /// since there are two: a line number alone would not say where to
    /// look.
    pub file: &'static str,
    /// Line of the file, from 1; `None` when the problem is not on a line
    /// (the file cannot be read).
    pub line: Option<u32>,
    /// Dotted key concerned, e.g. `accounts.perso.imap.port`.
    pub key: Option<String>,
    pub message: String,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.file)?;
        if let Some(line) = self.line {
            write!(f, ", line {line}")?;
        }
        f.write_str(": ")?;
        if let Some(key) = &self.key {
            write!(f, "{key}: ")?;
        }
        f.write_str(&self.message)
    }
}

/// A valid file: its settings, and the warnings about it.
#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    pub config: Config,
    /// Unknown keys.
    pub warnings: Vec<Problem>,
}

// ---------------------------------------------------------------------------
// Reading the file
// ---------------------------------------------------------------------------

/// Path of `config.toml`: `NICEMAILER_CONFIG` when set, otherwise
/// `~/.config/nicemailer/config.toml` (`configuration.md`, section 3).
pub fn default_path() -> PathBuf {
    if let Some(p) = std::env::var_os("NICEMAILER_CONFIG") {
        return PathBuf::from(p);
    }
    home().join(".config/nicemailer").join(CONFIG_FILE)
}

/// Path of `accounts.toml`: next to `config_path`, whatever its name. One
/// variable (`NICEMAILER_CONFIG`) thus moves both files, and they cannot
/// be separated by mistake (a test core reading the real accounts).
pub fn accounts_path(config_path: &Path) -> PathBuf {
    config_path.with_file_name(ACCOUNTS_FILE)
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

/// Reads one of the files; `file` ([`CONFIG_FILE`] or [`ACCOUNTS_FILE`])
/// names it in the problems. A missing file is `Ok(None)`: it means "no
/// settings", not an error.
pub fn read(path: &Path, file: &'static str) -> Result<Option<String>, Problem> {
    match std::fs::read(path) {
        Ok(bytes) => String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| Problem { file, line: None, key: None, message: "the file is not valid UTF-8".into() }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Problem { file, line: None, key: None, message: format!("cannot read {}: {e}", path.display()) }),
    }
}

/// Reads both files: `config_path`, and `accounts.toml` next to it.
pub fn read_both(config_path: &Path) -> Result<(Option<String>, Option<String>), Problem> {
    Ok((read(config_path, CONFIG_FILE)?, read(&accounts_path(config_path), ACCOUNTS_FILE)?))
}

/// Parses and validates the text of the two files (`""` for a missing
/// one). `config.toml` is read first: its `[sync]` and `[import]` are the
/// base of each account's overrides.
pub fn parse(config_text: &str, accounts_text: &str) -> Result<Parsed, Problem> {
    let doc = document(config_text, CONFIG_FILE)?;
    let mut r = Reader { text: config_text, file: CONFIG_FILE, warnings: Vec::new() };
    let mut config = r.config(doc.as_table())?;
    let mut warnings = r.warnings;

    let doc = document(accounts_text, ACCOUNTS_FILE)?;
    let mut r = Reader { text: accounts_text, file: ACCOUNTS_FILE, warnings: Vec::new() };
    config.accounts = r.accounts_file(doc.as_table(), &config.sync, &config.import)?;
    warnings.append(&mut r.warnings);
    Ok(Parsed { config, warnings })
}

/// Validates `accounts.toml` alone, against the default general settings:
/// what a writer checks before saving it (`config_write`). The bounds of an
/// override do not depend on the general values, so a file valid here is
/// valid with any `config.toml`.
pub fn parse_accounts(text: &str) -> Result<BTreeMap<String, AccountConfig>, Problem> {
    Ok(parse("", text)?.config.accounts)
}

/// The TOML tree of a file, or its syntax error with the line.
fn document<'a>(text: &'a str, file: &'static str) -> Result<toml_edit::Document<&'a str>, Problem> {
    toml_edit::Document::parse(text).map_err(|e| Problem {
        file,
        line: e.span().map(|s| line_of(text, s.start)),
        key: None,
        // The parser's message may span several lines (it quotes the
        // source); the first one says what is wrong.
        message: e.message().lines().next().unwrap_or("invalid TOML").to_string(),
    })
}

/// Line (from 1) of the byte `offset` of `text`.
fn line_of(text: &str, offset: usize) -> u32 {
    let end = offset.min(text.len());
    text.as_bytes()[..end].iter().filter(|&&b| b == b'\n').count() as u32 + 1
}

/// Walks the parsed tree of one file and builds its part of the [`Config`].
struct Reader<'t> {
    text: &'t str,
    /// The file read, named in every problem.
    file: &'static str,
    warnings: Vec<Problem>,
}

/// One table of the file being read: which keys it has, which were read.
///
/// Rust note: `'a` is a *lifetime*: the section borrows the parsed tree,
/// and the compiler checks that the tree outlives it. `&'a dyn TableLike`
/// is a reference to "any type implementing the `TableLike` trait", here
/// either a `[table]` or an inline `{ table }`, which `toml_edit` models as
/// two types with the same interface.
struct Section<'a> {
    table: &'a dyn TableLike,
    /// Dotted path of the table, `""` for the root.
    path: String,
    /// Line of the table's header, for the errors about a missing key.
    line: Option<u32>,
    /// Keys read so far; the others are unknown.
    read: Vec<&'static str>,
}

impl<'a> Section<'a> {
    fn key(&self, key: &str) -> String {
        if self.path.is_empty() { key.to_string() } else { format!("{}.{key}", self.path) }
    }

    /// The item of `key`, marked as known.
    fn get(&mut self, key: &'static str) -> Option<&'a Item> {
        self.read.push(key);
        // Rust note: `self.table` is a `&'a dyn TableLike`, so what `get`
        // returns lives as long as the tree (`'a`), not only as long as
        // this method call.
        let table: &'a dyn TableLike = self.table;
        table.get(key).filter(|i| !i.is_none())
    }
}

/// The `Result` of the reader: a value, or the problem that refuses the file.
type R<T> = Result<T, Problem>;

impl<'t> Reader<'t> {
    fn line(&self, span: Option<Range<usize>>) -> Option<u32> {
        span.map(|s| line_of(self.text, s.start))
    }

    fn problem(&self, item: Option<&Item>, key: String, message: impl Into<String>) -> Problem {
        Problem { file: self.file, line: self.line(item.and_then(|i| i.span())), key: Some(key), message: message.into() }
    }

    /// Error on a missing required key, placed on the table's header.
    fn missing(&self, s: &Section, key: &str) -> Problem {
        Problem { file: self.file, line: s.line, key: Some(s.key(key)), message: "required key missing".into() }
    }

    /// Ends a section: every key not read is reported as unknown.
    fn finish(&mut self, s: Section) {
        for (key, item) in s.table.iter() {
            if !s.read.contains(&key) && !item.is_none() {
                let line = s.table.get_key_value(key).and_then(|(k, _)| k.span()).or_else(|| item.span());
                self.warnings.push(Problem {
                    file: self.file,
                    line: self.line(line),
                    key: Some(s.key(key)),
                    message: "unknown key, ignored".into(),
                });
            }
        }
    }

    /// The sub-table `key` of `s`, if present.
    fn table<'a>(&self, s: &mut Section<'a>, key: &'static str) -> R<Option<Section<'a>>> {
        let Some(item) = s.get(key) else { return Ok(None) };
        let Some(table) = item.as_table_like() else {
            return Err(self.problem(Some(item), s.key(key), "expected a table"));
        };
        let line = s.table.get_key_value(key).and_then(|(k, _)| k.span()).or_else(|| item.span());
        Ok(Some(Section { table, path: s.key(key), line: self.line(line), read: Vec::new() }))
    }

    fn bool(&self, s: &mut Section, key: &'static str, default: bool) -> R<bool> {
        match s.get(key) {
            None => Ok(default),
            Some(item) => item
                .as_value()
                .and_then(|v| v.as_bool())
                .ok_or_else(|| self.problem(Some(item), s.key(key), "expected true or false")),
        }
    }

    fn int(&self, s: &mut Section, key: &'static str, default: u64, range: RangeInclusive<u64>) -> R<u64> {
        let Some(item) = s.get(key) else { return Ok(default) };
        let Some(v) = item.as_value().and_then(|v| v.as_integer()) else {
            return Err(self.problem(Some(item), s.key(key), "expected an integer"));
        };
        // Rust note: `u64::try_from` fails on a negative number instead of
        // wrapping it around, as `as u64` would.
        match u64::try_from(v) {
            Ok(v) if range.contains(&v) => Ok(v),
            _ => Err(self.problem(
                Some(item),
                s.key(key),
                format!("must be between {} and {}", range.start(), range.end()),
            )),
        }
    }

    /// A string, trimmed; an empty one is refused (a key set to `""` is
    /// more likely a mistake than a wish for "nothing", which is said by
    /// leaving the key out).
    fn string(&self, s: &mut Section, key: &'static str) -> R<Option<String>> {
        let Some(item) = s.get(key) else { return Ok(None) };
        let Some(v) = item.as_value().and_then(|v| v.as_str()) else {
            return Err(self.problem(Some(item), s.key(key), "expected a string"));
        };
        let v = v.trim();
        if v.is_empty() {
            return Err(self.problem(Some(item), s.key(key), "must not be empty (leave the key out instead)"));
        }
        Ok(Some(v.to_string()))
    }

    /// A string among `choices`.
    fn choice(&self, s: &mut Section, key: &'static str, choices: &[&'static str]) -> R<Option<&'static str>> {
        let item = s.table.get(key);
        match self.string(s, key)? {
            None => Ok(None),
            Some(v) => choices.iter().find(|c| **c == v).map(|c| Some(*c)).ok_or_else(|| {
                self.problem(item, s.key(key), format!("must be one of: {}", choices.join(", ")))
            }),
        }
    }

    /// An absolute path; `~/` at the start means the home folder.
    fn path(&self, s: &mut Section, key: &'static str) -> R<Option<PathBuf>> {
        let item = s.table.get(key);
        let Some(v) = self.string(s, key)? else { return Ok(None) };
        let path = match v.strip_prefix("~/") {
            Some(rest) => home().join(rest),
            None => PathBuf::from(&v),
        };
        if !path.is_absolute() {
            return Err(self.problem(item, s.key(key), "expected an absolute path (or one starting with ~/)"));
        }
        Ok(Some(path))
    }

    /// An email address, lowercased. Only its shape is checked: the server
    /// is the judge of the rest.
    fn email(&self, s: &mut Section, key: &'static str) -> R<Option<String>> {
        let item = s.table.get(key);
        let Some(v) = self.string(s, key)? else { return Ok(None) };
        match v.split_once('@') {
            Some((local, domain)) if !local.is_empty() && !domain.is_empty() && !v.contains(char::is_whitespace) => {
                Ok(Some(v.to_lowercase()))
            }
            _ => Err(self.problem(item, s.key(key), format!("not an email address: {v}"))),
        }
    }

    // --- The tables of the file ------------------------------------------------

    /// The `version` key of a file's root, refused if newer than this code.
    fn version(&self, s: &mut Section) -> R<i64> {
        let version_item = s.table.get("version");
        let version = self.int(s, "version", FORMAT_VERSION as u64, 1..=u32::MAX as u64)? as i64;
        if version > FORMAT_VERSION {
            return Err(self.problem(
                version_item,
                "version".into(),
                format!("format {version} was written by a newer NiceMailer; this one reads format {FORMAT_VERSION}"),
            ));
        }
        Ok(version)
    }

    /// `config.toml`: every setting but the accounts.
    fn config(&mut self, root: &dyn TableLike) -> R<Config> {
        let mut s = Section { table: root, path: String::new(), line: None, read: Vec::new() };
        let d = Config::default();
        let version = self.version(&mut s)?;

        let core = match self.table(&mut s, "core")? {
            None => d.core,
            Some(mut t) => {
                let c = CoreSettings {
                    background_sorting: self.bool(&mut t, "background_sorting", d.core.background_sorting)?,
                    idle_exit_secs: self.int(&mut t, "idle_exit_secs", d.core.idle_exit_secs, 1..=86_400)?,
                    update_timeout_secs: self.int(&mut t, "update_timeout_secs", d.core.update_timeout_secs, 60..=86_400)?,
                };
                self.finish(t);
                c
            }
        };
        let sync = match self.table(&mut s, "sync")? {
            None => d.sync.clone(),
            Some(t) => self.sync(t, &d.sync)?,
        };
        let config_sync = match self.table(&mut s, "config_sync")? {
            None => d.config_sync,
            Some(mut t) => {
                let c = ConfigSyncSettings {
                    push_delay_secs: self.int(&mut t, "push_delay_secs", d.config_sync.push_delay_secs, 0..=3_600)?,
                };
                self.finish(t);
                c
            }
        };
        let events = match self.table(&mut s, "events")? {
            None => d.events,
            Some(mut t) => {
                let e = EventSettings {
                    replay_buffer: self.int(&mut t, "replay_buffer", d.events.replay_buffer, 100..=1_000_000)?,
                    client_queue: self.int(&mut t, "client_queue", d.events.client_queue, 10..=100_000)?,
                };
                self.finish(t);
                e
            }
        };
        let import = match self.table(&mut s, "import")? {
            None => d.import.clone(),
            Some(t) => self.import(t, &d.import)?,
        };
        let raw = match self.table(&mut s, "raw")? {
            None => d.raw,
            Some(mut t) => {
                let r = RawSettings {
                    dir: self.path(&mut t, "dir")?,
                    zstd_level: self.int(&mut t, "zstd_level", d.raw.zstd_level, 1..=19)?,
                    fanout: self.int(&mut t, "fanout", d.raw.fanout, 0..=2)?,
                    gc_grace_secs: self.int(&mut t, "gc_grace_secs", d.raw.gc_grace_secs, 0..=30 * 86_400)?,
                    verify_on_read: self.bool(&mut t, "verify_on_read", d.raw.verify_on_read)?,
                };
                self.finish(t);
                r
            }
        };
        let ui = match self.table(&mut s, "ui")? {
            None => d.ui,
            Some(mut t) => {
                let u = UiSettings {
                    view_threshold_ms: self.int(&mut t, "view_threshold_ms", d.ui.view_threshold_ms, 0..=60_000)?,
                    title_bar: match self.choice(&mut t, "title_bar", &["hidden", "shown"])? {
                        Some("shown") => TitleBar::Shown,
                        _ => TitleBar::Hidden,
                    },
                };
                self.finish(t);
                u
            }
        };
        let audit = match self.table(&mut s, "audit")? {
            None => d.audit,
            Some(mut t) => {
                let a = AuditSettings {
                    retention_days: self.int(&mut t, "retention_days", d.audit.retention_days, 1..=36_500)?,
                };
                self.finish(t);
                a
            }
        };

        let compose = match self.table(&mut s, "compose")? {
            None => d.compose,
            Some(mut t) => {
                let c = ComposeSettings {
                    undo_send_seconds: self.int(
                        &mut t,
                        "undo_send_seconds",
                        d.compose.undo_send_seconds,
                        0..=nm_core::compose::UNDO_SEND_MAX_SECS,
                    )?,
                };
                self.finish(t);
                c
            }
        };

        // The accounts are in `accounts.toml`. Found here, they are an error
        // rather than a warning: ignoring them would start the core without
        // the user's accounts, and silently.
        if let Some(item) = s.get("accounts") {
            let line = s.table.get_key_value("accounts").and_then(|(k, _)| k.span()).or_else(|| item.span());
            return Err(Problem {
                file: self.file,
                line: self.line(line),
                key: Some("accounts".into()),
                message: format!("the accounts are declared in {ACCOUNTS_FILE}, next to this file: move the [accounts.…] tables there"),
            });
        }
        self.finish(s);
        Ok(Config { version, core, sync, config_sync, events, import, raw, ui, audit, compose, accounts: BTreeMap::new() })
    }

    /// `accounts.toml`: its `version` and its `[accounts.<key>]` tables,
    /// each starting from the general `sync` and `import`.
    fn accounts_file(&mut self, root: &dyn TableLike, sync: &SyncSettings, import: &ImportSettings) -> R<BTreeMap<String, AccountConfig>> {
        let mut s = Section { table: root, path: String::new(), line: None, read: Vec::new() };
        self.version(&mut s)?;
        let mut accounts = BTreeMap::new();
        if let Some(all) = self.table(&mut s, "accounts")? {
            // Every key of `[accounts]` is an account: none is "unknown".
            let keys: Vec<&str> = all.table.iter().filter(|(_, i)| !i.is_none()).map(|(k, _)| k).collect();
            for key in keys {
                let item = all.table.get(key);
                if !valid_account_key(key) {
                    return Err(self.problem(
                        item,
                        all.key(key),
                        "an account key is made of letters, digits, - _ . (64 at most)",
                    ));
                }
                let Some(table) = item.and_then(|i| i.as_table_like()) else {
                    return Err(self.problem(item, all.key(key), "expected a table"));
                };
                let line = all.table.get_key_value(key).and_then(|(k, _)| k.span()).or_else(|| item?.span());
                let section = Section { table, path: all.key(key), line: self.line(line), read: Vec::new() };
                let account = self.account(section, sync, import)?;
                accounts.insert(key.to_string(), account);
            }
            // `all` is not passed to `finish`: every key of `[accounts]`
            // was read above as an account, none can be unknown.
        }
        // Anything else at the root (a `[sync]` put here by mistake) is
        // reported as unknown, which says it has no effect.
        self.finish(s);
        Ok(accounts)
    }

    /// `[sync]`, or an account's override starting from `base`.
    fn sync(&mut self, mut t: Section, base: &SyncSettings) -> R<SyncSettings> {
        let s = SyncSettings {
            poll_interval_secs: self.int(&mut t, "poll_interval_secs", base.poll_interval_secs, 30..=86_400)?,
            max_connections: self.int(&mut t, "max_connections", base.max_connections, 1..=2)?,
        };
        self.finish(t);
        Ok(s)
    }

    /// `[import]`, or an account's override starting from `base`.
    fn import(&mut self, mut t: Section, base: &ImportSettings) -> R<ImportSettings> {
        let i = ImportSettings {
            threshold_total: self.int(&mut t, "threshold_total", base.threshold_total, 0..=1_000_000)?,
            threshold_unseen: self.int(&mut t, "threshold_unseen", base.threshold_unseen, 0..=1_000_000)?,
            unseen_default_age_days: self.int(&mut t, "unseen_default_age_days", base.unseen_default_age_days, 0..=36_500)?,
            batch_size: self.int(&mut t, "batch_size", base.batch_size, 1..=10_000)?,
        };
        self.finish(t);
        Ok(i)
    }

    fn account(&mut self, mut s: Section, sync: &SyncSettings, import: &ImportSettings) -> R<AccountConfig> {
        let email = self.email(&mut s, "email")?.ok_or_else(|| self.missing(&s, "email"))?;
        let display_name = self.string(&mut s, "display_name")?;
        let auth = match self.choice(&mut s, "auth", &["password", "oauth2"])? {
            Some("oauth2") => AuthMethod::Oauth2,
            _ => AuthMethod::Password,
        };
        let config_sync = self.bool(&mut s, "config_sync", false)?;
        let read_only = self.bool(&mut s, "read_only", false)?;
        let authserv_id = self.string(&mut s, "authserv_id")?;
        let imap = match self.table(&mut s, "imap")? {
            Some(t) => self.server(t, &email, Protocol::Imap)?,
            None => return Err(self.missing(&s, "imap")),
        };
        let smtp = match self.table(&mut s, "smtp")? {
            Some(t) => Some(self.server(t, &email, Protocol::Smtp)?),
            None => None,
        };
        let identities = self.identities(&mut s)?;
        let save_sent = match self.choice(&mut s, "save_sent", &["auto", "always", "never"])? {
            Some("always") => SaveSent::Always,
            Some("never") => SaveSent::Never,
            _ => SaveSent::Auto,
        };
        let sync = match self.table(&mut s, "sync")? {
            Some(t) => self.sync(t, sync)?,
            None => sync.clone(),
        };
        let import = match self.table(&mut s, "import")? {
            Some(t) => self.import(t, import)?,
            None => import.clone(),
        };
        self.finish(s);
        Ok(AccountConfig { email, display_name, auth, config_sync, read_only, authserv_id, imap, smtp, save_sent, identities, sync, import })
    }

    fn server(&mut self, mut s: Section, email: &str, protocol: Protocol) -> R<ServerConfig> {
        let host_item = s.table.get("host");
        let host = self.string(&mut s, "host")?.ok_or_else(|| self.missing(&s, "host"))?.to_lowercase();
        if host.contains(char::is_whitespace) || host.contains('/') || host.contains("://") {
            return Err(self.problem(host_item, s.key("host"), "expected a host name, e.g. imap.example.com"));
        }
        let security_item = s.table.get("security");
        let security = match self.choice(&mut s, "security", &["tls", "starttls", "none"])? {
            Some("starttls") => Security::Starttls,
            Some("none") => Security::None,
            _ => Security::Tls,
        };
        if security == Security::None && !is_loopback(&host) {
            return Err(self.problem(
                security_item,
                s.key("security"),
                "\"none\" would send the password in clear text: allowed only for a server on this Mac (localhost)",
            ));
        }
        let port = self.int(&mut s, "port", protocol.default_port(security) as u64, 1..=65_535)? as u16;
        let username = self.string(&mut s, "username")?.unwrap_or_else(|| email.to_string());
        let ca_file = self.path(&mut s, "ca_file")?;
        // No secret in the file (`configuration.md`, section 2): a password
        // here is refused rather than ignored, so it does not stay, in clear
        // text, in a file the specification invites to keep in Git.
        if let Some(item) = s.get("password") {
            let account = s.path.split('.').nth(1).unwrap_or("<key>");
            return Err(self.problem(
                Some(item),
                s.key("password"),
                format!(
                    "passwords are not kept in accounts.toml: remove this line, and save the password in the \
                     Keychain with: nicemailer account password {account}"
                ),
            ));
        }
        self.finish(s);
        Ok(ServerConfig { host, port, security, username, ca_file })
    }

    /// `identities`, written as `[[accounts.<key>.identities]]` tables or as
    /// an inline array of `{ email = …, name = … }`.
    fn identities(&mut self, s: &mut Section) -> R<Vec<Identity>> {
        let Some(item) = s.get("identities") else { return Ok(Vec::new()) };
        let path = s.key("identities");
        let tables: Vec<(&dyn TableLike, Option<Range<usize>>)> = if let Some(aot) = item.as_array_of_tables() {
            aot.iter().map(|t| (t as &dyn TableLike, t.span())).collect()
        } else if let Some(array) = item.as_array() {
            let mut v = Vec::new();
            for value in array.iter() {
                match value.as_inline_table() {
                    Some(t) => v.push((t as &dyn TableLike, value.span())),
                    None => {
                        return Err(Problem {
                            file: self.file,
                            line: self.line(value.span()),
                            key: Some(path),
                            message: "expected tables { email = … }".into(),
                        });
                    }
                }
            }
            v
        } else {
            return Err(self.problem(Some(item), path, "expected a list of identities"));
        };
        let mut out = Vec::new();
        for (i, (table, span)) in tables.into_iter().enumerate() {
            let mut t = Section { table, path: format!("{path}[{i}]"), line: self.line(span), read: Vec::new() };
            let email = self.email(&mut t, "email")?.ok_or_else(|| self.missing(&t, "email"))?;
            let name = self.string(&mut t, "name")?;
            self.finish(t);
            out.push(Identity { email, name });
        }
        Ok(out)
    }
}

/// Which protocol a server table is for: they differ by their default ports.
#[derive(Clone, Copy)]
enum Protocol {
    Imap,
    Smtp,
}

impl Protocol {
    fn default_port(self, security: Security) -> u16 {
        match (self, security) {
            (Protocol::Imap, Security::Tls) => 993,
            (Protocol::Imap, _) => 143,
            (Protocol::Smtp, Security::Tls) => 465,
            (Protocol::Smtp, Security::Starttls) => 587,
            (Protocol::Smtp, Security::None) => 25,
        }
    }
}

/// Same rule as the names of archive accounts (`nm-store`, `import.rs`):
/// keys end up in file names and Keychain entries.
pub fn valid_account_key(key: &str) -> bool {
    !key.is_empty() && key.len() <= 64 && key.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// A host that is this Mac: the only one allowed without encryption.
fn is_loopback(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1" | "[::1]")
}

// ---------------------------------------------------------------------------
// Comparing two versions of the file
// ---------------------------------------------------------------------------

/// Every setting as `dotted.key = value`, the values in JSON. Used to tell
/// which keys a new version of the file changed.
pub fn flatten(config: &Config) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    // Serializing these plain structs to JSON cannot fail.
    let value = serde_json::to_value(config).expect("config serializable");
    flatten_into(&value, String::new(), &mut out);
    out
}

fn flatten_into(value: &serde_json::Value, prefix: String, out: &mut BTreeMap<String, String>) {
    match value {
        serde_json::Value::Object(map) if !map.is_empty() => {
            for (k, v) in map {
                let key = if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") };
                flatten_into(v, key, out);
            }
        }
        // An empty table (no account) has no key.
        serde_json::Value::Object(_) => {}
        // Scalars, and lists (identities) as a whole.
        other => {
            out.insert(prefix, other.to_string());
        }
    }
}

/// Keys whose value differs between `old` and `new`, added or removed
/// included, sorted. For an account, every key of its table is listed: a
/// new account `perso` lists `accounts.perso.email`, `accounts.perso.imap.host`…
pub fn changed_keys(old: &Config, new: &Config) -> Vec<String> {
    let (a, b) = (flatten(old), flatten(new));
    let mut keys: Vec<String> = a.keys().chain(b.keys()).filter(|k| a.get(*k) != b.get(*k)).cloned().collect();
    keys.sort();
    keys.dedup();
    keys
}

// ---------------------------------------------------------------------------
// The files of defaults
// ---------------------------------------------------------------------------

/// The output of `nicemailer config defaults`: a valid `config.toml` with
/// every key at its default value, and what it does in a comment
/// (`configuration.md`, sections 9 and 11). A test checks that it parses
/// back to `Config::default()`, so it cannot drift from the code.
pub fn defaults_file() -> String {
    format!(
        "# NiceMailer settings: every key at its default value.\n\
         # A real file needs only the keys that differ (docs/configuration.md).\n{}",
        settings_body()
    )
}

/// The `config.toml` the core creates when there is none
/// (`configuration.md`, section 3): [`defaults_file`] with every setting
/// commented out.
///
/// Commented out rather than written: a value written in the file is the
/// user's choice, and stays when a later NiceMailer changes its default; a
/// commented line shows the key, its default and its role, and changes
/// nothing. The table headers (`[sync]`) stay, so removing the `# ` of a
/// line is all it takes to change a setting. A test checks that the file
/// parses to the defaults, and still parses once any line is uncommented.
pub fn config_template() -> String {
    let body: String = settings_body()
        .lines()
        .map(|line| {
            // A `key = value` line: neither a comment, a header, blank, nor
            // `version`, which says the format and is not a setting.
            if line.is_empty() || line.starts_with('#') || line.starts_with('[') || line.starts_with("version ") {
                line.to_string()
            } else {
                format!("# {line}")
            }
        })
        .map(|l| l + "\n")
        .collect();
    format!(
        "# NiceMailer settings (docs/configuration.md).\n\
         #\n\
         # Every setting is listed with its default value, commented out. To\n\
         # change one, remove the \"# \" in front of it and edit the value. A\n\
         # line left commented keeps the default, even when a later version of\n\
         # NiceMailer changes it. The accounts are in {ACCOUNTS_FILE}, next to\n\
         # this file. Passwords are never kept in either: they are in the\n\
         # Keychain.\n{body}"
    )
}

/// The `accounts.toml` the core creates when there is none, also printed by
/// `nicemailer config defaults --accounts`: no account, and an example one
/// commented out. A test checks that the example, uncommented, is valid.
pub fn accounts_template() -> String {
    format!(
        r#"# NiceMailer accounts (docs/accounts.md). The general settings are in
# {CONFIG_FILE}, next to this file.
#
# An account is best added from the app, or with `nicemailer account add`,
# which also saves its password in the Keychain. It can also be written
# here by hand; its password is then saved with
# `nicemailer account password <key>`. Passwords are never kept in this
# file.
version = {FORMAT_VERSION}

# An account. The key ("perso") names it; renaming it by hand creates a
# new account and disables the old one, whose data is kept. Rename it
# with `nicemailer account rename` instead.
#
# [accounts.perso]
# # The main address of the account: the only one required. It is the
# # login name on the servers unless `username` says otherwise, and the
# # default sender. Other addresses of the same mailbox are identities,
# # below.
# email = "me@example.com"
# display_name = "My Name"
# auth = "password"            # or "oauth2" (not supported yet)
# # Share the decisions of this mailbox with the other Macs (or the other
# # people) using it, through the mailbox itself: the screening of
# # addresses and domains, the roles of the folders, the tags and the
# # groups are written to a folder "nicemailer-sync" on its server, and
# # read back by the others. Off by default: anyone who can open the
# # mailbox, its administrator included, can read that folder. Neither
# # the passwords nor these two files are ever shared that way.
# # Not implemented yet: true is accepted and does nothing.
# config_sync = false
# read_only = false              # true: never change anything on the server
# authserv_id = "mx.example.com"   # only to replace the detected one
# save_sent = "auto"           # copy sent mail to Sent: auto (not on Gmail
#                              # and Fastmail, which do it), always, never
#
# [accounts.perso.imap]
# host = "imap.example.com"
# port = 993                   # default: 993 for tls, 143 otherwise
# security = "tls"             # tls, starttls, or none (localhost only)
# username = "me"              # default: the email
# ca_file = "~/certs/my-ca.pem"    # extra authority for this server
#
# [accounts.perso.smtp]            # without it, the account cannot send
# host = "smtp.example.com"
# port = 465                   # default: 465 for tls, 587 for starttls
# security = "tls"
# username = "me"              # default: the email
# # The password: the account's SMTP one if saved
# # (nicemailer account password perso --smtp), else the IMAP one.
#
# # Identities: the other addresses this mailbox sends from (aliases,
# # other domains delivered to it). Each is a [[...]] block, an item of
# # a list: repeat the block for each address. They are never used to
# # log in. With the main address, they are the only senders a draft
# # accepts, and a reply goes from the one the mail was sent to. They
# # also count as your own addresses: writing from one does not file it
# # as a correspondent. The SMTP server must allow sending from them,
# # which NiceMailer does not check: an address it refuses fails at
# # sending.
# [[accounts.perso.identities]]
# email = "contact@example.com"
# name = "My Company"            # the name shown; default: display_name
#
# [[accounts.perso.identities]]
# email = "info@example.com"
#
# # The [sync] and [import] settings of config.toml, for this account
# # only.
# [accounts.perso.sync]
# poll_interval_secs = 600
"#
    )
}

/// The settings of [`defaults_file`], after its header.
fn settings_body() -> String {
    let d = Config::default();
    format!(
        r#"version = {version}

[core]
# Keep sorting incoming mail while NiceMailer is closed.
background_sorting = {background_sorting}
# Seconds before the core exits without any client, when background
# sorting is off.
idle_exit_secs = {idle_exit_secs}
# Longest duration of the maintenance mode during an update, in seconds.
update_timeout_secs = {update_timeout_secs}

[sync]
# Seconds between two checks of the folders other than the Inbox (the
# Inbox is followed live). 30 to 86400. An account may override it in
# its own sync table, in accounts.toml.
poll_interval_secs = {poll_interval_secs}
# IMAP connections per account: 2 (folders and new mail on one, bodies
# downloaded on the other) or 1 (everything on one, for a server that
# allows few). An account may override it too.
max_connections = {max_connections}

[config_sync]
# Seconds of grouping before writing the synchronized configuration.
push_delay_secs = {push_delay_secs}

[events]
# Events kept in memory, replayed to a client that reconnects.
replay_buffer = {replay_buffer}
# Events a slow client may lag behind before it is told to reload.
client_queue = {client_queue}

[import]
# First import of an account: archiving the old mail of the Inbox is
# offered above this many mails...
threshold_total = {threshold_total}
# ...or this many unread mails.
threshold_unseen = {threshold_unseen}
# Default age, in days, of the unread mails archived.
unseen_default_age_days = {unseen_default_age_days}
# Mails moved per IMAP command. An account may override these in its
# own import table, in accounts.toml.
batch_size = {batch_size}

[raw]
# Root of the raw message store; by default raw.noindex in the data
# directory.
# dir = "~/Library/Application Support/NiceMailer/raw.noindex"
# zstd compression level, 1 to 19.
zstd_level = {zstd_level}
# Levels of subdirectories, 0 to 2.
fanout = {fanout}
# Seconds an unreferenced raw file is kept before it is deleted.
gc_grace_secs = {gc_grace_secs}
# Check the hash of every raw file read.
verify_on_read = {verify_on_read}

[ui]
# Milliseconds a mail must stay on screen to count as viewed.
view_threshold_ms = {view_threshold_ms}
# The title bar of the window (title, red, orange and green buttons):
# "hidden", the content up to the top edge (move the window by the status
# bar, quit with Cmd-Q), or "shown". Applied at once.
title_bar = "{title_bar}"

[audit]
# Days the history of changes is kept.
retention_days = {retention_days}

[compose]
# Seconds a mail waits after Cmd-Return before it leaves, during which
# z cancels it. 0 to 30; 0 sends at once.
undo_send_seconds = {undo_send_seconds}
"#,
        version = d.version,
        background_sorting = d.core.background_sorting,
        idle_exit_secs = d.core.idle_exit_secs,
        update_timeout_secs = d.core.update_timeout_secs,
        poll_interval_secs = d.sync.poll_interval_secs,
        max_connections = d.sync.max_connections,
        push_delay_secs = d.config_sync.push_delay_secs,
        replay_buffer = d.events.replay_buffer,
        client_queue = d.events.client_queue,
        threshold_total = d.import.threshold_total,
        threshold_unseen = d.import.threshold_unseen,
        unseen_default_age_days = d.import.unseen_default_age_days,
        batch_size = d.import.batch_size,
        zstd_level = d.raw.zstd_level,
        fanout = d.raw.fanout,
        gc_grace_secs = d.raw.gc_grace_secs,
        verify_on_read = d.raw.verify_on_read,
        view_threshold_ms = d.ui.view_threshold_ms,
        title_bar = d.ui.title_bar.as_str(),
        retention_days = d.audit.retention_days,
        undo_send_seconds = d.compose.undo_send_seconds,
    )
}
