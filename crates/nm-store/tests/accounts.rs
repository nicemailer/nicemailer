//! Display order of the accounts (`account.position`, `accounts.rs`).

use nm_core::Source;
use nm_store::{Store, StoreError};

/// Keys of the accounts as listed, and the account of each folder as
/// listed: the order the app's account picker shows.
fn order(store: &Store) -> (Vec<String>, Vec<i64>) {
    let keys = store.list_account_rows().unwrap().into_iter().map(|r| r.config_key).collect();
    let mut folder_accounts: Vec<i64> = Vec::new();
    for f in store.list_folders().unwrap() {
        if folder_accounts.last() != Some(&f.account_id) {
            folder_accounts.push(f.account_id);
        }
    }
    (keys, folder_accounts)
}

fn id(store: &Store, key: &str) -> i64 {
    store.list_account_rows().unwrap().into_iter().find(|r| r.config_key == key).unwrap().id
}

/// Three accounts with folders: two archive accounts, then one of the file.
fn three() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    store.prepare_archive_account("old", "old@example.com", Source::Cli).unwrap();
    store.prepare_archive_account("older", "older@example.com", Source::Cli).unwrap();
    store.sync_config_accounts(&["perso"]).unwrap();
    let perso = id(&store, "perso");
    store
        .conn()
        .execute("INSERT INTO folder (account_id, path, role, created_by, modified_by) VALUES (?1, 'INBOX', 'inbox', 'imap', 'imap')", [perso])
        .unwrap();
    store
}

#[test]
fn a_new_account_goes_last() {
    let store = three();
    let rows = store.list_account_rows().unwrap();
    assert_eq!(rows.iter().map(|r| r.position).collect::<Vec<_>>(), vec![1, 2, 3]);
    let (keys, folders) = order(&store);
    assert_eq!(keys, vec!["old", "older", "perso"]);
    assert_eq!(folders, vec![id(&store, "old"), id(&store, "older"), id(&store, "perso")]);
}

#[test]
fn reordering_moves_the_accounts_and_their_folders() {
    let mut store = three();
    let (old, older, perso) = (id(&store, "old"), id(&store, "older"), id(&store, "perso"));
    let changes = store.set_account_order(&[perso, old], Source::App).unwrap();
    let (keys, folders) = order(&store);
    assert_eq!(keys, vec!["perso", "old", "older"], "the named first, the others after");
    assert_eq!(folders, vec![perso, old, older], "the folder list follows, and so does the picker");
    // Every account moved: each is announced, with its folders.
    assert_eq!(changes.accounts.into_iter().collect::<Vec<_>>(), vec![old, older, perso]);
    assert!(!changes.folders.is_empty());
    let audits: i64 = store
        .conn()
        .query_row("SELECT count(*) FROM audit_log WHERE action = 'reorder' AND created_by = 'app'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(audits, 3);

    // The same order again changes nothing.
    let changes = store.set_account_order(&[perso, old, older], Source::App).unwrap();
    assert!(changes.accounts.is_empty() && changes.folders.is_empty());

    // A new account still goes last.
    store.sync_config_accounts(&["perso", "pro"]).unwrap();
    assert_eq!(order(&store).0.last().unwrap(), "pro");
}

#[test]
fn an_unknown_or_repeated_account_is_refused() {
    let mut store = three();
    let old = id(&store, "old");
    assert!(matches!(store.set_account_order(&[999], Source::App), Err(StoreError::NotFound(_))));
    assert!(matches!(store.set_account_order(&[old, old], Source::App), Err(StoreError::InvalidParams(_))));
    assert_eq!(order(&store).0, vec!["old", "older", "perso"], "nothing changed");
}

#[test]
fn a_disabled_account_keeps_its_place() {
    let mut store = three();
    store.sync_config_accounts(&[]).unwrap();
    let rows = store.list_account_rows().unwrap();
    assert!(rows[2].disabled);
    assert_eq!(rows[2].config_key, "perso");
    store.sync_config_accounts(&["perso"]).unwrap();
    assert_eq!(order(&store).0, vec!["old", "older", "perso"]);
}
