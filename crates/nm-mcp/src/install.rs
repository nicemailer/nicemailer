//! The entry of NiceMailer in the configuration of Claude Desktop
//! (`specifications/mcp.md`, section 3, "Installation"):
//! `~/Library/Application Support/Claude/claude_desktop_config.json`, key
//! `mcpServers.nicemailer`, `{"command": "<path of nicemailer-mcp>"}`.
//!
//! The app runs `nicemailer-mcp claude-desktop install` from its menu, and
//! `… install --repair` at each launch, which updates the entry only when
//! it exists and names another binary (the app was moved). The rules live
//! here, in Rust, tested once, rather than in the app:
//!
//! - only `mcpServers.nicemailer` is changed; every other key of the file
//!   is kept (their order is not: the file is rewritten with its keys
//!   sorted, which changes nothing for Claude Desktop);
//! - written to a temporary file next to it, then renamed: Claude Desktop
//!   never reads half a file;
//! - a file that is not a JSON object is never overwritten: the command
//!   fails, and the app shows the block to copy instead.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

/// The key of NiceMailer in `mcpServers`.
pub const KEY: &str = "nicemailer";

/// Where Claude Desktop keeps its configuration.
pub fn default_config() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    home.join("Library/Application Support/Claude/claude_desktop_config.json")
}

/// The state of the entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// No file, or no entry `nicemailer`.
    Absent,
    /// The entry runs this binary.
    Configured,
    /// The entry runs another binary: the app was moved, or another copy.
    Elsewhere(String),
    /// The file is not a JSON object: never written.
    Unreadable(String),
}

/// What `install` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Added,
    Updated,
    Unchanged,
    /// `--repair` without an entry: nothing to repair.
    NotInstalled,
}

/// The entry of `binary`.
pub fn entry(binary: &Path) -> Value {
    json!({"command": binary.display().to_string()})
}

/// The file's content as a JSON object; `None` when there is no file.
fn read(config: &Path) -> Result<Option<Map<String, Value>>, String> {
    let text = match std::fs::read_to_string(config) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", config.display())),
    };
    if text.trim().is_empty() {
        return Ok(Some(Map::new()));
    }
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(o)) => Ok(Some(o)),
        Ok(_) => Err(format!("{} is not a JSON object", config.display())),
        Err(e) => Err(format!("{} is not valid JSON ({e})", config.display())),
    }
}

pub fn status(config: &Path, binary: &Path) -> Status {
    match read(config) {
        Err(e) => Status::Unreadable(e),
        Ok(None) => Status::Absent,
        Ok(Some(o)) => match o.get("mcpServers").and_then(|s| s.get(KEY)) {
            None => Status::Absent,
            Some(e) => match e.get("command").and_then(Value::as_str) {
                Some(c) if Path::new(c) == binary => Status::Configured,
                Some(c) => Status::Elsewhere(c.to_string()),
                None => Status::Elsewhere(String::new()),
            },
        },
    }
}

/// Adds or updates the entry of `binary`. `repair`: only an entry that
/// exists is updated.
pub fn install(config: &Path, binary: &Path, repair: bool) -> Result<Outcome, String> {
    let mut root = read(config)?.unwrap_or_default();
    let servers = root.entry("mcpServers").or_insert_with(|| json!({}));
    let Some(servers) = servers.as_object_mut() else {
        return Err(format!("{}: mcpServers is not an object", config.display()));
    };
    let wanted = entry(binary);
    let outcome = match servers.get(KEY) {
        None if repair => return Ok(Outcome::NotInstalled),
        None => Outcome::Added,
        Some(e) if e.get("command") == wanted.get("command") => return Ok(Outcome::Unchanged),
        Some(_) => Outcome::Updated,
    };
    // An entry that exists keeps what the user added to it (`env`…);
    // only its command changes.
    match servers.get_mut(KEY).and_then(Value::as_object_mut) {
        Some(e) => {
            e.insert("command".into(), wanted["command"].clone());
            e.remove("args");
        }
        None => {
            servers.insert(KEY.into(), wanted);
        }
    }
    write(config, &Value::Object(root))?;
    Ok(outcome)
}

/// Removes the entry (the uninstallation of the app). `false`: there was
/// none.
pub fn remove(config: &Path) -> Result<bool, String> {
    let Some(mut root) = read(config)? else { return Ok(false) };
    let removed = root.get_mut("mcpServers").and_then(Value::as_object_mut).and_then(|s| s.remove(KEY)).is_some();
    if removed {
        write(config, &Value::Object(root))?;
    }
    Ok(removed)
}

/// Writes the file through a temporary one renamed over it. The folder of
/// Claude Desktop must exist: without it, Claude Desktop is not installed,
/// and creating its folder would be a surprise.
fn write(config: &Path, v: &Value) -> Result<(), String> {
    let dir = config.parent().ok_or_else(|| format!("{}: no folder", config.display()))?;
    if !dir.is_dir() {
        return Err(format!("Claude Desktop does not seem to be installed: no folder {}", dir.display()));
    }
    let tmp = dir.join(format!(".{}.nicemailer-{:08x}", config.file_name().unwrap_or_default().to_string_lossy(), rand::random::<u32>()));
    let mut text = serde_json::to_string_pretty(v).map_err(|e| e.to_string())?;
    text.push('\n');
    std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, config).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("{}: {e}", config.display())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BIN: &str = "/Applications/NiceMailer.app/Contents/Helpers/nicemailer-mcp";

    #[test]
    fn a_missing_file_gets_the_entry() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("claude_desktop_config.json");
        assert_eq!(status(&config, Path::new(BIN)), Status::Absent);
        assert_eq!(install(&config, Path::new(BIN), true), Ok(Outcome::NotInstalled));
        assert!(!config.exists());
        assert_eq!(install(&config, Path::new(BIN), false), Ok(Outcome::Added));
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
        assert_eq!(v, json!({"mcpServers": {"nicemailer": {"command": BIN}}}));
        assert_eq!(status(&config, Path::new(BIN)), Status::Configured);
        assert_eq!(install(&config, Path::new(BIN), false), Ok(Outcome::Unchanged));
        // No temporary file left.
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn the_other_servers_and_settings_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("claude_desktop_config.json");
        let before = json!({
            "mcpServers": {"filesystem": {"command": "npx", "args": ["-y", "server-filesystem", "/Users/me"]}},
            "preferences": {"quickEntryShortcut": "off"}
        });
        std::fs::write(&config, before.to_string()).unwrap();
        assert_eq!(install(&config, Path::new(BIN), false), Ok(Outcome::Added));
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
        assert_eq!(v["mcpServers"]["filesystem"], before["mcpServers"]["filesystem"]);
        assert_eq!(v["preferences"], before["preferences"]);
        assert_eq!(v["mcpServers"]["nicemailer"]["command"], BIN);
        assert_eq!(remove(&config), Ok(true));
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
        assert_eq!(v, before);
        assert_eq!(remove(&config), Ok(false));
    }

    #[test]
    fn a_moved_app_is_repaired_keeping_the_users_additions() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("claude_desktop_config.json");
        let old = "/Users/me/Downloads/NiceMailer.app/Contents/Helpers/nicemailer-mcp";
        std::fs::write(&config, json!({"mcpServers": {"nicemailer": {"command": old, "env": {"X": "1"}}}}).to_string()).unwrap();
        assert_eq!(status(&config, Path::new(BIN)), Status::Elsewhere(old.into()));
        assert_eq!(install(&config, Path::new(BIN), true), Ok(Outcome::Updated));
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
        assert_eq!(v["mcpServers"]["nicemailer"], json!({"command": BIN, "env": {"X": "1"}}));
    }

    #[test]
    fn an_unreadable_file_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("claude_desktop_config.json");
        for broken in ["{\"mcpServers\": {", "[1, 2]", "{\"mcpServers\": 3}"] {
            std::fs::write(&config, broken).unwrap();
            assert!(install(&config, Path::new(BIN), false).is_err(), "{broken}");
            assert_eq!(std::fs::read_to_string(&config).unwrap(), broken);
        }
        std::fs::write(&config, "[1, 2]").unwrap();
        assert!(matches!(status(&config, Path::new(BIN)), Status::Unreadable(_)));
        // An empty file is an empty configuration.
        std::fs::write(&config, "").unwrap();
        assert_eq!(install(&config, Path::new(BIN), false), Ok(Outcome::Added));
    }

    #[test]
    fn without_claude_desktop_nothing_is_created() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("Claude/claude_desktop_config.json");
        assert!(install(&config, Path::new(BIN), false).unwrap_err().contains("not seem to be installed"));
        assert!(!dir.path().join("Claude").exists());
    }
}
