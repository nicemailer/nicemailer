//! Reading and validating `config.toml` and `accounts.toml`
//! (`configuration.md`, sections 3, 4 and 12).

use nm_engine::config::{
    self, AuthMethod, Config, Parsed, Problem, Security, accounts_template, changed_keys, config_template, defaults_file,
    flatten, parse,
};

/// The example of `configuration.md`, section 3: `config.toml`...
const SETTINGS: &str = r#"version = 1

[core]
background_sorting = true

[sync]
poll_interval_secs = 300
"#;

/// ...and `accounts.toml`.
const EXAMPLE: &str = r#"version = 1

[accounts.perso]
email = "Prenom@Exemple.fr"
display_name = "Prénom Nom"
auth = "password"
config_sync = false

[accounts.perso.imap]
host = "imap.exemple.fr"
port = 993
security = "tls"
username = "prenom"

[accounts.perso.smtp]
host = "smtp.exemple.fr"
port = 465
security = "tls"
username = "prenom"

[[accounts.perso.identities]]
email = "contact@exemple.fr"
name = "Prénom Nom"
"#;

/// `text` as `config.toml`, without accounts.
fn settings(text: &str) -> Result<Parsed, Problem> {
    parse(text, "")
}

/// `text` as `accounts.toml`, with the default settings.
fn accounts(text: &str) -> Result<Parsed, Problem> {
    parse("", text)
}

/// A `config.toml` expected to be refused, and its problem.
fn refused(text: &str) -> Problem {
    match settings(text) {
        Err(p) => {
            assert_eq!(p.file, "config.toml", "{p}");
            p
        }
        Ok(_) => panic!("expected the file to be refused:\n{text}"),
    }
}

/// An `accounts.toml` expected to be refused, and its problem.
fn refused_accounts(text: &str) -> Problem {
    match accounts(text) {
        Err(p) => {
            assert_eq!(p.file, "accounts.toml", "{p}");
            p
        }
        Ok(_) => panic!("expected the file to be refused:\n{text}"),
    }
}

#[test]
fn an_empty_file_gives_the_defaults() {
    let parsed = parse("", "").unwrap();
    assert_eq!(parsed.config, Config::default());
    assert!(parsed.warnings.is_empty());
    assert!(parsed.config.accounts.is_empty());
}

#[test]
fn a_missing_file_reads_as_none() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(config::read(&dir.path().join("config.toml"), config::CONFIG_FILE).unwrap(), None);
}

#[test]
fn a_file_that_is_not_utf8_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, [0xff, 0xfe, b'a']).unwrap();
    assert!(config::read(&path, config::CONFIG_FILE).unwrap_err().message.contains("UTF-8"));
}

#[test]
fn the_example_of_the_specification_is_read() {
    let parsed = parse(SETTINGS, EXAMPLE).unwrap();
    assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
    let a = &parsed.config.accounts["perso"];
    // Addresses are lowercased; names are kept as written.
    assert_eq!(a.email, "prenom@exemple.fr");
    assert_eq!(a.display_name.as_deref(), Some("Prénom Nom"));
    assert_eq!(a.auth, AuthMethod::Password);
    assert_eq!(a.imap.host, "imap.exemple.fr");
    assert_eq!(a.imap.port, 993);
    assert_eq!(a.imap.security, Security::Tls);
    assert_eq!(a.imap.username, "prenom");
    assert_eq!(a.smtp.as_ref().unwrap().port, 465);
    assert_eq!(a.identities.len(), 1);
    assert_eq!(a.identities[0].email, "contact@exemple.fr");
    assert_eq!(a.sync.poll_interval_secs, 300);
}

#[test]
fn server_defaults_follow_the_security() {
    let text = r#"
[accounts.a]
email = "me@example.com"
[accounts.a.imap]
host = "IMAP.Example.com"
security = "starttls"
[accounts.a.smtp]
host = "smtp.example.com"
security = "starttls"
"#;
    let a = &accounts(text).unwrap().config.accounts["a"];
    assert_eq!(a.imap.host, "imap.example.com");
    assert_eq!(a.imap.port, 143);
    assert_eq!(a.imap.username, "me@example.com", "the username defaults to the email");
    assert_eq!(a.smtp.as_ref().unwrap().port, 587);

    let tls = accounts("[accounts.a]\nemail = \"me@example.com\"\n[accounts.a.imap]\nhost = \"h\"\n").unwrap();
    assert_eq!(tls.config.accounts["a"].imap.port, 993);
    assert_eq!(tls.config.accounts["a"].imap.security, Security::Tls);
}

#[test]
fn a_syntax_error_gives_its_line() {
    let p = refused("version = 1\n\n[sync]\npoll_interval_secs = = 3\n");
    assert_eq!(p.line, Some(4), "{p}");
}

#[test]
fn a_duplicate_key_is_refused() {
    let p = refused("[sync]\npoll_interval_secs = 60\npoll_interval_secs = 90\n");
    assert_eq!(p.line, Some(3), "{p}");
}

#[test]
fn a_wrong_type_gives_its_line_and_key() {
    let p = refused("version = 1\n[sync]\npoll_interval_secs = \"often\"\n");
    assert_eq!(p.line, Some(3));
    assert_eq!(p.key.as_deref(), Some("sync.poll_interval_secs"));
    assert!(p.message.contains("integer"));
}

#[test]
fn a_value_out_of_range_gives_its_line() {
    let p = refused("[events]\nreplay_buffer = 10000\nclient_queue = 3\n");
    assert_eq!(p.line, Some(3));
    assert_eq!(p.key.as_deref(), Some("events.client_queue"));
    assert!(p.message.contains("between 10 and 100000"), "{p}");
    // Negative numbers too, which must not wrap around.
    let p = refused("[sync]\npoll_interval_secs = -1\n");
    assert_eq!(p.key.as_deref(), Some("sync.poll_interval_secs"));
}

#[test]
fn the_port_must_fit() {
    let p = refused_accounts("[accounts.a]\nemail = \"a@b.c\"\n[accounts.a.imap]\nhost = \"h\"\nport = 70000\n");
    assert_eq!(p.line, Some(5));
    assert_eq!(p.key.as_deref(), Some("accounts.a.imap.port"));
}

#[test]
fn an_unknown_key_is_a_warning_and_the_rest_applies() {
    let text = "version = 1\nfuture_feature = true\n[sync]\npoll_interval_secs = 60\nturbo = 3\n[newer_table]\nx = 1\n";
    let parsed = settings(text).unwrap();
    assert_eq!(parsed.config.sync.poll_interval_secs, 60);
    let keys: Vec<_> = parsed.warnings.iter().map(|w| (w.key.clone().unwrap(), w.line)).collect();
    assert_eq!(
        keys,
        vec![
            ("sync.turbo".to_string(), Some(5)),
            ("future_feature".to_string(), Some(2)),
            ("newer_table".to_string(), Some(6)),
        ]
    );
}

#[test]
fn unknown_keys_inside_accounts_are_warnings() {
    let text = "[accounts.a]\nemail = \"a@b.c\"\nnickname = \"x\"\n[accounts.a.imap]\nhost = \"h\"\ncompress = true\n";
    let parsed = accounts(text).unwrap();
    let keys: Vec<_> = parsed.warnings.iter().map(|w| w.key.clone().unwrap()).collect();
    assert_eq!(keys, vec!["accounts.a.imap.compress", "accounts.a.nickname"]);
}

#[test]
fn an_account_needs_an_email_and_an_imap_server() {
    let p = refused_accounts("version = 1\n[accounts.a]\ndisplay_name = \"A\"\n[accounts.a.imap]\nhost = \"h\"\n");
    assert_eq!(p.key.as_deref(), Some("accounts.a.email"));
    assert_eq!(p.line, Some(2), "on the header of the account");
    let p = refused_accounts("[accounts.a]\nemail = \"a@b.c\"\n");
    assert_eq!(p.key.as_deref(), Some("accounts.a.imap"));
    let p = refused_accounts("[accounts.a]\nemail = \"a@b.c\"\n[accounts.a.imap]\nport = 993\n");
    assert_eq!(p.key.as_deref(), Some("accounts.a.imap.host"));
    let p = refused_accounts("[accounts.a]\nemail = \"not an address\"\n[accounts.a.imap]\nhost = \"h\"\n");
    assert_eq!(p.key.as_deref(), Some("accounts.a.email"));
}

#[test]
fn an_account_key_is_simple() {
    let p = refused_accounts("[accounts.\"mon compte\"]\nemail = \"a@b.c\"\n[accounts.\"mon compte\".imap]\nhost = \"h\"\n");
    assert!(p.message.contains("letters, digits"), "{p}");
}

#[test]
fn an_empty_string_is_refused() {
    let p = refused_accounts("[accounts.a]\nemail = \"a@b.c\"\ndisplay_name = \"  \"\n[accounts.a.imap]\nhost = \"h\"\n");
    assert_eq!(p.key.as_deref(), Some("accounts.a.display_name"));
}

#[test]
fn a_choice_outside_the_list_is_refused() {
    let p = refused_accounts("[accounts.a]\nemail = \"a@b.c\"\n[accounts.a.imap]\nhost = \"h\"\nsecurity = \"ssl\"\n");
    assert_eq!(p.line, Some(5));
    assert!(p.message.contains("tls, starttls, none"), "{p}");
}

#[test]
fn no_encryption_only_on_this_mac() {
    let p = refused_accounts("[accounts.a]\nemail = \"a@b.c\"\n[accounts.a.imap]\nhost = \"imap.example.com\"\nsecurity = \"none\"\n");
    assert_eq!(p.key.as_deref(), Some("accounts.a.imap.security"));
    assert!(p.message.contains("clear text"));
    let ok = accounts("[accounts.a]\nemail = \"a@b.c\"\n[accounts.a.imap]\nhost = \"localhost\"\nsecurity = \"none\"\n").unwrap();
    assert_eq!(ok.config.accounts["a"].imap.port, 143);
}

#[test]
fn a_newer_format_is_refused() {
    let p = refused("version = 2\n");
    assert_eq!(p.line, Some(1));
    assert!(p.message.contains("newer NiceMailer"));
    assert!(settings("version = 0\n").is_err());
}

#[test]
fn an_account_overrides_general_settings() {
    let general = "[sync]\npoll_interval_secs = 120\n[import]\nbatch_size = 500\n";
    let text = r#"
[accounts.a]
email = "a@b.c"
[accounts.a.imap]
host = "h"
[accounts.a.sync]
poll_interval_secs = 60

[accounts.b]
email = "b@b.c"
[accounts.b.imap]
host = "h"
[accounts.b.import]
threshold_total = 10
"#;
    let c = parse(general, text).unwrap().config;
    assert_eq!(c.sync.poll_interval_secs, 120);
    assert_eq!(c.accounts["a"].sync.poll_interval_secs, 60);
    assert_eq!(c.accounts["b"].sync.poll_interval_secs, 120, "inherits the general value");
    // An override starts from the general values: batch_size stays 500.
    assert_eq!(c.accounts["b"].import.threshold_total, 10);
    assert_eq!(c.accounts["b"].import.batch_size, 500);
    assert_eq!(c.accounts["a"].import.batch_size, 500);
}

#[test]
fn identities_as_tables_or_inline() {
    let tables = "[accounts.a]\nemail = \"a@b.c\"\n[accounts.a.imap]\nhost = \"h\"\n\
                  [[accounts.a.identities]]\nemail = \"X@b.c\"\n[[accounts.a.identities]]\nemail = \"y@b.c\"\nname = \"Y\"\n";
    let inline = "[accounts.a]\nemail = \"a@b.c\"\nidentities = [{ email = \"X@b.c\" }, { email = \"y@b.c\", name = \"Y\" }]\n\
                  [accounts.a.imap]\nhost = \"h\"\n";
    for text in [tables, inline] {
        let ids = accounts(text).unwrap().config.accounts["a"].identities.clone();
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0].email, "x@b.c");
        assert_eq!(ids[1].name.as_deref(), Some("Y"));
    }
    let p = refused_accounts("[accounts.a]\nemail = \"a@b.c\"\n[accounts.a.imap]\nhost = \"h\"\n[[accounts.a.identities]]\nname = \"N\"\n");
    assert_eq!(p.key.as_deref(), Some("accounts.a.identities[0].email"));
}

#[test]
fn paths_accept_the_home_folder() {
    let home = std::env::var("HOME").unwrap();
    let c = accounts("[accounts.a]\nemail = \"a@b.c\"\n[accounts.a.imap]\nhost = \"h\"\nca_file = \"~/certs/ca.pem\"\n").unwrap();
    assert_eq!(c.config.accounts["a"].imap.ca_file.as_ref().unwrap(), &std::path::PathBuf::from(home).join("certs/ca.pem"));
    let p = refused("[raw]\ndir = \"relative/dir\"\n");
    assert_eq!(p.key.as_deref(), Some("raw.dir"));
}

#[test]
fn a_password_in_the_file_is_refused() {
    let text = "[accounts.a]\nemail = \"a@b.c\"\n[accounts.a.imap]\nhost = \"h\"\npassword = \"hunter2\"\n";
    let p = refused_accounts(text);
    assert_eq!(p.line, Some(5));
    assert_eq!(p.key.as_deref(), Some("accounts.a.imap.password"));
    assert!(p.message.contains("nicemailer account password a"), "{p}");
    // The problem never quotes the password.
    assert!(!p.to_string().contains("hunter2"));
}

#[test]
fn a_secret_is_never_shown() {
    let secret = nm_engine::config::SecretString::new("hunter2");
    assert!(!format!("{secret:?}").contains("hunter2"));
    let json = serde_json::to_string(&secret).unwrap();
    assert!(json.starts_with("\"sha256:") && !json.contains("hunter2"), "{json}");
}

#[test]
fn changed_keys_name_what_changed() {
    let a = accounts(EXAMPLE).unwrap().config;
    let b = accounts(&EXAMPLE.replace("port = 993", "port = 994")).unwrap().config;
    assert_eq!(changed_keys(&a, &b), vec!["accounts.perso.imap.port"]);
    assert!(changed_keys(&a, &a).is_empty());
    // An account removed lists its keys.
    let removed = changed_keys(&a, &Config::default());
    assert!(removed.contains(&"accounts.perso.email".to_string()));
    assert!(removed.iter().all(|k| k.starts_with("accounts.perso.")), "{removed:?}");
}

#[test]
fn the_defaults_file_is_valid_and_complete() {
    let text = defaults_file();
    let parsed = settings(&text).unwrap();
    assert_eq!(parsed.config, Config::default(), "the defaults file gives the defaults");
    assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
    // Every setting that has a value appears explicitly, so the file shows
    // every key; `raw.dir` (no value: computed from the data directory)
    // appears in a comment. The accounts are not in this file.
    let doc: toml_edit::DocumentMut = text.parse().unwrap();
    for (key, value) in flatten(&Config::default()) {
        if value == "null" {
            assert!(text.contains(&format!("# {}", key.rsplit('.').next().unwrap())), "{key} commented");
            continue;
        }
        let mut item = doc.as_item();
        for part in key.split('.') {
            item = &item[part];
        }
        assert!(!item.is_none(), "{key} missing from the defaults file");
    }
    assert!(!text.contains("[accounts"), "the accounts are in accounts.toml:\n{text}");
}

/// Removes the `# ` in front of a line that is a commented `key = value`
/// or `[table]`, as a user would to change it.
fn uncomment(line: &str) -> String {
    match line.strip_prefix("# ") {
        // A header, maybe followed by a comment: `[table]  # why`.
        Some(rest) if rest.starts_with('[') && rest.split('#').next().unwrap().trim_end().ends_with(']') => rest.to_string(),
        Some(rest) if rest.contains(" = ") => rest.to_string(),
        _ => line.to_string(),
    }
}

#[test]
fn the_created_config_lists_every_setting_commented_out() {
    let text = config_template();
    // As created, it changes nothing.
    let parsed = settings(&text).unwrap();
    assert_eq!(parsed.config, Config::default());
    assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
    // Every key of `defaults_file` is there, commented out, with its
    // default value: the same lines, behind "# ".
    for line in defaults_file().lines() {
        let is_setting = !line.is_empty() && !line.starts_with('#') && !line.starts_with('[') && !line.starts_with("version ");
        if is_setting {
            assert!(text.lines().any(|l| l == format!("# {line}")), "{line} missing, or not commented out");
        } else if line.starts_with('[') {
            assert!(text.lines().any(|l| l == line), "the header {line} stays, to uncomment under it");
        }
    }
    // Uncommenting any one line gives a valid file, with that setting at
    // its default; so does uncommenting them all (raw.dir included, which
    // only names the default place).
    for (i, line) in text.lines().enumerate() {
        let edited = uncomment(line);
        if edited == line {
            continue;
        }
        let changed: String = text.lines().enumerate().map(|(j, l)| if j == i { edited.clone() } else { l.to_string() } + "\n").collect();
        let parsed = settings(&changed).unwrap_or_else(|p| panic!("uncommenting {line:?}: {p}"));
        assert!(parsed.warnings.is_empty(), "uncommenting {line:?}: {:?}", parsed.warnings);
    }
    let all: String = text.lines().map(|l| uncomment(l) + "\n").collect();
    let parsed = settings(&all).unwrap();
    assert_eq!(Config { raw: Config::default().raw, ..parsed.config }, Config::default());
}

#[test]
fn the_created_accounts_file_has_no_account_and_a_valid_example() {
    let text = accounts_template();
    let parsed = accounts(&text).unwrap();
    assert!(parsed.config.accounts.is_empty());
    assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
    // Uncommenting the example gives a valid account, without warning.
    let example: String = text.lines().map(|l| uncomment(l) + "\n").collect();
    let parsed = accounts(&example).unwrap_or_else(|p| panic!("{p}\n{example}"));
    let a = &parsed.config.accounts["perso"];
    assert_eq!(a.identities.len(), 2, "the example repeats the block");
    assert_eq!(a.identities[0].name.as_deref(), Some("My Company"));
    assert_eq!(a.identities[1].name, None);
    assert!(a.smtp.is_some());
    assert_eq!(a.sync.poll_interval_secs, 600, "the override example applies");
    assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
}

#[test]
fn accounts_in_config_toml_are_refused() {
    // Ignoring them would start without the user's accounts, silently.
    let p = refused(&format!("[sync]\npoll_interval_secs = 60\n\n{EXAMPLE}").replace("version = 1\n", ""));
    assert_eq!(p.key.as_deref(), Some("accounts"));
    assert_eq!(p.line, Some(5), "on the first account's header: {p}");
    assert!(p.message.contains("accounts.toml"), "{p}");
}

#[test]
fn every_problem_names_its_file() {
    // The same line, 2, in either file: only the file tells them apart.
    let p = refused("[sync]\npoll_interval_secs = 1\n");
    assert_eq!((p.file, p.line), ("config.toml", Some(2)));
    assert!(p.to_string().starts_with("config.toml, line 2: sync.poll_interval_secs: "), "{p}");
    let p = refused_accounts("[accounts.a]\nemail = 3\n");
    assert_eq!((p.file, p.line), ("accounts.toml", Some(2)));
    assert!(p.to_string().starts_with("accounts.toml, line 2: "), "{p}");
    // A valid config.toml does not save an invalid accounts.toml: both are
    // refused (`configuration.md`, section 4).
    assert_eq!(parse(SETTINGS, "[accounts.a]\nemail = 3\n").unwrap_err().file, "accounts.toml");
    let p = parse("[sync]\npoll_interval_secs = 1\n", EXAMPLE).unwrap_err();
    assert_eq!(p.file, "config.toml");
    // Warnings too.
    let parsed = parse("turbo = 1\n", "[sync]\npoll_interval_secs = 60\n").unwrap();
    let w: Vec<_> = parsed.warnings.iter().map(|w| (w.file, w.key.clone().unwrap())).collect();
    assert_eq!(w, vec![("config.toml", "turbo".into()), ("accounts.toml", "sync".into())]);
    assert_eq!(parsed.config.sync.poll_interval_secs, 300, "[sync] in accounts.toml has no effect");
}

#[test]
fn accounts_toml_has_its_own_version() {
    let p = refused_accounts("version = 2\n");
    assert_eq!(p.line, Some(1));
    assert!(p.message.contains("newer NiceMailer"));
}

#[test]
fn accounts_toml_is_next_to_config_toml() {
    let path = std::path::Path::new("/somewhere/dotfiles/nicemailer.toml");
    assert_eq!(config::accounts_path(path), std::path::Path::new("/somewhere/dotfiles/accounts.toml"));
}

#[test]
fn compose_and_save_sent() {
    // Defaults: 5 s to cancel, Sent copied unless the provider does it.
    let c = accounts(EXAMPLE).unwrap().config;
    assert_eq!(c.compose.undo_send_seconds, 5);
    assert_eq!(c.accounts["perso"].save_sent, config::SaveSent::Auto);
    let c = parse("[compose]\nundo_send_seconds = 0\n", &EXAMPLE.replace("config_sync = false", "config_sync = false\nsave_sent = \"never\""))
        .unwrap()
        .config;
    assert_eq!(c.compose.undo_send_seconds, 0);
    assert_eq!(c.accounts["perso"].save_sent, config::SaveSent::Never);
    // Out of range, or not a choice: refused with the line.
    let p = refused("[compose]\nundo_send_seconds = 31\n");
    assert_eq!(p.line, Some(2));
    let p = refused_accounts(&EXAMPLE.replace("config_sync = false", "config_sync = false\nsave_sent = \"sometimes\""));
    assert_eq!(p.key.as_deref(), Some("accounts.perso.save_sent"));
}

#[test]
fn the_title_bar_is_hidden_unless_asked() {
    assert_eq!(settings("").unwrap().config.ui.title_bar, config::TitleBar::Hidden);
    assert_eq!(settings("[ui]\ntitle_bar = \"shown\"\n").unwrap().config.ui.title_bar, config::TitleBar::Shown);
    let p = refused("[ui]\ntitle_bar = \"maybe\"\n");
    assert_eq!(p.key.as_deref(), Some("ui.title_bar"));
}

/// The app confirms its calls only when asked: the setting needs the
/// Accessibility permission (`docs/geek-ui.md`, "Calling a number").
#[test]
fn calls_are_confirmed_by_the_app_only_when_asked() {
    assert!(!settings("").unwrap().config.ui.auto_confirm_calls);
    assert!(settings("[ui]\nauto_confirm_calls = true\n").unwrap().config.ui.auto_confirm_calls);
    let p = refused("[ui]\nauto_confirm_calls = \"yes\"\n");
    assert_eq!(p.key.as_deref(), Some("ui.auto_confirm_calls"));
}

/// The unsubscribe of the mails of Banned: closed unless asked
/// (`ui.md`, "Désabonnement").
#[test]
fn unsubscribing_in_banned_is_off_unless_asked() {
    assert!(!settings("").unwrap().config.unsubscribe.in_banned);
    assert!(settings("[unsubscribe]\nin_banned = true\n").unwrap().config.unsubscribe.in_banned);
    let p = refused("[unsubscribe]\nin_banned = \"yes\"\n");
    assert_eq!(p.key.as_deref(), Some("unsubscribe.in_banned"));
}

/// `[ai]` (`ia.md`, section 13): off by default, every value checked.
#[test]
fn the_ai_is_off_unless_asked_and_its_settings_are_checked() {
    let d = settings("").unwrap().config.ai;
    assert!(!d.enabled, "nothing goes to an AI provider unless the user says so");
    assert_eq!(d.provider, config::AiProviderKind::ClaudeCode);
    assert_eq!(d.model, "claude-sonnet-5-5");
    assert_eq!(d.claude_path, None);
    assert_eq!((d.timeout_secs, d.max_parallel, d.batch_size, d.max_folder_items, d.history_days), (120, 2, 20, 500, 30));

    let a = settings(
        "[ai]\nenabled = true\nprovider = \"anthropic-api\"\nmodel = \"claude-haiku-5-5\"\n\
         claude_path = \"~/bin/claude\"\ntimeout_secs = 30\nmax_parallel = 1\nbatch_size = 5\n\
         max_folder_items = 100\nhistory_days = 7\n",
    )
    .unwrap()
    .config
    .ai;
    assert!(a.enabled);
    assert_eq!(a.provider, config::AiProviderKind::AnthropicApi);
    assert_eq!(a.model, "claude-haiku-5-5");
    assert!(a.claude_path.as_ref().unwrap().is_absolute() && a.claude_path.unwrap().ends_with("bin/claude"));
    assert_eq!((a.timeout_secs, a.max_parallel, a.batch_size, a.max_folder_items, a.history_days), (30, 1, 5, 100, 7));

    for (text, key) in [
        ("provider = \"openai\"", "ai.provider"),
        ("model = \"claude sonnet\"", "ai.model"),
        ("model = \"\"", "ai.model"),
        ("claude_path = \"bin/claude\"", "ai.claude_path"),
        ("timeout_secs = 4", "ai.timeout_secs"),
        ("timeout_secs = 601", "ai.timeout_secs"),
        ("max_parallel = 0", "ai.max_parallel"),
        ("max_parallel = 9", "ai.max_parallel"),
        ("batch_size = 101", "ai.batch_size"),
        ("max_folder_items = 5001", "ai.max_folder_items"),
        ("history_days = 0", "ai.history_days"),
        ("enabled = \"yes\"", "ai.enabled"),
    ] {
        let p = refused(&format!("[ai]\n{text}\n"));
        assert_eq!(p.key.as_deref(), Some(key), "{text}: {p:?}");
        assert_eq!(p.line, Some(2), "{text}: {p:?}");
    }
    // An unknown key is a warning, as everywhere: ai.languages was planned,
    // then dropped (`ia.md`, section 8).
    let w = settings("[ai]\nlanguages = [\"en\"]\n").unwrap().warnings;
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].key.as_deref(), Some("ai.languages"));
}
