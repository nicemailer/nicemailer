//! Reading and validating `config.toml` (`configuration.md`, sections 3, 4
//! and 12).

use nm_engine::config::{self, AuthMethod, Config, Security, changed_keys, defaults_file, flatten, parse};

/// The example of `configuration.md`, section 3.
const EXAMPLE: &str = r#"version = 1

[core]
background_sorting = true

[sync]
poll_interval_secs = 300

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

/// Parses a file expected to be refused, and returns the problem.
fn refused(text: &str) -> config::Problem {
    match parse(text) {
        Err(p) => p,
        Ok(_) => panic!("expected the file to be refused:\n{text}"),
    }
}

#[test]
fn an_empty_file_gives_the_defaults() {
    let parsed = parse("").unwrap();
    assert_eq!(parsed.config, Config::default());
    assert!(parsed.warnings.is_empty());
    assert!(parsed.config.accounts.is_empty());
}

#[test]
fn a_missing_file_reads_as_none() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(config::read(&dir.path().join("config.toml")).unwrap(), None);
}

#[test]
fn a_file_that_is_not_utf8_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, [0xff, 0xfe, b'a']).unwrap();
    assert!(config::read(&path).unwrap_err().message.contains("UTF-8"));
}

#[test]
fn the_example_of_the_specification_is_read() {
    let parsed = parse(EXAMPLE).unwrap();
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
    let a = &parse(text).unwrap().config.accounts["a"];
    assert_eq!(a.imap.host, "imap.example.com");
    assert_eq!(a.imap.port, 143);
    assert_eq!(a.imap.username, "me@example.com", "the username defaults to the email");
    assert_eq!(a.smtp.as_ref().unwrap().port, 587);

    let tls = parse("[accounts.a]\nemail = \"me@example.com\"\n[accounts.a.imap]\nhost = \"h\"\n").unwrap();
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
    let p = refused("[accounts.a]\nemail = \"a@b.c\"\n[accounts.a.imap]\nhost = \"h\"\nport = 70000\n");
    assert_eq!(p.line, Some(5));
    assert_eq!(p.key.as_deref(), Some("accounts.a.imap.port"));
}

#[test]
fn an_unknown_key_is_a_warning_and_the_rest_applies() {
    let text = "version = 1\nfuture_feature = true\n[sync]\npoll_interval_secs = 60\nturbo = 3\n[newer_table]\nx = 1\n";
    let parsed = parse(text).unwrap();
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
    let parsed = parse(text).unwrap();
    let keys: Vec<_> = parsed.warnings.iter().map(|w| w.key.clone().unwrap()).collect();
    assert_eq!(keys, vec!["accounts.a.imap.compress", "accounts.a.nickname"]);
}

#[test]
fn an_account_needs_an_email_and_an_imap_server() {
    let p = refused("version = 1\n[accounts.a]\ndisplay_name = \"A\"\n[accounts.a.imap]\nhost = \"h\"\n");
    assert_eq!(p.key.as_deref(), Some("accounts.a.email"));
    assert_eq!(p.line, Some(2), "on the header of the account");
    let p = refused("[accounts.a]\nemail = \"a@b.c\"\n");
    assert_eq!(p.key.as_deref(), Some("accounts.a.imap"));
    let p = refused("[accounts.a]\nemail = \"a@b.c\"\n[accounts.a.imap]\nport = 993\n");
    assert_eq!(p.key.as_deref(), Some("accounts.a.imap.host"));
    let p = refused("[accounts.a]\nemail = \"not an address\"\n[accounts.a.imap]\nhost = \"h\"\n");
    assert_eq!(p.key.as_deref(), Some("accounts.a.email"));
}

#[test]
fn an_account_key_is_simple() {
    let p = refused("[accounts.\"mon compte\"]\nemail = \"a@b.c\"\n[accounts.\"mon compte\".imap]\nhost = \"h\"\n");
    assert!(p.message.contains("letters, digits"), "{p}");
}

#[test]
fn an_empty_string_is_refused() {
    let p = refused("[accounts.a]\nemail = \"a@b.c\"\ndisplay_name = \"  \"\n[accounts.a.imap]\nhost = \"h\"\n");
    assert_eq!(p.key.as_deref(), Some("accounts.a.display_name"));
}

#[test]
fn a_choice_outside_the_list_is_refused() {
    let p = refused("[accounts.a]\nemail = \"a@b.c\"\n[accounts.a.imap]\nhost = \"h\"\nsecurity = \"ssl\"\n");
    assert_eq!(p.line, Some(5));
    assert!(p.message.contains("tls, starttls, none"), "{p}");
}

#[test]
fn no_encryption_only_on_this_mac() {
    let p = refused("[accounts.a]\nemail = \"a@b.c\"\n[accounts.a.imap]\nhost = \"imap.example.com\"\nsecurity = \"none\"\n");
    assert_eq!(p.key.as_deref(), Some("accounts.a.imap.security"));
    assert!(p.message.contains("clear text"));
    let ok = parse("[accounts.a]\nemail = \"a@b.c\"\n[accounts.a.imap]\nhost = \"localhost\"\nsecurity = \"none\"\n").unwrap();
    assert_eq!(ok.config.accounts["a"].imap.port, 143);
}

#[test]
fn a_newer_format_is_refused() {
    let p = refused("version = 2\n");
    assert_eq!(p.line, Some(1));
    assert!(p.message.contains("newer NiceMailer"));
    assert!(parse("version = 0\n").is_err());
}

#[test]
fn an_account_overrides_general_settings() {
    let text = r#"
[sync]
poll_interval_secs = 120
[import]
batch_size = 500

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
    let c = parse(text).unwrap().config;
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
        let ids = parse(text).unwrap().config.accounts["a"].identities.clone();
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0].email, "x@b.c");
        assert_eq!(ids[1].name.as_deref(), Some("Y"));
    }
    let p = refused("[accounts.a]\nemail = \"a@b.c\"\n[accounts.a.imap]\nhost = \"h\"\n[[accounts.a.identities]]\nname = \"N\"\n");
    assert_eq!(p.key.as_deref(), Some("accounts.a.identities[0].email"));
}

#[test]
fn paths_accept_the_home_folder() {
    let home = std::env::var("HOME").unwrap();
    let c = parse("[accounts.a]\nemail = \"a@b.c\"\n[accounts.a.imap]\nhost = \"h\"\nca_file = \"~/certs/ca.pem\"\n").unwrap();
    assert_eq!(c.config.accounts["a"].imap.ca_file.as_ref().unwrap(), &std::path::PathBuf::from(home).join("certs/ca.pem"));
    let p = refused("[raw]\ndir = \"relative/dir\"\n");
    assert_eq!(p.key.as_deref(), Some("raw.dir"));
}

#[test]
fn a_password_in_the_file_is_refused() {
    let text = "[accounts.a]\nemail = \"a@b.c\"\n[accounts.a.imap]\nhost = \"h\"\npassword = \"hunter2\"\n";
    let p = refused(text);
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
    let a = parse(EXAMPLE).unwrap().config;
    let b = parse(&EXAMPLE.replace("port = 993", "port = 994")).unwrap().config;
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
    let parsed = parse(&text).unwrap();
    assert_eq!(parsed.config, Config::default(), "the defaults file gives the defaults");
    assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
    // Every setting that has a value appears explicitly, so the file shows
    // every key; `raw.dir` (no value: computed from the data directory)
    // appears in a comment.
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
    // Uncommenting the example account, at the end of the file, gives a
    // valid account.
    let start = text.find("# [accounts.perso]").expect("example account");
    let example: String = text[start..]
        .lines()
        .map(|l| l.strip_prefix("# ").filter(|r| r.starts_with('[') || r.contains(" = ")).unwrap_or(l))
        .collect::<Vec<_>>()
        .join("\n");
    let parsed = parse(&format!("{}{example}", &text[..start])).unwrap();
    assert_eq!(parsed.config.accounts["perso"].identities.len(), 1);
    assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
}

#[test]
fn compose_and_save_sent() {
    // Defaults: 5 s to cancel, Sent copied unless the provider does it.
    let c = parse(EXAMPLE).unwrap().config;
    assert_eq!(c.compose.undo_send_seconds, 5);
    assert_eq!(c.accounts["perso"].save_sent, config::SaveSent::Auto);
    let c = parse(&format!("[compose]\nundo_send_seconds = 0\n{}", EXAMPLE.replace("config_sync = false", "config_sync = false\nsave_sent = \"never\"")))
        .unwrap()
        .config;
    assert_eq!(c.compose.undo_send_seconds, 0);
    assert_eq!(c.accounts["perso"].save_sent, config::SaveSent::Never);
    // Out of range, or not a choice: refused with the line.
    let p = refused("[compose]\nundo_send_seconds = 31\n");
    assert_eq!(p.line, Some(2));
    let p = refused(&EXAMPLE.replace("config_sync = false", "config_sync = false\nsave_sent = \"sometimes\""));
    assert_eq!(p.key.as_deref(), Some("accounts.perso.save_sent"));
}

#[test]
fn the_title_bar_is_hidden_unless_asked() {
    assert_eq!(parse("").unwrap().config.ui.title_bar, config::TitleBar::Hidden);
    assert_eq!(parse("[ui]\ntitle_bar = \"shown\"\n").unwrap().config.ui.title_bar, config::TitleBar::Shown);
    let p = refused("[ui]\ntitle_bar = \"maybe\"\n");
    assert_eq!(p.key.as_deref(), Some("ui.title_bar"));
}
