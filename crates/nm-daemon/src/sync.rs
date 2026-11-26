//! The synchronization in the core (`nm-engine`, `sync/`): the workers get
//! the store and the events through [`CoreHost`], are brought in line with
//! `accounts.toml` by [`apply`], and answer `sync_status`, `sync_now` and
//! `sync_verify`.

use std::collections::BTreeSet;
use std::sync::Arc;

use nm_api::reason;
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
        let mut store = self.0.lock_store("sync");
        let changes = f(&mut store)?;
        // Emitted with the store's lock held, like the commands' events:
        // their order is the order of the writes.
        self.0.events.lock().unwrap().emit(&changes);
        Ok(())
    }

    fn read(&self, f: &mut dyn FnMut(&Store) -> Result<(), StoreError>) -> Result<(), StoreError> {
        // The workers' own pool (`Core::sync_readers`).
        self.0.sync_readers.read(|s| f(s))
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

    fn account_blocked(&self, account_id: i64, why: Option<nm_api::Why>) {
        // The engine's lock only, not held while anything else is taken.
        self.0.engine.lock().unwrap().set_worker_error(account_id, why);
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
    // A read-only account gets no action in the spool, and its mail no
    // command (`spool.rs`, `set_read_only_accounts`).
    let writable = specs.iter().filter(|s| !s.config.read_only).map(|s| s.account_id).collect::<BTreeSet<_>>();
    let read_only = specs.iter().filter(|s| s.config.read_only).map(|s| s.account_id).collect::<BTreeSet<_>>();
    if let Err(e) = store.set_server_accounts(writable).and_then(|_| store.set_read_only_accounts(read_only)) {
        eprintln!("sync : {e}");
    }
    drop(engine);
    syncer.apply(specs);
}

fn syncer(core: &Core) -> Result<&Syncer, RpcError> {
    core.syncer.get().ok_or_else(|| RpcError::internal("synchronization not started"))
}

/// `sync_status`: the state of each worker, with the counters of the
/// database read now.
pub async fn status(core: &Arc<Core>, p: SyncAccountParams) -> Result<SyncStatus, RpcError> {
    let mut accounts: Vec<AccountSync> = syncer(core)?.status();
    if let Some(key) = &p.account {
        accounts.retain(|a| &a.key == key);
        if accounts.is_empty() {
            return Err(RpcError::new(error_code::NOT_FOUND, reason::ACCOUNT_NOT_SYNCHRONIZED.with("account", key.as_str())));
        }
    }
    // Read before borrowing a reading connection: no lock of the core is
    // taken while one is held.
    let configured: std::collections::BTreeMap<String, String> = {
        let engine = core.engine.lock().unwrap();
        engine.config().accounts.iter().filter_map(|(k, c)| c.authserv_id.clone().map(|id| (k.clone(), id))).collect()
    };
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
                a.actions_pending = s.spool_pending(a.account_id)?;
                if let Some((state, error)) = s.initial_archive_state(a.account_id)? {
                    a.initial_archive = Some(state);
                    a.initial_archive_error = error;
                }
                a.authserv_id = match configured.get(&a.key) {
                    Some(id) => Some(id.clone()),
                    None => s.account_authserv_id(a.account_id)?,
                };
            }
            Ok(SyncStatus { accounts })
        })
    })
    .await
    .map_err(|e| RpcError::internal(e))?
    .map_err(crate::store_error)
}

/// `sync_now`.
pub fn now(core: &Core, p: SyncAccountParams) -> Result<(), RpcError> {
    if syncer(core)?.sync_now(p.account.as_deref()) {
        Ok(())
    } else {
        Err(RpcError::new(error_code::NOT_FOUND, match p.account {
            Some(k) => reason::ACCOUNT_NOT_SYNCHRONIZED.with("account", k),
            None => reason::NO_IMAP_ACCOUNT.into(),
        }))
    }
}

/// `sync_verify`: on a blocking thread, with a connection of its own.
pub async fn verify(core: &Arc<Core>, p: SyncVerifyParams) -> Result<SyncVerifyReport, RpcError> {
    let Some(spec) = syncer(core)?.spec(&p.account) else {
        return Err(RpcError::new(error_code::NOT_FOUND, reason::ACCOUNT_NOT_SYNCHRONIZED.with("account", p.account.as_str())));
    };
    let secrets = core.engine.lock().unwrap().secrets();
    let host: Arc<dyn SyncHost> = Arc::new(CoreHost(core.clone()));
    let account_id = spec.account_id;
    let oauth = core.oauth.clone();
    let report = tokio::task::spawn_blocking(move || nm_engine::sync::verify::verify_blocking(&spec, secrets, oauth, host, p.skip_hash, p.repair))
        .await
        .map_err(|e| RpcError::internal(e))?
        .map_err(|e| RpcError::new(error_code::SERVER, reason::IMAP_FAILED.with("server_text", e)))?;
    if report.repaired > 0
        && let Some(syncer) = core.syncer.get()
    {
        syncer.refetch(&BTreeSet::from([account_id]));
    }
    Ok(report)
}

/// A command queued changes for servers: their workers send them now.
pub fn wake(core: &Core, changes: &ChangeSet) {
    if let Some(syncer) = core.syncer.get() {
        syncer.wake(&changes.spooled);
        syncer.wake_send(&changes.sending);
    }
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

/// The answer of a command that read a raw file: a damaged one (corrupt or
/// missing, `RAW_DAMAGED`) is set aside and its mail downloaded again
/// (`nm-store`, `gc.rs`), then the error goes to the client, which can try
/// again once the mail is back.
pub async fn repaired<T>(core: &Arc<Core>, r: Result<T, RpcError>) -> Result<T, RpcError> {
    if let Err(e) = &r
        && e.code == error_code::RAW_DAMAGED
        && let Some(hash) = hash_in(&e.message)
    {
        repair(core, hash).await;
    }
    r
}

/// The hash (64 hex digits) in an error message.
fn hash_in(message: &str) -> Option<nm_store::raw::Hash> {
    let token = message
        .split(|c: char| !c.is_ascii_hexdigit())
        .find(|t| t.len() == 64)?;
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&token[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

async fn repair(core: &Arc<Core>, hash: nm_store::raw::Hash) {
    let raw = core.raw.clone();
    match crate::with_store(core, move |s| s.raw_damaged(&raw, &hash)).await {
        Ok(r) => {
            eprintln!(
                "raw : {} endommagé, mis à l'écart ; {} mail(s) retéléchargé(s), {} sans serveur",
                nm_store::raw::hex(&hash),
                r.refetch,
                r.lost
            );
            if let Some(syncer) = core.syncer.get() {
                syncer.refetch(&r.accounts);
            }
        }
        Err(e) => eprintln!("raw : réparation impossible : {}", e.message),
    }
}

/// The garbage collector of the raw store (`nm-store`, `gc.rs`): a minute
/// after the start, then every hour, on a thread of its own.
pub fn start_gc(core: &Arc<Core>) {
    let core = core.clone();
    std::thread::Builder::new()
        .name("nm-raw-gc".into())
        .spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(60));
            loop {
                match run_gc(&core) {
                    Ok(r) if r != nm_store::gc::GcReport::default() => {
                        eprintln!("raw : ramasse-miettes : {r:?}")
                    }
                    Ok(_) => {}
                    Err(e) => eprintln!("raw : ramasse-miettes : {e}"),
                }
                std::thread::sleep(std::time::Duration::from_secs(3600));
            }
        })
        .expect("thread");
}

/// One collection, with the grace delay of `raw.gc_grace_secs`.
pub fn run_gc(core: &Core) -> Result<nm_store::gc::GcReport, StoreError> {
    let grace = std::time::Duration::from_secs(core.engine.lock().unwrap().config().raw.gc_grace_secs);
    let mut report = nm_store::gc::collect_garbage(&core.store, &core.raw, Some(core.attachments.root()), grace)?;
    report.outgoing = nm_store::gc::collect_outgoing(&core.store, &core.outgoing, grace)?;
    Ok(report)
}
