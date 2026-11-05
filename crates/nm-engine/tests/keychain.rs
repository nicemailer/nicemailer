//! The real Keychain (`keychain.rs`), on a throwaway keychain file: these
//! tests never touch the login keychain, and never show a dialog
//! (`never_prompt`).

use std::path::{Path, PathBuf};
use std::process::Command;

use nm_engine::config::SecretString;
use nm_engine::keychain::{Keychain, never_prompt};
use nm_engine::secrets::{Binding, Credential, Secrets};

/// A new, unlocked keychain file in a temporary directory. `security
/// create-keychain` does not add it to the user's search list, so nothing
/// else on the machine sees it.
fn throwaway() -> (tempfile::TempDir, PathBuf) {
    never_prompt().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.keychain-db");
    security(&["create-keychain", "-p", "test", path.to_str().unwrap()]);
    security(&["unlock-keychain", "-p", "test", path.to_str().unwrap()]);
    (dir, path)
}

fn security(args: &[&str]) {
    let out = Command::new("/usr/bin/security").args(args).output().unwrap();
    assert!(out.status.success(), "security {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn credential(host: &str, password: &str) -> Credential {
    Credential { binding: Binding { host: host.into(), username: "me@example.com".into() }, secret: SecretString::new(password) }
}

fn keychain(path: &Path) -> Keychain {
    Keychain::file(path).unwrap()
}

#[test]
fn a_saved_password_is_read_back_with_its_server() {
    let (_dir, path) = throwaway();
    let kc = keychain(&path);
    assert_eq!(kc.imap_binding("perso").unwrap(), None);
    assert_eq!(kc.imap("perso").unwrap(), None);
    kc.set_imap("perso", &credential("imap.example.com", "pâss wörd")).unwrap();
    let binding = kc.imap_binding("perso").unwrap().unwrap();
    assert_eq!(binding.host, "imap.example.com");
    assert_eq!(binding.username, "me@example.com");
    let c = kc.imap("perso").unwrap().unwrap();
    assert_eq!(c.secret.expose(), "pâss wörd", "UTF-8 and spaces kept");
    // Another account has nothing.
    assert_eq!(kc.imap("pro").unwrap(), None);
}

#[test]
fn saving_again_replaces_the_password_and_the_server() {
    let (_dir, path) = throwaway();
    let kc = keychain(&path);
    kc.set_imap("perso", &credential("imap.example.com", "one")).unwrap();
    kc.set_imap("perso", &credential("imap.new.example", "two")).unwrap();
    let c = kc.imap("perso").unwrap().unwrap();
    assert_eq!(c.secret.expose(), "two");
    assert_eq!(c.binding.host, "imap.new.example");
}

#[test]
fn the_item_is_readable_in_keychain_access() {
    // What a user sees: the label and the server, never needed to guess.
    let (_dir, path) = throwaway();
    keychain(&path).set_imap("perso", &credential("imap.example.com", "pw")).unwrap();
    let out = Command::new("/usr/bin/security")
        .args(["find-generic-password", "-s", "NiceMailer", "-a", "imap:perso", path.to_str().unwrap()])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("NiceMailer: perso (IMAP)"), "{text}");
    assert!(text.contains("imap.example.com"), "{text}");
    assert!(!text.contains("\"pw\""), "the password is not printed without -w");
}

#[test]
fn another_application_gets_the_server_but_not_the_password() {
    // An item saved by another program (here the `security` tool) is
    // readable by the core only with a permission it may not ask for: the
    // server is still given, the password is refused with a message.
    let (_dir, path) = throwaway();
    security(&[
        "add-generic-password", "-s", "NiceMailer", "-a", "imap:perso", "-j",
        r#"{"host":"imap.example.com","username":"me@example.com"}"#, "-w", "pw", path.to_str().unwrap(),
    ]);
    let kc = keychain(&path);
    assert_eq!(kc.imap_binding("perso").unwrap().unwrap().host, "imap.example.com");
    let err = kc.imap("perso").unwrap_err();
    assert!(err.contains("make dev-cert"), "{err}");
    // And it cannot be replaced either: deleting it needs the same
    // permission.
    assert!(kc.set_imap("perso", &credential("imap.example.com", "new")).is_err());
}

#[test]
fn a_locked_keychain_refuses_without_a_dialog() {
    let (_dir, path) = throwaway();
    let kc = keychain(&path);
    kc.set_imap("perso", &credential("imap.example.com", "pw")).unwrap();
    security(&["lock-keychain", path.to_str().unwrap()]);
    // The test would hang on a dialog if `never_prompt` did not work.
    let err = kc.imap("perso").unwrap_err();
    assert!(err.contains("locked"), "{err}");
    // The server is still known: the state of the account can be shown
    // while the keychain is locked.
    assert_eq!(kc.imap_binding("perso").unwrap().unwrap().host, "imap.example.com");
}
