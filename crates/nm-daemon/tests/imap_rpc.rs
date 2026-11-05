//! End to end: `probe_account`, the IMAP probe of the core
//! (`nicemailer imap probe`). The core reads the password from its
//! throwaway keychain (`common::start`), and talks to the server.
//!
//! The test against a real server is `#[ignore]` (`make imap-test`, after
//! `make imap-up`); the others need no server.

mod common;

use common::{Client, start};
use serde_json::json;

fn account(port: u16, security: &str, user: &str) -> String {
    let ca = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dev/dovecot/tls/ca.crt");
    format!(
        "[accounts.dovecot]\nemail = \"{user}@example.com\"\n[accounts.dovecot.imap]\nhost = \"127.0.0.1\"\n\
         port = {port}\nsecurity = \"{security}\"\nusername = \"{user}\"\nca_file = \"{}\"\n",
        ca.display()
    )
}

#[test]
fn probe_refuses_what_it_cannot_do_and_says_why() {
    let dir = tempfile::tempdir().unwrap();
    // Port 1 on this machine: nothing listens there.
    std::fs::write(dir.path().join("config.toml"), account(1, "none", "alice")).unwrap();
    let _core = start(dir.path());
    let mut c = Client::connect(dir.path());

    let err = c.call("probe_account", json!({"account": "nobody"})).unwrap_err();
    assert_eq!(err["code"], 1001, "{err}");

    // No password yet: the account is not ready, and the message says how.
    let err = c.call("probe_account", json!({"account": "dovecot"})).unwrap_err();
    assert_eq!(err["code"], 1008, "{err}");
    assert!(err["message"].as_str().unwrap().contains("nicemailer account password dovecot"), "{err}");

    // A password, but no server: the server's error.
    c.call("set_password", json!({"account": "dovecot", "password": "nicemailer"})).unwrap();
    let err = c.call("probe_account", json!({"account": "dovecot"})).unwrap_err();
    assert_eq!(err["code"], 1009, "{err}");
    assert!(err["message"].as_str().unwrap().contains("cannot connect"), "{err}");
    assert!(!err.to_string().contains("nicemailer\""), "the password is in no error");
}

#[test]
#[ignore = "needs the test servers: make imap-up, then make imap-test"]
fn probe_reports_capabilities_folders_and_roles() {
    let dir = tempfile::tempdir().unwrap();
    let user = format!("p{:x}", std::process::id());
    std::fs::write(dir.path().join("config.toml"), account(31993, "tls", &user)).unwrap();
    let _core = start(dir.path());
    let mut c = Client::connect(dir.path());
    c.call("set_password", json!({"account": "dovecot", "password": "nicemailer"})).unwrap();

    let r = c.call("probe_account", json!({"account": "dovecot"})).unwrap();
    assert_eq!(r["server"], "127.0.0.1:31993");
    assert_eq!(r["security"], "tls");
    let caps = |k: &str| r[k].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect::<Vec<_>>();
    assert!(!caps("capabilities_before_login").contains(&"QRESYNC".to_string()));
    assert!(caps("capabilities").contains(&"QRESYNC".to_string()));
    assert_eq!(caps("enabled"), vec!["CONDSTORE", "QRESYNC"]);
    let folders = r["folders"].as_array().unwrap();
    assert_eq!(folders[0]["name"], "INBOX", "INBOX first");
    assert_eq!(folders[0]["proposed_role"], "inbox");
    assert_eq!(folders[0]["messages"], 0);
    let sent = folders.iter().find(|f| f["name"] == "Sent").unwrap();
    assert_eq!(sent["proposed_role"], "sent");
    assert_eq!(sent["role_from"], "special-use");
    let trash = folders.iter().find(|f| f["name"] == "Trash").unwrap();
    assert_eq!(trash["proposed_role"], "deleted");

    // A wrong password is the server's refusal.
    c.call("set_password", json!({"account": "dovecot", "password": "wrong"})).unwrap();
    let err = c.call("probe_account", json!({"account": "dovecot"})).unwrap_err();
    assert_eq!(err["code"], 1009);
    assert!(err["message"].as_str().unwrap().contains("refused the login"), "{err}");
}
