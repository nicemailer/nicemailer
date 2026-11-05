//! The synchronization in the core (`nm-engine`, `sync/`): the workers get
//! the store and the events through [`CoreHost`], are brought in line with
//! `config.toml` by [`apply`], and answer `sync_status`, `sync_now` and
//! `sync_verify`.

use std::collections::BTreeSet;
use std::sync::Arc;

use nm_api::{
    AccountSync, Event, FolderSync, RpcError, SyncAccountParams, SyncStatus, SyncVerifyParams, SyncVerifyReport, error_code,
};
use nm_engine::sync::{AccountSpec, SyncHost, Syncer};
use nm_store::raw::RawStore;
use nm_store::{ChangeSet, Store, StoreError};

use crate::Core;

/// The core, as the workers see it.
pub struct CoreHost(pub Arc<Core>);

impl SyncHost for CoreHost {
    fn write(&self, f: &mut dyn FnMut(&mut Store) -> Result<ChangeSet, StoreError>) -> Result<(), StoreError> {
        let mut store = self.0.store.lock().unwrap();
        let changes = f(&mut store)?;
        // Emitted with the store's lock held, like the commands' events:
        // their order is the order of the writes.
        self.0.events.lock().unwrap().emit(&changes);
        Ok(())
    }

    fn read(&self, f: &mut dyn FnMut(&Store) -> Result<(), StoreError>) -> Result<(), StoreError> {
        self.0.readers.read(|s| f(s))
    }

    fn event(&self, event: Event) {
        self.0.events.lock().unwrap().push(event);
    }

    fn transient(&self, event: Event) {
        self.0.events.lock().unwrap().push_transient(event);
    }

    fn raw(&self) -> &RawStore {
        &self.0.raw
    }
}

/// Brings the workers in line with the accounts of the file applied, and
/// tells the store which accounts are synchronized (their mail cannot be
/// changed by commands before phase 5).
///
/// Called with the store's lock held (`store`), after each reading of the
/// file: lock order store, engine. Starting or stopping a worker waits for
/// nothing.
pub fn apply(core: &Core, store: &mut Store) {
    let Some(syncer) = core.syncer.get() else { return };
    let engine = core.engine.lock().unwrap();
    let mut specs = Vec::new();
    for (key, account) in &engine.config().accounts {
        if let Some(id) = engine.account_id(key) {
            specs.push(AccountSpec { key: key.clone(), account_id: id, config: account.clone() });
        }
    }
    store.set_server_accounts(specs.iter().map(|s| s.account_id).collect::<BTreeSet<_>>());
    drop(engine);
    syncer.apply(specs);
}

fn syncer(core: &Core) -> Result<&Syncer, RpcError> {
    core.syncer.get().ok_or_else(|| RpcError::new(error_code::INTERNAL_ERROR, "synchronization not started"))
}

/// `sync_status`: the state of each worker, with the counters of the
/// database read now.
pub async fn status(core: &Arc<Core>, p: SyncAccountParams) -> Result<SyncStatus, RpcError> {
    let mut accounts: Vec<AccountSync> = syncer(core)?.status();
    if let Some(key) = &p.account {
        accounts.retain(|a| &a.key == key);
        if accounts.is_empty() {
            return Err(RpcError::new(error_code::NOT_FOUND, format!("account {key} is not synchronized")));
        }
    }
    let core2 = core.clone();
    tokio::task::spawn_blocking(move || {
        core2.readers.read(|s| {
            for a in &mut accounts {
                let missing: BTreeSet<String> = a.folders.iter().filter(|f| f.missing_on_server).map(|f| f.path.clone()).collect();
                a.folders = s
                    .account_sync_folders(a.account_id)?
                    .into_iter()
                    .map(|(f, last, bodies)| FolderSync {
                        folder_id: f.id,
                        missing_on_server: missing.contains(&f.path),
                        path: f.path,
                        role: f.role,
                        messages: f.messages as u64,
                        bodies_missing: bodies,
                        last_sync: last,
                    })
                    .collect();
                a.bodies_missing = a.folders.iter().map(|f| f.bodies_missing).sum();
            }
            Ok(SyncStatus { accounts })
        })
    })
    .await
    .map_err(|e| RpcError::new(error_code::INTERNAL_ERROR, e.to_string()))?
    .map_err(crate::store_error)
}

/// `sync_now`.
pub fn now(core: &Core, p: SyncAccountParams) -> Result<(), RpcError> {
    if syncer(core)?.sync_now(p.account.as_deref()) {
        Ok(())
    } else {
        Err(RpcError::new(error_code::NOT_FOUND, match p.account {
            Some(k) => format!("account {k} is not synchronized"),
            None => "no IMAP account to synchronize".into(),
        }))
    }
}

/// `sync_verify`: on a blocking thread, with a connection of its own.
pub async fn verify(core: &Arc<Core>, p: SyncVerifyParams) -> Result<SyncVerifyReport, RpcError> {
    let Some(spec) = syncer(core)?.spec(&p.account) else {
        return Err(RpcError::new(error_code::NOT_FOUND, format!("account {} is not synchronized", p.account)));
    };
    let secrets = core.engine.lock().unwrap().secrets();
    let host: Arc<dyn SyncHost> = Arc::new(CoreHost(core.clone()));
    tokio::task::spawn_blocking(move || nm_engine::sync::verify::verify_blocking(&spec, secrets, host, p.skip_hash))
        .await
        .map_err(|e| RpcError::new(error_code::INTERNAL_ERROR, e.to_string()))?
        .map_err(|e| RpcError::new(error_code::SERVER, e))
}

/// A thread was opened: its messages still without body go first.
pub fn prioritize(core: &Core, missing: Vec<(i64, i64)>) {
    let Some(syncer) = core.syncer.get() else { return };
    let mut by_account: std::collections::BTreeMap<i64, Vec<i64>> = Default::default();
    for (account, id) in missing {
        by_account.entry(account).or_default().push(id);
    }
    for (account, ids) in by_account {
        syncer.prioritize(account, &ids);
    }
}
